//! Camada de dados da Biblioteca (sem egui): consultas paginadas por keyset em threads de fundo,
//! facetas (contagens da árvore lateral), detalhes e comandos de mutação.
//!
//! Regra: a UI nunca espera o SQLite. Ela manda pedidos (`QueryWorker::request_page`, `MiscWorker::*`) e
//! recolhe respostas a cada quadro. Respostas carregam a *geração* da consulta; o que chega de uma
//! geração velha é descartado.

use anigo_assets::*;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::sync::Arc;

/// Itens por página (igual ao padrão do backend).
pub const PAGE: usize = 200;

/// Acorda a UI quando chega resposta de uma thread de fundo (em testes é um no-op).
pub type Wake = Arc<dyn Fn() + Send + Sync>;

// ---------------------------------------------------------------------------------------------
// Filtros -> Query
// ---------------------------------------------------------------------------------------------

/// O que a árvore lateral seleciona.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    All,
    Favorites,
    Trash,
    Missing,
    Collection(CollectionId),
    Tag(String),
    Root(RootId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filters {
    pub section: Section,
    pub text: String,
    pub kinds: Vec<AssetKind>,
    pub min_rating: u8,
    pub sort: Sort,
    /// `true` quando o usuário escolheu a ordenação (senão a busca por texto usa relevância).
    pub user_sorted: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self { section: Section::All, text: String::new(), kinds: Vec::new(), min_rating: 0, sort: Sort::default(), user_sorted: false }
    }
}

impl Filters {
    pub fn to_query(&self) -> Query {
        let mut q = Query { limit: PAGE, ..Default::default() };
        let text = self.text.trim();
        if !text.is_empty() {
            q.text = Some(text.to_string());
        }
        q.kinds = self.kinds.clone();
        if self.min_rating > 0 {
            q.min_rating = Some(self.min_rating);
        }
        q.sort = self.sort;
        if !self.user_sorted && q.text.is_some() && q.sort.by == SortBy::Name {
            q.sort = Sort { by: SortBy::Relevance, desc: false };
        }
        match &self.section {
            Section::All => {}
            Section::Favorites => q.favorite = Some(true),
            Section::Trash => q.state = StateFilter::Trashed,
            Section::Missing => q.state = StateFilter::Missing,
            Section::Collection(c) => q.collection = Some(*c),
            Section::Tag(t) => q.tags = vec![t.clone()],
            Section::Root(r) => q.root = Some(*r),
        }
        q
    }

    /// Há algum filtro além da seção (texto, tipo, nota)?
    pub fn has_refinement(&self) -> bool {
        !self.text.trim().is_empty() || !self.kinds.is_empty() || self.min_rating > 0
    }
}

// ---------------------------------------------------------------------------------------------
// Paginador keyset (testável sem threads)
// ---------------------------------------------------------------------------------------------

/// Guarda o cursor de cada página já visitada. Pedir a página N caminha a partir do maior cursor
/// conhecido; as páginas intermediárias voltam junto (a UI as guarda, são baratas).
pub struct Pager {
    lib: Library,
    query: Query,
    /// `starts[i]` = cursor para buscar a página `i` (`starts[0] = None`).
    starts: Vec<Option<Cursor>>,
    /// Número de páginas, quando o fim da lista já foi visto.
    end: Option<usize>,
}

