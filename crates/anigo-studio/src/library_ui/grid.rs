//! Barra de ferramentas e área central: grade/lista VIRTUALIZADAS (`ScrollArea::show_rows`).
//!
//! Só as linhas visíveis são desenhadas e só as páginas que as cobrem (± um pouco) são pedidas ao banco;
//! as texturas das miniaturas são pedidas por visibilidade ao `ThumbService`.

use super::thumb_service::{bucket, ThumbReq};
use super::widgets::*;
use super::*;
use crate::i18n::tr;
use crate::icons;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::hash::{Hash, Hasher};

const GAP: f32 = 10.0;
const PAD: f32 = 6.0;
const META_H: f32 = 34.0;
const LIST_ROW_H: f32 = 30.0;

/// Ações do menu de contexto / atalhos que valem para a seleção.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardAction {
    Edit,
    Favorite(bool),
    Trash,
    Restore,
    Purge,
    Duplicate,
}

enum GridEvent {
    Click { idx: usize, id: AssetId, mods: egui::Modifiers },
    Double { id: AssetId },
    DragStart { idx: usize, id: AssetId },
    Favorite { id: AssetId, value: bool },
    Rating { id: AssetId, value: u8 },
    ContextSelect { idx: usize, id: AssetId },
    Action(CardAction),
}

struct CardCtx {
    selected: bool,
    tex: Option<egui::TextureId>,
    /// Textura de outro tamanho (mostrada enquanto o exato carrega).
    thumb_side: f32,
}

