//! Voxel Remesh: refaz a malha como uma superfície uniforme de quads a partir
//! do campo de distância assinada (SDF) da malha original — equivalente ao
//! *Voxel Remesh* do Blender Sculpt Mode. Implementação própria, sem crates.
//!
//! Pipeline de [`voxel_remesh`]:
//! 1. **Grade densa** com `padding` voxels de margem; erro claro se exceder
//!    [`VoxelParams::max_cells`].
//! 2. **Distância não assinada** apenas numa faixa estreita ao redor da
//!    superfície (distância exata ponto-triângulo, por triângulo).
//! 3. **Sinal** por *winding number* ao longo de colunas Z: cada triângulo que
//!    a coluna atravessa soma ±1 conforme a orientação. `winding ≠ 0` = dentro.
//!    Isso funde malhas sobrepostas (união) e independe da orientação global,
//!    ao contrário da paridade. Pontos de amostragem são perturbados e as
//!    arestas usam predicados antissimétricos em f64, então o raio nunca
//!    "vaza" entre triângulos vizinhos.
//! 4. **Surface Nets ingênuo**: um vértice por célula que cruza a superfície
//!    (média dos cruzamentos de aresta) e um quad por aresta de grade com
//!    mudança de sinal, orientado para fora.
//! 5. Suavização opcional (Taubin λ|μ, preserva volume) e **transferência de
//!    atributos** (cor/máscara/vinco por baricêntricas, skin do vértice mais
//!    próximo, material/face set/suave da face mais próxima).
//!
//! Limitações: exige malha fechada por padrão ([`VoxelParams::require_closed`]);
//! UVs, normais customizadas e atributos de aresta (sharp/seam/vinco/bevel) são
//! perdidos (como no Blender); estruturas mais finas que ~1 voxel podem sumir
//! ou gerar vértices não-manifold (o relatório conta arestas não-manifold).

use crate::bvh::{closest_point_on_triangle, TriBvh};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{EVert, EditMesh, Face};
use crate::topology::Topology;
use glam::Vec3;
use std::collections::HashMap;

/// Parâmetros do Voxel Remesh.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelParams {
    /// Limite de células da grade densa (erro se exceder).
    pub max_cells: usize,
    /// Margem da grade em voxels (≥ 3).
    pub padding: u32,
    /// Largura da faixa de distância em voxels (≥ 1.5; o necessário é ~1.2).
    pub band_voxels: f32,
    /// Iterações de suavização Taubin (0 = nenhuma).
    pub smooth_iterations: u32,
    /// Intensidade λ da suavização em `(0, 1]`.
    pub smooth_factor: f32,
    /// Transfere cor/máscara/skin/material/face set da malha original.
    pub transfer_attributes: bool,
    /// Recusa malhas com arestas de fronteira (o sinal fica indefinido).
    pub require_closed: bool,
}

impl Default for VoxelParams {
    fn default() -> Self {
        Self {
            max_cells: 16_000_000,
            padding: 3,
            band_voxels: 2.0,
            smooth_iterations: 0,
            smooth_factor: 0.5,
            transfer_attributes: true,
            require_closed: true,
        }
    }
}

impl VoxelParams {
    pub fn validate(&self) -> ModelResult<()> {
        if self.max_cells < 64 {
            return Err(ModelError::InvalidParameter(format!(
                "max_cells muito pequeno: {}",
                self.max_cells
            )));
        }
        if self.padding < 3 {
            return Err(ModelError::InvalidParameter(format!(
                "padding deve ser ≥ 3 voxels: {}",
                self.padding
            )));
        }
        if !(self.band_voxels >= 1.5) || !self.band_voxels.is_finite() {
            return Err(ModelError::InvalidParameter(format!(
                "band_voxels deve ser ≥ 1.5: {}",
                self.band_voxels
            )));
        }
        if !(self.smooth_factor > 0.0 && self.smooth_factor <= 1.0) {
            return Err(ModelError::InvalidParameter(format!(
                "smooth_factor deve estar em (0, 1]: {}",
                self.smooth_factor
            )));
        }
        Ok(())
    }
}

