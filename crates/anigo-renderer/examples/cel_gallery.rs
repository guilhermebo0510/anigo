//! Galeria de referência do motor cel: renderiza cenas determinísticas para PNG
//! (Vulkan por software/lavapipe ou qualquer adaptador disponível).
//!
//! Uso: `cargo run -p anigo-renderer --example cel_gallery -- <pasta_saida> [cena ...]`
//! Sem nomes de cena, renderiza todas. Veja `build` para o catálogo.

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::scene::{Scene, SceneNode, StylizedMaterial};
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;

fn female_base() -> Scene {
    let mesh = Mesh::create_canonical_base(BaseGender::Female);
    let mut scene = Scene::new_empty();
    scene.add_node(SceneNode::new("base", "CanonicalBase").with_mesh(mesh));
    scene
}

fn cube_node(id: &str, size: f32, at: Vec3) -> SceneNode {
    let mut node = SceneNode::new(id, id).with_mesh(Mesh::create_cube(size));
    node.transform.translation = at;
    node
}

/// Personagem com cubos em três profundidades (para ver o DoF separar planos).
fn dof_depth_scene(enabled: bool, focus: f32) -> Scene {
    let mut scene = female_base();
    scene.add_node(cube_node("bg_near", 0.8, Vec3::new(-1.3, 0.5, -3.0)));
    scene.add_node(cube_node("bg_far", 1.2, Vec3::new(1.5, 0.7, -6.0)));
    scene.add_node(cube_node("fg", 0.5, Vec3::new(0.9, 0.4, 2.0)));
    scene.camera.eye = Vec3::new(0.0, 1.2, 4.0);
    scene.camera.target = Vec3::new(0.0, 0.9, 0.0);
    scene.dof.enabled = enabled;
    scene.dof.focus_distance = focus;
    scene.dof.f_number = 1.4;
    scene
}

/// Esfera de sonda de iluminação (câmera fechada, luz oblíqua).
fn probe_sphere(tweak: impl Fn(&mut StylizedMaterial)) -> Scene {
    let mut scene = Scene::new_empty();
    let mut node = SceneNode::new("probe", "Probe").with_mesh(Mesh::create_uv_sphere(0.5, 48, 64));
    let mut material = StylizedMaterial::default();
    tweak(&mut material);
    node.material = Some(material);
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 0.0, 2.2);
    scene.camera.target = Vec3::ZERO;
    scene.light.direction = Vec3::new(0.65, 0.45, 0.62).normalize().to_array();
    scene
}

fn ramp_scene(blend: f32, row: f32, steps: f32) -> Scene {
    probe_sphere(|m| {
        m.toon_steps = steps;
        m.npr.ramp_blend = blend;
        m.npr.ramp_row = row;
    })
}

/// Fundo claro para a linha de contorno aparecer (o padrão do contrato é escuro).
const LIGHT_BG: [f32; 4] = [0.86, 0.88, 0.93, 1.0];

/// Cubo girado, câmera fechada, contorno largo: a fresta nos cantos do hull só
/// aparece com normais da malha (arestas duras).
fn cube_hull_scene(smooth: f32) -> Scene {
    let mut scene = Scene::new_empty();
    let mut node = SceneNode::new("cube", "Cube").with_mesh(Mesh::create_cube(1.0));
    node.transform.rotation = glam::Quat::from_euler(glam::EulerRot::YXZ, 0.55, 0.45, 0.0);
    let mut material = StylizedMaterial::default();
    material.outline_width = 0.02;
    material.npr.outline_smooth_normals = smooth;
    node.material = Some(material);
    scene.add_node(node);
    scene.camera.eye = Vec3::new(0.0, 0.0, 3.2);
    scene.camera.target = Vec3::ZERO;
    scene.background_color = LIGHT_BG;
    scene
}

