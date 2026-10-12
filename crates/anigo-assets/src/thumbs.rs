//! Miniaturas (T-5.6, parte CPU): cache em disco por `blake3 + pipeline_ver`, LRU em RAM com orcamento de
//! bytes, redimensionamento 512 -> 256/128/64 (fast_image_resize) e renderizadores CPU.
//!
//! O render 3D "cel" (GPU) NAO esta aqui: implemente [`ThumbnailRenderer`] no `anigo-studio` (usando
//! `HeadlessRenderer::render_to_texture`) e passe-o a [`Library::generate_thumbnails`]. Os testes deste modulo
//! nao precisam de GPU.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fast_image_resize as fir;
use image::RgbaImage;
use lru::LruCache;
use rayon::prelude::*;

use crate::error::{AssetError, Result};
use crate::job::{Job, JobCtx};
use crate::library::Library;
use crate::material::ToonMaterial;
use crate::model::*;
use crate::util::atomic_write;

/// Tamanhos em disco (lado do quadrado). 512 e o "master".
pub const THUMB_SIZES: [u32; 4] = [64, 128, 256, 512];
pub const MASTER_SIZE: u32 = 512;

/// Pedido de miniatura entregue ao renderizador.
#[derive(Debug, Clone)]
pub struct ThumbRequest {
    pub id: AssetId,
    pub kind: AssetKind,
    pub format: String,
    pub content_hash: ContentHash,
    /// Arquivo primario (absoluto).
    pub primary: PathBuf,
    /// Todos os arquivos da revisao (absolutos).
    pub files: Vec<PathBuf>,
    /// Pasta do ativo (onde estao `asset.json` e `materials/`).
    pub asset_dir: PathBuf,
    /// Lado do master desejado.
    pub size: u32,
}

/// Imagem produzida por um renderizador.
pub struct Rendered {
    pub image: RgbaImage,
    /// `true` = e um substituto (icone/cor), nao o conteudo real: NAO vai para o cache de disco.
    pub placeholder: bool,
}

/// Renderiza a miniatura master de um ativo. Implementacoes: [`ImageThumbnailRenderer`],
/// [`MaterialSwatchRenderer`], [`PlaceholderRenderer`], [`ChainRenderer`], [`ClosureRenderer`] e, fora deste
/// crate, o renderizador 3D cel (GPU).
pub trait ThumbnailRenderer: Send + Sync {
    /// Identificador estavel (aparece em logs; mude `pipeline_ver` da biblioteca quando o visual mudar).
    fn id(&self) -> &str;
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String>;
}

/// Adaptador de closure.
pub struct ClosureRenderer<F> {
    id: String,
    f: F,
}

impl<F: Fn(&ThumbRequest) -> std::result::Result<Rendered, String> + Send + Sync> ClosureRenderer<F> {
    pub fn new(id: impl Into<String>, f: F) -> Self {
        Self { id: id.into(), f }
    }
}

impl<F: Fn(&ThumbRequest) -> std::result::Result<Rendered, String> + Send + Sync> ThumbnailRenderer for ClosureRenderer<F> {
    fn id(&self) -> &str {
        &self.id
    }
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String> {
        (self.f)(req)
    }
}

/// Tenta cada renderizador em ordem; o primeiro que nao falhar vence (ex.: GPU -> imagem -> placeholder).
pub struct ChainRenderer {
    chain: Vec<Arc<dyn ThumbnailRenderer>>,
}

impl ChainRenderer {
    pub fn new(chain: Vec<Arc<dyn ThumbnailRenderer>>) -> Self {
        Self { chain }
    }
}

impl ThumbnailRenderer for ChainRenderer {
    fn id(&self) -> &str {
        "chain"
    }
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String> {
        let mut errs = Vec::new();
        for r in &self.chain {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.render(req))) {
                Ok(Ok(img)) => return Ok(img),
                Ok(Err(e)) => errs.push(format!("{}: {e}", r.id())),
                Err(_) => errs.push(format!("{}: panic", r.id())),
            }
        }
        Err(errs.join("; "))
    }
}

