//! Desdobramento (unwrap) por ilha: **LSCM** (Least Squares Conformal Maps, Lévy et al. 2002)
//! com solver próprio (matriz esparsa CSR + gradiente conjugado pré-condicionado), e
//! relaxamento **ARAP** (as-rigid-as-possible, Liu et al. 2008) para reduzir estiramento.
//!
//! Pipeline de [`unwrap_lscm`], por ilha (ilhas = componentes ligados sem atravessar costuras;
//! os UVs atuais são ignorados):
//!
//! 1. Os corners de um mesmo vértice ligados por arestas não-costura formam *wedges*; cada
//!    wedge é uma incógnita (vértices de costura têm um UV por lado do corte).
//! 2. Faces viram triângulos: triângulos ficam como estão; quads usam as duas diagonais
//!    válidas (peso 1/2 cada); n-gons (e quads côncavos degenerados) usam um leque a partir
//!    do centróide (variável virtual).
//! 3. Opcional (`fill_holes`): laços de fronteira internos são preenchidos virtualmente com
//!    leques de triângulos (não escritos no resultado), como "Fill Holes" do Blender.
//! 4. Dois pinos — o par de vértices de fronteira mais distantes no 3D — são fixados em
//!    `(0,0)` e `(d,0)`; o sistema normal `AᵀA x = −AᵀA_p x_p` é resolvido por CG.
//! 5. Cada ilha é reescalada para que sua área UV seja igual à área 3D (densidade de texels
//!    uniforme entre ilhas); depois, se `pack`, as ilhas são empacotadas em `[0,1]²`.
//!
//! **Limitação**: o método angle-based (ABF/ABF++) *não* foi implementado; a opção
//! [`UnwrapMethod::LscmArap`] (LSCM seguido de relaxamento ARAP, com reversão automática se
//! piorar) cobre o caso de uso de reduzir estiramento. Ilhas que não são discos (genus > 0 ou
//! vários laços sem `fill_holes`) são desdobradas mesmo assim, mas podem se dobrar sobre si
//! mesmas — o relatório conta essas ilhas em `non_disk_islands`.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::topology::Topology;
use crate::uv::metrics::poly_signed_area;
use crate::uv::pack::{pack_island_list, PackParams, PackReport};
use crate::uv::seams::{
    build_islands, compute_wedges, faces_uv_bounds, island_euler, membership, resolve_faces,
    Island, Wedges,
};
use glam::{DVec2, DVec3, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------- álgebra esparsa

/// Matriz esparsa em formato CSR (linhas ordenadas por coluna, sem duplicatas).
#[derive(Debug, Clone)]
pub(crate) struct Csr {
    n: usize,
    row_ptr: Vec<usize>,
    col: Vec<u32>,
    val: Vec<f64>,
}

impl Csr {
    /// Constrói a partir de triplas `(linha, coluna, valor)`, somando duplicatas numa ordem
    /// determinística (distribuição por linha + ordenação estável por coluna dentro da linha).
    pub(crate) fn from_triplets(n: usize, t: Vec<(u32, u32, f64)>) -> Self {
        let mut start = vec![0usize; n + 1];
        for &(r, _, _) in &t {
            start[r as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut tmp: Vec<(u32, f64)> = vec![(0, 0.0); t.len()];
        for (r, c, v) in t {
            tmp[fill[r as usize]] = (c, v);
            fill[r as usize] += 1;
        }
        let mut row_ptr = vec![0usize; n + 1];
        let mut col: Vec<u32> = Vec::with_capacity(tmp.len());
        let mut val: Vec<f64> = Vec::with_capacity(tmp.len());
        for r in 0..n {
            let seg = &mut tmp[start[r]..start[r + 1]];
            seg.sort_by_key(|e| e.0);
            let mut last: Option<u32> = None;
            for &(c, v) in seg.iter() {
                if last == Some(c) {
                    if let Some(x) = val.last_mut() {
                        *x += v;
                    }
                } else {
                    last = Some(c);
                    col.push(c);
                    val.push(v);
                }
            }
            row_ptr[r + 1] = col.len();
        }
        Self { n, row_ptr, col, val }
    }

    /// `y = A x`.
    pub(crate) fn mul(&self, x: &[f64], y: &mut [f64]) {
        for r in 0..self.n {
            let mut s = 0.0;
            for k in self.row_ptr[r]..self.row_ptr[r + 1] {
                s += self.val[k] * x[self.col[k] as usize];
            }
            y[r] = s;
        }
    }

    pub(crate) fn diag(&self) -> Vec<f64> {
        let mut d = vec![0.0; self.n];
        for (r, dr) in d.iter_mut().enumerate() {
            for k in self.row_ptr[r]..self.row_ptr[r + 1] {
                if self.col[k] as usize == r {
                    *dr = self.val[k];
                }
            }
        }
        d
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Gradiente conjugado com pré-condicionador de Jacobi para `A x = b` (A simétrica
/// positiva semi-definida). `x` entra como chute inicial. Devolve `(iterações, resíduo relativo)`.
pub(crate) fn pcg(a: &Csr, b: &[f64], x: &mut [f64], tol: f64, max_it: usize) -> (usize, f64) {
    let n = b.len();
    let inv_d: Vec<f64> = a.diag().iter().map(|&d| if d > 1e-300 { 1.0 / d } else { 1.0 }).collect();
    let bnorm = dot(b, b).sqrt().max(1e-300);
    let mut ax = vec![0.0; n];
    a.mul(x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(bi, ai)| bi - ai).collect();
    let mut z: Vec<f64> = r.iter().zip(&inv_d).map(|(ri, di)| ri * di).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    let mut iters = 0;
    let mut res = dot(&r, &r).sqrt() / bnorm;
    while iters < max_it && res > tol {
        a.mul(&p, &mut ap);
        let pap = dot(&p, &ap);
        if pap <= 1e-300 || !pap.is_finite() {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        for i in 0..n {
            z[i] = r[i] * inv_d[i];
        }
        let rz_new = dot(&r, &z);
        let beta = rz_new / rz.max(1e-300);
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
        iters += 1;
        res = dot(&r, &r).sqrt() / bnorm;
    }
    (iters, res)
}

// ---------------------------------------------------------------------- parâmetros / relatórios

/// Método de desdobramento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnwrapMethod {
    /// Mapa conforme por mínimos quadrados.
    Lscm,
    /// LSCM + relaxamento ARAP (menos estiramento; reverte se não melhorar).
    /// O método angle-based (ABF++) não está implementado.
    LscmArap,
}

/// Parâmetros de [`unwrap_lscm`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnwrapParams {
    pub method: UnwrapMethod,
    /// Preenche virtualmente buracos internos (laços de fronteira extras) para evitar
    /// sobreposições e preservar simetria.
    pub fill_holes: bool,
    /// Margem entre ilhas no empacotamento, como **fração** do espaço UV (padrão 0.001).
    pub margin: f32,
    /// Corrige a proporção da imagem: `u /= aspect` (aspect = largura/altura da textura).
    pub correct_aspect: bool,
    /// Largura/altura da textura (usado só com `correct_aspect`). Padrão 1.
    pub aspect: f32,
    /// Empacota as ilhas em `[0,1]²` ao final. Com `false`, cada ilha fica em unidades
    /// do mundo com o canto mínimo na origem (ilhas se sobrepõem até o `pack_islands`).
    pub pack: bool,
    /// Iterações ARAP para [`UnwrapMethod::LscmArap`].
    pub arap_iterations: u32,
    /// Máximo de iterações de CG por solve (0 = automático).
    pub max_cg_iterations: usize,
    /// Tolerância relativa de CG.
    pub tolerance: f64,
}

impl Default for UnwrapParams {
    fn default() -> Self {
        Self {
            method: UnwrapMethod::Lscm,
            fill_holes: true,
            margin: 0.001,
            correct_aspect: true,
            aspect: 1.0,
            pack: true,
            arap_iterations: 10,
            max_cg_iterations: 0,
            tolerance: 1e-10,
        }
    }
}

impl UnwrapParams {
    fn validate(&self) -> ModelResult<()> {
        if !self.margin.is_finite() || !(0.0..0.5).contains(&self.margin) {
            return Err(ModelError::InvalidParameter(format!(
                "margem deve estar em [0, 0.5), recebeu {}",
                self.margin
            )));
        }
        if !self.aspect.is_finite() || self.aspect <= 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "aspect deve ser > 0, recebeu {}",
                self.aspect
            )));
        }
        if !self.tolerance.is_finite() || self.tolerance <= 0.0 || self.tolerance >= 1.0 {
            return Err(ModelError::InvalidParameter(format!(
                "tolerância de CG deve estar em (0, 1), recebeu {}",
                self.tolerance
            )));
        }
        if self.arap_iterations > 10_000 {
            return Err(ModelError::InvalidParameter("arap_iterations > 10000".into()));
        }
        Ok(())
    }
}

/// Resumo de [`unwrap_lscm`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnwrapReport {
    pub islands: usize,
    pub faces: usize,
    /// Soma das iterações de CG (todas as ilhas, só o LSCM).
    pub cg_iterations: usize,
    /// Maior resíduo relativo final de CG entre as ilhas.
    pub max_residual: f64,
    /// Laços de fronteira preenchidos virtualmente.
    pub holes_filled: usize,
    /// Ilhas que não eram discos topológicos.
    pub non_disk_islands: usize,
    /// Faces degeneradas ignoradas no sistema (herdam o UV dos vizinhos pelos wedges).
    pub skipped_faces: usize,
    /// Ilhas em que o ARAP melhorou o resultado (método `LscmArap`).
    pub arap_improved_islands: usize,
    pub pack: Option<PackReport>,
}

