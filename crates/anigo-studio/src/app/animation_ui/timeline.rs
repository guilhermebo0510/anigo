//! Painel inferior da Animação: régua com timecode, dope sheet (chaves por osso/nó), Graph (curvas com alças Bézier) e a
//! lista de clips. Toda a entrada é tratada à mão sobre um retângulo (hit-test próprio), e cada gesto sobre chaves é
//! `preview` enquanto arrasta e UM `commit_preview` ao soltar.

use super::{describe_anim, DragKind, Editor, KeyDrag, Marquee, StudioApp};
use crate::anim::graph::{self, Handle};
use crate::anim::keys::{self, KeyId};
use crate::i18n::tr;
use crate::icons;
use crate::theme;
use anigo_core::animation::{timecode, AnimEdit, ChannelGroup, ChannelTarget, Extrapolation, Interp, ValueKind};
use anigo_core::command::Command;
use anigo_core::pose::bone_index;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke};
use std::collections::BTreeSet;

pub(crate) const LEFT_W: f32 = 210.0;
const KEY_R: f32 = 5.5;
const AXIS_COLORS: [Color32; 5] = [Color32::from_rgb(0xe8, 0x4a, 0x4a), Color32::from_rgb(0x6a, 0xcc, 0x4a), Color32::from_rgb(0x4a, 0x84, 0xf0), Color32::from_rgb(0xd0, 0xb0, 0x40), Color32::from_rgb(0xb0, 0x70, 0xe0)];
const PLAYHEAD: Color32 = Color32::from_rgb(0x4c, 0xc9, 0xff);
const KEY_COL: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xe4);

#[derive(Clone, Debug)]
pub(crate) enum RowKind {
    Summary,
    Group(ChannelGroup),
    Channel(ChannelTarget),
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub kind: RowKind,
    pub label: String,
    pub depth: usize,
    /// (quadro, interpolação) das chaves a desenhar nesta linha.
    pub keys: Vec<(i32, Interp)>,
    pub collapsed: bool,
}

/// Transforma o quadro em x (tela) e vice-versa.
#[derive(Clone, Copy)]
pub(crate) struct Axis {
    pub left: f32,
    pub x0: f32,
    pub ppf: f32,
}

impl Axis {
    pub fn x(&self, frame: f32) -> f32 {
        self.left + (frame - self.x0) * self.ppf
    }
    pub fn frame(&self, x: f32) -> f32 {
        self.x0 + (x - self.left) / self.ppf
    }
}

/// Ordem de exibição dos canais.
pub(crate) fn channel_order(t: &ChannelTarget) -> (u8, usize, String) {
    match t {
        ChannelTarget::Root => (0, 0, String::new()),
        ChannelTarget::Hips => (0, 1, String::new()),
        ChannelTarget::Bone { bone } => (0, 2 + bone_index(bone).unwrap_or(99), String::new()),
        ChannelTarget::Hand { side } => (0, 200 + *side as usize, String::new()),
        other => (1, 0, other.key_string()),
    }
}

fn group_key(g: ChannelGroup) -> &'static str {
    match g {
        ChannelGroup::Character => "anim.group.character",
        ChannelGroup::Nodes => "anim.group.nodes",
        ChannelGroup::Sun => "anim.group.sun",
        ChannelGroup::Camera => "anim.group.camera",
    }
}

impl StudioApp {
    /// Nome legível de um canal.
    pub(crate) fn anim_channel_label(&self, t: &ChannelTarget) -> String {
        let node = |id: &anigo_core::NodeId| self.core.project().scene.node(id).map(|n| n.name.clone()).unwrap_or_else(|| id.to_string());
        match t {
            ChannelTarget::Bone { bone } => tr(&format!("pose.bone.{bone}")).to_string(),
            ChannelTarget::Root => tr("anim.ch.root").to_string(),
            ChannelTarget::Hips => tr("anim.ch.hips").to_string(),
            ChannelTarget::Hand { side } => tr(if *side == anigo_core::animation::HandSide::Left { "anim.ch.hand_l" } else { "anim.ch.hand_r" }).to_string(),
            ChannelTarget::NodeTranslation { node_id } => format!("{} · {}", node(node_id), tr("anim.ch.position")),
            ChannelTarget::NodeRotation { node_id } => format!("{} · {}", node(node_id), tr("anim.ch.rotation")),
            ChannelTarget::NodeScale { node_id } => format!("{} · {}", node(node_id), tr("anim.ch.scale")),
            ChannelTarget::Sun { prop } => format!("{} · {}", tr("anim.group.sun"), tr(&format!("anim.ch.sun.{}", sun_key(*prop)))),
            ChannelTarget::Camera { prop } => format!("{} · {}", tr("anim.group.camera"), tr(&format!("anim.ch.cam.{}", cam_key(*prop)))),
        }
    }

    /// Linhas do dope sheet: resumo, grupos recolhíveis e canais.
    pub(crate) fn anim_rows(&self) -> Vec<Row> {
        let Some(clip) = self.anim_clip() else { return Vec::new() };
        if clip.channels.is_empty() {
            return Vec::new();
        }
        let sel_bones: BTreeSet<usize> = self.pose_ui.selection.iter().copied().collect();
        let only = self.anim_ui.only_selected_bones && !sel_bones.is_empty();
        let visible = |t: &ChannelTarget| -> bool {
            if !only {
                return true;
            }
            match t {
                ChannelTarget::Bone { bone } => bone_index(bone).is_some_and(|b| sel_bones.contains(&b)),
                ChannelTarget::Hips => sel_bones.contains(&anigo_core::pose::bone::HIPS),
                ChannelTarget::Root => sel_bones.contains(&anigo_core::pose::bone::ROOT),
                ChannelTarget::Hand { side } => sel_bones.contains(&if *side == anigo_core::animation::HandSide::Left { anigo_core::pose::bone::L_WRIST } else { anigo_core::pose::bone::R_WRIST }),
                _ => true,
            }
        };
        let frames_of = |chs: &[&anigo_core::animation::Channel]| -> Vec<(i32, Interp)> {
            let set: BTreeSet<i32> = chs.iter().flat_map(|c| c.keys.iter().map(|k| k.frame)).collect();
            set.into_iter().map(|f| (f, Interp::Linear)).collect()
        };
        let all: Vec<&anigo_core::animation::Channel> = clip.channels.iter().filter(|c| visible(&c.target)).collect();
        let mut rows = vec![Row { kind: RowKind::Summary, label: tr("anim.row.summary").to_string(), depth: 0, keys: frames_of(&all), collapsed: false }];
        for g in Self::anim_group_order() {
            let mut chs: Vec<&anigo_core::animation::Channel> = all.iter().copied().filter(|c| c.target.group() == g).collect();
            if chs.is_empty() {
                continue;
            }
            chs.sort_by_key(|c| channel_order(&c.target));
            let collapsed = self.anim_ui.collapsed.contains(group_key(g));
            rows.push(Row { kind: RowKind::Group(g), label: tr(group_key(g)).to_string(), depth: 0, keys: frames_of(&chs), collapsed });
            if !collapsed {
                for c in chs {
                    rows.push(Row {
                        kind: RowKind::Channel(c.target.clone()),
                        label: self.anim_channel_label(&c.target),
                        depth: 1,
                        keys: c.keys.iter().map(|k| (k.frame, k.interp)).collect(),
                        collapsed: false,
                    });
                }
            }
        }
        rows
    }

