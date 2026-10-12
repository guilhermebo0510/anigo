//! Dock dos workspaces (T-2.12): desenha os painéis de cada workspace numa árvore do `egui_tiles` (arrastar para dividir,
//! agrupar em abas, redimensionar, fechar), reabre painéis pelo menu Janela, redefine/salva/aplica layouts e persiste tudo
//! (arquivo do usuário + `SessionState`). O modelo persistido e o registro de painéis vivem em [`crate::layout`].
//!
//! Fluxo por quadro (`dock_ui`): comandos pendentes (menu) → garante a árvore do workspace → sincroniza painéis
//! controlados por flag (ex.: gaveta do Cenário) → `tree.ui` com o [`DockBehavior`] → processa fechamentos → persiste
//! quando o ponteiro solta (nunca no meio de um arrasto).

use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::layout::model::{insert_pane, LayoutNode, LayoutStore};
use crate::layout::registry::{self, PanelKind, PanelSpec};
use crate::layout::tiles::{self, PaneId};
use crate::modeling::Mode;
use crate::theme;
use crate::workspace::WorkspaceId;
use egui::{Rect, Sense, Stroke};
use egui_tiles::{Behavior, EditAction, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};
use std::collections::HashMap;
use std::path::PathBuf;

/// Altura do cabeçalho dos painéis "nus" (sem barra de abas).
pub(crate) const HEADER_H: f32 = 20.0;

/// Comandos de layout vindos dos menus; executados no começo do próximo `dock_ui`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DockCmd {
    /// Abre/fecha um painel do workspace atual.
    Toggle(&'static str),
    ResetCurrent,
    ResetAll,
    SavePreset(String),
    ApplyPreset(String),
    DeletePreset(String),
    /// Exporta TODOS os layouts (e presets) para um arquivo.
    Export(PathBuf),
    /// Importa layouts de um arquivo (entradas inválidas são descartadas com aviso).
    Import(PathBuf),
}

#[derive(Default)]
pub(crate) struct DockState {
    pub trees: HashMap<WorkspaceId, Tree<PaneId>>,
    /// Layouts por workspace que ainda não viraram árvore (carregados do arquivo/sessão) + presets.
    pub store: LayoutStore,
    /// Arquivo de layouts do usuário (`None` = não persiste, ex.: `--no-session`).
    pub path: Option<PathBuf>,
    pub cmds: Vec<DockCmd>,
    pub dirty: bool,
    pub last_saved: String,
    pub warnings: Vec<String>,
    /// Último aviso/estado para a barra de status (já traduzido).
    pub notice: String,
    /// Posição anterior de painéis controlados por flag que foram escondidos (volta ao lugar ao reabrir).
    pub remembered: HashMap<(WorkspaceId, &'static str), LayoutNode>,
    /// Diálogos dos menus (nome do preset, caminhos de importar/exportar).
    pub dialog: Option<DockDialog>,
    pub area: (f32, f32),
}

pub(crate) enum DockDialog {
    SavePreset(String),
    Export(String),
    Import(String),
}

impl DockState {
    pub fn new(path: Option<PathBuf>, store: LayoutStore, warnings: Vec<String>) -> Self {
        let last_saved = store.to_json().unwrap_or_default();
        Self { path, store, warnings, last_saved, area: (1200.0, 700.0), ..Default::default() }
    }

    /// Armazém atual: o que foi carregado + o estado vivo de cada árvore (para sessão e arquivo).
    pub fn snapshot(&self) -> LayoutStore {
        let mut s = self.store.clone();
        for (ws, tree) in &self.trees {
            match tiles::tree_to_node(tree) {
                Some(n) => {
                    s.workspaces.insert(ws.key().to_string(), n);
                }
                None => {
                    s.workspaces.remove(ws.key());
                }
            }
        }
        s
    }
}

// ----------------------------------------------------------------------------------------- comportamento do dock

struct DockBehavior<'a> {
    app: &'a mut StudioApp,
    frame: &'a mut eframe::Frame,
    in_tabs: std::collections::HashSet<TileId>,
    /// Painéis que o usuário fechou neste quadro (X do cabeçalho ou da aba).
    closed: Vec<PaneId>,
    edited: bool,
}

