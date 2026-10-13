mod common;

use std::path::Path;

use anigo_assets::*;
use common::*;
use serde_json::json;

fn linked_env() -> (Env, RootId, std::path::PathBuf) {
    let env = Env::new();
    let ext = env.dir.path().join("external");
    std::fs::create_dir_all(&ext).unwrap();
    let root = env.lib.add_root(&ext, RootMode::Linked, Some("HD externo")).unwrap();
    (env, root, ext.canonicalize().unwrap())
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| (e.path().strip_prefix(dir).unwrap().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap()))
        .collect();
    v.sort();
    v
}

#[test]
fn first_scan_registers_files_without_touching_the_external_folder() {
    let (env, root, ext) = linked_env();
    write(&ext, "chars/hero.glb", &glb(1));
    write(&ext, "chars/villain.glb", &glb(2));
    write(&ext, "tex/wall.png", &png(8, 8, [9, 9, 9]));
    write(&ext, "notes.txt", b"nada");
    let before = snapshot(&ext);

    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.imported.len(), 3, "{r:?}");
    assert!(r.import.failed.is_empty());
    assert!(!r.root_offline);

    assert_eq!(before, snapshot(&ext), "a pasta externa nao pode ser alterada (nem .staging, nem manifestos)");
    // linha linked, nada copiado
    let rows = env.lib.query(&Query { storage: Some(StorageMode::Linked), ..Default::default() }).unwrap().items;
    assert_eq!(rows.len(), 3);
    let hero = rows.iter().find(|r| r.name == "hero").unwrap();
    assert_eq!(hero.storage, StorageMode::Linked);
    assert_eq!(hero.rel_path, "chars/hero.glb");
    assert_eq!(hero.root_id, root);
    let files = env.lib.resolve_files(&hero.id).unwrap();
    assert_eq!(files.primary, ext.join("chars/hero.glb"));
    assert!(files.exists);
    // manifesto fica DENTRO da biblioteca
    assert!(files.asset_dir.starts_with(env.lib.home().join("linked")));
    assert!(files.asset_dir.join("asset.json").is_file());
    assert_eq!(env.lib.root(root).unwrap().asset_count, 3);
    assert!(env.lib.root(root).unwrap().last_scan.is_some());
}

#[test]
fn second_scan_skips_unchanged_files_by_mtime_and_size() {
    let (env, root, ext) = linked_env();
    for i in 0..20 {
        write(&ext, &format!("m{i}.glb"), &glb(i));
    }
    let r1 = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r1.import.imported.len(), 20);
    let r2 = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r2.unchanged, 20);
    assert!(r2.import.imported.is_empty() && r2.import.revised.is_empty() && r2.missing.is_empty());
    assert_eq!(r2.import.bytes, 0, "nada foi re-hasheado");
    // rehash_all forca o hash, mas o resultado e o mesmo (Unchanged)
    let r3 = env.lib.scan_blocking(root, ScanOptions { rehash_all: true, ..Default::default() }).unwrap();
    assert_eq!(r3.unchanged, 0);
    assert!(r3.import.imported.is_empty() && r3.import.revised.is_empty());
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 20);
}

#[test]
fn modified_file_gets_a_new_revision_and_keeps_its_identity() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "model.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let before = env.lib.query(&Query::default()).unwrap().items[0].clone();
    env.lib.set_rating(&before.id, 4).unwrap();
    env.lib.set_tags(&before.id, &["meu/tag".into()]).unwrap();
    // altera o conteudo (tamanho e mtime mudam)
    std::fs::write(&p, glb(2)).unwrap();
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(future).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.revised, vec![before.id], "{r:?}");
    let after = env.lib.require_row(&before.id).unwrap();
    assert_eq!(after.revision, 2);
    assert_ne!(after.content_hash, before.content_hash);
    assert_eq!(after.rating, 4, "metadados do usuario sobrevivem a nova revisao");
    assert_eq!(env.lib.tags_of(&before.id).unwrap(), vec!["meu/tag"]);
    assert_eq!(env.lib.manifest(&before.id).unwrap().revisions.len(), 2);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
}

