//! Biblioteca pela bridge (`LIBRARY`): o Asset Browser sem janela. Consulta e filtros (os mesmos da UI), seleção, metadados
//! (favorito, nota, nome, descrição, tags), coleções (manuais e inteligentes), tags, lixeira, importação, locais (raízes) e
//! manutenção (duplicados, ausentes, verificação, reindexação).
//!
//! Chama o backend `anigo-assets` pelas MESMAS funções dos botões e depois recarrega linhas/facetas/detalhes, então o que a bridge
//! muda aparece na UI. Tudo é validado ANTES de mexer (ids existem, tags normalizam, intervalos). Apagar de vez (`purge`,
//! `empty_trash`) exige `confirm=true`; a bridge NUNCA apaga arquivos externos de locais ligados nem remove arquivos de raízes.
//! Caminhos de arquivo/pasta (importar, locais) passam pelo sandbox do `anigo-mcp` antes de chegar aqui.

use super::*;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use serde_json::{json, Map, Value};

const MAX_IDS: usize = 500;
const MAX_PATHS: usize = 64;
/// Tamanho máximo de arquivos importados com `wait=true` (a UI fica parada durante o import).
const MAX_WAIT_BYTES: u64 = 64 * 1024 * 1024;

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn backend(e: impl std::fmt::Display) -> BridgeError {
    BridgeError::failed(e.to_string())
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn opt_str<'a>(m: &'a Map<String, Value>, k: &str) -> Result<Option<&'a str>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(bad(format!("'{k}' deve ser texto"))),
    }
}

fn opt_bool(m: &Map<String, Value>, k: &str) -> Result<Option<bool>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(format!("'{k}' deve ser booleano"))),
    }
}

fn int_of(v: &Value, name: &str) -> Result<i64, BridgeError> {
    v.as_i64().ok_or_else(|| bad(format!("'{name}' deve ser inteiro")))
}

fn text_limited<'a>(s: &'a str, name: &str, max: usize) -> Result<&'a str, BridgeError> {
    if s.chars().count() > max || s.contains('\0') {
        return Err(bad(format!("'{name}' inválido (máx. {max} caracteres, sem NUL)")));
    }
    Ok(s)
}

fn parse_id(s: &str) -> Result<AssetId, BridgeError> {
    s.parse().map_err(|e: AssetError| bad(e.to_string()))
}

fn ids_list(v: &Value, name: &str) -> Result<Vec<AssetId>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= MAX_IDS).ok_or_else(|| bad(format!("'{name}' deve ser uma lista de até {MAX_IDS} ids")))?;
    a.iter().map(|x| x.as_str().ok_or_else(|| bad(format!("'{name}' deve conter textos"))).and_then(parse_id)).collect()
}

fn string_list(v: &Value, name: &str, max: usize) -> Result<Vec<String>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= max).ok_or_else(|| bad(format!("'{name}' deve ser uma lista de até {max} textos")))?;
    a.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| bad(format!("'{name}' deve conter textos")))).collect()
}

