//! Modificador **Mirror** (espelho): equivalente ao Mirror Modifier do Blender
//! (eixos, merge, clipping, bisect, mirror UV), pensado para modelagem simétrica
//! de personagens (modela-se metade do corpo/rosto e o resto é espelhado).
//!
//! Convenções:
//! * Os eixos são processados em sequência X, Y, Z; cada eixo ativo duplica a
//!   malha acumulada até então (2, 4 ou 8 cópias).
//! * O plano de espelho passa por [`MirrorParams::pivot`] e é perpendicular ao eixo.
//! * A cópia espelhada tem o **enrolamento invertido** (a reflexão inverte a
//!   orientação), então as normais continuam para fora.
//! * As normais customizadas por corner são **refletidas** (não negadas).
//! * Vértices originais mantêm seus índices; os vértices espelhados vêm depois,
//!   na ordem dos originais.

use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EdgeAttr, EditMesh, Face};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Tolerância (em unidades de mundo) para considerar um ponto "sobre o plano"
/// no bisect.
const PLANE_EPS: f32 = 1e-6;

/// Parâmetros do espelho.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MirrorParams {
    /// Eixos [X, Y, Z] a espelhar.
    pub axes: [bool; 3],
    /// Solda vértices cuja distância até o próprio reflexo é `<= merge_distance`
    /// (ficam exatamente sobre o plano).
    pub merge: bool,
    /// Distância de solda (par vértice/reflexo), `>= 0`.
    pub merge_distance: f32,
    /// "Clipping": vértices a menos de `merge_distance` do plano são projetados
    /// sobre ele antes de espelhar (mesmo com `merge = false`).
    pub clip: bool,
    /// Espelha a cópia em U e/ou V (`u → 1 − u`, `v → 1 − v`).
    pub mirror_uv: [bool; 2],
    /// Corta a malha no plano e descarta o lado oposto antes de espelhar
    /// (por padrão mantém o lado positivo do eixo; ver `flip`).
    pub bisect: [bool; 3],
    /// Inverte o lado mantido pelo `bisect` (`true` = mantém o lado negativo).
    pub flip: [bool; 3],
    /// Ponto por onde passam os planos de espelho.
    pub pivot: Vec3,
    /// Remove as faces que ficam inteiras sobre o plano (paredes internas):
    /// só tem efeito quando a solda as torna duplicadas.
    pub remove_plane_faces: bool,
    /// Tabela de troca de joints para a cópia espelhada (índice = joint original,
    /// valor = joint do lado oposto, ex.: ossos L ↔ R). Joints fora da tabela
    /// permanecem iguais. `None` mantém os joints.
    pub joint_remap: Option<Vec<u16>>,
}

impl Default for MirrorParams {
    fn default() -> Self {
        Self {
            axes: [true, false, false],
            merge: true,
            merge_distance: 0.001,
            clip: false,
            mirror_uv: [false, false],
            bisect: [false; 3],
            flip: [false; 3],
            pivot: Vec3::ZERO,
            remove_plane_faces: true,
            joint_remap: None,
        }
    }
}

impl MirrorParams {
    /// Espelho simples no eixo X com solda (o caso comum de personagens).
    pub fn x() -> Self {
        Self::default()
    }

    fn validate(&self) -> ModelResult<()> {
        if !self.merge_distance.is_finite() || self.merge_distance < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "merge_distance deve ser finito e >= 0, recebeu {}",
                self.merge_distance
            )));
        }
        if !self.pivot.is_finite() {
            return Err(ModelError::InvalidParameter("pivot não finito".into()));
        }
        Ok(())
    }
}

fn combine_attr(a: EdgeAttr, b: EdgeAttr) -> EdgeAttr {
    EdgeAttr {
        sharp: a.sharp || b.sharp,
        seam: a.seam || b.seam,
        crease: a.crease.max(b.crease),
        bevel_weight: a.bevel_weight.max(b.bevel_weight),
    }
}

