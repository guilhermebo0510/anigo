//! Identidade visual do ANIGO dentro do app: ícone da janela, marca e lockup (texturas) e splash.
//!
//! Os arquivos vêm de `brand/gen_brand.py` (geometria vetorial própria; nada de fonte ou imagem de terceiros) e são copiados para
//! `assets/brand/` — o que está embutido aqui é exatamente o que a documentação de marca descreve. Dois conjuntos de cores:
//! `dark` (para a interface escura de hoje) e `light` (reservado ao modo claro que virá).

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};
use std::time::{Duration, Instant};

const ICON_256: &[u8] = include_bytes!("../assets/brand/icon_256.png");
const MARK_DARK: &[u8] = include_bytes!("../assets/brand/mark_dark_256.png");
const MARK_LIGHT: &[u8] = include_bytes!("../assets/brand/mark_light_256.png");
const LOCKUP_DARK_48: &[u8] = include_bytes!("../assets/brand/lockup_dark_h48.png");
const LOCKUP_LIGHT_48: &[u8] = include_bytes!("../assets/brand/lockup_light_h48.png");
const LOCKUP_DARK_96: &[u8] = include_bytes!("../assets/brand/lockup_dark_h96.png");
const LOCKUP_LIGHT_96: &[u8] = include_bytes!("../assets/brand/lockup_light_h96.png");
const LOCKUP_DARK_192: &[u8] = include_bytes!("../assets/brand/lockup_dark_h192.png");
const LOCKUP_LIGHT_192: &[u8] = include_bytes!("../assets/brand/lockup_light_h192.png");
const SPLASH_DARK: &[u8] = include_bytes!("../assets/brand/splash_dark.png");
const SPLASH_LIGHT: &[u8] = include_bytes!("../assets/brand/splash_light.png");

/// Ícone da janela (barra de tarefas, Alt+Tab, canto da janela). O ícone do arquivo `.exe` (atalho, Explorer) é embutido no
/// build pelo `build.rs`.
pub fn window_icon() -> egui::IconData {
    match image::load_from_memory(ICON_256) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
        }
        Err(e) => {
            tracing::warn!("ícone da janela inválido: {e}");
            egui::IconData::default()
        }
    }
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// Qual arte usar (a interface é escura hoje; o conjunto claro já existe para o modo claro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    pub fn of(ctx: &Context) -> Variant {
        if ctx.theme() == egui::Theme::Dark {
            Variant::Dark
        } else {
            Variant::Light
        }
    }
}

#[derive(Default)]
pub struct Brand {
    mark: [Option<TextureHandle>; 2],
    lockup48: [Option<TextureHandle>; 2],
    lockup96: [Option<TextureHandle>; 2],
    lockup192: [Option<TextureHandle>; 2],
    splash: [Option<TextureHandle>; 2],
}

fn slot(v: Variant) -> usize {
    match v {
        Variant::Dark => 0,
        Variant::Light => 1,
    }
}

fn load(cell: &mut Option<TextureHandle>, ctx: &Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    if cell.is_none() {
        if let Some(img) = decode(bytes) {
            *cell = Some(ctx.load_texture(name, img, TextureOptions::LINEAR));
        }
    }
    cell.clone()
}

impl Brand {
    pub fn mark(&mut self, ctx: &Context, v: Variant) -> Option<TextureHandle> {
        let bytes = if v == Variant::Dark { MARK_DARK } else { MARK_LIGHT };
        load(&mut self.mark[slot(v)], ctx, "brand-mark", bytes)
    }

    /// Lockup horizontal (marca + nome) na resolução mais próxima do tamanho pedido em pixels físicos de altura.
    pub fn lockup(&mut self, ctx: &Context, v: Variant, height_px: f32) -> Option<TextureHandle> {
        let i = slot(v);
        let dark = v == Variant::Dark;
        if height_px <= 60.0 {
            load(&mut self.lockup48[i], ctx, "brand-lockup-48", if dark { LOCKUP_DARK_48 } else { LOCKUP_LIGHT_48 })
        } else if height_px <= 120.0 {
            load(&mut self.lockup96[i], ctx, "brand-lockup-96", if dark { LOCKUP_DARK_96 } else { LOCKUP_LIGHT_96 })
        } else {
            load(&mut self.lockup192[i], ctx, "brand-lockup-192", if dark { LOCKUP_DARK_192 } else { LOCKUP_LIGHT_192 })
        }
    }

    pub fn splash(&mut self, ctx: &Context, v: Variant) -> Option<TextureHandle> {
        let bytes = if v == Variant::Dark { SPLASH_DARK } else { SPLASH_LIGHT };
        load(&mut self.splash[slot(v)], ctx, "brand-splash", bytes)
    }
}

/// Proporção largura/altura de um lockup (as três resoluções compartilham a mesma geometria).
pub fn lockup_aspect(tex: &TextureHandle) -> f32 {
    let s = tex.size_vec2();
    s.x / s.y.max(1.0)
}

/// Como o splash se comporta (`ANIGO_SPLASH`: `1` força mesmo com bridge/captura; `hold` espera um clique; `0` desliga).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashMode {
    Off,
    Timed,
    Hold,
}

pub fn splash_mode(automated: bool) -> SplashMode {
    match std::env::var("ANIGO_SPLASH").ok().as_deref() {
        Some("0") => SplashMode::Off,
        Some("hold") => SplashMode::Hold,
        Some("1") => SplashMode::Timed,
        _ if automated => SplashMode::Off,
        _ => SplashMode::Timed,
    }
}

/// Tela de abertura dentro da janela: a arte da marca, versão e uma barra indeterminada enquanto a GPU/biblioteca sobem.
/// Some sozinha (ou ao clique/tecla) e nunca bloqueia o carregamento — é só uma camada por cima da interface.
pub struct Splash {
    mode: SplashMode,
    started: Option<Instant>,
    done: bool,
}

