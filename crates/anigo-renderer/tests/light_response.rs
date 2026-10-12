//! A luz responde: prova por pixels (personagem canônico, renderer canônico) de que azimute, elevação,
//! intensidade, ambiente, tinta de sombra e rim mudam a imagem do jeito certo — o que o workspace Iluminação
//! promete. Sem GPU os testes se pulam. `ANIGO_TEST_OUT=<dir>` grava a folha de contato.

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::scene::{Scene, SceneNode, StylizedMaterial};
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;
use image::RgbaImage;

const W: u32 = 256;
const H: u32 = 384;
const BG: [u8; 3] = [5, 5, 10];

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

fn dir(az_deg: f32, el_deg: f32) -> [f32; 3] {
    let (a, e) = (az_deg.to_radians(), el_deg.to_radians());
    [e.cos() * a.sin(), e.sin(), e.cos() * a.cos()]
}

#[derive(Clone)]
struct Look {
    az: f32,
    el: f32,
    intensity: f32,
    ambient: f32,
    shadow_tint: [f32; 3],
    rim: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self { az: 30.0, el: 25.0, intensity: 1.0, ambient: 0.35, shadow_tint: [1.0, 1.0, 1.0], rim: 0.8 }
    }
}

fn render(r: &HeadlessRenderer, look: &Look) -> RgbaImage {
    let mut s = Scene::new_empty();
    s.background_color = [BG[0] as f32 / 255.0, BG[1] as f32 / 255.0, BG[2] as f32 / 255.0, 1.0];
    let mut node = SceneNode::new("c", "Character").with_mesh(Mesh::create_canonical_base(BaseGender::Female));
    node.material = Some(StylizedMaterial { rim_intensity: look.rim, ..Default::default() });
    s.add_node(node);
    s.camera.eye = Vec3::new(0.0, 0.95, 2.9);
    s.camera.target = Vec3::new(0.0, 0.9, 0.0);
    s.light.direction = dir(look.az, look.el);
    s.light.intensity = look.intensity;
    s.light.ambient_intensity = look.ambient;
    s.light.shadow_color = look.shadow_tint;
    pollster::block_on(r.render_scene(&s, W, H)).expect("render").0
}

fn luma(p: &image::Rgba<u8>) -> f32 {
    0.299 * p.0[0] as f32 + 0.587 * p.0[1] as f32 + 0.114 * p.0[2] as f32
}

fn is_subject(p: &image::Rgba<u8>) -> bool {
    (p.0[0] as i32 - BG[0] as i32).abs() > 8 || (p.0[1] as i32 - BG[1] as i32).abs() > 8 || (p.0[2] as i32 - BG[2] as i32).abs() > 8
}

/// (fração "iluminada" (luma > 170) do sujeito, centroide x dos pixels iluminados, luma médio, pixels de sombra)
struct Stats {
    lit_frac: f32,
    lit_cx: f32,
    mean: f32,
    /// pixels do sujeito em sombra (luma < 140; o contorno escuro fica de fora: só o que NÃO é linha)
    shadow_px: usize,
}

fn stats(img: &RgbaImage) -> Stats {
    let (mut area, mut lit, mut sx, mut sum, mut shadow_px) = (0usize, 0usize, 0.0f64, 0.0f64, 0usize);
    for (x, _, p) in img.enumerate_pixels() {
        if !is_subject(p) {
            continue;
        }
        area += 1;
        let l = luma(p);
        sum += l as f64;
        // sombra = tom frio de sombra (azul domina o vermelho), distinto da linha de contorno quase preta
        if l < 140.0 && l > 45.0 && p.0[2] > p.0[0] {
            shadow_px += 1;
        }
        if l > 170.0 {
            lit += 1;
            sx += x as f64;
        }
    }
    let lit_n = lit.max(1) as f64;
    Stats { lit_frac: lit as f32 / area.max(1) as f32, lit_cx: (sx / lit_n) as f32, mean: (sum / area.max(1) as f64) as f32, shadow_px }
}

fn save_sheet(name: &str, imgs: &[RgbaImage]) {
    let Ok(dir) = std::env::var("ANIGO_TEST_OUT") else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut sheet = RgbaImage::new(W * imgs.len() as u32, H);
    for (i, img) in imgs.iter().enumerate() {
        image::imageops::overlay(&mut sheet, img, (i as u32 * W) as i64, 0);
    }
    let _ = sheet.save(format!("{dir}/{name}.png"));
}

#[test]
fn azimuth_moves_the_terminator_across_the_body() {
    let Some(r) = renderer_or_skip("azimuth") else { return };
    let imgs: Vec<RgbaImage> = [-75.0f32, -40.0, 0.0, 40.0, 75.0].iter().map(|az| render(&r, &Look { az: *az, ..Look::default() })).collect();
    save_sheet("light_azimuth", &imgs);
    let cx: Vec<f32> = imgs.iter().map(|i| stats(i).lit_cx).collect();
    // o centroide da parte iluminada acompanha o lado da luz, monotonicamente
    for w in cx.windows(2) {
        assert!(w[0] < w[1] - 1.0, "o terminador deveria andar com o azimute: {cx:?}");
    }
    assert!(cx[4] - cx[0] > 8.0, "varredura de azimute muito pequena: {cx:?}");
}