/// Renderizador padrao sem GPU: imagens -> miniatura real; materiais `.anigomat.json` -> esfera toon; resto -> placeholder.
pub fn default_cpu_renderer() -> Arc<dyn ThumbnailRenderer> {
    Arc::new(ChainRenderer::new(vec![Arc::new(ImageThumbnailRenderer::default()), Arc::new(MaterialSwatchRenderer), Arc::new(PlaceholderRenderer)]))
}

// ---------------------------------------------------------------------------------------------
// Redimensionamento
// ---------------------------------------------------------------------------------------------

fn fir_err(e: impl std::fmt::Display) -> AssetError {
    AssetError::Internal(format!("redimensionamento: {e}"))
}

/// Redimensiona RGBA8 (Lanczos3, alpha tratado) com `fast_image_resize`.
pub fn resize_rgba(src: &RgbaImage, w: u32, h: u32) -> Result<RgbaImage> {
    if w == 0 || h == 0 || src.width() == 0 || src.height() == 0 {
        return Err(AssetError::InvalidInput("dimensoes zero".into()));
    }
    if src.width() == w && src.height() == h {
        return Ok(src.clone());
    }
    let s = fir::images::Image::from_vec_u8(src.width(), src.height(), src.as_raw().clone(), fir::PixelType::U8x4).map_err(fir_err)?;
    let mut d = fir::images::Image::new(w, h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    let opts = fir::ResizeOptions::new().resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3));
    resizer.resize(&s, &mut d, &opts).map_err(fir_err)?;
    RgbaImage::from_raw(w, h, d.into_vec()).ok_or_else(|| AssetError::Internal("buffer de saida invalido".into()))
}

/// Encaixa `img` num quadrado `size` x `size` (mantem a proporcao, centraliza, fundo transparente).
pub fn fit_square(img: &RgbaImage, size: u32) -> Result<RgbaImage> {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let scale = (size as f64 / w as f64).min(size as f64 / h as f64);
    let nw = ((w as f64 * scale).round() as u32).clamp(1, size);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, size);
    let resized = resize_rgba(img, nw, nh)?;
    if nw == size && nh == size {
        return Ok(resized);
    }
    let mut canvas = RgbaImage::new(size, size);
    let ox = (size - nw) / 2;
    let oy = (size - nh) / 2;
    image::imageops::replace(&mut canvas, &resized, ox as i64, oy as i64);
    Ok(canvas)
}

// ---------------------------------------------------------------------------------------------
// Renderizadores CPU
// ---------------------------------------------------------------------------------------------

/// Decodifica PNG/JPEG (com limites) e encaixa no quadrado.
#[derive(Debug, Clone)]
pub struct ImageThumbnailRenderer {
    /// Lado maximo aceito (px) e memoria maxima (bytes) de decodificacao.
    pub max_dimension: u32,
    pub max_alloc: u64,
}

impl Default for ImageThumbnailRenderer {
    fn default() -> Self {
        Self { max_dimension: 16_384, max_alloc: 512 * 1024 * 1024 }
    }
}

