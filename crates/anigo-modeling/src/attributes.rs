//! Atributos de vértice: pintura NPR (AO / shadow shift / outline width /
//! spec-rim) e pesos de skin.
//!
//! ## Canais NPR (`EVert::color`, mesmo contrato de `anigo_core::Vertex::color`)
//!
//! | canal | índice | uso |
//! |---|---|---|
//! | [`VertexChannel::AmbientOcclusion`] | R = 0 | AO estilizado (multiplica a sombra) |
//! | [`VertexChannel::ShadowShift`] | G = 1 | desloca o limiar de sombra (0.5 = neutro) |
//! | [`VertexChannel::OutlineWidth`] | B = 2 | multiplicador da espessura do contorno |
//! | [`VertexChannel::SpecRimMask`] | A = 3 | máscara de brilho/rim light |
//!
//! Todos os valores pintados são limitados a `[0, 1]`.
//!
//! ## Skin
//!
//! Cada vértice guarda até 4 influências (`joints[4]`, `weights[4]`). Um peso
//! `0` marca slot vazio (o índice de joint do slot é ignorado). As rotinas
//! mantêm os slots ordenados por peso decrescente e, quando `normalize`, a soma
//! dos pesos é 1.
//!
//! Funções que recebem posição de pincel trabalham em espaço do objeto. Em
//! qualquer `Err` a malha permanece intacta (a validação precede a mutação).

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::normals::compute_vertex_normals;
use crate::normals_tools::{PointHash, TriSurface};
use crate::topology::Topology;
use glam::Vec3;
use std::collections::HashMap;

// =============================================================================
// Tipos
// =============================================================================

/// Canal de `EVert::color` do contrato NPR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexChannel {
    /// R: oclusão ambiente estilizada.
    AmbientOcclusion,
    /// G: shift do limiar de sombra.
    ShadowShift,
    /// B: multiplicador da espessura do contorno.
    OutlineWidth,
    /// A: máscara spec/rim.
    SpecRimMask,
}

impl VertexChannel {
    /// Índice do canal em `EVert::color`.
    pub fn index(self) -> usize {
        match self {
            VertexChannel::AmbientOcclusion => 0,
            VertexChannel::ShadowShift => 1,
            VertexChannel::OutlineWidth => 2,
            VertexChannel::SpecRimMask => 3,
        }
    }
}

/// Como o valor pintado se combina com o atual `c` (com `w` = força × falloff):
///
/// * `Mix`: `c + (valor − c)·w`
/// * `Add`: `c + valor·w`
/// * `Subtract`: `c − valor·w`
/// * `Multiply`: `c · (1 + (valor − 1)·w)`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintBlend {
    Mix,
    Add,
    Subtract,
    Multiply,
}

/// Perfil de queda do pincel em função de `t = distância / raio ∈ [0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushFalloff {
    /// `1 − smoothstep(t) = 1 − (3t² − 2t³)`.
    Smooth,
    /// `1 − t`.
    Linear,
    /// `√(1 − t²)`.
    Sphere,
    /// `√(1 − t)`.
    Root,
    /// `(1 − t)²`.
    Sharp,
    /// `1` dentro do raio.
    Constant,
}

impl BrushFalloff {
    /// Peso para `t = d / r`. `t ≥ 1` dá 0 (exceto `Constant` em `t = 1` exato).
    pub fn weight(self, t: f32) -> f32 {
        if !t.is_finite() || t > 1.0 {
            return 0.0;
        }
        let t = t.max(0.0);
        match self {
            BrushFalloff::Smooth => 1.0 - t * t * (3.0 - 2.0 * t),
            BrushFalloff::Linear => 1.0 - t,
            BrushFalloff::Sphere => (1.0 - t * t).max(0.0).sqrt(),
            BrushFalloff::Root => (1.0 - t).max(0.0).sqrt(),
            BrushFalloff::Sharp => (1.0 - t) * (1.0 - t),
            BrushFalloff::Constant => 1.0,
        }
    }
}

fn blend_value(old: f32, value: f32, mode: PaintBlend, w: f32) -> f32 {
    let r = match mode {
        PaintBlend::Mix => old + (value - old) * w,
        PaintBlend::Add => old + value * w,
        PaintBlend::Subtract => old - value * w,
        PaintBlend::Multiply => old * (1.0 + (value - 1.0) * w),
    };
    r.clamp(0.0, 1.0)
}

fn check_finite(x: f32, what: &str) -> ModelResult<()> {
    if x.is_finite() {
        Ok(())
    } else {
        Err(ModelError::InvalidParameter(format!("{what} não finito")))
    }
}

fn check_verts(mesh: &EditMesh, verts: &[u32]) -> ModelResult<()> {
    for &v in verts {
        if v as usize >= mesh.vert_count() {
            return Err(ModelError::VertexOutOfRange(v, mesh.vert_count()));
        }
    }
    Ok(())
}

/// `None` = todos os vértices; `Some` = lista (validada, ordenada, sem duplicatas).
fn resolve_verts(mesh: &EditMesh, verts: Option<&[u32]>) -> ModelResult<Vec<u32>> {
    match verts {
        None => Ok((0..mesh.vert_count() as u32).collect()),
        Some(vs) => {
            check_verts(mesh, vs)?;
            let mut out = vs.to_vec();
            out.sort_unstable();
            out.dedup();
            Ok(out)
        }
    }
}

// =============================================================================
// Pintura de canais NPR
// =============================================================================

/// Pincel esférico de atributo: para cada vértice (não oculto) a menos de
/// `radius` de `center`, aplica `blend` com `w = strength · falloff(d / radius)`.
///
/// Equivale ao Vertex Paint do Blender restrito a um canal. `strength` é
/// limitado a `[0, 1]`; `radius` deve ser > 0. Devolve quantos vértices mudaram
/// de peso (dentro do raio).
#[allow(clippy::too_many_arguments)]
pub fn paint_vertex_channel(
    mesh: &mut EditMesh,
    center: Vec3,
    radius: f32,
    falloff: BrushFalloff,
    channel: VertexChannel,
    value: f32,
    blend: PaintBlend,
    strength: f32,
) -> ModelResult<usize> {
    if !center.is_finite() {
        return Err(ModelError::InvalidParameter("centro não finito".into()));
    }
    check_finite(radius, "raio")?;
    if radius <= 0.0 {
        return Err(ModelError::InvalidParameter("raio deve ser > 0".into()));
    }
    check_finite(value, "valor")?;
    check_finite(strength, "força")?;
    let strength = strength.clamp(0.0, 1.0);
    let ch = channel.index();
    let mut n = 0;
    for v in &mut mesh.verts {
        if v.hidden {
            continue;
        }
        let d = v.co.distance(center);
        if d > radius {
            continue;
        }
        let w = strength * falloff.weight(d / radius);
        if w <= 0.0 {
            continue;
        }
        v.color[ch] = blend_value(v.color[ch], value, blend, w);
        n += 1;
    }
    Ok(n)
}

/// Preenche o canal nos vértices dados (`None` = todos): mesma fórmula de
/// [`PaintBlend`] com `w = strength`. Ignora vértices ocultos.
pub fn fill_channel(
    mesh: &mut EditMesh,
    verts: Option<&[u32]>,
    channel: VertexChannel,
    value: f32,
    blend: PaintBlend,
    strength: f32,
) -> ModelResult<usize> {
    check_finite(value, "valor")?;
    check_finite(strength, "força")?;
    let ids = resolve_verts(mesh, verts)?;
    let strength = strength.clamp(0.0, 1.0);
    let ch = channel.index();
    let mut n = 0;
    for v in ids {
        let vert = &mut mesh.verts[v as usize];
        if vert.hidden {
            continue;
        }
        vert.color[ch] = blend_value(vert.color[ch], value, blend, strength);
        n += 1;
    }
    Ok(n)
}

