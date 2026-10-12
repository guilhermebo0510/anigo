//! Painéis do Shading: alvos + presets (esquerda) e parâmetros agrupados, rampa, paleta e texturas (direita).

use super::ramp::{ramp_editor, RampEdit};
use super::sources::TargetRow;
use super::{FaceMap, PresetTab, Source, StudioApp, Target};
use crate::i18n::tr;
use crate::icons;
use crate::shading::palette::{self, derive_shade, harmony_colors, hue_shift, Harmony, Palette, ShadeRecipe};
use crate::shading::params::{self, Group, Kind, Param, Value};
use crate::shading::presets::{self, BUILT_IN};
use crate::theme;
use anigo_core::npr::RampCustom;
use anigo_core::StylizedMaterial;
use egui::{Color32, RichText, Sense, TextureHandle};

/// Edição produzida por um controle: material novo e se é discreta (um passo agora) ou contínua (arrasto).
pub(crate) struct Edit {
    pub material: StylizedMaterial,
    pub discrete: bool,
}

fn rgb8(c: [f32; 4]) -> Color32 {
    Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8)
}

fn rgb8_3(c: [f32; 3]) -> Color32 {
    rgb8([c[0], c[1], c[2], 1.0])
}

fn full_width_slider(ui: &mut egui::Ui, v: &mut f32, min: f32, max: f32, decimals: usize) -> egui::Response {
    let w = (ui.available_width() - 74.0).max(80.0);
    ui.spacing_mut().slider_width = w;
    ui.add(egui::Slider::new(v, min..=max).clamping(egui::SliderClamping::Always).max_decimals(decimals).min_decimals(decimals.min(2)))
}

fn reset_btn(ui: &mut egui::Ui, enabled: bool, tip: &str) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).size(12.0)).small()).on_hover_text(tip)
}

fn swatch(ui: &mut egui::Ui, c: Color32, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
    ui.painter().rect_filled(r, 3.0, c);
    ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    resp
}

/// Uma linha de parâmetro. `link` (receita) faz a cor base arrastar a de sombra junto.
fn param_row(ui: &mut egui::Ui, p: &Param, m: &StylizedMaterial, link: Option<ShadeRecipe>, trace: &mut Vec<(String, egui::Rect)>) -> Option<Edit> {
    let cur = (p.get)(m);
    let modified = p.is_modified(m);
    let dim = !p.gate_on(m);
    let mut edit: Option<Edit> = None;
    ui.horizontal(|ui| {
        let mut text = RichText::new(tr(p.label)).size(12.5);
        if dim {
            text = text.color(theme::TEXT_DIM);
        } else if modified {
            text = text.color(theme::ACCENT);
        }
        let l = ui.add(egui::Label::new(text).truncate());
        let tip_key = format!("{}.tip", p.label);
        let tip = tr(&tip_key);
        if tip != tip_key {
            l.on_hover_text(tip);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if reset_btn(ui, modified, tr("shading.reset_param")).clicked() {
                let mut nm = m.clone();
                params::reset_param(&mut nm, p.id);
                if p.id == "base_color" {
                    if let Some(r) = link {
                        let s = derive_shade([nm.base_color[0], nm.base_color[1], nm.base_color[2]], r);
                        nm.shade_color = [s[0], s[1], s[2], nm.shade_color[3]];
                    }
                }
                edit = Some(Edit { material: nm, discrete: true });
            }
            if dim {
                ui.label(RichText::new(icons::LINK_BREAK).size(12.0).color(theme::TEXT_DIM)).on_hover_text(tr("shading.gate_off"));
            }
        });
    });
    match p.kind {
        Kind::Float { min, max, decimals } => {
            let mut v = cur.f();
            let resp = full_width_slider(ui, &mut v, min, max, decimals);
            if matches!(p.id, "rim_intensity" | "outline_width" | "ramp_blend") {
                trace.push((p.id.to_string(), resp.rect));
            }
            if resp.changed() {
                let mut nm = m.clone();
                p.write(&mut nm, Value::F(v));
                edit = Some(Edit { material: nm, discrete: false });
            }
        }
        Kind::Color => {
            let c = cur.c();
            let mut rgb = [c[0], c[1], c[2]];
            ui.horizontal(|ui| {
                let resp = ui.color_edit_button_rgb(&mut rgb);
                ui.label(RichText::new(palette::to_hex(rgb)).monospace().size(11.5).color(theme::TEXT_DIM));
                if resp.changed() {
                    let mut nm = m.clone();
                    p.write(&mut nm, Value::C([rgb[0], rgb[1], rgb[2], c[3]]));
                    if p.id == "base_color" {
                        if let Some(r) = link {
                            let s = derive_shade(rgb, r);
                            nm.shade_color = [s[0], s[1], s[2], nm.shade_color[3]];
                        }
                    }
                    edit = Some(Edit { material: nm, discrete: false });
                }
            });
        }
        Kind::Bool => {
            let mut b = cur.b();
            if ui.checkbox(&mut b, tr("shading.on")).changed() {
                let mut nm = m.clone();
                p.write(&mut nm, Value::B(b));
                edit = Some(Edit { material: nm, discrete: true });
            }
        }
        Kind::Choice(opts) => {
            let mut i = cur.i();
            ui.horizontal(|ui| {
                for (k, key) in opts.iter().enumerate() {
                    if ui.selectable_value(&mut i, k as u8, tr(key)).changed() {
                        let mut nm = m.clone();
                        p.write(&mut nm, Value::I(i));
                        edit = Some(Edit { material: nm, discrete: true });
                    }
                }
            });
        }
    }
    ui.add_space(2.0);
    edit
}

