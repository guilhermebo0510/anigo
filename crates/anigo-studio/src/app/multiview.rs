//! Multi-vista do viewport (estilo Blender): 1, 2 (lado a lado ou empilhadas) ou 4 vistas, cada uma com a sua câmera —
//! perspectiva, ortogonal livre ou ortogonal alinhada (frente/costas/direita/esquerda/topo/base).
//!
//! Modelo: cada vista é um *slot* (0..4) com a sua [`OrbitCamera`]. A câmera do slot **ativo** vive em `StudioApp::camera`
//! (todo o código de edição, atalhos e botões continua usando ela); as dos demais ficam em [`MultiView::cams`] e são
//! trocadas para dentro de `StudioApp::camera` só enquanto a vista correspondente é desenhada. A vista ativa é a que está
//! sob o ponteiro (como no Blender) e só ela recebe entrada; as outras apenas desenham.
//!
//! Este módulo é só dados e geometria (sem egui além de `Rect`); o desenho e a entrada ficam em `viewport_ui`.

use crate::viewport::{OrbitCamera, ViewPreset};
use egui::{pos2, Rect, Sense, Stroke};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_VIEWS: usize = crate::workbench::MAX_VIEW_SLOTS;
/// Espaço entre vistas vizinhas (pontos lógicos); aparece como a linha divisória.
const GAP: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewLayout {
    /// Uma vista (a ativa) ocupa toda a área.
    #[default]
    Single,
    /// Duas vistas lado a lado.
    SplitV,
    /// Duas vistas empilhadas.
    SplitH,
    /// Quatro vistas (2×2).
    Quad,
}

impl ViewLayout {
    pub(crate) const ALL: [ViewLayout; 4] = [ViewLayout::Single, ViewLayout::SplitV, ViewLayout::SplitH, ViewLayout::Quad];

    pub(crate) fn count(self) -> usize {
        match self {
            ViewLayout::Single => 1,
            ViewLayout::SplitV | ViewLayout::SplitH => 2,
            ViewLayout::Quad => 4,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            ViewLayout::Single => "single",
            ViewLayout::SplitV => "split_v",
            ViewLayout::SplitH => "split_h",
            ViewLayout::Quad => "quad",
        }
    }

    pub(crate) fn from_key(k: &str) -> Option<ViewLayout> {
        Self::ALL.into_iter().find(|l| l.key() == k)
    }

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            ViewLayout::Single => "mview.layout.single",
            ViewLayout::SplitV => "mview.layout.split_v",
            ViewLayout::SplitH => "mview.layout.split_h",
            ViewLayout::Quad => "mview.layout.quad",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            ViewLayout::Single => crate::icons::SQUARE,
            ViewLayout::SplitV => crate::icons::COLUMNS,
            ViewLayout::SplitH => crate::icons::ROWS,
            ViewLayout::Quad => crate::icons::SQUARES_FOUR,
        }
    }

    /// Divide `area` nos retângulos das vistas, na ordem dos slots (esq→dir, cima→baixo). `Single` devolve 1 retângulo.
    /// `ratio` = [posição da divisória vertical, da horizontal] como fração (0..1) da largura/altura (0,5 = no meio).
    pub(crate) fn split(self, area: Rect, ratio: [f32; 2]) -> Vec<Rect> {
        let (l, t, r, b) = (area.left(), area.top(), area.right(), area.bottom());
        let (mx, my) = (area.left() + area.width() * ratio[0], area.top() + area.height() * ratio[1]);
        let h = GAP * 0.5;
        let rc = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        match self {
            ViewLayout::Single => vec![area],
            ViewLayout::SplitV => vec![rc(l, t, mx - h, b), rc(mx + h, t, r, b)],
            ViewLayout::SplitH => vec![rc(l, t, r, my - h), rc(l, my + h, r, b)],
            ViewLayout::Quad => vec![rc(l, t, mx - h, my - h), rc(mx + h, t, r, my - h), rc(l, my + h, mx - h, b), rc(mx + h, my + h, r, b)],
        }
    }
}

/// Tipo de uma vista, derivado da câmera (não é estado separado: o que se vê é o que a câmera diz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewKind {
    Perspective,
    /// Ortogonal livre (câmera orbitada com projeção ortogonal).
    Ortho,
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

