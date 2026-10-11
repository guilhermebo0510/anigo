//! Testes de comportamento do motor cel (Fase 0/1): cada recurso opcional tem
//! de (a) não mudar nenhum pixel com o parâmetro no padrão e (b) mudar a imagem
//! do jeito esperado quando ligado. Rodam em qualquer adaptador (se não houver
//! GPU, pulam com mensagem) — não dependem do hash do lavapipe.

use anigo_core::mesh::Mesh;
use anigo_core::scene::{Scene, SceneNode, StylizedMaterial};
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;

const SIZE: u32 = 192;

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

fn render(renderer: &HeadlessRenderer, scene: &Scene) -> Vec<u8> {
    let (image, _) = pollster::block_on(renderer.render_scene(scene, SIZE, SIZE))
        .expect("render_scene deve funcionar");
    image.into_raw()
}

/// Esfera de sonda: câmera fechada, luz oblíqua.
fn probe_sphere(tweak: impl Fn(&mut StylizedMaterial)) -> Scene {
    let mut scene = Scene::new_empty();
    let mut node = SceneNode::new("probe", "Probe").with_mesh(Mesh::create_uv_sphere(0.5, 32, 48));
    let mut material = StylizedMaterial::default();
    tweak(&mut material);
    node.material = Some(material);
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 0.0, 2.2);
    scene.camera.target = Vec3::ZERO;
    scene.light.direction = Vec3::new(0.65, 0.45, 0.62).normalize().to_array();
    scene
}

fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| p != q)
        .count()
}

#[test]
fn ramp_2d_off_is_byte_identical_and_on_changes_the_terminator() {
    let Some(renderer) = renderer_or_skip("ramp_2d") else { return };
    let baseline = render(&renderer, &probe_sphere(|_| {}));

    // blend = 0 com qualquer linha: nada muda (nem a amostra da rampa entra).
    let off = render(&renderer, &probe_sphere(|m| {
        m.npr.ramp_blend = 0.0;
        m.npr.ramp_row = 3.0;
    }));
    assert_eq!(baseline, off, "ramp_blend = 0 tem de ser byte a byte o motor anterior");

    // linha 2 (dois degraus) com blend 1: aparece uma banda a mais.
    let two_steps = render(&renderer, &probe_sphere(|m| {
        m.npr.ramp_blend = 1.0;
        m.npr.ramp_row = 2.0;
    }));
    assert!(
        differing_pixels(&baseline, &two_steps) > 800,
        "a rampa 2D (linha 2) deveria mudar a sombra de forma visível"
    );

    // linha 1 (um degrau) == analítico de 1 degrau, salvo a penumbra de 1 texel
    // (a rampa tem borda dura, o analítico tem smoothstep de 0.02).
    let one_step = render(&renderer, &probe_sphere(|m| {
        m.npr.ramp_blend = 1.0;
        m.npr.ramp_row = 1.0;
    }));
    let changed = differing_pixels(&baseline, &one_step);
    assert!(
        changed < 900,
        "linha 1 da rampa difere do analítico só na borda do terminador ({changed} px)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Contorno (inverted hull)
// ─────────────────────────────────────────────────────────────────────────────

const OUTLINE_RGB: [f32; 3] = [0.25, 0.15, 0.20];

/// Pixels cuja cor está a menos de `tolerance` (soma de |Δcanal|) de (64, 38, 51),
/// a cor de linha dos testes (o MSAA mistura nas bordas, daí a tolerância).
fn pixels_near_outline_color(raw: &[u8], tolerance: i32) -> usize {
    let target = [64i32, 38, 51];
    raw.chunks_exact(4)
        .filter(|px| (0..3).map(|i| (px[i] as i32 - target[i]).abs()).sum::<i32>() < tolerance)
        .count()
}

fn outline_pixels(raw: &[u8]) -> usize {
    pixels_near_outline_color(raw, 40)
}

/// Cubo girado sobre fundo verde puro, contorno largo.
fn hull_cube(smooth: f32) -> Scene {
    let mut scene = Scene::new_empty();
    let mut node = SceneNode::new("cube", "Cube").with_mesh(Mesh::create_cube(1.0));
    node.transform.rotation = glam::Quat::from_euler(glam::EulerRot::YXZ, 0.55, 0.45, 0.0);
    let mut material = StylizedMaterial::default();
    material.outline_width = 0.03;
    material.outline_color = [OUTLINE_RGB[0], OUTLINE_RGB[1], OUTLINE_RGB[2], 1.0];
    material.npr.outline_smooth_normals = smooth;
    node.material = Some(material);
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 0.0, 3.2);
    scene.camera.target = Vec3::ZERO;
    scene.background_color = [0.1, 0.8, 0.2, 1.0];
    scene
}

fn render_size(renderer: &HeadlessRenderer, scene: &Scene, size: u32) -> Vec<u8> {
    let (image, _) = pollster::block_on(renderer.render_scene(scene, size, size)).unwrap();
    image.into_raw()
}

/// Pixels de face clara (quase branca) com fundo verde a até 2 px: onde há uma
/// fresta no hull a face "vaza" direto para o fundo, sem linha no meio.
fn face_pixels_touching_background(raw: &[u8], size: u32) -> usize {
    let at = |x: i64, y: i64| {
        let i = ((y as u32 * size + x as u32) * 4) as usize;
        &raw[i..i + 4]
    };
    let is_face = |p: &[u8]| p[0] > 215 && p[1] > 215 && p[2] > 205;
    let is_bg = |p: &[u8]| p[1] > 170 && p[0] < 90 && p[2] < 110;
    let mut count = 0;
    let n = size as i64;
    for y in 2..n - 2 {
        for x in 2..n - 2 {
            if !is_face(at(x, y)) {
                continue;
            }
            'near: for dy in -2..=2i64 {
                for dx in -2..=2i64 {
                    if is_bg(at(x + dx, y + dy)) {
                        count += 1;
                        break 'near;
                    }
                }
            }
        }
    }
    count
}