impl DockBehavior<'_> {
    /// Cabeçalho de um painel sem barra de abas: alça de arrastar + ícone/título + fechar.
    fn header(&mut self, ui: &mut egui::Ui, rect: Rect, spec: &PanelSpec, tile_id: TileId) -> UiResponse {
        let hdr = Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
        ui.painter().rect_filled(hdr, 0.0, theme::BG_DEEP);
        ui.painter().hline(hdr.x_range(), hdr.bottom(), Stroke::new(1.0, theme::BORDER));
        let closable = spec.kind == PanelKind::Panel;
        let close_rect = Rect::from_center_size(egui::pos2(hdr.right() - 12.0, hdr.center().y), egui::vec2(16.0, 16.0));
        let drag_rect = if closable { Rect::from_min_max(hdr.min, egui::pos2(close_rect.left() - 2.0, hdr.max.y)) } else { hdr };
        let drag = ui.interact(drag_rect, ui.id().with(("dock_hdr", tile_id)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tr("dock.drag"));
        let hovered = drag.hovered() || drag.dragged();
        ui.painter().text(egui::pos2(hdr.left() + 6.0, hdr.center().y), egui::Align2::LEFT_CENTER, icons::DOTS_SIX_VERTICAL, egui::FontId::proportional(13.0), if hovered { theme::ACCENT } else { theme::TEXT_DIM });
        ui.painter().text(
            egui::pos2(hdr.left() + 22.0, hdr.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{}  {}", spec.icon, tr(spec.title_key)),
            egui::FontId::proportional(11.5),
            theme::TEXT_DIM,
        );
        if closable {
            let c = ui.interact(close_rect, ui.id().with(("dock_close", tile_id)), Sense::click()).on_hover_text(tr("dock.close"));
            if c.hovered() {
                ui.painter().rect_filled(close_rect, 3.0, theme::BG_HOVER);
            }
            ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, icons::X, egui::FontId::proportional(11.0), if c.hovered() { theme::TEXT } else { theme::TEXT_DIM });
            if c.clicked() {
                self.closed.push(spec.id);
            }
        }
        if drag.drag_started() {
            UiResponse::DragStarted
        } else {
            UiResponse::None
        }
    }
}

impl Behavior<PaneId> for DockBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, pane: &mut PaneId) -> UiResponse {
        let Some(spec) = registry::find(pane) else { return UiResponse::None };
        let rect = ui.max_rect();
        let central = spec.kind == PanelKind::Viewport;
        if central {
            self.app.bridge_note_viewport(rect);
        }
        // o viewport 3D desenha o próprio fundo; a Biblioteca (grade) usa o quadro padrão de painel central
        if central && spec.id != "biblioteca.browser" {
            self.app.dock_draw(ui, self.frame, spec.id);
            return UiResponse::None;
        }
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        let mut resp = UiResponse::None;
        let mut body = rect;
        if !self.in_tabs.contains(&tile_id) {
            resp = self.header(ui, rect, spec, tile_id);
            body.min.y += HEADER_H;
        }
        let margin = if central { egui::vec2(8.0, 8.0) } else { egui::vec2(8.0, 2.0) };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(margin)));
        child.set_clip_rect(body);
        self.app.dock_draw(&mut child, self.frame, spec.id);
        resp
    }

    fn tab_title_for_pane(&mut self, pane: &PaneId) -> egui::WidgetText {
        match registry::find(pane) {
            Some(s) => format!("{}  {}", s.icon, tr(s.title_key)).into(),
            None => (*pane).into(),
        }
    }

    fn is_tab_closable(&self, tiles: &Tiles<PaneId>, tile_id: TileId) -> bool {
        matches!(tiles.get(tile_id), Some(Tile::Pane(p)) if registry::find(p).is_some_and(|s| s.kind == PanelKind::Panel))
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<PaneId>, tile_id: TileId) -> bool {
        if let Some(Tile::Pane(p)) = tiles.get(tile_id) {
            self.closed.push(p);
        }
        // quem remove é `dock_close_panel` (mantém flags e modelo coerentes)
        false
    }

    fn on_edit(&mut self, _action: EditAction) {
        self.edited = true;
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions::default()
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        1.0
    }

    fn min_size(&self) -> f32 {
        110.0
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        HEADER_H + 4.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        theme::BG_DEEP
    }

    fn resize_stroke(&self, style: &egui::Style, state: egui_tiles::ResizeState) -> Stroke {
        match state {
            egui_tiles::ResizeState::Idle => Stroke::new(1.0, theme::BORDER),
            egui_tiles::ResizeState::Hovering => style.visuals.widgets.hovered.fg_stroke,
            egui_tiles::ResizeState::Dragging => style.visuals.widgets.active.fg_stroke,
        }
    }
}

