//! Pipeline de um quadro e fila de render em thread dedicada.
//!
//! * [`render_frame`] — o caminho puro: cena → render a `N×` no `HeadlessRenderer` → redução por caixa (precisão 16 bits)
//!   → alfa reto → espaço de cor → PNG/JPEG com metadados; AOVs aditivos (contorno por diferença, profundidade/máscara/ID/normal
//!   por rasterizador de software). Cancelável entre as etapas.
//! * [`RenderService`] — uma thread com **device de GPU próprio** (não disputa o device do egui); jobs em série, progresso e
//!   cancelamento por eventos. O gancho da Animação é [`FrameSource`] + [`FrameRange`]: o nome do arquivo vem do quadro
//!   (`{frame}`/`%04d`) e "retomar" pula os quadros que já existem.

use super::aov;
use super::history::{self, HistoryEntry, THUMB_DIR, THUMB_SIDE};
use super::image_ops::FloatImage;
use super::png_out::{encode_jpeg, encode_png, write_atomic, PngColor, PngSpec};
use super::settings::{allocate, allocate_frame, plan, utc_date_time, Allocation, NameTokens, RenderPlan, DEFAULT_MAX_PIXELS};
use anigo_core::{ColorSpace, OutputFormat, RenderOutput, Scene};
use anigo_renderer::{GraphOverrides, HeadlessRenderer};
use crossbeam_channel::{Receiver, Sender};
use png::{BitDepth, ColorType};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Identificação do projeto e da câmera gravada nos metadados.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub project_id: String,
    pub project_name: String,
    pub fingerprint: u64,
    pub schema_version: u32,
    pub camera_name: String,
}

/// Tudo o que define a imagem de um quadro (além da cena).
#[derive(Debug, Clone)]
pub struct RenderSpec {
    pub output: RenderOutput,
    pub display_space: ColorSpace,
    /// Cor de fundo do projeto (sRGB de exibição, 0..1).
    pub bg: [f32; 4],
    /// Passes ligados/desligados e ordem (`RenderState.graph_*`).
    pub graph: GraphOverrides,
    pub meta: Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Prepare,
    Render,
    Outline,
    Reduce,
    Aov,
    Encode,
    Write,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    Cancelled,
    Failed(String),
}

impl From<String> for RenderError {
    fn from(s: String) -> Self {
        RenderError::Failed(s)
    }
}

#[derive(Debug, Clone)]
pub struct OutFile {
    pub pass: String,
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct FrameOutput {
    pub files: Vec<OutFile>,
    /// Miniatura RGBA8 reta da imagem principal.
    pub thumb: (u32, u32, Vec<u8>),
    pub notes: Vec<String>,
    pub render_ms: f64,
}

pub const AOV_OUTLINE: &str = "outline";
pub const AOV_DEPTH: &str = "depth";
pub const AOV_MASK: &str = "mask";
pub const AOV_ID: &str = "id";
pub const AOV_NORMAL: &str = "normal";

/// Nomes dos AOVs ligados, na ordem canônica.
pub fn enabled_aovs(o: &RenderOutput) -> Vec<&'static str> {
    let mut v = Vec::new();
    if o.aov.outline {
        v.push(AOV_OUTLINE);
    }
    if o.aov.depth {
        v.push(AOV_DEPTH);
    }
    if o.aov.mask {
        v.push(AOV_MASK);
    }
    if o.aov.object_id {
        v.push(AOV_ID);
    }
    if o.aov.normal {
        v.push(AOV_NORMAL);
    }
    v
}

