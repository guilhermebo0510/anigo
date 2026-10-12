//! Folha de modelo do personagem base (frente/lado/costas/rosto, macros extremos, expressões,
//! mão/pé) renderizada pelo renderer cel headless (lavapipe ou qualquer adaptador Vulkan).
//! O renderer segue com a paleta de 24 ossos: o skin passa por `collapse_to_canonical24`.
//!
//! Uso: `cargo run -p anigo-charbase --example render_sheet [pasta_saida] [folha ...]`
//! Folhas: female, male, macros, expressions, details, topology. Padrão: todas.
//! Salva `charbase_<folha>.png` (<= 150 KB cada).

use anigo_charbase::character::BaseCharacter;
use anigo_charbase::debugview::{render as wire_render, ViewOpts};
use anigo_charbase::morphs::{build_morphs, MorphSet};
use anigo_charbase::parts::{build_parts, Part};
use anigo_charbase::preview::{part_bone24, part_core_mesh, skin_core_mesh};
use anigo_charbase::qmesh::QuadMesh;
use anigo_charbase::topology::reference;
use anigo_charbase::{generate, generate_with, Gender, Params};
use anigo_core::math::OrthographicBounds;
use anigo_core::scene::{Scene, SceneNode, StylizedMaterial};
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;
use image::{imageops, ImageBuffer, Rgba, RgbaImage};

const MAX_BYTES: usize = 150_000;

fn mat(name: &str, base: [f32; 3], shade: [f32; 3], outline: f32) -> StylizedMaterial {
    let mut m = StylizedMaterial::default();
    m.name = name.into();
    m.base_color = [base[0], base[1], base[2], 1.0];
    m.shade_color = [shade[0], shade[1], shade[2], 1.0];
    m.outline_width = outline;
    m.outline_color = [0.30, 0.17, 0.17, 1.0];
    m.spec_intensity = 0.06;
    m.rim_intensity = 0.35;
    m
}

fn part_material(material: &str) -> StylizedMaterial {
    match material {
        "eye_white" => mat("eye_white", [0.98, 0.98, 1.0], [0.84, 0.86, 0.94], 0.0),
        "eye_iris" => mat("eye_iris", [0.20, 0.55, 0.85], [0.12, 0.35, 0.65], 0.0),
        "eye_highlight" => mat("eye_highlight", [1.0, 1.0, 1.0], [1.0, 1.0, 1.0], 0.0),
        "lash" => mat("lash", [0.10, 0.07, 0.10], [0.10, 0.07, 0.10], 0.0),
        "brow" => mat("brow", [0.30, 0.18, 0.12], [0.22, 0.13, 0.10], 0.0),
        "mouth_inside" => mat("mouth_inside", [0.45, 0.12, 0.15], [0.30, 0.08, 0.10], 0.0),
        "teeth" => mat("teeth", [0.99, 0.98, 0.95], [0.9, 0.88, 0.9], 0.0),
        _ => mat("tongue", [0.88, 0.40, 0.45], [0.7, 0.3, 0.38], 0.0),
    }
}

fn scene_of(ch: &BaseCharacter, parts: &[Part], eye: Vec3, target: Vec3, height: f32, aspect: f32) -> Scene {
    let mut scene = Scene::new_empty();
    let mut skin = SceneNode::new("skin", "Skin").with_mesh(skin_core_mesh(ch));
    skin.material = Some(mat("skin", [0.72, 0.46, 0.40], [0.48, 0.28, 0.30], 0.0030));
    scene.add_node(skin);
    for p in parts {
        let mut n = SceneNode::new(p.name.clone(), p.name.clone()).with_mesh(part_core_mesh(p, part_bone24(&p.name)));
        n.material = Some(part_material(p.material));
        scene.add_node(n);
    }
    scene.camera.eye = eye;
    scene.camera.target = target;
    if (target - eye).normalize().dot(Vec3::Y).abs() > 0.98 {
        scene.camera.up = Vec3::Z;
    }
    scene.camera.aspect = aspect;
    scene.camera.orthographic = Some(OrthographicBounds::from_height(height, aspect));
    scene.camera.z_near = 0.05;
    scene.camera.z_far = 20.0;
    scene.background_color = [0.93, 0.94, 0.97, 1.0];
    // luz fixa em relação à câmera: sempre de frente, um pouco de cima e da direita
    let fwd = (target - eye).normalize();
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let right = if right.length() < 1e-3 { Vec3::X } else { right };
    let up = right.cross(fwd).normalize();
    scene.light.direction = (-fwd * 0.75 + right * 0.40 + up * 0.55).normalize().to_array();
    scene
}