#[test]
fn smoothed_outline_normals_close_the_hull_gaps_of_a_hard_edged_cube() {
    let Some(renderer) = renderer_or_skip("hull_gaps") else { return };
    let mesh_normals = render_size(&renderer, &hull_cube(0.0), 320);
    let smoothed = render_size(&renderer, &hull_cube(1.0), 320);
    let gaps_mesh = face_pixels_touching_background(&mesh_normals, 320);
    let gaps_smooth = face_pixels_touching_background(&smoothed, 320);
    assert!(
        gaps_mesh >= 8,
        "com normais da malha o hull do cubo abre frestas nos cantos ({gaps_mesh} px de face encostando no fundo)"
    );
    assert_eq!(
        gaps_smooth, 0,
        "com normais suavizadas o hull fecha: nenhuma face encosta no fundo"
    );
}

/// Base canônica a `distance` m, linha nominal grossa, fundo claro.
fn base_at(distance: f32, tweak: impl Fn(&mut StylizedMaterial)) -> Scene {
    let mesh = Mesh::create_canonical_base(anigo_core::mesh::BaseGender::Female);
    let mut scene = Scene::new_empty();
    let mut node = SceneNode::new("base", "Base").with_mesh(mesh);
    let mut material = StylizedMaterial::default();
    material.outline_width = 0.008;
    material.outline_color = [OUTLINE_RGB[0], OUTLINE_RGB[1], OUTLINE_RGB[2], 1.0];
    tweak(&mut material);
    node.material = Some(material);
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 1.0, distance);
    scene.camera.target = Vec3::new(0.0, 0.9, 0.0);
    scene.background_color = [0.86, 0.88, 0.93, 1.0];
    scene
}

