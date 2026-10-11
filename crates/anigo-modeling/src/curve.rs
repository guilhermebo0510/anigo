//! Curvas 3D para cabelo, fitas e cabos, com geradores de malha.
//!
//! ## Curvas
//!
//! [`Curve`] é uma sequência de [`CurvePoint`] (posição, tilt, raio, alças
//! opcionais) de tipo [`CurveKind::Bezier`] (cúbica por segmento, alças
//! **absolutas**), [`CurveKind::CatmullRom`] (uniforme; equivale a Bézier com
//! alças `(P[i+1] − P[i−1]) / 6`) ou [`CurveKind::Polyline`]. Curvas abertas
//! usam pontos fantasma por reflexão nas pontas (`P[−1] = 2P[0] − P[1]`).
//! `cyclic` fecha a curva (o último ponto liga ao primeiro).
//!
//! Alças ausentes em Bézier são geradas automaticamente: no interior
//! `out = P + (P[i+1] − P[i−1]) / 6`, `in = P − (…)/6` (resultado idêntico ao
//! Catmull-Rom); nas pontas abertas `out = P + (P[1] − P) / 3`.
//!
//! A avaliação é por parâmetro ([`Curve::sample`], `t ∈ [0, 1]` linear nos
//! segmentos) ou por **comprimento de arco** ([`Curve::sample_uniform`]); tilt e
//! raio são interpolados linearmente entre pontos de controle.
//!
//! ## Frames
//!
//! [`parallel_transport_frames`] calcula frames de **mínima rotação (Bishop)**
//! pelo método da dupla reflexão (Wang et al. 2008):
//! `v₁ = x₁ − x₀; r_L = r₀ − 2(v₁·r₀)/(v₁·v₁)·v₁; t_L = t₀ − 2(v₁·t₀)/(v₁·v₁)·v₁;
//! v₂ = t₁ − t_L; r₁ = r_L − 2(v₂·r_L)/(v₂·v₂)·v₂`. Diferente de Frenet, não
//! gira em trechos retos nem acumula torção. Em curvas fechadas a
//! holonomia residual é distribuída linearmente ao longo do comprimento para
//! que a malha feche sem costura torcida.
//!
//! Convenção dos frames: `(normal, binormal, tangent)` é destro
//! (`normal × binormal = tangent`, logo `binormal = tangent × normal`).
//!
//! ## Malhas
//!
//! [`curve_to_tube`], [`curve_to_ribbon`], [`hair_card`] e [`hair_clump`]
//! (mecha procedural). Todas validam a entrada antes de construir e são
//! determinísticas.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{Corner, EditMesh, Face};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

// =============================================================================
// Curva
// =============================================================================

/// Tipo de interpolação da curva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurveKind {
    Bezier,
    CatmullRom,
    Polyline,
}

/// Ponto de controle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub pos: Vec3,
    /// Rotação do frame ao redor da tangente (radianos).
    pub tilt: f32,
    /// Multiplicador de raio/largura (>= 0).
    pub radius: f32,
    /// Alça de entrada (posição absoluta; só Bézier). `None` = automática.
    pub handle_in: Option<Vec3>,
    /// Alça de saída (posição absoluta; só Bézier). `None` = automática.
    pub handle_out: Option<Vec3>,
}

impl CurvePoint {
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            tilt: 0.0,
            radius: 1.0,
            handle_in: None,
            handle_out: None,
        }
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn with_tilt(mut self, tilt: f32) -> Self {
        self.tilt = tilt;
        self
    }
}

/// Amostra da curva.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSample {
    pub pos: Vec3,
    /// Tangente unitária.
    pub tangent: Vec3,
    pub tilt: f32,
    pub radius: f32,
    /// Parâmetro `t ∈ [0, 1]`.
    pub t: f32,
    /// Comprimento de arco acumulado desde o início.
    pub s: f32,
}

/// Frame ortonormal destro ao longo da curva.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub pos: Vec3,
    pub tangent: Vec3,
    pub normal: Vec3,
    pub binormal: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub points: Vec<CurvePoint>,
    pub kind: CurveKind,
    pub cyclic: bool,
}

/// Subdivisões por segmento na tabela de comprimento de arco.
const ARC_SUBDIV: usize = 32;

struct ArcTable {
    ts: Vec<f32>,
    lens: Vec<f32>,
}

impl ArcTable {
    fn total(&self) -> f32 {
        *self.lens.last().unwrap_or(&0.0)
    }

    /// Parâmetro `t` para o comprimento de arco `s` (interpolação linear na tabela).
    fn t_at(&self, s: f32) -> f32 {
        let total = self.total();
        if total <= 0.0 {
            return 0.0;
        }
        let s = s.clamp(0.0, total);
        let idx = self.lens.partition_point(|&l| l < s).clamp(1, self.lens.len() - 1);
        let (l0, l1) = (self.lens[idx - 1], self.lens[idx]);
        let f = if l1 > l0 { (s - l0) / (l1 - l0) } else { 0.0 };
        self.ts[idx - 1] + (self.ts[idx] - self.ts[idx - 1]) * f
    }
}

fn cubic(b: &[Vec3; 4], u: f32) -> Vec3 {
    let v = 1.0 - u;
    b[0] * (v * v * v) + b[1] * (3.0 * v * v * u) + b[2] * (3.0 * v * u * u) + b[3] * (u * u * u)
}

fn cubic_deriv(b: &[Vec3; 4], u: f32) -> Vec3 {
    let v = 1.0 - u;
    ((b[1] - b[0]) * (v * v) + (b[2] - b[1]) * (2.0 * v * u) + (b[3] - b[2]) * (u * u)) * 3.0
}

impl Curve {
    pub fn new(kind: CurveKind, points: Vec<CurvePoint>, cyclic: bool) -> Self {
        Self { points, kind, cyclic }
    }

    /// Curva a partir de posições (tilt 0, raio 1, alças automáticas).
    pub fn from_positions(kind: CurveKind, positions: &[Vec3], cyclic: bool) -> Self {
        Self {
            points: positions.iter().map(|&p| CurvePoint::new(p)).collect(),
            kind,
            cyclic,
        }
    }

    /// Círculo no plano XZ com `n` pontos de controle (fechado).
    pub fn circle_xz(radius: f32, n: usize, kind: CurveKind) -> Self {
        let n = n.max(3);
        let pts: Vec<Vec3> = (0..n)
            .map(|i| {
                let a = TAU * i as f32 / n as f32;
                Vec3::new(radius * a.cos(), 0.0, radius * a.sin())
            })
            .collect();
        Self::from_positions(kind, &pts, true)
    }

    /// Valida a curva: >= 2 pontos (>= 3 se cíclica), tudo finito, raio >= 0.
    pub fn validate(&self) -> ModelResult<()> {
        let min = if self.cyclic { 3 } else { 2 };
        if self.points.len() < min {
            return Err(ModelError::InvalidParameter(format!(
                "curva precisa de >= {min} pontos, tem {}",
                self.points.len()
            )));
        }
        for (i, p) in self.points.iter().enumerate() {
            let handles_ok = p.handle_in.map(|h| h.is_finite()).unwrap_or(true)
                && p.handle_out.map(|h| h.is_finite()).unwrap_or(true);
            if !p.pos.is_finite() || !p.tilt.is_finite() || !p.radius.is_finite() || !handles_ok {
                return Err(ModelError::Numeric(format!("ponto {i} da curva não finito")));
            }
            if p.radius < 0.0 {
                return Err(ModelError::InvalidParameter(format!("ponto {i} com raio negativo")));
            }
        }
        Ok(())
    }

    /// Número de segmentos (n − 1 aberta, n fechada).
    pub fn segment_count(&self) -> usize {
        let n = self.points.len();
        if n < 2 {
            0
        } else if self.cyclic {
            n
        } else {
            n - 1
        }
    }

    fn pt(&self, i: isize) -> Vec3 {
        let n = self.points.len() as isize;
        if self.cyclic {
            self.points[i.rem_euclid(n) as usize].pos
        } else if i < 0 {
            2.0 * self.points[0].pos - self.points[1.min(self.points.len() - 1)].pos
        } else if i >= n {
            let last = self.points.len() - 1;
            2.0 * self.points[last].pos - self.points[last.saturating_sub(1)].pos
        } else {
            self.points[i as usize].pos
        }
    }

    /// Pontos de controle Bézier do segmento `i`.
    fn segment(&self, i: usize) -> [Vec3; 4] {
        let n = self.points.len();
        let (a, b) = (i % n, (i + 1) % n);
        let (pa, pb) = (self.points[a].pos, self.points[b].pos);
        let ii = i as isize;
        match self.kind {
            CurveKind::Polyline => [pa, pa + (pb - pa) / 3.0, pa + (pb - pa) * (2.0 / 3.0), pb],
            CurveKind::CatmullRom => {
                let (p0, p3) = (self.pt(ii - 1), self.pt(ii + 2));
                [pa, pa + (pb - p0) / 6.0, pb - (p3 - pa) / 6.0, pb]
            }
            CurveKind::Bezier => {
                let out = self.points[a].handle_out.unwrap_or_else(|| {
                    if !self.cyclic && a == 0 {
                        pa + (pb - pa) / 3.0
                    } else {
                        pa + (pb - self.pt(ii - 1)) / 6.0
                    }
                });
                let inn = self.points[b].handle_in.unwrap_or_else(|| {
                    if !self.cyclic && b == n - 1 {
                        pb + (pa - pb) / 3.0
                    } else {
                        pb - (self.pt(ii + 2) - pa) / 6.0
                    }
                });
                [pa, out, inn, pb]
            }
        }
    }

    fn segments(&self) -> Vec<[Vec3; 4]> {
        (0..self.segment_count()).map(|i| self.segment(i)).collect()
    }

    /// `(segmento, u)` para `t`; fechada: `t` envolve; aberta: limitado.
    fn locate(&self, t: f32) -> (usize, f32) {
        let segs = self.segment_count();
        if segs == 0 {
            return (0, 0.0);
        }
        let t = if !t.is_finite() {
            0.0
        } else if self.cyclic {
            t.rem_euclid(1.0)
        } else {
            t.clamp(0.0, 1.0)
        };
        let x = t * segs as f32;
        let seg = (x.floor() as usize).min(segs - 1);
        (seg, (x - seg as f32).clamp(0.0, 1.0))
    }

