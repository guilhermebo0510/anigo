//! Ponte entre o modelo persistido ([`LayoutNode`]) e a árvore em tempo de execução do `egui_tiles`.
//! Sem desenho: só conversão (testável sem contexto egui).

use super::model::{LayoutNode, SplitChild};
use super::registry;
use crate::workspace::WorkspaceId;
use egui_tiles::{Container, Linear, LinearDir, Tabs, Tile, TileId, Tiles, Tree};

/// Id estável do painel (`&'static str` do registro) guardado em cada folha da árvore.
pub type PaneId = &'static str;

/// Monta a árvore do `egui_tiles` a partir do modelo. Painéis que não pertencem ao workspace são ignorados
/// (o chamador já sanitizou, isto é só defesa).
pub fn node_to_tree(ws: WorkspaceId, node: &LayoutNode) -> Tree<PaneId> {
    let mut tiles = Tiles::default();
    let root = build(&mut tiles, ws, node);
    let id = egui::Id::new(("anigo_dock", ws.key()));
    match root {
        Some(r) => Tree::new(id, r, tiles),
        None => Tree::empty(id),
    }
}

fn build(tiles: &mut Tiles<PaneId>, ws: WorkspaceId, node: &LayoutNode) -> Option<TileId> {
    match node {
        LayoutNode::Pane { id } => registry::static_id(ws, id).map(|p| tiles.insert_pane(p)),
        LayoutNode::Tabs { children, active } => {
            let kids: Vec<(usize, TileId)> = children.iter().enumerate().filter_map(|(i, c)| build(tiles, ws, c).map(|t| (i, t))).collect();
            if kids.is_empty() {
                return None;
            }
            let ids: Vec<TileId> = kids.iter().map(|(_, t)| *t).collect();
            let mut tabs = Tabs::new(ids.clone());
            let act = kids.iter().find(|(i, _)| i == active).map(|(_, t)| *t).unwrap_or(ids[0]);
            tabs.set_active(act);
            Some(tiles.insert_new(Tile::Container(Container::Tabs(tabs))))
        }
        LayoutNode::Split { vertical, children } => {
            let kids: Vec<(f32, TileId)> = children.iter().filter_map(|c| build(tiles, ws, &c.node).map(|t| (c.share, t))).collect();
            if kids.is_empty() {
                return None;
            }
            let ids: Vec<TileId> = kids.iter().map(|(_, t)| *t).collect();
            let mut lin = Linear::new(if *vertical { LinearDir::Vertical } else { LinearDir::Horizontal }, ids);
            for (share, t) in &kids {
                lin.shares.set_share(*t, *share);
            }
            Some(tiles.insert_new(Tile::Container(Container::Linear(lin))))
        }
    }
}

/// Lê a árvore atual de volta para o modelo persistido. `None` se estiver vazia.
pub fn tree_to_node(tree: &Tree<PaneId>) -> Option<LayoutNode> {
    let root = tree.root()?;
    to_node(&tree.tiles, root)
}

fn to_node(tiles: &Tiles<PaneId>, id: TileId) -> Option<LayoutNode> {
    match tiles.get(id)? {
        Tile::Pane(p) => Some(LayoutNode::pane(p)),
        Tile::Container(Container::Tabs(t)) => {
            let mut children = Vec::new();
            let mut active = 0;
            for c in &t.children {
                if let Some(n) = to_node(tiles, *c) {
                    if t.active == Some(*c) {
                        active = children.len();
                    }
                    children.push(n);
                }
            }
            (!children.is_empty()).then_some(LayoutNode::Tabs { children, active })
        }
        Tile::Container(Container::Linear(l)) => {
            let children: Vec<SplitChild> = l.children.iter().filter_map(|c| to_node(tiles, *c).map(|node| SplitChild { share: l.shares[*c], node })).collect();
            (!children.is_empty()).then_some(LayoutNode::Split { vertical: l.dir == LinearDir::Vertical, children })
        }
        // grades não são criadas pela UI; se aparecerem, viram uma linha (nada se perde)
        Tile::Container(c @ Container::Grid(_)) => {
            let children: Vec<SplitChild> = c.children().filter_map(|k| to_node(tiles, *k).map(|node| SplitChild { share: 1.0, node })).collect();
            (!children.is_empty()).then_some(LayoutNode::Split { vertical: false, children })
        }
    }
}