impl Pager {
    pub fn new(lib: Library, mut query: Query) -> Self {
        query.limit = PAGE;
        query.after = None;
        query.with_total = false;
        Self { lib, query, starts: vec![None], end: None }
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn total(&self) -> Result<u64> {
        self.lib.count(&self.query)
    }

    /// Devolve a página `idx` e as intermediárias percorridas (índice, linhas). Vazio além do fim.
    pub fn page(&mut self, idx: usize) -> Result<Vec<(usize, Vec<AssetRow>)>> {
        let mut out = Vec::new();
        if self.end.is_some_and(|e| idx >= e) {
            return Ok(out);
        }
        loop {
            let cur = idx.min(self.starts.len() - 1);
            let mut q = self.query.clone();
            q.after = self.starts[cur].clone();
            let page = self.lib.query(&q)?;
            let rows = page.items;
            match page.next {
                Some(c) => {
                    if self.starts.len() == cur + 1 {
                        self.starts.push(Some(c));
                    }
                }
                None => {
                    self.end = Some(cur + 1);
                }
            }
            if !rows.is_empty() || cur == idx {
                out.push((cur, rows));
            }
            if cur == idx || self.end.is_some_and(|e| idx >= e) {
                return Ok(out);
            }
        }
    }

    /// Ids dos itens `[from, to]` (inclusive) da lista atual.
    pub fn ids_in_range(&mut self, from: usize, to: usize) -> Result<Vec<AssetId>> {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        let mut ids = Vec::new();
        for p in (from / PAGE)..=(to / PAGE) {
            let pages = self.page(p)?;
            if let Some((_, rows)) = pages.into_iter().find(|(i, _)| *i == p) {
                for (k, r) in rows.iter().enumerate() {
                    let n = p * PAGE + k;
                    if n >= from && n <= to {
                        ids.push(r.id);
                    }
                }
            }
        }
        Ok(ids)
    }
}

// ---------------------------------------------------------------------------------------------
// Worker de consulta
// ---------------------------------------------------------------------------------------------

enum QReq {
    SetQuery { gen: u64, query: Query },
    Page { gen: u64, idx: usize },
    Range { gen: u64, from: usize, to: usize, tag: u32 },
}

pub enum QResp {
    Total { gen: u64, total: u64 },
    Pages { gen: u64, pages: Vec<(usize, Vec<AssetRow>)> },
    Range { gen: u64, tag: u32, ids: Vec<AssetId> },
    Error { gen: u64, message: String },
}

/// Thread dedicada às consultas da grade.
pub struct QueryWorker {
    tx: Sender<QReq>,
    pub rx: Receiver<QResp>,
}

impl QueryWorker {
    pub fn spawn(lib: Library, wake: Wake) -> Self {
        let (tx, rx_req) = unbounded::<QReq>();
        let (tx_resp, rx) = unbounded::<QResp>();
        std::thread::Builder::new()
            .name("anigo-lib-query".into())
            .spawn(move || {
                let mut pager: Option<(u64, Pager)> = None;
                while let Ok(first) = rx_req.recv() {
                    // junta o que já chegou: só a última SetQuery e os pedidos da geração atual valem
                    let mut batch = vec![first];
                    batch.extend(rx_req.try_iter());
                    let latest = batch.iter().filter_map(|r| if let QReq::SetQuery { gen, .. } = r { Some(*gen) } else { None }).max();
                    for req in batch {
                        match req {
                            QReq::SetQuery { gen, query } => {
                                if latest.is_some_and(|l| gen < l) {
                                    continue;
                                }
                                let p = Pager::new(lib.clone(), query);
                                match p.total() {
                                    Ok(total) => {
                                        let _ = tx_resp.send(QResp::Total { gen, total });
                                    }
                                    Err(e) => {
                                        let _ = tx_resp.send(QResp::Error { gen, message: e.to_string() });
                                    }
                                }
                                pager = Some((gen, p));
                            }
                            QReq::Page { gen, idx } => {
                                let Some((g, p)) = pager.as_mut() else { continue };
                                if *g != gen {
                                    continue;
                                }
                                match p.page(idx) {
                                    Ok(pages) => {
                                        let _ = tx_resp.send(QResp::Pages { gen, pages });
                                    }
                                    Err(e) => {
                                        let _ = tx_resp.send(QResp::Error { gen, message: e.to_string() });
                                    }
                                }
                            }
                            QReq::Range { gen, from, to, tag } => {
                                let Some((g, p)) = pager.as_mut() else { continue };
                                if *g != gen {
                                    continue;
                                }
                                match p.ids_in_range(from, to) {
                                    Ok(ids) => {
                                        let _ = tx_resp.send(QResp::Range { gen, tag, ids });
                                    }
                                    Err(e) => {
                                        let _ = tx_resp.send(QResp::Error { gen, message: e.to_string() });
                                    }
                                }
                            }
                        }
                    }
                    wake();
                }
            })
            .expect("thread de consulta");
        Self { tx, rx }
    }

