//! AOVs por **rasterizador de software** (CPU): profundidade, máscara, ID por objeto e normais.
//!
//! Por que CPU: o renderer canônico só entrega a cor final; um passe de dados na GPU exigiria shaders novos no
//! `render_contract_v1.json` (hashes congelados, paridade com o viewport). Aqui as MESMAS malhas, matrizes mundiais e
//! `view_proj` da cena do renderer produzem os dados com a convenção de projeção do `anigo-core` (RH, profundidade 0..1,
//! frente = anti-horário, descarte de costas), e o resultado é **idêntico bit a bit em qualquer GPU**.
//!
//! Como no Blender, profundidade/ID/normais **não têm antialiasing** (uma amostra no centro do pixel); a máscara usa a
//! cobertura de uma rasterização supersampleada (bordas suaves). Nenhum AOV leva DoF.

use anigo_core::{fnv1a64, Scene};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

pub const NO_NODE: u32 = u32::MAX;

/// Resultado da rasterização: um registro por pixel (linha de cima para baixo).
#[derive(Debug, Clone)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    /// Profundidade de tela (z NDC, 0..1) usada no teste de profundidade; `f32::INFINITY` = nada.
    pub z: Vec<f32>,
    /// Distância ao longo do eixo da câmera (metros), perspectiva-correta.
    pub dist: Vec<f32>,
    /// Índice do nó (posição em `Scene::nodes`) ou [`NO_NODE`].
    pub node: Vec<u32>,
    /// Normal no espaço da câmera (x direita, y cima, z para a câmera), unitária.
    pub normal: Vec<[f32; 3]>,
}

impl Raster {
    pub fn covered(&self, i: usize) -> bool {
        self.node[i] != NO_NODE
    }
}

#[derive(Clone, Copy)]
struct Vtx {
    clip: Vec4,
    /// Distância ao longo da câmera.
    dist: f32,
    n: Vec3,
}

impl Vtx {
    fn lerp(a: &Vtx, b: &Vtx, t: f32) -> Vtx {
        Vtx { clip: a.clip.lerp(b.clip, t), dist: a.dist + (b.dist - a.dist) * t, n: a.n.lerp(b.n, t) }
    }
}

/// Triângulo pronto para o laço de pixels.
struct ScreenTri {
    node: u32,
    p: [Vec2; 3],
    z: [f32; 3],
    inv_w: [f32; 3],
    /// atributos já divididos por w (interpolação perspectiva-correta)
    dist_w: [f32; 3],
    n_w: [Vec3; 3],
    min_y: i32,
    max_y: i32,
    min_x: i32,
    max_x: i32,
}

fn skin_matrix(palette: &[f32], j: usize) -> Mat4 {
    palette.get(j * 16..j * 16 + 16).map_or(Mat4::IDENTITY, |m| Mat4::from_cols_array(m.try_into().unwrap_or(&[0.0; 16])))
}