impl StudioApp {
    fn shading_apply_edit(&mut self, e: Edit) {
        if e.discrete {
            self.shading_apply_now(e.material);
        } else {
            self.shading_change(e.material);
        }
    }

    // =================================================================== esquerda

    pub(crate) fn shading_left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("shading_left").default_size(250.0).size_range(210.0..=380.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for s in Source::ALL {
                    if ui.selectable_label(self.shading_ui.source == s, tr(s.key())).clicked() {
                        self.shading_set_source(s);
                    }
                }
            });
            ui.separator();
            let h = (ui.available_height() * 0.38).clamp(110.0, 420.0);
            self.shading_target_list(ui, h);
            ui.separator();
            egui::ScrollArea::vertical().id_salt("shading_left_scroll").auto_shrink([false, false]).show(ui, |ui| self.shading_presets_section(ui));
        });
    }

    fn shading_target_list(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.label(RichText::new(tr("shading.targets")).strong().size(12.5));
        if self.shading_ui.source == Source::Cenario {
            ui.add(egui::TextEdit::singleline(&mut self.shading_ui.list_search).hint_text(tr("shading.search")).desired_width(f32::INFINITY));
        }
        let q = self.shading_ui.list_search.trim().to_lowercase();
        let rows: Vec<TargetRow> = self.shading_targets(self.shading_ui.source).into_iter().filter(|r| q.is_empty() || r.label.to_lowercase().contains(&q) || self.shading_ui.source != Source::Cenario).collect();
        if rows.is_empty() {
            ui.label(RichText::new(tr(match self.shading_ui.source {
                Source::Modelagem => "shading.empty_modeling",
                Source::Cenario => "shading.empty_scene",
                Source::Personagem => "shading.empty_character",
            })).color(theme::TEXT_DIM).size(12.0));
            return;
        }
        let mut pick: Option<Target> = None;
        egui::ScrollArea::vertical().id_salt("shading_targets").max_height(height).auto_shrink([false, true]).show_rows(ui, 24.0, rows.len(), |ui, range| {
            for r in &rows[range] {
                let selected = self.shading_ui.target.as_ref() == Some(&r.target);
                ui.horizontal(|ui| {
                    ui.add_space(r.depth as f32 * 12.0);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 16.0), Sense::hover());
                    ui.painter().rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.center().x, rect.max.y)), 3.0, rgb8(r.swatch.0));
                    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.center().x, rect.min.y), rect.max), 3.0, rgb8(r.swatch.1));
                    let resp = ui.selectable_label(selected, format!("{}  {}", r.icon, r.label));
                    if resp.clicked() {
                        pick = Some(r.target.clone());
                    }
                });
            }
        });
        if let Some(t) = pick {
            self.shading_select(t);
        }
    }

    // ------------------------------------------------------------------ presets

    fn shading_presets_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("shading.presets")).strong().size(12.5));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.shading_ui.preset_tab, PresetTab::BuiltIn, tr("shading.presets.builtin"));
            ui.selectable_value(&mut self.shading_ui.preset_tab, PresetTab::Library, tr("shading.presets.library"));
        });
        let ctx = ui.ctx().clone();
        let mut budget = 2;
        match self.shading_ui.preset_tab {
            PresetTab::BuiltIn => {
                let cols = ((ui.available_width() / 82.0).floor() as usize).max(1);
                egui::Grid::new("shading_builtin").num_columns(cols).spacing([6.0, 8.0]).show(ui, |ui| {
                    for (i, p) in BUILT_IN.iter().enumerate() {
                        let m = (p.build)();
                        let key = format!("bi:{}", p.id);
                        let tex = self.shading_thumb(&ctx, &key, &m, &mut budget);
                        ui.vertical(|ui| {
                            let r = self.shading_thumb_button(ui, tex.as_ref(), tr(&format!("shading.preset.{}", p.id)));
                            if r.clicked() {
                                let mut nm = m.clone();
                                if let Some(cur) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
                                    nm.name = cur.name;
                                }
                                self.shading_apply_now(nm);
                            }
                        });
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            }
            PresetTab::Library => self.shading_library_presets(ui, &ctx, &mut budget),
        }
        if budget < 2 {
            ctx.request_repaint();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(format!("{}  {}", icons::COPY, tr("shading.copy"))).on_hover_text("Ctrl+C").clicked() {
                self.shading_copy();
            }
            if ui.add_enabled(self.shading_ui.clipboard.is_some(), egui::Button::new(format!("{}  {}", icons::CLIPBOARD_TEXT, tr("shading.paste")))).on_hover_text("Ctrl+V").clicked() {
                self.shading_paste();
            }
        });
    }

    fn shading_thumb_button(&self, ui: &mut egui::Ui, tex: Option<&TextureHandle>, label: &str) -> egui::Response {
        let size = egui::vec2(72.0, 72.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        ui.painter().rect_filled(rect, 4.0, theme::BG_DEEP);
        if let Some(t) = tex {
            ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        if resp.hovered() {
            ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Outside);
        }
        ui.add(egui::Label::new(RichText::new(label).size(11.0)).truncate());
        resp.on_hover_text(label)
    }

    /// Miniatura em shader ball (renderer cel canônico); no máximo `budget` renders por quadro.
    fn shading_thumb(&mut self, ctx: &egui::Context, key: &str, m: &StylizedMaterial, budget: &mut i32) -> Option<TextureHandle> {
        if let Some(t) = self.shading_ui.thumbs.get(key) {
            return Some(t.clone());
        }
        if *budget <= 0 || self.shading_ui.thumb_failed.contains(key) {
            return None;
        }
        *budget -= 1;
        let cel = self.shading_ui.cel_thumbs.get_or_insert_with(|| std::sync::Arc::new(crate::library_ui::cel_thumb::CelThumbnailRenderer::new())).clone();
        let prep = crate::library_ui::cel_thumb::PreparedMesh::new((*crate::shading::ball::ball_mesh()).clone()).ok()?;
        match cel.render_prepared(&prep, crate::library_ui::cel_thumb::ViewParams::default(), Some(m.clone()), 96) {
            Ok(img) => {
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                let h = ctx.load_texture(format!("shading-thumb-{key}"), ci, egui::TextureOptions::LINEAR);
                self.shading_ui.thumbs.insert(key.to_string(), h.clone());
                Some(h)
            }
            Err(e) => {
                tracing::warn!("miniatura do preset {key}: {e}");
                self.shading_ui.thumb_failed.insert(key.to_string());
                None
            }
        }
    }

    fn shading_library_presets(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, budget: &mut i32) {
        let lib = self.lib_ws.as_ref().and_then(|w| w.library().cloned());
        let Some(lib) = lib else {
            ui.label(RichText::new(tr("shading.lib_opening")).color(theme::TEXT_DIM).size(12.0));
            return;
        };
        // salvar o material atual como preset
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.shading_ui.preset_name).hint_text(tr("shading.preset_name")).desired_width(ui.available_width() - 80.0));
            let can = !self.shading_ui.preset_name.trim().is_empty() && self.shading_ui.target.is_some();
            if ui.add_enabled(can, egui::Button::new(format!("{}  {}", icons::FLOPPY_DISK, tr("shading.save")))).on_hover_text(tr("shading.save_preset_tip")).clicked() {
                self.shading_save_preset(&lib);
            }
        });
        if self.shading_ui.preset_rows_for.is_none() {
            self.shading_ui.preset_rows = presets::list_presets(&lib);
            self.shading_ui.preset_rows_for = Some(1);
        }
        if self.shading_ui.preset_rows.is_empty() {
            ui.label(RichText::new(tr("shading.presets.empty")).color(theme::TEXT_DIM).size(12.0));
            return;
        }
        let rows = self.shading_ui.preset_rows.clone();
        let cols = ((ui.available_width() / 82.0).floor() as usize).max(1);
        let mut apply: Option<anigo_assets::AssetId> = None;
        egui::Grid::new("shading_libpresets").num_columns(cols).spacing([6.0, 8.0]).show(ui, |ui| {
            for (i, r) in rows.iter().enumerate() {
                let key = format!("lib:{}:{}", r.id, r.hash.short());
                let tex = match self.shading_ui.thumbs.get(&key).cloned() {
                    Some(t) => Some(t),
                    None if *budget > 0 && !self.shading_ui.thumb_failed.contains(&key) => {
                        *budget -= 1;
                        let t = self.shading_library_thumb(ctx, &lib, &key, &r.id);
                        if t.is_none() {
                            self.shading_ui.thumb_failed.insert(key.clone());
                        }
                        t
                    }
                    None => None,
                };
                ui.vertical(|ui| {
                    if self.shading_thumb_button(ui, tex.as_ref(), &r.name).clicked() {
                        apply = Some(r.id);
                    }
                });
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
        if let Some(id) = apply {
            match presets::load_preset_with_refs(&lib, &id) {
                Ok((mut m, refs)) => {
                    if let Some(cur) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
                        m.name = cur.name;
                    }
                    self.shading_apply_now(m);
                    // referências de textura gravadas no preset voltam para os slots
                    for (slot, uri) in refs {
                        let parsed = uri.strip_prefix("anigo://library/").and_then(|u| u.parse::<anigo_assets::AssetId>().ok());
                        if let Some(aid) = parsed {
                            let name = lib.row(&aid).ok().flatten().map(|r| r.name).unwrap_or_else(|| uri.clone());
                            self.shading_ui.tex_refs.insert(slot, (aid, name));
                        }
                    }
                }
                Err(e) => self.shading_ui.message = format!("{}: {e}", tr("char.error")),
            }
        }
    }

    /// Miniatura de um ativo `Material` da Biblioteca (cache de disco da própria biblioteca + shader ball 3D).
    fn shading_library_thumb(&mut self, ctx: &egui::Context, lib: &anigo_assets::Library, key: &str, id: &anigo_assets::AssetId) -> Option<TextureHandle> {
        let cel = self.shading_ui.cel_thumbs.get_or_insert_with(|| std::sync::Arc::new(crate::library_ui::cel_thumb::CelThumbnailRenderer::new())).clone();
        let req = lib.thumb_request(id).ok()?;
        let cache = lib.thumbnails();
        cache.ensure(&req, &*cel).ok()?;
        let img = cache.load(&req.content_hash, 128)?;
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let h = ctx.load_texture(format!("shading-thumb-{key}"), ci, egui::TextureOptions::LINEAR);
        self.shading_ui.thumbs.insert(key.to_string(), h.clone());
        Some(h)
    }

    pub(crate) fn shading_save_preset(&mut self, lib: &anigo_assets::Library) {
        let Some(m) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) else { return };
        let name = self.shading_ui.preset_name.trim().to_string();
        match presets::save_preset(lib, &name, &m, &self.shading_texture_refs()) {
            Ok(_) => {
                self.shading_ui.message = format!("{}: {name}", tr("shading.preset_saved"));
                self.shading_ui.preset_rows_for = None;
                self.shading_ui.preset_tab = PresetTab::Library;
            }
            Err(e) => self.shading_ui.message = format!("{}: {e}", tr("char.error")),
        }
    }

    /// Referências de textura escolhidas (slot → `anigo://library/<uuid>`), gravadas no preset.
    pub(crate) fn shading_texture_refs(&self) -> std::collections::BTreeMap<String, String> {
        self.shading_ui.tex_refs.iter().map(|(k, (id, _))| (k.clone(), format!("anigo://library/{id}"))).collect()
    }

    // =================================================================== direita

    pub(crate) fn shading_right_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("shading_right").default_size(350.0).size_range(300.0..=540.0).show(ui, |ui| {
            ui.add_space(4.0);
            self.shading_header(ui);
            ui.separator();
            let Some(target) = self.shading_ui.target.clone() else {
                ui.add_space(12.0);
                ui.label(RichText::new(tr("shading.no_target")).color(theme::TEXT_DIM));
                return;
            };
            let Some(mat) = self.shading_read(&target) else { return };
            let mut trace_rects: Vec<(String, egui::Rect)> = Vec::new();
            let mut edit: Option<Edit> = None;
            egui::ScrollArea::vertical().id_salt("shading_params").auto_shrink([false, false]).show(ui, |ui| {
                if self.shading_ui.palette_open {
                    if let Some(e) = self.shading_palette_section(ui, &mat) {
                        edit = Some(e);
                    }
                }
                let link = self.shading_ui.link_shade.then_some(self.shading_ui.recipe);
                for g in Group::ALL {
                    if let Some(e) = self.shading_group(ui, g, &mat, link, &mut trace_rects) {
                        edit = Some(e);
                    }
                }
                self.shading_textures_section(ui);
            });
            if let Some(e) = edit {
                self.shading_apply_edit(e);
            }
            if std::env::var_os("ANIGO_TRACE_SHADING").is_some() {
                for (id, r) in trace_rects {
                    let line = format!("shaderect {id} x={:.0} y={:.0} w={:.0} h={:.0}", r.min.x, r.min.y, r.width(), r.height());
                    if self.shading_ui.rect_log.get(&id) != Some(&line) {
                        eprintln!("{line}");
                        self.shading_ui.rect_log.insert(id, line);
                    }
                }
            }
        });
    }

    fn shading_header(&mut self, ui: &mut egui::Ui) {
        let name = self.shading_ui.target.as_ref().map(|t| self.shading_name(t)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  {}", icons::PALETTE, name)).strong().size(14.0));
        });
        ui.horizontal_wrapped(|ui| {
            let has_target = self.shading_ui.target.is_some();
            let (can_undo, can_redo) = self.shading_can_undo_redo();
            if ui.add_enabled(can_undo, egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE)).on_hover_text(format!("{} (Ctrl+Z)", tr("action.undo"))).clicked() {
                self.undo_redo(true);
            }
            if ui.add_enabled(can_redo, egui::Button::new(icons::ARROW_CLOCKWISE)).on_hover_text(format!("{} (Ctrl+Y)", tr("action.redo"))).clicked() {
                self.undo_redo(false);
            }
            ui.separator();
            if ui.add_enabled(has_target, egui::Button::new(format!("{}  {}", icons::SPARKLE, tr("shading.anime_on")))).on_hover_text(tr("shading.anime_on_tip")).clicked() {
                if let Some(mut m) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
                    params::enable_anime_features(&mut m);
                    self.shading_apply_now(m);
                }
            }
            let any_mod = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)).is_some_and(|m| {
                let d = StylizedMaterial::default();
                Group::ALL.iter().any(|g| params::group_modified_count(&m, *g) > 0) || m.base_color != d.base_color
            });
            if ui.add_enabled(any_mod, egui::Button::new(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("shading.reset_all")))).on_hover_text(tr("shading.reset_all_tip")).clicked() {
                if let Some(cur) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
                    let mut m = StylizedMaterial { name: cur.name, ..Default::default() };
                    // o gaze (solver de olhar) não é editado aqui: preserva
                    m.gaze_tracking_enabled = cur.gaze_tracking_enabled;
                    m.gaze_saccade_amplitude = cur.gaze_saccade_amplitude;
                    m.gaze_damping = cur.gaze_damping;
                    self.shading_apply_now(m);
                }
            }
        });
        if !self.shading_ui.message.is_empty() {
            ui.label(RichText::new(&self.shading_ui.message).size(11.5).color(theme::TEXT_DIM));
        }
    }

    fn shading_group(&mut self, ui: &mut egui::Ui, g: Group, mat: &StylizedMaterial, link: Option<ShadeRecipe>, trace: &mut Vec<(String, egui::Rect)>) -> Option<Edit> {
        let mut edit: Option<Edit> = None;
        let modified = params::group_modified_count(mat, g);
        let id = ui.make_persistent_id(("shading_group", g.id()));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, matches!(g, Group::Base));
        let focus = self.shading_ui.focus_group == Some(g);
        if focus {
            state.set_open(true);
        }
        let header = state.show_header(ui, |ui| {
            let mut title = RichText::new(tr(g.label_key())).strong();
            if modified > 0 {
                title = title.color(theme::ACCENT);
            }
            ui.label(title);
            if modified > 0 {
                ui.label(RichText::new(format!("· {modified}")).size(11.5).color(theme::ACCENT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reset_btn(ui, modified > 0, tr("shading.reset_group")).clicked() {
                    let mut nm = mat.clone();
                    params::reset_group(&mut nm, g);
                    edit = Some(Edit { material: nm, discrete: true });
                }
            });
        });
        let (_, head_resp, _) = header.body(|ui| {
            // dicas por grupo
            let hint_key = format!("shading.g.{}.hint", g.id());
            let hint = tr(&hint_key);
            if hint != hint_key {
                ui.label(RichText::new(hint).size(11.5).color(theme::TEXT_DIM));
            }
            for p in params::of_group(g) {
                if let Some(e) = param_row(ui, p, mat, if g == Group::Base { link } else { None }, trace) {
                    edit = Some(e);
                }
                if g == Group::Base && p.id == "shade_color" {
                    if let Some(e) = self.shading_shade_derive_row(ui, mat) {
                        edit = Some(e);
                    }
                }
                if g == Group::Ramp && p.id == "ramp_row" {
                    // atalhos: linha 0..3 exata
                    ui.horizontal(|ui| {
                        for r in 0..4 {
                            let on = (mat.npr.ramp_row - r as f32).abs() < 1e-3;
                            if ui.selectable_label(on, format!("{r}")).clicked() && !on {
                                let mut nm = mat.clone();
                                nm.npr.ramp_row = r as f32;
                                edit = Some(Edit { material: nm, discrete: true });
                            }
                        }
                        ui.label(RichText::new(tr("shading.ramp.rows_hint")).size(11.0).color(theme::TEXT_DIM));
                    });
                }
            }
            if g == Group::Ramp {
                ui.add_space(4.0);
                let mut sel = self.shading_ui.ramp_sel;
                match ramp_editor(ui, mat, &mut sel) {
                    RampEdit::None => {}
                    RampEdit::Live(m) => edit = Some(Edit { material: *m, discrete: false }),
                    RampEdit::Apply(m) => edit = Some(Edit { material: *m, discrete: true }),
                }
                self.shading_ui.ramp_sel = sel;
                if mat.npr.ramp_blend <= 0.0 {
                    ui.label(RichText::new(tr("shading.ramp.off_hint")).size(11.5).color(Color32::from_rgb(0xff, 0xb4, 0x5a)));
                }
            }
            if g == Group::Face {
                self.shading_face_map_ui(ui, mat);
            }
        });
        if focus {
            ui.scroll_to_rect(head_resp.response.rect, Some(egui::Align::TOP));
            self.shading_ui.focus_group = None;
        }
        ui.add_space(2.0);
        edit
    }

    /// Linha "derivar sombra da base" (HSV) sob a cor de sombra.
    fn shading_shade_derive_row(&mut self, ui: &mut egui::Ui, mat: &StylizedMaterial) -> Option<Edit> {
        let mut edit = None;
        ui.horizontal(|ui| {
            if ui.button(format!("{}  {}", icons::MAGIC_WAND, tr("shading.derive_shade"))).on_hover_text(tr("shading.derive_shade_tip")).clicked() {
                let s = derive_shade([mat.base_color[0], mat.base_color[1], mat.base_color[2]], self.shading_ui.recipe);
                let mut nm = mat.clone();
                nm.shade_color = [s[0], s[1], s[2], nm.shade_color[3]];
                edit = Some(Edit { material: nm, discrete: true });
            }
            ui.checkbox(&mut self.shading_ui.link_shade, tr("shading.link_shade")).on_hover_text(tr("shading.link_shade_tip"));
            if ui.small_button(icons::SLIDERS).on_hover_text(tr("shading.palette")).clicked() {
                self.shading_ui.palette_open = true;
            }
        });
        edit
    }

    // ------------------------------------------------------------------ face SDF

    fn shading_face_map_ui(&mut self, ui: &mut egui::Ui, mat: &StylizedMaterial) {
        ui.add_space(4.0);
        ui.label(RichText::new(tr("shading.face.map")).strong().size(12.0));
        let current = match &self.shading_ui.face_map {
            FaceMap::None => tr("shading.face.none").to_string(),
            FaceMap::Procedural => tr("shading.face.procedural").to_string(),
            FaceMap::Library(_, n) => n.clone(),
        };
        ui.label(RichText::new(current).size(11.5).color(theme::TEXT_DIM));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("shading.face.use_procedural")).on_hover_text(tr("shading.face.procedural_tip")).clicked() {
                self.shading_install_face_map(FaceMap::Procedural);
                if !mat.face_sdf_enabled {
                    let mut nm = mat.clone();
                    nm.face_sdf_enabled = true;
                    self.shading_apply_now(nm);
                }
            }
            if ui.add_enabled(self.shading_ui.face_map != FaceMap::None, egui::Button::new(tr("shading.face.clear"))).clicked() {
                self.shading_install_face_map(FaceMap::None);
            }
            if ui.button(tr("shading.face.mirror_on")).on_hover_text(tr("shading.face.mirror_tip")).clicked() {
                let mut nm = mat.clone();
                nm.npr.face_sdf_mirror_center = 0.25;
                self.shading_apply_now(nm);
            }
        });
        if let Some(e) = &self.shading_ui.face_map_error {
            ui.label(RichText::new(e).size(11.5).color(Color32::from_rgb(0xff, 0x70, 0x70)));
        }
        self.shading_texture_picker(ui, "face_sdf", true);
    }

    // ------------------------------------------------------------------ texturas

    fn shading_textures_section(&mut self, ui: &mut egui::Ui) {
        let id = ui.make_persistent_id("shading_textures");
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        state
            .show_header(ui, |ui| {
                ui.label(RichText::new(tr("shading.g.textures")).strong());
            })
            .body(|ui| {
                ui.label(RichText::new(tr("shading.tex.hint")).size(11.5).color(theme::TEXT_DIM));
                for (slot, label, live) in [
                    ("base", "shading.tex.base", false),
                    ("shade", "shading.tex.shade", false),
                    ("shade2", "shading.tex.shade2", false),
                    ("ramp", "shading.tex.ramp", false),
                    ("ilm", "shading.tex.ilm", false),
                    ("matcap", "shading.tex.matcap", false),
                    ("emission", "shading.tex.emission", false),
                    ("outline_width", "shading.tex.outline_width", false),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(tr(label));
                        ui.label(RichText::new(if live { tr("shading.tex.live") } else { tr("shading.tex.ref_only") }).size(11.0).color(if live { theme::ACCENT } else { theme::TEXT_DIM }));
                    });
                    self.shading_texture_picker(ui, slot, live);
                }
                ui.label(RichText::new(tr("shading.tex.alpha")).size(11.5).color(theme::TEXT_DIM));
            });
    }

    /// Seletor de textura da Biblioteca para um slot. `live` = o renderer usa de fato (mapa SDF facial).
    fn shading_texture_picker(&mut self, ui: &mut egui::Ui, slot: &str, live: bool) {
        let lib = self.lib_ws.as_ref().and_then(|w| w.library().cloned());
        let cur = self.shading_ui.tex_refs.get(slot).map(|(_, n)| n.clone());
        let label = cur.clone().unwrap_or_else(|| tr("shading.tex.choose").to_string());
        let mut choose: Option<(anigo_assets::AssetId, String)> = None;
        let mut clear = false;
        egui::ComboBox::from_id_salt(("shading_tex", slot)).selected_text(label).width(ui.available_width() - 40.0).show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.shading_ui.tex_search).hint_text(tr("shading.search")).desired_width(180.0));
            let q = self.shading_ui.tex_search.trim().to_string();
            let Some(lib) = &lib else {
                ui.label(tr("shading.lib_opening"));
                return;
            };
            let query = anigo_assets::Query { kinds: vec![anigo_assets::AssetKind::Texture], text: (!q.is_empty()).then_some(q), limit: 60, ..Default::default() };
            match lib.query(&query) {
                Ok(page) => {
                    if page.items.is_empty() {
                        ui.label(RichText::new(tr("shading.tex.none_in_library")).color(theme::TEXT_DIM));
                    }
                    for r in page.items {
                        if ui.selectable_label(false, &r.name).clicked() {
                            choose = Some((r.id, r.name));
                        }
                    }
                }
                Err(e) => {
                    ui.label(e.to_string());
                }
            }
            if ui.button(tr("shading.tex.clear")).clicked() {
                clear = true;
            }
        });
        if let Some((id, name)) = choose {
            self.shading_ui.tex_refs.insert(slot.to_string(), (id, name.clone()));
            if live && slot == "face_sdf" {
                self.shading_install_face_map(FaceMap::Library(id, name));
            }
        }
        if clear {
            self.shading_ui.tex_refs.remove(slot);
            if slot == "face_sdf" {
                self.shading_install_face_map(FaceMap::None);
            }
        }
    }

    // ------------------------------------------------------------------ paleta

    pub(crate) fn shading_load_project_palette(&mut self) {
        self.shading_ui.palette_loaded = true;
        let path = Palette::new("projeto").path_in(&self.shading_ui.palette_dir);
        if path.exists() {
            if let Ok(p) = Palette::load(&path) {
                self.shading_ui.palette = p;
            }
        }
    }

    fn shading_save_project_palette(&mut self) {
        let mut p = self.shading_ui.palette.clone();
        p.name = "projeto".into();
        if let Err(e) = p.save(&self.shading_ui.palette_dir) {
            self.shading_ui.message = e;
        }
    }

    fn shading_palette_section(&mut self, ui: &mut egui::Ui, mat: &StylizedMaterial) -> Option<Edit> {
        let mut edit: Option<Edit> = None;
        let id = ui.make_persistent_id("shading_palette");
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        if self.shading_ui.palette_open {
            state.set_open(true);
        }
        state
            .show_header(ui, |ui| {
                ui.label(RichText::new(format!("{}  {}", icons::PALETTE, tr("shading.palette"))).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(icons::X).clicked() {
                        self.shading_ui.palette_open = false;
                    }
                });
            })
            .body(|ui| {
                // ---- sombra derivada (HSV)
                ui.label(RichText::new(tr("shading.palette.derive")).strong().size(12.0));
                let mut r = self.shading_ui.recipe;
                let mut changed = false;
                changed |= ui.add(egui::Slider::new(&mut r.hue_shift_deg, -90.0..=90.0).text(tr("shading.palette.hue")).integer()).changed();
                changed |= ui.add(egui::Slider::new(&mut r.saturation, 0.3..=2.5).text(tr("shading.palette.sat")).max_decimals(2)).changed();
                changed |= ui.add(egui::Slider::new(&mut r.value, 0.2..=1.2).text(tr("shading.palette.val")).max_decimals(2)).changed();
                if changed {
                    self.shading_ui.recipe = r;
                    let s = derive_shade([mat.base_color[0], mat.base_color[1], mat.base_color[2]], r);
                    let mut nm = mat.clone();
                    nm.shade_color = [s[0], s[1], s[2], nm.shade_color[3]];
                    edit = Some(Edit { material: nm, discrete: false });
                }
                ui.label(RichText::new(tr("shading.palette.derive_hint")).size(11.0).color(theme::TEXT_DIM));
                // ---- girar o matiz da base e da sombra
                ui.add_space(4.0);
                let mut turn = self.shading_ui.hue_turn;
                let tr_resp = ui.add(egui::Slider::new(&mut turn, -180.0..=180.0).text(tr("shading.palette.turn")).integer());
                if tr_resp.changed() {
                    let start = self.shading_ui.hue_start.get_or_insert_with(|| mat.clone()).clone();
                    let mut nm = start.clone();
                    for (dst, src) in [(&mut nm.base_color, start.base_color), (&mut nm.shade_color, start.shade_color)] {
                        let c = hue_shift([src[0], src[1], src[2]], turn);
                        *dst = [c[0], c[1], c[2], src[3]];
                    }
                    self.shading_ui.hue_turn = turn;
                    edit = Some(Edit { material: nm, discrete: false });
                }
                if !ui.input(|i| i.pointer.any_down()) && self.shading_ui.hue_start.is_some() && !tr_resp.dragged() {
                    self.shading_ui.hue_start = None;
                    self.shading_ui.hue_turn = 0.0;
                }
                // ---- harmonias
                ui.add_space(6.0);
                ui.label(RichText::new(tr("shading.palette.harmony")).strong().size(12.0));
                ui.horizontal_wrapped(|ui| {
                    for h in Harmony::ALL {
                        ui.selectable_value(&mut self.shading_ui.harmony, h, tr(h.label_key()));
                    }
                });
                let base = [mat.base_color[0], mat.base_color[1], mat.base_color[2]];
                let cols = harmony_colors(base, self.shading_ui.harmony);
                ui.horizontal_wrapped(|ui| {
                    for c in &cols {
                        let resp = swatch(ui, rgb8_3(*c), 28.0).on_hover_text(format!("{}\n{}", palette::to_hex(*c), tr("shading.palette.swatch_tip")));
                        if let Some(e) = self.shading_swatch_actions(&resp, *c, mat) {
                            edit = Some(e);
                        }
                    }
                });
                // ---- paleta do projeto
                ui.add_space(6.0);
                ui.label(RichText::new(tr("shading.palette.project")).strong().size(12.0));
                ui.horizontal_wrapped(|ui| {
                    let colors = self.shading_ui.palette.colors.clone();
                    for (i, c) in colors.iter().enumerate() {
                        let resp = swatch(ui, rgb8_3(*c), 22.0).on_hover_text(palette::to_hex(*c));
                        if let Some(e) = self.shading_swatch_actions(&resp, *c, mat) {
                            edit = Some(e);
                        }
                        resp.context_menu(|ui| {
                            if ui.button(tr("shading.palette.remove")).clicked() {
                                self.shading_ui.palette.colors.remove(i);
                                self.shading_save_project_palette();
                                ui.close();
                            }
                        });
                    }
                    if colors.is_empty() {
                        ui.label(RichText::new(tr("shading.palette.empty")).size(11.5).color(theme::TEXT_DIM));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  {}", icons::PLUS, tr("shading.palette.add_base"))).clicked() {
                        self.shading_ui.palette.add(base);
                        self.shading_save_project_palette();
                    }
                    if ui.button(format!("{}  {}", icons::PLUS, tr("shading.palette.add_shade"))).clicked() {
                        self.shading_ui.palette.add([mat.shade_color[0], mat.shade_color[1], mat.shade_color[2]]);
                        self.shading_save_project_palette();
                    }
                    if ui.button(format!("{}  {}", icons::PLUS, tr("shading.palette.add_harmony"))).clicked() {
                        for c in &cols {
                            self.shading_ui.palette.add(*c);
                        }
                        self.shading_save_project_palette();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.shading_ui.palette_name).desired_width(120.0).hint_text(tr("shading.palette.name")));
                    if ui.button(format!("{}  {}", icons::FLOPPY_DISK, tr("shading.save"))).clicked() {
                        let mut p = self.shading_ui.palette.clone();
                        p.name = self.shading_ui.palette_name.clone();
                        match p.save(&self.shading_ui.palette_dir) {
                            Ok(path) => self.shading_ui.message = format!("{}: {}", tr("shading.palette.saved"), path.display()),
                            Err(e) => self.shading_ui.message = e,
                        }
                    }
                });
                let saved = palette::list_palettes(&self.shading_ui.palette_dir);
                if !saved.is_empty() {
                    ui.label(RichText::new(tr("shading.palette.saved_list")).size(11.5).color(theme::TEXT_DIM));
                    ui.horizontal_wrapped(|ui| {
                        for (name, path) in saved {
                            if name == "projeto" {
                                continue;
                            }
                            if ui.small_button(&name).on_hover_text(tr("shading.palette.load_tip")).clicked() {
                                match Palette::load(&path) {
                                    Ok(p) => {
                                        self.shading_ui.palette = p;
                                        self.shading_save_project_palette();
                                    }
                                    Err(e) => self.shading_ui.message = e,
                                }
                            }
                        }
                    });
                }
            });
        edit
    }

    /// Clique = cor base; menu de contexto = outras funções (sombra, contorno, rim, emissão, paleta).
    fn shading_swatch_actions(&mut self, resp: &egui::Response, c: [f32; 3], mat: &StylizedMaterial) -> Option<Edit> {
        let mut edit = None;
        let set = |f: &dyn Fn(&mut StylizedMaterial)| {
            let mut nm = mat.clone();
            f(&mut nm);
            Some(Edit { material: nm, discrete: true })
        };
        if resp.clicked() {
            edit = set(&|m| m.base_color = [c[0], c[1], c[2], m.base_color[3]]);
        }
        let mut add = false;
        resp.context_menu(|ui| {
            if ui.button(tr("shading.palette.use_base")).clicked() {
                edit = set(&|m| m.base_color = [c[0], c[1], c[2], m.base_color[3]]);
                ui.close();
            }
            if ui.button(tr("shading.palette.use_shade")).clicked() {
                edit = set(&|m| m.shade_color = [c[0], c[1], c[2], m.shade_color[3]]);
                ui.close();
            }
            if ui.button(tr("shading.palette.use_outline")).clicked() {
                edit = set(&|m| m.outline_color = [c[0], c[1], c[2], m.outline_color[3]]);
                ui.close();
            }
            if ui.button(tr("shading.palette.use_rim")).clicked() {
                edit = set(&|m| m.rim_color = [c[0], c[1], c[2], m.rim_color[3]]);
                ui.close();
            }
            if ui.button(tr("shading.palette.use_emission")).clicked() {
                edit = set(&|m| m.mtoon_emission_color = [c[0], c[1], c[2], m.mtoon_emission_color[3]]);
                ui.close();
            }
            ui.separator();
            if ui.button(tr("shading.palette.add")).clicked() {
                add = true;
                ui.close();
            }
        });
        if add {
            self.shading_ui.palette.add(c);
            self.shading_save_project_palette();
        }
        edit
    }
}

#[allow(dead_code)]
fn _keep(_: RampCustom) {}