    pub fn set_query(&self, gen: u64, query: Query) {
        let _ = self.tx.send(QReq::SetQuery { gen, query });
    }
    pub fn request_page(&self, gen: u64, idx: usize) {
        let _ = self.tx.send(QReq::Page { gen, idx });
    }
    pub fn request_range(&self, gen: u64, from: usize, to: usize, tag: u32) {
        let _ = self.tx.send(QReq::Range { gen, from, to, tag });
    }
}

// ---------------------------------------------------------------------------------------------
// Lista paginada no lado da UI
// ---------------------------------------------------------------------------------------------

/// Cache de páginas da consulta corrente (o que a grade desenha).
pub struct PagedList {
    pub gen: u64,
    pub total: Option<u64>,
    /// Último total conhecido (mostrado enquanto o novo não chega).
    pub last_total: u64,
    /// `idx -> (geração, linhas)`.
    pages: HashMap<usize, (u64, Arc<Vec<AssetRow>>)>,
    requested: HashMap<usize, u64>,
    pub error: Option<String>,
}

impl PagedList {
    pub fn new() -> Self {
        Self { gen: 0, total: None, last_total: 0, pages: HashMap::new(), requested: HashMap::new(), error: None }
    }

    /// Nova consulta: zera páginas (não mistura itens de filtros diferentes).
    pub fn reset(&mut self) -> u64 {
        self.gen += 1;
        self.pages.clear();
        self.requested.clear();
        self.total = None;
        self.error = None;
        self.gen
    }

    /// Mesma consulta, dados possivelmente alterados: mantém as páginas visíveis até as novas chegarem.
    pub fn refresh(&mut self) -> u64 {
        self.gen += 1;
        self.requested.clear();
        self.total = None;
        self.error = None;
        self.gen
    }

    pub fn total_or_last(&self) -> usize {
        self.total.unwrap_or(self.last_total) as usize
    }

    pub fn apply(&mut self, resp: QResp) {
        match resp {
            QResp::Total { gen, total } if gen == self.gen => {
                self.total = Some(total);
                self.last_total = total;
            }
            QResp::Pages { gen, pages } if gen == self.gen => {
                for (idx, rows) in pages {
                    self.pages.insert(idx, (gen, Arc::new(rows)));
                }
            }
            QResp::Error { gen, message } if gen == self.gen => self.error = Some(message),
            _ => {}
        }
    }

    /// Linha `n` da lista, se a página está carregada (pode ser de uma geração anterior).
    pub fn row(&self, n: usize) -> Option<&AssetRow> {
        self.pages.get(&(n / PAGE)).and_then(|(_, rows)| rows.get(n % PAGE))
    }

    /// Edita uma linha em cache (atualização otimista).
    pub fn update_row(&mut self, id: &AssetId, f: impl Fn(&mut AssetRow)) {
        for (_, rows) in self.pages.values_mut() {
            if rows.iter().any(|r| r.id == *id) {
                let v = Arc::make_mut(rows);
                for r in v.iter_mut().filter(|r| r.id == *id) {
                    f(r);
                }
            }
        }
    }

    /// Páginas que cobrem `[first, last]` e ainda não foram pedidas/recebidas nesta geração.
    pub fn missing_pages(&mut self, first: usize, last: usize) -> Vec<usize> {
        let total = self.total_or_last();
        if total == 0 && self.total.is_some() {
            return Vec::new();
        }
        let max_page = if total == 0 { 0 } else { (total - 1) / PAGE };
        let mut out = Vec::new();
        for p in (first / PAGE)..=(last / PAGE).min(max_page) {
            let fresh = self.pages.get(&p).is_some_and(|(g, _)| *g == self.gen);
            if !fresh && self.requested.get(&p) != Some(&self.gen) {
                self.requested.insert(p, self.gen);
                out.push(p);
            }
        }
        out
    }

    /// Descarta páginas longe da janela visível (memória constante com 50 000 itens).
    pub fn evict_far(&mut self, first: usize, last: usize, keep_pages: usize) {
        if self.pages.len() <= keep_pages {
            return;
        }
        let center = (first + last) / 2 / PAGE;
        let mut idxs: Vec<usize> = self.pages.keys().copied().collect();
        idxs.sort_by_key(|i| i.abs_diff(center));
        for i in idxs.into_iter().skip(keep_pages) {
            self.pages.remove(&i);
            self.requested.remove(&i);
        }
    }

