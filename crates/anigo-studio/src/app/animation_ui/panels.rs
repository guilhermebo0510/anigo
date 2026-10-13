//! Barra de transporte (topo do painel inferior) e inspetor (direita): clip, chave selecionada, onion skin, marcadores,
//! física secundária, arquivos (glTF/VRMA/BVH/clip), Biblioteca de clips e fila de render da sequência.

use super::super::props_tabs::{PropTab, TabSet};
use super::{describe_anim, AnimUi, InterpPick, SideTab, StudioApp};
use crate::anim::keys;
use crate::anim::secondary::{bake, SecondarySim};
use crate::anim::sequence::ClipFrameSource;
use crate::i18n::tr;
use crate::icons;
use crate::render_out::job::{FrameRange, JobSpec};
use crate::render_out::settings::{check_output_dir, NameTokens};
use crate::theme;
use anigo_core::animation::{timecode, AnimEdit, ChannelTarget, Extrapolation, Interp, Key, KeyInsert, KeyMove, KeyRef, KeyValue, LoopMode};
use anigo_core::bvh::{parse_bvh, write_bvh};
use anigo_core::clip_asset::{ClipAsset, CLIP_ASSET_SUFFIX};
use anigo_core::command::Command;
use anigo_core::ids::AnimationClipId;
use anigo_core::pose::{euler_deg_to_quat, quat_to_euler_deg};
use anigo_core::project::AnimationClip;
use anigo_core::retarget::{poses_to_clip, BoneMap, ClipBuild, Retargeter};
use egui::{Color32, RichText};
use std::sync::Arc;

/// Largura fixa dos campos do inspetor: larguras derivadas do espaço disponível realimentam o painel e o fazem crescer.
const FIELD_W: f32 = 300.0;
const FPS_PRESETS: [f32; 9] = [8.0, 12.0, 15.0, 24.0, 25.0, 30.0, 48.0, 50.0, 60.0];

impl StudioApp {
    /// Início do quadro da Animação: histórico profundo (como o Cenário).
    pub(crate) fn anim_frame_begin(&mut self) {
        if self.core.history().limit() < 400 {
            self.core.set_history_limit(400);
        }
    }

