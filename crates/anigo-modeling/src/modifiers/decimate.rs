//! Modificador **Decimate**: reduz a contagem de polígonos.
//!
//! * [`decimate_collapse`]: colapso de aresta por **erro quádrico**
//!   (Garland-Heckbert, 1997) com fila de prioridade, condição de link para
//!   preservar a topologia (manifold, sem pinçamentos), teste de inversão de
//!   normais, fronteira/costuras/arestas duras protegíveis e pesos por vértice.
//! * [`decimate_planar`]: dissolve faces adjacentes quase coplanares em n-gons e
//!   remove vértices colineares que sobraram (equivale a "Planar" do Blender).
//!
//! # Colapso quádrico: como funciona
//!
//! A malha é triangulada internamente. Cada vértice acumula a quádrica das
//! faces incidentes (planos ponderados por área). O custo de colapsar `(a, b)`
//! é `vᵀ(Qa + Qb)v` na posição `v` escolhida entre o ponto ótimo (se o sistema
//! 3×3 for bem condicionado e o ponto não fugir), as pontas e o ponto médio.
//! Colapsa-se sempre a aresta mais barata até atingir `ratio · triângulos`.
//!
//! Atributos: o vértice sobrevivente recebe a interpolação (cor, máscara,
//! skin do mais próximo) na posição final; o **UV de cada corner** sobrevivente é
//! deslocado de forma consistente com o colapso (a parametrização linear da
//! ilha é preservada); normais customizadas dos corners tocados são descartadas.
//!
//! # Proteções (travas)
//!
//! Um vértice **travado** nunca se move e só pode ser alvo de colapsos de um vizinho
//! livre. São travados: vértices de fronteira (`preserve_boundary`), de arestas
//! `seam` ou com descontinuidade de UV (`preserve_seams`), de arestas `sharp`
//! (`preserve_sharp`), de arestas não-manifold, e vértices com peso de grupo ≥ 0.999.
//! Arestas entre dois vértices travados nunca colapsam.
//!
//! # Limitações conhecidas
//!
//! * Com muitas travas (ex.: cartões de cabelo, onde quase todo vértice é de
//!   fronteira) o alvo `ratio` pode ser **inalcançável**: o resultado devolve o que
//!   foi possível sem violar as travas.
//! * Sem `preserve_boundary`, a fronteira é mantida apenas aproximadamente por
//!   quádricas de penalidade (podem encurtar cantos suavemente).
//! * Atributos por face (material, face set) vêm do triângulo original; ao
//!   colapsar, triângulos de materiais diferentes **não** são impedidos de se tocar.
//! * Malhas com arestas não-manifold ficam com essas regiões travadas.

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EdgeAttr, EditMesh, Face};
use crate::topology::Topology;
use glam::{DMat3, DVec3, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

/// Parâmetros do colapso quádrico.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecimateParams {
    /// Trava os vértices de arestas de fronteira (bordas abertas).
    pub preserve_boundary: bool,
    /// Trava os vértices de arestas `seam` e de descontinuidades de UV.
    pub preserve_seams: bool,
    /// Trava os vértices de arestas `sharp`.
    pub preserve_sharp: bool,
    /// `true`: devolve triângulos. `false`: tenta reunir triângulos coplanares
    /// em quads (`convert::quadify`), recuperando quads originais intactos.
    pub triangulate: bool,
    /// Peso 0..1 por vértice original (1 = protegido; >= 0.999 trava; valores
    /// intermediários encarecem o colapso). Comprimento = nº de vértices.
    pub vertex_group_weights: Option<Vec<f32>>,
}

impl Default for DecimateParams {
    fn default() -> Self {
        Self {
            preserve_boundary: true,
            preserve_seams: true,
            preserve_sharp: true,
            triangulate: true,
            vertex_group_weights: None,
        }
    }
}

/// Configuração de decimação para a pilha de modificadores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecimateSettings {
    /// Colapso quádrico; `ratio` em `(0, 1]`.
    Collapse { ratio: f32, params: DecimateParams },
    /// Dissolve planar; `angle` em radianos.
    Planar { angle: f32 },
}

impl Default for DecimateSettings {
    fn default() -> Self {
        DecimateSettings::Collapse {
            ratio: 0.5,
            params: DecimateParams::default(),
        }
    }
}

/// Aplica uma [`DecimateSettings`].
pub fn decimate(mesh: &EditMesh, settings: &DecimateSettings) -> ModelResult<EditMesh> {
    match settings {
        DecimateSettings::Collapse { ratio, params } => decimate_collapse(mesh, *ratio, params),
        DecimateSettings::Planar { angle } => decimate_planar(mesh, *angle),
    }
}

// ------------------------------------------------------------------ quádricas

/// Quádrica simétrica 4×4: ordem xx, xy, xz, xw, yy, yz, yw, zz, zw, ww.
#[derive(Debug, Clone, Copy, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn from_plane(n: DVec3, d: f64, w: f64) -> Self {
        let (a, b, c) = (n.x, n.y, n.z);
        Quadric([
            a * a * w,
            a * b * w,
            a * c * w,
            a * d * w,
            b * b * w,
            b * c * w,
            b * d * w,
            c * c * w,
            c * d * w,
            d * d * w,
        ])
    }

    fn add(&mut self, o: &Quadric) {
        for i in 0..10 {
            self.0[i] += o.0[i];
        }
    }

    fn sum(&self, o: &Quadric) -> Quadric {
        let mut q = *self;
        q.add(o);
        q
    }

    fn eval(&self, p: DVec3) -> f64 {
        let [xx, xy, xz, xw, yy, yz, yw, zz, zw, ww] = self.0;
        xx * p.x * p.x
            + 2.0 * xy * p.x * p.y
            + 2.0 * xz * p.x * p.z
            + 2.0 * xw * p.x
            + yy * p.y * p.y
            + 2.0 * yz * p.y * p.z
            + 2.0 * yw * p.y
            + zz * p.z * p.z
            + 2.0 * zw * p.z
            + ww
    }

    /// Ponto que minimiza a quádrica, se o sistema for bem condicionado.
    fn optimal(&self) -> Option<DVec3> {
        let [xx, xy, xz, xw, yy, yz, yw, zz, zw, _] = self.0;
        let trace = xx + yy + zz;
        if trace <= 0.0 {
            return None;
        }
        let m = DMat3::from_cols(DVec3::new(xx, xy, xz), DVec3::new(xy, yy, yz), DVec3::new(xz, yz, zz));
        let det = m.determinant();
        if det.abs() < 1e-8 * trace * trace * trace {
            return None;
        }
        let p = m.inverse() * DVec3::new(-xw, -yw, -zw);
        p.is_finite().then_some(p)
    }
}

/// Peso da penalidade de fronteira (multiplica o comprimento² da aresta).
const BOUNDARY_PENALTY: f64 = 100.0;
/// Peso `>=` este valor trava o vértice.
const LOCK_WEIGHT: f32 = 0.999;
/// Cosseno mínimo entre normais antes/depois do colapso (anti-inversão).
const FOLD_COS: f64 = 0.2;

#[derive(Clone)]
struct Tri {
    v: [u32; 3],
    uv: [Vec2; 3],
    normal: [Option<Vec3>; 3],
    src_face: u32,
    alive: bool,
}

