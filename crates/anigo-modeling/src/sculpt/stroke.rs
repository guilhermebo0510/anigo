//! Traços de escultura: converte amostras do mouse/caneta em dabs espaçados.
//!
//! Fluxo: [`StrokeState::begin`] → N × [`StrokeState::continue_stroke`] →
//! [`StrokeState::end`] (devolve os deltas de undo) ou [`StrokeState::cancel`]
//! (desfaz tudo).
//!
//! * **Raycast**: cada amostra (raio da câmera) é testada no [`TriBvh`]; pincéis
//!   de superfície seguem os pontos atingidos, pincéis "livres" (Grab, Elastic,
//!   Snake Hook) projetam o raio num plano paralelo à tela que passa pelo
//!   primeiro ponto atingido.
//! * **Estabilização** (lazy radius): a posição só avança quando o mouse sai de
//!   um círculo de raio `stabilize_radius × raio` ao redor dela (corda).
//! * **Suavização**: média exponencial da posição (`stroke_smooth`).
//! * **Espaçamento**: dabs a cada `spacing × raio` ao longo da polilinha do
//!   traço, re-ajustados à superfície (`closest_point`); independe da taxa de
//!   amostragem do mouse. O primeiro dab ocorre no início do traço.
//! * **Pressão**: escala força e/ou raio conforme o pincel.
//! * **Undo**: o [`StrokeOutcome`] traz deltas esparsos (só vértices tocados).

use crate::bvh::TriBvh;
use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::sculpt::brush::{apply_dab_symmetric, Brush, BrushKind, Dab, DabResult, SculptContext, StrokeDeltas};
use crate::sculpt::symmetry::Symmetry;
use glam::Vec3;

/// Amostra de entrada: raio da câmera pelo cursor, pressão e tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeSample {
    pub ray_origin: Vec3,
    pub ray_dir: Vec3,
    /// Pressão da caneta em `[0, 1]` (use 1.0 para mouse).
    pub pressure: f32,
    /// Tempo em segundos (monotônico; usado pelo airbrush).
    pub time: f64,
}

impl StrokeSample {
    /// Amostra de mouse (pressão 1).
    pub fn new(ray_origin: Vec3, ray_dir: Vec3, time: f64) -> Self {
        Self {
            ray_origin,
            ray_dir,
            pressure: 1.0,
            time,
        }
    }

    fn validate(&self) -> ModelResult<()> {
        if !self.ray_origin.is_finite() || !self.ray_dir.is_finite() || self.ray_dir.length_squared() < 1e-20 {
            return Err(ModelError::InvalidParameter("amostra com raio inválido".into()));
        }
        if !self.pressure.is_finite() {
            return Err(ModelError::InvalidParameter("pressão não finita".into()));
        }
        if !self.time.is_finite() {
            return Err(ModelError::InvalidParameter("tempo não finito".into()));
        }
        Ok(())
    }
}

/// O que uma chamada a `begin`/`continue_stroke` fez.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrokeUpdate {
    /// O traço começou (só relevante em `begin`; falso = o raio não atingiu a malha).
    pub started: bool,
    /// Dabs emitidos (sem contar as cópias de simetria).
    pub dabs: usize,
    /// Centros dos dabs emitidos (passe original), úteis para prévia/testes.
    pub centers: Vec<Vec3>,
    /// União dos resultados (vértices afetados/movidos/máscara).
    pub result: DabResult,
}

/// Resultado final do traço.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrokeOutcome {
    /// Deltas esparsos para undo/redo (só vértices tocados).
    pub deltas: StrokeDeltas,
    /// Total de dabs emitidos no traço.
    pub dab_count: usize,
    /// Comprimento da polilinha do traço (unidades do mundo).
    pub path_length: f32,
}

/// Estado de um traço em andamento. Um `StrokeState` serve vários traços em
/// sequência (`begin` reinicia).
#[derive(Debug, Clone)]
pub struct StrokeState {
    brush: Brush,
    symmetry: Symmetry,
    active: bool,
    start_loc: Vec3,
    last_loc: Vec3,
    last_dab_center: Vec3,
    last_normal: Vec3,
    lazy: Vec3,
    smooth: Vec3,
    carry: f32,
    plane_normal: Vec3,
    view_dir: Vec3,
    last_pressure: f32,
    last_time: f64,
    air_acc: f64,
    dab_count: usize,
    path_length: f32,
}

impl Default for StrokeState {
    fn default() -> Self {
        Self::new()
    }
}

const EPS_REL: f32 = 1e-4;

impl StrokeState {
    pub fn new() -> Self {
        Self {
            brush: Brush::new(BrushKind::Draw),
            symmetry: Symmetry::none(),
            active: false,
            start_loc: Vec3::ZERO,
            last_loc: Vec3::ZERO,
            last_dab_center: Vec3::ZERO,
            last_normal: Vec3::Y,
            lazy: Vec3::ZERO,
            smooth: Vec3::ZERO,
            carry: 0.0,
            plane_normal: Vec3::Z,
            view_dir: Vec3::ZERO,
            last_pressure: 1.0,
            last_time: 0.0,
            air_acc: 0.0,
            dab_count: 0,
            path_length: 0.0,
        }
    }

