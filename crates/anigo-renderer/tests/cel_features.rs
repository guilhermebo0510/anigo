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
