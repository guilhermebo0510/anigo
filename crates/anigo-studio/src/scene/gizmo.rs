//! Gizmo 3D do Cenário (mover/girar/escalar): geometria, hit-test e solvers de arrasto — tudo
//! matemática pura, em pixels de tela e raios de mundo, sem egui (o desenho fica no viewport).
//!
//! * tamanho constante em pixels (`frame_size`);
//! * arrasto de eixo por **reta–raio** (ponto da reta mais próximo do raio), de plano por
//!   **raio–plano**, de anel por ângulo no plano do anel, de escala por razão de distâncias;
//! * todos os solvers acumulam desde o ponto onde o arrasto COMEÇOU (sem deriva);
//! * World/Local só muda o `GizmoFrame` (os eixos), não os solvers.

use super::camview::CamView;
use anigo_core::{ids::NodeId, math::Transform};
use glam::{Mat4, Quat, Vec2, Vec3};

/// Tamanho do gizmo na tela, em pixels.
pub const GIZMO_PX: f32 = 100.0;
/// Raio de acerto das alças, em pixels.
pub const PICK_PX: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis3 {
    X,
    Y,
    Z,
}

impl Axis3 {
    pub const ALL: [Axis3; 3] = [Axis3::X, Axis3::Y, Axis3::Z];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn unit(self) -> Vec3 {
        match self {
            Axis3::X => Vec3::X,
            Axis3::Y => Vec3::Y,
            Axis3::Z => Vec3::Z,
        }
    }

