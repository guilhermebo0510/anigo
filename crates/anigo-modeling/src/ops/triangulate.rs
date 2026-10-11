//! Triangulate Faces (`Ctrl+T`) e Tris to Quads (`Alt+J`).
//!
//! Os triângulos preservam os cantos originais (UV e normal customizada por
//! corner) e os atributos de face; as diagonais novas nascem sem atributos de
//! aresta. Desmembrar/fundir não move vértices: `remap.vert` é a identidade.

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, newell_normal, Corner, EdgeKey, EditMesh, Face, Remap};
use crate::ops::delete::{face_edge_set, prune_vanished_edge_attrs, unique_faces};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::Vec3;
use std::collections::BTreeMap;

/// Como dividir quads (Blender: *Quad Method*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuadMethod {
    /// Diagonal que maximiza o menor ângulo interno dos dois triângulos
    /// (empate: a mais curta). Padrão do Blender.
    #[default]
    Beauty,
    /// Sempre a diagonal do 1º ao 3º vértice do quad.
    Fixed,
    /// Sempre a diagonal do 2º ao 4º vértice.
    FixedAlternate,
    /// A menor das duas diagonais.
    ShortestDiagonal,
    /// A maior das duas diagonais.
    LongestDiagonal,
}

/// Como dividir n-gons (5+ lados).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NgonMethod {
    /// Ear clipping seguido de otimização de diagonais por troca de arestas.
    #[default]
    Beauty,
    /// Ear clipping puro ([`crate::convert::triangulate_polygon`]).
    EarClip,
}

fn min_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let ang = |p: Vec3, q: Vec3, r: Vec3| {
        (q - p)
            .normalize_or_zero()
            .dot((r - p).normalize_or_zero())
            .clamp(-1.0, 1.0)
            .acos()
    };
    ang(a, b, c).min(ang(b, c, a)).min(ang(c, a, b))
}

/// Otimiza uma triangulação de polígono por troca de diagonais ("beautify"):
/// troca uma aresta interna quando isso aumenta o menor ângulo dos dois
/// triângulos adjacentes e mantém ambos com a orientação do polígono. `tris` são
/// índices em `pts`; o resultado permanece uma triangulação válida do mesmo
/// polígono. Determinístico (arestas visitadas em ordem de chave).
pub(crate) fn beautify_triangles(pts: &[Vec3], tris: &mut [[usize; 3]]) {
    let normal = newell_normal(pts);
    if normal == Vec3::ZERO || tris.len() < 2 {
        return;
    }
    let orient =
        |a: usize, b: usize, c: usize| (pts[b] - pts[a]).cross(pts[c] - pts[a]).dot(normal);
    let quality = |t: &[usize; 3]| min_angle(pts[t[0]], pts[t[1]], pts[t[2]]);
    for _ in 0..(tris.len() * 8 + 8) {
        // aresta (min,max) -> [(triângulo, índice do canto oposto)]
        let mut edges: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for (ti, t) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edges.entry((a.min(b), a.max(b))).or_default().push((ti, k));
            }
        }
        let mut flipped = false;
        for (_, users) in edges {
            if users.len() != 2 || users[0].0 == users[1].0 {
                continue;
            }
            let (t0, k0) = users[0];
            let (t1, k1) = users[1];
            // t0 = (p, q, r) com aresta p→q; t1 contém q→p com apex s
            let (p, q, r) = (tris[t0][k0], tris[t0][(k0 + 1) % 3], tris[t0][(k0 + 2) % 3]);
            let (q1, p1, s) = (tris[t1][k1], tris[t1][(k1 + 1) % 3], tris[t1][(k1 + 2) % 3]);
            if q1 != q || p1 != p {
                continue; // orientações incompatíveis (polígono degenerado)
            }
            let (n0, n1) = ([p, s, r], [s, q, r]);
            if orient(n0[0], n0[1], n0[2]) <= 1e-12 || orient(n1[0], n1[1], n1[2]) <= 1e-12 {
                continue; // quad côncavo: a troca inverteria um triângulo
            }
            let before = quality(&tris[t0]).min(quality(&tris[t1]));
            let after = quality(&n0).min(quality(&n1));
            if after > before + 1e-6 {
                tris[t0] = n0;
                tris[t1] = n1;
                flipped = true;
                break; // recomputa o mapa de arestas
            }
        }
        if !flipped {
            break;
        }
    }
}

