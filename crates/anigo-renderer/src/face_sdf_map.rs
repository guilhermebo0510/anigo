//! Mapa SDF de sombra facial procedural (demonstração e testes).
//!
//! O shader do cel (`cel_shading.wgsl`, bloco ANIGO-FACE-SDF) sombreia a região
//! do rosto onde `sdf < limiar(theta)`, com `limiar` de 0.5 (luz frontal) a 0.75
//! (luz traseira) mais o offset do material. Mapas reais vêm do artista (VRoid/
//! Genshin); este gerador produz um mapa mínimo mas CORRETO no sentido da
//! convenção — útil para validar espelhamento, cor de sombra e o frame da
//! cabeça sem depender de textura externa:
//!
//! - varredura lateral: `sdf = 0.5 + 0.25·t`, `t` de 0 (lado oposto à luz) a 1
//!   (lado da luz). Com luz lateral (limiar 0.625) metade do rosto fica na sombra,
//!   com luz traseira (0.75) quase tudo;
//! - um "nariz": mancha deslocada para o lado oposto à luz cujo núcleo entra na
//!   sombra assim que a luz sai da frente.
//!
//! Convenção de autoria: o mapa vale para luz em +X local (theta ≥ 0); o shader
//! espelha o `u` em torno de `center_u` para theta < 0.

/// Mapa em ponto flutuante (0..1) `width × height`, linhas de cima para baixo,
/// SEM a quantização de 8 bits (a varredura cobre só 0.5..0.75 do SDF; em R8 isso
/// são ~64 degraus e a penumbra mostra bandas — use [`HeadlessRenderer::set_face_sdf_map_f32`]
/// para a demonstração).
///
/// [`HeadlessRenderer::set_face_sdf_map_f32`]: crate::HeadlessRenderer::set_face_sdf_map_f32
///
/// - `center_u`: `u` do eixo de simetria do rosto (0.5 para mapas centrados; 0.25
///   na esfera UV/cabeça canônica, cujo rosto ocupa a metade `u ∈ [0, 0.5]`).
/// - `half_width_u`: meia-largura do rosto em `u` (0.25 na esfera UV).
/// - `shadow_on_high_u`: `true` quando `u` crescente aponta para -X local (a
///   convenção da esfera UV e da cabeça canônica): o lado oposto à luz em +X é
///   o de `u` maior.
pub fn procedural_face_sdf_f32(
    width: u32,
    height: u32,
    center_u: f32,
    half_width_u: f32,
    shadow_on_high_u: bool,
) -> Vec<f32> {
    let (w, h) = (width.max(1), height.max(1));
    let mut data = Vec::with_capacity((w * h) as usize);
    let half = half_width_u.max(1e-3);
    // mancha do nariz: deslocada para o lado oposto à luz, abaixo do meio
    let side = if shadow_on_high_u { 1.0 } else { -1.0 };
    let nose = (center_u + side * 0.35 * half, 0.58f32);
    let nose_radius = 0.09 * half.max(0.2) * 2.0;
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let d = (u - center_u) / (2.0 * half); // -0.5 .. 0.5 ao longo do rosto
            let t = if shadow_on_high_u { 0.5 - d } else { 0.5 + d }.clamp(0.0, 1.0);
            let mut sdf = 0.5 + 0.25 * t;
            let dist = ((u - nose.0).powi(2) + ((v - nose.1) * 0.5).powi(2)).sqrt();
            let blob = 0.5 + 0.25 * smoothstep(0.0, nose_radius, dist);
            sdf = sdf.min(blob);
            data.push(sdf.clamp(0.0, 1.0));
        }
    }
    data
}

/// Mesmo mapa quantizado em R8 (como um PNG de artista). Argumentos como em
/// [`procedural_face_sdf_f32`].
pub fn procedural_face_sdf(
    width: u32,
    height: u32,
    center_u: f32,
    half_width_u: f32,
    shadow_on_high_u: bool,
) -> Vec<u8> {
    procedural_face_sdf_f32(width, height, center_u, half_width_u, shadow_on_high_u)
        .into_iter()
        .map(|v| (v * 255.0 + 0.5) as u8)
        .collect()
}

/// `f32` → bits de `f16` (IEEE 754 half), com arredondamento ao mais próximo.
/// Suficiente para valores normalizados 0..1 do SDF (sem NaN/Inf).
pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 }; // Inf/NaN
    }
    let unbiased = exp - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00; // overflow → Inf
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign; // underflow → ±0
        }
        // subnormal
        let m = (mant | 0x0080_0000) >> (1 - unbiased);
        let round = (m >> 13) as u16 + (((m >> 12) & 1) as u16);
        return sign | round;
    }
    let half = sign | ((unbiased as u16) << 10) | ((mant >> 13) as u16);
    // arredonda ao mais próximo (empate para cima; o carry propaga certo)
    if (mant & 0x1000) != 0 {
        half + 1
    } else {
        half
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_conversion_matches_known_values() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(0.25), 0x3400);
        assert_eq!(f32_to_f16_bits(0.75), 0x3a00);
        assert_eq!(f32_to_f16_bits(-2.0), 0xc000);
        assert_eq!(f32_to_f16_bits(0.333_333_34), 0x3555);
        // monotônico em 0..1 (o SDF não pode "dobrar" na quantização)
        let mut last = 0u16;
        for i in 0..=4096 {
            let bits = f32_to_f16_bits(i as f32 / 4096.0);
            assert!(bits >= last);
            last = bits;
        }
    }

    #[test]
    fn sweep_runs_from_shadow_side_to_light_side() {
        let (w, h) = (64u32, 16u32);
        let map = procedural_face_sdf(w, h, 0.25, 0.25, true);
        assert_eq!(map.len(), (w * h) as usize);
        let at = |u: f32, v: f32| map[((v * h as f32) as u32 * w + (u * w as f32) as u32) as usize];
        // u alto = lado oposto à luz (+X) = sdf baixo (entra cedo na sombra);
        // u baixo = lado da luz = sdf alto. Linha do topo (sem a mancha do nariz).
        let far = at(0.49, 0.05);
        let near = at(0.01, 0.05);
        assert!(far < near, "far={far} near={near}");
        assert!((far as f32 / 255.0 - 0.5).abs() < 0.02, "extremo oposto ≈ 0.5");
        assert!((near as f32 / 255.0 - 0.75).abs() < 0.02, "extremo da luz ≈ 0.75");
        // centro do rosto ≈ 0.625 (limiar da luz lateral de 90°)
        let mid = at(0.25, 0.05) as f32 / 255.0;
        assert!((mid - 0.625).abs() < 0.02, "mid={mid}");
    }

    #[test]
    fn mirrored_authoring_flips_the_sweep() {
        let a = procedural_face_sdf(32, 8, 0.5, 0.5, true);
        let b = procedural_face_sdf(32, 8, 0.5, 0.5, false);
        assert_ne!(a, b);
        // linha do topo: b é o espelho de a
        let row = |m: &[u8]| m[..32].to_vec();
        let mut mirrored: Vec<u8> = row(&a);
        mirrored.reverse();
        assert_eq!(row(&b), mirrored);
    }
}
