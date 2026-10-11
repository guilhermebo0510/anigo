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
    /// Quanto a tinta escurece a cor base (0..1): V × (1 − darken) em HSV.
    pub outline_ink_darken: f32,
    /// Multiplicador de saturação (HSV) da tinta (1 = igual à base).
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
            outline_ink_darken: 0.7,
            outline_ink_saturation: 2.5,
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

// ─────────────────────────────────────────────────────────────────────────────
// Contorno (inverted hull) — espelha `inverted_hull.wgsl`
// ─────────────────────────────────────────────────────────────────────────────

/// FOV vertical de referência (rad) da largura nominal do contorno: 45°, o
/// padrão da câmera do contrato. `outline_ref_distance` vale neste FOV.
pub const OUTLINE_REF_FOV_Y: f32 = std::f32::consts::FRAC_PI_4;

/// Teto do fator de escala da largura (perto da câmera a linha engrossa até 4×).
/// WGSL: o `4.0` de `clamp(pow(k, falloff), min_scale, 4.0)`.
pub const OUTLINE_MAX_WIDTH_SCALE: f32 = 4.0;

/// Quanto o taper afina a linha onde a superfície encara a câmera: com
/// `taper = 1` a largura cai a `1 − 0.75 = 25%`. WGSL: `0.75` em `taper_scale`.
pub const OUTLINE_TAPER_DEPTH: f32 = 0.75;

/// Distância de referência efetiva (m) para o FOV atual: dobra a compensação de
/// FOV dentro da distância (`k = ref_eff / profundidade`). Com o FOV de
/// referência vale `ref_distance`; ao estreitar o FOV (teleobjetiva) o objeto
/// cresce na tela e a linha acompanha. Vai em `outline.params2.w`.
///
/// `ref_eff = ref_distance · tan(ref_fov/2) / tan(fov/2)`.
pub fn outline_ref_distance_effective(ref_distance: f32, fov_y: f32) -> f32 {
    let t_ref = (OUTLINE_REF_FOV_Y * 0.5).tan();
    let t = (fov_y * 0.5).tan().max(1e-4);
    ref_distance * t_ref / t
}

/// Fator de escala da largura do contorno por distância (profundidade de vista
/// `clip.w`). `falloff = 0` → 1 exato (espessura constante em pixels, o motor
/// anterior). WGSL: `clamp(pow(ref_eff / max(w, 0.001), falloff), min_scale, 4.0)`.
pub fn outline_width_scale(view_depth: f32, ref_eff: f32, falloff: f32, min_scale: f32) -> f32 {
    if falloff <= 0.0 {
        return 1.0;
    }
    let k = ref_eff / view_depth.max(0.001);
    k.powf(falloff).clamp(min_scale, OUTLINE_MAX_WIDTH_SCALE)
}

/// Fator de taper (peso de traço) por orientação: `facing = |N·V|` (0 = silhueta
/// rasante, 1 = de frente). `taper = 0` → 1 exato. WGSL: `1.0 - taper * 0.75 * facing`.
pub fn outline_taper_scale(n_dot_v: f32, taper: f32) -> f32 {
    1.0 - taper * OUTLINE_TAPER_DEPTH * n_dot_v.abs().min(1.0)
}

/// Largura final em unidades de clip-xy/w (NDC) de um vértice do hull:
/// `width · mask(B) · scale_distância · scale_taper` (WGSL: `thickness`).
pub fn outline_thickness(width: f32, vertex_mask_b: f32, distance_scale: f32, taper_scale: f32) -> f32 {
    width * vertex_mask_b * distance_scale * taper_scale
}

/// RGB (0..1) → HSV (h em 0..1).
pub fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 1e-9 {
        0.0
    } else if max == r {
        (((g - b) / d) % 6.0 + 6.0) % 6.0 / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= 1e-9 { 0.0 } else { d / max };
    [h, s, max]
}