// ----------------------------------------------------------------------------------------- app

impl StudioApp {
    /// Desenha o corpo do painel `id` (os métodos de cada workspace; o dock só dá o quadro).
    pub(crate) fn dock_draw(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, id: &str) {
        match id {
            "personagem.viewport" => self.character_viewport_ui(ui, frame),
            "personagem.inspector" => self.char_inspector_body(ui),
            "modelagem.viewport" => self.viewport_ui(ui, frame),
            "modelagem.inspector" => self.modeling_inspector_body(ui),
            "modelagem.uv" => self.uv_editor_panel(ui),
            "posing.viewport" | "animacao.viewport" => self.posing_viewport_ui(ui, frame),
            "posing.inspector" => self.pose_inspector_body(ui),
            "shading.viewport" => self.shading_viewport_ui(ui, frame),
            "shading.targets" => self.shading_targets_body(ui),
            "shading.params" => self.shading_params_body(ui),
            "iluminacao.viewport" => self.light_viewport_ui(ui, frame),
            "iluminacao.inspector" => self.light_inspector_body(ui),
            "cenario.viewport" => self.scene_viewport_ui(ui, frame),
            "cenario.place" => self.scene_place_body(ui),
            "cenario.outliner" => self.scene_outliner_body(ui),
            "cenario.history" => self.scene_history_body(ui),
            "cenario.details" => self.scene_details_body(ui),
            "cenario.drawer" => self.scene_drawer_body(ui),
            "render.viewport" => self.render_viewport_ui(ui, frame),
            "render.settings" => self.render_settings_body(ui),
            "render.queue" => self.render_queue_body(ui),
            "animacao.inspector" => self.animation_inspector_body(ui),
            "animacao.timeline" => self.animation_timeline_body(ui),
            "biblioteca.browser" => self.lib_workspace().dock_browser(ui),
            "biblioteca.sidebar" => self.lib_workspace().dock_sidebar(ui),
            "biblioteca.details" => self.lib_workspace().dock_details(ui),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ flags dos painéis condicionais

    /// `(ligado, disponível)` de um painel controlado por flag; `None` se o painel não é controlado por flag.
    pub(crate) fn dock_flag(&self, id: &str) -> Option<(bool, bool)> {
        Some(match id {
            "modelagem.uv" => (self.show_uv_editor, self.modeling.mode == Mode::Edit),
            "cenario.place" => (self.scene_ui.place_open, true),
            "cenario.drawer" => (self.scene_ui.drawer_open, true),
            "render.queue" => (self.render_ui.prefs.queue_open, true),
            _ => return None,
        })
    }

    pub(crate) fn dock_set_flag(&mut self, id: &str, on: bool) {
        match id {
            "modelagem.uv" => self.show_uv_editor = on,
            "cenario.place" => self.scene_ui.place_open = on,
            "cenario.drawer" => self.scene_ui.drawer_open = on,
            "render.queue" => self.render_ui.prefs.queue_open = on,
            _ => {}
        }
    }

    /// Painel visível agora? (flag ligada e disponível; os demais: presentes na árvore.)
    pub(crate) fn dock_panel_open(&self, ws: WorkspaceId, id: &str) -> bool {
        match self.dock_flag(id) {
            Some((on, avail)) => on && avail,
            None => match self.dock.trees.get(&ws) {
                Some(t) => tiles::find_pane(t, id).is_some(),
                // árvore ainda não construída: vale o layout salvo (ou o padrão, que tem todos)
                None => self.dock.store.workspaces.get(ws.key()).is_none_or(|n| n.contains(id)),
            },
        }
    }

    fn dock_present_ids(&self, ws: WorkspaceId) -> Vec<&'static str> {
        registry::panels(ws).iter().filter(|p| self.dock_flag(p.id).is_none_or(|(on, av)| on && av)).map(|p| p.id).collect()
    }

    // ------------------------------------------------------------------ árvore

    fn dock_known(ws: WorkspaceId) -> (Vec<&'static str>, [&'static str; 1]) {
        (registry::ids(ws), [registry::central(ws).id])
    }

