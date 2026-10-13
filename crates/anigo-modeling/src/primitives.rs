//! Primitivas de malha (convenção ANIGO: Y para cima, enrolamento anti-horário
//! visto de fora → normal para fora, igual ao glTF).

use crate::mesh::{Corner, EditMesh, Face};
use glam::{Vec2, Vec3};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

fn set_uvs(mesh: &mut EditMesh, face: u32, uvs: &[Vec2]) {
    for (c, uv) in mesh.faces[face as usize].corners.iter_mut().zip(uvs) {
        c.uv = *uv;
    }
}

/// Plano subdividido no plano XZ (normal +Y), `nx × nz` quads, UV 0..1.
pub fn grid(nx: u32, nz: u32, size_x: f32, size_z: f32) -> EditMesh {
    let nx = nx.max(1);
    let nz = nz.max(1);
    let mut m = EditMesh::new("Grid");
    for iz in 0..=nz {
        for ix in 0..=nx {
            let x = (ix as f32 / nx as f32 - 0.5) * size_x;
            let z = (iz as f32 / nz as f32 - 0.5) * size_z;
            m.add_vert(Vec3::new(x, 0.0, z));
        }
    }
    let idx = |ix: u32, iz: u32| iz * (nx + 1) + ix;
    for iz in 0..nz {
        for ix in 0..nx {
            let f = m
                .add_face(&[idx(ix, iz), idx(ix, iz + 1), idx(ix + 1, iz + 1), idx(ix + 1, iz)])
                .expect("grid quad é sempre válido");
            let (u0, u1) = (ix as f32 / nx as f32, (ix + 1) as f32 / nx as f32);
            let (v0, v1) = (iz as f32 / nz as f32, (iz + 1) as f32 / nz as f32);
            set_uvs(
                &mut m,
                f,
                &[Vec2::new(u0, v0), Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0)],
            );
        }
    }
    m
}

pub fn plane(size: f32) -> EditMesh {
    let mut m = grid(1, 1, size, size);
    m.name = "Plane".into();
    m
}

/// Cubo com 8 vértices compartilhados e 6 quads (sombreamento chapado, como no Blender).
pub fn cube(size: f32) -> EditMesh {
    cube_subdivided(size, 1)
}

/// Cubo com `div` divisões por lado (vértices soldados por posição).
pub fn cube_subdivided(size: f32, div: u32) -> EditMesh {
    let div = div.max(1);
    let h = size * 0.5;
    let mut m = EditMesh::new("Cube");
    let mut lookup: HashMap<[i64; 3], u32> = HashMap::new();
    let quant = |p: Vec3| -> [i64; 3] {
        [
            (p.x * 1.0e5).round() as i64,
            (p.y * 1.0e5).round() as i64,
            (p.z * 1.0e5).round() as i64,
        ]
    };
    // (origem, eixo u, eixo v) com u×v = normal para fora.
    let sides = [
        (Vec3::new(-h, -h, h), Vec3::X, Vec3::Y),   // +Z
        (Vec3::new(h, -h, -h), Vec3::NEG_X, Vec3::Y), // -Z
        (Vec3::new(-h, h, h), Vec3::X, Vec3::NEG_Z),  // +Y
        (Vec3::new(-h, -h, -h), Vec3::X, Vec3::Z),    // -Y
        (Vec3::new(h, -h, h), Vec3::NEG_Z, Vec3::Y),  // +X
        (Vec3::new(-h, -h, -h), Vec3::Z, Vec3::Y),    // -X
    ];
    for (origin, du, dv) in sides {
        let at = |i: u32, j: u32| origin + du * (size * i as f32 / div as f32) + dv * (size * j as f32 / div as f32);
        let mut get = |m: &mut EditMesh, p: Vec3| -> u32 {
            *lookup.entry(quant(p)).or_insert_with(|| m.add_vert(p))
        };
        for j in 0..div {
            for i in 0..div {
                let a = get(&mut m, at(i, j));
                let b = get(&mut m, at(i + 1, j));
                let c = get(&mut m, at(i + 1, j + 1));
                let d = get(&mut m, at(i, j + 1));
                let f = m.add_face(&[a, b, c, d]).expect("quad do cubo válido");
                let (u0, u1) = (i as f32 / div as f32, (i + 1) as f32 / div as f32);
                let (v0, v1) = (j as f32 / div as f32, (j + 1) as f32 / div as f32);
                set_uvs(
                    &mut m,
                    f,
                    &[Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)],
                );
            }
        }
    }
    for f in &mut m.faces {
        f.smooth = false;
    }
    m
}

