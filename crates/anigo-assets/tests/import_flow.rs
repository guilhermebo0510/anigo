mod common;

use anigo_assets::*;
use common::*;
use serde_json::json;

#[test]
fn imports_a_glb_end_to_end() {
    let env = Env::new();
    let src = env.src("a");
    let p = write(&src, "Hero Model.glb", &glb(1));
    let id = import_one(&env.lib, &p, ImportOptions { tags: vec!["Estilo/Cel".into(), "  heroi ".into()], author: Some("Ana".into()), ..Default::default() });

    let d = env.lib.get(&id).unwrap();
    assert_eq!(d.row.name, "Hero Model");
    assert_eq!(d.row.kind, AssetKind::Model);
    assert_eq!(d.row.format, "glb");
    assert_eq!(d.row.storage, StorageMode::Managed);
    assert_eq!(d.row.state, AssetState::Active);
    assert_eq!(d.row.tris, 1);
    assert_eq!(d.row.verts, 3);
    assert_eq!(d.row.revision, 1);
    assert_eq!(d.manifest.author, "Ana");
    assert_eq!(d.manifest.tags, vec!["estilo/cel", "heroi"]);
    assert_eq!(env.lib.tags_of(&id).unwrap(), vec!["estilo/cel", "heroi"]);
    // hash do conteudo == blake3 do arquivo original; arquivo copiado identico
    let bytes = std::fs::read(&p).unwrap();
    assert_eq!(d.row.content_hash.0, *blake3::hash(&bytes).as_bytes());
    let files = env.lib.resolve_files(&id).unwrap();
    assert!(files.exists);
    assert_eq!(std::fs::read(&files.primary).unwrap(), bytes);
    assert!(files.primary.starts_with(env.lib.home().join("assets")));
    // manifesto e a verdade: esta em disco ao lado
    assert!(files.asset_dir.join("asset.json").is_file());
    // materiais canonicos gerados
    let mats = env.lib.asset_materials(&id).unwrap();
    assert_eq!(mats.len(), 1);
    let m = env.lib.load_material(&id, &mats[0].0).unwrap();
    assert_eq!(m.converted_from.as_deref(), Some("gltf-pbr"));
    // busca por texto, por tag e por nome parcial
    for text in ["hero", "model", "her", "tag:cel", "author:ana"] {
        let page = env.lib.query(&Query { text: Some(text.into()), ..Default::default() }).unwrap();
        assert_eq!(page.items.len(), 1, "texto {text:?}");
    }
    assert_eq!(env.lib.query(&Query { text: Some("naoexiste".into()), ..Default::default() }).unwrap().items.len(), 0);
}

#[test]
fn events_report_progress_and_finish() {
    let env = Env::new();
    let src = env.src("batch");
    for i in 0..5 {
        write(&src, &format!("m{i}.glb"), &glb(i));
    }
    let job = env.lib.import(&src, ImportOptions::default());
    let report = job.join().unwrap();
    assert_eq!(report.imported.len(), 5);
    assert!(!report.cancelled);

    // eventos de outro job
    let src2 = env.src("batch2");
    for i in 10..13 {
        write(&src2, &format!("m{i}.glb"), &glb(i));
    }
    let job = env.lib.import(&src2, ImportOptions::default());
    let mut seen_discovered = false;
    let mut imported = 0;
    let mut finished = false;
    let mut last_done = 0;
    loop {
        match job.events().recv() {
            Ok(ImportEvent::Discovered { files }) => {
                assert_eq!(files, 3);
                seen_discovered = true;
            }
            Ok(ImportEvent::Imported { .. }) => imported += 1,
            Ok(ImportEvent::Progress { done, total, .. }) => {
                assert_eq!(total, 3);
                assert!(done >= last_done);
                last_done = done;
            }
            Ok(ImportEvent::Finished) => {
                finished = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(seen_discovered && finished);
    assert_eq!(imported, 3);
    job.join().unwrap();
}

#[test]
fn duplicate_content_is_skipped_by_default_and_allowed_on_request() {
    let env = Env::new();
    let src = env.src("a");
    let a = write(&src, "a.glb", &glb(7));
    let b = write(&src, "copy_of_a.glb", &glb(7));
    let id = import_one(&env.lib, &a, ImportOptions::default());
    let r = env.lib.import_blocking(&b, ImportOptions::default()).unwrap();
    assert!(r.imported.is_empty());
    assert_eq!(r.duplicates, vec![(b.canonicalize().unwrap(), id)]);
    // nada foi copiado para a biblioteca
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 1);
    // permitido: cria outro ativo e o duplicado aparece em find_duplicates
    let r = env.lib.import_blocking(&b, ImportOptions { duplicates: DuplicatePolicy::Allow, ..Default::default() }).unwrap();
    assert_eq!(r.imported.len(), 1);
    let groups = env.lib.find_duplicates(10).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].assets.len(), 2);
    assert!(groups[0].reclaimable_bytes() > 0);
    // duplicata dentro do MESMO lote
    let src2 = env.src("same_batch");
    write(&src2, "x1.glb", &glb(99));
    write(&src2, "x2.glb", &glb(99));
    let r = env.lib.import_blocking(&src2, ImportOptions::default()).unwrap();
    assert_eq!(r.imported.len(), 1);
    assert_eq!(r.duplicates.len(), 1);
}

