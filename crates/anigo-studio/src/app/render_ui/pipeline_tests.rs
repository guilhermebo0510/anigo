//! Testes do workspace Render que não precisam de janela: a cena de render sai do projeto (mesma geometria do Cenário),
//! o gesto de edição vira UM passo de desfazer e a câmera de cena enquadra igual no render real.

use super::*;
use crate::render_out::job::{render_frame, RenderError};
use crate::render_out::settings::{allocate, plan};
use crate::scene::geometry::{LoadedGeometry, MeshCache};
use crate::scene::runtime::SceneRuntime;
use anigo_core::command::Command;
use anigo_core::level::{self, PrimitiveShape};
use anigo_core::{AovSelection, ColorSpace, NodeSource, Transform};
use anigo_renderer::HeadlessRenderer;
use anigo_session::CoreSession;
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

/// Sessão com uma esfera em (0,1,0) e uma câmera de cena apontada para ela; devolve também o runtime resolvido.
fn session_with_scene() -> (CoreSession, SceneRuntime) {
    let mut core = CoreSession::new();
    let (add, _) = level::plan_place_primitive(core.project(), PrimitiveShape::Sphere, "Bola", Transform { translation: Vec3::new(0.0, 1.0, 0.0), scale: Vec3::splat(1.2), ..Default::default() }, &BTreeSet::new());
    core.apply(add).unwrap();
    let rot = Quat::from_euler(glam::EulerRot::YXZ, 0.35, -0.12, 0.0);
    let (cam_add, cam) = level::plan_place_camera(
        core.project(),
        "Camera A",
        Transform { translation: Vec3::new(2.0, 1.8, 5.0), rotation: rot, scale: Vec3::ONE },
        NodeSource::Camera { focal_mm: 50.0, sensor_width_mm: 24.89, sensor_height_mm: 18.66 },
        &BTreeSet::new(),
    );
    core.apply(cam_add).unwrap();
    core.apply(level::camera_command_from_node(core.project(), &cam).unwrap()).unwrap();
    let mut cache = MeshCache::new();
    let mut runtime = SceneRuntime::default();
    let ch = core.deformed_mesh().ok().and_then(|m| LoadedGeometry::new("char", m));
    runtime.rebuild(core.project(), &mut cache, None, ch.as_ref());
    (core, runtime)
}

#[test]
fn render_scene_comes_from_the_project_with_its_camera_light_and_background() {
    let (core, runtime) = session_with_scene();
    let (scene, notes) = scene_from_project(core.project(), &runtime, false).unwrap();
    assert!(notes.is_empty());
    assert_eq!(scene.camera, core.project().scene.camera.camera);
    assert_eq!(scene.background_color, core.project().render.background_color);
    let ball = scene.nodes.iter().find(|n| n.name == "Bola").expect("esfera");
    assert!(ball.mesh.is_some() && ball.visible);
    assert!(scene.nodes.iter().any(|n| n.name == "Camera A" && n.mesh.is_none()), "câmera de cena não tem malha");
    // enquanto algo carrega, o render é recusado com mensagem
    assert!(scene_from_project(core.project(), &runtime, true).is_err());
}

#[test]
fn hidden_nodes_are_left_out_and_an_empty_scene_is_refused() {
    let (mut core, mut runtime) = session_with_scene();
    let ids: Vec<_> = core.project().scene.nodes.iter().map(|n| n.node_id.clone()).collect();
    core.apply(level::plan_set_visible(core.project(), &ids, false).unwrap()).unwrap();
    let mut cache = MeshCache::new();
    runtime.rebuild(core.project(), &mut cache, None, None);
    assert!(scene_from_project(core.project(), &runtime, false).is_err(), "nada visível para renderizar");
}

