//! Geração de malha do cabelo: mechas (guia + seção) → [`EditMesh`] pronta para cel shading.
//!
//! * **Flat / Arc** → [`crate::curve::hair_card`] (card aberto de dois lados; `Arc` com `curl`).
//! * **Triangle / Tube** → varredura sólida própria (elipse `thickness × width` com 3 ou 6 lados) sobre
//!   os mesmos frames de Bishop de `curve.rs`; ponta em bico quando `tip_taper = 0`.
//! * **Torção**: soma `twist · s/L` ao `tilt` de cada ponto (os frames já aplicam o tilt).
//! * **Clump**: a guia vira `count` fios finos ([`crate::curve::hair_clump`]).
//! * **Normais** (por corner): normal da seção viesada para FORA da cabeça (`p − scalp_center`) por
//!   `material.normal_bias` — a forma do sombreamento é a de uma esfera suave, não a do card.
//! * **UV**: `V` = fração do comprimento (0 raiz → 1 ponta), `U` através; com [`UvLayout::LengthU`] os
//!   eixos trocam (a tangente derivada da UV passa a ser a direção do fio). `atlas` remapeia para uma
//!   célula de atlas (`atlas_cycle` varia a célula por mecha).
//! * **Cor de vértice** (canais NPR): `R` (AO) escurece na raiz (`material.root_ao`) e sobe até 1.
//!
//! [`HairMesh`] devolve, além da malha, dados por vértice: mecha de origem, `t` (fração do comprimento)
//! e direção do fio — insumos para pesos de skin (2 ossos vizinhos por `t`) e para o brilho anisotrópico.

use super::{HairGroup, HairSection, HairStrand, SectionKind, UvLayout};
use crate::curve::{hair_card, hair_clump, ClumpParams, Curve, HairCardParams, NormalBiasDir};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{Corner, EditMesh, Face};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

const APEX_EPS: f32 = 1.0e-7;

/// Malha gerada + dados por vértice.
#[derive(Debug, Clone)]
pub struct HairMesh {
    pub mesh: EditMesh,
    /// Id da mecha que originou cada vértice.
    pub vert_strand: Vec<u32>,
    /// Fração do comprimento (0 raiz → 1 ponta) de cada vértice.
    pub vert_t: Vec<f32>,
    /// Direção do fio (raiz → ponta, unitária) em cada vértice.
    pub vert_dir: Vec<Vec3>,
}

impl HairMesh {
    fn new(name: &str) -> Self {
        Self {
            mesh: EditMesh::new(name),
            vert_strand: Vec::new(),
            vert_t: Vec::new(),
            vert_dir: Vec::new(),
        }
    }

    fn append(&mut self, other: HairMesh) {
        self.mesh.append(&other.mesh);
        self.vert_strand.extend(other.vert_strand);
        self.vert_t.extend(other.vert_t);
        self.vert_dir.extend(other.vert_dir);
    }
}

