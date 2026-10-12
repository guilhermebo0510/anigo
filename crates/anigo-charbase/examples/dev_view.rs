//! Ferramenta de desenvolvimento: rasteriza a gaiola/Skin em CPU (flat shading + wireframe) para
//! inspeção rápida. Uso: `cargo run -p anigo-charbase --example dev_view -- <saida.png> [f|m] [front|side|back|head|hand|foot|top] [wire] [cage]`

use anigo_charbase::cage::build_cage;
use anigo_charbase::params::Params;
use anigo_charbase::qmesh::QuadMesh;
use anigo_charbase::subdiv::catmull_clark;
use glam::{Vec2, Vec3};

fn render(mesh: &QuadMesh, w: usize, h: usize, eye_dir: Vec3, center: Vec3, half_h: f32, wire: bool) -> image::RgbaImage {
    let fwd = (-eye_dir).normalize();
    let up0 = Vec3::Y;
    let right = fwd.cross(up0).normalize_or_zero();
    let right = if right.length() < 1e-4 { Vec3::X } else { right };
    let up = right.cross(fwd).normalize();
    let scale = h as f32 / (2.0 * half_h);
    let proj = |p: Vec3| -> (f32, f32, f32) {
        let q = p - center;
        (w as f32 * 0.5 + q.dot(right) * scale, h as f32 * 0.5 - q.dot(up) * scale, q.dot(fwd))
    };
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([232, 234, 240, 255]));
    let mut zb = vec![f32::MAX; w * h];
    let nrm = mesh.vertex_normals();
    let light = (-fwd * 0.8 + up * 0.5 + right * 0.3).normalize();
    for f in &mesh.faces {
        for tri in [[0usize, 1, 2], [0, 2, 3]] {
            let ids = [f[tri[0]], f[tri[1]], f[tri[2]]];
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
                        let lam = n.dot(light).max(0.0) * 0.75 + 0.25;
                        let facing = n.dot(-fwd);
                        let base = if facing < 0.0 { [200.0, 60.0, 60.0] } else { [236.0, 196.0, 176.0] };
                        img.put_pixel(x as u32, y as u32, image::Rgba([(base[0] * lam) as u8, (base[1] * lam) as u8, (base[2] * lam) as u8, 255]));
                    }
                }
            }
        }
    }
    if wire {
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
    img
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "/tmp/dev_view.png".into());
    let male = args.get(1).map(|s| s == "m").unwrap_or(false);
    let view = args.get(2).cloned().unwrap_or_else(|| "front".into());
    let wire = args.iter().any(|a| a == "wire");
    let cage_only = args.iter().any(|a| a == "cage");
    let mut p = Params::default();
    p.gender = if male { 0.0 } else { 1.0 };
    for a in &args {
        if let Some(v) = a.strip_prefix("age=") {
            p.age = v.parse().unwrap_or(0.0);
        }
        if let Some(v) = a.strip_prefix("weight=") {
            p.weight = v.parse().unwrap_or(0.0);
        }
        if let Some(v) = a.strip_prefix("muscle=") {
            p.muscle = v.parse().unwrap_or(0.0);
        }
        if let Some(v) = a.strip_prefix("height=") {
            p.height = v.parse().unwrap_or(0.0);
        }
    }
    let cage = build_cage(&p);
    let refined = catmull_clark(&cage.mesh);
    let mesh = if cage_only { cage.mesh.clone() } else { refined.mesh.clone() };
    eprintln!(
        "cage: V={} F={} euler={} boundary_loops={} | refined: V={} F={} euler={}",
        cage.mesh.pos.len(),
        cage.mesh.faces.len(),
        cage.mesh.euler(),
        cage.mesh.boundary_loops().len(),
        refined.mesh.pos.len(),
        refined.mesh.faces.len(),
        refined.mesh.euler()
    );
    let h = cage.dims.h;
    let hc = cage.dims.y_chin + cage.dims.head_h * 0.5;
    for view in view.split(',') {
    let (eye_dir, center, half_h, w, hh) = match view {
        "side" => (Vec3::X, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 700, 1100),
        "back" => (-Vec3::Z, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 1100, 1100),
        "head" => (Vec3::Z, Vec3::new(0.0, hc, 0.0), cage.dims.head_h * 0.75, 900, 900),
        "headside" => (Vec3::X, Vec3::new(0.0, hc, 0.0), cage.dims.head_h * 0.75, 900, 900),
        "head34" => (Vec3::new(0.6, 0.15, 0.8).normalize(), Vec3::new(0.0, hc, 0.0), cage.dims.head_h * 0.75, 900, 900),
        "hand" => (Vec3::Y, Vec3::new(cage.dims.h * 0.5, 1.2, 0.0), 0.16, 900, 900),
        "handside" => (Vec3::Z, Vec3::new(cage.dims.h * 0.5, 1.2, 0.0), 0.16, 900, 900),
        "foot" => (Vec3::X, Vec3::new(0.0, 0.06, 0.05), 0.16, 900, 900),
        "foot34" => (Vec3::new(0.6, 0.6, 0.7).normalize(), Vec3::new(0.0, 0.05, 0.05), 0.17, 900, 900),
        "top" => (Vec3::Y, Vec3::new(0.0, 0.0, 0.0), h * 0.55, 1100, 1100),
        "pelvis" => (Vec3::new(0.0, -0.7, 0.7).normalize(), Vec3::new(0.0, 0.8, 0.0), 0.3, 900, 900),
        "shoulder" => (Vec3::new(0.5, 0.4, 0.7).normalize(), Vec3::new(0.15, 1.25, 0.0), 0.2, 900, 900),
        _ => (Vec3::Z, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 1100, 1100),
    };
    let img = render(&mesh, w, hh, eye_dir, center, half_h, wire);
    let path = if out.ends_with(".png") { out.clone() } else { format!("{out}_{view}.png") };
    img.save(&path).unwrap();
    println!("{path}");
    }
}