/// Base feminina a `distance` m da câmera, linha nominal 0.008 (~2 px em 512).
fn base_at_distance(distance: f32, tweak: impl Fn(&mut StylizedMaterial)) -> Scene {
    let mut scene = female_base();
    let mut material = StylizedMaterial::default();
    material.outline_width = 0.008;
    tweak(&mut material);
    scene.nodes[0].material = Some(material);
    scene.camera.eye = Vec3::new(0.0, 1.0, distance);
    scene.camera.target = Vec3::new(0.0, 0.9, 0.0);
    scene.background_color = LIGHT_BG;
    scene
}

fn build(name: &str) -> Option<(Scene, u32)> {
    Some(match name {
        "female_base" => (female_base(), 512),
        "dof_off" => (dof_depth_scene(false, 4.0), 512),
        "dof_focus_char" => (dof_depth_scene(true, 4.0), 512),
        "dof_focus_fg" => (dof_depth_scene(true, 2.0), 512),
        "ramp_off" => (ramp_scene(0.0, 1.0, 1.0), 256),
        "ramp_r0" => (ramp_scene(1.0, 0.0, 1.0), 256),
        "ramp_r1" => (ramp_scene(1.0, 1.0, 1.0), 256),
        "ramp_r2" => (ramp_scene(1.0, 2.0, 1.0), 256),
        "ramp_r3" => (ramp_scene(1.0, 3.0, 1.0), 256),
        "ramp_r1_5" => (ramp_scene(1.0, 1.5, 1.0), 256),
        "ramp_half" => (ramp_scene(0.5, 2.0, 1.0), 256),
        "cube_hull_mesh" => (cube_hull_scene(0.0), 384),
        "cube_hull_smooth" => (cube_hull_scene(1.0), 384),
        "dist2_const" => (base_at_distance(2.5, |_| {}), 512),
        "dist5_const" => (base_at_distance(5.0, |_| {}), 512),
        "dist10_const" => (base_at_distance(10.0, |_| {}), 512),
        "dist2_falloff" => (base_at_distance(2.5, falloff_on), 512),
        "dist5_falloff" => (base_at_distance(5.0, falloff_on), 512),
        "dist10_falloff" => (base_at_distance(10.0, falloff_on), 512),
        "taper0" => (base_at_distance(2.5, |m| m.npr.outline_smooth_normals = 1.0), 512),
        "taper1" => (base_at_distance(2.5, |m| {
            m.npr.outline_smooth_normals = 1.0;
            m.npr.outline_taper = 1.0;
        }), 512),
        "ink0" => (base_at_distance(2.5, |m| m.npr.outline_smooth_normals = 1.0), 512),
        "ink1" => (base_at_distance(2.5, |m| {
            m.npr.outline_smooth_normals = 1.0;
            m.npr.outline_ink_blend = 1.0;
        }), 512),
        _ => return None,
    })
}

fn falloff_on(m: &mut StylizedMaterial) {
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_distance_falloff = 1.0;
    m.npr.outline_ref_distance = 5.0;
    m.npr.outline_min_width_scale = 0.2;
}

const ALL: &[&str] = &["female_base", "dof_off", "dof_focus_char", "dof_focus_fg"];

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().expect("uso: cel_gallery <pasta_saida> [cena ...]");
    let mut names: Vec<String> = args.collect();
    if names.is_empty() {
        names = ALL.iter().map(|s| s.to_string()).collect();
    }
    std::fs::create_dir_all(&out_dir).unwrap();
    let renderer = pollster::block_on(HeadlessRenderer::new()).expect("sem adaptador GPU");
    for name in names {
        let Some((scene, size)) = build(&name) else {
            eprintln!("cena desconhecida: {name}");
            continue;
        };
        let (img, _) = pollster::block_on(renderer.render_scene(&scene, size, size)).unwrap();
        let path = format!("{out_dir}/{name}.png");
        img.save(&path).unwrap();
        println!("{path}");
    }
}
