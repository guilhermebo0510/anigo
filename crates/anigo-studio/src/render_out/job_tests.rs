//! Testes do pipeline de render e da fila (precisam de um adaptador wgpu: lavapipe serve; sem adaptador, pulam).

use super::job::*;
use super::png_out::read_info;
use super::settings::{allocate, plan, NameTokens, DEFAULT_MAX_PIXELS};
use super::history;
use crate::scene::geometry::primitive_mesh;
use anigo_core::scene::SceneNode;
use anigo_core::{ColorSpace, OutputFormat, PrimitiveShape, RenderOutput, Scene, StylizedMaterial, Transform};
use anigo_renderer::{GraphOverrides, HeadlessRenderer};
use glam::Vec3;
use png::{BitDepth, ColorType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Esfera de 1,4 m em (0,1,0) + caixa à direita; câmera de 3/4 olhando a esfera.
pub(crate) fn test_scene() -> Scene {
    let mut s = Scene::new_empty();
    let mut ball = SceneNode::new("ball", "Bola");
    ball.mesh = Some(primitive_mesh(PrimitiveShape::Sphere));
    ball.transform = Transform { translation: Vec3::new(0.0, 1.0, 0.0), scale: Vec3::splat(1.4), ..Default::default() };
    ball.material = Some(StylizedMaterial {
        base_color: [0.9, 0.5, 0.4, 1.0],
        shade_color: [0.5, 0.3, 0.35, 1.0],
        outline_color: [0.1, 0.05, 0.1, 1.0],
        outline_width: 0.02,
        ..Default::default()
    });
    s.add_node(ball);
    let mut cube = SceneNode::new("box", "Caixa");
    cube.mesh = Some(primitive_mesh(PrimitiveShape::Cube));
    cube.transform = Transform { translation: Vec3::new(1.6, 0.5, -0.5), ..Default::default() };
    s.add_node(cube);
    s.camera.eye = Vec3::new(0.8, 1.6, 5.0);
    s.camera.target = Vec3::new(0.3, 1.0, 0.0);
    s.camera.fov_y = 35f32.to_radians();
    s
}

pub(crate) fn test_spec(w: u32, h: u32) -> RenderSpec {
    RenderSpec {
        output: RenderOutput { width: w, height: h, ..Default::default() },
        display_space: ColorSpace::Srgb,
        bg: [0.2, 0.3, 0.5, 1.0],
        graph: GraphOverrides::default(),
        meta: Meta { project_id: "prj_test".into(), project_name: "Projeto".into(), fingerprint: 0xdead_beef, schema_version: 1, camera_name: "Cam".into() },
    }
}

fn gpu() -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(_) => {
            eprintln!("sem adaptador wgpu: teste pulado");
            None
        }
    }
}

fn run(r: &mut HeadlessRenderer, spec: &RenderSpec, dir: &std::path::Path, name: &str) -> FrameOutput {
    let plan = plan(&spec.output, 8192, DEFAULT_MAX_PIXELS).unwrap();
    let aovs = enabled_aovs(&spec.output);
    let alloc = allocate(dir, name, &NameTokens { project: "p".into(), ..Default::default() }, spec.output.format.extension(), &aovs, true, 1);
    let out = render_frame(r, test_scene(), spec, &plan, &alloc, None, &mut |_, _| true).unwrap();
    if let Ok(keep) = std::env::var("ANIGO_TEST_OUT") {
        // inspeção visual: copia as saídas para `$ANIGO_TEST_OUT/render_out/<pasta>_<arquivo>`
        let tag = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let dest = std::path::Path::new(&keep).join("render_out");
        let _ = std::fs::create_dir_all(&dest);
        for f in &out.files {
            if let Err(e) = std::fs::copy(&f.path, dest.join(format!("{tag}_{}", f.path.file_name().unwrap().to_string_lossy()))) { eprintln!("copia: {e}"); }
        }
    }
    out
}

