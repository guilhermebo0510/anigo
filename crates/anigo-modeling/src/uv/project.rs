//! Projeções de UV: planar, caixa (cúbica), cilíndrica, esférica, a partir da câmera e
//! *Smart UV Project*. Equivalentes às entradas do menu *UV → Project* do Blender.
//!
//! Todas as projeções escrevem UV por corner nas faces escolhidas (`faces = None` = malha
//! toda), validam parâmetros antes de mutar e produzem UVs com **enrolamento
//! anti-horário** para faces voltadas para fora (sem espelhamento visto de fora).

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::topology::Topology;
use crate::uv::pack::{pack_island_list, PackParams, PackReport};
use crate::uv::seams::{faces_uv_bounds, membership, resolve_faces, Island};
use glam::{Mat4, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

fn check_vec(name: &str, v: Vec3) -> ModelResult<()> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(ModelError::InvalidParameter(format!("{name} não é finito: {v:?}")))
    }
}

fn check_axis(name: &str, v: Vec3) -> ModelResult<Vec3> {
    check_vec(name, v)?;
    v.try_normalize()
        .filter(|n| n.length_squared() > 0.5)
        .ok_or_else(|| ModelError::InvalidParameter(format!("{name} não pode ser nulo")))
}

/// Escreve `uvs[i]` (por corner, ordem da face) nas faces dadas.
fn write_face_uvs(mesh: &mut EditMesh, face: u32, uvs: &[Vec2]) {
    for (c, uv) in mesh.faces[face as usize].corners.iter_mut().zip(uvs) {
        c.uv = *uv;
    }
}

// ---------------------------------------------------------------------- planar

/// Projeção **planar**: `uv = ((p − origin)·û, (p − origin)·v̂) · scale`, onde `û`, `v̂` são
/// `u_axis`/`v_axis` normalizados (podem ser oblíquos entre si, mas não paralelos).
/// `scale` = unidades UV por unidade 3D (negativo espelha). Para ficar sem espelhamento visto
/// de fora, use `u_axis × v_axis` apontando para o observador.
///
/// Erros (malha intacta): eixos nulos/paralelos/não finitos, `scale` nulo/não finito,
/// seleção inválida ou vazia.
pub fn project_planar(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    origin: Vec3,
    u_axis: Vec3,
    v_axis: Vec3,
    scale: f32,
) -> ModelResult<()> {
    check_vec("origin", origin)?;
    let (u, v) = (check_axis("u_axis", u_axis)?, check_axis("v_axis", v_axis)?);
    if u.cross(v).length() < 1e-6 {
        return Err(ModelError::InvalidParameter("u_axis e v_axis são paralelos".into()));
    }
    if !scale.is_finite() || scale == 0.0 {
        return Err(ModelError::InvalidParameter(format!("scale deve ser finito e != 0, recebeu {scale}")));
    }
    let faces = resolve_faces(mesh, faces)?;
    for &f in &faces {
        let uvs: Vec<Vec2> = mesh.faces[f as usize]
            .corners
            .iter()
            .map(|c| {
                let r = mesh.co(c.vert) - origin;
                Vec2::new(r.dot(u), r.dot(v)) * scale
            })
            .collect();
        write_face_uvs(mesh, f, &uvs);
    }
    Ok(())
}

// ---------------------------------------------------------------------- caixa

/// Layout da projeção em caixa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxLayout {
    /// Cada face usa `[0,1]²` do seu plano dominante (as 6 direções se sobrepõem, como o
    /// *Cube Projection* do Blender).
    PerFace,
    /// "Cruz" 4×3 em células quadradas de 1/4 (ocupa `[0,1]×[0,0.75]`):
    /// `+Y` em cima, `-X +Z +X -Z` no meio, `-Y` embaixo. Faces vizinhas na cruz ficam contínuas.
    Cross,
}

