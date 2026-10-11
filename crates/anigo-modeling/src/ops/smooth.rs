//! Ferramentas de deformação de vértices do Edit Mode: Smooth, Relax (preserva
//! volume), Shrink/Fatten, To Sphere, Flatten (Make Planar) e Randomize.
//!
//! Todas só movem posições (topologia e demais atributos intactos), validam os
//! parâmetros antes de mutar e devolvem como seleção os vértices informados.
//! Algoritmos de suavização são Jacobi (cada iteração lê as posições da iteração
//! anterior), logo independentes da ordem dos vértices.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::normals::compute_vertex_normals;
use crate::ops::delete::{unique_faces, unique_verts};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{DMat3, DVec3, Vec3};

// ----------------------------------------------------------------------------- PRNG

/// Gerador xorshift64* determinístico (sem dependências). Mesma semente → mesma
/// sequência em qualquer plataforma.
#[derive(Debug, Clone)]
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    /// Cria o gerador; a semente `0` é válida (é misturada com uma constante).
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        };
        if s.state == 0 {
            s.state = 0x2545_F491_4F6C_DD1D;
        }
        // aquece para descorrelacionar sementes vizinhas
        for _ in 0..4 {
            s.next_u64();
        }
        s
    }

    /// Gerador independente para o elemento `index` (ordem de iteração irrelevante).
    pub fn for_index(seed: u64, index: u64) -> Self {
        Self::new(seed.wrapping_add(index.wrapping_mul(0xD1B5_4A32_D192_ED03)))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniforme em `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }

    /// Uniforme em `[-1, 1)`.
    pub fn next_signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

// --------------------------------------------------------------------------- helpers

fn selection_of(verts: &[u32]) -> Selection {
    Selection::from_verts(verts.iter().copied())
}

fn check_unit(name: &str, v: f32) -> ModelResult<()> {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(ModelError::InvalidParameter(format!(
            "{name} deve estar em [0, 1], recebeu {v}"
        )));
    }
    Ok(())
}

fn check_iterations(it: u32) -> ModelResult<()> {
    if it == 0 || it > 1000 {
        return Err(ModelError::InvalidParameter(format!(
            "iterations deve estar em 1..=1000, recebeu {it}"
        )));
    }
    Ok(())
}

/// Vizinhança de suavização: para cada vértice selecionado, as arestas que o
/// influenciam (todas; só as de fronteira se ele for de fronteira). Vértices
/// travados ou sem vizinhos ficam fora.
struct Stencil {
    /// (vértice, [(vizinho, índice de aresta)])
    rows: Vec<(u32, Vec<(u32, u32)>)>,
}

fn build_stencil(topo: &Topology, sel: &[u32], lock_boundary: bool) -> Stencil {
    let mut rows = Vec::with_capacity(sel.len());
    for &v in sel {
        let boundary = topo.is_boundary_vert(v);
        if boundary && lock_boundary {
            continue;
        }
        let mut nb: Vec<(u32, u32)> = Vec::new();
        for &e in &topo.vert_edges[v as usize] {
            let info = &topo.edges[e as usize];
            if boundary && info.faces.len() == 2 {
                continue; // fronteira só desliza ao longo da fronteira
            }
            let other = if info.verts.0 == v {
                info.verts.1
            } else {
                info.verts.0
            };
            nb.push((other, e));
        }
        if !nb.is_empty() {
            nb.sort_unstable();
            rows.push((v, nb));
        }
    }
    Stencil { rows }
}

fn axis_mask(axes: [bool; 3]) -> Vec3 {
    Vec3::new(
        axes[0] as u8 as f32,
        axes[1] as u8 as f32,
        axes[2] as u8 as f32,
    )
}

// ------------------------------------------------------------------- smooth_verts

/// Pesos do Laplaciano.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmoothWeights {
    /// Média simples dos vizinhos (rápido; distorce em malhas irregulares).
    #[default]
    Uniform,
    /// Pesos cotangente (½(cot α + cot β)), triângulos obtusos com peso 0: segue a
    /// geometria em vez da topologia. n-gons são triangulados (ear clipping) só para
    /// o cálculo; diagonais não contam como vizinhos.
    Cotangent,
}

/// Pesos cotangente por aresta da topologia (negativos viram 0).
fn cotangent_edge_weights(mesh: &EditMesh, topo: &Topology) -> Vec<f32> {
    let mut w = vec![0.0f32; topo.edges.len()];
    for face in &mesh.faces {
        let pts: Vec<Vec3> = face
            .corners
            .iter()
            .map(|c| mesh.verts[c.vert as usize].co)
            .collect();
        let tris = if pts.len() == 3 {
            vec![[0, 1, 2]]
        } else {
            crate::convert::triangulate_polygon(&pts)
        };
        for t in tris {
            for k in 0..3 {
                let (i, j, l) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                // ângulo em `i`, aresta oposta (j, l)
                let u = pts[j] - pts[i];
                let v = pts[l] - pts[i];
                let cross = u.cross(v).length();
                if cross < 1e-12 {
                    continue;
                }
                let cot = u.dot(v) / cross;
                let (a, b) = (face.corners[j].vert, face.corners[l].vert);
                if let Some(&e) = topo.edge_index.get(&crate::mesh::edge_key(a, b)) {
                    w[e as usize] += 0.5 * cot;
                }
            }
        }
    }
    for x in &mut w {
        if *x < 0.0 {
            *x = 0.0;
        }
    }
    w
}

fn laplacian_step(
    mesh: &mut EditMesh,
    stencil: &Stencil,
    factor: f32,
    mask: Vec3,
    edge_w: Option<&[f32]>,
) {
    let mut updates: Vec<(u32, Vec3)> = Vec::with_capacity(stencil.rows.len());
    for (v, nb) in &stencil.rows {
        let p = mesh.verts[*v as usize].co;
        let (mut acc, mut wsum) = (Vec3::ZERO, 0.0f32);
        if let Some(ew) = edge_w {
            for &(o, e) in nb {
                let w = ew[e as usize];
                acc += mesh.verts[o as usize].co * w;
                wsum += w;
            }
        }
        if wsum <= 1e-12 {
            acc = Vec3::ZERO;
            wsum = 0.0;
            for &(o, _) in nb {
                acc += mesh.verts[o as usize].co;
                wsum += 1.0;
            }
        }
        let avg = acc / wsum;
        updates.push((*v, p + (avg - p) * factor * mask));
    }
    for (v, p) in updates {
        mesh.verts[v as usize].co = p;
    }
}

/// *Smooth Vertices* (Laplaciano): move cada vértice em direção à média ponderada
/// dos vizinhos, `iterations` vezes.
///
/// * `factor` em `[0, 1]`: fração do caminho até a média por iteração;
/// * `lock_boundary`: vértices de fronteira (arestas com ≠ 2 faces) não se movem;
///   sem isso eles deslizam apenas ao longo da fronteira (não encolhem para dentro);
/// * `axes`: eixos X/Y/Z afetados (ao menos um);
/// * `weights`: [`SmoothWeights::Uniform`] ou [`SmoothWeights::Cotangent`] (recalculado
///   a cada iteração).
///
/// Só os vértices de `verts` se movem; os demais servem de vizinhos. Seleção
/// devolvida: `verts`. Erros (malha intacta): seleção vazia/índice inválido, fator
/// fora de `[0, 1]`, `iterations` fora de `1..=1000`, nenhum eixo.
pub fn smooth_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    factor: f32,
    iterations: u32,
    lock_boundary: bool,
    axes: [bool; 3],
    weights: SmoothWeights,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    check_unit("factor", factor)?;
    check_iterations(iterations)?;
    if !axes.iter().any(|&a| a) {
        return Err(ModelError::InvalidParameter(
            "nenhum eixo habilitado".into(),
        ));
    }
    let topo = Topology::build(mesh);
    let stencil = build_stencil(&topo, &sel, lock_boundary);
    let mask = axis_mask(axes);
    for _ in 0..iterations {
        let ew = (weights == SmoothWeights::Cotangent).then(|| cotangent_edge_weights(mesh, &topo));
        laplacian_step(mesh, &stencil, factor, mask, ew.as_deref());
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

// ------------------------------------------------------------------- relax_verts

/// Algoritmo de suavização que preserva volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RelaxMethod {
    /// Taubin λ|μ: um passo de suavização (λ = `factor`) seguido de um passo de
    /// "inflar" (μ < 0) calculado para a banda de passagem `pass_band` (típico 0.1,
    /// `(0, 1)`). Quase não encolhe.
    Taubin { pass_band: f32 },
    /// HC (Vollmer et al.): suaviza e depois corrige empurrando de volta pela
    /// diferença; `alpha` puxa para a posição original, `beta` controla a correção
    /// (típico 0 e 0.5; ambos em `[0, 1]`).
    Hc { alpha: f32, beta: f32 },
}

impl Default for RelaxMethod {
    fn default() -> Self {
        RelaxMethod::Taubin { pass_band: 0.1 }
    }
}

/// *Relax*: suavização que preserva volume e detalhes de baixa frequência (HC ou
/// Taubin); adequada para limpar topologia/ruído sem "murchar" a forma como o
/// [`smooth_verts`] faz. Usa pesos uniformes.
///
/// `factor` em `(0, 1]` (λ), `iterations` em `1..=1000` (cada uma é um par
/// suaviza+corrige), `lock_boundary` como em `smooth_verts`. Seleção devolvida:
/// `verts`. Erros: parâmetros fora de faixa, seleção vazia/inválida.
pub fn relax_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    factor: f32,
    iterations: u32,
    method: RelaxMethod,
    lock_boundary: bool,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(ModelError::InvalidParameter(format!(
            "factor deve estar em (0, 1], recebeu {factor}"
        )));
    }
    check_iterations(iterations)?;
    match method {
        RelaxMethod::Taubin { pass_band } => {
            if !pass_band.is_finite() || pass_band <= 0.0 || pass_band >= 1.0 {
                return Err(ModelError::InvalidParameter(format!(
                    "pass_band deve estar em (0, 1), recebeu {pass_band}"
                )));
            }
        }
        RelaxMethod::Hc { alpha, beta } => {
            check_unit("alpha", alpha)?;
            check_unit("beta", beta)?;
        }
    }
    let topo = Topology::build(mesh);
    let stencil = build_stencil(&topo, &sel, lock_boundary);
    match method {
        RelaxMethod::Taubin { pass_band } => {
            let mu = 1.0 / (pass_band - 1.0 / factor);
            for _ in 0..iterations {
                laplacian_step(mesh, &stencil, factor, Vec3::ONE, None);
                laplacian_step(mesh, &stencil, mu, Vec3::ONE, None);
            }
        }
        RelaxMethod::Hc { alpha, beta } => {
            // Vizinhança completa para o termo de correção `b` (todos os vértices com vizinhos).
            let nv = mesh.verts.len();
            let mut adj: Vec<Vec<u32>> = vec![Vec::new(); nv];
            for e in &topo.edges {
                adj[e.verts.0 as usize].push(e.verts.1);
                adj[e.verts.1 as usize].push(e.verts.0);
            }
            let original: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
            for _ in 0..iterations {
                let p: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
                let q: Vec<Vec3> = (0..nv)
                    .map(|i| {
                        if adj[i].is_empty() {
                            p[i]
                        } else {
                            adj[i].iter().map(|&j| p[j as usize]).sum::<Vec3>()
                                / adj[i].len() as f32
                        }
                    })
                    .collect();
                let b: Vec<Vec3> = (0..nv)
                    .map(|i| q[i] - (original[i] * alpha + p[i] * (1.0 - alpha)))
                    .collect();
                let mut updates: Vec<(u32, Vec3)> = Vec::with_capacity(stencil.rows.len());
                for (v, nb) in &stencil.rows {
                    let i = *v as usize;
                    let avg_b =
                        nb.iter().map(|&(o, _)| b[o as usize]).sum::<Vec3>() / nb.len() as f32;
                    let target = q[i] - (b[i] * beta + avg_b * (1.0 - beta));
                    updates.push((*v, p[i] + (target - p[i]) * factor));
                }
                for (v, pos) in updates {
                    mesh.verts[v as usize].co = pos;
                }
            }
        }
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

// ----------------------------------------------------------------- shrink / fatten

fn vertices_with_faces(topo: &Topology, verts: &[u32]) -> Vec<u32> {
    verts
        .iter()
        .copied()
        .filter(|&v| !topo.vert_faces[v as usize].is_empty())
        .collect()
}

/// *Shrink/Fatten* (`Alt+S`): desloca os vértices `distance` ao longo da normal do
/// vértice (média por ângulo das normais das faces, sem split). `distance > 0`
/// engorda, `< 0` encolhe. Vértices sem face são ignorados. Seleção devolvida:
/// `verts`. Erros: seleção vazia/inválida, `distance` não finita.
pub fn shrink_fatten(mesh: &mut EditMesh, verts: &[u32], distance: f32) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if !distance.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "distance não finita: {distance}"
        )));
    }
    let topo = Topology::build(mesh);
    let normals = compute_vertex_normals(mesh);
    for v in vertices_with_faces(&topo, &sel) {
        mesh.verts[v as usize].co += normals[v as usize] * distance;
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

/// [`shrink_fatten`] sobre os vértices das faces dadas (seleção em modo face).
/// Seleção devolvida: as faces.
pub fn shrink_fatten_faces(
    mesh: &mut EditMesh,
    faces: &[u32],
    distance: f32,
) -> ModelResult<OpOutput> {
    let faces = unique_faces(mesh, faces, "faces")?;
    let mut verts: Vec<u32> = faces
        .iter()
        .flat_map(|&f| mesh.faces[f as usize].verts().collect::<Vec<_>>())
        .collect();
    verts.sort_unstable();
    verts.dedup();
    shrink_fatten(mesh, &verts, distance)?;
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, faces.iter().copied()),
        ..Default::default()
    })
}

// ----------------------------------------------------------------------- to_sphere

/// *To Sphere* (`Shift+Alt+S`): aproxima os vértices de uma esfera. O raio é a
/// distância média ao centro (`center`, ou a média das posições se `None`); cada
/// vértice vai de `factor` (`[0, 1]`) do caminho até a esfera. Vértices exatamente
/// no centro não têm direção e ficam parados. Seleção devolvida: `verts`.
///
/// Erros: seleção vazia/inválida, `factor` fora de `[0, 1]`, centro não finito,
/// raio médio nulo (todos os vértices no centro).
pub fn to_sphere(
    mesh: &mut EditMesh,
    verts: &[u32],
    factor: f32,
    center: Option<Vec3>,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    check_unit("factor", factor)?;
    if let Some(c) = center {
        if !c.is_finite() {
            return Err(ModelError::InvalidParameter("centro não finito".into()));
        }
    }
    let c =
        center.unwrap_or_else(|| sel.iter().map(|&v| mesh.co(v)).sum::<Vec3>() / sel.len() as f32);
    let radius = sel.iter().map(|&v| (mesh.co(v) - c).length()).sum::<f32>() / sel.len() as f32;
    if radius < 1e-12 {
        return Err(ModelError::InvalidParameter(
            "raio médio nulo: todos os vértices coincidem com o centro".into(),
        ));
    }
    for &v in &sel {
        let p = mesh.co(v);
        let d = p - c;
        if let Some(dir) = d.try_normalize() {
            let target = c + dir * radius;
            mesh.verts[v as usize].co = p.lerp(target, factor);
        }
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

// ------------------------------------------------------------------------ flatten

/// Plano orientado: ponto + normal unitária.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub point: Vec3,
    pub normal: Vec3,
}

/// Autovetores de uma matriz simétrica 3×3 pelo método de Jacobi. Devolve
/// `(autovalores, autovetores como colunas)` sem ordenação.
#[allow(clippy::needless_range_loop)]
fn jacobi_eigen(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        // maior elemento fora da diagonal
        let (mut p, mut q, mut max) = (0, 1, a[0][1].abs());
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > max {
                max = a[i][j].abs();
                p = i;
                q = j;
            }
        }
        if max < 1e-15 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let t = if theta == 0.0 { 1.0 } else { t };
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        for k in 0..3 {
            let (akp, akq) = (a[k][p], a[k][q]);
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let (apk, aqk) = (a[p][k], a[q][k]);
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        for row in v.iter_mut() {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// Plano de melhor ajuste (mínimos quadrados ortogonais, PCA): passa pelo centróide
/// com a normal na direção de menor variância. `None` com menos de 3 pontos ou
/// coordenadas não finitas. Para pontos colineares a normal é uma das infinitas
/// perpendiculares (determinística, mas arbitrária).
pub fn best_fit_plane(points: &[Vec3]) -> Option<Plane> {
    if points.len() < 3 || points.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let n = points.len() as f64;
    let c = points.iter().map(|p| p.as_dvec3()).sum::<DVec3>() / n;
    let mut cov = [[0.0f64; 3]; 3];
    for p in points {
        let d = p.as_dvec3() - c;
        let d = [d.x, d.y, d.z];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let (vals, vecs) = jacobi_eigen(cov);
    let mut k = 0;
    for i in 1..3 {
        if vals[i] < vals[k] {
            k = i;
        }
    }
    let m = DMat3::from_cols(
        DVec3::new(vecs[0][0], vecs[1][0], vecs[2][0]),
        DVec3::new(vecs[0][1], vecs[1][1], vecs[2][1]),
        DVec3::new(vecs[0][2], vecs[1][2], vecs[2][2]),
    );
    let normal = m.col(k).normalize_or_zero();
    if normal == DVec3::ZERO {
        return None;
    }
    // sinal determinístico: componente dominante positiva
    let dominant = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
        normal.x
    } else if normal.y.abs() >= normal.z.abs() {
        normal.y
    } else {
        normal.z
    };
    let normal = if dominant < 0.0 { -normal } else { normal };
    Some(Plane {
        point: c.as_vec3(),
        normal: normal.as_vec3(),
    })
}

/// *Flatten / Make Planar* (`To Plane`): projeta os vértices em um plano.
///
/// * `plane`: plano explícito (a normal é normalizada) ou `None` para o plano de
///   melhor ajuste dos próprios vértices ([`best_fit_plane`]; exige ≥ 3 vértices);
/// * `factor` em `[0, 1]`: 1 = completamente no plano, 0.5 = metade do caminho.
///
/// Seleção devolvida: `verts`. Erros: seleção vazia/inválida, `factor` fora de faixa,
/// plano com normal nula/não finita, poucos vértices para o ajuste.
pub fn flatten(
    mesh: &mut EditMesh,
    verts: &[u32],
    plane: Option<Plane>,
    factor: f32,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    check_unit("factor", factor)?;
    let plane = match plane {
        Some(p) => {
            let n = p.normal.try_normalize().ok_or_else(|| {
                ModelError::InvalidParameter("normal do plano nula ou não finita".into())
            })?;
            if !p.point.is_finite() {
                return Err(ModelError::InvalidParameter(
                    "ponto do plano não finito".into(),
                ));
            }
            Plane {
                point: p.point,
                normal: n,
            }
        }
        None => {
            let pts: Vec<Vec3> = sel.iter().map(|&v| mesh.co(v)).collect();
            best_fit_plane(&pts).ok_or_else(|| {
                ModelError::InvalidParameter(
                    "flatten sem plano explícito exige pelo menos 3 vértices válidos".into(),
                )
            })?
        }
    };
    for &v in &sel {
        let p = mesh.co(v);
        let dist = (p - plane.point).dot(plane.normal);
        mesh.verts[v as usize].co = p - plane.normal * dist * factor;
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

// ----------------------------------------------------------------------- randomize

/// *Randomize Vertices*: desloca cada vértice por um vetor uniforme em
/// `[-amount, amount]³`, determinístico por `(seed, índice do vértice)` — o
/// resultado não depende da ordem em que os vértices são passados. `amount = 0` não
/// altera nada. Seleção devolvida: `verts`. Erros: seleção vazia/inválida, `amount`
/// negativo ou não finito.
pub fn randomize(
    mesh: &mut EditMesh,
    verts: &[u32],
    amount: f32,
    seed: u64,
) -> ModelResult<OpOutput> {
    let sel = unique_verts(mesh, verts, "vértices")?;
    if !amount.is_finite() || amount < 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "amount deve ser finito e >= 0, recebeu {amount}"
        )));
    }
    for &v in &sel {
        let mut rng = XorShift64::for_index(seed, v as u64);
        let d = Vec3::new(rng.next_signed(), rng.next_signed(), rng.next_signed()) * amount;
        mesh.verts[v as usize].co += d;
    }
    Ok(OpOutput {
        selection: selection_of(&sel),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    const ALL: [bool; 3] = [true; 3];

    fn all_verts(m: &EditMesh) -> Vec<u32> {
        (0..m.vert_count() as u32).collect()
    }

    fn volume(m: &EditMesh) -> f64 {
        let mut v = 0.0;
        for f in 0..m.face_count() as u32 {
            let p: Vec<DVec3> = m.face_positions(f).iter().map(|q| q.as_dvec3()).collect();
            for i in 1..p.len() - 1 {
                v += p[0].dot(p[i].cross(p[i + 1])) / 6.0;
            }
        }
        v
    }

    fn area(m: &EditMesh) -> f32 {
        (0..m.face_count() as u32).map(|f| m.face_area(f)).sum()
    }

    #[test]
    fn xorshift_is_deterministic_and_uniform() {
        let mut a = XorShift64::new(42);
        let mut b = XorShift64::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = XorShift64::new(0);
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let x = r.next_f32();
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02);
        assert_ne!(XorShift64::new(1).next_u64(), XorShift64::new(2).next_u64());
        assert_ne!(
            XorShift64::for_index(1, 0).next_u64(),
            XorShift64::for_index(1, 1).next_u64()
        );
    }

    #[test]
    fn smooth_reduces_a_bump_and_locks_boundary() {
        let mut m = primitives::grid(6, 6, 6.0, 6.0);
        let center = (0..m.vert_count() as u32)
            .find(|&v| m.co(v).length() < 1e-5)
            .unwrap();
        m.verts[center as usize].co.y = 1.0;
        let verts = all_verts(&m);
        let boundary_before: Vec<Vec3> = {
            let t = Topology::build(&m);
            (0..m.vert_count() as u32)
                .filter(|&v| t.is_boundary_vert(v))
                .map(|v| m.co(v))
                .collect()
        };
        smooth_verts(&mut m, &verts, 0.5, 5, true, ALL, SmoothWeights::Uniform).unwrap();
        assert!(m.co(center).y < 0.5);
        assert!(m.co(center).y > 0.0);
        let t = Topology::build(&m);
        let boundary_after: Vec<Vec3> = (0..m.vert_count() as u32)
            .filter(|&v| t.is_boundary_vert(v))
            .map(|v| m.co(v))
            .collect();
        assert_eq!(boundary_before, boundary_after);
        m.validate().unwrap();
    }

    #[test]
    fn smooth_axes_and_unlocked_boundary() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let center = (0..m.vert_count() as u32)
            .find(|&v| m.co(v).length() < 1e-5)
            .unwrap();
        m.verts[center as usize].co += Vec3::new(0.3, 1.0, 0.3);
        let verts = all_verts(&m);
        let mut only_y = m.clone();
        smooth_verts(
            &mut only_y,
            &verts,
            1.0,
            1,
            true,
            [false, true, false],
            SmoothWeights::Uniform,
        )
        .unwrap();
        assert_eq!(only_y.co(center).x, m.co(center).x);
        assert_eq!(only_y.co(center).z, m.co(center).z);
        assert!(only_y.co(center).y < m.co(center).y);
        // sem travar fronteira: cantos andam mas a malha continua plana dentro do bounding
        let mut free = primitives::grid(4, 4, 4.0, 4.0);
        let verts = all_verts(&free);
        let before = free.bounds().unwrap();
        smooth_verts(
            &mut free,
            &verts,
            0.5,
            3,
            false,
            ALL,
            SmoothWeights::Uniform,
        )
        .unwrap();
        let after = free.bounds().unwrap();
        // a fronteira desliza só ao longo da fronteira: bounding não encolhe nos lados retos,
        // mas os cantos (vizinhos só ao longo da borda) podem recuar
        assert!(after.1.x <= before.1.x + 1e-5 && after.1.x > before.1.x * 0.5);
        for v in &free.verts {
            assert!(v.co.y.abs() < 1e-6);
        }
    }

    #[test]
    fn smooth_flat_regular_grid_is_a_fixed_point_for_interior() {
        for w in [SmoothWeights::Uniform, SmoothWeights::Cotangent] {
            let mut m = primitives::grid(6, 6, 6.0, 6.0);
            let h = m.content_hash();
            let t = Topology::build(&m);
            let interior: Vec<u32> = (0..m.vert_count() as u32)
                .filter(|&v| !t.is_boundary_vert(v))
                .collect();
            smooth_verts(&mut m, &interior, 1.0, 3, true, ALL, w).unwrap();
            for (a, b) in m
                .verts
                .iter()
                .zip(primitives::grid(6, 6, 6.0, 6.0).verts.iter())
            {
                assert!((a.co - b.co).length() < 1e-4, "{w:?}");
            }
            let _ = h;
        }
    }

    #[test]
    fn cotangent_and_uniform_differ_on_irregular_mesh_and_cotangent_keeps_shape_better() {
        // grid com uma coluna comprimida: a média uniforme arrasta pontos; cotangente respeita a geometria
        let mut base = primitives::grid(8, 8, 8.0, 8.0);
        for v in &mut base.verts {
            if v.co.x > 0.0 {
                v.co.x = v.co.x * 0.25 + 0.0;
            }
        }
        let t = Topology::build(&base);
        let interior: Vec<u32> = (0..base.vert_count() as u32)
            .filter(|&v| !t.is_boundary_vert(v))
            .collect();
        let mut u = base.clone();
        let mut c = base.clone();
        smooth_verts(&mut u, &interior, 0.5, 4, true, ALL, SmoothWeights::Uniform).unwrap();
        smooth_verts(
            &mut c,
            &interior,
            0.5,
            4,
            true,
            ALL,
            SmoothWeights::Cotangent,
        )
        .unwrap();
        assert_ne!(u.content_hash(), c.content_hash());
        // ambos mantêm a malha plana
        assert!(u
            .verts
            .iter()
            .chain(c.verts.iter())
            .all(|v| v.co.y.abs() < 1e-5));
        // a cotangente muda menos os pontos de uma malha já "bem espaçada" geometricamente
        let drift = |m: &EditMesh| -> f32 {
            m.verts
                .iter()
                .zip(base.verts.iter())
                .map(|(a, b)| (a.co - b.co).length())
                .sum()
        };
        assert!(
            drift(&c) <= drift(&u) * 1.01,
            "cot {} uni {}",
            drift(&c),
            drift(&u)
        );
    }

    #[test]
    fn smooth_shrinks_sphere_and_is_deterministic() {
        let orig = primitives::uv_sphere(1.0, 12, 16);
        let run = |w| {
            let mut m = orig.clone();
            let v = all_verts(&m);
            smooth_verts(&mut m, &v, 0.5, 5, false, ALL, w).unwrap();
            m
        };
        for w in [SmoothWeights::Uniform, SmoothWeights::Cotangent] {
            let a = run(w);
            assert!(volume(&a) < volume(&orig) * 0.98, "{w:?}");
            assert_eq!(a.content_hash(), run(w).content_hash());
            a.validate().unwrap();
        }
    }

    #[test]
    fn smooth_errors_leave_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let u = SmoothWeights::Uniform;
        assert!(smooth_verts(&mut m, &[], 0.5, 1, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[99], 0.5, 1, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], -0.1, 1, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], 1.5, 1, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], f32::NAN, 1, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], 0.5, 0, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], 0.5, 5000, true, ALL, u).is_err());
        assert!(smooth_verts(&mut m, &[0], 0.5, 1, true, [false; 3], u).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn smooth_isolated_and_loose_vertices_are_ignored() {
        let mut m = primitives::cube(1.0);
        let loose = m.add_vert(Vec3::splat(9.0));
        smooth_verts(
            &mut m,
            &[loose],
            1.0,
            3,
            false,
            ALL,
            SmoothWeights::Cotangent,
        )
        .unwrap();
        assert_eq!(m.co(loose), Vec3::splat(9.0));
    }

    #[test]
    fn relax_preserves_volume_better_than_smooth() {
        let orig = primitives::uv_sphere(1.0, 14, 20);
        let v0 = volume(&orig);
        let verts = all_verts(&orig);
        let mut smooth = orig.clone();
        smooth_verts(
            &mut smooth,
            &verts,
            0.5,
            20,
            false,
            ALL,
            SmoothWeights::Uniform,
        )
        .unwrap();
        let loss_smooth = (v0 - volume(&smooth)) / v0;
        let mut taubin = orig.clone();
        relax_verts(
            &mut taubin,
            &verts,
            0.5,
            20,
            RelaxMethod::Taubin { pass_band: 0.1 },
            false,
        )
        .unwrap();
        let loss_taubin = (v0 - volume(&taubin)) / v0;
        let mut hc = orig.clone();
        relax_verts(
            &mut hc,
            &verts,
            0.5,
            20,
            RelaxMethod::Hc {
                alpha: 0.0,
                beta: 0.5,
            },
            false,
        )
        .unwrap();
        let loss_hc = (v0 - volume(&hc)) / v0;
        assert!(loss_smooth > 0.1, "{loss_smooth}");
        assert!(
            loss_taubin < loss_smooth * 0.2,
            "taubin {loss_taubin} vs smooth {loss_smooth}"
        );
        assert!(
            loss_hc < loss_smooth * 0.6,
            "hc {loss_hc} vs smooth {loss_smooth}"
        );
        taubin.validate().unwrap();
        hc.validate().unwrap();
    }

    #[test]
    fn relax_removes_noise_and_keeps_boundary_when_locked() {
        let clean = primitives::grid(10, 10, 10.0, 10.0);
        let mut noisy = clean.clone();
        let t = Topology::build(&noisy);
        let interior: Vec<u32> = (0..noisy.vert_count() as u32)
            .filter(|&v| !t.is_boundary_vert(v))
            .collect();
        randomize(&mut noisy, &interior, 0.2, 5).unwrap();
        let err = |m: &EditMesh| -> f32 { m.verts.iter().map(|v| v.co.y.abs()).sum() };
        let before = err(&noisy);
        let verts = all_verts(&noisy);
        for method in [
            RelaxMethod::default(),
            RelaxMethod::Hc {
                alpha: 0.0,
                beta: 0.5,
            },
        ] {
            let mut r = noisy.clone();
            relax_verts(&mut r, &verts, 0.7, 6, method, true).unwrap();
            assert!(err(&r) < before * 0.5, "{method:?}");
            for v in 0..r.vert_count() as u32 {
                if Topology::build(&r).is_boundary_vert(v) {
                    assert_eq!(r.co(v), noisy.co(v));
                }
            }
        }
    }

    #[test]
    fn relax_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        let d = RelaxMethod::default();
        assert!(relax_verts(&mut m, &[], 0.5, 1, d, true).is_err());
        assert!(relax_verts(&mut m, &[0], 0.0, 1, d, true).is_err());
        assert!(relax_verts(&mut m, &[0], 1.1, 1, d, true).is_err());
        assert!(relax_verts(&mut m, &[0], 0.5, 0, d, true).is_err());
        assert!(relax_verts(
            &mut m,
            &[0],
            0.5,
            1,
            RelaxMethod::Taubin { pass_band: 0.0 },
            true
        )
        .is_err());
        assert!(relax_verts(
            &mut m,
            &[0],
            0.5,
            1,
            RelaxMethod::Taubin { pass_band: 1.0 },
            true
        )
        .is_err());
        assert!(relax_verts(
            &mut m,
            &[0],
            0.5,
            1,
            RelaxMethod::Hc {
                alpha: -1.0,
                beta: 0.5
            },
            true
        )
        .is_err());
        assert!(relax_verts(
            &mut m,
            &[0],
            0.5,
            1,
            RelaxMethod::Hc {
                alpha: 0.0,
                beta: 2.0
            },
            true
        )
        .is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn shrink_fatten_moves_along_normals() {
        let mut m = primitives::quad_sphere(1.0, 6);
        let v = all_verts(&m);
        shrink_fatten(&mut m, &v, 0.25).unwrap();
        for vert in &m.verts {
            assert!(
                (vert.co.length() - 1.25).abs() < 0.02,
                "{}",
                vert.co.length()
            );
        }
        shrink_fatten(&mut m, &v, -0.5).unwrap();
        for vert in &m.verts {
            assert!((vert.co.length() - 0.75).abs() < 0.03);
        }
        // faces
        let mut c = primitives::cube_subdivided(2.0, 2);
        let before = c.bounds().unwrap();
        let faces: Vec<u32> = (0..c.face_count() as u32).collect();
        shrink_fatten_faces(&mut c, &faces, 0.1).unwrap();
        let after = c.bounds().unwrap();
        assert!(after.1.x > before.1.x);
        c.validate().unwrap();
    }

    #[test]
    fn shrink_fatten_errors_and_loose() {
        let mut m = primitives::cube(1.0);
        let loose = m.add_vert(Vec3::splat(4.0));
        let h = m.content_hash();
        assert!(shrink_fatten(&mut m, &[], 1.0).is_err());
        assert!(shrink_fatten(&mut m, &[0], f32::INFINITY).is_err());
        assert!(shrink_fatten(&mut m, &[99], 1.0).is_err());
        assert!(shrink_fatten_faces(&mut m, &[], 1.0).is_err());
        assert!(shrink_fatten_faces(&mut m, &[40], 1.0).is_err());
        assert_eq!(m.content_hash(), h);
        shrink_fatten(&mut m, &[loose], 1.0).unwrap();
        assert_eq!(m.co(loose), Vec3::splat(4.0));
    }

    #[test]
    fn to_sphere_projects_onto_average_radius() {
        let mut m = primitives::cube_subdivided(2.0, 3);
        let v = all_verts(&m);
        to_sphere(&mut m, &v, 1.0, None).unwrap();
        let r0 = m.verts[0].co.length();
        assert!(m.verts.iter().all(|x| (x.co.length() - r0).abs() < 1e-4));
        // meio caminho
        let mut h = primitives::cube_subdivided(2.0, 3);
        let orig = h.clone();
        to_sphere(&mut h, &v, 0.5, None).unwrap();
        for (a, b) in h.verts.iter().zip(orig.verts.iter()) {
            let expected = b.co.lerp(b.co.normalize() * r0, 0.5);
            assert!((a.co - expected).length() < 1e-3);
        }
        // fator 0 não altera
        let mut z = orig.clone();
        to_sphere(&mut z, &v, 0.0, None).unwrap();
        assert_eq!(z.content_hash(), orig.content_hash());
        // centro explícito
        let mut c = primitives::cube(2.0);
        let center = Vec3::new(1.0, 0.0, 0.0);
        let ids = all_verts(&c);
        to_sphere(&mut c, &ids, 1.0, Some(center)).unwrap();
        let r = (c.verts[0].co - center).length();
        assert!(c
            .verts
            .iter()
            .all(|x| ((x.co - center).length() - r).abs() < 1e-4));
    }

    #[test]
    fn to_sphere_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(to_sphere(&mut m, &[], 1.0, None).is_err());
        assert!(to_sphere(&mut m, &[0], 2.0, None).is_err());
        assert!(to_sphere(&mut m, &[0, 1], 1.0, Some(Vec3::NAN)).is_err());
        // um único vértice: centro = o próprio vértice → raio 0
        assert!(to_sphere(&mut m, &[0], 1.0, None).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn best_fit_plane_recovers_tilted_plane() {
        let n = Vec3::new(1.0, 2.0, 3.0).normalize();
        let u = n.cross(Vec3::Y).normalize();
        let w = n.cross(u);
        let pts: Vec<Vec3> = (0..50)
            .map(|i| {
                let (a, b) = ((i % 7) as f32 - 3.0, (i / 7) as f32 - 3.0);
                Vec3::new(1.0, 1.0, 1.0) + u * a + w * b * 0.5 + n * 0.0
            })
            .collect();
        let p = best_fit_plane(&pts).unwrap();
        assert!(p.normal.dot(n).abs() > 0.9999, "{:?} vs {:?}", p.normal, n);
        assert!(best_fit_plane(&pts[..2]).is_none());
        assert!(best_fit_plane(&[Vec3::ZERO, Vec3::X, Vec3::NAN]).is_none());
        // colineares: devolve algum plano válido
        let line = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0];
        let pl = best_fit_plane(&line).unwrap();
        assert!(pl.normal.dot(Vec3::X).abs() < 1e-5);
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn jacobi_eigen_diagonalizes() {
        let (vals, vecs) = jacobi_eigen([[2.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 0.0, 5.0]]);
        let mut v = vals;
        v.sort_by(|a, b| a.total_cmp(b));
        assert!(
            (v[0] - 1.0).abs() < 1e-9 && (v[1] - 3.0).abs() < 1e-9 && (v[2] - 5.0).abs() < 1e-9
        );
        // vetores ortonormais
        for i in 0..3 {
            let col = DVec3::new(vecs[0][i], vecs[1][i], vecs[2][i]);
            assert!((col.length() - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn flatten_projects_to_best_fit_or_given_plane() {
        // grid inclinado com ruído
        let mut m = primitives::grid(5, 5, 5.0, 5.0);
        m.transform_positions(|p| glam::Quat::from_rotation_x(0.6) * p + Vec3::new(0.0, 1.0, 0.0));
        let ids = all_verts(&m);
        let mut noisy = m.clone();
        randomize(&mut noisy, &ids, 0.05, 11).unwrap();
        let mut full = noisy.clone();
        flatten(&mut full, &ids, None, 1.0).unwrap();
        let pts: Vec<Vec3> = full.verts.iter().map(|v| v.co).collect();
        let plane = best_fit_plane(&pts).unwrap();
        for p in &pts {
            assert!((*p - plane.point).dot(plane.normal).abs() < 1e-4);
        }
        // fator 0.5 reduz a distância à metade
        let plane0 = best_fit_plane(&noisy.verts.iter().map(|v| v.co).collect::<Vec<_>>()).unwrap();
        let mut half = noisy.clone();
        flatten(&mut half, &ids, None, 0.5).unwrap();
        for (a, b) in half.verts.iter().zip(noisy.verts.iter()) {
            let d0 = (b.co - plane0.point).dot(plane0.normal);
            let d1 = (a.co - plane0.point).dot(plane0.normal);
            assert!((d1 - d0 * 0.5).abs() < 1e-4);
        }
        // plano explícito
        let mut e = primitives::cube(1.0);
        let ids = all_verts(&e);
        flatten(
            &mut e,
            &ids,
            Some(Plane {
                point: Vec3::ZERO,
                normal: Vec3::new(0.0, 5.0, 0.0),
            }),
            1.0,
        )
        .unwrap();
        assert!(e.verts.iter().all(|v| v.co.y.abs() < 1e-6));
    }

    #[test]
    fn flatten_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(flatten(&mut m, &[], None, 1.0).is_err());
        assert!(flatten(&mut m, &[0, 1], None, 1.0).is_err());
        assert!(flatten(&mut m, &[0, 1, 2], None, 2.0).is_err());
        assert!(flatten(
            &mut m,
            &[0],
            Some(Plane {
                point: Vec3::ZERO,
                normal: Vec3::ZERO
            }),
            1.0
        )
        .is_err());
        assert!(flatten(
            &mut m,
            &[0],
            Some(Plane {
                point: Vec3::NAN,
                normal: Vec3::Y
            }),
            1.0
        )
        .is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn randomize_is_deterministic_bounded_and_order_independent() {
        let orig = primitives::quad_sphere(1.0, 4);
        let ids = all_verts(&orig);
        let mut a = orig.clone();
        randomize(&mut a, &ids, 0.1, 7).unwrap();
        let mut b = orig.clone();
        let mut rev = ids.clone();
        rev.reverse();
        randomize(&mut b, &rev, 0.1, 7).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        let mut c = orig.clone();
        randomize(&mut c, &ids, 0.1, 8).unwrap();
        assert_ne!(a.content_hash(), c.content_hash());
        let mut mean = Vec3::ZERO;
        for (x, y) in a.verts.iter().zip(orig.verts.iter()) {
            let d = x.co - y.co;
            assert!(d.abs().max_element() <= 0.1 + 1e-6);
            mean += d;
        }
        assert!((mean / a.vert_count() as f32).length() < 0.03);
        let mut z = orig.clone();
        randomize(&mut z, &ids, 0.0, 1).unwrap();
        assert_eq!(z.content_hash(), orig.content_hash());
        assert!(randomize(&mut z, &ids, -1.0, 1).is_err());
        assert!(randomize(&mut z, &ids, f32::NAN, 1).is_err());
        assert!(randomize(&mut z, &[], 1.0, 1).is_err());
        assert!(randomize(&mut z, &[1000], 1.0, 1).is_err());
    }

    #[test]
    fn only_selected_vertices_move() {
        let orig = primitives::quad_sphere(1.0, 4);
        let sel: Vec<u32> = (0..orig.vert_count() as u32).step_by(5).collect();
        for op in 0..4 {
            let mut m = orig.clone();
            match op {
                0 => drop(
                    smooth_verts(&mut m, &sel, 1.0, 2, false, ALL, SmoothWeights::Cotangent)
                        .unwrap(),
                ),
                1 => {
                    drop(relax_verts(&mut m, &sel, 1.0, 2, RelaxMethod::default(), false).unwrap())
                }
                2 => drop(shrink_fatten(&mut m, &sel, 0.1).unwrap()),
                _ => drop(randomize(&mut m, &sel, 0.1, 3).unwrap()),
            }
            for (i, (a, b)) in m.verts.iter().zip(orig.verts.iter()).enumerate() {
                if !sel.contains(&(i as u32)) {
                    assert_eq!(a.co, b.co, "op {op} moveu o vértice não selecionado {i}");
                }
            }
            assert!(area(&m) > 0.0);
        }
    }

    #[test]
    fn large_mesh_smoothing_is_fast() {
        let mut m = primitives::quad_sphere(1.0, 64);
        let v = all_verts(&m);
        let t0 = std::time::Instant::now();
        smooth_verts(&mut m, &v, 0.5, 3, false, ALL, SmoothWeights::Cotangent).unwrap();
        relax_verts(
            &mut m,
            &v,
            0.5,
            3,
            RelaxMethod::Hc {
                alpha: 0.0,
                beta: 0.5,
            },
            false,
        )
        .unwrap();
        relax_verts(&mut m, &v, 0.5, 3, RelaxMethod::default(), false).unwrap();
        assert!(t0.elapsed().as_secs() < 60, "{:?}", t0.elapsed());
        m.validate().unwrap();
    }
}
