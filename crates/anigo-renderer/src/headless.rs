use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use bytemuck::cast_slice;
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use anigo_core::{MorphChannel, Scene, SparseMorphDelta, SparseMorphHeader, Vertex};
use crate::diagnostics;
use crate::mesh_validation;
use crate::render_contract as contract;
use crate::uniforms::{
    BonePaletteUniform, CameraUniform, LightUniform, MaterialUniform, OutlineUniform,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMetrics {
    pub render_time_ms: f64,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub adapter_name: String,
    pub backend: String,
}

/// P1-04: paleta de skinning entregue pelo núcleo (a mesma do snapshot).
///
/// Tamanho diferente do contrato vira diagnóstico `contract_drift` e a paleta
/// neutra assume: melhor um quadro sem deformação do que um buffer curto
/// interpretado como matrizes pela GPU.
fn bone_palette_uniform(scene: &Scene) -> BonePaletteUniform {
    let joints = contract::skinning_joint_count().max(1) as usize;
    let expected = joints * 16;
    if scene.skin.palette.len() != expected {
        diagnostics::report_with_detail(
            "contract_drift",
            "paleta de skinning fora do tamanho declarado no contrato",
            Some(format!(
                "{} floats (esperado {expected} = {joints} ossos × 16)",
                scene.skin.palette.len()
            )),
        );
        return BonePaletteUniform::identity(joints);
    }
    BonePaletteUniform::from_floats(&scene.skin.palette)
}

pub struct HeadlessRenderer {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
    cel_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    cel_bind_group_layout: wgpu::BindGroupLayout,
    outline_bind_group_layout: wgpu::BindGroupLayout,
    /// P1-04: binding da paleta de ossos em cada passe (vem do contrato).
    cel_skin_binding: u32,
    outline_skin_binding: u32,
    pub morph_compute_pipeline: wgpu::ComputePipeline,
    pub morph_bind_group_layout: wgpu::BindGroupLayout,
    pub toon_ramp_view: wgpu::TextureView,
    pub toon_ramp_sampler: wgpu::Sampler,
    /// Fase 2 (#18): neutro 1x1 branco ancorado nos slots de textura MToon
    /// (cel 6–15) e no mapa de espessura do contorno (outline 3–4) quando o
    /// material/nó não tem textura. Uma única textura + sampler para todos os
    /// nós e passes; o shader só amostra o slot habilitado, então o neutro
    /// nunca chega na imagem com os slots off.
    pub mtoon_neutral_view: wgpu::TextureView,
    pub mtoon_neutral_sampler: wgpu::Sampler,
    /// Fase 2 (#53): Anime Bokeh DoF — passe de pós-processamento (o MESMO
    /// shader/uniforms do viewport). Só roda quando `Scene.dof.enabled`.
    dof_pipeline: Option<wgpu::RenderPipeline>,
    dof_bind_group_layout: wgpu::BindGroupLayout,
    /// Textura de profundidade só é amostrável com sampler sem filtragem.
    dof_nearest_sampler: wgpu::Sampler,
}

impl HeadlessRenderer {
    pub async fn new() -> Result<Self> {
        // P1-02: um contrato em drift aparece na telemetria antes de qualquer
        // coisa ser criada (o resto do quadro fica suspeito).
        diagnostics::report_contract_health();

        let instance = match std::panic::catch_unwind(|| {
            wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            })
        }) {
            Ok(instance) => instance,
            Err(_) => {
                diagnostics::report(
                    "device_unavailable",
                    "nenhum backend wgpu utilizável encontrado (headless não pode renderizar)",
                );
                anyhow::bail!("Failed to find suitable GPU adapter for ANIGO engine");
            }
        };

        // P1-01/P1-02: falha de ambiente vira diagnóstico observável (com código)
        // antes de virar `anyhow::Error`.
        //
        // Além do caminho `None`, o wgpu 24 aborta com um pânico **cru** (sem
        // mensagem) quando o runner não tem backend utilizável — é o caso do CI.
        // Um pânico aqui seria um crash de ambiente, e é exatamente o que
        // P1-01/P1-02 mandam transformar em diagnóstico observável: o guarda
        // converte o pânico no mesmo `device_unavailable`, e os testes de GPU se
        // pulam pelo caminho de erro que já existe (`if let Ok(renderer) = …`).
        let requested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
        }));
        let adapter = match requested {
            Ok(Some(adapter)) => adapter,
            Ok(None) | Err(_) => {
                diagnostics::report(
                    "device_unavailable",
                    "nenhum adaptador wgpu compatível encontrado (headless não pode renderizar)",
                );
                anyhow::bail!("Failed to find suitable GPU adapter for ANIGO engine");
            }
        };

        let adapter_info = adapter.get_info();

        let (device, queue) = match adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ANIGO Headless Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
        {
            Ok(pair) => pair,
            Err(error) => {
                diagnostics::report_with_detail(
                    "device_unavailable",
                    "falha ao criar o device wgpu",
                    Some(error.to_string()),
                );
                return Err(anyhow::Error::new(error).context("Failed to create wgpu device and queue"));
            }
        };

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // Toon Ramp 2D (256x4): bytes e sampler vêm do render contract, então a
        // textura do headless é a mesma do viewport por construção.
        let ramp_spec = contract::toon_ramp_spec();
        let ramp_width = ramp_spec["width"].as_u64().unwrap_or(256) as u32;
        let ramp_height = ramp_spec["height"].as_u64().unwrap_or(4) as u32;
        let ramp_desc = wgpu::TextureDescriptor {
            label: Some("Toon Ramp 2D Texture"),
            size: wgpu::Extent3d {
                width: ramp_width,
                height: ramp_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: contract::texture_format(
                ramp_spec["format"].as_str().unwrap_or("rgba8unorm"),
            ),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let toon_ramp_texture = device.create_texture(&ramp_desc);
        let ramp_data = contract::toon_ramp_bytes();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &toon_ramp_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &ramp_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ramp_width * 4),
                rows_per_image: Some(ramp_height),
            },
            wgpu::Extent3d {
                width: ramp_width,
                height: ramp_height,
                depth_or_array_layers: 1,
            },
        );
        let toon_ramp_view = toon_ramp_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let toon_ramp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Toon Ramp Sampler"),
            address_mode_u: contract::address_mode(
                ramp_spec["address_mode"].as_str().unwrap_or("clamp_to_edge"),
            ),
            address_mode_v: contract::address_mode(
                ramp_spec["address_mode"].as_str().unwrap_or("clamp_to_edge"),
            ),
            mag_filter: contract::filter_mode(ramp_spec["mag_filter"].as_str().unwrap_or("linear")),
            min_filter: contract::filter_mode(ramp_spec["min_filter"].as_str().unwrap_or("linear")),
            ..Default::default()
        });

        // Fase 2 (#18): neutro 1x1 branco para os slots de textura MToon.
        // Criado uma vez e ancorado em todo bind group sem textura; o shader
        // só amostra o slot quando material.params5/params6 o habilita, então
        // o frame congelado (slots off) permanece byte-idêntico ao anterior.
        let neutral_desc = wgpu::TextureDescriptor {
            label: Some("MToon Neutral 1x1 White"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // mesmo formato do alvo offscreen (rgba8unorm) — vindo do contrato
            format: contract::offscreen_color_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let neutral_texture = device.create_texture(&neutral_desc);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &neutral_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let mtoon_neutral_view = neutral_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mtoon_neutral_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("MToon Neutral Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Load Shaders — uma cópia só (crates/anigo-renderer/shaders via contrato)
        let cel_pass_spec = contract::render_pass("cel");
        let outline_pass_spec = contract::render_pass("outline");
        let cel_shader_src = contract::CEL_SHADING_WGSL;
        let outline_shader_src = contract::INVERTED_HULL_WGSL;

        let cel_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cel Shading Shader"),
            source: wgpu::ShaderSource::Wgsl(cel_shader_src.into()),
        });

        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Inverted Hull Shader"),
            source: wgpu::ShaderSource::Wgsl(outline_shader_src.into()),
        });

        // P1-04: o binding da paleta de skinning vem do contrato (5 no cel, 2 no
        // outline). Se o contrato trocar o número, o layout e o WGSL mudam juntos
        // — o que não pode é o headless fixar um literal.
        let cel_skin_binding = contract::skinning_binding("cel").unwrap_or(5);
        let outline_skin_binding = contract::skinning_binding("outline").unwrap_or(2);
        let skin_min_binding = wgpu::BufferSize::new(contract::skinning_palette_bytes() as u64);

        // Cel Bind Group Layout (Camera, Light, Material, ToonRampTexture, ToonRampSampler, Bones)
        let cel_layout_entries = vec![
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: cel_skin_binding,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: skin_min_binding,
                    },
                    count: None,
                },
                // Fase 2 (#18): slots de textura do material anime (VRoid/MToon) —
                // main/shade/second_shade/emission/sphere_add. O slot só entra
                // no cálculo quando material.params5/params6 o habilita; sem
                // textura o headless ancora o neutro 1x1 branco abaixo.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Fase 2 (#17): mapa SDF da sombra facial — ancorado no neutro
                // 1x1 branco; o shader só amostra quando material.params7.z.
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 17,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
        ];
        let cel_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cel Bind Group Layout"),
            entries: &cel_layout_entries,
        });

        // Outline Bind Group Layout (Camera, OutlineUniform, Bones)
        let outline_layout_entries = vec![
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: outline_skin_binding,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: skin_min_binding,
                    },
                    count: None,
                },
                // Fase 2 (#18): mapa de espessura do contorno (MToon outlineWidth).
                // Somente no vertex; quando outline.params2.y == 0 o shader ignora.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
        ];
        let outline_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Outline Bind Group Layout"),
            entries: &outline_layout_entries,
        });

        // Vertex buffer layout (72 B) exatamente como o contrato declara
        let vertex_attributes: Vec<wgpu::VertexAttribute> = contract::vertex_attributes()
            .into_iter()
            .map(|(shader_location, offset, format)| wgpu::VertexAttribute {
                offset,
                shader_location,
                format,
            })
            .collect();
        assert_eq!(
            contract::vertex_stride() as usize,
            std::mem::size_of::<Vertex>(),
            "o vértice do core divergiu do vertex layout do contrato"
        );
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: contract::vertex_stride(),
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        };

        // Cel Shading Pipeline
        let cel_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cel Pipeline Layout"),
            bind_group_layouts: &[&cel_bind_group_layout],
            push_constant_ranges: &[],
        });

        let cel_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cel Shading Pipeline"),
            layout: Some(&cel_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cel_shader,
                entry_point: Some(cel_pass_spec.vertex_entry.unwrap_or("vs_main")),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &cel_shader,
                entry_point: Some(cel_pass_spec.fragment_entry.unwrap_or("fs_main")),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: contract::offscreen_color_format(),
                    blend: cel_pass_spec.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: cel_pass_spec.front_face,
                cull_mode: cel_pass_spec.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: contract::depth_format(),
                depth_write_enabled: cel_pass_spec.depth_write,
                depth_compare: cel_pass_spec.depth_compare,
                stencil: wgpu::StencilState::default(),
                bias: cel_pass_spec.depth_bias,
            }),
            multisample: wgpu::MultisampleState {
                count: contract::msaa_sample_count(),
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });
        // Inverted Hull Pipeline (Culls front faces to show backfaces extruded as outline)
        let outline_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Outline Pipeline Layout"),
            bind_group_layouts: &[&outline_bind_group_layout],
            push_constant_ranges: &[],
        });

        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Inverted Hull Pipeline"),
            layout: Some(&outline_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_shader,
                entry_point: Some(outline_pass_spec.vertex_entry.unwrap_or("vs_main")),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: Some(outline_pass_spec.fragment_entry.unwrap_or("fs_main")),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: contract::offscreen_color_format(),
                    blend: outline_pass_spec.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: outline_pass_spec.front_face,
                cull_mode: outline_pass_spec.cull_mode, // hull invertido: cull front, extruda backfaces
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: contract::depth_format(),
                depth_write_enabled: outline_pass_spec.depth_write,
                depth_compare: outline_pass_spec.depth_compare,
                stencil: wgpu::StencilState::default(),
                bias: outline_pass_spec.depth_bias,
            }),
            multisample: wgpu::MultisampleState {
                count: contract::msaa_sample_count(),
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });
        // Sparse Morph Target Compute Pipeline
        let morph_pass_spec = contract::compute_pass("sparse_morph");
        let morph_shader_src = contract::MORPH_SPARSE_COMPUTE_WGSL;
        let morph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sparse Morph Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(morph_shader_src.into()),
        });

        let morph_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sparse Morph Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let morph_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sparse Morph Pipeline Layout"),
            bind_group_layouts: &[&morph_bind_group_layout],
            push_constant_ranges: &[],
        });

        let morph_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Sparse Morph Compute Pipeline"),
            layout: Some(&morph_pipeline_layout),
            module: &morph_shader,
            entry_point: Some(
                morph_pass_spec.compute_entry.unwrap_or("cs_accumulate_morphs"),
            ),
            compilation_options: Default::default(),
            cache: None,
        });

        // Fase 2 (#53): Anime Bokeh DoF — fullscreen triangle (sem vertex
        // buffer), sem profundidade, 1x. O layout vem do bind group `dof` do
        // contrato; o passe só roda quando `Scene.dof.enabled`.
        let dof_pass_spec = contract::render_pass("dof_post");
        let dof_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Postprocess DoF Shader"),
            source: wgpu::ShaderSource::Wgsl(contract::POSTPROCESS_DOF_WGSL.into()),
        });
        let dof_layout_entries: Vec<wgpu::BindGroupLayoutEntry> =
            contract::bind_group_entries("dof")
                .into_iter()
                .map(|(binding, kind)| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: match kind {
                        "uniform" => wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(contract::uniform_size("dof") as u64),
                        },
                        // scene_color: textureSample (op filtrante) exige textura
                        // filtrável e sampler Filtering na validação da interface.
                        "texture_2d<f32>" => wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        // scene_depth: texture_depth_2d — no wgpu é Depth.
                        "texture_depth_2d" => wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        _ => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    },
                    count: None,
                })
                .collect();
        let dof_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DoF Bind Group Layout"),
            entries: &dof_layout_entries,
        });
        let dof_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DoF Pipeline Layout"),
            bind_group_layouts: &[&dof_bind_group_layout],
            push_constant_ranges: &[],
        });
        let dof_pipeline = if adapter_info.backend == wgpu::Backend::Gl {
            // Naga GLSL backend não suporta leitura de texture_depth_2d em GLSL.
            // Em backends GLES/OpenGL emulados (ex: Mesa llvmpipe no CI), o pipeline DoF não é criado.
            None
        } else {
            Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Anime Bokeh DoF Pipeline"),
                layout: Some(&dof_pipeline_layout),
                cache: None,
                vertex: wgpu::VertexState {
                    module: &dof_shader,
                    entry_point: Some(dof_pass_spec.vertex_entry.unwrap_or("vs_dof")),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &dof_shader,
                    entry_point: Some(dof_pass_spec.fragment_entry.unwrap_or("fs_dof")),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: contract::offscreen_color_format(),
                        blend: dof_pass_spec.blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: dof_pass_spec.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            }))
        };
        let dof_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("DoF Depth Sampler (nearest)"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            device,
            queue,
            adapter_info,
            cel_pipeline,
            outline_pipeline,
            cel_bind_group_layout,
            outline_bind_group_layout,
            cel_skin_binding,
            outline_skin_binding,
            morph_compute_pipeline,
            morph_bind_group_layout,
            toon_ramp_view,
            toon_ramp_sampler,
            mtoon_neutral_view,
            mtoon_neutral_sampler,
            dof_pipeline,
            dof_bind_group_layout,
            dof_nearest_sampler,
        })
    }

    /// Dispatches the sparse morph compute pass directly into an active command encoder.
    pub fn dispatch_sparse_morphs(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        header_buffer: &wgpu::Buffer,
        base_vertices_buffer: &wgpu::Buffer,
        morph_deltas_buffer: &wgpu::Buffer,
        active_channels_buffer: &wgpu::Buffer,
        out_vertices_buffer: &wgpu::Buffer,
        vertex_count: u32,
    ) {
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sparse Morph Bind Group"),
            layout: &self.morph_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: header_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: base_vertices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: morph_deltas_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: active_channels_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: out_vertices_buffer.as_entire_binding(),
                },
            ],
        });

        let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Sparse Morph Compute Pass"),
            timestamp_writes: None,
        });

        compute_pass.set_pipeline(&self.morph_compute_pipeline);
        compute_pass.set_bind_group(0, &bind_group, &[]);
        let workgroups = vertex_count.div_ceil(64);
        compute_pass.dispatch_workgroups(workgroups, 1, 1);
        // wgpu 24 encerra o passe no drop (não existe mais `end()`); o drop
        // explícito garante que o encoder ficou livre para novos comandos.
        drop(compute_pass);
    }

    /// Uploads buffers, runs sparse morph compute pass on GPU, and reads back transformed vertices.
    pub async fn execute_and_read_sparse_morphs(
        &self,
        header: &SparseMorphHeader,
        base_vertices: &[Vertex],
        deltas: &[SparseMorphDelta],
        channels: &[MorphChannel],
    ) -> Result<Vec<Vertex>> {
        let header_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Header Uniform Buffer"),
            contents: bytemuck::bytes_of(header),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let base_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Base Vertices Storage Buffer"),
            contents: cast_slice(base_vertices),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let dummy_delta = SparseMorphDelta::new(0, [0.0; 3], [0.0; 3]);
        let deltas_slice: &[SparseMorphDelta] = if deltas.is_empty() {
            std::slice::from_ref(&dummy_delta)
        } else {
            deltas
        };
        let deltas_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Deltas Storage Buffer"),
            contents: cast_slice(deltas_slice),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let dummy_channel = MorphChannel::new(0.0, 0, 0);
        let channels_slice: &[MorphChannel] = if channels.is_empty() {
            std::slice::from_ref(&dummy_channel)
        } else {
            channels
        };
        let channels_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Channels Storage Buffer"),
            contents: cast_slice(channels_slice),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let out_bytes_size = (base_vertices.len() * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress;
        let out_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Out Vertices Storage Buffer"),
            size: out_bytes_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Vertices Buffer"),
            size: out_bytes_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Morph Compute Command Encoder"),
        });

        self.dispatch_sparse_morphs(
            &mut encoder,
            &header_buffer,
            &base_buffer,
            &deltas_buffer,
            &channels_buffer,
            &out_buffer,
            base_vertices.len() as u32,
        );

        encoder.copy_buffer_to_buffer(&out_buffer, 0, &staging_buffer, 0, out_bytes_size);
        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);

        rx.recv()
            .context("Channel receive failed while waiting for GPU morph buffer mapping")
            .and_then(|result| {
                result.context("Failed to map GPU buffer for morph verification")
            })
            .map_err(|error| {
                // P1-02: falha de readback é observável (código + detalhe), não
                // só uma string no log de quem chamou.
                diagnostics::report_with_detail(
                    "readback_failed",
                    "falha ao mapear o buffer de morphs na GPU",
                    Some(error.to_string()),
                );
                error
            })?;

        let mapped = buffer_slice.get_mapped_range();
        let result_vertices: Vec<Vertex> = bytemuck::cast_slice(&mapped).to_vec();
        drop(mapped);

        Ok(result_vertices)
    }

    /// Renders a scene into an offscreen RGBA image buffer and returns the image + performance metrics.
    pub async fn render_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<(ImageBuffer<Rgba<u8>, Vec<u8>>, RenderMetrics)> {
        let start_time = Instant::now();

        // 1. Setup Render Target and Depth Texture (formato/MSAA do contrato)
        let color_format = contract::offscreen_color_format();
        let depth_format = contract::depth_format();
        let sample_count = contract::msaa_sample_count().max(1);

        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Color Texture (resolve target)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let color_texture = self.device.create_texture(&texture_desc);
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // P0 renderer: mesmo alvo MSAA do viewport, resolvido na textura lida
        let msaa_texture = if sample_count > 1 && contract::resolve_to_swapchain() {
            Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Color Texture (MSAA)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }))
        } else {
            None
        };
        let msaa_view = msaa_texture
            .as_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));

        // Fase 2 (#53): Anime Bokeh DoF — com DoF ligado, a cena resolve numa
        // textura intermediária 1× (amostrável) e o passe de pós escreve na
        // textura que é lida de volta. Sem DoF, tudo é exatamente como antes
        // (frame congelado intacto).
        let dof_enabled = scene.dof.enabled;
        let scene_texture = if dof_enabled {
            Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Color Texture (DoF scene, 1x)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }))
        } else {
            None
        };
        let scene_view = scene_texture
            .as_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let (attachment_view, resolve_target): (&wgpu::TextureView, Option<&wgpu::TextureView>) =
            match (&msaa_view, &scene_view) {
                (Some(view), Some(intermediate)) => (view, Some(intermediate)),
                (Some(view), None) => (view, Some(&color_view)),
                (None, Some(intermediate)) => (intermediate, None),
                (None, None) => (&color_view, None),
            };

        let depth_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Depth Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = self.device.create_texture(&depth_desc);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Profundidade 1× resolvida — o passe de DoF precisa AMOSTRAR a
        // distância (a MSAA depth só existe como attachment).
        let (dof_depth_texture, dof_depth_view) = if dof_enabled && sample_count > 1 {
            let resolved = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Depth Texture (DoF resolve, 1x)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(resolved), Some(view))
        } else {
            (None, None)
        };

        // 2. Setup Camera Uniform
        let mut camera_copy = scene.camera.clone();
        camera_copy.aspect = width as f32 / height as f32;
        let view_proj = camera_copy.build_view_projection_matrix();

        // P1-05: `view_proj`/`camera_pos` são da cena; o **model matrix** é por
        // nó e é montado no laço de desenho. Antes, o transform de
        // `scene.nodes[0]` era aplicado a todos os nós (o nó 2 herdava o
        // transform do nó 1).
        let camera_view_proj = view_proj.to_cols_array();
        let camera_pos = camera_copy.eye.extend(1.0).to_array();

        // 3. Setup Light Uniform
        let light_uniform = LightUniform {
            direction: [
                scene.light.direction[0],
                scene.light.direction[1],
                scene.light.direction[2],
                scene.light.intensity,
            ],
            color: [
                scene.light.color[0],
                scene.light.color[1],
                scene.light.color[2],
                scene.light.ambient_intensity,
            ],
            shadow_color: [
                scene.light.shadow_color[0],
                scene.light.shadow_color[1],
                scene.light.shadow_color[2],
                scene.light.shadow_saturation,
            ],
            ambient_sky: [scene.light.ambient_sky[0], scene.light.ambient_sky[1], scene.light.ambient_sky[2], 1.0],
            ambient_ground: [scene.light.ambient_ground[0], scene.light.ambient_ground[1], scene.light.ambient_ground[2], 1.0],
        };
        let light_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Light Uniform Buffer"),
            contents: cast_slice(&[light_uniform]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // P1-04: paleta de ossos (24 × mat4 = 1536 B) entregue pelo núcleo. Sem
        // ela o WGSL não compila (binding 5/2) e o vértice ficaria na pose de
        // repouso.
        let bone_palette = bone_palette_uniform(scene);
        let bones_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bone Palette Uniform Buffer"),
            contents: cast_slice(&[bone_palette]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // 4. Setup Output Buffer with row pitch alignment (wgpu requires 256 byte alignment)
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Download Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // 5. Command Encoding and Rendering
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Command Encoder"),
        });

        let mut draw_calls = 0;
        let mut triangle_count = 0;

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Anime Cel Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: scene.background_color[0] as f64,
                            g: scene.background_color[1] as f64,
                            b: scene.background_color[2] as f64,
                            a: scene.background_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    // Fase 2 (#53): a resolve da profundidade MSAA→1× é manual
                    // (copy_texture_to_texture) — o wgpu deste projeto não tem
                    // o campo de depth resolve no attachment.
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            for node in &scene.nodes {
                if !node.visible {
                    continue;
                }
                if let Some(mesh) = &node.mesh {
                    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                        continue;
                    }

                    // P1-03: valida a malha **antes** de criar VBO/IBO. Uma
                    // malha reprovada vira diagnóstico com código do contrato e
                    // o nó é pulado (sem buffer torto, sem erro do wgpu).
                    if let Err(problem) = mesh_validation::validate_mesh(mesh) {
                        diagnostics::report_with_detail(
                            "mesh_invalid",
                            "malha reprovada antes de criar buffers de GPU",
                            Some(problem.message()),
                        );
                        continue;
                    }

                    // P1-05: model/normal do nó atual (era o transform de nodes[0]).
                    let model_mat = node.transform.to_matrix();
                    let normal_mat = model_mat.inverse().transpose();
                    let camera_uniform = CameraUniform {
                        view_proj: camera_view_proj,
                        camera_pos,
                        model: model_mat.to_cols_array(),
                        normal_mat: normal_mat.to_cols_array(),
                    };
                    let camera_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Camera Uniform Buffer (node)"),
                        contents: cast_slice(&[camera_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });

                    let mat = node.material.clone().unwrap_or_default();

                    // Material Uniform with Anime Cel-Shading NPR parameters
                    let mat_uniform = MaterialUniform::from(&mat);
                    let mat_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Material Uniform Buffer"),
                        contents: cast_slice(&[mat_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                    // P0-04/09: outline uniform parity with viewport (depthBias/opacity/smoothness, was 0.0/0.0)
                    let aspect = width as f32 / height.max(1) as f32;
                    let outline_uniform = OutlineUniform {
                        color: mat.outline_color,
                        params: [mat.outline_width, aspect, mat.outline_depth_bias, mat.outline_opacity],
                        params2: [mat.outline_smoothness, 0.0, 0.0, 0.0],
                    };
                    let outline_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Outline Uniform Buffer"),
                        contents: cast_slice(&[outline_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                    let cel_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Cel Bind Group"),
                        layout: &self.cel_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: camera_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: light_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: mat_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&self.toon_ramp_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(&self.toon_ramp_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: self.cel_skin_binding,
                                resource: bones_buffer.as_entire_binding(),
                            },
                            // Fase 2 (#18): slots MToon ancoram o neutro 1x1 branco;
                            // o shader só amostra o slot habilitado (params5/6).
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 9,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 10,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 11,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 12,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 13,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 14,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 15,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            // Fase 2 (#17): SDF facial — neutro 1x1 (R=1 → fator 0).
                            wgpu::BindGroupEntry {
                                binding: 16,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 17,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                        ],
                    });

                    let outline_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Outline Bind Group"),
                        layout: &self.outline_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: camera_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: outline_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: self.outline_skin_binding,
                                resource: bones_buffer.as_entire_binding(),
                            },
                            // Fase 2 (#18): mapa de espessura do contorno (neutro 1x1).
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                        ],
                    });

                    let v_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Vertex Buffer"),
                        contents: cast_slice(&mesh.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });

                    let i_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Index Buffer"),
                        contents: cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });

                    // A ordem dos passes vem do contrato (outline → cel), a mesma do viewport
                    render_pass.set_vertex_buffer(0, v_buffer.slice(..));
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    for pass_name in contract::render_pass_order() {
                        match pass_name {
                            "outline" => {
                                render_pass.set_pipeline(&self.outline_pipeline);
                                render_pass.set_bind_group(0, &outline_bind_group, &[]);
                            }
                            "cel" => {
                                render_pass.set_pipeline(&self.cel_pipeline);
                                render_pass.set_bind_group(0, &cel_bind_group, &[]);
                            }
                            // Fase 2 (#53): passe de pós — roda UMA vez depois
                            // do laço de nós (não por nó).
                            "dof_post" => {
                                continue;
                            }
                            other => {
                                // P1-01: passe sem pipeline vira diagnóstico
                                // observável (era `debug_assert!`).
                                contract::report(format!(
                                    "passe '{}' do render contract não tem pipeline no headless",
                                    other
                                ));
                                continue;
                            }
                        }
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                        draw_calls += 1;
                    }

                    triangle_count += mesh.indices.len() / 3;
                }
            }
        }

        // Resolve manual da profundidade (MSAA → 1×): o wgpu deste projeto não
        // expõe depth resolve no attachment; o copy é o padrão do próprio
        // resolve de cor e só entra quando o DoF pediu a textura 1×.
        if let Some(dof_depth_texture) = &dof_depth_texture {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: dof_depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        // Fase 2 (#53): Anime Bokeh DoF — mesmo shader/uniforms do viewport;
        // a cena já resolveu na textura intermediária 1×. Sem as
        // intermediárias o passe é pulado com diagnóstico (sem panic).
        if dof_enabled {
            if let (Some(scene_view_for_dof), Some(dof_depth_for_dof)) =
                (scene_view.as_ref(), dof_depth_view.as_ref())
            {
            let dof_uniform = crate::uniforms::DofUniform::from_settings(
                scene.dof.focus_distance,
                scene.dof.f_number,
                scene.dof.bokeh_shape,
                scene.dof.focal_mm,
                scene.dof.max_radius_px,
                width,
                height,
                scene.camera.z_near,
                scene.camera.z_far,
            );
            let dof_uniform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("DofUniform"),
                contents: bytemuck::bytes_of(&dof_uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let dof_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("DoF Bind Group"),
                layout: &self.dof_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: dof_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(scene_view_for_dof),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(dof_depth_for_dof),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.dof_nearest_sampler),
                    },
                ],
            });
            let mut dof_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Anime Bokeh DoF Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(ref pipeline) = self.dof_pipeline {
                dof_pass.set_pipeline(pipeline);
                dof_pass.set_bind_group(0, &dof_bind_group, &[]);
                // Fullscreen triangle: 3 vértices, sem buffer (vs_dof deriva da index).
                dof_pass.draw(0..3, 0..1);
                draw_calls += 1;
            } else {
                diagnostics::report(
                    "dof_unavailable",
                    "DoF desabilitado: backend gráfico não suporta leitura de textura de profundidade",
                );
            }
            } else {
                diagnostics::report(
                    "dof_unavailable",
                    "DoF habilitado sem as texturas intermediárias 1× — passe pulado neste frame",
                );
            }
        }

        // Copy texture to output download buffer
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        // 6. Map buffer and read image pixels
        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);

        rx.recv()
            .context("Channel receive failed while waiting for GPU buffer mapping")
            .and_then(|result| result.context("Failed to map GPU buffer for image extraction"))
            .map_err(|error| {
                diagnostics::report_with_detail(
                    "readback_failed",
                    "falha ao mapear o buffer de imagem na GPU",
                    Some(error.to_string()),
                );
                error
            })?;

        let padded_data = buffer_slice.get_mapped_range();
        let mut unpadded_pixels = Vec::with_capacity((width * height * 4) as usize);

        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            unpadded_pixels.extend_from_slice(&padded_data[start..end]);
        }

        drop(padded_data);
        output_buffer.unmap();

        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, unpadded_pixels)
            .context("Failed to construct ImageBuffer from raw unpadded pixels")
            .map_err(|error| {
                diagnostics::report_with_detail(
                    "readback_failed",
                    "pixels lidos da GPU não formam uma imagem válida",
                    Some(error.to_string()),
                );
                error
            })?;

        let elapsed = start_time.elapsed().as_secs_f64() * 1000.0;

        let metrics = RenderMetrics {
            render_time_ms: elapsed,
            draw_calls,
            triangle_count,
            adapter_name: self.adapter_info.name.clone(),
            backend: format!("{:?}", self.adapter_info.backend),
        };

        Ok((image, metrics))
    }

    /// Renders a scene with the sparse morph compute pass executed on GPU immediately before cel-shading in the same frame.
    pub async fn render_scene_with_sparse_morphs(
        &self,
        scene: &Scene,
        header: &SparseMorphHeader,
        base_vertices: &[Vertex],
        deltas: &[SparseMorphDelta],
        channels: &[MorphChannel],
        width: u32,
        height: u32,
    ) -> Result<(ImageBuffer<Rgba<u8>, Vec<u8>>, RenderMetrics)> {
        let start_time = Instant::now();

        // 1. Setup Render Target and Depth Texture (formato/MSAA do contrato)
        let color_format = contract::offscreen_color_format();
        let depth_format = contract::depth_format();
        let sample_count = contract::msaa_sample_count().max(1);

        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Color Texture (Morphed, resolve target)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let color_texture = self.device.create_texture(&texture_desc);
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // P0 renderer: mesmo alvo MSAA do viewport, resolvido na textura lida
        let msaa_texture = if sample_count > 1 && contract::resolve_to_swapchain() {
            Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Color Texture (Morphed, resolve target) (MSAA)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }))
        } else {
            None
        };
        let msaa_view = msaa_texture
            .as_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));

        // Fase 2 (#53): mesmo fluxo de DoF do render_scene (intermediária 1×
        // + profundidade resolvida; sem DoF, comportamento anterior).
        let dof_enabled = scene.dof.enabled;
        let scene_texture = if dof_enabled {
            Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Color Texture (Morphed, DoF scene, 1x)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }))
        } else {
            None
        };
        let scene_view = scene_texture
            .as_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let (attachment_view, resolve_target): (&wgpu::TextureView, Option<&wgpu::TextureView>) =
            match (&msaa_view, &scene_view) {
                (Some(view), Some(intermediate)) => (view, Some(intermediate)),
                (Some(view), None) => (view, Some(&color_view)),
                (None, Some(intermediate)) => (intermediate, None),
                (None, None) => (&color_view, None),
            };

        let depth_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Depth Texture (Morphed)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = self.device.create_texture(&depth_desc);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let (dof_depth_texture, dof_depth_view) = if dof_enabled && sample_count > 1 {
            let resolved = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Depth Texture (Morphed, DoF resolve, 1x)"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(resolved), Some(view))
        } else {
            (None, None)
        };

        // 2. Setup Morph Compute Buffers
        let header_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Morph Header Uniform Buffer"),
            contents: bytemuck::bytes_of(header),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let base_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Morph Base Vertices Storage Buffer"),
            contents: cast_slice(base_vertices),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let dummy_delta = SparseMorphDelta::new(0, [0.0; 3], [0.0; 3]);
        let deltas_slice: &[SparseMorphDelta] = if deltas.is_empty() {
            std::slice::from_ref(&dummy_delta)
        } else {
            deltas
        };
        let deltas_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Morph Deltas Storage Buffer"),
            contents: cast_slice(deltas_slice),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let dummy_channel = MorphChannel::new(0.0, 0, 0);
        let channels_slice: &[MorphChannel] = if channels.is_empty() {
            std::slice::from_ref(&dummy_channel)
        } else {
            channels
        };
        let channels_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Morph Channels Storage Buffer"),
            contents: cast_slice(channels_slice),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let morphed_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Morphed Vertex Buffer (Compute Out -> Vertex In)"),
            size: (base_vertices.len() * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // 3. Setup Camera Uniform
        let mut camera_copy = scene.camera.clone();
        camera_copy.aspect = width as f32 / height as f32;
        let view_proj = camera_copy.build_view_projection_matrix();

        // P1-05: `view_proj`/`camera_pos` são da cena; o **model matrix** é por
        // nó e é montado no laço de desenho (antes o nó 2 herdava o transform
        // do nó 1).
        let camera_view_proj = view_proj.to_cols_array();
        let camera_pos = camera_copy.eye.extend(1.0).to_array();

        // 4. Setup Light Uniform
        let light_uniform = LightUniform {
            direction: [
                scene.light.direction[0],
                scene.light.direction[1],
                scene.light.direction[2],
                scene.light.intensity,
            ],
            color: [
                scene.light.color[0],
                scene.light.color[1],
                scene.light.color[2],
                scene.light.ambient_intensity,
            ],
            shadow_color: [
                scene.light.shadow_color[0],
                scene.light.shadow_color[1],
                scene.light.shadow_color[2],
                scene.light.shadow_saturation,
            ],
            ambient_sky: [scene.light.ambient_sky[0], scene.light.ambient_sky[1], scene.light.ambient_sky[2], 1.0],
            ambient_ground: [scene.light.ambient_ground[0], scene.light.ambient_ground[1], scene.light.ambient_ground[2], 1.0],
        };
        let light_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Light Uniform Buffer"),
            contents: cast_slice(&[light_uniform]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // P1-04: paleta de ossos (24 × mat4 = 1536 B) entregue pelo núcleo. Sem
        // ela o WGSL não compila (binding 5/2) e o vértice ficaria na pose de
        // repouso.
        let bone_palette = bone_palette_uniform(scene);
        let bones_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bone Palette Uniform Buffer"),
            contents: cast_slice(&[bone_palette]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // 5. Setup Output Download Buffer
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Download Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // 6. Command Encoding: Compute Pass followed by Render Pass
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Morph Compute & Render Command Encoder"),
        });

        // Dispatch Sparse Morph Compute Pass FIRST
        self.dispatch_sparse_morphs(
            &mut encoder,
            &header_buffer,
            &base_buffer,
            &deltas_buffer,
            &channels_buffer,
            &morphed_vertex_buffer,
            base_vertices.len() as u32,
        );

        let mut draw_calls = 0;
        let mut triangle_count = 0;

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Anime Cel Render Pass with Morphed Vertices"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: scene.background_color[0] as f64,
                            g: scene.background_color[1] as f64,
                            b: scene.background_color[2] as f64,
                            a: scene.background_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    // Fase 2 (#53): a resolve da profundidade MSAA→1× é manual
                    // (copy_texture_to_texture) — o wgpu deste projeto não tem
                    // o campo de depth resolve no attachment.
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            for node in &scene.nodes {
                if !node.visible {
                    continue;
                }
                if let Some(mesh) = &node.mesh {
                    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                        continue;
                    }

                    // P1-03: valida a malha **antes** de criar VBO/IBO. Uma
                    // malha reprovada vira diagnóstico com código do contrato e
                    // o nó é pulado (sem buffer torto, sem erro do wgpu).
                    if let Err(problem) = mesh_validation::validate_mesh(mesh) {
                        diagnostics::report_with_detail(
                            "mesh_invalid",
                            "malha reprovada antes de criar buffers de GPU",
                            Some(problem.message()),
                        );
                        continue;
                    }

                    // P1-05: model/normal do nó atual (era o transform de nodes[0]).
                    let model_mat = node.transform.to_matrix();
                    let normal_mat = model_mat.inverse().transpose();
                    let camera_uniform = CameraUniform {
                        view_proj: camera_view_proj,
                        camera_pos,
                        model: model_mat.to_cols_array(),
                        normal_mat: normal_mat.to_cols_array(),
                    };
                    let camera_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Camera Uniform Buffer (node)"),
                        contents: cast_slice(&[camera_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });

                    let mat = node.material.clone().unwrap_or_default();
                    let mat_uniform = MaterialUniform::from(&mat);
                    let mat_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Material Uniform Buffer"),
                        contents: cast_slice(&[mat_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                    // P0-04/09: outline uniform parity with viewport (depthBias/opacity/smoothness, was 0.0/0.0)
                    let aspect = width as f32 / height.max(1) as f32;
                    let outline_uniform = OutlineUniform {
                        color: mat.outline_color,
                        params: [mat.outline_width, aspect, mat.outline_depth_bias, mat.outline_opacity],
                        params2: [mat.outline_smoothness, 0.0, 0.0, 0.0],
                    };
                    let outline_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Outline Uniform Buffer"),
                        contents: cast_slice(&[outline_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                    let cel_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Cel Bind Group"),
                        layout: &self.cel_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: camera_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: light_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: mat_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&self.toon_ramp_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(&self.toon_ramp_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: self.cel_skin_binding,
                                resource: bones_buffer.as_entire_binding(),
                            },
                            // Fase 2 (#18): slots MToon ancoram o neutro 1x1 branco;
                            // o shader só amostra o slot habilitado (params5/6).
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 9,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 10,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 11,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 12,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 13,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 14,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 15,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                            // Fase 2 (#17): SDF facial — neutro 1x1 (R=1 → fator 0).
                            wgpu::BindGroupEntry {
                                binding: 16,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 17,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                        ],
                    });

                    let outline_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Outline Bind Group"),
                        layout: &self.outline_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: camera_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: outline_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: self.outline_skin_binding,
                                resource: bones_buffer.as_entire_binding(),
                            },
                            // Fase 2 (#18): mapa de espessura do contorno (neutro 1x1).
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&self.mtoon_neutral_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(&self.mtoon_neutral_sampler),
                            },
                        ],
                    });

                    let i_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Index Buffer"),
                        contents: cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });

                    let v_buffer = if mesh.vertices.len() == base_vertices.len() {
                        None
                    } else {
                        Some(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Static Vertex Buffer"),
                            contents: cast_slice(&mesh.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }))
                    };

                    let active_v_slice = match &v_buffer {
                        Some(buf) => buf.slice(..),
                        None => morphed_vertex_buffer.slice(..),
                    };

                    // A ordem dos passes vem do contrato (outline → cel), a mesma do viewport
                    render_pass.set_vertex_buffer(0, active_v_slice);
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    for pass_name in contract::render_pass_order() {
                        match pass_name {
                            "outline" => {
                                render_pass.set_pipeline(&self.outline_pipeline);
                                render_pass.set_bind_group(0, &outline_bind_group, &[]);
                            }
                            "cel" => {
                                render_pass.set_pipeline(&self.cel_pipeline);
                                render_pass.set_bind_group(0, &cel_bind_group, &[]);
                            }
                            // Fase 2 (#53): passe de pós — roda UMA vez depois
                            // do laço de nós (não por nó).
                            "dof_post" => {
                                continue;
                            }
                            other => {
                                // P1-01: passe sem pipeline vira diagnóstico
                                // observável (era `debug_assert!`).
                                contract::report(format!(
                                    "passe '{}' do render contract não tem pipeline no headless",
                                    other
                                ));
                                continue;
                            }
                        }
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                        draw_calls += 1;
                    }

                    triangle_count += mesh.indices.len() / 3;
                }
            }
        }

        // Resolve manual da profundidade (MSAA → 1×): o wgpu deste projeto não
        // expõe depth resolve no attachment; o copy é o padrão do próprio
        // resolve de cor e só entra quando o DoF pediu a textura 1×.
        if let Some(dof_depth_texture) = &dof_depth_texture {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: dof_depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        // Fase 2 (#53): Anime Bokeh DoF — idêntico ao do render_scene.
        if dof_enabled {
            if let (Some(scene_view_for_dof), Some(dof_depth_for_dof)) =
                (scene_view.as_ref(), dof_depth_view.as_ref())
            {
            let dof_uniform = crate::uniforms::DofUniform::from_settings(
                scene.dof.focus_distance,
                scene.dof.f_number,
                scene.dof.bokeh_shape,
                scene.dof.focal_mm,
                scene.dof.max_radius_px,
                width,
                height,
                scene.camera.z_near,
                scene.camera.z_far,
            );
            let dof_uniform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("DofUniform (Morphed)"),
                contents: bytemuck::bytes_of(&dof_uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let dof_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("DoF Bind Group (Morphed)"),
                layout: &self.dof_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: dof_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(scene_view_for_dof),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(dof_depth_for_dof),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.dof_nearest_sampler),
                    },
                ],
            });
            let mut dof_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Anime Bokeh DoF Pass (Morphed)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(ref pipeline) = self.dof_pipeline {
                dof_pass.set_pipeline(pipeline);
                dof_pass.set_bind_group(0, &dof_bind_group, &[]);
                dof_pass.draw(0..3, 0..1);
                draw_calls += 1;
            } else {
                diagnostics::report(
                    "dof_unavailable",
                    "DoF desabilitado: backend gráfico não suporta leitura de textura de profundidade",
                );
            }
            } else {
                diagnostics::report(
                    "dof_unavailable",
                    "DoF habilitado sem as texturas intermediárias 1× — passe pulado neste frame",
                );
            }
        }

        // 7. Copy rendered color texture to download buffer
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        // 8. Map buffer and read image pixels
        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);

        rx.recv()
            .context("Channel receive failed while waiting for GPU buffer mapping")
            .and_then(|result| result.context("Failed to map GPU buffer for image extraction"))
            .map_err(|error| {
                diagnostics::report_with_detail(
                    "readback_failed",
                    "falha ao mapear o buffer de imagem na GPU",
                    Some(error.to_string()),
                );
                error
            })?;

        let padded_data = buffer_slice.get_mapped_range();
        let mut unpadded_pixels = Vec::with_capacity((width * height * 4) as usize);

        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            unpadded_pixels.extend_from_slice(&padded_data[start..end]);
        }

        drop(padded_data);
        output_buffer.unmap();

        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, unpadded_pixels)
            .context("Failed to construct ImageBuffer from raw unpadded pixels")
            .map_err(|error| {
                diagnostics::report_with_detail(
                    "readback_failed",
                    "pixels lidos da GPU não formam uma imagem válida",
                    Some(error.to_string()),
                );
                error
            })?;

        let elapsed = start_time.elapsed().as_secs_f64() * 1000.0;

        let metrics = RenderMetrics {
            render_time_ms: elapsed,
            draw_calls,
            triangle_count,
            adapter_name: self.adapter_info.name.clone(),
            backend: format!("{:?}", self.adapter_info.backend),
        };

        Ok((image, metrics))
    }
}

