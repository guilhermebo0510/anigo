//! Empacotamento de ilhas de UV no quadrado unitário (equivalente a *Pack Islands* do
//! Blender) e normalização de densidade de texels entre ilhas.
//!
//! Algoritmo: **MaxRects** (heurística *best short side fit*, rotação de 90° opcional)
//! para até [`MAXRECTS_LIMIT`] ilhas e **shelf** (prateleiras) acima disso; a escala
//! global é procurada por bissecção (o maior fator uniforme que ainda cabe). Tudo é
//! determinístico: ordenações totais com desempate por índice, nenhum `HashMap` iterado.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::uv::metrics::poly_signed_area;
use crate::uv::seams::{faces_uv_bounds, resolve_faces, split_islands_in, Island};
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Acima desse número de ilhas usa-se o empacotador de prateleiras (mais rápido, menos denso).
pub const MAXRECTS_LIMIT: usize = 400;
const EPS: f64 = 1e-9;

/// Parâmetros de [`pack_islands`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackParams {
    /// Margem mínima **entre ilhas** em pixels de `texture_size` (cada ilha ganha metade
    /// da margem de cada lado, então a borda do quadrado unitário também fica protegida).
    pub margin: f32,
    /// Resolução da textura (px) usada para converter a margem em espaço UV.
    pub texture_size: u32,
    /// Permite girar ilhas em 90° (preserva o enrolamento; melhora a ocupação).
    pub rotate: bool,
    /// Permite escalar o layout uniformemente para preencher o quadrado. Se `false` o
    /// tamanho atual é mantido e é erro não caber.
    pub allow_scale: bool,
    /// `true`: escala global uniforme (densidade de texels preservada). `false`: ao final,
    /// o layout é esticado independentemente em u e v para preencher exatamente o quadrado.
    pub keep_aspect: bool,
    /// `true`: resultado normalizado em `[0,1]²`. `false`: mantém as unidades atuais
    /// (escala 1), numa caixa quadrada mínima encontrada automaticamente.
    pub to_unit_square: bool,
}

impl Default for PackParams {
    fn default() -> Self {
        Self {
            margin: 4.0,
            texture_size: 1024,
            rotate: true,
            allow_scale: true,
            keep_aspect: true,
            to_unit_square: true,
        }
    }
}

impl PackParams {
    /// Margem convertida para espaço UV (`margin / texture_size`).
    pub fn margin_uv(&self) -> f32 {
        self.margin / self.texture_size.max(1) as f32
    }

    fn validate(&self) -> ModelResult<()> {
        if self.texture_size == 0 {
            return Err(ModelError::InvalidParameter("texture_size deve ser > 0".into()));
        }
        if !self.margin.is_finite() || self.margin < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "margem deve ser finita e >= 0, recebeu {}",
                self.margin
            )));
        }
        if self.margin_uv() >= 0.5 {
            return Err(ModelError::InvalidParameter(
                "margem maior ou igual à metade da textura".into(),
            ));
        }
        Ok(())
    }
}

/// Resultado de [`pack_islands`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackReport {
    pub islands: usize,
    /// Fator uniforme aplicado às ilhas (1 = tamanho original).
    pub scale: f32,
    /// Σ área das caixas das ilhas (sem margem) / área da caixa de empacotamento.
    pub used_area_ratio: f32,
    /// Σ área real dos polígonos UV / área da caixa (≤ `used_area_ratio`).
    pub uv_area_ratio: f32,
    /// Ilhas giradas em 90°.
    pub rotated: usize,
    /// Extensão `(largura, altura)` realmente ocupada (antes de um eventual esticamento).
    pub extent: Vec2,
}

#[derive(Clone, Copy)]
struct Placement {
    x: f64,
    y: f64,
    rot: bool,
}