/// Divide um quad (4 pontos, ordem de enrolamento) em 2 triângulos índices locais.
fn split_quad(pts: &[Vec3], method: QuadMethod) -> Vec<[usize; 3]> {
    let n = newell_normal(pts);
    let t02 = [[0, 1, 2], [0, 2, 3]];
    let t13 = [[1, 2, 3], [1, 3, 0]];
    if n == Vec3::ZERO {
        return t02.to_vec();
    }
    let valid = |ts: &[[usize; 3]; 2]| {
        ts.iter()
            .all(|t| (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]).dot(n) > 1e-12)
    };
    let (v02, v13) = (valid(&t02), valid(&t13));
    let (d02, d13) = (
        pts[0].distance_squared(pts[2]),
        pts[1].distance_squared(pts[3]),
    );
    let prefer_02 = match method {
        QuadMethod::Fixed => true,
        QuadMethod::FixedAlternate => false,
        QuadMethod::ShortestDiagonal => d02 <= d13,
        QuadMethod::LongestDiagonal => d02 >= d13,
        QuadMethod::Beauty => {
            let q = |ts: &[[usize; 3]; 2]| {
                ts.iter()
                    .map(|t| min_angle(pts[t[0]], pts[t[1]], pts[t[2]]))
                    .fold(f32::MAX, f32::min)
            };
            let (q02, q13) = (q(&t02), q(&t13));
            if (q02 - q13).abs() > 1e-6 {
                q02 > q13
            } else {
                d02 <= d13
            }
        }
    };
    match (prefer_02, v02, v13) {
        (true, true, _) | (false, true, false) => t02.to_vec(),
        (false, _, true) | (true, false, true) => t13.to_vec(),
        _ => {
            if prefer_02 {
                t02.to_vec()
            } else {
                t13.to_vec()
            }
        }
    }
}

fn identity_remap(mesh: &EditMesh, face: Vec<Option<u32>>) -> Remap {
    Remap {
        vert: (0..mesh.verts.len() as u32).map(Some).collect(),
        face,
    }
}

fn triangulate_impl(
    mesh: &mut EditMesh,
    faces: &[u32],
    quad_method: QuadMethod,
    ngon_method: Option<NgonMethod>,
) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    let mut selected = vec![false; mesh.faces.len()];
    for &f in &sel {
        selected[f as usize] = true;
    }
    let needs = sel.iter().any(|&f| {
        let n = mesh.faces[f as usize].len();
        n == 4 || (n > 4 && ngon_method.is_some())
    });
    if !needs {
        return Ok(OpOutput {
            selection: Selection::from_faces(mesh, sel.iter().copied()),
            ..Default::default()
        });
    }
    let old = std::mem::take(&mut mesh.faces);
    let mut new_faces: Vec<Face> = Vec::with_capacity(old.len() + sel.len());
    let mut map: Vec<Option<u32>> = vec![None; old.len()];
    let mut produced: Vec<u32> = Vec::new();
    let mut created: Vec<u32> = Vec::new();
    for (fi, face) in old.iter().enumerate() {
        map[fi] = Some(new_faces.len() as u32);
        let n = face.len();
        let do_split = selected[fi] && (n == 4 || (n > 4 && ngon_method.is_some()));
        if !do_split {
            if selected[fi] {
                produced.push(new_faces.len() as u32);
            }
            new_faces.push(face.clone());
            continue;
        }
        let pts: Vec<Vec3> = face
            .corners
            .iter()
            .map(|c| mesh.verts[c.vert as usize].co)
            .collect();
        let tris: Vec<[usize; 3]> = if n == 4 {
            split_quad(&pts, quad_method)
        } else {
            let mut t = triangulate_polygon(&pts);
            if ngon_method == Some(NgonMethod::Beauty) {
                beautify_triangles(&pts, &mut t);
            }
            t
        };
        for (k, t) in tris.iter().enumerate() {
            let corners: Vec<Corner> = t.iter().map(|&i| face.corners[i]).collect();
            if k > 0 {
                created.push(new_faces.len() as u32);
            }
            produced.push(new_faces.len() as u32);
            new_faces.push(Face {
                corners,
                ..face.clone()
            });
        }
    }
    mesh.faces = new_faces;
    let remap = identity_remap(mesh, map);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, produced),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    })
}

