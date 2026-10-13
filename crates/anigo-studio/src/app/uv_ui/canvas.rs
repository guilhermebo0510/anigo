//! Painel 2D do editor de UV: desenho (egui painter), entrada (zoom/pan/seleção/G-R-S) e barra de ferramentas.

use super::geom::{self, AxisLock, UvView, Xform, XformKind};
use super::{cache, CheckerMode, ColorMode, UvAction, UvCache, UvModal};
use crate::app::StudioApp;
use crate::icons;
use crate::modeling::Mode;
use crate::theme;
use anigo_modeling::convert::triangulate_polygon;
use anigo_modeling::{edge_key, SelectMode};
use egui::{Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, RichText, Shape, Stroke};
use glam::{Vec2, Vec3};

const SELECT_FILL: Color32 = Color32::from_rgba_premultiplied(120, 72, 22, 120);
const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x15);
const BG_UNIT: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x21);
const WARN: Color32 = Color32::from_rgb(0xff, 0x6a, 0x5a);
const OK: Color32 = Color32::from_rgb(0x6c, 0xd0, 0x8a);
/// Raio de captura (px) de vértices/arestas.
const PICK_PX: f32 = 9.0;

fn p2(v: Vec2) -> Pos2 {
    Pos2::new(v.x, v.y)
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn rgb(c: [u8; 3], a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], a)
}

impl StudioApp {
    /// Recalcula as métricas se a malha/UVs/textura mudaram (barato quando nada mudou: só hashes).
    pub(crate) fn uv_refresh_cache(&mut self) {
        let Some(obj) = self.modeling.active_object() else {
            self.uv.cache = None;
            return;
        };
        let key = cache::cache_key(obj.id, &obj.mesh, self.uv.params.tex_size);
        if self.uv.cache.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        // malhas muito densas: não recalcular a cada quadro durante arrastos
        let busy = self.uv.modal.is_some() || self.modeling.modal_active();
        if busy && obj.mesh.faces.len() > 20_000 && self.uv.cache.as_ref().is_some_and(|c| c.key.0 == obj.id) {
            return;
        }
        self.uv.cache = Some(UvCache::build(key, &obj.mesh));
    }

    pub(super) fn uv_editor_impl(&mut self, ui: &mut egui::Ui) {
        self.uv_refresh_cache();
        self.uv_toolbar(ui);
        self.uv_stats_row(ui);
        ui.add_space(2.0);
        self.uv_canvas(ui);
    }

    // ------------------------------------------------------------------------- barra