/// Resumo de [`unwrap_minimum_stretch`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StretchReport {
    pub islands: usize,
    /// Ilhas efetivamente atualizadas (as demais foram revertidas por não melhorar).
    pub improved: usize,
    /// Distorção isométrica média `max(σ1/s, s/σ2)` (ponderada por área 3D) antes e depois,
    /// agregando todas as ilhas; 1 = isométrico a menos de escala.
    pub mean_before: f32,
    pub mean_after: f32,
}

// ---------------------------------------------------------------------- sistema por ilha

#[derive(Debug, Clone)]
struct Tri {
    v: [u32; 3],
    /// Coordenadas 2D locais (anti-horárias) no plano do triângulo.
    p: [DVec2; 3],
    w: f64,
    real: bool,
}

struct IslandSystem {
    faces: Vec<u32>,
    /// Variável local de cada corner de cada face da ilha.
    face_vars: Vec<Vec<u32>>,
    nv_real: usize,
    nv: usize,
    pos3: Vec<DVec3>,
    /// Pais (variáveis reais) de cada variável virtual (índice `v - nv_real`).
    virt_parents: Vec<Vec<u32>>,
    tris: Vec<Tri>,
    boundary: Vec<u32>,
    holes_filled: usize,
    skipped_faces: usize,
}

fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// Coordenadas 2D locais de um triângulo 3D (anti-horário em relação à ordem dos vértices).
fn tri_local(p0: DVec3, p1: DVec3, p2: DVec3) -> Option<[DVec2; 3]> {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let a1 = e1.length();
    if a1 < 1e-300 {
        return None;
    }
    let h = e1.cross(e2).length() / a1;
    if h < 1e-9 * a1 {
        return None;
    }
    Some([DVec2::ZERO, DVec2::new(a1, 0.0), DVec2::new(e1.dot(e2) / a1, h)])
}

fn newell(p: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..p.len() {
        n += p[i].cross(p[(i + 1) % p.len()]);
    }
    n * 0.5
}

impl IslandSystem {
    fn build(
        mesh: &EditMesh,
        wedges: &Wedges,
        faces: &[u32],
        fill_holes: bool,
    ) -> IslandSystem {
        // variáveis locais = wedges globais ordenados
        let mut gl: Vec<u32> = faces
            .iter()
            .flat_map(|&f| (0..mesh.faces[f as usize].corners.len()).map(move |c| (f, c)))
            .map(|(f, c)| wedges.corner_var(f, c))
            .collect();
        gl.sort_unstable();
        gl.dedup();
        let nv_real = gl.len();
        let local = |g: u32| gl.binary_search(&g).unwrap_or(0) as u32;
        let mut pos3 = vec![DVec3::ZERO; nv_real];
        let mut face_vars = Vec::with_capacity(faces.len());
        for &f in faces {
            let face = &mesh.faces[f as usize];
            let vars: Vec<u32> = (0..face.corners.len())
                .map(|c| local(wedges.corner_var(f, c)))
                .collect();
            for (c, &lv) in vars.iter().enumerate() {
                pos3[lv as usize] = mesh.co(face.corners[c].vert).as_dvec3();
            }
            face_vars.push(vars);
        }
        let mut sys = IslandSystem {
            faces: faces.to_vec(),
            face_vars,
            nv_real,
            nv: nv_real,
            pos3,
            virt_parents: Vec::new(),
            tris: Vec::new(),
            boundary: Vec::new(),
            holes_filled: 0,
            skipped_faces: 0,
        };
        for fi in 0..faces.len() {
            let vars = sys.face_vars[fi].clone();
            if !sys.triangulate_face(&vars) {
                sys.skipped_faces += 1;
            }
        }
        let loops = sys.find_boundary();
        if fill_holes && loops.len() > 1 {
            sys.fill_holes(loops);
        }
        sys
    }

    fn add_virtual(&mut self, parents: Vec<u32>, pos: DVec3) -> u32 {
        self.virt_parents.push(parents);
        self.pos3.push(pos);
        self.nv += 1;
        (self.nv - 1) as u32
    }

    /// Triangula uma face no plano de Newell. `false` se a face é degenerada.
    fn triangulate_face(&mut self, cv: &[u32]) -> bool {
        let n = cv.len();
        let p: Vec<DVec3> = cv.iter().map(|&v| self.pos3[v as usize]).collect();
        let nrm = newell(&p);
        let area = nrm.length();
        if area < 1e-30 {
            return false;
        }
        let nh = nrm / area;
        let mut b1 = DVec3::ZERO;
        for i in 1..n {
            let d = p[i] - p[0];
            let t = d - nh * nh.dot(d);
            if t.length() > b1.length() {
                b1 = t;
            }
        }
        if b1.length() < 1e-300 {
            return false;
        }
        let b1 = b1.normalize();
        let b2 = nh.cross(b1);
        let q: Vec<DVec2> = p
            .iter()
            .map(|&x| {
                let d = x - p[0];
                DVec2::new(d.dot(b1), d.dot(b2))
            })
            .collect();
        let eps = 1e-9 * area;
        let tri_area = |a: usize, b: usize, c: usize| 0.5 * cross2(q[b] - q[a], q[c] - q[a]);
        let push = |s: &mut Self, a: usize, b: usize, c: usize, w: f64| {
            s.tris.push(Tri {
                v: [cv[a], cv[b], cv[c]],
                p: [q[a], q[b], q[c]],
                w,
                real: true,
            });
        };
        if n == 3 {
            if tri_area(0, 1, 2) <= eps {
                return false;
            }
            push(self, 0, 1, 2, 1.0);
            return true;
        }
        if n == 4 {
            let d02 = tri_area(0, 1, 2) > eps && tri_area(0, 2, 3) > eps;
            let d13 = tri_area(1, 2, 3) > eps && tri_area(1, 3, 0) > eps;
            let w = if d02 && d13 { 0.5 } else { 1.0 };
            if d02 {
                push(self, 0, 1, 2, w);
                push(self, 0, 2, 3, w);
            }
            if d13 {
                push(self, 1, 2, 3, w);
                push(self, 1, 3, 0, w);
            }
            if d02 || d13 {
                return true;
            }
        }
        // leque a partir do centróide
        let centroid3 = p.iter().copied().sum::<DVec3>() / n as f64;
        let c2 = q.iter().copied().sum::<DVec2>() / n as f64;
        let cvar = self.add_virtual(cv.to_vec(), centroid3);
        let mut any = false;
        for i in 0..n {
            let j = (i + 1) % n;
            if 0.5 * cross2(q[i] - c2, q[j] - c2) > eps {
                self.tris.push(Tri {
                    v: [cvar, cv[i], cv[j]],
                    p: [c2, q[i], q[j]],
                    w: 1.0,
                    real: true,
                });
                any = true;
            }
        }
        any
    }