/// *Triangulate Faces*: divide as faces selecionadas em triângulos.
///
/// * quads (4 lados) seguem `quad_method`; se a diagonal pedida deixaria um
///   triângulo invertido (quad côncavo) usa-se a outra;
/// * n-gons (5+) seguem `ngon_method`; triângulos não mudam;
/// * cada triângulo mantém UV/normal customizada dos cantos de origem e os
///   atributos da face (material, suave, face set); as diagonais novas não têm
///   atributos de aresta;
/// * a ordem das faces é preservada (cada face é substituída in place pelos seus
///   triângulos): `remap.face[f]` = índice do primeiro triângulo de `f`.
///
/// Seleção devolvida: todos os triângulos resultantes das faces selecionadas;
/// `created_faces`: os triângulos "extras" (além do primeiro de cada face). Se nenhuma
/// face precisa de divisão é um no-op (`remap = None`). Erros: seleção vazia ou
/// índice inválido.
pub fn triangulate_faces(
    mesh: &mut EditMesh,
    faces: &[u32],
    quad_method: QuadMethod,
    ngon_method: NgonMethod,
) -> ModelResult<OpOutput> {
    triangulate_impl(mesh, faces, quad_method, Some(ngon_method))
}

/// Triangula **todos os quads** da malha (n-gons ficam como estão), com o método
/// pedido. Útil para exportar para formatos só de triângulos preservando n-gons
/// raros. Sem quads: no-op.
pub fn quads_to_tris_all(mesh: &mut EditMesh, quad_method: QuadMethod) -> ModelResult<OpOutput> {
    let quads: Vec<u32> = mesh
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| f.len() == 4)
        .map(|(i, _)| i as u32)
        .collect();
    if quads.is_empty() {
        return Ok(OpOutput::default());
    }
    triangulate_impl(mesh, &quads, quad_method, None)
}

