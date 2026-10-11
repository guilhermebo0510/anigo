//! Extrude (região, individual, arestas, vértices) — equivalente ao `E` do Blender.
//!
//! Todas as funções devolvem a "tampa" extrudada como seleção, pronta para o
//! usuário transladar (por exemplo, ao longo da normal). Atributos que viajam:
//! UV/normal por corner (copiados nas tampas, herdados da vizinhança nas
//! paredes), material/suave/face-set por face e sharp/seam/crease/bevel por aresta.
//!
//! Este módulo também hospeda helpers `pub(crate)` compartilhados pelos demais
//! operadores de A1 (`inset`, `bevel`, `loops`, ...): validação de faces,
//! manutenção de atributos de aresta e deslocamento de UV.
//!
//! # Arestas soltas (wire)
//! Uma aresta sem face só existe como entrada em `EditMesh::edge_attrs` (mesmo
//! com valor padrão). Por isso todo operador que remove/substitui faces precisa
//! podar entradas "órfãs" — ver [`prune_orphan_edge_attrs`].

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EdgeAttr, EdgeKey, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

// ------------------------------------------------------------------ helpers

/// Valida índices de face, ordena e remove duplicatas. Vazio → `EmptySelection`.
pub(crate) fn validated_faces(
    mesh: &EditMesh,
    faces: &[u32],
    what: &'static str,
) -> ModelResult<Vec<u32>> {
    let mut out: Vec<u32> = Vec::with_capacity(faces.len());
    for &f in faces {
        if f as usize >= mesh.faces.len() {
            return Err(ModelError::FaceOutOfRange(f, mesh.faces.len()));
        }
        out.push(f);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(ModelError::EmptySelection(what));
    }
    Ok(out)
}

/// Valida índices de vértice, ordena e remove duplicatas.
pub(crate) fn validated_verts(
    mesh: &EditMesh,
    verts: &[u32],
    what: &'static str,
) -> ModelResult<Vec<u32>> {
    let mut out: Vec<u32> = Vec::with_capacity(verts.len());
    for &v in verts {
        if v as usize >= mesh.verts.len() {
            return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
        }
        out.push(v);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(ModelError::EmptySelection(what));
    }
    Ok(out)
}

/// Todas as chaves de aresta usadas por alguma face.
pub(crate) fn face_edge_set(mesh: &EditMesh) -> HashSet<EdgeKey> {
    let mut set = HashSet::new();
    for f in &mesh.faces {
        for (a, b) in f.edges() {
            set.insert(edge_key(a, b));
        }
    }
    set
}

/// Arestas soltas (wire) atuais: chaves de `edge_attrs` que nenhuma face usa.
pub(crate) fn wire_edge_keys(mesh: &EditMesh) -> BTreeSet<EdgeKey> {
    let used = face_edge_set(mesh);
    mesh.edge_attrs
        .keys()
        .copied()
        .filter(|k| !used.contains(k))
        .collect()
}

/// Remove de `edge_attrs` as entradas que não pertencem a nenhuma face nem
/// estão em `keep_wire` (arestas soltas legítimas anteriores à operação).
pub(crate) fn prune_orphan_edge_attrs(mesh: &mut EditMesh, keep_wire: &BTreeSet<EdgeKey>) {
    let used = face_edge_set(mesh);
    mesh.edge_attrs
        .retain(|k, _| used.contains(k) || keep_wire.contains(k));
}

/// UV do corner de `v` dentro de `face` (primeira ocorrência).
pub(crate) fn corner_uv(face: &Face, v: u32) -> Option<Vec2> {
    face.corners.iter().find(|c| c.vert == v).map(|c| c.uv)
}