fn texts_for(spec: &RenderSpec, plan: &RenderPlan, scene: &Scene, pass: &str, frame: Option<u32>, adapter: &str, extra: Vec<(String, String)>) -> Vec<(String, String)> {
    let o = &spec.output;
    let c = &scene.camera;
    let camera = serde_json::json!({
        "name": spec.meta.camera_name,
        "eye": c.eye.to_array(), "target": c.target.to_array(), "up": c.up.to_array(),
        "fov_y_deg": c.fov_y.to_degrees(), "near": c.z_near, "far": c.z_far, "aspect": plan.target.0 as f32 / plan.target.1 as f32,
        "orthographic": c.orthographic.is_some(),
    });
    let render = serde_json::json!({
        "width": plan.target.0, "height": plan.target.1, "scale_percent": o.scale_percent,
        "supersample_requested": plan.ss_requested, "supersample": plan.ss, "transparent": o.transparent,
        "format": o.format, "jpeg_quality": o.jpeg_quality, "dpi": o.dpi, "color_space": spec.display_space,
        "graph_disabled": spec.graph.disabled, "graph_order": spec.graph.order, "depth_prepass": spec.graph.depth_prepass,
        "dof": o.dof.enabled.then_some(o.dof), "msaa": 4,
    });
    let mut v = vec![
        ("Software".to_string(), format!("ANIGO Studio {}", env!("CARGO_PKG_VERSION"))),
        ("anigo:schema".to_string(), "1".to_string()),
        ("anigo:pass".to_string(), pass.to_string()),
        ("anigo:project_id".to_string(), spec.meta.project_id.clone()),
        ("anigo:project_name".to_string(), spec.meta.project_name.clone()),
        ("anigo:project_fingerprint".to_string(), format!("{:016x}", spec.meta.fingerprint)),
        ("anigo:project_schema".to_string(), spec.meta.schema_version.to_string()),
        ("anigo:camera".to_string(), camera.to_string()),
        ("anigo:render".to_string(), render.to_string()),
        ("anigo:renderer".to_string(), adapter.to_string()),
    ];
    if let Some(f) = frame {
        v.push(("anigo:frame".to_string(), f.to_string()));
    }
    v.extend(extra);
    v
}

fn png_color(space: ColorSpace) -> PngColor {
    if space == ColorSpace::DisplayP3 {
        PngColor::DisplayP3
    } else {
        PngColor::Srgb
    }
}

/// Codifica uma imagem de COR (pré-multiplicada) como PNG com alfa reto, no espaço de saída e na profundidade pedida.
fn encode_color_png(mut img: FloatImage, with_alpha: bool, sixteen: bool, space: ColorSpace, dpi: u32, texts: Vec<(String, String)>) -> Result<Vec<u8>, String> {
    img.unpremultiply();
    img.to_output_space(space);
    let (data, depth) = if sixteen { (img.encode16(with_alpha), BitDepth::Sixteen) } else { (img.encode8(with_alpha), BitDepth::Eight) };
    encode_png(&PngSpec { w: img.w, h: img.h, color_type: if with_alpha { ColorType::Rgba } else { ColorType::Rgb }, depth, data: &data, color: png_color(space), dpi: Some(dpi), texts })
}

fn out_file(pass: &str, path: &std::path::Path, w: u32, h: u32, bytes: &[u8]) -> Result<OutFile, String> {
    write_atomic(path, bytes)?;
    Ok(OutFile { pass: pass.to_string(), path: path.to_path_buf(), w, h, bytes: bytes.len() as u64 })
}

