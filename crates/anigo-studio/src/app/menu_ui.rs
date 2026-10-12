//! Barra de menus (Arquivo / Editar / Exibir / Janela / Ajuda), diálogos dos menus (novo projeto, salvar layout, importar/exportar,
//! Sobre) e a janela de atalhos de teclado configuráveis (T-2.12). Todo item chama uma ação REAL já existente; o que não pode
//! rodar agora fica desabilitado com a explicação na dica (nada de item morto).

use super::dock_ui::{DockCmd, DockDialog};
use super::StudioApp;
use crate::i18n::{self, tr, Lang};
use crate::icons;
use crate::keymap::actions::{Area, ACTIONS};
use crate::keymap::{Chord, Keymap};
use crate::layout::registry::{self, PanelKind};
use crate::theme;
use crate::workspace::WorkspaceId;
use egui::{Key, RichText};
use std::sync::Arc;

/// Estado da janela de atalhos.
#[derive(Default)]
pub(crate) struct KeyboardUi {
    pub search: String,
    pub only_ws: bool,
    /// `(ação, índice do atalho a trocar)`: esperando a próxima combinação; `None` no índice = acrescentar.
    pub capture: Option<(&'static str, Option<usize>)>,
    pub path: String,
    pub export_all: bool,
    pub message: String,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub(crate) struct MenuState {
    pub new_confirm: bool,
    pub about: bool,
    pub keyboard: bool,
    pub kb: KeyboardUi,
    /// Cenários de captura: abre este menu (`"file"`, `"edit"`, `"view"`, `"window"`, `"help"`) sem mouse.
    pub force_open: Option<&'static str>,
    /// Último erro do carregamento de atalhos (para o "Sobre").
    pub keymap_warnings: Vec<String>,
}

fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, hint: Option<&str>) -> bool {
    let mut b = egui::Button::new(label);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    let mut r = ui.add_enabled(enabled, b);
    if let (false, Some(h)) = (enabled, hint) {
        r = r.on_disabled_hover_text(h);
    }
    let clicked = r.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl StudioApp {
    /// Atalho principal da ação para exibir nos menus (`""` se desligada).
    pub(crate) fn km_label(&self, id: &str) -> String {
        self.keymap.label(id)
    }

    /// Consulta o keymap em vigor: a ação disparou neste quadro? (consome a tecla)
    pub(crate) fn hk(&self, ctx: &egui::Context, id: &str) -> bool {
        self.keymap.consume(ctx, id)
    }

    pub(crate) fn keymap_save(&mut self) {
        if let Some(p) = self.keymap_path.clone() {
            if let Err(e) = self.keymap.save_to(&p, false) {
                tracing::warn!("falha ao salvar atalhos em {}: {e}", p.display());
                self.menu.kb.message = format!("{}: {e}", tr("kb.error"));
            }
        }
    }

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menubar").exact_size(24.0).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.menu_file(ui);
                self.menu_edit(ui);
                self.menu_view(ui);
                self.menu_window(ui);
                self.menu_help(ui);
            });
        });
        let ctx = ui.ctx().clone();
        self.menu_dialogs(&ctx);
        self.keyboard_window(&ctx);
    }

    /// Abre o menu `key` por código (só para os cenários de captura) e devolve o `Response` do botão para ancorar.
    fn menu_button<R>(&mut self, ui: &mut egui::Ui, key: &'static str, title: &str, add: impl FnOnce(&mut StudioApp, &mut egui::Ui) -> R) {
        let r = ui.menu_button(title, |ui| add(self, ui));
        if self.menu.force_open == Some(key) {
            let id = egui::Popup::default_response_id(&r.response);
            if !egui::Popup::is_id_open(ui.ctx(), id) {
                egui::Popup::open_id(ui.ctx(), id);
            }
        }
    }

    // ------------------------------------------------------------------------------------ cenários de captura

    /// Cenários `layout-*` (capturas determinísticas): menus abertos, janela de atalhos, "Sobre". O workspace vem de `--workspace`
    /// (ou do nome: `layout-<workspace>` só abre o workspace com o layout padrão/carregado).
    pub(crate) fn run_layout_scenario(&mut self, name: &str) {
        match name {
            "layout-menu-file" => self.menu.force_open = Some("file"),
            "layout-menu-edit" => self.menu.force_open = Some("edit"),
            "layout-menu-view" => self.menu.force_open = Some("view"),
            "layout-menu-window" => self.menu.force_open = Some("window"),
            "layout-menu-help" => self.menu.force_open = Some("help"),
            "layout-keyboard" => {
                self.menu.keyboard = true;
                self.menu.kb.only_ws = true;
            }
            // um conflito de propósito (escalar = W, como mover) para mostrar o aviso
            "layout-keyboard-conflict" => {
                let w = Chord::parse("W").expect("W é uma tecla válida");
                Arc::make_mut(&mut self.keymap).set("cenario.tool.scale", vec![w]);
                self.menu.keyboard = true;
                self.menu.kb.only_ws = true;
            }
            "layout-about" => self.menu.about = true,
            _ => {}
        }
    }

    /// Workspace implícito de um cenário `layout-<workspace>`.
    pub(crate) fn layout_scenario_workspace(name: &str) -> Option<WorkspaceId> {
        name.strip_prefix("layout-").and_then(WorkspaceId::from_key)
    }

    // ------------------------------------------------------------------------------------ Arquivo

    fn menu_file(&mut self, ui: &mut egui::Ui) {
        self.menu_button(ui, "file", tr("menu.file"), |app, ui| {
            if item(ui, tr("menu.file.new"), "", true, None) {
                app.menu.new_confirm = true;
            }
            if item(ui, tr("menu.file.open"), "", true, None) {
                app.file_open_dialog(false);
            }
            let has_path = !app.file_last_path().trim().is_empty();
            if item(ui, tr("menu.file.save"), "", has_path, Some(tr("menu.file.save_hint"))) {
                app.file_save_quick();
            }
            if item(ui, tr("menu.file.save_as"), "", true, None) {
                app.file_open_dialog(true);
            }
            ui.separator();
            let can_export = app.ws == WorkspaceId::Modelagem && app.modeling.active_object().is_some();
            if item(ui, tr("menu.file.export_lib"), "", can_export, Some(tr("menu.file.export_hint"))) {
                app.export_active_to_library();
            }
            let can_render = app.ws == WorkspaceId::Render;
            if item(ui, tr("menu.file.render"), "", can_render, Some(tr("menu.file.render_hint"))) {
                let ctx = ui.ctx().clone();
                app.render_enqueue(&ctx);
            }
        });
    }

    // ------------------------------------------------------------------------------------ Editar

    fn menu_edit(&mut self, ui: &mut egui::Ui) {
        self.menu_button(ui, "edit", tr("menu.edit"), |app, ui| {
            let (can_undo, can_redo) = app.can_undo_redo();
            if item(ui, tr("act.edit.undo"), &app.km_label("edit.undo"), can_undo, Some(tr("menu.edit.nothing"))) {
                app.undo_redo(true);
            }
            if item(ui, tr("act.edit.redo"), &app.km_label("edit.redo"), can_redo, Some(tr("menu.edit.nothing_redo"))) {
                app.undo_redo(false);
            }
            ui.separator();
            ui.menu_button(tr("menu.edit.history"), |ui| {
                let labels: Vec<String> = if app.ws == WorkspaceId::Modelagem {
                    app.modeling.history_labels().iter().rev().take(12).map(|s| s.to_string()).collect()
                } else {
                    app.core.history().undo_descriptions().iter().rev().take(12).map(|d| super::scene_ui::describe_scene(d)).collect()
                };
                if labels.is_empty() {
                    ui.add_enabled(false, egui::Button::new(tr("menu.edit.history_empty")));
                }
                for l in labels {
                    ui.add_enabled(false, egui::Button::new(l));
                }
                ui.separator();
                ui.label(RichText::new(tr("menu.edit.history_hint")).size(11.0).color(theme::TEXT_DIM));
            });
        });
    }

    // ------------------------------------------------------------------------------------ Exibir

    fn menu_view(&mut self, ui: &mut egui::Ui) {
        self.menu_button(ui, "view", tr("menu.view"), |app, ui| {
            ui.menu_button(tr("menu.view.workspace"), |ui| {
                for w in WorkspaceId::ALL {
                    let label = format!("{}  {}", w.icon(), tr(&format!("workspace.{}", w.key())));
                    if ui.selectable_label(app.ws == w, label).clicked() {
                        if app.ws != w {
                            app.switch_workspace(w);
                        }
                        ui.close();
                    }
                }
            });
            let in_modeling = app.ws == WorkspaceId::Modelagem;
            ui.add_enabled_ui(in_modeling, |ui| {
                ui.menu_button(tr("menu.view.shading"), |ui| {
                    for s in [super::ViewShading::Solid, super::ViewShading::Wireframe, super::ViewShading::Cel] {
                        if ui.selectable_label(app.view_shading == s, super::shading_label(s)).clicked() {
                            app.view_shading = s;
                            ui.close();
                        }
                    }
                });
            })
            .response
            .on_disabled_hover_text(tr("menu.view.shading_hint"));
            let (can_frame, shortcut) = match app.ws {
                WorkspaceId::Modelagem => (app.modeling.active_object().is_some(), app.km_label("modelagem.frame")),
                WorkspaceId::Cenario => (true, app.km_label("cenario.focus")),
                _ => (false, String::new()),
            };
            if item(ui, tr("menu.view.frame"), &shortcut, can_frame, Some(tr("menu.view.frame_hint"))) {
                match app.ws {
                    WorkspaceId::Modelagem => app.frame_selected_or_all(),
                    WorkspaceId::Cenario => app.scene_focus(),
                    _ => {}
                }
            }
            ui.separator();
            ui.menu_button(tr("menu.view.language"), |ui| {
                for (l, name) in [(Lang::PtBr, "Português (BR)"), (Lang::En, "English")] {
                    if ui.selectable_label(app.lang() == l, name).clicked() {
                        i18n::set_lang(l);
                        ui.close();
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------------------------------ Janela

    fn menu_window(&mut self, ui: &mut egui::Ui) {
        self.menu_button(ui, "window", tr("menu.window"), |app, ui| {
            let ws = app.ws;
            ui.menu_button(tr("menu.window.panels"), |ui| {
                let panels: Vec<_> = registry::panels(ws).iter().filter(|p| p.kind == PanelKind::Panel).collect();
                if panels.is_empty() {
                    ui.label(RichText::new(tr("menu.window.panels_none")).color(theme::TEXT_DIM));
                }
                for p in panels {
                    let avail = app.dock_flag(p.id).is_none_or(|(_, a)| a);
                    let mut open = app.dock_panel_open(ws, p.id);
                    let r = ui.add_enabled(avail, egui::Checkbox::new(&mut open, format!("{}  {}", p.icon, tr(p.title_key))));
                    if !avail {
                        r.on_disabled_hover_text(tr("menu.window.panel_unavailable"));
                    } else if r.changed() {
                        app.dock.cmds.push(DockCmd::Toggle(p.id));
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("menu.window.layouts"), |ui| {
                if item(ui, tr("menu.window.reset"), "", true, None) {
                    app.dock.cmds.push(DockCmd::ResetCurrent);
                }
                if item(ui, tr("menu.window.reset_all"), "", true, None) {
                    app.dock.cmds.push(DockCmd::ResetAll);
                }
                ui.separator();
                if item(ui, tr("menu.window.save_preset"), "", true, None) {
                    app.dock.dialog = Some(DockDialog::SavePreset(String::new()));
                }
                ui.menu_button(tr("menu.window.presets"), |ui| {
                    let names: Vec<String> = app.dock.store.presets_for(ws.key()).map(|p| p.name.clone()).collect();
                    if names.is_empty() {
                        ui.label(RichText::new(tr("menu.window.presets_none")).color(theme::TEXT_DIM));
                    }
                    for n in names {
                        ui.horizontal(|ui| {
                            if ui.button(&n).clicked() {
                                app.dock.cmds.push(DockCmd::ApplyPreset(n.clone()));
                                ui.close();
                            }
                            if ui.small_button(icons::TRASH).on_hover_text(tr("menu.window.preset_delete")).clicked() {
                                app.dock.cmds.push(DockCmd::DeletePreset(n.clone()));
                                ui.close();
                            }
                        });
                    }
                });
                ui.separator();
                if item(ui, tr("menu.window.export_layout"), "", true, None) {
                    app.dock.dialog = Some(DockDialog::Export(crate::layout::config_dir().join("layouts_export.json").display().to_string()));
                }
                if item(ui, tr("menu.window.import_layout"), "", true, None) {
                    app.dock.dialog = Some(DockDialog::Import(crate::layout::config_dir().join("layouts_export.json").display().to_string()));
                }
            });
            ui.separator();
            if item(ui, tr("menu.window.keyboard"), "", true, None) {
                app.menu.keyboard = true;
            }
        });
    }

    // ------------------------------------------------------------------------------------ Ajuda

    fn menu_help(&mut self, ui: &mut egui::Ui) {
        self.menu_button(ui, "help", tr("menu.help"), |app, ui| {
            if item(ui, tr("menu.help.shortcuts"), "", true, None) {
                app.menu.keyboard = true;
                app.menu.kb.only_ws = true;
            }
            if item(ui, tr("menu.help.about"), "", true, None) {
                app.menu.about = true;
            }
        });
    }

    // ------------------------------------------------------------------------------------ diálogos

    fn menu_dialogs(&mut self, ctx: &egui::Context) {
        // novo projeto (confirmação)
        if self.menu.new_confirm {
            let mut open = true;
            egui::Window::new(tr("menu.file.new_title")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(tr("menu.file.new_warn"));
                ui.horizontal(|ui| {
                    if ui.button(tr("menu.file.new_ok")).clicked() {
                        self.file_new_project();
                        self.menu.new_confirm = false;
                    }
                    if ui.button(tr("menu.cancel")).clicked() {
                        self.menu.new_confirm = false;
                    }
                });
            });
            if !open {
                self.menu.new_confirm = false;
            }
        }
        // salvar layout / exportar / importar (nome ou caminho)
        if let Some(mut dlg) = self.dock.dialog.take() {
            let (title, label, hint) = match &dlg {
                DockDialog::SavePreset(_) => (tr("menu.window.save_preset"), tr("menu.window.preset_name"), ""),
                DockDialog::Export(_) => (tr("menu.window.export_layout"), tr("menu.window.file_path"), ""),
                DockDialog::Import(_) => (tr("menu.window.import_layout"), tr("menu.window.file_path"), ""),
            };
            let (mut keep, mut ok) = (true, false);
            egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(label);
                let text = match &mut dlg {
                    DockDialog::SavePreset(t) | DockDialog::Export(t) | DockDialog::Import(t) => t,
                };
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).hint_text(hint));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    ok = true;
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("menu.ok")).clicked() {
                        ok = true;
                    }
                    if ui.button(tr("menu.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
            if ok {
                match dlg {
                    DockDialog::SavePreset(n) if !n.trim().is_empty() => self.dock.cmds.push(DockCmd::SavePreset(n)),
                    DockDialog::Export(p) if !p.trim().is_empty() => self.dock.cmds.push(DockCmd::Export(p.trim().into())),
                    DockDialog::Import(p) if !p.trim().is_empty() => self.dock.cmds.push(DockCmd::Import(p.trim().into())),
                    _ => {}
                }
            } else if keep {
                self.dock.dialog = Some(dlg);
            }
        }
        // sobre
        if self.menu.about {
            let mut open = true;
            egui::Window::new(tr("about.title")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(RichText::new("ANIGO Studio").strong().size(18.0).color(theme::ACCENT));
                ui.label(format!("{}: {}", tr("about.version"), env!("CARGO_PKG_VERSION")));
                if let Some(g) = &self.gpu {
                    ui.label(format!("{}: {} ({})", tr("about.gpu"), g.adapter_name, g.backend));
                }
                ui.separator();
                let path_text = |p: Option<&std::path::Path>| p.map(|p| p.display().to_string()).unwrap_or_else(|| tr("about.no_persist").to_string());
                ui.label(format!("{}: {}", tr("about.layouts_file"), path_text(self.dock.path.as_deref())));
                ui.label(format!("{}: {}", tr("about.keymap_file"), path_text(self.keymap_path.as_deref())));
                ui.label(format!("{}: {}", tr("about.session"), self.session_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| tr("about.none").to_string())));
                ui.separator();
                ui.label(RichText::new(tr("about.warnings")).strong());
                let warns: Vec<&String> = self.dock.warnings.iter().chain(self.menu.keymap_warnings.iter()).collect();
                if warns.is_empty() {
                    ui.label(RichText::new(tr("about.none")).color(theme::TEXT_DIM));
                }
                for w in warns {
                    ui.label(RichText::new(w).size(11.5).color(theme::TEXT_DIM));
                }
            });
            if !open {
                self.menu.about = false;
            }
        }
    }

    // ------------------------------------------------------------------------------------ atalhos

    /// Texto do contexto de uma ação para a tabela (workspace(s), modos, área).
    pub(crate) fn action_context_text(d: &crate::keymap::ActionDef) -> String {
        let mut groups: Vec<(Option<WorkspaceId>, Vec<&'static str>)> = Vec::new();
        for c in d.ctxs {
            match groups.iter_mut().find(|(w, _)| *w == c.ws) {
                Some((_, modes)) => {
                    if let Some(m) = c.mode {
                        if !modes.contains(&m) {
                            modes.push(m);
                        }
                    }
                }
                None => groups.push((c.ws, c.mode.into_iter().collect())),
            }
        }
        let mut parts = Vec::new();
        for (w, modes) in groups {
            let name = match w {
                Some(w) => tr(&format!("workspace.{}", w.key())).to_string(),
                None => tr("dock.ctx.global").to_string(),
            };
            if modes.is_empty() || modes.len() >= 4 {
                parts.push(name);
            } else {
                let ms: Vec<String> = modes.iter().map(|m| tr(&format!("mode.{m}")).to_string()).collect();
                parts.push(format!("{name} ({})", ms.join(", ")));
            }
        }
        let areas: Vec<Area> = d.ctxs.iter().map(|c| c.area).filter(|a| *a != Area::Any).collect();
        let mut s = parts.join(", ");
        if !areas.is_empty() && areas.iter().all(|a| *a == areas[0]) && areas.len() == d.ctxs.len() {
            s.push_str(&format!(" \u{b7} {}", tr(if areas[0] == Area::Timeline { "dock.area.timeline" } else { "dock.area.viewport" })));
        }
        s
    }

    fn keyboard_window(&mut self, ctx: &egui::Context) {
        if !self.menu.keyboard {
            return;
        }
        // captura: a próxima combinação vira o atalho (Esc cancela); a tecla é consumida para não disparar a ação
        if let Some((id, idx)) = self.menu.kb.capture {
            let hit = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !is_modifier_key(*key) => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, mods)) = hit {
                ctx.input_mut(|i| i.consume_key(mods, key));
                self.menu.kb.capture = None;
                if key != Key::Escape || mods.any() {
                    let chord = Chord { key, ctrl: mods.command || mods.ctrl, shift: mods.shift, alt: mods.alt };
                    let km = Arc::make_mut(&mut self.keymap);
                    let mut list = km.chords(id).to_vec();
                    match idx {
                        Some(i) if i < list.len() => list[i] = chord,
                        _ => list.push(chord),
                    }
                    km.set(id, list);
                    self.keymap_save();
                }
            }
        }
        let conflicts = self.keymap.conflicts();
        let mut open = true;
        egui::Window::new(tr("kb.title")).open(&mut open).default_size([760.0, 560.0]).resizable(true).collapsible(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(theme::TEXT_DIM));
                ui.add(egui::TextEdit::singleline(&mut self.menu.kb.search).hint_text(tr("kb.search")).desired_width(280.0));
                ui.checkbox(&mut self.menu.kb.only_ws, tr("kb.only_ws"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (txt, color) = if conflicts.is_empty() { (tr("kb.no_conflicts").to_string(), theme::TEXT_DIM) } else { (format!("{} {}", conflicts.len(), tr("kb.conflicts_n")), egui::Color32::from_rgb(0xff, 0x70, 0x70)) };
                    ui.label(RichText::new(txt).color(color).size(12.0));
                });
            });
            ui.add_space(2.0);
            let q = self.menu.kb.search.trim().to_lowercase();
            let ws = self.ws;
            let mut changes: Vec<(&'static str, Vec<Chord>)> = Vec::new();
            let mut reset: Vec<&'static str> = Vec::new();
            let mut capture: Option<(&'static str, Option<usize>)> = None;
            let mut shown = 0;
            ui.label(RichText::new(tr("kb.modal_note")).size(11.0).color(theme::TEXT_DIM));
            ui.separator();
            egui::ScrollArea::vertical().id_salt("kb_rows").auto_shrink([false, false]).max_height((ui.available_height() - 78.0).max(120.0)).show(ui, |ui| {
                let mut last_group: Option<Option<WorkspaceId>> = None;
                for d in ACTIONS {
                    let group = d.ctxs.first().and_then(|c| c.ws);
                    if self.menu.kb.only_ws && !d.ctxs.iter().any(|c| c.ws.is_none_or(|w| w == ws)) {
                        continue;
                    }
                    let label = tr(&d.label_key()).to_string();
                    let chords = self.keymap.chords(d.id).to_vec();
                    if !q.is_empty() {
                        let hay = format!("{} {} {}", label.to_lowercase(), d.id, chords.iter().map(|c| c.display().to_lowercase()).collect::<Vec<_>>().join(" "));
                        if !hay.contains(&q) {
                            continue;
                        }
                    }
                    shown += 1;
                    if last_group != Some(group) {
                        last_group = Some(group);
                        ui.add_space(6.0);
                        let name = match group {
                            Some(w) => tr(&format!("workspace.{}", w.key())).to_string(),
                            None => tr("dock.ctx.global").to_string(),
                        };
                        ui.label(RichText::new(name.to_uppercase()).size(10.5).strong().color(theme::TEXT_DIM));
                    }
                    let my_conflicts: Vec<&'static str> = conflicts.iter().filter_map(|c| if c.a == d.id { Some(c.b) } else if c.b == d.id { Some(c.a) } else { None }).collect();
                    ui.horizontal(|ui| {
                        let mut name = RichText::new(label.clone());
                        if self.keymap.is_modified(d.id) {
                            name = name.color(theme::ACCENT);
                        }
                        let left = egui::Layout::left_to_right(egui::Align::Center);
                        ui.allocate_ui_with_layout(egui::vec2(240.0, 20.0), left, |ui| ui.add(egui::Label::new(name).truncate())).inner.on_hover_text(d.id);
                        ui.allocate_ui_with_layout(egui::vec2(170.0, 20.0), left, |ui| ui.add(egui::Label::new(RichText::new(Self::action_context_text(d)).size(11.0).color(theme::TEXT_DIM)).truncate()));
                        if chords.is_empty() {
                            ui.label(RichText::new(tr("kb.disabled")).color(theme::TEXT_DIM).size(11.5));
                        }
                        for (i, c) in chords.iter().enumerate() {
                            let capturing = self.menu.kb.capture == Some((d.id, Some(i)));
                            let text = if capturing { tr("kb.press").to_string() } else { c.display() };
                            let mut b = egui::Button::new(RichText::new(text).monospace().size(12.0));
                            if my_conflicts.iter().any(|o| self.keymap.chords(o).contains(c)) {
                                b = b.stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(0xff, 0x70, 0x70)));
                            }
                            let r = ui.add(b).on_hover_text(tr("kb.assign"));
                            if r.clicked() {
                                capture = Some((d.id, Some(i)));
                            }
                            if r.secondary_clicked() {
                                let mut l = chords.clone();
                                l.remove(i);
                                changes.push((d.id, l));
                            }
                        }
                        if self.menu.kb.capture == Some((d.id, None)) {
                            ui.label(RichText::new(tr("kb.press")).size(11.5).color(theme::ACCENT));
                        } else if ui.small_button(icons::PLUS).on_hover_text(tr("kb.add")).clicked() {
                            capture = Some((d.id, None));
                        }
                        if self.keymap.is_modified(d.id) && ui.small_button(icons::ARROW_COUNTER_CLOCKWISE).on_hover_text(tr("kb.reset")).clicked() {
                            reset.push(d.id);
                        }
                        if !my_conflicts.is_empty() {
                            let names: Vec<String> = my_conflicts.iter().map(|o| tr(&format!("act.{o}")).to_string()).collect();
                            ui.label(RichText::new(icons::WARNING).color(egui::Color32::from_rgb(0xff, 0x70, 0x70))).on_hover_text(format!("{}: {}", tr("kb.conflict"), names.join(", ")));
                        }
                    });
                }
                if shown == 0 {
                    ui.add_space(12.0);
                    ui.label(RichText::new(tr("kb.empty")).color(theme::TEXT_DIM));
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("kb.reset_all"))).clicked() {
                    Arc::make_mut(&mut self.keymap).reset_all();
                    self.keymap_save();
                }
                ui.add(egui::TextEdit::singleline(&mut self.menu.kb.path).hint_text(tr("kb.path")).desired_width(260.0));
                if ui.button(tr("kb.import")).clicked() && !self.menu.kb.path.trim().is_empty() {
                    let p = self.menu.kb.path.trim().to_string();
                    match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|raw| Keymap::from_json(&raw).map_err(|e| e.to_string())) {
                        Ok((km, warns)) => {
                            self.keymap = Arc::new(km);
                            self.keymap_save();
                            self.menu.kb.message = format!("{} ({} {})", tr("kb.imported"), warns.len(), tr("kb.warnings"));
                            self.menu.kb.warnings = warns;
                        }
                        Err(e) => self.menu.kb.message = format!("{}: {e}", tr("kb.error")),
                    }
                }
                if ui.button(tr("kb.export")).clicked() && !self.menu.kb.path.trim().is_empty() {
                    let p = std::path::PathBuf::from(self.menu.kb.path.trim());
                    self.menu.kb.message = match self.keymap.save_to(&p, self.menu.kb.export_all) {
                        Ok(()) => format!("{}: {}", tr("kb.exported"), p.display()),
                        Err(e) => format!("{}: {e}", tr("kb.error")),
                    };
                }
                ui.checkbox(&mut self.menu.kb.export_all, tr("kb.export_all"));
            });
            if !self.menu.kb.message.is_empty() {
                ui.label(RichText::new(&self.menu.kb.message).size(11.5).color(theme::TEXT_DIM));
            }
            for w in self.menu.kb.warnings.iter().take(4) {
                ui.label(RichText::new(w).size(11.0).color(theme::TEXT_DIM));
            }
            if !changes.is_empty() || !reset.is_empty() {
                let km = Arc::make_mut(&mut self.keymap);
                for (id, list) in changes {
                    km.set(id, list);
                }
                for id in reset {
                    km.reset(id);
                }
                self.keymap_save();
            }
            if let Some(c) = capture {
                self.menu.kb.capture = Some(c);
            }
        });
        if !open {
            self.menu.keyboard = false;
            self.menu.kb.capture = None;
        }
    }
}