/// Projeção em **caixa**: cada face é projetada no plano do eixo dominante da sua normal
/// (desempate X < Y < Z; normal nula usa +Z). A caixa está centrada em `origin` com extensões
/// `size` (> 0), de modo que pontos dentro dela caem em `[0,1]` na célula. Sem espelhamento
/// visto de fora.
///
/// Erros (malha intacta): `size` com componente `<= 0`/não finita, `origin` não finito, seleção inválida.
pub fn project_box(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    origin: Vec3,
    size: Vec3,
    layout: BoxLayout,
) -> ModelResult<()> {
    check_vec("origin", origin)?;
    check_vec("size", size)?;
    if size.min_element() <= 0.0 {
        return Err(ModelError::InvalidParameter(format!("size deve ser > 0 em todos os eixos, recebeu {size:?}")));
    }
    let faces = resolve_faces(mesh, faces)?;
    for &f in &faces {
        let n = mesh.face_normal(f);
        let n = if n == Vec3::ZERO { Vec3::Z } else { n };
        let a = n.abs();
        let axis = if a.x >= a.y && a.x >= a.z {
            0
        } else if a.y >= a.z {
            1
        } else {
            2
        };
        let positive = n[axis] >= 0.0;
        let uvs: Vec<Vec2> = mesh.faces[f as usize]
            .corners
            .iter()
            .map(|c| {
                let t = (mesh.co(c.vert) - origin) / size + Vec3::splat(0.5);
                let (uv, cell) = match (axis, positive) {
                    (0, true) => (Vec2::new(1.0 - t.z, t.y), (2.0, 1.0)),
                    (0, false) => (Vec2::new(t.z, t.y), (0.0, 1.0)),
                    (1, true) => (Vec2::new(t.x, 1.0 - t.z), (1.0, 2.0)),
                    (1, false) => (Vec2::new(t.x, t.z), (1.0, 0.0)),
                    (_, true) => (Vec2::new(t.x, t.y), (1.0, 1.0)),
                    (_, false) => (Vec2::new(1.0 - t.x, t.y), (3.0, 1.0)),
                };
                match layout {
                    BoxLayout::PerFace => uv,
                    BoxLayout::Cross => (uv + Vec2::new(cell.0, cell.1)) * 0.25,
                }
            })
            .collect();
        write_face_uvs(mesh, f, &uvs);
    }
    Ok(())
}

// ---------------------------------------------------------------------- polar (cilindro / esfera)

/// Base ortonormal `(eixo, e1, e2)` com `e1 × e2 = eixo`; para eixo Y dá `e1 = X`, `e2 = −Z`
/// (mesma convenção angular das primitivas do crate).
fn polar_frame(axis: Vec3) -> (Vec3, Vec3, Vec3) {
    let helper = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let e1 = (helper - axis * helper.dot(axis)).normalize();
    (axis, e1, axis.cross(e1))
}

/// Ajusta os `u` (em voltas, periódicos) de uma face para minimizar o salto entre corners
/// consecutivos e os desloca em bloco para que o menor fique em `[0,1)`. Corners sem ângulo
/// definido (`None`, sobre o eixo/polo) recebem a média dos definidos.
fn wrap_face_u(raw: &[Option<f32>]) -> Vec<f32> {
    let mut out = vec![0.0f32; raw.len()];
    let mut prev: Option<f32> = None;
    let mut defined = Vec::new();
    for (i, r) in raw.iter().enumerate() {
        if let Some(u0) = r {
            let mut u = *u0;
            if let Some(p) = prev {
                u += (p - u).round();
            }
            prev = Some(u);
            out[i] = u;
            defined.push(i);
        }
    }
    let mean = if defined.is_empty() {
        0.0
    } else {
        defined.iter().map(|&i| out[i]).sum::<f32>() / defined.len() as f32
    };
    for (i, r) in raw.iter().enumerate() {
        if r.is_none() {
            out[i] = mean;
        }
    }
    let shift = out.iter().cloned().fold(f32::INFINITY, f32::min).floor();
    for u in &mut out {
        *u -= shift;
    }
    out
}

/// Ângulo normalizado `[0,1)` de `perp` no plano `(e1, e2)`; `None` se está sobre o eixo.
fn angle_turns(perp: Vec3, e1: Vec3, e2: Vec3, min_len: f32) -> Option<f32> {
    if perp.length() <= min_len {
        return None;
    }
    let t = perp.dot(e2).atan2(perp.dot(e1)) / TAU;
    Some(if t < 0.0 { t + 1.0 } else { t })
}

/// Projeção **cilíndrica** em torno do eixo `(axis_origin, axis_dir)`: `u` = ângulo em voltas
/// (`[0,1)`, crescendo para a direita visto de fora), `v` = altura ao longo do eixo dividida
/// por `2π·R` (o perímetro vira 1 em `u`, preservando proporção na altura do raio).
/// `radius`: `Some(R > 0)` fixa `R`; `None` usa a distância média ao eixo dos corners.
///
/// Faces que cruzam a emenda (`u = 0/1`) recebem `u` contínuo (podem passar de 1), de modo
/// que corners consecutivos nunca saltam mais de 0,5 — exceto faces que **envolvem** o eixo
/// (tampas), que não admitem projeção contínua. Corners exatamente sobre o eixo herdam a
/// média dos vizinhos na face.
///
/// Erros (malha intacta): eixo nulo/não finito, `radius` inválido, raio automático nulo
/// (tudo sobre o eixo), seleção inválida.
pub fn project_cylinder(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    axis_origin: Vec3,
    axis_dir: Vec3,
    radius: Option<f32>,
) -> ModelResult<()> {
    check_vec("axis_origin", axis_origin)?;
    let axis = check_axis("axis_dir", axis_dir)?;
    if let Some(r) = radius {
        if !r.is_finite() || r <= 0.0 {
            return Err(ModelError::InvalidParameter(format!("radius deve ser > 0, recebeu {r}")));
        }
    }
    let faces = resolve_faces(mesh, faces)?;
    let (a, e1, e2) = polar_frame(axis);
    let rel = |mesh: &EditMesh, v: u32| mesh.co(v) - axis_origin;
    let radius = match radius {
        Some(r) => r,
        None => {
            let (mut sum, mut n) = (0.0f64, 0usize);
            for &f in &faces {
                for c in &mesh.faces[f as usize].corners {
                    let r = rel(mesh, c.vert);
                    sum += (r - a * r.dot(a)).length() as f64;
                    n += 1;
                }
            }
            let r = (sum / n.max(1) as f64) as f32;
            if r <= 1e-9 {
                return Err(ModelError::InvalidParameter("todos os vértices estão sobre o eixo do cilindro".into()));
            }
            r
        }
    };
    let min_len = radius * 1e-5;
    for &f in &faces {
        let (mut raw, mut vs) = (Vec::new(), Vec::new());
        for c in &mesh.faces[f as usize].corners {
            let r = rel(mesh, c.vert);
            let h = r.dot(a);
            raw.push(angle_turns(r - a * h, e1, e2, min_len));
            vs.push(h / (TAU * radius));
        }
        let us = wrap_face_u(&raw);
        let uvs: Vec<Vec2> = us.iter().zip(&vs).map(|(&u, &v)| Vec2::new(u, v)).collect();
        write_face_uvs(mesh, f, &uvs);
    }
    Ok(())
}

