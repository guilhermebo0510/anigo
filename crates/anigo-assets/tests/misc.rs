mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anigo_assets::*;
use common::*;
use serde_json::json;

// ---------------------------------------------------------------------------------------------
// Miniaturas via biblioteca (sem GPU)
// ---------------------------------------------------------------------------------------------

#[test]
fn batch_thumbnails_with_cpu_renderers() {
    let env = Env::new();
    let src = env.src("t");
    for i in 0..6 {
        write(&src, &format!("tex{i}.png"), &unique_png(i));
    }
    let mut mat = ToonMaterial::named("Pele");
    mat.base.color = [0.9, 0.6, 0.5, 1.0];
    write(&src, "pele.anigomat.json", &mat.to_json().unwrap());
    write(&src, "model.glb", &glb(1));
    env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    let ids = env.lib.query_ids(&Query::default(), 100).unwrap();
    assert_eq!(ids.len(), 8);
    assert_eq!(env.lib.assets_without_thumbnail(&Query::default(), 100).unwrap().len(), 8);

    let job = env.lib.generate_thumbnails(ids.clone(), default_cpu_renderer(), ThumbOptions { parallelism: 2, force: false });
    let rep = job.join().unwrap();
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert_eq!(rep.generated, 7, "6 texturas + 1 material (esfera toon)");
    assert_eq!(rep.placeholders, 1, "o GLB cai no placeholder: o render 3D e do studio");
    // so os reais ficam no cache de disco
    let cache = env.lib.thumbnails();
    let rows = env.lib.query(&Query::default()).unwrap().items;
    for r in &rows {
        let cached = cache.cached_path(&r.content_hash, 128).is_some();
        assert_eq!(cached, r.kind != AssetKind::Model, "{:?}", r.kind);
    }
    assert_eq!(env.lib.assets_without_thumbnail(&Query::default(), 100).unwrap().len(), 1);
    // segunda rodada: tudo vem do cache
    let rep = env.lib.generate_thumbnails(ids.clone(), default_cpu_renderer(), ThumbOptions::default()).join().unwrap();
    assert_eq!(rep.already_cached, 7);
    assert_eq!(rep.generated, 0);
    // force re-renderiza
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let counting = Arc::new(ClosureRenderer::new("count", move |req: &ThumbRequest| {
        c2.fetch_add(1, Ordering::SeqCst);
        Ok(Rendered { image: image::RgbaImage::from_pixel(req.size, req.size, image::Rgba([1, 1, 1, 255])), placeholder: false })
    }));
    env.lib.generate_thumbnails(ids.clone(), counting.clone(), ThumbOptions { parallelism: 1, force: true }).join().unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 8);
    // o texto do material aparece como a esfera (nao como placeholder)
    let m = rows.iter().find(|r| r.kind == AssetKind::Material).unwrap();
    assert!(cache.load(&m.content_hash, 256).is_some());
}

#[test]
fn thumbnail_failures_are_per_asset_and_cancel_works() {
    let env = Env::new();
    let src = env.src("t");
    for i in 0..10 {
        write(&src, &format!("tex{i}.png"), &unique_png(i));
    }
    env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    let ids = env.lib.query_ids(&Query::default(), 100).unwrap();
    // apaga um arquivo gerenciado: so aquele ativo falha
    std::fs::remove_file(env.lib.resolve_files(&ids[3]).unwrap().primary).unwrap();
    let rep = env.lib.generate_thumbnails(ids.clone(), default_cpu_renderer(), ThumbOptions::default()).join().unwrap();
    // sem o arquivo o renderer de imagem falha e o placeholder assume; nada aborta
    assert!(rep.failed.is_empty());
    assert_eq!(rep.generated, 9);
    assert_eq!(rep.placeholders, 1);
    // id inexistente falha sozinho
    let rep = env.lib.generate_thumbnails(vec![AssetId::new(), ids[0]], default_cpu_renderer(), ThumbOptions::default()).join().unwrap();
    assert_eq!(rep.failed.len(), 1);
    // cancelamento
    let slow = Arc::new(ClosureRenderer::new("slow", |req: &ThumbRequest| {
        std::thread::sleep(std::time::Duration::from_millis(30));
        Ok(Rendered { image: image::RgbaImage::from_pixel(req.size, req.size, image::Rgba([9, 9, 9, 255])), placeholder: false })
    }));
    let job = env.lib.generate_thumbnails(ids.clone(), slow, ThumbOptions { parallelism: 1, force: true });
    std::thread::sleep(std::time::Duration::from_millis(40));
    job.cancel();
    let rep = job.join().unwrap();
    assert!(rep.cancelled);
    assert!(rep.generated < 10);
}

