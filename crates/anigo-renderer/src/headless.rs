use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use bytemuck::cast_slice;
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use anigo_core::{MorphChannel, Scene, SparseMorphDelta, SparseMorphHeader, Vertex};
use glam::Mat4;
use crate::uniforms::{CameraUniform, LightUniform, MaterialUniform, OutlineUniform};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMetrics {
    pub render_time_ms: f64,
    pub draw_calls: u32,
    pub triangle_count: usize,
    pub adapter_name: String,
    pub backend: String,
}

pub struct HeadlessRenderer {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
    cel_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    cel_bind_group_layout: wgpu::BindGroupLayout,
    outline_bind_group_layout: wgpu::BindGroupLayout,
    pub morph_compute_pipeline: wgpu::ComputePipeline,
    pub morph_bind_group_layout: wgpu::BindGroupLayout,
    pub toon_ramp_view: wgpu::TextureView,
    pub toon_ramp_sampler: wgpu::Sampler,
}

impl HeadlessRenderer {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .context("Failed to find suitable GPU adapter for ANIGO engine")?;

        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
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
            .context("Failed to create wgpu device and queue")?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // 1D/2D Toon Ramp Texture (256x4) with subpixel anti-aliased ramps
        let ramp_desc = wgpu::TextureDescriptor {
            label: Some("Toon Ramp 2D Texture"),
            size: wgpu::Extent3d {
                width: 256,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let toon_ramp_texture = device.create_texture(&ramp_desc);
        // P0-04: unified 256x4 ramp with TS viewport (was 85/135 vs 89/166, 64/120/180 vs 64/128/192)
        let mut ramp_data = Vec::with_capacity(256 * 4 * 4);
        for row in 0..4 {
            for col in 0..256 {
                let u = col as f32 / 255.0;
                let val = match row {
                    0 => col as u8,
                    1 => if u >= 0.5 { 255 } else { 0 },
                    2 => if u < 0.35 { 0 } else if u < 0.65 { 128 } else { 255 },
                    _ => if u < 0.25 { 0 } else if u < 0.50 { 89 } else if u < 0.75 { 179 } else { 255 },
                };
                ramp_data.extend_from_slice(&[val, val, val, 255]);
            }
        }
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
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(4),
            },
            wgpu::Extent3d {
                width: 256,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
        let toon_ramp_view = toon_ramp_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let toon_ramp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Toon Ramp Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Load Shaders
        let cel_shader_src = include_str!("../shaders/cel_shading.wgsl");
        let outline_shader_src = include_str!("../shaders/inverted_hull.wgsl");

        let cel_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cel Shading Shader"),
            source: wgpu::ShaderSource::Wgsl(cel_shader_src.into()),
        });

        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Inverted Hull Shader"),
            source: wgpu::ShaderSource::Wgsl(outline_shader_src.into()),
        });

        // Cel Bind Group Layout (Camera, Light, Material, ToonRampTexture, ToonRampSampler)
        let cel_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cel Bind Group Layout"),
            entries: &[
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
            ],
        });

        // Outline Bind Group Layout (Camera, OutlineUniform)
        let outline_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Outline Bind Group Layout"),
            entries: &[
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
            ],
        });

        // Vertex buffer layout
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3, // position
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3, // normal
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2, // uv
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4, // anime vertex attr (AO, shadow, outline, spec)
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint16x4, // joints: [u16; 4]
                },
                wgpu::VertexAttribute {
                    offset: 56,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4, // weights: [f32; 4]
                },
            ],
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
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &cel_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm, // P0-05: parity non-sRGB (was Rgba8UnormSrgb)
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
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
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm, // P0-05: parity non-sRGB (was Rgba8UnormSrgb)
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Front), // Key anime technique: cull front, render back extruded
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Sparse Morph Target Compute Pipeline
        let morph_shader_src = include_str!("../shaders/morph_sparse_compute.wgsl");
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
            entry_point: Some("cs_accumulate_morphs"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            adapter_info,
            cel_pipeline,
            outline_pipeline,
            cel_bind_group_layout,
            outline_bind_group_layout,
            morph_compute_pipeline,
            morph_bind_group_layout,
            toon_ramp_view,
            toon_ramp_sampler,
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
            .context("Channel receive failed while waiting for GPU morph buffer mapping")?
            .context("Failed to map GPU buffer for morph verification")?;

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

        // 1. Setup Render Target and Depth Texture
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Color Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, // P0-05: parity non-sRGB (was Rgba8UnormSrgb)
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let color_texture = self.device.create_texture(&texture_desc);
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let depth_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Depth Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = self.device.create_texture(&depth_desc);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. Setup Camera Uniform
        let mut camera_copy = scene.camera.clone();
        camera_copy.aspect = width as f32 / height as f32;
        let view_proj = camera_copy.build_view_projection_matrix();

        // P2-14 model matrix per node (was identity)
        let model_mat = scene.nodes.first().map(|n| n.transform.to_matrix()).unwrap_or(glam::Mat4::IDENTITY);
        let normal_mat = model_mat.inverse().transpose();
        let camera_uniform = CameraUniform {
            view_proj: view_proj.to_cols_array(),
            camera_pos: [camera_copy.eye.x, camera_copy.eye.y, camera_copy.eye.z, 1.0],
            model: model_mat.to_cols_array(),
            normal_mat: normal_mat.to_cols_array(),
        };
        let camera_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Uniform Buffer"),
            contents: cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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
                    view: &color_view,
                    resolve_target: None,
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

                    // P0-04: order unified to viewport cel→outline (was outline→cel diverging)
                    // Pass 1: Cel-Shading Surfaces (front faces)
                    render_pass.set_pipeline(&self.cel_pipeline);
                    render_pass.set_bind_group(0, &cel_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, v_buffer.slice(..));
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    draw_calls += 1;

                    // Pass 2: Inverted Hull Outline (backfaces extruded)
                    render_pass.set_pipeline(&self.outline_pipeline);
                    render_pass.set_bind_group(0, &outline_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, v_buffer.slice(..));
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    draw_calls += 1;

                    triangle_count += mesh.indices.len() / 3;
                }
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
            .context("Channel receive failed while waiting for GPU buffer mapping")?
            .context("Failed to map GPU buffer for image extraction")?;

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
            .context("Failed to construct ImageBuffer from raw unpadded pixels")?;

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

        // 1. Setup Render Target and Depth Texture
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Color Texture (Morphed)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, // P0-05: parity non-sRGB (was Rgba8UnormSrgb)
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let color_texture = self.device.create_texture(&texture_desc);
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let depth_desc = wgpu::TextureDescriptor {
            label: Some("Offscreen Depth Texture (Morphed)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = self.device.create_texture(&depth_desc);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

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

        // P2-14 model matrix per node (was identity)
        let model_mat = scene.nodes.first().map(|n| n.transform.to_matrix()).unwrap_or(glam::Mat4::IDENTITY);
        let normal_mat = model_mat.inverse().transpose();
        let camera_uniform = CameraUniform {
            view_proj: view_proj.to_cols_array(),
            camera_pos: [camera_copy.eye.x, camera_copy.eye.y, camera_copy.eye.z, 1.0],
            model: model_mat.to_cols_array(),
            normal_mat: normal_mat.to_cols_array(),
        };
        let camera_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Uniform Buffer"),
            contents: cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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
                    view: &color_view,
                    resolve_target: None,
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

                    // P0-04: order unified to viewport cel→outline
                    // Pass 1: Cel-Shading Surfaces
                    render_pass.set_pipeline(&self.cel_pipeline);
                    render_pass.set_bind_group(0, &cel_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, active_v_slice);
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    draw_calls += 1;

                    // Pass 2: Inverted Hull Outline
                    render_pass.set_pipeline(&self.outline_pipeline);
                    render_pass.set_bind_group(0, &outline_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, active_v_slice);
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    draw_calls += 1;

                    triangle_count += mesh.indices.len() / 3;
                }
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
            .context("Channel receive failed while waiting for GPU buffer mapping")?
            .context("Failed to map GPU buffer for image extraction")?;

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
            .context("Failed to construct ImageBuffer from raw unpadded pixels")?;

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