    /// Os outros dois eixos, em ordem cíclica (X→Y,Z; Y→Z,X; Z→X,Y): `a × b = self`.
    pub fn others(self) -> (Axis3, Axis3) {
        match self {
            Axis3::X => (Axis3::Y, Axis3::Z),
            Axis3::Y => (Axis3::Z, Axis3::X),
            Axis3::Z => (Axis3::X, Axis3::Y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GizmoMode {
    #[default]
    Select,
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    /// Espaço → mover → girar → escalar → mover… (Espaço cicla, como no UE).
    pub fn cycle(self) -> GizmoMode {
        match self {
            GizmoMode::Select | GizmoMode::Scale => GizmoMode::Translate,
            GizmoMode::Translate => GizmoMode::Rotate,
            GizmoMode::Rotate => GizmoMode::Scale,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GizmoSpace {
    #[default]
    World,
    Local,
}

/// Alça do gizmo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Seta (mover) / haste com cubo (escalar) de um eixo.
    Axis(Axis3),
    /// Quadrado entre dois eixos; o valor é o eixo NORMAL ao plano.
    Plane(Axis3),
    /// Centro: mover livre (plano da tela) / escala uniforme.
    Center,
    /// Anel de rotação em torno do eixo.
    Ring(Axis3),
}

/// Posição e orientação do gizmo no mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoFrame {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
    /// Comprimento (mundo) que dá `GIZMO_PX` pixels na profundidade da origem.
    pub size: f32,
}

impl GizmoFrame {
    pub fn new(origin: Vec3, orientation: Quat, view: &CamView) -> Self {
        let q = orientation.normalize();
        Self { origin, axes: [q * Vec3::X, q * Vec3::Y, q * Vec3::Z], size: frame_size(view, origin) }
    }

    pub fn axis(&self, a: Axis3) -> Vec3 {
        self.axes[a.index()]
    }

    /// Segmento da seta/haste de um eixo (da base até a ponta).
    pub fn axis_segment(&self, a: Axis3) -> (Vec3, Vec3) {
        (self.origin + self.axis(a) * (self.size * 0.14), self.origin + self.axis(a) * self.size)
    }

    /// Quadrado do plano cuja normal é `n`: de 0,28 a 0,52 do tamanho nos dois eixos.
    pub fn plane_quad(&self, n: Axis3) -> [Vec3; 4] {
        let (a, b) = n.others();
        let (ua, ub) = (self.axis(a), self.axis(b));
        let (lo, hi) = (self.size * 0.28, self.size * 0.52);
        [
            self.origin + ua * lo + ub * lo,
            self.origin + ua * hi + ub * lo,
            self.origin + ua * hi + ub * hi,
            self.origin + ua * lo + ub * hi,
        ]
    }

    /// Pontos do anel de rotação do eixo `a` (polilinha fechada).
    pub fn ring_points(&self, a: Axis3, segments: usize) -> Vec<Vec3> {
        let (u, v) = self.ring_basis(a);
        let r = self.size * 0.86;
        (0..=segments)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / segments as f32;
                self.origin + (u * t.cos() + v * t.sin()) * r
            })
            .collect()
    }

    /// Base (u, v) do plano do anel, com `u × v` = eixo.
    pub fn ring_basis(&self, a: Axis3) -> (Vec3, Vec3) {
        let (p, q) = a.others();
        (self.axis(p), self.axis(q))
    }
}

/// Tamanho do gizmo em unidades de mundo para ocupar `GIZMO_PX` pixels.
pub fn frame_size(view: &CamView, origin: Vec3) -> f32 {
    GIZMO_PX * view.world_per_pixel(origin)
}

fn dist_point_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 < 1e-9 {
        return p.distance(a);
    }
    p.distance(a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0))
}

fn point_in_quad(p: Vec2, q: &[Vec2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let c = (b - a).perp_dot(p - a);
        if c.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Alça sob o cursor para o modo atual (prioridade: centro > planos > eixos).
pub fn hit_test(mode: GizmoMode, frame: &GizmoFrame, view: &CamView, mouse: Vec2) -> Option<Handle> {
    let (origin_px, _) = view.project(frame.origin)?;
    match mode {
        GizmoMode::Select => None,
        GizmoMode::Translate | GizmoMode::Scale => {
            if origin_px.distance(mouse) <= 10.0 {
                return Some(Handle::Center);
            }
            for n in Axis3::ALL {
                let q = frame.plane_quad(n);
                let mut px = [Vec2::ZERO; 4];
                let mut ok = true;
                for (i, c) in q.iter().enumerate() {
                    match view.project(*c) {
                        Some((p, _)) => px[i] = p,
                        None => ok = false,
                    }
                }
                if ok && point_in_quad(mouse, &px) {
                    return Some(Handle::Plane(n));
                }
            }
            let mut best: Option<(f32, Axis3)> = None;
            for a in Axis3::ALL {
                let (s, e) = frame.axis_segment(a);
                let (Some((ps, _)), Some((pe, _))) = (view.project(s), view.project(e)) else { continue };
                let d = dist_point_segment(mouse, ps, pe);
                if d <= PICK_PX && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, a));
                }
            }
            best.map(|(_, a)| Handle::Axis(a))
        }
        GizmoMode::Rotate => {
            let mut best: Option<(f32, Axis3)> = None;
            for a in Axis3::ALL {
                let pts = frame.ring_points(a, 64);
                let mut prev: Option<Vec2> = None;
                let mut d_min = f32::MAX;
                for p in pts {
                    match view.project(p) {
                        Some((px, _)) => {
                            if let Some(q) = prev {
                                d_min = d_min.min(dist_point_segment(mouse, q, px));
                            }
                            prev = Some(px);
                        }
                        None => prev = None,
                    }
                }
                if d_min <= PICK_PX - 1.0 && best.is_none_or(|(bd, _)| d_min < bd) {
                    best = Some((d_min, a));
                }
            }
            best.map(|(_, a)| Handle::Ring(a))
        }
    }
}

/// Parâmetro `t` do ponto da reta `p0 + t·d` mais próximo do raio `(ro, rd)`; `None` se paralelos.
pub fn closest_param_on_line_to_ray(p0: Vec3, d: Vec3, ro: Vec3, rd: Vec3) -> Option<f32> {
    let w = p0 - ro;
    let a = d.dot(d);
    let b = d.dot(rd);
    let c = rd.dot(rd);
    let dd = d.dot(w);
    let e = rd.dot(w);
    let denom = a * c - b * b;
    if denom.abs() < 1e-8 * a * c {
        return None;
    }
    Some((b * e - c * dd) / denom)
}

/// Interseção raio–plano.
pub fn ray_plane(ro: Vec3, rd: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = rd.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - ro).dot(normal) / denom;
    (t.is_finite() && t > 0.0).then(|| ro + rd * t)
}

