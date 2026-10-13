//! Operações de imagem do render: redução por caixa do supersampling (com precisão acima de 8 bits), alfa
//! pré-multiplicado ↔ reto, conversão sRGB → Display P3, achatamento sobre o fundo e codificação 8/16 bits.
//! Tudo por linha, em paralelo, e **independente da ordem** (cada pixel de saída depende só das suas amostras): o
//! resultado é o mesmo qualquer que seja o número de threads.

use anigo_core::ColorSpace;

/// Imagem RGBA em ponto flutuante, **alfa pré-multiplicado**, valores 0..1 já codificados (sRGB de exibição).
#[derive(Debug, Clone)]
pub struct FloatImage {
    pub w: u32,
    pub h: u32,
    pub px: Vec<f32>,
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 8)
}

/// Executa `f(y, linha)` para cada linha de `buf` (`stride` floats/bytes por linha) em threads.
fn par_rows<T: Send>(buf: &mut [T], stride: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = if stride == 0 { 0 } else { buf.len() / stride };
    if rows == 0 {
        return;
    }
    let per = rows.div_ceil(threads());
    std::thread::scope(|s| {
        for (ci, chunk) in buf.chunks_mut(per * stride).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (ri, row) in chunk.chunks_mut(stride).enumerate() {
                    f(ci * per + ri, row);
                }
            });
        }
    });
}

impl FloatImage {
    /// Reduz `src` (RGBA8, `w*k × h*k`, saída do renderer: alfa pré-multiplicado pelo resolve do MSAA) por média `k×k`.
    pub fn from_rgba8_box(src: &[u8], w: u32, h: u32, k: u32) -> FloatImage {
        let k = k.max(1) as usize;
        let (wu, hu) = (w as usize, h as usize);
        let sw = wu * k;
        debug_assert_eq!(src.len(), sw * hu * k * 4);
        let mut px = vec![0.0f32; wu * hu * 4];
        let inv = 1.0 / (255.0 * (k * k) as f32);
        par_rows(&mut px, wu * 4, |y, row| {
            for x in 0..wu {
                let mut sum = [0u32; 4];
                for dy in 0..k {
                    let base = ((y * k + dy) * sw + x * k) * 4;
                    for dx in 0..k {
                        let o = base + dx * 4;
                        sum[0] += src[o] as u32;
                        sum[1] += src[o + 1] as u32;
                        sum[2] += src[o + 2] as u32;
                        sum[3] += src[o + 3] as u32;
                    }
                }
                for c in 0..4 {
                    row[x * 4 + c] = sum[c] as f32 * inv;
                }
            }
        });
        FloatImage { w, h, px }
    }

    /// Cor reta (des-pré-multiplica). Pixels sem alfa ficam `0,0,0,0`.
    pub fn unpremultiply(&mut self) {
        let stride = self.w as usize * 4;
        par_rows(&mut self.px, stride, |_, row| {
            for p in row.chunks_exact_mut(4) {
                let a = p[3];
                if a <= 1.0 / 65535.0 {
                    p.copy_from_slice(&[0.0; 4]);
                } else {
                    p[0] = (p[0] / a).min(1.0);
                    p[1] = (p[1] / a).min(1.0);
                    p[2] = (p[2] / a).min(1.0);
                }
            }
        });
    }

    /// Achata sobre `bg` (0..1 sRGB): saída opaca. Só vale antes de `unpremultiply` (a entrada é pré-multiplicada).
    pub fn flatten_over(&mut self, bg: [f32; 3]) {
        let stride = self.w as usize * 4;
        par_rows(&mut self.px, stride, |_, row| {
            for p in row.chunks_exact_mut(4) {
                let inv = 1.0 - p[3];
                p[0] = (p[0] + bg[0] * inv).min(1.0);
                p[1] = (p[1] + bg[1] * inv).min(1.0);
                p[2] = (p[2] + bg[2] * inv).min(1.0);
                p[3] = 1.0;
            }
        });
    }

    /// Reencoda a cor RETA (já des-pré-multiplicada) de sRGB para o espaço de saída. `Srgb` não faz nada.
    pub fn to_output_space(&mut self, space: ColorSpace) {
        if space != ColorSpace::DisplayP3 {
            return;
        }
        let stride = self.w as usize * 4;
        par_rows(&mut self.px, stride, |_, row| {
            for p in row.chunks_exact_mut(4) {
                let rgb = srgb_to_display_p3([p[0], p[1], p[2]]);
                p[..3].copy_from_slice(&rgb);
            }
        });
    }

