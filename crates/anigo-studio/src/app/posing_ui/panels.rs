//! Painéis do Posing (direita): árvore de ossos + osso ativo + ações (ferramenta "Estrutura óssea"),
//! IK/olhar/pés/equilíbrio (ferramenta "Cinemática inversa") e presets/mãos/Biblioteca de poses.

use super::viewport::PoseTool;
use super::{describe_pose, HandSide, StudioApp};
use crate::i18n::tr;
use crate::icons;
use crate::posing::edit;
use crate::posing::limits_vis::swing_twist_deg;
use crate::theme;
use anigo_core::pose::{bone, bone_limit, mirror_bone, quat_to_euler_deg, Limb, LimitProfile, PoseOp, PoseState, BONE_COUNT, BONE_NAMES, BONE_PARENTS};
use anigo_ik::pose_ik;
use egui::{Color32, RichText, Sense, Stroke};
use glam::Vec3;

impl StudioApp {
    pub(crate) fn posing_panels(&mut self, ui: &mut egui::Ui) {
        if self.core.history().limit() < 400 {
            self.core.set_history_limit(400);
        }
        egui::Panel::right("pose_inspector").default_size(360.0).size_range(300.0..=560.0).show(ui, |ui| {
            ui.add_space(6.0);
            let tool = self.posing_tool();
            let (title, icon) = match tool {
                PoseTool::Rig => ("tool.rig", icons::BONE),
                PoseTool::Ik => ("tool.ik", icons::TARGET),
                PoseTool::Presets => ("tool.poses_preset", icons::PERSON_SIMPLE_RUN),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{icon}  {}", tr(title))).strong().size(14.0));
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match tool {
                    PoseTool::Rig => self.rig_panel(ui),
                    PoseTool::Ik => self.ik_panel(ui),
                    PoseTool::Presets => self.presets_panel(ui),
                }
                ui.add_space(8.0);
                self.pose_history_section(ui);
            });
        });
    }

    // ============================================================ ferramenta: estrutura óssea

    pub(crate) fn rig_panel(&mut self, ui: &mut egui::Ui) {
        self.pose_actions_row(ui);
        ui.add_space(4.0);
        self.limits_row(ui);
        ui.add_space(6.0);
        if let Some(b) = self.pose_ui.active {
            egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::SLIDERS, tr(&format!("pose.bone.{}", BONE_NAMES[b])))).strong()).default_open(true).show(ui, |ui| {
                self.bone_panel(ui, b);
            });
        } else {
            ui.label(RichText::new(tr("pose.select_hint")).color(theme::TEXT_DIM));
        }
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::BONE, tr("pose.tree"))).strong()).default_open(true).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("pose_bone_tree").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                self.bone_tree(ui);
            });
        });
    }

    fn pose_actions_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::FLIP_HORIZONTAL, tr("pose.mirror"))).on_hover_text(tr("pose.mirror_tip")).clicked() {
                self.pose_mirror();
            }
            if ui.button(format!("{}  {}", icons::COPY, tr("pose.copy"))).on_hover_text(tr("pose.copy_tip")).clicked() {
                self.pose_copy();
            }
            let has = self.pose_ui.clipboard.is_some();
            if ui.add_enabled(has, egui::Button::new(format!("{}  {}", icons::CLIPBOARD, tr("pose.paste")))).on_hover_text(tr("pose.paste_tip")).clicked() {
                self.pose_paste(false);
            }
            if ui.add_enabled(has, egui::Button::new(tr("pose.paste_flipped"))).clicked() {
                self.pose_paste(true);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("pose.reset_selection"))).on_hover_text(tr("pose.reset_selection_tip")).clicked() {
                self.pose_reset_selection();
            }
            if ui.button(format!("{}  {}", icons::ERASER, tr("pose.reset_all"))).clicked() {
                self.pose_reset_all();
            }
        });
    }

    fn limits_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("pose.limits")).color(theme::TEXT_DIM));
            for (p, key) in [(LimitProfile::Human, "pose.limits.human"), (LimitProfile::Anime, "pose.limits.anime"), (LimitProfile::Free, "pose.limits.free")] {
                ui.selectable_value(&mut self.pose_ui.profile, p, tr(key));
            }
        })
        .response
        .on_hover_text(tr("pose.limits_tip"));
    }

    fn bone_tree(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.pose_ui.tree_filter).hint_text(tr("pose.tree.filter")).desired_width(f32::INFINITY));
        let filter = self.pose_ui.tree_filter.to_lowercase();
        let posed: std::collections::BTreeSet<usize> = self.core.pose().posed_bones().into_iter().collect();
        for b in 1..BONE_COUNT {
            let name = tr(&format!("pose.bone.{}", BONE_NAMES[b])).to_string();
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !BONE_NAMES[b].to_lowercase().contains(&filter) {
                continue;
            }
            let mut depth = 0usize;
            let mut p = BONE_PARENTS[b];
            while let Some(pi) = p {
                if pi != bone::ROOT {
                    depth += 1;
                }
                p = BONE_PARENTS[pi];
            }
            let depth = depth.saturating_sub(1);
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                let sel = self.pose_ui.selection.contains(&b);
                let side_col = if anigo_core::pose::is_left_bone(b) {
                    Color32::from_rgb(0x5a, 0xa8, 0xff)
                } else if anigo_core::pose::is_right_bone(b) {
                    Color32::from_rgb(0xff, 0x6a, 0x6a)
                } else {
                    theme::TEXT_DIM
                };
                let mut text = RichText::new(format!("{}  {}", if posed.contains(&b) { icons::CIRCLE } else { icons::DOT_OUTLINE }, name));
                if sel {
                    text = text.color(theme::SELECT).strong();
                } else if posed.contains(&b) {
                    text = text.color(Color32::from_rgb(0xff, 0xc0, 0x60));
                } else {
                    text = text.color(side_col);
                }
                let r = ui.selectable_label(sel, text);
                if r.clicked() {
                    let additive = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                    if additive {
                        self.pose_ui.toggle(b);
                    } else {
                        self.pose_ui.select_one(b);
                    }
                }
            });
        }
    }

    fn bone_panel(&mut self, ui: &mut egui::Ui, b: usize) {
        let pose = self.pose_now();
        let q = pose.rotation(b);
        let mut e = quat_to_euler_deg(q);
        let opts = self.pose_ui.edit_opts();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Euler XYZ").color(theme::TEXT_DIM));
            let mut changed = false;
            for (i, c) in ["X", "Y", "Z"].iter().enumerate() {
                ui.label(*c);
                let r = ui.add(egui::DragValue::new(&mut e[i]).speed(0.5).suffix("\u{b0}").fixed_decimals(1).range(-360.0..=360.0));
                changed |= r.changed();
            }
            if changed {
                let target = edit::set_bone_euler(&pose, b, e, &opts);
                self.pose_live(target, PoseOp::Edit);
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Quat").color(theme::TEXT_DIM));
            ui.label(RichText::new(format!("{:+.3} {:+.3} {:+.3} {:+.3}", q.x, q.y, q.z, q.w)).monospace().size(11.5));
        });
        if b == bone::HIPS {
            let mut t = pose.hips_translation;
            if self.translation_row(ui, tr("pose.translation"), &mut t) {
                let mut target = pose.clone();
                target.hips_translation = t;
                self.pose_live(target, PoseOp::Edit);
            }
        }
        // barras de limite
        match bone_limit(b, self.pose_ui.profile) {
            Some(l) => {
                let v = swing_twist_deg(&pose, b);
                for (label_key, val, range) in [("pose.swing_x", v[0], l.sx), ("pose.swing_z", v[1], l.sz), ("pose.twist", v[2], l.twist)] {
                    limit_bar(ui, tr(label_key), val, range);
                }
            }
            None => {
                ui.label(RichText::new(tr("pose.no_limit")).color(theme::TEXT_DIM).size(11.5));
            }
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("pose.reset_bone"))).clicked() {
                self.pose_reset_bone(b);
            }
            if self.pose_ui.symmetry && mirror_bone(b) != b {
                ui.label(RichText::new(format!("{} {}", icons::FLIP_HORIZONTAL, tr(&format!("pose.bone.{}", BONE_NAMES[mirror_bone(b)])))).color(theme::TEXT_DIM).size(11.5));
            }
        });
    }

    /// Linha de 3 `DragValue` (metros). Devolve `true` quando algum valor mudou.
    fn translation_row(&mut self, ui: &mut egui::Ui, label: &str, t: &mut [f32; 3]) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(theme::TEXT_DIM));
            for (i, c) in ["X", "Y", "Z"].iter().enumerate() {
                ui.label(*c);
                changed |= ui.add(egui::DragValue::new(&mut t[i]).speed(0.005).suffix(" m").fixed_decimals(3).range(-10.0..=10.0)).changed();
            }
        });
        changed
    }

    // ============================================================ ferramenta: IK

    fn ik_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("pose.ik.hint")).color(theme::TEXT_DIM).size(11.5));
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::LINK, tr("pose.ik.limbs"))).strong()).default_open(true).show(ui, |ui| {
            for limb in Limb::ALL {
                ui.horizontal(|ui| {
                    ui.label(format!("{:<14}", tr(&format!("pose.limb.{}", limb.key()))));
                    let i = super::limb_index(limb);
                    if ui.selectable_label(!self.pose_ui.limb_ik[i], "FK").clicked() {
                        self.pose_ui.limb_ik[i] = false;
                    }
                    if ui.selectable_label(self.pose_ui.limb_ik[i], "IK").clicked() {
                        self.pose_ui.limb_ik[i] = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.button(tr("pose.ik.all")).clicked() {
                    self.pose_ui.limb_ik = [true; 4];
                }
                if ui.button(tr("pose.ik.none")).clicked() {
                    self.pose_ui.limb_ik = [false; 4];
                }
            });
            ui.checkbox(&mut self.pose_ui.spine_ik, tr("pose.ik.spine")).on_hover_text(tr("pose.ik.spine_tip"));
            ui.label(RichText::new(tr("pose.ik.drag_hint")).color(theme::TEXT_DIM).size(11.0));
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::FOOTPRINTS, tr("pose.feet"))).strong()).default_open(true).show(ui, |ui| {
            ui.checkbox(&mut self.pose_ui.foot_lock, tr("pose.foot_lock")).on_hover_text(tr("pose.foot_lock_tip"));
            ui.checkbox(&mut self.pose_ui.auto_ground, tr("pose.auto_ground")).on_hover_text(tr("pose.auto_ground_tip"));
            if ui.button(format!("{}  {}", icons::ARROW_LINE_DOWN, tr("pose.ground_now"))).clicked() {
                self.pose_ground_now();
            }
            let pose = self.pose_now();
            let mut h = pose.hips_translation;
            if self.translation_row(ui, tr("pose.hips"), &mut h) {
                let rig = self.pose_rig();
                let mv = edit::HipsMove { delta: Vec3::from_array(h) - Vec3::from_array(pose.hips_translation), whole_body: false, foot_lock: self.pose_ui.foot_lock, auto_ground: self.pose_ui.auto_ground };
                let target = edit::move_hips(&rig, &pose, mv, &self.pose_ui.edit_opts());
                self.pose_live(target, PoseOp::Edit);
            }
            let mut r = pose.root_translation;
            if self.translation_row(ui, tr("pose.root"), &mut r) {
                let mut target = pose.clone();
                target.root_translation = r;
                self.pose_live(target, PoseOp::Edit);
            }
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::EYE, tr("pose.gaze"))).strong()).default_open(true).show(ui, |ui| {
            ui.checkbox(&mut self.pose_ui.gaze.enabled, tr("pose.gaze.show")).on_hover_text(tr("pose.gaze.tip"));
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("pose.gaze.weight")).color(theme::TEXT_DIM));
                ui.add(egui::Slider::new(&mut self.pose_ui.gaze.weight, 0.0..=1.0).fixed_decimals(2));
            });
            ui.horizontal(|ui| {
                if ui.button(format!("{}  {}", icons::CROSSHAIR, tr("pose.gaze.apply"))).clicked() {
                    let rig = self.pose_rig();
                    let t = pose_ik::look_at_head(&rig, &self.pose_now(), self.pose_ui.gaze.target, self.pose_ui.gaze.weight, 0.35, self.pose_ui.profile).pose;
                    self.pose_commit_to(t, PoseOp::Gaze);
                }
                if ui.button(tr("pose.gaze.front")).clicked() {
                    let rig = self.pose_rig();
                    let eye = pose_ik::eye_center(&rig, &self.pose_now());
                    let f = pose_ik::face_direction(&rig, &self.pose_now());
                    self.pose_ui.gaze.target = eye + f * 1.0;
                    self.pose_ui.gaze.enabled = true;
                }
                if ui.button(tr("pose.gaze.center")).clicked() {
                    let mut t = self.pose_now();
                    t.set_rotation(bone::NECK, glam::Quat::IDENTITY);
                    t.set_rotation(bone::HEAD, glam::Quat::IDENTITY);
                    self.pose_commit_to(t, PoseOp::Gaze);
                }
            });
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::SCALES, tr("pose.balance"))).strong()).default_open(true).show(ui, |ui| {
            ui.checkbox(&mut self.pose_ui.show_balance, tr("pose.balance.show"));
            let rig = self.pose_rig();
            let (com, _hull, inside) = anigo_core::pose::balance(&rig, &self.pose_now(), 0.03);
            let (txt, col) = if inside { (tr("pose.balance.ok"), Color32::from_rgb(0x58, 0xd0, 0x7a)) } else { (tr("pose.balance.out"), Color32::from_rgb(0xff, 0x5a, 0x4a)) };
            ui.label(RichText::new(format!("{txt}   CoM ({:.2}, {:.2}, {:.2})", com.x, com.y, com.z)).color(col).size(12.0));
            ui.label(RichText::new(tr("pose.balance.note")).color(theme::TEXT_DIM).size(11.0));
        });
    }

    // ============================================================ ferramenta: presets e biblioteca

    fn presets_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::PERSON, tr("pose.presets.body"))).strong()).default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (key, icon) in [("t", icons::PERSON_ARMS_SPREAD), ("a", icons::PERSON), ("standing", icons::PERSON_SIMPLE), ("sitting", icons::CHAIR), ("walking", icons::PERSON_SIMPLE_WALK), ("action", icons::PERSON_SIMPLE_RUN)] {
                    if ui.button(format!("{icon}  {}", tr(&format!("pose.preset.{key}")))).clicked() {
                        self.pose_apply_body_preset(key);
                    }
                }
            });
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::HAND, tr("pose.presets.hands"))).strong()).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.pose_ui.hand_side, HandSide::Left, tr("pose.hand.left"));
                ui.selectable_value(&mut self.pose_ui.hand_side, HandSide::Right, tr("pose.hand.right"));
                ui.selectable_value(&mut self.pose_ui.hand_side, HandSide::Both, tr("pose.hand.both"));
            });
            ui.horizontal_wrapped(|ui| {
                for (key, icon) in [("open", icons::HAND_PALM), ("relaxed", icons::HAND), ("fist", icons::HAND_FIST), ("point", icons::HAND_POINTING), ("v", icons::HAND_PEACE), ("hold", icons::HAND_GRABBING)] {
                    if ui.button(format!("{icon}  {}", tr(&format!("pose.hand.{key}")))).clicked() {
                        self.pose_apply_hand_preset(key);
                    }
                }
            });
            let pose = self.pose_now();
            let side = self.pose_ui.hand_side;
            let mut curls = if side == HandSide::Right { pose.hands.right } else { pose.hands.left };
            let mut changed = false;
            for (i, key) in ["pose.finger.thumb", "pose.finger.index", "pose.finger.middle", "pose.finger.ring", "pose.finger.little"].iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:<9}", tr(key))).color(theme::TEXT_DIM));
                    changed |= ui.add(egui::Slider::new(&mut curls[i], 0.0..=1.0).fixed_decimals(2)).changed();
                });
            }
            if changed {
                let mut t = pose.clone();
                if matches!(side, HandSide::Left | HandSide::Both) {
                    t.hands.left = curls;
                }
                if matches!(side, HandSide::Right | HandSide::Both) {
                    t.hands.right = curls;
                }
                self.pose_live(t, PoseOp::Hand);
            }
            ui.label(RichText::new(tr("pose.hand.note")).color(theme::TEXT_DIM).size(11.0));
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::STACK, tr("pose.lib"))).strong()).default_open(true).show(ui, |ui| {
            self.library_section(ui);
        });
    }

    fn library_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.pose_ui.lib_name).hint_text(tr("pose.lib.name")).desired_width(170.0));
            if ui.button(format!("{}  {}", icons::FLOPPY_DISK, tr("pose.lib.save"))).on_hover_text(tr("pose.lib.save_tip")).clicked() {
                self.pose_save_to_library();
            }
        });
        ui.checkbox(&mut self.pose_ui.lib_only_selected, tr("pose.lib.only_selected"));
        if !self.pose_ui.preset_status.is_empty() {
            ui.label(RichText::new(self.pose_ui.preset_status.clone()).color(theme::TEXT_DIM).size(11.5));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ({})", tr("pose.lib.assets"), self.pose_ui.poses.len())).strong());
            if ui.button(icons::ARROWS_CLOCKWISE).on_hover_text(tr("pose.lib.refresh")).clicked() || !self.pose_ui.poses_loaded {
                self.pose_refresh_library();
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("pose.lib.blend")).color(theme::TEXT_DIM));
            ui.add(egui::Slider::new(&mut self.pose_ui.blend, 0.0..=1.5).fixed_decimals(2).suffix(""));
        });
        ui.checkbox(&mut self.pose_ui.apply_only_selected, tr("pose.lib.apply_selected"));
        if self.pose_ui.poses.is_empty() {
            ui.label(RichText::new(tr("pose.lib.empty")).color(theme::TEXT_DIM).size(11.5));
        }
        let cards = self.pose_ui.poses.clone();
        for (i, c) in cards.iter().enumerate() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(44.0, 56.0), Sense::hover());
                stick_figure(ui.painter(), r, &c.asset.pose);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&c.name).strong());
                    ui.label(RichText::new(format!("rev {} \u{b7} {} {}", c.revision, c.asset.mask().len(), tr("pose.lib.bones"))).color(theme::TEXT_DIM).size(11.0));
                    ui.horizontal(|ui| {
                        if ui.small_button(tr("pose.lib.apply")).clicked() {
                            self.pose_apply_asset(i, false);
                        }
                        if ui.small_button(tr("pose.lib.apply_flipped")).clicked() {
                            self.pose_apply_asset(i, true);
                        }
                    });
                });
            });
        }
    }

    // ============================================================ histórico

    fn pose_history_section(&mut self, ui: &mut egui::Ui) {
        let undo: Vec<String> = self.core.history().undo_descriptions().iter().map(|s| s.to_string()).collect();
        let redo: Vec<String> = self.core.history().redo_descriptions().iter().map(|s| s.to_string()).collect();
        egui::CollapsingHeader::new(RichText::new(format!("{}  {} ({} / {})", icons::CLOCK_COUNTER_CLOCKWISE, tr("pose.history"), undo.len(), redo.len())).strong()).default_open(false).show(ui, |ui| {
            let mut jump: Option<usize> = None;
            let r0 = ui.selectable_label(undo.is_empty(), RichText::new(tr("pose.history.initial")));
            if r0.clicked() {
                jump = Some(0);
            }
            for (i, d) in undo.iter().enumerate() {
                let last = i + 1 == undo.len();
                if ui.selectable_label(last, describe_pose(d)).clicked() {
                    jump = Some(i + 1);
                }
            }
            for (i, d) in redo.iter().enumerate() {
                if ui.selectable_label(false, RichText::new(describe_pose(d)).color(theme::TEXT_DIM)).clicked() {
                    jump = Some(undo.len() + i + 1);
                }
            }
            if let Some(depth) = jump {
                self.pose_reset_transients();
                if let Err(e) = self.core.jump_to_depth(depth) {
                    self.pose_report(e);
                }
            }
        });
    }
}