/// Resultado bruto (sem snap) de um passo de arrasto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragDelta {
    /// Deslocamento de mundo desde o início.
    Translate(Vec3),
    /// Rotação acumulada (radianos) em torno de um eixo de mundo.
    Rotate { axis: Vec3, angle: f32 },
    /// Fatores de escala por eixo do gizmo (1 = sem mudança); `uniform` quando veio do centro.
    Scale { factors: Vec3, uniform: bool },
}

/// Estado de um arrasto de gizmo (guardado pela UI entre quadros).
#[derive(Debug, Clone, Copy)]
pub struct GizmoDrag {
    pub mode: GizmoMode,
    pub handle: Handle,
    pub frame: GizmoFrame,
    start_param: f32,
    start_point: Vec3,
    last_angle: f32,
    accum_angle: f32,
    start_mouse: Vec2,
    pivot_px: Vec2,
    ring_fallback: bool,
}

impl GizmoDrag {
    pub fn begin(mode: GizmoMode, handle: Handle, frame: GizmoFrame, view: &CamView, mouse: Vec2) -> Option<GizmoDrag> {
        let (ro, rd) = view.ray(mouse);
        let (pivot_px, _) = view.project(frame.origin)?;
        let mut d = GizmoDrag {
            mode,
            handle,
            frame,
            start_param: 0.0,
            start_point: frame.origin,
            last_angle: 0.0,
            accum_angle: 0.0,
            start_mouse: mouse,
            pivot_px,
            ring_fallback: false,
        };
        match handle {
            Handle::Axis(a) => {
                let t = closest_param_on_line_to_ray(frame.origin, frame.axis(a), ro, rd)?;
                d.start_param = t;
                d.start_point = frame.origin + frame.axis(a) * t;
            }
            Handle::Plane(n) => {
                d.start_point = ray_plane(ro, rd, frame.origin, frame.axis(n))?;
            }
            Handle::Center => {
                d.start_point = ray_plane(ro, rd, frame.origin, view.forward).unwrap_or(frame.origin);
            }
            Handle::Ring(a) => match ray_plane_any(ro, rd, frame.origin, frame.axis(a)) {
                Some(p) if rd.dot(frame.axis(a)).abs() > 0.12 => {
                    d.start_point = p;
                    d.last_angle = d.ring_angle(a, p);
                }
                _ => d.ring_fallback = true,
            },
        }
        Some(d)
    }

    fn ring_angle(&self, a: Axis3, p: Vec3) -> f32 {
        let (u, v) = self.frame.ring_basis(a);
        let rel = p - self.frame.origin;
        rel.dot(v).atan2(rel.dot(u))
    }

