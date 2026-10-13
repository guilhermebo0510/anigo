//! Estado e lógica (sem egui) do Modo Escultura: pincéis por tipo, `SculptContext` + `TriBvh` do objeto
//! ativo, traço (`StrokeState`), undo por checkpoint da sessão, ferramentas de malha inteira
//! (máscara, voxel remesh, simetrizar, suavizar tudo) e o espelho CPU do sólido para a GPU.
//!
//! A UI (`app/sculpt_ui.rs`) só traduz entrada em chamadas daqui; o motor (`anigo_modeling::sculpt`)
//! deforma a malha. Toda alteração persistente passa por `ModelingSession` com checkpoint.

use super::session::{ModelObject, ModelingSession};
use crate::viewport::OrbitCamera;
use crate::workbench::sculpt_mesh::SculptMirror;
use anigo_modeling::bvh::TriBvh;
use anigo_modeling::ops::OpOutput;
use anigo_modeling::sculpt::automask::AutoMask;
use anigo_modeling::sculpt::brush::{
    apply_dab, mask_clear_all, mask_invert_all, Brush, BrushKind, Dab, SculptContext,
};
use anigo_modeling::sculpt::dyntopo::{dyntopo_step, DyntopoParams};
use anigo_modeling::sculpt::falloff::CurveProfile;
use anigo_modeling::sculpt::stroke::{StrokeSample, StrokeState, StrokeUpdate};
use anigo_modeling::sculpt::symmetry::{symmetrize_mesh, Symmetry, SymmetrizeDirection};
use anigo_modeling::sculpt::voxel_remesh::{voxel_remesh_report, VoxelParams};
use anigo_modeling::{EditMesh, ModelError};
use glam::{Mat3, Mat4, Vec2, Vec3};

// ------------------------------------------------------------------ pincéis

/// Os 20 pincéis, na ordem do shelf (`app/inspector.rs::MODELAGEM_SCULPT`).
pub const SCULPT_BRUSHES: [BrushKind; 20] = [
    BrushKind::Draw,
    BrushKind::Clay,
    BrushKind::ClayStrips,
    BrushKind::Smooth,
    BrushKind::Flatten,
    BrushKind::Inflate,
    BrushKind::Pinch,
    BrushKind::Grab,
    BrushKind::SnakeHook,
    BrushKind::Crease,
    BrushKind::MaskPaint,
    BrushKind::Fill,
    BrushKind::Scrape,
    BrushKind::Nudge,
    BrushKind::Layer,
    BrushKind::Elastic,
    BrushKind::MaskSmooth,
    BrushKind::MaskSharpen,
    BrushKind::MaskInvert,
    BrushKind::MaskClear,
];

/// Id da ferramenta do shelf para um pincel (`sculpt.<nome>`; o pincel de máscara é `sculpt.mask`).
pub fn brush_tool_id(kind: BrushKind) -> &'static str {
    match kind {
        BrushKind::Draw => "sculpt.draw",
        BrushKind::Clay => "sculpt.clay",
        BrushKind::ClayStrips => "sculpt.clay_strips",
        BrushKind::Smooth => "sculpt.smooth",
        BrushKind::Flatten => "sculpt.flatten",
        BrushKind::Fill => "sculpt.fill",
        BrushKind::Scrape => "sculpt.scrape",
        BrushKind::Pinch => "sculpt.pinch",
        BrushKind::Inflate => "sculpt.inflate",
        BrushKind::Grab => "sculpt.grab",
        BrushKind::SnakeHook => "sculpt.snake_hook",
        BrushKind::Nudge => "sculpt.nudge",
        BrushKind::Crease => "sculpt.crease",
        BrushKind::Layer => "sculpt.layer",
        BrushKind::Elastic => "sculpt.elastic",
        BrushKind::MaskPaint => "sculpt.mask",
        BrushKind::MaskSmooth => "sculpt.mask_smooth",
        BrushKind::MaskSharpen => "sculpt.mask_sharpen",
        BrushKind::MaskInvert => "sculpt.mask_invert",
        BrushKind::MaskClear => "sculpt.mask_clear",
    }
}

/// `tool.id` do shelf → pincel.
pub fn tool_id_to_brush(id: &str) -> Option<BrushKind> {
    SCULPT_BRUSHES.iter().copied().find(|&k| brush_tool_id(k) == id)
}

/// Índice da ferramenta do shelf (`tool_idx`) → pincel, usando a tabela do modo Escultura.
pub fn brush_for_tool_index(idx: usize) -> Option<BrushKind> {
    crate::app::inspector::MODELAGEM_SCULPT
        .get(idx)
        .and_then(|t| tool_id_to_brush(t.id))
}

/// Posição do pincel na tabela do shelf (inverso de `brush_for_tool_index`).
pub fn tool_index_for_brush(kind: BrushKind) -> Option<usize> {
    crate::app::inspector::MODELAGEM_SCULPT
        .iter()
        .position(|t| t.id == brush_tool_id(kind))
}

pub const RADIUS_MIN_PX: f32 = 6.0;
pub const RADIUS_MAX_PX: f32 = 600.0;
pub const STRENGTH_MAX: f32 = 2.0;

/// Configuração persistente de um pincel (raio em pixels de tela; o raio de mundo sai da profundidade).
#[derive(Debug, Clone)]
pub struct BrushSlot {
    pub brush: Brush,
    pub radius_px: f32,
    /// Curva do falloff personalizado (editor do painel).
    pub curve: CurveProfile,
}

impl BrushSlot {
    pub fn new(kind: BrushKind) -> Self {
        Self {
            brush: Brush::new(kind),
            radius_px: 56.0,
            curve: CurveProfile::smooth(),
        }
    }
}

// ------------------------------------------------------------------ geometria pura

/// Raio de mundo (câmera) → local: usa a escala média do objeto.
pub fn mean_scale(model: Mat4) -> f32 {
    let (x, y, z) = (model.x_axis.truncate().length(), model.y_axis.truncate().length(), model.z_axis.truncate().length());
    let s = (x + y + z) / 3.0;
    if s.is_finite() && s > 1e-9 {
        s
    } else {
        1.0
    }
}

/// Tamanho de 1 pixel de tela em unidades de mundo à profundidade de `world_point`.
pub fn world_per_pixel(camera: &OrbitCamera, viewport_h_px: f32, world_point: Vec3) -> f32 {
    let h = viewport_h_px.max(1.0);
    let depth = if camera.orthographic {
        camera.distance
    } else {
        (world_point - camera.eye()).dot(camera.forward()).max(camera.near)
    };
    2.0 * depth * (camera.fov_y * 0.5).tan() / h
}

/// Raio do pincel em pixels → raio no espaço LOCAL do objeto no ponto `world_point`.
pub fn px_to_local_radius(camera: &OrbitCamera, viewport_h_px: f32, world_point: Vec3, px: f32, model: Mat4) -> f32 {
    (px * world_per_pixel(camera, viewport_h_px, world_point) / mean_scale(model)).max(1e-6)
}

