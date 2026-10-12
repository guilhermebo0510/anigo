//! Painel de Normais (cel shading anime): overlay de normais, sharp/auto-smooth, transferência de
//! normais (ponto, direção, objeto proxy), média, rotação, suavização, assar/limpar, relatório de
//! suavidade e pintura dos canais NPR (AO / ShadowShift / OutlineWidth / SpecRim) no viewport.
//!
//! A lógica de edição fica em funções puras (`apply_normal_op`, `normal_scope`, `proportional_weights`,
//! `proxy_in_space_of`, `selected_edges`…) que só chamam o motor `anigo-modeling` e são testadas sem UI.
//! O painel e os hooks do viewport apenas montam parâmetros e passam por `ModelingSession::edit_mesh`
//! (checkpoint/undo exatos).

use super::StudioApp;
use crate::i18n::{self, tr, Lang};
use crate::icons;
use crate::modeling::{Mode, ModelObject, ModelingSession};
use crate::theme;
use crate::viewport::picking;
use crate::workbench::NormalOverlay;
use anigo_modeling::attributes::{self, BrushFalloff, PaintBlend, VertexChannel};
use anigo_modeling::normals;
use anigo_modeling::normals_tools::{
    self as nt, AverageMode, NormalBlend, NormalEditOptions, NormalMapping, NormalReport, NormalScope,
};
use anigo_modeling::ops::misc;
use anigo_modeling::{edge_key, EdgeKey, EditMesh, ModelError, ModelResult, SelectMode, Selection, Topology};
use egui::{Color32, RichText};
use glam::{Mat4, Vec2, Vec3};
use std::collections::{BTreeSet, HashMap};

/// Texto bilíngue sem passar pela tabela global (rótulos locais do painel).
fn l(pt: &'static str, en: &'static str) -> &'static str {
    match i18n::lang() {
        Lang::PtBr => pt,
        Lang::En => en,
    }
}

// =============================================================================
// Estado da UI
// =============================================================================

/// Ferramenta de edição de normais selecionada no painel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NormalTool {
    #[default]
    Point,
    Direction,
    Proxy,
    Average,
    Rotate,
    Smooth,
}

impl NormalTool {
    pub const ALL: [NormalTool; 6] = [
        NormalTool::Point,
        NormalTool::Direction,
        NormalTool::Proxy,
        NormalTool::Average,
        NormalTool::Rotate,
        NormalTool::Smooth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NormalTool::Point => l("Ponto (esfera de sombreamento)", "Point (shading sphere)"),
            NormalTool::Direction => l("Direcional", "Directional"),
            NormalTool::Proxy => l("Objeto proxy", "Proxy object"),
            NormalTool::Average => l("Média", "Average"),
            NormalTool::Rotate => l("Rotacionar", "Rotate"),
            NormalTool::Smooth => l("Suavizar região", "Smooth region"),
        }
    }
}

/// Relatório de suavidade em cache (válido enquanto `revision` for a da sessão).
#[derive(Debug, Clone)]
pub struct ReportCache {
    pub revision: u64,
    pub report: NormalReport,
    /// Faces da região medida (`None` = malha toda).
    pub region_faces: Option<usize>,
}

/// Estado persistente do painel de normais e da pintura de atributos.
#[derive(Debug, Clone)]
pub struct NormalsUi {
    // overlay
    pub show_normals: bool,
    pub normal_length: f32,
    pub selected_only: bool,
    // sharp
    pub sharp_angle_deg: f32,
    smooth_cp: bool,
    // edição de normais
    pub tool: NormalTool,
    /// Pivô em coordenadas de mundo; `None` = centro da seleção (ou do objeto).
    pub pivot: Option<Vec3>,
    pub invert: bool,
    pub mix: f32,
    pub use_proportional: bool,
    /// Direção (mundo) da transferência direcional.
    pub direction: Vec3,
    /// Id do objeto proxy.
    pub proxy: Option<u32>,
    pub mapping: NormalMapping,
    pub limit_distance: bool,
    pub max_distance: f32,
    pub average: AverageMode,
    /// 0 = X, 1 = Y, 2 = Z (mundo).
    pub rotate_axis: usize,
    pub rotate_deg: f32,
    pub smooth_iterations: u32,
    pub smooth_factor: f32,
    pub report: Option<ReportCache>,
    // pintura de atributos
    pub channel: VertexChannel,
    pub paint_value: f32,
    /// Raio do pincel em unidades de mundo.
    pub paint_radius: f32,
    pub paint_strength: f32,
    pub falloff: BrushFalloff,
    pub blend: PaintBlend,
    pub view_channel: bool,
    /// Traço em andamento (último centro local pintado).
    stroke: Option<Option<Vec3>>,
    /// Ponto (mundo) sob o cursor, para o círculo do pincel.
    pub brush_hit: Option<Vec3>,
    /// Último quadro em que o corpo do painel foi desenhado (controla o marcador de pivô).
    panel_frame: u64,
    /// Abre/fecha (uma vez) as seções do painel; usado por cenários de screenshot.
    pub force_open_normals: Option<bool>,
    pub force_open_paint: Option<bool>,
    pub force_open_sharp: Option<bool>,
}

impl Default for NormalsUi {
    fn default() -> Self {
        Self {
            show_normals: false,
            normal_length: 0.2,
            selected_only: false,
            sharp_angle_deg: 30.0,
            smooth_cp: false,
            tool: NormalTool::Point,
            pivot: None,
            invert: false,
            mix: 1.0,
            use_proportional: false,
            direction: Vec3::Z,
            proxy: None,
            mapping: NormalMapping::ClosestSurface,
            limit_distance: false,
            max_distance: 0.5,
            average: AverageMode::CornerAngle,
            rotate_axis: 1,
            rotate_deg: 15.0,
            smooth_iterations: 3,
            smooth_factor: 0.5,
            report: None,
            channel: VertexChannel::AmbientOcclusion,
            paint_value: 0.0,
            paint_radius: 0.35,
            paint_strength: 1.0,
            falloff: BrushFalloff::Smooth,
            blend: PaintBlend::Mix,
            view_channel: false,
            stroke: None,
            brush_hit: None,
            panel_frame: 0,
            force_open_normals: None,
            force_open_paint: None,
            force_open_sharp: None,
        }
    }
}

impl NormalsUi {
    /// Overlay de normais pedido ao Workbench (`None` = desligado).
    pub fn overlay(&self) -> Option<NormalOverlay> {
        self.show_normals.then_some(NormalOverlay {
            length: self.normal_length,
            selected_only: self.selected_only,
        })
    }

    /// Canal exibido como mapa de calor no sólido.
    pub fn channel_view(&self) -> Option<VertexChannel> {
        self.view_channel.then_some(self.channel)
    }

    /// Hash do que muda o desenho do Workbench vindo deste estado (chave do cache de GPU).
    pub fn view_hash(&self) -> [u64; 4] {
        [
            self.show_normals as u64,
            self.normal_length.to_bits() as u64,
            self.selected_only as u64,
            self.channel_view().map_or(0, |c| 1 + c.index() as u64),
        ]
    }
}

pub fn channel_label(c: VertexChannel) -> &'static str {
    match c {
        VertexChannel::AmbientOcclusion => l("AO (R)", "AO (R)"),
        VertexChannel::ShadowShift => l("Shadow Shift (G)", "Shadow Shift (G)"),
        VertexChannel::OutlineWidth => l("Largura do contorno (B)", "Outline width (B)"),
        VertexChannel::SpecRimMask => l("Spec/Rim (A)", "Spec/Rim (A)"),
    }
}

const CHANNELS: [VertexChannel; 4] = [
    VertexChannel::AmbientOcclusion,
    VertexChannel::ShadowShift,
    VertexChannel::OutlineWidth,
    VertexChannel::SpecRimMask,
];

fn falloff_label(f: BrushFalloff) -> &'static str {
    match f {
        BrushFalloff::Smooth => l("Suave", "Smooth"),
        BrushFalloff::Linear => l("Linear", "Linear"),
        BrushFalloff::Sphere => l("Esfera", "Sphere"),
        BrushFalloff::Root => l("Raiz", "Root"),
        BrushFalloff::Sharp => l("Agudo", "Sharp"),
        BrushFalloff::Constant => l("Constante", "Constant"),
    }
}

fn blend_label(b: PaintBlend) -> &'static str {
    match b {
        PaintBlend::Mix => l("Misturar", "Mix"),
        PaintBlend::Add => l("Somar", "Add"),
        PaintBlend::Subtract => l("Subtrair", "Subtract"),
        PaintBlend::Multiply => l("Multiplicar", "Multiply"),
    }
}

// =============================================================================
// Lógica pura (testável)
// =============================================================================

/// Escopo das operações de normais: faces selecionadas (modo Face), corners dos vértices selecionados
/// (demais modos) ou a malha toda quando não há seleção.
pub fn normal_scope(sel: &Selection, mode: SelectMode) -> NormalScope {
    if mode == SelectMode::Face && !sel.faces.is_empty() {
        NormalScope::Faces(sel.faces.iter().copied().collect())
    } else if !sel.verts.is_empty() {
        NormalScope::Verts(sel.verts.iter().copied().collect())
    } else {
        NormalScope::All
    }
}

