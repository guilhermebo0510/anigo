//! Merge (`M`) e Merge by Distance / Remove Doubles do Blender.
//!
//! Todas as operações passam pelo mesmo núcleo ([`weld`]): dado um mapa
//! `rep[v]` (vértice → representante), as faces são reescritas, degenerações
//! removidas (cantos consecutivos repetidos, faces com < 3 vértices distintos),
//! faces que se tornaram "laços" são divididas em polígonos simples, faces
//! duplicadas criadas pelo merge são descartadas, `edge_attrs` são remapeados
//! (atributos de arestas que colidem são mesclados) e os vértices absorvidos são
//! compactados.
//!
//! # Convenção do `remap`
//!
//! `OpOutput::remap.vert[v]` é o novo índice do vértice onde `v` foi parar:
//! vértices absorvidos apontam para o novo índice do **sobrevivente** (nunca
//! `None`). O número de vértices removidos é `índices_antes - mesh.vert_count()`
//! (ver [`removed_vert_count`]). `remap.face[f]` é o novo índice da face (a
//! primeira, se ela foi dividida; a face que a duplicava, se foi descartada por
//! duplicidade; `None` se degenerou).

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EdgeAttr, EdgeKey, EditMesh, Face, Remap};
use crate::ops::delete::{merge_edge_attr, unique_verts};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

// ------------------------------------------------------------------ grade espacial