impl ViewKind {
    pub(crate) const ALL: [ViewKind; 8] = [
        ViewKind::Perspective,
        ViewKind::Ortho,
        ViewKind::Front,
        ViewKind::Back,
        ViewKind::Right,
        ViewKind::Left,
        ViewKind::Top,
        ViewKind::Bottom,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            ViewKind::Perspective => "perspective",
            ViewKind::Ortho => "ortho",
            ViewKind::Front => "front",
            ViewKind::Back => "back",
            ViewKind::Right => "right",
            ViewKind::Left => "left",
            ViewKind::Top => "top",
            ViewKind::Bottom => "bottom",
        }
    }

    pub(crate) fn from_key(k: &str) -> Option<ViewKind> {
        Self::ALL.into_iter().find(|v| v.key() == k)
    }

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            ViewKind::Perspective => "mview.kind.perspective",
            ViewKind::Ortho => "mview.kind.ortho",
            ViewKind::Front => "mview.kind.front",
            ViewKind::Back => "mview.kind.back",
            ViewKind::Right => "mview.kind.right",
            ViewKind::Left => "mview.kind.left",
            ViewKind::Top => "mview.kind.top",
            ViewKind::Bottom => "mview.kind.bottom",
        }
    }

    /// Que tipo é esta câmera agora?
    pub(crate) fn of(cam: &OrbitCamera) -> ViewKind {
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        if !cam.orthographic {
            return ViewKind::Perspective;
        }
        const EPS: f32 = 0.02;
        let yaw = cam.yaw.rem_euclid(TAU);
        let near = |a: f32, b: f32| (a - b).abs() < EPS || (a - b).abs() > TAU - EPS;
        if cam.pitch > 1.5 {
            return ViewKind::Top;
        }
        if cam.pitch < -1.5 {
            return ViewKind::Bottom;
        }
        if cam.pitch.abs() < EPS {
            if near(yaw, 0.0) {
                return ViewKind::Front;
            }
            if near(yaw, PI) {
                return ViewKind::Back;
            }
            if near(yaw, FRAC_PI_2) {
                return ViewKind::Right;
            }
            if near(yaw, TAU - FRAC_PI_2) {
                return ViewKind::Left;
            }
        }
        ViewKind::Ortho
    }

    /// Aplica o tipo à câmera (mantém alvo e distância: a vista continua olhando para o mesmo lugar).
    pub(crate) fn apply(self, cam: &mut OrbitCamera) {
        let preset = match self {
            ViewKind::Perspective => {
                cam.orthographic = false;
                return;
            }
            ViewKind::Ortho => {
                cam.orthographic = true;
                return;
            }
            ViewKind::Front => ViewPreset::Front,
            ViewKind::Back => ViewPreset::Back,
            ViewKind::Right => ViewPreset::Right,
            ViewKind::Left => ViewPreset::Left,
            ViewKind::Top => ViewPreset::Top,
            ViewKind::Bottom => ViewPreset::Bottom,
        };
        cam.orthographic = true;
        cam.set_preset(preset);
    }

    /// Tipo inicial de cada slot quando o layout multi-vista é aberto pela primeira vez (slot 0 é a câmera atual).
    pub(crate) fn default_for_slot(slot: usize) -> ViewKind {
        match slot {
            1 => ViewKind::Front,
            2 => ViewKind::Right,
            3 => ViewKind::Top,
            _ => ViewKind::Perspective,
        }
    }
}

pub(crate) struct MultiView {
    pub layout: ViewLayout,
    /// Layout multi-vista para onde Ctrl+Espaço volta ao "restaurar" (nunca `Single`).
    pub restore: ViewLayout,
    /// Câmeras por slot. A do slot ativo está DESATUALIZADA: a verdadeira é `StudioApp::camera`.
    pub cams: [OrbitCamera; MAX_VIEWS],
    /// Slot que já recebeu a câmera inicial (os demais são "semeados" a partir da câmera ativa ao aparecer).
    pub seeded: [bool; MAX_VIEWS],
    pub active: usize,
    /// Retângulos do último quadro, por slot (para achar a vista sob o ponteiro).
    pub rects: [Rect; MAX_VIEWS],
    /// Posição das divisórias (vertical, horizontal) como fração da área; arrastáveis, duplo clique volta a 0,5.
    pub ratio: [f32; 2],
}

