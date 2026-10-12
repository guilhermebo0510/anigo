//! Traço freehand sobre o couro cabeludo: raycast (BVH) → amostras na superfície → curva guia elevada.
//!
//! Fluxo (o mesmo do VRoid): o traço do mouse/caneta vira uma sequência de raios; cada raio que acerta a
//! malha-alvo dá uma [`SurfaceSample`]. Quando o traço sai da silhueta da cabeça (o raio erra), as
//! amostras seguem num plano paralelo à tela à profundidade do último acerto (o cabelo continua a
//! descer para fora da cabeça). [`strand_curve_from_stroke`] reamostra por comprimento de arco,
//! suaviza, reduz pontos (RDP) e eleva a curva ao longo da normal da superfície com um perfil de
//! distância `lift_root → lift_tip`.

use crate::bvh::TriBvh;
use crate::curve::{Curve, CurveKind};
use crate::error::{ModelError, ModelResult};
use glam::Vec3;

/// Ponto do traço sobre (ou logo fora de) a superfície-alvo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSample {
    pub point: Vec3,
    pub normal: Vec3,
    /// `true` se veio de um acerto de raio; `false` se foi estendido para fora da silhueta.
    pub on_surface: bool,
}

/// Acumula as amostras de um traço enquanto o ponteiro se move.
#[derive(Debug, Clone)]
pub struct StrokeAccumulator {
    samples: Vec<SurfaceSample>,
    /// Distância mínima (espaço local) entre amostras consecutivas.
    spacing: f32,
    /// Distância ao longo do raio do último acerto (para estender fora da silhueta).
    last_hit_t: f32,
}

impl StrokeAccumulator {
    pub fn new(spacing: f32) -> Self {
        Self {
            samples: Vec::new(),
            spacing: spacing.max(1e-5),
            last_hit_t: 0.0,
        }
    }

    pub fn samples(&self) -> &[SurfaceSample] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Soma um raio (espaço local do alvo, `dir` unitária). A PRIMEIRA amostra precisa acertar a
    /// superfície (a raiz fica no couro cabeludo). Devolve `true` se uma amostra foi adicionada.
    pub fn push_ray(&mut self, bvh: &TriBvh, origin: Vec3, dir: Vec3) -> bool {
        let sample = match bvh.raycast_ex(origin, dir, f32::INFINITY, true) {
            Some(h) => {
                self.last_hit_t = h.t;
                SurfaceSample {
                    point: h.point,
                    normal: h.normal,
                    on_surface: true,
                }
            }
            None => {
                let Some(last) = self.samples.last() else {
                    return false;
                };
                SurfaceSample {
                    point: origin + dir * self.last_hit_t,
                    normal: last.normal,
                    on_surface: false,
                }
            }
        };
        if let Some(last) = self.samples.last() {
            if last.point.distance(sample.point) < self.spacing {
                return false;
            }
        }
        self.samples.push(sample);
        true
    }

    /// Comprimento do traço até agora.
    pub fn path_length(&self) -> f32 {
        self.samples.windows(2).map(|w| w[0].point.distance(w[1].point)).sum()
    }
}

/// Parâmetros da conversão traço → guia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeParams {
    /// Passo da reamostragem por comprimento de arco.
    pub resample_step: f32,
    /// Iterações de suavização Laplaciana (extremos fixos).
    pub smooth_iters: u32,
    /// Tolerância do RDP: menor = mais pontos de controle.
    pub rdp_epsilon: f32,
    /// Distância à superfície na raiz (o fio nasce colado ao couro cabeludo).
    pub lift_root: f32,
    /// Distância à superfície na ponta (o cabelo se afasta da cabeça ao longo do comprimento).
    pub lift_tip: f32,
    /// Teto de pontos de controle.
    pub max_points: usize,
    /// Comprimento mínimo do traço para virar mecha.
    pub min_length: f32,
}

impl Default for StrokeParams {
    fn default() -> Self {
        Self {
            resample_step: 0.02,
            smooth_iters: 3,
            rdp_epsilon: 0.004,
            lift_root: 0.004,
            lift_tip: 0.05,
            max_points: 24,
            min_length: 0.03,
        }
    }
}

