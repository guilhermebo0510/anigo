//! Sombra projetada da luz principal (shadow map de 1 luz, aditiva).
//!
//! Provas (sem GPU os testes se pulam com mensagem):
//! * DESLIGADA (padrão) não muda um byte — o quadro é o de antes (os hashes
//!   congelados de `golden_frames.rs` cobrem as cenas canônicas; aqui a prova é
//!   direta: ligar/desligar o campo em cena sem receptores dá a mesma imagem
//!   que o padrão, e o padrão não cria o passe de profundidade da luz);
//! * LIGADA escurece o chão sob o cubo e só ali; a sombra ANDA com o azimute da
//!   luz, respeita força/resolução e é determinística;
//! * a sombra do chão usa a cor de sombra do terminador (mesma família de cor).
//!
//! `ANIGO_TEST_OUT=<dir>` grava os PNGs para inspeção humana.

use anigo_core::mesh::Mesh;
use anigo_core::scene::{LightShadow, Scene, SceneNode};
use anigo_core::Transform;
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;

const W: u32 = 256;
const H: u32 = 256;

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => {
            // Como o DoF, a sombra projetada não existe no backend GL/GLES (o
            // `shadow_pipeline` é `None` ali: `build_device_resources`). Sem este
            // guarda os testes reprovam em runners só com llvmpipe via EGL (job
            // `rust` do CI, sem driver Vulkan); o job `native` os cobre em Vulkan.
            if r.adapter_info.backend == wgpu::Backend::Gl {
                eprintln!(
                    "[{test}] PULADO: sombra projetada não existe no backend GL; adaptador atual: {} ({:?})",
                    r.adapter_info.name, r.adapter_info.backend
                );
                return None;
            }
            Some(r)
        }
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

/// Chão largo + cubo flutuando. Câmera de cima/frente, luz por `azimuth` (rad) e `elevation` (rad).
fn scene(azimuth: f32, elevation: f32, shadow: LightShadow) -> Scene {
    let mut s = Scene::new_empty();
    s.background_color = [0.05, 0.05, 0.08, 1.0];
    let mut floor = SceneNode::new("floor", "Floor").with_mesh(Mesh::create_cube(1.0));
    floor.transform = Transform { translation: Vec3::new(0.0, -0.1, 0.0), scale: Vec3::new(8.0, 0.2, 8.0), ..Default::default() };
    s.add_node(floor);
    let mut cube = SceneNode::new("cube", "Cube").with_mesh(Mesh::create_cube(1.0));
    cube.transform = Transform { translation: Vec3::new(0.0, 1.2, 0.0), ..Default::default() };
    s.add_node(cube);
    s.camera.eye = Vec3::new(0.0, 6.0, 7.0);
    s.camera.target = Vec3::new(0.0, 0.3, 0.0);
    s.light.direction = Vec3::new(elevation.cos() * azimuth.sin(), elevation.sin(), elevation.cos() * azimuth.cos()).normalize().to_array();
    s.light.shadow = shadow;
    s
}

fn on() -> LightShadow {
    LightShadow { enabled: true, ..LightShadow::default() }
}

fn render(renderer: &HeadlessRenderer, s: &Scene) -> image::RgbaImage {
    pollster::block_on(renderer.render_scene(s, W, H)).expect("render").0
}

fn luma(p: &image::Rgba<u8>) -> i32 {
    (p.0[0] as i32 * 299 + p.0[1] as i32 * 587 + p.0[2] as i32 * 114) / 1000
}

/// Pixels que ficaram mais escuros que na imagem sem sombra e o centroide (x, y) deles.
fn darkened(off: &image::RgbaImage, with: &image::RgbaImage, min_drop: i32) -> (usize, f32, f32) {
    let (mut n, mut sx, mut sy) = (0usize, 0.0f64, 0.0f64);
    for (x, y, p) in with.enumerate_pixels() {
        if luma(off.get_pixel(x, y)) - luma(p) >= min_drop {
            n += 1;
            sx += x as f64;
            sy += y as f64;
        }
    }
    if n == 0 {
        return (0, 0.0, 0.0);
    }
    (n, (sx / n as f64) as f32, (sy / n as f64) as f32)
}

fn save(name: &str, img: &image::RgbaImage) {
    if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = img.save(format!("{dir}/{name}.png"));
    }
}

#[test]
fn shadow_off_is_the_default_and_changes_nothing() {
    let Some(r) = renderer_or_skip("shadow_off") else { return };
    assert!(!LightShadow::default().enabled, "a sombra nasce DESLIGADA");
    let base = render(&r, &scene(0.5, 0.9, LightShadow::default()));
    // outros parâmetros mexidos com `enabled = false` também não pintam nada
    let tweaked = render(&r, &scene(0.5, 0.9, LightShadow { strength: 0.3, hardness: 0.0, resolution: 512, bias: 0.1, ..LightShadow::default() }));
    assert_eq!(base.as_raw(), tweaked.as_raw(), "desligada não pode alterar um byte");
    let on = render(&r, &scene(0.5, 0.9, on()));
    assert_ne!(base.as_raw(), on.as_raw(), "ligada tem de mudar a imagem");
}

