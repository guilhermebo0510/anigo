//! Malha de quads indexada, mínima e determinística (a "gaiola" e o resultado refinado).
//!
//! Tudo aqui é Rust puro, sem GPU. Os mapas usam `BTreeMap` para que a ordem de
//! iteração (e portanto os índices gerados) seja sempre a mesma.

use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type EdgeKey = (u32, u32);

#[inline]
pub fn edge_key(a: u32, b: u32) -> EdgeKey {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

#[derive(Debug, Clone, Default)]
pub struct QuadMesh {
    pub pos: Vec<Vec3>,
    pub faces: Vec<[u32; 4]>,
}

impl QuadMesh {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arestas -> faces incidentes (em ordem crescente de face).
    pub fn edge_faces(&self) -> BTreeMap<EdgeKey, Vec<u32>> {
        let mut m: BTreeMap<EdgeKey, Vec<u32>> = BTreeMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for k in 0..4 {
                m.entry(edge_key(f[k], f[(k + 1) % 4])).or_default().push(fi as u32);
            }
        }
        m
    }

    /// Orienta todas as faces de forma consistente (cada aresta interna é percorrida em
    /// sentidos opostos pelas duas faces) e inverte cada componente para que o volume
    /// com sinal seja positivo (normais para fora). Falha se houver aresta não-manifold.
    pub fn orient_consistently(&mut self) -> Result<(), String> {
        let ef = self.edge_faces();
        for (e, fs) in &ef {
            if fs.len() > 2 {
                return Err(format!("aresta {:?} compartilhada por {} faces (não-manifold)", e, fs.len()));
            }
        }
        let n = self.faces.len();
        let mut comp = vec![u32::MAX; n];
        let mut comps: Vec<Vec<u32>> = Vec::new();
        for start in 0..n {
            if comp[start] != u32::MAX {
                continue;
            }
            let cid = comps.len() as u32;
            let mut members = vec![start as u32];
            comp[start] = cid;
            let mut q = VecDeque::new();
            q.push_back(start as u32);
            while let Some(fi) = q.pop_front() {
                let f = self.faces[fi as usize];
                for k in 0..4 {
                    let (a, b) = (f[k], f[(k + 1) % 4]);
                    let Some(neigh) = ef.get(&edge_key(a, b)) else { continue };
                    for &g in neigh {
                        if g == fi {
                            continue;
                        }
                        let gf = self.faces[g as usize];
                        // g deve percorrer a aresta como (b, a)
                        let mut same_dir = false;
                        for j in 0..4 {
                            if gf[j] == a && gf[(j + 1) % 4] == b {
                                same_dir = true;
                            }
                        }
                        if comp[g as usize] == u32::MAX {
                            if same_dir {
                                self.faces[g as usize].swap(1, 3);
                            }
                            comp[g as usize] = cid;
                            members.push(g);
                            q.push_back(g);
                        } else if same_dir {
                            return Err(format!("malha não orientável perto da face {g}"));
                        }
                    }
                }
            }
            comps.push(members);
        }
        for members in &comps {
            let mut vol = 0.0f64;
            for &fi in members {
                let f = self.faces[fi as usize];
                let p: Vec<Vec3> = f.iter().map(|&i| self.pos[i as usize]).collect();
                // dois triângulos (0,1,2) e (0,2,3)
                vol += p[0].dot(p[1].cross(p[2])) as f64 / 6.0;
                vol += p[0].dot(p[2].cross(p[3])) as f64 / 6.0;
            }
            if vol < 0.0 {
                for &fi in members {
                    self.faces[fi as usize].swap(1, 3);
                }
            }
        }
        Ok(())
    }

    /// Volume com sinal (fórmula do divergente; confiável para malhas fechadas).
    pub fn signed_volume(&self) -> f32 {
        let mut vol = 0.0f64;
        for f in &self.faces {
            let p: Vec<Vec3> = f.iter().map(|&i| self.pos[i as usize]).collect();
            vol += p[0].dot(p[1].cross(p[2])) as f64 / 6.0;
            vol += p[0].dot(p[2].cross(p[3])) as f64 / 6.0;
        }
        vol as f32
    }

    /// Laços de fronteira (arestas com uma única face), cada um como ciclo de vértices.
    pub fn boundary_loops(&self) -> Vec<Vec<u32>> {
        let ef = self.edge_faces();
        let mut next: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (e, fs) in &ef {
            if fs.len() == 1 {
                next.entry(e.0).or_default().push(e.1);
                next.entry(e.1).or_default().push(e.0);
            }
        }
        let mut visited: BTreeSet<u32> = BTreeSet::new();
        let mut loops = Vec::new();
        let keys: Vec<u32> = next.keys().copied().collect();
        for s in keys {
            if visited.contains(&s) {
                continue;
            }
            let mut lp = vec![s];
            visited.insert(s);
            let mut prev = u32::MAX;
            let mut cur = s;
            loop {
                let nb = &next[&cur];
                let nxt = nb.iter().copied().find(|&x| x != prev && !visited.contains(&x));
                match nxt {
                    Some(x) => {
                        lp.push(x);
                        visited.insert(x);
                        prev = cur;
                        cur = x;
                    }
                    None => break,
                }
            }
            loops.push(lp);
        }
        loops
    }

    pub fn edge_count(&self) -> usize {
        self.edge_faces().len()
    }

    /// Característica de Euler V - E + F (conta só vértices usados).
    pub fn euler(&self) -> i64 {
        let used: BTreeSet<u32> = self.faces.iter().flatten().copied().collect();
        used.len() as i64 - self.edge_count() as i64 + self.faces.len() as i64
    }

    /// Remove vértices sem face, devolvendo `old -> new` (u32::MAX = removido).
    pub fn compact(&mut self) -> Vec<u32> {
        let mut used = vec![false; self.pos.len()];
        for f in &self.faces {
            for &i in f {
                used[i as usize] = true;
            }
        }
        let mut remap = vec![u32::MAX; self.pos.len()];
        let mut pos = Vec::new();
        for (i, u) in used.iter().enumerate() {
            if *u {
                remap[i] = pos.len() as u32;
                pos.push(self.pos[i]);
            }
        }
        for f in &mut self.faces {
            for i in f.iter_mut() {
                *i = remap[*i as usize];
            }
        }
        self.pos = pos;
        remap
    }

    /// Valência (número de arestas incidentes) por vértice.
    pub fn valences(&self) -> Vec<u32> {
        let mut v = vec![0u32; self.pos.len()];
        for (e, _) in self.edge_faces() {
            v[e.0 as usize] += 1;
            v[e.1 as usize] += 1;
        }
        v
    }

    /// Normais de vértice (média das normais de face ponderadas por área, via Newell).
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        let mut n = vec![Vec3::ZERO; self.pos.len()];
        for f in &self.faces {
            let p: Vec<Vec3> = f.iter().map(|&i| self.pos[i as usize]).collect();
            let fnorm = (p[2] - p[0]).cross(p[3] - p[1]); // 2 * área * normal
            for &i in f {
                n[i as usize] += fnorm;
            }
        }
        for v in &mut n {
            *v = v.normalize_or_zero();
        }
        n
    }
}
