//! Modo Cabelo no viewport: entrada (desenhar / editar pontos / pentear), painel do Inspector,
//! overlays e saída (exportar GLB e cadeias de física).
//!
//! A lógica mora em `modeling::hair`; aqui ficam a tradução da entrada do egui, o painel e o desenho.
//!
//! Atalhos (com o ponteiro no viewport):
//! `D` desenhar · `V` editar pontos · `C` pentear · `Shift+D` duplicar mecha · `X`/`Del` remover ponto
//! (ou mecha) · `Shift+X` remover mecha · `M` simetria X do grupo · `Shift+M` espelhar grupo ·
//! `[` `]` raio do pente · `Esc` cancela o traço · `Ctrl+clique` na mecha insere ponto ·
//! `Ctrl+Z` / `Ctrl+Shift+Z` desfaz / refaz · `Alt+arrastar` ou botão do meio orbita.

use super::props_tabs::PropTab;
use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::modeling::hair::{HairPick, HairTool};
use crate::modeling::sculpt::ViewInfo;
use crate::theme;
use anigo_modeling::hair::{SectionKind, StrandParams, StrandRef, UvLayout};
use egui::{Color32, Key, Pos2, Rect, RichText, Stroke};
use glam::{Vec2, Vec3};

fn gv(p: egui::Vec2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    crate::app::props_tabs::header(ui, RichText::new(title).strong())
        .id_salt(("hair_section", title))
        .default_open(open)
        .show(ui, add);
}

fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

/// Cor da mecha no overlay: a do material, clareada para contrastar com o fundo escuro.
fn overlay_color(c: [f32; 3]) -> Color32 {
    let l = |v: f32| (0.60 + 0.40 * v).clamp(0.0, 1.0);
    rgb([l(c[0]), l(c[1]), l(c[2])])
}

impl StudioApp {
    // ======================================================================= entrada

