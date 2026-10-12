//! Preview cel-shaded: usa o MESMO renderer canônico do render final/export (`anigo-renderer`),
//! compartilhando o `wgpu::Device` do egui. Regra da arquitetura: o que se vê no viewport é
//! o que o render final produz.

use crate::modeling::Document;
use crate::viewport::OrbitCamera;
use anigo_core::{scene::SceneNode, Camera, Scene, StylizedLight};
use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
use anigo_renderer::{HeadlessRenderer, RenderMetrics};
use anigo_session::CoreSession;
use eframe::wgpu;
use std::sync::Arc;

/// Quem escreveu a cena do renderer por último (os workspaces compartilham o mesmo renderer
/// canônico, mas cada um injeta o seu conteúdo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneOwner {
    Modeling,
    Character,
}

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
    owner: SceneOwner,
    /// (revisão dinâmica, revisão estática, revisão do histórico) da última sincronização do personagem.
    char_key: Option<(u64, u64, u64)>,
    /// Luz do projeto (mundo) — usada quando a luz do personagem não acompanha a câmera.
    char_light: StylizedLight,
    /// Caixa envolvente (min, max) da malha deformada do personagem.
    pub character_bounds: Option<(glam::Vec3, glam::Vec3)>,
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
            owner: SceneOwner::Modeling,
            char_key: None,
            char_light: StylizedLight::default(),
            character_bounds: None,
        })
    }

    /// Sincroniza a cena com o PERSONAGEM da sessão do núcleo: a malha já deformada
    /// (`CoreSession::sync_scene` = morphs + proporções + material + luz do projeto) é
    /// injetada no mesmo renderer canônico. A malha só é reconstruída quando as revisões
    /// da sessão mudam; câmera e luz são atualizadas a cada quadro.
    pub fn sync_character(&mut self, session: &mut CoreSession, camera: &OrbitCamera, aspect: f32, light_follows_camera: bool) {
        let key = (session.dynamic_revision(), session.static_revision(), session.history().revision());
        if self.owner != SceneOwner::Character || self.char_key != Some(key) {
            match session.sync_scene(&mut self.scene) {
                Ok(()) => {
                    self.last_error = None;
                    self.char_key = Some(key);
                    self.char_light = self.scene.light.clone();
                    self.character_bounds = scene_bounds(&self.scene);
                }
                Err(e) => {
                    self.last_error = Some(format!("personagem: {e}"));
                    self.char_key = None;
                }
            }
            // o fundo é do Workbench (gradiente + grid); aqui só o personagem, sobre transparente
            self.scene.background_color = [0.0, 0.0, 0.0, 0.0];
            self.node_keys.clear();
            self.owner = SceneOwner::Character;
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
        let mut light = self.char_light.clone();
        if light_follows_camera {
            let back = -camera.forward();
            let dir = (-camera.right() * 0.45 + camera.up() * 0.6 + back * 0.65).normalize_or_zero();
            light.direction = dir.to_array();
        }
        self.scene.light = light;
    }

    /// Sincroniza a cena do renderer com o documento de modelagem.
    pub fn sync_scene(&mut self, doc: &Document, camera: &OrbitCamera, aspect: f32) {
        if self.owner != SceneOwner::Modeling {
            // a cena veio de outro workspace: recomeça do zero
            self.scene.nodes.clear();
            self.node_keys.clear();
            self.scene.background_color = [0.0, 0.0, 0.0, 0.0];
            self.owner = SceneOwner::Modeling;
            self.char_key = None;
        }
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

/// Caixa envolvente de todos os vértices das malhas da cena (espaço do objeto, sem transformação).
fn scene_bounds(scene: &Scene) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut lo = glam::Vec3::splat(f32::INFINITY);
    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for mesh in scene.nodes.iter().filter_map(|n| n.mesh.as_ref()) {
        for v in &mesh.vertices {
            let p = glam::Vec3::from_array(v.position);
            if p.is_finite() {
                lo = lo.min(p);
                hi = hi.max(p);
                any = true;
            }
        }
    }
    any.then_some((lo, hi))
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

#[cfg(test)]
mod character_tests {
    use super::*;
    use anigo_core::scene::Scene;

    /// Frame canônico (headless) do personagem do núcleo: é a referência de paridade de
    /// silhueta com o viewport do workspace Personagem (que usa a mesma cena + renderer).
    fn canonical_frame(session: &mut CoreSession, w: u32, h: u32) -> Option<image::RgbaImage> {
        let renderer = pollster::block_on(HeadlessRenderer::new()).ok()?;
        let mut scene = Scene::new_empty();
        session.sync_scene(&mut scene).ok()?;
        let (img, _) = pollster::block_on(renderer.render_scene(&scene, w, h)).ok()?;
        Some(img)
    }

    #[test]
    fn canonical_character_frame_is_shaded_and_not_empty() {
        let mut session = CoreSession::new();
        let Some(img) = canonical_frame(&mut session, 400, 600) else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = img.save(format!("{dir}/character_canonical.png"));
        }
        let bg = img.get_pixel(0, 0).0;
        let fg: Vec<_> = img.pixels().filter(|p| p.0 != bg).collect();
        assert!(fg.len() > 3000, "personagem sumiu: {} pixels", fg.len());
    }
}
