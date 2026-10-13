//! Curvas de falloff dos pincéis (equivalentes às "Brush Falloff" do Blender) e
//! o editor de curva [`CurveProfile`] (pontos Bézier → tabela de 256 amostras).
//!
//! Convenção: `t = distância / raio` em `[0, 1]`; o resultado é o peso em
//! `[0, 1]` (1 no centro do pincel). Fora do raio (`t > 1`) o peso é sempre 0.

use crate::error::{ModelError, ModelResult};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Número de amostras da tabela de uma curva customizada.
pub const CURVE_TABLE_SIZE: usize = 256;

/// Perfil de falloff do pincel.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Falloff {
    /// `3p² − 2p³` com `p = 1 − t` (padrão do Blender).
    #[default]
    Smooth,
    /// `6p⁵ − 15p⁴ + 10p³`: derivada zero nas duas pontas.
    Smoother,
    /// `√(1 − t²)`: borda arredondada, centro largo.
    Sphere,
    /// `√p`: pico no centro, cauda longa.
    Root,
    /// `p²`: pico fino.
    Sharp,
    /// `p`.
    Linear,
    /// 1 em todo o raio (corte duro na borda).
    Constant,
    /// `p (2 − p)`.
    InverseSquare,
    /// Curva amostrada em `t ∈ [0, 1]` (interpolação linear entre amostras).
    /// Use [`CurveProfile::to_falloff`] para gerar a partir do editor.
    Custom(Vec<f32>),
}

impl Falloff {
    /// Peso para `t = dist / raio`. `t < 0` vale como 0; `t > 1` ou NaN → 0.
    /// Um `Custom` vazio devolve 0 (use [`Falloff::validate`] para detectar).
    pub fn eval(&self, t: f32) -> f32 {
        if t.is_nan() || t > 1.0 {
            return 0.0;
        }
        let t = t.max(0.0);
        let p = 1.0 - t;
        let w = match self {
            Falloff::Smooth => p * p * (3.0 - 2.0 * p),
            Falloff::Smoother => p * p * p * (p * (p * 6.0 - 15.0) + 10.0),
            Falloff::Sphere => (1.0 - t * t).max(0.0).sqrt(),
            Falloff::Root => p.max(0.0).sqrt(),
            Falloff::Sharp => p * p,
            Falloff::Linear => p,
            Falloff::Constant => 1.0,
            Falloff::InverseSquare => p * (2.0 - p),
            Falloff::Custom(table) => sample_table(table, t),
        };
        w.clamp(0.0, 1.0)
    }

    /// Valida o falloff (curva customizada precisa de amostras finitas).
    pub fn validate(&self) -> ModelResult<()> {
        if let Falloff::Custom(t) = self {
            if t.is_empty() {
                return Err(ModelError::InvalidParameter(
                    "falloff customizado sem amostras".into(),
                ));
            }
            if t.iter().any(|v| !v.is_finite()) {
                return Err(ModelError::InvalidParameter(
                    "falloff customizado com amostra não finita".into(),
                ));
            }
        }
        Ok(())
    }

    /// Nome estável (para UI/serialização manual).
    pub fn name(&self) -> &'static str {
        match self {
            Falloff::Smooth => "smooth",
            Falloff::Smoother => "smoother",
            Falloff::Sphere => "sphere",
            Falloff::Root => "root",
            Falloff::Sharp => "sharp",
            Falloff::Linear => "linear",
            Falloff::Constant => "constant",
            Falloff::InverseSquare => "inverse_square",
            Falloff::Custom(_) => "custom",
        }
    }

    /// Todos os presets (sem `Custom`), na ordem da UI.
    pub fn presets() -> [Falloff; 8] {
        [
            Falloff::Smooth,
            Falloff::Smoother,
            Falloff::Sphere,
            Falloff::Root,
            Falloff::Sharp,
            Falloff::Linear,
            Falloff::Constant,
            Falloff::InverseSquare,
        ]
    }
}

fn sample_table(table: &[f32], t: f32) -> f32 {
    match table.len() {
        0 => 0.0,
        1 => table[0],
        n => {
            let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 2);
            let f = x - i as f32;
            table[i] + (table[i + 1] - table[i]) * f
        }
    }
}

/// Ponto de controle Bézier do editor de curva. `handle_l`/`handle_r` são
/// posições **absolutas** (no mesmo espaço de `pos`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub pos: Vec2,
    pub handle_l: Vec2,
    pub handle_r: Vec2,
}