impl StrokeParams {
    pub fn validate(&self) -> ModelResult<()> {
        for (v, what) in [
            (self.resample_step, "resample_step"),
            (self.rdp_epsilon, "rdp_epsilon"),
            (self.lift_root, "lift_root"),
            (self.lift_tip, "lift_tip"),
            (self.min_length, "min_length"),
        ] {
            if !v.is_finite() || v < 0.0 {
                return Err(ModelError::InvalidParameter(format!("{what} deve ser finito e >= 0")));
            }
        }
        if self.resample_step <= 0.0 {
            return Err(ModelError::InvalidParameter("resample_step deve ser > 0".into()));
        }
        if self.max_points < 2 {
            return Err(ModelError::InvalidParameter("max_points deve ser >= 2".into()));
        }
        Ok(())
    }

    /// Distância de elevação em `s ∈ [0, 1]` (smoothstep entre raiz e ponta).
    pub fn lift_at(&self, s: f32) -> f32 {
        let s = s.clamp(0.0, 1.0);
        let k = s * s * (3.0 - 2.0 * s);
        self.lift_root + (self.lift_tip - self.lift_root) * k
    }
}

/// Resultado: guia + normal do couro cabeludo na raiz.
#[derive(Debug, Clone, PartialEq)]
pub struct StrandFromStroke {
    pub guide: Curve,
    pub root_normal: Vec3,
}

fn resample(samples: &[SurfaceSample], step: f32) -> Vec<SurfaceSample> {
    let mut cum = vec![0.0f32];
    for w in samples.windows(2) {
        cum.push(cum.last().copied().unwrap_or(0.0) + w[0].point.distance(w[1].point));
    }
    let total = cum.last().copied().unwrap_or(0.0);
    let n = ((total / step).ceil() as usize).max(2);
    let mut out = Vec::with_capacity(n + 1);
    let mut seg = 0usize;
    for i in 0..=n {
        let d = total * i as f32 / n as f32;
        while seg + 2 < cum.len() && cum[seg + 1] < d {
            seg += 1;
        }
        let (a, b) = (&samples[seg], &samples[seg + 1]);
        let span = (cum[seg + 1] - cum[seg]).max(1e-12);
        let u = ((d - cum[seg]) / span).clamp(0.0, 1.0);
        out.push(SurfaceSample {
            point: a.point.lerp(b.point, u),
            normal: a.normal.lerp(b.normal, u).try_normalize().unwrap_or(a.normal),
            on_surface: a.on_surface && b.on_surface,
        });
    }
    out
}

