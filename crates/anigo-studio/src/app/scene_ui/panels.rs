//! Painéis do Cenário: Outliner, Histórico, Details, Colocar atores, Content Drawer (Biblioteca) e o
//! diálogo de salvar/abrir cena.

use super::super::props_tabs::{PropTab, TabSet};
use super::actions::prim_label_key;
use super::{describe_scene, OutlinerDragPayload, PlaceDragPayload, PlaceKind, SceneDialog, StudioApp};
use crate::character::persist;
use crate::i18n::tr;
use crate::icons;
use crate::library_ui::HostAction;
use crate::scene::geometry::Slot;
use crate::scene::outliner::{build_rows, Query, Row};
use crate::scene::runtime::GeomState;
use crate::theme;
use anigo_core::command::Command;
use anigo_core::hierarchy::NodeKind;
use anigo_core::ids::NodeId;
use anigo_core::level::{focal_for_fov, vertical_fov, NodeOverrides, NodePatch};
use anigo_core::{NodeSource, NodeTransformEntry, PrimitiveShape};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke};
use glam::{EulerRot, Quat, Vec3};

const ROW_H: f32 = 22.0;
const SENSORS: [(&str, f32, f32); 5] = [
    ("Super35 (24,89 x 18,66)", 24.89, 18.66),
    ("Full-frame (36 x 24)", 36.0, 24.0),
    ("APS-C (23,6 x 15,6)", 23.6, 15.6),
    ("16:9 (24 x 13,5)", 24.0, 13.5),
    ("Micro 4/3 (17,3 x 13)", 17.3, 13.0),
];

fn node_icon(slot: &anigo_core::NodeSlot) -> &'static str {
    match (&slot.source, slot.kind) {
        (Some(NodeSource::Camera { .. }), _) => icons::VIDEO_CAMERA,
        (Some(NodeSource::LibraryAsset { .. }), _) => icons::PACKAGE,
        (Some(NodeSource::Primitive { shape }), _) => match shape {
            PrimitiveShape::Cube => icons::CUBE,
            PrimitiveShape::Sphere => icons::SPHERE,
            PrimitiveShape::Plane => icons::SQUARE,
            PrimitiveShape::Cylinder => icons::CYLINDER,
            PrimitiveShape::Cone => icons::TRIANGLE,
        },
        (_, NodeKind::CharacterRoot) => icons::USER,
        (_, NodeKind::Group) => icons::FOLDER_SIMPLE,
        (_, NodeKind::Light) => icons::LIGHTBULB,
        _ => icons::CUBE_FOCUS,
    }
}

fn euler_deg(q: Quat) -> [f32; 3] {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    [x.to_degrees() + 0.0, y.to_degrees() + 0.0, z.to_degrees() + 0.0]
}

fn quat_from_deg(v: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::YXZ, v[1].to_radians(), v[0].to_radians(), v[2].to_radians()).normalize()
}

impl StudioApp {
    // ============================================================ orquestração

    /// Início do quadro do Cenário (antes dos painéis).
    pub(crate) fn scene_frame_begin(&mut self) {
        self.scene_enter();
    }

    /// Fim do quadro: diálogo de abrir/salvar cena e o "fantasma" do arrasto de Posicionar.
    pub(crate) fn scene_frame_end(&mut self, ctx: &egui::Context) {
        self.scene_dialog(ctx);
        self.place_drag_ghost(ctx);
    }

    /// Painel Posicionar (corpo; o dock fornece quadro e cabeçalho).
    pub(crate) fn scene_place_body(&mut self, ui: &mut egui::Ui) {
        self.place_panel(ui);
    }