/// Peso 0..1 por vértice para "peso por proporcional": 1 nos selecionados e queda suave
/// (`BrushFalloff::Smooth`) até `radius` da seleção. Usa grade espacial (sem O(n·k)).
pub fn proportional_weights(mesh: &EditMesh, selected: &BTreeSet<u32>, radius: f32) -> Vec<f32> {
    let n = mesh.vert_count();
    let mut w = vec![0.0f32; n];
    if selected.is_empty() || !radius.is_finite() || radius <= 0.0 {
        return w;
    }
    let cell = |p: Vec3| -> (i32, i32, i32) {
        (
            (p.x / radius).floor() as i32,
            (p.y / radius).floor() as i32,
            (p.z / radius).floor() as i32,
        )
    };
    let mut grid: HashMap<(i32, i32, i32), Vec<Vec3>> = HashMap::new();
    for &v in selected {
        if let Some(vert) = mesh.verts.get(v as usize) {
            grid.entry(cell(vert.co)).or_default().push(vert.co);
        }
    }
    let r2 = radius * radius;
    for (i, vert) in mesh.verts.iter().enumerate() {
        if selected.contains(&(i as u32)) {
            w[i] = 1.0;
            continue;
        }
        let (cx, cy, cz) = cell(vert.co);
        let mut best = f32::INFINITY;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(pts) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for p in pts {
                            best = best.min(p.distance_squared(vert.co));
                        }
                    }
                }
            }
        }
        if best <= r2 {
            w[i] = BrushFalloff::Smooth.weight(best.sqrt() / radius);
        }
    }
    w
}

/// Operação sobre as normais, já em espaço **local** do objeto.
#[derive(Debug, Clone)]
pub enum NormalOp {
    Point { center: Vec3, invert: bool },
    Direction { dir: Vec3 },
    Proxy { src: EditMesh, mapping: NormalMapping, max_distance: Option<f32> },
    Average { mode: AverageMode },
    Rotate { axis: Vec3, angle: f32 },
    Smooth { iterations: u32, factor: f32 },
    Reset,
}

impl NormalOp {
    fn label(&self) -> &'static str {
        match self {
            NormalOp::Point { .. } => "Normais: de um ponto",
            NormalOp::Direction { .. } => "Normais: direcional",
            NormalOp::Proxy { .. } => "Normais: de objeto proxy",
            NormalOp::Average { .. } => "Normais: média",
            NormalOp::Rotate { .. } => "Normais: rotacionar",
            NormalOp::Smooth { .. } => "Normais: suavizar região",
            NormalOp::Reset => "Normais: redefinir",
        }
    }

    fn uses_weights(&self) -> bool {
        !matches!(self, NormalOp::Smooth { .. } | NormalOp::Reset)
    }
}

/// Operação + mistura (0..1) + raio proporcional (local) opcional.
#[derive(Debug, Clone)]
pub struct NormalOpSpec {
    pub op: NormalOp,
    pub mix: f32,
    pub proportional: Option<f32>,
}

/// Aplica a operação respeitando o escopo da seleção (ou malha toda). Devolve corners escritos.
/// Erros do motor deixam a malha intacta.
pub fn apply_normal_op(mesh: &mut EditMesh, sel: &Selection, mode: SelectMode, spec: &NormalOpSpec) -> ModelResult<usize> {
    let mut scope = normal_scope(sel, mode);
    let mut weights: Option<Vec<f32>> = None;
    if let (Some(radius), true, false) = (spec.proportional, spec.op.uses_weights(), matches!(scope, NormalScope::All)) {
        let w = proportional_weights(mesh, &sel.verts, radius);
        let verts: Vec<u32> = w.iter().enumerate().filter(|(_, x)| **x > 0.0).map(|(i, _)| i as u32).collect();
        if !verts.is_empty() {
            scope = NormalScope::Verts(verts);
            weights = Some(w);
        }
    }
    let mix = spec.mix.clamp(0.0, 1.0);
    let opts = NormalEditOptions {
        blend: NormalBlend::Mix(mix),
        weights: weights.as_deref(),
    };
    match &spec.op {
        NormalOp::Point { center, invert } => nt::transfer_normals_from_point(mesh, &scope, *center, *invert, &opts),
        NormalOp::Direction { dir } => nt::transfer_normals_directional(mesh, &scope, *dir, &opts),
        NormalOp::Proxy { src, mapping, max_distance } => {
            nt::transfer_normals_from_mesh(mesh, src, &scope, *mapping, mix, *max_distance, weights.as_deref())
        }
        NormalOp::Average { mode } => nt::average_normals(mesh, &scope, *mode, &opts),
        NormalOp::Rotate { axis, angle } => nt::rotate_normals(mesh, &scope, *axis, *angle, weights.as_deref()),
        NormalOp::Smooth { iterations, factor } => nt::smooth_normals_region(mesh, &scope, *iterations, *factor),
        NormalOp::Reset => nt::reset_normals(mesh, &scope),
    }
}

/// Cópia de `src` com posições (e normais customizadas) levadas ao espaço local de `dst`:
/// permite transferir de um proxy que está em outro lugar/escala da cena.
pub fn proxy_in_space_of(src: &ModelObject, dst: &ModelObject) -> EditMesh {
    let m: Mat4 = dst.model_matrix().inverse() * src.model_matrix();
    let nm = glam::Mat3::from_mat4(m).inverse().transpose();
    let mut mesh = src.mesh.clone();
    mesh.transform_positions(|p| m.transform_point3(p));
    for f in &mut mesh.faces {
        for c in &mut f.corners {
            if let Some(n) = c.normal {
                c.normal = (nm * n).try_normalize();
            }
        }
    }
    mesh
}

/// Arestas "selecionadas" para marcar/limpar sharp: as da seleção de arestas; no modo Face, todas as
/// arestas das faces; no modo Vértice, arestas com os dois extremos selecionados.
pub fn selected_edges(mesh: &EditMesh, sel: &Selection, mode: SelectMode) -> Vec<EdgeKey> {
    let mut out: BTreeSet<EdgeKey> = BTreeSet::new();
    if !sel.edges.is_empty() {
        out.extend(sel.edges.iter().copied());
    } else if mode == SelectMode::Face && !sel.faces.is_empty() {
        for &f in &sel.faces {
            if let Some(face) = mesh.faces.get(f as usize) {
                out.extend(face.edges().map(|(a, b)| edge_key(a, b)));
            }
        }
    } else if !sel.verts.is_empty() {
        let topo = Topology::build(mesh);
        out.extend(sel.edges_fully_selected(&topo));
    }
    out.into_iter().collect()
}

/// Faces a afetar por sombreamento suave/chapado: selecionadas (ou totalmente cobertas pelos vértices
/// selecionados); sem seleção, a malha toda.
pub fn selected_faces_or_all(mesh: &EditMesh, sel: &Selection) -> Vec<u32> {
    if !sel.faces.is_empty() {
        sel.faces.iter().copied().collect()
    } else if !sel.verts.is_empty() {
        let f = sel.faces_fully_selected(mesh);
        if f.is_empty() {
            (0..mesh.face_count() as u32).collect()
        } else {
            f
        }
    } else {
        (0..mesh.face_count() as u32).collect()
    }
}

/// Faces da região do relatório (`None` = malha toda).
pub fn report_region(mesh: &EditMesh, sel: &Selection) -> Option<Vec<u32>> {
    if !sel.faces.is_empty() {
        Some(sel.faces.iter().copied().collect())
    } else if !sel.verts.is_empty() {
        let f = sel.faces_fully_selected(mesh);
        (!f.is_empty()).then_some(f)
    } else {
        None
    }
}

/// Relatório de suavidade das normais efetivas da região. Devolve também o nº de faces medidas.
pub fn compute_report(mesh: &EditMesh, sel: &Selection) -> ModelResult<(NormalReport, Option<usize>)> {
    match report_region(mesh, sel) {
        Some(faces) => {
            let n = faces.len();
            Ok((nt::normal_smoothness_report_region(mesh, &faces)?, Some(n)))
        }
        None => Ok((nt::normal_smoothness_report(mesh), None)),
    }
}

/// Pivô padrão (local): centróide dos vértices selecionados; sem seleção, centro da caixa da malha.
pub fn default_pivot_local(mesh: &EditMesh, sel: &Selection) -> Vec3 {
    if !sel.verts.is_empty() {
        let pts: Vec<Vec3> = sel.verts.iter().filter_map(|v| mesh.verts.get(*v as usize)).map(|v| v.co).collect();
        if !pts.is_empty() {
            return pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        }
    }
    let used = mesh.used_verts();
    let mut it = mesh.verts.iter().enumerate().filter(|(i, _)| used[*i]).map(|(_, v)| v.co);
    match it.next() {
        Some(first) => {
            let (lo, hi) = it.fold((first, first), |(l, h), p| (l.min(p), h.max(p)));
            (lo + hi) * 0.5
        }
        None => Vec3::ZERO,
    }
}

/// Escala média do objeto (converte raios de mundo em locais).
pub fn mean_scale(obj: &ModelObject) -> f32 {
    let s = obj.transform.scale.abs();
    ((s.x + s.y + s.z) / 3.0).max(1e-4)
}

/// Um novo "dab" só acontece depois que o pincel andou 25% do raio desde o último.
pub fn should_dab(last: Option<Vec3>, p: Vec3, radius: f32) -> bool {
    match last {
        None => true,
        Some(l) => l.distance(p) >= radius * 0.25,
    }
}

/// Parâmetros do pincel de atributo (espaço local).
#[derive(Debug, Clone, Copy)]
pub struct Brush {
    pub channel: VertexChannel,
    pub value: f32,
    pub radius: f32,
    pub strength: f32,
    pub falloff: BrushFalloff,
    pub blend: PaintBlend,
}

/// Um toque do pincel de atributo em `center` (local): chama `paint_vertex_channel`.
pub fn paint_dab(mesh: &mut EditMesh, center: Vec3, b: &Brush) -> ModelResult<usize> {
    attributes::paint_vertex_channel(mesh, center, b.radius, b.falloff, b.channel, b.value, b.blend, b.strength)
}