#[test]
fn deleted_file_becomes_missing_and_comes_back() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "a.glb", &glb(1));
    write(&ext, "b.glb", &glb(2));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let a = env.lib.query(&Query { text: Some("a".into()), ..Default::default() }).unwrap().items.into_iter().find(|r| r.name == "a").unwrap();
    let bytes = std::fs::read(&p).unwrap();
    std::fs::remove_file(&p).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.missing, vec![a.id]);
    assert_eq!(env.lib.require_row(&a.id).unwrap().state, AssetState::Missing);
    assert_eq!(env.lib.find_missing().unwrap().len(), 1);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1, "ausentes saem da listagem padrao");
    assert_eq!(env.lib.count(&Query { state: StateFilter::Missing, ..Default::default() }).unwrap(), 1);
    assert!(!env.lib.resolve_files(&a.id).unwrap().exists);
    // volta no mesmo caminho: restaurado
    std::fs::write(&p, &bytes).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert!(r.missing.is_empty());
    assert_eq!(env.lib.require_row(&a.id).unwrap().state, AssetState::Active, "{r:?}");
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 2);
}

#[test]
fn moved_file_is_relinked_by_hash_not_duplicated() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "old/place/hero.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let before = env.lib.query(&Query::default()).unwrap().items[0].clone();
    env.lib.set_favorite(&before.id, true).unwrap();
    std::fs::create_dir_all(ext.join("new")).unwrap();
    std::fs::rename(&p, ext.join("new/renamed.glb")).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.relinked, vec![before.id], "{r:?}");
    assert!(r.import.imported.is_empty());
    assert!(r.missing.is_empty());
    let row = env.lib.require_row(&before.id).unwrap();
    assert_eq!(row.rel_path, "new/renamed.glb");
    assert_eq!(row.state, AssetState::Active);
    assert!(row.favorite);
    assert_eq!(env.lib.count(&Query { state: StateFilter::Any, ..Default::default() }).unwrap(), 1);
    assert!(env.lib.resolve_files(&before.id).unwrap().exists);
}

#[test]
fn moves_can_be_disabled() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "hero.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    std::fs::rename(&p, ext.join("hero2.glb")).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions { detect_moves: false, ..Default::default() }).unwrap();
    assert_eq!(r.import.imported.len(), 1);
    assert_eq!(r.missing.len(), 1);
}

#[test]
fn dependents_of_linked_models_are_not_registered_as_loose_assets() {
    let (env, root, ext) = linked_env();
    write(&ext, "obj/cube.obj", CUBE_OBJ.as_bytes());
    write(&ext, "obj/cube.mtl", b"newmtl Red\nmap_Kd red.png\n");
    write(&ext, "obj/red.png", &png(8, 8, [255, 0, 0]));
    write(&ext, "loose.png", &png(8, 8, [0, 255, 0]));
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.imported.len(), 2, "{r:?}");
    // um segundo scan (sem mudancas) continua sem registrar a textura dependente
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert!(r.import.imported.is_empty(), "{r:?}");
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 2);
    let cube = env.lib.query(&Query { text: Some("cube".into()), ..Default::default() }).unwrap().items[0].clone();
    assert_eq!(env.lib.manifest(&cube.id).unwrap().revisions[0].files.len(), 3);
    // e depois de reconstruir o indice
    env.lib.rebuild_index().unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions { rehash_all: true, ..Default::default() }).unwrap();
    assert!(r.import.imported.is_empty(), "{r:?}");
}

#[test]
fn trashed_linked_asset_is_not_reimported_and_purge_never_touches_external_files() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "a.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let id = env.lib.query(&Query::default()).unwrap().items[0].id;
    env.lib.trash(&[id], TrashOptions::default()).unwrap();
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert!(r.import.imported.is_empty(), "o usuario excluiu: nao reimportar. {r:?}");
    env.lib.purge(&[id]).unwrap();
    assert!(p.is_file(), "purge de ativo linked nao pode apagar o arquivo externo");
    assert!(!env.lib.asset_dir(&id).is_ok());
    // depois do purge o arquivo e redescoberto como novo (comportamento documentado)
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.imported.len(), 1);
}

#[test]
fn offline_root_is_detected_and_not_marked_missing() {
    let (env, root, ext) = linked_env();
    write(&ext, "a.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let id = env.lib.query(&Query::default()).unwrap().items[0].id;
    let parked = env.dir.path().join("external_parked");
    std::fs::rename(&ext, &parked).unwrap(); // "desconecta o HD"
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert!(r.root_offline);
    assert!(r.missing.is_empty());
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.state, AssetState::Active, "offline != ausente");
    assert!(!row.available);
    assert!(!env.lib.root(root).unwrap().online);
    // reaparece em OUTRO caminho: relink_root
    let (gone, back) = env.lib.relink_root(root, &parked).unwrap();
    assert!(gone.is_empty() && back.is_empty());
    assert!(env.lib.root(root).unwrap().online);
    assert!(env.lib.require_row(&id).unwrap().available);
    assert!(env.lib.resolve_files(&id).unwrap().exists);
    // rejeita caminhos ruins
    assert!(env.lib.relink_root(root, &env.dir.path().join("nao_existe")).is_err());
    assert!(env.lib.relink_root(HOME_ROOT, &parked).is_err());
}