/// Esfera "quad sphere" (cubo projetado): topologia de quads uniforme, sem
/// polos — a melhor base para cabeças e membros orgânicos (loops limpos).
pub fn quad_sphere(radius: f32, div: u32) -> EditMesh {
    let mut m = cube_subdivided(2.0, div.max(1));
    m.name = "QuadSphere".into();
    m.transform_positions(|p| p.normalize_or_zero() * radius);
    for f in &mut m.faces {
        f.smooth = true;
    }
    m
}

/// Esfera UV com polos triangulares.
pub fn uv_sphere(radius: f32, rings: u32, sectors: u32) -> EditMesh {
    let rings = rings.max(2);
    let sectors = sectors.max(3);
    let mut m = EditMesh::new("UVSphere");
    let top = m.add_vert(Vec3::new(0.0, radius, 0.0));
    for r in 1..rings {
        let phi = PI * r as f32 / rings as f32;
        for s in 0..sectors {
            let theta = TAU * s as f32 / sectors as f32;
            m.add_vert(Vec3::new(
                radius * phi.sin() * theta.cos(),
                radius * phi.cos(),
                -radius * phi.sin() * theta.sin(),
            ));
        }
    }
    let bottom = m.add_vert(Vec3::new(0.0, -radius, 0.0));
    let ring = |r: u32, s: u32| 1 + (r - 1) * sectors + (s % sectors);
    let uv = |s: u32, r: u32| Vec2::new(s as f32 / sectors as f32, 1.0 - r as f32 / rings as f32);
    for s in 0..sectors {
        let f = m.add_face(&[top, ring(1, s), ring(1, s + 1)]).expect("tri polo");
        set_uvs(&mut m, f, &[uv(s, 0), uv(s, 1), uv(s + 1, 1)]);
    }
    for r in 1..rings - 1 {
        for s in 0..sectors {
            let f = m
                .add_face(&[ring(r, s), ring(r + 1, s), ring(r + 1, s + 1), ring(r, s + 1)])
                .expect("quad esfera");
            set_uvs(&mut m, f, &[uv(s, r), uv(s, r + 1), uv(s + 1, r + 1), uv(s + 1, r)]);
        }
    }
    for s in 0..sectors {
        let f = m
            .add_face(&[ring(rings - 1, s + 1), ring(rings - 1, s), bottom])
            .expect("tri polo");
        set_uvs(&mut m, f, &[uv(s + 1, rings - 1), uv(s, rings - 1), uv(s, rings)]);
    }
    m
}

/// Cilindro (eixo Y). `caps`: tampas como n-gons.
pub fn cylinder(radius: f32, height: f32, sectors: u32, rings: u32, caps: bool) -> EditMesh {
    cone_like(radius, radius, height, sectors, rings, caps, "Cylinder")
}

/// Cone truncado (`radius_top` pode ser 0 → ponta com triângulos).
pub fn cone(radius_bottom: f32, radius_top: f32, height: f32, sectors: u32, caps: bool) -> EditMesh {
    cone_like(radius_bottom, radius_top, height, sectors, 1, caps, "Cone")
}

