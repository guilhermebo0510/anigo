//! Registro dos PAINÉIS de cada workspace: id estável, título (chave i18n), ícone, de onde reaparece quando
//! reaberto e o layout PADRÃO em pixels (que reproduz os painéis fixos do egui que existiam antes do dock).
//!
//! Sem egui aqui: só dados. A função de desenho de cada painel mora em `app/dock_ui.rs`
//! (um `match` por id), porque os corpos são métodos de `StudioApp`.

use super::model::{column, fill, insert_pane, resolve, row, sized, Home, LayoutNode, Spec};
use std::collections::HashMap;
use crate::icons;
use crate::workspace::WorkspaceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    /// Viewport/área central: obrigatória, não fecha, sem cabeçalho quando está sozinha.
    Viewport,
    Panel,
}

#[derive(Debug, Clone, Copy)]
pub struct PanelSpec {
    /// Id estável (persistido). Formato `<workspace>.<painel>`.
    pub id: &'static str,
    pub title_key: &'static str,
    pub icon: &'static str,
    pub kind: PanelKind,
    /// Onde reaparece ao reabrir (Janela → Painéis).
    pub home: Home,
    /// Largura/altura em px ao reaparecer.
    pub px: f32,
    /// `true` = a visibilidade é uma FLAG do app (ex.: `scene_ui.drawer_open`): fechar/abrir o painel liga/desliga a flag
    /// (e vice-versa), então os botões antigos da UI continuam valendo.
    pub flagged: bool,
}

const fn viewport(id: &'static str) -> PanelSpec {
    PanelSpec { id, title_key: "dock.panel.viewport", icon: icons::MONITOR, kind: PanelKind::Viewport, home: Home::Left, px: 0.0, flagged: false }
}

const fn panel(id: &'static str, title_key: &'static str, icon: &'static str, home: Home, px: f32) -> PanelSpec {
    PanelSpec { id, title_key, icon, kind: PanelKind::Panel, home, px, flagged: false }
}

const fn flagged(id: &'static str, title_key: &'static str, icon: &'static str, home: Home, px: f32) -> PanelSpec {
    PanelSpec { id, title_key, icon, kind: PanelKind::Panel, home, px, flagged: true }
}

const PERSONAGEM: &[PanelSpec] = &[
    viewport("personagem.viewport"),
    panel("personagem.inspector", "dock.panel.inspector", icons::SLIDERS, Home::Right, 350.0),
];
const MODELAGEM: &[PanelSpec] = &[
    viewport("modelagem.viewport"),
    panel("modelagem.inspector", "dock.panel.inspector", icons::SLIDERS, Home::Right, 340.0),
    flagged("modelagem.uv", "dock.panel.uv", icons::GRID_FOUR, Home::BelowViewport, 360.0),
];
const POSING: &[PanelSpec] = &[
    viewport("posing.viewport"),
    panel("posing.inspector", "dock.panel.pose_inspector", icons::BONE, Home::Right, 360.0),
];
const SHADING: &[PanelSpec] = &[
    viewport("shading.viewport"),
    panel("shading.targets", "dock.panel.shading_targets", icons::LIST_BULLETS, Home::Left, 250.0),
    panel("shading.params", "dock.panel.shading_params", icons::PALETTE, Home::Right, 350.0),
];
const ILUMINACAO: &[PanelSpec] = &[
    viewport("iluminacao.viewport"),
    panel("iluminacao.inspector", "dock.panel.lighting", icons::SUN, Home::Right, 350.0),
];
const CENARIO: &[PanelSpec] = &[
    viewport("cenario.viewport"),
    flagged("cenario.place", "dock.panel.place", icons::PLUS_SQUARE, Home::Left, 150.0),
    panel("cenario.outliner", "dock.panel.outliner", icons::LIST_BULLETS, Home::Right, 340.0),
    panel("cenario.history", "dock.panel.history", icons::CLOCK_COUNTER_CLOCKWISE, Home::Right, 340.0),
    panel("cenario.details", "dock.panel.details", icons::SLIDERS, Home::Right, 340.0),
    flagged("cenario.drawer", "dock.panel.drawer", icons::SQUARES_FOUR, Home::BelowViewport, 250.0),
];
const RENDER: &[PanelSpec] = &[
    viewport("render.viewport"),
    panel("render.settings", "dock.panel.render_settings", icons::CAMERA, Home::Right, 370.0),
    flagged("render.queue", "dock.panel.render_queue", icons::STACK, Home::Bottom, 230.0),
];
const ANIMACAO: &[PanelSpec] = &[
    viewport("animacao.viewport"),
    panel("animacao.inspector", "dock.panel.inspector", icons::SLIDERS, Home::Right, 360.0),
    panel("animacao.timeline", "dock.panel.timeline", icons::CLOCK, Home::BelowViewport, 330.0),
];
const BIBLIOTECA: &[PanelSpec] = &[
    // o "viewport" da Biblioteca é o navegador de ativos (grade) — central, obrigatório
    PanelSpec { id: "biblioteca.browser", title_key: "dock.panel.browser", icon: icons::SQUARES_FOUR, kind: PanelKind::Viewport, home: Home::Left, px: 0.0, flagged: false },
    panel("biblioteca.sidebar", "dock.panel.lib_sidebar", icons::LIST_BULLETS, Home::Left, 250.0),
    panel("biblioteca.details", "dock.panel.details", icons::SLIDERS, Home::Right, 330.0),
];

