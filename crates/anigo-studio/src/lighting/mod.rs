//! Núcleo puro do workspace Iluminação (sem egui): direção da luz ↔ azimute/elevação, temperatura de cor,
//! luz relativa à câmera, escolha de direção sobre a esfera do gizmo, geometria do gizmo e catálogo de rigs.
//!
//! A UI (`app::lighting_ui`) só costura estes módulos e emite `Command`s do núcleo; toda a matemática
//! que decide "para onde aponta a luz" mora aqui e tem testes unitários.
//!
//! Convenção (a mesma do Cenário e do núcleo): a direção `L` aponta DA superfície PARA a luz.
//! Azimute 0° = luz vinda de +Z (a frente do personagem, o lado da câmera padrão), positivo para +X
//! (a direita da tela vista de frente); elevação 0° = horizonte, +90° = zênite.

pub mod gizmo;
pub mod rigs;

use crate::viewport::OrbitCamera;
use glam::Vec3;

/// Direção `L` (unitária) para azimute/elevação em graus.
pub fn dir_from_az_el(az_deg: f32, el_deg: f32) -> Vec3 {
    let (a, e) = (az_deg.to_radians(), el_deg.to_radians());
    Vec3::new(e.cos() * a.sin(), e.sin(), e.cos() * a.cos())
}

/// Azimute (−180..180] e elevação [−90, 90] em graus de uma direção. Perto dos polos
/// (|elevação| > 89,9°) o azimute é indefinido: devolve `prev_az` para o controle não "pular".
pub fn az_el_from_dir(dir: Vec3, prev_az: f32) -> (f32, f32) {
    let d = dir.normalize_or_zero();
    if d == Vec3::ZERO {
        return (prev_az, 0.0);
    }
    let el = d.y.clamp(-1.0, 1.0).asin().to_degrees();
    if el.abs() > 89.9 {
        return (prev_az, el);
    }
    (d.x.atan2(d.z).to_degrees(), el)
}

/// Arredonda azimute/elevação a múltiplos de `step` graus (Shift durante o arrasto).
pub fn snap_az_el(az: f32, el: f32, step: f32) -> (f32, f32) {
    if step <= 0.0 {
        return (az, el);
    }
    let q = |v: f32| (v / step).round() * step;
    (q(az), q(el).clamp(-90.0, 90.0))
}

/// Faixa de temperatura do algoritmo de Tanner Helland (K).
pub const KELVIN_RANGE: (f32, f32) = (1000.0, 40000.0);

/// Cor RGB (sRGB 0..1) de um corpo negro de `kelvin` K — algoritmo de Tanner Helland
/// (https://tannerhelland.com/2012/09/18/convert-temperature-rgb-algorithm-code.html, válido de 1000 a 40000 K).
/// É uma aproximação de uso artístico, não colorimetria.
pub fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    let t = kelvin.clamp(KELVIN_RANGE.0, KELVIN_RANGE.1) / 100.0;
    let clamp255 = |v: f32| v.clamp(0.0, 255.0);
    let r = if t <= 66.0 { 255.0 } else { clamp255(329.698_73 * (t - 60.0).powf(-0.133_204_76)) };
    let g = if t <= 66.0 { clamp255(99.470_8 * t.ln() - 161.119_57) } else { clamp255(288.122_17 * (t - 60.0).powf(-0.075_514_85)) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        clamp255(138.517_73 * (t - 10.0).ln() - 305.044_8)
    };
    [r / 255.0, g / 255.0, b / 255.0]
}

/// Direção `L` para (azimute, elevação) RELATIVOS à câmera: azimute 0 = a luz está na câmera
/// (iluminação frontal chapada), positivo = à direita da câmera; elevação positiva = acima.
pub fn camera_relative_dir(cam: &OrbitCamera, az_deg: f32, el_deg: f32) -> Vec3 {
    let (a, e) = (az_deg.to_radians(), el_deg.to_radians());
    let back = -cam.forward();
    (cam.right() * (e.cos() * a.sin()) + cam.up() * e.sin() + back * (e.cos() * a.cos())).normalize_or_zero()
}

/// Inverso de [`camera_relative_dir`]: ângulos relativos à câmera de uma direção de mundo.
pub fn camera_relative_angles(cam: &OrbitCamera, dir: Vec3, prev_az: f32) -> (f32, f32) {
    let d = dir.normalize_or_zero();
    let local = Vec3::new(d.dot(cam.right()), d.dot(cam.up()), d.dot(-cam.forward()));
    az_el_from_dir_local(local, prev_az)
}

