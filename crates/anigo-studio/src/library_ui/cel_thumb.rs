//! Miniaturas 3D reais: renderizador `ThumbnailRenderer` do studio sobre o renderer cel canônico
//! (`anigo_renderer::HeadlessRenderer`, o MESMO do viewport e do render final).
//!
//! * Device PRÓPRIO, criado de forma preguiçosa na primeira miniatura (não disputa o `wgpu::Device` do egui).
//! * Um `Mutex` serializa o uso da GPU: mesmo que o backend peça paralelismo, só um render por vez.
//! * Sem adaptador GPU (ou falha ao iniciar) o renderer devolve `Err`; a `ChainRenderer` do backend cai
//!   no renderizador CPU (ícone do tipo) sem `panic`.
//! * Lê GLB/VRM, OBJ e FBX (via `anigo-native`/ufbx), enquadra pela caixa envolvente (3/4 frontal) e usa
//!   fundo neutro. O resultado depende de [`PIPELINE_VER`]: mude-o quando o visual mudar.

use anigo_assets::{Rendered, ThumbRequest, ThumbnailRenderer};
use anigo_core::{scene::SceneNode, Camera, Mesh, Scene, StylizedLight, StylizedMaterial};
use anigo_core::mesh::Vertex;
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Versão do visual das miniaturas (entra na chave do cache em disco).
pub const PIPELINE_VER: u32 = 3;

/// Fundo neutro das miniaturas (cor do clear, espaço do renderer).
pub const THUMB_BG: [f32; 4] = [0.30, 0.31, 0.35, 1.0];

/// Formatos 3D que o renderer sabe abrir.
pub fn is_3d_format(fmt: &str) -> bool {
    matches!(fmt.to_ascii_lowercase().as_str(), "glb" | "vrm" | "obj" | "fbx")
}

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_VERTS: usize = 4_000_000;

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("arquivo grande demais para miniatura ({} MiB)", meta.len() / 1048576));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Carrega o arquivo primário como malha do núcleo (GLB/VRM, OBJ ou FBX).
pub fn load_core_mesh(path: &Path, format: &str) -> Result<Mesh, String> {
    match format.to_ascii_lowercase().as_str() {
        "glb" | "vrm" => {
            let bytes = read_limited(path)?;
            Mesh::from_glb_bytes(&bytes).map_err(|e| e.to_string())
        }
        "obj" => {
            let bytes = read_limited(path)?;
            parse_obj(&String::from_utf8_lossy(&bytes), 3_000_000)
        }
        "fbx" => {
            let bytes = read_limited(path)?;
            fbx_to_mesh(&bytes)
        }
        other => Err(format!("formato {other} não é um modelo 3D renderizável")),
    }
}

fn fbx_to_mesh(bytes: &[u8]) -> Result<Mesh, String> {
    use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
    let scene = anigo_native::fbx::import_fbx(bytes).map_err(|e| e.to_string())?;
    let mut all = Mesh::new("fbx", Vec::new(), Vec::new());
    for node in 0..scene.nodes.len() {
        let Some(em) = scene.mesh_in_world_space(node) else { continue };
        let Ok(m) = to_core_mesh(&em, &ToCoreOptions::default()) else { continue };
        let base = all.vertices.len() as u32;
        all.vertices.extend(m.vertices);
        all.indices.extend(m.indices.iter().map(|i| i + base));
        if all.vertices.len() > MAX_VERTS {
            return Err("malha FBX grande demais para miniatura".into());
        }
    }
    if all.indices.is_empty() {
        return Err("FBX sem triângulos".into());
    }
    Ok(all)
}

