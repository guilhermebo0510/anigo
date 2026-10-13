//! Editor de gradiente da rampa 2D por linha: paradas (posição = half-lambert deslocado pelo limiar,
//! valor = fator de luz) arrastáveis num gráfico, interpolação linear/degraus, 4 linhas.
//!
//! O resultado vai em `StylizedMaterial::npr.ramp_custom`; o renderer monta a textura 256×4 do material
//! (as linhas sem gradiente próprio continuam as do contrato).

use crate::i18n::tr;
use crate::icons;
use crate::theme;
use anigo_core::npr::{RampCustom, RAMP_MAX_STOPS};
use anigo_core::StylizedMaterial;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke};

/// Resultado de um quadro do editor.
pub(crate) enum RampEdit {
    /// Nada mudou.
    None,
    /// Arrasto em andamento (pré-visualização; o passo de histórico sai ao soltar).
    Live(Box<StylizedMaterial>),
    /// Mudança discreta (um passo de histórico agora).
    Apply(Box<StylizedMaterial>),
}

const PLOT_H: f32 = 132.0;
const MIN_GAP: f32 = 0.004;

/// Linha em edição a partir de `ramp_row` (fração arredondada).
pub(crate) fn editing_row(m: &StylizedMaterial) -> usize {
    (m.npr.ramp_row.round().clamp(0.0, 3.0)) as usize
}

fn curve_points(c: &RampCustom, row: usize, plot: Rect) -> Vec<Pos2> {
    (0..=96)
        .map(|i| {
            let u = i as f32 / 96.0;
            let v = c.eval(row, u).unwrap_or(0.0);
            Pos2::new(plot.min.x + u * plot.width(), plot.max.y - v * plot.height())
        })
        .collect()
}

