//! Ferramentas de edição de normais (o diferencial do ANIGO para cel shading).
//!
//! No visual anime o sombreamento do rosto, do cabelo e da roupa é ditado por
//! **normais editadas** e atributos de vértice, não por normal map. Este módulo
//! reúne as operações equivalentes ao menu *Mesh > Normals* do Blender e ao
//! modificador *Normal Edit* / *Data Transfer*:
//!
//! * transferência a partir de um ponto (esfera de sombreamento), direcional,
//!   "point to target", média, rotação, reset, copiar/colar e suavização;
//! * transferência de um mesh proxy (esfera de cabeça, rosto simplificado) pela
//!   superfície/vértice/corner mais próximo;
//! * **normais de contorno** (inverted hull sem frestas) com codificação octaédrica;
//! * métrica de suavidade de normais (validação de rosto).
//!
//! Todas as operações escrevem em `Corner::normal` (normal customizada por
//! corner) e **nunca movem vértices**. Convenção de bloco: validar tudo antes de
//! mutar — em `Err` a malha permanece intacta.
//!
//! O módulo contém ainda [`TriSurface`], uma grade uniforme sobre triângulos com
//! consulta de ponto mais próximo e raycast, usada também por `attributes.rs`
//! (AO por vértice e transferência de pesos). Ela é independente de `bvh.rs`.

use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{EdgeKey, EditMesh};
use crate::normals::{compute_corner_normals, NormalParams, NormalWeighting};
use crate::topology::Topology;
use glam::{Quat, Vec2, Vec3};
use std::collections::HashMap;

// =============================================================================
// Tipos comuns
// =============================================================================

/// Como a nova normal se combina com a normal atual do corner.
///
/// Com `w` = peso por vértice (1 se não houver pesos), `old` = normal atual
/// (customizada ou calculada) e `new` = normal alvo:
///
/// * `Replace`: `n = normalize(lerp(old, new, w))` (com `w = 1` vira `new`);
/// * `Mix(f)`: `n = normalize(lerp(old, new, clamp(f, 0, 1) * w))`;
/// * `Add`: `n = normalize(old + new * w)`.
///
/// Quando o resultado da mistura é degenerado (normais opostas), usa-se a
/// normal alvo se o fator ≥ 0.5, senão a normal antiga.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormalBlend {
    Replace,
    Mix(f32),
    Add,
}

/// Opções comuns às operações de edição de normais.
#[derive(Debug, Clone, Copy)]
pub struct NormalEditOptions<'a> {
    pub blend: NormalBlend,
    /// Peso por **vértice** (0..1), por exemplo vindo de proportional editing
    /// ou de uma máscara; comprimento deve ser igual ao número de vértices.
    pub weights: Option<&'a [f32]>,
}

impl Default for NormalEditOptions<'_> {
    fn default() -> Self {
        Self {
            blend: NormalBlend::Replace,
            weights: None,
        }
    }
}

/// Quais corners uma operação afeta.
#[derive(Debug, Clone, PartialEq)]
pub enum NormalScope {
    /// Todos os corners da malha.
    All,
    /// Todos os corners cujo vértice está na lista.
    Verts(Vec<u32>),
    /// Todos os corners das faces listadas.
    Faces(Vec<u32>),
}

/// Modo de ponderação de [`average_normals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AverageMode {
    /// Pesa pela área da face do corner (*Face Area*).
    FaceArea,
    /// Pesa pelo ângulo interno do corner (*Corner Angle*).
    CornerAngle,
    /// Média simples (*Custom Normal*).
    Uniform,
}

/// Modo de busca em [`transfer_normals_from_mesh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalMapping {
    /// Normal interpolada (baricêntrica) no ponto mais próximo da superfície.
    ClosestSurface,
    /// Normal do vértice (média dos corners) mais próximo do triângulo mais próximo.
    ClosestVertex,
    /// Normal do corner mais próximo do triângulo mais próximo.
    ClosestCorner,
}

// =============================================================================
// Utilidades internas
// =============================================================================

#[inline]
fn corner_angle(pts: &[Vec3], i: usize) -> f32 {
    let n = pts.len();
    let prev = (pts[(i + n - 1) % n] - pts[i]).normalize_or_zero();
    let next = (pts[(i + 1) % n] - pts[i]).normalize_or_zero();
    prev.dot(next).clamp(-1.0, 1.0).acos()
}

/// Normais efetivas por corner: customizadas quando existem, calculadas
/// (grupos suaves + ângulo automático) nos demais — o que o renderer mostra.
pub fn effective_corner_normals(mesh: &EditMesh) -> Vec<Vec<Vec3>> {
    compute_corner_normals(mesh, &NormalParams::default())
}

fn check_vec3(v: Vec3, what: &str) -> ModelResult<()> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(ModelError::InvalidParameter(format!("{what} não finito")))
    }
}

fn validate_options(mesh: &EditMesh, opts: &NormalEditOptions) -> ModelResult<()> {
    if let NormalBlend::Mix(f) = opts.blend {
        if !f.is_finite() {
            return Err(ModelError::InvalidParameter("fator de mistura não finito".into()));
        }
    }
    if let Some(w) = opts.weights {
        if w.len() != mesh.vert_count() {
            return Err(ModelError::InvalidParameter(format!(
                "pesos têm {} entradas, a malha tem {} vértices",
                w.len(),
                mesh.vert_count()
            )));
        }
        if w.iter().any(|x| !x.is_finite()) {
            return Err(ModelError::InvalidParameter("peso não finito".into()));
        }
    }
    Ok(())
}

/// Resolve o escopo em pares `(face, corner)` ordenados; valida índices.
pub fn resolve_scope(mesh: &EditMesh, scope: &NormalScope) -> ModelResult<Vec<(u32, u32)>> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    match scope {
        NormalScope::All => {
            for (fi, f) in mesh.faces.iter().enumerate() {
                for ci in 0..f.corners.len() {
                    out.push((fi as u32, ci as u32));
                }
            }
        }
        NormalScope::Verts(vs) => {
            let mut mark = vec![false; mesh.vert_count()];
            for &v in vs {
                if v as usize >= mesh.vert_count() {
                    return Err(ModelError::VertexOutOfRange(v, mesh.vert_count()));
                }
                mark[v as usize] = true;
            }
            for (fi, f) in mesh.faces.iter().enumerate() {
                for (ci, c) in f.corners.iter().enumerate() {
                    if mark[c.vert as usize] {
                        out.push((fi as u32, ci as u32));
                    }
                }
            }
        }
        NormalScope::Faces(fs) => {
            let mut sorted: Vec<u32> = Vec::with_capacity(fs.len());
            for &f in fs {
                if f as usize >= mesh.face_count() {
                    return Err(ModelError::FaceOutOfRange(f, mesh.face_count()));
                }
                sorted.push(f);
            }
            sorted.sort_unstable();
            sorted.dedup();
            for f in sorted {
                for ci in 0..mesh.faces[f as usize].corners.len() {
                    out.push((f, ci as u32));
                }
            }
        }
    }
    if out.is_empty() {
        return Err(ModelError::EmptySelection("nenhum corner no escopo de normais"));
    }
    Ok(out)
}

fn blend_normals(old: Vec3, new: Vec3, blend: NormalBlend, w: f32) -> Vec3 {
    let w = w.clamp(0.0, 1.0);
    match blend {
        NormalBlend::Add => (old + new * w).try_normalize().unwrap_or(new),
        NormalBlend::Replace | NormalBlend::Mix(_) => {
            let f = match blend {
                NormalBlend::Mix(f) => f.clamp(0.0, 1.0),
                _ => 1.0,
            };
            let t = f * w;
            if t >= 1.0 {
                return new;
            }
            if t <= 0.0 {
                return old;
            }
            (old * (1.0 - t) + new * t)
                .try_normalize()
                .unwrap_or(if t >= 0.5 { new } else { old })
        }
    }
}

/// Aplica normais alvo (um por corner de `corners`; vetor nulo = pular).
/// Já assume entradas validadas. Devolve quantos corners foram escritos.
fn apply_targets(
    mesh: &mut EditMesh,
    corners: &[(u32, u32)],
    targets: &[Vec3],
    opts: &NormalEditOptions,
) -> usize {
    let needs_old = !(matches!(opts.blend, NormalBlend::Replace) && opts.weights.is_none());
    let old = if needs_old {
        Some(effective_corner_normals(mesh))
    } else {
        None
    };
    let mut written = 0;
    for (k, &(f, c)) in corners.iter().enumerate() {
        let Some(new) = targets[k].try_normalize() else {
            continue;
        };
        let vert = mesh.faces[f as usize].corners[c as usize].vert as usize;
        let w = opts.weights.map(|w| w[vert]).unwrap_or(1.0);
        let n = match &old {
            Some(old) => blend_normals(old[f as usize][c as usize], new, opts.blend, w),
            None => new,
        };
        mesh.faces[f as usize].corners[c as usize].normal = Some(n);
        written += 1;
    }
    written
}

fn corner_pos(mesh: &EditMesh, f: u32, c: u32) -> Vec3 {
    mesh.verts[mesh.faces[f as usize].corners[c as usize].vert as usize].co
}

// =============================================================================
// Edição de normais
// =============================================================================

