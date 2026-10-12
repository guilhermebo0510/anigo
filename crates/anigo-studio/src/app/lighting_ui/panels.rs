//! Painel direito do Iluminação: rigs de luz, sol, sombras, ambiente, rim, luzes extras (desabilitadas, com
//! explicação) e a cena de prova. Cada controle é ligado a um dado real do projeto: edições contínuas viram
//! `preview` + UM comando ao soltar (`light_edit`), botões viram um comando (`light_apply`).

use super::{LightPatch, Section, StudioApp};
use crate::i18n::tr;
use crate::icons;
use crate::lighting::{self, rigs::RIGS};
use crate::theme;
use anigo_core::command::MaterialPatch;
use anigo_core::scene::LightShadow;
use egui::{Color32, RichText};
use glam::Vec2;
use std::ops::RangeInclusive;

fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized([112.0, 20.0], egui::Label::new(RichText::new(label).size(12.5).color(theme::TEXT_DIM)));
        add(ui)
    })
    .inner
}

fn slider(ui: &mut egui::Ui, v: &mut f32, range: RangeInclusive<f32>, suffix: &str, decimals: usize) -> bool {
    ui.spacing_mut().slider_width = (ui.available_width() - 104.0).max(60.0);
    ui.add(egui::Slider::new(v, range).suffix(suffix).max_decimals(decimals).clamping(egui::SliderClamping::Always)).changed()
}

fn section(ui: &mut egui::Ui, icon: &str, title_key: &str, default_open: bool, focus: bool, body: impl FnOnce(&mut egui::Ui)) {
    let mut header = egui::CollapsingHeader::new(RichText::new(format!("{icon}  {}", tr(title_key))).strong()).default_open(default_open);
    if focus {
        header = header.open(Some(true));
    }
    let r = header.show(ui, body);
    if focus {
        r.header_response.scroll_to_me(Some(egui::Align::TOP));
    }
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(theme::TEXT_DIM));
}

/// Faixa de cor entre dois pontos (gradiente horizontal) — a "régua" da temperatura e do hemisfério.
fn gradient_strip(ui: &mut egui::Ui, height: f32, color_at: impl Fn(f32) -> Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let n = 48;
    let w = rect.width() / n as f32;
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32;
        let r = egui::Rect::from_min_size(egui::pos2(rect.left() + i as f32 * w, rect.top()), egui::vec2(w + 0.6, height));
        ui.painter().rect_filled(r, 0.0, color_at(t));
    }
}

