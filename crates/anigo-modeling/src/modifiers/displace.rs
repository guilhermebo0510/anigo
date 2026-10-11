//! Modificador **Displace**: desloca vértices por uma textura (constante,
//! ruído procedural determinístico ou imagem amostrada por UV), equivalente ao
//! Displace Modifier do Blender. Serve para rugas, tecido, terreno, pele de
//! dragão, etc.
//!
//! Deslocamento por vértice: `(textura − midlevel) · strength · peso`, aplicado
//! ao longo da normal do vértice ([`DisplaceDirection::Normal`], calculada na
//! malha **antes** de deslocar) ou de um eixo fixo.
//!
//! O ruído é próprio (sem dependências): hash inteiro + gradientes de Perlin,
//! totalmente determinístico para um dado `seed`.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::normals::compute_vertex_normals;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Direção do deslocamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DisplaceDirection {
    /// Ao longo da normal do vértice.
    #[default]
    Normal,
    X,
    Y,
    Z,
}

/// Fonte da textura (valores em `0..1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplaceSource {
    /// Valor constante `1.0` (como o Blender sem textura): todos os vértices
    /// andam `(1 − midlevel) · strength`.
    Constant,
    /// Ruído de Perlin 3D amostrado na posição do vértice dividida por `scale`
    /// (tamanho da feature em unidades de mundo), remapeado para `0..1`.
    Noise { scale: f32, seed: u32 },
    /// Imagem em tons de cinza `w × h` (`data.len() == w·h`, valores `0..1`,
    /// linha 0 em `v = 0`) amostrada bilinearmente pelo UV dos cantos, com
    /// repetição (wrap).
    Image { w: u32, h: u32, data: Vec<f32> },
}

/// Parâmetros do Displace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaceParams {
    /// Intensidade (unidades de mundo para textura 0→1).
    pub strength: f32,
    /// Nível da textura que não desloca (padrão 0.5).
    pub midlevel: f32,
    pub direction: DisplaceDirection,
    pub texture: DisplaceSource,
    /// Peso por vértice (0..1) multiplicado ao deslocamento (ex.: máscara de
    /// grupo de vértices). `None` = 1 em todos. Comprimento deve ser igual ao de `verts`.
    pub vertex_weights: Option<Vec<f32>>,
}

impl Default for DisplaceParams {
    fn default() -> Self {
        Self {
            strength: 1.0,
            midlevel: 0.5,
            direction: DisplaceDirection::Normal,
            texture: DisplaceSource::Constant,
            vertex_weights: None,
        }
    }
}

// ---------------------------------------------------------------- ruído