#[test]
fn elevation_changes_what_is_lit() {
    let Some(r) = renderer_or_skip("elevation") else { return };
    let imgs: Vec<RgbaImage> = [5.0f32, 35.0, 65.0, 88.0].iter().map(|el| render(&r, &Look { az: 0.0, el: *el, ..Look::default() })).collect();
    save_sheet("light_elevation", &imgs);
    let st: Vec<Stats> = imgs.iter().map(stats).collect();
    // luz rasante de frente ilumina mais a frente; subindo a luz a área iluminada encolhe, sempre, e o
    // centroide iluminado sobe (o topo continua iluminado, a frente e as pernas passam à sombra)
    let fr: Vec<f32> = st.iter().map(|s| s.lit_frac).collect();
    for w in fr.windows(2) {
        assert!(w[0] > w[1], "subir a luz reduz a área iluminada de frente: {fr:?}");
    }
    assert!(fr[0] - fr[3] > 0.03, "varredura de elevação muito pequena: {fr:?}");
    // a sombra de baixo da cabeça/ombros cresce com a elevação
    let sp: Vec<usize> = st.iter().map(|s| s.shadow_px).collect();
    assert!(sp[3] > sp[0] + 60, "luz de cima faz sombra sob a cabeça e os ombros: {sp:?}");
}

#[test]
fn intensity_scales_brightness() {
    let Some(r) = renderer_or_skip("intensity") else { return };
    let imgs: Vec<RgbaImage> = [0.25f32, 0.6, 1.0, 1.6].iter().map(|i| render(&r, &Look { intensity: *i, ..Look::default() })).collect();
    save_sheet("light_intensity", &imgs);
    let m: Vec<f32> = imgs.iter().map(|i| stats(i).mean).collect();
    for w in m.windows(2) {
        assert!(w[0] < w[1], "mais intensidade = mais brilho: {m:?}");
    }
    // o tonemap (ACES) comprime os brilhos: a resposta é monótona mas achatada (achado T-3.7 da pesquisa)
    assert!(m[0] < m[3] * 0.85, "intensidade 0,25 bem mais escura que 1,6: {m:?}");
}

#[test]
fn ambient_lifts_the_shadows_and_tint_colors_them() {
    let Some(r) = renderer_or_skip("ambient") else { return };
    // luz de lado para ter uma metade bem na sombra
    let dark = render(&r, &Look { az: 80.0, el: 10.0, ambient: 0.15, ..Look::default() });
    let lifted = render(&r, &Look { az: 80.0, el: 10.0, ambient: 1.4, ..Look::default() });
    let tinted = render(&r, &Look { az: 80.0, el: 10.0, ambient: 1.4, shadow_tint: [1.0, 0.25, 0.25], ..Look::default() });
    save_sheet("light_ambient_tint", &[dark.clone(), lifted.clone(), tinted.clone()]);
    // lado da sombra = metade esquerda da tela (luz vem de +X)
    let shadow_mean = |img: &RgbaImage, f: &dyn Fn(&image::Rgba<u8>) -> f32| {
        let (mut sum, mut n) = (0.0f64, 0usize);
        for (x, _, p) in img.enumerate_pixels() {
            if x < W / 2 && is_subject(p) && luma(p) < 150.0 {
                sum += f(p) as f64;
                n += 1;
            }
        }
        (sum / n.max(1) as f64) as f32
    };
    let l = |p: &image::Rgba<u8>| luma(p);
    assert!(shadow_mean(&lifted, &l) > shadow_mean(&dark, &l) + 15.0, "ambiente alto clareia a sombra");
    // tinta vermelha: vermelho domina o azul na sombra tingida e não na neutra
    let rb = |p: &image::Rgba<u8>| p.0[0] as f32 - p.0[2] as f32;
    assert!(shadow_mean(&tinted, &rb) > shadow_mean(&lifted, &rb) + 15.0, "a tinta de sombra avermelha a sombra");
}

#[test]
fn rim_shows_up_with_backlight_and_responds_to_its_intensity() {
    let Some(r) = renderer_or_skip("rim") else { return };
    let off = render(&r, &Look { az: 170.0, el: 20.0, intensity: 1.2, ambient: 0.9, rim: 0.0, ..Look::default() });
    let on = render(&r, &Look { az: 170.0, el: 20.0, intensity: 1.2, ambient: 0.9, rim: 2.5, ..Look::default() });
    let front = render(&r, &Look { az: 0.0, el: 20.0, intensity: 1.2, ambient: 0.9, rim: 2.5, ..Look::default() });
    save_sheet("light_rim", &[off.clone(), on.clone(), front.clone()]);
    // o rim só soma luz nas bordas: nenhum pixel escurece e vários clareiam
    let (mut brighter, mut darker) = (0usize, 0usize);
    for (a, b) in off.pixels().zip(on.pixels()) {
        let d = luma(b) - luma(a);
        if d > 25.0 {
            brighter += 1;
        }
        if d < -2.0 {
            darker += 1;
        }
    }
    assert!(brighter > 300, "rim 2,5 com contraluz deveria acender a silhueta ({brighter} px)");
    assert_eq!(darker, 0, "rim nunca escurece");
    // com a luz de frente o rim (que depende do contraluz) acende bem menos que com contraluz
    let front_off = render(&r, &Look { az: 0.0, el: 20.0, intensity: 1.2, ambient: 0.9, rim: 0.0, ..Look::default() });
    let front_brighter = front_off.pixels().zip(front.pixels()).filter(|(a, b)| luma(b) - luma(a) > 25.0).count();
    assert!(front_brighter < brighter, "o rim é mais forte com contraluz ({brighter}) que com luz de frente ({front_brighter})");
}

#[test]
fn the_same_light_is_deterministic() {
    let Some(r) = renderer_or_skip("determinism") else { return };
    let look = Look { az: -33.0, el: 41.0, ..Look::default() };
    assert_eq!(render(&r, &look).as_raw(), render(&r, &look).as_raw());
}
