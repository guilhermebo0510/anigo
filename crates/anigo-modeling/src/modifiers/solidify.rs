//! Modificador **Solidify**: dá espessura a uma superfície (roupas, cartões de
//! cabelo, folhas, cascas), equivalente ao Solidify Modifier do Blender.
//!
//! A malha vira duas camadas ligadas por uma borda ("rim") nas arestas de
//! fronteira:
//!
//! * camada `A`: vértices originais (mesmos índices) deslocados para
//!   `p + n · t · (1 + offset)/2`;
//! * camada `B`: cópias dos vértices (índices `n..2n`, na ordem dos originais)
//!   em `p + n · t · (offset − 1)/2`;
//! * `offset = −1` mantém a superfície original como face externa e cresce para
//!   dentro (padrão do Blender); `+1` cresce para fora; `0` é centrado.
//!
//! As normais de vértice vêm de [`crate::normals::compute_vertex_normals`].
//! Com `even_thickness` o deslocamento de cada vértice é escalado por
//! `Σc / Σc²` (`c = n_v · n_face` das faces incidentes, ajuste de mínimos
//! quadrados que equivale a `1/cos` quando as faces têm o mesmo ângulo), limitado a
//! [`MAX_EVEN_SCALE`].

use crate::error::{ModelError, ModelResult};
use crate::mesh::{Corner, EditMesh, Face};
use crate::normals::compute_vertex_normals;
use crate::topology::Topology;
use serde::{Deserialize, Serialize};

/// Fator máximo da espessura uniforme (evita picos em ângulos muito agudos).
pub const MAX_EVEN_SCALE: f32 = 4.0;

/// Parâmetros do Solidify.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SolidifyParams {
    /// Espessura (unidades de mundo). Negativa inverte o lado, mantendo a
    /// orientação externa correta. Não pode ser zero.
    pub thickness: f32,
    /// `-1..=1`: onde fica a superfície original dentro da casca.
    pub offset: f32,
    /// Espessura uniforme (corrige quinas por `1/cos`).
    pub even_thickness: bool,
    /// Fecha as bordas das fronteiras com faixas de quads.
    pub rim_fill: bool,
    /// Inverte as normais do resultado (casca "ao contrário").
    pub flip_normals: bool,
    /// Somado ao material das faces da borda (permite material próprio para o rim).
    pub material_offset_rim: u16,
}

impl Default for SolidifyParams {
    fn default() -> Self {
        Self {
            thickness: 0.01,
            offset: -1.0,
            even_thickness: true,
            rim_fill: true,
            flip_normals: false,
            material_offset_rim: 0,
        }
    }
}

impl SolidifyParams {
    fn validate(&self) -> ModelResult<()> {
        if !self.thickness.is_finite() || self.thickness.abs() < 1e-9 {
            return Err(ModelError::InvalidParameter(format!(
                "thickness deve ser finito e diferente de zero, recebeu {}",
                self.thickness
            )));
        }
        if !self.offset.is_finite() || !(-1.0..=1.0).contains(&self.offset) {
            return Err(ModelError::InvalidParameter(format!(
                "offset deve estar em -1..=1, recebeu {}",
                self.offset
            )));
        }
        Ok(())
    }
}

