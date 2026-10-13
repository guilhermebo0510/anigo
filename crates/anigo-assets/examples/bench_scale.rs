//! Benchmark de escala da Biblioteca (T-5.10). Gera N ativos sinteticos (manifestos reais em disco), mede a
//! construcao do indice, buscas FTS, listagem paginada por keyset e operacoes de gestao.
//!
//! ```text
//! cargo run -p anigo-assets --release --example bench_scale -- 50000 [pasta_de_trabalho]
//! ```
//! Imprime uma tabela Markdown. Sem GPU, sem rede.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anigo_assets::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const ADJ: &[&str] = &[
    "longo", "curto", "rosa", "azul", "vermelho", "dourado", "prateado", "antigo", "moderno", "sombrio", "brilhante", "suave", "pesado", "leve", "grande", "pequeno", "elegante", "rustico", "futurista", "classico",
];
const NOUN: &[&str] = &[
    "cabelo", "camisa", "vestido", "espada", "castelo", "floresta", "sapato", "luva", "chapeu", "capa", "armadura", "janela", "porta", "mesa", "cadeira", "lampada", "arvore", "pedra", "ponte", "torre", "casa", "carro", "nave", "robo",
    "gato", "cachorro", "dragao", "flor", "nuvem", "montanha",
];
const STYLE: &[&str] = &["anime", "cel", "noir", "fantasia", "cyber", "medieval", "escolar", "casual", "festa", "esporte"];
const TAGS: &[&str] = &[
    "estilo/cel/noir", "estilo/cel/pastel", "estilo/cel/neon", "estilo/aquarela/suave", "estilo/aquarela/forte", "estilo/realista", "categoria/roupa", "categoria/cabelo", "categoria/cenario", "categoria/prop", "categoria/personagem",
    "origem/loja", "origem/propria", "origem/comunidade", "qualidade/alta", "qualidade/media", "qualidade/baixa",
];

fn kind_for(r: &mut Rng) -> AssetKind {
    match r.below(100) {
        0..=29 => AssetKind::Model,
        30..=39 => AssetKind::Character,
        40..=49 => AssetKind::Outfit,
        50..=57 => AssetKind::Hair,
        58..=64 => AssetKind::Accessory,
        65..=76 => AssetKind::Material,
        77..=91 => AssetKind::Texture,
        92..=94 => AssetKind::Pose,
        95..=97 => AssetKind::Animation,
        _ => AssetKind::Hdri,
    }
}

fn synth_manifest(i: usize, r: &mut Rng, dup_of: Option<ContentHash>) -> AssetManifest {
    let kind = kind_for(r);
    let name = format!("{} {} {} {}", NOUN[r.below(NOUN.len())], ADJ[r.below(ADJ.len())], STYLE[r.below(STYLE.len())], i);
    let hash = dup_of.unwrap_or_else(|| ContentHash(*blake3::hash(&(i as u64).to_le_bytes()).as_bytes()));
    let size = 20_000 + (r.next() % 50_000_000);
    let mut tags: Vec<String> = Vec::new();
    for _ in 0..(2 + r.below(3)) {
        tags.push(TAGS[r.below(TAGS.len())].to_string());
    }
    tags.push(format!("pack/pack{:02}", r.below(50)));
    tags.sort();
    tags.dedup();
    let now = now_secs();
    AssetManifest {
        schema: MANIFEST_SCHEMA,
        asset_id: AssetId::new(),
        name,
        kind,
        description: if r.below(4) == 0 { format!("{} {} para cenas {}", NOUN[r.below(NOUN.len())], ADJ[r.below(ADJ.len())], STYLE[r.below(STYLE.len())]) } else { String::new() },
        author: ["Ana", "Bia", "Caio", "Davi", "Eva", "Fabio"][r.below(6)].to_string(),
        license: "CC0".into(),
        source_url: String::new(),
        format: if kind == AssetKind::Texture { "png".into() } else { "glb".into() },
        storage: StorageMode::Managed,
        primary: "asset.glb".into(),
        link: None,
        current_revision: 1,
        revisions: vec![RevisionInfo { rev: 1, content_hash: hash, size, created_at: now, files: vec![FileEntry { path: "asset.glb".into(), size, blake3: hash }], note: String::new() }],
        tags,
        rating: r.below(6) as u8,
        favorite: r.below(20) == 0,
        stats: AssetStats { tris: r.next() % 500_000, verts: r.next() % 300_000, materials: (r.next() % 8) as u32, ..Default::default() },
        dependencies: vec![],
        materials: vec![],
        importer_ver: IMPORTER_VERSION,
        created_at: now - (r.next() % 10_000_000) as i64,
        imported_at: now - (r.next() % 10_000_000) as i64,
        modified_at: now,
        trashed_at: None,
        props: serde_json::Value::Null,
        warnings: vec![],
    }
}