/// `TileId` do painel `id` na árvore.
pub fn find_pane(tree: &Tree<PaneId>, id: &str) -> Option<TileId> {
    tree.tiles.iter().find_map(|(tid, t)| match t {
        Tile::Pane(p) if *p == id => Some(*tid),
        _ => None,
    })
}

/// Ids dos painéis da árvore.
pub fn pane_ids(tree: &Tree<PaneId>) -> Vec<PaneId> {
    tree.tiles
        .iter()
        .filter_map(|(_, t)| match t {
            Tile::Pane(p) => Some(*p),
            _ => None,
        })
        .collect()
}

/// Painéis cujo pai é um grupo de abas (já têm barra de abas; não precisam de cabeçalho próprio).
pub fn panes_in_tabs(tree: &Tree<PaneId>) -> std::collections::HashSet<TileId> {
    let mut out = std::collections::HashSet::new();
    for (_, t) in tree.tiles.iter() {
        if let Tile::Container(Container::Tabs(tabs)) = t {
            out.extend(tabs.children.iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::model::{insert_pane, Home};

    #[test]
    fn default_layouts_roundtrip_through_the_tiles_tree_for_every_workspace() {
        for w in WorkspaceId::ALL {
            let n = registry::default_layout(w, (1234.0, 700.0), &|_| true);
            let tree = node_to_tree(w, &n);
            let back = tree_to_node(&tree).expect("árvore não vazia");
            assert_eq!(back, n, "ida e volta mudou o layout de {w:?}");
            let mut ids: Vec<&str> = pane_ids(&tree);
            ids.sort_unstable();
            let mut want = n.panes();
            want.sort_unstable();
            assert_eq!(ids, want);
        }
    }

    #[test]
    fn tabs_active_and_shares_survive() {
        let n = LayoutNode::split(
            false,
            vec![
                (700.0, LayoutNode::pane("cenario.viewport")),
                (
                    300.0,
                    LayoutNode::Tabs { children: vec![LayoutNode::pane("cenario.outliner"), LayoutNode::pane("cenario.history"), LayoutNode::pane("cenario.details")], active: 2 },
                ),
            ],
        );
        let t = node_to_tree(WorkspaceId::Cenario, &n);
        assert_eq!(tree_to_node(&t).unwrap(), n);
        let hist = find_pane(&t, "cenario.history").unwrap();
        assert!(panes_in_tabs(&t).contains(&hist));
        assert!(!panes_in_tabs(&t).contains(&find_pane(&t, "cenario.viewport").unwrap()));
    }

    #[test]
    fn unknown_ids_are_ignored_defensively_and_empty_trees_are_none() {
        let n = LayoutNode::split(false, vec![(1.0, LayoutNode::pane("fantasma")), (1.0, LayoutNode::pane("render.viewport"))]);
        let t = node_to_tree(WorkspaceId::Render, &n);
        assert_eq!(pane_ids(&t), vec!["render.viewport"]);
        let t = node_to_tree(WorkspaceId::Render, &LayoutNode::pane("fantasma"));
        assert!(tree_to_node(&t).is_none());
        // painel de outro workspace também é ignorado
        let t = node_to_tree(WorkspaceId::Render, &LayoutNode::pane("cenario.viewport"));
        assert!(t.is_empty());
    }

    #[test]
    fn inserting_through_the_model_then_rebuilding_keeps_the_rest_of_the_tree() {
        let w = WorkspaceId::Animacao;
        let base = registry::default_layout(w, (1234.0, 700.0), &|_| true).without("animacao.timeline");
        let n = insert_pane(base, "animacao.timeline", Home::BelowViewport, 330.0, (1234.0, 700.0), "animacao.viewport");
        let t = node_to_tree(w, &n);
        assert_eq!(tree_to_node(&t).unwrap(), n);
    }
}