/// Projeção **esférica** (equiretangular) a partir de `center`, com polos em ±Y:
/// `u` = longitude em voltas (mesma convenção de [`project_cylinder`] com eixo Y, `+X` em
/// `u = 0`), `v = 0.5 + latitude/π` (`v = 1` em +Y). O salto da emenda de `u` é tratado
/// por face (corners consecutivos nunca saltam mais de 0,5, exceto faces que envolvem um polo);
/// corners nos polos herdam o `u` médio dos vizinhos da face.
///
/// Erros (malha intacta): `center` não finito, seleção inválida.
pub fn project_sphere(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    center: Vec3,
) -> ModelResult<()> {
    check_vec("center", center)?;
    let faces = resolve_faces(mesh, faces)?;
    let (a, e1, e2) = polar_frame(Vec3::Y);
    for &f in &faces {
        let (mut raw, mut vs) = (Vec::new(), Vec::new());
        for c in &mesh.faces[f as usize].corners {
            let r = mesh.co(c.vert) - center;
            let len = r.length();
            let h = r.dot(a);
            raw.push(angle_turns(r - a * h, e1, e2, len * 1e-5));
            vs.push(if len > 0.0 { 0.5 + (h / len).clamp(-1.0, 1.0).asin() / PI } else { 0.5 });
        }
        let us = wrap_face_u(&raw);
        let uvs: Vec<Vec2> = us.iter().zip(&vs).map(|(&u, &v)| Vec2::new(u, v)).collect();
        write_face_uvs(mesh, f, &uvs);
    }
    Ok(())
}

// ---------------------------------------------------------------------- câmera

/// Projeção **a partir da vista** (*Project from View*): `uv = ndc.xy · 0.5 + 0.5`, com
/// `ndc = (view_proj · p).xyz / w`. Vale para matrizes perspectivas e ortográficas.
///
/// Erros (malha intacta): matriz não finita, vértice com `w <= 1e-8` (atrás/sobre o plano da
/// câmera) ou resultado não finito, seleção inválida.
pub fn project_from_view(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    view_proj: Mat4,
) -> ModelResult<()> {
    if !view_proj.to_cols_array().iter().all(|x| x.is_finite()) {
        return Err(ModelError::InvalidParameter("view_proj contém valores não finitos".into()));
    }
    let faces = resolve_faces(mesh, faces)?;
    let project = |mesh: &EditMesh, v: u32| -> ModelResult<Vec2> {
        let p = mesh.co(v);
        let clip = view_proj * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w.is_nan() || clip.w <= 1e-8 {
            return Err(ModelError::InvalidParameter(format!(
                "vértice {v} está atrás da câmera (w = {})",
                clip.w
            )));
        }
        let uv = Vec2::new(clip.x / clip.w, clip.y / clip.w) * 0.5 + Vec2::splat(0.5);
        if !uv.is_finite() {
            return Err(ModelError::Numeric(format!("projeção não finita no vértice {v}")));
        }
        Ok(uv)
    };
    let mut staged: Vec<(u32, Vec<Vec2>)> = Vec::with_capacity(faces.len());
    for &f in &faces {
        let uvs = mesh.faces[f as usize]
            .corners
            .iter()
            .map(|c| project(mesh, c.vert))
            .collect::<ModelResult<Vec<_>>>()?;
        staged.push((f, uvs));
    }
    for (f, uvs) in staged {
        write_face_uvs(mesh, f, &uvs);
    }
    Ok(())
}

// ---------------------------------------------------------------------- smart uv project

/// Resultado de [`smart_uv_project`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SmartUvReport {
    /// Grupos planares formados.
    pub groups: usize,
    pub pack: PackReport,
}

