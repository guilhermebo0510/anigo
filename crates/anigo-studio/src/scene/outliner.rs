//! Modelo do Outliner (árvore + pastas + busca com a sintaxe do UE), sem egui.
//!
//! Busca (como o Outliner do UE): termos parciais, todos precisam casar; `-termo` exclui;
//! `+termo` exige o nome exato; `"duas palavras"` casa a frase inteira. Os termos parciais olham
//! nome, tags, pasta e tipo.

use anigo_core::ids::NodeId;
use anigo_core::project::{NodeSlot, ProjectState};
use anigo_core::NodeSource;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    /// Substring em nome/tags/pasta/tipo.
    Partial(String),
    /// Não pode casar.
    Exclude(String),
    /// Nome idêntico.
    Exact(String),
    /// Frase (substring do nome, com espaços).
    Phrase(String),
}

/// Consulta compilada.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let mut terms = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            if c == '"' {
                let mut j = i + 1;
                let mut s = String::new();
                while j < chars.len() && chars[j] != '"' {
                    s.push(chars[j]);
                    j += 1;
                }
                if !s.trim().is_empty() {
                    terms.push(Term::Phrase(s.trim().to_lowercase()));
                }
                i = j + 1;
                continue;
            }
            let mut j = i;
            let mut word = String::new();
            while j < chars.len() && !chars[j].is_whitespace() {
                word.push(chars[j]);
                j += 1;
            }
            i = j;
            let lower = word.to_lowercase();
            if let Some(rest) = lower.strip_prefix('-') {
                if !rest.is_empty() {
                    terms.push(Term::Exclude(rest.to_string()));
                }
            } else if let Some(rest) = lower.strip_prefix('+') {
                if !rest.is_empty() {
                    terms.push(Term::Exact(rest.to_string()));
                }
            } else {
                terms.push(Term::Partial(lower));
            }
        }
        Query { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// `true` se o nó satisfaz TODOS os termos.
    pub fn matches(&self, node: &NodeSlot) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let name = node.name.to_lowercase();
        let haystack = || {
            let mut h = name.clone();
            for t in &node.tags {
                h.push('\n');
                h.push_str(&t.to_lowercase());
            }
            if let Some(f) = &node.folder {
                h.push('\n');
                h.push_str(&f.to_lowercase());
            }
            h.push('\n');
            h.push_str(node.kind.as_str());
            h
        };
        let hay = haystack();
        self.terms.iter().all(|t| match t {
            Term::Partial(p) => hay.contains(p.as_str()),
            Term::Exclude(p) => !hay.contains(p.as_str()),
            Term::Exact(p) => name == *p,
            Term::Phrase(p) => name.contains(p.as_str()),
        })
    }
}

/// Uma linha do Outliner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder { path: String, name: String, depth: usize, expanded: bool, count: usize },
    Node { id: NodeId, depth: usize, has_children: bool, expanded: bool },
}

/// Estado de expansão (UI).
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    pub nodes: BTreeSet<NodeId>,
    pub folders: BTreeSet<String>,
    /// Nós/pastas recolhidos explicitamente (o padrão é expandido).
    pub collapsed_nodes: BTreeSet<NodeId>,
    pub collapsed_folders: BTreeSet<String>,
}

impl Expansion {
    pub fn node_open(&self, id: &NodeId) -> bool {
        !self.collapsed_nodes.contains(id)
    }

    pub fn folder_open(&self, path: &str) -> bool {
        !self.collapsed_folders.contains(path)
    }

    pub fn toggle_node(&mut self, id: &NodeId) {
        if !self.collapsed_nodes.remove(id) {
            self.collapsed_nodes.insert(id.clone());
        }
    }

    pub fn toggle_folder(&mut self, path: &str) {
        if !self.collapsed_folders.remove(path) {
            self.collapsed_folders.insert(path.to_string());
        }
    }

    pub fn open_node(&mut self, id: &NodeId) {
        self.collapsed_nodes.remove(id);
    }
}

