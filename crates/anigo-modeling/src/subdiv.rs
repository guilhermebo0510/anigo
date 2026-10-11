//! Subdivision Surface **Catmull-Clark** (equivalente ao modificador Subdivision
//! Surface do Blender / OpenSubdiv) sobre n-gons, com vincos semi-nítidos,
//! regras de fronteira, suavização de UV por corner e interpolação de atributos.
//!
//! # Visão geral do algoritmo
//!
//! Cada nível de subdivisão transforma toda face de `k` cantos em `k` quads:
//! para cada canto `i` nasce o quad `[ponto-do-vértice(v_i), ponto-da-aresta(e_i),
//! ponto-da-face, ponto-da-aresta(e_{i-1})]`, preservando o enrolamento. Por isso o
//! primeiro nível já elimina triângulos e n-gons (V' = V + E + F, F' = Σ k).
//!
//! As posições novas são calculadas como **stencils** (combinações lineares dos
//! vértices do nível anterior). O mesmo stencil é aplicado às posições e aos
//! atributos por vértice (cor, máscara, skin), o que garante que atributos
//! seguem exatamente a mesma suavização da geometria.
//!
//! # Vincos (creases) — regra adotada
//!
//! * `EdgeAttr::crease` (0..1) é convertido em *nitidez* (sharpness) medida em
//!   **níveis de subdivisão**: `sharpness = crease * crease_scale` (padrão
//!   `crease_scale = 6`, o máximo de níveis suportado). `crease >= 1.0` vale
//!   **infinito** (aresta sempre nítida, também em subdivisões encadeadas).
//! * A cada nível a nitidez decai em 1 (`max(s - 1, 0)`, método "uniform" do
//!   OpenSubdiv). Nitidez fracionária `0 < s < 1` mistura linearmente a regra
//!   suave e a regra nítida com peso `s`.
//! * `EVert::crease` segue a mesma regra (vértice de canto: fica parado enquanto
//!   a nitidez for > 0).
//! * Vértice com 0/1 aresta nítida: regra suave; com 2: regra de vinco
//!   `(6v + a + b)/8`; com ≥ 3 (ou vinco de vértice): regra de canto (fica parado).
//! * O resultado devolve a nitidez residual convertida de volta em `crease`
//!   (`residual / crease_scale`, saturando em 1), então empilhar dois
//!   `catmull_clark` mantém um comportamento coerente.
//!
//! # Fronteira
//!
//! Arestas com ≠ 2 faces (fronteira, soltas, não-manifold) são tratadas como
//! infinitamente nítidas. [`BoundaryMode::EdgeOnly`] = "Smooth/All" do Blender
//! (os cantos também deslizam); [`BoundaryMode::EdgeAndCorner`] = "Keep Corners"
//! (vértice de fronteira com uma única face permanece fixo).
//!
//! # UV
//!
//! Os UVs por corner são subdivididos numa "malha de UV" paralela (vértices =
//! grupos de cantos com mesmo vértice e mesmo UV; ilhas/costuras viram fronteira).
//! [`UvSmooth::None`] interpola bilinearmente (sem suavizar), [`UvSmooth::PreserveBoundaries`]
//! mantém fixa toda a fronteira das ilhas (equivale a "Boundaries" do OpenSubdiv),
//! [`UvSmooth::PreserveCorners`] só prende os cantos das ilhas ("Keep Corners").
//!
//! # O que NÃO sobrevive
//!
//! * **Normais customizadas por corner são descartadas** (`Corner::normal = None`)
//!   quando `levels > 0`: a topologia muda e não há interpolação bem definida.
//!   Recalcule/transfira normais depois da subdivisão (módulo `normals_tools`).
//! * Com `levels == 0` a malha é devolvida clonada (normais preservadas).

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EVert, EdgeAttr, EditMesh, Face};
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Nível máximo suportado.
pub const MAX_LEVELS: u32 = 6;

/// Regra de fronteira (equivalente a "Boundary Smooth" do Blender).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BoundaryMode {
    /// "All/Smooth": arestas de fronteira interpoladas; os cantos também deslizam.
    #[default]
    EdgeOnly,
    /// "Keep Corners": como `EdgeOnly`, mas vértices de fronteira com uma única face
    /// (cantos) ficam fixos.
    EdgeAndCorner,
}

/// Suavização de UV por corner (equivalente a "UV Smooth" do Blender).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UvSmooth {
    /// UVs apenas interpolados bilinearmente (não suavizados).
    None,
    /// Toda a fronteira das ilhas de UV fica fixa; o interior é suavizado.
    PreserveBoundaries,
    /// Só os cantos das ilhas ficam fixos; a fronteira desliza (padrão do Blender).
    #[default]
    PreserveCorners,
}

/// Parâmetros da subdivisão.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubdivParams {
    /// Número de níveis, `0..=6` (0 = devolve a malha clonada).
    pub levels: u32,
    pub boundary: BoundaryMode,
    pub uv_smooth: UvSmooth,
    /// Usa `EdgeAttr::crease`/`EVert::crease`. Falso ignora todos os vincos.
    pub use_creases: bool,
    /// Conversão `crease (0..1)` → nitidez em níveis (`crease * crease_scale`);
    /// `crease >= 1` é sempre infinito. Deve ser finito e > 0.
    pub crease_scale: f32,
    /// Limite de segurança de faces do resultado (erro se a estimativa passar).
    pub max_faces: usize,
}

impl Default for SubdivParams {
    fn default() -> Self {
        Self {
            levels: 1,
            boundary: BoundaryMode::EdgeOnly,
            uv_smooth: UvSmooth::PreserveCorners,
            use_creases: true,
            crease_scale: 6.0,
            max_faces: 2_000_000,
        }
    }
}

impl SubdivParams {
    /// Atalho: parâmetros padrão com `levels` níveis.
    pub fn with_levels(levels: u32) -> Self {
        Self {
            levels,
            ..Self::default()
        }
    }

    fn validate(&self) -> ModelResult<()> {
        if self.levels > MAX_LEVELS {
            return Err(ModelError::InvalidParameter(format!(
                "levels deve estar em 0..={MAX_LEVELS}, recebeu {}",
                self.levels
            )));
        }
        if !self.crease_scale.is_finite() || self.crease_scale <= 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "crease_scale deve ser finito e > 0, recebeu {}",
                self.crease_scale
            )));
        }
        if self.max_faces == 0 {
            return Err(ModelError::InvalidParameter("max_faces deve ser > 0".into()));
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ stencils

type Stencil = Vec<(u32, f32)>;

/// Funde termos repetidos (ordenados por índice) e descarta pesos nulos.
fn merge_terms(mut t: Vec<(u32, f64)>) -> Stencil {
    t.sort_by_key(|x| x.0);
    let mut out: Stencil = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        let id = t[i].0;
        let mut w = 0.0f64;
        while i < t.len() && t[i].0 == id {
            w += t[i].1;
            i += 1;
        }
        if w.abs() > 1e-12 {
            out.push((id, w as f32));
        }
    }
    out
}

fn add_scaled(dst: &mut Vec<(u32, f64)>, src: &[(u32, f32)], k: f64) {
    for &(i, w) in src {
        dst.push((i, w as f64 * k));
    }
}

