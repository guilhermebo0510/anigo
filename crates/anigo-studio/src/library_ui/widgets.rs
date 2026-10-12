//! Peças visuais compartilhadas da Biblioteca: ícones por tipo, estrelas, formatação de números.

use crate::icons;
use crate::theme;
use anigo_assets::AssetKind;
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};

pub fn kind_icon(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Model => icons::CUBE,
        AssetKind::Character => icons::USER,
        AssetKind::Outfit => icons::T_SHIRT,
        AssetKind::Hair => icons::SCISSORS,
        AssetKind::Accessory => icons::SPARKLE,
        AssetKind::Material => icons::PALETTE,
        AssetKind::Texture => icons::IMAGE,
        AssetKind::Pose => icons::PERSON_SIMPLE_RUN,
        AssetKind::Animation => icons::FILM_STRIP,
        AssetKind::Hdri => icons::CLOUD,
        AssetKind::Scene => icons::MOUNTAINS,
        AssetKind::Prefab => icons::PACKAGE,
    }
}

pub fn kind_color(k: AssetKind) -> Color32 {
    match k {
        AssetKind::Model => Color32::from_rgb(0x8a, 0x9b, 0xff),
        AssetKind::Character => Color32::from_rgb(0xff, 0x8a, 0xb4),
        AssetKind::Outfit => Color32::from_rgb(0x6c, 0xd0, 0xc0),
        AssetKind::Hair => Color32::from_rgb(0xe0, 0x9a, 0xff),
        AssetKind::Accessory => Color32::from_rgb(0xff, 0xc8, 0x6c),
        AssetKind::Material => Color32::from_rgb(0xff, 0x9a, 0x6c),
        AssetKind::Texture => Color32::from_rgb(0x7c, 0xd0, 0x7c),
        AssetKind::Pose => Color32::from_rgb(0x6c, 0xb8, 0xff),
        AssetKind::Animation => Color32::from_rgb(0xff, 0x7c, 0x7c),
        AssetKind::Hdri => Color32::from_rgb(0x9c, 0xc8, 0xe8),
        AssetKind::Scene => Color32::from_rgb(0xa8, 0xd0, 0x6c),
        AssetKind::Prefab => Color32::from_rgb(0xc8, 0xc8, 0x8a),
    }
}

pub fn kind_label(k: AssetKind) -> &'static str {
    crate::i18n::tr(match k {
        AssetKind::Model => "lib.kind.model",
        AssetKind::Character => "lib.kind.character",
        AssetKind::Outfit => "lib.kind.outfit",
        AssetKind::Hair => "lib.kind.hair",
        AssetKind::Accessory => "lib.kind.accessory",
        AssetKind::Material => "lib.kind.material",
        AssetKind::Texture => "lib.kind.texture",
        AssetKind::Pose => "lib.kind.pose",
        AssetKind::Animation => "lib.kind.animation",
        AssetKind::Hdri => "lib.kind.hdri",
        AssetKind::Scene => "lib.kind.scene",
        AssetKind::Prefab => "lib.kind.prefab",
    })
}

/// Tipos que a Modelagem consegue abrir como malha editável.
pub fn editable_in_modeling(kind: AssetKind, format: &str) -> bool {
    let mesh_kind = matches!(kind, AssetKind::Model | AssetKind::Character | AssetKind::Outfit | AssetKind::Hair | AssetKind::Accessory | AssetKind::Prefab | AssetKind::Scene);
    mesh_kind && matches!(format.to_ascii_lowercase().as_str(), "glb" | "vrm" | "obj" | "fbx")
}

pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// 1234567 -> "1 234 567" (espaço fino, legível em colunas).
pub fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

/// "12,3 mil" / "1,2 mi" para tris em cartões estreitos.
pub fn fmt_short(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} mi", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{:.0} mil", n as f64 / 1e3)
    } else if n >= 1_000 {
        format!("{:.1} mil", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

pub fn fmt_time(secs: i64) -> String {
    // dd/mm/aaaa hh:mm em UTC (sem dependência de fuso/crates de data)
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}/{m:02}/{y} {:02}:{:02} UTC", rem / 3600, (rem % 3600) / 60)
}

// algoritmo de Howard Hinnant (dias desde 1970-01-01 -> data civil)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Família Phosphor "fill" (instalada em `icons::install`).
pub fn fill_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("phosphor-fill".into()))
}

