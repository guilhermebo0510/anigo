mod common;

use std::collections::HashSet;

use anigo_assets::*;
use common::*;

/// Importa `n` PNGs `img000..` e devolve ids em ordem de nome.
fn populate(env: &Env, n: u32) -> Vec<AssetId> {
    let src = env.src("pop");
    for i in 0..n {
        write(&src, &format!("img{i:03}.png"), &unique_png(i));
    }
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert_eq!(r.imported.len(), n as usize, "{:?}", r.failed);
    let mut rows = env.lib.query(&Query { limit: 1000, ..Default::default() }).unwrap().items;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.into_iter().map(|r| r.id).collect()
}

fn page_all(lib: &Library, mut q: Query) -> Vec<AssetRow> {
    let mut out = Vec::new();
    let mut guard = 0;
    loop {
        let page = lib.query(&q).unwrap();
        out.extend(page.items);
        match page.next {
            Some(c) => q.after = Some(c),
            None => return out,
        }
        guard += 1;
        assert!(guard < 1000, "paginacao nao termina");
    }
}

#[test]
fn keyset_pagination_is_exact_for_every_sort() {
    let env = Env::new();
    let ids = populate(&env, 250);
    // varia rating para ter muitos empates (testa o desempate por seq)
    for (i, id) in ids.iter().enumerate() {
        env.lib.set_rating(id, (i % 6) as u8).unwrap();
    }
    for by in [SortBy::Name, SortBy::ImportedAt, SortBy::ModifiedAt, SortBy::Size, SortBy::Rating, SortBy::Tris] {
        for desc in [false, true] {
            let q = Query { sort: Sort { by, desc }, limit: 37, ..Default::default() };
            let rows = page_all(&env.lib, q);
            assert_eq!(rows.len(), 250, "{by:?} desc={desc}");
            let uniq: HashSet<AssetId> = rows.iter().map(|r| r.id).collect();
            assert_eq!(uniq.len(), 250, "duplicatas/omissoes em {by:?} desc={desc}");
            // monotonia
            let key = |r: &AssetRow| -> (String, i64) {
                match by {
                    SortBy::Name => (r.name.to_lowercase(), 0),
                    SortBy::ImportedAt => (String::new(), r.imported_at),
                    SortBy::ModifiedAt => (String::new(), r.modified_at),
                    SortBy::Size => (String::new(), r.size as i64),
                    SortBy::Rating => (String::new(), r.rating as i64),
                    SortBy::Tris => (String::new(), r.tris as i64),
                    SortBy::Relevance => unreachable!(),
                }
            };
            for w in rows.windows(2) {
                let (a, b) = (key(&w[0]), key(&w[1]));
                if desc {
                    assert!(a >= b, "{by:?} desc: {a:?} < {b:?}");
                } else {
                    assert!(a <= b, "{by:?} asc: {a:?} > {b:?}");
                }
            }
        }
    }
    // nome asc == ordem alfabetica exata
    let rows = page_all(&env.lib, Query { limit: 100, ..Default::default() });
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn total_limit_and_cursor_validation() {
    let env = Env::new();
    populate(&env, 30);
    let p = env.lib.query(&Query { limit: 10, with_total: true, ..Default::default() }).unwrap();
    assert_eq!(p.items.len(), 10);
    assert_eq!(p.total, Some(30));
    assert!(p.next.is_some());
    let p = env.lib.query(&Query { limit: 0, ..Default::default() }).unwrap();
    assert_eq!(p.items.len(), 30);
    assert!(p.next.is_none() && p.total.is_none());
    // limite enorme e truncado a 1000
    assert_eq!(env.lib.query(&Query { limit: usize::MAX, ..Default::default() }).unwrap().items.len(), 30);
    // cursor lixo e cursor de outra ordenacao
    assert!(matches!(env.lib.query(&Query { after: Some(Cursor("lixo".into())), ..Default::default() }), Err(AssetError::InvalidInput(_))));
    let page = env.lib.query(&Query { limit: 5, ..Default::default() }).unwrap();
    let bad = Query { sort: Sort { by: SortBy::Size, desc: false }, after: page.next, ..Default::default() };
    assert!(matches!(env.lib.query(&bad), Err(AssetError::InvalidInput(_))));
    // pagina vazia
    let none = env.lib.query(&Query { text: Some("zzzzzz".into()), with_total: true, ..Default::default() }).unwrap();
    assert!(none.items.is_empty() && none.next.is_none());
    assert_eq!(none.total, Some(0));
}

#[test]
fn relevance_sort_ranks_name_matches_first_and_paginates() {
    let env = Env::new();
    let src = env.src("rel");
    // 'cabelo' no nome vs so na descricao
    let a = write(&src, "cabelo longo.png", &unique_png(1));
    let b = write(&src, "outra coisa.png", &unique_png(2));
    let c = write(&src, "mais uma.png", &unique_png(3));
    let ida = import_one(&env.lib, &a, ImportOptions::default());
    let idb = import_one(&env.lib, &b, ImportOptions { description: Some("um cabelo bonito".into()), ..Default::default() });
    let _idc = import_one(&env.lib, &c, ImportOptions::default());
    let q = Query { text: Some("cabelo".into()), sort: Sort { by: SortBy::Relevance, desc: false }, limit: 1, ..Default::default() };
    let rows = page_all(&env.lib, q);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, ida, "nome pesa mais que descricao");
    assert_eq!(rows[1].id, idb);
    // Relevance sem texto cai para nome
    let rows = page_all(&env.lib, Query { sort: Sort { by: SortBy::Relevance, desc: false }, limit: 2, ..Default::default() });
    assert_eq!(rows.len(), 3);
}