    fn encode<const BYTES: usize>(&self, with_alpha: bool) -> Vec<u8> {
        let ch = if with_alpha { 4 } else { 3 };
        let (wu, hu) = (self.w as usize, self.h as usize);
        let mut out = vec![0u8; wu * hu * ch * BYTES];
        let scale = if BYTES == 2 { 65535.0 } else { 255.0 };
        par_rows(&mut out, wu * ch * BYTES, |y, row| {
            for x in 0..wu {
                let p = &self.px[(y * wu + x) * 4..(y * wu + x) * 4 + 4];
                for c in 0..ch {
                    let v = (p[c].clamp(0.0, 1.0) * scale + 0.5) as u32;
                    if BYTES == 2 {
                        row[(x * ch + c) * 2] = (v >> 8) as u8;
                        row[(x * ch + c) * 2 + 1] = v as u8;
                    } else {
                        row[x * ch + c] = v as u8;
                    }
                }
            }
        });
        out
    }

    /// Bytes RGB(A) de 8 bits (a imagem deve estar com alfa reto).
    pub fn encode8(&self, with_alpha: bool) -> Vec<u8> {
        self.encode::<1>(with_alpha)
    }

    /// Bytes RGB(A) de 16 bits big-endian (formato do PNG).
    pub fn encode16(&self, with_alpha: bool) -> Vec<u8> {
        self.encode::<2>(with_alpha)
    }

