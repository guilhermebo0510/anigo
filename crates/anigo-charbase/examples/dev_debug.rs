//! Depuração: contatos entre superfícies e NaN nas peças.
use anigo_charbase::character::skin_positions;
use anigo_charbase::params::Params;
use anigo_charbase::parts::build_parts;
use anigo_charbase::topology::reference;
use anigo_modeling::bvh::TriBvh;
use anigo_modeling::EditMesh;
use std::collections::BTreeSet;

fn ball(adj: &[Vec<u32>], src: u32, depth: u32) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut fr = vec![src];
    seen.insert(src);
    for _ in 0..depth {
        let mut nx = Vec::new();
        for v in fr {
            for &n in &adj[v as usize] {
                if seen.insert(n) {
                    nx.push(n);
                }
            }
        }
        fr = nx;
    }
    seen
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut p = Params::default();
    for a in &args {
        let kv: Vec<&str> = a.splitn(2, '=').collect();
        if kv.len() == 2 {
            let v: f32 = kv[1].parse().unwrap_or(0.0);
            match kv[0] {
                "weight" => p.weight = v,
                "age" => p.age = v,
                "muscle" => p.muscle = v,
                "height" => p.height = v,
                "head" => p.head_size = v,
                "gender" => p.gender = v,
                _ => {}
            }
        }
    }
    let topo = reference();
    let pos = skin_positions(&p);
    let mut em = EditMesh::new("s");
    for q in &pos {
        em.add_vert(*q);
    }
    for f in &topo.faces {
        em.add_face(&[f[0], f[1], f[2], f[3]]).unwrap();
    }
    let bvh = TriBvh::build(&em);
    let mut n = 0;
    for v in 0..pos.len() {
        let b = ball(&topo.adjacency, v as u32, 7);
        for t in bvh.query_sphere_tris(pos[v], 0.0012) {
            let tr = bvh.tri_ref(t).unwrap();
            if tr.verts.iter().any(|u| b.contains(u)) {
                continue;
            }
            let other = topo.region[tr.verts[0] as usize];
            println!("contato v{v} {:?} @ {:?} com tri de {:?}", topo.region[v], pos[v], other);
            n += 1;
            break;
        }
    }
    println!("total {n}");
    let parts = build_parts(topo, &pos, &pos, 0.0);
    for pt in &parts {
        let bad = pt.positions.iter().filter(|v| !v.is_finite()).count();
        if bad > 0 {
            println!("peça {} com {bad} posições não finitas", pt.name);
        }
    }
}