impl ThumbnailRenderer for ImageThumbnailRenderer {
    fn id(&self) -> &str {
        "cpu-image"
    }
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String> {
        if !matches!(req.format.as_str(), "png" | "jpg" | "jpeg") {
            return Err(format!("formato {:?} nao decodificavel", req.format));
        }
        let mut reader = image::ImageReader::open(&req.primary).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
        let mut lim = image::Limits::default();
        lim.max_image_width = Some(self.max_dimension);
        lim.max_image_height = Some(self.max_dimension);
        lim.max_alloc = Some(self.max_alloc);
        reader.limits(lim);
        let img = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
        let out = fit_square(&img, req.size).map_err(|e| e.to_string())?;
        Ok(Rendered { image: out, placeholder: false })
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Esfera "toon" desenhada na CPU a partir de um [`ToonMaterial`]: mostra a cor base, a sombra e o corte do
/// material sem GPU. Nao usa as texturas (so cores/limiar). Aceita assets `material` (`.anigomat.json`).
#[derive(Debug, Clone, Copy, Default)]
pub struct MaterialSwatchRenderer;

impl MaterialSwatchRenderer {
    /// Desenha a esfera de `m` em `size` x `size`.
    pub fn draw(m: &ToonMaterial, size: u32) -> RgbaImage {
        let mut img = RgbaImage::new(size, size);
        let n = size as f32;
        let light = {
            let v = [-0.45f32, 0.6, 0.66];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / l, v[1] / l, v[2] / l]
        };
        for y in 0..size {
            for x in 0..size {
                let px = ((x as f32 + 0.5) / n) * 2.0 - 1.0;
                let py = 1.0 - ((y as f32 + 0.5) / n) * 2.0;
                let r2 = px * px + py * py;
                let radius = 0.86f32;
                let d = r2.sqrt() - radius;
                let aa = 1.5 / n * 2.0;
                let cover = 1.0 - smoothstep(-aa, aa, d);
                if cover <= 0.0 {
                    continue;
                }
                let rr = (radius * radius - r2).max(0.0).sqrt();
                let nrm = [px / radius, py / radius, rr / radius];
                let hl = (nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]) * 0.5 + 0.5;
                let lit = smoothstep(m.shade.threshold - m.shade.smoothness.max(0.004), m.shade.threshold + m.shade.smoothness.max(0.004), hl);
                let base = [m.base.color[0], m.base.color[1], m.base.color[2]];
                let mut col = [0.0f32; 3];
                for i in 0..3 {
                    col[i] = m.shade.color[i] * (1.0 - lit) + base[i] * lit;
                }
                // contorno fino
                let edge = smoothstep(-aa * 3.0, -aa, d);
                if m.outline.mode != crate::material::OutlineMode::None {
                    for i in 0..3 {
                        col[i] = m.outline.color[i] * edge + col[i] * (1.0 - edge);
                    }
                }
                img.put_pixel(x, y, image::Rgba([to_u8(linear_to_srgb(col[0])), to_u8(linear_to_srgb(col[1])), to_u8(linear_to_srgb(col[2])), to_u8(cover * m.base.color[3].max(0.2))]));
            }
        }
        img
    }
}

impl ThumbnailRenderer for MaterialSwatchRenderer {
    fn id(&self) -> &str {
        "cpu-material-swatch"
    }
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String> {
        if req.format != "anigomat" {
            return Err("nao e um .anigomat.json".into());
        }
        let meta = std::fs::metadata(&req.primary).map_err(|e| e.to_string())?;
        if meta.len() > crate::material::MATERIAL_MAX_BYTES as u64 {
            return Err("material grande demais".into());
        }
        let m = ToonMaterial::from_json(&std::fs::read(&req.primary).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(Rendered { image: Self::draw(&m, req.size), placeholder: false })
    }
}

/// Icone por tipo, deterministico, sem E/S: usado enquanto o render real nao existe.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaceholderRenderer;

impl PlaceholderRenderer {
    fn tint(kind: AssetKind) -> [u8; 3] {
        match kind {
            AssetKind::Model => [92, 124, 196],
            AssetKind::Character => [201, 112, 160],
            AssetKind::Outfit => [160, 110, 200],
            AssetKind::Hair => [214, 150, 80],
            AssetKind::Accessory => [90, 170, 160],
            AssetKind::Material => [120, 190, 120],
            AssetKind::Texture => [150, 150, 160],
            AssetKind::Pose => [200, 120, 110],
            AssetKind::Animation => [220, 180, 70],
            AssetKind::Hdri => [100, 160, 210],
            AssetKind::Scene => [110, 140, 120],
            AssetKind::Prefab => [140, 120, 180],
        }
    }