#[test]
fn purging_an_asset_removes_its_cached_thumbnails() {
    let env = Env::new();
    let src = env.src("t");
    let id = import_one(&env.lib, &write(&src, "a.png", &unique_png(1)), ImportOptions::default());
    env.lib.generate_thumbnails(vec![id], default_cpu_renderer(), ThumbOptions::default()).join().unwrap();
    let h = env.lib.require_row(&id).unwrap().content_hash;
    let cache = env.lib.thumbnails();
    assert!(cache.cached_path(&h, 64).is_some());
    env.lib.trash(&[id], TrashOptions::default()).unwrap();
    assert!(cache.cached_path(&h, 64).is_some(), "lixeira ainda pode restaurar: cache fica");
    env.lib.purge(&[id]).unwrap();
    assert!(cache.cached_path(&h, 64).is_none());
}

#[test]
fn changing_pipeline_ver_invalidates_thumbnails() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("lib");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let p = write(&src, "a.png", &unique_png(1));
    let lib = Library::open(&home).unwrap();
    let id = import_one(&lib, &p, ImportOptions::default());
    lib.generate_thumbnails(vec![id], default_cpu_renderer(), ThumbOptions::default()).join().unwrap();
    assert!(lib.assets_without_thumbnail(&Query::default(), 10).unwrap().is_empty());
    drop(lib);
    let lib = Library::open_with(&home, OpenOptions { pipeline_ver: 2, ..Default::default() }).unwrap();
    assert_eq!(lib.assets_without_thumbnail(&Query::default(), 10).unwrap(), vec![id], "novo pipeline_ver = cache invalido");
    assert!(lib.thumbnails().purge_stale(None) > 0, "versoes antigas podem ser varridas");
}

// ---------------------------------------------------------------------------------------------
// Seguranca
// ---------------------------------------------------------------------------------------------

#[test]
fn malicious_gltf_cannot_read_files_outside_its_folder() {
    let env = Env::new();
    let secret = write(env.dir.path(), "secret.bin", &[7u8; 42]);
    assert!(secret.exists());
    let src = env.src("evil");
    for (i, uri) in ["../../secret.bin", "../secret.bin", "/etc/passwd", "file:///etc/passwd", "..%2f..%2fsecret.bin", "C:\\Windows\\win.ini", "\\\\server\\share\\x"].iter().enumerate() {
        let (mut j, _) = triangle_json(None);
        j["buffers"][0]["uri"] = json!(uri);
        write(&src, &format!("evil{i}.gltf"), &serde_json::to_vec(&j).unwrap());
    }
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert!(r.imported.is_empty(), "{r:?}");
    assert_eq!(r.failed.len(), 7);
    // nada do segredo foi copiado para a biblioteca
    for e in walkdir::WalkDir::new(env.lib.home()).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()) {
        assert_ne!(std::fs::read(e.path()).unwrap_or_default(), vec![7u8; 42], "{}", e.path().display());
    }
}

#[test]
fn manifest_with_escaping_paths_is_rejected_on_load() {
    let env = Env::new();
    let src = env.src("m");
    let id = import_one(&env.lib, &write(&src, "a.glb", &glb(1)), ImportOptions::default());
    let dir = env.lib.asset_dir(&id).unwrap();
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("asset.json")).unwrap()).unwrap();
    m["revisions"][0]["files"][0]["path"] = json!("../../../../etc/passwd");
    std::fs::write(dir.join("asset.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(matches!(env.lib.manifest(&id), Err(AssetError::Corrupt(_))));
    let rep = env.lib.rebuild_index().unwrap();
    assert_eq!(rep.bad_manifests.len(), 1);
    // load_material nao aceita separadores nem ..
    for bad in ["../x.anigomat.json", "a/b.anigomat.json", "..\\x.anigomat.json", "x.json"] {
        assert!(env.lib.load_material(&id, bad).is_err(), "{bad}");
    }
}

#[test]
fn tampered_index_cannot_make_purge_delete_outside_the_trash() {
    let env = Env::new();
    let src = env.src("m");
    let id = import_one(&env.lib, &write(&src, "a.glb", &glb(1)), ImportOptions::default());
    let victim = env.dir.path().join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("important.txt"), "x").unwrap();
    env.lib.trash(&[id], TrashOptions::default()).unwrap();
    {
        let conn = rusqlite::Connection::open(env.lib.index_path()).unwrap();
        conn.execute("UPDATE asset SET manifest_dir = '../../victim' WHERE id = ?1", [id.as_bytes().as_slice()]).unwrap();
    }
    assert!(matches!(env.lib.purge(&[id]), Err(AssetError::UnsafePath(_))));
    assert!(victim.join("important.txt").exists());
}

