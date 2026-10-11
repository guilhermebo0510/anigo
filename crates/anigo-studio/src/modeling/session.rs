use super::history::History;
use super::modal::{ModalContext, ModalTransform, TransformKind};
use anigo_core::math::Transform;
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
        };
        obj.mesh.name = obj.name.clone();
        self.doc.objects.push(obj);
        let idx = self.doc.objects.len() - 1;
        self.doc.active = Some(idx);
        self.selection.clear();
        self.revision += 1;
        idx
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
        let (pivot, originals) = match self.mode {
            Mode::Edit => {
                if self.selection.verts.is_empty() {
                    self.status = "Nada selecionado".into();
                    return false;
                }
                let originals: Vec<(u32, Vec3)> = self
                    .selection
                    .verts
                    .iter()
                    .map(|v| (*v, model.transform_point3(obj.mesh.co(*v))))
                    .collect();
                let pivot = originals.iter().map(|(_, p)| *p).sum::<Vec3>() / originals.len() as f32;
                (pivot, originals)
            }
            Mode::Object => {
                let t = obj.transform.translation;
                (t, vec![(0u32, t)])
            }
            Mode::Sculpt => return false,
        };
        let mut transform = ModalTransform::new(kind, mouse, pivot, originals);
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
}