/// Barra de limite: faixa permitida em verde, marcador do valor atual (vermelho se fora).
fn limit_bar(ui: &mut egui::Ui, label: &str, value: f32, range: [f32; 2]) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{label:<8}")).color(theme::TEXT_DIM).size(11.5));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(210.0) - 54.0, 14.0), Sense::hover());
        let span = (range[1] - range[0]).max(1.0);
        let lo = range[0] - span * 0.15;
        let hi = range[1] + span * 0.15;
        let x = |v: f32| rect.min.x + (v - lo) / (hi - lo) * rect.width();
        let p = ui.painter();
        p.rect_filled(rect, 3.0, Color32::from_rgb(0x24, 0x24, 0x2c));
        let ok = egui::Rect::from_min_max(egui::pos2(x(range[0]), rect.min.y), egui::pos2(x(range[1]), rect.max.y));
        p.rect_filled(ok, 3.0, Color32::from_rgba_unmultiplied(0x58, 0xd0, 0x7a, 70));
        let zero = x(0.0);
        p.line_segment([egui::pos2(zero, rect.min.y), egui::pos2(zero, rect.max.y)], Stroke::new(1.0, Color32::from_gray(110)));
        let inside = value >= range[0] - 0.05 && value <= range[1] + 0.05;
        let col = if inside { Color32::WHITE } else { Color32::from_rgb(0xff, 0x5a, 0x4a) };
        let vx = x(value.clamp(lo, hi));
        p.line_segment([egui::pos2(vx, rect.min.y - 1.0), egui::pos2(vx, rect.max.y + 1.0)], Stroke::new(2.4, col));
        ui.label(RichText::new(format!("{value:+.0}\u{b0}")).monospace().size(11.0).color(col));
    });
}