impl Default for MultiView {
    fn default() -> Self {
        Self {
            layout: ViewLayout::Single,
            restore: ViewLayout::Quad,
            cams: [OrbitCamera::default(); MAX_VIEWS],
            seeded: [true, false, false, false],
            active: 0,
            rects: [Rect::NOTHING; MAX_VIEWS],
            ratio: [0.5, 0.5],
        }
    }
}

impl MultiView {
    /// Slots desenhados agora. `Single` mostra o slot ativo (a câmera que o usuário está vendo).
    pub(crate) fn visible(&self) -> Vec<usize> {
        match self.layout {
            ViewLayout::Single => vec![self.active],
            l => (0..l.count()).collect(),
        }
    }

    /// Slot cujo retângulo (do último quadro) contém `pos`.
    pub(crate) fn slot_at(&self, pos: egui::Pos2) -> Option<usize> {
        self.visible().into_iter().find(|&s| self.rects[s].contains(pos))
    }

    /// Alterna "maximizar a vista ativa" (Ctrl+Espaço): `Single` ↔ último layout multi-vista.
    pub(crate) fn toggle_maximize(&mut self, active_cam: &mut OrbitCamera) {
        if self.layout == ViewLayout::Single {
            self.set_layout(active_cam, self.restore);
        } else {
            self.set_layout(active_cam, ViewLayout::Single);
        }
    }

    /// Muda o layout. Se a vista ativa não existe no novo layout (ex.: slot 3 → duas vistas), a ativa passa a ser a 0.
    pub(crate) fn set_layout(&mut self, active_cam: &mut OrbitCamera, layout: ViewLayout) {
        if self.layout != ViewLayout::Single && layout == ViewLayout::Single {
            // "restaurar" volta para o layout multi-vista de onde se saiu
            self.restore = self.layout;
        }
        if layout != ViewLayout::Single {
            self.restore = layout;
        }
        self.layout = layout;
        if layout != ViewLayout::Single && self.active >= layout.count() {
            self.set_active(active_cam, 0);
        }
    }

    /// Dá a câmera inicial aos slots que acabaram de aparecer (a partir da câmera ativa: mesmo alvo e distância).
    pub(crate) fn seed(&mut self, active_cam: &OrbitCamera) {
        for s in self.visible() {
            if !self.seeded[s] {
                let mut cam = *active_cam;
                let kind = ViewKind::default_for_slot(s);
                if matches!(kind, ViewKind::Top | ViewKind::Bottom) {
                    // o topo/base alinhado não herda a rotação da câmera em perspectiva
                    cam.yaw = 0.0;
                }
                kind.apply(&mut cam);
                self.cams[s] = cam;
                self.seeded[s] = true;
            }
        }
    }

    /// Passa a vista ativa para `slot`: a câmera em uso volta para o seu slot e a do novo slot assume `active_cam`.
    pub(crate) fn set_active(&mut self, active_cam: &mut OrbitCamera, slot: usize) {
        let slot = slot.min(MAX_VIEWS - 1);
        if slot == self.active {
            return;
        }
        self.cams[self.active] = *active_cam;
        *active_cam = self.cams[slot];
        self.active = slot;
    }

    /// Torna ativa a vista sob o ponteiro (como no Blender). `busy` = há um gesto/modal em curso que deve manter a vista.
    /// Arrastar da Biblioteca (DnD) NÃO bloqueia: soltar numa vista usa a câmera dela.
    pub(crate) fn follow_pointer(&mut self, active_cam: &mut OrbitCamera, ctx: &egui::Context, busy: bool) {
        if self.layout == ViewLayout::Single || busy || ctx.any_popup_open() {
            return;
        }
        if ctx.input(|i| i.pointer.any_down()) && !egui::DragAndDrop::has_any_payload(ctx) {
            return;
        }
        if let Some(slot) = ctx.input(|i| i.pointer.hover_pos()).and_then(|p| self.slot_at(p)) {
            self.set_active(active_cam, slot);
        }
    }

    /// Retângulos das vistas visíveis dentro de `area` (com as divisórias nas posições atuais).
    pub(crate) fn pane_rects(&self, area: Rect) -> Vec<Rect> {
        self.layout.split(area, self.ratio)
    }