#[test]
fn one_bad_file_does_not_abort_the_batch() {
    let env = Env::new();
    let src = env.src("mixed");
    write(&src, "good1.glb", &glb(1));
    let g = glb(2);
    write(&src, "truncated.glb", &g[..g.len() / 2]);
    write(&src, "garbage.glb", &[0xAB; 777]);
    write(&src, "empty.glb", b"");
    write(&src, "bad.png", b"not a png at all");
    write(&src, "bad.fbx", b"nope");
    write(&src, "bad.obj", b"\x00\x01\x02\x03");
    write(&src, "good2.glb", &glb(3));
    write(&src, "notes.txt", b"ignorado");
    write(&src, "readme.md", b"ignorado");
    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert_eq!(r.imported.len(), 2, "{r:?}");
    assert_eq!(r.failed.len(), 6, "{:?}", r.failed);
    assert_eq!(r.skipped, 2, "txt e md");
    let names: Vec<String> = r.failed.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
    for n in ["truncated.glb", "garbage.glb", "empty.glb", "bad.png", "bad.fbx", "bad.obj"] {
        assert!(names.contains(&n.to_string()), "{n} deveria falhar: {names:?}");
    }
    // staging nao deixa lixo
    let staging = env.lib.home().join(".staging");
    if staging.exists() {
        assert_eq!(std::fs::read_dir(staging).unwrap().count(), 0);
    }
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 2);
}

#[test]
fn single_unsupported_file_is_an_error() {
    let env = Env::new();
    let src = env.src("x");
    let p = write(&src, "a.blend", b"BLENDER");
    let r = env.lib.import_blocking(&p, ImportOptions::default());
    assert!(matches!(r, Err(AssetError::Unsupported(_))), "{r:?}");
    let r = env.lib.import_blocking(src.join("naoexiste.glb"), ImportOptions::default());
    assert!(r.is_err());
}

