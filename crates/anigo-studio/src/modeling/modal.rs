//! Transformação modal estilo Blender (G/R/S + X/Y/Z + entrada numérica).
//!
//! A matemática é pura: parte das posições originais a cada atualização (sem acumular
//! erro) e mapeia o movimento do mouse em pixels para espaço de mundo usando a câmera.

use crate::viewport::OrbitCamera;
use glam::{Mat3, Quat, Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Grab,
    Rotate,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn vector(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }
}

/// Restrição de eixo: um eixo (Axis) ou o plano que o exclui (Shift+eixo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    None,
    Axis(Axis),
    Plane(Axis),
}

#[derive(Debug, Clone)]
pub struct ModalTransform {
    pub kind: TransformKind,
    pub constraint: Constraint,
    /// Entrada numérica digitada (ex.: "1.5", "-0.25"); vazia = usa o mouse.
    pub numeric: String,
    pub start_mouse: Vec2,
    pub pivot: Vec3,
    /// Posições originais (índice do vértice, posição).
    pub originals: Vec<(u32, Vec3)>,
    /// Peso por vértice (edição proporcional); mesma ordem de `originals`. `None` = 1.0.
    pub weights: Option<Vec<f32>>,
    /// Passo de snap por incremento (Ctrl): 0 desliga.
    pub snap_step: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalContext {
    pub camera: OrbitCamera,
    pub viewport: Vec2,
}

impl ModalTransform {
    pub fn new(kind: TransformKind, start_mouse: Vec2, pivot: Vec3, originals: Vec<(u32, Vec3)>) -> Self {
        Self {
            kind,
            constraint: Constraint::None,
            numeric: String::new(),
            start_mouse,
            pivot,
            originals,
            weights: None,
            snap_step: 0.0,
        }
    }

    /// Alterna a restrição ao pressionar X/Y/Z (repetir o mesmo eixo remove).
    pub fn toggle_axis(&mut self, axis: Axis, plane: bool) {
        let target = if plane { Constraint::Plane(axis) } else { Constraint::Axis(axis) };
        self.constraint = if self.constraint == target { Constraint::None } else { target };
    }

    /// Acrescenta um caractere à entrada numérica; devolve `false` se inválido.
    pub fn push_numeric(&mut self, ch: char) -> bool {
        let ok = match ch {
            '0'..='9' => true,
            '.' | ',' => !self.numeric.contains('.'),
            '-' => {
                // alterna o sinal
                if let Some(rest) = self.numeric.strip_prefix('-') {
                    self.numeric = rest.to_string();
                } else {
                    self.numeric.insert(0, '-');
                }
                return true;
            }
            _ => false,
        };
        if ok {
            self.numeric.push(if ch == ',' { '.' } else { ch });
        }
        ok
    }

    pub fn pop_numeric(&mut self) {
        self.numeric.pop();
    }

    fn numeric_value(&self) -> Option<f32> {
        match self.numeric.as_str() {
            "" | "-" | "." | "-." => None,
            s => s.parse().ok(),
        }
    }

    fn snap(&self, v: f32) -> f32 {
        if self.snap_step > 0.0 {
            (v / self.snap_step).round() * self.snap_step
        } else {
            v
        }
    }