fn cone_like(rb: f32, rt: f32, height: f32, sectors: u32, rings: u32, caps: bool, name: &str) -> EditMesh {
    let sectors = sectors.max(3);
    let rings = rings.max(1);
    let mut m = EditMesh::new(name);
    let half = height * 0.5;
    // anéis de baixo para cima; raio zero colapsa para um único vértice
    let mut ring_ids: Vec<Vec<u32>> = Vec::new();
    for r in 0..=rings {
        let t = r as f32 / rings as f32;
        let radius = rb + (rt - rb) * t;
        let y = -half + height * t;
        if radius.abs() < 1e-7 {
            let v = m.add_vert(Vec3::new(0.0, y, 0.0));
            ring_ids.push(vec![v; sectors as usize]);
        } else {
            ring_ids.push(
                (0..sectors)
                    .map(|s| {
                        let a = TAU * s as f32 / sectors as f32;
                        m.add_vert(Vec3::new(radius * a.cos(), y, -radius * a.sin()))
                    })
                    .collect(),
            );
        }
    }
    for r in 0..rings as usize {
        for s in 0..sectors as usize {
            let s1 = (s + 1) % sectors as usize;
            let (a, b) = (ring_ids[r][s], ring_ids[r][s1]);
            let (c, d) = (ring_ids[r + 1][s1], ring_ids[r + 1][s]);
            let verts: Vec<u32> = if a == b {
                vec![a, c, d]
            } else if c == d {
                vec![a, b, c]
            } else {
                vec![a, b, c, d]
            };
            let f = m.add_face(&verts).expect("lateral do cilindro");
            let (u0, u1) = (s as f32 / sectors as f32, (s + 1) as f32 / sectors as f32);
            let (v0, v1) = (r as f32 / rings as f32, (r + 1) as f32 / rings as f32);
            let uvs: Vec<Vec2> = match verts.len() {
                4 => vec![Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)],
                _ if a == b => vec![Vec2::new(u0, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)],
                _ => vec![Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1)],
            };
            set_uvs(&mut m, f, &uvs);
        }
    }
    if caps {
        if rb.abs() > 1e-7 {
            let verts: Vec<u32> = (0..sectors as usize).rev().map(|s| ring_ids[0][s]).collect();
            m.add_face(&verts).expect("tampa inferior");
        }
        if rt.abs() > 1e-7 {
            let verts: Vec<u32> = (0..sectors as usize).map(|s| ring_ids[rings as usize][s]).collect();
            m.add_face(&verts).expect("tampa superior");
        }
    }
    m
}

/// Toro (anel) no plano XZ.
pub fn torus(major: f32, minor: f32, major_segments: u32, minor_segments: u32) -> EditMesh {
    let ms = major_segments.max(3);
    let ns = minor_segments.max(3);
    let mut m = EditMesh::new("Torus");
    for i in 0..ms {
        let a = TAU * i as f32 / ms as f32;
        for j in 0..ns {
            let b = TAU * j as f32 / ns as f32;
            let r = major + minor * b.cos();
            m.add_vert(Vec3::new(r * a.cos(), minor * b.sin(), -r * a.sin()));
        }
    }
    let id = |i: u32, j: u32| (i % ms) * ns + (j % ns);
    for i in 0..ms {
        for j in 0..ns {
            let f = m
                .add_face(&[id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)])
                .expect("quad do toro");
            let (u0, u1) = (i as f32 / ms as f32, (i + 1) as f32 / ms as f32);
            let (v0, v1) = (j as f32 / ns as f32, (j + 1) as f32 / ns as f32);
            set_uvs(
                &mut m,
                f,
                &[Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)],
            );
        }
    }
    m
}

/// Cápsula (eixo Y): cilindro com hemisférios — base para membros e dedos.
pub fn capsule(radius: f32, height: f32, sectors: u32, cap_rings: u32) -> EditMesh {
    let sectors = sectors.max(3);
    let cap_rings = cap_rings.max(1);
    let mut m = EditMesh::new("Capsule");
    let half = height * 0.5;
    let mut rings: Vec<Vec<u32>> = Vec::new();
    let mut push_ring = |m: &mut EditMesh, y: f32, r: f32| {
        if r.abs() < 1e-7 {
            let v = m.add_vert(Vec3::new(0.0, y, 0.0));
            rings.push(vec![v; sectors as usize]);
        } else {
            rings.push(
                (0..sectors)
                    .map(|s| {
                        let a = TAU * s as f32 / sectors as f32;
                        m.add_vert(Vec3::new(r * a.cos(), y, -r * a.sin()))
                    })
                    .collect(),
            );
        }
    };
    // hemisfério inferior: do polo (-Y) até o equador
    for k in 0..=cap_rings {
        let phi = (PI * 0.5) * k as f32 / cap_rings as f32; // 0 = polo
        push_ring(&mut m, -half - radius * phi.cos(), radius * phi.sin());
    }
    // hemisfério superior: do equador ao polo (+Y)
    for k in 0..=cap_rings {
        let phi = (PI * 0.5) * k as f32 / cap_rings as f32;
        push_ring(&mut m, half + radius * phi.sin(), radius * phi.cos());
    }
    for r in 0..rings.len() - 1 {
        for s in 0..sectors as usize {
            let s1 = (s + 1) % sectors as usize;
            let (a, b) = (rings[r][s], rings[r][s1]);
            let (c, d) = (rings[r + 1][s1], rings[r + 1][s]);
            let verts: Vec<u32> = if a == b {
                vec![a, c, d]
            } else if c == d {
                vec![a, b, c]
            } else {
                vec![a, b, c, d]
            };
            m.add_face(&verts).expect("face da cápsula");
        }
    }
    m
}