/// Aponta as normais **radialmente a partir de `center`**: `n = normalize(p − center)`.
///
/// É a "esfera de sombreamento" clássica de cabeça/cabelo em anime: a forma da
/// sombra passa a ser a de uma esfera, independente da malha. `invert = true`
/// inverte o sentido (`center − p`). Corners exatamente em `center` são
/// ignorados (direção indefinida). Equivale ao *Point to Target* do Blender com
/// o alvo no centro; para a variante clássica *Normal Edit* use `invert = false`.
///
/// Devolve o número de corners escritos.
pub fn transfer_normals_from_point(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    center: Vec3,
    invert: bool,
    opts: &NormalEditOptions,
) -> ModelResult<usize> {
    check_vec3(center, "centro")?;
    validate_options(mesh, opts)?;
    let corners = resolve_scope(mesh, scope)?;
    let targets: Vec<Vec3> = corners
        .iter()
        .map(|&(f, c)| {
            let d = corner_pos(mesh, f, c) - center;
            if invert {
                -d
            } else {
                d
            }
        })
        .collect();
    Ok(apply_targets(mesh, &corners, &targets, opts))
}

/// Aponta todas as normais do escopo para a direção `dir` (normalizada).
/// `dir` nulo ou não finito é erro. Equivale a *Set From Faces* com direção fixa.
pub fn transfer_normals_directional(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    dir: Vec3,
    opts: &NormalEditOptions,
) -> ModelResult<usize> {
    check_vec3(dir, "direção")?;
    if dir.try_normalize().is_none() {
        return Err(ModelError::InvalidParameter("direção nula".into()));
    }
    validate_options(mesh, opts)?;
    let corners = resolve_scope(mesh, scope)?;
    let targets = vec![dir; corners.len()];
    Ok(apply_targets(mesh, &corners, &targets, opts))
}

/// *Point to Target*: cada normal aponta **para** `target`
/// (`n = normalize(target − p)`). Útil para fazer a luz "enxergar" o rosto como
/// se todo o sombreamento convergisse para um ponto.
pub fn align_normals_to_target(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    target: Vec3,
    opts: &NormalEditOptions,
) -> ModelResult<usize> {
    transfer_normals_from_point(mesh, scope, target, true, opts)
}

/// *Average Normals*: une as normais dos corners do escopo ao redor de cada
/// vértice numa só (média ponderada das normais atuais). Para um cubo duro isso
/// produz a normal diagonal em cada quina. Em `NormalScope::Faces` só entram
/// os corners das faces escolhidas.
pub fn average_normals(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    mode: AverageMode,
    opts: &NormalEditOptions,
) -> ModelResult<usize> {
    validate_options(mesh, opts)?;
    let corners = resolve_scope(mesh, scope)?;
    let current = effective_corner_normals(mesh);
    let mut sums: HashMap<u32, Vec3> = HashMap::new();
    for &(f, c) in &corners {
        let face = &mesh.faces[f as usize];
        let vert = face.corners[c as usize].vert;
        let w = match mode {
            AverageMode::Uniform => 1.0,
            AverageMode::FaceArea => mesh.face_area(f),
            AverageMode::CornerAngle => corner_angle(&mesh.face_positions(f), c as usize),
        };
        *sums.entry(vert).or_insert(Vec3::ZERO) += current[f as usize][c as usize] * w;
    }
    let targets: Vec<Vec3> = corners
        .iter()
        .map(|&(f, c)| {
            let vert = mesh.faces[f as usize].corners[c as usize].vert;
            sums.get(&vert).copied().unwrap_or(Vec3::ZERO)
        })
        .collect();
    Ok(apply_targets(mesh, &corners, &targets, opts))
}

/// Remove as normais customizadas do escopo (volta ao cálculo automático).
/// Devolve quantos corners tinham normal customizada.
pub fn reset_normals(mesh: &mut EditMesh, scope: &NormalScope) -> ModelResult<usize> {
    let corners = resolve_scope(mesh, scope)?;
    let mut n = 0;
    for (f, c) in corners {
        let slot = &mut mesh.faces[f as usize].corners[c as usize].normal;
        if slot.take().is_some() {
            n += 1;
        }
    }
    Ok(n)
}

/// Gira as normais atuais do escopo em `angle` radianos em torno de `axis`
/// (regra da mão direita). O ângulo é multiplicado pelo peso do vértice.
/// Eixo nulo ou ângulo não finito é erro.
pub fn rotate_normals(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    axis: Vec3,
    angle: f32,
    weights: Option<&[f32]>,
) -> ModelResult<usize> {
    check_vec3(axis, "eixo")?;
    let axis = axis
        .try_normalize()
        .ok_or_else(|| ModelError::InvalidParameter("eixo de rotação nulo".into()))?;
    if !angle.is_finite() {
        return Err(ModelError::InvalidParameter("ângulo não finito".into()));
    }
    validate_options(
        mesh,
        &NormalEditOptions {
            blend: NormalBlend::Replace,
            weights,
        },
    )?;
    let corners = resolve_scope(mesh, scope)?;
    let current = effective_corner_normals(mesh);
    for &(f, c) in &corners {
        let vert = mesh.faces[f as usize].corners[c as usize].vert as usize;
        let w = weights.map(|w| w[vert].clamp(0.0, 1.0)).unwrap_or(1.0);
        let q = Quat::from_axis_angle(axis, angle * w);
        let n = (q * current[f as usize][c as usize]).try_normalize();
        if let Some(n) = n {
            mesh.faces[f as usize].corners[c as usize].normal = Some(n);
        }
    }
    Ok(corners.len())
}

/// Normal atual (efetiva) de um corner — "copiar normal" do Blender.
pub fn copy_corner_normal(mesh: &EditMesh, face: u32, corner: u32) -> ModelResult<Vec3> {
    let f = mesh
        .faces
        .get(face as usize)
        .ok_or(ModelError::FaceOutOfRange(face, mesh.face_count()))?;
    if corner as usize >= f.corners.len() {
        return Err(ModelError::InvalidParameter(format!(
            "corner {corner} fora da face {face} ({} corners)",
            f.corners.len()
        )));
    }
    Ok(effective_corner_normals(mesh)[face as usize][corner as usize])
}

/// Normal média (unitária) de todos os corners de um vértice — "copiar normal
/// do vértice ativo". Erro se o vértice não pertence a nenhuma face.
pub fn copy_vertex_normal(mesh: &EditMesh, vert: u32) -> ModelResult<Vec3> {
    if vert as usize >= mesh.vert_count() {
        return Err(ModelError::VertexOutOfRange(vert, mesh.vert_count()));
    }
    let cn = effective_corner_normals(mesh);
    let mut sum = Vec3::ZERO;
    for (fi, f) in mesh.faces.iter().enumerate() {
        for (ci, c) in f.corners.iter().enumerate() {
            if c.vert == vert {
                sum += cn[fi][ci];
            }
        }
    }
    sum.try_normalize()
        .ok_or(ModelError::EmptySelection("vértice sem normais utilizáveis"))
}

/// "Colar normal": aponta o escopo para `normal` (equivale a
/// [`transfer_normals_directional`] com a normal copiada).
pub fn paste_normal(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    normal: Vec3,
    opts: &NormalEditOptions,
) -> ModelResult<usize> {
    transfer_normals_directional(mesh, scope, normal, opts)
}

/// Suavização laplaciana de normais, **sem mover vértices**.
///
/// Seja `N_v` a normal média (unitária) dos corners do vértice `v`. A cada
/// iteração (Jacobi, todas as atualizações usam o estado anterior), para cada
/// corner `c` do escopo no vértice `v`:
///
/// `n_c ← normalize((1 − f)·n_c + f·normalize(mean_{u ∈ vizinhos(v)} N_u))`
///
/// com `f = factor ∈ [0, 1]`. Vértices vizinhos fora do escopo servem de
/// condição de contorno fixa. Preserva a variação por corner (normais divididas)
/// enquanto remove ruído de alta frequência. Devolve corners atualizados.
pub fn smooth_normals_region(
    mesh: &mut EditMesh,
    scope: &NormalScope,
    iterations: u32,
    factor: f32,
) -> ModelResult<usize> {
    if !factor.is_finite() || !(0.0..=1.0).contains(&factor) {
        return Err(ModelError::InvalidParameter(format!(
            "fator {factor} fora de [0, 1]"
        )));
    }
    let corners = resolve_scope(mesh, scope)?;
    if iterations == 0 || factor == 0.0 {
        return Ok(0);
    }
    let topo = Topology::build(mesh);
    let mut cn = effective_corner_normals(mesh);
    let nv = mesh.vert_count();
    let mut touched = vec![false; mesh.face_count()];
    for &(f, _) in &corners {
        touched[f as usize] = true;
    }
    for _ in 0..iterations {
        let mut vsum = vec![Vec3::ZERO; nv];
        for (fi, f) in mesh.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                vsum[c.vert as usize] += cn[fi][ci];
            }
        }
        let vn: Vec<Vec3> = vsum.iter().map(|s| s.normalize_or_zero()).collect();
        let mut next = cn.clone();
        for &(f, c) in &corners {
            let v = mesh.faces[f as usize].corners[c as usize].vert;
            let nbrs = topo.vert_neighbors(v);
            let mut acc = Vec3::ZERO;
            for u in nbrs {
                acc += vn[u as usize];
            }
            let Some(avg) = acc.try_normalize() else {
                continue;
            };
            let cur = cn[f as usize][c as usize];
            if let Some(n) = (cur * (1.0 - factor) + avg * factor).try_normalize() {
                next[f as usize][c as usize] = n;
            }
        }
        cn = next;
    }
    for &(f, c) in &corners {
        mesh.faces[f as usize].corners[c as usize].normal = Some(cn[f as usize][c as usize]);
    }
    Ok(corners.len())
}

// =============================================================================
// Transferência de um mesh proxy
// =============================================================================