/// Estatísticas do remesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoxelReport {
    /// Pontos da grade por eixo.
    pub grid_dims: [usize; 3],
    /// Total de pontos da grade.
    pub grid_points: usize,
    /// Células que cruzam a superfície (= vértices gerados).
    pub active_cells: usize,
    pub quads: usize,
    /// Arestas com ≠ 2 faces no resultado (0 = manifold fechado).
    pub non_manifold_edges: usize,
    /// Arestas com 1 face no resultado.
    pub boundary_edges: usize,
}

/// Refaz `mesh` com voxels de lado `voxel_size`. Veja o módulo para o algoritmo.
pub fn voxel_remesh(mesh: &EditMesh, voxel_size: f32, params: &VoxelParams) -> ModelResult<EditMesh> {
    voxel_remesh_report(mesh, voxel_size, params).map(|(m, _)| m)
}

/// Como [`voxel_remesh`], devolvendo também o [`VoxelReport`].
pub fn voxel_remesh_report(
    mesh: &EditMesh,
    voxel_size: f32,
    params: &VoxelParams,
) -> ModelResult<(EditMesh, VoxelReport)> {
    // ---------------------------------------------------------- validação
    params.validate()?;
    if !(voxel_size > 0.0) || !voxel_size.is_finite() {
        return Err(ModelError::InvalidParameter(format!(
            "tamanho de voxel inválido: {voxel_size}"
        )));
    }
    mesh.validate()?;
    if mesh.faces.is_empty() {
        return Err(ModelError::EmptySelection("malha sem faces"));
    }
    let topo = Topology::build(mesh);
    if params.require_closed && topo.boundary_edge_count() > 0 {
        return Err(ModelError::UnsupportedTopology(format!(
            "a malha tem {} aresta(s) de fronteira; o voxel remesh exige malha fechada",
            topo.boundary_edge_count()
        )));
    }
    let bvh = TriBvh::build(mesh);
    let Some(bounds) = bvh.bounds() else {
        return Err(ModelError::EmptySelection("malha sem triângulos"));
    };
    let h = voxel_size;
    let pad = params.padding as f32 * h;
    let origin = bounds.min - Vec3::splat(pad);
    let ext = bounds.max - bounds.min + Vec3::splat(2.0 * pad);
    let dims_f = (ext / h).ceil() + Vec3::ONE;
    if !dims_f.is_finite() {
        return Err(ModelError::Numeric("extensão da grade não finita".into()));
    }
    let total = (dims_f.x as f64) * (dims_f.y as f64) * (dims_f.z as f64);
    if total > params.max_cells as f64 {
        return Err(ModelError::InvalidParameter(format!(
            "a grade teria {:.0} células ({}×{}×{}), acima do limite de {}; aumente voxel_size ou max_cells",
            total, dims_f.x, dims_f.y, dims_f.z, params.max_cells
        )));
    }
    let dims = [dims_f.x as usize, dims_f.y as usize, dims_f.z as usize];
    let (nx, ny, nz) = (dims[0], dims[1], dims[2]);
    let npoints = nx * ny * nz;
    let idx = |i: usize, j: usize, k: usize| (k * ny + j) * nx + i;
    let gp = |i: usize, j: usize, k: usize| origin + Vec3::new(i as f32, j as f32, k as f32) * h;

    // ------------------------------------------- distância não assinada (faixa)
    let band_w = params.band_voxels * h;
    let far = band_w + h;
    let mut sdf = vec![far; npoints];
    let ntri = bvh.tri_count() as u32;
    let range = |lo: f32, hi: f32, o: f32, n: usize| -> Option<(usize, usize)> {
        let a = ((lo - o) / h).floor();
        let b = ((hi - o) / h).ceil();
        if b < 0.0 || a > (n - 1) as f32 {
            return None;
        }
        Some((a.max(0.0) as usize, (b as usize).min(n - 1)))
    };
    for t in 0..ntri {
        let [a, b, c] = bvh.triangle(t);
        let lo = a.min(b).min(c) - Vec3::splat(band_w);
        let hi = a.max(b).max(c) + Vec3::splat(band_w);
        let (Some((i0, i1)), Some((j0, j1)), Some((k0, k1))) = (
            range(lo.x, hi.x, origin.x, nx),
            range(lo.y, hi.y, origin.y, ny),
            range(lo.z, hi.z, origin.z, nz),
        ) else {
            continue;
        };
        for k in k0..=k1 {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let p = gp(i, j, k);
                    let (q, _) = closest_point_on_triangle(p, a, b, c);
                    let d = q.distance(p);
                    let s = &mut sdf[idx(i, j, k)];
                    if d < *s {
                        *s = d;
                    }
                }
            }
        }
    }

    // ------------------------------------------ sinal por winding number (colunas Z)
    // perturbação fixa do ponto de amostragem para evitar empates exatos
    let (ex, ey) = (0.012_345_678_9 * h, 0.009_876_543_1 * h);
    let mut crossings: Vec<(u32, f32, i8)> = Vec::new();
    for t in 0..ntri {
        let [a, b, c] = bvh.triangle(t);
        let na = (b - a).cross(c - a);
        let (nxd, nyd, nzd) = (na.x as f64, na.y as f64, na.z as f64);
        if nzd == 0.0 {
            continue;
        }
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        let (Some((i0, i1)), Some((j0, j1))) = (
            range(lo.x - ex, hi.x - ex, origin.x, nx),
            range(lo.y - ey, hi.y - ey, origin.y, ny),
        ) else {
            continue;
        };
        let p2 = |v: Vec3| (v.x as f64, v.y as f64);
        let (pa, pb, pc) = (p2(a), p2(b), p2(c));
        let delta: i8 = if nzd < 0.0 { 1 } else { -1 };
        for j in j0..=j1 {
            let y = (origin.y + j as f32 * h + ey) as f64;
            for i in i0..=i1 {
                let x = (origin.x + i as f32 * h + ex) as f64;
                let p = (x, y);
                let w0 = orient_canon(pb, pc, p);
                let w1 = orient_canon(pc, pa, p);
                let w2 = orient_canon(pa, pb, p);
                let inside = (w0 > 0.0 && w1 > 0.0 && w2 > 0.0) || (w0 < 0.0 && w1 < 0.0 && w2 < 0.0);
                if inside {
                    let z = a.z as f64 - (nxd * (x - a.x as f64) + nyd * (y - a.y as f64)) / nzd;
                    crossings.push(((j * nx + i) as u32, z as f32, delta));
                }
            }
        }
    }
    crossings.sort_unstable_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut g = 0usize;
    while g < crossings.len() {
        let col = crossings[g].0;
        let mut e = g;
        while e < crossings.len() && crossings[e].0 == col {
            e += 1;
        }
        let (i, j) = (col as usize % nx, col as usize / nx);
        let mut w = 0i32;
        let mut ptr = g;
        for k in 0..nz {
            let zk = origin.z + k as f32 * h;
            while ptr < e && crossings[ptr].1 < zk {
                w += crossings[ptr].2 as i32;
                ptr += 1;
            }
            if w != 0 {
                let s = &mut sdf[idx(i, j, k)];
                *s = -(*s).max(1e-9 * h);
            }
        }
        g = e;
    }
    drop(crossings);

    // ------------------------------------------------------------ surface nets
    let ncx = nx - 1;
    let ncy = ny - 1;
    let ncz = nz - 1;
    let cell_id = |i: usize, j: usize, k: usize| (k * ncy + j) * ncx + i;
    let mut cell_vert: HashMap<usize, u32> = HashMap::new();
    let mut positions: Vec<Vec3> = Vec::new();
    const CORNERS: [(usize, usize, usize); 8] = [
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (1, 1, 0),
        (0, 0, 1),
        (1, 0, 1),
        (0, 1, 1),
        (1, 1, 1),
    ];
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for k in 0..ncz {
        for j in 0..ncy {
            for i in 0..ncx {
                let mut vals = [0.0f32; 8];
                let mut mask = 0u8;
                for (c, &(di, dj, dk)) in CORNERS.iter().enumerate() {
                    let v = sdf[idx(i + di, j + dj, k + dk)];
                    vals[c] = v;
                    if v < 0.0 {
                        mask |= 1 << c;
                    }
                }
                if mask == 0 || mask == 255 {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                let mut n = 0.0;
                for &(a, b) in &EDGES {
                    let (sa, sb) = (vals[a] < 0.0, vals[b] < 0.0);
                    if sa == sb {
                        continue;
                    }
                    let t = vals[a] / (vals[a] - vals[b]);
                    let pa = gp(i + CORNERS[a].0, j + CORNERS[a].1, k + CORNERS[a].2);
                    let pb = gp(i + CORNERS[b].0, j + CORNERS[b].1, k + CORNERS[b].2);
                    sum += pa + (pb - pa) * t.clamp(0.0, 1.0);
                    n += 1.0;
                }
                if n > 0.0 {
                    cell_vert.insert(cell_id(i, j, k), positions.len() as u32);
                    positions.push(sum / n);
                }
            }
        }
    }
    if positions.is_empty() {
        return Err(ModelError::InvalidParameter(
            "nenhuma célula cruza a superfície: voxel_size grande demais para a malha".into(),
        ));
    }

    let inside = |i: usize, j: usize, k: usize| sdf[idx(i, j, k)] < 0.0;
    let mut quads: Vec<[u32; 4]> = Vec::new();
    // `cells`: 4 células na ordem anti-horária para a normal +eixo
    let mut push_quad = |cells: [(usize, usize, usize); 4], flip: bool| {
        let mut v = [0u32; 4];
        for (n, &(i, j, k)) in cells.iter().enumerate() {
            match cell_vert.get(&cell_id(i, j, k)) {
                Some(&id) => v[n] = id,
                None => return,
            }
        }
        if flip {
            v.reverse();
        }
        quads.push(v);
    };
    // arestas X: u=y, v=z
    for k in 1..nz - 1 {
        for j in 1..ny - 1 {
            for i in 0..nx - 1 {
                let (e0, e1) = (inside(i, j, k), inside(i + 1, j, k));
                if e0 != e1 {
                    push_quad(
                        [(i, j - 1, k - 1), (i, j, k - 1), (i, j, k), (i, j - 1, k)],
                        !e0,
                    );
                }
            }
        }
    }
    // arestas Y: u=z, v=x
    for k in 1..nz - 1 {
        for j in 0..ny - 1 {
            for i in 1..nx - 1 {
                let (e0, e1) = (inside(i, j, k), inside(i, j + 1, k));
                if e0 != e1 {
                    push_quad(
                        [(i - 1, j, k - 1), (i - 1, j, k), (i, j, k), (i, j, k - 1)],
                        !e0,
                    );
                }
            }
        }
    }
    // arestas Z: u=x, v=y
    for k in 0..nz - 1 {
        for j in 1..ny - 1 {
            for i in 1..nx - 1 {
                let (e0, e1) = (inside(i, j, k), inside(i, j, k + 1));
                if e0 != e1 {
                    push_quad(
                        [(i - 1, j - 1, k), (i, j - 1, k), (i, j, k), (i - 1, j, k)],
                        !e0,
                    );
                }
            }
        }
    }
    drop(sdf);

    // ---------------------------------------------------------- suavização Taubin
    let mut out = EditMesh::new(format!("{}_voxel", mesh.name));
    out.verts = positions.iter().map(|&p| EVert::new(p)).collect();
    for q in &quads {
        // as quatro células são distintas por construção
        out.faces.push(Face::new(q));
    }
    if params.smooth_iterations > 0 {
        taubin_smooth(&mut out, params.smooth_iterations, params.smooth_factor);
    }

    // ------------------------------------------------------ transferência de atributos
    if params.transfer_attributes {
        transfer_attributes(mesh, &bvh, &mut out);
    }

    let t2 = Topology::build(&out);
    let report = VoxelReport {
        grid_dims: dims,
        grid_points: npoints,
        active_cells: positions.len(),
        quads: quads.len(),
        non_manifold_edges: t2.edges.iter().filter(|e| e.faces.len() > 2).count(),
        boundary_edges: t2.boundary_edge_count(),
    };
    Ok((out, report))
}

