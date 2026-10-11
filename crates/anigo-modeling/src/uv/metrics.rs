//! Métricas de qualidade de UV: área, estiramento, sobreposição, densidade de texels,
//! faces invertidas e um relatório agregado serializável.
//!
//! Convenção: um polígono UV com área assinada **positiva** (anti-horário no plano UV,
//! v para cima) mantém o enrolamento da face 3D; área negativa = face **invertida**.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::uv::seams::split_islands;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// Tolerância de interpenetração (espaço UV) abaixo da qual duas formas apenas "se tocam".
const OVERLAP_EPS: f32 = 1e-6;

// ---------------------------------------------------------------------- áreas e limites

/// Área assinada (fórmula do cadarço) de um polígono 2D, acumulada em `f64`.
pub(crate) fn poly_signed_area(pts: &[Vec2]) -> f64 {
    let n = pts.len();
    let mut a = 0.0f64;
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        a += p.x as f64 * q.y as f64 - q.x as f64 * p.y as f64;
    }
    a * 0.5
}

fn face_uvs(mesh: &EditMesh, f: u32) -> Vec<Vec2> {
    mesh.faces[f as usize].corners.iter().map(|c| c.uv).collect()
}

/// Área assinada do polígono UV da face `f` (0 se o índice for inválido).
pub fn face_uv_area_signed(mesh: &EditMesh, f: u32) -> f32 {
    if f as usize >= mesh.faces.len() {
        return 0.0;
    }
    poly_signed_area(&face_uvs(mesh, f)) as f32
}

/// Área total ocupada no espaço UV: soma dos módulos das áreas dos polígonos.
/// (Sobreposições contam em dobro — compare com [`overlap_count`].)
pub fn uv_area(mesh: &EditMesh) -> f32 {
    (0..mesh.faces.len() as u32)
        .map(|f| poly_signed_area(&face_uvs(mesh, f)).abs())
        .sum::<f64>() as f32
}

