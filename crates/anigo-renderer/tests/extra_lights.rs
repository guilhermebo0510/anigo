//! Luzes de cena (ponto, holofote, painel retangular e direcional extra), ADITIVAS à luz principal.
//!
//! Provas (sem GPU os testes se pulam com mensagem):
//! * SEM luzes (padrão) o quadro é o de antes — os hashes congelados de `golden_frames.rs` cobrem as cenas canônicas e aqui
//!   `a_scene_without_extra_lights_is_deterministic` fixa o contrato;
//! * o ponto clareia só perto dele e some no alcance; o holofote só dentro do cone; o painel só pela face da frente;
//!   a direcional extra clareia a cena toda do lado dela; mais luzes que o máximo são ignoradas.
//!
//! `ANIGO_TEST_OUT=<dir>` grava os PNGs para inspeção humana.

use anigo_core::mesh::Mesh;
use anigo_core::scene::{RenderLight, Scene, SceneNode, MAX_RENDER_LIGHTS};
use anigo_core::{LightShape, Transform};
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;

const W: u32 = 256;
const H: u32 = 256;

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

/// Só um chão largo, visto de cima, com a luz principal fraca (o chão não satura: dá para ver o que as extras somam).
fn floor_scene() -> Scene {
    let mut s = Scene::new_empty();
    s.background_color = [0.0, 0.0, 0.0, 1.0];
    let mut floor = SceneNode::new("floor", "Floor").with_mesh(Mesh::create_cube(1.0));
    floor.transform = Transform { translation: Vec3::new(0.0, -0.1, 0.0), scale: Vec3::new(12.0, 0.2, 12.0), ..Default::default() };
    s.add_node(floor);
    s.camera.eye = Vec3::new(0.0, 12.0, 0.01);
    s.camera.target = Vec3::ZERO;
    s.light.intensity = 0.2;
    s.light.ambient_intensity = 0.05;
    s
}

fn light(kind: LightShape, position: [f32; 3], direction: [f32; 3]) -> RenderLight {
    RenderLight { kind, position, direction, color: [1.0, 1.0, 1.0], intensity: 1.5, range: 6.0, spot_angle_deg: 40.0, spot_softness: 0.2, size: [2.0, 2.0] }
}

fn render(r: &HeadlessRenderer, s: &Scene) -> image::RgbaImage {
    pollster::block_on(r.render_scene(s, W, H)).expect("render").0
}

fn luma(p: &image::Rgba<u8>) -> i32 {
    (p.0[0] as i32 * 299 + p.0[1] as i32 * 587 + p.0[2] as i32 * 114) / 1000
}

/// Pixels que ficaram mais claros com a luz e o centroide (x, y) deles.
fn brightened(off: &image::RgbaImage, with: &image::RgbaImage, min_gain: i32) -> (usize, f32, f32) {
    let (mut n, mut sx, mut sy) = (0usize, 0.0f64, 0.0f64);
    for (x, y, p) in with.enumerate_pixels() {
        if luma(p) - luma(off.get_pixel(x, y)) >= min_gain {
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
        let _ = img.save(std::path::Path::new(&dir).join(format!("{name}.png")));
    }
}

#[test]
fn a_scene_without_extra_lights_is_deterministic() {
    let Some(r) = renderer_or_skip("deterministic") else { return };
    let s = floor_scene();
    assert!(s.extra_lights.is_empty());
    assert_eq!(render(&r, &s).as_raw(), render(&r, &s).as_raw());
}

#[test]
fn a_point_light_brightens_only_around_it() {
    let Some(r) = renderer_or_skip("point") else { return };
    let off = render(&r, &floor_scene());
    let mut s = floor_scene();
    // à direita (x > 0) do centro da imagem, a 1,5 m do chão, alcance 6 m: a esquerda do chão (x < -4) fica de fora
    s.extra_lights.push(light(LightShape::Point, [3.0, 1.5, 0.0], [0.0, -1.0, 0.0]));
    let on = render(&r, &s);
    save("point", &on);
    let (n, cx, _) = brightened(&off, &on, 6);
    assert!(n > 500, "o ponto devia clarear uma região: {n}");
    assert!(cx > W as f32 * 0.55, "região à direita do centro: cx={cx}");
    // a borda esquerda está fora do alcance: nenhuma mudança
    for y in 0..H {
        assert_eq!(off.get_pixel(0, y), on.get_pixel(0, y));
    }
}

#[test]
fn a_spot_light_lights_only_inside_its_cone() {
    let Some(r) = renderer_or_skip("spot") else { return };
    let off = render(&r, &floor_scene());
    let mut s = floor_scene();
    // holofote em (-3, 4, 0) apontando para baixo, cone de 40° → ~1,5 m de raio no chão
    let mut l = light(LightShape::Spot, [-3.0, 4.0, 0.0], [0.0, -1.0, 0.0]);
    l.range = 12.0;
    s.extra_lights.push(l);
    let on = render(&r, &s);
    save("spot", &on);
    let (n, cx, _) = brightened(&off, &on, 6);
    let total = (W * H) as usize;
    assert!(n > 300, "o cone devia clarear uma região: {n}");
    assert!(n < total / 4, "o cone é pequeno: {n} de {total}");
    assert!(cx < W as f32 * 0.45, "região à esquerda do centro: cx={cx}");
}

#[test]
fn a_rect_light_only_lights_through_its_front_face() {
    let Some(r) = renderer_or_skip("rect") else { return };
    let off = render(&r, &floor_scene());
    // painel de frente para baixo (chão iluminado)
    let mut front = floor_scene();
    front.extra_lights.push(light(LightShape::Rect, [0.0, 1.5, 0.0], [0.0, -1.0, 0.0]));
    let lit = render(&r, &front);
    // o mesmo painel virado para cima: o chão fica na face de trás e não recebe nada
    let mut back = floor_scene();
    back.extra_lights.push(light(LightShape::Rect, [0.0, 1.5, 0.0], [0.0, 1.0, 0.0]));
    let dark = render(&r, &back);
    save("rect_front", &lit);
    save("rect_back", &dark);
    let (n_front, _, _) = brightened(&off, &lit, 6);
    let (n_back, _, _) = brightened(&off, &dark, 3);
    assert!(n_front > 1000, "a face da frente ilumina: {n_front}");
    assert_eq!(n_back, 0, "a face de trás não ilumina");
}

#[test]
fn an_extra_directional_light_brightens_the_whole_floor() {
    let Some(r) = renderer_or_skip("directional") else { return };
    let off = render(&r, &floor_scene());
    let mut s = floor_scene();
    s.extra_lights.push(light(LightShape::Directional, [0.0, 5.0, 0.0], [0.0, -1.0, 0.0]));
    let on = render(&r, &s);
    save("directional", &on);
    let (n, _, _) = brightened(&off, &on, 6);
    assert!(n > (W * H) as usize / 4, "a direcional clareia o chão inteiro: {n}");
}

#[test]
fn more_lights_than_the_maximum_are_ignored() {
    let Some(r) = renderer_or_skip("max") else { return };
    let mut eight = floor_scene();
    let mut twelve = floor_scene();
    for i in 0..12 {
        let l = light(LightShape::Point, [-4.0 + i as f32 * 0.7, 1.0, 0.0], [0.0, -1.0, 0.0]);
        if i < MAX_RENDER_LIGHTS {
            eight.extra_lights.push(l);
        }
        twelve.extra_lights.push(l);
    }
    assert_eq!(render(&r, &eight).as_raw(), render(&r, &twelve).as_raw());
}
