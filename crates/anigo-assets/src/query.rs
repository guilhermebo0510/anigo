//! Consulta ao indice: texto FTS5, tags hierarquicas, tipo, colecao, ordenacao e paginacao keyset.
//!
//! Todo valor do usuario vai por parametro SQL; o unico texto interpolado vem de enums internos.
//! Colecoes inteligentes guardam um [`Query`] serializado (JSON), nunca SQL cru.

use rusqlite::types::Value;
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};
use crate::library::Library;
use crate::model::*;
use crate::store::{normalize_tag, row_to_asset, ROW_COLS, ROW_FROM};
use crate::util::{hex_decode, hex_encode};

/// Filtro de estado. O padrao e so ativos `Active`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateFilter {
    #[default]
    Active,
    Trashed,
    Missing,
    Corrupt,
    /// Qualquer estado (inclui lixeira).
    Any,
}

/// Campo de ordenacao.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Name,
    ImportedAt,
    ModifiedAt,
    Size,
    Rating,
    Tris,
    /// bm25 do FTS5 (so tem efeito com `text`; sem texto cai para `Name`).
    Relevance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Sort {
    pub by: SortBy,
    pub desc: bool,
}

/// Cursor opaco de paginacao keyset. Invalida se o indice for reconstruido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor(pub String);

/// Pagina de resultados.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Passe em `Query::after` para a proxima pagina; `None` no fim.
    pub next: Option<Cursor>,
    /// Total de itens que casam (so se `Query::with_total`).
    pub total: Option<u64>,
}

/// Consulta. `Default` = todos os ativos ativos, por nome, 200 por pagina.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    /// Texto livre (FTS5). Veja [`parse_text`] para a sintaxe.
    pub text: Option<String>,
    pub kinds: Vec<AssetKind>,
    /// O ativo precisa ter TODAS estas tags (uma tag pai casa com seus descendentes).
    pub tags: Vec<String>,
    /// O ativo precisa ter PELO MENOS UMA destas tags.
    pub any_tags: Vec<String>,
    pub collection: Option<CollectionId>,
    pub favorite: Option<bool>,
    pub min_rating: Option<u8>,
    pub formats: Vec<String>,
    pub root: Option<RootId>,
    pub storage: Option<StorageMode>,
    pub min_tris: Option<u64>,
    pub max_tris: Option<u64>,
    pub state: StateFilter,
    pub sort: Sort,
    /// Itens por pagina (0 = 200; maximo 1000).
    pub limit: usize,
    pub after: Option<Cursor>,
    /// Calcula `Page::total` (custa um `COUNT(*)` extra).
    pub with_total: bool,
}

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;

// ---------------------------------------------------------------------------------------------
// Texto -> FTS5
// ---------------------------------------------------------------------------------------------

/// Resultado de [`parse_text`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFilter {
    /// Expressao FTS5 que o ativo deve satisfazer.
    pub positive: Option<String>,
    /// Expressao FTS5 que o ativo NAO pode satisfazer.
    pub negative: Option<String>,
}

fn alnum_segments(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric()))
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

fn column_for(prefix: &str) -> Option<&'static str> {
    match prefix.to_lowercase().as_str() {
        "name" | "nome" => Some("name"),
        "tag" | "tags" => Some("tags"),
        "author" | "autor" => Some("author"),
        "desc" | "description" | "descricao" => Some("description"),
        _ => None,
    }
}

