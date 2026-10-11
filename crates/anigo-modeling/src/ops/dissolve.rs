//! Dissolve (`Ctrl+X`) de vértices, arestas e faces, e Limited Dissolve (Planar
//! Decimate) do Blender.
//!
//! Princípio: *dissolver* remove o elemento **sem abrir buracos** — as faces
//! vizinhas são fundidas em um n-gon cuja fronteira é o contorno da união
//! ([`merge_face_region`]). Se a união não for um disco simples (tem buracos, toca
//! a si mesma num vértice, enrolamento inconsistente ou é uma superfície fechada
//! sem fronteira) a operação falha com `UnsupportedTopology` e a malha fica intacta
//! (todo operador trabalha numa cópia e só confirma no fim).
//!
//! UV e normais customizadas do n-gon resultante vêm dos cantos originais da
//! fronteira; material/suavização/face set vêm da face de menor índice do grupo.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EdgeKey, EditMesh, Face, Remap};
use crate::ops::delete::{
    face_edge_set, prune_vanished_edge_attrs, remove_unused_candidates, unique_edges, unique_faces,
    unique_verts,
};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// ------------------------------------------------------------------ união de regiões

/// Funde um conjunto de faces (região conexa, enrolamento consistente) no
/// contorno único da união. Retorna os cantos do polígono resultante, na ordem do
/// contorno e com a orientação das faces originais, ou uma mensagem de erro.
///
/// Falha quando: uma aresta orientada se repete (enrolamento inconsistente ou
/// aresta usada 3+ vezes), não há fronteira (superfície fechada), a fronteira
/// passa duas vezes pelo mesmo vértice, ou há mais de um contorno (buracos).
pub(crate) fn merge_face_region(mesh: &EditMesh, faces: &[u32]) -> Result<Vec<Corner>, String> {
    let mut directed: HashMap<(u32, u32), Corner> = HashMap::new();
    for &f in faces {
        let face = &mesh.faces[f as usize];
        let n = face.corners.len();
        for i in 0..n {
            let a = face.corners[i];
            let b = face.corners[(i + 1) % n];
            if directed.insert((a.vert, b.vert), a).is_some() {
                return Err(format!(
                    "aresta ({}, {}) repetida na mesma direção: enrolamento inconsistente ou não-manifold",
                    a.vert, b.vert
                ));
            }
        }
    }
    let mut next: BTreeMap<u32, (u32, Corner)> = BTreeMap::new();
    for (&(a, b), &c) in &directed {
        if directed.contains_key(&(b, a)) {
            continue; // aresta interior
        }
        if next.insert(a, (b, c)).is_some() {
            return Err(format!(
                "a região toca a si mesma no vértice {a} (fronteira não simples)"
            ));
        }
    }
    let Some((&start, _)) = next.iter().next() else {
        return Err("a região não tem fronteira (superfície fechada)".into());
    };
    let mut loop_corners: Vec<Corner> = Vec::with_capacity(next.len());
    let mut cur = start;
    loop {
        let Some(&(nxt, corner)) = next.get(&cur) else {
            return Err("fronteira da região aberta".into());
        };
        loop_corners.push(corner);
        cur = nxt;
        if cur == start {
            break;
        }
        if loop_corners.len() > next.len() {
            return Err("fronteira da região não fecha".into());
        }
    }
    if loop_corners.len() != next.len() {
        return Err("a região tem buracos (mais de um contorno)".into());
    }
    if loop_corners.len() < 3 {
        return Err("contorno degenerado (< 3 vértices)".into());
    }
    Ok(loop_corners)
}

/// Grupos de faces ligadas por arestas "unidas" (`joins`: pares de faces).
fn group_faces(n_faces: usize, joins: &[(u32, u32)], members: &[u32]) -> Vec<Vec<u32>> {
    let mut parent: Vec<u32> = (0..n_faces as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            let g = p[p[x as usize] as usize];
            p[x as usize] = g;
            x = g;
        }
        x
    }
    for &(a, b) in joins {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent[hi as usize] = lo;
        }
    }
    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &f in members {
        let r = find(&mut parent, f);
        groups.entry(r).or_default().push(f);
    }
    groups.into_values().collect()
}

/// Substitui cada grupo (já validado) por sua face fundida, no índice da menor
/// face do grupo. Devolve o mapa antigo → novo de faces e os índices novos das
/// faces fundidas.
fn replace_groups(
    mesh: &mut EditMesh,
    merged: &[(Vec<u32>, Vec<Corner>)],
) -> (Vec<Option<u32>>, Vec<u32>) {
    let nf = mesh.faces.len();
    let mut head: HashMap<u32, usize> = HashMap::new(); // menor face -> índice do grupo
    let mut member: Vec<Option<usize>> = vec![None; nf];
    for (gi, (faces, _)) in merged.iter().enumerate() {
        head.insert(faces[0], gi);
        for &f in faces {
            member[f as usize] = Some(gi);
        }
    }
    let old = std::mem::take(&mut mesh.faces);
    let mut map: Vec<Option<u32>> = vec![None; nf];
    let mut group_new: Vec<Option<u32>> = vec![None; merged.len()];
    let mut created = Vec::new();
    for (fi, face) in old.into_iter().enumerate() {
        match member[fi] {
            None => {
                map[fi] = Some(mesh.faces.len() as u32);
                mesh.faces.push(face);
            }
            Some(gi) => {
                if head.get(&(fi as u32)) == Some(&gi) {
                    let idx = mesh.faces.len() as u32;
                    group_new[gi] = Some(idx);
                    created.push(idx);
                    mesh.faces.push(Face {
                        corners: merged[gi].1.clone(),
                        ..face
                    });
                }
                map[fi] = group_new[gi];
            }
        }
    }
    // membros do grupo vistos antes do `head` não existem (head é o menor índice)
    (map, created)
}

fn compose(a: &[Option<u32>], b: &[Option<u32>]) -> Vec<Option<u32>> {
    a.iter()
        .map(|x| x.and_then(|i| b.get(i as usize).copied().flatten()))
        .collect()
}

/// Combina o mapa de faces das etapas anteriores com o remap da compactação de
/// vértices (que nunca renumera faces).
fn finish(face_map: Vec<Option<u32>>, mut remap: Remap) -> Remap {
    remap.face = compose(&face_map, &remap.face);
    remap
}

// ------------------------------------------------------------ vértices de valência 2