struct Decimator {
    pos: Vec<DVec3>,
    verts: Vec<EVert>,
    quadric: Vec<Quadric>,
    locked: Vec<bool>,
    weight: Vec<f32>,
    vert_alive: Vec<bool>,
    ver: Vec<u32>,
    rep: Vec<u32>,
    tris: Vec<Tri>,
    vtris: Vec<Vec<u32>>,
    alive_tris: usize,
}

type HeapItem = Reverse<(u64, u32, u32, u32, u32)>;

impl Decimator {
    fn tri_normal(&self, v: [u32; 3]) -> DVec3 {
        let (a, b, c) = (self.pos[v[0] as usize], self.pos[v[1] as usize], self.pos[v[2] as usize]);
        (b - a).cross(c - a)
    }

    /// Vizinhos de `a` com a contagem de triângulos vivos que usam cada aresta.
    fn neighbors(&self, a: u32) -> Vec<(u32, u32)> {
        let mut raw: Vec<u32> = Vec::new();
        for &t in &self.vtris[a as usize] {
            let tri = &self.tris[t as usize];
            if !tri.alive {
                continue;
            }
            for &x in &tri.v {
                if x != a {
                    raw.push(x);
                }
            }
        }
        raw.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::new();
        for x in raw {
            match out.last_mut() {
                Some(l) if l.0 == x => l.1 += 1,
                _ => out.push((x, 1)),
            }
        }
        out
    }

