//! Motor cel NPR — parâmetros por material e matemática pura.
//!
//! Este módulo concentra (1) o grupo [`NprParams`] de parâmetros opcionais do
//! motor de cel shading e (2) a matemática **pura e testável** que o WGSL
//! (`crates/anigo-renderer/shaders/*.wgsl`) executa na GPU. Cada função aqui
//! diz, no comentário, qual trecho de shader ela espelha; os testes `golden_*`
//! no fim do arquivo fixam os valores que o shader calcula, de modo que uma
//! mudança num lado sem o outro quebra o teste.
//!
//! Regra de ouro dos parâmetros: o valor **padrão** de cada um reproduz byte a
//! byte o comportamento anterior do motor (o frame congelado do render
//! contract e os hashes de `golden_frames.rs` não mudam com tudo no padrão).

use serde::{Deserialize, Serialize};

/// Parâmetros opcionais do motor cel (rampa 2D, contorno de alta qualidade,
/// face SDF). Vive em `StylizedMaterial::npr`; é omitido da serialização
/// enquanto estiver todo no padrão (arquivos de projeto/fixtures antigos
/// continuam idênticos).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NprParams {
    // ── Rampa 2D (cel_shading.wgsl) ─────────────────────────────────────────
    /// 0 = desligada (padrão: fator de luz 100% analítico). 1 = a rampa 2D
    /// decide o fator de luz. Faixa 0..1 (mistura linear).
    pub ramp_blend: f32,
    /// Linha da rampa 2D usada quando `ramp_blend > 0`. Faixa 0..3 (as linhas do
    /// contrato: 0 contínua, 1 um degrau, 2 dois, 3 três). Fração interpola
    /// linearmente entre linhas vizinhas.
    pub ramp_row: f32,

    // ── Contorno (inverted_hull.wgsl + lado Rust do vertex buffer) ─────────
    /// 0 = normais da malha (padrão); 1 = normais SUAVIZADAS de contorno
    /// (soldadas por posição, média ponderada por ângulo): sem frestas em
    /// arestas duras. Fração mistura as duas.
    pub outline_smooth_normals: f32,
    /// 0 = espessura constante em pixels (padrão); 1 = espessura proporcional ao
    /// tamanho projetado do objeto (afina ao longe, engrossa perto, e já
    /// compensa o FOV). Fração é um expoente (`escala = k^falloff`).
    pub outline_distance_falloff: f32,
    /// Distância (m) em que a largura é a nominal `outline_width` (com o FOV de
    /// referência). Só vale com `outline_distance_falloff > 0`.
    pub outline_ref_distance: f32,
    /// Piso do fator de escala da largura (0..1) — impede a linha de sumir ao
    /// longe. 0 = sem piso.
    pub outline_min_width_scale: f32,
    /// Taper (peso de traço): 0 = largura uniforme (padrão); 1 = a linha afina
    /// onde a superfície se volta de frente para a câmera e fica cheia na
    /// silhueta rasante (variação de peso estilo caneta G).
    pub outline_taper: f32,
    /// 0 = cor `outline_color` do material (padrão); 1 = tinta derivada da cor
    /// base (escurecida e saturada). Fração mistura.
    pub outline_ink_blend: f32,
    /// Quanto a tinta escurece a cor base (0..1). Luminância × (1 − darken).
    pub outline_ink_darken: f32,
    /// Multiplicador de saturação da tinta (1 = igual à base).
    pub outline_ink_saturation: f32,

    // ── Face SDF (cel_shading.wgsl) ─────────────────────────────────────────
    /// 0 = sem espelhamento (padrão, sombra simétrica). > 0 = u do eixo de
    /// simetria do rosto no mapa SDF (0.5 para mapas centrados); o mapa é
    /// sampleado espelhado quando a luz vem do lado oposto ao autorado.
    pub face_sdf_mirror_center: f32,
}

impl Default for NprParams {
    fn default() -> Self {
        Self {
            ramp_blend: 0.0,
            ramp_row: 1.0,
            outline_smooth_normals: 0.0,
            outline_distance_falloff: 0.0,
            outline_ref_distance: 4.0,
            outline_min_width_scale: 0.0,
            outline_taper: 0.0,
            outline_ink_blend: 0.0,
            outline_ink_darken: 0.55,
            outline_ink_saturation: 1.15,
            face_sdf_mirror_center: 0.0,
        }
    }
}

