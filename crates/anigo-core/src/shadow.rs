//! Sombra projetada da luz principal (shadow map de 1 luz) — matemática pura.
//!
//! Este módulo concentra o que o renderer faz na CPU (a matriz ortográfica da
//! luz, ajustada ao conteúdo e estável quando a câmera se move) e a matemática
//! que o WGSL (`cel_shading.wgsl`, bloco "Sombra projetada") executa na GPU.
//!
//! ## Equivalência com o shader
//!
//! | Rust (`anigo_core::shadow`)  | WGSL (`cel_shading.wgsl`)      |
//! |------------------------------|--------------------------------|
//! | `receiver_position`          | `shadow_receiver_pos`          |
//! | `shadow_step`                | `shadow_step`                  |
//! | `project`                    | `shadow_project`               |
//! | `pcf_radius_texels`, `step_half_width` | calculados na CPU e enviados no uniform |
//! | `fit_directional`            | só Rust (CPU): o shader recebe a matriz |
//!
//! Cada função pura tem teste aqui e um teste de paridade na GPU
//! (`crates/anigo-renderer/tests/shader_math_parity.rs`) que extrai o TEXTO das
//! funções do shader, executa num compute e compara com estas.
//!
//! Convenção: `direction` da luz aponta DA superfície PARA a luz (a mesma de
//! `StylizedLight::direction` e do `L` do shader). Profundidade da luz em
//! `[0, 1]` (convenção wgpu).

use crate::scene::LightShadow;
use glam::{Mat4, Vec3};

/// Matrizes e métricas do mapa de sombra de um quadro.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightFit {
    /// `projeção_ortográfica × vista_da_luz`: mundo → clip da luz (z em 0..1).
    pub view_proj: Mat4,
    /// Tamanho de um texel do mapa em metros de mundo.
    pub world_per_texel: f32,
    /// Extensão em profundidade (metros) coberta pelo mapa (near=0 .. far).
    pub depth_range: f32,
    /// Raio (m) da esfera ajustada.
    pub radius: f32,
    /// Centro (m) da esfera ajustada (já com snap de texel).
    pub center: Vec3,
}

/// Base ortonormal (direita, cima) do plano perpendicular a `l` (unitário).
fn light_basis(l: Vec3) -> (Vec3, Vec3) {
    let up_hint = if l.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    // a vista olha de `center + l*d` para `center` (direção −l)
    let right = up_hint.cross(l).normalize_or_zero();
    let up = l.cross(right).normalize_or_zero();
    (right, up)
}

/// Ajusta uma projeção ORTOGRÁFICA da luz a um conjunto de pontos de mundo (os
/// vértices de uma caixa por nó visível, ver o renderer), com:
///
/// * **esfera envolvente** (raio constante sob rotação da luz: o tamanho do texel não oscila);
/// * **snap de texel**: o centro é arredondado a múltiplos do texel no plano da luz, então a
///   borda da sombra não cintila quando a câmera/objeto se move;
/// * `focus = Some((centro, raio))` limita a cobertura (distância de sombra): só essa esfera
///   recebe resolução, mas todo ponto de `points` na direção da luz ainda ENTRA no mapa como
///   projetor (o olho da luz recua até o ponto mais próximo da luz).
///
/// Devolve `None` sem pontos finitos.
pub fn fit_directional(direction: [f32; 3], points: &[Vec3], focus: Option<(Vec3, f32)>, resolution: u32) -> Option<LightFit> {
    let l = Vec3::from_array(direction).normalize_or_zero();
    if l == Vec3::ZERO || resolution == 0 {
        return None;
    }
    let pts: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.is_empty() {
        return None;
    }
    // esfera dos pontos (centro da caixa, raio até o canto mais distante)
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in &pts {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let content_center = (lo + hi) * 0.5;
    let content_radius = pts.iter().map(|p| p.distance(content_center)).fold(0.0_f32, f32::max).max(0.05);
    let (center, radius) = match focus {
        Some((c, r)) if r > 0.0 && r < content_radius && c.is_finite() => (c, r),
        _ => (content_center, content_radius),
    };
    // pequena folga para a esfera não encostar na borda do mapa (PCF/normal offset)
    let radius = (radius * 1.04 + 0.02).max(0.1);
    let texel = 2.0 * radius / resolution as f32;
    // snap do centro, no plano da luz, a múltiplos do texel
    let (right, up) = light_basis(l);
    let (cx, cy) = (center.dot(right), center.dot(up));
    let snapped = center + right * ((cx / texel).round() * texel - cx) + up * ((cy / texel).round() * texel - cy);
    // o olho recua até o projetor mais próximo da luz (nunca menos que o raio)
    let toward_light = pts.iter().map(|p| (*p - snapped).dot(l)).fold(f32::NEG_INFINITY, f32::max).max(radius);
    let eye_distance = toward_light + 0.05;
    let eye = snapped + l * eye_distance;
    let view = Mat4::look_at_rh(eye, snapped, up);
    let depth_range = eye_distance + radius + 0.05;
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, depth_range);
    Some(LightFit { view_proj: proj * view, world_per_texel: texel, depth_range, radius, center: snapped })
}