/// Raio de mundo (origem, direção) → espaço local do objeto (direção normalizada).
pub fn ray_to_local(model: Mat4, origin: Vec3, dir: Vec3) -> (Vec3, Vec3) {
    let inv = model.inverse();
    (inv.transform_point3(origin), inv.transform_vector3(dir).normalize_or_zero())
}

/// Normal local → mundo (inversa transposta, correta para escala não uniforme).
pub fn normal_to_world(model: Mat4, n: Vec3) -> Vec3 {
    let m3 = Mat3::from_mat4(model);
    (m3.inverse().transpose() * n).normalize_or_zero()
}

/// Anel do pincel no plano tangente (espaço local), levado ao mundo. `segments + 1` pontos (fecha o anel).
pub fn ring_points_world(model: Mat4, center_local: Vec3, normal_local: Vec3, radius_local: f32, segments: usize) -> Vec<Vec3> {
    let n = normal_local.try_normalize().unwrap_or(Vec3::Y);
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = n.cross(helper).normalize_or_zero();
    let v = n.cross(u).normalize_or_zero();
    let segs = segments.max(8);
    (0..=segs)
        .map(|i| {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            model.transform_point3(center_local + (u * a.cos() + v * a.sin()) * radius_local)
        })
        .collect()
}

/// Modal F / Shift+F: redimensiona raio ou força arrastando.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resize {
    pub strength: bool,
    /// Raio: centro do círculo na tela. Força: posição inicial do mouse.
    pub center: Vec2,
    pub start_value: f32,
}

impl Resize {
    /// Começa o ajuste do raio com a borda do círculo sob o mouse (como o Blender).
    pub fn radius(mouse: Vec2, radius_px: f32) -> Self {
        Self {
            strength: false,
            center: mouse - Vec2::new(radius_px, 0.0),
            start_value: radius_px,
        }
    }

    pub fn strength(mouse: Vec2, strength: f32) -> Self {
        Self {
            strength: true,
            center: mouse,
            start_value: strength,
        }
    }

    /// Valor correspondente à posição atual do mouse.
    pub fn value_at(&self, mouse: Vec2) -> f32 {
        if self.strength {
            (self.start_value + (mouse.x - self.center.x) / 400.0).clamp(0.0, STRENGTH_MAX)
        } else {
            (mouse - self.center).length().clamp(RADIUS_MIN_PX, RADIUS_MAX_PX)
        }
    }
}

/// Voxel sugerido: ~1/64 da maior dimensão da malha (limitado para não explodir a grade).
pub fn suggest_voxel_size(mesh: &EditMesh) -> f32 {
    mesh.bounds()
        .map(|(lo, hi)| ((hi - lo).max_element() / 64.0).max(1e-4))
        .unwrap_or(0.05)
}

// ------------------------------------------------------------------ estado

/// Câmera + tamanho do viewport (pixels lógicos) usados para raio de tela e projeção.
#[derive(Debug, Clone, Copy)]
pub struct ViewInfo {
    pub camera: OrbitCamera,
    pub size: Vec2,
}

