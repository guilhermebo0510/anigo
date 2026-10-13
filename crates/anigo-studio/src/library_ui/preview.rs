//! Preview 3D girável do ativo selecionado (opcional): arrastar gira, rolar aproxima, "girar" liga o turntable.
//! Renderiza com o renderer cel canônico numa thread própria (device próprio), sempre só o pedido mais novo.

use super::cel_thumb::{self, CelThumbnailRenderer, PreparedMesh, ViewParams};
use super::*;
use crate::i18n::tr;
use crate::icons;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Ui, Vec2};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReqKey {
    id: AssetId,
    hash: ContentHash,
    yaw_q: i32,
    pitch_q: i32,
    zoom_q: i32,
    size: u32,
}

struct Req {
    key: ReqKey,
    path: PathBuf,
    format: String,
    asset_dir: PathBuf,
    view: ViewParams,
}

enum Resp {
    Image { key: ReqKey, image: image::RgbaImage },
    Error { key: ReqKey, message: String },
}

struct Svc {
    tx: Sender<Req>,
    rx: Receiver<Resp>,
}

impl Svc {
    fn spawn(wake: Wake) -> Self {
        let (tx, rx_req) = unbounded::<Req>();
        let (tx_resp, rx) = unbounded::<Resp>();
        std::thread::Builder::new()
            .name("anigo-lib-preview".into())
            .spawn(move || {
                let cel = CelThumbnailRenderer::new();
                let mut cached: Option<(PathBuf, ContentHash, PreparedMesh)> = None;
                while let Ok(mut req) = rx_req.recv() {
                    while let Ok(newer) = rx_req.try_recv() {
                        req = newer;
                    }
                    let need_load = cached.as_ref().is_none_or(|(p, h, _)| *p != req.path || *h != req.key.hash);
                    if need_load {
                        match cel_thumb::load_core_mesh(&req.path, &req.format).and_then(PreparedMesh::new) {
                            Ok(p) => cached = Some((req.path.clone(), req.key.hash, p)),
                            Err(e) => {
                                cached = None;
                                let _ = tx_resp.send(Resp::Error { key: req.key, message: e });
                                wake();
                                continue;
                            }
                        }
                    }
                    let (_, _, prep) = cached.as_ref().expect("malha carregada");
                    let material = cel_thumb::single_material(&req.asset_dir);
                    match cel.render_prepared(prep, req.view, material, req.key.size) {
                        Ok(image) => {
                            let _ = tx_resp.send(Resp::Image { key: req.key, image });
                        }
                        Err(e) => {
                            let _ = tx_resp.send(Resp::Error { key: req.key, message: e });
                        }
                    }
                    wake();
                }
            })
            .expect("thread de preview");
        Self { tx, rx }
    }
}

pub struct Preview3d {
    pub enabled: bool,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    auto: bool,
    svc: Option<Svc>,
    tex: Option<egui::TextureHandle>,
    shown: Option<ReqKey>,
    requested: Option<ReqKey>,
    error: Option<String>,
}

impl Default for Preview3d {
    fn default() -> Self {
        let v = ViewParams::default();
        Self { enabled: false, yaw: v.yaw, pitch: v.pitch, zoom: 1.0, auto: false, svc: None, tex: None, shown: None, requested: None, error: None }
    }
}

impl LibraryWorkspace {
    pub(super) fn preview_widget(&mut self, ui: &mut Ui, d: &DetailData, side: f32) {
        let ctx = ui.ctx().clone();
        let wake = self.wake.clone();
        let p = &mut self.preview3d;
        let svc = p.svc.get_or_insert_with(|| Svc::spawn(wake));
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
        if resp.dragged() {
            let dlt = resp.drag_delta();
            p.yaw -= dlt.x * 0.012;
            p.pitch = (p.pitch + dlt.y * 0.01).clamp(-1.35, 1.35);
        }
        if resp.hovered() {
            let s = ui.input(|i| i.smooth_scroll_delta.y);
            if s != 0.0 {
                p.zoom = (p.zoom * (1.0 + s * 0.002)).clamp(0.4, 5.0);
            }
        }
        if resp.double_clicked() {
            let v = ViewParams::default();
            (p.yaw, p.pitch, p.zoom) = (v.yaw, v.pitch, 1.0);
        }
        if p.auto && !resp.dragged() {
            p.yaw += ui.input(|i| i.stable_dt).min(0.1) * 0.7;
            ctx.request_repaint();
        }
        let ppp = ctx.pixels_per_point();
        let size = (((side * ppp) / 32.0).round() as u32 * 32).clamp(128, 512);
        let row = &d.detail.row;
        let key = ReqKey { id: row.id, hash: row.content_hash, yaw_q: (p.yaw * 200.0) as i32, pitch_q: (p.pitch * 200.0) as i32, zoom_q: (p.zoom * 100.0) as i32, size };
        if p.requested.as_ref() != Some(&key) {
            if let Some(f) = &d.files {
                let _ = svc.tx.send(Req { key: key.clone(), path: f.primary.clone(), format: row.format.clone(), asset_dir: f.asset_dir.clone(), view: ViewParams { yaw: p.yaw, pitch: p.pitch, zoom: p.zoom } });
                p.requested = Some(key.clone());
            }
        }
        for r in svc.rx.try_iter() {
            match r {
                Resp::Image { key, image } => {
                    let color = egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
                    // só aceita o resultado do ativo atual (descarta o que sobrou de seleções anteriores)
                    if key.id == row.id {
                        p.tex = Some(ctx.load_texture("preview3d", color, egui::TextureOptions::LINEAR));
                        p.shown = Some(key);
                        p.error = None;
                    }
                }
                Resp::Error { key, message } => {
                    if key.id == row.id {
                        p.error = Some(message);
                    }
                }
            }
        }
        ui.painter().rect_filled(rect, CornerRadius::same(6), theme::BG_DEEP);
        match (&p.tex, &p.error, &p.shown) {
            (_, Some(e), _) => {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, e.chars().take(120).collect::<String>(), egui::FontId::proportional(11.5), super::widgets::err_color());
            }
            (Some(t), None, Some(shown)) if shown.id == row.id => {
                ui.painter().image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                if p.requested.as_ref() != p.shown.as_ref() {
                    ui.painter().text(rect.right_top() + Vec2::new(-8.0, 10.0), egui::Align2::RIGHT_CENTER, icons::CIRCLE_NOTCH, egui::FontId::proportional(14.0), theme::TEXT_DIM);
                }
            }
            _ => {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tr("lib.det.preview_loading"), egui::FontId::proportional(12.0), theme::TEXT_DIM);
            }
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut p.auto, RichText::new(tr("lib.det.turntable")).size(11.5));
            ui.label(super::widgets::small_dim(tr("lib.det.preview_hint")));
        });
    }
}

impl Preview3d {
    /// Já existe uma imagem renderizada para o pedido mais recente (ou o renderer falhou).
    pub fn is_shown(&self) -> bool {
        self.error.is_some() || (self.shown.is_some() && self.shown == self.requested)
    }
}