    /// Chaves (ids) que uma tecla de uma linha representa: resumo/grupo = todas do quadro (do grupo).
    fn ids_of_row_key(&self, row: &Row, frame: i32) -> Vec<KeyId> {
        let Some(clip) = self.anim_clip() else { return Vec::new() };
        match &row.kind {
            RowKind::Channel(t) => vec![(t.clone(), frame)],
            RowKind::Summary => keys::keys_at_frame(clip, frame, |_| true).into_iter().collect(),
            RowKind::Group(g) => keys::keys_at_frame(clip, frame, |t| t.group() == *g).into_iter().collect(),
        }
    }

    // ============================================================ painel inferior

    pub(crate) fn animation_timeline_body(&mut self, ui: &mut egui::Ui) {
        let editor = Editor::from_tool_index(*self.tool_idx.get(&self.ws).unwrap_or(&0));
        self.anim_toolbar(ui);
        ui.separator();
        let rect = ui.available_rect_before_wrap();
        self.anim_ui.timeline_hovered = ui.rect_contains_pointer(rect) || self.anim_ui.drag.is_some();
        match editor {
            Editor::Dope => self.dope_sheet(ui),
            Editor::Curves => self.graph_editor(ui),
            Editor::Clips => self.clips_editor(ui),
        }
    }

    // ============================================================ régua

    fn ruler(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, ruler: Rect, axis: Axis) {
        let fps = self.anim_fps();
        let span = self.anim_span();
        painter.rect_filled(ruler, 0.0, Color32::from_rgb(0x1a, 0x1a, 0x20));
        // faixa do clip
        let (a, b) = (axis.x(span.start as f32), axis.x(span.end as f32));
        painter.rect_filled(Rect::from_min_max(Pos2::new(a.max(ruler.min.x), ruler.min.y), Pos2::new(b.min(ruler.max.x), ruler.max.y)), 0.0, Color32::from_rgba_unmultiplied(0x4c, 0xc9, 0xff, 24));
        // passo da escala: o menor "bonito" com >= 56 px
        let steps = [1.0f32, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 24.0, 48.0, 72.0, 120.0, 240.0, 480.0, 1200.0];
        let step = steps.into_iter().find(|s| s * axis.ppf >= 56.0).unwrap_or(2400.0);
        let first = (axis.frame(ruler.min.x) / step).floor() * step;
        let mut f = first;
        while axis.x(f) < ruler.max.x {
            let x = axis.x(f);
            if x >= ruler.min.x - 1.0 {
                painter.line_segment([Pos2::new(x, ruler.max.y - 8.0), Pos2::new(x, ruler.max.y)], Stroke::new(1.0, theme::TEXT_DIM));
                painter.text(Pos2::new(x + 3.0, ruler.min.y + 2.0), Align2::LEFT_TOP, format!("{}", f as i32), FontId::proportional(10.5), theme::TEXT_DIM);
            }
            f += step;
        }
        // marcadores
        if let Some(clip) = self.anim_clip() {
            for m in &clip.markers {
                let x = axis.x(m.frame as f32);
                if x < ruler.min.x - 2.0 || x > ruler.max.x + 2.0 {
                    continue;
                }
                painter.add(Shape::convex_polygon(vec![Pos2::new(x, ruler.max.y), Pos2::new(x + 8.0, ruler.max.y - 6.0), Pos2::new(x, ruler.max.y - 12.0)], Color32::from_rgb(0xff, 0xc0, 0x40), Stroke::NONE));
                painter.text(Pos2::new(x + 10.0, ruler.max.y - 10.0), Align2::LEFT_CENTER, &m.name, FontId::proportional(10.0), Color32::from_rgb(0xff, 0xd0, 0x70));
            }
        }
        // playhead (cabeça + rótulo com o timecode)
        let cur = self.anim_ui.transport.displayed(span);
        let x = axis.x(cur);
        if x >= ruler.min.x - 20.0 && x <= ruler.max.x + 20.0 {
            let label = format!("{}  {}", cur.round() as i32, timecode(cur.round() as i32, fps));
            let g = painter.layout_no_wrap(label, FontId::monospace(10.5), Color32::BLACK);
            let r = Rect::from_min_size(Pos2::new(x + 6.0, ruler.min.y + 1.0), g.size()).expand2(egui::vec2(4.0, 1.0));
            painter.rect_filled(r, 3.0, PLAYHEAD);
            painter.galley(r.min + egui::vec2(4.0, 1.0), g, Color32::BLACK);
            painter.add(Shape::convex_polygon(vec![Pos2::new(x - 6.0, ruler.min.y), Pos2::new(x + 6.0, ruler.min.y), Pos2::new(x, ruler.min.y + 9.0)], PLAYHEAD, Stroke::NONE));
        }
        let _ = ui;
    }