impl NprParams {
    /// `true` com tudo no padrão (usado por `skip_serializing_if`).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Valores seguros para a GPU (faixas documentadas; NaN vira o padrão).
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let c = |v: f32, lo: f32, hi: f32, fallback: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                fallback
            }
        };
        Self {
            ramp_blend: c(self.ramp_blend, 0.0, 1.0, d.ramp_blend),
            ramp_row: c(self.ramp_row, 0.0, 3.0, d.ramp_row),
            outline_smooth_normals: c(self.outline_smooth_normals, 0.0, 1.0, d.outline_smooth_normals),
            outline_distance_falloff: c(self.outline_distance_falloff, 0.0, 1.0, d.outline_distance_falloff),
            outline_ref_distance: c(self.outline_ref_distance, 0.05, 1000.0, d.outline_ref_distance),
            outline_min_width_scale: c(self.outline_min_width_scale, 0.0, 1.0, d.outline_min_width_scale),
            outline_taper: c(self.outline_taper, 0.0, 1.0, d.outline_taper),
            outline_ink_blend: c(self.outline_ink_blend, 0.0, 1.0, d.outline_ink_blend),
            outline_ink_darken: c(self.outline_ink_darken, 0.0, 1.0, d.outline_ink_darken),
            outline_ink_saturation: c(self.outline_ink_saturation, 0.0, 3.0, d.outline_ink_saturation),
            face_sdf_mirror_center: c(self.face_sdf_mirror_center, 0.0, 1.0, d.face_sdf_mirror_center),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rampa 2D — espelha `cel_shading.wgsl` (bloco "Rampa 2D opcional")
// ─────────────────────────────────────────────────────────────────────────────

/// Linhas da rampa 2D do contrato (`toon_ramp.height`).
pub const RAMP_ROWS: u32 = 4;

/// Coordenada V (0..1) do centro da linha `row` (0..3, fração interpola):
/// `(clamp(row, 0, 3) + 0.5) / 4`. WGSL: `(clamp(material.params7.w, 0.0, 3.0) + 0.5) * 0.25`.
pub fn ramp_v(row: f32) -> f32 {
    (row.clamp(0.0, (RAMP_ROWS - 1) as f32) + 0.5) * 0.25
}

/// Coordenada U (0..1) da coluna: half-lambert deslocado pelo limiar de sombra
/// de modo que o degrau da rampa caia em `0.5` quando `half_lambert == threshold`.
/// WGSL: `clamp((half_lambert - threshold) + 0.5, 0.002, 0.998)`.
pub fn ramp_u(half_lambert: f32, threshold: f32) -> f32 {
    ((half_lambert - threshold) + 0.5).clamp(0.002, 0.998)
}

/// `half_lambert = N·L * 0.5 + 0.5` (WGSL: `n_dot_l * 0.5 + 0.5`).
pub fn half_lambert(n_dot_l: f32) -> f32 {
    n_dot_l * 0.5 + 0.5
}

/// Texel (coluna, linha) mais próximo do par (u, v) numa rampa `width × height`
/// — o "índice de rampa 2D" (amostragem `nearest`; o shader usa bilinear).
pub fn ramp_texel(u: f32, v: f32, width: u32, height: u32) -> (u32, u32) {
    let col = ((u * width as f32).floor() as i64).clamp(0, width as i64 - 1) as u32;
    let row = ((v * height as f32).floor() as i64).clamp(0, height as i64 - 1) as u32;
    (col, row)
}

/// Amostragem bilinear (clamp_to_edge) do canal R de uma rampa RGBA8 —
/// reproduz o que `textureSample` + sampler linear entrega ao shader.
pub fn sample_ramp_bilinear(rgba: &[u8], width: u32, height: u32, u: f32, v: f32) -> f32 {
    let fx = (u * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
    let fy = (v * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: u32, y: u32| rgba[((y * width + x) * 4) as usize] as f32 / 255.0;
    let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
    let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Fator de luz final: `mix(analytic, ramp, blend)` (WGSL: `mix(analytical_toon,
/// ramp_value, ramp_blend)`); com `blend == 0` devolve `analytic` exatamente.
pub fn blend_toon_factor(analytic: f32, ramp_value: f32, blend: f32) -> f32 {
    let blend = blend.clamp(0.0, 1.0);
    if blend <= 0.0 {
        analytic
    } else {
        analytic * (1.0 - blend) + ramp_value * blend
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_are_the_legacy_behaviour_and_skip_serialization() {
        let p = NprParams::default();
        assert!(p.is_default());
        assert_eq!(p.ramp_blend, 0.0);
        assert_eq!(p.outline_smooth_normals, 0.0);
        assert_eq!(p.outline_distance_falloff, 0.0);
        assert_eq!(p.outline_taper, 0.0);
        assert_eq!(p.outline_ink_blend, 0.0);
        assert_eq!(p.face_sdf_mirror_center, 0.0);
        // sanitized() não altera valores já válidos
        assert_eq!(p.sanitized(), p);
    }

    #[test]
    fn sanitized_clamps_and_replaces_nan() {
        let p = NprParams {
            ramp_blend: 3.0,
            ramp_row: f32::NAN,
            outline_distance_falloff: -1.0,
            ..NprParams::default()
        }
        .sanitized();
        assert_eq!(p.ramp_blend, 1.0);
        assert_eq!(p.ramp_row, 1.0);
        assert_eq!(p.outline_distance_falloff, 0.0);
    }

    // Goldens da rampa: valores que o shader calcula (ver cel_shading.wgsl).
    #[test]
    fn golden_ramp_v_is_row_center() {
        assert_eq!(ramp_v(0.0), 0.125);
        assert_eq!(ramp_v(1.0), 0.375);
        assert_eq!(ramp_v(2.0), 0.625);
        assert_eq!(ramp_v(3.0), 0.875);
        assert_eq!(ramp_v(1.5), 0.5); // fração = meio entre os centros 1 e 2
        assert_eq!(ramp_v(-5.0), 0.125);
        assert_eq!(ramp_v(99.0), 0.875);
    }

    #[test]
    fn golden_ramp_u_puts_the_step_at_half_when_hl_equals_threshold() {
        assert_eq!(ramp_u(0.5, 0.5), 0.5);
        assert_eq!(ramp_u(0.0, 0.5), 0.002); // clamp inferior
        assert_eq!(ramp_u(1.0, 0.5), 0.998); // clamp superior
        assert!((ramp_u(0.75, 0.5) - 0.75).abs() < 1e-7);
        assert!((ramp_u(0.60, 0.55) - 0.55).abs() < 1e-6);
    }

    #[test]
    fn golden_half_lambert_and_texel_index() {
        assert_eq!(half_lambert(-1.0), 0.0);
        assert_eq!(half_lambert(0.0), 0.5);
        assert_eq!(half_lambert(1.0), 1.0);
        assert_eq!(ramp_texel(0.5, 0.375, 256, 4), (128, 1));
        assert_eq!(ramp_texel(0.998, 0.875, 256, 4), (255, 3));
        assert_eq!(ramp_texel(0.002, 0.125, 256, 4), (0, 0));
    }

    #[test]
    fn golden_blend_toon_factor_zero_is_exact_analytic() {
        assert_eq!(blend_toon_factor(0.3, 1.0, 0.0), 0.3);
        assert_eq!(blend_toon_factor(0.3, 1.0, 1.0), 1.0);
        assert!((blend_toon_factor(0.2, 0.8, 0.5) - 0.5).abs() < 1e-7);
        assert_eq!(blend_toon_factor(0.2, 0.8, -3.0), 0.2);
    }

    #[test]
    fn bilinear_ramp_sampling_matches_texel_centers() {
        // rampa 4x2: linha 0 = 0,85,170,255 ; linha 1 = 255 constante
        let mut rgba = vec![0u8; 4 * 2 * 4];
        for x in 0..4u32 {
            rgba[(x * 4) as usize] = (x * 85) as u8;
            rgba[((4 + x) * 4) as usize] = 255;
        }
        // centro exato do texel (1, 0): u = 1.5/4, v = 0.25
        assert!((sample_ramp_bilinear(&rgba, 4, 2, 1.5 / 4.0, 0.25) - 85.0 / 255.0).abs() < 1e-6);
        // entre os centros dos texels 1 e 2
        assert!((sample_ramp_bilinear(&rgba, 4, 2, 2.0 / 4.0, 0.25) - 127.5 / 255.0).abs() < 1e-6);
        // clamp_to_edge
        assert!((sample_ramp_bilinear(&rgba, 4, 2, 0.0, 0.25) - 0.0).abs() < 1e-6);
        assert!((sample_ramp_bilinear(&rgba, 4, 2, 1.0, 0.25) - 1.0).abs() < 1e-6);
    }
}