    /// Delta de translação em mundo para a posição atual do mouse.
    pub fn translation(&self, mouse: Vec2, ctx: &ModalContext) -> Vec3 {
        let cam = &ctx.camera;
        let numeric = self.numeric_value();
        // Pixels → mundo na profundidade do pivô.
        let dist = (cam.eye() - self.pivot).dot(-cam.forward()).abs().max(1e-3);
        let world_per_px = if cam.orthographic {
            2.0 * cam.distance * (cam.fov_y * 0.5).tan() / ctx.viewport.y.max(1.0)
        } else {
            2.0 * dist * (cam.fov_y * 0.5).tan() / ctx.viewport.y.max(1.0)
        };
        let d = mouse - self.start_mouse;
        let screen_delta = cam.right() * d.x * world_per_px - cam.up() * d.y * world_per_px;

        let delta = match self.constraint {
            Constraint::None => match numeric {
                Some(n) => cam.right() * n, // sem eixo, número move para a direita da tela
                None => screen_delta,
            },
            Constraint::Axis(a) => {
                let axis = a.vector();
                let amount = match numeric {
                    Some(n) => n,
                    None => {
                        // projeta o delta de tela sobre o eixo
                        let along = screen_delta.dot(axis);
                        // fator de escorço: eixo quase paralelo ao olhar rende pouco movimento de tela;
                        // compensa para manter controle (limitado)
                        let foreshorten = (1.0 - axis.dot(cam.forward()).powi(2)).sqrt().max(0.15);
                        along / foreshorten
                    }
                };
                axis * self.snap(amount)
            }
            Constraint::Plane(a) => {
                let mut v = match numeric {
                    Some(n) => cam.right() * n,
                    None => screen_delta,
                };
                // remove a componente no eixo excluído
                let axis = a.vector();
                v -= axis * v.dot(axis);
                if self.snap_step > 0.0 {
                    v = Vec3::new(self.snap(v.x), self.snap(v.y), self.snap(v.z));
                }
                v
            }
        };
        if matches!(self.constraint, Constraint::None) && self.snap_step > 0.0 {
            Vec3::new(self.snap(delta.x), self.snap(delta.y), self.snap(delta.z))
        } else {
            delta
        }
    }

    /// Ângulo de rotação (radianos) pela variação do ângulo do mouse em torno do pivô na tela.
    pub fn rotation_angle(&self, mouse: Vec2, ctx: &ModalContext) -> f32 {
        if let Some(n) = self.numeric_value() {
            return n.to_radians();
        }
        let Some((pivot_px, _)) = ctx.camera.project(self.pivot, ctx.viewport) else {
            return 0.0;
        };
        let a0 = (self.start_mouse - pivot_px).to_array();
        let a1 = (mouse - pivot_px).to_array();
        let ang0 = a0[1].atan2(a0[0]);
        let ang1 = a1[1].atan2(a1[0]);
        let mut d = ang1 - ang0;
        // normaliza para (-π, π]
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d <= -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        if self.snap_step > 0.0 {
            let step = 5f32.to_radians();
            (d / step).round() * step
        } else {
            d
        }
    }

    /// Fator de escala: razão das distâncias mouse↔pivô na tela.
    pub fn scale_factor(&self, mouse: Vec2, ctx: &ModalContext) -> f32 {
        if let Some(n) = self.numeric_value() {
            return n;
        }
        let Some((pivot_px, _)) = ctx.camera.project(self.pivot, ctx.viewport) else {
            return 1.0;
        };
        let d0 = (self.start_mouse - pivot_px).length().max(4.0);
        let d1 = (mouse - pivot_px).length();
        let f = d1 / d0;
        if self.snap_step > 0.0 {
            (f / 0.1).round() * 0.1
        } else {
            f
        }
    }

    /// Eixo de rotação de mundo: restrição, senão o eixo de visão.
    fn rotation_axis(&self, ctx: &ModalContext) -> Vec3 {
        match self.constraint {
            Constraint::Axis(a) | Constraint::Plane(a) => a.vector(),
            Constraint::None => -ctx.camera.forward(),
        }
    }

    fn scale_vector(&self, f: f32) -> Vec3 {
        match self.constraint {
            Constraint::None => Vec3::splat(f),
            Constraint::Axis(a) => {
                let mut s = Vec3::ONE;
                match a {
                    Axis::X => s.x = f,
                    Axis::Y => s.y = f,
                    Axis::Z => s.z = f,
                }
                s
            }
            Constraint::Plane(a) => {
                let mut s = Vec3::splat(f);
                match a {
                    Axis::X => s.x = 1.0,
                    Axis::Y => s.y = 1.0,
                    Axis::Z => s.z = 1.0,
                }
                s
            }
        }
    }