    /// Constrói (uma vez por workspace) a árvore: layout salvo válido, senão o padrão para a janela atual.
    fn dock_ensure_tree(&mut self, ws: WorkspaceId, area: (f32, f32)) {
        if self.dock.trees.contains_key(&ws) {
            return;
        }
        let (known, req) = Self::dock_known(ws);
        let saved = self.dock.store.workspaces.get(ws.key()).cloned();
        let had_saved = saved.is_some();
        let node = saved.and_then(|n| n.sanitized_with_required(&known, &req));
        if had_saved && node.is_none() {
            self.dock.warnings.push(format!("{}: {}", ws.key(), tr("dock.layout_warn")));
            self.dock.notice = tr("dock.layout_warn").to_string();
        }
        let node = node.unwrap_or_else(|| {
            let present = self.dock_present_ids(ws);
            registry::default_layout(ws, area, &|id| present.contains(&id))
        });
        self.dock.trees.insert(ws, tiles::node_to_tree(ws, &node));
    }

    fn dock_model(&self, ws: WorkspaceId) -> Option<LayoutNode> {
        self.dock.trees.get(&ws).and_then(tiles::tree_to_node)
    }

    fn dock_rebuild(&mut self, ws: WorkspaceId, node: LayoutNode) {
        let (known, req) = Self::dock_known(ws);
        let node = node.sanitized_with_required(&known, &req).unwrap_or_else(|| registry::default_layout(ws, self.dock.area, &|_| true));
        self.dock.trees.insert(ws, tiles::node_to_tree(ws, &node));
        self.dock.dirty = true;
    }

    /// Painéis controlados por flag: a flag manda (entra/sai da árvore para refletir a flag).
    fn dock_sync_flags(&mut self, ws: WorkspaceId, area: (f32, f32)) {
        let central = registry::central(ws).id;
        for p in registry::panels(ws).iter().filter(|p| p.flagged) {
            let Some((on, avail)) = self.dock_flag(p.id) else { continue };
            let want = on && avail;
            let Some(model) = self.dock_model(ws) else { return };
            let have = model.contains(p.id);
            if want && !have {
                // volta ao lugar anterior se o resto do layout não mudou; senão ao "lar" do painel
                let mut current: Vec<&str> = model.panes();
                current.push(p.id);
                current.sort_unstable();
                let restored = self.dock.remembered.remove(&(ws, p.id)).filter(|m| {
                    let mut ids = m.panes();
                    ids.sort_unstable();
                    ids == current
                });
                let node = restored.unwrap_or_else(|| insert_pane(Some(model), p.id, p.home, p.px, area, central));
                self.dock_rebuild(ws, node);
            } else if !want && have {
                self.dock.remembered.insert((ws, p.id), model.clone());
                if let Some(node) = model.without(p.id) {
                    self.dock_rebuild(ws, node);
                }
            }
        }
    }