impl LibraryWorkspace {
    // =============================================================== toolbar

    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(theme::TEXT_DIM));
            let edit = egui::TextEdit::singleline(&mut self.search_text).hint_text(tr("lib.search_hint")).desired_width(260.0);
            let resp = ui.add(edit);
            if self.focus_search {
                resp.request_focus();
                self.focus_search = false;
            }
            if resp.changed() {
                self.search_edited_at = Some(Instant::now());
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.search_edited_at = Some(Instant::now() - Duration::from_secs(1));
            }
            if !self.search_text.is_empty() && icon_button(ui, icons::X, tr("lib.search_clear")).clicked() {
                self.search_text.clear();
                self.search_edited_at = Some(Instant::now() - Duration::from_secs(1));
            }
            ui.label(RichText::new(icons::INFO).color(theme::TEXT_DIM)).on_hover_text(tr("lib.search_help"));
            ui.separator();

            // tipos
            let kinds_label = if self.filters.kinds.is_empty() { tr("lib.filter.all_kinds").to_string() } else { format!("{} ({})", tr("lib.filter.kinds"), self.filters.kinds.len()) };
            ui.menu_button(format!("{}  {kinds_label}", icons::FUNNEL), |ui| {
                let counts = self.facets.kind_counts.clone();
                for k in AssetKind::ALL {
                    let mut on = self.filters.kinds.contains(k);
                    let n = counts.iter().find(|(kk, _)| kk == k).map(|(_, n)| *n).unwrap_or(0);
                    let label = format!("{}  {}  ({})", kind_icon(*k), kind_label(*k), fmt_int(n));
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            self.filters.kinds.push(*k);
                        } else {
                            self.filters.kinds.retain(|x| x != k);
                        }
                        self.filters_changed();
                    }
                }
                ui.separator();
                if ui.button(tr("lib.filter.clear")).clicked() {
                    self.filters.kinds.clear();
                    self.filters_changed();
                    ui.close();
                }
            });

            // nota mínima
            let rating_label = if self.filters.min_rating == 0 { tr("lib.filter.any_rating").to_string() } else { format!("\u{2265} {}", self.filters.min_rating) };
            egui::ComboBox::from_id_salt("lib_min_rating").selected_text(format!("{}  {rating_label}", icons::STAR)).show_ui(ui, |ui| {
                for r in 0..=5u8 {
                    let text = if r == 0 { tr("lib.filter.any_rating").to_string() } else { format!("\u{2265} {r}") };
                    if ui.selectable_label(self.filters.min_rating == r, text).clicked() {
                        self.filters.min_rating = r;
                        self.filters_changed();
                    }
                }
            });
            ui.separator();

            // ordenação
            let sort_options: [(SortBy, &str); 7] = [
                (SortBy::Name, "lib.sort.name"),
                (SortBy::ImportedAt, "lib.sort.imported"),
                (SortBy::ModifiedAt, "lib.sort.modified"),
                (SortBy::Size, "lib.sort.size"),
                (SortBy::Rating, "lib.sort.rating"),
                (SortBy::Tris, "lib.sort.tris"),
                (SortBy::Relevance, "lib.sort.relevance"),
            ];
            let cur = self.filters.to_query().sort;
            let cur_label = sort_options.iter().find(|(s, _)| *s == cur.by).map(|(_, k)| tr(k)).unwrap_or("");
            egui::ComboBox::from_id_salt("lib_sort").selected_text(format!("{}  {cur_label}", icons::ARROWS_DOWN_UP)).show_ui(ui, |ui| {
                for (s, key) in sort_options {
                    if s == SortBy::Relevance && self.filters.text.trim().is_empty() {
                        continue;
                    }
                    if ui.selectable_label(cur.by == s, tr(key)).clicked() {
                        self.filters.sort.by = s;
                        self.filters.user_sorted = true;
                        // dica: nota/tamanho/tris/datas fazem mais sentido do maior para o menor
                        self.filters.sort.desc = matches!(s, SortBy::Rating | SortBy::Size | SortBy::Tris | SortBy::ImportedAt | SortBy::ModifiedAt);
                        self.filters_changed();
                    }
                }
            });
            let desc = cur.desc;
            if icon_button(ui, if desc { icons::SORT_DESCENDING } else { icons::SORT_ASCENDING }, tr("lib.sort.toggle")).clicked() {
                self.filters.sort = Sort { by: cur.by, desc: !desc };
                self.filters.user_sorted = true;
                self.filters_changed();
            }
            ui.separator();

            // visão
            if ui.selectable_label(self.view == ViewMode::Grid, RichText::new(icons::GRID_FOUR).size(16.0)).on_hover_text(tr("lib.view.grid")).clicked() {
                self.view = ViewMode::Grid;
                self.pending_scroll = Some(0.0);
            }
            if ui.selectable_label(self.view == ViewMode::List, RichText::new(icons::LIST_BULLETS).size(16.0)).on_hover_text(tr("lib.view.list")).clicked() {
                self.view = ViewMode::List;
                self.pending_scroll = Some(0.0);
            }
            if self.view == ViewMode::Grid {
                ui.add(egui::Slider::new(&mut self.card_size, 90.0..=240.0).show_value(false)).on_hover_text(tr("lib.view.card_size"));
            }
            ui.separator();

            if ui.button(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("lib.import"))).on_hover_text(tr("lib.import.tip")).clicked() {
                self.dlg.open_import();
            }
            if ui.button(format!("{}  {}", icons::GEAR, tr("lib.manage"))).on_hover_text(tr("lib.manage.tip")).clicked() {
                self.dlg.open_manage();
            }
        });

        // linha de contexto: seção atual, chips, contagem, ações da seção
        ui.horizontal_wrapped(|ui| {
            let (icon, title) = self.section_title();
            ui.label(RichText::new(format!("{icon}  {title}")).strong());
            ui.label(small_dim(format!("{} {}", fmt_int(self.list.total_or_last() as u64), tr("lib.items"))));
            if self.list.total.is_none() && self.rt.is_some() {
                ui.spinner();
            }
            if !self.selection.is_empty() {
                ui.label(RichText::new(format!("\u{00b7}  {} {}", self.selection.len(), tr("lib.selected"))).color(theme::SELECT).size(12.0));
            }
            let mut changed = false;
            let kinds = self.filters.kinds.clone();
            for k in kinds {
                if ui.small_button(format!("{} {}  {}", kind_icon(k), kind_label(k), icons::X)).clicked() {
                    self.filters.kinds.retain(|x| *x != k);
                    changed = true;
                }
            }
            if !self.filters.text.trim().is_empty() && ui.small_button(format!("\u{201c}{}\u{201d}  {}", self.filters.text.trim(), icons::X)).clicked() {
                self.filters.text.clear();
                self.search_text.clear();
                changed = true;
            }
            if self.filters.min_rating > 0 && ui.small_button(format!("\u{2265} {} {}  {}", self.filters.min_rating, icons::STAR, icons::X)).clicked() {
                self.filters.min_rating = 0;
                changed = true;
            }
            if changed {
                self.filters_changed();
            }
            match self.filters.section.clone() {
                Section::Trash => {
                    if ui.small_button(format!("{}  {}", icons::RECYCLE, tr("lib.trash.empty"))).clicked() {
                        self.dlg.confirm = Some(dialogs::Confirm::EmptyTrash);
                    }
                }
                Section::Missing => {
                    if ui.small_button(format!("{}  {}", icons::LINK, tr("lib.relink.batch"))).clicked() {
                        self.dlg.open_relink_batch();
                    }
                }
                Section::Collection(_) if self.filters.has_refinement() => {}
                _ => {}
            }
            // salvar a busca atual como coleção inteligente
            if self.filters.has_refinement() || matches!(self.filters.section, Section::Tag(_)) {
                if ui.small_button(format!("{}  {}", icons::BOOKMARK_SIMPLE, tr("lib.coll.save_smart"))).on_hover_text(tr("lib.coll.save_smart.tip")).clicked() {
                    self.dlg.smart_name = self.filters.text.trim().to_string();
                    self.dlg.open_smart = true;
                }
            }
        });
        ui.add_space(2.0);
        ui.separator();
    }

    fn section_title(&self) -> (&'static str, String) {
        match &self.filters.section {
            Section::All => (icons::SQUARES_FOUR, tr("lib.all").to_string()),
            Section::Favorites => (icons::HEART, tr("lib.favorites").to_string()),
            Section::Trash => (icons::TRASH, tr("lib.trash").to_string()),
            Section::Missing => (icons::WARNING, tr("lib.missing").to_string()),
            Section::Collection(id) => (icons::FOLDER_SIMPLE, self.facets.collections.iter().find(|(c, _)| c.id == *id).map(|(c, _)| c.name.clone()).unwrap_or_default()),
            Section::Tag(t) => (icons::TAG, t.clone()),
            Section::Root(r) => (icons::HARD_DRIVES, self.facets.roots.iter().find(|x| x.id == *r).map(|x| x.name.clone()).unwrap_or_default()),
        }
    }

    // ================================================================ centro

    pub(super) fn center(&mut self, ui: &mut Ui) {
        let n = self.list.total_or_last();
        if self.list.total == Some(0) {
            self.empty_state(ui);
            return;
        }
        match self.view {
            ViewMode::Grid => self.grid(ui, n),
            ViewMode::List => self.list_view(ui, n),
        }
    }

    fn empty_state(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.28).max(20.0));
            let (icon, msg, hint) = match &self.filters.section {
                Section::Trash => (icons::TRASH, tr("lib.empty.trash"), ""),
                Section::Missing => (icons::CHECK_CIRCLE, tr("lib.empty.missing"), ""),
                Section::Favorites => (icons::HEART, tr("lib.empty.favorites"), tr("lib.empty.favorites.hint")),
                _ if self.filters.has_refinement() => (icons::MAGNIFYING_GLASS, tr("lib.empty.search"), tr("lib.empty.search.hint")),
                _ => (icons::CUBE, tr("lib.empty.library"), tr("lib.empty.library.hint")),
            };
            ui.label(RichText::new(icon).size(44.0).color(theme::ACCENT_DIM));
            ui.label(RichText::new(msg).size(16.0).strong());
            if !hint.is_empty() {
                ui.label(dim(hint));
            }
            if matches!(self.filters.section, Section::All) && !self.filters.has_refinement() {
                ui.add_space(8.0);
                if ui.button(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("lib.import"))).clicked() {
                    self.dlg.open_import();
                }
            }
        });
    }

    // ================================================================= grade

    fn grid(&mut self, ui: &mut Ui, n: usize) {
        let size = self.card_size.clamp(90.0, 260.0);
        let thumb_side = size - 2.0 * PAD;
        let card_h = thumb_side + 2.0 * PAD + META_H;
        let avail_w = (ui.available_width() - 16.0).max(size);
        let cols = (((avail_w + GAP) / (size + GAP)).floor() as usize).max(1);
        self.cols = cols;
        let rows = n.div_ceil(cols);
        let ppp = ui.ctx().pixels_per_point();
        let want_bucket = bucket(thumb_side * ppp);
        ui.spacing_mut().item_spacing = Vec2::splat(GAP);

        let mut sa = egui::ScrollArea::vertical().id_salt("lib_grid").auto_shrink([false, false]);
        if let Some(off) = self.pending_scroll.take() {
            sa = sa.vertical_scroll_offset(off);
        }
        let mut events: Vec<GridEvent> = Vec::new();
        let mut card_hit = false;
        let mut first_item = 0usize;
        let mut last_item = 0usize;
        let editable_ctx = self.filters.section.clone();
        let out = {
            let Self { list, tex, selection, .. } = self;
            let trash_view = matches!(editable_ctx, Section::Trash);
            sa.show_rows(ui, card_h, rows, |ui, range| {
                let origin = ui.max_rect().left_top();
                first_item = range.start * cols;
                last_item = (range.end * cols).min(n);
                for idx in first_item..last_item {
                    let (r, c) = (idx / cols - range.start, idx % cols);
                    let pos = Pos2::new(origin.x + c as f32 * (size + GAP), origin.y + r as f32 * (card_h + GAP));
                    let rect = Rect::from_min_size(pos, Vec2::new(size, card_h));
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    match list.row(idx) {
                        Some(row) => {
                            let tex_id = tex.get(&row.content_hash, want_bucket).map(|t| t.id()).or_else(|| tex.best_available(&row.content_hash).map(|t| t.id()));
                            let cc = CardCtx { selected: selection.contains(&row.id), tex: tex_id, thumb_side };
                            let unavailable = tex.unavailable.contains(&row.content_hash) && tex_id.is_none();
                            paint_card(ui, rect, idx, row, &cc, unavailable, trash_view, &selection.ids, &mut events, &mut card_hit);
                        }
                        None => paint_skeleton(ui, rect),
                    }
                }
            })
        };
        self.visible = (first_item, last_item);
        self.after_scroll(ui, &out, rows, card_h + GAP, n, want_bucket, cols);
        self.apply_grid_events(events, card_hit, out.inner_rect, ui);
    }

    /// Pede páginas e miniaturas para o que está na tela (± uma tela) e guarda a rolagem.
    fn after_scroll(&mut self, ui: &Ui, out: &egui::scroll_area::ScrollAreaOutput<()>, _rows: usize, row_h: f32, n: usize, want_bucket: u32, cols: usize) {
        self.scroll_y = out.state.offset.y;
        self.viewport_h = out.inner_rect.height();
        self.row_h = row_h;
        let (first, last) = self.visible;
        let span = (last.saturating_sub(first)).max(cols * 2);
        let from = first.saturating_sub(span);
        let to = (last + span).min(n.saturating_sub(1));
        let gen = self.list.gen;
        let pages = self.list.missing_pages(from, to);
        if let Some(rt) = &self.rt {
            for p in pages {
                rt.q.request_page(gen, p);
            }
        }
        self.list.evict_far(first, last, 48);
        self.request_thumbs(first, last, from, to, want_bucket);
        if self.list.total.is_none() || !self.list.is_settled() {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
    }

    fn request_thumbs(&mut self, first: usize, last: usize, from: usize, to: usize, want_bucket: u32) {
        let Some(rt) = &self.rt else { return };
        let mut wanted: Vec<ThumbReq> = Vec::new();
        let mut seen: HashSet<ContentHash> = HashSet::new();
        // visíveis primeiro; depois as vizinhas (a ordem é a prioridade)
        let order = (first..last).chain((last..to.saturating_add(1)).chain((from..first).rev()));
        for idx in order {
            let Some(row) = self.list.row(idx) else { continue };
            if self.tex.contains(&row.content_hash, want_bucket) || self.tex.unavailable.contains(&row.content_hash) || !seen.insert(row.content_hash) {
                continue;
            }
            wanted.push(ThumbReq { id: row.id, hash: row.content_hash, size: want_bucket });
            if wanted.len() >= 400 {
                break;
            }
        }
        // o detalhe pede o tamanho maior do ativo selecionado
        if let (Some(d), true) = (&self.detail, self.selection.len() == 1) {
            let h = d.detail.row.content_hash;
            if !self.tex.contains(&h, 256) && !self.tex.unavailable.contains(&h) {
                wanted.insert(0, ThumbReq { id: d.detail.row.id, hash: h, size: 256 });
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for w in &wanted {
            w.hash.0.hash(&mut hasher);
            w.size.hash(&mut hasher);
        }
        rt.thumbs.set_wanted(wanted, hasher.finish() | 1);
    }

    fn apply_grid_events(&mut self, events: Vec<GridEvent>, card_hit: bool, area: Rect, ui: &Ui) {
        // clique no vazio da área limpa a seleção
        let (clicked, pos, mods) = ui.input(|i| (i.pointer.primary_clicked(), i.pointer.interact_pos(), i.modifiers));
        if clicked && !card_hit && !mods.command && !mods.shift && pos.is_some_and(|p| area.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id())) {
            self.selection.clear();
            self.sync_detail_target();
        }
        for ev in events {
            match ev {
                GridEvent::Click { idx, id, mods } => self.click_item(idx, id, mods),
                GridEvent::ContextSelect { idx, id } => {
                    if !self.selection.contains(&id) {
                        self.selection.set_one(id, idx);
                        self.sync_detail_target();
                    }
                }
                GridEvent::DragStart { idx, id } => {
                    if !self.selection.contains(&id) {
                        self.selection.set_one(id, idx);
                        self.sync_detail_target();
                    }
                }
                GridEvent::Double { id } => self.edit_in_modeling(id),
                GridEvent::Favorite { id, value } => self.set_favorite(&[id], value),
                GridEvent::Rating { id, value } => self.set_rating(id, value),
                GridEvent::Action(a) => self.card_action(a),
            }
        }
    }

    // ================================================================= lista

    fn list_view(&mut self, ui: &mut Ui, n: usize) {
        let w = ui.available_width() - 16.0;
        let wide = w > 760.0;
        // cabeçalho clicável
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
        let cols_x = list_columns(rect.left() + 6.0, w, wide);
        let cur = self.filters.to_query().sort;
        let heads: [(Option<SortBy>, &str, f32); 6] = [
            (Some(SortBy::Name), "lib.sort.name", cols_x.name),
            (None, "lib.col.kind", cols_x.kind),
            (None, "lib.col.format", cols_x.format),
            (Some(SortBy::Tris), "lib.sort.tris", cols_x.tris),
            (Some(SortBy::Size), "lib.sort.size", cols_x.size),
            (Some(SortBy::Rating), "lib.sort.rating", cols_x.rating),
        ];
        for (sort, key, x) in heads {
            if (key == "lib.col.kind" || key == "lib.col.format") && !wide {
                continue;
            }
            let hr = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(70.0, 22.0));
            let resp = ui.interact(hr, ui.id().with(("lh", key)), if sort.is_some() { Sense::click() } else { Sense::hover() });
            let active = sort == Some(cur.by);
            let arrow = if active { if cur.desc { " \u{25be}" } else { " \u{25b4}" } } else { "" };
            ui.painter().text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, format!("{}{arrow}", tr(key)), egui::FontId::proportional(11.5), if active { theme::ACCENT } else { theme::TEXT_DIM });
            if resp.clicked() {
                if let Some(s) = sort {
                    self.filters.sort = Sort { by: s, desc: if active { !cur.desc } else { matches!(s, SortBy::Rating | SortBy::Size | SortBy::Tris) } };
                    self.filters.user_sorted = true;
                    self.filters_changed();
                }
            }
        }
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, theme::BORDER));

        let ppp = ui.ctx().pixels_per_point();
        let want_bucket = bucket(LIST_ROW_H * ppp);
        self.cols = 1;
        ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
        let mut sa = egui::ScrollArea::vertical().id_salt("lib_list").auto_shrink([false, false]);
        if let Some(off) = self.pending_scroll.take() {
            sa = sa.vertical_scroll_offset(off);
        }
        let mut events: Vec<GridEvent> = Vec::new();
        let mut card_hit = false;
        let (mut first_item, mut last_item) = (0usize, 0usize);
        let trash_view = matches!(self.filters.section, Section::Trash);
        let out = {
            let Self { list, tex, selection, .. } = self;
            sa.show_rows(ui, LIST_ROW_H, n, |ui, range| {
                let origin = ui.max_rect().left_top();
                first_item = range.start;
                last_item = range.end;
                let cols_x = list_columns(origin.x + 6.0, w, wide);
                for idx in range.clone() {
                    let r = idx - range.start;
                    let rect = Rect::from_min_size(Pos2::new(origin.x, origin.y + r as f32 * (LIST_ROW_H + 2.0)), Vec2::new(w + 8.0, LIST_ROW_H));
                    match list.row(idx) {
                        Some(row) => {
                            let tex_id = tex.get(&row.content_hash, want_bucket).map(|t| t.id()).or_else(|| tex.best_available(&row.content_hash).map(|t| t.id()));
                            paint_list_row(ui, rect, idx, row, selection.contains(&row.id), tex_id, trash_view, &cols_x, wide, &selection.ids, &mut events, &mut card_hit);
                        }
                        None => paint_skeleton(ui, rect),
                    }
                }
            })
        };
        self.visible = (first_item, last_item);
        self.after_scroll(ui, &out, n, LIST_ROW_H + 2.0, n, want_bucket, 1);
        self.apply_grid_events(events, card_hit, out.inner_rect, ui);
    }

    // =============================================================== entrada

    pub(super) fn handle_global_input(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dlg.any_modal_open() {
            return;
        }
        let (ctrl, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let key = |k: egui::Key| ctx.input(|i| i.key_pressed(k));
        if ctrl && key(egui::Key::A) {
            self.select_all();
        }
        if (ctrl && key(egui::Key::F)) || ctx.input(|i| i.key_pressed(egui::Key::Slash)) {
            self.focus_search = true;
        }
        if key(egui::Key::Escape) {
            self.selection.clear();
            self.sync_detail_target();
        }
        if key(egui::Key::Delete) {
            if matches!(self.filters.section, Section::Trash) {
                self.card_action(CardAction::Purge);
            } else {
                self.card_action(CardAction::Trash);
            }
        }
        if key(egui::Key::Enter) && self.selection.len() == 1 {
            self.card_action(CardAction::Edit);
        }
        if key(egui::Key::F) && !ctrl && !self.selection.is_empty() {
            let make_fav = self.selection.primary.and_then(|id| self.row_of(&id)).map(|r| !r.favorite).unwrap_or(true);
            self.card_action(CardAction::Favorite(make_fav));
        }
        let mut step: Option<isize> = None;
        let cols = self.cols as isize;
        if key(egui::Key::ArrowRight) {
            step = Some(1);
        }
        if key(egui::Key::ArrowLeft) {
            step = Some(-1);
        }
        if key(egui::Key::ArrowDown) {
            step = Some(cols);
        }
        if key(egui::Key::ArrowUp) {
            step = Some(-cols);
        }
        if let Some(s) = step {
            let total = self.list.total_or_last() as isize;
            if total > 0 {
                let cur = self.selection.cursor.unwrap_or(0) as isize;
                let next = if self.selection.cursor.is_none() { 0 } else { (cur + s).clamp(0, total - 1) };
                if let Some(row) = self.list.row(next as usize) {
                    let id = row.id;
                    let mods = egui::Modifiers { shift, ..Default::default() };
                    self.click_item(next as usize, id, mods);
                    self.scroll_to_index(next as usize);
                }
            }
        }
    }

    pub(super) fn scroll_to_index(&mut self, idx: usize) {
        let cols = self.cols.max(1);
        let r = (idx / cols) as f32;
        let (top, bottom) = (r * self.row_h, (r + 1.0) * self.row_h);
        if top < self.scroll_y {
            self.pending_scroll = Some(top);
        } else if bottom > self.scroll_y + self.viewport_h {
            self.pending_scroll = Some((bottom - self.viewport_h).max(0.0));
        }
    }

    /// Linha em cache de um ativo (se alguma página carregada a contém).
    pub(crate) fn row_of(&self, id: &AssetId) -> Option<&AssetRow> {
        let (first, last) = self.visible;
        (first.saturating_sub(PAGE)..last + PAGE).find_map(|i| self.list.row(i).filter(|r| r.id == *id))
    }
}