/// Triângulos de tela da cena para um alvo `w × h`.
fn build_tris(scene: &Scene, w: u32, h: u32) -> Vec<ScreenTri> {
    let aspect = w as f32 / h.max(1) as f32;
    let mut cam = scene.camera.clone();
    cam.aspect = aspect;
    let view = cam.build_view_matrix();
    let view_proj = cam.build_view_projection_matrix();
    let world = scene.resolve_world_transforms().unwrap_or_default();
    let skin = !scene.skin.palette_is_identity;
    let ortho = cam.orthographic.is_some();
    let mut tris = Vec::new();
    for (ni, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.as_ref().filter(|_| node.visible) else { continue };
        let m = world.get(&node.id).copied().unwrap_or_else(|| node.transform.to_matrix());
        let nm = Mat3::from_mat4(view * m).inverse().transpose();
        let vp_m = view_proj * m;
        let v_m = view * m;
        let verts: Vec<Vtx> = mesh
            .vertices
            .iter()
            .map(|v| {
                let mut p = Vec3::from_array(v.position);
                let mut n = Vec3::from_array(v.normal);
                if skin {
                    let mut ps = Vec3::ZERO;
                    let mut ns = Vec3::ZERO;
                    for k in 0..4 {
                        let wgt = v.weights[k];
                        if wgt > 0.0 {
                            let sm = skin_matrix(&scene.skin.palette, v.joints[k] as usize);
                            ps += sm.transform_point3(p) * wgt;
                            ns += sm.transform_vector3(n) * wgt;
                        }
                    }
                    p = ps;
                    n = ns;
                }
                let clip = vp_m * p.extend(1.0);
                let vp = v_m * p.extend(1.0);
                Vtx { clip, dist: -vp.z, n: (nm * n).normalize_or_zero() }
            })
            .collect();
        for t in mesh.indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (verts.get(t[0] as usize), verts.get(t[1] as usize), verts.get(t[2] as usize)) else { continue };
            push_clipped(&mut tris, ni as u32, [*a, *b, *c], w, h, ortho);
        }
    }
    tris
}

/// Recorta contra o plano de perto (z_clip >= 0), faz o leque e projeta para a tela.
fn push_clipped(out: &mut Vec<ScreenTri>, node: u32, tri: [Vtx; 3], w: u32, h: u32, _ortho: bool) {
    if tri.iter().any(|v| !v.clip.is_finite()) {
        return;
    }
    // trivialmente fora de um dos planos laterais/ topo/ base / longe
    let outside = |f: &dyn Fn(&Vtx) -> bool| tri.iter().all(f);
    if outside(&|v| v.clip.x > v.clip.w) || outside(&|v| v.clip.x < -v.clip.w) || outside(&|v| v.clip.y > v.clip.w) || outside(&|v| v.clip.y < -v.clip.w) || outside(&|v| v.clip.z > v.clip.w) || outside(&|v| v.clip.z < 0.0) {
        return;
    }
    let mut poly: Vec<Vtx> = tri.to_vec();
    if tri.iter().any(|v| v.clip.z < 0.0) {
        let mut next = Vec::with_capacity(4);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ina, inb) = (a.clip.z >= 0.0, b.clip.z >= 0.0);
            if ina {
                next.push(a);
            }
            if ina != inb {
                let t = a.clip.z / (a.clip.z - b.clip.z);
                next.push(Vtx::lerp(&a, &b, t));
            }
        }
        poly = next;
    }
    if poly.len() < 3 {
        return;
    }
    // NDC: o sinal da área decide a frente (anti-horário com y para cima)
    let ndc: Vec<(Vec2, f32, f32)> = poly
        .iter()
        .map(|v| {
            let iw = 1.0 / v.clip.w;
            (Vec2::new(v.clip.x * iw, v.clip.y * iw), v.clip.z * iw, iw)
        })
        .collect();
    for k in 1..poly.len() - 1 {
        let (i0, i1, i2) = (0, k, k + 1);
        let (a, b, c) = (ndc[i0].0, ndc[i1].0, ndc[i2].0);
        let area = (b - a).perp_dot(c - a);
        if area <= 0.0 {
            continue; // costas ou degenerado (pipeline do cel: cull de costas)
        }
        let to_px = |p: Vec2| Vec2::new((p.x * 0.5 + 0.5) * w as f32, (1.0 - (p.y * 0.5 + 0.5)) * h as f32);
        let p = [to_px(a), to_px(b), to_px(c)];
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        let (min_x, max_x) = ((min.x.floor() as i32).max(0), (max.x.ceil() as i32).min(w as i32 - 1));
        let (min_y, max_y) = ((min.y.floor() as i32).max(0), (max.y.ceil() as i32).min(h as i32 - 1));
        if min_x > max_x || min_y > max_y {
            continue;
        }
        let idx = [i0, i1, i2];
        out.push(ScreenTri {
            node,
            p,
            z: [ndc[i0].1, ndc[i1].1, ndc[i2].1],
            inv_w: [ndc[i0].2, ndc[i1].2, ndc[i2].2],
            dist_w: [0, 1, 2].map(|j| poly[idx[j]].dist * ndc[idx[j]].2),
            n_w: [0, 1, 2].map(|j| poly[idx[j]].n * ndc[idx[j]].2),
            min_y,
            max_y,
            min_x,
            max_x,
        });
    }
}