#[test]
fn same_project_gives_the_same_bytes_and_correct_framing() {
    let Some(mut r) = gpu() else { return };
    let mut spec = test_spec(320, 180);
    spec.output.transparent = true;
    spec.output.supersample = 2;
    spec.output.dpi = 300;
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    run(&mut r, &spec, a.path(), "x");
    run(&mut r, &spec, b.path(), "x");
    let (ba, bb) = (std::fs::read(a.path().join("x.png")).unwrap(), std::fs::read(b.path().join("x.png")).unwrap());
    assert_eq!(ba, bb, "mesmo projeto, mesmos bytes");
    let info = read_info(&ba).unwrap();
    assert_eq!((info.w, info.h, info.dpi), (320, 180, Some(300)));
    assert_eq!(info.color_type, Some(ColorType::Rgba));
    assert!(info.srgb);
    let text = |k: &str| info.texts.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(text("anigo:project_fingerprint").as_deref(), Some("00000000deadbeef"));
    assert_eq!(text("anigo:pass").as_deref(), Some("beauty"));
    assert!(text("anigo:camera").unwrap().contains("\"fov_y_deg\""));
    // alfa: cantos transparentes
    let img = image::load_from_memory(&ba).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0[3], 0);
    assert_eq!(img.get_pixel(319, 179).0[3], 0);
    // enquadramento: o centro de massa do alfa da esfera fica onde a câmera projeta (0,1,0)
    let mut cam = test_scene().camera;
    cam.aspect = 320.0 / 180.0;
    let clip = cam.build_view_projection_matrix() * glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
    let (ex, ey) = ((clip.x / clip.w * 0.5 + 0.5) * 320.0, (1.0 - (clip.y / clip.w * 0.5 + 0.5)) * 180.0);
    let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 200 && (x as f32) < ex + 40.0 {
            sx += x as f64;
            sy += y as f64;
            n += 1.0;
        }
    }
    assert!(n > 500.0, "esfera ausente: {n} pixels");
    let (cx, cy) = ((sx / n) as f32, (sy / n) as f32);
    assert!((cx - ex).abs() < 6.0 && (cy - ey).abs() < 6.0, "centro da esfera em ({cx:.1},{cy:.1}), esperado ({ex:.1},{ey:.1})");
}

#[test]
fn transparent_render_composited_over_the_background_matches_the_solid_render() {
    let Some(mut r) = gpu() else { return };
    let mut solid = test_spec(256, 144);
    solid.output.supersample = 2;
    let mut transp = solid.clone();
    transp.output.transparent = true;
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    run(&mut r, &solid, a.path(), "s");
    run(&mut r, &transp, b.path(), "t");
    let s = image::open(a.path().join("s.png")).unwrap().to_rgba8();
    let t = image::open(b.path().join("t.png")).unwrap().to_rgba8();
    assert_eq!(image::open(a.path().join("s.png")).unwrap().color(), image::ColorType::Rgb8, "fundo sólido grava RGB sem alfa");
    let bg = [0.2f32, 0.3, 0.5];
    let mut worst = 0.0f32;
    let mut partial = 0;
    for (p, q) in s.pixels().zip(t.pixels()) {
        let a = q.0[3] as f32 / 255.0;
        if a > 0.0 && a < 1.0 {
            partial += 1;
        }
        for c in 0..3 {
            let comp = q.0[c] as f32 / 255.0 * a + bg[c] * (1.0 - a);
            worst = worst.max((comp * 255.0 - p.0[c] as f32).abs());
        }
    }
    assert!(partial > 50, "devia haver bordas com alfa parcial: {partial}");
    assert!(worst < 4.0, "o alfa reto não recompõe o render sólido (pior diferença {worst} níveis)");
}