/// Predicado de orientação 2D em f64, antissimétrico por construção: a aresta
/// é sempre avaliada na ordem lexicográfica, e empates exatos são resolvidos
/// por uma regra consistente entre triângulos vizinhos (simulação de simplicidade).
fn orient_canon(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    let forward = (a.0, a.1) < (b.0, b.1);
    let (s, e) = if forward { (a, b) } else { (b, a) };
    let w = (e.0 - s.0) * (p.1 - s.1) - (e.1 - s.1) * (p.0 - s.0);
    let w = if forward { w } else { -w };
    if w == 0.0 {
        if forward {
            1e-300
        } else {
            -1e-300
        }
    } else {
        w
    }
}

/// Suavização Taubin (passo λ seguido de passo μ < 0): remove o ruído de
/// escada dos voxels sem encolher o volume.
fn taubin_smooth(mesh: &mut EditMesh, iterations: u32, lambda: f32) {
    let mu = 1.0 / (0.1 - 1.0 / lambda);
    let topo = Topology::build(mesh);
    let nbs: Vec<Vec<u32>> = (0..mesh.verts.len() as u32).map(|v| topo.vert_neighbors(v)).collect();
    let step = |mesh: &mut EditMesh, f: f32| {
        let old: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
        for (v, nb) in nbs.iter().enumerate() {
            if nb.is_empty() {
                continue;
            }
            let avg = nb.iter().map(|&u| old[u as usize]).sum::<Vec3>() / nb.len() as f32;
            mesh.verts[v].co = old[v] + (avg - old[v]) * f;
        }
    };
    for _ in 0..iterations {
        step(mesh, lambda);
        step(mesh, mu);
    }
}