/// Converte texto digitado em expressoes FTS5 SEGURAS (nunca repassa o texto cru ao `MATCH`).
///
/// Sintaxe: palavras sao combinadas com E e casam por prefixo (`cabel` acha `cabelo`); `"frase exata"`;
/// `-palavra` exclui; `tag:noir`, `name:`, `author:`, `desc:` restringem a coluna. Pontuacao vira separador.
/// No maximo 32 termos e 512 caracteres sao considerados.
pub fn parse_text(input: &str) -> TextFilter {
    let input: String = input.chars().take(512).collect();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut pos: Vec<String> = Vec::new();
    let mut neg: Vec<String> = Vec::new();
    let mut terms = 0usize;

    while i < chars.len() && terms < 32 {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let negated = chars[i] == '-' && chars.get(i + 1).map(|c| !c.is_whitespace()).unwrap_or(false);
        if negated {
            i += 1;
        }
        // coluna?
        let mut column: Option<&'static str> = None;
        {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphabetic()) {
                j += 1;
            }
            if j > i && j < chars.len() && chars[j] == ':' {
                let word: String = chars[i..j].iter().collect();
                if let Some(c) = column_for(&word) {
                    column = Some(c);
                    i = j + 1;
                }
            }
        }
        // frase ou palavra
        let (raw, is_phrase): (String, bool) = if i < chars.len() && chars[i] == '"' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            if i < chars.len() {
                i += 1; // aspas de fechamento
            }
            (s, true)
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            (chars[start..i].iter().collect(), false)
        };
        let segs = alnum_segments(&raw);
        if segs.is_empty() {
            continue;
        }
        terms += segs.len();
        let prefix = |c: Option<&str>, body: String| match c {
            Some(col) => format!("{col} : {body}"),
            None => body,
        };
        let expr = if is_phrase && segs.len() > 1 {
            prefix(column, format!("\"{}\"", segs.join(" ")))
        } else {
            let parts: Vec<String> = segs
                .iter()
                .map(|s| {
                    let body = if s.chars().count() >= 2 { format!("\"{s}\"*") } else { format!("\"{s}\"") };
                    prefix(column, body)
                })
                .collect();
            parts.join(" AND ")
        };
        if negated {
            neg.push(format!("({expr})"));
        } else {
            pos.push(format!("({expr})"));
        }
    }
    TextFilter {
        positive: if pos.is_empty() { None } else { Some(pos.join(" AND ")) },
        negative: if neg.is_empty() { None } else { Some(neg.join(" OR ")) },
    }
}

// ---------------------------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum KeyVal {
    Int(i64),
    Real(f64),
    Text(String),
}

fn encode_cursor(k: &KeyVal, seq: i64) -> Cursor {
    let body = match k {
        KeyVal::Int(v) => format!("i:{v}"),
        KeyVal::Real(v) => format!("r:{:016x}", v.to_bits()),
        KeyVal::Text(t) => format!("t:{}", hex_encode(t.as_bytes())),
    };
    Cursor(format!("{body}:{seq}"))
}

fn decode_cursor(c: &Cursor) -> Result<(KeyVal, i64)> {
    let bad = || AssetError::InvalidInput(format!("cursor invalido: {:?}", c.0));
    let (head, seq) = c.0.rsplit_once(':').ok_or_else(bad)?;
    let seq: i64 = seq.parse().map_err(|_| bad())?;
    let (tag, val) = head.split_once(':').ok_or_else(bad)?;
    let key = match tag {
        "i" => KeyVal::Int(val.parse().map_err(|_| bad())?),
        "r" => KeyVal::Real(f64::from_bits(u64::from_str_radix(val, 16).map_err(|_| bad())?)),
        "t" => KeyVal::Text(String::from_utf8(hex_decode(val).ok_or_else(bad)?).map_err(|_| bad())?),
        _ => return Err(bad()),
    };
    Ok((key, seq))
}

// ---------------------------------------------------------------------------------------------
// Construcao do SQL
// ---------------------------------------------------------------------------------------------

struct SqlBuilder {
    wheres: Vec<String>,
    params: Vec<Value>,
}

