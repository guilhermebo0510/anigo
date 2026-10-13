//! Testes visuais (renderer cel canônico em software): cada controle do painel Shading precisa
//! MUDAR a imagem da shader ball. Com `ANIGO_TEST_OUT=<pasta>` salva folhas de contato (par
//! padrão | alterado, um par por parâmetro) para inspeção humana.

use super::ball::{ball_mesh, LightRig};
use super::params::{self, alt_value, Group, Kind, Param, PARAMS};
use super::presets;
use anigo_core::scene::{Scene, SceneNode};
use anigo_core::StylizedMaterial;
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;
use image::RgbaImage;

const SIZE: u32 = 160;

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU ({e})");
            None
        }
    }
}

/// Bola vista de frente com a luz em coordenadas do mundo (az 0 = +Z, positivo para +X).
fn ball_scene(m: &StylizedMaterial, rig: LightRig) -> Scene {
    let mut scene = Scene::new_empty();
    scene.background_color = [0.20, 0.21, 0.25, 1.0];
    let mut node = SceneNode::new("ball", "ball");
    node.mesh = Some((*ball_mesh()).clone());
    node.material = Some(m.clone());
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 0.0, 2.3);
    scene.camera.target = Vec3::ZERO;
    scene.camera.fov_y = 30f32.to_radians();
    scene.light.direction = rig.direction(&crate::viewport::OrbitCamera::default()).to_array();
    scene
}

fn world_light(az: f32, el: f32) -> LightRig {
    LightRig { azimuth_deg: az, elevation_deg: el, follows_camera: false }
}

fn render(r: &HeadlessRenderer, m: &StylizedMaterial, rig: LightRig) -> RgbaImage {
    let (img, _) = pollster::block_on(r.render_scene(&ball_scene(m, rig), SIZE, SIZE)).expect("render");
    img
}

fn differing(a: &RgbaImage, b: &RgbaImage) -> usize {
    a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count()
}

fn install_face_map(r: &HeadlessRenderer) {
    let sdf = anigo_renderer::face_sdf_map::procedural_face_sdf_f32(256, 128, 0.25, 0.25, true);
    r.set_face_sdf_map_f32(256, 128, &sdf).unwrap();
}

/// Material de partida do teste: recursos visíveis ligados para que os gates dos parâmetros não
/// escondam o efeito (cada parâmetro é testado a partir daqui, com os seus gates acesos).
fn probe_base() -> StylizedMaterial {
    let mut m = mid_tone();
    m.spec_intensity = 0.8;
    m.rim_intensity = 1.2;
    m.outline_width = 0.014;
    m
}

/// Cor de meio-tom (o padrão claro "pele" estoura em branco e esconde sombra/rim nas folhas de contato).
fn mid_tone() -> StylizedMaterial {
    StylizedMaterial {
        base_color: [0.42, 0.60, 0.86, 1.0],
        shade_color: [0.18, 0.22, 0.46, 1.0],
        rim_color: [1.0, 0.85, 0.6, 1.0],
        outline_color: [0.05, 0.05, 0.12, 1.0],
        ..Default::default()
    }
}

/// Acende (com o valor alternativo) a cadeia de gates de `p`.
fn open_gates(m: &mut StylizedMaterial, p: &Param) {
    let mut cur = p.gate;
    while let Some(g) = cur.and_then(params::find) {
        g.write(m, alt_value(g));
        cur = g.gate;
    }
}

/// Parâmetros que NÃO alteram a shader ball por limitação conhecida do renderer (documentada na UI).
/// A razão fica aqui para o teste falhar se algum deles passar a ter efeito (ou se aparecer outro sem).
fn known_invisible(id: &str) -> Option<&'static str> {
    match id {
        "mtoon_second_shade_shift" | "mtoon_second_shade_softness" => Some("a 2ª sombra só entra com a textura da 2ª sombra (slot MToon sem textura no renderer)"),
        "ao_intensity" => Some("o AO vem do canal R da cor de vértice; a esfera tem R = 1"),
        "outline_depth_bias" => Some("o bias de profundidade do contorno só desloca contra a própria malha; na esfera isolada não aparece"),
        "mtoon_shade_toony" => Some("quantiza a AMOSTRA da textura de sombra (slot MToon sem textura no renderer)"),
        "eye_enabled" | "eye_depth_scale" => Some("o olho anime só desloca o UV de uma textura de íris; sem textura só os highlights aparecem"),
        _ => None,
    }
}