/// Índices mantidos por Ramer–Douglas–Peucker (sempre inclui o primeiro e o último).
pub fn rdp_indices(pts: &[Vec3], eps: f32) -> Vec<usize> {
    let n = pts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let ab = pts[b] - pts[a];
        let len2 = ab.length_squared();
        let (mut worst, mut worst_d) = (a, -1.0f32);
        for i in a + 1..b {
            let d = if len2 < 1e-20 {
                pts[i].distance(pts[a])
            } else {
                let t = ((pts[i] - pts[a]).dot(ab) / len2).clamp(0.0, 1.0);
                pts[i].distance(pts[a] + ab * t)
            };
            if d > worst_d {
                worst_d = d;
                worst = i;
            }
        }
        if worst_d > eps {
            keep[worst] = true;
            stack.push((a, worst));
            stack.push((worst, b));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// Converte as amostras do traço em guia. A direção do traço define raiz → ponta.
///
/// `scalp` (opcional) recoloca os pontos suavizados que estavam na superfície de volta nela (a
/// suavização puxa para dentro em superfícies convexas); a elevação vem DEPOIS, ao longo da normal.
pub fn strand_curve_from_stroke(samples: &[SurfaceSample], params: &StrokeParams, scalp: Option<&TriBvh>) -> ModelResult<StrandFromStroke> {
    params.validate()?;
    if samples.len() < 2 {
        return Err(ModelError::InvalidParameter("traço curto demais (menos de 2 amostras)".into()));
    }
    if samples.iter().any(|s| !s.point.is_finite() || !s.normal.is_finite()) {
        return Err(ModelError::Numeric("amostra do traço não finita".into()));
    }
    let length: f32 = samples.windows(2).map(|w| w[0].point.distance(w[1].point)).sum();
    if length < params.min_length.max(1e-6) {
        return Err(ModelError::InvalidParameter("traço curto demais".into()));
    }
    let mut pts = resample(samples, params.resample_step);
    // suavização Laplaciana com as pontas fixas
    for _ in 0..params.smooth_iters {
        let prev: Vec<Vec3> = pts.iter().map(|s| s.point).collect();
        for i in 1..pts.len() - 1 {
            pts[i].point = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
    if let Some(bvh) = scalp {
        for s in pts.iter_mut().filter(|s| s.on_surface) {
            if let Some(c) = bvh.closest_point(s.point) {
                s.point = c.point;
                s.normal = c.normal;
            }
        }
    }
    let positions: Vec<Vec3> = pts.iter().map(|s| s.point).collect();
    let mut keep = rdp_indices(&positions, params.rdp_epsilon);
    if keep.len() > params.max_points {
        // sobra de pontos: amostra uniformemente sem perder as pontas
        let m = params.max_points;
        keep = (0..m).map(|k| keep[(k * (keep.len() - 1)) / (m - 1)]).collect();
        keep.dedup();
    }
    if keep.len() < 2 {
        return Err(ModelError::Numeric("traço degenerado após simplificação".into()));
    }
    // fração de comprimento de arco de cada ponto mantido
    let mut cum = vec![0.0f32];
    for &i in keep.iter().skip(1) {
        let last = *cum.last().unwrap_or(&0.0);
        let prev = keep[cum.len() - 1];
        cum.push(last + positions[prev].distance(positions[i]));
    }
    let total = cum.last().copied().unwrap_or(0.0).max(1e-9);
    let lifted: Vec<Vec3> = keep
        .iter()
        .zip(&cum)
        .map(|(&i, &c)| {
            let n = pts[i].normal.try_normalize().unwrap_or(Vec3::Y);
            positions[i] + n * params.lift_at(c / total)
        })
        .collect();
    let root_normal = pts[keep[0]].normal.try_normalize().unwrap_or(Vec3::Y);
    Ok(StrandFromStroke {
        guide: Curve::from_positions(CurveKind::CatmullRom, &lifted, false),
        root_normal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn sphere_bvh(r: f32) -> TriBvh {
        TriBvh::build(&primitives::uv_sphere(r, 24, 48))
    }

    /// Raio vindo de fora, em direção ao centro, passando pelo ponto da esfera (lat, lon).
    fn ray_to(r: f32, lat: f32, lon: f32) -> (Vec3, Vec3) {
        let p = Vec3::new(r * lat.cos() * lon.sin(), r * lat.sin(), r * lat.cos() * lon.cos());
        let o = p * 3.0;
        (o, (p - o).normalize())
    }

    fn stroke_on_sphere(bvh: &TriBvh, r: f32) -> StrokeAccumulator {
        let mut acc = StrokeAccumulator::new(0.01);
        for k in 0..=40 {
            let lat = 1.2 - 1.0 * k as f32 / 40.0;
            let (o, d) = ray_to(r, lat, 0.5);
            acc.push_ray(bvh, o, d);
        }
        acc
    }

    #[test]
    fn stroke_projects_onto_scalp_with_constant_lift() {
        let r = 1.0;
        let bvh = sphere_bvh(r);
        let acc = stroke_on_sphere(&bvh, r);
        assert!(acc.len() > 10);
        assert!(acc.samples().iter().all(|s| s.on_surface));
        let params = StrokeParams {
            lift_root: 0.02,
            lift_tip: 0.02,
            ..StrokeParams::default()
        };
        let out = strand_curve_from_stroke(acc.samples(), &params, Some(&bvh)).unwrap();
        // pontos de controle a lift constante da superfície (erro < tolerância da malha facetada + RDP)
        for p in &out.guide.points {
            let c = bvh.closest_point(p.pos).unwrap();
            assert!((c.distance - 0.02).abs() < 0.012, "distância {} longe de 0.02", c.distance);
        }
        assert!(out.guide.points.len() >= 2 && out.guide.points.len() <= params.max_points);
        // a raiz está no começo do traço (a direção do traço manda)
        assert!(out.guide.points[0].pos.y > out.guide.points.last().unwrap().pos.y);
    }

    #[test]
    fn lift_profile_grows_from_root_to_tip() {
        let bvh = sphere_bvh(1.0);
        let acc = stroke_on_sphere(&bvh, 1.0);
        let params = StrokeParams {
            lift_root: 0.0,
            lift_tip: 0.2,
            ..StrokeParams::default()
        };
        let out = strand_curve_from_stroke(acc.samples(), &params, Some(&bvh)).unwrap();
        let d: Vec<f32> = out.guide.points.iter().map(|p| bvh.closest_point(p.pos).unwrap().distance).collect();
        assert!(d[0] < 0.02, "raiz colada ao couro cabeludo, {}", d[0]);
        assert!((d[d.len() - 1] - 0.2).abs() < 0.03, "ponta a ~0.2, {}", d[d.len() - 1]);
        assert!(d.windows(2).all(|w| w[1] + 0.01 >= w[0]), "elevação monotônica");
    }

    #[test]
    fn first_sample_must_hit_and_miss_extends_off_silhouette() {
        let bvh = sphere_bvh(1.0);
        let mut acc = StrokeAccumulator::new(0.01);
        // raio que erra: sem amostra
        assert!(!acc.push_ray(&bvh, Vec3::new(5.0, 5.0, 5.0), Vec3::new(1.0, 0.0, 0.0)));
        assert!(acc.is_empty());
        let (o, d) = ray_to(1.0, 0.3, 0.0);
        assert!(acc.push_ray(&bvh, o, d));
        // agora um raio que erra a esfera é estendido à profundidade do último acerto
        let o2 = Vec3::new(2.0, -0.2, 3.0);
        let d2 = Vec3::new(0.0, 0.0, -1.0);
        assert!(acc.push_ray(&bvh, o2, d2));
        assert!(!acc.samples()[1].on_surface);
        let off = acc.samples()[1].point - o2;
        assert!(off.cross(d2).length() < 1e-4, "a amostra estendida fica sobre o raio");
    }

    #[test]
    fn spacing_filters_dense_input_and_short_strokes_fail() {
        let bvh = sphere_bvh(1.0);
        let mut acc = StrokeAccumulator::new(0.2);
        let (o, d) = ray_to(1.0, 0.3, 0.0);
        assert!(acc.push_ray(&bvh, o, d));
        assert!(!acc.push_ray(&bvh, o, d), "mesmo ponto não duplica");
        let one = strand_curve_from_stroke(acc.samples(), &StrokeParams::default(), None);
        assert!(one.is_err());
        let a = SurfaceSample {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            on_surface: true,
        };
        let b = SurfaceSample {
            point: Vec3::new(0.001, 0.0, 0.0),
            ..a
        };
        assert!(strand_curve_from_stroke(&[a, b], &StrokeParams::default(), None).is_err());
        let nan = SurfaceSample {
            point: Vec3::new(f32::NAN, 0.0, 0.0),
            ..a
        };
        assert!(strand_curve_from_stroke(&[a, nan], &StrokeParams::default(), None).is_err());
    }

    #[test]
    fn rdp_keeps_ends_and_removes_collinear() {
        let pts: Vec<Vec3> = (0..10).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        assert_eq!(rdp_indices(&pts, 0.01), vec![0, 9]);
        let mut bent = pts.clone();
        bent[5].y = 3.0;
        let k = rdp_indices(&bent, 0.01);
        assert!(k.contains(&5) && k.first() == Some(&0) && k.last() == Some(&9));
    }

    #[test]
    fn max_points_is_respected() {
        let bvh = sphere_bvh(1.0);
        let acc = stroke_on_sphere(&bvh, 1.0);
        let params = StrokeParams {
            rdp_epsilon: 0.0,
            resample_step: 0.01,
            max_points: 6,
            ..StrokeParams::default()
        };
        let out = strand_curve_from_stroke(acc.samples(), &params, None).unwrap();
        assert!(out.guide.points.len() <= 6 && out.guide.points.len() >= 2);
    }
}