/// Monta as linhas visíveis. Sem consulta: pastas (de nós raiz) + árvore de pais. Com consulta: lista
/// plana dos nós que casam.
pub fn build_rows(state: &ProjectState, expansion: &Expansion, query: &Query) -> Vec<Row> {
    let nodes = &state.scene.nodes;
    if !query.is_empty() {
        return nodes
            .iter()
            .filter(|n| query.matches(n))
            .map(|n| Row::Node { id: n.node_id.clone(), depth: 0, has_children: false, expanded: false })
            .collect();
    }
    let mut children: BTreeMap<&NodeId, Vec<&NodeSlot>> = BTreeMap::new();
    let mut roots: Vec<&NodeSlot> = Vec::new();
    let ids: BTreeSet<&NodeId> = nodes.iter().map(|n| &n.node_id).collect();
    for n in nodes {
        match &n.parent_id {
            Some(p) if ids.contains(p) => children.entry(p).or_default().push(n),
            _ => roots.push(n),
        }
    }
    // pastas: caminho "A/B" → raízes dentro dela
    let mut by_folder: BTreeMap<String, Vec<&NodeSlot>> = BTreeMap::new();
    let mut loose: Vec<&NodeSlot> = Vec::new();
    for r in roots {
        match r.folder.as_deref().filter(|f| !f.trim().is_empty()) {
            Some(f) => by_folder.entry(f.trim_matches('/').to_string()).or_default().push(r),
            None => loose.push(r),
        }
    }
    let mut rows = Vec::new();
    // todas as pastas e seus ancestrais, em ordem alfabética (A, A/B, A/B/C, D…)
    let mut all_folders: BTreeSet<String> = BTreeSet::new();
    for f in by_folder.keys() {
        let mut acc = String::new();
        for part in f.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            all_folders.insert(acc.clone());
        }
    }
    fn push_node(rows: &mut Vec<Row>, n: &NodeSlot, depth: usize, children: &BTreeMap<&NodeId, Vec<&NodeSlot>>, expansion: &Expansion) {
        let kids = children.get(&n.node_id);
        let has = kids.is_some_and(|k| !k.is_empty());
        let open = expansion.node_open(&n.node_id);
        rows.push(Row::Node { id: n.node_id.clone(), depth, has_children: has, expanded: open });
        if has && open {
            for k in kids.into_iter().flatten() {
                push_node(rows, k, depth + 1, children, expansion);
            }
        }
    }
    let count_in = |path: &str| -> usize { by_folder.iter().filter(|(k, _)| *k == path || k.starts_with(&format!("{path}/"))).map(|(_, v)| v.len()).sum() };
    let mut hidden_under: Option<String> = None;
    for path in &all_folders {
        if let Some(h) = &hidden_under {
            if path.starts_with(&format!("{h}/")) {
                continue;
            }
            hidden_under = None;
        }
        let depth = path.matches('/').count();
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let open = expansion.folder_open(path);
        rows.push(Row::Folder { path: path.clone(), name, depth, expanded: open, count: count_in(path) });
        if !open {
            hidden_under = Some(path.clone());
            continue;
        }
        if let Some(list) = by_folder.get(path) {
            for n in list {
                push_node(&mut rows, n, depth + 1, &children, expansion);
            }
        }
    }
    for n in loose {
        push_node(&mut rows, n, 0, &children, expansion);
    }
    rows
}

