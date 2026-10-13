//! Matemática da câmera de render (pura): enquadrar caixas respeitando o aspecto da SAÍDA, focal ↔ FOV por sensor,
//! navegação (órbita/pan/dolly) sobre (olho, alvo) e o retângulo útil da imagem dentro da viewport (letterbox + margens).

use glam::Vec3;

/// Sensores de referência (nome, largura mm, altura mm). A altura define o FOV vertical.
pub const SENSORS: [(&str, f32, f32); 5] = [
    ("Super35 (24,89 x 18,66)", 24.89, 18.66),
    ("Full-frame (36 x 24)", 36.0, 24.0),
    ("APS-C (23,6 x 15,6)", 23.6, 15.6),
    ("16:9 (24 x 13,5)", 24.0, 13.5),
    ("Micro 4/3 (17,3 x 13)", 17.3, 13.0),
];

/// Distância focal (mm) que dá o FOV vertical `fov_rad` num sensor de altura `sensor_h_mm`.
pub fn focal_from_fov(fov_rad: f32, sensor_h_mm: f32) -> f32 {
    sensor_h_mm / (2.0 * (fov_rad * 0.5).tan().max(1e-6))
}

/// FOV vertical (rad) de uma focal `focal_mm` num sensor de altura `sensor_h_mm`.
pub fn fov_from_focal(focal_mm: f32, sensor_h_mm: f32) -> f32 {
    2.0 * (sensor_h_mm / (2.0 * focal_mm.max(0.01))).atan()
}

/// Novo (olho, alvo) que enquadra a caixa `[lo, hi]` mantendo a DIREÇÃO de olhar `forward`: os 8 cantos precisam caber
/// na vertical (`fov_y`) e na horizontal (`fov_y` + `aspect` da saída) — ajuste justo, não pela esfera envolvente.
/// `margin` > 1 deixa folga (distância × margin). O alvo é o centro da caixa.
pub fn frame_bounds(forward: Vec3, lo: Vec3, hi: Vec3, fov_y: f32, aspect: f32, margin: f32) -> (Vec3, Vec3) {
    let center = (lo + hi) * 0.5;
    let half = (hi - lo) * 0.5;
    let f = if forward.length_squared() > 1e-12 { forward.normalize() } else { Vec3::NEG_Z };
    let mut right = f.cross(Vec3::Y);
    if right.length_squared() < 1e-8 {
        right = f.cross(Vec3::Z);
    }
    let right = right.normalize();
    let up = right.cross(f).normalize();
    let ty = (fov_y * 0.5).tan().max(1e-4);
    let tx = ty * aspect.max(0.05);
    // cada canto c (relativo ao centro) tem profundidade d + c·f: |c·up| <= ty (d + c·f) e |c·right| <= tx (d + c·f)
    let mut dist = 0.05f32;
    for sx in [-1.0f32, 1.0] {
        for sy in [-1.0f32, 1.0] {
            for sz in [-1.0f32, 1.0] {
                let c = Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                let (x, y, z) = (c.dot(right).abs(), c.dot(up).abs(), c.dot(f));
                dist = dist.max(y / ty - z).max(x / tx - z);
            }
        }
    }
    let dist = (dist * margin.max(1.0)).max(0.05);
    (center - f * dist, center)
}

/// Órbita em torno do alvo: gira `d_yaw` (em torno de Y mundial) e `d_pitch` (em torno do eixo direito), mantendo a distância.
pub fn orbit(eye: Vec3, target: Vec3, d_yaw: f32, d_pitch: f32) -> Vec3 {
    let v = eye - target;
    let dist = v.length().max(1e-4);
    let yaw = v.x.atan2(v.z) + d_yaw;
    let pitch = (v.y / dist).clamp(-1.0, 1.0).asin() + d_pitch;
    let pitch = pitch.clamp(-1.5, 1.5);
    target + Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()) * dist
}

/// Pan no plano da imagem: `dx`/`dy` em unidades de mundo (direita / cima da câmera).
pub fn pan(eye: Vec3, target: Vec3, dx: f32, dy: f32) -> (Vec3, Vec3) {
    let forward = (target - eye).normalize_or_zero();
    let right = forward.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(forward).normalize_or_zero();
    let delta = right * dx + up * dy;
    (eye + delta, target + delta)
}

/// Dolly: aproxima/afasta o olho do alvo por `factor` (1,1 = 10 % mais longe).
pub fn dolly(eye: Vec3, target: Vec3, factor: f32) -> Vec3 {
    let v = eye - target;
    target + v * factor.clamp(0.05, 20.0)
}