    /// Entrada do viewport no Modo Cabelo. Devolve `true` se consumiu o botão primário (o viewport não
    /// seleciona nem faz caixa de seleção).
    pub(crate) fn hair_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect, size: Vec2) -> bool {
        let idx = *self.tool_idx.get(&self.ws).unwrap_or(&0);
        self.hair.tool = HairTool::from_index(idx);
        self.hair.sync(&self.modeling);
        self.hair.prune(&self.modeling);
        let view = ViewInfo { camera: self.camera, size };
        let mouse = gv(self.last_mouse - rect.min);

        // roteiro (cenários): roda uma vez, com o tamanho real do viewport
        if let Some(steps) = self.hair.script.take() {
            self.hair.run_script(&mut self.modeling, &view, &steps);
            if let Some(e) = self.hair.error.clone() {
                self.modeling.status = e;
            }
            ui.ctx().request_repaint();
        }

        let (alt, ctrl, pressed, down, released) = ui.input(|i| {
            (
                i.modifiers.alt,
                i.modifiers.command,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
            )
        });
        let hovered = resp.hovered();
        let tool = self.hair.tool;

        // hover (anel do pincel)
        if hovered && !self.hair.is_busy() {
            self.hair.update_hover(&self.modeling, view.ray(mouse));
        } else if !hovered {
            self.hair.hover = None;
        }

        // ---- traço de desenho em andamento
        if self.hair.is_drawing() {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.hair.cancel_draw();
                self.modeling.status = tr("hairtool.stroke_cancelled").to_string();
                return true;
            }
            for p in self.pointer_samples(ui, rect) {
                self.hair.continue_draw(&self.modeling, view.ray(p));
            }
            if released || !down {
                self.hair.continue_draw(&self.modeling, view.ray(mouse));
                match self.hair.end_draw(&mut self.modeling) {
                    Ok(_) => {}
                    Err(e) => self.modeling.status = e,
                }
            }
            ui.ctx().request_repaint();
            return true;
        }
        // ---- arrasto de ponto de controle
        if self.hair.is_dragging_point() {
            if down && !released {
                if let Err(e) = self.hair.drag_point(&mut self.modeling, &view, mouse) {
                    self.modeling.status = e;
                }
            } else {
                self.hair.end_point_drag();
            }
            ui.ctx().request_repaint();
            return true;
        }
        // ---- pente em andamento
        if self.hair.is_combing() {
            if down && !released {
                for p in self.pointer_samples(ui, rect) {
                    if let Err(e) = self.hair.comb_move(&mut self.modeling, &view, p) {
                        self.modeling.status = e;
                    }
                }
            } else {
                self.hair.end_comb();
            }
            ui.ctx().request_repaint();
            return true;
        }

        // ---- início de gesto
        if pressed && hovered && !alt {
            match tool {
                HairTool::Draw => match self.hair.begin_draw(&mut self.modeling, view.ray(mouse)) {
                    Ok(true) => {}
                    Ok(false) => self.modeling.status = tr("hairtool.start_on_head").to_string(),
                    Err(e) => self.modeling.status = e,
                },
                HairTool::Edit => {
                    match self.hair.pick(&self.modeling, &view, mouse) {
                        Some(HairPick::Point(r, i)) => self.hair.begin_point_drag(&self.modeling, r, i),
                        Some(HairPick::Segment(r, after, _)) if ctrl => match self.hair.insert_point(&mut self.modeling, r, after) {
                            Ok(()) => {
                                if let Some(i) = self.hair.point {
                                    self.hair.begin_point_drag(&self.modeling, r, i);
                                }
                            }
                            Err(e) => self.modeling.status = e,
                        },
                        Some(HairPick::Segment(r, _, _)) => self.hair.select(Some(r), None),
                        None => self.hair.select(None, None),
                    }
                }
                HairTool::Comb => self.hair.begin_comb(mouse),
            }
            ui.ctx().request_repaint();
            return true;
        }
        false
    }

    /// Atalhos do Modo Cabelo (o chamador já garantiu ponteiro no viewport e nenhum campo de texto em foco).
    pub(crate) fn hair_hotkeys(&mut self, ui: &mut egui::Ui) {
        let km = self.keymap.clone();
        let consume = |ui: &mut egui::Ui, id: &str| km.consume(ui.ctx(), id);
        // o atalho com Shift antes do sem Shift (Shift+D antes de D): o egui ignora Shift extra e a primeira consome a tecla
        if consume(ui, "modelagem.hair.duplicate") {
            let r = self.hair.duplicate_selected(&mut self.modeling);
            self.hair_report(r);
        }
        for (id, tool) in [("modelagem.hair.tool_draw", HairTool::Draw), ("modelagem.hair.tool_edit", HairTool::Edit), ("modelagem.hair.tool_comb", HairTool::Comb)] {
            if consume(ui, id) {
                self.hair_set_tool(tool);
            }
        }
        if consume(ui, "modelagem.hair.delete_hard") {
            let r = self.hair.delete_selected(&mut self.modeling, true);
            self.hair_report(r);
        } else if consume(ui, "modelagem.hair.delete") {
            let r = self.hair.delete_selected(&mut self.modeling, false);
            self.hair_report(r);
        }
        if consume(ui, "modelagem.hair.flip_group") {
            self.hair_flip_active_group();
        } else if consume(ui, "modelagem.hair.symmetry") {
            self.hair_toggle_symmetry();
        }
        if consume(ui, "modelagem.hair.comb_smaller") {
            self.hair.comb_radius_px = (self.hair.comb_radius_px / 1.12).clamp(10.0, 400.0);
        }
        if consume(ui, "modelagem.hair.comb_larger") {
            self.hair.comb_radius_px = (self.hair.comb_radius_px * 1.12).clamp(10.0, 400.0);
        }
    }

    fn hair_set_tool(&mut self, tool: HairTool) {
        self.hair.tool = tool;
        self.tool_idx.insert(self.ws, tool.index());
    }

    fn hair_report(&mut self, r: Result<(), String>) {
        if let Err(e) = r {
            self.modeling.status = e.clone();
            self.hair.error = Some(e);
        }
    }

    pub(crate) fn hair_toggle_symmetry(&mut self) {
        let Some(gid) = self.hair.active_group(&self.modeling) else {
            return;
        };
        self.modeling.checkpoint("Simetria do grupo");
        let on = self.modeling.doc.hair.group(gid).map(|g| !g.symmetry).unwrap_or(false);
        if let Err(e) = self.modeling.doc.hair.set_symmetry(gid, on) {
            self.modeling.discard_checkpoint();
            self.hair_report(Err(e.to_string()));
            return;
        }
        let r = self.modeling.hair_regenerate(Some(gid));
        self.hair_report(r);
    }

    pub(crate) fn hair_flip_active_group(&mut self) {
        let Some(gid) = self.hair.active_group(&self.modeling) else {
            return;
        };
        self.modeling.checkpoint("Espelhar grupo");
        match self.modeling.doc.hair.flip_group(gid) {
            Ok(id) => {
                self.hair.group = Some(id);
                let r = self.modeling.hair_regenerate(Some(id));
                self.hair_report(r);
            }
            Err(e) => {
                self.modeling.discard_checkpoint();
                self.hair_report(Err(e.to_string()));
            }
        }
    }

    // ======================================================================= saída

    /// Exporta cada grupo de cabelo como GLB para a Biblioteca (um ativo por grupo).
    pub(crate) fn export_hair_to_library(&mut self) {
        let objs: Vec<_> = self
            .modeling
            .doc
            .hair
            .groups
            .iter()
            .filter_map(|g| g.object_id)
            .filter_map(|id| self.modeling.doc.objects.iter().find(|o| o.id == id).cloned())
            .collect();
        if objs.is_empty() {
            self.modeling.status = tr("hairtool.nothing_to_export").to_string();
            return;
        }
        let tmp = std::env::temp_dir().join(format!("anigo-hair-export-{}", std::process::id()));
        let mut done = 0;
        let mut last_err = None;
        for obj in &objs {
            let r = crate::library::export_object_glb(obj, &tmp).map_err(|e| format!("Exportar: {e}")).and_then(|glb| {
                let r = self.lib_workspace().import_authored(&obj.name, &glb).map_err(|e| format!("Biblioteca: {e}"));
                let _ = std::fs::remove_file(&glb);
                r
            });
            match r {
                Ok(_) => done += 1,
                Err(e) => last_err = Some(e),
            }
        }
        let _ = std::fs::remove_dir(&tmp);
        self.modeling.status = match last_err {
            Some(e) => e,
            None => format!("{} {done}", tr("hairtool.exported")),
        };
    }

    /// Grava as cadeias de física em JSON (`<dados do studio>/Hair/chains.json`) e devolve o caminho.
    pub(crate) fn export_hair_chains(&mut self) -> Result<std::path::PathBuf, String> {
        let chains = self.modeling.doc.hair.generate_chains().map_err(|e| e.to_string())?;
        if chains.is_empty() {
            return Err(tr("hairtool.no_chains").to_string());
        }
        let dir = crate::library::studio_data_dir().join("Hair");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join("chains.json");
        let json = serde_json::to_string_pretty(&chains).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    // ======================================================================= painel

    pub(crate) fn inspector_hair(&mut self, ui: &mut egui::Ui, tab: PropTab) {
        if !ui.input(|i| i.pointer.any_down()) {
            self.hair.end_gesture();
        }
        self.hair.sync(&self.modeling);
        self.hair.prune(&self.modeling);
        if let Some(e) = self.hair.error.clone() {
            ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x80, 0x80)).size(12.0));
            ui.add_space(4.0);
        }

        if tab == PropTab::Hair {
            // ---- ferramenta ativa (a escolha fica na barra da esquerda; aqui só a dica)
            let hint = match self.hair.tool {
                HairTool::Draw => "hairtool.hint.draw",
                HairTool::Edit => "hairtool.hint.edit",
                HairTool::Comb => "hairtool.hint.comb",
            };
            ui.label(RichText::new(tr(hint)).size(11.0).color(theme::TEXT_DIM));
            ui.add_space(4.0);

            // ---- cabeça-alvo
            section(ui, tr("hairtool.target"), true, |ui| {
                let name = self.modeling.hair_target().map(|o| o.name.clone());
                match &name {
                    Some(n) => ui.label(format!("{}  {n}", icons::USER)),
                    None => ui.label(RichText::new(tr("hairtool.no_target")).color(theme::TEXT_DIM)),
                };
                let active_is_hair = self
                    .modeling
                    .active_object()
                    .is_some_and(|a| self.modeling.doc.hair.groups.iter().any(|g| g.object_id == Some(a.id)));
                let can = self.modeling.active_object().is_some() && !active_is_hair;
                if ui.add_enabled(can, egui::Button::new(format!("{}  {}", icons::TARGET, tr("hairtool.use_active")))).clicked() {
                    self.modeling.checkpoint("Definir cabeça do cabelo");
                    self.modeling.hair_set_target_from_active();
                    let r = self.modeling.hair_regenerate(None);
                    self.hair_report(r);
                }
            });

            self.hair_groups_panel(ui);
            self.hair_strand_panel(ui);
        }
        if tab == PropTab::Drawing {
            self.hair_drawing_panel(ui);
        }
        if tab == PropTab::Physics {
            self.hair_bounce_panel(ui);
        }

        if tab == PropTab::Output {
            section(ui, tr("hairtool.output"), false, |ui| {
                if ui.button(format!("{}  {}", icons::EXPORT, tr("hairtool.export_glb"))).clicked() {
                    self.export_hair_to_library();
                }
                if ui.button(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("hairtool.regenerate"))).clicked() {
                    let r = self.modeling.hair_regenerate(None);
                    self.hair_report(r);
                }
                ui.label(RichText::new(tr("hairtool.output_note")).size(11.0).color(theme::TEXT_DIM));
            });
        }
        if tab == PropTab::Help {
            section(ui, tr("hairtool.shortcuts"), false, |ui| {
                for l in [
                    "hairtool.keys.1",
                    "hairtool.keys.2",
                    "hairtool.keys.3",
                    "hairtool.keys.4",
                    "hairtool.keys.5",
                ] {
                    ui.label(RichText::new(tr(l)).size(11.0).color(theme::TEXT_DIM));
                }
            });
        }
    }

    fn hair_groups_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("hairtool.groups"), true, |ui| {
            let active = self.hair.active_group(&self.modeling);
            let mut pick = None;
            let mut toggle_vis = None;
            for g in &self.modeling.doc.hair.groups {
                ui.horizontal_wrapped(|ui| {
                    let eye = if g.visible { icons::EYE } else { icons::EYE_SLASH };
                    if ui.small_button(eye).clicked() {
                        toggle_vis = Some(g.id);
                    }
                    let (c1, c2) = (rgb(g.material.base_color), rgb(g.material.shade_color));
                    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 3.0, c1);
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, c2), egui::StrokeKind::Inside);
                    let label = format!("{}  ({})", g.name, g.strands.len());
                    if ui.selectable_label(active == Some(g.id), label).clicked() {
                        pick = Some(g.id);
                    }
                    if g.symmetry {
                        ui.label(RichText::new("X").strong().color(Color32::from_rgb(0xe8, 0x45, 0x4b)));
                    }
                });
            }
            if self.modeling.doc.hair.groups.is_empty() {
                ui.label(RichText::new(tr("hairtool.no_groups")).size(12.0).color(theme::TEXT_DIM));
            }
            if let Some(g) = pick {
                self.hair.group = Some(g);
            }
            if let Some(g) = toggle_vis {
                self.modeling.checkpoint("Visibilidade do grupo");
                if let Some(gr) = self.modeling.doc.hair.group_mut(g) {
                    gr.visible = !gr.visible;
                }
                let r = self.modeling.hair_regenerate(Some(g));
                self.hair_report(r);
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(format!("{}  {}", icons::PLUS, tr("hairtool.group_new"))).clicked() {
                    self.modeling.checkpoint("Novo grupo");
                    let n = self.modeling.doc.hair.groups.len() + 1;
                    match self.modeling.doc.hair.add_group(format!("Grupo {n}")) {
                        Ok(id) => self.hair.group = Some(id),
                        Err(e) => {
                            self.modeling.discard_checkpoint();
                            self.hair_report(Err(e.to_string()));
                        }
                    }
                }
                let has = self.hair.group.is_some();
                if ui.add_enabled(has, egui::Button::new(format!("{}  {}", icons::COPY, tr("hairtool.group_duplicate")))).clicked() {
                    if let Some(g) = self.hair.group {
                        self.modeling.checkpoint("Duplicar grupo");
                        match self.modeling.doc.hair.duplicate_group(g) {
                            Ok(id) => {
                                self.hair.group = Some(id);
                                let r = self.modeling.hair_regenerate(Some(id));
                                self.hair_report(r);
                            }
                            Err(e) => {
                                self.modeling.discard_checkpoint();
                                self.hair_report(Err(e.to_string()));
                            }
                        }
                    }
                }
                if ui
                    .add_enabled(has, egui::Button::new(format!("{}  {}", icons::FLIP_HORIZONTAL, tr("hairtool.group_flip"))))
                    .on_hover_text(tr("hairtool.group_flip_tip"))
                    .clicked()
                {
                    self.hair_flip_active_group();
                }
                if ui.add_enabled(has, egui::Button::new(format!("{}  {}", icons::TRASH, tr("action.delete")))).clicked() {
                    if let Some(g) = self.hair.group {
                        let r = self.modeling.hair_remove_group(g);
                        self.hair_report(r);
                        self.hair.prune(&self.modeling);
                    }
                }
            });
            // grupo ativo: nome, simetria, mover mecha
            let Some(gid) = self.hair.group.filter(|g| self.modeling.doc.hair.group(*g).is_some()) else {
                return;
            };
            let mut name = self.modeling.doc.hair.group(gid).map(|g| g.name.clone()).unwrap_or_default();
            let r = ui.text_edit_singleline(&mut name);
            if r.changed() && !name.trim().is_empty() {
                self.hair.gesture_checkpoint(&mut self.modeling, "group_name", "Renomear grupo");
                let _ = self.modeling.doc.hair.rename_group(gid, name);
                let rr = self.modeling.hair_regenerate(Some(gid));
                self.hair_report(rr);
            }
            let mut sym = self.modeling.doc.hair.group(gid).map(|g| g.symmetry).unwrap_or(false);
            if ui.checkbox(&mut sym, tr("hairtool.symmetry")).changed() {
                self.hair_toggle_symmetry();
            }
            if let Some(sel) = self.hair.selected {
                let others: Vec<(u32, String)> = self
                    .modeling
                    .doc
                    .hair
                    .groups
                    .iter()
                    .filter(|g| g.id != sel.group)
                    .map(|g| (g.id, g.name.clone()))
                    .collect();
                if !others.is_empty() {
                    let mut dest = None;
                    crate::app::props_tabs::combo(ui, "hair_move_to")
                        .selected_text(tr("hairtool.move_to"))
                        .show_ui(ui, |ui| {
                            for (id, n) in &others {
                                if ui.selectable_label(false, n).clicked() {
                                    dest = Some(*id);
                                }
                            }
                        });
                    if let Some(to) = dest {
                        self.modeling.checkpoint("Mover mecha de grupo");
                        match self.modeling.doc.hair.move_strand(sel, to) {
                            Ok(()) => {
                                let r = self.modeling.hair_regenerate(None);
                                self.hair_report(r);
                                self.hair.select(Some(StrandRef { group: to, strand: sel.strand }), None);
                            }
                            Err(e) => {
                                self.modeling.discard_checkpoint();
                                self.hair_report(Err(e.to_string()));
                            }
                        }
                    }
                }
            }
            // material do grupo
            let Some(mut mat) = self.modeling.doc.hair.group(gid).map(|g| g.material.clone()) else {
                return;
            };
            let before = mat.clone();
            let mut layout = self.modeling.doc.hair.group(gid).map(|g| g.uv_layout).unwrap_or_default();
            let layout_before = layout;
            ui.separator();
            ui.label(RichText::new(tr("hairtool.material")).strong());
            ui.horizontal_wrapped(|ui| {
                for (c, tip) in [
                    (&mut mat.base_color, "hairtool.color_base"),
                    (&mut mat.shade_color, "hairtool.color_shade"),
                    (&mut mat.highlight_color, "hairtool.color_highlight"),
                ] {
                    // o egui edita sRGB em u8; o material guarda 0..1 (mesma interpretação do chip do grupo)
                    let mut b = [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
                    if ui.color_edit_button_srgb(&mut b).on_hover_text(tr(tip)).changed() {
                        *c = [b[0] as f32 / 255.0, b[1] as f32 / 255.0, b[2] as f32 / 255.0];
                    }
                }
            });
            ui.add(crate::app::props_tabs::lab(tr("hairtool.root_ao"), egui::Slider::new(&mut mat.root_ao, 0.0..=1.0)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.normal_bias"), egui::Slider::new(&mut mat.normal_bias, 0.0..=1.0)));
            crate::app::props_tabs::combo(ui, "hair_uv_layout")
                .selected_text(match layout {
                    UvLayout::LengthV => tr("hairtool.uv_length_v"),
                    UvLayout::LengthU => tr("hairtool.uv_length_u"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut layout, UvLayout::LengthV, tr("hairtool.uv_length_v"));
                    ui.selectable_value(&mut layout, UvLayout::LengthU, tr("hairtool.uv_length_u"));
                });
            if mat != before || layout != layout_before {
                self.hair.gesture_checkpoint(&mut self.modeling, "group_material", "Material do grupo");
                if let Some(g) = self.modeling.doc.hair.group_mut(gid) {
                    g.material = mat;
                    g.uv_layout = layout;
                }
                let r = self.modeling.hair_regenerate(Some(gid));
                self.hair_report(r);
            }
        });
    }

    fn hair_strand_panel(&mut self, ui: &mut egui::Ui) {
        let selected = self.hair.selected.filter(|r| self.modeling.doc.hair.strand(*r).is_some());
        let title = if selected.is_some() { tr("hairtool.strand_selected") } else { tr("hairtool.strand_new") };
        section(ui, title, true, |ui| {
            let mut p: StrandParams = match selected.and_then(|r| self.modeling.doc.hair.strand(r)) {
                Some(s) => s.params,
                None => self.hair.template,
            };
            let before = p;
            crate::app::props_tabs::combo(ui, "hair_section_kind")
                .selected_text(tr(p.section.kind.key()))
                .show_ui(ui, |ui| {
                    for k in SectionKind::ALL {
                        ui.selectable_value(&mut p.section.kind, k, tr(k.key()));
                    }
                });
            ui.add(crate::app::props_tabs::lab(tr("hairtool.width"), egui::Slider::new(&mut p.section.width, 0.005..=0.4).logarithmic(true)));
            if p.section.kind.is_solid() {
                ui.add(crate::app::props_tabs::lab(tr("hairtool.thickness"), egui::Slider::new(&mut p.section.thickness, 0.003..=0.3).logarithmic(true)));
            }
            let mut twist_deg = p.section.twist.to_degrees();
            if ui.add(crate::app::props_tabs::lab(tr("hairtool.twist"), egui::Slider::new(&mut twist_deg, -720.0..=720.0).suffix("°"))).changed() {
                p.section.twist = twist_deg.to_radians();
            }
            if p.section.kind == SectionKind::Arc {
                let mut curl_deg = p.section.curl.to_degrees();
                if ui.add(crate::app::props_tabs::lab(tr("hairtool.curl"), egui::Slider::new(&mut curl_deg, -300.0..=300.0).suffix("°"))).changed() {
                    p.section.curl = curl_deg.to_radians().clamp(-5.2, 5.2);
                }
            }
            ui.add(crate::app::props_tabs::lab(tr("hairtool.tip_taper"), egui::Slider::new(&mut p.tip_taper, 0.0..=1.0)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.taper_power"), egui::Slider::new(&mut p.taper_power, 0.3..=4.0)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.flare"), egui::Slider::new(&mut p.flare, -0.5..=1.0)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.resolution"), egui::Slider::new(&mut p.resolution, 1..=32)));
            let mut clump = p.clump.is_some();
            if ui.checkbox(&mut clump, tr("hairtool.clump")).changed() {
                p.clump = clump.then(Default::default);
            }
            if let Some(c) = p.clump.as_mut() {
                ui.add(crate::app::props_tabs::lab(tr("hairtool.clump_count"), egui::Slider::new(&mut c.count, 1..=24)));
                ui.add(crate::app::props_tabs::lab(tr("hairtool.clump_spread"), egui::Slider::new(&mut c.spread, 0.0..=0.15)));
                ui.add(crate::app::props_tabs::lab(tr("hairtool.clump_noise"), egui::Slider::new(&mut c.noise, 0.0..=1.0)));
                ui.add(crate::app::props_tabs::lab(tr("hairtool.clump_width"), egui::Slider::new(&mut c.strand_width, 0.004..=0.1).logarithmic(true)));
            }
            if p != before {
                self.hair.gesture_checkpoint(&mut self.modeling, "strand_params", "Parâmetros da mecha");
                match selected {
                    Some(r) => match self.modeling.doc.hair.set_params(r, p) {
                        Ok(()) => {
                            let rr = self.modeling.hair_regenerate(Some(r.group));
                            self.hair_report(rr);
                        }
                        Err(e) => self.hair_report(Err(e.to_string())),
                    },
                    None => self.hair.template = p,
                }
            }
            ui.horizontal_wrapped(|ui| {
                let gid = self.hair.group;
                if ui.add_enabled(gid.is_some(), egui::Button::new(tr("hairtool.apply_group"))).on_hover_text(tr("hairtool.apply_group_tip")).clicked() {
                    if let Some(g) = gid {
                        self.modeling.checkpoint("Parâmetros do grupo");
                        match self.modeling.doc.hair.set_group_params(g, p) {
                            Ok(()) => {
                                let r = self.modeling.hair_regenerate(Some(g));
                                self.hair_report(r);
                            }
                            Err(e) => {
                                self.modeling.discard_checkpoint();
                                self.hair_report(Err(e.to_string()));
                            }
                        }
                    }
                }
                if let Some(r) = selected {
                    if ui.button(format!("{}  {}", icons::COPY, tr("hairtool.strand_duplicate"))).clicked() {
                        let rr = self.hair.duplicate_selected(&mut self.modeling);
                        self.hair_report(rr);
                    }
                    if ui.button(format!("{}  {}", icons::TRASH, tr("hairtool.strand_delete"))).clicked() {
                        let rr = self.hair.delete_selected(&mut self.modeling, true);
                        self.hair_report(rr);
                    }
                    if ui.button(tr("hairtool.simplify")).on_hover_text(tr("hairtool.simplify_tip")).clicked() {
                        self.modeling.checkpoint("Simplificar mecha");
                        if let Some(s) = self.modeling.doc.hair.strand_mut(r) {
                            s.simplify(0.006);
                        }
                        self.modeling.doc.hair.sync_mirror(r);
                        let rr = self.modeling.hair_regenerate(Some(r.group));
                        self.hair_report(rr);
                    }
                }
            });
            if let Some(s) = selected.and_then(|r| self.modeling.doc.hair.strand(r)) {
                ui.label(
                    RichText::new(format!("{} {} · {:.2}", s.guide.points.len(), tr("hairtool.points"), s.length()))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
            }
        });
    }

    fn hair_drawing_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("hairtool.drawing"), false, |ui| {
            let s = &mut self.hair.stroke;
            ui.add(crate::app::props_tabs::lab(tr("hairtool.lift_root"), egui::Slider::new(&mut s.lift_root, 0.0..=0.1)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.lift_tip"), egui::Slider::new(&mut s.lift_tip, 0.0..=0.4)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.smooth"), egui::Slider::new(&mut s.smooth_iters, 0..=10)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.rdp"), egui::Slider::new(&mut s.rdp_epsilon, 0.0005..=0.03).logarithmic(true)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.step"), egui::Slider::new(&mut s.resample_step, 0.005..=0.08).logarithmic(true)));
            ui.separator();
            ui.add(crate::app::props_tabs::lab(tr("hairtool.comb_radius"), egui::Slider::new(&mut self.hair.comb_radius_px, 10.0..=400.0)));
            ui.add(crate::app::props_tabs::lab(tr("hairtool.comb_strength"), egui::Slider::new(&mut self.hair.comb_strength, 0.02..=1.0)));
            ui.checkbox(&mut self.hair.comb_all_groups, tr("hairtool.comb_all"));
        });
    }

    fn hair_bounce_panel(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("hairtool.physics"), false, |ui| {
            ui.label(RichText::new(tr("hairtool.physics_note")).size(11.0).color(theme::TEXT_DIM));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut self.hair.bounce_groups).range(1..=30).prefix(format!("{} ", tr("hairtool.bounce_groups"))));
                ui.add(egui::DragValue::new(&mut self.hair.bounce_bones).range(1..=16).prefix(format!("{} ", tr("hairtool.bones"))));
            });
            if ui.button(format!("{}  {}", icons::BONE, tr("hairtool.auto_bounce"))).clicked() {
                self.modeling.checkpoint("Auto-gerar grupos de bounce");
                match self.modeling.doc.hair.auto_bounce(self.hair.bounce_groups, self.hair.bounce_bones) {
                    Ok(n) => self.modeling.status = format!("{n} {}", tr("hairtool.bounce_created")),
                    Err(e) => {
                        self.modeling.discard_checkpoint();
                        self.hair_report(Err(e.to_string()));
                    }
                }
            }
            let mut edit: Option<(usize, anigo_modeling::hair::chain::BounceGroup)> = None;
            for (i, b) in self.modeling.doc.hair.bounce.iter().enumerate() {
                let mut nb = b.clone();
                crate::app::props_tabs::header(ui, format!("{} ({})", nb.name, nb.strands.len()))
                    .id_salt(("bounce", nb.id))
                    .show(ui, |ui| {
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.bones"), egui::Slider::new(&mut nb.bones, 1..=16)));
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.fixed_point"), egui::Slider::new(&mut nb.fixed_point, 0.01..=0.99)));
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.stiffness"), egui::Slider::new(&mut nb.stiffness, 0.0..=1.0)));
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.drag"), egui::Slider::new(&mut nb.drag, 0.0..=1.0)));
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.gravity"), egui::Slider::new(&mut nb.gravity, 0.0..=1.0)));
                        ui.add(crate::app::props_tabs::lab(tr("hairtool.hit_radius"), egui::Slider::new(&mut nb.hit_radius, 0.0..=0.1)));
                    });
                if nb != *b {
                    edit = Some((i, nb));
                }
            }
            if let Some((i, nb)) = edit {
                self.hair.gesture_checkpoint(&mut self.modeling, "bounce_edit", "Parâmetros de bounce");
                self.modeling.doc.hair.bounce[i] = nb;
            }
            let n = self.modeling.doc.hair.generate_chains().map(|c| c.len()).unwrap_or(0);
            ui.label(RichText::new(format!("{n} {}", tr("hairtool.chains"))).size(11.0).color(theme::TEXT_DIM));
            if ui.add_enabled(n > 0, egui::Button::new(format!("{}  {}", icons::EXPORT, tr("hairtool.export_chains")))).clicked() {
                self.modeling.status = match self.export_hair_chains() {
                    Ok(p) => format!("{} {}", tr("hairtool.chains_saved"), p.display()),
                    Err(e) => e,
                };
            }
        });
    }

    // ======================================================================= overlay

    /// Guias, pontos de controle, traço em andamento e anel do pente.
    pub(crate) fn hair_overlay(&mut self, ui: &mut egui::Ui, rect: Rect, size: Vec2) {
        let painter = ui.painter_at(rect);
        let Some(model) = self.modeling.hair_target().map(|o| o.model_matrix()) else {
            painter.text(
                rect.min + egui::vec2(10.0, 62.0),
                egui::Align2::LEFT_TOP,
                tr("hairtool.no_target"),
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0xff, 0xb0, 0x70),
            );
            return;
        };
        let cam = self.camera;
        let to_screen = |p: Vec3| -> Option<Pos2> { cam.project(model.transform_point3(p), size).map(|(px, _)| rect.min + egui::vec2(px.x, px.y)) };

        // plano de simetria X (no espaço local da cabeça)
        let active = self.hair.group.and_then(|g| self.modeling.doc.hair.group(g));
        if active.is_some_and(|g| g.symmetry) {
            if let Some(bvh) = self.hair.bvh() {
                if let Some(bb) = bvh.bounds() {
                    let (c, e) = (bb.center(), bb.extent().max_element() * 0.65);
                    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
                    let pts: Vec<Pos2> = corners.iter().filter_map(|(a, b)| to_screen(Vec3::new(0.0, c.y + b * e, c.z + a * e))).collect();
                    if pts.len() == 4 {
                        painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(232, 69, 75, 22), Stroke::NONE));
                        let mut closed = pts;
                        closed.push(closed[0]);
                        painter.add(egui::Shape::line(closed, Stroke::new(1.2, Color32::from_rgba_unmultiplied(232, 69, 75, 150))));
                    }
                }
            }
        }

        let sel = self.hair.selected;
        for g in self.modeling.doc.hair.groups.iter().filter(|g| g.visible) {
            let base = overlay_color(g.material.base_color);
            for s in &g.strands {
                let r = StrandRef { group: g.id, strand: s.id };
                let is_sel = sel == Some(r);
                let n = s.guide.points.len();
                let samples = s.guide.sample_uniform((n * 8).max(16)).unwrap_or_default();
                let line: Vec<Pos2> = samples.iter().filter_map(|c| to_screen(c.pos)).collect();
                if line.len() > 1 {
                    painter.add(egui::Shape::line(line.clone(), Stroke::new(if is_sel { 4.0 } else { 3.0 }, Color32::from_black_alpha(150))));
                    painter.add(egui::Shape::line(line, Stroke::new(if is_sel { 2.4 } else { 1.5 }, if is_sel { theme::SELECT } else { base })));
                }
                if let Some(root) = to_screen(s.root()) {
                    painter.circle_filled(root, 3.2, Color32::from_rgb(0x6c, 0xc0, 0x4a));
                }
                let show_points = is_sel || self.hair.tool == HairTool::Edit;
                if show_points {
                    for (i, p) in s.guide.points.iter().enumerate() {
                        let Some(px) = to_screen(p.pos) else {
                            continue;
                        };
                        let hot = is_sel && self.hair.point == Some(i);
                        let half = if is_sel { 4.5 } else { 3.0 };
                        let r = Rect::from_center_size(px, egui::vec2(half * 2.0, half * 2.0));
                        painter.rect_filled(r, 1.0, if hot { theme::SELECT } else if is_sel { Color32::WHITE } else { Color32::from_rgb(0xbb, 0xbb, 0xc8) });
                        painter.rect_stroke(r, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
                    }
                }
            }
        }

        // traço de desenho em andamento
        if let Some(samples) = self.hair.draw_preview() {
            let pts: Vec<Pos2> = samples.iter().filter_map(|s| to_screen(s.point + s.normal * 0.01)).collect();
            if pts.len() > 1 {
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(4.0, Color32::from_black_alpha(160))));
                painter.add(egui::Shape::line(pts, Stroke::new(2.2, Color32::from_rgb(0xff, 0xe0, 0x60))));
            }
        }

        // cursor da ferramenta
        let mouse = self.last_mouse;
        let in_view = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if in_view {
            match self.hair.tool {
                HairTool::Comb => {
                    let r = self.hair.comb_radius_px;
                    painter.circle_stroke(mouse, r, Stroke::new(3.2, Color32::from_black_alpha(120)));
                    painter.circle_stroke(mouse, r, Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 230)));
                    painter.circle_stroke(mouse, r * 0.5, Stroke::new(1.0, Color32::from_white_alpha(80)));
                }
                HairTool::Draw => {
                    if let Some(h) = self.hair.hover.and_then(&to_screen) {
                        painter.circle_stroke(h, 7.0, Stroke::new(3.0, Color32::from_black_alpha(120)));
                        painter.circle_stroke(h, 7.0, Stroke::new(1.5, Color32::WHITE));
                    }
                }
                HairTool::Edit => {}
            }
        }

        // legenda
        let g = active.map(|g| g.name.as_str()).unwrap_or("—");
        let sym = if active.is_some_and(|g| g.symmetry) { format!("  |  {} X", tr("hairtool.symmetry")) } else { String::new() };
        painter.text(
            rect.min + egui::vec2(10.0, 62.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{}  |  {}: {g}  |  {} {}{sym}",
                tr(self.hair.tool.tool_id()),
                tr("hairtool.group"),
                self.modeling.doc.hair.strand_count(),
                tr("hairtool.strands")
            ),
            egui::FontId::proportional(12.0),
            Color32::from_rgba_unmultiplied(255, 200, 130, 220),
        );
        if self.hair.is_busy() {
            ui.ctx().request_repaint();
        }
    }
}