impl SqlBuilder {
    fn new() -> Self {
        Self { wheres: Vec::new(), params: Vec::new() }
    }
    fn p(&mut self, v: impl Into<Value>) -> String {
        self.params.push(v.into());
        format!("?{}", self.params.len())
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn tag_exists_clause(b: &mut SqlBuilder, tag: &str) -> String {
    let exact = b.p(tag.to_string());
    let like = b.p(format!("{}/%", escape_like(tag)));
    format!(
        "EXISTS (SELECT 1 FROM asset_tag at JOIN tag t ON t.id = at.tag_id WHERE at.asset_seq = a.seq \
         AND (t.path = {exact} OR t.path LIKE {like} ESCAPE '\\'))"
    )
}

struct Built {
    sql: String,
    params: Vec<Value>,
    key_is_real: bool,
}

/// Une os filtros de uma colecao inteligente (`base`) com os do chamador (`user`).
/// `None` = a intersecao e vazia por construcao.
pub(crate) fn merge_smart(user: &Query, base: &Query) -> Option<Query> {
    let mut q = user.clone();
    q.collection = None;
    q.text = match (&user.text, &base.text) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (None, None) => None,
    };
    q.kinds = match (user.kinds.is_empty(), base.kinds.is_empty()) {
        (true, _) => base.kinds.clone(),
        (_, true) => user.kinds.clone(),
        _ => {
            let inter: Vec<AssetKind> = user.kinds.iter().copied().filter(|k| base.kinds.contains(k)).collect();
            if inter.is_empty() {
                return None;
            }
            inter
        }
    };
    q.formats = match (user.formats.is_empty(), base.formats.is_empty()) {
        (true, _) => base.formats.clone(),
        (_, true) => user.formats.clone(),
        _ => {
            let inter: Vec<String> = user.formats.iter().filter(|f| base.formats.contains(f)).cloned().collect();
            if inter.is_empty() {
                return None;
            }
            inter
        }
    };
    q.tags.extend(base.tags.iter().cloned());
    q.any_tags = match (user.any_tags.is_empty(), base.any_tags.is_empty()) {
        (true, _) => base.any_tags.clone(),
        (_, true) => user.any_tags.clone(),
        // dois "qualquer um" independentes: manter o do usuario e exigir o da colecao como tags nao e equivalente;
        // aproximamos pela intersecao.
        _ => {
            let inter: Vec<String> = user.any_tags.iter().filter(|t| base.any_tags.contains(t)).cloned().collect();
            if inter.is_empty() {
                return None;
            }
            inter
        }
    };
    q.favorite = match (user.favorite, base.favorite) {
        (Some(a), Some(b)) if a != b => return None,
        (a, b) => a.or(b),
    };
    q.min_rating = match (user.min_rating, base.min_rating) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    q.root = match (user.root, base.root) {
        (Some(a), Some(b)) if a != b => return None,
        (a, b) => a.or(b),
    };
    q.storage = match (user.storage, base.storage) {
        (Some(a), Some(b)) if a != b => return None,
        (a, b) => a.or(b),
    };
    q.min_tris = match (user.min_tris, base.min_tris) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    q.max_tris = match (user.max_tris, base.max_tris) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Some(q)
}

fn build(q: &Query, manual_collection: Option<i64>, count_only: bool) -> Result<Built> {
    let mut b = SqlBuilder::new();
    let mut joins = String::new();
    let text = q.text.as_deref().map(parse_text).unwrap_or_default();
    let relevance = q.sort.by == SortBy::Relevance && text.positive.is_some();

    match q.state {
        StateFilter::Active => b.wheres.push("a.state = 0".into()),
        StateFilter::Trashed => b.wheres.push("a.state = 1".into()),
        StateFilter::Missing => b.wheres.push("a.state = 2".into()),
        StateFilter::Corrupt => b.wheres.push("a.state = 3".into()),
        StateFilter::Any => {}
    }
    if !q.kinds.is_empty() {
        let ph: Vec<String> = q.kinds.iter().map(|k| b.p(k.as_str().to_string())).collect();
        b.wheres.push(format!("a.kind IN ({})", ph.join(",")));
    }
    if !q.formats.is_empty() {
        let ph: Vec<String> = q.formats.iter().map(|f| b.p(f.to_lowercase())).collect();
        b.wheres.push(format!("a.format IN ({})", ph.join(",")));
    }
    match q.favorite {
        Some(true) => b.wheres.push("a.favorite = 1".into()),
        Some(false) => b.wheres.push("a.favorite = 0".into()),
        None => {}
    }
    if let Some(r) = q.min_rating {
        let p = b.p(r as i64);
        b.wheres.push(format!("a.rating >= {p}"));
    }
    if let Some(r) = q.root {
        let p = b.p(r.0);
        b.wheres.push(format!("a.root_id = {p}"));
    }
    if let Some(s) = q.storage {
        let p = b.p(s.to_db());
        b.wheres.push(format!("a.storage = {p}"));
    }
    if let Some(t) = q.min_tris {
        let p = b.p(t.min(i64::MAX as u64) as i64);
        b.wheres.push(format!("a.tris >= {p}"));
    }
    if let Some(t) = q.max_tris {
        let p = b.p(t.min(i64::MAX as u64) as i64);
        b.wheres.push(format!("a.tris <= {p}"));
    }
    for t in &q.tags {
        let n = normalize_tag(t).ok_or_else(|| AssetError::InvalidInput(format!("tag invalida: {t:?}")))?;
        let c = tag_exists_clause(&mut b, &n);
        b.wheres.push(c);
    }
    if !q.any_tags.is_empty() {
        let mut ors = Vec::new();
        for t in &q.any_tags {
            let n = normalize_tag(t).ok_or_else(|| AssetError::InvalidInput(format!("tag invalida: {t:?}")))?;
            ors.push(tag_exists_clause(&mut b, &n));
        }
        b.wheres.push(format!("({})", ors.join(" OR ")));
    }
    if let Some(cid) = manual_collection {
        let p = b.p(cid);
        b.wheres.push(format!("EXISTS (SELECT 1 FROM collection_item ci WHERE ci.coll_id = {p} AND ci.asset_id = a.id)"));
    }
    if let Some(pos) = &text.positive {
        let p = b.p(pos.clone());
        if relevance {
            joins.push_str(&format!(
                " JOIN (SELECT rowid AS rid, bm25(asset_fts, 10.0, 1.0, 3.0, 2.0) AS rank FROM asset_fts WHERE asset_fts MATCH {p}) f ON f.rid = a.seq"
            ));
        } else {
            b.wheres.push(format!("a.seq IN (SELECT rowid FROM asset_fts WHERE asset_fts MATCH {p})"));
        }
    }
    if let Some(neg) = &text.negative {
        let p = b.p(neg.clone());
        b.wheres.push(format!("a.seq NOT IN (SELECT rowid FROM asset_fts WHERE asset_fts MATCH {p})"));
    }

    // ordenacao
    let sort_by = if q.sort.by == SortBy::Relevance && !relevance { SortBy::Name } else { q.sort.by };
    let (key_expr, key_is_real) = match sort_by {
        SortBy::Name => ("a.name_key", false),
        SortBy::ImportedAt => ("a.imported_at", false),
        SortBy::ModifiedAt => ("a.modified_at", false),
        SortBy::Size => ("a.size", false),
        SortBy::Rating => ("a.rating", false),
        SortBy::Tris => ("a.tris", false),
        SortBy::Relevance => ("f.rank", true),
    };
    let dir = if q.sort.desc { "DESC" } else { "ASC" };

    if count_only {
        let w = if b.wheres.is_empty() { String::new() } else { format!(" WHERE {}", b.wheres.join(" AND ")) };
        return Ok(Built { sql: format!("SELECT count(*) FROM asset a{joins}{w}"), params: b.params, key_is_real });
    }

    if let Some(cur) = &q.after {
        let (kv, seq) = decode_cursor(cur)?;
        let val: Value = match (&kv, sort_by) {
            (KeyVal::Text(t), SortBy::Name) => Value::Text(t.clone()),
            (KeyVal::Real(r), SortBy::Relevance) => Value::Real(*r),
            (KeyVal::Int(i), s) if !matches!(s, SortBy::Name | SortBy::Relevance) => Value::Integer(*i),
            _ => return Err(AssetError::InvalidInput("cursor nao corresponde a ordenacao atual".into())),
        };
        let pk = b.p(val);
        let ps = b.p(seq);
        let op = if q.sort.desc { "<" } else { ">" };
        b.wheres.push(format!("({key_expr}, a.seq) {op} ({pk}, {ps})"));
    }

    let limit = if q.limit == 0 { DEFAULT_LIMIT } else { q.limit.min(MAX_LIMIT) };
    let w = if b.wheres.is_empty() { String::new() } else { format!(" WHERE {}", b.wheres.join(" AND ")) };
    let sql = format!(
        "SELECT {ROW_COLS}, {key_expr} AS k {ROW_FROM}{joins}{w} ORDER BY {key_expr} {dir}, a.seq {dir} LIMIT {}",
        limit + 1
    );
    Ok(Built { sql, params: b.params, key_is_real })
}

impl Library {
    /// Resolve uma colecao: `(Some(id) manual, None)` ou `(None, Some(query inteligente))`.
    fn resolve_collection(&self, q: &Query) -> Result<(Query, Option<i64>, bool)> {
        match q.collection {
            None => Ok((q.clone(), None, false)),
            Some(cid) => {
                let def = self.collection(cid)?;
                match def.query {
                    Some(base) => match merge_smart(q, &base) {
                        Some(m) => Ok((m, None, false)),
                        None => Ok((q.clone(), None, true)), // vazio por construcao
                    },
                    None => Ok((q.clone(), Some(cid.0), false)),
                }
            }
        }
    }

