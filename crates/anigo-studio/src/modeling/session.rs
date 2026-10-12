use super::history::History;
use super::modal::{ModalContext, ModalTransform, TransformKind};
use anigo_core::math::Transform;
use anigo_modeling::normals::NormalParams;
use anigo_modeling::{primitives, select, EditMesh, SelectMode, Selection, Topology};
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Object,
    Edit,
    Sculpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveKind {
    Plane,
    Grid,
    Cube,
    QuadSphere,
    UvSphere,
    Cylinder,
    Cone,
    Torus,
    Capsule,
    Circle,
}

impl PrimitiveKind {
    pub const ALL: [PrimitiveKind; 10] = [
        PrimitiveKind::Plane,
        PrimitiveKind::Grid,
        PrimitiveKind::Cube,
        PrimitiveKind::QuadSphere,
        PrimitiveKind::UvSphere,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Cone,
        PrimitiveKind::Torus,
        PrimitiveKind::Capsule,
        PrimitiveKind::Circle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrimitiveKind::Plane => "Plano",
            PrimitiveKind::Grid => "Grade",
            PrimitiveKind::Cube => "Cubo",
            PrimitiveKind::QuadSphere => "Esfera Quad",
            PrimitiveKind::UvSphere => "Esfera UV",
            PrimitiveKind::Cylinder => "Cilindro",
            PrimitiveKind::Cone => "Cone",
            PrimitiveKind::Torus => "Toro",
            PrimitiveKind::Capsule => "Cápsula",
            PrimitiveKind::Circle => "Círculo",
        }
    }

    pub fn build(self) -> EditMesh {
        match self {
            PrimitiveKind::Plane => primitives::plane(2.0),
            PrimitiveKind::Grid => primitives::grid(8, 8, 4.0, 4.0),
            PrimitiveKind::Cube => primitives::cube(2.0),
            PrimitiveKind::QuadSphere => primitives::quad_sphere(1.0, 6),
            PrimitiveKind::UvSphere => primitives::uv_sphere(1.0, 12, 24),
            PrimitiveKind::Cylinder => primitives::cylinder(1.0, 2.0, 24, 2, true),
            PrimitiveKind::Cone => primitives::cone(1.0, 0.0, 2.0, 24, true),
            PrimitiveKind::Torus => primitives::torus(1.0, 0.3, 32, 12),
            PrimitiveKind::Capsule => primitives::capsule(0.5, 1.0, 16, 6),
            PrimitiveKind::Circle => primitives::circle(1.0, 24, false),
        }
    }
}

/// Ângulo padrão de auto-smooth (30°), em radianos.
pub const DEFAULT_AUTO_SMOOTH: f32 = std::f32::consts::PI * 30.0 / 180.0;