    /// Detecta a fronteira (arestas de wedge usadas uma única vez). Preenche `self.boundary`
    /// e devolve os laços orientados (maior perímetro primeiro).
    fn find_boundary(&mut self) -> Vec<Vec<u32>> {
        let mut count: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
        for vars in &self.face_vars {
            let n = vars.len();
            for i in 0..n {
                let (a, b) = (vars[i], vars[(i + 1) % n]);
                if a == b {
                    continue;
                }
                let e = count.entry((a.min(b), a.max(b))).or_insert((0, a, b));
                e.0 += 1;
            }
        }
        let mut dir: Vec<(u32, u32)> = count
            .values()
            .filter(|e| e.0 == 1)
            .map(|e| (e.1, e.2))
            .collect();
        dir.sort_unstable();
        let mut bverts: Vec<u32> = dir.iter().flat_map(|&(a, b)| [a, b]).collect();
        bverts.sort_unstable();
        bverts.dedup();
        self.boundary = bverts;
        let mut out: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, &(a, _)) in dir.iter().enumerate() {
            out.entry(a).or_default().push(i);
        }
        let mut used = vec![false; dir.len()];
        let mut loops: Vec<Vec<u32>> = Vec::new();
        for s in 0..dir.len() {
            if used[s] {
                continue;
            }
            used[s] = true;
            let start = dir[s].0;
            let mut lp = vec![start];
            let mut v = dir[s].1;
            let mut closed = true;
            while v != start {
                lp.push(v);
                let next = out
                    .get(&v)
                    .and_then(|l| l.iter().copied().find(|&k| !used[k]));
                match next {
                    Some(k) => {
                        used[k] = true;
                        v = dir[k].1;
                    }
                    None => {
                        closed = false;
                        break;
                    }
                }
            }
            if closed {
                loops.push(lp);
            }
        }
        let perim = |lp: &Vec<u32>| -> f64 {
            (0..lp.len())
                .map(|i| (self.pos3[lp[i] as usize] - self.pos3[lp[(i + 1) % lp.len()] as usize]).length())
                .sum()
        };
        loops.sort_by(|a, b| perim(b).total_cmp(&perim(a)));
        loops
    }

    /// Preenche todos os laços exceto o maior com leques de triângulos virtuais.
    fn fill_holes(&mut self, loops: Vec<Vec<u32>>) {
        for lp in loops.into_iter().skip(1) {
            if lp.len() < 3 {
                continue;
            }
            let c3 = lp.iter().map(|&v| self.pos3[v as usize]).sum::<DVec3>() / lp.len() as f64;
            let c = self.add_virtual(lp.clone(), c3);
            for i in 0..lp.len() {
                let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                if let Some(p) = tri_local(self.pos3[b as usize], self.pos3[a as usize], c3) {
                    self.tris.push(Tri {
                        v: [b, a, c],
                        p,
                        w: 1.0,
                        real: false,
                    });
                }
            }
            self.holes_filled += 1;
        }
    }

    /// Par de vértices de fronteira mais distantes (varredura dupla; determinístico).
    fn farthest_boundary_pair(&self) -> Option<(u32, u32)> {
        let b = &self.boundary;
        if b.len() < 2 {
            return None;
        }
        let far = |from: u32| -> u32 {
            let pf = self.pos3[from as usize];
            let mut best = from;
            let mut bd = -1.0;
            for &v in b {
                let d = (self.pos3[v as usize] - pf).length_squared();
                if d > bd {
                    bd = d;
                    best = v;
                }
            }
            best
        };
        let dist = |a: u32, c: u32| (self.pos3[a as usize] - self.pos3[c as usize]).length();
        let mut best = (b[0], far(b[0]));
        for _ in 0..3 {
            let cand = (best.1, far(best.1));
            if dist(cand.0, cand.1) > dist(best.0, best.1) * (1.0 + 1e-12) {
                best = cand;
            } else {
                break;
            }
        }
        if dist(best.0, best.1) < 1e-300 {
            None
        } else {
            Some(best)
        }
    }

    /// UV inicial por variável a partir dos UVs atuais da malha (virtuais = média dos pais).
    fn uvs_from_mesh(&self, mesh: &EditMesh) -> Vec<DVec2> {
        let mut q = vec![DVec2::ZERO; self.nv];
        for (fi, &f) in self.faces.iter().enumerate() {
            for (c, &v) in self.face_vars[fi].iter().enumerate() {
                q[v as usize] = mesh.faces[f as usize].corners[c].uv.as_dvec2();
            }
        }
        self.fill_virtual(&mut q);
        q
    }

    fn fill_virtual(&self, q: &mut [DVec2]) {
        for (k, parents) in self.virt_parents.iter().enumerate() {
            let m = parents.iter().map(|&p| q[p as usize]).sum::<DVec2>() / parents.len().max(1) as f64;
            q[self.nv_real + k] = m;
        }
    }

    /// Área UV líquida (com sinal) das faces reais da ilha.
    fn net_uv_area(&self, q: &[DVec2]) -> f64 {
        let mut total = 0.0;
        for vars in &self.face_vars {
            let pts: Vec<Vec2> = vars.iter().map(|&v| q[v as usize].as_vec2()).collect();
            total += poly_signed_area(&pts);
        }
        total
    }
}

// ---------------------------------------------------------------------- LSCM

struct LscmOutcome {
    q: Vec<DVec2>,
    iterations: usize,
    residual: f64,
}

fn lscm_solve(
    sys: &IslandSystem,
    pins: (u32, u32),
    params: &UnwrapParams,
) -> ModelResult<LscmOutcome> {
    let nv = sys.nv;
    let (p0, p1) = pins;
    let d = (sys.pos3[p0 as usize] - sys.pos3[p1 as usize]).length();
    // índices: coluna (comp, var) = comp * nv + var
    let col = |comp: usize, v: u32| comp * nv + v as usize;
    let mut x_pin = vec![0.0f64; 2 * nv];
    let mut free_idx = vec![u32::MAX; 2 * nv];
    let pinned = [col(0, p0), col(1, p0), col(0, p1), col(1, p1)];
    x_pin[col(0, p1)] = d;
    let mut m = 0u32;
    for (c, fi) in free_idx.iter_mut().enumerate() {
        if !pinned.contains(&c) {
            *fi = m;
            m += 1;
        }
    }
    let m = m as usize;

    let mut trip: Vec<(u32, u32, f64)> = Vec::with_capacity(sys.tris.len() * 36);
    let mut rhs = vec![0.0f64; m];
    for t in &sys.tris {
        let [a, b, c] = t.p;
        let dd = cross2(b - a, c - a);
        if dd <= 1e-300 {
            continue;
        }
        let coef = (t.w / dd).sqrt();
        let w = [c - b, a - c, b - a];
        let (wr, wi): ([f64; 3], [f64; 3]) = (
            [w[0].x * coef, w[1].x * coef, w[2].x * coef],
            [w[0].y * coef, w[1].y * coef, w[2].y * coef],
        );
        // Bloco 6x6 de AᵀA desta face, ordem [u0 u1 u2 v0 v1 v2]:
        // uu = vv = K, uv = S (K, S de Re/Im de ΣW z).
        let mut blk = [[0.0f64; 6]; 6];
        for i in 0..3 {
            for j in 0..3 {
                let k = wr[i] * wr[j] + wi[i] * wi[j];
                let sij = wi[i] * wr[j] - wr[i] * wi[j];
                blk[i][j] = k;
                blk[3 + i][3 + j] = k;
                blk[i][3 + j] = sij;
                blk[3 + j][i] = sij;
            }
        }
        let cols: [usize; 6] = [
            col(0, t.v[0]),
            col(0, t.v[1]),
            col(0, t.v[2]),
            col(1, t.v[0]),
            col(1, t.v[1]),
            col(1, t.v[2]),
        ];
        for i in 0..6 {
            let fi = free_idx[cols[i]];
            if fi == u32::MAX {
                continue;
            }
            for j in 0..6 {
                let fj = free_idx[cols[j]];
                if fj == u32::MAX {
                    rhs[fi as usize] -= blk[i][j] * x_pin[cols[j]];
                } else {
                    trip.push((fi, fj, blk[i][j]));
                }
            }
        }
    }
    if m == 0 {
        // só os dois pinos: nada a resolver
        let mut q = vec![DVec2::ZERO; nv];
        q[p1 as usize] = DVec2::new(d, 0.0);
        return Ok(LscmOutcome { q, iterations: 0, residual: 0.0 });
    }
    let a = Csr::from_triplets(m, trip);

    // chute inicial: projeção no plano médio, alinhada aos pinos
    let mut nrm = DVec3::ZERO;
    for t in sys.tris.iter().filter(|t| t.real) {
        let [x0, x1, x2] = t.v.map(|v| sys.pos3[v as usize]);
        nrm += (x1 - x0).cross(x2 - x0);
    }
    let nh = nrm.try_normalize().unwrap_or(DVec3::Y);
    let pin_vec = sys.pos3[p1 as usize] - sys.pos3[p0 as usize];
    let mut ax = pin_vec - nh * nh.dot(pin_vec);
    let ax_len = ax.length();
    let scale0 = if ax_len > 1e-12 * d.max(1e-300) {
        ax /= ax_len;
        d / ax_len
    } else {
        ax = nh.any_orthonormal_vector();
        1.0
    };
    let ay = nh.cross(ax);
    let mut x = vec![0.0f64; m];
    for v in 0..nv as u32 {
        let r = sys.pos3[v as usize] - sys.pos3[p0 as usize];
        let g = DVec2::new(r.dot(ax), r.dot(ay)) * scale0;
        for (comp, val) in [(0, g.x), (1, g.y)] {
            let fi = free_idx[col(comp, v)];
            if fi != u32::MAX {
                x[fi as usize] = val;
            }
        }
    }
    let max_it = if params.max_cg_iterations > 0 {
        params.max_cg_iterations
    } else {
        (m.max(2000)).min(50_000)
    };
    let (iterations, residual) = pcg(&a, &rhs, &mut x, params.tolerance, max_it);
    let mut q = vec![DVec2::ZERO; nv];
    for v in 0..nv as u32 {
        for comp in 0..2 {
            let c = col(comp, v);
            let val = if free_idx[c] == u32::MAX { x_pin[c] } else { x[free_idx[c] as usize] };
            if comp == 0 {
                q[v as usize].x = val;
            } else {
                q[v as usize].y = val;
            }
        }
    }
    if q.iter().any(|p| !p.is_finite()) {
        return Err(ModelError::Numeric("LSCM produziu UV não finito".into()));
    }
    Ok(LscmOutcome { q, iterations, residual })
}

