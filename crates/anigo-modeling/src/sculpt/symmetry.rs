//! Simetria de escultura: espelhamento X/Y/Z e replicação radial de dabs, mais
//! utilidades topológicas (mapa vértice → espelho e "Symmetrize").
//!
//! Os planos de espelho passam pela origem do objeto (como no Blender, que
//! espelha em torno da origem). Os eixos radiais também.

use crate::bvh::VertexGrid;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, EditMesh};
use crate::sculpt::brush::Dab;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Máximo de repetições radiais por eixo.
pub const MAX_RADIAL: u32 = 64;

/// Configuração de simetria do pincel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symmetry {
    /// Espelhar em X/Y/Z (plano `coordenada = 0`).
    pub mirror: [bool; 3],
    /// Repetições radiais ao redor de cada eixo (0 ou 1 = desligado).
    pub radial: [u32; 3],
    /// Tolerância espacial (unidades do mundo): dabs coincidentes dentro dela
    /// são aplicados uma única vez; vértices a menos dela do plano estão "no plano".
    pub tolerance: f32,
}

impl Default for Symmetry {
    fn default() -> Self {
        Self {
            mirror: [false; 3],
            radial: [0; 3],
            tolerance: 1e-4,
        }
    }
}

impl Symmetry {
    /// Sem simetria.
    pub fn none() -> Self {
        Self::default()
    }

    /// Espelho em X (o caso mais comum em personagens).
    pub fn mirror_x() -> Self {
        Self {
            mirror: [true, false, false],
            ..Self::default()
        }
    }

    /// `true` se algum espelho ou repetição radial está ligado.
    pub fn is_active(&self) -> bool {
        self.mirror.iter().any(|&m| m) || self.radial.iter().any(|&r| r > 1)
    }

    /// Valida os parâmetros (tolerância finita ≥ 0, radial ≤ [`MAX_RADIAL`]).
    pub fn validate(&self) -> ModelResult<()> {
        if !self.tolerance.is_finite() || self.tolerance < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "tolerância de simetria inválida: {}",
                self.tolerance
            )));
        }
        for (i, &r) in self.radial.iter().enumerate() {
            if r > MAX_RADIAL {
                return Err(ModelError::InvalidParameter(format!(
                    "repetição radial do eixo {i} excede {MAX_RADIAL}: {r}"
                )));
            }
        }
        Ok(())
    }

    fn radial_counts(&self) -> [u32; 3] {
        [self.radial[0].max(1), self.radial[1].max(1), self.radial[2].max(1)]
    }

    /// Número máximo de passes (antes de remover duplicatas): `2^espelhos × Πradiais`.
    pub fn max_passes(&self) -> usize {
        let m = self.mirror.iter().filter(|&&b| b).count() as u32;
        let [a, b, c] = self.radial_counts();
        (1usize << m) * (a * b * c) as usize
    }

    /// Gera os dabs simétricos de `dab`. O primeiro elemento é sempre o dab
    /// original (`pass = 0`). Cópias coincidentes com outra (dab sobre o plano
    /// de espelho com normal simétrica, por exemplo) são removidas. `pass`
    /// identifica a combinação espelho/radial e é **estável** entre dabs do
    /// mesmo traço (necessário para o estado de Grab por passe).
    ///
    /// Reflete posição, normal, direção de visão e deltas; a repetição radial
    /// rotaciona os mesmos vetores. Parâmetros inválidos devolvem só o original.
    pub fn symmetric_dabs(&self, dab: &Dab) -> Vec<Dab> {
        let mut out: Vec<Dab> = Vec::with_capacity(self.max_passes().min(512));
        let mut d0 = *dab;
        d0.pass = 0;
        out.push(d0);
        if !self.is_active() || self.validate().is_err() {
            return out;
        }
        let [nx, ny, nz] = self.radial_counts();
        let per_mirror = nx * ny * nz;
        let tol = self.tolerance.max(1e-7);
        let mut mirror_masks: Vec<u32> = vec![0];
        for axis in 0..3 {
            if self.mirror[axis] {
                let existing = mirror_masks.clone();
                mirror_masks.extend(existing.into_iter().map(|m| m | (1 << axis)));
            }
        }
        mirror_masks.sort_unstable();
        for &mask in &mirror_masks {
            for kx in 0..nx {
                for ky in 0..ny {
                    for kz in 0..nz {
                        let pass = mask * per_mirror + (kx * ny + ky) * nz + kz;
                        if pass == 0 {
                            continue;
                        }
                        let xf = |v: Vec3| transform_vec(v, mask, [kx, ky, kz], [nx, ny, nz]);
                        let mut d = *dab;
                        d.pass = pass;
                        d.center = xf(dab.center);
                        d.normal = xf(dab.normal);
                        d.view_dir = xf(dab.view_dir);
                        d.delta = xf(dab.delta);
                        d.total_delta = xf(dab.total_delta);
                        let dup = out.iter().any(|o| {
                            o.center.distance(d.center) <= tol
                                && o.normal.dot(d.normal) > 0.999
                                && o.total_delta.distance(d.total_delta) <= tol.max(1e-5)
                                && o.delta.distance(d.delta) <= tol.max(1e-5)
                        });
                        if !dup {
                            out.push(d);
                        }
                    }
                }
            }
        }
        out
    }
}