    fn uv_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut act: Option<UvAction> = None;
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{}  Editor de UV", icons::GRID_FOUR)).strong());
            ui.separator();
            ui.label("Cor");
            egui::ComboBox::from_id_salt("uv_color_mode")
                .selected_text(match self.uv.color_mode {
                    ColorMode::Islands => "Ilhas",
                    ColorMode::Distortion => "Distorção",
                    ColorMode::Density => "Densidade",
                })
                .width(92.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.uv.color_mode, ColorMode::Islands, "Ilhas");
                    ui.selectable_value(&mut self.uv.color_mode, ColorMode::Distortion, "Distorção (estiramento)");
                    ui.selectable_value(&mut self.uv.color_mode, ColorMode::Density, "Densidade de texel");
                });
            ui.label("Fundo");
            egui::ComboBox::from_id_salt("uv_checker")
                .selected_text(match self.uv.checker {
                    CheckerMode::Off => "Liso",
                    CheckerMode::C8 => "Xadrez 8x8",
                    CheckerMode::C64 => "Xadrez 64x64",
                })
                .width(100.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.uv.checker, CheckerMode::Off, "Liso");
                    ui.selectable_value(&mut self.uv.checker, CheckerMode::C8, "Xadrez 8x8");
                    ui.selectable_value(&mut self.uv.checker, CheckerMode::C64, "Xadrez 64x64");
                });
            ui.checkbox(&mut self.uv.show_overlaps, "Sobreposições");
            ui.checkbox(&mut self.uv.show_grid, "Grade");
            if ui.button(icons::FRAME_CORNERS).on_hover_text("Enquadrar (Home)").clicked() {
                self.uv.fit_request = true;
            }
            ui.separator();
            if ui.button(icons::FLIP_HORIZONTAL).on_hover_text("Espelhar em U").clicked() {
                act = Some(UvAction::FlipU);
            }
            if ui.button(icons::FLIP_VERTICAL).on_hover_text("Espelhar em V").clicked() {
                act = Some(UvAction::FlipV);
            }
            if ui.button(icons::ARROWS_CLOCKWISE).on_hover_text("Girar 90°").clicked() {
                act = Some(UvAction::Rotate90);
            }
            ui.separator();
            if ui.button(format!("{} Unwrap", icons::CUBE_FOCUS)).clicked() {
                act = Some(UvAction::Unwrap);
            }
            if ui.button(format!("{} Empacotar", icons::PACKAGE)).clicked() {
                act = Some(UvAction::Pack);
            }
        });
        if let Some(a) = act {
            self.run_uv_action(a);
        }
    }

    fn uv_stats_row(&self, ui: &mut egui::Ui) {
        let dim = |s: String| RichText::new(s).size(12.0).color(theme::TEXT_DIM);
        let flagged = |label: &str, n: usize| {
            let c = if n == 0 { OK } else { WARN };
            RichText::new(format!("{label} {n}")).size(12.0).color(c)
        };
        ui.horizontal_wrapped(|ui| match self.uv.cache.as_ref().map(|c| &c.report) {
            Some(Ok(r)) => {
                let mx = if r.max_stretch.is_finite() { format!("{:.2}", r.max_stretch) } else { "∞".into() };
                ui.label(dim(format!("Estiramento {:.3} (máx {mx})", r.mean_stretch)));
                ui.label(flagged("· Invertidas", r.flipped_faces));
                ui.label(flagged("· Sobrepostas", r.overlapping_island_pairs + r.self_overlapping_faces));
                if r.outside_unit_square > 0 {
                    ui.label(flagged("· Fora de [0,1]", r.outside_unit_square));
                }
                if let Some(d) = r.texel_density {
                    ui.label(dim(format!("· Densidade {:.0} px/u (±{:.0}) a {} px", d.mean, d.std_dev, self.uv.params.tex_size)));
                }
                ui.label(dim(format!("· Ilhas {} · Ocupação {:.0}%", r.island_count, r.uv_area * 100.0)));
                let n = self.modeling.selection.faces.len();
                let v = self.modeling.selection.verts.len();
                ui.label(dim(format!("· Seleção: {n} faces, {v} vértices")));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).size(12.0).color(WARN));
            }
            None => {
                ui.label(dim("Sem objeto ativo".into()));
            }
        });
    }

    // ------------------------------------------------------------------------- textura de xadrez

    fn uv_checker_texture(&mut self, ctx: &egui::Context, cells: usize) -> egui::TextureId {
        let slot = if cells <= 8 { 0 } else { 1 };
        if self.uv.tex_checker[slot].is_none() {
            const PX: usize = 512;
            let bytes: Vec<u8> = geom::checker_pixels(cells, PX).into_iter().flatten().collect();
            let img = egui::ColorImage::from_rgb([PX, PX], &bytes);
            self.uv.tex_checker[slot] = Some(ctx.load_texture(format!("uv_checker_{cells}"), img, egui::TextureOptions::NEAREST));
        }
        self.uv.tex_checker[slot].as_ref().map(|t| t.id()).unwrap_or(egui::TextureId::default())
    }

    // ------------------------------------------------------------------------- canvas

    fn uv_canvas(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let (resp, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        let origin = v2(rect.center());
        let size = Vec2::new(rect.width().max(1.0), rect.height().max(1.0));

        // enquadramento inicial / pedido
        if !self.uv.view_inited || self.uv.fit_request {
            let (mut lo, mut hi) = (Vec2::ZERO, Vec2::ONE);
            if self.uv.fit_request {
                if let Some(obj) = self.modeling.active_object() {
                    if let Some((a, b)) = anigo_modeling::uv::metrics::uv_bounds(&obj.mesh) {
                        if a.is_finite() && b.is_finite() {
                            lo = lo.min(a);
                            hi = hi.max(b);
                        }
                    }
                }
            }
            self.uv.view = UvView::frame(lo, hi, size);
            self.uv.view_inited = true;
            self.uv.fit_request = false;
        }

        painter.rect_filled(rect, 0.0, BG);
        if self.modeling.active_object().is_none() || self.modeling.mode != Mode::Edit {
            painter.text(rect.center(), Align2::CENTER_CENTER, "Selecione um objeto e entre no Modo Edição", FontId::proportional(14.0), theme::TEXT_DIM);
            return;
        }

        // ---- entrada
        let pointer = ui.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos()));
        let mouse_uv = pointer.map(|p| self.uv.view.to_uv(v2(p), origin));
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                if let Some(p) = pointer {
                    self.uv.view.zoom_at((scroll * 0.0025).exp(), v2(p), origin);
                }
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            let d = ui.input(|i| i.pointer.delta());
            self.uv.view.pan_px(Vec2::new(d.x, d.y));
        }
        if self.uv.modal.is_some() {
            self.uv_modal_step(ui, mouse_uv);
        } else {
            self.uv_select_input(ui, &resp, mouse_uv, origin);
        }

        // ---- desenho (depois de aplicar a entrada: mostra o estado mais novo)
        self.uv_refresh_cache();
        let view = self.uv.view;
        let to_s = |uv: Vec2| p2(view.to_screen(uv, origin));
        let ctx = ui.ctx().clone();

        // quadrado unitário + textura
        let unit = Rect::from_two_pos(to_s(Vec2::new(0.0, 0.0)), to_s(Vec2::new(1.0, 1.0)));
        painter.rect_filled(unit, 0.0, BG_UNIT);
        if let Some(cells) = self.uv.checker.cells() {
            let tex = self.uv_checker_texture(&ctx, cells);
            painter.image(tex, unit, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        if self.uv.show_grid {
            draw_grid(&painter, rect, &view, origin);
        }

        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let mesh = &obj.mesh;
        let sel = &self.modeling.selection;
        let sel_mode = self.modeling.select_mode;
        let cache = self.uv.cache.as_ref();
        let checker_on = self.uv.checker != CheckerMode::Off;
        let alpha = if checker_on { 120 } else { 170 };
        let face_count = mesh.faces.len();

        // preenchimento das faces numa única malha
        let mut fill = egui::Mesh::default();
        let mut screen_polys: Vec<Option<Vec<Pos2>>> = Vec::with_capacity(face_count);
        for (fi, face) in mesh.faces.iter().enumerate() {
            if face.hidden || face.corners.len() < 3 {
                screen_polys.push(None);
                continue;
            }
            let pts: Vec<Pos2> = face.corners.iter().map(|c| to_s(c.uv)).collect();
            let (mut lo, mut hi) = (pts[0], pts[0]);
            for p in &pts {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            if !Rect::from_min_max(lo, hi).intersects(rect) {
                screen_polys.push(None);
                continue;
            }
            let (flipped, overlapped) = cache
                .map(|c| (c.flipped.get(fi).copied().unwrap_or(false), c.overlap.get(fi).copied().unwrap_or(false)))
                .unwrap_or((false, false));
            let mut color = match (self.uv.color_mode, cache) {
                (ColorMode::Islands, Some(c)) => rgb(geom::island_rgb(c.island.get(fi).copied().unwrap_or(0)), alpha),
                (ColorMode::Distortion, Some(c)) => rgb(geom::stretch_rgb(c.stretch.get(fi).copied().unwrap_or(1.0)), alpha + 30),
                (ColorMode::Density, Some(c)) => {
                    let d = c.density.get(fi).copied().unwrap_or(0.0);
                    rgb(geom::density_rgb(d / c.mean_density.max(1e-6)), alpha + 30)
                }
                _ => rgb([120, 120, 130], alpha),
            };
            if flipped {
                color = Color32::from_rgba_unmultiplied(235, 40, 40, 215);
            } else if overlapped && self.uv.show_overlaps {
                color = Color32::from_rgba_unmultiplied(255, 90, 60, 190);
            }
            add_polygon(&mut fill, &pts, color);
            if sel.faces.contains(&(fi as u32)) {
                add_polygon(&mut fill, &pts, SELECT_FILL);
            }
            screen_polys.push(Some(pts));
        }
        painter.add(Shape::mesh(fill));

        // arestas
        let border: std::collections::HashSet<(u32, u16)> = cache.map(|c| c.borders.iter().copied().collect()).unwrap_or_default();
        let mut shapes: Vec<Shape> = Vec::new();
        let mut selected_edges: Vec<Shape> = Vec::new();
        for (fi, face) in mesh.faces.iter().enumerate() {
            let Some(pts) = screen_polys.get(fi).and_then(|p| p.as_ref()) else {
                continue;
            };
            let n = pts.len();
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (va, vb) = (face.corners[i].vert, face.corners[(i + 1) % n].vert);
                let selected = match sel_mode {
                    SelectMode::Face => sel.faces.contains(&(fi as u32)),
                    SelectMode::Edge => sel.edges.contains(&edge_key(va, vb)),
                    SelectMode::Vertex => sel.verts.contains(&va) && sel.verts.contains(&vb),
                };
                if selected {
                    selected_edges.push(Shape::line_segment([a, b], Stroke::new(2.0, theme::SELECT)));
                } else if border.contains(&(fi as u32, i as u16)) {
                    shapes.push(Shape::line_segment([a, b], Stroke::new(1.6, Color32::from_rgba_unmultiplied(238, 238, 246, 235))));
                } else {
                    shapes.push(Shape::line_segment([a, b], Stroke::new(1.0, Color32::from_rgba_unmultiplied(8, 8, 12, 150))));
                }
            }
        }
        painter.extend(shapes);
        painter.extend(selected_edges);

        // cantos
        let total_corners: usize = mesh.faces.iter().map(|f| f.corners.len()).sum();
        if total_corners <= 40_000 {
            for (fi, face) in mesh.faces.iter().enumerate() {
                let Some(pts) = screen_polys.get(fi).and_then(|p| p.as_ref()) else {
                    continue;
                };
                for (ci, c) in face.corners.iter().enumerate() {
                    let selected = geom::corner_selected(sel, sel_mode, fi as u32, c.vert);
                    if selected && sel_mode != SelectMode::Face {
                        painter.circle_filled(pts[ci], 3.4, theme::SELECT);
                        painter.circle_stroke(pts[ci], 3.4, Stroke::new(1.0, Color32::BLACK));
                    } else if sel_mode == SelectMode::Vertex {
                        painter.circle_filled(pts[ci], 2.2, Color32::from_rgb(190, 190, 202));
                    }
                }
            }
        }

        // moldura do quadrado unitário + rótulos
        painter.rect_stroke(unit, 0.0, Stroke::new(1.5, Color32::from_rgb(0xc4, 0xc4, 0xd4)), egui::StrokeKind::Middle);
        let lab = FontId::proportional(11.0);
        painter.text(unit.left_bottom() + egui::vec2(-4.0, 3.0), Align2::RIGHT_TOP, "0", lab.clone(), theme::TEXT_DIM);
        painter.text(unit.right_bottom() + egui::vec2(0.0, 3.0), Align2::CENTER_TOP, "U 1", lab.clone(), theme::TEXT_DIM);
        painter.text(unit.left_top() + egui::vec2(-4.0, 0.0), Align2::RIGHT_TOP, "V 1", lab, theme::TEXT_DIM);

        // caixa de seleção
        if let (Some(start), true) = (self.uv.box_start, ui.input(|i| i.pointer.primary_down())) {
            if let Some(cur) = pointer {
                let r = Rect::from_two_pos(p2(start), cur);
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 154, 46, 28));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::SELECT), egui::StrokeKind::Inside);
            }
        }

        // pivô e eixos da transformação
        if let Some(m) = &self.uv.modal {
            let pv = to_s(m.xf.pivot);
            painter.circle_stroke(pv, 5.0, Stroke::new(1.5, Color32::WHITE));
            if let Some(p) = pointer {
                painter.line_segment([pv, p], Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90)));
            }
            let col = |l: AxisLock, me: AxisLock| if l == me { theme::SELECT } else { Color32::from_rgba_unmultiplied(255, 255, 255, 40) };
            painter.line_segment([Pos2::new(rect.left(), pv.y), Pos2::new(rect.right(), pv.y)], Stroke::new(1.0, col(m.xf.lock, AxisLock::U)));
            painter.line_segment([Pos2::new(pv.x, rect.top()), Pos2::new(pv.x, rect.bottom())], Stroke::new(1.0, col(m.xf.lock, AxisLock::V)));
        }

        // legenda e dicas
        if self.uv.color_mode != ColorMode::Islands {
            draw_legend(&painter, rect, self.uv.color_mode);
        }
        let hint = "Roda: zoom · MMB: mover vista · clique/caixa: selecionar (Shift soma, Alt/duplo: ilha) · G/R/S: mover/girar/escalar · Home: enquadrar";
        painter.text(rect.left_top() + egui::vec2(8.0, 5.0), Align2::LEFT_TOP, hint, FontId::proportional(11.0), Color32::from_rgba_unmultiplied(160, 160, 172, 170));
        painter.text(
            rect.right_bottom() + egui::vec2(-8.0, -22.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} px/u", self.uv.view.zoom),
            FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(160, 160, 172, 170),
        );

        // face sob o cursor (informações em tempo real)
        self.uv.hover_face = None;
        if resp.hovered() && self.uv.modal.is_none() {
            if let (Some(uv), Some(p)) = (mouse_uv, pointer) {
                if let Some(f) = geom::pick_face(mesh, uv) {
                    self.uv.hover_face = Some(f);
                    if let Some(pts) = screen_polys.get(f as usize).and_then(|p| p.as_ref()) {
                        painter.add(Shape::closed_line(pts.clone(), Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 200))));
                    }
                    if let Some(c) = cache {
                        let s = c.stretch.get(f as usize).copied().unwrap_or(1.0);
                        let d = c.density.get(f as usize).copied().unwrap_or(0.0);
                        let txt = format!(
                            "Face {f} · estiramento {} · {:.0} px/u{}{}",
                            if s.is_finite() { format!("{s:.2}") } else { "∞".into() },
                            d,
                            if c.flipped.get(f as usize).copied().unwrap_or(false) { " · INVERTIDA" } else { "" },
                            if c.overlap.get(f as usize).copied().unwrap_or(false) { " · sobreposta" } else { "" },
                        );
                        draw_tip(&painter, rect, p, &txt);
                    }
                }
            }
        }
        // rastreio para testes de entrada reais (xdotool): ANIGO_TRACE_UV=<face alvo>
        if let Ok(v) = std::env::var("ANIGO_TRACE_UV") {
            let target: u32 = v.parse().unwrap_or(0);
            let c = mesh.faces.get(target as usize).map(|f| f.corners.iter().map(|c| c.uv).sum::<Vec2>() / f.corners.len().max(1) as f32);
            let at = c.map(|c| view.to_screen(c, origin)).unwrap_or(Vec2::ZERO);
            let line = format!(
                "uvtrace ppp={} face{target}_px={:.0},{:.0} sel_faces={:?} sel_verts={}",
                ctx.pixels_per_point(),
                at.x,
                at.y,
                sel.faces,
                sel.verts.len()
            );
            if line != self.uv.trace_last {
                eprintln!("{line}");
                self.uv.trace_last = line;
            }
        }
        if resp.hovered() || self.uv.modal.is_some() {
            ctx.request_repaint();
        }
    }

    // ------------------------------------------------------------------------- seleção e atalhos

    fn uv_hit_selected(&self, p: Vec2, radius: f32) -> bool {
        let Some(obj) = self.modeling.active_object() else {
            return false;
        };
        let (mesh, sel) = (&obj.mesh, &self.modeling.selection);
        match self.modeling.select_mode {
            SelectMode::Face => geom::pick_face(mesh, p).is_some_and(|f| sel.faces.contains(&f)),
            SelectMode::Vertex => geom::pick_vert(mesh, p, radius).is_some_and(|v| sel.verts.contains(&v)),
            SelectMode::Edge => geom::pick_edge(mesh, p, radius).is_some_and(|e| sel.edges.contains(&e)),
        }
    }

    fn uv_click(&mut self, p: Vec2, radius: f32, extend: bool, island: bool) {
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let (sel, active) = geom::click_selection(&obj.mesh, &self.modeling.selection, self.modeling.select_mode, p, radius, extend, island);
        self.modeling.set_selection(sel, active.or(self.modeling.active_vert));
    }

    fn uv_box(&mut self, a: Vec2, b: Vec2, extend: bool) {
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let boxed = geom::box_select(&obj.mesh, self.modeling.select_mode, a.min(b), a.max(b));
        let sel = if extend { geom::merge_selection(&self.modeling.selection, &boxed) } else { boxed };
        self.modeling.set_selection(sel, None);
    }

    fn uv_select_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, mouse_uv: Option<Vec2>, origin: Vec2) {
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let radius = PICK_PX / self.uv.view.zoom;
        let view = self.uv.view;
        let uv_at = |p: Pos2| view.to_uv(v2(p), origin);

        if resp.drag_started_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                let start_uv = uv_at(p);
                if !shift && !alt && self.uv_hit_selected(start_uv, radius) {
                    self.uv_begin_modal(XformKind::Grab, start_uv, true);
                } else {
                    self.uv.box_start = Some(v2(p));
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let (Some(start), Some(end)) = (self.uv.box_start.take(), ui.input(|i| i.pointer.latest_pos())) {
                if (v2(end) - start).length() > 4.0 {
                    let a = view.to_uv(start, origin);
                    self.uv_box(a, uv_at(end), shift);
                } else {
                    self.uv_click(uv_at(end), radius, shift, alt);
                }
            }
        } else if resp.clicked_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                self.uv_click(uv_at(p), radius, shift, alt);
            }
            self.uv.box_start = None;
        }
        if resp.double_clicked_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                self.uv_click(uv_at(p), radius, shift, true);
            }
        }

        // ---- teclas (só com o ponteiro sobre o editor e sem campo de texto focado)
        if !resp.hovered() || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let km = self.keymap.clone();
        let consume = |ui: &mut egui::Ui, id: &str| km.consume(ui.ctx(), id);
        if consume(ui, "edit.redo") {
            self.modeling.redo();
            return;
        }
        if consume(ui, "edit.undo") {
            self.modeling.undo();
            return;
        }
        let at = mouse_uv.unwrap_or(Vec2::splat(0.5));
        for (id, kind) in [("modelagem.uv.grab", XformKind::Grab), ("modelagem.uv.rotate", XformKind::Rotate), ("modelagem.uv.scale", XformKind::Scale)] {
            if consume(ui, id) {
                self.uv_begin_modal(kind, at, false);
                return;
            }
        }
        if consume(ui, "modelagem.uv.select_none") {
            self.modeling.select_none();
        } else if consume(ui, "modelagem.uv.select_all") {
            self.modeling.select_all();
        }
        if consume(ui, "modelagem.uv.select_invert") {
            self.modeling.select_invert();
        }
        if consume(ui, "modelagem.uv.select_linked") {
            self.uv_click(at, radius, true, true);
        }
        for (id, m) in [("modelagem.uv.vertex_mode", SelectMode::Vertex), ("modelagem.uv.edge_mode", SelectMode::Edge), ("modelagem.uv.face_mode", SelectMode::Face)] {
            if consume(ui, id) {
                self.modeling.set_select_mode(m);
            }
        }
        if consume(ui, "modelagem.uv.fit") {
            self.uv.fit_request = true;
        }
        if consume(ui, "modelagem.uv.frame_selection") {
            self.uv_frame_selection(ui);
        }
    }

    fn uv_frame_selection(&mut self, ui: &egui::Ui) {
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let corners = geom::selected_corners(&obj.mesh, &self.modeling.selection, self.modeling.select_mode);
        if let Some((lo, hi)) = geom::corners_bounds(&obj.mesh, &corners) {
            let size = ui.available_size();
            self.uv.view = UvView::frame(lo - Vec2::splat(0.02), hi + Vec2::splat(0.02), Vec2::new(size.x.max(64.0), size.y.max(64.0)));
        }
    }

    // ------------------------------------------------------------------------- G/R/S

    fn uv_begin_modal(&mut self, kind: XformKind, at: Vec2, drag: bool) -> bool {
        let Some(obj) = self.modeling.active_object() else {
            return false;
        };
        let corners = geom::selected_corners(&obj.mesh, &self.modeling.selection, self.modeling.select_mode);
        let Some((lo, hi)) = geom::corners_bounds(&obj.mesh, &corners) else {
            self.modeling.status = "UV: nada selecionado".into();
            return false;
        };
        let orig: Vec<(u32, usize, Vec2)> = corners
            .iter()
            .filter_map(|&(f, c)| obj.mesh.faces.get(f as usize)?.corners.get(c).map(|x| (f, c, x.uv)))
            .collect();
        let modal = UvModal {
            xf: Xform::new(kind, (lo + hi) * 0.5, at),
            orig,
            before: self.modeling.capture(),
            drag,
            num_buf: String::new(),
            obj_id: obj.id,
            geo_hash: obj.mesh.content_hash(),
        };
        self.uv.modal = Some(modal);
        true
    }

    fn uv_modal_step(&mut self, ui: &mut egui::Ui, mouse_uv: Option<Vec2>) {
        let Some(m) = self.uv.modal.as_mut() else {
            return;
        };
        let valid = self.modeling.active_object().is_some_and(|o| o.id == m.obj_id && o.mesh.content_hash() == m.geo_hash);
        if !valid {
            self.uv.modal = None;
            return;
        }
        let (enter, esc, back, lmb, lmb_up, rmb, x, y, text) = ui.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Backspace),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.secondary_pressed(),
                i.key_pressed(Key::X),
                i.key_pressed(Key::Y),
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect::<String>(),
            )
        });
        if x {
            m.xf.toggle_lock(AxisLock::U);
        }
        if y {
            m.xf.toggle_lock(AxisLock::V);
        }
        for ch in text.chars() {
            if ch.is_ascii_digit() || ch == '.' || ch == '-' {
                m.num_buf.push(ch);
            } else if ch == ',' {
                m.num_buf.push('.');
            }
        }
        if back {
            m.num_buf.pop();
        }
        m.xf.numeric = m.num_buf.parse::<f32>().ok();
        let cur = mouse_uv.unwrap_or(m.xf.start);
        if let Some(obj) = self.modeling.active_object_mut() {
            for &(f, c, uv0) in &m.orig {
                if let Some(corner) = obj.mesh.faces.get_mut(f as usize).and_then(|x| x.corners.get_mut(c)) {
                    corner.uv = m.xf.map(cur, uv0);
                }
            }
        }
        self.modeling.status = m.xf.describe(cur);
        let confirm = enter || (m.drag && lmb_up) || (!m.drag && lmb);
        let cancel = esc || rmb;
        if confirm {
            if let Some(m) = self.uv.modal.take() {
                let label = m.xf.kind.label();
                self.modeling.commit_snapshot(label, m.before);
                self.modeling.status = format!("{label}: ok");
            }
        } else if cancel {
            if let Some(m) = self.uv.modal.take() {
                self.modeling.restore_snapshot(m.before);
                self.modeling.status.clear();
            }
        }
        ui.ctx().request_repaint();
    }
}

