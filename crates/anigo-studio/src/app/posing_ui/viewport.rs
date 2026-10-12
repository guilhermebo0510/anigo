//! Viewport do Posing: navegação, seleção de juntas (clique/marquee), gizmo FK de rotação, handles de IK
//! (mãos/pés com pole), quadril, alvo do olhar, cones de limites, equilíbrio e render cel do personagem posado.

use super::{character_xf, describe_pose, DragKind, Hot, Marquee, PoseDrag, StudioApp};
use crate::i18n::tr;
use crate::icons;
use crate::posing::edit::{self, HipsMove};
use crate::posing::limits_vis::limit_cone;
use crate::posing::pick;
use crate::posing::GestureTracker;
use crate::scene::camview::CamView;
use crate::scene::gizmo::{Axis3, DragDelta, GizmoDrag, GizmoFrame, GizmoMode, Handle};
use crate::theme;
use crate::workbench::{px_size, ShadingMode, ViewSettings};
use anigo_core::pose::{bone, Limb, PoseOp, PoseRig, PoseState, BONE_COUNT, BONE_NAMES, BONE_PARENTS};
use anigo_ik::pose_ik;
use eframe::wgpu;
use egui::{Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke};
use glam::{Mat4, Quat, Vec2, Vec3};

const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(0xe8, 0x4a, 0x4a), Color32::from_rgb(0x6a, 0xcc, 0x4a), Color32::from_rgb(0x4a, 0x84, 0xf0)];
const HILITE: Color32 = Color32::from_rgb(0xff, 0xe0, 0x3a);
const LEFT_COL: Color32 = Color32::from_rgb(0x5a, 0xa8, 0xff);
const RIGHT_COL: Color32 = Color32::from_rgb(0xff, 0x6a, 0x6a);
const CENTER_COL: Color32 = Color32::from_rgb(0xd0, 0xd0, 0xdc);
const IK_COL: Color32 = Color32::from_rgb(0xff, 0xd0, 0x40);
const OK_COL: Color32 = Color32::from_rgb(0x58, 0xd0, 0x7a);
const BAD_COL: Color32 = Color32::from_rgb(0xff, 0x5a, 0x4a);

fn e2g(v: egui::Vec2) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// Ferramenta do shelf do Posing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoseTool {
    Rig,
    Ik,
    Presets,
}

/// Dados da pose avaliados uma vez por quadro.
pub(crate) struct PoseView {
    pub rig: PoseRig,
    pub pose: PoseState,
    /// Juntas no espaço do personagem.
    pub joints_char: [Vec3; BONE_COUNT],
    /// Juntas no mundo (já com a transformação do nó do personagem).
    pub joints: [Vec3; BONE_COUNT],
    pub world_rots: [Quat; BONE_COUNT],
    pub xf: Mat4,
    pub xf_rot: Quat,
}

impl PoseView {
    pub fn to_world(&self, p: Vec3) -> Vec3 {
        self.xf.transform_point3(p)
    }
    pub fn to_char(&self, p: Vec3) -> Vec3 {
        self.xf.inverse().transform_point3(p)
    }
    pub fn dir_to_char(&self, d: Vec3) -> Vec3 {
        self.xf_rot.inverse() * d
    }
}

impl StudioApp {
    pub(crate) fn posing_tool(&self) -> PoseTool {
        // na Animação o índice da tool shelf escolhe o editor (dope/curvas/clips): o viewport é sempre o FK
        if self.ws == crate::workspace::WorkspaceId::Animacao {
            return PoseTool::Rig;
        }
        match *self.tool_idx.get(&self.ws).unwrap_or(&0) {
            1 => PoseTool::Ik,
            2 => PoseTool::Presets,
            _ => PoseTool::Rig,
        }
    }

    pub(crate) fn pose_view(&mut self) -> PoseView {
        let rig = self.pose_rig();
        let pose = self.pose_now();
        let joints_char = rig.joint_positions(&pose);
        let xf = character_xf(&self.core);
        let xf_rot = self.core.project().character_node().map(|n| n.transform.rotation).unwrap_or(Quat::IDENTITY);
        let mut joints = joints_char;
        for j in joints.iter_mut() {
            *j = xf.transform_point3(*j);
        }
        let world_rots = rig.world_rotations(&pose);
        PoseView { rig, pose, joints_char, joints, world_rots, xf, xf_rot }
    }

