//! Fill (`F`), Triangle Fill, Grid Fill e Fill All Holes.
//!
//! Todos partem de *loops fechados* de arestas de fronteira (1 face) ou wire
//! (0 faces). O loop é ordenado e **orientado** para que a face nova seja
//! consistente com a vizinhança: cada aresta de fronteira `a→b` de uma face
//! existente exige que a face nova a percorra como `b→a`. Loops só de arestas wire
//! não têm referência de orientação e usam a ordem determinística de travessia
//! (começa pelo menor índice de vértice, seguindo o menor vizinho).
//!
//! Atributos da face nova (material, suavização, face set) vêm da face adjacente à
//! primeira aresta de fronteira; UV e normais por corner ficam zerados/automáticos
//! (não há dado confiável para interpolar — use as ferramentas de UV depois).

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EVert, EdgeKey, EditMesh, Face};
use crate::ops::delete::unique_edges;
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};

/// Loop fechado já ordenado/orientado para a face nova.
#[derive(Debug, Clone)]
pub(crate) struct HoleLoop {
    /// Vértices na ordem em que a face nova deve percorrê-los.
    pub verts: Vec<u32>,
    /// Face adjacente da qual herdar atributos.
    pub face_hint: Option<u32>,
}

/// Ordena as arestas em loops fechados e os orienta (ver doc do módulo).
///
/// Erros (`InvalidParameter`): aresta com 2+ faces (não é de fronteira), vértice
/// com grau ≠ 2 na seleção (loop aberto ou ramificado). `UnsupportedTopology`:
/// arestas de fronteira do mesmo loop com direções incompatíveis.
pub(crate) fn ordered_loops(
    mesh: &EditMesh,
    topo: &Topology,
    edges: &[EdgeKey],
) -> ModelResult<Vec<HoleLoop>> {
    let mut adj: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(a, b) in edges {
        let nf = topo.edge_faces(a, b).len();
        if nf > 1 {
            return Err(ModelError::InvalidParameter(format!(
                "aresta ({a}, {b}) tem {nf} faces: só arestas de fronteira ou wire podem ser preenchidas"
            )));
        }
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    for (v, n) in &adj {
        if n.len() != 2 {
            return Err(ModelError::InvalidParameter(format!(
                "a seleção não forma loops fechados: o vértice {v} tem {} aresta(s) selecionada(s) (esperado 2)",
                n.len()
            )));
        }
    }
    for n in adj.values_mut() {
        n.sort_unstable();
    }
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    let mut loops = Vec::new();
    for &start in adj.keys() {
        if visited.contains(&start) {
            continue;
        }
        let mut verts = vec![start];
        visited.insert(start);
        let mut prev = start;
        let mut cur = adj[&start][0];
        while cur != start {
            verts.push(cur);
            visited.insert(cur);
            let nb = &adj[&cur];
            let nxt = if nb[0] != prev { nb[0] } else { nb[1] };
            prev = cur;
            cur = nxt;
        }
        // orientação
        let n = verts.len();
        let (mut agree, mut disagree) = (0usize, 0usize);
        let mut hint = None;
        for i in 0..n {
            let (a, b) = (verts[i], verts[(i + 1) % n]);
            if let Some(&f) = topo.edge_faces(a, b).first() {
                hint.get_or_insert(f);
                let same = mesh.faces[f as usize]
                    .edges()
                    .any(|(x, y)| x == a && y == b);
                if same {
                    agree += 1; // a travessia segue a face existente: a nova deve ir ao contrário
                } else {
                    disagree += 1;
                }
            }
        }
        if agree > 0 && disagree > 0 {
            return Err(ModelError::UnsupportedTopology(
                "as faces ao redor do loop têm enrolamentos inconsistentes (rode Make Consistent antes)".into(),
            ));
        }
        if agree > 0 {
            verts.reverse();
        }
        loops.push(HoleLoop {
            verts,
            face_hint: hint,
        });
    }
    Ok(loops)
}

fn new_face_like(mesh: &EditMesh, hint: Option<u32>, verts: &[u32]) -> Face {
    match hint {
        Some(f) => mesh.faces[f as usize].with_verts_from(verts),
        None => Face::new(verts),
    }
}

fn output(mesh: &EditMesh, created_faces: Vec<u32>, created_verts: Vec<u32>) -> OpOutput {
    let mut selection = Selection::from_faces(mesh, created_faces.iter().copied());
    selection.verts.extend(created_verts.iter().copied());
    OpOutput {
        selection,
        created_verts,
        created_faces,
        remap: None,
    }
}

/// *Fill* (`F`): cria um n-gon por loop fechado de arestas de fronteira/wire.
///
/// `edges` deve formar um ou mais loops fechados disjuntos (cada vértice com
/// exatamente 2 arestas na seleção). Cada n-gon é orientado de forma consistente
/// com as faces vizinhas. Seleção devolvida: as faces criadas.
///
/// Erros (malha intacta): seleção vazia, aresta inexistente, aresta com 2+ faces,
/// loop aberto/ramificado, enrolamento inconsistente na vizinhança.
pub fn fill_hole(mesh: &mut EditMesh, edges: &[EdgeKey]) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    let loops = ordered_loops(mesh, &topo, &edges)?;
    let mut new_faces = Vec::with_capacity(loops.len());
    for l in &loops {
        new_faces.push(new_face_like(mesh, l.face_hint, &l.verts));
    }
    let mut created = Vec::new();
    for f in new_faces {
        created.push(mesh.faces.len() as u32);
        mesh.faces.push(f);
    }
    Ok(output(mesh, created, Vec::new()))
}

