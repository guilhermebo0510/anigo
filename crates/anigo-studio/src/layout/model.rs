//! Modelo PERSISTIDO do layout dockável (sem egui): uma árvore de `Split` (linha/coluna com `share`),
//! `Tabs` e `Pane(id estável)`. O dock em tempo de execução (`egui_tiles`) converte de/para este
//! formato, então o arquivo em disco não depende da versão do crate de docking.
//!
//! Regras de robustez (testadas): ids de painel desconhecidos ou repetidos são descartados,
//! containers vazios somem, `share` inválido (NaN, negativo, zero) é corrigido, `active` fora do intervalo
//! volta para 0 — nenhum JSON, por mais estranho que seja, derruba o app.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Versão atual do esquema do arquivo/sessão de layouts.
pub const LAYOUT_SCHEMA: u32 = 1;

/// Fração mínima (do total do container) que qualquer filho de um `Split` mantém após a sanitização.
pub const MIN_FRACTION: f32 = 0.04;

fn default_share() -> f32 {
    1.0
}

/// Aceita número ou `null` (NaN/inf viram `null` no JSON); qualquer outra coisa também vira 1.
fn de_share<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().map(|f| f as f32).unwrap_or(1.0))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitChild {
    #[serde(default = "default_share", deserialize_with = "de_share")]
    pub share: f32,
    pub node: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayoutNode {
    /// `vertical = false`: filhos lado a lado (esquerda→direita); `true`: empilhados (cima→baixo).
    Split { vertical: bool, children: Vec<SplitChild> },
    Tabs {
        children: Vec<LayoutNode>,
        #[serde(default)]
        active: usize,
    },
    Pane { id: String },
}

impl LayoutNode {
    pub fn pane(id: &str) -> Self {
        LayoutNode::Pane { id: id.to_string() }
    }

    pub fn split(vertical: bool, children: Vec<(f32, LayoutNode)>) -> Self {
        LayoutNode::Split { vertical, children: children.into_iter().map(|(share, node)| SplitChild { share, node }).collect() }
    }

    /// Ids dos painéis em ordem de visita (pré-ordem).
    pub fn panes(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            LayoutNode::Pane { id } => out.push(id),
            LayoutNode::Split { children, .. } => children.iter().for_each(|c| c.node.collect(out)),
            LayoutNode::Tabs { children, .. } => children.iter().for_each(|c| c.collect(out)),
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.panes().contains(&id)
    }

    /// Remove os painéis para os quais `keep` é falso (e duplicatas), colapsando containers que sobram vazios
    /// ou com um filho. Devolve `None` se nada restou. `seen` acumula os ids já mantidos.
    fn prune(self, keep: &dyn Fn(&str) -> bool, seen: &mut HashSet<String>) -> Option<LayoutNode> {
        match self {
            LayoutNode::Pane { id } => {
                if keep(&id) && seen.insert(id.clone()) {
                    Some(LayoutNode::Pane { id })
                } else {
                    None
                }
            }
            LayoutNode::Split { vertical, children } => {
                let mut kept: Vec<SplitChild> = children
                    .into_iter()
                    .filter_map(|c| {
                        let share = c.share;
                        c.node.prune(keep, seen).map(|node| SplitChild { share, node })
                    })
                    .collect();
                match kept.len() {
                    0 => None,
                    1 => Some(kept.remove(0).node),
                    _ => {
                        normalize_shares(&mut kept);
                        Some(LayoutNode::Split { vertical, children: kept })
                    }
                }
            }
            LayoutNode::Tabs { children, active } => {
                let was_active: Option<Vec<String>> = children.get(active).map(|c| c.panes().into_iter().map(String::from).collect());
                let mut kept: Vec<LayoutNode> = children.into_iter().filter_map(|c| c.prune(keep, seen)).collect();
                match kept.len() {
                    0 => None,
                    1 => Some(kept.remove(0)),
                    _ => {
                        // mantém a aba ativa se ela sobreviveu; senão a primeira
                        let active = was_active
                            .and_then(|ids| kept.iter().position(|k| k.panes().iter().any(|p| ids.iter().any(|i| i == p))))
                            .unwrap_or(0);
                        Some(LayoutNode::Tabs { children: kept, active })
                    }
                }
            }
        }
    }

    /// Mantém só painéis `known` (sem repetição). `None` se nada restou.
    pub fn sanitized(self, known: &[&str]) -> Option<LayoutNode> {
        let mut seen = HashSet::new();
        self.prune(&|id| known.contains(&id), &mut seen)
    }

    /// Como [`Self::sanitized`], mas exige TODOS os `required` presentes (senão `None`: o chamador usa o padrão).
    pub fn sanitized_with_required(self, known: &[&str], required: &[&str]) -> Option<LayoutNode> {
        let n = self.sanitized(known)?;
        required.iter().all(|r| n.contains(r)).then_some(n)
    }

    /// Remove um painel (e colapsa o que sobrar vazio/unitário).
    pub fn without(self, id: &str) -> Option<LayoutNode> {
        let mut seen = HashSet::new();
        self.prune(&|p| p != id, &mut seen)
    }

    /// Mantém só painéis para os quais `pred` é verdadeiro.
    pub fn retaining(self, pred: &dyn Fn(&str) -> bool) -> Option<LayoutNode> {
        let mut seen = HashSet::new();
        self.prune(pred, &mut seen)
    }
}

/// Corrige `share` inválido e impede filhos "invisíveis": cada filho mantém ao menos `MIN_FRACTION` do total
/// (exato: os que ficariam abaixo são elevados a `MIN_FRACTION` do total FINAL).
fn normalize_shares(children: &mut [SplitChild]) {
    for c in children.iter_mut() {
        if !c.share.is_finite() || c.share <= 0.0 {
            c.share = 1.0;
        }
    }
    let total: f32 = children.iter().map(|c| c.share).sum();
    let floor = total * MIN_FRACTION;
    let small: Vec<bool> = children.iter().map(|c| c.share < floor).collect();
    let n = small.iter().filter(|s| **s).count();
    if n == 0 || n == children.len() || (n as f32) * MIN_FRACTION >= 1.0 {
        return;
    }
    let others: f32 = children.iter().zip(&small).filter(|(_, s)| !**s).map(|(c, _)| c.share).sum();
    let raised = MIN_FRACTION * others / (1.0 - n as f32 * MIN_FRACTION);
    for (c, s) in children.iter_mut().zip(small) {
        if s {
            c.share = raised;
        }
    }
}

// ----------------------------------------------------------------------------------------- inserção

/// Onde um painel reaparece quando reaberto (Janela → Painéis) ou quando uma flag o liga.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// Coluna à esquerda de tudo.
    Left,
    /// Coluna à direita de tudo.
    Right,
    /// Faixa embaixo de tudo.
    Bottom,
    /// Faixa embaixo do viewport (só sob ele; as colunas laterais seguem altas).
    BelowViewport,
}