struct Panel {
    eye_dir: Vec3,
    target: Vec3,
    height: f32,
    w: u32,
    h: u32,
}

fn render_panel(r: &HeadlessRenderer, ch: &BaseCharacter, parts: &[Part], p: &Panel) -> RgbaImage {
    let aspect = p.w as f32 / p.h as f32;
    let scene = scene_of(ch, parts, p.target + p.eye_dir.normalize() * 6.0, p.target, p.height, aspect);
    let (img, _) = pollster::block_on(r.render_scene(&scene, p.w, p.h)).expect("render");
    img
}

fn concat(panels: &[RgbaImage]) -> RgbaImage {
    let w: u32 = panels.iter().map(|p| p.width()).sum();
    let h: u32 = panels.iter().map(|p| p.height()).max().unwrap_or(1);
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([238, 240, 245, 255]));
    let mut x = 0;
    for p in panels {
        imageops::replace(&mut out, p, x as i64, 0);
        x += p.width();
    }
    out
}

fn grid(rows: &[RgbaImage]) -> RgbaImage {
    let w: u32 = rows.iter().map(|p| p.width()).max().unwrap_or(1);
    let h: u32 = rows.iter().map(|p| p.height()).sum();
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([238, 240, 245, 255]));
    let mut y = 0;
    for p in rows {
        imageops::replace(&mut out, p, 0, y as i64);
        y += p.height();
    }
    out
}

fn encode_png(img: &RgbaImage) -> Vec<u8> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut buf = Vec::new();
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    PngEncoder::new_with_quality(&mut buf, CompressionType::Best, FilterType::Adaptive)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .expect("png");
    buf
}

/// Salva reduzindo a escala até caber em 150 KB.
fn save(out: &str, name: &str, img: RgbaImage) {
    let mut cur = img;
    let mut bytes = encode_png(&cur);
    let mut guard = 0;
    while bytes.len() > MAX_BYTES && guard < 8 {
        let (w, h) = (cur.width() * 85 / 100, cur.height() * 85 / 100);
        cur = imageops::resize(&cur, w.max(1), h.max(1), imageops::FilterType::Lanczos3);
        bytes = encode_png(&cur);
        guard += 1;
    }
    let path = format!("{out}/{name}.png");
    std::fs::write(&path, &bytes).expect("gravar png");
    println!("{path}  {}x{}  {} KB", cur.width(), cur.height(), bytes.len() / 1024);
}

fn full_body_panels(h: f32, w: u32, ph: u32) -> Vec<Panel> {
    let c = Vec3::new(0.0, h * 0.5, 0.0);
    vec![
        Panel { eye_dir: Vec3::Z, target: c, height: h * 1.08, w: ph * 9 / 10, h: ph },
        Panel { eye_dir: Vec3::X, target: c, height: h * 1.08, w: w, h: ph },
        Panel { eye_dir: -Vec3::Z, target: c, height: h * 1.08, w: ph * 9 / 10, h: ph },
    ]
}