fn az_el_from_dir_local(local: Vec3, prev_az: f32) -> (f32, f32) {
    // mesma fórmula de `az_el_from_dir` em coordenadas (x = direita, y = cima, z = trás)
    az_el_from_dir(local, prev_az)
}

/// Direção de câmera-relativa padrão de "luz de estúdio": esquerda/acima/frente (a do preview do Personagem).
pub const STUDIO_REL: (f32, f32) = (-35.0, 37.0);

/// Resultado de [`pick_on_sphere`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpherePick {
    /// Direção unitária do centro para o ponto escolhido.
    pub dir: Vec3,
    /// `true` quando o raio errou a esfera (o ponto é o da silhueta mais próximo).
    pub outside: bool,
}

/// Direção escolhida por um raio do mouse sobre a esfera do gizmo: a interseção do hemisfério voltado
/// para a câmera (`far = false`) ou do oposto (`far = true`); com o raio fora da esfera, o ponto da
/// silhueta mais próximo (a luz fica "presa" à borda enquanto o ponteiro estiver fora).
pub fn pick_on_sphere(origin: Vec3, ray_dir: Vec3, center: Vec3, radius: f32, far: bool) -> SpherePick {
    let d = ray_dir.normalize_or_zero();
    let oc = origin - center;
    let b = oc.dot(d);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - c;
    if disc >= 0.0 {
        let s = disc.sqrt();
        let t = if far { -b + s } else { -b - s };
        let p = origin + d * t;
        return SpherePick { dir: (p - center).normalize_or_zero(), outside: false };
    }
    // fora: ponto do raio mais próximo do centro, levado à esfera
    let closest = origin + d * (-b);
    let v = (closest - center).normalize_or_zero();
    SpherePick { dir: if v == Vec3::ZERO { Vec3::Y } else { v }, outside: true }
}

/// Máquina de estado do hemisfério durante um arrasto: ao sair da esfera e voltar, inverte o hemisfério
/// (é assim que se leva a luz para trás do personagem arrastando sobre a silhueta).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcballHemisphere {
    pub far: bool,
    pub was_outside: bool,
}

impl ArcballHemisphere {
    /// Começa no hemisfério em que a luz está agora: `facing` = `dot(L, eixo do centro para a câmera)`.
    pub fn starting_at(facing: f32) -> Self {
        Self { far: facing < 0.0, was_outside: false }
    }

    /// Atualiza com o resultado do quadro e devolve o `far` a usar.
    pub fn step(&mut self, outside: bool) -> bool {
        if !outside && self.was_outside {
            self.far = !self.far;
        }
        self.was_outside = outside;
        self.far
    }
}