/// Leitor OBJ mínimo (v/vt/vn/f com n-gons em leque e índices negativos). Calcula normais suaves se faltarem.
pub fn parse_obj(text: &str, max_tris: usize) -> Result<Mesh, String> {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut has_normal: Vec<bool> = Vec::new();
    let mut map: HashMap<(usize, Option<usize>, Option<usize>), u32> = HashMap::new();
    let mut indices: Vec<u32> = Vec::new();
    let parse3 = |it: &mut std::str::SplitWhitespace| -> Option<[f32; 3]> {
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        let c = it.next()?.parse().ok()?;
        Some([a, b, c])
    };
    let resolve = |s: &str, len: usize| -> Option<usize> {
        let n: i64 = s.parse().ok()?;
        let i = if n > 0 { n - 1 } else { len as i64 + n };
        (i >= 0 && (i as usize) < len).then_some(i as usize)
    };
    for line in text.lines() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                if let Some(p) = parse3(&mut it) {
                    if p.iter().all(|c| c.is_finite()) {
                        pos.push(p);
                    } else {
                        pos.push([0.0; 3]);
                    }
                }
            }
            Some("vt") => {
                let u = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let v = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                uvs.push([u, 1.0 - v]);
            }
            Some("vn") => {
                if let Some(n) = parse3(&mut it) {
                    nrm.push(n);
                }
            }
            Some("f") => {
                let mut face: Vec<u32> = Vec::new();
                for tok in it {
                    let mut parts = tok.split('/');
                    let Some(vi) = parts.next().and_then(|s| resolve(s, pos.len())) else { continue };
                    let ti = parts.next().filter(|s| !s.is_empty()).and_then(|s| resolve(s, uvs.len()));
                    let ni = parts.next().filter(|s| !s.is_empty()).and_then(|s| resolve(s, nrm.len()));
                    let idx = *map.entry((vi, ti, ni)).or_insert_with(|| {
                        let n = ni.map(|k| nrm[k]).unwrap_or([0.0; 3]);
                        let uv = ti.map(|k| uvs[k]).unwrap_or([0.0; 2]);
                        verts.push(Vertex::new(pos[vi], n, uv));
                        has_normal.push(ni.is_some());
                        (verts.len() - 1) as u32
                    });
                    face.push(idx);
                }
                for k in 1..face.len().saturating_sub(1) {
                    indices.extend([face[0], face[k], face[k + 1]]);
                }
                if indices.len() / 3 > max_tris {
                    return Err("OBJ com triângulos demais para miniatura".into());
                }
            }
            _ => {}
        }
    }
    if indices.is_empty() {
        return Err("OBJ sem faces".into());
    }
    // normais suaves para os vértices sem 'vn'
    let mut acc = vec![Vec3::ZERO; verts.len()];
    for t in indices.chunks_exact(3) {
        let p = |i: u32| Vec3::from_array(verts[i as usize].position);
        let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        for &i in t {
            acc[i as usize] += n;
        }
    }
    for (i, v) in verts.iter_mut().enumerate() {
        if !has_normal[i] {
            v.normal = acc[i].try_normalize().unwrap_or(Vec3::Y).to_array();
        }
    }
    Ok(Mesh::new("obj", verts, indices))
}

/// Malha pronta para renderizar: sem skinning, com caixa envolvente.
pub struct PreparedMesh {
    pub mesh: Mesh,
    pub center: Vec3,
    /// Raio da esfera que envolve a caixa.
    pub radius: f32,
}

impl PreparedMesh {
    pub fn new(mut mesh: Mesh) -> Result<Self, String> {
        if mesh.vertices.is_empty() || mesh.indices.len() < 3 {
            return Err("malha vazia".into());
        }
        if mesh.vertices.len() > MAX_VERTS {
            return Err("malha grande demais para miniatura".into());
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in &mut mesh.vertices {
            let p = Vec3::from_array(v.position);
            if p.is_finite() {
                lo = lo.min(p);
                hi = hi.max(p);
            } else {
                v.position = [0.0; 3];
            }
            // miniatura = pose de repouso: sem a paleta de ossos da cena
            v.joints = [0; 4];
            v.weights = [1.0, 0.0, 0.0, 0.0];
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Err("malha sem vértices finitos".into());
        }
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(1e-4);
        Ok(Self { mesh, center, radius })
    }
}

/// Ângulos da câmera orbital da miniatura/preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewParams {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for ViewParams {
    fn default() -> Self {
        Self { yaw: 35f32.to_radians(), pitch: 22f32.to_radians(), zoom: 1.0 }
    }
}

const FOV_Y: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// Câmera que enquadra a esfera `(center, radius)` inteira.
pub fn frame_camera(center: Vec3, radius: f32, view: ViewParams) -> Camera {
    let dist = radius / (FOV_Y * 0.5).sin() * 1.04 / view.zoom.max(0.1);
    let dir = Vec3::new(view.yaw.sin() * view.pitch.cos(), view.pitch.sin(), view.yaw.cos() * view.pitch.cos());
    Camera {
        eye: center + dir * dist,
        target: center,
        up: Vec3::Y,
        fov_y: FOV_Y,
        aspect: 1.0,
        z_near: (dist - radius * 1.5).max(dist * 0.02),
        z_far: dist + radius * 3.0,
        orthographic: None,
    }
}