#[test]
fn outline_params_at_default_are_byte_identical_to_the_previous_engine() {
    let Some(renderer) = renderer_or_skip("outline_defaults") else { return };
    let baseline = render_size(&renderer, &base_at(3.0, |_| {}), 256);
    // Campos auxiliares em valores esquisitos mas com os "liga/desliga" no padrão
    // (falloff 0 / taper 0 / tinta 0 / suavização 0) não mudam um pixel.
    let neutral = render_size(
        &renderer,
        &base_at(3.0, |m| {
            m.npr.outline_ref_distance = 12.0;
            m.npr.outline_min_width_scale = 0.9;
            m.npr.outline_ink_darken = 0.1;
            m.npr.outline_ink_saturation = 0.3;
        }),
        256,
    );
    assert_eq!(baseline, neutral);
}

#[test]
fn distance_falloff_thins_far_lines_and_thickens_near_ones() {
    let Some(renderer) = renderer_or_skip("falloff") else { return };
    let falloff = |m: &mut StylizedMaterial| {
        m.npr.outline_distance_falloff = 1.0;
        m.npr.outline_ref_distance = 5.0;
    };
    let count = |distance: f32, on: bool| {
        let scene = if on {
            base_at(distance, falloff)
        } else {
            base_at(distance, |_| {})
        };
        outline_pixels(&render_size(&renderer, &scene, 384)) as f64
    };
    // Mesma câmera/personagem; só o falloff muda a largura da linha.
    let far_const = count(10.0, false);
    let far_falloff = count(10.0, true);
    let near_const = count(2.5, false);
    let near_falloff = count(2.5, true);
    assert!(far_falloff < far_const * 0.75, "ao longe a linha afina ({far_falloff} vs {far_const})");
    assert!(near_falloff > near_const * 1.4, "perto a linha engrossa ({near_falloff} vs {near_const})");
}

#[test]
fn taper_thins_the_line_and_ink_changes_its_color() {
    let Some(renderer) = renderer_or_skip("taper_ink") else { return };
    let plain = render_size(&renderer, &base_at(2.5, |_| {}), 384);
    let tapered = render_size(&renderer, &base_at(2.5, |m| m.npr.outline_taper = 1.0), 384);
    assert!(
        outline_pixels(&tapered) < outline_pixels(&plain),
        "taper afina a linha: {} vs {}",
        outline_pixels(&tapered),
        outline_pixels(&plain)
    );
    let inked = render_size(&renderer, &base_at(2.5, |m| m.npr.outline_ink_blend = 1.0), 384);
    // a tinta (marrom derivado da pele, ~(75, 63, 50)) não é mais a cor (64, 38, 51)
    assert!(pixels_near_outline_color(&plain, 12) > 500);
    assert_eq!(pixels_near_outline_color(&inked, 12), 0, "a linha deixou de ter a cor outline_color");
    assert_ne!(plain, inked);
}

// ─────────────────────────────────────────────────────────────────────────────
// Face SDF
// ─────────────────────────────────────────────────────────────────────────────

/// Cabeça (esfera UV) com sombra facial SDF e luz a `angle_deg` do eixo frontal
/// (+Z), positivo para +X. O terminador do cel fica fora do caminho
/// (`shadow_threshold` baixo) para o SDF ser o único sombreador visível.
fn face_head(angle_deg: f32, mirror_center: f32) -> Scene {
    let a = angle_deg.to_radians();
    let mut scene = probe_sphere(|m| {
        m.face_sdf_enabled = true;
        m.face_shadow_smoothness = 0.04;
        m.shadow_threshold = 0.2;
        m.spec_intensity = 0.0;
        m.rim_intensity = 0.0;
        m.npr.face_sdf_mirror_center = mirror_center;
    });
    scene.light.direction = [a.sin(), 0.0, a.cos()];
    scene
}

fn install_face_map(renderer: &HeadlessRenderer) {
    // rosto = metade u ∈ [0, 0.5] da esfera UV; sombra começa no lado de u alto
    let sdf = anigo_renderer::face_sdf_map::procedural_face_sdf_f32(256, 128, 0.25, 0.25, true);
    renderer.set_face_sdf_map_f32(256, 128, &sdf).unwrap();
}