// ---------------------------------------------------------------------- ARAP / qualidade

/// `(a, b, c, d, det)` com `P = [[a, b], [c, d]]` = colunas `p1-p0`, `p2-p0`.
fn tri_matrix(p: &[DVec2; 3]) -> (f64, f64, f64, f64, f64) {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    (e1.x, e2.x, e1.y, e2.y, e1.x * e2.y - e2.x * e1.y)
}

/// Jacobiana `J = Q P⁻¹` do triângulo (linha-major `[[j00,j01],[j10,j11]]`).
fn tri_jacobian(p: &[DVec2; 3], q: [DVec2; 3]) -> Option<[f64; 4]> {
    let (a, b, c, d, det) = tri_matrix(p);
    if det.abs() < 1e-300 {
        return None;
    }
    let (f1, f2) = (q[1] - q[0], q[2] - q[0]);
    let inv = [d / det, -b / det, -c / det, a / det];
    Some([
        f1.x * inv[0] + f2.x * inv[2],
        f1.x * inv[1] + f2.x * inv[3],
        f1.y * inv[0] + f2.y * inv[2],
        f1.y * inv[1] + f2.y * inv[3],
    ])
}

/// Distorção isométrica das faces reais: `(média ponderada de max(σ1/s, s/σ2), nº de
/// triângulos invertidos, peso total)`, com `s² = ΣA_uv / ΣA_3d`. Vale 1 para um mapa
/// isométrico (a menos de escala global); penaliza estiramento *e* variação de área.
fn quality(sys: &IslandSystem, q: &[DVec2]) -> (f64, usize, f64) {
    let mut data: Vec<(f64, f64, f64)> = Vec::new(); // (σ1, σ2, peso 3D)
    let mut flipped = 0usize;
    let (mut uv_a, mut a3) = (0.0, 0.0);
    for t in sys.tris.iter().filter(|t| t.real) {
        let qq = t.v.map(|v| q[v as usize]);
        let Some(j) = tri_jacobian(&t.p, qq) else { continue };
        let det = j[0] * j[3] - j[1] * j[2];
        let sum = j.iter().map(|x| x * x).sum::<f64>();
        let sp = (sum + 2.0 * det.abs()).max(0.0).sqrt();
        let sm = (sum - 2.0 * det.abs()).max(0.0).sqrt();
        let w = t.w * 0.5 * tri_matrix(&t.p).4.abs();
        uv_a += w * det.abs();
        a3 += w;
        if det < 0.0 {
            flipped += 1;
        }
        data.push(((sp + sm) * 0.5, (sp - sm) * 0.5, w));
    }
    if a3 <= 0.0 {
        return (1.0, flipped, 0.0);
    }
    let s = (uv_a / a3).sqrt().max(1e-300);
    let mut num = 0.0;
    for (s1, s2, w) in &data {
        let d = if *s2 <= 1e-12 * s1.max(1e-300) { 1e3 } else { (s1 / s).max(s / s2).min(1e3) };
        num += w * d;
    }
    (num / a3, flipped, a3)
}

/// Relaxamento ARAP (local/global). Fixa a variável 0 e resolve o laplaciano de cotangentes
/// do 3D. `q` entra como estado inicial e sai atualizado. Devolve o total de iterações de CG.
fn arap_refine(sys: &IslandSystem, q: &mut [DVec2], iterations: u32, params: &UnwrapParams) -> usize {
    let nv = sys.nv;
    if nv < 2 {
        return 0;
    }
    let pin = 0usize;
    let idx = |v: usize| if v > pin { v - 1 } else { v };
    let mut trip: Vec<(u32, u32, f64)> = Vec::new();
    let mut coupling = vec![0.0f64; nv - 1];
    // arestas (i, j, k oposto, peso) por triângulo
    struct E {
        i: usize,
        j: usize,
        c: f64,
        tri: usize,
        pi: DVec2,
        pj: DVec2,
    }
    let mut edges: Vec<E> = Vec::new();
    for (ti, t) in sys.tris.iter().enumerate() {
        for k in 0..3 {
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let (pi, pj, pk) = (t.p[i], t.p[j], t.p[k]);
            let (a, b) = (pi - pk, pj - pk);
            let cr = cross2(a, b).abs().max(1e-300);
            let c = (a.dot(b) / cr).max(1e-4) * t.w;
            edges.push(E { i: t.v[i] as usize, j: t.v[j] as usize, c, tri: ti, pi, pj });
        }
    }
    for e in &edges {
        for (r, cidx, v) in [(e.i, e.i, e.c), (e.j, e.j, e.c), (e.i, e.j, -e.c), (e.j, e.i, -e.c)] {
            if r == pin {
                continue;
            }
            if cidx == pin {
                coupling[idx(r)] += v;
            } else {
                trip.push((idx(r) as u32, idx(cidx) as u32, v));
            }
        }
    }
    let l = Csr::from_triplets(nv - 1, trip);
    let max_it = if params.max_cg_iterations > 0 { params.max_cg_iterations } else { (nv.max(2000)).min(20_000) };
    let mut total_it = 0;
    for _ in 0..iterations {
        // passo local: rotação mais próxima de cada triângulo
        let rot: Vec<(f64, f64)> = sys
            .tris
            .iter()
            .map(|t| {
                let qq = t.v.map(|v| q[v as usize]);
                match tri_jacobian(&t.p, qq) {
                    Some(j) => {
                        let th = (j[2] - j[1]).atan2(j[0] + j[3]);
                        (th.cos(), th.sin())
                    }
                    None => (1.0, 0.0),
                }
            })
            .collect();
        let mut bx = vec![0.0f64; nv - 1];
        let mut by = vec![0.0f64; nv - 1];
        for e in &edges {
            let (c, s) = rot[e.tri];
            let d = e.pi - e.pj;
            let rd = DVec2::new(c * d.x - s * d.y, s * d.x + c * d.y) * e.c;
            if e.i != pin {
                bx[idx(e.i)] += rd.x;
                by[idx(e.i)] += rd.y;
            }
            if e.j != pin {
                bx[idx(e.j)] -= rd.x;
                by[idx(e.j)] -= rd.y;
            }
        }
        for r in 0..nv - 1 {
            bx[r] -= coupling[r] * q[pin].x;
            by[r] -= coupling[r] * q[pin].y;
        }
        let mut xs: Vec<f64> = (0..nv).filter(|&v| v != pin).map(|v| q[v].x).collect();
        let mut ys: Vec<f64> = (0..nv).filter(|&v| v != pin).map(|v| q[v].y).collect();
        let (i1, _) = pcg(&l, &bx, &mut xs, 1e-9, max_it);
        let (i2, _) = pcg(&l, &by, &mut ys, 1e-9, max_it);
        total_it += i1 + i2;
        for v in 0..nv {
            if v != pin {
                q[v] = DVec2::new(xs[idx(v)], ys[idx(v)]);
            }
        }
    }
    total_it
}

// ---------------------------------------------------------------------- API de unwrap

/// Resultado final de uma ilha: UV por corner das faces da ilha.
struct IslandResult {
    faces: Vec<u32>,
    corner_uvs: Vec<Vec<Vec2>>,
}

