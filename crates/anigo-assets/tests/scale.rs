//! Teste de escala. O caso padrao (5 000 ativos) roda no CI e valida CORRECAO em volume; o caso de 50 000 e
//! `#[ignore]` e valida tambem tempo: `cargo test -p anigo-assets --release --test scale -- --ignored --nocapture`.
//! Numeros reais e metodologia: `docs/native/BIBLIOTECA_BENCH.md` (`examples/bench_scale.rs`).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anigo_assets::*;

const WORDS: &[&str] = &["cabelo", "camisa", "vestido", "espada", "castelo", "floresta", "sapato", "luva", "chapeu", "capa"];
const ADJ: &[&str] = &["longo", "curto", "rosa", "azul", "dourado", "antigo", "sombrio", "leve"];

fn manifest(i: usize) -> AssetManifest {
    let hash = ContentHash(*blake3::hash(&(i as u64).to_le_bytes()).as_bytes());
    let kinds = [AssetKind::Model, AssetKind::Hair, AssetKind::Texture, AssetKind::Material, AssetKind::Outfit];
    AssetManifest {
        schema: MANIFEST_SCHEMA,
        asset_id: AssetId::new(),
        name: format!("{} {} {}", WORDS[i % WORDS.len()], ADJ[(i / 10) % ADJ.len()], i),
        kind: kinds[i % kinds.len()],
        description: String::new(),
        author: "teste".into(),
        license: String::new(),
        source_url: String::new(),
        format: "glb".into(),
        storage: StorageMode::Managed,
        primary: "a.glb".into(),
        link: None,
        current_revision: 1,
        revisions: vec![RevisionInfo { rev: 1, content_hash: hash, size: 1000 + i as u64, created_at: 0, files: vec![FileEntry { path: "a.glb".into(), size: 1000 + i as u64, blake3: hash }], note: String::new() }],
        tags: vec![format!("pack/p{}", i % 7), if i % 3 == 0 { "estilo/cel".into() } else { "estilo/aquarela".into() }],
        rating: (i % 6) as u8,
        favorite: i % 20 == 0,
        stats: AssetStats { tris: (i * 37 % 100_000) as u64, ..Default::default() },
        dependencies: vec![],
        materials: vec![],
        importer_ver: IMPORTER_VERSION,
        created_at: 0,
        imported_at: (i * 13 % 1_000_003) as i64,
        modified_at: 0,
        trashed_at: None,
        props: serde_json::Value::Null,
        warnings: vec![],
    }
}

fn build(n: usize) -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    for i in 0..n {
        let m = manifest(i);
        let d = lib.home().join(layout::managed_dir_rel(&m.asset_id));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(MANIFEST_FILE), m.to_json_pretty().unwrap()).unwrap();
    }
    let rep = lib.rebuild_index().unwrap();
    assert_eq!(rep.assets as usize, n);
    assert!(rep.bad_manifests.is_empty());
    (dir, lib)
}

fn walk(lib: &Library, mut q: Query) -> Vec<AssetRow> {
    let mut out = Vec::new();
    loop {
        let p = lib.query(&q).unwrap();
        out.extend(p.items);
        match p.next {
            Some(c) => q.after = Some(c),
            None => return out,
        }
    }
}

fn exercise(n: usize, budget: Option<Duration>) {
    let (_dir, lib) = build(n);
    assert_eq!(lib.count(&Query::default()).unwrap() as usize, n);

    // varredura completa por nome: sem repeticao nem omissao
    let rows = walk(&lib, Query { limit: 200, ..Default::default() });
    assert_eq!(rows.len(), n);
    assert_eq!(rows.iter().map(|r| r.id).collect::<HashSet<_>>().len(), n);

    // FTS: 'cabelo' aparece em n/10 ativos
    let expect = (0..n).filter(|i| i % WORDS.len() == 0).count();
    let hits = walk(&lib, Query { text: Some("cabelo".into()), limit: 500, ..Default::default() });
    assert_eq!(hits.len(), expect);
    assert!(hits.iter().all(|r| r.name.starts_with("cabelo")));
    // FTS + tipo + tag: a contagem confere com a conta feita a mao
    let c = lib.count(&Query { text: Some("camisa".into()), kinds: vec![AssetKind::Hair], tags: vec!["estilo/cel".into()], ..Default::default() }).unwrap() as usize;
    let want = (0..n).filter(|i| i % WORDS.len() == 1 && i % 5 == 1 && i % 3 == 0).count();
    assert_eq!(c, want);
    // relevancia pagina tudo
    let rel = walk(&lib, Query { text: Some("rosa".into()), sort: Sort { by: SortBy::Relevance, desc: false }, limit: 333, ..Default::default() });
    assert_eq!(rel.len(), (0..n).filter(|i| (i / 10) % ADJ.len() == 2).count());
    // ordenacoes por colunas com muitos empates
    for by in [SortBy::Rating, SortBy::Tris, SortBy::ImportedAt, SortBy::Size] {
        for desc in [false, true] {
            let r = walk(&lib, Query { sort: Sort { by, desc }, limit: 777, ..Default::default() });
            assert_eq!(r.len(), n, "{by:?} {desc}");
            assert_eq!(r.iter().map(|x| x.id).collect::<HashSet<_>>().len(), n);
        }
    }

    // reconstruir de novo e reabrir devolve o mesmo
    let before: HashSet<AssetId> = rows.iter().map(|r| r.id).collect();
    lib.rebuild_index().unwrap();
    let after: HashSet<AssetId> = walk(&lib, Query { limit: 1000, ..Default::default() }).into_iter().map(|r| r.id).collect();
    assert_eq!(before, after);

    if let Some(budget) = budget {
        // medianas de 15 repeticoes
        let med = |q: Query| {
            let mut t: Vec<Duration> = (0..15)
                .map(|_| {
                    let s = Instant::now();
                    lib.query(&q).unwrap();
                    s.elapsed()
                })
                .collect();
            t.sort();
            t[7]
        };
        let cases: Vec<(&str, Query)> = vec![
            ("pagina 1", Query::default()),
            ("fts cabelo", Query { text: Some("cabelo".into()), ..Default::default() }),
            ("fts cabelo longo", Query { text: Some("cabelo longo".into()), ..Default::default() }),
            ("fts relevancia", Query { text: Some("cabelo".into()), sort: Sort { by: SortBy::Relevance, desc: false }, ..Default::default() }),
            ("tag + tipo", Query { tags: vec!["estilo/cel".into()], kinds: vec![AssetKind::Hair], ..Default::default() }),
            ("favoritos", Query { favorite: Some(true), ..Default::default() }),
        ];
        for (name, q) in cases {
            let d = med(q);
            println!("{name}: {:.2} ms", d.as_secs_f64() * 1000.0);
            assert!(d <= budget, "{name}: {d:?} > {budget:?}");
        }
    }
}

#[test]
fn five_thousand_assets_are_correct() {
    exercise(5_000, None);
}

#[test]
#[ignore = "pesado: rode com --release -- --ignored"]
fn fifty_thousand_assets_meet_the_50ms_search_budget() {
    exercise(50_000, Some(Duration::from_millis(50)));
}