#[inline]
fn hash3(ix: i32, iy: i32, iz: i32, seed: u32) -> u32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    for v in [ix as u32, iy as u32, iz as u32] {
        h ^= v.wrapping_mul(0xC2B2_AE35);
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xE654_6B64);
        h ^= h >> 16;
        h = h.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 13;
    }
    h ^= h >> 16;
    h
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad(h: u32, x: f32, y: f32, z: f32) -> f32 {
    // 12 arestas do cubo (Perlin melhorado)
    match h % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

/// Ruído de Perlin 3D determinístico, aproximadamente em `[-1, 1]` e igual a
/// zero nos pontos inteiros da malha.
pub fn perlin_noise(p: Vec3, seed: u32) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (x, y, z) = (p.x - fx, p.y - fy, p.z - fz);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let g = |dx: i32, dy: i32, dz: i32| {
        grad(
            hash3(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed),
            x - dx as f32,
            y - dy as f32,
            z - dz as f32,
        )
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    lerp(y0, y1, w).clamp(-1.0, 1.0)
}

/// Ruído de valor 3D determinístico em `[0, 1]` (interpolação suave de valores
/// pseudo-aleatórios nos pontos inteiros). Mais "blocado" que o Perlin.
pub fn value_noise(p: Vec3, seed: u32) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (u, v, w) = (fade(p.x - fx), fade(p.y - fy), fade(p.z - fz));
    let val = |dx: i32, dy: i32, dz: i32| {
        hash3(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed) as f32 / u32::MAX as f32
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(val(0, 0, 0), val(1, 0, 0), u);
    let x10 = lerp(val(0, 1, 0), val(1, 1, 0), u);
    let x01 = lerp(val(0, 0, 1), val(1, 0, 1), u);
    let x11 = lerp(val(0, 1, 1), val(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Amostra bilinear de uma imagem `w × h` (linha 0 em `v = 0`) com repetição.
/// Devolve 0 se as dimensões forem inválidas.
pub fn sample_image(w: u32, h: u32, data: &[f32], uv: Vec2) -> f32 {
    if w == 0 || h == 0 || data.len() != (w as usize) * (h as usize) || !uv.is_finite() {
        return 0.0;
    }
    let x = uv.x * w as f32 - 0.5;
    let y = uv.y * h as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let wrap = |i: f32, n: u32| (i as i64).rem_euclid(n as i64) as usize;
    let (xa, xb) = (wrap(x0, w), wrap(x0 + 1.0, w));
    let (ya, yb) = (wrap(y0, h), wrap(y0 + 1.0, h));
    let px = |xx: usize, yy: usize| data[yy * w as usize + xx];
    let top = px(xa, ya) * (1.0 - fx) + px(xb, ya) * fx;
    let bot = px(xa, yb) * (1.0 - fx) + px(xb, yb) * fx;
    top * (1.0 - fy) + bot * fy
}

impl DisplaceParams {
    fn validate(&self, mesh: &EditMesh) -> ModelResult<()> {
        if !self.strength.is_finite() {
            return Err(ModelError::InvalidParameter("strength não finito".into()));
        }
        if !self.midlevel.is_finite() {
            return Err(ModelError::InvalidParameter("midlevel não finito".into()));
        }
        match &self.texture {
            DisplaceSource::Constant => {}
            DisplaceSource::Noise { scale, .. } => {
                if !scale.is_finite() || *scale <= 0.0 {
                    return Err(ModelError::InvalidParameter(format!(
                        "scale do ruído deve ser finito e > 0, recebeu {scale}"
                    )));
                }
            }
            DisplaceSource::Image { w, h, data } => {
                if *w == 0 || *h == 0 {
                    return Err(ModelError::InvalidParameter("imagem com largura/altura zero".into()));
                }
                let expected = (*w as usize).checked_mul(*h as usize);
                if expected != Some(data.len()) {
                    return Err(ModelError::InvalidParameter(format!(
                        "imagem {w}x{h} exige {} valores, recebeu {}",
                        (*w as u64) * (*h as u64),
                        data.len()
                    )));
                }
                if data.iter().any(|v| !v.is_finite()) {
                    return Err(ModelError::InvalidParameter("imagem com valores não finitos".into()));
                }
            }
        }
        if let Some(w) = &self.vertex_weights {
            if w.len() != mesh.verts.len() {
                return Err(ModelError::InvalidParameter(format!(
                    "vertex_weights tem {} valores para {} vértices",
                    w.len(),
                    mesh.verts.len()
                )));
            }
            if w.iter().any(|x| !x.is_finite()) {
                return Err(ModelError::InvalidParameter("vertex_weights com valor não finito".into()));
            }
        }
        Ok(())
    }
}

/// Aplica o deslocamento e devolve a malha nova (a entrada não é alterada;
/// topologia e atributos são mantidos, só `co` muda).
///
/// * Textura `Image`: o valor de cada vértice é a **média** das amostras bilineares
///   nos UVs de todos os cantos que o usam (costuras de UV não rasgam a malha).
///   Vértices sem faces recebem `midlevel` (deslocamento nulo).
/// * Textura `Noise`: usa o ruído de Perlin em `posição / scale`, remapeado
///   para `0..1`.
/// * `Normal` usa as normais suaves de [`compute_vertex_normals`] da malha original.
///
/// Erros: parâmetros não finitos, `scale` ≤ 0, imagem com dimensões/dados
/// inconsistentes, `vertex_weights` de tamanho errado ou malha inválida.
pub fn displace(mesh: &EditMesh, params: &DisplaceParams) -> ModelResult<EditMesh> {
    params.validate(mesh)?;
    mesh.validate()?;
    let n = mesh.verts.len();

    // textura por vértice
    let tex: Vec<f32> = match &params.texture {
        DisplaceSource::Constant => vec![1.0; n],
        DisplaceSource::Noise { scale, seed } => mesh
            .verts
            .iter()
            .map(|v| perlin_noise(v.co / *scale, *seed) * 0.5 + 0.5)
            .collect(),
        DisplaceSource::Image { w, h, data } => {
            let mut acc = vec![0.0f32; n];
            let mut cnt = vec![0u32; n];
            for f in &mesh.faces {
                for c in &f.corners {
                    acc[c.vert as usize] += sample_image(*w, *h, data, c.uv);
                    cnt[c.vert as usize] += 1;
                }
            }
            (0..n)
                .map(|i| if cnt[i] == 0 { params.midlevel } else { acc[i] / cnt[i] as f32 })
                .collect()
        }
    };
    let normals = if params.direction == DisplaceDirection::Normal {
        Some(compute_vertex_normals(mesh))
    } else {
        None
    };

    let mut out = mesh.clone();
    for (i, v) in out.verts.iter_mut().enumerate() {
        let weight = params.vertex_weights.as_ref().map_or(1.0, |w| w[i]);
        let amount = (tex[i] - params.midlevel) * params.strength * weight;
        let dir = match params.direction {
            DisplaceDirection::Normal => normals.as_ref().map_or(Vec3::ZERO, |nn| nn[i]),
            DisplaceDirection::X => Vec3::X,
            DisplaceDirection::Y => Vec3::Y,
            DisplaceDirection::Z => Vec3::Z,
        };
        v.co += dir * amount;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn noise_params(strength: f32, scale: f32, seed: u32) -> DisplaceParams {
        DisplaceParams {
            strength,
            texture: DisplaceSource::Noise { scale, seed },
            ..DisplaceParams::default()
        }
    }

    #[test]
    fn constant_normal_displacement_inflates_sphere() {
        let s = primitives::quad_sphere(1.0, 6);
        let m = displace(
            &s,
            &DisplaceParams {
                strength: 0.2,
                midlevel: 0.0,
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        // textura 1, midlevel 0 => +0.2 ao longo da normal (radial na esfera)
        for (a, b) in s.verts.iter().zip(&m.verts) {
            let dr = b.co.length() - a.co.length();
            assert!((dr - 0.2).abs() < 0.01, "{dr}");
            // topologia e atributos intactos
            assert_eq!(a.color, b.color);
        }
        assert_eq!(s.faces, m.faces);
        // midlevel 1 => sem deslocamento
        let same = displace(
            &s,
            &DisplaceParams {
                midlevel: 1.0,
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        assert_eq!(same.content_hash(), s.content_hash());
        // midlevel padrão 0.5 com textura constante: (1 - .5) * strength
        let half = displace(&s, &DisplaceParams::default()).unwrap();
        assert!(((half.verts[3].co.length() - s.verts[3].co.length()) - 0.5).abs() < 0.01);
    }

    #[test]
    fn axis_directions() {
        let c = primitives::cube(1.0);
        for (dir, axis) in [
            (DisplaceDirection::X, Vec3::X),
            (DisplaceDirection::Y, Vec3::Y),
            (DisplaceDirection::Z, Vec3::Z),
        ] {
            let m = displace(
                &c,
                &DisplaceParams {
                    strength: 2.0,
                    midlevel: 0.0,
                    direction: dir,
                    ..DisplaceParams::default()
                },
            )
            .unwrap();
            for (a, b) in c.verts.iter().zip(&m.verts) {
                assert!((b.co - a.co - axis * 2.0).length() < 1e-6);
            }
        }
        // strength negativo inverte
        let m = displace(
            &c,
            &DisplaceParams {
                strength: -1.0,
                midlevel: 0.0,
                direction: DisplaceDirection::Y,
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        assert!((m.verts[0].co.y - (c.verts[0].co.y - 1.0)).abs() < 1e-6);
    }

    #[test]
    fn perlin_properties() {
        // zero nos pontos da malha inteira
        for (x, y, z) in [(0, 0, 0), (1, 2, 3), (-4, 5, -6)] {
            assert_eq!(perlin_noise(Vec3::new(x as f32, y as f32, z as f32), 7), 0.0);
        }
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        let mut sum = 0.0f64;
        let n = 20_000;
        for i in 0..n {
            let t = i as f32;
            let p = Vec3::new(t * 0.137, t * 0.071 + 3.0, t * 0.0293 - 7.0);
            let v = perlin_noise(p, 1);
            assert!(v.is_finite() && (-1.0..=1.0).contains(&v));
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v as f64;
        }
        assert!(lo < -0.5 && hi > 0.5, "faixa [{lo}, {hi}]");
        assert!((sum / n as f64).abs() < 0.05, "média {}", sum / n as f64);
        // contínuo (Lipschitz): passos pequenos => mudanças pequenas
        for i in 0..2000 {
            let p = Vec3::new(i as f32 * 0.31, i as f32 * 0.17, 0.5);
            let d = (perlin_noise(p, 3) - perlin_noise(p + Vec3::splat(1e-3), 3)).abs();
            assert!(d < 0.02, "{d}");
        }
        // determinismo e dependência do seed
        let p = Vec3::new(0.3, 0.7, 0.1);
        assert_eq!(perlin_noise(p, 5), perlin_noise(p, 5));
        let differ = (0..50).any(|i| {
            let q = p + Vec3::splat(i as f32 * 0.37);
            perlin_noise(q, 5) != perlin_noise(q, 6)
        });
        assert!(differ);
        // valores muito grandes/negativos não entram em pânico
        assert!(perlin_noise(Vec3::splat(1e9), 0).is_finite());
        assert!(perlin_noise(Vec3::splat(-1e9), 0).is_finite());
    }

    #[test]
    fn value_noise_is_in_unit_range_and_continuous() {
        for i in 0..5000 {
            let p = Vec3::new(i as f32 * 0.123, -(i as f32) * 0.077, 4.0);
            let v = value_noise(p, 9);
            assert!((0.0..=1.0).contains(&v));
            let w = value_noise(p + Vec3::splat(1e-3), 9);
            assert!((v - w).abs() < 0.02);
        }
        assert_eq!(value_noise(Vec3::new(0.2, 0.3, 0.4), 1), value_noise(Vec3::new(0.2, 0.3, 0.4), 1));
    }

    #[test]
    fn noise_displacement_is_deterministic_and_bounded() {
        let s = primitives::quad_sphere(1.0, 8);
        let a = displace(&s, &noise_params(0.3, 0.5, 11)).unwrap();
        let b = displace(&s, &noise_params(0.3, 0.5, 11)).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        let c = displace(&s, &noise_params(0.3, 0.5, 12)).unwrap();
        assert_ne!(a.content_hash(), c.content_hash());
        // |deslocamento| <= 0.5 * strength (textura em 0..1, midlevel 0.5)
        for (o, d) in s.verts.iter().zip(&a.verts) {
            assert!((d.co - o.co).length() <= 0.15 + 1e-5);
        }
        assert!(a.verts.iter().zip(&s.verts).any(|(d, o)| (d.co - o.co).length() > 1e-4));
    }

    #[test]
    fn image_displacement_follows_uv() {
        // imagem 2x2: [0 1; 0 1] (cresce com u)
        let data = vec![0.0, 1.0, 0.0, 1.0];
        assert!((sample_image(2, 2, &data, Vec2::new(0.25, 0.25))).abs() < 1e-6);
        assert!((sample_image(2, 2, &data, Vec2::new(0.75, 0.5)) - 1.0).abs() < 1e-6);
        assert!((sample_image(2, 2, &data, Vec2::new(0.5, 0.5)) - 0.5).abs() < 1e-6);
        // wrap: u=1.25 equivale a u=0.25
        assert!((sample_image(2, 2, &data, Vec2::new(1.25, 0.25))).abs() < 1e-6);
        assert!((sample_image(2, 2, &data, Vec2::new(-0.75, 0.5)) - sample_image(2, 2, &data, Vec2::new(0.25, 0.5))).abs() < 1e-6);
        // dados inválidos: 0
        assert_eq!(sample_image(0, 2, &data, Vec2::ZERO), 0.0);
        assert_eq!(sample_image(2, 2, &data[..3], Vec2::ZERO), 0.0);
        assert_eq!(sample_image(2, 2, &data, Vec2::NAN), 0.0);

        // plano com UV 0..1: deslocamento em Y proporcional à imagem
        let g = primitives::grid(8, 8, 1.0, 1.0);
        let m = displace(
            &g,
            &DisplaceParams {
                strength: 1.0,
                midlevel: 0.0,
                direction: DisplaceDirection::Y,
                texture: DisplaceSource::Image { w: 2, h: 2, data },
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for v in &m.verts {
            lo = lo.min(v.co.y);
            hi = hi.max(v.co.y);
            assert!((0.0..=1.0).contains(&v.co.y));
        }
        assert!(lo < 0.05 && hi > 0.95, "[{lo}, {hi}]");
        // centro do texel esquerdo (u = .25) vale 0 e o direito (u = .75) vale 1 (9 vértices cada)
        let left = m.verts.iter().filter(|v| (v.co.x + 0.25).abs() < 1e-4).map(|v| v.co.y).sum::<f32>();
        let right = m.verts.iter().filter(|v| (v.co.x - 0.25).abs() < 1e-4).map(|v| v.co.y).sum::<f32>();
        assert!(left < 1e-4 && (right - 9.0).abs() < 1e-3, "{left} {right}");
    }

    #[test]
    fn uv_seams_average_instead_of_tearing() {
        // cubo: 24 cantos com UV diferentes por face => vértice compartilhado vira média
        let c = primitives::cube(1.0);
        let data = vec![0.0, 1.0, 1.0, 0.0];
        let m = displace(
            &c,
            &DisplaceParams {
                strength: 1.0,
                midlevel: 0.0,
                direction: DisplaceDirection::Y,
                texture: DisplaceSource::Image { w: 2, h: 2, data },
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        assert_eq!(m.vert_count(), c.vert_count());
        m.validate().unwrap();
        assert!(m.verts.iter().all(|v| v.co.is_finite()));
        // vértice sem faces recebe midlevel (deslocamento 0)
        let mut iso = c.clone();
        let extra = iso.add_vert(Vec3::splat(2.0));
        let r = displace(
            &iso,
            &DisplaceParams {
                strength: 5.0,
                midlevel: 0.3,
                direction: DisplaceDirection::Y,
                texture: DisplaceSource::Image {
                    w: 1,
                    h: 1,
                    data: vec![0.9],
                },
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        assert_eq!(r.verts[extra as usize].co, Vec3::splat(2.0));
    }

    #[test]
    fn vertex_weights_scale_the_effect() {
        let g = primitives::grid(2, 2, 1.0, 1.0);
        let w: Vec<f32> = (0..g.vert_count()).map(|i| if i == 4 { 1.0 } else { 0.0 }).collect();
        let m = displace(
            &g,
            &DisplaceParams {
                strength: 1.0,
                midlevel: 0.0,
                vertex_weights: Some(w),
                ..DisplaceParams::default()
            },
        )
        .unwrap();
        for (i, v) in m.verts.iter().enumerate() {
            if i == 4 {
                assert!((v.co.y - 1.0).abs() < 1e-6);
            } else {
                assert_eq!(v.co, g.verts[i].co);
            }
        }
    }

    #[test]
    fn invalid_parameters_leave_input_untouched() {
        let g = primitives::grid(2, 2, 1.0, 1.0);
        let before = g.clone();
        let bads = vec![
            DisplaceParams {
                strength: f32::NAN,
                ..DisplaceParams::default()
            },
            DisplaceParams {
                midlevel: f32::INFINITY,
                ..DisplaceParams::default()
            },
            noise_params(1.0, 0.0, 0),
            noise_params(1.0, -1.0, 0),
            noise_params(1.0, f32::NAN, 0),
            DisplaceParams {
                texture: DisplaceSource::Image {
                    w: 0,
                    h: 4,
                    data: vec![],
                },
                ..DisplaceParams::default()
            },
            DisplaceParams {
                texture: DisplaceSource::Image {
                    w: 2,
                    h: 2,
                    data: vec![0.0; 3],
                },
                ..DisplaceParams::default()
            },
            DisplaceParams {
                texture: DisplaceSource::Image {
                    w: 1,
                    h: 1,
                    data: vec![f32::NAN],
                },
                ..DisplaceParams::default()
            },
            DisplaceParams {
                vertex_weights: Some(vec![1.0; 2]),
                ..DisplaceParams::default()
            },
            DisplaceParams {
                vertex_weights: Some(vec![f32::NAN; g.vert_count()]),
                ..DisplaceParams::default()
            },
        ];
        for b in bads {
            assert!(matches!(displace(&g, &b), Err(ModelError::InvalidParameter(_))), "{b:?}");
        }
        assert_eq!(g, before);
        let mut broken = g.clone();
        broken.faces[0].corners[0].vert = 99;
        assert!(displace(&broken, &DisplaceParams::default()).is_err());
        // imagem gigante com dimensões que estouram usize não entra em pânico
        let huge = DisplaceParams {
            texture: DisplaceSource::Image {
                w: u32::MAX,
                h: u32::MAX,
                data: vec![0.0; 4],
            },
            ..DisplaceParams::default()
        };
        assert!(displace(&g, &huge).is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let p = DisplaceParams {
            texture: DisplaceSource::Image {
                w: 2,
                h: 1,
                data: vec![0.25, 0.75],
            },
            vertex_weights: Some(vec![0.5; 3]),
            direction: DisplaceDirection::Z,
            ..DisplaceParams::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<DisplaceParams>(&json).unwrap(), p);
        let n = noise_params(2.0, 3.0, 4);
        assert_eq!(serde_json::from_str::<DisplaceParams>(&serde_json::to_string(&n).unwrap()).unwrap(), n);
    }

    #[test]
    fn large_mesh_is_fast() {
        let s = primitives::quad_sphere(1.0, 64);
        let t = std::time::Instant::now();
        let m = displace(&s, &noise_params(0.05, 0.3, 1)).unwrap();
        assert_eq!(m.vert_count(), s.vert_count());
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }
}
