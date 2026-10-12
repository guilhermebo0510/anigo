//! Painel esquerdo: biblioteca (todos, favoritos, lixeira, ausentes, duplicados), locais (raízes gerenciadas e
//! ligadas), tags hierárquicas com contagem e coleções (manuais e inteligentes). As linhas também são alvos de
//! soltar (arrastar cards para uma coleção, tag, favoritos ou lixeira).

use super::widgets::*;
use super::*;
use crate::i18n::tr;
use crate::icons;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

#[derive(Default)]
pub struct SideUi {
    pub open_tags: HashSet<String>,
    pub new_collection: String,
    pub adding_collection: bool,
    pub rename: Option<(CollectionId, String)>,
    pub root_path: String,
    pub root_linked: bool,
}

enum SideAction {
    Select(Section),
    OpenDuplicates,
    ScanRoot(RootId),
    ScanAll,
    RemoveRoot(RootInfo),
    RelinkRoot(RootInfo),
    CreateCollection(String),
    RenameCollection(CollectionId, String),
    DeleteCollection(CollectionId, String),
    UpdateSmart(CollectionId),
    Drop(DropTarget, Vec<AssetId>),
    AddRoot,
    PickRoot,
}

#[derive(Clone)]
pub(super) enum DropTarget {
    Collection(CollectionId, String),
    Tag(String),
    Favorites,
    Trash,
}

