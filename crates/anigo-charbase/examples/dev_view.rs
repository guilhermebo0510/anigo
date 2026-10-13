//! Ferramenta de desenvolvimento: rasteriza o Skin em CPU (flat shading + wireframe) para
//! inspeção rápida. Uso:
//! `cargo run -p anigo-charbase --example dev_view -- <prefixo|arquivo.png> [f|m] <vistas,separadas,por,virgula> [wire] [skel] [wt=<osso>] [zone=<n>] [age=..] [weight=..] [muscle=..] [height=..] [gender=..]`

use anigo_charbase::generate_with;
use anigo_charbase::params::Params;
use anigo_charbase::qmesh::QuadMesh;
use anigo_charbase::topology::reference;
use glam::Vec3;

struct Opts {
    wire: bool,
    skel: bool,
}

#[allow(clippy::too_many_arguments)]
fn render(
    mesh: &QuadMesh,
    colors: Option<&[Vec3]>,
    skel: Option<&[(Vec3, Vec3)]>,
    w: usize,
    h: usize,
    eye_dir: Vec3,
    center: Vec3,
    half_h: f32,
    o: &Opts,
) -> image::RgbaImage {
    anigo_charbase::debugview::render(
        mesh,
        colors,
        skel,
        w,
        h,
        eye_dir,
        center,
        half_h,
        &anigo_charbase::debugview::ViewOpts { wire: o.wire, ..Default::default() },
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "/tmp/dev_view.png".into());
    let male = args.get(1).map(|s| s == "m").unwrap_or(false);
    let view = args.get(2).cloned().unwrap_or_else(|| "front".into());
    let o = Opts {
        wire: args.iter().any(|a| a == "wire"),
        skel: args.iter().any(|a| a == "skel"),
    };
    let mut p = Params::default();
    p.gender = if male { 0.0 } else { 1.0 };
    let mut wt: Option<String> = None;
    let mut zone: Option<usize> = None;
    for a in &args {
        let kv: Vec<&str> = a.splitn(2, '=').collect();
        if kv.len() != 2 {
            continue;
        }
        let v: f32 = kv[1].parse().unwrap_or(0.0);
        match kv[0] {
            "age" => p.age = v,
            "weight" => p.weight = v,
            "muscle" => p.muscle = v,
            "height" => p.height = v,
            "gender" => p.gender = v,
            "head" => p.head_size = v,
            "wt" => wt = Some(kv[1].to_string()),
            "zone" => zone = Some(kv[1].parse().unwrap_or(0)),
            _ => {}
        }
    }
    let mut ch = generate_with(&p);
    let topo = reference();
    let morph_args: Vec<(String, f32)> = args
        .iter()
        .filter_map(|a| a.strip_prefix("morph=").map(|s| {
            let mut it = s.splitn(2, ':');
            let n = it.next().unwrap().to_string();
            let w = it.next().and_then(|x| x.parse().ok()).unwrap_or(1.0);
            (n, w)
        }))
        .collect();
    let base_positions = ch.positions.clone();
    let morph_set = if morph_args.is_empty() { None } else { Some(anigo_charbase::morphs::build_morphs(&ch)) };
    if let Some(ms) = &morph_set {
        let w: Vec<(&str, f32)> = morph_args.iter().map(|(n, w)| (n.as_str(), *w)).collect();
        ch.positions = ms.apply(&base_positions, &w);
        // o esqueleto também é re-derivado da malha morfada
        ch.skeleton = anigo_charbase::skeleton::Skeleton::derive(&anigo_charbase::skeleton::bone_defs(topo), &ch.positions);
    }
    let mesh = QuadMesh {
        pos: ch.positions.clone(),
        faces: topo.faces.clone(),
    };
    eprintln!(
        "skin: V={} F={} euler={} boundary_loops={} bones={}",
        mesh.pos.len(),
        mesh.faces.len(),
        mesh.euler(),
        mesh.boundary_loops().len(),
        ch.skeleton.len()
    );
    let colors: Option<Vec<Vec3>> = if let Some(bn) = &wt {
        let sk = &ch.skeleton;
        let bi = sk.index(bn).expect("osso desconhecido");
        Some(
            (0..mesh.pos.len())
                .map(|v| {
                    let wv: f32 = topo.weights[v].iter().filter(|(b, _)| *b as usize == bi).map(|(_, w)| *w).sum();
                    Vec3::new(0.15 + 0.85 * wv, 0.2 + 0.1 * (1.0 - wv), 0.9 * (1.0 - wv) + 0.1)
                })
                .collect(),
        )
    } else if let Some(z) = zone {
        Some((0..mesh.pos.len()).map(|v| Vec3::splat(0.2) + Vec3::new(0.8, 0.1, 0.1) * topo.zones[v][z]).collect())
    } else {
        None
    };
    // peças auxiliares (olhos, cílios...) anexadas como quads degenerados com cor própria
    let mut mesh = mesh;
    let mut colors = colors;
    if args.iter().any(|a| a == "parts") {
        let mut parts = anigo_charbase::parts::build_parts(topo, &base_positions, &base_positions, 0.0);
        if let Some(ms) = &morph_set {
            let w: Vec<(&str, f32)> = morph_args.iter().map(|(n, w)| (n.as_str(), *w)).collect();
            parts = ms.apply_parts(&parts, &w);
        }
        let mut cols: Vec<Vec3> = colors.take().unwrap_or_else(|| vec![Vec3::new(0.925, 0.77, 0.69); mesh.pos.len()]);
        for p in &parts {
            let col = match p.material {
                "eye_white" => Vec3::new(0.97, 0.97, 0.98),
                "eye_iris" => Vec3::new(0.15, 0.5, 0.8),
                "eye_highlight" => Vec3::new(1.0, 1.0, 1.0),
                "lash" => Vec3::new(0.08, 0.06, 0.08),
                "brow" => Vec3::new(0.25, 0.15, 0.1),
                "mouth_inside" => Vec3::new(0.45, 0.12, 0.14),
                "teeth" => Vec3::new(0.98, 0.97, 0.93),
                _ => Vec3::new(0.85, 0.35, 0.4),
            };
            let base = mesh.pos.len() as u32;
            mesh.pos.extend(p.positions.iter().copied());
            cols.extend(std::iter::repeat(col).take(p.positions.len()));
            for t in &p.tris {
                mesh.faces.push([base + t[0], base + t[1], base + t[2], base + t[2]]);
            }
        }
        colors = Some(cols);
    }
    let skel_lines: Option<Vec<(Vec3, Vec3)>> = if o.skel {
        let sk = &ch.skeleton;
        Some(
            sk.bones
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.parent.map(|p| (sk.pos(i), sk.pos(p))))
                .collect(),
        )
    } else {
        None
    };
    let d = &ch.params;
    let _ = d;
    let h = ch.skeleton.pos(ch.skeleton.index("head").unwrap()).y + 0.3;
    let hc = ch.skeleton.pos(ch.skeleton.index("head").unwrap()).y + 0.1;
    let hand = ch.skeleton.pos(ch.skeleton.index("leftHand").unwrap());
    let foot = ch.skeleton.pos(ch.skeleton.index("leftFoot").unwrap());
    for view in view.split(',') {
        let (eye_dir, center, half_h, w, hh) = match view {
            "side" => (Vec3::X, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 700, 1100),
            "back" => (-Vec3::Z, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 1100, 1100),
            "head" => (Vec3::Z, Vec3::new(0.0, hc, 0.0), 0.19, 900, 900),
            "headside" => (Vec3::X, Vec3::new(0.0, hc, 0.0), 0.19, 900, 900),
            "head34" => (Vec3::new(0.6, 0.15, 0.8).normalize(), Vec3::new(0.0, hc, 0.0), 0.19, 900, 900),
            "hand" => (Vec3::Y, hand + Vec3::new(0.08, 0.0, 0.0), 0.14, 900, 900),
            "handside" => (Vec3::Z, hand + Vec3::new(0.08, 0.0, 0.0), 0.14, 900, 900),
            "foot" => (Vec3::X, Vec3::new(0.0, 0.06, 0.05), 0.16, 900, 900),
            "foot34" => (Vec3::new(0.6, 0.6, 0.7).normalize(), Vec3::new(foot.x, 0.05, 0.05), 0.17, 900, 900),
            "top" => (Vec3::Y, Vec3::new(0.0, 0.0, 0.0), h * 0.55, 1100, 1100),
            "pelvis" => (Vec3::new(0.0, -0.7, 0.7).normalize(), Vec3::new(0.0, 0.8, 0.0), 0.3, 900, 900),
            "shoulder" => (Vec3::new(0.5, 0.4, 0.7).normalize(), Vec3::new(0.15, 1.25, 0.0), 0.2, 900, 900),
            "torso" => (Vec3::Z, Vec3::new(0.0, 1.05, 0.0), 0.42, 900, 900),
            "torsoside" => (Vec3::X, Vec3::new(0.0, 1.05, 0.0), 0.42, 900, 900),
            _ => (Vec3::Z, Vec3::new(0.0, h * 0.5, 0.0), h * 0.55, 1100, 1100),
        };
        let img = render(&mesh, colors.as_deref(), skel_lines.as_deref(), w, hh, eye_dir, center, half_h, &o);
        let path = if out.ends_with(".png") { out.clone() } else { format!("{out}_{view}.png") };
        img.save(&path).unwrap();
        println!("{path}");
    }
}
