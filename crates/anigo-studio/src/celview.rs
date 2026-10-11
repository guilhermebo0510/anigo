//! Preview cel-shaded: usa o MESMO renderer canônico do render final/export (`anigo-renderer`),
//! compartilhando o `wgpu::Device` do egui. Regra da arquitetura: o que se vê no viewport é
//! o que o render final produz.

use crate::modeling::Document;
use crate::viewport::OrbitCamera;
use anigo_core::{scene::SceneNode, Camera, Scene, StylizedLight};
use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
use anigo_renderer::{HeadlessRenderer, RenderMetrics};
use eframe::wgpu;
use std::sync::Arc;

pub struct CelPreview {
    renderer: HeadlessRenderer,
    device: Arc<wgpu::Device>,
    texture: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    scene: Scene,
    /// (id do objeto, hash do conteúdo) por nó — só reconverte a malha quando muda.
    node_keys: Vec<(u32, u64)>,
    pub metrics: Option<RenderMetrics>,
    pub last_error: Option<String>,
    pub light: StylizedLight,
    /// A luz acompanha a câmera (padrão do editor); desligue para fixar a direção em `light`.
    pub light_follows_camera: bool,
}

impl CelPreview {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, adapter: &wgpu::AdapterInfo) -> Result<Self, String> {
        let device = Arc::new(device);
        let renderer = HeadlessRenderer::from_device(device.clone(), Arc::new(queue), adapter.clone())
            .map_err(|e| format!("{e:#}"))?;
        let mut scene = Scene::new_empty();
        scene.background_color = [0.0, 0.0, 0.0, 0.0];
        Ok(Self {
            renderer,
            device,
            texture: None,
            scene,
            node_keys: Vec::new(),
            metrics: None,
            last_error: None,
            light: StylizedLight::default(),
            light_follows_camera: true,
        })
    }

    /// Sincroniza a cena do renderer com o documento de modelagem.
    pub fn sync_scene(&mut self, doc: &Document, camera: &OrbitCamera, aspect: f32) {
        let wanted: Vec<(u32, u64)> = doc
            .objects
            .iter()
            .filter(|o| o.visible)
            .map(|o| (o.id, o.mesh.content_hash() ^ o.modifiers_hash().rotate_left(17)))
            .collect();
        if wanted != self.node_keys {
            self.scene.nodes.clear();
            for o in doc.objects.iter().filter(|o| o.visible) {
                let mut node = SceneNode::new(format!("obj{}", o.id), o.name.clone());
                let eval = o.evaluated(crate::modeling::EvalMode::Viewport);
                let source = match &eval {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        self.last_error = Some(format!("{}: {e}", o.name));
                        &o.mesh
                    }
                    None => &o.mesh,
                };
                match to_core_mesh(source, &ToCoreOptions::default()) {
                    Ok(m) => node.mesh = Some(m),
                    Err(e) => self.last_error = Some(format!("{}: {e}", o.name)),
                }
                self.scene.nodes.push(node);
            }
            self.node_keys = wanted;
        }
        for (node, obj) in self.scene.nodes.iter_mut().zip(doc.objects.iter().filter(|o| o.visible)) {
            node.transform = obj.transform;
        }
        self.scene.camera = Camera {
            eye: camera.eye(),
            target: camera.target,
            up: camera.up(),
            fov_y: camera.fov_y,
            aspect,
            z_near: camera.near,
            z_far: camera.far,
            orthographic: None,
        };
        let mut light = self.light.clone();
        if self.light_follows_camera {
            // luz de estúdio presa à câmera: esquerda/acima/frente (mesma ideia do Workbench)
            let back = -camera.forward();
            let dir = (-camera.right() * 0.45 + camera.up() * 0.6 + back * 0.65).normalize_or_zero();
            light.direction = dir.to_array();
        }
        self.scene.light = light;
    }

    /// Renderiza na textura interna (recriada se o tamanho mudar). `true` se foi recriada.
    pub fn render(&mut self, size: (u32, u32)) -> bool {
        let size = (size.0.max(1), size.1.max(1));
        let recreated = self.texture.as_ref().is_none_or(|t| t.2 != size);
        if recreated {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cel preview"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: anigo_renderer::offscreen_color_format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.texture = Some((tex, view, size));
        }
        if let Some((tex, _, _)) = &self.texture {
            match self.renderer.render_to_texture(&self.scene, tex) {
                Ok(m) => {
                    self.metrics = Some(m);
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(format!("{e:#}")),
            }
        }
        recreated
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.texture.as_ref().map(|t| &t.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{ModelingSession, PrimitiveKind};
    use anigo_core::Scene;

    /// Todo primitiva do editor precisa produzir pixels no renderer canônico (nenhuma some por
    /// enrolamento/normais inválidos).
    #[test]
    fn every_primitive_renders_visible_pixels_in_the_canonical_renderer() {
        let Ok(renderer) = pollster::block_on(HeadlessRenderer::new()) else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        for kind in PrimitiveKind::ALL {
            let mut s = ModelingSession::new();
            s.add_primitive(kind, glam::Vec3::new(0.0, 1.0, 0.0));
            let obj = &s.doc.objects[0];
            let core = to_core_mesh(&obj.mesh, &ToCoreOptions::default()).unwrap();
            let mut scene = Scene::new_empty();
            scene.background_color = [0.0, 0.0, 0.0, 1.0];
            let mut node = SceneNode::new("n", "n");
            node.mesh = Some(core);
            node.transform = obj.transform;
            scene.add_node(node);
            scene.camera.eye = glam::Vec3::new(2.5, 3.0, 5.0);
            scene.camera.target = glam::Vec3::new(0.0, 1.0, 0.0);
            let (img, _) = pollster::block_on(renderer.render_scene(&scene, 128, 128)).unwrap();
            let lit = img.pixels().filter(|p| p.0[0] as u32 + p.0[1] as u32 + p.0[2] as u32 > 30).count();
            if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
                let _ = std::fs::create_dir_all(&dir);
                let _ = img.save(format!("{dir}/prim_{:?}.png", kind));
            }
            assert!(lit > 200, "{kind:?} renderizou só {lit} pixels visíveis");
        }
    }
}

#[cfg(test)]
mod multi_tests {
    use super::*;
    use crate::modeling::{ModelingSession, PrimitiveKind};

    #[test]
    fn three_objects_are_all_drawn_not_culled() {
        let Ok(renderer) = pollster::block_on(HeadlessRenderer::new()) else {
            return;
        };
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::QuadSphere, glam::Vec3::new(0.0, 1.0, 0.0));
        s.add_primitive(PrimitiveKind::Torus, glam::Vec3::new(2.4, 1.0, 0.0));
        s.add_primitive(PrimitiveKind::Cone, glam::Vec3::new(-2.4, 1.0, 0.0));
        let mut scene = Scene::new_empty();
        for o in &s.doc.objects {
            let mut n = SceneNode::new(format!("o{}", o.id), o.name.clone());
            n.mesh = Some(to_core_mesh(&o.mesh, &ToCoreOptions::default()).unwrap());
            n.transform = o.transform;
            scene.add_node(n);
        }
        scene.camera.eye = glam::Vec3::new(5.0, 5.0, 8.0);
        scene.camera.target = glam::Vec3::new(0.0, 1.0, 0.0);
        let (img, m) = pollster::block_on(renderer.render_scene(&scene, 400, 300)).unwrap();
        if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
            let _ = img.save(format!("{dir}/three.png"));
        }
        eprintln!("draw_calls={} culled={}", m.draw_calls, m.culled_draw_calls);
        assert_eq!(m.culled_draw_calls, 0, "nenhum objeto visível pode ser descartado");
    }
}
