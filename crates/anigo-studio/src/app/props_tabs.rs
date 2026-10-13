//! Barra de abas verticais do painel Propriedades (estilo do editor de Propriedades do Blender): uma faixa de ícones
//! encostada na borda esquerda do painel, entre a viewport e o conteúdo. Cada ícone abre UMA categoria, então o
//! painel deixa de ser uma lista longa de seções empilhadas.
//!
//! Este módulo só sabe desenhar a faixa e quais categorias existem em cada modo da Modelagem; o conteúdo de cada
//! categoria continua nos módulos de cada modo (`inspector`, `sculpt_ui`, `hair_ui`), que recebem a aba atual.

use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::modeling::Mode;
use crate::theme;
use egui::{Rect, Sense, Stroke};

/// Largura da faixa de ícones (pontos lógicos).
pub(crate) const STRIP_W: f32 = 34.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PropTab {
    /// Modo Objeto: cena, primitivas, objeto ativo e exportação.
    Object,
    Modifiers,
    /// Modo Edição: seleção, edição proporcional e operadores.
    Tool,
    /// Normais e UV.
    Data,
    /// Snap e overlays.
    View,
    /// Escultura: opções do pincel ativo, falloff, auto-máscara.
    Brush,
    Symmetry,
    Mesh,
    /// Cabelo: alvo, grupos e mecha.
    Hair,
    Drawing,
    Physics,
    Output,
    History,
    Help,
    // ---- Personagem, Iluminação, Animação, Shading
    /// Salvar/abrir/resetar o projeto.
    Project,
    /// Rigs de luz (predefinições).
    Rigs,
    /// Sol e luz ambiente.
    Sun,
    /// Sombras da luz e rim.
    Shadows,
    /// Luzes extras e teste.
    Advanced,
    /// Clipe de animação.
    Clip,
    /// Osso selecionado (estrutura óssea).
    Bone,
    /// Material: base, sombra e rampa.
    Shade,
    /// Material: especular, matcap e emissão.
    Highlight,
    /// Material: rim e contorno.
    Edge,
    /// Material: rosto e olhos.
    Face,
    /// Texturas do material.
    Textures,
    // ---- Cenário (Detalhes)
    Transform,
    /// Ativo da Biblioteca (origem, revisão, substituir).
    Asset,
    /// Lente da câmera de cena.
    Lens,
    Material,
    /// Pastas, tags e hierarquia.
    Organize,
    /// Configurações da cena (luz principal, câmera de render, unidade).
    Scene,
}