/// HSV (h em 0..1) → RGB (0..1).
pub fn hsv_to_rgb(hsv: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv;
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// Tinta do contorno derivada da cor base (sRGB 0..1), em HSV: mantém o matiz,
/// multiplica a saturação (`saturation`) e escurece o valor (`V × (1 − darken)`).
/// `saturation = 1, darken = 0` devolve a própria base. Em pele clara
/// (S ≈ 0.13) uma saturação ~2.5 vira o marrom-avermelhado típico de linha de
/// anime, em vez de cinza.
pub fn outline_ink_rgb(base_rgb: [f32; 3], darken: f32, saturation: f32) -> [f32; 3] {
    let [h, s, v] = rgb_to_hsv([
        base_rgb[0].clamp(0.0, 1.0),
        base_rgb[1].clamp(0.0, 1.0),
        base_rgb[2].clamp(0.0, 1.0),
    ]);
    let ink = hsv_to_rgb([h, (s * saturation).clamp(0.0, 1.0), v * (1.0 - darken).clamp(0.0, 1.0)]);
    [ink[0].clamp(0.0, 1.0), ink[1].clamp(0.0, 1.0), ink[2].clamp(0.0, 1.0)]
}

/// Cor final do contorno de um material: `mix(outline_color, tinta, blend)` no
/// RGB; o alfa é o de `outline_color`. `blend = 0` devolve `outline_color` exato.
pub fn outline_color_for_material(
    outline_color: [f32; 4],
    base_color: [f32; 4],
    params: &NprParams,
) -> [f32; 4] {
    let p = params.sanitized();
    if p.outline_ink_blend <= 0.0 {
        return outline_color;
    }
    let ink = outline_ink_rgb(
        [base_color[0], base_color[1], base_color[2]],
        p.outline_ink_darken,
        p.outline_ink_saturation,
    );
    let t = p.outline_ink_blend;
    [
        outline_color[0] * (1.0 - t) + ink[0] * t,
        outline_color[1] * (1.0 - t) + ink[1] * t,
        outline_color[2] * (1.0 - t) + ink[2] * t,
        outline_color[3],
    ]
}

/// Normais SUAVIZADAS de contorno: soldadas por posição (vértices de arestas
/// duras compartilham a mesma normal) com média das normais de face ponderada
/// pelo ÂNGULO interno em cada canto (independe da triangulação). Sem isto, o
/// hull invertido de uma aresta dura (cubo, queixo, roupa) abre fresta — cada
/// face extruda para um lado. Vai num segundo stream de vértices só do pipeline
/// de contorno (o vértice de 72 B e o do cel não mudam).
///
/// Saída: uma normal por vértice de entrada. Vértices isolados/degenerados
/// (sem face utilizável) ou cuja soma contradiz a normal original (malha com
/// winding invertido) mantêm a normal original.
pub fn smooth_outline_normals(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u32],
) -> Vec<[f32; 3]> {
    use std::collections::HashMap;
    let count = positions.len().min(normals.len());
    if count == 0 {
        return Vec::new();
    }
    // passo de soldagem: 1e-5 da diagonal da caixa (≥ 1e-7), invariante à escala
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in &positions[..count] {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let step = (diag * 1e-5).max(1e-7);
    let key = |p: &[f32; 3]| {
        (
            (p[0] / step).round() as i64,
            (p[1] / step).round() as i64,
            (p[2] / step).round() as i64,
        )
    };
    let mut group_of: Vec<usize> = Vec::with_capacity(count);
    let mut table: HashMap<(i64, i64, i64), usize> = HashMap::new();
    for p in &positions[..count] {
        let next = table.len();
        group_of.push(*table.entry(key(p)).or_insert(next));
    }
    let mut sums = vec![[0.0f64; 3]; table.len()];

    let sub = |a: [f32; 3], b: [f32; 3]| [(a[0] - b[0]) as f64, (a[1] - b[1]) as f64, (a[2] - b[2]) as f64];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let norm = |a: [f64; 3]| dot(a, a).sqrt();
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= count || i1 >= count || i2 >= count {
            continue;
        }
        let corners = [i0, i1, i2];
        let face = cross(sub(positions[i1], positions[i0]), sub(positions[i2], positions[i0]));
        let face_len = norm(face);
        if face_len < 1e-18 {
            continue;
        }
        let face_n = [face[0] / face_len, face[1] / face_len, face[2] / face_len];
        for c in 0..3 {
            let here = positions[corners[c]];
            let e1 = sub(positions[corners[(c + 1) % 3]], here);
            let e2 = sub(positions[corners[(c + 2) % 3]], here);
            let (l1, l2) = (norm(e1), norm(e2));
            if l1 < 1e-18 || l2 < 1e-18 {
                continue;
            }
            let angle = (dot(e1, e2) / (l1 * l2)).clamp(-1.0, 1.0).acos();
            let g = group_of[corners[c]];
            for a in 0..3 {
                sums[g][a] += face_n[a] * angle;
            }
        }
    }
    (0..count)
        .map(|v| {
            let s = sums[group_of[v]];
            let len = norm(s);
            if len < 1e-12 {
                return normals[v];
            }
            let n = [(s[0] / len) as f32, (s[1] / len) as f32, (s[2] / len) as f32];
            let along = n[0] as f64 * normals[v][0] as f64
                + n[1] as f64 * normals[v][1] as f64
                + n[2] as f64 * normals[v][2] as f64;
            if along < 0.0 {
                normals[v]
            } else {
                n
            }
        })
        .collect()
}