    /// Há um traço em andamento.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Dabs emitidos no traço atual.
    pub fn dab_count(&self) -> usize {
        self.dab_count
    }

    /// Pincel do traço em andamento.
    pub fn brush(&self) -> &Brush {
        &self.brush
    }

    /// Inicia um traço. Valida tudo antes de mutar. Se o raio não atinge a
    /// malha devolve `Ok` com `started = false` (nada muda e o estado fica
    /// inativo; os próximos `continue_stroke` são no-ops).
    ///
    /// Erros: pincel/simetria/amostra inválidos, contexto ou BVH desatualizados,
    /// traço já ativo (neste estado ou no `ctx`).
    pub fn begin(
        &mut self,
        mesh: &mut EditMesh,
        bvh: &mut TriBvh,
        ctx: &mut SculptContext,
        brush: &Brush,
        symmetry: &Symmetry,
        sample: &StrokeSample,
    ) -> ModelResult<StrokeUpdate> {
        if self.active {
            return Err(ModelError::InvalidParameter(
                "já existe um traço ativo; chame end ou cancel".into(),
            ));
        }
        brush.validate()?;
        symmetry.validate()?;
        sample.validate()?;
        ctx.check_sync(mesh)?;
        if bvh.vert_count() != mesh.verts.len() || bvh.face_count() != mesh.faces.len() {
            return Err(ModelError::InvalidParameter(
                "BVH desatualizado em relação à malha; reconstrua".into(),
            ));
        }
        if ctx.stroke.active {
            return Err(ModelError::InvalidParameter(
                "o contexto de escultura já tem um traço ativo".into(),
            ));
        }
        let dir = sample.ray_dir.normalize();
        let Some(hit) = bvh.raycast(sample.ray_origin, dir, f32::MAX) else {
            return Ok(StrokeUpdate::default());
        };
        let normal = surface_normal(bvh, ctx, hit.tri_index, hit.bary).unwrap_or(hit.normal);
        let start_vert = mesh.faces[hit.face as usize].verts().min_by(|&a, &b| {
            mesh.co(a)
                .distance_squared(hit.point)
                .total_cmp(&mesh.co(b).distance_squared(hit.point))
        });
        ctx.begin_stroke(mesh, brush, start_vert, normal, Some(hit.face))?;

        self.brush = brush.clone();
        self.symmetry = symmetry.clone();
        self.active = true;
        self.start_loc = hit.point;
        self.last_loc = hit.point;
        self.last_dab_center = hit.point;
        self.last_normal = normal;
        self.lazy = hit.point;
        self.smooth = hit.point;
        self.carry = 0.0;
        self.plane_normal = -dir;
        self.view_dir = dir;
        self.last_pressure = sample.pressure.clamp(0.0, 1.0);
        self.last_time = sample.time;
        self.air_acc = 0.0;
        self.dab_count = 0;
        self.path_length = 0.0;

        let mut upd = StrokeUpdate {
            started: true,
            ..Default::default()
        };
        let p = self.last_pressure;
        self.emit(mesh, bvh, ctx, hit.point, p, true, &mut upd)?;
        Ok(upd)
    }