/// Renderiza UM quadro e grava os arquivos de `alloc`. `progress(etapa, fração 0..1)` devolve `false` para cancelar.
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    r: &mut HeadlessRenderer,
    mut scene: Scene,
    spec: &RenderSpec,
    plan: &RenderPlan,
    alloc: &Allocation,
    frame: Option<u32>,
    progress: &mut dyn FnMut(Stage, f32) -> bool,
) -> Result<FrameOutput, RenderError> {
    let start = std::time::Instant::now();
    let mut step = |stage: Stage, f: f32| -> Result<(), RenderError> {
        if progress(stage, f) {
            Ok(())
        } else {
            Err(RenderError::Cancelled)
        }
    };
    step(Stage::Prepare, 0.0)?;
    let o = &spec.output;
    let (tw, th) = plan.target;
    let (rw, rh) = plan.render_size;
    let ss = plan.ss;
    let adapter = format!("{} ({:?})", r.adapter_info.name, r.adapter_info.backend);
    let mut notes = Vec::new();
    if plan.clamped() {
        notes.push(format!("supersampling reduzido de {}× para {}× (limite da GPU/memória)", plan.ss_requested, plan.ss));
    }

    scene.camera.aspect = tw as f32 / th as f32;
    scene.background_color = if o.transparent { [0.0, 0.0, 0.0, 0.0] } else { [spec.bg[0], spec.bg[1], spec.bg[2], 1.0] };
    let dof_on = o.dof.enabled && !spec.graph.disabled.iter().any(|p| p == "dof_post");
    scene.dof = o.dof;
    scene.dof.enabled = dof_on;
    scene.dof.max_radius_px = o.dof.max_radius_px * ss as f32;
    r.set_graph_overrides(spec.graph.clone());

    step(Stage::Render, 0.05)?;
    let main = pollster::block_on(r.render_scene(&scene, rw, rh)).map_err(|e| RenderError::Failed(format!("render: {e:#}")))?.0;

    // ---- contorno isolado: diferença entre com e sem o passe `outline` (sem DoF nos dois)
    let mut outline_pm: Option<Vec<u8>> = None;
    if o.aov.outline {
        if spec.graph.disabled.iter().any(|p| p == "outline") {
            notes.push("AOV de contorno ignorado: o passe `outline` está desligado".to_string());
        } else {
            step(Stage::Outline, 0.55)?;
            scene.dof.enabled = false;
            let with = if dof_on { pollster::block_on(r.render_scene(&scene, rw, rh)).map_err(|e| RenderError::Failed(format!("render (contorno): {e:#}")))?.0 } else { main.clone() };
            let mut ov = spec.graph.clone();
            ov.disabled.push("outline".to_string());
            r.set_graph_overrides(ov);
            step(Stage::Outline, 0.62)?;
            let without = pollster::block_on(r.render_scene(&scene, rw, rh)).map_err(|e| RenderError::Failed(format!("render (sem contorno): {e:#}")))?.0;
            r.set_graph_overrides(spec.graph.clone());
            scene.dof.enabled = dof_on;
            outline_pm = Some(aov::outline_from_diff(with.as_raw(), without.as_raw()));
        }
    }
    let render_ms = start.elapsed().as_secs_f64() * 1000.0;

    // ---- redução (supersampling) e imagem principal
    step(Stage::Reduce, 0.7)?;
    let mut fi = FloatImage::from_rgba8_box(main.as_raw(), tw, th, ss);
    drop(main);
    if !o.transparent {
        for p in fi.px.chunks_exact_mut(4) {
            p[3] = 1.0;
        }
    }
    let thumb = fi.thumbnail_rgba8(THUMB_SIDE);
    step(Stage::Encode, 0.78)?;
    let sixteen = o.format == OutputFormat::Png16;
    let keep_alpha = o.transparent && o.format.has_alpha();
    let main_bytes = match o.format {
        OutputFormat::Jpeg => {
            if o.transparent {
                fi.flatten_over([spec.bg[0], spec.bg[1], spec.bg[2]]);
                notes.push("JPEG não tem alfa: o fundo transparente foi achatado sobre a cor de fundo".to_string());
            }
            fi.unpremultiply();
            fi.to_output_space(spec.display_space);
            if spec.display_space == ColorSpace::DisplayP3 {
                notes.push("JPEG sem perfil embutido: valores convertidos para Display P3 mas sem marcação".to_string());
            }
            encode_jpeg(tw, th, &fi.encode8(false), o.jpeg_quality)?
        }
        _ => {
            let texts = texts_for(spec, plan, &scene, "beauty", frame, &adapter, vec![]);
            encode_color_png(fi, keep_alpha, sixteen, spec.display_space, o.dpi, texts)?
        }
    };
    step(Stage::Write, 0.85)?;
    let mut files = vec![out_file("beauty", &alloc.main, tw, th, &main_bytes)?];

    // ---- AOVs aditivos
    let path_of = |pass: &str| alloc.aovs.iter().find(|(n, _)| n == pass).map(|(_, p)| p.clone());
    if let (Some(pm), Some(path)) = (outline_pm, path_of(AOV_OUTLINE)) {
        step(Stage::Aov, 0.88)?;
        let img = FloatImage::from_rgba8_box(&pm, tw, th, ss);
        let texts = texts_for(spec, plan, &scene, AOV_OUTLINE, frame, &adapter, vec![]);
        let bytes = encode_color_png(img, true, sixteen, spec.display_space, o.dpi, texts)?;
        files.push(out_file(AOV_OUTLINE, &path, tw, th, &bytes)?);
    }
    if o.aov.needs_raster() {
        step(Stage::Aov, 0.9)?;
        let lo = if o.aov.depth || o.aov.object_id || o.aov.normal || (o.aov.mask && ss == 1) { Some(aov::rasterize(&scene, tw, th)) } else { None };
        if let (true, Some(path), Some(rs)) = (o.aov.depth, path_of(AOV_DEPTH), lo.as_ref()) {
            let (data, range) = aov::depth_image(rs);
            let extra = match range {
                Some((n, f)) => vec![("anigo:depth_range_m".to_string(), format!("{n:.6},{f:.6}")), ("anigo:depth_encoding".to_string(), "gray16: branco = perto (near), preto = longe (far) ou sem geometria".to_string())],
                None => vec![("anigo:depth_encoding".to_string(), "sem geometria".to_string())],
            };
            let texts = texts_for(spec, plan, &scene, AOV_DEPTH, frame, &adapter, extra);
            let bytes = encode_png(&PngSpec { w: tw, h: th, color_type: ColorType::Grayscale, depth: BitDepth::Sixteen, data: &data, color: PngColor::Data, dpi: Some(o.dpi), texts })?;
            files.push(out_file(AOV_DEPTH, &path, tw, th, &bytes)?);
        }
        if let (true, Some(path)) = (o.aov.mask, path_of(AOV_MASK)) {
            let hi_owned;
            let (hi, k) = if ss > 1 {
                hi_owned = aov::rasterize(&scene, rw, rh);
                (&hi_owned, ss)
            } else {
                (lo.as_ref().ok_or_else(|| "máscara: rasterização ausente".to_string())?, 1)
            };
            let (mw, mh, data) = aov::mask_image(hi, k);
            let texts = texts_for(spec, plan, &scene, AOV_MASK, frame, &adapter, vec![("anigo:mask_encoding".to_string(), "cobertura da geometria (sem o contorno), 0–255".to_string())]);
            let bytes = encode_png(&PngSpec { w: mw, h: mh, color_type: ColorType::Grayscale, depth: BitDepth::Eight, data: &data, color: PngColor::Data, dpi: Some(o.dpi), texts })?;
            files.push(out_file(AOV_MASK, &path, mw, mh, &bytes)?);
        }
        if let (true, Some(path), Some(rs)) = (o.aov.object_id, path_of(AOV_ID), lo.as_ref()) {
            let colors = aov::object_id_colors(&scene);
            let legend: std::collections::BTreeMap<String, String> =
                scene.nodes.iter().zip(&colors).filter(|(n, _)| n.mesh.is_some()).map(|(n, c)| (n.id.clone(), format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))).collect();
            let data = aov::id_image(rs, &colors);
            let texts = texts_for(spec, plan, &scene, AOV_ID, frame, &adapter, vec![("anigo:object_ids".to_string(), serde_json::json!(legend).to_string())]);
            let bytes = encode_png(&PngSpec { w: tw, h: th, color_type: ColorType::Rgba, depth: BitDepth::Eight, data: &data, color: PngColor::Data, dpi: Some(o.dpi), texts })?;
            files.push(out_file(AOV_ID, &path, tw, th, &bytes)?);
        }
        if let (true, Some(path), Some(rs)) = (o.aov.normal, path_of(AOV_NORMAL), lo.as_ref()) {
            let data = aov::normal_image(rs);
            let texts = texts_for(spec, plan, &scene, AOV_NORMAL, frame, &adapter, vec![("anigo:normal_encoding".to_string(), "espaço da câmera, n·0,5+0,5 (x direita, y cima, z para a câmera)".to_string())]);
            let bytes = encode_png(&PngSpec { w: tw, h: th, color_type: ColorType::Rgb, depth: BitDepth::Eight, data: &data, color: PngColor::Data, dpi: Some(o.dpi), texts })?;
            files.push(out_file(AOV_NORMAL, &path, tw, th, &bytes)?);
        }
    }
    step(Stage::Write, 1.0)?;
    Ok(FrameOutput { files, thumb, notes, render_ms })
}