/// Insere `id` no layout em `home`, com `px` de largura/altura numa área de `area` (largura, altura) pixels.
/// `viewport` é o painel central (âncora de `BelowViewport`). Painel já presente = sem efeito.
pub fn insert_pane(root: Option<LayoutNode>, id: &str, home: Home, px: f32, area: (f32, f32), viewport: &str) -> LayoutNode {
    let new = LayoutNode::pane(id);
    let Some(root) = root else { return new };
    if root.contains(id) {
        return root;
    }
    let (w, h) = (area.0.max(200.0), area.1.max(200.0));
    match home {
        Home::Left | Home::Right => {
            let px = px.clamp(80.0, w * 0.6);
            let mut kids = match root {
                LayoutNode::Split { vertical: false, children } => children,
                other => vec![SplitChild { share: w - px, node: other }],
            };
            // as `shares` existentes ocupam (w - px) px: reescala para somarem isso e dá `px` ao novo
            let total: f32 = kids.iter().map(|c| c.share).sum::<f32>().max(1e-3);
            for c in kids.iter_mut() {
                c.share = c.share / total * (w - px);
            }
            let child = SplitChild { share: px, node: new };
            if home == Home::Left {
                kids.insert(0, child);
            } else {
                kids.push(child);
            }
            LayoutNode::Split { vertical: false, children: kids }
        }
        Home::Bottom => {
            let px = px.clamp(80.0, h * 0.7);
            LayoutNode::Split { vertical: true, children: vec![SplitChild { share: h - px, node: root }, SplitChild { share: px, node: new }] }
        }
        Home::BelowViewport => {
            let px = px.clamp(80.0, h * 0.7);
            let mut root = root;
            if !wrap_below(&mut root, viewport, new, px, h) {
                // sem viewport no layout (não deveria): cai para o fundo
                return insert_pane(Some(root), id, Home::Bottom, px, area, viewport);
            }
            root
        }
    }
}