impl CurvePoint {
    /// Ponto com alças horizontais de comprimento `h` (curva suave padrão).
    pub fn flat(x: f32, y: f32, h: f32) -> Self {
        Self {
            pos: Vec2::new(x, y),
            handle_l: Vec2::new(x - h, y),
            handle_r: Vec2::new(x + h, y),
        }
    }
}

/// Curva editável `y = f(x)` em `[0, 1]²` feita de segmentos Bézier cúbicos.
///
/// As alças têm o `x` limitado ao intervalo do segmento (e ordenado), o que
/// garante que `x(s)` seja monótono e a curva seja uma função de `x`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveProfile {
    points: Vec<CurvePoint>,
}

impl CurveProfile {
    /// Cria a curva validando: ≥ 2 pontos, valores finitos, `x` estritamente
    /// crescente e em `[0, 1]`.
    pub fn new(points: Vec<CurvePoint>) -> ModelResult<Self> {
        Self::validate_points(&points)?;
        Ok(Self { points })
    }

    fn validate_points(points: &[CurvePoint]) -> ModelResult<()> {
        if points.len() < 2 {
            return Err(ModelError::InvalidParameter(
                "curva precisa de ao menos 2 pontos".into(),
            ));
        }
        for p in points {
            if !(p.pos.is_finite() && p.handle_l.is_finite() && p.handle_r.is_finite()) {
                return Err(ModelError::InvalidParameter(
                    "ponto de curva com valor não finito".into(),
                ));
            }
            if !(0.0..=1.0).contains(&p.pos.x) {
                return Err(ModelError::InvalidParameter(format!(
                    "x do ponto fora de [0, 1]: {}",
                    p.pos.x
                )));
            }
        }
        for w in points.windows(2) {
            if w[1].pos.x <= w[0].pos.x {
                return Err(ModelError::InvalidParameter(
                    "pontos da curva devem ter x estritamente crescente".into(),
                ));
            }
        }
        Ok(())
    }

    /// Reta de 1 (centro) a 0 (borda).
    pub fn linear() -> Self {
        Self {
            points: vec![
                CurvePoint {
                    pos: Vec2::new(0.0, 1.0),
                    handle_l: Vec2::new(0.0, 1.0),
                    handle_r: Vec2::new(1.0 / 3.0, 2.0 / 3.0),
                },
                CurvePoint {
                    pos: Vec2::new(1.0, 0.0),
                    handle_l: Vec2::new(2.0 / 3.0, 1.0 / 3.0),
                    handle_r: Vec2::new(1.0, 0.0),
                },
            ],
        }
    }

    /// Curva suave estilo "smooth" (alças horizontais): 1 → 0.
    pub fn smooth() -> Self {
        Self {
            points: vec![CurvePoint::flat(0.0, 1.0, 0.4), CurvePoint::flat(1.0, 0.0, 0.4)],
        }
    }

    /// Curva constante `v` (clamp em [0, 1] na avaliação).
    pub fn constant(v: f32) -> Self {
        let v = if v.is_finite() { v } else { 1.0 };
        Self {
            points: vec![
                CurvePoint {
                    pos: Vec2::new(0.0, v),
                    handle_l: Vec2::new(0.0, v),
                    handle_r: Vec2::new(1.0 / 3.0, v),
                },
                CurvePoint {
                    pos: Vec2::new(1.0, v),
                    handle_l: Vec2::new(2.0 / 3.0, v),
                    handle_r: Vec2::new(1.0, v),
                },
            ],
        }
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// Insere um ponto em `(x, y)` com alças horizontais a 1/3 da distância
    /// aos vizinhos. Devolve o índice. Erro se `x` fora de `[0, 1]`, não finito
    /// ou repetido.
    pub fn add_point(&mut self, x: f32, y: f32) -> ModelResult<usize> {
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) {
            return Err(ModelError::InvalidParameter(format!(
                "ponto de curva inválido ({x}, {y})"
            )));
        }
        let idx = self.points.partition_point(|p| p.pos.x < x);
        if self.points.get(idx).is_some_and(|p| p.pos.x == x) {
            return Err(ModelError::InvalidParameter(format!(
                "já existe ponto com x = {x}"
            )));
        }
        let left = idx.checked_sub(1).map(|i| self.points[i].pos.x).unwrap_or(x);
        let right = self.points.get(idx).map(|p| p.pos.x).unwrap_or(x);
        let hl = ((x - left) / 3.0).max(0.0);
        let hr = ((right - x) / 3.0).max(0.0);
        self.points.insert(
            idx,
            CurvePoint {
                pos: Vec2::new(x, y),
                handle_l: Vec2::new(x - hl, y),
                handle_r: Vec2::new(x + hr, y),
            },
        );
        Ok(idx)
    }

