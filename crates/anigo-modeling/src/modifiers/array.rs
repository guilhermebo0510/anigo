//! Modificador **Array**: repete a malha `count` vezes com deslocamento
//! relativo/constante e transformação incremental (rotação e escala por
//! instância), com solda entre cópias vizinhas e tampas inicial/final.
//! Equivalente ao Array Modifier do Blender (cercas, escadas, correntes,
//! anéis de cabelo, franjas…).
//!
//! # Transformação por instância
//!
//! Seja `d = relative_offset · tamanho_da_bbox + constant_offset` e
//! `M = T(d) · T(pivô) · R · S · T(−pivô)` (R = Euler XYZ de `rotation_step`,
//! S = `scale_step`). A instância `i` usa `Mᶦ` (as transformações **acumulam**,
//! então rotação + deslocamento produzem hélices/escadas em caracol). O pivô é o
//! centro da bbox da malha-base, a menos que `pivot` seja informado (use
//! `Some(Vec3::ZERO)` para girar em torno da origem e formar anéis). Tampas:
//! `start_cap` usa `M⁻¹` e `end_cap` usa `Mᶜᵒᵘⁿᵗ`.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EdgeAttr, EditMesh, Face};
use glam::{EulerRot, Mat3, Mat4, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Número máximo de cópias.
pub const MAX_COUNT: u32 = 100_000;
/// Limite de faces do resultado.
pub const MAX_FACES: usize = 8_000_000;

/// Modo de ajuste automático do número de cópias.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ArrayFit {
    /// Preenche um comprimento total: `count = floor(L / |d|)` (mínimo 1), como
    /// "Fit Length" do Blender. Ignora `count`.
    Length(f32),
}

/// Parâmetros do Array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrayParams {
    /// Número de cópias (incluindo a original), `1..=100000`.
    pub count: u32,
    /// Deslocamento por instância como fração do tamanho da bbox da malha-base.
    pub relative_offset: Vec3,
    /// Deslocamento por instância em unidades de mundo (somado ao relativo).
    pub constant_offset: Vec3,
    /// Rotação incremental por instância (radianos, Euler XYZ).
    pub rotation_step: Vec3,
    /// Fator de escala incremental por instância (1 = sem mudança; não pode ser ~0).
    pub scale_step: Vec3,
    /// Pivô da rotação/escala; `None` = centro da bbox.
    pub pivot: Option<Vec3>,
    /// Solda vértices coincidentes entre cópias vizinhas.
    pub merge: bool,
    /// Distância de solda (`>= 0`).
    pub merge_distance: f32,
    /// Solda também a última cópia com a primeira (anéis fechados).
    pub merge_ends: bool,
    /// Malha usada como tampa antes da primeira cópia.
    pub start_cap: Option<EditMesh>,
    /// Malha usada como tampa depois da última cópia.
    pub end_cap: Option<EditMesh>,
    pub fit: Option<ArrayFit>,
}

impl Default for ArrayParams {
    fn default() -> Self {
        Self {
            count: 2,
            relative_offset: Vec3::new(1.0, 0.0, 0.0),
            constant_offset: Vec3::ZERO,
            rotation_step: Vec3::ZERO,
            scale_step: Vec3::ONE,
            pivot: None,
            merge: false,
            merge_distance: 1e-4,
            merge_ends: false,
            start_cap: None,
            end_cap: None,
            fit: None,
        }
    }
}

impl ArrayParams {
    fn validate(&self) -> ModelResult<()> {
        let finite = |v: Vec3, what: &str| -> ModelResult<()> {
            if v.is_finite() {
                Ok(())
            } else {
                Err(ModelError::InvalidParameter(format!("{what} não finito")))
            }
        };
        finite(self.relative_offset, "relative_offset")?;
        finite(self.constant_offset, "constant_offset")?;
        finite(self.rotation_step, "rotation_step")?;
        finite(self.scale_step, "scale_step")?;
        if let Some(p) = self.pivot {
            finite(p, "pivot")?;
        }
        if self.scale_step.abs().min_element() < 1e-6 {
            return Err(ModelError::InvalidParameter(
                "scale_step não pode ter componente nula (|s| < 1e-6)".into(),
            ));
        }
        if !self.merge_distance.is_finite() || self.merge_distance < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "merge_distance deve ser finito e >= 0, recebeu {}",
                self.merge_distance
            )));
        }
        if self.count == 0 || self.count > MAX_COUNT {
            return Err(ModelError::InvalidParameter(format!(
                "count deve estar em 1..={MAX_COUNT}, recebeu {}",
                self.count
            )));
        }
        if let Some(ArrayFit::Length(l)) = self.fit {
            if !l.is_finite() || l <= 0.0 {
                return Err(ModelError::InvalidParameter(format!(
                    "fit Length deve ser finito e > 0, recebeu {l}"
                )));
            }
        }
        Ok(())
    }
}