fn default_auto_smooth() -> f32 {
    DEFAULT_AUTO_SMOOTH
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelObject {
    pub id: u32,
    pub name: String,
    pub mesh: EditMesh,
    pub transform: Transform,
    pub visible: bool,
    /// Pilha de modificadores não destrutivos (Subsurf, Mirror, Solidify…).
    #[serde(default)]
    pub modifiers: anigo_modeling::modifiers::stack::ModifierStack,
    /// Ângulo de auto-smooth da malha (radianos). Arestas com diedro maior viram duras no cálculo
    /// de normais (Workbench, preview Cel e export). `>= π` desliga o corte por ângulo.
    #[serde(default = "default_auto_smooth")]
    pub auto_smooth_angle: f32,
}

impl ModelObject {
    pub fn model_matrix(&self) -> Mat4 {
        self.transform.to_matrix()
    }

    /// Malha após a pilha de modificadores (modo viewport). Sem modificadores devolve `None`
    /// (use `mesh` direto). Em erro devolve a mensagem e o chamador cai na malha base.
    pub fn evaluated(&self, mode: anigo_modeling::modifiers::stack::EvalMode) -> Option<Result<EditMesh, String>> {
        if self.modifiers.modifiers.iter().all(|m| !m.enabled_in(mode)) {
            return None;
        }
        Some(self.modifiers.evaluate(&self.mesh, mode).map_err(|e| e.to_string()))
    }

    /// Hash do estado da pilha (para chaves de cache de GPU).
    pub fn modifiers_hash(&self) -> u64 {
        let json = serde_json::to_string(&self.modifiers).unwrap_or_default();
        anigo_core::ids::fnv1a64(&json)
    }

    /// Parâmetros de normais deste objeto (usados por Workbench, preview Cel e `to_core`).
    pub fn normal_params(&self) -> NormalParams {
        normal_params_for(self.auto_smooth_angle)
    }

    /// Conversão para o formato do renderer respeitando o auto-smooth e as normais customizadas.
    pub fn to_core(&self) -> anigo_modeling::ModelResult<anigo_core::Mesh> {
        anigo_modeling::convert::to_core_mesh(
            &self.mesh,
            &anigo_modeling::convert::ToCoreOptions {
                normals: self.normal_params(),
            },
        )
    }

    /// Hash do que influencia o desenho: geometria + normais customizadas + cores de vértice
    /// (canais NPR) + ângulo de auto-smooth. `EditMesh::content_hash` sozinho não enxerga
    /// normais nem cores, então caches de GPU usam este.
    pub fn render_hash(&self) -> u64 {
        let mut h: u64 = self.mesh.content_hash() ^ self.modifiers_hash().rotate_left(17);
        let mut mix = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        mix(&self.auto_smooth_angle.to_bits().to_le_bytes());
        for f in &self.mesh.faces {
            for c in &f.corners {
                match c.normal {
                    Some(n) => {
                        mix(&[1]);
                        for x in n.to_array() {
                            mix(&x.to_bits().to_le_bytes());
                        }
                    }
                    None => mix(&[0]),
                }
            }
        }
        for v in &self.mesh.verts {
            for x in v.color {
                mix(&x.to_bits().to_le_bytes());
            }
        }
        h
    }
}

/// `NormalParams` para um ângulo de auto-smooth em radianos (`>= π` = sem corte por ângulo).
pub fn normal_params_for(auto_smooth_angle: f32) -> NormalParams {
    NormalParams {
        auto_smooth_angle: (auto_smooth_angle < std::f32::consts::PI - 1e-3).then_some(auto_smooth_angle.max(0.0)),
        ..NormalParams::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Document {
    pub objects: Vec<ModelObject>,
    pub active: Option<usize>,
    next_id: u32,
}

#[derive(Debug, Clone)]
struct Snapshot {
    doc: Document,
    selection: Selection,
}

struct ActiveModal {
    transform: ModalTransform,
    object: usize,
    model: Mat4,
    before: Snapshot,
}

/// Edição proporcional (tecla O no Blender).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proportional {
    pub enabled: bool,
    pub radius: f32,
    pub falloff: anigo_modeling::ops::transform::ProportionalFalloff,
    /// Só vértices conectados (distância geodésica) — não vaza entre ilhas.
    pub connected_only: bool,
}

pub struct ModelingSession {
    pub doc: Document,
    pub mode: Mode,
    pub select_mode: SelectMode,
    pub selection: Selection,
    pub active_vert: Option<u32>,
    pub proportional: Proportional,
    pub xray: bool,
    pub snap_enabled: bool,
    history: History<SnapshotPub>,
    modal: Option<ActiveModal>,
    /// Incrementa a cada mudança que exige reenviar dados à GPU.
    pub revision: u64,
    pub status: String,
}

/// Wrapper público opaco para o histórico (Snapshot é privado).
#[derive(Clone)]
pub struct SnapshotPub(Snapshot);

impl Default for ModelingSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelingSession {
    pub fn new() -> Self {
        Self {
            doc: Document::default(),
            mode: Mode::Object,
            select_mode: SelectMode::Vertex,
            selection: Selection::default(),
            active_vert: None,
            proportional: Proportional {
                enabled: false,
                radius: 1.0,
                falloff: anigo_modeling::ops::transform::ProportionalFalloff::Smooth,
                connected_only: false,
            },
            xray: false,
            snap_enabled: false,
            history: History::new(100),
            modal: None,
            revision: 1,
            status: String::new(),
        }
    }

    /// Sessão com um cubo (estado inicial padrão, como o Blender).
    pub fn with_default_cube() -> Self {
        let mut s = Self::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::new(0.0, 1.0, 0.0));
        s.history.clear();
        s
    }

    fn snapshot(&self) -> SnapshotPub {
        SnapshotPub(Snapshot {
            doc: self.doc.clone(),
            selection: self.selection.clone(),
        })
    }

    fn restore(&mut self, snap: Snapshot) {
        self.doc = snap.doc;
        self.selection = snap.selection;
        self.active_vert = None;
        self.revision += 1;
    }

    /// Registra o estado atual como ponto de undo (chamar ANTES de mutar).
    pub fn checkpoint(&mut self, label: &str) {
        let snap = self.snapshot();
        self.history.record(label, &snap);
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) -> Option<String> {
        if self.modal.is_some() {
            self.cancel_modal();
            return None;
        }
        let mut cur = self.snapshot();
        let label = self.history.undo(&mut cur)?;
        self.restore(cur.0);
        self.fix_mode_after_restore();
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let mut cur = self.snapshot();
        let label = self.history.redo(&mut cur)?;
        self.restore(cur.0);
        self.fix_mode_after_restore();
        Some(label)
    }

    fn fix_mode_after_restore(&mut self) {
        if self.active_object().is_none() {
            self.mode = Mode::Object;
        }
    }

    pub fn history_labels(&self) -> Vec<&str> {
        self.history.labels()
    }

    // ------------------------------------------------------------------ objetos

    pub fn active_object(&self) -> Option<&ModelObject> {
        self.doc.active.and_then(|i| self.doc.objects.get(i))
    }

    pub fn active_object_mut(&mut self) -> Option<&mut ModelObject> {
        self.doc.active.and_then(|i| self.doc.objects.get_mut(i))
    }

    pub fn add_primitive(&mut self, kind: PrimitiveKind, at: Vec3) -> usize {
        self.checkpoint(&format!("Adicionar {}", kind.label()));
        let mut mesh = kind.build();
        mesh.name = kind.label().to_string();
        self.doc.next_id += 1;
        let id = self.doc.next_id;
        let mut obj = ModelObject {
            id,
            name: format!("{}.{:03}", kind.label(), id),
            mesh,
            transform: Transform::from_translation(at),
            visible: true,
            modifiers: Default::default(),
            auto_smooth_angle: DEFAULT_AUTO_SMOOTH,
        };
        obj.mesh.name = obj.name.clone();
        self.doc.objects.push(obj);
        let idx = self.doc.objects.len() - 1;
        self.doc.active = Some(idx);
        self.selection.clear();
        self.revision += 1;
        idx
    }

    /// Adiciona um objeto a partir de uma malha existente (importação).
    pub fn add_object_from_mesh(&mut self, name: &str, mesh: EditMesh, at: Vec3) -> usize {
        self.checkpoint(&format!("Importar {name}"));
        self.doc.next_id += 1;
        let id = self.doc.next_id;
        let mut mesh = mesh;
        mesh.name = name.to_string();
        self.doc.objects.push(ModelObject {
            id,
            name: name.to_string(),
            mesh,
            transform: Transform::from_translation(at),
            visible: true,
            modifiers: Default::default(),
            auto_smooth_angle: DEFAULT_AUTO_SMOOTH,
        });
        self.doc.active = Some(self.doc.objects.len() - 1);
        self.selection.clear();
        self.revision += 1;
        self.doc.objects.len() - 1
    }

    pub fn delete_active_object(&mut self) -> bool {
        let Some(i) = self.doc.active else {
            return false;
        };
        self.checkpoint("Excluir objeto");
        self.doc.objects.remove(i);
        self.doc.active = if self.doc.objects.is_empty() {
            None
        } else {
            Some(i.min(self.doc.objects.len() - 1))
        };
        self.selection.clear();
        self.mode = Mode::Object;
        self.revision += 1;
        true
    }

    pub fn duplicate_active_object(&mut self) -> Option<usize> {
        let src = self.active_object()?.clone();
        self.checkpoint("Duplicar objeto");
        self.doc.next_id += 1;
        let mut obj = src;
        obj.id = self.doc.next_id;
        obj.name = format!("{}.{:03}", obj.name.split('.').next().unwrap_or("Objeto"), obj.id);
        obj.transform.translation += Vec3::new(0.5, 0.0, 0.5);
        self.doc.objects.push(obj);
        self.doc.active = Some(self.doc.objects.len() - 1);
        self.revision += 1;
        self.doc.active
    }

    pub fn set_active(&mut self, idx: usize) {
        if idx < self.doc.objects.len() && self.mode == Mode::Object {
            self.doc.active = Some(idx);
            self.revision += 1;
        }
    }

    // -------------------------------------------------------------------- modos

    pub fn set_mode(&mut self, mode: Mode) {
        if self.modal.is_some() {
            self.cancel_modal();
        }
        if mode != Mode::Object && self.active_object().is_none() {
            self.status = "Selecione um objeto".into();
            return;
        }
        if self.mode != mode {
            self.selection.clear();
            self.active_vert = None;
            self.mode = mode;
            self.revision += 1;
        }
    }

    pub fn toggle_edit_mode(&mut self) {
        let target = if self.mode == Mode::Edit { Mode::Object } else { Mode::Edit };
        self.set_mode(target);
    }

    pub fn set_select_mode(&mut self, mode: SelectMode) {
        if self.select_mode == mode {
            return;
        }
        self.select_mode = mode;
        if let Some(obj) = self.active_object() {
            // converte a seleção atual para o novo modo (como o Blender: flush por vértices)
            let verts = self.selection.verts.clone();
            let base = Selection::from_verts(verts);
            self.selection = match mode {
                SelectMode::Vertex => base.flush(&obj.mesh, SelectMode::Vertex),
                SelectMode::Edge => {
                    let topo = Topology::build(&obj.mesh);
                    Selection::from_edges(base.edges_fully_selected(&topo)).flush(&obj.mesh, SelectMode::Edge)
                }
                SelectMode::Face => {
                    let faces = base.faces_fully_selected(&obj.mesh);
                    Selection::from_faces(&obj.mesh, faces)
                }
            };
        }
        self.revision += 1;
    }

    // ----------------------------------------------------------------- seleção

    pub fn select_all(&mut self) {
        if let Some(obj) = self.active_object() {
            self.selection = Selection::select_all(&obj.mesh);
            self.revision += 1;
        }
    }

    pub fn select_none(&mut self) {
        self.selection.clear();
        self.active_vert = None;
        self.revision += 1;
    }

    pub fn select_invert(&mut self) {
        if let Some(obj) = self.active_object() {
            let inv = self.selection.invert(&obj.mesh, self.select_mode);
            self.selection = inv;
            self.revision += 1;
        }
    }

    pub fn select_more(&mut self) {
        if let Some(obj) = self.active_object() {
            let faces = Selection::from_verts(self.selection.verts.clone());
            let grown = select::grow_faces(&obj.mesh, &Selection::from_faces(&obj.mesh, faces.faces_fully_selected(&obj.mesh)));
            self.selection = grown;
            self.revision += 1;
        }
    }

    pub fn select_less(&mut self) {
        if let Some(obj) = self.active_object() {
            let faces = Selection::from_verts(self.selection.verts.clone()).faces_fully_selected(&obj.mesh);
            let shrunk = select::shrink_faces(&obj.mesh, &Selection::from_faces(&obj.mesh, faces));
            self.selection = shrunk;
            self.revision += 1;
        }
    }

    pub fn select_linked(&mut self) {
        if let Some(obj) = self.active_object() {
            self.selection = select::select_linked(&obj.mesh, &self.selection);
            self.revision += 1;
        }
    }

    pub fn set_selection(&mut self, sel: Selection, active_vert: Option<u32>) {
        self.selection = sel;
        self.active_vert = active_vert;
        self.revision += 1;
    }

    /// Centróide (mundo) dos vértices selecionados — pivô padrão.
    pub fn selection_center_world(&self) -> Option<Vec3> {
        let obj = self.active_object()?;
        if self.selection.verts.is_empty() {
            return None;
        }
        let model = obj.model_matrix();
        let sum: Vec3 = self
            .selection
            .verts
            .iter()
            .map(|v| model.transform_point3(obj.mesh.co(*v)))
            .sum();
        Some(sum / self.selection.verts.len() as f32)
    }

    pub fn object_center_world(&self) -> Option<Vec3> {
        self.active_object().map(|o| o.transform.translation)
    }

    // ------------------------------------------------- transformação modal (G/R/S)

    pub fn modal_active(&self) -> bool {
        self.modal.is_some()
    }

    /// Inicia G/R/S. No modo Edição transforma os vértices selecionados; no modo Objeto, o objeto ativo.
    pub fn begin_modal(&mut self, kind: TransformKind, mouse: Vec2) -> bool {
        if self.modal.is_some() {
            return false;
        }
        let Some(idx) = self.doc.active else {
            return false;
        };
        let before = Snapshot {
            doc: self.doc.clone(),
            selection: self.selection.clone(),
        };
        let obj = &self.doc.objects[idx];
        let model = obj.model_matrix();
        let mut prop_weights: Option<Vec<f32>> = None;
        let (pivot, originals) = match self.mode {
            Mode::Edit => {
                if self.selection.verts.is_empty() {
                    self.status = "Nada selecionado".into();
                    return false;
                }
                let mut originals: Vec<(u32, Vec3)> = self
                    .selection
                    .verts
                    .iter()
                    .map(|v| (*v, model.transform_point3(obj.mesh.co(*v))))
                    .collect();
                let pivot = originals.iter().map(|(_, p)| *p).sum::<Vec3>() / originals.len() as f32;
                if self.proportional.enabled {
                    // pesos por vértice (espaço local → raio convertido pela escala média do objeto)
                    let avg_scale = (obj.transform.scale.x + obj.transform.scale.y + obj.transform.scale.z) / 3.0;
                    let selected: Vec<u32> = self.selection.verts.iter().copied().collect();
                    let w = anigo_modeling::ops::transform::proportional_weights(
                        &obj.mesh,
                        &selected,
                        self.proportional.radius / avg_scale.max(1e-4),
                        self.proportional.falloff,
                        self.proportional.connected_only,
                    );
                    originals = (0..obj.mesh.vert_count() as u32)
                        .filter(|v| w.get(*v as usize).copied().unwrap_or(0.0) > 1e-4)
                        .map(|v| (v, model.transform_point3(obj.mesh.co(v))))
                        .collect();
                    prop_weights = Some(originals.iter().map(|(v, _)| w[*v as usize]).collect());
                }
                (pivot, originals)
            }
            Mode::Object => {
                let t = obj.transform.translation;
                (t, vec![(0u32, t)])
            }
            Mode::Sculpt => return false,
        };
        let mut transform = ModalTransform::new(kind, mouse, pivot, originals);
        transform.weights = prop_weights;
        if self.snap_enabled {
            transform.snap_step = match kind {
                TransformKind::Grab => 0.25,
                _ => 1.0,
            };
        }
        self.modal = Some(ActiveModal {
            transform,
            object: idx,
            model,
            before,
        });
        true
    }

    pub fn modal_mut(&mut self) -> Option<&mut ModalTransform> {
        self.modal.as_mut().map(|m| &mut m.transform)
    }

    pub fn modal_status(&self, mouse: Vec2, ctx: &ModalContext) -> Option<String> {
        self.modal.as_ref().map(|m| m.transform.status_text(mouse, ctx))
    }

    /// Atualiza a pré-visualização ao vivo.
    pub fn update_modal(&mut self, mouse: Vec2, ctx: &ModalContext) {
        let Some(m) = self.modal.as_ref() else {
            return;
        };
        let new_positions = m.transform.apply(mouse, ctx);
        let (idx, model, mode) = (m.object, m.model, self.mode);
        match mode {
            Mode::Edit => {
                let inv = model.inverse();
                if let Some(obj) = self.doc.objects.get_mut(idx) {
                    for (v, world) in new_positions {
                        if let Some(vert) = obj.mesh.verts.get_mut(v as usize) {
                            vert.co = inv.transform_point3(world);
                        }
                    }
                }
            }
            Mode::Object => {
                if let (Some(obj), Some((_, p))) = (self.doc.objects.get_mut(idx), new_positions.first()) {
                    let kind = self.modal.as_ref().map(|m| m.transform.kind);
                    match kind {
                        Some(TransformKind::Grab) => obj.transform.translation = *p,
                        Some(TransformKind::Rotate) => {
                            if let Some(m) = self.modal.as_ref() {
                                let angle = m.transform.rotation_angle(mouse, ctx);
                                let axis = match m.transform.constraint {
                                    super::modal::Constraint::Axis(a) | super::modal::Constraint::Plane(a) => a.vector(),
                                    super::modal::Constraint::Direction(d) => d.normalize_or_zero(),
                                    super::modal::Constraint::None => -ctx.camera.forward(),
                                };
                                let base = m.before.doc.objects.get(idx).map(|o| o.transform.rotation);
                                if let Some(base) = base {
                                    obj.transform.rotation = glam::Quat::from_axis_angle(axis, angle) * base;
                                }
                            }
                        }
                        Some(TransformKind::Scale) => {
                            if let Some(m) = self.modal.as_ref() {
                                let f = m.transform.scale_factor(mouse, ctx);
                                if let Some(base) = m.before.doc.objects.get(idx).map(|o| o.transform.scale) {
                                    obj.transform.scale = (base * f).max(Vec3::splat(1e-3));
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            Mode::Sculpt => {}
        }
        self.revision += 1;
    }

    pub fn confirm_modal(&mut self) {
        if let Some(m) = self.modal.take() {
            let label = match m.transform.kind {
                TransformKind::Grab => "Mover",
                TransformKind::Rotate => "Rotacionar",
                TransformKind::Scale => "Escalar",
            };
            self.history.record(label, &SnapshotPub(m.before));
            self.revision += 1;
        }
    }

    pub fn cancel_modal(&mut self) {
        if let Some(m) = self.modal.take() {
            self.restore(m.before);
        }
    }

    // --------------------------------------------------------- operadores de edição

    /// Executa um operador sobre a malha ativa em **cópia**; só se der certo aplica, registra
    /// o undo e troca a seleção. Em erro nada muda (a malha e o histórico ficam intactos).
    pub fn edit_op<F>(&mut self, label: &str, f: F) -> Result<anigo_modeling::ops::OpOutput, anigo_modeling::ModelError>
    where
        F: FnOnce(&mut EditMesh, &Selection) -> anigo_modeling::ModelResult<anigo_modeling::ops::OpOutput>,
    {
        let idx = self.doc.active.ok_or(anigo_modeling::ModelError::EmptySelection("sem objeto ativo"))?;
        let before = self.snapshot();
        let mut mesh = self.doc.objects[idx].mesh.clone();
        let out = f(&mut mesh, &self.selection)?;
        mesh.validate()?;
        self.history.record(label, &before);
        self.doc.objects[idx].mesh = mesh;
        self.selection = out.selection.clone();
        self.active_vert = None;
        self.revision += 1;
        Ok(out)
    }

    /// Edita a **malha ativa** (UVs, costuras, atributos) de forma transacional: `f` roda numa cópia;
    /// só se der certo (e a malha validar) a cópia substitui a original. A seleção é mantida. O undo
    /// só é registrado se a malha realmente mudou (operações sem efeito não poluem o histórico).
    pub fn edit_uv<T, F>(&mut self, label: &str, f: F) -> Result<T, anigo_modeling::ModelError>
    where
        F: FnOnce(&mut EditMesh, &Selection) -> anigo_modeling::ModelResult<T>,
    {
        let idx = self.doc.active.ok_or(anigo_modeling::ModelError::EmptySelection("sem objeto ativo"))?;
        let mut mesh = self.doc.objects[idx].mesh.clone();
        let out = f(&mut mesh, &self.selection)?;
        mesh.validate()?;
        if mesh != self.doc.objects[idx].mesh {
            let before = self.snapshot();
            self.history.record(label, &before);
            self.doc.objects[idx].mesh = mesh;
            self.revision += 1;
        }
        Ok(out)
    }

    /// Registra `before` (capturado com [`capture`](Self::capture) ANTES de uma edição interativa
    /// feita diretamente na malha, como o arrasto de UVs) como ponto de undo.
    pub fn commit_snapshot(&mut self, label: &str, before: SnapshotPub) {
        self.history.record(label, &before);
        self.revision += 1;
    }

    /// Volta ao estado capturado sem mexer no histórico (cancelar uma edição interativa).
    pub fn restore_snapshot(&mut self, snap: SnapshotPub) {
        self.restore(snap.0);
    }

    /// Edita a malha ativa em **cópia** mantendo a seleção: se `f` der certo e a malha validar,
    /// registra o undo e aplica; em erro nada muda (malha e histórico intactos).
    pub fn edit_mesh<R, F>(&mut self, label: &str, f: F) -> Result<R, anigo_modeling::ModelError>
    where
        F: FnOnce(&mut EditMesh, &Selection) -> anigo_modeling::ModelResult<R>,
    {
        let idx = self.doc.active.ok_or(anigo_modeling::ModelError::EmptySelection("sem objeto ativo"))?;
        let before = self.snapshot();
        let mut mesh = self.doc.objects[idx].mesh.clone();
        let out = f(&mut mesh, &self.selection)?;
        mesh.validate()?;
        self.history.record(label, &before);
        self.doc.objects[idx].mesh = mesh;
        self.revision += 1;
        Ok(out)
    }

    /// Muda a malha ativa **sem** registrar undo (para traços contínuos já cobertos por um `checkpoint`,
    /// como a pintura de atributos). Só atributos; não use para topologia. `None` sem objeto ativo.
    pub fn mutate_active_mesh<R>(&mut self, f: impl FnOnce(&mut EditMesh) -> R) -> Option<R> {
        let obj = self.active_object_mut()?;
        let r = f(&mut obj.mesh);
        self.revision += 1;
        Some(r)
    }

    /// Altera o ângulo de auto-smooth do objeto ativo (radianos, limitado a `0..=π`) com um passo de undo.
    /// `record = false` serve para arrastos contínuos já cobertos por um `checkpoint`.
    pub fn set_auto_smooth_angle(&mut self, angle: f32, record: bool) -> bool {
        if !angle.is_finite() {
            return false;
        }
        let angle = angle.clamp(0.0, std::f32::consts::PI);
        let Some(cur) = self.active_object().map(|o| o.auto_smooth_angle) else {
            return false;
        };
        if (cur - angle).abs() < 1e-6 {
            return false;
        }
        if record {
            self.checkpoint("Ângulo de auto-smooth");
        }
        if let Some(o) = self.active_object_mut() {
            o.auto_smooth_angle = angle;
        }
        self.revision += 1;
        true
    }

    /// Estado completo atual (para o painel "ajustar última operação").
    /// Aplica a pilha de modificadores à malha base (destrutivo, com undo) e a esvazia.
    pub fn apply_modifier_stack(&mut self) -> Result<(), String> {
        let idx = self.doc.active.ok_or("sem objeto ativo")?;
        let evaluated = match self.doc.objects[idx].evaluated(anigo_modeling::modifiers::stack::EvalMode::Viewport) {
            Some(r) => r?,
            None => return Err("nenhum modificador ativo".into()),
        };
        self.checkpoint("Aplicar modificadores");
        let obj = &mut self.doc.objects[idx];
        obj.mesh = evaluated;
        obj.modifiers.modifiers.clear();
        self.selection.clear();
        self.revision += 1;
        Ok(())
    }

    pub fn capture(&self) -> SnapshotPub {
        self.snapshot()
    }

    /// Volta ao estado capturado e descarta a última entrada de undo (refazer com outros parâmetros).
    pub fn rewind_to(&mut self, snap: SnapshotPub) {
        self.history.pop_undo();
        self.restore(snap.0);
    }

    // ------------------------------------------------------------------ estatísticas

    pub fn stats(&self) -> MeshStats {
        match self.active_object() {
            Some(o) => MeshStats {
                verts: o.mesh.vert_count(),
                faces: o.mesh.face_count(),
                tris: o.mesh.triangle_count(),
                selected_verts: self.selection.verts.len(),
                selected_faces: self.selection.faces.len(),
            },
            None => MeshStats::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshStats {
    pub verts: usize,
    pub faces: usize,
    pub tris: usize,
    pub selected_verts: usize,
    pub selected_faces: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::OrbitCamera;

    fn ctx() -> ModalContext {
        ModalContext {
            camera: OrbitCamera {
                target: Vec3::new(0.0, 1.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                distance: 8.0,
                ..Default::default()
            },
            viewport: Vec2::new(800.0, 600.0),
        }
    }

    #[test]
    fn default_session_has_a_cube_and_no_undo() {
        let s = ModelingSession::with_default_cube();
        assert_eq!(s.doc.objects.len(), 1);
        assert!(!s.can_undo());
        assert_eq!(s.stats().faces, 6);
    }

    #[test]
    fn add_delete_undo_redo_objects() {
        let mut s = ModelingSession::with_default_cube();
        s.add_primitive(PrimitiveKind::QuadSphere, Vec3::ZERO);
        assert_eq!(s.doc.objects.len(), 2);
        assert!(s.delete_active_object());
        assert_eq!(s.doc.objects.len(), 1);
        assert_eq!(s.undo().as_deref(), Some("Excluir objeto"));
        assert_eq!(s.doc.objects.len(), 2);
        assert_eq!(s.redo().as_deref(), Some("Excluir objeto"));
        assert_eq!(s.doc.objects.len(), 1);
    }

    #[test]
    fn edit_mode_requires_object_and_clears_selection() {
        let mut s = ModelingSession::new();
        s.set_mode(Mode::Edit);
        assert_eq!(s.mode, Mode::Object);
        let mut s = ModelingSession::with_default_cube();
        s.toggle_edit_mode();
        assert_eq!(s.mode, Mode::Edit);
        s.select_all();
        assert!(!s.selection.verts.is_empty());
        s.toggle_edit_mode();
        assert!(s.selection.is_empty());
    }

    #[test]
    fn grab_in_edit_mode_moves_selected_vertices_and_undo_restores() {
        let mut s = ModelingSession::with_default_cube();
        s.toggle_edit_mode();
        s.select_all();
        let before = s.active_object().unwrap().mesh.clone();
        assert!(s.begin_modal(TransformKind::Grab, Vec2::new(400.0, 300.0)));
        {
            let m = s.modal_mut().unwrap();
            m.toggle_axis(super::super::modal::Axis::X, false);
            for ch in "2".chars() {
                m.push_numeric(ch);
            }
        }
        s.update_modal(Vec2::ZERO, &ctx());
        let moved = s.active_object().unwrap().mesh.co(0);
        assert!((moved.x - before.co(0).x - 2.0).abs() < 1e-5);
        s.confirm_modal();
        assert!(s.can_undo());
        assert_eq!(s.undo().as_deref(), Some("Mover"));
        assert_eq!(s.active_object().unwrap().mesh, before);
    }

    #[test]
    fn proportional_editing_moves_neighbors_with_falloff_and_not_other_islands() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Grid, Vec3::ZERO);
        s.toggle_edit_mode();
        // centro do grid selecionado
        let mesh = s.active_object().unwrap().mesh.clone();
        let center = (0..mesh.vert_count() as u32).find(|&v| mesh.co(v).length() < 1e-5).unwrap();
        s.set_selection(Selection::from_verts([center]), Some(center));
        s.proportional.enabled = true;
        s.proportional.radius = 1.5;
        s.begin_modal(TransformKind::Grab, Vec2::ZERO);
        {
            let m = s.modal_mut().unwrap();
            m.toggle_axis(super::super::modal::Axis::Y, false);
            m.push_numeric('1');
        }
        s.update_modal(Vec2::ZERO, &ctx());
        let moved = &s.active_object().unwrap().mesh;
        let dy = |v: u32| moved.co(v).y - mesh.co(v).y;
        assert!((dy(center) - 1.0).abs() < 1e-5, "o selecionado move 100%");
        let near = (0..mesh.vert_count() as u32).filter(|&v| v != center && (mesh.co(v) - mesh.co(center)).length() < 0.6).next().unwrap();
        assert!(dy(near) > 0.0 && dy(near) < 1.0, "vizinho move parcialmente: {}", dy(near));
        let far = (0..mesh.vert_count() as u32).find(|&v| (mesh.co(v) - mesh.co(center)).length() > 1.9).unwrap();
        assert!(dy(far).abs() < 1e-6, "fora do raio não move");
        s.cancel_modal();
        assert_eq!(s.active_object().unwrap().mesh, mesh);
    }

    #[test]
    fn cancel_modal_restores_exactly() {
        let mut s = ModelingSession::with_default_cube();
        s.toggle_edit_mode();
        s.select_all();
        let before = s.active_object().unwrap().mesh.clone();
        s.begin_modal(TransformKind::Scale, Vec2::new(500.0, 300.0));
        s.update_modal(Vec2::new(650.0, 300.0), &ctx());
        assert_ne!(s.active_object().unwrap().mesh, before);
        s.cancel_modal();
        assert_eq!(s.active_object().unwrap().mesh, before);
        assert!(!s.modal_active());
        assert!(!s.can_undo(), "cancelar não deixa entrada no histórico");
    }

    #[test]
    fn object_mode_grab_rotate_scale_update_transform() {
        let mut s = ModelingSession::with_default_cube();
        let c = ctx();
        s.begin_modal(TransformKind::Grab, Vec2::new(400.0, 300.0));
        s.update_modal(Vec2::new(500.0, 300.0), &c);
        assert!(s.active_object().unwrap().transform.translation.x > 0.1);
        s.confirm_modal();
        let t0 = s.active_object().unwrap().transform;
        s.begin_modal(TransformKind::Scale, Vec2::new(500.0, 300.0));
        {
            let m = s.modal_mut().unwrap();
            m.push_numeric('2');
        }
        s.update_modal(Vec2::ZERO, &c);
        assert!((s.active_object().unwrap().transform.scale - t0.scale * 2.0).length() < 1e-5);
        s.confirm_modal();
        assert_eq!(s.history_labels().len(), 2);
    }

    #[test]
    fn selection_ops_work_in_edit_mode() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Grid, Vec3::ZERO);
        s.toggle_edit_mode();
        s.set_select_mode(SelectMode::Face);
        s.select_all();
        assert_eq!(s.selection.faces.len(), 64);
        s.select_none();
        assert!(s.selection.is_empty());
        s.select_invert();
        assert_eq!(s.selection.faces.len(), 64);
        let mut s2 = ModelingSession::new();
        s2.add_primitive(PrimitiveKind::Grid, Vec3::ZERO);
        s2.toggle_edit_mode();
        s2.set_selection(Selection::from_faces(&s2.active_object().unwrap().mesh, [27u32]), None);
        s2.select_more();
        assert_eq!(s2.selection.faces.len(), 9);
        s2.select_less();
        assert_eq!(s2.selection.faces.len(), 1);
    }

    #[test]
    fn mode_switch_preserves_object_world_transform_during_edit_modal() {
        // objeto transladado: o modal em mundo + conversão inversa não pode "pular" vértices
        let mut s = ModelingSession::with_default_cube(); // cubo em (0,1,0)
        s.toggle_edit_mode();
        s.select_all();
        s.begin_modal(TransformKind::Grab, Vec2::new(0.0, 0.0));
        s.modal_mut().unwrap().toggle_axis(super::super::modal::Axis::Y, false);
        s.modal_mut().unwrap().push_numeric('1');
        s.update_modal(Vec2::ZERO, &ctx());
        let obj = s.active_object().unwrap();
        // em espaço local, o cubo inteiro subiu 1; em mundo, o centro passa de y=1 para y=2
        let c = obj.mesh.centroid();
        assert!((c.y - 1.0).abs() < 1e-5);
        assert!((obj.model_matrix().transform_point3(c).y - 2.0).abs() < 1e-5);
    }

    #[test]
    fn interactive_uv_edit_commit_undo_and_cancel_restore_exactly() {
        let mut s = ModelingSession::with_default_cube();
        s.toggle_edit_mode();
        let before = s.active_object().unwrap().mesh.clone();
        // edição interativa: captura, muta direto, confirma
        let snap = s.capture();
        s.active_object_mut().unwrap().mesh.faces[0].corners[0].uv.x += 0.25;
        s.commit_snapshot("Mover UV", snap);
        assert_eq!(s.history_labels().last().copied(), Some("Mover UV"));
        assert_eq!(s.undo().as_deref(), Some("Mover UV"));
        assert_eq!(s.active_object().unwrap().mesh, before);
        // cancelar: restaura sem deixar entrada de histórico
        let depth = s.history_labels().len();
        let snap = s.capture();
        s.active_object_mut().unwrap().mesh.faces[1].corners[2].uv.y -= 0.5;
        s.restore_snapshot(snap);
        assert_eq!(s.active_object().unwrap().mesh, before);
        assert_eq!(s.history_labels().len(), depth);
    }

    #[test]
    fn edit_uv_is_transactional_and_skips_noop_entries() {
        let mut s = ModelingSession::with_default_cube();
        s.toggle_edit_mode();
        let depth = s.history_labels().len();
        assert!(s.edit_uv("nada", |_, _| Ok(())).is_ok());
        assert_eq!(s.history_labels().len(), depth, "sem mudança não registra undo");
        let r: Result<(), _> = s.edit_uv("falha", |m, _| {
            m.faces[0].corners[0].uv.x = 9.0;
            Err(anigo_modeling::ModelError::EmptySelection("x"))
        });
        assert!(r.is_err());
        assert_ne!(s.active_object().unwrap().mesh.faces[0].corners[0].uv.x, 9.0);
        s.edit_uv("muda", |m, _| {
            m.faces[0].corners[0].uv.x = 9.0;
            Ok(())
        })
        .unwrap();
        assert_eq!(s.history_labels().len(), depth + 1);
    }

    #[test]
    fn duplicate_creates_distinct_object() {
        let mut s = ModelingSession::with_default_cube();
        let i = s.duplicate_active_object().unwrap();
        assert_eq!(i, 1);
        assert_ne!(s.doc.objects[0].id, s.doc.objects[1].id);
        assert_ne!(s.doc.objects[0].transform.translation, s.doc.objects[1].transform.translation);
    }

    #[test]
    fn document_serde_roundtrip() {
        let mut s = ModelingSession::with_default_cube();
        s.add_primitive(PrimitiveKind::Torus, Vec3::X);
        let json = serde_json::to_string(&s.doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objects.len(), 2);
        assert_eq!(back.objects[1].mesh, s.doc.objects[1].mesh);
    }

    // ------------------------------------------------------------ normais / auto-smooth

    #[test]
    fn auto_smooth_angle_defaults_and_roundtrips_in_serde() {
        let mut s = ModelingSession::with_default_cube();
        assert!((s.doc.objects[0].auto_smooth_angle - DEFAULT_AUTO_SMOOTH).abs() < 1e-6);
        assert!(s.set_auto_smooth_angle(75f32.to_radians(), true));
        let json = serde_json::to_string(&s.doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert!((back.objects[0].auto_smooth_angle - 75f32.to_radians()).abs() < 1e-6);
        // documento antigo, sem o campo: assume o padrão (30°)
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["objects"][0].as_object_mut().unwrap().remove("auto_smooth_angle");
        let old: Document = serde_json::from_value(v).unwrap();
        assert!((old.objects[0].auto_smooth_angle - DEFAULT_AUTO_SMOOTH).abs() < 1e-6);
    }

    #[test]
    fn auto_smooth_angle_feeds_normal_params_and_core_mesh() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        // cubo suave: com 30° as quinas de 90° continuam duras; com 180° o corte desaparece e
        // as normais se fundem (menos vértices de render, mesmo com UVs por face)
        for f in &mut s.doc.objects[0].mesh.faces {
            f.smooth = true;
            for c in &mut f.corners {
                c.uv = glam::Vec2::ZERO; // sem costuras de UV: só as normais separam vértices
            }
        }
        let hard = s.doc.objects[0].to_core().unwrap().vertices.len();
        assert_eq!(hard, 24);
        s.set_auto_smooth_angle(std::f32::consts::PI, true);
        let o = &s.doc.objects[0];
        assert!(o.normal_params().auto_smooth_angle.is_none());
        assert_eq!(o.to_core().unwrap().vertices.len(), 8);
        s.set_auto_smooth_angle(100f32.to_radians(), true);
        assert!((s.doc.objects[0].normal_params().auto_smooth_angle.unwrap() - 100f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn set_auto_smooth_angle_is_undoable_and_ignores_invalid() {
        let mut s = ModelingSession::with_default_cube();
        let before = s.doc.objects[0].auto_smooth_angle;
        assert!(!s.set_auto_smooth_angle(f32::NAN, true));
        assert!(!s.set_auto_smooth_angle(before, true), "mesmo valor não gera undo");
        assert!(!s.can_undo());
        assert!(s.set_auto_smooth_angle(1.0, true));
        assert!(s.undo().is_some());
        assert_eq!(s.doc.objects[0].auto_smooth_angle, before);
    }

    #[test]
    fn render_hash_sees_custom_normals_colors_and_angle() {
        let mut s = ModelingSession::with_default_cube();
        let base = s.doc.objects[0].render_hash();
        assert_eq!(base, s.doc.objects[0].render_hash());
        s.doc.objects[0].mesh.faces[0].corners[0].normal = Some(Vec3::Y);
        let with_normal = s.doc.objects[0].render_hash();
        assert_ne!(base, with_normal);
        s.doc.objects[0].mesh.verts[0].color[0] = 0.25;
        let with_color = s.doc.objects[0].render_hash();
        assert_ne!(with_normal, with_color);
        s.doc.objects[0].auto_smooth_angle = 1.0;
        assert_ne!(with_color, s.doc.objects[0].render_hash());
    }

    #[test]
    fn edit_mesh_keeps_selection_and_undo_is_exact() {
        let mut s = ModelingSession::with_default_cube();
        s.set_mode(Mode::Edit);
        s.select_all();
        let sel_before = s.selection.clone();
        let mesh_before = s.doc.objects[0].mesh.clone();
        let n = s
            .edit_mesh("teste", |m, _| {
                m.verts[0].color[0] = 0.0;
                Ok(7usize)
            })
            .unwrap();
        assert_eq!(n, 7);
        assert_eq!(s.selection, sel_before);
        assert_ne!(s.doc.objects[0].mesh, mesh_before);
        s.undo();
        assert_eq!(s.doc.objects[0].mesh, mesh_before);
        // erro do operador: nada muda e nada entra no histórico
        let labels = s.history_labels().len();
        let r: Result<(), _> = s.edit_mesh("falha", |_, _| Err(anigo_modeling::ModelError::EmptySelection("x")));
        assert!(r.is_err());
        assert_eq!(s.history_labels().len(), labels);
        assert_eq!(s.doc.objects[0].mesh, mesh_before);
    }
}