#[test]
fn hostile_search_text_never_errors() {
    let env = Env::new();
    let src = env.src("m");
    import_one(&env.lib, &write(&src, "a.glb", &glb(1)), ImportOptions::default());
    for evil in [
        "\"", "\"\"\"", "*", "(", ")", "AND", "OR NOT", "NEAR(a b, 5)", "a:b:c", "-", "--", "- -", "name:", "name:\"", "{", "}", "^a", "a*", "'; DROP TABLE asset; --",
        "\u{0}", "\u{202e}rtl", "\u{1F4A9}", "tag:", "tags:\"x", &"a ".repeat(1000), &"\"".repeat(999), "col:x", "rowid:1",
    ] {
        let r = env.lib.query(&Query { text: Some(evil.to_string()), ..Default::default() });
        assert!(r.is_ok(), "texto {evil:?} gerou erro: {r:?}");
    }
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1, "a tabela continua la");
}

#[test]
fn zero_byte_and_huge_declared_sizes_do_not_blow_up() {
    let env = Env::new();
    let src = env.src("z");
    write(&src, "zero.glb", b"");
    write(&src, "zero.obj", b"");
    write(&src, "zero.png", b"");
    write(&src, "zero.fbx", b"");
    write(&src, "zero.anigomat.json", b"");
    // GLB cujo cabecalho declara 4 GiB
    let mut huge = b"glTF".to_vec();
    huge.extend_from_slice(&2u32.to_le_bytes());
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    huge.extend_from_slice(b"JSON{}");
    write(&src, "huge.glb", &huge);
    // glTF com array de milhoes de nos declarados em JSON pequeno nao existe; testa limite configurado
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert!(r.imported.is_empty());
    assert_eq!(r.failed.len(), 6, "{:?}", r.failed);
}

#[test]
fn configured_limits_apply_to_imports() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_with(dir.path().join("lib"), OpenOptions { limits: Limits { max_file_bytes: 100, ..Limits::default() }, ..Default::default() }).unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let p = write(&src, "big.glb", &glb(1));
    let r = lib.import_blocking(&p, ImportOptions::default()).unwrap();
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].1.contains("limite"), "{}", r.failed[0].1);
    let lib2 = Library::open_with(dir.path().join("lib2"), OpenOptions { limits: Limits { max_files_per_asset: 1, ..Limits::default() }, ..Default::default() }).unwrap();
    write(&src, "cube.obj", CUBE_OBJ.as_bytes());
    write(&src, "cube.mtl", b"newmtl Red\nmap_Kd red.png\n");
    write(&src, "red.png", &unique_png(1));
    let r = lib2.import_blocking(src.join("cube.obj"), ImportOptions::default()).unwrap();
    assert_eq!(r.failed.len(), 1, "obj + mtl + png > 1 arquivo por ativo");
}

#[cfg(unix)]
#[test]
fn symlinks_in_source_folders_are_not_followed() {
    let env = Env::new();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "ext.glb", &glb(77));
    let src = env.src("links");
    write(&src, "real.glb", &glb(1));
    std::os::unix::fs::symlink(outside.path(), src.join("lnk")).unwrap();
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert_eq!(r.imported.len(), 1, "o symlink de diretorio nao e seguido: {r:?}");
}

// ---------------------------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "watch")]
#[test]
fn watcher_triggers_incremental_scan_on_new_files() {
    let env = Env::new();
    let ext = env.dir.path().join("ext");
    std::fs::create_dir_all(&ext).unwrap();
    let root = env.lib.add_root(&ext, RootMode::Linked, None).unwrap();
    let w = match env.lib.watch(root, std::time::Duration::from_millis(200)) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("watch indisponivel neste ambiente (inotify?): {e}; teste pulado");
            return;
        }
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    write(&ext, "novo.glb", &glb(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut imported = 0;
    while std::time::Instant::now() < deadline && imported == 0 {
        if let Ok(WatchEvent::Scanned(rep)) = w.events().recv_timeout(std::time::Duration::from_millis(200)) {
            imported += rep.import.imported.len();
        }
    }
    assert_eq!(imported, 1, "o watcher deveria ter disparado um scan que importou o arquivo");
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    drop(w); // nao pode travar
    // roots managed e offline sao recusadas
    assert!(env.lib.watch(HOME_ROOT, std::time::Duration::from_millis(100)).is_err());
}

#[test]
fn api_is_send_sync_and_clone() {
    fn is_send_sync<T: Send + Sync + Clone>() {}
    is_send_sync::<Library>();
    fn is_send<T: Send>() {}
    is_send::<ImportJob>();
    is_send::<ScanJob>();
    is_send::<ThumbJob>();
    is_send::<VerifyJob>();
}