/// Copia normais por corner de `src` (proxy: esfera de cabeça, rosto
/// simplificado) para `dst`, usando a posição do vértice de `dst`.
///
/// * `mapping`: ver [`NormalMapping`]. `ClosestVertex` escolhe o vértice mais
///   próximo **do triângulo mais próximo** (aproximação exata quando `dst` está
///   próximo da superfície do proxy, o caso típico), não o vértice global.
/// * `mix_factor ∈ [0, 1]`: mistura com a normal atual (`1` = substitui).
/// * `max_distance`: corners cujo vértice está mais longe que isto do proxy são
///   ignorados (`None` = sem limite).
/// * `weights`: peso opcional por vértice de `dst`, multiplicado ao fator.
///
/// As normais de `src` são as efetivas (customizadas ou calculadas). Devolve o
/// número de corners escritos.
pub fn transfer_normals_from_mesh(
    dst: &mut EditMesh,
    src: &EditMesh,
    scope: &NormalScope,
    mapping: NormalMapping,
    mix_factor: f32,
    max_distance: Option<f32>,
    weights: Option<&[f32]>,
) -> ModelResult<usize> {
    if !mix_factor.is_finite() || !(0.0..=1.0).contains(&mix_factor) {
        return Err(ModelError::InvalidParameter(format!(
            "mix_factor {mix_factor} fora de [0, 1]"
        )));
    }
    if let Some(d) = max_distance {
        if !d.is_finite() || d < 0.0 {
            return Err(ModelError::InvalidParameter("max_distance inválido".into()));
        }
    }
    let opts = NormalEditOptions {
        blend: NormalBlend::Mix(mix_factor),
        weights,
    };
    validate_options(dst, &opts)?;
    let surface = TriSurface::build(src);
    if surface.tris.is_empty() {
        return Err(ModelError::EmptySelection("mesh de origem sem triângulos"));
    }
    let corners = resolve_scope(dst, scope)?;
    let src_cn = effective_corner_normals(src);
    // normal média por vértice do proxy (para ClosestVertex)
    let mut vsum = vec![Vec3::ZERO; src.vert_count()];
    for (fi, f) in src.faces.iter().enumerate() {
        for (ci, c) in f.corners.iter().enumerate() {
            vsum[c.vert as usize] += src_cn[fi][ci];
        }
    }

    let mut cache: Vec<Option<Option<Vec3>>> = vec![None; dst.vert_count()];
    let mut targets = Vec::with_capacity(corners.len());
    for &(f, c) in &corners {
        let vert = dst.faces[f as usize].corners[c as usize].vert as usize;
        if cache[vert].is_none() {
            let p = dst.verts[vert].co;
            let value = surface.closest_point(p, max_distance).map(|hit| {
                let tri = &surface.tris[hit.tri];
                match mapping {
                    NormalMapping::ClosestSurface => {
                        let mut n = Vec3::ZERO;
                        for k in 0..3 {
                            n += src_cn[tri.face as usize][tri.corners[k] as usize] * hit.bary[k];
                        }
                        n
                    }
                    NormalMapping::ClosestVertex | NormalMapping::ClosestCorner => {
                        let mut best = 0;
                        let mut best_d = f32::INFINITY;
                        for k in 0..3 {
                            let d = tri.p[k].distance_squared(p);
                            if d < best_d {
                                best_d = d;
                                best = k;
                            }
                        }
                        if mapping == NormalMapping::ClosestVertex {
                            vsum[tri.verts[best] as usize]
                        } else {
                            src_cn[tri.face as usize][tri.corners[best] as usize]
                        }
                    }
                }
            });
            cache[vert] = Some(value);
        }
        targets.push(cache[vert].flatten().unwrap_or(Vec3::ZERO));
    }
    Ok(apply_targets(dst, &corners, &targets, &opts))
}

// =============================================================================
// Normais de contorno (inverted hull sem frestas)
// =============================================================================

/// Parâmetros de [`outline_normals_per_vertex`] e derivados.
#[derive(Debug, Clone, Copy)]
pub struct OutlineParams {
    /// Ponderação da média (padrão: ângulo do corner, como `normals.rs`).
    pub weighting: NormalWeighting,
    /// Distância de soldagem por posição (0 = apenas posições idênticas).
    pub weld_epsilon: f32,
}

impl Default for OutlineParams {
    fn default() -> Self {
        Self {
            weighting: NormalWeighting::Angle,
            weld_epsilon: 1.0e-5,
        }
    }
}

/// Grade hash de pontos para busca por vizinhança (usada em soldagem/espelho).
pub(crate) struct PointHash {
    cell: f32,
    map: HashMap<(i32, i32, i32), Vec<u32>>,
}

impl PointHash {
    /// `cell` precisa ser > 0 e finito.
    pub(crate) fn new(points: impl Iterator<Item = (u32, Vec3)>, cell: f32) -> Self {
        let mut map: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        let cell = cell.max(1e-12);
        for (id, p) in points {
            map.entry(Self::key_of(p, cell)).or_default().push(id);
        }
        Self { cell, map }
    }

    fn key_of(p: Vec3, cell: f32) -> (i32, i32, i32) {
        let q = |x: f32| (x / cell).floor().clamp(-2.0e9, 2.0e9) as i32;
        (q(p.x), q(p.y), q(p.z))
    }

    /// Ids na vizinhança 3×3×3 da célula de `p` (candidatos; o chamador filtra
    /// por distância). Para `radius ≤ cell` não perde nenhum ponto.
    pub(crate) fn candidates(&self, p: Vec3) -> Vec<u32> {
        let (cx, cy, cz) = Self::key_of(p, self.cell);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = (
                        cx.saturating_add(dx),
                        cy.saturating_add(dy),
                        cz.saturating_add(dz),
                    );
                    if let Some(v) = self.map.get(&key) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out
    }
}

/// Agrupa vértices coincidentes (distância ≤ `eps`) e devolve o representante
/// (menor índice) de cada vértice. Determinístico (union-find com raiz = menor índice).
pub fn weld_clusters(positions: &[Vec3], eps: f32) -> Vec<u32> {
    let n = positions.len();
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            parent[x as usize] = parent[parent[x as usize] as usize];
            x = parent[x as usize];
        }
        x
    }
    let union = |parent: &mut Vec<u32>, a: u32, b: u32| {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent[hi as usize] = lo;
        }
    };
    if eps <= 0.0 {
        let mut map: HashMap<[u32; 3], u32> = HashMap::new();
        for (i, p) in positions.iter().enumerate() {
            // -0.0 e +0.0 devem soldar
            let key = [
                (p.x + 0.0).to_bits(),
                (p.y + 0.0).to_bits(),
                (p.z + 0.0).to_bits(),
            ];
            match map.get(&key) {
                Some(&j) => union(&mut parent, j, i as u32),
                None => {
                    map.insert(key, i as u32);
                }
            }
        }
    } else {
        let hash = PointHash::new(positions.iter().enumerate().map(|(i, p)| (i as u32, *p)), eps);
        let eps2 = eps * eps;
        for (i, p) in positions.iter().enumerate() {
            for j in hash.candidates(*p) {
                if (j as usize) > i && positions[j as usize].distance_squared(*p) <= eps2 {
                    union(&mut parent, i as u32, j);
                }
            }
        }
    }
    (0..n as u32).map(|i| find(&mut parent, i)).collect()
}

/// **Normais suavizadas de contorno por vértice** (uma por vértice de `mesh`).
///
/// Para o inverted hull sem frestas, os corners que ocupam a mesma posição
/// precisam receber a **mesma** normal de deslocamento. Fórmula: vértices
/// soldados por posição (ignorando sharp, seams, divisões de UV e `Face::smooth`)
/// formam um cluster `C`; a normal é
///
/// `n_C = normalize( Σ_{corners (f,i) ∈ C} w(f,i) · N_f )`
///
/// onde `N_f` é a normal (Newell) unitária da face e `w` é o ângulo interno do
/// corner (padrão), a área, ângulo×área ou 1 — [`OutlineParams::weighting`].
/// Faces degeneradas não contribuem. Se a soma for nula (ex.: casca fechada de
/// duas faces opostas), usa-se a direção do centro da malha ao vértice e, em
/// último caso, `+Y`.
pub fn outline_normals_per_vertex(mesh: &EditMesh, params: &OutlineParams) -> ModelResult<Vec<Vec3>> {
    if !params.weld_epsilon.is_finite() || params.weld_epsilon < 0.0 {
        return Err(ModelError::InvalidParameter("weld_epsilon inválido".into()));
    }
    mesh.validate()?;
    let positions: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let rep = weld_clusters(&positions, params.weld_epsilon);
    let mut acc = vec![Vec3::ZERO; mesh.vert_count()];
    for (fi, face) in mesh.faces.iter().enumerate() {
        let pts = mesh.face_positions(fi as u32);
        let nf = crate::mesh::newell_normal(&pts);
        if nf == Vec3::ZERO {
            continue;
        }
        let area = mesh.face_area(fi as u32);
        for (ci, c) in face.corners.iter().enumerate() {
            let w = match params.weighting {
                NormalWeighting::Angle => corner_angle(&pts, ci),
                NormalWeighting::Area => area,
                NormalWeighting::AngleArea => corner_angle(&pts, ci) * area,
                NormalWeighting::Uniform => 1.0,
            };
            acc[rep[c.vert as usize] as usize] += nf * w;
        }
    }
    let centroid = mesh.centroid();
    let mut out = vec![Vec3::Y; mesh.vert_count()];
    for v in 0..mesh.vert_count() {
        let r = rep[v] as usize;
        out[v] = acc[r]
            .try_normalize()
            .or_else(|| (mesh.verts[r].co - centroid).try_normalize())
            .unwrap_or(Vec3::Y);
    }
    Ok(out)
}