/// Projeta um ponto de mundo no mapa: `(u, v, profundidade)`, com `u,v` em 0..1 (origem no
/// canto superior esquerdo, v para baixo, como a textura) e profundidade 0..1.
/// WGSL: `shadow_project`. A projeção da luz é ortográfica (w = 1).
pub fn project(view_proj: &Mat4, world: Vec3) -> Vec3 {
    let clip = *view_proj * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    Vec3::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z)
}

/// Posição do receptor deslocada ao longo da normal (offset de normal): a quantidade
/// cresce com a inclinação (`1 − N·L`), onde a acne nasce. WGSL: `shadow_receiver_pos`.
pub fn receiver_position(world: Vec3, normal: Vec3, light: Vec3, normal_bias_world: f32) -> Vec3 {
    let slope = (1.0 - normal.dot(light)).clamp(0.0, 1.0);
    world + normal * (normal_bias_world * slope)
}

/// Degrau estilizado da oclusão filtrada (0 = iluminado, 1 = na sombra) em torno de 0,5:
/// `smoothstep(0.5 − w, 0.5 + w, occlusion)`. WGSL: `shadow_step`.
pub fn shadow_step(occlusion: f32, half_width: f32) -> f32 {
    let w = half_width.max(0.001);
    let t = ((occlusion - (0.5 - w)) / (2.0 * w)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Raio do filtro PCF em texels para a dureza (1 = duro: 1 texel; 0 = suave: 4 texels).
pub fn pcf_radius_texels(hardness: f32) -> f32 {
    let h = hardness.clamp(0.0, 1.0);
    4.0 + (1.0 - 4.0) * h
}

/// Meia-largura do degrau para a dureza (1 = 0,05; 0 = 0,5).
pub fn step_half_width(hardness: f32) -> f32 {
    let h = hardness.clamp(0.0, 1.0);
    0.5 + (0.05 - 0.5) * h
}

/// Conteúdo do `ShadowUniform` (96 B) do shader, na ordem do contrato:
/// `light_view_proj` (16 floats) + `params` + `params2`.
///
/// * `params  = (enabled, strength, bias NDC, normal_bias em metros)`
/// * `params2 = (1/resolução, raio PCF em texels, meia-largura do degrau, 0)`
pub fn uniform_floats(shadow: &LightShadow, fit: Option<&LightFit>) -> [f32; 24] {
    let s = shadow.sanitized();
    let mut out = [0.0_f32; 24];
    let Some(fit) = fit.filter(|_| s.enabled) else {
        // desligado/sem ajuste: matriz identidade e `enabled = 0` (o shader não amostra nada)
        out[..16].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
        out[20] = 1.0 / s.resolution as f32;
        return out;
    };
    out[..16].copy_from_slice(&fit.view_proj.to_cols_array());
    out[16] = 1.0;
    out[17] = s.strength;
    out[18] = s.bias / fit.depth_range.max(1e-3);
    out[19] = s.normal_bias * fit.world_per_texel;
    out[20] = 1.0 / s.resolution as f32;
    out[21] = pcf_radius_texels(s.hardness);
    out[22] = step_half_width(s.hardness);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_points(c: Vec3, h: f32) -> Vec<Vec3> {
        let mut v = Vec::new();
        for x in [-h, h] {
            for y in [-h, h] {
                for z in [-h, h] {
                    v.push(c + Vec3::new(x, y, z));
                }
            }
        }
        v
    }

    #[test]
    fn fit_contains_every_point_with_depth_in_unit_range() {
        let pts: Vec<Vec3> = cube_points(Vec3::new(0.0, 1.0, 0.0), 1.0).into_iter().chain(cube_points(Vec3::new(4.0, 0.5, -3.0), 0.7)).collect();
        for dir in [[0.5, 0.8, 0.3], [0.0, 1.0, 0.0], [-0.9, 0.1, 0.4], [0.0, -0.5, 1.0]] {
            let fit = fit_directional(dir, &pts, None, 2048).expect("ajuste");
            for p in &pts {
                let t = project(&fit.view_proj, *p);
                assert!((0.0..=1.0).contains(&t.x) && (0.0..=1.0).contains(&t.y), "uv fora do mapa para {dir:?}: {t:?}");
                assert!((0.0..=1.0).contains(&t.z), "profundidade fora de 0..1 para {dir:?}: {t:?}");
            }
        }
    }

    #[test]
    fn fit_is_stable_under_sub_texel_motion() {
        // mover o conteúdo menos que um texel não muda a matriz (snap de texel)
        let base = cube_points(Vec3::new(0.0, 1.0, 0.0), 1.0);
        let dir = [0.4, 0.7, 0.5];
        let a = fit_directional(dir, &base, None, 1024).unwrap();
        let nudge = Vec3::new(a.world_per_texel * 0.1, 0.0, 0.0);
        let moved: Vec<Vec3> = base.iter().map(|p| *p + nudge).collect();
        let b = fit_directional(dir, &moved, None, 1024).unwrap();
        // o raio é o mesmo e o centro só anda em múltiplos do texel
        assert!((a.radius - b.radius).abs() < 1e-5);
        assert!((a.world_per_texel - b.world_per_texel).abs() < 1e-7);
        let (right, up) = light_basis(Vec3::from_array(dir).normalize());
        let d = b.center - a.center;
        for axis in [right, up] {
            let steps = d.dot(axis) / a.world_per_texel;
            assert!((steps - steps.round()).abs() < 1e-2, "centro andou {steps} texels (não inteiro)");
        }
    }

    #[test]
    fn fit_radius_does_not_depend_on_light_direction() {
        let pts = cube_points(Vec3::new(1.0, 1.0, 1.0), 1.5);
        let r0 = fit_directional([0.0, 1.0, 0.0], &pts, None, 2048).unwrap().radius;
        let r1 = fit_directional([0.7, 0.2, -0.3], &pts, None, 2048).unwrap().radius;
        assert!((r0 - r1).abs() < 1e-5, "esfera envolvente deve ter raio constante: {r0} vs {r1}");
    }

    #[test]
    fn focus_shrinks_the_map_and_raises_texel_density() {
        let pts: Vec<Vec3> = cube_points(Vec3::new(0.0, 1.0, 0.0), 1.0).into_iter().chain(cube_points(Vec3::new(20.0, 0.5, 0.0), 2.0)).collect();
        let full = fit_directional([0.3, 0.8, 0.5], &pts, None, 2048).unwrap();
        let focus = fit_directional([0.3, 0.8, 0.5], &pts, Some((Vec3::new(0.0, 1.0, 0.0), 3.0)), 2048).unwrap();
        assert!(focus.radius < full.radius * 0.5, "foco deveria reduzir o raio: {} vs {}", focus.radius, full.radius);
        assert!(focus.world_per_texel < full.world_per_texel * 0.5);
        // o conteúdo focado cabe no mapa
        for p in cube_points(Vec3::new(0.0, 1.0, 0.0), 1.0) {
            let t = project(&focus.view_proj, p);
            assert!((0.0..=1.0).contains(&t.x) && (0.0..=1.0).contains(&t.y) && (0.0..=1.0).contains(&t.z), "{t:?}");
        }
    }

    #[test]
    fn caster_between_light_and_focus_is_inside_the_depth_range() {
        // um projetor FORA do foco, no caminho da luz, ainda tem profundidade válida (>= 0)
        let near_light = Vec3::new(0.0, 12.0, 0.0);
        let pts = vec![Vec3::new(0.0, 0.0, 0.0), near_light];
        let fit = fit_directional([0.0, 1.0, 0.0], &pts, Some((Vec3::ZERO, 1.0)), 1024).unwrap();
        let t = project(&fit.view_proj, near_light);
        assert!(t.z >= 0.0 && t.z < 0.5, "projetor perto da luz deve ter profundidade pequena: {t:?}");
    }

    #[test]
    fn fit_rejects_empty_or_degenerate_input() {
        assert!(fit_directional([0.0, 1.0, 0.0], &[], None, 2048).is_none());
        assert!(fit_directional([0.0, 0.0, 0.0], &[Vec3::ZERO], None, 2048).is_none());
        assert!(fit_directional([0.0, 1.0, 0.0], &[Vec3::NAN], None, 2048).is_none());
    }

    #[test]
    fn golden_project_ortho_identity() {
        // matriz identidade: x,y ∈ [-1,1] → u = x/2+0.5, v = 0.5 − y/2; z passa
        let t = project(&Mat4::IDENTITY, Vec3::new(1.0, -1.0, 0.25));
        assert_eq!(t, Vec3::new(1.0, 1.0, 0.25));
        let t = project(&Mat4::IDENTITY, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(t, Vec3::new(0.5, 0.5, 0.0));
    }

    #[test]
    fn golden_receiver_position_grows_with_slope() {
        let n = Vec3::Y;
        // de frente para a luz (N·L = 1): sem deslocamento
        assert_eq!(receiver_position(Vec3::ZERO, n, Vec3::Y, 0.1), Vec3::ZERO);
        // rasante (N·L = 0): deslocamento total
        assert_eq!(receiver_position(Vec3::ZERO, n, Vec3::X, 0.1), Vec3::new(0.0, 0.1, 0.0));
        // costas (N·L < 0): saturado em 1
        assert_eq!(receiver_position(Vec3::ZERO, n, -Vec3::Y, 0.1), Vec3::new(0.0, 0.1, 0.0));
    }

    #[test]
    fn golden_shadow_step_is_a_centered_smoothstep() {
        assert_eq!(shadow_step(0.0, 0.1), 0.0);
        assert_eq!(shadow_step(1.0, 0.1), 1.0);
        assert!((shadow_step(0.5, 0.1) - 0.5).abs() < 1e-6);
        // mais duro = mais perto de degrau
        assert!(shadow_step(0.55, 0.05) > shadow_step(0.55, 0.4));
        // meia-largura mínima evita divisão por zero
        assert!(shadow_step(0.5, 0.0).is_finite());
    }

    #[test]
    fn hardness_maps_to_documented_ranges() {
        assert_eq!(pcf_radius_texels(1.0), 1.0);
        assert_eq!(pcf_radius_texels(0.0), 4.0);
        assert!((step_half_width(1.0) - 0.05).abs() < 1e-6);
        assert!((step_half_width(0.0) - 0.5).abs() < 1e-6);
        assert!(pcf_radius_texels(0.5) < pcf_radius_texels(0.2));
    }

    #[test]
    fn uniform_is_neutral_when_disabled_and_complete_when_enabled() {
        let off = uniform_floats(&LightShadow::default(), None);
        assert_eq!(off[16], 0.0, "desligado: params.x == 0");
        assert_eq!(&off[..16], &Mat4::IDENTITY.to_cols_array());
        let pts = cube_points(Vec3::new(0.0, 1.0, 0.0), 1.0);
        let fit = fit_directional([0.4, 0.7, 0.5], &pts, None, 2048).unwrap();
        let on = LightShadow { enabled: true, ..LightShadow::default() };
        let u = uniform_floats(&on, Some(&fit));
        assert_eq!(u[16], 1.0);
        assert_eq!(u[17], 1.0);
        assert!(u[18] > 0.0 && u[18] < 0.05, "bias NDC {}", u[18]);
        assert!((u[19] - 1.5 * fit.world_per_texel).abs() < 1e-7);
        assert!((u[20] - 1.0 / 2048.0).abs() < 1e-9);
        // habilitado mas sem ajuste (cena vazia) também fica neutro
        assert_eq!(uniform_floats(&on, None)[16], 0.0);
    }

    #[test]
    fn light_shadow_validation_and_sanitizing() {
        let d = LightShadow::default();
        assert!(d.validate().is_ok());
        assert!(LightShadow { strength: 1.5, ..d }.validate().is_err());
        assert!(LightShadow { bias: f32::NAN, ..d }.validate().is_err());
        assert!(LightShadow { resolution: 1000, ..d }.validate().is_err());
        let s = LightShadow { strength: 9.0, bias: f32::NAN, resolution: 1000, ..d }.sanitized();
        assert_eq!(s.strength, 1.0);
        assert_eq!(s.bias, d.bias);
        assert_eq!(s.resolution, 1024);
        assert!(d.is_default());
        assert!(!LightShadow { enabled: true, ..d }.is_default());
    }

    #[test]
    fn stylized_light_json_is_unchanged_with_default_shadow() {
        let json = serde_json::to_string(&crate::scene::StylizedLight::default()).unwrap();
        assert!(!json.contains("shadow\""), "o campo `shadow` deve ser omitido no padrão: {json}");
        let on = crate::scene::StylizedLight { shadow: LightShadow { enabled: true, ..d_shadow() }, ..Default::default() };
        let json = serde_json::to_string(&on).unwrap();
        assert!(json.contains("\"shadow\""));
        let back: crate::scene::StylizedLight = serde_json::from_str(&json).unwrap();
        assert_eq!(back, on);
        // JSON antigo (sem o campo) continua carregando
        let old: crate::scene::StylizedLight = serde_json::from_str(r#"{"direction":[0,1,0],"color":[1,1,1],"intensity":1.0,"shadow_color":[1,1,1],"ambient_intensity":0.3}"#).unwrap();
        assert!(!old.shadow.enabled);
    }

    fn d_shadow() -> LightShadow {
        LightShadow::default()
    }
}