impl PropTab {
    pub(crate) fn key(self) -> &'static str {
        match self {
            PropTab::Object => "object",
            PropTab::Modifiers => "modifiers",
            PropTab::Tool => "tool",
            PropTab::Data => "data",
            PropTab::View => "view",
            PropTab::Brush => "brush",
            PropTab::Symmetry => "symmetry",
            PropTab::Mesh => "mesh",
            PropTab::Hair => "hair",
            PropTab::Drawing => "drawing",
            PropTab::Physics => "physics",
            PropTab::Output => "output",
            PropTab::History => "history",
            PropTab::Help => "help",
            PropTab::Project => "project",
            PropTab::Rigs => "rigs",
            PropTab::Sun => "sun",
            PropTab::Shadows => "shadows",
            PropTab::Advanced => "advanced",
            PropTab::Clip => "clip",
            PropTab::Bone => "bone",
            PropTab::Shade => "shade",
            PropTab::Highlight => "highlight",
            PropTab::Edge => "edge",
            PropTab::Face => "face",
            PropTab::Textures => "textures",
            PropTab::Transform => "transform",
            PropTab::Asset => "asset",
            PropTab::Lens => "lens",
            PropTab::Material => "material",
            PropTab::Organize => "organize",
            PropTab::Scene => "scene",
        }
    }

    pub(crate) fn from_key(k: &str) -> Option<PropTab> {
        ALL.iter().copied().find(|t| t.key() == k)
    }

    fn icon(self) -> &'static str {
        match self {
            PropTab::Object => icons::CUBE,
            PropTab::Modifiers => icons::WRENCH,
            PropTab::Tool => icons::SLIDERS_HORIZONTAL,
            PropTab::Data => icons::TREE_STRUCTURE,
            PropTab::View => icons::EYE,
            PropTab::Brush => icons::PAINT_BRUSH,
            PropTab::Symmetry => icons::FLIP_HORIZONTAL,
            PropTab::Mesh => icons::GRID_FOUR,
            PropTab::Hair => icons::WAVES,
            PropTab::Drawing => icons::PEN_NIB,
            PropTab::Physics => icons::BONE,
            PropTab::Output => icons::EXPORT,
            PropTab::History => icons::CLOCK_COUNTER_CLOCKWISE,
            PropTab::Help => icons::KEYBOARD,
            PropTab::Project => icons::FLOPPY_DISK,
            PropTab::Rigs => icons::LIGHTBULB,
            PropTab::Sun => icons::SUN,
            PropTab::Shadows => icons::CIRCLE_HALF,
            PropTab::Advanced => icons::FLASHLIGHT,
            PropTab::Clip => icons::FILM_STRIP,
            PropTab::Bone => icons::BONE,
            PropTab::Shade => icons::CIRCLE_HALF,
            PropTab::Highlight => icons::SPARKLE,
            PropTab::Edge => icons::PEN_NIB,
            PropTab::Face => icons::SMILEY,
            PropTab::Textures => icons::IMAGE,
            PropTab::Transform => icons::ARROWS_OUT_CARDINAL,
            PropTab::Asset => icons::CUBE,
            PropTab::Lens => icons::VIDEO_CAMERA,
            PropTab::Material => icons::PALETTE,
            PropTab::Organize => icons::FOLDER_SIMPLE,
            PropTab::Scene => icons::GLOBE_SIMPLE,
        }
    }

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            PropTab::Object => "ptab.object",
            PropTab::Modifiers => "ptab.modifiers",
            PropTab::Tool => "ptab.tool",
            PropTab::Data => "ptab.data",
            PropTab::View => "ptab.view",
            PropTab::Brush => "ptab.brush",
            PropTab::Symmetry => "ptab.symmetry",
            PropTab::Mesh => "ptab.mesh",
            PropTab::Hair => "ptab.hair",
            PropTab::Drawing => "ptab.drawing",
            PropTab::Physics => "ptab.physics",
            PropTab::Output => "ptab.output",
            PropTab::History => "ptab.history",
            PropTab::Help => "ptab.help",
            PropTab::Project => "ptab.project",
            PropTab::Rigs => "ptab.rigs",
            PropTab::Sun => "ptab.sun",
            PropTab::Shadows => "ptab.shadows",
            PropTab::Advanced => "ptab.advanced",
            PropTab::Clip => "ptab.clip",
            PropTab::Bone => "ptab.bone",
            PropTab::Shade => "ptab.shade",
            PropTab::Highlight => "ptab.highlight",
            PropTab::Edge => "ptab.edge",
            PropTab::Face => "ptab.face",
            PropTab::Textures => "ptab.textures",
            PropTab::Transform => "ptab.transform",
            PropTab::Asset => "ptab.asset",
            PropTab::Lens => "ptab.lens",
            PropTab::Material => "ptab.material",
            PropTab::Organize => "ptab.organize",
            PropTab::Scene => "ptab.scene",
        }
    }
}

const ALL: [PropTab; 32] = [
    PropTab::Object,
    PropTab::Modifiers,
    PropTab::Tool,
    PropTab::Data,
    PropTab::View,
    PropTab::Brush,
    PropTab::Symmetry,
    PropTab::Mesh,
    PropTab::Hair,
    PropTab::Drawing,
    PropTab::Physics,
    PropTab::Output,
    PropTab::History,
    PropTab::Help,
    PropTab::Project,
    PropTab::Rigs,
    PropTab::Sun,
    PropTab::Shadows,
    PropTab::Advanced,
    PropTab::Clip,
    PropTab::Bone,
    PropTab::Shade,
    PropTab::Highlight,
    PropTab::Edge,
    PropTab::Face,
    PropTab::Textures,
    PropTab::Transform,
    PropTab::Asset,
    PropTab::Lens,
    PropTab::Material,
    PropTab::Organize,
    PropTab::Scene,
];

/// Categorias de cada modo da Modelagem, na ordem em que aparecem na faixa.
pub(crate) fn modeling_tabs(mode: Mode) -> &'static [PropTab] {
    match mode {
        Mode::Object => &[PropTab::Object, PropTab::Modifiers, PropTab::View, PropTab::History],
        Mode::Edit => &[PropTab::Tool, PropTab::Modifiers, PropTab::Data, PropTab::View, PropTab::History],
        Mode::Sculpt => &[PropTab::Brush, PropTab::Symmetry, PropTab::Mesh, PropTab::History, PropTab::Help],
        Mode::Hair => &[PropTab::Hair, PropTab::Drawing, PropTab::Physics, PropTab::Output, PropTab::History, PropTab::Help],
    }
}