/// Raio (m) da esfera do gizmo para ocupar `px` pixels de tela a `world_per_pixel` m/px.
pub fn gizmo_radius(world_per_pixel: f32, px: f32) -> f32 {
    (world_per_pixel * px).clamp(0.05, 200.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn az_el_round_trips_and_matches_the_core_convention() {
        for az in (-170..=180).step_by(10) {
            for el in (-80..=80).step_by(10) {
                let d = dir_from_az_el(az as f32, el as f32);
                assert!(approx(d.length(), 1.0));
                let (a, e) = az_el_from_dir(d, 0.0);
                assert!(approx(e, el as f32), "el {el} → {e}");
                let da = ((a - az as f32 + 540.0) % 360.0) - 180.0;
                assert!(da.abs() < 1e-2, "az {az} → {a}");
            }
        }
        // convenções: az 0 = +Z (frente), az 90 = +X, el 90 = +Y
        assert!((dir_from_az_el(0.0, 0.0) - Vec3::Z).length() < 1e-5);
        assert!((dir_from_az_el(90.0, 0.0) - Vec3::X).length() < 1e-5);
        assert!((dir_from_az_el(0.0, 90.0) - Vec3::Y).length() < 1e-5);
        // a mesma fórmula que o Details do Cenário já usava
        let (a, e) = (30f32.to_radians(), 40f32.to_radians());
        let scene = Vec3::new(e.cos() * a.sin(), e.sin(), e.cos() * a.cos());
        assert!((dir_from_az_el(30.0, 40.0) - scene).length() < 1e-6);
    }

    #[test]
    fn az_is_kept_near_the_poles() {
        let (a, e) = az_el_from_dir(Vec3::Y, 42.0);
        assert_eq!(a, 42.0);
        assert!(approx(e, 90.0));
        assert_eq!(az_el_from_dir(Vec3::ZERO, 7.0), (7.0, 0.0));
    }

    #[test]
    fn snapping_rounds_to_the_step() {
        assert_eq!(snap_az_el(37.0, 12.0, 15.0), (30.0, 15.0));
        assert_eq!(snap_az_el(-52.0, 99.0, 5.0), (-50.0, 90.0));
        assert_eq!(snap_az_el(10.0, 10.0, 0.0), (10.0, 10.0));
    }

    #[test]
    fn kelvin_matches_the_published_algorithm() {
        // valores de referência do algoritmo de Tanner Helland (sRGB 0..255)
        let c = |k: f32| kelvin_to_rgb(k).map(|v| (v * 255.0).round() as i32);
        assert_eq!(c(6600.0), [255, 255, 255]);
        let warm = c(3000.0);
        assert_eq!(warm[0], 255);
        assert!(warm[1] > 150 && warm[1] < 200, "{warm:?}");
        assert!(warm[2] > 60 && warm[2] < 120, "{warm:?}");
        let cool = c(10000.0);
        assert_eq!(cool[2], 255);
        assert!(cool[0] < 220 && cool[0] > 180, "{cool:?}");
        // quente é mais vermelho que frio; clamp fora da faixa
        assert!(kelvin_to_rgb(2000.0)[2] < kelvin_to_rgb(9000.0)[2]);
        assert_eq!(kelvin_to_rgb(1.0), kelvin_to_rgb(1000.0));
        assert_eq!(kelvin_to_rgb(1e9), kelvin_to_rgb(40000.0));
        assert_eq!(c(1500.0)[2], 0, "abaixo de 1900 K não há azul");
    }

    #[test]
    fn camera_relative_light_follows_the_camera_and_round_trips() {
        let mut cam = OrbitCamera::default();
        for yaw in [0.0f32, 0.9, -2.0, 3.0] {
            cam.yaw = yaw;
            cam.pitch = 0.3;
            // az 0, el 0 = a luz está NA câmera
            let d = camera_relative_dir(&cam, 0.0, 0.0);
            assert!((d - (-cam.forward())).length() < 1e-5);
            // az +90 = à direita da câmera
            assert!((camera_relative_dir(&cam, 90.0, 0.0) - cam.right()).length() < 1e-5);
            let want = (-35.0, 37.0);
            let dir = camera_relative_dir(&cam, want.0, want.1);
            let (a, e) = camera_relative_angles(&cam, dir, 0.0);
            assert!(approx(a, want.0) && approx(e, want.1), "yaw {yaw}: {a},{e}");
        }
    }

    #[test]
    fn sphere_pick_chooses_the_requested_hemisphere() {
        let center = Vec3::new(0.0, 1.0, 0.0);
        let eye = Vec3::new(0.0, 1.0, 6.0);
        let near = pick_on_sphere(eye, -Vec3::Z, center, 1.5, false);
        assert!(!near.outside);
        assert!((near.dir - Vec3::Z).length() < 1e-5, "{:?}", near.dir);
        let far = pick_on_sphere(eye, -Vec3::Z, center, 1.5, true);
        assert!((far.dir + Vec3::Z).length() < 1e-5, "{:?}", far.dir);
        // raio que passa fora: silhueta (perpendicular ao raio), marcado como fora
        let out = pick_on_sphere(eye + Vec3::X * 3.0, -Vec3::Z, center, 1.5, false);
        assert!(out.outside);
        assert!(out.dir.dot(Vec3::Z).abs() < 1e-5);
        assert!(out.dir.x > 0.99);
    }

    #[test]
    fn arcball_flips_the_hemisphere_after_crossing_the_silhouette() {
        let mut h = ArcballHemisphere::starting_at(0.6); // luz voltada para a câmera
        assert!(!h.far);
        assert!(!h.step(false));
        assert!(!h.step(true)); // saiu: ainda o mesmo hemisfério
        assert!(h.step(false)); // voltou: inverteu
        assert!(h.step(false));
        assert!(h.step(true));
        assert!(!h.step(false)); // inverteu de novo
        assert!(ArcballHemisphere::starting_at(-0.2).far);
    }

    #[test]
    fn gizmo_radius_is_clamped() {
        assert!(approx(gizmo_radius(0.01, 100.0), 1.0));
        assert_eq!(gizmo_radius(0.0, 100.0), 0.05);
        assert_eq!(gizmo_radius(100.0, 100.0), 200.0);
    }
}