/// Aplica espelho (bits de `mask`) e depois rotações radiais ao vetor.
fn transform_vec(mut v: Vec3, mask: u32, k: [u32; 3], n: [u32; 3]) -> Vec3 {
    for axis in 0..3 {
        if mask & (1 << axis) != 0 {
            v[axis] = -v[axis];
        }
    }
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    for axis in 0..3 {
        if n[axis] > 1 && k[axis] > 0 {
            let angle = std::f32::consts::TAU * k[axis] as f32 / n[axis] as f32;
            v = Quat::from_axis_angle(axes[axis], angle) * v;
        }
    }
    v
}

// ------------------------------------------------------------- mapa de espelho

/// Mapa vértice → vértice espelhado em um eixo.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorMap {
    pub axis: usize,
    /// `map[v] = Some(m)`: `m` é o espelho de `v` (`m == v` para vértices no
    /// plano). `None`: sem par dentro da tolerância.
    pub map: Vec<Option<u32>>,
    /// Quantidade de vértices sem par.
    pub unmatched: usize,
}

impl MirrorMap {
    /// `true` se todo vértice tem espelho.
    pub fn is_complete(&self) -> bool {
        self.unmatched == 0
    }

    pub fn mirror_of(&self, v: u32) -> Option<u32> {
        self.map.get(v as usize).copied().flatten()
    }
}

/// Constrói o mapa de espelho de `mesh` no eixo `axis` (0 = X, 1 = Y, 2 = Z)
/// casando cada vértice com o vértice mais próximo da sua posição refletida,
/// dentro de `tolerance`. Pares precisam ser mútuos; os demais ficam `None`.
/// Vértices ocultos entram normalmente. Custo O(n) (grade hash).
pub fn build_mirror_map(mesh: &EditMesh, axis: usize, tolerance: f32) -> ModelResult<MirrorMap> {
    if axis > 2 {
        return Err(ModelError::InvalidParameter(format!("eixo inválido: {axis}")));
    }
    if !(tolerance > 0.0) || !tolerance.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "tolerância deve ser > 0 e finita: {tolerance}"
        )));
    }
    let n = mesh.verts.len();
    let mut map: Vec<Option<u32>> = vec![None; n];
    if n == 0 {
        return Ok(MirrorMap {
            axis,
            map,
            unmatched: 0,
        });
    }
    let grid = VertexGrid::build(mesh, tolerance * 2.0)?;
    for v in 0..n {
        let mut p = mesh.verts[v].co;
        p[axis] = -p[axis];
        if let Some((m, _)) = grid.nearest(p, tolerance) {
            map[v] = Some(m);
        }
    }
    // pares precisam ser mútuos
    let snapshot = map.clone();
    let mut unmatched = 0;
    for v in 0..n {
        if let Some(m) = snapshot[v] {
            if snapshot[m as usize] != Some(v as u32) {
                map[v] = None;
            }
        }
        if map[v].is_none() {
            unmatched += 1;
        }
    }
    Ok(MirrorMap {
        axis,
        map,
        unmatched,
    })
}

