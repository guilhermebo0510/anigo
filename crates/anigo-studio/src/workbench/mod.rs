//! Workbench: renderer do viewport de edição (equivalente ao "Workbench" do Blender).
//!
//! Desenha grid infinito, malha sólida com luz de estúdio, wireframe/arestas de edição
//! e pontos de vértice em um alvo offscreen `Rgba8Unorm` (MSAA 4×), que o egui exibe
//! como textura. O render final cel-shaded é do `anigo-renderer` (pipeline canônico);
//! este módulo é só a ferramenta de edição — por isso não herda o contrato do render.

pub mod mesh_data;
pub mod sculpt_mesh;

use crate::viewport::OrbitCamera;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use glam::{Mat3, Mat4, Vec2};
pub use mesh_data::{build_draw_data, build_draw_data_eval, heat_color, BuildOptions, LineInstance, MeshDrawData, NormalOverlay, PointInstance, SolidVertex};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const MSAA_SAMPLES: u32 = 4;
/// Máximo de vistas simultâneas do viewport (layout 2×2).
pub const MAX_VIEW_SLOTS: usize = 4;

/// Fonte WGSL embutida (release). Em dev, `ANIGO_SHADER_DIR` aponta para o diretório
/// de fontes e o hot reload relê do disco.
pub const EMBEDDED_SHADER: &str = include_str!("workbench.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    viewport: [f32; 4],
    params: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    normal0: [f32; 4],
    normal1: [f32; 4],
    normal2: [f32; 4],
    tint: [f32; 4],
}