// =================================================================== pintura

fn paint_skeleton(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(0x1a, 0x1a, 0x1f));
}

fn state_badge(row: &AssetRow) -> Option<(&'static str, Color32)> {
    match row.state {
        AssetState::Missing => Some((icons::WARNING, err_color())),
        AssetState::Corrupt => Some((icons::WARNING_CIRCLE, err_color())),
        AssetState::Trashed => Some((icons::TRASH, warn_color())),
        AssetState::Active if !row.available => Some((icons::PLUGS, warn_color())),
        _ => None,
    }
}

fn subtitle(row: &AssetRow) -> String {
    if row.tris > 0 {
        format!("{} \u{00b7} {} tris", kind_label(row.kind), fmt_short(row.tris))
    } else if row.kind == AssetKind::Texture || row.kind == AssetKind::Hdri {
        format!("{} \u{00b7} {}", kind_label(row.kind), fmt_bytes(row.size))
    } else {
        kind_label(row.kind).to_string()
    }
}

fn truncated(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, color, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    ui.painter().layout_job(job)
}

fn tooltip(ui: &mut Ui, row: &AssetRow) {
    ui.label(RichText::new(&row.name).strong());
    ui.label(small_dim(format!("{} \u{00b7} {} \u{00b7} {}", kind_label(row.kind), row.format.to_uppercase(), fmt_bytes(row.size))));
    if row.tris > 0 {
        ui.label(small_dim(format!("{} tris \u{00b7} {} {}", fmt_int(row.tris), fmt_int(row.verts), tr("lib.verts"))));
    }
    if row.state == AssetState::Missing {
        ui.label(RichText::new(tr("lib.state.missing")).color(err_color()).size(12.0));
    }
    if !row.available {
        ui.label(RichText::new(tr("lib.state.offline")).color(warn_color()).size(12.0));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_card(ui: &mut Ui, rect: Rect, idx: usize, row: &AssetRow, cc: &CardCtx, unavailable: bool, trash_view: bool, selected_ids: &HashSet<AssetId>, events: &mut Vec<GridEvent>, card_hit: &mut bool) {
    let id = ui.id().with(("card", row.seq));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let hovered = resp.hovered();
    *card_hit |= resp.contains_pointer();
    let painter = ui.painter().clone();
    let bg = if cc.selected {
        theme::ACCENT_DIM.gamma_multiply(0.55)
    } else if hovered {
        theme::BG_HOVER
    } else {
        theme::BG_WIDGET
    };
    painter.rect_filled(rect, CornerRadius::same(6), bg);
    if cc.selected {
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.5, theme::SELECT), StrokeKind::Inside);
    }
    let thumb = Rect::from_min_size(rect.min + Vec2::splat(PAD), Vec2::splat(cc.thumb_side));
    painter.rect_filled(thumb, CornerRadius::same(4), theme::BG_DEEP);
    let dim_alpha = if trash_view || row.state != AssetState::Active || !row.available { 0.55 } else { 1.0 };
    match cc.tex {
        Some(t) => {
            painter.image(t, thumb, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(dim_alpha));
        }
        None => {
            let col = kind_color(row.kind).gamma_multiply(if unavailable { 0.75 } else { 0.35 });
            painter.text(thumb.center(), Align2::CENTER_CENTER, kind_icon(row.kind), egui::FontId::proportional(cc.thumb_side * 0.38), col);
        }
    }
    // selo do formato (canto superior esquerdo) e estado
    let fmt = row.format.to_uppercase();
    let g = painter.layout_no_wrap(fmt, egui::FontId::proportional(9.5), Color32::from_rgb(0xe0, 0xe0, 0xea));
    let pill = Rect::from_min_size(thumb.min + Vec2::splat(4.0), g.size() + Vec2::new(8.0, 3.0));
    painter.rect_filled(pill, CornerRadius::same(3), Color32::from_black_alpha(150));
    painter.galley(pill.min + Vec2::new(4.0, 1.5), g, Color32::WHITE);
    if let Some((ic, col)) = state_badge(row) {
        painter.text(Pos2::new(thumb.left() + 10.0, thumb.bottom() - 12.0), Align2::LEFT_CENTER, ic, egui::FontId::proportional(16.0), col);
    }

    // favorito (canto superior direito)
    let heart = Rect::from_center_size(Pos2::new(thumb.right() - 13.0, thumb.top() + 13.0), Vec2::splat(22.0));
    if hovered || cc.selected || row.favorite {
        let hid = id.with("fav");
        let hresp = ui.interact(heart, hid, Sense::click());
        let col = if row.favorite { Color32::from_rgb(0xff, 0x5c, 0x7a) } else if hresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(170) };
        painter.circle_filled(heart.center(), 11.0, Color32::from_black_alpha(if hresp.hovered() { 170 } else { 110 }));
        painter.text(heart.center(), Align2::CENTER_CENTER, icons::HEART, if row.favorite { fill_font(14.0) } else { egui::FontId::proportional(14.0) }, col);
        if hresp.clicked() {
            events.push(GridEvent::Favorite { id: row.id, value: !row.favorite });
        }
        hresp.on_hover_text(tr("lib.favorite"));
    }

    // nota (barra inferior da miniatura)
    let stars = Rect::from_min_size(Pos2::new(thumb.center().x - 40.0, thumb.bottom() - 17.0), Vec2::new(80.0, 14.0));
    if hovered || cc.selected {
        painter.rect_filled(stars.expand2(Vec2::new(4.0, 1.0)), CornerRadius::same(7), Color32::from_black_alpha(150));
        if let Some(v) = rating_row(ui, id.with("rate"), stars, row.rating, true) {
            events.push(GridEvent::Rating { id: row.id, value: v });
        }
    } else if row.rating > 0 {
        let small = Rect::from_min_size(Pos2::new(thumb.right() - 44.0, thumb.bottom() - 15.0), Vec2::new(40.0, 11.0));
        let step = small.width() / 5.0;
        for i in 0..row.rating {
            paint_star(&painter, Pos2::new(small.left() + step * (i as f32 + 0.5), small.center().y), step * 0.42, true, Color32::from_rgb(0xff, 0xc8, 0x3d));
        }
    }

    // nome e subtítulo
    let text_w = rect.width() - 2.0 * PAD;
    let name = truncated(ui, &row.name, egui::FontId::proportional(12.5), theme::TEXT, text_w);
    painter.galley(Pos2::new(rect.left() + PAD, thumb.bottom() + 5.0), name, theme::TEXT);
    let mut sub = subtitle(row);
    if row.storage == StorageMode::Linked {
        sub = format!("{} {sub}", icons::LINK);
    }
    let subg = truncated(ui, &sub, egui::FontId::proportional(10.5), theme::TEXT_DIM, text_w);
    painter.galley(Pos2::new(rect.left() + PAD, thumb.bottom() + 20.0), subg, theme::TEXT_DIM);

    // interações do card
    let mods = ui.input(|i| i.modifiers);
    if resp.clicked() {
        events.push(GridEvent::Click { idx, id: row.id, mods });
    }
    if resp.double_clicked() {
        events.push(GridEvent::Double { id: row.id });
    }
    if resp.secondary_clicked() {
        events.push(GridEvent::ContextSelect { idx, id: row.id });
    }
    if resp.drag_started() {
        events.push(GridEvent::DragStart { idx, id: row.id });
        let ids: Vec<AssetId> = if selected_ids.contains(&row.id) { selected_ids.iter().copied().collect() } else { vec![row.id] };
        let label = if ids.len() > 1 { format!("{} {}", ids.len(), tr("lib.items")) } else { row.name.clone() };
        resp.dnd_set_drag_payload(AssetDragPayload { ids, label, kind: row.kind });
    }
    resp.context_menu(|ui| context_menu(ui, row, trash_view, events));
    resp.on_hover_ui(|ui| tooltip(ui, row));
}