/// Luz de estúdio presa à câmera (esquerda/acima/frente), como no viewport.
pub fn studio_light(cam: &Camera) -> StylizedLight {
    let forward = (cam.target - cam.eye).normalize_or_zero();
    let right = forward.cross(cam.up).normalize_or_zero();
    let up = right.cross(forward).normalize_or_zero();
    let dir = (-right * 0.45 + up * 0.6 + -forward * 0.65).normalize_or_zero();
    StylizedLight { direction: dir.to_array(), ..StylizedLight::default() }
}

enum Gpu {
    Unready,
    Ready(Box<HeadlessRenderer>),
    Failed(String),
}

/// Renderizador de miniaturas 3D (e do preview girável).
pub struct CelThumbnailRenderer {
    gpu: Mutex<Gpu>,
}

impl Default for CelThumbnailRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl CelThumbnailRenderer {
    pub fn new() -> Self {
        Self { gpu: Mutex::new(Gpu::Unready) }
    }

    /// Renderer que já nasce "sem GPU" (testes do caminho de degradação; o app usa `new`).
    pub fn unavailable(reason: &str) -> Self {
        Self { gpu: Mutex::new(Gpu::Failed(reason.to_string())) }
    }

    /// `Some(false)` se a GPU falhou ao iniciar; `None` se ainda não tentou.
    pub fn available(&self) -> Option<bool> {
        match &*self.gpu.lock().unwrap_or_else(|e| e.into_inner()) {
            Gpu::Unready => None,
            Gpu::Ready(_) => Some(true),
            Gpu::Failed(_) => Some(false),
        }
    }

    pub fn failure_reason(&self) -> Option<String> {
        match &*self.gpu.lock().unwrap_or_else(|e| e.into_inner()) {
            Gpu::Failed(e) => Some(e.clone()),
            _ => None,
        }
    }

    fn with_gpu<R>(&self, f: impl FnOnce(&HeadlessRenderer) -> Result<R, String>) -> Result<R, String> {
        let mut g = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*g, Gpu::Unready) {
            *g = match std::panic::catch_unwind(|| pollster::block_on(HeadlessRenderer::new())) {
                Ok(Ok(r)) => Gpu::Ready(Box::new(r)),
                Ok(Err(e)) => Gpu::Failed(format!("{e:#}")),
                Err(_) => Gpu::Failed("panic ao iniciar o renderer".into()),
            };
        }
        match &*g {
            Gpu::Ready(r) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(r))).unwrap_or_else(|_| Err("panic no render".into())),
            Gpu::Failed(e) => Err(format!("sem GPU para miniaturas 3D: {e}")),
            Gpu::Unready => Err("renderer não iniciado".into()),
        }
    }

    /// Renderiza uma malha preparada em `size`x`size` com a câmera `view`.
    pub fn render_prepared(&self, prep: &PreparedMesh, view: ViewParams, material: Option<StylizedMaterial>, size: u32) -> Result<RgbaImage, String> {
        let size = size.clamp(16, 2048);
        let mut scene = Scene::new_empty();
        scene.background_color = THUMB_BG;
        let mut node = SceneNode::new("thumb", "thumb");
        node.mesh = Some(prep.mesh.clone());
        if let Some(m) = material {
            node.material = Some(m);
        }
        scene.nodes.push(node);
        scene.camera = frame_camera(prep.center, prep.radius, view);
        scene.light = studio_light(&scene.camera);
        self.with_gpu(|r| {
            let (img, _) = pollster::block_on(r.render_scene(&scene, size, size)).map_err(|e| format!("{e:#}"))?;
            Ok(img)
        })
    }
}

impl CelThumbnailRenderer {
    /// Miniatura 3D de uma POSE: o personagem canônico padrão, posado de verdade (paleta de skinning do núcleo),
    /// enquadrado de corpo inteiro em 3/4.
    pub fn render_pose(&self, pose: &anigo_core::pose::PoseState, size: u32) -> Result<RgbaImage, String> {
        let size = size.clamp(16, 2048);
        let mut session = anigo_session::CoreSession::new();
        if let Some(cmd) = anigo_core::pose::plan_pose_change(&anigo_core::pose::PoseState::default(), pose, anigo_core::pose::PoseOp::Apply) {
            session.apply(cmd).map_err(|e| format!("pose: {e}"))?;
        }
        let mut scene = Scene::new_empty();
        session.sync_scene(&mut scene)?;
        scene.background_color = THUMB_BG;
        // enquadra pela caixa da malha rígida (a pose pode levantar os braços acima da cabeça)
        let center = Vec3::new(0.0, 0.92, 0.0);
        scene.camera = frame_camera(center, 1.02, ViewParams { yaw: 28f32.to_radians(), pitch: 10f32.to_radians(), zoom: 1.0 });
        scene.light = studio_light(&scene.camera);
        self.with_gpu(|r| {
            let (img, _) = pollster::block_on(r.render_scene(&scene, size, size)).map_err(|e| format!("{e:#}"))?;
            Ok(img)
        })
    }
}

