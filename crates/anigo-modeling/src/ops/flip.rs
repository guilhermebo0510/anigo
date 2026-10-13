//! Flip Normals, Make Consistent e Recalculate Normals Outside/Inside (`Shift+N`).
//!
//! "Normal" aqui é o enrolamento das faces (anti-horário visto de fora = normal para
//! fora). Inverter uma face reverte a ordem dos cantos (cada corner leva consigo UV
//! e normal customizada, esta última negada) — fazer duas vezes restaura a face
//! exatamente.
//!
//! # Orientação por componente
//!
//! Componentes são ilhas de faces ligadas por arestas **manifold** (exatamente 2
//! faces). Dentro de uma ilha o enrolamento é propagado por BFS a partir da menor
//! face (determinístico). Superfícies não orientáveis (Möbius) e arestas
//! não-manifold não impedem a operação: a árvore do BFS decide, e as arestas que
//! discordam permanecem discordando (melhor esforço, como no Blender).

use crate::error::ModelResult;
use crate::ops::delete::unique_faces;
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::DVec3;
use std::collections::VecDeque;

/// Resultado interno da propagação de enrolamento.
struct Orientation {
    /// Ilha de cada face.
    comp: Vec<u32>,
    n_comp: usize,
    /// `true` = a face precisa ser invertida para ficar consistente com a semente da sua ilha.
    flip: Vec<bool>,
}

/// Direção da aresta `e` dentro da face `f`: `Some(true)` se percorrida do menor
/// para o maior índice de vértice.
fn edge_forward(mesh: &crate::mesh::EditMesh, topo: &Topology, f: u32, e: u32) -> Option<bool> {
    let pos = topo.face_edges[f as usize].iter().position(|&x| x == e)?;
    let face = &mesh.faces[f as usize];
    let n = face.corners.len();
    let (a, b) = (face.corners[pos].vert, face.corners[(pos + 1) % n].vert);
    Some(a < b)
}

fn propagate(mesh: &crate::mesh::EditMesh, topo: &Topology) -> Orientation {
    let nf = mesh.faces.len();
    let mut comp = vec![u32::MAX; nf];
    let mut flip = vec![false; nf];
    let mut n_comp = 0u32;
    for seed in 0..nf {
        if comp[seed] != u32::MAX {
            continue;
        }
        comp[seed] = n_comp;
        let mut queue = VecDeque::from([seed as u32]);
        while let Some(f) = queue.pop_front() {
            for &e in &topo.face_edges[f as usize] {
                let info = &topo.edges[e as usize];
                if info.faces.len() != 2 || info.faces[0] == info.faces[1] {
                    continue;
                }
                let g = if info.faces[0] == f {
                    info.faces[1]
                } else {
                    info.faces[0]
                };
                let (Some(df), Some(dg)) = (
                    edge_forward(mesh, topo, f, e),
                    edge_forward(mesh, topo, g, e),
                ) else {
                    continue;
                };
                // após os flips as direções precisam ser opostas
                let want_flip_g = !(df ^ flip[f as usize]) ^ dg;
                if comp[g as usize] == u32::MAX {
                    comp[g as usize] = n_comp;
                    flip[g as usize] = want_flip_g;
                    queue.push_back(g);
                }
            }
        }
        n_comp += 1;
    }
    Orientation {
        comp,
        n_comp: n_comp as usize,
        flip,
    }
}

fn apply_flips(mesh: &mut crate::mesh::EditMesh, flip: &[bool]) -> Vec<u32> {
    let mut flipped = Vec::new();
    for (f, &fl) in flip.iter().enumerate() {
        if fl {
            let face = &mut mesh.faces[f];
            face.corners.reverse();
            for c in &mut face.corners {
                if let Some(n) = c.normal.as_mut() {
                    *n = -*n;
                }
            }
            flipped.push(f as u32);
        }
    }
    flipped
}

fn output(mesh: &crate::mesh::EditMesh, flipped: Vec<u32>) -> OpOutput {
    OpOutput {
        selection: Selection::from_faces(mesh, flipped),
        ..Default::default()
    }
}

/// *Flip Normals* (`Alt+N → Flip`): inverte o enrolamento das faces dadas (normais
/// customizadas dos corners são negadas). Duas chamadas restauram a malha. Seleção
/// devolvida: as faces invertidas. Erros: lista vazia ou índice inválido (malha intacta).
pub fn flip_faces(mesh: &mut crate::mesh::EditMesh, faces: &[u32]) -> ModelResult<OpOutput> {
    let faces = unique_faces(mesh, faces, "faces")?;
    let mut flip = vec![false; mesh.faces.len()];
    for &f in &faces {
        flip[f as usize] = true;
    }
    let flipped = apply_flips(mesh, &flip);
    Ok(output(mesh, flipped))
}

