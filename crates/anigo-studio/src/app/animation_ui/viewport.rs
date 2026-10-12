//! Viewport da Animação: é o do Posing (seleção de juntas, gizmo FK, IK: o auto-key e a edição ao vivo usam a MESMA
//! rota) com camadas extras por cima: onion skin (fantasmas do esqueleto em quadros/chaves vizinhos), física
//! secundária (cabelo e saia de teste), HUD de quadro/timecode e o trace do smoke.

use super::{StudioApp, View};
use crate::i18n::tr;
use crate::icons;
use crate::scene::camview::CamView;
use crate::theme;
use anigo_core::pose::{PoseState, BONE_COUNT, BONE_PARENTS};
use egui::{Color32, Pos2, Rect, Stroke};
use glam::Vec3;

const BEFORE_COL: Color32 = Color32::from_rgb(0x4a, 0x9c, 0xff);
const AFTER_COL: Color32 = Color32::from_rgb(0x58, 0xd0, 0x7a);
const HAIR_COL: Color32 = Color32::from_rgb(0x70, 0xe0, 0xff);
const SKIRT_COL: Color32 = Color32::from_rgb(0xff, 0xb0, 0x40);

/// Fantasma do onion skin: quadro, posição relativa (-1 antes / +1 depois) e peso de opacidade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ghost {
    pub frame: i32,
    pub after: bool,
    pub weight: f32,
}

/// Quadros dos fantasmas do onion skin. Modo "chaves": as chaves vizinhas; senão `step` quadros de distância.
pub(crate) fn onion_ghosts(o: &super::Onion, cur: i32, keys: &[i32], range_start: i32) -> Vec<Ghost> {
    let mut out = Vec::new();
    let weight = |i: usize, n: u8| 1.0 - (i as f32 - 1.0) / (n.max(1) as f32 + 0.5);
    if o.on_keys {
        let prev: Vec<i32> = keys.iter().copied().filter(|f| *f < cur).rev().take(o.before as usize).collect();
        let next: Vec<i32> = keys.iter().copied().filter(|f| *f > cur).take(o.after as usize).collect();
        for (i, f) in prev.into_iter().enumerate() {
            out.push(Ghost { frame: f, after: false, weight: weight(i + 1, o.before) });
        }
        for (i, f) in next.into_iter().enumerate() {
            out.push(Ghost { frame: f, after: true, weight: weight(i + 1, o.after) });
        }
    } else {
        let step = o.step.max(1) as i32;
        for i in 1..=o.before as i32 {
            let f = cur - i * step;
            if f >= range_start.min(0) {
                out.push(Ghost { frame: f, after: false, weight: weight(i as usize, o.before) });
            }
        }
        for i in 1..=o.after as i32 {
            out.push(Ghost { frame: cur + i * step, after: true, weight: weight(i as usize, o.after) });
        }
    }
    out
}