/// Boneco de palitos de uma pose (miniatura leve da lista de poses da Biblioteca).
fn stick_figure(painter: &egui::Painter, rect: egui::Rect, pose: &PoseState) {
    let rig = anigo_core::pose::PoseRig::canonical(1.0);
    let j = rig.joint_positions(pose);
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::INFINITY), glam::Vec2::splat(f32::NEG_INFINITY));
    for p in j.iter().skip(1) {
        let v = glam::Vec2::new(p.x, p.y);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    hi.y += 0.2;
    let span = (hi - lo).max_element().max(0.5);
    let scale = rect.height().min(rect.width() * 1.3) * 0.92 / span;
    let c = (lo + hi) * 0.5;
    let to = |p: Vec3| egui::pos2(rect.center().x + (p.x - c.x) * scale, rect.center().y - (p.y - c.y) * scale);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(0x23, 0x23, 0x2a));
    for b in 2..BONE_COUNT {
        let Some(p) = BONE_PARENTS[b] else { continue };
        if p == bone::ROOT {
            continue;
        }
        let col = if anigo_core::pose::is_left_bone(b) {
            Color32::from_rgb(0x5a, 0xa8, 0xff)
        } else if anigo_core::pose::is_right_bone(b) {
            Color32::from_rgb(0xff, 0x6a, 0x6a)
        } else {
            Color32::from_rgb(0xd0, 0xd0, 0xdc)
        };
        painter.line_segment([to(j[p]), to(j[b])], Stroke::new(1.6, col));
    }
    painter.line_segment([to(j[bone::HEAD]), to(j[bone::HEAD] + Vec3::new(0.0, 0.2, 0.0))], Stroke::new(1.6, Color32::from_rgb(0xd0, 0xd0, 0xdc)));
}