/// Instância já transformada: vértices/faces/arestas prontos para anexar.
fn transformed(mesh: &EditMesh, m: &Mat4) -> EditMesh {
    let mut out = mesh.clone();
    for v in &mut out.verts {
        v.co = m.transform_point3(v.co);
    }
    let m3 = Mat3::from_mat4(*m);
    let det = m3.determinant();
    let normal_m = if det.abs() > 1e-20 { Some(m3.inverse().transpose()) } else { None };
    for f in &mut out.faces {
        for c in &mut f.corners {
            c.normal = match (c.normal, normal_m) {
                (Some(n), Some(nm)) => (nm * n).try_normalize(),
                _ => None,
            };
        }
        if det < 0.0 {
            f.corners.reverse();
        }
    }
    out
}

fn combine(a: EdgeAttr, b: EdgeAttr) -> EdgeAttr {
    EdgeAttr {
        sharp: a.sharp || b.sharp,
        seam: a.seam || b.seam,
        crease: a.crease.max(b.crease),
        bevel_weight: a.bevel_weight.max(b.bevel_weight),
    }
}

/// Gera o array.
///
/// * Devolve uma malha nova: a cópia 0 vem primeiro (os índices da malha original
///   são preservados), depois as cópias `1..count`, depois `start_cap` e `end_cap`.
/// * Atributos (UV, cor, skin, material…) são copiados em cada instância; normais
///   customizadas são transformadas pela inversa-transposta; escalas negativas
///   invertem o enrolamento para manter as normais para fora.
/// * `merge`: solda, entre instâncias *consecutivas* (e com as tampas), cada
///   vértice da instância seguinte ao vértice mais próximo da anterior dentro de
///   `merge_distance`; o vértice da instância anterior prevalece. Faces coincidentes
///   internas não são removidas (como no Blender).
/// * Erros: `count` fora de `1..=100000`, parâmetros não finitos, `scale_step`
///   quase zero, `merge_distance` negativa, `fit` inválido (comprimento ≤ 0 ou passo
///   nulo) ou resultado acima de [`MAX_FACES`].
pub fn array(mesh: &EditMesh, params: &ArrayParams) -> ModelResult<EditMesh> {
    params.validate()?;
    mesh.validate()?;
    for cap in [&params.start_cap, &params.end_cap].into_iter().flatten() {
        cap.validate()?;
    }
    let (lo, hi) = mesh.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let size = hi - lo;
    let d = params.relative_offset * size + params.constant_offset;
    let pivot = params.pivot.unwrap_or((lo + hi) * 0.5);

    let count = match params.fit {
        Some(ArrayFit::Length(l)) => {
            let step = d.length();
            if step < 1e-9 {
                return Err(ModelError::InvalidParameter(
                    "fit Length exige deslocamento por instância não nulo".into(),
                ));
            }
            let c = (l / step).floor();
            if c > MAX_COUNT as f32 {
                return Err(ModelError::InvalidParameter(format!(
                    "fit Length geraria mais de {MAX_COUNT} cópias"
                )));
            }
            (c as u32).max(1)
        }
        None => params.count,
    };

    let cap_faces = |c: &Option<EditMesh>| c.as_ref().map_or(0, |m| m.faces.len());
    let total_faces = (count as usize)
        .saturating_mul(mesh.faces.len())
        .saturating_add(cap_faces(&params.start_cap))
        .saturating_add(cap_faces(&params.end_cap));
    if total_faces > MAX_FACES {
        return Err(ModelError::InvalidParameter(format!(
            "array geraria {total_faces} faces (limite {MAX_FACES})"
        )));
    }

    let rot = Mat4::from_euler(EulerRot::XYZ, params.rotation_step.x, params.rotation_step.y, params.rotation_step.z);
    let step = Mat4::from_translation(d)
        * Mat4::from_translation(pivot)
        * rot
        * Mat4::from_scale(params.scale_step)
        * Mat4::from_translation(-pivot);

    // instâncias na ordem lógica: [start_cap?, 0..count, end_cap?]
    let mut parts: Vec<EditMesh> = Vec::with_capacity(count as usize + 2);
    let mut m = Mat4::IDENTITY;
    let mut inst_mats: Vec<Mat4> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        inst_mats.push(m);
        m *= step;
    }
    let end_mat = m;
    for mat in &inst_mats {
        parts.push(transformed(mesh, mat));
    }
    let start_idx = params.start_cap.as_ref().map(|c| {
        parts.push(transformed(c, &step.inverse()));
        parts.len() - 1
    });
    let end_idx = params.end_cap.as_ref().map(|c| {
        parts.push(transformed(c, &end_mat));
        parts.len() - 1
    });
    let mut order: Vec<usize> = Vec::new();
    order.extend(start_idx);
    order.extend(0..count as usize);
    order.extend(end_idx);

    // concatena e registra o deslocamento de vértices de cada parte
    let mut joined = EditMesh::new(mesh.name.clone());
    let mut offsets: Vec<u32> = Vec::with_capacity(parts.len());
    for p in &parts {
        offsets.push(joined.verts.len() as u32);
        joined.append(p);
    }
    let total_verts = joined.verts.len();
    let mut target: Vec<u32> = (0..total_verts as u32).collect();

    if params.merge && order.len() > 1 {
        let mut pairs: Vec<(usize, usize)> = order.windows(2).map(|w| (w[0], w[1])).collect();
        if params.merge_ends && count > 1 {
            // a primeira cópia prevalece sobre a última
            pairs.push((0, count as usize - 1));
        }
        for (a, b) in pairs {
            merge_parts(&joined, &parts, &offsets, a, b, params.merge_distance, &mut target);
        }
    }

    // resolve cadeias de solda até a raiz (as atribuições nunca criam ciclos)
    for v in 0..total_verts {
        let mut r = target[v];
        while target[r as usize] != r {
            r = target[r as usize];
        }
        target[v] = r;
    }
    if target.iter().enumerate().all(|(i, t)| *t as usize == i) {
        return Ok(joined);
    }
    // compacta
    let mut new_id = vec![u32::MAX; total_verts];
    let mut verts = Vec::with_capacity(total_verts);
    for v in 0..total_verts {
        if target[v] as usize == v {
            new_id[v] = verts.len() as u32;
            verts.push(joined.verts[v].clone());
        }
    }
    let map = |v: u32| new_id[target[v as usize] as usize];
    let mut faces: Vec<Face> = Vec::with_capacity(joined.faces.len());
    for f in &joined.faces {
        let mut corners: Vec<Corner> = Vec::with_capacity(f.corners.len());
        for c in &f.corners {
            let nv = map(c.vert);
            if corners.last().is_some_and(|l| l.vert == nv) {
                continue;
            }
            corners.push(Corner { vert: nv, ..*c });
        }
        while corners.len() > 1 && corners[0].vert == corners[corners.len() - 1].vert {
            corners.pop();
        }
        if corners.len() >= 3 {
            let mut nf = f.clone();
            nf.corners = corners;
            faces.push(nf);
        }
    }
    let mut edge_attrs: BTreeMap<(u32, u32), EdgeAttr> = BTreeMap::new();
    for (&(a, b), attr) in &joined.edge_attrs {
        let (na, nb) = (map(a), map(b));
        if na == nb {
            continue;
        }
        let key = edge_key(na, nb);
        let merged = edge_attrs.get(&key).map_or(*attr, |old| combine(*old, *attr));
        edge_attrs.insert(key, merged);
    }
    Ok(EditMesh {
        name: joined.name,
        verts,
        faces,
        edge_attrs,
    })
}