/// Quantos corners têm normal customizada / total.
pub fn custom_normal_counts(mesh: &EditMesh) -> (usize, usize) {
    let mut custom = 0;
    let mut total = 0;
    for f in &mesh.faces {
        for c in &f.corners {
            total += 1;
            custom += c.normal.is_some() as usize;
        }
    }
    (custom, total)
}

// =============================================================================
// Painel
// =============================================================================

/// Ações coletadas durante o desenho do painel e executadas depois (evita emprestar `self` na UI).
enum Act {
    MarkSharp(bool),
    SetSmooth(bool),
    SelectSharpByAngle,
    MarkSharpByAngle,
    Apply,
    Reset,
    Bake,
    ClearCustom,
    GotoEdge(EdgeKey),
    FillChannel,
}

impl StudioApp {
    /// Seção "Normais" (e "Pintura NPR") do Inspector no modo Edição.
    pub(crate) fn inspector_normals(&mut self, ui: &mut egui::Ui) {
        let mut st = std::mem::take(&mut self.normals_ui);
        let mut acts: Vec<Act> = Vec::new();
        let frame = self.frame_count;

        egui::CollapsingHeader::new(RichText::new(tr("panel.normals")).strong())
            .default_open(true)
            .open(st.force_open_normals.take())
            .show(ui, |ui| {
                st.panel_frame = frame;
                self.normals_panel_body(ui, &mut st, &mut acts);
            });
        egui::CollapsingHeader::new(RichText::new(l("Pintura NPR", "NPR paint")).strong())
            .default_open(false)
            .open(st.force_open_paint.take())
            .show(ui, |ui| {
                self.paint_panel_body(ui, &mut st, &mut acts);
            });

        self.normals_ui = st;
        for a in acts {
            self.run_normals_action(a);
        }
    }

