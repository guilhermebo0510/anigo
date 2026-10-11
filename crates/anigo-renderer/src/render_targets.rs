//! Alvos de render: o destino de [`crate::HeadlessRenderer::encode_scene`] e o
//! cache dos recursos intermediários (MSAA, profundidade, DoF) reutilizado
//! entre quadros.
//!
//! Antes, cada quadro recriava MSAA + profundidade (+ DoF) — custo inútil num
//! viewport interativo. O cache só realoca quando `width`/`height` mudam; a
//! cor final (resolve) pertence ao chamador (`RenderTarget::color_view`).

use crate::render_contract as contract;

/// Destino de um quadro: a cor **final/resolvida** em formato do contrato
/// ([`contract::offscreen_color_format`]), 1 amostra, usage `RENDER_ATTACHMENT`.
#[derive(Clone, Copy)]
pub struct RenderTarget<'a> {
    pub color_view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

/// Handles (clones baratos, contados por referência) dos recursos do cache
/// para um quadro.
pub(crate) struct TargetViews {
    pub msaa_view: Option<wgpu::TextureView>,
    pub depth_view: wgpu::TextureView,
    pub scene_view: Option<wgpu::TextureView>,
    pub dof_depth_view: Option<wgpu::TextureView>,
}

struct Allocated {
    width: u32,
    height: u32,
    msaa_view: Option<wgpu::TextureView>,
    // Os views mantêm as texturas vivas; só eles são guardados.
    depth_view: wgpu::TextureView,
    /// Intermediária 1× do DoF (a cena resolve aqui e o pós escreve na cor
    /// final). Alocada sob demanda, na primeira cena com DoF.
    scene_view: Option<wgpu::TextureView>,
    dof_depth_view: Option<wgpu::TextureView>,
}

/// Cache dos alvos intermediários, com tamanho único (o do último quadro).
#[derive(Default)]
pub struct RenderTargetCache {
    current: Option<Allocated>,
    allocations: u64,
}

impl RenderTargetCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tamanho atualmente alocado, se houver.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.current.as_ref().map(|a| (a.width, a.height))
    }

    /// Quantas vezes algum recurso foi (re)alocado (telemetria/teste).
    pub fn allocations(&self) -> u64 {
        self.allocations
    }

    /// Descarta tudo (troca de device: os handles antigos não servem mais).
    pub fn clear(&mut self) {
        self.current = None;
    }

    /// Garante os alvos para `width`×`height` e devolve os handles.
    /// Realoca apenas quando o tamanho mudou; o DoF é alocado sob demanda.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        dof_enabled: bool,
    ) -> TargetViews {
        let color_format = contract::offscreen_color_format();
        let depth_format = contract::depth_format();
        let sample_count = contract::msaa_sample_count().max(1);
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let stale = !matches!(&self.current, Some(a) if a.width == width && a.height == height);
        if stale {
            // P0 renderer: mesmo alvo MSAA do viewport, resolvido na cor final.
            let msaa_view = if sample_count > 1 && contract::resolve_to_swapchain() {
                Some(
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Offscreen Color Texture (MSAA)"),
                            size: extent,
                            mip_level_count: 1,
                            sample_count,
                            dimension: wgpu::TextureDimension::D2,
                            format: color_format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        })
                        .create_view(&wgpu::TextureViewDescriptor::default()),
                )
            } else {
                None
            };
            let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen Depth Texture"),
                size: extent,
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.current = Some(Allocated {
                width,
                height,
                msaa_view,
                depth_view,
                scene_view: None,
                dof_depth_view: None,
            });
            self.allocations += 1;
        }

        let allocated = self
            .current
            .as_mut()
            .expect("cache preenchido logo acima");

        // Fase 2 (#53): Anime Bokeh DoF — a cena resolve numa textura 1×
        // amostrável; a profundidade 1× idem (a MSAA depth só existe como
        // attachment). Sob demanda: cenas sem DoF não pagam por elas.
        if dof_enabled {
            if allocated.scene_view.is_none() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Offscreen Color Texture (DoF scene, 1x)"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: color_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                allocated.scene_view =
                    Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
                self.allocations += 1;
            }
            if allocated.dof_depth_view.is_none() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Offscreen Depth Texture (DoF resolve, 1x)"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: depth_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                allocated.dof_depth_view =
                    Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
                self.allocations += 1;
            }
        }

        TargetViews {
            msaa_view: allocated.msaa_view.clone(),
            depth_view: allocated.depth_view.clone(),
            scene_view: if dof_enabled { allocated.scene_view.clone() } else { None },
            dof_depth_view: if dof_enabled { allocated.dof_depth_view.clone() } else { None },
        }
    }
}