impl CelThumbnailRenderer {
    /// Miniatura 3D de um CLIP: tira 2x2 com o personagem posado em 4 instantes do clip.
    pub fn render_clip_strip(&self, asset: &anigo_core::clip_asset::ClipAsset, size: u32) -> Result<RgbaImage, String> {
        let size = size.clamp(32, 2048);
        let tile = (size / 2).max(16);
        let mut img = RgbaImage::new(size, size);
        for (i, pose) in asset.strip_poses(4).iter().enumerate() {
            let t = self.render_pose(pose, tile)?;
            let (ox, oy) = ((i as u32 % 2) * tile, (i as u32 / 2) * tile);
            for (x, y, p) in t.enumerate_pixels() {
                if ox + x < size && oy + y < size {
                    img.put_pixel(ox + x, oy + y, *p);
                }
            }
        }
        Ok(img)
    }
}

/// Se o ativo tem exatamente UM material canônico, devolve-o como `StylizedMaterial` (cor-base e sombra reais).
pub fn single_material(asset_dir: &Path) -> Option<StylizedMaterial> {
    let rd = std::fs::read_dir(asset_dir.join("materials")).ok()?;
    let files: Vec<_> = rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(anigo_assets::MATERIAL_SUFFIX)).collect();
    if files.len() != 1 {
        return None;
    }
    let bytes = std::fs::read(files[0].path()).ok()?;
    anigo_assets::ToonMaterial::from_json(&bytes).ok().map(|m| m.to_stylized())
}

impl ThumbnailRenderer for CelThumbnailRenderer {
    fn id(&self) -> &str {
        "cel-gpu"
    }