    /// Processa uma nova amostra, emitindo os dabs necessários. No-op sem traço ativo.
    pub fn continue_stroke(
        &mut self,
        mesh: &mut EditMesh,
        bvh: &mut TriBvh,
        ctx: &mut SculptContext,
        sample: &StrokeSample,
    ) -> ModelResult<StrokeUpdate> {
        let mut upd = StrokeUpdate {
            started: self.active,
            ..Default::default()
        };
        if !self.active {
            return Ok(upd);
        }
        sample.validate()?;
        ctx.check_sync(mesh)?;
        if bvh.vert_count() != mesh.verts.len() || bvh.face_count() != mesh.faces.len() {
            return Err(ModelError::InvalidParameter(
                "BVH desatualizado em relação à malha; reconstrua".into(),
            ));
        }
        let dir = sample.ray_dir.normalize();
        self.view_dir = dir;
        let pressure = sample.pressure.clamp(0.0, 1.0);
        let kind = self.brush.kind;

        // posição bruta da amostra
        let raw = if kind.is_grab_like() {
            let denom = dir.dot(self.plane_normal);
            if denom.abs() < 1e-5 {
                None
            } else {
                let t = (self.start_loc - sample.ray_origin).dot(self.plane_normal) / denom;
                Some(sample.ray_origin + dir * t)
            }
        } else {
            bvh.raycast(sample.ray_origin, dir, f32::MAX).map(|h| h.point)
        };

        let mut emitted_by_distance = false;
        if let Some(raw) = raw {
            let target = self.stabilize_and_smooth(raw);
            let seg = target - self.last_loc;
            let len = seg.length();
            let p0 = self.last_pressure;
            if kind.uses_frozen_set() {
                // Grab/Elastic: um dab por amostra que se moveu
                if len > 1e-9 {
                    self.path_length += len;
                    self.last_loc = target;
                    self.emit(mesh, bvh, ctx, target, pressure, false, &mut upd)?;
                    emitted_by_distance = true;
                }
            } else if len > 1e-9 {
                self.path_length += len;
                let dirn = seg / len;
                let mut cursor = self.last_loc;
                let mut remaining = len;
                let radius_now = self.effective_radius(pressure);
                let step = (self.brush.spacing * radius_now).max(1e-6);
                let mut need = step - self.carry;
                while remaining + step * EPS_REL >= need {
                    let adv = need.min(remaining).max(0.0);
                    cursor += dirn * adv;
                    remaining -= adv;
                    let frac = if len > 0.0 { 1.0 - remaining / len } else { 1.0 };
                    let p = p0 + (pressure - p0) * frac.clamp(0.0, 1.0);
                    self.emit(mesh, bvh, ctx, cursor, p, false, &mut upd)?;
                    emitted_by_distance = true;
                    self.carry = 0.0;
                    need = step;
                    if remaining <= step * EPS_REL * 0.5 {
                        remaining = 0.0;
                        break;
                    }
                }
                self.carry += remaining.max(0.0);
                self.last_loc = target;
            }
        }

        // airbrush: dabs pelo tempo quando o cursor não gerou dabs por distância
        if self.brush.airbrush && !kind.uses_frozen_set() {
            if emitted_by_distance {
                self.air_acc = 0.0;
            } else {
                self.air_acc += (sample.time - self.last_time).max(0.0);
                let period = 1.0 / self.brush.airbrush_rate as f64;
                let loc = self.last_loc;
                while self.air_acc + 1e-9 >= period {
                    self.air_acc -= period;
                    self.emit(mesh, bvh, ctx, loc, pressure, false, &mut upd)?;
                }
            }
        }
        self.last_pressure = pressure;
        self.last_time = sample.time;
        Ok(upd)
    }

    /// Encerra o traço e devolve os deltas de undo. Sem traço ativo devolve vazio.
    pub fn end(&mut self, mesh: &EditMesh, ctx: &mut SculptContext) -> StrokeOutcome {
        if !self.active {
            return StrokeOutcome::default();
        }
        self.active = false;
        let deltas = ctx.end_stroke(mesh);
        StrokeOutcome {
            deltas,
            dab_count: self.dab_count,
            path_length: self.path_length,
        }
    }

    /// Cancela o traço desfazendo tudo o que ele alterou (posições, máscara,
    /// normais locais e BVH).
    pub fn cancel(&mut self, mesh: &mut EditMesh, bvh: &mut TriBvh, ctx: &mut SculptContext) -> ModelResult<()> {
        if !self.active {
            return Ok(());
        }
        let outcome = self.end(mesh, ctx);
        outcome.deltas.revert(mesh);
        let moved: Vec<u32> = outcome.deltas.positions.entries.iter().map(|e| e.0).collect();
        ctx.refresh_normals(mesh, &moved);
        bvh.refit_verts(mesh, &moved)
    }

    // ---------------------------------------------------------------- interno

    fn effective_radius(&self, pressure: f32) -> f32 {
        self.brush.radius
            * if self.brush.use_pressure_size {
                pressure.clamp(0.0, 1.0).max(0.05)
            } else {
                1.0
            }
    }

    fn effective_strength(&self, pressure: f32) -> f32 {
        self.brush.strength
            * if self.brush.use_pressure_strength {
                pressure.clamp(0.0, 1.0)
            } else {
                1.0
            }
    }

    fn stabilize_and_smooth(&mut self, raw: Vec3) -> Vec3 {
        let mut target = raw;
        let lazy_r = self.brush.stabilize_radius * self.brush.radius;
        if lazy_r > 0.0 {
            let d = raw.distance(self.lazy);
            if d > lazy_r {
                self.lazy += (raw - self.lazy) * (1.0 - lazy_r / d);
            }
            target = self.lazy;
        }
        if self.brush.stroke_smooth > 0.0 {
            self.smooth += (target - self.smooth) * (1.0 - self.brush.stroke_smooth);
            target = self.smooth;
        }
        target
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        mesh: &mut EditMesh,
        bvh: &mut TriBvh,
        ctx: &mut SculptContext,
        loc: Vec3,
        pressure: f32,
        first: bool,
        upd: &mut StrokeUpdate,
    ) -> ModelResult<()> {
        let radius = self.effective_radius(pressure);
        let strength = self.effective_strength(pressure);
        if strength <= 0.0 {
            return Ok(()); // pressão zero: nada a aplicar
        }
        let kind = self.brush.kind;
        let (center, normal) = match kind {
            BrushKind::Grab | BrushKind::Elastic => (self.start_loc, self.last_normal),
            BrushKind::SnakeHook => (loc, self.last_normal),
            _ => {
                let Some(c) = bvh.closest_point(loc) else {
                    return Ok(());
                };
                let n = surface_normal(bvh, ctx, c.tri_index, c.bary).unwrap_or(c.normal);
                (c.point, n)
            }
        };
        let mut dab = Dab::new(center, normal, radius, strength);
        dab.view_dir = self.view_dir;
        dab.first = first;
        dab.delta = if first { Vec3::ZERO } else { loc_delta(kind, center, loc, self.last_dab_center, self.last_loc) };
        dab.total_delta = if kind.is_grab_like() { loc - self.start_loc } else { center - self.start_loc };
        let res = apply_dab_symmetric(mesh, bvh, &dab, &self.brush, &self.symmetry, ctx)?;
        upd.result.merge(&res);
        upd.dabs += 1;
        upd.centers.push(center);
        self.dab_count += 1;
        self.last_dab_center = if kind.is_grab_like() { loc } else { center };
        self.last_normal = normal;
        Ok(())
    }
}