    /// Painel Outliner: abrir/salvar a cena + busca + árvore.
    pub(crate) fn scene_outliner_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(icons::FOLDER_OPEN).on_hover_text(tr("char.open")).clicked() {
                    self.scene_ui.dialog = Some(SceneDialog { save: false, path: self.scene_ui.last_path.clone(), error: None });
                }
                if ui.button(icons::FLOPPY_DISK).on_hover_text(tr("char.save")).clicked() {
                    self.scene_ui.dialog = Some(SceneDialog { save: true, path: self.scene_ui.last_path.clone(), error: None });
                }
            });
        });
        ui.separator();
        self.outliner_panel(ui);
    }

    /// Painel Histórico da cena.
    pub(crate) fn scene_history_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        self.history_panel(ui);
    }

    /// Painel Detalhes do nó selecionado.
    pub(crate) fn scene_details_body(&mut self, ui: &mut egui::Ui) {
        self.details_panel(ui);
    }

    /// Content Drawer (a grade da Biblioteca dentro do Cenário).
    pub(crate) fn scene_drawer_body(&mut self, ui: &mut egui::Ui) {
        self.drawer_panel(ui);
    }

    // ============================================================ Outliner

    fn outliner_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(theme::TEXT_DIM));
            let resp = ui.add(egui::TextEdit::singleline(&mut self.scene_ui.query).hint_text(tr("scene.outliner.search")).desired_width(ui.available_width() - 84.0));
            resp.on_hover_text(tr("scene.outliner.search_help"));
            if !self.scene_ui.query.is_empty() && ui.small_button(icons::X).clicked() {
                self.scene_ui.query.clear();
            }
            ui.menu_button(icons::FOLDER_PLUS, |ui| {
                ui.label(tr("scene.outliner.new_folder"));
                let r = ui.add(egui::TextEdit::singleline(&mut self.scene_ui.new_folder).hint_text("Props/Mesas").desired_width(150.0));
                let go = ui.button(tr("scene.ok")).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go {
                    let name = self.scene_ui.new_folder.trim().trim_matches('/').to_string();
                    if !name.is_empty() {
                        let ids = self.scene_ui.selection.ids().to_vec();
                        self.scene_move_to_folder(&ids, &name);
                    }
                    self.scene_ui.new_folder.clear();
                    ui.close();
                }
            })
            .response
            .on_hover_text(tr("scene.outliner.folder_tip"));
        });
        let query = Query::parse(&self.scene_ui.query);
        let rows = build_rows(self.core.project(), &self.scene_ui.expansion, &query);
        let n = rows.len();
        let mut clicks: Vec<(NodeId, egui::Modifiers)> = Vec::new();
        let mut toggles: Vec<NodeId> = Vec::new();
        let mut folder_toggles: Vec<String> = Vec::new();
        let mut vis: Vec<NodeId> = Vec::new();
        let mut lock: Vec<NodeId> = Vec::new();
        let mut reparent: Vec<(Vec<NodeId>, Option<NodeId>)> = Vec::new();
        let mut to_folder: Vec<(Vec<NodeId>, String)> = Vec::new();
        let mut begin_rename: Option<NodeId> = None;
        let mut focus: Option<NodeId> = None;
        let mut commit_rename: Option<(NodeId, String)> = None;
        let mut menu_action: Option<(char, NodeId)> = None;
        let avail_h = (ui.available_height() - 24.0).max(40.0);
        egui::ScrollArea::vertical().id_salt("scene_outliner_rows").max_height(avail_h).auto_shrink([false, false]).show_rows(ui, ROW_H, n.max(1), |ui, range| {
            if n == 0 {
                ui.label(RichText::new(tr("scene.outliner.empty")).color(theme::TEXT_DIM));
                return;
            }
            for idx in range {
                let Some(row) = rows.get(idx) else { continue };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                match row {
                    Row::Folder { path, name, depth, expanded, count } => {
                        let x0 = rect.min.x + 6.0 + *depth as f32 * 14.0;
                        if resp.hovered() {
                            painter.rect_filled(rect, 3.0, theme::BG_HOVER);
                        }
                        if resp.dnd_hover_payload::<OutlinerDragPayload>().is_some() {
                            painter.rect_stroke(rect, 3.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
                        }
                        painter.text(Pos2::new(x0, rect.center().y), egui::Align2::LEFT_CENTER, if *expanded { icons::CARET_DOWN } else { icons::CARET_RIGHT }, egui::FontId::proportional(12.0), theme::TEXT_DIM);
                        painter.text(Pos2::new(x0 + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, format!("{}  {}  ({})", icons::FOLDER_SIMPLE, name, count), egui::FontId::proportional(13.0), Color32::from_rgb(0xd8, 0xc8, 0x90));
                        if resp.clicked() {
                            folder_toggles.push(path.clone());
                        }
                        if let Some(p) = resp.dnd_release_payload::<OutlinerDragPayload>() {
                            to_folder.push((p.0.clone(), path.clone()));
                        }
                    }
                    Row::Node { id, depth, has_children, expanded } => {
                        let Some(slot) = self.core.project().scene.node(id) else { continue };
                        let selected = self.scene_ui.selection.contains(id);
                        let active = self.scene_ui.selection.active() == Some(id);
                        if selected {
                            painter.rect_filled(rect, 3.0, if active { theme::ACCENT_DIM } else { Color32::from_rgba_unmultiplied(0x5a, 0x48, 0xa8, 120) });
                        } else if resp.hovered() {
                            painter.rect_filled(rect, 3.0, theme::BG_HOVER);
                        }
                        let hover_drop = resp.dnd_hover_payload::<OutlinerDragPayload>().is_some();
                        if hover_drop {
                            painter.rect_stroke(rect, 3.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
                        }
                        let x0 = rect.min.x + 4.0 + *depth as f32 * 14.0;
                        let dim = !slot.visible;
                        let text_col = if dim { theme::TEXT_DIM } else { theme::TEXT };
                        if *has_children {
                            let caret = Rect::from_center_size(Pos2::new(x0 + 6.0, rect.center().y), egui::vec2(14.0, ROW_H));
                            painter.text(caret.center(), egui::Align2::CENTER_CENTER, if *expanded { icons::CARET_DOWN } else { icons::CARET_RIGHT }, egui::FontId::proportional(12.0), theme::TEXT_DIM);
                            if ui.interact(caret, resp.id.with("caret"), Sense::click()).clicked() {
                                toggles.push(id.clone());
                            }
                        }
                        painter.text(Pos2::new(x0 + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, node_icon(slot), egui::FontId::proportional(14.0), if selected { Color32::WHITE } else { Color32::from_rgb(0xa8, 0xb0, 0xd0) });
                        let renaming = self.scene_ui.rename.as_ref().is_some_and(|(r, _, _)| r == id);
                        let name_rect = Rect::from_min_max(Pos2::new(x0 + 36.0, rect.min.y), Pos2::new(rect.max.x - 52.0, rect.max.y));
                        if renaming {
                            if let Some((_, buf, first)) = self.scene_ui.rename.as_mut() {
                                let r = ui.put(name_rect.shrink2(egui::vec2(0.0, 1.0)), egui::TextEdit::singleline(buf).desired_width(name_rect.width()));
                                if *first {
                                    r.request_focus();
                                    *first = false;
                                }
                                if r.lost_focus() {
                                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        commit_rename = Some((id.clone(), String::new()));
                                    } else {
                                        commit_rename = Some((id.clone(), buf.clone()));
                                    }
                                }
                            }
                        } else {
                            let mut label = slot.name.clone();
                            if let Some(GeomState::Missing(_)) = self.scene_ui.runtime.geom.get(id) {
                                label = format!("{} {}", icons::WARNING, label);
                            }
                            let galley = painter.layout(label, egui::FontId::proportional(13.0), text_col, name_rect.width());
                            painter.with_clip_rect(name_rect).galley(Pos2::new(name_rect.min.x, rect.center().y - galley.size().y * 0.5), galley, text_col);
                        }
                        // olho e cadeado
                        let eye = Rect::from_center_size(Pos2::new(rect.max.x - 40.0, rect.center().y), egui::vec2(20.0, ROW_H));
                        let lck = Rect::from_center_size(Pos2::new(rect.max.x - 18.0, rect.center().y), egui::vec2(20.0, ROW_H));
                        painter.text(eye.center(), egui::Align2::CENTER_CENTER, if slot.visible { icons::EYE } else { icons::EYE_SLASH }, egui::FontId::proportional(14.0), if slot.visible { theme::TEXT_DIM } else { Color32::from_rgb(0xff, 0x90, 0x70) });
                        painter.text(lck.center(), egui::Align2::CENTER_CENTER, if slot.locked { icons::LOCK } else { icons::LOCK_OPEN }, egui::FontId::proportional(14.0), if slot.locked { Color32::from_rgb(0xff, 0xb0, 0x50) } else { Color32::from_rgb(0x50, 0x52, 0x60) });
                        if ui.interact(eye, resp.id.with("eye"), Sense::click()).clicked() {
                            vis.push(id.clone());
                        }
                        if ui.interact(lck, resp.id.with("lock"), Sense::click()).clicked() {
                            lock.push(id.clone());
                        }
                        if resp.clicked() && !renaming {
                            clicks.push((id.clone(), ui.input(|i| i.modifiers)));
                        }
                        if resp.double_clicked() {
                            begin_rename = Some(id.clone());
                        }
                        if resp.drag_started() {
                            let ids: Vec<NodeId> = if selected { self.scene_ui.selection.ids().to_vec() } else { vec![id.clone()] };
                            resp.dnd_set_drag_payload(OutlinerDragPayload(ids));
                        }
                        if let Some(p) = resp.dnd_release_payload::<OutlinerDragPayload>() {
                            reparent.push((p.0.clone(), Some(id.clone())));
                        }
                        resp.context_menu(|ui| {
                            if !self.scene_ui.selection.contains(id) {
                                self.scene_ui.selection.set_one(id.clone());
                            }
                            for (key, label, ch) in [
                                ("scene.menu.rename", icons::PENCIL_SIMPLE, 'r'),
                                ("scene.menu.focus", icons::FRAME_CORNERS, 'f'),
                                ("scene.menu.duplicate", icons::COPY, 'd'),
                                ("scene.menu.delete", icons::TRASH, 'x'),
                                ("scene.menu.group", icons::FOLDER_SIMPLE, 'g'),
                                ("scene.menu.ungroup", icons::FOLDER_SIMPLE_DASHED, 'u'),
                                ("scene.menu.select_children", icons::TREE_STRUCTURE, 'c'),
                                ("scene.menu.select_same", icons::SELECTION_ALL, 's'),
                                ("scene.menu.unparent", icons::ARROW_BEND_UP_LEFT, 'p'),
                            ] {
                                if ui.button(format!("{label}  {}", tr(key))).clicked() {
                                    menu_action = Some((ch, id.clone()));
                                    ui.close();
                                }
                            }
                        });
                        if resp.double_clicked() {
                            focus = Some(id.clone());
                        }
                    }
                }
            }
        });
        // soltar na área vazia abaixo da lista: volta à raiz (sem pai, sem pasta)
        let (dz, dz_resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
        if dz_resp.dnd_hover_payload::<OutlinerDragPayload>().is_some() {
            ui.painter().rect_stroke(dz, 3.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
        }
        ui.painter().text(dz.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, tr("scene.outliner.drop_root"), egui::FontId::proportional(11.0), theme::TEXT_DIM);
        if let Some(p) = dz_resp.dnd_release_payload::<OutlinerDragPayload>() {
            reparent.push((p.0.clone(), None));
            to_folder.push((p.0.clone(), String::new()));
        }

        // ---- aplica os eventos (depois de soltar o empréstimo das linhas)
        for (id, mods) in clicks {
            if mods.command || mods.shift {
                self.scene_ui.selection.toggle(id);
            } else {
                self.scene_ui.selection.set_one(id);
            }
        }
        for id in toggles {
            self.scene_ui.expansion.toggle_node(&id);
        }
        for f in folder_toggles {
            self.scene_ui.expansion.toggle_folder(&f);
        }
        for id in vis {
            self.scene_toggle_visible(&id);
        }
        for id in lock {
            let locked = self.core.project().scene.node(&id).is_some_and(|n| n.locked);
            self.scene_set_locked(&[id], !locked);
        }
        for (ids, parent) in reparent {
            self.scene_reparent(&ids, parent.as_ref());
        }
        for (ids, folder) in to_folder {
            self.scene_move_to_folder(&ids, &folder);
        }
        if let Some(id) = begin_rename {
            let name = self.core.project().scene.node(&id).map(|n| n.name.clone()).unwrap_or_default();
            self.scene_ui.rename = Some((id, name, true));
        }
        if let Some((id, name)) = commit_rename {
            self.scene_ui.rename = None;
            if !name.is_empty() {
                self.scene_rename(&id, &name);
            }
        }
        if let Some(id) = focus {
            self.scene_ui.selection.set_one(id);
            self.scene_focus();
        }
        if let Some((ch, id)) = menu_action {
            match ch {
                'r' => {
                    let name = self.core.project().scene.node(&id).map(|n| n.name.clone()).unwrap_or_default();
                    self.scene_ui.rename = Some((id, name, true));
                }
                'f' => self.scene_focus(),
                'd' => self.scene_duplicate(),
                'x' => self.scene_delete(),
                'g' => self.scene_group(),
                'u' => self.scene_ungroup(),
                'c' => self.scene_select_children(),
                's' => self.scene_select_same(),
                'p' => {
                    let ids = self.scene_ui.selection.ids().to_vec();
                    self.scene_reparent(&ids, None);
                }
                _ => {}
            }
        }
    }

    // ============================================================ Histórico

    fn history_panel(&mut self, ui: &mut egui::Ui) {
        let h = self.core.history();
        let undo: Vec<String> = h.undo_descriptions().iter().map(|d| describe_scene(d)).collect();
        let redo: Vec<String> = h.redo_descriptions().iter().map(|d| describe_scene(d)).collect();
        let mut jump: Option<usize> = None;
        ui.label(RichText::new(format!("{} {}  \u{b7}  {} {}", undo.len(), tr("scene.history.steps"), redo.len(), tr("scene.history.redoable"))).size(11.5).color(theme::TEXT_DIM));
        egui::ScrollArea::vertical().id_salt("scene_history").auto_shrink([false, false]).stick_to_bottom(false).show(ui, |ui| {
            let mut row = |ui: &mut egui::Ui, label: String, depth: usize, current: bool, dim: bool| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::click());
                if current {
                    ui.painter().rect_filled(rect, 3.0, theme::ACCENT_DIM);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, theme::BG_HOVER);
                }
                let col = if dim { theme::TEXT_DIM } else { theme::TEXT };
                ui.painter().text(rect.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("{depth:>3}   {label}"), egui::FontId::proportional(12.5), col);
                if resp.clicked() {
                    jump = Some(depth);
                }
            };
            row(ui, tr("scene.history.initial").to_string(), 0, undo.is_empty(), false);
            for (i, d) in undo.iter().enumerate() {
                row(ui, d.clone(), i + 1, i + 1 == undo.len(), false);
            }
            for (j, d) in redo.iter().enumerate() {
                row(ui, d.clone(), undo.len() + j + 1, false, true);
            }
        });
        if let Some(depth) = jump {
            if let Err(e) = self.core.jump_to_depth(depth) {
                self.scene_say(e);
            }
        }
    }

    // ============================================================ Colocar atores

    fn place_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new(tr("scene.place")).strong());
        ui.label(RichText::new(tr("scene.place.hint")).size(11.0).color(theme::TEXT_DIM));
        ui.separator();
        let mut chosen: Option<PlaceKind> = None;
        let mut item = |ui: &mut egui::Ui, icon: &str, label: String, kind: PlaceKind, enabled: bool| {
            let resp = ui.add_enabled(enabled, egui::Button::new(format!("{icon}  {label}")).sense(Sense::click_and_drag()).min_size(egui::vec2(ui.available_width(), 26.0)));
            if resp.drag_started() {
                resp.dnd_set_drag_payload(PlaceDragPayload(kind));
            }
            if resp.clicked() {
                chosen = Some(kind);
            }
        };
        ui.label(RichText::new(tr("scene.place.shapes")).size(11.5).color(theme::TEXT_DIM));
        for shape in Self::scene_prim_shapes() {
            let icon = match shape {
                PrimitiveShape::Cube => icons::CUBE,
                PrimitiveShape::Sphere => icons::SPHERE,
                PrimitiveShape::Plane => icons::SQUARE,
                PrimitiveShape::Cylinder => icons::CYLINDER,
                PrimitiveShape::Cone => icons::TRIANGLE,
            };
            item(ui, icon, tr(prim_label_key(shape)).to_string(), PlaceKind::Primitive(shape), true);
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("scene.place.other")).size(11.5).color(theme::TEXT_DIM));
        item(ui, icons::VIDEO_CAMERA, tr("scene.actor.camera").to_string(), PlaceKind::Camera, true);
        item(ui, icons::FOLDER_SIMPLE, tr("scene.actor.group").to_string(), PlaceKind::Group, true);
        ui.add_enabled(false, egui::Button::new(format!("{}  {}", icons::LIGHTBULB, tr("scene.actor.light"))).min_size(egui::vec2(ui.available_width(), 26.0))).on_disabled_hover_text(tr("scene.actor.light_tip"));
        if let Some(k) = chosen {
            self.scene_place_actor(k, None);
        }
    }

    fn place_drag_ghost(&self, ctx: &egui::Context) {
        let Some(p) = egui::DragAndDrop::payload::<PlaceDragPayload>(ctx) else { return };
        let Some(pos) = ctx.pointer_latest_pos() else { return };
        let label = match p.0 {
            PlaceKind::Primitive(s) => tr(prim_label_key(s)).to_string(),
            PlaceKind::Camera => tr("scene.actor.camera").to_string(),
            PlaceKind::Group => tr("scene.actor.group").to_string(),
        };
        egui::Area::new(egui::Id::new("scene_place_ghost")).order(egui::Order::Tooltip).fixed_pos(pos + egui::vec2(14.0, 14.0)).interactable(false).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(label));
        });
    }

    // ============================================================ Content Drawer

    fn drawer_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  {}", icons::SQUARES_FOUR, tr("scene.drawer"))).strong());
            ui.label(RichText::new(tr("scene.drawer.hint")).size(11.5).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::X).clicked() {
                    self.scene_ui.drawer_open = false;
                }
                if ui.small_button(icons::ARROWS_CLOCKWISE).on_hover_text(tr("scene.drawer.reload")).clicked() {
                    self.scene_ui.cache.forget_resolutions();
                    self.scene_ui.runtime.key = None;
                }
            });
        });
        ui.separator();
        let actions = self.lib_workspace().drawer_ui(ui);
        for a in actions {
            match a {
                HostAction::PlaceAsset { id } => {
                    self.scene_place_library(&[id], None);
                }
                HostAction::Status(s) => self.scene_say(s),
                HostAction::EditInModeling { .. } => {}
            }
        }
    }

    // ============================================================ Details

    /// Aplica uma edição ao vivo (arrasto de campo): preview agora, UM passo de desfazer ao soltar.
    fn scene_edit(&mut self, cmd: Command) {
        let _ = self.core.preview(cmd.clone());
        self.scene_ui.gesture = Some(cmd);
    }

    fn details_panel(&mut self, ui: &mut egui::Ui) {
        let sel = self.scene_ui.selection.ids().to_vec();
        let project = self.core.project().clone();
        let slots: Vec<&anigo_core::NodeSlot> = sel.iter().filter_map(|i| project.scene.node(i)).collect();
        // abas que valem para a seleção: Ativo só para ativos da Biblioteca, Lente só para uma câmera de cena
        let mut tabs: Vec<PropTab> = Vec::new();
        if let Some(first) = slots.first() {
            tabs.push(PropTab::Transform);
            if matches!(first.source, Some(NodeSource::LibraryAsset { .. })) {
                tabs.push(PropTab::Asset);
            }
            if matches!(first.source, Some(NodeSource::Camera { .. })) && slots.len() == 1 {
                tabs.push(PropTab::Lens);
            }
            if first.kind != NodeKind::Group && first.kind != NodeKind::Camera {
                tabs.push(PropTab::Material);
            }
            tabs.push(PropTab::Organize);
        }
        tabs.push(PropTab::Scene);
        let (tab, mut content) = self.tabbed_with(ui, TabSet::Cenario, &tabs);
        let ui = &mut content;
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("scene_details").auto_shrink([false, false]).show(ui, |ui| self.details_tab(ui, tab, &project, &slots));
    }

    fn details_tab(&mut self, ui: &mut egui::Ui, tab: PropTab, project: &anigo_core::ProjectState, slots: &[&anigo_core::NodeSlot]) {
        if slots.is_empty() || tab == PropTab::Scene {
            self.scene_settings_panel(ui, project);
            return;
        }
        let project = project.clone();
        let editable: Vec<NodeId> = slots.iter().filter(|s| !s.locked).map(|s| s.node_id.clone()).collect();
        let first = slots[0];
        // ---- nome + visível/bloqueado
        ui.horizontal(|ui| {
            if slots.len() == 1 {
                let mut name = first.name.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width() - 96.0));
                if r.changed() {
                    self.scene_ui.rename = None;
                }
                if r.lost_focus() && name.trim() != first.name && !name.trim().is_empty() {
                    self.scene_rename(&first.node_id, &name);
                } else if r.changed() {
                    // edição em andamento: guarda no buffer do próprio campo (egui mantém o texto)
                }
            } else {
                ui.label(RichText::new(format!("{} {}", slots.len(), tr("scene.details.objects"))).strong());
            }
            let all_visible = slots.iter().all(|s| s.visible);
            let all_locked = slots.iter().all(|s| s.locked);
            if ui.selectable_label(!all_visible, RichText::new(if all_visible { icons::EYE } else { icons::EYE_SLASH }).size(15.0)).on_hover_text(tr("scene.details.visible")).clicked() {
                let ids: Vec<NodeId> = slots.iter().map(|s| s.node_id.clone()).collect();
                if let Ok(c) = anigo_core::level::plan_set_visible(&project, &ids, !all_visible) {
                    self.scene_apply(c);
                }
            }
            if ui.selectable_label(all_locked, RichText::new(if all_locked { icons::LOCK } else { icons::LOCK_OPEN }).size(15.0)).on_hover_text(tr("scene.details.locked")).clicked() {
                let ids: Vec<NodeId> = slots.iter().map(|s| s.node_id.clone()).collect();
                self.scene_set_locked(&ids, !all_locked);
            }
        });
        ui.label(RichText::new(format!("{}  \u{b7}  {}", first.kind.as_str(), crate::scene::outliner::source_label(first))).size(11.0).color(theme::TEXT_DIM));
        if editable.is_empty() {
            ui.label(RichText::new(tr("scene.details.all_locked")).color(Color32::from_rgb(0xff, 0xb0, 0x50)));
        }

        // ---- transform
        if tab == PropTab::Transform {
            self.transform_section(ui, &project, &editable, slots);
        }

        // ---- ativo / origem
        if let (PropTab::Asset, Some(NodeSource::LibraryAsset { uri, revision })) = (tab, &first.source) {
            egui::CollapsingHeader::new(RichText::new(tr("scene.details.asset")).strong()).default_open(true).show(ui, |ui| {
                let resolved = self.scene_ui.cache.resolved(uri).cloned();
                match resolved {
                    Some(Ok(r)) => {
                        ui.label(format!("{}", r.name));
                        ui.label(RichText::new(format!("{} {}  \u{b7}  {}  \u{b7}  {}", tr("scene.details.revision"), revision, r.format.to_uppercase(), format!("{:.1} MB", r.size as f64 / 1_048_576.0))).size(11.5).color(theme::TEXT_DIM));
                        if r.revision != *revision {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{} {} {}", icons::WARNING, tr("scene.details.stale"), r.revision)).color(Color32::from_rgb(0xff, 0xb0, 0x50)));
                                if ui.button(tr("scene.details.update")).clicked() {
                                    let ids: Vec<NodeId> = slots.iter().map(|s| s.node_id.clone()).collect();
                                    self.scene_update_stale(&ids);
                                }
                            });
                        }
                        match self.scene_ui.cache.slot(&r.key) {
                            Some(Slot::Failed(e)) => {
                                ui.label(RichText::new(format!("{} {e}", icons::WARNING)).color(Color32::from_rgb(0xff, 0x80, 0x70)));
                            }
                            Some(Slot::Loading) => {
                                ui.label(RichText::new(format!("{}…", tr("scene.loading"))).color(theme::TEXT_DIM));
                            }
                            Some(Slot::Ready(g)) => {
                                ui.label(RichText::new(format!("{} {}", g.tris, tr("scene.details.tris"))).size(11.5).color(theme::TEXT_DIM));
                            }
                            None => {}
                        }
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(format!("{} {e}", icons::WARNING)).color(Color32::from_rgb(0xff, 0x80, 0x70)));
                    }
                    None => {
                        ui.label(RichText::new(uri).size(11.0).color(theme::TEXT_DIM));
                    }
                }
                if ui.button(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("scene.details.replace"))).on_hover_text(tr("scene.details.replace_tip")).clicked() {
                    self.scene_replace_with_library_selection();
                }
            });
        }

        // ---- câmera de cena: lente
        if let (PropTab::Lens, Some(NodeSource::Camera { focal_mm, sensor_width_mm, sensor_height_mm })) = (tab, first.source.clone()) {
            if slots.len() == 1 {
                egui::CollapsingHeader::new(RichText::new(tr("scene.details.lens")).strong()).default_open(true).show(ui, |ui| {
                    self.lens_section(ui, first.node_id.clone(), focal_mm, sensor_width_mm, sensor_height_mm);
                });
            }
        }

        // ---- material (overrides)
        if tab == PropTab::Material && first.kind != NodeKind::Group && first.kind != NodeKind::Camera {
            self.material_section(ui, &editable, slots);
        }

        // ---- organização
        if tab == PropTab::Organize {
            self.organize_section(ui, &editable, slots);
        }
    }

    fn transform_section(&mut self, ui: &mut egui::Ui, project: &anigo_core::ProjectState, editable: &[NodeId], slots: &[&anigo_core::NodeSlot]) {
        let disabled = editable.is_empty();
        let tvals: Vec<[f32; 3]> = slots.iter().map(|s| s.transform.translation.to_array()).collect();
        let rvals: Vec<[f32; 3]> = slots.iter().map(|s| euler_deg(s.transform.rotation)).collect();
        let svals: Vec<[f32; 3]> = slots.iter().map(|s| s.transform.scale.to_array()).collect();
        let mut edit: Option<Command> = None;
        let mut reset: Option<Command> = None;
        let unit = project.settings.units;
        let (mul, suffix) = match unit {
            anigo_core::project::LengthUnit::Meters => (1.0, " m"),
            anigo_core::project::LengthUnit::Centimeters => (100.0, " cm"),
        };
        ui.add_enabled_ui(!disabled, |ui| {
            // linhas: rótulo, três campos, reset
            let mut row = |ui: &mut egui::Ui, label: String, vals: &[[f32; 3]], scale: f32, speed: f64, suffix: &str, default: [f32; 3], channel: u8| {
                ui.horizontal(|ui| {
                    ui.add_sized([62.0, 18.0], egui::Label::new(label));
                    let mut changed: Option<(usize, f32)> = None;
                    for c in 0..3 {
                        let first = vals[0][c] * scale;
                        let mixed = vals.iter().any(|v| (v[c] - vals[0][c]).abs() > 1e-5);
                        let mut v = first;
                        let color = AXIS_TEXT[c];
                        let mut dv = egui::DragValue::new(&mut v).speed(speed).max_decimals(3).suffix(suffix);
                        if mixed {
                            dv = dv.custom_formatter(|_, _| "\u{2014}".to_string()).custom_parser(|s| s.trim().replace(',', ".").parse::<f64>().ok());
                        }
                        let r = ui.add_sized([64.0, 18.0], dv).on_hover_text(["X", "Y", "Z"][c]);
                        ui.painter().rect_filled(Rect::from_min_size(r.rect.left_top(), egui::vec2(2.0, r.rect.height())), 0.0, color);
                        if r.changed() {
                            changed = Some((c, v / scale));
                        }
                    }
                    let differs = vals.iter().any(|v| v.iter().zip(default).any(|(a, b)| (a - b).abs() > 1e-5));
                    if ui.add_enabled(differs, egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).size(12.0)).small()).on_hover_text(tr("char.reset_slider")).clicked() {
                        reset = Some(Self::transform_command(editable, channel, |_| default));
                    }
                    if let Some((c, nv)) = changed {
                        edit = Some(Self::transform_edit(project, editable, channel, c, nv, self.scene_ui.scale_lock));
                    }
                });
            };
            row(ui, tr("scene.details.location").to_string(), &tvals, mul, 0.02 * mul as f64, suffix, [0.0; 3], 0);
            row(ui, tr("scene.details.rotation").to_string(), &rvals, 1.0, 0.5, "\u{b0}", [0.0; 3], 1);
            row(ui, tr("scene.details.scale").to_string(), &svals, 1.0, 0.01, "", [1.0; 3], 2);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.scene_ui.scale_lock, tr("scene.details.scale_lock")).on_hover_text(tr("scene.details.scale_lock_tip"));
            ui.label(RichText::new(tr("scene.details.relative")).size(11.0).color(theme::TEXT_DIM));
        });
        if let Some(a) = slots.first().and_then(|s| s.parent_id.as_ref()) {
            if let Some(p) = project.scene.node(a) {
                ui.label(RichText::new(format!("{} {}", tr("scene.details.parent"), p.name)).size(11.0).color(theme::TEXT_DIM));
            }
        }
        if let Some(c) = reset {
            self.scene_apply(c);
        } else if let Some(c) = edit {
            self.scene_edit(c);
        }
    }

    /// `SetNodesTransform` que fixa o canal (0 = posição, 1 = rotação em graus, 2 = escala) de todos os nós.
    fn transform_command(ids: &[NodeId], channel: u8, f: impl Fn(usize) -> [f32; 3]) -> Command {
        Command::SetNodesTransform {
            entries: ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let v = f(i);
                    NodeTransformEntry {
                        node_id: id.clone(),
                        translation: (channel == 0).then_some(v),
                        rotation: (channel == 1).then(|| quat_from_deg(v).to_array()),
                        scale: (channel == 2).then_some(v),
                    }
                })
                .collect(),
        }
    }

    /// Edição de UM componente: cada nó mantém os outros dois (a escala trava proporção se pedido).
    fn transform_edit(project: &anigo_core::ProjectState, ids: &[NodeId], channel: u8, comp: usize, value: f32, lock: bool) -> Command {
        let entries = ids
            .iter()
            .filter_map(|id| {
                let t = project.scene.node(id)?.transform;
                let mut e = NodeTransformEntry { node_id: id.clone(), translation: None, rotation: None, scale: None };
                match channel {
                    0 => {
                        let mut v = t.translation.to_array();
                        v[comp] = value;
                        e.translation = Some(v);
                    }
                    1 => {
                        let mut v = euler_deg(t.rotation);
                        v[comp] = value;
                        e.rotation = Some(quat_from_deg(v).to_array());
                    }
                    _ => {
                        let mut v = t.scale.to_array();
                        if lock && v[comp].abs() > 1e-6 {
                            let k = value / v[comp];
                            v = [v[0] * k, v[1] * k, v[2] * k];
                        } else {
                            v[comp] = value;
                        }
                        e.scale = Some(v.map(|x| if x.abs() < 1e-4 { 1e-4 } else { x }));
                    }
                }
                Some(e)
            })
            .collect();
        Command::SetNodesTransform { entries }
    }

    fn lens_section(&mut self, ui: &mut egui::Ui, id: NodeId, focal: f32, sw: f32, sh: f32) {
        let mut f = focal;
        let mut cmd: Option<Command> = None;
        let mk = |focal_mm: f32, w: f32, h: f32| Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { source: Some(NodeSource::Camera { focal_mm, sensor_width_mm: w, sensor_height_mm: h }), ..Default::default() } };
        ui.horizontal(|ui| {
            ui.label(tr("scene.lens.focal"));
            if ui.add(egui::Slider::new(&mut f, 8.0..=300.0).logarithmic(true).suffix(" mm").max_decimals(1)).changed() {
                cmd = Some(mk(f, sw, sh));
            }
        });
        ui.horizontal_wrapped(|ui| {
            for p in [18.0f32, 24.0, 35.0, 50.0, 85.0, 135.0] {
                if ui.selectable_label((focal - p).abs() < 0.01, format!("{p}")).clicked() {
                    cmd = Some(mk(p, sw, sh));
                }
            }
        });
        let current = SENSORS.iter().find(|s| (s.1 - sw).abs() < 0.01 && (s.2 - sh).abs() < 0.01).map(|s| s.0).unwrap_or("custom");
        egui::ComboBox::from_id_salt("scene_sensor").selected_text(current).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
            for (name, w, h) in SENSORS {
                if ui.selectable_label((sw - w).abs() < 0.01 && (sh - h).abs() < 0.01, name).clicked() {
                    cmd = Some(mk(focal, w, h));
                }
            }
        });
        let fov_v = vertical_fov(focal, sh).to_degrees();
        let fov_h = (2.0 * (sw / (2.0 * focal)).atan()).to_degrees();
        ui.label(RichText::new(format!("FOV  {:.1}\u{b0} V  \u{b7}  {:.1}\u{b0} H   (f = {:.1} mm)", fov_v, fov_h, focal_for_fov(vertical_fov(focal, sh), sh))).size(11.5).color(theme::TEXT_DIM));
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::VIDEO_CAMERA, tr("scene.lens.use_as_render"))).on_hover_text(tr("scene.lens.use_as_render_tip")).clicked() {
                self.scene_use_as_render_camera(&id);
            }
            if ui.button(format!("{}  {}", icons::CROSSHAIR, tr("scene.lens.align_to_view"))).on_hover_text(tr("scene.lens.align_tip")).clicked() {
                self.scene_align_node_to_view(&id);
            }
        });
        if let Some(c) = cmd {
            self.scene_edit(c);
        }
    }

    fn material_section(&mut self, ui: &mut egui::Ui, editable: &[NodeId], slots: &[&anigo_core::NodeSlot]) {
        let first = slots[0];
        // material completo editado no workspace Shading: os 3 campos básicos abaixo seriam sobrescritos pelo patch
        if first.overrides.material.is_some() {
            ui.label(RichText::new(tr("shading.scene_note")).size(11.5).color(theme::TEXT_DIM));
            if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("scene.material.reset"))).clicked() {
                let cmds: Vec<Command> = editable.iter().map(|id| Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { overrides: Some(NodeOverrides::default()), ..Default::default() } }).collect();
                if let Some(c) = cmds.into_iter().next() {
                    self.scene_apply(c);
                }
            }
            return;
        }
        let mut ov: NodeOverrides = first.overrides.clone();
        let mut changed = false;
        let mut clear = false;
        let base_default = [0.80, 0.81, 0.86, 1.0];
        let shade_default = [0.50, 0.52, 0.66, 1.0];
        ui.horizontal(|ui| {
            ui.label(tr("scene.material.base"));
            let mut c = ov.base_color.unwrap_or(base_default);
            if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                ov.base_color = Some(c);
                changed = true;
            }
            ui.label(tr("scene.material.shade"));
            let mut c = ov.shade_color.unwrap_or(shade_default);
            if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                ov.shade_color = Some(c);
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("scene.material.outline"));
            let mut w = ov.outline_width.unwrap_or(StylizedWidth::DEFAULT);
            if ui.add(egui::DragValue::new(&mut w).speed(0.0002).range(0.0..=0.05).max_decimals(4)).changed() {
                ov.outline_width = Some(w);
                changed = true;
            }
            if ui.add_enabled(!first.overrides.is_default(), egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE).small()).on_hover_text(tr("scene.material.reset")).clicked() {
                clear = true;
            }
        });
        if clear || changed {
            let value = if clear { NodeOverrides::default() } else { ov };
            let cmds: Vec<Command> = editable.iter().map(|id| Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { overrides: Some(value.clone()), ..Default::default() } }).collect();
            if !cmds.is_empty() {
                let cmd = if cmds.len() == 1 { cmds.into_iter().next().unwrap_or(Command::Batch { commands: vec![] }) } else { Command::Batch { commands: cmds } };
                if clear {
                    self.scene_apply(cmd);
                } else {
                    self.scene_edit(cmd);
                }
            }
        }
    }

    fn organize_section(&mut self, ui: &mut egui::Ui, editable: &[NodeId], slots: &[&anigo_core::NodeSlot]) {
        let first = slots[0];
        // tags
        ui.label(RichText::new(tr("scene.details.tags")).size(11.5).color(theme::TEXT_DIM));
        let mut remove: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            for t in &first.tags {
                if ui.small_button(format!("{t}  {}", icons::X)).clicked() {
                    remove = Some(t.clone());
                }
            }
        });
        let r = ui.add(egui::TextEdit::singleline(&mut self.scene_ui.tag_input).hint_text(tr("scene.details.add_tag")).desired_width(ui.available_width() - 8.0));
        let mut add: Option<String> = None;
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let t = self.scene_ui.tag_input.trim().to_string();
            if !t.is_empty() {
                add = Some(t);
            }
            self.scene_ui.tag_input.clear();
        }
        if add.is_some() || remove.is_some() {
            let cmds: Vec<Command> = editable
                .iter()
                .filter_map(|id| {
                    let mut tags = self.core.project().scene.node(id)?.tags.clone();
                    if let Some(t) = &add {
                        if !tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                            tags.push(t.clone());
                        }
                    }
                    if let Some(t) = &remove {
                        tags.retain(|x| x != t);
                    }
                    Some(Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { tags: Some(tags), ..Default::default() } })
                })
                .collect();
            if !cmds.is_empty() {
                let cmd = if cmds.len() == 1 { cmds.into_iter().next().unwrap_or(Command::Batch { commands: vec![] }) } else { Command::Batch { commands: cmds } };
                self.scene_apply(cmd);
            }
        }
        // pasta
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("scene.details.folder")).size(11.5).color(theme::TEXT_DIM));
            let mut folder = first.folder.clone().unwrap_or_default();
            let r = ui.add(egui::TextEdit::singleline(&mut folder).desired_width(ui.available_width() - 8.0).hint_text("Props/Mesas"));
            if r.lost_focus() && folder.trim() != first.folder.clone().unwrap_or_default() {
                let ids: Vec<NodeId> = editable.to_vec();
                self.scene_move_to_folder(&ids, folder.trim().trim_matches('/'));
            }
        });
    }

    /// Painel "Cena" (nada selecionado): luz principal estilizada, câmera de render e unidades.
    fn scene_settings_panel(&mut self, ui: &mut egui::Ui, project: &anigo_core::ProjectState) {
        ui.label(RichText::new(tr("scene.settings")).strong());
        ui.label(RichText::new(tr("scene.settings.hint")).size(11.0).color(theme::TEXT_DIM));
        let Some(slot) = project.scene.lights.first() else { return };
        let light = &slot.light;
        let d = Vec3::from_array(light.direction).normalize_or_zero();
        let mut az = d.x.atan2(d.z).to_degrees();
        let mut el = d.y.clamp(-1.0, 1.0).asin().to_degrees();
        let mut intensity = light.intensity;
        let mut ambient = light.ambient_intensity;
        let mut color = light.color;
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::SUN, tr("scene.light.main"))).strong()).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("scene.light.azimuth"));
                changed |= ui.add(egui::DragValue::new(&mut az).speed(0.5).suffix("\u{b0}").range(-180.0..=180.0)).changed();
                ui.label(tr("scene.light.elevation"));
                changed |= ui.add(egui::DragValue::new(&mut el).speed(0.5).suffix("\u{b0}").range(-89.0..=89.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr("scene.light.color"));
                changed |= ui.color_edit_button_rgb(&mut color).changed();
                ui.label(tr("scene.light.intensity"));
                changed |= ui.add(egui::DragValue::new(&mut intensity).speed(0.01).range(0.0..=8.0).max_decimals(2)).changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr("scene.light.ambient"));
                changed |= ui.add(egui::DragValue::new(&mut ambient).speed(0.01).range(0.0..=2.0).max_decimals(2)).changed();
            });
        });
        if changed {
            let (a, e) = (az.to_radians(), el.to_radians());
            let dir = [e.cos() * a.sin(), e.sin(), e.cos() * a.cos()];
            self.scene_edit(Command::SetLight {
                light_id: Some(slot.light_id.clone()),
                direction: Some(dir),
                color: Some(color),
                intensity: Some(intensity),
                shadow_color: None,
                ambient_intensity: Some(ambient),
                shadow_saturation: None,
                ambient_sky: None,
                ambient_ground: None,
                shadow: None,
            });
        }
        let c = &project.scene.camera.camera;
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::VIDEO_CAMERA, tr("scene.camera.render"))).strong()).default_open(true).show(ui, |ui| {
            ui.label(RichText::new(format!("FOV {:.1}\u{b0}   {:.2}:1", c.fov_y.to_degrees(), c.aspect)).size(12.0));
            ui.label(RichText::new(format!("eye {}  \u{2192}  {}", super::fmt_vec3(c.eye), super::fmt_vec3(c.target))).size(11.0).color(theme::TEXT_DIM));
            if ui.button(format!("{}  {}", icons::VIDEO_CAMERA, tr("scene.view.through_camera"))).clicked() {
                self.scene_ui.camera_view = true;
            }
            if ui.button(format!("{}  {}", icons::CROSSHAIR, tr("scene.camera.from_view"))).on_hover_text(tr("scene.camera.from_view_tip")).clicked() {
                let cam = self.scene_ui.camera;
                self.scene_apply(Command::SetCamera { eye: Some(cam.eye().to_array()), target: Some(cam.target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: Some(cam.fov_y.to_degrees()), projection: None });
            }
        });
        ui.label(RichText::new(format!("{}: {}", tr("scene.settings.units"), match project.settings.units {
            anigo_core::project::LengthUnit::Meters => tr("scene.settings.meters"),
            anigo_core::project::LengthUnit::Centimeters => tr("scene.settings.centimeters"),
        })).size(11.5).color(theme::TEXT_DIM));
        ui.label(RichText::new(format!("{} {}   {} {}", project.scene.nodes.len(), tr("scene.settings.objects"), self.scene_ui.runtime.bvh.instances.len(), tr("scene.settings.pickable"))).size(11.5).color(theme::TEXT_DIM));
    }

    // ============================================================ Salvar / Abrir

    fn scene_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.scene_ui.dialog.take() else { return };
        let mut keep = true;
        let mut confirm = false;
        egui::Window::new(if dlg.save { tr("scene.dialog.save") } else { tr("scene.dialog.open") })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(tr("char.dialog.path"));
                let resp = ui.add(egui::TextEdit::singleline(&mut dlg.path).desired_width(f32::INFINITY));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
                ui.label(RichText::new(tr("char.dialog.hint")).size(11.5).color(theme::TEXT_DIM));
                if let Some(e) = &dlg.error {
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x70, 0x70)));
                }
                ui.horizontal(|ui| {
                    if ui.button(if dlg.save { tr("char.save") } else { tr("char.open") }).clicked() {
                        confirm = true;
                    }
                    if ui.button(tr("char.dialog.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
        if confirm {
            let r = persist::normalize_path(&dlg.path).and_then(|path| {
                if dlg.save {
                    persist::save_project(&self.core, &path)?;
                    self.scene_ui.message = format!("{}: {}", tr("char.saved"), path.display());
                } else {
                    self.core.cancel_preview();
                    persist::load_project(&mut self.core, &path)?;
                    self.core.set_history_limit(400);
                    self.scene_ui.selection.clear();
                    self.scene_ui.session = None;
                    self.scene_ui.cache.forget_resolutions();
                    self.scene_ui.runtime.key = None;
                    self.scene_ui.framed = false;
                    self.char_ui.framed = false;
                    self.scene_ui.message = format!("{}: {}", tr("char.opened"), path.display());
                }
                self.scene_ui.last_path = path.display().to_string();
                Ok(())
            });
            match r {
                Ok(()) => keep = false,
                Err(e) => dlg.error = Some(e),
            }
        }
        if keep {
            self.scene_ui.dialog = Some(dlg);
        }
    }
}

const AXIS_TEXT: [Color32; 3] = [Color32::from_rgb(0xe8, 0x4a, 0x4a), Color32::from_rgb(0x6a, 0xcc, 0x4a), Color32::from_rgb(0x4a, 0x84, 0xf0)];

/// Largura de contorno padrão do material (espelha `StylizedMaterial::default`).
struct StylizedWidth;
impl StylizedWidth {
    const DEFAULT: f32 = 0.0035;
}