/// Regras para dissolver vértices "em linha" (exatamente 2 arestas).
#[derive(Debug, Clone, Copy)]
struct PairRule {
    /// Ângulo máximo (rad) entre as duas arestas (0 = retas) quando o vértice
    /// separa 2 faces. Negativo desativa.
    max_angle_2face: f32,
    /// Idem quando o vértice está numa fronteira (1 face). Negativo desativa.
    max_angle_1face: f32,
    delimit_seams: bool,
    delimit_sharp: bool,
}

fn line_angle(a: glam::Vec3, v: glam::Vec3, b: glam::Vec3) -> f32 {
    let d0 = (v - a).normalize_or_zero();
    let d1 = (b - v).normalize_or_zero();
    d0.dot(d1).clamp(-1.0, 1.0).acos()
}

/// Dissolve vértices com exatamente 2 arestas que satisfaçam `rule`, iterando até
/// estabilizar. Não compacta o vetor de vértices: devolve a lista dos vértices
/// dissolvidos (já sem faces) para o chamador remover.
fn dissolve_pair_verts(mesh: &mut EditMesh, candidates: &[u32], rule: PairRule) -> Vec<u32> {
    let mut dead: Vec<u32> = Vec::new();
    let mut is_dead = vec![false; mesh.verts.len()];
    for _pass in 0..64 {
        let topo = Topology::build(mesh);
        let mut changed = false;
        for &v in candidates {
            let vi = v as usize;
            if is_dead[vi] || topo.vert_edges[vi].len() != 2 {
                continue;
            }
            let (e1, e2) = (
                &topo.edges[topo.vert_edges[vi][0] as usize],
                &topo.edges[topo.vert_edges[vi][1] as usize],
            );
            let other =
                |e: &crate::topology::EdgeInfo| if e.verts.0 == v { e.verts.1 } else { e.verts.0 };
            let (a, b) = (other(e1), other(e2));
            if a == b || is_dead[a as usize] || is_dead[b as usize] {
                continue;
            }
            let faces = &topo.vert_faces[vi];
            let ok_faces = match faces.len() {
                2 => {
                    e1.faces.len() == 2
                        && e2.faces.len() == 2
                        && e1.faces.iter().all(|f| faces.contains(f))
                        && e2.faces.iter().all(|f| faces.contains(f))
                }
                1 => e1.faces.len() == 1 && e2.faces.len() == 1,
                _ => false,
            };
            if !ok_faces {
                continue;
            }
            let limit = if faces.len() == 2 {
                rule.max_angle_2face
            } else {
                rule.max_angle_1face
            };
            if limit < 0.0 {
                continue;
            }
            let ang = line_angle(mesh.co(a), mesh.co(v), mesh.co(b));
            if ang > limit {
                continue;
            }
            let (aa, ab) = (mesh.edge_attr(a, v), mesh.edge_attr(v, b));
            if (rule.delimit_seams && aa.seam != ab.seam)
                || (rule.delimit_sharp && aa.sharp != ab.sharp)
            {
                continue;
            }
            if topo.edge(a, b).is_some() {
                continue; // criaria aresta duplicada
            }
            if faces.iter().any(|&f| mesh.faces[f as usize].len() <= 3) {
                continue;
            }
            for &f in faces {
                mesh.faces[f as usize].corners.retain(|c| c.vert != v);
            }
            mesh.edge_attrs.remove(&edge_key(a, v));
            mesh.edge_attrs.remove(&edge_key(v, b));
            let merged = crate::ops::delete::merge_edge_attr(aa, ab);
            if !merged.is_default() {
                mesh.edge_attrs.insert(edge_key(a, b), merged);
            }
            is_dead[vi] = true;
            dead.push(v);
            changed = true;
        }
        if !changed {
            break;
        }
    }
    dead
}

// --------------------------------------------------------------------- dissolve_verts

/// *Dissolve Vertices*: remove os vértices fundindo as faces que os cercam num
/// n-gon (o vértice some do contorno). Vértices de fronteira também são dissolvidos
/// (o contorno passa direto entre seus vizinhos).
///
/// * `use_face_split` (Blender: *Face Split*): antes de fundir, corta de cada face
///   a "orelha" `(anterior, v, seguinte)` de cada vértice selecionado, de modo que
///   só as orelhas são fundidas e o resto da geometria ao redor é preservado
///   (ex.: vértice interno de um grid vira um quad entre seus 4 vizinhos em vez de
///   um octógono).
/// * `use_boundary_tear` (Blender: *Tear Boundary*): vértices de fronteira não fundem
///   as faces; cada face apenas perde o canto (triângulos desaparecem).
///
/// Vértices sem face: com 2 arestas wire são "pulados" (as arestas viram uma só);
/// demais são apagados com suas arestas. Também remove vértices interiores que
/// ficam sem uso. Seleção devolvida: as faces fundidas/criadas. Falha com
/// `UnsupportedTopology` se alguma união não for um disco simples; malha intacta.
pub fn dissolve_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    use_face_split: bool,
    use_boundary_tear: bool,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    let mut work = mesh.clone();
    let out = dissolve_verts_impl(&mut work, &sel, use_face_split, use_boundary_tear)?;
    *mesh = work;
    Ok(out)
}

/// Corta a orelha de cada vértice selecionado em faces com mais de 3 cantos.
/// Devolve o mapa antigo → novo (primeira peça).
fn ear_split(mesh: &mut EditMesh, selected: &BTreeSet<u32>) -> Vec<Option<u32>> {
    let nf = mesh.faces.len();
    let old = std::mem::take(&mut mesh.faces);
    let mut map = vec![None; nf];
    for (fi, face) in old.into_iter().enumerate() {
        map[fi] = Some(mesh.faces.len() as u32);
        if face.len() <= 3 || !face.verts().any(|v| selected.contains(&v)) {
            mesh.faces.push(face);
            continue;
        }
        let mut poly = face.corners.clone();
        let mut ears: Vec<Vec<Corner>> = Vec::new();
        while poly.len() > 3 {
            let Some(i) = poly.iter().position(|c| selected.contains(&c.vert)) else {
                break;
            };
            let n = poly.len();
            ears.push(vec![poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]]);
            poly.remove(i);
        }
        for c in ears.into_iter().chain(std::iter::once(poly)) {
            mesh.faces.push(Face {
                corners: c,
                ..face.clone()
            });
        }
    }
    map
}