    fn normals_panel_body(&mut self, ui: &mut egui::Ui, st: &mut NormalsUi, acts: &mut Vec<Act>) {
        let has_obj = self.modeling.active_object().is_some();
        if !has_obj {
            ui.label(RichText::new(l("Sem objeto ativo", "No active object")).color(theme::TEXT_DIM));
            return;
        }
        // ---------------------------------------------------------- overlay
        ui.checkbox(&mut st.show_normals, l("Mostrar normais por corner", "Show normals per corner"));
        if st.show_normals {
            ui.add(
                egui::Slider::new(&mut st.normal_length, 0.02..=1.0)
                    .logarithmic(true)
                    .text(l("Comprimento", "Length")),
            );
            ui.checkbox(&mut st.selected_only, l("Só vértices selecionados", "Selected vertices only"));
        }
        ui.horizontal_wrapped(|ui| {
            legend_dot(ui, [0.35, 0.58, 1.0], l("automática", "automatic"));
            legend_dot(ui, [1.0, 0.90, 0.20], l("customizada", "custom"));
            legend_dot(ui, [0.15, 0.85, 0.95], "sharp");
        });
        if let Some(o) = self.modeling.active_object() {
            let (custom, total) = custom_normal_counts(&o.mesh);
            ui.label(
                RichText::new(format!("{}: {custom} / {total}", l("Corners com normal customizada", "Corners with custom normal")))
                    .size(11.5)
                    .color(theme::TEXT_DIM),
            );
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(l("Sem seleção, as operações valem para a malha toda.", "With nothing selected, operations affect the whole mesh."))
                .size(11.0)
                .color(theme::TEXT_DIM),
        );

        // ------------------------------------------------ sharp e sombreamento
        egui::CollapsingHeader::new(l("Sharp e sombreamento", "Sharp & shading"))
            .default_open(true)
            .open(st.force_open_sharp.take())
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(l("Marcar sharp", "Mark sharp")).on_hover_text(l("Arestas selecionadas viram duras (ciano)", "Selected edges become hard (cyan)")).clicked() {
                        acts.push(Act::MarkSharp(true));
                    }
                    if ui.button(l("Limpar sharp", "Clear sharp")).clicked() {
                        acts.push(Act::MarkSharp(false));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(l("Sombreamento suave", "Shade smooth"))
                        .on_hover_text(l(
                            "Normais interpoladas nas faces selecionadas (o ângulo de auto-smooth ainda corta quinas)",
                            "Interpolated normals on selected faces (auto-smooth angle still cuts sharp corners)",
                        ))
                        .clicked()
                    {
                        acts.push(Act::SetSmooth(true));
                    }
                    if ui.button(l("Sombreamento chapado", "Shade flat")).clicked() {
                        acts.push(Act::SetSmooth(false));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(l("Sharp por ângulo", "Sharp by angle"));
                    ui.add(egui::DragValue::new(&mut st.sharp_angle_deg).range(0.0..=180.0).speed(0.5).suffix("°"));
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(l("Selecionar", "Select")).clicked() {
                        acts.push(Act::SelectSharpByAngle);
                    }
                    if ui.button(l("Marcar sharp", "Mark sharp")).clicked() {
                        acts.push(Act::MarkSharpByAngle);
                    }
                });
                if let Some(o) = self.modeling.active_object() {
                    let mut deg = o.auto_smooth_angle.to_degrees();
                    let resp = ui
                        .add(egui::Slider::new(&mut deg, 0.0..=180.0).suffix("°").text(l("Auto-smooth", "Auto-smooth")))
                        .on_hover_text(l(
                            "Ângulo máximo entre faces para suavizar (Workbench, preview Cel e export). 180° = sem corte.",
                            "Max angle between faces to smooth (Workbench, Cel preview and export). 180° = no cut.",
                        ));
                    if resp.changed() {
                        let record = !st.smooth_cp;
                        st.smooth_cp = true;
                        self.modeling.set_auto_smooth_angle(deg.to_radians(), record);
                    }
                    if !resp.dragged() && !resp.has_focus() {
                        st.smooth_cp = false;
                    }
                }
            });

        // --------------------------------------------------- editar normais
        egui::CollapsingHeader::new(l("Editar / transferir normais", "Edit / transfer normals"))
            .default_open(true)
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("normal_tool")
                    .selected_text(st.tool.label())
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for t in NormalTool::ALL {
                            ui.selectable_value(&mut st.tool, t, t.label());
                        }
                    });
                self.normal_tool_params(ui, st);
                if st.tool != NormalTool::Average && st.tool != NormalTool::Smooth {
                    ui.add(egui::Slider::new(&mut st.mix, 0.0..=1.0).text(l("Mistura", "Mix")));
                }
                if !matches!(st.tool, NormalTool::Smooth) {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut st.use_proportional, l("Peso proporcional", "Proportional weight"));
                        ui.label(
                            RichText::new(format!("{:.2}", self.modeling.proportional.radius))
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        )
                        .on_hover_text(l("Raio vem de Edição proporcional (O)", "Radius comes from Proportional editing (O)"));
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button(format!("{}  {}", icons::CHECK, l("Aplicar", "Apply"))).clicked() {
                        acts.push(Act::Apply);
                    }
                    if ui.button(l("Redefinir escopo", "Reset scope")).on_hover_text(l("Remove as normais customizadas do escopo", "Removes custom normals from the scope")).clicked() {
                        acts.push(Act::Reset);
                    }
                });
            });

        // -------------------------------------------------------- assar
        egui::CollapsingHeader::new(l("Assar / limpar", "Bake / clear"))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(l("Assar normais", "Bake normals"))
                        .on_hover_text(l(
                            "Congela as normais calculadas (grupos suaves + ângulo) como customizadas",
                            "Freezes computed normals (smooth groups + angle) as custom",
                        ))
                        .clicked()
                    {
                        acts.push(Act::Bake);
                    }
                    if ui.button(l("Limpar customizadas", "Clear custom")).clicked() {
                        acts.push(Act::ClearCustom);
                    }
                });
            });

        // ------------------------------------------------------ relatório
        egui::CollapsingHeader::new(l("Relatório de suavidade", "Smoothness report"))
            .default_open(false)
            .show(ui, |ui| {
                let rev = self.modeling.revision;
                if st.report.as_ref().is_none_or(|r| r.revision != rev) {
                    st.report = self.modeling.active_object().and_then(|o| {
                        compute_report(&o.mesh, &self.modeling.selection)
                            .ok()
                            .map(|(report, region_faces)| ReportCache { revision: rev, report, region_faces })
                    });
                }
                match &st.report {
                    Some(r) => {
                        let scope = match r.region_faces {
                            Some(n) => format!("{n} {}", l("faces selecionadas", "selected faces")),
                            None => l("malha toda", "whole mesh").to_string(),
                        };
                        ui.label(RichText::new(scope).size(11.5).color(theme::TEXT_DIM));
                        egui::Grid::new("normal_report").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
                            let rep = &r.report;
                            ui.label(l("Arestas medidas", "Edges measured"));
                            ui.label(format!("{}", rep.edge_count));
                            ui.end_row();
                            ui.label(l("Maior salto angular", "Largest angular jump"));
                            ui.label(format!("{:.1}°", rep.max_gradient_deg()));
                            ui.end_row();
                            ui.label(l("Salto médio", "Mean jump"));
                            ui.label(format!("{:.1}°", rep.mean_gradient.to_degrees()));
                            ui.end_row();
                            ui.label(l("Maior descontinuidade", "Largest discontinuity"));
                            ui.label(format!("{:.1}°", rep.max_discontinuity_deg()));
                            ui.end_row();
                            ui.label(l("Arestas descontínuas", "Discontinuous edges"));
                            ui.label(format!("{}", rep.discontinuous_edges));
                            ui.end_row();
                        });
                        if let Some(e) = r.report.worst_edge {
                            if ui.button(l("Ir para a pior aresta", "Go to worst edge")).clicked() {
                                acts.push(Act::GotoEdge(e));
                            }
                        }
                    }
                    None => {
                        ui.label(RichText::new("—").color(theme::TEXT_DIM));
                    }
                }
            });
    }

    /// Parâmetros específicos da ferramenta de normais escolhida.
    fn normal_tool_params(&mut self, ui: &mut egui::Ui, st: &mut NormalsUi) {
        match st.tool {
            NormalTool::Point => {
                let auto = self.auto_pivot_world();
                let shown = st.pivot.unwrap_or(auto);
                let mut p = shown;
                ui.horizontal(|ui| {
                    ui.label(l("Pivô", "Pivot"));
                    for v in [&mut p.x, &mut p.y, &mut p.z] {
                        ui.add(egui::DragValue::new(v).speed(0.02).fixed_decimals(2));
                    }
                });
                if p != shown {
                    st.pivot = Some(p);
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.invert, l("Inverter", "Invert"));
                    let label = if st.pivot.is_some() { l("Centro da seleção", "Selection center") } else { l("(centro da seleção)", "(selection center)") };
                    if ui.add_enabled(st.pivot.is_some(), egui::Button::new(label)).clicked() {
                        st.pivot = None;
                    }
                });
            }
            NormalTool::Direction => {
                let mut d = st.direction;
                ui.horizontal(|ui| {
                    ui.label(l("Direção", "Direction"));
                    for v in [&mut d.x, &mut d.y, &mut d.z] {
                        ui.add(egui::DragValue::new(v).speed(0.02).fixed_decimals(2));
                    }
                });
                st.direction = d;
                ui.horizontal_wrapped(|ui| {
                    for (name, v) in [("+X", Vec3::X), ("+Y", Vec3::Y), ("+Z", Vec3::Z), ("-X", -Vec3::X), ("-Y", -Vec3::Y), ("-Z", -Vec3::Z)] {
                        if ui.small_button(name).clicked() {
                            st.direction = v;
                        }
                    }
                });
            }
            NormalTool::Proxy => {
                let active_id = self.modeling.active_object().map(|o| o.id);
                let others: Vec<(u32, String)> = self
                    .modeling
                    .doc
                    .objects
                    .iter()
                    .filter(|o| Some(o.id) != active_id)
                    .map(|o| (o.id, o.name.clone()))
                    .collect();
                if st.proxy.is_some_and(|id| !others.iter().any(|(i, _)| *i == id)) {
                    st.proxy = None;
                }
                let current = st
                    .proxy
                    .and_then(|id| others.iter().find(|(i, _)| *i == id))
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| l("(escolha um objeto)", "(pick an object)").to_string());
                if others.is_empty() {
                    ui.label(RichText::new(l("Adicione outro objeto à cena para usar como proxy.", "Add another object to the scene to use as proxy.")).color(theme::TEXT_DIM));
                }
                egui::ComboBox::from_id_salt("normal_proxy")
                    .selected_text(current)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for (id, name) in &others {
                            ui.selectable_value(&mut st.proxy, Some(*id), name);
                        }
                    });
                egui::ComboBox::from_id_salt("normal_mapping")
                    .selected_text(mapping_label(st.mapping))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for m in [NormalMapping::ClosestSurface, NormalMapping::ClosestVertex, NormalMapping::ClosestCorner] {
                            ui.selectable_value(&mut st.mapping, m, mapping_label(m));
                        }
                    });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.limit_distance, l("Distância máx.", "Max distance"));
                    ui.add_enabled(st.limit_distance, egui::DragValue::new(&mut st.max_distance).speed(0.01).range(0.0..=100.0).fixed_decimals(2));
                });
            }
            NormalTool::Average => {
                egui::ComboBox::from_id_salt("normal_avg")
                    .selected_text(average_label(st.average))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for m in [AverageMode::FaceArea, AverageMode::CornerAngle, AverageMode::Uniform] {
                            ui.selectable_value(&mut st.average, m, average_label(m));
                        }
                    });
            }
            NormalTool::Rotate => {
                ui.horizontal(|ui| {
                    for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.selectable_value(&mut st.rotate_axis, i, *name);
                    }
                    ui.add(egui::DragValue::new(&mut st.rotate_deg).speed(0.5).range(-360.0..=360.0).suffix("°"));
                });
            }
            NormalTool::Smooth => {
                ui.add(egui::Slider::new(&mut st.smooth_iterations, 1..=30).text(l("Iterações", "Iterations")));
                ui.add(egui::Slider::new(&mut st.smooth_factor, 0.0..=1.0).text(l("Fator", "Factor")));
            }
        }
    }

    fn paint_panel_body(&mut self, ui: &mut egui::Ui, st: &mut NormalsUi, acts: &mut Vec<Act>) {
        if self.modeling.active_object().is_none() {
            ui.label(RichText::new(l("Sem objeto ativo", "No active object")).color(theme::TEXT_DIM));
            return;
        }
        let was = self.paint_attr_active;
        ui.toggle_value(
            &mut self.paint_attr_active,
            format!("{}  {}", icons::PAINT_BRUSH, l("Pintar atributo (LMB)", "Paint attribute (LMB)")),
        )
        .on_hover_text(l(
            "Arraste no viewport para pintar o canal no objeto ativo. Alt+arrastar orbita.",
            "Drag in the viewport to paint the channel on the active object. Alt+drag orbits.",
        ));
        if self.paint_attr_active && !was {
            st.view_channel = true;
        }
        egui::ComboBox::from_id_salt("paint_channel")
            .selected_text(channel_label(st.channel))
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                for c in CHANNELS {
                    ui.selectable_value(&mut st.channel, c, channel_label(c));
                }
            });
        ui.add(egui::Slider::new(&mut st.paint_value, 0.0..=1.0).text(l("Valor", "Value")));
        ui.add(egui::Slider::new(&mut st.paint_radius, 0.02..=3.0).logarithmic(true).text(l("Raio", "Radius")));
        ui.add(egui::Slider::new(&mut st.paint_strength, 0.0..=1.0).text(l("Força", "Strength")));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("paint_falloff")
                .selected_text(falloff_label(st.falloff))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for f in [BrushFalloff::Smooth, BrushFalloff::Linear, BrushFalloff::Sphere, BrushFalloff::Root, BrushFalloff::Sharp, BrushFalloff::Constant] {
                        ui.selectable_value(&mut st.falloff, f, falloff_label(f));
                    }
                });
            egui::ComboBox::from_id_salt("paint_blend")
                .selected_text(blend_label(st.blend))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for b in [PaintBlend::Mix, PaintBlend::Add, PaintBlend::Subtract, PaintBlend::Multiply] {
                        ui.selectable_value(&mut st.blend, b, blend_label(b));
                    }
                });
        });
        ui.checkbox(&mut st.view_channel, l("Ver canal (mapa de calor)", "View channel (heat map)"));
        if st.view_channel {
            heat_legend(ui);
        }
        if ui.button(l("Preencher seleção", "Fill selection")).on_hover_text(l("Aplica valor/modo/força aos vértices selecionados (ou a todos)", "Applies value/mode/strength to selected vertices (or all)")).clicked() {
            acts.push(Act::FillChannel);
        }
        ui.label(
            RichText::new(l("Neutro: AO 1, ShadowShift 0.5, Contorno 1, Spec/Rim 1.", "Neutral: AO 1, ShadowShift 0.5, Outline 1, Spec/Rim 1."))
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
    }

    // ----------------------------------------------------------- execução

    /// Pivô padrão em mundo (centro da seleção ou do objeto).
    fn auto_pivot_world(&self) -> Vec3 {
        match self.modeling.active_object() {
            Some(o) => o.model_matrix().transform_point3(default_pivot_local(&o.mesh, &self.modeling.selection)),
            None => Vec3::ZERO,
        }
    }

    /// Monta a operação em espaço local a partir do estado do painel.
    fn build_normal_spec(&self, st: &NormalsUi) -> Result<NormalOpSpec, String> {
        let obj = self.modeling.active_object().ok_or_else(|| l("sem objeto ativo", "no active object").to_string())?;
        let inv = obj.model_matrix().inverse();
        let op = match st.tool {
            NormalTool::Point => NormalOp::Point {
                center: inv.transform_point3(st.pivot.unwrap_or_else(|| self.auto_pivot_world())),
                invert: st.invert,
            },
            NormalTool::Direction => {
                let d = inv.transform_vector3(st.direction);
                if d.try_normalize().is_none() {
                    return Err(l("direção nula", "zero direction").to_string());
                }
                NormalOp::Direction { dir: d }
            }
            NormalTool::Proxy => {
                let id = st.proxy.ok_or_else(|| l("escolha um objeto proxy", "pick a proxy object").to_string())?;
                let src = self
                    .modeling
                    .doc
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .ok_or_else(|| l("proxy não existe mais", "proxy no longer exists").to_string())?;
                NormalOp::Proxy {
                    src: proxy_in_space_of(src, obj),
                    mapping: st.mapping,
                    max_distance: st.limit_distance.then_some(st.max_distance / mean_scale(obj)),
                }
            }
            NormalTool::Average => NormalOp::Average { mode: st.average },
            NormalTool::Rotate => {
                let mut axis = Vec3::ZERO;
                axis[st.rotate_axis.min(2)] = 1.0;
                NormalOp::Rotate {
                    axis: inv.transform_vector3(axis),
                    angle: st.rotate_deg.to_radians(),
                }
            }
            NormalTool::Smooth => NormalOp::Smooth {
                iterations: st.smooth_iterations,
                factor: st.smooth_factor,
            },
        };
        let proportional = (st.use_proportional && self.modeling.proportional.radius > 0.0)
            .then(|| self.modeling.proportional.radius / mean_scale(obj));
        Ok(NormalOpSpec { op, mix: st.mix, proportional })
    }

    fn run_normals_action(&mut self, act: Act) {
        let mode = self.modeling.select_mode;
        match act {
            Act::Apply => match self.build_normal_spec(&self.normals_ui) {
                Ok(spec) => self.run_normal_spec(spec),
                Err(e) => self.modeling.status = format!("Normais: {e}"),
            },
            Act::Reset => self.run_normal_spec(NormalOpSpec { op: NormalOp::Reset, mix: 1.0, proportional: None }),
            Act::MarkSharp(on) => {
                let label = if on { "Marcar sharp" } else { "Limpar sharp" };
                let r = self.modeling.edit_mesh(label, |mesh, sel| {
                    let edges = selected_edges(mesh, sel, mode);
                    misc::mark_sharp(mesh, &edges, on).map(|o| o.selection.edges.len())
                });
                self.report_result(label, r, "arestas");
            }
            Act::SetSmooth(on) => {
                let label = if on { "Sombreamento suave" } else { "Sombreamento chapado" };
                let r = self.modeling.edit_mesh(label, |mesh, sel| {
                    let faces = selected_faces_or_all(mesh, sel);
                    misc::set_faces_smooth(mesh, &faces, on).map(|o| o.selection.faces.len())
                });
                self.report_result(label, r, "faces");
            }
            Act::SelectSharpByAngle => {
                let angle = self.normals_ui.sharp_angle_deg.to_radians();
                let res = self.modeling.active_object().map(|o| misc::select_sharp_edges(&o.mesh, angle));
                match res {
                    Some(Ok(sel)) => {
                        let n = sel.edges.len();
                        self.modeling.set_select_mode(SelectMode::Edge);
                        self.modeling.set_selection(sel, None);
                        self.modeling.status = format!("Sharp por ângulo: {n} arestas selecionadas");
                    }
                    Some(Err(e)) => self.modeling.status = format!("Sharp por ângulo: {e}"),
                    None => {}
                }
            }
            Act::MarkSharpByAngle => {
                let angle = self.normals_ui.sharp_angle_deg.to_radians();
                let r = self.modeling.edit_mesh("Sharp por ângulo", |mesh, _| {
                    let sel = misc::select_sharp_edges(mesh, angle)?;
                    if sel.edges.is_empty() {
                        return Err(ModelError::EmptySelection("nenhuma aresta acima do ângulo"));
                    }
                    let edges: Vec<EdgeKey> = sel.edges.iter().copied().collect();
                    misc::mark_sharp(mesh, &edges, true).map(|o| o.selection.edges.len())
                });
                self.report_result("Sharp por ângulo", r, "arestas marcadas");
            }
            Act::Bake => {
                let params = self.modeling.active_object().map(|o| o.normal_params());
                if let Some(params) = params {
                    let r = self.modeling.edit_mesh("Assar normais", |mesh, _| {
                        normals::bake_custom_normals(mesh, &params);
                        Ok(mesh.faces.iter().map(|f| f.corners.len()).sum::<usize>())
                    });
                    self.report_result("Assar normais", r, "corners");
                }
            }
            Act::ClearCustom => {
                let r = self.modeling.edit_mesh("Limpar normais customizadas", |mesh, _| {
                    let (custom, _) = custom_normal_counts(mesh);
                    normals::clear_custom_normals(mesh);
                    Ok(custom)
                });
                self.report_result("Limpar normais customizadas", r, "corners");
            }
            Act::GotoEdge(e) => {
                self.modeling.set_select_mode(SelectMode::Edge);
                self.modeling.set_selection(Selection::from_edges([e]), None);
            }
            Act::FillChannel => {
                let st = &self.normals_ui;
                let (ch, value, blend, strength) = (st.channel, st.paint_value, st.blend, st.paint_strength);
                let r = self.modeling.edit_mesh("Preencher canal", |mesh, sel| {
                    let verts: Vec<u32> = sel.verts.iter().copied().collect();
                    let list = (!verts.is_empty()).then_some(verts.as_slice());
                    attributes::fill_channel(mesh, list, ch, value, blend, strength)
                });
                self.report_result("Preencher canal", r, "vértices");
            }
        }
    }

    fn run_normal_spec(&mut self, spec: NormalOpSpec) {
        let mode = self.modeling.select_mode;
        let label = spec.op.label();
        let r = self.modeling.edit_mesh(label, |mesh, sel| apply_normal_op(mesh, sel, mode, &spec));
        self.report_result(label, r, "corners");
    }

    fn report_result(&mut self, label: &str, r: Result<usize, ModelError>, unit: &str) {
        self.modeling.status = match r {
            Ok(n) => format!("{label}: {n} {unit}"),
            Err(e) => format!("{label}: {e}"),
        };
    }

    // ---------------------------------------------------- pintura no viewport

    /// Entrada do viewport com a ferramenta "Pintar atributo" ativa. Devolve `true` se consumiu o
    /// botão primário (o viewport não seleciona nem faz caixa).
    pub(crate) fn paint_attr_input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, size: Vec2) -> bool {
        if !self.paint_attr_active || self.modeling.mode != Mode::Edit || self.modeling.modal_active() {
            self.normals_ui.brush_hit = None;
            self.normals_ui.stroke = None;
            return false;
        }
        let alt = ui.input(|i| i.modifiers.alt);
        let (pressed, down) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down()));
        let Some(obj) = self.modeling.active_object() else {
            return false;
        };
        let mouse = Vec2::new(self.last_mouse.x - rect.min.x, self.last_mouse.y - rect.min.y);
        let (hit_local, hit_world) = if resp.hovered() || self.normals_ui.stroke.is_some() {
            let model = obj.model_matrix();
            let inv = model.inverse();
            let (o, d) = self.camera.ray_from_pixel(mouse, size);
            let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d));
            match picking::raycast_mesh(&obj.mesh, lo, ld) {
                Some((_, t)) => {
                    let p = lo + ld * t;
                    (Some(p), Some(model.transform_point3(p)))
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };
        let radius_local = self.normals_ui.paint_radius / mean_scale(obj);
        self.normals_ui.brush_hit = hit_world;

        if alt {
            return false;
        }
        if pressed && resp.hovered() && self.normals_ui.stroke.is_none() && hit_local.is_some() {
            self.modeling.checkpoint("Pintar atributo");
            self.normals_ui.stroke = Some(None);
        }
        if !down {
            self.normals_ui.stroke = None;
        }
        if let (Some(last), Some(p)) = (self.normals_ui.stroke, hit_local) {
            if should_dab(last, p, radius_local) {
                let st = &self.normals_ui;
                let brush = Brush {
                    channel: st.channel,
                    value: st.paint_value,
                    radius: radius_local,
                    strength: st.paint_strength,
                    falloff: st.falloff,
                    blend: st.blend,
                };
                if let Some(Err(e)) = self.modeling.mutate_active_mesh(|mesh| paint_dab(mesh, p, &brush)) {
                    self.modeling.status = format!("Pintura: {e}");
                }
                self.normals_ui.stroke = Some(Some(p));
                ui.ctx().request_repaint();
            }
        }
        resp.hovered() || self.normals_ui.stroke.is_some()
    }

    /// Overlays do painel de normais: círculo do pincel de atributo e marcador do pivô da esfera de sombreamento.
    pub(crate) fn normals_overlay(&mut self, ui: &mut egui::Ui, rect: egui::Rect, size: Vec2) {
        if self.modeling.mode != Mode::Edit {
            return;
        }
        let painter = ui.painter_at(rect);
        let cam = &self.camera;
        let to_screen = |p: Vec3| cam.project(p, size).map(|(px, _)| rect.min + egui::vec2(px.x, px.y));
        if self.paint_attr_active {
            if let Some(hit) = self.normals_ui.brush_hit {
                if let (Some(c), Some(e)) = (to_screen(hit), to_screen(hit + cam.right() * self.normals_ui.paint_radius)) {
                    let r = c.distance(e).max(3.0);
                    painter.circle_stroke(c, r, egui::Stroke::new(1.5, Color32::WHITE));
                    painter.circle_stroke(c, r * 0.5, egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 120)));
                }
            }
        }
        let panel_visible = self.frame_count.saturating_sub(self.normals_ui.panel_frame) <= 1 && self.normals_ui.panel_frame > 0;
        if panel_visible && self.normals_ui.tool == NormalTool::Point {
            let pivot = self.normals_ui.pivot.unwrap_or_else(|| self.auto_pivot_world());
            if let Some(c) = to_screen(pivot) {
                let col = Color32::from_rgb(255, 230, 51);
                painter.circle_stroke(c, 9.0, egui::Stroke::new(1.8, col));
                painter.line_segment([c - egui::vec2(14.0, 0.0), c + egui::vec2(14.0, 0.0)], egui::Stroke::new(1.4, col));
                painter.line_segment([c - egui::vec2(0.0, 14.0), c + egui::vec2(0.0, 14.0)], egui::Stroke::new(1.4, col));
            }
        }
    }
}

