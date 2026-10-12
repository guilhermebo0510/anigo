//! Viewport do Cenário: navegação (órbita, voo RMB+WASD, foco, vistas ortográficas), seleção (clique,
//! marquee), gizmo W/E/R com snap, soltar da Biblioteca com fantasma, render (Workbench + Cel) e overlays.

use super::actions::prim_label_key;
use super::{Ghost, GhostItem, GizmoSession, Marquee, PlaceDragPayload, PlaceKind, SceneShading, StudioApp};
use crate::celview::LevelNodeIn;
use crate::i18n::tr;
use crate::icons;
use crate::library_ui::AssetDragPayload;
use crate::scene::bvh::Aabb;
use crate::scene::camview::{fly_look, fly_move, fly_speed, CamView, SceneViewKind};
use crate::scene::gizmo::{self, Axis3, DragDelta, GizmoDrag, GizmoFrame, GizmoMode, GizmoSpace, Handle, NewLocal};
use crate::scene::placement::{drop_target, place_asset, row_offsets, DropTarget};
use crate::scene::runtime::GeomState;
use crate::scene::selection::{self, is_pickable};
use crate::scene::snap::{self, align_up_to_normal, snap_angle, snap_scale_value, snap_translation_delta, SnapSettings};
use crate::theme;
use crate::viewport::OrbitCamera;
use crate::workbench::{px_size, DrawItem, LineInstance, MeshDrawData, ShadingMode, ViewSettings};
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level;
use anigo_core::{NodeKind, NodeSource, NodeTransformEntry, StylizedMaterial};
use eframe::wgpu;
use egui::{Color32, Key, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Stroke};
use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::{BTreeSet, HashSet};

const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(0xe8, 0x4a, 0x4a), Color32::from_rgb(0x6a, 0xcc, 0x4a), Color32::from_rgb(0x4a, 0x84, 0xf0)];
const HILITE: Color32 = Color32::from_rgb(0xff, 0xe0, 0x3a);

fn e2g(v: egui::Vec2) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// Câmera de edição equivalente a (olho, alvo, fov) — usada para "ver pela câmera da cena".
pub(crate) fn orbit_from_camera(eye: Vec3, target: Vec3, fov_y: f32) -> OrbitCamera {
    let dir = target - eye;
    let distance = dir.length().max(0.05);
    let back = (-dir / distance).normalize_or_zero();
    OrbitCamera {
        target,
        yaw: back.x.atan2(back.z),
        pitch: back.y.clamp(-1.0, 1.0).asin(),
        distance,
        fov_y,
        near: 0.05,
        far: 600.0,
        orthographic: false,
    }
}

/// Maior retângulo com a proporção `aspect` centrado em `outer`.
pub(crate) fn letterbox(outer: Rect, aspect: f32) -> Rect {
    let (w, h) = (outer.width(), outer.height());
    let (rw, rh) = if w / h.max(1.0) > aspect { (h * aspect, h) } else { (w, w / aspect) };
    Rect::from_center_size(outer.center(), egui::vec2(rw, rh))
}

pub(crate) fn node_material(project: &anigo_core::ProjectState, slot: &anigo_core::NodeSlot) -> StylizedMaterial {
    if slot.kind == NodeKind::CharacterRoot {
        return project.character_material().map(|m| m.material.clone()).unwrap_or_default();
    }
    let mut m = StylizedMaterial {
        base_color: [0.70, 0.73, 0.82, 1.0],
        shade_color: [0.36, 0.38, 0.56, 1.0],
        outline_color: [0.12, 0.10, 0.16, 1.0],
        ..Default::default()
    };
    slot.overrides.apply_to_material(&mut m);
    m
}

fn placeholder_draw() -> MeshDrawData {
    let b = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
    let c = b.corners();
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
    MeshDrawData {
        solid: vec![],
        lines: edges.iter().map(|(a, b)| LineInstance { a: c[*a].to_array(), b: c[*b].to_array(), color: [1.0, 0.66, 0.2, 0.95], width: 1.6 }).collect(),
        points: vec![],
    }
}

enum MenuAction {
    Duplicate,
    Delete,
    Hide,
    Group,
    Ungroup,
    Focus,
    Floor,
    Copy,
    Paste,
    SelectChildren,
    SelectSame,
    Lock(bool),
}