    /// Remove o ponto `i` (a curva mantém ao menos 2 pontos).
    pub fn remove_point(&mut self, i: usize) -> ModelResult<()> {
        if self.points.len() <= 2 {
            return Err(ModelError::InvalidParameter(
                "a curva precisa manter ao menos 2 pontos".into(),
            ));
        }
        if i >= self.points.len() {
            return Err(ModelError::InvalidParameter(format!(
                "índice de ponto {i} fora do intervalo ({} pontos)",
                self.points.len()
            )));
        }
        self.points.remove(i);
        Ok(())
    }

    /// Move o ponto `i` (e suas alças junto). O `x` é limitado entre os
    /// vizinhos; `y` livre.
    pub fn move_point(&mut self, i: usize, pos: Vec2) -> ModelResult<()> {
        if i >= self.points.len() {
            return Err(ModelError::InvalidParameter(format!(
                "índice de ponto {i} fora do intervalo ({} pontos)",
                self.points.len()
            )));
        }
        if !pos.is_finite() {
            return Err(ModelError::InvalidParameter("posição não finita".into()));
        }
        let lo = if i == 0 { 0.0 } else { self.points[i - 1].pos.x + 1e-4 };
        let hi = if i + 1 == self.points.len() { 1.0 } else { self.points[i + 1].pos.x - 1e-4 };
        let x = pos.x.clamp(lo, hi.max(lo));
        let delta = Vec2::new(x, pos.y) - self.points[i].pos;
        let p = &mut self.points[i];
        p.pos += delta;
        p.handle_l += delta;
        p.handle_r += delta;
        Ok(())
    }

    /// Define as alças do ponto `i` (posições absolutas).
    pub fn set_handles(&mut self, i: usize, left: Vec2, right: Vec2) -> ModelResult<()> {
        let n = self.points.len();
        let p = self.points.get_mut(i).ok_or_else(|| {
            ModelError::InvalidParameter(format!("índice de ponto {i} fora do intervalo ({n} pontos)"))
        })?;
        if !(left.is_finite() && right.is_finite()) {
            return Err(ModelError::InvalidParameter("alça não finita".into()));
        }
        p.handle_l = left;
        p.handle_r = right;
        Ok(())
    }

    /// Avalia `y(x)` com `x` em `[0, 1]` (fora do alcance dos pontos, estende
    /// constante). O resultado é limitado a `[0, 1]`.
    pub fn eval(&self, x: f32) -> f32 {
        let first = &self.points[0];
        let last = &self.points[self.points.len() - 1];
        if x.is_nan() {
            return 0.0;
        }
        let y = if x <= first.pos.x {
            first.pos.y
        } else if x >= last.pos.x {
            last.pos.y
        } else {
            let i = self.points.partition_point(|p| p.pos.x <= x).saturating_sub(1);
            let i = i.min(self.points.len() - 2);
            eval_segment(&self.points[i], &self.points[i + 1], x)
        };
        y.clamp(0.0, 1.0)
    }

    /// Tabela de [`CURVE_TABLE_SIZE`] amostras uniformes em `x ∈ [0, 1]`.
    pub fn to_table(&self) -> Vec<f32> {
        (0..CURVE_TABLE_SIZE)
            .map(|i| self.eval(i as f32 / (CURVE_TABLE_SIZE - 1) as f32))
            .collect()
    }

    /// Falloff customizado a partir da curva.
    pub fn to_falloff(&self) -> Falloff {
        Falloff::Custom(self.to_table())
    }
}