fn finalize(sys: &IslandSystem, mesh: &EditMesh, q: &mut [DVec2], params: &UnwrapParams) -> IslandResult {
    if sys.net_uv_area(q) < 0.0 {
        for p in q.iter_mut() {
            p.x = -p.x;
        }
    }
    let a_uv = sys.net_uv_area(q).abs();
    let a_3d: f64 = sys.faces.iter().map(|&f| mesh.face_area(f) as f64).sum();
    if a_uv > 1e-300 && a_3d > 0.0 {
        let s = (a_3d / a_uv).sqrt();
        for p in q.iter_mut() {
            *p *= s;
        }
    }
    if params.correct_aspect && (params.aspect - 1.0).abs() > f32::EPSILON {
        for p in q.iter_mut() {
            p.x /= params.aspect as f64;
        }
    }
    let mut lo = DVec2::splat(f64::INFINITY);
    for vars in &sys.face_vars {
        for &v in vars {
            lo = lo.min(q[v as usize]);
        }
    }
    let corner_uvs = sys
        .face_vars
        .iter()
        .map(|vars| vars.iter().map(|&v| (q[v as usize] - lo).as_vec2()).collect())
        .collect();
    IslandResult { faces: sys.faces.clone(), corner_uvs }
}

fn check_mesh(mesh: &EditMesh) -> ModelResult<()> {
    mesh.validate()
}