fn rss_kb() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let get = |k: &str| s.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0);
    (get("VmRSS:"), get("VmHWM:"))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Stat {
    name: String,
    runs: usize,
    median: f64,
    p95: f64,
    max: f64,
    extra: String,
}

fn measure<F: FnMut() -> String>(name: &str, runs: usize, mut f: F) -> Stat {
    // 1 aquecimento descartado
    let _ = f();
    let mut times = Vec::with_capacity(runs);
    let mut extra = String::new();
    for _ in 0..runs {
        let t = Instant::now();
        extra = f();
        times.push(ms(t.elapsed()));
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Stat { name: name.to_string(), runs, median: times[times.len() / 2], p95: times[((times.len() as f64) * 0.95) as usize % times.len()], max: *times.last().unwrap(), extra }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(50_000);
    let work: PathBuf = args.get(2).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("anigo-bench-{n}")));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let home = work.join("lib");
    println!("# Benchmark anigo-assets: {n} ativos sinteticos\n");
    println!("- pasta de trabalho: `{}` (so o sistema de arquivos local; ver docs)", work.display());
    println!("- cpus: {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    println!("- build: {}", if cfg!(debug_assertions) { "DEBUG (numeros nao representativos)" } else { "release" });

    let t = Instant::now();
    let lib = Library::open(&home).unwrap();
    let t_open_empty = ms(t.elapsed());

    // 1. manifestos em disco
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let t = Instant::now();
    let mut hashes: Vec<ContentHash> = Vec::new();
    for i in 0..n {
        let dup = if i > 100 && rng.below(100) == 0 { Some(hashes[rng.below(hashes.len())]) } else { None };
        let m = synth_manifest(i, &mut rng, dup);
        hashes.push(m.content_hash().unwrap());
        let dir = lib.home().join(layout::managed_dir_rel(&m.asset_id));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), m.to_json_pretty().unwrap()).unwrap();
    }
    let t_write = t.elapsed();
    let (rss0, _) = rss_kb();

    // 2. indice: construcao em massa (e o mesmo caminho da recuperacao)
    let t = Instant::now();
    let rep = lib.rebuild_index().unwrap();
    let t_rebuild = t.elapsed();
    assert_eq!(rep.assets as usize, n);
    let db_size = std::fs::metadata(lib.index_path()).map(|m| m.len()).unwrap_or(0)
        + std::fs::metadata(format!("{}-wal", lib.index_path().display())).map(|m| m.len()).unwrap_or(0);
    let (rss1, hwm1) = rss_kb();

    // 3. reabrir com indice pronto
    drop(lib);
    let t = Instant::now();
    let lib = Library::open(&home).unwrap();
    let t_reopen = ms(t.elapsed());

    let mut stats: Vec<Stat> = Vec::new();
    let q0 = || Query::default();
    stats.push(measure("COUNT(*) (sem filtro)", 50, || format!("{}", lib.count(&q0()).unwrap())));
    stats.push(measure("pagina 1 (nome, 200)", 50, || format!("{}", lib.query(&q0()).unwrap().items.len())));
    stats.push(measure("pagina 1 (nome, 200) + total", 30, || format!("{:?}", lib.query(&Query { with_total: true, ..q0() }).unwrap().total)));
    stats.push(measure("pagina 1, tipo=hair, nota desc", 50, || format!("{}", lib.query(&Query { kinds: vec![AssetKind::Hair], sort: Sort { by: SortBy::Rating, desc: true }, ..q0() }).unwrap().items.len())));
    stats.push(measure("favoritos (pag. 1)", 50, || format!("{}", lib.query(&Query { favorite: Some(true), ..q0() }).unwrap().items.len())));
    stats.push(measure("ordenar por tamanho desc (pag. 1)", 50, || format!("{}", lib.query(&Query { sort: Sort { by: SortBy::Size, desc: true }, ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS: 'cabelo' (prefixo, pag. 1, por nome)", 50, || format!("{}", lib.query(&Query { text: Some("cabelo".into()), ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS: 'cabelo longo' (2 termos)", 50, || format!("{}", lib.query(&Query { text: Some("cabelo longo".into()), ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS: 'cabelo longo' + total", 30, || format!("{:?}", lib.query(&Query { text: Some("cabelo longo".into()), with_total: true, ..q0() }).unwrap().total)));
    stats.push(measure("FTS: 'cab' (prefixo curto)", 50, || format!("{}", lib.query(&Query { text: Some("cab".into()), ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS: termo raro (numero exato)", 50, || format!("{}", lib.query(&Query { text: Some(format!("{}", n / 2)), ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS: 'tag:noir -vestido'", 50, || format!("{}", lib.query(&Query { text: Some("tag:noir -vestido".into()), ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS relevancia: 'cabelo' (pior caso, ~3% dos ativos)", 20, || format!("{}", lib.query(&Query { text: Some("cabelo".into()), sort: Sort { by: SortBy::Relevance, desc: false }, ..q0() }).unwrap().items.len())));
    stats.push(measure("FTS relevancia: 'anime' (~10% dos ativos)", 20, || format!("{}", lib.query(&Query { text: Some("anime".into()), sort: Sort { by: SortBy::Relevance, desc: false }, ..q0() }).unwrap().items.len())));
    stats.push(measure("tag hierarquica 'estilo/cel' (pag. 1)", 50, || format!("{}", lib.query(&Query { tags: vec!["estilo/cel".into()], ..q0() }).unwrap().items.len())));
    stats.push(measure("tag 'estilo' + tag 'categoria/roupa' (E)", 50, || format!("{}", lib.query(&Query { tags: vec!["estilo".into(), "categoria/roupa".into()], ..q0() }).unwrap().items.len())));
    stats.push(measure("combinada: texto + tipo + tag + nota>=3", 50, || {
        format!("{}", lib.query(&Query { text: Some("cabelo".into()), kinds: vec![AssetKind::Hair, AssetKind::Model], tags: vec!["estilo".into()], min_rating: Some(3), ..q0() }).unwrap().items.len())
    }));
    stats.push(measure("tags() com contagens", 20, || format!("{}", lib.tags().unwrap().len())));

    // paginacao keyset: percorre TUDO
    let mut page_times: Vec<f64> = Vec::new();
    let t = Instant::now();
    let mut q = Query { limit: 200, ..Query::default() };
    let mut seen = 0usize;
    loop {
        let tp = Instant::now();
        let p = lib.query(&q).unwrap();
        page_times.push(ms(tp.elapsed()));
        seen += p.items.len();
        match p.next {
            Some(c) => q.after = Some(c),
            None => break,
        }
    }
    let t_walk = ms(t.elapsed());
    assert_eq!(seen, n);
    page_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let first_pages: f64 = page_times.iter().take(1).sum();
    // pagina profunda (apos ~metade) com cursor
    let mut q = Query { limit: 200, ..Query::default() };
    for _ in 0..(n / 200 / 2) {
        q.after = lib.query(&q).unwrap().next;
    }
    let deep = measure("pagina PROFUNDA (keyset, ~50% da lista)", 50, || format!("{}", lib.query(&q).unwrap().items.len()));
    stats.push(deep);

    // colecoes
    let ids = lib.query_ids(&Query { kinds: vec![AssetKind::Texture], ..Query::default() }, 5000).unwrap();
    let coll = lib.create_collection("bench manual").unwrap();
    let t = Instant::now();
    lib.add_to_collection(coll, &ids).unwrap();
    let t_coll_add = ms(t.elapsed());
    stats.push(measure("colecao manual (5000 itens) pag. 1", 50, || format!("{}", lib.query(&Query { collection: Some(coll), ..Query::default() }).unwrap().items.len())));
    let smart = lib.create_smart_collection("bench smart", &Query { favorite: Some(true), tags: vec!["categoria".into()], ..Query::default() }).unwrap();
    stats.push(measure("colecao inteligente pag. 1", 50, || format!("{}", lib.query(&Query { collection: Some(smart), ..Query::default() }).unwrap().items.len())));

    // gestao
    stats.push(measure("find_duplicates (limite 1000)", 5, || format!("{}", lib.find_duplicates(1000).unwrap().len())));
    let some = lib.query(&Query::default()).unwrap().items;
    let mut k = 0usize;
    stats.push(measure("set_favorite (reescreve asset.json + SQL)", 100, || {
        k += 1;
        lib.set_favorite(&some[k % some.len()].id, k % 2 == 0).unwrap();
        String::new()
    }));
    stats.push(measure("add_tags (reescreve asset.json + reindexa)", 100, || {
        k += 1;
        lib.add_tags(&some[k % some.len()].id, &[format!("bench/t{}", k % 7)]).unwrap();
        String::new()
    }));
    let mut tr = 0usize;
    stats.push(measure("trash + restore de 1 ativo", 50, || {
        tr += 1;
        let id = some[(tr * 7) % some.len()].id;
        lib.trash(&[id], TrashOptions::default()).unwrap();
        lib.restore(&[id]).unwrap();
        String::new()
    }));

    // concorrencia de leitura
    let readers = 4usize;
    let t = Instant::now();
    let handles: Vec<_> = (0..readers)
        .map(|r| {
            let lib = lib.clone();
            std::thread::spawn(move || {
                let words = ["cabelo", "camisa", "anime", "rosa", "castelo", "azul"];
                let mut n = 0;
                for i in 0..200 {
                    n += lib.query(&Query { text: Some(words[(i + r) % words.len()].into()), limit: 50, ..Query::default() }).unwrap().items.len();
                }
                n
            })
        })
        .collect();
    let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    let t_conc = t.elapsed();

    // import real (disco): 2000 PNGs pequenos
    let png_dir = work.join("pngs");
    std::fs::create_dir_all(&png_dir).unwrap();
    let n_png = 2000usize;
    for i in 0..n_png {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([(i % 251) as u8, ((i / 251) % 251) as u8, (i * 7 % 251) as u8, 255]));
        img.save(png_dir.join(format!("p{i:05}.png"))).unwrap();
    }
    let t = Instant::now();
    let r = lib.import_blocking(&png_dir, ImportOptions::default()).unwrap();
    let t_import = t.elapsed();
    assert_eq!(r.imported.len(), n_png, "{:?}", r.failed.first());
    // scan linked: 2000 arquivos novos e depois sem mudancas
    let ext = work.join("linked_ext");
    std::fs::create_dir_all(&ext).unwrap();
    for i in 0..n_png {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([(i % 250) as u8, 3, ((i / 250) % 251) as u8, 255]));
        img.save(ext.join(format!("l{i:05}.png"))).unwrap();
    }
    let root = lib.add_root(&ext, RootMode::Linked, None).unwrap();
    let t = Instant::now();
    let r1 = lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let t_scan1 = t.elapsed();
    let t = Instant::now();
    let r2 = lib.scan_blocking(root, ScanOptions::default()).unwrap();
    let t_scan2 = t.elapsed();

    // verify (sem rehash e com rehash) numa amostra: o indice inteiro, so existencia
    let t = Instant::now();
    let v = lib.verify_blocking(VerifyOptions { rehash: false, fix: false }).unwrap();
    let t_verify = t.elapsed();

    let (rss2, hwm2) = rss_kb();

    println!("\n## Construcao\n");
    println!("| Etapa | Tempo | Taxa |\n|---|---|---|");
    println!("| abrir biblioteca vazia | {t_open_empty:.1} ms | |");
    println!("| gravar {n} `asset.json` (disco, fora da API) | {:.2} s | {:.0} arq/s |", t_write.as_secs_f64(), n as f64 / t_write.as_secs_f64());
    println!("| `rebuild_index` ({n} ativos: SQLite + FTS5 + tags, 1 transacao) | {:.2} s | {:.0} ativos/s |", t_rebuild.as_secs_f64(), n as f64 / t_rebuild.as_secs_f64());
    println!("| reabrir com indice pronto | {t_reopen:.1} ms | |");
    println!("| tamanho do indice (index.db + wal) | {:.1} MiB | {:.0} B/ativo |", db_size as f64 / 1048576.0, db_size as f64 / n as f64);
    println!("| percorrer TODAS as paginas (keyset, 200/pag., {} pags.) | {t_walk:.0} ms | mediana {:.2} ms, max {:.2} ms por pagina |", page_times.len(), page_times[page_times.len() / 2], page_times.last().unwrap());
    println!("| primeira pagina da varredura | {first_pages:.2} ms | |");
    println!("| adicionar 5000 itens a colecao manual | {t_coll_add:.0} ms | |");
    println!("| 4 leitores concorrentes x 200 buscas FTS (limite 50) | {:.0} ms | {:.0} buscas/s ({} itens) |", ms(t_conc), (readers * 200) as f64 / t_conc.as_secs_f64(), total);
    println!("| import real de {n_png} PNGs (copia + BLAKE3 + analise + indexacao) | {:.2} s | {:.0} arq/s |", t_import.as_secs_f64(), n_png as f64 / t_import.as_secs_f64());
    println!("| scan linked de {n_png} PNGs novos | {:.2} s | {:.0} arq/s (importados {}) |", t_scan1.as_secs_f64(), n_png as f64 / t_scan1.as_secs_f64(), r1.import.imported.len());
    println!("| scan linked sem mudancas (pula por mtime+tamanho) | {:.0} ms | {} pulados, {} reimportados |", ms(t_scan2), r2.unchanged, r2.import.imported.len());
    println!("| `verify` (existencia+tamanho, sem hash) de {} ativos (sinteticos: sem arquivo de payload, entao os problemas sao esperados) | {:.2} s | {} problemas |", v.checked, t_verify.as_secs_f64(), v.issues.len());
    println!("\n## Consultas (mediana / p95 / max em ms)\n");
    println!("| Consulta | execucoes | mediana | p95 | max | resultado |\n|---|---|---|---|---|---|");
    for s in &stats {
        println!("| {} | {} | {:.2} | {:.2} | {:.2} | {} |", s.name, s.runs, s.median, s.p95, s.max, s.extra);
    }
    println!("\n## Memoria do processo (VmRSS / pico VmHWM)\n");
    println!("- apos gravar manifestos: {:.0} MiB RSS", rss0 as f64 / 1024.0);
    println!("- apos `rebuild_index`: {:.0} MiB RSS, pico {:.0} MiB", rss1 as f64 / 1024.0, hwm1 as f64 / 1024.0);
    println!("- ao final: {:.0} MiB RSS, pico {:.0} MiB", rss2 as f64 / 1024.0, hwm2 as f64 / 1024.0);
    let _ = (Path::new(""), hashes.len());
}
