//! Painel direito: metadados reais do ativo selecionado (do manifesto/índice), edição de nome, tags, nota e
//! descrição, dependências, materiais toon canônicos, preview 3D girável e as ações de gestão
//! (editar na Modelagem, duplicar, lixeira, restaurar, excluir definitivamente, relink).

use super::grid::CardAction;
use super::widgets::*;
use super::*;
use crate::i18n::tr;
use crate::icons;
use anigo_assets::material::AlphaMode;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Ui, Vec2};

#[derive(Default)]
pub struct DetailsUi {
    pub for_key: Option<(AssetId, i64)>,
    pub name: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub source_url: String,
    pub new_tag: String,
    pub batch_tag: String,
}

impl DetailsUi {
    pub fn reset_edit(&mut self) {
        self.for_key = None;
        self.new_tag.clear();
    }
}

fn lin_to_srgb8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn swatch(ui: &mut Ui, lin: [f32; 3], tip: &str) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(26.0, 14.0), Sense::hover());
    let col = Color32::from_rgb(lin_to_srgb8(lin[0]), lin_to_srgb8(lin[1]), lin_to_srgb8(lin[2]));
    ui.painter().rect_filled(rect, CornerRadius::same(3), col);
    ui.painter().rect_stroke(rect, CornerRadius::same(3), egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    resp.on_hover_text(format!("{tip}  #{:02x}{:02x}{:02x}  (linear {:.3}, {:.3}, {:.3})", col.r(), col.g(), col.b(), lin[0], lin[1], lin[2]));
}

fn kv(ui: &mut Ui, k: &str, v: impl Into<String>) {
    ui.label(small_dim(k));
    ui.add(egui::Label::new(RichText::new(v).size(12.0)).wrap());
    ui.end_row();
}

fn commit_field(ui: &mut Ui, value: &mut String, multiline: bool, hint: &str) -> bool {
    let resp = if multiline {
        ui.add(egui::TextEdit::multiline(value).desired_rows(3).desired_width(f32::INFINITY).hint_text(hint))
    } else {
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY).hint_text(hint))
    };
    resp.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape))
}