    /// Painel Inspetor da Animação (corpo): abas Clip/Osso.
    pub(crate) fn animation_inspector_body(&mut self, ui: &mut egui::Ui) {
        // a aba do Clipe/Osso continua sendo `anim_ui.side_tab` (outros trechos a leem): a faixa só a espelha
        let want = match self.anim_ui.side_tab {
            SideTab::Clip => PropTab::Clip,
            SideTab::Bone => PropTab::Bone,
        };
        self.set_prop_tab(TabSet::Animacao, want);
        let (tab, mut content) = self.tabbed(ui, TabSet::Animacao);
        self.anim_ui.side_tab = if tab == PropTab::Bone { SideTab::Bone } else { SideTab::Clip };
        let ui = &mut content;
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| match self.anim_ui.side_tab {
                SideTab::Clip => self.inspector_clip_tab(ui),
                SideTab::Bone => self.rig_panel(ui),
            });
        });
    }

    // ============================================================ barra de transporte

    pub(crate) fn anim_toolbar(&mut self, ui: &mut egui::Ui) {
        let span = self.anim_span();
        let fps = self.anim_fps();
        let clip = self.anim_clip().cloned();
        ui.horizontal_wrapped(|ui| {
            let b = |ui: &mut egui::Ui, icon: &str, tip: &str| ui.add(egui::Button::new(icon).min_size(egui::vec2(28.0, 24.0))).on_hover_text(tip).clicked();
            if b(ui, icons::SKIP_BACK, &format!("{} (Home)", tr("anim.t.start"))) {
                self.anim_ui.transport.home(span);
            }
            if b(ui, icons::CARET_DOUBLE_LEFT, &format!("{} (\u{2193})", tr("anim.t.prev_key"))) {
                if let Some(c) = &clip {
                    self.anim_ui.transport.jump_key(c, span, false);
                }
            }
            if b(ui, icons::CARET_LEFT, &format!("{} (\u{2190})", tr("anim.t.prev_frame"))) {
                self.anim_ui.transport.step(-1, span);
            }
            let playing = self.anim_ui.transport.playing;
            let play = ui.add(egui::Button::new(egui::RichText::new(if playing { icons::PAUSE } else { icons::PLAY }).size(16.0)).min_size(egui::vec2(36.0, 24.0)).selected(playing)).on_hover_text(format!("{} ({})", tr("anim.t.play"), tr("anim.t.space")));
            self.anim_ui.play_btn = play.rect.center();
            if play.clicked() {
                self.anim_ui.transport.toggle(span);
                self.anim_ui.phys.last_raw = None;
            }
            if b(ui, icons::CARET_RIGHT, &format!("{} (\u{2192})", tr("anim.t.next_frame"))) {
                self.anim_ui.transport.step(1, span);
            }
            if b(ui, icons::CARET_DOUBLE_RIGHT, &format!("{} (\u{2191})", tr("anim.t.next_key"))) {
                if let Some(c) = &clip {
                    self.anim_ui.transport.jump_key(c, span, true);
                }
            }
            if b(ui, icons::SKIP_FORWARD, &format!("{} (End)", tr("anim.t.end"))) {
                self.anim_ui.transport.end(span);
            }
            ui.separator();
            let mut f = self.anim_frame();
            if ui.add(egui::DragValue::new(&mut f).speed(0.25).range(0..=1_000_000).prefix(format!("{} ", tr("anim.t.frame")))).changed() {
                self.scrub_to(f as f32, false);
            }
            ui.label(RichText::new(timecode(self.anim_frame(), fps)).monospace().color(theme::TEXT_DIM));
            ui.label(RichText::new(format!("{fps:.0} fps")).color(theme::TEXT_DIM));
            ui.separator();
            // loop / ping-pong
            if let Some(id) = self.anim_clip_id() {
                let cur = clip.as_ref().map(|c| c.loop_mode).unwrap_or(LoopMode::Once);
                for (mode, icon, key) in [(LoopMode::Once, icons::ARROW_FAT_LINE_RIGHT, "anim.loop.once"), (LoopMode::Loop, icons::REPEAT, "anim.loop.loop"), (LoopMode::PingPong, icons::ARROWS_LEFT_RIGHT, "anim.loop.pingpong")] {
                    if ui.add(egui::Button::new(icon).selected(cur == mode).min_size(egui::vec2(26.0, 24.0))).on_hover_text(tr(key)).clicked() && cur != mode {
                        self.anim_apply(AnimEdit::SetClipPlayback { clip_id: id.clone(), loop_mode: Some(mode), extrapolation: None, hold_step: None });
                    }
                }
            }
            egui::ComboBox::from_id_salt("anim_speed").width(64.0).selected_text(format!("{}x", self.anim_ui.transport.speed)).show_ui(ui, |ui| {
                for s in [-1.0f32, 0.25, 0.5, 1.0, 2.0] {
                    if ui.selectable_label((self.anim_ui.transport.speed - s).abs() < 1e-3, format!("{s}x")).clicked() {
                        self.anim_ui.transport.set_speed(s);
                    }
                }
            })
            .response
            .on_hover_text(tr("anim.t.speed"));
            ui.separator();
            // animar em 1s/2s/3s (modo anime)
            if let Some(id) = self.anim_clip_id() {
                let cur = clip.as_ref().map(|c| c.hold_step).unwrap_or(1);
                ui.label(RichText::new(tr("anim.twos")).color(theme::TEXT_DIM)).on_hover_text(tr("anim.twos_tip"));
                for n in [1u8, 2, 3] {
                    if ui.add(egui::Button::new(format!("{n}s")).selected(cur == n).min_size(egui::vec2(26.0, 24.0))).on_hover_text(tr("anim.twos_tip")).clicked() && cur != n {
                        self.anim_apply(AnimEdit::SetClipPlayback { clip_id: id.clone(), loop_mode: None, extrapolation: None, hold_step: Some(n) });
                    }
                }
            }
        });
        // segunda linha: chaves, auto-key, interpolação, onion skin
        ui.horizontal_wrapped(|ui| {
            let key = ui.add(egui::Button::new(format!("{}  {}", icons::KEY, tr("anim.key.insert"))).min_size(egui::vec2(0.0, 24.0))).on_hover_text(format!("{} (I) · {} (Alt+I)", tr("anim.key.insert_tip"), tr("anim.key.delete_here")));
            self.anim_ui.key_btn = key.rect.center();
            if key.clicked() {
                self.anim_insert_key();
            }
            let auto = self.anim_ui.autokey;
            let rec = egui::RichText::new(icons::RECORD).color(if auto { Color32::from_rgb(0xff, 0x40, 0x40) } else { theme::TEXT_DIM });
            if ui.add(egui::Button::new(rec).selected(auto).min_size(egui::vec2(28.0, 24.0))).on_hover_text(tr("anim.autokey_tip")).clicked() {
                self.anim_ui.autokey = !auto;
            }
            egui::ComboBox::from_id_salt("anim_interp").width(88.0).selected_text(tr(self.anim_ui.interp.key())).show_ui(ui, |ui| {
                for p in InterpPick::ALL {
                    ui.selectable_value(&mut self.anim_ui.interp, p, tr(p.key()));
                }
            })
            .response
            .on_hover_text(tr("anim.interp_tip"));
            ui.separator();
            ui.toggle_value(&mut self.anim_ui.onion.enabled, format!("{}  {}", icons::GHOST, tr("anim.onion"))).on_hover_text(tr("anim.onion_tip"));
            ui.toggle_value(&mut self.anim_ui.only_selected_bones, icons::FUNNEL).on_hover_text(tr("anim.only_selected_tip"));
            if self.anim_ui.dirty_pose {
                ui.label(RichText::new(format!("{}  {}", icons::WARNING, tr("anim.unkeyed"))).color(Color32::from_rgb(0xff, 0xb0, 0x40))).on_hover_text(tr("anim.unkeyed_tip"));
            }
        });
    }

    // ============================================================ inspetor: aba Clip

    fn inspector_clip_tab(&mut self, ui: &mut egui::Ui) {
        self.section_clip(ui);
        ui.add_space(6.0);
        self.anim_section(ui, "sel", format!("{}  {}", icons::KEY, tr("anim.sel.title")), true, |s, ui| s.section_selected_keys(ui));
        self.anim_section(ui, "scene", format!("{}  {}", icons::CUBE, tr("anim.scene.title")), false, |s, ui| s.section_scene_keys(ui));
        self.anim_section(ui, "onion", format!("{}  {}", icons::GHOST, tr("anim.onion")), false, |s, ui| s.section_onion(ui));
        self.anim_section(ui, "markers", format!("{}  {}", icons::FLAG, tr("anim.markers")), false, |s, ui| s.section_markers(ui));
        self.anim_section(ui, "phys", format!("{}  {}", icons::WIND, tr("anim.phys.title")), false, |s, ui| s.section_physics(ui));
        self.anim_section(ui, "io", format!("{}  {}", icons::EXPORT, tr("anim.io.title")), false, |s, ui| s.section_files(ui));
        self.anim_section(ui, "seq", format!("{}  {}", icons::FILM_REEL, tr("anim.seq.title")), false, |s, ui| s.section_sequence(ui));
        self.anim_ui.force_section = None;
    }

    /// Seção recolhível do inspetor (abre à força quando um cenário pede).
    fn anim_section(&mut self, ui: &mut egui::Ui, id: &'static str, title: String, default_open: bool, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let force = (self.anim_ui.force_section == Some(id)).then_some(true);
        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(id).default_open(default_open).open(force).show(ui, |ui| body(self, ui));
    }

    fn section_clip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  {}", icons::FILM_STRIP, tr("anim.clip.title"))).strong());
            if ui.button(icons::PLUS).on_hover_text(tr("anim.clip.new")).clicked() {
                self.anim_new_clip();
            }
        });
        // seletor de clip
        let ids: Vec<(AnimationClipId, String)> = self.core.project().animation.clips.iter().map(|(i, c)| (i.clone(), c.name.clone())).collect();
        let active = self.anim_clip_id();
        let cur = active.as_ref().and_then(|a| ids.iter().find(|(i, _)| i == a)).map(|(_, n)| n.clone()).unwrap_or_else(|| tr("anim.no_clip").to_string());
        egui::ComboBox::from_id_salt("anim_clip_pick").selected_text(cur).width(FIELD_W).show_ui(ui, |ui| {
            for (id, name) in &ids {
                if ui.selectable_label(active.as_ref() == Some(id), name).clicked() {
                    self.anim_select_clip(id);
                }
            }
        });
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else {
            ui.label(RichText::new(tr("anim.no_clip_hint")).color(theme::TEXT_DIM));
            return;
        };
        let fps = self.anim_fps();
        // nome
        ui.horizontal(|ui| {
            if self.anim_ui.rename_for.as_ref() != Some(&id) {
                self.anim_ui.rename_for = Some(id.clone());
                self.anim_ui.rename_buf = clip.name.clone();
            }
            let te = ui.add(egui::TextEdit::singleline(&mut self.anim_ui.rename_buf).desired_width(FIELD_W - 84.0));
            let commit = te.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(tr("anim.clip.rename")).clicked() || commit {
                let name = self.anim_ui.rename_buf.trim().to_string();
                if !name.is_empty() && name != clip.name {
                    self.anim_apply(AnimEdit::RenameClip { clip_id: id.clone(), name });
                }
            }
        });
        // faixa
        let (a, b) = clip.effective_range(fps);
        let auto = clip.range.is_none();
        ui.horizontal(|ui| {
            let mut auto_v = auto;
            if ui.checkbox(&mut auto_v, tr("anim.range.auto")).on_hover_text(tr("anim.range.auto_tip")).changed() {
                self.anim_apply(AnimEdit::SetClipRange { clip_id: id.clone(), range: if auto_v { None } else { Some([a, b]) } });
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.range")).color(theme::TEXT_DIM));
            let (mut s, mut e) = (a, b);
            let c1 = ui.add(egui::DragValue::new(&mut s).range(0..=1_000_000).speed(0.5).prefix(format!("{} ", tr("anim.range.start")))).changed();
            let c2 = ui.add(egui::DragValue::new(&mut e).range(0..=1_000_000).speed(0.5).prefix(format!("{} ", tr("anim.range.end")))).changed();
            if (c1 || c2) && e >= s {
                self.anim_live(AnimEdit::SetClipRange { clip_id: id.clone(), range: Some([s, e]) });
            }
            if ui.small_button(icons::ARROWS_IN_LINE_HORIZONTAL).on_hover_text(tr("anim.range.fit_keys")).clicked() {
                if let Some((lo, hi)) = clip.key_extent() {
                    self.anim_apply(AnimEdit::SetClipRange { clip_id: id.clone(), range: Some([lo, hi]) });
                }
            }
        });
        ui.label(RichText::new(format!("{:.2} s  ·  {} {}", (b - a) as f32 / fps, clip.key_count(), tr("anim.keys"))).color(theme::TEXT_DIM));
        // fps do projeto
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.fps")).color(theme::TEXT_DIM));
            egui::ComboBox::from_id_salt("anim_fps").width(70.0).selected_text(format!("{fps:.0}")).show_ui(ui, |ui| {
                for p in FPS_PRESETS {
                    if ui.selectable_label((p - fps).abs() < 1e-3, format!("{p:.0}")).clicked() && (p - fps).abs() > 1e-3 {
                        let retime = self.anim_ui.retime;
                        self.anim_apply(AnimEdit::SetFps { fps: p, retime });
                    }
                }
            });
            ui.checkbox(&mut self.anim_ui.retime, tr("anim.fps.retime")).on_hover_text(tr("anim.fps.retime_tip"));
        });
        // extrapolação e "em Ns"
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.extrap")).color(theme::TEXT_DIM));
            for (e, key) in [(Extrapolation::Constant, "anim.extrap.constant"), (Extrapolation::Linear, "anim.extrap.linear")] {
                if ui.selectable_label(clip.extrapolation == e, tr(key)).clicked() && clip.extrapolation != e {
                    self.anim_apply(AnimEdit::SetClipPlayback { clip_id: id.clone(), loop_mode: None, extrapolation: Some(e), hold_step: None });
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.twos")).color(theme::TEXT_DIM));
            let mut h = clip.hold_step as i32;
            if ui.add(egui::DragValue::new(&mut h).range(1..=12).speed(0.1).suffix("s")).changed() {
                self.anim_apply(AnimEdit::SetClipPlayback { clip_id: id.clone(), loop_mode: None, extrapolation: None, hold_step: Some(h.clamp(1, 12) as u8) });
            }
            ui.checkbox(&mut self.anim_ui.transport.quantize, tr("anim.quantize")).on_hover_text(tr("anim.quantize_tip"));
        });
    }

    fn section_selected_keys(&mut self, ui: &mut egui::Ui) {
        let sel = self.anim_selected_existing();
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else {
            ui.label(RichText::new(tr("anim.sel.none")).color(theme::TEXT_DIM));
            return;
        };
        if sel.is_empty() {
            ui.label(RichText::new(tr("anim.sel.none")).color(theme::TEXT_DIM));
            ui.label(RichText::new(tr("anim.sel.help")).color(theme::TEXT_DIM).size(11.5));
            return;
        }
        ui.label(format!("{} {}", sel.len(), tr("anim.sel.count")));
        // interpolação das selecionadas
        let first = clip.channel(&sel[0].0).and_then(|c| c.key_at(sel[0].1)).copied();
        let cur = first.map(|k| k.interp).unwrap_or(Interp::Linear);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("anim.interp")).color(theme::TEXT_DIM));
            for (i, key) in [(Interp::Step, "anim.interp.step"), (Interp::Linear, "anim.interp.linear"), (Interp::Ease, "anim.interp.ease"), (Interp::EaseIn, "anim.interp.ease_in"), (Interp::EaseOut, "anim.interp.ease_out"), (crate::anim::graph::as_bezier(cur), "anim.interp.bezier")] {
                let selected = std::mem::discriminant(&cur) == std::mem::discriminant(&i);
                if ui.selectable_label(selected, tr(key)).clicked() {
                    self.anim_set_interp_selected(i);
                }
            }
        });
        // ações em lote
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::COPY, tr("anim.sel.duplicate"))).on_hover_text(tr("anim.sel.duplicate_tip")).clicked() {
                let step = clip.hold_step.max(1) as i32;
                self.anim_duplicate_selected(step.max(1) * 2);
            }
            if ui.button(format!("{}  {}", icons::FLIP_HORIZONTAL, tr("anim.sel.mirror"))).on_hover_text(tr("anim.sel.mirror_tip")).clicked() {
                self.anim_mirror_selected();
            }
            if ui.button(format!("{}  {}", icons::TRASH, tr("anim.sel.delete"))).clicked() {
                self.anim_delete_selected();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::CLIPBOARD, tr("anim.sel.copy"))).clicked() {
                self.anim_copy_keys();
            }
            let has = self.anim_ui.clipboard.is_some();
            if ui.add_enabled(has, egui::Button::new(format!("{}  {}", icons::CLIPBOARD, tr("anim.sel.paste")))).clicked() {
                self.anim_paste_keys(false);
            }
            if ui.add_enabled(has, egui::Button::new(tr("anim.sel.paste_mirrored"))).clicked() {
                self.anim_paste_keys(true);
            }
        });
        ui.horizontal(|ui| {
            for (label, d) in [("\u{2212}10", -10), ("\u{2212}1", -1), ("+1", 1), ("+10", 10)] {
                if ui.small_button(label).on_hover_text(tr("anim.sel.shift_tip")).clicked() {
                    let set: std::collections::BTreeSet<_> = sel.iter().cloned().collect();
                    if let Some((e, applied)) = keys::plan_move(&id, &clip, &set, d) {
                        let moved = set.iter().map(|(t, f)| (t.clone(), f + applied)).collect();
                        if self.anim_apply(e) {
                            self.anim_ui.selection = moved;
                        }
                    }
                }
            }
            ui.label(RichText::new(tr("anim.sel.shift")).color(theme::TEXT_DIM));
        });
        // uma chave só: canal, quadro, valor, alças
        if sel.len() == 1 {
            let (target, frame) = sel[0].clone();
            let Some(key) = clip.channel(&target).and_then(|c| c.key_at(frame)).copied() else { return };
            ui.separator();
            ui.label(RichText::new(self.anim_channel_label(&target)).strong());
            let mut f = frame;
            ui.horizontal(|ui| {
                ui.label(tr("anim.t.frame"));
                if ui.add(egui::DragValue::new(&mut f).range(0..=1_000_000).speed(0.25)).changed() && f != frame {
                    self.anim_live(AnimEdit::MoveKeys { clip_id: id.clone(), moves: vec![KeyMove { target: target.clone(), frame, to_frame: f }] });
                    self.anim_ui.selection = [(target.clone(), f)].into_iter().collect();
                }
            });
            // valor
            let mut new_value: Option<KeyValue> = None;
            match key.value {
                KeyValue::Scalar(v) => {
                    let mut x = v;
                    if ui.add(egui::DragValue::new(&mut x).speed(0.05)).changed() {
                        new_value = Some(KeyValue::Scalar(x));
                    }
                }
                KeyValue::Vec3(p) => {
                    let mut x = p;
                    ui.horizontal(|ui| {
                        let mut ch = false;
                        for (i, l) in ["X", "Y", "Z"].iter().enumerate() {
                            ch |= ui.add(egui::DragValue::new(&mut x[i]).speed(0.01).prefix(format!("{l} "))).changed();
                        }
                        if ch {
                            new_value = Some(KeyValue::Vec3(x));
                        }
                    });
                }
                KeyValue::Quat(q) => {
                    let e = quat_to_euler_deg(glam::Quat::from_xyzw(q[0], q[1], q[2], q[3])).map(|v| if v.abs() < 0.05 { 0.0 } else { v });
                    let mut x = e;
                    ui.horizontal(|ui| {
                        let mut ch = false;
                        for (i, l) in ["X", "Y", "Z"].iter().enumerate() {
                            ch |= ui.add(egui::DragValue::new(&mut x[i]).speed(0.5).suffix("\u{b0}").prefix(format!("{l} "))).changed();
                        }
                        if ch {
                            new_value = Some(KeyValue::Quat(euler_deg_to_quat(x).to_array()));
                        }
                    });
                }
                KeyValue::Curl(c) => {
                    let mut x = c;
                    ui.horizontal(|ui| {
                        let mut ch = false;
                        for (i, l) in ["T", "I", "M", "A", "P"].iter().enumerate() {
                            ch |= ui.add(egui::DragValue::new(&mut x[i]).speed(0.01).range(0.0..=1.0).prefix(format!("{l} "))).changed();
                        }
                        if ch {
                            new_value = Some(KeyValue::Curl(x));
                        }
                    });
                }
            }
            if let Some(v) = new_value {
                self.anim_live(AnimEdit::InsertKeys { clip_id: id.clone(), keys: vec![KeyInsert { target: target.clone(), key: Key { frame, value: v, interp: key.interp } }] });
            }
            // alças Bézier numéricas (o Graph as arrasta)
            if let Interp::Bezier { c } = key.interp {
                ui.label(RichText::new(tr("anim.handles")).color(theme::TEXT_DIM));
                let mut x = c;
                ui.horizontal(|ui| {
                    let mut ch = false;
                    for (i, l) in ["x1", "y1", "x2", "y2"].iter().enumerate() {
                        let r = if i % 2 == 0 { 0.0..=1.0 } else { -4.0..=5.0 };
                        ch |= ui.add(egui::DragValue::new(&mut x[i]).speed(0.01).range(r).prefix(format!("{l} "))).changed();
                    }
                    if ch {
                        self.anim_live(AnimEdit::SetInterpolation { clip_id: id.clone(), keys: vec![KeyRef { target: target.clone(), frame }], interp: Interp::Bezier { c: x } });
                    }
                });
            } else if !matches!(key.interp, Interp::Step | Interp::Linear) {
                ui.label(RichText::new(tr("anim.handles_preset")).color(theme::TEXT_DIM).size(11.5));
            }
        }
    }

    /// Chaves de sol, câmera de cena e transformação de um nó da cena (canais fora do corpo do personagem).
    fn section_scene_keys(&mut self, ui: &mut egui::Ui) {
        use anigo_core::animation::{CameraProp, SunProp};
        ui.label(RichText::new(tr("anim.scene.note")).color(theme::TEXT_DIM).size(11.5));
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::SUN, tr("anim.scene.key_sun"))).on_hover_text(tr("anim.scene.key_sun_tip")).clicked() {
                self.anim_insert_targets(vec![ChannelTarget::Sun { prop: SunProp::Direction }, ChannelTarget::Sun { prop: SunProp::Intensity }, ChannelTarget::Sun { prop: SunProp::Color }]);
            }
            if ui.button(format!("{}  {}", icons::VIDEO_CAMERA, tr("anim.scene.key_camera"))).on_hover_text(tr("anim.scene.key_camera_tip")).clicked() {
                self.anim_insert_targets(vec![ChannelTarget::Camera { prop: CameraProp::Fov }, ChannelTarget::Camera { prop: CameraProp::Eye }, ChannelTarget::Camera { prop: CameraProp::Target }]);
            }
        });
        let nodes: Vec<(anigo_core::NodeId, String)> = self.core.project().scene.nodes.iter().map(|n| (n.node_id.clone(), n.name.clone())).collect();
        if self.anim_ui.scene_node.as_ref().is_none_or(|id| !nodes.iter().any(|(n, _)| n == id)) {
            self.anim_ui.scene_node = nodes.first().map(|(i, _)| i.clone());
        }
        let cur = self.anim_ui.scene_node.as_ref().and_then(|id| nodes.iter().find(|(n, _)| n == id)).map(|(_, name)| name.clone()).unwrap_or_default();
        egui::ComboBox::from_id_salt("anim_scene_node").selected_text(cur).width(FIELD_W - 60.0).show_ui(ui, |ui| {
            for (id, name) in &nodes {
                if ui.selectable_label(self.anim_ui.scene_node.as_ref() == Some(id), name).clicked() {
                    self.anim_ui.scene_node = Some(id.clone());
                }
            }
        });
        if let Some(id) = self.anim_ui.scene_node.clone() {
            if ui.button(format!("{}  {}", icons::CUBE, tr("anim.scene.key_node"))).on_hover_text(tr("anim.scene.key_node_tip")).clicked() {
                self.anim_insert_targets(vec![ChannelTarget::NodeTranslation { node_id: id.clone() }, ChannelTarget::NodeRotation { node_id: id.clone() }, ChannelTarget::NodeScale { node_id: id }]);
            }
        }
    }

    fn section_onion(&mut self, ui: &mut egui::Ui) {
        let o = &mut self.anim_ui.onion;
        ui.checkbox(&mut o.enabled, tr("anim.onion"));
        ui.horizontal(|ui| {
            ui.label(tr("anim.onion.before"));
            ui.add(egui::Slider::new(&mut o.before, 0..=6));
        });
        ui.horizontal(|ui| {
            ui.label(tr("anim.onion.after"));
            ui.add(egui::Slider::new(&mut o.after, 0..=6));
        });
        ui.horizontal(|ui| {
            ui.label(tr("anim.onion.step"));
            ui.add(egui::Slider::new(&mut o.step, 1..=12));
        });
        ui.checkbox(&mut o.on_keys, tr("anim.onion.keys")).on_hover_text(tr("anim.onion.keys_tip"));
        ui.label(RichText::new(tr("anim.onion.note")).color(theme::TEXT_DIM).size(11.5));
    }

    fn section_markers(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.anim_clip_id() else {
            ui.label(RichText::new(tr("anim.no_clip")).color(theme::TEXT_DIM));
            return;
        };
        if ui.button(format!("{}  {}", icons::FLAG, tr("anim.marker.toggle"))).on_hover_text(tr("anim.marker.toggle_tip")).clicked() {
            self.anim_add_marker();
        }
        let markers = self.anim_clip().map(|c| c.markers.clone()).unwrap_or_default();
        let mut change: Option<Vec<anigo_core::animation::Marker>> = None;
        for (i, m) in markers.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button(format!("{}", m.frame)).on_hover_text(tr("anim.marker.go")).clicked() {
                    self.scrub_to(m.frame as f32, false);
                }
                let mut name = m.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).lost_focus() && name != m.name && !name.trim().is_empty() {
                    let mut v = markers.clone();
                    v[i].name = name.chars().take(64).collect();
                    change = Some(v);
                }
                if ui.small_button(icons::TRASH).clicked() {
                    let mut v = markers.clone();
                    v.remove(i);
                    change = Some(v);
                }
            });
        }
        if let Some(v) = change {
            self.anim_apply(AnimEdit::SetMarkers { clip_id: id, markers: v });
        }
    }

    fn section_physics(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("anim.phys.note")).color(theme::TEXT_DIM).size(11.5));
        let mut en = self.anim_ui.phys.enabled;
        if ui.checkbox(&mut en, tr("anim.phys.enable")).changed() {
            self.anim_ui.phys.enabled = en;
            self.anim_ui.phys.sim = None;
            self.anim_ui.phys.last_raw = None;
        }
        let mut cfg = self.anim_ui.phys.cfg;
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut cfg.hair, tr("anim.phys.hair")).changed();
            changed |= ui.checkbox(&mut cfg.skirt, tr("anim.phys.skirt")).changed();
        });
        ui.horizontal(|ui| {
            ui.label(tr("anim.phys.preroll"));
            changed |= ui.add(egui::Slider::new(&mut cfg.pre_roll_s, 0.0..=1.5).suffix(" s")).on_hover_text(tr("anim.phys.preroll_tip")).changed();
        });
        if changed {
            self.anim_ui.phys.cfg = cfg;
            self.anim_ui.phys.sim = None;
            self.anim_ui.phys.last_raw = None;
        }
        if let Some(s) = &self.anim_ui.phys.sim {
            ui.label(RichText::new(format!("{} {} · {} {} · {} {} · {:016x}", s.particle_count(), tr("anim.phys.particles"), s.steps, tr("anim.phys.steps"), s.resets, tr("anim.phys.resets"), s.state_hash())).color(theme::TEXT_DIM).size(11.5).monospace());
        }
        if ui.add_enabled(self.anim_clip().is_some(), egui::Button::new(format!("{}  {}", icons::FLOPPY_DISK, tr("anim.phys.bake")))).on_hover_text(tr("anim.phys.bake_tip")).clicked() {
            self.anim_bake_physics();
        }
        if !self.anim_ui.phys.status.is_empty() {
            ui.label(RichText::new(self.anim_ui.phys.status.clone()).color(theme::TEXT_DIM).size(11.5));
        }
    }

    // ============================================================ arquivos

    fn section_files(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("anim.io.note")).color(theme::TEXT_DIM).size(11.5));
        ui.add(egui::TextEdit::singleline(&mut self.anim_ui.io_path).hint_text(tr("anim.io.path_hint")).desired_width(FIELD_W));
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::DOWNLOAD_SIMPLE, tr("anim.io.import"))).on_hover_text(tr("anim.io.import_tip")).clicked() {
                self.anim_import_file();
            }
        });
        let has = self.anim_clip().is_some();
        ui.horizontal_wrapped(|ui| {
            for (kind, key) in [(ExportKind::Glb, "anim.io.export_glb"), (ExportKind::Vrma, "anim.io.export_vrma"), (ExportKind::Bvh, "anim.io.export_bvh"), (ExportKind::ClipJson, "anim.io.export_clip")] {
                if ui.add_enabled(has, egui::Button::new(format!("{}  {}", icons::UPLOAD_SIMPLE, tr(key)))).clicked() {
                    self.anim_export_file(kind);
                }
            }
        });
        if !self.anim_ui.io_status.is_empty() {
            ui.label(RichText::new(self.anim_ui.io_status.clone()).color(theme::TEXT_DIM).size(11.5));
        }
    }

    /// Pasta padrão das exportações.
    fn anim_export_dir(&self) -> std::path::PathBuf {
        if let Some(d) = &self.anim_ui.export_dir {
            return d.clone();
        }
        if let Some(d) = std::env::var_os("ANIGO_ANIMATION_DIR") {
            return std::path::PathBuf::from(d);
        }
        self.render_ui.out_dir()
    }

    pub(crate) fn anim_import_file(&mut self) {
        let path = std::path::PathBuf::from(self.anim_ui.io_path.trim());
        let msg = self.anim_import_path(&path);
        self.anim_ui.io_status = msg.clone();
        self.anim_say(msg);
    }

    /// Importa um arquivo de animação como clip(s) novo(s) (um passo de histórico). Devolve a mensagem de status.
    pub(crate) fn anim_import_path(&mut self, path: &std::path::Path) -> String {
        let lower = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let stem = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let stem = stem.split('.').next().unwrap_or("clip").to_string();
        let fps = self.anim_fps();
        let rig = self.pose_rig();
        let meta = match std::fs::metadata(path) {
            Ok(m) if m.is_file() => m,
            _ => return format!("{}: {}", tr("anim.io.error"), path.display()),
        };
        if meta.len() > 128 * 1024 * 1024 {
            return format!("{}: {}", tr("anim.io.error"), tr("anim.io.too_big"));
        }
        let mut clips: Vec<AnimationClip> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let result: Result<(), String> = (|| {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if lower.ends_with(CLIP_ASSET_SUFFIX) {
                let a = ClipAsset::from_json(&bytes).map_err(|e| e.to_string())?;
                let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &a.name);
                clips.push(a.to_clip(id, fps));
            } else if lower.ends_with(".bvh") {
                let text = String::from_utf8_lossy(&bytes).to_string();
                let m = parse_bvh(&text).map_err(|e| e.to_string())?;
                let map = BoneMap::from_names(&m.skeleton);
                if map.mapped_count() < 6 {
                    return Err(format!("{} ({}/23)", tr("anim.io.no_humanoid"), map.mapped_count()));
                }
                let rt = Retargeter::new(&m.skeleton, &map, &rig)?;
                notes.extend(rt.warnings.iter().cloned());
                let poses: Vec<_> = m.frames.iter().map(|f| rt.pose(f)).collect();
                let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &stem);
                let mut clip = poses_to_clip(&poses, m.fps, fps, id, &stem, ClipBuild::default());
                clip.duration_seconds = poses.len().saturating_sub(1) as f32 / m.fps;
                clips.push(clip);
            } else if lower.ends_with(".glb") || lower.ends_with(".gltf") || lower.ends_with(".vrma") || lower.ends_with(".vrm") {
                let imp = anigo_vrm::anim_io::import_gltf_animations(&bytes, path.parent(), &rig, fps)?;
                notes.extend(imp.warnings.iter().cloned());
                let multi = imp.clips.len() > 1;
                for ic in imp.clips {
                    let name = if multi { format!("{stem} · {}", ic.name) } else { stem.clone() };
                    let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &name);
                    let mut c = ic.clip;
                    c.clip_id = id;
                    c.name = name;
                    clips.push(c);
                }
            } else {
                return Err(tr("anim.io.unsupported").to_string());
            }
            Ok(())
        })();
        if let Err(e) = result {
            return format!("{}: {e}", tr("anim.io.error"));
        }
        // ids únicos entre os clips do mesmo arquivo
        let mut used: std::collections::BTreeSet<AnimationClipId> = self.core.project().animation.clips.keys().cloned().collect();
        let mut commands = Vec::new();
        let mut first: Option<AnimationClipId> = None;
        for mut c in clips {
            let mut n = 2;
            while used.contains(&c.clip_id) {
                c.clip_id = AnimationClipId::from_label(&format!("{} {n}", c.name));
                n += 1;
            }
            used.insert(c.clip_id.clone());
            first.get_or_insert_with(|| c.clip_id.clone());
            commands.push(Command::EditAnimation { edit: AnimEdit::PutClip { clip: c } });
        }
        let Some(first) = first else { return tr("anim.io.nothing").to_string() };
        let count = commands.len();
        commands.push(Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(first) } });
        if !self.anim_apply_cmd(Command::Batch { commands }) {
            return tr("anim.io.error").to_string();
        }
        self.anim_ui.selection.clear();
        self.anim_ui.transport.goto(0.0, true);
        let warn = if notes.is_empty() { String::new() } else { format!(" · {} {}", notes.len(), tr("anim.io.warnings")) };
        format!("{}: {stem} ({count} {}){warn}", tr("anim.io.imported"), tr("anim.clips_n"))
    }

    pub(crate) fn anim_export_file(&mut self, kind: ExportKind) {
        let Some(clip) = self.anim_clip().cloned() else { return };
        let fps = self.anim_fps();
        let rig = self.pose_rig();
        let ext = match kind {
            ExportKind::Glb => "glb",
            ExportKind::Vrma => "vrma",
            ExportKind::Bvh => "bvh",
            ExportKind::ClipJson => &CLIP_ASSET_SUFFIX[1..],
        };
        let typed = self.anim_ui.io_path.trim().to_string();
        let mut path = if typed.is_empty() { self.anim_export_dir() } else { std::path::PathBuf::from(&typed) };
        if path.is_dir() || typed.is_empty() {
            let safe: String = clip.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
            path = path.join(format!("{}.{ext}", safe.trim()));
        }
        let result: Result<Vec<u8>, String> = match kind {
            ExportKind::Glb => anigo_vrm::anim_io::export_gltf_animation(&clip, fps, &rig, false),
            ExportKind::Vrma => anigo_vrm::anim_io::export_gltf_animation(&clip, fps, &rig, true),
            ExportKind::Bvh => Ok(write_bvh(&clip, fps, &rig).into_bytes()),
            ExportKind::ClipJson => ClipAsset::from_clip(clip.name.clone(), &clip, fps).and_then(|a| a.to_json()).map(String::into_bytes).map_err(|e| e.to_string()),
        };
        let msg = match result {
            Ok(bytes) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&path, &bytes) {
                    Ok(()) => format!("{}: {} ({} KiB)", tr("anim.io.exported"), path.display(), bytes.len().div_ceil(1024)),
                    Err(e) => format!("{}: {e}", tr("anim.io.error")),
                }
            }
            Err(e) => format!("{}: {e}", tr("anim.io.error")),
        };
        self.anim_ui.io_status = msg.clone();
        self.anim_say(msg);
    }

    pub(crate) fn anim_bake_physics(&mut self) {
        let Some(clip) = self.anim_clip().cloned() else { return };
        let fps = self.anim_fps();
        let (a, b) = clip.effective_range(fps);
        let rig = self.pose_rig();
        let msg = match SecondarySim::new(&rig, self.anim_ui.phys.cfg) {
            Ok(mut sim) => {
                let baked = bake(&mut sim, &clip, fps, a, b);
                let safe: String = clip.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
                let path = self.anim_export_dir().join(format!("{}.physics.json", safe.trim()));
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
                match serde_json::to_vec(&baked).map_err(|e| e.to_string()).and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string())) {
                    Ok(()) => format!("{}: {} ({} {})", tr("anim.phys.baked"), path.display(), baked.frames.len(), tr("anim.phys.frames")),
                    Err(e) => format!("{}: {e}", tr("anim.io.error")),
                }
            }
            Err(e) => e,
        };
        self.anim_ui.phys.status = msg.clone();
        self.anim_say(msg);
    }

    // ============================================================ sequência de render

    fn section_sequence(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self.anim_clip().cloned() else {
            ui.label(RichText::new(tr("anim.no_clip")).color(theme::TEXT_DIM));
            return;
        };
        let fps = self.anim_fps();
        let (a, b) = clip.effective_range(fps);
        if !self.anim_ui.seq_custom {
            self.anim_ui.seq_start = a;
            self.anim_ui.seq_end = b;
        }
        ui.label(RichText::new(tr("anim.seq.note")).color(theme::TEXT_DIM).size(11.5));
        ui.checkbox(&mut self.anim_ui.seq_custom, tr("anim.seq.custom"));
        ui.add_enabled_ui(self.anim_ui.seq_custom, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.anim_ui.seq_start).range(0..=1_000_000).prefix(format!("{} ", tr("anim.range.start"))));
                ui.add(egui::DragValue::new(&mut self.anim_ui.seq_end).range(0..=1_000_000).prefix(format!("{} ", tr("anim.range.end"))));
                ui.add(egui::DragValue::new(&mut self.anim_ui.seq_step).range(1..=100).prefix(format!("{} ", tr("anim.seq.step"))));
            });
        });
        if self.anim_ui.seq_end < self.anim_ui.seq_start {
            self.anim_ui.seq_end = self.anim_ui.seq_start;
        }
        let frames = FrameRange { start: self.anim_ui.seq_start.max(0) as u32, end: self.anim_ui.seq_end.max(0) as u32, step: self.anim_ui.seq_step.max(1) }.frames().len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.seq.folder")).color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.render_ui.prefs.dir).desired_width(FIELD_W - 60.0));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("anim.seq.template")).color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.anim_ui.seq_template).desired_width(FIELD_W - 60.0)).on_hover_text(tr("anim.seq.template_tip"));
        });
        let ctx = ui.ctx().clone();
        let busy = self.anim_seq_job().is_some_and(|j| matches!(j.status, super::super::render_ui::JobStatus::Queued | super::super::render_ui::JobStatus::Running));
        if ui.add_enabled(!busy, egui::Button::new(format!("{}  {} ({frames} {})", icons::FILM_REEL, tr("anim.seq.render"), tr("anim.seq.frames")))).clicked() {
            self.anim_render_sequence(&ctx);
        }
        if let Some(j) = self.anim_seq_job() {
            use super::super::render_ui::JobStatus;
            let (frac, status, id) = (j.frac, j.status.clone(), j.id);
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            let txt = match &status {
                JobStatus::Queued => tr("anim.seq.st.queued").to_string(),
                JobStatus::Running => tr("anim.seq.st.running").to_string(),
                JobStatus::Done => tr("anim.seq.st.done").to_string(),
                JobStatus::Failed(e) => format!("{}: {e}", tr("anim.seq.st.failed")),
                JobStatus::Cancelled => tr("anim.seq.st.cancelled").to_string(),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(txt).color(theme::TEXT_DIM));
                if busy && ui.button(tr("anim.seq.cancel")).clicked() {
                    self.render_cancel(id);
                }
                if ui.button(tr("anim.seq.open_render")).clicked() {
                    self.switch_workspace(crate::workspace::WorkspaceId::Render);
                }
            });
        }
        ui.label(RichText::new(ffmpeg_hint(&self.render_ui.out_dir(), &self.anim_ui.seq_template, &self.core.project().name, fps)).color(theme::TEXT_DIM).size(11.0).monospace());
    }

    fn anim_seq_job(&self) -> Option<&super::super::render_ui::JobView> {
        let id = self.anim_ui.seq_job?;
        self.render_ui.jobs.iter().find(|j| j.id == id)
    }

    /// "Renderizar sequência": congela a cena como o Render a monta e enfileira um job com uma `FrameSource` que avalia o clip.
    pub(crate) fn anim_render_sequence(&mut self, ctx: &egui::Context) {
        let Some(clip) = self.anim_clip().cloned() else { return };
        let dir = self.render_ui.out_dir();
        if let Err(e) = check_output_dir(&dir) {
            self.anim_say(format!("{}: {e}", tr("render.err.dir")));
            return;
        }
        // a Biblioteca abre em segundo plano (as malhas dos ativos da cena vêm dela)
        {
            let w = self.lib_workspace();
            w.ensure_open();
            w.pump(ctx);
        }
        let (scene, notes) = match self.build_render_scene() {
            Ok(v) => v,
            Err(e) => {
                self.anim_say(e);
                return;
            }
        };
        let fps = self.anim_fps();
        let rig = self.pose_rig();
        let project = self.core.project().clone();
        let source = Arc::new(ClipFrameSource::new(scene, project, rig, clip.clone(), fps));
        let spec = self.render_spec();
        let (a, b) = if self.anim_ui.seq_custom { (self.anim_ui.seq_start, self.anim_ui.seq_end) } else { clip.effective_range(fps) };
        let range = FrameRange { start: a.max(0) as u32, end: b.max(a).max(0) as u32, step: self.anim_ui.seq_step.max(1) };
        let (tw, th) = spec.output.target_size();
        let tokens = NameTokens { project: spec.meta.project_name.clone(), camera: spec.meta.camera_name.clone(), w: tw, h: th, ..Default::default() };
        let id = self.render_ui.next_id;
        self.render_ui.next_id += 1;
        let job = JobSpec {
            id,
            label: format!("{} · {}–{} · {}×{}", clip.name, range.start, range.end, tw, th),
            source,
            spec,
            dir,
            template: self.anim_ui.seq_template.clone(),
            overwrite: self.render_ui.prefs.overwrite,
            counter: self.render_ui.prefs.counter,
            frames: Some(range),
            skip_existing: false,
            reuse: None,
            tokens,
        };
        self.render_submit(ctx, job, notes);
        self.anim_ui.seq_job = Some(id);
        self.anim_say(tr("anim.seq.queued"));
    }

    // ============================================================ Biblioteca de clips

    pub(crate) fn clip_library_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(format!("{}  {}", icons::SQUARES_FOUR, tr("anim.lib.title"))).strong());
        if self.anim_ui.lib_name.is_empty() {
            if let Some(c) = self.anim_clip() {
                self.anim_ui.lib_name = c.name.clone();
            }
        }
        ui.add(egui::TextEdit::singleline(&mut self.anim_ui.lib_name).hint_text(tr("anim.lib.name_hint")).desired_width(FIELD_W - 40.0));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.anim_clip().is_some(), egui::Button::new(format!("{}  {}", icons::FLOPPY_DISK, tr("anim.lib.save")))).on_hover_text(tr("anim.lib.save_tip")).clicked() {
                self.anim_save_clip_to_library();
            }
            if ui.button(icons::ARROW_CLOCKWISE).on_hover_text(tr("anim.lib.refresh")).clicked() {
                self.anim_refresh_library();
            }
        });
        if !self.anim_ui.cards_loaded {
            self.anim_refresh_library();
        }
        let cards = self.anim_ui.cards.clone();
        egui::ScrollArea::vertical().id_salt("anim_lib_cards").auto_shrink([false, false]).show(ui, |ui| {
            if cards.is_empty() {
                ui.label(RichText::new(tr("anim.lib.empty")).color(theme::TEXT_DIM));
            }
            for (i, c) in cards.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{}  {}", icons::FILM_STRIP, c.name));
                    ui.label(RichText::new(format!("r{} · {:.1}s · {} {}", c.revision, c.asset.duration_seconds(), c.asset.key_count(), tr("anim.keys"))).color(theme::TEXT_DIM).size(11.5));
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button(tr("anim.lib.apply_new")).on_hover_text(tr("anim.lib.apply_new_tip")).clicked() {
                        self.anim_apply_card(i, false);
                    }
                    if ui.small_button(tr("anim.lib.apply_at")).on_hover_text(tr("anim.lib.apply_at_tip")).clicked() {
                        self.anim_apply_card(i, true);
                    }
                });
                ui.add_space(2.0);
            }
        });
    }

    pub(crate) fn anim_refresh_library(&mut self) {
        match self.lib_workspace().list_clip_assets(200) {
            Ok(list) => {
                self.anim_ui.cards = list;
                self.anim_ui.cards_loaded = true;
            }
            Err(e) => {
                self.anim_ui.cards.clear();
                self.anim_ui.cards_loaded = true;
                self.anim_say(format!("{}: {e}", tr("anim.lib.error")));
            }
        }
    }

    pub(crate) fn anim_save_clip_to_library(&mut self) {
        let Some(clip) = self.anim_clip().cloned() else { return };
        let name = if self.anim_ui.lib_name.trim().is_empty() { clip.name.clone() } else { self.anim_ui.lib_name.trim().to_string() };
        let fps = self.anim_fps();
        match ClipAsset::from_clip(name, &clip, fps) {
            Ok(asset) => match self.lib_workspace().save_clip_asset(&asset) {
                Ok(msg) => {
                    self.anim_say(msg);
                    self.anim_refresh_library();
                }
                Err(e) => self.anim_say(format!("{}: {e}", tr("anim.lib.error"))),
            },
            Err(e) => self.anim_say(format!("{}: {e}", tr("anim.lib.error"))),
        }
    }

    /// Aplica o card `idx`: como clip NOVO (já ativo) ou inserindo as chaves a partir do playhead no clip ativo.
    pub(crate) fn anim_apply_card(&mut self, idx: usize, at_playhead: bool) {
        let Some(card) = self.anim_ui.cards.get(idx).cloned() else { return };
        let fps = self.anim_fps();
        if at_playhead {
            let Some(id) = self.anim_ensure_clip() else { return };
            let at = self.anim_key_frame();
            let keys = card.asset.key_inserts(at, fps);
            if keys.is_empty() {
                return;
            }
            self.anim_apply(AnimEdit::InsertKeys { clip_id: id, keys });
        } else {
            let id = crate::anim::autokey::fresh_clip_id(self.core.project(), &card.name);
            let clip = card.asset.to_clip(id.clone(), fps);
            let batch = Command::Batch { commands: vec![Command::EditAnimation { edit: AnimEdit::PutClip { clip } }, Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(id) } }] };
            self.anim_apply_cmd(batch);
            self.anim_ui.selection.clear();
            self.anim_ui.transport.goto(0.0, true);
        }
        let _ = describe_anim;
    }
}

/// Linha do `ffmpeg` sugerida para montar a sequência (só texto; nada é executado). O template usa `%04d` (ou
/// `{frame:04}`) para o número do quadro; sem token, o Render acrescenta `_NNNN`.
pub(crate) fn ffmpeg_hint(dir: &std::path::Path, template: &str, project: &str, fps: f32) -> String {
    let tokens = crate::render_out::settings::NameTokens { project: project.to_string(), n: 0, frame: Some(0), ..Default::default() };
    let stem = crate::render_out::settings::allocate_frame(dir, template, &tokens, "png", &[]).stem;
    // padrão printf reconstruído a partir do nome do quadro 0
    let pattern = stem.replacen("0000", "%04d", 1);
    format!("ffmpeg -framerate {} -i \"{}\" -c:v libx264 -pix_fmt yuv420p out.mp4", fps.round() as u32, dir.join(format!("{pattern}.png")).display())
}

/// O que exportar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportKind {
    Glb,
    Vrma,
    Bvh,
    ClipJson,
}

#[allow(dead_code)]
fn _unused(_: &AnimUi, _: &ChannelTarget) {}