/// Desenha o editor para a linha `row` e devolve a edição.
pub(crate) fn ramp_editor(ui: &mut egui::Ui, m: &StylizedMaterial, sel: &mut Option<usize>) -> RampEdit {
    let row = editing_row(m);
    let custom = m.npr.ramp_custom;
    let is_custom = custom.is_custom(row);
    let mut out = RampEdit::None;

    // ---- linha em edição + iniciar / voltar ao contrato
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} {row}", tr("shading.ramp.row"))).strong());
        if !is_custom {
            if ui.button(format!("{}  {}", icons::PENCIL_SIMPLE, tr("shading.ramp.customize"))).on_hover_text(tr("shading.ramp.customize_tip")).clicked() {
                let (constant, stops) = RampCustom::contract_stops(row);
                let mut nm = m.clone();
                nm.npr.ramp_custom.set_stops(row, constant, &stops);
                if nm.npr.ramp_blend <= 0.0 {
                    nm.npr.ramp_blend = 1.0;
                }
                *sel = Some(1);
                out = RampEdit::Apply(Box::new(nm));
            }
        } else {
            if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("shading.ramp.to_contract"))).on_hover_text(tr("shading.ramp.to_contract_tip")).clicked() {
                let mut nm = m.clone();
                nm.npr.ramp_custom.clear_row(row);
                *sel = None;
                out = RampEdit::Apply(Box::new(nm));
            }
            let mut constant = custom.constant[row];
            ui.label(RichText::new(tr("shading.ramp.interp")).color(theme::TEXT_DIM));
            let mut changed = false;
            changed |= ui.selectable_value(&mut constant, false, tr("shading.ramp.linear")).changed();
            changed |= ui.selectable_value(&mut constant, true, tr("shading.ramp.steps")).changed();
            if changed {
                let mut nm = m.clone();
                let stops = custom.stops(row);
                nm.npr.ramp_custom.set_stops(row, constant, &stops);
                out = RampEdit::Apply(Box::new(nm));
            }
        }
    });

    // ---- gráfico
    let w = ui.available_width().clamp(180.0, 420.0);
    let (outer, resp) = ui.allocate_exact_size(egui::vec2(w, PLOT_H + 8.0), Sense::click());
    let plot = outer.shrink2(egui::vec2(6.0, 4.0));
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 4.0, theme::BG_DEEP);
    for q in 1..4 {
        let f = q as f32 / 4.0;
        let c = Color32::from_rgba_unmultiplied(120, 120, 140, 40);
        painter.line_segment([Pos2::new(plot.min.x + f * plot.width(), plot.min.y), Pos2::new(plot.min.x + f * plot.width(), plot.max.y)], Stroke::new(1.0, c));
        painter.line_segment([Pos2::new(plot.min.x, plot.min.y + f * plot.height()), Pos2::new(plot.max.x, plot.min.y + f * plot.height())], Stroke::new(1.0, c));
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    // referência: a linha do contrato (ou o degrau/gradiente base) em cinza
    {
        let (constant, stops) = RampCustom::contract_stops(row);
        let mut reference = RampCustom::DEFAULT;
        reference.set_stops(row, constant, &stops);
        painter.add(egui::Shape::line(curve_points(&reference, row, plot), Stroke::new(1.0, Color32::from_rgba_unmultiplied(160, 160, 175, 90))));
    }
    // marca do limiar de sombra (u = 0.5 quando half_lambert == limiar)
    painter.line_segment([Pos2::new(plot.min.x + 0.5 * plot.width(), plot.min.y), Pos2::new(plot.min.x + 0.5 * plot.width(), plot.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x2e, 80)));
    if is_custom {
        painter.add(egui::Shape::line(curve_points(&custom, row, plot), Stroke::new(2.0, theme::ACCENT)));
    }

    // ---- paradas
    let stops = custom.stops(row);
    let to_screen = |p: f32, v: f32| Pos2::new(plot.min.x + p * plot.width(), plot.max.y - v * plot.height());
    let mut live: Option<Vec<(f32, f32)>> = None;
    for (i, (p, v)) in stops.iter().enumerate() {
        let pos = to_screen(*p, *v);
        let handle = Rect::from_center_size(pos, egui::vec2(16.0, 16.0));
        let hr = ui.interact(handle, ui.id().with(("ramp_stop", row, i)), Sense::click_and_drag());
        let selected = *sel == Some(i);
        painter.circle_filled(pos, if selected { 6.5 } else { 5.0 }, if selected { Color32::WHITE } else { theme::ACCENT });
        painter.circle_stroke(pos, if selected { 6.5 } else { 5.0 }, Stroke::new(1.5, Color32::BLACK));
        if hr.clicked() || hr.drag_started() {
            *sel = Some(i);
        }
        if hr.dragged() {
            if let Some(ptr) = hr.interact_pointer_pos() {
                let u = ((ptr.x - plot.min.x) / plot.width()).clamp(0.0, 1.0);
                let val = ((plot.max.y - ptr.y) / plot.height()).clamp(0.0, 1.0);
                let lo = if i == 0 { 0.0 } else { stops[i - 1].0 + MIN_GAP };
                let hi = if i + 1 == stops.len() { 1.0 } else { stops[i + 1].0 - MIN_GAP };
                let mut ns = stops.clone();
                ns[i] = (u.clamp(lo.min(hi), hi.max(lo)), val);
                live = Some(ns);
            }
        }
        hr.on_hover_text(format!("{:.2} → {:.2}", p, v));
    }
    if let Some(ns) = live {
        let mut nm = m.clone();
        nm.npr.ramp_custom.set_stops(row, custom.constant[row], &ns);
        out = RampEdit::Live(Box::new(nm));
    }
    // duplo clique no fundo: nova parada sobre a curva
    if is_custom && resp.double_clicked() && stops.len() < RAMP_MAX_STOPS {
        if let Some(ptr) = resp.interact_pointer_pos() {
            let u = ((ptr.x - plot.min.x) / plot.width()).clamp(0.0, 1.0);
            let v = custom.eval(row, u).unwrap_or(u);
            let mut ns = stops.clone();
            ns.push((u, v));
            let mut nm = m.clone();
            nm.npr.ramp_custom.set_stops(row, custom.constant[row], &ns);
            *sel = nm.npr.ramp_custom.stops(row).iter().position(|s| (s.0 - u).abs() < 1e-5);
            out = RampEdit::Apply(Box::new(nm));
        }
    }

    // ---- faixa em tons de cinza (a linha da textura que o shader amostra)
    let (strip, _) = ui.allocate_exact_size(egui::vec2(w, 16.0), Sense::hover());
    let n = 64;
    for i in 0..n {
        let u0 = i as f32 / n as f32;
        let v = if is_custom {
            custom.eval(row, u0 + 0.5 / n as f32).unwrap_or(0.0)
        } else {
            let (constant, stops) = RampCustom::contract_stops(row);
            let mut r = RampCustom::DEFAULT;
            r.set_stops(row, constant, &stops);
            r.eval(row, u0 + 0.5 / n as f32).unwrap_or(0.0)
        };
        let g = (v * 255.0) as u8;
        let x0 = strip.min.x + u0 * strip.width();
        ui.painter().rect_filled(Rect::from_min_max(Pos2::new(x0, strip.min.y), Pos2::new(x0 + strip.width() / n as f32 + 0.5, strip.max.y)), 0.0, Color32::from_gray(g));
    }

    // ---- edição numérica / remover da parada selecionada
    if is_custom {
        let idx = sel.filter(|i| *i < stops.len());
        ui.horizontal(|ui| {
            match idx {
                Some(i) => {
                    let (mut p, mut v) = stops[i];
                    let lo = if i == 0 { 0.0 } else { stops[i - 1].0 + MIN_GAP };
                    let hi = if i + 1 == stops.len() { 1.0 } else { stops[i + 1].0 - MIN_GAP };
                    ui.label(tr("shading.ramp.pos"));
                    let a = ui.add(egui::DragValue::new(&mut p).speed(0.004).range(lo.min(hi)..=hi.max(lo)).fixed_decimals(3)).changed();
                    ui.label(tr("shading.ramp.val"));
                    let b = ui.add(egui::DragValue::new(&mut v).speed(0.004).range(0.0..=1.0).fixed_decimals(3)).changed();
                    if a || b {
                        let mut ns = stops.clone();
                        ns[i] = (p, v);
                        let mut nm = m.clone();
                        nm.npr.ramp_custom.set_stops(row, custom.constant[row], &ns);
                        out = RampEdit::Live(Box::new(nm));
                    }
                    if ui.add_enabled(stops.len() > 2, egui::Button::new(icons::TRASH).small()).on_hover_text(tr("shading.ramp.remove")).clicked() {
                        let mut ns = stops.clone();
                        ns.remove(i);
                        let mut nm = m.clone();
                        nm.npr.ramp_custom.set_stops(row, custom.constant[row], &ns);
                        *sel = None;
                        out = RampEdit::Apply(Box::new(nm));
                    }
                }
                None => {
                    ui.label(RichText::new(tr("shading.ramp.hint")).size(11.5).color(theme::TEXT_DIM));
                }
            }
            if ui.add_enabled(stops.len() < RAMP_MAX_STOPS, egui::Button::new(icons::PLUS).small()).on_hover_text(tr("shading.ramp.add")).clicked() {
                // nova parada no maior intervalo
                let mut best = (0usize, 0.0f32);
                for k in 0..stops.len() - 1 {
                    let gap = stops[k + 1].0 - stops[k].0;
                    if gap > best.1 {
                        best = (k, gap);
                    }
                }
                let u = (stops[best.0].0 + stops[best.0 + 1].0) * 0.5;
                let v = custom.eval(row, u).unwrap_or(0.5);
                let mut ns = stops.clone();
                ns.push((u, v));
                let mut nm = m.clone();
                nm.npr.ramp_custom.set_stops(row, custom.constant[row], &ns);
                *sel = nm.npr.ramp_custom.stops(row).iter().position(|s| (s.0 - u).abs() < 1e-5);
                out = RampEdit::Apply(Box::new(nm));
            }
        });
    }
    ui.label(RichText::new(tr("shading.ramp.axes")).size(11.0).color(theme::TEXT_DIM));
    out
}