/// *Make Consistent*: faz o enrolamento coerente dentro de cada ilha (faces ligadas
/// por arestas manifold), tomando a menor face da ilha como referência — não decide
/// qual lado é "fora" (use [`recalc_normals_outside`]). Idempotente. Seleção
/// devolvida: as faces que precisaram ser invertidas (vazia se já consistente).
pub fn make_consistent(mesh: &mut crate::mesh::EditMesh) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let o = propagate(mesh, &topo);
    let flipped = apply_flips(mesh, &o.flip);
    Ok(output(mesh, flipped))
}

/// *Recalculate Normals Outside / Inside*: torna o enrolamento consistente em cada
/// ilha e então orienta cada ilha para fora (`inside = false`) ou para dentro.
///
/// * ilha **fechada** (nenhuma aresta de fronteira): volume com sinal pelo teorema
///   da divergência (somatório de determinantes por triângulo em `f64`); volume
///   positivo ⇒ normais para fora;
/// * ilha **aberta** (ou volume ≈ 0): soma de `área · (normal · (centro da face −
///   centro da ilha))`; positivo ⇒ apontando para fora do centro. Ilhas
///   perfeitamente planas não têm "fora" e ficam com a orientação da menor face;
/// * cada ilha é tratada independentemente: uma casca interna (cavidade) também é
///   orientada "para fora" do seu próprio centro (como no Blender, que decide por ilha).
///
/// Seleção devolvida: as faces invertidas. Determinístico.
pub fn recalc_normals_outside(
    mesh: &mut crate::mesh::EditMesh,
    inside: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let mut o = propagate(mesh, &topo);
    let nf = mesh.faces.len();

    // Agrupa faces por ilha.
    let mut by_comp: Vec<Vec<u32>> = vec![Vec::new(); o.n_comp];
    for f in 0..nf {
        by_comp[o.comp[f] as usize].push(f as u32);
    }
    let mut open = vec![false; o.n_comp];
    for e in &topo.edges {
        if e.faces.len() == 1 {
            open[o.comp[e.faces[0] as usize] as usize] = true;
        }
    }
    for (ci, faces) in by_comp.iter().enumerate() {
        // centro da ilha = média dos centros de face (ponderada igualmente)
        let mut center = DVec3::ZERO;
        for &f in faces {
            center += mesh.face_center(f).as_dvec3();
        }
        center /= faces.len().max(1) as f64;

        let mut volume = 0.0f64;
        let mut score = 0.0f64;
        let mut area_sum = 0.0f64;
        for &f in faces {
            let sign = if o.flip[f as usize] { -1.0 } else { 1.0 };
            let pts: Vec<DVec3> = mesh
                .face_positions(f)
                .iter()
                .map(|p| p.as_dvec3() - center)
                .collect();
            for i in 1..pts.len().saturating_sub(1) {
                volume += sign * pts[0].dot(pts[i].cross(pts[i + 1])) / 6.0;
            }
            let area = mesh.face_area(f) as f64;
            let n = mesh.face_normal(f).as_dvec3() * sign;
            let c = mesh.face_center(f).as_dvec3() - center;
            score += area * n.dot(c);
            area_sum += area;
        }
        let scale = area_sum.max(1e-30).powf(1.5);
        let use_volume = !open[ci] && volume.abs() > 1e-9 * scale;
        let outward_positive = if use_volume {
            volume > 0.0
        } else if score.abs() > 1e-9 * area_sum.max(1e-30) * area_sum.sqrt() {
            score > 0.0
        } else {
            continue; // sem noção de "fora" (ex.: plano)
        };
        // `outward_positive`: com os flips relativos atuais a ilha aponta para fora?
        let want_outward = !inside;
        if outward_positive != want_outward {
            for &f in faces {
                o.flip[f as usize] = !o.flip[f as usize];
            }
        }
    }
    let flipped = apply_flips(mesh, &o.flip);
    Ok(output(mesh, flipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::EditMesh;
    use crate::primitives;
    use crate::primitives::assert_outward;
    use glam::Vec3;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn chance(&mut self, pct: u64) -> bool {
            self.next() % 100 < pct
        }
    }

    fn corrupt(m: &mut EditMesh, pct: u64, seed: u64) -> usize {
        let mut r = Lcg(seed);
        let mut n = 0;
        for f in 0..m.face_count() as u32 {
            if r.chance(pct) {
                m.flip_face(f).unwrap();
                n += 1;
            }
        }
        n
    }

    #[test]
    fn flip_faces_is_an_involution() {
        let mut m = primitives::quad_sphere(1.0, 3);
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.normal = Some(Vec3::Y);
                c.uv = glam::Vec2::new(c.vert as f32, 0.0);
            }
        }
        let h = m.content_hash();
        let snapshot = m.clone();
        let sel: Vec<u32> = (0..m.face_count() as u32).step_by(2).collect();
        let out = flip_faces(&mut m, &sel).unwrap();
        assert_eq!(out.selection.faces.len(), sel.len());
        assert_ne!(m.content_hash(), h);
        assert_eq!(m.faces[0].corners[0].normal, Some(-Vec3::Y));
        flip_faces(&mut m, &sel).unwrap();
        assert_eq!(m, snapshot);
    }

    #[test]
    fn flip_faces_reverses_normal_direction() {
        let mut m = primitives::cube(1.0);
        let n0 = m.face_normal(0);
        flip_faces(&mut m, &[0]).unwrap();
        assert!((m.face_normal(0) + n0).length() < 1e-6);
        // uv do corner acompanha o vértice
        let mut p = primitives::plane(1.0);
        let before: Vec<(u32, glam::Vec2)> =
            p.faces[0].corners.iter().map(|c| (c.vert, c.uv)).collect();
        flip_faces(&mut p, &[0]).unwrap();
        for c in &p.faces[0].corners {
            assert!(before.contains(&(c.vert, c.uv)));
        }
    }

    #[test]
    fn flip_faces_errors() {
        let mut m = primitives::cube(1.0);
        let h = m.content_hash();
        assert!(flip_faces(&mut m, &[]).is_err());
        assert!(flip_faces(&mut m, &[0, 6]).is_err());
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn recalc_outside_repairs_a_sphere_with_thirty_percent_inverted() {
        for (name, orig) in [
            ("quad_sphere", primitives::quad_sphere(1.0, 8)),
            ("uv_sphere", primitives::uv_sphere(1.0, 10, 16)),
            ("cube", primitives::cube(1.0)),
            ("cylinder", primitives::cylinder(1.0, 2.0, 12, 3, true)),
            ("capsule", primitives::capsule(0.5, 1.0, 10, 4)),
            ("torus", primitives::torus(1.0, 0.3, 12, 8)),
        ] {
            for seed in [1u64, 7, 42, 99, 12345] {
                let mut m = orig.clone();
                let n = corrupt(&mut m, 30, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
                let out = recalc_normals_outside(&mut m, false).unwrap();
                assert_eq!(
                    m.content_hash(),
                    orig.content_hash(),
                    "{name} seed {seed} ({n} invertidas)"
                );
                m.validate().unwrap();
                if name != "torus" {
                    assert_outward(&m);
                }
                assert_eq!(out.selection.faces.len(), n);
            }
        }
    }

    #[test]
    fn recalc_outside_handles_global_inversion_and_inside_mode() {
        let orig = primitives::quad_sphere(1.0, 4);
        let mut inv = orig.clone();
        for f in 0..inv.face_count() as u32 {
            inv.flip_face(f).unwrap();
        }
        let mut a = inv.clone();
        recalc_normals_outside(&mut a, false).unwrap();
        assert_eq!(a.content_hash(), orig.content_hash());
        // inside = true: tudo para dentro
        let mut b = orig.clone();
        recalc_normals_outside(&mut b, true).unwrap();
        assert_eq!(b.content_hash(), inv.content_hash());
        // idempotente
        let h = a.content_hash();
        let out = recalc_normals_outside(&mut a, false).unwrap();
        assert_eq!(a.content_hash(), h);
        assert!(out.selection.faces.is_empty());
    }

    #[test]
    fn recalc_outside_works_per_island() {
        let mut m = primitives::cube(1.0);
        let mut s = primitives::quad_sphere(1.0, 4);
        s.transform_positions(|p| p + Vec3::new(5.0, 0.0, 0.0));
        let (cube_faces, sphere_start) = (m.face_count(), m.face_count());
        m.append(&s);
        let reference = m.clone();
        let mut r = Lcg(5);
        for f in 0..m.face_count() as u32 {
            if r.chance(40) {
                m.flip_face(f).unwrap();
            }
        }
        // inverte a esfera inteira para garantir que o sinal é decidido por ilha
        for f in sphere_start..m.face_count() {
            m.flip_face(f as u32).unwrap();
        }
        let _ = cube_faces;
        recalc_normals_outside(&mut m, false).unwrap();
        assert_eq!(m.content_hash(), reference.content_hash());
    }

    #[test]
    fn recalc_outside_open_hemisphere_uses_centroid_rule() {
        let mut m = primitives::uv_sphere(1.0, 8, 12);
        let low: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_center(f).y < -0.05)
            .collect();
        m.delete_faces(&low).unwrap();
        let orig = m.clone();
        corrupt(&mut m, 50, 77);
        recalc_normals_outside(&mut m, false).unwrap();
        // calota superior: normais apontam para longe do centro da ilha (mais para cima que o centro)
        for f in 0..m.face_count() as u32 {
            assert!(m.face_normal(f).dot(m.face_center(f)) > 0.0);
        }
        assert_eq!(m.content_hash(), orig.content_hash());
    }

    #[test]
    fn make_consistent_on_flat_grid_only_aligns() {
        let mut m = primitives::grid(5, 5, 5.0, 5.0);
        let mut r = Lcg(3);
        for f in 0..m.face_count() as u32 {
            if r.chance(50) {
                m.flip_face(f).unwrap();
            }
        }
        let out = make_consistent(&mut m).unwrap();
        let sign = m.face_normal(0).y.signum();
        for f in 0..m.face_count() as u32 {
            assert!((m.face_normal(f).y - sign).abs() < 1e-5 || sign * m.face_normal(f).y > 0.99);
        }
        // idempotente
        let h = m.content_hash();
        let again = make_consistent(&mut m).unwrap();
        assert!(again.selection.faces.is_empty());
        assert_eq!(m.content_hash(), h);
        // a face 0 é a referência: nunca é invertida
        assert!(!out.selection.faces.contains(&0));
        // recalc em plano perfeito mantém a referência
        recalc_normals_outside(&mut m, false).unwrap();
        assert_eq!(m.content_hash(), h);
    }

    #[test]
    fn mobius_strip_does_not_panic() {
        let n = 12usize;
        let mut m = EditMesh::new("mobius");
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..n {
            let t = std::f32::consts::TAU * i as f32 / n as f32;
            let half = t / 2.0;
            let c = Vec3::new(t.cos(), 0.0, t.sin()) * 2.0;
            let dir = Vec3::new(t.cos() * half.cos(), half.sin(), t.sin() * half.cos()) * 0.4;
            a.push(m.add_vert(c + dir));
            b.push(m.add_vert(c - dir));
        }
        for i in 0..n - 1 {
            m.add_face(&[a[i], a[i + 1], b[i + 1], b[i]]).unwrap();
        }
        m.add_face(&[a[n - 1], b[0], a[0], b[n - 1]]).unwrap();
        m.validate().unwrap();
        let mut c = m.clone();
        make_consistent(&mut c).unwrap();
        c.validate().unwrap();
        recalc_normals_outside(&mut m, false).unwrap();
        m.validate().unwrap();
    }

    #[test]
    fn non_manifold_edge_islands_are_independent() {
        // três quads compartilhando uma aresta (livro): nenhuma propagação, sem pânico
        let mut m = EditMesh::new("book");
        let a = m.add_vert(Vec3::ZERO);
        let b = m.add_vert(Vec3::X);
        for (i, p) in [Vec3::Y, Vec3::Z, -Vec3::Y].iter().enumerate() {
            let c = m.add_vert(Vec3::X + *p);
            let d = m.add_vert(*p);
            let _ = i;
            m.add_face(&[a, b, c, d]).unwrap();
        }
        let h = m.content_hash();
        let out = make_consistent(&mut m).unwrap();
        assert!(out.selection.faces.is_empty());
        assert_eq!(m.content_hash(), h);
        recalc_normals_outside(&mut m, false).unwrap();
        m.validate().unwrap();
    }

    #[test]
    fn empty_mesh_is_a_noop() {
        let mut m = EditMesh::new("empty");
        assert!(make_consistent(&mut m).unwrap().selection.is_empty());
        assert!(recalc_normals_outside(&mut m, false)
            .unwrap()
            .selection
            .is_empty());
        assert!(flip_faces(&mut m, &[0]).is_err());
    }

    #[test]
    fn corner_attributes_follow_through_recalc() {
        let mut m = primitives::cube(1.0);
        m.flip_face(2).unwrap();
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.uv = glam::Vec2::new(c.vert as f32, 1.0);
            }
        }
        recalc_normals_outside(&mut m, false).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                assert_eq!(c.uv.x, c.vert as f32);
            }
        }
        assert_outward(&m);
    }

    #[test]
    fn recalc_large_mesh_is_fast() {
        let orig = primitives::quad_sphere(1.0, 64);
        let mut m = orig.clone();
        corrupt(&mut m, 30, 2024);
        let t0 = std::time::Instant::now();
        recalc_normals_outside(&mut m, false).unwrap();
        assert!(t0.elapsed().as_secs() < 30, "{:?}", t0.elapsed());
        assert_eq!(m.content_hash(), orig.content_hash());
    }
}
