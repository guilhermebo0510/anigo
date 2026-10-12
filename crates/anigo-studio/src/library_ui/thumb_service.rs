//! Serviço de miniaturas por visibilidade: a UI declara, a cada quadro, quais miniaturas quer (as visíveis
//! primeiro); threads de fundo atendem na ordem, pelo `ThumbnailCache` do backend (RAM -> disco -> render).
//!
//! * `loaders` (2 threads): `ThumbnailCache::load` (decodifica PNG do cache de disco). Rápidas, paralelas.
//! * `render` (1 thread): `ThumbnailCache::ensure` com o `ThumbnailRenderer` (GPU cel -> CPU). UM job de GPU por vez.
//! * O que sai da lista de desejos (rolou para fora da tela) deixa de ser atendido: o cancelamento é implícito.
//! * Itens para os quais não há miniatura (ícone do tipo, erro) entram em `failed` e não são pedidos de novo.
//!
//! Do lado da UI, [`TexCache`] guarda as texturas egui em LRU por orçamento de bytes.

use anigo_assets::{AssetId, ContentHash, Library, ThumbnailCache, ThumbnailRenderer};
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::RgbaImage;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Tamanho de cache (lado) mais próximo de `px` pixels físicos.
pub fn bucket(px: f32) -> u32 {
    if px <= 64.0 {
        64
    } else if px <= 128.0 {
        128
    } else if px <= 256.0 {
        256
    } else {
        512
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbReq {
    pub id: AssetId,
    pub hash: ContentHash,
    pub size: u32,
}

pub enum ThumbMsg {
    Ready { hash: ContentHash, size: u32, image: Arc<RgbaImage> },
    /// Sem miniatura para este conteúdo (a UI desenha o ícone do tipo).
    Unavailable { hash: ContentHash, reason: String },
}

#[derive(Default)]
struct State {
    wanted: Vec<ThumbReq>,
    sig: u64,
    need_render: HashSet<ContentHash>,
    inflight: HashSet<(ContentHash, u32)>,
    rendering: Option<ContentHash>,
    failed: HashSet<ContentHash>,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    stop: AtomicBool,
    rendered: AtomicU64,
    loaded: AtomicU64,
    failed: AtomicU64,
}

pub struct ThumbService {
    shared: Arc<Shared>,
    pub rx: Receiver<ThumbMsg>,
    pub cache: Arc<ThumbnailCache>,
    renderer: Arc<dyn ThumbnailRenderer>,
}

impl ThumbService {
    pub fn spawn(lib: Library, renderer: Arc<dyn ThumbnailRenderer>, wake: super::data::Wake) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            rendered: AtomicU64::new(0),
            loaded: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        });
        let (tx, rx) = unbounded::<ThumbMsg>();
        let cache = lib.thumbnails();
        for n in 0..2 {
            let (shared, cache, tx, wake) = (shared.clone(), cache.clone(), tx.clone(), wake.clone());
            std::thread::Builder::new().name(format!("anigo-thumb-load-{n}")).spawn(move || loader_loop(shared, cache, tx, wake)).expect("thread de miniaturas");
        }
        {
            let (shared, cache, tx, wake, renderer, lib) = (shared.clone(), cache.clone(), tx, wake, renderer.clone(), lib);
            std::thread::Builder::new().name("anigo-thumb-render".into()).spawn(move || render_loop(shared, cache, renderer, lib, tx, wake)).expect("thread de render");
        }
        Self { shared, rx, cache, renderer }
    }

    pub fn renderer_id(&self) -> String {
        self.renderer.id().to_string()
    }

    /// Substitui a lista de desejos (ordem = prioridade). `sig` identifica o conteúdo: igual => ignora.
    pub fn set_wanted(&self, wanted: Vec<ThumbReq>, sig: u64) {
        let Ok(mut st) = self.shared.state.lock() else { return };
        if st.sig == sig {
            return;
        }
        st.sig = sig;
        st.wanted = wanted;
        drop(st);
        self.shared.cv.notify_all();
    }

    /// Quantas miniaturas da lista de desejos ainda estão na fila de render 3D.
    pub fn render_queue_len(&self) -> usize {
        let Ok(st) = self.shared.state.lock() else { return 0 };
        let mut seen = HashSet::new();
        st.wanted.iter().filter(|r| st.need_render.contains(&r.hash) && seen.insert(r.hash)).count()
    }

    pub fn is_idle(&self) -> bool {
        let Ok(st) = self.shared.state.lock() else { return true };
        st.inflight.is_empty() && st.rendering.is_none() && !st.wanted.iter().any(|r| !st.failed.contains(&r.hash) && !st.need_render.contains(&r.hash)) && !st.wanted.iter().any(|r| st.need_render.contains(&r.hash))
    }

    pub fn stats(&self) -> (u64, u64, u64) {
        (self.shared.loaded.load(Ordering::Relaxed), self.shared.rendered.load(Ordering::Relaxed), self.shared.failed.load(Ordering::Relaxed))
    }

    /// Esquece as falhas (depois de limpar o cache ou trocar de renderer).
    pub fn reset_failures(&self) {
        if let Ok(mut st) = self.shared.state.lock() {
            st.failed.clear();
            st.need_render.clear();
            st.sig = 0;
        }
        self.shared.cv.notify_all();
    }

    /// Pede de novo as miniaturas de um conteúdo (ex.: depois de uma revisão nova).
    pub fn forget(&self, hash: &ContentHash) {
        if let Ok(mut st) = self.shared.state.lock() {
            st.failed.remove(hash);
            st.need_render.remove(hash);
            st.sig = 0;
        }
        self.shared.cv.notify_all();
    }
}