    fn tangent_from(&self, bz: &[Vec3; 4], u: f32) -> Vec3 {
        if let Some(t) = cubic_deriv(bz, u).try_normalize() {
            return t;
        }
        // derivada nula (alças coincidentes): secante local
        let d = 1.0e-3;
        let (u0, u1) = ((u - d).max(0.0), (u + d).min(1.0));
        if let Some(t) = (cubic(bz, u1) - cubic(bz, u0)).try_normalize() {
            return t;
        }
        if let Some(t) = (bz[3] - bz[0]).try_normalize() {
            return t;
        }
        // último recurso: primeira corda não nula da curva
        for w in self.points.windows(2) {
            if let Some(t) = (w[1].pos - w[0].pos).try_normalize() {
                return t;
            }
        }
        Vec3::Z
    }

    fn sample_in(&self, bz: &[Vec3; 4], seg: usize, u: f32, s: f32) -> CurveSample {
        let n = self.points.len();
        let (pa, pb) = (&self.points[seg % n], &self.points[(seg + 1) % n]);
        let segs = self.segment_count().max(1);
        CurveSample {
            pos: cubic(bz, u),
            tangent: self.tangent_from(bz, u),
            tilt: pa.tilt + (pb.tilt - pa.tilt) * u,
            radius: pa.radius + (pb.radius - pa.radius) * u,
            t: (seg as f32 + u) / segs as f32,
            s,
        }
    }

    /// Amostra por parâmetro `t ∈ [0, 1]` (fechada: envolve; não finito → 0).
    /// `s` fica 0 (use [`Self::sample_uniform`] para comprimento de arco).
    /// Curvas com < 2 pontos devolvem o ponto único (ou a origem).
    pub fn sample(&self, t: f32) -> CurveSample {
        if self.points.len() < 2 {
            let p = self.points.first().copied().unwrap_or(CurvePoint::new(Vec3::ZERO));
            return CurveSample {
                pos: p.pos,
                tangent: Vec3::Z,
                tilt: p.tilt,
                radius: p.radius,
                t: 0.0,
                s: 0.0,
            };
        }
        let (seg, u) = self.locate(t);
        let bz = self.segment(seg);
        self.sample_in(&bz, seg, u, 0.0)
    }

    fn arc_table(&self, segs: &[[Vec3; 4]]) -> ArcTable {
        let nseg = segs.len();
        let mut ts = vec![0.0];
        let mut lens = vec![0.0];
        if nseg == 0 {
            return ArcTable { ts, lens };
        }
        let mut prev = cubic(&segs[0], 0.0);
        let mut acc = 0.0f32;
        for (i, bz) in segs.iter().enumerate() {
            for k in 1..=ARC_SUBDIV {
                let u = k as f32 / ARC_SUBDIV as f32;
                let p = cubic(bz, u);
                acc += p.distance(prev);
                prev = p;
                ts.push((i as f32 + u) / nseg as f32);
                lens.push(acc);
            }
        }
        ArcTable { ts, lens }
    }

    /// Comprimento de arco total (soma de cordas; erro O(h²), h = 1/32 do segmento).
    pub fn length(&self) -> f32 {
        if self.points.len() < 2 {
            return 0.0;
        }
        self.arc_table(&self.segments()).total()
    }

    /// `n` amostras equidistantes em comprimento de arco. Aberta: inclui as duas
    /// pontas (`n >= 2`); fechada: `s_i = i·L/n` sem repetir o ponto inicial
    /// (`n >= 3`). Curva sem comprimento é erro.
    pub fn sample_uniform(&self, n: usize) -> ModelResult<Vec<CurveSample>> {
        self.validate()?;
        let min = if self.cyclic { 3 } else { 2 };
        if n < min {
            return Err(ModelError::InvalidParameter(format!("sample_uniform precisa de n >= {min}")));
        }
        let segs = self.segments();
        let table = self.arc_table(&segs);
        let total = table.total();
        if !(total > 1e-12) {
            return Err(ModelError::Numeric("curva de comprimento zero".into()));
        }
        let denom = if self.cyclic { n } else { n - 1 } as f32;
        Ok((0..n)
            .map(|i| {
                let s = total * i as f32 / denom;
                let t = table.t_at(s);
                let (seg, u) = self.locate(t);
                // t = 1 exato em curva aberta cai no fim do último segmento
                let (seg, u) = if !self.cyclic && i == n - 1 { (segs.len() - 1, 1.0) } else { (seg, u) };
                self.sample_in(&segs[seg], seg, u, s)
            })
            .collect())
    }

    /// Frames de Bishop em `n` amostras uniformes (ver [`parallel_transport_frames`]).
    /// O `tilt` interpolado de cada amostra já está aplicado. `up_hint` orienta
    /// a normal inicial.
    pub fn frames_uniform(&self, n: usize, up_hint: Option<Vec3>) -> ModelResult<(Vec<CurveSample>, Vec<Frame>)> {
        let samples = self.sample_uniform(n)?;
        let frames = frames_for_samples(&samples, self.cyclic, up_hint);
        Ok((samples, frames))
    }
}

// =============================================================================
// Frames de Bishop
// =============================================================================

fn any_perpendicular(t: Vec3) -> Vec3 {
    let helper = if t.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    t.cross(helper).try_normalize().unwrap_or(Vec3::X)
}

fn project_perp(r: Vec3, t: Vec3) -> Vec3 {
    (r - t * t.dot(r))
        .try_normalize()
        .unwrap_or_else(|| any_perpendicular(t))
}

/// Transporta o vetor `r` (perpendicular a `t0`) de `(x0, t0)` para `(x1, t1)`
/// por dupla reflexão.
fn transport(x0: Vec3, t0: Vec3, r0: Vec3, x1: Vec3, t1: Vec3) -> Vec3 {
    let v1 = x1 - x0;
    let c1 = v1.length_squared();
    if c1 < 1e-20 {
        return project_perp(r0, t1);
    }
    let r_l = r0 - v1 * (2.0 / c1 * v1.dot(r0));
    let t_l = t0 - v1 * (2.0 / c1 * v1.dot(t0));
    let v2 = t1 - t_l;
    let c2 = v2.length_squared();
    let r1 = if c2 < 1e-20 {
        r_l
    } else {
        r_l - v2 * (2.0 / c2 * v2.dot(r_l))
    };
    project_perp(r1, t1)
}

fn rotate_about(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// Frames de mínima rotação ao longo de pontos `positions` com tangentes
/// unitárias `tangents`. `initial_normal` (opcional) é projetada perpendicular à
/// primeira tangente; se for paralela, usa-se uma perpendicular arbitrária
/// estável. Com `closed`, a holonomia (ângulo entre o frame transportado até o
/// início e o frame inicial) é removida distribuindo `−holonomia · s/L`
/// (comprimento de corda acumulado) ao longo dos frames. Sem tilt aplicado.
///
/// Entradas vazias ou de tamanhos diferentes devolvem vetor vazio.
pub fn parallel_transport_frames(
    positions: &[Vec3],
    tangents: &[Vec3],
    closed: bool,
    initial_normal: Option<Vec3>,
) -> Vec<Frame> {
    let n = positions.len();
    if n == 0 || tangents.len() != n {
        return Vec::new();
    }
    let tg: Vec<Vec3> = tangents.iter().map(|t| t.try_normalize().unwrap_or(Vec3::Z)).collect();
    let n0 = match initial_normal {
        Some(h) if h.is_finite() && (h - tg[0] * tg[0].dot(h)).length_squared() > 1e-10 => project_perp(h, tg[0]),
        _ => any_perpendicular(tg[0]),
    };
    let mut normals = Vec::with_capacity(n);
    normals.push(n0);
    for i in 1..n {
        let prev = normals[i - 1];
        normals.push(transport(positions[i - 1], tg[i - 1], prev, positions[i], tg[i]));
    }
    if closed && n >= 3 {
        // holonomia: transporta o último frame até o início
        let back = transport(positions[n - 1], tg[n - 1], normals[n - 1], positions[0], tg[0]);
        let b0 = tg[0].cross(n0);
        let angle = back.dot(b0).atan2(back.dot(n0));
        let mut cum = vec![0.0f32; n + 1];
        for i in 0..n {
            cum[i + 1] = cum[i] + positions[(i + 1) % n].distance(positions[i]);
        }
        let total = cum[n];
        if total > 0.0 {
            for i in 0..n {
                // o frame transportado difere do inicial por +angle ao fim do laço;
                // gira cada frame de −angle·s/L para fechar
                normals[i] = rotate_about(normals[i], tg[i], -angle * cum[i] / total);
            }
        }
    }
    (0..n)
        .map(|i| {
            let normal = project_perp(normals[i], tg[i]);
            Frame {
                pos: positions[i],
                tangent: tg[i],
                normal,
                binormal: tg[i].cross(normal),
            }
        })
        .collect()
}

/// Frames de Bishop para amostras de curva, com o `tilt` de cada amostra aplicado.
fn frames_for_samples(samples: &[CurveSample], cyclic: bool, up_hint: Option<Vec3>) -> Vec<Frame> {
    let pos: Vec<Vec3> = samples.iter().map(|s| s.pos).collect();
    let tan: Vec<Vec3> = samples.iter().map(|s| s.tangent).collect();
    let mut frames = parallel_transport_frames(&pos, &tan, cyclic, up_hint);
    for (f, s) in frames.iter_mut().zip(samples) {
        if s.tilt != 0.0 {
            f.normal = rotate_about(f.normal, f.tangent, s.tilt);
            f.binormal = f.tangent.cross(f.normal);
        }
    }
    frames
}

// =============================================================================
// Perfis e utilidades de malha
// =============================================================================

/// Perfil de afilamento: `v(t) = start + (end − start)·t^power`, `t ∈ [0, 1]`
/// da raiz (0) à ponta (1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Taper {
    pub start: f32,
    pub end: f32,
    pub power: f32,
}

impl Taper {
    /// Sem afilamento (constante 1).
    pub const NONE: Taper = Taper {
        start: 1.0,
        end: 1.0,
        power: 1.0,
    };

    pub fn new(start: f32, end: f32, power: f32) -> Self {
        Self { start, end, power }
    }