fn dissolve_verts_impl(
    work: &mut EditMesh,
    sel: &[u32],
    use_face_split: bool,
    use_boundary_tear: bool,
) -> ModelResult<OpOutput> {
    let selected: BTreeSet<u32> = sel.iter().copied().collect();
    let old_face_edges = face_edge_set(work);
    let mut face_map: Vec<Option<u32>> = (0..work.faces.len() as u32).map(Some).collect();
    if use_face_split {
        face_map = ear_split(work, &selected);
    }
    let topo = Topology::build(work);

    // Vértices wire (sem face): junta o par de arestas ou apaga.
    let mut bridges: Vec<(u32, u32, crate::mesh::EdgeAttr)> = Vec::new();
    for &v in sel {
        if !topo.vert_faces[v as usize].is_empty() {
            continue;
        }
        let nbrs = topo.vert_neighbors(v);
        if nbrs.len() == 2 && topo.vert_edges[v as usize].len() == 2 {
            let (a, b) = (nbrs[0], nbrs[1]);
            if !selected.contains(&a) && !selected.contains(&b) {
                bridges.push((a, b, work.edge_attr(a, v)));
            }
        }
    }

    // Grupos de faces a fundir (união das faces ao redor de cada vértice não "rasgado").
    let mut joins: Vec<(u32, u32)> = Vec::new();
    let mut involved: BTreeSet<u32> = BTreeSet::new();
    for &v in sel {
        let fs = &topo.vert_faces[v as usize];
        if use_boundary_tear && topo.is_boundary_vert(v) {
            involved.extend(fs.iter().copied());
            continue;
        }
        involved.extend(fs.iter().copied());
        for w in fs.windows(2) {
            joins.push((w[0], w[1]));
        }
    }
    let members: Vec<u32> = involved.iter().copied().collect();
    let groups = group_faces(work.faces.len(), &joins, &members);
    let mut merged: Vec<(Vec<u32>, Vec<Corner>)> = Vec::new();
    for g in groups {
        if g.len() < 2 {
            continue;
        }
        let corners = merge_face_region(work, &g)
            .map_err(|e| ModelError::UnsupportedTopology(format!("dissolve de vértices: {e}")))?;
        merged.push((g, corners));
    }
    let candidates: Vec<u32> = involved
        .iter()
        .flat_map(|&f| work.faces[f as usize].verts().collect::<Vec<_>>())
        .chain(sel.iter().copied())
        .collect();
    let (map2, created) = replace_groups(work, &merged);
    face_map = compose(&face_map, &map2);

    // Remove os vértices selecionados de todos os contornos; faces < 3 cantos somem.
    let nf = work.faces.len();
    let mut kill = vec![false; nf];
    for (fi, f) in work.faces.iter_mut().enumerate() {
        if f.corners.iter().any(|c| selected.contains(&c.vert)) {
            f.corners.retain(|c| !selected.contains(&c.vert));
            if f.corners.len() < 3 {
                kill[fi] = true;
            }
        }
    }
    let mut map3: Vec<Option<u32>> = vec![None; nf];
    let old = std::mem::take(&mut work.faces);
    for (fi, f) in old.into_iter().enumerate() {
        if !kill[fi] {
            map3[fi] = Some(work.faces.len() as u32);
            work.faces.push(f);
        }
    }
    face_map = compose(&face_map, &map3);
    let created: Vec<u32> = created.iter().filter_map(|&c| map3[c as usize]).collect();

    // Atributos de aresta.
    prune_vanished_edge_attrs(work, &old_face_edges);
    for (a, b, attr) in bridges {
        work.edge_attrs.insert(edge_key(a, b), attr);
    }
    work.edge_attrs
        .retain(|&(a, b), _| !selected.contains(&a) && !selected.contains(&b));

    let mut cleanup = candidates;
    cleanup.sort_unstable();
    cleanup.dedup();
    // vértices selecionados já não aparecem em face nem em aresta wire
    let remap = remove_unused_candidates(work, &cleanup);
    let remap = finish(face_map, remap);
    Ok(OpOutput {
        selection: Selection::from_faces(work, created.iter().copied()),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    })
}

// --------------------------------------------------------------------- dissolve_edges

/// *Dissolve Edges*: remove as arestas fundindo as duas faces vizinhas de cada uma
/// em um n-gon (cadeias de arestas fundem regiões inteiras).
///
/// * arestas com exatamente 2 faces distintas são fundidas;
/// * arestas wire são apagadas;
/// * arestas de fronteira (1 face) e não-manifold (3+) são ignoradas;
/// * `use_verts`: depois da fusão, dissolve os extremos que ficaram com apenas 2
///   arestas (entre 2 faces, ou numa fronteira se as arestas forem colineares),
///   sem criar faces com menos de 3 cantos.
///
/// Erros: seleção vazia, aresta inexistente, nada dissolvível
/// (`EmptySelection("arestas dissolvíveis")`) ou união que não é um disco simples
/// (`UnsupportedTopology`). Seleção devolvida: faces resultantes.
pub fn dissolve_edges(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    use_verts: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let edges = unique_edges(&topo, edges, "arestas")?;
    let mut work = mesh.clone();
    let out = dissolve_edges_impl(&mut work, &topo, &edges, use_verts)?;
    *mesh = work;
    Ok(out)
}

