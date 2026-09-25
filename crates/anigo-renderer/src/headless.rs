use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use bytemuck::cast_slice;
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use anigo_core::{MorphChannel, Scene, SparseMorphDelta, SparseMorphHeader, Vertex};
use crate::diagnostics;
use crate::device_recovery::{DeviceRecreationReport, PresentationMode, UncapturedErrorBus};
use crate::mesh_validation;
use crate::render_contract as contract;
use crate::render_graph::{ExecutionPlan, GraphOverrides, RenderGraph};
use crate::uniforms::{
    BonePaletteUniform, CameraUniform, LightUniform, MaterialUniform, OutlineUniform,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMetrics {
    pub render_time_ms: f64,
    pub draw_calls: u32,
    /// Issue #13: draw calls que o frustum culling suprimiu neste quadro.
    /// `draw_calls` conta só o que de fato entrou no `RenderPass`, então a soma
    /// dos dois é o total de nós desenháveis considerados.
    ///
    /// `serde(default)`: telemetria antiga (sem o campo) continua desserializável.
    #[serde(default)]
    pub culled_draw_calls: u32,
    /// Issue #14: passes executados neste quadro, na ordem do plano
    /// (`depth_prepass` abre quando ligado) — a prova observável do grafo.
    #[serde(default)]
    pub executed_passes: Vec<String>,
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

/// Issue #11: recursos GPU persistentes pertencentes ao device — recriáveis
/// como um bloco inteiro por `recreate_device_and_swapchain` (reinstanciação
/// dos PSOs + re-alocação dos buffers essenciais a partir do contrato
/// canônico, a mesma fonte do snapshot do núcleo).
struct DeviceResources {
    cel_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    /// Issue #14: pipeline só-profundidade do pré-passe (Early-Z).
    depth_prepass_pipeline: wgpu::RenderPipeline,
    cel_bind_group_layout: wgpu::BindGroupLayout,
    outline_bind_group_layout: wgpu::BindGroupLayout,
    morph_compute_pipeline: wgpu::ComputePipeline,
    morph_bind_group_layout: wgpu::BindGroupLayout,
    toon_ramp_view: wgpu::TextureView,
    toon_ramp_sampler: wgpu::Sampler,
    mtoon_neutral_view: wgpu::TextureView,
    mtoon_neutral_sampler: wgpu::Sampler,
    dof_pipeline: Option<wgpu::RenderPipeline>,
    dof_bind_group_layout: wgpu::BindGroupLayout,
    dof_nearest_sampler: wgpu::Sampler,
}

pub struct HeadlessRenderer {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
    /// Issue #11: canal MPSC `on_uncaptured_error` → módulo de diagnósticos.
    error_bus: UncapturedErrorBus,
    /// Issue #11: modo de apresentação explícito (Fifo/Immediate/Mailbox).
    presentation_mode: PresentationMode,
    cel_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    /// Issue #14: pipeline só-profundidade do pré-passe (Early-Z).
    depth_prepass_pipeline: wgpu::RenderPipeline,
    /// Issue #14: reconfiguração do plano vinda do snapshot do núcleo.
    graph_overrides: GraphOverrides,
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

/// Issue #14: recursos de desenho de um nó preparados uma vez por quadro — o
/// pré-passe de profundidade e o passe principal desenham os mesmos buffers
/// (mesmo VBO/IBO, mesmos bind groups).
struct PreparedPassInputs {
    cel_bind_group: wgpu::BindGroup,
    outline_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl HeadlessRenderer {
    /// Solicita um novo adapter + device com os mesmos guardas da criação
    /// inicial (P1-01/P1-02: um pânico do wgpu em runner headless vira
    /// diagnóstico `device_unavailable`, nunca crash de processo).
    ///
    /// Reutilizado por [`Self::recreate_device_and_swapchain`] (issue #11):
    /// perda de GPU, troca dedicada/integrada ou suspensão do SO passam pela
    /// mesma rota, com o mesmo tratamento observável.
    async fn request_device() -> Result<(
        Arc<wgpu::Device>,
        Arc<wgpu::Queue>,
        wgpu::AdapterInfo,
    )> {
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

        // P1-01/P1-02: além do caminho `None`, o wgpu 24 aborta com um pânico
        // **cru** (sem mensagem) quando o runner não tem backend utilizável.
        // O guarda converte o pânico no mesmo `device_unavailable`.
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

        Ok((Arc::new(device), Arc::new(queue), adapter_info))
    }

    /// Constrói todos os recursos GPU persistentes (toon ramp, shader modules,
    /// bind group layouts e pipelines) em um device.
    ///
    /// Issue #11: estar desacoplado da solicitação do device é o que permite a
    /// `recreate_device_and_swapchain` reinstanciar os PSOs e re-alocar os
    /// buffers essenciais a partir do snapshot canônico — bytes de shader e
    /// layout vêm do contrato (a mesma fonte que o núcleo serializa).
    fn build_device_resources(
        device: &Arc<wgpu::Device>,
        queue: &Arc<wgpu::Queue>,
        adapter_info: &wgpu::AdapterInfo,
    ) -> Result<DeviceResources> {
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
                    binding: contract::skinning_binding("cel").unwrap_or(5),
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
                    binding: contract::skinning_binding("outline").unwrap_or(2),
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

        // Issue #14: pré-passe de profundidade — mesmo vértice do cel
        // (`vs_main`, mesmo bind group layout), sem fragmento: só escreve o
        // z-buffer para o Early-Z do passe principal. `Less` estrito contra o
        // clear 1.0; o cel (`less_equal`) redesenha a mesma geometria e passa
        // com a mesma profundidade (sem z-fighting, sem pixel diferente).
        let depth_prepass_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Depth Prepass Pipeline Layout"),
                bind_group_layouts: &[&cel_bind_group_layout],
                push_constant_ranges: &[],
            });
        let depth_prepass_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Depth Prepass Pipeline"),
                layout: Some(&depth_prepass_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &cel_shader,
                    entry_point: Some(cel_pass_spec.vertex_entry.unwrap_or("vs_main")),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vertex_layout),
                },
                fragment: None,
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
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
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

        Ok(DeviceResources {
            cel_pipeline,
            outline_pipeline,
            depth_prepass_pipeline,
            cel_bind_group_layout,
            outline_bind_group_layout,
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

    pub async fn new() -> Result<Self> {
        // P1-02: um contrato em drift aparece na telemetria antes de qualquer
        // coisa ser criada (o resto do quadro fica suspeito).
        diagnostics::report_contract_health();

        let (device, queue, adapter_info) = Self::request_device().await?;

        // Issue #11: erros `on_uncaptured_error` são interceptados pelo canal
        // MPSC estruturado (nada de pânico no caminho crítico); o dreno para o
        // módulo de diagnósticos acontece a cada quadro, no `render_scene`.
        let error_bus = UncapturedErrorBus::new();
        error_bus.install_on(&device);

        // P1-04: o binding da paleta de skinning vem do contrato (5 no cel, 2 no
        // outline) — contrato e WGSL mudam juntos, nunca um literal solto.
        let cel_skin_binding = contract::skinning_binding("cel").unwrap_or(5);
        let outline_skin_binding = contract::skinning_binding("outline").unwrap_or(2);

        let resources = Self::build_device_resources(&device, &queue, &adapter_info)?;

        Ok(Self {
            device,
            queue,
            adapter_info,
            error_bus,
            presentation_mode: PresentationMode::Fifo,
            cel_pipeline: resources.cel_pipeline,
            outline_pipeline: resources.outline_pipeline,
            depth_prepass_pipeline: resources.depth_prepass_pipeline,
            graph_overrides: GraphOverrides::default(),
            cel_bind_group_layout: resources.cel_bind_group_layout,
            outline_bind_group_layout: resources.outline_bind_group_layout,
            cel_skin_binding,
            outline_skin_binding,
            morph_compute_pipeline: resources.morph_compute_pipeline,
            morph_bind_group_layout: resources.morph_bind_group_layout,
            toon_ramp_view: resources.toon_ramp_view,
            toon_ramp_sampler: resources.toon_ramp_sampler,
            mtoon_neutral_view: resources.mtoon_neutral_view,
            mtoon_neutral_sampler: resources.mtoon_neutral_sampler,
            dof_pipeline: resources.dof_pipeline,
            dof_bind_group_layout: resources.dof_bind_group_layout,
            dof_nearest_sampler: resources.dof_nearest_sampler,
        })
    }

    /// Issue #11 — recuperação de perda de device (device lost / crash recovery).
    ///
    /// Re-solicita adapter e device, reinstancia os PSOs e re-aloca os buffers
    /// essenciais a partir do snapshot canônico (contrato de render + shaders
    /// embutidos: a mesma fonte do estado serializado pelo núcleo). O estado do
    /// personagem — morphs, transforms, materiais — vive no `ProjectState` e
    /// desce novamente pela `Scene` no próximo `render_scene`, então a
    /// recuperação não perde nenhum dado do usuário.
    ///
    /// Chamada pelo Tauri quando um frame falha e pelo teste de falha forçada
    /// (aceitação #1: sem crash do processo).
    pub async fn recreate_device_and_swapchain(&mut self) -> Result<DeviceRecreationReport> {
        let started = Instant::now();
        let previous_adapter = self.adapter_info.name.clone();
        self.error_bus.drain_to_diagnostics();
        diagnostics::report_with_detail(
            "device_lost",
            "recriando device wgpu e pipelines gráficos",
            Some(format!("adapter anterior: {previous_adapter}")),
        );

        let (device, queue, adapter_info) = Self::request_device().await?;
        self.error_bus.install_on(&device);
        let resources = Self::build_device_resources(&device, &queue, &adapter_info)?;

        self.device = device;
        self.queue = queue;
        self.adapter_info = adapter_info.clone();
        self.cel_pipeline = resources.cel_pipeline;
        self.outline_pipeline = resources.outline_pipeline;
        self.depth_prepass_pipeline = resources.depth_prepass_pipeline;
        self.cel_bind_group_layout = resources.cel_bind_group_layout;
        self.outline_bind_group_layout = resources.outline_bind_group_layout;
        self.morph_compute_pipeline = resources.morph_compute_pipeline;
        self.morph_bind_group_layout = resources.morph_bind_group_layout;
        self.toon_ramp_view = resources.toon_ramp_view;
        self.toon_ramp_sampler = resources.toon_ramp_sampler;
        self.mtoon_neutral_view = resources.mtoon_neutral_view;
        self.mtoon_neutral_sampler = resources.mtoon_neutral_sampler;
        self.dof_pipeline = resources.dof_pipeline;
        self.dof_bind_group_layout = resources.dof_bind_group_layout;
        self.dof_nearest_sampler = resources.dof_nearest_sampler;

        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        diagnostics::report_with_detail(
            "device_recreated",
            "device recriado; estado do personagem reapresentado a partir do snapshot canônico",
            Some(format!("adapter: {} em {duration_ms:.1} ms", adapter_info.name)),
        );

        Ok(DeviceRecreationReport {
            adapter_name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend),
            duration_ms,
        })
    }

    /// Issue #11: modo de apresentação explícito (`Fifo` = VSync ligado,
    /// `Immediate` = menor latência, `Mailbox` quando suportado pela GPU).
    ///
    /// O headless renderiza offscreen (sem surface), então o modo é parte do
    /// contrato de device — propagado ao status/telemetria e espelhado pelo
    /// viewport TypeScript, que é quem efetivamente configura o `presentMode`.
    pub fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.presentation_mode = mode;
    }

    /// Modo de apresentação configurado (status/telemetria).
    pub fn presentation_mode(&self) -> PresentationMode {
        self.presentation_mode
    }

    /// Issue #14: overrides do render graph vindos do snapshot do núcleo
    /// (ordem/ativação dos passes + pré-passe de profundidade).
    ///
    /// O Tauri chama a cada quadro antes de renderizar; o plano é remontado no
    /// `render_scene` a partir destes overrides sobre a topologia do contrato.
    pub fn set_graph_overrides(&mut self, overrides: GraphOverrides) {
        self.graph_overrides = overrides;
    }

    /// Overrides configurados (status/telemetria).
    pub fn graph_overrides(&self) -> &GraphOverrides {
        &self.graph_overrides
    }

    /// Issue #14: plano de execução do quadro — overrides do snapshot sobre a
    /// topologia do contrato. Um plano inválido (nome estranho, tudo desligado)
    /// nunca derruba o quadro: cai na ordem do contrato com diagnóstico.
    fn graph_execution_plan(&self) -> ExecutionPlan {
        Self::plan_for_overrides(&self.graph_overrides)
    }

    /// Núcleo puro do planejamento (testável sem GPU).
    fn plan_for_overrides(overrides: &GraphOverrides) -> ExecutionPlan {
        RenderGraph::from_contract()
            .build(overrides)
            .unwrap_or_else(|error| {
                diagnostics::report_with_detail(
                    "render_plan_fallback",
                    "overrides do render graph inválidos; usando a ordem do contrato",
                    Some(error.to_string()),
                );
                ExecutionPlan {
                    order: contract::render_pass_order()
                        .into_iter()
                        .map(|name| name.to_string())
                        .collect(),
                    slots: std::collections::BTreeMap::new(),
                    slot_count: 0,
                }
            })
    }

    /// Drena o canal MPSC de erros de device para o módulo de diagnósticos
    /// (uma vez por quadro: o callback do wgpu só `send` no canal).
    fn drain_device_errors(&self) {
        self.error_bus.drain_to_diagnostics();
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

    /// Issue #14: prepara os recursos de desenho de um nó (uniforms por nó +
    /// bind groups + IBO) uma vez por quadro, para o pré-passe e o passe
    /// principal desenharem os mesmos buffers.
    ///
    /// O VBO entra pronto (`vertex_buffer`) porque a fonte varia: o caminho
    /// principal sobe os vértices da malha, o caminho de morphs usa o buffer
    /// que o compute deformou na GPU.
    #[allow(clippy::too_many_arguments)]
    fn prepare_node_draw(
        &self,
        node: &anigo_core::scene::SceneNode,
        mesh: &anigo_core::mesh::Mesh,
        model_mat: glam::Mat4,
        camera_view_proj: [f32; 16],
        camera_pos: [f32; 4],
        aspect: f32,
        light_buffer: &wgpu::Buffer,
        bones_buffer: &wgpu::Buffer,
        vertex_buffer: wgpu::Buffer,
    ) -> PreparedPassInputs {
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

        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        PreparedPassInputs {
            cel_bind_group,
            outline_bind_group,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
        }
    }

    /// Renders a scene into an offscreen RGBA image buffer and returns the image + performance metrics.
    pub async fn render_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<(ImageBuffer<Rgba<u8>, Vec<u8>>, RenderMetrics)> {
        let start_time = Instant::now();
        self.drain_device_errors();

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

        // Issue #12: matrizes mundiais resolvidas uma vez por quadro
        // (W = W_pai × T_local). Uma cena cuja topologia não resolve — o
        // `ProjectState` já a teria recusado — cai no modo plano: cada nó usa a
        // própria transformação local, sem propagação.
        let world_matrices = scene
            .resolve_world_transforms()
            .unwrap_or_default();

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

        // Issue #14: plano de execução do quadro — overrides do snapshot sobre
        // a topologia do contrato (nunca falha: cai no contrato com diagnóstico).
        let plan = self.graph_execution_plan();
        let depth_prepass = plan.contains(GraphOverrides::DEPTH_PREPASS);

        let mut draw_calls = 0;
        let mut triangle_count = 0;
        // Issue #13: o frustum sai da mesma `view_proj` que o shader recebe, e
        // cada nó vira um volume em espaço de mundo (uma vez por quadro, nunca
        // por vértice). O contador vive fora do escopo do passe porque a
        // telemetria é montada depois dele.
        let frustum = anigo_core::math::Frustum::from_view_projection(&view_proj);
        let mut culled_draw_calls = 0u32;

        // Issue #14: aspecto do quadro (era recalculado por nó) + recursos de
        // desenho preparados uma vez — o pré-passe e o passe principal
        // desenham os mesmos buffers, sem upload duplicado.
        let aspect = width as f32 / height.max(1) as f32;
        let mut prepared: Vec<PreparedPassInputs> = Vec::new();
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
                // Issue #12: a matriz é a **mundial** (pai × local); nós
                // ausentes do mapa caem na transformação local.
                let model_mat = world_matrices
                    .get(&node.id)
                    .copied()
                    .unwrap_or_else(|| node.transform.to_matrix());

                // Issue #13: filtro antes de qualquer trabalho de GPU — um nó
                // fora do cone não cria buffer nem entra no `RenderPass`.
                if let Some(local_bounds) = mesh.local_bounds() {
                    let volume = crate::culling::SceneVolume::from_local(&local_bounds, &model_mat);
                    if !volume.is_visible(&frustum) {
                        culled_draw_calls += 1;
                        continue;
                    }
                }

                triangle_count += mesh.indices.len() / 3;
                let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                prepared.push(self.prepare_node_draw(
                    node,
                    mesh,
                    model_mat,
                    camera_view_proj,
                    camera_pos,
                    aspect,
                    &light_buffer,
                    &bones_buffer,
                    vertex_buffer,
                ));
            }
        }

        // Issue #14: pré-passe de profundidade — `RenderPass` só com o anexo
        // de profundidade (sem alvos de cor), que o passe principal carrega em
        // vez de limpar. Mesmos buffers, mesma geometria: cada fragmento do
        // cel testa contra o z-buffer já resolvido (Early-Z).
        if depth_prepass {
            let mut prepass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Prepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for inputs in &prepared {
                prepass.set_pipeline(&self.depth_prepass_pipeline);
                prepass.set_bind_group(0, &inputs.cel_bind_group, &[]);
                prepass.set_vertex_buffer(0, inputs.vertex_buffer.slice(..));
                prepass.set_index_buffer(inputs.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                prepass.draw_indexed(0..inputs.index_count, 0, 0..1);
                draw_calls += 1;
            }
        }

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
                        // Issue #14: com o pré-passe, o z-buffer resolvido é
                        // carregado em vez de limpo.
                        load: if depth_prepass {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            for inputs in &prepared {
                render_pass.set_vertex_buffer(0, inputs.vertex_buffer.slice(..));
                render_pass.set_index_buffer(inputs.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                // Issue #14: a ordem dos passes vem do plano (contrato +
                // overrides do snapshot), a mesma do viewport.
                for pass_name in &plan.order {
                    // O pré-passe executa no `RenderPass` dedicado acima.
                    if pass_name == GraphOverrides::DEPTH_PREPASS {
                        continue;
                    }
                    match pass_name.as_str() {
                        "outline" => {
                            render_pass.set_pipeline(&self.outline_pipeline);
                            render_pass.set_bind_group(0, &inputs.outline_bind_group, &[]);
                        }
                        "cel" => {
                            render_pass.set_pipeline(&self.cel_pipeline);
                            render_pass.set_bind_group(0, &inputs.cel_bind_group, &[]);
                        }
                        "dof_post" => {
                            continue;
                        }
                        other => {
                            // P1-01: passe sem pipeline vira diagnóstico
                            // observável (era debug_assert!).
                            contract::report(format!(
                                "passe '{}' do plano não tem pipeline no headless",
                                other
                            ));
                            continue;
                        }
                    }
                    render_pass.draw_indexed(0..inputs.index_count, 0, 0..1);
                    draw_calls += 1;
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
            culled_draw_calls,
            executed_passes: plan.order.clone(),
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
        self.drain_device_errors();

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

        // Issue #12: matrizes mundiais resolvidas uma vez por quadro, como no
        // caminho principal (`W = W_pai × T_local`).
        let world_matrices = scene.resolve_world_transforms().unwrap_or_default();

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

        // Issue #14: o mesmo plano do caminho principal — os dois caminhos do
        // headless não podem divergir no que desenham.
        let plan = self.graph_execution_plan();
        let depth_prepass = plan.contains(GraphOverrides::DEPTH_PREPASS);

        let mut draw_calls = 0;
        let mut triangle_count = 0;
        // Issue #13: o mesmo filtro de frustum do caminho principal — os dois
        // caminhos do headless não podem divergir no que desenham.
        let frustum = anigo_core::math::Frustum::from_view_projection(&view_proj);
        let mut culled_draw_calls = 0u32;

        // Issue #14: aspecto do quadro + recursos de desenho preparados uma vez,
        // como no caminho principal.
        let aspect = width as f32 / height.max(1) as f32;
        let mut prepared: Vec<PreparedPassInputs> = Vec::new();
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
                // Issue #12: a matriz é a **mundial** (pai × local); nós
                // ausentes do mapa caem na transformação local.
                let model_mat = world_matrices
                    .get(&node.id)
                    .copied()
                    .unwrap_or_else(|| node.transform.to_matrix());

                // Issue #13: fora do frustum não há buffer nem draw call.
                if let Some(local_bounds) = mesh.local_bounds() {
                    let volume = crate::culling::SceneVolume::from_local(&local_bounds, &model_mat);
                    if !volume.is_visible(&frustum) {
                        culled_draw_calls += 1;
                        continue;
                    }
                }

                triangle_count += mesh.indices.len() / 3;
                // Vértices morfados na GPU quando os tamanhos batem (o compute
                // escreveu no `morphed_vertex_buffer`), senão VBO estático — o
                // handle é clonado, não os bytes.
                let vertex_buffer = if mesh.vertices.len() == base_vertices.len() {
                    morphed_vertex_buffer.clone()
                } else {
                    self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Static Vertex Buffer"),
                        contents: cast_slice(&mesh.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                };
                prepared.push(self.prepare_node_draw(
                    node,
                    mesh,
                    model_mat,
                    camera_view_proj,
                    camera_pos,
                    aspect,
                    &light_buffer,
                    &bones_buffer,
                    vertex_buffer,
                ));
            }
        }

        // Issue #14: pré-passe de profundidade sobre a geometria morfada.
        if depth_prepass {
            let mut prepass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Prepass (Morphed)"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for inputs in &prepared {
                prepass.set_pipeline(&self.depth_prepass_pipeline);
                prepass.set_bind_group(0, &inputs.cel_bind_group, &[]);
                prepass.set_vertex_buffer(0, inputs.vertex_buffer.slice(..));
                prepass.set_index_buffer(inputs.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                prepass.draw_indexed(0..inputs.index_count, 0, 0..1);
                draw_calls += 1;
            }
        }

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
                        // Issue #14: com o pré-passe, o z-buffer resolvido é
                        // carregado em vez de limpo.
                        load: if depth_prepass {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            for inputs in &prepared {
                render_pass.set_vertex_buffer(0, inputs.vertex_buffer.slice(..));
                render_pass.set_index_buffer(inputs.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                // Issue #14: a ordem dos passes vem do plano (contrato +
                // overrides do snapshot), a mesma do viewport.
                for pass_name in &plan.order {
                    // O pré-passe executa no `RenderPass` dedicado acima.
                    if pass_name == GraphOverrides::DEPTH_PREPASS {
                        continue;
                    }
                    match pass_name.as_str() {
                        "outline" => {
                            render_pass.set_pipeline(&self.outline_pipeline);
                            render_pass.set_bind_group(0, &inputs.outline_bind_group, &[]);
                        }
                        "cel" => {
                            render_pass.set_pipeline(&self.cel_pipeline);
                            render_pass.set_bind_group(0, &inputs.cel_bind_group, &[]);
                        }
                        "dof_post" => {
                            continue;
                        }
                        other => {
                            // P1-01: passe sem pipeline vira diagnóstico
                            // observável (era debug_assert!).
                            contract::report(format!(
                                "passe '{}' do plano não tem pipeline no headless",
                                other
                            ));
                            continue;
                        }
                    }
                    render_pass.draw_indexed(0..inputs.index_count, 0, 0..1);
                    draw_calls += 1;
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
            culled_draw_calls,
            executed_passes: plan.order.clone(),
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
    use anigo_core::scene::{Scene, SceneNode, StylizedMaterial};

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
    fn frustum_culling_suppresses_draw_calls_for_offscreen_nodes() {
        // Critério de aceitação #1/#3 do issue #13: um nó fora do cone perde a
        // draw call **na telemetria** (e um nó visível continua desenhando).
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let renderer = match HeadlessRenderer::new().await {
                Ok(renderer) => renderer,
                Err(_) => return, // sem adaptador: o teste de GPU se pula
            };

            let mut scene = Scene::default();
            let visible_node = scene.nodes[0].id.clone();
            let mut far = SceneNode::new("nod_far", "Far Away");
            far.mesh = scene.nodes[0].mesh.clone();
            far.transform.translation = glam::Vec3::new(0.0, 0.0, -100.0);
            scene.nodes.push(far);
            scene.rebuild_children();

            let (_, metrics) = renderer.render_scene(&scene, 64, 64).await.unwrap();
            assert!(
                metrics.culled_draw_calls >= 1,
                "o nó em z = -100 precisa ser recusado pelo frustum (culled = {})",
                metrics.culled_draw_calls
            );
            // O nó visível segue desenhando: o filtro não comeu o personagem.
            assert!(metrics.draw_calls >= 1);

            // A mesma cena sem o nó distante não tem nada para filtrar.
            scene.nodes.retain(|node| node.id == visible_node);
            scene.rebuild_children();
            let (_, clean) = renderer.render_scene(&scene, 64, 64).await.unwrap();
            assert_eq!(clean.culled_draw_calls, 0);
            assert!(clean.draw_calls >= 1);
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

    #[test]
    fn test_headless_recreate_device_and_swapchain_restores_rendering() {
        // Issue #11, aceitações #1 e #2: falha forçada de device não provaca
        // crash, e o estado (Scene do snapshot canônico) é reapresentado nos
        // novos buffers após `recreate_device_and_swapchain`.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let created = HeadlessRenderer::new().await;
            if created.is_err() {
                // Sem adaptador no ambiente: o caminho de erro já é coberto
                // pelos demais testes (skipped por design no CI sem GPU).
                return;
            }
            let mut renderer = created.unwrap();
            renderer.set_presentation_mode(crate::device_recovery::PresentationMode::Fifo);

            // Falha forçada entra pelo MESMO canal MPSC do callback
            // `on_uncaptured_error` (sem device para provocar de verdade).
            renderer.error_bus.inject(
                crate::device_recovery::UncapturedErrorRecord::device_lost(
                    "driver crash simulado (issue #11)",
                ),
            );
            assert_eq!(renderer.error_bus.pending(), 1);

            let report = renderer
                .recreate_device_and_swapchain()
                .await
                .expect("recreate_device_and_swapchain deve re-solicitar adapter/device");
            assert!(!report.adapter_name.is_empty());
            assert!(report.duration_ms >= 0.0);

            // O canal foi drenado para os diagnósticos (código estável).
            let summary = crate::diagnostics::summary();
            assert!(summary.codes.contains(&"device_lost".to_string()));
            assert!(summary.codes.contains(&"device_recreated".to_string()));

            // Aceitação #2: o personagem (Scene do snapshot) renderiza de novo
            // nos buffers re-alocados.
            let scene = Scene::default();
            let (img, metrics) = renderer
                .render_scene(&scene, 64, 64)
                .await
                .expect("render após recriação deve recuperar o estado");
            assert_eq!(img.width(), 64);
            assert!(metrics.draw_calls >= 1);
        });
    }

    #[test]
    fn render_graph_plan_layers_snapshot_overrides_over_the_contract() {
        // Issue #14: sem overrides o plano é a ordem do contrato; o pré-passe
        // ancora no início quando ligado (núcleo puro, sem GPU).
        let plan = HeadlessRenderer::plan_for_overrides(&GraphOverrides::default());
        assert_eq!(
            plan.order,
            vec!["outline".to_string(), "cel".to_string(), "dof_post".to_string()]
        );

        let plan = HeadlessRenderer::plan_for_overrides(&GraphOverrides {
            depth_prepass: true,
            ..GraphOverrides::default()
        });
        assert_eq!(
            plan.order,
            vec![
                "depth_prepass".to_string(),
                "outline".to_string(),
                "cel".to_string(),
                "dof_post".to_string(),
            ]
        );
    }

    #[test]
    fn render_graph_plan_falls_back_with_a_diagnostic_on_unknown_names() {
        // Issue #14: plano inválido nunca derruba o quadro — cai na ordem do
        // contrato com `render_plan_fallback` observável.
        let _guard = crate::diagnostics::test_guard();
        crate::diagnostics::clear();
        let plan = HeadlessRenderer::plan_for_overrides(&GraphOverrides {
            order: vec!["bloom".to_string()],
            ..GraphOverrides::default()
        });
        assert_eq!(
            plan.order,
            vec!["outline".to_string(), "cel".to_string(), "dof_post".to_string()]
        );
        let summary = crate::diagnostics::summary();
        assert!(summary.codes.contains(&"render_plan_fallback".to_string()));
    }

    #[test]
    fn depth_prepass_preserves_pixels_and_reports_executed_passes() {
        // Issue #14, aceitação #2: o pré-passe é otimização pura — os pixels
        // são idênticos com e sem ele, e `executed_passes` prova o plano.
        let _guard = crate::diagnostics::test_guard();
        pollster::block_on(async {
            let mut renderer = match HeadlessRenderer::new().await {
                Ok(renderer) => renderer,
                Err(_) => return, // sem adaptador: o teste de GPU se pula
            };

            let cube = anigo_core::mesh::Mesh::create_cube(1.0);
            let mut scene = Scene::new_empty();
            scene.add_node(SceneNode::new("cube", "Cube").with_mesh(cube));

            let (plain, plain_metrics) = renderer.render_scene(&scene, 64, 64).await.unwrap();
            assert_eq!(
                plain_metrics.executed_passes,
                vec!["outline".to_string(), "cel".to_string(), "dof_post".to_string()]
            );
            assert_eq!(plain_metrics.draw_calls, 2);

            renderer.set_graph_overrides(GraphOverrides {
                depth_prepass: true,
                ..GraphOverrides::default()
            });
            let (prepassed, prepassed_metrics) =
                renderer.render_scene(&scene, 64, 64).await.unwrap();
            assert_eq!(
                prepassed_metrics.executed_passes,
                vec![
                    "depth_prepass".to_string(),
                    "outline".to_string(),
                    "cel".to_string(),
                    "dof_post".to_string(),
                ]
            );
            assert_eq!(prepassed_metrics.draw_calls, 3);
            assert_eq!(
                plain.as_raw(),
                prepassed.as_raw(),
                "o pré-passe não pode mudar um pixel (Early-Z puro)"
            );

            // Desligar o outline some com o passe do plano e da telemetria.
            renderer.set_graph_overrides(GraphOverrides {
                disabled: vec!["outline".to_string()],
                ..GraphOverrides::default()
            });
            let (_, cel_only) = renderer.render_scene(&scene, 64, 64).await.unwrap();
            assert_eq!(
                cel_only.executed_passes,
                vec!["cel".to_string(), "dof_post".to_string()]
            );
            assert_eq!(cel_only.draw_calls, 1);
        });
    }
}
