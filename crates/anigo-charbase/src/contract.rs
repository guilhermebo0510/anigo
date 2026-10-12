//! Contrato de topologia: o que o usuário pode esculpir (posições) e o que fica congelado
//! (índices e conectividade). `topology_contract.json` descreve contagens, hash da conectividade,
//! grupos, laços, landmarks, ossos (localizadores) e peças; `validate_topology` confere uma malha
//! importada contra ele; `rederive` recalcula esqueleto, landmarks e pesos a partir de posições
//! esculpidas (e, se a topologia mudou, transfere por vizinho mais próximo).

use crate::character::skin_positions;
use crate::expressions::expression_specs;
use crate::morphs::shape_defs;
use crate::parts::parts_topology;
use crate::qmesh::QuadMesh;
use crate::skeleton::{bone_defs, Skeleton};
use crate::topology::{hash_faces, reference, reference_params, Topology, ZONES};
use crate::uv::{chart_rect, CHART_NAMES};
use glam::Vec3;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const CONTRACT_VERSION: u32 = 1;

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

/// Posições da geração de referência (neutra), usadas para o casamento por vizinho mais próximo.
pub fn reference_positions() -> &'static Vec<Vec3> {
    static P: OnceLock<Vec<Vec3>> = OnceLock::new();
    P.get_or_init(|| skin_positions(&reference_params()))
}