impl StudioApp {
    /// Camadas da Animação sobre o viewport do Posing (chamada por `posing_viewport_ui` quando o workspace é a Animação).
    pub(crate) fn anim_viewport_overlays(&mut self, painter: &egui::Painter, rect: Rect, view: &CamView, pv: &super::super::posing_ui::PoseView) {
        let to = |p: glam::Vec2| Pos2::new(rect.min.x + p.x, rect.min.y + p.y);
        let proj = |w: Vec3| view.project(w).map(|(p, _)| to(p));
        let fps = self.anim_fps();
        // ---- onion skin
        if self.anim_ui.onion.enabled {
            if let Some(clip) = self.anim_clip() {
                let cur = self.anim_frame();
                let (start, _) = clip.effective_range(fps);
                let keys = clip.key_frames();
                let ghosts = onion_ghosts(&self.anim_ui.onion, cur, &keys, start);
                for g in ghosts {
                    let pose = clip.evaluate(fps, g.frame as f32).apply_to_pose(&PoseState::default());
                    let joints = pv.rig.joint_positions(&pose);
                    let col = if g.after { AFTER_COL } else { BEFORE_COL };
                    let a = (0.20 + 0.55 * g.weight.clamp(0.0, 1.0)).clamp(0.1, 0.8);
                    let stroke = Stroke::new(2.4, col.gamma_multiply(a));
                    for b in 2..BONE_COUNT {
                        let Some(p) = BONE_PARENTS[b] else { continue };
                        if p == anigo_core::pose::bone::ROOT {
                            continue;
                        }
                        if let (Some(pa), Some(pb)) = (proj(pv.to_world(joints[p])), proj(pv.to_world(joints[b]))) {
                            painter.line_segment([pa, pb], stroke);
                        }
                    }
                    if let Some(ph) = proj(pv.to_world(joints[anigo_core::pose::bone::HEAD])) {
                        painter.circle_stroke(ph, 6.0, stroke);
                        painter.text(ph + egui::vec2(0.0, -12.0), egui::Align2::CENTER_CENTER, format!("{}", g.frame), egui::FontId::proportional(10.0), col.gamma_multiply(a + 0.15));
                    }
                }
            }
        }
        // ---- física secundária (dado + overlay de depuração)
        if self.anim_ui.phys.enabled {
            if let Some(sim) = &self.anim_ui.phys.sim {
                for (a, b) in sim.skirt_edges() {
                    if let (Some(pa), Some(pb)) = (proj(pv.to_world(a)), proj(pv.to_world(b))) {
                        painter.line_segment([pa, pb], Stroke::new(1.0, SKIRT_COL.gamma_multiply(0.75)));
                    }
                }
                for chain in sim.hair_chains() {
                    let pts: Vec<Pos2> = chain.iter().filter_map(|p| proj(pv.to_world(*p))).collect();
                    if pts.len() > 1 {
                        painter.add(egui::Shape::line(pts, Stroke::new(1.8, HAIR_COL)));
                    }
                }
            }
        }
        // ---- HUD: clip, quadro e timecode
        let span = self.anim_span();
        let shown = self.anim_ui.transport.displayed(span);
        let name = self.anim_clip().map(|c| c.name.clone()).unwrap_or_else(|| tr("anim.no_clip").to_string());
        let text = format!("{}  ·  {} {:.0}  ·  {}", name, tr("anim.t.frame"), shown.round(), anigo_core::animation::timecode(shown.round() as i32, fps));
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(12.5), Color32::WHITE);
        let pos = rect.right_bottom() + egui::vec2(-10.0, -10.0);
        let r = Rect::from_min_size(pos - galley.size() - egui::vec2(0.0, 0.0), galley.size()).expand2(egui::vec2(7.0, 3.0));
        painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 205));
        painter.galley(r.min + egui::vec2(7.0, 3.0), galley, Color32::WHITE);
        if self.anim_ui.autokey {
            let c = Color32::from_rgb(0xff, 0x40, 0x40);
            let t = format!("{}  {}", icons::RECORD, tr("anim.autokey_on"));
            painter.text(Pos2::new(r.min.x - 14.0, r.center().y), egui::Align2::RIGHT_CENTER, t, egui::FontId::proportional(12.5), c);
        }
        let _ = (theme::TEXT, View::default);
    }

    /// `ANIGO_TRACE_ANIMATION=1`: uma linha `animtrace ...` por mudança de estado, para o smoke de xdotool.
    pub(crate) fn anim_trace(&mut self, ctx: &egui::Context) {
        if !self.anim_ui.trace {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let p = self.core.project();
        let (clip_id, keys, chans) = match self.anim_clip() {
            Some(c) => (c.clip_id.to_string().replace(' ', "%20"), c.key_count(), c.channels.len()),
            None => ("-".to_string(), 0, 0),
        };
        let span = self.anim_span();
        let v = self.anim_ui.view;
        let c = self.anim_ui.canvas;
        let pt = |p: Pos2| format!("{:.0},{:.0}", p.x * ppp, p.y * ppp);
        let h = self.core.history();
        let pose = p.character.pose.clone();
        let sig: Vec<String> = pose.rotations.iter().map(|(k, q)| format!("{k}:{:.3},{:.3},{:.3},{:.3}", q[0], q[1], q[2], q[3])).collect();
        let mut s = format!(
            "animtrace clip={} keys={} chans={} frame={} raw={:.2} playing={} loop={:?} undo={} redo={} prev={} dirty={} sel={} drag={} autokey={} fps={} canvas={:.0},{:.0},{:.0},{:.0} ppp={:.3} ruler_h={:.0} x0={:.3} ppf={:.3} play_btn={} key_btn={} script={} pose=[{}]",
            clip_id,
            keys,
            chans,
            self.anim_ui.transport.current_frame(span),
            self.anim_ui.transport.raw,
            self.anim_ui.transport.playing,
            span.mode,
            h.undo_depth(),
            h.redo_depth(),
            self.core.is_previewing(),
            self.anim_ui.dirty_pose,
            self.anim_ui.selection.len(),
            self.anim_ui.drag.is_some(),
            self.anim_ui.autokey,
            self.anim_fps(),
            c.min.x * ppp,
            c.min.y * ppp,
            c.width() * ppp,
            c.height() * ppp,
            ppp,
            self.anim_ui.ruler_h * ppp,
            v.x0,
            v.ppf,
            pt(self.anim_ui.play_btn),
            pt(self.anim_ui.key_btn),
            match &self.anim_ui.script {
                Some(sc) if sc.is_done() => "done",
                Some(_) => "run",
                None => "none",
            },
            sig.join(";"),
        );
        // y (pixels) de cada canal visível no dope sheet, e as chaves (quadro) do primeiro canal
        for (i, (t, y)) in self.anim_ui.rows_geom.iter().enumerate().take(8) {
            s.push_str(&format!(" row{}={}@{:.0}", i, t.key_string().replace(' ', "%20"), y * ppp));
        }
        if let Some(clip) = self.anim_clip() {
            if let Some(ch) = clip.channels.first() {
                let frames: Vec<String> = ch.keys.iter().map(|k| k.frame.to_string()).collect();
                s.push_str(&format!(" ch0={} ch0keys={}", ch.target.key_string().replace(' ', "%20"), frames.join(",")));
            }
        }
        if let Some(sim) = &self.anim_ui.phys.sim {
            s.push_str(&format!(" phys={:016x} physsteps={} physresets={}", sim.state_hash(), sim.steps, sim.resets));
        }
        if s != self.anim_ui.last_trace {
            eprintln!("{s}");
            self.anim_ui.last_trace = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::animation_ui::Onion;

    #[test]
    fn onion_ghosts_by_step_are_symmetric_and_fade() {
        let o = Onion { enabled: true, before: 2, after: 2, step: 3, on_keys: false };
        let g = onion_ghosts(&o, 12, &[], 0);
        let frames: Vec<(i32, bool)> = g.iter().map(|g| (g.frame, g.after)).collect();
        assert_eq!(frames, vec![(9, false), (6, false), (15, true), (18, true)]);
        assert!(g[0].weight > g[1].weight, "o mais próximo é o mais opaco");
    }

    #[test]
    fn onion_ghosts_never_go_before_frame_zero() {
        let o = Onion { enabled: true, before: 3, after: 0, step: 2, on_keys: false };
        let g = onion_ghosts(&o, 3, &[], 0);
        assert_eq!(g.iter().map(|g| g.frame).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn onion_ghosts_on_keys_use_the_neighbour_keys() {
        let o = Onion { enabled: true, before: 1, after: 2, step: 3, on_keys: true };
        let g = onion_ghosts(&o, 10, &[0, 4, 10, 16, 30, 40], 0);
        let frames: Vec<i32> = g.iter().map(|g| g.frame).collect();
        assert_eq!(frames, vec![4, 16, 30]);
        assert_eq!(g.iter().filter(|g| g.after).count(), 2);
    }
}