impl StudioApp {
    pub(crate) fn scene_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
            let ppp = ui.ctx().pixels_per_point();
            self.scene_ui.view_size = Vec2::new(rect.width(), rect.height());
            self.scene_refresh();
            self.scene_input(ui, &resp, rect);
            self.scene_refresh();
            let (cam, render_rect) = self.scene_effective_camera(rect);
            self.scene_render(frame, render_rect, ppp, &cam);

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, theme::BG_DEEP);
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            if let Some(g) = self.gpu.as_ref() {
                if self.scene_ui.shading == SceneShading::Cel {
                    if let Some(t) = g.cel_tex {
                        painter.image(t, render_rect, uv, Color32::WHITE);
                    }
                }
                if let Some(t) = g.tex {
                    painter.image(t, render_rect, uv, Color32::WHITE);
                }
                if self.scene_ui.shading == SceneShading::Cel && g.cel.is_none() {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("char.no_preview"), egui::FontId::proportional(14.0), theme::TEXT_DIM);
                }
            }
            let view = CamView::from_orbit(&cam, Vec2::new(render_rect.width(), render_rect.height()));
            self.scene_overlays(ui, &painter, rect, render_rect, &view);
            self.scene_toolbars(ui, rect);
            self.scene_finish_gesture(ui.ctx());
            self.scene_trace(ui.ctx(), rect, render_rect, &view);
            if self.scene_ui.message_seen.0 != self.scene_ui.message {
                self.scene_ui.message_seen = (self.scene_ui.message.clone(), Some(std::time::Instant::now()));
            }
            let fresh = self.scene_ui.message_seen.1.is_some_and(|t| t.elapsed().as_secs_f32() < 6.0);
            if !self.scene_ui.hud.is_empty() || (fresh && !self.scene_ui.message.is_empty()) {
                let text = if self.scene_ui.hud.is_empty() { self.scene_ui.message.clone() } else { self.scene_ui.hud.clone() };
                if fresh {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                let pos = rect.left_bottom() + egui::vec2(10.0, -10.0);
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.5), Color32::WHITE);
                let r = Rect::from_min_size(pos - egui::vec2(0.0, galley.size().y), galley.size()).expand2(egui::vec2(6.0, 3.0));
                painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 205));
                painter.galley(r.min + egui::vec2(6.0, 3.0), galley, Color32::WHITE);
            }
        });
    }

    /// `ANIGO_TRACE_SCENE=1`: uma linha `scenetrace ...` por mudança de estado, para o smoke de xdotool
    /// (scripts/native_scene_smoke.sh). Coordenadas em pixels físicos da janela.
    fn scene_trace(&mut self, ctx: &egui::Context, rect: Rect, render_rect: Rect, view: &CamView) {
        if std::env::var_os("ANIGO_TRACE_SCENE").is_none() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let su = &self.scene_ui;
        let project = self.core.project();
        let h = self.core.history();
        let active = su.selection.active().and_then(|a| project.scene.node(a)).map(|n| n.transform.translation);
        let active = active.map(|t| format!("{:.3},{:.3},{:.3}", t.x, t.y, t.z)).unwrap_or_else(|| "-".into());
        let gx = self
            .gizmo_frame(view)
            .and_then(|f| {
                let (s, e) = f.axis_segment(Axis3::X);
                view.project(s.lerp(e, 0.7)).map(|(p, _)| p)
            })
            .map(|p| format!("{:.0},{:.0}", (render_rect.min.x + p.x) * ppp, (render_rect.min.y + p.y) * ppp))
            .unwrap_or_else(|| "-".into());
        let c = self.lib_ws.as_ref().map(|w| w.first_card_center).unwrap_or(Pos2::ZERO);
        let ready = su.runtime.geom.values().filter(|g| matches!(g, crate::scene::runtime::GeomState::Ready(_))).count();
        let line = format!(
            "scenetrace nodes={} sel={} undo={} redo={} active={} tool={:?} ghost={} loaded={} card0={:.0},{:.0} view={:.0},{:.0},{:.0},{:.0} gizmo_x={} hover={} drag={} snap={} grid={} fly={} script={}",
            project.scene.nodes.len(),
            su.selection.len(),
            h.undo_depth(),
            h.redo_depth(),
            active,
            su.tool,
            su.ghost.is_some(),
            ready,
            c.x * ppp,
            c.y * ppp,
            rect.min.x * ppp,
            rect.min.y * ppp,
            rect.max.x * ppp,
            rect.max.y * ppp,
            gx,
            su.hover_handle.map(|h| format!("{h:?}").replace(' ', "")).unwrap_or_else(|| "-".into()),
            su.session.is_some(),
            su.snap.grid_on,
            su.snap.grid_m,
            su.fly_active,
            match &su.script {
                None => "none",
                Some(s) if s.is_done() => "done",
                Some(_) => "running",
            },
        );
        if self.scene_ui.trace.as_deref() != Some(line.as_str()) {
            eprintln!("{line}");
            self.scene_ui.trace = Some(line);
        }
    }

    /// Câmera da viewport e o retângulo onde a imagem é desenhada (letterbox na vista pela câmera da cena).
    fn scene_effective_camera(&self, rect: Rect) -> (OrbitCamera, Rect) {
        let su = &self.scene_ui;
        if su.camera_view {
            let c = &self.core.project().scene.camera.camera;
            let r = letterbox(rect.shrink(18.0), c.aspect.clamp(0.3, 4.0));
            (orbit_from_camera(c.eye, c.target, c.fov_y), r)
        } else {
            (su.camera, rect)
        }
    }

    // ===================================================================== entrada

    fn scene_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect) {
        let ctx = ui.ctx().clone();
        let size = self.scene_ui.view_size;
        let mouse_abs = resp.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(rect.center());
        let mouse = e2g(mouse_abs - rect.min);
        let (shift, ctrl, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let aspect = size.x / size.y.max(1.0);

        // sessão de gizmo invalidada por outro comando (undo, apply…)?
        if self.scene_ui.session.is_some() && !self.core.is_previewing() {
            self.scene_ui.session = None;
        }

        if self.scene_ui.camera_view {
            // olhando pela câmera da cena: sem navegação; Esc/botão voltam à vista de edição
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.scene_ui.camera_view = false;
            }
            return;
        }

        // ---- câmera: roda = zoom (ou velocidade, com RMB); MMB = órbita; Shift+MMB = pan; Alt+LMB = órbita
        let secondary_down = ctx.input(|i| i.pointer.secondary_down());
        if ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary)) && resp.hovered() {
            self.scene_ui.fly_active = true;
        }
        if !secondary_down {
            self.scene_ui.fly_active = false;
        }
        let fly = self.scene_ui.fly_active;
        if resp.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                if fly {
                    self.scene_ui.speed = (self.scene_ui.speed * (1.0 + scroll * 0.004)).clamp(0.1, 16.0);
                } else {
                    let factor = (1.0 - scroll * 0.0025).clamp(0.5, 1.5);
                    let ndc = Vec2::new(mouse.x / size.x * 2.0 - 1.0, 1.0 - mouse.y / size.y * 2.0);
                    self.scene_ui.camera.zoom_to_cursor(factor, ndc, aspect);
                }
            }
        }
        let delta = e2g(ctx.input(|i| i.pointer.delta()));
        // alça do gizmo sob o ponto onde o botão foi pressionado (Alt+arrastar numa alça duplica, não orbita)
        let view = CamView::from_orbit(&self.scene_ui.camera, size);
        let tool = self.scene_ui.tool;
        let frame = if tool != GizmoMode::Select { self.gizmo_frame(&view) } else { None };
        let press_local = ctx.input(|i| i.pointer.press_origin()).map(|p| e2g(p - rect.min)).unwrap_or(mouse);
        let handle_at_press = frame.and_then(|f| gizmo::hit_test(tool, &f, &view, press_local));
        if self.scene_ui.session.is_none() {
            let alt_orbit = alt && handle_at_press.is_none() && resp.dragged_by(PointerButton::Primary);
            if resp.dragged_by(PointerButton::Middle) || alt_orbit {
                if shift {
                    self.scene_ui.camera.pan(delta.x, delta.y, size.y);
                } else if ctrl {
                    self.scene_ui.camera.zoom(1.0 + delta.y * 0.01);
                } else {
                    self.scene_ui.camera.orbit(delta.x, delta.y);
                }
            }
            if alt && resp.dragged_by(PointerButton::Secondary) {
                self.scene_ui.camera.zoom(1.0 + delta.y * 0.01);
            } else if fly && resp.dragged_by(PointerButton::Secondary) && !self.scene_ui.camera.orthographic {
                fly_look(&mut self.scene_ui.camera, delta.x, delta.y);
            } else if self.scene_ui.camera.orthographic && resp.dragged_by(PointerButton::Secondary) && !alt {
                self.scene_ui.camera.pan(delta.x, delta.y, size.y);
            }
        }
        if fly {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            let mut local = Vec3::ZERO;
            ctx.input(|i| {
                if i.key_down(Key::W) {
                    local.z += 1.0;
                }
                if i.key_down(Key::S) {
                    local.z -= 1.0;
                }
                if i.key_down(Key::D) {
                    local.x += 1.0;
                }
                if i.key_down(Key::A) {
                    local.x -= 1.0;
                }
                if i.key_down(Key::E) {
                    local.y += 1.0;
                }
                if i.key_down(Key::Q) {
                    local.y -= 1.0;
                }
            });
            if local != Vec3::ZERO {
                let cam = &mut self.scene_ui.camera;
                let boost = if shift { 3.0 } else { 1.0 };
                let speed = fly_speed(2.0, self.scene_ui.speed * boost, cam.distance);
                fly_move(cam, local.normalize(), speed, dt);
                ctx.request_repaint();
            }
        } else if alt {
            // Alt+LMB é órbita; o cursor avisa
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Move);
            }
        }

        // ---- gizmo e seleção
        let view = CamView::from_orbit(&self.scene_ui.camera, size);
        self.scene_pointer(&ctx, resp, &view, mouse, press_local, (shift, ctrl, alt), fly, frame, handle_at_press);

        // ---- soltar da Biblioteca / Colocar atores
        self.scene_drops(&ctx, resp, rect, &view, ctrl);

        // ---- menu de contexto (RMB sem arrastar)
        if resp.secondary_clicked() && !alt {
            let ids = self.scene_ui.runtime.proxies.len();
            let _ = ids;
            let p = self.core.project();
            let hit = selection::pick(&self.scene_ui.runtime.bvh, &self.scene_ui.runtime.proxies, &view, mouse, |id| is_pickable(p, id));
            if let Some(h) = hit {
                if !self.scene_ui.selection.contains(&h) {
                    self.scene_ui.selection.set_one(h);
                }
            }
        }
        let mut action: Option<MenuAction> = None;
        let has_sel = !self.scene_ui.selection.is_empty();
        let has_clip = self.scene_ui.clipboard.is_some();
        resp.context_menu(|ui| {
            let mut item = |ui: &mut egui::Ui, label: String, enabled: bool, a: MenuAction| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    action = Some(a);
                    ui.close();
                }
            };
            item(ui, format!("{}  {}  (F)", icons::FRAME_CORNERS, tr("scene.menu.focus")), true, MenuAction::Focus);
            ui.separator();
            item(ui, format!("{}  {}  (Ctrl+W)", icons::COPY, tr("scene.menu.duplicate")), has_sel, MenuAction::Duplicate);
            item(ui, format!("{}  {}  (Ctrl+C)", icons::CLIPBOARD, tr("scene.menu.copy")), has_sel, MenuAction::Copy);
            item(ui, format!("{}  {}  (Ctrl+V)", icons::CLIPBOARD_TEXT, tr("scene.menu.paste")), has_clip, MenuAction::Paste);
            item(ui, format!("{}  {}  (Del)", icons::TRASH, tr("scene.menu.delete")), has_sel, MenuAction::Delete);
            ui.separator();
            item(ui, format!("{}  {}  (End)", icons::ARROW_LINE_DOWN, tr("scene.menu.snap_floor")), has_sel, MenuAction::Floor);
            item(ui, format!("{}  {}  (H)", icons::EYE_SLASH, tr("scene.menu.hide")), has_sel, MenuAction::Hide);
            item(ui, format!("{}  {}", icons::LOCK, tr("scene.menu.lock")), has_sel, MenuAction::Lock(true));
            item(ui, format!("{}  {}", icons::LOCK_OPEN, tr("scene.menu.unlock")), has_sel, MenuAction::Lock(false));
            ui.separator();
            item(ui, format!("{}  {}  (Ctrl+G)", icons::FOLDER_SIMPLE, tr("scene.menu.group")), has_sel, MenuAction::Group);
            item(ui, format!("{}  {}  (Shift+G)", icons::FOLDER_SIMPLE_DASHED, tr("scene.menu.ungroup")), has_sel, MenuAction::Ungroup);
            item(ui, format!("{}  {}", icons::TREE_STRUCTURE, tr("scene.menu.select_children")), has_sel, MenuAction::SelectChildren);
            item(ui, format!("{}  {}  (Shift+E)", icons::SELECTION_ALL, tr("scene.menu.select_same")), has_sel, MenuAction::SelectSame);
        });
        if let Some(a) = action {
            match a {
                MenuAction::Duplicate => self.scene_duplicate(),
                MenuAction::Delete => self.scene_delete(),
                MenuAction::Hide => self.scene_set_hidden(true),
                MenuAction::Group => self.scene_group(),
                MenuAction::Ungroup => self.scene_ungroup(),
                MenuAction::Focus => self.scene_focus(),
                MenuAction::Floor => self.scene_snap_to_floor(),
                MenuAction::Copy => self.scene_copy(),
                MenuAction::Paste => self.scene_paste(),
                MenuAction::SelectChildren => self.scene_select_children(),
                MenuAction::SelectSame => self.scene_select_same(),
                MenuAction::Lock(l) => {
                    let ids = self.scene_ui.selection.ids().to_vec();
                    self.scene_set_locked(&ids, l);
                }
            }
        }
    }

    /// Orientação do gizmo: mundo, ou a do nó ativo no espaço Local (a escala é sempre local, como no UE).
    fn gizmo_orientation(&self) -> Quat {
        let su = &self.scene_ui;
        if su.space == GizmoSpace::Local || su.tool == GizmoMode::Scale {
            if let Some(a) = su.selection.active() {
                let (_, r, _) = su.runtime.world_of(a).to_scale_rotation_translation();
                return r.normalize();
            }
        }
        Quat::IDENTITY
    }

    fn gizmo_frame(&self, view: &CamView) -> Option<GizmoFrame> {
        let ids = self.scene_editable_selection();
        let roots = level::top_level(self.core.project(), &ids);
        let origin = self.scene_ui.pivot(&roots)?;
        Some(GizmoFrame::new(origin, self.gizmo_orientation(), view))
    }

    #[allow(clippy::too_many_arguments)]
    fn scene_pointer(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        view: &CamView,
        mouse: Vec2,
        press: Vec2,
        mods: (bool, bool, bool),
        fly: bool,
        frame: Option<GizmoFrame>,
        handle_at_press: Option<Handle>,
    ) {
        let (shift, ctrl, alt) = mods;

        // hover do gizmo (destaque)
        self.scene_ui.hover_handle = None;
        if let (Some(f), None) = (&frame, &self.scene_ui.session) {
            if resp.hovered() && !fly && !alt {
                self.scene_ui.hover_handle = gizmo::hit_test(self.scene_ui.tool, f, view, mouse);
            }
        }

        // ---- um arrasto de gizmo em curso
        if self.scene_ui.session.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.core.cancel_preview();
                self.scene_ui.session = None;
                self.scene_ui.hud.clear();
                return;
            }
            self.scene_gizmo_update(view, mouse, ctrl);
            if resp.drag_stopped_by(PointerButton::Primary) || !ctx.input(|i| i.pointer.primary_down()) {
                self.scene_gizmo_finish();
            } else {
                ctx.request_repaint();
            }
            return;
        }
        if fly {
            return;
        }

        // ---- início de arrasto: alça do gizmo (com Alt = duplica), senão retângulo de seleção
        if resp.drag_started_by(PointerButton::Primary) {
            if let (Some(f), Some(h)) = (frame, handle_at_press) {
                if self.scene_gizmo_begin(view, f, h, press, alt) {
                    return;
                }
            }
            if !alt {
                self.scene_ui.marquee = Some(Marquee { start: press, cur: mouse, additive: shift || ctrl });
            }
        }
        if let Some(m) = self.scene_ui.marquee.as_mut() {
            m.cur = mouse;
            ctx.request_repaint();
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Some(m) = self.scene_ui.marquee.take() {
                if (m.cur - m.start).length() > 4.0 {
                    let p = self.core.project();
                    let ids = selection::marquee(&self.scene_ui.runtime.proxies, view, m.start, m.cur, |id| is_pickable(p, id));
                    if m.additive {
                        for id in ids {
                            self.scene_ui.selection.add(id);
                        }
                    } else {
                        self.scene_ui.selection.set_many(ids);
                    }
                } else {
                    self.scene_click(view, m.cur, shift || ctrl);
                }
            }
        } else if resp.clicked_by(PointerButton::Primary) && !alt {
            // clique em cima de uma alça não troca a seleção
            let on_handle = frame.is_some_and(|f| gizmo::hit_test(self.scene_ui.tool, &f, view, mouse).is_some());
            if !on_handle {
                self.scene_click(view, mouse, shift || ctrl);
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.scene_ui.marquee = None;
        }
    }

    fn scene_click(&mut self, view: &CamView, mouse: Vec2, additive: bool) {
        let p = self.core.project();
        let hit = selection::pick(&self.scene_ui.runtime.bvh, &self.scene_ui.runtime.proxies, view, mouse, |id| is_pickable(p, id));
        match hit {
            Some(id) if additive => self.scene_ui.selection.toggle(id),
            Some(id) => self.scene_ui.selection.set_one(id),
            None if !additive => self.scene_ui.selection.clear(),
            None => {}
        }
    }

    // ===================================================================== gizmo

    fn scene_gizmo_begin(&mut self, view: &CamView, frame: GizmoFrame, handle: Handle, press: Vec2, duplicate: bool) -> bool {
        let mut ids = self.scene_editable_selection();
        if ids.is_empty() {
            return false;
        }
        let start_sel = self.scene_ui.selection.ids().to_vec();
        let mut dup = None;
        if duplicate {
            match level::plan_duplicate(self.core.project(), &ids, None) {
                Ok((cmd, map)) => {
                    if let Err(e) = self.core.preview(cmd.clone()) {
                        self.scene_say(e);
                        return false;
                    }
                    let copies: Vec<NodeId> = map.values().cloned().collect();
                    ids = level::top_level(self.core.project(), &copies);
                    self.scene_ui.selection.set_many(ids.clone());
                    dup = Some(cmd);
                    self.scene_refresh();
                }
                Err(_) => return false,
            }
        }
        let tool = self.scene_ui.tool;
        let Some(drag) = GizmoDrag::begin(tool, handle, frame, view, press) else {
            if dup.is_some() {
                self.core.cancel_preview();
                self.scene_ui.selection.set_many(start_sel);
            }
            return false;
        };
        self.core.begin_preview();
        let xf = self.scene_ui.runtime.xf_for(self.core.project(), &ids);
        if xf.is_empty() {
            self.core.cancel_preview();
            return false;
        }
        let pivot = frame.origin;
        self.scene_ui.session = Some(GizmoSession { drag, xf, pivot, dup, last: None, label: String::new(), moved: false, start_sel, start_bounds: self.scene_ui.runtime.bounds_of_many(&ids) });
        true
    }

    fn scene_gizmo_update(&mut self, view: &CamView, mouse: Vec2, ctrl: bool) {
        let Some(mut s) = self.scene_ui.session.take() else { return };
        let eff: SnapSettings = self.scene_ui.snap.effective(ctrl);
        let frame = s.drag.frame;
        let raw = s.drag.update(view, mouse);
        let mut hud = String::new();
        let new: Option<Vec<NewLocal>> = match raw {
            Some(DragDelta::Translate(delta)) => {
                let handle = s.drag.handle;
                let mut d = delta;
                if eff.surface_on && handle == Handle::Center {
                    let moving: BTreeSet<NodeId> = level::closure(self.core.project(), &s.xf.iter().map(|n| n.id.clone()).collect::<Vec<_>>()).into_iter().collect();
                    let (o, dir) = view.ray(mouse);
                    let tgt = drop_target(&self.scene_ui.runtime.bvh, o, dir, 0.0, 10.0, |id| moving.contains(id));
                    if let Some(b) = s.start_bounds {
                        let base = Vec3::new(b.center().x, b.min.y, b.center().z);
                        let mut desired = tgt.point + tgt.normal * eff.surface_offset;
                        if eff.grid_on && tgt.normal.dot(Vec3::Y) > 0.85 {
                            desired.x = snap::snap_value(desired.x, eff.grid_m);
                            desired.z = snap::snap_value(desired.z, eff.grid_m);
                        }
                        d = desired - base;
                    }
                } else if eff.grid_on {
                    let basis: Vec<Vec3> = match handle {
                        Handle::Axis(a) => vec![frame.axis(a)],
                        Handle::Plane(n) => {
                            let (a, b) = n.others();
                            vec![frame.axis(a), frame.axis(b)]
                        }
                        _ => frame.axes.to_vec(),
                    };
                    d = snap_translation_delta(frame.origin, delta, &basis, eff.grid_m, eff.absolute);
                }
                hud = format!("{} {}   {}", tr("scene.hud.move"), super::fmt_vec3(d), self.fmt_len(eff.grid_m, eff.grid_on));
                let mut out = gizmo::apply_translate(&s.xf, d);
                if eff.surface_on && handle == Handle::Center && eff.align_to_normal && s.xf.len() == 1 {
                    let (o, dir) = view.ray(mouse);
                    let moving: BTreeSet<NodeId> = level::closure(self.core.project(), &[s.xf[0].id.clone()]).into_iter().collect();
                    if let Some(h) = self.scene_ui.runtime.bvh.raycast(o, dir, 1e6, |id| moving.contains(id)) {
                        if s.xf[0].parent_world.abs_diff_eq(Mat4::IDENTITY, 1e-6) {
                            out[0].rotation = Some(align_up_to_normal(s.xf[0].start_local.rotation, h.normal));
                        }
                    }
                }
                Some(out)
            }
            Some(DragDelta::Rotate { axis, angle }) => {
                let a = if eff.rot_on { snap_angle(angle, eff.rot_deg) } else { angle };
                hud = format!("{} {:.1}\u{b0}", tr("scene.hud.rotate"), a.to_degrees());
                Some(gizmo::apply_rotate(&s.xf, s.pivot, axis, a))
            }
            Some(DragDelta::Scale { factors, uniform }) => {
                let mut f = factors;
                if eff.scale_on {
                    let s0 = s.xf[0].start_local.scale;
                    for i in 0..3 {
                        if (f[i] - 1.0).abs() > 1e-6 && s0[i].abs() > 1e-6 {
                            f[i] = snap_scale_value(s0[i], s0[i] * f[i], eff.scale_step, eff.absolute) / s0[i];
                        }
                    }
                    if uniform {
                        let u = f.x;
                        f = Vec3::splat(u);
                    }
                }
                hud = format!("{} {}", tr("scene.hud.scale"), super::fmt_vec3(f));
                Some(gizmo::apply_scale(&s.xf, s.pivot, f, uniform))
            }
            None => None,
        };
        if let Some(new) = new {
            let entries = full_entries(&new);
            if !entries.is_empty() {
                let cmd = Command::SetNodesTransform { entries };
                let _ = self.core.preview(cmd.clone());
                s.last = Some(cmd);
                s.moved = true;
            }
        }
        s.label = hud.clone();
        self.scene_ui.hud = hud;
        self.scene_ui.session = Some(s);
    }

    fn fmt_len(&self, meters: f32, on: bool) -> String {
        if on {
            format!("[{} {}]", tr("scene.hud.grid"), snap::format_length(meters, self.core.project().settings.units))
        } else {
            String::new()
        }
    }

    fn scene_gizmo_finish(&mut self) {
        let Some(s) = self.scene_ui.session.take() else { return };
        self.scene_ui.hud.clear();
        let Some(transform) = s.last.clone().filter(|_| s.moved) else {
            // nenhum movimento: nada entra no histórico (e a duplicação some)
            self.core.cancel_preview();
            if s.dup.is_some() {
                self.scene_ui.selection.set_many(s.start_sel);
            }
            return;
        };
        let cmd = match s.dup {
            Some(d) => Command::Batch { commands: vec![d, transform] },
            None => transform,
        };
        match self.core.commit_preview(cmd) {
            Ok(o) => self.scene_ui.message = super::describe_scene(&o.description),
            Err(e) => {
                if !e.contains("no effect") {
                    self.scene_ui.message = e;
                }
            }
        }
    }

    /// Termina edições numéricas (Details) quando o ponteiro é solto: UM passo de desfazer.
    pub(crate) fn scene_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.scene_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(cmd) = self.scene_ui.gesture.take() {
            match self.core.commit_preview(cmd) {
                Ok(o) => self.scene_ui.message = super::describe_scene(&o.description),
                Err(e) => {
                    if !e.contains("no effect") && !e.contains("already") {
                        self.scene_ui.message = e;
                    }
                }
            }
        }
    }

    // ===================================================================== soltar

    fn scene_drops(&mut self, ctx: &egui::Context, resp: &egui::Response, rect: Rect, view: &CamView, ctrl: bool) {
        let _ = ctx;
        let eff = self.scene_ui.snap.effective(ctrl);
        let pointer = resp.hover_pos().map(|p| e2g(p - rect.min));
        let lib = self.lib_ws.as_ref().and_then(|w| w.library().cloned());
        let target_at = |app: &StudioApp, p: Vec2| -> DropTarget {
            let (o, d) = view.ray(p);
            drop_target(&app.scene_ui.runtime.bvh, o, d, 0.0, app.scene_ui.camera.distance.clamp(3.0, 14.0), |_| false)
        };
        let mut ghost: Option<Ghost> = None;
        if let (Some(payload), Some(p), Some(lib)) = (resp.dnd_hover_payload::<AssetDragPayload>(), pointer, lib.as_ref()) {
            let target = target_at(self, p);
            let mut items = Vec::new();
            let mut specs = Vec::new();
            for id in &payload.ids {
                let uri = anigo_core::library_uri(&id.to_string());
                match self.scene_ui.cache.request_library(lib, &uri) {
                    Ok(r) => specs.push((r.name.clone(), self.scene_ui.cache.ready(&r.key), None::<String>)),
                    Err(e) => specs.push((String::new(), None, Some(e))),
                }
            }
            let widths: Vec<f32> = specs.iter().map(|(_, g, _)| g.as_ref().map_or(0.8, |g| (g.local.max.x - g.local.min.x).max(0.2))).collect();
            let offs = row_offsets(&widths, 0.3);
            for ((name, geom, err), dx) in specs.into_iter().zip(offs) {
                let mut t = target;
                if widths.len() > 1 {
                    t.point.x += dx;
                }
                let local = geom.as_ref().map_or(snap::LocalBox { min: Vec3::new(-0.4, 0.0, -0.4), max: Vec3::new(0.4, 0.8, 0.4) }, |g| g.local);
                let transform = place_asset(&t, &local, 0.0, Vec3::ONE, &eff);
                items.push(GhostItem { loading: geom.is_none() && err.is_none(), geom, transform, name, error: err });
            }
            ghost = Some(Ghost { items });
        } else if let (Some(payload), Some(p)) = (resp.dnd_hover_payload::<PlaceDragPayload>(), pointer) {
            let target = target_at(self, p);
            let (geom, label) = match payload.0 {
                PlaceKind::Primitive(shape) => (Some(self.scene_ui.cache.primitive(shape)), tr(prim_label_key(shape)).to_string()),
                PlaceKind::Camera => (None, tr("scene.actor.camera").to_string()),
                PlaceKind::Group => (None, tr("scene.actor.group").to_string()),
            };
            let local = geom.as_ref().map_or(snap::LocalBox { min: Vec3::splat(-0.2), max: Vec3::splat(0.2) }, |g| g.local);
            let transform = place_asset(&target, &local, 0.0, Vec3::ONE, &eff);
            ghost = Some(Ghost { items: vec![GhostItem { geom, transform, name: label, loading: false, error: None }] });
        }
        if ghost.is_some() || !self.scene_ui.ghost_pinned {
            self.scene_ui.ghost = ghost;
        }

        if let Some(payload) = resp.dnd_release_payload::<AssetDragPayload>() {
            if let Some(p) = pointer {
                let target = target_at(self, p);
                self.scene_place_library(&payload.ids, Some(target));
            }
            self.scene_ui.ghost = None;
        } else if let Some(payload) = resp.dnd_release_payload::<PlaceDragPayload>() {
            if let Some(p) = pointer {
                let target = target_at(self, p);
                self.scene_place_actor(payload.0, Some(target));
            }
            self.scene_ui.ghost = None;
        }
    }

    // ===================================================================== hotkeys

    pub(crate) fn scene_hotkeys(&mut self, ctx: &egui::Context) {
        if self.scene_ui.fly_active {
            return;
        }
        let consume = |mods: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(mods, key));
        // ferramentas (Q W E R, Espaço)
        if consume(Modifiers::NONE, Key::Q) {
            self.set_scene_tool(GizmoMode::Select);
        }
        if consume(Modifiers::NONE, Key::W) {
            self.set_scene_tool(GizmoMode::Translate);
        }
        if consume(Modifiers::NONE, Key::E) {
            self.set_scene_tool(GizmoMode::Rotate);
        }
        if consume(Modifiers::NONE, Key::R) {
            self.set_scene_tool(GizmoMode::Scale);
        }
        if consume(Modifiers::NONE, Key::Space) {
            self.scene_cycle_tool();
        }
        if consume(Modifiers::COMMAND, Key::Backtick) {
            self.scene_ui.space = if self.scene_ui.space == GizmoSpace::World { GizmoSpace::Local } else { GizmoSpace::World };
        }
        if consume(Modifiers::COMMAND, Key::Space) {
            self.scene_ui.drawer_open = !self.scene_ui.drawer_open;
        }
        // edição
        if consume(Modifiers::NONE, Key::Delete) || consume(Modifiers::NONE, Key::Backspace) {
            self.scene_delete();
        }
        if consume(Modifiers::COMMAND, Key::W) {
            self.scene_duplicate();
        }
        if consume(Modifiers::COMMAND, Key::A) {
            self.scene_select_all();
        }
        if consume(Modifiers::COMMAND, Key::I) {
            self.scene_invert_selection();
        }
        if consume(Modifiers::SHIFT, Key::E) {
            self.scene_select_same();
        }
        if consume(Modifiers::COMMAND, Key::H) {
            self.scene_unhide_all();
        } else if consume(Modifiers::NONE, Key::H) {
            self.scene_set_hidden(true);
        }
        if consume(Modifiers::COMMAND | Modifiers::SHIFT, Key::G) || consume(Modifiers::SHIFT, Key::G) {
            self.scene_ungroup();
        } else if consume(Modifiers::COMMAND, Key::G) {
            self.scene_group();
        }
        let (copy, cut, paste) = ctx.input(|i| {
            (
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
                i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            )
        });
        if copy || consume(Modifiers::COMMAND, Key::C) {
            self.scene_copy();
        }
        if cut || consume(Modifiers::COMMAND, Key::X) {
            self.scene_cut();
        }
        if paste || consume(Modifiers::COMMAND, Key::V) {
            self.scene_paste();
        }
        if consume(Modifiers::NONE, Key::Escape) {
            if self.scene_ui.session.is_none() {
                self.scene_ui.selection.clear();
            }
            self.scene_ui.marquee = None;
        }
        if consume(Modifiers::NONE, Key::F2) {
            if let Some(a) = self.scene_ui.selection.active().cloned() {
                let name = self.core.project().scene.node(&a).map(|n| n.name.clone()).unwrap_or_default();
                self.scene_ui.rename = Some((a, name, true));
            }
        }
        // snap
        if consume(Modifiers::COMMAND, Key::End) {
            self.scene_snap_origin_to_grid();
        } else if consume(Modifiers::NONE, Key::End) {
            self.scene_snap_to_floor();
        }
        if consume(Modifiers::NONE, Key::Z) {
            self.scene_ui.snap.surface_on = !self.scene_ui.snap.surface_on;
            let on = self.scene_ui.snap.surface_on;
            self.scene_say(format!("{}: {}", tr("scene.snap.surface"), if on { tr("scene.on") } else { tr("scene.off") }));
        }
        if consume(Modifiers::NONE, Key::V) {
            self.scene_say(tr("scene.msg.vertex_snap_phase2"));
        }
        let steps = &snap::GRID_STEPS_M;
        let rsteps = &snap::ROT_STEPS_DEG;
        if consume(Modifiers::NONE, Key::CloseBracket) {
            let i = steps.iter().position(|s| (*s - self.scene_ui.snap.grid_m).abs() < 1e-6).unwrap_or(4);
            self.scene_ui.snap.grid_m = steps[(i + 1).min(steps.len() - 1)];
        }
        if consume(Modifiers::NONE, Key::OpenBracket) {
            let i = steps.iter().position(|s| (*s - self.scene_ui.snap.grid_m).abs() < 1e-6).unwrap_or(4);
            self.scene_ui.snap.grid_m = steps[i.saturating_sub(1)];
        }
        if consume(Modifiers::SHIFT, Key::CloseBracket) {
            let i = rsteps.iter().position(|s| (*s - self.scene_ui.snap.rot_deg).abs() < 1e-6).unwrap_or(2);
            self.scene_ui.snap.rot_deg = rsteps[(i + 1).min(rsteps.len() - 1)];
        }
        if consume(Modifiers::SHIFT, Key::OpenBracket) {
            let i = rsteps.iter().position(|s| (*s - self.scene_ui.snap.rot_deg).abs() < 1e-6).unwrap_or(2);
            self.scene_ui.snap.rot_deg = rsteps[i.saturating_sub(1)];
        }
        // visão
        if consume(Modifiers::NONE, Key::F) {
            self.scene_focus();
        }
        if consume(Modifiers::NONE, Key::G) {
            self.scene_ui.game_view = !self.scene_ui.game_view;
        }
        for (key, view) in [(Key::G, SceneViewKind::Perspective), (Key::H, SceneViewKind::Front), (Key::J, SceneViewKind::Top), (Key::K, SceneViewKind::Right)] {
            if consume(Modifiers::ALT, key) {
                self.scene_set_view(view);
            }
        }
        for (key, sh) in [(Key::Num2, SceneShading::Wire), (Key::Num3, SceneShading::Solid), (Key::Num4, SceneShading::Cel)] {
            if consume(Modifiers::ALT, key) {
                self.scene_ui.shading = sh;
            }
        }
    }

    // ===================================================================== render

    fn scene_render(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32, cam: &OrbitCamera) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = px_size(Vec2::new(rect.width(), rect.height()), ppp);
        let aspect = size_px.0 as f32 / size_px.1.max(1) as f32;
        let project = self.core.project();
        let su = &mut self.scene_ui;
        let Some(gpu) = self.gpu.as_mut() else { return };
        let wire = su.shading == SceneShading::Wire;
        let selected: HashSet<&NodeId> = su.selection.ids().iter().collect();

        // ---- lista de nós de render (projeto + fantasmas da pré-visualização do soltar)
        let mut nodes: Vec<LevelNodeIn> = Vec::with_capacity(project.scene.nodes.len() + 4);
        for slot in &project.scene.nodes {
            let geom = match su.runtime.geom.get(&slot.node_id) {
                Some(GeomState::Ready(g)) => Some((g.key.clone(), g.mesh.clone())),
                _ => None,
            };
            nodes.push(LevelNodeIn {
                id: slot.node_id.to_string(),
                parent: slot.parent_id.as_ref().map(|p| p.to_string()),
                transform: slot.transform,
                visible: !su.runtime.hidden.contains(&slot.node_id),
                geom,
                material: node_material(project, slot),
            });
        }
        let ghost_items: Vec<GhostItem> = su.ghost.as_ref().map(|g| g.items.clone()).unwrap_or_default();
        for (i, g) in ghost_items.iter().enumerate() {
            if let Some(geom) = &g.geom {
                nodes.push(LevelNodeIn {
                    id: format!("__ghost{i}"),
                    parent: None,
                    transform: g.transform,
                    visible: true,
                    geom: Some((geom.key.clone(), geom.mesh.clone())),
                    material: StylizedMaterial { base_color: [0.55, 0.85, 1.0, 1.0], shade_color: [0.30, 0.50, 0.75, 1.0], ..Default::default() },
                });
            }
        }

        // ---- instâncias do Workbench (compartilham os buffers do ativo)
        let mut present: HashSet<NodeId> = HashSet::new();
        if su.gpu.placeholder.is_none() {
            su.gpu.placeholder = Some(gpu.wb.upload(&placeholder_draw(), Mat4::IDENTITY, [1.0; 4]));
        }
        for slot in &project.scene.nodes {
            if su.runtime.hidden.contains(&slot.node_id) {
                continue;
            }
            let world = su.runtime.world_of(&slot.node_id);
            let tint = if selected.contains(&slot.node_id) && !wire { [1.0, 0.88, 0.7, 1.0] } else { [1.0; 4] };
            match su.runtime.geom.get(&slot.node_id) {
                Some(GeomState::Ready(g)) => {
                    let key = format!("{}|{}", g.key, wire as u8);
                    if !su.gpu.base.contains_key(&key) {
                        let data = if wire { (*g.draw).clone() } else { MeshDrawData { solid: g.draw.solid.clone(), lines: vec![], points: vec![] } };
                        su.gpu.base.insert(key.clone(), gpu.wb.upload(&data, Mat4::IDENTITY, [1.0; 4]));
                    }
                    match su.gpu.inst.get_mut(&slot.node_id) {
                        Some((k, m)) if *k == key => gpu.wb.set_object_transform(m, world, tint),
                        _ => {
                            let m = gpu.wb.instance(&su.gpu.base[&key], world, tint);
                            su.gpu.inst.insert(slot.node_id.clone(), (key, m));
                        }
                    }
                    present.insert(slot.node_id.clone());
                }
                Some(GeomState::Loading) | Some(GeomState::Missing(_)) => {
                    if let Some(ph) = su.gpu.placeholder.as_ref() {
                        let m = gpu.wb.instance(ph, world * Mat4::from_scale(Vec3::splat(0.6)), [1.0; 4]);
                        su.gpu.inst.insert(slot.node_id.clone(), ("placeholder".into(), m));
                    }
                    present.insert(slot.node_id.clone());
                }
                _ => {}
            }
        }
        su.gpu.inst.retain(|id, _| present.contains(id));
        su.gpu.ghost.clear();
        for g in &ghost_items {
            if let Some(geom) = &g.geom {
                let key = format!("{}|{}", geom.key, wire as u8);
                if !su.gpu.base.contains_key(&key) {
                    let data = if wire { (*geom.draw).clone() } else { MeshDrawData { solid: geom.draw.solid.clone(), lines: vec![], points: vec![] } };
                    su.gpu.base.insert(key.clone(), gpu.wb.upload(&data, Mat4::IDENTITY, [1.0; 4]));
                }
                su.gpu.ghost.push(gpu.wb.instance(&su.gpu.base[&key], g.transform.to_matrix(), [0.7, 0.9, 1.0, 1.0]));
            }
        }
        // limpa buffers de ativos que ninguém usa mais
        if su.gpu.base.len() > 64 {
            let used: HashSet<String> = su.gpu.inst.values().map(|(k, _)| k.clone()).collect();
            su.gpu.base.retain(|k, _| used.contains(k));
        }

        let items: Vec<DrawItem> = su.gpu.inst.values().map(|(_, m)| DrawItem { mesh: m }).chain(su.gpu.ghost.iter().map(|m| DrawItem { mesh: m })).collect();
        let settings = ViewSettings {
            shading: match su.shading {
                SceneShading::Cel => ShadingMode::OverlayOnly,
                SceneShading::Solid => ShadingMode::Solid,
                SceneShading::Wire => ShadingMode::Wireframe,
            },
            show_grid: !su.game_view,
            xray: false,
            grid_scale: 1.0,
        };
        let recreated = gpu.wb.render(size_px, cam, &items, &settings);
        drop(items);

        let mut cel_recreated = false;
        if su.shading == SceneShading::Cel {
            if let Some(cel) = gpu.cel.as_mut() {
                let light = project.scene.lights.first().map(|l| l.light.clone()).unwrap_or_default();
                let dirty = cel.sync_level(&nodes, light, cam, aspect, [0.0, 0.0, 0.0, 0.0], 0);
                cel_recreated = cel.render_if(size_px, dirty);
                if let Some(e) = &cel.last_error {
                    self.shader_error = Some(format!("cel: {e}"));
                }
            }
        }
        if let Some(rs) = frame.wgpu_render_state() {
            if recreated || gpu.tex.is_none() {
                let mut renderer = rs.renderer.write();
                if let Some(old) = gpu.tex.take() {
                    renderer.free_texture(&old);
                }
                if let Some(view) = gpu.wb.output_view() {
                    gpu.tex = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear));
                }
            }
            if su.shading == SceneShading::Cel && (cel_recreated || gpu.cel_tex.is_none()) {
                let mut renderer = rs.renderer.write();
                if let Some(old) = gpu.cel_tex.take() {
                    renderer.free_texture(&old);
                }
                if let Some(view) = gpu.cel.as_ref().and_then(|c| c.view()) {
                    gpu.cel_tex = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear));
                }
            }
        }
    }

    // ===================================================================== overlays

    fn scene_overlays(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, render_rect: Rect, view: &CamView) {
        let to_screen = |p: Vec2| Pos2::new(render_rect.min.x + p.x, render_rect.min.y + p.y);
        let proj = |w: Vec3| view.project(w).map(|(p, _)| to_screen(p));
        let su = &self.scene_ui;
        let game = su.game_view;

        // máscara + guias na vista pela câmera
        if su.camera_view {
            let dim = Color32::from_rgba_unmultiplied(8, 8, 12, 215);
            for r in [
                Rect::from_min_max(rect.min, Pos2::new(rect.max.x, render_rect.min.y)),
                Rect::from_min_max(Pos2::new(rect.min.x, render_rect.max.y), rect.max),
                Rect::from_min_max(Pos2::new(rect.min.x, render_rect.min.y), Pos2::new(render_rect.min.x, render_rect.max.y)),
                Rect::from_min_max(Pos2::new(render_rect.max.x, render_rect.min.y), Pos2::new(rect.max.x, render_rect.max.y)),
            ] {
                if r.width() > 0.0 && r.height() > 0.0 {
                    painter.rect_filled(r, 0.0, dim);
                }
            }
            self.draw_guides(painter, render_rect);
        }
        if game {
            return;
        }

        // ---- caixas de seleção
        for id in su.selection.ids() {
            let active = su.selection.active() == Some(id);
            let color = if active { Color32::from_rgb(0xff, 0xb0, 0x40) } else { theme::SELECT };
            if let Some(GeomState::Ready(g)) = su.runtime.geom.get(id) {
                let m = su.runtime.world_of(id);
                let c: Vec<Option<Pos2>> = g.local.corners().iter().map(|c| proj(m.transform_point3(*c))).collect();
                let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
                for (a, b) in edges {
                    if let (Some(a), Some(b)) = (c[a], c[b]) {
                        painter.line_segment([a, b], Stroke::new(if active { 1.8 } else { 1.3 }, color));
                    }
                }
            } else if let Some(b) = su.runtime.bounds_of(id) {
                let c: Vec<Option<Pos2>> = b.corners().iter().map(|c| proj(*c)).collect();
                for (a, bb) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    if let (Some(a), Some(bb)) = (c[a], c[bb]) {
                        painter.line_segment([a, bb], Stroke::new(1.3, color));
                    }
                }
            }
        }

        // ---- marcadores: câmera de cena, grupo, ativo carregando/ausente
        let project = self.core.project();
        for slot in &project.scene.nodes {
            if su.runtime.hidden.contains(&slot.node_id) {
                continue;
            }
            let state = su.runtime.geom.get(&slot.node_id);
            let world = su.runtime.world_of(&slot.node_id);
            let origin = world.transform_point3(Vec3::ZERO);
            let Some(p) = proj(origin) else { continue };
            let selected = su.selection.contains(&slot.node_id);
            let col = if selected { theme::SELECT } else { Color32::from_rgb(0xb8, 0xc0, 0xd8) };
            match (&slot.source, state) {
                (Some(NodeSource::Camera { focal_mm, sensor_width_mm, sensor_height_mm }), _) => {
                    // pirâmide do campo de visão (0,6 m de profundidade)
                    let depth = 0.6;
                    let fov_y = anigo_core::level::vertical_fov(*focal_mm, *sensor_height_mm);
                    let hh = (fov_y * 0.5).tan() * depth;
                    let hw = hh * sensor_width_mm / sensor_height_mm;
                    let corners = [Vec3::new(-hw, -hh, -depth), Vec3::new(hw, -hh, -depth), Vec3::new(hw, hh, -depth), Vec3::new(-hw, hh, -depth)];
                    let pts: Vec<Option<Pos2>> = corners.iter().map(|c| proj(world.transform_point3(*c))).collect();
                    for i in 0..4 {
                        if let (Some(a), Some(b)) = (pts[i], pts[(i + 1) % 4]) {
                            painter.line_segment([a, b], Stroke::new(1.4, col));
                        }
                        if let Some(a) = pts[i] {
                            painter.line_segment([p, a], Stroke::new(1.0, col.gamma_multiply(0.7)));
                        }
                    }
                    painter.text(p, egui::Align2::CENTER_CENTER, icons::VIDEO_CAMERA, egui::FontId::proportional(18.0), col);
                }
                (_, Some(GeomState::None)) | (_, None) => {
                    if slot.kind != NodeKind::Light {
                        painter.text(p, egui::Align2::CENTER_CENTER, if slot.kind == NodeKind::Group { icons::FOLDER_SIMPLE } else { icons::CUBE_FOCUS }, egui::FontId::proportional(16.0), col);
                    }
                }
                (_, Some(GeomState::Loading)) => {
                    painter.text(p + egui::vec2(0.0, -26.0), egui::Align2::CENTER_CENTER, format!("{}…", tr("scene.loading")), egui::FontId::proportional(11.0), theme::TEXT_DIM);
                }
                (_, Some(GeomState::Missing(e))) => {
                    painter.text(p + egui::vec2(0.0, -26.0), egui::Align2::CENTER_CENTER, format!("{} {}", icons::WARNING, tr("scene.asset_missing")), egui::FontId::proportional(11.5), Color32::from_rgb(0xff, 0xb0, 0x50));
                    let _ = e;
                }
                _ => {}
            }
        }

        // ---- fantasma do soltar: caixa tracejada + base + rótulo
        if let Some(g) = &su.ghost {
            for item in &g.items {
                let m = item.transform.to_matrix();
                let lb = item.geom.as_ref().map(|g| g.local).unwrap_or(snap::LocalBox { min: Vec3::new(-0.4, 0.0, -0.4), max: Vec3::new(0.4, 0.8, 0.4) });
                let c: Vec<Option<Pos2>> = lb.corners().iter().map(|c| proj(m.transform_point3(*c))).collect();
                for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    if let (Some(a), Some(b)) = (c[a], c[b]) {
                        painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.4, Color32::from_rgb(0x70, 0xd8, 0xff)), 6.0, 4.0));
                    }
                }
                if let Some(p) = proj(m.transform_point3(lb.bottom_center())) {
                    painter.circle_filled(p, 3.5, Color32::from_rgb(0x70, 0xd8, 0xff));
                    let label = if item.loading { format!("{}  {}…", item.name, tr("scene.loading")) } else if let Some(e) = &item.error { format!("{} {e}", icons::WARNING) } else { item.name.clone() };
                    painter.text(p + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(12.0), Color32::WHITE);
                }
            }
        }

        // ---- gizmo
        if su.tool != GizmoMode::Select || su.session.is_some() {
            let drag_frame = su.session.as_ref().map(|s| s.drag.frame);
            let frame = drag_frame.or_else(|| self.gizmo_frame(view));
            if let Some(f) = frame {
                let active = su.session.as_ref().map(|s| s.drag.handle);
                self.draw_scene_gizmo(painter, &f, view, render_rect, active.or(su.hover_handle), su.session.is_some());
            }
        }

        // ---- marquee
        if let Some(m) = &su.marquee {
            if (m.cur - m.start).length() > 4.0 {
                let r = Rect::from_two_pos(to_screen(m.start), to_screen(m.cur));
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 40));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
        }

        // ---- indicador de eixos (canto inferior direito)
        let origin = rect.right_bottom() + egui::vec2(-46.0, -46.0);
        for (i, a) in Axis3::ALL.iter().enumerate() {
            let d = a.unit();
            let sx = d.dot(view.right);
            let sy = -d.dot(view.up);
            let tip = origin + egui::vec2(sx, sy) * 26.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, AXIS_COLORS[i]));
            painter.text(tip + egui::vec2(sx, sy) * 7.0, egui::Align2::CENTER_CENTER, ["X", "Y", "Z"][i], egui::FontId::proportional(11.0), AXIS_COLORS[i]);
        }
        let _ = ui;
    }

    fn draw_guides(&self, painter: &egui::Painter, r: Rect) {
        let g = self.scene_ui.guides;
        let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 120));
        if g.frame {
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 200)), egui::StrokeKind::Inside);
        }
        if g.thirds {
            for k in [1.0 / 3.0, 2.0 / 3.0] {
                let x = r.min.x + r.width() * k;
                let y = r.min.y + r.height() * k;
                painter.line_segment([Pos2::new(x, r.min.y), Pos2::new(x, r.max.y)], line);
                painter.line_segment([Pos2::new(r.min.x, y), Pos2::new(r.max.x, y)], line);
            }
        }
        if g.center {
            let c = r.center();
            painter.line_segment([Pos2::new(c.x, r.min.y), Pos2::new(c.x, r.max.y)], line);
            painter.line_segment([Pos2::new(r.min.x, c.y), Pos2::new(r.max.x, c.y)], line);
            painter.circle_stroke(c, 5.0, line);
        }
        if g.safe {
            for (k, label) in [(0.9, "action"), (0.8, "title")] {
                let s = Rect::from_center_size(r.center(), r.size() * k);
                painter.rect_stroke(s, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 220, 90, 150)), egui::StrokeKind::Inside);
                painter.text(s.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(10.0), Color32::from_rgba_unmultiplied(255, 220, 90, 190));
            }
        }
        let c = &self.core.project().scene.camera.camera;
        painter.text(r.left_top() + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{:.0}\u{b0}  {:.2}:1", c.fov_y.to_degrees(), c.aspect), egui::FontId::proportional(11.5), Color32::WHITE);
    }

    fn draw_scene_gizmo(&self, painter: &egui::Painter, f: &GizmoFrame, view: &CamView, rect: Rect, hot: Option<Handle>, dragging: bool) {
        let to = |p: Vec2| Pos2::new(rect.min.x + p.x, rect.min.y + p.y);
        let proj = |w: Vec3| view.project(w).map(|(p, _)| to(p));
        let Some(o) = proj(f.origin) else { return };
        let tool = self.scene_ui.tool;
        let col = |a: Axis3, hot_axis: bool| if hot_axis { HILITE } else { AXIS_COLORS[a.index()] };
        let is_hot = |h: Handle| hot == Some(h);
        match tool {
            GizmoMode::Translate | GizmoMode::Scale => {
                for a in Axis3::ALL {
                    if dragging && hot != Some(Handle::Axis(a)) && !matches!(hot, Some(Handle::Center)) && !matches!(hot, Some(Handle::Plane(_))) {
                        continue;
                    }
                    let (s, e) = f.axis_segment(a);
                    let (Some(ps), Some(pe)) = (proj(s), proj(e)) else { continue };
                    let c = col(a, is_hot(Handle::Axis(a)));
                    painter.line_segment([ps, pe], Stroke::new(if is_hot(Handle::Axis(a)) { 3.2 } else { 2.4 }, c));
                    let dir = (pe - ps).normalized();
                    let perp = egui::vec2(-dir.y, dir.x);
                    if tool == GizmoMode::Translate {
                        painter.add(egui::Shape::convex_polygon(vec![pe + dir * 13.0, pe + perp * 5.0, pe - perp * 5.0], c, Stroke::NONE));
                    } else {
                        painter.rect_filled(Rect::from_center_size(pe, egui::vec2(9.0, 9.0)), 1.0, c);
                    }
                }
                for n in Axis3::ALL {
                    let q = f.plane_quad(n);
                    let pts: Vec<Pos2> = q.iter().filter_map(|c| proj(*c)).collect();
                    if pts.len() == 4 {
                        let hotp = is_hot(Handle::Plane(n));
                        let c = if hotp { HILITE } else { AXIS_COLORS[n.index()] };
                        painter.add(egui::Shape::convex_polygon(pts, c.gamma_multiply(if hotp { 0.55 } else { 0.28 }), Stroke::new(1.0, c.gamma_multiply(0.8))));
                    }
                }
                let hc = is_hot(Handle::Center);
                painter.rect_filled(Rect::from_center_size(o, egui::vec2(11.0, 11.0)), 2.0, if hc { HILITE } else { Color32::from_rgb(0xdd, 0xdd, 0xe6) });
            }
            GizmoMode::Rotate => {
                for a in Axis3::ALL {
                    let pts: Vec<Pos2> = f.ring_points(a, 64).into_iter().filter_map(proj).collect();
                    let hotr = is_hot(Handle::Ring(a));
                    if dragging && !hotr {
                        continue;
                    }
                    painter.add(egui::Shape::line(pts, Stroke::new(if hotr { 3.4 } else { 2.2 }, col(a, hotr))));
                }
                painter.circle_filled(o, 3.0, Color32::from_rgb(0xdd, 0xdd, 0xe6));
            }
            GizmoMode::Select => {}
        }
    }

    // ===================================================================== barras

    pub(crate) fn scene_toolbars(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let bar = |fill: u8| egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(24, 24, 30, fill)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3));
        // ---- barra 1: ferramentas + snap
        let r1 = Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(rect.width() - 16.0, 30.0));
        let mut tool = self.scene_ui.tool;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r1), |ui| {
            bar(220).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (t, icon, key) in [
                        (GizmoMode::Select, icons::CURSOR, "tool.select_box"),
                        (GizmoMode::Translate, icons::ARROWS_OUT_CARDINAL, "tool.move"),
                        (GizmoMode::Rotate, icons::ARROWS_CLOCKWISE, "tool.rotate"),
                        (GizmoMode::Scale, icons::ARROWS_OUT, "tool.scale"),
                    ] {
                        let hint = format!("{}  ({})", tr(key), match t {
                            GizmoMode::Select => "Q",
                            GizmoMode::Translate => "W",
                            GizmoMode::Rotate => "E",
                            GizmoMode::Scale => "R",
                        });
                        if ui.add(egui::Button::new(RichText::new(icon).size(16.0)).selected(tool == t)).on_hover_text(hint).clicked() {
                            tool = t;
                        }
                    }
                    ui.separator();
                    let local = self.scene_ui.space == GizmoSpace::Local;
                    if ui.add(egui::Button::new(RichText::new(if local { icons::CUBE } else { icons::GLOBE_SIMPLE }).size(16.0))).on_hover_text(format!("{}  (Ctrl+`)", if local { tr("scene.space.local") } else { tr("scene.space.world") })).clicked() {
                        self.scene_ui.space = if local { GizmoSpace::World } else { GizmoSpace::Local };
                    }
                    ui.separator();
                    self.snap_controls(ui);
                });
            });
        });
        if tool != self.scene_ui.tool {
            self.set_scene_tool(tool);
        }

        // ---- barra 2: vista
        let r2 = Rect::from_min_size(rect.min + egui::vec2(8.0, 42.0), egui::vec2(rect.width() - 16.0, 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(r2), |ui| {
            bar(190).show(ui, |ui| {
                ui.horizontal(|ui| self.view_controls(ui));
            });
        });
    }

    fn snap_controls(&mut self, ui: &mut egui::Ui) {
        let unit = self.core.project().settings.units;
        let snap = &mut self.scene_ui.snap;
        // posição
        ui.toggle_value(&mut snap.grid_on, RichText::new(icons::GRID_FOUR).size(15.0)).on_hover_text(tr("scene.snap.grid"));
        ui.menu_button(snap::format_length(snap.grid_m, unit), |ui| {
            for s in snap::GRID_STEPS_M {
                if ui.selectable_label((snap.grid_m - s).abs() < 1e-6, snap::format_length(s, unit)).clicked() {
                    snap.grid_m = s;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(format!("{}  ( [ ] )", tr("scene.snap.grid_size")));
        // rotação
        ui.toggle_value(&mut snap.rot_on, RichText::new(icons::ARROW_CLOCKWISE).size(15.0)).on_hover_text(tr("scene.snap.rotation"));
        ui.menu_button(format!("{}\u{b0}", snap.rot_deg), |ui| {
            for s in snap::ROT_STEPS_DEG {
                if ui.selectable_label((snap.rot_deg - s).abs() < 1e-6, format!("{s}\u{b0}")).clicked() {
                    snap.rot_deg = s;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(format!("{}  ( Shift+[ ] )", tr("scene.snap.rot_size")));
        // escala
        ui.toggle_value(&mut snap.scale_on, RichText::new(icons::ARROWS_OUT_SIMPLE).size(15.0)).on_hover_text(tr("scene.snap.scale"));
        ui.menu_button(format!("{}", snap.scale_step), |ui| {
            for s in snap::SCALE_STEPS {
                if ui.selectable_label((snap.scale_step - s).abs() < 1e-6, format!("{s}")).clicked() {
                    snap.scale_step = s;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr("scene.snap.scale_size"));
        ui.separator();
        // superfície
        ui.toggle_value(&mut snap.surface_on, RichText::new(icons::MAGNET).size(15.0)).on_hover_text(format!("{}  (Z)", tr("scene.snap.surface")));
        ui.menu_button(icons::CARET_DOWN, |ui| {
            ui.checkbox(&mut snap.align_to_normal, tr("scene.snap.align_normal"));
            ui.add(egui::DragValue::new(&mut snap.surface_offset).speed(0.01).range(0.0..=2.0).prefix(format!("{} ", tr("scene.snap.offset"))).suffix(" m"));
        });
        let abs_label = if snap.absolute { tr("scene.snap.absolute") } else { tr("scene.snap.incremental") };
        ui.toggle_value(&mut snap.absolute, abs_label).on_hover_text(tr("scene.snap.abs_tip"));
        ui.label(RichText::new(icons::INFO).color(theme::TEXT_DIM)).on_hover_text(tr("scene.snap.ctrl_tip"));
    }

    fn view_controls(&mut self, ui: &mut egui::Ui) {
        let mut set_view: Option<SceneViewKind> = None;
        let su = &mut self.scene_ui;
        // sombreamento
        for (s, key, icon) in [(SceneShading::Cel, "viewport.cel", icons::SPARKLE), (SceneShading::Solid, "viewport.solid", icons::CUBE), (SceneShading::Wire, "viewport.wireframe", icons::POLYGON)] {
            if ui.add(egui::Button::new(RichText::new(icon).size(15.0)).selected(su.shading == s)).on_hover_text(tr(key)).clicked() {
                su.shading = s;
            }
        }
        ui.separator();
        // vistas
        ui.menu_button(format!("{}  {}", icons::VIDEO, tr(su.view_kind.key())), |ui| {
            for v in SceneViewKind::ALL {
                if ui.selectable_label(su.view_kind == v, tr(v.key())).clicked() {
                    set_view = Some(v);
                    ui.close();
                }
            }
        });
        if ui.button(RichText::new(icons::FRAME_CORNERS).size(15.0)).on_hover_text(format!("{}  (F)", tr("scene.view.focus"))).clicked() {
            su.pending_focus = true;
        }
        ui.separator();
        ui.toggle_value(&mut su.game_view, RichText::new(icons::GAME_CONTROLLER).size(15.0)).on_hover_text(format!("{}  (G)", tr("scene.view.game")));
        ui.toggle_value(&mut su.camera_view, RichText::new(icons::VIDEO_CAMERA).size(15.0)).on_hover_text(tr("scene.view.through_camera"));
        ui.menu_button(icons::GRID_NINE, |ui| {
            ui.checkbox(&mut su.guides.thirds, tr("scene.guides.thirds"));
            ui.checkbox(&mut su.guides.center, tr("scene.guides.center"));
            ui.checkbox(&mut su.guides.safe, tr("scene.guides.safe"));
            ui.checkbox(&mut su.guides.frame, tr("scene.guides.frame"));
        })
        .response
        .on_hover_text(tr("scene.guides"));
        ui.separator();
        ui.label(RichText::new(icons::GAUGE).color(theme::TEXT_DIM)).on_hover_text(tr("scene.view.speed"));
        ui.add(egui::DragValue::new(&mut su.speed).speed(0.02).range(0.1..=16.0).max_decimals(2).suffix("\u{d7}")).on_hover_text(tr("scene.view.speed_tip"));
        ui.separator();
        ui.toggle_value(&mut su.drawer_open, RichText::new(icons::SQUARES_FOUR).size(15.0)).on_hover_text(format!("{}  (Ctrl+Space)", tr("scene.drawer")));
        ui.toggle_value(&mut su.place_open, RichText::new(icons::PLUS_SQUARE).size(15.0)).on_hover_text(tr("scene.place"));
        if let Some(v) = set_view {
            self.scene_set_view(v);
        }
        if std::mem::take(&mut self.scene_ui.pending_focus) {
            self.scene_focus();
        }
    }
}

/// Entradas `SetNodesTransform` com TUDO que o solver definiu (o comando final é absoluto).
pub(crate) fn full_entries(new: &[NewLocal]) -> Vec<NodeTransformEntry> {
    new.iter()
        .map(|n| NodeTransformEntry {
            node_id: n.id.clone(),
            translation: n.translation.map(|t| t.to_array()),
            rotation: n.rotation.map(|r| r.to_array()),
            scale: n.scale.map(|s| s.to_array()),
        })
        .filter(|e| e.translation.is_some() || e.rotation.is_some() || e.scale.is_some())
        .collect()
}
