//! Preview cel-shaded: usa o MESMO renderer canônico do render final/export (`anigo-renderer`),
//! compartilhando o `wgpu::Device` do egui. Regra da arquitetura: o que se vê no viewport é
//! o que o render final produz.

use crate::modeling::Document;
use crate::viewport::OrbitCamera;
use anigo_core::{scene::SceneNode, Camera, Scene, StylizedLight};
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
    Level,
}

/// Nó do cenário montado pelo workspace Cenário (um por `NodeSlot` do projeto).
#[derive(Clone)]
pub struct LevelNodeIn {
    pub id: String,
    pub parent: Option<String>,
    pub transform: anigo_core::Transform,
    pub visible: bool,
    /// Chave da geometria + a malha (só é clonada para o renderer quando a chave muda).
    pub geom: Option<(String, Arc<anigo_core::Mesh>)>,
    pub material: anigo_core::StylizedMaterial,
}

/// Câmera do renderer canônico a partir da câmera orbital do editor — INCLUINDO a projeção ortogonal (com a mesma
/// altura de volume que o Workbench usa: `2·distância·tan(fov/2)`), senão o preview Cel ficaria em perspectiva.
fn core_camera(camera: &OrbitCamera, aspect: f32) -> Camera {
    Camera {
        eye: camera.eye(),
        target: camera.target,
        up: camera.up(),
        fov_y: camera.fov_y,
        aspect,
        z_near: camera.near,
        z_far: camera.far,
        orthographic: camera.orthographic.then(|| anigo_core::OrthographicBounds::from_height(2.0 * camera.distance * (camera.fov_y * 0.5).tan(), aspect)),
    }
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
    /// Cenário: geometria atual de cada nó (id → chave) e assinatura do último quadro desenhado.
    level_geom: std::collections::HashMap<String, String>,
    level_sig: u64,
    /// Luzes de cena para o PRÓXIMO `sync_level` (consumidas por ele): quem não as pede recebe a cena sem luzes extras.
    pending_lights: Vec<anigo_core::scene::RenderLight>,
    /// Workspace Render: assinatura da última câmera/DoF/passes aplicados por `apply_render_view`.
    render_view_sig: u64,
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
            level_geom: std::collections::HashMap::new(),
            level_sig: 0,
            pending_lights: Vec::new(),
            render_view_sig: 0,
        })
    }

    /// Workspace Render: usa a câmera EXATA do projeto (perspectiva/ortográfica, near/far), o DoF e os passes
    /// ligados/desligados do grafo — o que o render final vai usar. Chame DEPOIS de `sync_level` e ANTES de
    /// `render_if`; devolve `true` se algo disso mudou (sujo). Ao SAIR do workspace Render chame
    /// [`Self::clear_render_view`]: os outros workspaces não herdam DoF nem overrides.
    pub fn apply_render_view(&mut self, camera: Camera, overrides: anigo_renderer::GraphOverrides, dof: anigo_core::scene::DofSettings) -> bool {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |bits: u32| {
            h ^= bits as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        };
        for b in camera.eye.to_array().into_iter().chain(camera.target.to_array()).chain(camera.up.to_array()).chain([camera.fov_y, camera.aspect, camera.z_near, camera.z_far]) {
            mix(b.to_bits());
        }
        if let Some(o) = camera.orthographic {
            for b in [o.left, o.right, o.bottom, o.top] {
                mix(b.to_bits());
            }
        }
        mix(dof.enabled as u32);
        for b in [dof.focus_distance, dof.f_number, dof.focal_mm, dof.max_radius_px] {
            mix(b.to_bits());
        }
        mix(dof.bokeh_shape);
        for name in overrides.disabled.iter().chain(&overrides.order) {
            for byte in name.bytes() {
                mix(byte as u32);
            }
            mix(0xff);
        }
        mix(overrides.depth_prepass as u32);
        self.scene.camera = camera;
        self.scene.dof = dof;
        self.renderer.set_graph_overrides(overrides);
        let changed = h != self.render_view_sig;
        self.render_view_sig = h;
        changed
    }

    /// Desfaz [`Self::apply_render_view`] (DoF desligado, passes do contrato) e força o próximo `sync_level` a redesenhar.
    pub fn clear_render_view(&mut self) {
        self.scene.dof = anigo_core::scene::DofSettings::default();
        self.renderer.set_graph_overrides(anigo_renderer::GraphOverrides::default());
        self.render_view_sig = 0;
        self.level_sig = 0;
    }

    /// Sincroniza a cena do renderer com o workspace Cenário: um nó por `NodeSlot` (hierarquia
    /// respeitada pelo `parent_id`), malhas compartilhadas por chave, luz e câmera do projeto.
    /// Devolve `true` quando algo que muda a imagem mudou desde o último quadro desenhado.
    /// Luzes de cena (ponto, holofote, painel, direcional extra) para o próximo [`Self::sync_level`].
    pub fn set_extra_lights(&mut self, lights: Vec<anigo_core::scene::RenderLight>) {
        self.pending_lights = lights;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync_level(
        &mut self,
        nodes: &[LevelNodeIn],
        light: StylizedLight,
        camera: &OrbitCamera,
        aspect: f32,
        background: [f32; 4],
        extra_sig: u64,
    ) -> bool {
        if self.owner != SceneOwner::Level {
            self.scene.nodes.clear();
            self.level_geom.clear();
            self.owner = SceneOwner::Level;
            self.char_key = None;
            self.level_sig = 0;
        }
        let mut old: std::collections::HashMap<String, SceneNode> =
            std::mem::take(&mut self.scene.nodes).into_iter().map(|n| (n.id.clone(), n)).collect();
        let mut h = extra_sig ^ 0xcbf2_9ce4_8422_2325;
        let mut mix = |bits: u32| {
            h ^= bits as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        };
        let mut next_keys = std::collections::HashMap::with_capacity(nodes.len());
        for n in nodes {
            let wanted_key = n.geom.as_ref().map(|g| g.0.clone());
            let mut node = match old.remove(&n.id) {
                Some(existing) if self.level_geom.get(&n.id).cloned() == wanted_key => existing,
                _ => {
                    let mut fresh = SceneNode::new(n.id.clone(), n.id.clone());
                    fresh.mesh = n.geom.as_ref().map(|g| (*g.1).clone());
                    fresh
                }
            };
            node.parent_id = n.parent.clone();
            node.transform = n.transform;
            node.visible = n.visible;
            node.material = Some(n.material.clone());
            if let Some(k) = wanted_key {
                next_keys.insert(n.id.clone(), k);
            }
            for b in n.transform.translation.to_array().into_iter().chain(n.transform.scale.to_array()).chain(n.transform.rotation.to_array()) {
                mix(b.to_bits());
            }
            mix(n.visible as u32);
            for b in n.material.base_color.into_iter().chain(n.material.shade_color).chain(n.material.rim_color) {
                mix(b.to_bits());
            }
            // parâmetros de iluminação do material (terminador e rim): mexer neles tem de redesenhar
            for v in [
                n.material.outline_width,
                n.material.rim_intensity,
                n.material.rim_spread,
                n.material.shadow_threshold,
                n.material.shadow_smoothness,
                n.material.toon_steps,
                n.material.hue_shift,
                n.material.spec_intensity,
            ] {
                mix(v.to_bits());
            }
            mix(n.geom.as_ref().map_or(0, |g| g.0.len() as u32));
            self.scene.nodes.push(node);
        }
        self.level_geom = next_keys;
        self.scene.rebuild_children();
        self.scene.background_color = background;
        self.scene.light = light;
        self.scene.camera = core_camera(camera, aspect);
        for b in camera.eye().to_array().into_iter().chain(camera.target.to_array()).chain([camera.fov_y, camera.distance, aspect]) {
            mix(b.to_bits());
        }
        mix(camera.orthographic as u32);
        // TODOS os campos da luz entram na assinatura (cor, ambiente, tinta de sombra, sombra projetada):
        // antes só direção e intensidade, e editar a cor/ambiente não redesenhava o quadro parado.
        {
            let l = &self.scene.light;
            for v in l
                .direction
                .iter()
                .chain(&l.color)
                .chain(&l.shadow_color)
                .chain(&l.ambient_sky)
                .chain(&l.ambient_ground)
                .chain([&l.intensity, &l.ambient_intensity, &l.shadow_saturation, &l.shadow.strength, &l.shadow.hardness, &l.shadow.bias, &l.shadow.normal_bias, &l.shadow.distance])
            {
                mix(v.to_bits());
            }
            mix(l.shadow.enabled as u32);
            mix(l.shadow.resolution);
        }
        for b in self.scene.background_color {
            mix(b.to_bits());
        }
        // luzes de cena: qualquer mudança (posição, cor, cone...) redesenha o quadro parado
        self.scene.extra_lights = std::mem::take(&mut self.pending_lights);
        mix(self.scene.extra_lights.len() as u32);
        for l in &self.scene.extra_lights {
            mix(l.kind as u32);
            for v in l.position.iter().chain(&l.direction).chain(&l.color).chain(&l.size).chain([&l.intensity, &l.range, &l.spot_angle_deg, &l.spot_softness]) {
                mix(v.to_bits());
            }
        }
        let changed = h != self.level_sig;
        self.level_sig = h;
        changed
    }

    /// Como [`CelPreview::render`], mas só redesenha se `dirty` ou se a textura foi recriada
    /// (o renderer sobe os buffers de cada nó a cada quadro: cenas grandes não devem pagar isso parados).
    pub fn render_if(&mut self, size: (u32, u32), dirty: bool) -> bool {
        let size = (size.0.max(1), size.1.max(1));
        if !dirty && self.texture.as_ref().is_some_and(|t| t.2 == size) {
            return false;
        }
        self.render(size)
    }

    /// Sincroniza a cena com o PERSONAGEM da sessão do núcleo: a malha já deformada
    /// (`CoreSession::sync_scene` = morphs + proporções + material + luz do projeto) é
    /// injetada no mesmo renderer canônico. A malha só é reconstruída quando as revisões
    /// da sessão mudam; câmera e luz são atualizadas a cada quadro.
    pub fn sync_character(&mut self, session: &mut CoreSession, camera: &OrbitCamera, aspect: f32, light_follows_camera: bool) {
        let key = (session.dynamic_revision(), session.static_revision(), session.history().revision());
        self.scene.extra_lights.clear();
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
        self.scene.camera = core_camera(camera, aspect);
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
        self.scene.extra_lights.clear();
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
            .map(|o| (o.id, o.render_hash() ^ doc.stylized.get(&o.id).map_or(0, |m| anigo_core::fnv1a64(&format!("{m:?}")).rotate_left(7))))
            .collect();
        if wanted != self.node_keys {
            self.scene.nodes.clear();
            for o in doc.objects.iter().filter(|o| o.visible) {
                let mut node = SceneNode::new(format!("obj{}", o.id), o.name.clone());
                if let Some(m) = &o.material {
                    // material por objeto (grupos de cabelo): cor base, sombra e brilho próprios
                    let mut sm = anigo_core::scene::StylizedMaterial::default();
                    sm.name = m.name.clone();
                    sm.base_color = [m.base_color[0], m.base_color[1], m.base_color[2], 1.0];
                    sm.shade_color = [m.shade_color[0], m.shade_color[1], m.shade_color[2], 1.0];
                    sm.specular_color = [m.highlight_color[0], m.highlight_color[1], m.highlight_color[2], 1.0];
                    sm.spec_intensity = 0.9;
                    sm.rim_intensity = 0.45;
                    node.material = Some(sm);
                }
                if let Some(full) = doc.stylized.get(&o.id) {
                    // material completo editado no workspace Shading
                    node.material = Some(full.clone());
                }
                let eval = o.evaluated(crate::modeling::EvalMode::Viewport);
                let source = match &eval {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        self.last_error = Some(format!("{}: {e}", o.name));
                        &o.mesh
                    }
                    None => &o.mesh,
                };
                match anigo_modeling::convert::to_core_mesh(source, &anigo_modeling::convert::ToCoreOptions { normals: o.normal_params() }) {
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
        self.scene.camera = core_camera(camera, aspect);
        let mut light = self.light.clone();
        if self.light_follows_camera {
            // luz de estúdio presa à câmera: esquerda/acima/frente (mesma ideia do Workbench)
            let back = -camera.forward();
            let dir = (-camera.right() * 0.45 + camera.up() * 0.6 + back * 0.65).normalize_or_zero();
            light.direction = dir.to_array();
        }
        self.scene.light = light;
    }

    /// Mapa SDF da sombra facial (R16Float, 0..1) usado pelos materiais com `face_sdf_enabled`
    /// (renderer-global: vale para toda a cena até ser trocado/limpo).
    pub fn set_face_sdf_f32(&self, width: u32, height: u32, values: &[f32]) -> Result<(), String> {
        self.renderer.set_face_sdf_map_f32(width, height, values).map_err(|e| format!("{e:#}"))
    }

    /// Mapa SDF 8 bits (canal R de uma imagem do artista).
    pub fn set_face_sdf_r8(&self, width: u32, height: u32, r8: &[u8]) -> Result<(), String> {
        self.renderer.set_face_sdf_map(width, height, r8).map_err(|e| format!("{e:#}"))
    }

    /// Volta ao mapa neutro (a face SDF não altera a imagem).
    pub fn clear_face_sdf(&self) {
        self.renderer.clear_face_sdf_map();
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

    /// `false` no backend GL: o renderer não cria o passe de sombra projetada.
    pub fn shadow_supported(&self) -> bool {
        self.renderer.shadow_supported()
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
    use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
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
    use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};

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