impl ObjectUniform {
    fn new(model: Mat4, tint: [f32; 4]) -> Self {
        let n = Mat3::from_mat4(model).inverse().transpose();
        // as linhas da normal matrix: dot(row_i, normal)
        let r0 = n.row(0);
        let r1 = n.row(1);
        let r2 = n.row(2);
        Self {
            model: model.to_cols_array_2d(),
            normal0: [r0.x, r0.y, r0.z, 0.0],
            normal1: [r1.x, r1.y, r1.z, 0.0],
            normal2: [r2.x, r2.y, r2.z, 0.0],
            tint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadingMode {
    #[default]
    Solid,
    Wireframe,
    /// Só overlays (grid, arestas, pontos) sobre fundo transparente + profundidade do sólido:
    /// usado por cima do preview cel-shaded do `anigo-renderer`.
    OverlayOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewSettings {
    pub shading: ShadingMode,
    pub show_grid: bool,
    pub xray: bool,
    pub grid_scale: f32,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            shading: ShadingMode::Solid,
            show_grid: true,
            xray: false,
            grid_scale: 1.0,
        }
    }
}

/// Buffers de um objeto na GPU.
pub struct GpuMesh {
    solid: Option<wgpu::Buffer>,
    solid_count: u32,
    lines: Option<wgpu::Buffer>,
    line_count: u32,
    points: Option<wgpu::Buffer>,
    point_count: u32,
    object_buffer: wgpu::Buffer,
    object_bind: wgpu::BindGroup,
}

pub struct DrawItem<'a> {
    pub mesh: &'a GpuMesh,
}

struct Targets {
    size: (u32, u32),
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

struct Pipelines {
    bg: wgpu::RenderPipeline,
    solid: wgpu::RenderPipeline,
    solid_depth: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    point: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
}

pub struct Workbench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_layout: wgpu::BindGroupLayout,
    object_layout: wgpu::BindGroupLayout,
    globals_buffer: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    pipelines: Pipelines,
    /// Um alvo por vista (multi-vista): `slot` escolhe qual os métodos abaixo usam. Fora do multi-vista só existe o 0.
    targets: [Option<Targets>; MAX_VIEW_SLOTS],
    slot: usize,
    /// Objeto vazio: satisfaz o layout do grupo 1 quando não há itens (fundo + grid com a cena vazia).
    empty_object: Option<GpuMesh>,
    pub background_top: [f32; 4],
    pub background_bottom: [f32; 4],
}

fn layout_entry(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Workbench {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, String> {
        let stages = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("workbench globals"),
            entries: &[layout_entry(stages)],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("workbench object"),
            entries: &[layout_entry(stages)],
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("workbench globals"),
            size: std::mem::size_of::<GlobalsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("workbench globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let pipelines = Self::build_pipelines(&device, &globals_layout, &object_layout, EMBEDDED_SHADER)?;
        let mut wb = Self {
            device,
            queue,
            globals_layout,
            object_layout,
            globals_buffer,
            globals_bind,
            pipelines,
            targets: Default::default(),
            slot: 0,
            empty_object: None,
            background_top: [0.215, 0.220, 0.240, 1.0],
            background_bottom: [0.160, 0.163, 0.178, 1.0],
        };
        wb.empty_object = Some(wb.upload(&MeshDrawData::default(), Mat4::IDENTITY, [1.0; 4]));
        Ok(wb)
    }

    /// Recompila os shaders (hot reload). Só troca os pipelines se a validação passar;
    /// erro de WGSL volta como `Err(mensagem)` e o frame segue com os pipelines antigos.
    pub fn reload_shaders(&mut self, source: &str) -> Result<(), String> {
        let new = Self::build_pipelines(&self.device, &self.globals_layout, &self.object_layout, source)?;
        self.pipelines = new;
        Ok(())
    }

    fn build_pipelines(
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
        object_layout: &wgpu::BindGroupLayout,
        source: &str,
    ) -> Result<Pipelines, String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("workbench.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("workbench layout"),
            bind_group_layouts: &[Some(globals_layout), Some(object_layout)],
            immediate_size: 0,
        });

        let multisample = wgpu::MultisampleState {
            count: MSAA_SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let alpha = wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        };
        let opaque = wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let depth_only = wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        };
        let depth = |write: bool, compare: wgpu::CompareFunction, bias: wgpu::DepthBiasState| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias,
            })
        };
        let no_bias = wgpu::DepthBiasState::default();
        let tri = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        };

        let solid_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32];
        let point_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32];

        let solid_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SolidVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &solid_attrs,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attrs,
        })];
        let point_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PointInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &point_attrs,
        })];

        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    target: &wgpu::ColorTargetState,
                    depth_state: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: tri,
                depth_stencil: depth_state,
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(target.clone())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let solid_bias = wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 1.5,
            clamp: 0.0,
        };
        let pipelines = Pipelines {
            bg: make(
                "wb bg",
                "vs_bg",
                "fs_bg",
                &[],
                &opaque,
                depth(false, wgpu::CompareFunction::Always, no_bias),
            ),
            solid: make(
                "wb solid",
                "vs_solid",
                "fs_solid",
                &solid_buffers,
                &opaque,
                depth(true, wgpu::CompareFunction::Less, solid_bias),
            ),
            solid_depth: make(
                "wb solid depth",
                "vs_solid",
                "fs_solid",
                &solid_buffers,
                &depth_only,
                depth(true, wgpu::CompareFunction::Less, solid_bias),
            ),
            line: make(
                "wb line",
                "vs_line",
                "fs_line",
                &line_buffers,
                &alpha,
                depth(false, wgpu::CompareFunction::LessEqual, no_bias),
            ),
            point: make(
                "wb point",
                "vs_point",
                "fs_point",
                &point_buffers,
                &alpha,
                depth(false, wgpu::CompareFunction::LessEqual, no_bias),
            ),
            grid: make(
                "wb grid",
                "vs_grid",
                "fs_grid",
                &[],
                &alpha,
                depth(false, wgpu::CompareFunction::LessEqual, no_bias),
            ),
        };

        match pollster_block(scope.pop()) {
            Some(err) => Err(err.to_string()),
            None => Ok(pipelines),
        }
    }

    /// Cria/atualiza os buffers de um objeto.
    pub fn upload(&self, data: &MeshDrawData, model: Mat4, tint: [f32; 4]) -> GpuMesh {
        let buf = |label: &str, bytes: &[u8]| {
            (!bytes.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    // COPY_DST: o sólido pode ser atualizado em lugar (`update_solid`) durante a escultura
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                })
            })
        };
        let object_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("workbench object"),
            contents: bytemuck::bytes_of(&ObjectUniform::new(model, tint)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let object_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("workbench object"),
            layout: &self.object_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: object_buffer.as_entire_binding(),
            }],
        });
        GpuMesh {
            solid: buf("wb solid", bytemuck::cast_slice(&data.solid)),
            solid_count: data.solid.len() as u32,
            lines: buf("wb lines", bytemuck::cast_slice(&data.lines)),
            line_count: data.lines.len() as u32,
            points: buf("wb points", bytemuck::cast_slice(&data.points)),
            point_count: data.points.len() as u32,
            object_buffer,
            object_bind,
        }
    }

    /// Nova instância (matriz e tinta próprias) que COMPARTILHA os buffers de vértices de `src`:
    /// 300 cópias da mesma cadeira custam 300 uniforms pequenos, não 300 malhas na GPU.
    pub fn instance(&self, src: &GpuMesh, model: Mat4, tint: [f32; 4]) -> GpuMesh {
        let object_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("workbench instance"),
            contents: bytemuck::bytes_of(&ObjectUniform::new(model, tint)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let object_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("workbench instance"),
            layout: &self.object_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: object_buffer.as_entire_binding() }],
        });
        GpuMesh {
            solid: src.solid.clone(),
            solid_count: src.solid_count,
            lines: src.lines.clone(),
            line_count: src.line_count,
            points: src.points.clone(),
            point_count: src.point_count,
            object_buffer,
            object_bind,
        }
    }

    /// Sobrescreve vértices do sólido a partir de `first` (posições/normais/cores novas, mesma contagem).
    /// Usado pela escultura para não recriar o buffer a cada quadro do traço. `false` se não couber.
    pub fn update_solid(&self, mesh: &GpuMesh, first: usize, data: &[SolidVertex]) -> bool {
        let Some(buf) = mesh.solid.as_ref() else {
            return false;
        };
        if data.is_empty() {
            return true;
        }
        if first + data.len() > mesh.solid_count as usize {
            return false;
        }
        let stride = std::mem::size_of::<SolidVertex>() as u64;
        self.queue
            .write_buffer(buf, first as u64 * stride, bytemuck::cast_slice(data));
        true
    }

    /// Número de vértices do sólido na GPU.
    pub fn solid_len(mesh: &GpuMesh) -> usize {
        mesh.solid_count as usize
    }

    /// Atualiza só a matriz do objeto (mover/rotacionar sem reenviar a malha).
    pub fn set_object_transform(&self, mesh: &GpuMesh, model: Mat4, tint: [f32; 4]) {
        self.queue
            .write_buffer(&mesh.object_buffer, 0, bytemuck::bytes_of(&ObjectUniform::new(model, tint)));
    }

    /// Escolhe a vista (alvo) em que `render`, `output_view`, `output_size` e `read_rgba` passam a operar.
    pub fn set_slot(&mut self, slot: usize) {
        self.slot = slot.min(MAX_VIEW_SLOTS - 1);
    }

    fn ensure_targets(&mut self, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.targets[self.slot].as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        };
        let make = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = make("wb msaa", COLOR_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = make("wb depth", DEPTH_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolve = make(
            "wb resolve",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        self.targets[self.slot] = Some(Targets {
            size,
            msaa_color: msaa.create_view(&Default::default()),
            depth: depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            resolve,
        });
    }

    /// Alvo resolvido (1×) — registrar no egui como textura nativa.
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets[self.slot].as_ref().map(|t| &t.resolve_view)
    }

    pub fn output_size(&self) -> Option<(u32, u32)> {
        self.targets[self.slot].as_ref().map(|t| t.size)
    }

    /// `true` se o alvo foi recriado nesta chamada de `render` (a textura do egui precisa ser re-registrada).
    pub fn render(
        &mut self,
        size_px: (u32, u32),
        camera: &OrbitCamera,
        items: &[DrawItem],
        settings: &ViewSettings,
    ) -> bool {
        let recreated = self.targets[self.slot].as_ref().is_none_or(|t| t.size != (size_px.0.max(1), size_px.1.max(1)));
        self.ensure_targets(size_px);
        let Some(targets) = self.targets[self.slot].as_ref() else {
            return false;
        };
        let (w, h) = targets.size;
        let aspect = w as f32 / h as f32;
        let view = camera.view();
        let proj = camera.projection(aspect);
        let vp = proj * view;
        let eye = camera.eye();
        let globals = GlobalsUniform {
            view: view.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            viewport: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            params: [
                settings.xray as u32 as f32,
                // desloca overlays levemente para a câmera (evita z-fighting com o sólido)
                if camera.orthographic { 0.0004 } else { 0.00008 },
                settings.grid_scale.max(1e-4),
                camera.orthographic as u32 as f32,
            ],
            bg_top: self.background_top,
            bg_bottom: self.background_bottom,
        };
        self.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("workbench") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("workbench pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);

            // fundo/grid — usam o bind group do primeiro objeto só para satisfazer o layout
            // (com a cena vazia, o objeto vazio do próprio Workbench)
            let layout_bind = items
                .first()
                .map(|i| &i.mesh.object_bind)
                .or(self.empty_object.as_ref().map(|m| &m.object_bind));
            if settings.shading != ShadingMode::OverlayOnly {
                if let Some(bind) = layout_bind {
                    pass.set_bind_group(1, bind, &[]);
                    pass.set_pipeline(&self.pipelines.bg);
                    pass.draw(0..3, 0..1);
                }
            }

            if matches!(settings.shading, ShadingMode::Solid | ShadingMode::OverlayOnly) {
                pass.set_pipeline(if settings.shading == ShadingMode::OverlayOnly {
                    &self.pipelines.solid_depth
                } else {
                    &self.pipelines.solid
                });
                for item in items {
                    if let Some(buf) = &item.mesh.solid {
                        pass.set_bind_group(1, &item.mesh.object_bind, &[]);
                        pass.set_vertex_buffer(0, buf.slice(..));
                        pass.draw(0..item.mesh.solid_count, 0..1);
                    }
                }
            }

            if settings.show_grid {
                if let Some(bind) = layout_bind {
                    pass.set_bind_group(1, bind, &[]);
                    pass.set_pipeline(&self.pipelines.grid);
                    pass.draw(0..3, 0..1);
                }
            }

            pass.set_pipeline(&self.pipelines.line);
            for item in items {
                if let Some(buf) = &item.mesh.lines {
                    pass.set_bind_group(1, &item.mesh.object_bind, &[]);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..item.mesh.line_count);
                }
            }
            pass.set_pipeline(&self.pipelines.point);
            for item in items {
                if let Some(buf) = &item.mesh.points {
                    pass.set_bind_group(1, &item.mesh.object_bind, &[]);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..item.mesh.point_count);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        recreated
    }

    /// Lê o alvo resolvido em RGBA8 (screenshots e testes).
    pub fn read_rgba(&self) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets[self.slot].as_ref()?;
        let (w, h) = t.size;
        let bytes_per_row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wb readback"),
            size: (bytes_per_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("wb readback") });
        encoder.copy_texture_to_buffer(
            t.resolve.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * bytes_per_row) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        Some((w, h, out))
    }
}