const HOLD: Duration = Duration::from_millis(1300);
const FADE: Duration = Duration::from_millis(450);

impl Splash {
    pub fn new(mode: SplashMode) -> Splash {
        Splash { mode, started: None, done: mode == SplashMode::Off }
    }

    pub fn active(&self) -> bool {
        !self.done
    }

    /// Desenha a camada; chamar no fim do quadro.
    pub fn show(&mut self, ctx: &Context, brand: &mut Brand, status: &str, version: &str) {
        if self.done {
            return;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        let elapsed = started.elapsed();
        let skip = elapsed > Duration::from_millis(250)
            && ctx.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        let fade_start = HOLD;
        let alpha = match self.mode {
            SplashMode::Hold if !skip => 1.0,
            _ if skip => 0.0,
            _ => {
                if elapsed <= fade_start {
                    1.0
                } else {
                    1.0 - ((elapsed - fade_start).as_secs_f32() / FADE.as_secs_f32()).min(1.0)
                }
            }
        };
        if alpha <= 0.0 {
            self.done = true;
            ctx.request_repaint();
            return;
        }
        ctx.request_repaint();

        let variant = Variant::of(ctx);
        let screen = ctx.content_rect();
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("anigo-splash"));
        let a = (alpha * 255.0) as u8;
        // captura o ponteiro: nada abaixo do splash recebe clique enquanto ele está visível
        egui::Area::new(egui::Id::new("anigo-splash-area")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
            ui.allocate_rect(screen, egui::Sense::click_and_drag());
        });
        let painter = ctx.layer_painter(layer);
        let bg = if variant == Variant::Dark { Color32::from_rgb(0x0f, 0x0c, 0x1b) } else { Color32::from_rgb(0xf6, 0xf4, 0xff) };
        painter.rect_filled(screen, 0.0, bg.gamma_multiply(alpha));
        if let Some(tex) = brand.splash(ctx, variant) {
            // "cover" centralizado: a arte é 16:9; a janela pode ser outra proporção
            let t = tex.size_vec2();
            let s = (screen.width() / t.x).max(screen.height() / t.y);
            let size = t * s;
            let rect = egui::Rect::from_center_size(screen.center(), size);
            painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(a));
        }
        let dim = if variant == Variant::Dark { Color32::from_rgb(0x9a, 0x94, 0xb8) } else { Color32::from_rgb(0x5d, 0x55, 0x86) };
        let accent = crate::theme::ACCENT;
        // linha de progresso indeterminada logo abaixo do nome
        let w = (screen.width() * 0.22).clamp(160.0, 360.0);
        let y = screen.center().y + screen.height() * 0.36;
        let track = egui::Rect::from_center_size(egui::pos2(screen.center().x, y), egui::vec2(w, 2.0));
        painter.rect_filled(track, 1.0, dim.gamma_multiply(0.25 * alpha));
        let t = (started.elapsed().as_secs_f32() * 0.9).fract();
        let seg = w * 0.32;
        let x0 = track.left() + (w + seg) * t - seg;
        let bar = egui::Rect::from_min_max(egui::pos2(x0.max(track.left()), track.top()), egui::pos2((x0 + seg).min(track.right()), track.bottom()));
        if bar.width() > 0.0 {
            painter.rect_filled(bar, 1.0, accent.gamma_multiply(alpha));
        }
        painter.text(egui::pos2(screen.center().x, y + 18.0), egui::Align2::CENTER_CENTER, status, egui::FontId::proportional(12.5), dim.gamma_multiply(alpha));
        painter.text(egui::pos2(screen.right() - 16.0, screen.bottom() - 14.0), egui::Align2::RIGHT_CENTER, version, egui::FontId::proportional(11.5), dim.gamma_multiply(alpha));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_asset_decodes() {
        for (name, bytes) in [
            ("icon", ICON_256),
            ("mark dark", MARK_DARK),
            ("mark light", MARK_LIGHT),
            ("lockup dark 48", LOCKUP_DARK_48),
            ("lockup light 48", LOCKUP_LIGHT_48),
            ("lockup dark 96", LOCKUP_DARK_96),
            ("lockup light 96", LOCKUP_LIGHT_96),
            ("lockup dark 192", LOCKUP_DARK_192),
            ("lockup light 192", LOCKUP_LIGHT_192),
            ("splash dark", SPLASH_DARK),
            ("splash light", SPLASH_LIGHT),
        ] {
            let img = decode(bytes).unwrap_or_else(|| panic!("{name} não decodifica"));
            assert!(img.size[0] > 8 && img.size[1] > 8, "{name} vazio");
        }
    }

    #[test]
    fn window_icon_is_a_256_square() {
        let ic = window_icon();
        assert_eq!((ic.width, ic.height), (256, 256));
        assert_eq!(ic.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn splash_mode_follows_the_environment_contract() {
        // sem a variável: automação (bridge/captura) desliga, uso normal liga
        std::env::remove_var("ANIGO_SPLASH");
        assert_eq!(splash_mode(true), SplashMode::Off);
        assert_eq!(splash_mode(false), SplashMode::Timed);
    }

    #[test]
    fn lockups_keep_the_same_proportion_across_resolutions() {
        let a = |b: &[u8]| {
            let i = decode(b).unwrap();
            i.size[0] as f32 / i.size[1] as f32
        };
        let (x, y, z) = (a(LOCKUP_DARK_48), a(LOCKUP_DARK_96), a(LOCKUP_DARK_192));
        assert!((x - y).abs() < 0.05 && (y - z).abs() < 0.05, "{x} {y} {z}");
    }
}