/// Rasteriza a cena em `w × h` (uma amostra no centro de cada pixel).
pub fn rasterize(scene: &Scene, w: u32, h: u32) -> Raster {
    let (wu, hu) = (w as usize, h as usize);
    let tris = build_tris(scene, w, h);
    let mut r = Raster { w, h, z: vec![f32::INFINITY; wu * hu], dist: vec![0.0; wu * hu], node: vec![NO_NODE; wu * hu], normal: vec![[0.0, 0.0, 1.0]; wu * hu] };
    if wu == 0 || hu == 0 {
        return r;
    }
    // faixas de linhas independentes; cada faixa percorre os triângulos na MESMA ordem => resultado determinístico
    let bands = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 8).min(hu);
    let per = hu.div_ceil(bands);
    let Raster { z, dist, node, normal, .. } = &mut r;
    std::thread::scope(|s| {
        let zs = z.chunks_mut(per * wu);
        let ds = dist.chunks_mut(per * wu);
        let ns = node.chunks_mut(per * wu);
        let nrm = normal.chunks_mut(per * wu);
        for (bi, (((zc, dc), nc), nrc)) in zs.zip(ds).zip(ns).zip(nrm).enumerate() {
            let tris = &tris;
            s.spawn(move || {
                let y0 = bi * per;
                let y1 = (y0 + zc.len() / wu).min(hu);
                for t in tris.iter() {
                    if (t.max_y as usize) < y0 || (t.min_y as usize) >= y1 {
                        continue;
                    }
                    shade_tri(t, y0, y1, wu, zc, dc, nc, nrc);
                }
            });
        }
    });
    r
}