    pub fn validate(&self) -> ModelResult<()> {
        if !self.start.is_finite() || !self.end.is_finite() || !self.power.is_finite() {
            return Err(ModelError::InvalidParameter("taper não finito".into()));
        }
        if self.start < 0.0 || self.end < 0.0 {
            return Err(ModelError::InvalidParameter("taper negativo".into()));
        }
        if self.power <= 0.0 {
            return Err(ModelError::InvalidParameter("taper.power deve ser > 0".into()));
        }
        Ok(())
    }

    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        self.start + (self.end - self.start) * t.powf(self.power)
    }
}

impl Default for Taper {
    fn default() -> Self {
        Taper::NONE
    }
}

const APEX_EPS: f32 = 1.0e-7;

fn check_pos(x: f32, what: &str) -> ModelResult<()> {
    if x.is_finite() && x > 0.0 {
        Ok(())
    } else {
        Err(ModelError::InvalidParameter(format!("{what} deve ser finito e > 0")))
    }
}

/// Monta uma face a partir de `(vértice, uv)` removendo repetições consecutivas
/// (cíclicas) — colapsa quads degenerados em triângulos. `None` se sobrar < 3.
fn dedup_face(corners: &[(u32, Vec2, Option<Vec3>)], smooth: bool) -> Option<Face> {
    let mut list: Vec<(u32, Vec2, Option<Vec3>)> = Vec::with_capacity(corners.len());
    for &c in corners {
        if list.last().map(|l| l.0 == c.0).unwrap_or(false) {
            continue;
        }
        list.push(c);
    }
    while list.len() > 1 && list[0].0 == list[list.len() - 1].0 {
        list.pop();
    }
    if list.len() < 3 {
        return None;
    }
    // vértices repetidos não consecutivos tornariam a face inválida
    let mut seen = std::collections::HashSet::new();
    if !list.iter().all(|c| seen.insert(c.0)) {
        return None;
    }
    Some(Face {
        corners: list
            .into_iter()
            .map(|(vert, uv, normal)| Corner { vert, uv, normal })
            .collect(),
        material: 0,
        smooth,
        face_set: 0,
        hidden: false,
    })
}

// =============================================================================
// Tubo
// =============================================================================

/// Parâmetros de [`curve_to_tube`].
#[derive(Debug, Clone, Copy)]
pub struct TubeParams {
    /// Lados da seção (3..=256).
    pub sides: u32,
    /// Anéis ao longo da curva (>= 2; >= 3 se cíclica), uniformes em arco.
    pub rings: u32,
    /// Raio base (> 0).
    pub radius: f32,
    /// Afilamento do raio ao longo do comprimento.
    pub radius_profile: Taper,
    /// Multiplica pelo raio interpolado dos pontos de controle.
    pub use_point_radius: bool,
    /// Tampas nas pontas (ignoradas em curva cíclica e em pontas com raio 0).
    pub cap_start: bool,
    pub cap_end: bool,
    /// `None`: V vai de 0 a 1 ao longo do comprimento; `Some(k)`: V = comprimento · k.
    pub uv_v_per_unit: Option<f32>,
    /// Orienta a costura de UV (normal inicial do frame).
    pub up_hint: Option<Vec3>,
}

impl Default for TubeParams {
    fn default() -> Self {
        Self {
            sides: 8,
            rings: 16,
            radius: 0.05,
            radius_profile: Taper::NONE,
            use_point_radius: true,
            cap_start: true,
            cap_end: true,
            uv_v_per_unit: None,
            up_hint: None,
        }
    }
}

/// Tubo (cabo, tronco, tentáculo) ao longo da curva, com frames de Bishop.
///
/// Vértice `(r, s)` = `c_r + ρ_r·(cos θ_s · N_r + sin θ_s · B_r)`, `θ_s = 2πs/sides`,
/// `ρ_r = radius · taper(t_r) · raio_do_ponto`. Faces quad `(r,s) (r,s+1)
/// (r+1,s+1) (r+1,s)` com normal para fora. Anéis com `ρ ≤ 1e-7` nas pontas
/// colapsam num único vértice (ponta cônica, sem tampa); no interior o raio é
/// limitado a `1e-6`. UV: U ao redor (0..1, com costura marcada em `θ = 0`),
/// V ao longo (cíclica: costura também no anel 0). Curvas cíclicas dão um toro
/// fechado; abertas com tampas dão um sólido 2-manifold fechado.
pub fn curve_to_tube(curve: &Curve, params: &TubeParams) -> ModelResult<EditMesh> {
    curve.validate()?;
    params.radius_profile.validate()?;
    check_pos(params.radius, "radius")?;
    if !(3..=256).contains(&params.sides) {
        return Err(ModelError::InvalidParameter(format!("sides {} fora de 3..=256", params.sides)));
    }
    let min_rings = if curve.cyclic { 3 } else { 2 };
    if params.rings < min_rings || params.rings > 100_000 {
        return Err(ModelError::InvalidParameter(format!(
            "rings {} fora de {min_rings}..=100000",
            params.rings
        )));
    }
    if let Some(k) = params.uv_v_per_unit {
        check_pos(k, "uv_v_per_unit")?;
    }
    let (samples, frames) = curve.frames_uniform(params.rings as usize, params.up_hint)?;
    let total_len = if curve.cyclic {
        curve.length()
    } else {
        samples.last().map(|s| s.s).unwrap_or(0.0)
    };
    let rings = params.rings as usize;
    let sides = params.sides as usize;
    let mut mesh = EditMesh::new("Tube");
    // ids[r][s]
    let mut ids: Vec<Vec<u32>> = Vec::with_capacity(rings);
    let mut apex = vec![false; rings];
    let mut vcoord = vec![0.0f32; rings];
    for r in 0..rings {
        let t = if curve.cyclic {
            r as f32 / rings as f32
        } else {
            r as f32 / (rings - 1) as f32
        };
        let pr = if params.use_point_radius { samples[r].radius } else { 1.0 };
        let mut rho = params.radius * params.radius_profile.eval(t) * pr;
        let is_end = !curve.cyclic && (r == 0 || r == rings - 1);
        vcoord[r] = match params.uv_v_per_unit {
            Some(k) => samples[r].s * k,
            None => {
                if total_len > 0.0 {
                    samples[r].s / total_len
                } else {
                    0.0
                }
            }
        };
        let f = &frames[r];
        if rho <= APEX_EPS && is_end {
            apex[r] = true;
            let v = mesh.add_vert(f.pos);
            ids.push(vec![v; sides]);
        } else {
            rho = rho.max(1.0e-6);
            ids.push(
                (0..sides)
                    .map(|s| {
                        let th = TAU * s as f32 / sides as f32;
                        mesh.add_vert(f.pos + (f.normal * th.cos() + f.binormal * th.sin()) * rho)
                    })
                    .collect(),
            );
        }
    }
    let seg_count = if curve.cyclic { rings } else { rings - 1 };
    let v_end_wrap = match params.uv_v_per_unit {
        Some(k) => total_len * k,
        None => 1.0,
    };
    for r in 0..seg_count {
        let r1 = (r + 1) % rings;
        let (va, vb) = (vcoord[r], if r + 1 == rings { v_end_wrap } else { vcoord[r1] });
        for s in 0..sides {
            let s1 = (s + 1) % sides;
            let (u0, u1) = (s as f32 / sides as f32, (s + 1) as f32 / sides as f32);
            let um = (s as f32 + 0.5) / sides as f32;
            let ua = if apex[r] { (um, um) } else { (u0, u1) };
            let ub = if apex[r1] { (um, um) } else { (u0, u1) };
            let corners = [
                (ids[r][s], Vec2::new(ua.0, va), None),
                (ids[r][s1], Vec2::new(ua.1, va), None),
                (ids[r1][s1], Vec2::new(ub.1, vb), None),
                (ids[r1][s], Vec2::new(ub.0, vb), None),
            ];
            if let Some(face) = dedup_face(&corners, true) {
                mesh.push_face(face)?;
            }
        }
        if !apex[r] && !apex[r1] {
            mesh.set_seam(ids[r][0], ids[r1][0], true);
        }
    }
    if curve.cyclic {
        for s in 0..sides {
            mesh.set_seam(ids[0][s], ids[0][(s + 1) % sides], true);
        }
    } else {
        let cap = |mesh: &mut EditMesh, ring: usize, reverse: bool| -> ModelResult<()> {
            let order: Vec<usize> = if reverse {
                (0..sides).rev().collect()
            } else {
                (0..sides).collect()
            };
            let corners: Vec<(u32, Vec2, Option<Vec3>)> = order
                .iter()
                .map(|&s| {
                    let th = TAU * s as f32 / sides as f32;
                    (ids[ring][s], Vec2::new(0.5 + 0.5 * th.cos(), 0.5 + 0.5 * th.sin()), None)
                })
                .collect();
            if let Some(face) = dedup_face(&corners, false) {
                mesh.push_face(face)?;
            }
            Ok(())
        };
        if params.cap_start && !apex[0] {
            cap(&mut mesh, 0, true)?;
        }
        if params.cap_end && !apex[rings - 1] {
            cap(&mut mesh, rings - 1, false)?;
        }
    }
    mesh.validate()?;
    Ok(mesh)
}

// =============================================================================
// Fita
// =============================================================================

/// Parâmetros de [`curve_to_ribbon`].
#[derive(Debug, Clone, Copy)]
pub struct RibbonParams {
    /// Anéis ao longo da curva (>= 2; >= 3 se cíclica).
    pub rings: u32,
    /// Divisões na largura (>= 1).
    pub width_segments: u32,
    /// Largura base (> 0).
    pub width: f32,
    pub width_profile: Taper,
    pub use_point_radius: bool,
    /// Torção total ao longo do comprimento (radianos, aplicada linearmente em s).
    pub twist: f32,
    /// Direção para onde a face da fita olha no início (`None` = arbitrária estável).
    pub normal_up: Option<Vec3>,
}

impl Default for RibbonParams {
    fn default() -> Self {
        Self {
            rings: 16,
            width_segments: 1,
            width: 0.2,
            width_profile: Taper::NONE,
            use_point_radius: true,
            twist: 0.0,
            normal_up: None,
        }
    }
}

