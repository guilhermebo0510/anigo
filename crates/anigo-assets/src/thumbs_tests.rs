//! Testes do cache e dos renderizadores CPU (sem GPU).

use super::*;
use image::Rgba;

fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(c))
}

fn req_for(hash: ContentHash) -> ThumbRequest {
    ThumbRequest {
        id: AssetId::new(),
        kind: AssetKind::Model,
        format: "glb".into(),
        content_hash: hash,
        primary: PathBuf::from("/nao/importa"),
        files: vec![],
        asset_dir: PathBuf::new(),
        size: MASTER_SIZE,
    }
}

#[test]
fn resize_keeps_flat_colors_and_dimensions() {
    let src = solid(512, 512, [200, 100, 50, 255]);
    for s in [256u32, 128, 64] {
        let out = resize_rgba(&src, s, s).unwrap();
        assert_eq!((out.width(), out.height()), (s, s));
        for px in [out.get_pixel(0, 0), out.get_pixel(s / 2, s / 2), out.get_pixel(s - 1, s - 1)] {
            for (a, b) in px.0.iter().zip([200u8, 100, 50, 255]) {
                assert!((*a as i32 - b as i32).abs() <= 1, "{px:?}");
            }
        }
    }
    assert!(resize_rgba(&src, 0, 10).is_err());
    assert!(resize_rgba(&RgbaImage::new(0, 0), 10, 10).is_err());
}

#[test]
fn resize_does_not_darken_transparent_edges() {
    // metade esquerda opaca branca, metade direita transparente com RGB preto (o pior caso)
    let mut src = RgbaImage::new(64, 64);
    for y in 0..64 {
        for x in 0..32 {
            src.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    let out = resize_rgba(&src, 16, 16).unwrap();
    for y in 0..16 {
        for x in [7u32, 8] {
            let px = out.get_pixel(x, y).0;
            if px[3] > 8 {
                assert!(px[0] >= 240, "borda escureceu (alpha nao tratado): {px:?}");
            }
        }
    }
}

#[test]
fn fit_square_preserves_aspect_and_centers() {
    let src = solid(200, 100, [255, 0, 0, 255]);
    let out = fit_square(&src, 64).unwrap();
    assert_eq!((out.width(), out.height()), (64, 64));
    assert_eq!(out.get_pixel(32, 32).0[3], 255, "centro opaco");
    assert_eq!(out.get_pixel(32, 2).0[3], 0, "faixa de cima transparente (letterbox)");
    assert_eq!(out.get_pixel(32, 61).0[3], 0, "faixa de baixo transparente");
    let tall = fit_square(&solid(10, 100, [0, 0, 255, 255]), 50).unwrap();
    assert_eq!(tall.get_pixel(1, 25).0[3], 0);
    assert_eq!(tall.get_pixel(25, 25).0[3], 255);
    assert_eq!(fit_square(&solid(1, 1, [1, 2, 3, 255]), 32).unwrap().width(), 32);
}

#[test]
fn placeholder_and_swatch_are_deterministic_and_distinct() {
    let a = PlaceholderRenderer::draw(AssetKind::Model, 96);
    assert_eq!(a, PlaceholderRenderer::draw(AssetKind::Model, 96));
    let mut seen = HashSet::new();
    for k in AssetKind::ALL {
        let img = PlaceholderRenderer::draw(*k, 96);
        assert_eq!((img.width(), img.height()), (96, 96));
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "canto arredondado transparente");
        assert!(img.get_pixel(48, 48).0[3] > 0);
        seen.insert(img.into_raw());
    }
    assert!(seen.len() >= 8, "icones por tipo devem ser distintos (vistos: {})", seen.len());

    let mut m = ToonMaterial::default();
    m.base.color = [1.0, 0.0, 0.0, 1.0];
    m.shade.color = [0.1, 0.0, 0.0];
    let s1 = MaterialSwatchRenderer::draw(&m, 128);
    assert_eq!(s1, MaterialSwatchRenderer::draw(&m, 128));
    assert_eq!(s1.get_pixel(0, 0).0[3], 0, "fora da esfera transparente");
    let reds: Vec<u8> = s1.pixels().filter(|p| p.0[3] == 255).map(|p| p.0[0]).collect();
    assert!(reds.iter().any(|r| *r > 200) && reds.iter().any(|r| *r < 100), "esfera toon deve ter luz e sombra");
    m.base.color = [0.0, 1.0, 0.0, 1.0];
    assert_ne!(s1, MaterialSwatchRenderer::draw(&m, 128));
}

#[test]
fn disk_cache_layout_versioning_and_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailCache::new(dir.path(), 3, 10 * 1024 * 1024);
    let h = ContentHash(*blake3::hash(b"x").as_bytes());
    assert!(cache.cached_path(&h, 128).is_none());
    cache.store_master(&h, &solid(512, 512, [10, 20, 30, 255])).unwrap();
    for s in THUMB_SIZES {
        let p = cache.cached_path(&h, s).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name, format!("{}-v3.png", h.to_hex()));
        assert_eq!(p.parent().unwrap().file_name().unwrap().to_string_lossy(), h.to_hex()[..2]);
        let img = image::open(&p).unwrap();
        assert_eq!((img.width(), img.height()), (s, s));
    }
    // outra versao do pipeline nao enxerga as miniaturas antigas (invalidacao por chave)
    let v4 = ThumbnailCache::new(dir.path(), 4, 1024);
    assert!(v4.cached_path(&h, 64).is_none());
    assert!(v4.purge_stale(None) > 0);
    assert!(cache.cached_path(&h, 64).is_none());
}