#[test]
fn aovs_have_the_documented_contents() {
    let Some(mut r) = gpu() else { return };
    let mut spec = test_spec(256, 144);
    spec.output.supersample = 2;
    spec.output.transparent = true;
    spec.output.format = OutputFormat::Png16;
    spec.output.aov = anigo_core::AovSelection { outline: true, depth: true, mask: true, object_id: true, normal: true };
    let dir = tempfile::tempdir().unwrap();
    let out = run(&mut r, &spec, dir.path(), "a");
    let passes: Vec<_> = out.files.iter().map(|f| f.pass.as_str()).collect();
    assert_eq!(passes, ["beauty", "outline", "depth", "mask", "id", "normal"]);
    for f in &out.files {
        assert!(f.path.is_file() && f.bytes > 0, "{}", f.pass);
        let info = read_info(&std::fs::read(&f.path).unwrap()).unwrap();
        assert_eq!((info.w, info.h), (256, 144), "{}", f.pass);
    }
    let beauty = read_info(&std::fs::read(&out.files[0].path).unwrap()).unwrap();
    assert_eq!(beauty.depth, Some(BitDepth::Sixteen));
    let outline = image::open(dir.path().join("a_outline.png")).unwrap().to_rgba8();
    let covered = outline.pixels().filter(|p| p.0[3] > 0).count();
    assert!(covered > 100 && covered < 256 * 144 / 6, "contorno com {covered} px");
    let mask = image::open(dir.path().join("a_mask.png")).unwrap().to_luma8();
    let mask_px = mask.pixels().filter(|p| p.0[0] > 0).count();
    assert!(mask_px > covered, "a máscara cobre a geometria inteira");
    let id = image::open(dir.path().join("a_id.png")).unwrap().to_rgba8();
    let colors: std::collections::BTreeSet<[u8; 3]> = id.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    assert_eq!(colors.len(), 2, "{colors:?}");
    assert_eq!(id.get_pixel(0, 0).0[3], 0);
    let id_info = read_info(&std::fs::read(dir.path().join("a_id.png")).unwrap()).unwrap();
    assert!(id_info.texts.iter().any(|(k, v)| k == "anigo:object_ids" && v.contains("\"ball\"") && v.contains("\"box\"")));
    let depth_info = read_info(&std::fs::read(dir.path().join("a_depth.png")).unwrap()).unwrap();
    let range = depth_info.texts.iter().find(|(k, _)| k == "anigo:depth_range_m").unwrap().1.clone();
    let (a, b) = range.split_once(',').unwrap();
    let (near, far): (f32, f32) = (a.parse().unwrap(), b.parse().unwrap());
    assert!(near > 2.0 && far > near && far < 10.0, "{range}");
    let normal = image::open(dir.path().join("a_normal.png")).unwrap().to_rgb8();
    let distinct: std::collections::BTreeSet<[u8; 3]> = normal.pixels().map(|p| p.0).collect();
    assert!(distinct.len() > 50, "{}", distinct.len());
}

#[test]
fn disabling_the_outline_pass_removes_the_line_from_the_beauty_image() {
    let Some(mut r) = gpu() else { return };
    let mut with = test_spec(200, 112);
    with.output.aov.outline = true;
    let mut without = with.clone();
    without.graph.disabled = vec!["outline".into()];
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    run(&mut r, &with, a.path(), "w");
    let out = run(&mut r, &without, b.path(), "w");
    assert_ne!(std::fs::read(a.path().join("w.png")).unwrap(), std::fs::read(b.path().join("w.png")).unwrap(), "o passe outline desligado muda a imagem");
    assert!(out.notes.iter().any(|n| n.contains("outline")), "o AOV de contorno avisa que foi ignorado: {:?}", out.notes);
    assert!(!b.path().join("w_outline.png").exists());
    assert!(a.path().join("w_outline.png").exists());
}