    /// Executa uma consulta paginada (keyset). Nunca carrega manifestos nem arquivos.
    pub fn query(&self, q: &Query) -> Result<Page<AssetRow>> {
        let (eff, manual, empty) = self.resolve_collection(q)?;
        if empty {
            return Ok(Page { items: vec![], next: None, total: q.with_total.then_some(0) });
        }
        let built = build(&eff, manual, false)?;
        let limit = if eff.limit == 0 { DEFAULT_LIMIT } else { eff.limit.min(MAX_LIMIT) };
        let (mut items, mut keys) = self.inner.db.read(|c| {
            let mut st = c.prepare_cached(&built.sql)?;
            let mut rows = st.query(params_from_iter(built.params.iter()))?;
            let mut items = Vec::new();
            let mut keys = Vec::new();
            while let Some(r) = rows.next()? {
                items.push(row_to_asset(r)?);
                let k: Value = r.get(24)?;
                keys.push(k);
            }
            Ok((items, keys))
        })?;
        let mut next = None;
        if items.len() > limit {
            items.truncate(limit);
            keys.truncate(limit);
            if let (Some(last), Some(k)) = (items.last(), keys.last()) {
                let kv = match k {
                    Value::Integer(i) => KeyVal::Int(*i),
                    Value::Real(r) => KeyVal::Real(*r),
                    Value::Text(t) => KeyVal::Text(t.clone()),
                    _ => KeyVal::Int(0),
                };
                let _ = built.key_is_real;
                next = Some(encode_cursor(&kv, last.seq));
            }
        }
        let total = if q.with_total { Some(self.count(q)?) } else { None };
        Ok(Page { items, next, total })
    }