impl LibraryWorkspace {
    pub(super) fn right_panel(&mut self, ui: &mut Ui) {
        egui::Panel::right("lib_right").default_size(330.0).size_range(260.0..=560.0).resizable(true).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("lib_right_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(6.0);
                match self.selection.len() {
                    0 => self.details_empty(ui),
                    1 => self.details_single(ui),
                    _ => self.details_multi(ui),
                }
                ui.add_space(12.0);
            });
        });
    }

    fn details_empty(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(RichText::new(icons::CURSOR_CLICK).size(34.0).color(theme::ACCENT_DIM));
            ui.label(dim(tr("lib.det.select")));
        });
        ui.add_space(14.0);
        ui.separator();
        ui.label(RichText::new(tr("lib.det.shortcuts")).strong().size(12.0));
        for (k, v) in [
            ("Ctrl+A", "lib.sc.select_all"),
            ("Ctrl/Shift + clique", "lib.sc.multi"),
            ("\u{2190} \u{2191} \u{2192} \u{2193}", "lib.sc.arrows"),
            ("Enter / 2\u{00d7}clique", "lib.sc.edit"),
            ("F", "lib.sc.fav"),
            ("Del", "lib.sc.del"),
            ("/  Ctrl+F", "lib.sc.search"),
            ("Esc", "lib.sc.clear"),
        ] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(k).monospace().size(11.0).color(theme::ACCENT));
                ui.label(small_dim(tr(v)));
            });
        }
    }

    fn sync_edit_buffers(&mut self, ctx: &egui::Context, d: &DetailData) {
        let key = (d.detail.row.id, d.detail.row.modified_at);
        if self.details_ui.for_key != Some(key) && ctx.memory(|m| m.focused().is_none()) {
            let m = &d.detail.manifest;
            self.details_ui.name = m.name.clone();
            self.details_ui.description = m.description.clone();
            self.details_ui.author = m.author.clone();
            self.details_ui.license = m.license.clone();
            self.details_ui.source_url = m.source_url.clone();
            self.details_ui.for_key = Some(key);
        }
    }

    fn details_single(&mut self, ui: &mut Ui) {
        let Some(d) = self.detail.clone() else {
            if let Some(e) = &self.detail_error {
                ui.label(RichText::new(e).color(err_color()));
            } else {
                ui.vertical_centered(|ui| {
                    ui.spinner();
                });
            }
            return;
        };
        let ctx = ui.ctx().clone();
        self.sync_edit_buffers(&ctx, &d);
        let row = d.detail.row.clone();
        let m = &d.detail.manifest;
        let id = row.id;

        // ---- miniatura / preview 3D
        let side = (ui.available_width() - 4.0).min(300.0);
        let is_3d = cel_thumb::is_3d_format(&row.format);
        if self.preview3d.enabled && is_3d {
            self.preview_widget(ui, &d, side);
        } else {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(6), theme::BG_DEEP);
            let tex_id = self.tex.get(&row.content_hash, 256).map(|t| t.id()).or_else(|| self.tex.best_available(&row.content_hash).map(|t| t.id()));
            match tex_id {
                Some(t) => {
                    ui.painter().image(t, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, kind_icon(row.kind), egui::FontId::proportional(side * 0.3), kind_color(row.kind).gamma_multiply(0.5));
                }
            }
        }
        ui.horizontal(|ui| {
            if is_3d {
                let mut on = self.preview3d.enabled;
                if ui.toggle_value(&mut on, format!("{}  {}", icons::CUBE_FOCUS, tr("lib.det.preview3d"))).on_hover_text(tr("lib.det.preview3d.tip")).changed() {
                    self.preview3d.enabled = on;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut fav = row.favorite;
                if ui.selectable_label(fav, RichText::new(icons::HEART).size(16.0).color(if fav { Color32::from_rgb(0xff, 0x5c, 0x7a) } else { theme::TEXT_DIM })).on_hover_text(tr("lib.favorite")).clicked() {
                    fav = !fav;
                    self.set_favorite(&[id], fav);
                }
                let rect = ui.allocate_exact_size(Vec2::new(86.0, 16.0), Sense::hover()).0;
                if let Some(v) = rating_row(ui, ui.id().with("det_rate"), rect, row.rating, true, true) {
                    self.set_rating(id, v);
                }
            });
        });
        ui.add_space(4.0);

        // ---- nome (editável) + estado
        if row.state != AssetState::Active || !row.available {
            let (txt, col) = match (row.state, row.available) {
                (AssetState::Trashed, _) => (tr("lib.state.trashed"), warn_color()),
                (AssetState::Missing, _) => (tr("lib.state.missing"), err_color()),
                (AssetState::Corrupt, _) => (tr("lib.state.corrupt"), err_color()),
                (_, false) => (tr("lib.state.offline"), warn_color()),
                _ => ("", theme::TEXT),
            };
            ui.label(RichText::new(txt).color(col).size(12.0));
        }
        let trashed = row.state == AssetState::Trashed;
        ui.add_enabled_ui(!trashed, |ui| {
            let mut name = std::mem::take(&mut self.details_ui.name);
            let resp = ui.add(egui::TextEdit::singleline(&mut name).font(egui::TextStyle::Heading).desired_width(f32::INFINITY));
            let committed = resp.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape));
            self.details_ui.name = name.clone();
            if committed && name.trim() != m.name && !name.trim().is_empty() {
                let patch = InfoPatch { name: Some(name.trim().to_string()), ..Default::default() };
                self.update_info(id, patch);
            }
        });
        ui.label(RichText::new(format!("{}  {}  \u{00b7}  {}", kind_icon(row.kind), kind_label(row.kind), row.format.to_uppercase())).color(kind_color(row.kind)).size(12.0));
        ui.add_space(6.0);

        // ---- ações
        self.detail_actions(ui, &d);
        ui.add_space(6.0);

        // ---- metadados
        egui::CollapsingHeader::new(RichText::new(tr("lib.det.meta")).strong()).default_open(true).show(ui, |ui| {
            egui::Grid::new("lib_meta_grid").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                kv(ui, tr("lib.det.size"), fmt_bytes(row.size));
                if m.stats.tris > 0 || m.stats.verts > 0 {
                    kv(ui, tr("lib.det.tris"), fmt_int(m.stats.tris));
                    kv(ui, tr("lib.det.verts"), fmt_int(m.stats.verts));
                }
                if m.stats.width > 0 {
                    kv(ui, tr("lib.det.dims"), format!("{} \u{00d7} {} px", m.stats.width, m.stats.height));
                }
                if m.stats.meshes > 0 {
                    kv(ui, tr("lib.det.meshes"), m.stats.meshes.to_string());
                }
                if m.stats.materials > 0 || m.stats.textures > 0 {
                    kv(ui, tr("lib.det.materials"), m.stats.materials.to_string());
                    kv(ui, tr("lib.det.textures"), m.stats.textures.to_string());
                }
                if m.stats.bones > 0 {
                    kv(ui, tr("lib.det.bones"), m.stats.bones.to_string());
                }
                if m.stats.morphs > 0 {
                    kv(ui, tr("lib.det.morphs"), m.stats.morphs.to_string());
                }
                if m.stats.anims > 0 {
                    kv(ui, tr("lib.det.anims"), m.stats.anims.to_string());
                }
                if let Some(b) = m.stats.bbox {
                    kv(ui, tr("lib.det.bbox"), format!("{:.2} \u{00d7} {:.2} \u{00d7} {:.2}", b[3] - b[0], b[4] - b[1], b[5] - b[2]));
                }
                if m.stats.has_vrm {
                    kv(ui, "VRM", tr("lib.det.yes"));
                }
                kv(ui, tr("lib.det.storage"), if row.storage == StorageMode::Managed { tr("lib.root.managed") } else { tr("lib.root.linked") });
                kv(ui, tr("lib.det.revision"), format!("r{} / {}", m.current_revision, m.revisions.len()));
                kv(ui, tr("lib.det.imported"), fmt_time(row.imported_at));
                kv(ui, tr("lib.det.modified"), fmt_time(row.modified_at));
                if let Some(t) = m.trashed_at {
                    kv(ui, tr("lib.det.trashed_at"), fmt_time(t));
                }
            });
            ui.add_space(4.0);
            let origin = match (&row.storage, &d.root_path, &m.link) {
                (StorageMode::Linked, Some(rp), Some(l)) => rp.join(&l.rel_path).display().to_string(),
                _ => d.files.as_ref().map(|f| f.primary.display().to_string()).unwrap_or_default(),
            };
            ui.horizontal(|ui| {
                ui.label(small_dim(tr("lib.det.origin")));
                if icon_button(ui, icons::COPY, tr("lib.copy")).clicked() {
                    ui.ctx().copy_text(origin.clone());
                }
            });
            ui.add(egui::Label::new(RichText::new(&origin).monospace().size(10.5)).wrap());
            let hex = row.content_hash.to_hex();
            ui.horizontal(|ui| {
                ui.label(small_dim(tr("lib.det.hash")));
                ui.label(RichText::new(format!("{}\u{2026}", &hex[..16])).monospace().size(11.0)).on_hover_text(&hex);
                if icon_button(ui, icons::COPY, tr("lib.copy")).clicked() {
                    ui.ctx().copy_text(hex.clone());
                }
            });
            if m.revisions.len() > 1 {
                ui.add_space(4.0);
                egui::CollapsingHeader::new(small_dim(tr("lib.det.revisions"))).show(ui, |ui| {
                    for r in m.revisions.iter().rev() {
                        let cur = r.rev == m.current_revision;
                        ui.label(RichText::new(format!("r{}{}  {}  {}  {}\u{2026}", r.rev, if cur { " \u{2022}" } else { "" }, fmt_time(r.created_at), fmt_bytes(r.size), &r.content_hash.to_hex()[..8])).monospace().size(10.5));
                    }
                });
            }
        });

        // ---- informações editáveis
        egui::CollapsingHeader::new(RichText::new(tr("lib.det.info")).strong()).default_open(true).show(ui, |ui| {
            ui.add_enabled_ui(!trashed, |ui| {
                ui.label(small_dim(tr("lib.det.description")));
                let mut s = std::mem::take(&mut self.details_ui.description);
                let c = commit_field(ui, &mut s, true, tr("lib.det.description.hint"));
                self.details_ui.description = s.clone();
                if c && s != m.description {
                    self.update_info(id, InfoPatch { description: Some(s), ..Default::default() });
                }
                for (label, which) in [("lib.det.author", 0), ("lib.det.license", 1), ("lib.det.source", 2)] {
                    ui.label(small_dim(tr(label)));
                    let (mut s, orig) = match which {
                        0 => (std::mem::take(&mut self.details_ui.author), m.author.clone()),
                        1 => (std::mem::take(&mut self.details_ui.license), m.license.clone()),
                        _ => (std::mem::take(&mut self.details_ui.source_url), m.source_url.clone()),
                    };
                    let c = commit_field(ui, &mut s, false, "");
                    match which {
                        0 => self.details_ui.author = s.clone(),
                        1 => self.details_ui.license = s.clone(),
                        _ => self.details_ui.source_url = s.clone(),
                    }
                    if c && s != orig {
                        let patch = match which {
                            0 => InfoPatch { author: Some(s), ..Default::default() },
                            1 => InfoPatch { license: Some(s), ..Default::default() },
                            _ => InfoPatch { source_url: Some(s), ..Default::default() },
                        };
                        self.update_info(id, patch);
                    }
                }
            });
        });

        // ---- tags
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("lib.det.tags"), m.tags.len())).strong()).default_open(true).show(ui, |ui| {
            let mut remove: Option<String> = None;
            ui.horizontal_wrapped(|ui| {
                for t in &m.tags {
                    if ui.small_button(format!("{t}  {}", icons::X)).on_hover_text(tr("lib.det.tag.remove")).clicked() {
                        remove = Some(t.clone());
                    }
                }
            });
            if let Some(t) = remove {
                self.command("tags", Refresh { rows: false, facets: true, detail: true }, move |lib| lib.remove_tags(&id, &[t]).map(|_| String::new()).map_err(|e| e.to_string()));
            }
            ui.add_enabled_ui(!trashed, |ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.details_ui.new_tag).hint_text(tr("lib.det.tag.add_hint")).desired_width(f32::INFINITY));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let t = self.details_ui.new_tag.trim().to_string();
                    self.details_ui.new_tag.clear();
                    if !t.is_empty() {
                        let parts: Vec<String> = t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        self.command("tags", Refresh { rows: false, facets: true, detail: true }, move |lib| lib.add_tags(&id, &parts).map(|_| String::new()).map_err(|e| e.to_string()));
                    }
                    resp.request_focus();
                }
            });
        });

        // ---- coleções
        if !d.collections.is_empty() {
            let names: Vec<String> = d.collections.iter().filter_map(|c| self.facets.collections.iter().find(|(x, _)| x.id == *c).map(|(x, _)| x.name.clone())).collect();
            if !names.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(small_dim(tr("lib.det.collections")));
                    for n in names {
                        ui.label(RichText::new(format!("{} {n}", icons::FOLDER_SIMPLE)).size(11.5).color(Color32::from_rgb(0xff, 0xc8, 0x6c)));
                    }
                });
            }
        }

        // ---- dependências
        self.detail_dependencies(ui, &d);

        // ---- materiais toon canônicos
        if !d.loaded_materials.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("lib.det.toon"), d.materials.len())).strong()).default_open(true).show(ui, |ui| {
                for (file, mat) in &d.loaded_materials {
                    material_view(ui, file, mat);
                }
                if d.materials.len() > d.loaded_materials.len() {
                    ui.label(small_dim(format!("+{} {}", d.materials.len() - d.loaded_materials.len(), tr("lib.det.more"))));
                }
            });
        }

        // ---- avisos do import
        if !m.warnings.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("lib.det.warnings"), m.warnings.len())).strong().color(warn_color())).show(ui, |ui| {
                for w in &m.warnings {
                    ui.add(egui::Label::new(RichText::new(format!("\u{2022} {w}")).size(11.5)).wrap());
                }
            });
        }
    }

    fn detail_actions(&mut self, ui: &mut Ui, d: &DetailData) {
        let row = &d.detail.row;
        let id = row.id;
        ui.horizontal_wrapped(|ui| {
            match row.state {
                AssetState::Trashed => {
                    if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("lib.act.restore"))).clicked() {
                        self.restore_ids(vec![id]);
                    }
                    if ui.button(RichText::new(format!("{}  {}", icons::TRASH, tr("lib.act.purge"))).color(err_color())).clicked() {
                        self.dlg.confirm = Some(dialogs::Confirm::Purge(vec![id]));
                    }
                }
                _ => {
                    let editable = editable_in_modeling(row.kind, &row.format) && row.available && row.state != AssetState::Missing;
                    if ui.add_enabled(editable, egui::Button::new(format!("{}  {}", icons::CUBE_FOCUS, tr("lib.act.edit")))).on_hover_text(tr("lib.act.edit.tip")).clicked() {
                        self.edit_in_modeling(id);
                    }
                    if ui.button(format!("{}  {}", icons::COPY, tr("lib.act.duplicate"))).clicked() {
                        self.duplicate_asset(id);
                    }
                    if ui.button(format!("{}  {}", icons::TRASH, tr("lib.act.trash"))).clicked() {
                        self.trash_ids(vec![id]);
                    }
                    if row.state == AssetState::Missing && row.storage == StorageMode::Linked && ui.button(RichText::new(format!("{}  {}", icons::LINK, tr("lib.relink"))).color(warn_color())).clicked() {
                        self.dlg.open_relink(id, row.name.clone());
                    }
                }
            }
        });
    }

    fn detail_dependencies(&mut self, ui: &mut Ui, d: &DetailData) {
        let id = d.detail.row.id;
        if d.deps.is_empty() && d.used_by.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(RichText::new(format!("{} ({} / {})", tr("lib.det.deps"), d.deps.len(), d.used_by.len())).strong()).default_open(true).show(ui, |ui| {
            if !d.deps.is_empty() {
                ui.label(small_dim(tr("lib.det.uses")));
            }
            let mut goto: Option<AssetId> = None;
            let mut remove: Option<DependencyRef> = None;
            for (dep, row) in &d.deps {
                ui.horizontal(|ui| {
                    let (label, kind) = match row {
                        Some(r) => (r.name.clone(), Some(r.kind)),
                        None => (tr("lib.det.dep_missing").to_string(), None),
                    };
                    if let Some(k) = kind {
                        ui.label(RichText::new(kind_icon(k)).color(kind_color(k)));
                    }
                    if ui.link(RichText::new(label).size(12.0)).clicked() {
                        goto = Some(dep.id);
                    }
                    ui.label(small_dim(&dep.kind));
                    if icon_button(ui, icons::X, tr("lib.det.dep_remove")).clicked() {
                        remove = Some(dep.clone());
                    }
                });
            }
            if !d.used_by.is_empty() {
                ui.label(small_dim(tr("lib.det.used_by")));
                for r in &d.used_by {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(kind_icon(r.kind)).color(kind_color(r.kind)));
                        if ui.link(RichText::new(&r.name).size(12.0)).clicked() {
                            goto = Some(r.id);
                        }
                    });
                }
            }
            if let Some(g) = goto {
                self.selection.set_one(g, 0);
                self.selection.cursor = None;
                self.sync_detail_target();
            }
            if let Some(dep) = remove {
                self.command("deps", Refresh::ALL, move |lib| lib.remove_dependency(&id, &dep.id, &dep.kind).map(|_| String::new()).map_err(|e| e.to_string()));
            }
        });
    }

    fn details_multi(&mut self, ui: &mut Ui) {
        let ids = self.selected_ids();
        ui.label(RichText::new(format!("{} {}", ids.len(), tr("lib.selected"))).strong().size(15.0));
        ui.add_space(4.0);
        // somas das linhas já carregadas (o restante entra quando as páginas chegam)
        let mut size = 0u64;
        let mut tris = 0u64;
        let mut known = 0usize;
        for id in &ids {
            if let Some(r) = self.row_of(id) {
                size += r.size;
                tris += r.tris;
                known += 1;
            }
        }
        if known > 0 {
            ui.label(small_dim(format!("{}{} \u{00b7} {} tris", fmt_bytes(size), if known < ids.len() { "+" } else { "" }, fmt_int(tris))));
        }
        ui.add_space(6.0);
        let trash_view = matches!(self.filters.section, Section::Trash);
        ui.horizontal_wrapped(|ui| {
            if trash_view {
                if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("lib.act.restore"))).clicked() {
                    self.restore_ids(ids.clone());
                }
                if ui.button(RichText::new(format!("{}  {}", icons::TRASH, tr("lib.act.purge"))).color(err_color())).clicked() {
                    self.dlg.confirm = Some(dialogs::Confirm::Purge(ids.clone()));
                }
            } else {
                if ui.button(format!("{}  {}", icons::HEART, tr("lib.act.favorite"))).clicked() {
                    self.set_favorite(&ids, true);
                }
                if ui.button(format!("{}  {}", icons::HEART_BREAK, tr("lib.act.unfavorite"))).clicked() {
                    self.set_favorite(&ids, false);
                }
                if ui.button(format!("{}  {}", icons::TRASH, tr("lib.act.trash"))).clicked() {
                    self.trash_ids(ids.clone());
                }
            }
        });
        ui.add_space(8.0);
        ui.label(small_dim(tr("lib.det.batch_tag")));
        let resp = ui.add(egui::TextEdit::singleline(&mut self.details_ui.batch_tag).hint_text(tr("lib.det.tag.add_hint")).desired_width(f32::INFINITY));
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let t = std::mem::take(&mut self.details_ui.batch_tag);
            let parts: Vec<String> = t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if !parts.is_empty() {
                let n = ids.len();
                let ids2 = ids.clone();
                self.command("tags", Refresh { rows: false, facets: true, detail: false }, move |lib| lib.add_tags_many(&ids2, &parts).map(|_| format!("{} \u{2192} {n}", parts.join(", "))).map_err(|e| e.to_string()));
            }
        }
        ui.add_space(6.0);
        let colls: Vec<(CollectionId, String)> = self.facets.collections.iter().filter(|(c, _)| !c.is_smart()).map(|(c, _)| (c.id, c.name.clone())).collect();
        ui.menu_button(format!("{}  {}", icons::FOLDER_PLUS, tr("lib.det.add_to_collection")), |ui| {
            if colls.is_empty() {
                ui.label(small_dim(tr("lib.coll.none")));
            }
            for (cid, name) in &colls {
                if ui.button(name).clicked() {
                    let (ids2, name2, n) = (ids.clone(), name.clone(), ids.len());
                    let cid = *cid;
                    self.command("collection", Refresh::ALL, move |lib| lib.add_to_collection(cid, &ids2).map(|_| format!("{n} \u{2192} {name2}")).map_err(|e| e.to_string()));
                    ui.close();
                }
            }
        });
        ui.add_space(8.0);
        if ui.button(tr("lib.sel.clear")).clicked() {
            self.selection.clear();
            self.sync_detail_target();
        }
    }

    // ====================================================== comandos de mutação

    pub(crate) fn update_info(&mut self, id: AssetId, patch: InfoPatch) {
        self.command("info", Refresh::ALL, move |lib| lib.update_info(&id, &patch).map(|_| String::new()).map_err(|e| e.to_string()));
    }

    /// Favorito com atualização otimista da grade (o disco confirma em seguida).
    pub(crate) fn set_favorite(&mut self, ids: &[AssetId], value: bool) {
        for id in ids {
            self.list.update_row(id, |r| r.favorite = value);
        }
        let in_favorites = matches!(self.filters.section, Section::Favorites);
        let ids = ids.to_vec();
        let refresh = Refresh { rows: in_favorites, facets: true, detail: self.detail_for.is_some_and(|d| ids.contains(&d)) };
        self.command("favorite", refresh, move |lib| {
            for id in &ids {
                lib.set_favorite(id, value).map_err(|e| e.to_string())?;
            }
            Ok(String::new())
        });
    }

    pub(crate) fn set_rating(&mut self, id: AssetId, value: u8) {
        self.list.update_row(&id, |r| r.rating = value);
        let refresh = Refresh { rows: false, facets: false, detail: self.detail_for == Some(id) };
        self.command("rating", refresh, move |lib| lib.set_rating(&id, value).map(|_| String::new()).map_err(|e| e.to_string()));
    }

    pub(crate) fn trash_ids(&mut self, ids: Vec<AssetId>) {
        if ids.is_empty() {
            return;
        }
        self.spawn_task(move |lib| match lib.trash(&ids, TrashOptions::default()) {
            Ok(rep) => TaskOut::TrashResult { trashed: rep.trashed.len(), blocked: rep.blocked.into_iter().map(|(id, users)| (id, users.len())).collect(), error: None },
            Err(e) => TaskOut::TrashResult { trashed: 0, blocked: vec![], error: Some(e.to_string()) },
        });
    }

    pub(crate) fn force_trash(&mut self, ids: Vec<AssetId>) {
        let n = ids.len();
        self.selection.clear();
        self.sync_detail_target();
        self.command("trash", Refresh::ALL, move |lib| lib.trash(&ids, TrashOptions { force: true }).map(|r| format!("{} {}", r.trashed.len().max(n.min(r.trashed.len())), tr("lib.msg.trashed"))).map_err(|e| e.to_string()));
    }

    pub(crate) fn restore_ids(&mut self, ids: Vec<AssetId>) {
        self.selection.clear();
        self.sync_detail_target();
        self.command("restore", Refresh::ALL, move |lib| lib.restore(&ids).map(|v| format!("{} {}", v.len(), tr("lib.msg.restored"))).map_err(|e| e.to_string()));
    }

    pub(crate) fn purge_ids(&mut self, ids: Vec<AssetId>) {
        self.selection.clear();
        self.sync_detail_target();
        self.command("purge", Refresh::ALL, move |lib| lib.purge(&ids).map(|r| format!("{} {} ({})", r.purged.len(), tr("lib.msg.purged"), fmt_bytes(r.bytes_freed))).map_err(|e| e.to_string()));
    }

    pub(crate) fn empty_trash(&mut self) {
        self.selection.clear();
        self.sync_detail_target();
        self.command("trash", Refresh::ALL, move |lib| lib.empty_trash(None).map(|r| format!("{} {} ({})", r.purged.len(), tr("lib.msg.purged"), fmt_bytes(r.bytes_freed))).map_err(|e| e.to_string()));
    }

    /// Duplica um ativo como NOVO ativo (importa de novo o arquivo primário com o mesmo conteúdo).
    pub(crate) fn duplicate_asset(&mut self, id: AssetId) {
        let Some(rt) = &self.rt else { return };
        let res = rt.lib.get(&id).and_then(|d| rt.lib.resolve_files(&id).map(|f| (d, f)));
        match res {
            Ok((d, f)) if f.exists => {
                let m = d.manifest;
                let opts = ImportOptions {
                    name: Some(format!("{} ({})", m.name, tr("lib.copy_suffix"))),
                    tags: m.tags.clone(),
                    kind: Some(m.kind),
                    description: Some(m.description.clone()),
                    author: Some(m.author.clone()),
                    license: Some(m.license.clone()),
                    source_url: Some(m.source_url.clone()),
                    duplicates: DuplicatePolicy::Allow,
                    ..Default::default()
                };
                self.queue_import(vec![f.primary], opts);
                self.info(tr("lib.msg.duplicating"));
            }
            Ok(_) => self.error(tr("lib.msg.file_missing")),
            Err(e) => self.error(e.to_string()),
        }
    }

    pub(crate) fn edit_in_modeling(&mut self, id: AssetId) {
        let Some(rt) = &self.rt else { return };
        let row = match rt.lib.require_row(&id) {
            Ok(r) => r,
            Err(e) => return self.error(e.to_string()),
        };
        if !editable_in_modeling(row.kind, &row.format) {
            return;
        }
        match rt.lib.resolve_files(&id) {
            Ok(f) if f.exists => self.actions.push(HostAction::EditInModeling { path: f.primary, format: row.format.clone(), name: row.name.clone() }),
            Ok(_) => self.error(tr("lib.msg.file_missing")),
            Err(e) => self.error(e.to_string()),
        }
    }

    pub(crate) fn card_action(&mut self, a: CardAction) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        match a {
            CardAction::Edit => {
                if let Some(id) = self.selection.primary {
                    self.edit_in_modeling(id);
                }
            }
            CardAction::Favorite(v) => self.set_favorite(&ids, v),
            CardAction::Trash => self.trash_ids(ids),
            CardAction::Restore => self.restore_ids(ids),
            CardAction::Purge => self.dlg.confirm = Some(dialogs::Confirm::Purge(ids)),
            CardAction::Duplicate => {
                for id in ids {
                    self.duplicate_asset(id);
                }
            }
        }
    }
}