/// Espelha `mesh` conforme `params` e devolve uma malha nova (a entrada não é
/// alterada). Sem nenhum eixo ativo devolve um clone.
///
/// Equivale a adicionar um Mirror Modifier com os mesmos ajustes. A seleção/
/// índices: vértices e faces originais mantêm o índice (exceto quando `bisect`
/// remove geometria, caso em que são renumerados); as cópias vêm depois.
/// Faces totalmente contidas no plano de espelho (após solda) são removidas se
/// `remove_plane_faces`.
///
/// Erros: `merge_distance` negativa/não finita, `pivot` não finito ou malha inválida.
pub fn mirror(mesh: &EditMesh, params: &MirrorParams) -> ModelResult<EditMesh> {
    params.validate()?;
    mesh.validate()?;
    let mut cur = mesh.clone();
    for axis in 0..3 {
        if !params.axes[axis] {
            continue;
        }
        if params.bisect[axis] {
            cur = bisect_keep(&cur, axis, params.pivot[axis], params.flip[axis]);
        }
        if params.clip {
            let pv = params.pivot[axis];
            for v in &mut cur.verts {
                if (v.co[axis] - pv).abs() <= params.merge_distance.max(PLANE_EPS) {
                    v.co[axis] = pv;
                }
            }
        }
        cur = mirror_axis(&cur, axis, params);
    }
    Ok(cur)
}

/// Corta `mesh` no plano `coord[axis] = pivot` e mantém o lado positivo
/// (negativo se `flip`). Faces cortadas são recortadas (Sutherland-Hodgman); UV,
/// atributos de vértice e normais customizadas são interpolados.
fn bisect_keep(mesh: &EditMesh, axis: usize, pivot: f32, flip: bool) -> EditMesh {
    let sign = if flip { -1.0 } else { 1.0 };
    let d: Vec<f32> = mesh.verts.iter().map(|v| sign * (v.co[axis] - pivot)).collect();
    let mut verts = mesh.verts.clone();
    let mut cut: HashMap<(u32, u32), u32> = HashMap::new();
    let mut faces: Vec<Face> = Vec::with_capacity(mesh.faces.len());

    for face in &mesh.faces {
        let n = face.corners.len();
        let ds: Vec<f32> = face.corners.iter().map(|c| d[c.vert as usize]).collect();
        if ds.iter().all(|x| *x >= -PLANE_EPS) {
            faces.push(face.clone());
            continue;
        }
        if ds.iter().all(|x| *x <= PLANE_EPS) {
            continue;
        }
        let mut out: Vec<Corner> = Vec::with_capacity(n + 2);
        for i in 0..n {
            let (cc, nc) = (face.corners[i], face.corners[(i + 1) % n]);
            let (dc, dn) = (ds[i], ds[(i + 1) % n]);
            if dc >= -PLANE_EPS {
                out.push(cc);
            }
            let crossing = (dc > PLANE_EPS && dn < -PLANE_EPS) || (dc < -PLANE_EPS && dn > PLANE_EPS);
            if crossing {
                let t = dc / (dc - dn);
                let key = edge_key(cc.vert, nc.vert);
                let idx = *cut.entry(key).or_insert_with(|| {
                    // interpola sempre a partir do menor índice para ser independente da direção
                    let (lo, hi, t_lo) = if cc.vert <= nc.vert { (cc.vert, nc.vert, t) } else { (nc.vert, cc.vert, 1.0 - t) };
                    let mut v = EVert::lerp(&mesh.verts[lo as usize], &mesh.verts[hi as usize], t_lo);
                    v.co[axis] = pivot;
                    verts.push(v);
                    (verts.len() - 1) as u32
                });
                let normal = match (cc.normal, nc.normal) {
                    (Some(a), Some(b)) => a.lerp(b, t).try_normalize(),
                    _ => None,
                };
                out.push(Corner {
                    vert: idx,
                    uv: cc.uv.lerp(nc.uv, t),
                    normal,
                });
            }
        }
        // remove repetições consecutivas (cíclicas)
        out.dedup_by(|a, b| a.vert == b.vert);
        while out.len() > 1 && out[0].vert == out[out.len() - 1].vert {
            out.pop();
        }
        if out.len() >= 3 {
            let mut nf = face.clone();
            nf.corners = out;
            faces.push(nf);
        }
    }

    let mut edge_attrs: BTreeMap<(u32, u32), EdgeAttr> = BTreeMap::new();
    for (&(a, b), attr) in &mesh.edge_attrs {
        if let Some(&m) = cut.get(&(a, b)) {
            let inside = if d[a as usize] > PLANE_EPS { a } else { b };
            edge_attrs.insert(edge_key(inside, m), *attr);
        } else if d[a as usize] >= -PLANE_EPS && d[b as usize] >= -PLANE_EPS {
            edge_attrs.insert((a, b), *attr);
        }
    }

    let mut keep: Vec<bool> = d.iter().map(|x| *x >= -PLANE_EPS).collect();
    keep.resize(verts.len(), true);
    let mut out = EditMesh {
        name: mesh.name.clone(),
        verts,
        faces,
        edge_attrs,
    };
    out.compact_verts(&keep);
    out
}