struct Stencils {
    vp: Vec<Stencil>,
    ep: Vec<Stencil>,
    fp: Vec<Stencil>,
}

/// Conjunto de regras aplicado a uma malha (geometria ou malha de UV).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// Catmull-Clark completo, fronteira "edge only".
    Smooth,
    /// Catmull-Clark completo; vértices de fronteira com uma face ficam fixos.
    Corners,
    /// Catmull-Clark; todo vértice com aresta estrutural dura fica fixo.
    AllFixed,
    /// Sem suavização (interpolação linear).
    Linear,
}

/// Nitidez infinita.
const INF: f32 = f32::INFINITY;

fn decay(s: f32) -> f32 {
    if s.is_infinite() {
        s
    } else {
        (s - 1.0).max(0.0)
    }
}

/// Nitidez por aresta (índice da `Topology`) e por vértice, já com as regras
/// estruturais (fronteira dura, cantos fixos) aplicadas.
fn sharpness(mesh: &EditMesh, topo: &Topology, rule: Rule) -> (Vec<f32>, Vec<f32>) {
    let mut es = vec![0.0f32; topo.edges.len()];
    for (&k, attr) in &mesh.edge_attrs {
        if let Some(&ei) = topo.edge_index.get(&k) {
            es[ei as usize] = attr.crease;
        }
    }
    let mut hard_cnt = vec![0u32; mesh.verts.len()];
    for (ei, e) in topo.edges.iter().enumerate() {
        if e.faces.len() != 2 {
            es[ei] = INF;
            hard_cnt[e.verts.0 as usize] += 1;
            hard_cnt[e.verts.1 as usize] += 1;
        }
    }
    let mut vs: Vec<f32> = mesh.verts.iter().map(|v| v.crease).collect();
    for (v, s) in vs.iter_mut().enumerate() {
        let nf = topo.vert_faces[v].len();
        match rule {
            Rule::Corners if nf == 1 && hard_cnt[v] >= 2 => *s = INF,
            Rule::AllFixed if hard_cnt[v] >= 1 => *s = INF,
            _ => {}
        }
    }
    (es, vs)
}

fn other_end(topo: &Topology, e: u32, v: u32) -> u32 {
    let (a, b) = topo.edges[e as usize].verts;
    if a == v {
        b
    } else {
        a
    }
}

fn face_point_stencils(mesh: &EditMesh) -> Vec<Stencil> {
    mesh.faces
        .iter()
        .map(|f| {
            let k = f.corners.len() as f64;
            merge_terms(f.corners.iter().map(|c| (c.vert, 1.0 / k)).collect())
        })
        .collect()
}

fn edge_point_stencils(topo: &Topology, es: &[f32], fp: &[Stencil], linear: bool) -> Vec<Stencil> {
    topo.edges
        .iter()
        .enumerate()
        .map(|(ei, e)| {
            let (a, b) = e.verts;
            let s = es[ei];
            let t = if linear || e.faces.len() != 2 {
                1.0f64
            } else {
                (s as f64).clamp(0.0, 1.0)
            };
            let mut terms: Vec<(u32, f64)> = Vec::with_capacity(8);
            terms.push((a, 0.5 * t));
            terms.push((b, 0.5 * t));
            if t < 1.0 {
                let w = 0.25 * (1.0 - t);
                terms.push((a, w));
                terms.push((b, w));
                add_scaled(&mut terms, &fp[e.faces[0] as usize], w);
                add_scaled(&mut terms, &fp[e.faces[1] as usize], w);
            }
            merge_terms(terms)
        })
        .collect()
}

/// Stencil do ponto-do-vértice conforme as regras suave/vinco/canto.
fn vertex_point_stencil(v: u32, topo: &Topology, es: &[f32], sv: f32, fp: &[Stencil]) -> Stencil {
    let edges = &topo.vert_edges[v as usize];
    let faces = &topo.vert_faces[v as usize];
    if faces.is_empty() || edges.is_empty() {
        return vec![(v, 1.0)];
    }
    let mut sharp: Vec<(f32, u32)> = Vec::new();
    for &e in edges {
        let s = es[e as usize];
        if s > 0.0 {
            sharp.push((s, other_end(topo, e, v)));
        }
    }
    sharp.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let t_r = if sharp.len() >= 2 {
        (((sharp[0].0 as f64) + (sharp[1].0 as f64)) * 0.5).min(1.0)
    } else {
        0.0
    };
    let t_c = if sharp.len() >= 3 {
        (sharp[2].0 as f64).min(1.0)
    } else {
        0.0
    };
    let t_v = if sv > 0.0 { (sv as f64).min(1.0) } else { 0.0 };
    let ws = (1.0 - t_v) * (1.0 - t_c) * (1.0 - t_r);
    let wr = (1.0 - t_v) * (1.0 - t_c) * t_r;
    let wc = (1.0 - ws - wr).max(0.0);

    let mut terms: Vec<(u32, f64)> = Vec::with_capacity(4 * edges.len() + 4);
    if wc > 0.0 {
        terms.push((v, wc));
    }
    if wr > 0.0 {
        // sharp.len() >= 2 garantido por t_r > 0
        terms.push((v, 0.75 * wr));
        terms.push((sharp[0].1, 0.125 * wr));
        terms.push((sharp[1].1, 0.125 * wr));
    }
    if ws > 0.0 {
        let n = edges.len() as f64;
        let nf = faces.len() as f64;
        terms.push((v, ws * (n - 2.0) / n));
        let we = ws / (n * n);
        for &e in edges {
            terms.push((other_end(topo, e, v), we));
        }
        let wf = ws / (n * nf);
        for &f in faces {
            add_scaled(&mut terms, &fp[f as usize], wf);
        }
    }
    merge_terms(terms)
}

fn compute_stencils(mesh: &EditMesh, topo: &Topology, rule: Rule) -> Stencils {
    let (es, vs) = sharpness(mesh, topo, rule);
    let fp = face_point_stencils(mesh);
    let linear = rule == Rule::Linear;
    let ep = edge_point_stencils(topo, &es, &fp, linear);
    let vp: Vec<Stencil> = (0..mesh.verts.len() as u32)
        .map(|v| {
            if linear {
                vec![(v, 1.0)]
            } else {
                vertex_point_stencil(v, topo, &es, vs[v as usize], &fp)
            }
        })
        .collect();
    Stencils { vp, ep, fp }
}