/// Procura o nó (painel ou grupo de abas) que contém `anchor` e o troca por `Split vertical [nó, novo]`.
fn wrap_below(node: &mut LayoutNode, anchor: &str, new: LayoutNode, px: f32, h: f32) -> bool {
    let hit = match node {
        LayoutNode::Pane { id } => id == anchor,
        LayoutNode::Tabs { children, .. } => children.iter().any(|c| c.contains(anchor)),
        LayoutNode::Split { .. } => false,
    };
    if hit {
        let old = std::mem::replace(node, LayoutNode::pane(""));
        *node = LayoutNode::split(true, vec![((h - px).max(100.0), old), (px, new)]);
        return true;
    }
    if let LayoutNode::Split { children, .. } = node {
        for c in children.iter_mut() {
            if c.node.contains(anchor) {
                return wrap_below(&mut c.node, anchor, new, px, h);
            }
        }
    }
    false
}

// ----------------------------------------------------------------------------------------- padrão por px

/// Um item de um `Split` padrão: `px = Some(n)` ocupa `n` pixels; `None` divide o que sobra igualmente.
#[derive(Debug, Clone)]
pub struct Item {
    pub px: Option<f32>,
    pub spec: Spec,
}

/// Descrição do layout padrão em pixels (resolvida em `LayoutNode` com o tamanho real da área no primeiro quadro).
#[derive(Debug, Clone)]
pub enum Spec {
    Pane(&'static str),
    Split { vertical: bool, items: Vec<Item> },
    Tabs(Vec<Spec>),
}

pub fn fill(spec: Spec) -> Item {
    Item { px: None, spec }
}

pub fn sized(px: f32, spec: Spec) -> Item {
    Item { px: Some(px), spec }
}

pub fn row(items: Vec<Item>) -> Spec {
    Spec::Split { vertical: false, items }
}

pub fn column(items: Vec<Item>) -> Spec {
    Spec::Split { vertical: true, items }
}

/// Resolve `spec` numa área `(w, h)`: `shares` em pixels, de modo que as larguras/alturas fixas do layout antigo
/// (painéis do egui com `default_size`) se reproduzam exatamente na janela atual.
pub fn resolve(spec: &Spec, area: (f32, f32)) -> LayoutNode {
    match spec {
        Spec::Pane(id) => LayoutNode::pane(id),
        Spec::Tabs(kids) => LayoutNode::Tabs { children: kids.iter().map(|k| resolve(k, area)).collect(), active: 0 },
        Spec::Split { vertical, items } => {
            let extent = if *vertical { area.1 } else { area.0 };
            let fixed: f32 = items.iter().filter_map(|i| i.px).sum();
            let fills = items.iter().filter(|i| i.px.is_none()).count().max(1) as f32;
            let each = ((extent - fixed) / fills).max(extent * 0.15);
            let children = items
                .iter()
                .map(|i| {
                    let along = i.px.unwrap_or(each);
                    let child_area = if *vertical { (area.0, along) } else { (along, area.1) };
                    SplitChild { share: along, node: resolve(&i.spec, child_area) }
                })
                .collect();
            LayoutNode::Split { vertical: *vertical, children }
        }
    }
}

// ----------------------------------------------------------------------------------------- arquivo

/// Layout nomeado (Janela → Layouts → Salvar como…), amarrado a UM workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutPreset {
    pub name: String,
    pub workspace: String,
    pub layout: LayoutNode,
}