/// Normais de contorno por corner (`result[face][corner]`): cada corner recebe a
/// normal do seu vértice soldado — corners coincidentes ficam idênticos.
pub fn outline_normals_per_corner(mesh: &EditMesh, params: &OutlineParams) -> ModelResult<Vec<Vec<Vec3>>> {
    let pv = outline_normals_per_vertex(mesh, params)?;
    Ok(mesh
        .faces
        .iter()
        .map(|f| f.corners.iter().map(|c| pv[c.vert as usize]).collect())
        .collect())
}

/// Versão achatada de [`outline_normals_per_corner`]: um `Vec3` por corner na
/// ordem face-major (face 0 corner 0, face 0 corner 1, …).
pub fn bake_outline_normals(mesh: &EditMesh, params: &OutlineParams) -> ModelResult<Vec<Vec3>> {
    Ok(outline_normals_per_corner(mesh, params)?
        .into_iter()
        .flatten()
        .collect())
}

/// Codificação octaédrica de uma direção unitária em `[-1, 1]²`
/// (Cigolle et al., "A Survey of Efficient Representations for Independent
/// Unit Vectors"). Vetores nulos/não finitos codificam `+Z`.
pub fn oct_encode(n: Vec3) -> Vec2 {
    let n = n.try_normalize().unwrap_or(Vec3::Z);
    let l1 = n.x.abs() + n.y.abs() + n.z.abs();
    let mut p = Vec2::new(n.x, n.y) / l1;
    if n.z < 0.0 {
        let sx = if p.x >= 0.0 { 1.0 } else { -1.0 };
        let sy = if p.y >= 0.0 { 1.0 } else { -1.0 };
        p = Vec2::new((1.0 - p.y.abs()) * sx, (1.0 - p.x.abs()) * sy);
    }
    p
}

/// Inversa de [`oct_encode`]; sempre devolve vetor unitário.
pub fn oct_decode(e: Vec2) -> Vec3 {
    let e = Vec2::new(e.x.clamp(-1.0, 1.0), e.y.clamp(-1.0, 1.0));
    let mut n = Vec3::new(e.x, e.y, 1.0 - e.x.abs() - e.y.abs());
    if n.z < 0.0 {
        let sx = if n.x >= 0.0 { 1.0 } else { -1.0 };
        let sy = if n.y >= 0.0 { 1.0 } else { -1.0 };
        let (x, y) = (n.x, n.y);
        n.x = (1.0 - y.abs()) * sx;
        n.y = (1.0 - x.abs()) * sy;
    }
    n.try_normalize().unwrap_or(Vec3::Z)
}

/// Base ortonormal (T, B, N) a partir da normal original e de uma tangente
/// (re-ortogonalizada). Tangente degenerada → qualquer perpendicular estável.
pub fn tangent_frame(normal: Vec3, tangent: Vec3) -> (Vec3, Vec3, Vec3) {
    let n = normal.try_normalize().unwrap_or(Vec3::Y);
    let t = (tangent - n * n.dot(tangent)).try_normalize().unwrap_or_else(|| {
        let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        (helper - n * n.dot(helper)).normalize_or_zero()
    });
    let b = n.cross(t);
    (t, b, n)
}

/// Escreve a normal de contorno `outline` no **espaço tangente** da normal
/// original `normal` / `tangent` e a codifica em octaedro (`Vec2` para UV1).
/// Guardar em espaço tangente faz o contorno sobreviver a skinning e morphs.
pub fn pack_tangent_space(outline: Vec3, normal: Vec3, tangent: Vec3) -> Vec2 {
    let (t, b, n) = tangent_frame(normal, tangent);
    let o = outline.try_normalize().unwrap_or(n);
    oct_encode(Vec3::new(o.dot(t), o.dot(b), o.dot(n)))
}

/// Inversa de [`pack_tangent_space`] (usa o mesmo `normal`/`tangent`).
pub fn unpack_tangent_space(packed: Vec2, normal: Vec3, tangent: Vec3) -> Vec3 {
    let (t, b, n) = tangent_frame(normal, tangent);
    let l = oct_decode(packed);
    (t * l.x + b * l.y + n * l.z).normalize_or_zero()
}

/// Tangente de UV de um corner (derivada de posição em relação a `u`). Cai para
/// uma perpendicular arbitrária quando a UV é degenerada.
pub fn corner_tangent(mesh: &EditMesh, face: u32, corner: u32, normal: Vec3) -> Vec3 {
    let Some(f) = mesh.faces.get(face as usize) else {
        return tangent_frame(normal, Vec3::ZERO).0;
    };
    let n = f.corners.len();
    let i = corner as usize % n.max(1);
    if n >= 3 {
        let p = mesh.face_positions(face);
        let e1 = p[(i + 1) % n] - p[i];
        let e2 = p[(i + n - 1) % n] - p[i];
        let d1 = f.corners[(i + 1) % n].uv - f.corners[i].uv;
        let d2 = f.corners[(i + n - 1) % n].uv - f.corners[i].uv;
        let det = d1.x * d2.y - d1.y * d2.x;
        if det.abs() > 1e-12 {
            let t = (e1 * d2.y - e2 * d1.y) / det;
            if t.try_normalize().is_some() {
                return tangent_frame(normal, t).0;
            }
        }
    }
    tangent_frame(normal, Vec3::ZERO).0
}

/// Normais de contorno em espaço tangente, codificadas em octaedro, por corner
/// (`result[face][corner]`), prontas para UV1. A normal/tangente base de cada
/// corner são a normal efetiva e a tangente de UV ([`corner_tangent`]).
pub fn bake_outline_tangent_space(mesh: &EditMesh, params: &OutlineParams) -> ModelResult<Vec<Vec<Vec2>>> {
    let outline = outline_normals_per_corner(mesh, params)?;
    let base = effective_corner_normals(mesh);
    Ok(mesh
        .faces
        .iter()
        .enumerate()
        .map(|(fi, f)| {
            (0..f.corners.len())
                .map(|ci| {
                    let n = base[fi][ci];
                    let t = corner_tangent(mesh, fi as u32, ci as u32, n);
                    pack_tangent_space(outline[fi][ci], n, t)
                })
                .collect()
        })
        .collect())
}

// =============================================================================
// Métrica de suavidade
// =============================================================================

/// Resultado de [`normal_smoothness_report`]. Ângulos em radianos.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalReport {
    /// Arestas medidas.
    pub edge_count: usize,
    /// Maior salto entre as normais de um mesmo vértice em faces vizinhas
    /// (descontinuidade através da aresta).
    pub max_discontinuity: f32,
    /// Maior variação entre as normais dos dois extremos de uma aresta
    /// (gradiente ao longo da aresta) — o "salto entre vizinhos".
    pub max_gradient: f32,
    pub mean_gradient: f32,
    /// Aresta onde ocorre o maior gradiente.
    pub worst_edge: Option<EdgeKey>,
    /// Arestas com descontinuidade acima de 0.001 rad.
    pub discontinuous_edges: usize,
}

impl NormalReport {
    pub fn max_gradient_deg(&self) -> f32 {
        self.max_gradient.to_degrees()
    }
    pub fn max_discontinuity_deg(&self) -> f32 {
        self.max_discontinuity.to_degrees()
    }
}

/// Ângulo entre vetores via `atan2(|a × b|, a · b)` (estável perto de 0 e π,
/// ao contrário de `acos(a · b)`).
fn angle_between(a: Vec3, b: Vec3) -> f32 {
    a.cross(b).length().atan2(a.dot(b))
}

/// Mede a suavidade das normais efetivas em toda a malha.
pub fn normal_smoothness_report(mesh: &EditMesh) -> NormalReport {
    report_impl(mesh, None)
}

/// Como [`normal_smoothness_report`], restrito às arestas que tocam as `faces`
/// dadas (região do rosto, por exemplo). Índice inválido é erro.
pub fn normal_smoothness_report_region(mesh: &EditMesh, faces: &[u32]) -> ModelResult<NormalReport> {
    let mut mark = vec![false; mesh.face_count()];
    for &f in faces {
        if f as usize >= mesh.face_count() {
            return Err(ModelError::FaceOutOfRange(f, mesh.face_count()));
        }
        mark[f as usize] = true;
    }
    Ok(report_impl(mesh, Some(&mark)))
}

fn report_impl(mesh: &EditMesh, region: Option<&[bool]>) -> NormalReport {
    let topo = Topology::build(mesh);
    let cn = effective_corner_normals(mesh);
    let normal_at = |f: u32, v: u32| -> Option<Vec3> {
        mesh.faces[f as usize]
            .corners
            .iter()
            .position(|c| c.vert == v)
            .map(|ci| cn[f as usize][ci])
    };
    let mut rep = NormalReport {
        edge_count: 0,
        max_discontinuity: 0.0,
        max_gradient: 0.0,
        mean_gradient: 0.0,
        worst_edge: None,
        discontinuous_edges: 0,
    };
    let mut grad_sum = 0.0f64;
    for e in &topo.edges {
        if e.faces.is_empty() {
            continue;
        }
        if let Some(mark) = region {
            if !e.faces.iter().any(|&f| mark[f as usize]) {
                continue;
            }
        }
        let (a, b) = e.verts;
        rep.edge_count += 1;
        let mut edge_grad = 0.0f32;
        for &f in &e.faces {
            if let (Some(na), Some(nb)) = (normal_at(f, a), normal_at(f, b)) {
                edge_grad = edge_grad.max(angle_between(na, nb));
            }
        }
        let mut edge_disc = 0.0f32;
        if e.faces.len() >= 2 {
            let f0 = e.faces[0];
            for &f1 in &e.faces[1..] {
                for v in [a, b] {
                    if let (Some(n0), Some(n1)) = (normal_at(f0, v), normal_at(f1, v)) {
                        edge_disc = edge_disc.max(angle_between(n0, n1));
                    }
                }
            }
        }
        grad_sum += edge_grad as f64;
        if edge_grad > rep.max_gradient {
            rep.max_gradient = edge_grad;
            rep.worst_edge = Some(e.verts);
        }
        rep.max_discontinuity = rep.max_discontinuity.max(edge_disc);
        if edge_disc > 1e-3 {
            rep.discontinuous_edges += 1;
        }
    }
    if rep.edge_count > 0 {
        rep.mean_gradient = (grad_sum / rep.edge_count as f64) as f32;
    }
    rep
}