/// Aponta cada vértice da parte `b` para o vértice mais próximo (dentro de
/// `dist`) da parte `a`, resolvendo cadeias de solda já registradas.
fn merge_parts(
    joined: &EditMesh,
    parts: &[EditMesh],
    offsets: &[u32],
    a: usize,
    b: usize,
    dist: f32,
    target: &mut [u32],
) {
    let cell = dist.max(1e-6) as f64;
    let key = |p: Vec3| -> (i64, i64, i64) {
        (
            (p.x as f64 / cell).floor() as i64,
            (p.y as f64 / cell).floor() as i64,
            (p.z as f64 / cell).floor() as i64,
        )
    };
    let d2 = (dist as f64) * (dist as f64);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for i in 0..parts[a].verts.len() as u32 {
        let g = offsets[a] + i;
        grid.entry(key(joined.verts[g as usize].co)).or_default().push(g);
    }
    for j in 0..parts[b].verts.len() as u32 {
        let g = offsets[b] + j;
        if target[g as usize] != g {
            continue;
        }
        let p = joined.verts[g as usize].co;
        let (cx, cy, cz) = key(p);
        let mut best: Option<(f64, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &w in list {
                            let dd = (p.as_dvec3() - joined.verts[w as usize].co.as_dvec3()).length_squared();
                            let better = match best {
                                None => true,
                                Some((bd, bw)) => dd < bd || (dd == bd && w < bw),
                            };
                            if dd <= d2 && better {
                                best = Some((dd, w));
                            }
                        }
                    }
                }
            }
        }
        if let Some((_, w)) = best {
            // resolve a cadeia para a raiz
            let mut root = w;
            while target[root as usize] != root {
                root = target[root as usize];
            }
            if root != g {
                target[g as usize] = root;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::topology::Topology;

    fn p(count: u32) -> ArrayParams {
        ArrayParams {
            count,
            ..ArrayParams::default()
        }
    }

    fn volume(m: &EditMesh) -> f64 {
        let mut vol = 0.0f64;
        for f in &m.faces {
            let q: Vec<glam::DVec3> = f.corners.iter().map(|c| m.verts[c.vert as usize].co.as_dvec3()).collect();
            for i in 1..q.len() - 1 {
                vol += q[0].dot(q[i].cross(q[i + 1])) / 6.0;
            }
        }
        vol
    }

    fn face_centroid_of_component(m: &EditMesh, comp: u32, comps: &[u32]) -> Vec3 {
        let mut acc = Vec3::ZERO;
        let mut n = 0.0;
        for f in 0..m.face_count() as u32 {
            if comps[f as usize] == comp {
                acc += m.face_center(f);
                n += 1.0;
            }
        }
        acc / n
    }

    #[test]
    fn relative_offset_places_touching_copies() {
        let c = primitives::cube(1.0);
        let m = array(&c, &p(3)).unwrap();
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 24);
        assert_eq!(m.face_count(), 18);
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.x + 0.5).abs() < 1e-6 && (hi.x - 2.5).abs() < 1e-6);
        // índices da malha original preservados
        assert_eq!(&m.verts[..8], &c.verts[..]);
        let merged = array(
            &c,
            &ArrayParams {
                merge: true,
                ..p(3)
            },
        )
        .unwrap();
        assert_eq!(merged.vert_count(), 8 * 3 - 4 * 2);
        merged.validate().unwrap();
        assert_eq!(merged.face_count(), 18, "faces internas permanecem");
        // faces internas se anulam no volume com sinal: 3 cubos unitários
        assert!((volume(&merged) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn constant_offset_and_count_one() {
        let c = primitives::cube(1.0);
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                constant_offset: Vec3::new(2.0, 0.0, 0.0),
                ..p(4)
            },
        )
        .unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.x + 0.5).abs() < 1e-6 && (hi.x - 6.5).abs() < 1e-6);
        assert_eq!(Topology::build(&m).face_components(&m).1, 4);
        // count 1 = clone
        assert_eq!(array(&c, &p(1)).unwrap(), c);
    }

    #[test]
    fn fit_length_computes_count() {
        let c = primitives::cube(1.0);
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::new(2.0, 0.0, 0.0),
                fit: Some(ArrayFit::Length(10.0)),
                ..p(99)
            },
        )
        .unwrap();
        assert_eq!(m.vert_count(), 8 * 5);
        // comprimento menor que um passo: pelo menos 1 cópia
        let one = array(
            &c,
            &ArrayParams {
                fit: Some(ArrayFit::Length(0.1)),
                ..p(5)
            },
        )
        .unwrap();
        assert_eq!(one.vert_count(), 8);
        // passo nulo e comprimento inválido
        for bad in [
            ArrayParams {
                relative_offset: Vec3::ZERO,
                fit: Some(ArrayFit::Length(5.0)),
                ..p(2)
            },
            ArrayParams {
                fit: Some(ArrayFit::Length(-1.0)),
                ..p(2)
            },
            ArrayParams {
                fit: Some(ArrayFit::Length(f32::NAN)),
                ..p(2)
            },
            ArrayParams {
                constant_offset: Vec3::new(1e-6, 0.0, 0.0),
                relative_offset: Vec3::ZERO,
                fit: Some(ArrayFit::Length(1e9)),
                ..p(2)
            },
        ] {
            assert!(matches!(array(&c, &bad), Err(ModelError::InvalidParameter(_))));
        }
    }

    #[test]
    fn rotation_around_origin_forms_a_ring() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|q| q + Vec3::new(3.0, 0.0, 0.0));
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                rotation_step: Vec3::new(0.0, std::f32::consts::TAU / 8.0, 0.0),
                pivot: Some(Vec3::ZERO),
                ..p(8)
            },
        )
        .unwrap();
        let (comps, n) = Topology::build(&m).face_components(&m);
        assert_eq!(n, 8);
        for i in 0..n {
            let ctr = face_centroid_of_component(&m, i, &comps);
            assert!((ctr.length() - 3.0).abs() < 1e-4, "{ctr:?}");
            assert!(ctr.y.abs() < 1e-5);
        }
        // todas as cópias são rotações rígidas: mesmo volume
        assert_outward_per_component(&m);
    }

    fn assert_outward_per_component(m: &EditMesh) {
        let (comps, n) = Topology::build(m).face_components(m);
        for i in 0..n {
            let ctr = face_centroid_of_component(m, i, &comps);
            for f in 0..m.face_count() as u32 {
                if comps[f as usize] == i {
                    assert!(m.face_normal(f).dot(m.face_center(f) - ctr) > 0.0, "componente {i} face {f}");
                }
            }
        }
    }

    #[test]
    fn default_pivot_rotates_about_bbox_center() {
        let c = primitives::cube(1.0);
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                constant_offset: Vec3::new(5.0, 0.0, 0.0),
                rotation_step: Vec3::new(0.0, std::f32::consts::FRAC_PI_4, 0.0),
                ..p(2)
            },
        )
        .unwrap();
        // segunda cópia: cubo girado 45° em torno do próprio centro, em x = 5
        let second = &m.verts[8..16];
        let ctr: Vec3 = second.iter().map(|v| v.co).sum::<Vec3>() / 8.0;
        assert!((ctr - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-5);
        let r = second.iter().map(|v| (v.co - ctr).length()).fold(0.0, f32::max);
        assert!((r - 0.75f32.sqrt()).abs() < 1e-5);
        let ext = second.iter().map(|v| (v.co - ctr).x.abs()).fold(0.0, f32::max);
        assert!((ext - 0.5 * 2.0f32.sqrt()).abs() < 1e-5, "girou 45°");
    }

    #[test]
    fn scale_step_accumulates_and_negative_scale_keeps_orientation() {
        let c = primitives::cube(2.0);
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                constant_offset: Vec3::new(10.0, 0.0, 0.0),
                scale_step: Vec3::splat(0.5),
                pivot: Some(Vec3::ZERO),
                ..p(3)
            },
        )
        .unwrap();
        // volumes 8, 8*(1/8), 8*(1/64)? o deslocamento também escala (M^i)
        let (comps, n) = Topology::build(&m).face_components(&m);
        assert_eq!(n, 3);
        let mut sizes: Vec<f32> = (0..n)
            .map(|i| {
                let vs: Vec<Vec3> = (0..m.face_count() as u32)
                    .filter(|f| comps[*f as usize] == i)
                    .flat_map(|f| m.face_positions(f))
                    .collect();
                let lo = vs.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
                let hi = vs.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
                (hi - lo).x
            })
            .collect();
        sizes.sort_by(f32::total_cmp);
        assert!((sizes[0] - 0.5).abs() < 1e-5 && (sizes[1] - 1.0).abs() < 1e-5 && (sizes[2] - 2.0).abs() < 1e-5);

        // escala negativa: cópia espelhada com normais ainda para fora
        let mirrored = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                constant_offset: Vec3::new(5.0, 0.0, 0.0),
                scale_step: Vec3::new(-1.0, 1.0, 1.0),
                pivot: Some(Vec3::ZERO),
                ..p(2)
            },
        )
        .unwrap();
        assert_outward_per_component(&mirrored);
    }

    #[test]
    fn caps_are_placed_before_and_after() {
        let base = primitives::cube(1.0);
        let mut cap = primitives::cube(0.5);
        cap.name = "cap".into();
        let m = array(
            &base,
            &ArrayParams {
                start_cap: Some(cap.clone()),
                end_cap: Some(cap.clone()),
                ..p(2)
            },
        )
        .unwrap();
        assert_eq!(m.vert_count(), 8 * 4);
        let (lo, hi) = m.bounds().unwrap();
        // tampa inicial em -1, final em 2 (centros)
        assert!((lo.x - (-1.0 - 0.25)).abs() < 1e-5 && (hi.x - (2.0 + 0.25)).abs() < 1e-5, "{lo:?} {hi:?}");
        // com merge a tampa menor não coincide (vértices a 0.25 de distância): nada solda
        let mm = array(
            &base,
            &ArrayParams {
                start_cap: Some(cap.clone()),
                merge: true,
                ..p(2)
            },
        )
        .unwrap();
        // 4 vértices soldados entre as cópias 0 e 1; a tampa pequena não toca em nada
        assert_eq!(mm.vert_count(), 8 * 3 - 4);
        // tampa igual à base soldada
        let same = array(
            &base,
            &ArrayParams {
                end_cap: Some(base.clone()),
                merge: true,
                ..p(2)
            },
        )
        .unwrap();
        assert_eq!(same.vert_count(), 8 * 3 - 8);
    }

    #[test]
    fn merge_ends_closes_a_ring() {
        let mut quad = EditMesh::new("quad");
        for q in [
            Vec3::new(0.5, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -0.5),
        ] {
            quad.add_vert(q);
        }
        quad.add_face(&[0, 1, 2, 3]).unwrap();
        let base = ArrayParams {
            relative_offset: Vec3::ZERO,
            rotation_step: Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
            pivot: Some(Vec3::ZERO),
            merge: true,
            ..p(4)
        };
        let open = array(&quad, &base).unwrap();
        assert_eq!(open.vert_count(), 16 - 6);
        let ring = array(
            &quad,
            &ArrayParams {
                merge_ends: true,
                ..base.clone()
            },
        )
        .unwrap();
        assert_eq!(ring.vert_count(), 8);
        assert_eq!(ring.face_count(), 4);
        let t = Topology::build(&ring);
        assert_eq!(t.euler_characteristic(&ring), 0);
        assert_eq!(t.boundary_edge_count(), 8);
        ring.validate().unwrap();
        // todas as faces do anel apontam para o mesmo lado
        assert!(ring.face_normal(0).dot(ring.face_normal(2)) > 0.99);
    }

    #[test]
    fn attributes_and_custom_normals_follow_instances() {
        let mut c = primitives::cube(1.0);
        c.set_seam(0, 1, true);
        c.set_crease(0, 1, 0.5);
        c.faces[0].material = 3;
        c.faces[0].corners[0].normal = Some(Vec3::X);
        c.verts[2].mask = 0.8;
        let m = array(
            &c,
            &ArrayParams {
                relative_offset: Vec3::ZERO,
                constant_offset: Vec3::new(0.0, 3.0, 0.0),
                rotation_step: Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),
                ..p(2)
            },
        )
        .unwrap();
        assert!(m.edge_attr(8, 9).seam && (m.edge_attr(8, 9).crease - 0.5).abs() < 1e-6);
        assert_eq!(m.faces[6].material, 3);
        assert_eq!(m.faces[6].corners[0].uv, c.faces[0].corners[0].uv);
        assert_eq!(m.verts[10].mask, 0.8);
        // girou 90° em Z: X -> Y
        let n = m.faces[6].corners[0].normal.unwrap();
        assert!((n - Vec3::Y).length() < 1e-5, "{n:?}");
        assert_eq!(m.faces[0].corners[0].normal, Some(Vec3::X));
    }

    #[test]
    fn invalid_parameters() {
        let c = primitives::cube(1.0);
        let before = c.clone();
        let bad: Vec<ArrayParams> = vec![
            p(0),
            p(MAX_COUNT + 1),
            ArrayParams {
                merge_distance: -1.0,
                ..p(2)
            },
            ArrayParams {
                relative_offset: Vec3::new(f32::NAN, 0.0, 0.0),
                ..p(2)
            },
            ArrayParams {
                scale_step: Vec3::new(1.0, 0.0, 1.0),
                ..p(2)
            },
            ArrayParams {
                pivot: Some(Vec3::INFINITY),
                ..p(2)
            },
            ArrayParams {
                rotation_step: Vec3::NAN,
                ..p(2)
            },
        ];
        for b in bad {
            assert!(matches!(array(&c, &b), Err(ModelError::InvalidParameter(_))), "{b:?}");
        }
        // limite de faces
        let big = primitives::grid(100, 100, 1.0, 1.0);
        assert!(array(&big, &p(MAX_COUNT)).is_err());
        // tampa inválida
        let mut broken_cap = primitives::cube(1.0);
        broken_cap.faces[0].corners[0].vert = 99;
        assert!(array(
            &c,
            &ArrayParams {
                end_cap: Some(broken_cap),
                ..p(2)
            }
        )
        .is_err());
        assert_eq!(c, before);
        // malha vazia
        let e = EditMesh::new("e");
        assert_eq!(array(&e, &p(3)).unwrap().vert_count(), 0);
    }

    #[test]
    fn staircase_helix_character_case_and_determinism() {
        // degrau de escada em caracol: sobe 0.2 e gira 15° a cada degrau
        let mut step = primitives::cube(1.0);
        step.transform_positions(|q| Vec3::new(q.x * 1.0 + 1.5, q.y * 0.2, q.z * 0.5));
        let params = ArrayParams {
            relative_offset: Vec3::ZERO,
            constant_offset: Vec3::new(0.0, 0.2, 0.0),
            rotation_step: Vec3::new(0.0, 15.0f32.to_radians(), 0.0),
            pivot: Some(Vec3::ZERO),
            ..p(24)
        };
        let a = array(&step, &params).unwrap();
        let b = array(&step, &params).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        let (lo, hi) = a.bounds().unwrap();
        assert!(hi.y - lo.y > 4.0 && hi.y - lo.y < 5.2);
        // após 24 passos de 15° (360°) a última cópia fica por cima da primeira em XZ
        let first: Vec3 = a.verts[..8].iter().map(|v| v.co).sum::<Vec3>() / 8.0;
        let last: Vec3 = a.verts[23 * 8..24 * 8].iter().map(|v| v.co).sum::<Vec3>() / 8.0;
        assert!((last.y - first.y - 23.0 * 0.2).abs() < 1e-3);
        // 23 passos de 15° = 345° = -15°
        let ang = |v: Vec3| v.z.atan2(v.x);
        let diff = (ang(last) - ang(first)).rem_euclid(std::f32::consts::TAU);
        assert!((diff - 15.0f32.to_radians()).abs() < 1e-3 || (diff - (std::f32::consts::TAU - 345.0f32.to_radians())).abs() < 1e-3, "{diff}");
        assert_outward_per_component(&a);
    }

    #[test]
    fn large_mesh_with_merge_is_fast() {
        let g = primitives::grid(100, 100, 1.0, 1.0);
        let t = std::time::Instant::now();
        let m = array(
            &g,
            &ArrayParams {
                relative_offset: Vec3::new(1.0, 0.0, 0.0),
                merge: true,
                ..p(4)
            },
        )
        .unwrap();
        assert_eq!(m.vert_count(), 101 * 101 * 4 - 101 * 3);
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }

    #[test]
    fn serde_roundtrip_including_caps() {
        let params = ArrayParams {
            start_cap: Some(primitives::cube(0.5)),
            fit: Some(ArrayFit::Length(3.0)),
            pivot: Some(Vec3::ONE),
            ..p(3)
        };
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(serde_json::from_str::<ArrayParams>(&json).unwrap(), params);
    }
}