fn hash_u32s(vals: impl Iterator<Item = u32>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in vals {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Hash dos UV (por canto de face), independente do hash de conectividade.
pub fn uv_hash(t: &Topology) -> u64 {
    hash_u32s(t.uv.iter().flat_map(|f| f.iter().flat_map(|c| c.iter().map(|x| ((x * 65535.0).round()) as u32))))
}

pub fn weights_hash(t: &Topology) -> u64 {
    hash_u32s(t.weights.iter().flat_map(|w| w.iter().flat_map(|(b, x)| [*b as u32, (x * 10000.0).round() as u32])))
}

/// Hash da conectividade dos triângulos (cada quad vira (0,1,2) e (0,2,3)), para malhas triangulares.
pub fn triangulated_hash(faces: &[[u32; 4]], n: usize) -> u64 {
    let tris: Vec<[u32; 3]> = faces.iter().flat_map(|f| [[f[0], f[1], f[2]], [f[0], f[2], f[3]]]).collect();
    tri_hash(&tris, n)
}

pub fn tri_hash(tris: &[[u32; 3]], n: usize) -> u64 {
    hash_u32s(std::iter::once(n as u32).chain(std::iter::once(tris.len() as u32)).chain(tris.iter().flatten().copied()))
}

/// Gera o contrato como JSON (determinístico).
pub fn topology_contract() -> Value {
    let t = reference();
    let mut regions: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &t.region {
        *regions.entry(r.name()).or_insert(0) += 1;
    }
    let qm = QuadMesh {
        pos: reference_positions().clone(),
        faces: t.faces.clone(),
    };
    let loops: Vec<Value> = t
        .loops
        .iter()
        .map(|l| json!({"name": l.name, "closed": l.closed, "vertices": l.verts}))
        .collect();
    let zones: Vec<Value> = ZONES
        .iter()
        .enumerate()
        .map(|(i, (z, name))| {
            let nz = t.zones.iter().filter(|w| w[i] > 0.0).count();
            json!({"name": name, "zone": format!("{z:?}"), "vertices_with_weight": nz})
        })
        .collect();
    let masks: Vec<Value> = t
        .masks
        .iter()
        .map(|(n, v)| json!({"name": n, "vertices_with_weight": v.iter().filter(|x| **x > 0.0).count()}))
        .collect();
    let landmarks: Vec<Value> = t
        .landmarks
        .iter()
        .map(|(n, l)| json!({"name": n, "locator": l.0.iter().map(|(v, w)| json!([v, w])).collect::<Vec<_>>()}))
        .collect();
    let bones: Vec<Value> = bone_defs(t)
        .iter()
        .map(|b| {
            json!({
                "name": b.name,
                "parent": b.parent.map(|p| bone_defs(t)[p].name.clone()),
                "humanoid": b.humanoid,
                "locator": b.locator.0.iter().map(|(v, w)| json!([v, w])).collect::<Vec<_>>(),
            })
        })
        .collect();
    let parts: Vec<Value> = parts_topology()
        .iter()
        .map(|(name, mat, tris, uv)| {
            json!({
                "name": name,
                "material": mat,
                "vertices": uv.len(),
                "triangles": tris.len(),
                "hash": hex(tri_hash(tris, uv.len())),
            })
        })
        .collect();
    let mut morph_names: Vec<String> = shape_defs().into_iter().map(|(n, _, _)| n).collect();
    morph_names.extend(expression_specs().into_iter().map(|(n, _)| n.to_string()));
    let charts: Vec<Value> = CHART_NAMES.iter().enumerate().map(|(i, n)| json!({"name": n, "rect": chart_rect(i)})).collect();
    json!({
        "contract_version": CONTRACT_VERSION,
        "generator": "anigo-charbase Gen-0 (procedural: gaiola de quads + Catmull-Clark de 1 nivel)",
        "note": "Esculpa apenas POSICOES. Mudar contagem, ordem de vertices ou conectividade invalida morphs, pesos, roupa e fisica; use validate_topology/rederive.",
        "skin": {
            "vertex_count": t.n_verts,
            "face_count": t.faces.len(),
            "quads_only": true,
            "connectivity_hash": hex(t.connectivity_hash()),
            "triangulated_hash": hex(triangulated_hash(&t.faces, t.n_verts)),
            "uv_hash": hex(uv_hash(t)),
            "weights_hash": hex(weights_hash(t)),
            "euler_characteristic": qm.euler(),
            "boundary_loops": qm.boundary_loops().len(),
            "cage_vertex_count": t.n_cage,
            "vertex_order": "indices [0, cage_vertex_count) = vertices da gaiola (em ordem de construcao: cabeca, tronco, bracos, pernas); depois pontos de aresta (ordem crescente da aresta da gaiola); depois pontos de face (ordem das faces da gaiola)",
            "regions": regions,
        },
        "bone_count": bones.len(),
        "bones": bones,
        "landmarks": landmarks,
        "loops": loops,
        "groups": { "zones": zones, "masks": masks },
        "uv_charts": charts,
        "parts": parts,
        "morph_targets": morph_names,
    })
}

pub fn contract_json_string() -> String {
    serde_json::to_string_pretty(&topology_contract()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Validação
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopologyStatus {
    /// Mesma contagem, mesma ordem de vértices e mesma conectividade: tudo do contrato vale.
    Exact,
    /// Algo diferente: use `rederive` (vizinho mais próximo) ou volte à topologia do contrato.
    Mismatch,
}

#[derive(Debug, Clone)]
pub struct TopologyReport {
    pub status: TopologyStatus,
    pub vertex_count: usize,
    pub face_count: usize,
    pub expected_vertex_count: usize,
    pub expected_face_count: usize,
    pub quads_only: bool,
    pub manifold: bool,
    pub finite: bool,
    pub connectivity_hash: u64,
    pub expected_hash: u64,
    pub messages: Vec<String>,
}

impl TopologyReport {
    pub fn is_exact(&self) -> bool {
        self.status == TopologyStatus::Exact
    }
}

fn base_report(positions: &[Vec3], nfaces: usize) -> TopologyReport {
    let t = reference();
    TopologyReport {
        status: TopologyStatus::Mismatch,
        vertex_count: positions.len(),
        face_count: nfaces,
        expected_vertex_count: t.n_verts,
        expected_face_count: t.faces.len(),
        quads_only: true,
        manifold: true,
        finite: positions.iter().all(|p| p.is_finite()),
        connectivity_hash: 0,
        expected_hash: t.connectivity_hash(),
        messages: Vec::new(),
    }
}

/// Valida uma malha de quads contra o contrato. Nunca entra em pânico, mesmo com índices fora
/// da faixa, posições não finitas ou listas vazias.
pub fn validate_topology(positions: &[Vec3], faces: &[[u32; 4]]) -> TopologyReport {
    let t = reference();
    let mut r = base_report(positions, faces.len());
    if positions.is_empty() || faces.is_empty() {
        r.messages.push("malha vazia".into());
        return r;
    }
    if !r.finite {
        r.messages.push("há posições não finitas (NaN/infinito)".into());
    }
    if let Some(bad) = faces.iter().position(|f| f.iter().any(|&i| i as usize >= positions.len())) {
        r.messages.push(format!("a face {bad} referencia vértice fora da faixa"));
        return r;
    }
    r.connectivity_hash = hash_faces(faces, positions.len());
    {
        let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for f in faces {
            for k in 0..4 {
                let (a, b) = (f[k].min(f[(k + 1) % 4]), f[k].max(f[(k + 1) % 4]));
                *edges.entry((a, b)).or_insert(0) += 1;
            }
        }
        r.manifold = edges.values().all(|&c| c <= 2);
        if !r.manifold {
            r.messages.push("há arestas compartilhadas por mais de 2 faces (não-manifold)".into());
        }
    }
    if positions.len() != t.n_verts {
        r.messages.push(format!("contagem de vértices {} != {} do contrato", positions.len(), t.n_verts));
    }
    if faces.len() != t.faces.len() {
        r.messages.push(format!("contagem de faces {} != {} do contrato", faces.len(), t.faces.len()));
    }
    if r.connectivity_hash != r.expected_hash {
        if let Some(i) = faces.iter().zip(&t.faces).position(|(a, b)| a != b) {
            r.messages.push(format!("a face {i} difere do contrato ({:?} vs {:?}): a ordem de vértices/faces mudou", faces[i], t.faces[i]));
        } else {
            r.messages.push("hash de conectividade difere do contrato".into());
        }
    }
    if r.finite && r.connectivity_hash == r.expected_hash && positions.len() == t.n_verts {
        r.status = TopologyStatus::Exact;
    }
    r
}

/// Como `validate_topology`, mas para malhas triangulares exportadas (GLB): cada quad do contrato
/// vira os triângulos (0,1,2) e (0,2,3), nessa ordem.
pub fn validate_topology_tris(positions: &[Vec3], tris: &[[u32; 3]]) -> TopologyReport {
    let t = reference();
    let mut r = base_report(positions, tris.len() / 2);
    r.quads_only = false;
    if positions.is_empty() || tris.is_empty() {
        r.messages.push("malha vazia".into());
        return r;
    }
    if tris.iter().any(|f| f.iter().any(|&i| i as usize >= positions.len())) {
        r.messages.push("triângulo com índice fora da faixa".into());
        return r;
    }
    r.connectivity_hash = tri_hash(tris, positions.len());
    r.expected_hash = triangulated_hash(&t.faces, t.n_verts);
    if positions.len() != t.n_verts {
        r.messages.push(format!("contagem de vértices {} != {} do contrato", positions.len(), t.n_verts));
    }
    if tris.len() != t.faces.len() * 2 {
        r.messages.push(format!("contagem de triângulos {} != {} do contrato", tris.len(), t.faces.len() * 2));
    }
    if r.connectivity_hash != r.expected_hash {
        r.messages.push("hash de conectividade (triangulada) difere do contrato".into());
    }
    if r.finite && r.connectivity_hash == r.expected_hash && positions.len() == t.n_verts {
        r.status = TopologyStatus::Exact;
    }
    r
}

// ---------------------------------------------------------------------------------------------
// Re-derivação
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RederiveMethod {
    /// Topologia idêntica ao contrato: localizadores e pesos usados como estão.
    Exact,
    /// Topologia diferente: localizadores e pesos transferidos por vizinho mais próximo.
    NearestNeighbor,
}

#[derive(Debug, Clone)]
pub struct Rederived {
    pub method: RederiveMethod,
    pub skeleton: Skeleton,
    /// Pesos por vértice da malha fornecida (até 4 influências).
    pub weights: Vec<[(u16, f32); 4]>,
    pub landmarks: Vec<(String, Vec3)>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ContractError {
    #[error("malha vazia")]
    Empty,
    #[error("posições não finitas")]
    NotFinite,
    #[error("a malha não cobre o corpo (altura {0} m)")]
    BadScale(f32),
}

/// Para cada ponto de `src`, o índice do ponto de `dst` mais próximo (grade uniforme).
pub fn nearest_map(src: &[Vec3], dst: &[Vec3]) -> Vec<u32> {
    let cell = 0.02f32;
    let key = |p: Vec3| -> (i32, i32, i32) { ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32, (p.z / cell).floor() as i32) };
    let mut grid: BTreeMap<(i32, i32, i32), Vec<u32>> = BTreeMap::new();
    for (i, p) in dst.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i as u32);
    }
    src.iter()
        .map(|p| {
            let k = key(*p);
            let mut best = (f32::MAX, 0u32);
            let mut radius: i32 = 1;
            loop {
                for dx in -radius..=radius {
                    for dy in -radius..=radius {
                        for dz in -radius..=radius {
                            if dx.abs().max(dy.abs()).max(dz.abs()) < radius && radius > 1 {
                                continue;
                            }
                            if let Some(c) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                                for &j in c {
                                    let d = (dst[j as usize] - *p).length_squared();
                                    if d < best.0 {
                                        best = (d, j);
                                    }
                                }
                            }
                        }
                    }
                }
                // um vizinho achado a menos de `radius * cell` não pode ser superado fora do anel
                if best.0 < (radius as f32 * cell).powi(2) || radius > 40 {
                    break;
                }
                radius += 1;
            }
            best.1
        })
        .collect()
}

/// Referências por gênero (masculino, neutro, feminino) para o casamento por vizinho mais próximo.
fn reference_candidates() -> &'static [Vec<Vec3>; 3] {
    static P: OnceLock<[Vec<Vec3>; 3]> = OnceLock::new();
    P.get_or_init(|| {
        let mk = |g: f32| skin_positions(&crate::params::Params { gender: g, ..Default::default() });
        [mk(0.0), mk(0.5), mk(1.0)]
    })
}