    /// Desenha o icone de `kind` em `size` x `size`.
    pub fn draw(kind: AssetKind, size: u32) -> RgbaImage {
        let [r, g, b] = Self::tint(kind);
        let mut img = RgbaImage::new(size, size);
        let n = size as f32;
        for y in 0..size {
            for x in 0..size {
                let u = (x as f32 + 0.5) / n * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / n * 2.0 - 1.0;
                // tile arredondado
                let q = [u.abs() - 0.86, v.abs() - 0.86];
                let rd = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt() + q[0].max(q[1]).min(0.0) - 0.12;
                let cover = 1.0 - smoothstep(-0.01, 0.01, rd);
                if cover <= 0.0 {
                    continue;
                }
                let shade = 0.78 + 0.22 * (1.0 - (v * 0.5 + 0.5));
                let sdf = Self::glyph_sdf(kind, u, v);
                let glyph = 1.0 - smoothstep(-0.01, 0.02, sdf);
                let k = glyph * 0.9;
                let bg = [r as f32 * shade, g as f32 * shade, b as f32 * shade];
                let col = [bg[0] * (1.0 - k) + 245.0 * k, bg[1] * (1.0 - k) + 245.0 * k, bg[2] * (1.0 - k) + 245.0 * k];
                img.put_pixel(x, y, image::Rgba([col[0] as u8, col[1] as u8, col[2] as u8, to_u8(cover)]));
            }
        }
        img
    }

    fn glyph_sdf(kind: AssetKind, u: f32, v: f32) -> f32 {
        let circle = |cx: f32, cy: f32, r: f32| ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() - r;
        let rect = |cx: f32, cy: f32, hx: f32, hy: f32| {
            let dx = (u - cx).abs() - hx;
            let dy = (v - cy).abs() - hy;
            (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0)
        };
        match kind {
            AssetKind::Character | AssetKind::Outfit | AssetKind::Hair | AssetKind::Accessory | AssetKind::Pose => circle(0.0, -0.3, 0.22).min(rect(0.0, 0.3, 0.38, 0.28)),
            AssetKind::Material => circle(0.0, 0.0, 0.5),
            AssetKind::Texture => {
                let cx = ((u * 3.0 + 3.0).floor() as i32 + (v * 3.0 + 3.0).floor() as i32) % 2 == 0;
                if rect(0.0, 0.0, 0.5, 0.5) < 0.0 && cx {
                    -1.0
                } else {
                    1.0
                }
            }
            AssetKind::Animation => rect(-0.3, 0.1, 0.1, 0.4).min(rect(0.0, 0.0, 0.1, 0.5)).min(rect(0.3, -0.1, 0.1, 0.4)),
            AssetKind::Hdri => circle(0.0, 0.2, 0.5).max(-(v - 0.2)),
            // cubo isometrico simplificado: hexagono
            AssetKind::Model | AssetKind::Scene | AssetKind::Prefab => {
                let (ax, ay) = (u.abs(), v.abs());
                (ax * 0.866 + ay * 0.5).max(ay) - 0.5
            }
        }
    }
}

impl ThumbnailRenderer for PlaceholderRenderer {
    fn id(&self) -> &str {
        "placeholder"
    }
    fn render(&self, req: &ThumbRequest) -> std::result::Result<Rendered, String> {
        Ok(Rendered { image: Self::draw(req.kind, req.size), placeholder: true })
    }
}

// ---------------------------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------------------------

struct Ram {
    lru: LruCache<(ContentHash, u32), Arc<RgbaImage>>,
    bytes: usize,
    budget: usize,
}

/// Cache de miniaturas: disco (`<cache>/thumbs/<lado>/<hh>/<blake3>-v<ver>.png`) + LRU em RAM com orcamento de bytes.
///
/// Chave = BLAKE3 do conteudo + `pipeline_ver`: invalida sozinho quando o conteudo OU o visual do pipeline muda.
/// Tudo aqui e DESCARTAVEL: `clear()` apaga sem perder nada.
pub struct ThumbnailCache {
    root: PathBuf,
    ver: u32,
    ram: Mutex<Ram>,
}

/// Resultado de `ensure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ensured {
    /// Ja existia no disco (nada foi renderizado).
    pub cached: bool,
    /// O renderizador devolveu um substituto (nao persistido).
    pub placeholder: bool,
}