fn context_menu(ui: &mut Ui, row: &AssetRow, trash_view: bool, events: &mut Vec<GridEvent>) {
    let mut act = |ui: &mut Ui, a: CardAction, icon: &str, key: &str, enabled: bool| {
        if ui.add_enabled(enabled, egui::Button::new(format!("{icon}  {}", tr(key)))).clicked() {
            events.push(GridEvent::Action(a));
            ui.close();
        }
    };
    if trash_view {
        act(ui, CardAction::Restore, icons::ARROW_COUNTER_CLOCKWISE, "lib.act.restore", true);
        act(ui, CardAction::Purge, icons::TRASH, "lib.act.purge", true);
    } else {
        act(ui, CardAction::Edit, icons::CUBE_FOCUS, "lib.act.edit", editable_in_modeling(row.kind, &row.format));
        act(ui, CardAction::Favorite(!row.favorite), icons::HEART, if row.favorite { "lib.act.unfavorite" } else { "lib.act.favorite" }, true);
        act(ui, CardAction::Duplicate, icons::COPY, "lib.act.duplicate", true);
        ui.separator();
        act(ui, CardAction::Trash, icons::TRASH, "lib.act.trash", true);
    }
}

struct ListCols {
    name: f32,
    kind: f32,
    format: f32,
    tris: f32,
    size: f32,
    rating: f32,
    fav: f32,
}