/// Ajusta uma malha esculpida à escala/posição da referência `r` (pés em y = 0, mesma altura, centrada em x/z).
fn fit_to_reference(positions: &[Vec3], r: &[Vec3]) -> Vec<Vec3> {
    let (rmn, rmx) = r.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    let (mn, mx) = positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    let s = (rmx.y - rmn.y) / (mx.y - mn.y).max(1e-6);
    let c_ref = (rmn + rmx) * 0.5;
    let c = (mn + mx) * 0.5;
    positions
        .iter()
        .map(|p| Vec3::new((p.x - c.x) * s + c_ref.x, (p.y - mn.y) * s + rmn.y, (p.z - c.z) * s + c_ref.z))
        .collect()
}

/// Re-deriva esqueleto, landmarks e pesos de posições esculpidas.
///
/// * Com a topologia do contrato (mesma contagem e ordem): usa localizadores e pesos do contrato
///   diretamente sobre as novas posições.
/// * Com topologia diferente (re-malhada, reordenada, subdividida): casa cada vértice de referência
///   com o vizinho mais próximo (depois de ajustar escala e posição) para re-localizar as juntas e
///   copia os pesos do vértice de referência mais próximo para cada vértice novo.
///
/// Com `faces = None` a função confia que as posições estão na ordem do contrato quando a contagem
/// de vértices coincide; passe as faces (ou use [`rederive_tris`]) para conferir de verdade.
pub fn rederive(positions: &[Vec3], faces: Option<&[[u32; 4]]>) -> Result<Rederived, ContractError> {
    let t = reference();
    let exact = positions.len() == t.n_verts && faces.map(|f| hash_faces(f, positions.len()) == t.connectivity_hash()).unwrap_or(true);
    rederive_inner(positions, exact)
}

