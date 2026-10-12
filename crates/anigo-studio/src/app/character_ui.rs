//! Workspace Personagem nativo: viewport cel-shaded do personagem deformado pelo núcleo,
//! painel de sliders por zona anatômica, macros (gênero, somatótipo, proporções), presets e
//! persistência do projeto.
//!
//! O estado autoritativo é o `anigo_session::CoreSession` (`StudioApp::core`). A UI só cria
//! `Command`s (ver `crate::character`); o núcleo valida, aplica e desfaz, e a malha deformada
//! vem de `CoreSession::sync_scene` — nada é deformado aqui.

use super::StudioApp;
use crate::character::macros::{self, Proportion, PAD_ECTO, PAD_ENDO, PAD_H, PAD_MESO, PAD_W};
use crate::character::sliders::{self, Region};
use crate::character::{persist, presets};
use crate::i18n::{lang, tr};
use crate::icons;
use crate::theme;
use crate::viewport::OrbitCamera;
use crate::workbench::{px_size, ShadingMode, ViewSettings};
use anigo_core::command::Command;
use anigo_core::morph_catalog::MorphSliderDef;
use anigo_core::project::ProjectState;
use eframe::wgpu;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2 as EVec2};
use glam::Vec3;

/// Gesto em andamento (arrasto de slider/pad): o último comando pré-visualizado e, quando o
/// comando final difere dele, o que deve ser gravado no histórico ao soltar.
pub(crate) struct Gesture {
    last: Command,
    commit: Option<Command>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogKind {
    Save,
    Open,
}

pub(crate) struct ProjectDialog {
    kind: DialogKind,
    path: String,
    error: Option<String>,
}

pub(crate) struct CharacterUi {
    pub camera: OrbitCamera,
    /// Câmera já enquadrada no personagem (acontece quando a primeira malha chega).
    pub framed: bool,
    pub turntable: bool,
    pub light_follows_camera: bool,
    pub search: String,
    pub gesture: Option<Gesture>,
    pub dialog: Option<ProjectDialog>,
    pub last_path: String,
    pub message: String,
}

impl Default for CharacterUi {
    fn default() -> Self {
        Self {
            camera: OrbitCamera {
                target: Vec3::new(0.0, 0.85, 0.0),
                yaw: 0.45,
                pitch: 0.12,
                distance: 3.4,
                ..OrbitCamera::default()
            },
            framed: false,
            turntable: false,
            light_follows_camera: false,
            search: String::new(),
            gesture: None,
            dialog: None,
            last_path: "personagem.anigo".to_string(),
            message: String::new(),
        }
    }
}

/// Enquadra o personagem inteiro (mantém yaw/pitch).
pub(crate) fn frame_character(cam: &mut OrbitCamera, lo: Vec3, hi: Vec3) {
    let h = (hi.y - lo.y).max(0.2);
    cam.target = Vec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, (lo.z + hi.z) * 0.5);
    cam.distance = (h * 0.5 / (cam.fov_y * 0.5).tan() * 1.15).clamp(0.3, 50.0);
}

/// Enquadra a cabeça (parte alta da caixa).
pub(crate) fn frame_face(cam: &mut OrbitCamera, lo: Vec3, hi: Vec3) {
    let h = (hi.y - lo.y).max(0.2);
    cam.target = Vec3::new((lo.x + hi.x) * 0.5, hi.y - h * 0.075, (lo.z + hi.z) * 0.5);
    cam.distance = (h * 0.13 / (cam.fov_y * 0.5).tan() * 1.35).clamp(0.2, 50.0);
}