    /// X do cabeçalho/aba: painéis de flag desligam a flag (o sync tira da árvore); os demais saem direto.
    fn dock_close_panel(&mut self, ws: WorkspaceId, id: &'static str) {
        if self.dock_flag(id).is_some() {
            self.dock_set_flag(id, false);
        } else if let Some(model) = self.dock_model(ws) {
            if let Some(node) = model.without(id) {
                self.dock_rebuild(ws, node);
            }
        }
        self.dock.dirty = true;
    }

    fn dock_open_panel(&mut self, ws: WorkspaceId, id: &'static str, area: (f32, f32)) {
        if let Some((_, avail)) = self.dock_flag(id) {
            if avail {
                self.dock_set_flag(id, true);
            }
            return;
        }
        let Some(spec) = registry::panels(ws).iter().find(|p| p.id == id) else { return };
        let model = self.dock_model(ws);
        let node = insert_pane(model, id, spec.home, spec.px, area, registry::central(ws).id);
        self.dock_rebuild(ws, node);
    }

    /// Valores padrão das flags de visibilidade do workspace (usados ao redefinir o layout).
    fn dock_reset_flags(&mut self, ws: WorkspaceId) {
        for p in registry::panels(ws).iter().filter(|p| p.flagged && p.id != "modelagem.uv") {
            self.dock_set_flag(p.id, true);
        }
    }

    fn dock_notice(&mut self, key: &str, detail: impl AsRef<str>) {
        let d = detail.as_ref();
        self.dock.notice = if d.is_empty() { tr(key).to_string() } else { format!("{}: {d}", tr(key)) };
    }