// =============================================================================
// TriSurface: grade uniforme sobre triângulos (ponto mais próximo + raycast)
// =============================================================================

/// Triângulo da superfície, com a origem (face/corners/vértices) do polígono.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceTri {
    pub p: [Vec3; 3],
    pub face: u32,
    /// Índice de corner dentro da face de cada ponta do triângulo.
    pub corners: [u32; 3],
    /// Índice de vértice da malha de cada ponta.
    pub verts: [u32; 3],
}

/// Resultado de [`TriSurface::closest_point`].
#[derive(Debug, Clone, Copy)]
pub struct SurfaceHit {
    pub tri: usize,
    pub point: Vec3,
    /// Coordenadas baricêntricas do ponto no triângulo (somam 1).
    pub bary: [f32; 3],
    pub dist: f32,
}

/// Triângulos de uma malha em grade uniforme (CSR) para consultas espaciais.
#[derive(Debug, Clone)]
pub struct TriSurface {
    pub tris: Vec<SurfaceTri>,
    min: Vec3,
    cell: Vec3,
    res: [usize; 3],
    starts: Vec<u32>,
    items: Vec<u32>,
}

/// Ponto mais próximo de `p` no triângulo `abc` e baricêntricas
/// (Ericson, *Real-Time Collision Detection* §5.1.5). Triângulos degenerados
/// caem para o segmento mais próximo.
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let den = d1 - d3;
        let v = if den.abs() > 1e-30 { d1 / den } else { 0.0 };
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let den = d2 - d6;
        let w = if den.abs() > 1e-30 { d2 / den } else { 0.0 };
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let den = (d4 - d3) + (d5 - d6);
        let w = if den.abs() > 1e-30 { (d4 - d3) / den } else { 0.0 };
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-30 {
        // degenerado: melhor dos três vértices
        let cands = [(a, [1.0, 0.0, 0.0]), (b, [0.0, 1.0, 0.0]), (c, [0.0, 0.0, 1.0])];
        let mut best = cands[0];
        for cand in cands {
            if cand.0.distance_squared(p) < best.0.distance_squared(p) {
                best = cand;
            }
        }
        return best;
    }
    let v = vb / denom;
    let w = vc / denom;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

/// Interseção raio–triângulo (Möller–Trumbore, dois lados). Devolve `t > 0`.
pub fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = inv * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = inv * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = inv * e2.dot(q);
    (t > 0.0).then_some(t)
}

impl TriSurface {
    /// Triangula todas as faces de `mesh` (ear clipping do `convert`) e monta a grade.
    pub fn build(mesh: &EditMesh) -> Self {
        let mut tris: Vec<SurfaceTri> = Vec::new();
        for (fi, face) in mesh.faces.iter().enumerate() {
            let pts = mesh.face_positions(fi as u32);
            for [a, b, c] in triangulate_polygon(&pts) {
                tris.push(SurfaceTri {
                    p: [pts[a], pts[b], pts[c]],
                    face: fi as u32,
                    corners: [a as u32, b as u32, c as u32],
                    verts: [face.corners[a].vert, face.corners[b].vert, face.corners[c].vert],
                });
            }
        }
        Self::from_tris(tris)
    }