impl Drop for ThumbService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}

fn loader_loop(shared: Arc<Shared>, cache: Arc<ThumbnailCache>, tx: Sender<ThumbMsg>, wake: super::data::Wake) {
    loop {
        let req = {
            let Ok(mut st) = shared.state.lock() else { return };
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                let pick = st
                    .wanted
                    .iter()
                    .find(|r| !st.inflight.contains(&(r.hash, r.size)) && !st.need_render.contains(&r.hash) && !st.failed.contains(&r.hash) && st.rendering != Some(r.hash))
                    .cloned();
                if let Some(r) = pick {
                    st.inflight.insert((r.hash, r.size));
                    break r;
                }
                st = match shared.cv.wait(st) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let img = cache.load(&req.hash, req.size);
        let mut notify = false;
        if let Ok(mut st) = shared.state.lock() {
            st.inflight.remove(&(req.hash, req.size));
            match &img {
                Some(_) => {}
                None => {
                    st.need_render.insert(req.hash);
                    notify = true;
                }
            }
        }
        if let Some(image) = img {
            shared.loaded.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(ThumbMsg::Ready { hash: req.hash, size: req.size, image });
            wake();
        }
        if notify {
            shared.cv.notify_all();
        }
    }
}

fn render_loop(shared: Arc<Shared>, cache: Arc<ThumbnailCache>, renderer: Arc<dyn ThumbnailRenderer>, lib: Library, tx: Sender<ThumbMsg>, wake: super::data::Wake) {
    loop {
        let req = {
            let Ok(mut st) = shared.state.lock() else { return };
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                let pick = st.wanted.iter().find(|r| st.need_render.contains(&r.hash) && !st.failed.contains(&r.hash)).cloned();
                if let Some(r) = pick {
                    st.rendering = Some(r.hash);
                    break r;
                }
                st = match shared.cv.wait(st) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let outcome = lib.thumb_request(&req.id).map_err(|e| e.to_string()).and_then(|tr| cache.ensure(&tr, renderer.as_ref()).map_err(|e| e.to_string()));
        let mut msgs = Vec::new();
        if let Ok(mut st) = shared.state.lock() {
            st.rendering = None;
            st.need_render.remove(&req.hash);
            match outcome {
                Ok(e) if !e.placeholder => {
                    // as miniaturas agora existem em disco: o loader as lê no próximo ciclo
                    if !e.cached {
                        shared.rendered.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(_) => {
                    st.failed.insert(req.hash);
                    shared.failed.fetch_add(1, Ordering::Relaxed);
                    msgs.push(ThumbMsg::Unavailable { hash: req.hash, reason: "sem miniatura para este tipo".into() });
                }
                Err(e) => {
                    st.failed.insert(req.hash);
                    shared.failed.fetch_add(1, Ordering::Relaxed);
                    msgs.push(ThumbMsg::Unavailable { hash: req.hash, reason: e });
                }
            }
        }
        for m in msgs {
            let _ = tx.send(m);
        }
        shared.cv.notify_all();
        wake();
    }
}

// ---------------------------------------------------------------------------------------------
// Texturas egui em LRU
// ---------------------------------------------------------------------------------------------

/// Texturas da janela visível, com descarte do menos usado por orçamento de bytes.
pub struct TexCache {
    map: lru::LruCache<(ContentHash, u32), (egui::TextureHandle, usize)>,
    bytes: usize,
    budget: usize,
    /// Hashes sem miniatura (ícone do tipo).
    pub unavailable: HashSet<ContentHash>,
}

impl TexCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self { map: lru::LruCache::unbounded(), bytes: 0, budget: budget_bytes, unavailable: HashSet::new() }
    }

    pub fn get(&mut self, hash: &ContentHash, size: u32) -> Option<&egui::TextureHandle> {
        self.map.get(&(*hash, size)).map(|(t, _)| t)
    }

    pub fn contains(&self, hash: &ContentHash, size: u32) -> bool {
        self.map.contains(&(*hash, size))
    }

    /// Maior tamanho já carregado para o hash (para desenhar algo enquanto o tamanho exato carrega).
    pub fn best_available(&mut self, hash: &ContentHash) -> Option<&egui::TextureHandle> {
        for s in [64, 128, 256, 512] {
            if self.map.contains(&(*hash, s)) {
                return self.map.get(&(*hash, s)).map(|(t, _)| t);
            }
        }
        None
    }

    pub fn insert(&mut self, ctx: &egui::Context, hash: ContentHash, size: u32, image: &RgbaImage) {
        let color = egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
        let tex = ctx.load_texture(format!("thumb-{}-{size}", hash.short()), color, egui::TextureOptions::LINEAR);
        let cost = image.as_raw().len();
        if let Some((_, old)) = self.map.put((hash, size), (tex, cost)) {
            self.bytes -= old;
        }
        self.bytes += cost;
        while self.bytes > self.budget {
            match self.map.pop_lru() {
                Some((_, (_, c))) => self.bytes -= c,
                None => break,
            }
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
        self.unavailable.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_assets::{default_cpu_renderer, AssetKind, ImportOptions};
    use std::time::{Duration, Instant};

    #[test]
    fn bucket_picks_the_nearest_cache_size_above() {
        assert_eq!(bucket(40.0), 64);
        assert_eq!(bucket(64.0), 64);
        assert_eq!(bucket(100.0), 128);
        assert_eq!(bucket(200.0), 256);
        assert_eq!(bucket(900.0), 512);
    }

    fn wait_for<F: FnMut() -> bool>(mut f: F) {
        let end = Instant::now() + Duration::from_secs(20);
        while !f() {
            assert!(Instant::now() < end, "tempo esgotado esperando o serviço de miniaturas");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn service_renders_png_thumbnails_then_serves_them_from_cache() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().join("lib")).unwrap();
        let png = dir.path().join("a.png");
        image::RgbaImage::from_pixel(40, 40, image::Rgba([200, 30, 30, 255])).save(&png).unwrap();
        let rep = lib.import_blocking(&png, ImportOptions::default()).unwrap();
        let id = rep.imported[0];
        let row = lib.require_row(&id).unwrap();
        let svc = ThumbService::spawn(lib.clone(), default_cpu_renderer(), Arc::new(|| {}));
        let req = ThumbReq { id, hash: row.content_hash, size: 128 };
        svc.set_wanted(vec![req.clone()], 1);
        let mut got = None;
        wait_for(|| {
            for m in svc.rx.try_iter() {
                if let ThumbMsg::Ready { hash, size, image } = m {
                    assert_eq!((hash, size), (req.hash, 128));
                    got = Some(image);
                }
            }
            got.is_some()
        });
        let img = got.unwrap();
        assert_eq!((img.width(), img.height()), (128, 128));
        assert_eq!(img.get_pixel(64, 64).0[0], 200, "cor da imagem real, não um ícone");
        assert!(svc.cache.cached_path(&row.content_hash, 128).is_some(), "ficou no cache de disco");
        let (_, rendered, _) = svc.stats();
        assert_eq!(rendered, 1);
        // segunda rodada: vem do cache, sem render novo
        svc.set_wanted(vec![ThumbReq { size: 64, ..req }], 2);
        let mut again = false;
        wait_for(|| {
            again |= svc.rx.try_iter().any(|m| matches!(m, ThumbMsg::Ready { size: 64, .. }));
            again
        });
        assert_eq!(svc.stats().1, 1, "nenhum render adicional");
    }

    #[test]
    fn items_without_a_thumbnail_are_reported_once_and_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let lib = crate::library_ui::synth::build_synthetic(dir.path(), 3, 5).unwrap();
        let rows = lib.query(&anigo_assets::Query::default()).unwrap().items;
        let svc = ThumbService::spawn(lib, default_cpu_renderer(), Arc::new(|| {}));
        let wanted: Vec<_> = rows.iter().map(|r| ThumbReq { id: r.id, hash: r.content_hash, size: 128 }).collect();
        svc.set_wanted(wanted, 1);
        let mut unavailable = 0;
        wait_for(|| {
            unavailable += svc.rx.try_iter().filter(|m| matches!(m, ThumbMsg::Unavailable { .. })).count();
            unavailable >= rows.len()
        });
        std::thread::sleep(Duration::from_millis(100));
        unavailable += svc.rx.try_iter().filter(|m| matches!(m, ThumbMsg::Unavailable { .. })).count();
        assert_eq!(unavailable, rows.len(), "uma notificação por conteúdo, sem repetição");
        assert!(rows.iter().any(|r| r.kind != AssetKind::Texture));
    }

    #[test]
    fn tex_cache_evicts_least_recently_used_by_bytes() {
        let ctx = egui::Context::default();
        let img = RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]));
        let one = 16 * 16 * 4;
        let mut c = TexCache::new(one * 3);
        let h = |n: u8| ContentHash([n; 32]);
        for n in 0..3 {
            c.insert(&ctx, h(n), 64, &img);
        }
        assert_eq!(c.len(), 3);
        // toca no 0 (fica recente) e insere um quarto: sai o 1
        assert!(c.get(&h(0), 64).is_some());
        c.insert(&ctx, h(9), 64, &img);
        assert_eq!(c.len(), 3);
        assert!(c.contains(&h(0), 64) && c.contains(&h(9), 64) && !c.contains(&h(1), 64));
        assert!(c.bytes() <= one * 3);
    }
}