#[derive(Clone, Copy)]
struct R {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn contains(a: &R, b: &R) -> bool {
    // a contém b
    b.x >= a.x - EPS && b.y >= a.y - EPS && b.x + b.w <= a.x + a.w + EPS && b.y + b.h <= a.y + a.h + EPS
}

fn total(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

fn order_by_size(dims: &[(f64, f64)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..dims.len()).collect();
    order.sort_by(|&a, &b| {
        let (da, db) = (dims[a], dims[b]);
        total(db.0.max(db.1), da.0.max(da.1))
            .then(total(db.0 * db.1, da.0 * da.1))
            .then(a.cmp(&b))
    });
    order
}

/// MaxRects / best-short-side-fit. `dims` já inclui a margem. `None` se algum item não couber.
fn maxrects(dims: &[(f64, f64)], bw: f64, bh: f64, rotate: bool) -> Option<Vec<Placement>> {
    let mut free = vec![R { x: 0.0, y: 0.0, w: bw, h: bh }];
    let mut out = vec![Placement { x: 0.0, y: 0.0, rot: false }; dims.len()];
    for i in order_by_size(dims) {
        let (w, h) = dims[i];
        let mut best: Option<(usize, bool, [f64; 4])> = None;
        for (k, fr) in free.iter().enumerate() {
            for rot in [false, true] {
                if rot && (!rotate || (w - h).abs() < EPS) {
                    continue;
                }
                let (rw, rh) = if rot { (h, w) } else { (w, h) };
                if rw > fr.w + EPS || rh > fr.h + EPS {
                    continue;
                }
                let (lw, lh) = (fr.w - rw, fr.h - rh);
                let key = [lw.min(lh), lw.max(lh), fr.y, fr.x];
                let better = match &best {
                    None => true,
                    Some((_, _, bk)) => key
                        .iter()
                        .zip(bk.iter())
                        .map(|(a, b)| total(*a, *b))
                        .find(|o| *o != Ordering::Equal)
                        == Some(Ordering::Less),
                };
                if better {
                    best = Some((k, rot, key));
                }
            }
        }
        let (k, rot, _) = best?;
        let fr = free[k];
        let (rw, rh) = if rot { (h, w) } else { (w, h) };
        out[i] = Placement { x: fr.x, y: fr.y, rot };
        let p = R { x: fr.x, y: fr.y, w: rw, h: rh };
        let mut kept = Vec::with_capacity(free.len() + 4);
        let mut added: Vec<R> = Vec::new();
        for f in free.drain(..) {
            let hit = p.x < f.x + f.w - EPS && p.x + p.w > f.x + EPS && p.y < f.y + f.h - EPS && p.y + p.h > f.y + EPS;
            if !hit {
                kept.push(f);
                continue;
            }
            if p.x > f.x + EPS {
                added.push(R { x: f.x, y: f.y, w: p.x - f.x, h: f.h });
            }
            if p.x + p.w < f.x + f.w - EPS {
                added.push(R { x: p.x + p.w, y: f.y, w: f.x + f.w - (p.x + p.w), h: f.h });
            }
            if p.y > f.y + EPS {
                added.push(R { x: f.x, y: f.y, w: f.w, h: p.y - f.y });
            }
            if p.y + p.h < f.y + f.h - EPS {
                added.push(R { x: f.x, y: p.y + p.h, w: f.w, h: f.y + f.h - (p.y + p.h) });
            }
        }
        for (a, r) in added.iter().enumerate() {
            let dominated = kept.iter().any(|o| contains(o, r))
                || added.iter().enumerate().any(|(b, o)| {
                    b != a && contains(o, r) && (!contains(r, o) || b < a)
                });
            if !dominated && r.w > EPS && r.h > EPS {
                kept.push(*r);
            }
        }
        free = kept;
    }
    Some(out)
}

/// Prateleiras: itens deitados (lado menor como altura quando `rotate`), por altura decrescente.
fn shelf(dims: &[(f64, f64)], bw: f64, bh: f64, rotate: bool) -> Option<Vec<Placement>> {
    let oriented: Vec<(f64, f64, bool)> = dims
        .iter()
        .map(|&(w, h)| if rotate && h > w { (h, w, true) } else { (w, h, false) })
        .collect();
    let mut order: Vec<usize> = (0..dims.len()).collect();
    order.sort_by(|&a, &b| total(oriented[b].1, oriented[a].1).then(total(oriented[b].0, oriented[a].0)).then(a.cmp(&b)));
    let mut out = vec![Placement { x: 0.0, y: 0.0, rot: false }; dims.len()];
    let (mut x, mut y, mut row) = (0.0f64, 0.0f64, 0.0f64);
    for i in order {
        let (w, h, rot) = oriented[i];
        if x + w > bw + EPS {
            y += row;
            x = 0.0;
            row = 0.0;
        }
        if x + w > bw + EPS || y + h > bh + EPS {
            return None;
        }
        out[i] = Placement { x, y, rot };
        x += w;
        row = row.max(h);
    }
    Some(out)
}

fn try_pack(
    sizes: &[(f64, f64)],
    scale: f64,
    margin: f64,
    bw: f64,
    bh: f64,
    rotate: bool,
) -> Option<Vec<Placement>> {
    let dims: Vec<(f64, f64)> = sizes.iter().map(|&(w, h)| (w * scale + margin, h * scale + margin)).collect();
    if dims.len() <= MAXRECTS_LIMIT {
        maxrects(&dims, bw, bh, rotate)
    } else {
        shelf(&dims, bw, bh, rotate)
    }
}

/// Procura o maior fator uniforme que cabe em 1×1. `None` se nem escala minúscula cabe.
fn fit_scale(sizes: &[(f64, f64)], margin: f64, rotate: bool) -> Option<(f64, Vec<Placement>)> {
    let max_dim = sizes.iter().map(|s| s.0.max(s.1)).fold(0.0f64, f64::max);
    let area: f64 = sizes.iter().map(|s| s.0 * s.1).sum();
    if max_dim <= EPS {
        return try_pack(sizes, 1.0, margin, 1.0, 1.0, rotate).map(|p| (1.0, p));
    }
    let mut hi = ((1.0 - margin) / max_dim).max(0.0);
    if area > EPS {
        hi = hi.min((1.0 / area).sqrt());
    }
    if hi <= 0.0 {
        return None;
    }
    if let Some(p) = try_pack(sizes, hi, margin, 1.0, 1.0, rotate) {
        return Some((hi, p));
    }
    let mut ok_scale = hi * 0.5;
    let mut ok_pack = None;
    for _ in 0..80 {
        if let Some(p) = try_pack(sizes, ok_scale, margin, 1.0, 1.0, rotate) {
            ok_pack = Some(p);
            break;
        }
        ok_scale *= 0.5;
    }
    let mut ok_pack = ok_pack?;
    let mut fail = ok_scale * 2.0;
    for _ in 0..28 {
        let mid = 0.5 * (ok_scale + fail);
        if let Some(p) = try_pack(sizes, mid, margin, 1.0, 1.0, rotate) {
            ok_scale = mid;
            ok_pack = p;
        } else {
            fail = mid;
        }
        if (fail - ok_scale) < 1e-7 * ok_scale {
            break;
        }
    }
    Some((ok_scale, ok_pack))
}

/// Empacota **todas** as ilhas de [`split_islands`](crate::uv::seams::split_islands). Ver
/// [`pack_island_list`].
pub fn pack_islands(mesh: &mut EditMesh, params: &PackParams) -> ModelResult<PackReport> {
    pack_islands_in(mesh, None, params)
}

/// Empacota as ilhas formadas pelas `faces` dadas (`None` = todas). As demais faces não
/// são tocadas (e podem sobrepor o resultado, como no Blender).
pub fn pack_islands_in(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    params: &PackParams,
) -> ModelResult<PackReport> {
    let faces = resolve_faces(mesh, faces)?;
    let islands = split_islands_in(mesh, &faces)?;
    pack_island_list(mesh, &islands, params)
}

/// Empacota uma lista explícita de ilhas (faces disjuntas entre si). Os UVs dos corners
/// são reescritos por transformação rígida (translação, rotação de 90° opcional) mais
/// **escala uniforme global**; o enrolamento nunca é invertido.
///
/// Erros (malha intacta): parâmetros inválidos, lista vazia, índices de face inválidos,
/// UVs não finitos, ilhas que não cabem (`allow_scale = false`) ou margem grande demais.
/// Determinístico: mesmas entradas → mesmos bits.
pub fn pack_island_list(
    mesh: &mut EditMesh,
    islands: &[Island],
    params: &PackParams,
) -> ModelResult<PackReport> {
    params.validate()?;
    if islands.is_empty() {
        return Err(ModelError::EmptySelection("nenhuma ilha para empacotar"));
    }
    for isl in islands {
        for &f in &isl.faces {
            let face = mesh
                .faces
                .get(f as usize)
                .ok_or(ModelError::FaceOutOfRange(f, mesh.faces.len()))?;
            if face.corners.iter().any(|c| !c.uv.is_finite()) {
                return Err(ModelError::Numeric(format!("UV não finito na face {f}")));
            }
        }
    }
    let bbs: Vec<(Vec2, Vec2)> = islands.iter().map(|i| faces_uv_bounds(mesh, &i.faces)).collect();
    let sizes: Vec<(f64, f64)> = bbs
        .iter()
        .map(|(lo, hi)| ((hi.x - lo.x) as f64, (hi.y - lo.y) as f64))
        .collect();
    let margin = params.margin_uv() as f64;
    let uv_area: f64 = islands
        .iter()
        .flat_map(|i| i.faces.iter())
        .map(|&f| {
            let uv: Vec<Vec2> = mesh.faces[f as usize].corners.iter().map(|c| c.uv).collect();
            poly_signed_area(&uv).abs()
        })
        .sum();

    let (scale, places, bin) = if params.to_unit_square {
        if params.allow_scale {
            let (s, p) = fit_scale(&sizes, margin, params.rotate).ok_or_else(|| {
                ModelError::InvalidParameter(
                    "não foi possível empacotar: margem grande demais para o número de ilhas".into(),
                )
            })?;
            (s, p, 1.0)
        } else {
            let p = try_pack(&sizes, 1.0, margin, 1.0, 1.0, params.rotate).ok_or_else(|| {
                ModelError::InvalidParameter(
                    "as ilhas não cabem no quadrado unitário sem escala (allow_scale = false)".into(),
                )
            })?;
            (1.0, p, 1.0)
        }
    } else {
        let area: f64 = sizes.iter().map(|s| (s.0 + margin) * (s.1 + margin)).sum();
        let max_side = sizes.iter().map(|s| s.0.max(s.1) + margin).fold(0.0f64, f64::max);
        let mut side = area.sqrt().max(max_side).max(EPS);
        let mut found = None;
        for _ in 0..200 {
            if let Some(p) = try_pack(&sizes, 1.0, margin, side, side, params.rotate) {
                found = Some(p);
                break;
            }
            side *= 1.05;
        }
        let p = found.ok_or_else(|| ModelError::Numeric("empacotamento sem solução".into()))?;
        (1.0, p, side)
    };

    // Extensão ocupada.
    let (mut ext_x, mut ext_y) = (0.0f64, 0.0f64);
    let mut box_area = 0.0f64;
    let mut rotated = 0;
    for (i, pl) in places.iter().enumerate() {
        let (w, h) = (sizes[i].0 * scale, sizes[i].1 * scale);
        let (rw, rh) = if pl.rot { (h, w) } else { (w, h) };
        ext_x = ext_x.max(pl.x + rw + margin);
        ext_y = ext_y.max(pl.y + rh + margin);
        box_area += w * h;
        rotated += pl.rot as usize;
    }
    let (sx, sy) = if params.to_unit_square && !params.keep_aspect {
        (1.0 / ext_x.max(EPS), 1.0 / ext_y.max(EPS))
    } else {
        (1.0, 1.0)
    };

    for (i, isl) in islands.iter().enumerate() {
        let (lo, _) = bbs[i];
        let pl = places[i];
        let h = sizes[i].1;
        for &f in &isl.faces {
            for c in &mut mesh.faces[f as usize].corners {
                let rx = (c.uv.x - lo.x) as f64;
                let ry = (c.uv.y - lo.y) as f64;
                let (rx, ry) = if pl.rot { (h - ry, rx) } else { (rx, ry) };
                let u = (rx * scale + pl.x + margin * 0.5) * sx;
                let v = (ry * scale + pl.y + margin * 0.5) * sy;
                c.uv = Vec2::new(u as f32, v as f32);
            }
        }
    }
    Ok(PackReport {
        islands: islands.len(),
        scale: scale as f32,
        used_area_ratio: (box_area / (bin * bin)) as f32,
        uv_area_ratio: (uv_area * scale * scale / (bin * bin)) as f32,
        rotated,
        extent: Vec2::new(ext_x as f32, ext_y as f32),
    })
}

// ------------------------------------------------------------------ densidade entre ilhas

/// `(área 3D, área UV)` por ilha de [`split_islands`](crate::uv::seams::split_islands).
fn island_areas(mesh: &EditMesh, islands: &[Island]) -> Vec<(f64, f64)> {
    islands
        .iter()
        .map(|isl| {
            let a3: f64 = isl.faces.iter().map(|&f| mesh.face_area(f) as f64).sum();
            let auv: f64 = isl
                .faces
                .iter()
                .map(|&f| {
                    let uv: Vec<Vec2> = mesh.faces[f as usize].corners.iter().map(|c| c.uv).collect();
                    poly_signed_area(&uv).abs()
                })
                .sum();
            (a3, auv)
        })
        .collect()
}

/// Escala cada ilha (em torno do centro da sua caixa UV) para a razão `sqrt(uv/3d)`
/// alvo `k` (ou a média global se `None`). Devolve `k` usado.
fn scale_islands_to_density(mesh: &mut EditMesh, k_target: Option<f64>) -> ModelResult<f64> {
    let all: Vec<u32> = (0..mesh.faces.len() as u32).collect();
    let faces = resolve_faces(mesh, Some(&all))?;
    let islands = split_islands_in(mesh, &faces)?;
    for isl in &islands {
        for &f in &isl.faces {
            if mesh.faces[f as usize].corners.iter().any(|c| !c.uv.is_finite()) {
                return Err(ModelError::Numeric(format!("UV não finito na face {f}")));
            }
        }
    }
    let areas = island_areas(mesh, &islands);
    let valid = |a: &(f64, f64)| a.0 > 1e-14 && a.1 > 1e-14;
    let (sum3, sumuv) = areas.iter().filter(|a| valid(a)).fold((0.0, 0.0), |s, a| (s.0 + a.0, s.1 + a.1));
    if sum3 <= 0.0 {
        return Err(ModelError::EmptySelection("nenhuma ilha com área 3D e UV válidas"));
    }
    let k = k_target.unwrap_or((sumuv / sum3).sqrt());
    for (isl, a) in islands.iter().zip(&areas) {
        if !valid(a) {
            continue;
        }
        let ki = (a.1 / a.0).sqrt();
        let s = (k / ki) as f32;
        let (lo, hi) = faces_uv_bounds(mesh, &isl.faces);
        let c = (lo + hi) * 0.5;
        for &f in &isl.faces {
            for corner in &mut mesh.faces[f as usize].corners {
                corner.uv = c + (corner.uv - c) * s;
            }
        }
    }
    Ok(k)
}

/// *Average Islands Scale*: reescala cada ilha em torno do próprio centro para que todas
/// tenham a mesma razão `UV/3D` — a razão global `sqrt(ΣA_uv / ΣA_3d)`, de modo que a área
/// UV total é preservada. Devolve essa razão (unidades UV por unidade 3D). Ilhas com área 3D
/// ou UV nula são ignoradas. Pode causar sobreposição: rode [`pack_islands`] depois.
pub fn average_islands_scale(mesh: &mut EditMesh) -> ModelResult<f32> {
    scale_islands_to_density(mesh, None).map(|k| k as f32)
}

/// Normaliza a densidade de texels entre ilhas para `target_px_per_unit` pixels por unidade 3D
/// em uma textura de `texture_size` (ou para a média atual, se `None`). Devolve a densidade
/// final em px/unidade. Os UVs podem sair de `[0,1]²`: rode [`pack_islands`] em seguida.
pub fn normalize_texel_density(
    mesh: &mut EditMesh,
    texture_size: u32,
    target_px_per_unit: Option<f32>,
) -> ModelResult<f32> {
    if texture_size == 0 {
        return Err(ModelError::InvalidParameter("texture_size deve ser > 0".into()));
    }
    if let Some(t) = target_px_per_unit {
        if !t.is_finite() || t <= 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "densidade alvo deve ser > 0, recebeu {t}"
            )));
        }
    }
    let k = scale_islands_to_density(mesh, target_px_per_unit.map(|t| t as f64 / texture_size as f64))?;
    Ok((k * texture_size as f64) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::uv::metrics::{flipped_faces, overlap_count, texel_density, uv_bounds, uv_hash};
    use crate::uv::seams::split_islands;

    /// Malha com um quad solto por retângulo; o UV de cada quad é o retângulo dado.
    fn rect_mesh(rects: &[(f32, f32)]) -> EditMesh {
        let mut m = EditMesh::new("rects");
        for &(w, h) in rects {
            let before = m.faces.len();
            m.append(&primitives::plane(1.0));
            let f = &mut m.faces[before];
            let uvs = [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)];
            for (c, uv) in f.corners.iter_mut().zip(uvs) {
                c.uv = uv;
            }
        }
        m
    }

    fn boxes(m: &EditMesh) -> Vec<(Vec2, Vec2)> {
        split_islands(m).iter().map(|i| i.bounds_uv).collect()
    }

    fn assert_packed(m: &EditMesh, margin_uv: f32) {
        let bs = boxes(m);
        for (i, (lo, hi)) in bs.iter().enumerate() {
            assert!(lo.x >= -1e-6 && lo.y >= -1e-6 && hi.x <= 1.0 + 1e-6 && hi.y <= 1.0 + 1e-6, "ilha {i} fora: {lo:?} {hi:?}");
            for (j, (lo2, hi2)) in bs.iter().enumerate().skip(i + 1) {
                // caixas inflacionadas por margem/2 não podem se sobrepor => afastadas >= margem
                let gap_x = (lo2.x - hi.x).max(lo.x - hi2.x);
                let gap_y = (lo2.y - hi.y).max(lo.y - hi2.y);
                assert!(
                    gap_x.max(gap_y) >= margin_uv - 1e-5,
                    "ilhas {i} e {j} sem margem: gap {} < {margin_uv}",
                    gap_x.max(gap_y)
                );
            }
        }
    }

    fn benign() -> Vec<(f32, f32)> {
        (0..12)
            .map(|i| {
                let a = 1.0 + (i % 4) as f32 * 0.5;
                let b = 1.0 + (i % 3) as f32 * 0.7;
                (a, b)
            })
            .collect()
    }

    #[test]
    fn packs_benign_rectangles_without_overlap_inside_unit_square() {
        let mut m = rect_mesh(&benign());
        let params = PackParams::default();
        let rep = pack_islands(&mut m, &params).unwrap();
        assert_eq!(rep.islands, 12);
        assert_packed(&m, params.margin_uv());
        assert_eq!(overlap_count(&m), 0);
        assert!(rep.used_area_ratio > 0.6, "ocupação {}", rep.used_area_ratio);
        assert!((rep.uv_area_ratio - rep.used_area_ratio).abs() < 1e-3, "retângulos: áreas coincidem");
        assert!(flipped_faces(&m).is_empty());
        m.validate().unwrap();
    }

    #[test]
    fn packing_is_deterministic_bit_by_bit() {
        let run = || {
            let mut m = rect_mesh(&benign());
            pack_islands(&mut m, &PackParams::default()).unwrap();
            uv_hash(&m)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn rotation_improves_tall_thin_islands() {
        let rects: Vec<(f32, f32)> = (0..10).map(|_| (0.2, 3.0)).collect();
        let mut a = rect_mesh(&rects);
        let mut b = rect_mesh(&rects);
        let ra = pack_islands(&mut a, &PackParams { rotate: false, ..Default::default() }).unwrap();
        let rb = pack_islands(&mut b, &PackParams { rotate: true, ..Default::default() }).unwrap();
        assert!(rb.scale >= ra.scale * 0.999, "{} vs {}", rb.scale, ra.scale);
        assert_eq!(ra.rotated, 0);
        assert_packed(&a, PackParams::default().margin_uv());
        assert_packed(&b, PackParams::default().margin_uv());
        assert!(flipped_faces(&b).is_empty(), "rotação de 90° não inverte enrolamento");
        assert_eq!(overlap_count(&b), 0);
    }

    #[test]
    fn rotated_island_keeps_edge_lengths() {
        let mut m = rect_mesh(&[(4.0, 1.0), (4.0, 1.0), (1.0, 6.0), (1.0, 6.0)]);
        let rep = pack_islands(&mut m, &PackParams { margin: 0.0, ..Default::default() }).unwrap();
        for isl in split_islands(&m) {
            let (lo, hi) = isl.bounds_uv;
            let (w, h) = ((hi.x - lo.x) / rep.scale, (hi.y - lo.y) / rep.scale);
            let (a, b) = (w.min(h), w.max(h));
            assert!((a - 1.0).abs() < 1e-3 || (a - 4.0).abs() < 1e-3, "{a} {b}");
        }
    }

    #[test]
    fn zero_margin_still_has_no_overlap() {
        let mut m = rect_mesh(&benign());
        let params = PackParams { margin: 0.0, ..Default::default() };
        let rep = pack_islands(&mut m, &params).unwrap();
        assert_packed(&m, 0.0);
        assert_eq!(overlap_count(&m), 0);
        assert!(rep.used_area_ratio > 0.6);
    }

    #[test]
    fn allow_scale_false_keeps_size_or_errors_leaving_mesh_intact() {
        let mut small = rect_mesh(&[(0.2, 0.2), (0.3, 0.1), (0.1, 0.4)]);
        let rep = pack_islands(&mut small, &PackParams { allow_scale: false, margin: 2.0, ..Default::default() }).unwrap();
        assert_eq!(rep.scale, 1.0);
        let isl = split_islands(&small);
        let (lo, hi) = isl[0].bounds_uv;
        assert!(((hi.x - lo.x) - 0.2).abs() < 1e-5 || ((hi.y - lo.y) - 0.2).abs() < 1e-5);
        assert_packed(&small, 2.0 / 1024.0);

        let mut big = rect_mesh(&[(3.0, 3.0), (3.0, 3.0)]);
        let before = big.clone();
        let err = pack_islands(&mut big, &PackParams { allow_scale: false, ..Default::default() }).unwrap_err();
        assert!(matches!(err, ModelError::InvalidParameter(_)));
        assert_eq!(big, before);
    }

    #[test]
    fn to_unit_square_false_keeps_world_units() {
        let mut m = rect_mesh(&[(3.0, 3.0), (3.0, 1.0), (2.0, 2.0)]);
        let rep = pack_islands(&mut m, &PackParams { to_unit_square: false, margin: 8.0, texture_size: 64, ..Default::default() }).unwrap();
        assert_eq!(rep.scale, 1.0);
        assert!(rep.extent.x > 1.0);
        assert_eq!(overlap_count(&m), 0);
        let (_, hi) = uv_bounds(&m).unwrap();
        assert!(hi.x > 1.0 || hi.y > 1.0);
    }

    #[test]
    fn keep_aspect_false_fills_square() {
        let mut m = rect_mesh(&[(4.0, 1.0)]);
        pack_islands(&mut m, &PackParams { keep_aspect: false, margin: 0.0, ..Default::default() }).unwrap();
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.length() < 1e-5 && (hi - Vec2::ONE).length() < 1e-4, "{lo:?} {hi:?}");
        let mut k = rect_mesh(&[(4.0, 1.0)]);
        pack_islands(&mut k, &PackParams { rotate: false, margin: 0.0, ..Default::default() }).unwrap();
        let (_, hi) = uv_bounds(&k).unwrap();
        assert!((hi.x - 1.0).abs() < 1e-5 && (hi.y - 0.25).abs() < 1e-5, "{hi:?}");
    }

    #[test]
    fn invalid_parameters_and_inputs_are_rejected_without_mutation() {
        let mut m = rect_mesh(&benign());
        let before = m.clone();
        for p in [
            PackParams { margin: -1.0, ..Default::default() },
            PackParams { margin: f32::NAN, ..Default::default() },
            PackParams { texture_size: 0, ..Default::default() },
            PackParams { margin: 600.0, texture_size: 1024, ..Default::default() },
        ] {
            assert!(matches!(pack_islands(&mut m, &p), Err(ModelError::InvalidParameter(_))));
        }
        assert_eq!(m, before);
        assert!(matches!(
            pack_islands(&mut EditMesh::new("v"), &PackParams::default()),
            Err(ModelError::EmptySelection(_))
        ));
        let mut nan = m.clone();
        nan.faces[0].corners[0].uv.x = f32::NAN;
        let b2 = nan.clone();
        assert!(matches!(pack_islands(&mut nan, &PackParams::default()), Err(ModelError::Numeric(_))));
        assert_eq!(nan.faces[3], b2.faces[3]);
        assert!(nan.faces[0].corners[0].uv.x.is_nan());
        assert!(matches!(
            pack_islands_in(&mut m, Some(&[999]), &PackParams::default()),
            Err(ModelError::FaceOutOfRange(999, _))
        ));
        // margem tão grande que nada cabe
        let mut many = rect_mesh(&vec![(1.0, 1.0); 40]);
        let r = pack_islands(&mut many, &PackParams { margin: 200.0, texture_size: 1024, ..Default::default() });
        assert!(r.is_err());
    }

    #[test]
    fn subset_packing_leaves_other_faces_untouched() {
        let mut m = rect_mesh(&benign());
        let untouched = m.faces[11].clone();
        pack_islands_in(&mut m, Some(&[0, 1, 2, 3]), &PackParams::default()).unwrap();
        assert_eq!(m.faces[11], untouched);
        assert!(m.faces[0].corners.iter().all(|c| c.uv.x <= 1.0 + 1e-6));
    }

    #[test]
    fn many_islands_use_shelf_fallback_without_overlap() {
        let rects: Vec<(f32, f32)> = (0..600)
            .map(|i| (1.0 + (i % 5) as f32 * 0.3, 1.0 + (i % 7) as f32 * 0.2))
            .collect();
        let mut m = rect_mesh(&rects);
        assert!(m.faces.len() > MAXRECTS_LIMIT);
        let t = std::time::Instant::now();
        let params = PackParams { margin: 1.0, ..Default::default() };
        let rep = pack_islands(&mut m, &params).unwrap();
        assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
        assert_packed(&m, params.margin_uv());
        assert_eq!(overlap_count(&m), 0);
        assert!(rep.used_area_ratio > 0.45, "{}", rep.used_area_ratio);
    }

    #[test]
    fn many_islands_maxrects_property_loop_varied_shapes() {
        // property-style: formas variadas, vários tamanhos de lote
        for n in [1usize, 2, 3, 7, 25, 90] {
            for seed in 0..3u32 {
                let rects: Vec<(f32, f32)> = (0..n)
                    .map(|i| {
                        let k = (i as u32).wrapping_mul(2654435761u32).wrapping_add(seed * 97) % 1000;
                        (0.05 + (k % 37) as f32 * 0.03, 0.05 + ((k / 37) % 29) as f32 * 0.04)
                    })
                    .collect();
                let mut m = rect_mesh(&rects);
                let params = PackParams { margin: 3.0, ..Default::default() };
                let rep = pack_islands(&mut m, &params).unwrap();
                assert_packed(&m, params.margin_uv());
                assert_eq!(overlap_count(&m), 0, "n={n} seed={seed}");
                assert!(rep.scale > 0.0);
            }
        }
    }

    #[test]
    fn average_islands_scale_equalizes_density_and_preserves_total_area() {
        // dois planos 3D de tamanhos diferentes com a mesma escala UV: densidades distintas
        let mut m = rect_mesh(&[(1.0, 1.0), (1.0, 1.0)]);
        // segundo plano 4x maior em 3D
        for v in &mut m.verts[4..] {
            v.co *= 4.0;
        }
        let d0 = texel_density(&m, 1024).unwrap();
        assert!(d0.max / d0.min > 3.0);
        let total_before = crate::uv::metrics::uv_area(&m);
        let k = average_islands_scale(&mut m).unwrap();
        let d1 = texel_density(&m, 1024).unwrap();
        assert!((d1.max - d1.min) / d1.mean < 1e-4, "{d1:?}");
        assert!((crate::uv::metrics::uv_area(&m) - total_before).abs() < 1e-4);
        assert!((d1.mean - k * 1024.0).abs() < 0.5);
        assert!(flipped_faces(&m).is_empty());
    }

    #[test]
    fn normalize_texel_density_hits_target_and_validates() {
        let mut m = rect_mesh(&[(1.0, 1.0), (2.0, 2.0)]);
        let d = normalize_texel_density(&mut m, 512, Some(100.0)).unwrap();
        assert!((d - 100.0).abs() < 1e-3);
        let st = texel_density(&m, 512).unwrap();
        assert!((st.min - 100.0).abs() < 0.05 && (st.max - 100.0).abs() < 0.05);
        let before = m.clone();
        assert!(normalize_texel_density(&mut m, 0, None).is_err());
        assert!(normalize_texel_density(&mut m, 512, Some(-1.0)).is_err());
        assert!(normalize_texel_density(&mut m, 512, Some(f32::NAN)).is_err());
        assert_eq!(m, before);
        // malha sem UV (área zero): nada utilizável
        let mut z = primitives::plane(1.0);
        for c in &mut z.faces[0].corners {
            c.uv = Vec2::ZERO;
        }
        assert!(matches!(
            normalize_texel_density(&mut z, 512, None),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(average_islands_scale(&mut EditMesh::new("v")).is_err());
    }

    #[test]
    fn pack_cube_default_uvs_gives_six_disjoint_islands() {
        let mut m = primitives::cube(1.0);
        let rep = pack_islands(&mut m, &PackParams::default()).unwrap();
        assert_eq!(rep.islands, 6);
        assert_eq!(overlap_count(&m), 0);
        assert_packed(&m, PackParams::default().margin_uv());
    }
}