/// Copia atributos da malha original para o resultado.
fn transfer_attributes(src: &EditMesh, bvh: &TriBvh, dst: &mut EditMesh) {
    for v in dst.verts.iter_mut() {
        let Some(hit) = bvh.closest_point(v.co) else { continue };
        let Some(tr) = bvh.tri_ref(hit.tri_index) else { continue };
        let sv = [
            &src.verts[tr.verts[0] as usize],
            &src.verts[tr.verts[1] as usize],
            &src.verts[tr.verts[2] as usize],
        ];
        let b = hit.bary;
        let mut color = [0.0f32; 4];
        for (c, out) in color.iter_mut().enumerate() {
            *out = sv[0].color[c] * b[0] + sv[1].color[c] * b[1] + sv[2].color[c] * b[2];
        }
        let nearest = (0..3)
            .max_by(|&x, &y| b[x].total_cmp(&b[y]))
            .map(|i| sv[i])
            .unwrap_or(sv[0]);
        v.color = color;
        v.mask = sv[0].mask * b[0] + sv[1].mask * b[1] + sv[2].mask * b[2];
        v.crease = sv[0].crease * b[0] + sv[1].crease * b[1] + sv[2].crease * b[2];
        v.joints = nearest.joints;
        v.weights = nearest.weights;
    }
    let centers: Vec<Vec3> = (0..dst.faces.len() as u32).map(|f| dst.face_center(f)).collect();
    for (f, c) in dst.faces.iter_mut().zip(centers) {
        if let Some(hit) = bvh.closest_point(c) {
            if let Some(sf) = src.faces.get(hit.face as usize) {
                f.material = sf.material;
                f.face_set = sf.face_set;
                f.smooth = sf.smooth;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::triangulate_polygon;
    use crate::primitives;

    fn volume(m: &EditMesh) -> f32 {
        let mut v = 0.0;
        for f in 0..m.face_count() as u32 {
            let pts = m.face_positions(f);
            for t in triangulate_polygon(&pts) {
                v += pts[t[0]].dot(pts[t[1]].cross(pts[t[2]])) / 6.0;
            }
        }
        v
    }

    fn assert_closed_manifold(m: &EditMesh, euler: i64) {
        m.validate().unwrap();
        let t = Topology::build(m);
        assert!(t.is_closed_manifold(), "deve ser fechada e manifold ({} não-manifold)", t.non_manifold_edge_count());
        assert_eq!(t.euler_characteristic(m), euler);
    }

    fn components(m: &EditMesh) -> u32 {
        Topology::build(m).face_components(m).1
    }

    #[test]
    fn sphere_remesh_preserves_volume_and_topology() {
        let src = primitives::quad_sphere(1.0, 8);
        let (out, rep) = voxel_remesh_report(&src, 0.1, &VoxelParams::default()).unwrap();
        assert_closed_manifold(&out, 2);
        assert!(out.faces.iter().all(|f| f.len() == 4));
        assert_eq!(rep.non_manifold_edges, 0);
        assert_eq!(rep.boundary_edges, 0);
        assert_eq!(rep.quads, out.face_count());
        assert_eq!(rep.active_cells, out.vert_count());
        let (v0, v1) = (volume(&src), volume(&out));
        assert!(((v1 - v0) / v0).abs() < 0.05, "volume {v1} vs {v0}");
        assert!(v1 > 0.0, "orientação para fora");
        for v in &out.verts {
            assert!((v.co.length() - 1.0).abs() < 0.08, "vértice fora da casca: {}", v.co.length());
        }
        // normais para fora
        for f in 0..out.face_count() as u32 {
            assert!(out.face_normal(f).dot(out.face_center(f)) > 0.0);
        }
        // grade dimensionada pelo bbox + padding
        assert!(rep.grid_dims.iter().all(|&d| d >= 20 && d <= 40), "{:?}", rep.grid_dims);
        assert_eq!(rep.grid_points, rep.grid_dims.iter().product::<usize>());
    }

    #[test]
    fn finer_voxels_give_more_faces_and_better_volume() {
        let src = primitives::quad_sphere(1.0, 8);
        let v0 = volume(&src);
        let coarse = voxel_remesh(&src, 0.2, &VoxelParams::default()).unwrap();
        let fine = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        let ratio = fine.face_count() as f32 / coarse.face_count() as f32;
        assert!(ratio > 3.0 && ratio < 5.5, "razão {ratio}");
        assert_closed_manifold(&coarse, 2);
        assert_closed_manifold(&fine, 2);
        assert!((volume(&fine) - v0).abs() <= (volume(&coarse) - v0).abs() + 0.05);
    }

    #[test]
    fn torus_remesh_has_genus_one() {
        let src = primitives::torus(1.0, 0.35, 32, 16);
        let out = voxel_remesh(&src, 0.08, &VoxelParams::default()).unwrap();
        assert_closed_manifold(&out, 0);
        assert_eq!(components(&out), 1);
        let (v0, v1) = (volume(&src), volume(&out));
        assert!(((v1 - v0) / v0).abs() < 0.08, "{v1} vs {v0}");
    }

    #[test]
    fn overlapping_spheres_are_fused_into_one_component() {
        let mut src = primitives::quad_sphere(1.0, 8);
        let mut b = primitives::quad_sphere(1.0, 8);
        b.transform_positions(|p| p + Vec3::new(1.0, 0.0, 0.0));
        src.append(&b);
        assert_eq!(components(&src), 2, "entrada tem duas cascas");
        let out = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        assert_eq!(components(&out), 1, "união numa casca só");
        assert_closed_manifold(&out, 2);
        // volume da união: duas esferas unitárias a distância 1 = 2·V − lente
        let v_sphere = 4.0 / 3.0 * std::f32::consts::PI;
        let lens = std::f32::consts::PI * (4.0 + 1.0) * (2.0 - 1.0f32).powi(2) / 12.0;
        let want = 2.0 * v_sphere - lens;
        let got = volume(&out);
        assert!(((got - want) / want).abs() < 0.06, "{got} vs {want}");
        // nenhum vértice dentro de ambas as esferas (a casca interna sumiu)
        for v in &out.verts {
            let d0 = v.co.length();
            let d1 = (v.co - Vec3::X).length();
            assert!(d0.min(d1) > 0.9, "casca interna residual em {:?}", v.co);
        }
    }

    #[test]
    fn disjoint_spheres_stay_two_components() {
        let mut src = primitives::quad_sphere(0.8, 6);
        let mut b = primitives::quad_sphere(0.8, 6);
        b.transform_positions(|p| p + Vec3::new(3.0, 0.0, 0.0));
        src.append(&b);
        let out = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        assert_eq!(components(&out), 2);
        assert_closed_manifold(&out, 4);
    }

    #[test]
    fn inverted_orientation_still_gives_outward_solid() {
        let mut src = primitives::quad_sphere(1.0, 6);
        for f in 0..src.face_count() as u32 {
            src.flip_face(f).unwrap();
        }
        assert!(volume(&src) < 0.0);
        let out = voxel_remesh(&src, 0.12, &VoxelParams::default()).unwrap();
        assert_closed_manifold(&out, 2);
        assert!(volume(&out) > 0.0);
    }

    #[test]
    fn cube_cylinder_capsule_remesh_are_valid_shells() {
        let cube = primitives::cube(1.0);
        let out = voxel_remesh(&cube, 0.1, &VoxelParams::default()).unwrap();
        assert_closed_manifold(&out, 2);
        let (lo, hi) = out.bounds().unwrap();
        for a in 0..3 {
            assert!((lo[a] + 0.5).abs() < 0.1 && (hi[a] - 0.5).abs() < 0.1, "eixo {a}: {lo:?} {hi:?}");
        }
        for src in [primitives::cylinder(0.5, 1.5, 16, 3, true), primitives::capsule(0.4, 1.0, 16, 4)] {
            let out = voxel_remesh(&src, 0.08, &VoxelParams::default()).unwrap();
            assert_closed_manifold(&out, 2);
            let (a, b) = (volume(&src), volume(&out));
            assert!(((b - a) / a).abs() < 0.08, "{}: {b} vs {a}", src.name);
        }
    }

    #[test]
    fn smoothing_keeps_volume_and_reduces_roughness() {
        let src = primitives::quad_sphere(1.0, 8);
        let rough = voxel_remesh(&src, 0.15, &VoxelParams::default()).unwrap();
        let smooth = voxel_remesh(
            &src,
            0.15,
            &VoxelParams {
                smooth_iterations: 6,
                ..Default::default()
            },
        )
        .unwrap();
        assert_closed_manifold(&smooth, 2);
        let v0 = volume(&src);
        assert!(((volume(&smooth) - v0) / v0).abs() < 0.06, "Taubin preserva volume");
        let radial_dev = |m: &EditMesh| {
            let mean = m.verts.iter().map(|v| v.co.length()).sum::<f32>() / m.vert_count() as f32;
            m.verts.iter().map(|v| (v.co.length() - mean).powi(2)).sum::<f32>() / m.vert_count() as f32
        };
        assert!(radial_dev(&smooth) < radial_dev(&rough), "{} !< {}", radial_dev(&smooth), radial_dev(&rough));
    }

    #[test]
    fn attributes_are_transferred_from_the_closest_source() {
        let mut src = primitives::quad_sphere(1.0, 8);
        for v in src.verts.iter_mut() {
            if v.co.y > 0.3 {
                v.mask = 1.0;
                v.color = [0.9, 0.1, 0.2, 1.0];
                v.joints = [7, 0, 0, 0];
            }
        }
        for f in 0..src.face_count() {
            if src.face_center(f as u32).x > 0.0 {
                src.faces[f].material = 5;
                src.faces[f].face_set = 2;
            }
        }
        let out = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        let top = out.verts.iter().filter(|v| v.co.y > 0.7);
        let mut n = 0;
        for v in top {
            assert!(v.mask > 0.9, "máscara do topo transferida: {}", v.mask);
            assert!((v.color[0] - 0.9).abs() < 0.05);
            assert_eq!(v.joints[0], 7);
            n += 1;
        }
        assert!(n > 5);
        let bottom_mask = out.verts.iter().filter(|v| v.co.y < -0.5).map(|v| v.mask).fold(0.0f32, f32::max);
        assert!(bottom_mask < 0.1);
        // material/face set pela face mais próxima
        for (fi, f) in out.faces.iter().enumerate() {
            let c = out.face_center(fi as u32);
            if c.x > 0.2 {
                assert_eq!(f.material, 5);
                assert_eq!(f.face_set, 2);
            }
            if c.x < -0.2 {
                assert_eq!(f.material, 0);
            }
        }
        // sem transferência: tudo padrão
        let plain = voxel_remesh(
            &src,
            0.1,
            &VoxelParams {
                transfer_attributes: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(plain.verts.iter().all(|v| v.mask == 0.0));
        assert!(plain.faces.iter().all(|f| f.material == 0 && f.face_set == 0));
    }

    #[test]
    fn invalid_arguments_and_inputs_are_rejected() {
        let src = primitives::quad_sphere(1.0, 4);
        let p = VoxelParams::default();
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(voxel_remesh(&src, bad, &p), Err(ModelError::InvalidParameter(_))), "{bad}");
        }
        for bad in [
            VoxelParams {
                padding: 2,
                ..Default::default()
            },
            VoxelParams {
                band_voxels: 1.0,
                ..Default::default()
            },
            VoxelParams {
                smooth_factor: 0.0,
                ..Default::default()
            },
            VoxelParams {
                smooth_factor: 1.5,
                ..Default::default()
            },
            VoxelParams {
                max_cells: 8,
                ..Default::default()
            },
        ] {
            assert!(voxel_remesh(&src, 0.1, &bad).is_err());
        }
        assert!(matches!(
            voxel_remesh(&EditMesh::new("vazio"), 0.1, &p),
            Err(ModelError::EmptySelection(_))
        ));
        let mut nan = src.clone();
        nan.verts[0].co = Vec3::NAN;
        assert!(voxel_remesh(&nan, 0.1, &p).is_err());
        // malha aberta
        let open = primitives::grid(4, 4, 2.0, 2.0);
        assert!(matches!(voxel_remesh(&open, 0.1, &p), Err(ModelError::UnsupportedTopology(_))));
        // permitindo malha aberta não entra em pânico
        let lax = VoxelParams {
            require_closed: false,
            ..Default::default()
        };
        let _ = voxel_remesh(&open, 0.2, &lax);
    }

    #[test]
    fn grid_size_limit_gives_clear_error() {
        let src = primitives::quad_sphere(1.0, 4);
        let tight = VoxelParams {
            max_cells: 1000,
            ..Default::default()
        };
        match voxel_remesh(&src, 0.05, &tight) {
            Err(ModelError::InvalidParameter(msg)) => {
                assert!(msg.contains("células") && msg.contains("limite"), "{msg}");
            }
            other => panic!("esperava erro de limite, veio {other:?}"),
        }
        // dentro do limite funciona
        assert!(voxel_remesh(&src, 0.2, &VoxelParams::default()).is_ok());
    }

    #[test]
    fn voxel_larger_than_the_object_fails_cleanly() {
        let tiny = primitives::quad_sphere(0.05, 3);
        let r = voxel_remesh(&tiny, 1.0, &VoxelParams::default());
        assert!(matches!(r, Err(ModelError::InvalidParameter(_))), "{r:?}");
    }

    #[test]
    fn remesh_is_deterministic_and_does_not_touch_the_input() {
        let src = primitives::torus(1.0, 0.3, 24, 12);
        let h = src.content_hash();
        let a = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        let b = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(src.content_hash(), h);
    }

    #[test]
    fn translated_and_rotated_inputs_give_the_same_topology_class() {
        let mut src = primitives::quad_sphere(0.9, 8);
        src.transform_positions(|p| glam::Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.7, 1.1) * p + Vec3::new(5.0, -3.0, 2.0));
        let out = voxel_remesh(&src, 0.1, &VoxelParams::default()).unwrap();
        assert_closed_manifold(&out, 2);
        let v0 = volume(&src);
        assert!(((volume(&out) - v0) / v0).abs() < 0.05);
    }

    #[test]
    fn property_sphere_radii_and_voxel_sizes() {
        let mut rng = crate::bvh::test_util::Rng(2);
        for _ in 0..6 {
            let r = rng.range(0.5, 1.5);
            let h = r * rng.range(0.07, 0.2);
            let src = primitives::quad_sphere(r, 8);
            let out = voxel_remesh(&src, h, &VoxelParams::default()).unwrap();
            assert_closed_manifold(&out, 2);
            let v0 = volume(&src);
            assert!(((volume(&out) - v0) / v0).abs() < 0.08, "r={r} h={h}");
        }
    }

    #[test]
    fn perf_voxel_remesh_of_a_large_mesh() {
        let src = primitives::quad_sphere(1.0, 64);
        let t0 = std::time::Instant::now();
        let (out, rep) = voxel_remesh_report(&src, 0.04, &VoxelParams::default()).unwrap();
        println!(
            "[voxel perf] {} faces -> {} quads em grade {:?} ({:?})",
            src.face_count(),
            out.face_count(),
            rep.grid_dims,
            t0.elapsed()
        );
        assert_closed_manifold(&out, 2);
        assert_eq!(rep.non_manifold_edges, 0);
    }

    #[test]
    fn orient_canon_is_antisymmetric_and_breaks_ties() {
        let (a, b) = ((0.0, 0.0), (1.0, 0.0));
        let p = (0.3, 0.2);
        assert_eq!(orient_canon(a, b, p), -orient_canon(b, a, p));
        let on_edge = (0.5, 0.0);
        let w1 = orient_canon(a, b, on_edge);
        let w2 = orient_canon(b, a, on_edge);
        assert!(w1 != 0.0 && w2 != 0.0 && w1.signum() == -w2.signum());
    }
}