/// Painéis de um workspace (o primeiro é sempre o central).
pub fn panels(ws: WorkspaceId) -> &'static [PanelSpec] {
    match ws {
        WorkspaceId::Personagem => PERSONAGEM,
        WorkspaceId::Modelagem => MODELAGEM,
        WorkspaceId::Posing => POSING,
        WorkspaceId::Shading => SHADING,
        WorkspaceId::Iluminacao => ILUMINACAO,
        WorkspaceId::Cenario => CENARIO,
        WorkspaceId::Animacao => ANIMACAO,
        WorkspaceId::Render => RENDER,
        WorkspaceId::Biblioteca => BIBLIOTECA,
    }
}

/// O painel central (viewport) do workspace.
pub fn central(ws: WorkspaceId) -> &'static PanelSpec {
    panels(ws).iter().find(|p| p.kind == PanelKind::Viewport).expect("todo workspace tem painel central")
}

pub fn ids(ws: WorkspaceId) -> Vec<&'static str> {
    panels(ws).iter().map(|p| p.id).collect()
}

/// Procura um painel por id em todos os workspaces (para títulos e abas).
pub fn find(id: &str) -> Option<&'static PanelSpec> {
    WorkspaceId::ALL.iter().flat_map(|w| panels(*w).iter()).find(|p| p.id == id)
}

/// Devolve o `&'static str` do id (para guardar na árvore do dock sem alocar).
pub fn static_id(ws: WorkspaceId, id: &str) -> Option<&'static str> {
    panels(ws).iter().find(|p| p.id == id).map(|p| p.id)
}

/// Layout padrão (em px) do workspace — reproduz os painéis do egui anteriores ao dock. Inclui TODOS os painéis;
/// os controlados por flag que estiverem desligados são podados por quem monta o layout.
pub fn default_spec(ws: WorkspaceId) -> Spec {
    use Spec::Pane;
    match ws {
        WorkspaceId::Personagem => row(vec![fill(Pane("personagem.viewport")), sized(350.0, Pane("personagem.inspector"))]),
        // inspector à direita (alto) e o editor de UV só sob o viewport
        WorkspaceId::Modelagem => row(vec![fill(column(vec![fill(Pane("modelagem.viewport")), sized(360.0, Pane("modelagem.uv"))])), sized(340.0, Pane("modelagem.inspector"))]),
        WorkspaceId::Posing => row(vec![fill(Pane("posing.viewport")), sized(360.0, Pane("posing.inspector"))]),
        WorkspaceId::Shading => row(vec![sized(250.0, Pane("shading.targets")), fill(Pane("shading.viewport")), sized(350.0, Pane("shading.params"))]),
        WorkspaceId::Iluminacao => row(vec![fill(Pane("iluminacao.viewport")), sized(350.0, Pane("iluminacao.inspector"))]),
        WorkspaceId::Cenario => row(vec![
            sized(150.0, Pane("cenario.place")),
            fill(column(vec![fill(Pane("cenario.viewport")), sized(250.0, Pane("cenario.drawer"))])),
            sized(
                340.0,
                column(vec![fill(Spec::Tabs(vec![Pane("cenario.outliner"), Pane("cenario.history")])), fill(Pane("cenario.details"))]),
            ),
        ]),
        // a fila ocupa a largura toda embaixo (o painel inferior era declarado antes do direito)
        WorkspaceId::Render => column(vec![fill(row(vec![fill(Pane("render.viewport")), sized(370.0, Pane("render.settings"))])), sized(230.0, Pane("render.queue"))]),
        // a timeline fica sob o viewport; o inspector segue alto
        WorkspaceId::Animacao => row(vec![fill(column(vec![fill(Pane("animacao.viewport")), sized(330.0, Pane("animacao.timeline"))])), sized(360.0, Pane("animacao.inspector"))]),
        WorkspaceId::Biblioteca => row(vec![sized(250.0, Pane("biblioteca.sidebar")), fill(Pane("biblioteca.browser")), sized(330.0, Pane("biblioteca.details"))]),
    }
}