    /// Posições novas (índice, posição) para o estado atual do mouse.
    pub fn apply(&self, mouse: Vec2, ctx: &ModalContext) -> Vec<(u32, Vec3)> {
        let w = |i: usize| self.weights.as_ref().and_then(|w| w.get(i)).copied().unwrap_or(1.0);
        match self.kind {
            TransformKind::Grab => {
                let delta = self.translation(mouse, ctx);
                self.originals
                    .iter()
                    .enumerate()
                    .map(|(i, (v, p))| (*v, *p + delta * w(i)))
                    .collect()
            }
            TransformKind::Rotate => {
                let angle = self.rotation_angle(mouse, ctx);
                let axis = self.rotation_axis(ctx);
                self.originals
                    .iter()
                    .enumerate()
                    .map(|(i, (v, p))| {
                        let q = Quat::from_axis_angle(axis, angle * w(i));
                        (*v, self.pivot + q * (*p - self.pivot))
                    })
                    .collect()
            }
            TransformKind::Scale => {
                let f = self.scale_factor(mouse, ctx);
                let s = self.scale_vector(f);
                self.originals
                    .iter()
                    .enumerate()
                    .map(|(i, (v, p))| {
                        let wi = w(i);
                        let sv = Vec3::ONE + (s - Vec3::ONE) * wi;
                        (*v, self.pivot + (*p - self.pivot) * sv)
                    })
                    .collect()
            }
        }
    }