// ------------------------------------------------------------------------------------------------
// Fila
// ------------------------------------------------------------------------------------------------

/// Fonte de cenas por quadro. O render avulso usa [`StillSource`]; a Animação implementará um que avalia a timeline.
pub trait FrameSource: Send + Sync {
    fn scene(&self, frame: Option<u32>) -> Result<Scene, String>;
}

pub struct StillSource(pub Scene);

impl FrameSource for StillSource {
    fn scene(&self, _frame: Option<u32>) -> Result<Scene, String> {
        Ok(self.0.clone())
    }
}

/// Intervalo de quadros (inclusivo) para sequências.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRange {
    pub start: u32,
    pub end: u32,
    pub step: u32,
}

impl FrameRange {
    pub fn frames(&self) -> Vec<u32> {
        let step = self.step.max(1) as usize;
        (self.start..=self.end.max(self.start)).step_by(step).collect()
    }
}

#[derive(Clone)]
pub struct JobSpec {
    pub id: u64,
    pub label: String,
    pub source: Arc<dyn FrameSource>,
    pub spec: RenderSpec,
    pub dir: PathBuf,
    pub template: String,
    pub overwrite: bool,
    /// Primeiro número do contador (render avulso).
    pub counter: u32,
    pub frames: Option<FrameRange>,
    /// Retomar: pula os quadros cujos arquivos já existem.
    pub skip_existing: bool,
    /// Nomes já alocados (retomar um avulso reaproveita o mesmo nome).
    pub reuse: Option<Allocation>,
    pub tokens: NameTokens,
}