// ------------------------------------------------------------------------------- desenho auxiliar

fn add_polygon(mesh: &mut egui::Mesh, pts: &[Pos2], color: Color32) {
    let base = mesh.vertices.len() as u32;
    for p in pts {
        mesh.colored_vertex(*p, color);
    }
    let flat: Vec<Vec3> = pts.iter().map(|p| Vec3::new(p.x, p.y, 0.0)).collect();
    for [a, b, c] in triangulate_polygon(&flat) {
        mesh.add_triangle(base + a as u32, base + b as u32, base + c as u32);
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, view: &UvView, origin: Vec2) {
    let lo = view.to_uv(Vec2::new(rect.left(), rect.bottom()), origin);
    let hi = view.to_uv(Vec2::new(rect.right(), rect.top()), origin);
    // linhas inteiras (tiles) sempre; subdivisão de 1/8 e 1/64 conforme o zoom
    let steps: [(f32, u8); 3] = [(1.0, 70), (0.125, 36), (0.015_625, 18)];
    for (step, alpha) in steps {
        if step * view.zoom < 14.0 {
            continue;
        }
        let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(170, 170, 200, alpha));
        let n0 = (lo.x / step).floor() as i64;
        let n1 = (hi.x / step).ceil() as i64;
        if (n1 - n0) > 400 {
            continue;
        }
        for n in n0..=n1 {
            let x = view.to_screen(Vec2::new(n as f32 * step, 0.0), origin).x;
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], stroke);
        }
        let m0 = (lo.y / step).floor() as i64;
        let m1 = (hi.y / step).ceil() as i64;
        if (m1 - m0) > 400 {
            continue;
        }
        for n in m0..=m1 {
            let y = view.to_screen(Vec2::new(0.0, n as f32 * step), origin).y;
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
        }
    }
}