    fn render(&self, req: &ThumbRequest) -> Result<Rendered, String> {
        // Ativo Material (.anigomat.json): shader ball no renderer cel canônico (o mesmo do viewport)
        if req.kind == anigo_assets::AssetKind::Material && req.format == "anigomat" {
            if self.available() == Some(false) {
                return Err(self.failure_reason().unwrap_or_default());
            }
            let meta = std::fs::metadata(&req.primary).map_err(|e| format!("{}: {e}", req.primary.display()))?;
            if meta.len() > anigo_assets::material::MATERIAL_MAX_BYTES as u64 {
                return Err("material grande demais".into());
            }
            let tm = anigo_assets::ToonMaterial::from_json(&std::fs::read(&req.primary).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            let prep = PreparedMesh::new((*crate::shading::ball::ball_mesh()).clone())?;
            let image = self.render_prepared(&prep, ViewParams::default(), Some(tm.to_stylized()), req.size)?;
            return Ok(Rendered { image, placeholder: false });
        }
        if req.format == "anigopose" {
            if self.available() == Some(false) {
                return Err(self.failure_reason().unwrap_or_default());
            }
            let bytes = read_limited(&req.primary)?;
            let asset = anigo_core::pose::PoseAsset::from_json(&bytes).map_err(|e| e.to_string())?;
            let image = self.render_pose(&asset.pose, req.size)?;
            return Ok(Rendered { image, placeholder: false });
        }
        if req.format == "anigoclip" {
            if self.available() == Some(false) {
                return Err(self.failure_reason().unwrap_or_default());
            }
            let bytes = read_limited(&req.primary)?;
            let asset = anigo_core::clip_asset::ClipAsset::from_json(&bytes).map_err(|e| e.to_string())?;
            let image = self.render_clip_strip(&asset, req.size)?;
            return Ok(Rendered { image, placeholder: false });
        }
        if !is_3d_format(&req.format) {
            return Err(format!("{} não é 3D", req.format));
        }
        // falha cedo (sem carregar a malha) quando a GPU já se mostrou indisponível
        if self.available() == Some(false) {
            return Err(self.failure_reason().unwrap_or_default());
        }
        let mesh = load_core_mesh(&req.primary, &req.format)?;
        let prep = PreparedMesh::new(mesh)?;
        let image = self.render_prepared(&prep, ViewParams::default(), single_material(&req.asset_dir), req.size)?;
        Ok(Rendered { image, placeholder: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_parser_handles_ngons_negative_indices_and_missing_normals() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf -4 -3 -2\n";
        let m = parse_obj(obj, 1000).unwrap();
        assert_eq!(m.indices.len(), 9, "quad em leque (2 tris) + 1 tri");
        // normais calculadas (+Z)
        assert!(m.vertices.iter().all(|v| v.normal[2] > 0.99), "{:?}", m.vertices[0].normal);
        assert!(parse_obj("v 0 0 0\n", 10).is_err());
        assert!(parse_obj("", 10).is_err());
        // índice fora de faixa é ignorado, não dá panic
        let m = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3 9\n", 10).unwrap();
        assert_eq!(m.indices.len(), 3);
    }

    #[test]
    fn obj_parser_obeys_the_triangle_limit() {
        let mut s = String::from("v 0 0 0\nv 1 0 0\nv 0 1 0\n");
        for _ in 0..50 {
            s.push_str("f 1 2 3\n");
        }
        assert!(parse_obj(&s, 10).is_err());
        assert!(parse_obj(&s, 100).is_ok());
    }

    #[test]
    fn camera_frames_the_whole_bounding_sphere() {
        let prep = PreparedMesh::new(Mesh::create_cube(2.0)).unwrap();
        assert!((prep.radius - 3f32.sqrt()).abs() < 1e-4);
        let cam = frame_camera(prep.center, prep.radius, ViewParams::default());
        let d = (cam.eye - cam.target).length();
        // a esfera cabe no campo de visão vertical
        assert!(prep.radius / d < (FOV_Y * 0.5).sin());
        assert!(cam.z_near > 0.0 && cam.z_far > d + prep.radius);
        // zoom aproxima
        let near = frame_camera(prep.center, prep.radius, ViewParams { zoom: 2.0, ..Default::default() });
        assert!((near.eye - near.target).length() < d);
    }

    #[test]
    fn prepared_mesh_rejects_empty_and_neutralizes_skinning() {
        assert!(PreparedMesh::new(Mesh::new("x", vec![], vec![])).is_err());
        let mut m = Mesh::create_cube(1.0);
        m.vertices[0].joints = [5, 0, 0, 0];
        m.vertices[0].weights = [0.5, 0.5, 0.0, 0.0];
        let p = PreparedMesh::new(m).unwrap();
        assert_eq!(p.mesh.vertices[0].joints, [0; 4]);
        assert_eq!(p.mesh.vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn non_3d_formats_are_refused_without_touching_the_gpu() {
        let r = CelThumbnailRenderer::new();
        let req = ThumbRequest {
            id: anigo_assets::AssetId::new(),
            kind: anigo_assets::AssetKind::Texture,
            format: "png".into(),
            content_hash: anigo_assets::ContentHash([0; 32]),
            primary: "x.png".into(),
            files: vec![],
            asset_dir: ".".into(),
            size: 512,
        };
        assert!(r.render(&req).is_err());
        assert_eq!(r.available(), None, "não deve iniciar a GPU para um PNG");
    }

    /// Renderiza de verdade quando há adaptador (lavapipe no CI); sem adaptador, prova que NÃO dá panic.
    #[test]
    fn renders_a_glb_or_degrades_gracefully_without_gpu() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cubo.glb");
        Mesh::create_cube(1.0).to_glb_file(&p).unwrap();
        let r = CelThumbnailRenderer::new();
        let req = ThumbRequest {
            id: anigo_assets::AssetId::new(),
            kind: anigo_assets::AssetKind::Model,
            format: "glb".into(),
            content_hash: anigo_assets::ContentHash([1; 32]),
            primary: p,
            files: vec![],
            asset_dir: dir.path().to_path_buf(),
            size: 128,
        };
        match r.render(&req) {
            Ok(out) => {
                assert!(!out.placeholder);
                assert_eq!((out.image.width(), out.image.height()), (128, 128));
                let bg = out.image.get_pixel(0, 0).0;
                let fg = out.image.pixels().filter(|px| px.0 != bg).count();
                assert!(fg > 1500, "o cubo deveria cobrir uma parte da imagem (fg={fg})");
                let corner = out.image.get_pixel(127, 127).0;
                assert_eq!(corner, bg, "fundo uniforme nos cantos");
                if let Ok(d) = std::env::var("ANIGO_TEST_OUT") {
                    let _ = std::fs::create_dir_all(&d);
                    let _ = out.image.save(format!("{d}/lib_thumb_cube.png"));
                }
            }
            Err(e) => {
                eprintln!("sem GPU, miniatura 3D indisponível (esperado em máquina sem adaptador): {e}");
                assert_eq!(r.available(), Some(false));
            }
        }
    }
}