#[test]
fn hierarchical_tags_and_filters() {
    let env = Env::new();
    let ids = populate(&env, 12);
    env.lib.set_tags(&ids[0], &["estilo/cel/noir".into(), "personagem".into()]).unwrap();
    env.lib.set_tags(&ids[1], &["estilo/cel".into()]).unwrap();
    env.lib.set_tags(&ids[2], &["estilo/aquarela".into(), "personagem".into()]).unwrap();
    env.lib.set_tags(&ids[3], &["50%_off".into()]).unwrap();
    env.lib.set_tags(&ids[4], &["500x_off".into()]).unwrap();
    let count = |tags: Vec<&str>, any: Vec<&str>| {
        env.lib.count(&Query { tags: tags.into_iter().map(String::from).collect(), any_tags: any.into_iter().map(String::from).collect(), ..Default::default() }).unwrap()
    };
    assert_eq!(count(vec!["estilo"], vec![]), 3, "pai casa com descendentes");
    assert_eq!(count(vec!["estilo/cel"], vec![]), 2);
    assert_eq!(count(vec!["estilo/cel/noir"], vec![]), 1);
    assert_eq!(count(vec!["estilo", "personagem"], vec![]), 2, "E entre tags");
    assert_eq!(count(vec![], vec!["estilo/aquarela", "estilo/cel/noir"]), 2, "OU em any_tags");
    assert_eq!(count(vec!["50%_off"], vec![]), 1, "% e _ sao literais, nao curingas do LIKE");
    assert_eq!(count(vec!["estil"], vec![]), 0, "prefixo de string nao e prefixo de caminho");
    assert_eq!(count(vec!["ESTILO / Cel"], vec![]), 2, "normalizada");
    assert!(env.lib.count(&Query { tags: vec!["///".into()], ..Default::default() }).is_err());
    // tags() com contagem
    let tags = env.lib.tags().unwrap();
    let get = |p: &str| tags.iter().find(|t| t.path == p).map(|t| t.count);
    assert_eq!(get("personagem"), Some(2));
    assert_eq!(get("estilo/cel"), Some(1));
    // FTS acha pelo texto da tag
    assert_eq!(env.lib.query(&Query { text: Some("tag:noir".into()), ..Default::default() }).unwrap().items.len(), 1);
    assert_eq!(env.lib.query(&Query { text: Some("tag:cel".into()), ..Default::default() }).unwrap().items.len(), 2);
    // renomear arvore
    assert_eq!(env.lib.rename_tag("estilo/cel", "look/cel").unwrap(), 2);
    assert_eq!(count(vec!["look/cel/noir"], vec![]), 1);
    assert_eq!(count(vec!["estilo"], vec![]), 1);
    assert!(env.lib.rename_tag("look", "look/dentro").is_err());
    // apagar tag
    assert_eq!(env.lib.delete_tag("look").unwrap(), 2);
    assert_eq!(count(vec!["look"], vec![]), 0);
    assert!(env.lib.tags().unwrap().iter().all(|t| !t.path.starts_with("look")));
    // cor de tag e add/remove
    env.lib.set_tag_color("personagem", "#ff00aa").unwrap();
    assert_eq!(env.lib.tags().unwrap().iter().find(|t| t.path == "personagem").unwrap().color, "#ff00aa");
    env.lib.add_tags(&ids[5], &["novo".into(), "Novo".into()]).unwrap();
    assert_eq!(env.lib.tags_of(&ids[5]).unwrap(), vec!["novo"]);
    env.lib.remove_tags(&ids[5], &["novo".into()]).unwrap();
    assert!(env.lib.tags_of(&ids[5]).unwrap().is_empty());
    assert!(env.lib.add_tags(&ids[5], &["//".into()]).is_err());
}