/// Fita (faixa plana) ao longo da curva: a face olha para `N` do frame de
/// Bishop (rodado por `twist·s/L`), a largura vai ao longo de `B`. Quads
/// `(r,0) (r,1)… (r+1,…)` com normal `+N`. Larguras zero nas pontas colapsam em
/// triângulos. UV: U across (0..1), V ao longo (0..1). Fita cíclica fecha como
/// faixa (não é sólido). Largura mínima no interior: `1e-6`.
pub fn curve_to_ribbon(curve: &Curve, params: &RibbonParams) -> ModelResult<EditMesh> {
    curve.validate()?;
    params.width_profile.validate()?;
    check_pos(params.width, "width")?;
    if !params.twist.is_finite() {
        return Err(ModelError::InvalidParameter("twist não finito".into()));
    }
    let min_rings = if curve.cyclic { 3 } else { 2 };
    if params.rings < min_rings || params.rings > 100_000 {
        return Err(ModelError::InvalidParameter(format!(
            "rings {} fora de {min_rings}..=100000",
            params.rings
        )));
    }
    if params.width_segments == 0 || params.width_segments > 256 {
        return Err(ModelError::InvalidParameter("width_segments fora de 1..=256".into()));
    }
    let (samples, frames) = curve.frames_uniform(params.rings as usize, params.normal_up)?;
    let total_len = if curve.cyclic {
        curve.length()
    } else {
        samples.last().map(|s| s.s).unwrap_or(0.0)
    };
    let rings = params.rings as usize;
    let m = params.width_segments as usize;
    let mut mesh = EditMesh::new("Ribbon");
    let mut ids: Vec<Vec<u32>> = Vec::with_capacity(rings);
    let mut apex = vec![false; rings];
    for r in 0..rings {
        let t = if curve.cyclic {
            r as f32 / rings as f32
        } else {
            r as f32 / (rings - 1) as f32
        };
        let pr = if params.use_point_radius { samples[r].radius } else { 1.0 };
        let mut hw = 0.5 * params.width * params.width_profile.eval(t) * pr;
        let is_end = !curve.cyclic && (r == 0 || r == rings - 1);
        let f = &frames[r];
        let frac = if total_len > 0.0 { samples[r].s / total_len } else { 0.0 };
        let b = rotate_about(f.binormal, f.tangent, params.twist * frac);
        if hw <= APEX_EPS && is_end {
            apex[r] = true;
            let v = mesh.add_vert(f.pos);
            ids.push(vec![v; m + 1]);
        } else {
            hw = hw.max(1.0e-6);
            ids.push(
                (0..=m)
                    .map(|k| {
                        let x = -1.0 + 2.0 * k as f32 / m as f32;
                        mesh.add_vert(f.pos + b * (hw * x))
                    })
                    .collect(),
            );
        }
    }
    let seg_count = if curve.cyclic { rings } else { rings - 1 };
    for r in 0..seg_count {
        let r1 = (r + 1) % rings;
        let va = if total_len > 0.0 { samples[r].s / total_len } else { 0.0 };
        let vb = if r + 1 == rings {
            1.0
        } else if total_len > 0.0 {
            samples[r1].s / total_len
        } else {
            0.0
        };
        for k in 0..m {
            let (u0, u1) = (k as f32 / m as f32, (k + 1) as f32 / m as f32);
            let um = 0.5 * (u0 + u1);
            let ua = if apex[r] { (um, um) } else { (u0, u1) };
            let ub = if apex[r1] { (um, um) } else { (u0, u1) };
            let corners = [
                (ids[r][k], Vec2::new(ua.0, va), None),
                (ids[r][k + 1], Vec2::new(ua.1, va), None),
                (ids[r1][k + 1], Vec2::new(ub.1, vb), None),
                (ids[r1][k], Vec2::new(ub.0, vb), None),
            ];
            if let Some(face) = dedup_face(&corners, true) {
                mesh.push_face(face)?;
            }
        }
    }
    mesh.validate()?;
    Ok(mesh)
}

// =============================================================================
// Hair card
// =============================================================================

/// Direção do viés de normal em [`HairCardParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalBiasDir {
    /// Normais voltadas **para** o alvo (couro cabeludo).
    TowardTarget,
    /// Normais voltadas para longe do alvo (esfera de sombreamento).
    AwayFromTarget,
}

/// Célula de um atlas de cartões (`cols × rows`, origem V embaixo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasCell {
    pub cols: u32,
    pub rows: u32,
    pub col: u32,
    pub row: u32,
}

/// Parâmetros de [`hair_card`].
#[derive(Debug, Clone, Copy)]
pub struct HairCardParams {
    /// Largura na raiz (> 0).
    pub width: f32,
    /// Razão de largura na ponta (0 = ponta em bico, 1 = sem afilamento).
    pub taper: f32,
    /// Expoente do afilamento (> 0): `taper(t) = 1 + (taper − 1)·t^power`.
    pub taper_power: f32,
    /// Segmentos ao longo do comprimento (>= 1).
    pub segments: u32,
    /// Divisões na largura (>= 1; com `curl != 0` usa-se no mínimo 2).
    pub width_segments: u32,
    /// Ângulo total (rad) do arco da seção transversal (0 = plano). Positivo:
    /// a face `+N` fica convexa.
    pub curl: f32,
    /// Bojo: largura × `(1 + flare·sin(π t))` (folha de cabelo). `>= −1`.
    pub flare: f32,
    /// Ponto (couro cabeludo/centro da cabeça) para o qual viesar as normais.
    pub normal_bias_to: Option<Vec3>,
    /// Quanto viesar (0..1). Ver [`hair_card`].
    pub normal_bias_amount: f32,
    pub normal_bias_dir: NormalBiasDir,
    /// Remapeia a UV para uma célula do atlas.
    pub uv_atlas_cell: Option<AtlasCell>,
    /// Orienta a face do cartão no início.
    pub up_hint: Option<Vec3>,
}

impl Default for HairCardParams {
    fn default() -> Self {
        Self {
            width: 0.1,
            taper: 0.0,
            taper_power: 1.0,
            segments: 8,
            width_segments: 1,
            curl: 0.0,
            flare: 0.0,
            normal_bias_to: None,
            normal_bias_amount: 0.5,
            normal_bias_dir: NormalBiasDir::TowardTarget,
            uv_atlas_cell: None,
            up_hint: None,
        }
    }
}

/// Cartão de cabelo (hair card) ao longo da curva, pronto para shading anime.
///
/// Largura em `t ∈ [0, 1]` (raiz → ponta):
/// `w(t) = width · (1 + (taper − 1)·t^taper_power) · (1 + flare·sin(πt)) · raio_do_ponto`.
/// A seção em cada anel é um arco de ângulo `curl` (reta se `curl = 0`) com
/// comprimento de arco `w`: para `φ_k = curl·x_k/2`, `x_k ∈ [−1, 1]` e
/// `R = w/curl`, o vértice é `c + B·R sin φ + N·R(cos φ − 1)` e a normal
/// analítica `N cos φ + B sin φ`. Toda face recebe **normais customizadas** por
/// corner (a forma do sombreamento é definida pelo cartão, não pelo ângulo
/// automático).
///
/// Viés (`normal_bias_to = Some(P)`): com `d = normalize(P − p)` (ou `p − P` em
/// [`NormalBiasDir::AwayFromTarget`]), a direção é espelhada para o hemisfério
/// da normal do cartão (`d ← −d` se `n·d < 0`, porque o cartão é de dois lados)
/// e `n' = normalize(lerp(n, d, amount))`. Largura zero na ponta colapsa em
/// triângulos. UV: U across (0..1), V da raiz (0) à ponta (1), estritamente
/// crescente; `uv_atlas_cell` mapeia `(u, v) → ((col + u)/cols, (row + v)/rows)`.
/// Curvas cíclicas não são suportadas.
pub fn hair_card(curve: &Curve, params: &HairCardParams) -> ModelResult<EditMesh> {
    curve.validate()?;
    if curve.cyclic {
        return Err(ModelError::UnsupportedTopology("hair card não suporta curva cíclica".into()));
    }
    check_pos(params.width, "width")?;
    if !params.taper.is_finite() || params.taper < 0.0 {
        return Err(ModelError::InvalidParameter("taper deve ser finito e >= 0".into()));
    }
    check_pos(params.taper_power, "taper_power")?;
    if params.segments == 0 || params.segments > 100_000 {
        return Err(ModelError::InvalidParameter("segments fora de 1..=100000".into()));
    }
    if params.width_segments == 0 || params.width_segments > 256 {
        return Err(ModelError::InvalidParameter("width_segments fora de 1..=256".into()));
    }
    if !params.curl.is_finite() || params.curl.abs() > TAU - 1e-3 {
        return Err(ModelError::InvalidParameter("curl deve estar em (−2π, 2π)".into()));
    }
    if !params.flare.is_finite() || params.flare < -1.0 {
        return Err(ModelError::InvalidParameter("flare deve ser finito e >= −1".into()));
    }
    if let Some(p) = params.normal_bias_to {
        if !p.is_finite() {
            return Err(ModelError::InvalidParameter("normal_bias_to não finito".into()));
        }
    }
    if !params.normal_bias_amount.is_finite() || !(0.0..=1.0).contains(&params.normal_bias_amount) {
        return Err(ModelError::InvalidParameter("normal_bias_amount fora de [0, 1]".into()));
    }
    if let Some(c) = params.uv_atlas_cell {
        if c.cols == 0 || c.rows == 0 || c.col >= c.cols || c.row >= c.rows {
            return Err(ModelError::InvalidParameter("célula de atlas inválida".into()));
        }
    }
    let rings = params.segments as usize + 1;
    let (samples, frames) = curve.frames_uniform(rings, params.up_hint)?;
    let curled = params.curl.abs() > 1.0e-6;
    let m = if curled {
        params.width_segments.max(2) as usize
    } else {
        params.width_segments as usize
    };
    let remap = |u: f32, v: f32| -> Vec2 {
        match params.uv_atlas_cell {
            Some(c) => Vec2::new((c.col as f32 + u) / c.cols as f32, (c.row as f32 + v) / c.rows as f32),
            None => Vec2::new(u, v),
        }
    };
    let mut mesh = EditMesh::new("HairCard");
    let mut ids: Vec<Vec<u32>> = Vec::with_capacity(rings);
    let mut normals: Vec<Vec<Vec3>> = Vec::with_capacity(rings);
    let mut apex = vec![false; rings];
    for r in 0..rings {
        let t = r as f32 / (rings - 1) as f32;
        let taper = 1.0 + (params.taper - 1.0) * t.powf(params.taper_power);
        let w = params.width * taper * (1.0 + params.flare * (PI * t).sin()) * samples[r].radius;
        let f = &frames[r];
        if w <= APEX_EPS && r == rings - 1 {
            apex[r] = true;
            let v = mesh.add_vert(f.pos);
            ids.push(vec![v; m + 1]);
            normals.push(vec![f.normal; m + 1]);
            continue;
        }
        let w = w.max(1.0e-6);
        let mut row_ids = Vec::with_capacity(m + 1);
        let mut row_n = Vec::with_capacity(m + 1);
        for k in 0..=m {
            let x = -1.0 + 2.0 * k as f32 / m as f32;
            let (pos, n) = if curled {
                let phi = params.curl * x * 0.5;
                let radius = w / params.curl;
                (
                    f.pos + f.binormal * (radius * phi.sin()) + f.normal * (radius * (phi.cos() - 1.0)),
                    f.normal * phi.cos() + f.binormal * phi.sin(),
                )
            } else {
                (f.pos + f.binormal * (0.5 * w * x), f.normal)
            };
            row_ids.push(mesh.add_vert(pos));
            row_n.push(n.normalize_or_zero());
        }
        ids.push(row_ids);
        normals.push(row_n);
    }
    // normais finais (com viés opcional)
    let final_normal = |mesh: &EditMesh, r: usize, k: usize| -> Vec3 {
        let n = normals[r][k];
        match params.normal_bias_to {
            None => n,
            Some(target) => {
                let p = mesh.co(ids[r][k]);
                let d = match params.normal_bias_dir {
                    NormalBiasDir::TowardTarget => target - p,
                    NormalBiasDir::AwayFromTarget => p - target,
                };
                let Some(mut d) = d.try_normalize() else {
                    return n;
                };
                if n.dot(d) < 0.0 {
                    d = -d;
                }
                (n * (1.0 - params.normal_bias_amount) + d * params.normal_bias_amount)
                    .try_normalize()
                    .unwrap_or(n)
            }
        }
    };
    for r in 0..rings - 1 {
        let (va, vb) = (r as f32 / (rings - 1) as f32, (r + 1) as f32 / (rings - 1) as f32);
        for k in 0..m {
            let (u0, u1) = (k as f32 / m as f32, (k + 1) as f32 / m as f32);
            let um = 0.5 * (u0 + u1);
            let ua = if apex[r] { (um, um) } else { (u0, u1) };
            let ub = if apex[r + 1] { (um, um) } else { (u0, u1) };
            let spec = [(r, k, ua.0, va), (r, k + 1, ua.1, va), (r + 1, k + 1, ub.1, vb), (r + 1, k, ub.0, vb)];
            let corners: Vec<(u32, Vec2, Option<Vec3>)> = spec
                .iter()
                .map(|&(rr, kk, u, v)| (ids[rr][kk], remap(u, v), Some(final_normal(&mesh, rr, kk))))
                .collect();
            if let Some(face) = dedup_face(&corners, true) {
                mesh.push_face(face)?;
            }
        }
    }
    mesh.validate()?;
    Ok(mesh)
}