    /// Linha do playhead e sombra fora da faixa, sobre a área de chaves.
    fn playhead_and_range(&self, painter: &egui::Painter, area: Rect, axis: Axis) {
        let span = self.anim_span();
        let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 70);
        let (a, b) = (axis.x(span.start as f32), axis.x(span.end as f32));
        if a > area.min.x {
            painter.rect_filled(Rect::from_min_max(area.min, Pos2::new(a.min(area.max.x), area.max.y)), 0.0, shade);
        }
        if b < area.max.x {
            painter.rect_filled(Rect::from_min_max(Pos2::new(b.max(area.min.x), area.min.y), area.max), 0.0, shade);
        }
        for x in [a, b] {
            if x >= area.min.x && x <= area.max.x {
                painter.line_segment([Pos2::new(x, area.min.y), Pos2::new(x, area.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x4c, 0xc9, 0xff, 90)));
            }
        }
        let x = axis.x(self.anim_ui.transport.displayed(span));
        if x >= area.min.x && x <= area.max.x {
            painter.line_segment([Pos2::new(x, area.min.y), Pos2::new(x, area.max.y)], Stroke::new(1.5, PLAYHEAD));
        }
    }

    fn grid(&self, painter: &egui::Painter, area: Rect, axis: Axis) {
        let steps = [1.0f32, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 24.0, 48.0, 72.0, 120.0, 240.0, 480.0, 1200.0];
        let step = steps.into_iter().find(|s| s * axis.ppf >= 28.0).unwrap_or(2400.0);
        let mut f = (axis.frame(area.min.x) / step).floor() * step;
        while axis.x(f) < area.max.x {
            let major = (f / step).round() as i64 % 4 == 0;
            painter.line_segment([Pos2::new(axis.x(f), area.min.y), Pos2::new(axis.x(f), area.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, if major { 16 } else { 8 })));
            f += step;
        }
    }

    /// Entrada comum: zoom na roda, pan com o botão do meio, rolagem. Devolve `true` se o mouse está sobre `area`.
    fn view_input(&mut self, ui: &egui::Ui, resp: &egui::Response, area: Rect) {
        let ctx = ui.ctx().clone();
        if resp.hovered() {
            let (scroll, mods) = ctx.input(|i| (i.smooth_scroll_delta, i.modifiers));
            if scroll.y.abs() > 0.0 || scroll.x.abs() > 0.0 {
                let v = &mut self.anim_ui.view;
                if mods.command {
                    v.x0 -= (scroll.y + scroll.x) / v.ppf;
                } else if mods.shift {
                    v.scroll_y = (v.scroll_y - scroll.y).max(0.0);
                } else {
                    let mouse = resp.hover_pos().unwrap_or(area.center());
                    let axis = Axis { left: area.min.x, x0: v.x0, ppf: v.ppf };
                    let f = axis.frame(mouse.x);
                    v.ppf = (v.ppf * (scroll.y * 0.0035).exp()).clamp(1.0, 120.0);
                    v.x0 = f - (mouse.x - area.min.x) / v.ppf;
                    if scroll.x.abs() > 0.0 {
                        v.x0 -= scroll.x / v.ppf;
                    }
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Middle) {
            let d = ctx.input(|i| i.pointer.delta());
            let v = &mut self.anim_ui.view;
            v.x0 -= d.x / v.ppf;
            v.scroll_y = (v.scroll_y - d.y).max(0.0);
            v.y_lo += d.y * 0.0; // a vista vertical do Graph é refeita no próximo quadro
        }
    }

    /// Mantém o playhead visível durante a reprodução (vira a "página" quando sai pela direita).
    fn follow_playhead(&mut self, area: Rect) {
        if !self.anim_ui.transport.playing {
            return;
        }
        let span = self.anim_span();
        let cur = self.anim_ui.transport.displayed(span);
        let v = &mut self.anim_ui.view;
        let right = v.x0 + area.width() / v.ppf;
        if cur > right - 2.0 || cur < v.x0 {
            v.x0 = cur - 2.0;
        }
    }

    // ============================================================ dope sheet

    fn dope_sheet(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, Sense::hover());
        let left = Rect::from_min_size(full.min, egui::vec2(LEFT_W, full.height()));
        let canvas = Rect::from_min_max(Pos2::new(full.min.x + LEFT_W, full.min.y), full.max);
        self.anim_ui.canvas = canvas;
        self.follow_playhead(canvas);
        let v = self.anim_ui.view;
        let axis = Axis { left: canvas.min.x, x0: v.x0, ppf: v.ppf };
        let ruler_h = self.anim_ui.ruler_h;
        let ruler_rect = Rect::from_min_size(canvas.min, egui::vec2(canvas.width(), ruler_h));
        let area = Rect::from_min_max(Pos2::new(canvas.min.x, canvas.min.y + ruler_h), canvas.max);
        let row_h = self.anim_ui.row_h;
        let rows = self.anim_rows();
        let content_h = rows.len() as f32 * row_h;
        let max_scroll = (content_h - area.height()).max(0.0);
        self.anim_ui.view.scroll_y = self.anim_ui.view.scroll_y.min(max_scroll);
        let scroll_y = self.anim_ui.view.scroll_y;
        let row_y = |i: usize| area.min.y + i as f32 * row_h - scroll_y;

        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, theme::BG_DEEP);
        painter.rect_filled(left, 0.0, theme::BG_PANEL);
        // ---- interação: um retângulo para a régua+área de chaves
        let resp = ui.interact(canvas, ui.id().with("dope_canvas"), Sense::click_and_drag());
        let left_resp = ui.interact(left, ui.id().with("dope_left"), Sense::click());
        self.view_input(ui, &resp, area);
        let mouse = resp.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos()));
        let (shift, ctrl, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let press = ctx.input(|i| i.pointer.press_origin());

        // ---- fundo das linhas
        let cp = ui.painter_at(canvas);
        for (i, row) in rows.iter().enumerate() {
            let y = row_y(i);
            if y + row_h < area.min.y || y > area.max.y {
                continue;
            }
            let r = Rect::from_min_max(Pos2::new(canvas.min.x, y), Pos2::new(canvas.max.x, y + row_h));
            let tint = match row.kind {
                RowKind::Summary => Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 22),
                RowKind::Group(_) => Color32::from_rgba_unmultiplied(255, 255, 255, 14),
                RowKind::Channel(_) if i % 2 == 0 => Color32::from_rgba_unmultiplied(255, 255, 255, 5),
                RowKind::Channel(_) => Color32::TRANSPARENT,
            };
            cp.rect_filled(r.intersect(area), 0.0, tint);
        }
        self.grid(&cp.with_clip_rect(area), area, axis);

        // ---- geometria para o smoke (posições dos canais) e hit-test
        self.anim_ui.rows_geom.clear();
        for (i, row) in rows.iter().enumerate() {
            if let RowKind::Channel(t) = &row.kind {
                self.anim_ui.rows_geom.push((t.clone(), row_y(i) + row_h * 0.5));
            }
        }

        // ---- teclas
        let area_painter = cp.with_clip_rect(area);
        let sel = &self.anim_ui.selection;
        for (i, row) in rows.iter().enumerate() {
            let y = row_y(i) + row_h * 0.5;
            if y < area.min.y - row_h || y > area.max.y + row_h {
                continue;
            }
            // barras de "hold": segmentos em degrau (valor constante até a próxima chave)
            if let RowKind::Channel(_) = &row.kind {
                for w in row.keys.windows(2) {
                    if w[0].1 == Interp::Step {
                        let (xa, xb) = (axis.x(w[0].0 as f32), axis.x(w[1].0 as f32));
                        area_painter.rect_filled(Rect::from_min_max(Pos2::new(xa, y - 2.0), Pos2::new(xb, y + 2.0)), 1.0, Color32::from_rgba_unmultiplied(0xff, 0xc0, 0x60, 90));
                    }
                }
            }
            for (frame, interp) in &row.keys {
                let x = axis.x(*frame as f32);
                if x < area.min.x - 8.0 || x > area.max.x + 8.0 {
                    continue;
                }
                let ids = self.ids_of_row_key(row, *frame);
                let selected = !ids.is_empty() && ids.iter().all(|k| sel.contains(k));
                let partly = !selected && ids.iter().any(|k| sel.contains(k));
                draw_key(&area_painter, Pos2::new(x, y), *interp, selected, partly, matches!(row.kind, RowKind::Channel(_)), *frame == self.anim_ui.transport.current_frame(self.anim_span()));
            }
        }
        self.playhead_and_range(&area_painter, area, axis);

        // ---- régua
        let rp = ui.painter_at(ruler_rect);
        self.ruler(ui, &rp, ruler_rect, axis);

        // ---- coluna da esquerda (nomes)
        let lp = ui.painter_at(left);
        lp.rect_filled(Rect::from_min_size(left.min, egui::vec2(LEFT_W, ruler_h)), 0.0, Color32::from_rgb(0x1a, 0x1a, 0x20));
        lp.text(left.min + egui::vec2(8.0, ruler_h * 0.5), Align2::LEFT_CENTER, format!("{} {}", tr("anim.dope"), self.anim_clip().map(|c| c.name.as_str()).unwrap_or("-")), FontId::proportional(11.5), theme::TEXT_DIM);
        let left_area = Rect::from_min_max(Pos2::new(left.min.x, left.min.y + ruler_h), left.max);
        let lpa = lp.with_clip_rect(left_area);
        let mut left_click: Option<(usize, Pos2)> = None;
        for (i, row) in rows.iter().enumerate() {
            let y = row_y(i);
            if y + row_h < left_area.min.y || y > left_area.max.y {
                continue;
            }
            let rr = Rect::from_min_size(Pos2::new(left.min.x, y), egui::vec2(LEFT_W, row_h));
            let hot = left_resp.hover_pos().is_some_and(|p| rr.contains(p));
            if hot {
                lpa.rect_filled(rr, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12));
            }
            let (label_col, icon) = self.row_style(row);
            let x = left.min.x + 8.0 + row.depth as f32 * 14.0;
            if let RowKind::Group(_) = row.kind {
                lpa.text(Pos2::new(x, y + row_h * 0.5), Align2::LEFT_CENTER, if row.collapsed { icons::CARET_RIGHT } else { icons::CARET_DOWN }, FontId::proportional(12.0), theme::TEXT_DIM);
                lpa.text(Pos2::new(x + 14.0, y + row_h * 0.5), Align2::LEFT_CENTER, &row.label, FontId::proportional(12.0), label_col);
            } else {
                lpa.text(Pos2::new(x, y + row_h * 0.5), Align2::LEFT_CENTER, format!("{icon} {}", row.label), FontId::proportional(11.5), label_col);
            }
            if left_resp.clicked() && hot {
                left_click = left_resp.interact_pointer_pos().map(|p| (i, p));
            }
        }
        if let Some((i, _)) = left_click {
            self.on_row_clicked(&rows[i], shift || ctrl);
        }

        // ---- gestos na área de chaves
        self.dope_gestures(&ctx, &resp, area, ruler_rect, axis, &rows, row_y_vec(&rows, area.min.y, scroll_y, row_h), mouse, press, (shift, ctrl, alt));
        if rows.is_empty() {
            let msg = if self.anim_clip().is_some() { tr("anim.empty_clip") } else { tr("anim.no_clip") };
            cp.text(area.center(), Align2::CENTER_CENTER, msg, FontId::proportional(13.0), theme::TEXT_DIM);
        }
        // marquee
        if let Some(m) = &self.anim_ui.marquee {
            if (m.cur - m.start).length() > 3.0 {
                let r = Rect::from_two_pos(m.start, m.cur);
                cp.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 40));
                cp.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
        }
        self.anim_finish_gesture(&ctx);
    }

    fn row_style(&self, row: &Row) -> (Color32, &'static str) {
        match &row.kind {
            RowKind::Summary => (theme::ACCENT, icons::DIAMOND),
            RowKind::Group(_) => (theme::TEXT, ""),
            RowKind::Channel(t) => {
                let selected = matches!(t, ChannelTarget::Bone { bone } if bone_index(bone).is_some_and(|b| self.pose_ui.selection.contains(&b)));
                let side = match t {
                    ChannelTarget::Bone { bone } => match bone_index(bone) {
                        Some(b) if anigo_core::pose::is_left_bone(b) => Some(Color32::from_rgb(0x5a, 0xa8, 0xff)),
                        Some(b) if anigo_core::pose::is_right_bone(b) => Some(Color32::from_rgb(0xff, 0x6a, 0x6a)),
                        _ => None,
                    },
                    ChannelTarget::Hand { side } => Some(if *side == anigo_core::animation::HandSide::Left { Color32::from_rgb(0x5a, 0xa8, 0xff) } else { Color32::from_rgb(0xff, 0x6a, 0x6a) }),
                    _ => None,
                };
                let col = if selected { theme::SELECT } else { side.unwrap_or(theme::TEXT) };
                (col, match t.value_kind() {
                    ValueKind::Quat => icons::ARROWS_CLOCKWISE,
                    ValueKind::Vec3 => icons::ARROWS_OUT_CARDINAL,
                    ValueKind::Curl => icons::HAND,
                    ValueKind::Scalar => icons::SLIDERS_HORIZONTAL,
                })
            }
        }
    }

    fn on_row_clicked(&mut self, row: &Row, additive: bool) {
        match &row.kind {
            RowKind::Group(g) => {
                let k = group_key(*g).to_string();
                if !self.anim_ui.collapsed.remove(&k) {
                    self.anim_ui.collapsed.insert(k);
                }
            }
            RowKind::Channel(t) => {
                if let ChannelTarget::Bone { bone } = t {
                    if let Some(b) = bone_index(bone) {
                        if additive {
                            self.pose_ui.toggle(b);
                        } else {
                            self.pose_ui.select_one(b);
                        }
                    }
                }
                // selecionar as chaves do canal inteiro no dope sheet
                if let Some(c) = self.anim_clip().and_then(|c| c.channel(t)) {
                    let all: BTreeSet<KeyId> = c.keys.iter().map(|k| (t.clone(), k.frame)).collect();
                    if additive {
                        self.anim_ui.selection.extend(all);
                    } else {
                        self.anim_ui.selection = all;
                    }
                }
            }
            RowKind::Summary => {
                if let Some(c) = self.anim_clip() {
                    self.anim_ui.selection = c.channels.iter().flat_map(|ch| ch.keys.iter().map(|k| (ch.target.clone(), k.frame))).collect();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dope_gestures(&mut self, ctx: &egui::Context, resp: &egui::Response, area: Rect, ruler: Rect, axis: Axis, rows: &[Row], ys: Vec<f32>, mouse: Option<Pos2>, press: Option<Pos2>, mods: (bool, bool, bool)) {
        let (shift, ctrl, alt) = mods;
        let row_h = self.anim_ui.row_h;
        let hit = |p: Pos2| -> Option<(usize, i32)> {
            if !area.contains(p) {
                return None;
            }
            for (i, row) in rows.iter().enumerate() {
                let y = ys[i] + row_h * 0.5;
                if (p.y - y).abs() > row_h * 0.5 {
                    continue;
                }
                let mut best: Option<(f32, i32)> = None;
                for (f, _) in &row.keys {
                    let d = (axis.x(*f as f32) - p.x).abs();
                    if d <= KEY_R + 2.0 && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, *f));
                    }
                }
                if let Some((_, f)) = best {
                    return Some((i, f));
                }
            }
            None
        };
        let frame_at = |x: f32| axis.frame(x);
        // ---- Esc cancela o arrasto de chaves
        if self.anim_ui.drag.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_key_drag();
            return;
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = press {
                if ruler.contains(p) {
                    self.anim_ui.scrubbing = true;
                } else if let Some((ri, f)) = hit(p) {
                    let ids = self.ids_of_row_key(&rows[ri], f);
                    let already = ids.iter().all(|k| self.anim_ui.selection.contains(k));
                    if !already {
                        if shift || ctrl {
                            self.anim_ui.selection.extend(ids);
                        } else {
                            self.anim_ui.selection = ids.into_iter().collect();
                        }
                    }
                    let kind = if alt {
                        DragKind::Scale { pivot: self.anim_ui.transport.displayed(self.anim_span()) }
                    } else if shift {
                        DragKind::Duplicate
                    } else {
                        DragKind::Move
                    };
                    self.begin_key_drag(kind, frame_at(p.x));
                } else {
                    self.anim_ui.marquee = Some(Marquee { start: p, cur: p, additive: shift || ctrl });
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let Some(m) = mouse {
                if self.anim_ui.scrubbing {
                    self.scrub_to(frame_at(m.x), shift);
                } else if self.anim_ui.drag.is_some() {
                    self.update_key_drag(frame_at(m.x));
                } else if let Some(mq) = self.anim_ui.marquee.as_mut() {
                    mq.cur = m;
                }
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            if self.anim_ui.scrubbing {
                self.anim_ui.scrubbing = false;
            } else if self.anim_ui.drag.is_some() {
                self.finish_key_drag();
            } else if let Some(mq) = self.anim_ui.marquee.take() {
                let r = Rect::from_two_pos(mq.start, mq.cur);
                let mut picked: BTreeSet<KeyId> = BTreeSet::new();
                for (i, row) in rows.iter().enumerate() {
                    let y = ys[i] + row_h * 0.5;
                    if y < r.min.y - row_h * 0.5 || y > r.max.y + row_h * 0.5 {
                        continue;
                    }
                    for (f, _) in &row.keys {
                        let x = axis.x(*f as f32);
                        if x >= r.min.x && x <= r.max.x {
                            picked.extend(self.ids_of_row_key(&rows[i], *f));
                        }
                    }
                }
                if !mq.additive {
                    self.anim_ui.selection.clear();
                }
                self.anim_ui.selection.extend(picked);
            }
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                if ruler.contains(p) {
                    // marcador? vai até ele; senão posiciona o playhead
                    let f = frame_at(p.x);
                    let marker = self.anim_clip().and_then(|c| c.markers.iter().find(|m| (axis.x(m.frame as f32) - p.x).abs() < 7.0).map(|m| m.frame));
                    self.scrub_to(marker.map(|m| m as f32).unwrap_or(f), shift);
                } else if let Some((ri, f)) = hit(p) {
                    let ids = self.ids_of_row_key(&rows[ri], f);
                    if shift || ctrl {
                        let all = ids.iter().all(|k| self.anim_ui.selection.contains(k));
                        for k in ids {
                            if all {
                                self.anim_ui.selection.remove(&k);
                            } else {
                                self.anim_ui.selection.insert(k);
                            }
                        }
                    } else {
                        self.anim_ui.selection = ids.into_iter().collect();
                    }
                    // seleciona também o osso do canal (o gizmo do viewport acompanha)
                    if let RowKind::Channel(ChannelTarget::Bone { bone }) = &rows[ri].kind {
                        if let Some(b) = bone_index(bone) {
                            self.pose_ui.select_one(b);
                        }
                    }
                } else if !(shift || ctrl) {
                    self.anim_ui.selection.clear();
                }
            }
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                if area.contains(p) && hit(p).is_none() {
                    self.scrub_to(frame_at(p.x), false);
                }
            }
        }
    }

    pub(crate) fn scrub_to(&mut self, frame: f32, free: bool) {
        let f = if free { frame } else { frame.round() };
        self.anim_ui.transport.goto(f.max(0.0), true);
        self.anim_ui.phys.last_raw = None;
    }

    // ============================================================ arrasto de chaves

    fn begin_key_drag(&mut self, kind: DragKind, start_frame: f32) {
        let Some(clip) = self.anim_clip().cloned() else { return };
        let base_sel = self.anim_ui.selection.clone();
        self.core.begin_preview();
        self.anim_ui.drag = Some(KeyDrag { kind, start_frame, base_clip: clip, base_sel, last: None });
    }

    fn cancel_key_drag(&mut self) {
        if let Some(d) = self.anim_ui.drag.take() {
            self.core.cancel_preview();
            self.anim_ui.selection = d.base_sel;
            self.anim_say(tr("anim.drag.cancelled"));
        }
    }

    /// Edição e nova seleção do arrasto para o ponteiro em `ptr_frame` (puro: usa o clip do início do gesto).
    fn drag_plan(&self, d: &KeyDrag, ptr_frame: f32) -> Option<(AnimEdit, BTreeSet<KeyId>)> {
        let id = self.anim_clip_id()?;
        let hold = d.base_clip.hold_step.max(1) as i32;
        let snap = |delta: i32| -> i32 {
            if hold > 1 {
                ((delta as f32 / hold as f32).round() as i32) * hold
            } else {
                delta
            }
        };
        let delta = snap((ptr_frame - d.start_frame).round() as i32);
        match &d.kind {
            DragKind::Move => {
                let (edit, applied) = keys::plan_move(&id, &d.base_clip, &d.base_sel, delta)?;
                let sel = d.base_sel.iter().map(|(t, f)| (t.clone(), f + applied)).collect();
                Some((edit, sel))
            }
            DragKind::Duplicate => {
                let edit = keys::plan_duplicate(&id, &d.base_clip, &d.base_sel, delta)?;
                let sel = d.base_sel.iter().map(|(t, f)| (t.clone(), f + delta)).collect();
                Some((edit, sel))
            }
            DragKind::Scale { pivot } => {
                let denom = d.start_frame - pivot;
                if denom.abs() < 0.5 {
                    return None;
                }
                let factor = ((ptr_frame - pivot) / denom).clamp(0.05, 20.0);
                let edit = keys::plan_scale(&id, &d.base_clip, &d.base_sel, *pivot, factor)?;
                let mut map = std::collections::BTreeMap::new();
                if let AnimEdit::MoveKeys { moves, .. } = &edit {
                    for m in moves {
                        map.insert((m.target.clone(), m.frame), m.to_frame);
                    }
                }
                let sel = d.base_sel.iter().map(|(t, f)| (t.clone(), map.get(&(t.clone(), *f)).copied().unwrap_or(*f))).collect();
                Some((edit, sel))
            }
            DragKind::Handle { .. } => None,
        }
    }

    fn update_key_drag(&mut self, ptr_frame: f32) {
        let Some(mut d) = self.anim_ui.drag.take() else { return };
        let plan = self.drag_plan(&d, ptr_frame);
        self.core.cancel_preview();
        self.core.begin_preview();
        match plan {
            Some((edit, sel)) => {
                if self.core.preview(Command::EditAnimation { edit: edit.clone() }).is_ok() {
                    d.last = Some(edit);
                    self.anim_ui.selection = sel;
                }
            }
            None => {
                d.last = None;
                self.anim_ui.selection = d.base_sel.clone();
            }
        }
        self.anim_ui.drag = Some(d);
    }

    fn finish_key_drag(&mut self) {
        let Some(d) = self.anim_ui.drag.take() else { return };
        match d.last {
            Some(edit) => match self.core.commit_preview(Command::EditAnimation { edit }) {
                Ok(o) => self.anim_ui.message = describe_anim(&o.description),
                Err(e) => {
                    self.anim_ui.selection = d.base_sel;
                    if !(e.contains("no effect") || e.contains("already")) {
                        self.anim_ui.message = format!("{}: {e}", tr("char.error"));
                    }
                }
            },
            None => {
                self.core.cancel_preview();
                self.anim_ui.selection = d.base_sel;
            }
        }
    }

    // ============================================================ Graph

    fn graph_channels(&mut self) -> Vec<ChannelTarget> {
        let Some(clip) = self.anim_clip() else { return Vec::new() };
        let mut chs: Vec<ChannelTarget> = clip.channels.iter().map(|c| c.target.clone()).collect();
        chs.sort_by_key(channel_order);
        // visíveis por padrão: canais das chaves selecionadas; senão os ossos selecionados; senão os 6 primeiros
        if self.anim_ui.graph_visible.is_empty() || !self.anim_ui.graph_visible.iter().any(|t| chs.contains(t)) {
            let from_sel: BTreeSet<ChannelTarget> = self.anim_ui.selection.iter().map(|(t, _)| t.clone()).filter(|t| chs.contains(t)).collect();
            self.anim_ui.graph_visible = if !from_sel.is_empty() { from_sel } else { chs.iter().take(6).cloned().collect() };
        }
        chs
    }

    fn graph_editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, Sense::hover());
        let left = Rect::from_min_size(full.min, egui::vec2(LEFT_W, full.height()));
        let canvas = Rect::from_min_max(Pos2::new(full.min.x + LEFT_W, full.min.y), full.max);
        self.anim_ui.canvas = canvas;
        self.follow_playhead(canvas);
        let channels = self.graph_channels();
        let v = self.anim_ui.view;
        let axis = Axis { left: canvas.min.x, x0: v.x0, ppf: v.ppf };
        let ruler_h = self.anim_ui.ruler_h;
        let ruler_rect = Rect::from_min_size(canvas.min, egui::vec2(canvas.width(), ruler_h));
        let area = Rect::from_min_max(Pos2::new(canvas.min.x, canvas.min.y + ruler_h), canvas.max);
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, theme::BG_DEEP);
        painter.rect_filled(left, 0.0, theme::BG_PANEL);
        let resp = ui.interact(canvas, ui.id().with("graph_canvas"), Sense::click_and_drag());
        let left_resp = ui.interact(left, ui.id().with("graph_left"), Sense::click());
        self.view_input(ui, &resp, area);
        let ext = self.anim_clip().map(|c| c.extrapolation).unwrap_or(Extrapolation::Constant);
        let (f0, f1) = (axis.frame(area.min.x), axis.frame(area.max.x));

        // ---- curvas visíveis
        struct Curve {
            target: ChannelTarget,
            comps: Vec<Vec<[f32; 2]>>,
        }
        let mut curves: Vec<Curve> = Vec::new();
        if let Some(clip) = self.anim_clip() {
            for t in &channels {
                if !self.anim_ui.graph_visible.contains(t) {
                    continue;
                }
                if let Some(ch) = clip.channel(t) {
                    curves.push(Curve { target: t.clone(), comps: graph::curves(ch, ext, f0.max(-50.0), f1, (axis.ppf / 3.0).clamp(0.5, 4.0)) });
                }
            }
        }
        // faixa vertical automática
        let all: Vec<Vec<[f32; 2]>> = curves.iter().flat_map(|c| c.comps.clone()).collect();
        if self.anim_ui.view.fit_y || !self.anim_ui.view.fitted {
            let (lo, hi) = graph::value_range(&all);
            self.anim_ui.view.y_lo = lo;
            self.anim_ui.view.y_hi = hi;
            self.anim_ui.view.fitted = true;
        }
        let (y_lo, y_hi) = (self.anim_ui.view.y_lo, self.anim_ui.view.y_hi);
        let to_y = |val: f32| area.max.y - 6.0 - (val - y_lo) / (y_hi - y_lo).max(1e-6) * (area.height() - 12.0);
        let from_y = |y: f32| y_lo + (area.max.y - 6.0 - y) / (area.height() - 12.0).max(1.0) * (y_hi - y_lo);

        let cp = ui.painter_at(area);
        self.grid(&cp, area, axis);
        // linhas horizontais de valor
        let span_v = (y_hi - y_lo).max(1e-6);
        let raw_step = span_v / 6.0;
        let mag = 10f32.powf(raw_step.log10().floor());
        let vstep = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).find(|s| *s >= raw_step).unwrap_or(raw_step);
        let mut v = (y_lo / vstep).ceil() * vstep;
        while v < y_hi {
            let y = to_y(v);
            cp.line_segment([Pos2::new(area.min.x, y), Pos2::new(area.max.x, y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, if v.abs() < vstep * 0.01 { 40 } else { 12 })));
            cp.text(Pos2::new(area.min.x + 3.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{v:.2}"), FontId::proportional(9.5), theme::TEXT_DIM);
            v += vstep;
        }
        // curvas
        for c in &curves {
            let kind = c.target.value_kind();
            for (ci, pts) in c.comps.iter().enumerate() {
                let color = match kind {
                    ValueKind::Scalar => theme::ACCENT,
                    _ => AXIS_COLORS[ci % 5],
                };
                let line: Vec<Pos2> = pts.iter().map(|p| Pos2::new(axis.x(p[0]), to_y(p[1]))).collect();
                if line.len() > 1 {
                    cp.add(Shape::line(line, Stroke::new(1.6, color.gamma_multiply(0.95))));
                }
            }
        }
        // chaves e alças
        let mut key_hits: Vec<(Pos2, KeyId, usize)> = Vec::new();
        let mut handle_hits: Vec<(Pos2, Handle, KeyId, usize)> = Vec::new();
        if let Some(clip) = self.anim_clip() {
            for c in &curves {
                let Some(ch) = clip.channel(&c.target) else { continue };
                let comps = graph::key_components(ch);
                let ncomp = comps.first().map(|v| v.len()).unwrap_or(0);
                for (ki, key) in ch.keys.iter().enumerate() {
                    for ci in 0..ncomp {
                        let p = Pos2::new(axis.x(key.frame as f32), to_y(comps[ki][ci]));
                        if !area.expand(8.0).contains(p) {
                            continue;
                        }
                        let id: KeyId = (c.target.clone(), key.frame);
                        let selected = self.anim_ui.selection.contains(&id);
                        let color = if c.target.value_kind() == ValueKind::Scalar { theme::ACCENT } else { AXIS_COLORS[ci % 5] };
                        cp.rect_filled(Rect::from_center_size(p, egui::vec2(7.0, 7.0)), 1.0, if selected { theme::SELECT } else { color });
                        cp.rect_stroke(Rect::from_center_size(p, egui::vec2(7.0, 7.0)), 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
                        key_hits.push((p, id.clone(), ci));
                        // alças do segmento que sai desta chave, se a chave está selecionada e é Bézier
                        if selected && ki + 1 < ch.keys.len() {
                            let nxt = &ch.keys[ki + 1];
                            if let Interp::Bezier { .. } | Interp::Ease | Interp::EaseIn | Interp::EaseOut = key.interp {
                                let comp = graph::dominant_component(&key.value, &nxt.value);
                                if ci == comp {
                                    if let Some((p1, p2)) = graph::handle_points(key, nxt, comp, comps[ki][comp], comps[ki + 1][comp]) {
                                        let (s1, s2) = (Pos2::new(axis.x(p1[0]), to_y(p1[1])), Pos2::new(axis.x(p2[0]), to_y(p2[1])));
                                        let pn = Pos2::new(axis.x(nxt.frame as f32), to_y(comps[ki + 1][comp]));
                                        cp.line_segment([p, s1], Stroke::new(1.2, Color32::from_rgb(0xff, 0xd0, 0x40)));
                                        cp.line_segment([pn, s2], Stroke::new(1.2, Color32::from_rgb(0xff, 0xd0, 0x40)));
                                        for s in [s1, s2] {
                                            cp.circle_filled(s, 4.5, Color32::from_rgb(0xff, 0xd0, 0x40));
                                            cp.circle_stroke(s, 4.5, Stroke::new(1.0, Color32::BLACK));
                                        }
                                        handle_hits.push((s1, Handle::Out, id.clone(), comp));
                                        handle_hits.push((s2, Handle::In, id.clone(), comp));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.playhead_and_range(&cp, area, axis);
        let rp = ui.painter_at(ruler_rect);
        self.ruler(ui, &rp, ruler_rect, axis);

        // ---- coluna esquerda: lista de canais com olho (visibilidade)
        let lp = ui.painter_at(left);
        lp.rect_filled(Rect::from_min_size(left.min, egui::vec2(LEFT_W, ruler_h)), 0.0, Color32::from_rgb(0x1a, 0x1a, 0x20));
        lp.text(left.min + egui::vec2(8.0, ruler_h * 0.5), Align2::LEFT_CENTER, tr("anim.curves"), FontId::proportional(11.5), theme::TEXT_DIM);
        let left_area = Rect::from_min_max(Pos2::new(left.min.x, left.min.y + ruler_h), left.max);
        let lpa = lp.with_clip_rect(left_area);
        let row_h = self.anim_ui.row_h;
        let total = channels.len() as f32 * row_h;
        let max_scroll = (total - left_area.height()).max(0.0);
        self.anim_ui.view.scroll_y = self.anim_ui.view.scroll_y.min(max_scroll);
        let sy = self.anim_ui.view.scroll_y;
        let mut toggle: Option<ChannelTarget> = None;
        for (i, t) in channels.iter().enumerate() {
            let y = left_area.min.y + i as f32 * row_h - sy;
            if y + row_h < left_area.min.y || y > left_area.max.y {
                continue;
            }
            let rr = Rect::from_min_size(Pos2::new(left.min.x, y), egui::vec2(LEFT_W, row_h));
            let vis = self.anim_ui.graph_visible.contains(t);
            if left_resp.hover_pos().is_some_and(|p| rr.contains(p)) {
                lpa.rect_filled(rr, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12));
                if left_resp.clicked() {
                    toggle = Some(t.clone());
                }
            }
            lpa.text(Pos2::new(left.min.x + 8.0, y + row_h * 0.5), Align2::LEFT_CENTER, if vis { icons::EYE } else { icons::EYE_SLASH }, FontId::proportional(12.0), if vis { theme::ACCENT } else { theme::TEXT_DIM });
            lpa.text(Pos2::new(left.min.x + 28.0, y + row_h * 0.5), Align2::LEFT_CENTER, self.anim_channel_label(t), FontId::proportional(11.5), if vis { theme::TEXT } else { theme::TEXT_DIM });
        }
        if let Some(t) = toggle {
            if !self.anim_ui.graph_visible.remove(&t) {
                self.anim_ui.graph_visible.insert(t);
            }
            self.anim_ui.view.fitted = false;
        }

        // ---- gestos
        let mouse = resp.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos()));
        let (shift, ctrl, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let press = ctx.input(|i| i.pointer.press_origin());
        let near = |p: Pos2, q: Pos2, r: f32| (p - q).length() <= r;
        if self.anim_ui.drag.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_key_drag();
        } else {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(p) = press {
                    if ruler_rect.contains(p) {
                        self.anim_ui.scrubbing = true;
                    } else if let Some((_, which, id, comp)) = handle_hits.iter().find(|(h, ..)| near(*h, p, 8.0)).cloned() {
                        self.begin_key_drag(DragKind::Handle { which, key: id, comp }, axis.frame(p.x));
                    } else if let Some((_, id, _)) = key_hits.iter().find(|(k, ..)| near(*k, p, 7.0)).cloned() {
                        if !self.anim_ui.selection.contains(&id) {
                            if shift || ctrl {
                                self.anim_ui.selection.insert(id.clone());
                            } else {
                                self.anim_ui.selection = [id.clone()].into_iter().collect();
                            }
                        }
                        let kind = if alt { DragKind::Scale { pivot: self.anim_ui.transport.displayed(self.anim_span()) } } else if shift { DragKind::Duplicate } else { DragKind::Move };
                        self.begin_key_drag(kind, axis.frame(p.x));
                    } else {
                        self.anim_ui.marquee = Some(Marquee { start: p, cur: p, additive: shift || ctrl });
                    }
                }
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                if let Some(m) = mouse {
                    if self.anim_ui.scrubbing {
                        self.scrub_to(axis.frame(m.x), shift);
                    } else if self.anim_ui.drag.as_ref().is_some_and(|d| matches!(d.kind, DragKind::Handle { .. })) {
                        self.update_handle_drag(axis.frame(m.x), from_y(m.y));
                    } else if self.anim_ui.drag.is_some() {
                        self.update_key_drag(axis.frame(m.x));
                    } else if let Some(mq) = self.anim_ui.marquee.as_mut() {
                        mq.cur = m;
                    }
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                if self.anim_ui.scrubbing {
                    self.anim_ui.scrubbing = false;
                } else if self.anim_ui.drag.is_some() {
                    self.finish_key_drag();
                } else if let Some(mq) = self.anim_ui.marquee.take() {
                    let r = Rect::from_two_pos(mq.start, mq.cur);
                    if !mq.additive {
                        self.anim_ui.selection.clear();
                    }
                    for (p, id, _) in &key_hits {
                        if r.contains(*p) {
                            self.anim_ui.selection.insert(id.clone());
                        }
                    }
                }
            }
            if resp.clicked_by(egui::PointerButton::Primary) {
                if let Some(p) = resp.interact_pointer_pos() {
                    if ruler_rect.contains(p) {
                        self.scrub_to(axis.frame(p.x), shift);
                    } else if let Some((_, id, _)) = key_hits.iter().find(|(k, ..)| near(*k, p, 7.0)).cloned() {
                        if shift || ctrl {
                            if !self.anim_ui.selection.remove(&id) {
                                self.anim_ui.selection.insert(id);
                            }
                        } else {
                            self.anim_ui.selection = [id].into_iter().collect();
                        }
                    } else if !(shift || ctrl) {
                        self.anim_ui.selection.clear();
                    }
                }
            }
        }
        if let Some(m) = &self.anim_ui.marquee {
            if (m.cur - m.start).length() > 3.0 {
                let r = Rect::from_two_pos(m.start, m.cur);
                cp.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 40));
                cp.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
        }
        if curves.is_empty() {
            cp.text(area.center(), Align2::CENTER_CENTER, if self.anim_clip().is_some() { tr("anim.empty_clip") } else { tr("anim.no_clip") }, FontId::proportional(13.0), theme::TEXT_DIM);
        }
        self.anim_finish_gesture(&ctx);
    }

    /// Arrasta uma alça Bézier: `preview` do `SetInterpolation` com as alças novas; UM passo ao soltar.
    fn update_handle_drag(&mut self, frame: f32, value: f32) {
        let Some(mut d) = self.anim_ui.drag.take() else { return };
        let DragKind::Handle { which, key, comp } = d.kind.clone() else {
            self.anim_ui.drag = Some(d);
            return;
        };
        let Some(id) = self.anim_clip_id() else { return };
        let ch = d.base_clip.channel(&key.0);
        if let Some(ch) = ch {
            if let Some(ki) = ch.key_index(key.1) {
                if ki + 1 < ch.keys.len() {
                    let (a, b) = (ch.keys[ki], ch.keys[ki + 1]);
                    let comps = graph::key_components(ch);
                    let (av, bv) = (comps[ki][comp], comps[ki + 1][comp]);
                    let base = graph::as_bezier(a.interp);
                    if let Interp::Bezier { c } = base {
                        let nc = graph::drag_handle(c, which, &a, &b, av, bv, frame, value);
                        let edit = AnimEdit::SetInterpolation { clip_id: id, keys: vec![anigo_core::animation::KeyRef { target: key.0.clone(), frame: key.1 }], interp: Interp::Bezier { c: nc } };
                        self.core.cancel_preview();
                        self.core.begin_preview();
                        if self.core.preview(Command::EditAnimation { edit: edit.clone() }).is_ok() {
                            d.last = Some(edit);
                        }
                    }
                }
            }
        }
        self.anim_ui.drag = Some(d);
    }

    // ============================================================ clips

    fn clips_editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ids: Vec<_> = self.core.project().animation.clips.keys().cloned().collect();
        let active = self.anim_clip_id();
        ui.horizontal_top(|ui| {
            // ---- lista de clips do projeto
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() * 0.5 - 8.0).max(260.0));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("anim.clips")).strong());
                    if ui.button(format!("{}  {}", icons::PLUS, tr("anim.clip.new"))).clicked() {
                        self.anim_new_clip();
                    }
                });
                egui::ScrollArea::vertical().id_salt("anim_clip_list").auto_shrink([false, false]).show(ui, |ui| {
                    if ids.is_empty() {
                        ui.label(egui::RichText::new(tr("anim.no_clip")).color(theme::TEXT_DIM));
                    }
                    for id in &ids {
                        let Some(c) = self.core.project().animation.clips.get(id).cloned() else { continue };
                        let fps = self.anim_fps();
                        let (a, b) = c.effective_range(fps);
                        let is_active = active.as_ref() == Some(id);
                        ui.horizontal(|ui| {
                            let label = format!("{}  {}   {}  {}–{}  ·  {} {}", if is_active { icons::CHECK_CIRCLE } else { icons::CIRCLE }, c.name, icons::FILM_STRIP, a, b, c.key_count(), tr("anim.keys"));
                            if ui.selectable_label(is_active, label).clicked() {
                                self.anim_select_clip(id);
                            }
                            if ui.small_button(icons::COPY).on_hover_text(tr("anim.clip.duplicate")).clicked() {
                                self.anim_duplicate_clip(id);
                            }
                            if ui.small_button(icons::PENCIL_SIMPLE).on_hover_text(tr("anim.clip.rename")).clicked() {
                                self.anim_ui.list_rename_for = Some(id.clone());
                                self.anim_ui.list_rename_buf = c.name.clone();
                            }
                            if ui.small_button(icons::TRASH).on_hover_text(tr("anim.clip.delete")).clicked() {
                                self.anim_apply(AnimEdit::RemoveClip { clip_id: id.clone() });
                                self.anim_ui.selection.clear();
                            }
                        });
                        if self.anim_ui.list_rename_for.as_ref() == Some(id) {
                            ui.horizontal(|ui| {
                                let te = ui.add(egui::TextEdit::singleline(&mut self.anim_ui.list_rename_buf).desired_width(180.0));
                                let ok = ui.button(icons::CHECK).clicked() || (te.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)));
                                if ok {
                                    let name = self.anim_ui.list_rename_buf.trim().to_string();
                                    if !name.is_empty() {
                                        self.anim_apply(AnimEdit::RenameClip { clip_id: id.clone(), name });
                                    }
                                    self.anim_ui.list_rename_for = None;
                                }
                                if ui.button(icons::X).clicked() {
                                    self.anim_ui.list_rename_for = None;
                                }
                            });
                        }
                    }
                });
            });
            ui.separator();
            // ---- Biblioteca de clips
            ui.vertical(|ui| {
                self.clip_library_section(ui);
            });
        });
    }
}

