//! Rasterizador de CPU mínimo (flat shading, wireframe, esqueleto) para inspeção e para as figuras
//! de topologia da documentação. Independe de GPU. Não faz parte do contrato do personagem.

use crate::qmesh::QuadMesh;
use glam::{Vec2, Vec3};

pub struct ViewOpts {
    pub wire: bool,
    pub background: [u8; 4],
    /// Cor chapada (sem sombreamento): comprime bem e destaca o wireframe nas figuras da documentação.
    pub flat: bool,
}

impl Default for ViewOpts {
    fn default() -> Self {
        Self {
            wire: false,
            background: [232, 234, 240, 255],
            flat: false,
        }
    }
}

/// Renderiza `mesh` (quads) em projeção ortográfica olhando de `eye_dir` para `center`.
/// `colors` (opcional) dá a cor por vértice (0..1); `lines` desenha segmentos (esqueleto).
#[allow(clippy::too_many_arguments)]
pub fn render(
    mesh: &QuadMesh,
    colors: Option<&[Vec3]>,
    lines: Option<&[(Vec3, Vec3)]>,
    w: usize,
    h: usize,
    eye_dir: Vec3,
    center: Vec3,
    half_h: f32,
    o: &ViewOpts,
) -> image::RgbaImage {
    let fwd = (-eye_dir).normalize();
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let right = if right.length() < 1e-4 { Vec3::X } else { right };
    let up = right.cross(fwd).normalize();
    let scale = h as f32 / (2.0 * half_h);
    let proj = |p: Vec3| -> (f32, f32, f32) {
        let q = p - center;
        (w as f32 * 0.5 + q.dot(right) * scale, h as f32 * 0.5 - q.dot(up) * scale, q.dot(fwd))
    };
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba(o.background));
    let mut zb = vec![f32::MAX; w * h];
    let nrm = mesh.vertex_normals();
    let light = (-fwd * 0.8 + up * 0.5 + right * 0.3).normalize();
    for f in &mesh.faces {
        for tri in [[0usize, 1, 2], [0, 2, 3]] {
            let ids = [f[tri[0]], f[tri[1]], f[tri[2]]];
            if tri[2] == 3 && f[2] == f[3] {
                continue;
            }
            let ps: Vec<(f32, f32, f32)> = ids.iter().map(|&i| proj(mesh.pos[i as usize])).collect();
            let minx = ps.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let maxx = ps.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(w as f32 - 1.0) as usize;
            let miny = ps.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let maxy = ps.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(h as f32 - 1.0) as usize;
            let (a, b, c) = (Vec2::new(ps[0].0, ps[0].1), Vec2::new(ps[1].0, ps[1].1), Vec2::new(ps[2].0, ps[2].1));
            let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if det.abs() < 1e-9 {
                continue;
            }
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let l1 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / det;
                    let l2 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / det;
                    let l3 = 1.0 - l1 - l2;
                    if l1 < -1e-4 || l2 < -1e-4 || l3 < -1e-4 {
                        continue;
                    }
                    let z = l1 * ps[0].2 + l2 * ps[1].2 + l3 * ps[2].2;
                    if z < zb[y * w + x] {
                        zb[y * w + x] = z;
                        let n = (nrm[ids[0] as usize] * l1 + nrm[ids[1] as usize] * l2 + nrm[ids[2] as usize] * l3).normalize_or_zero();
                        let lam = if o.flat { 0.92 } else { n.dot(light).max(0.0) * 0.75 + 0.25 };
                        let facing = n.dot(-fwd);
                        let base = if o.flat && colors.is_none() {
                            [236.0, 214.0, 200.0]
                        } else if let Some(c) = colors {
                            let col = c[ids[0] as usize] * l1 + c[ids[1] as usize] * l2 + c[ids[2] as usize] * l3;
                            [col.x * 255.0, col.y * 255.0, col.z * 255.0]
                        } else if facing < 0.0 {
                            [200.0, 60.0, 60.0]
                        } else {
                            [236.0, 196.0, 176.0]
                        };
                        img.put_pixel(x as u32, y as u32, image::Rgba([(base[0] * lam) as u8, (base[1] * lam) as u8, (base[2] * lam) as u8, 255]));
                    }
                }
            }
        }
    }
    if o.wire {
        for f in &mesh.faces {
            for k in 0..4 {
                let a = proj(mesh.pos[f[k] as usize]);
                let b = proj(mesh.pos[f[(k + 1) % 4] as usize]);
                let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let (x, y, z) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t);
                    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                        continue;
                    }
                    let idx = y as usize * w + x as usize;
                    if z <= zb[idx] + 0.004 {
                        img.put_pixel(x as u32, y as u32, image::Rgba([30, 40, 90, 255]));
                    }
                }
            }
        }
    }
    if let Some(lines) = lines {
        for (a, b) in lines {
            let (pa, pb) = (proj(*a), proj(*b));
            let steps = ((pb.0 - pa.0).abs().max((pb.1 - pa.1).abs()) as usize).max(1);
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let (x, y) = (pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t);
                for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0.0 && yy >= 0.0 && xx < w as f32 && yy < h as f32 {
                        img.put_pixel(xx as u32, yy as u32, image::Rgba([20, 160, 40, 255]));
                    }
                }
            }
            for (dx, dy) in [(-2.0, -2.0), (-2.0, 2.0), (2.0, -2.0), (2.0, 2.0), (0.0, 0.0)] {
                let (xx, yy) = (pa.0 + dx, pa.1 + dy);
                if xx >= 0.0 && yy >= 0.0 && xx < w as f32 && yy < h as f32 {
                    img.put_pixel(xx as u32, yy as u32, image::Rgba([230, 20, 20, 255]));
                }
            }
        }
    }
    img
}