/// *Tris to Quads*: une pares de triângulos adjacentes em quads.
///
/// Condições para um par (ambos triângulos em `faces`, compartilhando uma aresta
/// manifold com enrolamentos consistentes):
/// * ângulo entre as normais ≤ `max_angle` (rad, `0..=π`; tolerância ≈ 0.08°);
/// * o quad resultante é convexo;
/// * `respect_seams`/`respect_sharp`: a aresta compartilhada não é costura/sharp;
/// * `respect_materials`: os dois triângulos têm o mesmo material.
///
/// Os pares são escolhidos de forma gulosa por "quadratura" (cantos mais próximos
/// de 90° primeiro; desempate por índice de face), então o resultado é
/// determinístico. O quad ocupa o lugar do menor índice do par e herda os
/// atributos dessa face; UV/normal por corner vêm dos triângulos de origem. Atributos
/// da diagonal removida são descartados. Seleção devolvida: os quads criados.
/// Sem pares: no-op (`remap = None`). Erros: seleção vazia/índice inválido, ângulo
/// inválido.
pub fn tris_to_quads(
    mesh: &mut EditMesh,
    faces: &[u32],
    max_angle: f32,
    respect_seams: bool,
    respect_sharp: bool,
    respect_materials: bool,
) -> ModelResult<OpOutput> {
    if !max_angle.is_finite() || !(0.0..=std::f32::consts::PI + 1e-4).contains(&max_angle) {
        return Err(ModelError::InvalidParameter(format!(
            "max_angle deve estar em [0, π], recebeu {max_angle}"
        )));
    }
    let sel = unique_faces(mesh, faces, "faces")?;
    let mut selected = vec![false; mesh.faces.len()];
    for &f in &sel {
        selected[f as usize] = true;
    }
    let topo = Topology::build(mesh);
    let cos_limit = max_angle.cos() - 1e-6;
    struct Cand {
        score: f32,
        f0: u32,
        f1: u32,
        quad: [Corner; 4],
    }
    let mut cands: Vec<Cand> = Vec::new();
    for e in &topo.edges {
        if e.faces.len() != 2 {
            continue;
        }
        let (f0, f1) = (e.faces[0].min(e.faces[1]), e.faces[0].max(e.faces[1]));
        if f0 == f1 || !selected[f0 as usize] || !selected[f1 as usize] {
            continue;
        }
        let (c0, c1) = (&mesh.faces[f0 as usize], &mesh.faces[f1 as usize]);
        if c0.len() != 3 || c1.len() != 3 {
            continue;
        }
        let attr = mesh.edge_attr(e.verts.0, e.verts.1);
        if (respect_seams && attr.seam) || (respect_sharp && attr.sharp) {
            continue;
        }
        if respect_materials && c0.material != c1.material {
            continue;
        }
        if mesh.face_normal(f0).dot(mesh.face_normal(f1)) < cos_limit {
            continue;
        }
        let (a, b) = e.verts;
        // aresta dirigida x→y em f0; f1 deve percorrê-la como y→x
        let Some(i0) = (0..3).find(|&i| {
            let (u, v) = (c0.corners[i].vert, c0.corners[(i + 1) % 3].vert);
            edge_key(u, v) == (a, b)
        }) else {
            continue;
        };
        let (x, y) = (c0.corners[i0], c0.corners[(i0 + 1) % 3]);
        let apex0 = c0.corners[(i0 + 2) % 3];
        let consistent =
            (0..3).any(|i| c1.corners[i].vert == y.vert && c1.corners[(i + 1) % 3].vert == x.vert);
        if !consistent {
            continue;
        }
        let Some(apex1) = c1
            .corners
            .iter()
            .copied()
            .find(|c| c.vert != a && c.vert != b)
        else {
            continue;
        };
        if apex1.vert == apex0.vert {
            continue;
        }
        let quad = [x, apex1, y, apex0];
        let pts: Vec<Vec3> = quad.iter().map(|c| mesh.co(c.vert)).collect();
        let n = newell_normal(&pts);
        if n == Vec3::ZERO {
            continue;
        }
        let mut worst = 0.0f32;
        let mut convex = true;
        for i in 0..4 {
            let (p, c, q) = (pts[(i + 3) % 4], pts[i], pts[(i + 1) % 4]);
            if (c - p).cross(q - c).dot(n) <= 1e-10 {
                convex = false;
                break;
            }
            let ang = (p - c)
                .normalize_or_zero()
                .dot((q - c).normalize_or_zero())
                .clamp(-1.0, 1.0)
                .acos();
            worst = worst.max((ang - std::f32::consts::FRAC_PI_2).abs());
        }
        if convex {
            cands.push(Cand {
                score: worst,
                f0,
                f1,
                quad,
            });
        }
    }
    cands.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then(a.f0.cmp(&b.f0))
            .then(a.f1.cmp(&b.f1))
    });
    let mut used = vec![false; mesh.faces.len()];
    let mut replace: BTreeMap<u32, ([Corner; 4], u32)> = BTreeMap::new(); // f0 -> (quad, f1)
    let mut absorbed = vec![false; mesh.faces.len()];
    for c in cands {
        if used[c.f0 as usize] || used[c.f1 as usize] {
            continue;
        }
        used[c.f0 as usize] = true;
        used[c.f1 as usize] = true;
        absorbed[c.f1 as usize] = true;
        replace.insert(c.f0, (c.quad, c.f1));
    }
    if replace.is_empty() {
        return Ok(OpOutput::default());
    }
    let old_face_edges = face_edge_set(mesh);
    let old = std::mem::take(&mut mesh.faces);
    let mut map: Vec<Option<u32>> = vec![None; old.len()];
    let mut created: Vec<u32> = Vec::new();
    // f1 pode vir antes de f0? f0 = min do par, então o quad já foi emitido
    for (fi, face) in old.iter().enumerate() {
        if absorbed[fi] {
            continue;
        }
        let idx = mesh.faces.len() as u32;
        map[fi] = Some(idx);
        if let Some((quad, f1)) = replace.get(&(fi as u32)) {
            map[*f1 as usize] = Some(idx);
            created.push(idx);
            mesh.faces.push(Face {
                corners: quad.to_vec(),
                ..face.clone()
            });
        } else {
            mesh.faces.push(face.clone());
        }
    }
    prune_vanished_edge_attrs(mesh, &old_face_edges);
    let remap = identity_remap(mesh, map);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, created.iter().copied()),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    })
}