fn dissolve_edges_impl(
    work: &mut EditMesh,
    topo: &Topology,
    edges: &[EdgeKey],
    use_verts: bool,
) -> ModelResult<OpOutput> {
    let old_face_edges = face_edge_set(work);
    let mut joins: Vec<(u32, u32)> = Vec::new();
    let mut wire: Vec<EdgeKey> = Vec::new();
    let mut involved: BTreeSet<u32> = BTreeSet::new();
    let mut endpoints: Vec<u32> = Vec::new();
    for &(a, b) in edges {
        let Some(info) = topo.edge(a, b) else {
            return Err(ModelError::MissingEdge(a, b));
        };
        match info.faces.len() {
            0 => {
                wire.push((a, b));
                endpoints.extend([a, b]);
            }
            2 if info.faces[0] != info.faces[1] => {
                joins.push((info.faces[0], info.faces[1]));
                involved.extend(info.faces.iter().copied());
                endpoints.extend([a, b]);
            }
            _ => {}
        }
    }
    if wire.is_empty() && joins.is_empty() {
        return Err(ModelError::EmptySelection("arestas dissolvíveis"));
    }
    let members: Vec<u32> = involved.iter().copied().collect();
    let groups = group_faces(work.faces.len(), &joins, &members);
    let mut merged: Vec<(Vec<u32>, Vec<Corner>)> = Vec::new();
    for g in groups {
        if g.len() < 2 {
            continue;
        }
        let corners = merge_face_region(work, &g)
            .map_err(|e| ModelError::UnsupportedTopology(format!("dissolve de arestas: {e}")))?;
        merged.push((g, corners));
    }
    let mut cleanup: Vec<u32> = involved
        .iter()
        .flat_map(|&f| work.faces[f as usize].verts().collect::<Vec<_>>())
        .collect();
    cleanup.extend(endpoints.iter().copied());
    for k in &wire {
        work.edge_attrs.remove(k);
    }
    let (face_map, created) = replace_groups(work, &merged);
    prune_vanished_edge_attrs(work, &old_face_edges);

    let mut dead: Vec<u32> = Vec::new();
    if use_verts {
        let mut cands = endpoints.clone();
        cands.sort_unstable();
        cands.dedup();
        dead = dissolve_pair_verts(
            work,
            &cands,
            PairRule {
                max_angle_2face: std::f32::consts::PI,
                max_angle_1face: 1e-3,
                delimit_seams: false,
                delimit_sharp: false,
            },
        );
    }
    cleanup.extend(dead);
    cleanup.sort_unstable();
    cleanup.dedup();
    let remap = remove_unused_candidates(work, &cleanup);
    let remap = finish(face_map, remap);
    let created: Vec<u32> = created;
    Ok(OpOutput {
        selection: Selection::from_faces(work, created.iter().copied()),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    })
}

// --------------------------------------------------------------------- dissolve_faces

/// *Dissolve Faces*: funde as faces selecionadas que compartilham arestas num único
/// n-gon por região conexa (cada região precisa ser um disco simples). Vértices
/// interiores à região que ficam sem uso são removidos; arestas internas somem
/// com seus atributos. Faces isoladas (sem vizinha selecionada) ficam como estão;
/// se nenhuma região tem 2+ faces a operação é um no-op (`remap = None`).
///
/// Erros: seleção vazia/índice inválido, ou região que não é disco simples
/// (`UnsupportedTopology`). Seleção devolvida: as faces resultantes.
pub fn dissolve_faces(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let sel = unique_faces(mesh, faces, "faces")?;
    let topo = Topology::build(mesh);
    let in_sel: BTreeSet<u32> = sel.iter().copied().collect();
    let mut joins: Vec<(u32, u32)> = Vec::new();
    for e in &topo.edges {
        let fs: Vec<u32> = e
            .faces
            .iter()
            .copied()
            .filter(|f| in_sel.contains(f))
            .collect();
        for w in fs.windows(2) {
            if w[0] != w[1] {
                joins.push((w[0], w[1]));
            }
        }
    }
    let groups = group_faces(mesh.faces.len(), &joins, &sel);
    let mut merged: Vec<(Vec<u32>, Vec<Corner>)> = Vec::new();
    for g in groups {
        if g.len() < 2 {
            continue;
        }
        let corners = merge_face_region(mesh, &g)
            .map_err(|e| ModelError::UnsupportedTopology(format!("dissolve de faces: {e}")))?;
        merged.push((g, corners));
    }
    if merged.is_empty() {
        return Ok(OpOutput {
            selection: Selection::from_faces(mesh, sel.iter().copied()),
            ..Default::default()
        });
    }
    let old_face_edges = face_edge_set(mesh);
    let cleanup: Vec<u32> = merged
        .iter()
        .flat_map(|(g, _)| {
            g.iter()
                .flat_map(|&f| mesh.faces[f as usize].verts().collect::<Vec<_>>())
        })
        .collect();
    let (face_map, created) = replace_groups(mesh, &merged);
    prune_vanished_edge_attrs(mesh, &old_face_edges);
    let mut cleanup = cleanup;
    cleanup.sort_unstable();
    cleanup.dedup();
    let remap = remove_unused_candidates(mesh, &cleanup);
    let remap = finish(face_map, remap);
    let created: Vec<u32> = created;
    // faces selecionadas que não participaram de nenhuma região também ficam selecionadas
    let mut selection = Selection::from_faces(mesh, created.iter().copied());
    for &f in &sel {
        if let Some(n) = remap.face[f as usize] {
            selection.faces.insert(n);
        }
    }
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, selection.faces.iter().copied()),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    })
}

// ----------------------------------------------------------------------- limited dissolve

/// Delimitadores do *Limited Dissolve*: arestas que não podem ser dissolvidas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Delimit {
    /// Não funde faces de materiais diferentes.
    pub materials: bool,
    /// Preserva arestas marcadas como costura (seam).
    pub seams: bool,
    /// Preserva arestas marcadas como sharp.
    pub sharp: bool,
    /// Preserva arestas onde as UVs das duas faces não coincidem.
    pub uv: bool,
}

fn uv_continuous(mesh: &EditMesh, f0: u32, f1: u32, a: u32, b: u32) -> bool {
    let uv_of = |f: u32, v: u32| {
        mesh.faces[f as usize]
            .corners
            .iter()
            .find(|c| c.vert == v)
            .map(|c| c.uv)
    };
    match (uv_of(f0, a), uv_of(f1, a), uv_of(f0, b), uv_of(f1, b)) {
        (Some(a0), Some(a1), Some(b0), Some(b1)) => {
            (a0 - a1).length() < 1e-5 && (b0 - b1).length() < 1e-5
        }
        _ => true,
    }
}

/// *Limited Dissolve* (Planar Decimate): dissolve arestas entre faces quase
/// coplanares e vértices em linha reta.
///
/// * `angle_limit`: ângulo máximo (rad, `0..=π`) entre as normais de duas faces —
///   ou entre a normal média dos grupos já unidos — para fundi-las (tolerância
///   numérica interna ≈ 0.08°, para que `0` funda faces exatamente coplanares);
/// * `use_dissolve_boundaries`: também dissolve vértices colineares em fronteiras;
/// * `delimit`: tipos de aresta que nunca são dissolvidos.
///
/// Arestas são processadas por ordem crescente de ângulo (desempate pelo índice),
/// então o resultado é determinístico. Um grupo cuja união não seja um disco simples
/// (anel, buraco) simplesmente permanece como estava. Vértices de valência 2 são
/// dissolvidos quando o desvio de reta é ≤ `angle_limit` (entre 2 faces: sempre
/// respeitando `delimit.seams/sharp`; em fronteira: só com
/// `use_dissolve_boundaries`). Seleção devolvida: faces fundidas.
pub fn limited_dissolve(
    mesh: &mut EditMesh,
    angle_limit: f32,
    use_dissolve_boundaries: bool,
    delimit: Delimit,
) -> ModelResult<OpOutput> {
    if !angle_limit.is_finite() || !(0.0..=std::f32::consts::PI + 1e-4).contains(&angle_limit) {
        return Err(ModelError::InvalidParameter(format!(
            "angle_limit deve estar em [0, π], recebeu {angle_limit}"
        )));
    }
    let mut work = mesh.clone();
    let out = limited_dissolve_impl(&mut work, angle_limit, use_dissolve_boundaries, delimit);
    *mesh = work;
    Ok(out)
}