    /// Melhor posição para colapsar `(a, b)`: `(custo, posição, t de a para b)`.
    fn candidate(&self, a: u32, b: u32) -> Option<(f64, DVec3, f64)> {
        let (la, lb) = (self.locked[a as usize], self.locked[b as usize]);
        if la && lb {
            return None;
        }
        let q = self.quadric[a as usize].sum(&self.quadric[b as usize]);
        let (pa, pb) = (self.pos[a as usize], self.pos[b as usize]);
        let (p, t) = if la {
            (pa, 0.0)
        } else if lb {
            (pb, 1.0)
        } else {
            let mid = (pa + pb) * 0.5;
            let mut best = (q.eval(pa), pa, 0.0);
            for (c, t) in [(pb, 1.0), (mid, 0.5)] {
                let e = q.eval(c);
                if e < best.0 {
                    best = (e, c, t);
                }
            }
            if let Some(o) = q.optimal() {
                let len = (pb - pa).length();
                if (o - mid).length() <= 2.0 * len {
                    let e = q.eval(o);
                    if e < best.0 {
                        let ab = pb - pa;
                        let t = if ab.length_squared() > 0.0 {
                            ((o - pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        best = (e, o, t);
                    }
                }
            }
            (best.1, best.2)
        };
        let factor = 1.0 + 100.0 * self.weight[a as usize].max(self.weight[b as usize]) as f64;
        let cost = q.eval(p).max(0.0) * factor;
        cost.is_finite().then_some((cost, p, t))
    }

    fn push_edge(&self, heap: &mut BinaryHeap<HeapItem>, a: u32, b: u32) {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        if let Some((cost, _, _)) = self.candidate(a, b) {
            heap.push(Reverse((cost.to_bits(), a, b, self.ver[a as usize], self.ver[b as usize])));
        }
    }

    /// Tenta colapsar `(a, b)` com a posição `p` e o parâmetro `t` (de a para b).
    fn try_collapse(&mut self, a: u32, b: u32, p: DVec3, t: f64) -> bool {
        // sobrevivente: o vértice travado, ou `a`
        let (keep, remove, tk) = if self.locked[b as usize] && !self.locked[a as usize] {
            (b, a, 0.0)
        } else {
            (a, b, t)
        };
        let na = self.neighbors(keep);
        let nb = self.neighbors(remove);
        if na.iter().chain(nb.iter()).any(|(_, c)| *c > 2) {
            return false;
        }
        // triângulos que contêm a aresta
        let shared: Vec<u32> = self.vtris[keep as usize]
            .iter()
            .copied()
            .filter(|&ti| {
                let tri = &self.tris[ti as usize];
                tri.alive && tri.v.contains(&remove)
            })
            .collect();
        if shared.is_empty() || shared.len() > 2 {
            return false;
        }
        let mut opp: Vec<u32> = shared
            .iter()
            .filter_map(|&ti| self.tris[ti as usize].v.iter().copied().find(|&x| x != keep && x != remove))
            .collect();
        opp.sort_unstable();
        // vizinhos comuns (condição de link)
        let mut common = 0usize;
        let (mut i, mut j) = (0, 0);
        while i < na.len() && j < nb.len() {
            match na[i].0.cmp(&nb[j].0) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    if na[i].0 != remove && nb[j].0 != keep {
                        common += 1;
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        if common != opp.len() {
            return false;
        }
        let boundary = |n: &[(u32, u32)]| n.iter().any(|(_, c)| *c == 1);
        if shared.len() == 2 && boundary(&na) && boundary(&nb) {
            return false;
        }
        // não inverter normais dos triângulos que sobrevivem
        for &vertex in &[keep, remove] {
            for &ti in &self.vtris[vertex as usize] {
                let tri = &self.tris[ti as usize];
                if !tri.alive || shared.contains(&ti) {
                    continue;
                }
                let old = self.tri_normal(tri.v);
                let mut nv = tri.v;
                for x in &mut nv {
                    if *x == vertex {
                        *x = u32::MAX;
                    }
                }
                let pts: Vec<DVec3> = nv
                    .iter()
                    .map(|&x| if x == u32::MAX { p } else { self.pos[x as usize] })
                    .collect();
                let new = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
                let (lo, ln) = (old.length(), new.length());
                if ln <= 1e-14 * lo.max(1e-30) {
                    return false;
                }
                if lo > 1e-18 && old.dot(new) / (lo * ln) <= FOLD_COS {
                    return false;
                }
            }
        }

        // ---- executa
        // UVs dos triângulos removidos (sobrevivente, removido)
        let mut dead_uv: Vec<(Vec2, Vec2)> = Vec::new();
        for &ti in &shared {
            let tri = &self.tris[ti as usize];
            let sk = tri.v.iter().position(|&x| x == keep).unwrap_or(0);
            let sr = tri.v.iter().position(|&x| x == remove).unwrap_or(0);
            dead_uv.push((tri.uv[sk], tri.uv[sr]));
        }
        for &ti in &shared {
            self.tris[ti as usize].alive = false;
            self.alive_tris -= 1;
        }
        let near = |a: Vec2, b: Vec2| (a - b).abs().max_element() < 1e-4;
        let remove_tris = std::mem::take(&mut self.vtris[remove as usize]);
        for &ti in &remove_tris {
            if !self.tris[ti as usize].alive {
                continue;
            }
            let tri = &mut self.tris[ti as usize];
            for s in 0..3 {
                if tri.v[s] == remove {
                    tri.v[s] = keep;
                    if let Some(d) = dead_uv.iter().find(|d| near(d.1, tri.uv[s])) {
                        tri.uv[s] += ((1.0 - tk) as f32) * (d.0 - d.1);
                    }
                    tri.normal[s] = None;
                }
            }
            self.vtris[keep as usize].push(ti);
        }
        // corners do sobrevivente (não os que vieram de `remove`): ajusta UV
        let came_from_remove: HashSet<u32> = remove_tris.iter().copied().collect();
        for &ti in &self.vtris[keep as usize].clone() {
            if came_from_remove.contains(&ti) {
                continue;
            }
            let tri = &mut self.tris[ti as usize];
            if !tri.alive {
                continue;
            }
            for s in 0..3 {
                if tri.v[s] == keep {
                    if let Some(d) = dead_uv.iter().find(|d| near(d.0, tri.uv[s])) {
                        tri.uv[s] += (tk as f32) * (d.1 - d.0);
                    }
                    tri.normal[s] = None;
                }
            }
        }
        // limpa a lista de triângulos do sobrevivente
        let mut list = std::mem::take(&mut self.vtris[keep as usize]);
        list.retain(|&ti| self.tris[ti as usize].alive);
        list.sort_unstable();
        list.dedup();
        self.vtris[keep as usize] = list;
        // triângulos mortos saem das listas dos outros vértices (lazy: filtrados por `alive`)

        let mut merged = EVert::lerp(&self.verts[keep as usize], &self.verts[remove as usize], tk as f32);
        merged.co = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
        self.verts[keep as usize] = merged;
        self.pos[keep as usize] = p;
        let qr = self.quadric[remove as usize];
        self.quadric[keep as usize].add(&qr);
        self.weight[keep as usize] = self.weight[keep as usize].max(self.weight[remove as usize]);
        self.vert_alive[remove as usize] = false;
        self.rep[remove as usize] = keep;
        self.ver[keep as usize] += 1;
        true
    }

    fn find(&self, mut v: u32) -> u32 {
        while self.rep[v as usize] != v {
            v = self.rep[v as usize];
        }
        v
    }
}

/// Colapso de arestas por erro quádrico. Devolve uma malha nova (a entrada não
/// é alterada).
///
/// * `ratio`: fração de **triângulos** a manter, em `(0, 1]`. `1.0` devolve a malha
///   clonada sem triangular. O alvo é `max(4, round(ratio · triângulos))`.
/// * `params`: ver [`DecimateParams`] (travas, `triangulate`, pesos por vértice).
/// * Topologia: a condição de link impede pinçamentos/buracos; uma malha fechada
///   manifold continua fechada, manifold e com a mesma característica de Euler.
/// * O resultado pode ter mais triângulos que o alvo se as travas ou a topologia
///   impedirem novos colapsos.
/// * Erros: `ratio` fora de `(0, 1]` ou não finito, pesos com tamanho errado ou
///   fora de `0..=1`, malha inválida.
pub fn decimate_collapse(mesh: &EditMesh, ratio: f32, params: &DecimateParams) -> ModelResult<EditMesh> {
    if !ratio.is_finite() || ratio <= 0.0 || ratio > 1.0 {
        return Err(ModelError::InvalidParameter(format!("ratio deve estar em (0, 1], recebeu {ratio}")));
    }
    if let Some(w) = &params.vertex_group_weights {
        if w.len() != mesh.verts.len() {
            return Err(ModelError::InvalidParameter(format!(
                "vertex_group_weights tem {} valores para {} vértices",
                w.len(),
                mesh.verts.len()
            )));
        }
        if w.iter().any(|x| !x.is_finite() || !(0.0..=1.0).contains(x)) {
            return Err(ModelError::InvalidParameter("vertex_group_weights deve estar em 0..=1".into()));
        }
    }
    mesh.validate()?;
    if ratio >= 1.0 || mesh.faces.is_empty() {
        return Ok(mesh.clone());
    }

    // ---- triangulação interna
    let nv = mesh.verts.len();
    let mut tris: Vec<Tri> = Vec::with_capacity(mesh.triangle_count());
    for (fi, face) in mesh.faces.iter().enumerate() {
        let pts: Vec<Vec3> = face.corners.iter().map(|c| mesh.verts[c.vert as usize].co).collect();
        for t in triangulate_polygon(&pts) {
            let c = [face.corners[t[0]], face.corners[t[1]], face.corners[t[2]]];
            if c[0].vert == c[1].vert || c[1].vert == c[2].vert || c[0].vert == c[2].vert {
                continue;
            }
            tris.push(Tri {
                v: [c[0].vert, c[1].vert, c[2].vert],
                uv: [c[0].uv, c[1].uv, c[2].uv],
                normal: [c[0].normal, c[1].normal, c[2].normal],
                src_face: fi as u32,
                alive: true,
            });
        }
    }
    let total_tris = tris.len();
    if total_tris == 0 {
        return Ok(mesh.clone());
    }
    let target = ((ratio as f64 * total_tris as f64).round() as usize).max(4);
    if total_tris <= target {
        return Ok(mesh.clone());
    }

    let mut d = Decimator {
        pos: mesh.verts.iter().map(|v| v.co.as_dvec3()).collect(),
        verts: mesh.verts.clone(),
        quadric: vec![Quadric::default(); nv],
        locked: vec![false; nv],
        weight: params.vertex_group_weights.clone().unwrap_or_else(|| vec![0.0; nv]),
        vert_alive: vec![true; nv],
        ver: vec![0; nv],
        rep: (0..nv as u32).collect(),
        vtris: vec![Vec::new(); nv],
        alive_tris: tris.len(),
        tris,
    };
    for (ti, tri) in d.tris.iter().enumerate() {
        for &v in &tri.v {
            d.vtris[v as usize].push(ti as u32);
        }
    }
    for w in 0..nv {
        if d.weight[w] >= LOCK_WEIGHT {
            d.locked[w] = true;
        }
    }

    // quádricas de face
    for tri in &d.tris {
        let n = d.tri_normal(tri.v);
        let len = n.length();
        if len < 1e-18 {
            continue;
        }
        let nn = n / len;
        let area = len * 0.5;
        let dd = -nn.dot(d.pos[tri.v[0] as usize]);
        let q = Quadric::from_plane(nn, dd, area);
        for &v in &tri.v {
            d.quadric[v as usize].add(&q);
        }
    }

    // mapa de arestas -> (triângulo, slot)
    let mut edge_map: HashMap<(u32, u32), Vec<(u32, u8)>> = HashMap::new();
    for (ti, tri) in d.tris.iter().enumerate() {
        for s in 0..3u8 {
            let (a, b) = (tri.v[s as usize], tri.v[(s as usize + 1) % 3]);
            edge_map.entry(edge_key(a, b)).or_default().push((ti as u32, s));
        }
    }
    let mut edge_list: Vec<(u32, u32)> = edge_map.keys().copied().collect();
    edge_list.sort_unstable();

    for &(a, b) in &edge_list {
        let entries = &edge_map[&(a, b)];
        let attr = mesh.edge_attr(a, b);
        let mut lock = false;
        match entries.len() {
            1 => {
                if params.preserve_boundary {
                    lock = true;
                } else {
                    // penalidade: plano perpendicular à face contendo a aresta
                    let tri = &d.tris[entries[0].0 as usize];
                    let n = d.tri_normal(tri.v);
                    let (pa, pb) = (d.pos[a as usize], d.pos[b as usize]);
                    let e = pb - pa;
                    let m = e.cross(n);
                    if m.length() > 1e-18 {
                        let mn = m.normalize();
                        let q = Quadric::from_plane(mn, -mn.dot(pa), BOUNDARY_PENALTY * e.length_squared());
                        d.quadric[a as usize].add(&q);
                        d.quadric[b as usize].add(&q);
                    }
                }
            }
            2 => {
                if params.preserve_seams {
                    // descontinuidade de UV entre os dois triângulos
                    let uv_of = |(ti, s): (u32, u8), vert: u32| {
                        let tri = &d.tris[ti as usize];
                        let idx = tri.v.iter().position(|&x| x == vert).unwrap_or(s as usize);
                        tri.uv[idx]
                    };
                    for vert in [a, b] {
                        let (u0, u1) = (uv_of(entries[0], vert), uv_of(entries[1], vert));
                        if (u0 - u1).abs().max_element() > 1e-5 {
                            lock = true;
                        }
                    }
                }
            }
            _ => lock = true,
        }
        if attr.seam && params.preserve_seams {
            lock = true;
        }
        if attr.sharp && params.preserve_sharp {
            lock = true;
        }
        if lock {
            d.locked[a as usize] = true;
            d.locked[b as usize] = true;
        }
    }

    // ---- laço principal
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(edge_list.len());
    for &(a, b) in &edge_list {
        d.push_edge(&mut heap, a, b);
    }
    while d.alive_tris > target {
        let Some(Reverse((_, a, b, va, vb))) = heap.pop() else {
            break;
        };
        if !d.vert_alive[a as usize] || !d.vert_alive[b as usize] || d.ver[a as usize] != va || d.ver[b as usize] != vb {
            continue;
        }
        let Some((_, p, t)) = d.candidate(a, b) else {
            continue;
        };
        if !d.try_collapse(a, b, p, t) {
            continue;
        }
        let keep = d.find(a);
        for (x, _) in d.neighbors(keep) {
            d.push_edge(&mut heap, keep, x);
        }
    }

    // ---- montagem do resultado
    let mut used = vec![false; nv];
    for tri in d.tris.iter().filter(|t| t.alive) {
        for &v in &tri.v {
            used[v as usize] = true;
        }
    }
    let orig_used = mesh.used_verts();
    let mut new_id = vec![u32::MAX; nv];
    let mut out = EditMesh::new(mesh.name.clone());
    for v in 0..nv {
        if d.vert_alive[v] && (used[v] || !orig_used[v]) {
            new_id[v] = out.verts.len() as u32;
            out.verts.push(d.verts[v].clone());
        }
    }
    let mut final_edges: HashSet<(u32, u32)> = HashSet::new();
    for tri in d.tris.iter().filter(|t| t.alive) {
        let src = &mesh.faces[tri.src_face as usize];
        let mut face = src.with_verts_from(&[new_id[tri.v[0] as usize], new_id[tri.v[1] as usize], new_id[tri.v[2] as usize]]);
        for s in 0..3 {
            face.corners[s].uv = tri.uv[s];
            face.corners[s].normal = tri.normal[s];
            final_edges.insert(edge_key(tri.v[s], tri.v[(s + 1) % 3]));
        }
        out.faces.push(face);
    }
    let orig_topo = Topology::build(mesh);
    let mut attrs: BTreeMap<(u32, u32), EdgeAttr> = BTreeMap::new();
    for (&(a, b), attr) in &mesh.edge_attrs {
        let (ra, rb) = (d.find(a), d.find(b));
        if ra == rb {
            continue;
        }
        let key = edge_key(ra, rb);
        let was_wire = orig_topo.edge(a, b).is_some_and(|e| e.faces.is_empty());
        if !was_wire && !final_edges.contains(&key) {
            continue;
        }
        let (na, nb) = (new_id[ra as usize], new_id[rb as usize]);
        if na == u32::MAX || nb == u32::MAX {
            continue;
        }
        let nk = edge_key(na, nb);
        let merged = match attrs.get(&nk) {
            Some(o) => EdgeAttr {
                sharp: o.sharp || attr.sharp,
                seam: o.seam || attr.seam,
                crease: o.crease.max(attr.crease),
                bevel_weight: o.bevel_weight.max(attr.bevel_weight),
            },
            None => *attr,
        };
        attrs.insert(nk, merged);
    }
    out.edge_attrs = attrs;
    if !params.triangulate {
        crate::convert::quadify(&mut out, 1e-3);
    }
    Ok(out)
}

// ------------------------------------------------------------- dissolve planar

/// Dissolve planar: reúne faces vizinhas cujas normais diferem no máximo `angle`
/// (radianos, `0..=π`) em n-gons e remove vértices colineares remanescentes.
///
/// * Não dissolve através de arestas `sharp`/`seam` nem entre faces de material,
///   face-set ou visibilidade diferentes. O ângulo é medido contra a normal da
///   face semente da região **e** contra a vizinha, evitando deriva em superfícies
///   curvas.
/// * Regiões cujo contorno não é um único laço simples (com buracos ou pinçamentos)
///   permanecem intactas.
/// * UV e normal customizada de cada canto do n-gon vêm da face que o originou.
/// * Vértices com exatamente duas arestas, colineares dentro de `angle`, sem
///   atributos de aresta, em faces com ≥ 4 cantos, são removidos das faces.
/// * Erros: `angle` não finito ou fora de `0..=π`; malha inválida.
pub fn decimate_planar(mesh: &EditMesh, angle: f32) -> ModelResult<EditMesh> {
    if !angle.is_finite() || !(0.0..=std::f32::consts::PI).contains(&angle) {
        return Err(ModelError::InvalidParameter(format!("angle deve estar em 0..=π, recebeu {angle}")));
    }
    mesh.validate()?;
    if mesh.faces.len() < 2 {
        return Ok(mesh.clone());
    }
    let topo = Topology::build(mesh);
    let nf = mesh.faces.len();
    let normals: Vec<Vec3> = (0..nf as u32).map(|f| mesh.face_normal(f)).collect();
    let cos_t = angle.cos() - 1e-6;

    // regiões por crescimento
    let mut region = vec![u32::MAX; nf];
    let mut regions: Vec<Vec<u32>> = Vec::new();
    for seed in 0..nf as u32 {
        if region[seed as usize] != u32::MAX {
            continue;
        }
        let id = regions.len() as u32;
        region[seed as usize] = id;
        let mut members = vec![seed];
        let mut stack = vec![seed];
        let ref_n = normals[seed as usize];
        if ref_n != Vec3::ZERO {
            while let Some(f) = stack.pop() {
                for &e in &topo.face_edges[f as usize] {
                    let edge = &topo.edges[e as usize];
                    if edge.faces.len() != 2 {
                        continue;
                    }
                    let attr = mesh.edge_attr(edge.verts.0, edge.verts.1);
                    if attr.sharp || attr.seam {
                        continue;
                    }
                    let g = if edge.faces[0] == f { edge.faces[1] } else { edge.faces[0] };
                    if g == f || region[g as usize] != u32::MAX {
                        continue;
                    }
                    let (fa, fb) = (&mesh.faces[f as usize], &mesh.faces[g as usize]);
                    if fa.material != fb.material || fa.face_set != fb.face_set || fa.hidden != fb.hidden {
                        continue;
                    }
                    let ng = normals[g as usize];
                    if ng.dot(ref_n) < cos_t || ng.dot(normals[f as usize]) < cos_t {
                        continue;
                    }
                    region[g as usize] = id;
                    members.push(g);
                    stack.push(g);
                }
            }
        }
        members.sort_unstable();
        regions.push(members);
    }

    let mut faces: Vec<Option<Face>> = mesh.faces.iter().cloned().map(Some).collect();
    for (id, members) in regions.iter().enumerate() {
        if members.len() < 2 {
            continue;
        }
        // semi-arestas de contorno
        let mut next: HashMap<u32, (u32, u32, usize)> = HashMap::new();
        let mut pinch = false;
        for &f in members {
            let face = &mesh.faces[f as usize];
            let k = face.corners.len();
            for i in 0..k {
                let e = topo.face_edges[f as usize][i] as usize;
                let edge = &topo.edges[e];
                let interior = edge.faces.len() == 2 && edge.faces.iter().all(|g| region[*g as usize] == id as u32);
                if interior {
                    continue;
                }
                let (a, b) = (face.corners[i].vert, face.corners[(i + 1) % k].vert);
                if next.insert(a, (b, f, i)).is_some() {
                    pinch = true;
                }
            }
        }
        if pinch || next.is_empty() {
            continue;
        }
        let start = *next.keys().min().unwrap_or(&0);
        let mut loop_corners: Vec<Corner> = Vec::with_capacity(next.len());
        let mut cur = start;
        let mut ok = true;
        loop {
            let Some(&(b, f, i)) = next.get(&cur) else {
                ok = false;
                break;
            };
            loop_corners.push(mesh.faces[f as usize].corners[i]);
            cur = b;
            if cur == start {
                break;
            }
            if loop_corners.len() > next.len() {
                ok = false;
                break;
            }
        }
        if !ok || loop_corners.len() != next.len() || loop_corners.len() < 3 {
            continue; // contorno com buraco ou não simples
        }
        let mut poly = mesh.faces[members[0] as usize].clone();
        poly.corners = loop_corners;
        faces[members[0] as usize] = Some(poly);
        for &f in &members[1..] {
            faces[f as usize] = None;
        }
    }

    let mut out = EditMesh {
        name: mesh.name.clone(),
        verts: mesh.verts.clone(),
        faces: faces.into_iter().flatten().collect(),
        edge_attrs: mesh.edge_attrs.clone(),
    };
    dissolve_collinear(&mut out, angle);

    // remove atributos de arestas que deixaram de existir (exceto arestas soltas originais)
    let new_topo = Topology::build(&out);
    let orig_topo = &topo;
    out.edge_attrs.retain(|&(a, b), _| {
        let alive = new_topo.edge(a, b).is_some_and(|e| !e.faces.is_empty());
        let was_wire = orig_topo.edge(a, b).is_some_and(|e| e.faces.is_empty());
        alive || was_wire
    });
    // descarta vértices que ficaram sem uso (vértices originalmente soltos permanecem)
    let orig_used = mesh.used_verts();
    let used = out.used_verts();
    let keep: Vec<bool> = (0..out.verts.len())
        .map(|v| used[v] || !orig_used[v] || out.edge_attrs.keys().any(|k| k.0 as usize == v || k.1 as usize == v))
        .collect();
    out.compact_verts(&keep);
    Ok(out)
}

/// Remove vértices colineares de grau 2 (veja [`decimate_planar`]).
fn dissolve_collinear(mesh: &mut EditMesh, angle: f32) {
    let cos_t = angle.cos() - 1e-6;
    for _ in 0..16 {
        let topo = Topology::build(mesh);
        let mut taken: HashSet<u32> = HashSet::new();
        let mut plan: Vec<u32> = Vec::new();
        for v in 0..mesh.verts.len() as u32 {
            let edges = &topo.vert_edges[v as usize];
            if edges.len() != 2 || topo.vert_faces[v as usize].is_empty() {
                continue;
            }
            let ends: Vec<u32> = edges
                .iter()
                .map(|&e| {
                    let (a, b) = topo.edges[e as usize].verts;
                    if a == v {
                        b
                    } else {
                        a
                    }
                })
                .collect();
            let (p, n) = (ends[0], ends[1]);
            if p == n || taken.contains(&p) || taken.contains(&n) {
                continue;
            }
            if !mesh.edge_attr(v, p).is_default() || !mesh.edge_attr(v, n).is_default() {
                continue;
            }
            if edges.iter().any(|&e| topo.edges[e as usize].faces.is_empty()) {
                continue;
            }
            let d1 = (mesh.co(v) - mesh.co(p)).normalize_or_zero();
            let d2 = (mesh.co(n) - mesh.co(v)).normalize_or_zero();
            if d1 == Vec3::ZERO || d2 == Vec3::ZERO || d1.dot(d2) < cos_t {
                continue;
            }
            // todas as faces incidentes precisam ter ≥ 4 cantos e (p, v, n) consecutivos
            let mut fine = true;
            let mut seen_faces: HashSet<u32> = HashSet::new();
            for &f in &topo.vert_faces[v as usize] {
                if !seen_faces.insert(f) {
                    fine = false;
                    break;
                }
                let face = &mesh.faces[f as usize];
                let k = face.corners.len();
                let Some(ci) = face.corners.iter().position(|c| c.vert == v) else {
                    fine = false;
                    break;
                };
                let (a, b) = (face.corners[(ci + k - 1) % k].vert, face.corners[(ci + 1) % k].vert);
                if k < 4 || !((a == p && b == n) || (a == n && b == p)) {
                    fine = false;
                    break;
                }
            }
            if fine {
                taken.insert(v);
                plan.push(v);
            }
        }
        if plan.is_empty() {
            break;
        }
        for v in plan {
            for f in &topo.vert_faces[v as usize] {
                let face = &mut mesh.faces[*f as usize];
                if face.corners.len() > 3 {
                    if let Some(ci) = face.corners.iter().position(|c| c.vert == v) {
                        face.corners.remove(ci);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};

    fn point_tri_dist(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
        // distância de ponto a triângulo (Ericson, Real-Time Collision Detection)
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return ap.length();
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return bp.length();
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            return (p - (a + ab * v)).length();
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return cp.length();
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            return (p - (a + ac * w)).length();
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            return (p - (b + (c - b) * w)).length();
        }
        let denom = 1.0 / (va + vb + vc);
        let v = vb * denom;
        let w = vc * denom;
        (p - (a + ab * v + ac * w)).length()
    }

    fn dist_to_mesh(p: Vec3, m: &EditMesh) -> f32 {
        let mut best = f32::MAX;
        for f in &m.faces {
            let q: Vec<Vec3> = f.corners.iter().map(|c| m.verts[c.vert as usize].co).collect();
            for i in 1..q.len() - 1 {
                best = best.min(point_tri_dist(p, q[0], q[i], q[i + 1]));
            }
        }
        best
    }

    fn tri_area(m: &EditMesh) -> f64 {
        (0..m.face_count() as u32).map(|f| m.face_area(f) as f64).sum()
    }

    fn volume(m: &EditMesh) -> f64 {
        let mut vol = 0.0f64;
        for f in &m.faces {
            let q: Vec<DVec3> = f.corners.iter().map(|c| m.verts[c.vert as usize].co.as_dvec3()).collect();
            for i in 1..q.len() - 1 {
                vol += q[0].dot(q[i].cross(q[i + 1])) / 6.0;
            }
        }
        vol
    }

    #[test]
    fn sphere_24k_triangles_to_25_percent_keeps_shape_and_topology() {
        let s = primitives::uv_sphere(1.0, 100, 120);
        let tris = s.triangle_count();
        assert!((23_000..25_000).contains(&tris), "{tris}");
        let t0 = std::time::Instant::now();
        let m = decimate_collapse(&s, 0.25, &DecimateParams::default()).unwrap();
        let elapsed = t0.elapsed();
        m.validate().unwrap();
        let topo = Topology::build(&m);
        assert!(topo.is_closed_manifold(), "continua fechada e manifold");
        assert_eq!(topo.euler_characteristic(&m), 2);
        assert!(m.faces.iter().all(|f| f.len() == 3));
        let out_tris = m.face_count();
        let target = (tris as f64 * 0.25).round() as usize;
        assert!(out_tris <= target + 4 && out_tris as f64 >= target as f64 * 0.9, "{out_tris} vs alvo {target}");
        assert_outward(&m);
        // forma: distância dos vértices novos à esfera ideal e dos originais à nova superfície
        for v in &m.verts {
            assert!((v.co.length() - 1.0).abs() < 0.01, "vértice a {}", v.co.length());
        }
        let mut hd = 0.0f32;
        for v in s.verts.iter().step_by(11) {
            hd = hd.max(dist_to_mesh(v.co, &m));
        }
        assert!(hd < 0.01, "Hausdorff aproximado {hd}");
        let vol_ratio = volume(&m) / volume(&s);
        assert!((0.97..=1.001).contains(&vol_ratio), "volume {vol_ratio}");
        assert!(elapsed.as_secs_f32() < 60.0, "demorou {elapsed:?}");
    }

    #[test]
    fn ratio_one_is_identity_and_validation() {
        let s = primitives::quad_sphere(1.0, 6);
        let before = s.clone();
        assert_eq!(decimate_collapse(&s, 1.0, &DecimateParams::default()).unwrap(), s);
        for bad in [0.0, -0.5, 1.5, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                decimate_collapse(&s, bad, &DecimateParams::default()),
                Err(ModelError::InvalidParameter(_))
            ));
        }
        let p = DecimateParams {
            vertex_group_weights: Some(vec![0.0; 3]),
            ..DecimateParams::default()
        };
        assert!(decimate_collapse(&s, 0.5, &p).is_err());
        let p = DecimateParams {
            vertex_group_weights: Some(vec![2.0; s.vert_count()]),
            ..DecimateParams::default()
        };
        assert!(decimate_collapse(&s, 0.5, &p).is_err());
        let p = DecimateParams {
            vertex_group_weights: Some(vec![f32::NAN; s.vert_count()]),
            ..DecimateParams::default()
        };
        assert!(decimate_collapse(&s, 0.5, &p).is_err());
        let mut broken = s.clone();
        broken.faces[0].corners[0].vert = 9999;
        assert!(decimate_collapse(&broken, 0.5, &DecimateParams::default()).is_err());
        assert_eq!(s, before);
        // malha vazia e alvo já atingido
        let e = EditMesh::new("e");
        assert_eq!(decimate_collapse(&e, 0.5, &DecimateParams::default()).unwrap(), e);
        let tet = primitives::cube(1.0);
        assert!(decimate_collapse(&tet, 0.99, &DecimateParams::default()).is_ok());
    }

    #[test]
    fn quad_sphere_and_ngons_are_triangulated_and_reduced() {
        for m in [
            primitives::quad_sphere(1.0, 16),
            primitives::capsule(0.5, 1.0, 24, 8),
            primitives::cylinder(1.0, 2.0, 24, 6, true),
            primitives::torus(1.0, 0.3, 32, 16),
        ] {
            let tris = m.triangle_count();
            let d = decimate_collapse(&m, 0.5, &DecimateParams::default()).unwrap();
            d.validate().unwrap();
            let t = Topology::build(&d);
            assert!(t.is_closed_manifold(), "{}", m.name);
            assert_eq!(t.euler_characteristic(&d), Topology::build(&m).euler_characteristic(&m), "{}", m.name);
            assert!(d.face_count() < tris, "{}", m.name);
            assert!(d.face_count() as f64 <= tris as f64 * 0.55, "{}: {} de {}", m.name, d.face_count(), tris);
        }
    }

    #[test]
    fn determinism() {
        let s = primitives::uv_sphere(1.0, 30, 40);
        let a = decimate_collapse(&s, 0.3, &DecimateParams::default()).unwrap();
        let b = decimate_collapse(&s, 0.3, &DecimateParams::default()).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn boundary_is_preserved_when_requested() {
        let g = primitives::grid(20, 20, 2.0, 2.0);
        let orig_boundary: HashSet<[i64; 3]> = {
            let t = Topology::build(&g);
            (0..g.vert_count() as u32)
                .filter(|&v| t.is_boundary_vert(v))
                .map(|v| q(g.co(v)))
                .collect()
        };
        fn q(p: Vec3) -> [i64; 3] {
            [(p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64]
        }
        let m = decimate_collapse(&g, 0.3, &DecimateParams::default()).unwrap();
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.euler_characteristic(&m), 1);
        let new_boundary: HashSet<[i64; 3]> = (0..m.vert_count() as u32)
            .filter(|&v| t.is_boundary_vert(v))
            .map(|v| q(m.co(v)))
            .collect();
        assert_eq!(new_boundary, orig_boundary, "fronteira intacta");
        assert!(m.face_count() < g.triangle_count());
        assert!(m.verts.iter().all(|v| v.co.y.abs() < 1e-6));
    }

    #[test]
    fn boundary_can_be_decimated_with_penalty_keeping_outline() {
        let g = primitives::grid(20, 20, 2.0, 2.0);
        let p = DecimateParams {
            preserve_boundary: false,
            ..DecimateParams::default()
        };
        let m = decimate_collapse(&g, 0.1, &p).unwrap();
        m.validate().unwrap();
        let t = Topology::build(&m);
        assert_eq!(t.non_manifold_edge_count(), 0);
        assert_eq!(t.euler_characteristic(&m), 1);
        assert!(m.face_count() <= (g.triangle_count() as f64 * 0.1) as usize + 4, "{}", m.face_count());
        // plano: área e contorno preservados (cantos presos pela penalidade)
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo - Vec3::new(-1.0, 0.0, -1.0)).length() < 1e-3 && (hi - Vec3::new(1.0, 0.0, 1.0)).length() < 1e-3);
        assert!((tri_area(&m) - 4.0).abs() < 1e-2, "{}", tri_area(&m));
        assert!(m.verts.iter().all(|v| v.co.y.abs() < 1e-5));
    }

    #[test]
    fn uv_is_kept_consistent_on_planar_mesh() {
        let g = primitives::grid(16, 16, 2.0, 2.0);
        let p = DecimateParams {
            preserve_boundary: false,
            ..DecimateParams::default()
        };
        let m = decimate_collapse(&g, 0.15, &p).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                let pos = m.co(c.vert);
                // grid: u = x/size + .5 ; v = z/size + .5
                assert!((c.uv.x - (pos.x / 2.0 + 0.5)).abs() < 2e-3, "u {} vs x {}", c.uv.x, pos.x);
                assert!((c.uv.y - (pos.z / 2.0 + 0.5)).abs() < 2e-3, "v {} vs z {}", c.uv.y, pos.z);
            }
        }
    }

    #[test]
    fn seams_and_sharp_edges_are_protected() {
        let mut s = primitives::quad_sphere(1.0, 12);
        let topo = Topology::build(&s);
        let mut equator: Vec<(u32, u32)> = Vec::new();
        for e in &topo.edges {
            if s.co(e.verts.0).y.abs() < 1e-6 && s.co(e.verts.1).y.abs() < 1e-6 {
                equator.push(e.verts);
            }
        }
        assert!(!equator.is_empty());
        for &(a, b) in &equator {
            s.set_seam(a, b, true);
        }
        let eq_pos: HashSet<[i64; 3]> = equator
            .iter()
            .flat_map(|&(a, b)| [s.co(a), s.co(b)])
            .map(|p| [(p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64])
            .collect();
        let m = decimate_collapse(&s, 0.3, &DecimateParams::default()).unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        // vértices do equador intactos e arestas de costura ainda existem
        let kept: HashSet<[i64; 3]> = m
            .verts
            .iter()
            .map(|v| v.co)
            .filter(|p| p.y.abs() < 1e-6)
            .map(|p| [(p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64])
            .collect();
        assert!(eq_pos.is_subset(&kept), "equador preservado");
        assert!(m.edge_attrs.values().filter(|a| a.seam).count() >= equator.len());
        // sem preserve_seams o equador é livre para ser reduzido
        let free = DecimateParams {
            preserve_seams: false,
            ..DecimateParams::default()
        };
        let m2 = decimate_collapse(&s, 0.1, &free).unwrap();
        assert!(m2.face_count() < m.face_count());
        // sharp
        let mut s2 = primitives::quad_sphere(1.0, 12);
        for &(a, b) in &equator {
            s2.set_sharp(a, b, true);
        }
        let m3 = decimate_collapse(&s2, 0.3, &DecimateParams::default()).unwrap();
        assert!(m3.edge_attrs.values().filter(|a| a.sharp).count() >= equator.len());
        let m4 = decimate_collapse(
            &s2,
            0.3,
            &DecimateParams {
                preserve_sharp: false,
                ..DecimateParams::default()
            },
        )
        .unwrap();
        assert!(m4.face_count() <= m3.face_count());
    }

    #[test]
    fn uv_islands_boundaries_count_as_seams() {
        // o cubo subdividido tem 6 ilhas de UV: a borda de cada ilha deve sobreviver
        let c = primitives::cube_subdivided(2.0, 6);
        let m = decimate_collapse(&c, 0.3, &DecimateParams::default()).unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        // os 8 cantos do cubo continuam presentes
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let corner = Vec3::new(sx, sy, sz);
                    assert!(m.verts.iter().any(|v| (v.co - corner).length() < 1e-5), "canto {corner:?} perdido");
                }
            }
        }
        // sem proteção, o cubo plano reduz mais
        let free = decimate_collapse(
            &c,
            0.1,
            &DecimateParams {
                preserve_seams: false,
                preserve_sharp: false,
                ..DecimateParams::default()
            },
        )
        .unwrap();
        assert!(free.face_count() < m.face_count());
        assert!(Topology::build(&free).is_closed_manifold());
    }

    #[test]
    fn vertex_weights_protect_a_region() {
        let s = primitives::uv_sphere(1.0, 40, 48);
        let w: Vec<f32> = s.verts.iter().map(|v| if v.co.y > 0.0 { 1.0 } else { 0.0 }).collect();
        let m = decimate_collapse(
            &s,
            0.2,
            &DecimateParams {
                vertex_group_weights: Some(w),
                ..DecimateParams::default()
            },
        )
        .unwrap();
        let north_before = s.verts.iter().filter(|v| v.co.y > 0.0).count();
        let north_after = m.verts.iter().filter(|v| v.co.y > 0.0).count();
        let south_before = s.verts.iter().filter(|v| v.co.y <= 0.0).count();
        let south_after = m.verts.iter().filter(|v| v.co.y <= 0.0).count();
        assert_eq!(north_after, north_before, "região protegida intacta");
        assert!(south_after < south_before / 2, "{south_after} vs {south_before}");
        assert!(Topology::build(&m).is_closed_manifold());
        // peso intermediário: custo maior, mas não trava
        let half = vec![0.5; s.vert_count()];
        let mh = decimate_collapse(
            &s,
            0.5,
            &DecimateParams {
                vertex_group_weights: Some(half),
                ..DecimateParams::default()
            },
        )
        .unwrap();
        assert!(mh.face_count() < s.triangle_count());
    }

    #[test]
    fn attributes_survive_decimation() {
        let mut s = primitives::quad_sphere(1.0, 16);
        for v in &mut s.verts {
            v.color = [0.3, 0.6, 0.9, 1.0];
            v.mask = 0.4;
            v.joints = [3, 0, 0, 0];
            v.weights = [1.0, 0.0, 0.0, 0.0];
        }
        for f in &mut s.faces {
            f.material = 5;
            f.face_set = 2;
            f.smooth = true;
        }
        let m = decimate_collapse(&s, 0.3, &DecimateParams::default()).unwrap();
        for v in &m.verts {
            for (a, b) in v.color.iter().zip([0.3, 0.6, 0.9, 1.0]) {
                assert!((a - b).abs() < 1e-5);
            }
            assert!((v.mask - 0.4).abs() < 1e-5);
            assert_eq!(v.joints[0], 3);
            assert!((v.weights[0] - 1.0).abs() < 1e-6);
        }
        assert!(m.faces.iter().all(|f| f.material == 5 && f.face_set == 2 && f.smooth));
    }

    #[test]
    fn custom_normals_are_dropped_only_where_geometry_changed() {
        let mut s = primitives::quad_sphere(1.0, 12);
        for f in &mut s.faces {
            for c in &mut f.corners {
                c.normal = Some(Vec3::Y);
            }
        }
        let m = decimate_collapse(&s, 0.5, &DecimateParams::default()).unwrap();
        let with = m.faces.iter().flat_map(|f| &f.corners).filter(|c| c.normal.is_some()).count();
        let without = m.faces.iter().flat_map(|f| &f.corners).filter(|c| c.normal.is_none()).count();
        assert!(with > 0 && without > 0);
    }

    #[test]
    fn triangulate_false_restores_untouched_quads() {
        let g = primitives::grid(8, 8, 1.0, 1.0);
        let p = DecimateParams {
            triangulate: false,
            ..DecimateParams::default()
        };
        // ratio 0.95: pouquíssimas mudanças, a maior parte dos quads é recuperada
        let m = decimate_collapse(&g, 0.95, &p).unwrap();
        m.validate().unwrap();
        let quads = m.faces.iter().filter(|f| f.len() == 4).count();
        assert!(quads > 20, "{quads}");
        let tri = decimate_collapse(&g, 0.95, &DecimateParams::default()).unwrap();
        assert!(tri.faces.iter().all(|f| f.len() == 3));
    }

    #[test]
    fn non_manifold_and_loose_geometry_do_not_break() {
        let mut m = primitives::quad_sphere(1.0, 8);
        // aresta extra compartilhada por 3 faces (aba) + vértice solto + aresta solta
        let a = m.add_vert(Vec3::new(0.0, 0.0, 5.0));
        m.add_face(&[0, 1, a]).unwrap();
        let lone = m.add_vert(Vec3::splat(9.0));
        let lone2 = m.add_vert(Vec3::splat(9.5));
        m.set_sharp(lone, lone2, true);
        let d = decimate_collapse(&m, 0.4, &DecimateParams::default()).unwrap();
        d.validate().unwrap();
        assert!(d.verts.iter().any(|v| v.co == Vec3::splat(9.0)));
        assert!(d.edge_attrs.values().any(|a| a.sharp));
    }

    #[test]
    fn large_ratio_extremes() {
        let s = primitives::quad_sphere(1.0, 8);
        // pedir quase nada: para antes de degenerar (>= 4 triângulos, fechada)
        let d = decimate_collapse(&s, 0.001, &DecimateParams::default()).unwrap();
        let t = Topology::build(&d);
        assert!(t.is_closed_manifold());
        assert!(d.face_count() >= 4);
        assert_eq!(t.euler_characteristic(&d), 2);
        d.validate().unwrap();
    }

    #[test]
    fn character_head_like_mesh_keeps_volume() {
        // "cabeça": esfera achatada + nariz (displace local) com 6k triângulos
        let mut head = primitives::uv_sphere(1.0, 48, 64);
        head.transform_positions(|p| {
            let nose = (-(p - Vec3::new(0.0, 0.0, 1.0)).length_squared() * 8.0).exp() * 0.3;
            Vec3::new(p.x * 0.9, p.y * 1.1, p.z + nose)
        });
        let d = decimate_collapse(&head, 0.25, &DecimateParams::default()).unwrap();
        assert!(Topology::build(&d).is_closed_manifold());
        let ratio = volume(&d) / volume(&head);
        assert!((0.95..=1.001).contains(&ratio), "{ratio}");
        // o nariz (maior z) foi preservado
        let zmax = d.verts.iter().map(|v| v.co.z).fold(f32::MIN, f32::max);
        let zmax0 = head.verts.iter().map(|v| v.co.z).fold(f32::MIN, f32::max);
        assert!(zmax > zmax0 - 0.05, "{zmax} vs {zmax0}");
    }

    // ------------------------------------------------------------- planar

    #[test]
    fn planar_dissolves_subdivided_cube_back_to_six_quads() {
        let c = primitives::cube_subdivided(2.0, 4);
        let m = decimate_planar(&c, 1.0f32.to_radians()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 6);
        assert_eq!(m.vert_count(), 8);
        assert!(m.faces.iter().all(|f| f.len() == 4));
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_outward(&m);
        assert!((volume(&m) - 8.0).abs() < 1e-5);
        // UV por corner veio das faces originais
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
    }

    #[test]
    fn planar_grid_becomes_one_quad_and_curved_surfaces_stay() {
        let g = primitives::grid(10, 10, 4.0, 4.0);
        let m = decimate_planar(&g, 0.01).unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.vert_count(), 4);
        assert!((m.face_area(0) - 16.0).abs() < 1e-4);
        // esfera com ângulo pequeno: nada é dissolvido
        let s = primitives::quad_sphere(1.0, 8);
        let d = decimate_planar(&s, 1.0f32.to_radians()).unwrap();
        assert_eq!(d.face_count(), s.face_count());
        // ângulo 0 em malha plana ainda dissolve (diedral exatamente 0)
        assert_eq!(decimate_planar(&g, 0.0).unwrap().face_count(), 1);
        // ângulo grande dissolve mais regiões da esfera
        let wide = decimate_planar(&s, 40.0f32.to_radians()).unwrap();
        assert!(wide.face_count() < s.face_count());
        let t = Topology::build(&wide);
        assert!(t.is_closed_manifold() || t.non_manifold_edge_count() == 0);
        wide.validate().unwrap();
    }

    #[test]
    fn planar_respects_materials_seams_sharp_and_holes() {
        let mut g = primitives::grid(4, 4, 4.0, 4.0);
        for f in 0..g.face_count() as u32 {
            if g.face_center(f).x > 0.0 {
                g.faces[f as usize].material = 1;
            }
        }
        let m = decimate_planar(&g, 0.01).unwrap();
        assert_eq!(m.face_count(), 2, "uma região por material");
        // costura impede a fusão
        let mut s = primitives::grid(4, 4, 4.0, 4.0);
        let topo = Topology::build(&s);
        for e in &topo.edges {
            if s.co(e.verts.0).x.abs() < 1e-6 && s.co(e.verts.1).x.abs() < 1e-6 {
                s.set_seam(e.verts.0, e.verts.1, true);
            }
        }
        let ms = decimate_planar(&s, 0.01).unwrap();
        assert_eq!(ms.face_count(), 2);
        assert!(ms.edge_attrs.values().filter(|a| a.seam).count() >= 1);
        // anel plano (buraco no meio): o contorno não é um laço simples => nada muda
        let mut ring = primitives::grid(3, 3, 3.0, 3.0);
        let center = (0..ring.face_count() as u32).find(|&f| ring.face_center(f).length() < 1e-4).unwrap();
        ring.delete_faces(&[center]).unwrap();
        let mr = decimate_planar(&ring, 0.01).unwrap();
        mr.validate().unwrap();
        assert_eq!(mr.face_count(), ring.face_count());
        assert_eq!(mr.vert_count(), ring.vert_count());
    }

    #[test]
    fn planar_keeps_marked_edges_and_wire_attrs() {
        let mut c = primitives::cube_subdivided(2.0, 2);
        // aresta nítida no meio de uma lateral impede a remoção do vértice colinear
        let topo = Topology::build(&c);
        let mut marked = None;
        for e in &topo.edges {
            let (a, b) = (c.co(e.verts.0), c.co(e.verts.1));
            if a.y > 0.99 && b.y > 0.99 && (a.x.abs() < 1e-6 || b.x.abs() < 1e-6) {
                marked = Some(e.verts);
                break;
            }
        }
        let (a, b) = marked.unwrap();
        c.set_sharp(a, b, true);
        let m = decimate_planar(&c, 1.0f32.to_radians()).unwrap();
        m.validate().unwrap();
        assert!(m.edge_attrs.values().any(|a| a.sharp));
        // aresta solta com atributo sobrevive
        let mut w = primitives::cube(1.0);
        let x = w.add_vert(Vec3::splat(4.0));
        let y = w.add_vert(Vec3::splat(5.0));
        w.set_seam(x, y, true);
        let mw = decimate_planar(&w, 0.1).unwrap();
        assert!(mw.edge_attrs.values().any(|a| a.seam));
    }

    #[test]
    fn planar_invalid_parameters() {
        let c = primitives::cube(1.0);
        for bad in [-0.1, 3.5, f32::NAN, f32::INFINITY] {
            assert!(matches!(decimate_planar(&c, bad), Err(ModelError::InvalidParameter(_))));
        }
        let mut broken = c.clone();
        broken.faces[0].corners[0].vert = 50;
        assert!(decimate_planar(&broken, 0.1).is_err());
        let one = EditMesh::new("e");
        assert_eq!(decimate_planar(&one, 0.1).unwrap(), one);
    }

    #[test]
    fn planar_large_mesh_and_determinism() {
        let g = primitives::grid(150, 150, 4.0, 4.0);
        let t = std::time::Instant::now();
        let a = decimate_planar(&g, 0.01).unwrap();
        let b = decimate_planar(&g, 0.01).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.face_count(), 1);
        assert!(t.elapsed().as_secs_f32() < 30.0);
    }

    #[test]
    fn settings_dispatch_and_serde() {
        let c = primitives::cube_subdivided(2.0, 3);
        let planar = decimate(&c, &DecimateSettings::Planar { angle: 0.01 }).unwrap();
        assert_eq!(planar.face_count(), 6);
        let coll = decimate(&c, &DecimateSettings::default()).unwrap();
        assert!(coll.face_count() <= c.triangle_count());
        let s = DecimateSettings::Collapse {
            ratio: 0.25,
            params: DecimateParams {
                vertex_group_weights: Some(vec![0.0, 1.0]),
                ..DecimateParams::default()
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<DecimateSettings>(&json).unwrap(), s);
        let p = DecimateSettings::Planar { angle: 0.5 };
        assert_eq!(serde_json::from_str::<DecimateSettings>(&serde_json::to_string(&p).unwrap()).unwrap(), p);
    }
}