/// Conteúdo do arquivo de layouts do usuário e do campo `layouts` da sessão.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutStore {
    pub schema: u32,
    /// chave do workspace (`cenario`, `render`…) → layout
    #[serde(default)]
    pub workspaces: BTreeMap<String, LayoutNode>,
    #[serde(default)]
    pub presets: Vec<LayoutPreset>,
}

impl Default for LayoutStore {
    fn default() -> Self {
        Self { schema: LAYOUT_SCHEMA, workspaces: BTreeMap::new(), presets: Vec::new() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("esquema de layout {0} é mais novo que o suportado ({LAYOUT_SCHEMA})")]
    FutureSchema(u64),
    #[error("o arquivo de layout não é um objeto JSON")]
    NotAnObject,
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

/// Resultado de um carregamento tolerante.
#[derive(Debug, Default)]
pub struct Loaded {
    pub store: LayoutStore,
    /// Problemas recuperados (entradas descartadas etc.) — vão para o log/diálogo "Sobre".
    pub warnings: Vec<String>,
}

impl LayoutStore {
    /// Migra um valor JSON para o esquema atual. `schema` ausente (arquivo escrito à mão) = 0, tratado como 1
    /// com campos faltantes preenchidos; versões futuras são recusadas (o chamador usa os padrões e faz backup).
    pub fn migrate(mut v: serde_json::Value) -> Result<serde_json::Value, LayoutError> {
        let obj = v.as_object_mut().ok_or(LayoutError::NotAnObject)?;
        let schema = obj.get("schema").and_then(|s| s.as_u64()).unwrap_or(0);
        if schema > LAYOUT_SCHEMA as u64 {
            return Err(LayoutError::FutureSchema(schema));
        }
        if schema < 1 {
            // v0 → v1: sem `schema`; `workspaces`/`presets` podiam faltar
            obj.entry("workspaces").or_insert_with(|| serde_json::json!({}));
            obj.entry("presets").or_insert_with(|| serde_json::json!([]));
        }
        obj.insert("schema".into(), serde_json::json!(LAYOUT_SCHEMA));
        Ok(v)
    }

    /// Carrega validando entrada por entrada: um workspace/preset ruim é descartado (com aviso) sem perder os demais.
    pub fn parse(raw: &str) -> Result<Loaded, LayoutError> {
        let v: serde_json::Value = serde_json::from_str(raw)?;
        let v = Self::migrate(v)?;
        let mut out = Loaded::default();
        if let Some(ws) = v.get("workspaces").and_then(|w| w.as_object()) {
            for (k, node) in ws {
                match serde_json::from_value::<LayoutNode>(node.clone()) {
                    Ok(n) => {
                        out.store.workspaces.insert(k.clone(), n);
                    }
                    Err(e) => out.warnings.push(format!("layout de '{k}' descartado: {e}")),
                }
            }
        }
        if let Some(ps) = v.get("presets").and_then(|p| p.as_array()) {
            for p in ps {
                match serde_json::from_value::<LayoutPreset>(p.clone()) {
                    Ok(p) => out.store.presets.push(p),
                    Err(e) => out.warnings.push(format!("preset de layout descartado: {e}")),
                }
            }
        }
        Ok(out)
    }

    pub fn to_json(&self) -> Result<String, LayoutError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Salva atomicamente (grava ao lado e renomeia). Cria os diretórios.
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), LayoutError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_json()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Lê o arquivo. Arquivo ausente = padrão silencioso; ilegível/futuro = padrão + cópia `.bak` do original (para
    /// o usuário não perder nada quando o próximo salvamento sobrescrever) + aviso. Nunca entra em pânico.
    pub fn load_file(path: &std::path::Path) -> Loaded {
        let raw = match std::fs::read_to_string(path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
            Err(e) => {
                // bytes não-UTF8 também caem aqui: guarda cópia antes de qualquer sobrescrita
                let bak = path.with_extension("json.bak");
                let _ = std::fs::copy(path, &bak);
                return Loaded { store: Default::default(), warnings: vec![format!("layout ilegível ({}): {e}", path.display())] };
            }
        };
        match Self::parse(&raw) {
            Ok(l) => l,
            Err(e) => {
                let bak = path.with_extension("json.bak");
                let note = match std::fs::copy(path, &bak) {
                    Ok(_) => format!("; cópia em {}", bak.display()),
                    Err(_) => String::new(),
                };
                Loaded { store: Default::default(), warnings: vec![format!("layout ignorado ({}): {e}{note}", path.display())] }
            }
        }
    }

    /// Presets de um workspace, na ordem salva.
    pub fn presets_for<'a>(&'a self, ws: &'a str) -> impl Iterator<Item = &'a LayoutPreset> + 'a {
        self.presets.iter().filter(move |p| p.workspace == ws)
    }

    /// Salva (ou sobrescreve) o preset `name` do workspace.
    pub fn save_preset(&mut self, name: &str, ws: &str, layout: LayoutNode) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(p) = self.presets.iter_mut().find(|p| p.name == name && p.workspace == ws) {
            p.layout = layout;
        } else {
            self.presets.push(LayoutPreset { name: name.to_string(), workspace: ws.to_string(), layout });
        }
    }