/// Índice do corner de `v` em `face`, apenas se `v` aparece exatamente uma vez.
pub(crate) fn unique_corner_index(face: &Face, v: u32) -> Option<usize> {
    let mut found = None;
    for (i, c) in face.corners.iter().enumerate() {
        if c.vert == v {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    found
}

/// UV que o corner `i` teria se sua posição 3D passasse a `new_pos`, assumindo
/// um mapeamento afim local definido pelos dois vizinhos do corner.
pub(crate) fn uv_at_shifted(pts: &[Vec3], uvs: &[Vec2], i: usize, new_pos: Vec3) -> Vec2 {
    let n = pts.len();
    if n < 3 {
        return uvs.get(i).copied().unwrap_or(Vec2::ZERO);
    }
    let (nx, pv) = ((i + 1) % n, (i + n - 1) % n);
    let p = pts[i];
    let e1 = pts[nx] - p;
    let e2 = pts[pv] - p;
    let u1 = uvs[nx] - uvs[i];
    let u2 = uvs[pv] - uvs[i];
    let d = new_pos - p;
    let (g11, g12, g22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = g11 * g22 - g12 * g12;
    if det.abs() <= 1e-18 * (g11 * g22).max(1e-30) {
        return uvs[i];
    }
    let (r1, r2) = (d.dot(e1), d.dot(e2));
    let alpha = (r1 * g22 - r2 * g12) / det;
    let beta = (g11 * r2 - g12 * r1) / det;
    let uv = uvs[i] + u1 * alpha + u2 * beta;
    if uv.is_finite() {
        uv
    } else {
        uvs[i]
    }
}

/// Posições e UVs dos corners de uma face (snapshot).
pub(crate) fn face_pts_uvs(positions: &[Vec3], face: &Face) -> (Vec<Vec3>, Vec<Vec2>) {
    (
        face.corners
            .iter()
            .map(|c| positions[c.vert as usize])
            .collect(),
        face.corners.iter().map(|c| c.uv).collect(),
    )
}

/// Interpola dois corners (posição `t` de `a` para `b`) já com o vértice dado.
pub(crate) fn lerp_corner(a: &Corner, b: &Corner, t: f32, vert: u32) -> Corner {
    let normal = match (a.normal, b.normal) {
        (Some(na), Some(nb)) => {
            let n = na.lerp(nb, t);
            Some(n.try_normalize().unwrap_or(na))
        }
        _ => None,
    };
    Corner {
        vert,
        uv: a.uv.lerp(b.uv, t),
        normal,
    }
}

/// Soma das normais de face ponderadas por área (direção "média" da região).
fn region_normal(mesh: &EditMesh, faces: &[u32]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for &f in faces {
        n += mesh.face_normal(f) * mesh.face_area(f);
    }
    n.try_normalize().unwrap_or(Vec3::ZERO)
}

/// Primeira face (≠ `exclude`) que usa a aresta, preferindo as que não estão em `avoid`.
fn neighbor_face(
    topo: &Topology,
    a: u32,
    b: u32,
    exclude: u32,
    avoid: &dyn Fn(u32) -> bool,
) -> Option<u32> {
    let faces = topo.edge_faces(a, b);
    faces
        .iter()
        .copied()
        .find(|&g| g != exclude && !avoid(g))
        .or_else(|| faces.iter().copied().find(|&g| g != exclude))
}

// ------------------------------------------------------------ extrude region

/// Extrude de região (`E` com faces selecionadas, "Extrude Region").
///
/// * `faces`: faces da região (índices; duplicatas são ignoradas).
///
/// Duplica os vértices da fronteira da região (e os que tocam faces fora dela
/// ou arestas soltas), cria uma parede de quads por aresta de fronteira da
/// região e **mantém a região conectada**: arestas internas não geram parede.
/// As faces da região mudam de vértices *no próprio índice* (a tampa), de modo
/// que a malha continua fechada/manifold. Em malha aberta, arestas da região
/// que também são fronteira da malha geram parede (como no Blender).
///
/// Se a região não tem fronteira (todas as faces de uma casca fechada), nada é
/// duplicado — o resultado é a própria malha, como no Blender, que descarta o
/// "miolo" interno.
///
/// UV/normal por corner da tampa são copiados; nas paredes, a aresta antiga usa
/// a UV da face vizinha (continuidade) e a aresta nova usa a UV da tampa.
///
/// Seleção devolvida: a tampa (`faces` + seus vértices e arestas). `created_faces`
/// lista as paredes, `created_verts` os vértices duplicados.
pub fn extrude_faces_region(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let region = validated_faces(
        mesh,
        faces,
        "extrude de região precisa de ao menos uma face",
    )?;
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);
    let mut in_region = vec![false; mesh.faces.len()];
    for &f in &region {
        in_region[f as usize] = true;
    }
    // contagem de faces da região por aresta
    let mut region_count: HashMap<u32, u32> = HashMap::new();
    for &f in &region {
        for &e in &topo.face_edges[f as usize] {
            *region_count.entry(e).or_insert(0) += 1;
        }
    }
    // paredes: (face, índice do corner de a)
    let mut walls: Vec<(u32, usize)> = Vec::new();
    let mut need_dup = vec![false; mesh.verts.len()];
    for &f in &region {
        let face = &mesh.faces[f as usize];
        for (i, (a, b)) in face.edges().enumerate() {
            let e = topo.face_edges[f as usize][i];
            if region_count.get(&e).copied().unwrap_or(0) == 1 {
                walls.push((f, i));
                need_dup[a as usize] = true;
                need_dup[b as usize] = true;
            }
        }
    }
    for &f in &region {
        for v in mesh.faces[f as usize].verts() {
            let touches_outside = topo.vert_faces[v as usize]
                .iter()
                .any(|&g| !in_region[g as usize]);
            let has_wire = topo.vert_edges[v as usize]
                .iter()
                .any(|&e| topo.edges[e as usize].faces.is_empty());
            if touches_outside || has_wire {
                need_dup[v as usize] = true;
            }
        }
    }

    // ---- mutação
    let mut dup: HashMap<u32, u32> = HashMap::new();
    let mut created_verts = Vec::new();
    for (v, need) in need_dup.iter().enumerate() {
        if *need {
            let nv = mesh.push_vert(mesh.verts[v].clone());
            dup.insert(v as u32, nv);
            created_verts.push(nv);
        }
    }
    let snapshot: Vec<Face> = region
        .iter()
        .map(|&f| mesh.faces[f as usize].clone())
        .collect();
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for (idx, &f) in region.iter().enumerate() {
        let old = &snapshot[idx];
        let face = &mut mesh.faces[f as usize];
        for c in &mut face.corners {
            if let Some(&nv) = dup.get(&c.vert) {
                c.vert = nv;
            }
        }
        for (a, b) in old.edges() {
            let (na, nb) = (
                dup.get(&a).copied().unwrap_or(a),
                dup.get(&b).copied().unwrap_or(b),
            );
            if (na, nb) != (a, b) {
                let attr = mesh.edge_attr(a, b);
                if !attr.is_default() {
                    attr_moves.push((edge_key(na, nb), attr));
                }
            }
        }
    }
    let mut created_faces = Vec::new();
    for &(f, i) in &walls {
        let old = &snapshot[region.binary_search(&f).unwrap_or(0)];
        let n = old.corners.len();
        let (ca, cb) = (old.corners[i], old.corners[(i + 1) % n]);
        let (a, b) = (ca.vert, cb.vert);
        let (na, nb) = (dup[&a], dup[&b]);
        let (ua, ub) = match neighbor_face(&topo, a, b, f, &|g| in_region[g as usize]) {
            Some(g) => {
                let gf = &mesh.faces[g as usize];
                (
                    corner_uv(gf, a).unwrap_or(ca.uv),
                    corner_uv(gf, b).unwrap_or(cb.uv),
                )
            }
            None => (ca.uv, cb.uv),
        };
        let mut wall = old.with_verts_from(&[a, b, nb, na]);
        wall.corners[0].uv = ua;
        wall.corners[1].uv = ub;
        wall.corners[2].uv = cb.uv;
        wall.corners[3].uv = ca.uv;
        mesh.faces.push(wall);
        created_faces.push((mesh.faces.len() - 1) as u32);
    }
    for (k, attr) in attr_moves {
        mesh.edge_attrs.insert(k, attr);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, region.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

/// Conveniência: extrude de região seguido de translação da tampa por
/// `distance` ao longo da normal média (ponderada por área) da região —
/// equivale ao "Extrude Region and Move" com orientação *Normal*.
///
/// `distance` pode ser negativo (extrude para dentro). Falha com `Numeric` se
/// a normal média da região for nula (ex.: casca fechada inteira) e com
/// `InvalidParameter` se `distance` não for finito. A malha fica intacta em erro.
pub fn extrude_faces_region_along_normal(
    mesh: &mut EditMesh,
    faces: &[u32],
    distance: f32,
) -> ModelResult<OpOutput> {
    if !distance.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "distância de extrude não finita: {distance}"
        )));
    }
    let region = validated_faces(
        mesh,
        faces,
        "extrude de região precisa de ao menos uma face",
    )?;
    let normal = region_normal(mesh, &region);
    if normal == Vec3::ZERO {
        return Err(ModelError::Numeric(
            "normal média da região é nula; não há direção de extrusão".into(),
        ));
    }
    let out = extrude_faces_region(mesh, &region)?;
    for &v in &out.selection.verts {
        mesh.verts[v as usize].co += normal * distance;
    }
    Ok(out)
}