fn rgb8(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

impl StudioApp {
    pub(crate) fn light_panels(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("light_inspector").default_size(350.0).size_range(300.0..=540.0).show(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{}  {}", icons::SUN, tr("light.title"))).strong().size(14.0));
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let focus = self.light_ui.focus.take();
                section(ui, icons::LIGHTBULB, "light.section.rigs", true, focus == Some(Section::Rigs), |ui| self.light_rigs_body(ui));
                section(ui, icons::SUN, "light.section.sun", true, focus == Some(Section::Sun), |ui| self.light_sun_body(ui));
                section(ui, icons::CIRCLE_HALF, "light.section.shadows", true, focus == Some(Section::Shadows), |ui| self.light_shadows_body(ui));
                section(ui, icons::CLOUD_SUN, "light.section.ambient", true, focus == Some(Section::Ambient), |ui| self.light_ambient_body(ui));
                section(ui, icons::SPARKLE, "light.section.rim", false, false, |ui| self.light_rim_body(ui));
                section(ui, icons::FLASHLIGHT, "light.section.extra", false, focus == Some(Section::Extra), |ui| Self::light_extra_body(ui));
                section(ui, icons::STACK, "light.section.test", false, false, |ui| self.light_test_body(ui));
            });
        });
    }

    // ================================================================= rigs

    fn light_rigs_body(&mut self, ui: &mut egui::Ui) {
        hint(ui, tr("light.rigs.hint"));
        let ppp = ui.ctx().pixels_per_point();
        let mut centers: Vec<(&'static str, Vec2)> = Vec::new();
        let mut clicked: Option<&'static str> = None;
        let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        for pair in RIGS.chunks(2) {
            ui.horizontal(|ui| {
                for rig in pair {
                    let selected = self.light_ui.last_rig == Some(rig.id);
                    let btn = egui::Button::new(format!("{}  {}", rig.icon, tr(rig.label_key))).min_size(egui::vec2(w, 28.0)).selected(selected);
                    let r = ui.add(btn).on_hover_text(tr(rig.tip_key));
                    centers.push((rig.id, Vec2::new(r.rect.center().x * ppp, r.rect.center().y * ppp)));
                    if r.clicked() {
                        clicked = Some(rig.id);
                    }
                }
            });
        }
        self.light_ui.rig_px = centers;
        if let Some(id) = clicked {
            self.light_apply_rig(id);
        }
        ui.add_space(4.0);
        let mut follow = self.light_ui.follow_camera;
        if ui.checkbox(&mut follow, format!("{}  {}", icons::VIDEO_CAMERA, tr("light.follow_camera"))).on_hover_text(tr("light.follow_camera_tip")).changed() {
            self.light_set_follow(follow);
        }
        if self.light_ui.follow_camera {
            hint(ui, tr("light.follow_hint"));
            if ui.button(format!("{}  {}", icons::CHECK, tr("light.follow_apply"))).on_hover_text(tr("light.follow_apply_tip")).clicked() {
                self.light_bake_follow();
            }
        }
    }

    // ================================================================= sol

    fn light_sun_body(&mut self, ui: &mut egui::Ui) {
        let v = self.light_view();
        let follow = self.light_ui.follow_camera;
        let (mut az, mut el) = self.light_current_angles();
        let mut dir_changed = false;
        if follow {
            hint(ui, tr("light.relative_hint"));
        }
        dir_changed |= row(ui, tr("light.azimuth"), |ui| slider(ui, &mut az, -180.0..=180.0, "\u{b0}", 1));
        dir_changed |= row(ui, tr("light.elevation"), |ui| slider(ui, &mut el, -90.0..=90.0, "\u{b0}", 1));
        if dir_changed {
            if follow {
                self.light_ui.rel_az = az;
                self.light_ui.rel_el = el;
            } else {
                let d = lighting::dir_from_az_el(az, el);
                self.light_edit_patch(LightPatch { direction: Some(d.to_array()), ..Default::default() });
            }
        }
        hint(ui, tr("light.az_tip"));

        ui.add_space(4.0);
        let mut color = v.light.color;
        let mut color_changed = false;
        row(ui, tr("light.color"), |ui| {
            color_changed |= ui.color_edit_button_rgb(&mut color).changed();
        });
        // temperatura: régua de corpo negro + slider em Kelvin (escreve a COR da luz)
        row(ui, tr("light.temperature"), |ui| {
            let mut k = self.light_ui.temp_k;
            if slider(ui, &mut k, 1500.0..=12000.0, " K", 0) {
                self.light_ui.temp_k = k;
                color = lighting::kelvin_to_rgb(k);
                color_changed = true;
            }
        });
        gradient_strip(ui, 6.0, |t| rgb8(lighting::kelvin_to_rgb(1500.0 + t * (12000.0 - 1500.0))));
        if color_changed {
            self.light_edit_patch(LightPatch { color: Some(color), ..Default::default() });
        }
        let mut intensity = v.light.intensity;
        if row(ui, tr("light.intensity"), |ui| slider(ui, &mut intensity, 0.0..=3.0, "", 2)) {
            self.light_edit_patch(LightPatch { intensity: Some(intensity), ..Default::default() });
        }
        ui.add_space(2.0);
        if ui.button(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("light.reset_sun"))).clicked() {
            let d = anigo_core::StylizedLight::default();
            let id = v.id.clone();
            self.light_apply(LightPatch { direction: Some(d.direction), color: Some(d.color), intensity: Some(d.intensity), ..Default::default() }.into_command(id));
            self.light_ui.temp_k = 5600.0;
        }
    }

    // ================================================================= sombras

    fn light_shadows_body(&mut self, ui: &mut egui::Ui) {
        let v = self.light_view();
        // ---- terminador (material do personagem)
        ui.label(RichText::new(tr("light.terminator")).strong().size(12.5));
        if let Some(m) = &v.material {
            let mut threshold = m.shadow_threshold;
            let mut smooth = m.shadow_smoothness;
            if row(ui, tr("light.threshold"), |ui| slider(ui, &mut threshold, 0.2..=0.8, "", 2)) {
                self.light_edit_material(MaterialPatch { shadow_threshold: Some(threshold), ..Default::default() });
            }
            if row(ui, tr("light.penumbra"), |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 104.0).max(60.0);
                ui.add(egui::Slider::new(&mut smooth, 0.005..=0.4).logarithmic(true).max_decimals(3).clamping(egui::SliderClamping::Always)).changed()
            }) {
                self.light_edit_material(MaterialPatch { shadow_smoothness: Some(smooth), ..Default::default() });
            }
            hint(ui, tr("light.terminator_tip"));
        }
        // ---- tinta
        ui.add_space(4.0);
        ui.label(RichText::new(tr("light.shadow_tint_title")).strong().size(12.5));
        let mut tint = v.light.shadow_color;
        let mut sat = v.light.shadow_saturation;
        if row(ui, tr("light.shadow_tint"), |ui| ui.color_edit_button_rgb(&mut tint).changed()) {
            self.light_edit_patch(LightPatch { shadow_color: Some(tint), ..Default::default() });
        }
        if row(ui, tr("light.shadow_sat"), |ui| slider(ui, &mut sat, 0.0..=2.0, "", 2)) {
            self.light_edit_patch(LightPatch { shadow_saturation: Some(sat), ..Default::default() });
        }
        hint(ui, tr("light.shadow_tint_tip"));

        // ---- sombra projetada (shadow map de 1 luz)
        ui.add_space(6.0);
        ui.separator();
        ui.label(RichText::new(format!("{}  {}", icons::CIRCLE_HALF, tr("light.cast"))).strong().size(12.5));
        let supported = self.gpu.as_ref().and_then(|g| g.cel.as_ref()).is_none_or(|c| c.shadow_supported());
        let mut sh: LightShadow = v.light.shadow;
        let mut changed = false;
        ui.add_enabled_ui(supported, |ui| {
            changed |= ui.checkbox(&mut sh.enabled, tr("light.cast_on")).on_hover_text(tr("light.cast_tip")).changed();
            ui.add_enabled_ui(sh.enabled, |ui| {
                changed |= row(ui, tr("light.cast_strength"), |ui| slider(ui, &mut sh.strength, 0.0..=1.0, "", 2));
                changed |= row(ui, tr("light.cast_hardness"), |ui| slider(ui, &mut sh.hardness, 0.0..=1.0, "", 2));
                changed |= row(ui, tr("light.cast_bias"), |ui| slider(ui, &mut sh.bias, 0.0..=0.25, " m", 3));
                changed |= row(ui, tr("light.cast_normal_bias"), |ui| slider(ui, &mut sh.normal_bias, 0.0..=6.0, " tx", 1));
                row(ui, tr("light.cast_res"), |ui| {
                    egui::ComboBox::from_id_salt("light_shadow_res").selected_text(format!("{0}\u{d7}{0}", sh.resolution)).show_ui(ui, |ui| {
                        for r in LightShadow::RESOLUTIONS {
                            if ui.selectable_value(&mut sh.resolution, r, format!("{r}\u{d7}{r}")).changed() {
                                changed = true;
                            }
                        }
                    });
                });
                changed |= row(ui, tr("light.cast_distance"), |ui| {
                    let auto = sh.distance <= 0.0;
                    let mut d = if auto { 0.0 } else { sh.distance };
                    ui.spacing_mut().slider_width = (ui.available_width() - 110.0).max(60.0);
                    let r = ui.add(egui::Slider::new(&mut d, 0.0..=60.0).custom_formatter(|n, _| if n <= 0.05 { tr("light.auto").to_string() } else { format!("{n:.1} m") }).clamping(egui::SliderClamping::Always));
                    if r.changed() {
                        sh.distance = if d <= 0.05 { 0.0 } else { d };
                    }
                    r.changed()
                });
            });
        });
        if changed {
            self.light_edit_patch(LightPatch { shadow: Some(sh), ..Default::default() });
        }
        if !supported {
            hint(ui, tr("light.cast_unsupported"));
        } else {
            hint(ui, tr("light.cast_note"));
        }
    }

    // ================================================================= ambiente

    fn light_ambient_body(&mut self, ui: &mut egui::Ui) {
        let v = self.light_view();
        let mut amb = v.light.ambient_intensity;
        let (mut sky, mut ground) = (v.light.ambient_sky, v.light.ambient_ground);
        if row(ui, tr("light.ambient"), |ui| slider(ui, &mut amb, 0.0..=2.0, "", 2)) {
            self.light_edit_patch(LightPatch { ambient_intensity: Some(amb), ..Default::default() });
        }
        if row(ui, tr("light.sky"), |ui| ui.color_edit_button_rgb(&mut sky).changed()) {
            self.light_edit_patch(LightPatch { ambient_sky: Some(sky), ..Default::default() });
        }
        if row(ui, tr("light.ground"), |ui| ui.color_edit_button_rgb(&mut ground).changed()) {
            self.light_edit_patch(LightPatch { ambient_ground: Some(ground), ..Default::default() });
        }
        // hemisfério: do chão (esquerda) ao céu (direita), na intensidade do ambiente
        gradient_strip(ui, 8.0, |t| {
            let k = amb.clamp(0.0, 1.0);
            let c = [0, 1, 2].map(|i| (ground[i] * (1.0 - t) + sky[i] * t) * k.max(0.15));
            rgb8(c)
        });
        hint(ui, tr("light.hemi_tip"));
    }

    // ================================================================= rim

    fn light_rim_body(&mut self, ui: &mut egui::Ui) {
        let v = self.light_view();
        let Some(m) = v.material else {
            hint(ui, tr("light.no_material"));
            return;
        };
        let (mut intensity, mut spread, mut color) = (m.rim_intensity, m.rim_spread, [m.rim_color[0], m.rim_color[1], m.rim_color[2]]);
        if row(ui, tr("light.rim_intensity"), |ui| slider(ui, &mut intensity, 0.0..=3.0, "", 2)) {
            self.light_edit_material(MaterialPatch { rim_intensity: Some(intensity), ..Default::default() });
        }
        if row(ui, tr("light.rim_spread"), |ui| slider(ui, &mut spread, 0.05..=0.95, "", 2)) {
            self.light_edit_material(MaterialPatch { rim_spread: Some(spread), ..Default::default() });
        }
        if row(ui, tr("light.rim_color"), |ui| ui.color_edit_button_rgb(&mut color).changed()) {
            self.light_edit_material(MaterialPatch { rim_color: Some([color[0], color[1], color[2], m.rim_color[3]]), ..Default::default() });
        }
        hint(ui, tr("light.rim_tip"));
    }

    // ================================================================= luzes extras (desabilitadas)

    fn light_extra_body(ui: &mut egui::Ui) {
        hint(ui, tr("light.extra_hint"));
        ui.horizontal(|ui| {
            for (icon, key) in [(icons::LIGHTBULB, "light.extra.point"), (icons::FLASHLIGHT, "light.extra.spot"), (icons::SQUARE, "light.extra.rect")] {
                ui.add_enabled(false, egui::Button::new(format!("{icon}  {}", tr(key)))).on_disabled_hover_text(tr("light.extra_tip"));
            }
        });
    }

    // ================================================================= cena de prova

    fn light_test_body(&mut self, ui: &mut egui::Ui) {
        hint(ui, tr("light.test_hint"));
        if ui.button(format!("{}  {}", icons::SQUARE, tr("light.add_floor"))).on_hover_text(tr("light.add_floor_tip")).clicked() {
            self.light_add_test_floor();
        }
        if ui.button(format!("{}  {}", icons::SPHERE, tr("light.add_hair"))).on_hover_text(tr("light.add_hair_tip")).clicked() {
            self.light_add_hair_proxy();
        }
    }
}