#[test]
fn every_panel_control_changes_the_ball_image_or_is_a_documented_limitation() {
    let Some(r) = renderer_or_skip("params_visual") else { return };
    install_face_map(&r);
    let out = std::env::var("ANIGO_TEST_OUT").ok();
    let rig = world_light(-45.0, 30.0);
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for g in Group::ALL {
        let mut tiles: Vec<RgbaImage> = Vec::new();
        for p in params::of_group(g) {
            let mut m0 = probe_base();
            open_gates(&mut m0, p);
            // face SDF: luz lateral para a sombra do mapa aparecer (espelho: do lado oposto)
            let mut light = if p.group == Group::Face { world_light(80.0, 0.0) } else { rig };
            if p.group == Group::Face {
                m0.shadow_threshold = 0.2;
                m0.spec_intensity = 0.0;
                m0.rim_intensity = 0.0;
                m0.face_sdf_enabled = p.id != "face_sdf_enabled";
                if p.id == "face_sdf_mirror_center" {
                    light = world_light(-80.0, 0.0);
                }
            }
            match p.id {
                // a emissão precisa de cor; o piso do falloff só age com a câmera além da distância de referência
                "mtoon_emission_intensity" => m0.mtoon_emission_color = [1.0, 0.3, 0.3, 1.0],
                "outline_min_width_scale" => m0.npr.outline_ref_distance = 0.4,
                "eye_highlight_intensity" => m0.eye_enabled = true,
                _ => {}
            }
            let mut m1 = m0.clone();
            p.write(&mut m1, alt_value(p));
            let (a, b) = (render(&r, &m0, light), render(&r, &m1, light));
            let n = differing(&a, &b);
            report.push(format!("{:>28} {:>6} px", p.id, n));
            match (known_invisible(p.id), n) {
                (None, 0) => failures.push(format!("{} não alterou a imagem", p.id)),
                (Some(why), n) if n > 0 => failures.push(format!("{} agora altera a imagem ({n} px) — remova de known_invisible: {why}", p.id)),
                _ => {}
            }
            tiles.push(a);
            tiles.push(b);
        }
        if let Some(dir) = &out {
            let _ = std::fs::create_dir_all(dir);
            sheet(&tiles, 6).save(format!("{dir}/shading_params_{}.png", g.id())).unwrap();
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Grade `cols` colunas de tiles.
fn sheet(tiles: &[RgbaImage], cols: usize) -> RgbaImage {
    let rows = tiles.len().div_ceil(cols).max(1);
    let mut out = RgbaImage::new(cols as u32 * SIZE, rows as u32 * SIZE);
    for (i, t) in tiles.iter().enumerate() {
        let (cx, cy) = ((i % cols) as u32, (i / cols) as u32);
        image::imageops::replace(&mut out, t, (cx * SIZE) as i64, (cy * SIZE) as i64);
    }
    out
}

#[test]
fn custom_ramp_stops_reshape_the_terminator_and_the_sheet_is_saved() {
    let Some(r) = renderer_or_skip("ramp_visual") else { return };
    let rig = world_light(-50.0, 25.0);
    let base = StylizedMaterial { rim_intensity: 0.0, spec_intensity: 0.0, outline_width: 0.0, ..mid_tone() };
    let mut tiles = vec![render(&r, &base, rig)];
    // contrato: linhas 0..3
    for row in 0..4 {
        let mut m = base.clone();
        m.npr.ramp_blend = 1.0;
        m.npr.ramp_row = row as f32;
        tiles.push(render(&r, &m, rig));
    }
    // gradientes próprios na linha 2
    let variants: [(bool, &[(f32, f32)]); 4] = [
        (true, &[(0.0, 0.0), (0.7, 1.0)]),
        (true, &[(0.0, 0.0), (0.25, 0.3), (0.5, 0.6), (0.75, 1.0)]),
        (false, &[(0.2, 0.0), (0.8, 1.0)]),
        (false, &[(0.0, 0.0), (0.45, 0.1), (0.55, 0.9), (1.0, 1.0)]),
    ];
    let mut prev: Option<RgbaImage> = None;
    for (constant, stops) in variants {
        let mut m = base.clone();
        m.npr.ramp_blend = 1.0;
        m.npr.ramp_row = 2.0;
        m.npr.ramp_custom.set_stops(2, constant, stops);
        let img = render(&r, &m, rig);
        if let Some(p) = &prev {
            assert!(differing(p, &img) > 300, "cada gradiente deveria desenhar um terminador diferente");
        }
        prev = Some(img.clone());
        tiles.push(img);
    }
    if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        sheet(&tiles, 5).save(format!("{dir}/shading_ramp_custom.png")).unwrap();
    }
}

#[test]
fn face_sdf_shadow_moves_with_the_light_and_mirrors_for_the_other_side() {
    let Some(r) = renderer_or_skip("face_visual") else { return };
    install_face_map(&r);
    let mut m = StylizedMaterial { shadow_threshold: 0.2, spec_intensity: 0.0, rim_intensity: 0.0, outline_width: 0.0, face_sdf_enabled: true, ..mid_tone() };
    m.face_shadow_smoothness = 0.04;
    let mut tiles = Vec::new();
    let mut mirrored_tiles = Vec::new();
    let angles = [0.0, 30.0, 60.0, 90.0, 135.0, -45.0, -90.0];
    for a in angles {
        let mut plain = m.clone();
        plain.npr.face_sdf_mirror_center = 0.0;
        tiles.push(render(&r, &plain, world_light(a, 0.0)));
        let mut mir = m.clone();
        mir.npr.face_sdf_mirror_center = 0.25;
        mirrored_tiles.push(render(&r, &mir, world_light(a, 0.0)));
    }
    // sem espelho, -90 repete a sombra do lado da luz; com espelho, as imagens de +90 e -90 são espelhadas
    let p90 = &mirrored_tiles[3];
    let m90 = &mirrored_tiles[6];
    let mut flipped = m90.clone();
    image::imageops::flip_horizontal_in_place(&mut flipped);
    // espelhar a luz espelha a imagem (a esfera é quase simétrica): poucos pixels de diferença
    let d = differing(p90, &flipped);
    assert!(d < (SIZE * SIZE / 5) as usize, "espelho: +90 e -90 deveriam ser quase espelhados ({d} px)");
    assert!(differing(&tiles[0], &tiles[3]) > 500, "a luz lateral muda a sombra do rosto");
    tiles.extend(mirrored_tiles);
    if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        sheet(&tiles, angles.len()).save(format!("{dir}/shading_face_sdf.png")).unwrap();
    }
}

#[test]
fn outline_controls_sheet_and_rim_sheet() {
    let Some(r) = renderer_or_skip("outline_visual") else { return };
    let rig = world_light(-40.0, 30.0);
    let mut tiles = Vec::new();
    let base = StylizedMaterial { outline_width: 0.02, rim_intensity: 0.0, ..mid_tone() };
    tiles.push(render(&r, &base, rig));
    for (id, v) in [("outline_taper", 1.0), ("outline_ink_blend", 1.0), ("outline_smooth_normals", 1.0), ("outline_opacity", 0.4)] {
        let mut m = base.clone();
        if id == "outline_ink_blend" {
            m.npr.outline_ink_saturation = 3.0;
        }
        let p = params::find(id).unwrap();
        p.write(&mut m, params::Value::F(v));
        tiles.push(render(&r, &m, rig));
    }
    // rim: intensidade e propagação
    let rim = StylizedMaterial { rim_intensity: 0.0, outline_width: 0.0, ..mid_tone() };
    for (i, s) in [(0.0, 0.4), (1.0, 0.2), (1.5, 0.5), (2.5, 0.9)] {
        let mut m = rim.clone();
        m.rim_intensity = i;
        m.rim_spread = s;
        tiles.push(render(&r, &m, rig));
    }
    if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        sheet(&tiles, 5).save(format!("{dir}/shading_outline_rim.png")).unwrap();
    }
    assert!(differing(&tiles[0], &tiles[2]) > 100, "ink muda a cor da linha");
    assert!(differing(&tiles[5], &tiles[7]) > 200, "rim muda a borda");
}

#[test]
fn built_in_presets_render_distinct_balls() {
    let Some(r) = renderer_or_skip("presets_visual") else { return };
    let rig = world_light(-40.0, 30.0);
    let tiles: Vec<RgbaImage> = presets::BUILT_IN.iter().map(|p| render(&r, &(p.build)(), rig)).collect();
    for i in 0..tiles.len() {
        for j in i + 1..tiles.len() {
            assert!(differing(&tiles[i], &tiles[j]) > 200, "presets {i} e {j} parecem iguais");
        }
    }
    if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        sheet(&tiles, 4).save(format!("{dir}/shading_presets.png")).unwrap();
    }
}

#[test]
fn param_table_float_kinds_have_sane_ranges() {
    for p in PARAMS {
        if let Kind::Float { min, max, .. } = p.kind {
            assert!(max > min, "{}", p.id);
        }
    }
}