    /// Divisórias arrastáveis entre as vistas: arrastar move; duplo clique volta ao meio; no 2×2 o ponto central move as
    /// duas. Chame DEPOIS de desenhar as vistas (fica por cima das bordas).
    pub(crate) fn dividers_ui(&mut self, ui: &mut egui::Ui, area: Rect) {
        const HIT: f32 = 8.0;
        let layout = self.layout;
        if layout == ViewLayout::Single || area.width() < 1.0 || area.height() < 1.0 {
            return;
        }
        let vertical = matches!(layout, ViewLayout::SplitV | ViewLayout::Quad);
        let horizontal = matches!(layout, ViewLayout::SplitH | ViewLayout::Quad);
        let x = area.left() + area.width() * self.ratio[0];
        let y = area.top() + area.height() * self.ratio[1];
        let id = ui.id().with("mview_dividers");
        let line = |on: bool| Stroke::new(if on { 2.5 } else { 1.0 }, if on { crate::theme::ACCENT } else { crate::theme::BORDER });
        let mut new = self.ratio;
        let mut cursor = None;
        if vertical {
            let r = Rect::from_min_max(pos2(x - HIT * 0.5, area.top()), pos2(x + HIT * 0.5, area.bottom()));
            let resp = ui.interact(r, id.with("v"), Sense::click_and_drag());
            let on = resp.hovered() || resp.dragged();
            if on {
                cursor = Some(egui::CursorIcon::ResizeHorizontal);
            }
            ui.painter().vline(x, area.y_range(), line(on));
            if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged()) {
                new[0] = fraction(p.x, area.left(), area.width());
            }
            if resp.double_clicked() {
                new[0] = 0.5;
            }
        }
        if horizontal {
            let r = Rect::from_min_max(pos2(area.left(), y - HIT * 0.5), pos2(area.right(), y + HIT * 0.5));
            let resp = ui.interact(r, id.with("h"), Sense::click_and_drag());
            let on = resp.hovered() || resp.dragged();
            if on {
                cursor = Some(egui::CursorIcon::ResizeVertical);
            }
            ui.painter().hline(area.x_range(), y, line(on));
            if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged()) {
                new[1] = fraction(p.y, area.top(), area.height());
            }
            if resp.double_clicked() {
                new[1] = 0.5;
            }
        }
        if layout == ViewLayout::Quad {
            // o cruzamento move as duas divisórias de uma vez (registrado por último: ganha sobre as duas faixas)
            let r = Rect::from_center_size(pos2(x, y), egui::vec2(HIT * 2.5, HIT * 2.5));
            let resp = ui.interact(r, id.with("c"), Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                cursor = Some(egui::CursorIcon::Move);
                ui.painter().circle_filled(pos2(x, y), 4.0, crate::theme::ACCENT);
            }
            if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged()) {
                new[0] = fraction(p.x, area.left(), area.width());
                new[1] = fraction(p.y, area.top(), area.height());
            }
            if resp.double_clicked() {
                new = [0.5, 0.5];
            }
        }
        if let Some(c) = cursor {
            ui.ctx().set_cursor_icon(c);
        }
        self.ratio = new;
    }

    /// Estado persistido na sessão. `active_cam` é a câmera viva da vista ativa (a do slot está desatualizada).
    pub(crate) fn to_state(&self, active_cam: &OrbitCamera) -> MultiViewState {
        let mut cams = self.cams;
        cams[self.active] = *active_cam;
        MultiViewState { layout: self.layout, restore: self.restore, active: self.active, cams, seeded: self.seeded, ratio: self.ratio }
    }

    /// Restaura de uma sessão (valida tudo: uma sessão adulterada nunca derruba o app). Devolve a câmera da vista ativa,
    /// que quem chama põe na câmera viva do workspace.
    pub(crate) fn from_state(st: &MultiViewState) -> (MultiView, OrbitCamera) {
        let mut mv = MultiView::default();
        mv.layout = st.layout;
        mv.restore = if st.restore == ViewLayout::Single { ViewLayout::Quad } else { st.restore };
        mv.active = st.active.min(MAX_VIEWS - 1);
        if mv.layout != ViewLayout::Single && mv.active >= mv.layout.count() {
            mv.active = 0;
        }
        mv.cams = st.cams;
        mv.seeded = st.seeded;
        let clamp = |v: f32| if v.is_finite() { v.clamp(MIN_RATIO, 1.0 - MIN_RATIO) } else { 0.5 };
        mv.ratio = [clamp(st.ratio[0]), clamp(st.ratio[1])];
        let cam = mv.cams[mv.active];
        (mv, cam)
    }

    /// Câmera do slot (a ativa vive fora, em `active_cam`).
    pub(crate) fn camera_mut<'a>(&'a mut self, active_cam: &'a mut OrbitCamera, slot: usize) -> &'a mut OrbitCamera {
        if slot == self.active {
            active_cam
        } else {
            &mut self.cams[slot.min(MAX_VIEWS - 1)]
        }
    }
}

