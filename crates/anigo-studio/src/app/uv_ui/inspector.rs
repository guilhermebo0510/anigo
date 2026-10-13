//! Seção "UV" do Inspector: costuras, unwrap, empacotamento, projeções e relatório de qualidade.

use super::{UvAction, UvParams};
use crate::app::StudioApp;
use crate::icons;
use crate::theme;
use anigo_modeling::uv::metrics::UvReport;
use anigo_modeling::uv::unwrap::UnwrapMethod;
use egui::{Color32, RichText};

const TEX_SIZES: [u32; 5] = [256, 512, 1024, 2048, 4096];
const WARN: Color32 = Color32::from_rgb(0xff, 0x6a, 0x5a);
const OK: Color32 = Color32::from_rgb(0x6c, 0xd0, 0x8a);

fn sub(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).color(theme::TEXT).size(13.0))
        .default_open(open)
        .show(ui, add);
}

fn drag(ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).speed(speed).range(range).suffix(suffix).fixed_decimals(1));
    });
}

impl StudioApp {
    pub(super) fn inspector_uv_impl(&mut self, ui: &mut egui::Ui) {
        self.uv_refresh_cache();
        egui::CollapsingHeader::new(RichText::new(crate::i18n::tr("panel.uv")).strong())
            .default_open(true)
            .show(ui, |ui| {
                let mut act: Option<UvAction> = None;
                let p: &mut UvParams = &mut self.uv.params;

                ui.horizontal_wrapped(|ui| {
                    ui.toggle_value(&mut self.show_uv_editor, format!("{} Editor de UV", icons::GRID_FOUR))
                        .on_hover_text("Mostra/oculta o painel 2D de UV (botão UV do viewport)");
                    ui.label("Textura");
                    egui::ComboBox::from_id_salt("uv_tex_size")
                        .selected_text(format!("{} px", p.tex_size))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for s in TEX_SIZES {
                                ui.selectable_value(&mut p.tex_size, s, format!("{s} px"));
                            }
                        });
                });

                sub(ui, "Costuras", true, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(format!("{} Marcar", icons::SCISSORS)).on_hover_text("Marca as arestas selecionadas como costura").clicked() {
                            act = Some(UvAction::MarkSeam);
                        }
                        if ui.button("Limpar").on_hover_text("Remove a costura das arestas selecionadas").clicked() {
                            act = Some(UvAction::ClearSeam);
                        }
                        if ui.button("Limpar todas").clicked() {
                            act = Some(UvAction::ClearAllSeams);
                        }
                    });
                    if ui.button(format!("{} Caminho mais curto", icons::PATH)).on_hover_text("Costura entre 2 vértices selecionados").clicked() {
                        act = Some(UvAction::SeamPath);
                    }
                    drag(ui, "Ângulo", &mut p.seam_angle_deg, 0.5, 1.0..=179.0, "°");
                    if ui.button("Costuras por ângulo").on_hover_text("Costura em toda aresta com ângulo diedral maior que o limite").clicked() {
                        act = Some(UvAction::SeamsByAngle);
                    }
                    ui.separator();
                    drag(ui, "Ângulo (personagem)", &mut p.char_angle_deg, 0.5, 1.0..=179.0, "°");
                    if ui
                        .button(format!("{} Automático para personagem", icons::MAGIC_WAND))
                        .on_hover_text("Ângulo diedral + cortes para abrir ilhas fechadas + pontas")
                        .clicked()
                    {
                        act = Some(UvAction::AutoSeamsCharacter);
                    }
                    if ui.button("Das arestas duras").clicked() {
                        act = Some(UvAction::SeamsFromSharp);
                    }
                });

                sub(ui, "Unwrap", true, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut p.method, UnwrapMethod::Lscm, "LSCM");
                        ui.selectable_value(&mut p.method, UnwrapMethod::LscmArap, "LSCM + ARAP");
                    });
                    drag(ui, "Margem", &mut p.margin_px, 0.1, 0.0..=64.0, " px");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut p.correct_aspect, "Corrigir proporção");
                        ui.add_enabled(
                            p.correct_aspect,
                            egui::DragValue::new(&mut p.aspect).speed(0.01).range(0.1..=10.0).fixed_decimals(2).prefix("L/A "),
                        );
                    });
                    ui.checkbox(&mut p.fill_holes, "Preencher buracos");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(format!("{} Unwrap", icons::CUBE_FOCUS)).on_hover_text("Desdobra as ilhas definidas pelas costuras (faces selecionadas, ou tudo)").clicked() {
                            act = Some(UvAction::Unwrap);
                        }
                        ui.add(egui::DragValue::new(&mut p.stretch_iters).range(1..=500).suffix(" it."));
                        if ui.button("Minimizar estiramento").clicked() {
                            act = Some(UvAction::MinimizeStretch);
                        }
                    });
                });

                sub(ui, "Empacotar", true, |ui| {
                    drag(ui, "Margem", &mut p.pack_margin_px, 0.1, 0.0..=64.0, " px");
                    ui.checkbox(&mut p.pack_rotate, "Permitir rotação (90°)");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(format!("{} Empacotar", icons::PACKAGE)).clicked() {
                            act = Some(UvAction::Pack);
                        }
                        if ui.button("Normalizar densidade").on_hover_text("Mesma densidade de texel em todas as ilhas e reempacota").clicked() {
                            act = Some(UvAction::NormalizeDensity);
                        }
                    });
                });

                sub(ui, "Projeções", false, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Planar em");
                        for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                            ui.selectable_value(&mut p.planar_axis, i, *n);
                        }
                        if ui.button("Planar").clicked() {
                            act = Some(UvAction::ProjectPlanar);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut p.box_cross, "Cruz");
                        if ui.button("Caixa").clicked() {
                            act = Some(UvAction::ProjectBox);
                        }
                        if ui.button("Cilindro").clicked() {
                            act = Some(UvAction::ProjectCylinder);
                        }
                        if ui.button("Esfera").clicked() {
                            act = Some(UvAction::ProjectSphere);
                        }
                    });
                    drag(ui, "Limite de ângulo", &mut p.smart_angle_deg, 0.5, 1.0..=88.0, "°");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Smart UV Project").clicked() {
                            act = Some(UvAction::SmartProject);
                        }
                        if ui.button("Da vista").on_hover_text("Projeta pela câmera do viewport").clicked() {
                            act = Some(UvAction::ProjectFromView);
                        }
                    });
                });

                sub(ui, "Relatório", true, |ui| {
                    match self.uv.cache.as_ref().map(|c| &c.report) {
                        Some(Ok(r)) => report_grid(ui, r),
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(WARN));
                        }
                        None => {
                            ui.label(RichText::new("—").color(theme::TEXT_DIM));
                        }
                    };
                });

                if let Some(err) = &self.uv.last_error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(err).color(WARN).size(12.0));
                }
                if let Some(a) = act {
                    self.run_uv_action(a);
                }
            });
    }
}