fn limited_dissolve_impl(
    work: &mut EditMesh,
    angle_limit: f32,
    use_dissolve_boundaries: bool,
    delimit: Delimit,
) -> OpOutput {
    let old_face_edges = face_edge_set(work);
    let topo = Topology::build(work);
    let nf = work.faces.len();
    let cos_limit = angle_limit.cos() - 1e-6;
    let normals: Vec<glam::Vec3> = (0..nf as u32).map(|f| work.face_normal(f)).collect();
    let areas: Vec<f32> = (0..nf as u32).map(|f| work.face_area(f)).collect();

    // Arestas candidatas ordenadas por ângulo.
    let mut cands: Vec<(f32, EdgeKey, u32, u32)> = Vec::new();
    for e in &topo.edges {
        if e.faces.len() != 2 || e.faces[0] == e.faces[1] {
            continue;
        }
        let (f0, f1) = (e.faces[0], e.faces[1]);
        let (n0, n1) = (normals[f0 as usize], normals[f1 as usize]);
        if n0 == glam::Vec3::ZERO || n1 == glam::Vec3::ZERO {
            continue;
        }
        let dot = n0.dot(n1);
        if dot < cos_limit {
            continue;
        }
        let attr = work.edge_attr(e.verts.0, e.verts.1);
        if (delimit.seams && attr.seam) || (delimit.sharp && attr.sharp) {
            continue;
        }
        if delimit.materials && work.faces[f0 as usize].material != work.faces[f1 as usize].material
        {
            continue;
        }
        if delimit.uv && !uv_continuous(work, f0, f1, e.verts.0, e.verts.1) {
            continue;
        }
        cands.push((dot, e.verts, f0, f1));
    }
    // maior cosseno (menor ângulo) primeiro
    cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut parent: Vec<u32> = (0..nf as u32).collect();
    let mut nsum: Vec<glam::Vec3> = (0..nf).map(|i| normals[i] * areas[i].max(1e-12)).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            let g = p[p[x as usize] as usize];
            p[x as usize] = g;
            x = g;
        }
        x
    }
    for &(_, _, f0, f1) in &cands {
        let (r0, r1) = (find(&mut parent, f0), find(&mut parent, f1));
        if r0 == r1 {
            continue;
        }
        let (s0, s1) = (
            nsum[r0 as usize].normalize_or_zero(),
            nsum[r1 as usize].normalize_or_zero(),
        );
        if s0.dot(s1) < cos_limit {
            continue;
        }
        let (lo, hi) = if r0 < r1 { (r0, r1) } else { (r1, r0) };
        parent[hi as usize] = lo;
        nsum[lo as usize] = nsum[lo as usize] + nsum[hi as usize];
    }
    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for f in 0..nf as u32 {
        let r = find(&mut parent, f);
        groups.entry(r).or_default().push(f);
    }
    let mut merged: Vec<(Vec<u32>, Vec<Corner>)> = Vec::new();
    for g in groups.into_values() {
        if g.len() < 2 {
            continue;
        }
        if let Ok(corners) = merge_face_region(work, &g) {
            merged.push((g, corners));
        }
    }
    let mut cleanup: Vec<u32> = merged
        .iter()
        .flat_map(|(g, _)| {
            g.iter()
                .flat_map(|&f| work.faces[f as usize].verts().collect::<Vec<_>>())
        })
        .collect();
    let (face_map, created) = replace_groups(work, &merged);
    prune_vanished_edge_attrs(work, &old_face_edges);

    // Vértices em linha reta.
    let topo2 = Topology::build(work);
    let line_cands: Vec<u32> = (0..work.verts.len() as u32)
        .filter(|&v| topo2.vert_edges[v as usize].len() == 2)
        .collect();
    let dead = dissolve_pair_verts(
        work,
        &line_cands,
        PairRule {
            max_angle_2face: angle_limit,
            max_angle_1face: if use_dissolve_boundaries {
                angle_limit
            } else {
                -1.0
            },
            delimit_seams: delimit.seams,
            delimit_sharp: delimit.sharp,
        },
    );
    cleanup.extend(dead);
    cleanup.sort_unstable();
    cleanup.dedup();
    let remap = remove_unused_candidates(work, &cleanup);
    let remap = finish(face_map, remap);
    OpOutput {
        selection: Selection::from_faces(work, created.iter().copied()),
        created_verts: Vec::new(),
        created_faces: created,
        remap: Some(remap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::primitives::assert_outward;
    use glam::Vec3;

    fn total_area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    fn interior_edges(m: &EditMesh) -> Vec<EdgeKey> {
        let t = Topology::build(m);
        t.edges
            .iter()
            .filter(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .collect()
    }

    fn center_vert(m: &EditMesh) -> u32 {
        (0..m.vert_count() as u32)
            .min_by(|&a, &b| m.co(a).length().total_cmp(&m.co(b).length()))
            .unwrap()
    }

    #[test]
    fn dissolve_all_interior_edges_of_grid_gives_one_face() {
        for (nx, nz) in [(2, 2), (3, 3), (4, 2), (1, 5)] {
            let mut m = primitives::grid(nx, nz, nx as f32, nz as f32);
            let area = total_area(&m);
            let edges = interior_edges(&m);
            if edges.is_empty() {
                continue;
            }
            let out = dissolve_edges(&mut m, &edges, false).unwrap();
            m.validate().unwrap();
            assert_eq!(m.face_count(), 1, "{nx}x{nz}");
            assert_eq!(m.faces[0].len(), 2 * (nx + nz) as usize);
            assert!((total_area(&m) - area).abs() < 1e-4);
            assert_eq!(out.selection.faces.len(), 1);
            let t = Topology::build(&m);
            assert_eq!(t.euler_characteristic(&m), 1);
            assert!(m.edge_attrs.is_empty());
            assert_eq!(
                m.vert_count(),
                2 * (nx + nz) as usize,
                "vértices interiores removidos"
            );
        }
    }

    #[test]
    fn dissolve_all_interior_edges_with_use_verts_gives_a_quad() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let area = total_area(&m);
        let edges = interior_edges(&m);
        dissolve_edges(&mut m, &edges, true).unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.faces[0].len(), 4);
        assert_eq!(m.vert_count(), 4);
        assert!((total_area(&m) - area).abs() < 1e-4);
    }

    #[test]
    fn dissolve_one_cube_edge_gives_hexagon_and_stays_closed() {
        let mut m = primitives::cube(1.0);
        let e = Topology::build(&m).edges[0].verts;
        let out = dissolve_edges(&mut m, &[e], false).unwrap();
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(m.face_count(), 5);
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 6).count(), 1);
        assert_eq!(out.created_faces.len(), 1);
        assert_outward(&m);
        // use_verts: os dois extremos têm 2 arestas e 2 faces
        let mut m2 = primitives::cube(1.0);
        dissolve_edges(&mut m2, &[e], true).unwrap();
        m2.validate().unwrap();
        let t2 = Topology::build(&m2);
        assert!(t2.is_closed_manifold());
        assert_eq!(m2.vert_count(), 6);
        assert_eq!(t2.euler_characteristic(&m2), 2);
    }

    #[test]
    fn dissolve_edges_ignores_boundary_and_errors_when_nothing_dissolvable() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let t = Topology::build(&m);
        let boundary: Vec<EdgeKey> = t
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .collect();
        let h = m.content_hash();
        assert!(matches!(
            dissolve_edges(&mut m, &boundary, false),
            Err(ModelError::EmptySelection(_))
        ));
        assert_eq!(m.content_hash(), h);
        // misturado: fronteira é ignorada, interior dissolve
        let mut all: Vec<EdgeKey> = boundary.clone();
        all.push(interior_edges(&m)[0]);
        dissolve_edges(&mut m, &all, false).unwrap();
        assert_eq!(m.face_count(), 3);
    }

    #[test]
    fn dissolve_edges_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            dissolve_edges(&mut m, &[], false),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            dissolve_edges(&mut m, &[(0, 6)], false),
            Err(ModelError::MissingEdge(0, 6))
        ));
        // todas as arestas de um cubo: união fechada, sem fronteira
        let all: Vec<EdgeKey> = Topology::build(&m).edges.iter().map(|e| e.verts).collect();
        assert!(matches!(
            dissolve_edges(&mut m, &all, false),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn dissolve_wire_edge_removes_it() {
        let mut m = EditMesh::new("w");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        m.edge_attrs.insert(edge_key(a, b), Default::default());
        // aresta wire é dissolvida (apagada) e os vértices soltos removidos
        dissolve_edges(&mut m, &[(a, b)], false).unwrap();
        assert!(m.edge_attrs.is_empty());
        assert_eq!(m.vert_count(), 0);
    }

    #[test]
    fn dissolve_edges_prunes_attrs_of_interior_edges() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        let e = interior_edges(&m)[0];
        m.set_sharp(e.0, e.1, true);
        m.set_seam(e.0, e.1, true);
        dissolve_edges(&mut m, &[e], false).unwrap();
        assert!(m.edge_attrs.is_empty());
    }

    #[test]
    fn dissolve_edges_keeps_uv_and_face_attrs() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        m.faces[0].material = 7;
        m.faces[0].smooth = false;
        let e = interior_edges(&m)[0];
        dissolve_edges(&mut m, &[e], false).unwrap();
        assert_eq!(m.faces[0].material, 7);
        assert!(!m.faces[0].smooth);
        // UVs cobrem 0..1 e todos os cantos têm UV distinto
        let us: BTreeSet<u32> = m.faces[0]
            .corners
            .iter()
            .map(|c| (c.uv.x * 100.0) as u32)
            .collect();
        assert!(us.contains(&0) && us.contains(&100) && us.contains(&50));
    }

    #[test]
    fn dissolve_inconsistent_winding_errors() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        m.flip_face(1).unwrap();
        let e = interior_edges(&m)[0];
        let h = m.content_hash();
        assert!(matches!(
            dissolve_edges(&mut m, &[e], false),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn dissolve_interior_vertex_of_grid_makes_octagon() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let v = (0..m.vert_count() as u32)
            .find(|&v| (m.co(v) - Vec3::new(-0.5, 0.0, -0.5)).length() < 1e-5)
            .unwrap();
        let area = total_area(&m);
        let out = dissolve_verts(&mut m, &[v], false, false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 9 - 4 + 1);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 8).count(), 1);
        assert_eq!(m.vert_count(), 15);
        assert!((total_area(&m) - area).abs() < 1e-4);
        assert_eq!(Topology::build(&m).euler_characteristic(&m), 1);
        assert_eq!(out.created_faces.len(), 1);
    }

    #[test]
    fn dissolve_vertex_face_split_keeps_surrounding_geometry() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let v = (0..m.vert_count() as u32)
            .find(|&v| (m.co(v) - Vec3::new(-0.5, 0.0, -0.5)).length() < 1e-5)
            .unwrap();
        let area = total_area(&m);
        dissolve_verts(&mut m, &[v], true, false).unwrap();
        m.validate().unwrap();
        // 5 faces intactas + 4 triângulos remanescentes + 1 quad central
        assert_eq!(m.face_count(), 10);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 4).count(), 5 + 1);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 3).count(), 4);
        assert_eq!(m.vert_count(), 15);
        assert!((total_area(&m) - area).abs() < 1e-4);
        assert_eq!(Topology::build(&m).euler_characteristic(&m), 1);
    }

    #[test]
    fn dissolve_cube_corner_gives_hexagon() {
        let mut m = primitives::cube(1.0);
        dissolve_verts(&mut m, &[0], false, false).unwrap();
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(m.vert_count(), 7);
        assert_eq!(m.face_count(), 4);
        assert_eq!(t.euler_characteristic(&m), 2);
    }

    #[test]
    fn dissolve_boundary_vertex_variants() {
        // grid 2x2: vértice do meio de um lado (2 faces)
        let base = primitives::grid(2, 2, 2.0, 2.0);
        let v = (0..base.vert_count() as u32)
            .find(|&v| (base.co(v) - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5)
            .unwrap();
        let mut m = base.clone();
        dissolve_verts(&mut m, &[v], false, false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 3);
        assert!(m.faces.iter().any(|f| f.len() == 5));
        assert_eq!(m.vert_count(), 8);
        // tear: cada face perde o canto, não funde
        let mut t = base.clone();
        dissolve_verts(&mut t, &[v], false, true).unwrap();
        t.validate().unwrap();
        assert_eq!(t.face_count(), 4);
        assert_eq!(t.faces.iter().filter(|f| f.len() == 3).count(), 2);
        // vértice de canto: quad → triângulo
        let corner = (0..base.vert_count() as u32)
            .find(|&v| (base.co(v) - Vec3::new(-1.0, 0.0, -1.0)).length() < 1e-5)
            .unwrap();
        let mut c = base.clone();
        dissolve_verts(&mut c, &[corner], false, false).unwrap();
        assert_eq!(c.face_count(), 4);
        assert_eq!(c.faces.iter().filter(|f| f.len() == 3).count(), 1);
        // tear num vértice cujo canto está num triângulo remove a face
        let mut tri = primitives::circle(1.0, 3, false);
        let out = dissolve_verts(&mut tri, &[0], false, true).unwrap();
        assert_eq!(tri.face_count(), 0);
        assert_eq!(out.remap.unwrap().face, vec![None]);
    }

    #[test]
    fn dissolve_two_adjacent_interior_verts_merges_into_one_region() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let a = center_vert(&m);
        let nb = Topology::build(&m).vert_neighbors(a)[0];
        let area = total_area(&m);
        dissolve_verts(&mut m, &[a, nb], false, false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 16 - 6 + 1);
        assert_eq!(m.vert_count(), 25 - 2);
        assert!((total_area(&m) - area).abs() < 1e-4);
    }

    #[test]
    fn dissolve_ring_around_vertex_removes_enclosed_vertex() {
        // os 8 vizinhos do centro de um grid 4x4: a união das faces ao redor deles cobre o grid
        // inteiro e o centro (não selecionado) fica enterrado na região: some junto
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let area = total_area(&m);
        let c = center_vert(&m);
        let t = Topology::build(&m);
        let mut ring: Vec<u32> = Vec::new();
        for f in &t.vert_faces[c as usize] {
            ring.extend(m.faces[*f as usize].verts().filter(|&v| v != c));
        }
        ring.sort_unstable();
        ring.dedup();
        assert_eq!(ring.len(), 8);
        dissolve_verts(&mut m, &ring, false, false).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.vert_count(), 16);
        assert!((total_area(&m) - area).abs() < 1e-4);
        assert!(!(0..m.vert_count() as u32).any(|v| m.co(v).length() < 1e-6));
    }

    #[test]
    fn dissolve_verts_wire_chain_and_errors() {
        let mut m = EditMesh::new("w");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        let c = m.add_vert(Vec3::new(2.0, 0.0, 0.0));
        m.edge_attrs.insert(
            edge_key(a, b),
            crate::mesh::EdgeAttr {
                sharp: true,
                ..Default::default()
            },
        );
        m.edge_attrs.insert(edge_key(b, c), Default::default());
        dissolve_verts(&mut m, &[b], false, false).unwrap();
        assert_eq!(m.vert_count(), 2);
        assert_eq!(m.edge_attrs.len(), 1);
        assert!(m.edge_attrs.values().next().unwrap().sharp);
        let h = m.content_hash();
        assert!(matches!(
            dissolve_verts(&mut m, &[], false, false),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            dissolve_verts(&mut m, &[9], false, false),
            Err(ModelError::VertexOutOfRange(9, 2))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn dissolve_verts_on_pinch_fails_cleanly() {
        // duas faces que só compartilham um vértice (gravata-borboleta): a união toca a si mesma
        let mut m = EditMesh::new("bow");
        let o = m.add_vert(Vec3::ZERO);
        let a = m.add_vert(Vec3::new(1.0, 0.0, 0.0));
        let b = m.add_vert(Vec3::new(0.0, 0.0, 1.0));
        let c = m.add_vert(Vec3::new(-1.0, 0.0, 0.0));
        let d = m.add_vert(Vec3::new(0.0, 0.0, -1.0));
        m.add_face(&[o, b, a]).unwrap();
        m.add_face(&[o, d, c]).unwrap();
        let h = m.content_hash();
        assert!(matches!(
            dissolve_verts(&mut m, &[o], false, false),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(m.content_hash(), h);
        // com tear nada é fundido: cada triângulo perde o canto e desaparece
        let mut t = m.clone();
        dissolve_verts(&mut t, &[o], false, true).unwrap();
        assert_eq!(t.face_count(), 0);
    }

    #[test]
    fn dissolve_faces_regions() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let area = total_area(&m);
        let out = dissolve_faces(&mut m, &[0, 1, 2, 3]).unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.faces[0].len(), 8);
        assert_eq!(m.vert_count(), 8);
        assert!((total_area(&m) - area).abs() < 1e-4);
        assert_eq!(out.selection.faces.len(), 1);
        m.validate().unwrap();

        // duas regiões separadas
        let mut g = primitives::grid(5, 1, 5.0, 1.0);
        let out = dissolve_faces(&mut g, &[0, 1, 3, 4]).unwrap();
        assert_eq!(g.face_count(), 3);
        assert_eq!(out.created_faces.len(), 2);
        g.validate().unwrap();

        // face isolada: no-op
        let mut s = primitives::grid(3, 1, 3.0, 1.0);
        let h = s.content_hash();
        let out = dissolve_faces(&mut s, &[1]).unwrap();
        assert_eq!(s.content_hash(), h);
        assert!(out.remap.is_none());
        assert_eq!(out.selection.faces.len(), 1);
    }

    #[test]
    fn dissolve_faces_cube_pairs_and_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        // cubo inteiro: fechado, sem fronteira
        assert!(matches!(
            dissolve_faces(&mut m, &[0, 1, 2, 3, 4, 5]),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert!(matches!(
            dissolve_faces(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            dissolve_faces(&mut m, &[9]),
            Err(ModelError::FaceOutOfRange(9, 6))
        ));
        assert_eq!(m.content_hash(), h);
        // 5 faces de 6: tampa aberta — disco simples
        dissolve_faces(&mut m, &[0, 1, 2, 3, 4]).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 2);
        assert!(Topology::build(&m).is_closed_manifold());
    }

    #[test]
    fn dissolve_faces_with_hole_fails() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let ring: Vec<u32> = (0..9u32).filter(|&f| f != 4).collect();
        let h = m.content_hash();
        assert!(matches!(
            dissolve_faces(&mut m, &ring),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn limited_dissolve_flat_grid_becomes_quad() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let area = total_area(&m);
        limited_dissolve(&mut m, 0.0, false, Delimit::default()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(
            m.faces[0].len(),
            16,
            "vértices de borda permanecem sem use_dissolve_boundaries"
        );
        let mut m2 = primitives::grid(4, 4, 4.0, 4.0);
        limited_dissolve(&mut m2, 0.0, true, Delimit::default()).unwrap();
        assert_eq!(m2.face_count(), 1);
        assert_eq!(m2.faces[0].len(), 4);
        assert_eq!(m2.vert_count(), 4);
        assert!((total_area(&m2) - area).abs() < 1e-4);
    }

    #[test]
    fn limited_dissolve_zero_angle_keeps_curved_shape() {
        let m0 = primitives::quad_sphere(1.0, 4);
        let mut m = m0.clone();
        let out = limited_dissolve(&mut m, 0.0, true, Delimit::default()).unwrap();
        assert_eq!(m.content_hash(), m0.content_hash());
        assert!(out.selection.faces.is_empty());
        let mut u = primitives::uv_sphere(1.0, 8, 12);
        let h = u.content_hash();
        limited_dissolve(&mut u, 0.0, true, Delimit::default()).unwrap();
        assert_eq!(u.content_hash(), h);
    }

    #[test]
    fn limited_dissolve_subdivided_cube_returns_to_six_quads() {
        let mut m = primitives::cube_subdivided(2.0, 3);
        limited_dissolve(&mut m, 0.01, true, Delimit::default()).unwrap();
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_eq!(m.face_count(), 6);
        assert_eq!(m.vert_count(), 8);
        assert_outward(&m);
        assert!((total_area(&m) - 24.0).abs() < 1e-3);
    }

    #[test]
    fn limited_dissolve_delimiters() {
        // material
        let mut m = primitives::grid(4, 1, 4.0, 1.0);
        for f in 2..4 {
            m.faces[f].material = 1;
        }
        let mut a = m.clone();
        limited_dissolve(
            &mut a,
            0.0,
            true,
            Delimit {
                materials: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(a.face_count(), 2);
        let mut b = m.clone();
        limited_dissolve(&mut b, 0.0, true, Delimit::default()).unwrap();
        assert_eq!(b.face_count(), 1);
        // seam e sharp
        let e = interior_edges(&m)[1];
        let mut s = m.clone();
        s.set_seam(e.0, e.1, true);
        limited_dissolve(
            &mut s,
            0.0,
            true,
            Delimit {
                seams: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.face_count(), 2);
        let mut h = m.clone();
        h.set_sharp(e.0, e.1, true);
        limited_dissolve(
            &mut h,
            0.0,
            true,
            Delimit {
                sharp: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(h.face_count(), 2);
        // uv
        let mut u = m.clone();
        for c in &mut u.faces[3].corners {
            c.uv += glam::Vec2::new(5.0, 0.0);
        }
        limited_dissolve(
            &mut u,
            0.0,
            true,
            Delimit {
                uv: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(u.face_count(), 2);
    }

    #[test]
    fn limited_dissolve_angle_limits_and_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        for bad in [-0.1, f32::NAN, 4.0] {
            assert!(matches!(
                limited_dissolve(&mut m, bad, false, Delimit::default()),
                Err(ModelError::InvalidParameter(_))
            ));
        }
        assert_eq!(m.content_hash(), h);
        // dobra de 90° do cubo: limite abaixo não funde, acima funde (a casca vira anel
        // sem fronteira ⇒ permanece como estava)
        limited_dissolve(&mut m, 1.0, true, Delimit::default()).unwrap();
        assert_eq!(m.content_hash(), h);
        m.validate().unwrap();
    }

    #[test]
    fn limited_dissolve_on_organic_meshes_keeps_validity() {
        for angle in [0.05f32, 0.3, 0.8] {
            for m0 in [
                primitives::uv_sphere(1.0, 10, 14),
                primitives::capsule(0.5, 1.0, 12, 4),
                primitives::cylinder(1.0, 2.0, 16, 4, true),
                primitives::torus(1.0, 0.3, 16, 8),
            ] {
                let mut m = m0.clone();
                limited_dissolve(&mut m, angle, true, Delimit::default()).unwrap();
                m.validate().unwrap();
                let t = Topology::build(&m);
                assert!(t.is_closed_manifold(), "{} @ {angle}", m0.name);
                assert_eq!(
                    t.euler_characteristic(&m),
                    Topology::build(&m0).euler_characteristic(&m0),
                    "{} @ {angle}",
                    m0.name
                );
                assert!(m.face_count() <= m0.face_count());
            }
        }
    }

    #[test]
    fn dissolve_is_deterministic() {
        let run = || {
            let mut m = primitives::grid(5, 5, 5.0, 5.0);
            let edges: Vec<EdgeKey> = interior_edges(&m).into_iter().step_by(3).collect();
            dissolve_edges(&mut m, &edges, true).unwrap();
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn property_random_edge_subsets_preserve_area_and_euler() {
        let mut seed = 0x1234_5678_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..30 {
            let mut m = primitives::grid(6, 6, 6.0, 6.0);
            let area = total_area(&m);
            let edges: Vec<EdgeKey> = interior_edges(&m)
                .into_iter()
                .filter(|_| rnd() % 3 == 0)
                .collect();
            if edges.is_empty() {
                continue;
            }
            match dissolve_edges(&mut m, &edges, false) {
                Ok(_) => {
                    m.validate().unwrap();
                    assert!((total_area(&m) - area).abs() < 1e-3);
                    assert_eq!(Topology::build(&m).euler_characteristic(&m), 1);
                }
                Err(ModelError::UnsupportedTopology(_)) => {} // região com buraco: aceito
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    #[test]
    fn dissolve_large_mesh_is_fast() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let t = Topology::build(&m);
        let edges: Vec<EdgeKey> = t.edges.iter().step_by(5).map(|e| e.verts).collect();
        let t0 = std::time::Instant::now();
        let rr = dissolve_edges(&mut m, &edges, false).map(|_| ());
        assert!(rr.is_ok(), "{rr:?}");
        m.validate().unwrap();
        let mut big = primitives::quad_sphere(1.0, 64);
        limited_dissolve(&mut big, 0.0, true, Delimit::default()).unwrap();
        assert!(t0.elapsed().as_secs() < 60, "{:?}", t0.elapsed());
    }
}