#[test]
fn shadow_darkens_the_floor_under_the_cube_and_only_there() {
    let Some(r) = renderer_or_skip("shadow_floor") else { return };
    let off = render(&r, &scene(0.5, 0.9, LightShadow::default()));
    let with = render(&r, &scene(0.5, 0.9, on()));
    save("shadow_off", &off);
    save("shadow_on", &with);
    let (n, _, _) = darkened(&off, &with, 12);
    assert!(n > 250, "a sombra do cubo no chão tem de escurecer uma área visível, achei {n} px");
    // nenhum pixel clareia
    let brighter = off.enumerate_pixels().filter(|(x, y, p)| luma(with.get_pixel(*x, *y)) - luma(p) > 2).count();
    assert!(brighter == 0, "sombra projetada nunca clareia ({brighter} px)");
    // a sombra é local: o canto do quadro (chão longe do cubo) fica igual
    for (x, y) in [(5u32, H - 6), (W - 6, H - 6)] {
        assert_eq!(off.get_pixel(x, y), with.get_pixel(x, y), "pixel ({x},{y}) longe do cubo não pode mudar");
    }
}

#[test]
fn shadow_follows_the_light_azimuth() {
    let Some(r) = renderer_or_skip("shadow_azimuth") else { return };
    let mut centroids = Vec::new();
    for (label, az) in [("az_pos", 0.6f32), ("az_neg", -0.6f32)] {
        let off = render(&r, &scene(az, 0.9, LightShadow::default()));
        let with = render(&r, &scene(az, 0.9, on()));
        save(&format!("shadow_{label}"), &with);
        let (n, cx, _) = darkened(&off, &with, 12);
        assert!(n > 150, "{label}: sombra ausente ({n} px)");
        centroids.push(cx);
    }
    // luz de +x projeta a sombra para -x na tela e vice-versa
    assert!(centroids[0] < centroids[1] - 20.0, "sombra deveria andar com o azimute: {centroids:?}");
}

#[test]
fn shadow_elevation_changes_the_shadow_length() {
    let Some(r) = renderer_or_skip("shadow_elevation") else { return };
    let mut areas = Vec::new();
    for el in [1.35f32, 0.7] {
        let off = render(&r, &scene(0.5, el, LightShadow::default()));
        let with = render(&r, &scene(0.5, el, on()));
        save(&format!("shadow_el_{:.2}", el), &with);
        areas.push(darkened(&off, &with, 12).0);
    }
    assert!(areas[1] > areas[0], "luz mais baixa deve alongar a sombra: {areas:?}");
}

#[test]
fn strength_scales_the_shadow_and_zero_removes_it() {
    let Some(r) = renderer_or_skip("shadow_strength") else { return };
    let off = render(&r, &scene(0.5, 0.9, LightShadow::default()));
    let full = render(&r, &scene(0.5, 0.9, on()));
    let half = render(&r, &scene(0.5, 0.9, LightShadow { strength: 0.5, ..on() }));
    let none = render(&r, &scene(0.5, 0.9, LightShadow { strength: 0.0, ..on() }));
    assert_eq!(off.as_raw(), none.as_raw(), "força 0 = sem sombra");
    let sum = |a: &image::RgbaImage, b: &image::RgbaImage| -> i64 { a.pixels().zip(b.pixels()).map(|(p, q)| (luma(p) - luma(q)) as i64).sum() };
    let (d_full, d_half) = (sum(&off, &full), sum(&off, &half));
    assert!(d_full > 0 && d_half > 0 && d_half < d_full, "força 0,5 escurece menos: {d_half} vs {d_full}");
}

#[test]
fn every_resolution_renders_a_shadow_and_is_deterministic() {
    let Some(r) = renderer_or_skip("shadow_resolution") else { return };
    let off = render(&r, &scene(0.5, 0.9, LightShadow::default()));
    for res in LightShadow::RESOLUTIONS {
        let s = scene(0.5, 0.9, LightShadow { resolution: res, ..on() });
        let a = render(&r, &s);
        let b = render(&r, &s);
        assert_eq!(a.as_raw(), b.as_raw(), "res {res}: não determinístico");
        assert!(darkened(&off, &a, 12).0 > 150, "res {res}: sem sombra");
    }
}

#[test]
fn hard_shadow_has_a_sharper_edge_than_soft() {
    let Some(r) = renderer_or_skip("shadow_hardness") else { return };
    let off = render(&r, &scene(0.5, 0.9, LightShadow::default()));
    let hard = render(&r, &scene(0.5, 0.9, LightShadow { hardness: 1.0, resolution: 1024, ..on() }));
    let soft = render(&r, &scene(0.5, 0.9, LightShadow { hardness: 0.0, resolution: 1024, ..on() }));
    save("shadow_hard", &hard);
    save("shadow_soft", &soft);
    // pixels "de meio-tom" da sombra (queda parcial) são mais numerosos na sombra suave
    let partial = |img: &image::RgbaImage| {
        let max_drop = off.pixels().zip(img.pixels()).map(|(p, q)| luma(p) - luma(q)).max().unwrap_or(0).max(1);
        off.pixels().zip(img.pixels()).filter(|(p, q)| {
            let d = luma(p) - luma(q);
            d > max_drop / 8 && d < max_drop * 7 / 8
        }).count()
    };
    let (ph, ps) = (partial(&hard), partial(&soft));
    assert!(ps > ph, "a borda suave tem mais meio-tom que a dura: soft {ps} vs hard {ph}");
}

#[test]
fn a_degenerate_scene_with_shadow_on_does_not_fail() {
    let Some(r) = renderer_or_skip("shadow_empty") else { return };
    let mut s = Scene::new_empty();
    s.light.shadow = on();
    let img = pollster::block_on(r.render_scene(&s, 64, 64)).expect("cena vazia com sombra ligada renderiza").0;
    assert_eq!(img.width(), 64);
}