/// Caixa envolvente `(min, max)` de todos os UVs de corners, `None` se não há faces.
pub fn uv_bounds(mesh: &EditMesh) -> Option<(Vec2, Vec2)> {
    let mut it = mesh.faces.iter().flat_map(|f| f.corners.iter().map(|c| c.uv));
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Faces cujo polígono UV tem área assinada negativa (enrolamento invertido), em ordem crescente.
pub fn flipped_faces(mesh: &EditMesh) -> Vec<u32> {
    (0..mesh.faces.len() as u32)
        .filter(|&f| poly_signed_area(&face_uvs(mesh, f)) < -1e-12)
        .collect()
}

/// Hash FNV-1a dos UVs de todos os corners (bits exatos). Complementa
/// [`EditMesh::content_hash`], que não cobre UVs; usado nos testes de determinismo.
pub fn uv_hash(mesh: &EditMesh) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |b: u32| {
        for byte in b.to_le_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    for f in &mesh.faces {
        mix(f.corners.len() as u32);
        for c in &f.corners {
            mix(c.uv.x.to_bits());
            mix(c.uv.y.to_bits());
        }
    }
    h
}

// ---------------------------------------------------------------------- estiramento

/// Valores singulares `(σ1 ≥ σ2)` e determinante da Jacobiana do mapa afim 3D→UV de um
/// triângulo (medida no plano do triângulo). `None` se o triângulo 3D é degenerado.
pub(crate) fn tri_sigmas(p: [Vec3; 3], uv: [Vec2; 3]) -> Option<(f64, f64, f64)> {
    let e1 = (p[1] - p[0]).as_dvec3();
    let e2 = (p[2] - p[0]).as_dvec3();
    let a1 = e1.length();
    if a1 < 1e-12 {
        return None;
    }
    let h = e1.cross(e2).length() / a1;
    if h < 1e-9 * a1 {
        return None;
    }
    let x2 = e1.dot(e2) / a1;
    let f1 = (uv[1] - uv[0]).as_dvec2();
    let f2 = (uv[2] - uv[0]).as_dvec2();
    let j00 = f1.x / a1;
    let j10 = f1.y / a1;
    let j01 = (-f1.x * x2 + f2.x * a1) / (a1 * h);
    let j11 = (-f1.y * x2 + f2.y * a1) / (a1 * h);
    let det = j00 * j11 - j01 * j10;
    let sum = j00 * j00 + j01 * j01 + j10 * j10 + j11 * j11;
    let s_plus = (sum + 2.0 * det.abs()).max(0.0).sqrt();
    let s_minus = (sum - 2.0 * det.abs()).max(0.0).sqrt();
    Some(((s_plus + s_minus) * 0.5, (s_plus - s_minus) * 0.5, det))
}

/// Estiramento anisotrópico da face `f`: razão `σ1/σ2` dos valores singulares da Jacobiana
/// 3D→UV (1 = conforme/isométrico localmente; cresce com a distorção). Para n-gons, é a
/// média ponderada pela área 3D dos triângulos de canto `(i-1, i, i+1)`.
///
/// Retornos especiais: `NaN` se `f` é inválido; `INFINITY` se o UV colapsou (σ2 ≈ 0);
/// `1.0` se a face 3D é degenerada (sem Jacobiana definida). Independe de escala/rotação do UV.
pub fn face_stretch(mesh: &EditMesh, f: u32) -> f32 {
    face_stretch_info(mesh, f).map_or(f32::NAN, |s| s.0)
}

/// `(stretch, area3d_ponderação)` da face; `None` para índice inválido.
fn face_stretch_info(mesh: &EditMesh, f: u32) -> Option<(f32, f64)> {
    let face = mesh.faces.get(f as usize)?;
    let n = face.corners.len();
    let pos: Vec<Vec3> = face.corners.iter().map(|c| mesh.co(c.vert)).collect();
    let uv: Vec<Vec2> = face.corners.iter().map(|c| c.uv).collect();
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    let tris: Vec<[usize; 3]> = if n == 3 {
        vec![[0, 1, 2]]
    } else {
        (0..n).map(|i| [(i + n - 1) % n, i, (i + 1) % n]).collect()
    };
    for t in tris {
        let p = [pos[t[0]], pos[t[1]], pos[t[2]]];
        let q = [uv[t[0]], uv[t[1]], uv[t[2]]];
        let Some((s1, s2, _)) = tri_sigmas(p, q) else {
            continue;
        };
        let area = ((p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5) as f64;
        if s1 <= 1e-30 || s2 <= 1e-9 * s1 {
            return Some((f32::INFINITY, mesh.face_area(f) as f64));
        }
        num += area * (s1 / s2);
        den += area;
    }
    if den <= 0.0 {
        return Some((1.0, 0.0));
    }
    Some(((num / den) as f32, mesh.face_area(f) as f64))
}

/// Estatísticas de estiramento de uma ilha.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandStretch {
    /// Índice da ilha na ordem de [`split_islands`].
    pub island: usize,
    pub faces: usize,
    /// Média simples de `face_stretch` (ignora faces colapsadas).
    pub mean: f32,
    /// Máximo de `face_stretch` entre faces não colapsadas.
    pub max: f32,
    /// Média ponderada pela área 3D.
    pub area_weighted_mean: f32,
    /// Faces com UV colapsado (σ2 ≈ 0).
    pub degenerate: usize,
    /// Faces com enrolamento invertido.
    pub flipped: usize,
}

/// Estatísticas de estiramento por ilha (ilhas de [`split_islands`], ordem determinística).
pub fn island_stretch_stats(mesh: &EditMesh) -> Vec<IslandStretch> {
    split_islands(mesh)
        .iter()
        .enumerate()
        .map(|(i, isl)| {
            let (mut sum, mut wsum, mut wden, mut max) = (0.0f64, 0.0f64, 0.0f64, 1.0f32);
            let (mut ok, mut degenerate, mut flipped) = (0usize, 0usize, 0usize);
            for &f in &isl.faces {
                if poly_signed_area(&face_uvs(mesh, f)) < -1e-12 {
                    flipped += 1;
                }
                match face_stretch_info(mesh, f) {
                    Some((s, w)) if s.is_finite() => {
                        sum += s as f64;
                        wsum += s as f64 * w;
                        wden += w;
                        max = max.max(s);
                        ok += 1;
                    }
                    Some(_) => degenerate += 1,
                    None => {}
                }
            }
            IslandStretch {
                island: i,
                faces: isl.faces.len(),
                mean: if ok > 0 { (sum / ok as f64) as f32 } else { 1.0 },
                max,
                area_weighted_mean: if wden > 0.0 { (wsum / wden) as f32 } else { 1.0 },
                degenerate,
                flipped,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------- sobreposição

struct UTri {
    face: u32,
    island: u32,
    p: [Vec2; 3],
    lo: Vec2,
    hi: Vec2,
}

/// Teste de interseção por eixos separadores; tocar (borda/vértice) não conta.
fn tri_overlap(a: &[Vec2; 3], b: &[Vec2; 3]) -> bool {
    for tri in [a, b] {
        for i in 0..3 {
            let e = tri[(i + 1) % 3] - tri[i];
            let len = e.length();
            if len < 1e-12 {
                continue;
            }
            let axis = Vec2::new(-e.y, e.x) / len;
            let proj = |t: &[Vec2; 3]| {
                let d: [f32; 3] = [axis.dot(t[0]), axis.dot(t[1]), axis.dot(t[2])];
                (d[0].min(d[1]).min(d[2]), d[0].max(d[1]).max(d[2]))
            };
            let ((amin, amax), (bmin, bmax)) = (proj(a), proj(b));
            if amax <= bmin + OVERLAP_EPS || bmax <= amin + OVERLAP_EPS {
                return false;
            }
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OverlapMode {
    /// Pares de faces em ilhas diferentes.
    Between,
    /// Pares de faces na mesma ilha (dobras/autointerseção).
    Within,
}

/// Pares de faces `(a < b)` cujos polígonos UV se sobrepõem, usando grade uniforme.
fn find_overlaps(mesh: &EditMesh, mode: OverlapMode) -> Vec<(u32, u32, u32, u32)> {
    let islands = split_islands(mesh);
    let mut island_of = vec![0u32; mesh.faces.len()];
    for (i, isl) in islands.iter().enumerate() {
        for &f in &isl.faces {
            island_of[f as usize] = i as u32;
        }
    }
    let mut tris: Vec<UTri> = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = face.corners.len();
        for i in 1..n.saturating_sub(1) {
            let p = [face.corners[0].uv, face.corners[i].uv, face.corners[i + 1].uv];
            if poly_signed_area(&p).abs() < 1e-12 || p.iter().any(|q| !q.is_finite()) {
                continue;
            }
            tris.push(UTri {
                face: fi as u32,
                island: island_of[fi],
                lo: p[0].min(p[1]).min(p[2]),
                hi: p[0].max(p[1]).max(p[2]),
                p,
            });
        }
    }
    if tris.len() < 2 {
        return Vec::new();
    }
    let lo = tris.iter().fold(Vec2::splat(f32::INFINITY), |a, t| a.min(t.lo));
    let hi = tris.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, t| a.max(t.hi));
    let g = ((tris.len() as f32).sqrt().ceil() as usize).clamp(1, 256);
    let ext = (hi - lo).max(Vec2::splat(1e-9));
    let cell = |p: Vec2| -> (usize, usize) {
        let c = ((p - lo) / ext * g as f32).floor();
        (
            (c.x.max(0.0) as usize).min(g - 1),
            (c.y.max(0.0) as usize).min(g - 1),
        )
    };
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); g * g];
    for (ti, t) in tris.iter().enumerate() {
        let (x0, y0) = cell(t.lo);
        let (x1, y1) = cell(t.hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                cells[y * g + x].push(ti as u32);
            }
        }
    }
    let mut found: BTreeSet<(u32, u32, u32, u32)> = BTreeSet::new();
    for (ci, list) in cells.iter().enumerate() {
        for (k, &i) in list.iter().enumerate() {
            for &j in &list[k + 1..] {
                let (a, b) = (&tris[i as usize], &tris[j as usize]);
                if a.face == b.face {
                    continue;
                }
                let same = a.island == b.island;
                if (mode == OverlapMode::Between) == same {
                    continue;
                }
                // só testa na célula que contém o canto "máximo dos mínimos" (evita duplicatas)
                let (rx, ry) = cell(a.lo.max(b.lo));
                if ry * g + rx != ci {
                    continue;
                }
                if a.lo.x >= b.hi.x - OVERLAP_EPS
                    || b.lo.x >= a.hi.x - OVERLAP_EPS
                    || a.lo.y >= b.hi.y - OVERLAP_EPS
                    || b.lo.y >= a.hi.y - OVERLAP_EPS
                {
                    continue;
                }
                if tri_overlap(&a.p, &b.p) {
                    let (fa, ia, fb, ib) = if a.face < b.face {
                        (a.face, a.island, b.face, b.island)
                    } else {
                        (b.face, b.island, a.face, a.island)
                    };
                    found.insert((fa, ia, fb, ib));
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Número de **pares de ilhas distintas** cujos polígonos UV se sobrepõem (tocar na borda
/// não conta; tolerância 1e-6). Usa grade uniforme, ~O(n) para layouts bem espalhados.
/// Ilhas empilhadas de propósito (mirror) também são contadas.
pub fn overlap_count(mesh: &EditMesh) -> usize {
    let pairs: HashSet<(u32, u32)> = find_overlaps(mesh, OverlapMode::Between)
        .into_iter()
        .map(|(_, ia, _, ib)| (ia.min(ib), ia.max(ib)))
        .collect();
    pairs.len()
}

/// Faces que se sobrepõem a alguma face de **outra** ilha, em ordem crescente.
pub fn overlapping_faces(mesh: &EditMesh) -> Vec<u32> {
    let set: BTreeSet<u32> = find_overlaps(mesh, OverlapMode::Between)
        .into_iter()
        .flat_map(|(fa, _, fb, _)| [fa, fb])
        .collect();
    set.into_iter().collect()
}

/// Faces que se sobrepõem a outra face **da mesma ilha** (dobras / autointerseção do
/// unwrap), em ordem crescente.
pub fn self_overlapping_faces(mesh: &EditMesh) -> Vec<u32> {
    let set: BTreeSet<u32> = find_overlaps(mesh, OverlapMode::Within)
        .into_iter()
        .flat_map(|(fa, _, fb, _)| [fa, fb])
        .collect();
    set.into_iter().collect()
}

// ---------------------------------------------------------------------- densidade de texels

/// Densidade de texels em pixels por unidade 3D (`sqrt(área_uv / área_3d) * texture_size`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TexelDensity {
    pub min: f32,
    pub max: f32,
    /// Média ponderada pela área 3D.
    pub mean: f32,
    /// Desvio-padrão ponderado pela área 3D.
    pub std_dev: f32,
    /// Faces consideradas (área 3D não degenerada).
    pub faces: usize,
}

/// Densidade de texels da face `f` (px por unidade 3D); `None` se `f` é inválido,
/// `texture_size == 0` ou a face 3D é degenerada.
pub fn face_texel_density(mesh: &EditMesh, f: u32, texture_size: u32) -> Option<f32> {
    if texture_size == 0 || f as usize >= mesh.faces.len() {
        return None;
    }
    let a3 = mesh.face_area(f) as f64;
    if a3 < 1e-14 {
        return None;
    }
    let auv = poly_signed_area(&face_uvs(mesh, f)).abs();
    Some(((auv / a3).sqrt() * texture_size as f64) as f32)
}

/// Estatísticas de densidade de texels de toda a malha. Erros: `texture_size == 0`
/// (`InvalidParameter`) ou nenhuma face utilizável (`EmptySelection`).
pub fn texel_density(mesh: &EditMesh, texture_size: u32) -> ModelResult<TexelDensity> {
    if texture_size == 0 {
        return Err(ModelError::InvalidParameter("texture_size deve ser > 0".into()));
    }
    let mut samples: Vec<(f64, f64)> = Vec::new();
    for f in 0..mesh.faces.len() as u32 {
        if let Some(d) = face_texel_density(mesh, f, texture_size) {
            samples.push((d as f64, mesh.face_area(f) as f64));
        }
    }
    if samples.is_empty() {
        return Err(ModelError::EmptySelection("nenhuma face com área 3D válida"));
    }
    let wsum: f64 = samples.iter().map(|s| s.1).sum();
    let mean = samples.iter().map(|s| s.0 * s.1).sum::<f64>() / wsum;
    let var = samples.iter().map(|s| s.1 * (s.0 - mean).powi(2)).sum::<f64>() / wsum;
    Ok(TexelDensity {
        min: samples.iter().map(|s| s.0).fold(f64::INFINITY, f64::min) as f32,
        max: samples.iter().map(|s| s.0).fold(f64::NEG_INFINITY, f64::max) as f32,
        mean: mean as f32,
        std_dev: var.sqrt() as f32,
        faces: samples.len(),
    })
}

// ---------------------------------------------------------------------- relatório

/// Relatório agregado de qualidade de UV (serializável para o painel/CI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UvReport {
    pub face_count: usize,
    pub island_count: usize,
    /// Soma dos módulos das áreas UV.
    pub uv_area: f32,
    pub bounds: Option<(Vec2, Vec2)>,
    /// Média do `face_stretch` entre faces não colapsadas.
    pub mean_stretch: f32,
    pub max_stretch: f32,
    /// Faces com UV colapsado.
    pub degenerate_faces: usize,
    pub flipped_faces: usize,
    /// Pares de ilhas sobrepostas.
    pub overlapping_island_pairs: usize,
    /// Faces com autointerseção dentro da própria ilha.
    pub self_overlapping_faces: usize,
    /// Faces com algum UV fora de `[0,1]²` (tolerância 1e-6).
    pub outside_unit_square: usize,
    pub texel_density: Option<TexelDensity>,
}

/// Monta o [`UvReport`] de uma malha (`texture_size` em pixels, > 0).
pub fn uv_report(mesh: &EditMesh, texture_size: u32) -> ModelResult<UvReport> {
    if texture_size == 0 {
        return Err(ModelError::InvalidParameter("texture_size deve ser > 0".into()));
    }
    let stats = island_stretch_stats(mesh);
    let (mut sum, mut cnt, mut max) = (0.0f64, 0usize, 1.0f32);
    let mut degenerate = 0;
    let mut flipped = 0;
    for s in &stats {
        degenerate += s.degenerate;
        flipped += s.flipped;
        let ok = s.faces - s.degenerate;
        sum += s.mean as f64 * ok as f64;
        cnt += ok;
        max = max.max(s.max);
    }
    let outside = mesh
        .faces
        .iter()
        .filter(|f| {
            f.corners.iter().any(|c| {
                c.uv.x < -1e-6 || c.uv.y < -1e-6 || c.uv.x > 1.0 + 1e-6 || c.uv.y > 1.0 + 1e-6
            })
        })
        .count();
    Ok(UvReport {
        face_count: mesh.faces.len(),
        island_count: stats.len(),
        uv_area: uv_area(mesh),
        bounds: uv_bounds(mesh),
        mean_stretch: if cnt > 0 { (sum / cnt as f64) as f32 } else { 1.0 },
        max_stretch: max,
        degenerate_faces: degenerate,
        flipped_faces: flipped,
        overlapping_island_pairs: overlap_count(mesh),
        self_overlapping_faces: self_overlapping_faces(mesh).len(),
        outside_unit_square: outside,
        texel_density: texel_density(mesh, texture_size).ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn set_uv(m: &mut EditMesh, f: usize, uvs: &[Vec2]) {
        for (c, uv) in m.faces[f].corners.iter_mut().zip(uvs) {
            c.uv = *uv;
        }
    }

    fn square(x: f32, y: f32, s: f32) -> [Vec2; 4] {
        [
            Vec2::new(x, y),
            Vec2::new(x + s, y),
            Vec2::new(x + s, y + s),
            Vec2::new(x, y + s),
        ]
    }

    #[test]
    fn unit_quad_has_unit_area_bounds_and_unit_stretch() {
        let mut m = primitives::plane(1.0);
        // plano no XZ com normal +Y; UV anti-horário no plano UV mapeando x->u, z->-v
        let uvs: Vec<Vec2> = (0..4)
            .map(|i| {
                let p = m.co(m.faces[0].corners[i].vert);
                Vec2::new(p.x + 0.5, 0.5 - p.z)
            })
            .collect();
        set_uv(&mut m, 0, &uvs);
        assert!((uv_area(&m) - 1.0).abs() < 1e-5);
        assert!((face_stretch(&m, 0) - 1.0).abs() < 1e-4);
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.abs().max_element() < 1e-6 && (hi - Vec2::ONE).abs().max_element() < 1e-6);
        assert!(flipped_faces(&m).is_empty());
        assert!(uv_bounds(&EditMesh::new("v")).is_none());
    }

    #[test]
    fn anisotropic_scale_gives_stretch_equal_to_ratio() {
        let mut m = primitives::plane(1.0);
        let uvs: Vec<Vec2> = (0..4)
            .map(|i| {
                let p = m.co(m.faces[0].corners[i].vert);
                Vec2::new((p.x + 0.5) * 3.0, 0.5 - p.z)
            })
            .collect();
        set_uv(&mut m, 0, &uvs);
        assert!((face_stretch(&m, 0) - 3.0).abs() < 1e-3);
        // invariante a rotação e escala uniforme
        let rot = |v: Vec2| Vec2::new(-v.y, v.x) * 7.0;
        let r: Vec<Vec2> = uvs.iter().map(|&v| rot(v)).collect();
        set_uv(&mut m, 0, &r);
        assert!((face_stretch(&m, 0) - 3.0).abs() < 1e-3);
    }

    #[test]
    fn face_stretch_special_values() {
        let mut m = primitives::plane(1.0);
        assert!(face_stretch(&m, 99).is_nan());
        set_uv(&mut m, 0, &[Vec2::ZERO; 4]);
        assert!(face_stretch(&m, 0).is_infinite());
        // face 3D degenerada: três pontos colineares
        let mut d = EditMesh::new("deg");
        for x in 0..3 {
            d.add_vert(Vec3::new(x as f32, 0.0, 0.0));
        }
        d.add_face(&[0, 1, 2]).unwrap();
        assert_eq!(face_stretch(&d, 0), 1.0);
    }

    #[test]
    fn flipped_faces_detected_by_winding() {
        let mut m = primitives::plane(1.0);
        let mut uvs = square(0.0, 0.0, 1.0).to_vec();
        set_uv(&mut m, 0, &uvs);
        assert!(flipped_faces(&m).is_empty());
        uvs.reverse();
        set_uv(&mut m, 0, &uvs);
        assert_eq!(flipped_faces(&m), vec![0]);
        assert!(face_uv_area_signed(&m, 0) < 0.0);
        assert_eq!(face_uv_area_signed(&m, 9), 0.0);
        assert!((uv_area(&m) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn overlap_count_pairs_islands_and_ignores_touching() {
        // dois quads separados (2 ilhas) com UVs que se tocam, depois sobrepõem
        let mut m = primitives::plane(1.0);
        m.append(&primitives::plane(1.0));
        set_uv(&mut m, 0, &square(0.0, 0.0, 0.5));
        set_uv(&mut m, 1, &square(0.5, 0.0, 0.5)); // tocam na borda x = 0.5
        assert_eq!(split_islands(&m).len(), 2);
        assert_eq!(overlap_count(&m), 0);
        assert!(overlapping_faces(&m).is_empty());
        set_uv(&mut m, 1, &square(0.25, 0.1, 0.5));
        assert_eq!(overlap_count(&m), 1);
        assert_eq!(overlapping_faces(&m), vec![0, 1]);
        // afastar de novo
        set_uv(&mut m, 1, &square(0.6, 0.6, 0.3));
        assert_eq!(overlap_count(&m), 0);
    }

    #[test]
    fn overlap_count_scales_with_grid_on_many_islands() {
        // 40x40 quads soltos lado a lado: nenhum overlap; empilha dois e aparece 1
        let mut m = EditMesh::new("muitos");
        for _ in 0..1600 {
            m.append(&primitives::plane(1.0));
        }
        for i in 0..1600usize {
            let (x, y) = ((i % 40) as f32 / 40.0, (i / 40) as f32 / 40.0);
            set_uv(&mut m, i, &square(x, y, 1.0 / 40.0));
        }
        let t = std::time::Instant::now();
        assert_eq!(overlap_count(&m), 0);
        set_uv(&mut m, 5, &square(0.0, 0.0, 1.0 / 80.0));
        assert_eq!(overlap_count(&m), 1);
        assert!(t.elapsed().as_secs() < 30);
    }

    #[test]
    fn self_overlap_detects_folds() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        // dois quads adjacentes: o segundo dobrado por cima do primeiro
        let uvs0 = [Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)];
        set_uv(&mut m, 0, &uvs0);
        let uvs1 = [Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0)];
        set_uv(&mut m, 1, &uvs1);
        assert_eq!(self_overlapping_faces(&m), vec![0, 1]);
        assert_eq!(overlap_count(&m), 0);
    }

    #[test]
    fn texel_density_uniform_and_errors() {
        let mut m = primitives::plane(2.0);
        let uvs: Vec<Vec2> = (0..4)
            .map(|i| {
                let p = m.co(m.faces[0].corners[i].vert);
                Vec2::new(p.x / 2.0 + 0.5, 0.5 - p.z / 2.0)
            })
            .collect();
        set_uv(&mut m, 0, &uvs);
        let d = texel_density(&m, 1024).unwrap();
        // 1 unidade UV = 2 unidades 3D => 512 px/unidade
        assert!((d.mean - 512.0).abs() < 1e-2 && (d.min - d.max).abs() < 1e-2 && d.std_dev < 1e-2);
        assert_eq!(d.faces, 1);
        assert!(texel_density(&m, 0).is_err());
        assert!(matches!(
            texel_density(&EditMesh::new("v"), 512),
            Err(ModelError::EmptySelection(_))
        ));
        assert_eq!(face_texel_density(&m, 5, 512), None);
    }

    #[test]
    fn island_stats_and_report_roundtrip_through_serde() {
        let m = primitives::cube(1.0);
        let stats = island_stretch_stats(&m);
        assert_eq!(stats.len(), 6);
        assert!(stats.iter().all(|s| (s.mean - 1.0).abs() < 1e-3 && s.flipped == 0));
        let rep = uv_report(&m, 1024).unwrap();
        assert_eq!(rep.island_count, 6);
        assert_eq!(rep.flipped_faces, 0);
        assert_eq!(rep.outside_unit_square, 0);
        assert!(rep.overlapping_island_pairs > 0, "as 6 faces do cubo primitivo se empilham em 0..1");
        let json = serde_json::to_string(&rep).unwrap();
        let back: UvReport = serde_json::from_str(&json).unwrap();
        assert_eq!(rep, back);
        assert!(uv_report(&m, 0).is_err());
    }

    #[test]
    fn uv_hash_is_sensitive_and_stable() {
        let a = primitives::cube(1.0);
        let mut b = a.clone();
        assert_eq!(uv_hash(&a), uv_hash(&b));
        b.faces[0].corners[0].uv.x += 1e-3;
        assert_ne!(uv_hash(&a), uv_hash(&b));
    }
}