fn list_columns(left: f32, w: f32, wide: bool) -> ListCols {
    let right = left + w;
    if wide {
        ListCols { name: left + 34.0, kind: right - 450.0, format: right - 350.0, tris: right - 290.0, size: right - 205.0, rating: right - 125.0, fav: right - 18.0 }
    } else {
        ListCols { name: left + 34.0, kind: f32::MAX, format: f32::MAX, tris: right - 270.0, size: right - 190.0, rating: right - 115.0, fav: right - 18.0 }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_list_row(ui: &mut Ui, rect: Rect, idx: usize, row: &AssetRow, selected: bool, tex: Option<egui::TextureId>, trash_view: bool, cols: &ListCols, wide: bool, selected_ids: &HashSet<AssetId>, events: &mut Vec<GridEvent>, card_hit: &mut bool) {
    let id = ui.id().with(("row", row.seq));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    *card_hit |= resp.contains_pointer();
    let painter = ui.painter().clone();
    let bg = if selected {
        theme::ACCENT_DIM.gamma_multiply(0.55)
    } else if resp.hovered() {
        theme::BG_HOVER
    } else if idx % 2 == 0 {
        Color32::from_rgb(0x1f, 0x1f, 0x25)
    } else {
        theme::BG_PANEL
    };
    painter.rect_filled(rect, CornerRadius::same(4), bg);
    if selected {
        painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, theme::SELECT), StrokeKind::Inside);
    }
    let thumb = Rect::from_min_size(Pos2::new(rect.left() + 4.0, rect.top() + 2.0), Vec2::splat(LIST_ROW_H - 4.0));
    painter.rect_filled(thumb, CornerRadius::same(3), theme::BG_DEEP);
    match tex {
        Some(t) => {
            painter.image(t, thumb, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.text(thumb.center(), Align2::CENTER_CENTER, kind_icon(row.kind), egui::FontId::proportional(14.0), kind_color(row.kind).gamma_multiply(0.6));
        }
    }
    let cy = rect.center().y;
    let dim = trash_view || row.state != AssetState::Active || !row.available;
    let name_w = (if wide { cols.kind } else { cols.tris }) - cols.name - 8.0;
    let g = truncated(ui, &row.name, egui::FontId::proportional(12.5), if dim { theme::TEXT_DIM } else { theme::TEXT }, name_w.max(40.0));
    painter.galley(Pos2::new(cols.name, cy - g.size().y / 2.0), g, theme::TEXT);
    if let Some((ic, col)) = state_badge(row) {
        painter.text(Pos2::new(cols.name - 14.0, cy), Align2::CENTER_CENTER, ic, egui::FontId::proportional(12.0), col);
    }
    if wide {
        painter.text(Pos2::new(cols.kind, cy), Align2::LEFT_CENTER, format!("{}  {}", kind_icon(row.kind), kind_label(row.kind)), egui::FontId::proportional(11.5), kind_color(row.kind));
        painter.text(Pos2::new(cols.format, cy), Align2::LEFT_CENTER, row.format.to_uppercase(), egui::FontId::proportional(11.5), theme::TEXT_DIM);
    }
    painter.text(Pos2::new(cols.tris, cy), Align2::LEFT_CENTER, if row.tris > 0 { fmt_int(row.tris) } else { "\u{2014}".into() }, egui::FontId::proportional(11.5), theme::TEXT_DIM);
    painter.text(Pos2::new(cols.size, cy), Align2::LEFT_CENTER, fmt_bytes(row.size), egui::FontId::proportional(11.5), theme::TEXT_DIM);
    let stars = Rect::from_min_size(Pos2::new(cols.rating, cy - 7.0), Vec2::new(80.0, 14.0));
    if let Some(v) = rating_row(ui, id.with("rate"), stars, row.rating, true) {
        events.push(GridEvent::Rating { id: row.id, value: v });
    }
    let heart = Rect::from_center_size(Pos2::new(cols.fav, cy), Vec2::splat(22.0));
    let hresp = ui.interact(heart, id.with("fav"), Sense::click());
    let col = if row.favorite { Color32::from_rgb(0xff, 0x5c, 0x7a) } else if hresp.hovered() { Color32::WHITE } else { theme::TEXT_DIM.gamma_multiply(0.6) };
    painter.text(heart.center(), Align2::CENTER_CENTER, icons::HEART, if row.favorite { fill_font(14.0) } else { egui::FontId::proportional(14.0) }, col);
    if hresp.clicked() {
        events.push(GridEvent::Favorite { id: row.id, value: !row.favorite });
    }

    let mods = ui.input(|i| i.modifiers);
    if resp.clicked() {
        events.push(GridEvent::Click { idx, id: row.id, mods });
    }
    if resp.double_clicked() {
        events.push(GridEvent::Double { id: row.id });
    }
    if resp.secondary_clicked() {
        events.push(GridEvent::ContextSelect { idx, id: row.id });
    }
    if resp.drag_started() {
        events.push(GridEvent::DragStart { idx, id: row.id });
        let ids: Vec<AssetId> = if selected_ids.contains(&row.id) { selected_ids.iter().copied().collect() } else { vec![row.id] };
        let label = if ids.len() > 1 { format!("{} {}", ids.len(), tr("lib.items")) } else { row.name.clone() };
        resp.dnd_set_drag_payload(AssetDragPayload { ids, label, kind: row.kind });
    }
    resp.context_menu(|ui| context_menu(ui, row, trash_view, events));
}