/// Mistura (e renormaliza) a normal da malha com a suavizada: `blend = 0`
/// devolve a normal da malha EXATA (sem renormalizar), `1` a suavizada.
pub fn blend_outline_normal(mesh_normal: [f32; 3], smooth_normal: [f32; 3], blend: f32) -> [f32; 3] {
    let t = blend.clamp(0.0, 1.0);
    if t <= 0.0 {
        return mesh_normal;
    }
    let m = [
        mesh_normal[0] * (1.0 - t) + smooth_normal[0] * t,
        mesh_normal[1] * (1.0 - t) + smooth_normal[1] * t,
        mesh_normal[2] * (1.0 - t) + smooth_normal[2] * t,
    ];
    let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
    if len < 1e-8 {
        return smooth_normal;
    }
    [m[0] / len, m[1] / len, m[2] / len]
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

    // ── Contorno ────────────────────────────────────────────────────────────

    #[test]
    fn golden_outline_ref_distance_effective_folds_the_fov() {
        // no FOV de referência (45°) vale a própria distância
        assert!((outline_ref_distance_effective(4.0, OUTLINE_REF_FOV_Y) - 4.0).abs() < 1e-5);
        // 20°: 4 · tan(22.5°) / tan(10°) = 9.39657...
        let tele = outline_ref_distance_effective(4.0, 20.0f32.to_radians());
        assert!((tele - 9.396_58).abs() < 1e-3, "{tele}");
        // grande angular (90°): 4 · 0.41421356 / 1 = 1.6568542
        let wide = outline_ref_distance_effective(4.0, 90.0f32.to_radians());
        assert!((wide - 1.656_854).abs() < 1e-4, "{wide}");
    }

    #[test]
    fn golden_outline_width_scale_matches_the_shader() {
        // falloff 0 → 1 exato, qualquer distância (motor anterior)
        assert_eq!(outline_width_scale(0.5, 5.0, 0.0, 0.0), 1.0);
        assert_eq!(outline_width_scale(50.0, 5.0, 0.0, 0.7), 1.0);
        // falloff 1: k = ref / profundidade
        assert!((outline_width_scale(5.0, 5.0, 1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((outline_width_scale(10.0, 5.0, 1.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((outline_width_scale(2.5, 5.0, 1.0, 0.0) - 2.0).abs() < 1e-6);
        // teto 4 perto da câmera
        assert_eq!(outline_width_scale(1.0, 5.0, 1.0, 0.0), OUTLINE_MAX_WIDTH_SCALE);
        // piso ao longe
        assert!((outline_width_scale(100.0, 5.0, 1.0, 0.2) - 0.2).abs() < 1e-6);
        // falloff fracionário é expoente: (5/20)^0.5 = 0.5
        assert!((outline_width_scale(20.0, 5.0, 0.5, 0.0) - 0.5).abs() < 1e-6);
        // profundidade ≤ 0 não explode (guarda 0.001 do shader → teto)
        assert_eq!(outline_width_scale(0.0, 5.0, 1.0, 0.0), OUTLINE_MAX_WIDTH_SCALE);
    }

    #[test]
    fn golden_outline_taper_scale_matches_the_shader() {
        assert_eq!(outline_taper_scale(1.0, 0.0), 1.0);
        assert_eq!(outline_taper_scale(0.3, 0.0), 1.0);
        assert!((outline_taper_scale(1.0, 1.0) - 0.25).abs() < 1e-7);
        assert_eq!(outline_taper_scale(0.0, 1.0), 1.0); // silhueta rasante: largura cheia
        assert!((outline_taper_scale(-0.5, 0.5) - 0.8125).abs() < 1e-7);
        assert!((outline_taper_scale(2.0, 1.0) - 0.25).abs() < 1e-7); // |N·V| limitado a 1
    }

    #[test]
    fn golden_outline_thickness_is_the_product_of_the_factors() {
        assert_eq!(outline_thickness(0.004, 1.0, 1.0, 1.0), 0.004);
        assert_eq!(outline_thickness(0.004, 0.5, 2.0, 0.25), 0.004 * 0.5 * 2.0 * 0.25);
        assert_eq!(outline_thickness(0.004, 0.0, 2.0, 1.0), 0.0);
    }

    #[test]
    fn golden_outline_ink_for_light_skin_is_a_warm_dark_brown() {
        // pele (0.98, 0.92, 0.85): S = 0.13265, V = 0.98, H = 0.0897;
        // saturação ×2.5 e V × 0.3 → (0.294, 0.2490, 0.1965)
        let ink = outline_ink_rgb([0.98, 0.92, 0.85], 0.7, 2.5);
        assert!((ink[0] - 0.294).abs() < 1e-4, "{ink:?}");
        assert!((ink[1] - 0.2490).abs() < 2e-4, "{ink:?}");
        assert!((ink[2] - 0.1965).abs() < 1e-4, "{ink:?}");
        // identidade: sem escurecer nem saturar devolve a própria cor
        let same = outline_ink_rgb([0.2, 0.6, 0.9], 0.0, 1.0);
        for (a, b) in same.iter().zip([0.2, 0.6, 0.9]) {
            assert!((a - b).abs() < 1e-6);
        }
        // cinza fica cinza (sem matiz para saturar)
        let grey = outline_ink_rgb([0.5, 0.5, 0.5], 0.5, 3.0);
        assert!((grey[0] - 0.25).abs() < 1e-6 && grey[0] == grey[1] && grey[1] == grey[2]);
    }

    #[test]
    fn hsv_roundtrip() {
        for rgb in [[0.9, 0.1, 0.2], [0.1, 0.8, 0.3], [0.2, 0.3, 0.95], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0]] {
            let back = hsv_to_rgb(rgb_to_hsv(rgb));
            for i in 0..3 {
                assert!((back[i] - rgb[i]).abs() < 1e-5, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn outline_color_for_material_default_is_the_material_color_exactly() {
        let outline = [0.25, 0.15, 0.2, 0.8];
        let base = [0.98, 0.92, 0.85, 1.0];
        assert_eq!(outline_color_for_material(outline, base, &NprParams::default()), outline);
        let p = NprParams { outline_ink_blend: 1.0, ..NprParams::default() };
        let c = outline_color_for_material(outline, base, &p);
        assert!((c[0] - 0.294).abs() < 1e-4);
        assert_eq!(c[3], 0.8, "alfa é o de outline_color");
        let half = NprParams { outline_ink_blend: 0.5, ..NprParams::default() };
        let m = outline_color_for_material(outline, base, &half);
        assert!((m[0] - (0.25 * 0.5 + 0.294 * 0.5)).abs() < 1e-4);
    }

    fn tri_cube(flip_corner_diagonal: bool) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
        // cubo de aresta 2 com 24 vértices (normais por face) — como Mesh::create_cube.
        let h = 1.0;
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([0.0, 0.0, 1.0], [[-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h]]),
            ([0.0, 0.0, -1.0], [[h, -h, -h], [-h, -h, -h], [-h, h, -h], [h, h, -h]]),
            ([0.0, 1.0, 0.0], [[-h, h, h], [h, h, h], [h, h, -h], [-h, h, -h]]),
            ([0.0, -1.0, 0.0], [[-h, -h, -h], [h, -h, -h], [h, -h, h], [-h, -h, h]]),
            ([1.0, 0.0, 0.0], [[h, -h, h], [h, -h, -h], [h, h, -h], [h, h, h]]),
            ([-1.0, 0.0, 0.0], [[-h, -h, -h], [-h, -h, h], [-h, h, h], [-h, h, -h]]),
        ];
        let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
        for (face_index, (n, corners)) in faces.iter().enumerate() {
            let base = pos.len() as u32;
            for c in corners {
                pos.push(*c);
                nrm.push(*n);
            }
            if flip_corner_diagonal && face_index == 0 {
                // diagonal 1-3 em vez de 0-2 só na face +Z
                idx.extend_from_slice(&[base, base + 1, base + 3, base + 1, base + 2, base + 3]);
            } else {
                idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        (pos, nrm, idx)
    }

    #[test]
    fn golden_smooth_normals_of_a_cube_point_along_the_diagonals() {
        let (pos, nrm, idx) = tri_cube(false);
        let smooth = smooth_outline_normals(&pos, &nrm, &idx);
        let inv = 1.0 / 3.0f32.sqrt();
        for (n, p) in smooth.iter().zip(&pos) {
            for a in 0..3 {
                assert!((n[a] - p[a].signum() * inv).abs() < 1e-6, "{n:?} @ {p:?}");
            }
        }
    }

    #[test]
    fn smooth_normals_do_not_depend_on_the_triangulation() {
        // Média ponderada por ÂNGULO: a diagonal que passa pelo canto divide o
        // ângulo de 90° em 45°+45°; o resultado é o mesmo (média por contagem
        // de triângulos daria peso dobrado a essa face).
        let (pos, nrm, idx_a) = tri_cube(false);
        let (_, _, idx_b) = tri_cube(true);
        let a = smooth_outline_normals(&pos, &nrm, &idx_a);
        let b = smooth_outline_normals(&pos, &nrm, &idx_b);
        for (x, y) in a.iter().zip(&b) {
            for i in 0..3 {
                assert!((x[i] - y[i]).abs() < 1e-6, "{x:?} vs {y:?}");
            }
        }
    }

    #[test]
    fn smooth_normals_weld_duplicates_and_keep_isolated_vertices() {
        let (mut pos, mut nrm, mut idx) = tri_cube(false);
        // vértice solto (sem triângulo) mantém a normal original
        pos.push([5.0, 5.0, 5.0]);
        nrm.push([0.0, 1.0, 0.0]);
        // triângulo degenerado é ignorado
        let last = pos.len() as u32 - 1;
        idx.extend_from_slice(&[last, last, last]);
        let smooth = smooth_outline_normals(&pos, &nrm, &idx);
        assert_eq!(smooth.len(), pos.len());
        assert_eq!(smooth[pos.len() - 1], [0.0, 1.0, 0.0]);
        // as 3 cópias do canto (1,1,1) têm a mesma normal
        let corner: Vec<[f32; 3]> = pos
            .iter()
            .zip(&smooth)
            .filter(|(p, _)| **p == [1.0, 1.0, 1.0])
            .map(|(_, n)| *n)
            .collect();
        assert_eq!(corner.len(), 3);
        assert_eq!(corner[0], corner[1]);
        assert_eq!(corner[1], corner[2]);
    }

    #[test]
    fn smooth_normals_of_inverted_winding_fall_back_to_the_mesh_normal() {
        let (pos, nrm, mut idx) = tri_cube(false);
        for tri in idx.chunks_exact_mut(3) {
            tri.swap(1, 2); // winding invertido: a soma aponta para dentro
        }
        let smooth = smooth_outline_normals(&pos, &nrm, &idx);
        assert_eq!(smooth, nrm);
    }

    #[test]
    fn golden_blend_outline_normal() {
        let mesh = [0.0, 0.0, 1.0];
        let smooth = [0.0, 1.0, 0.0];
        assert_eq!(blend_outline_normal(mesh, smooth, 0.0), mesh); // exato, sem renormalizar
        assert_eq!(blend_outline_normal(mesh, smooth, 1.0), smooth);
        let half = blend_outline_normal(mesh, smooth, 0.5);
        let r = 0.5f32.sqrt();
        assert!((half[1] - r).abs() < 1e-6 && (half[2] - r).abs() < 1e-6);
    }
}