/// Resolve o segmento Bézier cúbico `p0 → p1` para o `x` dado (bissecção em `s`).
fn eval_segment(p0: &CurvePoint, p1: &CurvePoint, x: f32) -> f32 {
    let a = p0.pos;
    let d = p1.pos;
    let mut b = p0.handle_r;
    let mut c = p1.handle_l;
    b.x = b.x.clamp(a.x, d.x);
    c.x = c.x.clamp(b.x, d.x);
    let bez = |s: f32, v0: f32, v1: f32, v2: f32, v3: f32| {
        let u = 1.0 - s;
        u * u * u * v0 + 3.0 * u * u * s * v1 + 3.0 * u * s * s * v2 + s * s * s * v3
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if bez(mid, a.x, b.x, c.x, d.x) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let s = 0.5 * (lo + hi);
    bez(s, a.y, b.y, c.y, d.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_one_at_center_and_zero_at_edge() {
        for f in Falloff::presets() {
            assert!((f.eval(0.0) - 1.0).abs() < 1e-6, "{} centro", f.name());
            if f != Falloff::Constant {
                assert!(f.eval(1.0).abs() < 1e-6, "{} borda", f.name());
            } else {
                assert_eq!(f.eval(1.0), 1.0);
            }
            assert_eq!(f.eval(1.0001), 0.0, "{} fora do raio", f.name());
            assert_eq!(f.eval(f32::NAN), 0.0);
            assert_eq!(f.eval(-0.5), f.eval(0.0), "{} t negativo", f.name());
        }
    }

    #[test]
    fn presets_are_monotone_non_increasing_and_in_unit_range() {
        for f in Falloff::presets() {
            let mut prev = f.eval(0.0);
            for i in 1..=200 {
                let w = f.eval(i as f32 / 200.0);
                assert!((0.0..=1.0).contains(&w), "{} fora de [0,1]: {w}", f.name());
                assert!(w <= prev + 1e-6, "{} cresceu em t={}", f.name(), i as f32 / 200.0);
                prev = w;
            }
        }
    }

    #[test]
    fn known_values_match_blender_formulas() {
        assert!((Falloff::Linear.eval(0.25) - 0.75).abs() < 1e-6);
        assert!((Falloff::Smooth.eval(0.5) - 0.5).abs() < 1e-6);
        assert!((Falloff::Smoother.eval(0.5) - 0.5).abs() < 1e-6);
        assert!((Falloff::Sharp.eval(0.5) - 0.25).abs() < 1e-6);
        assert!((Falloff::Root.eval(0.75) - 0.5).abs() < 1e-6);
        assert!((Falloff::Sphere.eval(0.6) - 0.8).abs() < 1e-6);
        assert!((Falloff::InverseSquare.eval(0.5) - 0.75).abs() < 1e-6);
        // Smooth: 3p²−2p³ com p=0.9
        let p = 0.9f32;
        assert!((Falloff::Smooth.eval(0.1) - (3.0 * p * p - 2.0 * p * p * p)).abs() < 1e-6);
    }

    #[test]
    fn shapes_are_ordered_at_midpoint() {
        // sharp < smooth/linear < root, sphere acima de linear
        let t = 0.5;
        assert!(Falloff::Sharp.eval(t) < Falloff::Linear.eval(t));
        assert!(Falloff::Linear.eval(t) < Falloff::Root.eval(t));
        assert!(Falloff::Linear.eval(t) < Falloff::Sphere.eval(t));
        assert!(Falloff::Linear.eval(t) < Falloff::InverseSquare.eval(t));
    }

    #[test]
    fn custom_interpolates_table_and_validates() {
        let f = Falloff::Custom(vec![1.0, 0.5, 0.0]);
        assert_eq!(f.eval(0.0), 1.0);
        assert!((f.eval(0.25) - 0.75).abs() < 1e-6);
        assert!((f.eval(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(f.eval(1.0), 0.0);
        assert_eq!(Falloff::Custom(vec![0.7]).eval(0.3), 0.7);
        assert!(f.validate().is_ok());
        assert!(Falloff::Custom(vec![]).validate().is_err());
        assert_eq!(Falloff::Custom(vec![]).eval(0.5), 0.0);
        assert!(Falloff::Custom(vec![1.0, f32::NAN]).validate().is_err());
        assert!(Falloff::Smooth.validate().is_ok());
        // valores fora de [0,1] são limitados
        assert_eq!(Falloff::Custom(vec![3.0, 3.0]).eval(0.5), 1.0);
    }

    #[test]
    fn curve_profile_linear_and_table() {
        let c = CurveProfile::linear();
        let table = c.to_table();
        assert_eq!(table.len(), CURVE_TABLE_SIZE);
        assert!((table[0] - 1.0).abs() < 1e-4);
        assert!(table[CURVE_TABLE_SIZE - 1].abs() < 1e-4);
        for (i, v) in table.iter().enumerate() {
            let want = 1.0 - i as f32 / (CURVE_TABLE_SIZE - 1) as f32;
            assert!((v - want).abs() < 2e-3, "i={i}: {v} vs {want}");
        }
        // falloff gerado concorda com a curva
        let f = c.to_falloff();
        assert!((f.eval(0.3) - 0.7).abs() < 3e-3);
    }

    #[test]
    fn curve_profile_smooth_is_s_shaped_and_monotone() {
        let c = CurveProfile::smooth();
        assert!((c.eval(0.5) - 0.5).abs() < 1e-3);
        assert!(c.eval(0.1) > 0.9, "alça horizontal mantém o topo");
        let t = c.to_table();
        assert!(t.windows(2).all(|w| w[1] <= w[0] + 1e-6));
    }

    #[test]
    fn curve_profile_constant_and_extension() {
        let c = CurveProfile::constant(0.4);
        assert!((c.eval(0.0) - 0.4).abs() < 1e-6 && (c.eval(0.77) - 0.4).abs() < 1e-5);
        assert!((CurveProfile::constant(f32::NAN).eval(0.5) - 1.0).abs() < 1e-6);
        assert_eq!(CurveProfile::linear().eval(f32::NAN), 0.0);
        // fora de [0,1] estende
        assert!((CurveProfile::linear().eval(2.0)).abs() < 1e-6);
        assert!((CurveProfile::linear().eval(-1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn curve_profile_add_move_remove_points() {
        let mut c = CurveProfile::linear();
        let i = c.add_point(0.5, 0.9).unwrap();
        assert_eq!(i, 1);
        assert_eq!(c.points().len(), 3);
        assert!((c.eval(0.5) - 0.9).abs() < 1e-3);
        // x repetido / fora do domínio / não finito
        assert!(c.add_point(0.5, 0.1).is_err());
        assert!(c.add_point(1.5, 0.1).is_err());
        assert!(c.add_point(f32::NAN, 0.1).is_err());
        // mover limita x entre vizinhos
        c.move_point(1, Vec2::new(2.0, 0.2)).unwrap();
        assert!(c.points()[1].pos.x < 1.0 && c.points()[1].pos.x > 0.0);
        assert!((c.points()[1].pos.y - 0.2).abs() < 1e-6);
        assert!(c.move_point(9, Vec2::ZERO).is_err());
        assert!(c.move_point(0, Vec2::new(f32::NAN, 0.0)).is_err());
        assert!(c.set_handles(0, Vec2::ZERO, Vec2::new(0.2, 0.8)).is_ok());
        assert!(c.set_handles(7, Vec2::ZERO, Vec2::ZERO).is_err());
        assert!(c.set_handles(0, Vec2::NAN, Vec2::ZERO).is_err());
        c.remove_point(1).unwrap();
        assert!(c.remove_point(0).is_err(), "mínimo de 2 pontos");
        let mut c3 = CurveProfile::linear();
        c3.add_point(0.4, 0.5).unwrap();
        assert!(c3.remove_point(10).is_err());
    }

    #[test]
    fn curve_profile_rejects_invalid_points() {
        let p = |x: f32, y: f32| CurvePoint::flat(x, y, 0.1);
        assert!(CurveProfile::new(vec![p(0.0, 1.0)]).is_err());
        assert!(CurveProfile::new(vec![p(0.5, 1.0), p(0.5, 0.0)]).is_err());
        assert!(CurveProfile::new(vec![p(0.6, 1.0), p(0.2, 0.0)]).is_err());
        assert!(CurveProfile::new(vec![p(-0.1, 1.0), p(1.0, 0.0)]).is_err());
        assert!(CurveProfile::new(vec![p(0.0, f32::INFINITY), p(1.0, 0.0)]).is_err());
        assert!(CurveProfile::new(vec![p(0.0, 1.0), p(1.0, 0.0)]).is_ok());
    }

    #[test]
    fn curve_profile_handles_cannot_break_function_property() {
        // alças "ao contrário" não podem gerar x não monótono
        let pts = vec![
            CurvePoint {
                pos: Vec2::new(0.0, 1.0),
                handle_l: Vec2::new(-1.0, 1.0),
                handle_r: Vec2::new(5.0, 0.0),
            },
            CurvePoint {
                pos: Vec2::new(1.0, 0.0),
                handle_l: Vec2::new(-4.0, 0.5),
                handle_r: Vec2::new(2.0, 0.0),
            },
        ];
        let c = CurveProfile::new(pts).unwrap();
        for v in c.to_table() {
            assert!((0.0..=1.0).contains(&v) && v.is_finite());
        }
    }

    #[test]
    fn serde_roundtrip() {
        let f = Falloff::Custom(vec![1.0, 0.25, 0.0]);
        let j = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Falloff>(&j).unwrap(), f);
        let c = CurveProfile::smooth();
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CurveProfile>(&j).unwrap(), c);
    }
}