#[test]
fn purge_stale_keeps_only_listed_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailCache::new(dir.path(), 1, 1 << 20);
    let (a, b) = (ContentHash(*blake3::hash(b"a").as_bytes()), ContentHash(*blake3::hash(b"b").as_bytes()));
    cache.store_master(&a, &solid(512, 512, [1, 1, 1, 255])).unwrap();
    cache.store_master(&b, &solid(512, 512, [2, 2, 2, 255])).unwrap();
    let keep: HashSet<ContentHash> = [a].into_iter().collect();
    assert!(cache.purge_stale(Some(&keep)) > 0);
    assert!(cache.cached_path(&a, 64).is_some());
    assert!(cache.cached_path(&b, 64).is_none());
    assert_eq!(cache.ram_len(), 0, "purge limpa a RAM");
    cache.clear();
    assert!(cache.cached_path(&a, 64).is_none());
}

#[test]
fn ram_lru_respects_the_byte_budget_and_evicts_oldest() {
    let dir = tempfile::tempdir().unwrap();
    // 64x64x4 = 16 KiB por miniatura; orcamento de 3 miniaturas de 64
    let budget = 3 * 64 * 64 * 4;
    let cache = ThumbnailCache::new(dir.path(), 1, budget);
    let hs: Vec<ContentHash> = (0..5u8).map(|i| ContentHash(*blake3::hash(&[i]).as_bytes())).collect();
    for h in &hs {
        cache.ram_put(*h, 64, Arc::new(solid(64, 64, [9, 9, 9, 255])));
        assert!(cache.ram_bytes() <= budget);
    }
    assert_eq!(cache.ram_len(), 3);
    assert!(cache.ram_get(&hs[0], 64).is_none() && cache.ram_get(&hs[1], 64).is_none(), "as mais antigas sairam");
    assert!(cache.ram_get(&hs[4], 64).is_some());
    // tocar um item o torna recente: hs[2] sobrevive, hs[3] (menos recente) sai
    let _ = cache.ram_get(&hs[2], 64);
    cache.ram_put(hs[0], 64, Arc::new(solid(64, 64, [1, 1, 1, 255])));
    assert!(cache.ram_get(&hs[2], 64).is_some());
    assert!(cache.ram_get(&hs[3], 64).is_none());
    // item maior que o orcamento inteiro nao entra
    cache.ram_put(hs[1], 512, Arc::new(solid(512, 512, [1, 1, 1, 255])));
    assert!(cache.ram_get(&hs[1], 512).is_none());
}

#[test]
fn load_decodes_disk_then_serves_from_ram_and_heals_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailCache::new(dir.path(), 1, 1 << 24);
    let h = ContentHash(*blake3::hash(b"z").as_bytes());
    cache.store_master(&h, &solid(512, 512, [5, 6, 7, 255])).unwrap();
    let fresh = ThumbnailCache::new(dir.path(), 1, 1 << 24);
    assert_eq!(fresh.ram_len(), 0);
    let img = fresh.load(&h, 128).unwrap();
    assert_eq!(img.get_pixel(5, 5).0, [5, 6, 7, 255]);
    assert_eq!(fresh.ram_len(), 1);
    // arquivo corrompido: load devolve None e remove o arquivo (sera regerado)
    let p = fresh.path_for(&h, 256);
    std::fs::write(&p, b"nao e png").unwrap();
    assert!(fresh.load(&h, 256).is_none());
    assert!(!p.exists());
    // exibicao: placeholder imediato quando nao ha cache
    let (img, ph) = fresh.load_or_placeholder(&ContentHash([7; 32]), AssetKind::Hair, 64);
    assert!(ph && img.width() == 64);
    let (_, ph) = fresh.load_or_placeholder(&h, AssetKind::Hair, 64);
    assert!(!ph);
}