/// Layout padrão resolvido para `area` (largura, altura) com os painéis `flagged` desligados podados.
pub fn default_layout(ws: WorkspaceId, area: (f32, f32), flag_on: &dyn Fn(&str) -> bool) -> LayoutNode {
    let n = resolve(&default_spec(ws), area);
    n.retaining(&|id| find(id).is_none_or(|p| !p.flagged || flag_on(id))).expect("o viewport nunca é podado")
}

/// Painéis controlados por flag: faz o layout refletir `present` (a flag manda). Painel que some guarda o layout de antes em
/// `remembered`; ao voltar, se o RESTO do layout não mudou nesse meio-tempo, reaparece exatamente onde estava; senão no "lar"
/// do painel (`PanelSpec::home`). Devolve `Some(novo)` só se algo mudou.
pub fn reconcile_flagged(ws: WorkspaceId, root: &LayoutNode, present: &dyn Fn(&str) -> bool, area: (f32, f32), remembered: &mut HashMap<(WorkspaceId, &'static str), LayoutNode>) -> Option<LayoutNode> {
    let central_id = central(ws).id;
    let mut cur = root.clone();
    let mut changed = false;
    for p in panels(ws).iter().filter(|p| p.flagged) {
        let want = present(p.id);
        let have = cur.contains(p.id);
        if want && !have {
            let mut now: Vec<&str> = cur.panes();
            now.push(p.id);
            now.sort_unstable();
            let restored = remembered.remove(&(ws, p.id)).filter(|m| {
                let mut ids = m.panes();
                ids.sort_unstable();
                ids == now
            });
            cur = restored.unwrap_or_else(|| insert_pane(Some(cur.clone()), p.id, p.home, p.px, area, central_id));
            changed = true;
        } else if !want && have {
            remembered.insert((ws, p.id), cur.clone());
            match cur.clone().without(p.id) {
                Some(n) => {
                    cur = n;
                    changed = true;
                }
                None => return None,
            }
        }
    }
    changed.then_some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{tr_in, Lang};
    use std::collections::HashSet;

    #[test]
    fn every_workspace_has_one_central_panel_with_unique_ids_and_prefix() {
        let mut all = HashSet::new();
        for w in WorkspaceId::ALL {
            let ps = panels(w);
            assert_eq!(ps.iter().filter(|p| p.kind == PanelKind::Viewport).count(), 1, "{w:?}");
            assert_eq!(ps[0].kind, PanelKind::Viewport, "o central vem primeiro em {w:?}");
            for p in ps {
                assert!(p.id.starts_with(&format!("{}.", w.key())), "{} fora do prefixo de {w:?}", p.id);
                assert!(all.insert(p.id), "id repetido {}", p.id);
                if p.kind == PanelKind::Panel {
                    assert!(p.px >= 80.0, "{} sem tamanho ao reabrir", p.id);
                }
            }
        }
    }

    #[test]
    fn titles_are_translated_in_both_languages() {
        for w in WorkspaceId::ALL {
            for p in panels(w) {
                for l in [Lang::PtBr, Lang::En] {
                    assert_ne!(tr_in(l, p.title_key), p.title_key, "{} sem tradução {l:?}", p.title_key);
                }
            }
        }
    }

    #[test]
    fn default_layout_contains_exactly_the_active_panels_and_is_stable_under_sanitize() {
        for w in WorkspaceId::ALL {
            let known = ids(w);
            let area = (1234.0, 700.0);
            // todas as flags ligadas: todos os painéis
            let all = default_layout(w, area, &|_| true);
            let mut got: Vec<&str> = all.panes();
            got.sort_unstable();
            let mut want = known.clone();
            want.sort_unstable();
            assert_eq!(got, want, "{w:?}");
            assert_eq!(all.clone().sanitized_with_required(&known, &[central(w).id]), Some(all.clone()), "o padrão é um ponto fixo da sanitização em {w:?}");
            // todas as flags desligadas: só os não-controlados
            let none = default_layout(w, area, &|_| false);
            let expected: Vec<&str> = panels(w).iter().filter(|p| !p.flagged).map(|p| p.id).collect();
            let mut got2 = none.panes();
            got2.sort_unstable();
            let mut exp2 = expected;
            exp2.sort_unstable();
            assert_eq!(got2, exp2, "{w:?}");
        }
    }

    #[test]
    fn default_widths_match_the_old_fixed_panels() {
        // Cenário a 1234 px: Posicionar 150 | viewport | direita 340
        let n = default_layout(WorkspaceId::Cenario, (1234.0, 700.0), &|_| true);
        let LayoutNode::Split { vertical: false, children } = &n else { panic!() };
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].share, 150.0);
        assert_eq!(children[2].share, 340.0);
        assert_eq!(children[1].share, 1234.0 - 150.0 - 340.0);
        // Animação: timeline (330) só sob o viewport
        let n = default_layout(WorkspaceId::Animacao, (1234.0, 700.0), &|_| true);
        let LayoutNode::Split { vertical: false, children } = &n else { panic!() };
        assert_eq!(children[1].share, 360.0);
        assert!(matches!(&children[0].node, LayoutNode::Split { vertical: true, .. }));
    }

    #[test]
    fn flag_driven_panels_follow_the_flag_and_come_back_where_they_were() {
        let ws = WorkspaceId::Cenario;
        let area = (1234.0, 700.0);
        let mut mem = HashMap::new();
        let all_on = |_: &str| true;
        let drawer_off = |id: &str| id != "cenario.drawer";
        let base = default_layout(ws, area, &all_on);
        // nada a fazer quando o layout já reflete as flags
        assert!(reconcile_flagged(ws, &base, &all_on, area, &mut mem).is_none());
        // desliga a gaveta: sai do layout e o layout anterior fica guardado
        let hidden = reconcile_flagged(ws, &base, &drawer_off, area, &mut mem).expect("gaveta some");
        assert!(!hidden.contains("cenario.drawer") && hidden.contains("cenario.viewport"));
        assert!(mem.contains_key(&(ws, "cenario.drawer")));
        // o usuário NÃO mexeu no resto: ao religar volta exatamente igual
        let back = reconcile_flagged(ws, &hidden, &all_on, area, &mut mem).expect("gaveta volta");
        assert_eq!(back, base, "voltou exatamente ao lugar");
        // se o usuário reorganizou o resto enquanto a gaveta estava escondida, usa o lar (BelowViewport)
        let hidden = reconcile_flagged(ws, &base, &drawer_off, area, &mut mem).unwrap();
        let moved = hidden.clone().without("cenario.place").unwrap();
        let back = reconcile_flagged(ws, &moved, &all_on, area, &mut mem).unwrap();
        assert!(back.contains("cenario.drawer") && back.contains("cenario.place"), "o lar recoloca a gaveta e a flag reabre o Posicionar");
        // tudo desligado: sobra só o que não é condicional
        let none = |_: &str| false;
        let n = reconcile_flagged(ws, &base, &none, area, &mut HashMap::new()).unwrap();
        let mut got = n.panes();
        got.sort_unstable();
        assert_eq!(got, vec!["cenario.details", "cenario.history", "cenario.outliner", "cenario.viewport"]);
    }

    #[test]
    fn reopening_every_closed_panel_restores_all_ids() {
        for w in WorkspaceId::ALL {
            let area = (1234.0, 700.0);
            let mut n = Some(default_layout(w, area, &|_| true));
            for p in panels(w).iter().filter(|p| p.kind == PanelKind::Panel) {
                n = n.unwrap().without(p.id);
                assert!(n.as_ref().is_some_and(|n| !n.contains(p.id)));
            }
            assert_eq!(n.as_ref().unwrap().panes(), vec![central(w).id]);
            for p in panels(w).iter().filter(|p| p.kind == PanelKind::Panel) {
                n = Some(super::super::model::insert_pane(n, p.id, p.home, p.px, area, central(w).id));
            }
            let mut got: Vec<&str> = n.as_ref().unwrap().panes();
            got.sort_unstable();
            let mut want = ids(w);
            want.sort_unstable();
            assert_eq!(got, want, "{w:?}");
        }
    }
}