/// Luminância média (0..255) de uma metade vertical do disco central da imagem.
fn half_luma(raw: &[u8], size: u32, right: bool) -> f64 {
    let (mut sum, mut n) = (0.0f64, 0u32);
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            // dentro do disco da esfera (raio 0.5 a 2.2 m ≈ 0.2·c·… usa 0.45·size)
            if dx * dx + dy * dy > (0.30 * size as f32).powi(2) || (dx > 0.0) != right {
                continue;
            }
            let i = ((y * size + x) * 4) as usize;
            sum += raw[i] as f64 * 0.2126 + raw[i + 1] as f64 * 0.7152 + raw[i + 2] as f64 * 0.0722;
            n += 1;
        }
    }
    sum / n.max(1) as f64
}

#[test]
fn face_sdf_without_a_map_does_not_change_the_image() {
    let Some(renderer) = renderer_or_skip("face_off") else { return };
    // sem mapa ancorado (neutro 1x1 branco) o fator é 0 mesmo com o flag ligado
    let flagged = render(&renderer, &face_head(60.0, 0.25));
    let mut off = face_head(60.0, 0.25);
    off.nodes[0].material.as_mut().unwrap().face_sdf_enabled = false;
    assert_eq!(flagged, render(&renderer, &off), "neutro 1x1: face SDF ligada não muda um pixel");
}

#[test]
fn face_sdf_mirror_puts_the_shadow_on_the_correct_side_for_both_light_directions() {
    let Some(renderer) = renderer_or_skip("face_mirror") else { return };
    install_face_map(&renderer);
    let size = SIZE;
    let frontal = render(&renderer, &face_head(0.0, 0.25));
    let from_right = render(&renderer, &face_head(90.0, 0.25));
    let from_left_mirrored = render(&renderer, &face_head(-90.0, 0.25));
    let from_left_plain = render(&renderer, &face_head(-90.0, 0.0));

    // luz em +X: lado esquerdo da tela (−X) fica na sombra, o direito (lado da luz) não
    assert!(half_luma(&from_right, size, false) < half_luma(&from_right, size, true) - 25.0);
    // luz em −X COM espelhamento: a sombra passa para o lado direito da tela
    assert!(half_luma(&from_left_mirrored, size, true) < half_luma(&from_left_mirrored, size, false) - 25.0);
    // luz em −X SEM espelhamento: a sombra continua à esquerda, do lado da luz (errado):
    // o espelhamento é o que corrige
    assert!(
        half_luma(&from_left_plain, size, false) < half_luma(&from_left_mirrored, size, false) - 20.0,
        "sem espelho o lado da luz fica sombreado"
    );
    // simetria: a imagem espelhada da luz em +X ≈ a da luz em −X com espelho
    let a = half_luma(&from_right, size, false);
    let b = half_luma(&from_left_mirrored, size, true);
    assert!((a - b).abs() < 6.0, "luma do lado sombreado: {a} vs {b}");
    // luz frontal: quase sem sombra facial (limiar mínimo 0.5; só a mancha do nariz)
    assert!(half_luma(&frontal, size, false) > 190.0 && half_luma(&frontal, size, true) > 190.0);
}

#[test]
fn face_frame_follows_the_head_bone_palette() {
    let Some(renderer) = renderer_or_skip("face_head_bone") else { return };
    install_face_map(&renderer);
    let straight = face_head(0.0, 0.25);
    let mut turned = face_head(0.0, 0.25);
    // osso 7 (cabeça) girado 90° em Y: a luz frontal do mundo vira lateral (−X local)
    let head = anigo_core::npr::FACE_HEAD_JOINT * 16;
    turned.skin.palette[head..head + 16]
        .copy_from_slice(&glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2).to_cols_array());
    let straight = render(&renderer, &straight);
    let turned = render(&renderer, &turned);
    // a esfera está na junta 0 (identidade): só o frame do rosto mudou. Com theta = −90°
    // e espelhamento, a sombra aparece à direita da tela
    assert!(half_luma(&turned, SIZE, true) < half_luma(&straight, SIZE, true) - 25.0);
    assert!((half_luma(&turned, SIZE, false) - half_luma(&straight, SIZE, false)).abs() < 6.0);
}