/// Suavização laplaciana de um canal: a cada iteração
/// `c_v ← c_v + factor·(mean_{u ∈ N(v)} c_u − c_v)` (Jacobi). Vértices sem
/// vizinhos ficam como estão. `verts = None` suaviza tudo; vizinhos fora da
/// lista servem de fronteira fixa. `factor ∈ [0, 1]`.
pub fn smooth_channel(
    mesh: &mut EditMesh,
    verts: Option<&[u32]>,
    channel: VertexChannel,
    iterations: u32,
    factor: f32,
) -> ModelResult<usize> {
    check_finite(factor, "fator")?;
    if !(0.0..=1.0).contains(&factor) {
        return Err(ModelError::InvalidParameter(format!(
            "fator {factor} fora de [0, 1]"
        )));
    }
    let ids = resolve_verts(mesh, verts)?;
    let topo = Topology::build(mesh);
    let ch = channel.index();
    let mut cur: Vec<f32> = mesh.verts.iter().map(|v| v.color[ch]).collect();
    for _ in 0..iterations {
        let mut next = cur.clone();
        for &v in &ids {
            if mesh.verts[v as usize].hidden {
                continue;
            }
            let nb = topo.vert_neighbors(v);
            if nb.is_empty() {
                continue;
            }
            let mean = nb.iter().map(|&u| cur[u as usize]).sum::<f32>() / nb.len() as f32;
            let c = cur[v as usize];
            next[v as usize] = (c + factor * (mean - c)).clamp(0.0, 1.0);
        }
        cur = next;
    }
    for &v in &ids {
        mesh.verts[v as usize].color[ch] = cur[v as usize];
    }
    Ok(ids.len())
}

/// Gradiente linear: o valor varia de `v0` em `p0` a `v1` em `p1` (projeção
/// ortogonal, `t` limitado a `[0, 1]`). `p0 == p1` é erro. Sobrescreve o canal
/// (`Mix` com força 1) em `verts` (`None` = todos).
pub fn gradient_channel(
    mesh: &mut EditMesh,
    verts: Option<&[u32]>,
    p0: Vec3,
    p1: Vec3,
    v0: f32,
    v1: f32,
    channel: VertexChannel,
) -> ModelResult<usize> {
    if !p0.is_finite() || !p1.is_finite() {
        return Err(ModelError::InvalidParameter("ponto do gradiente não finito".into()));
    }
    check_finite(v0, "valor inicial")?;
    check_finite(v1, "valor final")?;
    let axis = p1 - p0;
    let len2 = axis.length_squared();
    if len2 < 1e-20 {
        return Err(ModelError::InvalidParameter("p0 e p1 coincidem".into()));
    }
    let ids = resolve_verts(mesh, verts)?;
    let ch = channel.index();
    let mut n = 0;
    for v in ids {
        let vert = &mut mesh.verts[v as usize];
        if vert.hidden {
            continue;
        }
        let t = ((vert.co - p0).dot(axis) / len2).clamp(0.0, 1.0);
        vert.color[ch] = (v0 + (v1 - v0) * t).clamp(0.0, 1.0);
        n += 1;
    }
    Ok(n)
}

/// Inverte o canal (`c ← 1 − c`) nos vértices dados (`None` = todos).
pub fn invert_channel(mesh: &mut EditMesh, verts: Option<&[u32]>, channel: VertexChannel) -> ModelResult<usize> {
    let ids = resolve_verts(mesh, verts)?;
    let ch = channel.index();
    let mut n = 0;
    for v in ids {
        let vert = &mut mesh.verts[v as usize];
        if vert.hidden {
            continue;
        }
        vert.color[ch] = (1.0 - vert.color[ch]).clamp(0.0, 1.0);
        n += 1;
    }
    Ok(n)
}

// =============================================================================
// AO por vértice
// =============================================================================

/// Parâmetros de [`bake_ao_per_vertex`].
#[derive(Debug, Clone, Copy)]
pub struct AoParams {
    /// Raios por vértice (≥ 1).
    pub samples: u32,
    /// Só oclusores a menos de `max_dist` contam.
    pub max_dist: f32,
    /// Deslocamento da origem ao longo da normal (evita auto-interseção).
    pub bias: f32,
    /// Semente da rotação por vértice (resultado determinístico).
    pub seed: u64,
    /// Mistura com o valor atual do canal R (1 = substitui).
    pub strength: f32,
}