    pub fn delete_preset(&mut self, name: &str, ws: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|p| !(p.name == name && p.workspace == ws));
        self.presets.len() != before
    }

    /// Mescla `other` por cima de `self` (entradas de `other` vencem; presets por nome+workspace).
    pub fn merge_over(&mut self, other: LayoutStore) {
        for (k, v) in other.workspaces {
            self.workspaces.insert(k, v);
        }
        for p in other.presets {
            self.save_preset(&p.name, &p.workspace, p.layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LayoutNode {
        LayoutNode::split(
            false,
            vec![
                (900.0, LayoutNode::split(true, vec![(500.0, LayoutNode::pane("ws.viewport")), (200.0, LayoutNode::pane("ws.timeline"))])),
                (340.0, LayoutNode::Tabs { children: vec![LayoutNode::pane("ws.outliner"), LayoutNode::pane("ws.history")], active: 1 }),
            ],
        )
    }

    const KNOWN: &[&str] = &["ws.viewport", "ws.timeline", "ws.outliner", "ws.history", "ws.details"];

    #[test]
    fn json_roundtrip_is_identity() {
        let n = sample();
        let back: LayoutNode = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
        let mut s = LayoutStore::default();
        s.workspaces.insert("cenario".into(), n.clone());
        s.save_preset("meu", "cenario", n);
        let l = LayoutStore::parse(&s.to_json().unwrap()).unwrap();
        assert_eq!(l.store, s);
        assert!(l.warnings.is_empty());
    }

    #[test]
    fn unknown_and_duplicate_panes_are_dropped_and_containers_collapse() {
        let n = LayoutNode::split(
            false,
            vec![
                (1.0, LayoutNode::pane("ws.viewport")),
                (1.0, LayoutNode::pane("painel.que.nao.existe")),
                (1.0, LayoutNode::pane("ws.viewport")),
                (1.0, LayoutNode::Tabs { children: vec![LayoutNode::pane("ws.details"), LayoutNode::pane("outro.desconhecido")], active: 7 }),
            ],
        );
        let s = n.sanitized(KNOWN).unwrap();
        // restam viewport + details (a aba com 1 filho colapsa), cada um uma vez
        assert_eq!(s.panes(), vec!["ws.viewport", "ws.details"]);
        assert!(matches!(&s, LayoutNode::Split { children, .. } if children.len() == 2));
        // tudo desconhecido = nada
        assert!(LayoutNode::pane("x").sanitized(KNOWN).is_none());
    }

    #[test]
    fn required_panes_must_survive() {
        let n = LayoutNode::pane("ws.details");
        assert!(n.clone().sanitized_with_required(KNOWN, &["ws.viewport"]).is_none());
        assert!(n.sanitized_with_required(KNOWN, &["ws.details"]).is_some());
    }

    #[test]
    fn invalid_shares_and_active_index_are_repaired() {
        let json = r#"{"kind":"split","vertical":false,"children":[
            {"share":null,"node":{"kind":"pane","id":"ws.viewport"}},
            {"share":-5.0,"node":{"kind":"pane","id":"ws.details"}},
            {"share":0.0000001,"node":{"kind":"tabs","active":99,"children":[{"kind":"pane","id":"ws.outliner"},{"kind":"pane","id":"ws.history"}]}}]}"#;
        let n: LayoutNode = serde_json::from_str(json).unwrap();
        let s = n.sanitized(KNOWN).unwrap();
        let LayoutNode::Split { children, .. } = &s else { panic!() };
        assert!(children.iter().all(|c| c.share.is_finite() && c.share > 0.0));
        let total: f32 = children.iter().map(|c| c.share).sum();
        assert!(children.iter().all(|c| c.share >= total * MIN_FRACTION - 1e-6), "nenhum filho some");
        let LayoutNode::Tabs { active, .. } = &children[2].node else { panic!() };
        assert_eq!(*active, 0);
    }

    #[test]
    fn active_tab_survives_pruning_when_possible() {
        let n = LayoutNode::Tabs { children: vec![LayoutNode::pane("ws.outliner"), LayoutNode::pane("fantasma"), LayoutNode::pane("ws.history")], active: 2 };
        let LayoutNode::Tabs { children, active } = n.sanitized(KNOWN).unwrap() else { panic!() };
        assert_eq!(children.len(), 2);
        assert_eq!(active, 1, "a aba ativa (history) foi reposicionada");
    }

    #[test]
    fn removing_a_pane_collapses_its_parents() {
        let n = sample().without("ws.timeline").unwrap();
        assert_eq!(n.panes(), vec!["ws.viewport", "ws.outliner", "ws.history"]);
        // o split vertical com 1 filho sumiu: o viewport é filho direto
        let LayoutNode::Split { vertical: false, children } = &n else { panic!() };
        assert!(matches!(children[0].node, LayoutNode::Pane { .. }));
        assert!(LayoutNode::pane("a").without("a").is_none());
    }

    #[test]
    fn insert_left_right_bottom_and_below_viewport() {
        let area = (1200.0, 700.0);
        let base = LayoutNode::split(false, vec![(860.0, LayoutNode::pane("ws.viewport")), (340.0, LayoutNode::pane("ws.details"))]);
        let r = insert_pane(Some(base.clone()), "ws.outliner", Home::Left, 200.0, area, "ws.viewport");
        assert_eq!(r.panes(), vec!["ws.outliner", "ws.viewport", "ws.details"]);
        let LayoutNode::Split { children, .. } = &r else { panic!() };
        let total: f32 = children.iter().map(|c| c.share).sum();
        assert!((total - 1200.0).abs() < 0.5 && (children[0].share - 200.0).abs() < 0.5, "px preservados: {children:?}");
        let r = insert_pane(Some(base.clone()), "ws.history", Home::Right, 200.0, area, "ws.viewport");
        assert_eq!(r.panes().last(), Some(&"ws.history"));
        let r = insert_pane(Some(base.clone()), "ws.timeline", Home::Bottom, 250.0, area, "ws.viewport");
        assert!(matches!(&r, LayoutNode::Split { vertical: true, .. }));
        assert_eq!(r.panes(), vec!["ws.viewport", "ws.details", "ws.timeline"]);
        let r = insert_pane(Some(base.clone()), "ws.timeline", Home::BelowViewport, 250.0, area, "ws.viewport");
        // o details continua alto: o split vertical está DENTRO do primeiro filho
        let LayoutNode::Split { vertical: false, children } = &r else { panic!() };
        assert!(matches!(&children[0].node, LayoutNode::Split { vertical: true, .. }));
        assert!(matches!(&children[1].node, LayoutNode::Pane { .. }));
        // já presente = sem efeito; sem raiz = o próprio painel
        assert_eq!(insert_pane(Some(base.clone()), "ws.details", Home::Left, 100.0, area, "ws.viewport"), base);
        assert_eq!(insert_pane(None, "ws.viewport", Home::Left, 100.0, area, "ws.viewport"), LayoutNode::pane("ws.viewport"));
    }

    #[test]
    fn resolve_reproduces_pixel_widths_and_heights() {
        // viewport + coluna direita de 340 px; embaixo do viewport uma faixa de 250 px
        let spec = row(vec![fill(column(vec![fill(Spec::Pane("v")), sized(250.0, Spec::Pane("t"))])), sized(340.0, Spec::Pane("d"))]);
        let n = resolve(&spec, (1200.0, 700.0));
        let LayoutNode::Split { vertical: false, children } = &n else { panic!() };
        assert_eq!(children[1].share, 340.0);
        assert_eq!(children[0].share, 860.0);
        let LayoutNode::Split { vertical: true, children: v } = &children[0].node else { panic!() };
        assert_eq!((v[0].share, v[1].share), (450.0, 250.0));
    }

    #[test]
    fn migration_handles_missing_schema_and_refuses_the_future() {
        // arquivo escrito à mão, sem `schema`/`presets`
        let l = LayoutStore::parse(r#"{"workspaces":{"render":{"kind":"pane","id":"render.viewport"}}}"#).unwrap();
        assert_eq!(l.store.schema, LAYOUT_SCHEMA);
        assert!(l.store.workspaces.contains_key("render") && l.store.presets.is_empty());
        let e = LayoutStore::parse(r#"{"schema":99,"workspaces":{}}"#).unwrap_err();
        assert!(matches!(e, LayoutError::FutureSchema(99)), "{e}");
        assert!(matches!(LayoutStore::parse("[1,2]").unwrap_err(), LayoutError::NotAnObject));
        assert!(LayoutStore::parse("{ isto nao e json").is_err());
    }

    #[test]
    fn a_bad_entry_is_dropped_but_the_rest_loads() {
        let raw = r#"{"schema":1,"workspaces":{"a":{"kind":"pane","id":"x"},"b":{"kind":"bolo"},"c":42},
                      "presets":[{"name":"ok","workspace":"a","layout":{"kind":"pane","id":"x"}},{"name":"ruim"}]}"#;
        let l = LayoutStore::parse(raw).unwrap();
        assert_eq!(l.store.workspaces.len(), 1);
        assert_eq!(l.store.presets.len(), 1);
        assert_eq!(l.warnings.len(), 3, "{:?}", l.warnings);
    }

    #[test]
    fn corrupted_file_falls_back_without_panic_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("anigo_layout_corrupt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("layouts.json");
        std::fs::write(&p, "{{{ lixo").unwrap();
        let l = LayoutStore::load_file(&p);
        assert!(l.store.workspaces.is_empty());
        assert_eq!(l.warnings.len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("layouts.json.bak")).unwrap(), "{{{ lixo");
        // bytes não-UTF8 também
        std::fs::write(&p, [0xff, 0xfe, 0x00, 0x9f]).unwrap();
        assert!(!LayoutStore::load_file(&p).warnings.is_empty());
        // ausente: silencioso
        assert!(LayoutStore::load_file(&dir.join("nao_existe.json")).warnings.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_roundtrip_is_atomic_and_creates_dirs() {
        let root = std::env::temp_dir().join(format!("anigo_layout_rt_{}", std::process::id()));
        let p = root.join("a").join("b").join("layouts.json");
        let mut s = LayoutStore::default();
        s.workspaces.insert("render".into(), sample());
        s.save_to(&p).unwrap();
        assert!(!p.with_extension("tmp").exists());
        assert_eq!(LayoutStore::load_file(&p).store, s);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn presets_save_overwrite_delete_and_merge() {
        let mut s = LayoutStore::default();
        s.save_preset("  ", "render", sample());
        assert!(s.presets.is_empty(), "nome vazio é ignorado");
        s.save_preset("A", "render", sample());
        s.save_preset("A", "cenario", LayoutNode::pane("x"));
        s.save_preset("A", "render", LayoutNode::pane("y"));
        assert_eq!(s.presets.len(), 2);
        assert_eq!(s.presets_for("render").count(), 1);
        assert_eq!(s.presets_for("render").next().unwrap().layout, LayoutNode::pane("y"));
        let mut o = LayoutStore::default();
        o.workspaces.insert("render".into(), LayoutNode::pane("z"));
        o.save_preset("B", "render", LayoutNode::pane("w"));
        s.merge_over(o);
        assert_eq!(s.workspaces["render"], LayoutNode::pane("z"));
        assert_eq!(s.presets_for("render").count(), 2);
        assert!(s.delete_preset("A", "render") && !s.delete_preset("A", "render"));
    }
}