/// Círculo como n-gon (ou leque de triângulos se `fan`).
pub fn circle(radius: f32, sectors: u32, fan: bool) -> EditMesh {
    let sectors = sectors.max(3);
    let mut m = EditMesh::new("Circle");
    let ring: Vec<u32> = (0..sectors)
        .map(|s| {
            let a = TAU * s as f32 / sectors as f32;
            m.add_vert(Vec3::new(radius * a.cos(), 0.0, -radius * a.sin()))
        })
        .collect();
    if fan {
        let c = m.add_vert(Vec3::ZERO);
        for s in 0..sectors as usize {
            m.add_face(&[c, ring[s], ring[(s + 1) % sectors as usize]]).expect("fan");
        }
    } else {
        m.add_face(&ring).expect("ngon");
    }
    m
}

/// Constrói uma face de cantos já preparados (útil nos testes dos operadores).
pub fn face_from_corners(corners: Vec<Corner>) -> Face {
    Face {
        corners,
        material: 0,
        smooth: true,
        face_set: 0,
        hidden: false,
    }
}

#[cfg(test)]
pub(crate) fn assert_outward(m: &EditMesh) {
    let c = m.centroid();
    for f in 0..m.face_count() as u32 {
        let n = m.face_normal(f);
        let to_face = m.face_center(f) - c;
        assert!(
            n.dot(to_face) > 0.0,
            "face {f} aponta para dentro (n={n:?}, c→f={to_face:?}) em {}",
            m.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::Topology;

    #[test]
    fn grid_normal_is_plus_y() {
        let m = grid(3, 2, 3.0, 2.0);
        assert_eq!(m.face_count(), 6);
        for f in 0..6 {
            assert!((m.face_normal(f) - Vec3::Y).length() < 1e-5);
        }
    }

    #[test]
    fn cube_and_subdivided_cube_are_closed_and_outward() {
        for div in [1, 2, 4] {
            let m = cube_subdivided(2.0, div);
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold(), "div {div}");
            assert_eq!(m.face_count() as u32, 6 * div * div);
            assert_eq!(t.euler_characteristic(&m), 2);
            assert_outward(&m);
        }
        assert_eq!(cube(1.0).vert_count(), 8);
    }

    #[test]
    fn quad_sphere_has_unit_radius_and_no_triangles() {
        let m = quad_sphere(1.5, 4);
        assert!(m.verts.iter().all(|v| (v.co.length() - 1.5).abs() < 1e-4));
        assert!(m.faces.iter().all(|f| f.len() == 4));
        assert!(Topology::build(&m).is_closed_manifold());
        assert_outward(&m);
    }

    #[test]
    fn uv_sphere_is_closed_with_euler_2() {
        let m = uv_sphere(1.0, 8, 12);
        let t = Topology::build(&m);
        assert!(t.is_closed_manifold());
        assert_eq!(t.euler_characteristic(&m), 2);
        assert_outward(&m);
    }

    #[test]
    fn cylinder_cone_capsule_torus_are_closed_and_outward() {
        for m in [
            cylinder(1.0, 2.0, 8, 2, true),
            cone(1.0, 0.0, 2.0, 8, true),
            cone(1.0, 0.5, 2.0, 8, true),
            capsule(0.5, 1.0, 8, 3),
        ] {
            let t = Topology::build(&m);
            assert!(t.is_closed_manifold(), "{} deveria ser fechado", m.name);
            assert_eq!(t.euler_characteristic(&m), 2, "{}", m.name);
            assert_outward(&m);
        }
        let t = torus(1.0, 0.25, 12, 6);
        let topo = Topology::build(&t);
        assert!(topo.is_closed_manifold());
        assert_eq!(topo.euler_characteristic(&t), 0);
    }

    #[test]
    fn circle_variants() {
        assert_eq!(circle(1.0, 6, false).face_count(), 1);
        assert_eq!(circle(1.0, 6, true).face_count(), 6);
    }
}