    /// Um passo do arrasto com o cursor em `mouse`. `None` se não há solução neste quadro.
    pub fn update(&mut self, view: &CamView, mouse: Vec2) -> Option<DragDelta> {
        let (ro, rd) = view.ray(mouse);
        let f = &self.frame;
        match (self.mode, self.handle) {
            (GizmoMode::Translate, Handle::Axis(a)) => {
                let t = closest_param_on_line_to_ray(f.origin, f.axis(a), ro, rd)?;
                Some(DragDelta::Translate(f.axis(a) * (t - self.start_param)))
            }
            (GizmoMode::Translate, Handle::Plane(n)) => {
                let p = ray_plane(ro, rd, f.origin, f.axis(n))?;
                Some(DragDelta::Translate(p - self.start_point))
            }
            (GizmoMode::Translate, Handle::Center) => {
                let p = ray_plane(ro, rd, f.origin, view.forward)?;
                Some(DragDelta::Translate(p - self.start_point))
            }
            (GizmoMode::Rotate, Handle::Ring(a)) => {
                if self.ring_fallback {
                    // anel visto de perfil: arrasto horizontal gira (≈ 0,5°/px)
                    self.accum_angle = (mouse.x - self.start_mouse.x) * 0.0087;
                } else if let Some(p) = ray_plane_any(ro, rd, f.origin, f.axis(a)) {
                    let ang = self.ring_angle(a, p);
                    let mut d = ang - self.last_angle;
                    while d > std::f32::consts::PI {
                        d -= std::f32::consts::TAU;
                    }
                    while d < -std::f32::consts::PI {
                        d += std::f32::consts::TAU;
                    }
                    self.accum_angle += d;
                    self.last_angle = ang;
                }
                Some(DragDelta::Rotate { axis: f.axis(a), angle: self.accum_angle })
            }
            (GizmoMode::Scale, Handle::Axis(a)) => {
                let t = closest_param_on_line_to_ray(f.origin, f.axis(a), ro, rd)?;
                let t0 = if self.start_param.abs() < f.size * 0.1 { f.size * 0.5 } else { self.start_param };
                let factor = (t / t0).clamp(0.01, 100.0);
                let mut factors = Vec3::ONE;
                factors[a.index()] = factor;
                Some(DragDelta::Scale { factors, uniform: false })
            }
            (GizmoMode::Scale, Handle::Plane(n)) => {
                let p = ray_plane(ro, rd, f.origin, f.axis(n))?;
                let r0 = (self.start_point - f.origin).length().max(f.size * 0.05);
                let factor = ((p - f.origin).length() / r0).clamp(0.01, 100.0);
                let mut factors = Vec3::splat(factor);
                factors[n.index()] = 1.0;
                Some(DragDelta::Scale { factors, uniform: false })
            }
            (GizmoMode::Scale, Handle::Center) => {
                let r0 = self.start_mouse.distance(self.pivot_px).max(12.0);
                let factor = (mouse.distance(self.pivot_px) / r0).clamp(0.01, 100.0);
                Some(DragDelta::Scale { factors: Vec3::splat(factor), uniform: true })
            }
            _ => None,
        }
    }
}

/// Raio–plano sem exigir `t > 0` (o plano do anel pode estar "atrás" do olho em ortográfica).
fn ray_plane_any(ro: Vec3, rd: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = rd.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - ro).dot(normal) / denom;
    t.is_finite().then(|| ro + rd * t)
}

// ---------------------------------------------------------------------------------------------
// Aplicação do arrasto aos nós
// ---------------------------------------------------------------------------------------------

/// Estado inicial de um nó no arrasto.
#[derive(Debug, Clone)]
pub struct NodeXf {
    pub id: NodeId,
    pub parent_world: Mat4,
    pub start_local: Transform,
    pub start_world: Mat4,
}

impl NodeXf {
    pub fn world_pos(&self) -> Vec3 {
        self.start_world.transform_point3(Vec3::ZERO)
    }

    fn parent_is_identity(&self) -> bool {
        self.parent_world.abs_diff_eq(Mat4::IDENTITY, 1e-6)
    }
}

/// Qual componente do TRS local mudou (o comando só escreve o que mudou, sem ruído numérico).
#[derive(Debug, Clone, PartialEq)]
pub struct NewLocal {
    pub id: NodeId,
    pub translation: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
}

fn local_from_world(node: &NodeXf, new_world: Mat4) -> (Vec3, Quat, Vec3) {
    let local = node.parent_world.inverse() * new_world;
    let (s, r, t) = local.to_scale_rotation_translation();
    (t, r.normalize(), s)
}

/// Move todos os nós por `delta` (mundo).
pub fn apply_translate(nodes: &[NodeXf], delta: Vec3) -> Vec<NewLocal> {
    nodes
        .iter()
        .map(|n| {
            let target = n.world_pos() + delta;
            let t = if n.parent_is_identity() { target } else { n.parent_world.inverse().transform_point3(target) };
            NewLocal { id: n.id.clone(), translation: Some(t), rotation: None, scale: None }
        })
        .collect()
}

/// Gira todos os nós em torno de `pivot` (mundo) por `angle` em torno de `axis` (mundo).
pub fn apply_rotate(nodes: &[NodeXf], pivot: Vec3, axis: Vec3, angle: f32) -> Vec<NewLocal> {
    let dq = Quat::from_axis_angle(axis.normalize_or_zero(), angle);
    nodes
        .iter()
        .map(|n| {
            if n.parent_is_identity() {
                NewLocal {
                    id: n.id.clone(),
                    translation: Some(pivot + dq * (n.start_local.translation - pivot)),
                    rotation: Some((dq * n.start_local.rotation).normalize()),
                    scale: None,
                }
            } else {
                let m = Mat4::from_translation(pivot) * Mat4::from_quat(dq) * Mat4::from_translation(-pivot) * n.start_world;
                let (t, r, _) = local_from_world(n, m);
                NewLocal { id: n.id.clone(), translation: Some(t), rotation: Some(r), scale: None }
            }
        })
        .collect()
}

