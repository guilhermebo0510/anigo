mod common;

use anigo_assets::*;
use common::*;

fn import_n(env: &Env, n: u32) -> Vec<AssetId> {
    let src = env.src("n");
    for i in 0..n {
        write(&src, &format!("g{i:03}.glb"), &glb(i));
    }
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert_eq!(r.imported.len(), n as usize);
    let mut rows = env.lib.query(&Query::default()).unwrap().items;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.into_iter().map(|r| r.id).collect()
}

fn wipe_index(env: Env) -> Env {
    let idx = env.lib.index_path().to_path_buf();
    let home = env.lib.home().to_path_buf();
    let dir = env.dir;
    drop(env.lib);
    for ext in ["", "-wal", "-shm"] {
        let mut p = idx.clone().into_os_string();
        p.push(ext);
        let _ = std::fs::remove_file(p);
    }
    Env { dir, lib: Library::open(home).unwrap() }
}

// ---------------------------------------------------------------------------------------------
// Lixeira
// ---------------------------------------------------------------------------------------------

#[test]
fn trash_restore_purge_roundtrip_on_disk() {
    let env = Env::new();
    let ids = import_n(&env, 3);
    let id = ids[0];
    let dir_before = env.lib.asset_dir(&id).unwrap();
    assert!(dir_before.starts_with(env.lib.home().join("assets")));

    let rep = env.lib.trash(&[id], TrashOptions::default()).unwrap();
    assert_eq!(rep.trashed, vec![id]);
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.state, AssetState::Trashed);
    let tdir = env.lib.asset_dir(&id).unwrap();
    assert!(tdir.starts_with(env.lib.home().join("trash")));
    assert!(tdir.join("asset.json").is_file());
    assert!(!dir_before.exists(), "saiu do lugar original");
    assert!(env.lib.manifest(&id).unwrap().trashed_at.is_some());
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 2);
    assert!(env.lib.resolve_files(&id).unwrap().exists, "os arquivos continuam acessiveis na lixeira");
    // trash duplo e idempotente
    assert_eq!(env.lib.trash(&[id], TrashOptions::default()).unwrap().already, vec![id]);
    // purge exige lixeira
    assert!(env.lib.purge(&[ids[1]]).is_err());

    // restaurar
    assert_eq!(env.lib.restore(&[id]).unwrap(), vec![id]);
    assert_eq!(env.lib.require_row(&id).unwrap().state, AssetState::Active);
    assert_eq!(env.lib.asset_dir(&id).unwrap(), dir_before);
    assert!(env.lib.manifest(&id).unwrap().trashed_at.is_none());
    assert!(env.lib.restore(&[id]).unwrap().is_empty(), "restaurar o que nao esta na lixeira e no-op");

    // purge de verdade
    env.lib.trash(&[id], TrashOptions::default()).unwrap();
    let tdir = env.lib.asset_dir(&id).unwrap();
    let rep = env.lib.purge(&[id]).unwrap();
    assert_eq!(rep.purged, vec![id]);
    assert!(rep.bytes_freed > 0);
    assert!(!tdir.exists());
    assert!(env.lib.row(&id).unwrap().is_none());
    assert!(env.lib.get(&id).is_err());
    assert_eq!(env.lib.count(&Query { state: StateFilter::Any, ..Default::default() }).unwrap(), 2);
    // FTS e tags tambem foram limpos
    assert_eq!(env.lib.query(&Query { text: Some("g000".into()), state: StateFilter::Any, ..Default::default() }).unwrap().items.len(), 0);
}

#[test]
fn empty_trash_respects_age_and_collections_forget_purged_assets() {
    let env = Env::new();
    let ids = import_n(&env, 4);
    let coll = env.lib.create_collection("c").unwrap();
    env.lib.add_to_collection(coll, &ids).unwrap();
    env.lib.trash(&ids[..3], TrashOptions::default()).unwrap();
    // nada e "velho" ainda
    assert!(env.lib.empty_trash(Some(3600)).unwrap().purged.is_empty());
    assert_eq!(env.lib.count(&Query { state: StateFilter::Trashed, ..Default::default() }).unwrap(), 3);
    let rep = env.lib.empty_trash(None).unwrap();
    assert_eq!(rep.purged.len(), 3);
    assert_eq!(env.lib.collection(coll).unwrap().item_count, 1, "item purgado sai da colecao");
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
}