    pub(crate) fn posing_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
            let ppp = ui.ctx().pixels_per_point();
            self.pose_ui.view_size = Vec2::new(rect.width(), rect.height());
            // sessão de arrasto invalidada por outro comando (undo, apply…)?
            if self.pose_ui.drag.is_some() && !self.core.is_previewing() {
                self.pose_ui.drag = None;
            }
            let pv = self.pose_view();
            self.posing_input(ui, &resp, rect, &pv);
            let pv = self.pose_view();
            self.posing_render(frame, rect, ppp);

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, theme::BG_DEEP);
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            if let Some(g) = self.gpu.as_ref() {
                if let Some(t) = g.tex {
                    painter.image(t, rect, uv, Color32::WHITE);
                }
                if self.pose_ui.show_mesh {
                    if let Some(t) = g.cel_tex {
                        painter.image(t, rect, uv, Color32::WHITE);
                    }
                }
                if g.cel.is_none() {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("char.no_preview"), egui::FontId::proportional(14.0), theme::TEXT_DIM);
                }
            }
            let view = CamView::from_orbit(&self.pose_ui.camera, Vec2::new(rect.width(), rect.height()));
            self.posing_overlays(&painter, rect, &view, &pv);
            if self.ws == crate::workspace::WorkspaceId::Animacao {
                self.anim_viewport_overlays(&painter, rect, &view, &pv);
                self.anim_trace(ui.ctx());
            }
            self.posing_header(ui, rect);
            self.posing_trace(ui.ctx(), rect, &view, &pv);
            self.posing_finish_gesture(ui.ctx());
            self.posing_hud(&painter, rect, ui.ctx());
        });
    }

    fn posing_hud(&mut self, painter: &egui::Painter, rect: Rect, ctx: &egui::Context) {
        if self.pose_ui.message_seen.0 != self.pose_ui.message {
            self.pose_ui.message_seen = (self.pose_ui.message.clone(), Some(std::time::Instant::now()));
        }
        let fresh = self.pose_ui.message_seen.1.is_some_and(|t| t.elapsed().as_secs_f32() < 6.0);
        if !self.pose_ui.hud.is_empty() || (fresh && !self.pose_ui.message.is_empty()) {
            let text = if self.pose_ui.hud.is_empty() { self.pose_ui.message.clone() } else { self.pose_ui.hud.clone() };
            if fresh {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
            let pos = rect.left_bottom() + egui::vec2(10.0, -10.0);
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.5), Color32::WHITE);
            let r = Rect::from_min_size(pos - egui::vec2(0.0, galley.size().y), galley.size()).expand2(egui::vec2(6.0, 3.0));
            painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 205));
            painter.galley(r.min + egui::vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }

    // ===================================================================== render

    fn posing_render(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = px_size(glam::Vec2::new(rect.width(), rect.height()), ppp);
        let aspect = size_px.0 as f32 / size_px.1.max(1) as f32;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let settings = ViewSettings { shading: ShadingMode::Solid, xray: false, ..self.settings };
        let recreated = gpu.wb.render(size_px, &self.pose_ui.camera, &[], &settings);
        let mut cel_recreated = false;
        if let Some(cel) = gpu.cel.as_mut() {
            cel.sync_character(&mut self.core, &self.pose_ui.camera, aspect, true);
            cel_recreated = cel.render(size_px);
            if let Some(e) = &cel.last_error {
                self.shader_error = Some(format!("cel: {e}"));
            }
            if !self.pose_ui.framed {
                if let Some((lo, hi)) = cel.character_bounds {
                    super::super::character_ui::frame_character(&mut self.pose_ui.camera, lo, hi);
                    self.pose_ui.framed = true;
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
            if cel_recreated || gpu.cel_tex.is_none() {
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

    // ===================================================================== entrada

    fn fk_frame(&self, view: &CamView, pv: &PoseView) -> Option<GizmoFrame> {
        if self.posing_tool() != PoseTool::Rig {
            return None;
        }
        let b = self.pose_ui.active?;
        if b == bone::ROOT {
            return None;
        }
        let orientation = if self.pose_ui.space_local { pv.xf_rot * pv.world_rots[b] } else { Quat::IDENTITY };
        Some(GizmoFrame::new(pv.joints[b], orientation, view))
    }

    /// Posição de mundo do pole do membro (junto da junta do meio, para o lado em que ela dobra).
    pub(crate) fn pole_position(&self, pv: &PoseView, limb: Limb) -> Vec3 {
        let [r, m, t] = limb.chain();
        let (a, b, c) = (pv.joints_char[r], pv.joints_char[m], pv.joints_char[t]);
        let bend = b - (a + c) * 0.5;
        let dir = if bend.length() > 0.02 { bend.normalize() } else { pose_ik::default_pole(&pv.rig, &pv.pose, limb) - b };
        pv.to_world(b + dir.normalize_or_zero() * 0.32)
    }

    fn gaze_world(&self, pv: &PoseView) -> Vec3 {
        pv.to_world(self.pose_ui.gaze.target)
    }

    /// Alça sob o cursor.
    fn hot_at(&self, view: &CamView, pv: &PoseView, mouse: Vec2) -> Option<Hot> {
        if let Some(f) = self.fk_frame(view, pv) {
            if let Some(Handle::Ring(a)) = crate::scene::gizmo::hit_test(GizmoMode::Rotate, &f, view, mouse) {
                return Some(Hot::Ring(a));
            }
        }
        let near = |p: Vec3, r: f32| view.project(p).is_some_and(|(px, _)| px.distance(mouse) <= r);
        for limb in Limb::ALL {
            if self.pose_ui.is_ik(limb) {
                if near(pv.joints[limb.chain()[2]], 11.0) {
                    return Some(Hot::Effector(limb));
                }
                if near(self.pole_position(pv, limb), 10.0) {
                    return Some(Hot::Pole(limb));
                }
            }
        }
        if self.posing_tool() == PoseTool::Ik {
            if self.pose_ui.gaze.enabled && near(self.gaze_world(pv), 12.0) {
                return Some(Hot::Gaze);
            }
            if self.pose_ui.spine_ik && near(pv.joints[bone::HEAD], 12.0) {
                return Some(Hot::Head);
            }
            if near(pv.joints[bone::HIPS], 12.0) {
                return Some(Hot::Hips);
            }
        }
        None
    }

    fn pose_pickable(b: usize) -> bool {
        b != bone::ROOT
    }

    fn posing_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect, pv: &PoseView) {
        let ctx = ui.ctx().clone();
        let size = self.pose_ui.view_size;
        let mouse_abs = resp.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(rect.center());
        let mouse = e2g(mouse_abs - rect.min);
        let (shift, ctrl, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let aspect = size.x / size.y.max(1.0);

        // ---- câmera: roda = zoom; MMB/RMB ou Alt+LMB = órbita; Shift+MMB = pan
        if resp.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let factor = (1.0 - scroll * 0.0025).clamp(0.5, 1.5);
                let ndc = Vec2::new(mouse.x / size.x * 2.0 - 1.0, 1.0 - mouse.y / size.y * 2.0);
                self.pose_ui.camera.zoom_to_cursor(factor, ndc, aspect);
            }
        }
        let delta = e2g(ctx.input(|i| i.pointer.delta()));
        let view = CamView::from_orbit(&self.pose_ui.camera, size);
        let press = ctx.input(|i| i.pointer.press_origin()).map(|p| e2g(p - rect.min)).unwrap_or(mouse);
        let hot_at_press = self.hot_at(&view, pv, press);
        if self.pose_ui.drag.is_none() {
            let alt_orbit = alt && hot_at_press.is_none() && resp.dragged_by(PointerButton::Primary);
            if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) || alt_orbit {
                if shift {
                    self.pose_ui.camera.pan(delta.x, delta.y, size.y);
                } else if ctrl {
                    self.pose_ui.camera.zoom(1.0 + delta.y * 0.01);
                } else {
                    self.pose_ui.camera.orbit(delta.x, delta.y);
                }
            }
        }
        if resp.hovered() && self.pose_ui.drag.is_none() {
            ctx.set_cursor_icon(if alt { egui::CursorIcon::Grab } else { egui::CursorIcon::Default });
        }
        // hover
        self.pose_ui.hover_hot = None;
        self.pose_ui.hover_bone = None;
        if self.pose_ui.drag.is_none() && resp.hovered() && !alt {
            self.pose_ui.hover_hot = self.hot_at(&view, pv, mouse);
            if self.pose_ui.hover_hot.is_none() {
                self.pose_ui.hover_bone = pick::pick_joint(&view, &pv.joints, mouse, Self::pose_pickable);
            }
        }
        self.posing_pointer(&ctx, resp, &view, pv, mouse, press, (shift, ctrl, alt), hot_at_press);
    }

    #[allow(clippy::too_many_arguments)]
    fn posing_pointer(&mut self, ctx: &egui::Context, resp: &egui::Response, view: &CamView, pv: &PoseView, mouse: Vec2, press: Vec2, mods: (bool, bool, bool), hot_at_press: Option<Hot>) {
        let (shift, ctrl, alt) = mods;
        // ---- arrasto em curso
        if self.pose_ui.drag.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.core.cancel_preview();
                self.pose_ui.drag = None;
                self.pose_ui.hud.clear();
                return;
            }
            self.posing_drag_update(ctx, view, pv, mouse, ctrl, shift);
            if resp.drag_stopped_by(PointerButton::Primary) || !ctx.input(|i| i.pointer.primary_down()) {
                self.posing_drag_finish();
            } else {
                ctx.request_repaint();
            }
            return;
        }
        // ---- início de arrasto: alça; senão marquee
        if resp.drag_started_by(PointerButton::Primary) && !alt {
            if let Some(h) = hot_at_press {
                if self.posing_drag_begin(view, pv, h, press, shift) {
                    return;
                }
            }
            self.pose_ui.marquee = Some(Marquee { start: press, cur: mouse, additive: shift || ctrl });
        }
        if let Some(m) = self.pose_ui.marquee.as_mut() {
            m.cur = mouse;
            ctx.request_repaint();
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Some(m) = self.pose_ui.marquee.take() {
                if (m.cur - m.start).length() > 4.0 {
                    let ids = pick::marquee(view, &pv.joints, m.start, m.cur, Self::pose_pickable);
                    if !m.additive {
                        self.pose_ui.selection.clear();
                    }
                    for b in &ids {
                        self.pose_ui.selection.insert(*b);
                    }
                    self.pose_ui.active = ids.last().copied().or(self.pose_ui.active.filter(|a| self.pose_ui.selection.contains(a)));
                    if self.pose_ui.selection.is_empty() {
                        self.pose_ui.active = None;
                    }
                } else {
                    self.posing_click(view, pv, m.cur, shift || ctrl);
                }
            }
        } else if resp.clicked_by(PointerButton::Primary) && !alt {
            let on_handle = self.hot_at(view, pv, mouse).is_some();
            if !on_handle {
                self.posing_click(view, pv, mouse, shift || ctrl);
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.pose_ui.marquee = None;
        }
    }

    fn posing_click(&mut self, view: &CamView, pv: &PoseView, mouse: Vec2, additive: bool) {
        match pick::pick_joint(view, &pv.joints, mouse, Self::pose_pickable) {
            Some(b) if additive => self.pose_ui.toggle(b),
            Some(b) => self.pose_ui.select_one(b),
            None if !additive => self.pose_ui.clear_selection(),
            None => {}
        }
    }

    // ===================================================================== arrasto

    fn posing_drag_begin(&mut self, view: &CamView, pv: &PoseView, hot: Hot, press: Vec2, shift: bool) -> bool {
        let base = pv.pose.clone();
        let (kind, gizmo, op) = match hot {
            Hot::Ring(a) => {
                let Some(b) = self.pose_ui.active else { return false };
                let Some(frame) = self.fk_frame(view, pv) else { return false };
                let Some(g) = GizmoDrag::begin(GizmoMode::Rotate, Handle::Ring(a), frame, view, press) else { return false };
                (DragKind::Fk { bone: b }, g, PoseOp::Edit)
            }
            Hot::Effector(limb) => {
                let origin = pv.joints[limb.chain()[2]];
                let frame = GizmoFrame::new(origin, Quat::IDENTITY, view);
                let Some(g) = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, frame, view, press) else { return false };
                let pole = pv.to_char(self.pole_position(pv, limb));
                (DragKind::Effector { limb, pole, start_target: pv.joints_char[limb.chain()[2]] }, g, PoseOp::Ik)
            }
            Hot::Pole(limb) => {
                let origin = self.pole_position(pv, limb);
                let frame = GizmoFrame::new(origin, Quat::IDENTITY, view);
                let Some(g) = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, frame, view, press) else { return false };
                let start_pole = pv.to_char(origin);
                (DragKind::Pole { limb, target: pv.joints_char[limb.chain()[2]], start_pole }, g, PoseOp::Ik)
            }
            Hot::Hips => {
                let frame = GizmoFrame::new(pv.joints[bone::HIPS], Quat::IDENTITY, view);
                let Some(g) = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, frame, view, press) else { return false };
                (DragKind::Hips { whole_body: shift }, g, PoseOp::Edit)
            }
            Hot::Head => {
                let frame = GizmoFrame::new(pv.joints[bone::HEAD], Quat::IDENTITY, view);
                let Some(g) = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, frame, view, press) else { return false };
                (DragKind::Head { start_target: pv.joints_char[bone::HEAD] }, g, PoseOp::Ik)
            }
            Hot::Gaze => {
                let frame = GizmoFrame::new(self.gaze_world(pv), Quat::IDENTITY, view);
                let Some(g) = GizmoDrag::begin(GizmoMode::Translate, Handle::Center, frame, view, press) else { return false };
                (DragKind::Gaze { start_target: self.pose_ui.gaze.target }, g, PoseOp::Gaze)
            }
        };
        self.core.begin_preview();
        self.pose_ui.drag = Some(PoseDrag { kind, gizmo, tracker: GestureTracker::begin(base, op), axis_lock: None });
        true
    }

    fn posing_drag_update(&mut self, ctx: &egui::Context, view: &CamView, pv: &PoseView, mouse: Vec2, ctrl: bool, shift: bool) {
        let Some(mut d) = self.pose_ui.drag.take() else { return };
        // travas de eixo (X/Y/Z segurados)
        let lock = ctx.input(|i| if i.key_down(Key::X) { Some(0) } else if i.key_down(Key::Y) { Some(1) } else if i.key_down(Key::Z) { Some(2) } else { None });
        d.axis_lock = lock;
        let mut opts = self.pose_ui.edit_opts();
        if ctrl {
            opts.snap_deg = if opts.snap_deg.is_some() { None } else { Some(15.0) };
        }
        let base = d.tracker.base.clone();
        let rig = pv.rig.clone();
        let raw = d.gizmo.update(view, mouse);
        let mut hud = String::new();
        let target: Option<PoseState> = match (&d.kind, raw) {
            (DragKind::Fk { bone: b }, Some(DragDelta::Rotate { axis, angle })) => {
                let a = edit::snap_angle(angle, opts.snap_deg);
                hud = format!("{} {}  {:.1}\u{b0}", tr(&format!("pose.bone.{}", BONE_NAMES[*b])), tr("pose.hud.rotate"), a.to_degrees());
                Some(edit::rotate_bone_world(&rig, &base, *b, pv.dir_to_char(axis), a, &opts))
            }
            (DragKind::Effector { limb, pole, start_target }, Some(DragDelta::Translate(v))) => {
                let v = lock_axis_fn(v, d.axis_lock);
                let target = *start_target + pv.dir_to_char(v);
                hud = format!("IK {}  ({:.2}, {:.2}, {:.2})", tr(&format!("pose.limb.{}", limb.key())), target.x, target.y, target.z);
                Some(edit::drag_effector(&rig, &base, *limb, target, Some(*pole), &opts))
            }
            (DragKind::Pole { limb, target, start_pole }, Some(DragDelta::Translate(v))) => {
                let v = lock_axis_fn(v, d.axis_lock);
                let pole = *start_pole + pv.dir_to_char(v);
                hud = format!("Pole {}", tr(&format!("pose.limb.{}", limb.key())));
                Some(edit::drag_effector(&rig, &base, *limb, *target, Some(pole), &opts))
            }
            (DragKind::Hips { whole_body }, Some(DragDelta::Translate(v))) => {
                let v = lock_axis_fn(v, d.axis_lock);
                let whole = *whole_body || shift;
                let delta = pv.dir_to_char(v);
                hud = format!("{} ({:.2}, {:.2}, {:.2}){}", if whole { tr("pose.hud.move_body") } else { tr("pose.hud.move_hips") }, delta.x, delta.y, delta.z, if self.pose_ui.foot_lock && !whole { format!("  [{}]", tr("pose.foot_lock")) } else { String::new() });
                let mv = HipsMove { delta, whole_body: whole, foot_lock: self.pose_ui.foot_lock, auto_ground: self.pose_ui.auto_ground };
                Some(edit::move_hips(&rig, &base, mv, &opts))
            }
            (DragKind::Head { start_target }, Some(DragDelta::Translate(v))) => {
                let v = lock_axis_fn(v, d.axis_lock);
                let t = *start_target + pv.dir_to_char(v);
                hud = format!("FABRIK  ({:.2}, {:.2}, {:.2})", t.x, t.y, t.z);
                let ik = pose_ik::IkOptions { limits: opts.profile, keep_end_orientation: true };
                Some(pose_ik::solve_chain(&rig, &base, &pose_ik::UPPER_BODY_CHAIN, t, &ik).pose)
            }
            (DragKind::Gaze { start_target }, Some(DragDelta::Translate(v))) => {
                let v = lock_axis_fn(v, d.axis_lock);
                let t = *start_target + pv.dir_to_char(v);
                self.pose_ui.gaze.target = t;
                hud = tr("pose.hud.gaze").to_string();
                Some(pose_ik::look_at_head(&rig, &base, t, self.pose_ui.gaze.weight, 0.35, opts.profile).pose)
            }
            _ => None,
        };
        if let Some(mut t) = target {
            if self.pose_ui.auto_ground && !matches!(d.kind, DragKind::Hips { .. }) {
                t = rig.grounded(&t, 0.0);
            }
            let now = self.pose_now();
            if let Some(cmd) = d.tracker.step(&now, t) {
                if let Err(e) = self.core.preview(cmd) {
                    self.pose_report(e);
                }
            }
        }
        self.pose_ui.hud = hud;
        self.pose_ui.drag = Some(d);
    }

    fn posing_drag_finish(&mut self) {
        let Some(d) = self.pose_ui.drag.take() else { return };
        self.pose_ui.hud.clear();
        match d.tracker.finish().filter(|_| d.tracker.moved).map(|c| self.anim_wrap_pose_cmd(c)) {
            Some(cmd) => match self.core.commit_preview(cmd) {
                Ok(o) => self.pose_ui.message = describe_pose(&o.description),
                Err(e) => self.pose_report(e),
            },
            None => self.core.cancel_preview(),
        }
    }

    // ===================================================================== overlays

    fn bone_color(&self, b: usize) -> Color32 {
        if self.pose_ui.active == Some(b) {
            return HILITE;
        }
        if self.pose_ui.selection.contains(&b) {
            return theme::SELECT;
        }
        if self.pose_ui.hover_bone == Some(b) {
            return Color32::WHITE;
        }
        if anigo_core::pose::is_left_bone(b) {
            LEFT_COL
        } else if anigo_core::pose::is_right_bone(b) {
            RIGHT_COL
        } else {
            CENTER_COL
        }
    }

    fn posing_overlays(&mut self, painter: &egui::Painter, rect: Rect, view: &CamView, pv: &PoseView) {
        let to = |p: Vec2| Pos2::new(rect.min.x + p.x, rect.min.y + p.y);
        let proj = |w: Vec3| view.project(w).map(|(p, _)| to(p));
        let tool = self.posing_tool();

        // ---- equilíbrio (sob o esqueleto)
        if self.pose_ui.show_balance {
            self.draw_balance(painter, view, pv, &proj);
        }

        // ---- cone de limites do osso ativo
        if self.pose_ui.show_limits && tool == PoseTool::Rig {
            if let Some(b) = self.pose_ui.active {
                if let Some(c) = limit_cone(&pv.rig, &pv.pose, b, self.pose_ui.profile, 48) {
                    let col = if c.inside { OK_COL } else { BAD_COL };
                    let o = pv.to_world(c.origin);
                    let radius = 0.28;
                    let pts: Vec<Pos2> = c.boundary.iter().filter_map(|d| proj(o + pv.xf_rot * *d * radius)).collect();
                    if let Some(po) = proj(o) {
                        if pts.len() > 3 {
                            for w in pts.windows(2) {
                                painter.add(egui::Shape::convex_polygon(vec![po, w[0], w[1]], col.gamma_multiply(0.13), Stroke::NONE));
                            }
                            painter.add(egui::Shape::line(pts, Stroke::new(1.6, col.gamma_multiply(0.9))));
                        }
                        if let Some(tip) = proj(o + pv.xf_rot * c.current * radius) {
                            painter.line_segment([po, tip], Stroke::new(2.4, col));
                            painter.circle_filled(tip, 3.5, col);
                        }
                    }
                }
            }
        }

        // ---- esqueleto
        if self.pose_ui.show_skeleton {
            self.draw_skeleton(painter, view, pv, &proj);
        }

        // ---- FK: anéis do gizmo
        if let Some(f) = self.fk_frame(view, pv) {
            let hot = match &self.pose_ui.drag {
                Some(d) => match d.gizmo.handle {
                    Handle::Ring(a) => Some(a),
                    _ => None,
                },
                None => match self.pose_ui.hover_hot {
                    Some(Hot::Ring(a)) => Some(a),
                    _ => None,
                },
            };
            let dragging = self.pose_ui.drag.is_some();
            for a in Axis3::ALL {
                let hotr = hot == Some(a);
                if dragging && !hotr {
                    continue;
                }
                let pts: Vec<Pos2> = f.ring_points(a, 64).into_iter().filter_map(proj).collect();
                let c = if hotr { HILITE } else { AXIS_COLORS[a.index()] };
                painter.add(egui::Shape::line(pts, Stroke::new(if hotr { 3.4 } else { 2.2 }, c)));
            }
            if let Some(o) = proj(f.origin) {
                painter.circle_filled(o, 3.0, Color32::from_rgb(0xdd, 0xdd, 0xe6));
            }
        }

        // ---- IK: efetuadores + pole
        for limb in Limb::ALL {
            if !self.pose_ui.is_ik(limb) {
                continue;
            }
            let [_, m, t] = limb.chain();
            let eff = pv.joints[t];
            let pole = self.pole_position(pv, limb);
            let hot_e = self.pose_ui.hover_hot == Some(Hot::Effector(limb)) || matches!(&self.pose_ui.drag, Some(PoseDrag { kind: DragKind::Effector { limb: l, .. }, .. }) if *l == limb);
            let hot_p = self.pose_ui.hover_hot == Some(Hot::Pole(limb)) || matches!(&self.pose_ui.drag, Some(PoseDrag { kind: DragKind::Pole { limb: l, .. }, .. }) if *l == limb);
            if let (Some(pm), Some(pp)) = (proj(pv.joints[m]), proj(pole)) {
                painter.extend(egui::Shape::dashed_line(&[pm, pp], Stroke::new(1.3, IK_COL.gamma_multiply(0.8)), 5.0, 4.0));
                let c = if hot_p { HILITE } else { IK_COL };
                painter.add(egui::Shape::convex_polygon(vec![pp + egui::vec2(0.0, -8.0), pp + egui::vec2(8.0, 0.0), pp + egui::vec2(0.0, 8.0), pp + egui::vec2(-8.0, 0.0)], c.gamma_multiply(0.35), Stroke::new(1.6, c)));
            }
            if let Some(pe) = proj(eff) {
                let c = if hot_e { HILITE } else { IK_COL };
                let r = Rect::from_center_size(pe, egui::vec2(15.0, 15.0));
                painter.rect_filled(r, 2.0, c.gamma_multiply(0.30));
                painter.rect_stroke(r, 2.0, Stroke::new(2.0, c), egui::StrokeKind::Inside);
                painter.text(pe + egui::vec2(0.0, -16.0), egui::Align2::CENTER_CENTER, "IK", egui::FontId::proportional(10.5), c);
            }
        }

        // ---- quadril (ferramenta IK) e alvo do olhar
        if tool == PoseTool::Ik {
            if let Some(ph) = proj(pv.joints[bone::HIPS]) {
                let hot = self.pose_ui.hover_hot == Some(Hot::Hips) || matches!(&self.pose_ui.drag, Some(PoseDrag { kind: DragKind::Hips { .. }, .. }));
                let c = if hot { HILITE } else { Color32::from_rgb(0xb0, 0x90, 0xff) };
                painter.add(egui::Shape::convex_polygon(vec![ph + egui::vec2(0.0, -11.0), ph + egui::vec2(11.0, 0.0), ph + egui::vec2(0.0, 11.0), ph + egui::vec2(-11.0, 0.0)], c.gamma_multiply(0.30), Stroke::new(2.0, c)));
                painter.text(ph + egui::vec2(0.0, 21.0), egui::Align2::CENTER_CENTER, tr("pose.hud.hips"), egui::FontId::proportional(10.5), c);
            }
            if self.pose_ui.spine_ik {
                if let Some(ph) = proj(pv.joints[bone::HEAD]) {
                    let hot = self.pose_ui.hover_hot == Some(Hot::Head) || matches!(&self.pose_ui.drag, Some(PoseDrag { kind: DragKind::Head { .. }, .. }));
                    let c = if hot { HILITE } else { IK_COL };
                    let r = Rect::from_center_size(ph, egui::vec2(15.0, 15.0));
                    painter.rect_filled(r, 2.0, c.gamma_multiply(0.30));
                    painter.rect_stroke(r, 2.0, Stroke::new(2.0, c), egui::StrokeKind::Inside);
                    painter.text(ph + egui::vec2(0.0, -16.0), egui::Align2::CENTER_CENTER, "IK", egui::FontId::proportional(10.5), c);
                }
            }
            if self.pose_ui.gaze.enabled {
                let g = self.gaze_world(pv);
                let eye = pv.to_world(pose_ik::eye_center(&pv.rig, &pv.pose));
                if let (Some(pg), Some(pe)) = (proj(g), proj(eye)) {
                    painter.extend(egui::Shape::dashed_line(&[pe, pg], Stroke::new(1.4, Color32::from_rgb(0x70, 0xd8, 0xff)), 6.0, 4.0));
                    let hot = self.pose_ui.hover_hot == Some(Hot::Gaze) || matches!(&self.pose_ui.drag, Some(PoseDrag { kind: DragKind::Gaze { .. }, .. }));
                    let c = if hot { HILITE } else { Color32::from_rgb(0x70, 0xd8, 0xff) };
                    painter.circle_stroke(pg, 9.0, Stroke::new(2.0, c));
                    painter.circle_filled(pg, 3.0, c);
                    painter.text(pg + egui::vec2(0.0, 18.0), egui::Align2::CENTER_CENTER, icons::EYE, egui::FontId::proportional(14.0), c);
                }
            }
        }

        // ---- nome do osso sob o cursor / ativo
        if self.pose_ui.show_labels {
            for b in [self.pose_ui.hover_bone, self.pose_ui.active].into_iter().flatten() {
                if let Some(p) = proj(pv.joints[b]) {
                    let key = format!("pose.bone.{}", BONE_NAMES[b]);
                    let name = tr(&key);
                    let g = painter.layout_no_wrap(name.to_string(), egui::FontId::proportional(11.5), Color32::WHITE);
                    let r = Rect::from_min_size(p + egui::vec2(10.0, -22.0), g.size()).expand2(egui::vec2(5.0, 2.0));
                    painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(20, 20, 26, 220));
                    painter.galley(r.min + egui::vec2(5.0, 2.0), g, Color32::WHITE);
                }
            }
        }

        // ---- marquee
        if let Some(m) = &self.pose_ui.marquee {
            if (m.cur - m.start).length() > 3.0 {
                let r = Rect::from_two_pos(to(m.start), to(m.cur));
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 40));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
        }

        // ---- eixos do mundo (canto)
        let origin = rect.left_bottom() + egui::vec2(46.0, -46.0);
        for (i, a) in Axis3::ALL.iter().enumerate() {
            let d = a.unit();
            let sx = d.dot(view.right);
            let sy = -d.dot(view.up);
            let tip = origin + egui::vec2(sx, sy) * 26.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, AXIS_COLORS[i]));
            painter.text(tip + egui::vec2(sx, sy) * 7.0, egui::Align2::CENTER_CENTER, ["X", "Y", "Z"][i], egui::FontId::proportional(11.0), AXIS_COLORS[i]);
        }
    }

    fn draw_skeleton(&self, painter: &egui::Painter, view: &CamView, pv: &PoseView, proj: &dyn Fn(Vec3) -> Option<Pos2>) {
        let _ = view;
        let draw_bone = |a: Vec3, b: Vec3, color: Color32, hot: bool| {
            let (Some(pa), Some(pb)) = (proj(a), proj(b)) else { return };
            let v = pb - pa;
            let len = v.length();
            if len < 1.5 {
                return;
            }
            let dir = v / len;
            let perp = egui::vec2(-dir.y, dir.x);
            let w = (len * 0.11).clamp(2.0, 7.0);
            let mid = pa + v * 0.18;
            let fill = color.gamma_multiply(if hot { 0.55 } else { 0.30 });
            painter.add(egui::Shape::convex_polygon(vec![pa, mid + perp * w, pb, mid - perp * w], fill, Stroke::new(1.4, color)));
        };
        // segmentos pai→filho
        for b in 2..BONE_COUNT {
            let Some(p) = BONE_PARENTS[b] else { continue };
            if p == bone::ROOT {
                continue;
            }
            // o osso é desenhado do pai ao filho e herda a cor do FILHO (a junta que ele leva)
            let color = self.bone_color(b);
            draw_bone(pv.joints[p], pv.joints[b], color, self.pose_ui.selection.contains(&b));
        }
        // pontas: mão, pé, cabeça
        for (b, dir, len) in [(bone::L_WRIST, bone::L_ELBOW, 0.09f32), (bone::R_WRIST, bone::R_ELBOW, 0.09), (bone::HEAD, bone::NECK, 0.2)] {
            let d = (pv.joints[b] - pv.joints[dir]).normalize_or_zero();
            draw_bone(pv.joints[b], pv.joints[b] + d * len, self.bone_color(b), self.pose_ui.selection.contains(&b));
        }
        for (b, an) in [(bone::L_TOES, bone::L_ANKLE), (bone::R_TOES, bone::R_ANKLE)] {
            let d = (pv.joints[b] - pv.joints[an]).normalize_or_zero();
            draw_bone(pv.joints[b], pv.joints[b] + d * 0.06, self.bone_color(b), self.pose_ui.selection.contains(&b));
        }
        // juntas
        for b in 1..BONE_COUNT {
            let Some(p) = proj(pv.joints[b]) else { continue };
            let sel = self.pose_ui.selection.contains(&b);
            let active = self.pose_ui.active == Some(b);
            let hover = self.pose_ui.hover_bone == Some(b);
            let c = self.bone_color(b);
            let r = if active { 6.5 } else if sel || hover { 5.5 } else { 3.6 };
            painter.circle_filled(p, r, if sel || hover { c } else { c.gamma_multiply(0.85) });
            painter.circle_stroke(p, r + 0.5, Stroke::new(1.0, Color32::from_rgba_unmultiplied(10, 10, 14, 220)));
        }
        // ossos "posados": pequeno ponto na junta indica rotação diferente do repouso
        for b in pv.pose.posed_bones() {
            if let Some(p) = proj(pv.joints[b]) {
                painter.circle_filled(p + egui::vec2(7.0, -7.0), 2.0, Color32::from_rgb(0xff, 0xb0, 0x40));
            }
        }
    }

    fn draw_balance(&self, painter: &egui::Painter, _view: &CamView, pv: &PoseView, proj: &dyn Fn(Vec3) -> Option<Pos2>) {
        let (com, hull, inside) = anigo_core::pose::balance(&pv.rig, &pv.pose, 0.03);
        let col = if inside { OK_COL } else { BAD_COL };
        let floor_y = pv.rig.lowest_contact_y(&pv.pose);
        let pts: Vec<Pos2> = hull.iter().filter_map(|p| proj(pv.to_world(Vec3::new(p[0], floor_y, p[1])))).collect();
        if pts.len() >= 3 {
            painter.add(egui::Shape::convex_polygon(pts.clone(), col.gamma_multiply(0.18), Stroke::new(1.6, col)));
        } else if pts.len() == 2 {
            painter.line_segment([pts[0], pts[1]], Stroke::new(2.0, col));
        }
        let ground = Vec3::new(com.x, floor_y, com.z);
        if let (Some(pc), Some(pg)) = (proj(pv.to_world(com)), proj(pv.to_world(ground))) {
            painter.extend(egui::Shape::dashed_line(&[pc, pg], Stroke::new(1.2, col), 4.0, 3.0));
            painter.circle_filled(pg, 4.0, col);
            painter.circle_stroke(pc, 6.0, Stroke::new(2.0, col));
            painter.circle_filled(pc, 2.0, col);
            painter.text(pc + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, if inside { tr("pose.balance.ok") } else { tr("pose.balance.out") }, egui::FontId::proportional(11.5), col);
        }
    }

    // ===================================================================== barras

    fn posing_header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let header = Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(rect.width() - 12.0, 30.0));
        let bounds = self.gpu.as_ref().and_then(|g| g.cel.as_ref()).and_then(|c| c.character_bounds);
        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(24, 24, 30, 215)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let cam = &mut self.pose_ui.camera;
                    let views: [(&str, f32); 3] = [("char.view_front", 0.0), ("char.view_side", std::f32::consts::FRAC_PI_2), ("char.view_back", std::f32::consts::PI)];
                    for (key, yaw) in views {
                        if ui.button(tr(key)).clicked() {
                            cam.yaw = yaw;
                            cam.pitch = 0.05;
                            if let Some((lo, hi)) = bounds {
                                super::super::character_ui::frame_character(cam, lo, hi);
                            }
                        }
                    }
                    if ui.button(tr("char.frame")).clicked() {
                        if let Some((lo, hi)) = bounds {
                            super::super::character_ui::frame_character(cam, lo, hi);
                        }
                    }
                    ui.separator();
                    ui.toggle_value(&mut self.pose_ui.show_skeleton, format!("{}  {}", icons::BONE, tr("pose.view.skeleton"))).on_hover_text(tr("pose.view.skeleton_tip"));
                    ui.toggle_value(&mut self.pose_ui.show_mesh, format!("{}  {}", icons::PERSON, tr("pose.view.mesh")));
                    ui.toggle_value(&mut self.pose_ui.show_limits, format!("{}  {}", icons::GAUGE, tr("pose.view.limits"))).on_hover_text(tr("pose.view.limits_tip"));
                    ui.toggle_value(&mut self.pose_ui.show_balance, format!("{}  {}", icons::SCALES, tr("pose.view.balance"))).on_hover_text(tr("pose.view.balance_tip"));
                    ui.separator();
                    ui.selectable_value(&mut self.pose_ui.space_local, false, tr("pose.space.world"));
                    ui.selectable_value(&mut self.pose_ui.space_local, true, tr("pose.space.local"));
                    ui.separator();
                    ui.toggle_value(&mut self.pose_ui.symmetry, format!("{}  {}", icons::FLIP_HORIZONTAL, tr("pose.symmetry"))).on_hover_text(tr("pose.symmetry_tip"));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(tr("pose.snap")).color(theme::TEXT_DIM));
                        for (label, v) in [("off", None), ("5\u{b0}", Some(5.0f32)), ("15\u{b0}", Some(15.0)), ("45\u{b0}", Some(45.0))] {
                            let text = if v.is_none() { tr("pose.snap_off").to_string() } else { label.to_string() };
                            ui.selectable_value(&mut self.pose_ui.snap_deg, v, text);
                        }
                    });
                });
            });
        });
    }

    // ===================================================================== trace (smoke)

    /// `ANIGO_TRACE_POSING=1`: uma linha `posetrace ...` por mudança de estado, para o smoke de xdotool.
    fn posing_trace(&mut self, ctx: &egui::Context, rect: Rect, view: &CamView, pv: &PoseView) {
        if !self.pose_ui.trace {
            return;
        }
        let ppp = ctx.pixels_per_point();
        // posição de janela em pixels de uma coordenada local (pontos) do viewport
        let px = |p: Vec2| format!("{:.0},{:.0}", (rect.min.x + p.x) * ppp, (rect.min.y + p.y) * ppp);
        let mut s = format!(
            "posetrace sel={} active={} undo={} redo={} prev={} drag={} hover={} script={} limits={:?} ik={} sym={} rest={} rects={:.0},{:.0},{:.0},{:.0}",
            if self.pose_ui.selection.is_empty() { "-".to_string() } else { self.pose_ui.selection.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",") },
            self.pose_ui.active.map(|b| BONE_NAMES[b]).unwrap_or("-"),
            self.core.history().undo_depth(),
            self.core.history().redo_depth(),
            self.core.is_previewing(),
            self.pose_ui.drag.is_some(),
            self.pose_ui.hover_hot.map(|h| format!("{h:?}")).unwrap_or_else(|| "-".into()),
            match &self.pose_ui.script {
                Some(sc) if sc.is_done() => "done",
                Some(_) => "run",
                None => "none",
            },
            self.pose_ui.profile,
            self.pose_ui.limb_ik.iter().map(|b| if *b { '1' } else { '0' }).collect::<String>(),
            self.pose_ui.symmetry,
            pv.pose.is_rest(),
            rect.min.x * ppp,
            rect.min.y * ppp,
            rect.width() * ppp,
            rect.height() * ppp,
        );
        for b in 1..BONE_COUNT {
            if let Some((p, _)) = view.project(pv.joints[b]) {
                s.push_str(&format!(" j{}={}", b, px(p)));
            }
        }
        for limb in Limb::ALL {
            if let Some((p, _)) = view.project(pv.joints[limb.chain()[2]]) {
                s.push_str(&format!(" eff{}={}", limb_idx(limb), px(p)));
            }
        }
        if let Some(f) = self.fk_frame(view, pv) {
            for a in Axis3::ALL {
                let pts = f.ring_points(a, 64);
                if let Some((p, _)) = view.project(pts[8]) {
                    s.push_str(&format!(" ring{}={}", a.index(), px(p)));
                }
            }
        }
        let pose_sig: Vec<String> = pv.pose.rotations.iter().map(|(k, q)| format!("{k}:{:.3},{:.3},{:.3},{:.3}", q[0], q[1], q[2], q[3])).collect();
        let h = pv.pose.hips_translation;
        s.push_str(&format!(" pose=[{}] hips={:.3},{:.3},{:.3}", pose_sig.join(";"), h[0], h[1], h[2]));
        let ankle = |b: usize| format!("{:.3},{:.3},{:.3}", pv.joints_char[b].x, pv.joints_char[b].y, pv.joints_char[b].z);
        s.push_str(&format!(" lankle={} rankle={} lwrist={}", ankle(bone::L_ANKLE), ankle(bone::R_ANKLE), ankle(bone::L_WRIST)));
        if s != self.pose_ui.last_trace {
            eprintln!("{s}");
            self.pose_ui.last_trace = s;
        }
    }
}

fn limb_idx(l: Limb) -> usize {
    super::limb_index(l)
}

fn lock_axis_fn(v: Vec3, lock: Option<usize>) -> Vec3 {
    match lock {
        Some(i) => {
            let mut out = Vec3::ZERO;
            out[i] = v[i];
            out
        }
        None => v,
    }
}