#[derive(Debug, Clone)]
pub enum Event {
    Started { id: u64, total: usize },
    Progress { id: u64, frame_idx: usize, total: usize, stage: Stage, frac: f32 },
    FrameDone { id: u64, frame: Option<u32>, files: Vec<OutFile>, thumb: (u32, u32, Vec<u8>), notes: Vec<String>, render_ms: f64, alloc: Allocation },
    FrameSkipped { id: u64, frame: Option<u32> },
    Finished { id: u64, next_counter: u32 },
    Failed { id: u64, error: String },
    Cancelled { id: u64 },
}

enum Cmd {
    Run(Box<JobSpec>, Arc<AtomicBool>),
    Quit,
}

pub struct RenderService {
    tx: Sender<Cmd>,
    pub events: Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RenderService {
    /// `wake` é chamada a cada evento (acorda a UI).
    pub fn start(wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        let (etx, erx) = crossbeam_channel::unbounded::<Event>();
        let thread = std::thread::Builder::new()
            .name("anigo-render".into())
            .spawn(move || worker(rx, etx, wake))
            .ok();
        Self { tx, events: erx, thread }
    }

    pub fn submit(&self, job: JobSpec, cancel: Arc<AtomicBool>) {
        let _ = self.tx.send(Cmd::Run(Box::new(job), cancel));
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            // um render em andamento termina a etapa atual; não bloqueamos o fechamento do app esperando
            let _ = std::thread::Builder::new().name("anigo-render-join".into()).spawn(move || {
                let _ = t.join();
            });
        }
    }
}

fn worker(rx: Receiver<Cmd>, tx: Sender<Event>, wake: Option<Arc<dyn Fn() + Send + Sync>>) {
    let mut renderer: Option<Result<HeadlessRenderer, String>> = None;
    let send = |e: Event| {
        let _ = tx.send(e);
        if let Some(w) = &wake {
            w();
        }
    };
    while let Ok(cmd) = rx.recv() {
        let (job, cancel) = match cmd {
            Cmd::Run(j, c) => (*j, c),
            Cmd::Quit => break,
        };
        if cancel.load(Ordering::Relaxed) {
            send(Event::Cancelled { id: job.id });
            continue;
        }
        let r = renderer.get_or_insert_with(|| pollster::block_on(HeadlessRenderer::new()).map_err(|e| format!("sem GPU utilizável: {e:#}")));
        match r {
            Err(e) => send(Event::Failed { id: job.id, error: e.clone() }),
            Ok(r) => run_job(r, &job, &cancel, &send),
        }
    }
}