/// Aplica Solidify e devolve a malha nova (a entrada não é alterada).
///
/// * Resultado: `2·V` vértices (camadas A e B; vértices isolados não são
///   duplicados), as faces originais na camada externa, as faces invertidas na
///   interna e, com `rim_fill`, um quad por aresta de fronteira (sombreamento
///   chapado, UV colapsado sobre a aresta, material `+ material_offset_rim`).
///   O resultado é fechado (2-manifold) quando a entrada é manifold com fronteira.
/// * Atributos: UV, vértices (cor/skin/máscara) e `sharp/seam/crease/bevel`
///   são copiados para as duas camadas; normais customizadas da camada interna
///   são negadas.
/// * Erros: `thickness` zero/não finita, `offset` fora de `-1..=1`, malha inválida.
pub fn solidify(mesh: &EditMesh, params: &SolidifyParams) -> ModelResult<EditMesh> {
    params.validate()?;
    mesh.validate()?;
    if mesh.faces.is_empty() {
        return Ok(mesh.clone());
    }
    let n = mesh.verts.len();
    let topo = Topology::build(mesh);
    let normals = compute_vertex_normals(mesh);

    // fator de espessura uniforme por vértice
    let mut factor = vec![1.0f32; n];
    if params.even_thickness {
        let mut sc = vec![0.0f32; n];
        let mut scc = vec![0.0f32; n];
        for (fi, face) in mesh.faces.iter().enumerate() {
            let nf = mesh.face_normal(fi as u32);
            for c in &face.corners {
                let v = c.vert as usize;
                let dot = normals[v].dot(nf);
                sc[v] += dot;
                scc[v] += dot * dot;
            }
        }
        for v in 0..n {
            if scc[v] > 1e-8 && sc[v] > 0.0 {
                factor[v] = (sc[v] / scc[v]).clamp(1.0, MAX_EVEN_SCALE);
            }
        }
    }

    let t = params.thickness;
    let up = t * (1.0 + params.offset) * 0.5;
    let down = t * (params.offset - 1.0) * 0.5;
    let used = mesh.used_verts();

    let mut out = EditMesh::new(mesh.name.clone());
    out.verts = Vec::with_capacity(2 * n);
    for v in 0..n {
        let mut a = mesh.verts[v].clone();
        a.co = mesh.verts[v].co + normals[v] * (up * factor[v]);
        out.verts.push(a);
    }
    for v in 0..n {
        let mut b = mesh.verts[v].clone();
        b.co = mesh.verts[v].co + normals[v] * (down * factor[v]);
        out.verts.push(b);
    }

    // A é a camada "externa" (normal original) quando a espessura é positiva.
    let a_forward = (t >= 0.0) != params.flip_normals;
    let off = n as u32;
    let (fwd, back) = if a_forward { (0, off) } else { (off, 0) };

    out.faces.reserve(mesh.faces.len() * 2);
    for face in &mesh.faces {
        let mut f = face.clone();
        for c in &mut f.corners {
            c.vert += fwd;
        }
        out.faces.push(f);
    }
    for face in &mesh.faces {
        let mut f = face.clone();
        f.corners.reverse();
        for c in &mut f.corners {
            c.vert += back;
            c.normal = c.normal.map(|n| -n);
        }
        out.faces.push(f);
    }

    if params.rim_fill {
        for (fi, face) in mesh.faces.iter().enumerate() {
            let k = face.corners.len();
            for i in 0..k {
                let e = topo.face_edges[fi][i] as usize;
                if topo.edges[e].faces.len() != 1 {
                    continue;
                }
                let (ca, cb) = (face.corners[i], face.corners[(i + 1) % k]);
                let corner = |vert: u32, from: &Corner| Corner {
                    vert,
                    uv: from.uv,
                    normal: None,
                };
                out.faces.push(Face {
                    corners: vec![
                        corner(cb.vert + fwd, &cb),
                        corner(ca.vert + fwd, &ca),
                        corner(ca.vert + back, &ca),
                        corner(cb.vert + back, &cb),
                    ],
                    material: face.material.saturating_add(params.material_offset_rim),
                    smooth: false,
                    face_set: face.face_set,
                    hidden: face.hidden,
                });
            }
        }
    }

    for (&(a, b), attr) in &mesh.edge_attrs {
        out.edge_attrs.insert((a, b), *attr);
        out.edge_attrs.insert((a + off, b + off), *attr);
    }

    // vértices isolados: a camada B só existe para vértices usados
    let mut keep = vec![true; 2 * n];
    keep[n..].copy_from_slice(&used);
    out.compact_verts(&keep);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{self, assert_outward};
    use glam::Vec3;

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

    fn params(t: f32, offset: f32) -> SolidifyParams {
        SolidifyParams {
            thickness: t,
            offset,
            ..SolidifyParams::default()
        }
    }

    #[test]
    fn plane_becomes_closed_slab_with_correct_orientation() {
        let g = primitives::grid(4, 4, 2.0, 2.0);
        for (offset, lo, hi) in [(-1.0, -0.2, 0.0), (1.0, 0.0, 0.2), (0.0, -0.1, 0.1)] {
            let m = solidify(&g, &params(0.2, offset)).unwrap();
            m.validate().unwrap();
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold(), "offset {offset}");
            assert_eq!(t.euler_characteristic(&m), 2);
            assert_outward(&m);
            let (b0, b1) = m.bounds().unwrap();
            assert!((b0.y - lo).abs() < 1e-6 && (b1.y - hi).abs() < 1e-6, "offset {offset}: {b0:?} {b1:?}");
            assert!((volume(&m) - 0.2 * 4.0).abs() < 1e-5, "volume {}", volume(&m));
            // 25 vértices originais x2; faces = 16 + 16 + 16 rim
            assert_eq!(m.vert_count(), 50);
            assert_eq!(m.face_count(), 16 + 16 + 16);
        }
    }

    #[test]
    fn negative_thickness_and_flip_normals() {
        let g = primitives::grid(2, 2, 2.0, 2.0);
        let neg = solidify(&g, &params(-0.2, -1.0)).unwrap();
        assert!(Topology::build(&neg).is_closed_manifold());
        let (lo, hi) = neg.bounds().unwrap();
        assert!(lo.y.abs() < 1e-6 && (hi.y - 0.2).abs() < 1e-6, "cresce para cima");
        assert!(volume(&neg) > 0.0, "orientação externa preservada");
        assert_outward(&neg);

        let flipped = solidify(
            &g,
            &SolidifyParams {
                flip_normals: true,
                ..params(0.2, -1.0)
            },
        )
        .unwrap();
        assert!(Topology::build(&flipped).is_closed_manifold());
        assert!((volume(&flipped) + 0.2 * 4.0).abs() < 1e-5, "volume com sinal invertido");
    }

    #[test]
    fn closed_sphere_makes_two_shells() {
        let s = primitives::quad_sphere(1.0, 12);
        let m = solidify(&s, &params(0.1, -1.0)).unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.face_components(&m).1, 2);
        assert_eq!(t.euler_characteristic(&m), 4);
        let outer = volume(&s);
        let shell = volume(&m);
        let ideal = outer * (1.0 - 0.9f64.powi(3));
        assert!((shell - ideal).abs() / ideal < 0.03, "{shell} vs {ideal}");
        // sem rim_fill numa superfície fechada o resultado é o mesmo
        let m2 = solidify(
            &s,
            &SolidifyParams {
                rim_fill: false,
                ..params(0.1, -1.0)
            },
        )
        .unwrap();
        assert_eq!(m2.face_count(), m.face_count());
    }

    #[test]
    fn even_thickness_offsets_corners_by_one_over_cos() {
        let c = primitives::cube(2.0);
        let even = solidify(&c, &params(0.1, -1.0)).unwrap();
        let plain = solidify(
            &c,
            &SolidifyParams {
                even_thickness: false,
                ..params(0.1, -1.0)
            },
        )
        .unwrap();
        // camada interna: vértice de canto
        let inner_even = even.verts[8].co.abs();
        let inner_plain = plain.verts[8].co.abs();
        assert!((inner_even - Vec3::splat(0.9)).length() < 1e-5, "{inner_even:?}");
        assert!((inner_plain.x - (1.0 - 0.1 / 3.0f32.sqrt())).abs() < 1e-5, "{inner_plain:?}");
        // a espessura entre as faces planas é exatamente 0.1 com even
        let inner_vol = 1.8f64.powi(3);
        assert!((volume(&even) - (8.0 - inner_vol)).abs() < 1e-4, "{}", volume(&even));
    }

    #[test]
    fn open_tube_gets_two_rims_and_stays_manifold() {
        let cyl = primitives::cylinder(1.0, 2.0, 12, 2, false);
        let m = solidify(&cyl, &params(0.1, 0.0)).unwrap();
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 0);
        assert_eq!(m.face_count(), cyl.face_count() * 2 + 2 * 12);
        // sem rim: duas folhas abertas
        let open = solidify(
            &cyl,
            &SolidifyParams {
                rim_fill: false,
                ..params(0.1, 0.0)
            },
        )
        .unwrap();
        assert_eq!(Topology::build(&open).boundary_edge_count(), 4 * 12);
    }

    #[test]
    fn rim_material_uv_and_normals() {
        let mut g = primitives::grid(1, 1, 2.0, 2.0);
        g.faces[0].material = 3;
        g.faces[0].face_set = 4;
        g.faces[0].corners[0].normal = Some(Vec3::Y);
        let m = solidify(
            &g,
            &SolidifyParams {
                material_offset_rim: 2,
                ..params(0.5, -1.0)
            },
        )
        .unwrap();
        assert_eq!(m.face_count(), 6);
        assert_eq!(m.faces[0].material, 3);
        assert_eq!(m.faces[0].corners[0].normal, Some(Vec3::Y));
        assert_eq!(m.faces[1].material, 3);
        assert!(m.faces[1].corners.iter().all(|c| c.normal.is_none() || c.normal == Some(-Vec3::Y)));
        assert!(m.faces[1].corners.iter().any(|c| c.normal == Some(-Vec3::Y)));
        for f in &m.faces[2..] {
            assert_eq!(f.material, 5);
            assert!(!f.smooth);
            assert_eq!(f.face_set, 4);
            // UV da borda colapsa sobre a aresta: dois pares idênticos
            assert_eq!(f.corners[0].uv, f.corners[3].uv);
            assert_eq!(f.corners[1].uv, f.corners[2].uv);
        }
        // material saturante
        let mut hi = g.clone();
        hi.faces[0].material = u16::MAX;
        let m = solidify(
            &hi,
            &SolidifyParams {
                material_offset_rim: 5,
                ..params(0.5, -1.0)
            },
        )
        .unwrap();
        assert_eq!(m.faces[2].material, u16::MAX);
    }

    #[test]
    fn attributes_are_copied_to_both_layers() {
        let mut g = primitives::grid(2, 2, 2.0, 2.0);
        g.set_seam(0, 1, true);
        g.verts[4].mask = 0.7;
        g.verts[4].color = [0.1, 0.2, 0.3, 0.4];
        let m = solidify(&g, &params(0.2, -1.0)).unwrap();
        let nv = g.vert_count() as u32;
        assert!(m.edge_attr(0, 1).seam);
        assert!(m.edge_attr(nv, nv + 1).seam);
        assert_eq!(m.verts[(nv + 4) as usize].mask, 0.7);
        assert_eq!(m.verts[(nv + 4) as usize].color, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn isolated_vertices_are_not_duplicated() {
        let mut g = primitives::grid(1, 1, 1.0, 1.0);
        g.add_vert(Vec3::splat(9.0));
        let m = solidify(&g, &params(0.1, -1.0)).unwrap();
        assert_eq!(m.vert_count(), 4 + 1 + 4);
        m.validate().unwrap();
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let g = primitives::grid(1, 1, 1.0, 1.0);
        for bad in [
            params(0.0, -1.0),
            params(f32::NAN, -1.0),
            params(f32::INFINITY, -1.0),
            params(0.1, 1.5),
            params(0.1, -1.1),
            params(0.1, f32::NAN),
        ] {
            assert!(matches!(solidify(&g, &bad), Err(ModelError::InvalidParameter(_))));
        }
        let mut broken = g.clone();
        broken.faces[0].corners[0].vert = 50;
        assert!(solidify(&broken, &params(0.1, -1.0)).is_err());
        // malha vazia passa direto
        let empty = EditMesh::new("e");
        assert_eq!(solidify(&empty, &params(0.1, -1.0)).unwrap(), empty);
    }

    #[test]
    fn non_manifold_input_does_not_panic() {
        let mut m = EditMesh::new("nm");
        for p in [Vec3::ZERO, Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z] {
            m.add_vert(p);
        }
        m.add_face(&[0, 1, 2]).unwrap();
        m.add_face(&[1, 0, 3]).unwrap();
        m.add_face(&[0, 1, 4]).unwrap();
        let s = solidify(&m, &params(0.1, 0.0)).unwrap();
        s.validate().unwrap();
        assert!(s.verts.iter().all(|v| v.co.is_finite()));
    }

    #[test]
    fn hair_card_strip_character_case_and_determinism() {
        // cartão de cabelo: tira longa e estreita levemente curva
        let mut strip = primitives::grid(2, 20, 0.1, 2.0);
        strip.transform_positions(|p| Vec3::new(p.x, p.y + 0.1 * p.z * p.z, p.z));
        let a = solidify(&strip, &params(0.004, 0.0)).unwrap();
        let b = solidify(&strip, &params(0.004, 0.0)).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        let t = Topology::build(&a);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&a), 2);
    }

    #[test]
    fn large_mesh_is_fast() {
        let g = primitives::grid(150, 150, 2.0, 2.0);
        let t = std::time::Instant::now();
        let m = solidify(&g, &params(0.01, -1.0)).unwrap();
        assert!(Topology::build(&m).is_closed_manifold());
        assert!(t.elapsed().as_secs_f32() < 20.0);
    }
}