/// Escala os nós: `factors` por eixo LOCAL de cada nó (como o UE, a escala é sempre local); com
/// `uniform` as posições também escalam em torno de `pivot`.
pub fn apply_scale(nodes: &[NodeXf], pivot: Vec3, factors: Vec3, uniform: bool) -> Vec<NewLocal> {
    nodes
        .iter()
        .map(|n| {
            let scale = n.start_local.scale * factors;
            let translation = if uniform && nodes.len() > 1 {
                let target = pivot + (n.world_pos() - pivot) * factors.x;
                Some(if n.parent_is_identity() { target } else { n.parent_world.inverse().transform_point3(target) })
            } else {
                None
            };
            NewLocal { id: n.id.clone(), translation, rotation: None, scale: Some(scale) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::OrbitCamera;

    const VP: Vec2 = Vec2::new(900.0, 600.0);

    fn view() -> CamView {
        let cam = OrbitCamera { target: Vec3::new(0.0, 1.0, 0.0), yaw: 0.35, pitch: 0.3, distance: 7.0, ..Default::default() };
        CamView::from_orbit(&cam, VP)
    }

    fn frame(v: &CamView) -> GizmoFrame {
        GizmoFrame::new(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY, v)
    }

    #[test]
    fn gizmo_keeps_a_constant_size_in_pixels_at_any_distance() {
        for distance in [2.0, 7.0, 30.0] {
            let cam = OrbitCamera { target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance, ..Default::default() };
            let v = CamView::from_orbit(&cam, VP);
            let f = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY, &v);
            let (a, _) = v.project(f.origin).unwrap();
            let (b, _) = v.project(f.origin + Vec3::X * f.size).unwrap();
            assert!((a.distance(b) - GIZMO_PX).abs() < 3.0, "d={distance}: {}", a.distance(b));
        }
    }

    #[test]
    fn hit_test_prioritises_center_then_planes_then_axes() {
        let v = view();
        let f = frame(&v);
        let (o, _) = v.project(f.origin).unwrap();
        assert_eq!(hit_test(GizmoMode::Translate, &f, &v, o + Vec2::new(2.0, 2.0)), Some(Handle::Center));
        // meio da haste do eixo Y
        let (s, e) = f.axis_segment(Axis3::Y);
        let (ps, _) = v.project(s).unwrap();
        let (pe, _) = v.project(e).unwrap();
        assert_eq!(hit_test(GizmoMode::Translate, &f, &v, (ps + pe) * 0.5), Some(Handle::Axis(Axis3::Y)));
        // centro do quadrado do plano XZ (normal Y)
        let q = f.plane_quad(Axis3::Y);
        let c = (q[0] + q[2]) * 0.5;
        let (pc, _) = v.project(c).unwrap();
        assert_eq!(hit_test(GizmoMode::Translate, &f, &v, pc), Some(Handle::Plane(Axis3::Y)));
        assert_eq!(hit_test(GizmoMode::Translate, &f, &v, Vec2::new(5.0, 5.0)), None);
        assert_eq!(hit_test(GizmoMode::Select, &f, &v, o), None);
        // anel: um ponto do anel Y é acertado como Ring(Y)
        let ring = f.ring_points(Axis3::Y, 64);
        let (pr, _) = v.project(ring[13]).unwrap();
        let hit = hit_test(GizmoMode::Rotate, &f, &v, pr);
        assert!(matches!(hit, Some(Handle::Ring(_))), "{hit:?}");
    }

    #[test]
    fn axis_drag_follows_the_cursor_along_the_axis() {
        let v = view();
        let f = frame(&v);
        let start_world = f.origin + Vec3::X * f.size * 0.6;
        let (p0, _) = v.project(start_world).unwrap();
        let mut drag = GizmoDrag::begin(GizmoMode::Translate, Handle::Axis(Axis3::X), f, &v, p0).unwrap();
        // arrastar até o ponto que está 1,5 m adiante no eixo X
        let target = start_world + Vec3::X * 1.5;
        let (p1, _) = v.project(target).unwrap();
        let Some(DragDelta::Translate(d)) = drag.update(&v, p1) else { panic!("sem delta") };
        assert!((d - Vec3::X * 1.5).length() < 0.02, "{d:?}");
        // voltar ao ponto inicial zera (sem deriva)
        let Some(DragDelta::Translate(d0)) = drag.update(&v, p0) else { panic!() };
        assert!(d0.length() < 1e-3, "{d0:?}");
    }

    #[test]
    fn plane_drag_moves_in_the_plane_only() {
        let v = view();
        let f = frame(&v);
        let q = f.plane_quad(Axis3::Y);
        let c = (q[0] + q[2]) * 0.5;
        let (p0, _) = v.project(c).unwrap();
        let mut drag = GizmoDrag::begin(GizmoMode::Translate, Handle::Plane(Axis3::Y), f, &v, p0).unwrap();
        let (p1, _) = v.project(c + Vec3::new(1.0, 0.0, -2.0)).unwrap();
        let Some(DragDelta::Translate(d)) = drag.update(&v, p1) else { panic!() };
        assert!(d.y.abs() < 1e-4, "{d:?}");
        assert!((d.x - 1.0).abs() < 0.02 && (d.z + 2.0).abs() < 0.02, "{d:?}");
    }

    #[test]
    fn center_drag_moves_in_the_screen_plane() {
        let v = view();
        let f = frame(&v);
        let (o, _) = v.project(f.origin).unwrap();
        let mut drag = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, f, &v, o).unwrap();
        let Some(DragDelta::Translate(d)) = drag.update(&v, o + Vec2::new(60.0, 0.0)) else { panic!() };
        assert!(d.dot(v.forward).abs() < 1e-3, "sem componente de profundidade");
        assert!(d.dot(v.right) > 0.0);
    }

    #[test]
    fn ring_drag_recovers_the_angle_and_unwraps_past_pi() {
        let v = view();
        let f = frame(&v);
        let (u, w) = f.ring_basis(Axis3::Y);
        let r = f.size * 0.86;
        let at = |a: f32| f.origin + (u * a.cos() + w * a.sin()) * r;
        let (p0, _) = v.project(at(0.2)).unwrap();
        let mut drag = GizmoDrag::begin(GizmoMode::Rotate, Handle::Ring(Axis3::Y), f, &v, p0).unwrap();
        let mut last = 0.0;
        // varre 200° em passos de 20° (cruza ±π do atan2)
        for step in 1..=10 {
            let a = 0.2 + (step as f32) * 20f32.to_radians();
            let (p, _) = v.project(at(a)).unwrap();
            let Some(DragDelta::Rotate { axis, angle }) = drag.update(&v, p) else { panic!() };
            assert!((axis - f.axis(Axis3::Y)).length() < 1e-5);
            last = angle;
        }
        assert!((last.to_degrees() - 200.0).abs() < 3.0, "{}", last.to_degrees());
    }

    #[test]
    fn scale_drag_gives_ratio_of_distances_and_uniform_from_center() {
        let v = view();
        let f = frame(&v);
        let tip = f.origin + Vec3::X * f.size;
        let (p0, _) = v.project(tip).unwrap();
        let mut drag = GizmoDrag::begin(GizmoMode::Scale, Handle::Axis(Axis3::X), f, &v, p0).unwrap();
        let (p1, _) = v.project(f.origin + Vec3::X * f.size * 2.0).unwrap();
        let Some(DragDelta::Scale { factors, uniform }) = drag.update(&v, p1) else { panic!() };
        assert!(!uniform && (factors.x - 2.0).abs() < 0.05 && factors.y == 1.0 && factors.z == 1.0, "{factors:?}");

        let (o, _) = v.project(f.origin).unwrap();
        let start = o + Vec2::new(40.0, 0.0);
        let mut c = GizmoDrag::begin(GizmoMode::Scale, Handle::Center, f, &v, start).unwrap();
        let Some(DragDelta::Scale { factors, uniform }) = c.update(&v, o + Vec2::new(80.0, 0.0)) else { panic!() };
        assert!(uniform && (factors.x - 2.0).abs() < 1e-3 && factors.x == factors.y && factors.y == factors.z);
    }

    fn node(id: &str, pos: Vec3, parent: Mat4) -> NodeXf {
        let local = Transform::from_translation(pos);
        NodeXf { id: NodeId::from_slug(id), parent_world: parent, start_local: local, start_world: parent * local.to_matrix() }
    }

    #[test]
    fn translate_in_a_rotated_parent_moves_the_world_position_exactly() {
        let parent = Mat4::from_rotation_translation(Quat::from_rotation_y(0.9), Vec3::new(3.0, 0.0, 1.0));
        let n = node("a", Vec3::new(1.0, 0.0, 0.0), parent);
        let before = n.world_pos();
        let out = apply_translate(&[n.clone()], Vec3::new(0.0, 0.0, 2.0));
        let t = out[0].translation.unwrap();
        let after = parent.transform_point3(t);
        assert!((after - (before + Vec3::new(0.0, 0.0, 2.0))).length() < 1e-4);
        assert!(out[0].rotation.is_none() && out[0].scale.is_none(), "só a translação entra no comando");
    }

    #[test]
    fn rotate_a_group_orbits_around_the_pivot() {
        let a = node("a", Vec3::new(2.0, 0.0, 0.0), Mat4::IDENTITY);
        let b = node("b", Vec3::new(4.0, 0.0, 0.0), Mat4::IDENTITY);
        let pivot = Vec3::new(3.0, 0.0, 0.0);
        let out = apply_rotate(&[a, b], pivot, Vec3::Y, std::f32::consts::PI);
        // 180° em torno de Y: x=2 ↔ x=4
        assert!((out[0].translation.unwrap() - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-4);
        assert!((out[1].translation.unwrap() - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4);
        let q = out[0].rotation.unwrap();
        assert!((q * Vec3::X - Vec3::NEG_X).length() < 1e-4);
        // com pai transformado o resultado mundial é o mesmo
        let parent = Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::from_rotation_y(0.5), Vec3::new(1.0, 2.0, 3.0));
        let n = node("c", Vec3::new(1.0, 0.0, 0.0), parent);
        let p = n.world_pos();
        let out = apply_rotate(&[n.clone()], p, Vec3::Y, 0.4);
        let local = Transform { translation: out[0].translation.unwrap(), rotation: out[0].rotation.unwrap(), scale: n.start_local.scale };
        let world = parent * local.to_matrix();
        assert!((world.transform_point3(Vec3::ZERO) - p).length() < 1e-4, "girar no próprio pivô não move o nó");
    }

    #[test]
    fn scale_applies_per_local_axis_and_uniform_scales_the_layout_too() {
        let a = node("a", Vec3::new(1.0, 0.0, 0.0), Mat4::IDENTITY);
        let out = apply_scale(&[a.clone()], a.world_pos(), Vec3::new(2.0, 1.0, 1.0), false);
        assert_eq!(out[0].scale.unwrap(), Vec3::new(2.0, 1.0, 1.0));
        assert!(out[0].translation.is_none());
        let b = node("b", Vec3::new(3.0, 0.0, 0.0), Mat4::IDENTITY);
        let pivot = Vec3::new(2.0, 0.0, 0.0);
        let out = apply_scale(&[a, b], pivot, Vec3::splat(2.0), true);
        assert!((out[0].translation.unwrap().x - 0.0).abs() < 1e-5, "(1-2)*2+2 = 0");
        assert!((out[1].translation.unwrap().x - 4.0).abs() < 1e-5, "(3-2)*2+2 = 4");
    }

    #[test]
    fn cycle_visits_translate_rotate_scale() {
        let mut m = GizmoMode::Select;
        let mut seen = vec![];
        for _ in 0..4 {
            m = m.cycle();
            seen.push(m);
        }
        assert_eq!(seen, [GizmoMode::Translate, GizmoMode::Rotate, GizmoMode::Scale, GizmoMode::Translate]);
    }
}