fn mapping_label(m: NormalMapping) -> &'static str {
    match m {
        NormalMapping::ClosestSurface => l("Superfície mais próxima", "Closest surface"),
        NormalMapping::ClosestVertex => l("Vértice mais próximo", "Closest vertex"),
        NormalMapping::ClosestCorner => l("Corner mais próximo", "Closest corner"),
    }
}

fn average_label(m: AverageMode) -> &'static str {
    match m {
        AverageMode::FaceArea => l("Por área da face", "Face area"),
        AverageMode::CornerAngle => l("Por ângulo do corner", "Corner angle"),
        AverageMode::Uniform => l("Simples", "Uniform"),
    }
}

fn legend_dot(ui: &mut egui::Ui, rgb: [f32; 3], text: &str) {
    let c = Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8);
    let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, c);
    ui.label(RichText::new(text).size(11.5).color(theme::TEXT_DIM));
}

/// Barra de gradiente 0..1 do mapa de calor.
fn heat_legend(ui: &mut egui::Ui) {
    let w = ui.available_width().min(240.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), egui::Sense::hover());
    let steps = 32;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let c = crate::workbench::heat_color(t0 + 0.5 / steps as f32);
        let col = Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
        let x0 = rect.left() + rect.width() * t0;
        let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1 + 0.5, rect.bottom())), 0.0, col);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("0").size(10.5).color(theme::TEXT_DIM));
        ui.add_space((w - 24.0).max(0.0));
        ui.label(RichText::new("1").size(10.5).color(theme::TEXT_DIM));
    });
}