#[test]
fn favorites_rating_kind_and_info_edits_persist_in_the_manifest() {
    let env = Env::new();
    let ids = populate(&env, 6);
    env.lib.set_favorite(&ids[0], true).unwrap();
    env.lib.set_favorite(&ids[1], true).unwrap();
    env.lib.set_rating(&ids[1], 5).unwrap();
    env.lib.set_rating(&ids[2], 3).unwrap();
    assert!(env.lib.set_rating(&ids[2], 6).is_err());
    assert_eq!(env.lib.count(&Query { favorite: Some(true), ..Default::default() }).unwrap(), 2);
    assert_eq!(env.lib.count(&Query { favorite: Some(false), ..Default::default() }).unwrap(), 4);
    assert_eq!(env.lib.count(&Query { min_rating: Some(3), ..Default::default() }).unwrap(), 2);
    assert_eq!(env.lib.count(&Query { min_rating: Some(4), ..Default::default() }).unwrap(), 1);
    env.lib.update_info(&ids[3], &InfoPatch { name: Some("Camisa Azul".into()), description: Some("uma camisa".into()), author: Some("Bia".into()), kind: Some(AssetKind::Outfit), ..Default::default() }).unwrap();
    assert_eq!(env.lib.query(&Query { text: Some("camisa".into()), ..Default::default() }).unwrap().items.len(), 1);
    assert_eq!(env.lib.query(&Query { text: Some("azul".into()), kinds: vec![AssetKind::Outfit], ..Default::default() }).unwrap().items.len(), 1);
    assert_eq!(env.lib.query(&Query { text: Some("img003".into()), ..Default::default() }).unwrap().items.len(), 0, "nome antigo some do FTS");
    assert!(env.lib.rename(&ids[3], "   ").is_err());
    assert!(env.lib.rename(&ids[3], &"x".repeat(300)).is_err());
    assert!(env.lib.update_info(&ids[3], &InfoPatch { description: Some("a\0b".into()), ..Default::default() }).is_err());
    // o manifesto em disco reflete tudo
    let m = env.lib.manifest(&ids[1]).unwrap();
    assert!(m.favorite);
    assert_eq!(m.rating, 5);
    let m = env.lib.manifest(&ids[3]).unwrap();
    assert_eq!(m.name, "Camisa Azul");
    assert_eq!(m.kind, AssetKind::Outfit);
    // sobrevive a perda do indice
    let env = rebuild_from_scratch(env);
    assert_eq!(env.lib.count(&Query { favorite: Some(true), ..Default::default() }).unwrap(), 2);
    assert_eq!(env.lib.query(&Query { text: Some("camisa".into()), ..Default::default() }).unwrap().items.len(), 1);
    assert_eq!(env.lib.count(&Query { min_rating: Some(5), ..Default::default() }).unwrap(), 1);
}

fn rebuild_from_scratch(env: Env) -> Env {
    let idx = env.lib.index_path().to_path_buf();
    let env = {
        let home = env.lib.home().to_path_buf();
        let dir = env.dir;
        drop(env.lib);
        for ext in ["", "-wal", "-shm"] {
            let mut p = idx.clone().into_os_string();
            p.push(ext);
            let _ = std::fs::remove_file(p);
        }
        let lib = Library::open(home).unwrap();
        Env { dir, lib }
    };
    env
}