#[allow(clippy::too_many_arguments)]
fn shade_tri(t: &ScreenTri, y0: usize, y1: usize, w: usize, z: &mut [f32], dist: &mut [f32], node: &mut [u32], normal: &mut [[f32; 3]]) {
    let (a, b, c) = (t.p[0], t.p[1], t.p[2]);
    let area = (b - a).perp_dot(c - a); // sinal depende do eixo y da tela (invertido); usamos o módulo
    if area == 0.0 {
        return;
    }
    let inv_area = 1.0 / area;
    let ys = (t.min_y as usize).max(y0)..=(t.max_y as usize).min(y1 - 1);
    for y in ys {
        let py = y as f32 + 0.5;
        for x in t.min_x..=t.max_x {
            let p = Vec2::new(x as f32 + 0.5, py);
            // coordenadas baricêntricas por funções de aresta (a área tem o sinal da tela; dividir normaliza)
            let w0 = (b - p).perp_dot(c - p) * inv_area;
            let w1 = (c - p).perp_dot(a - p) * inv_area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let zz = w0 * t.z[0] + w1 * t.z[1] + w2 * t.z[2];
            if !(0.0..=1.0).contains(&zz) {
                continue;
            }
            let i = (y - y0) * w + x as usize;
            if zz >= z[i] {
                continue;
            }
            let iw = w0 * t.inv_w[0] + w1 * t.inv_w[1] + w2 * t.inv_w[2];
            if iw <= 0.0 {
                continue;
            }
            z[i] = zz;
            node[i] = t.node;
            dist[i] = (w0 * t.dist_w[0] + w1 * t.dist_w[1] + w2 * t.dist_w[2]) / iw;
            let n = (t.n_w[0] * w0 + t.n_w[1] * w1 + t.n_w[2] * w2) / iw;
            normal[i] = n.normalize_or_zero().to_array();
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Saídas
// ------------------------------------------------------------------------------------------------

/// Cor única e determinística de cada nó (por hash do id; sem preto; sem colisões entre nós da cena).
pub fn object_id_colors(scene: &Scene) -> Vec<[u8; 3]> {
    let mut used = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(scene.nodes.len());
    for node in &scene.nodes {
        let mut salt = 0u32;
        loop {
            let h = fnv1a64(&format!("{}#{salt}", node.id));
            // espalha os bits e garante canais não nulos o bastante para separar visualmente
            let c = [((h >> 8) & 0xff) as u8 | 0x20, ((h >> 24) & 0xff) as u8 | 0x20, ((h >> 40) & 0xff) as u8 | 0x20];
            if used.insert(c) {
                out.push(c);
                break;
            }
            salt += 1;
        }
    }
    out
}

/// RGBA8 do ID por objeto: cor do nó onde há geometria, `0,0,0,0` no resto.
pub fn id_image(r: &Raster, colors: &[[u8; 3]]) -> Vec<u8> {
    let mut out = vec![0u8; r.w as usize * r.h as usize * 4];
    for (i, &n) in r.node.iter().enumerate() {
        if n != NO_NODE {
            let c = colors.get(n as usize).copied().unwrap_or([255, 0, 255]);
            out[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    out
}

/// RGB8 das normais (espaço da câmera, `n × 0,5 + 0,5`); fundo `128,128,255` (normal para a câmera) com alfa 0 não existe: sem
/// geometria o pixel fica `0,0,0`.
pub fn normal_image(r: &Raster) -> Vec<u8> {
    let mut out = vec![0u8; r.w as usize * r.h as usize * 3];
    for i in 0..r.node.len() {
        if r.covered(i) {
            let n = r.normal[i];
            for c in 0..3 {
                out[i * 3 + c] = ((n[c] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// Profundidade normalizada: gray16 BE (branco = perto) e o intervalo `(perto, longe)` em metros usado para normalizar.
pub fn depth_image(r: &Raster) -> (Vec<u8>, Option<(f32, f32)>) {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in 0..r.node.len() {
        if r.covered(i) {
            lo = lo.min(r.dist[i]);
            hi = hi.max(r.dist[i]);
        }
    }
    let mut out = vec![0u8; r.w as usize * r.h as usize * 2];
    if !lo.is_finite() {
        return (out, None);
    }
    let span = (hi - lo).max(1e-6);
    for i in 0..r.node.len() {
        if r.covered(i) {
            let v = 1.0 - (r.dist[i] - lo) / span;
            let q = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
            out[i * 2..i * 2 + 2].copy_from_slice(&q.to_be_bytes());
        }
    }
    (out, Some((lo, hi)))
}

/// Máscara de cobertura (gray8) de uma rasterização `k×` maior reduzida por média `k×k`.
pub fn mask_image(hi: &Raster, k: u32) -> (u32, u32, Vec<u8>) {
    let k = k.max(1) as usize;
    let (w, h) = (hi.w as usize / k, hi.h as usize / k);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut n = 0u32;
            for dy in 0..k {
                for dx in 0..k {
                    if hi.node[(y * k + dy) * hi.w as usize + x * k + dx] != NO_NODE {
                        n += 1;
                    }
                }
            }
            out[y * w + x] = ((n as f32 / (k * k) as f32) * 255.0 + 0.5) as u8;
        }
    }
    (w as u32, h as u32, out)
}

/// Contorno isolado: onde `with_outline` difere de `without_outline` (RGBA8 do renderer, mesma câmera) mantém a cor do
/// quadro com contorno; o resto fica transparente. Devolve RGBA8 pré-multiplicado (cor = A onde difere, alfa = 255).
pub fn outline_from_diff(with_outline: &[u8], without_outline: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; with_outline.len()];
    for (i, (a, b)) in with_outline.chunks_exact(4).zip(without_outline.chunks_exact(4)).enumerate() {
        if a != b {
            out[i * 4..i * 4 + 4].copy_from_slice(&[a[0], a[1], a[2], 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::scene::SceneNode;
    use anigo_core::{Mesh, Transform, Vertex};

    fn quad_mesh(z: f32, half: f32) -> Mesh {
        let n = [0.0, 0.0, 1.0];
        let v = |x: f32, y: f32| Vertex::new([x, y, z], n, [0.0, 0.0]);
        Mesh {
            name: "quad".into(),
            vertices: vec![v(-half, -half), v(half, -half), v(half, half), v(-half, half)],
            indices: vec![0, 1, 2, 0, 2, 3],
            custom_attributes: Default::default(),
        }
    }

    fn scene_with(nodes: Vec<(&str, Mesh, Transform)>) -> Scene {
        let mut s = Scene::new_empty();
        for (id, mesh, t) in nodes {
            let mut n = SceneNode::new(id, id);
            n.mesh = Some(mesh);
            n.transform = t;
            s.add_node(n);
        }
        s.camera.eye = Vec3::new(0.0, 0.0, 5.0);
        s.camera.target = Vec3::ZERO;
        s.camera.fov_y = 40f32.to_radians();
        s
    }

    #[test]
    fn a_front_facing_quad_covers_the_expected_pixels_with_exact_depth() {
        let s = scene_with(vec![("a", quad_mesh(0.0, 1.0), Transform::default())]);
        let r = rasterize(&s, 64, 64);
        let covered = (0..64 * 64).filter(|&i| r.covered(i)).count();
        // meia altura visível a 5 m com fov 40°: 2·5·tan(20°) = 3,64 m => o quadrado de 2 m ocupa (2/3,64)² ≈ 30 %
        let expect = (2.0f32 / (2.0 * 5.0 * 20f32.to_radians().tan())).powi(2) * 64.0 * 64.0;
        assert!((covered as f32 - expect).abs() < expect * 0.06, "{covered} vs {expect}");
        let center = 32 * 64 + 32;
        assert!((r.dist[center] - 5.0).abs() < 1e-3, "{}", r.dist[center]);
        assert!(r.normal[center][2] > 0.999);
        assert!(!r.covered(0));
    }

    #[test]
    fn back_faces_are_culled_like_the_cel_pipeline() {
        let mut q = quad_mesh(0.0, 1.0);
        q.indices = vec![0, 2, 1, 0, 3, 2]; // enrolamento invertido
        let s = scene_with(vec![("a", q, Transform::default())]);
        let r = rasterize(&s, 32, 32);
        assert_eq!((0..32 * 32).filter(|&i| r.covered(i)).count(), 0);
    }

    #[test]
    fn nearer_geometry_wins_and_ids_are_unique_per_node() {
        let s = scene_with(vec![("far", quad_mesh(-1.0, 1.5), Transform::default()), ("near", quad_mesh(1.0, 0.5), Transform::default())]);
        let r = rasterize(&s, 64, 64);
        let center = 32 * 64 + 32;
        assert_eq!(r.node[center], 1, "o quadrado pequeno e próximo cobre o centro");
        assert!((r.dist[center] - 4.0).abs() < 1e-3);
        let off = 32 * 64 + 12;
        assert_eq!(r.node[off], 0, "fora do pequeno aparece o grande");
        let colors = object_id_colors(&s);
        assert_ne!(colors[0], colors[1]);
        let img = id_image(&r, &colors);
        assert_eq!(&img[center * 4..center * 4 + 3], &colors[1]);
        assert_eq!(img[3], 0, "fora da geometria o ID é transparente");
        // determinismo das cores
        assert_eq!(colors, object_id_colors(&s));
    }

    #[test]
    fn near_plane_clipping_keeps_a_floor_that_crosses_the_camera() {
        // piso horizontal enorme sob a câmera inclinada: cruza o plano de perto
        let n = [0.0, 1.0, 0.0];
        let v = |x: f32, z: f32| Vertex::new([x, 0.0, z], n, [0.0, 0.0]);
        let floor = Mesh { name: "f".into(), vertices: vec![v(-50.0, -50.0), v(-50.0, 50.0), v(50.0, 50.0), v(50.0, -50.0)], indices: vec![0, 1, 2, 0, 2, 3], custom_attributes: Default::default() };
        let mut s = scene_with(vec![("floor", floor, Transform::default())]);
        s.camera.eye = Vec3::new(0.0, 1.5, 0.0);
        s.camera.target = Vec3::new(0.0, 1.0, -3.0);
        let r = rasterize(&s, 48, 48);
        let covered = (0..48 * 48).filter(|&i| r.covered(i)).count();
        assert!(covered > 48 * 48 / 4, "o piso some quando cruza o plano de perto: {covered}");
        // a metade de baixo está coberta
        assert!(r.covered(47 * 48 + 24));
    }

    #[test]
    fn depth_mask_and_normal_images_have_the_documented_encoding() {
        let s = scene_with(vec![("far", quad_mesh(-1.0, 1.5), Transform::default()), ("near", quad_mesh(1.0, 0.5), Transform::default())]);
        let r = rasterize(&s, 64, 64);
        let (d, range) = depth_image(&r);
        let (lo, hi) = range.unwrap();
        assert!((lo - 4.0).abs() < 1e-3 && (hi - 6.0).abs() < 0.2, "{lo} {hi}");
        let center = 32 * 64 + 32;
        assert_eq!(&d[center * 2..center * 2 + 2], &[0xFF, 0xFF], "o mais perto é branco");
        assert_eq!(&d[0..2], &[0, 0], "o fundo é preto");
        let n = normal_image(&r);
        assert_eq!(&n[center * 3..center * 3 + 3], &[128, 128, 255]);
        // quadrado girado: bordas diagonais => pixels de cobertura parcial
        let s2 = scene_with(vec![("near", quad_mesh(1.0, 0.8), Transform { rotation: glam::Quat::from_rotation_z(0.5), ..Default::default() })]);
        let hi_r = rasterize(&s2, 128, 128);
        let (mw, mh, m) = mask_image(&hi_r, 2);
        assert_eq!((mw, mh), (64, 64));
        assert_eq!(m[center], 255);
        assert_eq!(m[0], 0);
        let _ = &r;
        assert!(m.iter().any(|&v| v > 0 && v < 255), "a máscara tem bordas suaves (cobertura parcial)");
    }

    #[test]
    fn rasterization_is_deterministic() {
        let s = scene_with(vec![("far", quad_mesh(-1.0, 1.5), Transform::default()), ("near", quad_mesh(1.0, 0.7), Transform { rotation: glam::Quat::from_rotation_y(0.4), ..Default::default() })]);
        let a = rasterize(&s, 97, 61);
        let b = rasterize(&s, 97, 61);
        assert_eq!(a.node, b.node);
        assert_eq!(a.dist, b.dist);
        assert_eq!(a.z, b.z);
    }

    #[test]
    fn outline_isolation_keeps_only_the_differing_pixels() {
        let with = [10u8, 10, 10, 255, 99, 0, 0, 255, 0, 0, 0, 0, 5, 5, 5, 255];
        let without = [10u8, 10, 10, 255, 20, 20, 20, 255, 0, 0, 0, 0, 5, 5, 5, 255];
        let out = outline_from_diff(&with, &without);
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        assert_eq!(&out[4..8], &[99, 0, 0, 255]);
        assert_eq!(&out[8..16], &[0; 8]);
    }
}