#[test]
fn a_live_edit_gesture_is_one_undo_step_and_undo_redo_restore_everything() {
    let (mut core, _) = session_with_scene();
    let before = core.project().clone();
    let depth = core.history().undo_depth();
    // arrastar a largura: vários previews, um commit
    let mut last = None;
    for w in [1900u32, 1800, 1700, 1600] {
        let cmd = Command::SetRenderOutput { patch: RenderOutputPatch { width: Some(w), height: Some(w * 9 / 16), ..Default::default() } };
        core.preview(cmd.clone()).unwrap();
        last = Some(cmd);
    }
    assert_eq!(core.history().undo_depth(), depth, "o preview não entra no histórico");
    core.commit_preview(last.unwrap()).unwrap();
    assert_eq!(core.history().undo_depth(), depth + 1, "UM passo de desfazer");
    let after = core.project().clone();
    assert_eq!((after.render.output.width, after.render.output.height), (1600, 900));
    assert!((after.scene.camera.camera.aspect - 1600.0 / 900.0).abs() < 1e-5);
    core.undo().unwrap();
    assert_eq!(*core.project(), before);
    core.redo().unwrap();
    assert_eq!(*core.project(), after);
    // vários campos de uma vez também são um passo só
    let batch = Command::Batch {
        commands: vec![
            Command::SetRenderOutput { patch: RenderOutputPatch { transparent: Some(true), supersample: Some(3), display_space: Some(ColorSpace::DisplayP3), ..Default::default() } },
            Command::SetRenderSettings { msaa_samples: None, tonemap: None, graph_order: None, graph_disabled: Some(vec!["outline".into()]), depth_prepass: None },
        ],
    };
    let d = core.history().undo_depth();
    core.apply(batch).unwrap();
    assert_eq!(core.history().undo_depth(), d + 1);
    assert_eq!(core.project().render.graph_disabled, vec!["outline".to_string()]);
    core.undo().unwrap();
    assert!(core.project().render.graph_disabled.is_empty() && !core.project().render.output.transparent);
}

#[test]
fn the_real_render_frames_the_scene_like_the_render_camera_projects_it() {
    let Ok(mut r) = pollster::block_on(HeadlessRenderer::new()) else {
        eprintln!("sem adaptador wgpu: teste pulado");
        return;
    };
    let (mut core, runtime) = session_with_scene();
    core.apply(Command::SetRenderOutput { patch: RenderOutputPatch { width: Some(480), height: Some(270), transparent: Some(true), aov: Some(AovSelection { mask: true, ..Default::default() }), ..Default::default() } }).unwrap();
    let (scene, _) = scene_from_project(core.project(), &runtime, false).unwrap();
    // esconde o manequim: só a esfera aparece
    let mut scene = scene;
    for n in scene.nodes.iter_mut() {
        if n.name != "Bola" {
            n.visible = false;
        }
    }
    let spec = crate::render_out::job::RenderSpec {
        output: core.project().render.output,
        display_space: core.project().render.color.display_space,
        bg: core.project().render.background_color,
        graph: anigo_renderer::GraphOverrides::default(),
        meta: Default::default(),
    };
    let p = plan(&spec.output, 8192, crate::render_out::settings::DEFAULT_MAX_PIXELS).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let alloc = allocate(dir.path(), "f", &Default::default(), "png", &["mask"], true, 1);
    let out = render_frame(&mut r, scene, &spec, &p, &alloc, None, &mut |_, _| true).map_err(|e: RenderError| format!("{e:?}")).unwrap();
    assert_eq!(out.files.len(), 2);
    // o centro da esfera, projetado pela câmera do projeto, cai no centro de massa da máscara
    let cam = &core.project().scene.camera.camera;
    let clip = cam.build_view_projection_matrix() * glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
    let (ex, ey) = ((clip.x / clip.w * 0.5 + 0.5) * 480.0, (1.0 - (clip.y / clip.w * 0.5 + 0.5)) * 270.0);
    let mask = image::open(dir.path().join("f_mask.png")).unwrap().to_luma8();
    let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y, px) in mask.enumerate_pixels() {
        let w = px.0[0] as f64 / 255.0;
        sx += x as f64 * w;
        sy += y as f64 * w;
        n += w;
    }
    assert!(n > 300.0, "esfera ausente na máscara ({n})");
    let (cx, cy) = ((sx / n) as f32, (sy / n) as f32);
    assert!((cx - ex).abs() < 4.0 && (cy - ey).abs() < 4.0, "máscara em ({cx:.1},{cy:.1}); a câmera do projeto projeta a esfera em ({ex:.1},{ey:.1})");
    // e a imagem principal tem alfa na mesma região
    let beauty = image::open(dir.path().join("f.png")).unwrap().to_rgba8();
    assert!(beauty.get_pixel(ex as u32, ey as u32).0[3] > 200);
    assert_eq!(beauty.get_pixel(2, 2).0[3], 0);
}
