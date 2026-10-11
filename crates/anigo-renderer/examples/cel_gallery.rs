//! Galeria de referência do motor cel: renderiza cenas determinísticas para PNG
//! (Vulkan por software/lavapipe ou qualquer adaptador disponível).
//!
//! Uso: `cargo run -p anigo-renderer --example cel_gallery -- <pasta_saida> [cena ...]`
//! Sem nomes de cena, renderiza todas. Veja `build` para o catálogo.

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::scene::{Scene, SceneNode};
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

fn build(name: &str) -> Option<(Scene, u32)> {
    Some(match name {
        "female_base" => (female_base(), 512),
        "dof_off" => (dof_depth_scene(false, 4.0), 512),
        "dof_focus_char" => (dof_depth_scene(true, 4.0), 512),
        "dof_focus_fg" => (dof_depth_scene(true, 2.0), 512),
        _ => return None,
    })
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