fn draw_legend(painter: &egui::Painter, rect: Rect, mode: ColorMode) {
    let (w, h) = (150.0, 10.0);
    let x0 = rect.right() - w - 14.0;
    let y0 = rect.top() + 22.0;
    let n = 30;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let c = match mode {
            ColorMode::Distortion => geom::stretch_rgb(1.0 + t * 1.2),
            _ => geom::density_rgb(2.0f32.powf((t * 2.0 - 1.0) * 1.5)),
        };
        let r = Rect::from_min_size(Pos2::new(x0 + t * (w - w / n as f32), y0), egui::vec2(w / n as f32 + 0.5, h));
        painter.rect_filled(r, 0.0, rgb(c, 255));
    }
    let font = FontId::proportional(10.5);
    let (a, b, title) = match mode {
        ColorMode::Distortion => ("1.0", "2.2+", "Estiramento"),
        _ => ("0.35x", "2.8x", "Densidade vs. média"),
    };
    painter.text(Pos2::new(x0, y0 - 3.0), Align2::LEFT_BOTTOM, title, font.clone(), theme::TEXT_DIM);
    painter.text(Pos2::new(x0, y0 + h + 2.0), Align2::LEFT_TOP, a, font.clone(), theme::TEXT_DIM);
    painter.text(Pos2::new(x0 + w, y0 + h + 2.0), Align2::RIGHT_TOP, b, font, theme::TEXT_DIM);
}

fn draw_tip(painter: &egui::Painter, clip: Rect, at: Pos2, text: &str) {
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(text.to_string(), font, theme::TEXT);
    let mut pos = at + egui::vec2(14.0, 16.0);
    if pos.x + galley.size().x + 10.0 > clip.right() {
        pos.x = at.x - galley.size().x - 18.0;
    }
    if pos.y + galley.size().y + 8.0 > clip.bottom() {
        pos.y = at.y - galley.size().y - 14.0;
    }
    let r = Rect::from_min_size(pos - egui::vec2(5.0, 3.0), galley.size() + egui::vec2(10.0, 6.0));
    painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 235));
    painter.rect_stroke(r, 4.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    painter.galley(pos, galley, theme::TEXT);
}