/// Aplica um stencil aos atributos por vértice (posição, cor, máscara, skin).
fn apply_stencil(verts: &[EVert], st: &[(u32, f32)]) -> EVert {
    if st.len() == 1 && st[0].1 == 1.0 {
        let mut v = verts[st[0].0 as usize].clone();
        v.crease = 0.0;
        return v;
    }
    let mut co = [0.0f64; 3];
    let mut color = [0.0f64; 4];
    let mut mask = 0.0f64;
    let mut hidden = true;
    let mut skin: Vec<(u16, f64)> = Vec::new();
    let mut skin_total = 0.0f64;
    for &(i, w) in st {
        let v = &verts[i as usize];
        let w = w as f64;
        co[0] += w * v.co.x as f64;
        co[1] += w * v.co.y as f64;
        co[2] += w * v.co.z as f64;
        for (c, s) in color.iter_mut().zip(v.color.iter()) {
            *c += w * *s as f64;
        }
        mask += w * v.mask as f64;
        hidden &= v.hidden;
        if w > 0.0 {
            for k in 0..4 {
                let wt = v.weights[k] as f64;
                if wt > 0.0 {
                    skin_total += w * wt;
                    let j = v.joints[k];
                    match skin.iter_mut().find(|(jj, _)| *jj == j) {
                        Some(e) => e.1 += w * wt,
                        None => skin.push((j, w * wt)),
                    }
                }
            }
        }
    }
    skin.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    skin.truncate(4);
    let top_sum: f64 = skin.iter().map(|s| s.1).sum();
    let mut joints = [0u16; 4];
    let mut weights = [0.0f32; 4];
    if top_sum > 0.0 {
        let scale = skin_total / top_sum;
        for (k, (j, w)) in skin.iter().enumerate() {
            joints[k] = *j;
            weights[k] = (*w * scale) as f32;
        }
    } else {
        weights[0] = 1.0;
    }
    EVert {
        co: Vec3::new(co[0] as f32, co[1] as f32, co[2] as f32),
        color: [color[0] as f32, color[1] as f32, color[2] as f32, color[3] as f32],
        joints,
        weights,
        mask: (mask as f32).clamp(0.0, 1.0),
        crease: 0.0,
        hidden,
    }
}

fn apply_stencil_pos(verts: &[EVert], st: &[(u32, f32)]) -> EVert {
    let mut co = [0.0f64; 3];
    for &(i, w) in st {
        let p = verts[i as usize].co;
        co[0] += w as f64 * p.x as f64;
        co[1] += w as f64 * p.y as f64;
        co[2] += w as f64 * p.z as f64;
    }
    EVert::new(Vec3::new(co[0] as f32, co[1] as f32, co[2] as f32))
}

/// Gera os `Σk` quads filhos (mesma ordem para geometria e malha de UV).
fn child_quads(mesh: &EditMesh, topo: &Topology, make: impl Fn(&Face, [u32; 4]) -> Face) -> Vec<Face> {
    let nv = mesh.verts.len() as u32;
    let ne = topo.edges.len() as u32;
    let total: usize = mesh.faces.iter().map(|f| f.corners.len()).sum();
    let mut faces = Vec::with_capacity(total);
    for (fi, face) in mesh.faces.iter().enumerate() {
        let k = face.corners.len();
        for i in 0..k {
            let prev = (i + k - 1) % k;
            let quad = [
                face.corners[i].vert,
                nv + topo.face_edges[fi][i],
                nv + ne + fi as u32,
                nv + topo.face_edges[fi][prev],
            ];
            faces.push(make(face, quad));
        }
    }
    faces
}

/// Um nível de Catmull-Clark sobre a malha de trabalho (vincos em `crease` bruto).
fn subdivide_once(
    src: &EditMesh,
    uv_src: Option<&EditMesh>,
    rule: Rule,
    uv_rule: Rule,
) -> (EditMesh, Option<EditMesh>) {
    let topo = Topology::build(src);
    let st = compute_stencils(src, &topo, rule);
    let nv = src.verts.len();
    let ne = topo.edges.len();
    let nf = src.faces.len();

    let mut verts: Vec<EVert> = Vec::with_capacity(nv + ne + nf);
    for v in 0..nv {
        let mut ev = apply_stencil(&src.verts, &st.vp[v]);
        ev.crease = decay(src.verts[v].crease);
        verts.push(ev);
    }
    for s in &st.ep {
        verts.push(apply_stencil(&src.verts, s));
    }
    for s in &st.fp {
        verts.push(apply_stencil(&src.verts, s));
    }
    let faces = child_quads(src, &topo, |f, q| f.with_verts_from(&q));

    let mut edge_attrs: BTreeMap<(u32, u32), EdgeAttr> = BTreeMap::new();
    for (&(a, b), attr) in &src.edge_attrs {
        let Some(&ei) = topo.edge_index.get(&(a, b)) else {
            continue;
        };
        let m = nv as u32 + ei;
        let mut child = *attr;
        if topo.edges[ei as usize].faces.len() == 2 {
            child.crease = decay(attr.crease);
        }
        if !child.is_default() {
            edge_attrs.insert(edge_key(a, m), child);
            edge_attrs.insert(edge_key(m, b), child);
        }
    }
    let next = EditMesh {
        name: src.name.clone(),
        verts,
        faces,
        edge_attrs,
    };

    let next_uv = uv_src.map(|uvm| {
        let utopo = Topology::build(uvm);
        let ust = compute_stencils(uvm, &utopo, uv_rule);
        let mut uverts: Vec<EVert> = Vec::with_capacity(uvm.verts.len() + utopo.edges.len() + uvm.faces.len());
        for s in ust.vp.iter().chain(ust.ep.iter()).chain(ust.fp.iter()) {
            uverts.push(apply_stencil_pos(&uvm.verts, s));
        }
        let ufaces = child_quads(uvm, &utopo, |_, q| Face::new(&q));
        EditMesh {
            name: uvm.name.clone(),
            verts: uverts,
            faces: ufaces,
            edge_attrs: BTreeMap::new(),
        }
    });
    (next, next_uv)
}

fn uv_group_key(vert: u32, uv: Vec2) -> (u32, i32, i32) {
    (
        vert,
        (uv.x * 65536.0).round() as i32,
        (uv.y * 65536.0).round() as i32,
    )
}

/// Malha de UV: vértices = (vértice, UV) distintos; faces espelham a malha.
fn build_uv_mesh(mesh: &EditMesh) -> EditMesh {
    let mut groups: HashMap<(u32, i32, i32), u32> = HashMap::new();
    let mut uvm = EditMesh::new("uv");
    for f in &mesh.faces {
        let mut ids: Vec<u32> = Vec::with_capacity(f.corners.len());
        for c in &f.corners {
            let key = uv_group_key(c.vert, c.uv);
            let id = *groups
                .entry(key)
                .or_insert_with(|| uvm.add_vert(Vec3::new(c.uv.x, c.uv.y, 0.0)));
            ids.push(id);
        }
        uvm.faces.push(Face::new(&ids));
    }
    uvm
}

fn sanitize_sharp(c: f32, scale: f32) -> f32 {
    if !c.is_finite() || c <= 0.0 {
        0.0
    } else if c >= 1.0 {
        INF
    } else {
        c * scale
    }
}

fn restore_crease(s: f32, scale: f32) -> f32 {
    if s.is_infinite() {
        1.0
    } else {
        (s / scale).clamp(0.0, 1.0)
    }
}

/// Estimativa (limite superior) de faces do resultado.
fn predicted_faces(mesh: &EditMesh, levels: u32) -> u128 {
    let first: u128 = mesh.faces.iter().map(|f| f.corners.len() as u128).sum();
    first.saturating_mul(4u128.pow(levels.saturating_sub(1)))
}