/// Reflete um vetor no eixo `axis`.
fn reflect(mut v: Vec3, axis: usize) -> Vec3 {
    v[axis] = -v[axis];
    v
}

/// Duplica `mesh` refletida no plano do `axis`, com solda opcional.
fn mirror_axis(mesh: &EditMesh, axis: usize, p: &MirrorParams) -> EditMesh {
    let pv = p.pivot[axis];
    let n = mesh.verts.len();
    let mut out = mesh.clone();

    // quais vértices são soldados com o próprio reflexo
    let merged: Vec<bool> = mesh
        .verts
        .iter()
        .map(|v| p.merge && 2.0 * (v.co[axis] - pv).abs() <= p.merge_distance)
        .collect();
    for (i, m) in merged.iter().enumerate() {
        if *m {
            out.verts[i].co[axis] = pv;
        }
    }

    let remap_joint = |j: u16| -> u16 {
        match &p.joint_remap {
            Some(t) => t.get(j as usize).copied().unwrap_or(j),
            None => j,
        }
    };
    let mut twin: Vec<u32> = Vec::with_capacity(n);
    for i in 0..n {
        if merged[i] {
            twin.push(i as u32);
        } else {
            let mut v = mesh.verts[i].clone();
            v.co[axis] = 2.0 * pv - v.co[axis];
            for j in &mut v.joints {
                *j = remap_joint(*j);
            }
            out.verts.push(v);
            twin.push((out.verts.len() - 1) as u32);
        }
    }

    // faces totalmente sobre o plano (todos os vértices soldados)
    let on_plane: Vec<bool> = mesh
        .faces
        .iter()
        .map(|f| f.corners.iter().all(|c| merged[c.vert as usize]))
        .collect();
    if p.remove_plane_faces && on_plane.iter().any(|x| *x) {
        let mut i = 0;
        out.faces.retain(|_| {
            let keep = !on_plane[i];
            i += 1;
            keep
        });
    }

    for (fi, face) in mesh.faces.iter().enumerate() {
        if on_plane[fi] {
            continue;
        }
        let mut corners: Vec<Corner> = Vec::with_capacity(face.corners.len());
        for c in face.corners.iter().rev() {
            let mut uv = c.uv;
            if p.mirror_uv[0] {
                uv.x = 1.0 - uv.x;
            }
            if p.mirror_uv[1] {
                uv.y = 1.0 - uv.y;
            }
            corners.push(Corner {
                vert: twin[c.vert as usize],
                uv: Vec2::new(uv.x, uv.y),
                normal: c.normal.map(|nrm| reflect(nrm, axis)),
            });
        }
        corners.dedup_by(|a, b| a.vert == b.vert);
        while corners.len() > 1 && corners[0].vert == corners[corners.len() - 1].vert {
            corners.pop();
        }
        if corners.len() < 3 {
            continue;
        }
        let mut nf = face.clone();
        nf.corners = corners;
        out.faces.push(nf);
    }

    for (&(a, b), attr) in &mesh.edge_attrs {
        let (ta, tb) = (twin[a as usize], twin[b as usize]);
        if ta == tb {
            continue;
        }
        let key = edge_key(ta, tb);
        let merged_attr = match out.edge_attrs.get(&key) {
            Some(old) => combine_attr(*old, *attr),
            None => *attr,
        };
        out.edge_attrs.insert(key, merged_attr);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};
    use crate::topology::Topology;

    fn closed(m: &EditMesh) {
        m.validate().unwrap();
        let t = Topology::build(m);
        assert!(t.is_closed_manifold(), "deveria ser fechada/manifold (arestas de fronteira: {})", t.boundary_edge_count());
        assert_eq!(t.euler_characteristic(m), 2);
    }

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

    /// Cubo ocupando x em [0, 2].
    fn half_box() -> EditMesh {
        let mut c = primitives::cube(2.0);
        c.transform_positions(|p| p + Vec3::X);
        c
    }

    #[test]
    fn mirror_of_box_touching_plane_yields_closed_box() {
        let m = mirror(&half_box(), &MirrorParams::default()).unwrap();
        closed(&m);
        // 8 originais + 4 espelhados (os outros 4 estão no plano e foram soldados)
        assert_eq!(m.vert_count(), 12);
        // 6 + 6 faces, menos a face do plano nas duas cópias
        assert_eq!(m.face_count(), 10);
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.x + 2.0).abs() < 1e-6 && (hi.x - 2.0).abs() < 1e-6);
        assert_outward(&m);
        assert!((volume(&m) - 16.0).abs() < 1e-4);
    }

    #[test]
    fn plane_faces_are_kept_when_requested() {
        let p = MirrorParams {
            remove_plane_faces: false,
            ..MirrorParams::default()
        };
        let m = mirror(&half_box(), &p).unwrap();
        // a parede interna original permanece (3 faces na aresta => não-manifold)
        assert_eq!(m.face_count(), 11);
        assert!(Topology::build(&m).non_manifold_edge_count() > 0);
    }

    #[test]
    fn merge_off_duplicates_plane_vertices() {
        let p = MirrorParams {
            merge: false,
            ..MirrorParams::default()
        };
        let m = mirror(&half_box(), &p).unwrap();
        assert_eq!(m.vert_count(), 16);
        assert_eq!(m.face_count(), 12);
        let (_, comps) = Topology::build(&m).face_components(&m);
        assert_eq!(comps, 2);
    }

    #[test]
    fn far_from_plane_gives_disjoint_copy_with_outward_normals() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p + Vec3::new(5.0, 1.0, 0.0));
        let m = mirror(&c, &MirrorParams::default()).unwrap();
        assert_eq!(m.vert_count(), 16);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.face_components(&m).1, 2);
        // cada metade tem normais para fora do seu próprio centróide
        for f in 0..m.face_count() as u32 {
            let n = m.face_normal(f);
            let center = m.face_center(f);
            let half_center = Vec3::new(5.0 * center.x.signum(), 1.0, 0.0);
            assert!(n.dot(center - half_center) > 0.0, "face {f} aponta para dentro");
        }
        assert!((volume(&m) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn bisect_makes_sphere_symmetric_and_closed() {
        let mut s = primitives::quad_sphere(1.0, 3);
        // assimetria proposital: o lado negativo é deformado e deve ser descartado
        for v in &mut s.verts {
            if v.co.x < 0.0 {
                v.co *= 3.0;
            }
        }
        let p = MirrorParams {
            bisect: [true, false, false],
            ..MirrorParams::default()
        };
        let m = mirror(&s, &p).unwrap();
        closed(&m);
        assert_outward(&m);
        let (lo, hi) = m.bounds().unwrap();
        assert!(hi.x <= 1.0 + 1e-5 && lo.x >= -1.0 - 1e-5, "[{lo:?}, {hi:?}]");
        // simetria: cada vértice tem reflexo
        for v in &m.verts {
            let r = Vec3::new(-v.co.x, v.co.y, v.co.z);
            assert!(m.verts.iter().any(|w| (w.co - r).length() < 1e-4), "sem reflexo para {:?}", v.co);
        }
        // flip mantém o outro lado
        let pf = MirrorParams {
            bisect: [true, false, false],
            flip: [true, false, false],
            ..MirrorParams::default()
        };
        let mf = mirror(&s, &pf).unwrap();
        closed(&mf);
        let (lof, _) = mf.bounds().unwrap();
        assert!(lof.x < -2.5, "o lado deformado (x<0) foi mantido: {lof:?}");
    }

    #[test]
    fn bisect_interpolates_uv_and_attributes() {
        let mut g = primitives::grid(2, 2, 2.0, 2.0); // x em [-1,1]; UV u = x/2 + .5
        for v in &mut g.verts {
            v.mask = if v.co.x > 0.0 { 1.0 } else { 0.0 };
        }
        // desloca o plano para cortar o meio de uma coluna de quads
        let p = MirrorParams {
            bisect: [true, false, false],
            pivot: Vec3::new(0.5, 0.0, 0.0),
            merge: true,
            ..MirrorParams::default()
        };
        let cut = bisect_keep(&g, 0, 0.5, false);
        cut.validate().unwrap();
        for f in &cut.faces {
            for c in &f.corners {
                let x = cut.co(c.vert).x;
                assert!(x >= 0.5 - 1e-5);
                assert!((c.uv.x - (x / 2.0 + 0.5)).abs() < 1e-5, "UV interpolado deve acompanhar x");
            }
        }
        // vértices criados sobre o plano têm máscara interpolada entre 0 e 1
        let on_plane: Vec<&EVert> = cut.verts.iter().filter(|v| (v.co.x - 0.5).abs() < 1e-6).collect();
        assert!(!on_plane.is_empty());
        assert!(on_plane.iter().all(|v| (0.0..=1.0).contains(&v.mask)));
        let m = mirror(&g, &p).unwrap();
        m.validate().unwrap();
    }

    #[test]
    fn uv_mirror_flips_u_on_copy_only() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p + Vec3::new(3.0, 0.0, 0.0));
        let p = MirrorParams {
            mirror_uv: [true, false],
            ..MirrorParams::default()
        };
        let m = mirror(&c, &p).unwrap();
        let n = c.face_count();
        for i in 0..n {
            let orig = &c.faces[i];
            let copy = &m.faces[n + i];
            assert_eq!(copy.corners.len(), orig.corners.len());
            for (k, oc) in orig.corners.iter().rev().enumerate() {
                let cc = &copy.corners[k];
                assert!((cc.uv.x - (1.0 - oc.uv.x)).abs() < 1e-6);
                assert!((cc.uv.y - oc.uv.y).abs() < 1e-6);
            }
            // original intacto
            assert_eq!(m.faces[i].corners[0].uv, orig.corners[0].uv);
        }
        // v espelhado também
        let pv = MirrorParams {
            mirror_uv: [false, true],
            ..MirrorParams::default()
        };
        let mv = mirror(&c, &pv).unwrap();
        assert!((mv.faces[n].corners[0].uv.y - (1.0 - c.faces[0].corners[3].uv.y)).abs() < 1e-6);
    }

    #[test]
    fn custom_normals_are_reflected_not_negated() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p + Vec3::new(3.0, 0.0, 0.0));
        let n = Vec3::new(0.6, 0.0, 0.8);
        for f in &mut c.faces {
            for k in &mut f.corners {
                k.normal = Some(n);
            }
        }
        let m = mirror(&c, &MirrorParams::default()).unwrap();
        for f in &m.faces[c.face_count()..] {
            for k in &f.corners {
                let r = k.normal.unwrap();
                assert!((r - Vec3::new(-0.6, 0.0, 0.8)).length() < 1e-6);
            }
        }
        for f in &m.faces[..c.face_count()] {
            assert!(f.corners.iter().all(|k| k.normal == Some(n)));
        }
    }

    #[test]
    fn edge_attributes_are_mirrored_and_combined_on_plane() {
        let mut c = half_box();
        // aresta fora do plano e aresta sobre o plano (x = 0)
        let topo = Topology::build(&c);
        let mut off_plane = None;
        let mut on_plane = None;
        for e in &topo.edges {
            let (a, b) = (c.co(e.verts.0), c.co(e.verts.1));
            if a.x > 1.9 && b.x > 1.9 && off_plane.is_none() {
                off_plane = Some(e.verts);
            }
            if a.x < 0.01 && b.x < 0.01 && on_plane.is_none() {
                on_plane = Some(e.verts);
            }
        }
        let (o, w) = (off_plane.unwrap(), on_plane.unwrap());
        c.set_seam(o.0, o.1, true);
        c.set_crease(o.0, o.1, 0.7);
        c.set_sharp(w.0, w.1, true);
        let m = mirror(&c, &MirrorParams::default()).unwrap();
        let seams = m.edge_attrs.values().filter(|a| a.seam && (a.crease - 0.7).abs() < 1e-6).count();
        assert_eq!(seams, 2, "aresta fora do plano aparece nas duas metades");
        // a aresta no plano continua única e sharp
        let sharp = m.edge_attrs.iter().filter(|(_, a)| a.sharp).count();
        assert_eq!(sharp, 1);
    }

    #[test]
    fn skin_joints_are_remapped_on_copy() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p + Vec3::new(3.0, 0.0, 0.0));
        for v in &mut c.verts {
            v.joints = [1, 2, 9, 0];
            v.weights = [0.5, 0.3, 0.2, 0.0];
        }
        let p = MirrorParams {
            joint_remap: Some(vec![0, 2, 1, 3]), // 1 <-> 2; joint 9 fora da tabela fica igual
            ..MirrorParams::default()
        };
        let m = mirror(&c, &p).unwrap();
        assert!(m.verts[..8].iter().all(|v| v.joints == [1, 2, 9, 0]));
        assert!(m.verts[8..].iter().all(|v| v.joints == [2, 1, 9, 0] && v.weights == [0.5, 0.3, 0.2, 0.0]));
        // sem tabela os joints são idênticos
        let m2 = mirror(&c, &MirrorParams::default()).unwrap();
        assert!(m2.verts.iter().all(|v| v.joints == [1, 2, 9, 0]));
    }

    #[test]
    fn clip_snaps_near_plane_vertices() {
        let mut c = half_box();
        // um vértice a 0.0004 do plano (dentro da distância de merge 0.001)
        for v in &mut c.verts {
            if v.co.x < 0.5 {
                v.co.x = 0.0004;
            }
        }
        let p = MirrorParams {
            clip: true,
            merge: false,
            merge_distance: 0.001,
            ..MirrorParams::default()
        };
        let m = mirror(&c, &p).unwrap();
        assert!(m.verts[..8].iter().filter(|v| v.co.x == 0.0).count() == 4);
        // com merge: soldados e fechados
        let pm = MirrorParams {
            merge: true,
            merge_distance: 0.001,
            ..MirrorParams::default()
        };
        let mm = mirror(&c, &pm).unwrap();
        closed(&mm);
        assert_eq!(mm.vert_count(), 12);
        // fora da distância não solda
        let mut far = half_box();
        for v in &mut far.verts {
            if v.co.x < 0.5 {
                v.co.x = 0.01;
            }
        }
        let mf = mirror(&far, &pm).unwrap();
        assert_eq!(mf.vert_count(), 16);
    }

    #[test]
    fn multiple_axes_make_quadrants() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p + Vec3::new(3.0, 3.0, 3.0));
        let p = MirrorParams {
            axes: [true, true, true],
            ..MirrorParams::default()
        };
        let m = mirror(&c, &p).unwrap();
        assert_eq!(m.vert_count(), 64);
        assert_eq!(m.face_count(), 48);
        assert_eq!(Topology::build(&m).face_components(&m).1, 8);
        for f in 0..m.face_count() as u32 {
            let n = m.face_normal(f);
            let center = m.face_center(f);
            let box_center = Vec3::new(3.0 * center.x.signum(), 3.0 * center.y.signum(), 3.0 * center.z.signum());
            assert!(n.dot(center - box_center) > 0.0, "face {f}");
        }
        // pivô deslocado
        let pp = MirrorParams {
            pivot: Vec3::new(1.0, 0.0, 0.0),
            ..MirrorParams::default()
        };
        let mp = mirror(&primitives::cube(1.0), &pp).unwrap();
        let (lo, hi) = mp.bounds().unwrap();
        assert!((lo.x + 0.5).abs() < 1e-6 && (hi.x - 2.5).abs() < 1e-6);
    }

    #[test]
    fn no_axes_is_noop_and_invalid_params_fail() {
        let c = primitives::cube(1.0);
        let p = MirrorParams {
            axes: [false; 3],
            ..MirrorParams::default()
        };
        assert_eq!(mirror(&c, &p).unwrap(), c);
        for bad in [
            MirrorParams {
                merge_distance: -1.0,
                ..MirrorParams::default()
            },
            MirrorParams {
                merge_distance: f32::NAN,
                ..MirrorParams::default()
            },
            MirrorParams {
                pivot: Vec3::new(f32::INFINITY, 0.0, 0.0),
                ..MirrorParams::default()
            },
        ] {
            assert!(matches!(mirror(&c, &bad), Err(ModelError::InvalidParameter(_))));
        }
        let mut broken = primitives::cube(1.0);
        broken.faces[0].corners[0].vert = 77;
        assert!(mirror(&broken, &MirrorParams::default()).is_err());
    }

    #[test]
    fn bisect_removing_everything_gives_empty_mesh() {
        let mut c = primitives::cube(1.0);
        c.transform_positions(|p| p - Vec3::new(5.0, 0.0, 0.0));
        let p = MirrorParams {
            bisect: [true, false, false],
            ..MirrorParams::default()
        };
        let m = mirror(&c, &p).unwrap();
        assert_eq!(m.face_count(), 0);
        assert_eq!(m.vert_count(), 0);
    }

    #[test]
    fn character_head_workflow_with_capsule() {
        // metade de uma cápsula (corte no plano x = 0) espelhada de volta
        let cap = primitives::capsule(0.5, 1.0, 8, 3);
        let p = MirrorParams {
            bisect: [true, false, false],
            ..MirrorParams::default()
        };
        let m = mirror(&cap, &p).unwrap();
        closed(&m);
        assert_outward(&m);
        assert!((volume(&m) - volume(&cap)).abs() / volume(&cap) < 0.05);
    }

    #[test]
    fn determinism_and_large_mesh() {
        let mut s = primitives::quad_sphere(1.0, 40);
        s.transform_positions(|p| p + Vec3::new(0.2, 0.0, 0.0));
        let p = MirrorParams {
            axes: [true, true, false],
            ..MirrorParams::default()
        };
        let t = std::time::Instant::now();
        let a = mirror(&s, &p).unwrap();
        let b = mirror(&s, &p).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.face_count(), s.face_count() * 4);
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }

    #[test]
    fn serde_roundtrip() {
        let p = MirrorParams {
            axes: [true, true, false],
            joint_remap: Some(vec![1, 0]),
            pivot: Vec3::new(0.1, 0.2, 0.3),
            ..MirrorParams::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<MirrorParams>(&json).unwrap(), p);
    }
}