/// Deslocamento do centro entre dabs consecutivos.
fn loc_delta(kind: BrushKind, center: Vec3, loc: Vec3, last_center: Vec3, last_loc: Vec3) -> Vec3 {
    match kind {
        BrushKind::Grab | BrushKind::Elastic => loc - last_loc,
        BrushKind::SnakeHook => loc - last_center,
        _ => center - last_center,
    }
}

/// Normal suave no ponto do triângulo (interpolação baricêntrica das normais
/// dos vértices do contexto).
fn surface_normal(bvh: &TriBvh, ctx: &SculptContext, tri_index: u32, bary: [f32; 3]) -> Option<Vec3> {
    let tr = bvh.tri_ref(tri_index)?;
    let mut n = Vec3::ZERO;
    for k in 0..3 {
        n += *ctx.vert_normals.get(tr.verts[k] as usize)? * bary[k];
    }
    n.try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bvh::test_util::Rng;
    use crate::primitives;

    struct Rig {
        mesh: EditMesh,
        bvh: TriBvh,
        ctx: SculptContext,
        stroke: StrokeState,
    }

    impl Rig {
        fn new(mesh: EditMesh) -> Self {
            let bvh = TriBvh::build(&mesh);
            let ctx = SculptContext::new(&mesh).unwrap();
            Rig {
                mesh,
                bvh,
                ctx,
                stroke: StrokeState::new(),
            }
        }
        fn begin(&mut self, b: &Brush, s: &Symmetry, sample: &StrokeSample) -> ModelResult<StrokeUpdate> {
            self.stroke.begin(&mut self.mesh, &mut self.bvh, &mut self.ctx, b, s, sample)
        }
        fn cont(&mut self, sample: &StrokeSample) -> ModelResult<StrokeUpdate> {
            self.stroke
                .continue_stroke(&mut self.mesh, &mut self.bvh, &mut self.ctx, sample)
        }
        fn end(&mut self) -> StrokeOutcome {
            self.stroke.end(&self.mesh, &mut self.ctx)
        }
        /// Índice do vértice originalmente mais próximo de `p` (a malha de teste é uma esfera).
        fn vert_idx_near(&self, p: Vec3) -> usize {
            let orig = primitives::quad_sphere(1.0, 16);
            (0..orig.vert_count())
                .min_by(|&a, &b| orig.verts[a].co.distance(p).total_cmp(&orig.verts[b].co.distance(p)))
                .unwrap()
        }
    }

    /// Raio vertical (câmera acima olhando para baixo) sobre o plano XZ.
    fn down(x: f32, z: f32, t: f64) -> StrokeSample {
        StrokeSample::new(Vec3::new(x, 5.0, z), -Vec3::Y, t)
    }

    fn flat() -> Rig {
        Rig::new(primitives::grid(40, 40, 2.0, 2.0))
    }

    fn draw(radius: f32, strength: f32, spacing: f32) -> Brush {
        let mut b = Brush::new(BrushKind::Draw);
        b.radius = radius;
        b.strength = strength;
        b.spacing = spacing;
        b
    }

    fn run_line(rig: &mut Rig, b: &Brush, sym: &Symmetry, x0: f32, x1: f32, samples: usize) -> (usize, Vec<Vec3>) {
        let mut total = 0;
        let mut centers = Vec::new();
        let u = rig.begin(b, sym, &down(x0, 0.0, 0.0)).unwrap();
        assert!(u.started);
        total += u.dabs;
        centers.extend(u.centers);
        for i in 1..=samples {
            let x = x0 + (x1 - x0) * i as f32 / samples as f32;
            let u = rig.cont(&down(x, 0.0, i as f64 * 0.01)).unwrap();
            total += u.dabs;
            centers.extend(u.centers);
        }
        (total, centers)
    }

    #[test]
    fn dab_count_follows_spacing_and_is_independent_of_sampling_rate() {
        // raio 0.2, espaçamento 0.25 -> passo 0.05; trajeto de 1.0 -> 1 + 20 dabs
        let b = draw(0.2, 0.001, 0.25);
        for samples in [1usize, 4, 10, 37, 100] {
            let mut rig = flat();
            let (n, centers) = run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, samples);
            assert_eq!(n, 21, "samples={samples}");
            // dabs igualmente espaçados em x (ao longo da linha)
            for w in centers.windows(2) {
                assert!((w[1].x - w[0].x - 0.05).abs() < 2e-3, "{:?}", (w[0], w[1]));
            }
            let out = rig.end();
            assert_eq!(out.dab_count, 21);
            assert!((out.path_length - 1.0).abs() < 1e-4);
        }
        // espaçamento maior -> menos dabs
        let wide = draw(0.2, 0.001, 1.0); // passo 0.2 => 1 + 5
        let mut rig = flat();
        let (n, _) = run_line(&mut rig, &wide, &Symmetry::none(), -0.5, 0.5, 10);
        assert_eq!(n, 6);
        // traço sem movimento: só o dab inicial
        let mut rig = flat();
        let (n, _) = run_line(&mut rig, &b, &Symmetry::none(), 0.0, 0.0, 5);
        assert_eq!(n, 1);
    }

    #[test]
    fn spacing_carry_survives_many_tiny_segments() {
        let b = draw(0.2, 0.001, 0.25);
        let mut rig = flat();
        let (n, _) = run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, 1000);
        assert_eq!(n, 21);
    }

    #[test]
    fn stroke_modifies_surface_and_undo_restores_exactly() {
        let mut rig = flat();
        let orig = rig.mesh.clone();
        let b = draw(0.2, 0.2, 0.1);
        run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, 20);
        let edited = rig.mesh.clone();
        assert_ne!(edited, orig);
        let out = rig.end();
        assert!(!out.deltas.is_empty());
        assert!(out.deltas.positions.entries.len() < rig.mesh.vert_count() / 2, "delta esparso");
        out.deltas.revert(&mut rig.mesh);
        assert_eq!(rig.mesh, orig);
        out.deltas.apply(&mut rig.mesh);
        assert_eq!(rig.mesh, edited);
        assert!(!rig.stroke.is_active() && !rig.ctx.stroke.active);
    }

    #[test]
    fn miss_at_begin_does_not_start_and_later_calls_are_noops() {
        let mut rig = flat();
        let orig = rig.mesh.clone();
        let b = draw(0.2, 0.2, 0.1);
        let u = rig.begin(&b, &Symmetry::none(), &down(9.0, 9.0, 0.0)).unwrap();
        assert!(!u.started && u.dabs == 0);
        assert!(!rig.stroke.is_active());
        let u = rig.cont(&down(0.0, 0.0, 0.1)).unwrap();
        assert!(!u.started && u.dabs == 0);
        assert_eq!(rig.end(), StrokeOutcome::default());
        assert_eq!(rig.mesh, orig);
    }

    #[test]
    fn miss_during_stroke_is_skipped() {
        let mut rig = flat();
        let b = draw(0.2, 0.05, 0.1);
        rig.begin(&b, &Symmetry::none(), &down(-0.9, 0.0, 0.0)).unwrap();
        let u = rig.cont(&down(5.0, 5.0, 0.1)).unwrap(); // fora da malha
        assert_eq!(u.dabs, 0);
        let u = rig.cont(&down(-0.7, 0.0, 0.2)).unwrap();
        assert!(u.dabs > 0);
    }

    #[test]
    fn invalid_inputs_are_rejected_without_side_effects() {
        let mut rig = flat();
        let orig = rig.mesh.clone();
        let good = draw(0.2, 0.2, 0.1);
        let mut bad = good.clone();
        bad.radius = -1.0;
        assert!(rig.begin(&bad, &Symmetry::none(), &down(0.0, 0.0, 0.0)).is_err());
        let sym = Symmetry {
            tolerance: f32::NAN,
            ..Default::default()
        };
        assert!(rig.begin(&good, &sym, &down(0.0, 0.0, 0.0)).is_err());
        let s = StrokeSample::new(Vec3::NAN, -Vec3::Y, 0.0);
        assert!(rig.begin(&good, &Symmetry::none(), &s).is_err());
        let s = StrokeSample::new(Vec3::Y, Vec3::ZERO, 0.0);
        assert!(rig.begin(&good, &Symmetry::none(), &s).is_err());
        let mut s = down(0.0, 0.0, 0.0);
        s.pressure = f32::NAN;
        assert!(rig.begin(&good, &Symmetry::none(), &s).is_err());
        let mut s = down(0.0, 0.0, 0.0);
        s.time = f64::INFINITY;
        assert!(rig.begin(&good, &Symmetry::none(), &s).is_err());
        assert!(!rig.stroke.is_active() && !rig.ctx.stroke.active);
        assert_eq!(rig.mesh, orig);
        // dessincronizado
        rig.mesh.add_vert(Vec3::ZERO);
        assert!(rig.begin(&good, &Symmetry::none(), &down(0.0, 0.0, 0.0)).is_err());
        // begin duplo
        let mut rig = flat();
        rig.begin(&good, &Symmetry::none(), &down(0.0, 0.0, 0.0)).unwrap();
        assert!(rig.begin(&good, &Symmetry::none(), &down(0.0, 0.0, 0.0)).is_err());
        // amostra inválida no meio do traço
        assert!(rig.cont(&StrokeSample::new(Vec3::Y, Vec3::ZERO, 0.1)).is_err());
        assert!(rig.stroke.is_active(), "erro de amostra não encerra o traço");
        rig.end();
    }

    #[test]
    fn pressure_scales_strength_and_radius() {
        let height_at = |pressure: f32, size: bool, strength: bool| {
            let mut rig = flat();
            let mut b = draw(0.3, 0.2, 0.1);
            b.use_pressure_size = size;
            b.use_pressure_strength = strength;
            let mut s = down(0.0, 0.0, 0.0);
            s.pressure = pressure;
            let u = rig.begin(&b, &Symmetry::none(), &s).unwrap();
            let max_y = rig.mesh.verts.iter().map(|v| v.co.y).fold(f32::MIN, f32::max);
            let affected = u.result.affected_verts.len();
            (max_y, affected)
        };
        let (full, n_full) = height_at(1.0, true, true);
        let (half_s, _) = height_at(0.5, false, true);
        assert!((half_s - full * 0.5).abs() < 1e-5, "{half_s} vs {full}");
        let (_, n_small) = height_at(0.4, true, false);
        assert!(n_small < n_full, "raio menor afeta menos vértices");
        let (no_p, _) = height_at(0.2, false, false);
        assert!((no_p - full).abs() < 1e-6, "sem pressão habilitada ignora pressão");
        // pressão zero com strength: nada
        let (zero, n0) = height_at(0.0, false, true);
        assert_eq!(zero, 0.0);
        assert_eq!(n0, 0);
    }

    #[test]
    fn stabilizer_makes_the_brush_lag_behind_the_cursor() {
        let mut b = draw(0.2, 0.001, 0.25);
        let mut plain = flat();
        let (n_plain, c_plain) = run_line(&mut plain, &b, &Symmetry::none(), -0.5, 0.5, 20);
        b.stabilize_radius = 1.5; // corda de 0.3
        let mut lazy = flat();
        let (n_lazy, c_lazy) = run_line(&mut lazy, &b, &Symmetry::none(), -0.5, 0.5, 20);
        assert!(n_lazy < n_plain);
        let end_plain = c_plain.last().unwrap().x;
        let end_lazy = c_lazy.last().unwrap().x;
        assert!(end_lazy < end_plain - 0.2, "{end_lazy} !< {end_plain}");
        // o dab nunca fica mais longe do mouse do que a corda
        assert!(0.5 - end_lazy <= 0.3 + 0.06);
        // mouse parado depois do traço: nada de novos dabs
        let u = lazy.cont(&down(0.5, 0.0, 1.0)).unwrap();
        assert_eq!(u.dabs, 0);
    }

    #[test]
    fn smoothing_damps_a_zigzag() {
        let zigzag = |smooth: f32| {
            let mut b = draw(0.15, 0.001, 0.2);
            b.stroke_smooth = smooth;
            let mut rig = flat();
            rig.begin(&b, &Symmetry::none(), &down(-0.8, 0.0, 0.0)).unwrap();
            let mut centers = Vec::new();
            for i in 1..=60 {
                let x = -0.8 + 1.6 * i as f32 / 60.0;
                let z = if i % 2 == 0 { 0.2 } else { -0.2 };
                centers.extend(rig.cont(&down(x, z, i as f64 * 0.01)).unwrap().centers);
            }
            let mean = centers.iter().map(|c| c.z).sum::<f32>() / centers.len() as f32;
            centers.iter().map(|c| (c.z - mean).abs()).fold(0.0f32, f32::max)
        };
        let raw = zigzag(0.0);
        let smooth = zigzag(0.9);
        assert!(raw > 0.15, "{raw}");
        assert!(smooth < raw * 0.5, "{smooth} !< {raw}/2");
    }

    #[test]
    fn airbrush_emits_by_time_when_cursor_is_still() {
        let mut b = draw(0.2, 0.01, 0.5);
        b.airbrush = true;
        b.airbrush_rate = 10.0;
        let mut rig = flat();
        rig.begin(&b, &Symmetry::none(), &down(0.0, 0.0, 0.0)).unwrap();
        let mut n = 0;
        for i in 1..=10 {
            n += rig.cont(&down(0.0, 0.0, i as f64 * 0.1)).unwrap().dabs;
        }
        assert_eq!(n, 10, "10 Hz durante 1 s");
        // movendo: dabs por distância, sem somar tempo
        let mut rig = flat();
        rig.begin(&b, &Symmetry::none(), &down(-0.5, 0.0, 0.0)).unwrap();
        let u = rig.cont(&down(0.5, 0.0, 0.05)).unwrap();
        assert_eq!(u.dabs, 10, "1.0 de trajeto / passo 0.1");
    }

    #[test]
    fn symmetry_stroke_is_symmetric() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 16));
        let b = draw(0.25, 0.2, 0.15);
        let sym = Symmetry::mirror_x();
        // câmera em +Z olhando para -Z; traço da direita para o meio na face frontal
        let ray = |x: f32, y: f32, t: f64| StrokeSample::new(Vec3::new(x, y, 5.0), -Vec3::Z, t);
        rig.begin(&b, &sym, &ray(0.7, 0.2, 0.0)).unwrap();
        for i in 1..=10 {
            rig.cont(&ray(0.7 - 0.05 * i as f32, 0.2 + 0.03 * i as f32, i as f64 * 0.01)).unwrap();
        }
        rig.end();
        let mm = crate::sculpt::symmetry::build_mirror_map(&rig.mesh, 0, 2e-3).unwrap();
        assert!(mm.is_complete(), "{} sem par", mm.unmatched);
        let moved = rig.mesh.verts.iter().filter(|v| (v.co.length() - 1.0).abs() > 1e-4).count();
        assert!(moved > 30);
    }

    #[test]
    fn grab_stroke_drags_in_the_view_plane() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 16));
        let orig = rig.mesh.clone();
        let mut b = Brush::new(BrushKind::Grab);
        b.radius = 0.4;
        let ray = |x: f32, y: f32, t: f64| StrokeSample::new(Vec3::new(x, y, 5.0), -Vec3::Z, t);
        let u = rig.begin(&b, &Symmetry::none(), &ray(0.0, 0.0, 0.0)).unwrap();
        assert!(u.started && u.result.moved_verts.is_empty());
        let front = rig.mesh.verts.iter().position(|v| v.co.z > 0.99).unwrap();
        let mut last_total = 0;
        for i in 1..=10 {
            let u = rig.cont(&ray(0.0, 0.03 * i as f32, i as f64 * 0.01)).unwrap();
            assert_eq!(u.dabs, 1);
            last_total = u.result.affected_verts.len();
        }
        assert!(last_total > 5);
        // o ponto atingido (centro) acompanhou o mouse: +0.3 em y
        let hit_v = (0..rig.mesh.vert_count())
            .filter(|&v| orig.verts[v].co.distance(Vec3::new(0.0, 0.0, 1.0)) < 0.05)
            .min_by(|&a, &b| {
                orig.verts[a]
                    .co
                    .distance(Vec3::Z)
                    .total_cmp(&orig.verts[b].co.distance(Vec3::Z))
            })
            .unwrap();
        let disp = rig.mesh.verts[hit_v].co - orig.verts[hit_v].co;
        assert!((disp.y - 0.3).abs() < 0.02 && disp.z.abs() < 1e-4 && disp.x.abs() < 1e-4, "{disp:?}");
        let _ = front;
        let out = rig.end();
        out.deltas.revert(&mut rig.mesh);
        assert_eq!(rig.mesh, orig);
        // mouse parado: nenhum dab
        rig.begin(&b, &Symmetry::none(), &ray(0.0, 0.0, 0.0)).unwrap();
        assert_eq!(rig.cont(&ray(0.0, 0.0, 0.1)).unwrap().dabs, 0);
        rig.end();
    }

    #[test]
    fn grab_ray_parallel_to_plane_is_ignored() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 8));
        let mut b = Brush::new(BrushKind::Grab);
        b.radius = 0.4;
        rig.begin(&b, &Symmetry::none(), &StrokeSample::new(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z, 0.0))
            .unwrap();
        // raio paralelo ao plano de visão: sem interseção
        let u = rig.cont(&StrokeSample::new(Vec3::new(0.0, 0.0, 5.0), Vec3::X, 0.1)).unwrap();
        assert_eq!(u.dabs, 0);
        rig.end();
    }

    #[test]
    fn snake_hook_stroke_pulls_geometry_out_of_the_surface() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 16));
        let mut b = Brush::new(BrushKind::SnakeHook);
        b.radius = 0.3;
        b.spacing = 0.05;
        // arrasta em +X no plano de visão: o conteúdo acompanha
        let ray = |x: f32, t: f64| StrokeSample::new(Vec3::new(x, 0.0, 5.0), -Vec3::Z, t);
        rig.begin(&b, &Symmetry::none(), &ray(0.0, 0.0)).unwrap();
        for i in 1..=20 {
            rig.cont(&ray(0.04 * i as f32, i as f64 * 0.01)).unwrap();
        }
        rig.end();
        // o vértice atingido no início foi arrastado junto com o pincel (com algum atraso, como no Blender)
        let v0 = rig.vert_idx_near(Vec3::Z);
        let moved_x = rig.mesh.verts[v0].co.x;
        assert!(moved_x > 0.5, "vértice arrastado: x = {moved_x}");
        rig.mesh.validate().unwrap();
    }

    #[test]
    fn nudge_and_clay_strips_receive_stroke_direction() {
        let mut rig = flat();
        let mut b = Brush::new(BrushKind::Nudge);
        b.radius = 0.2;
        b.strength = 0.5;
        b.spacing = 0.2;
        let (n, _) = run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, 10);
        assert!(n > 5);
        rig.end();
        // nudge empurrou material para +X: o centróide X das posições subiu
        let cx: f32 = rig.mesh.verts.iter().map(|v| v.co.x).sum::<f32>() / rig.mesh.vert_count() as f32;
        assert!(cx > 0.0, "{cx}");
        assert!(rig.mesh.verts.iter().all(|v| v.co.y.abs() < 1e-4), "nudge não levanta o plano");

        let mut rig = flat();
        let mut b = Brush::new(BrushKind::ClayStrips);
        b.radius = 0.2;
        b.spacing = 0.2;
        run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, 10);
        rig.end();
        assert!(rig.mesh.verts.iter().any(|v| v.co.y > 1e-4));
    }

    #[test]
    fn mask_stroke_paints_mask_and_undo_restores_it() {
        let mut rig = flat();
        let mut b = Brush::new(BrushKind::MaskPaint);
        b.radius = 0.2;
        b.strength = 0.5;
        b.spacing = 0.2;
        let orig = rig.mesh.clone();
        run_line(&mut rig, &b, &Symmetry::none(), -0.5, 0.5, 10);
        let out = rig.end();
        assert!(rig.mesh.verts.iter().any(|v| v.mask > 0.5));
        assert!(out.deltas.positions.is_empty());
        assert!(!out.deltas.masks.is_empty());
        out.deltas.revert(&mut rig.mesh);
        assert_eq!(rig.mesh, orig);
    }

    #[test]
    fn cancel_reverts_mesh_normals_and_bvh() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 12));
        let orig = rig.mesh.clone();
        let mut b = draw(0.3, 0.3, 0.1);
        b.use_frontface = true;
        let ray = |x: f32, t: f64| StrokeSample::new(Vec3::new(x, 0.1, 5.0), -Vec3::Z, t);
        rig.begin(&b, &Symmetry::none(), &ray(-0.3, 0.0)).unwrap();
        for i in 1..=8 {
            rig.cont(&ray(-0.3 + 0.08 * i as f32, i as f64 * 0.01)).unwrap();
        }
        assert_ne!(rig.mesh, orig);
        rig.stroke.cancel(&mut rig.mesh, &mut rig.bvh, &mut rig.ctx).unwrap();
        assert_eq!(rig.mesh, orig);
        assert!(!rig.stroke.is_active() && !rig.ctx.stroke.active);
        let fresh = SculptContext::new(&rig.mesh).unwrap();
        for v in 0..rig.mesh.vert_count() {
            assert!((fresh.vert_normals[v] - rig.ctx.vert_normals[v]).length() < 1e-5);
        }
        for i in 0..rig.bvh.tri_count() as u32 {
            let tr = rig.bvh.tri_ref(i).unwrap();
            for k in 0..3 {
                assert_eq!(rig.bvh.triangle(i)[k], rig.mesh.co(tr.verts[k]));
            }
        }
        // cancel sem traço é no-op
        rig.stroke.cancel(&mut rig.mesh, &mut rig.bvh, &mut rig.ctx).unwrap();
    }

    #[test]
    fn consecutive_strokes_reuse_state_and_are_deterministic() {
        let run = || {
            let mut rig = Rig::new(primitives::quad_sphere(1.0, 12));
            let b = draw(0.25, 0.2, 0.15);
            let mut rng = Rng(8);
            for s in 0..4 {
                let y = rng.range(-0.5, 0.5);
                rig.begin(&b, &Symmetry::mirror_x(), &StrokeSample::new(Vec3::new(-0.4, y, 5.0), -Vec3::Z, 0.0))
                    .unwrap();
                for i in 1..=10 {
                    rig.cont(&StrokeSample::new(
                        Vec3::new(-0.4 + 0.08 * i as f32, y + 0.02 * s as f32, 5.0),
                        -Vec3::Z,
                        i as f64 * 0.01,
                    ))
                    .unwrap();
                }
                rig.end();
            }
            rig.mesh.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn long_stroke_on_large_mesh_runs_in_reasonable_time() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 64));
        let b = draw(0.1, 0.05, 0.1);
        let ray = |t: f32, time: f64| {
            let a = t * std::f32::consts::PI;
            StrokeSample::new(Vec3::new(a.cos() * 0.7, a.sin() * 0.7, 5.0), -Vec3::Z, time)
        };
        let t0 = std::time::Instant::now();
        rig.begin(&b, &Symmetry::mirror_x(), &ray(0.0, 0.0)).unwrap();
        let mut dabs = 0;
        for i in 1..=300 {
            dabs += rig.cont(&ray(i as f32 / 300.0, i as f64 * 0.005)).unwrap().dabs;
        }
        let out = rig.end();
        let el = t0.elapsed();
        println!(
            "[stroke perf] {} dabs (x2 simetria) em quad_sphere(1,64): {:?}; delta com {} vértices",
            dabs + 1,
            el,
            out.deltas.positions.entries.len()
        );
        assert!(dabs > 20);
        assert!(el.as_secs() < 120);
    }
}