// -------------------------------------------------------- extrude individual

/// Extrude individual: cada face vira um prisma independente (paredes por todas
/// as arestas, inclusive entre faces selecionadas adjacentes).
///
/// Cada face recebe vértices próprios (a tampa não compartilha vértices com a
/// tampa vizinha); os vértices originais permanecem para as faces não
/// selecionadas. Seleção devolvida: as tampas.
pub fn extrude_faces_individual(mesh: &mut EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let region = validated_faces(
        mesh,
        faces,
        "extrude individual precisa de ao menos uma face",
    )?;
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);
    let selected: HashSet<u32> = region.iter().copied().collect();

    let mut created_verts = Vec::new();
    let mut created_faces = Vec::new();
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for &f in &region {
        let old = mesh.faces[f as usize].clone();
        let mut dup: HashMap<u32, u32> = HashMap::new();
        for c in &old.corners {
            dup.entry(c.vert).or_insert_with(|| {
                let nv = mesh.push_vert(mesh.verts[c.vert as usize].clone());
                created_verts.push(nv);
                nv
            });
        }
        for c in &mut mesh.faces[f as usize].corners {
            c.vert = dup[&c.vert];
        }
        let n = old.corners.len();
        for i in 0..n {
            let (ca, cb) = (old.corners[i], old.corners[(i + 1) % n]);
            let (a, b) = (ca.vert, cb.vert);
            let (na, nb) = (dup[&a], dup[&b]);
            let attr = mesh.edge_attr(a, b);
            if !attr.is_default() {
                attr_moves.push((edge_key(na, nb), attr));
            }
            let (ua, ub) = match neighbor_face(&topo, a, b, f, &|g| selected.contains(&g)) {
                Some(g) => {
                    let gf = &mesh.faces[g as usize];
                    (
                        corner_uv(gf, a).unwrap_or(ca.uv),
                        corner_uv(gf, b).unwrap_or(cb.uv),
                    )
                }
                None => (ca.uv, cb.uv),
            };
            let mut wall = old.with_verts_from(&[a, b, nb, na]);
            wall.corners[0].uv = ua;
            wall.corners[1].uv = ub;
            wall.corners[2].uv = cb.uv;
            wall.corners[3].uv = ca.uv;
            mesh.faces.push(wall);
            created_faces.push((mesh.faces.len() - 1) as u32);
        }
    }
    for (k, attr) in attr_moves {
        mesh.edge_attrs.insert(k, attr);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, region.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

// ------------------------------------------------------------- extrude edges

/// Extrude de arestas: duplica os vértices das arestas e cria um quad por
/// aresta ligando a original à cópia. Funciona com arestas de fronteira
/// (a parede continua a superfície, com enrolamento consistente), arestas
/// internas (cria uma "aba", deixando a aresta com 3 faces — igual ao Blender)
/// e arestas **soltas** (cria faces novas a partir de uma polilinha).
///
/// O enrolamento das paredes de arestas soltas é propagado ao longo das
/// cadeias de arestas para que a faixa gerada seja coerente.
///
/// Falha com `MissingEdge` se alguma aresta não existe. Seleção devolvida: as
/// novas arestas (cópias) com seus vértices.
pub fn extrude_edges(mesh: &mut EditMesh, edges: &[EdgeKey]) -> ModelResult<OpOutput> {
    let mut set: BTreeSet<EdgeKey> = BTreeSet::new();
    for &(a, b) in edges {
        for v in [a, b] {
            if v as usize >= mesh.verts.len() {
                return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
            }
        }
        set.insert(edge_key(a, b));
    }
    if set.is_empty() {
        return Err(ModelError::EmptySelection(
            "extrude de arestas precisa de ao menos uma aresta",
        ));
    }
    let topo = Topology::build(mesh);
    for &(a, b) in &set {
        if a == b || topo.edge(a, b).is_none() {
            return Err(ModelError::MissingEdge(a, b));
        }
    }
    let wires = wire_edge_keys(mesh);

    // orientação (p -> q) de cada parede: [p, q, q', p']
    let mut dir: BTreeMap<EdgeKey, (u32, u32)> = BTreeMap::new();
    for &(a, b) in &set {
        let faces = topo.edge_faces(a, b);
        if let Some(&f0) = faces.first() {
            let face = &mesh.faces[f0 as usize];
            if let Some((x, y)) = face.edges().find(|&(x, y)| edge_key(x, y) == (a, b)) {
                // a parede percorre a aresta no sentido oposto ao da face
                dir.insert((a, b), (y, x));
            }
        }
    }
    // propaga orientação para as arestas sem face (BFS determinístico)
    let mut by_vert: HashMap<u32, Vec<EdgeKey>> = HashMap::new();
    for &k in &set {
        by_vert.entry(k.0).or_default().push(k);
        by_vert.entry(k.1).or_default().push(k);
    }
    let mut queue: VecDeque<EdgeKey> = dir.keys().copied().collect();
    let propagate = |queue: &mut VecDeque<EdgeKey>, dir: &mut BTreeMap<EdgeKey, (u32, u32)>| {
        while let Some(k) = queue.pop_front() {
            let (p, q) = dir[&k];
            for &(vtx, is_head) in &[(q, true), (p, false)] {
                for &k2 in by_vert.get(&vtx).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if dir.contains_key(&k2) {
                        continue;
                    }
                    let other = if k2.0 == vtx { k2.1 } else { k2.0 };
                    // continuação do caminho: cabeça de uma é cauda da outra
                    let d = if is_head { (vtx, other) } else { (other, vtx) };
                    dir.insert(k2, d);
                    queue.push_back(k2);
                }
            }
        }
    };
    propagate(&mut queue, &mut dir);
    for &k in &set {
        if !dir.contains_key(&k) {
            dir.insert(k, (k.0, k.1));
            queue.push_back(k);
            propagate(&mut queue, &mut dir);
        }
    }

    // ---- mutação
    let mut verts: BTreeSet<u32> = BTreeSet::new();
    for &(a, b) in &set {
        verts.insert(a);
        verts.insert(b);
    }
    let mut dup: HashMap<u32, u32> = HashMap::new();
    let mut created_verts = Vec::new();
    for &v in &verts {
        let nv = mesh.push_vert(mesh.verts[v as usize].clone());
        dup.insert(v, nv);
        created_verts.push(nv);
    }
    let mut created_faces = Vec::new();
    let mut new_edges = Vec::new();
    for &k in &set {
        let (p, q) = dir[&k];
        let (np, nq) = (dup[&p], dup[&q]);
        let faces = topo.edge_faces(k.0, k.1);
        let (mut face, up, uq) = match faces.first() {
            Some(&f0) => {
                let src = &mesh.faces[f0 as usize];
                (
                    src.with_verts_from(&[p, q, nq, np]),
                    corner_uv(src, p).unwrap_or(Vec2::ZERO),
                    corner_uv(src, q).unwrap_or(Vec2::ZERO),
                )
            }
            None => (Face::new(&[p, q, nq, np]), Vec2::ZERO, Vec2::ZERO),
        };
        face.corners[0].uv = up;
        face.corners[1].uv = uq;
        face.corners[2].uv = uq;
        face.corners[3].uv = up;
        mesh.faces.push(face);
        created_faces.push((mesh.faces.len() - 1) as u32);
        let attr = mesh.edge_attr(p, q);
        if !attr.is_default() {
            mesh.edge_attrs.insert(edge_key(np, nq), attr);
        }
        new_edges.push(edge_key(np, nq));
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_edges(new_edges),
        created_verts,
        created_faces,
        remap: None,
    })
}