/// Desenha a faixa de ícones e devolve `(aba escolhida, Ui do conteúdo)`. O `Ui` do conteúdo ocupa o que sobra do
/// painel à direita da faixa, já com o slider adaptado à largura (rótulo e campo numérico cabem).
pub(crate) fn tab_strip(ui: &mut egui::Ui, tabs: &[PropTab], current: PropTab) -> (PropTab, egui::Ui) {
    let body = ui.max_rect();
    let clip = ui.clip_rect();
    let strip = Rect::from_min_max(egui::pos2(clip.left(), clip.top()), egui::pos2(clip.left() + STRIP_W, clip.bottom()));
    let painter = ui.painter_at(clip);
    painter.rect_filled(strip, 0.0, theme::BG_DEEP);
    painter.vline(strip.right(), strip.y_range(), Stroke::new(1.0, theme::BORDER));

    let mut chosen = current;
    let size = egui::vec2(STRIP_W - 8.0, STRIP_W - 8.0);
    let mut y = strip.top() + 6.0;
    for &tab in tabs {
        let r = Rect::from_min_size(egui::pos2(strip.left() + 4.0, y), size);
        let resp = ui
            .interact(r, ui.id().with(("props_tab", tab.key())), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr(tab.label_key()));
        let selected = tab == current;
        if selected {
            painter.rect_filled(r, 4.0, theme::ACCENT_DIM);
            painter.rect_filled(Rect::from_min_size(egui::pos2(strip.left(), r.top() + 3.0), egui::vec2(2.0, r.height() - 6.0)), 1.0, theme::ACCENT);
        } else if resp.hovered() {
            painter.rect_filled(r, 4.0, theme::BG_HOVER);
        }
        let color = if selected { theme::TEXT } else if resp.hovered() { theme::TEXT } else { theme::TEXT_DIM };
        painter.text(r.center(), egui::Align2::CENTER_CENTER, tab.icon(), egui::FontId::proportional(17.0), color);
        if resp.clicked() {
            chosen = tab;
        }
        y += size.y + 4.0;
    }

    // o conteúdo começa depois da faixa; a margem esquerda do dock já foi "gasta" por ela
    let content = Rect::from_min_max(egui::pos2(strip.right() + 6.0, body.top()), body.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
    child.set_clip_rect(clip.intersect(Rect::from_min_max(egui::pos2(strip.right() + 1.0, clip.top()), clip.max)));
    // o rótulo do slider vem à direita e o campo numérico logo depois: reserva espaço para os dois
    let w = child.available_width();
    child.spacing_mut().slider_width = (w - 185.0).clamp(60.0, 220.0);
    (chosen, child)
}

/// Um painel Propriedades com faixa de abas. Cada um guarda a aba atual em `StudioApp::prop_tabs` (estado de apresentação).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabSet {
    Modeling(Mode),
    Personagem,
    Posing,
    Iluminacao,
    Animacao,
    Shading,
    /// Detalhes do Cenário: as abas dependem da seleção (ver `StudioApp::scene_details_tabs`).
    Cenario,
}

impl TabSet {
    /// Chave estável no mapa de abas atuais.
    fn slot(self) -> u8 {
        match self {
            TabSet::Modeling(m) => m as u8,
            TabSet::Personagem => 16,
            TabSet::Posing => 17,
            TabSet::Iluminacao => 18,
            TabSet::Animacao => 19,
            TabSet::Shading => 20,
            TabSet::Cenario => 21,
        }
    }

    pub(crate) fn tabs(self) -> &'static [PropTab] {
        match self {
            TabSet::Modeling(m) => modeling_tabs(m),
            TabSet::Personagem => &[PropTab::Tool, PropTab::Project, PropTab::History],
            TabSet::Posing => &[PropTab::Tool, PropTab::History],
            TabSet::Iluminacao => &[PropTab::Rigs, PropTab::Sun, PropTab::Shadows, PropTab::Advanced],
            TabSet::Animacao => &[PropTab::Clip, PropTab::Bone],
            TabSet::Shading => &[PropTab::Shade, PropTab::Highlight, PropTab::Edge, PropTab::Face, PropTab::Textures],
            // máximo possível; a faixa mostra só as que valem para a seleção (`tabbed_with`)
            TabSet::Cenario => &[PropTab::Transform, PropTab::Asset, PropTab::Lens, PropTab::Material, PropTab::Organize, PropTab::Scene],
        }
    }
}