fn sheet_character(r: &HeadlessRenderer, ch: &BaseCharacter, morphs: &MorphSet) -> RgbaImage {
    let _ = morphs;
    let parts = build_parts(reference(), &ch.positions, &ch.positions, 0.0);
    let h = ch.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    let head = ch.skeleton.pos(ch.skeleton.index("head").unwrap());
    let mut imgs: Vec<RgbaImage> = full_body_panels(h, 230, 440).iter().map(|p| render_panel(r, ch, &parts, p)).collect();
    let face_c = head + Vec3::new(0.0, 0.1, 0.0);
    for dir in [Vec3::Z, Vec3::new(0.62, 0.12, 0.78)] {
        imgs.push(render_panel(r, ch, &parts, &Panel { eye_dir: dir, target: face_c, height: 0.34, w: 440, h: 440 }));
    }
    concat(&imgs)
}

fn sheet_macros(r: &HeadlessRenderer) -> RgbaImage {
    let mut rows = Vec::new();
    for g in [Gender::Female, Gender::Male] {
        let base = generate(g);
        let ms = build_morphs(&base);
        let variants: [(&str, Vec<(&str, f32)>); 6] = [
            ("magro", vec![("weight_thin", 1.0)]),
            ("base", vec![]),
            ("gordo", vec![("weight_heavy", 1.0)]),
            ("musculoso", vec![("muscle_high", 1.0)]),
            ("idoso", vec![("age_elderly", 1.0)]),
            ("alto", vec![("height_tall", 1.0)]),
        ];
        let mut imgs = Vec::new();
        for (_, w) in variants.iter() {
            let positions = ms.apply(&base.positions, w);
            let mut ch = base.clone();
            ch.positions = positions;
            ch.normals = QuadMesh { pos: ch.positions.clone(), faces: reference().faces.clone() }.vertex_normals();
            ch.skeleton = anigo_charbase::skeleton::Skeleton::derive(&anigo_charbase::skeleton::bone_defs(reference()), &ch.positions);
            let base_parts = build_parts(reference(), &base.positions, &base.positions, 0.0);
            let parts = ms.apply_parts(&base_parts, w);
            let h = 1.78;
            imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: Vec3::Z, target: Vec3::new(0.0, h * 0.5 - 0.02, 0.0), height: h * 1.12, w: 290, h: 380 }));
        }
        rows.push(concat(&imgs));
    }
    grid(&rows)
}

fn sheet_expressions(r: &HeadlessRenderer) -> RgbaImage {
    let base = generate(Gender::Female);
    let ms = build_morphs(&base);
    let head = base.skeleton.pos(base.skeleton.index("head").unwrap());
    let names = ["neutral", "happy", "angry", "sad", "surprised", "aa", "ou", "blink"];
    let mut imgs = Vec::new();
    for n in names {
        let w = [(n, 1.0f32)];
        let mut ch = base.clone();
        ch.positions = ms.apply(&base.positions, &w);
        ch.normals = QuadMesh { pos: ch.positions.clone(), faces: reference().faces.clone() }.vertex_normals();
        let base_parts = build_parts(reference(), &base.positions, &base.positions, 0.0);
        let parts = ms.apply_parts(&base_parts, &w);
        imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: Vec3::Z, target: head + Vec3::new(0.0, 0.08, 0.0), height: 0.30, w: 230, h: 230 }));
    }
    grid(&[concat(&imgs[..4]), concat(&imgs[4..])])
}

fn sheet_details(r: &HeadlessRenderer) -> RgbaImage {
    let ch = generate(Gender::Female);
    let parts = build_parts(reference(), &ch.positions, &ch.positions, 0.0);
    let hand = ch.skeleton.pos(ch.skeleton.index("leftHand").unwrap());
    let foot = ch.skeleton.pos(ch.skeleton.index("leftFoot").unwrap());
    let mut imgs = Vec::new();
    imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: Vec3::Y, target: hand + Vec3::new(0.08, 0.0, 0.0), height: 0.28, w: 300, h: 300 }));
    imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: -Vec3::Y, target: hand + Vec3::new(0.08, 0.0, 0.0), height: 0.28, w: 300, h: 300 }));
    imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: Vec3::new(0.7, 0.5, 0.5), target: Vec3::new(foot.x, 0.05, 0.05), height: 0.34, w: 300, h: 300 }));
    imgs.push(render_panel(r, &ch, &parts, &Panel { eye_dir: Vec3::new(0.2, -0.3, 0.9), target: Vec3::new(0.0, 0.85, 0.0), height: 0.62, w: 300, h: 300 }));
    concat(&imgs)
}