impl ThumbnailCache {
    /// `dir` = pasta de cache (ex.: `Library::cache_dir()`); `ram_budget` em bytes.
    pub fn new(dir: &Path, pipeline_ver: u32, ram_budget: usize) -> Self {
        Self {
            root: dir.join("thumbs"),
            ver: pipeline_ver,
            ram: Mutex::new(Ram { lru: LruCache::unbounded(), bytes: 0, budget: ram_budget }),
        }
    }

    pub fn pipeline_ver(&self) -> u32 {
        self.ver
    }

    fn size_dir(&self, size: u32) -> PathBuf {
        self.root.join(size.to_string())
    }

    /// Caminho (existente ou nao) da miniatura.
    pub fn path_for(&self, hash: &ContentHash, size: u32) -> PathBuf {
        let hex = hash.to_hex();
        self.size_dir(size).join(&hex[..2]).join(format!("{hex}-v{}.png", self.ver))
    }

    /// Caminho se a miniatura existe em disco.
    pub fn cached_path(&self, hash: &ContentHash, size: u32) -> Option<PathBuf> {
        let p = self.path_for(hash, size);
        p.is_file().then_some(p)
    }

    fn ram_get(&self, hash: &ContentHash, size: u32) -> Option<Arc<RgbaImage>> {
        self.ram.lock().ok()?.lru.get(&(*hash, size)).cloned()
    }

    fn ram_put(&self, hash: ContentHash, size: u32, img: Arc<RgbaImage>) {
        let Ok(mut r) = self.ram.lock() else { return };
        let cost = img.as_raw().len();
        if cost > r.budget {
            return;
        }
        if let Some(old) = r.lru.put((hash, size), img) {
            r.bytes -= old.as_raw().len();
        }
        r.bytes += cost;
        while r.bytes > r.budget {
            match r.lru.pop_lru() {
                Some((_, v)) => r.bytes -= v.as_raw().len(),
                None => break,
            }
        }
    }

    /// Bytes atualmente na RAM.
    pub fn ram_bytes(&self) -> usize {
        self.ram.lock().map(|r| r.bytes).unwrap_or(0)
    }

    /// Itens na RAM.
    pub fn ram_len(&self) -> usize {
        self.ram.lock().map(|r| r.lru.len()).unwrap_or(0)
    }

    /// RAM -> disco (decodifica PNG). `None` se nao existe ou o arquivo esta corrompido (e o remove).
    pub fn load(&self, hash: &ContentHash, size: u32) -> Option<Arc<RgbaImage>> {
        if let Some(i) = self.ram_get(hash, size) {
            return Some(i);
        }
        let p = self.cached_path(hash, size)?;
        match image::open(&p) {
            Ok(img) => {
                let arc = Arc::new(img.to_rgba8());
                self.ram_put(*hash, size, arc.clone());
                Some(arc)
            }
            Err(_) => {
                let _ = std::fs::remove_file(&p);
                None
            }
        }
    }