impl ViewInfo {
    pub fn ray(&self, px: Vec2) -> (Vec3, Vec3) {
        self.camera.ray_from_pixel(px, self.size)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrokeMods {
    /// Ctrl: inverte o efeito (Draw ↔ rebaixar…).
    pub invert: bool,
    /// Shift: Smooth temporário.
    pub smooth: bool,
}

/// Ponto sob o cursor (para o círculo do pincel).
#[derive(Debug, Clone, Copy)]
pub struct Hover {
    pub local_point: Vec3,
    pub local_normal: Vec3,
    pub world_point: Vec3,
    pub world_normal: Vec3,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StrokeSummary {
    pub dabs: usize,
    pub topology_changed: bool,
    pub millis: f32,
}

#[derive(Debug, Clone)]
struct Bound {
    obj_id: u32,
    revision: u64,
    verts: usize,
    faces: usize,
}

struct Run {
    obj_idx: usize,
    obj_id: u32,
    kind: BrushKind,
    dabs: usize,
    topology_changed: bool,
    dyntopo_edge: Option<f32>,
    started: std::time::Instant,
}

/// Traço roteirizado (cenários de screenshot e testes): posições normalizadas (0..1) do viewport.
#[derive(Debug, Clone)]
pub struct ScriptedStroke {
    pub brush: BrushKind,
    pub invert: bool,
    pub paths: Vec<Vec<Vec2>>,
    /// Posição (normalizada) onde fica o círculo do pincel depois do script.
    pub hover: Option<Vec2>,
}

pub struct SculptState {
    pub slots: Vec<BrushSlot>,
    pub active: BrushKind,
    pub symmetry: Symmetry,
    pub dyntopo: bool,
    pub dyntopo_detail_px: f32,
    /// 0 = automático (`suggest_voxel_size`).
    pub voxel_size: f32,
    pub voxel_smooth: u32,
    pub smooth_all_iters: u32,
    pub symmetrize_axis: usize,
    /// O botão primário foi apertado dentro do viewport em escultura (suprime caixa de seleção etc.).
    pub press_captured: bool,
    pub hover: Option<Hover>,
    /// Último erro amigável (painel e status).
    pub error: Option<String>,
    pub resize: Option<Resize>,
    pub script: Option<ScriptedStroke>,
    /// Segundo traço roteirizado, executado depois de `script`.
    pub followup: Option<ScriptedStroke>,
    /// Posição (px do viewport) forçada para o círculo do pincel (cenários).
    pub hover_override: Option<Vec2>,
    pub last_stroke: Option<StrokeSummary>,
    /// Incrementa quando o sólido da GPU precisa ser recriado inteiro (topologia/objeto/undo).
    pub gpu_gen: u64,
    pub mirror: Option<SculptMirror>,
    ctx: Option<SculptContext>,
    bvh: Option<TriBvh>,
    bound: Option<Bound>,
    stroke: StrokeState,
    run: Option<Run>,
}

impl Default for SculptState {
    fn default() -> Self {
        Self::new()
    }
}

fn kind_slot_index(kind: BrushKind) -> usize {
    SCULPT_BRUSHES.iter().position(|&k| k == kind).unwrap_or(0)
}

fn err_text(e: &ModelError) -> String {
    e.to_string()
}

impl SculptState {
    pub fn new() -> Self {
        Self {
            slots: SCULPT_BRUSHES.iter().map(|&k| BrushSlot::new(k)).collect(),
            active: BrushKind::Draw,
            symmetry: Symmetry::none(),
            dyntopo: false,
            dyntopo_detail_px: 14.0,
            voxel_size: 0.0,
            voxel_smooth: 0,
            smooth_all_iters: 3,
            symmetrize_axis: 0,
            press_captured: false,
            hover: None,
            error: None,
            resize: None,
            script: None,
            followup: None,
            hover_override: None,
            last_stroke: None,
            gpu_gen: 0,
            mirror: None,
            ctx: None,
            bvh: None,
            bound: None,
            stroke: StrokeState::new(),
            run: None,
        }
    }

    pub fn slot(&self, kind: BrushKind) -> &BrushSlot {
        &self.slots[kind_slot_index(kind)]
    }

    pub fn slot_mut(&mut self, kind: BrushKind) -> &mut BrushSlot {
        let i = kind_slot_index(kind);
        &mut self.slots[i]
    }

    pub fn active_slot(&self) -> &BrushSlot {
        self.slot(self.active)
    }

    pub fn active_slot_mut(&mut self) -> &mut BrushSlot {
        let k = self.active;
        self.slot_mut(k)
    }

    pub fn is_stroking(&self) -> bool {
        self.stroke.is_active()
    }

    pub fn ctx(&self) -> Option<&SculptContext> {
        self.ctx.as_ref()
    }

    pub fn bvh(&self) -> Option<&TriBvh> {
        self.bvh.as_ref()
    }

    /// `true` se o contexto/BVH descrevem o objeto ativo.
    pub fn is_bound(&self) -> bool {
        self.ctx.is_some() && self.bvh.is_some()
    }

    /// Esquece o objeto (ao sair do modo).
    pub fn clear(&mut self) {
        self.ctx = None;
        self.bvh = None;
        self.bound = None;
        self.mirror = None;
        self.hover = None;
        self.run = None;
        self.gpu_gen += 1;
    }

    // ------------------------------------------------------------ vínculo com a malha

    /// Garante que contexto, BVH e espelho descrevem o objeto ativo na revisão atual da sessão.
    /// Reconstrói (custo O(faces)) quando o objeto, a topologia ou a revisão mudaram — undo/redo, voxel
    /// remesh etc. Não faz nada durante um traço.
    pub fn sync(&mut self, session: &ModelingSession) {
        if self.stroke.is_active() {
            return;
        }
        let Some(obj) = session.active_object() else {
            if self.bound.is_some() {
                self.clear();
            }
            return;
        };
        if let Some(b) = &self.bound {
            if b.obj_id == obj.id && b.revision == session.revision && b.verts == obj.mesh.verts.len() && b.faces == obj.mesh.faces.len() {
                return;
            }
        }
        self.rebuild(obj, session.revision);
    }

    fn rebuild(&mut self, obj: &ModelObject, revision: u64) {
        self.gpu_gen += 1;
        self.hover = None;
        match SculptContext::new(&obj.mesh) {
            Ok(ctx) => {
                self.bvh = Some(TriBvh::build(&obj.mesh));
                self.mirror = Some(SculptMirror::build(&obj.mesh, &ctx.vert_normals, BASE_COLOR));
                self.ctx = Some(ctx);
                self.error = None;
            }
            Err(e) => {
                self.ctx = None;
                self.bvh = None;
                self.mirror = None;
                self.error = Some(format!("Malha inválida para escultura: {}", err_text(&e)));
            }
        }
        self.bound = Some(Bound {
            obj_id: obj.id,
            revision,
            verts: obj.mesh.verts.len(),
            faces: obj.mesh.faces.len(),
        });
    }

    fn rebuild_engine_after_topology_change(&mut self, mesh: &EditMesh) {
        match SculptContext::new(mesh) {
            Ok(ctx) => {
                self.bvh = Some(TriBvh::build(mesh));
                self.mirror = Some(SculptMirror::build(mesh, &ctx.vert_normals, BASE_COLOR));
                self.ctx = Some(ctx);
            }
            Err(e) => {
                self.error = Some(format!("Malha inválida após o dyntopo: {}", err_text(&e)));
                self.ctx = None;
                self.bvh = None;
                self.mirror = None;
            }
        }
        self.gpu_gen += 1;
        if let Some(b) = self.bound.as_mut() {
            b.verts = mesh.verts.len();
            b.faces = mesh.faces.len();
        }
    }

    /// Depois de uma edição local feita aqui (traço, máscara): incrementa a revisão da sessão e
    /// registra que o contexto continua válido, com o espelho atualizado nos vértices `dirty`.
    fn commit_local_edit(&mut self, session: &mut ModelingSession, dirty: &[u32]) {
        session.revision += 1;
        if let (Some(obj), Some(ctx), Some(mirror)) = (session.active_object(), self.ctx.as_ref(), self.mirror.as_mut()) {
            mirror.update(&obj.mesh, &ctx.vert_normals, dirty);
        }
        if let Some(b) = self.bound.as_mut() {
            b.revision = session.revision;
        }
    }

    // ------------------------------------------------------------ hover

    /// Raycast do cursor (raio de MUNDO) na malha ativa; atualiza `hover`.
    pub fn update_hover(&mut self, session: &ModelingSession, ray_world: (Vec3, Vec3)) {
        self.hover = None;
        let (Some(obj), Some(bvh), Some(ctx)) = (session.active_object(), self.bvh.as_ref(), self.ctx.as_ref()) else {
            return;
        };
        let model = obj.model_matrix();
        let (o, d) = ray_to_local(model, ray_world.0, ray_world.1);
        if d == Vec3::ZERO {
            return;
        }
        let Some(hit) = bvh.raycast(o, d, f32::MAX) else {
            return;
        };
        let mut n = Vec3::ZERO;
        if let Some(tr) = bvh.tri_ref(hit.tri_index) {
            for k in 0..3 {
                if let Some(vn) = ctx.vert_normals.get(tr.verts[k] as usize) {
                    n += *vn * hit.bary[k];
                }
            }
        }
        let n = n.try_normalize().unwrap_or(hit.normal);
        self.hover = Some(Hover {
            local_point: hit.point,
            local_normal: n,
            world_point: model.transform_point3(hit.point),
            world_normal: normal_to_world(model, n),
        });
    }

    // ------------------------------------------------------------ traço

    /// Inicia um traço com o raio de MUNDO do cursor. `Ok(false)` se o raio não acerta o objeto.
    /// Registra o checkpoint (undo) ANTES de mexer na malha.
    pub fn begin_stroke(
        &mut self,
        session: &mut ModelingSession,
        view: &ViewInfo,
        ray_world: (Vec3, Vec3),
        pressure: f32,
        time: f64,
        mods: StrokeMods,
    ) -> Result<bool, String> {
        if self.stroke.is_active() {
            return Err("já existe um traço em andamento".into());
        }
        self.sync(session);
        let Some(idx) = session.doc.active else {
            return Err("Selecione um objeto".into());
        };
        let (model, obj_id) = {
            let o = &session.doc.objects[idx];
            (o.model_matrix(), o.id)
        };
        if self.ctx.is_none() || self.bvh.is_none() {
            return Err(self.error.clone().unwrap_or_else(|| "Escultura indisponível para este objeto".into()));
        }
        let (Some(ctx), Some(bvh)) = (self.ctx.as_mut(), self.bvh.as_mut()) else {
            return Ok(false);
        };
        let (lo, ld) = ray_to_local(model, ray_world.0, ray_world.1);
        if ld == Vec3::ZERO {
            return Ok(false);
        }
        let Some(hit) = bvh.raycast(lo, ld, f32::MAX) else {
            return Ok(false);
        };
        let world_hit = model.transform_point3(hit.point);
        let kind = if mods.smooth && !self.active.is_mask() { BrushKind::Smooth } else { self.active };
        let slot = &self.slots[kind_slot_index(kind)];
        let mut brush = slot.brush.clone();
        if mods.invert && !mods.smooth {
            brush.direction_invert = !brush.direction_invert;
        }
        brush.radius = px_to_local_radius(&view.camera, view.size.y, world_hit, slot.radius_px, model);
        let dyn_edge = (self.dyntopo && !kind.is_mask() && !kind.uses_frozen_set())
            .then(|| px_to_local_radius(&view.camera, view.size.y, world_hit, self.dyntopo_detail_px, model));

        let label = format!("Escultura: {}", crate::i18n::tr(brush_tool_id(kind)));
        session.checkpoint(&label);
        let started = std::time::Instant::now();
        let sample = StrokeSample {
            ray_origin: lo,
            ray_dir: ld,
            pressure: pressure.clamp(0.0, 1.0),
            time,
        };
        let sym = self.symmetry.clone();
        let Some(obj) = session.doc.objects.get_mut(idx) else {
            session.discard_checkpoint();
            return Err("Objeto ativo inválido".into());
        };
        let upd = match self.stroke.begin(&mut obj.mesh, bvh, ctx, &brush, &sym, &sample) {
            Ok(u) => u,
            Err(e) => {
                session.discard_checkpoint();
                let msg = format!("Traço recusado: {}", err_text(&e));
                self.error = Some(msg.clone());
                return Err(msg);
            }
        };
        if !upd.started {
            session.discard_checkpoint();
            return Ok(false);
        }
        self.run = Some(Run {
            obj_idx: idx,
            obj_id,
            kind,
            dabs: upd.dabs,
            topology_changed: false,
            dyntopo_edge: dyn_edge,
            started,
        });
        self.absorb(session, &upd);
        Ok(true)
    }

    /// Nova amostra do traço (raio de MUNDO). No-op sem traço.
    pub fn continue_stroke(&mut self, session: &mut ModelingSession, ray_world: (Vec3, Vec3), pressure: f32, time: f64) {
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let (idx, obj_id) = (run.obj_idx, run.obj_id);
        let Some(obj) = session.doc.objects.get_mut(idx) else {
            return;
        };
        if obj.id != obj_id {
            return;
        }
        let model = obj.model_matrix();
        let (lo, ld) = ray_to_local(model, ray_world.0, ray_world.1);
        if ld == Vec3::ZERO {
            return;
        }
        let (Some(ctx), Some(bvh)) = (self.ctx.as_mut(), self.bvh.as_mut()) else {
            return;
        };
        let sample = StrokeSample {
            ray_origin: lo,
            ray_dir: ld,
            pressure: pressure.clamp(0.0, 1.0),
            time,
        };
        match self.stroke.continue_stroke(&mut obj.mesh, bvh, ctx, &sample) {
            Ok(upd) => {
                self.absorb(session, &upd);
                self.apply_dyntopo(session, &upd);
            }
            Err(e) => self.error = Some(format!("Erro no traço: {}", err_text(&e))),
        }
    }

    /// Atualiza o espelho da GPU com os vértices tocados (e o anel de vizinhos, por causa das normais).
    fn absorb(&mut self, session: &ModelingSession, upd: &StrokeUpdate) {
        if let Some(run) = self.run.as_mut() {
            run.dabs += upd.dabs;
        }
        let (Some(ctx), Some(mirror)) = (self.ctx.as_ref(), self.mirror.as_mut()) else {
            return;
        };
        let Some(obj) = session.active_object() else {
            return;
        };
        let touched: Vec<u32> = upd.result.moved_verts.iter().chain(upd.result.mask_changed_verts.iter()).copied().collect();
        if touched.is_empty() {
            return;
        }
        let mut dirty = touched.clone();
        for &v in &touched {
            if let Some(faces) = ctx.topo.vert_faces.get(v as usize) {
                for &f in faces {
                    if let Some(face) = obj.mesh.faces.get(f as usize) {
                        dirty.extend(face.verts());
                    }
                }
            }
        }
        dirty.sort_unstable();
        dirty.dedup();
        mirror.update(&obj.mesh, &ctx.vert_normals, &dirty);
    }

    fn apply_dyntopo(&mut self, session: &mut ModelingSession, upd: &StrokeUpdate) {
        let Some(edge) = self.run.as_ref().and_then(|r| r.dyntopo_edge) else {
            return;
        };
        if upd.result.affected_verts.is_empty() {
            return;
        }
        let Some(obj) = session.active_object_mut() else {
            return;
        };
        let region: Vec<u32> = upd.result.affected_verts.clone();
        match dyntopo_step(&mut obj.mesh, &region, edge, &DyntopoParams::default()) {
            Ok(rep) if rep.changed() => {
                let mesh = obj.mesh.clone();
                self.rebuild_engine_after_topology_change(&mesh);
                if let Some(run) = self.run.as_mut() {
                    run.topology_changed = true;
                }
            }
            Ok(_) => {}
            Err(e) => self.error = Some(format!("Dyntopo: {}", err_text(&e))),
        }
    }

    /// Encerra o traço. Sem efeito (nenhum vértice mudou) o checkpoint é descartado.
    pub fn end_stroke(&mut self, session: &mut ModelingSession) -> Option<StrokeSummary> {
        let run = self.run.take()?;
        let (Some(ctx), Some(obj)) = (self.ctx.as_mut(), session.doc.objects.get(run.obj_idx)) else {
            return None;
        };
        let outcome = self.stroke.end(&obj.mesh, ctx);
        let changed = !outcome.deltas.is_empty() || run.topology_changed;
        let summary = StrokeSummary {
            dabs: run.dabs,
            topology_changed: run.topology_changed,
            millis: run.started.elapsed().as_secs_f32() * 1000.0,
        };
        if changed {
            session.revision += 1;
            if let Some(b) = self.bound.as_mut() {
                b.revision = session.revision;
            }
            session.status = format!("{}: {} dabs", crate::i18n::tr(brush_tool_id(run.kind)), run.dabs);
        } else {
            session.discard_checkpoint();
        }
        self.last_stroke = Some(summary);
        Some(summary)
    }

    /// Cancela o traço desfazendo o que ele fez (Esc).
    pub fn cancel_stroke(&mut self, session: &mut ModelingSession) {
        let Some(run) = self.run.take() else {
            return;
        };
        if run.topology_changed {
            // a topologia mudou: volta pelo checkpoint da sessão
            self.stroke = StrokeState::new();
            session.undo();
            return;
        }
        let (Some(ctx), Some(bvh), Some(obj)) = (self.ctx.as_mut(), self.bvh.as_mut(), session.doc.objects.get_mut(run.obj_idx)) else {
            return;
        };
        let _ = self.stroke.cancel(&mut obj.mesh, bvh, ctx);
        session.discard_checkpoint();
        if let (Some(mirror), Some(ctx)) = (self.mirror.as_mut(), self.ctx.as_ref()) {
            if let Some(obj) = session.doc.objects.get(run.obj_idx) {
                mirror.update(&obj.mesh, &ctx.vert_normals, &(0..obj.mesh.verts.len() as u32).collect::<Vec<_>>());
            }
        }
    }

    /// Roda um traço roteirizado pelo mesmo caminho do mouse. Devolve o total de dabs.
    pub fn run_script(&mut self, session: &mut ModelingSession, view: &ViewInfo, script: &ScriptedStroke) -> usize {
        self.active = script.brush;
        let mods = StrokeMods {
            invert: script.invert,
            smooth: false,
        };
        let mut total = 0;
        let mut t = 0.0f64;
        for path in &script.paths {
            let pts: Vec<Vec2> = path.iter().map(|p| *p * view.size).collect();
            let Some(&first) = pts.first() else {
                continue;
            };
            match self.begin_stroke(session, view, view.ray(first), 1.0, t, mods) {
                Ok(true) => {}
                _ => continue,
            }
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let steps = (((b - a).length() / 4.0).ceil() as usize).max(1);
                for s in 1..=steps {
                    t += 0.004;
                    let p = a.lerp(b, s as f32 / steps as f32);
                    self.continue_stroke(session, view.ray(p), 1.0, t);
                }
            }
            if let Some(s) = self.end_stroke(session) {
                total += s.dabs;
            }
        }
        total
    }

    // ------------------------------------------------------------ ferramentas de malha inteira

    fn finish_tool(&mut self, session: &mut ModelingSession, msg: String) {
        session.status = msg;
        self.error = None;
    }

    /// Inverte a máscara de todos os vértices.
    pub fn mask_invert(&mut self, session: &mut ModelingSession) -> Result<(), String> {
        self.mask_op(session, "Inverter máscara", mask_invert_all)
    }

    /// Limpa a máscara.
    pub fn mask_clear(&mut self, session: &mut ModelingSession) -> Result<(), String> {
        self.mask_op(session, "Limpar máscara", mask_clear_all)
    }

    fn mask_op(
        &mut self,
        session: &mut ModelingSession,
        label: &str,
        f: fn(&mut EditMesh) -> anigo_modeling::sculpt::brush::MaskDelta,
    ) -> Result<(), String> {
        self.sync(session);
        if session.active_object().is_none() {
            return Err("Selecione um objeto".into());
        }
        session.checkpoint(label);
        let Some(obj) = session.active_object_mut() else {
            return Err("Selecione um objeto".into());
        };
        let delta = f(&mut obj.mesh);
        if delta.is_empty() {
            session.discard_checkpoint();
            self.finish_tool(session, "Nada a alterar na máscara".into());
            return Ok(());
        }
        let dirty: Vec<u32> = delta.entries.iter().map(|e| e.0).collect();
        self.commit_local_edit(session, &dirty);
        self.finish_tool(session, format!("{label}: {} vértices", dirty.len()));
        Ok(())
    }

    /// Suaviza a malha toda com o motor (pincel Smooth gigante, respeita a máscara).
    pub fn smooth_all(&mut self, session: &mut ModelingSession, iterations: u32, strength: f32) -> Result<(), String> {
        self.sync(session);
        let Some(idx) = session.doc.active else {
            return Err("Selecione um objeto".into());
        };
        if self.ctx.is_none() || self.bvh.is_none() {
            return Err(self.error.clone().unwrap_or_else(|| "Escultura indisponível para este objeto".into()));
        }
        let (Some(ctx), Some(bvh)) = (self.ctx.as_mut(), self.bvh.as_mut()) else {
            return Err("Escultura indisponível para este objeto".into());
        };
        let Some((lo, hi)) = session.doc.objects[idx].mesh.bounds() else {
            return Err("Malha vazia".into());
        };
        let mut brush = Brush::new(BrushKind::Smooth);
        brush.falloff = anigo_modeling::sculpt::falloff::Falloff::Constant;
        brush.radius = ((hi - lo).length() * 2.0).max(1e-3);
        brush.automask = AutoMask::default();
        let dab = Dab::new((lo + hi) * 0.5, Vec3::Y, brush.radius, strength.clamp(0.01, 1.0));
        session.checkpoint("Suavizar tudo");
        let Some(obj) = session.doc.objects.get_mut(idx) else {
            session.discard_checkpoint();
            return Err("Objeto ativo inválido".into());
        };
        if let Err(e) = ctx.begin_stroke(&obj.mesh, &brush, None, Vec3::Y, None) {
            session.discard_checkpoint();
            return Err(format!("Suavizar tudo falhou: {}", err_text(&e)));
        }
        let mut res = Ok(());
        for _ in 0..iterations.clamp(1, 32) {
            if let Err(e) = apply_dab(&mut obj.mesh, bvh, &dab, &brush, ctx) {
                res = Err(format!("Suavizar tudo falhou: {}", err_text(&e)));
                break;
            }
        }
        let deltas = ctx.end_stroke(&obj.mesh);
        if let Err(msg) = res {
            // restaura o estado anterior pelo histórico (o checkpoint acabou de ser gravado)
            session.undo();
            self.error = Some(msg.clone());
            return Err(msg);
        }
        if deltas.is_empty() {
            session.discard_checkpoint();
            self.finish_tool(session, "Suavizar tudo: nada mudou".into());
            return Ok(());
        }
        let dirty: Vec<u32> = (0..session.doc.objects[idx].mesh.verts.len() as u32).collect();
        self.commit_local_edit(session, &dirty);
        self.finish_tool(session, format!("Suavizar tudo: {iterations} iterações"));
        Ok(())
    }

    /// Voxel Remesh da malha ativa (com checkpoint; erro amigável). Devolve o resumo.
    pub fn voxel_remesh(&mut self, session: &mut ModelingSession) -> Result<String, String> {
        let Some(obj) = session.active_object() else {
            return Err("Selecione um objeto".into());
        };
        let size = if self.voxel_size > 0.0 { self.voxel_size } else { suggest_voxel_size(&obj.mesh) };
        let params = VoxelParams {
            smooth_iterations: self.voxel_smooth,
            ..VoxelParams::default()
        };
        let mut report_text = String::new();
        let res = session.edit_op("Voxel Remesh", |mesh, _sel| {
            let (mut new, rep) = voxel_remesh_report(mesh, size, &params)?;
            new.name = mesh.name.clone();
            report_text = format!(
                "Voxel Remesh: {} faces, voxel {:.4}{}",
                rep.quads,
                size,
                if rep.non_manifold_edges > 0 {
                    format!(" ({} arestas não-manifold)", rep.non_manifold_edges)
                } else {
                    String::new()
                }
            );
            *mesh = new;
            Ok(OpOutput::default())
        });
        match res {
            Ok(_) => {
                self.finish_tool(session, report_text.clone());
                Ok(report_text)
            }
            Err(e) => {
                let msg = voxel_error_message(&e);
                self.error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// Simetriza a malha (topologicamente simétrica) em `axis` (0 X, 1 Y, 2 Z).
    pub fn symmetrize(&mut self, session: &mut ModelingSession, axis: usize, positive_to_negative: bool) -> Result<usize, String> {
        let dir = if positive_to_negative {
            SymmetrizeDirection::PositiveToNegative
        } else {
            SymmetrizeDirection::NegativeToPositive
        };
        let mut changed = 0usize;
        let res = session.edit_op("Simetrizar", |mesh, _| {
            changed = symmetrize_mesh(mesh, axis.min(2), dir, 1e-3)?;
            Ok(OpOutput::default())
        });
        match res {
            Ok(_) => {
                self.finish_tool(session, format!("Simetrizar: {changed} vértices ajustados"));
                Ok(changed)
            }
            Err(e) => {
                let msg = match &e {
                    ModelError::UnsupportedTopology(_) => {
                        format!("Não foi possível simetrizar: {} (a malha precisa ser topologicamente simétrica e centrada na origem do objeto)", err_text(&e))
                    }
                    _ => format!("Não foi possível simetrizar: {}", err_text(&e)),
                };
                self.error = Some(msg.clone());
                Err(msg)
            }
        }
    }
}

/// Cor base do sólido em escultura (argila clara).
pub const BASE_COLOR: [f32; 4] = [0.74, 0.70, 0.66, 1.0];

fn voxel_error_message(e: &ModelError) -> String {
    match e {
        ModelError::UnsupportedTopology(m) => format!("Voxel Remesh precisa de uma malha fechada: {m}. Feche os buracos no modo Edição."),
        ModelError::InvalidParameter(m) if m.contains("células") => {
            format!("Voxel muito pequeno para esta malha: {m}")
        }
        other => format!("Voxel Remesh falhou: {}", err_text(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{Mode, PrimitiveKind};
    use anigo_core::math::Transform;
    use anigo_modeling::primitives;
    use glam::Quat;

    fn session_with_sphere(div: u32) -> ModelingSession {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
        if let Some(o) = s.active_object_mut() {
            o.mesh = primitives::quad_sphere(1.0, div);
        }
        s.clear_history();
        s.revision += 1;
        s.set_mode(Mode::Sculpt);
        s
    }

    fn front_view() -> ViewInfo {
        let camera = OrbitCamera {
            target: Vec3::new(0.0, 1.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            distance: 5.0,
            ..Default::default()
        };
        ViewInfo {
            camera,
            size: Vec2::new(800.0, 600.0),
        }
    }

    #[test]
    fn tool_to_brush_covers_all_twenty_brushes_and_roundtrips() {
        let shelf = crate::app::inspector::MODELAGEM_SCULPT;
        assert_eq!(shelf.len(), 20);
        let mut seen = std::collections::HashSet::new();
        for (i, t) in shelf.iter().enumerate() {
            let k = tool_id_to_brush(t.id).unwrap_or_else(|| panic!("ferramenta sem pincel: {}", t.id));
            assert!(seen.insert(k), "pincel repetido {k:?}");
            assert_eq!(brush_for_tool_index(i), Some(k));
            assert_eq!(tool_index_for_brush(k), Some(i));
        }
        assert_eq!(seen.len(), 20);
        assert_eq!(tool_id_to_brush("sculpt.mask"), Some(BrushKind::MaskPaint));
        assert_eq!(tool_id_to_brush("sculpt.draw"), Some(BrushKind::Draw));
        assert_eq!(tool_id_to_brush("move"), None);
        assert_eq!(brush_for_tool_index(99), None);
        // todo pincel do motor tem ferramenta
        for k in SCULPT_BRUSHES {
            assert!(tool_index_for_brush(k).is_some());
        }
    }

    #[test]
    fn ray_to_local_handles_translated_rotated_scaled_objects() {
        let t = Transform {
            translation: Vec3::new(3.0, 1.0, -2.0),
            rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(0.3),
            scale: Vec3::new(2.0, 2.0, 2.0),
        };
        let model = t.to_matrix();
        // ponto da superfície em local → mundo; um raio de mundo que o atinge vira um raio local que o atinge
        let p_local = Vec3::new(0.2, 0.5, 0.7);
        let p_world = model.transform_point3(p_local);
        let origin = p_world + Vec3::new(1.0, 2.0, 5.0);
        let dir = (p_world - origin).normalize();
        let (lo, ld) = ray_to_local(model, origin, dir);
        assert!((ld.length() - 1.0).abs() < 1e-5);
        // o ponto local está sobre o raio local
        let to_p = p_local - lo;
        let along = ld * to_p.dot(ld);
        assert!((to_p - along).length() < 1e-4, "raio local não passa pelo ponto: {}", (to_p - along).length());
        // distância ao longo do raio em mundo = escala × distância local
        assert!(((p_world - origin).length() - to_p.length() * 2.0).abs() < 1e-3);
    }

    #[test]
    fn ray_to_local_hits_the_same_vertex_on_a_transformed_sphere() {
        let mut s = session_with_sphere(8);
        let o = s.active_object_mut().unwrap();
        o.transform = Transform {
            translation: Vec3::new(1.5, 0.5, -1.0),
            rotation: Quat::from_rotation_z(0.6) * Quat::from_rotation_y(-0.4),
            scale: Vec3::splat(1.7),
        };
        let model = o.model_matrix();
        let mesh = o.mesh.clone();
        let bvh = TriBvh::build(&mesh);
        let target_local = mesh.verts[40].co;
        let target_world = model.transform_point3(target_local);
        let origin = target_world + (target_world - model.transform_point3(Vec3::ZERO)).normalize() * 4.0;
        let dir = (target_world - origin).normalize();
        let (lo, ld) = ray_to_local(model, origin, dir);
        let hit = bvh.raycast(lo, ld, f32::MAX).expect("acerta");
        assert!(hit.point.distance(target_local) < 0.2, "ponto local atingido deve ficar perto do vértice alvo");
        let back = model.transform_point3(hit.point);
        assert!(back.distance(target_world) < 0.3);
    }

    #[test]
    fn local_radius_accounts_for_depth_and_object_scale() {
        let view = front_view();
        let p = Vec3::new(0.0, 1.0, 0.0);
        let r1 = px_to_local_radius(&view.camera, view.size.y, p, 50.0, Mat4::IDENTITY);
        let r2 = px_to_local_radius(&view.camera, view.size.y, p, 100.0, Mat4::IDENTITY);
        assert!((r2 / r1 - 2.0).abs() < 1e-4);
        let scaled = Mat4::from_scale(Vec3::splat(2.0));
        let r3 = px_to_local_radius(&view.camera, view.size.y, p, 50.0, scaled);
        assert!((r1 / r3 - 2.0).abs() < 1e-4);
        // mais perto da câmera, o mesmo raio em px vale menos mundo
        let near = Vec3::new(0.0, 1.0, 3.0);
        assert!(px_to_local_radius(&view.camera, view.size.y, near, 50.0, Mat4::IDENTITY) < r1);
    }

    #[test]
    fn ring_points_lie_on_a_circle_in_the_tangent_plane() {
        let model = Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::from_rotation_x(0.5), Vec3::new(1.0, 2.0, 3.0));
        let c = Vec3::new(0.3, 0.2, 0.1);
        let n = Vec3::new(0.0, 1.0, 1.0).normalize();
        let pts = ring_points_world(model, c, n, 0.5, 32);
        assert_eq!(pts.len(), 33);
        let cw = model.transform_point3(c);
        let nw = normal_to_world(model, n);
        for p in &pts {
            assert!(((*p - cw).length() - 0.5).abs() < 1e-4);
            assert!((*p - cw).dot(nw).abs() < 1e-4);
        }
        assert!(pts[0].distance(pts[32]) < 1e-4);
    }

    #[test]
    fn resize_modal_values() {
        let r = Resize::radius(Vec2::new(300.0, 200.0), 60.0);
        // no início, a borda está sob o mouse: raio inalterado
        assert!((r.value_at(Vec2::new(300.0, 200.0)) - 60.0).abs() < 1e-4);
        assert!((r.value_at(Vec2::new(340.0, 200.0)) - 100.0).abs() < 1e-4);
        assert_eq!(r.value_at(r.center), RADIUS_MIN_PX);
        assert_eq!(r.value_at(Vec2::new(5000.0, 200.0)), RADIUS_MAX_PX);
        let s = Resize::strength(Vec2::new(100.0, 100.0), 0.5);
        assert!((s.value_at(Vec2::new(100.0, 100.0)) - 0.5).abs() < 1e-6);
        assert!((s.value_at(Vec2::new(140.0, 0.0)) - 0.6).abs() < 1e-4);
        assert_eq!(s.value_at(Vec2::new(-10000.0, 0.0)), 0.0);
        assert_eq!(s.value_at(Vec2::new(10000.0, 0.0)), STRENGTH_MAX);
    }

    #[test]
    fn stroke_deforms_mesh_and_undo_restores_exactly() {
        let mut s = session_with_sphere(16);
        let before = s.active_object().unwrap().mesh.clone();
        let view = front_view();
        let mut st = SculptState::new();
        st.slot_mut(BrushKind::Draw).brush.strength = 0.2;
        let script = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.4, 0.5), Vec2::new(0.6, 0.5)]],
            hover: None,
        };
        let dabs = st.run_script(&mut s, &view, &script);
        assert!(dabs >= 2, "dabs = {dabs}");
        let after = s.active_object().unwrap().mesh.clone();
        assert_ne!(after, before, "o traço deve deformar a malha");
        assert!(s.can_undo());
        assert_eq!(s.history_labels().last().copied(), Some("Escultura: Desenhar"));
        s.undo();
        assert_eq!(s.active_object().unwrap().mesh, before, "undo restaura a malha exatamente");
        s.redo();
        assert_eq!(s.active_object().unwrap().mesh, after, "redo reaplica o traço");
        // depois do undo/redo, o estado de escultura ressincroniza sozinho e continua funcionando
        st.sync(&s);
        assert!(st.is_bound());
        let n = st.run_script(&mut s, &view, &script);
        assert!(n >= 2);
    }

    #[test]
    fn invert_draw_digs_in_and_one_stroke_is_one_undo_entry() {
        let mut s = session_with_sphere(16);
        let view = front_view();
        let mut st = SculptState::new();
        let center_before = s.active_object().unwrap().mesh.verts.iter().map(|v| v.co.length()).fold(f32::MIN, f32::max);
        st.slot_mut(BrushKind::Draw).brush.strength = 0.2;
        let up = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.5, 0.5), Vec2::new(0.52, 0.5)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &up);
        let max_out = s.active_object().unwrap().mesh.verts.iter().map(|v| v.co.length()).fold(f32::MIN, f32::max);
        assert!(max_out > center_before + 1e-3);
        let down = ScriptedStroke { invert: true, ..up.clone() };
        let n_before = s.history_labels().len();
        st.run_script(&mut s, &view, &down);
        assert_eq!(s.history_labels().len(), n_before + 1, "um traço = um ponto de undo");
    }

    #[test]
    fn stroke_missing_the_mesh_leaves_no_undo_entry() {
        let mut s = session_with_sphere(8);
        let view = front_view();
        let mut st = SculptState::new();
        let script = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.02, 0.02), Vec2::new(0.05, 0.05)]],
            hover: None,
        };
        let before = s.history_labels().len();
        assert_eq!(st.run_script(&mut s, &view, &script), 0);
        assert_eq!(s.history_labels().len(), before);
    }

    #[test]
    fn mask_paint_then_clear_and_invert_with_undo() {
        let mut s = session_with_sphere(12);
        let view = front_view();
        let mut st = SculptState::new();
        st.slot_mut(BrushKind::MaskPaint).brush.strength = 1.0;
        let script = ScriptedStroke {
            brush: BrushKind::MaskPaint,
            invert: false,
            paths: vec![vec![Vec2::new(0.45, 0.5), Vec2::new(0.55, 0.5)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &script);
        let masked = s.active_object().unwrap().mesh.verts.iter().filter(|v| v.mask > 0.0).count();
        assert!(masked > 0, "o pincel de máscara deve mascarar vértices");
        let mirror_dark = st
            .mirror
            .as_ref()
            .unwrap()
            .verts
            .iter()
            .filter(|v| v.color[0] < BASE_COLOR[0] - 0.05)
            .count();
        assert!(mirror_dark > 0, "o espelho da GPU deve escurecer os vértices mascarados");

        st.mask_invert(&mut s).unwrap();
        let inv = s.active_object().unwrap().mesh.verts.iter().filter(|v| v.mask > 0.5).count();
        assert!(inv > s.active_object().unwrap().mesh.verts.len() / 2);
        st.mask_clear(&mut s).unwrap();
        assert!(s.active_object().unwrap().mesh.verts.iter().all(|v| v.mask == 0.0));
        s.undo();
        assert!(s.active_object().unwrap().mesh.verts.iter().filter(|v| v.mask > 0.5).count() == inv);
    }

    #[test]
    fn masked_vertices_are_protected_from_draw() {
        let mut s = session_with_sphere(12);
        for v in &mut s.active_object_mut().unwrap().mesh.verts {
            v.mask = 1.0;
        }
        s.revision += 1;
        let before = s.active_object().unwrap().mesh.clone();
        let view = front_view();
        let mut st = SculptState::new();
        let script = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.4, 0.5), Vec2::new(0.6, 0.5)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &script);
        assert_eq!(s.active_object().unwrap().mesh, before);
        assert!(!s.can_undo() || s.history_labels().iter().all(|l| !l.starts_with("Escultura")), "traço sem efeito não deve gravar undo");
    }

    #[test]
    fn symmetry_x_mirrors_the_stroke() {
        let mut s = session_with_sphere(16);
        let view = front_view();
        let mut st = SculptState::new();
        st.symmetry = Symmetry::mirror_x();
        st.slot_mut(BrushKind::Draw).brush.strength = 0.2;
        let script = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.6, 0.5), Vec2::new(0.64, 0.5)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &script);
        let mesh = &s.active_object().unwrap().mesh;
        let orig = primitives::quad_sphere(1.0, 16);
        let moved_pos = mesh.verts.iter().zip(&orig.verts).filter(|(a, b)| a.co != b.co && a.co.x > 0.0).count();
        let moved_neg = mesh.verts.iter().zip(&orig.verts).filter(|(a, b)| a.co != b.co && a.co.x < 0.0).count();
        assert!(moved_pos > 0 && moved_neg > 0, "pos {moved_pos}, neg {moved_neg}");
    }

    #[test]
    fn smooth_all_changes_a_bumpy_mesh_and_undoes() {
        let mut s = session_with_sphere(10);
        {
            let o = s.active_object_mut().unwrap();
            for (i, v) in o.mesh.verts.iter_mut().enumerate() {
                if i % 7 == 0 {
                    v.co *= 1.15;
                }
            }
        }
        s.revision += 1;
        let before = s.active_object().unwrap().mesh.clone();
        let mut st = SculptState::new();
        st.smooth_all(&mut s, 3, 0.5).unwrap();
        assert_ne!(s.active_object().unwrap().mesh, before);
        s.undo();
        assert_eq!(s.active_object().unwrap().mesh, before);
    }

    #[test]
    fn voxel_remesh_replaces_mesh_with_undo_and_friendly_error_on_open_mesh() {
        let mut s = session_with_sphere(8);
        let before = s.active_object().unwrap().mesh.clone();
        let mut st = SculptState::new();
        st.voxel_size = 0.15;
        let msg = st.voxel_remesh(&mut s).unwrap();
        assert!(msg.contains("Voxel Remesh"));
        assert_ne!(s.active_object().unwrap().mesh.faces.len(), before.faces.len());
        s.undo();
        assert_eq!(s.active_object().unwrap().mesh, before);

        // malha aberta -> mensagem amigável, nada muda
        let mut s2 = ModelingSession::new();
        s2.add_primitive(PrimitiveKind::Plane, Vec3::ZERO);
        let m0 = s2.active_object().unwrap().mesh.clone();
        let err = st.voxel_remesh(&mut s2).unwrap_err();
        assert!(err.contains("fechada"), "{err}");
        assert_eq!(s2.active_object().unwrap().mesh, m0);
    }

    #[test]
    fn dyntopo_stroke_adds_geometry_and_stays_valid() {
        let mut s = session_with_sphere(6);
        let view = front_view();
        let mut st = SculptState::new();
        st.dyntopo = true;
        st.dyntopo_detail_px = 8.0;
        st.slot_mut(BrushKind::Draw).brush.strength = 0.1;
        st.slot_mut(BrushKind::Draw).radius_px = 40.0;
        let faces_before = s.active_object().unwrap().mesh.faces.len();
        let script = ScriptedStroke {
            brush: BrushKind::Draw,
            invert: false,
            paths: vec![vec![Vec2::new(0.48, 0.5), Vec2::new(0.52, 0.5)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &script);
        let mesh = &s.active_object().unwrap().mesh;
        assert!(mesh.faces.len() > faces_before, "dyntopo deve subdividir sob o pincel");
        mesh.validate().unwrap();
        s.undo();
        assert_eq!(s.active_object().unwrap().mesh.faces.len(), faces_before);
    }

    #[test]
    fn mirror_matches_mesh_after_strokes() {
        let mut s = session_with_sphere(12);
        let view = front_view();
        let mut st = SculptState::new();
        st.slot_mut(BrushKind::Inflate).brush.strength = 0.2;
        let script = ScriptedStroke {
            brush: BrushKind::Inflate,
            invert: false,
            paths: vec![vec![Vec2::new(0.45, 0.45), Vec2::new(0.55, 0.55)]],
            hover: None,
        };
        st.run_script(&mut s, &view, &script);
        let mesh = &s.active_object().unwrap().mesh;
        let mir = st.mirror.as_ref().unwrap();
        for i in 0..mir.len() {
            let v = mir.mesh_vert(i).unwrap();
            assert_eq!(mir.verts[i].pos, mesh.verts[v as usize].co.to_array());
        }
    }

    #[test]
    fn switching_object_rebinds_the_engine() {
        let mut s = session_with_sphere(6);
        let mut st = SculptState::new();
        st.sync(&s);
        let gen1 = st.gpu_gen;
        assert!(st.is_bound());
        s.set_mode(Mode::Object);
        s.add_primitive(PrimitiveKind::Cube, Vec3::new(4.0, 1.0, 0.0));
        s.set_mode(Mode::Sculpt);
        st.sync(&s);
        assert!(st.gpu_gen > gen1);
        assert_eq!(st.ctx().unwrap().vert_normals.len(), 8);
    }

    /// Desempenho em quad_sphere(64) (24 576 faces): custo por quadro do traço (dabs + espelho incremental)
    /// versus reconstruir o sólido inteiro como o caminho antigo fazia.
    #[test]
    fn perf_incremental_frame_vs_full_rebuild_on_24k_faces() {
        let mut s = session_with_sphere(64);
        let view = front_view();
        let mut st = SculptState::new();
        st.slot_mut(BrushKind::Draw).radius_px = 60.0;
        st.sync(&s);
        assert!(st.is_bound());
        let t0 = std::time::Instant::now();
        assert!(st.begin_stroke(&mut s, &view, view.ray(Vec2::new(300.0, 300.0)), 1.0, 0.0, StrokeMods::default()).unwrap());
        let mut frames = 0;
        for k in 1..=30 {
            let p = Vec2::new(300.0 + k as f32 * 6.0, 300.0 + (k as f32 * 0.3).sin() * 20.0);
            st.continue_stroke(&mut s, view.ray(p), 1.0, k as f64 * 0.016);
            frames += 1;
        }
        st.end_stroke(&mut s);
        let per_frame = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let mesh = &s.active_object().unwrap().mesh;
        let t1 = std::time::Instant::now();
        let data = crate::workbench::build_draw_data(mesh, &crate::workbench::BuildOptions::default());
        let full = t1.elapsed().as_secs_f64() * 1000.0;
        eprintln!("perf 24k faces (debug): traço {per_frame:.2} ms/quadro; reconstrução completa antiga {full:.1} ms ({} tris)", data.solid.len() / 3);
        assert!(per_frame < full, "o caminho incremental deve ser mais barato que reconstruir tudo");
    }
}
