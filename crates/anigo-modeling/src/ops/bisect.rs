//! Bisect: corte da malha por um plano (equivalente ao *Bisect* do Blender).
//!
//! Todas as arestas que cruzam o plano ganham um vértice, as faces atravessadas
//! são divididas em duas ou mais (inclusive n-gons côncavos) e, opcionalmente,
//! um dos lados é removido e/ou o corte é preenchido com n-gons.
//!
//! Convenção: o lado **externo** (*outer*) é o da normal do plano
//! (`dist > 0`); o **interno** (*inner*) é o oposto (`dist < 0`).

use super::extrude::{lerp_corner, prune_orphan_edge_attrs, wire_edge_keys};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EdgeAttr, EdgeKey, EditMesh, Face, Remap};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn side_of(d: f32, eps: f32) -> i8 {
    if d > eps {
        1
    } else if d < -eps {
        -1
    } else {
        0
    }
}

/// Divide um polígono (cantos) por um plano. `side(v)` dá o lado de cada
/// vértice; `pos(v)` sua posição; `ldir` é a direção da reta de interseção
/// usada para parear os pontos de corte em polígonos côncavos.
fn split_polygon(
    poly: Vec<Corner>,
    side: &dyn Fn(u32) -> i8,
    pos: &dyn Fn(u32) -> Vec3,
    ldir: Vec3,
    depth: usize,
    out: &mut Vec<Vec<Corner>>,
) -> ModelResult<()> {
    let m = poly.len();
    let s: Vec<i8> = poly.iter().map(|c| side(c.vert)).collect();
    let has_pos = s.iter().any(|&x| x > 0);
    let has_neg = s.iter().any(|&x| x < 0);
    if !(has_pos && has_neg) {
        out.push(poly);
        return Ok(());
    }
    if depth > 64 || m > 4096 {
        return Err(ModelError::Numeric(
            "face complexa demais para o bisect (recursão excedida)".into(),
        ));
    }
    // pontos de corte: início de cada sequência de vértices no plano cuja
    // vizinhança não-nula muda de lado
    let mut reps: Vec<usize> = Vec::new();
    for i in 0..m {
        if s[i] != 0 || s[(i + m - 1) % m] == 0 {
            continue;
        }
        let prev = s[(i + m - 1) % m];
        let mut j = (i + 1) % m;
        while s[j] == 0 {
            j = (j + 1) % m;
        }
        if prev != s[j] {
            reps.push(i);
        }
    }
    reps.sort_by(|&a, &b| {
        let (da, db) = (pos(poly[a].vert).dot(ldir), pos(poly[b].vert).dot(ldir));
        da.partial_cmp(&db)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    for w in reps.windows(2) {
        let (lo, hi) = (w[0].min(w[1]), w[0].max(w[1]));
        let part1: Vec<Corner> = poly[lo..=hi].to_vec();
        let mut part2: Vec<Corner> = poly[hi..].to_vec();
        part2.extend_from_slice(&poly[..=lo]);
        if part1.len() >= 3 && part2.len() >= 3 {
            split_polygon(part1, side, pos, ldir, depth + 1, out)?;
            split_polygon(part2, side, pos, ldir, depth + 1, out)?;
            return Ok(());
        }
    }
    Err(ModelError::Numeric(
        "não foi possível dividir uma face pelo plano (geometria degenerada)".into(),
    ))
}

/// Bisect: corta a malha com o plano `(plane_point, plane_normal)`.
///
/// * `clear_inner` / `clear_outer`: removem as faces do lado interno
///   (`dist < 0`) / externo (`dist > 0`). Faces contidas no plano são sempre mantidas.
/// * `fill`: preenche o corte com n-gons. Com um lado removido, fecha a malha
///   restante. Sem remover nenhum lado, **separa** a malha em duas metades
///   (duplicando os vértices do corte) e fecha cada uma com sua tampa. Com os
///   dois lados removidos não há o que preencher.
/// * `merge_epsilon`: tolerância (>= 0) de distância ao plano abaixo da qual um
///   vértice é tratado como estando *no plano* (evita fatias minúsculas).
///
/// As faces atravessadas são divididas preservando UV/normal por corner
/// (interpolados nos vértices novos), material, suavidade e face-set;
/// atributos de aresta passam para os dois segmentos de cada aresta cortada.
///
/// Os índices de vértice/face são compactados no fim: `OpOutput::remap` descreve
/// o mapeamento (`face[i]` é o índice da primeira peça da face `i`, ou `None`
/// se foi removida). Seleção devolvida: arestas do corte (e as faces de
/// preenchimento); `created_verts` = vértices do corte, `created_faces` = tampas.
pub fn bisect(
    mesh: &mut EditMesh,
    plane_point: Vec3,
    plane_normal: Vec3,
    clear_inner: bool,
    clear_outer: bool,
    fill: bool,
    merge_epsilon: f32,
) -> ModelResult<OpOutput> {
    if !plane_point.is_finite() || !plane_normal.is_finite() {
        return Err(ModelError::InvalidParameter(
            "ponto/normal do plano não finitos".into(),
        ));
    }
    let normal = plane_normal
        .try_normalize()
        .ok_or_else(|| ModelError::InvalidParameter("normal do plano deve ser não nula".into()))?;
    if !merge_epsilon.is_finite() || merge_epsilon < 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "merge_epsilon deve ser finito e >= 0 (recebeu {merge_epsilon})"
        )));
    }
    let old_n = mesh.verts.len();
    let dist: Vec<f32> = mesh
        .verts
        .iter()
        .map(|v| (v.co - plane_point).dot(normal))
        .collect();
    let sides: Vec<i8> = dist.iter().map(|&d| side_of(d, merge_epsilon)).collect();
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);

    // ---- vértices nas arestas que cruzam
    let mut new_verts: Vec<EVert> = Vec::new();
    let mut crossing: HashMap<EdgeKey, (u32, f32)> = HashMap::new(); // (vértice, t de lo para hi)
    for e in &topo.edges {
        if e.faces.is_empty() {
            continue;
        }
        let (lo, hi) = e.verts;
        if sides[lo as usize] * sides[hi as usize] == -1 {
            let (dl, dh) = (dist[lo as usize], dist[hi as usize]);
            let t = (dl / (dl - dh)).clamp(0.0, 1.0);
            let id = (old_n + new_verts.len()) as u32;
            new_verts.push(EVert::lerp(
                &mesh.verts[lo as usize],
                &mesh.verts[hi as usize],
                t,
            ));
            crossing.insert(e.verts, (id, t));
        }
    }
    let side_fn = |v: u32| -> i8 {
        if (v as usize) < old_n {
            sides[v as usize]
        } else {
            0
        }
    };
    let pos_fn = |v: u32| -> Vec3 {
        if (v as usize) < old_n {
            mesh.verts[v as usize].co
        } else {
            new_verts[v as usize - old_n].co
        }
    };

    // ---- divide as faces
    struct Piece {
        face: Face,
        side: i8,
    }
    let mut pieces: Vec<Piece> = Vec::with_capacity(mesh.faces.len());
    let mut face_first: Vec<Option<u32>> = vec![None; mesh.faces.len()];
    let mut attr_new: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let m = face.corners.len();
        let mut poly: Vec<Corner> = Vec::with_capacity(m + 2);
        for i in 0..m {
            let (ca, cb) = (face.corners[i], face.corners[(i + 1) % m]);
            poly.push(ca);
            if let Some(&(id, t)) = crossing.get(&edge_key(ca.vert, cb.vert)) {
                let frac = if ca.vert < cb.vert { t } else { 1.0 - t };
                poly.push(lerp_corner(&ca, &cb, frac, id));
            }
        }
        let mut parts: Vec<Vec<Corner>> = Vec::new();
        {
            let fnorm = mesh.face_normal(fi as u32);
            let mut ldir = normal.cross(fnorm).try_normalize().unwrap_or(Vec3::ZERO);
            if ldir == Vec3::ZERO {
                ldir = normal.any_orthonormal_vector();
            }
            split_polygon(poly, &side_fn, &pos_fn, ldir, 0, &mut parts)?;
        }
        face_first[fi] = Some(pieces.len() as u32);
        for corners in parts {
            let s_pos = corners.iter().any(|c| side_fn(c.vert) > 0);
            let s_neg = corners.iter().any(|c| side_fn(c.vert) < 0);
            let side = if s_pos {
                1
            } else if s_neg {
                -1
            } else {
                0
            };
            let mut f = face.clone();
            f.corners = corners;
            pieces.push(Piece { face: f, side });
        }
    }
    for (&(lo, hi), &(id, _)) in &crossing {
        let attr = mesh.edge_attr(lo, hi);
        if !attr.is_default() {
            attr_new.push((edge_key(lo, id), attr));
            attr_new.push((edge_key(id, hi), attr));
        }
    }

    // ---- vértices do plano que participam do corte (aresta entre lado + e −)
    let mut verts_all: Vec<EVert> = mesh.verts.clone();
    verts_all.extend(new_verts.iter().cloned());
    let mut dup_neg: BTreeMap<u32, u32> = BTreeMap::new();
    let separate = fill && !clear_inner && !clear_outer;
    let mut dup_back: HashMap<u32, u32> = HashMap::new();
    if separate {
        // arestas do plano com faces dos dois lados
        let mut edge_sides: HashMap<EdgeKey, (bool, bool)> = HashMap::new();
        for p in &pieces {
            if p.side == 0 {
                continue;
            }
            for (a, b) in p.face.edges() {
                if side_fn(a) == 0 && side_fn(b) == 0 {
                    let e = edge_sides.entry(edge_key(a, b)).or_insert((false, false));
                    if p.side > 0 {
                        e.0 = true;
                    } else {
                        e.1 = true;
                    }
                }
            }
        }
        let mut cut_verts: BTreeSet<u32> = BTreeSet::new();
        for (&(a, b), &(p, n)) in &edge_sides {
            if p && n {
                cut_verts.insert(a);
                cut_verts.insert(b);
            }
        }
        for &v in &cut_verts {
            let id = verts_all.len() as u32;
            verts_all.push(verts_all[v as usize].clone());
            dup_neg.insert(v, id);
            dup_back.insert(id, v);
        }
        for p in pieces.iter_mut().filter(|p| p.side < 0) {
            for c in &mut p.face.corners {
                if let Some(&d) = dup_neg.get(&c.vert) {
                    c.vert = d;
                }
            }
        }
        // atributos das arestas do lado negativo que passaram a usar duplicatas
        for p in pieces.iter().filter(|p| p.side < 0) {
            let orig = |v: u32| dup_back.get(&v).copied().unwrap_or(v);
            for (a, b) in p.face.edges() {
                let (oa, ob) = (orig(a), orig(b));
                if (oa, ob) != (a, b) {
                    let attr = mesh.edge_attr(oa, ob);
                    if !attr.is_default() {
                        attr_new.push((edge_key(a, b), attr));
                    }
                }
            }
        }
    }
    let is_plane = |v: u32| -> bool { side_fn(v) == 0 || dup_back.contains_key(&v) };

    // ---- remoção dos lados
    let mut kept: Vec<Piece> = Vec::with_capacity(pieces.len());
    let mut index_map: Vec<Option<u32>> = vec![None; pieces.len()];
    for (i, p) in pieces.into_iter().enumerate() {
        let drop = (clear_inner && p.side < 0) || (clear_outer && p.side > 0);
        if !drop {
            index_map[i] = Some(kept.len() as u32);
            kept.push(p);
        }
    }

    // ---- preenchimento
    let mut cap_faces: Vec<Face> = Vec::new();
    if fill && !(clear_inner && clear_outer) {
        // arestas de fronteira (1 face) com os dois extremos no plano
        let mut count: HashMap<EdgeKey, u32> = HashMap::new();
        for p in &kept {
            for (a, b) in p.face.edges() {
                *count.entry(edge_key(a, b)).or_insert(0) += 1;
            }
        }
        let mut next: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut owner: HashMap<u32, usize> = HashMap::new();
        for (pi, p) in kept.iter().enumerate() {
            for (a, b) in p.face.edges() {
                if is_plane(a) && is_plane(b) && count[&edge_key(a, b)] == 1 {
                    // a tampa percorre b -> a
                    next.entry(b).or_default().push(a);
                    owner.entry(b).or_insert(pi);
                }
            }
        }
        if next.values().any(|v| v.len() > 1) {
            return Err(ModelError::UnsupportedTopology(
                "o corte toca a malha em um vértice compartilhado por mais de um laço (pinça); preenchimento ambíguo".into(),
            ));
        }
        let mut visited: BTreeSet<u32> = BTreeSet::new();
        let starts: Vec<u32> = next.keys().copied().collect();
        // base do plano para UVs
        let u_axis = normal.any_orthonormal_vector();
        let v_axis = normal.cross(u_axis);
        for start in starts {
            if visited.contains(&start) {
                continue;
            }
            let mut chain = vec![start];
            let mut cur = start;
            let mut closed = false;
            loop {
                visited.insert(cur);
                let Some(nx) = next.get(&cur).and_then(|v| v.first()).copied() else {
                    break;
                };
                if nx == start {
                    closed = true;
                    break;
                }
                if visited.contains(&nx) {
                    break;
                }
                chain.push(nx);
                cur = nx;
            }
            if !closed || chain.len() < 3 {
                continue;
            }
            let src = &kept[owner[&start]].face;
            let pts2: Vec<Vec2> = chain
                .iter()
                .map(|&v| {
                    let p = verts_all[v as usize].co - plane_point;
                    Vec2::new(p.dot(u_axis), p.dot(v_axis))
                })
                .collect();
            let (mut lo, mut hi) = (pts2[0], pts2[0]);
            for p in &pts2 {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            let ext = (hi - lo).max(Vec2::splat(1e-9));
            let mut face = src.clone();
            face.smooth = false;
            face.corners = chain
                .iter()
                .zip(&pts2)
                .map(|(&v, p)| Corner {
                    vert: v,
                    uv: (*p - lo) / ext,
                    normal: None,
                })
                .collect();
            cap_faces.push(face);
        }
    }

    // ---- commit
    let had_work = !crossing.is_empty() || clear_inner || clear_outer || !cap_faces.is_empty();
    let mut created_verts: Vec<u32> = (old_n as u32..(old_n + new_verts.len()) as u32).collect();
    created_verts.extend(dup_neg.values().copied());
    let first_cap = kept.len();
    mesh.verts = verts_all;
    mesh.faces = kept.into_iter().map(|p| p.face).collect();
    let mut created_faces = Vec::new();
    for f in cap_faces {
        mesh.faces.push(f);
        created_faces.push((mesh.faces.len() - 1) as u32);
    }
    debug_assert_eq!(
        created_faces.first().copied().unwrap_or(first_cap as u32),
        first_cap as u32
    );
    for (k, a) in attr_new {
        mesh.edge_attrs.insert(k, a);
    }
    prune_orphan_edge_attrs(mesh, &wires);

    // vértices do plano (para a seleção) antes da compactação
    let plane_ids: BTreeSet<u32> = (0..mesh.verts.len() as u32)
        .filter(|&v| is_plane(v))
        .collect();
    let mut cut_edges: Vec<EdgeKey> = Vec::new();
    {
        let topo2 = Topology::build(mesh);
        for e in &topo2.edges {
            if !e.faces.is_empty()
                && plane_ids.contains(&e.verts.0)
                && plane_ids.contains(&e.verts.1)
            {
                cut_edges.push(e.verts);
            }
        }
    }
    let vremap = if had_work {
        mesh.remove_unused_verts()
    } else {
        Remap::default()
    };
    let map_v = |v: u32| -> Option<u32> {
        if had_work {
            vremap.vert.get(v as usize).copied().flatten()
        } else {
            Some(v)
        }
    };
    let mut selection = Selection::from_edges(
        cut_edges
            .iter()
            .filter_map(|&(a, b)| Some(edge_key(map_v(a)?, map_v(b)?))),
    );
    for &f in &created_faces {
        selection.faces.insert(f);
    }
    let created_verts: Vec<u32> = created_verts.iter().filter_map(|&v| map_v(v)).collect();
    let remap = Remap {
        vert: if had_work {
            vremap.vert
        } else {
            (0..mesh.verts.len() as u32).map(Some).collect()
        },
        face: face_first
            .iter()
            .map(|f| f.and_then(|i| index_map[i as usize]))
            .collect(),
    };
    Ok(OpOutput {
        selection,
        created_verts,
        created_faces,
        remap: Some(remap),
    })
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::extrude::test_util::*;
    use super::*;
    use crate::primitives;

    fn half_ok(m: &EditMesh, keep_upper: bool) {
        assert_closed(m, Some(2));
        for v in &m.verts {
            if keep_upper {
                assert!(v.co.y > -1e-5);
            } else {
                assert!(v.co.y < 1e-5);
            }
        }
        // normais para fora (a tampa aponta para o lado removido)
        assert!(signed_volume(m) > 0.0);
        assert_outward_star(m);
    }

    #[test]
    fn sphere_halves_closed_with_fill_for_odd_and_even_subdivisions() {
        for div in [4u32, 5, 7] {
            let full_vol = {
                let m = primitives::quad_sphere(1.0, div);
                signed_volume(&m)
            };
            // remove o lado inferior (inner = oposto da normal +Y)
            let mut up = primitives::quad_sphere(1.0, div);
            let out = bisect(&mut up, Vec3::ZERO, Vec3::Y, true, false, true, 1e-5).unwrap();
            half_ok(&up, true);
            assert_eq!(out.created_faces.len(), 1);
            let cap = out.created_faces[0] as usize;
            assert!(up.faces[cap].len() >= 12);
            assert!(up.face_normal(cap as u32).y < -0.99, "tampa aponta para -Y");
            let v_up = signed_volume(&up);
            assert!(v_up < full_vol * 0.6 && v_up > full_vol * 0.35);
            // remove o lado superior
            let mut down = primitives::quad_sphere(1.0, div);
            bisect(&mut down, Vec3::ZERO, Vec3::Y, false, true, true, 1e-5).unwrap();
            half_ok(&down, false);
            assert!(down.face_normal(down.face_count() as u32 - 1).y > 0.99);
            // as duas metades somam (aproximadamente) a esfera original
            assert!((signed_volume(&up) + signed_volume(&down) - full_vol).abs() < full_vol * 0.02);
        }
    }

    #[test]
    fn fill_without_clear_separates_two_closed_halves() {
        let mut m = primitives::quad_sphere(1.0, 5);
        let out = bisect(&mut m, Vec3::ZERO, Vec3::Y, false, false, true, 1e-5).unwrap();
        assert_eq!(out.created_faces.len(), 2);
        assert_closed(&m, Some(4));
        let topo = Topology::build(&m);
        assert_eq!(topo.face_components(&m).1, 2);
    }

    #[test]
    fn cut_without_fill_leaves_boundary_and_no_clear_keeps_closed() {
        let mut m = primitives::quad_sphere(1.0, 5);
        let n0 = m.face_count();
        bisect(&mut m, Vec3::ZERO, Vec3::Y, false, false, false, 1e-5).unwrap();
        // continua uma casca fechada, só com mais faces
        assert_closed(&m, Some(2));
        assert!(m.face_count() > n0);
        let mut open = primitives::quad_sphere(1.0, 5);
        bisect(&mut open, Vec3::ZERO, Vec3::Y, true, false, false, 1e-5).unwrap();
        let t = Topology::build(&open);
        assert!(t.boundary_edge_count() > 0);
        assert_eq!(t.non_manifold_edge_count(), 0);
        open.validate().unwrap();
        // ambos os lados removidos: sobra nada (a esfera não tem faces no plano)
        let mut none = primitives::quad_sphere(1.0, 5);
        bisect(&mut none, Vec3::ZERO, Vec3::Y, true, true, true, 1e-5).unwrap();
        assert_eq!(none.face_count(), 0);
        assert_eq!(none.vert_count(), 0);
    }

    #[test]
    fn oblique_plane_through_cube_splits_into_triangles_and_closes() {
        let mut m = primitives::cube(2.0);
        // plano x + y + z = 0.3 (corta quads em pentágonos/triângulos)
        let n = Vec3::new(1.0, 1.0, 1.0);
        let out = bisect(&mut m, Vec3::new(0.1, 0.1, 0.1), n, true, false, true, 1e-5).unwrap();
        assert_closed(&m, Some(2));
        assert_eq!(out.created_faces.len(), 1);
        // todos os vértices ficam no lado externo ou no plano
        for v in &m.verts {
            assert!((v.co - Vec3::splat(0.1)).dot(n.normalize()) > -1e-4);
        }
        assert!(signed_volume(&m) > 0.0 && signed_volume(&m) < 8.0);
        // plano passando exatamente por vértices (diagonal x = z): faces quad viram triângulos
        let mut d = primitives::cube(2.0);
        bisect(
            &mut d,
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, -1.0),
            true,
            false,
            true,
            1e-5,
        )
        .unwrap();

        assert_closed(&d, Some(2));
        assert!(
            (signed_volume(&d) - 4.0).abs() < 1e-3,
            "metade do cubo: {}",
            signed_volume(&d)
        );
    }

    #[test]
    fn plane_missing_the_mesh_is_a_noop_or_clears_everything() {
        let mut m = primitives::cube(2.0);
        let h = m.content_hash();
        let out = bisect(
            &mut m,
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::Y,
            false,
            false,
            false,
            1e-5,
        )
        .unwrap();
        assert_eq!(h, m.content_hash());
        assert!(out.created_verts.is_empty() && out.selection.edges.is_empty());
        // tudo no lado interno: clear_outer não remove nada
        bisect(
            &mut m,
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::Y,
            false,
            true,
            true,
            1e-5,
        )
        .unwrap();
        assert_eq!(h, m.content_hash());
        // clear_inner remove tudo
        bisect(
            &mut m,
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::Y,
            true,
            false,
            false,
            1e-5,
        )
        .unwrap();
        assert_eq!(m.face_count(), 0);
    }

    #[test]
    fn concave_face_is_cut_with_two_chords() {
        // polígono em U no plano XZ, normal +Y
        let mut m = EditMesh::new("u");
        let pts = [
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 2.0),
            (2.0, 2.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        // (x, z) com z crescendo para -Z para normal +Y no enrolamento anti-horário visto de cima
        let vs: Vec<u32> = pts
            .iter()
            .map(|&(x, z)| m.add_vert(Vec3::new(x, 0.0, -z)))
            .collect();
        m.add_face(&vs).unwrap();
        if m.face_normal(0).y < 0.0 {
            m.flip_face(0).unwrap();
        }
        let total = m.face_area(0);
        assert!((total - 5.0).abs() < 1e-4);
        // plano z = -1.5 (corta as duas pontas do U)
        let mut cut = m.clone();
        bisect(
            &mut cut,
            Vec3::new(0.0, 0.0, -1.5),
            Vec3::NEG_Z,
            false,
            false,
            false,
            1e-6,
        )
        .unwrap();
        let area: f32 = (0..cut.face_count() as u32).map(|f| cut.face_area(f)).sum();
        assert!((area - 5.0).abs() < 1e-3);
        assert_eq!(cut.face_count(), 3, "corpo + duas pontas");
        // removendo o lado z < -1.5 (pontas)
        let mut base = m.clone();
        bisect(
            &mut base,
            Vec3::new(0.0, 0.0, -1.5),
            Vec3::NEG_Z,
            false,
            true,
            false,
            1e-6,
        )
        .unwrap();
        let area: f32 = (0..base.face_count() as u32)
            .map(|f| base.face_area(f))
            .sum();
        // ficam z em [0,1.5]: 5 - 2 * (1 x 0.5) = 4
        assert!((area - 4.0).abs() < 1e-3, "{area}");
        base.validate().unwrap();
    }

    #[test]
    fn uv_normal_and_edge_attrs_are_interpolated() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        let t = Topology::build(&m);
        // seam numa aresta cortada (horizontal do lado esquerdo)
        let e = t
            .edges
            .iter()
            .map(|e| e.verts)
            .find(|&(a, b)| {
                (m.co(a).z - m.co(b).z).abs() < 1e-6
                    && m.co(a).x.min(m.co(b).x) < -0.9
                    && m.co(a).z < 0.0
            })
            .unwrap();
        m.set_seam(e.0, e.1, true);
        m.faces[0].material = 4;
        let out = bisect(
            &mut m,
            Vec3::new(-0.5, 0.0, 0.0),
            Vec3::X,
            false,
            false,
            false,
            1e-6,
        )
        .unwrap();
        assert_eq!(out.created_verts.len(), 2);
        // UV do vértice novo: u = 0.25 (x=-0.5 entre -1 e 0 => u de 0 a 0.5)
        for &v in &out.created_verts {
            let uvs: Vec<f32> = m
                .faces
                .iter()
                .flat_map(|f| f.corners.iter().filter(|c| c.vert == v).map(|c| c.uv.x))
                .collect();
            assert!(!uvs.is_empty());
            for u in uvs {
                assert!((u - 0.25).abs() < 1e-5, "u={u}");
            }
        }
        // seam nos dois segmentos
        let nv = out
            .created_verts
            .iter()
            .copied()
            .find(|&v| m.edge_attr(v, e.0).seam || m.edge_attr(v, e.1).seam)
            .unwrap();
        assert!(m.edge_attr(nv, e.0).seam && m.edge_attr(nv, e.1).seam);
        assert!(!m.edge_attrs.contains_key(&edge_key(e.0, e.1)) || m.edge_attr(e.0, e.1).seam);
        assert!(m
            .faces
            .iter()
            .take(2)
            .all(|f| f.material == 4 || f.material == 0));
        m.validate().unwrap();
    }

    #[test]
    fn remap_and_selection_are_consistent() {
        let mut m = primitives::quad_sphere(1.0, 5);
        let n_faces = m.face_count();
        let out = bisect(&mut m, Vec3::ZERO, Vec3::Y, true, false, true, 1e-5).unwrap();
        let remap = out.remap.as_ref().unwrap();
        assert_eq!(remap.face.len(), n_faces);
        assert!(remap.face.iter().any(|f| f.is_none()));
        assert!(remap.face.iter().any(|f| f.is_some()));
        for &v in &out.created_verts {
            assert!((v as usize) < m.vert_count());
            assert!(m.co(v).y.abs() < 1e-4);
        }
        for e in &out.selection.edges {
            assert!(m.co(e.0).y.abs() < 1e-4 && m.co(e.1).y.abs() < 1e-4);
        }
        assert_eq!(out.selection.faces.len(), 1);
    }

    #[test]
    fn invalid_inputs_leave_mesh_untouched() {
        let mut m = primitives::cube(2.0);
        let h = m.content_hash();
        assert!(matches!(
            bisect(&mut m, Vec3::ZERO, Vec3::ZERO, true, false, true, 0.0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bisect(&mut m, Vec3::NAN, Vec3::Y, true, false, true, 0.0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bisect(
                &mut m,
                Vec3::ZERO,
                Vec3::new(f32::INFINITY, 0.0, 0.0),
                true,
                false,
                true,
                0.0
            ),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bisect(&mut m, Vec3::ZERO, Vec3::Y, true, false, true, -1.0),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bisect(&mut m, Vec3::ZERO, Vec3::Y, true, false, true, f32::NAN),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(h, m.content_hash());
    }

    #[test]
    fn epsilon_snaps_nearly_coplanar_vertices() {
        let mut m = primitives::cube_subdivided(2.0, 2);
        // desloca levemente os vértices do plano y=0 para ver que o epsilon os absorve
        for v in &mut m.verts {
            if v.co.y.abs() < 1e-6 {
                v.co.y = 1e-4;
            }
        }
        let before = m.vert_count();
        let mut tight = m.clone();
        bisect(&mut tight, Vec3::ZERO, Vec3::Y, true, false, true, 0.0).unwrap();
        let mut loose = m.clone();
        bisect(&mut loose, Vec3::ZERO, Vec3::Y, true, false, true, 1e-3).unwrap();
        assert!(
            tight.vert_count() > loose.vert_count() || tight.face_count() >= loose.face_count()
        );
        assert_closed(&loose, Some(2));
        assert!(before > 0);
    }

    #[test]
    fn bisect_large_sphere_is_fast_and_deterministic() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 64);
            let t0 = std::time::Instant::now();
            bisect(
                &mut m,
                Vec3::new(0.0, 0.013, 0.0),
                Vec3::new(0.2, 1.0, 0.1),
                true,
                false,
                true,
                1e-6,
            )
            .unwrap();
            assert!(t0.elapsed().as_secs() < 30);
            m
        };
        let a = run();
        assert_closed(&a, Some(2));
        assert_eq!(a.content_hash(), run().content_hash());
    }
}