fn flag(ui: &mut egui::Ui, label: &str, n: usize) {
    ui.label(label);
    ui.label(RichText::new(n.to_string()).color(if n == 0 { OK } else { WARN }).strong());
    ui.end_row();
}

/// Tabela do `UvReport`.
pub(super) fn report_grid(ui: &mut egui::Ui, r: &UvReport) {
    egui::Grid::new("uv_report").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
        ui.label("Faces / ilhas");
        ui.label(format!("{} / {}", r.face_count, r.island_count));
        ui.end_row();
        ui.label("Estiramento médio");
        ui.label(RichText::new(format!("{:.3}", r.mean_stretch)).strong());
        ui.end_row();
        ui.label("Estiramento máximo");
        ui.label(if r.max_stretch.is_finite() { format!("{:.2}", r.max_stretch) } else { "colapsado".to_string() });
        ui.end_row();
        flag(ui, "Faces invertidas", r.flipped_faces);
        flag(ui, "Ilhas sobrepostas", r.overlapping_island_pairs);
        flag(ui, "Autointerseção", r.self_overlapping_faces);
        flag(ui, "Faces colapsadas", r.degenerate_faces);
        flag(ui, "Fora de [0,1]", r.outside_unit_square);
        ui.label("Ocupação");
        ui.label(format!("{:.0}%", r.uv_area * 100.0));
        ui.end_row();
        if let Some(d) = r.texel_density {
            ui.label("Densidade média");
            ui.label(format!("{:.0} px/u (±{:.0})", d.mean, d.std_dev));
            ui.end_row();
            ui.label("Densidade mín–máx");
            ui.label(format!("{:.0} – {:.0}", d.min, d.max));
            ui.end_row();
        }
    });
}