/// Retângulo (x, y, w, h) da imagem de aspecto `aspect` centrado em `outer` (x, y, w, h), com `pad` de cada lado.
pub fn fit_rect(outer: (f32, f32, f32, f32), aspect: f32, pad: f32) -> (f32, f32, f32, f32) {
    let (ox, oy, ow, oh) = (outer.0 + pad, outer.1 + pad, (outer.2 - 2.0 * pad).max(1.0), (outer.3 - 2.0 * pad).max(1.0));
    let aspect = aspect.max(0.01);
    let (w, h) = if ow / oh > aspect { (oh * aspect, oh) } else { (ow, ow / aspect) };
    (ox + (ow - w) * 0.5, oy + (oh - h) * 0.5, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focal_and_fov_are_inverse_and_match_known_lenses() {
        // 35 mm em sensor Super35 (18,66 mm de altura): FOV vertical ≈ 29,9°
        let fov = fov_from_focal(35.0, 18.66);
        assert!((fov.to_degrees() - 29.9).abs() < 0.2, "{}", fov.to_degrees());
        assert!((focal_from_fov(fov, 18.66) - 35.0).abs() < 1e-3);
        // 50 mm em full-frame (24 mm de altura): ≈ 27°
        assert!((fov_from_focal(50.0, 24.0).to_degrees() - 26.99).abs() < 0.1);
    }

    #[test]
    fn framing_fits_the_box_corners_in_both_axes_for_wide_and_tall_outputs() {
        let (lo, hi) = (Vec3::new(-2.0, 0.0, -1.0), Vec3::new(2.0, 3.0, 1.0));
        for aspect in [16.0 / 9.0, 0.71, 1.0] {
            let fov = 40f32.to_radians();
            let (eye, target) = frame_bounds(Vec3::new(0.3, -0.2, -1.0), lo, hi, fov, aspect, 1.0);
            // ajuste justo: algum canto toca a borda (nenhuma folga desperdiçada)
            let touch = {
                let view = glam::Mat4::look_at_rh(eye, target, Vec3::Y);
                let proj = glam::Mat4::perspective_rh(fov, aspect, 0.05, 100.0);
                let mut m = 0.0f32;
                for x in [lo.x, hi.x] {
                    for y in [lo.y, hi.y] {
                        for z in [lo.z, hi.z] {
                            let c = proj * view * glam::Vec4::new(x, y, z, 1.0);
                            m = m.max((c.x / c.w).abs()).max((c.y / c.w).abs());
                        }
                    }
                }
                m
            };
            assert!(touch > 0.97 && touch <= 1.0 + 1e-3, "aspect {aspect}: o canto mais externo fica em {touch}");
            let view = glam::Mat4::look_at_rh(eye, target, Vec3::Y);
            let proj = glam::Mat4::perspective_rh(fov, aspect, 0.05, 100.0);
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let c = proj * view * glam::Vec4::new(x, y, z, 1.0);
                        let (nx, ny) = (c.x / c.w, c.y / c.w);
                        assert!(nx.abs() <= 1.0 + 1e-3 && ny.abs() <= 1.0 + 1e-3, "aspect {aspect}: canto ({x},{y},{z}) -> ({nx:.3},{ny:.3})");
                    }
                }
            }
            // e a direção de olhar foi preservada
            let dir = (target - eye).normalize();
            assert!(dir.dot(Vec3::new(0.3, -0.2, -1.0).normalize()) > 0.9999);
        }
    }

    #[test]
    fn orbit_keeps_distance_pan_keeps_direction_dolly_scales() {
        let (eye, target) = (Vec3::new(0.0, 1.5, 5.0), Vec3::new(0.0, 1.0, 0.0));
        let e2 = orbit(eye, target, 0.7, 0.2);
        assert!(((e2 - target).length() - (eye - target).length()).abs() < 1e-4);
        assert!((e2 - eye).length() > 1.0);
        let (pe, pt) = pan(eye, target, 1.0, 0.5);
        assert!(((pt - pe) - (target - eye)).length() < 1e-5, "o pan não gira a câmera");
        assert!(((pe - eye).length() - (1.0f32 + 0.25).sqrt()).abs() < 1e-3);
        let d = dolly(eye, target, 2.0);
        assert!(((d - target).length() - 2.0 * (eye - target).length()).abs() < 1e-4);
        // o pitch nunca passa dos polos
        let top = orbit(eye, target, 0.0, 10.0);
        assert!(top.is_finite() && (top.y - target.y) < (top - target).length() * 1.0001);
    }

    #[test]
    fn fit_rect_letterboxes_inside_the_panel() {
        let (x, y, w, h) = fit_rect((0.0, 0.0, 1000.0, 600.0), 16.0 / 9.0, 20.0);
        assert!((w / h - 16.0 / 9.0).abs() < 1e-3);
        assert!(x >= 20.0 && y >= 20.0 && x + w <= 980.0 + 1e-3 && y + h <= 580.0 + 1e-3);
        let (x, _, w, h) = fit_rect((0.0, 0.0, 1000.0, 600.0), 0.5, 0.0);
        assert!((h - 600.0).abs() < 1e-3 && (w - 300.0).abs() < 1e-3 && (x - 350.0).abs() < 1e-3);
    }
}