#[test]
fn folder_import_claims_dependents_and_orders_models_first() {
    let env = Env::new();
    let src = env.src("scene");
    // OBJ + MTL + textura; textura nao deve virar ativo proprio
    write(&src, "cube.obj", CUBE_OBJ.as_bytes());
    write(&src, "cube.mtl", b"newmtl Red\nKd 1 0 0\nmap_Kd tex/red.png\n");
    write(&src, "tex/red.png", &png(8, 8, [255, 0, 0]));
    // textura realmente solta
    write(&src, "loose.png", &png(16, 16, [0, 0, 255]));
    // gltf com buffer externo e imagem
    let (mut j, bin) = triangle_json(Some(json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}})));
    j["buffers"][0]["uri"] = json!("tri.bin");
    j["images"] = json!([{"uri": "gtex/a.png"}]);
    j["textures"] = json!([{"source": 0}]);
    write(&src, "tri.gltf", &serde_json::to_vec(&j).unwrap());
    write(&src, "tri.bin", &bin[..42]);
    write(&src, "gtex/a.png", &png(8, 8, [1, 2, 3]));

    let r = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.imported.len(), 3, "cube.obj, tri.gltf e loose.png: {r:?}");
    assert_eq!(r.skipped, 4, "cube.mtl e tri.bin (extensao nao suportada) + red.png e a.png (dependentes)");

    let kinds = env.lib.query(&Query { kinds: vec![AssetKind::Texture], ..Default::default() }).unwrap();
    assert_eq!(kinds.items.len(), 1);
    assert_eq!(kinds.items[0].name, "loose");

    // OBJ: arquivos copiados mantendo a estrutura relativa
    let obj = env.lib.query(&Query { text: Some("cube".into()), ..Default::default() }).unwrap().items[0].clone();
    let files = env.lib.resolve_files(&obj.id).unwrap();
    assert_eq!(files.files.len(), 3);
    assert!(files.files.iter().all(|f| f.is_file()));
    assert!(files.files.iter().any(|f| f.ends_with("tex/red.png")));
    let m = env.lib.manifest(&obj.id).unwrap();
    assert_eq!(m.stats.tris, 12);
    assert_eq!(m.revisions[0].files.len(), 3);
    // hash combinado de varios arquivos != hash do primario
    assert_ne!(obj.content_hash.0, *blake3::hash(CUBE_OBJ.as_bytes()).as_bytes());

    // glTF: buffer e textura copiados
    let g = env.lib.query(&Query { text: Some("tri".into()), ..Default::default() }).unwrap().items[0].clone();
    assert_eq!(env.lib.resolve_files(&g.id).unwrap().files.len(), 3);
    let mats = env.lib.asset_materials(&g.id).unwrap();
    let mat = env.lib.load_material(&g.id, &mats[0].0).unwrap();
    assert_eq!(mat.base.texture.unwrap().uri.as_deref(), Some("gtex/a.png"));
}

#[test]
fn cancel_stops_cleanly_and_a_rerun_resumes() {
    let env = Env::new();
    let src = env.src("many");
    for i in 0..300 {
        write(&src, &format!("t{i:04}.png"), &unique_png(i));
    }
    let job = env.lib.import(&src, ImportOptions { threads: 2, ..Default::default() });
    // cancela assim que o primeiro item for importado
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
    let r1 = job.join().unwrap();
    assert!(r1.cancelled || r1.imported.len() == 300, "cancelou ou terminou muito rapido");
    let after_cancel = env.lib.count(&Query::default()).unwrap();
    assert_eq!(after_cancel as usize, r1.imported.len());
    // nenhum staging orfao
    let staging = env.lib.home().join(".staging");
    if staging.exists() {
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0, "staging deve ser limpo ao cancelar");
    }
    // retomar = rodar de novo: o que ja entrou vira duplicado, o resto e importado
    let r2 = env.lib.import_blocking(&src, ImportOptions::default()).unwrap();
    assert_eq!(r2.imported.len() + r2.duplicates.len(), 300);
    assert_eq!(r2.duplicates.len(), r1.imported.len());
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 300);
}

#[test]
fn import_with_replace_creates_a_new_revision() {
    let env = Env::new();
    let src = env.src("rev");
    let p = write(&src, "model.glb", &glb(1));
    let id = import_one(&env.lib, &p, ImportOptions::default());
    let old_hash = env.lib.require_row(&id).unwrap().content_hash;
    // mesmo conteudo: nada muda
    let r = env.lib.import_blocking(&p, ImportOptions { replace: Some(id), ..Default::default() }).unwrap();
    assert!(r.revised.is_empty());
    assert_eq!(r.duplicates.len(), 1);
    // conteudo novo
    let p2 = write(&src, "model_v2.glb", &glb(2));
    let r = env.lib.import_blocking(&p2, ImportOptions { replace: Some(id), ..Default::default() }).unwrap();
    assert_eq!(r.revised, vec![id]);
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.revision, 2);
    assert_ne!(row.content_hash, old_hash);
    assert_eq!(row.id, id, "a identidade nao muda");
    let m = env.lib.manifest(&id).unwrap();
    assert_eq!(m.revisions.len(), 2);
    assert_eq!(m.current_revision, 2);
    // a revisao antiga continua em disco (imutavel)
    let dir = env.lib.asset_dir(&id).unwrap();
    assert!(dir.join("r1/model.glb").is_file());
    assert!(dir.join("r2/model_v2.glb").is_file());
    assert_eq!(env.lib.resolve_files(&id).unwrap().primary, dir.join("r2/model_v2.glb"));
    // replace exige um unico arquivo
    let r = env.lib.import_blocking(&src, ImportOptions { replace: Some(id), ..Default::default() });
    assert!(r.is_err());
}