/// Variante de [`fill_hole`] que recebe vértices: usa as arestas de fronteira/wire
/// cujos dois extremos estão na lista.
pub fn fill_hole_verts(mesh: &mut EditMesh, verts: &[u32]) -> ModelResult<OpOutput> {
    let verts = crate::ops::delete::unique_verts(mesh, verts, "vértices")?;
    let set: BTreeSet<u32> = verts.iter().copied().collect();
    let topo = Topology::build(mesh);
    let edges: Vec<EdgeKey> = topo
        .edges
        .iter()
        .filter(|e| e.faces.len() <= 1 && set.contains(&e.verts.0) && set.contains(&e.verts.1))
        .map(|e| e.verts)
        .collect();
    if edges.is_empty() {
        return Err(ModelError::EmptySelection(
            "arestas de fronteira entre os vértices selecionados",
        ));
    }
    fill_hole(mesh, &edges)
}

/// *Triangle Fill*: preenche cada loop com triângulos (ear clipping no plano de
/// Newell do loop). Com `beauty`, os diagonais são otimizados por troca de
/// arestas (maximiza o menor ângulo, ver `triangulate::beautify_triangles`).
/// Mesmas validações e orientação de [`fill_hole`]. Seleção devolvida: os triângulos.
pub fn fill_triangles(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    beauty: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    let loops = ordered_loops(mesh, &topo, &edges)?;
    let mut new_faces = Vec::new();
    for l in &loops {
        let pts: Vec<Vec3> = l.verts.iter().map(|&v| mesh.co(v)).collect();
        let mut tris = triangulate_polygon(&pts);
        if beauty {
            crate::ops::triangulate::beautify_triangles(&pts, &mut tris);
        }
        for t in tris {
            let vs = [l.verts[t[0]], l.verts[t[1]], l.verts[t[2]]];
            new_faces.push(new_face_like(mesh, l.face_hint, &vs));
        }
    }
    let mut created = Vec::new();
    for f in new_faces {
        created.push(mesh.faces.len() as u32);
        mesh.faces.push(f);
    }
    Ok(output(mesh, created, Vec::new()))
}