impl StudioApp {
    /// Aba atual do painel (a primeira, se a guardada não existe mais no conjunto).
    pub(crate) fn prop_tab(&self, set: TabSet) -> PropTab {
        let tabs = set.tabs();
        match self.prop_tabs.get(&set.slot()) {
            Some(t) if tabs.contains(t) => *t,
            _ => tabs[0],
        }
    }

    /// Abas que o painel está mostrando (as da seleção, no Cenário); as do conjunto se ainda não desenhou.
    pub(crate) fn shown_tabs(&self, set: TabSet) -> Vec<PropTab> {
        self.tabs_shown.get(&set.slot()).cloned().unwrap_or_else(|| set.tabs().to_vec())
    }

    pub(crate) fn set_prop_tab(&mut self, set: TabSet, tab: PropTab) {
        if set.tabs().contains(&tab) {
            self.prop_tabs.insert(set.slot(), tab);
            if set == TabSet::Animacao {
                // a aba do Clipe/Osso é `anim_ui.side_tab` (outros trechos a leem): os dois andam juntos
                self.anim_ui.side_tab = if tab == PropTab::Bone { super::animation_ui::SideTab::Bone } else { super::animation_ui::SideTab::Clip };
            }
        }
    }

    /// Faixa de abas + conteúdo: desenha a faixa do `set`, grava a aba escolhida e devolve `(aba, Ui do conteúdo)`.
    pub(crate) fn tabbed(&mut self, ui: &mut egui::Ui, set: TabSet) -> (PropTab, egui::Ui) {
        self.tabbed_with(ui, set, set.tabs())
    }

    /// Como [`Self::tabbed`], com um subconjunto de abas (as que valem agora). Se a aba guardada não está nele, vale a primeira.
    pub(crate) fn tabbed_with(&mut self, ui: &mut egui::Ui, set: TabSet, tabs: &[PropTab]) -> (PropTab, egui::Ui) {
        self.tabs_shown.insert(set.slot(), tabs.to_vec());
        let stored = self.prop_tab(set);
        let current = if tabs.contains(&stored) { stored } else { tabs[0] };
        let (tab, content) = tab_strip(ui, tabs, current);
        if tab != stored {
            self.set_prop_tab(set, tab);
        }
        (tab, content)
    }

    /// O painel Propriedades com abas do workspace atual (`None` nos que não têm: Cenário, Render, Biblioteca).
    pub(crate) fn current_tab_set(&self) -> Option<TabSet> {
        use crate::workspace::WorkspaceId as W;
        Some(match self.ws {
            W::Modelagem => TabSet::Modeling(self.modeling.mode),
            W::Personagem => TabSet::Personagem,
            W::Posing => TabSet::Posing,
            W::Iluminacao => TabSet::Iluminacao,
            W::Animacao => TabSet::Animacao,
            W::Shading => TabSet::Shading,
            W::Cenario => TabSet::Cenario,
            _ => return None,
        })
    }

    pub(crate) fn modeling_prop_tab(&self, mode: Mode) -> PropTab {
        self.prop_tab(TabSet::Modeling(mode))
    }

    pub(crate) fn set_modeling_prop_tab(&mut self, mode: Mode, tab: PropTab) {
        self.set_prop_tab(TabSet::Modeling(mode), tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_set_has_unique_tabs_and_a_unique_slot() {
        let sets = [
            TabSet::Modeling(Mode::Object),
            TabSet::Modeling(Mode::Edit),
            TabSet::Modeling(Mode::Sculpt),
            TabSet::Modeling(Mode::Hair),
            TabSet::Personagem,
            TabSet::Posing,
            TabSet::Iluminacao,
            TabSet::Animacao,
            TabSet::Shading,
            TabSet::Cenario,
        ];
        let mut slots = std::collections::HashSet::new();
        for s in sets {
            assert!(slots.insert(s.slot()), "slot repetido: {s:?}");
            let tabs = s.tabs();
            assert!(!tabs.is_empty());
            let uniq: std::collections::HashSet<_> = tabs.iter().collect();
            assert_eq!(uniq.len(), tabs.len(), "{s:?} repete aba");
        }
    }

    #[test]
    fn all_tab_keys_are_unique_and_roundtrip() {
        let mut keys = std::collections::HashSet::new();
        for t in ALL {
            assert!(keys.insert(t.key()), "chave repetida: {}", t.key());
            assert_eq!(PropTab::from_key(t.key()), Some(t));
        }
    }
}