fn is_modifier_key(k: Key) -> bool {
    matches!(k, Key::AltLeft | Key::AltRight | Key::ControlLeft | Key::ControlRight | Key::ShiftLeft | Key::ShiftRight | Key::SuperLeft | Key::SuperRight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_text_is_readable_for_every_action() {
        for l in [Lang::PtBr, Lang::En] {
            i18n::set_lang(l);
            for d in ACTIONS {
                let t = StudioApp::action_context_text(d);
                assert!(!t.is_empty() && !t.contains("workspace."), "{}: {t}", d.id);
            }
        }
        i18n::set_lang(Lang::PtBr);
        let t = StudioApp::action_context_text(crate::keymap::actions::def("modelagem.object.delete").unwrap());
        assert!(t.contains("Modelagem") && t.contains("Objeto"), "{t}");
        let t = StudioApp::action_context_text(crate::keymap::actions::def("animacao.marker").unwrap());
        assert!(t.contains("Timeline"), "{t}");
        let t = StudioApp::action_context_text(crate::keymap::actions::def("modelagem.xray").unwrap());
        assert_eq!(t, "Modelagem \u{b7} Viewport");
        assert_eq!(StudioApp::action_context_text(crate::keymap::actions::def("edit.undo").unwrap()), "Global");
    }
}