/// Resolve um future simples (error scope) sem runtime assíncrono.
fn pollster_block<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop_raw() -> RawWaker {
        fn clone(_: *const ()) -> RawWaker {
            noop_raw()
        }
        fn noop(_: *const ()) {}
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    // SAFETY: o waker é um no-op sem estado; nenhum ponteiro é desreferenciado.
    let waker = unsafe { Waker::from_raw(noop_raw()) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// Helper de teste/CLI: tamanho em pixels a partir de pontos lógicos.
pub fn px_size(logical: Vec2, pixels_per_point: f32) -> (u32, u32) {
    (
        (logical.x * pixels_per_point).round().max(1.0) as u32,
        (logical.y * pixels_per_point).round().max(1.0) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::{primitives, SelectMode, Selection};
    use glam::Vec3;

    /// Cria device via lavapipe/qualquer adaptador; `None` pula o teste (sem GPU).
    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).ok()?;
        pollster_block(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn save_png(path: &str, w: u32, h: u32, rgba: &[u8]) {
        if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = image::save_buffer(format!("{dir}/{path}"), rgba, w, h, image::ColorType::Rgba8);
        }
    }

    #[test]
    fn renders_cube_with_overlays_and_pixels_are_not_blank() {
        let Some((device, queue)) = device() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let mut wb = Workbench::new(device, queue).expect("pipelines do workbench devem compilar");
        let mesh = primitives::cube(2.0);
        let sel = Selection::from_faces(&mesh, [0u32, 2]);
        let data = build_draw_data(
            &mesh,
            &BuildOptions {
                edit_mode: true,
                select_mode: SelectMode::Face,
                selection: Some(&sel),
                ..Default::default()
            },
        );
        let gpu = wb.upload(&data, Mat4::IDENTITY, [1.0; 4]);
        let cam = OrbitCamera {
            target: Vec3::ZERO,
            distance: 7.0,
            ..Default::default()
        };
        wb.render((640, 480), &cam, &[DrawItem { mesh: &gpu }], &ViewSettings::default());
        let (w, h, px) = wb.read_rgba().expect("readback");
        save_png("workbench_cube.png", w, h, &px);
        assert_eq!((w, h), (640, 480));
        // centro: cubo sólido (cinza claro), canto: gradiente do fundo (escuro)
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2]]
        };
        let center = at(w / 2, h / 2);
        let corner = at(4, 4);
        assert!(center.iter().map(|c| *c as u32).sum::<u32>() > 200, "centro {center:?}");
        assert!(corner.iter().map(|c| *c as u32).sum::<u32>() < 200, "fundo {corner:?}");
        // há pixels laranja (seleção) e pixels de aresta escura
        let orange = px.chunks(4).filter(|p| p[0] > 200 && p[1] > 90 && p[1] < 190 && p[2] < 120).count();
        assert!(orange > 200, "pixels de seleção: {orange}");
    }

    #[test]
    fn wireframe_xray_and_hot_reload_with_invalid_shader_keeps_old_pipelines() {
        let Some((device, queue)) = device() else {
            return;
        };
        let mut wb = Workbench::new(device, queue).unwrap();
        let mesh = primitives::uv_sphere(1.0, 12, 16);
        let data = build_draw_data(&mesh, &BuildOptions { edit_mode: true, ..Default::default() });
        let gpu = wb.upload(&data, Mat4::IDENTITY, [1.0; 4]);
        let cam = OrbitCamera::default();
        let settings = ViewSettings {
            shading: ShadingMode::Wireframe,
            xray: true,
            ..Default::default()
        };
        wb.render((320, 240), &cam, &[DrawItem { mesh: &gpu }], &settings);
        let (w, h, px) = wb.read_rgba().unwrap();
        save_png("workbench_wire_xray.png", w, h, &px);

        // shader inválido: erro explícito e frame continua renderizando
        let err = wb.reload_shaders("this is not wgsl").unwrap_err();
        assert!(!err.is_empty());
        wb.render((320, 240), &cam, &[DrawItem { mesh: &gpu }], &settings);
        assert!(wb.read_rgba().is_some());
        // shader embutido recarrega sem erro
        wb.reload_shaders(EMBEDDED_SHADER).unwrap();
    }

    #[test]
    fn resize_recreates_targets() {
        let Some((device, queue)) = device() else {
            return;
        };
        let mut wb = Workbench::new(device, queue).unwrap();
        let gpu = wb.upload(&MeshDrawData::default(), Mat4::IDENTITY, [1.0; 4]);
        let cam = OrbitCamera::default();
        assert!(wb.render((64, 64), &cam, &[DrawItem { mesh: &gpu }], &ViewSettings::default()));
        assert!(!wb.render((64, 64), &cam, &[DrawItem { mesh: &gpu }], &ViewSettings::default()));
        assert!(wb.render((96, 64), &cam, &[DrawItem { mesh: &gpu }], &ViewSettings::default()));
        assert_eq!(wb.output_size(), Some((96, 64)));
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use anigo_modeling::primitives;
    use glam::Vec3;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).ok()?;
        pollster_block(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    /// No modo OverlayOnly o interior de um sólido destacado deve ficar TRANSPARENTE (alpha 0):
    /// só linhas/pontos/grid aparecem; o sólido existe só como profundidade.
    #[test]
    fn overlay_only_keeps_solid_interior_transparent() {
        let Some((device, queue)) = device() else {
            return;
        };
        let mut wb = Workbench::new(device, queue).unwrap();
        let mesh = primitives::cone(1.0, 0.0, 2.0, 24, true);
        let data = build_draw_data(&mesh, &BuildOptions { highlight_object: true, ..Default::default() });
        let gpu = wb.upload(&data, Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)), [1.0; 4]);
        let cam = OrbitCamera { distance: 6.0, ..Default::default() };
        let settings = ViewSettings {
            shading: ShadingMode::OverlayOnly,
            show_grid: false,
            ..Default::default()
        };
        wb.render((200, 200), &cam, &[DrawItem { mesh: &gpu }], &settings);
        let (w, h, px) = wb.read_rgba().unwrap();
        if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
            let _ = image::save_buffer(format!("{dir}/overlay_cone.png"), &px, w, h, image::ColorType::Rgba8);
        }
        let (center, _) = cam.project(Vec3::new(0.0, 0.8, 0.3), glam::Vec2::new(200.0, 200.0)).unwrap();
        let i = ((center.y as u32 * w + center.x as u32) * 4) as usize;
        eprintln!("pixel central do cone: {:?}", &px[i..i + 4]);
        let opaque = px.chunks(4).filter(|p| p[3] > 200).count();
        let any = px.chunks(4).filter(|p| p[3] > 0).count();
        eprintln!("alpha>200: {opaque}, alpha>0: {any}");
        assert!(opaque < 2500, "o sólido não pode virar opaco no modo overlay ({opaque} px opacos)");
    }
}