#[test]
fn manual_and_smart_collections() {
    let env = Env::new();
    let ids = populate(&env, 20);
    for id in &ids[..5] {
        env.lib.set_favorite(id, true).unwrap();
    }
    for id in &ids[..3] {
        env.lib.set_tags(id, &["pack/a".into()]).unwrap();
    }
    let manual = env.lib.create_collection("Minha selecao").unwrap();
    assert_eq!(env.lib.add_to_collection(manual, &ids[10..14]).unwrap(), 4);
    assert_eq!(env.lib.add_to_collection(manual, &ids[12..16]).unwrap(), 2, "so os novos");
    assert_eq!(env.lib.collection(manual).unwrap().item_count, 6);
    assert_eq!(env.lib.count(&Query { collection: Some(manual), ..Default::default() }).unwrap(), 6);
    assert_eq!(env.lib.collections_of(&ids[10]).unwrap(), vec![manual]);
    assert_eq!(env.lib.remove_from_collection(manual, &ids[10..12]).unwrap(), 2);
    assert_eq!(env.lib.count(&Query { collection: Some(manual), ..Default::default() }).unwrap(), 4);
    // nome unico e validacao
    assert!(matches!(env.lib.create_collection("Minha selecao"), Err(AssetError::Conflict(_))));
    assert!(env.lib.create_collection("   ").is_err());
    assert!(env.lib.add_to_collection(CollectionId(9999), &ids[..1]).is_err());
    assert!(env.lib.add_to_collection(manual, &[AssetId::new()]).is_err());

    // inteligente: favoritos com a tag pack
    let smart = env.lib.create_smart_collection("Favoritos do pack", &Query { favorite: Some(true), tags: vec!["pack".into()], ..Default::default() }).unwrap();
    assert!(env.lib.collection(smart).unwrap().is_smart());
    assert_eq!(env.lib.count(&Query { collection: Some(smart), ..Default::default() }).unwrap(), 3);
    // a colecao inteligente se combina (E) com a consulta do usuario
    let q = Query { collection: Some(smart), text: Some("img001".into()), ..Default::default() };
    assert_eq!(env.lib.query(&q).unwrap().items.len(), 1);
    // e acompanha mudancas
    env.lib.set_favorite(&ids[0], false).unwrap();
    assert_eq!(env.lib.count(&Query { collection: Some(smart), ..Default::default() }).unwrap(), 2);
    // nao aceita itens manuais nem referencia a outra colecao
    assert!(env.lib.add_to_collection(smart, &ids[..1]).is_err());
    assert!(env.lib.create_smart_collection("ref", &Query { collection: Some(manual), ..Default::default() }).is_err());
    // interseccao vazia por construcao
    let tex = env.lib.create_smart_collection("So hdri", &Query { kinds: vec![AssetKind::Hdri], ..Default::default() }).unwrap();
    assert_eq!(env.lib.count(&Query { collection: Some(tex), kinds: vec![AssetKind::Texture], ..Default::default() }).unwrap(), 0);
    env.lib.update_smart_collection(smart, &Query { favorite: Some(true), ..Default::default() }).unwrap();
    assert_eq!(env.lib.count(&Query { collection: Some(smart), ..Default::default() }).unwrap(), 4);
    env.lib.rename_collection(smart, "Todos favoritos").unwrap();
    assert!(env.lib.rename_collection(smart, "Minha selecao").is_err());
    let names: Vec<String> = env.lib.collections().unwrap().into_iter().map(|c| c.name).collect();
    assert_eq!(names, vec!["Minha selecao", "So hdri", "Todos favoritos"]);
    env.lib.delete_collection(manual).unwrap();
    assert!(env.lib.collection(manual).is_err());
    assert!(env.lib.delete_collection(manual).is_err());
}