/// *Fill Holes*: preenche com n-gons todos os buracos (loops de fronteira) com no
/// máximo `max_sides` arestas (`usize::MAX` = todos; mínimo 3).
///
/// Buracos que se tocam num vértice (fronteira "pinçada") são ambíguos e
/// ignorados. Arestas wire não contam como buraco. Se não há nada para preencher o
/// resultado é um no-op com seleção vazia. Seleção devolvida: as faces criadas.
pub fn fill_all_holes(mesh: &mut EditMesh, max_sides: usize) -> ModelResult<OpOutput> {
    if max_sides < 3 {
        return Err(ModelError::InvalidParameter(format!(
            "max_sides deve ser >= 3, recebeu {max_sides}"
        )));
    }
    let topo = Topology::build(mesh);
    // sucessor na travessia da face nova: para aresta de face a→b, a nova vai b→a
    let mut succ: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut hint: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for e in &topo.edges {
        if e.faces.len() != 1 {
            continue;
        }
        let f = e.faces[0];
        for (a, b) in mesh.faces[f as usize].edges() {
            if edge_key(a, b) == e.verts {
                succ.entry(b).or_default().push(a);
                hint.insert((b, a), f);
            }
        }
    }
    let ambiguous: BTreeSet<u32> = succ
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, _)| *k)
        .collect();
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    let mut new_faces: Vec<Face> = Vec::new();
    for &start in succ.keys() {
        if visited.contains(&start) || ambiguous.contains(&start) {
            continue;
        }
        let mut verts = vec![start];
        let mut cur = succ[&start][0];
        let mut ok = true;
        while cur != start {
            if ambiguous.contains(&cur) || verts.len() > max_sides {
                ok = false;
                break;
            }
            let Some(nx) = succ.get(&cur) else {
                ok = false;
                break;
            };
            verts.push(cur);
            cur = nx[0];
        }
        for &v in &verts {
            visited.insert(v);
        }
        if !ok || verts.len() < 3 || verts.len() > max_sides {
            continue;
        }
        let face_hint = hint.get(&(verts[0], verts[1 % verts.len()])).copied();
        new_faces.push(new_face_like(mesh, face_hint, &verts));
    }
    let mut created = Vec::new();
    for f in new_faces {
        created.push(mesh.faces.len() as u32);
        mesh.faces.push(f);
    }
    Ok(output(mesh, created, Vec::new()))
}

// ---------------------------------------------------------------------------- grid fill

/// Ponto da superfície de Coons em `(i, j)` de uma grade `a × b` cujas bordas são
/// `bottom` (j=0, `a+1` pontos), `top` (j=b), `left` (i=0, `b+1` pontos) e `right`
/// (i=a). Reproduz exatamente as bordas e, para bordas retas, o bilinear.
pub(crate) fn coons_point(
    bottom: &[Vec3],
    top: &[Vec3],
    left: &[Vec3],
    right: &[Vec3],
    i: usize,
    j: usize,
) -> Vec3 {
    let a = bottom.len() - 1;
    let b = left.len() - 1;
    let (u, v) = (i as f32 / a as f32, j as f32 / b as f32);
    let (p00, p10, p01, p11) = (bottom[0], bottom[a], top[0], top[a]);
    bottom[i] * (1.0 - v) + top[i] * v + left[j] * (1.0 - u) + right[j] * u
        - (p00 * ((1.0 - u) * (1.0 - v))
            + p10 * (u * (1.0 - v))
            + p01 * ((1.0 - u) * v)
            + p11 * (u * v))
}

fn bilinear_point(p00: Vec3, p10: Vec3, p01: Vec3, p11: Vec3, u: f32, v: f32) -> Vec3 {
    p00 * ((1.0 - u) * (1.0 - v)) + p10 * (u * (1.0 - v)) + p01 * ((1.0 - u) * v) + p11 * (u * v)
}