#[test]
fn check_presence_is_a_cheap_existence_pass() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "a.glb", &glb(1));
    write(&ext, "b.glb", &glb(2));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    std::fs::remove_file(&p).unwrap();
    let (gone, back) = env.lib.check_presence(root).unwrap();
    assert_eq!((gone.len(), back.len()), (1, 0));
    std::fs::write(&p, glb(1)).unwrap();
    let (gone, back) = env.lib.check_presence(root).unwrap();
    assert_eq!((gone.len(), back.len()), (0, 1));
}

#[test]
fn manual_relink_requires_identical_content() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "a.glb", &glb(1));
    env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let id = env.lib.query(&Query::default()).unwrap().items[0].id;
    let moved = ext.join("sub/a_moved.glb");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&p, &moved).unwrap();
    env.lib.check_presence(root).unwrap();
    assert_eq!(env.lib.find_missing().unwrap().len(), 1);
    // candidatos por hash
    let cands = env.lib.relink_candidates(&id, &ext, 5, &CancelToken::new()).unwrap();
    assert_eq!(cands, vec![moved.clone()]);
    // arquivo diferente e recusado
    let other = write(&ext, "sub/other.glb", &glb(9));
    assert!(matches!(env.lib.relink(&id, &other), Err(AssetError::Conflict(_))));
    // arquivo fora de qualquer raiz e recusado
    let outside = write(env.dir.path(), "outside.glb", &glb(1));
    assert!(matches!(env.lib.relink(&id, &outside), Err(AssetError::InvalidInput(_))));
    env.lib.relink(&id, &moved).unwrap();
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.state, AssetState::Active);
    assert_eq!(row.rel_path, "sub/a_moved.glb");
    assert!(env.lib.resolve_files(&id).unwrap().exists);
    // managed nao tem relink
    let m = import_one(&env.lib, &write(env.dir.path(), "m.glb", &glb(50)), ImportOptions::default());
    assert!(env.lib.relink(&m, &moved).is_err());
}

#[test]
fn scan_can_be_cancelled_and_resumed() {
    let (env, root, ext) = linked_env();
    for i in 0..400 {
        write(&ext, &format!("t{i:04}.png"), &unique_png(i));
    }
    let job = env.lib.scan(root, ScanOptions { threads: 2, ..Default::default() });
    loop {
        match job.events().recv() {
            Ok(ImportEvent::Imported { .. }) => {
                job.cancel();
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let r = job.join().unwrap();
    assert!(r.import.cancelled || r.import.imported.len() == 400);
    if r.import.cancelled {
        assert!(r.missing.is_empty(), "cancelado nunca marca ausentes");
    }
    let r2 = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 400);
    assert_eq!(r.import.imported.len() + r2.import.imported.len(), 400, "nenhum duplicado apos retomar");
}

#[test]
fn linked_import_of_a_subfolder_and_validation() {
    let (env, root, ext) = linked_env();
    let p = write(&ext, "pack/a.glb", &glb(1));
    write(&ext, "other/b.glb", &glb(2));
    let r = env.lib.import_blocking(ext.join("pack"), ImportOptions { mode: StorageMode::Linked, ..Default::default() }).unwrap();
    assert_eq!(r.imported.len(), 1);
    // reimportar o mesmo caminho nao duplica
    let r = env.lib.import_blocking(&p, ImportOptions { mode: StorageMode::Linked, ..Default::default() }).unwrap();
    assert!(r.imported.is_empty());
    // fora de qualquer raiz linked
    let outside = write(env.dir.path(), "outside.glb", &glb(3));
    assert!(matches!(env.lib.import_blocking(&outside, ImportOptions { mode: StorageMode::Linked, ..Default::default() }), Err(AssetError::InvalidInput(_))));
    // managed em raiz linked
    assert!(env.lib.import_blocking(&p, ImportOptions { root: Some(root), ..Default::default() }).is_err());
}

#[test]
fn linked_file_with_garbage_is_reported_and_skipped() {
    let (env, root, ext) = linked_env();
    write(&ext, "ok.glb", &glb(1));
    write(&ext, "broken.glb", &glb(2)[..40]);
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert_eq!(r.import.imported.len(), 1);
    assert_eq!(r.import.failed.len(), 1);
    // o arquivo quebrado nao vira "ausente" depois
    let r = env.lib.scan_blocking(root, ScanOptions::default()).unwrap();
    assert!(r.missing.is_empty());
}

#[test]
fn unused_helpers_compile() {
    let _ = json!({});
}