    fn write_png(&self, path: &Path, img: &RgbaImage) -> Result<()> {
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| AssetError::Internal(format!("png: {e}")))?;
        atomic_write(path, buf.get_ref())
    }

    /// Grava o master (512) e deriva 256/128/64.
    pub fn store_master(&self, hash: &ContentHash, master: &RgbaImage) -> Result<()> {
        let master = if master.width() == MASTER_SIZE && master.height() == MASTER_SIZE { master.clone() } else { fit_square(master, MASTER_SIZE)? };
        for &s in THUMB_SIZES.iter().rev() {
            let img = if s == MASTER_SIZE { master.clone() } else { resize_rgba(&master, s, s)? };
            self.write_png(&self.path_for(hash, s), &img)?;
            self.ram_put(*hash, s, Arc::new(img));
        }
        Ok(())
    }

    /// Garante que todos os tamanhos existem em disco; renderiza se faltar.
    pub fn ensure(&self, req: &ThumbRequest, renderer: &dyn ThumbnailRenderer) -> Result<Ensured> {
        if THUMB_SIZES.iter().all(|s| self.cached_path(&req.content_hash, *s).is_some()) {
            return Ok(Ensured { cached: true, placeholder: false });
        }
        let mut req = req.clone();
        req.size = MASTER_SIZE;
        let rendered = crate::job::guard(|| renderer.render(&req).map_err(|e| AssetError::Internal(format!("{}: {e}", renderer.id()))))?;
        if rendered.image.width() == 0 || rendered.image.height() == 0 {
            return Err(AssetError::Internal("renderizador devolveu imagem vazia".into()));
        }
        if rendered.placeholder {
            return Ok(Ensured { cached: false, placeholder: true });
        }
        self.store_master(&req.content_hash, &rendered.image)?;
        Ok(Ensured { cached: false, placeholder: false })
    }

    /// Imagem para exibir AGORA: cache (RAM/disco) ou, se nao houver, o icone do tipo (sem disco).
    pub fn load_or_placeholder(&self, hash: &ContentHash, kind: AssetKind, size: u32) -> (Arc<RgbaImage>, bool) {
        match self.load(hash, size) {
            Some(i) => (i, false),
            None => (Arc::new(PlaceholderRenderer::draw(kind, size)), true),
        }
    }

    /// Remove tudo de versoes antigas e (se `keep` for dado) de hashes que nao estao em `keep`. Devolve bytes liberados.
    pub fn purge_stale(&self, keep: Option<&HashSet<ContentHash>>) -> u64 {
        let suffix = format!("-v{}.png", self.ver);
        let mut freed = 0u64;
        for e in walkdir::WalkDir::new(&self.root).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()) {
            let name = e.file_name().to_string_lossy().to_string();
            let stale_ver = !name.ends_with(&suffix);
            let orphan = keep
                .map(|k| name.get(..64).and_then(ContentHash::from_hex).map(|h| !k.contains(&h)).unwrap_or(true))
                .unwrap_or(false);
            if stale_ver || orphan {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if std::fs::remove_file(e.path()).is_ok() {
                    freed += len;
                }
            }
        }
        if let Ok(mut r) = self.ram.lock() {
            r.lru.clear();
            r.bytes = 0;
        }
        freed
    }

    /// Tamanho total em disco.
    pub fn disk_usage(&self) -> u64 {
        crate::util::dir_size(&self.root)
    }

    /// Remove os arquivos mais antigos (mtime) ate caber em `max_bytes`. Devolve bytes liberados.
    pub fn evict_to(&self, max_bytes: u64) -> u64 {
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path().to_path_buf())))
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= max_bytes {
            return 0;
        }
        files.sort_by_key(|f| f.0);
        let mut freed = 0;
        for (_, len, p) in files {
            if total <= max_bytes {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= len;
                freed += len;
            }
        }
        if let Ok(mut r) = self.ram.lock() {
            r.lru.clear();
            r.bytes = 0;
        }
        freed
    }

    /// Apaga todo o cache (disco e RAM).
    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
        if let Ok(mut r) = self.ram.lock() {
            r.lru.clear();
            r.bytes = 0;
        }
    }
}