fn run_job(r: &mut HeadlessRenderer, job: &JobSpec, cancel: &AtomicBool, send: &dyn Fn(Event)) {
    let id = job.id;
    let max_dim = r.device.limits().max_texture_dimension_2d;
    let plan = match plan(&job.spec.output, max_dim, DEFAULT_MAX_PIXELS) {
        Ok(p) => p,
        Err(e) => return send(Event::Failed { id, error: e }),
    };
    let frames: Vec<Option<u32>> = match job.frames {
        Some(fr) => fr.frames().into_iter().map(Some).collect(),
        None => vec![None],
    };
    let total = frames.len();
    send(Event::Started { id, total });
    let aovs = enabled_aovs(&job.spec.output);
    let ext = job.spec.output.format.extension();
    let mut next_counter = job.counter;
    for (idx, frame) in frames.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return send(Event::Cancelled { id });
        }
        let scene = match job.source.scene(frame) {
            Ok(s) => s,
            Err(e) => return send(Event::Failed { id, error: e }),
        };
        let mut tokens = job.tokens.clone();
        tokens.frame = frame;
        tokens.w = plan.target.0;
        tokens.h = plan.target.1;
        tokens.fmt = ext.to_string();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (date, time) = utc_date_time(now);
        tokens.date = date.clone();
        tokens.time = time.clone();
        if let Err(e) = super::settings::check_output_dir(&job.dir) {
            return send(Event::Failed { id, error: e });
        }
        let alloc = if frame.is_some() {
            allocate_frame(&job.dir, &job.template, &tokens, ext, &aovs)
        } else if let Some(a) = &job.reuse {
            a.clone()
        } else {
            allocate(&job.dir, &job.template, &tokens, ext, &aovs, job.overwrite, job.counter)
        };
        if job.skip_existing && frame.is_some() && alloc.main.is_file() && alloc.aovs.iter().all(|(_, p)| p.is_file()) {
            send(Event::FrameSkipped { id, frame });
            continue;
        }
        next_counter = alloc.n.max(next_counter) + 1;
        let mut cb = |stage: Stage, f: f32| -> bool {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            send(Event::Progress { id, frame_idx: idx, total, stage, frac: f });
            true
        };
        match render_frame(r, scene, &job.spec, &plan, &alloc, frame, &mut cb) {
            Ok(out) => {
                record_history(job, &out, frame, &date, &time);
                send(Event::FrameDone { id, frame, files: out.files, thumb: out.thumb, notes: out.notes, render_ms: out.render_ms, alloc });
            }
            Err(RenderError::Cancelled) => return send(Event::Cancelled { id }),
            Err(RenderError::Failed(e)) => return send(Event::Failed { id, error: e }),
        }
    }
    send(Event::Finished { id, next_counter });
}

fn record_history(job: &JobSpec, out: &FrameOutput, frame: Option<u32>, date: &str, time: &str) {
    let thumb_dir = job.dir.join(THUMB_DIR);
    let when = format!("{date} {}", time.replace('-', ":"));
    let mut entries = Vec::new();
    for f in &out.files {
        let file = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mut thumb = None;
        if f.pass == "beauty" {
            let (tw, th, px) = &out.thumb;
            if let Ok(bytes) = encode_png(&PngSpec { w: *tw, h: *th, color_type: ColorType::Rgba, depth: BitDepth::Eight, data: px, color: PngColor::Data, dpi: None, texts: vec![] }) {
                let stem = f.path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let name = format!("{stem}.png");
                if write_atomic(&thumb_dir.join(&name), &bytes).is_ok() {
                    thumb = Some(name);
                }
            }
        }
        entries.push(HistoryEntry {
            file,
            pass: f.pass.clone(),
            w: f.w,
            h: f.h,
            bytes: f.bytes,
            project: job.spec.meta.project_name.clone(),
            camera: job.spec.meta.camera_name.clone(),
            when: when.clone(),
            fingerprint: format!("{:016x}", job.spec.meta.fingerprint),
            frame,
            thumb,
        });
    }
    if let Err(e) = history::append(&job.dir, entries) {
        tracing::warn!("histórico de render: {e}");
    }
}