/// Subdivisão Catmull-Clark de `levels` níveis. **Não muta** `mesh`: devolve uma
/// malha nova com todas as faces convertidas em quads (para `levels > 0`).
///
/// * `params`: ver [`SubdivParams`] (níveis 0..=6, vincos, fronteira, UV).
/// * Atributos: material/suave/face-set/oculto são herdados pelas 4 (ou k) faces
///   filhas; cor, máscara e skin (top-4 por peso acumulado) são interpolados com
///   os mesmos pesos da posição; `sharp`/`seam`/`bevel_weight` se propagam para as
///   duas metades de cada aresta; normais customizadas são **descartadas**.
/// * Os índices dos vértices originais são preservados (vértice `i` do resultado
///   é o descendente do vértice `i`), seguidos pelos pontos de aresta e de face.
/// * Erros: `levels > 6`, `crease_scale` inválido, malha inválida
///   ([`EditMesh::validate`]) ou resultado maior que `max_faces`.
pub fn catmull_clark(mesh: &EditMesh, params: &SubdivParams) -> ModelResult<EditMesh> {
    params.validate()?;
    mesh.validate()?;
    if params.levels == 0 || mesh.faces.is_empty() {
        return Ok(mesh.clone());
    }
    if predicted_faces(mesh, params.levels) > params.max_faces as u128 {
        return Err(ModelError::InvalidParameter(format!(
            "subdivisão geraria mais de {} faces (levels = {})",
            params.max_faces, params.levels
        )));
    }

    let mut cur = mesh.clone();
    for v in &mut cur.verts {
        v.crease = if params.use_creases {
            sanitize_sharp(v.crease, params.crease_scale)
        } else {
            0.0
        };
    }
    for a in cur.edge_attrs.values_mut() {
        a.crease = if params.use_creases {
            sanitize_sharp(a.crease, params.crease_scale)
        } else {
            0.0
        };
    }
    let has_uv = mesh
        .faces
        .iter()
        .any(|f| f.corners.iter().any(|c| c.uv != Vec2::ZERO));
    let mut uv = has_uv.then(|| build_uv_mesh(&cur));

    let rule = match params.boundary {
        BoundaryMode::EdgeOnly => Rule::Smooth,
        BoundaryMode::EdgeAndCorner => Rule::Corners,
    };
    let uv_rule = match params.uv_smooth {
        UvSmooth::None => Rule::Linear,
        UvSmooth::PreserveBoundaries => Rule::AllFixed,
        UvSmooth::PreserveCorners => Rule::Corners,
    };
    for _ in 0..params.levels {
        let (next, next_uv) = subdivide_once(&cur, uv.as_ref(), rule, uv_rule);
        cur = next;
        uv = next_uv;
    }

    if let Some(uvm) = &uv {
        for (f, uf) in cur.faces.iter_mut().zip(&uvm.faces) {
            for (c, uc) in f.corners.iter_mut().zip(&uf.corners) {
                let p = uvm.verts[uc.vert as usize].co;
                c.uv = Vec2::new(p.x, p.y);
            }
        }
    }
    for v in &mut cur.verts {
        v.crease = restore_crease(v.crease, params.crease_scale);
    }
    for a in cur.edge_attrs.values_mut() {
        a.crease = restore_crease(a.crease, params.crease_scale);
    }
    cur.edge_attrs.retain(|_, a| !a.is_default());
    Ok(cur)
}

/// Posições-limite dos vértices originais de `mesh` (cada vértice `i` da entrada
/// recebe a posição onde ele estaria na superfície de subdivisão infinita).
///
/// Usa os parâmetros padrão de [`SubdivParams`] (vincos ligados, fronteira
/// `EdgeOnly`). Internamente refina um nível (para que toda face seja quad) e
/// aplica as fórmulas de limite: vértice suave de valência `n`
/// `(n²·v + 4·Σe + Σf)/(n(n+5))`; vértice de vinco/fronteira `(4v + a + b)/6`;
/// canto: fica no lugar. Nitidez fracionária mistura as regras como na subdivisão.
/// Se a malha for inválida devolve as posições atuais (nunca entra em pânico).
pub fn limit_positions(mesh: &EditMesh) -> Vec<Vec3> {
    limit_positions_with(mesh, &SubdivParams::default())
        .unwrap_or_else(|_| mesh.verts.iter().map(|v| v.co).collect())
}

/// Como [`limit_positions`], respeitando `boundary`, `use_creases` e
/// `crease_scale` de `params` (`levels`/`uv_smooth` são ignorados).
pub fn limit_positions_with(mesh: &EditMesh, params: &SubdivParams) -> ModelResult<Vec<Vec3>> {
    params.validate()?;
    mesh.validate()?;
    let mut cur = mesh.clone();
    for v in &mut cur.verts {
        v.crease = if params.use_creases {
            sanitize_sharp(v.crease, params.crease_scale)
        } else {
            0.0
        };
    }
    for a in cur.edge_attrs.values_mut() {
        a.crease = if params.use_creases {
            sanitize_sharp(a.crease, params.crease_scale)
        } else {
            0.0
        };
    }
    let rule = match params.boundary {
        BoundaryMode::EdgeOnly => Rule::Smooth,
        BoundaryMode::EdgeAndCorner => Rule::Corners,
    };
    let (next, _) = subdivide_once(&cur, None, rule, Rule::Linear);
    let topo = Topology::build(&next);
    let (es, vs) = sharpness(&next, &topo, rule);
    let pos = |i: u32| next.verts[i as usize].co.as_dvec3();
    let mut out = Vec::with_capacity(mesh.verts.len());
    for v in 0..mesh.verts.len() as u32 {
        out.push(limit_of_vertex(v, &next, &topo, &es, vs[v as usize], &pos).as_vec3());
    }
    Ok(out)
}