impl StudioApp {
    fn char_tool_id(&self) -> &'static str {
        let idx = *self.tool_idx.get(&self.ws).unwrap_or(&0);
        crate::workspace::PERSONAGEM.get(idx).map(|t| t.id).unwrap_or("body")
    }

    // ------------------------------------------------------------ comandos / gesto

    /// Comando discreto (clique): vai direto ao histórico do núcleo.
    fn char_apply(&mut self, cmd: Command) {
        match self.core.apply(cmd) {
            Ok(o) => self.char_ui.message = describe(&o.description),
            Err(e) => self.char_report(e),
        }
    }

    fn char_apply_composed(&mut self, r: Result<Option<Command>, anigo_core::command::CommandError>) {
        match r {
            Ok(Some(cmd)) => self.char_apply(cmd),
            Ok(None) => {}
            Err(e) => self.char_report(e.to_string()),
        }
    }

    /// "Sem efeito" não é erro para o usuário; o resto aparece na barra de status.
    fn char_report(&mut self, e: String) {
        if !e.contains("no effect") && !e.contains("already") {
            self.char_ui.message = format!("{}: {e}", tr("char.error"));
        }
    }

    /// Passo de um arrasto: aplica ao vivo (sem histórico). `commit` é o comando final quando
    /// ele precisa diferir do último passo (ex.: dimorfismo também troca o gênero base).
    fn char_drag(&mut self, cmd: Command, commit: Option<Command>) {
        if let Err(e) = self.core.preview(cmd.clone()) {
            self.char_report(e);
        }
        self.char_ui.gesture = Some(Gesture { last: cmd, commit });
    }

    /// Ao soltar o ponteiro, o gesto vira UM passo de histórico.
    fn char_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.char_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(g) = self.char_ui.gesture.take() {
            let fin = g.commit.unwrap_or(g.last);
            match self.core.commit_preview(fin) {
                Ok(o) => self.char_ui.message = describe(&o.description),
                Err(e) => self.char_report(e),
            }
        }
    }

    /// Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y no workspace Personagem (fora de campos de texto).
    pub(crate) fn character_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        let redo = ctx.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y)
        });
        if redo {
            self.undo_redo(false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.undo_redo(true);
        }
    }

    // ---------------------------------------------------------------- painel direito

    pub(crate) fn inspector_character(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("char_inspector")
            .default_size(350.0)
            .size_range(290.0..=540.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr(&format!("tool.{}", self.char_tool_id()))).strong().size(14.0));
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let st = self.core.project().clone();
                    self.char_project_section(ui);
                    match self.char_tool_id() {
                        "body" => {
                            self.char_macro_sections(ui, &st);
                            self.char_slider_list(ui, &st, Region::Body);
                        }
                        "face" => self.char_slider_list(ui, &st, Region::Face),
                        other => self.char_soon_panel(ui, &st, other),
                    }
                    self.char_history_section(ui);
                });
            });
        let ctx = ui.ctx().clone();
        self.char_project_dialog(&ctx);
        self.char_finish_gesture(&ctx);
    }

    fn char_project_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{}  {}", icons::FLOPPY_DISK, tr("char.save"))).clicked() {
                self.char_ui.dialog = Some(ProjectDialog {
                    kind: DialogKind::Save,
                    path: self.char_ui.last_path.clone(),
                    error: None,
                });
            }
            if ui.button(format!("{}  {}", icons::FOLDER_OPEN, tr("char.open"))).clicked() {
                self.char_ui.dialog = Some(ProjectDialog {
                    kind: DialogKind::Open,
                    path: self.char_ui.last_path.clone(),
                    error: None,
                });
            }
            let fresh = macros::reset_all(self.core.project()).ok().flatten().is_none();
            if ui
                .add_enabled(!fresh, egui::Button::new(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, tr("char.reset_all"))))
                .clicked()
            {
                let r = macros::reset_all(self.core.project());
                self.char_apply_composed(r);
            }
        });
        if !self.char_ui.message.is_empty() {
            ui.label(RichText::new(&self.char_ui.message).size(11.5).color(theme::TEXT_DIM));
        }
        ui.add_space(2.0);
    }

    fn char_macro_sections(&mut self, ui: &mut egui::Ui, st: &ProjectState) {
        // ---- gênero
        egui::CollapsingHeader::new(RichText::new(tr("char.gender")).strong())
            .default_open(true)
            .show(ui, |ui| {
                use anigo_core::mesh::BaseGender;
                let male = st.character.gender_dimorphism >= 0.5;
                ui.horizontal(|ui| {
                    if ui.selectable_label(male, format!("{}  {}", icons::GENDER_MALE, tr("char.male"))).clicked() && !male {
                        let r = macros::set_gender(self.core.project(), BaseGender::Male);
                        self.char_apply_composed(r);
                    }
                    if ui.selectable_label(!male, format!("{}  {}", icons::GENDER_FEMALE, tr("char.female"))).clicked() && male {
                        let r = macros::set_gender(self.core.project(), BaseGender::Female);
                        self.char_apply_composed(r);
                    }
                });
                let mut g = st.character.gender_dimorphism;
                ui.label(RichText::new(format!("{}  ({} 0 — 1 {})", tr("char.dimorphism"), tr("char.female"), tr("char.male"))).size(12.0).color(theme::TEXT_DIM));
                let resp = full_width_slider(ui, &mut g, 0.0, 1.0, 2);
                if resp.changed() {
                    self.char_drag(macros::dimorphism_preview(g), Some(macros::dimorphism_final(g)));
                }
            });

        // ---- somatótipo
        egui::CollapsingHeader::new(RichText::new(tr("char.somatotype")).strong())
            .default_open(true)
            .show(ui, |ui| self.char_somatotype_pad(ui, st));

        // ---- proporções
        egui::CollapsingHeader::new(RichText::new(tr("char.proportions")).strong())
            .default_open(true)
            .show(ui, |ui| {
                for p in Proportion::ALL {
                    let (lo, hi) = p.range();
                    let cur = p.get(&st.character.proportions);
                    let dflt = p.default_value();
                    ui.horizontal(|ui| {
                        ui.label(tr(p.label_key()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if reset_button(ui, (cur - dflt).abs() > 1e-6).clicked() {
                                self.char_apply(p.command(dflt));
                            }
                        });
                    });
                    let mut v = cur;
                    if full_width_slider(ui, &mut v, lo, hi, 2).changed() {
                        self.char_drag(p.command(v), None);
                    }
                }
            });

        // ---- presets
        egui::CollapsingHeader::new(RichText::new(tr("char.presets")).strong())
            .default_open(false)
            .show(ui, |ui| {
                for p in presets::PRESETS {
                    ui.horizontal(|ui| {
                        if ui.button(tr(&p.name_key())).on_hover_text(tr(&p.description_key())).clicked() {
                            let r = presets::apply_preset(self.core.project(), p);
                            self.char_apply_composed(r);
                        }
                        ui.label(RichText::new(tr(&p.description_key())).size(11.0).color(theme::TEXT_DIM));
                    });
                }
            });
        ui.add_space(4.0);
    }

    /// Triângulo de Sheldon arrastável (porte de `SomatotypePad2D.svelte`).
    fn char_somatotype_pad(&mut self, ui: &mut egui::Ui, st: &ProjectState) {
        let w = ui.available_width().clamp(160.0, 300.0);
        let (rect, resp) = ui.allocate_exact_size(EVec2::new(w, w * PAD_H / PAD_W), Sense::click_and_drag());
        let map = |p: (f32, f32)| Pos2::new(rect.min.x + p.0 / PAD_W * rect.width(), rect.min.y + p.1 / PAD_H * rect.height());
        let (meso, endo, ecto) = (map(PAD_MESO), map(PAD_ENDO), map(PAD_ECTO));
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, 4.0, theme::BG_DEEP);
        painter.add(egui::Shape::convex_polygon(
            vec![meso, endo, ecto],
            Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xff, 46),
            Stroke::new(1.5, theme::BORDER),
        ));
        // medianas tracejadas até o centro neutro
        let center = map(macros::somatotype_to_pad(anigo_core::somatotype::SomatotypeCoords::default()));
        for (a, b) in [(meso, mid(endo, ecto)), (endo, mid(meso, ecto)), (ecto, mid(meso, endo))] {
            painter.line_segment([a, b], Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 120, 150, 70)));
        }
        painter.circle_filled(center, 2.5, theme::TEXT_DIM);
        let font = egui::FontId::proportional(11.0);
        painter.text(meso + EVec2::new(0.0, 3.0), egui::Align2::CENTER_TOP, tr("somatotype.meso"), font.clone(), theme::TEXT_DIM);
        painter.text(endo + EVec2::new(4.0, -2.0), egui::Align2::LEFT_BOTTOM, tr("somatotype.endo"), font.clone(), theme::TEXT_DIM);
        painter.text(ecto + EVec2::new(-4.0, -2.0), egui::Align2::RIGHT_BOTTOM, tr("somatotype.ecto"), font, theme::TEXT_DIM);

        let cur = st.character.somatotype.normalized();
        let puck = map(macros::somatotype_to_pad(cur));
        painter.circle_filled(puck, 7.0, theme::ACCENT);
        painter.circle_stroke(puck, 7.0, Stroke::new(1.5, Color32::WHITE));

        if resp.is_pointer_button_down_on() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let px = (pos.x - rect.min.x) / rect.width() * PAD_W;
                let py = (pos.y - rect.min.y) / rect.height() * PAD_H;
                let c = macros::pad_to_somatotype(px, py);
                let same = (c.endomorph - cur.endomorph).abs() < 1e-5
                    && (c.mesomorph - cur.mesomorph).abs() < 1e-5
                    && (c.ectomorph - cur.ectomorph).abs() < 1e-5;
                if !same {
                    self.char_drag(macros::somatotype_command(c), None);
                }
            }
        }
        ui.label(
            RichText::new(format!(
                "{} {:.0}%   {} {:.0}%   {} {:.0}%",
                tr("somatotype.endo"),
                cur.endomorph * 100.0,
                tr("somatotype.meso"),
                cur.mesomorph * 100.0,
                tr("somatotype.ecto"),
                cur.ectomorph * 100.0
            ))
            .size(11.5)
            .color(theme::TEXT_DIM),
        );
        ui.horizontal_wrapped(|ui| {
            for (key, e, m, c) in macros::SOMATOTYPE_SHORTCUTS {
                if ui.small_button(tr(key)).clicked() {
                    self.char_apply(macros::somatotype_command(anigo_core::somatotype::SomatotypeCoords::new(e, m, c)));
                }
            }
        });
    }

    fn char_slider_list(&mut self, ui: &mut egui::Ui, st: &ProjectState, region: Region) {
        ui.horizontal(|ui| {
            ui.label(icons::MAGNIFYING_GLASS);
            ui.add(
                egui::TextEdit::singleline(&mut self.char_ui.search)
                    .hint_text(tr("char.search_hint"))
                    .desired_width(ui.available_width() - 30.0),
            );
            if !self.char_ui.search.is_empty() && ui.small_button(icons::X).clicked() {
                self.char_ui.search.clear();
            }
        });
        let query = self.char_ui.search.clone();
        let groups = sliders::visible_groups(region, &query);
        if groups.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(tr("char.no_results")).color(theme::TEXT_DIM));
            return;
        }
        for (i, (zone, defs)) in groups.into_iter().enumerate() {
            let modified = sliders::modified_in_zone(st, zone);
            let mut title = format!("{}  ({})", sliders::zone_label(zone), defs.len());
            if modified > 0 {
                title.push_str(&format!("  ·  {modified} {}", tr("char.modified")));
            }
            let mut header = egui::CollapsingHeader::new(RichText::new(title).strong())
                .id_salt(sliders::zone_key(zone))
                .default_open(i == 0);
            if !query.trim().is_empty() {
                header = header.open(Some(true));
            }
            header.show(ui, |ui| {
                for def in defs {
                    self.char_slider_row(ui, st, def);
                }
            });
        }
    }

    fn char_slider_row(&mut self, ui: &mut egui::Ui, st: &ProjectState, def: &'static MorphSliderDef) {
        let value = sliders::slider_value(st, def);
        let modified = sliders::is_modified(st, def);
        let dimmed = sliders::is_dimorphism_dimmed(def, st.character.gender_dimorphism);
        let label = sliders::slider_label(def, lang());
        ui.horizontal(|ui| {
            let mut text = RichText::new(label).size(12.5);
            if dimmed {
                text = text.color(theme::TEXT_DIM);
            } else if modified {
                text = text.color(theme::ACCENT);
            }
            let resp = ui.add(egui::Label::new(text).truncate());
            use anigo_core::morph_catalog::GenderDimorphism as G;
            let hint = match def.dimorphism {
                G::FemaleOnly => Some((icons::GENDER_FEMALE, tr("char.female_only"))),
                G::MaleOnly => Some((icons::GENDER_MALE, tr("char.male_only"))),
                G::Both => None,
            };
            if let Some((_, tip)) = hint {
                resp.on_hover_text(tip);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reset_button(ui, modified).clicked() {
                    self.char_apply(sliders::slider_reset_command(def));
                }
                if let Some((icon, tip)) = hint {
                    ui.label(RichText::new(icon).size(13.0).color(if dimmed { theme::TEXT_DIM } else { theme::ACCENT })).on_hover_text(tip);
                }
            });
        });
        let mut v = value;
        let resp = full_width_slider(ui, &mut v, def.min, def.max, sliders::decimals(def));
        if resp.changed() {
            self.char_drag(sliders::slider_command(def, v), None);
        }
        ui.add_space(2.0);
    }

    fn char_history_section(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new(tr("char.history")).strong())
            .default_open(false)
            .show(ui, |ui| {
                let h = self.core.history();
                ui.label(RichText::new(format!("{} {}", h.undo_depth(), tr("char.history_steps"))).color(theme::TEXT_DIM).size(12.0));
                match h.last_undo_description() {
                    Some(d) => ui.label(RichText::new(format!("{}: {d}", tr("char.history_next"))).size(12.0)),
                    None => ui.label(RichText::new(tr("char.history_empty")).color(theme::TEXT_DIM).size(12.0)),
                };
            });
    }

    /// Painel honesto das ferramentas ainda não portadas: mostra o que o núcleo já guarda.
    fn char_soon_panel(&mut self, ui: &mut egui::Ui, st: &ProjectState, tool: &str) {
        ui.add_space(10.0);
        ui.label(RichText::new(tr("char.soon")).size(18.0).strong().color(theme::ACCENT));
        ui.add_space(4.0);
        ui.label(RichText::new(tr(&format!("char.soon.{tool}"))).color(theme::TEXT_DIM));
        ui.add_space(8.0);
        let c = &st.character;
        let rows: Vec<(String, String)> = match tool {
            "hair" => vec![
                (tr("hair.volume").into(), format!("{:.2}", c.hair.volume)),
                (tr("hair.thickness").into(), format!("{:.3}", c.hair.thickness)),
                (tr("hair.curvature").into(), format!("{:.2}", c.hair.curvature)),
                (tr("hair.strands").into(), c.hair.strands.to_string()),
            ],
            "cloth" => vec![
                (tr("cloth.layer").into(), c.cloth.layer.clone()),
                (tr("cloth.tension").into(), format!("{:.2}", c.cloth.tension)),
                (tr("cloth.rigidity").into(), format!("{:.2}", c.cloth.rigidity)),
                (tr("cloth.gravity").into(), format!("{:.2}", c.cloth.gravity)),
            ],
            "accessories" => vec![
                (tr("accessory.socket").into(), c.accessory.socket.clone()),
                (tr("accessory.scale").into(), format!("{:.2}", c.accessory.scale)),
                (
                    tr("accessory.offset").into(),
                    format!("{:.2}, {:.2}, {:.2}", c.accessory.offset[0], c.accessory.offset[1], c.accessory.offset[2]),
                ),
            ],
            _ => Vec::new(),
        };
        if rows.is_empty() {
            return;
        }
        ui.label(RichText::new(tr("char.soon.stored")).strong().size(12.5));
        egui::Grid::new(("soon_grid", tool)).num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(k).color(theme::TEXT_DIM));
                ui.label(v);
                ui.end_row();
            }
        });
    }

    // ------------------------------------------------------------------ salvar/abrir

    fn char_project_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.char_ui.dialog.take() else {
            return;
        };
        let mut keep = true;
        let mut confirm = false;
        let title = match dlg.kind {
            DialogKind::Save => tr("char.dialog.save"),
            DialogKind::Open => tr("char.dialog.open"),
        };
        egui::Window::new(title)
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
                    let ok = match dlg.kind {
                        DialogKind::Save => tr("char.save"),
                        DialogKind::Open => tr("char.open"),
                    };
                    if ui.button(ok).clicked() {
                        confirm = true;
                    }
                    if ui.button(tr("char.dialog.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
        if confirm {
            match self.char_run_dialog(&dlg) {
                Ok(()) => keep = false,
                Err(e) => dlg.error = Some(e),
            }
        }
        if keep {
            self.char_ui.dialog = Some(dlg);
        }
    }

    fn char_run_dialog(&mut self, dlg: &ProjectDialog) -> Result<(), String> {
        let path = persist::normalize_path(&dlg.path)?;
        match dlg.kind {
            DialogKind::Save => {
                persist::save_project(&self.core, &path)?;
                self.char_ui.message = format!("{}: {}", tr("char.saved"), path.display());
            }
            DialogKind::Open => {
                self.core.cancel_preview();
                persist::load_project(&mut self.core, &path)?;
                self.char_ui.gesture = None;
                self.char_ui.message = format!("{}: {}", tr("char.opened"), path.display());
            }
        }
        self.char_ui.last_path = path.display().to_string();
        Ok(())
    }

    // ---------------------------------------------------------------------- viewport

    pub(crate) fn character_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
            let ppp = ui.ctx().pixels_per_point();

            // turntable
            if self.char_ui.turntable && !resp.dragged() {
                let dt = ui.input(|i| i.stable_dt).min(0.1);
                self.char_ui.camera.yaw += dt * 0.6;
                ui.ctx().request_repaint();
            }
            self.render_character(frame, rect, ppp);

            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().rect_filled(rect, 0.0, theme::BG_DEEP);
            if let Some(g) = self.gpu.as_ref() {
                if let Some(t) = g.tex {
                    ui.painter().image(t, rect, uv, Color32::WHITE);
                }
                if let Some(t) = g.cel_tex {
                    ui.painter().image(t, rect, uv, Color32::WHITE);
                }
            }
            if self.gpu.as_ref().is_some_and(|g| g.cel.is_none()) {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    tr("char.no_preview"),
                    egui::FontId::proportional(14.0),
                    theme::TEXT_DIM,
                );
            }

            self.char_camera_input(ui, &resp, rect);
            self.char_viewport_header(ui, rect);
        });
    }

    fn char_camera_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect) {
        use egui::PointerButton;
        let size = glam::Vec2::new(rect.width(), rect.height());
        let aspect = size.x / size.y.max(1.0);
        let cam = &mut self.char_ui.camera;
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let factor = (1.0 - scroll * 0.0025).clamp(0.5, 1.5);
                let m = self.last_mouse - rect.min;
                let ndc = glam::Vec2::new(m.x / size.x * 2.0 - 1.0, 1.0 - m.y / size.y * 2.0);
                cam.zoom_to_cursor(factor, ndc, aspect);
            }
        }
        if let Some(p) = resp.hover_pos() {
            self.last_mouse = p;
        }
        let cam = &mut self.char_ui.camera;
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        let delta = ui.input(|i| i.pointer.delta());
        // sem ferramentas de seleção aqui: arrastar com o botão esquerdo também orbita
        let drag = resp.dragged_by(PointerButton::Primary) || resp.dragged_by(PointerButton::Middle);
        if drag {
            if shift {
                cam.pan(delta.x, delta.y, size.y);
            } else if ctrl {
                cam.zoom(1.0 + delta.y * 0.01);
            } else {
                cam.orbit(delta.x, delta.y);
            }
        }
    }

    fn char_viewport_header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let header = Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(rect.width() - 12.0, 30.0));
        let bounds = self.gpu.as_ref().and_then(|g| g.cel.as_ref()).and_then(|c| c.character_bounds);
        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(24, 24, 30, 215))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let cam = &mut self.char_ui.camera;
                        let views: [(&str, f32); 3] = [("char.view_front", 0.0), ("char.view_side", std::f32::consts::FRAC_PI_2), ("char.view_back", std::f32::consts::PI)];
                        for (key, yaw) in views {
                            if ui.button(tr(key)).clicked() {
                                cam.yaw = yaw;
                                cam.pitch = 0.05;
                                if let Some((lo, hi)) = bounds {
                                    frame_character(cam, lo, hi);
                                }
                            }
                        }
                        if ui.button(tr("char.view_face")).clicked() {
                            cam.yaw = 0.0;
                            cam.pitch = 0.0;
                            if let Some((lo, hi)) = bounds {
                                frame_face(cam, lo, hi);
                            }
                        }
                        if ui.button(tr("char.frame")).clicked() {
                            if let Some((lo, hi)) = bounds {
                                frame_character(cam, lo, hi);
                            }
                        }
                        ui.separator();
                        ui.toggle_value(&mut self.char_ui.turntable, format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("char.turntable")));
                        ui.toggle_value(&mut self.char_ui.light_follows_camera, format!("{}  {}", icons::SUN, tr("char.light_camera")));
                    });
                });
        });
    }

    fn render_character(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = px_size(glam::Vec2::new(rect.width(), rect.height()), ppp);
        let aspect = size_px.0 as f32 / size_px.1.max(1) as f32;
        let follow = self.char_ui.light_follows_camera;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let settings = ViewSettings { shading: ShadingMode::Solid, xray: false, ..self.settings };
        // fundo + grid do Workbench (cena vazia), com o personagem cel-shaded por cima
        let recreated = gpu.wb.render(size_px, &self.char_ui.camera, &[], &settings);
        let mut cel_recreated = false;
        if let Some(cel) = gpu.cel.as_mut() {
            cel.sync_character(&mut self.core, &self.char_ui.camera, aspect, follow);
            cel_recreated = cel.render(size_px);
            if let Some(e) = &cel.last_error {
                self.shader_error = Some(format!("cel: {e}"));
            }
            if !self.char_ui.framed {
                if let Some((lo, hi)) = cel.character_bounds {
                    frame_character(&mut self.char_ui.camera, lo, hi);
                    self.char_ui.framed = true;
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
}

/// Desfazer/refazer roteado pelo workspace ativo: Personagem fala com a sessão do núcleo
/// (`CoreSession`), os demais com a `ModelingSession`. Devolve a mensagem de status do núcleo.
pub(crate) fn route_undo_redo(
    ws: crate::workspace::WorkspaceId,
    core: &mut anigo_session::CoreSession,
    modeling: &mut crate::modeling::ModelingSession,
    undo: bool,
) -> Option<String> {
    if ws == crate::workspace::WorkspaceId::Personagem {
        let r = if undo { core.undo() } else { core.redo() };
        return Some(match r {
            Ok(o) => describe(&o.description),
            Err(e) => e,
        });
    }
    if undo {
        modeling.undo();
    } else {
        modeling.redo();
    }
    None
}

pub(crate) fn can_undo_redo(
    ws: crate::workspace::WorkspaceId,
    core: &anigo_session::CoreSession,
    modeling: &crate::modeling::ModelingSession,
) -> (bool, bool) {
    if ws == crate::workspace::WorkspaceId::Personagem {
        (core.history().can_undo(), core.history().can_redo())
    } else {
        (modeling.can_undo(), modeling.can_redo())
    }
}

/// Descrição de um comando do núcleo (em inglês, para log/undo) no idioma da UI.
pub(crate) fn describe(desc: &str) -> String {
    let (undo, redo) = (desc.strip_prefix("Undo "), desc.strip_prefix("Redo "));
    if let Some(rest) = undo {
        return format!("{}: {}", tr("char.undone"), describe(rest));
    }
    if let Some(rest) = redo {
        return format!("{}: {}", tr("char.redone"), describe(rest));
    }
    if let Some(id) = desc.strip_prefix("Morph ") {
        let name = anigo_core::morph_catalog::find_slider_def(id)
            .map(|d| sliders::slider_label(d, lang()))
            .unwrap_or_else(|| id.to_string());
        return name;
    }
    let key = match desc {
        "Proportions" => "char.proportions",
        "Somatotype" => "char.somatotype",
        "Gender dimorphism" => "char.dimorphism",
        "Reset morphs" => "char.reset_all",
        d if d.starts_with("Base gender") => "char.gender",
        d if d.starts_with("Batch") => "char.batch",
        _ => return desc.to_string(),
    };
    tr(key).to_string()
}

fn mid(a: Pos2, b: Pos2) -> Pos2 {
    Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// Slider de largura total (o valor numérico do próprio widget é editável).
fn full_width_slider(ui: &mut egui::Ui, value: &mut f32, min: f32, max: f32, decimals: usize) -> egui::Response {
    let w = (ui.available_width() - 70.0).max(80.0);
    ui.spacing_mut().slider_width = w;
    ui.add(
        egui::Slider::new(value, min..=max)
            .clamping(egui::SliderClamping::Always)
            .max_decimals(decimals)
            .min_decimals(decimals.min(2)),
    )
}

/// Botão de reset por slider (habilitado só quando difere do padrão).
fn reset_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).size(12.0)).small())
        .on_hover_text(tr("char.reset_slider"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_descriptions_are_translated_and_unknown_ones_pass_through() {
        crate::i18n::set_lang(crate::i18n::Lang::PtBr);
        assert_eq!(describe("Proportions"), "Proporções");
        assert_eq!(describe("Undo Somatotype"), format!("{}: Somatótipo", tr("char.undone")));
        assert!(describe("Morph jaw_v_line_taper").len() > 3);
        assert_eq!(describe("Algo novo"), "Algo novo");
    }

    #[test]
    fn undo_redo_is_routed_to_the_session_of_the_active_workspace() {
        use crate::modeling::{ModelingSession, PrimitiveKind};
        use crate::workspace::WorkspaceId;
        let mut core = anigo_session::CoreSession::new();
        let mut modeling = ModelingSession::with_default_cube();
        core.apply(crate::character::macros::Proportion::Height.command(1.2)).unwrap();
        modeling.add_primitive(PrimitiveKind::Capsule, glam::Vec3::X);
        let objs = modeling.doc.objects.len();

        // Personagem: desfaz só o núcleo
        assert_eq!(can_undo_redo(WorkspaceId::Personagem, &core, &modeling), (true, false));
        assert!(route_undo_redo(WorkspaceId::Personagem, &mut core, &mut modeling, true).is_some());
        assert_eq!(core.project().character.proportions.height_overall, 1.0);
        assert_eq!(modeling.doc.objects.len(), objs, "a modelagem não foi tocada");
        assert_eq!(can_undo_redo(WorkspaceId::Personagem, &core, &modeling), (false, true));
        route_undo_redo(WorkspaceId::Personagem, &mut core, &mut modeling, false);
        assert_eq!(core.project().character.proportions.height_overall, 1.2);

        // Modelagem: desfaz só a ModelingSession
        assert!(route_undo_redo(WorkspaceId::Modelagem, &mut core, &mut modeling, true).is_none());
        assert_eq!(modeling.doc.objects.len(), objs - 1);
        assert_eq!(core.project().character.proportions.height_overall, 1.2, "o núcleo não foi tocado");
        // desfazer sem histórico não quebra
        assert!(route_undo_redo(WorkspaceId::Personagem, &mut core, &mut modeling, true).is_some());
        assert!(route_undo_redo(WorkspaceId::Personagem, &mut core, &mut modeling, true).is_some());
    }

    #[test]
    fn framing_keeps_the_whole_character_in_view() {
        let mut cam = OrbitCamera::default();
        frame_character(&mut cam, Vec3::new(-0.4, 0.0, -0.2), Vec3::new(0.4, 1.7, 0.2));
        // a altura de 1.7 m cabe no campo vertical à distância escolhida
        let visible = 2.0 * cam.distance * (cam.fov_y * 0.5).tan();
        assert!(visible > 1.7 && visible < 3.0, "{visible}");
        assert!((cam.target.y - 0.85).abs() < 1e-5);
        frame_face(&mut cam, Vec3::new(-0.4, 0.0, -0.2), Vec3::new(0.4, 1.7, 0.2));
        assert!(cam.target.y > 1.5 && cam.distance < 1.5);
    }
}