/// Fração mínima/máxima de uma divisória (nenhuma vista fica menor que isso).
const MIN_RATIO: f32 = 0.15;

fn fraction(p: f32, origin: f32, len: f32) -> f32 {
    ((p - origin) / len.max(1.0)).clamp(MIN_RATIO, 1.0 - MIN_RATIO)
}

/// Multi-vista de um workspace como vai para a sessão (JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiViewState {
    pub layout: ViewLayout,
    pub restore: ViewLayout,
    pub active: usize,
    pub cams: [OrbitCamera; MAX_VIEWS],
    pub seeded: [bool; MAX_VIEWS],
    pub ratio: [f32; 2],
}

/// Botões 1 / 2 lado a lado / 2 empilhadas / 4 vistas no cabeçalho do viewport (para usar dentro de um layout `right_to_left`).
pub(crate) fn layout_switcher(ui: &mut egui::Ui, mv: &mut MultiView, active_cam: &mut OrbitCamera) {
    layout_buttons(ui, mv, active_cam, true);
}

/// Os mesmos botões; `right_to_left` = o contêiner desenha da direita para a esquerda (então inverte a ordem para ler 1·2·2·4).
pub(crate) fn layout_buttons(ui: &mut egui::Ui, mv: &mut MultiView, active_cam: &mut OrbitCamera, right_to_left: bool) {
    use crate::i18n::tr;
    let mut pick: Option<ViewLayout> = None;
    let order: Vec<ViewLayout> = if right_to_left { ViewLayout::ALL.into_iter().rev().collect() } else { ViewLayout::ALL.to_vec() };
    for l in order {
        if ui.selectable_label(mv.layout == l, egui::RichText::new(l.icon()).size(15.0)).on_hover_text(tr(l.label_key())).clicked() {
            pick = Some(l);
        }
    }
    if let Some(l) = pick {
        mv.set_layout(active_cam, l);
    }
}