/// Estado de sessão usado só por cenários/testes: aplica uma operação como o painel faria.
#[allow(dead_code)]
pub(crate) fn apply_spec_to_session(s: &mut ModelingSession, spec: NormalOpSpec) -> Result<usize, ModelError> {
    let mode = s.select_mode;
    s.edit_mesh(spec.op.label(), |mesh, sel| apply_normal_op(mesh, sel, mode, &spec))
}

// =============================================================================
// Cenários roteirizados (screenshots determinísticos)
// =============================================================================

/// Deforma uma esfera em algo parecido com uma cabeça (queixo afilado, nariz): a geometria sozinha
/// produz um terminador de sombra "torto"; é o caso em que a esfera de sombreamento ajuda.
fn head_shape(mesh: &mut EditMesh) {
    mesh.transform_positions(|p| {
        let mut q = p;
        if q.y < 0.0 {
            let t = (-q.y).clamp(0.0, 1.0);
            let k = 1.0 - 0.55 * t * t;
            q.x *= k;
            q.z *= 0.8 + 0.2 * k;
        }
        q.y *= 1.12;
        if q.z > 0.0 {
            let d2 = q.x * q.x + (q.y + 0.05) * (q.y + 0.05);
            q.z += 0.22 * (-d2 / 0.03).exp();
            // brow
            let b = (q.y - 0.32) * (q.y - 0.32);
            q.z += 0.07 * (-(q.x * q.x * 0.6 + b) / 0.02).exp();
        }
        q
    });
}

impl StudioApp {
    /// Cenários `normals-*`. Devolve `true` se o nome era de um deles.
    pub(crate) fn scenario_normals(&mut self, name: &str) -> bool {
        use crate::modeling::PrimitiveKind;
        let light = Vec3::new(-0.85, 0.35, 0.45).normalize();
        match name {
            "normals-overlay" => {
                let m = &mut self.modeling;
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let mesh = m.active_object().map(|o| o.mesh.clone()).unwrap_or_else(|| EditMesh::new("vazio"));
                // faixa de faces na frente: recebem normais customizadas de um ponto atrás da esfera
                let faces: Vec<u32> = (0..mesh.face_count() as u32)
                    .filter(|f| {
                        let c = mesh.face_positions(*f).iter().copied().sum::<Vec3>() / mesh.faces[*f as usize].corners.len() as f32;
                        c.z > 0.3 && c.y > 0.05 && c.y < 0.7
                    })
                    .collect();
                m.set_selection(Selection::from_faces(&mesh, faces), None);
                let r = apply_spec_to_session(m, NormalOpSpec { op: NormalOp::Point { center: Vec3::new(0.0, -0.2, -1.6), invert: false }, mix: 1.0, proportional: None });
                if let Err(e) = r {
                    eprintln!("normals-overlay: {e}");
                }
                // equador marcado como sharp (ciano)
                let r = m.edit_mesh("Marcar sharp", |mesh, _| {
                    let topo = Topology::build(mesh);
                    let edges: Vec<EdgeKey> = topo
                        .edges
                        .iter()
                        .filter(|e| mesh.co(e.verts.0).y.abs() < 1e-3 && mesh.co(e.verts.1).y.abs() < 1e-3)
                        .map(|e| e.verts)
                        .collect();
                    misc::mark_sharp(mesh, &edges, true).map(|o| o.selection.edges.len())
                });
                if let Err(e) = r {
                    eprintln!("normals-overlay sharp: {e}");
                }
                m.select_none();
                self.normals_ui.show_normals = true;
                self.normals_ui.normal_length = 0.24;
                self.normals_ui.force_open_normals = Some(true);
                self.normals_ui.force_open_sharp = Some(true);
                self.normals_ui.force_open_paint = Some(false);
                self.camera.distance = 4.2;
                true
            }
            "normals-transfer" | "normals-transfer-clean" => {
                let clean = name == "normals-transfer-clean";
                let m = &mut self.modeling;
                m.delete_active_object();
                let a = m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(-1.6, 1.0, 0.0));
                let b = m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(1.6, 1.0, 0.0));
                for idx in [a, b] {
                    head_shape(&mut m.doc.objects[idx].mesh);
                }
                // direita: normais de um ponto no centro da cabeça (esfera de sombreamento)
                m.doc.active = Some(b);
                let pivot = default_pivot_local(&m.doc.objects[b].mesh, &Selection::default());
                let r = m.edit_mesh("Normais: de um ponto", |mesh, sel| {
                    apply_normal_op(mesh, sel, SelectMode::Face, &NormalOpSpec { op: NormalOp::Point { center: pivot, invert: false }, mix: 1.0, proportional: None })
                });
                if let Err(e) = r {
                    eprintln!("normals-transfer: {e}");
                }
                self.view_shading = crate::app::ViewShading::Cel;
                self.camera.distance = 7.5;
                if let Some(c) = self.gpu.as_mut().and_then(|g| g.cel.as_mut()) {
                    c.light.direction = light.to_array();
                }
                if clean {
                    self.modeling.doc.active = None;
                } else {
                    self.modeling.set_mode(Mode::Edit);
                    self.normals_ui.tool = NormalTool::Point;
                    self.normals_ui.show_normals = false;
                    self.normals_ui.force_open_normals = Some(true);
                    self.normals_ui.force_open_sharp = Some(false);
                    self.normals_ui.force_open_paint = Some(false);
                }
                true
            }
            "normals-paint" => {
                let m = &mut self.modeling;
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Vertex);
                m.checkpoint("Pintar atributo");
                let on_sphere = |az: f32, el: f32| Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
                let strokes: [(f32, f32, PaintBlend, f32, BrushFalloff); 2] = [
                    (0.0, 0.0, PaintBlend::Mix, 0.0, BrushFalloff::Smooth),
                    (0.5, 0.6, PaintBlend::Mix, 0.5, BrushFalloff::Linear),
                ];
                for (k, (_, _, blend, value, falloff)) in strokes.iter().enumerate() {
                    let brush = Brush { channel: VertexChannel::AmbientOcclusion, value: *value, radius: 0.3, strength: 1.0, falloff: *falloff, blend: *blend };
                    for i in 0..12 {
                        let t = i as f32 / 11.0;
                        let p = if k == 0 {
                            on_sphere(-0.9 + 1.8 * t, 0.15 + 0.45 * (t * std::f32::consts::PI).sin())
                        } else {
                            on_sphere(-0.5 + 1.2 * t, -0.35 - 0.2 * t)
                        };
                        m.mutate_active_mesh(|mesh| paint_dab(mesh, p, &brush));
                    }
                }
                self.paint_attr_active = true;
                self.normals_ui.force_open_normals = Some(false);
                self.normals_ui.force_open_sharp = Some(true);
                self.normals_ui.force_open_paint = Some(true);
                self.normals_ui.view_channel = true;
                self.normals_ui.channel = VertexChannel::AmbientOcclusion;
                self.normals_ui.paint_radius = 0.3;
                self.normals_ui.brush_hit = Some(Vec3::new(0.0, 1.0, 0.0) + on_sphere(0.35, 0.1));
                self.camera.distance = 4.0;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::PrimitiveKind;
    use anigo_modeling::primitives;

    fn spec(op: NormalOp) -> NormalOpSpec {
        NormalOpSpec { op, mix: 1.0, proportional: None }
    }