/// Linha de navegação com ícone, rótulo e contagem à direita. Também é alvo de soltar.
fn nav_row(ui: &mut Ui, selected: bool, indent: f32, icon: &str, icon_color: Color32, label: &str, count: Option<u64>, droppable: bool) -> (egui::Response, bool) {
    let h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let dropping = droppable && resp.dnd_hover_payload::<AssetDragPayload>().is_some();
    let bg = if dropping {
        theme::ACCENT_DIM
    } else if selected {
        theme::ACCENT_DIM.gamma_multiply(0.6)
    } else if resp.hovered() {
        theme::BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), bg);
    if dropping {
        p.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
    }
    p.text(Pos2::new(rect.left() + 8.0 + indent, rect.center().y), Align2::LEFT_CENTER, icon, egui::FontId::proportional(14.0), icon_color);
    let text_x = rect.left() + 28.0 + indent;
    let right_pad = if count.is_some() { 52.0 } else { 6.0 };
    let mut job = egui::text::LayoutJob::simple(label.to_string(), egui::FontId::proportional(12.5), if selected { Color32::WHITE } else { theme::TEXT }, (rect.right() - text_x - right_pad).max(20.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    let g = p.layout_job(job);
    p.galley(Pos2::new(text_x, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    if let Some(c) = count {
        p.text(Pos2::new(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, fmt_short(c), egui::FontId::proportional(11.0), theme::TEXT_DIM);
    }
    (resp, dropping)
}

fn section_header(ui: &mut Ui, text: &str) -> Rect {
    ui.add_space(8.0);
    let r = ui.horizontal(|ui| {
        ui.label(RichText::new(text.to_uppercase()).size(10.5).strong().color(theme::TEXT_DIM));
    });
    r.response.rect
}

impl LibraryWorkspace {
    pub(super) fn left_panel(&mut self, ui: &mut Ui) {
        let mut actions: Vec<SideAction> = Vec::new();
        egui::Panel::left("lib_left").default_size(250.0).size_range(190.0..=420.0).resizable(true).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("lib_left_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                self.side_library(ui, &mut actions);
                self.side_roots(ui, &mut actions);
                self.side_tags(ui, &mut actions);
                self.side_collections(ui, &mut actions);
                ui.add_space(12.0);
            });
        });
        for a in actions {
            self.apply_side_action(a);
        }
    }

    fn side_library(&mut self, ui: &mut Ui, actions: &mut Vec<SideAction>) {
        section_header(ui, tr("lib.side.library"));
        let f = &self.facets;
        let cur = self.filters.section.clone();
        let (n_all, n_fav, n_trash, n_missing, n_dup) = (f.n_all, f.n_favorites, f.n_trash, f.n_missing, f.dup_groups as u64);
        let rows: [(Section, &str, &str, u64, Color32, bool); 4] = [
            (Section::All, icons::SQUARES_FOUR, "lib.all", n_all, theme::ACCENT, false),
            (Section::Favorites, icons::HEART, "lib.favorites", n_fav, Color32::from_rgb(0xff, 0x5c, 0x7a), true),
            (Section::Trash, icons::TRASH, "lib.trash", n_trash, theme::TEXT_DIM, true),
            (Section::Missing, icons::WARNING, "lib.missing", n_missing, if n_missing > 0 { warn_color() } else { theme::TEXT_DIM }, false),
        ];
        for (sec, icon, key, count, color, droppable) in rows {
            let (resp, _) = nav_row(ui, cur == sec, 0.0, icon, color, tr(key), Some(count), droppable);
            if resp.clicked() {
                actions.push(SideAction::Select(sec.clone()));
            }
            if droppable {
                if let Some(p) = resp.dnd_release_payload::<AssetDragPayload>() {
                    let target = if sec == Section::Trash { DropTarget::Trash } else { DropTarget::Favorites };
                    actions.push(SideAction::Drop(target, p.ids.clone()));
                }
            }
        }
        let (resp, _) = nav_row(ui, false, 0.0, icons::COPY, if n_dup > 0 { warn_color() } else { theme::TEXT_DIM }, tr("lib.duplicates"), Some(n_dup), false);
        if resp.clicked() {
            actions.push(SideAction::OpenDuplicates);
        }
        if self.facets.dup_reclaimable > 0 {
            resp.on_hover_text(format!("{} {}", fmt_bytes(self.facets.dup_reclaimable), tr("lib.dup.reclaimable")));
        }
    }

    fn side_roots(&mut self, ui: &mut Ui, actions: &mut Vec<SideAction>) {
        let r = section_header(ui, tr("lib.side.roots"));
        let _ = r;
        let cur = self.filters.section.clone();
        let roots = self.facets.roots.clone();
        for root in &roots {
            let (icon, mut color) = match root.mode {
                RootMode::Managed => (icons::HARD_DRIVES, theme::ACCENT),
                RootMode::Linked => (icons::LINK, Color32::from_rgb(0x6c, 0xd0, 0xc0)),
            };
            if !root.online {
                color = err_color();
            }
            let label = if root.id == HOME_ROOT { tr("lib.root.home").to_string() } else { root.name.clone() };
            let (resp, _) = nav_row(ui, cur == Section::Root(root.id), 0.0, if root.online { icon } else { icons::PLUGS }, color, &label, Some(root.asset_count), false);
            if resp.clicked() {
                actions.push(SideAction::Select(Section::Root(root.id)));
            }
            let tip = format!("{}\n{}{}", root.path.display(), if root.mode == RootMode::Managed { tr("lib.root.managed") } else { tr("lib.root.linked") }, if root.online { "" } else { " \u{00b7} offline" });
            let resp = resp.on_hover_text(tip);
            let root_c = root.clone();
            resp.context_menu(|ui| {
                if root_c.mode == RootMode::Linked && ui.button(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("lib.root.rescan"))).clicked() {
                    actions.push(SideAction::ScanRoot(root_c.id));
                    ui.close();
                }
                if root_c.mode == RootMode::Linked && ui.button(format!("{}  {}", icons::LINK, tr("lib.root.relink"))).clicked() {
                    actions.push(SideAction::RelinkRoot(root_c.clone()));
                    ui.close();
                }
                if root_c.id != HOME_ROOT && ui.button(format!("{}  {}", icons::TRASH, tr("lib.root.remove"))).clicked() {
                    actions.push(SideAction::RemoveRoot(root_c.clone()));
                    ui.close();
                }
            });
        }
        // adicionar local por caminho digitado ou pelo navegador de pastas
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 62.0).max(60.0);
            let resp = ui.add(egui::TextEdit::singleline(&mut self.side_ui.root_path).hint_text(tr("lib.root.path_hint")).desired_width(w));
            if icon_button(ui, icons::FOLDER_OPEN, tr("lib.root.add_folder")).clicked() {
                actions.push(SideAction::PickRoot);
            }
            let can = !self.side_ui.root_path.trim().is_empty();
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if icon_button_enabled(ui, can, icons::PLUS, tr("lib.root.add")).clicked() || (enter && can) {
                actions.push(SideAction::AddRoot);
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.side_ui.root_linked, true, RichText::new(format!("{} {}", icons::LINK, tr("lib.root.linked"))).size(11.5)).on_hover_text(tr("lib.root.linked.tip"));
            ui.selectable_value(&mut self.side_ui.root_linked, false, RichText::new(format!("{} {}", icons::HARD_DRIVES, tr("lib.root.managed"))).size(11.5)).on_hover_text(tr("lib.root.managed.tip"));
            if icon_button(ui, icons::ARROWS_CLOCKWISE, tr("lib.root.rescan_all")).clicked() {
                actions.push(SideAction::ScanAll);
            }
        });
    }

    fn side_tags(&mut self, ui: &mut Ui, actions: &mut Vec<SideAction>) {
        section_header(ui, tr("lib.side.tags"));
        if self.facets.tags.is_empty() {
            ui.label(small_dim(tr("lib.tags.none")));
            return;
        }
        let cur = self.filters.section.clone();
        // ancestrais da tag selecionada ficam abertos
        if let Section::Tag(t) = &cur {
            let mut acc = String::new();
            for seg in t.split('/') {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                if acc != *t {
                    self.side_ui.open_tags.insert(acc.clone());
                }
            }
        }
        let tags = self.facets.tags.clone();
        let mut hidden_below: Option<usize> = None; // profundidade de um nó fechado cujos filhos ocultamos
        for node in &tags {
            if let Some(d) = hidden_below {
                if node.depth > d {
                    continue;
                }
                hidden_below = None;
            }
            let open = self.side_ui.open_tags.contains(&node.path);
            let indent = node.depth as f32 * 14.0;
            let selected = cur == Section::Tag(node.path.clone());
            let dot = parse_hex(&node.color).unwrap_or(theme::TEXT_DIM);
            let icon = if node.has_children { if open { icons::CARET_DOWN } else { icons::CARET_RIGHT } } else { icons::TAG };
            let (resp, _) = nav_row(ui, selected, indent, icon, dot, &node.name, Some(node.total), true);
            if resp.clicked() {
                let caret_zone = resp.interact_pointer_pos().is_some_and(|p| p.x < resp.rect.left() + 28.0 + indent);
                if node.has_children && caret_zone {
                    if open {
                        self.side_ui.open_tags.remove(&node.path);
                    } else {
                        self.side_ui.open_tags.insert(node.path.clone());
                    }
                } else {
                    actions.push(SideAction::Select(Section::Tag(node.path.clone())));
                }
            }
            if let Some(p) = resp.dnd_release_payload::<AssetDragPayload>() {
                actions.push(SideAction::Drop(DropTarget::Tag(node.path.clone()), p.ids.clone()));
            }
            resp.on_hover_text(&node.path);
            if node.has_children && !open {
                hidden_below = Some(node.depth);
            }
        }
    }

    fn side_collections(&mut self, ui: &mut Ui, actions: &mut Vec<SideAction>) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("lib.side.collections").to_uppercase()).size(10.5).strong().color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, icons::PLUS, tr("lib.coll.new")).clicked() {
                    self.side_ui.adding_collection = true;
                }
            });
        });
        if self.side_ui.adding_collection {
            ui.horizontal(|ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.side_ui.new_collection).hint_text(tr("lib.coll.name_hint")).desired_width(ui.available_width() - 30.0));
                resp.request_focus();
                let name = self.side_ui.new_collection.trim().to_string();
                if (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || icon_button_enabled(ui, !name.is_empty(), icons::CHECK, tr("lib.coll.create")).clicked() {
                    if !name.is_empty() {
                        actions.push(SideAction::CreateCollection(name));
                    }
                    self.side_ui.adding_collection = false;
                    self.side_ui.new_collection.clear();
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.side_ui.adding_collection = false;
                }
            });
        }
        if self.facets.collections.is_empty() && !self.side_ui.adding_collection {
            ui.label(small_dim(tr("lib.coll.none")));
        }
        let cur = self.filters.section.clone();
        let colls = self.facets.collections.clone();
        for (c, n) in &colls {
            if let Some((rid, text)) = &mut self.side_ui.rename {
                if *rid == c.id {
                    let resp = ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width() - 4.0));
                    resp.request_focus();
                    if resp.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                            actions.push(SideAction::RenameCollection(c.id, text.trim().to_string()));
                        }
                        self.side_ui.rename = None;
                    }
                    continue;
                }
            }
            let smart = c.is_smart();
            let icon = if smart { icons::MAGIC_WAND } else { icons::FOLDER_SIMPLE };
            let (resp, _) = nav_row(ui, cur == Section::Collection(c.id), 0.0, icon, if smart { Color32::from_rgb(0xe0, 0x9a, 0xff) } else { Color32::from_rgb(0xff, 0xc8, 0x6c) }, &c.name, Some(*n), !smart);
            if resp.clicked() {
                actions.push(SideAction::Select(Section::Collection(c.id)));
            }
            if !smart {
                if let Some(p) = resp.dnd_release_payload::<AssetDragPayload>() {
                    actions.push(SideAction::Drop(DropTarget::Collection(c.id, c.name.clone()), p.ids.clone()));
                }
            }
            let (cid, cname) = (c.id, c.name.clone());
            resp.on_hover_text(if smart { tr("lib.coll.smart") } else { tr("lib.coll.manual") }).context_menu(|ui| {
                if ui.button(format!("{}  {}", icons::PENCIL_SIMPLE, tr("lib.coll.rename"))).clicked() {
                    self.side_ui.rename = Some((cid, cname.clone()));
                    ui.close();
                }
                if smart && ui.button(format!("{}  {}", icons::FUNNEL, tr("lib.coll.update_smart"))).clicked() {
                    actions.push(SideAction::UpdateSmart(cid));
                    ui.close();
                }
                if ui.button(format!("{}  {}", icons::TRASH, tr("lib.coll.delete"))).clicked() {
                    actions.push(SideAction::DeleteCollection(cid, cname.clone()));
                    ui.close();
                }
            });
        }
    }

    fn apply_side_action(&mut self, a: SideAction) {
        match a {
            SideAction::Select(s) => self.set_section(s),
            SideAction::OpenDuplicates => self.open_duplicates(),
            SideAction::ScanRoot(r) => self.queue_scan(r),
            SideAction::ScanAll => {
                self.queue_scan_all_linked();
                self.info(tr("lib.scan.started"));
            }
            SideAction::RemoveRoot(r) => self.dlg.confirm = Some(dialogs::Confirm::RemoveRoot { id: r.id, name: r.name.clone(), managed: r.mode == RootMode::Managed, delete_files: false }),
            SideAction::RelinkRoot(r) => self.dlg.open_relink_root(r),
            SideAction::CreateCollection(name) => {
                self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.create_collection(&name).map(|_| String::new()).map_err(|e| e.to_string()));
            }
            SideAction::RenameCollection(id, name) => {
                self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.rename_collection(id, &name).map(|_| String::new()).map_err(|e| e.to_string()));
            }
            SideAction::DeleteCollection(id, name) => self.dlg.confirm = Some(dialogs::Confirm::DeleteCollection(id, name)),
            SideAction::UpdateSmart(id) => {
                let q = self.filters.to_query();
                self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.update_smart_collection(id, &Query { after: None, with_total: false, ..q }).map(|_| tr("lib.coll.updated").to_string()).map_err(|e| e.to_string()));
            }
            SideAction::Drop(target, ids) => self.drop_on(target, ids),
            SideAction::AddRoot => {
                let path = PathBuf::from(self.side_ui.root_path.trim());
                let linked = self.side_ui.root_linked;
                self.add_root(path, linked);
            }
            SideAction::PickRoot => {
                let purpose = if self.side_ui.root_linked { dialogs::PickPurpose::AddRootLinked } else { dialogs::PickPurpose::AddRootManaged };
                self.dlg.open_picker(purpose, None);
            }
        }
    }

    /// Registra um local (síncrono: só toca a configuração e o índice) e, se ligado, já escaneia.
    pub(crate) fn add_root(&mut self, path: PathBuf, linked: bool) {
        let Some(rt) = &self.rt else { return };
        let mode = if linked { RootMode::Linked } else { RootMode::Managed };
        match rt.lib.add_root(&path, mode, None) {
            Ok(id) => {
                self.side_ui.root_path.clear();
                self.info(format!("{}: {}", tr("lib.root.added"), path.display()));
                if linked {
                    self.queue_scan(id);
                }
                self.reload_facets();
                self.reload_rows(false);
            }
            Err(e) => self.error(format!("{}: {e}", tr("lib.root.add_failed"))),
        }
    }

    pub(super) fn drop_on(&mut self, target: DropTarget, ids: Vec<AssetId>) {
        if ids.is_empty() {
            return;
        }
        match target {
            DropTarget::Collection(c, name) => {
                let n = ids.len();
                self.command("collection", Refresh::ROWS_FACETS, move |lib| lib.add_to_collection(c, &ids).map(|_| format!("{n} \u{2192} {name}")).map_err(|e| e.to_string()));
            }
            DropTarget::Tag(tag) => {
                let n = ids.len();
                self.command("tag", Refresh { rows: false, facets: true, detail: true }, move |lib| lib.add_tags_many(&ids, std::slice::from_ref(&tag)).map(|_| format!("{n} \u{2192} #{tag}")).map_err(|e| e.to_string()));
            }
            DropTarget::Favorites => self.set_favorite(&ids, true),
            DropTarget::Trash => self.trash_ids(ids),
        }
    }
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