fn merge(base: &mut Value, patch: &Value) {
    match (base.as_object_mut(), patch) {
        (Some(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        _ => *base = patch.clone(),
    }
}

/// `base` + patch JSON -> `Query` (campos desconhecidos são erro; o serde os ignoraria calado).
pub(super) fn query_from(base: Query, patch: &Value) -> Result<Query, BridgeError> {
    if !patch.is_object() {
        return Err(bad("a consulta deve ser um objeto"));
    }
    let mut v = serde_json::to_value(&base).map_err(backend)?;
    let known: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    if let Some(p) = patch.as_object() {
        for k in p.keys() {
            if !known.contains(k) {
                let mut k2 = known.clone();
                k2.sort();
                return Err(bad(format!("campo de consulta desconhecido '{k}'; disponíveis: {}", k2.join(", "))));
            }
        }
    }
    merge(&mut v, patch);
    let q: Query = serde_json::from_value(v).map_err(|e| bad(format!("consulta inválida: {e}")))?;
    if q.text.as_deref().is_some_and(|t| t.len() > 256) {
        return Err(bad("'text' longo demais (máx. 256)"));
    }
    if q.tags.len() > 32 || q.any_tags.len() > 32 || q.kinds.len() > 16 || q.formats.len() > 16 {
        return Err(bad("listas longas demais na consulta (tags/kinds/formats)"));
    }
    Ok(q)
}

fn rows_json(rows: &[AssetRow]) -> Vec<Value> {
    rows.iter().filter_map(|r| serde_json::to_value(r).ok()).collect()
}

fn root_json(r: &RootInfo) -> Value {
    json!({
        "root_id": r.id.0, "name": r.name, "path": r.path.display().to_string(), "mode": r.mode.as_str(), "online": r.online,
        "watch": r.watch, "last_scan": r.last_scan, "asset_count": r.asset_count,
    })
}

fn section_from(v: &Value) -> Result<Section, BridgeError> {
    if let Some(s) = v.as_str() {
        return match s {
            "all" => Ok(Section::All),
            "favorites" => Ok(Section::Favorites),
            "trash" => Ok(Section::Trash),
            "missing" => Ok(Section::Missing),
            o => Err(bad(format!("section '{o}' desconhecida: all, favorites, trash, missing, ou {{collection|tag|root}}"))),
        };
    }
    let o = v.as_object().filter(|o| o.len() == 1).ok_or_else(|| bad("'section' deve ser um texto ou um objeto com UMA chave (collection, tag ou root)"))?;
    let (k, val) = o.iter().next().expect("um item");
    match k.as_str() {
        "collection" => Ok(Section::Collection(CollectionId(int_of(val, "section.collection")?))),
        "root" => Ok(Section::Root(RootId(int_of(val, "section.root")?))),
        "tag" => Ok(Section::Tag(normalize_tag(val.as_str().ok_or_else(|| bad("'section.tag' deve ser texto"))?).ok_or_else(|| bad("tag inválida"))?)),
        o => Err(bad(format!("section.{o} desconhecida (collection, tag, root)"))),
    }
}

fn section_json(s: &Section) -> Value {
    match s {
        Section::All => json!("all"),
        Section::Favorites => json!("favorites"),
        Section::Trash => json!("trash"),
        Section::Missing => json!("missing"),
        Section::Collection(c) => json!({ "collection": c.0 }),
        Section::Tag(t) => json!({ "tag": t }),
        Section::Root(r) => json!({ "root": r.0 }),
    }
}

impl LibraryWorkspace {
    /// Faz o que `op` pede. A Biblioteca precisa estar aberta (o app adia o pedido até lá).
    pub(crate) fn bridge_call(&mut self, ctx: &egui::Context, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        self.pump(ctx);
        let lib = self.library().cloned().ok_or_else(|| BridgeError::failed("a Biblioteca não está aberta"))?;
        let out = match op {
            "info" => self.lib_info(&lib),
            "query" => self.lib_query(&lib, m),
            "filter" => self.lib_filter(&lib, m),
            "select" => self.lib_select(&lib, m),
            "detail" => self.lib_detail(&lib, m),
            "meta" => self.lib_meta(&lib, m),
            "collections" => self.lib_collections(&lib, m),
            "tags" => self.lib_tags(&lib, m),
            "trash" => self.lib_trash(&lib, m),
            "import" => self.lib_import(&lib, m),
            "roots" => self.lib_roots(&lib, m),
            "maintenance" => self.lib_maintenance(&lib, m),
            other => Err(bad(format!("LIBRARY op '{other}' desconhecida (info, query, filter, select, detail, meta, collections, tags, trash, import, roots, maintenance)"))),
        }?;
        Ok(out)
    }

    fn lib_refresh(&mut self) {
        self.reload_rows(false);
        self.reload_facets();
        self.reload_detail();
    }

    // ------------------------------------------------------------------------------------------------ info

    fn lib_info(&mut self, lib: &Library) -> Result<Value, BridgeError> {
        let f = compute_facets(lib).map_err(backend)?;
        let j = &self.jobs;
        let verify = j.verify.as_ref().map(|v| json!({ "done": v.done, "total": v.total, "issues": v.issues.len() }));
        let last_verify = j.last_verify.as_ref().map(|v| json!({ "issues": v.issues.iter().map(|i| json!({ "asset": i.asset.map(|a| a.to_string()), "kind": format!("{:?}", i.kind), "detail": i.detail })).take(100).collect::<Vec<_>>() }));
        Ok(json!({
            "home": lib.home().display().to_string(),
            "filters": {
                "section": section_json(&self.filters.section), "text": self.filters.text, "kinds": self.filters.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                "min_rating": self.filters.min_rating, "sort": serde_json::to_value(self.filters.sort).unwrap_or(Value::Null),
            },
            "view": format!("{:?}", self.view),
            "list_total": self.list.total_or_last(),
            "selection": { "ids": self.selection.ids.iter().map(|i| i.to_string()).collect::<Vec<_>>(), "primary": self.selection.primary.map(|i| i.to_string()) },
            "counts": { "all": f.n_all, "favorites": f.n_favorites, "trash": f.n_trash, "missing": f.n_missing, "duplicate_groups": f.dup_groups, "duplicate_reclaimable_bytes": f.dup_reclaimable },
            "kinds": f.kind_counts.iter().map(|(k, n)| json!({ "kind": k.as_str(), "count": n })).collect::<Vec<_>>(),
            "roots": f.roots.iter().map(root_json).collect::<Vec<_>>(),
            "collections": f.collections.iter().map(|(c, n)| json!({ "collection_id": c.id.0, "name": c.name, "smart": c.is_smart(), "count": n, "query": c.query.as_ref().and_then(|q| serde_json::to_value(q).ok()) })).collect::<Vec<_>>(),
            "tags": f.tags.iter().map(|t| json!({ "path": t.path, "total": t.total, "depth": t.depth, "color": t.color })).collect::<Vec<_>>(),
            "jobs": {
                "import": j.import.as_ref().map(|r| json!({ "label": r.label, "discovered": r.discovered, "done": r.done, "total": r.total, "bytes": r.bytes, "cancel_requested": r.cancel_requested })),
                "import_queue": j.queue.len(),
                "import_log": { "imported": j.log.imported, "revised": j.log.revised, "duplicates": j.log.duplicates, "failed": j.log.failed, "skipped": j.log.skipped, "cancelled": j.log.cancelled,
                    "entries": j.log.entries.iter().rev().take(50).map(|e| json!({ "kind": format!("{:?}", e.kind), "path": e.path.display().to_string(), "detail": e.detail })).collect::<Vec<_>>() },
                "scan": j.scan.as_ref().map(|s| json!({ "root_id": s.root.0, "name": s.name, "done": s.done, "total": s.total })),
                "scan_queue": j.scan_queue.len(),
                "verify": verify, "last_verify": last_verify, "rebuild": j.rebuild.is_some(), "last_rebuild": j.last_rebuild,
                "any_running": j.any_running(),
            },
            "toast": self.toast.as_ref().map(|(t, k, _)| json!({ "text": t, "error": *k == ToastKind::Error })),
        }))
    }

    // ------------------------------------------------------------------------------------------------ consulta

    fn lib_query(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let base = Query { limit: 100, with_total: true, ..Default::default() };
        let mut q = match m.get("query") {
            Some(p) => query_from(base, p)?,
            None => base,
        };
        q.limit = q.limit.clamp(1, 500);
        q.with_total = true;
        let page = lib.query(&q).map_err(backend)?;
        Ok(json!({ "total": page.total, "next": page.next.map(|c| c.0), "assets": rows_json(&page.items) }))
    }

    fn lib_filter(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (section, text, kinds, min_rating, sort)"))?;
        let mut f = self.filters.clone();
        for (k, v) in patch {
            match k.as_str() {
                "section" => f.section = section_from(v)?,
                "text" => f.text = text_limited(v.as_str().ok_or_else(|| bad("'text' deve ser texto"))?, "text", 256)?.to_string(),
                "kinds" => f.kinds = string_list(v, "kinds", 16)?.iter().map(|s| s.parse::<AssetKind>().map_err(|e| bad(e.to_string()))).collect::<Result<_, _>>()?,
                "min_rating" => f.min_rating = v.as_u64().filter(|n| *n <= 5).ok_or_else(|| bad("'min_rating' deve ser 0..5"))? as u8,
                "sort" => {
                    let mut s = serde_json::to_value(f.sort).map_err(backend)?;
                    let keys: Vec<String> = s.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                    if let Some(p) = v.as_object() {
                        if let Some(k) = p.keys().find(|k| !keys.contains(k)) {
                            return Err(bad(format!("sort.{k} desconhecido (by, desc)")));
                        }
                    }
                    merge(&mut s, v);
                    f.sort = serde_json::from_value(s).map_err(|e| bad(format!("sort inválido: {e}")))?;
                    f.user_sorted = true;
                }
                o => return Err(bad(format!("campo desconhecido 'patch.{o}'; disponíveis: kinds, min_rating, section, sort, text"))),
            }
        }
        let section_changed = f.section != self.filters.section;
        self.filters = f;
        if section_changed {
            self.selection.clear();
            self.sync_detail_target();
        }
        self.filters_changed();
        let mut q = self.filters.to_query();
        q.limit = m.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 200) as usize;
        q.with_total = true;
        let page = lib.query(&q).map_err(backend)?;
        Ok(json!({ "total": page.total, "assets": rows_json(&page.items), "filters": self.lib_info_filters() }))
    }

    fn lib_info_filters(&self) -> Value {
        json!({
            "section": section_json(&self.filters.section), "text": self.filters.text, "kinds": self.filters.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "min_rating": self.filters.min_rating, "sort": serde_json::to_value(self.filters.sort).unwrap_or(Value::Null),
        })
    }

    fn lib_select(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let ids: Vec<AssetId> = if opt_bool(m, "none")?.unwrap_or(false) {
            Vec::new()
        } else if opt_bool(m, "all_matching")?.unwrap_or(false) {
            let mut q = self.filters.to_query();
            q.after = None;
            lib.query_ids(&q, MAX_IDS).map_err(backend)?
        } else {
            let ids = ids_list(m.get("asset_ids").ok_or_else(|| bad("informe 'asset_ids', 'all_matching' ou 'none'"))?, "asset_ids")?;
            for id in &ids {
                lib.require_row(id).map_err(backend)?;
            }
            ids
        };
        self.selection.clear();
        for id in &ids {
            self.selection.ids.insert(*id);
            self.selection.primary = Some(*id);
        }
        self.sync_detail_target();
        Ok(json!({ "selected": ids.len(), "ids": ids.iter().map(|i| i.to_string()).collect::<Vec<_>>() }))
    }

    fn lib_detail(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let id = parse_id(str_of(m, "asset_id")?)?;
        let d = lib.get(&id).map_err(backend)?;
        let tags = lib.tags_of(&id).map_err(backend)?;
        let cols = lib.collections_of(&id).map_err(backend)?;
        let mut v = serde_json::to_value(&d).map_err(backend)?;
        v["tags"] = json!(tags);
        v["collections"] = json!(cols.iter().map(|c| c.0).collect::<Vec<_>>());
        if let Ok(f) = lib.resolve_files(&id) {
            v["files"] = json!({ "primary": f.primary.display().to_string(), "exists": f.exists, "count": f.files.len() });
        }
        Ok(v)
    }

    // ------------------------------------------------------------------------------------------------ metadados

    /// Ids do pedido: `asset_ids`, ou a seleção atual da UI.
    fn target_ids(&self, lib: &Library, m: &Map<String, Value>) -> Result<Vec<AssetId>, BridgeError> {
        let ids = match m.get("asset_ids").filter(|v| !v.is_null()) {
            Some(v) => ids_list(v, "asset_ids")?,
            None => self.selected_ids(),
        };
        if ids.is_empty() {
            return Err(bad("informe 'asset_ids' (ou selecione ativos)"));
        }
        for id in &ids {
            lib.require_row(id).map_err(backend)?;
        }
        Ok(ids)
    }

    fn lib_meta(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let ids = self.target_ids(lib, m)?;
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (favorite, rating, name, description, author, license, source_url, kind, tags_add, tags_remove, tags_set)"))?;
        // validação completa antes de gravar
        let mut info = InfoPatch::default();
        let (mut favorite, mut rating) = (None, None);
        let (mut add, mut remove, mut set): (Vec<String>, Vec<String>, Option<Vec<String>>) = (Vec::new(), Vec::new(), None);
        let norm = |list: Vec<String>| -> Result<Vec<String>, BridgeError> { list.iter().map(|t| normalize_tag(t).ok_or_else(|| bad(format!("tag inválida: {t:?}")))).collect() };
        for (k, v) in patch {
            let txt = |n: &str, max: usize| -> Result<String, BridgeError> { Ok(text_limited(v.as_str().ok_or_else(|| bad(format!("'{n}' deve ser texto")))?, n, max)?.to_string()) };
            match k.as_str() {
                "favorite" => favorite = Some(v.as_bool().ok_or_else(|| bad("'favorite' deve ser booleano"))?),
                "rating" => rating = Some(v.as_u64().filter(|n| *n <= 5).ok_or_else(|| bad("'rating' deve ser 0..5"))? as u8),
                "name" => {
                    if ids.len() != 1 {
                        return Err(bad("'name' só vale para UM ativo"));
                    }
                    let n = txt("name", 200)?;
                    if n.trim().is_empty() {
                        return Err(bad("'name' não pode ser vazio"));
                    }
                    info.name = Some(n);
                }
                "description" => info.description = Some(txt("description", 4000)?),
                "author" => info.author = Some(txt("author", 200)?),
                "license" => info.license = Some(txt("license", 200)?),
                "source_url" => info.source_url = Some(txt("source_url", 1000)?),
                "kind" => info.kind = Some(txt("kind", 32)?.parse::<AssetKind>().map_err(|e| bad(e.to_string()))?),
                "tags_add" => add = norm(string_list(v, "tags_add", 64)?)?,
                "tags_remove" => remove = norm(string_list(v, "tags_remove", 64)?)?,
                "tags_set" => set = Some(norm(string_list(v, "tags_set", 64)?)?),
                o => return Err(bad(format!("campo desconhecido 'patch.{o}'; disponíveis: author, description, favorite, kind, license, name, rating, source_url, tags_add, tags_remove, tags_set"))),
            }
        }
        let has_info = info.name.is_some() || info.description.is_some() || info.author.is_some() || info.license.is_some() || info.source_url.is_some() || info.kind.is_some();
        for id in &ids {
            if let Some(f) = favorite {
                lib.set_favorite(id, f).map_err(backend)?;
            }
            if let Some(r) = rating {
                lib.set_rating(id, r).map_err(backend)?;
            }
            if has_info {
                lib.update_info(id, &info).map_err(backend)?;
            }
            if let Some(s) = &set {
                lib.set_tags(id, s).map_err(backend)?;
            }
            if !remove.is_empty() {
                lib.remove_tags(id, &remove).map_err(backend)?;
            }
        }
        if !add.is_empty() {
            lib.add_tags_many(&ids, &add).map_err(backend)?;
        }
        self.lib_refresh();
        let rows: Vec<AssetRow> = ids.iter().filter_map(|i| lib.row(i).ok().flatten()).collect();
        let tags: Vec<Value> = ids.iter().map(|i| json!({ "asset_id": i.to_string(), "tags": lib.tags_of(i).unwrap_or_default() })).collect();
        Ok(json!({ "updated": ids.len(), "assets": rows_json(&rows), "tags": tags }))
    }

    // ------------------------------------------------------------------------------------------------ coleções e tags

    fn lib_collections(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let cid = |m: &Map<String, Value>| -> Result<CollectionId, BridgeError> { Ok(CollectionId(int_of(m.get("collection_id").ok_or_else(|| bad("'collection_id' é obrigatório"))?, "collection_id")?)) };
        let name = |m: &Map<String, Value>| -> Result<String, BridgeError> {
            let n = text_limited(str_of(m, "name")?, "name", 120)?.trim().to_string();
            if n.is_empty() {
                return Err(bad("'name' não pode ser vazio"));
            }
            Ok(n)
        };
        let mut changed = 0u64;
        let mut created = None;
        match str_of(m, "action")? {
            "list" => {}
            "create" => created = Some(lib.create_collection(&name(m)?).map_err(backend)?),
            "create_smart" => {
                let q = query_from(Query::default(), m.get("query").ok_or_else(|| bad("'query' é obrigatória (text, kinds, tags, favorite, min_rating…)"))?)?;
                created = Some(lib.create_smart_collection(&name(m)?, &q).map_err(backend)?);
            }
            "update_smart" => {
                let q = query_from(Query::default(), m.get("query").ok_or_else(|| bad("'query' é obrigatória"))?)?;
                lib.update_smart_collection(cid(m)?, &q).map_err(backend)?;
            }
            "rename" => lib.rename_collection(cid(m)?, &name(m)?).map_err(backend)?,
            "delete" => lib.delete_collection(cid(m)?).map_err(backend)?,
            "add" | "remove" => {
                let id = cid(m)?;
                let ids = ids_list(m.get("asset_ids").ok_or_else(|| bad("'asset_ids' é obrigatório"))?, "asset_ids")?;
                for a in &ids {
                    lib.require_row(a).map_err(backend)?;
                }
                changed = if str_of(m, "action")? == "add" { lib.add_to_collection(id, &ids) } else { lib.remove_from_collection(id, &ids) }.map_err(backend)?;
            }
            o => return Err(bad(format!("collections.action '{o}' desconhecida (list, create, create_smart, update_smart, rename, delete, add, remove)"))),
        }
        self.lib_refresh();
        let cols = lib.collections().map_err(backend)?;
        Ok(json!({
            "created": created.map(|c| c.0), "changed": changed,
            "collections": cols.iter().map(|c| json!({ "collection_id": c.id.0, "name": c.name, "smart": c.is_smart(), "items": if c.is_smart() { lib.count(&Query { collection: Some(c.id), ..Default::default() }).unwrap_or(0) } else { c.item_count }, "query": c.query.as_ref().and_then(|q| serde_json::to_value(q).ok()) })).collect::<Vec<_>>(),
        }))
    }

    fn lib_tags(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let path = |m: &Map<String, Value>, k: &str| -> Result<String, BridgeError> { normalize_tag(str_of(m, k)?).ok_or_else(|| bad(format!("'{k}' não é uma tag válida"))) };
        let mut changed = 0usize;
        match str_of(m, "action")? {
            "list" => {}
            "rename" => changed = lib.rename_tag(&path(m, "path")?, &path(m, "new")?).map_err(backend)?,
            "delete" => changed = lib.delete_tag(&path(m, "path")?).map_err(backend)?,
            "color" => {
                let c = text_limited(str_of(m, "color")?, "color", 16)?;
                lib.set_tag_color(&path(m, "path")?, c).map_err(backend)?;
            }
            "prune" => changed = lib.prune_tags().map_err(backend)?,
            o => return Err(bad(format!("tags.action '{o}' desconhecida (list, rename, delete, color, prune)"))),
        }
        self.lib_refresh();
        let tags = lib.tags().map_err(backend)?;
        Ok(json!({ "changed": changed, "tags": tags.iter().map(|t| json!({ "path": t.path, "color": t.color, "count": t.count })).collect::<Vec<_>>() }))
    }

    // ------------------------------------------------------------------------------------------------ lixeira

    fn lib_trash(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let action = str_of(m, "action")?;
        let confirmed = opt_bool(m, "confirm")?.unwrap_or(false);
        let mut out = Map::new();
        match action {
            "trash" => {
                let ids = self.target_ids(lib, m)?;
                let rep = lib.trash(&ids, TrashOptions { force: opt_bool(m, "force")?.unwrap_or(false) }).map_err(backend)?;
                out.insert("trashed".into(), json!(rep.trashed.iter().map(|i| i.to_string()).collect::<Vec<_>>()));
                out.insert("already".into(), json!(rep.already.iter().map(|i| i.to_string()).collect::<Vec<_>>()));
                out.insert("blocked".into(), json!(rep.blocked.iter().map(|(a, d)| json!({ "asset_id": a.to_string(), "used_by": d.iter().map(|i| i.to_string()).collect::<Vec<_>>() })).collect::<Vec<_>>()));
                self.selection.clear();
                self.sync_detail_target();
            }
            "restore" => {
                let ids = self.target_ids(lib, m)?;
                let back = lib.restore(&ids).map_err(backend)?;
                out.insert("restored".into(), json!(back.iter().map(|i| i.to_string()).collect::<Vec<_>>()));
            }
            "purge" | "empty" => {
                if !confirmed {
                    return Err(bad("apagar DEFINITIVAMENTE exige confirm=true (irreversível)"));
                }
                let rep = if action == "purge" {
                    let ids = ids_list(m.get("asset_ids").ok_or_else(|| bad("'asset_ids' é obrigatório"))?, "asset_ids")?;
                    for id in &ids {
                        let r = lib.require_row(id).map_err(backend)?;
                        if r.state != AssetState::Trashed {
                            return Err(bad(format!("{id} não está na lixeira; envie-o antes com action=trash")));
                        }
                    }
                    lib.purge(&ids).map_err(backend)?
                } else {
                    lib.empty_trash(m.get("older_than_secs").and_then(Value::as_u64)).map_err(backend)?
                };
                out.insert("purged".into(), json!(rep.purged.iter().map(|i| i.to_string()).collect::<Vec<_>>()));
                out.insert("bytes_freed".into(), json!(rep.bytes_freed));
                self.selection.clear();
                self.sync_detail_target();
            }
            o => return Err(bad(format!("trash.action '{o}' desconhecida (trash, restore, purge, empty)"))),
        }
        self.lib_refresh();
        Ok(Value::Object(out))
    }

    // ------------------------------------------------------------------------------------------------ importação

    fn lib_import(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        if let Some(true) = opt_bool(m, "cancel")? {
            self.cancel_imports();
            return Ok(json!({ "cancel_requested": true }));
        }
        let paths: Vec<std::path::PathBuf> = string_list(m.get("paths").ok_or_else(|| bad("'paths' é obrigatório (arquivos ou pastas)"))?, "paths", MAX_PATHS)?
            .into_iter()
            .map(|p| {
                if p.is_empty() || p.contains('\0') {
                    Err(bad("caminho vazio ou com NUL"))
                } else {
                    Ok(std::path::PathBuf::from(p))
                }
            })
            .collect::<Result<_, _>>()?;
        if paths.is_empty() {
            return Err(bad("'paths' não pode ser vazio"));
        }
        for p in &paths {
            if !p.exists() {
                return Err(bad(format!("{} não existe", p.display())));
            }
        }
        let mut o = ImportOptions::default();
        if let Some(opts) = m.get("options").filter(|v| !v.is_null()) {
            let opts = opts.as_object().ok_or_else(|| bad("'options' deve ser um objeto"))?;
            for (k, v) in opts {
                let txt = |n: &str, max: usize| -> Result<String, BridgeError> { Ok(text_limited(v.as_str().ok_or_else(|| bad(format!("'options.{n}' deve ser texto")))?, n, max)?.to_string()) };
                match k.as_str() {
                    "mode" => o.mode = txt("mode", 16)?.parse::<StorageMode>().map_err(|e| bad(e.to_string()))?,
                    "recursive" => o.recursive = v.as_bool().ok_or_else(|| bad("'options.recursive' deve ser booleano"))?,
                    "kind" => o.kind = Some(txt("kind", 32)?.parse::<AssetKind>().map_err(|e| bad(e.to_string()))?),
                    "name" => o.name = Some(txt("name", 200)?),
                    "tags" => o.tags = string_list(v, "options.tags", 64)?.iter().map(|t| normalize_tag(t).ok_or_else(|| bad(format!("tag inválida: {t:?}")))).collect::<Result<_, _>>()?,
                    "description" => o.description = Some(txt("description", 4000)?),
                    "author" => o.author = Some(txt("author", 200)?),
                    "license" => o.license = Some(txt("license", 200)?),
                    "source_url" => o.source_url = Some(txt("source_url", 1000)?),
                    "duplicates" => {
                        o.duplicates = match txt("duplicates", 8)?.as_str() {
                            "skip" => DuplicatePolicy::Skip,
                            "allow" => DuplicatePolicy::Allow,
                            _ => return Err(bad("options.duplicates deve ser skip|allow")),
                        }
                    }
                    "collection" => o.collection = Some(CollectionId(int_of(v, "options.collection")?)),
                    "root" => o.root = Some(RootId(int_of(v, "options.root")?)),
                    "replace" => o.replace = Some(parse_id(&txt("replace", 64)?)?),
                    other => return Err(bad(format!("campo desconhecido 'options.{other}'; disponíveis: author, collection, description, duplicates, kind, license, mode, name, recursive, replace, root, source_url, tags"))),
                }
            }
        }
        if o.name.is_some() && paths.len() > 1 {
            return Err(bad("'options.name' só vale quando se importa UM caminho"));
        }
        if opt_bool(m, "wait")?.unwrap_or(false) {
            // síncrono: a janela fica parada durante o import, então só arquivos pequenos
            let mut total = 0u64;
            for p in &paths {
                let md = std::fs::metadata(p).map_err(backend)?;
                if !md.is_file() {
                    return Err(bad("wait=true só aceita arquivos (use wait=false para pastas)"));
                }
                total += md.len();
            }
            if total > MAX_WAIT_BYTES {
                return Err(bad(format!("{total} bytes é demais para wait=true (máx. {MAX_WAIT_BYTES}); use wait=false e acompanhe com op=info")));
            }
            let mut rep_all = Vec::new();
            for p in &paths {
                let rep = lib.import_blocking(p, o.clone()).map_err(backend)?;
                rep_all.push(json!({
                    "path": p.display().to_string(),
                    "imported": rep.imported.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
                    "revised": rep.revised.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
                    "duplicates": rep.duplicates.iter().map(|(p, i)| json!({ "path": p.display().to_string(), "existing": i.to_string() })).collect::<Vec<_>>(),
                    "failed": rep.failed.iter().map(|(p, e)| json!({ "path": p.display().to_string(), "error": e })).collect::<Vec<_>>(),
                    "skipped": rep.skipped, "bytes": rep.bytes,
                }));
            }
            self.lib_refresh();
            return Ok(json!({ "waited": true, "reports": rep_all }));
        }
        let n = paths.len();
        self.queue_import(paths, o);
        Ok(json!({ "queued": n, "note": "acompanhe com op=info (jobs.import / jobs.import_log)" }))
    }

    // ------------------------------------------------------------------------------------------------ locais

    fn lib_roots(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        let rid = |m: &Map<String, Value>| -> Result<RootId, BridgeError> { Ok(RootId(int_of(m.get("root_id").ok_or_else(|| bad("'root_id' é obrigatório"))?, "root_id")?)) };
        let mut added = None;
        match str_of(m, "action")? {
            "list" => {}
            "refresh" => {
                lib.refresh_roots().map_err(backend)?;
            }
            "add" => {
                let path = str_of(m, "path")?;
                let mode: RootMode = opt_str(m, "mode")?.unwrap_or("linked").parse().map_err(|e: AssetError| bad(e.to_string()))?;
                let name = opt_str(m, "name")?.map(|n| text_limited(n, "name", 120)).transpose()?;
                added = Some(lib.add_root(std::path::Path::new(path), mode, name).map_err(backend)?.0);
                if mode == RootMode::Linked && opt_bool(m, "scan")?.unwrap_or(true) {
                    self.queue_scan(RootId(added.expect("recém criada")));
                }
            }
            "remove" => {
                let id = rid(m)?;
                if id.0 == 1 {
                    return Err(bad("o local 1 (a própria biblioteca) não pode ser removido"));
                }
                // NUNCA apaga arquivos: delete_files=false
                lib.remove_root(id, false).map_err(backend)?;
            }
            "scan" => match m.get("root_id").filter(|v| !v.is_null()) {
                Some(_) => {
                    let id = rid(m)?;
                    lib.root(id).map_err(backend)?;
                    self.queue_scan(id);
                }
                None => self.queue_scan_all_linked(),
            },
            o => return Err(bad(format!("roots.action '{o}' desconhecida (list, refresh, add, remove, scan)"))),
        }
        self.lib_refresh();
        Ok(json!({ "added": added, "roots": lib.roots().map_err(backend)?.iter().map(root_json).collect::<Vec<_>>() }))
    }

    // ------------------------------------------------------------------------------------------------ manutenção

    fn lib_maintenance(&mut self, lib: &Library, m: &Map<String, Value>) -> Result<Value, BridgeError> {
        match str_of(m, "action")? {
            "duplicates" => {
                let limit = m.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 500) as usize;
                let groups = lib.find_duplicates(limit).map_err(backend)?;
                Ok(json!({ "groups": groups.iter().map(|g| json!({ "content_hash": g.content_hash.to_hex(), "size": g.size, "reclaimable_bytes": g.reclaimable_bytes(), "assets": rows_json(&g.assets) })).collect::<Vec<_>>() }))
            }
            "missing" => {
                let rows = lib.find_missing().map_err(backend)?;
                Ok(json!({ "total": rows.len(), "assets": rows_json(&rows.into_iter().take(500).collect::<Vec<_>>()) }))
            }
            "verify" => {
                let opts = VerifyOptions { rehash: opt_bool(m, "rehash")?.unwrap_or(true), fix: opt_bool(m, "fix")?.unwrap_or(false) };
                self.start_verify(opts);
                Ok(json!({ "started": self.jobs.verify.is_some() }))
            }
            "rebuild" => {
                self.start_rebuild();
                Ok(json!({ "started": self.jobs.rebuild.is_some() }))
            }
            "cancel_imports" => {
                self.cancel_imports();
                Ok(json!({ "cancel_requested": true }))
            }
            o => Err(bad(format!("maintenance.action '{o}' desconhecida (duplicates, missing, verify, rebuild, cancel_imports)"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_patch_rejects_unknown_fields_and_bad_types() {
        let q = query_from(Query::default(), &json!({ "text": "hero", "kinds": ["model"], "min_rating": 2, "sort": { "by": "size", "desc": true }, "limit": 10 })).unwrap();
        assert_eq!(q.text.as_deref(), Some("hero"));
        assert_eq!(q.kinds, vec![AssetKind::Model]);
        assert_eq!(q.min_rating, Some(2));
        assert!(q.sort.desc && q.sort.by == SortBy::Size);
        assert_eq!(q.limit, 10);
        assert!(query_from(Query::default(), &json!({ "textt": "x" })).is_err());
        assert!(query_from(Query::default(), &json!({ "kinds": ["bomba"] })).is_err());
        assert!(query_from(Query::default(), &json!({ "tags": "a" })).is_err());
        assert!(query_from(Query::default(), &json!(3)).is_err());
        assert!(query_from(Query::default(), &json!({ "text": "x".repeat(300) })).is_err());
    }

    #[test]
    fn sections_roundtrip_through_json() {
        for s in [Section::All, Section::Favorites, Section::Trash, Section::Missing, Section::Collection(CollectionId(3)), Section::Tag("a/b".into()), Section::Root(RootId(2))] {
            assert_eq!(section_from(&section_json(&s)).unwrap(), s);
        }
        assert!(section_from(&json!("nada")).is_err());
        assert!(section_from(&json!({ "collection": 1, "tag": "x" })).is_err());
        assert!(section_from(&json!({ "tag": 4 })).is_err());
    }

    #[test]
    fn ids_and_texts_are_validated() {
        assert!(ids_list(&json!(["não-é-id"]), "x").is_err());
        assert!(ids_list(&json!("a"), "x").is_err());
        assert!(ids_list(&json!([]), "x").unwrap().is_empty());
        assert!(text_limited("a\0b", "t", 10).is_err());
        assert!(text_limited("abcdef", "t", 3).is_err());
    }
}
