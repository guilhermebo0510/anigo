use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use bytemuck::cast_slice;
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

use anigo_core::{Scene, Vertex};
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

        // Cel Bind Group Layout (Camera, Light, Material)
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
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3, // normal
                },
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 3]>() * 2) as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2, // uv
                },
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 3]>() * 2 + std::mem::size_of::<[f32; 2]>()) as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4, // anime vertex attr (AO, shadow, outline, spec)
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                format: wgpu::TextureFormat::Depth32Float,
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
        })
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = self.device.create_texture(&depth_desc);
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. Setup Camera Uniform
        let mut camera_copy = scene.camera.clone();
        camera_copy.aspect = width as f32 / height as f32;
        let view_proj = camera_copy.build_view_projection_matrix();

        let camera_uniform = CameraUniform {
            view_proj: view_proj.to_cols_array(),
            camera_pos: [camera_copy.eye.x, camera_copy.eye.y, camera_copy.eye.z, 1.0],
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
                1.0,
            ],
        };
        let light_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Light Uniform Buffer"),
            contents: cast_slice(&[light_uniform]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // 4. Setup Output Buffer with row pitch alignment (wgpu requires 256 byte alignment)
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
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

                    // Material Uniform
                    let mat_uniform = MaterialUniform {
                        base_color: mat.base_color,
                        shade_color: mat.shade_color,
                        params: [mat.shadow_threshold, mat.shadow_smoothness, 0.0, 0.0],
                    };
                    let mat_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Material Uniform Buffer"),
                        contents: cast_slice(&[mat_uniform]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });

                    // Outline Uniform
                    let outline_uniform = OutlineUniform {
                        color: mat.outline_color,
                        params: [mat.outline_width, 1.0, 0.0, 0.0],
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

                    // Pass 1: Inverted Hull Outline (renders backfaces first or with depth test)
                    render_pass.set_pipeline(&self.outline_pipeline);
                    render_pass.set_bind_group(0, &outline_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, v_buffer.slice(..));
                    render_pass.set_index_buffer(i_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    draw_calls += 1;

                    // Pass 2: Cel-Shading Surfaces (renders front faces with toon shading)
                    render_pass.set_pipeline(&self.cel_pipeline);
                    render_pass.set_bind_group(0, &cel_bind_group, &[]);
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
}