/// Como [`rederive`], para uma malha triangulada (GLB): confere a conectividade triangulada.
pub fn rederive_tris(positions: &[Vec3], tris: &[[u32; 3]]) -> Result<Rederived, ContractError> {
    let t = reference();
    let exact = positions.len() == t.n_verts && tri_hash(tris, positions.len()) == triangulated_hash(&t.faces, t.n_verts);
    rederive_inner(positions, exact)
}

fn rederive_inner(positions: &[Vec3], exact: bool) -> Result<Rederived, ContractError> {
    if positions.is_empty() {
        return Err(ContractError::Empty);
    }
    if positions.iter().any(|p| !p.is_finite()) {
        return Err(ContractError::NotFinite);
    }
    let t = reference();
    let defs = bone_defs(t);
    if exact {
        let skeleton = Skeleton::derive(&defs, positions);
        let landmarks = t.landmarks.iter().map(|(n, l)| (n.clone(), l.eval(positions))).collect();
        return Ok(Rederived {
            method: RederiveMethod::Exact,
            skeleton,
            weights: t.weights.clone(),
            landmarks,
        });
    }
    let h = positions.iter().map(|p| p.y).fold(f32::MIN, f32::max) - positions.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    if !(0.3..=5.0).contains(&h) {
        return Err(ContractError::BadScale(h));
    }
    // escolhe a referência (masculina, neutra ou feminina) mais parecida com a malha
    let mut best: Option<(f32, usize, Vec<Vec3>)> = None;
    for (ci, cand) in reference_candidates().iter().enumerate() {
        let fitted = fit_to_reference(positions, cand);
        let sample: Vec<Vec3> = fitted.iter().step_by((fitted.len() / 600).max(1)).copied().collect();
        let nn = nearest_map(&sample, cand);
        let cost: f32 = sample.iter().zip(&nn).map(|(p, &j)| (*p - cand[j as usize]).length()).sum::<f32>() / sample.len().max(1) as f32;
        if best.as_ref().map(|b| cost < b.0).unwrap_or(true) {
            best = Some((cost, ci, fitted));
        }
    }
    let (_, ci, fitted) = best.expect("há candidatos");
    let refp = &reference_candidates()[ci];
    // vértice de referência -> vértice novo mais próximo
    let to_new = nearest_map(refp, &fitted);
    let relocate = |loc: &crate::topology::Locator| -> Vec3 {
        let mut p = Vec3::ZERO;
        for &(i, w) in &loc.0 {
            p += positions[to_new[i as usize] as usize] * w;
        }
        p
    };
    let world: Vec<Vec3> = defs.iter().map(|d| relocate(&d.locator)).collect();
    let bones = defs
        .iter()
        .enumerate()
        .map(|(i, d)| crate::skeleton::Bone {
            name: d.name.clone(),
            parent: d.parent,
            humanoid: d.humanoid,
            world: world[i].to_array(),
            local: d.parent.map(|p| world[i] - world[p]).unwrap_or(world[i]).to_array(),
        })
        .collect();
    // vértice novo -> vértice de referência mais próximo (pesos copiados)
    let to_ref = nearest_map(&fitted, refp);
    let weights = to_ref.iter().map(|&r| t.weights[r as usize]).collect();
    let landmarks = t.landmarks.iter().map(|(n, l)| (n.clone(), relocate(l))).collect();
    Ok(Rederived {
        method: RederiveMethod::NearestNeighbor,
        skeleton: Skeleton { bones },
        weights,
        landmarks,
    })
}