    pub fn loaded_pages(&self) -> usize {
        self.pages.len()
    }

    /// O total é conhecido e todas as páginas pedidas nesta geração já chegaram.
    pub fn is_settled(&self) -> bool {
        self.total.is_some() && self.requested.iter().all(|(p, g)| *g != self.gen || self.pages.get(p).is_some_and(|(pg, _)| pg == g))
    }
}

impl Default for PagedList {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// Facetas (árvore lateral)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TagNode {
    pub path: String,
    pub name: String,
    pub depth: usize,
    /// Ativos com esta tag ou qualquer descendente.
    pub total: u64,
    pub has_children: bool,
    pub color: String,
}

#[derive(Debug, Clone, Default)]
pub struct Facets {
    pub roots: Vec<RootInfo>,
    /// Em ordem de árvore (pai antes dos filhos, irmãos por nome).
    pub tags: Vec<TagNode>,
    /// Coleção + quantidade de itens ativos.
    pub collections: Vec<(CollectionDef, u64)>,
    pub n_all: u64,
    pub n_favorites: u64,
    pub n_trash: u64,
    pub n_missing: u64,
    pub dup_groups: usize,
    pub dup_reclaimable: u64,
    pub kind_counts: Vec<(AssetKind, u64)>,
}

/// Monta a árvore de tags a partir dos caminhos (sintetiza ancestrais que não existem como tag própria).
pub fn build_tag_tree(tags: &[TagInfo], total_of: &dyn Fn(&str) -> u64) -> Vec<TagNode> {
    use std::collections::BTreeMap;
    let mut all: BTreeMap<String, String> = BTreeMap::new(); // path -> cor
    for t in tags {
        let mut acc = String::new();
        for seg in t.path.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            all.entry(acc.clone()).or_default();
        }
        all.insert(t.path.clone(), t.color.clone());
    }
    // ordena por segmentos ("a", "a/b", "a-x"): a ordem lexicográfica de strings separaria pais e filhos
    let mut sorted: Vec<String> = all.keys().cloned().collect();
    sorted.sort_by(|a, b| a.split('/').collect::<Vec<_>>().cmp(&b.split('/').collect::<Vec<_>>()));
    sorted
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let depth = p.matches('/').count();
            let has_children = sorted.get(i + 1).is_some_and(|n| n.starts_with(&format!("{p}/")));
            TagNode {
                path: p.clone(),
                name: p.rsplit('/').next().unwrap_or(p).to_string(),
                depth,
                total: total_of(p),
                has_children,
                color: all.get(p).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

pub fn compute_facets(lib: &Library) -> Result<Facets> {
    let count = |q: Query| lib.count(&q);
    let roots = lib.roots()?;
    let tags = lib.tags()?;
    let exact: HashMap<String, u64> = tags.iter().map(|t| (t.path.clone(), t.count)).collect();
    // Pais precisam de contagem exata (união dos descendentes); folhas usam a contagem da própria tag.
    let mut nonleaf: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in &tags {
        let segs: Vec<&str> = t.path.split('/').collect();
        let mut acc = String::new();
        for s in &segs[..segs.len().saturating_sub(1)] {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(s);
            nonleaf.insert(acc.clone());
        }
    }
    let budget_exact = nonleaf.len() <= 400;
    let total_of = |p: &str| -> u64 {
        if nonleaf.contains(p) {
            if budget_exact {
                count(Query { tags: vec![p.to_string()], ..Default::default() }).unwrap_or(0)
            } else {
                let prefix = format!("{p}/");
                exact.iter().filter(|(k, _)| k.as_str() == p || k.starts_with(&prefix)).map(|(_, v)| *v).sum()
            }
        } else {
            *exact.get(p).unwrap_or(&0)
        }
    };
    let tag_nodes = build_tag_tree(&tags, &total_of);
    let mut collections = Vec::new();
    for c in lib.collections()? {
        let n = if c.is_smart() { count(Query { collection: Some(c.id), ..Default::default() }).unwrap_or(0) } else { c.item_count };
        collections.push((c, n));
    }
    let n_all = count(Query::default())?;
    let n_favorites = count(Query { favorite: Some(true), ..Default::default() })?;
    let n_trash = count(Query { state: StateFilter::Trashed, ..Default::default() })?;
    let n_missing = count(Query { state: StateFilter::Missing, ..Default::default() })?;
    let dups = lib.find_duplicates(5000)?;
    let dup_reclaimable = dups.iter().map(|g| g.reclaimable_bytes()).sum();
    let mut kind_counts = Vec::new();
    for k in AssetKind::ALL {
        let n = count(Query { kinds: vec![*k], ..Default::default() })?;
        if n > 0 {
            kind_counts.push((*k, n));
        }
    }
    Ok(Facets { roots, tags: tag_nodes, collections, n_all, n_favorites, n_trash, n_missing, dup_groups: dups.len(), dup_reclaimable, kind_counts })
}

// ---------------------------------------------------------------------------------------------
// Detalhes de um ativo
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DetailData {
    pub detail: AssetDetail,
    pub files: Option<ResolvedFiles>,
    pub root_name: Option<String>,
    pub root_path: Option<std::path::PathBuf>,
    /// `(arquivo, nome)` dos materiais canônicos.
    pub materials: Vec<(String, String)>,
    /// Conteúdo dos primeiros materiais.
    pub loaded_materials: Vec<(String, ToonMaterial)>,
    pub collections: Vec<CollectionId>,
    pub deps: Vec<(DependencyRef, Option<AssetRow>)>,
    pub used_by: Vec<AssetRow>,
}

pub fn load_detail(lib: &Library, id: &AssetId) -> Result<DetailData> {
    let detail = lib.get(id)?;
    let files = lib.resolve_files(id).ok();
    let (root_name, root_path) = lib.root(detail.row.root_id).map(|r| (Some(r.name), Some(r.path))).unwrap_or((None, None));
    let materials = lib.asset_materials(id).unwrap_or_default();
    let loaded_materials = materials.iter().take(8).filter_map(|(f, _)| lib.load_material(id, f).ok().map(|m| (f.clone(), m))).collect();
    let collections = lib.collections_of(id).unwrap_or_default();
    let deps = detail.dependencies.iter().map(|d| (d.clone(), lib.row(&d.id).ok().flatten())).collect();
    let used_by = detail.used_by.iter().filter_map(|u| lib.row(u).ok().flatten()).collect();
    Ok(DetailData { detail, files, root_name, root_path, materials, loaded_materials, collections, deps, used_by })
}

// ---------------------------------------------------------------------------------------------
// Worker "misc": facetas, detalhes e comandos de mutação
// ---------------------------------------------------------------------------------------------

/// O que recarregar depois de um comando.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refresh {
    pub rows: bool,
    pub facets: bool,
    pub detail: bool,
}

impl Refresh {
    pub const NONE: Refresh = Refresh { rows: false, facets: false, detail: false };
    pub const ALL: Refresh = Refresh { rows: true, facets: true, detail: true };
    pub const DETAIL: Refresh = Refresh { rows: false, facets: false, detail: true };
    pub const ROWS_FACETS: Refresh = Refresh { rows: true, facets: true, detail: false };
    pub const ROWS_DETAIL: Refresh = Refresh { rows: true, facets: false, detail: true };
}

type CmdFn = Box<dyn FnOnce(&Library) -> std::result::Result<String, String> + Send>;

enum MReq {
    Facets,
    Detail(AssetId),
    Cmd { label: String, refresh: Refresh, f: CmdFn },
}

pub enum MResp {
    Facets(Box<Facets>),
    Detail(AssetId, Box<std::result::Result<DetailData, String>>),
    CmdDone { label: String, refresh: Refresh, result: std::result::Result<String, String> },
}

pub struct MiscWorker {
    tx: Sender<MReq>,
    pub rx: Receiver<MResp>,
}

impl MiscWorker {
    pub fn spawn(lib: Library, wake: Wake) -> Self {
        let (tx, rx_req) = unbounded::<MReq>();
        let (tx_resp, rx) = unbounded::<MResp>();
        std::thread::Builder::new()
            .name("anigo-lib-misc".into())
            .spawn(move || {
                while let Ok(first) = rx_req.recv() {
                    let mut batch = vec![first];
                    batch.extend(rx_req.try_iter());
                    // só as últimas facetas e o último detalhe interessam
                    let last_facets = batch.iter().rposition(|r| matches!(r, MReq::Facets));
                    let last_detail = batch.iter().rposition(|r| matches!(r, MReq::Detail(_)));
                    for (i, req) in batch.into_iter().enumerate() {
                        match req {
                            MReq::Facets => {
                                if Some(i) == last_facets {
                                    match compute_facets(&lib) {
                                        Ok(f) => {
                                            let _ = tx_resp.send(MResp::Facets(Box::new(f)));
                                        }
                                        Err(e) => tracing::warn!("facetas da biblioteca: {e}"),
                                    }
                                }
                            }
                            MReq::Detail(id) => {
                                if Some(i) == last_detail {
                                    let r = load_detail(&lib, &id).map_err(|e| e.to_string());
                                    let _ = tx_resp.send(MResp::Detail(id, Box::new(r)));
                                }
                            }
                            MReq::Cmd { label, refresh, f } => {
                                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&lib))).unwrap_or_else(|_| Err("panic no comando".to_string()));
                                let _ = tx_resp.send(MResp::CmdDone { label, refresh, result });
                            }
                        }
                    }
                    wake();
                }
            })
            .expect("thread misc");
        Self { tx, rx }
    }

    pub fn request_facets(&self) {
        let _ = self.tx.send(MReq::Facets);
    }
    pub fn request_detail(&self, id: AssetId) {
        let _ = self.tx.send(MReq::Detail(id));
    }
    pub fn command(&self, label: impl Into<String>, refresh: Refresh, f: impl FnOnce(&Library) -> std::result::Result<String, String> + Send + 'static) {
        let _ = self.tx.send(MReq::Cmd { label: label.into(), refresh, f: Box::new(f) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_ui::synth;

    #[test]
    fn filters_map_to_queries() {
        let mut f = Filters::default();
        assert_eq!(f.to_query().state, StateFilter::Active);
        f.section = Section::Favorites;
        assert_eq!(f.to_query().favorite, Some(true));
        f.section = Section::Trash;
        assert_eq!(f.to_query().state, StateFilter::Trashed);
        f.section = Section::Tag("estilo/cel".into());
        assert_eq!(f.to_query().tags, vec!["estilo/cel".to_string()]);
        f.text = "  cabelo -vestido ".into();
        f.kinds = vec![AssetKind::Hair];
        f.min_rating = 3;
        let q = f.to_query();
        assert_eq!(q.text.as_deref(), Some("cabelo -vestido"));
        assert_eq!(q.min_rating, Some(3));
        // texto sem ordenação explícita => relevância; com ordenação explícita respeita a escolha
        assert_eq!(q.sort.by, SortBy::Relevance);
        f.user_sorted = true;
        assert_eq!(f.to_query().sort.by, SortBy::Name);
    }

    #[test]
    fn tag_tree_synthesizes_ancestors_and_orders_by_segments() {
        let tags = vec![
            TagInfo { path: "estilo/cel/noir".into(), color: String::new(), count: 2 },
            TagInfo { path: "estilo-x".into(), color: String::new(), count: 1 },
            TagInfo { path: "categoria/roupa".into(), color: String::new(), count: 3 },
        ];
        let tree = build_tag_tree(&tags, &|_| 7);
        let paths: Vec<&str> = tree.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, ["categoria", "categoria/roupa", "estilo", "estilo/cel", "estilo/cel/noir", "estilo-x"]);
        assert!(tree[0].has_children && !tree[1].has_children);
        assert_eq!(tree[4].depth, 2);
    }

    #[test]
    fn pager_walks_keyset_pages_and_reports_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let lib = synth::build_synthetic(dir.path(), 1234, 7).unwrap();
        let mut pager = Pager::new(lib.clone(), Filters::default().to_query());
        assert_eq!(pager.total().unwrap(), 1234);
        // salto direto para a página 5: percorre 0..=5 e devolve todas
        let pages = pager.page(5).unwrap();
        assert_eq!(pages.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
        assert!(pages.iter().all(|(_, r)| r.len() == PAGE));
        // página já conhecida: só ela
        let again = pager.page(3).unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].0, 3);
        // última página (1234 = 6*200 + 34) e além do fim
        let last = pager.page(6).unwrap();
        assert_eq!(last.last().unwrap().1.len(), 34);
        assert!(pager.page(7).unwrap().is_empty());
        // sem repetição nem omissão
        let mut seen = std::collections::HashSet::new();
        let mut p2 = Pager::new(lib, Filters::default().to_query());
        for i in 0..=6 {
            for (_, rows) in p2.page(i).unwrap() {
                for r in rows {
                    seen.insert(r.id);
                }
            }
        }
        assert_eq!(seen.len(), 1234);
    }

    #[test]
    fn range_ids_cross_pages() {
        let dir = tempfile::tempdir().unwrap();
        let lib = synth::build_synthetic(dir.path(), 500, 3).unwrap();
        let mut pager = Pager::new(lib.clone(), Filters::default().to_query());
        let ids = pager.ids_in_range(190, 410).unwrap();
        assert_eq!(ids.len(), 221);
        let all = lib.query_ids(&Filters::default().to_query(), 500).unwrap();
        assert_eq!(ids[0], all[190]);
        assert_eq!(*ids.last().unwrap(), all[410]);
        // ordem invertida dá o mesmo conjunto
        assert_eq!(pager.ids_in_range(410, 190).unwrap(), ids);
    }

    #[test]
    fn paged_list_discards_stale_generations_and_tracks_missing_pages() {
        let mut l = PagedList::new();
        let g1 = l.reset();
        l.apply(QResp::Total { gen: g1, total: 1000 });
        assert_eq!(l.missing_pages(0, 450), vec![0, 1, 2]);
        assert!(l.missing_pages(0, 450).is_empty(), "já pedidas");
        let g2 = l.reset();
        // resposta da geração 1 chega depois do reset: ignorada
        l.apply(QResp::Total { gen: g1, total: 5 });
        assert!(l.total.is_none());
        l.apply(QResp::Total { gen: g2, total: 1000 });
        assert_eq!(l.total, Some(1000));
        assert_eq!(l.missing_pages(0, 199), vec![0]);
        assert!(!l.is_settled());
    }

    #[test]
    fn eviction_keeps_pages_nearest_to_the_viewport() {
        let mut l = PagedList::new();
        let g = l.reset();
        l.apply(QResp::Total { gen: g, total: 20_000 });
        for p in 0..30 {
            l.apply(QResp::Pages { gen: g, pages: vec![(p, Vec::new())] });
        }
        l.evict_far(10 * PAGE, 11 * PAGE, 8);
        assert_eq!(l.loaded_pages(), 8);
        assert!(l.pages.contains_key(&10) && l.pages.contains_key(&11));
        assert!(!l.pages.contains_key(&0));
    }

    #[test]
    fn workers_deliver_pages_and_drop_stale_generations() {
        let dir = tempfile::tempdir().unwrap();
        let lib = synth::build_synthetic(dir.path(), 450, 1).unwrap();
        let w = QueryWorker::spawn(lib, Arc::new(|| {}));
        let mut list = PagedList::new();
        let g = list.reset();
        w.set_query(g, Filters::default().to_query());
        // uma nova consulta logo em seguida invalida a primeira
        let g2 = list.reset();
        w.set_query(g2, Filters { section: Section::Favorites, ..Default::default() }.to_query());
        w.request_page(g2, 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !(list.total.is_some() && list.is_settled()) && std::time::Instant::now() < deadline {
            for r in w.rx.try_iter() {
                list.apply(r);
            }
            list.missing_pages(0, 10);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(list.total.is_some(), "total da consulta de favoritos nunca chegou");
        assert!(list.total.unwrap() < 450);
        assert!(list.row(0).is_some_and(|r| r.favorite));
    }
}