// =============================================================================
// Mecha procedural
// =============================================================================

/// Parâmetros de [`hair_clump`].
#[derive(Debug, Clone, Copy)]
pub struct ClumpParams {
    /// Número de fios (1..=100000).
    pub count: u32,
    /// Raio da mecha na raiz.
    pub spread: f32,
    /// Raio da mecha na ponta.
    pub tip_spread: f32,
    /// Ondulação por fio (fração do raio local).
    pub noise: f32,
    /// Giro total da mecha ao longo do comprimento (radianos).
    pub twist: f32,
    /// Pontos de controle por fio (>= 2).
    pub points: u32,
    /// Variação de comprimento: cada fio termina em `1 − variation·rand` (0..1).
    pub length_variation: f32,
    pub seed: u64,
}

impl Default for ClumpParams {
    fn default() -> Self {
        Self {
            count: 8,
            spread: 0.05,
            tip_spread: 0.01,
            noise: 0.2,
            twist: 0.0,
            points: 12,
            length_variation: 0.1,
            seed: 1,
        }
    }
}

struct SplitMix(u64);

impl SplitMix {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniforme em `[0, 1)`.
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// Gera `count` fios Catmull-Rom derivados da curva guia (mecha).
///
/// Para o fio `i`, com sorteio determinístico (SplitMix64 a partir de `seed`):
/// ângulo `α ∈ [0, 2π)`, raio relativo `ρ = √u` (disco uniforme), comprimento
/// `L_i = L·(1 − length_variation·u)`, frequências/fases de ondulação. Em
/// `σ = s/L ∈ [0, L_i/L]`:
///
/// `p = guia(s) + (N cos a + B sin a)·ρ·e(σ) + (N·sin(2π f₁σ + φ₁) + B·cos(2π f₂σ + φ₂))·noise·e(σ)`
///
/// com `a = α + twist·σ`, `e(σ) = lerp(spread, tip_spread, σ)` e `(N, B)` o frame
/// de Bishop da guia interpolado. Tilt e raio do fio são os da guia. Mesma
/// semente → mesma saída. A guia não pode ser cíclica.
pub fn hair_clump(guide: &Curve, params: &ClumpParams) -> ModelResult<Vec<Curve>> {
    guide.validate()?;
    if guide.cyclic {
        return Err(ModelError::UnsupportedTopology("guia cíclica não suportada".into()));
    }
    if params.count == 0 || params.count > 100_000 {
        return Err(ModelError::InvalidParameter("count fora de 1..=100000".into()));
    }
    if params.points < 2 || params.points > 10_000 {
        return Err(ModelError::InvalidParameter("points fora de 2..=10000".into()));
    }
    for (v, what) in [
        (params.spread, "spread"),
        (params.tip_spread, "tip_spread"),
        (params.noise, "noise"),
    ] {
        if !v.is_finite() || v < 0.0 {
            return Err(ModelError::InvalidParameter(format!("{what} deve ser finito e >= 0")));
        }
    }
    if !params.twist.is_finite() {
        return Err(ModelError::InvalidParameter("twist não finito".into()));
    }
    if !params.length_variation.is_finite() || !(0.0..1.0).contains(&params.length_variation) {
        return Err(ModelError::InvalidParameter("length_variation fora de [0, 1)".into()));
    }
    let dense = (params.points as usize * 2).max(32);
    let (dsamples, dframes) = guide.frames_uniform(dense, None)?;
    let total = dsamples.last().map(|s| s.s).unwrap_or(0.0);
    let segs = guide.segments();
    let table = guide.arc_table(&segs);
    let frame_at = |s: f32| -> (Vec3, Vec3) {
        let x = (s / total).clamp(0.0, 1.0) * (dense - 1) as f32;
        let i = (x.floor() as usize).min(dense - 2);
        let f = x - i as f32;
        let n = (dframes[i].normal * (1.0 - f) + dframes[i + 1].normal * f)
            .try_normalize()
            .unwrap_or(dframes[i].normal);
        let b = (dframes[i].binormal * (1.0 - f) + dframes[i + 1].binormal * f)
            .try_normalize()
            .unwrap_or(dframes[i].binormal);
        (n, b)
    };
    let mut rng = SplitMix(params.seed ^ 0xC1D2_E3F4_0A1B_2C3D);
    let mut out = Vec::with_capacity(params.count as usize);
    for _ in 0..params.count {
        let alpha = rng.next_f32() * TAU;
        let rho = rng.next_f32().sqrt();
        let len_frac = 1.0 - params.length_variation * rng.next_f32();
        let (f1, f2) = (1.0 + 2.0 * rng.next_f32(), 1.0 + 2.0 * rng.next_f32());
        let (ph1, ph2) = (rng.next_f32() * TAU, rng.next_f32() * TAU);
        let mut pts = Vec::with_capacity(params.points as usize);
        for j in 0..params.points {
            let sigma = len_frac * j as f32 / (params.points - 1) as f32;
            let s = sigma * total;
            let t = table.t_at(s);
            let (seg, u) = guide.locate(t);
            let g = guide.sample_in(&segs[seg], seg, u, s);
            let (n, b) = frame_at(s);
            let e = params.spread + (params.tip_spread - params.spread) * sigma;
            let a = alpha + params.twist * sigma;
            let wob = n * (TAU * f1 * sigma + ph1).sin() + b * (TAU * f2 * sigma + ph2).cos();
            let pos = g.pos + (n * a.cos() + b * a.sin()) * (rho * e) + wob * (params.noise * e);
            pts.push(CurvePoint {
                pos,
                tilt: g.tilt,
                radius: g.radius,
                handle_in: None,
                handle_out: None,
            });
        }
        out.push(Curve::new(CurveKind::CatmullRom, pts, false));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::Topology;

    fn signed_volume(mesh: &EditMesh) -> f32 {
        let mut v = 0.0;
        for f in 0..mesh.face_count() as u32 {
            let p = mesh.face_positions(f);
            for k in 1..p.len().saturating_sub(1) {
                v += p[0].dot(p[k].cross(p[k + 1])) / 6.0;
            }
        }
        v
    }

    fn line(n: usize, len: f32) -> Curve {
        let pts: Vec<Vec3> = (0..n).map(|i| Vec3::new(0.0, 0.0, len * i as f32 / (n - 1) as f32)).collect();
        Curve::from_positions(CurveKind::CatmullRom, &pts, false)
    }

    fn helix(turns: f32, n: usize, r: f32, rise: f32) -> Curve {
        let pts: Vec<Vec3> = (0..n)
            .map(|i| {
                let a = TAU * turns * i as f32 / (n - 1) as f32;
                Vec3::new(r * a.cos(), rise * a / TAU, r * a.sin())
            })
            .collect();
        Curve::from_positions(CurveKind::CatmullRom, &pts, false)
    }

    fn assert_closed_outward(m: &EditMesh) {
        let t = Topology::build(m);
        assert!(t.is_closed_manifold(), "{} não é fechado e manifold", m.name);
        m.validate().unwrap();
        assert!(signed_volume(m) > 0.0, "{} orientado para dentro", m.name);
    }

    #[test]
    fn arc_length_of_catmull_circle_matches_analytic() {
        for kind in [CurveKind::CatmullRom, CurveKind::Bezier] {
            let c = Curve::circle_xz(2.0, 24, kind);
            let l = c.length();
            let exact = TAU * 2.0;
            assert!((l - exact).abs() / exact < 0.005, "{kind:?}: {l} vs {exact}");
        }
        // polilinha: perímetro do polígono regular
        let poly = Curve::circle_xz(1.0, 8, CurveKind::Polyline);
        let side = 2.0 * (PI / 8.0).sin();
        assert!((poly.length() - 8.0 * side).abs() < 1e-4);
        // reta
        assert!((line(5, 3.0).length() - 3.0).abs() < 1e-4);
        assert_eq!(Curve::from_positions(CurveKind::Polyline, &[Vec3::ZERO], false).length(), 0.0);
    }

    #[test]
    fn sample_endpoints_tangent_and_interpolation() {
        let mut c = line(4, 3.0);
        c.points[0].radius = 0.0;
        c.points[3].radius = 2.0;
        c.points[0].tilt = 0.0;
        c.points[3].tilt = 1.0;
        let a = c.sample(0.0);
        let b = c.sample(1.0);
        assert!((a.pos - Vec3::ZERO).length() < 1e-6);
        assert!((b.pos - Vec3::new(0.0, 0.0, 3.0)).length() < 1e-5);
        assert!((a.tangent - Vec3::Z).length() < 1e-5);
        assert!((b.radius - 2.0).abs() < 1e-6 && (b.tilt - 1.0).abs() < 1e-6);
        let mid = c.sample(0.5);
        assert!((mid.radius - 1.0).abs() < 1e-5 && (mid.t - 0.5).abs() < 1e-6);
        // fora do intervalo / NaN
        assert_eq!(c.sample(-3.0).pos, a.pos);
        assert_eq!(c.sample(f32::NAN).pos, a.pos);
        // fechada: t = 1 volta ao início
        let circ = Curve::circle_xz(1.0, 12, CurveKind::CatmullRom);
        assert!((circ.sample(1.0).pos - circ.sample(0.0).pos).length() < 1e-5);
        assert!((circ.sample(0.25).pos - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-3);
        // curvas degeneradas não entram em pânico
        let empty = Curve::new(CurveKind::Bezier, vec![], false);
        assert_eq!(empty.sample(0.5).pos, Vec3::ZERO);
        assert_eq!(empty.segment_count(), 0);
    }

    #[test]
    fn bezier_explicit_handles_and_auto_equal_catmull() {
        let pts = [Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 0.0)];
        let cr = Curve::from_positions(CurveKind::CatmullRom, &pts, false);
        let bz = Curve::from_positions(CurveKind::Bezier, &pts, false);
        // segmento interior idêntico; pontas diferem (reflexão vs 1/3)
        for t in [0.4, 0.5, 0.6] {
            assert!((cr.sample(t).pos - bz.sample(t).pos).length() < 1e-5);
        }
        // alças explícitas viram segmento exato
        let mut c = Curve::from_positions(CurveKind::Bezier, &[Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0)], false);
        c.points[0].handle_out = Some(Vec3::new(1.0, 2.0, 0.0));
        c.points[1].handle_in = Some(Vec3::new(2.0, 2.0, 0.0));
        let mid = c.sample(0.5).pos; // (b0 + 3b1 + 3b2 + b3)/8
        let want = (Vec3::ZERO + Vec3::new(1.0, 2.0, 0.0) * 3.0 + Vec3::new(2.0, 2.0, 0.0) * 3.0 + Vec3::new(3.0, 0.0, 0.0)) / 8.0;
        assert!((mid - want).length() < 1e-5);
        // alças coincidentes (derivada zero): tangente continua finita
        let mut z = Curve::from_positions(CurveKind::Bezier, &[Vec3::ZERO, Vec3::X], false);
        z.points[0].handle_out = Some(Vec3::ZERO);
        z.points[1].handle_in = Some(Vec3::X);
        assert!(z.sample(0.0).tangent.is_finite());
        assert!((z.sample(0.0).tangent.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sample_uniform_is_equidistant() {
        let c = Curve::circle_xz(1.5, 20, CurveKind::CatmullRom);
        let n = 64;
        let s = c.sample_uniform(n).unwrap();
        assert_eq!(s.len(), n);
        let chords: Vec<f32> = (0..n).map(|i| s[i].pos.distance(s[(i + 1) % n].pos)).collect();
        let mean = chords.iter().sum::<f32>() / n as f32;
        for ch in &chords {
            assert!((ch - mean).abs() / mean < 0.01, "{ch} vs {mean}");
        }
        assert!((s[0].s).abs() < 1e-6);
        assert!(s.windows(2).all(|w| w[1].s > w[0].s));
        // aberta inclui as duas pontas
        let l = line(6, 5.0);
        let u = l.sample_uniform(11).unwrap();
        assert!((u[0].pos - l.points[0].pos).length() < 1e-5);
        assert!((u[10].pos - l.points[5].pos).length() < 1e-4);
        assert!((u[10].s - 5.0).abs() < 1e-3);
        // erros
        assert!(l.sample_uniform(1).is_err());
        assert!(c.sample_uniform(2).is_err());
        let zero = Curve::from_positions(CurveKind::Polyline, &[Vec3::ONE, Vec3::ONE], false);
        assert!(matches!(zero.sample_uniform(4), Err(ModelError::Numeric(_))));
        let one = Curve::from_positions(CurveKind::Polyline, &[Vec3::ONE], false);
        assert!(one.sample_uniform(4).is_err());
        let nan = Curve::from_positions(CurveKind::Polyline, &[Vec3::ONE, Vec3::NAN], false);
        assert!(nan.validate().is_err());
        let mut neg = line(3, 1.0);
        neg.points[1].radius = -1.0;
        assert!(neg.validate().is_err());
        let cyc2 = Curve::from_positions(CurveKind::Polyline, &[Vec3::ZERO, Vec3::X], true);
        assert!(cyc2.validate().is_err());
    }

    #[test]
    fn bishop_frames_have_no_twist_on_helix() {
        let c = helix(3.0, 90, 1.0, 1.0);
        let (samples, frames) = c.frames_uniform(400, None).unwrap();
        let mut total_twist = 0.0f32;
        let mut worst = 0.0f32;
        let mut frenet_rate = 0.0f32;
        for i in 0..frames.len() {
            let f = &frames[i];
            // ortonormalidade e mão direita
            assert!((f.normal.length() - 1.0).abs() < 1e-4);
            assert!(f.normal.dot(f.tangent).abs() < 1e-3);
            assert!((f.normal.cross(f.binormal) - f.tangent).length() < 1e-3);
        }
        for i in 0..frames.len() - 1 {
            let ds = samples[i + 1].s - samples[i].s;
            let (a, b) = (&frames[i], &frames[i + 1]);
            // taxa de giro de N em torno de T: (dN · B)/ds
            let rate = (b.normal - a.normal).dot((a.binormal + b.binormal) * 0.5) / ds;
            worst = worst.max(rate.abs());
            total_twist += rate * ds;
            // Frenet (aprox.): componente de dN/ds ao longo de B da curva real
            let kappa_dir = (b.tangent - a.tangent) / ds;
            frenet_rate = frenet_rate.max(kappa_dir.length());
        }
        assert!(worst < 4e-3, "taxa de torção {worst}");
        assert!(total_twist.abs() < 0.05, "torção acumulada {total_twist}");
        // a hélice tem curvatura e torção reais (frames de Frenet girariam ~0.5 rad/unid.)
        assert!(frenet_rate > 0.5);
        // o frame final não girou ao redor da tangente em relação ao transportado
        // (comparação direta: transporta frame 0 até o fim e compara com o último)
        let mut n = frames[0].normal;
        for i in 1..frames.len() {
            n = transport(frames[i - 1].pos, frames[i - 1].tangent, n, frames[i].pos, frames[i].tangent);
        }
        assert!(n.dot(frames[frames.len() - 1].normal) > 0.999);
    }

    #[test]
    fn closed_frames_have_no_seam_after_holonomy_fix() {
        // laço fechado não planar: holonomia != 0
        // nó trifólio: a tangente varre uma área grande na esfera => holonomia ~ −2.2 rad
        let pts: Vec<Vec3> = (0..90)
            .map(|i| {
                let a = TAU * i as f32 / 90.0;
                Vec3::new(a.sin() + 2.0 * (2.0 * a).sin(), a.cos() - 2.0 * (2.0 * a).cos(), -(3.0 * a).sin()) * 0.3
            })
            .collect();
        let c = Curve::from_positions(CurveKind::CatmullRom, &pts, true);
        let samples = c.sample_uniform(600).unwrap();
        let pos: Vec<Vec3> = samples.iter().map(|s| s.pos).collect();
        let tan: Vec<Vec3> = samples.iter().map(|s| s.tangent).collect();
        let open = parallel_transport_frames(&pos, &tan, false, None);
        let closed = parallel_transport_frames(&pos, &tan, true, None);
        let seam = |fr: &[Frame]| {
            let n = fr.len();
            let t = transport(fr[n - 1].pos, fr[n - 1].tangent, fr[n - 1].normal, fr[0].pos, fr[0].tangent);
            t.dot(fr[0].normal)
        };
        assert!(seam(&open) < 0.995, "o laço deveria ter holonomia ({})", seam(&open));
        assert!(seam(&closed) > 0.999, "costura após correção: {}", seam(&closed));
        for f in &closed {
            assert!((f.normal.cross(f.binormal) - f.tangent).length() < 1e-3);
        }
        // entradas inválidas
        assert!(parallel_transport_frames(&[], &[], false, None).is_empty());
        assert!(parallel_transport_frames(&pos, &tan[..3], false, None).is_empty());
        // dica paralela à tangente -> perpendicular estável
        let f = parallel_transport_frames(&[Vec3::ZERO, Vec3::Z], &[Vec3::Z, Vec3::Z], false, Some(Vec3::Z));
        assert!(f[0].normal.dot(Vec3::Z).abs() < 1e-6);
        let g = parallel_transport_frames(&[Vec3::ZERO, Vec3::Z], &[Vec3::Z, Vec3::Z], false, Some(Vec3::X));
        assert!((g[0].normal - Vec3::X).length() < 1e-6);
        // pontos coincidentes não geram NaN
        let h = parallel_transport_frames(&[Vec3::ZERO, Vec3::ZERO], &[Vec3::Z, Vec3::X], false, None);
        assert!(h.iter().all(|f| f.normal.is_finite() && f.binormal.is_finite()));
    }

    #[test]
    fn tilt_rotates_frames() {
        let mut c = line(4, 3.0);
        for p in &mut c.points {
            p.tilt = std::f32::consts::FRAC_PI_2;
        }
        let (_, f0) = line(4, 3.0).frames_uniform(5, Some(Vec3::X)).unwrap();
        let (_, f1) = c.frames_uniform(5, Some(Vec3::X)).unwrap();
        assert!((f0[0].normal - Vec3::X).length() < 1e-5);
        // N gira para B em 90 graus
        assert!((f1[2].normal - f0[2].binormal).length() < 1e-4);
    }

    #[test]
    fn tube_closed_manifold_with_caps_and_volume() {
        let c = line(5, 4.0);
        let t = curve_to_tube(
            &c,
            &TubeParams {
                sides: 64,
                rings: 9,
                radius: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_closed_outward(&t);
        assert_eq!(Topology::build(&t).euler_characteristic(&t), 2);
        assert_eq!(t.face_count(), 64 * 8 + 2);
        let area = 0.5 * 64.0 * 0.25 * (TAU / 64.0).sin();
        assert!((signed_volume(&t) - area * 4.0).abs() / (area * 4.0) < 0.01);
        // sem tampas: fronteira de duas bordas
        let open = curve_to_tube(&c, &TubeParams { cap_start: false, cap_end: false, ..Default::default() }).unwrap();
        assert_eq!(Topology::build(&open).boundary_edge_count(), 16);
        // tampa só no fim
        let one = curve_to_tube(&c, &TubeParams { cap_start: false, ..Default::default() }).unwrap();
        assert_eq!(Topology::build(&one).boundary_edge_count(), 8);
        // sólido curvo
        let h = helix(1.5, 40, 2.0, 1.0);
        let ht = curve_to_tube(&h, &TubeParams { sides: 12, rings: 120, radius: 0.1, ..Default::default() }).unwrap();
        assert_closed_outward(&ht);
    }

    #[test]
    fn tube_taper_apex_cyclic_and_uvs() {
        let c = line(5, 2.0);
        // ponta com raio 0 -> apex cônico, ainda fechado
        let cone = curve_to_tube(
            &c,
            &TubeParams {
                sides: 8,
                rings: 6,
                radius: 0.3,
                radius_profile: Taper::new(1.0, 0.0, 1.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_closed_outward(&cone);
        assert_eq!(Topology::build(&cone).euler_characteristic(&cone), 2);
        assert_eq!(cone.vert_count(), 8 * 5 + 1);
        // apex nas duas pontas
        let spindle = curve_to_tube(
            &c,
            &TubeParams {
                sides: 8,
                rings: 7,
                radius: 0.3,
                radius_profile: Taper::new(0.0, 0.0, 1.0),
                use_point_radius: false,
                ..Default::default()
            },
        );
        // perfil 0 em todo o comprimento: interior limitado a 1e-6 -> válido mas degenerado
        assert!(spindle.is_ok());
        // toro
        let circ = Curve::circle_xz(2.0, 16, CurveKind::CatmullRom);
        let torus = curve_to_tube(&circ, &TubeParams { sides: 24, rings: 64, radius: 0.3, ..Default::default() }).unwrap();
        assert_closed_outward(&torus);
        assert_eq!(Topology::build(&torus).euler_characteristic(&torus), 0);
        // volume do toro ~ 2π²Rr²
        let exact = 2.0 * PI * PI * 2.0 * 0.09;
        assert!((signed_volume(&torus) - exact).abs() / exact < 0.03);
        // UV: V monotônico ao longo, U em [0, 1], costura marcada
        let t = curve_to_tube(&c, &TubeParams { sides: 8, rings: 6, radius: 0.2, ..Default::default() }).unwrap();
        let mut vs = Vec::new();
        for f in &t.faces[..8 * 5] {
            for cn in &f.corners {
                assert!((0.0..=1.0 + 1e-6).contains(&cn.uv.x) && (0.0..=1.0 + 1e-6).contains(&cn.uv.y));
            }
            vs.push(f.corners.iter().map(|c| c.uv.y).fold(f32::INFINITY, f32::min));
        }
        assert!(vs.chunks(8).map(|c| c[0]).collect::<Vec<_>>().windows(2).all(|w| w[1] > w[0]));
        assert!(t.edge_attrs.values().any(|e| e.seam));
        // escala de V por unidade
        let scaled = curve_to_tube(&c, &TubeParams { uv_v_per_unit: Some(2.0), rings: 5, ..Default::default() }).unwrap();
        let vmax = scaled.faces.iter().flat_map(|f| f.corners.iter()).map(|c| c.uv.y).fold(0.0f32, f32::max);
        assert!((vmax - 4.0).abs() < 1e-3);
    }

    #[test]
    fn tube_errors() {
        let c = line(3, 1.0);
        let p = TubeParams::default();
        assert!(curve_to_tube(&c, &TubeParams { sides: 2, ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { sides: 1000, ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { rings: 1, ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { radius: 0.0, ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { radius: f32::NAN, ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { radius_profile: Taper::new(1.0, 1.0, 0.0), ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { radius_profile: Taper::new(-1.0, 1.0, 1.0), ..p }).is_err());
        assert!(curve_to_tube(&c, &TubeParams { uv_v_per_unit: Some(0.0), ..p }).is_err());
        let circ = Curve::circle_xz(1.0, 8, CurveKind::Polyline);
        assert!(curve_to_tube(&circ, &TubeParams { rings: 2, ..p }).is_err());
        let zero = Curve::from_positions(CurveKind::Polyline, &[Vec3::ONE, Vec3::ONE], false);
        assert!(curve_to_tube(&zero, &p).is_err());
        assert!(curve_to_tube(&Curve::new(CurveKind::Bezier, vec![], false), &p).is_err());
    }

    #[test]
    fn ribbon_area_orientation_twist() {
        let c = line(4, 4.0);
        let r = curve_to_ribbon(
            &c,
            &RibbonParams {
                rings: 9,
                width: 2.0,
                normal_up: Some(Vec3::Y),
                ..Default::default()
            },
        )
        .unwrap();
        r.validate().unwrap();
        let area: f32 = (0..r.face_count() as u32).map(|f| r.face_area(f)).sum();
        assert!((area - 8.0).abs() < 1e-3);
        for f in 0..r.face_count() as u32 {
            assert!((r.face_normal(f) - Vec3::Y).length() < 1e-4);
        }
        // largura ao longo de B
        assert!(r.verts.iter().all(|v| v.co.y.abs() < 1e-5 && v.co.x.abs() <= 1.0 + 1e-5));
        // twist de pi: a normal final inverte
        let tw = curve_to_ribbon(
            &c,
            &RibbonParams {
                rings: 33,
                width: 2.0,
                twist: PI,
                normal_up: Some(Vec3::Y),
                ..Default::default()
            },
        )
        .unwrap();
        let first = tw.face_normal(0);
        let last = tw.face_normal(tw.face_count() as u32 - 1);
        assert!(first.dot(last) < -0.9, "{first:?} {last:?}");
        // largura segmentada e UV
        let ws = curve_to_ribbon(&c, &RibbonParams { width_segments: 4, rings: 5, ..Default::default() }).unwrap();
        assert_eq!(ws.face_count(), 4 * 4);
        let us: Vec<f32> = ws.faces[0].corners.iter().map(|c| c.uv.x).collect();
        assert!((us.iter().cloned().fold(0.0, f32::max) - 0.25).abs() < 1e-6);
        // ponta em bico
        let tip = curve_to_ribbon(
            &c,
            &RibbonParams {
                rings: 5,
                width_profile: Taper::new(1.0, 0.0, 1.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tip.faces.last().unwrap().len(), 3);
        // faixa cíclica
        let circ = Curve::circle_xz(1.0, 12, CurveKind::CatmullRom);
        let band = curve_to_ribbon(&circ, &RibbonParams { rings: 24, width: 0.2, ..Default::default() }).unwrap();
        assert_eq!(band.face_count(), 24);
        assert_eq!(Topology::build(&band).boundary_edge_count(), 48);
        // erros
        assert!(curve_to_ribbon(&c, &RibbonParams { width: 0.0, ..Default::default() }).is_err());
        assert!(curve_to_ribbon(&c, &RibbonParams { rings: 1, ..Default::default() }).is_err());
        assert!(curve_to_ribbon(&c, &RibbonParams { width_segments: 0, ..Default::default() }).is_err());
        assert!(curve_to_ribbon(&c, &RibbonParams { twist: f32::NAN, ..Default::default() }).is_err());
    }

    fn arch() -> Curve {
        // arco de cabelo saindo do topo da cabeça (origem) para baixo
        let pts: Vec<Vec3> = (0..8)
            .map(|i| {
                let a = 0.2 + i as f32 * 0.25;
                Vec3::new(a.sin() * 1.2, a.cos() * 1.2, 0.0)
            })
            .collect();
        Curve::from_positions(CurveKind::CatmullRom, &pts, false)
    }

    #[test]
    fn hair_card_uv_monotonic_and_normals_defined() {
        let c = arch();
        let p = HairCardParams {
            width: 0.2,
            taper: 0.3,
            segments: 10,
            width_segments: 2,
            curl: 1.0,
            flare: 0.4,
            normal_bias_to: Some(Vec3::ZERO),
            normal_bias_amount: 0.7,
            ..Default::default()
        };
        let m = hair_card(&c, &p).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 10 * 2);
        // normais definidas, unitárias e do lado da face
        for (fi, f) in m.faces.iter().enumerate() {
            let fnorm = m.face_normal(fi as u32);
            for cn in &f.corners {
                let n = cn.normal.expect("normal definida");
                assert!((n.length() - 1.0).abs() < 1e-4);
                assert!(n.dot(fnorm) > 0.0, "normal virada para trás");
            }
        }
        // V estritamente crescente por anel; faces em ordem de anel
        let mut prev_v = -1.0f32;
        for r in 0..10 {
            let f = &m.faces[r * 2];
            let v0 = f.corners.iter().map(|c| c.uv.y).fold(f32::INFINITY, f32::min);
            let v1 = f.corners.iter().map(|c| c.uv.y).fold(f32::NEG_INFINITY, f32::max);
            assert!(v0 > prev_v - 1e-6 && v1 > v0);
            prev_v = v1;
        }
        assert!((prev_v - 1.0).abs() < 1e-6);
        // largura da raiz > ponta (taper)
        let root_w = m.co(m.faces[0].corners[0].vert).distance(m.co(m.faces[1].corners[2].vert));
        let tip_f = m.faces.last().unwrap();
        let tip_w = m.co(tip_f.corners[2].vert).distance(m.co(tip_f.corners[3].vert));
        assert!(tip_w < root_w);
    }

    #[test]
    fn hair_card_bias_points_to_target_and_flat_card_defaults() {
        let c = arch();
        // sem viés e plano: todas as normais iguais à normal da face
        let flat = hair_card(&c, &HairCardParams { taper: 1.0, ..Default::default() }).unwrap();
        for (fi, f) in flat.faces.iter().enumerate() {
            for cn in &f.corners {
                assert!((cn.normal.unwrap() - flat.face_normal(fi as u32)).length() < 1e-3);
            }
        }
        // viés 1 em direção ao centro: normal = ±direção ao alvo
        let toward = hair_card(
            &c,
            &HairCardParams {
                taper: 1.0,
                normal_bias_to: Some(Vec3::ZERO),
                normal_bias_amount: 1.0,
                up_hint: Some(Vec3::Z),
                ..Default::default()
            },
        )
        .unwrap();
        for f in &toward.faces {
            for cn in &f.corners {
                let d = (-toward.co(cn.vert)).normalize();
                assert!(cn.normal.unwrap().dot(d).abs() > 0.999);
            }
        }
        let away = hair_card(
            &c,
            &HairCardParams {
                taper: 1.0,
                normal_bias_to: Some(Vec3::ZERO),
                normal_bias_amount: 1.0,
                normal_bias_dir: NormalBiasDir::AwayFromTarget,
                up_hint: Some(Vec3::Z),
                ..Default::default()
            },
        )
        .unwrap();
        // ambos dentro do hemisfério da face: a normal só difere por sinal global
        for (a, b) in toward.faces.iter().zip(&away.faces) {
            for (x, y) in a.corners.iter().zip(&b.corners) {
                assert!((x.normal.unwrap().dot(y.normal.unwrap()).abs() - 1.0).abs() < 1e-3);
            }
        }
        // viés 0 equivale a sem viés
        let zero = hair_card(
            &c,
            &HairCardParams { taper: 1.0, normal_bias_to: Some(Vec3::ZERO), normal_bias_amount: 0.0, ..Default::default() },
        )
        .unwrap();
        assert_eq!(zero.faces, flat.faces);
    }

    #[test]
    fn hair_card_atlas_apex_curl_geometry() {
        let c = arch();
        let cell = AtlasCell { cols: 4, rows: 2, col: 3, row: 1 };
        let m = hair_card(&c, &HairCardParams { uv_atlas_cell: Some(cell), taper: 0.5, ..Default::default() }).unwrap();
        for f in &m.faces {
            for cn in &f.corners {
                assert!(cn.uv.x >= 0.75 - 1e-6 && cn.uv.x <= 1.0 + 1e-6);
                assert!(cn.uv.y >= 0.5 - 1e-6 && cn.uv.y <= 1.0 + 1e-6);
            }
        }
        // ponta em bico: triângulo final, vértice único
        let tip = hair_card(&c, &HairCardParams { taper: 0.0, segments: 6, ..Default::default() }).unwrap();
        assert_eq!(tip.faces.last().unwrap().len(), 3);
        assert_eq!(tip.vert_count(), 6 * 2 + 1);
        // curl: seção em arco -> vértices centrais deslocados para -N, comprimento de arco = w
        let l = line(3, 2.0);
        let flat_w = 0.4;
        let curled = hair_card(
            &l,
            &HairCardParams { width: flat_w, taper: 1.0, segments: 2, width_segments: 8, curl: 2.0, up_hint: Some(Vec3::Y), ..Default::default() },
        )
        .unwrap();
        let ring: Vec<Vec3> = (0..=8).map(|k| curled.co(k as u32)).collect();
        let arc: f32 = ring.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!((arc - flat_w).abs() / flat_w < 0.02, "arco {arc}");
        // extremos abaixo do centro (face +N convexa)
        assert!(ring[0].y < ring[4].y - 1e-3 && ring[8].y < ring[4].y - 1e-3);
        // curl pequeno força >= 2 divisões
        let auto = hair_card(&l, &HairCardParams { curl: 0.5, width_segments: 1, ..Default::default() }).unwrap();
        assert_eq!(auto.face_count(), 2 * 8);
        // flare alarga o meio
        let fl = hair_card(&l, &HairCardParams { width: 0.2, taper: 1.0, segments: 2, flare: 1.0, ..Default::default() }).unwrap();
        let w_root = fl.co(0).distance(fl.co(1));
        let w_mid = fl.co(2).distance(fl.co(3));
        assert!((w_mid / w_root - 2.0).abs() < 0.01);
    }

    #[test]
    fn hair_card_errors() {
        let c = arch();
        let d = HairCardParams::default();
        assert!(hair_card(&c, &HairCardParams { width: 0.0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { taper: -0.1, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { taper_power: 0.0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { segments: 0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { width_segments: 0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { curl: f32::NAN, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { curl: 7.0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { flare: -2.0, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { normal_bias_to: Some(Vec3::NAN), ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { normal_bias_amount: 1.5, ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { uv_atlas_cell: Some(AtlasCell { cols: 0, rows: 1, col: 0, row: 0 }), ..d }).is_err());
        assert!(hair_card(&c, &HairCardParams { uv_atlas_cell: Some(AtlasCell { cols: 2, rows: 2, col: 2, row: 0 }), ..d }).is_err());
        let cyc = Curve::circle_xz(1.0, 8, CurveKind::CatmullRom);
        assert!(matches!(hair_card(&cyc, &d), Err(ModelError::UnsupportedTopology(_))));
        assert!(hair_card(&Curve::new(CurveKind::Bezier, vec![], false), &d).is_err());
    }

    #[test]
    fn hair_clump_is_deterministic_and_bounded() {
        let guide = arch();
        let p = ClumpParams { count: 20, spread: 0.1, tip_spread: 0.02, noise: 0.3, points: 10, ..Default::default() };
        let a = hair_clump(&guide, &p).unwrap();
        let b = hair_clump(&guide, &p).unwrap();
        assert_eq!(a, b);
        let other = hair_clump(&guide, &ClumpParams { seed: 99, ..p }).unwrap();
        assert_ne!(a, other);
        assert_eq!(a.len(), 20);
        let l = guide.length();
        for s in &a {
            assert_eq!(s.points.len(), 10);
            assert_eq!(s.kind, CurveKind::CatmullRom);
            s.validate().unwrap();
            // todo ponto fica perto da guia: distância <= spread*(1+noise*sqrt2)
            for pt in &s.points {
                let mut best = f32::INFINITY;
                for g in guide.sample_uniform(400).unwrap() {
                    best = best.min(g.pos.distance(pt.pos));
                }
                assert!(best <= 0.1 * (1.0 + 0.3 * 1.5) + 0.01, "distância {best}");
            }
            // comprimento não excede o da guia por muito
            assert!(s.length() < l * 1.3);
        }
        // sem espalhamento nem ruído: fios sobre a guia
        let tight = hair_clump(&guide, &ClumpParams { count: 3, spread: 0.0, tip_spread: 0.0, noise: 0.0, length_variation: 0.0, points: 6, ..Default::default() }).unwrap();
        for s in &tight {
            assert!((s.points[0].pos - guide.sample(0.0).pos).length() < 1e-4);
            assert!((s.points[5].pos - guide.points.last().unwrap().pos).length() < 1e-3);
        }
        // afunila: espalhamento na raiz maior que na ponta
        let a = hair_clump(&guide, &ClumpParams { length_variation: 0.0, ..p }).unwrap();
        let spread_at = |idx: usize| {
            let c: Vec3 = a.iter().map(|s| s.points[idx].pos).sum::<Vec3>() / a.len() as f32;
            a.iter().map(|s| s.points[idx].pos.distance(c)).sum::<f32>() / a.len() as f32
        };
        assert!(spread_at(0) > spread_at(9) * 2.0, "{} {}", spread_at(0), spread_at(9));
        // variação de comprimento: fios mais curtos
        let var = hair_clump(&guide, &ClumpParams { length_variation: 0.5, ..p }).unwrap();
        assert!(var.iter().any(|s| s.length() < l * 0.9));
    }

    #[test]
    fn hair_clump_errors() {
        let g = arch();
        let d = ClumpParams::default();
        assert!(hair_clump(&g, &ClumpParams { count: 0, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { points: 1, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { spread: -1.0, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { tip_spread: f32::NAN, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { noise: -0.1, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { twist: f32::INFINITY, ..d }).is_err());
        assert!(hair_clump(&g, &ClumpParams { length_variation: 1.0, ..d }).is_err());
        assert!(hair_clump(&Curve::circle_xz(1.0, 8, CurveKind::CatmullRom), &d).is_err());
        assert!(hair_clump(&Curve::new(CurveKind::Bezier, vec![], false), &d).is_err());
    }

    #[test]
    fn clump_to_cards_pipeline_and_serde() {
        let guide = arch();
        let strands = hair_clump(&guide, &ClumpParams { count: 5, points: 8, ..Default::default() }).unwrap();
        let mut all = EditMesh::new("hair");
        for s in &strands {
            let card = hair_card(s, &HairCardParams { segments: 4, ..Default::default() }).unwrap();
            all.append(&card);
        }
        all.validate().unwrap();
        assert_eq!(all.face_count(), 5 * 4);
        let json = serde_json::to_string(&guide).unwrap();
        let back: Curve = serde_json::from_str(&json).unwrap();
        assert_eq!(guide, back);
        let t = Taper::default();
        assert_eq!(t, Taper::NONE);
        assert!((Taper::new(1.0, 0.0, 2.0).eval(0.5) - 0.75).abs() < 1e-6);
        assert_eq!(Taper::new(1.0, 0.0, 2.0).eval(7.0), 0.0);
    }

    #[test]
    fn large_tube_sanity() {
        let h = helix(10.0, 400, 3.0, 2.0);
        let t0 = std::time::Instant::now();
        let m = curve_to_tube(&h, &TubeParams { sides: 16, rings: 3000, radius: 0.1, ..Default::default() }).unwrap();
        assert!(m.face_count() > 40_000);
        assert!(Topology::build(&m).is_closed_manifold());
        assert!(t0.elapsed().as_secs() < 60);
    }
}
