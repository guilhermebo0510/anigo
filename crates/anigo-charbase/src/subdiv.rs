//! Catmull-Clark de um nível sobre uma gaiola só de quads, com regra de fronteira.
//!
//! Layout dos índices do resultado (contrato): `[0, n_vert)` são os vértices da gaiola
//! (mesmo índice, posição suavizada), depois os pontos de aresta (na ordem crescente de
//! `EdgeKey`), depois os pontos de face (na ordem das faces). Isso permite que
//! laços/âncoras definidos na gaiola continuem válidos no resultado.

use crate::qmesh::{edge_key, EdgeKey, QuadMesh};
use glam::Vec3;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Refined {
    pub mesh: QuadMesh,
    pub n_vert: usize,
    /// aresta da gaiola -> índice do ponto de aresta
    pub edge_point: BTreeMap<EdgeKey, u32>,
    /// face da gaiola -> índice do ponto de face
    pub face_point: Vec<u32>,
}

impl Refined {
    /// Propaga um atributo por vértice da gaiola para o resultado (pontos de aresta e de
    /// face combinam os pais com `merge`).
    pub fn propagate<T: Copy>(&self, cage: &QuadMesh, base: &[T], merge: impl Fn(T, T) -> T) -> Vec<T> {
        let mut out: Vec<T> = Vec::with_capacity(self.mesh.pos.len());
        out.extend_from_slice(&base[..self.n_vert]);
        out.resize(self.mesh.pos.len(), base[0]);
        for (e, &idx) in &self.edge_point {
            out[idx as usize] = merge(base[e.0 as usize], base[e.1 as usize]);
        }
        for (fi, f) in cage.faces.iter().enumerate() {
            let m = merge(merge(base[f[0] as usize], base[f[1] as usize]), merge(base[f[2] as usize], base[f[3] as usize]));
            out[self.face_point[fi] as usize] = m;
        }
        out
    }

    /// Expande um laço da gaiola (ciclo) para o laço correspondente no resultado,
    /// intercalando os pontos de aresta. `closed` fecha o ciclo.
    pub fn expand_loop(&self, lp: &[u32], closed: bool) -> Vec<u32> {
        let mut out = Vec::with_capacity(lp.len() * 2);
        let n = lp.len();
        for k in 0..n {
            out.push(lp[k]);
            let last = k + 1 == n;
            if last && !closed {
                break;
            }
            let next = lp[(k + 1) % n];
            if let Some(&e) = self.edge_point.get(&edge_key(lp[k], next)) {
                out.push(e);
            }
        }
        out
    }
}

pub fn catmull_clark(cage: &QuadMesh) -> Refined {
    let nv = cage.pos.len();
    let ef = cage.edge_faces();
    let face_pts: Vec<Vec3> = cage
        .faces
        .iter()
        .map(|f| (cage.pos[f[0] as usize] + cage.pos[f[1] as usize] + cage.pos[f[2] as usize] + cage.pos[f[3] as usize]) * 0.25)
        .collect();

    let mut edge_point: BTreeMap<EdgeKey, u32> = BTreeMap::new();
    let mut new_pos: Vec<Vec3> = Vec::with_capacity(nv * 4);
    new_pos.resize(nv, Vec3::ZERO);

    // pontos de aresta
    for (e, fs) in &ef {
        let a = cage.pos[e.0 as usize];
        let b = cage.pos[e.1 as usize];
        let p = if fs.len() == 2 {
            (a + b + face_pts[fs[0] as usize] + face_pts[fs[1] as usize]) * 0.25
        } else {
            (a + b) * 0.5
        };
        edge_point.insert(*e, new_pos.len() as u32);
        new_pos.push(p);
    }
    let mut face_point = Vec::with_capacity(cage.faces.len());
    for fp in &face_pts {
        face_point.push(new_pos.len() as u32);
        new_pos.push(*fp);
    }

    // vértices originais
    let mut vf: Vec<Vec<u32>> = vec![Vec::new(); nv]; // faces incidentes
    let mut ve: Vec<Vec<u32>> = vec![Vec::new(); nv]; // vizinhos por aresta
    let mut vb: Vec<Vec<u32>> = vec![Vec::new(); nv]; // vizinhos por aresta de fronteira
    for (fi, f) in cage.faces.iter().enumerate() {
        for &v in f {
            vf[v as usize].push(fi as u32);
        }
    }
    for (e, fs) in &ef {
        ve[e.0 as usize].push(e.1);
        ve[e.1 as usize].push(e.0);
        if fs.len() == 1 {
            vb[e.0 as usize].push(e.1);
            vb[e.1 as usize].push(e.0);
        }
    }
    for v in 0..nv {
        let p = cage.pos[v];
        if vf[v].is_empty() {
            new_pos[v] = p;
            continue;
        }
        if vb[v].len() >= 2 {
            // fronteira: (6P + A + B) / 8
            let a = cage.pos[vb[v][0] as usize];
            let b = cage.pos[vb[v][1] as usize];
            new_pos[v] = (p * 6.0 + a + b) * 0.125;
        } else {
            let n = ve[v].len() as f32;
            let f_avg = vf[v].iter().map(|&fi| face_pts[fi as usize]).sum::<Vec3>() / vf[v].len() as f32;
            let r_avg = ve[v].iter().map(|&u| (p + cage.pos[u as usize]) * 0.5).sum::<Vec3>() / n;
            new_pos[v] = (f_avg + r_avg * 2.0 + p * (n - 3.0)) / n;
        }
    }

    let mut faces = Vec::with_capacity(cage.faces.len() * 4);
    for (fi, f) in cage.faces.iter().enumerate() {
        let fp = face_point[fi];
        let e = |a: u32, b: u32| edge_point[&edge_key(a, b)];
        let (e01, e12, e23, e30) = (e(f[0], f[1]), e(f[1], f[2]), e(f[2], f[3]), e(f[3], f[0]));
        faces.push([f[0], e01, fp, e30]);
        faces.push([e01, f[1], e12, fp]);
        faces.push([fp, e12, f[2], e23]);
        faces.push([e30, fp, e23, f[3]]);
    }
    Refined {
        mesh: QuadMesh { pos: new_pos, faces },
        n_vert: nv,
        edge_point,
        face_point,
    }
}