    fn dock_run_cmds(&mut self, ws: WorkspaceId, area: (f32, f32)) {
        let cmds = std::mem::take(&mut self.dock.cmds);
        for cmd in cmds {
            match cmd {
                DockCmd::Toggle(id) => {
                    if self.dock_panel_open(ws, id) {
                        self.dock_close_panel(ws, id);
                    } else {
                        self.dock_ensure_tree(ws, area);
                        self.dock_open_panel(ws, id, area);
                    }
                }
                DockCmd::ResetCurrent => {
                    self.dock_reset(ws);
                    self.dock_notice("dock.layout_reset", "");
                }
                DockCmd::ResetAll => {
                    for w in WorkspaceId::ALL {
                        self.dock_reset(w);
                    }
                    self.dock_notice("dock.layout_reset", "");
                }
                DockCmd::SavePreset(name) => {
                    self.dock_ensure_tree(ws, area);
                    if let Some(model) = self.dock_model(ws) {
                        self.dock.store.save_preset(&name, ws.key(), model);
                        self.dock.dirty = true;
                        self.dock_notice("dock.layout_saved", name.trim());
                    }
                }
                DockCmd::ApplyPreset(name) => {
                    let (known, req) = Self::dock_known(ws);
                    let node = self.dock.store.presets_for(ws.key()).find(|p| p.name == name).and_then(|p| p.layout.clone().sanitized_with_required(&known, &req));
                    match node {
                        Some(n) => {
                            // as flags dos painéis condicionais seguem o que o preset contém
                            for p in registry::panels(ws).iter().filter(|p| p.flagged) {
                                if self.dock_flag(p.id).is_some_and(|(_, avail)| avail) || p.id != "modelagem.uv" {
                                    self.dock_set_flag(p.id, n.contains(p.id));
                                }
                            }
                            self.dock.remembered.retain(|(w, _), _| *w != ws);
                            self.dock_rebuild(ws, n);
                            self.dock_notice("dock.layout_applied", name);
                        }
                        None => self.dock_notice("dock.layout_warn", name),
                    }
                }
                DockCmd::DeletePreset(name) => {
                    if self.dock.store.delete_preset(&name, ws.key()) {
                        self.dock.dirty = true;
                        self.dock_notice("dock.layout_deleted", name);
                    }
                }
                DockCmd::Export(path) => {
                    let snap = self.dock.snapshot();
                    match snap.save_to(&path) {
                        Ok(()) => self.dock_notice("dock.layout_exported", path.display().to_string()),
                        Err(e) => self.dock_notice("dock.layout_error", e.to_string()),
                    }
                }
                DockCmd::Import(path) => match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|raw| LayoutStore::parse(&raw).map_err(|e| e.to_string())) {
                    Ok(l) => {
                        let n = l.store.workspaces.len();
                        self.dock.warnings.extend(l.warnings);
                        // o importado vira o layout de cada workspace citado; árvores vivas são refeitas
                        for (k, node) in l.store.workspaces.iter() {
                            if let Some(w) = WorkspaceId::from_key(k) {
                                self.dock.trees.remove(&w);
                                self.dock.remembered.retain(|(x, _), _| *x != w);
                                self.dock.store.workspaces.insert(k.clone(), node.clone());
                            }
                        }
                        for p in l.store.presets {
                            self.dock.store.save_preset(&p.name, &p.workspace, p.layout);
                        }
                        self.dock.dirty = true;
                        self.dock_notice("dock.layout_imported", format!("{n}"));
                    }
                    Err(e) => self.dock_notice("dock.layout_error", e),
                },
            }
        }
    }

    fn dock_reset(&mut self, ws: WorkspaceId) {
        self.dock.trees.remove(&ws);
        self.dock.store.workspaces.remove(ws.key());
        self.dock.remembered.retain(|(w, _), _| *w != ws);
        self.dock_reset_flags(ws);
        self.dock.dirty = true;
    }

    /// Salva o arquivo do usuário quando algo mudou e o ponteiro está solto (nunca no meio de um arrasto).
    fn dock_persist(&mut self, ctx: &egui::Context) {
        if !self.dock.dirty || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.dock.dirty = false;
        let snap = self.dock.snapshot();
        let Some(path) = self.dock.path.clone() else { return };
        let Ok(json) = snap.to_json() else { return };
        if json == self.dock.last_saved {
            return;
        }
        match snap.save_to(&path) {
            Ok(()) => self.dock.last_saved = json,
            Err(e) => {
                tracing::warn!("falha ao salvar layouts em {}: {e}", path.display());
                self.dock_notice("dock.layout_error", e.to_string());
            }
        }
    }

    /// O dock do workspace atual (chamado depois da barra superior, do shelf e da barra de status).
    pub(crate) fn dock_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ws = self.ws;
        let rect = ui.available_rect_before_wrap();
        let area = (rect.width().max(200.0), rect.height().max(200.0));
        self.dock.area = area;
        self.dock_run_cmds(ws, area);
        self.dock_ensure_tree(ws, area);
        self.dock_sync_flags(ws, area);
        let Some(mut tree) = self.dock.trees.remove(&ws) else { return };
        let in_tabs = tiles::panes_in_tabs(&tree);
        let mut behavior = DockBehavior { app: self, frame, in_tabs, closed: Vec::new(), edited: false };
        tree.ui(&mut behavior, ui);
        let DockBehavior { closed, edited, .. } = behavior;
        self.dock.trees.insert(ws, tree);
        if edited {
            self.dock.dirty = true;
        }
        for id in closed {
            self.dock_close_panel(ws, id);
        }
        let ctx = ui.ctx().clone();
        self.dock_persist(&ctx);
    }
}