fn row_y_vec(rows: &[Row], top: f32, scroll: f32, row_h: f32) -> Vec<f32> {
    (0..rows.len()).map(|i| top + i as f32 * row_h - scroll).collect()
}

fn sun_key(p: anigo_core::animation::SunProp) -> &'static str {
    match p {
        anigo_core::animation::SunProp::Intensity => "intensity",
        anigo_core::animation::SunProp::Direction => "direction",
        anigo_core::animation::SunProp::Color => "color",
    }
}

fn cam_key(p: anigo_core::animation::CameraProp) -> &'static str {
    match p {
        anigo_core::animation::CameraProp::Fov => "fov",
        anigo_core::animation::CameraProp::Eye => "eye",
        anigo_core::animation::CameraProp::Target => "target",
    }
}

/// Desenha uma chave: losango (linear), quadrado (degrau) ou círculo (curva); laranja quando selecionada.
fn draw_key(p: &egui::Painter, c: Pos2, interp: Interp, selected: bool, partly: bool, leaf: bool, current: bool) {
    let col = if selected {
        theme::SELECT
    } else if partly {
        Color32::from_rgb(0xff, 0xc8, 0x90)
    } else if leaf {
        KEY_COL
    } else {
        Color32::from_rgb(0xa8, 0xa0, 0xd8)
    };
    let r = if leaf { KEY_R } else { KEY_R + 0.5 };
    let stroke = Stroke::new(1.0, Color32::from_rgb(0x10, 0x10, 0x14));
    match interp {
        Interp::Step => {
            let rect = Rect::from_center_size(c, egui::vec2(r * 1.6, r * 1.6));
            p.rect_filled(rect, 1.0, col);
            p.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Outside);
        }
        Interp::Linear => {
            let pts = vec![c + egui::vec2(0.0, -r - 1.0), c + egui::vec2(r + 1.0, 0.0), c + egui::vec2(0.0, r + 1.0), c + egui::vec2(-r - 1.0, 0.0)];
            p.add(Shape::convex_polygon(pts, col, stroke));
        }
        _ => {
            p.circle_filled(c, r, col);
            p.circle_stroke(c, r, stroke);
        }
    }
    if current {
        p.circle_stroke(c, r + 3.0, Stroke::new(1.2, PLAYHEAD));
    }
}