/// **Unwrap LSCM** (ou LSCM+ARAP, conforme `params.method`) das ilhas definidas por
/// costuras. `faces = None` desdobra a malha toda; com `Some`, só essas faces mudam (arestas
/// para faces fora da seleção contam como fronteira). Equivalente ao *Unwrap → Conformal* do
/// Blender.
///
/// Os UVs atuais são **ignorados**; as ilhas vêm de `EdgeAttr::seam`, fronteiras e arestas
/// não-manifold. Cada ilha precisa ter fronteira: uma ilha fechada (ex.: caixa sem costuras)
/// devolve `UnsupportedTopology` exigindo ao menos uma costura. Todos os erros ocorrem antes
/// de qualquer escrita (malha intacta). Veja o módulo para o algoritmo e as limitações.
///
/// Com `params.pack` (padrão) as ilhas terminam empacotadas em `[0,1]²` com a margem dada;
/// sem pack cada ilha fica em unidades do mundo com o canto mínimo na origem.
pub fn unwrap_lscm(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    params: &UnwrapParams,
) -> ModelResult<UnwrapReport> {
    params.validate()?;
    let faces = resolve_faces(mesh, faces)?;
    check_mesh(mesh)?;
    let topo = Topology::build(mesh);
    let in_set = membership(mesh, &faces);
    let groups = build_islands(mesh, &topo, &faces, false);
    let wedges = compute_wedges(mesh, &topo, &in_set, false);

    let mut results: Vec<IslandResult> = Vec::with_capacity(groups.len());
    let mut report = UnwrapReport {
        islands: groups.len(),
        faces: faces.len(),
        cg_iterations: 0,
        max_residual: 0.0,
        holes_filled: 0,
        non_disk_islands: 0,
        skipped_faces: 0,
        arap_improved_islands: 0,
        pack: None,
    };
    for (gi, group) in groups.iter().enumerate() {
        let sys = IslandSystem::build(mesh, &wedges, group, params.fill_holes);
        if sys.boundary.is_empty() {
            return Err(ModelError::UnsupportedTopology(format!(
                "a ilha {gi} (face {}) é fechada e não tem fronteira: marque pelo menos uma costura",
                group[0]
            )));
        }
        if !sys.tris.iter().any(|t| t.real) {
            return Err(ModelError::Numeric(format!(
                "a ilha {gi} (face {}) só tem faces degeneradas",
                group[0]
            )));
        }
        let Some(pins) = sys.farthest_boundary_pair() else {
            return Err(ModelError::Numeric(format!(
                "a ilha {gi}: pinos de fronteira coincidentes (geometria colapsada)"
            )));
        };
        if island_euler(mesh, &wedges, group) != 1 {
            report.non_disk_islands += 1;
        }
        let LscmOutcome { mut q, iterations, residual } = lscm_solve(&sys, pins, params)?;
        report.cg_iterations += iterations;
        report.max_residual = report.max_residual.max(residual);
        report.holes_filled += sys.holes_filled;
        report.skipped_faces += sys.skipped_faces;
        if params.method == UnwrapMethod::LscmArap && params.arap_iterations > 0 {
            let before = quality(&sys, &q);
            let mut q2 = q.clone();
            arap_refine(&sys, &mut q2, params.arap_iterations, params);
            let after = quality(&sys, &q2);
            if q2.iter().all(|p| p.is_finite()) && after.1 <= before.1 && after.0 <= before.0 {
                q = q2;
                report.arap_improved_islands += 1;
            }
        }
        results.push(finalize(&sys, mesh, &mut q, params));
    }

    // Escrita (nenhum erro possível a partir daqui, exceto o pack, que também valida antes).
    let backup: Vec<(u32, Vec<Vec2>)> = if params.pack {
        faces
            .iter()
            .map(|&f| (f, mesh.faces[f as usize].corners.iter().map(|c| c.uv).collect()))
            .collect()
    } else {
        Vec::new()
    };
    for r in &results {
        for (fi, &f) in r.faces.iter().enumerate() {
            for (c, uv) in mesh.faces[f as usize].corners.iter_mut().zip(&r.corner_uvs[fi]) {
                c.uv = *uv;
            }
        }
    }
    if params.pack {
        let islands: Vec<Island> = results
            .iter()
            .map(|r| Island { bounds_uv: faces_uv_bounds(mesh, &r.faces), faces: r.faces.clone() })
            .collect();
        let rotate = !(params.correct_aspect && (params.aspect - 1.0).abs() > f32::EPSILON);
        let pp = PackParams {
            margin: params.margin * 1024.0,
            texture_size: 1024,
            rotate,
            ..PackParams::default()
        };
        match pack_island_list(mesh, &islands, &pp) {
            Ok(rep) => report.pack = Some(rep),
            Err(e) => {
                for (f, uvs) in backup {
                    for (c, uv) in mesh.faces[f as usize].corners.iter_mut().zip(uvs) {
                        c.uv = uv;
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(report)
}

/// Relaxamento de estiramento (ARAP) sobre os UVs atuais da malha toda. Ver
/// [`unwrap_minimum_stretch_in`].
pub fn unwrap_minimum_stretch(mesh: &mut EditMesh, iterations: u32) -> ModelResult<StretchReport> {
    unwrap_minimum_stretch_in(mesh, None, iterations)
}

/// Reduz o estiramento dos UVs existentes (*Minimize Stretch* do Blender) com `iterations`
/// passos ARAP local/global por ilha (ilhas = costuras **e** descontinuidades de UV). Usa o
/// laplaciano de cotangentes do 3D, onde cada triângulo puxa o UV para a forma isométrica
/// mais próxima (rotação ótima), ponderado pelas áreas 3D vs. UV atuais.
///
/// A fronteira também se move (apenas um vértice fica fixo). Cada ilha só é atualizada se a
/// distorção isométrica média `max(σ1/s, s/σ2)` diminuir sem criar triângulos invertidos; caso contrário é
/// revertida. A ilha mantém a área UV e o centro da caixa que tinha. Pode introduzir
/// sobreposições entre ilhas vizinhas no layout: reempacote se necessário.
///
/// Erros: `iterations == 0` ou `> 10000` (`InvalidParameter`), face inválida, UV não finito.
pub fn unwrap_minimum_stretch_in(
    mesh: &mut EditMesh,
    faces: Option<&[u32]>,
    iterations: u32,
) -> ModelResult<StretchReport> {
    if iterations == 0 || iterations > 10_000 {
        return Err(ModelError::InvalidParameter(format!(
            "iterations deve estar em 1..=10000, recebeu {iterations}"
        )));
    }
    let faces = resolve_faces(mesh, faces)?;
    check_mesh(mesh)?;
    for &f in &faces {
        if mesh.faces[f as usize].corners.iter().any(|c| !c.uv.is_finite()) {
            return Err(ModelError::Numeric(format!("UV não finito na face {f}")));
        }
    }
    let topo = Topology::build(mesh);
    let in_set = membership(mesh, &faces);
    let groups = build_islands(mesh, &topo, &faces, true);
    let wedges = compute_wedges(mesh, &topo, &in_set, true);
    let params = UnwrapParams::default();
    let mut rep = StretchReport { islands: groups.len(), improved: 0, mean_before: 1.0, mean_after: 1.0 };
    let (mut nb, mut na, mut den) = (0.0f64, 0.0f64, 0.0f64);
    let mut updates: Vec<IslandResult> = Vec::new();
    for group in &groups {
        let sys = IslandSystem::build(mesh, &wedges, group, true);
        let q0 = sys.uvs_from_mesh(mesh);
        let (mb, fb, wb) = quality(&sys, &q0);
        if wb <= 0.0 {
            continue;
        }
        let mut q = q0.clone();
        arap_refine(&sys, &mut q, iterations, &params);
        let (ma, fa, _) = quality(&sys, &q);
        let ok = q.iter().all(|p| p.is_finite()) && fa <= fb && ma < mb * (1.0 - 1e-6);
        nb += mb * wb;
        den += wb;
        if !ok {
            na += mb * wb;
            continue;
        }
        na += ma * wb;
        rep.improved += 1;
        // preserva área e centro da caixa de cada ilha
        let area_old = sys.net_uv_area(&q0).abs();
        let area_new = sys.net_uv_area(&q).abs();
        let bounds = |q: &[DVec2]| {
            let mut lo = DVec2::splat(f64::INFINITY);
            let mut hi = DVec2::splat(f64::NEG_INFINITY);
            for vars in &sys.face_vars {
                for &v in vars {
                    lo = lo.min(q[v as usize]);
                    hi = hi.max(q[v as usize]);
                }
            }
            (lo + hi) * 0.5
        };
        let (c_old, c_new) = (bounds(&q0), bounds(&q));
        let s = if area_new > 1e-300 { (area_old / area_new).sqrt() } else { 1.0 };
        let corner_uvs = sys
            .face_vars
            .iter()
            .map(|vars| vars.iter().map(|&v| (c_old + (q[v as usize] - c_new) * s).as_vec2()).collect())
            .collect();
        updates.push(IslandResult { faces: sys.faces.clone(), corner_uvs });
    }
    for r in &updates {
        for (fi, &f) in r.faces.iter().enumerate() {
            for (c, uv) in mesh.faces[f as usize].corners.iter_mut().zip(&r.corner_uvs[fi]) {
                c.uv = *uv;
            }
        }
    }
    if den > 0.0 {
        rep.mean_before = (nb / den) as f32;
        rep.mean_after = (na / den) as f32;
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::uv::metrics::{flipped_faces, island_stretch_stats, overlap_count, self_overlapping_faces, uv_hash};
    use crate::uv::seams::{mark_seams, seam_path, split_islands};
    use glam::{Quat, Vec3};
    use std::collections::BTreeMap as Map;

    fn no_pack() -> UnwrapParams {
        UnwrapParams { pack: false, ..Default::default() }
    }

    /// UV por vértice (assume um único UV por vértice; falha se divergir).
    fn vert_uvs(m: &EditMesh) -> Map<u32, Vec2> {
        let mut out: Map<u32, Vec2> = Map::new();
        for f in &m.faces {
            for c in &f.corners {
                if let Some(old) = out.insert(c.vert, c.uv) {
                    assert!((old - c.uv).length() < 1e-6, "vértice {} com 2 UVs", c.vert);
                }
            }
        }
        out
    }

    fn mean_stretch(m: &EditMesh) -> f32 {
        let st = island_stretch_stats(m);
        st.iter().map(|s| s.mean * s.faces as f32).sum::<f32>() / st.iter().map(|s| s.faces as f32).sum::<f32>()
    }

    fn hemisphere(rings: u32, sectors: u32) -> EditMesh {
        let mut m = primitives::uv_sphere(1.0, rings, sectors);
        let kill: Vec<u32> = (0..m.face_count() as u32)
            .filter(|&f| m.face_center(f).y < -1e-4)
            .collect();
        m.delete_faces(&kill).unwrap();
        m.remove_unused_verts();
        m
    }

    fn open_cylinder_with_seam(sectors: u32, rings: u32) -> EditMesh {
        let mut m = primitives::cylinder(1.0, 2.0, sectors, rings, false);
        let path = seam_path(&m, 0, rings * sectors).unwrap();
        assert_eq!(path.len() as u32, rings);
        mark_seams(&mut m, &path, true).unwrap();
        m
    }

    // ---------------- álgebra

    #[test]
    fn csr_sums_duplicates_and_pcg_solves_spd_system() {
        // laplaciano 1D com Dirichlet: [2 -1; -1 2 -1; -1 2] x = b
        let n = 50;
        let mut t = Vec::new();
        for i in 0..n {
            t.push((i as u32, i as u32, 1.0));
            t.push((i as u32, i as u32, 1.0)); // duplicata: soma 2
            if i + 1 < n {
                t.push((i as u32, i as u32 + 1, -1.0));
                t.push((i as u32 + 1, i as u32, -1.0));
            }
        }
        let a = Csr::from_triplets(n, t);
        assert_eq!(a.diag()[3], 2.0);
        let xs: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
        let mut b = vec![0.0; n];
        a.mul(&xs, &mut b);
        let mut x = vec![0.0; n];
        let (it, res) = pcg(&a, &b, &mut x, 1e-12, 500);
        assert!(res < 1e-10 && it <= n + 5, "it {it} res {res}");
        assert!(x.iter().zip(&xs).all(|(a, b)| (a - b).abs() < 1e-7));
        // b = 0 => solução nula, zero iterações
        let mut x0 = vec![0.0; n];
        assert_eq!(pcg(&a, &vec![0.0; n], &mut x0, 1e-12, 10).0, 0);
    }

    // ---------------- isometria e casos felizes

    #[test]
    fn inclined_plane_is_reproduced_isometrically() {
        let mut m = primitives::grid(6, 4, 3.0, 2.0);
        let q = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 0.5).normalize(), 0.9);
        m.transform_positions(|p| q * p + Vec3::new(5.0, -2.0, 1.0));
        let rep = unwrap_lscm(&mut m, None, &no_pack()).unwrap();
        assert_eq!(rep.islands, 1);
        assert_eq!(rep.non_disk_islands, 0);
        assert!(rep.max_residual < 1e-8);
        let uv = vert_uvs(&m);
        let ids: Vec<u32> = uv.keys().copied().collect();
        let mut worst = 0.0f32;
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                let d3 = (m.co(a) - m.co(b)).length();
                let d2 = (uv[&a] - uv[&b]).length();
                worst = worst.max((d3 - d2).abs());
            }
        }
        assert!(worst < 1e-3, "erro de isometria {worst}");
        assert!(flipped_faces(&m).is_empty());
    }

    #[test]
    fn inclined_plane_after_packing_keeps_constant_scale() {
        let mut m = primitives::grid(5, 5, 2.0, 2.0);
        let q = Quat::from_rotation_x(0.7) * Quat::from_rotation_z(0.3);
        m.transform_positions(|p| q * p);
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        let pk = rep.pack.unwrap();
        let uv = vert_uvs(&m);
        let ids: Vec<u32> = uv.keys().copied().collect();
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                let d3 = (m.co(a) - m.co(b)).length();
                let d2 = (uv[&a] - uv[&b]).length();
                assert!((d2 / d3 - pk.scale).abs() < 2e-3 * pk.scale.max(1.0), "razão {} vs escala {}", d2 / d3, pk.scale);
            }
        }
        let (lo, hi) = crate::uv::metrics::uv_bounds(&m).unwrap();
        assert!(lo.min_element() >= 0.0 && hi.max_element() <= 1.0 + 1e-6);
    }

    #[test]
    fn open_cylinder_with_seam_unrolls_isometrically_without_flips() {
        let mut m = open_cylinder_with_seam(16, 6);
        let rep = unwrap_lscm(&mut m, None, &no_pack()).unwrap();
        assert_eq!(rep.islands, 1);
        assert_eq!(rep.non_disk_islands, 0);
        assert!(flipped_faces(&m).is_empty());
        assert!(mean_stretch(&m) < 1.01, "estiramento {}", mean_stretch(&m));
        assert!(self_overlapping_faces(&m).is_empty());
        // vértices da costura têm UVs diferentes nos dois lados
        let mut seam_uvs: Map<u32, Vec<Vec2>> = Map::new();
        for f in &m.faces {
            for c in &f.corners {
                seam_uvs.entry(c.vert).or_default().push(c.uv);
            }
        }
        let two = seam_uvs[&0].iter().any(|u| (*u - seam_uvs[&0][0]).length() > 1.0);
        assert!(two, "vértice de costura deveria ter dois UVs distantes");
        // as duas cópias do vértice de costura distam 16 cordas (perímetro do polígono)
        let circ = 2.0 * 16.0 * (std::f32::consts::PI / 16.0).sin();
        let d = (seam_uvs[&0][0] - seam_uvs[&0][1]).length();
        assert!((d - circ).abs() < 1e-2, "{d} vs {circ}");
    }

    #[test]
    fn hemisphere_unwraps_conformally_without_flips() {
        let mut m = hemisphere(12, 24);
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        assert_eq!(rep.islands, 1);
        assert_eq!(rep.non_disk_islands, 0);
        assert!(flipped_faces(&m).is_empty(), "faces invertidas: {:?}", flipped_faces(&m));
        let ms = mean_stretch(&m);
        assert!(ms < 1.5, "estiramento médio {ms}");
        assert!(self_overlapping_faces(&m).is_empty());
        m.validate().unwrap();
    }

    #[test]
    fn sphere_cut_at_equator_gives_two_clean_islands() {
        let mut m = primitives::uv_sphere(1.0, 12, 24);
        // anel equatorial: vértices com y ~ 0 (ring 6 => índices 1 + 5*24 ..)
        let ring: Vec<u32> = (0..m.vert_count() as u32).filter(|&v| m.co(v).y.abs() < 1e-5).collect();
        assert_eq!(ring.len(), 24);
        let topo = Topology::build(&m);
        let edges: Vec<_> = topo
            .edges
            .iter()
            .map(|e| e.verts)
            .filter(|(a, b)| ring.contains(a) && ring.contains(b))
            .collect();
        assert_eq!(edges.len(), 24);
        mark_seams(&mut m, &edges, true).unwrap();
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        assert_eq!(rep.islands, 2);
        assert_eq!(rep.non_disk_islands, 0);
        assert!(flipped_faces(&m).is_empty());
        assert_eq!(overlap_count(&m), 0);
        assert!(self_overlapping_faces(&m).is_empty());
        assert!(mean_stretch(&m) < 1.5, "{}", mean_stretch(&m));
        assert_eq!(split_islands(&m).len(), 2);
    }

    #[test]
    fn cube_with_cross_seams_is_one_island_and_unfolds() {
        let mut m = primitives::cube(1.0);
        // árvore geradora do dual => cruz
        let topo = Topology::build(&m);
        let mut dsu = crate::uv::seams::Dsu::new(m.faces.len());
        let mut cut = Vec::new();
        for e in &topo.edges {
            if !(e.faces.len() == 2 && dsu.union(e.faces[0], e.faces[1])) {
                cut.push(e.verts);
            }
        }
        assert_eq!(cut.len(), 7);
        mark_seams(&mut m, &cut, true).unwrap();
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        assert_eq!(rep.islands, 1);
        assert_eq!(rep.non_disk_islands, 0);
        assert_eq!(split_islands(&m).len(), 1);
        assert!(flipped_faces(&m).is_empty());
        assert!(self_overlapping_faces(&m).is_empty());
    }

    #[test]
    fn cube_with_all_edges_seamed_gives_six_non_overlapping_squares() {
        let mut m = primitives::cube(2.0);
        let all: Vec<_> = Topology::build(&m).edges.iter().map(|e| e.verts).collect();
        mark_seams(&mut m, &all, true).unwrap();
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        assert_eq!(rep.islands, 6);
        assert_eq!(overlap_count(&m), 0);
        assert!(flipped_faces(&m).is_empty());
        assert!(mean_stretch(&m) < 1.001);
    }

    #[test]
    fn ngon_circle_and_mixed_polygons_unwrap_isometrically() {
        let mut m = primitives::circle(1.5, 12, false);
        unwrap_lscm(&mut m, None, &no_pack()).unwrap();
        assert!(mean_stretch(&m) < 1.001, "{}", mean_stretch(&m));
        assert!(flipped_faces(&m).is_empty());
        // pentágono + quad + triângulo coplanares
        let mut p = EditMesh::new("misto");
        for q in [
            (0.0, 0.0), (1.0, 0.0), (2.0, 0.2), (1.5, 1.2), (0.3, 1.0), // pentágono
            (3.0, 0.0), (3.0, 1.0), (2.2, 2.0),
        ] {
            p.add_vert(Vec3::new(q.0, 0.0, -q.1));
        }
        p.add_face(&[0, 1, 2, 3, 4]).unwrap();
        p.add_face(&[2, 5, 6, 3]).unwrap();
        p.add_face(&[3, 6, 7]).unwrap();
        let rep = unwrap_lscm(&mut p, None, &no_pack()).unwrap();
        assert_eq!(rep.islands, 1);
        assert!(mean_stretch(&p) < 1.001, "{}", mean_stretch(&p));
        assert!(flipped_faces(&p).is_empty());
    }

    #[test]
    fn concave_quad_uses_only_valid_diagonal() {
        // quad côncavo (dardo) no plano
        let mut m = EditMesh::new("dardo");
        for q in [(0.0, 0.0), (2.0, 0.0), (0.5, 0.5), (0.0, 2.0)] {
            m.add_vert(Vec3::new(q.0, 0.0, -q.1));
        }
        m.add_face(&[0, 1, 2, 3]).unwrap();
        unwrap_lscm(&mut m, None, &no_pack()).unwrap();
        assert!(flipped_faces(&m).is_empty());
        assert!(mean_stretch(&m) < 1.001);
    }

    #[test]
    fn tube_without_seam_reports_non_disk_and_fills_hole() {
        let mut m = primitives::cylinder(1.0, 2.0, 12, 4, false);
        let rep = unwrap_lscm(&mut m, None, &no_pack()).unwrap();
        assert_eq!(rep.non_disk_islands, 1);
        assert_eq!(rep.holes_filled, 1);
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
        let rep2 = unwrap_lscm(&mut m, None, &UnwrapParams { fill_holes: false, pack: false, ..Default::default() }).unwrap();
        assert_eq!(rep2.holes_filled, 0);
    }

    #[test]
    fn lscm_arap_trades_angle_for_area_distortion_without_flips() {
        use crate::uv::metrics::mean_area_distortion;
        for base in [hemisphere(10, 20), {
            let mut s = primitives::quad_sphere(1.0, 6);
            crate::uv::seams::auto_seams_for_character(&mut s).unwrap();
            s
        }] {
            let mut a = base.clone();
            let mut b = base.clone();
            unwrap_lscm(&mut a, None, &no_pack()).unwrap();
            let rep = unwrap_lscm(
                &mut b,
                None,
                &UnwrapParams { method: UnwrapMethod::LscmArap, pack: false, ..Default::default() },
            )
            .unwrap();
            assert!(rep.arap_improved_islands <= 1);
            // a distorção isométrica (que o ARAP minimiza) nunca piora: ou melhora ou reverte
            assert!(mean_area_distortion(&b) <= mean_area_distortion(&a) + 1e-4, "{} vs {}", mean_area_distortion(&b), mean_area_distortion(&a));
            assert!(flipped_faces(&b).len() <= flipped_faces(&a).len());
            assert!(self_overlapping_faces(&b).is_empty());
        }
    }

    #[test]
    fn aspect_correction_scales_u_axis() {
        let mut m = primitives::grid(2, 2, 1.0, 1.0);
        unwrap_lscm(&mut m, None, &UnwrapParams { pack: false, correct_aspect: true, aspect: 2.0, ..Default::default() }).unwrap();
        let (lo, hi) = crate::uv::metrics::uv_bounds(&m).unwrap();
        let ext = hi - lo;
        assert!((ext.x / ext.y - 0.5).abs() < 1e-3 || (ext.y / ext.x - 0.5).abs() < 1e-3, "{ext:?}");
        let mut k = primitives::grid(2, 2, 1.0, 1.0);
        unwrap_lscm(&mut k, None, &UnwrapParams { pack: false, correct_aspect: false, aspect: 2.0, ..Default::default() }).unwrap();
        let (lo, hi) = crate::uv::metrics::uv_bounds(&k).unwrap();
        assert!(((hi - lo).x - (hi - lo).y).abs() < 1e-3);
    }

    // ---------------- erros e atomicidade

    #[test]
    fn closed_box_without_seams_requires_a_seam_and_leaves_mesh_intact() {
        let mut m = primitives::cube(1.0);
        let before = m.clone();
        let h = uv_hash(&m);
        let err = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap_err();
        assert!(matches!(err, ModelError::UnsupportedTopology(ref s) if s.contains("costura")), "{err:?}");
        assert_eq!(m, before);
        assert_eq!(uv_hash(&m), h);
        // esfera fechada idem
        let mut s = primitives::quad_sphere(1.0, 4);
        assert!(matches!(unwrap_lscm(&mut s, None, &UnwrapParams::default()), Err(ModelError::UnsupportedTopology(_))));
    }

    #[test]
    fn one_bad_island_aborts_everything_without_partial_writes() {
        // um plano aberto (ok) + um cubo fechado (erro) na mesma malha
        let mut m = primitives::grid(2, 2, 1.0, 1.0);
        m.append(&primitives::cube(1.0));
        let h = uv_hash(&m);
        assert!(unwrap_lscm(&mut m, None, &UnwrapParams::default()).is_err());
        assert_eq!(uv_hash(&m), h);
    }

    #[test]
    fn invalid_parameters_and_selections_are_rejected() {
        let mut m = primitives::grid(2, 2, 1.0, 1.0);
        let h = uv_hash(&m);
        for p in [
            UnwrapParams { margin: -0.1, ..Default::default() },
            UnwrapParams { margin: 0.6, ..Default::default() },
            UnwrapParams { margin: f32::NAN, ..Default::default() },
            UnwrapParams { aspect: 0.0, ..Default::default() },
            UnwrapParams { aspect: f32::INFINITY, ..Default::default() },
            UnwrapParams { tolerance: 0.0, ..Default::default() },
            UnwrapParams { tolerance: 2.0, ..Default::default() },
            UnwrapParams { arap_iterations: 100_000, ..Default::default() },
        ] {
            assert!(matches!(unwrap_lscm(&mut m, None, &p), Err(ModelError::InvalidParameter(_))), "{p:?}");
        }
        assert!(matches!(unwrap_lscm(&mut m, Some(&[]), &UnwrapParams::default()), Err(ModelError::EmptySelection(_))));
        assert!(matches!(unwrap_lscm(&mut m, Some(&[99]), &UnwrapParams::default()), Err(ModelError::FaceOutOfRange(99, 4))));
        assert!(matches!(unwrap_lscm(&mut EditMesh::new("v"), None, &UnwrapParams::default()), Err(ModelError::EmptySelection(_))));
        assert_eq!(uv_hash(&m), h);
        // vértice não finito
        let mut bad = primitives::grid(1, 1, 1.0, 1.0);
        bad.verts[0].co.x = f32::NAN;
        assert!(matches!(unwrap_lscm(&mut bad, None, &UnwrapParams::default()), Err(ModelError::Numeric(_))));
    }

    #[test]
    fn degenerate_geometry_is_reported_not_panicking() {
        // todos os pontos coincidentes
        let mut m = EditMesh::new("colapsada");
        for _ in 0..3 {
            m.add_vert(Vec3::ZERO);
        }
        m.add_face(&[0, 1, 2]).unwrap();
        assert!(matches!(unwrap_lscm(&mut m, None, &UnwrapParams::default()), Err(ModelError::Numeric(_))));
        // uma face degenerada no meio de faces boas é ignorada
        let mut g = primitives::grid(2, 1, 2.0, 1.0);
        let extra = g.add_vert(Vec3::new(5.0, 0.0, 0.0));
        let extra2 = g.add_vert(Vec3::new(6.0, 0.0, 0.0));
        let extra3 = g.add_vert(Vec3::new(7.0, 0.0, 0.0));
        g.add_face(&[extra, extra2, extra3]).unwrap();
        let rep = unwrap_lscm(&mut g, None, &no_pack());
        assert!(matches!(rep, Err(ModelError::Numeric(_))), "ilha só com face degenerada: {rep:?}");
    }

    #[test]
    fn subset_unwrap_touches_only_selected_faces() {
        let mut m = primitives::grid(4, 4, 4.0, 4.0);
        let before = m.clone();
        let sel: Vec<u32> = (0..16).filter(|f| f % 4 < 2).collect();
        unwrap_lscm(&mut m, Some(&sel), &no_pack()).unwrap();
        for f in 0..16usize {
            if sel.contains(&(f as u32)) {
                continue;
            }
            assert_eq!(m.faces[f], before.faces[f], "face {f} fora da seleção mudou");
        }
        assert!(sel.iter().any(|&f| m.faces[f as usize] != before.faces[f as usize]));
    }

    // ---------------- determinismo / escala

    #[test]
    fn unwrap_is_deterministic_bit_by_bit() {
        let run = |method| {
            let mut m = hemisphere(10, 20);
            unwrap_lscm(&mut m, None, &UnwrapParams { method, ..Default::default() }).unwrap();
            uv_hash(&m)
        };
        assert_eq!(run(UnwrapMethod::Lscm), run(UnwrapMethod::Lscm));
        assert_eq!(run(UnwrapMethod::LscmArap), run(UnwrapMethod::LscmArap));
    }

    #[test]
    fn large_mesh_unwrap_runs_in_reasonable_time() {
        let mut m = primitives::grid(70, 70, 7.0, 7.0); // 4900 quads
        m.transform_positions(|p| Vec3::new(p.x, (p.x * 0.5).sin() * 0.3, p.z));
        let t = std::time::Instant::now();
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        let el = t.elapsed();
        assert!(el.as_secs() < 120, "demorou {el:?} ({} its)", rep.cg_iterations);
        assert!(flipped_faces(&m).is_empty());
        assert!(mean_stretch(&m) < 1.05);
    }

    #[test]
    fn auto_seamed_quad_sphere_unwraps_without_nans() {
        let mut m = primitives::quad_sphere(1.0, 12);
        crate::uv::seams::auto_seams_for_character(&mut m).unwrap();
        let rep = unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        assert_eq!(rep.non_disk_islands, 0);
        assert!(m.faces.iter().all(|f| f.corners.iter().all(|c| c.uv.is_finite())));
        assert!(rep.pack.unwrap().used_area_ratio > 0.0);
    }

    // ---------------- minimum stretch

    #[test]
    fn minimum_stretch_reduces_distortion_of_a_bad_parameterization() {
        // plano com UV deformado (compressão não linear): ARAP deve recuperar ~isometria
        let mut m = primitives::grid(8, 8, 2.0, 2.0);
        for f in &mut m.faces {
            for c in &mut f.corners {
                let u = c.uv.x;
                let v = c.uv.y;
                c.uv = Vec2::new(u * u, 1.0 - v); // enrolamento anti-horário
            }
        }
        let before = mean_stretch(&m);
        let rep = unwrap_minimum_stretch(&mut m, 15).unwrap();
        let after = mean_stretch(&m);
        assert!(before > 1.3, "{before}");
        assert!(after < before * 0.7, "{before} -> {after}");
        assert_eq!(rep.improved, 1);
        assert!(rep.mean_after < rep.mean_before);
        assert!(flipped_faces(&m).is_empty());
        // preserva área total e centro
        let (lo, hi) = crate::uv::metrics::uv_bounds(&m).unwrap();
        assert!(((lo + hi) * 0.5 - Vec2::splat(0.5)).abs().max_element() < 0.3);
    }

    #[test]
    fn minimum_stretch_keeps_already_isometric_uvs_and_validates_input() {
        let mut m = primitives::grid(3, 3, 3.0, 3.0);
        for f in &mut m.faces {
            for c in &mut f.corners {
                c.uv = Vec2::new(c.uv.x * 3.0, (1.0 - c.uv.y) * 3.0);
            }
        }
        let h = uv_hash(&m);
        let rep = unwrap_minimum_stretch(&mut m, 5).unwrap();
        assert_eq!(rep.improved, 0, "nada a melhorar");
        assert_eq!(uv_hash(&m), h, "ilha revertida deve manter bits");
        assert!(matches!(unwrap_minimum_stretch(&mut m, 0), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(unwrap_minimum_stretch(&mut m, 20_000), Err(ModelError::InvalidParameter(_))));
        assert!(matches!(unwrap_minimum_stretch_in(&mut m, Some(&[500]), 3), Err(ModelError::FaceOutOfRange(500, _))));
        let mut nan = m.clone();
        nan.faces[0].corners[0].uv = Vec2::new(f32::NAN, 0.0);
        assert!(matches!(unwrap_minimum_stretch(&mut nan, 3), Err(ModelError::Numeric(_))));
    }

    #[test]
    fn minimum_stretch_after_lscm_does_not_increase_isometric_distortion() {
        use crate::uv::metrics::mean_area_distortion;
        let mut m = hemisphere(10, 20);
        unwrap_lscm(&mut m, None, &UnwrapParams::default()).unwrap();
        let before = mean_area_distortion(&m);
        let flips = flipped_faces(&m).len();
        let rep = unwrap_minimum_stretch(&mut m, 8).unwrap();
        assert!(mean_area_distortion(&m) <= before + 1e-4, "{before} -> {}", mean_area_distortion(&m));
        assert!(flipped_faces(&m).len() <= flips);
        assert!(rep.mean_after <= rep.mean_before + 1e-6);
        assert!(self_overlapping_faces(&m).is_empty());
    }
}