    /// Conta os ativos que casam (ignora `limit`/`after`).
    pub fn count(&self, q: &Query) -> Result<u64> {
        let (eff, manual, empty) = self.resolve_collection(q)?;
        if empty {
            return Ok(0);
        }
        let mut eff = eff;
        eff.after = None;
        let built = build(&eff, manual, true)?;
        self.inner.db.read(|c| {
            let n: i64 = c.prepare_cached(&built.sql)?.query_row(params_from_iter(built.params.iter()), |r| r.get(0))?;
            Ok(n.max(0) as u64)
        })
    }

    /// Ids que casam com a consulta (todas as paginas), para operacoes em lote. Limitado a `max`.
    pub fn query_ids(&self, q: &Query, max: usize) -> Result<Vec<AssetId>> {
        let mut out = Vec::new();
        let mut q = q.clone();
        q.limit = MAX_LIMIT;
        q.with_total = false;
        loop {
            let page = self.query(&q)?;
            for r in &page.items {
                if out.len() >= max {
                    return Ok(out);
                }
                out.push(r.id);
            }
            match page.next {
                Some(c) => q.after = Some(c),
                None => return Ok(out),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_basic() {
        assert_eq!(parse_text(""), TextFilter::default());
        assert_eq!(parse_text("   ").positive, None);
        assert_eq!(parse_text("cabelo").positive.unwrap(), "(\"cabelo\"*)");
        assert_eq!(parse_text("a").positive.unwrap(), "(\"a\")");
        assert_eq!(parse_text("cabelo longo").positive.unwrap(), "(\"cabelo\"*) AND (\"longo\"*)");
        assert_eq!(parse_text("\"cabelo longo\"").positive.unwrap(), "(\"cabelo longo\")");
        let f = parse_text("rosa -azul tag:noir");
        assert_eq!(f.positive.unwrap(), "(\"rosa\"*) AND (tags : \"noir\"*)");
        assert_eq!(f.negative.unwrap(), "(\"azul\"*)");
        assert_eq!(parse_text("cel-shade").positive.unwrap(), "(\"cel\"* AND \"shade\"*)");
        assert_eq!(parse_text("-so").positive, None);
        assert_eq!(parse_text("-so").negative.unwrap(), "(\"so\"*)");
        assert_eq!(parse_text("author:ana").positive.unwrap(), "(author : \"ana\"*)");
    }

    #[test]
    fn parse_text_neutralizes_fts_syntax() {
        // operadores e aspas soltas nunca chegam ao MATCH
        for evil in ["\" OR 1=1 --", "a AND NOT b", "NEAR(a b)", "(((", "*", "col:", "\"", "''; DROP TABLE asset; --", "name:\"", "- -- -"] {
            let f = parse_text(evil);
            for e in [f.positive, f.negative].into_iter().flatten() {
                // so aspas balanceadas, letras e operadores gerados por nos
                assert_eq!(e.matches('"').count() % 2, 0, "aspas desbalanceadas em {e}");
                assert!(!e.contains(';') && !e.contains("--"), "{e}");
            }
        }
        // limite de termos
        let many = (0..100).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        let f = parse_text(&many);
        assert!(f.positive.unwrap().matches(" AND ").count() < 32);
        // unicode
        assert_eq!(parse_text("Caf\u{e9}").positive.unwrap(), "(\"Caf\u{e9}\"*)");
    }

    #[test]
    fn parse_text_is_valid_fts5() {
        // Valida a sintaxe gerada contra o proprio SQLite (inclui filtros de coluna e negacao).
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING fts5(name, description, tags, author, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
             INSERT INTO t(rowid,name,description,tags,author) VALUES
              (1,'Cabelo Longo','rosa','estilo cel noir','Ana'),
              (2,'Cabelo Curto','azul','estilo cel','Bia'),
              (3,'Camisa','rosa claro','roupa','Ana');",
        )
        .unwrap();
        let run = |s: &str| -> Vec<i64> {
            let f = parse_text(s);
            let mut sql = String::from("SELECT rowid FROM t");
            let mut ws = vec![];
            let mut ps: Vec<String> = vec![];
            if let Some(p) = f.positive {
                ws.push("rowid IN (SELECT rowid FROM t WHERE t MATCH ?)".to_string());
                ps.push(p);
            }
            if let Some(n) = f.negative {
                ws.push("rowid NOT IN (SELECT rowid FROM t WHERE t MATCH ?)".to_string());
                ps.push(n);
            }
            if !ws.is_empty() {
                sql.push_str(&format!(" WHERE {}", ws.join(" AND ")));
            }
            sql.push_str(" ORDER BY rowid");
            let mut st = conn.prepare(&sql).unwrap();
            st.query_map(rusqlite::params_from_iter(ps.iter()), |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(run("cabel"), vec![1, 2]);
        assert_eq!(run("cabel long"), vec![1]);
        assert_eq!(run("cabelo -curto"), vec![1]);
        assert_eq!(run("tag:noir"), vec![1]);
        assert_eq!(run("tag:cel"), vec![1, 2]);
        assert_eq!(run("author:ana"), vec![1, 3]);
        assert_eq!(run("desc:rosa"), vec![1, 3]);
        assert_eq!(run("\"cabelo longo\""), vec![1]);
        assert_eq!(run("-ana"), vec![2]);
        assert_eq!(run("cabelo\u{300}"), vec![1, 2]); // diacritico combinante ignorado
        assert_eq!(run("c\u{e1}belo"), vec![1, 2]);
        assert_eq!(run(""), vec![1, 2, 3]);
        // nenhuma entrada hostil deve gerar erro de sintaxe FTS5
        for evil in ["\" OR 1", "a AND", "NOT", "NEAR(", "(", ")", "*", "^", "{a}:", "-", "--", "name:", "tag:\"", "\"\"\"\""] {
            let f = parse_text(evil);
            for e in [f.positive, f.negative].into_iter().flatten() {
                conn.prepare("SELECT rowid FROM t WHERE t MATCH ?").unwrap().query_map([e.as_str()], |r| r.get::<_, i64>(0)).unwrap().for_each(|r| {
                    r.unwrap();
                });
            }
        }
    }

    #[test]
    fn cursor_roundtrip_and_rejects_garbage() {
        for (k, seq) in [(KeyVal::Int(-5), 7), (KeyVal::Real(0.25), 9), (KeyVal::Text("a:b\u{e9}".into()), 1)] {
            let c = encode_cursor(&k, seq);
            let (k2, s2) = decode_cursor(&c).unwrap();
            assert_eq!(s2, seq);
            match (k, k2) {
                (KeyVal::Int(a), KeyVal::Int(b)) => assert_eq!(a, b),
                (KeyVal::Real(a), KeyVal::Real(b)) => assert_eq!(a, b),
                (KeyVal::Text(a), KeyVal::Text(b)) => assert_eq!(a, b),
                _ => panic!("tipo mudou"),
            }
        }
        for bad in ["", "x", "i:1", "i:x:1", "q:1:1", "t:zz:1", "r:1:1:zz", "i:1:99999999999999999999"] {
            assert!(decode_cursor(&Cursor(bad.into())).is_err(), "{bad}");
        }
    }

    #[test]
    fn merge_smart_semantics() {
        let user = Query { kinds: vec![AssetKind::Model, AssetKind::Hair], favorite: Some(true), ..Default::default() };
        let base = Query { kinds: vec![AssetKind::Hair], tags: vec!["estilo".into()], min_rating: Some(3), ..Default::default() };
        let m = merge_smart(&user, &base).unwrap();
        assert_eq!(m.kinds, vec![AssetKind::Hair]);
        assert_eq!(m.tags, vec!["estilo"]);
        assert_eq!(m.min_rating, Some(3));
        assert_eq!(m.favorite, Some(true));
        let disjoint = Query { kinds: vec![AssetKind::Texture], ..Default::default() };
        assert!(merge_smart(&user, &disjoint).is_none());
        let a = Query { favorite: Some(true), ..Default::default() };
        let b = Query { favorite: Some(false), ..Default::default() };
        assert!(merge_smart(&a, &b).is_none());
        let t = merge_smart(&Query { text: Some("a".into()), ..Default::default() }, &Query { text: Some("b".into()), ..Default::default() }).unwrap();
        assert_eq!(t.text.as_deref(), Some("a b"));
    }

    #[test]
    fn query_serde_defaults() {
        let q: Query = serde_json::from_str("{}").unwrap();
        assert_eq!(q, Query::default());
        let q: Query = serde_json::from_str("{\"kinds\":[\"hair\"],\"sort\":{\"by\":\"rating\",\"desc\":true}}").unwrap();
        assert_eq!(q.kinds, vec![AssetKind::Hair]);
        assert!(q.sort.desc);
    }
}