/// *Smart UV Project* de toda a malha. Ver [`smart_uv_project_in`].
pub fn smart_uv_project(
    mesh: &mut EditMesh,
    angle_limit: f32,
    island_margin: f32,
) -> ModelResult<SmartUvReport> {
    smart_uv_project_in(mesh, None, angle_limit, island_margin)
}

/// *Smart UV Project*: agrupa as faces em regiões quase planares e projeta cada grupo no seu
/// plano médio (isometria exata em regiões planas), depois empacota tudo em `[0,1]²`.
///
/// Algoritmo (determinístico): sementes por área decrescente (desempate por índice); cada
/// grupo cresce por vizinhança de aresta (BFS) com as faces cuja normal difere da normal da
/// semente em no máximo `angle_limit` (radianos, `(0, π/2)`: como o Blender, abaixo de 90°
/// para que o grupo nunca dobre sobre si). A projeção usa a normal média ponderada por área
/// (ou a da semente, se alguma face ficaria quase de perfil); o eixo vertical da UV é o Y do
/// mundo projetado (Z para grupos horizontais), sem espelhamento. `island_margin` é a margem entre ilhas como fração do
/// espaço UV (`[0, 0.5)`), e a densidade de texels resultante é uniforme entre grupos.
///
/// As costuras (`EdgeAttr::seam`) não são alteradas; as fronteiras entre grupos ficam
/// registradas apenas nos UVs. Erros (malha intacta): parâmetros fora do intervalo ou
/// seleção inválida.
pub fn smart_uv_project_in(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    angle_limit: f32,
    island_margin: f32,
) -> ModelResult<SmartUvReport> {
    if !angle_limit.is_finite() || angle_limit <= 0.0 || angle_limit >= PI / 2.0 {
        return Err(ModelError::InvalidParameter(format!(
            "angle_limit deve estar em (0, π/2) radianos (como o limite de 89° do Blender), recebeu {angle_limit}"
        )));
    }
    if !island_margin.is_finite() || !(0.0..0.5).contains(&island_margin) {
        return Err(ModelError::InvalidParameter(format!(
            "island_margin deve estar em [0, 0.5), recebeu {island_margin}"
        )));
    }
    let faces = resolve_faces(mesh, faces)?;
    mesh.validate()?;
    let topo = Topology::build(mesh);
    let in_set = membership(mesh, &faces);
    let normals: Vec<Vec3> = (0..mesh.faces.len() as u32).map(|f| mesh.face_normal(f)).collect();
    let areas: Vec<f32> = (0..mesh.faces.len() as u32).map(|f| mesh.face_area(f)).collect();
    let mut order = faces.clone();
    order.sort_by(|&a, &b| areas[b as usize].total_cmp(&areas[a as usize]).then(a.cmp(&b)));
    let cos_limit = angle_limit.cos();

    let mut assigned = vec![false; mesh.faces.len()];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for &seed in &order {
        if assigned[seed as usize] {
            continue;
        }
        assigned[seed as usize] = true;
        let mut group = vec![seed];
        let mut queue = VecDeque::from([seed]);
        let n_seed = normals[seed as usize];
        while let Some(f) = queue.pop_front() {
            for g in topo.face_neighbors(f) {
                if !in_set[g as usize] || assigned[g as usize] {
                    continue;
                }
                let ng = normals[g as usize];
                let ok = if n_seed == Vec3::ZERO || ng == Vec3::ZERO {
                    n_seed == ng
                } else {
                    n_seed.dot(ng) >= cos_limit - 1e-6
                };
                if ok {
                    assigned[g as usize] = true;
                    group.push(g);
                    queue.push_back(g);
                }
            }
        }
        group.sort_unstable();
        groups.push(group);
    }

    // projeção de cada grupo
    for g in &groups {
        let mut n = Vec3::ZERO;
        let mut center = Vec3::ZERO;
        let mut wsum = 0.0;
        for &f in g {
            n += normals[f as usize] * areas[f as usize];
            center += mesh.face_center(f) * areas[f as usize];
            wsum += areas[f as usize];
        }
        let n_seed = normals[g[0] as usize].try_normalize().unwrap_or(Vec3::Z);
        let mut n = n.try_normalize().unwrap_or(n_seed);
        // a semente é a maior face do grupo (primeira escolhida), não necessariamente g[0]
        if g.iter().any(|&f| normals[f as usize] != Vec3::ZERO && normals[f as usize].dot(n) < 0.1) {
            n = g
                .iter()
                .copied()
                .max_by(|&a, &b| areas[a as usize].total_cmp(&areas[b as usize]).then(b.cmp(&a)))
                .and_then(|f| normals[f as usize].try_normalize())
                .unwrap_or(n_seed);
        }
        let center = if wsum > 0.0 { center / wsum } else { mesh.face_center(g[0]) };
        let up = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::Z };
        let v_axis = (up - n * up.dot(n)).normalize_or_zero();
        let u_axis = v_axis.cross(n);
        for &f in g {
            let uvs: Vec<Vec2> = mesh.faces[f as usize]
                .corners
                .iter()
                .map(|c| {
                    let r = mesh.co(c.vert) - center;
                    Vec2::new(r.dot(u_axis), r.dot(v_axis))
                })
                .collect();
            write_face_uvs(mesh, f, &uvs);
        }
    }

    let islands: Vec<Island> = groups
        .iter()
        .map(|g| Island { faces: g.clone(), bounds_uv: faces_uv_bounds(mesh, g) })
        .collect();
    let pack = pack_island_list(
        mesh,
        &islands,
        &PackParams { margin: island_margin * 1024.0, texture_size: 1024, ..PackParams::default() },
    )?;
    Ok(SmartUvReport { groups: groups.len(), pack })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::uv::metrics::{flipped_faces, overlap_count, self_overlapping_faces, texel_density, uv_bounds, uv_hash};
    use crate::uv::seams::split_islands;
    use glam::Quat;

    fn max_jump(m: &EditMesh, f: usize) -> f32 {
        let cs = &m.faces[f].corners;
        (0..cs.len())
            .map(|i| (cs[i].uv.x - cs[(i + 1) % cs.len()].uv.x).abs())
            .fold(0.0, f32::max)
    }

    // ------------------------------------------------ planar

    #[test]
    fn planar_projection_matches_formula_and_scale() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        project_planar(&mut m, None, Vec3::new(-1.0, 0.0, 1.0), Vec3::X, Vec3::NEG_Z, 0.5).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                let p = m.co(c.vert);
                assert!((c.uv - Vec2::new((p.x + 1.0) * 0.5, (1.0 - p.z) * 0.5)).length() < 1e-6);
            }
        }
        // visto de cima (+Y) com u=X, v=-Z: anti-horário, sem espelhar
        assert!(flipped_faces(&m).is_empty());
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.length() < 1e-6 && (hi - Vec2::ONE).length() < 1e-6);
    }

    #[test]
    fn planar_on_inclined_plane_is_isometric_with_unit_scale() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        let q = Quat::from_rotation_x(0.6) * Quat::from_rotation_y(1.1);
        m.transform_positions(|p| q * p);
        project_planar(&mut m, None, Vec3::ZERO, q * Vec3::X, q * Vec3::NEG_Z, 1.0).unwrap();
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.0001);
        assert!(flipped_faces(&m).is_empty());
    }

    #[test]
    fn planar_validation_and_subset() {
        let mut m = primitives::grid(2, 2, 1.0, 1.0);
        let before = m.clone();
        let bad: [(Vec3, Vec3, Vec3, f32); 6] = [
            (Vec3::ZERO, Vec3::ZERO, Vec3::Z, 1.0),
            (Vec3::ZERO, Vec3::X, Vec3::X * 2.0, 1.0),
            (Vec3::ZERO, Vec3::X, Vec3::Z, 0.0),
            (Vec3::ZERO, Vec3::X, Vec3::Z, f32::NAN),
            (Vec3::NAN, Vec3::X, Vec3::Z, 1.0),
            (Vec3::ZERO, Vec3::new(f32::INFINITY, 0.0, 0.0), Vec3::Z, 1.0),
        ];
        for (o, u, v, s) in bad {
            assert!(matches!(project_planar(&mut m, None, o, u, v, s), Err(ModelError::InvalidParameter(_))), "{o:?} {u:?} {v:?} {s}");
        }
        assert!(matches!(project_planar(&mut m, Some(&[]), Vec3::ZERO, Vec3::X, Vec3::Z, 1.0), Err(ModelError::EmptySelection(_))));
        assert!(matches!(project_planar(&mut m, Some(&[77]), Vec3::ZERO, Vec3::X, Vec3::Z, 1.0), Err(ModelError::FaceOutOfRange(77, 4))));
        assert_eq!(m, before);
        project_planar(&mut m, Some(&[1, 1]), Vec3::ZERO, Vec3::X, Vec3::Z, 2.0).unwrap();
        assert_eq!(m.faces[0], before.faces[0]);
        assert_ne!(m.faces[1], before.faces[1]);
        // escala negativa espelha (aceita)
        project_planar(&mut m, None, Vec3::ZERO, Vec3::X, Vec3::Z, -1.0).unwrap();
    }

    // ------------------------------------------------ caixa

    #[test]
    fn box_per_face_fills_unit_square_per_side_without_flips() {
        let mut m = primitives::cube(2.0);
        project_box(&mut m, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::PerFace).unwrap();
        assert!(flipped_faces(&m).is_empty(), "{:?}", flipped_faces(&m));
        for f in 0..6u32 {
            assert!((crate::uv::metrics::face_uv_area_signed(&m, f) - 1.0).abs() < 1e-5);
        }
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.0001);
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.min_element() >= -1e-6 && hi.max_element() <= 1.0 + 1e-6);
    }

    #[test]
    fn box_cross_layout_is_one_continuous_net_without_overlap() {
        let mut m = primitives::cube(2.0);
        project_box(&mut m, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::Cross).unwrap();
        assert!(flipped_faces(&m).is_empty());
        assert!(self_overlapping_faces(&m).is_empty());
        assert_eq!(overlap_count(&m), 0);
        assert_eq!(split_islands(&m).len(), 1, "a cruz é uma rede contínua");
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.min_element() >= -1e-6 && hi.x <= 1.0 + 1e-6 && hi.y <= 0.75 + 1e-6);
        // as 6 faces ocupam 6 células distintas
        let mut cells = std::collections::BTreeSet::new();
        for f in 0..6u32 {
            let c = m.face_center(f);
            let _ = c;
            let uv: Vec2 = m.faces[f as usize].corners.iter().map(|c| c.uv).sum::<Vec2>() / 4.0;
            cells.insert(((uv.x * 4.0).floor() as i32, (uv.y * 4.0).floor() as i32));
        }
        assert_eq!(cells.len(), 6);
    }

    #[test]
    fn box_on_subdivided_cube_and_sphere_uses_dominant_axis() {
        let mut m = primitives::cube_subdivided(2.0, 3);
        project_box(&mut m, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::PerFace).unwrap();
        assert!(flipped_faces(&m).is_empty());
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.0001);
        let mut s = primitives::quad_sphere(1.0, 6);
        project_box(&mut s, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::PerFace).unwrap();
        // faces voltadas para +Z (dominante) têm u = t.x crescente => nenhuma invertida
        assert!(flipped_faces(&s).is_empty());
    }

    #[test]
    fn box_validation() {
        let mut m = primitives::cube(1.0);
        let before = m.clone();
        for size in [Vec3::new(1.0, 0.0, 1.0), Vec3::new(-1.0, 1.0, 1.0), Vec3::NAN] {
            assert!(project_box(&mut m, None, Vec3::ZERO, size, BoxLayout::PerFace).is_err());
        }
        assert!(project_box(&mut m, None, Vec3::INFINITY, Vec3::ONE, BoxLayout::Cross).is_err());
        assert_eq!(m, before);
    }

    // ------------------------------------------------ cilindro

    #[test]
    fn cylinder_projection_unrolls_lateral_surface_isometrically() {
        let mut m = primitives::cylinder(1.0, 2.0, 24, 4, false);
        project_cylinder(&mut m, None, Vec3::ZERO, Vec3::Y, Some(1.0)).unwrap();
        assert!(flipped_faces(&m).is_empty(), "{:?}", flipped_faces(&m));
        for f in 0..m.faces.len() {
            assert!(max_jump(&m, f) <= 0.5, "face {f} salta {}", max_jump(&m, f));
        }
        // u dá a volta completa, v cobre 2/(2π)
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.x >= -1e-6 && hi.x >= 0.95 && hi.x <= 1.0 + 1.0 / 24.0 + 1e-5, "{lo:?} {hi:?}");
        assert!(((hi.y - lo.y) - 2.0 / TAU).abs() < 1e-5);
        // com R fixo = raio real, distorção ~ corda vs arco (<2%)
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.02);
        // raio automático dá o mesmo resultado aqui
        let mut a = primitives::cylinder(1.0, 2.0, 24, 4, false);
        project_cylinder(&mut a, None, Vec3::ZERO, Vec3::Y, None).unwrap();
        assert!(m.faces.iter().zip(&a.faces).all(|(x, y)| x.corners.iter().zip(&y.corners).all(|(p, q)| (p.uv - q.uv).length() < 1e-5)));
    }

    #[test]
    fn cylinder_is_rotation_equivariant_up_to_a_u_offset() {
        let base = primitives::cylinder(0.7, 1.5, 16, 3, false);
        let mut a = base.clone();
        project_cylinder(&mut a, None, Vec3::ZERO, Vec3::Y, Some(0.7)).unwrap();
        let q = Quat::from_axis_angle(Vec3::new(1.0, 1.0, 0.0).normalize(), 0.8);
        let mut b = base.clone();
        b.transform_positions(|p| q * p);
        project_cylinder(&mut b, None, Vec3::ZERO, q * Vec3::Y, Some(0.7)).unwrap();
        let mut offset: Option<f32> = None;
        for (fa, fb) in a.faces.iter().zip(&b.faces) {
            for (ca, cb) in fa.corners.iter().zip(&fb.corners) {
                assert!((ca.uv.y - cb.uv.y).abs() < 1e-4, "v difere");
                let d = (cb.uv.x - ca.uv.x).rem_euclid(1.0);
                let d = if d > 0.5 { d - 1.0 } else { d };
                match offset {
                    None => offset = Some(d),
                    Some(o) => assert!((o - d).abs() < 1e-3, "offset de u inconsistente {o} {d}"),
                }
            }
        }
    }

    #[test]
    fn cylinder_handles_axis_vertices_and_validates() {
        // cone com ponta sobre o eixo: o corner da ponta herda u dos vizinhos
        let mut m = primitives::cone(1.0, 0.0, 2.0, 12, false);
        project_cylinder(&mut m, None, Vec3::ZERO, Vec3::Y, Some(1.0)).unwrap();
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
        for f in 0..m.faces.len() {
            assert!(max_jump(&m, f) <= 0.5);
        }
        let before = m.clone();
        for (axis, r) in [(Vec3::ZERO, Some(1.0)), (Vec3::Y, Some(0.0)), (Vec3::Y, Some(-1.0)), (Vec3::Y, Some(f32::NAN)), (Vec3::NAN, None)] {
            assert!(matches!(project_cylinder(&mut m, None, Vec3::ZERO, axis, r), Err(ModelError::InvalidParameter(_))));
        }
        // tudo sobre o eixo e raio automático
        let mut line = EditMesh::new("linha");
        for y in 0..3 {
            line.add_vert(Vec3::new(0.0, y as f32, 0.0));
        }
        line.add_face(&[0, 1, 2]).unwrap();
        assert!(matches!(project_cylinder(&mut line, None, Vec3::ZERO, Vec3::Y, None), Err(ModelError::InvalidParameter(_))));
        assert_eq!(m, before);
    }

    // ------------------------------------------------ esfera

    #[test]
    fn sphere_projection_has_no_wrap_jumps_on_pole_vertex_spheres() {
        for m0 in [primitives::uv_sphere(1.0, 12, 24), primitives::quad_sphere(1.0, 4), primitives::quad_sphere(1.0, 8)] {
            let mut m = m0;
            project_sphere(&mut m, None, Vec3::ZERO).unwrap();
            for f in 0..m.faces.len() {
                assert!(max_jump(&m, f) <= 0.5 + 1e-6, "{}: face {f} salta {}", m.name, max_jump(&m, f));
            }
            assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite() && c.uv.y >= -1e-6 && c.uv.y <= 1.0 + 1e-6)));
            // faixa equatorial: sem faces invertidas
            let flipped = flipped_faces(&m);
            assert!(flipped.iter().all(|&f| m.face_center(f).y.abs() > 0.5), "{}: invertidas fora dos polos: {flipped:?}", m.name);
            // v cresce com y
            for f in &m.faces {
                for c in &f.corners {
                    let y = m.co(c.vert).y;
                    assert!((c.uv.y - (0.5 + y.clamp(-1.0, 1.0).asin() / PI)).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    fn sphere_projection_respects_center_and_validates() {
        let mut m = primitives::uv_sphere(1.0, 8, 16);
        m.transform_positions(|p| p + Vec3::new(3.0, 1.0, -2.0));
        project_sphere(&mut m, None, Vec3::new(3.0, 1.0, -2.0)).unwrap();
        for f in 0..m.faces.len() {
            assert!(max_jump(&m, f) <= 0.5 + 1e-6);
        }
        let before = m.clone();
        assert!(project_sphere(&mut m, None, Vec3::NAN).is_err());
        assert!(project_sphere(&mut m, Some(&[9999]), Vec3::ZERO).is_err());
        assert_eq!(m, before);
        // vértice exatamente no centro: não gera NaN
        let mut d = EditMesh::new("centro");
        d.add_vert(Vec3::ZERO);
        d.add_vert(Vec3::X);
        d.add_vert(Vec3::Z);
        d.add_face(&[0, 1, 2]).unwrap();
        project_sphere(&mut d, None, Vec3::ZERO).unwrap();
        assert!(d.faces[0].corners.iter().all(|c| c.uv.is_finite()));
    }

    // ------------------------------------------------ câmera

    #[test]
    fn view_projection_ortho_and_perspective() {
        let mut m = primitives::grid(2, 2, 2.0, 2.0);
        let ortho = Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0) * Mat4::look_at_rh(Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO, Vec3::NEG_Z);
        project_from_view(&mut m, None, ortho).unwrap();
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.length() < 1e-5 && (hi - Vec2::ONE).length() < 1e-5, "{lo:?} {hi:?}");
        assert!(flipped_faces(&m).is_empty());
        let persp = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0) * Mat4::look_at_rh(Vec3::new(0.0, 4.0, 0.0), Vec3::ZERO, Vec3::NEG_Z);
        project_from_view(&mut m, None, persp).unwrap();
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
        // câmera dentro/atrás da malha: erro e malha intacta
        let before = m.clone();
        let behind = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0) * Mat4::look_at_rh(Vec3::new(0.0, -4.0, 0.0), Vec3::new(0.0, -10.0, 0.0), Vec3::NEG_Z);
        assert!(matches!(project_from_view(&mut m, None, behind), Err(ModelError::InvalidParameter(_))));
        let mut nan = Mat4::IDENTITY;
        nan.x_axis.x = f32::NAN;
        assert!(project_from_view(&mut m, None, nan).is_err());
        assert_eq!(m, before);
    }

    // ------------------------------------------------ smart uv project

    #[test]
    fn smart_project_cube_gives_six_planar_groups_packed_uniformly() {
        let mut m = primitives::cube(2.0);
        let rep = smart_uv_project(&mut m, 30.0_f32.to_radians(), 0.01).unwrap();
        assert_eq!(rep.groups, 6);
        assert_eq!(rep.pack.islands, 6);
        assert_eq!(overlap_count(&m), 0);
        assert!(flipped_faces(&m).is_empty());
        let (lo, hi) = uv_bounds(&m).unwrap();
        assert!(lo.min_element() >= 0.0 && hi.max_element() <= 1.0 + 1e-6);
        let d = texel_density(&m, 1024).unwrap();
        assert!((d.max - d.min) / d.mean < 1e-3, "{d:?}");
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.0001);
        assert!(rep.pack.used_area_ratio > 0.5);
    }

    #[test]
    fn smart_project_angle_controls_group_count() {
        let base = primitives::quad_sphere(1.0, 6);
        let mut counts = Vec::new();
        for deg in [15.0f32, 45.0, 70.0, 89.0] {
            let mut m = base.clone();
            let rep = smart_uv_project(&mut m, deg.to_radians(), 0.002).unwrap();
            counts.push(rep.groups);
            assert_eq!(overlap_count(&m), 0, "{deg}°");
            assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
            assert!(self_overlapping_faces(&m).is_empty(), "{deg}°");
            assert!(flipped_faces(&m).is_empty(), "{deg}°");
        }
        assert!(counts.windows(2).all(|w| w[0] >= w[1]), "mais ângulo => menos grupos: {counts:?}");
    }

    #[test]
    fn smart_project_planar_grid_is_one_isometric_group() {
        let mut m = primitives::grid(5, 5, 3.0, 3.0);
        let q = Quat::from_rotation_z(0.9) * Quat::from_rotation_x(0.4);
        m.transform_positions(|p| q * p);
        let rep = smart_uv_project(&mut m, 66.0_f32.to_radians(), 0.0).unwrap();
        assert_eq!(rep.groups, 1);
        assert!(crate::uv::metrics::mean_area_distortion(&m) < 1.0001);
        assert!(flipped_faces(&m).is_empty());
    }

    #[test]
    fn smart_project_validation_subset_and_determinism() {
        let mut m = primitives::cube(1.0);
        let before = m.clone();
        for (a, g) in [(0.0f32, 0.0f32), (-1.0, 0.0), (4.0, 0.0), (1.6, 0.0), (f32::NAN, 0.0), (1.0, -0.1), (1.0, 0.5), (1.0, f32::NAN)] {
            assert!(matches!(smart_uv_project(&mut m, a, g), Err(ModelError::InvalidParameter(_))), "{a} {g}");
        }
        assert!(matches!(smart_uv_project_in(&mut m, Some(&[]), 1.0, 0.0), Err(ModelError::EmptySelection(_))));
        assert!(matches!(smart_uv_project_in(&mut m, Some(&[10]), 1.0, 0.0), Err(ModelError::FaceOutOfRange(10, 6))));
        assert_eq!(m, before);
        assert_eq!(uv_hash(&m), uv_hash(&before));
        let rep = smart_uv_project_in(&mut m, Some(&[0, 1, 2]), 30.0_f32.to_radians(), 0.01).unwrap();
        assert_eq!(rep.groups, 3);
        assert_eq!(m.faces[5], before.faces[5]);
        let run = || {
            let mut s = primitives::capsule(0.5, 1.0, 12, 4);
            smart_uv_project(&mut s, 60.0_f32.to_radians(), 0.003).unwrap();
            uv_hash(&s)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn projections_are_deterministic_bit_by_bit() {
        let run = |k: u32| {
            let mut m = primitives::quad_sphere(1.0, 8);
            match k {
                0 => project_box(&mut m, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::Cross).unwrap(),
                1 => project_cylinder(&mut m, None, Vec3::ZERO, Vec3::Y, None).unwrap(),
                _ => project_sphere(&mut m, None, Vec3::ZERO).unwrap(),
            }
            uv_hash(&m)
        };
        for k in 0..3 {
            assert_eq!(run(k), run(k));
        }
    }

    #[test]
    fn large_mesh_projections_sanity() {
        let mut m = primitives::quad_sphere(1.0, 64);
        assert!(m.face_count() > 24_000);
        let t = std::time::Instant::now();
        project_sphere(&mut m, None, Vec3::ZERO).unwrap();
        project_cylinder(&mut m, None, Vec3::ZERO, Vec3::Y, None).unwrap();
        project_box(&mut m, None, Vec3::ZERO, Vec3::splat(2.0), BoxLayout::PerFace).unwrap();
        let rep = smart_uv_project(&mut m, 40.0_f32.to_radians(), 0.002).unwrap();
        assert!(rep.groups > 6);
        assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
        assert_eq!(overlap_count(&m), 0);
    }
}