/// Qual lado é copiado sobre o outro em [`symmetrize_mesh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymmetrizeDirection {
    /// Lado `coordenada > 0` é a fonte; sobrescreve o lado negativo.
    PositiveToNegative,
    /// Lado `coordenada < 0` é a fonte; sobrescreve o lado positivo.
    NegativeToPositive,
}

/// Simetriza uma malha **topologicamente simétrica**: copia posição, cor, skin,
/// máscara, vinco, atributos de aresta e atributos de face (material, suave,
/// face set) do lado fonte para o lado espelhado, e cola no plano os vértices
/// que estão sobre ele (`coordenada = 0` exato). Devolve quantos vértices
/// mudaram de posição.
///
/// Diferença para o Symmetrize do Blender: não reconstrói topologia. Se algum
/// vértice não tem espelho dentro de `tolerance` devolve
/// `UnsupportedTopology` e a malha fica intacta (valida antes de mutar).
pub fn symmetrize_mesh(
    mesh: &mut EditMesh,
    axis: usize,
    direction: SymmetrizeDirection,
    tolerance: f32,
) -> ModelResult<usize> {
    let mm = build_mirror_map(mesh, axis, tolerance)?;
    if !mm.is_complete() {
        return Err(ModelError::UnsupportedTopology(format!(
            "{} vértice(s) sem espelho no eixo {axis}; a malha precisa ser topologicamente simétrica",
            mm.unmatched
        )));
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Side {
        Source,
        Plane,
        Opposite,
    }
    let sign = match direction {
        SymmetrizeDirection::PositiveToNegative => 1.0f32,
        SymmetrizeDirection::NegativeToPositive => -1.0,
    };
    let n = mesh.verts.len();
    let side: Vec<Side> = (0..n)
        .map(|v| {
            let c = mesh.verts[v].co[axis] * sign;
            if c > tolerance {
                Side::Source
            } else if c < -tolerance {
                Side::Opposite
            } else {
                Side::Plane
            }
        })
        .collect();

    // arestas e faces espelhadas (antes de mexer em posições)
    let edge_updates: Vec<((u32, u32), crate::mesh::EdgeAttr)> = mesh
        .edge_attrs
        .iter()
        .filter_map(|(&(a, b), attr)| {
            let (sa, sb) = (side[a as usize], side[b as usize]);
            if sa == Side::Opposite || sb == Side::Opposite || (sa == Side::Plane && sb == Side::Plane) {
                return None;
            }
            let (ma, mb) = (mm.map[a as usize]?, mm.map[b as usize]?);
            Some((edge_key(ma, mb), *attr))
        })
        .collect();
    let mut face_lookup: HashMap<Vec<u32>, u32> = HashMap::with_capacity(mesh.faces.len());
    for (fi, f) in mesh.faces.iter().enumerate() {
        let mut key: Vec<u32> = f.verts().collect();
        key.sort_unstable();
        face_lookup.entry(key).or_insert(fi as u32);
    }
    let mut face_updates: Vec<(u32, u16, bool, i32)> = Vec::new();
    for f in &mesh.faces {
        let vs: Vec<u32> = f.verts().collect();
        let has_source = vs.iter().any(|&v| side[v as usize] == Side::Source);
        let has_opposite = vs.iter().any(|&v| side[v as usize] == Side::Opposite);
        if !has_source || has_opposite {
            continue;
        }
        let mut mk: Vec<u32> = vs.iter().filter_map(|&v| mm.map[v as usize]).collect();
        mk.sort_unstable();
        if let Some(&target) = face_lookup.get(&mk) {
            face_updates.push((target, f.material, f.smooth, f.face_set));
        }
    }

    // posições + atributos de vértice
    let mut changed = 0usize;
    for v in 0..n {
        match side[v] {
            Side::Source => {
                let Some(m) = mm.map[v] else { continue };
                if m as usize == v {
                    continue;
                }
                let src = mesh.verts[v].clone();
                let mut co = src.co;
                co[axis] = -co[axis];
                let dst = &mut mesh.verts[m as usize];
                if dst.co != co {
                    changed += 1;
                }
                dst.co = co;
                dst.color = src.color;
                dst.joints = src.joints;
                dst.weights = src.weights;
                dst.mask = src.mask;
                dst.crease = src.crease;
            }
            Side::Plane => {
                if mesh.verts[v].co[axis] != 0.0 {
                    mesh.verts[v].co[axis] = 0.0;
                    changed += 1;
                }
            }
            Side::Opposite => {}
        }
    }
    for (k, attr) in edge_updates {
        mesh.edge_attrs.insert(k, attr);
    }
    for (fi, material, smooth, face_set) in face_updates {
        let f = &mut mesh.faces[fi as usize];
        f.material = material;
        f.smooth = smooth;
        f.face_set = face_set;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn dab(center: Vec3, normal: Vec3) -> Dab {
        let mut d = Dab::new(center, normal, 0.3, 0.5);
        d.delta = Vec3::new(0.1, 0.2, 0.3);
        d.total_delta = Vec3::new(0.4, 0.5, 0.6);
        d.view_dir = Vec3::new(0.2, -0.3, -0.9).normalize();
        d
    }

    #[test]
    fn no_symmetry_returns_only_original() {
        let d = dab(Vec3::new(0.5, 0.2, 0.1), Vec3::X);
        let out = Symmetry::none().symmetric_dabs(&d);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], d);
        assert!(!Symmetry::none().is_active());
        assert_eq!(Symmetry::none().max_passes(), 1);
    }

    #[test]
    fn mirror_x_reflects_position_normal_and_vectors() {
        let d = dab(Vec3::new(0.5, 0.2, 0.1), Vec3::new(0.6, 0.0, 0.8));
        let out = Symmetry::mirror_x().symmetric_dabs(&d);
        assert_eq!(out.len(), 2);
        let m = &out[1];
        assert_eq!(m.center, Vec3::new(-0.5, 0.2, 0.1));
        assert_eq!(m.normal, Vec3::new(-0.6, 0.0, 0.8));
        assert_eq!(m.delta, Vec3::new(-0.1, 0.2, 0.3));
        assert_eq!(m.total_delta, Vec3::new(-0.4, 0.5, 0.6));
        assert!((m.view_dir.x + d.view_dir.x).abs() < 1e-6 && m.view_dir.y == d.view_dir.y);
        assert_ne!(m.pass, 0);
        assert_eq!(m.radius, d.radius);
        assert_eq!(m.strength, d.strength);
    }

    #[test]
    fn mirror_combinations_count() {
        let d = dab(Vec3::new(0.5, 0.4, 0.3), Vec3::new(0.3, 0.5, 0.8).normalize());
        let n = |m: [bool; 3]| {
            Symmetry {
                mirror: m,
                ..Default::default()
            }
            .symmetric_dabs(&d)
            .len()
        };
        assert_eq!(n([true, false, false]), 2);
        assert_eq!(n([true, true, false]), 4);
        assert_eq!(n([true, true, true]), 8);
        assert_eq!(n([false, false, true]), 2);
        // todos os passes têm ids distintos
        let s = Symmetry {
            mirror: [true, true, true],
            ..Default::default()
        };
        let mut ids: Vec<u32> = s.symmetric_dabs(&d).iter().map(|x| x.pass).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8);
    }

    #[test]
    fn dab_on_mirror_plane_is_not_duplicated() {
        let d = dab(Vec3::new(0.0, 0.5, 0.5), Vec3::new(0.0, 0.6, 0.8));
        let mut d = d;
        d.delta = Vec3::new(0.0, 0.2, 0.3);
        d.total_delta = Vec3::new(0.0, 0.5, 0.6);
        d.view_dir = Vec3::new(0.0, -0.3, -0.9).normalize();
        assert_eq!(Symmetry::mirror_x().symmetric_dabs(&d).len(), 1);
        // com normal inclinada o dab refletido é diferente e permanece
        let tilted = dab(Vec3::new(0.0, 0.5, 0.5), Vec3::new(0.6, 0.0, 0.8));
        assert_eq!(Symmetry::mirror_x().symmetric_dabs(&tilted).len(), 2);
    }

    #[test]
    fn pass_ids_are_stable_when_dedup_changes() {
        // mesmo passe de espelho recebe o mesmo id dentro e fora do plano
        let s = Symmetry {
            mirror: [true, true, false],
            ..Default::default()
        };
        let off = dab(Vec3::new(0.5, 0.5, 0.2), Vec3::new(0.3, 0.5, 0.8).normalize());
        let mut on = off;
        on.center.x = 0.0;
        on.normal = Vec3::new(0.0, 0.6, 0.8);
        on.delta.x = 0.0;
        on.total_delta.x = 0.0;
        on.view_dir.x = 0.0;
        let ids_off: Vec<u32> = s.symmetric_dabs(&off).iter().map(|d| d.pass).collect();
        let ids_on: Vec<u32> = s.symmetric_dabs(&on).iter().map(|d| d.pass).collect();
        assert_eq!(ids_off.len(), 4);
        assert!(ids_on.len() < 4);
        for id in &ids_on {
            assert!(ids_off.contains(id));
        }
    }

    #[test]
    fn radial_replicates_by_rotation() {
        let s = Symmetry {
            radial: [0, 6, 0],
            ..Default::default()
        };
        let d = dab(Vec3::new(1.0, 0.3, 0.0), Vec3::X);
        let out = s.symmetric_dabs(&d);
        assert_eq!(out.len(), 6);
        for o in &out {
            let r = (o.center.x * o.center.x + o.center.z * o.center.z).sqrt();
            assert!((r - 1.0).abs() < 1e-5 && (o.center.y - 0.3).abs() < 1e-6);
            assert!((o.normal.length() - 1.0).abs() < 1e-5);
        }
        // ângulos múltiplos de 60°
        let mut angles: Vec<i32> = out
            .iter()
            .map(|o| {
                let a = (-o.center.z).atan2(o.center.x).to_degrees().rem_euclid(360.0);
                a.round() as i32 % 360
            })
            .collect();
        angles.sort_unstable();
        assert_eq!(angles, vec![0, 60, 120, 180, 240, 300]);
        // normal acompanha a rotação
        for o in &out {
            let dir = Vec3::new(o.center.x, 0.0, o.center.z).normalize();
            assert!(o.normal.dot(dir) > 0.999);
        }
    }

    #[test]
    fn radial_plus_mirror_multiplies() {
        let s = Symmetry {
            mirror: [true, false, false],
            radial: [0, 3, 0],
            ..Default::default()
        };
        let d = dab(Vec3::new(0.7, 0.3, 0.2), Vec3::new(0.3, 0.4, 0.8).normalize());
        assert_eq!(s.symmetric_dabs(&d).len(), 6);
        assert_eq!(s.max_passes(), 6);
        assert!(s.is_active());
    }

    #[test]
    fn radial_on_axis_dab_collapses() {
        // dab no eixo de rotação com normal ao longo do eixo: todas as cópias coincidem
        let s = Symmetry {
            radial: [0, 4, 0],
            ..Default::default()
        };
        let mut d = dab(Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        d.delta = Vec3::ZERO;
        d.total_delta = Vec3::ZERO;
        d.view_dir = -Vec3::Y;
        assert_eq!(s.symmetric_dabs(&d).len(), 1);
    }

    #[test]
    fn invalid_symmetry_is_rejected_and_degrades_safely() {
        let bad = Symmetry {
            tolerance: f32::NAN,
            mirror: [true, false, false],
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let d = dab(Vec3::new(0.5, 0.0, 0.0), Vec3::X);
        assert_eq!(bad.symmetric_dabs(&d).len(), 1);
        let too_many = Symmetry {
            radial: [0, MAX_RADIAL + 1, 0],
            ..Default::default()
        };
        assert!(too_many.validate().is_err());
        assert!(Symmetry {
            tolerance: -1.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Symmetry::mirror_x().validate().is_ok());
    }

    #[test]
    fn mirror_map_of_symmetric_sphere_is_complete_and_involutive() {
        let m = primitives::quad_sphere(1.0, 6);
        for axis in 0..3 {
            let mm = build_mirror_map(&m, axis, 1e-4).unwrap();
            assert!(mm.is_complete(), "eixo {axis}: {} sem par", mm.unmatched);
            for v in 0..m.vert_count() as u32 {
                let s = mm.mirror_of(v).unwrap();
                assert_eq!(mm.mirror_of(s), Some(v));
                let mut p = m.co(v);
                p[axis] = -p[axis];
                assert!((m.co(s) - p).length() < 1e-4);
            }
        }
        // vértices no plano são seus próprios espelhos
        let mm = build_mirror_map(&m, 0, 1e-4).unwrap();
        let on_plane = (0..m.vert_count() as u32).filter(|&v| m.co(v).x.abs() < 1e-6);
        let mut count = 0;
        for v in on_plane {
            assert_eq!(mm.mirror_of(v), Some(v));
            count += 1;
        }
        assert!(count > 0);
    }

    #[test]
    fn mirror_map_reports_unmatched_and_validates_input() {
        let mut m = primitives::quad_sphere(1.0, 4);
        m.verts[5].co += Vec3::new(0.05, 0.02, 0.0);
        let mm = build_mirror_map(&m, 0, 1e-3).unwrap();
        assert!(!mm.is_complete());
        assert!(mm.unmatched >= 1);
        assert!(mm.mirror_of(5).is_none());
        assert!(build_mirror_map(&m, 3, 1e-3).is_err());
        assert!(build_mirror_map(&m, 0, 0.0).is_err());
        assert!(build_mirror_map(&m, 0, f32::NAN).is_err());
        assert_eq!(mm.mirror_of(9999), None);
        // malha vazia
        let e = build_mirror_map(&EditMesh::new("e"), 0, 1e-3).unwrap();
        assert!(e.is_complete() && e.map.is_empty());
    }

    fn assert_x_symmetric(m: &EditMesh, tol: f32) {
        let mm = build_mirror_map(m, 0, tol).unwrap();
        assert!(mm.is_complete());
    }

    #[test]
    fn symmetrize_positive_to_negative_on_perturbed_half() {
        // constrói pares exatos e perturba o lado positivo DEPOIS do mapa: usa tolerância
        // maior que a perturbação.
        let mut m = primitives::quad_sphere(1.0, 6);
        let originals = m.clone();
        for v in m.verts.iter_mut() {
            if v.co.x > 0.01 {
                v.co += Vec3::new(0.004, -0.003, 0.002);
                v.mask = 0.75;
            }
        }
        let n = symmetrize_mesh(&mut m, 0, SymmetrizeDirection::PositiveToNegative, 0.02).unwrap();
        assert!(n > 0);
        assert_x_symmetric(&m, 1e-5);
        let mm = build_mirror_map(&m, 0, 1e-5).unwrap();
        for v in 0..m.vert_count() as u32 {
            let s = mm.mirror_of(v).unwrap();
            let mut p = m.co(v);
            p.x = -p.x;
            assert!((m.co(s) - p).length() < 1e-6);
            assert_eq!(m.verts[v as usize].mask, m.verts[s as usize].mask);
            // vértices no plano ficam exatamente em x = 0
            if s == v {
                assert_eq!(m.co(v).x, 0.0);
            }
        }
        // o lado positivo é a fonte: não se moveu
        for (a, b) in m.verts.iter().zip(&originals.verts) {
            if a.co.x > 0.02 {
                assert!((a.co - b.co).length() < 0.01);
            }
        }
        m.validate().unwrap();
    }

    #[test]
    fn symmetrize_negative_to_positive_uses_the_other_side() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let neg_ref: Vec<(usize, Vec3)> = m
            .verts
            .iter()
            .enumerate()
            .filter(|(_, v)| v.co.x < -0.01)
            .map(|(i, v)| (i, v.co))
            .collect();
        for v in m.verts.iter_mut() {
            if v.co.x > 0.01 {
                v.co += Vec3::new(0.003, 0.0, 0.0);
            }
        }
        symmetrize_mesh(&mut m, 0, SymmetrizeDirection::NegativeToPositive, 0.02).unwrap();
        for (i, co) in neg_ref {
            assert_eq!(m.co(i as u32), co, "lado fonte intocado");
        }
        let mm = build_mirror_map(&m, 0, 1e-5).unwrap();
        assert!(mm.is_complete());
    }

    #[test]
    fn symmetrize_copies_edge_and_face_attributes() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let topo = crate::topology::Topology::build(&m);
        // marca costura numa aresta totalmente do lado positivo e material numa face
        let e = topo
            .edges
            .iter()
            .find(|e| m.co(e.verts.0).x > 0.2 && m.co(e.verts.1).x > 0.2)
            .unwrap()
            .verts;
        m.set_seam(e.0, e.1, true);
        let fi = (0..m.face_count())
            .find(|&f| m.faces[f].verts().all(|v| m.co(v).x > 0.1))
            .unwrap();
        m.faces[fi].material = 7;
        m.faces[fi].face_set = 3;
        symmetrize_mesh(&mut m, 0, SymmetrizeDirection::PositiveToNegative, 1e-3).unwrap();
        let mm = build_mirror_map(&m, 0, 1e-4).unwrap();
        let (ma, mb) = (mm.mirror_of(e.0).unwrap(), mm.mirror_of(e.1).unwrap());
        assert!(m.edge_attr(ma, mb).seam, "costura espelhada");
        let mut key: Vec<u32> = m.faces[fi].verts().map(|v| mm.mirror_of(v).unwrap()).collect();
        key.sort_unstable();
        let mirrored = m
            .faces
            .iter()
            .find(|f| {
                let mut k: Vec<u32> = f.verts().collect();
                k.sort_unstable();
                k == key
            })
            .unwrap();
        assert_eq!(mirrored.material, 7);
        assert_eq!(mirrored.face_set, 3);
    }

    #[test]
    fn symmetrize_rejects_asymmetric_topology_and_leaves_mesh_intact() {
        let mut m = primitives::quad_sphere(1.0, 4);
        m.verts[7].co += Vec3::new(0.2, 0.1, 0.0);
        let before = m.clone();
        let r = symmetrize_mesh(&mut m, 0, SymmetrizeDirection::PositiveToNegative, 1e-3);
        assert!(matches!(r, Err(ModelError::UnsupportedTopology(_))));
        assert_eq!(m, before);
        assert!(symmetrize_mesh(&mut m, 5, SymmetrizeDirection::PositiveToNegative, 1e-3).is_err());
        assert_eq!(m, before);
        assert!(symmetrize_mesh(&mut m, 0, SymmetrizeDirection::PositiveToNegative, -1.0).is_err());
        assert_eq!(m, before);
    }

    #[test]
    fn symmetrize_is_idempotent_on_symmetric_mesh() {
        let mut m = primitives::quad_sphere(1.0, 6);
        symmetrize_mesh(&mut m, 1, SymmetrizeDirection::PositiveToNegative, 1e-3).unwrap();
        let h = m.content_hash();
        let again = symmetrize_mesh(&mut m, 1, SymmetrizeDirection::PositiveToNegative, 1e-3).unwrap();
        assert_eq!(again, 0);
        assert_eq!(m.content_hash(), h);
    }
}