#[cfg(test)]
mod tests {


    use super::*;
    use anigo_core::scene::{Scene, StylizedMaterial};

    #[test]
    fn test_headless_renderer_initialization_and_render() {
        // O coletor de diagnósticos é global: segura o mesmo cadeado dos testes
        // de `diagnostics` enquanto este teste reporta `device_unavailable`.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let renderer = HeadlessRenderer::new().await;
            if let Ok(renderer) = renderer {
                assert!(!renderer.adapter_info.name.is_empty());

                let mut scene = Scene::default();
                let mat = StylizedMaterial {
                    shadow_threshold: 0.55,
                    hue_shift: -20.0,
                    toon_steps: 1.0,
                    ..StylizedMaterial::default()
                };
                scene.update_material_for_all(mat);

                let (img, metrics) = renderer.render_scene(&scene, 128, 128).await.unwrap();
                assert_eq!(img.width(), 128);
                assert_eq!(img.height(), 128);
                assert!(metrics.triangle_count > 0);
                assert!(metrics.draw_calls >= 1);
            }
        });
    }

    #[test]
    fn test_headless_renderer_canonical_base_mesh() {
        // O coletor de diagnósticos é global: segura o mesmo cadeado dos testes
        // de `diagnostics` enquanto este teste reporta `device_unavailable`.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let renderer = HeadlessRenderer::new().await;
            if let Ok(renderer) = renderer {
                for gender in [anigo_core::mesh::BaseGender::Male, anigo_core::mesh::BaseGender::Female] {
                    let mesh = anigo_core::mesh::Mesh::create_canonical_base(gender);
                    let mut scene = Scene::new_empty();
                    let node = anigo_core::scene::SceneNode::new("base", "CanonicalBase").with_mesh(mesh);
                    scene.add_node(node);

                    let (img, metrics) = renderer.render_scene(&scene, 128, 128).await.unwrap();
                    assert_eq!(img.width(), 128);
                    assert_eq!(img.height(), 128);
                    assert_eq!(metrics.triangle_count, 20640 / 3);
                    assert_eq!(metrics.draw_calls, 2);
                }
            }
        });
    }

    #[test]
    fn test_headless_renderer_sparse_morph_compute_matches_cpu() {
        // O coletor de diagnósticos é global: segura o mesmo cadeado dos testes
        // de `diagnostics` enquanto este teste reporta `device_unavailable`.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let renderer = HeadlessRenderer::new().await;
            if let Ok(renderer) = renderer {
                use anigo_core::morph::{SparseMorphDelta, SparseMorphSet};

                let mut morph_set = SparseMorphSet::new();
                morph_set.add_target(
                    "chin_forward",
                    vec![
                        SparseMorphDelta::new(0, [0.0, 0.0, 0.15], [0.0, 0.0, 0.3]),
                        SparseMorphDelta::new(2, [0.0, 0.05, 0.10], [0.0, 0.2, 0.2]),
                    ],
                );
                morph_set.add_target(
                    "jaw_width",
                    vec![
                        SparseMorphDelta::new(1, [0.08, 0.0, 0.0], [0.2, 0.0, 0.0]),
                        SparseMorphDelta::new(2, [-0.08, 0.0, 0.0], [-0.2, 0.0, 0.0]),
                    ],
                );

                let base_vertices = vec![
                    Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
                    Vertex::new([1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 0.0]),
                    Vertex::new([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.5]),
                    Vertex::new([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], [0.5, 0.5]),
                ];

                let weights = [0.8, 0.6];
                let (header, channels, deltas) =
                    morph_set.pack_active_channels(&weights, base_vertices.len() as u32, 1e-6);

                let mut cpu_vertices = vec![Vertex::new([0.0; 3], [0.0; 3], [0.0; 2]); base_vertices.len()];
                morph_set.apply_cpu(&weights, &base_vertices, &mut cpu_vertices);

                let gpu_vertices = renderer
                    .execute_and_read_sparse_morphs(&header, &base_vertices, &deltas, &channels)
                    .await
                    .expect("GPU compute pass for sparse morphs must succeed");

                assert_eq!(gpu_vertices.len(), base_vertices.len());
                for (i, (gpu_v, cpu_v)) in gpu_vertices.iter().zip(cpu_vertices.iter()).enumerate() {
                    for axis in 0..3 {
                        assert!(
                            (gpu_v.position[axis] - cpu_v.position[axis]).abs() < 1e-4,
                            "Vertex {} axis {} mismatch: GPU {} vs CPU {}",
                            i,
                            axis,
                            gpu_v.position[axis],
                            cpu_v.position[axis]
                        );
                        assert!(
                            (gpu_v.normal[axis] - cpu_v.normal[axis]).abs() < 1e-4,
                            "Vertex {} normal axis {} mismatch: GPU {} vs CPU {}",
                            i,
                            axis,
                            gpu_v.normal[axis],
                            cpu_v.normal[axis]
                        );
                    }
                    assert_eq!(gpu_v.uv, cpu_v.uv);
                    assert_eq!(gpu_v.color, cpu_v.color);
                    assert_eq!(gpu_v.joints, cpu_v.joints);
                    assert_eq!(gpu_v.weights, cpu_v.weights);
                }
            }
        });
    }

    #[test]
    fn test_headless_renderer_render_scene_with_sparse_morphs() {
        // O coletor de diagnósticos é global: segura o mesmo cadeado dos testes
        // de `diagnostics` enquanto este teste reporta `device_unavailable`.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let renderer = HeadlessRenderer::new().await;
            if let Ok(renderer) = renderer {
                use anigo_core::morph::{SparseMorphDelta, SparseMorphSet};
                use anigo_core::mesh::Mesh;
                use anigo_core::scene::{Scene, SceneNode};

                let cube = Mesh::create_cube(1.0);
                let base_vertices = cube.vertices.clone();

                let mut morph_set = SparseMorphSet::new();
                morph_set.add_target(
                    "bulge",
                    vec![
                        SparseMorphDelta::new(0, [0.2, 0.2, 0.2], [0.0, 1.0, 0.0]),
                        SparseMorphDelta::new(1, [0.2, -0.2, 0.2], [0.0, -1.0, 0.0]),
                    ],
                );

                let weights = [0.75];
                let (header, channels, deltas) =
                    morph_set.pack_active_channels(&weights, base_vertices.len() as u32, 1e-6);

                let mut scene = Scene::new_empty();
                let node = SceneNode::new("morphed_cube", "MorphedCube").with_mesh(cube);
                scene.add_node(node);

                let (img, metrics) = renderer
                    .render_scene_with_sparse_morphs(&scene, &header, &base_vertices, &deltas, &channels, 128, 128)
                    .await
                    .expect("Offscreen render with GPU sparse morph compute pass must succeed");

                assert_eq!(img.width(), 128);
                assert_eq!(img.height(), 128);
                assert_eq!(metrics.draw_calls, 2);
                assert_eq!(metrics.triangle_count, 12);
            }
        });
    }
}