    /// Texto da barra de status ("D: 1.250 m  X").
    pub fn status_text(&self, mouse: Vec2, ctx: &ModalContext) -> String {
        let axis = match self.constraint {
            Constraint::None => String::new(),
            Constraint::Axis(a) => format!("  {a:?}"),
            Constraint::Plane(a) => format!("  plano ⊥ {a:?}"),
        };
        match self.kind {
            TransformKind::Grab => {
                let d = self.translation(mouse, ctx);
                format!("Mover  Δ({:.3}, {:.3}, {:.3}){axis}", d.x, d.y, d.z)
            }
            TransformKind::Rotate => format!("Rotacionar  {:.1}°{axis}", self.rotation_angle(mouse, ctx).to_degrees()),
            TransformKind::Scale => format!("Escalar  ×{:.3}{axis}", self.scale_factor(mouse, ctx)),
        }
    }
}

/// Matriz de orientação de um eixo (útil para orientação local/normal no futuro).
pub fn basis_from_normal(n: Vec3) -> Mat3 {
    let n = n.normalize_or_zero();
    let helper = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
    let t = helper.cross(n).normalize_or_zero();
    let b = n.cross(t);
    Mat3::from_cols(t, n, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ModalContext {
        let camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 6.0,
            ..Default::default()
        };
        ModalContext {
            camera,
            viewport: Vec2::new(800.0, 600.0),
        }
    }

    fn one_point(kind: TransformKind) -> ModalTransform {
        ModalTransform::new(kind, Vec2::new(400.0, 300.0), Vec3::ZERO, vec![(0, Vec3::new(1.0, 0.0, 0.0))])
    }

    #[test]
    fn grab_free_follows_mouse_in_screen_plane() {
        let c = ctx();
        let m = one_point(TransformKind::Grab);
        let out = m.apply(Vec2::new(500.0, 300.0), &c);
        let delta = out[0].1 - Vec3::new(1.0, 0.0, 0.0);
        assert!(delta.x > 0.1 && delta.y.abs() < 1e-5 && delta.z.abs() < 1e-5, "{delta:?}");
        // mouse para cima (y menor) → +Y no mundo
        let up = m.apply(Vec2::new(400.0, 200.0), &c);
        assert!(up[0].1.y > 0.05);
    }

    #[test]
    fn grab_axis_constraint_only_moves_that_axis() {
        let c = ctx();
        let mut m = one_point(TransformKind::Grab);
        m.toggle_axis(Axis::X, false);
        let out = m.apply(Vec2::new(500.0, 250.0), &c);
        let delta = out[0].1 - Vec3::new(1.0, 0.0, 0.0);
        assert!(delta.x.abs() > 0.1 && delta.y.abs() < 1e-6 && delta.z.abs() < 1e-6);
    }

    #[test]
    fn numeric_input_overrides_mouse_and_is_exact() {
        let c = ctx();
        let mut m = one_point(TransformKind::Grab);
        m.toggle_axis(Axis::Y, false);
        for ch in "1.25".chars() {
            assert!(m.push_numeric(ch));
        }
        let out = m.apply(Vec2::new(0.0, 0.0), &c);
        assert!((out[0].1 - Vec3::new(1.0, 1.25, 0.0)).length() < 1e-6);
        assert!(!m.push_numeric('.'), "segundo ponto decimal é recusado");
        m.push_numeric('-');
        assert!(m.numeric.starts_with('-'));
        m.pop_numeric();
        assert!(m.numeric.starts_with('-'));
    }

    #[test]
    fn rotate_about_pivot_by_numeric_degrees() {
        let c = ctx();
        let mut m = one_point(TransformKind::Rotate);
        m.toggle_axis(Axis::Y, false);
        for ch in "90".chars() {
            m.push_numeric(ch);
        }
        let out = m.apply(Vec2::ZERO, &c);
        // (1,0,0) girado 90° em Y → (0,0,-1)
        assert!((out[0].1 - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5, "{:?}", out[0].1);
    }

    #[test]
    fn rotate_by_mouse_quarter_turn_around_pivot() {
        let c = ctx();
        let m = ModalTransform::new(
            TransformKind::Rotate,
            Vec2::new(500.0, 300.0), // à direita do pivô (centro da tela)
            Vec3::ZERO,
            vec![(0, Vec3::new(1.0, 0.0, 0.0))],
        );
        let angle = m.rotation_angle(Vec2::new(400.0, 400.0), &c); // abaixo do pivô: +90° em coords de tela
        assert!((angle.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.02, "{angle}");
    }

    #[test]
    fn scale_with_constraint_and_proportional_weights() {
        let c = ctx();
        let mut m = ModalTransform::new(
            TransformKind::Scale,
            Vec2::new(500.0, 300.0),
            Vec3::ZERO,
            vec![(0, Vec3::new(1.0, 1.0, 1.0)), (1, Vec3::new(2.0, 2.0, 2.0))],
        );
        m.toggle_axis(Axis::Z, false);
        for ch in "2".chars() {
            m.push_numeric(ch);
        }
        m.weights = Some(vec![1.0, 0.5]);
        let out = m.apply(Vec2::ZERO, &c);
        assert!((out[0].1 - Vec3::new(1.0, 1.0, 2.0)).length() < 1e-6);
        // peso 0.5: fator efetivo 1.5 em Z
        assert!((out[1].1 - Vec3::new(2.0, 2.0, 3.0)).length() < 1e-6);
    }

    #[test]
    fn plane_constraint_excludes_axis() {
        let c = ctx();
        let mut m = one_point(TransformKind::Grab);
        m.toggle_axis(Axis::Z, true);
        let out = m.apply(Vec2::new(480.0, 260.0), &c);
        assert!((out[0].1.z - 0.0).abs() < 1e-6);
    }

    #[test]
    fn axis_toggle_twice_clears_and_snap_rounds() {
        let c = ctx();
        let mut m = one_point(TransformKind::Grab);
        m.toggle_axis(Axis::X, false);
        m.toggle_axis(Axis::X, false);
        assert_eq!(m.constraint, Constraint::None);
        m.toggle_axis(Axis::X, false);
        m.snap_step = 0.5;
        let out = m.apply(Vec2::new(437.0, 300.0), &c);
        let dx = out[0].1.x - 1.0;
        assert!(((dx / 0.5).round() * 0.5 - dx).abs() < 1e-6);
    }

    #[test]
    fn status_text_mentions_axis() {
        let c = ctx();
        let mut m = one_point(TransformKind::Scale);
        m.toggle_axis(Axis::Y, false);
        assert!(m.status_text(Vec2::new(500.0, 300.0), &c).contains("Y"));
    }

    #[test]
    fn basis_is_orthonormal() {
        let b = basis_from_normal(Vec3::new(0.3, 0.8, -0.2));
        assert!((b.x_axis.length() - 1.0).abs() < 1e-5);
        assert!(b.x_axis.dot(b.y_axis).abs() < 1e-5);
        assert!(b.y_axis.dot(b.z_axis).abs() < 1e-5);
    }
}