/// Mostra o `ToonMaterial` canônico do ativo (campos reais do `.anigomat.json`).
fn material_view(ui: &mut Ui, file: &str, m: &ToonMaterial) {
    let title = RichText::new(format!("{}", if m.name.is_empty() { file } else { &m.name })).size(12.5);
    egui::CollapsingHeader::new(title).id_salt(("mat", file)).default_open(true).show(ui, |ui| {
        if let Some(c) = &m.converted_from {
            ui.label(small_dim(format!("{} {c}", tr("lib.mat.converted_from"))));
        }
        egui::Grid::new(("mat_grid", file)).num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.label(small_dim(tr("lib.mat.base")));
            ui.horizontal(|ui| {
                swatch(ui, [m.base.color[0], m.base.color[1], m.base.color[2]], tr("lib.mat.base"));
                if let Some(t) = &m.base.texture {
                    ui.label(small_dim(tex_label(t)));
                }
                if m.base.alpha_mode != AlphaMode::Opaque {
                    ui.label(small_dim(format!("{:?}", m.base.alpha_mode)));
                }
            });
            ui.end_row();
            ui.label(small_dim(tr("lib.mat.shade")));
            ui.horizontal(|ui| {
                swatch(ui, m.shade.color, tr("lib.mat.shade"));
                if let Some(c2) = m.shade.color2 {
                    swatch(ui, c2, tr("lib.mat.shade2"));
                }
                ui.label(small_dim(format!("{} {:.2} \u{00b7} {} {:.2} \u{00b7} {} {}", tr("lib.mat.threshold"), m.shade.threshold, tr("lib.mat.smooth"), m.shade.smoothness, tr("lib.mat.steps"), m.shade.steps)));
            });
            ui.end_row();
            if m.shade.hue_shift.abs() > 1e-3 {
                kv(ui, tr("lib.mat.hue"), format!("{:.1}\u{00b0}", m.shade.hue_shift));
            }
            ui.label(small_dim(tr("lib.mat.spec")));
            ui.horizontal(|ui| {
                swatch(ui, m.spec.color, tr("lib.mat.spec"));
                ui.label(small_dim(format!("{:.2} \u{00b7} {} {:.0} \u{00b7} {} {:.2}", m.spec.intensity, tr("lib.mat.power"), m.spec.power, tr("lib.mat.size"), m.spec.size)));
            });
            ui.end_row();
            ui.label(small_dim(tr("lib.mat.rim")));
            ui.horizontal(|ui| {
                swatch(ui, m.rim.color, tr("lib.mat.rim"));
                ui.label(small_dim(format!("{:.2} \u{00b7} {} {:.2}", m.rim.intensity, tr("lib.mat.spread"), m.rim.spread)));
            });
            ui.end_row();
            ui.label(small_dim(tr("lib.mat.outline")));
            ui.horizontal(|ui| {
                swatch(ui, m.outline.color, tr("lib.mat.outline"));
                ui.label(small_dim(format!("{:?} \u{00b7} {:.4}", m.outline.mode, m.outline.width)));
            });
            ui.end_row();
            if m.emission.intensity > 0.0 {
                ui.label(small_dim(tr("lib.mat.emission")));
                ui.horizontal(|ui| {
                    swatch(ui, m.emission.color, tr("lib.mat.emission"));
                    ui.label(small_dim(format!("{:.2}", m.emission.intensity)));
                });
                ui.end_row();
            }
            if let Some(t) = &m.normal.texture {
                kv(ui, tr("lib.mat.normal"), format!("{} ({:?})", tex_label(t), m.normal.convention));
            }
            if let Some(r) = &m.shade.ramp {
                kv(ui, tr("lib.mat.ramp"), tex_label(r));
            }
            if m.ilm.is_some() {
                kv(ui, "ILM", tr("lib.det.yes"));
            }
            if m.face_sdf.is_some() {
                kv(ui, "Face SDF", tr("lib.det.yes"));
            }
        });
        if ui.small_button(format!("{}  {}", icons::COPY, tr("lib.mat.copy_json"))).clicked() {
            if let Ok(b) = m.to_json() {
                ui.ctx().copy_text(String::from_utf8_lossy(&b).to_string());
            }
        }
    });
}

fn tex_label(t: &anigo_assets::material::TextureRef) -> String {
    let src = match (&t.uri, t.gltf_image) {
        (Some(u), _) => u.clone(),
        (None, Some(i)) => format!("glTF image #{i}"),
        _ => "?".into(),
    };
    format!("{src} [{:?}, {:?}]", t.role, t.color_space)
}