/// Remove miniaturas de TODAS as versoes de um hash (usado no purge de ativos).
pub(crate) fn remove_cached(cache_dir: &Path, hash: &ContentHash) -> Result<()> {
    let hex = hash.to_hex();
    for s in THUMB_SIZES {
        let d = cache_dir.join("thumbs").join(s.to_string()).join(&hex[..2]);
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&hex) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Geracao em lote
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ThumbEvent {
    Started { total: usize },
    Done { id: AssetId, cached: bool, placeholder: bool },
    Failed { id: AssetId, error: String },
    Progress { done: usize, total: usize },
    Finished,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThumbReport {
    pub generated: usize,
    pub already_cached: usize,
    pub placeholders: usize,
    pub failed: Vec<(AssetId, String)>,
    pub cancelled: bool,
}

pub type ThumbJob = Job<ThumbEvent, ThumbReport>;

/// Opcoes de `generate_thumbnails`.
#[derive(Debug, Clone)]
pub struct ThumbOptions {
    /// Renderizacoes simultaneas. **Deixe 1 para renderizadores GPU** (um job de GPU por vez).
    pub parallelism: usize,
    /// Re-renderiza mesmo que o cache exista.
    pub force: bool,
}

impl Default for ThumbOptions {
    fn default() -> Self {
        Self { parallelism: 1, force: false }
    }
}

impl Library {
    /// Cache de miniaturas da biblioteca (compartilhado; orcamento de RAM de 128 MiB).
    pub fn thumbnails(&self) -> Arc<ThumbnailCache> {
        self.inner.thumbs.get_or_init(|| Arc::new(ThumbnailCache::new(&self.inner.cache_dir, self.inner.opts.pipeline_ver, 128 * 1024 * 1024))).clone()
    }

    /// Monta o pedido de miniatura de um ativo.
    pub fn thumb_request(&self, id: &AssetId) -> Result<ThumbRequest> {
        let row = self.require_row(id)?;
        let f = self.resolve_files(id)?;
        Ok(ThumbRequest {
            id: *id,
            kind: row.kind,
            format: row.format.clone(),
            content_hash: row.content_hash,
            primary: f.primary,
            files: f.files,
            asset_dir: f.asset_dir,
            size: MASTER_SIZE,
        })
    }

    /// Ids (ate `max`) que casam com `q` e ainda NAO tem miniatura em disco.
    pub fn assets_without_thumbnail(&self, q: &crate::query::Query, max: usize) -> Result<Vec<AssetId>> {
        let cache = self.thumbnails();
        let mut out = Vec::new();
        let mut q = q.clone();
        q.limit = 1000;
        loop {
            let page = self.query(&q)?;
            for r in &page.items {
                if cache.cached_path(&r.content_hash, 64).is_none() {
                    out.push(r.id);
                    if out.len() >= max {
                        return Ok(out);
                    }
                }
            }
            match page.next {
                Some(c) => q.after = Some(c),
                None => return Ok(out),
            }
        }
    }

    /// Gera miniaturas em background com `renderer`. Falha por ativo nao interrompe o lote.
    pub fn generate_thumbnails(&self, ids: Vec<AssetId>, renderer: Arc<dyn ThumbnailRenderer>, opts: ThumbOptions) -> ThumbJob {
        let lib = self.clone();
        Job::spawn("anigo-thumbs", move |ctx: JobCtx<ThumbEvent>| {
            let cache = lib.thumbnails();
            let total = ids.len();
            ctx.emit(ThumbEvent::Started { total });
            let mut rep = ThumbReport::default();
            let par = opts.parallelism.clamp(1, 16);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(par).build().map_err(|e| AssetError::Internal(e.to_string()))?;
            let mut done = 0usize;
            for chunk in ids.chunks(par * 4) {
                if ctx.cancel.is_cancelled() {
                    rep.cancelled = true;
                    break;
                }
                let results: Vec<(AssetId, Result<Ensured>)> = pool.install(|| {
                    chunk
                        .par_iter()
                        .map(|id| {
                            if ctx.cancel.is_cancelled() {
                                return (*id, Err(AssetError::Cancelled));
                            }
                            let r = lib.thumb_request(id).and_then(|req| {
                                if opts.force {
                                    let _ = remove_cached(lib.cache_dir(), &req.content_hash);
                                }
                                cache.ensure(&req, renderer.as_ref())
                            });
                            (*id, r)
                        })
                        .collect()
                });
                for (id, r) in results {
                    done += 1;
                    match r {
                        Ok(e) => {
                            if e.placeholder {
                                rep.placeholders += 1;
                            } else if e.cached {
                                rep.already_cached += 1;
                            } else {
                                rep.generated += 1;
                            }
                            ctx.emit(ThumbEvent::Done { id, cached: e.cached, placeholder: e.placeholder });
                        }
                        Err(AssetError::Cancelled) => rep.cancelled = true,
                        Err(e) => {
                            rep.failed.push((id, e.to_string()));
                            ctx.emit(ThumbEvent::Failed { id, error: e.to_string() });
                        }
                    }
                }
                ctx.emit(ThumbEvent::Progress { done, total });
            }
            ctx.emit(ThumbEvent::Finished);
            Ok(rep)
        })
    }
}