fn sheet_topology() -> RgbaImage {
    let ch = generate(Gender::Female);
    let t = reference();
    let mesh = QuadMesh { pos: ch.positions.clone(), faces: t.faces.clone() };
    let head = ch.skeleton.pos(ch.skeleton.index("head").unwrap());
    let hand = ch.skeleton.pos(ch.skeleton.index("leftHand").unwrap());
    let o = ViewOpts { wire: true, flat: true, ..Default::default() };
    let hc = head + Vec3::new(0.0, 0.1, 0.0);
    let a = wire_render(&mesh, None, None, 420, 420, Vec3::Z, hc, 0.17, &o);
    let b = wire_render(&mesh, None, None, 420, 420, Vec3::new(0.62, 0.12, 0.78), hc, 0.17, &o);
    let c = wire_render(&mesh, None, None, 420, 420, Vec3::Y, hand + Vec3::new(0.08, 0.0, 0.0), 0.13, &o);
    let d = wire_render(&mesh, None, None, 420, 420, Vec3::new(0.0, -0.7, 0.7), Vec3::new(0.0, 0.8, 0.0), 0.29, &o);
    concat(&[a, b, c, d])
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| format!("{}/../../docs/native/img", env!("CARGO_MANIFEST_DIR")));
    let mut which: Vec<String> = args.collect();
    if which.is_empty() {
        which = ["female", "male", "macros", "expressions", "details", "topology"].iter().map(|s| s.to_string()).collect();
    }
    std::fs::create_dir_all(&out).unwrap();
    let renderer = pollster::block_on(HeadlessRenderer::new()).expect("sem adaptador GPU (use lavapipe: VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json)");
    for w in which {
        match w.as_str() {
            "female" => {
                let ch = generate(Gender::Female);
                let m = build_morphs(&ch);
                save(&out, "charbase_female", sheet_character(&renderer, &ch, &m));
            }
            "male" => {
                let ch = generate(Gender::Male);
                let m = build_morphs(&ch);
                save(&out, "charbase_male", sheet_character(&renderer, &ch, &m));
            }
            "macros" => save(&out, "charbase_macros", sheet_macros(&renderer)),
            "expressions" => save(&out, "charbase_expressions", sheet_expressions(&renderer)),
            "details" => save(&out, "charbase_details", sheet_details(&renderer)),
            "topology" => save(&out, "charbase_topology", sheet_topology()),
            // inspeção em tamanho grande (não faz parte das figuras da documentação)
            "big_f" | "big_m" => {
                let ch = generate(if w == "big_f" { Gender::Female } else { Gender::Male });
                let parts = build_parts(reference(), &ch.positions, &ch.positions, 0.0);
                let c = Vec3::new(0.0, 0.88, 0.0);
                let a = render_panel(&renderer, &ch, &parts, &Panel { eye_dir: Vec3::Z, target: c, height: 1.9, w: 700, h: 900 });
                let b = render_panel(&renderer, &ch, &parts, &Panel { eye_dir: Vec3::new(0.8, 0.1, 0.6), target: c, height: 1.9, w: 700, h: 900 });
                save(&out, &format!("big_{}", &w[4..]), concat(&[a, b]));
            }
            other => eprintln!("folha desconhecida: {other}"),
        }
    }
    let _ = generate_with(&Params::default());
}