/// Seletor do tipo da vista (perspectiva/ortogonal/frente/…) numa pequena área `area`, seguido de `suffix` (ex.: nome do
/// objeto). Devolve o tipo escolhido; quem chama aplica (cada workspace tem o seu jeito de enquadrar).
pub(crate) fn view_kind_menu(ui: &mut egui::Ui, area: Rect, cam: &OrbitCamera, suffix: &str) -> Option<ViewKind> {
    use crate::i18n::tr;
    let kind = ViewKind::of(cam);
    let mut pick = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(4.0, 1.0);
            let label = egui::RichText::new(format!("{}  {}", tr(kind.label_key()), crate::icons::CARET_DOWN)).size(12.0).color(egui::Color32::from_rgba_unmultiplied(225, 225, 235, 215));
            ui.menu_button(label, |ui| {
                for k in ViewKind::ALL {
                    if ui.selectable_label(k == kind, tr(k.label_key())).clicked() {
                        pick = Some(k);
                        ui.close();
                    }
                }
            });
            if !suffix.is_empty() {
                ui.label(egui::RichText::new(format!("|  {suffix}")).size(12.0).color(egui::Color32::from_rgba_unmultiplied(220, 220, 230, 170)));
            }
        });
    });
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0))
    }

    #[test]
    fn splits_cover_the_area_without_overlap() {
        for l in ViewLayout::ALL {
            let rs = l.split(area(), [0.5, 0.5]);
            assert_eq!(rs.len(), l.count());
            let total: f32 = rs.iter().map(|r| r.width() * r.height()).sum();
            let gaps = match l {
                ViewLayout::Single => 0.0,
                ViewLayout::SplitV => GAP * 600.0,
                ViewLayout::SplitH => GAP * 1000.0,
                ViewLayout::Quad => GAP * 600.0 + GAP * 1000.0 - GAP * GAP,
            };
            assert!((total + gaps - 600_000.0).abs() < 1.0, "{l:?}: {total} + {gaps}");
            for (i, a) in rs.iter().enumerate() {
                for b in rs.iter().skip(i + 1) {
                    assert!(!a.intersects(*b), "{l:?}: {a:?} cruza {b:?}");
                }
            }
        }
    }

    #[test]
    fn dividers_move_the_split_and_keep_a_minimum_size() {
        let rs = ViewLayout::Quad.split(area(), [0.25, 0.75]);
        assert!((rs[0].width() - (250.0 - GAP * 0.5)).abs() < 0.01);
        assert!((rs[2].height() - (150.0 - GAP * 0.5)).abs() < 0.01);
        assert_eq!(fraction(-500.0, 0.0, 1000.0), MIN_RATIO);
        assert_eq!(fraction(5000.0, 0.0, 1000.0), 1.0 - MIN_RATIO);
        assert!((fraction(300.0, 0.0, 1000.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn state_roundtrips_through_json_and_rejects_nonsense() {
        let mut mv = MultiView { layout: ViewLayout::Quad, active: 2, ratio: [0.3, 0.6], ..Default::default() };
        mv.cams[1] = OrbitCamera { yaw: 1.25, orthographic: true, ..Default::default() };
        mv.seeded = [true, true, true, true];
        let live = OrbitCamera { yaw: -0.5, ..Default::default() };
        let json = serde_json::to_string(&mv.to_state(&live)).unwrap();
        let (back, cam) = MultiView::from_state(&serde_json::from_str(&json).unwrap());
        assert_eq!((back.layout, back.active, back.ratio), (ViewLayout::Quad, 2, [0.3, 0.6]));
        assert_eq!(cam.yaw, -0.5, "a câmera da vista ativa volta como a câmera viva");
        assert_eq!(back.cams[1].yaw, 1.25);
        // proporções absurdas e vista ativa fora do layout são corrigidas
        let mut st = mv.to_state(&live);
        st.ratio = [f32::NAN, 7.0];
        st.layout = ViewLayout::SplitV;
        st.active = 3;
        let (fixed, _) = MultiView::from_state(&st);
        assert_eq!(fixed.ratio, [0.5, 1.0 - MIN_RATIO]);
        assert_eq!(fixed.active, 0);
    }

    #[test]
    fn layout_and_kind_keys_roundtrip() {
        for l in ViewLayout::ALL {
            assert_eq!(ViewLayout::from_key(l.key()), Some(l));
        }
        for k in ViewKind::ALL {
            assert_eq!(ViewKind::from_key(k.key()), Some(k));
        }
    }

    #[test]
    fn applying_a_kind_is_recognised_back() {
        for k in ViewKind::ALL {
            let mut cam = OrbitCamera { yaw: 0.6, pitch: 0.35, ..Default::default() };
            k.apply(&mut cam);
            // Ortho livre mantém os ângulos arbitrários; os alinhados voltam como eles mesmos
            assert_eq!(ViewKind::of(&cam), k, "{k:?}: yaw {} pitch {}", cam.yaw, cam.pitch);
        }
    }

    #[test]
    fn orbiting_an_aligned_view_turns_it_into_free_ortho() {
        let mut cam = OrbitCamera::default();
        ViewKind::Front.apply(&mut cam);
        cam.orbit(30.0, 10.0);
        assert_eq!(ViewKind::of(&cam), ViewKind::Ortho);
    }

    #[test]
    fn single_shows_the_active_slot_and_maximize_toggles() {
        let mut cam = OrbitCamera::default();
        let mut mv = MultiView { active: 2, ..Default::default() };
        assert_eq!(mv.visible(), vec![2]);
        mv.set_layout(&mut cam, ViewLayout::Quad);
        assert_eq!(mv.visible(), vec![0, 1, 2, 3]);
        mv.toggle_maximize(&mut cam);
        assert_eq!(mv.layout, ViewLayout::Single);
        mv.toggle_maximize(&mut cam);
        assert_eq!(mv.layout, ViewLayout::Quad);
    }

    #[test]
    fn a_hidden_active_view_falls_back_to_slot_zero_and_keeps_the_cameras() {
        let mut cam = OrbitCamera { yaw: 1.0, ..Default::default() };
        let mut mv = MultiView { layout: ViewLayout::Quad, active: 3, ..Default::default() };
        mv.cams[0] = OrbitCamera { yaw: 2.0, ..Default::default() };
        mv.set_layout(&mut cam, ViewLayout::SplitV);
        assert_eq!(mv.active, 0);
        assert_eq!(cam.yaw, 2.0, "a câmera do slot 0 assume o lugar da ativa");
        assert_eq!(mv.cams[3].yaw, 1.0, "a câmera que estava em uso volta para o slot 3");
    }

    #[test]
    fn slot_at_uses_the_last_frame_rects() {
        let mut mv = MultiView { layout: ViewLayout::Quad, ..Default::default() };
        for (i, r) in ViewLayout::Quad.split(area(), [0.5, 0.5]).into_iter().enumerate() {
            mv.rects[i] = r;
        }
        assert_eq!(mv.slot_at(pos2(10.0, 10.0)), Some(0));
        assert_eq!(mv.slot_at(pos2(990.0, 10.0)), Some(1));
        assert_eq!(mv.slot_at(pos2(10.0, 590.0)), Some(2));
        assert_eq!(mv.slot_at(pos2(990.0, 590.0)), Some(3));
        assert_eq!(mv.slot_at(pos2(2000.0, 10.0)), None);
    }
}

// ------------------------------------------------------------------------------------------------ por workspace

use super::StudioApp;
use crate::workspace::WorkspaceId;
use std::collections::HashMap;

/// Chave do workspace na sessão. Posing e Animação dividem o mesmo viewport (e portanto as mesmas vistas).
fn session_key(ws: WorkspaceId) -> Option<&'static str> {
    match ws {
        WorkspaceId::Modelagem => Some("modelagem"),
        WorkspaceId::Cenario => Some("cenario"),
        WorkspaceId::Posing | WorkspaceId::Animacao => Some("posing"),
        WorkspaceId::Iluminacao => Some("iluminacao"),
        _ => None,
    }
}

/// Workspaces com multi-vista (um por viewport 3D próprio).
pub(crate) const VIEW_WORKSPACES: [WorkspaceId; 4] = [WorkspaceId::Modelagem, WorkspaceId::Cenario, WorkspaceId::Posing, WorkspaceId::Iluminacao];

impl StudioApp {
    /// Vistas e câmera ativa do workspace; `None` nos que não têm viewport 3D editável (Render, Shading, Biblioteca…).
    pub(crate) fn view_parts_mut(&mut self, ws: WorkspaceId) -> Option<(&mut MultiView, &mut OrbitCamera)> {
        match ws {
            WorkspaceId::Modelagem => Some((&mut self.mview, &mut self.camera)),
            WorkspaceId::Cenario => Some((&mut self.scene_ui.mview, &mut self.scene_ui.camera)),
            WorkspaceId::Posing | WorkspaceId::Animacao => Some((&mut self.pose_ui.mview, &mut self.pose_ui.camera)),
            WorkspaceId::Iluminacao => Some((&mut self.light_ui.mview, &mut self.light_ui.camera)),
            _ => None,
        }
    }

    pub(crate) fn view_parts(&self, ws: WorkspaceId) -> Option<(&MultiView, &OrbitCamera)> {
        match ws {
            WorkspaceId::Modelagem => Some((&self.mview, &self.camera)),
            WorkspaceId::Cenario => Some((&self.scene_ui.mview, &self.scene_ui.camera)),
            WorkspaceId::Posing | WorkspaceId::Animacao => Some((&self.pose_ui.mview, &self.pose_ui.camera)),
            WorkspaceId::Iluminacao => Some((&self.light_ui.mview, &self.light_ui.camera)),
            _ => None,
        }
    }

    /// Vistas de todos os workspaces, para a sessão.
    pub(crate) fn multiview_states(&self) -> HashMap<String, MultiViewState> {
        VIEW_WORKSPACES
            .into_iter()
            .filter_map(|ws| {
                let (mv, cam) = self.view_parts(ws)?;
                Some((session_key(ws)?.to_string(), mv.to_state(cam)))
            })
            .collect()
    }

    /// Restaura as vistas salvas (chaves desconhecidas são ignoradas; o que está inválido é corrigido em `from_state`).
    pub(crate) fn restore_multiviews(&mut self, saved: &HashMap<String, MultiViewState>) {
        for ws in VIEW_WORKSPACES {
            let Some(st) = session_key(ws).and_then(|k| saved.get(k)) else { continue };
            let (mv, cam) = MultiView::from_state(st);
            if let Some((slot_mv, slot_cam)) = self.view_parts_mut(ws) {
                *slot_mv = mv;
                *slot_cam = cam;
            }
        }
    }
}