impl Default for AoParams {
    fn default() -> Self {
        Self {
            samples: 32,
            max_dist: 1.0,
            bias: 1.0e-4,
            seed: 0x4e49474f,
            strength: 1.0,
        }
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit_float(x: u64) -> f32 {
    ((x >> 40) as f32) / (1u64 << 24) as f32
}

/// AO por vértice no canal R. Para cada vértice lança `samples` raios com
/// distribuição **cosseno** no hemisfério da normal suave do vértice
/// (`r = √u₁`, `z = √(1 − u₁)`, `φ = 2π·u₂`; `u₁` estratificado, `u₂` por razão
/// áurea com deslocamento aleatório determinístico por vértice) contra todos os
/// triângulos da malha (grade uniforme + DDA). `AO = 1 − ocluídos / samples`,
/// contando apenas acertos a menos de `max_dist`. A origem é deslocada `bias`
/// ao longo da normal; faces vizinhas contam como oclusores (cantos côncavos
/// escurecem), e como a distribuição fica acima do plano tangente, superfícies
/// planas e convexas não se auto-ocluem. Vértices sem face e vértices ocultos
/// não são tocados. Devolve quantos vértices foram escritos.
pub fn bake_ao_per_vertex(mesh: &mut EditMesh, params: &AoParams) -> ModelResult<usize> {
    if params.samples == 0 {
        return Err(ModelError::InvalidParameter("samples deve ser ≥ 1".into()));
    }
    check_finite(params.max_dist, "max_dist")?;
    if params.max_dist <= 0.0 {
        return Err(ModelError::InvalidParameter("max_dist deve ser > 0".into()));
    }
    check_finite(params.bias, "bias")?;
    if params.bias < 0.0 {
        return Err(ModelError::InvalidParameter("bias deve ser ≥ 0".into()));
    }
    check_finite(params.strength, "strength")?;
    mesh.validate()?;
    let surface = TriSurface::build(mesh);
    if surface.tris.is_empty() {
        return Err(ModelError::EmptySelection("malha sem faces para AO"));
    }
    let normals = compute_vertex_normals(mesh);
    let used = mesh.used_verts();
    let strength = params.strength.clamp(0.0, 1.0);
    let golden = 0.618_033_95_f32;
    let mut results: Vec<Option<f32>> = vec![None; mesh.vert_count()];
    for v in 0..mesh.vert_count() {
        if !used[v] || mesh.verts[v].hidden {
            continue;
        }
        let n = normals[v];
        let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let t = n.cross(helper).normalize_or_zero();
        let b = n.cross(t);
        let rnd = unit_float(splitmix(params.seed ^ (v as u64).wrapping_mul(0x9E37_79B9)));
        let origin = mesh.verts[v].co + n * params.bias;
        let mut occluded = 0u32;
        for i in 0..params.samples {
            let u1 = (i as f32 + 0.5) / params.samples as f32;
            let u2 = (i as f32 * golden + rnd).fract();
            let r = u1.sqrt();
            let z = (1.0 - u1).max(0.0).sqrt();
            let phi = std::f32::consts::TAU * u2;
            let dir = (t * (r * phi.cos()) + b * (r * phi.sin()) + n * z).normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            if surface.ray_hits(origin, dir, params.max_dist, &|_| false) {
                occluded += 1;
            }
        }
        results[v] = Some(1.0 - occluded as f32 / params.samples as f32);
    }
    let mut n = 0;
    for (v, r) in results.into_iter().enumerate() {
        if let Some(ao) = r {
            let c = &mut mesh.verts[v].color[0];
            *c = (*c + (ao - *c) * strength).clamp(0.0, 1.0);
            n += 1;
        }
    }
    Ok(n)
}

// =============================================================================
// Pesos de skin
// =============================================================================

/// Influência `(joint, peso)`.
pub type Influence = (u16, f32);

/// Influências ativas (peso > 0) de um vértice, na ordem dos slots.
pub fn influences(v: &crate::mesh::EVert) -> Vec<Influence> {
    (0..4)
        .filter(|&i| v.weights[i] > 0.0 && v.weights[i].is_finite())
        .map(|i| (v.joints[i], v.weights[i]))
        .collect()
}

/// Funde duplicatas de joint, ordena por peso decrescente (empate: joint
/// crescente), mantém no máximo `max` e, se `normalize` e a soma > 0, reescala
/// para soma 1. Pesos não finitos ou ≤ 0 são descartados.
fn clean_influences(list: &[Influence], max: usize, normalize: bool) -> Vec<Influence> {
    let mut merged: Vec<Influence> = Vec::with_capacity(list.len());
    for &(j, w) in list {
        if !w.is_finite() || w <= 0.0 {
            continue;
        }
        match merged.iter_mut().find(|(jj, _)| *jj == j) {
            Some(e) => e.1 += w,
            None => merged.push((j, w)),
        }
    }
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    merged.truncate(max.clamp(1, 4));
    if normalize {
        let sum: f32 = merged.iter().map(|x| x.1).sum();
        if sum > 0.0 {
            for x in &mut merged {
                x.1 /= sum;
            }
        }
    }
    merged
}

/// Escreve uma lista de influências nos slots do vértice (restante zerado).
pub fn set_influences(v: &mut crate::mesh::EVert, list: &[Influence]) {
    let mut joints = [0u16; 4];
    let mut weights = [0.0f32; 4];
    for (i, &(j, w)) in list.iter().take(4).enumerate() {
        joints[i] = j;
        weights[i] = w;
    }
    v.joints = joints;
    v.weights = weights;
}

/// Define o peso de `joint` num vértice conforme a regra de [`assign_weight`].
fn assign_weight_inner(v: &mut crate::mesh::EVert, joint: u16, weight: f32, normalize: bool) {
    let weight = weight.clamp(0.0, 1.0);
    let mut list = influences(v);
    list.retain(|&(j, _)| j != joint);
    if weight > 0.0 {
        if list.len() >= 4 {
            // slots cheios: substitui a menor influência se a nova for maior
            let mut mi = 0;
            for (i, x) in list.iter().enumerate() {
                if x.1 < list[mi].1 {
                    mi = i;
                }
            }
            if weight > list[mi].1 {
                list.remove(mi);
            } else {
                return;
            }
        }
        list.push((joint, weight));
    }
    let out = if normalize {
        let others: f32 = list.iter().filter(|x| x.0 != joint).map(|x| x.1).sum();
        if weight > 0.0 && others > 0.0 {
            let scale = (1.0 - weight) / others;
            let l: Vec<Influence> = list
                .iter()
                .map(|&(j, w)| if j == joint { (j, w) } else { (j, w * scale) })
                .collect();
            clean_influences(&l, 4, false)
        } else {
            clean_influences(&list, 4, true)
        }
    } else {
        clean_influences(&list, 4, false)
    };
    set_influences(v, &out);
}

/// Atribui `weight` ao `joint` do vértice.
///
/// * `weight` é limitado a `[0, 1]`; `0` remove a influência.
/// * Se o joint não está entre as 4 influências e todos os slots estão cheios,
///   substitui a menor influência — se a nova for maior; senão nada muda.
/// * `normalize`: as **demais** influências são reescaladas para somar
///   `1 − weight` (se não há outras, o peso vira 1). Sem `normalize` os outros
///   pesos ficam intactos.
pub fn assign_weight(
    mesh: &mut EditMesh,
    vert: u32,
    joint: u16,
    weight: f32,
    normalize: bool,
) -> ModelResult<()> {
    check_finite(weight, "peso")?;
    if vert as usize >= mesh.vert_count() {
        return Err(ModelError::VertexOutOfRange(vert, mesh.vert_count()));
    }
    assign_weight_inner(&mut mesh.verts[vert as usize], joint, weight, normalize);
    Ok(())
}

/// Pincel de peso de skin para um `joint`: mesma fórmula de [`PaintBlend`] e
/// falloff de [`paint_vertex_channel`] aplicada ao peso atual do joint. Devolve
/// quantos vértices foram tocados.
#[allow(clippy::too_many_arguments)]
pub fn paint_weight(
    mesh: &mut EditMesh,
    center: Vec3,
    radius: f32,
    falloff: BrushFalloff,
    joint: u16,
    value: f32,
    blend: PaintBlend,
    strength: f32,
    normalize: bool,
) -> ModelResult<usize> {
    if !center.is_finite() {
        return Err(ModelError::InvalidParameter("centro não finito".into()));
    }
    check_finite(radius, "raio")?;
    if radius <= 0.0 {
        return Err(ModelError::InvalidParameter("raio deve ser > 0".into()));
    }
    check_finite(value, "valor")?;
    check_finite(strength, "força")?;
    let strength = strength.clamp(0.0, 1.0);
    let mut n = 0;
    for v in &mut mesh.verts {
        if v.hidden {
            continue;
        }
        let d = v.co.distance(center);
        if d > radius {
            continue;
        }
        let w = strength * falloff.weight(d / radius);
        if w <= 0.0 {
            continue;
        }
        let cur = influences(v)
            .iter()
            .find(|x| x.0 == joint)
            .map(|x| x.1)
            .unwrap_or(0.0);
        let new = blend_value(cur, value, blend, w);
        assign_weight_inner(v, joint, new, normalize);
        n += 1;
    }
    Ok(n)
}

/// Normaliza todos os vértices (soma dos pesos = 1, slots ordenados). Pesos
/// negativos/não finitos viram 0. Vértice sem nenhum peso fica com
/// `weights = [1, 0, 0, 0]` mantendo `joints[0]` (convenção do `EVert::new`).
pub fn normalize_all(mesh: &mut EditMesh) {
    for v in &mut mesh.verts {
        let list = clean_influences(&influences(v), 4, true);
        if list.is_empty() {
            v.weights = [1.0, 0.0, 0.0, 0.0];
        } else {
            set_influences(v, &list);
        }
    }
}

/// Limita o número de influências a `max` (1..=4) mantendo as maiores e
/// renormalizando. Devolve quantos vértices mudaram.
pub fn limit_influences(mesh: &mut EditMesh, max: usize) -> ModelResult<usize> {
    if !(1..=4).contains(&max) {
        return Err(ModelError::InvalidParameter(format!(
            "max de influências {max} fora de 1..=4"
        )));
    }
    let mut changed = 0;
    for v in &mut mesh.verts {
        let old = influences(v);
        let list = clean_influences(&old, max, true);
        if list.is_empty() {
            continue;
        }
        let same = old.len() == list.len()
            && old
                .iter()
                .zip(&list)
                .all(|(a, b)| a.0 == b.0 && (a.1 - b.1).abs() < 1e-6);
        if !same {
            set_influences(v, &list);
            changed += 1;
        }
    }
    Ok(changed)
}

/// Suaviza os pesos: a cada iteração, para cada vértice `v` da lista,
/// `w_v ← (1 − f)·w_v + f·mean_{u ∈ N(v)} w_u` aplicado a **todos** os joints
/// (mapas esparsos), depois top-4 e normalização. Vizinhos fora da lista são
/// fonte fixa. `factor ∈ [0, 1]`.
pub fn smooth_weights(mesh: &mut EditMesh, verts: &[u32], iterations: u32, factor: f32) -> ModelResult<usize> {
    check_finite(factor, "fator")?;
    if !(0.0..=1.0).contains(&factor) {
        return Err(ModelError::InvalidParameter(format!(
            "fator {factor} fora de [0, 1]"
        )));
    }
    check_verts(mesh, verts)?;
    if verts.is_empty() {
        return Err(ModelError::EmptySelection("nenhum vértice para suavizar pesos"));
    }
    let mut ids = verts.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let topo = Topology::build(mesh);
    let mut cur: Vec<Vec<Influence>> = mesh.verts.iter().map(influences).collect();
    for _ in 0..iterations {
        let mut next = cur.clone();
        for &v in &ids {
            let nb = topo.vert_neighbors(v);
            if nb.is_empty() {
                continue;
            }
            let mut acc: HashMap<u16, f32> = HashMap::new();
            for &(j, w) in &cur[v as usize] {
                *acc.entry(j).or_insert(0.0) += w * (1.0 - factor);
            }
            let share = factor / nb.len() as f32;
            for &u in &nb {
                for &(j, w) in &cur[u as usize] {
                    *acc.entry(j).or_insert(0.0) += w * share;
                }
            }
            let mut list: Vec<Influence> = acc.into_iter().collect();
            list.sort_by_key(|x| x.0);
            next[v as usize] = clean_influences(&list, 4, true);
        }
        cur = next;
    }
    for &v in &ids {
        let list = &cur[v as usize];
        if !list.is_empty() {
            set_influences(&mut mesh.verts[v as usize], list);
        }
    }
    Ok(ids.len())
}

/// Transfere pesos de `src` (corpo) para `dst` (roupa): para cada vértice de
/// `dst`, ponto mais próximo na superfície de `src` e interpolação baricêntrica
/// das influências dos 3 vértices do triângulo (mapas esparsos, top-4,
/// normalizado). Vértices a mais de `max_distance` (se dado) não mudam.
/// Devolve quantos vértices receberam pesos.
pub fn transfer_weights(dst: &mut EditMesh, src: &EditMesh, max_distance: Option<f32>) -> ModelResult<usize> {
    if let Some(d) = max_distance {
        if !d.is_finite() || d <= 0.0 {
            return Err(ModelError::InvalidParameter("max_distance inválido".into()));
        }
    }
    let surface = TriSurface::build(src);
    if surface.tris.is_empty() {
        return Err(ModelError::EmptySelection("mesh de origem sem triângulos"));
    }
    let mut updates: Vec<(usize, Vec<Influence>)> = Vec::new();
    for (i, v) in dst.verts.iter().enumerate() {
        let Some(hit) = surface.closest_point(v.co, max_distance) else {
            continue;
        };
        let tri = &surface.tris[hit.tri];
        let mut acc: HashMap<u16, f32> = HashMap::new();
        for k in 0..3 {
            for (j, w) in influences(&src.verts[tri.verts[k] as usize]) {
                *acc.entry(j).or_insert(0.0) += w * hit.bary[k];
            }
        }
        let mut list: Vec<Influence> = acc.into_iter().collect();
        list.sort_by_key(|x| x.0);
        let list = clean_influences(&list, 4, true);
        if !list.is_empty() {
            updates.push((i, list));
        }
    }
    let n = updates.len();
    for (i, list) in updates {
        set_influences(&mut dst.verts[i], &list);
    }
    Ok(n)
}

/// Osso para o auto-peso por envelope: cápsula de `head` a `tail` com raio
/// linear `radius_head → radius_tail`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneEnvelope {
    pub joint: u16,
    pub head: Vec3,
    pub tail: Vec3,
    pub radius_head: f32,
    pub radius_tail: f32,
}

/// Auto-peso por envelope (cápsulas). Para cada vértice e osso: `q` = ponto mais
/// próximo no segmento `head–tail` (parâmetro `s` limitado a `[0, 1]`), `r =
/// lerp(radius_head, radius_tail, s)`, `d = |p − q|`. O peso bruto é
///
/// * `1` se `d ≤ r` (dentro da cápsula);
/// * `smoothstep(1 − (d − r) / (r·falloff))` se `r < d < r·(1 + falloff)`;
/// * `0` fora (e sempre `0` se `falloff = 0` fora da cápsula).
///
/// Os pesos brutos são normalizados (top-4). Se nenhum osso alcança o vértice,
/// ele recebe peso 1 no osso de menor `d − r`. Vértices ocultos e sem face são
/// também processados (pesos dependem só da posição). Devolve o número de
/// vértices escritos (= todos). Erros: lista vazia, raios ≤ 0, `falloff` < 0,
/// pontos não finitos, joints duplicados.
pub fn auto_weights_envelope(mesh: &mut EditMesh, bones: &[BoneEnvelope], falloff: f32) -> ModelResult<usize> {
    if bones.is_empty() {
        return Err(ModelError::EmptySelection("nenhum osso para o envelope"));
    }
    check_finite(falloff, "falloff")?;
    if falloff < 0.0 {
        return Err(ModelError::InvalidParameter("falloff deve ser ≥ 0".into()));
    }
    for (i, b) in bones.iter().enumerate() {
        if !b.head.is_finite() || !b.tail.is_finite() {
            return Err(ModelError::InvalidParameter(format!("osso {i} com posição não finita")));
        }
        if !(b.radius_head.is_finite() && b.radius_tail.is_finite())
            || b.radius_head <= 0.0
            || b.radius_tail <= 0.0
        {
            return Err(ModelError::InvalidParameter(format!("osso {i} com raio ≤ 0")));
        }
        if bones[..i].iter().any(|o| o.joint == b.joint) {
            return Err(ModelError::InvalidParameter(format!("joint {} duplicado", b.joint)));
        }
    }
    for v in &mut mesh.verts {
        let p = v.co;
        let mut raw: Vec<Influence> = Vec::new();
        let mut nearest = (f32::INFINITY, bones[0].joint);
        for b in bones {
            let seg = b.tail - b.head;
            let len2 = seg.length_squared();
            let s = if len2 > 1e-20 {
                ((p - b.head).dot(seg) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = b.head + seg * s;
            let r = b.radius_head + (b.radius_tail - b.radius_head) * s;
            let d = p.distance(q);
            if d - r < nearest.0 {
                nearest = (d - r, b.joint);
            }
            let w = if d <= r {
                1.0
            } else if falloff > 0.0 && d < r * (1.0 + falloff) {
                let t = 1.0 - (d - r) / (r * falloff);
                t * t * (3.0 - 2.0 * t)
            } else {
                0.0
            };
            if w > 0.0 {
                raw.push((b.joint, w));
            }
        }
        let list = if raw.is_empty() {
            vec![(nearest.1, 1.0)]
        } else {
            clean_influences(&raw, 4, true)
        };
        set_influences(v, &list);
    }
    Ok(mesh.vert_count())
}

/// Eixo de espelhamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorAxis {
    X,
    Y,
    Z,
}

impl MirrorAxis {
    fn index(self) -> usize {
        match self {
            MirrorAxis::X => 0,
            MirrorAxis::Y => 1,
            MirrorAxis::Z => 2,
        }
    }
    fn reflect(self, p: Vec3) -> Vec3 {
        let mut q = p;
        q[self.index()] = -q[self.index()];
        q
    }
}

/// Espelha pesos de skin do lado `from_positive` (coordenada do eixo > tol)
/// para o lado oposto: o vértice parceiro (posição refletida, distância ≤
/// `tolerance`, o mais próximo) recebe as influências do original com os joints
/// traduzidos por `joint_map` (`(origem, destino)`; joints ausentes do mapa
/// ficam iguais, p.ex. coluna). Vértices sobre o plano (|coord| ≤ tol) não
/// mudam. Devolve quantos parceiros foram escritos.
pub fn mirror_weights(
    mesh: &mut EditMesh,
    axis: MirrorAxis,
    from_positive: bool,
    joint_map: &[(u16, u16)],
    tolerance: f32,
) -> ModelResult<usize> {
    check_finite(tolerance, "tolerância")?;
    if tolerance <= 0.0 {
        return Err(ModelError::InvalidParameter("tolerância deve ser > 0".into()));
    }
    let map: HashMap<u16, u16> = joint_map.iter().copied().collect();
    let ai = axis.index();
    let hash = PointHash::new(
        mesh.verts.iter().enumerate().map(|(i, v)| (i as u32, v.co)),
        tolerance * 2.0,
    );
    let mut writes: Vec<(usize, Vec<Influence>)> = Vec::new();
    for (i, v) in mesh.verts.iter().enumerate() {
        let c = v.co[ai];
        let on_source = if from_positive { c > tolerance } else { c < -tolerance };
        if !on_source {
            continue;
        }
        let target = axis.reflect(v.co);
        let mut best: Option<(f32, u32)> = None;
        for j in hash.candidates(target) {
            if j as usize == i {
                continue;
            }
            let d = mesh.verts[j as usize].co.distance(target);
            if d <= tolerance && best.map(|b| d < b.0 || (d == b.0 && j < b.1)).unwrap_or(true) {
                best = Some((d, j));
            }
        }
        let Some((_, partner)) = best else {
            continue;
        };
        let list: Vec<Influence> = influences(v)
            .into_iter()
            .map(|(j, w)| (map.get(&j).copied().unwrap_or(j), w))
            .collect();
        writes.push((partner as usize, clean_influences(&list, 4, false)));
    }
    let n = writes.len();
    for (p, list) in writes {
        set_influences(&mut mesh.verts[p], &list);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn sphere() -> EditMesh {
        primitives::quad_sphere(1.0, 6)
    }

    #[test]
    fn falloff_profiles() {
        for f in [
            BrushFalloff::Smooth,
            BrushFalloff::Linear,
            BrushFalloff::Sphere,
            BrushFalloff::Root,
            BrushFalloff::Sharp,
            BrushFalloff::Constant,
        ] {
            assert!((f.weight(0.0) - 1.0).abs() < 1e-6, "{f:?}");
            assert_eq!(f.weight(1.5), 0.0);
            assert_eq!(f.weight(f32::NAN), 0.0);
            let mut prev = 2.0;
            for i in 0..=20 {
                let w = f.weight(i as f32 / 20.0);
                assert!(w <= prev + 1e-6 && (0.0..=1.0).contains(&w));
                prev = w;
            }
        }
        assert!(BrushFalloff::Linear.weight(1.0).abs() < 1e-6);
        assert!((BrushFalloff::Linear.weight(0.25) - 0.75).abs() < 1e-6);
        assert!((BrushFalloff::Smooth.weight(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(BrushFalloff::Constant.weight(1.0), 1.0);
        assert_eq!(VertexChannel::SpecRimMask.index(), 3);
    }

    #[test]
    fn paint_blend_modes() {
        for (mode, value, expect) in [
            (PaintBlend::Mix, 1.0, 0.5 + 0.5 * 0.5),
            (PaintBlend::Add, 0.2, 0.7),
            (PaintBlend::Subtract, 0.2, 0.3),
            (PaintBlend::Multiply, 0.0, 0.25),
        ] {
            let mut m = sphere();
            for v in &mut m.verts {
                v.color = [0.5; 4];
            }
            let w = if mode == PaintBlend::Mix || mode == PaintBlend::Multiply { 0.5 } else { 1.0 };
            let n = paint_vertex_channel(
                &mut m,
                Vec3::ZERO,
                10.0,
                BrushFalloff::Constant,
                VertexChannel::OutlineWidth,
                value,
                mode,
                w,
            )
            .unwrap();
            assert_eq!(n, m.vert_count());
            let c = m.verts[0].color;
            let want = if mode == PaintBlend::Multiply { 0.5 * (1.0 + (0.0 - 1.0) * 0.5) } else { expect };
            assert!((c[2] - want).abs() < 1e-5, "{mode:?}: {} vs {}", c[2], want);
            assert_eq!(c[0], 0.5); // outros canais intactos
            assert_eq!(c[3], 0.5);
        }
    }

    #[test]
    fn paint_respects_radius_hidden_and_clamps() {
        let mut m = sphere();
        let top = Vec3::new(0.0, 1.0, 0.0);
        let h = m.verts.iter().position(|v| v.co.distance(top) < 0.3).unwrap();
        m.verts[h].hidden = true;
        let n = paint_vertex_channel(&mut m, top, 0.5, BrushFalloff::Linear, VertexChannel::ShadowShift, 1.0, PaintBlend::Add, 1.0).unwrap();
        assert!(n > 0 && n < m.vert_count());
        assert_eq!(m.verts[h].color[1], 0.5); // oculto intocado
        for v in &m.verts {
            assert!(v.color.iter().all(|c| (0.0..=1.0).contains(c)));
            if v.co.distance(top) > 0.5 {
                assert_eq!(v.color[1], 0.5);
            }
        }
        // força > 1 é limitada
        let mut m2 = sphere();
        paint_vertex_channel(&mut m2, Vec3::ZERO, 5.0, BrushFalloff::Constant, VertexChannel::AmbientOcclusion, 0.0, PaintBlend::Mix, 5.0).unwrap();
        assert!(m2.verts.iter().all(|v| v.color[0] == 0.0));
    }

    #[test]
    fn paint_errors_leave_mesh_intact() {
        let mut m = sphere();
        let before = m.clone();
        let c = Vec3::ZERO;
        let p = |m: &mut EditMesh, c, r, v, s| {
            paint_vertex_channel(m, c, r, BrushFalloff::Smooth, VertexChannel::SpecRimMask, v, PaintBlend::Mix, s)
        };
        assert!(p(&mut m, c, 0.0, 1.0, 1.0).is_err());
        assert!(p(&mut m, c, -1.0, 1.0, 1.0).is_err());
        assert!(p(&mut m, c, f32::NAN, 1.0, 1.0).is_err());
        assert!(p(&mut m, Vec3::NAN, 1.0, 1.0, 1.0).is_err());
        assert!(p(&mut m, c, 1.0, f32::INFINITY, 1.0).is_err());
        assert!(p(&mut m, c, 1.0, 1.0, f32::NAN).is_err());
        assert!(fill_channel(&mut m, Some(&[9999]), VertexChannel::SpecRimMask, 1.0, PaintBlend::Mix, 1.0).is_err());
        assert!(fill_channel(&mut m, None, VertexChannel::SpecRimMask, f32::NAN, PaintBlend::Mix, 1.0).is_err());
        assert!(smooth_channel(&mut m, None, VertexChannel::SpecRimMask, 1, 2.0).is_err());
        assert!(gradient_channel(&mut m, None, Vec3::ONE, Vec3::ONE, 0.0, 1.0, VertexChannel::SpecRimMask).is_err());
        assert!(gradient_channel(&mut m, None, Vec3::NAN, Vec3::ONE, 0.0, 1.0, VertexChannel::SpecRimMask).is_err());
        assert!(invert_channel(&mut m, Some(&[9999]), VertexChannel::SpecRimMask).is_err());
        assert!(paint_weight(&mut m, c, 0.0, BrushFalloff::Smooth, 1, 1.0, PaintBlend::Mix, 1.0, true).is_err());
        assert!(assign_weight(&mut m, 9999, 0, 1.0, true).is_err());
        assert!(assign_weight(&mut m, 0, 0, f32::NAN, true).is_err());
        assert!(limit_influences(&mut m, 0).is_err());
        assert!(limit_influences(&mut m, 5).is_err());
        assert!(smooth_weights(&mut m, &[], 1, 0.5).is_err());
        assert!(smooth_weights(&mut m, &[9999], 1, 0.5).is_err());
        assert!(smooth_weights(&mut m, &[0], 1, -0.5).is_err());
        assert!(transfer_weights(&mut m, &EditMesh::new("e"), None).is_err());
        assert!(transfer_weights(&mut m, &sphere(), Some(0.0)).is_err());
        assert!(auto_weights_envelope(&mut m, &[], 0.5).is_err());
        let bone = BoneEnvelope { joint: 0, head: Vec3::ZERO, tail: Vec3::Y, radius_head: 1.0, radius_tail: 1.0 };
        assert!(auto_weights_envelope(&mut m, &[BoneEnvelope { radius_head: 0.0, ..bone }], 0.5).is_err());
        assert!(auto_weights_envelope(&mut m, &[BoneEnvelope { head: Vec3::NAN, ..bone }], 0.5).is_err());
        assert!(auto_weights_envelope(&mut m, &[bone, bone], 0.5).is_err());
        assert!(auto_weights_envelope(&mut m, &[bone], -1.0).is_err());
        assert!(mirror_weights(&mut m, MirrorAxis::X, true, &[], 0.0).is_err());
        let mut ao = AoParams::default();
        ao.samples = 0;
        assert!(bake_ao_per_vertex(&mut m, &ao).is_err());
        ao = AoParams { max_dist: 0.0, ..Default::default() };
        assert!(bake_ao_per_vertex(&mut m, &ao).is_err());
        ao = AoParams { bias: -1.0, ..Default::default() };
        assert!(bake_ao_per_vertex(&mut m, &ao).is_err());
        assert!(bake_ao_per_vertex(&mut EditMesh::new("e"), &AoParams::default()).is_err());
        assert_eq!(m, before);
    }

    #[test]
    fn fill_gradient_invert() {
        let mut m = sphere();
        fill_channel(&mut m, Some(&[0, 1, 1]), VertexChannel::ShadowShift, 0.9, PaintBlend::Mix, 1.0).unwrap();
        assert_eq!(m.verts[0].color[1], 0.9);
        assert_eq!(m.verts[1].color[1], 0.9);
        assert_eq!(m.verts[2].color[1], 0.5);
        invert_channel(&mut m, Some(&[0]), VertexChannel::ShadowShift).unwrap();
        assert!((m.verts[0].color[1] - 0.1).abs() < 1e-6);
        invert_channel(&mut m, None, VertexChannel::AmbientOcclusion).unwrap();
        assert!(m.verts.iter().all(|v| v.color[0] == 0.0));
        // gradiente de baixo (y=-1) para cima (y=1)
        gradient_channel(&mut m, None, Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 0.0, 1.0, VertexChannel::OutlineWidth).unwrap();
        for v in &m.verts {
            assert!((v.color[2] - (v.co.y + 1.0) * 0.5).abs() < 1e-5);
        }
        // fora do segmento: saturado
        gradient_channel(&mut m, None, Vec3::ZERO, Vec3::Y * 0.1, 0.2, 0.8, VertexChannel::SpecRimMask).unwrap();
        assert!(m.verts.iter().all(|v| (0.2..=0.8).contains(&v.color[3])));
    }

    #[test]
    fn smooth_channel_reduces_variance_and_preserves_mean_range() {
        let mut m = sphere();
        for (i, v) in m.verts.iter_mut().enumerate() {
            v.color[0] = if i % 2 == 0 { 0.0 } else { 1.0 };
        }
        let var = |m: &EditMesh| {
            let mean = m.verts.iter().map(|v| v.color[0]).sum::<f32>() / m.vert_count() as f32;
            m.verts.iter().map(|v| (v.color[0] - mean).powi(2)).sum::<f32>() / m.vert_count() as f32
        };
        let v0 = var(&m);
        smooth_channel(&mut m, None, VertexChannel::AmbientOcclusion, 4, 0.7).unwrap();
        assert!(var(&m) < v0 * 0.5);
        assert!(m.verts.iter().all(|v| (0.0..=1.0).contains(&v.color[0])));
        // subconjunto: fora da lista não muda
        let mut m = sphere();
        for (i, v) in m.verts.iter_mut().enumerate() {
            v.color[1] = (i % 3) as f32 / 2.0;
        }
        let keep = m.verts[5].color[1];
        smooth_channel(&mut m, Some(&[0, 1, 2]), VertexChannel::ShadowShift, 2, 1.0).unwrap();
        assert_eq!(m.verts[5].color[1], keep);
        // iterações 0: sem mudança
        let snap = m.clone();
        smooth_channel(&mut m, None, VertexChannel::ShadowShift, 0, 1.0).unwrap();
        assert_eq!(m, snap);
    }

    #[test]
    fn ao_convex_sphere_is_open_and_inverted_sphere_is_occluded() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let n = bake_ao_per_vertex(&mut m, &AoParams { samples: 24, max_dist: 3.0, ..Default::default() }).unwrap();
        assert_eq!(n, m.vert_count());
        for v in &m.verts {
            assert!(v.color[0] > 0.99, "AO {} em esfera convexa", v.color[0]);
        }
        // esfera invertida (normais para dentro): hemisfério aponta para o interior
        let mut inv = primitives::quad_sphere(1.0, 4);
        for f in 0..inv.face_count() as u32 {
            inv.flip_face(f).unwrap();
        }
        bake_ao_per_vertex(&mut inv, &AoParams { samples: 24, max_dist: 3.0, ..Default::default() }).unwrap();
        for v in &inv.verts {
            assert!(v.color[0] < 0.1, "AO {} dentro da esfera", v.color[0]);
        }
        // max_dist curto: rays não alcançam a parede oposta
        let mut inv2 = inv.clone();
        for v in &mut inv2.verts {
            v.color[0] = 1.0;
        }
        bake_ao_per_vertex(&mut inv2, &AoParams { samples: 24, max_dist: 0.05, ..Default::default() }).unwrap();
        assert!(inv2.verts.iter().all(|v| v.color[0] > 0.9));
    }

    #[test]
    fn ao_crease_is_darker_than_flat_and_is_deterministic() {
        // duas placas em L: vértice na dobra é mais ocluído do que longe dela
        let mut m = EditMesh::new("L");
        let n = 6u32;
        let mut ids = Vec::new();
        // chão y=0, x in [0,2]; parede x=0, y in [0,2]
        for i in 0..=n {
            for j in 0..=n {
                ids.push(m.add_vert(Vec3::new(i as f32 / n as f32 * 2.0, 0.0, j as f32 / n as f32 * 2.0)));
            }
        }
        let f_idx = |i: u32, j: u32| i * (n + 1) + j;
        for i in 0..n {
            for j in 0..n {
                m.add_face(&[f_idx(i, j), f_idx(i, j + 1), f_idx(i + 1, j + 1), f_idx(i + 1, j)]).unwrap();
            }
        }
        let base = m.vert_count() as u32;
        for k in 1..=n {
            for j in 0..=n {
                m.add_vert(Vec3::new(0.0, k as f32 / n as f32 * 2.0, j as f32 / n as f32 * 2.0));
            }
        }
        let w_idx = |k: u32, j: u32| if k == 0 { f_idx(0, j) } else { base + (k - 1) * (n + 1) + j };
        for k in 0..n {
            for j in 0..n {
                m.add_face(&[w_idx(k, j), w_idx(k + 1, j), w_idx(k + 1, j + 1), w_idx(k, j + 1)]).unwrap();
            }
        }
        // orienta o chão para +Y e a parede para +X
        for f in 0..m.face_count() as u32 {
            let nrm = m.face_normal(f);
            let want = if m.face_center(f).y < 1e-6 { Vec3::Y } else { Vec3::X };
            if nrm.dot(want) < 0.0 {
                m.flip_face(f).unwrap();
            }
        }
        let p = AoParams { samples: 64, max_dist: 2.0, ..Default::default() };
        let mut a = m.clone();
        let mut b = m.clone();
        bake_ao_per_vertex(&mut a, &p).unwrap();
        bake_ao_per_vertex(&mut b, &p).unwrap();
        assert_eq!(a, b);
        let at = |m: &EditMesh, x: f32, y: f32| {
            m.verts.iter().find(|v| (v.co.x - x).abs() < 1e-4 && (v.co.y - y).abs() < 1e-4 && (v.co.z - 1.0).abs() < 0.2).unwrap().color[0]
        };
        let crease = at(&a, 0.0, 0.0);
        let far = at(&a, 2.0, 0.0);
        assert!(crease < far - 0.05, "crease {crease} far {far}");
        let weak = AoParams { strength: 0.0, ..p };
        let mut c = m.clone();
        bake_ao_per_vertex(&mut c, &weak).unwrap();
        assert_eq!(c.verts, m.verts);
    }

    #[test]
    fn assign_weight_rules() {
        let mut m = sphere();
        m.verts[0].joints = [1, 2, 0, 0];
        m.verts[0].weights = [0.6, 0.4, 0.0, 0.0];
        // normalize: outros reescalados para 1 - w
        assign_weight(&mut m, 0, 3, 0.5, true).unwrap();
        let inf = influences(&m.verts[0]);
        let sum: f32 = inf.iter().map(|x| x.1).sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!((inf.iter().find(|x| x.0 == 3).unwrap().1 - 0.5).abs() < 1e-5);
        assert!((inf.iter().find(|x| x.0 == 1).unwrap().1 - 0.3).abs() < 1e-5);
        // sem normalize: demais intactos
        assign_weight(&mut m, 0, 9, 0.2, false).unwrap();
        let inf = influences(&m.verts[0]);
        assert!((inf.iter().find(|x| x.0 == 1).unwrap().1 - 0.3).abs() < 1e-5);
        assert!(inf.iter().find(|x| x.0 == 9).is_some());
        // slots cheios: substitui o menor se maior, senão ignora
        assert_eq!(influences(&m.verts[0]).len(), 4);
        let before = m.verts[0].clone();
        assign_weight(&mut m, 0, 100, 0.01, false).unwrap();
        assert_eq!(influences(&m.verts[0]), influences(&before));
        assign_weight(&mut m, 0, 100, 0.4, true).unwrap();
        assert!(influences(&m.verts[0]).iter().any(|x| x.0 == 100));
        assert_eq!(influences(&m.verts[0]).len(), 4);
        let sum: f32 = influences(&m.verts[0]).iter().map(|x| x.1).sum();
        assert!((sum - 1.0).abs() < 1e-5);
        // peso 0 remove
        assign_weight(&mut m, 0, 100, 0.0, true).unwrap();
        assert!(!influences(&m.verts[0]).iter().any(|x| x.0 == 100));
        // sozinho com normalize vira 1
        let mut s = sphere();
        s.verts[1].weights = [0.0; 4];
        assign_weight(&mut s, 1, 7, 0.3, true).unwrap();
        assert_eq!(influences(&s.verts[1]), vec![(7, 1.0)]);
        // peso > 1 limitado
        assign_weight(&mut s, 1, 7, 5.0, false).unwrap();
        assert_eq!(influences(&s.verts[1]), vec![(7, 1.0)]);
    }

    #[test]
    fn normalize_and_limit() {
        let mut m = sphere();
        m.verts[0].joints = [1, 2, 3, 4];
        m.verts[0].weights = [0.1, 0.4, 0.2, 0.3];
        m.verts[1].weights = [0.0; 4];
        m.verts[2].weights = [f32::NAN, -1.0, 0.0, 0.0];
        m.verts[3].joints = [5, 5, 6, 0];
        m.verts[3].weights = [0.2, 0.2, 0.2, 0.0];
        normalize_all(&mut m);
        for v in &m.verts {
            let s: f32 = v.weights.iter().sum();
            assert!((s - 1.0).abs() < 1e-5);
            assert!(v.weights.windows(2).all(|w| w[0] >= w[1]));
        }
        assert_eq!(m.verts[0].joints[0], 2);
        assert_eq!(m.verts[1].weights, [1.0, 0.0, 0.0, 0.0]);
        // joints duplicados fundidos: 5 -> 0.4, 6 -> 0.2 => 2/3 e 1/3
        assert_eq!(influences(&m.verts[3]).len(), 2);
        assert!((m.verts[3].weights[0] - 2.0 / 3.0).abs() < 1e-5);
        let changed = limit_influences(&mut m, 2).unwrap();
        assert!(changed >= 1);
        let inf = influences(&m.verts[0]);
        assert_eq!(inf.len(), 2);
        assert_eq!((inf[0].0, inf[1].0), (2, 4));
        assert!((inf[0].1 - 0.4 / 0.7).abs() < 1e-5 && (inf[1].1 - 0.3 / 0.7).abs() < 1e-5);
        let s: f32 = m.verts[0].weights.iter().sum();
        assert!((s - 1.0).abs() < 1e-5);
        assert_eq!(limit_influences(&mut m, 4).unwrap(), 0);
    }

    #[test]
    fn paint_weight_with_normalization() {
        let mut m = sphere();
        for v in &mut m.verts {
            v.joints = [0, 1, 0, 0];
            v.weights = [1.0, 0.0, 0.0, 0.0];
        }
        let top = Vec3::Y;
        paint_weight(&mut m, top, 0.8, BrushFalloff::Linear, 1, 1.0, PaintBlend::Add, 1.0, true).unwrap();
        let near = m.verts.iter().min_by(|a, b| a.co.distance(top).partial_cmp(&b.co.distance(top)).unwrap()).unwrap();
        let w1 = influences(near).iter().find(|x| x.0 == 1).map(|x| x.1).unwrap_or(0.0);
        assert!(w1 > 0.5);
        for v in &m.verts {
            let s: f32 = v.weights.iter().sum();
            assert!((s - 1.0).abs() < 1e-4);
            if v.co.distance(top) > 0.8 {
                assert_eq!(influences(v), vec![(0, 1.0)]);
            }
        }
        // subtrair tudo devolve ao joint 0 (normalize)
        paint_weight(&mut m, top, 5.0, BrushFalloff::Constant, 1, 1.0, PaintBlend::Subtract, 1.0, true).unwrap();
        // quem só tinha o joint 1 fica sem influência; os demais voltam ao joint 0
        let mut unweighted = 0;
        for v in &m.verts {
            let inf = influences(v);
            if inf.is_empty() {
                unweighted += 1;
            } else {
                assert_eq!(inf, vec![(0, 1.0)]);
            }
        }
        assert!(unweighted >= 1);
        normalize_all(&mut m);
        assert!(m.verts.iter().all(|v| influences(v) == vec![(0, 1.0)]));
    }

    #[test]
    fn smooth_weights_blends_across_boundary() {
        let mut m = primitives::grid(8, 1, 8.0, 1.0);
        for v in &mut m.verts {
            v.joints = [if v.co.x < 0.0 { 0 } else { 1 }, 0, 0, 0];
            v.weights = [1.0, 0.0, 0.0, 0.0];
        }
        let all: Vec<u32> = (0..m.vert_count() as u32).collect();
        smooth_weights(&mut m, &all, 3, 0.8).unwrap();
        // vértice na junção tem as duas influências
        let mid = m.verts.iter().find(|v| v.co.x.abs() < 1e-5).unwrap();
        assert_eq!(influences(mid).len(), 2);
        for v in &m.verts {
            let s: f32 = v.weights.iter().sum();
            assert!((s - 1.0).abs() < 1e-4);
        }
        // extremos permanecem dominantes
        let left = m.verts.iter().find(|v| (v.co.x + 4.0).abs() < 1e-5).unwrap();
        assert_eq!(left.joints[0], 0);
        assert!(left.weights[0] > 0.9);
    }

    #[test]
    fn transfer_weights_body_to_cloth() {
        // corpo: cilindro com pesos que variam com y (joint 0 embaixo, joint 1 em cima)
        let mut body = primitives::cylinder(1.0, 4.0, 16, 8, true);
        for v in &mut body.verts {
            let t = ((v.co.y + 2.0) / 4.0).clamp(0.0, 1.0);
            v.joints = [0, 1, 0, 0];
            v.weights = [1.0 - t, t, 0.0, 0.0];
        }
        let mut cloth = primitives::cylinder(1.05, 3.0, 12, 6, false);
        let n = transfer_weights(&mut cloth, &body, None).unwrap();
        assert_eq!(n, cloth.vert_count());
        for v in &cloth.verts {
            let t = ((v.co.y + 2.0) / 4.0).clamp(0.0, 1.0);
            let w1 = influences(v).iter().find(|x| x.0 == 1).map(|x| x.1).unwrap_or(0.0);
            assert!((w1 - t).abs() < 0.08, "y={} w1={} t={}", v.co.y, w1, t);
            let s: f32 = v.weights.iter().sum();
            assert!((s - 1.0).abs() < 1e-4);
        }
        // max_distance exclui vértices longe
        let mut far = primitives::cylinder(1.5, 3.0, 12, 6, false);
        let snapshot = far.verts.clone();
        let n = transfer_weights(&mut far, &body, Some(0.2)).unwrap();
        assert_eq!(n, 0);
        assert_eq!(far.verts, snapshot);
    }

    #[test]
    fn envelope_weights_two_bones() {
        let mut m = primitives::cylinder(0.5, 4.0, 12, 16, true); // y in [-2, 2]
        let bones = [
            BoneEnvelope { joint: 10, head: Vec3::new(0.0, -2.0, 0.0), tail: Vec3::new(0.0, 0.0, 0.0), radius_head: 0.6, radius_tail: 0.6 },
            BoneEnvelope { joint: 11, head: Vec3::new(0.0, 0.0, 0.0), tail: Vec3::new(0.0, 2.0, 0.0), radius_head: 0.6, radius_tail: 0.6 },
        ];
        let n = auto_weights_envelope(&mut m, &bones, 0.5).unwrap();
        assert_eq!(n, m.vert_count());
        for v in &m.verts {
            let s: f32 = v.weights.iter().sum();
            assert!((s - 1.0).abs() < 1e-5);
            let inf = influences(v);
            let w11 = inf.iter().find(|x| x.0 == 11).map(|x| x.1).unwrap_or(0.0);
            if v.co.y < -0.5 {
                assert_eq!(inf, vec![(10, 1.0)]);
            } else if v.co.y > 0.5 {
                assert_eq!(inf, vec![(11, 1.0)]);
            } else if v.co.y.abs() < 1e-4 {
                assert!((w11 - 0.5).abs() < 1e-4);
            }
        }
        // sem falloff e vértice fora de todas as cápsulas: osso mais próximo
        let mut far = EditMesh::new("f");
        far.add_vert(Vec3::new(5.0, 1.5, 0.0));
        far.add_vert(Vec3::new(5.0, -1.5, 0.0));
        auto_weights_envelope(&mut far, &bones, 0.0).unwrap();
        assert_eq!(influences(&far.verts[0]), vec![(11, 1.0)]);
        assert_eq!(influences(&far.verts[1]), vec![(10, 1.0)]);
        // osso degenerado (head == tail) funciona como esfera
        let mut one = EditMesh::new("o");
        one.add_vert(Vec3::new(0.2, 0.0, 0.0));
        let sphere_bone = BoneEnvelope { joint: 3, head: Vec3::ZERO, tail: Vec3::ZERO, radius_head: 0.5, radius_tail: 0.5 };
        auto_weights_envelope(&mut one, &[sphere_bone], 1.0).unwrap();
        assert_eq!(influences(&one.verts[0]), vec![(3, 1.0)]);
        // mais de 4 ossos sobrepostos: no máximo 4 influências, soma 1
        let many: Vec<BoneEnvelope> = (0..6)
            .map(|i| BoneEnvelope { joint: i, head: Vec3::ZERO, tail: Vec3::ZERO, radius_head: 1.0 + i as f32 * 0.1, radius_tail: 1.0 })
            .collect();
        auto_weights_envelope(&mut one, &many, 0.5).unwrap();
        assert_eq!(influences(&one.verts[0]).len(), 4);
        let s: f32 = one.verts[0].weights.iter().sum();
        assert!((s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn envelope_falloff_is_smooth_and_monotonic() {
        let bone = BoneEnvelope { joint: 1, head: Vec3::ZERO, tail: Vec3::ZERO, radius_head: 1.0, radius_tail: 1.0 };
        let other = BoneEnvelope { joint: 2, head: Vec3::new(10.0, 0.0, 0.0), tail: Vec3::new(10.0, 0.0, 0.0), radius_head: 100.0, radius_tail: 100.0 };
        let mut prev = 2.0;
        for i in 0..=10 {
            let mut m = EditMesh::new("p");
            m.add_vert(Vec3::new(1.0 + i as f32 * 0.1, 0.0, 0.0));
            auto_weights_envelope(&mut m, &[bone, BoneEnvelope { radius_head: 0.01, radius_tail: 0.01, ..other }], 1.0).unwrap();
            let w1 = influences(&m.verts[0]).iter().find(|x| x.0 == 1).map(|x| x.1).unwrap_or(0.0);
            assert!(w1 <= prev + 1e-6);
            prev = w1;
        }
    }

    #[test]
    fn mirror_weights_maps_joints() {
        let mut m = primitives::quad_sphere(1.0, 4);
        for v in &mut m.verts {
            v.joints = [if v.co.x > 0.0 { 1 } else { 2 }, 5, 0, 0];
            v.weights = [0.75, 0.25, 0.0, 0.0];
        }
        // lado +X tem joint 1 (L), -X tem 2 (R) -> espelha +X sobre -X com 1->2 e 2->1
        for v in &mut m.verts {
            if v.co.x > 1e-4 {
                v.joints = [1, 5, 0, 0];
                v.weights = [0.9, 0.1, 0.0, 0.0];
            } else if v.co.x < -1e-4 {
                v.joints = [9, 9, 0, 0];
                v.weights = [1.0, 0.0, 0.0, 0.0];
            }
        }
        let n = mirror_weights(&mut m, MirrorAxis::X, true, &[(1, 2), (2, 1)], 1e-4).unwrap();
        let neg = m.verts.iter().filter(|v| v.co.x < -1e-4).count();
        assert_eq!(n, neg);
        for v in &m.verts {
            if v.co.x < -1e-4 {
                assert_eq!(influences(v), vec![(2, 0.9), (5, 0.1)]);
            }
        }
        // vértices no plano não mudam
        let on_plane = m.verts.iter().filter(|v| v.co.x.abs() <= 1e-4).count();
        assert!(on_plane > 0);
        // direção oposta e eixo Y
        let mut m2 = primitives::quad_sphere(1.0, 4);
        for v in &mut m2.verts {
            v.joints = [if v.co.y > 0.0 { 3 } else { 4 }, 0, 0, 0];
        }
        let n = mirror_weights(&mut m2, MirrorAxis::Y, false, &[(4, 3)], 1e-4).unwrap();
        assert!(n > 0);
        assert!(m2.verts.iter().filter(|v| v.co.y > 1e-4).all(|v| v.joints[0] == 3));
        // sem parceiro: nada escrito
        let mut asym = EditMesh::new("a");
        asym.add_vert(Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(mirror_weights(&mut asym, MirrorAxis::X, true, &[], 1e-4).unwrap(), 0);
    }

    #[test]
    fn large_mesh_ao_and_transfer_sanity() {
        let mut m = primitives::quad_sphere(1.0, 32);
        let t = std::time::Instant::now();
        bake_ao_per_vertex(&mut m, &AoParams { samples: 8, max_dist: 0.5, ..Default::default() }).unwrap();
        let src = primitives::quad_sphere(1.0, 24);
        let mut dst = primitives::quad_sphere(1.01, 32);
        transfer_weights(&mut dst, &src, None).unwrap();
        smooth_channel(&mut dst, None, VertexChannel::AmbientOcclusion, 2, 0.5).unwrap();
        assert!(t.elapsed().as_secs() < 90);
    }
}