// ------------------------------------------------------------- extrude verts

/// Extrude de vértices: cada vértice ganha uma cópia ligada por uma **aresta
/// solta** nova (registrada em `edge_attrs` com valor padrão). Seleção
/// devolvida: os vértices novos (para o usuário mover).
pub fn extrude_verts(mesh: &mut EditMesh, verts: &[u32]) -> ModelResult<OpOutput> {
    let verts = validated_verts(
        mesh,
        verts,
        "extrude de vértices precisa de ao menos um vértice",
    )?;
    let mut created = Vec::new();
    for &v in &verts {
        let nv = mesh.push_vert(mesh.verts[v as usize].clone());
        mesh.edge_attrs.insert(edge_key(v, nv), EdgeAttr::default());
        created.push(nv);
    }
    Ok(OpOutput {
        selection: Selection::from_verts(created.iter().copied()),
        created_verts: created,
        created_faces: Vec::new(),
        remap: None,
    })
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
pub(crate) mod test_util {
    use crate::mesh::EditMesh;
    use crate::topology::Topology;
    use glam::Vec3;

    /// Volume com sinal (positivo se as normais apontam para fora).
    pub fn signed_volume(m: &EditMesh) -> f32 {
        let mut vol = 0.0;
        for f in &m.faces {
            let p: Vec<Vec3> = f
                .corners
                .iter()
                .map(|c| m.verts[c.vert as usize].co)
                .collect();
            for i in 1..p.len() - 1 {
                vol += p[0].dot(p[i].cross(p[i + 1]));
            }
        }
        vol / 6.0
    }

    /// Fechada, 2-manifold, válida e (opcionalmente) com Euler esperado.
    pub fn assert_closed(m: &EditMesh, euler: Option<i64>) {
        m.validate().expect("malha válida");
        let t = Topology::build(m);
        assert!(
            t.is_closed_manifold(),
            "esperava malha fechada manifold (bordas={}, não-manifold={})",
            t.boundary_edge_count(),
            t.non_manifold_edge_count()
        );
        if let Some(e) = euler {
            assert_eq!(t.euler_characteristic(m), e, "característica de Euler");
        }
        let used = m.used_verts();
        assert!(used.iter().all(|u| *u), "há vértices órfãos");
        // arestas soltas não devem existir
        assert!(
            m.edge_attrs
                .keys()
                .all(|k| t.edge(k.0, k.1).is_some_and(|e| !e.faces.is_empty())),
            "attrs de aresta órfãos viraram wire edges"
        );
    }

    /// Cada face aponta para longe do centroide (vale para formas convexas/estreladas).
    pub fn assert_outward_star(m: &EditMesh) {
        let c = m.centroid();
        for f in 0..m.face_count() as u32 {
            let n = m.face_normal(f);
            assert!(
                n.dot(m.face_center(f) - c) > 0.0,
                "face {f} aponta para dentro"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use crate::primitives;

    fn top_face(m: &EditMesh) -> u32 {
        (0..m.face_count() as u32)
            .max_by(|&a, &b| m.face_normal(a).y.partial_cmp(&m.face_normal(b).y).unwrap())
            .unwrap()
    }

    #[test]
    fn cube_face_extrude_is_closed_with_12_verts_10_faces() {
        let mut m = primitives::cube(2.0);
        let f = top_face(&m);
        let out = extrude_faces_region(&mut m, &[f]).unwrap();
        assert_eq!(m.vert_count(), 12);
        assert_eq!(m.face_count(), 10);
        assert_eq!(out.created_verts.len(), 4);
        assert_eq!(out.created_faces.len(), 4);
        assert_eq!(out.selection.faces.len(), 1);
        assert!(out.selection.faces.contains(&f));
        assert_closed(&m, Some(2));
        assert!(
            (signed_volume(&m) - 8.0).abs() < 1e-4,
            "ainda sem mover: volume igual"
        );
        for &v in &out.selection.verts {
            m.verts[v as usize].co.y += 0.5;
        }
        assert_outward_star(&m);
    }

    #[test]
    fn extrude_along_normal_grows_volume_and_stays_outward() {
        let mut m = primitives::cube(2.0);
        let f = top_face(&m);
        let out = extrude_faces_region_along_normal(&mut m, &[f], 1.0).unwrap();
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
        assert!((signed_volume(&m) - 12.0).abs() < 1e-4);
        for &v in &out.selection.verts {
            assert!((m.co(v).y - 2.0).abs() < 1e-5);
        }
        // para dentro
        let mut m2 = primitives::cube(2.0);
        let f2 = top_face(&m2);
        extrude_faces_region_along_normal(&mut m2, &[f2], -0.5).unwrap();
        assert!((signed_volume(&m2) - 6.0).abs() < 1e-4);
        assert_closed(&m2, Some(2));
    }

    #[test]
    fn region_of_two_faces_has_no_wall_on_shared_edge() {
        let mut m = primitives::cube_subdivided(2.0, 2);
        // duas faces vizinhas na face +Y
        let top: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_normal(f).y > 0.9)
            .collect();
        assert_eq!(top.len(), 4);
        let sel = vec![top[0], top[1]];
        let topo = Topology::build(&m);
        assert!(topo.face_neighbors(top[0]).contains(&top[1]));
        let before_f = m.face_count();
        let out = extrude_faces_region(&mut m, &sel).unwrap();
        // região 2x1 quads: perímetro 6 arestas -> 6 paredes
        assert_eq!(out.created_faces.len(), 6);
        assert_eq!(m.face_count(), before_f + 6);
        assert_closed(&m, Some(2));
        // a aresta compartilhada entre as duas tampas continua com 2 faces
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
    }

    #[test]
    fn region_extrude_whole_strip_keeps_interior_vertices_shared() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let center = (0..m.face_count() as u32)
            .find(|&f| m.face_center(f).length() < 1e-4)
            .unwrap();
        // região 3x3 inteira com fronteira = fronteira da malha (use parede)
        let all: Vec<u32> = (0..m.face_count() as u32).collect();
        let _ = center;
        let out = extrude_faces_region(&mut m, &all).unwrap();
        // 12 arestas de fronteira -> 12 paredes; vértices internos (4) não duplicam
        assert_eq!(out.created_faces.len(), 12);
        assert_eq!(out.created_verts.len(), 12);
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.boundary_edge_count(), 12);
    }

    #[test]
    fn closed_shell_full_selection_is_a_noop_topologically() {
        let mut m = primitives::cube(1.0);
        let all: Vec<u32> = (0..6).collect();
        let h = m.content_hash();
        let out = extrude_faces_region(&mut m, &all).unwrap();
        assert!(out.created_faces.is_empty() && out.created_verts.is_empty());
        assert_eq!(h, m.content_hash());
        // e a conveniência falha limpo (normal média nula)
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            extrude_faces_region_along_normal(&mut m, &all, 1.0),
            Err(ModelError::Numeric(_))
        ));
        assert_eq!(h, m.content_hash());
    }

    #[test]
    fn extrude_propagates_uv_material_smooth_and_edge_attrs() {
        let mut m = primitives::cube(2.0);
        let f = top_face(&m);
        m.faces[f as usize].material = 3;
        m.faces[f as usize].face_set = 7;
        let verts: Vec<u32> = m.faces[f as usize].verts().collect();
        let uv_before: Vec<Vec2> = m.faces[f as usize].corners.iter().map(|c| c.uv).collect();
        m.set_seam(verts[0], verts[1], true);
        m.set_sharp(verts[1], verts[2], true);
        let out = extrude_faces_region(&mut m, &[f]).unwrap();
        // tampa: mesmo material/face_set, UV idênticas
        let cap = &m.faces[f as usize];
        assert_eq!((cap.material, cap.face_set), (3, 7));
        for (c, u) in cap.corners.iter().zip(&uv_before) {
            assert_eq!(c.uv, *u);
        }
        // paredes herdam material do lado da região
        for &w in &out.created_faces {
            assert_eq!(m.faces[w as usize].material, 3);
            assert!(m.faces[w as usize].corners.iter().all(|c| c.uv.is_finite()));
        }
        // atributos de aresta copiados para a tampa
        let cv: Vec<u32> = cap.verts().collect();
        assert!(m.edge_attr(cv[0], cv[1]).seam);
        assert!(m.edge_attr(cv[1], cv[2]).sharp);
        // as originais continuam
        assert!(m.edge_attr(verts[0], verts[1]).seam);
        assert_closed(&m, Some(2));
    }

    #[test]
    fn region_extrude_on_sphere_patch_stays_closed_and_outward() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let seed = 0u32;
        let sel = crate::select::grow_faces(&m, &Selection::from_faces(&m, [seed]));
        let sel2 = crate::select::grow_faces(&m, &sel);
        let faces: Vec<u32> = sel2.faces.iter().copied().collect();
        let n_faces = faces.len();
        let before = m.face_count();
        extrude_faces_region_along_normal(&mut m, &faces, 0.2).unwrap();
        assert_closed(&m, Some(2));
        assert!(m.face_count() > before + n_faces / 2);
        assert!(signed_volume(&m) > 0.0);
    }

    #[test]
    fn individual_extrude_makes_independent_prisms() {
        let mut m = primitives::cube_subdivided(2.0, 2);
        let top: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_normal(f).y > 0.9)
            .collect();
        let out = extrude_faces_individual(&mut m, &top).unwrap();
        assert_eq!(out.created_faces.len(), 16);
        assert_eq!(out.created_verts.len(), 16);
        assert_closed(&m, None);
        let t = Topology::build(&m);
        assert_eq!(t.euler_characteristic(&m), 2);
        // as tampas não compartilham vértices entre si
        let mut seen = HashSet::new();
        for &f in &top {
            for v in m.faces[f as usize].verts() {
                assert!(seen.insert(v), "vértice {v} compartilhado entre tampas");
            }
        }
        for &v in out.selection.verts.iter() {
            m.verts[v as usize].co.y += 0.5;
        }
        // 4 prismas de 1 x 1 x 0.5 sobre o cubo de volume 8
        assert!((signed_volume(&m) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn extrude_edges_on_open_boundary_continues_surface() {
        let mut m = primitives::cylinder(1.0, 2.0, 8, 1, false);
        let topo = Topology::build(&m);
        let boundary: Vec<EdgeKey> = topo
            .edges
            .iter()
            .filter(|e| e.faces.len() == 1)
            .map(|e| e.verts)
            .filter(|&(a, b)| m.co(a).y > 0.0 && m.co(b).y > 0.0)
            .collect();
        assert_eq!(boundary.len(), 8);
        let out = extrude_edges(&mut m, &boundary).unwrap();
        assert_eq!(out.created_faces.len(), 8);
        assert_eq!(out.created_verts.len(), 8);
        assert_eq!(out.selection.edges.len(), 8);
        // sobe o novo anel: cilindro mais alto, ainda aberto com 2 bordas e normais para fora
        for &v in &out.selection.verts {
            m.verts[v as usize].co.y += 1.0;
        }
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.boundary_edge_count(), 16);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // as normais das paredes apontam para fora do eixo
        for &w in &out.created_faces {
            let n = m.face_normal(w);
            let c = m.face_center(w);
            assert!(n.dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
        }
    }

    #[test]
    fn extrude_wire_edges_creates_a_consistent_strip() {
        let mut m = EditMesh::new("wire");
        let pts = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ];
        let vs: Vec<u32> = pts.iter().map(|&p| m.add_vert(p)).collect();
        // ordem embaralhada de propósito
        for (a, b) in [(vs[1], vs[2]), (vs[0], vs[1]), (vs[3], vs[2])] {
            m.edge_attrs.insert(edge_key(a, b), EdgeAttr::default());
        }
        let edges: Vec<EdgeKey> = m.edge_attrs.keys().copied().collect();
        let out = extrude_edges(&mut m, &edges).unwrap();
        assert_eq!(out.created_faces.len(), 3);
        for v in out.created_verts.clone() {
            m.verts[v as usize].co.z += 1.0;
        }
        // todas as normais iguais (faixa coerente)
        let n0 = m.face_normal(0);
        for f in 1..3u32 {
            assert!(
                m.face_normal(f).dot(n0) > 0.99,
                "normal inconsistente na faixa"
            );
        }
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        // as arestas soltas originais continuam existindo como arestas da faixa
        m.validate().unwrap();
    }

    #[test]
    fn extrude_interior_edge_makes_a_fin() {
        let mut m = primitives::grid(2, 1, 2.0, 1.0);
        let topo = Topology::build(&m);
        let interior: Vec<EdgeKey> = topo
            .edges
            .iter()
            .filter(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .collect();
        assert_eq!(interior.len(), 1);
        extrude_edges(&mut m, &interior).unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.edge(interior[0].0, interior[0].1).unwrap().faces.len(), 3);
        m.validate().unwrap();
    }

    #[test]
    fn extrude_verts_creates_wire_edges() {
        let mut m = primitives::cube(1.0);
        let out = extrude_verts(&mut m, &[0, 3, 3]).unwrap();
        assert_eq!(out.created_verts.len(), 2);
        assert_eq!(m.vert_count(), 10);
        let t = Topology::build(&m);
        assert!(t.is_wire_edge(0, out.created_verts[0]));
        assert!(t.is_wire_edge(3, out.created_verts[1]));
        assert_eq!(out.selection.verts.len(), 2);
        m.validate().unwrap();
        // idempotência de determinismo
        let mut m2 = primitives::cube(1.0);
        extrude_verts(&mut m2, &[0, 3, 3]).unwrap();
        assert_eq!(m.content_hash(), m2.content_hash());
    }

    #[test]
    fn invalid_inputs_leave_mesh_untouched() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(matches!(
            extrude_faces_region(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            extrude_faces_region(&mut m, &[0, 99]),
            Err(ModelError::FaceOutOfRange(99, 6))
        ));
        assert!(matches!(
            extrude_faces_individual(&mut m, &[77]),
            Err(ModelError::FaceOutOfRange(77, 6))
        ));
        assert!(matches!(
            extrude_edges(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            extrude_edges(&mut m, &[(0, 99)]),
            Err(ModelError::VertexOutOfRange(99, 8))
        ));
        // 0 e 6 não formam aresta no cubo (diagonais): confira que existe algum par ausente
        let topo = Topology::build(&m);
        let missing = (0..8u32)
            .flat_map(|a| (a + 1..8).map(move |b| (a, b)))
            .find(|&(a, b)| topo.edge(a, b).is_none())
            .unwrap();
        assert!(matches!(
            extrude_edges(&mut m, &[missing]),
            Err(ModelError::MissingEdge(_, _))
        ));
        assert!(matches!(
            extrude_verts(&mut m, &[8]),
            Err(ModelError::VertexOutOfRange(8, 8))
        ));
        assert!(matches!(
            extrude_verts(&mut m, &[]),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            extrude_faces_region_along_normal(&mut m, &[0], f32::NAN),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(h, m.content_hash());
    }

    #[test]
    fn extrude_is_deterministic() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 4);
            let faces: Vec<u32> = (0..10).collect();
            extrude_faces_region_along_normal(&mut m, &faces, 0.1).unwrap();
            m.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn wire_edges_and_vertex_with_wire_are_preserved_on_region_extrude() {
        let mut m = primitives::cube(2.0);
        let f = top_face(&m);
        let v0 = m.faces[f as usize].corners[0].vert;
        let out = extrude_verts(&mut m, &[v0]).unwrap();
        let wire_v = out.created_verts[0];
        extrude_faces_region(&mut m, &[f]).unwrap();
        let t = Topology::build(&m);
        // a aresta solta continua solta e válida
        assert!(t.is_wire_edge(v0, wire_v));
        m.validate().unwrap();
    }

    #[test]
    fn large_mesh_sanity() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let faces: Vec<u32> = (0..m.face_count() as u32).step_by(7).collect();
        let t0 = std::time::Instant::now();
        extrude_faces_individual(&mut m, &faces).unwrap();
        let all: Vec<u32> = (0..2000).collect();
        extrude_faces_region(&mut m, &all).unwrap();
        m.validate().unwrap();
        assert!(t0.elapsed().as_secs() < 30, "lento: {:?}", t0.elapsed());
    }
}