#[test]
fn cancelling_stops_the_frame_and_leaves_no_files() {
    let Some(mut r) = gpu() else { return };
    let spec = test_spec(160, 90);
    let p = plan(&spec.output, 8192, DEFAULT_MAX_PIXELS).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let alloc = allocate(dir.path(), "c", &NameTokens::default(), "png", &[], true, 1);
    let mut calls = 0;
    let res = render_frame(&mut r, test_scene(), &spec, &p, &alloc, None, &mut |_, _| {
        calls += 1;
        calls < 3
    });
    assert_eq!(res.err(), Some(RenderError::Cancelled));
    assert!(!alloc.main.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

struct Orbit(Scene);
impl FrameSource for Orbit {
    fn scene(&self, frame: Option<u32>) -> Result<Scene, String> {
        let mut s = self.0.clone();
        s.nodes[0].transform.translation.x = frame.unwrap_or(0) as f32 * 0.3;
        Ok(s)
    }
}

fn collect(svc: &RenderService, id: u64) -> Vec<Event> {
    let mut out = Vec::new();
    loop {
        let e = svc.events.recv_timeout(std::time::Duration::from_secs(120)).expect("evento");
        let done = matches!(&e, Event::Finished { id: i, .. } | Event::Failed { id: i, .. } | Event::Cancelled { id: i } if *i == id);
        out.push(e);
        if done {
            return out;
        }
    }
}

fn seq_job(dir: &std::path::Path, id: u64, skip: bool) -> JobSpec {
    JobSpec {
        id,
        label: "seq".into(),
        source: Arc::new(Orbit(test_scene())),
        spec: test_spec(128, 72),
        dir: dir.to_path_buf(),
        template: "seq_%04d".into(),
        overwrite: false,
        counter: 1,
        frames: Some(FrameRange { start: 1, end: 4, step: 1 }),
        skip_existing: skip,
        reuse: None,
        tokens: NameTokens { project: "p".into(), ..Default::default() },
    }
}

#[test]
fn the_queue_renders_a_sequence_cancels_and_resumes_skipping_finished_frames() {
    let dir = tempfile::tempdir().unwrap();
    let svc = RenderService::start(None);
    // 1) cancela depois do 2º quadro
    let cancel = Arc::new(AtomicBool::new(false));
    svc.submit(seq_job(dir.path(), 1, false), cancel.clone());
    let mut frames_done = 0;
    let mut ev = Vec::new();
    loop {
        let e = svc.events.recv_timeout(std::time::Duration::from_secs(120)).expect("evento");
        if let Event::Failed { error, .. } = &e {
            if error.contains("sem GPU") {
                eprintln!("sem GPU: teste pulado");
                return;
            }
        }
        if matches!(e, Event::FrameDone { .. }) {
            frames_done += 1;
            if frames_done == 2 {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        let end = matches!(e, Event::Finished { .. } | Event::Failed { .. } | Event::Cancelled { .. });
        ev.push(e);
        if end {
            break;
        }
    }
    assert!(matches!(ev.last(), Some(Event::Cancelled { id: 1 })), "{:?}", ev.last());
    let written: Vec<_> = (1..=4).filter(|f| dir.path().join(format!("seq_{f:04}.png")).is_file()).collect();
    assert_eq!(written, vec![1, 2], "só os quadros concluídos ficam no disco");
    let hist = history::load(dir.path());
    assert_eq!(hist.len(), 2);
    assert!(hist.iter().all(|h| h.thumb_path(dir.path()).is_some_and(|t| t.is_file())), "miniaturas gravadas");
    // 2) retomar: pula 1 e 2, renderiza 3 e 4
    let before = std::fs::read(dir.path().join("seq_0001.png")).unwrap();
    let mtime = std::fs::metadata(dir.path().join("seq_0001.png")).unwrap().modified().unwrap();
    svc.submit(seq_job(dir.path(), 2, true), Arc::new(AtomicBool::new(false)));
    let ev = collect(&svc, 2);
    let skipped = ev.iter().filter(|e| matches!(e, Event::FrameSkipped { .. })).count();
    let done = ev.iter().filter(|e| matches!(e, Event::FrameDone { .. })).count();
    assert_eq!((skipped, done), (2, 2), "{ev:?}");
    assert!(matches!(ev.last(), Some(Event::Finished { .. })));
    assert_eq!(std::fs::read(dir.path().join("seq_0001.png")).unwrap(), before);
    assert_eq!(std::fs::metadata(dir.path().join("seq_0001.png")).unwrap().modified().unwrap(), mtime, "quadro existente não foi regravado");
    assert!((1..=4).all(|f| dir.path().join(format!("seq_{f:04}.png")).is_file()));
    assert_ne!(std::fs::read(dir.path().join("seq_0001.png")).unwrap(), std::fs::read(dir.path().join("seq_0004.png")).unwrap(), "a bola anda entre os quadros");
}

#[test]
fn an_oversized_job_fails_cleanly_without_touching_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let svc = RenderService::start(None);
    let mut spec = test_spec(16384, 16384);
    spec.output.supersample = 1;
    svc.submit(
        JobSpec {
            id: 7,
            label: "grande".into(),
            source: Arc::new(StillSource(test_scene())),
            spec,
            dir: dir.path().to_path_buf(),
            template: "g".into(),
            overwrite: true,
            counter: 1,
            frames: None,
            skip_existing: false,
            reuse: None,
            tokens: NameTokens::default(),
        },
        Arc::new(AtomicBool::new(false)),
    );
    let ev = collect(&svc, 7);
    assert!(matches!(ev.last(), Some(Event::Failed { .. })), "{ev:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