/// Soma `twist·s/L` ao tilt de cada ponto de controle (fração de comprimento por corda acumulada).
fn bake_twist(curve: &Curve, twist: f32) -> Curve {
    if twist.abs() < 1e-7 || curve.points.len() < 2 {
        return curve.clone();
    }
    let mut cum = vec![0.0f32];
    for w in curve.points.windows(2) {
        cum.push(cum.last().copied().unwrap_or(0.0) + w[0].pos.distance(w[1].pos));
    }
    let total = cum.last().copied().unwrap_or(0.0).max(1e-9);
    let mut c = curve.clone();
    for (p, s) in c.points.iter_mut().zip(cum) {
        p.tilt += twist * s / total;
    }
    c
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn dedup_face(corners: &[(u32, Vec2, Option<Vec3>)], smooth: bool, material: u16) -> Option<Face> {
    let mut list: Vec<(u32, Vec2, Option<Vec3>)> = Vec::with_capacity(corners.len());
    for &c in corners {
        if list.last().map(|l| l.0 == c.0).unwrap_or(false) {
            continue;
        }
        list.push(c);
    }
    while list.len() > 1 && list[0].0 == list[list.len() - 1].0 {
        list.pop();
    }
    if list.len() < 3 {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    if !list.iter().all(|c| seen.insert(c.0)) {
        return None;
    }
    Some(Face {
        corners: list
            .into_iter()
            .map(|(vert, uv, normal)| Corner { vert, uv, normal })
            .collect(),
        material,
        smooth,
        face_set: 0,
        hidden: false,
    })
}

/// Parâmetros comuns de uma peça (um card/mecha).
struct Piece<'a> {
    curve: Curve,
    section: HairSection,
    strand: &'a HairStrand,
    group: &'a HairGroup,
    scalp_center: Vec3,
}

fn bias_normal(n: Vec3, p: Vec3, center: Vec3, amount: f32) -> Vec3 {
    let Some(d) = (p - center).try_normalize() else {
        return n;
    };
    (n * (1.0 - amount) + d * amount).try_normalize().unwrap_or(n)
}

fn build_card(piece: &Piece) -> ModelResult<(EditMesh, Vec<f32>, Vec<Vec3>)> {
    let st = piece.strand;
    let sec = piece.section;
    let params = HairCardParams {
        width: sec.width,
        taper: st.params.tip_taper,
        taper_power: st.params.taper_power,
        segments: st.params.resolution,
        width_segments: if sec.kind == SectionKind::Arc { 4 } else { 1 },
        curl: if sec.kind == SectionKind::Arc { sec.curl } else { 0.0 },
        flare: st.params.flare,
        normal_bias_to: Some(piece.scalp_center),
        normal_bias_amount: piece.group.material.normal_bias,
        normal_bias_dir: NormalBiasDir::AwayFromTarget,
        uv_atlas_cell: None,
        up_hint: st.root_normal.try_normalize(),
    };
    let mesh = hair_card(&piece.curve, &params)?;
    // t por vértice: a V do card é r/(rings−1) e cada vértice só existe num anel
    let rings = st.params.resolution as usize + 1;
    let mut t = vec![0.0f32; mesh.vert_count()];
    for f in &mesh.faces {
        for c in &f.corners {
            t[c.vert as usize] = c.uv.y.clamp(0.0, 1.0);
        }
    }
    let samples = piece.curve.sample_uniform(rings)?;
    let dir: Vec<Vec3> = t
        .iter()
        .map(|&t| samples[((t * (rings - 1) as f32).round() as usize).min(rings - 1)].tangent)
        .collect();
    Ok((mesh, t, dir))
}

fn build_solid(piece: &Piece) -> ModelResult<(EditMesh, Vec<f32>, Vec<Vec3>)> {
    let st = piece.strand;
    let sec = piece.section;
    let sides = sec.kind.sides();
    let rings = st.params.resolution as usize + 1;
    let (samples, frames) = piece.curve.frames_uniform(rings, st.root_normal.try_normalize())?;
    let total = samples.last().map(|s| s.s).unwrap_or(0.0).max(1e-9);
    let bias = piece.group.material.normal_bias;
    let mut mesh = EditMesh::new("HairStrand");
    let mut t_of: Vec<f32> = Vec::new();
    let mut dir_of: Vec<Vec3> = Vec::new();
    let mut ids: Vec<Vec<u32>> = Vec::with_capacity(rings);
    let mut ring_normals: Vec<Vec<Vec3>> = Vec::with_capacity(rings);
    let mut apex = vec![false; rings];
    let mut tvals = vec![0.0f32; rings];
    for r in 0..rings {
        let t = r as f32 / (rings - 1) as f32;
        tvals[r] = samples[r].s / total;
        let taper = 1.0 + (st.params.tip_taper - 1.0) * t.powf(st.params.taper_power);
        let k = taper * (1.0 + st.params.flare * (std::f32::consts::PI * t).sin()) * samples[r].radius;
        let (hw, ht) = (0.5 * sec.width * k, 0.5 * sec.thickness * k);
        let f = &frames[r];
        if hw.max(ht) <= APEX_EPS && r == rings - 1 {
            apex[r] = true;
            let v = mesh.add_vert(f.pos);
            t_of.push(tvals[r]);
            dir_of.push(f.tangent);
            ids.push(vec![v; sides]);
            ring_normals.push(vec![bias_normal(f.tangent, f.pos, piece.scalp_center, bias); sides]);
            continue;
        }
        let (hw, ht) = (hw.max(1e-6), ht.max(1e-6));
        let mut row = Vec::with_capacity(sides);
        let mut nrm = Vec::with_capacity(sides);
        for s in 0..sides {
            let th = TAU * s as f32 / sides as f32;
            let p = f.pos + f.normal * (ht * th.cos()) + f.binormal * (hw * th.sin());
            let n = (f.normal * (th.cos() / ht) + f.binormal * (th.sin() / hw)).try_normalize().unwrap_or(f.normal);
            row.push(mesh.add_vert(p));
            t_of.push(tvals[r]);
            dir_of.push(f.tangent);
            nrm.push(bias_normal(n, p, piece.scalp_center, bias));
        }
        ids.push(row);
        ring_normals.push(nrm);
    }
    for r in 0..rings - 1 {
        let (va, vb) = (tvals[r], tvals[r + 1]);
        for s in 0..sides {
            let s1 = (s + 1) % sides;
            let (u0, u1) = (s as f32 / sides as f32, (s + 1) as f32 / sides as f32);
            let um = 0.5 * (u0 + u1);
            let ua = if apex[r] { (um, um) } else { (u0, u1) };
            let ub = if apex[r + 1] { (um, um) } else { (u0, u1) };
            let corners = [
                (ids[r][s], Vec2::new(ua.0, va), Some(ring_normals[r][s])),
                (ids[r][s1], Vec2::new(ua.1, va), Some(ring_normals[r][s1])),
                (ids[r + 1][s1], Vec2::new(ub.1, vb), Some(ring_normals[r + 1][s1])),
                (ids[r + 1][s], Vec2::new(ub.0, vb), Some(ring_normals[r + 1][s])),
            ];
            if let Some(face) = dedup_face(&corners, true, 0) {
                mesh.push_face(face)?;
            }
        }
    }
    // tampas (raiz: para dentro da cabeça; ponta: se não terminar em bico)
    let cap = |mesh: &mut EditMesh, ring: usize, reverse: bool| -> ModelResult<()> {
        let order: Vec<usize> = if reverse { (0..sides).rev().collect() } else { (0..sides).collect() };
        let n = if reverse { -frames[ring].tangent } else { frames[ring].tangent };
        let corners: Vec<(u32, Vec2, Option<Vec3>)> = order
            .iter()
            .map(|&s| {
                // UV da tampa: mesma linha V do anel (sem esticar a textura de mecha)
                (ids[ring][s], Vec2::new(s as f32 / sides as f32, tvals[ring]), Some(n))
            })
            .collect();
        if let Some(face) = dedup_face(&corners, false, 0) {
            mesh.push_face(face)?;
        }
        Ok(())
    };
    cap(&mut mesh, 0, true)?;
    if !apex[rings - 1] {
        cap(&mut mesh, rings - 1, false)?;
    }
    mesh.validate()?;
    Ok((mesh, t_of, dir_of))
}

fn build_piece(piece: &Piece, atlas_index: u32) -> ModelResult<HairMesh> {
    let group = piece.group;
    let (mut mesh, t, dir) = if piece.section.kind.is_solid() {
        build_solid(piece)?
    } else {
        build_card(piece)?
    };
    let uv_swap = group.uv_layout == UvLayout::LengthU;
    let cell = group.atlas.map(|a| {
        if group.atlas_cycle {
            let n = (a.cols * a.rows).max(1);
            let k = atlas_index % n;
            (a.cols, a.rows, k % a.cols, k / a.cols)
        } else {
            (a.cols, a.rows, a.col, a.row)
        }
    });
    for f in &mut mesh.faces {
        f.material = group.material_slot;
        for c in &mut f.corners {
            let mut uv = c.uv;
            if uv_swap {
                uv = Vec2::new(uv.y, uv.x);
            }
            if let Some((cols, rows, col, row)) = cell {
                uv = Vec2::new((col as f32 + uv.x) / cols as f32, (row as f32 + uv.y) / rows as f32);
            }
            c.uv = uv;
        }
    }
    // AO de raiz no canal R (canais NPR: R = AO, G = deslocamento de sombra, B = contorno, A = spec/rim)
    let root_ao = group.material.root_ao;
    for (v, &t) in mesh.verts.iter_mut().zip(&t) {
        v.color = [root_ao + (1.0 - root_ao) * smoothstep(0.0, 0.6, t), 0.5, 1.0, 1.0];
    }
    let n = mesh.vert_count();
    Ok(HairMesh {
        mesh,
        vert_strand: vec![piece.strand.id; n],
        vert_t: t,
        vert_dir: dir,
    })
}

/// Malha de UMA mecha (com clump, várias peças).
pub fn build_strand(strand: &HairStrand, group: &HairGroup, scalp_center: Vec3) -> ModelResult<HairMesh> {
    strand.validate()?;
    let guide = bake_twist(&strand.guide, strand.params.section.twist);
    let mut out = HairMesh::new("HairStrand");
    match &strand.params.clump {
        None => {
            let piece = Piece {
                curve: guide,
                section: strand.params.section,
                strand,
                group,
                scalp_center,
            };
            out.append(build_piece(&piece, strand.id)?);
        }
        Some(c) => {
            let fibers = hair_clump(
                &guide,
                &ClumpParams {
                    count: c.count,
                    spread: c.spread,
                    tip_spread: c.tip_spread,
                    noise: c.noise,
                    twist: c.twist,
                    points: c.points,
                    length_variation: c.length_variation,
                    seed: c.seed ^ (strand.id as u64).wrapping_mul(0x9E37_79B9),
                },
            )?;
            let section = HairSection {
                width: c.strand_width,
                ..strand.params.section
            };
            for (i, fiber) in fibers.into_iter().enumerate() {
                let piece = Piece {
                    curve: fiber,
                    section,
                    strand,
                    group,
                    scalp_center,
                };
                out.append(build_piece(&piece, strand.id.wrapping_add(i as u32))?);
            }
        }
    }
    Ok(out)
}

/// Malha de todo o grupo (todas as mechas, mesmo slot de material). Grupo vazio → malha vazia válida.
pub fn build_group(group: &HairGroup, scalp_center: Vec3) -> ModelResult<HairMesh> {
    group.validate()?;
    let mut out = HairMesh::new(&group.name);
    for s in &group.strands {
        let m = build_strand(s, group, scalp_center)
            .map_err(|e| ModelError::InvalidParameter(format!("mecha {} do grupo '{}': {e}", s.id, group.name)))?;
        out.append(m);
    }
    out.mesh.name = group.name.clone();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::drop_guide;
    use super::super::{HairAtlas, HairClump, HairSet, StrandParams};
    use super::*;
    use crate::topology::Topology;

    fn group_with(kind: SectionKind, tip_taper: f32) -> (HairSet, u32) {
        let mut s = HairSet::new();
        let g = s.add_group("T").unwrap();
        let mut p = StrandParams::default();
        p.section.kind = kind;
        p.tip_taper = tip_taper;
        s.add_strand(g, drop_guide(Vec3::new(0.0, 1.0, 0.0), 0.8, 5), p, Vec3::Z).unwrap();
        (s, g)
    }

    #[test]
    fn every_section_builds_a_valid_mesh_with_uv_along_the_length() {
        for kind in SectionKind::ALL {
            let (s, g) = group_with(kind, 0.0);
            let hm = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
            hm.mesh.validate().unwrap();
            assert!(hm.mesh.face_count() > 0, "{kind:?}");
            assert_eq!(hm.vert_t.len(), hm.mesh.vert_count());
            assert_eq!(hm.vert_dir.len(), hm.mesh.vert_count());
            assert_eq!(hm.vert_strand.len(), hm.mesh.vert_count());
            // V cresce da raiz à ponta (cada vértice tem t coerente com a altura: guia desce em Y)
            let mut pairs: Vec<(f32, f32)> = hm.mesh.verts.iter().zip(&hm.vert_t).map(|(v, &t)| (t, v.co.y)).collect();
            pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
            assert!(pairs.first().unwrap().1 > pairs.last().unwrap().1, "{kind:?}: raiz acima da ponta");
            // direção do fio aponta para baixo e é unitária
            assert!(hm.vert_dir.iter().all(|d| (d.length() - 1.0).abs() < 1e-3 && d.y < -0.5), "{kind:?}");
            // UV: V no intervalo [0,1] e todas as faces carregam normais customizadas
            for f in &hm.mesh.faces {
                for c in &f.corners {
                    assert!((0.0..=1.0 + 1e-5).contains(&c.uv.y) && (0.0..=1.0 + 1e-5).contains(&c.uv.x), "{kind:?} uv {:?}", c.uv);
                }
            }
            if kind.is_solid() {
                assert!(hm.mesh.faces.iter().filter(|f| f.smooth).all(|f| f.corners.iter().all(|c| c.normal.is_some())));
                let topo = Topology::build(&hm.mesh);
                assert!(topo.is_closed_manifold(), "{kind:?} sólido deve ser fechado (ponta em bico + tampa na raiz)");
            }
        }
    }

    #[test]
    fn tip_taper_zero_ends_in_a_point_and_one_keeps_width() {
        for kind in SectionKind::ALL {
            let (s0, g0) = group_with(kind, 0.0);
            let m0 = build_group(s0.group(g0).unwrap(), Vec3::ZERO).unwrap();
            let ys: Vec<f32> = m0.mesh.verts.iter().map(|v| v.co.y).collect();
            let ymin = ys.iter().cloned().fold(f32::INFINITY, f32::min);
            let at_tip = m0.mesh.verts.iter().filter(|v| (v.co.y - ymin).abs() < 1e-4).count();
            assert_eq!(at_tip, 1, "{kind:?}: ponta degenerada num único vértice");
            // tip_taper = 1: a ponta mantém largura (vários vértices no anel final)
            let (s1, g1) = group_with(kind, 1.0);
            let m1 = build_group(s1.group(g1).unwrap(), Vec3::ZERO).unwrap();
            let ymin1 = m1.mesh.verts.iter().map(|v| v.co.y).fold(f32::INFINITY, f32::min);
            let ring = m1.mesh.verts.iter().filter(|v| (v.co.y - ymin1).abs() < 1e-4).count();
            assert!(ring >= 2, "{kind:?}: ponta com largura tem >= 2 vértices, {ring}");
        }
    }

    #[test]
    fn normals_bias_away_from_the_head_and_uv_layout_swaps() {
        let (mut s, g) = group_with(SectionKind::Tube, 0.2);
        s.group_mut(g).unwrap().material.normal_bias = 1.0;
        let center = Vec3::new(0.0, 0.5, 0.0);
        let hm = build_group(s.group(g).unwrap(), center).unwrap();
        // viés total: as normais dos corners são a direção radial a partir do centro da cabeça
        for f in hm.mesh.faces.iter().filter(|f| f.smooth) {
            for c in &f.corners {
                let p = hm.mesh.co(c.vert);
                let want = (p - center).normalize();
                assert!(c.normal.unwrap().dot(want) > 0.999);
            }
        }
        let v_len = |m: &HairMesh| m.mesh.faces.iter().flat_map(|f| f.corners.iter().map(|c| c.uv)).fold((0.0f32, 0.0f32), |a, uv| (a.0.max(uv.x), a.1.max(uv.y)));
        assert!((v_len(&hm).1 - 1.0).abs() < 1e-4);
        s.group_mut(g).unwrap().uv_layout = UvLayout::LengthU;
        let swapped = build_group(s.group(g).unwrap(), center).unwrap();
        // com LengthU, o comprimento (0..1 ao longo do fio) está em U: o par de maiores valores troca
        let mx = v_len(&swapped);
        assert!((mx.0 - 1.0).abs() < 1e-4, "U cobre o comprimento");
        // vértice de raiz tem U ~ 0 e o de ponta U ~ 1
        let root_u = swapped.mesh.faces.iter().flat_map(|f| f.corners.iter()).filter(|c| swapped.vert_t[c.vert as usize] < 1e-5).map(|c| c.uv.x).fold(0.0f32, f32::max);
        assert!(root_u < 1e-4);
    }

    #[test]
    fn twist_rotates_the_card_and_width_follows_params() {
        let (mut s, g) = group_with(SectionKind::Flat, 1.0);
        let flat = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        let spread = |m: &HairMesh| {
            let (lo, hi) = m.mesh.bounds().unwrap();
            hi - lo
        };
        let f = spread(&flat);
        s.group_mut(g).unwrap().strands[0].params.section.twist = std::f32::consts::FRAC_PI_2;
        let twisted = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        let t = spread(&twisted);
        // sem torção a fita fica num plano (extensão pequena num dos eixos horizontais); com 90° o plano gira
        assert!(f.x.min(f.z) < 0.02, "card plano: {f:?}");
        assert!((t - f).length() > 0.01 || t.x.min(t.z) > f.x.min(f.z) + 0.005, "a torção muda a forma: {f:?} -> {t:?}");
        // largura: dobrar a largura dobra a extensão na direção da fita
        s.group_mut(g).unwrap().strands[0].params.section.twist = 0.0;
        let w1 = spread(&flat).x.max(spread(&flat).z);
        s.group_mut(g).unwrap().strands[0].params.section.width *= 2.0;
        let w2 = {
            let m = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
            spread(&m).x.max(spread(&m).z)
        };
        assert!((w2 / w1 - 2.0).abs() < 0.15, "{w1} -> {w2}");
    }

    #[test]
    fn clump_builds_multiple_fibres_deterministically() {
        let (mut s, g) = group_with(SectionKind::Flat, 0.0);
        s.group_mut(g).unwrap().strands[0].params.clump = Some(HairClump::default());
        let a = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        let b = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        assert_eq!(a.mesh.content_hash(), b.mesh.content_hash());
        let single = {
            let (s1, g1) = group_with(SectionKind::Flat, 0.0);
            build_group(s1.group(g1).unwrap(), Vec3::ZERO).unwrap()
        };
        assert!(a.mesh.face_count() > single.mesh.face_count() * 3, "clump de 5 fios");
        a.mesh.validate().unwrap();
    }

    #[test]
    fn material_slot_atlas_and_root_ao_land_on_the_mesh() {
        let (mut s, g) = group_with(SectionKind::Arc, 0.0);
        {
            let gr = s.group_mut(g).unwrap();
            gr.material_slot = 3;
            gr.atlas = Some(HairAtlas { cols: 4, rows: 2, col: 1, row: 1 });
            gr.material.root_ao = 0.5;
        }
        let hm = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        assert!(hm.mesh.faces.iter().all(|f| f.material == 3));
        for f in &hm.mesh.faces {
            for c in &f.corners {
                assert!(c.uv.x >= 0.25 - 1e-5 && c.uv.x <= 0.5 + 1e-5 && c.uv.y >= 0.5 - 1e-5, "célula do atlas: {:?}", c.uv);
            }
        }
        let (root, tip): (Vec<_>, Vec<_>) = hm.mesh.verts.iter().zip(&hm.vert_t).partition(|(_, &t)| t < 0.05);
        assert!(root.iter().all(|(v, _)| (v.color[0] - 0.5).abs() < 0.05), "AO de raiz");
        assert!(tip.iter().any(|(v, _)| v.color[0] > 0.95), "AO sobe até 1");
        // ciclo de atlas varia a célula por mecha
        let gr = s.group_mut(g).unwrap();
        gr.atlas_cycle = true;
        let p = gr.strands[0].params;
        let mut second = gr.strands[0].clone();
        second.id = 99;
        second.params = p;
        gr.strands.push(second);
        let hm2 = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        let cells: std::collections::HashSet<(i32, i32)> = hm2
            .mesh
            .faces
            .iter()
            .flat_map(|f| f.corners.iter().map(|c| ((c.uv.x * 4.0 - 1e-4).floor() as i32, (c.uv.y * 2.0 - 1e-4).floor() as i32)))
            .collect();
        assert!(cells.len() >= 2, "mechas diferentes usam células diferentes: {cells:?}");
    }

    #[test]
    fn generation_is_deterministic_and_errors_name_the_strand() {
        let (s, g) = group_with(SectionKind::Triangle, 0.1);
        let a = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        let b = build_group(s.group(g).unwrap(), Vec3::ZERO).unwrap();
        assert_eq!(a.mesh.content_hash(), b.mesh.content_hash());
        let mut bad = s.group(g).unwrap().clone();
        bad.strands[0].params.section.width = 0.0;
        let e = build_group(&bad, Vec3::ZERO).unwrap_err().to_string();
        assert!(e.contains("width"), "{e}");
        let empty = HairGroup::new(1, "vazio");
        assert_eq!(build_group(&empty, Vec3::ZERO).unwrap().mesh.face_count(), 0);
    }
}