    fn from_tris(tris: Vec<SurfaceTri>) -> Self {
        if tris.is_empty() {
            return Self {
                tris,
                min: Vec3::ZERO,
                cell: Vec3::ONE,
                res: [1, 1, 1],
                starts: vec![0, 0],
                items: Vec::new(),
            };
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for t in &tris {
            for p in t.p {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        let ext0 = hi - lo;
        let max_ext = ext0.max_element().max(1e-6);
        let ext = ext0.max(Vec3::splat(max_ext * 1e-3));
        let target = (tris.len() as f32 / 2.0).max(1.0);
        let h = (ext.x * ext.y * ext.z / target).cbrt().max(max_ext * 1e-4);
        let res = [
            ((ext.x / h).ceil() as usize).clamp(1, 128),
            ((ext.y / h).ceil() as usize).clamp(1, 128),
            ((ext.z / h).ceil() as usize).clamp(1, 128),
        ];
        let cell = Vec3::new(
            ext.x / res[0] as f32,
            ext.y / res[1] as f32,
            ext.z / res[2] as f32,
        );
        let mut grid = Self {
            tris,
            min: lo,
            cell,
            res,
            starts: Vec::new(),
            items: Vec::new(),
        };
        let ncells = res[0] * res[1] * res[2];
        let mut counts = vec![0u32; ncells + 1];
        let mut ranges: Vec<([usize; 3], [usize; 3])> = Vec::with_capacity(grid.tris.len());
        for t in &grid.tris {
            let tlo = t.p[0].min(t.p[1]).min(t.p[2]);
            let thi = t.p[0].max(t.p[1]).max(t.p[2]);
            let a = grid.cell_of(tlo);
            let b = grid.cell_of(thi);
            ranges.push((a, b));
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        counts[grid.cell_index(x, y, z) + 1] += 1;
                    }
                }
            }
        }
        for i in 0..ncells {
            counts[i + 1] += counts[i];
        }
        let mut items = vec![0u32; counts[ncells] as usize];
        let mut fill = counts.clone();
        for (ti, (a, b)) in ranges.iter().enumerate() {
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        let ci = grid.cell_index(x, y, z);
                        items[fill[ci] as usize] = ti as u32;
                        fill[ci] += 1;
                    }
                }
            }
        }
        grid.starts = counts;
        grid.items = items;
        grid
    }

    #[inline]
    fn cell_index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.res[1] + y) * self.res[0] + x
    }

    fn cell_of(&self, p: Vec3) -> [usize; 3] {
        let f = |v: f32, lo: f32, size: f32, r: usize| -> usize {
            (((v - lo) / size).floor().max(0.0) as usize).min(r - 1)
        };
        [
            f(p.x, self.min.x, self.cell.x, self.res[0]),
            f(p.y, self.min.y, self.cell.y, self.res[1]),
            f(p.z, self.min.z, self.cell.z, self.res[2]),
        ]
    }

    fn cell_items(&self, x: usize, y: usize, z: usize) -> &[u32] {
        let ci = self.cell_index(x, y, z);
        &self.items[self.starts[ci] as usize..self.starts[ci + 1] as usize]
    }

    /// Ponto mais próximo de `p` na superfície. `max_dist` limita a busca
    /// (devolve `None` se nada estiver dentro). Busca em cascas de células
    /// crescentes com critério de parada exato (`melhor ≤ k·menor_célula`).
    pub fn closest_point(&self, p: Vec3, max_dist: Option<f32>) -> Option<SurfaceHit> {
        if self.tris.is_empty() || !p.is_finite() {
            return None;
        }
        let hi = self.min + self.cell * Vec3::new(self.res[0] as f32, self.res[1] as f32, self.res[2] as f32);
        let q = p.clamp(self.min, hi);
        let start = self.cell_of(q);
        let cs_min = self.cell.min_element();
        let max_ring = self.res[0].max(self.res[1]).max(self.res[2]);
        let mut best: Option<SurfaceHit> = None;
        let limit = max_dist.unwrap_or(f32::INFINITY);
        for k in 0..=max_ring {
            let lo = [
                start[0].saturating_sub(k),
                start[1].saturating_sub(k),
                start[2].saturating_sub(k),
            ];
            let hi_i = [
                (start[0] + k).min(self.res[0] - 1),
                (start[1] + k).min(self.res[1] - 1),
                (start[2] + k).min(self.res[2] - 1),
            ];
            for z in lo[2]..=hi_i[2] {
                for y in lo[1]..=hi_i[1] {
                    for x in lo[0]..=hi_i[0] {
                        let on_shell = x.abs_diff(start[0]) == k
                            || y.abs_diff(start[1]) == k
                            || z.abs_diff(start[2]) == k;
                        if !on_shell {
                            continue;
                        }
                        for &ti in self.cell_items(x, y, z) {
                            let t = &self.tris[ti as usize];
                            let (pt, bary) = closest_point_on_triangle(p, t.p[0], t.p[1], t.p[2]);
                            let d = pt.distance(p);
                            if d <= limit && best.map(|b| d < b.dist).unwrap_or(true) {
                                best = Some(SurfaceHit {
                                    tri: ti as usize,
                                    point: pt,
                                    bary,
                                    dist: d,
                                });
                            }
                        }
                    }
                }
            }
            let bound = k as f32 * cs_min;
            if let Some(b) = best {
                if b.dist <= bound {
                    break;
                }
            }
            if bound > limit {
                break;
            }
        }
        best
    }

    /// Há algum triângulo (não rejeitado por `skip`) atingido pelo raio
    /// `origin + t·dir` com `0 < t ≤ t_max`? `dir` deve ser unitário. Percorre a
    /// grade por DDA (Amanatides–Woo).
    pub fn ray_hits(&self, origin: Vec3, dir: Vec3, t_max: f32, skip: &dyn Fn(&SurfaceTri) -> bool) -> bool {
        if self.tris.is_empty() || !origin.is_finite() || !dir.is_finite() || t_max <= 0.0 {
            return false;
        }
        let hi = self.min + self.cell * Vec3::new(self.res[0] as f32, self.res[1] as f32, self.res[2] as f32);
        // recorta o raio contra a caixa da grade (slab)
        let (mut t0, mut t1) = (0.0f32, t_max);
        for a in 0..3 {
            let (o, d, lo_a, hi_a) = (origin[a], dir[a], self.min[a], hi[a]);
            if d.abs() < 1e-12 {
                if o < lo_a || o > hi_a {
                    return false;
                }
            } else {
                let (mut ta, mut tb) = ((lo_a - o) / d, (hi_a - o) / d);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        let entry = origin + dir * t0;
        let mut c = self.cell_of(entry);
        let mut step = [0i64; 3];
        let mut t_next = [f32::INFINITY; 3];
        let mut t_delta = [f32::INFINITY; 3];
        for a in 0..3 {
            if dir[a].abs() < 1e-12 {
                continue;
            }
            step[a] = if dir[a] > 0.0 { 1 } else { -1 };
            let next_bound = if dir[a] > 0.0 {
                self.min[a] + (c[a] + 1) as f32 * self.cell[a]
            } else {
                self.min[a] + c[a] as f32 * self.cell[a]
            };
            t_next[a] = (next_bound - origin[a]) / dir[a];
            t_delta[a] = self.cell[a] / dir[a].abs();
        }
        loop {
            for &ti in self.cell_items(c[0], c[1], c[2]) {
                let t = &self.tris[ti as usize];
                if skip(t) {
                    continue;
                }
                if let Some(h) = ray_triangle(origin, dir, t.p[0], t.p[1], t.p[2]) {
                    if h <= t_max {
                        return true;
                    }
                }
            }
            // avança para a próxima célula
            let axis = if t_next[0] <= t_next[1] && t_next[0] <= t_next[2] {
                0
            } else if t_next[1] <= t_next[2] {
                1
            } else {
                2
            };
            if t_next[axis] > t1 || !t_next[axis].is_finite() {
                return false;
            }
            let nc = c[axis] as i64 + step[axis];
            if nc < 0 || nc >= self.res[axis] as i64 {
                return false;
            }
            c[axis] = nc as usize;
            t_next[axis] += t_delta[axis];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normals::{compute_vertex_normals, NormalParams};
    use crate::primitives;

    fn all() -> NormalScope {
        NormalScope::All
    }
    fn replace() -> NormalEditOptions<'static> {
        NormalEditOptions::default()
    }

    /// Cubo com vértices duplicados por face (UV splits), sem soldar posições.
    fn split_cube() -> EditMesh {
        let src = primitives::cube(2.0);
        let mut m = EditMesh::new("split");
        for f in &src.faces {
            let ids: Vec<u32> = f.corners.iter().map(|c| m.add_vert(src.co(c.vert))).collect();
            let mut face = f.with_verts_from(&ids);
            for (nc, oc) in face.corners.iter_mut().zip(&f.corners) {
                nc.uv = oc.uv;
            }
            m.push_face(face).unwrap();
        }
        m
    }

    #[test]
    fn oct_roundtrip_error_small() {
        let mut worst = 0.0f32;
        for i in 0..2000 {
            let u = (i as f32 * 0.6180339887).fract() * 2.0 - 1.0;
            let phi = (i as f32 * 0.7548776662).fract() * std::f32::consts::TAU;
            let r = (1.0 - u * u).sqrt();
            let n = Vec3::new(r * phi.cos(), u, r * phi.sin());
            let d = oct_decode(oct_encode(n));
            worst = worst.max((d - n).length());
        }
        // eixos e extremos
        for n in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z, Vec3::splat(-1.0)] {
            let n = n.normalize();
            worst = worst.max((oct_decode(oct_encode(n)) - n).length());
        }
        assert!(worst < 1e-3, "erro {worst}");
        // nulo -> +Z
        assert_eq!(oct_decode(oct_encode(Vec3::ZERO)), Vec3::Z);
        let e = oct_encode(Vec3::new(-0.3, 0.2, -0.9));
        assert!(e.x.abs() <= 1.0 && e.y.abs() <= 1.0);
    }

    #[test]
    fn point_transfer_on_sphere_is_geometry_and_cube_becomes_soft() {
        let mut s = primitives::quad_sphere(1.0, 4);
        let n = transfer_normals_from_point(&mut s, &all(), Vec3::ZERO, false, &replace()).unwrap();
        assert_eq!(n, s.faces.iter().map(|f| f.len()).sum::<usize>());
        let cn = effective_corner_normals(&s);
        for (fi, f) in s.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                assert!((cn[fi][ci] - s.co(c.vert).normalize()).length() < 1e-5);
            }
        }
        // cubo duro com normais da esfera fica macio: gradiente pequeno, sem saltos
        let mut cube = primitives::cube_subdivided(2.0, 3);
        let hard = normal_smoothness_report(&cube);
        assert!(hard.max_discontinuity > 1.0);
        transfer_normals_from_point(&mut cube, &all(), Vec3::ZERO, false, &replace()).unwrap();
        let soft = normal_smoothness_report(&cube);
        assert!(soft.max_discontinuity < 1e-4, "{soft:?}");
        assert!(soft.max_gradient < hard.max_gradient.max(1.5));
        assert!(soft.max_gradient.to_degrees() < 40.0);
        // invert
        let mut s2 = primitives::quad_sphere(1.0, 2);
        transfer_normals_from_point(&mut s2, &all(), Vec3::ZERO, true, &replace()).unwrap();
        assert!(s2.faces[0].corners[0].normal.unwrap().dot(s2.co(s2.faces[0].corners[0].vert)) < 0.0);
    }

    #[test]
    fn blend_modes_and_weights() {
        let mut m = primitives::cube(2.0);
        // Mix 0.5 entre normal de face (+Z) e +X
        let nz = (0..6u32).find(|&i| (m.face_normal(i) - Vec3::Z).length() < 1e-5).unwrap();
        let opts = NormalEditOptions {
            blend: NormalBlend::Mix(0.5),
            weights: None,
        };
        transfer_normals_directional(&mut m, &NormalScope::Faces(vec![nz]), Vec3::X, &opts).unwrap();
        let want = (Vec3::Z + Vec3::X).normalize();
        for c in &m.faces[nz as usize].corners {
            assert!((c.normal.unwrap() - want).length() < 1e-5);
        }
        // Add
        let mut m = primitives::cube(2.0);
        let add = NormalEditOptions {
            blend: NormalBlend::Add,
            weights: None,
        };
        transfer_normals_directional(&mut m, &NormalScope::Faces(vec![nz]), Vec3::X, &add).unwrap();
        for c in &m.faces[nz as usize].corners {
            assert!((c.normal.unwrap() - want).length() < 1e-5);
        }
        // pesos: vértices com peso 0 não mudam, peso 1 mudam
        let mut m = primitives::cube(2.0);
        let mut w = vec![0.0; m.vert_count()];
        let v0 = m.faces[nz as usize].corners[0].vert;
        w[v0 as usize] = 1.0;
        let wopts = NormalEditOptions {
            blend: NormalBlend::Replace,
            weights: Some(&w),
        };
        transfer_normals_directional(&mut m, &NormalScope::Faces(vec![nz]), Vec3::X, &wopts).unwrap();
        for c in &m.faces[nz as usize].corners {
            let n = c.normal.unwrap();
            if c.vert == v0 {
                assert!((n - Vec3::X).length() < 1e-5);
            } else {
                assert!((n - Vec3::Z).length() < 1e-5);
            }
        }
        // oposto: mistura degenerada não gera NaN
        let b = blend_normals(Vec3::X, -Vec3::X, NormalBlend::Mix(0.5), 1.0);
        assert!(b.is_finite() && (b.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn errors_leave_mesh_intact() {
        let mut m = primitives::cube(2.0);
        let h = m.content_hash();
        let before = m.clone();
        assert!(transfer_normals_from_point(&mut m, &all(), Vec3::NAN, false, &replace()).is_err());
        assert!(transfer_normals_directional(&mut m, &all(), Vec3::ZERO, &replace()).is_err());
        assert!(transfer_normals_directional(&mut m, &NormalScope::Verts(vec![99]), Vec3::X, &replace()).is_err());
        assert!(transfer_normals_directional(&mut m, &NormalScope::Faces(vec![99]), Vec3::X, &replace()).is_err());
        assert!(matches!(
            transfer_normals_directional(&mut m, &NormalScope::Verts(vec![]), Vec3::X, &replace()),
            Err(ModelError::EmptySelection(_))
        ));
        let short = [1.0f32; 3];
        let bad = NormalEditOptions {
            blend: NormalBlend::Replace,
            weights: Some(&short),
        };
        assert!(transfer_normals_directional(&mut m, &all(), Vec3::X, &bad).is_err());
        let nan = vec![f32::NAN; m.vert_count()];
        let bad2 = NormalEditOptions {
            blend: NormalBlend::Replace,
            weights: Some(&nan),
        };
        assert!(transfer_normals_directional(&mut m, &all(), Vec3::X, &bad2).is_err());
        let bad3 = NormalEditOptions {
            blend: NormalBlend::Mix(f32::NAN),
            weights: None,
        };
        assert!(transfer_normals_directional(&mut m, &all(), Vec3::X, &bad3).is_err());
        assert!(rotate_normals(&mut m, &all(), Vec3::ZERO, 1.0, None).is_err());
        assert!(rotate_normals(&mut m, &all(), Vec3::X, f32::NAN, None).is_err());
        assert!(smooth_normals_region(&mut m, &all(), 1, 2.0).is_err());
        assert!(copy_corner_normal(&m, 99, 0).is_err());
        assert!(copy_corner_normal(&m, 0, 9).is_err());
        assert!(copy_vertex_normal(&m, 99).is_err());
        let src = primitives::cube(1.0);
        assert!(transfer_normals_from_mesh(&mut m, &src, &all(), NormalMapping::ClosestSurface, 2.0, None, None).is_err());
        assert!(transfer_normals_from_mesh(&mut m, &src, &all(), NormalMapping::ClosestSurface, 1.0, Some(-1.0), None).is_err());
        let empty = EditMesh::new("e");
        assert!(transfer_normals_from_mesh(&mut m, &empty, &all(), NormalMapping::ClosestSurface, 1.0, None, None).is_err());
        assert!(normal_smoothness_report_region(&m, &[99]).is_err());
        assert_eq!(m, before);
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn align_to_target_points_toward() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let target = Vec3::new(0.0, 0.0, 5.0);
        align_normals_to_target(&mut m, &all(), target, &replace()).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                let want = (target - m.co(c.vert)).normalize();
                assert!((c.normal.unwrap() - want).length() < 1e-5);
            }
        }
    }

    #[test]
    fn average_normals_cube_corner_is_diagonal() {
        let mut m = primitives::cube(2.0);
        for mode in [AverageMode::FaceArea, AverageMode::CornerAngle, AverageMode::Uniform] {
            clear(&mut m);
            let v = 0u32;
            average_normals(&mut m, &NormalScope::Verts(vec![v]), mode, &replace()).unwrap();
            let want = m.co(v).normalize();
            let mut seen = 0;
            for f in &m.faces {
                for c in &f.corners {
                    if c.vert == v {
                        assert!((c.normal.unwrap() - want).length() < 1e-5, "{mode:?}");
                        seen += 1;
                    } else {
                        assert!(c.normal.is_none());
                    }
                }
            }
            assert_eq!(seen, 3);
        }
        // por faces: só os corners das faces escolhidas entram
        clear(&mut m);
        let v = 0u32;
        let faces: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.faces[f as usize].verts().any(|x| x == v))
            .take(2)
            .collect();
        average_normals(&mut m, &NormalScope::Faces(faces.clone()), AverageMode::Uniform, &replace()).unwrap();
        let expect = (m.face_normal(faces[0]) + m.face_normal(faces[1])).normalize();
        let c = m.faces[faces[0] as usize].corners.iter().find(|c| c.vert == v).unwrap();
        assert!((c.normal.unwrap() - expect).length() < 1e-5);
    }

    fn clear(m: &mut EditMesh) {
        crate::normals::clear_custom_normals(m);
    }

    #[test]
    fn reset_rotate_copy_paste() {
        let mut m = primitives::quad_sphere(1.0, 3);
        transfer_normals_directional(&mut m, &all(), Vec3::Y, &replace()).unwrap();
        let n_reset = reset_normals(&mut m, &NormalScope::Faces(vec![0])).unwrap();
        assert_eq!(n_reset, 4);
        assert!(m.faces[0].corners.iter().all(|c| c.normal.is_none()));
        assert!(m.faces[1].corners.iter().all(|c| c.normal.is_some()));
        assert_eq!(reset_normals(&mut m, &NormalScope::Faces(vec![0])).unwrap(), 0);

        // rotacao: +Y em torno de +Z por 90 graus -> -X
        let mut r = primitives::cube(2.0);
        transfer_normals_directional(&mut r, &all(), Vec3::Y, &replace()).unwrap();
        rotate_normals(&mut r, &all(), Vec3::Z, std::f32::consts::FRAC_PI_2, None).unwrap();
        assert!((r.faces[0].corners[0].normal.unwrap() - Vec3::NEG_X).length() < 1e-5);
        // com pesos 0.5 -> 45 graus
        let mut r = primitives::cube(2.0);
        transfer_normals_directional(&mut r, &all(), Vec3::Y, &replace()).unwrap();
        let w = vec![0.5; r.vert_count()];
        rotate_normals(&mut r, &all(), Vec3::Z, std::f32::consts::FRAC_PI_2, Some(&w)).unwrap();
        let want = Vec3::new(-1.0, 1.0, 0.0).normalize();
        assert!((r.faces[0].corners[0].normal.unwrap() - want).length() < 1e-5);

        // copiar/colar
        let mut c = primitives::cube(2.0);
        let n = copy_corner_normal(&c, 0, 0).unwrap();
        assert!((n - c.face_normal(0)).length() < 1e-5);
        let nv = copy_vertex_normal(&c, 0).unwrap();
        assert!((nv.length() - 1.0).abs() < 1e-5);
        paste_normal(&mut c, &NormalScope::Verts(vec![1, 2]), nv, &replace()).unwrap();
        assert!(c.faces.iter().flat_map(|f| &f.corners).filter(|c| c.normal.is_some()).count() >= 6);
        let mut lone = EditMesh::new("l");
        lone.add_vert(Vec3::ZERO);
        assert!(copy_vertex_normal(&lone, 0).is_err());
    }

    #[test]
    fn smoothing_reduces_noise() {
        let mut m = primitives::quad_sphere(1.0, 8);
        transfer_normals_from_point(&mut m, &all(), Vec3::ZERO, false, &replace()).unwrap();
        // ruído determinístico nas normais
        let mut k = 0u32;
        for f in &mut m.faces {
            for c in &mut f.corners {
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let a = (k >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let b = (k >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                let n = c.normal.unwrap();
                c.normal = Some((n + Vec3::new(a, b, a - b) * 0.8).normalize());
            }
        }
        let before = normal_smoothness_report(&m);
        let hash_pos = m.verts.clone();
        smooth_normals_region(&mut m, &all(), 6, 0.8).unwrap();
        let after = normal_smoothness_report(&m);
        assert!(after.max_gradient < before.max_gradient, "{before:?} {after:?}");
        assert!(after.mean_gradient < before.mean_gradient * 0.7);
        assert_eq!(m.verts, hash_pos); // vértices intocados
        assert_eq!(smooth_normals_region(&mut m, &all(), 0, 0.5).unwrap(), 0);
        // região restrita só toca faces escolhidas
        let mut m2 = primitives::quad_sphere(1.0, 4);
        transfer_normals_directional(&mut m2, &all(), Vec3::X, &replace()).unwrap();
        smooth_normals_region(&mut m2, &NormalScope::Faces(vec![0]), 3, 1.0).unwrap();
        assert!(m2.faces[5].corners.iter().all(|c| c.normal == Some(Vec3::X)));
    }

    #[test]
    fn transfer_from_proxy_mesh() {
        // dst: cubo subdividido; proxy: esfera -> normais radiais interpoladas
        let mut dst = primitives::cube_subdivided(2.0, 4);
        let proxy = primitives::quad_sphere(1.2, 8);
        let n = transfer_normals_from_mesh(&mut dst, &proxy, &all(), NormalMapping::ClosestSurface, 1.0, None, None).unwrap();
        assert!(n > 0);
        for f in &dst.faces {
            for c in &f.corners {
                let want = dst.co(c.vert).normalize();
                assert!(c.normal.unwrap().dot(want) > 0.97, "{:?} {:?}", c.normal, want);
            }
        }
        // suave apos transferencia
        let rep = normal_smoothness_report(&dst);
        assert!(rep.max_discontinuity < 1e-3);
        // outros modos
        for mapping in [NormalMapping::ClosestVertex, NormalMapping::ClosestCorner] {
            let mut d = primitives::cube_subdivided(2.0, 3);
            transfer_normals_from_mesh(&mut d, &proxy, &all(), mapping, 1.0, None, None).unwrap();
            for f in &d.faces {
                for c in &f.corners {
                    assert!(c.normal.unwrap().dot(d.co(c.vert).normalize()) > 0.9);
                }
            }
        }
        // max_distance: nada dentro do limite -> nada escrito
        let mut far = primitives::cube(10.0);
        let w = transfer_normals_from_mesh(&mut far, &proxy, &all(), NormalMapping::ClosestSurface, 1.0, Some(0.5), None).unwrap();
        assert_eq!(w, 0);
        assert!(far.faces.iter().flat_map(|f| &f.corners).all(|c| c.normal.is_none()));
        // mix 0.5 com normal de face
        let mut half = primitives::cube(2.0);
        transfer_normals_from_mesh(&mut half, &proxy, &all(), NormalMapping::ClosestSurface, 0.5, None, None).unwrap();
        let c = &half.faces[0].corners[0];
        let fnorm = half.face_normal(0);
        let radial = half.co(c.vert).normalize();
        let n = c.normal.unwrap();
        assert!(n.dot(fnorm) > fnorm.dot(radial) - 1e-3 || n.dot(radial) > 0.5);
    }

    #[test]
    fn closest_point_matches_brute_force() {
        let m = primitives::uv_sphere(1.0, 10, 14);
        let surf = TriSurface::build(&m);
        let mut k = 7u32;
        let mut rnd = || {
            k = k.wrapping_mul(1664525).wrapping_add(1013904223);
            (k >> 8) as f32 / (1u32 << 24) as f32 * 6.0 - 3.0
        };
        for _ in 0..300 {
            let p = Vec3::new(rnd(), rnd(), rnd());
            let hit = surf.closest_point(p, None).unwrap();
            let mut best = f32::INFINITY;
            for t in &surf.tris {
                let (pt, _) = closest_point_on_triangle(p, t.p[0], t.p[1], t.p[2]);
                best = best.min(pt.distance(p));
            }
            assert!((hit.dist - best).abs() < 1e-5, "{} vs {}", hit.dist, best);
            assert!((hit.bary.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
        assert!(surf.closest_point(Vec3::new(50.0, 0.0, 0.0), Some(1.0)).is_none());
        assert!(surf.closest_point(Vec3::NAN, None).is_none());
        assert!(TriSurface::build(&EditMesh::new("e")).closest_point(Vec3::ZERO, None).is_none());
    }

    #[test]
    fn closest_point_on_degenerate_triangle() {
        let (p, b) = closest_point_on_triangle(Vec3::new(0.5, 1.0, 0.0), Vec3::ZERO, Vec3::X, Vec3::X * 2.0);
        assert!(p.is_finite() && b.iter().all(|x| x.is_finite()));
        let (p, _) = closest_point_on_triangle(Vec3::Y, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(p, Vec3::ZERO);
    }

    #[test]
    fn ray_hits_matches_brute_force() {
        let m = primitives::uv_sphere(1.0, 10, 14);
        let surf = TriSurface::build(&m);
        let mut k = 11u32;
        let mut rnd = || {
            k = k.wrapping_mul(1664525).wrapping_add(1013904223);
            (k >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for _ in 0..300 {
            let o = Vec3::new(rnd(), rnd(), rnd()) * 2.0;
            let d = Vec3::new(rnd(), rnd(), rnd()).normalize_or_zero();
            if d == Vec3::ZERO {
                continue;
            }
            let t_max = 2.5;
            let brute = surf
                .tris
                .iter()
                .any(|t| ray_triangle(o, d, t.p[0], t.p[1], t.p[2]).map(|h| h <= t_max).unwrap_or(false));
            assert_eq!(surf.ray_hits(o, d, t_max, &|_| false), brute, "o={o:?} d={d:?}");
        }
        assert!(!surf.ray_hits(Vec3::ZERO, Vec3::X, 0.0, &|_| false));
    }

    #[test]
    fn outline_normals_cube_corners_share_diagonal() {
        let m = split_cube();
        assert_eq!(m.vert_count(), 24);
        let per_corner = outline_normals_per_corner(&m, &OutlineParams::default()).unwrap();
        // todos os corners na mesma posição recebem a mesma normal (diagonal)
        let mut by_pos: HashMap<[i32; 3], Vec<Vec3>> = HashMap::new();
        for (fi, f) in m.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                let p = m.co(c.vert);
                by_pos
                    .entry([p.x as i32, p.y as i32, p.z as i32])
                    .or_default()
                    .push(per_corner[fi][ci]);
            }
        }
        assert_eq!(by_pos.len(), 8);
        for (pos, ns) in &by_pos {
            assert_eq!(ns.len(), 3);
            let want = Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32).normalize();
            for n in ns {
                assert!((*n - want).length() < 1e-5, "{n:?} vs {want:?}");
            }
        }
        // hull sem frestas: deslocar cada corner por d ao longo da normal bakeada
        // mantém corners coincidentes coincidentes
        let d = 0.1;
        let flat = bake_outline_normals(&m, &OutlineParams::default()).unwrap();
        let mut k = 0;
        let mut moved: HashMap<[i32; 3], Vec<Vec3>> = HashMap::new();
        for f in &m.faces {
            for c in &f.corners {
                let p = m.co(c.vert);
                moved
                    .entry([p.x as i32, p.y as i32, p.z as i32])
                    .or_default()
                    .push(p + flat[k] * d);
                k += 1;
            }
        }
        for ps in moved.values() {
            for p in ps {
                assert!((*p - ps[0]).length() < 1e-6);
            }
        }
        // vertex-wise: vértices soldados compartilham o mesmo valor
        let pv = outline_normals_per_vertex(&m, &OutlineParams::default()).unwrap();
        assert_eq!(pv.len(), 24);
        // com o cubo soldado (8 verts) o resultado é o mesmo
        let welded = primitives::cube(2.0);
        let pw = outline_normals_per_vertex(&welded, &OutlineParams::default()).unwrap();
        for (v, n) in pw.iter().enumerate() {
            assert!((*n - welded.co(v as u32).normalize()).length() < 1e-5);
        }
    }

    #[test]
    fn outline_ignores_sharp_and_smooth_flags_and_weightings() {
        let mut m = primitives::cube(2.0);
        for (a, b) in [(0u32, 1u32), (1, 2)] {
            m.set_sharp(a, b, true);
        }
        m.faces[0].smooth = false;
        for w in [
            NormalWeighting::Angle,
            NormalWeighting::Area,
            NormalWeighting::AngleArea,
            NormalWeighting::Uniform,
        ] {
            let p = OutlineParams {
                weighting: w,
                weld_epsilon: 0.0,
            };
            let n = outline_normals_per_vertex(&m, &p).unwrap();
            for (v, nn) in n.iter().enumerate() {
                assert!((nn.length() - 1.0).abs() < 1e-5);
                assert!(nn.dot(m.co(v as u32).normalize()) > 0.99);
            }
        }
        assert!(outline_normals_per_vertex(
            &m,
            &OutlineParams {
                weld_epsilon: -1.0,
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn outline_degenerate_inputs_are_finite() {
        // vértice solto e face degenerada
        let mut m = EditMesh::new("d");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        let c = m.add_vert(Vec3::X * 2.0); // colinear
        m.add_vert(Vec3::new(5.0, 5.0, 5.0)); // solto
        m.add_face(&[a, b, c]).unwrap();
        let n = outline_normals_per_vertex(&m, &OutlineParams::default()).unwrap();
        assert!(n.iter().all(|v| v.is_finite() && (v.length() - 1.0).abs() < 1e-5));
        let empty = EditMesh::new("e");
        assert!(outline_normals_per_vertex(&empty, &OutlineParams::default()).unwrap().is_empty());
    }

    #[test]
    fn weld_clusters_is_deterministic_and_handles_borders() {
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.999e-5, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-0.0, 0.0, 0.0),
            Vec3::new(1.0 + 0.5e-5, 1.0, 1.0),
        ];
        let r = weld_clusters(&pts, 1e-5);
        assert_eq!(r, vec![0, 0, 2, 0, 2]);
        let r0 = weld_clusters(&pts, 0.0);
        assert_eq!(r0[0], r0[3]);
        assert_ne!(r0[0], r0[1]);
        // vizinhos em células vizinhas se unem
        let cell = 1e-5;
        let p = vec![Vec3::splat(cell * 3.0 - 1e-7), Vec3::splat(cell * 3.0 + 1e-7)];
        assert_eq!(weld_clusters(&p, cell)[1], 0);
    }

    #[test]
    fn tangent_space_pack_roundtrip() {
        let m = primitives::uv_sphere(1.0, 8, 12);
        let params = OutlineParams::default();
        let packed = bake_outline_tangent_space(&m, &params).unwrap();
        let outline = outline_normals_per_corner(&m, &params).unwrap();
        let base = effective_corner_normals(&m);
        for (fi, f) in m.faces.iter().enumerate() {
            for ci in 0..f.corners.len() {
                let n = base[fi][ci];
                let t = corner_tangent(&m, fi as u32, ci as u32, n);
                assert!((t.length() - 1.0).abs() < 1e-4 && t.dot(n).abs() < 1e-3);
                let back = unpack_tangent_space(packed[fi][ci], n, t);
                assert!((back - outline[fi][ci]).length() < 1e-3);
            }
        }
        // tangente degenerada continua finita
        let e = pack_tangent_space(Vec3::Y, Vec3::Y, Vec3::Y);
        assert!(e.is_finite());
        assert!((unpack_tangent_space(e, Vec3::Y, Vec3::Y) - Vec3::Y).length() < 1e-3);
        assert_eq!(corner_tangent(&m, 9999, 0, Vec3::Y).length() > 0.99, true);
    }

    #[test]
    fn smoothness_report_basics() {
        let sphere = primitives::quad_sphere(1.0, 8);
        let r = normal_smoothness_report(&sphere);
        assert_eq!(r.edge_count, Topology::build(&sphere).edges.len());
        assert!(r.max_discontinuity < 1e-4);
        assert!(r.max_gradient < 0.3);
        assert!(r.worst_edge.is_some());
        let cube = primitives::cube(1.0);
        let rc = normal_smoothness_report(&cube);
        assert!(rc.max_discontinuity > 1.5);
        assert_eq!(rc.discontinuous_edges, 12);
        // região: um quad da esfera
        let rr = normal_smoothness_report_region(&sphere, &[0]).unwrap();
        assert!(rr.edge_count < r.edge_count && rr.edge_count > 0);
        let empty = normal_smoothness_report(&EditMesh::new("e"));
        assert_eq!(empty.edge_count, 0);
    }

    #[test]
    fn determinism_and_perf_sanity() {
        let mut a = primitives::quad_sphere(1.0, 64);
        let mut b = a.clone();
        let t = std::time::Instant::now();
        transfer_normals_from_point(&mut a, &all(), Vec3::new(0.0, 0.1, 0.0), false, &replace()).unwrap();
        transfer_normals_from_point(&mut b, &all(), Vec3::new(0.0, 0.1, 0.0), false, &replace()).unwrap();
        assert_eq!(a, b);
        let proxy = primitives::quad_sphere(1.05, 16);
        let mut dst = primitives::quad_sphere(1.0, 64);
        transfer_normals_from_mesh(&mut dst, &proxy, &all(), NormalMapping::ClosestSurface, 1.0, None, None).unwrap();
        let ol = outline_normals_per_vertex(&dst, &OutlineParams::default()).unwrap();
        assert_eq!(ol.len(), dst.vert_count());
        let _ = compute_vertex_normals(&dst);
        let _ = NormalParams::default();
        assert!(t.elapsed().as_secs() < 60);
    }
}