/// *Grid Fill*: preenche **um** loop fechado de `N` arestas (N par, ≥ 4) com uma
/// grade de quads.
///
/// O loop é dividido em 4 lados por 4 cantos escolhidos automaticamente:
/// lados opostos têm o mesmo número de arestas, `a` (lados 0 e 2) e `b = N/2 − a`
/// (lados 1 e 3). Resultado: `a × b` quads, com `(a−1)(b−1)` vértices novos no
/// interior.
///
/// * `span`: `a` desejado (`0` = automático, `N/4`, o mais quadrado possível). Deve
///   estar em `1..=N/2−1`;
/// * `offset`: gira o ponto de partida do primeiro canto (`offset % N`);
/// * `use_interp_simple`: `false` (padrão) posiciona o interior por superfície de
///   Coons, que segue a curvatura do contorno; `true` usa interpolação bilinear
///   simples dos 4 cantos (interior plano).
///
/// Exemplos: círculo de 8 arestas → 2×2 = 4 quads e 1 vértice central; 12 arestas →
/// 3×3 = 9 quads e 4 vértices; 6 arestas → 1×2 = 2 quads, sem vértices novos; 4
/// arestas → 1 quad. Atributos dos vértices novos: interpolação bilinear dos
/// atributos dos 4 cantos. Seleção: as faces criadas e os vértices novos.
///
/// Erros (malha intacta): mais de um loop, `N` ímpar ou < 4, `span` fora do
/// intervalo, e os de [`fill_hole`].
pub fn grid_fill(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    span: usize,
    offset: usize,
    use_interp_simple: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    let loops = ordered_loops(mesh, &topo, &edges)?;
    if loops.len() != 1 {
        return Err(ModelError::InvalidParameter(format!(
            "grid_fill espera exatamente 1 loop, recebeu {}",
            loops.len()
        )));
    }
    let lp = &loops[0];
    let n = lp.verts.len();
    if n < 4 || n % 2 != 0 {
        return Err(ModelError::InvalidParameter(format!(
            "grid_fill exige um loop com número par de arestas (>= 4); este tem {n}"
        )));
    }
    let a = if span == 0 { (n / 4).max(1) } else { span };
    if a < 1 || a > n / 2 - 1 {
        return Err(ModelError::InvalidParameter(format!(
            "span {a} fora do intervalo 1..={} para um loop de {n} arestas",
            n / 2 - 1
        )));
    }
    let b = n / 2 - a;
    let r: Vec<u32> = (0..n).map(|k| lp.verts[(k + offset) % n]).collect();
    let rr = |k: usize| r[k % n];
    let bottom_ids: Vec<u32> = (0..=a).map(rr).collect();
    let right_ids: Vec<u32> = (0..=b).map(|j| rr(a + j)).collect();
    let top_ids: Vec<u32> = (0..=a).map(|i| rr(a + b + (a - i))).collect();
    let left_ids: Vec<u32> = (0..=b).map(|j| rr(n - j)).collect();
    let pos = |ids: &[u32]| -> Vec<Vec3> { ids.iter().map(|&v| mesh.co(v)).collect() };
    let (pb, pt, pl, pr) = (
        pos(&bottom_ids),
        pos(&top_ids),
        pos(&left_ids),
        pos(&right_ids),
    );

    // Atributos dos 4 cantos para os vértices novos.
    let c00 = mesh.verts[bottom_ids[0] as usize].clone();
    let c10 = mesh.verts[bottom_ids[a] as usize].clone();
    let c01 = mesh.verts[top_ids[0] as usize].clone();
    let c11 = mesh.verts[top_ids[a] as usize].clone();

    let mut grid: Vec<Vec<u32>> = vec![vec![0; b + 1]; a + 1];
    let mut created_verts = Vec::new();
    for (i, col) in grid.iter_mut().enumerate() {
        for (j, slot) in col.iter_mut().enumerate() {
            *slot = if j == 0 {
                bottom_ids[i]
            } else if j == b {
                top_ids[i]
            } else if i == 0 {
                left_ids[j]
            } else if i == a {
                right_ids[j]
            } else {
                let (u, v) = (i as f32 / a as f32, j as f32 / b as f32);
                let co = if use_interp_simple {
                    bilinear_point(pb[0], pb[a], pt[0], pt[a], u, v)
                } else {
                    coons_point(&pb, &pt, &pl, &pr, i, j)
                };
                let low = EVert::lerp(&c00, &c10, u);
                let high = EVert::lerp(&c01, &c11, u);
                let mut vert = EVert::lerp(&low, &high, v);
                vert.co = co;
                let id = mesh.push_vert(vert);
                created_verts.push(id);
                id
            };
        }
    }
    let mut created_faces = Vec::new();
    for i in 0..a {
        for j in 0..b {
            let vs = [
                grid[i][j],
                grid[i + 1][j],
                grid[i + 1][j + 1],
                grid[i][j + 1],
            ];
            let f = new_face_like(mesh, lp.face_hint, &vs);
            created_faces.push(mesh.faces.len() as u32);
            mesh.faces.push(f);
        }
    }
    Ok(output(mesh, created_faces, created_verts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::primitives::assert_outward;

    fn boundary_edges(m: &EditMesh) -> Vec<EdgeKey> {
        Topology::build(m)
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .collect()
    }

    fn total_area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    /// Cubo sem a face `f` (buraco de 4 arestas).
    fn cube_without(f: u32) -> EditMesh {
        let mut m = primitives::cube(1.0);
        m.delete_faces(&[f]).unwrap();
        m
    }

    #[test]
    fn fill_hole_closes_cube_with_consistent_outward_winding() {
        for f in 0..6 {
            let mut m = cube_without(f);
            let edges = boundary_edges(&m);
            assert_eq!(edges.len(), 4);
            let out = fill_hole(&mut m, &edges).unwrap();
            m.validate().unwrap();
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold(), "face {f}");
            assert_eq!(t.euler_characteristic(&m), 2);
            assert_outward(&m);
            assert_eq!(out.created_faces, vec![5]);
            assert!(out.selection.faces.contains(&5));
        }
    }

    #[test]
    fn fill_hole_on_open_cylinder_and_capsule_loops() {
        let mut m = primitives::cylinder(1.0, 2.0, 12, 3, false);
        let t = Topology::build(&m);
        let b: Vec<EdgeKey> = t
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .collect();
        assert_eq!(b.len(), 24);
        // dois loops de uma vez
        let out = fill_hole(&mut m, &b).unwrap();
        assert_eq!(out.created_faces.len(), 2);
        let t2 = Topology::build(&m);
        assert!(t2.is_closed_manifold());
        assert_eq!(t2.euler_characteristic(&m), 2);
        assert_outward(&m);
        assert!(m.faces.iter().filter(|f| f.len() == 12).count() == 2);
    }

    #[test]
    fn fill_hole_attribute_inheritance() {
        let mut m = cube_without(0);
        for f in &mut m.faces {
            f.material = 4;
            f.smooth = false;
            f.face_set = 9;
        }
        let edges = boundary_edges(&m);
        fill_hole(&mut m, &edges).unwrap();
        let nf = m.faces.last().unwrap();
        assert_eq!((nf.material, nf.smooth, nf.face_set), (4, false, 9));
    }

    #[test]
    fn fill_hole_wire_loop_uses_deterministic_order() {
        let mut m = EditMesh::new("w");
        let ids: Vec<u32> = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z]
            .iter()
            .map(|&p| m.add_vert(p))
            .collect();
        for i in 0..4 {
            m.edge_attrs
                .insert(edge_key(ids[i], ids[(i + 1) % 4]), Default::default());
        }
        let edges: Vec<EdgeKey> = m.edge_attrs.keys().copied().collect();
        let mut m2 = m.clone();
        fill_hole(&mut m, &edges).unwrap();
        fill_hole(&mut m2, &edges).unwrap();
        assert_eq!(m.content_hash(), m2.content_hash());
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.faces[0].len(), 4);
        m.validate().unwrap();
    }

    #[test]
    fn fill_hole_errors_leave_mesh_intact() {
        let mut m = cube_without(0);
        let h = m.content_hash();
        let edges = boundary_edges(&m);
        assert!(matches!(
            fill_hole(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            fill_hole(&mut m, &[(0, 6)]),
            Err(ModelError::MissingEdge(..))
        ));
        // 3 das 4 arestas: loop aberto
        assert!(matches!(
            fill_hole(&mut m, &edges[..3]),
            Err(ModelError::InvalidParameter(_))
        ));
        // aresta interior (2 faces)
        let t = Topology::build(&m);
        let interior = t.edges.iter().find(|e| e.faces.len() == 2).unwrap().verts;
        assert!(matches!(
            fill_hole(&mut m, &[interior]),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn fill_hole_inconsistent_winding_errors() {
        let mut m = cube_without(0);
        // inverte uma das faces vizinhas do buraco
        let edges = boundary_edges(&m);
        let t = Topology::build(&m);
        let f = t.edge_faces(edges[0].0, edges[0].1)[0];
        m.flip_face(f).unwrap();
        let h = m.content_hash();
        assert!(matches!(
            fill_hole(&mut m, &edges),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn fill_hole_verts_variant() {
        let mut m = cube_without(2);
        let t = Topology::build(&m);
        let mut vs: BTreeSet<u32> = BTreeSet::new();
        for e in t.edges.iter().filter(|e| e.faces.len() == 1) {
            vs.insert(e.verts.0);
            vs.insert(e.verts.1);
        }
        let vs: Vec<u32> = vs.into_iter().collect();
        fill_hole_verts(&mut m, &vs).unwrap();
        assert!(Topology::build(&m).is_closed_manifold());
        // vértices sem fronteira entre si
        let mut c = primitives::cube(1.0);
        assert!(matches!(
            fill_hole_verts(&mut c, &[0, 1]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(fill_hole_verts(&mut c, &[]).is_err());
    }

    #[test]
    fn fill_triangles_splits_loop_and_preserves_area() {
        for beauty in [false, true] {
            let mut m = cube_without(1);
            let before = total_area(&m);
            let edges = boundary_edges(&m);
            let out = fill_triangles(&mut m, &edges, beauty).unwrap();
            assert_eq!(out.created_faces.len(), 2);
            assert!(m.faces.iter().rev().take(2).all(|f| f.len() == 3));
            assert!((total_area(&m) - before - 1.0).abs() < 1e-4);
            assert!(Topology::build(&m).is_closed_manifold());
            assert_outward(&m);
        }
        // n-gon grande (círculo de 12)
        let mut c = primitives::cylinder(1.0, 1.0, 12, 1, false);
        let edges: Vec<EdgeKey> = Topology::build(&c)
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1 && c.co(e.verts.0).y > 0.0)
            .map(|e| e.verts)
            .collect();
        let out = fill_triangles(&mut c, &edges, true).unwrap();
        assert_eq!(out.created_faces.len(), 10);
    }

    #[test]
    fn fill_all_holes_filters_by_size() {
        // cubo com dois buracos opostos de 4 arestas e uma "chaminé" maior
        let mut m = cube_without(0);
        let mut other = primitives::cylinder(1.0, 1.0, 10, 1, false);
        other.transform_positions(|p| p + Vec3::new(10.0, 0.0, 0.0));
        m.append(&other);
        let mut small = m.clone();
        let out = fill_all_holes(&mut small, 4).unwrap();
        assert_eq!(out.created_faces.len(), 1);
        let mut all = m.clone();
        let out = fill_all_holes(&mut all, usize::MAX).unwrap();
        assert_eq!(out.created_faces.len(), 3);
        assert!(Topology::build(&all).is_closed_manifold());
        assert_eq!(Topology::build(&all).boundary_edge_count(), 0);
        // nada a fazer
        let mut closed = primitives::cube(1.0);
        let h = closed.content_hash();
        let out = fill_all_holes(&mut closed, usize::MAX).unwrap();
        assert!(out.created_faces.is_empty());
        assert_eq!(closed.content_hash(), h);
        assert!(matches!(
            fill_all_holes(&mut closed, 2),
            Err(ModelError::InvalidParameter(_))
        ));
    }

    #[test]
    fn fill_all_holes_skips_pinched_boundaries() {
        // duas faces que se tocam num vértice: fronteira ambígua em O
        let mut m = EditMesh::new("bow");
        let o = m.add_vert(Vec3::ZERO);
        let a = m.add_vert(Vec3::X);
        let b = m.add_vert(Vec3::Z);
        let c = m.add_vert(-Vec3::X);
        let d = m.add_vert(-Vec3::Z);
        m.add_face(&[o, b, a]).unwrap();
        m.add_face(&[o, d, c]).unwrap();
        let out = fill_all_holes(&mut m, usize::MAX).unwrap();
        // cada triângulo tem seu loop de 3 passando por `o` => ambíguo, ignorado
        assert!(out.created_faces.is_empty());
        m.validate().unwrap();
    }

    fn ring(n: u32, radius: f32) -> EditMesh {
        // anel aberto (cilindro curto sem tampas) para ter um loop com orientação conhecida
        primitives::cylinder(radius, 1.0, n, 1, false)
    }

    fn top_loop(m: &EditMesh) -> Vec<EdgeKey> {
        Topology::build(m)
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1 && m.co(e.verts.0).y > 0.0)
            .map(|e| e.verts)
            .collect()
    }

    #[test]
    fn grid_fill_patterns() {
        // (arestas, quads, vértices novos)
        for (n, quads, new_verts) in [
            (4u32, 1usize, 0usize),
            (6, 2, 0),
            (8, 4, 1),
            (10, 6, 2),
            (12, 9, 4),
            (16, 16, 9),
        ] {
            let mut m = ring(n, 1.0);
            let edges = top_loop(&m);
            let nv = m.vert_count();
            let out = grid_fill(&mut m, &edges, 0, 0, false).unwrap();
            m.validate().unwrap();
            assert_eq!(out.created_faces.len(), quads, "n={n}");
            assert_eq!(m.vert_count() - nv, new_verts, "n={n}");
            assert_eq!(out.created_verts.len(), new_verts);
            assert!(m.faces[out.created_faces[0] as usize..]
                .iter()
                .all(|f| f.len() == 4));
            // tampa de cima fechada: o resto (fundo aberto) continua com 1 loop
            let t = Topology::build(&m);
            assert_eq!(t.boundary_edge_count(), n as usize);
            // normais da grade apontam para +Y (para fora do cilindro)
            for &f in &out.created_faces {
                assert!(
                    m.face_normal(f).y > 0.9,
                    "n={n} face {f}: {:?}",
                    m.face_normal(f)
                );
            }
        }
    }

    #[test]
    fn grid_fill_circle_of_eight_is_two_by_two_with_center_vertex() {
        let mut m = ring(8, 1.0);
        let edges = top_loop(&m);
        let nv = m.vert_count();
        let out = grid_fill(&mut m, &edges, 0, 0, false).unwrap();
        assert_eq!(out.created_faces.len(), 4);
        let c = out.created_verts[0];
        // o vértice central fica no eixo (círculo simétrico) e na altura do loop
        let p = m.co(c);
        assert!(p.x.abs() < 1e-4 && p.z.abs() < 1e-4 && (p.y - 0.5).abs() < 1e-5);
        assert_eq!(c as usize, nv);
        // todos os 4 quads têm o vértice central e ele tem valência 4
        let t = Topology::build(&m);
        assert_eq!(t.valence(c), 4);
        assert_eq!(t.vert_faces[c as usize].len(), 4);
    }

    #[test]
    fn grid_fill_span_offset_and_simple_interp() {
        let mut m = ring(12, 1.0);
        let edges = top_loop(&m);
        let mut variants = Vec::new();
        for (span, offset, simple) in [
            (0, 0, false),
            (2, 0, false),
            (4, 0, false),
            (3, 5, false),
            (3, 0, true),
        ] {
            let mut c = m.clone();
            let out = grid_fill(&mut c, &edges, span, offset, simple).unwrap();
            c.validate().unwrap();
            let a = if span == 0 { 3 } else { span };
            assert_eq!(out.created_faces.len(), a * (6 - a));
            assert_eq!(out.created_verts.len(), (a - 1) * (6 - a - 1));
            variants.push(c.content_hash());
        }
        // variantes distintas produzem malhas distintas
        let uniq: BTreeSet<u64> = variants.iter().copied().collect();
        assert!(uniq.len() >= 4);
        // interp simples: interior plano (todos no plano do loop) e Coons do círculo também
        let out = grid_fill(&mut m, &edges, 3, 0, true).unwrap();
        for &v in &out.created_verts {
            assert!((m.co(v).y - 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn grid_fill_coons_interior_stays_inside_the_circle() {
        let mut m = ring(16, 2.0);
        let edges = top_loop(&m);
        let out = grid_fill(&mut m, &edges, 0, 0, false).unwrap();
        for &v in &out.created_verts {
            let p = m.co(v);
            assert!(Vec3::new(p.x, 0.0, p.z).length() < 2.0 + 1e-4);
        }
        // área próxima à do polígono regular de 16 lados
        let poly_area = 0.5 * 16.0 * 4.0 * (std::f32::consts::TAU / 16.0).sin();
        let area: f32 = out.created_faces.iter().map(|&f| m.face_area(f)).sum();
        assert!(
            (area - poly_area).abs() / poly_area < 0.05,
            "{area} vs {poly_area}"
        );
    }

    #[test]
    fn grid_fill_errors_are_clear_and_non_destructive() {
        let mut m = ring(9, 1.0);
        let h = m.content_hash();
        let edges = top_loop(&m);
        let e = grid_fill(&mut m, &edges, 0, 0, false).unwrap_err();
        assert!(e.to_string().contains("par"), "{e}");
        let mut m2 = ring(8, 1.0);
        let h2 = m2.content_hash();
        let edges2 = top_loop(&m2);
        assert!(grid_fill(&mut m2, &edges2, 4, 0, false).is_err());
        assert!(grid_fill(&mut m2, &edges2, 100, 0, false).is_err());
        // dois loops
        let all: Vec<EdgeKey> = Topology::build(&m2)
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .collect();
        assert!(grid_fill(&mut m2, &all, 0, 0, false).is_err());
        assert!(grid_fill(&mut m2, &[], 0, 0, false).is_err());
        // loop de 2 lados não existe; triângulo (3 arestas) é ímpar
        let mut tri = primitives::circle(1.0, 3, false);
        let tri_edges: Vec<EdgeKey> = Topology::build(&tri)
            .edges
            .iter()
            .map(|e| e.verts)
            .collect();
        assert!(grid_fill(&mut tri, &tri_edges, 0, 0, false).is_err());
        assert_eq!(m.content_hash(), h);
        assert_eq!(m2.content_hash(), h2);
    }

    #[test]
    fn grid_fill_closes_open_sphere_patch_with_consistent_winding() {
        let mut m = primitives::quad_sphere(1.0, 4);
        // remove um bloco 2x2 de faces de um lado para abrir buraco de 8 arestas
        let kill: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| {
                let c = m.face_center(f);
                c.y > 0.7 && c.x.abs() < 0.45 && c.z.abs() < 0.45
            })
            .collect();
        assert_eq!(kill.len(), 4);
        m.delete_faces(&kill).unwrap();
        let t = Topology::build(&m);
        let edges: Vec<EdgeKey> = t
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .collect();
        assert_eq!(edges.len(), 8);
        let out = grid_fill(&mut m, &edges, 0, 0, false).unwrap();
        assert_eq!(out.created_faces.len(), 4);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_outward(&m);
    }

    #[test]
    fn coons_matches_boundaries_and_bilinear_for_flat_quad() {
        let bottom = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0];
        let top = [
            Vec3::Z * 2.0,
            Vec3::new(1.0, 0.0, 2.0),
            Vec3::new(2.0, 0.0, 2.0),
        ];
        let left = [Vec3::ZERO, Vec3::Z, Vec3::Z * 2.0];
        let right = [
            Vec3::X * 2.0,
            Vec3::new(2.0, 0.0, 1.0),
            Vec3::new(2.0, 0.0, 2.0),
        ];
        let p = coons_point(&bottom, &top, &left, &right, 1, 1);
        assert!((p - Vec3::new(1.0, 0.0, 1.0)).length() < 1e-6);
        assert!((coons_point(&bottom, &top, &left, &right, 2, 0) - bottom[2]).length() < 1e-6);
        assert!((coons_point(&bottom, &top, &left, &right, 0, 2) - left[2]).length() < 1e-6);
    }

    #[test]
    fn fill_ops_are_deterministic() {
        let run = || {
            let mut m = ring(12, 1.0);
            let edges = top_loop(&m);
            grid_fill(&mut m, &edges, 0, 1, false).unwrap();
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn fill_all_holes_large_mesh() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let kill: Vec<u32> = (0..m.face_count() as u32).step_by(997).collect();
        m.delete_faces(&kill).unwrap();
        let t0 = std::time::Instant::now();
        let out = fill_all_holes(&mut m, usize::MAX).unwrap();
        assert!(t0.elapsed().as_secs() < 30);
        assert_eq!(out.created_faces.len(), kill.len());
        assert!(Topology::build(&m).is_closed_manifold());
    }
}