    fn custom_corners(mesh: &EditMesh) -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        for (fi, f) in mesh.faces.iter().enumerate() {
            for (ci, c) in f.corners.iter().enumerate() {
                if c.normal.is_some() {
                    v.push((fi as u32, ci as u32));
                }
            }
        }
        v
    }

    fn edit_session(kind: PrimitiveKind) -> ModelingSession {
        let mut s = ModelingSession::new();
        s.add_primitive(kind, Vec3::ZERO);
        s.set_mode(Mode::Edit);
        s
    }

    #[test]
    fn scope_follows_selection_mode_and_falls_back_to_whole_mesh() {
        let m = primitives::cube(2.0);
        let faces = Selection::from_faces(&m, [1u32, 3]);
        assert_eq!(normal_scope(&faces, SelectMode::Face), NormalScope::Faces(vec![1, 3]));
        let verts = Selection::from_verts([4u32, 2]);
        assert_eq!(normal_scope(&verts, SelectMode::Vertex), NormalScope::Verts(vec![2, 4]));
        assert_eq!(normal_scope(&Selection::default(), SelectMode::Face), NormalScope::All);
        assert_eq!(normal_scope(&Selection::default(), SelectMode::Vertex), NormalScope::All);
    }

    #[test]
    fn point_transfer_only_touches_the_selected_faces() {
        let mut m = primitives::cube(2.0);
        let sel = Selection::from_faces(&m, [2u32]);
        let n = apply_normal_op(&mut m, &sel, SelectMode::Face, &spec(NormalOp::Point { center: Vec3::ZERO, invert: false })).unwrap();
        assert_eq!(n, 4);
        let custom = custom_corners(&m);
        assert_eq!(custom.len(), 4);
        assert!(custom.iter().all(|(f, _)| *f == 2));
        // radiais a partir do centro
        for (f, c) in custom {
            let corner = &m.faces[f as usize].corners[c as usize];
            let radial = m.co(corner.vert).normalize();
            assert!((corner.normal.unwrap() - radial).length() < 1e-5);
        }
        // sem seleção: malha toda
        let mut whole = primitives::cube(2.0);
        let n = apply_normal_op(&mut whole, &Selection::default(), SelectMode::Face, &spec(NormalOp::Point { center: Vec3::ZERO, invert: true })).unwrap();
        assert_eq!(n, 24);
        assert_eq!(custom_corners(&whole).len(), 24);
        let c = &whole.faces[0].corners[0];
        assert!((c.normal.unwrap() + whole.co(c.vert).normalize()).length() < 1e-5, "invert aponta para o centro");
    }

    #[test]
    fn mix_blends_between_old_and_new_normal() {
        let mut m = primitives::cube(2.0);
        let face_n = m.face_normal(0);
        let sel = Selection::from_faces(&m, [0u32]);
        let target = NormalOp::Direction { dir: face_n.any_orthogonal_vector().normalize() };
        let dir = face_n.any_orthogonal_vector().normalize();
        let mut half = spec(target.clone());
        half.mix = 0.5;
        apply_normal_op(&mut m, &sel, SelectMode::Face, &half).unwrap();
        let n = m.faces[0].corners[0].normal.unwrap();
        let expected = (face_n * 0.5 + dir * 0.5).normalize();
        assert!((n - expected).length() < 1e-4, "{n:?} vs {expected:?}");
        let mut zero = primitives::cube(2.0);
        let mut none = spec(target);
        none.mix = 0.0;
        apply_normal_op(&mut zero, &sel, SelectMode::Face, &none).unwrap();
        assert!((zero.faces[0].corners[0].normal.unwrap() - face_n).length() < 1e-4);
    }

    #[test]
    fn proportional_weights_fall_off_smoothly_from_the_selection() {
        let m = primitives::grid(8, 8, 4.0, 4.0);
        let center = (0..m.vert_count() as u32)
            .min_by(|a, b| m.co(*a).length().total_cmp(&m.co(*b).length()))
            .unwrap();
        let sel: BTreeSet<u32> = [center].into_iter().collect();
        let w = proportional_weights(&m, &sel, 1.0);
        assert_eq!(w[center as usize], 1.0);
        for (i, wi) in w.iter().enumerate() {
            let d = m.co(i as u32).distance(m.co(center));
            if d > 1.0 {
                assert_eq!(*wi, 0.0, "fora do raio");
            } else if i as u32 != center {
                assert!(*wi >= 0.0 && *wi < 1.0);
                assert!((*wi - BrushFalloff::Smooth.weight(d)).abs() < 1e-5);
            }
        }
        assert!(proportional_weights(&m, &BTreeSet::new(), 1.0).iter().all(|x| *x == 0.0));
        assert!(proportional_weights(&m, &sel, 0.0).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn proportional_scope_reaches_beyond_the_selection() {
        let base = primitives::grid(8, 8, 4.0, 4.0);
        let center = (0..base.vert_count() as u32)
            .min_by(|a, b| base.co(*a).length().total_cmp(&base.co(*b).length()))
            .unwrap();
        let sel = Selection::from_verts([center]);
        let mut plain = base.clone();
        let n_plain = apply_normal_op(&mut plain, &sel, SelectMode::Vertex, &spec(NormalOp::Direction { dir: Vec3::X })).unwrap();
        let mut prop = base.clone();
        let mut s = spec(NormalOp::Direction { dir: Vec3::X });
        s.proportional = Some(1.0);
        let n_prop = apply_normal_op(&mut prop, &sel, SelectMode::Vertex, &s).unwrap();
        assert!(n_prop > n_plain, "{n_prop} <= {n_plain}");
        // o vértice selecionado recebe o alvo inteiro (peso 1)
        let at = |m: &EditMesh, v: u32| -> Vec3 {
            for f in &m.faces {
                for c in &f.corners {
                    if c.vert == v {
                        return c.normal.unwrap();
                    }
                }
            }
            panic!("sem corner");
        };
        assert!((at(&prop, center) - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn normal_edit_through_session_is_undoable_bit_for_bit() {
        let mut s = edit_session(PrimitiveKind::UvSphere);
        let before = s.doc.objects[0].mesh.clone();
        let faces: Vec<u32> = (0..20).collect();
        s.set_select_mode(SelectMode::Face);
        s.set_selection(Selection::from_faces(&before, faces), None);
        let sel_before = s.selection.clone();
        let n = apply_spec_to_session(&mut s, spec(NormalOp::Point { center: Vec3::new(0.0, -0.5, 0.0), invert: false })).unwrap();
        assert!(n > 0);
        let after = s.doc.objects[0].mesh.clone();
        assert_ne!(after, before);
        assert_eq!(s.selection, sel_before, "a seleção é preservada");
        assert!(s.undo().is_some());
        assert_eq!(s.doc.objects[0].mesh, before, "undo exato");
        assert!(s.redo().is_some());
        assert_eq!(s.doc.objects[0].mesh, after, "redo exato");
    }

    #[test]
    fn failing_op_leaves_mesh_and_history_untouched() {
        let mut s = edit_session(PrimitiveKind::Cube);
        let before = s.doc.objects[0].mesh.clone();
        let r = apply_spec_to_session(&mut s, spec(NormalOp::Direction { dir: Vec3::ZERO }));
        assert!(r.is_err());
        assert_eq!(s.doc.objects[0].mesh, before);
        let undo_before = s.history_labels().len();
        let r = apply_spec_to_session(&mut s, spec(NormalOp::Rotate { axis: Vec3::ZERO, angle: 1.0 }));
        assert!(r.is_err());
        assert_eq!(s.history_labels().len(), undo_before);
    }

    #[test]
    fn reset_rotate_average_and_smooth_run_on_the_selection_scope() {
        let mut m = primitives::uv_sphere(1.0, 8, 12);
        let all = Selection::default();
        apply_normal_op(&mut m, &all, SelectMode::Vertex, &spec(NormalOp::Point { center: Vec3::ZERO, invert: false })).unwrap();
        let total = custom_normal_counts(&m).1;
        assert_eq!(custom_normal_counts(&m).0, total);
        // rotacionar 90° em Y gira as normais
        let before = m.faces[10].corners[0].normal.unwrap();
        apply_normal_op(&mut m, &all, SelectMode::Vertex, &spec(NormalOp::Rotate { axis: Vec3::Y, angle: std::f32::consts::FRAC_PI_2 })).unwrap();
        let after = m.faces[10].corners[0].normal.unwrap();
        let expected = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * before;
        assert!((after - expected).length() < 1e-4);
        // média e suavização mantêm normais unitárias
        apply_normal_op(&mut m, &all, SelectMode::Vertex, &spec(NormalOp::Average { mode: AverageMode::FaceArea })).unwrap();
        apply_normal_op(&mut m, &all, SelectMode::Vertex, &spec(NormalOp::Smooth { iterations: 2, factor: 0.5 })).unwrap();
        for f in &m.faces {
            for c in &f.corners {
                assert!((c.normal.unwrap().length() - 1.0).abs() < 1e-4);
            }
        }
        // reset só no escopo: seleciona 1 vértice -> só os corners dele perdem a customizada
        let one = Selection::from_verts([5u32]);
        let n = apply_normal_op(&mut m, &one, SelectMode::Vertex, &spec(NormalOp::Reset)).unwrap();
        assert!(n > 0 && n < total);
        assert_eq!(custom_normal_counts(&m).0, total - n);
        apply_normal_op(&mut m, &all, SelectMode::Vertex, &spec(NormalOp::Reset)).unwrap();
        assert_eq!(custom_normal_counts(&m).0, 0);
    }

    #[test]
    fn proxy_is_brought_into_the_target_object_space() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::new(0.0, 0.0, 0.0));
        s.add_primitive(PrimitiveKind::UvSphere, Vec3::new(5.0, 0.0, 0.0));
        let (dst, src) = (&s.doc.objects[0], &s.doc.objects[1]);
        let proxy = proxy_in_space_of(src, dst);
        let c = proxy.centroid();
        assert!((c - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4, "{c:?}");
        // escala do destino compensa: destino com escala 2 enxerga o proxy com metade da distância
        let mut dst2 = dst.clone();
        dst2.transform.scale = Vec3::splat(2.0);
        let half = proxy_in_space_of(src, &dst2);
        assert!((half.centroid() - Vec3::new(2.5, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn transfer_from_proxy_object_gives_spherical_normals_to_a_cube() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        s.add_primitive(PrimitiveKind::UvSphere, Vec3::new(1.0, 0.0, 0.0));
        s.set_active(0);
        s.set_mode(Mode::Edit);
        let proxy = proxy_in_space_of(&s.doc.objects[1], &s.doc.objects[0]);
        let n = apply_spec_to_session(
            &mut s,
            spec(NormalOp::Proxy { src: proxy, mapping: NormalMapping::ClosestSurface, max_distance: None }),
        )
        .unwrap();
        assert_eq!(n, 24);
        let cube = &s.doc.objects[0].mesh;
        for f in &cube.faces {
            for c in &f.corners {
                // a esfera proxy está centrada em (1,0,0) no espaço do cubo
                let radial = (cube.co(c.vert) - Vec3::new(1.0, 0.0, 0.0)).normalize();
                assert!(c.normal.unwrap().dot(radial) > 0.9, "{:?} vs {:?}", c.normal, radial);
            }
        }
        // distância máxima curta: só vértices praticamente sobre a esfera recebem normal
        let mut s2 = ModelingSession::new();
        s2.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        s2.add_primitive(PrimitiveKind::UvSphere, Vec3::new(1.0, 0.0, 0.0));
        s2.set_active(0);
        let proxy = proxy_in_space_of(&s2.doc.objects[1], &s2.doc.objects[0]);
        let m = &mut s2.doc.objects[0].mesh;
        let n = apply_normal_op(
            m,
            &Selection::default(),
            SelectMode::Face,
            &spec(NormalOp::Proxy { src: proxy, mapping: NormalMapping::ClosestSurface, max_distance: Some(1e-3) }),
        )
        .unwrap();
        assert!(n < 24);
    }

    #[test]
    fn selected_edges_and_faces_follow_the_selection_mode() {
        let m = primitives::cube(2.0);
        let face = Selection::from_faces(&m, [0u32]);
        assert_eq!(selected_edges(&m, &face, SelectMode::Face).len(), 4);
        let topo = Topology::build(&m);
        let e = topo.edges[0].verts;
        let verts = Selection::from_verts([e.0, e.1]);
        assert_eq!(selected_edges(&m, &verts, SelectMode::Vertex), vec![edge_key(e.0, e.1)]);
        let edges = Selection::from_edges([e]);
        assert_eq!(selected_edges(&m, &edges, SelectMode::Edge), vec![edge_key(e.0, e.1)]);
        assert!(selected_edges(&m, &Selection::default(), SelectMode::Edge).is_empty());

        assert_eq!(selected_faces_or_all(&m, &Selection::default()).len(), 6);
        assert_eq!(selected_faces_or_all(&m, &face), vec![0]);
        let all_verts = Selection::from_verts(0..8u32);
        assert_eq!(selected_faces_or_all(&m, &all_verts).len(), 6);
    }

    #[test]
    fn mark_sharp_and_shading_ops_via_session_are_undoable() {
        let mut s = edit_session(PrimitiveKind::UvSphere);
        let before = s.doc.objects[0].mesh.clone();
        s.set_select_mode(SelectMode::Face);
        s.set_selection(Selection::from_faces(&before, [0u32, 1]), None);
        let mode = s.select_mode;
        let n = s
            .edit_mesh("sharp", |mesh, sel| {
                let edges = selected_edges(mesh, sel, mode);
                misc::mark_sharp(mesh, &edges, true).map(|o| o.selection.edges.len())
            })
            .unwrap();
        assert!(n >= 4);
        assert!(s.doc.objects[0].mesh.edge_attrs.values().any(|a| a.sharp));
        s.edit_mesh("flat", |mesh, sel| {
            let f = selected_faces_or_all(mesh, sel);
            misc::set_faces_smooth(mesh, &f, false).map(|_| ())
        })
        .unwrap();
        assert!(!s.doc.objects[0].mesh.faces[0].smooth);
        assert!(s.doc.objects[0].mesh.faces[5].smooth, "fora da seleção continua suave");
        s.undo();
        s.undo();
        assert_eq!(s.doc.objects[0].mesh, before);
    }

    #[test]
    fn report_reflects_region_and_normal_edits() {
        let mut m = primitives::cube(2.0);
        let (whole, region) = compute_report(&m, &Selection::default()).unwrap();
        assert_eq!(region, None);
        assert_eq!(whole.edge_count, 12);
        assert_eq!(whole.discontinuous_edges, 12);
        assert!((whole.max_discontinuity_deg() - 90.0).abs() < 1.0);
        // região: 1 face -> 4 arestas
        let face0 = Selection::from_faces(&m, [0u32]);
        let (part, region) = compute_report(&m, &face0).unwrap();
        assert_eq!(region, Some(1));
        assert_eq!(part.edge_count, 4);
        // normais de ponto em toda a malha -> contínuas nas arestas
        apply_normal_op(&mut m, &Selection::default(), SelectMode::Face, &spec(NormalOp::Point { center: Vec3::ZERO, invert: false })).unwrap();
        let (after, _) = compute_report(&m, &Selection::default()).unwrap();
        assert_eq!(after.discontinuous_edges, 0);
        assert!(after.max_gradient > 0.0 && after.worst_edge.is_some());
    }

    #[test]
    fn default_pivot_is_selection_centroid_or_mesh_center() {
        let mut m = primitives::cube(2.0);
        m.transform_positions(|p| p + Vec3::new(0.0, 3.0, 0.0));
        assert!((default_pivot_local(&m, &Selection::default()) - Vec3::new(0.0, 3.0, 0.0)).length() < 1e-5);
        let top: Vec<u32> = (0..8u32).filter(|v| m.co(*v).y > 3.5).collect();
        let sel = Selection::from_verts(top);
        assert!((default_pivot_local(&m, &sel) - Vec3::new(0.0, 4.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn dab_spacing_is_a_quarter_of_the_radius() {
        assert!(should_dab(None, Vec3::ZERO, 1.0));
        assert!(!should_dab(Some(Vec3::ZERO), Vec3::new(0.2, 0.0, 0.0), 1.0));
        assert!(should_dab(Some(Vec3::ZERO), Vec3::new(0.25, 0.0, 0.0), 1.0));
    }

    #[test]
    fn paint_dab_respects_radius_falloff_and_blend() {
        let mut m = primitives::uv_sphere(1.0, 10, 16);
        let center = m.co(3);
        let brush = Brush {
            channel: VertexChannel::AmbientOcclusion,
            value: 0.0,
            radius: 0.5,
            strength: 1.0,
            falloff: BrushFalloff::Linear,
            blend: PaintBlend::Mix,
        };
        let n = paint_dab(&mut m, center, &brush).unwrap();
        assert!(n > 1);
        let r = VertexChannel::AmbientOcclusion.index();
        assert_eq!(m.verts[3].color[r], 0.0, "no centro o Mix com força 1 chega ao valor");
        for (i, v) in m.verts.iter().enumerate() {
            let d = v.co.distance(center);
            if d > 0.5 {
                assert_eq!(v.color[r], 1.0, "vértice {i} fora do raio intacto");
            } else {
                // Mix: 1 + (0 - 1) * w, com w = 1 - d/r (linear)
                assert!((v.color[r] - d / 0.5).abs() < 1e-4, "vértice {i}");
            }
            // os outros canais não mudam
            assert_eq!(v.color[1], 0.5);
        }
        // Subtract não sai de [0,1]
        let sub = Brush { blend: PaintBlend::Subtract, value: 1.0, ..brush };
        paint_dab(&mut m, center, &sub).unwrap();
        assert!(m.verts.iter().all(|v| (0.0..=1.0).contains(&v.color[r])));
        // raio inválido é erro e nada muda
        let snapshot = m.clone();
        assert!(paint_dab(&mut m, center, &Brush { radius: 0.0, ..brush }).is_err());
        assert_eq!(m, snapshot);
    }

    #[test]
    fn a_whole_paint_stroke_is_one_undo_step() {
        let mut s = edit_session(PrimitiveKind::UvSphere);
        let before = s.doc.objects[0].mesh.clone();
        let labels_before = s.history_labels().len() + 1;
        s.checkpoint("Pintar atributo");
        let brush = Brush {
            channel: VertexChannel::SpecRimMask,
            value: 0.0,
            radius: 0.6,
            strength: 1.0,
            falloff: BrushFalloff::Smooth,
            blend: PaintBlend::Mix,
        };
        for i in 0..5 {
            let p = Vec3::new(-0.8 + 0.4 * i as f32, 0.3, 0.7).normalize();
            s.mutate_active_mesh(|m| paint_dab(m, p, &brush)).unwrap().unwrap();
        }
        assert_ne!(s.doc.objects[0].mesh, before);
        assert!(s.undo().is_some());
        assert_eq!(s.doc.objects[0].mesh, before, "um único undo desfaz o traço inteiro");
        assert_eq!(s.history_labels().len(), labels_before - 1);
    }

    #[test]
    fn view_state_maps_to_workbench_options_and_cache_hash() {
        let mut ui = NormalsUi::default();
        assert!(ui.overlay().is_none() && ui.channel_view().is_none());
        let h0 = ui.view_hash();
        ui.show_normals = true;
        ui.normal_length = 0.4;
        assert_eq!(ui.overlay(), Some(NormalOverlay { length: 0.4, selected_only: false }));
        assert_ne!(ui.view_hash(), h0);
        let h1 = ui.view_hash();
        ui.view_channel = true;
        ui.channel = VertexChannel::OutlineWidth;
        assert_eq!(ui.channel_view(), Some(VertexChannel::OutlineWidth));
        assert_ne!(ui.view_hash(), h1);
        let h2 = ui.view_hash();
        ui.channel = VertexChannel::SpecRimMask;
        assert_ne!(ui.view_hash(), h2, "trocar de canal invalida o cache");
    }

    #[test]
    fn bake_and_clear_custom_normals_roundtrip() {
        let mut s = edit_session(PrimitiveKind::UvSphere);
        let params = s.doc.objects[0].normal_params();
        let before = s.doc.objects[0].mesh.clone();
        s.edit_mesh("bake", |m, _| {
            normals::bake_custom_normals(m, &params);
            Ok(())
        })
        .unwrap();
        let (custom, total) = custom_normal_counts(&s.doc.objects[0].mesh);
        assert_eq!(custom, total);
        s.edit_mesh("clear", |m, _| {
            normals::clear_custom_normals(m);
            Ok(())
        })
        .unwrap();
        assert_eq!(custom_normal_counts(&s.doc.objects[0].mesh).0, 0);
        assert_eq!(s.doc.objects[0].mesh, before, "assar + limpar volta ao original");
    }
}