/// Texto curto do tipo/origem de um nó (coluna de informação e tooltip).
pub fn source_label(n: &NodeSlot) -> &'static str {
    match &n.source {
        Some(NodeSource::LibraryAsset { .. }) => "lib",
        Some(NodeSource::Primitive { .. }) => "prim",
        Some(NodeSource::Camera { .. }) => "cam",
        Some(NodeSource::Light { .. }) => "light",
        Some(NodeSource::Card { .. }) => "card",
        None => n.kind.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::Command;
    use anigo_core::level::{plan_place_primitive, NodePatch, PrimitiveShape};
    use anigo_core::Transform;

    fn state_with(names: &[&str]) -> (ProjectState, Vec<NodeId>) {
        let mut st = ProjectState::default();
        let mut ids = vec![];
        for n in names {
            let (cmd, id) = plan_place_primitive(&st, PrimitiveShape::Cube, n, Transform::default(), &Default::default());
            cmd.apply(&mut st).unwrap();
            ids.push(id);
        }
        (st, ids)
    }

    fn patch(st: &mut ProjectState, id: &NodeId, p: NodePatch) {
        Command::SetNodeProps { node_id: id.clone(), patch: p }.apply(st).unwrap();
    }

    #[test]
    fn query_syntax_matches_the_ue_outliner() {
        let (mut st, ids) = state_with(&["Mesa Grande", "Mesa Pequena", "Cadeira"]);
        patch(&mut st, &ids[2], NodePatch { tags: Some(vec!["madeira".into()]), ..Default::default() });
        let find = |q: &str| -> Vec<String> { st.scene.nodes.iter().filter(|n| Query::parse(q).matches(n)).map(|n| n.name.clone()).collect() };
        assert_eq!(find("mesa").len(), 2, "parcial");
        assert_eq!(find("mesa grande"), vec!["Mesa Grande"], "todos os termos precisam casar");
        assert_eq!(find("mesa -peq"), vec!["Mesa Grande"], "-x exclui");
        assert_eq!(find("+cadeira"), vec!["Cadeira"], "+x = nome exato");
        assert!(find("+cadei").is_empty());
        assert_eq!(find("\"mesa pequena\""), vec!["Mesa Pequena"], "frase inteira");
        assert_eq!(find("madeira"), vec!["Cadeira"], "busca também nas tags");
        assert_eq!(find("").len(), st.scene.nodes.len(), "consulta vazia mostra tudo");
        assert!(Query::parse("  - + \"\" ").is_empty(), "tokens vazios são ignorados");
    }

    #[test]
    fn tree_rows_nest_children_and_honor_collapse() {
        let (mut st, ids) = state_with(&["Pai", "Filho", "Neto"]);
        Command::SetNodeParent { node_id: ids[1].clone(), parent_id: Some(ids[0].clone()) }.apply(&mut st).unwrap();
        Command::SetNodeParent { node_id: ids[2].clone(), parent_id: Some(ids[1].clone()) }.apply(&mut st).unwrap();
        let mut ex = Expansion::default();
        let rows = build_rows(&st, &ex, &Query::default());
        let depth_of = |rows: &[Row], id: &NodeId| rows.iter().find_map(|r| match r {
            Row::Node { id: i, depth, .. } if i == id => Some(*depth),
            _ => None,
        });
        assert_eq!(depth_of(&rows, &ids[0]), Some(0));
        assert_eq!(depth_of(&rows, &ids[1]), Some(1));
        assert_eq!(depth_of(&rows, &ids[2]), Some(2));
        ex.toggle_node(&ids[0]);
        let rows = build_rows(&st, &ex, &Query::default());
        assert_eq!(depth_of(&rows, &ids[1]), None, "filhos de um nó recolhido somem");
        assert!(rows.iter().any(|r| matches!(r, Row::Node { id, has_children: true, expanded: false, .. } if id == &ids[0])));
    }

    #[test]
    fn folders_group_root_nodes_and_nest_by_path() {
        let (mut st, ids) = state_with(&["A", "B", "C"]);
        patch(&mut st, &ids[0], NodePatch { folder: Some("Props/Mesas".into()), ..Default::default() });
        patch(&mut st, &ids[1], NodePatch { folder: Some("Props".into()), ..Default::default() });
        let mut ex = Expansion::default();
        let rows = build_rows(&st, &ex, &Query::default());
        let folders: Vec<(String, usize, usize)> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Folder { path, depth, count, .. } => Some((path.clone(), *depth, *count)),
                _ => None,
            })
            .collect();
        assert_eq!(folders, vec![("Props".to_string(), 0, 2), ("Props/Mesas".to_string(), 1, 1)]);
        // recolher "Props" esconde a subpasta e os nós dela
        ex.toggle_folder("Props");
        let rows = build_rows(&st, &ex, &Query::default());
        assert!(!rows.iter().any(|r| matches!(r, Row::Folder { path, .. } if path == "Props/Mesas")));
        assert!(!rows.iter().any(|r| matches!(r, Row::Node { id, .. } if id == &ids[0])));
        // o nó sem pasta continua na raiz
        assert!(rows.iter().any(|r| matches!(r, Row::Node { id, depth: 0, .. } if id == &ids[2])));
    }

    #[test]
    fn a_query_flattens_the_list() {
        let (st, _) = state_with(&["Alfa", "Beta"]);
        let rows = build_rows(&st, &Expansion::default(), &Query::parse("alf"));
        assert_eq!(rows.len(), 1);
    }
}