#[test]
fn source_inside_the_library_is_refused() {
    let env = Env::new();
    let src = env.src("a");
    let p = write(&src, "a.glb", &glb(1));
    let id = import_one(&env.lib, &p, ImportOptions::default());
    let inside = env.lib.resolve_files(&id).unwrap().primary;
    let r = env.lib.import_blocking(&inside, ImportOptions { duplicates: DuplicatePolicy::Allow, ..Default::default() });
    assert!(matches!(r, Err(AssetError::Conflict(_))), "{r:?}");
}

#[test]
fn manual_material_file_becomes_a_material_asset() {
    let env = Env::new();
    let src = env.src("m");
    let mut m = ToonMaterial::named("Pele Anime");
    m.base.color = [0.9, 0.7, 0.6, 1.0];
    let p = write(&src, "pele.anigomat.json", &m.to_json().unwrap());
    let id = import_one(&env.lib, &p, ImportOptions::default());
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.kind, AssetKind::Material);
    assert_eq!(row.name, "Pele Anime");
    assert_eq!(row.format, "anigomat");
    // material invalido nao entra
    let bad = write(&src, "ruim.anigomat.json", b"{\"schema\":\"x\"}");
    let r = env.lib.import_blocking(&bad, ImportOptions::default()).unwrap();
    assert_eq!(r.failed.len(), 1);
}

#[test]
fn preview_reports_without_importing() {
    let env = Env::new();
    let src = env.src("p");
    let p = write(&src, "a.glb", &glb(1));
    let pv = anigo_assets::preview(&p, env.lib.limits()).unwrap();
    assert_eq!(pv.format, "glb");
    assert_eq!(pv.stats.tris, 1);
    assert_eq!(pv.material_names, vec!["Mat"]);
    assert_eq!(env.lib.count(&Query::default()).unwrap(), 0);
}

#[test]
fn draco_required_is_refused_with_a_clear_error() {
    let env = Env::new();
    let src = env.src("d");
    let (mut j, bin) = triangle_json(None);
    j["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    j["extensionsUsed"] = json!(["KHR_draco_mesh_compression"]);
    let p = write(&src, "draco.glb", &anigo_vrm::gltf::build_glb(&j, &bin));
    let r = env.lib.import_blocking(&p, ImportOptions::default()).unwrap();
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].1.contains("Draco"), "{}", r.failed[0].1);
}

#[test]
fn explicit_kind_name_and_collection_options() {
    let env = Env::new();
    let coll = env.lib.create_collection("Favoritos do projeto").unwrap();
    let src = env.src("o");
    let p = write(&src, "a.glb", &glb(5));
    let id = import_one(&env.lib, &p, ImportOptions { kind: Some(AssetKind::Prefab), name: Some("  Meu Prefab ".into()), collection: Some(coll), description: Some("desc".into()), ..Default::default() });
    let row = env.lib.require_row(&id).unwrap();
    assert_eq!(row.kind, AssetKind::Prefab);
    assert_eq!(row.name, "Meu Prefab");
    assert_eq!(env.lib.collection(coll).unwrap().item_count, 1);
    let page = env.lib.query(&Query { collection: Some(coll), ..Default::default() }).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(env.lib.manifest(&id).unwrap().description, "desc");
}

#[cfg(feature = "fbx")]
#[test]
fn fbx_with_garbage_body_fails_without_panic() {
    let env = Env::new();
    let src = env.src("f");
    let mut data = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
    data.extend_from_slice(&[0xFF; 500]);
    let p = write(&src, "bad.fbx", &data);
    let r = env.lib.import_blocking(&p, ImportOptions::default()).unwrap();
    assert_eq!(r.failed.len(), 1);
}