/// Estrela de 5 pontas preenchida/contornada, desenhada com o painter (sem depender de fonte).
pub fn paint_star(p: &Painter, c: Pos2, r: f32, filled: bool, color: Color32) {
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rr = if i % 2 == 0 { r } else { r * 0.45 };
        pts.push(Pos2::new(c.x + rr * a.cos(), c.y + rr * a.sin()));
    }
    if filled {
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    } else {
        p.add(egui::Shape::closed_line(pts, Stroke::new(1.0, color)));
    }
}

/// Linha de 5 estrelas clicável. Devolve a nova nota se o usuário clicou (clicar na nota atual zera).
pub fn rating_row(ui: &mut Ui, id: egui::Id, rect: Rect, rating: u8, enabled: bool, show_empty: bool) -> Option<u8> {
    let step = rect.width() / 5.0;
    let r = (step * 0.42).min(rect.height() * 0.5);
    let mut out = None;
    let hover = if enabled { ui.input(|i| i.pointer.hover_pos()) } else { None };
    let hovered_star = hover.filter(|p| rect.contains(*p)).map(|p| (((p.x - rect.left()) / step).floor() as i32 + 1).clamp(1, 5) as u8);
    for i in 0..5u8 {
        let c = Pos2::new(rect.left() + step * (i as f32 + 0.5), rect.center().y);
        let shown = hovered_star.unwrap_or(rating);
        let on = i < shown;
        if !on && !show_empty && hovered_star.is_none() {
            continue;
        }
        let col = if on { Color32::from_rgb(0xff, 0xc8, 0x3d) } else { theme::TEXT_DIM };
        paint_star(ui.painter(), c, r, on, col);
    }
    if enabled {
        let resp = ui.interact(rect, id, Sense::click());
        if resp.clicked() {
            if let Some(n) = hovered_star {
                out = Some(if n == rating { 0 } else { n });
            }
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    out
}

/// Botão de ícone pequeno com dica.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(icon).size(15.0)).min_size(Vec2::new(24.0, 22.0)).frame(false)).on_hover_text(tip)
}

/// Botão de ícone pequeno desativável.
pub fn icon_button_enabled(ui: &mut Ui, enabled: bool, icon: &str, tip: &str) -> Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(icon).size(15.0)).min_size(Vec2::new(24.0, 22.0)).frame(false)).on_hover_text(tip)
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::TEXT_DIM)
}

pub fn small_dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::TEXT_DIM).size(11.5)
}

pub fn err_color() -> Color32 {
    Color32::from_rgb(0xff, 0x70, 0x70)
}

pub fn warn_color() -> Color32 {
    Color32::from_rgb(0xff, 0xc0, 0x4d)
}

pub fn ok_color() -> Color32 {
    Color32::from_rgb(0x6c, 0xd0, 0x8c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KiB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(fmt_int(1234567), "1\u{2009}234\u{2009}567");
        assert_eq!(fmt_int(12), "12");
        assert_eq!(fmt_short(999), "999");
        assert_eq!(fmt_short(1500), "1.5 mil");
        assert_eq!(fmt_short(250_000), "250 mil");
        assert_eq!(fmt_short(2_400_000), "2.4 mi");
    }

    #[test]
    fn civil_dates_match_known_epochs() {
        assert_eq!(fmt_time(0), "01/01/1970 00:00 UTC");
        assert_eq!(fmt_time(1_700_000_000), "14/11/2023 22:13 UTC");
        assert_eq!(fmt_time(951_782_400), "29/02/2000 00:00 UTC");
    }

    #[test]
    fn only_mesh_formats_open_in_modeling() {
        assert!(editable_in_modeling(AssetKind::Model, "glb"));
        assert!(editable_in_modeling(AssetKind::Character, "VRM"));
        assert!(!editable_in_modeling(AssetKind::Texture, "png"));
        assert!(!editable_in_modeling(AssetKind::Material, "anigomat"));
        assert!(!editable_in_modeling(AssetKind::Model, "gltf"));
    }
}