#[test]
fn evict_to_removes_oldest_files_first() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailCache::new(dir.path(), 1, 1 << 20);
    let hs: Vec<ContentHash> = (0..4u8).map(|i| ContentHash(*blake3::hash(&[i, 1]).as_bytes())).collect();
    for (i, h) in hs.iter().enumerate() {
        cache.store_master(h, &solid(512, 512, [i as u8 * 40, 0, 0, 255])).unwrap();
        for s in THUMB_SIZES {
            let p = cache.path_for(h, s);
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64 * 100);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
    }
    let total = cache.disk_usage();
    assert!(total > 0);
    assert_eq!(cache.evict_to(total), 0);
    let freed = cache.evict_to(total / 2);
    assert!(freed > 0 && cache.disk_usage() <= total / 2);
    assert!(cache.cached_path(&hs[0], 64).is_none(), "o mais antigo sai primeiro");
    assert!(cache.cached_path(&hs[3], 64).is_some(), "o mais novo fica");
}

#[test]
fn ensure_skips_rendering_when_cached_and_never_persists_placeholders() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailCache::new(dir.path(), 1, 1 << 24);
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let renderer = ClosureRenderer::new("fake", move |req: &ThumbRequest| {
        c2.fetch_add(1, Ordering::SeqCst);
        Ok(Rendered { image: solid(req.size, req.size, [1, 2, 3, 255]), placeholder: false })
    });
    let req = req_for(ContentHash([3; 32]));
    assert_eq!(cache.ensure(&req, &renderer).unwrap(), Ensured { cached: false, placeholder: false });
    assert_eq!(cache.ensure(&req, &renderer).unwrap(), Ensured { cached: true, placeholder: false });
    assert_eq!(calls.load(Ordering::SeqCst), 1, "segunda chamada nao renderiza");
    // placeholder nao e persistido
    let req2 = req_for(ContentHash([4; 32]));
    assert_eq!(cache.ensure(&req2, &PlaceholderRenderer).unwrap(), Ensured { cached: false, placeholder: true });
    assert!(cache.cached_path(&req2.content_hash, 64).is_none());
    // erro e panic do renderizador viram erro, nao crash
    let failing = ClosureRenderer::new("falha", |_: &ThumbRequest| Err("sem gpu".to_string()));
    assert!(cache.ensure(&req2, &failing).is_err());
    let panicking = ClosureRenderer::new("panic", |_: &ThumbRequest| -> std::result::Result<Rendered, String> { panic!("boom") });
    assert!(matches!(cache.ensure(&req2, &panicking), Err(AssetError::Internal(_))));
    // imagem de tamanho errado e normalizada para o master
    let odd = ClosureRenderer::new("odd", |_: &ThumbRequest| Ok(Rendered { image: solid(100, 40, [9, 9, 9, 255]), placeholder: false }));
    let req3 = req_for(ContentHash([5; 32]));
    cache.ensure(&req3, &odd).unwrap();
    let m = image::open(cache.path_for(&req3.content_hash, 512)).unwrap();
    assert_eq!((m.width(), m.height()), (512, 512));
}

#[test]
fn chain_falls_through_and_reports_all_errors() {
    let mut req = req_for(ContentHash([1; 32]));
    req.kind = AssetKind::Texture;
    req.format = "tga".into();
    req.size = 64;
    let r = default_cpu_renderer().render(&req).unwrap();
    assert!(r.placeholder, "formato sem decodificador cai no placeholder");
    let chain = ChainRenderer::new(vec![
        Arc::new(ClosureRenderer::new("a", |_: &ThumbRequest| Err("x".to_string()))),
        Arc::new(ClosureRenderer::new("b", |_: &ThumbRequest| Err("y".to_string()))),
    ]);
    let e = chain.render(&req).err().unwrap();
    assert!(e.contains("a: x") && e.contains("b: y"), "{e}");
}

#[test]
fn image_renderer_rejects_corrupt_and_oversized() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.png");
    std::fs::write(&p, b"\x89PNG\r\n\x1a\n garbage").unwrap();
    let mut req = req_for(ContentHash([1; 32]));
    req.kind = AssetKind::Texture;
    req.format = "png".into();
    req.primary = p.clone();
    req.size = 64;
    assert!(ImageThumbnailRenderer::default().render(&req).is_err());
    let mut buf = std::io::Cursor::new(Vec::new());
    solid(64, 64, [1, 1, 1, 255]).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    std::fs::write(&p, buf.get_ref()).unwrap();
    assert!(ImageThumbnailRenderer::default().render(&req).is_ok());
    assert!(ImageThumbnailRenderer { max_dimension: 16, max_alloc: 1 << 20 }.render(&req).is_err());
    req.format = "tga".into();
    assert!(ImageThumbnailRenderer::default().render(&req).is_err());
}