fn limit_of_vertex(
    v: u32,
    next: &EditMesh,
    topo: &Topology,
    es: &[f32],
    sv: f32,
    pos: &dyn Fn(u32) -> glam::DVec3,
) -> glam::DVec3 {
    let edges = &topo.vert_edges[v as usize];
    let faces = &topo.vert_faces[v as usize];
    let p = pos(v);
    if faces.is_empty() || edges.is_empty() {
        return p;
    }
    let mut sharp: Vec<(f32, u32)> = Vec::new();
    for &e in edges {
        let s = es[e as usize];
        if s > 0.0 {
            sharp.push((s, other_end(topo, e, v)));
        }
    }
    sharp.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let t_r = if sharp.len() >= 2 {
        (((sharp[0].0 as f64) + (sharp[1].0 as f64)) * 0.5).min(1.0)
    } else {
        0.0
    };
    let t_c = if sharp.len() >= 3 {
        (sharp[2].0 as f64).min(1.0)
    } else {
        0.0
    };
    let t_v = if sv > 0.0 { (sv as f64).min(1.0) } else { 0.0 };
    let ws = (1.0 - t_v) * (1.0 - t_c) * (1.0 - t_r);
    let wr = (1.0 - t_v) * (1.0 - t_c) * t_r;
    let wc = (1.0 - ws - wr).max(0.0);

    let mut res = p * wc;
    if wr > 0.0 {
        res += (p * 4.0 + pos(sharp[0].1) + pos(sharp[1].1)) / 6.0 * wr;
    }
    if ws > 0.0 {
        let n = edges.len() as f64;
        let all_quads = faces.iter().all(|&f| next.faces[f as usize].corners.len() == 4);
        let smooth = if faces.len() == edges.len() && all_quads {
            let mut se = glam::DVec3::ZERO;
            for &e in edges {
                se += pos(other_end(topo, e, v));
            }
            let mut sf = glam::DVec3::ZERO;
            for &f in faces {
                let face = &next.faces[f as usize];
                if let Some(ci) = face.corners.iter().position(|c| c.vert == v) {
                    sf += pos(face.corners[(ci + 2) % 4].vert);
                }
            }
            (p * (n * n) + se * 4.0 + sf) / (n * (n + 5.0))
        } else {
            p
        };
        res += smooth * ws;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};

    fn volume(m: &EditMesh) -> f64 {
        let mut vol = 0.0f64;
        for f in &m.faces {
            let p: Vec<glam::DVec3> = f.corners.iter().map(|c| m.verts[c.vert as usize].co.as_dvec3()).collect();
            for i in 1..p.len() - 1 {
                vol += p[0].dot(p[i].cross(p[i + 1])) / 6.0;
            }
        }
        vol
    }

    fn area(m: &EditMesh) -> f64 {
        (0..m.face_count() as u32).map(|f| m.face_area(f) as f64).sum()
    }

    fn closed_ok(m: &EditMesh) {
        m.validate().unwrap();
        let t = Topology::build(m);
        assert!(t.is_closed_manifold(), "deveria ser fechada e manifold");
        assert_eq!(t.euler_characteristic(m), 2);
    }

    #[test]
    fn cube_level_counts() {
        let c = primitives::cube(2.0);
        let l1 = catmull_clark(&c, &SubdivParams::with_levels(1)).unwrap();
        assert_eq!(l1.vert_count(), 26);
        assert_eq!(l1.face_count(), 24);
        assert!(l1.faces.iter().all(|f| f.len() == 4));
        let l2 = catmull_clark(&c, &SubdivParams::with_levels(2)).unwrap();
        assert_eq!(l2.vert_count(), 98);
        assert_eq!(l2.face_count(), 96);
        closed_ok(&l1);
        closed_ok(&l2);
        assert_outward(&l1);
        assert_outward(&l2);
    }

    #[test]
    fn cube_level_one_matches_hand_computed_catmull_clark() {
        let c = primitives::cube(2.0);
        let m = catmull_clark(&c, &SubdivParams::with_levels(1)).unwrap();
        // vértice de canto (valência 3): 5/9 em cada eixo
        for v in 0..8 {
            let p = m.verts[v].co;
            assert!((p.abs() - Vec3::splat(5.0 / 9.0)).length() < 1e-6, "{p:?}");
            assert_eq!(p.signum(), c.verts[v].co.signum());
        }
        // pontos de aresta: (0.75, 0.75, 0); pontos de face: centros das faces
        let edge_pts = &m.verts[8..20];
        for p in edge_pts {
            let mut a = [p.co.x.abs(), p.co.y.abs(), p.co.z.abs()];
            a.sort_by(f32::total_cmp);
            assert!((a[0]).abs() < 1e-6 && (a[1] - 0.75).abs() < 1e-6 && (a[2] - 0.75).abs() < 1e-6);
        }
        for p in &m.verts[20..26] {
            assert!((p.co.abs().max_element() - 1.0).abs() < 1e-6 && (p.co.length() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn level_zero_is_identity_and_keeps_normals() {
        let mut c = primitives::cube(2.0);
        c.faces[0].corners[0].normal = Some(Vec3::X);
        let out = catmull_clark(&c, &SubdivParams::with_levels(0)).unwrap();
        assert_eq!(out, c);
    }

    #[test]
    fn invalid_params_are_rejected_and_input_untouched() {
        let c = primitives::cube(2.0);
        let before = c.clone();
        assert!(matches!(
            catmull_clark(&c, &SubdivParams::with_levels(7)),
            Err(ModelError::InvalidParameter(_))
        ));
        let mut p = SubdivParams::with_levels(1);
        p.crease_scale = 0.0;
        assert!(catmull_clark(&c, &p).is_err());
        p.crease_scale = f32::NAN;
        assert!(catmull_clark(&c, &p).is_err());
        p = SubdivParams::with_levels(1);
        p.max_faces = 0;
        assert!(catmull_clark(&c, &p).is_err());
        p.max_faces = 10;
        assert!(catmull_clark(&c, &p).is_err());
        assert_eq!(c, before);
        // malha inválida
        let mut bad = primitives::cube(1.0);
        bad.faces[0].corners[0].vert = 999;
        assert!(catmull_clark(&bad, &SubdivParams::with_levels(1)).is_err());
        let mut nan = primitives::cube(1.0);
        nan.verts[0].co = Vec3::NAN;
        assert!(catmull_clark(&nan, &SubdivParams::with_levels(1)).is_err());
    }

    #[test]
    fn empty_mesh_is_returned_as_is() {
        let m = EditMesh::new("vazio");
        assert_eq!(catmull_clark(&m, &SubdivParams::with_levels(3)).unwrap(), m);
        assert!(limit_positions(&m).is_empty());
    }

    #[test]
    fn cube_area_and_volume_decrease_monotonically_and_converge() {
        let c = primitives::cube(2.0);
        let mut vols = vec![volume(&c)];
        let mut areas = vec![area(&c)];
        for l in 1..=5 {
            let m = catmull_clark(&c, &SubdivParams::with_levels(l)).unwrap();
            vols.push(volume(&m));
            areas.push(area(&m));
        }
        for w in vols.windows(2) {
            assert!(w[1] < w[0], "volume deve diminuir: {vols:?}");
        }
        for w in areas.windows(2) {
            assert!(w[1] < w[0], "área deve diminuir: {areas:?}");
        }
        // convergência geométrica: as diferenças caem ~4x por nível
        let d: Vec<f64> = vols.windows(2).map(|w| w[0] - w[1]).collect();
        assert!(d[4] < d[3] / 3.0 && d[3] < d[2] / 3.0, "diferenças {d:?}");
        // limite (Richardson): o cubo CC encolhe bastante, mas fica bem acima de 1 e abaixo de 8
        let limit = vols[5] - d[4] / 3.0;
        assert!(limit > 2.0 && limit < 3.5, "limite {limit}");
    }

    #[test]
    fn quad_sphere_converges_to_sphere() {
        let s = primitives::quad_sphere(1.0, 8);
        let m = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        closed_ok(&m);
        assert_outward(&m);
        let (lo, hi) = m
            .verts
            .iter()
            .fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(v.co.length()), hi.max(v.co.length())));
        assert!(lo > 0.97 && hi < 1.001, "raios [{lo}, {hi}]");
        let vol = volume(&m);
        assert!(vol > 4.0 / 3.0 * std::f64::consts::PI * 0.93 && vol < 4.19, "volume {vol}");
        let s = primitives::quad_sphere(1.0, 4);
        // nível maior fica mais redondo
        let m2 = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        let m3 = catmull_clark(&s, &SubdivParams::with_levels(3)).unwrap();
        let spread = |m: &EditMesh| {
            let (lo, hi) = m
                .verts
                .iter()
                .fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(v.co.length()), hi.max(v.co.length())));
            hi - lo
        };
        assert!(spread(&m3) < 0.1 && spread(&m2) < 0.1, "{} {}", spread(&m2), spread(&m3));
    }

    #[test]
    fn euler_is_preserved_for_assorted_closed_meshes() {
        let meshes = vec![
            primitives::uv_sphere(1.0, 6, 8), // triângulos nos polos
            primitives::cylinder(1.0, 2.0, 7, 2, true), // tampas n-gon (heptágono)
            primitives::cone(1.0, 0.0, 2.0, 5, true),
            primitives::capsule(0.5, 1.0, 8, 3),
            primitives::cube_subdivided(2.0, 3),
        ];
        for m in meshes {
            for l in 1..=2 {
                let out = catmull_clark(&m, &SubdivParams::with_levels(l)).unwrap();
                closed_ok(&out);
                assert_outward(&out);
                assert!(out.faces.iter().all(|f| f.len() == 4), "{}", m.name);
            }
        }
        let t = primitives::torus(1.0, 0.25, 12, 6);
        let out = catmull_clark(&t, &SubdivParams::with_levels(2)).unwrap();
        let topo = Topology::build(&out);
        assert!(topo.is_closed_manifold());
        assert_eq!(topo.euler_characteristic(&out), 0);
    }

    #[test]
    fn ngon_counts_follow_formula() {
        let m = primitives::cylinder(1.0, 2.0, 7, 1, true);
        let k: usize = m.faces.iter().map(|f| f.len()).sum();
        let l1 = catmull_clark(&m, &SubdivParams::with_levels(1)).unwrap();
        assert_eq!(l1.face_count(), k);
        let topo = Topology::build(&m);
        assert_eq!(l1.vert_count(), m.vert_count() + topo.edges.len() + m.face_count());
    }

    #[test]
    fn full_crease_keeps_cube_exact() {
        let mut c = primitives::cube(2.0);
        let topo = Topology::build(&c);
        for e in &topo.edges {
            c.set_crease(e.verts.0, e.verts.1, 1.0);
        }
        for l in 1..=3 {
            let m = catmull_clark(&c, &SubdivParams::with_levels(l)).unwrap();
            closed_ok(&m);
            assert!((volume(&m) - 8.0).abs() < 1e-4, "volume {}", volume(&m));
            assert!((area(&m) - 24.0).abs() < 1e-3);
            // os 8 cantos originais continuam no lugar
            for v in 0..8 {
                assert!((m.verts[v].co - c.verts[v].co).length() < 1e-6);
            }
            assert!(m.verts.iter().all(|v| v.co.abs().max_element() <= 1.0 + 1e-5));
            // vincos infinitos persistem como crease 1
            assert!(m.edge_attrs.values().any(|a| a.crease == 1.0));
        }
    }

    #[test]
    fn crease_is_monotonic_and_ignored_when_disabled() {
        let make = |crease: f32| {
            let mut c = primitives::cube(2.0);
            let topo = Topology::build(&c);
            for e in &topo.edges {
                c.set_crease(e.verts.0, e.verts.1, crease);
            }
            c
        };
        let vol_at = |c: &EditMesh, use_creases: bool| {
            let mut p = SubdivParams::with_levels(2);
            p.use_creases = use_creases;
            volume(&catmull_clark(c, &p).unwrap())
        };
        let v0 = vol_at(&make(0.0), true);
        let v_a = vol_at(&make(0.1), true);
        let v_b = vol_at(&make(0.25), true);
        let v_half = vol_at(&make(0.5), true);
        let v_one = vol_at(&make(1.0), true);
        assert!(v0 < v_a && v_a < v_b && v_b < v_half, "{v0} {v_a} {v_b} {v_half}");
        // nitidez 3 >= 2 níveis: já é totalmente nítido; crease 1 é igual
        assert!((v_half - 8.0).abs() < 1e-4 && (v_one - 8.0).abs() < 1e-4);
        assert!((vol_at(&make(1.0), false) - v0).abs() < 1e-6);
        // crease_scale maior => mais nítido para o mesmo crease fracionário
        let mut p = SubdivParams::with_levels(2);
        p.crease_scale = 1.0;
        let weak = volume(&catmull_clark(&make(0.25), &p).unwrap());
        p.crease_scale = 6.0;
        let strong = volume(&catmull_clark(&make(0.25), &p).unwrap());
        assert!(weak < strong);
    }

    #[test]
    fn crease_ring_keeps_top_face_flat() {
        let mut c = primitives::cube(2.0);
        let topo = Topology::build(&c);
        // arestas do topo (y = +1)
        for e in &topo.edges {
            if c.co(e.verts.0).y > 0.99 && c.co(e.verts.1).y > 0.99 {
                c.set_crease(e.verts.0, e.verts.1, 1.0);
            }
        }
        let m = catmull_clark(&c, &SubdivParams::with_levels(3)).unwrap();
        closed_ok(&m);
        let top: Vec<&EVert> = m.verts.iter().filter(|v| v.co.y > 0.999).collect();
        assert!(top.len() > 8);
        assert!(top.iter().all(|v| (v.co.y - 1.0).abs() < 1e-5), "topo continua plano");
        // o anel crescente (vinco de 4 arestas) é suavizado: cantos do topo recuam
        let max_x = top.iter().map(|v| v.co.x).fold(f32::MIN, f32::max);
        assert!(max_x < 1.0 - 1e-3 && max_x > 0.5);
    }

    #[test]
    fn vertex_crease_pins_a_single_corner() {
        let mut c = primitives::cube(2.0);
        c.verts[0].crease = 1.0;
        let m = catmull_clark(&c, &SubdivParams::with_levels(3)).unwrap();
        assert!((m.verts[0].co - c.verts[0].co).length() < 1e-6);
        assert!(m.verts[0].crease > 0.99);
        // os outros cantos recuam
        for v in 1..8 {
            assert!(m.verts[v].co.length() < c.verts[v].co.length() - 0.05);
        }
        // semi-nítido: um nível de nitidez dura só um nível
        let mut semi = primitives::cube(2.0);
        semi.verts[0].crease = 1.0 / 6.0; // nitidez = 1
        let m1 = catmull_clark(&semi, &SubdivParams::with_levels(1)).unwrap();
        assert!((m1.verts[0].co - semi.verts[0].co).length() < 1e-6);
        assert_eq!(m1.verts[0].crease, 0.0);
    }

    #[test]
    fn plane_boundary_stays_straight() {
        let g = primitives::grid(4, 4, 4.0, 4.0);
        let mut p = SubdivParams::with_levels(3);
        p.boundary = BoundaryMode::EdgeAndCorner;
        let m = catmull_clark(&g, &p).unwrap();
        m.validate().unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo - Vec3::new(-2.0, 0.0, -2.0)).length() < 1e-5);
        assert!((hi - Vec3::new(2.0, 0.0, 2.0)).length() < 1e-5);
        let topo = Topology::build(&m);
        let mut boundary_verts = 0;
        for v in 0..m.vert_count() as u32 {
            assert!(m.co(v).y.abs() < 1e-6);
            if topo.is_boundary_vert(v) {
                boundary_verts += 1;
                let c = m.co(v);
                let on_line = (c.x.abs() - 2.0).abs() < 1e-5 || (c.z.abs() - 2.0).abs() < 1e-5;
                assert!(on_line, "fronteira saiu da reta: {c:?}");
            }
        }
        assert_eq!(boundary_verts, 4 * 4 * 8);
        // EdgeOnly: continua plano e dentro do quadrado; só o canto desliza
        let m2 = catmull_clark(&g, &SubdivParams::with_levels(2)).unwrap();
        let (lo2, hi2) = m2.bounds().unwrap();
        assert!(lo2.x >= -2.0 - 1e-6 && hi2.x <= 2.0 + 1e-6);
        assert!(m2.verts.iter().all(|v| v.co.y.abs() < 1e-6));
        let corner_cut = m2.verts.iter().all(|v| !(v.co.x.abs() > 1.999 && v.co.z.abs() > 1.999));
        assert!(corner_cut, "no modo EdgeOnly o canto deve recuar");
        // fronteira não-canto continua sobre a reta
        let topo2 = Topology::build(&m2);
        for v in 0..m2.vert_count() as u32 {
            if topo2.is_boundary_vert(v) {
                let c = m2.co(v);
                let on_line = (c.x.abs() - 2.0).abs() < 1e-5 || (c.z.abs() - 2.0).abs() < 1e-5;
                let near_corner = c.x.abs() > 1.4 && c.z.abs() > 1.4;
                assert!(on_line || near_corner);
            }
        }
    }

    #[test]
    fn open_cylinder_boundary_stays_planar_and_circular() {
        let cyl = primitives::cylinder(1.0, 2.0, 12, 2, false);
        let m = catmull_clark(&cyl, &SubdivParams::with_levels(2)).unwrap();
        m.validate().unwrap();
        let topo = Topology::build(&m);
        assert_eq!(topo.euler_characteristic(&m), 0);
        for v in 0..m.vert_count() as u32 {
            if topo.is_boundary_vert(v) {
                let y = m.co(v).y;
                assert!((y.abs() - 1.0).abs() < 1e-5, "anel de borda saiu do plano: {y}");
            }
        }
    }

    #[test]
    fn uv_of_plane_stays_in_unit_square_and_monotonic() {
        let g = primitives::grid(3, 3, 3.0, 3.0);
        for mode in [UvSmooth::None, UvSmooth::PreserveBoundaries, UvSmooth::PreserveCorners] {
            for boundary in [BoundaryMode::EdgeOnly, BoundaryMode::EdgeAndCorner] {
                let mut p = SubdivParams::with_levels(3);
                p.uv_smooth = mode;
                p.boundary = boundary;
                let m = catmull_clark(&g, &p).unwrap();
                let mut min = Vec2::splat(f32::MAX);
                let mut max = Vec2::splat(f32::MIN);
                for f in &m.faces {
                    for c in &f.corners {
                        assert!(c.uv.is_finite());
                        min = min.min(c.uv);
                        max = max.max(c.uv);
                    }
                }
                assert!(min.x >= -1e-5 && min.y >= -1e-5 && max.x <= 1.0 + 1e-5 && max.y <= 1.0 + 1e-5);
                assert!(min.x < 1e-5 && max.x > 1.0 - 1e-5, "{mode:?} {boundary:?}: [{min:?},{max:?}]");
                // o UV é uma função monotônica de x e z (cada face tem uv.x crescente com x)
                for f in &m.faces {
                    for a in &f.corners {
                        for b in &f.corners {
                            let (pa, pb) = (m.co(a.vert), m.co(b.vert));
                            if pa.x < pb.x - 1e-4 {
                                assert!(a.uv.x <= b.uv.x + 1e-4, "{mode:?} {boundary:?} u não monotônico");
                            }
                            if pa.z < pb.z - 1e-4 {
                                assert!(a.uv.y <= b.uv.y + 1e-4, "{mode:?} {boundary:?} v não monotônico");
                            }
                        }
                    }
                }
            }
        }
        // com cantos mantidos e UV linear, UV == (x,z) mapeado exatamente
        let mut p = SubdivParams::with_levels(2);
        p.boundary = BoundaryMode::EdgeAndCorner;
        p.uv_smooth = UvSmooth::PreserveBoundaries;
        let m = catmull_clark(&g, &p).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                let q = m.co(c.vert);
                assert!((c.uv.x - (q.x / 3.0 + 0.5)).abs() < 1e-4);
                assert!((c.uv.y - (q.z / 3.0 + 0.5)).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn uv_seams_are_preserved_on_cube() {
        // O cubo tem 6 ilhas de UV desconexas: cada face filha deve ficar dentro da sua ilha 0..1
        let c = primitives::cube(2.0);
        for mode in [UvSmooth::None, UvSmooth::PreserveBoundaries, UvSmooth::PreserveCorners] {
            let mut p = SubdivParams::with_levels(2);
            p.uv_smooth = mode;
            let m = catmull_clark(&c, &p).unwrap();
            for f in &m.faces {
                for k in &f.corners {
                    assert!(k.uv.x >= -1e-5 && k.uv.x <= 1.0 + 1e-5 && k.uv.y >= -1e-5 && k.uv.y <= 1.0 + 1e-5);
                }
            }
            // as faces filhas de uma mesma face-mãe cobrem 0..1 inteiro (área de UV total = 6)
            let uv_area: f32 = m
                .faces
                .iter()
                .map(|f| {
                    let p: Vec<Vec3> = f.corners.iter().map(|k| k.uv.extend(0.0)).collect();
                    crate::mesh::polygon_area(&p)
                })
                .sum();
            assert!((uv_area - 6.0).abs() < 0.05, "{mode:?}: {uv_area}");
        }
    }

    #[test]
    fn meshes_without_uv_get_zero_uv() {
        let mut c = primitives::cube(2.0);
        for f in &mut c.faces {
            for k in &mut f.corners {
                k.uv = Vec2::ZERO;
            }
        }
        let m = catmull_clark(&c, &SubdivParams::with_levels(2)).unwrap();
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|k| k.uv == Vec2::ZERO)));
    }

    #[test]
    fn face_and_edge_attributes_propagate() {
        let mut c = primitives::cube(2.0);
        c.faces[2].material = 7;
        c.faces[2].smooth = false;
        c.faces[2].face_set = 5;
        c.faces[3].hidden = true;
        c.faces[0].corners[1].normal = Some(Vec3::Y);
        let topo = Topology::build(&c);
        let e0 = topo.edges[0].verts;
        c.set_sharp(e0.0, e0.1, true);
        c.set_seam(e0.0, e0.1, true);
        c.set_bevel_weight(e0.0, e0.1, 0.5);
        let m = catmull_clark(&c, &SubdivParams::with_levels(1)).unwrap();
        for (i, f) in m.faces.iter().enumerate() {
            let parent = &c.faces[i / 4];
            assert_eq!(f.material, parent.material);
            assert_eq!(f.smooth, parent.smooth);
            assert_eq!(f.face_set, parent.face_set);
            assert_eq!(f.hidden, parent.hidden);
            assert!(f.corners.iter().all(|k| k.normal.is_none()), "normais customizadas são descartadas");
        }
        // a aresta marcada virou duas arestas filhas com os mesmos flags
        let marked: Vec<_> = m.edge_attrs.iter().filter(|(_, a)| a.sharp && a.seam).collect();
        assert_eq!(marked.len(), 2);
        assert!(marked.iter().all(|(_, a)| a.bevel_weight == 0.5));
        // as duas metades compartilham o ponto da aresta e uma delas termina em e0.0 / e0.1
        let ends: Vec<u32> = marked.iter().flat_map(|((a, b), _)| [*a, *b]).collect();
        assert!(ends.contains(&e0.0) && ends.contains(&e0.1));
    }

    #[test]
    fn vertex_attributes_are_interpolated() {
        let mut s = primitives::quad_sphere(1.0, 3);
        for v in &mut s.verts {
            v.color = [0.2, 0.4, 0.6, 0.8];
            v.mask = 0.5;
        }
        let m = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        for v in &m.verts {
            for (a, b) in v.color.iter().zip([0.2, 0.4, 0.6, 0.8]) {
                assert!((a - b).abs() < 1e-5, "cor constante deve continuar constante");
            }
            assert!((v.mask - 0.5).abs() < 1e-5);
            let sum: f32 = v.weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
        // gradiente: máscara 0 em um lado e 1 no outro permanece em [0,1] e varia suavemente
        for v in &mut s.verts {
            v.mask = if v.co.x > 0.0 { 1.0 } else { 0.0 };
        }
        let m = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        assert!(m.verts.iter().all(|v| (0.0..=1.0).contains(&v.mask)));
        assert!(m.verts.iter().any(|v| v.mask > 0.05 && v.mask < 0.95));
    }

    #[test]
    fn skin_weights_keep_top_four_normalized() {
        let mut s = primitives::quad_sphere(1.0, 2);
        for (i, v) in s.verts.iter_mut().enumerate() {
            let j = (i % 6) as u16;
            v.joints = [j, j + 1, j + 2, j + 3];
            v.weights = [0.4, 0.3, 0.2, 0.1];
        }
        let m = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        for v in &m.verts {
            let sum: f32 = v.weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "soma de pesos {sum}");
            assert!(v.weights.windows(2).all(|w| w[0] >= w[1] - 1e-6), "ordem decrescente");
            let mut j = v.joints.to_vec();
            j.sort_unstable();
            j.dedup();
            let nonzero = v.weights.iter().filter(|w| **w > 0.0).count();
            assert!(j.len() >= nonzero);
        }
        // vértice com um único joint nunca mistura joints
        let mut s2 = primitives::quad_sphere(1.0, 2);
        for v in &mut s2.verts {
            v.joints = [9, 0, 0, 0];
            v.weights = [1.0, 0.0, 0.0, 0.0];
        }
        let m2 = catmull_clark(&s2, &SubdivParams::with_levels(1)).unwrap();
        assert!(m2.verts.iter().all(|v| v.joints[0] == 9 && v.weights[0] == 1.0));
    }

    #[test]
    fn determinism() {
        let s = primitives::uv_sphere(1.0, 7, 9);
        let a = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        let b = catmull_clark(&s, &SubdivParams::with_levels(2)).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a, b);
    }

    #[test]
    fn limit_positions_match_deep_subdivision() {
        let c = primitives::cube(2.0);
        let lim = limit_positions(&c);
        let deep = catmull_clark(&c, &SubdivParams::with_levels(6)).unwrap();
        for (i, p) in lim.iter().enumerate() {
            assert!((*p - deep.verts[i].co).length() < 2e-3, "{i}: {p:?} vs {:?}", deep.verts[i].co);
            // o limite está contido no cubo e é simétrico
            assert!(p.abs().max_element() < 1.0);
        }
        let (l0, l1) = (lim[0].abs(), lim[7].abs());
        assert!((l0 - l1).length() < 1e-5);
        // uma esfera: o limite fica ligeiramente dentro, nunca fora
        let s = primitives::quad_sphere(1.0, 4);
        for p in limit_positions(&s) {
            assert!(p.length() < 1.0 + 1e-4 && p.length() > 0.9, "{}", p.length());
        }
    }

    #[test]
    fn limit_positions_respect_boundary_creases_and_corners() {
        let g = primitives::grid(2, 2, 2.0, 2.0);
        let lim = limit_positions(&g);
        assert_eq!(lim.len(), g.vert_count());
        let topo = Topology::build(&g);
        for v in 0..g.vert_count() as u32 {
            assert!(lim[v as usize].y.abs() < 1e-6);
            if topo.is_boundary_vert(v) {
                // fronteira reta: pontos de aresta ficam na reta
                let c = g.co(v);
                if c.x.abs() > 0.99 && c.z.abs() < 0.01 {
                    assert!((lim[v as usize].x - c.x).abs() < 1e-5);
                }
            }
        }
        // canto com vinco de vértice fica parado
        let mut c = primitives::cube(2.0);
        c.verts[3].crease = 1.0;
        let l = limit_positions(&c);
        assert!((l[3] - c.verts[3].co).length() < 1e-6);
        // malha inválida não entra em pânico
        let mut bad = primitives::cube(1.0);
        bad.faces[0].corners[0].vert = 99;
        assert_eq!(limit_positions(&bad).len(), bad.vert_count());
    }

    #[test]
    fn non_manifold_and_wire_edges_do_not_panic() {
        // três faces compartilhando a aresta (0,1)
        let mut m = EditMesh::new("nm");
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.5, 1.0, 0.0),
            Vec3::new(0.5, -1.0, 0.0),
            Vec3::new(0.5, 0.0, 1.0),
            Vec3::new(5.0, 5.0, 5.0),
            Vec3::new(6.0, 5.0, 5.0),
        ] {
            m.add_vert(p);
        }
        m.add_face(&[0, 1, 2]).unwrap();
        m.add_face(&[1, 0, 3]).unwrap();
        m.add_face(&[0, 1, 4]).unwrap();
        m.set_sharp(5, 6, true); // aresta solta
        let out = catmull_clark(&m, &SubdivParams::with_levels(2)).unwrap();
        out.validate().unwrap();
        assert_eq!(out.face_count(), 36);
        assert!(out.verts.iter().all(|v| v.co.is_finite()));
        // a aresta solta continua existindo (duas metades por nível => 4 arestas)
        assert_eq!(out.edge_attrs.iter().filter(|(_, a)| a.sharp).count(), 4);
    }

    #[test]
    fn isolated_vertices_are_kept() {
        let mut m = primitives::cube(2.0);
        let extra = m.add_vert(Vec3::splat(9.0));
        let out = catmull_clark(&m, &SubdivParams::with_levels(1)).unwrap();
        assert_eq!(out.verts[extra as usize].co, Vec3::splat(9.0));
    }

    #[test]
    fn large_mesh_is_fast_enough() {
        let s = primitives::quad_sphere(1.0, 32);
        let t = std::time::Instant::now();
        let m = catmull_clark(&s, &SubdivParams::with_levels(1)).unwrap();
        assert_eq!(m.face_count(), s.face_count() * 4);
        let topo = Topology::build(&m);
        assert!(topo.is_closed_manifold());
        assert!(t.elapsed().as_secs_f32() < 20.0, "demorou {:?}", t.elapsed());
    }

    #[test]
    fn serde_roundtrip_of_params() {
        let mut p = SubdivParams::with_levels(3);
        p.uv_smooth = UvSmooth::None;
        p.boundary = BoundaryMode::EdgeAndCorner;
        let json = serde_json::to_string(&p).unwrap();
        let back: SubdivParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        // campos ausentes usam o padrão
        let partial: SubdivParams = serde_json::from_str(r#"{"levels":2}"#).unwrap();
        assert_eq!(partial.levels, 2);
        assert_eq!(partial.crease_scale, 6.0);
    }
}