#[test]
fn state_filter_and_unavailable_flag() {
    let env = Env::new();
    let ids = populate(&env, 5);
    env.lib.trash(&ids[..2], TrashOptions::default()).unwrap();
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 3);
    assert_eq!(env.lib.count(&Query { state: StateFilter::Trashed, ..Default::default() }).unwrap(), 2);
    assert_eq!(env.lib.count(&Query { state: StateFilter::Any, ..Default::default() }).unwrap(), 5);
    assert_eq!(env.lib.count(&Query { state: StateFilter::Missing, ..Default::default() }).unwrap(), 0);
    let rows = env.lib.query(&Query::default()).unwrap().items;
    assert!(rows.iter().all(|r| r.available));
}

#[test]
fn dependencies_graph_and_cycles() {
    let env = Env::new();
    let ids = populate(&env, 4);
    let (model, mat, tex, other) = (ids[0], ids[1], ids[2], ids[3]);
    env.lib.add_dependency(&model, &mat, "material").unwrap();
    env.lib.add_dependency(&mat, &tex, "texture").unwrap();
    env.lib.add_dependency(&model, &mat, "material").unwrap(); // idempotente
    assert_eq!(env.lib.dependencies(&model).unwrap().len(), 1);
    assert_eq!(env.lib.used_by(&mat).unwrap(), vec![model]);
    assert_eq!(env.lib.used_by(&tex).unwrap(), vec![mat]);
    // ciclo e auto-referencia
    assert!(matches!(env.lib.add_dependency(&tex, &model, "x"), Err(AssetError::Conflict(_))));
    assert!(env.lib.add_dependency(&model, &model, "x").is_err());
    assert!(env.lib.add_dependency(&model, &AssetId::new(), "x").is_err());
    // grafo
    let g = env.lib.reference_graph(&model, 5, false, 100).unwrap();
    assert_eq!(g.len(), 2);
    assert!(g.contains(&(model, mat, "material".into())) && g.contains(&(mat, tex, "texture".into())));
    let g1 = env.lib.reference_graph(&model, 1, false, 100).unwrap();
    assert_eq!(g1.len(), 1);
    let gu = env.lib.reference_graph(&mat, 3, true, 100).unwrap();
    assert!(gu.contains(&(model, mat, "material".into())));
    assert_eq!(env.lib.reference_graph(&model, 5, false, 1).unwrap().len(), 1, "limite de arestas");
    // as dependencias vivem no manifesto
    assert_eq!(env.lib.manifest(&model).unwrap().dependencies.len(), 1);
    // delete seguro: material em uso por model -> bloqueado
    let rep = env.lib.trash(&[mat], TrashOptions::default()).unwrap();
    assert!(rep.trashed.is_empty());
    assert_eq!(rep.blocked, vec![(mat, vec![model])]);
    // junto com o dependente, passa
    let rep = env.lib.trash(&[mat, model], TrashOptions::default()).unwrap();
    assert_eq!(rep.trashed.len(), 2);
    // force
    let rep = env.lib.trash(&[tex], TrashOptions { force: true }).unwrap();
    assert_eq!(rep.trashed, vec![tex]);
    let _ = other;
    // remover dependencia
    env.lib.restore(&[mat, model]).unwrap();
    env.lib.remove_dependency(&model, &mat, "material").unwrap();
    assert!(env.lib.dependencies(&model).unwrap().is_empty());
    assert!(env.lib.used_by(&mat).unwrap().is_empty());
}

#[test]
fn concurrent_queries_while_importing() {
    let env = Env::new();
    let src = env.src("conc");
    for i in 0..200 {
        write(&src, &format!("c{i:03}.png"), &unique_png(i));
    }
    let job = env.lib.import(&src, ImportOptions { threads: 4, ..Default::default() });
    let mut readers = Vec::new();
    for _ in 0..4 {
        let lib = env.lib.clone();
        readers.push(std::thread::spawn(move || {
            let mut last = 0u64;
            for _ in 0..50 {
                let n = lib.count(&Query::default()).unwrap();
                assert!(n >= last || n + 64 >= last, "contagem nao deve regredir");
                last = n;
                let _ = lib.query(&Query { text: Some("c0".into()), limit: 20, ..Default::default() }).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }));
    }
    let r = job.join().unwrap();
    for t in readers {
        t.join().unwrap();
    }
    assert_eq!(r.imported.len(), 200);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 200);
}