#[test]
fn restoring_keeps_collection_membership_and_metadata() {
    let env = Env::new();
    let ids = import_n(&env, 2);
    let coll = env.lib.create_collection("c").unwrap();
    env.lib.add_to_collection(coll, &ids[..1]).unwrap();
    env.lib.set_rating(&ids[0], 5).unwrap();
    env.lib.set_tags(&ids[0], &["a/b".into()]).unwrap();
    env.lib.trash(&ids[..1], TrashOptions::default()).unwrap();
    assert_eq!(env.lib.count(&Query { collection: Some(coll), ..Default::default() }).unwrap(), 0, "lixeira some da colecao");
    env.lib.restore(&ids[..1]).unwrap();
    assert_eq!(env.lib.count(&Query { collection: Some(coll), ..Default::default() }).unwrap(), 1);
    assert_eq!(env.lib.require_row(&ids[0]).unwrap().rating, 5);
    assert_eq!(env.lib.tags_of(&ids[0]).unwrap(), vec!["a/b"]);
}

#[test]
fn purging_a_dependency_cleans_dependents_manifests() {
    let env = Env::new();
    let ids = import_n(&env, 2);
    env.lib.add_dependency(&ids[0], &ids[1], "material").unwrap();
    env.lib.trash(&[ids[1]], TrashOptions { force: true }).unwrap();
    env.lib.purge(&[ids[1]]).unwrap();
    assert!(env.lib.dependencies(&ids[0]).unwrap().is_empty());
    assert!(env.lib.manifest(&ids[0]).unwrap().dependencies.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Reconstrucao do indice
// ---------------------------------------------------------------------------------------------

#[test]
fn deleting_the_index_loses_nothing() {
    let env = Env::new();
    let src = env.src("mix");
    write(&src, "a.glb", &glb(1));
    write(&src, "b.glb", &glb(2));
    write(&src, "c.png", &unique_png(3));
    let r = env.lib.import_blocking(&src, ImportOptions { tags: vec!["lote/um".into()], ..Default::default() }).unwrap();
    let ids = r.imported.clone();
    env.lib.set_favorite(&ids[0], true).unwrap();
    env.lib.set_rating(&ids[1], 4).unwrap();
    env.lib.add_dependency(&ids[0], &ids[1], "texture").unwrap();
    env.lib.trash(&[ids[2]], TrashOptions::default()).unwrap();
    let coll = env.lib.create_collection("Manual").unwrap();
    env.lib.add_to_collection(coll, &ids[..2]).unwrap();
    let smart = env.lib.create_smart_collection("Fav", &Query { favorite: Some(true), ..Default::default() }).unwrap();

    let snap = |lib: &Library| -> Vec<AssetRow> {
        let mut v = lib.query(&Query { state: StateFilter::Any, limit: 1000, ..Default::default() }).unwrap().items;
        for r in &mut v {
            r.seq = 0; // seq nao e estavel entre reconstrucoes
        }
        v.sort_by_key(|r| r.id);
        v
    };
    let before = snap(&env.lib);
    let tags_before = env.lib.tags().unwrap();
    let env = wipe_index(env);
    assert_eq!(snap(&env.lib), before, "mesmas linhas apos reconstruir");
    assert_eq!(env.lib.tags().unwrap().iter().map(|t| (&t.path, t.count)).collect::<Vec<_>>(), tags_before.iter().map(|t| (&t.path, t.count)).collect::<Vec<_>>());
    assert_eq!(env.lib.dependencies(&ids[0]).unwrap().len(), 1);
    assert_eq!(env.lib.used_by(&ids[1]).unwrap(), vec![ids[0]]);
    assert_eq!(env.lib.require_row(&ids[2]).unwrap().state, AssetState::Trashed);
    // colecoes restauradas do snapshot
    let colls = env.lib.collections().unwrap();
    assert_eq!(colls.len(), 2);
    assert_eq!(env.lib.count(&Query { collection: Some(coll), ..Default::default() }).unwrap(), 2);
    assert_eq!(env.lib.count(&Query { collection: Some(smart), ..Default::default() }).unwrap(), 1);
    // FTS funciona
    assert_eq!(env.lib.query(&Query { text: Some("tag:lote".into()), ..Default::default() }).unwrap().items.len(), 2);
    // e o explicito: rebuild_index no indice saudavel e idempotente
    let rep = env.lib.rebuild_index().unwrap();
    assert_eq!(rep.assets, 3);
    assert_eq!(rep.trashed, 1);
    assert!(rep.bad_manifests.is_empty());
    assert_eq!(snap(&env.lib), before);
}

#[test]
fn rebuild_reports_corrupt_manifests_and_keeps_going() {
    let env = Env::new();
    let ids = import_n(&env, 3);
    let bad = env.lib.asset_dir(&ids[1]).unwrap().join("asset.json");
    std::fs::write(&bad, b"{ truncated").unwrap();
    // manifesto valido em JSON mas com revisao inexistente
    let bad2 = env.lib.asset_dir(&ids[2]).unwrap().join("asset.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&bad2).unwrap()).unwrap();
    m["current_revision"] = serde_json::json!(42);
    std::fs::write(&bad2, serde_json::to_vec(&m).unwrap()).unwrap();
    let rep = env.lib.rebuild_index().unwrap();
    assert_eq!(rep.assets, 1);
    assert_eq!(rep.bad_manifests.len(), 2);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    // a pasta do ativo ruim continua la (nada e apagado)
    assert!(bad.is_file() && bad2.is_file());
}

#[test]
fn managed_root_added_later_adopts_existing_assets() {
    let env = Env::new();
    let second = env.dir.path().join("second_lib_root");
    let root = env.lib.add_root(&second, RootMode::Managed, Some("Segundo disco")).unwrap();
    let src = env.src("x");
    let p = write(&src, "a.glb", &glb(1));
    let id = import_one(&env.lib, &p, ImportOptions { root: Some(root), ..Default::default() });
    assert_eq!(env.lib.require_row(&id).unwrap().root_id, root);
    assert!(env.lib.resolve_files(&id).unwrap().primary.starts_with(&second));
    // esquecer a raiz sem apagar arquivos
    assert_eq!(env.lib.remove_root(root, false).unwrap(), 1);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 0);
    assert!(second.join("assets").is_dir());
    // readicionar recupera os ativos
    let root2 = env.lib.add_root(&second, RootMode::Managed, None).unwrap();
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    assert_eq!(env.lib.require_row(&id).unwrap().root_id, root2);
    // e apagar de verdade
    assert_eq!(env.lib.remove_root(root2, true).unwrap(), 1);
    assert!(second.join("assets").read_dir().map(|mut d| d.next().is_none()).unwrap_or(true), "arquivos apagados e pastas de shard vazias podadas");
}

#[test]
fn removing_a_linked_root_forgets_assets_and_keeps_files() {
    let env = Env::new();
    let ext = env.dir.path().join("ext");
    std::fs::create_dir_all(&ext).unwrap();
    let p = write(&ext, "a.glb", &glb(1));
    let root = env.lib.add_root(&ext, RootMode::Linked, None).unwrap();
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    assert_eq!(env.lib.remove_root(root, true).unwrap(), 1, "delete_files e ignorado em raiz linked");
    assert!(p.is_file());
    assert_eq!(env.lib.count(&Query { state: StateFilter::Any, ..Default::default() }).unwrap(), 0);
    assert_eq!(std::fs::read_dir(env.lib.home().join("linked")).map(|d| d.count()).unwrap_or(0), 0, "manifestos do linked removidos");
}

// ---------------------------------------------------------------------------------------------
// Integridade
// ---------------------------------------------------------------------------------------------

#[test]
fn verify_detects_managed_corruption_and_fix_marks_state() {
    let env = Env::new();
    let ids = import_n(&env, 4);
    let ok = env.lib.verify_blocking(VerifyOptions::default()).unwrap();
    assert_eq!(ok.checked, 4);
    assert_eq!(ok.ok, 4);
    assert!(ok.issues.is_empty(), "{:?}", ok.issues);
    assert!(ok.bytes_hashed > 0);

    // 1) bit flip (mesmo tamanho) 2) truncado 3) apagado
    let f0 = env.lib.resolve_files(&ids[0]).unwrap().primary;
    let mut b = std::fs::read(&f0).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0xFF;
    std::fs::write(&f0, &b).unwrap();
    let f1 = env.lib.resolve_files(&ids[1]).unwrap().primary;
    std::fs::write(&f1, &b[..b.len() / 2]).unwrap();
    let f2 = env.lib.resolve_files(&ids[2]).unwrap().primary;
    std::fs::remove_file(&f2).unwrap();

    let rep = env.lib.verify_blocking(VerifyOptions::default()).unwrap();
    let kinds = |id: &AssetId| -> Vec<IssueKind> { rep.issues.iter().filter(|i| i.asset == Some(*id)).map(|i| i.kind).collect() };
    assert_eq!(kinds(&ids[0]), vec![IssueKind::HashMismatch]);
    assert_eq!(kinds(&ids[1]), vec![IssueKind::SizeMismatch]);
    assert_eq!(kinds(&ids[2]), vec![IssueKind::MissingFile]);
    assert!(kinds(&ids[3]).is_empty());
    assert_eq!(rep.ok, 1);
    // sem rehash, o bit flip passa (so tamanho/existencia)
    let fast = env.lib.verify_blocking(VerifyOptions { rehash: false, fix: false }).unwrap();
    assert!(fast.issues.iter().all(|i| i.asset != Some(ids[0])));
    assert_eq!(fast.bytes_hashed, 0);
    // verificacao sem fix nao muda estados
    assert_eq!(env.lib.require_row(&ids[0]).unwrap().state, AssetState::Active);
    // fix marca corrupt e tira da listagem padrao
    let fixed = env.lib.verify_blocking(VerifyOptions { rehash: true, fix: true }).unwrap();
    assert_eq!(fixed.fixed, 3);
    for id in &ids[..3] {
        assert_eq!(env.lib.require_row(id).unwrap().state, AssetState::Corrupt);
    }
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    assert_eq!(env.lib.count(&Query { state: StateFilter::Corrupt, ..Default::default() }).unwrap(), 3);
    // restaura o conteudo -> fix reativa
    b[last] ^= 0xFF;
    std::fs::write(&f0, &b).unwrap();
    let again = env.lib.verify_blocking(VerifyOptions { rehash: true, fix: true }).unwrap();
    assert!(again.issues.iter().all(|i| i.asset != Some(ids[0])));
    assert_eq!(env.lib.require_row(&ids[0]).unwrap().state, AssetState::Active);
}

#[test]
fn verify_finds_orphans_in_both_directions_and_fix_reconciles() {
    let env = Env::new();
    let ids = import_n(&env, 3);
    // 1) manifesto em disco que o indice nao conhece
    let idx_only = ids[0];
    env.lib.trash(&[ids[1]], TrashOptions::default()).unwrap(); // so para variar o layout
    let dir0 = env.lib.asset_dir(&idx_only).unwrap();
    // simula perda de linha: apaga a linha do indice via reconstrucao parcial (manifesto fica)
    {
        // abre uma segunda conexao e apaga a linha (como um indice desatualizado)
        let conn = rusqlite::Connection::open(env.lib.index_path()).unwrap();
        conn.execute("DELETE FROM asset WHERE id = ?1", [idx_only.as_bytes().as_slice()]).unwrap();
    }
    // 2) linha do indice sem pasta
    std::fs::remove_dir_all(env.lib.asset_dir(&ids[2]).unwrap()).unwrap();
    assert!(dir0.exists());
    let rep = env.lib.verify_blocking(VerifyOptions { rehash: false, fix: false }).unwrap();
    let k: Vec<IssueKind> = rep.issues.iter().map(|i| i.kind).collect();
    assert!(k.contains(&IssueKind::OrphanManifest), "{:?}", rep.issues);
    assert!(k.contains(&IssueKind::OrphanRow), "{:?}", rep.issues);
    let rep = env.lib.verify_blocking(VerifyOptions { rehash: false, fix: true }).unwrap();
    assert!(rep.fixed >= 2);
    // reconciliado: agora o indice == disco
    let rep = env.lib.verify_blocking(VerifyOptions { rehash: false, fix: false }).unwrap();
    assert!(rep.issues.iter().all(|i| !matches!(i.kind, IssueKind::OrphanManifest | IssueKind::OrphanRow)), "{:?}", rep.issues);
    assert!(env.lib.row(&idx_only).unwrap().is_some());
    assert!(env.lib.row(&ids[2]).unwrap().is_none());
}

#[test]
fn verify_can_be_cancelled_and_emits_events() {
    let env = Env::new();
    import_n(&env, 30);
    let job = env.lib.verify(VerifyOptions::default());
    let mut started = false;
    let mut finished = false;
    while let Ok(ev) = job.events().recv() {
        match ev {
            VerifyEvent::Started { total } => {
                assert_eq!(total, 30);
                started = true;
            }
            VerifyEvent::Finished => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(started && finished);
    let rep = job.join().unwrap();
    assert_eq!(rep.checked, 30);
    // cancelamento antes de comecar
    let job = env.lib.verify(VerifyOptions::default());
    job.cancel();
    let rep = job.join().unwrap();
    assert!(rep.cancelled || rep.checked == 30);
}

#[test]
fn verify_reports_changed_external_files_without_marking_them_corrupt() {
    let env = Env::new();
    let ext = env.dir.path().join("ext");
    std::fs::create_dir_all(&ext).unwrap();
    let p = write(&ext, "a.glb", &glb(1));
    let root = env.lib.add_root(&ext, RootMode::Linked, None).unwrap();
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let id = env.lib.query(&Query::default()).unwrap().items[0].id;
    // mesmo tamanho, conteudo diferente
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 1;
    b[last] ^= 1;
    std::fs::write(&p, &b).unwrap();
    let rep = env.lib.verify_blocking(VerifyOptions { rehash: true, fix: true }).unwrap();
    assert!(rep.issues.iter().any(|i| i.asset == Some(id) && i.kind == IssueKind::ExternalChanged));
    assert_eq!(env.lib.require_row(&id).unwrap().state, AssetState::Active);
    // arquivo externo apagado -> missing
    std::fs::remove_file(&p).unwrap();
    let rep = env.lib.verify_blocking(VerifyOptions { rehash: true, fix: true }).unwrap();
    assert!(rep.issues.iter().any(|i| i.asset == Some(id) && i.kind == IssueKind::MissingFile));
    assert_eq!(env.lib.require_row(&id).unwrap().state, AssetState::Missing);
    // raiz offline gera aviso, nao erro de arquivo
    std::fs::remove_dir_all(&ext).unwrap();
    env.lib.refresh_roots().unwrap();
    let rep = env.lib.verify_blocking(VerifyOptions::default()).unwrap();
    assert!(rep.issues.iter().any(|i| i.kind == IssueKind::RootOffline));
}

// ---------------------------------------------------------------------------------------------
// Duplicados
// ---------------------------------------------------------------------------------------------

#[test]
fn resolving_duplicates_merges_metadata_into_the_keeper() {
    let env = Env::new();
    let src = env.src("d");
    let p = write(&src, "a.glb", &glb(1));
    let keep = import_one(&env.lib, &p, ImportOptions { tags: vec!["t/um".into()], ..Default::default() });
    let dup = import_one(&env.lib, &p, ImportOptions { duplicates: DuplicatePolicy::Allow, tags: vec!["t/dois".into()], ..Default::default() });
    let user = import_one(&env.lib, &write(&src, "u.glb", &glb(2)), ImportOptions::default());
    env.lib.set_favorite(&dup, true).unwrap();
    env.lib.set_rating(&dup, 4).unwrap();
    env.lib.add_dependency(&user, &dup, "material").unwrap();
    let coll = env.lib.create_collection("c").unwrap();
    env.lib.add_to_collection(coll, &[dup]).unwrap();

    let groups = env.lib.find_duplicates(10).unwrap();
    assert_eq!(groups.len(), 1);
    assert!(matches!(env.lib.resolve_duplicates(&keep, &[user]), Err(AssetError::InvalidInput(_))), "hash diferente");
    assert!(env.lib.resolve_duplicates(&keep, &[keep]).is_err());
    env.lib.resolve_duplicates(&keep, &[dup]).unwrap();

    assert_eq!(env.lib.require_row(&dup).unwrap().state, AssetState::Trashed);
    let k = env.lib.require_row(&keep).unwrap();
    assert!(k.favorite);
    assert_eq!(k.rating, 4);
    assert_eq!(env.lib.tags_of(&keep).unwrap(), vec!["t/dois", "t/um"]);
    assert_eq!(env.lib.dependencies(&user).unwrap()[0].id, keep, "dependentes passam para o ativo mantido");
    assert_eq!(env.lib.collections_of(&keep).unwrap(), vec![coll]);
    assert!(env.lib.find_duplicates(10).unwrap().is_empty());
}

#[test]
fn duplicates_report_orders_by_waste() {
    let env = Env::new();
    let src = env.src("w");
    let small = write(&src, "s.png", &png(8, 8, [1, 1, 1]));
    let big = write(&src, "b.png", &png(256, 256, [200, 100, 50]));
    for p in [&small, &big] {
        import_one(&env.lib, p, ImportOptions::default());
        import_one(&env.lib, p, ImportOptions { duplicates: DuplicatePolicy::Allow, ..Default::default() });
    }
    let g = env.lib.find_duplicates(10).unwrap();
    assert_eq!(g.len(), 2);
    assert!(g[0].size >= g[1].size);
    assert_eq!(env.lib.find_duplicates(1).unwrap().len(), 1);
}