/// Atalho: arestas das diagonais criadas por uma triangulação — útil para a UI
/// selecionar/ocultar "arestas de triangulação". Retorna as arestas que têm 2 faces
/// triangulares coplanares (até `max_angle`).
pub fn triangulation_diagonals(mesh: &EditMesh, max_angle: f32) -> Vec<EdgeKey> {
    let topo = Topology::build(mesh);
    let cos_limit = max_angle.cos() - 1e-6;
    topo.edges
        .iter()
        .filter(|e| {
            e.faces.len() == 2
                && e.faces[0] != e.faces[1]
                && mesh.faces[e.faces[0] as usize].len() == 3
                && mesh.faces[e.faces[1] as usize].len() == 3
                && mesh
                    .face_normal(e.faces[0])
                    .dot(mesh.face_normal(e.faces[1]))
                    >= cos_limit
        })
        .map(|e| e.verts)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::primitives::assert_outward;

    fn total_area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    fn all(m: &EditMesh) -> Vec<u32> {
        (0..m.face_count() as u32).collect()
    }

    #[test]
    fn triangulate_cube_every_method() {
        for qm in [
            QuadMethod::Beauty,
            QuadMethod::Fixed,
            QuadMethod::FixedAlternate,
            QuadMethod::ShortestDiagonal,
            QuadMethod::LongestDiagonal,
        ] {
            let mut m = primitives::cube(1.0);
            let area = total_area(&m);
            let ids = all(&m);
            let out = triangulate_faces(&mut m, &ids, qm, NgonMethod::Beauty).unwrap();
            m.validate().unwrap();
            assert_eq!(m.face_count(), 12);
            assert!(m.faces.iter().all(|f| f.len() == 3));
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold());
            assert_eq!(t.euler_characteristic(&m), 2);
            assert!((total_area(&m) - area).abs() < 1e-5);
            assert_outward(&m);
            assert_eq!(out.selection.faces.len(), 12);
            assert_eq!(out.created_faces.len(), 6);
            let r = out.remap.unwrap();
            assert_eq!(r.face[0], Some(0));
            assert_eq!(r.face[1], Some(2));
        }
    }

    #[test]
    fn fixed_methods_choose_expected_diagonals() {
        let mk = || {
            let mut m = EditMesh::new("q");
            for p in [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y] {
                m.add_vert(p);
            }
            m.add_face(&[0, 1, 2, 3]).unwrap();
            m
        };
        let diag_of = |m: &EditMesh| {
            let t = Topology::build(m);
            t.edges.iter().find(|e| e.faces.len() == 2).unwrap().verts
        };
        let mut a = mk();
        triangulate_faces(&mut a, &[0], QuadMethod::Fixed, NgonMethod::EarClip).unwrap();
        assert_eq!(diag_of(&a), (0, 2));
        let mut b = mk();
        triangulate_faces(
            &mut b,
            &[0],
            QuadMethod::FixedAlternate,
            NgonMethod::EarClip,
        )
        .unwrap();
        assert_eq!(diag_of(&b), (1, 3));
    }

    #[test]
    fn shortest_and_longest_diagonal_on_a_kite() {
        // diagonal 0-2 curta (1.0), 1-3 longa (2.0): losango alongado convexo
        let mk = || {
            let mut m = EditMesh::new("kite");
            for p in [
                Vec3::new(-0.5, 0.0, 0.0),
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.5, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ] {
                m.add_vert(p);
            }
            // sentido anti-horário visto de +Y é (0,3,2,1)
            m.add_face(&[0, 3, 2, 1]).unwrap();
            m
        };
        let diag = |m: &EditMesh| {
            Topology::build(m)
                .edges
                .iter()
                .find(|e| e.faces.len() == 2)
                .unwrap()
                .verts
        };
        let mut s = mk();
        triangulate_faces(
            &mut s,
            &[0],
            QuadMethod::ShortestDiagonal,
            NgonMethod::EarClip,
        )
        .unwrap();
        assert_eq!(diag(&s), (0, 2));
        let mut l = mk();
        triangulate_faces(
            &mut l,
            &[0],
            QuadMethod::LongestDiagonal,
            NgonMethod::EarClip,
        )
        .unwrap();
        assert_eq!(diag(&l), (1, 3));
        // beauty também prefere a curta (maximiza o menor ângulo)
        let mut b = mk();
        triangulate_faces(&mut b, &[0], QuadMethod::Beauty, NgonMethod::EarClip).unwrap();
        assert_eq!(diag(&b), (0, 2));
    }

    #[test]
    fn concave_quad_falls_back_to_valid_diagonal() {
        // seta côncava: vértice 2 empurrado para dentro
        let mut m = EditMesh::new("dart");
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.5, 0.0, 0.5),
            Vec3::new(0.0, 0.0, 2.0),
        ] {
            m.add_vert(p);
        }
        m.add_face(&[0, 3, 2, 1]).unwrap();
        let area = total_area(&m);
        for qm in [
            QuadMethod::Fixed,
            QuadMethod::FixedAlternate,
            QuadMethod::ShortestDiagonal,
            QuadMethod::LongestDiagonal,
            QuadMethod::Beauty,
        ] {
            let mut c = m.clone();
            triangulate_faces(&mut c, &[0], qm, NgonMethod::EarClip).unwrap();
            assert_eq!(c.face_count(), 2);
            assert!((total_area(&c) - area).abs() < 1e-5, "{qm:?}");
            for f in 0..2 {
                assert!(c.face_normal(f).y > 0.9, "{qm:?} gerou triângulo invertido");
            }
        }
    }

    #[test]
    fn triangulate_ngons_both_methods() {
        for nm in [NgonMethod::Beauty, NgonMethod::EarClip] {
            let mut m = primitives::circle(1.0, 9, false);
            let area = total_area(&m);
            let out = triangulate_faces(&mut m, &[0], QuadMethod::Beauty, nm).unwrap();
            assert_eq!(m.face_count(), 7);
            assert!((total_area(&m) - area).abs() < 1e-5);
            assert_eq!(out.created_faces.len(), 6);
            for f in 0..7 {
                assert!(m.face_normal(f).y > 0.99);
            }
            m.validate().unwrap();
        }
    }

    #[test]
    fn triangulate_concave_ngon_beauty_keeps_valid_triangles() {
        // "L" côncavo de 6 vértices
        let mut m = EditMesh::new("L");
        let pts = [
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        // anti-horário visto de +Y: z decrescente quando x cresce => usa (x, -z)
        for (x, z) in pts {
            m.add_vert(Vec3::new(x, 0.0, -z));
        }
        m.add_face(&[0, 1, 2, 3, 4, 5]).unwrap();
        let area = total_area(&m);
        triangulate_faces(&mut m, &[0], QuadMethod::Beauty, NgonMethod::Beauty).unwrap();
        assert_eq!(m.face_count(), 4);
        assert!((total_area(&m) - area).abs() < 1e-5);
        for f in 0..4 {
            assert!(m.face_normal(f).y > 0.99, "triângulo {f} invertido");
        }
    }

    #[test]
    fn triangulate_only_selected_and_keeps_triangles() {
        let mut m = primitives::cube(1.0);
        m.append(&primitives::circle(1.0, 3, false));
        let before = m.face_count();
        let out =
            triangulate_faces(&mut m, &[0, 6], QuadMethod::Beauty, NgonMethod::Beauty).unwrap();
        assert_eq!(m.face_count(), before + 1);
        assert_eq!(out.selection.faces.len(), 3);
        // só triângulos: no-op
        let mut tri = primitives::circle(1.0, 3, false);
        let h = tri.content_hash();
        let out =
            triangulate_faces(&mut tri, &[0], QuadMethod::Beauty, NgonMethod::Beauty).unwrap();
        assert_eq!(tri.content_hash(), h);
        assert!(out.remap.is_none());
    }

    #[test]
    fn triangulate_preserves_corner_attributes() {
        let mut m = primitives::plane(1.0);
        for (i, c) in m.faces[0].corners.iter_mut().enumerate() {
            c.uv = glam::Vec2::new(i as f32, 1.0);
            c.normal = Some(Vec3::Y);
        }
        m.faces[0].material = 5;
        m.faces[0].face_set = 3;
        triangulate_faces(&mut m, &[0], QuadMethod::Fixed, NgonMethod::Beauty).unwrap();
        assert_eq!(m.face_count(), 2);
        for f in &m.faces {
            assert_eq!(f.material, 5);
            assert_eq!(f.face_set, 3);
            assert!(f.corners.iter().all(|c| c.normal == Some(Vec3::Y)));
        }
        let ids = [m.faces[0].corners[0].vert, m.faces[1].corners[0].vert];
        assert_eq!(
            ids[0], ids[1],
            "Fixed divide em 0-2: ambos triângulos começam em 0"
        );
    }

    #[test]
    fn triangulate_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            triangulate_faces(&mut m, &[], QuadMethod::Beauty, NgonMethod::Beauty),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            triangulate_faces(&mut m, &[0, 40], QuadMethod::Beauty, NgonMethod::Beauty),
            Err(ModelError::FaceOutOfRange(40, 6))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn quads_to_tris_all_leaves_ngons() {
        let mut m = primitives::cube(1.0);
        m.append(&primitives::circle(1.0, 7, false));
        quads_to_tris_all(&mut m, QuadMethod::Beauty).unwrap();
        assert_eq!(m.face_count(), 12 + 1);
        assert!(m.faces.iter().any(|f| f.len() == 7));
        // sem quads: no-op
        let mut c = primitives::circle(1.0, 7, false);
        let h = c.content_hash();
        assert!(quads_to_tris_all(&mut c, QuadMethod::Fixed)
            .unwrap()
            .remap
            .is_none());
        assert_eq!(c.content_hash(), h);
    }

    #[test]
    fn tris_to_quads_roundtrip_on_organic_meshes() {
        for m0 in [
            primitives::cube_subdivided(2.0, 3),
            primitives::quad_sphere(1.0, 4),
            primitives::capsule(0.5, 1.0, 8, 3),
            primitives::grid(5, 4, 5.0, 4.0),
        ] {
            for qm in [
                QuadMethod::Fixed,
                QuadMethod::Beauty,
                QuadMethod::LongestDiagonal,
            ] {
                let mut m = m0.clone();
                {
                    let ids = all(&m);
                    triangulate_faces(&mut m, &ids, qm, NgonMethod::Beauty).unwrap();
                }
                let tris = all(&m);
                let out = tris_to_quads(&mut m, &tris, 0.2, true, true, true).unwrap();
                m.validate().unwrap();
                let quads = m.faces.iter().filter(|f| f.len() == 4).count();
                assert!(
                    quads >= m0.faces.iter().filter(|f| f.len() == 4).count() * 9 / 10,
                    "{} {qm:?}: {quads}",
                    m0.name
                );
                assert!((total_area(&m) - total_area(&m0)).abs() / total_area(&m0) < 1e-3);
                if Topology::build(&m0).is_closed_manifold() {
                    assert!(Topology::build(&m).is_closed_manifold(), "{}", m0.name);
                }
                assert!(!out.created_faces.is_empty());
            }
        }
    }

    #[test]
    fn tris_to_quads_flat_grid_pairs_perfectly() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        {
            let ids = all(&m);
            triangulate_faces(
                &mut m,
                &ids,
                QuadMethod::ShortestDiagonal,
                NgonMethod::EarClip,
            )
            .unwrap();
        }
        let tris = all(&m);
        let out = tris_to_quads(&mut m, &tris, 0.01, false, false, false).unwrap();
        assert_eq!(m.face_count(), 16);
        assert_eq!(out.created_faces.len(), 16);
        assert!(m.faces.iter().all(|f| f.len() == 4));
        let r = out.remap.unwrap();
        assert!(r.face.iter().all(|f| f.is_some()));
        assert!(m.edge_attrs.is_empty());
    }

    #[test]
    fn tris_to_quads_delimiters() {
        let base = {
            let mut m = primitives::plane(1.0);
            triangulate_faces(&mut m, &[0], QuadMethod::Fixed, NgonMethod::EarClip).unwrap();
            m
        };
        let diag = Topology::build(&base)
            .edges
            .iter()
            .find(|e| e.faces.len() == 2)
            .unwrap()
            .verts;
        // sem restrições: junta
        let mut a = base.clone();
        assert_eq!(
            tris_to_quads(&mut a, &[0, 1], 0.1, true, true, true)
                .unwrap()
                .created_faces
                .len(),
            1
        );
        // seam
        let mut s = base.clone();
        s.set_seam(diag.0, diag.1, true);
        assert!(tris_to_quads(&mut s, &[0, 1], 0.1, true, false, false)
            .unwrap()
            .created_faces
            .is_empty());
        assert_eq!(s.face_count(), 2);
        let mut s2 = base.clone();
        s2.set_seam(diag.0, diag.1, true);
        tris_to_quads(&mut s2, &[0, 1], 0.1, false, false, false).unwrap();
        assert_eq!(s2.face_count(), 1);
        assert!(
            s2.edge_attrs.is_empty(),
            "atributo da diagonal removida some"
        );
        // sharp
        let mut h = base.clone();
        h.set_sharp(diag.0, diag.1, true);
        assert!(tris_to_quads(&mut h, &[0, 1], 0.1, false, true, false)
            .unwrap()
            .created_faces
            .is_empty());
        // material
        let mut mt = base.clone();
        mt.faces[1].material = 2;
        assert!(tris_to_quads(&mut mt, &[0, 1], 0.1, false, false, true)
            .unwrap()
            .created_faces
            .is_empty());
        assert_eq!(
            tris_to_quads(&mut mt, &[0, 1], 0.1, false, false, false)
                .unwrap()
                .created_faces
                .len(),
            1
        );
        // faces não selecionadas não são tocadas
        let mut one = base.clone();
        assert!(tris_to_quads(&mut one, &[0], 0.1, false, false, false)
            .unwrap()
            .created_faces
            .is_empty());
    }

    #[test]
    fn tris_to_quads_respects_angle_and_convexity() {
        // dobra de 90° (dois triângulos formando um telhado): max_angle pequeno impede
        let mut m = EditMesh::new("roof");
        for p in [
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(0.0, 1.0, 1.0),
            Vec3::new(1.0, 1.0, 0.0),
        ] {
            m.add_vert(p);
        }
        m.add_face(&[0, 1, 3]).unwrap();
        m.add_face(&[0, 3, 2]).unwrap();
        let mut a = m.clone();
        assert!(tris_to_quads(&mut a, &[0, 1], 0.1, false, false, false)
            .unwrap()
            .created_faces
            .is_empty());
        // com o limite máximo, o telhado dobrado (convexo) pode ser unido
        let mut b = m.clone();
        assert_eq!(
            tris_to_quads(&mut b, &[0, 1], std::f32::consts::PI, false, false, false)
                .unwrap()
                .created_faces
                .len(),
            1
        );
        // dois triângulos coplanares formando um quad côncavo (seta): não junta
        let mut dart = EditMesh::new("dart");
        for p in [
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.5, 0.0, -0.5),
            Vec3::new(0.0, 0.0, -2.0),
        ] {
            dart.add_vert(p);
        }
        dart.add_face(&[0, 1, 2]).unwrap();
        dart.add_face(&[0, 2, 3]).unwrap();
        assert!(tris_to_quads(&mut dart, &[0, 1], 0.1, false, false, false)
            .unwrap()
            .created_faces
            .is_empty());
    }

    #[test]
    fn tris_to_quads_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(tris_to_quads(&mut m, &[], 0.1, false, false, false).is_err());
        assert!(tris_to_quads(&mut m, &[0], -1.0, false, false, false).is_err());
        assert!(tris_to_quads(&mut m, &[0], f32::NAN, false, false, false).is_err());
        assert!(tris_to_quads(&mut m, &[99], 0.1, false, false, false).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn tris_to_quads_skips_inconsistent_winding() {
        let mut m = primitives::plane(1.0);
        triangulate_faces(&mut m, &[0], QuadMethod::Fixed, NgonMethod::EarClip).unwrap();
        m.flip_face(1).unwrap();
        let out = tris_to_quads(&mut m, &[0, 1], 0.1, false, false, false).unwrap();
        assert!(out.created_faces.is_empty());
    }

    #[test]
    fn beautify_improves_min_angle_and_keeps_area() {
        // retângulo largo triangulado com a diagonal "errada" não existe (qualquer diagonal
        // de retângulo tem o mesmo mínimo), então usa um pentágono irregular
        let pts = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, -1.0),
            Vec3::new(2.0, 0.0, -1.2),
            Vec3::new(0.0, 0.0, -1.0),
        ];
        let mut ear = triangulate_polygon(&pts);
        let q = |t: &[[usize; 3]]| {
            t.iter()
                .map(|t| min_angle(pts[t[0]], pts[t[1]], pts[t[2]]))
                .fold(f32::MAX, f32::min)
        };
        let before = q(&ear);
        beautify_triangles(&pts, &mut ear);
        assert!(q(&ear) >= before - 1e-6);
        assert_eq!(ear.len(), 3);
        let area: f32 = ear
            .iter()
            .map(|t| {
                (pts[t[1]] - pts[t[0]])
                    .cross(pts[t[2]] - pts[t[0]])
                    .length()
                    * 0.5
            })
            .sum();
        let poly = crate::mesh::polygon_area(&pts);
        assert!((area - poly).abs() < 1e-5);
    }

    #[test]
    fn triangulation_is_deterministic() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 5);
            {
                let ids = all(&m);
                triangulate_faces(&mut m, &ids, QuadMethod::Beauty, NgonMethod::Beauty).unwrap();
            }
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn diagonals_helper_finds_triangulation_edges() {
        let mut m = primitives::cube(1.0);
        {
            let ids = all(&m);
            triangulate_faces(&mut m, &ids, QuadMethod::Fixed, NgonMethod::EarClip).unwrap();
        }
        assert_eq!(triangulation_diagonals(&m, 0.01).len(), 6);
    }

    #[test]
    fn triangulate_and_requad_large_mesh_is_fast() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let t0 = std::time::Instant::now();
        {
            let ids = all(&m);
            triangulate_faces(&mut m, &ids, QuadMethod::Beauty, NgonMethod::Beauty).unwrap();
        }
        assert_eq!(m.face_count(), 2 * 6 * 64 * 64);
        let tris = all(&m);
        tris_to_quads(&mut m, &tris, 0.3, false, false, false).unwrap();
        assert!(t0.elapsed().as_secs() < 60, "{:?}", t0.elapsed());
        assert!(Topology::build(&m).is_closed_manifold());
    }
}