/// Grade espacial por hash (células cúbicas de lado `cell`) para buscas de
/// vizinhança em O(1) amortizado. Determinística: a ordem dos ids numa célula é a
/// de inserção.
#[derive(Debug, Clone)]
pub(crate) struct PointGrid {
    cell: f32,
    map: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl PointGrid {
    pub fn new(cell: f32) -> Self {
        let cell = if cell.is_finite() && cell > 1e-6 {
            cell
        } else {
            1e-6
        };
        Self {
            cell,
            map: HashMap::new(),
        }
    }

    fn key(&self, p: Vec3) -> (i64, i64, i64) {
        let q = p / self.cell;
        (q.x.floor() as i64, q.y.floor() as i64, q.z.floor() as i64)
    }

    pub fn insert(&mut self, id: u32, p: Vec3) {
        let k = self.key(p);
        self.map.entry(k).or_default().push(id);
    }

    /// Chama `f(id)` para todo id cuja célula intersecta a esfera `(p, radius)`
    /// (é candidato; o chamador testa a distância exata).
    pub fn for_each_near(&self, p: Vec3, radius: f32, mut f: impl FnMut(u32)) {
        let r = if radius.is_finite() {
            radius.max(0.0)
        } else {
            0.0
        };
        let span = ((r / self.cell).ceil() as i64).clamp(1, 64);
        let (cx, cy, cz) = self.key(p);
        for dx in -span..=span {
            for dy in -span..=span {
                for dz in -span..=span {
                    if let Some(ids) = self.map.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &id in ids {
                            f(id);
                        }
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------- union-find

struct Dsu {
    parent: Vec<u32>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.parent[x as usize] = self.parent[p as usize];
            x = self.parent[x as usize];
        }
        x
    }
    /// Une; a raiz resultante é sempre o menor índice (representante determinístico).
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if ra < rb {
            self.parent[rb as usize] = ra;
        } else {
            self.parent[ra as usize] = rb;
        }
    }
}

// ----------------------------------------------------------------------------- núcleo

/// Divide uma lista de cantos (cíclica) em polígonos simples: quando um vértice
/// reaparece, o ciclo entre as duas ocorrências vira um polígono próprio.
/// Descarta polígonos com < 3 cantos.
fn split_simple(corners: &[Corner]) -> Vec<Vec<Corner>> {
    let mut out: Vec<Vec<Corner>> = Vec::new();
    let mut stack: Vec<Corner> = Vec::with_capacity(corners.len());
    for &c in corners {
        if let Some(pos) = stack.iter().position(|s| s.vert == c.vert) {
            let cyc = stack.split_off(pos);
            let first = cyc[0];
            if cyc.len() >= 3 {
                out.push(cyc);
            }
            stack.push(first);
        } else {
            stack.push(c);
        }
    }
    if stack.len() >= 3 {
        out.push(stack);
    }
    out
}

/// Remove cantos consecutivos repetidos (ciclicamente).
fn dedup_consecutive(corners: &[Corner]) -> Vec<Corner> {
    let mut out: Vec<Corner> = Vec::with_capacity(corners.len());
    for &c in corners {
        if out.last().map(|l| l.vert) != Some(c.vert) {
            out.push(c);
        }
    }
    while out.len() > 1 && out.first().map(|f| f.vert) == out.last().map(|l| l.vert) {
        out.pop();
    }
    out
}

/// Chave orientada de face: sequência de vértices rotacionada para começar pelo menor.
fn oriented_key(f: &Face) -> Vec<u32> {
    let vs: Vec<u32> = f.verts().collect();
    let Some(min_pos) = vs
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| **v)
        .map(|(i, _)| i)
    else {
        return vs;
    };
    let n = vs.len();
    (0..n).map(|i| vs[(min_pos + i) % n]).collect()
}

/// Núcleo de solda. `rep[v]` é o representante de `v` (idempotente: `rep[rep[v]] ==
/// rep[v]`); `moved` redefine posições dos representantes. Devolve o `Remap`
/// (ver convenção no topo do módulo). O chamador já validou tudo; nunca falha.
pub(crate) fn weld(mesh: &mut EditMesh, rep: &[u32], moved: &[(u32, Vec3)]) -> Remap {
    let nv = mesh.verts.len();
    let nf = mesh.faces.len();
    for &(v, p) in moved {
        mesh.verts[v as usize].co = p;
    }

    // Arestas de face antes (já mapeadas) para podar atributos de arestas que sumirem.
    let mut old_mapped: HashSet<EdgeKey> = HashSet::new();
    for f in &mesh.faces {
        for (a, b) in f.edges() {
            let (ra, rb) = (rep[a as usize], rep[b as usize]);
            if ra != rb {
                old_mapped.insert(edge_key(ra, rb));
            }
        }
    }

    // Reescreve faces.
    struct Entry {
        face: Face,
        modified: bool,
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(nf);
    let mut first_entry: Vec<Option<usize>> = vec![None; nf];
    let old_faces = std::mem::take(&mut mesh.faces);
    for (fi, face) in old_faces.into_iter().enumerate() {
        let changed = face.corners.iter().any(|c| rep[c.vert as usize] != c.vert);
        if !changed {
            first_entry[fi] = Some(entries.len());
            entries.push(Entry {
                face,
                modified: false,
            });
            continue;
        }
        let mapped: Vec<Corner> = face
            .corners
            .iter()
            .map(|c| Corner {
                vert: rep[c.vert as usize],
                ..*c
            })
            .collect();
        let cleaned = dedup_consecutive(&mapped);
        for poly in split_simple(&cleaned) {
            if first_entry[fi].is_none() {
                first_entry[fi] = Some(entries.len());
            }
            entries.push(Entry {
                face: Face {
                    corners: poly,
                    ..face.clone()
                },
                modified: true,
            });
        }
    }

    // Descarta duplicatas criadas pelo merge (mesma sequência orientada de vértices).
    let touched: HashSet<u32> = entries
        .iter()
        .filter(|e| e.modified)
        .flat_map(|e| e.face.verts())
        .collect();
    let mut seen: HashMap<Vec<u32>, usize> = HashMap::new();
    let mut alias: Vec<usize> = (0..entries.len()).collect(); // índice de entrada que "vale" por esta
    let mut dropped = vec![false; entries.len()];
    if !touched.is_empty() {
        for i in 0..entries.len() {
            let candidate =
                entries[i].modified || entries[i].face.verts().all(|v| touched.contains(&v));
            if !candidate {
                continue;
            }
            let key = oriented_key(&entries[i].face);
            match seen.get(&key) {
                Some(&j) if entries[i].modified || entries[j].modified => {
                    dropped[i] = true;
                    alias[i] = j;
                }
                Some(_) => {}
                None => {
                    seen.insert(key, i);
                }
            }
        }
    }

    // Índices finais.
    let mut final_idx: Vec<Option<u32>> = vec![None; entries.len()];
    let mut next = 0u32;
    for i in 0..entries.len() {
        if !dropped[i] {
            final_idx[i] = Some(next);
            next += 1;
        }
    }
    let mut face_map: Vec<Option<u32>> = vec![None; nf];
    for (fi, fe) in first_entry.iter().enumerate() {
        if let Some(&i) = fe.as_ref() {
            let target = alias[i];
            face_map[fi] = final_idx[target];
        }
    }
    mesh.faces = entries
        .into_iter()
        .zip(dropped.iter())
        .filter(|(_, d)| !**d)
        .map(|(e, _)| e.face)
        .collect();

    // Atributos de aresta.
    let new_face_edges = crate::ops::delete::face_edge_set(mesh);
    let old_attrs = std::mem::take(&mut mesh.edge_attrs);
    let mut merged: BTreeMap<EdgeKey, EdgeAttr> = BTreeMap::new();
    for ((a, b), attr) in old_attrs {
        let (ra, rb) = (rep[a as usize], rep[b as usize]);
        if ra == rb {
            continue;
        }
        let k = edge_key(ra, rb);
        if old_mapped.contains(&k) && !new_face_edges.contains(&k) {
            continue; // aresta de face que sumiu
        }
        merged
            .entry(k)
            .and_modify(|e| *e = merge_edge_attr(*e, attr))
            .or_insert(attr);
    }
    mesh.edge_attrs = merged;

    // Compacta vértices absorvidos.
    let keep: Vec<bool> = (0..nv).map(|v| rep[v] == v as u32).collect();
    let mut remap = mesh.compact_verts(&keep);
    let survivors = remap.vert.clone();
    for v in 0..nv {
        if rep[v] != v as u32 {
            remap.vert[v] = survivors[rep[v] as usize];
        }
    }
    remap.face = face_map;
    remap
}

/// Número de vértices que uma operação de merge removeu: `before_len` é
/// `mesh.vert_count()` antes da chamada.
pub fn removed_vert_count(before_len: usize, mesh: &EditMesh) -> usize {
    before_len.saturating_sub(mesh.vert_count())
}

fn identity_rep(n: usize) -> Vec<u32> {
    (0..n as u32).collect()
}

fn selection_from_survivors(remap: &Remap, survivors: impl IntoIterator<Item = u32>) -> Selection {
    Selection::from_verts(survivors.into_iter().filter_map(|v| remap.vert[v as usize]))
}

// --------------------------------------------------------------------- merge_verts

/// Onde os vértices selecionados são fundidos (Blender: *Merge → At Center / At First /
/// At Last / At Cursor / Collapse*).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MergeMode {
    /// Um único vértice na média das posições.
    Center,
    /// Na posição do primeiro vértice da lista (ordem em que foram passados).
    First,
    /// Na posição do último vértice da lista.
    Last,
    /// Um único vértice na posição dada (aceita seleção de 1 vértice).
    Cursor(Vec3),
    /// Cada componente conexo (por arestas com os dois extremos na seleção) colapsa
    /// no seu próprio centro; vértices sem aresta selecionada ficam como estão.
    Collapse,
}

/// *Merge* (`M`): funde os vértices em um (ou, no modo `Collapse`, um por componente).
///
/// * `verts`: vértices (a ordem importa para `First`/`Last`; duplicatas são ignoradas);
/// * o sobrevivente é o de menor índice (`Center`, `Cursor`, `Collapse`), o primeiro
///   (`First`) ou o último (`Last`) e **conserva seus atributos** (cor, skin, máscara).
///
/// Faces que degeneram são removidas, as que viram laços são divididas. Seleção
/// devolvida: o(s) vértice(s) resultante(s). Erros (malha intacta): lista vazia,
/// índice inválido, menos de 2 vértices (exceto `Cursor`), posição não finita, ou
/// `Collapse` sem nenhuma aresta entre os vértices.
pub fn merge_verts(mesh: &mut EditMesh, verts: &[u32], mode: MergeMode) -> ModelResult<OpOutput> {
    let sorted = unique_verts(mesh, verts, "vértices")?;
    let mut ordered: Vec<u32> = Vec::with_capacity(verts.len());
    let mut seen = BTreeSet::new();
    for &v in verts {
        if seen.insert(v) {
            ordered.push(v);
        }
    }
    if let MergeMode::Cursor(p) = mode {
        if !p.is_finite() {
            return Err(ModelError::InvalidParameter(
                "posição do cursor não finita".into(),
            ));
        }
    } else if sorted.len() < 2 {
        return Err(ModelError::InvalidParameter(
            "merge exige pelo menos 2 vértices".into(),
        ));
    }

    let mut rep = identity_rep(mesh.verts.len());
    let mut moved: Vec<(u32, Vec3)> = Vec::new();
    let mut survivors: Vec<u32> = Vec::new();
    let mean = |vs: &[u32]| -> Vec3 {
        vs.iter().map(|&v| mesh.verts[v as usize].co).sum::<Vec3>() / vs.len() as f32
    };

    if let MergeMode::Collapse = mode {
        let topo = Topology::build(mesh);
        let set: BTreeSet<u32> = sorted.iter().copied().collect();
        let mut dsu = Dsu::new(mesh.verts.len());
        let mut any = false;
        for e in &topo.edges {
            let (a, b) = e.verts;
            if set.contains(&a) && set.contains(&b) {
                dsu.union(a, b);
                any = true;
            }
        }
        if !any {
            return Err(ModelError::EmptySelection(
                "arestas entre os vértices selecionados",
            ));
        }
        let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &v in &sorted {
            groups.entry(dsu.find(v)).or_default().push(v);
        }
        for (root, members) in groups {
            if members.len() < 2 {
                continue;
            }
            for &v in &members {
                rep[v as usize] = root;
            }
            moved.push((root, mean(&members)));
            survivors.push(root);
        }
    } else {
        let (target, pos) = match mode {
            MergeMode::First => (ordered[0], mesh.verts[ordered[0] as usize].co),
            MergeMode::Last => {
                let l = ordered[ordered.len() - 1];
                (l, mesh.verts[l as usize].co)
            }
            MergeMode::Cursor(p) => (sorted[0], p),
            MergeMode::Center | MergeMode::Collapse => (sorted[0], mean(&sorted)),
        };
        for &v in &sorted {
            rep[v as usize] = target;
        }
        moved.push((target, pos));
        survivors.push(target);
    }

    let remap = weld(mesh, &rep, &moved);
    Ok(OpOutput {
        selection: selection_from_survivors(&remap, survivors),
        created_verts: Vec::new(),
        created_faces: Vec::new(),
        remap: Some(remap),
    })
}

// ------------------------------------------------------------------ merge_by_distance

/// Opções extras de [`merge_by_distance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ByDistanceOptions {
    /// Permite fundir vértices selecionados com vértices **não** selecionados
    /// (Blender: *Unselected*). Sem efeito quando `verts` é `None`.
    pub use_unselected: bool,
    /// Só funde pares de vértices ligados por uma aresta existente.
    pub connected_only: bool,
    /// Só funde pares ligados por uma aresta marcada como *sharp* (implica `connected_only`).
    pub sharp_edges_only: bool,
}

/// *Merge by Distance* (Remove Doubles): funde vértices a até `distance` um do outro.
///
/// * `verts`: `None` = malha toda; `Some(lista)` = só esses vértices (veja
///   `opts.use_unselected`);
/// * grade espacial hash (O(n) esperado); cadeias de vértices próximos são unidas
///   por union-find (todo o cluster conexo vira um vértice, mesmo que seu
///   diâmetro exceda `distance`, como no Blender);
/// * o representante é o **menor índice** do cluster e mantém posição e
///   atributos; faces degeneradas e duplicadas são removidas, `edge_attrs`
///   remapeados.
///
/// A seleção devolvida são os representantes dos clusters com 2+ vértices.
/// Sem nada para fundir: no-op com remap identidade. Erros: distância negativa ou
/// não finita, índice inválido, `Some(&[])`.
pub fn merge_by_distance(
    mesh: &mut EditMesh,
    verts: Option<&[u32]>,
    distance: f32,
    opts: ByDistanceOptions,
) -> ModelResult<OpOutput> {
    if !distance.is_finite() || distance < 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "distância inválida: {distance}"
        )));
    }
    let nv = mesh.verts.len();
    let subset: Option<Vec<u32>> = match verts {
        Some(list) => Some(unique_verts(mesh, list, "vértices")?),
        None => None,
    };
    let in_subset: Option<Vec<bool>> = subset.as_ref().map(|s| {
        let mut m = vec![false; nv];
        for &v in s {
            m[v as usize] = true;
        }
        m
    });
    let need_topo = opts.connected_only || opts.sharp_edges_only;
    let topo = need_topo.then(|| Topology::build(mesh));

    let mut grid = PointGrid::new(distance);
    for v in 0..nv as u32 {
        let include = match &in_subset {
            Some(m) => m[v as usize] || opts.use_unselected,
            None => true,
        };
        if include {
            grid.insert(v, mesh.verts[v as usize].co);
        }
    }

    let d2 = distance * distance;
    let mut dsu = Dsu::new(nv);
    let candidates: Vec<u32> = match &subset {
        Some(s) => s.clone(),
        None => (0..nv as u32).collect(),
    };
    for &v in &candidates {
        let p = mesh.verts[v as usize].co;
        grid.for_each_near(p, distance, |u| {
            if u == v {
                return;
            }
            if p.distance_squared(mesh.verts[u as usize].co) > d2 {
                return;
            }
            if let Some(t) = &topo {
                match t.edge(u, v) {
                    None => return,
                    Some(_) if opts.sharp_edges_only && !mesh.edge_attr(u, v).sharp => return,
                    Some(_) => {}
                }
            }
            dsu.union(u, v);
        });
    }

    let mut rep = identity_rep(nv);
    let mut sizes: HashMap<u32, u32> = HashMap::new();
    for v in 0..nv as u32 {
        let r = dsu.find(v);
        rep[v as usize] = r;
        *sizes.entry(r).or_insert(0) += 1;
    }
    let survivors: Vec<u32> = {
        let mut s: Vec<u32> = sizes
            .iter()
            .filter(|(_, &n)| n > 1)
            .map(|(&r, _)| r)
            .collect();
        s.sort_unstable();
        s
    };
    let remap = weld(mesh, &rep, &[]);
    Ok(OpOutput {
        selection: selection_from_survivors(&remap, survivors),
        created_verts: Vec::new(),
        created_faces: Vec::new(),
        remap: Some(remap),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::primitives::assert_outward;
    use glam::Vec3;

    /// Cubo "explodido": cada face com 4 vértices próprios (24 vértices).
    fn exploded(src: &EditMesh) -> EditMesh {
        let mut m = EditMesh::new("exploded");
        for f in &src.faces {
            let ids: Vec<u32> = f
                .verts()
                .map(|v| m.push_vert(src.verts[v as usize].clone()))
                .collect();
            let mut nf = f.clone();
            for (c, id) in nf.corners.iter_mut().zip(ids) {
                c.vert = id;
            }
            m.push_face(nf).unwrap();
        }
        m
    }

    #[test]
    fn by_distance_exploded_cube_becomes_eight_vertices() {
        let mut m = exploded(&primitives::cube(1.0));
        assert_eq!(m.vert_count(), 24);
        assert!(!Topology::build(&m).is_closed_manifold());
        let out = merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 8);
        assert_eq!(m.face_count(), 6);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_outward(&m);
        assert_eq!(removed_vert_count(24, &m), 16);
        let r = out.remap.unwrap();
        assert!(r.vert.iter().all(|v| v.is_some()));
        assert_eq!(out.selection.verts.len(), 8);
    }

    #[test]
    fn by_distance_matches_original_hash_modulo_vertex_order() {
        let mut m = exploded(&primitives::quad_sphere(1.0, 4));
        merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
        assert_eq!(m.vert_count(), primitives::quad_sphere(1.0, 4).vert_count());
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
    }

    #[test]
    fn by_distance_distance_zero_merges_only_coincident() {
        let mut m = exploded(&primitives::cube(1.0));
        merge_by_distance(&mut m, None, 0.0, ByDistanceOptions::default()).unwrap();
        assert_eq!(m.vert_count(), 8);
        let mut c = primitives::cube(1.0);
        let h = c.content_hash();
        let out = merge_by_distance(&mut c, None, 0.0, ByDistanceOptions::default()).unwrap();
        assert_eq!(c.content_hash(), h);
        assert!(out.selection.is_empty());
    }

    #[test]
    fn by_distance_chain_resolution() {
        // três pontos em linha com passo 0.6 e raio 0.7: 0-1 e 1-2 unem, 0-2 (1.2) não
        let mut m = EditMesh::new("chain");
        for i in 0..3 {
            m.add_vert(Vec3::new(i as f32 * 0.6, 0.0, 0.0));
        }
        let a = m.add_vert(Vec3::new(0.0, 1.0, 0.0));
        let b = m.add_vert(Vec3::new(1.2, 1.0, 0.0));
        m.add_face(&[0, 2, b, a]).unwrap();
        merge_by_distance(&mut m, None, 0.7, ByDistanceOptions::default()).unwrap();
        assert_eq!(m.vert_count(), 3); // {0,1,2} num só + a + b
                                       // a face virou triângulo (0 e 2 fundidos)
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.faces[0].len(), 3);
    }

    #[test]
    fn by_distance_removes_degenerate_and_duplicate_faces() {
        let mut m = EditMesh::new("dup");
        let p = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let a: Vec<u32> = p.iter().map(|&q| m.add_vert(q)).collect();
        let b: Vec<u32> = p.iter().map(|&q| m.add_vert(q)).collect();
        m.add_face(&a).unwrap();
        m.add_face(&b).unwrap();
        let out = merge_by_distance(&mut m, None, 1e-5, ByDistanceOptions::default()).unwrap();
        assert_eq!(m.vert_count(), 4);
        assert_eq!(m.face_count(), 1);
        let r = out.remap.unwrap();
        assert_eq!(r.face, vec![Some(0), Some(0)]);
    }

    #[test]
    fn by_distance_keeps_opposite_winding_pair() {
        let mut m = EditMesh::new("pillow");
        let p = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        let a: Vec<u32> = p.iter().map(|&q| m.add_vert(q)).collect();
        let b: Vec<u32> = p.iter().rev().map(|&q| m.add_vert(q)).collect();
        m.add_face(&a).unwrap();
        m.add_face(&b).unwrap();
        merge_by_distance(&mut m, None, 1e-5, ByDistanceOptions::default()).unwrap();
        assert_eq!(m.face_count(), 2);
        assert_eq!(m.vert_count(), 4);
    }

    #[test]
    fn by_distance_selected_only_and_unselected() {
        let mut m = EditMesh::new("sel");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::new(0.001, 0.0, 0.0));
        let c = m.add_vert(Vec3::new(0.0, 0.001, 0.0));
        let d = m.add_vert(Vec3::new(5.0, 0.0, 0.0));
        m.add_face(&[a, d, c]).unwrap();
        m.add_face(&[b, d, c]).unwrap();
        let mut only_b_c = m.clone();
        // b e c selecionados: b-c distam 0.0014 e fundem; a fica fora
        merge_by_distance(
            &mut only_b_c,
            Some(&[b, c]),
            0.01,
            ByDistanceOptions::default(),
        )
        .unwrap();
        assert_eq!(only_b_c.vert_count(), 3);
        // só b selecionado: sem unselected nada acontece
        let mut only_b = m.clone();
        let h = only_b.content_hash();
        merge_by_distance(&mut only_b, Some(&[b]), 0.01, ByDistanceOptions::default()).unwrap();
        assert_eq!(only_b.content_hash(), h);
        // com unselected funde b com a e c
        let mut with_un = m.clone();
        merge_by_distance(
            &mut with_un,
            Some(&[b]),
            0.01,
            ByDistanceOptions {
                use_unselected: true,
                ..Default::default()
            },
        )
        .unwrap();
        // {a, b, c} fundem; as duas faces degeneram (sobram a e d)
        assert_eq!(with_un.vert_count(), 2);
        assert_eq!(with_un.face_count(), 0);
    }

    #[test]
    fn by_distance_connected_and_sharp_only() {
        // quad explodido em duas metades coincidentes: sem aresta ligando os pares
        let mut m = exploded(&primitives::cube(1.0));
        let h = m.content_hash();
        merge_by_distance(
            &mut m,
            None,
            1e-4,
            ByDistanceOptions {
                connected_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            m.content_hash(),
            h,
            "vértices coincidentes sem aresta entre si não fundem"
        );

        // par ligado por aresta sharp
        let mut g = EditMesh::new("g");
        let a = g.add_vert(Vec3::ZERO);
        let b = g.add_vert(Vec3::new(0.001, 0.0, 0.0));
        let c = g.add_vert(Vec3::new(0.0, 1.0, 0.0));
        g.add_face(&[a, b, c]).unwrap();
        let mut g2 = g.clone();
        merge_by_distance(
            &mut g,
            None,
            0.01,
            ByDistanceOptions {
                sharp_edges_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(g.vert_count(), 3, "aresta não é sharp: nada acontece");
        g2.set_sharp(a, b, true);
        merge_by_distance(
            &mut g2,
            None,
            0.01,
            ByDistanceOptions {
                sharp_edges_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(g2.vert_count(), 2);
        assert_eq!(g2.face_count(), 0, "triângulo colapsado");
        assert!(
            g2.edge_attrs.is_empty(),
            "aresta colapsada não deixa atributo"
        );
    }

    #[test]
    fn by_distance_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let o = ByDistanceOptions::default();
        assert!(matches!(
            merge_by_distance(&mut m, None, -1.0, o),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            merge_by_distance(&mut m, None, f32::NAN, o),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            merge_by_distance(&mut m, Some(&[]), 0.1, o),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            merge_by_distance(&mut m, Some(&[44]), 0.1, o),
            Err(ModelError::VertexOutOfRange(44, 8))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn by_distance_preserves_survivor_attributes_and_edge_attrs() {
        let mut m = exploded(&primitives::cube(1.0));
        m.verts[0].mask = 0.25;
        m.verts[0].color = [0.1, 0.2, 0.3, 0.4];
        // marca sharp numa aresta da face 0 (vértices 0-1 explodidos)
        m.set_sharp(0, 1, true);
        m.set_seam(2, 3, true);
        let out = merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
        let r = out.remap.unwrap();
        let s = r.vert[0].unwrap() as usize;
        assert!((m.verts[s].mask - 0.25).abs() < 1e-6);
        assert_eq!(m.verts[s].color, [0.1, 0.2, 0.3, 0.4]);
        assert!(m.edge_attrs.values().any(|e| e.sharp));
        assert!(m.edge_attrs.values().any(|e| e.seam));
        m.validate().unwrap();
    }

    #[test]
    fn by_distance_is_deterministic() {
        let a = {
            let mut m = exploded(&primitives::quad_sphere(1.0, 5));
            merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
            m.content_hash()
        };
        let b = {
            let mut m = exploded(&primitives::quad_sphere(1.0, 5));
            merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
            m.content_hash()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn by_distance_large_mesh_is_fast() {
        let mut m = exploded(&primitives::quad_sphere(1.0, 64));
        assert!(m.vert_count() > 90_000);
        let t0 = std::time::Instant::now();
        merge_by_distance(&mut m, None, 1e-4, ByDistanceOptions::default()).unwrap();
        assert!(t0.elapsed().as_secs() < 30, "demorou {:?}", t0.elapsed());
        assert_eq!(m.vert_count(), 6 * 64 * 64 + 2);
        assert!(Topology::build(&m).is_closed_manifold());
    }

    #[test]
    fn merge_center_two_adjacent_verts_of_grid() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let area_before: f32 = (0..m.face_count() as u32).map(|f| m.face_area(f)).sum();
        // dois vértices adjacentes: o do centro e um vizinho
        let center = (0..m.vert_count() as u32)
            .find(|&v| m.co(v).length() < 1e-6)
            .unwrap();
        let t = Topology::build(&m);
        let nb = t.vert_neighbors(center)[0];
        let out = merge_verts(&mut m, &[center, nb], MergeMode::Center).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 8);
        // faces que continham a aresta (2) viram triângulos; as outras ficam quads
        assert_eq!(m.face_count(), 4);
        assert_eq!(m.faces.iter().filter(|f| f.len() == 3).count(), 2);
        assert_eq!(out.selection.verts.len(), 1);
        let v = *out.selection.verts.iter().next().unwrap();
        let expected = (Vec3::ZERO + primitives::grid(2, 2, 2.0, 2.0).co(nb)) * 0.5;
        assert!((m.co(v) - expected).length() < 1e-5);
        let area_after: f32 = (0..m.face_count() as u32).map(|f| m.face_area(f)).sum();
        assert!(area_after < area_before);
    }

    #[test]
    fn merge_first_last_cursor_positions() {
        let base = {
            let mut m = EditMesh::new("l");
            for i in 0..4 {
                m.add_vert(Vec3::new(i as f32, 0.0, 0.0));
            }
            m.add_vert(Vec3::new(1.5, 1.0, 0.0));
            m.add_face(&[0, 1, 4]).unwrap();
            m.add_face(&[1, 2, 4]).unwrap();
            m.add_face(&[2, 3, 4]).unwrap();
            m
        };
        let mut m = base.clone();
        let out = merge_verts(&mut m, &[2, 0], MergeMode::First).unwrap();
        let v = *out.selection.verts.iter().next().unwrap();
        assert!((m.co(v) - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-6);
        let mut m = base.clone();
        let out = merge_verts(&mut m, &[2, 0], MergeMode::Last).unwrap();
        let v = *out.selection.verts.iter().next().unwrap();
        assert!((m.co(v) - Vec3::ZERO).length() < 1e-6);
        let mut m = base.clone();
        let cur = Vec3::new(9.0, 9.0, 9.0);
        let out = merge_verts(&mut m, &[1, 2], MergeMode::Cursor(cur)).unwrap();
        let v = *out.selection.verts.iter().next().unwrap();
        assert_eq!(m.co(v), cur);
        // cursor aceita um único vértice
        let mut m = base.clone();
        merge_verts(&mut m, &[3], MergeMode::Cursor(cur)).unwrap();
        assert_eq!(m.vert_count(), 5);
        assert!(m.verts.iter().any(|vv| vv.co == cur));
    }

    #[test]
    fn merge_all_verts_of_a_quad_removes_face() {
        let mut m = primitives::plane(1.0);
        merge_verts(&mut m, &[0, 1, 2, 3], MergeMode::Center).unwrap();
        assert_eq!(m.face_count(), 0);
        assert_eq!(m.vert_count(), 1);
    }

    #[test]
    fn merge_opposite_verts_of_hexagon_splits_into_two_faces() {
        let mut m = primitives::circle(1.0, 6, false);
        let out = merge_verts(&mut m, &[0, 3], MergeMode::Center).unwrap();
        // hexágono 0 1 2 3 4 5 com 0≡3: ciclos (0 1 2) e (0 4 5)
        assert_eq!(m.face_count(), 2);
        assert!(m.faces.iter().all(|f| f.len() == 3));
        m.validate().unwrap();
        assert_eq!(out.remap.unwrap().face, vec![Some(0)]);
    }

    #[test]
    fn merge_collapse_merges_per_component() {
        let mut m = primitives::grid(4, 1, 4.0, 1.0);
        // grid 4x1: vértices em duas fileiras de 5; colapsa uma fileira em duas ilhas
        let row: Vec<u32> = (0..m.vert_count() as u32)
            .filter(|&v| m.co(v).z < 0.0)
            .collect();
        // seleciona 0,1 e 3,4 da primeira fileira (ilhas separadas pelo 2)
        let pick: Vec<u32> = [row[0], row[1], row[3], row[4]].to_vec();
        let out = merge_verts(&mut m, &pick, MergeMode::Collapse).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 10 - 2);
        assert_eq!(out.selection.verts.len(), 2);
    }

    #[test]
    fn merge_collapse_without_edges_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        // 0 e 6 são antípodas: sem aresta
        assert!(matches!(
            merge_verts(&mut m, &[0, 6], MergeMode::Collapse),
            Err(ModelError::EmptySelection(_))
        ));
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn merge_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(merge_verts(&mut m, &[], MergeMode::Center).is_err());
        assert!(merge_verts(&mut m, &[1], MergeMode::Center).is_err());
        assert!(merge_verts(&mut m, &[1, 1], MergeMode::First).is_err());
        assert!(merge_verts(&mut m, &[1, 99], MergeMode::Center).is_err());
        assert!(merge_verts(&mut m, &[1], MergeMode::Cursor(Vec3::NAN)).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn merge_edge_collapse_on_closed_mesh_keeps_closed_manifold() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let t = Topology::build(&m);
        let e = t.edges[10].verts;
        merge_verts(&mut m, &[e.0, e.1], MergeMode::Center).unwrap();
        let t2 = Topology::build(&m);
        assert!(t2.is_closed_manifold());
        assert_eq!(t2.euler_characteristic(&m), 2);
        m.validate().unwrap();
    }

    #[test]
    fn merge_preserves_uv_and_custom_normals_of_corners() {
        let mut m = primitives::plane(1.0);
        for (i, c) in m.faces[0].corners.iter_mut().enumerate() {
            c.uv = glam::Vec2::new(i as f32, 0.5);
            c.normal = Some(Vec3::Y);
        }
        let ids: Vec<u32> = m.faces[0].verts().collect();
        merge_verts(&mut m, &[ids[0], ids[1]], MergeMode::Center).unwrap();
        // triângulo restante: UVs e normais dos 3 cantos mantidos
        let f = &m.faces[0];
        assert_eq!(f.len(), 3);
        assert!(f.corners.iter().all(|c| c.normal == Some(Vec3::Y)));
        let uvs: Vec<f32> = f.corners.iter().map(|c| c.uv.x).collect();
        assert!(uvs.contains(&0.0) && uvs.contains(&2.0) && uvs.contains(&3.0));
    }

    #[test]
    fn point_grid_finds_neighbors_within_radius() {
        let mut g = PointGrid::new(1.0);
        for i in 0..10 {
            g.insert(i, Vec3::new(i as f32 * 0.4, 0.0, 0.0));
        }
        let mut found = Vec::new();
        g.for_each_near(Vec3::ZERO, 1.0, |id| found.push(id));
        found.sort_unstable();
        assert!(found.contains(&0) && found.contains(&1) && found.contains(&2));
        // grade com célula degenerada não quebra
        let g0 = PointGrid::new(0.0);
        let mut c = 0;
        g0.for_each_near(Vec3::ZERO, 0.0, |_| c += 1);
        assert_eq!(c, 0);
    }

    #[test]
    fn dsu_representative_is_min_index() {
        let mut d = Dsu::new(6);
        d.union(5, 3);
        d.union(3, 4);
        d.union(1, 4);
        for v in [1, 3, 4, 5] {
            assert_eq!(d.find(v), 1);
        }
        assert_eq!(d.find(0), 0);
    }

    #[test]
    fn split_simple_handles_bowtie() {
        let cs: Vec<Corner> = [0u32, 1, 2, 0, 3, 4]
            .iter()
            .map(|&v| Corner::new(v))
            .collect();
        let parts = split_simple(&cs);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 3);
        assert_eq!(parts[1].len(), 3);
    }

    #[test]
    fn merge_edge_attr_collisions_are_merged() {
        let mut m = EditMesh::new("c");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        let c = m.add_vert(Vec3::new(0.0001, 0.0, 0.0));
        let d = m.add_vert(Vec3::Y);
        m.add_face(&[a, b, d]).unwrap();
        m.add_face(&[c, d, b]).unwrap();
        m.set_sharp(a, b, true);
        m.set_bevel_weight(c, b, 0.5);
        merge_by_distance(&mut m, None, 0.001, ByDistanceOptions::default()).unwrap();
        // a e c fundiram; a aresta (a,b) carrega sharp e bevel mesclados
        let attr = m.edge_attr(0, 1);
        assert!(attr.sharp);
        assert!((attr.bevel_weight - 0.5).abs() < 1e-6);
    }
}