    /// Miniatura RGBA8 reta (média em blocos, pré-multiplicada) com o maior lado ≤ `max_side`.
    pub fn thumbnail_rgba8(&self, max_side: u32) -> (u32, u32, Vec<u8>) {
        let f = (self.w.max(self.h)).div_ceil(max_side.max(1)).max(1) as usize;
        let (tw, th) = ((self.w as usize).div_ceil(f), (self.h as usize).div_ceil(f));
        let mut out = vec![0u8; tw * th * 4];
        for ty in 0..th {
            for tx in 0..tw {
                let mut sum = [0.0f32; 4];
                let mut n = 0.0;
                for y in ty * f..((ty + 1) * f).min(self.h as usize) {
                    for x in tx * f..((tx + 1) * f).min(self.w as usize) {
                        let o = (y * self.w as usize + x) * 4;
                        for c in 0..4 {
                            sum[c] += self.px[o + c];
                        }
                        n += 1.0;
                    }
                }
                let a = sum[3] / n;
                let o = (ty * tw + tx) * 4;
                for c in 0..3 {
                    let v = if a > 1e-6 { (sum[c] / n / a).min(1.0) } else { 0.0 };
                    out[o + c] = (v * 255.0 + 0.5) as u8;
                }
                out[o + 3] = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        (tw as u32, th as u32, out)
    }
}

// ------------------------------------------------------------------------------------------------
// Cor
// ------------------------------------------------------------------------------------------------

/// sRGB codificado → linear.
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear → sRGB codificado (a transferência do Display P3 é a mesma).
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Matriz linear-sRGB → linear-Display-P3 (D65 → D65, só muda as primárias).
pub const SRGB_TO_P3: [[f32; 3]; 3] = [[0.822_462_1, 0.177_538_0, 0.0], [0.033_194_1, 0.966_805_8, 0.0], [0.017_082_7, 0.072_397_4, 0.910_519_9]];

/// Reencoda uma cor sRGB (codificada) para Display P3 (codificada com a mesma curva). Cores do sRGB ficam dentro do P3.
pub fn srgb_to_display_p3(c: [f32; 3]) -> [f32; 3] {
    let l = [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])];
    let m = SRGB_TO_P3;
    [
        linear_to_srgb(m[0][0] * l[0] + m[0][1] * l[1] + m[0][2] * l[2]),
        linear_to_srgb(m[1][0] * l[0] + m[1][1] * l[1] + m[1][2] * l[2]),
        linear_to_srgb(m[2][0] * l[0] + m[2][1] * l[1] + m[2][2] * l[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|_| rgba).collect()
    }

    #[test]
    fn box_filter_averages_with_more_than_8_bits_of_precision() {
        // 2×2 → 1 pixel: 0,0,0,255 e 255 em três vizinhos → 0,75 de luminância
        let mut src = solid(2, 2, [255, 255, 255, 255]);
        src[0..4].copy_from_slice(&[0, 0, 0, 255]);
        let img = FloatImage::from_rgba8_box(&src, 1, 1, 2);
        assert!((img.px[0] - 0.75).abs() < 1e-6);
        assert_eq!(img.encode8(false), vec![191, 191, 191]); // 0,75 × 255 = 191,25
        let b16 = img.encode16(false);
        assert_eq!(&b16[0..2], &[0xBF, 0xFF], "0,75 × 65535 = 49151,25 → 0xBFFF (a média sobrevive em 16 bits)");
    }

    #[test]
    fn premultiplied_alpha_round_trips_to_straight_color() {
        // pixel de borda: cor (200,100,50) com cobertura 50 % → pré-multiplicado (100,50,25,128)
        let src = [100u8, 50, 25, 128];
        let mut img = FloatImage::from_rgba8_box(&src, 1, 1, 1);
        img.unpremultiply();
        let out = img.encode8(true);
        assert!((out[0] as i32 - 199).abs() <= 1 && (out[1] as i32 - 99).abs() <= 1 && (out[2] as i32 - 50).abs() <= 1, "{out:?}");
        assert_eq!(out[3], 128);
        // sem cobertura: tudo zero (sem franja de cor)
        let mut empty = FloatImage::from_rgba8_box(&[9, 9, 9, 0], 1, 1, 1);
        empty.unpremultiply();
        assert_eq!(empty.encode8(true), vec![0, 0, 0, 0]);
    }

    #[test]
    fn flatten_composites_over_the_background() {
        let mut img = FloatImage::from_rgba8_box(&[100, 50, 25, 128], 1, 1, 1);
        img.flatten_over([1.0, 1.0, 1.0]);
        let out = img.encode8(false);
        // 100/255 + (1 − 128/255) × 1 ≈ 0,8922 → 227
        assert!((out[0] as i32 - 227).abs() <= 1, "{out:?}");
    }

    #[test]
    fn p3_matrix_matches_the_primaries_and_keeps_neutrals() {
        // deriva a matriz a partir das cromaticidades (xy) de sRGB e Display P3 (D65)
        fn rgb_to_xyz(r: (f32, f32), g: (f32, f32), b: (f32, f32), w: (f32, f32)) -> [[f32; 3]; 3] {
            let col = |p: (f32, f32)| [p.0 / p.1, 1.0, (1.0 - p.0 - p.1) / p.1];
            let (cr, cg, cb, cw) = (col(r), col(g), col(b), col(w));
            // resolve S tal que [cr cg cb] · S = cw
            let m = [[cr[0], cg[0], cb[0]], [cr[1], cg[1], cb[1]], [cr[2], cg[2], cb[2]]];
            let det = |m: [[f32; 3]; 3]| {
                m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
            };
            let d = det(m);
            let mut s = [0.0; 3];
            for i in 0..3 {
                let mut mi = m;
                for r in 0..3 {
                    mi[r][i] = [cw[0], cw[1], cw[2]][r];
                }
                s[i] = det(mi) / d;
            }
            [[m[0][0] * s[0], m[0][1] * s[1], m[0][2] * s[2]], [m[1][0] * s[0], m[1][1] * s[1], m[1][2] * s[2]], [m[2][0] * s[0], m[2][1] * s[1], m[2][2] * s[2]]]
        }
        fn inv(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
            let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            let c = |r: usize, c: usize| {
                let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
                let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
                (m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1]) / d
            };
            let mut o = [[0.0; 3]; 3];
            for r in 0..3 {
                for cc in 0..3 {
                    o[r][cc] = c(cc, r);
                }
            }
            o
        }
        let w = (0.3127, 0.3290);
        let srgb = rgb_to_xyz((0.64, 0.33), (0.30, 0.60), (0.15, 0.06), w);
        let p3 = rgb_to_xyz((0.68, 0.32), (0.265, 0.69), (0.15, 0.06), w);
        let p3inv = inv(p3);
        for r in 0..3 {
            for c in 0..3 {
                let v: f32 = (0..3).map(|k| p3inv[r][k] * srgb[k][c]).sum();
                assert!((v - SRGB_TO_P3[r][c]).abs() < 2e-4, "[{r}][{c}] {v} vs {}", SRGB_TO_P3[r][c]);
            }
        }
        for v in [0.0, 0.5, 1.0] {
            let o = srgb_to_display_p3([v, v, v]);
            assert!(o.iter().all(|c| (c - v).abs() < 2e-3), "neutro {v} → {o:?}");
        }
        // vermelho puro sRGB cai dentro do P3 (menos saturado)
        let red = srgb_to_display_p3([1.0, 0.0, 0.0]);
        assert!(red[0] < 1.0 && red[0] > 0.9 && red[1] > 0.1 && red[1] < 0.3, "{red:?}");
    }

    #[test]
    fn thumbnail_keeps_aspect_and_straight_color() {
        let src = solid(8, 4, [255, 0, 0, 255]);
        let img = FloatImage::from_rgba8_box(&src, 8, 4, 1);
        let (w, h, px) = img.thumbnail_rgba8(4);
        assert_eq!((w, h), (4, 2));
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn parallel_rows_cover_every_row_exactly_once() {
        let mut v = vec![0u32; 37 * 5];
        par_rows(&mut v, 5, |y, row| {
            for c in row.iter_mut() {
                *c = y as u32 + 1;
            }
        });
        for (i, c) in v.iter().enumerate() {
            assert_eq!(*c, (i / 5) as u32 + 1);
        }
    }
}
