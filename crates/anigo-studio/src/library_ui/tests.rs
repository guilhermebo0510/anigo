//! Testes do workspace Biblioteca: egui sem janela (`Context::run_ui`), biblioteca real em disco temporário.
//! Nenhum exige GPU. O benchmark da grade (`grid_frame_time_*`) mede a UI com milhares de ativos.

use super::*;
use egui::{Pos2, Rect, Vec2};
use std::time::{Duration, Instant};

struct TestFile(PathBuf);
impl std::fmt::Debug for TestFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestFile({})", self.0.display())
    }
}
impl egui::DroppedFile for TestFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

struct Harness {
    ctx: egui::Context,
    ws: LibraryWorkspace,
    t: f64,
    size: Vec2,
    /// Shapes do último quadro e tempo de CPU (layout + tessellation) de cada quadro.
    last_shapes: usize,
    frame_ms: Vec<f64>,
    actions: Vec<HostAction>,
}

impl Harness {
    fn new(lib: Library) -> Self {
        Self::with_renderer(lib, Arc::new(cel_thumb::CelThumbnailRenderer::new()), default_cpu_renderer())
    }

    fn with_renderer(lib: Library, cel: Arc<cel_thumb::CelThumbnailRenderer>, renderer: Arc<dyn ThumbnailRenderer>) -> Self {
        let mut ws = LibraryWorkspace::empty(None);
        let ctx = egui::Context::default();
        crate::icons::install(&ctx);
        ws.set_context(&ctx);
        ws.attach_with_renderer(lib, cel, renderer);
        Self { ctx, ws, t: 0.0, size: Vec2::new(1440.0, 900.0), last_shapes: 0, frame_ms: Vec::new(), actions: Vec::new() }
    }

    fn frame_with(&mut self, mut input: egui::RawInput) {
        self.t += 0.016;
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, self.size));
        input.time = Some(self.t);
        let t0 = Instant::now();
        let mut out = self.ctx.run_ui(input, |ui| {
            let a = self.ws.ui(ui);
            self.actions.extend(a);
        });
        out.textures_delta.clear();
        self.last_shapes = out.shapes.len();
        let ppp = out.pixels_per_point;
        let prims = self.ctx.tessellate(out.shapes, ppp);
        self.frame_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(prims.len());
    }

    fn frame(&mut self) {
        self.frame_with(egui::RawInput::default());
    }

    fn run_until(&mut self, what: &str, mut cond: impl FnMut(&mut Harness) -> bool) {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            self.frame();
            if cond(self) {
                return;
            }
            assert!(Instant::now() < end, "tempo esgotado esperando: {what}");
            std::thread::sleep(Duration::from_millis(4));
        }
    }

    fn settle(&mut self) {
        let mut stable = 0;
        self.run_until("a UI assentar", |h| {
            if h.ws.is_settled() {
                stable += 1;
            } else {
                stable = 0;
            }
            stable >= 3
        });
    }
}

fn synthetic(n: usize) -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let lib = synth::build_synthetic(&dir.path().join("lib"), n, 11).unwrap();
    (dir, lib)
}

#[test]
fn first_page_loads_and_memory_stays_bounded() {
    let (_d, lib) = synthetic(1500);
    let mut h = Harness::new(lib);
    h.settle();
    assert_eq!(h.ws.list.total, Some(1500));
    assert!(h.ws.list.row(0).is_some());
    assert!(h.ws.list.loaded_pages() <= 8, "só as páginas perto da janela: {}", h.ws.list.loaded_pages());
    assert!(h.ws.facets.n_all == 1500, "facetas contam o total: {}", h.ws.facets.n_all);
    assert!(!h.ws.facets.tags.is_empty());
}

#[test]
fn jumping_far_loads_only_nearby_pages() {
    let (_d, lib) = synthetic(6000);
    let mut h = Harness::new(lib);
    h.settle();
    let (cols, row_h) = (h.ws.cols, h.ws.row_h);
    assert!(cols >= 4 && row_h > 100.0);
    // vai direto para o meio da lista (o que a barra de rolagem faz ao arrastar)
    h.ws.pending_scroll = Some((3000 / cols) as f32 * row_h);
    h.run_until("páginas do meio", |h| {
        let (first, _) = h.ws.visible;
        first >= 2900 && h.ws.list.row(first).is_some() && h.ws.list.row(first + 20).is_some()
    });
    h.settle();
    assert!(h.ws.list.loaded_pages() <= 16, "{} páginas em memória", h.ws.list.loaded_pages());
    let (first, last) = h.ws.visible;
    assert!(last > first && last - first < 200, "janela visível pequena: {first}..{last}");
}

#[test]
fn shape_count_does_not_grow_with_library_size() {
    let shapes_for = |n: usize| {
        let (_d, lib) = synthetic(n);
        let mut h = Harness::new(lib);
        h.settle();
        h.frame();
        h.last_shapes
    };
    let small = shapes_for(400);
    let big = shapes_for(4000);
    assert!(big as f64 <= small as f64 * 1.35 + 20.0, "a grade deve desenhar só o visível: {small} shapes (400) vs {big} (4000)");
}

#[test]
fn click_ctrl_shift_and_select_all() {
    let (_d, lib) = synthetic(700);
    let mut h = Harness::new(lib);
    h.settle();
    let ids = h.ws.library().unwrap().query_ids(&h.ws.filters.to_query(), 700).unwrap();
    let none = egui::Modifiers::default();
    h.ws.click_item(2, ids[2], none);
    assert_eq!(h.ws.selection.len(), 1);
    assert_eq!(h.ws.selection.primary, Some(ids[2]));
    // Shift: faixa entre o âncora e o clique, atravessando páginas (resolvida na thread de consulta)
    h.ws.click_item(250, ids[250], egui::Modifiers::SHIFT);
    h.run_until("faixa", |h| h.ws.selection.len() == 249);
    for id in &ids[2..=250] {
        assert!(h.ws.selection.contains(id));
    }
    // Ctrl alterna um item sem perder o resto (e vira o novo âncora)
    h.ws.click_item(300, ids[300], egui::Modifiers::COMMAND);
    assert_eq!(h.ws.selection.len(), 250);
    h.ws.click_item(300, ids[300], egui::Modifiers::COMMAND);
    assert_eq!(h.ws.selection.len(), 249);
    // um clique simples recomeça
    h.ws.click_item(5, ids[5], none);
    assert_eq!(h.ws.selection.len(), 1);
    // Ctrl+A
    h.ws.select_all();
    h.run_until("tudo", |h| h.ws.selection.len() == 700);
}

#[test]
fn favorites_and_ratings_are_optimistic_then_persisted() {
    let (_d, lib) = synthetic(300);
    let mut h = Harness::new(lib.clone());
    h.settle();
    let row = h.ws.list.row(3).unwrap().clone();
    h.ws.set_favorite(&[row.id], !row.favorite);
    assert_eq!(h.ws.list.row(3).unwrap().favorite, !row.favorite, "a grade reflete na hora");
    h.ws.set_rating(row.id, 5);
    assert_eq!(h.ws.list.row(3).unwrap().rating, 5);
    h.run_until("gravar no disco", |_| {
        let r = lib.require_row(&row.id).unwrap();
        r.favorite == !row.favorite && r.rating == 5
    });
    // o manifesto (fonte da verdade) também
    let m = lib.manifest(&row.id).unwrap();
    assert_eq!((m.favorite, m.rating), (!row.favorite, 5));
}

#[test]
fn text_search_with_operators_changes_the_list() {
    let (_d, lib) = synthetic(2000);
    let mut h = Harness::new(lib.clone());
    h.settle();
    h.ws.search_text = "cabelo -vestido".into();
    h.ws.search_edited_at = Some(Instant::now() - Duration::from_secs(1));
    h.run_until("busca", |h| h.ws.list.total.is_some_and(|t| t < 2000) && h.ws.list.row(0).is_some());
    let expected = lib.count(&Query { text: Some("cabelo -vestido".into()), ..Default::default() }).unwrap();
    assert_eq!(h.ws.list.total, Some(expected));
    assert!(expected > 0);
    let first = h.ws.list.row(0).unwrap();
    assert!(first.name.to_lowercase().contains("cabelo") || first.name.to_lowercase().contains("cabel"), "{}", first.name);
    assert!(!first.name.to_lowercase().contains("vestido"));
    // seção + tipo
    h.ws.set_section(Section::Favorites);
    h.ws.filters.text.clear();
    h.ws.search_text.clear();
    h.ws.filters.kinds = vec![AssetKind::Hair];
    h.ws.filters_changed();
    h.settle();
    let n = lib.count(&Query { favorite: Some(true), kinds: vec![AssetKind::Hair], ..Default::default() }).unwrap();
    assert_eq!(h.ws.list.total, Some(n));
}

fn write_cube_glb(dir: &std::path::Path, name: &str, size: f32) -> PathBuf {
    let p = dir.join(name);
    anigo_core::Mesh::create_cube(size).to_glb_file(&p).unwrap();
    p
}

#[test]
fn import_reports_each_file_including_errors_and_cancel_state_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_cube_glb(&src, "bom.glb", 1.0);
    let good = std::fs::read(src.join("bom.glb")).unwrap();
    std::fs::write(src.join("truncado.glb"), &good[..good.len() / 2]).unwrap();
    std::fs::write(src.join("lixo.glb"), b"isto nao e glb").unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(src.join("t.png")).unwrap();
    std::fs::write(src.join("notas.txt"), b"ignorado").unwrap();
    let mut h = Harness::new(lib.clone());
    h.settle();
    h.ws.queue_import(vec![src.clone()], ImportOptions::default());
    h.run_until("import", |h| h.ws.jobs.import.is_none() && h.ws.jobs.queue.is_empty() && h.ws.jobs.log.imported + h.ws.jobs.log.failed >= 4);
    let log = &h.ws.jobs.log;
    assert_eq!(log.imported, 2, "bom.glb e t.png: {:?}", log.entries);
    assert_eq!(log.failed, 2, "truncado e lixo viram erro por arquivo: {:?}", log.entries);
    assert!(log.entries.iter().filter(|e| e.kind == jobs::EntryKind::Failed).all(|e| !e.detail.is_empty()), "o erro tem mensagem");
    assert!(log.open, "o relatório abre sozinho");
    h.settle();
    assert_eq!(h.ws.list.total, Some(2), "só o que importou aparece na grade");
    // re-importar a mesma pasta: nada novo (duplicados por conteúdo)
    h.ws.jobs.log.clear();
    h.ws.queue_import(vec![src], ImportOptions::default());
    h.run_until("reimport", |h| h.ws.jobs.import.is_none() && h.ws.jobs.log.duplicates >= 2);
    assert_eq!(h.ws.jobs.log.imported, 0);
}

#[test]
fn files_dropped_from_the_os_start_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let glb = write_cube_glb(dir.path(), "solto.glb", 2.0);
    let mut h = Harness::new(lib.clone());
    h.settle();
    let mut input = egui::RawInput::default();
    input.dropped_files.push(Arc::new(TestFile(glb)));
    h.frame_with(input);
    h.run_until("import do arquivo solto", |h| h.ws.jobs.log.imported == 1 && h.ws.jobs.import.is_none());
    h.settle();
    let rows = lib.query(&Query::default()).unwrap().items;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "solto");
    assert_eq!(rows[0].format, "glb");
    assert!(rows[0].tris > 0, "estatísticas reais do GLB");
}

#[test]
fn trash_restore_purge_and_blocked_by_dependents() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let a = write_cube_glb(dir.path(), "a.glb", 1.0);
    let b = write_cube_glb(dir.path(), "b.glb", 1.5);
    let ida = lib.import_blocking(&a, ImportOptions::default()).unwrap().imported[0];
    let idb = lib.import_blocking(&b, ImportOptions::default()).unwrap().imported[0];
    lib.add_dependency(&idb, &ida, "texture").unwrap(); // b usa a
    let mut h = Harness::new(lib.clone());
    h.settle();
    // a é usado por b: a lixeira recusa e pede confirmação
    h.ws.trash_ids(vec![ida]);
    h.run_until("bloqueio", |h| h.ws.dlg.confirm.is_some());
    assert!(matches!(h.ws.dlg.confirm, Some(dialogs::Confirm::ForceTrash(ref v)) if v.len() == 1 && v[0].0 == ida));
    assert_eq!(lib.require_row(&ida).unwrap().state, AssetState::Active, "nada mudou antes da confirmação");
    // b não é usado por ninguém: vai para a lixeira
    h.ws.dlg.confirm = None;
    h.ws.trash_ids(vec![idb]);
    h.run_until("lixeira", |_| lib.require_row(&idb).unwrap().state == AssetState::Trashed);
    h.settle();
    assert_eq!(h.ws.facets.n_trash, 1);
    h.ws.set_section(Section::Trash);
    h.settle();
    assert_eq!(h.ws.list.total, Some(1));
    h.ws.restore_ids(vec![idb]);
    h.run_until("restaurar", |_| lib.require_row(&idb).unwrap().state == AssetState::Active);
    // excluir definitivamente exige a lixeira primeiro
    lib.trash(&[idb], TrashOptions::default()).unwrap();
    h.ws.purge_ids(vec![idb]);
    h.run_until("excluir", |_| lib.row(&idb).unwrap().is_none());
    assert!(lib.row(&ida).unwrap().is_some());
}

#[test]
fn edit_in_modeling_emits_a_host_action_only_for_meshes() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let glb = write_cube_glb(dir.path(), "cubo.glb", 1.0);
    let png = dir.path().join("t.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([9, 9, 9, 255])).save(&png).unwrap();
    let id_m = lib.import_blocking(&glb, ImportOptions::default()).unwrap().imported[0];
    let id_t = lib.import_blocking(&png, ImportOptions::default()).unwrap().imported[0];
    let mut h = Harness::new(lib);
    h.settle();
    h.ws.edit_in_modeling(id_t);
    h.frame();
    assert!(h.actions.is_empty(), "textura não abre na Modelagem");
    h.ws.edit_in_modeling(id_m);
    h.frame();
    match h.actions.as_slice() {
        [HostAction::EditInModeling { path, format, name }] => {
            assert!(path.is_file());
            assert_eq!((format.as_str(), name.as_str()), ("glb", "cubo"));
            // e o caminho leva à malha editável
            let mesh = crate::library::import_model_mesh(path, format).unwrap();
            assert!(mesh.face_count() > 0);
        }
        other => panic!("ação inesperada: {other:?}"),
    }
}

#[test]
fn authored_assets_get_a_new_revision_instead_of_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let mut h = Harness::new(lib.clone());
    h.settle();
    let a = write_cube_glb(dir.path(), "peca.glb", 1.0);
    let msg = h.ws.import_authored("Peça", &a).unwrap();
    assert!(msg.contains("registrado"), "{msg}");
    let rows = lib.query(&Query::default()).unwrap().items;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Peça");
    assert!(lib.tags_of(&rows[0].id).unwrap().contains(&"autoral".to_string()));
    // mesmo conteúdo: não duplica
    let msg = h.ws.import_authored("Peça", &a).unwrap();
    assert!(msg.contains("já existe") || msg.contains("revisão"), "{msg}");
    assert_eq!(lib.count(&Query::default()).unwrap(), 1);
    // conteúdo novo: mesma identidade, revisão 2
    let b = write_cube_glb(dir.path(), "peca2.glb", 3.0);
    let msg = h.ws.import_authored("Peça", &b).unwrap();
    assert!(msg.contains("revisão"), "{msg}");
    let rows = lib.query(&Query::default()).unwrap().items;
    assert_eq!(rows.len(), 1, "ainda um ativo só");
    assert_eq!(rows[0].revision, 2);
}

#[test]
fn duplicates_window_lists_groups_and_resolves_them() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let a = write_cube_glb(dir.path(), "a.glb", 1.0);
    let id1 = lib.import_blocking(&a, ImportOptions::default()).unwrap().imported[0];
    let id2 = lib.import_blocking(&a, ImportOptions { duplicates: DuplicatePolicy::Allow, name: Some("a (cópia)".into()), ..Default::default() }).unwrap().imported[0];
    let mut h = Harness::new(lib.clone());
    h.settle();
    assert_eq!(h.ws.facets.dup_groups, 1);
    h.ws.open_duplicates();
    h.run_until("grupos", |h| matches!(&h.ws.dlg.dups, Some(Ok(g)) if g.len() == 1));
    let Some(Ok(groups)) = &h.ws.dlg.dups else { unreachable!() };
    assert_eq!(groups[0].assets.len(), 2);
    let keep = h.ws.dlg.dup_keep[&groups[0].content_hash];
    let other = if keep == id1 { id2 } else { id1 };
    lib.resolve_duplicates(&keep, &[other]).unwrap();
    assert_eq!(lib.require_row(&other).unwrap().state, AssetState::Trashed);
    assert_eq!(lib.require_row(&keep).unwrap().state, AssetState::Active);
}

#[test]
fn removing_a_linked_root_never_touches_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let ext = dir.path().join("externo");
    std::fs::create_dir_all(&ext).unwrap();
    let f = write_cube_glb(&ext, "x.glb", 1.0);
    let before = std::fs::read(&f).unwrap();
    let mut h = Harness::new(lib.clone());
    h.settle();
    h.ws.add_root(ext.clone(), true);
    h.run_until("scan do local ligado", |h| h.ws.jobs.scan.is_none() && h.ws.jobs.scan_queue.is_empty() && lib.count(&Query::default()).unwrap_or(0) == 1);
    let root = lib.roots().unwrap().into_iter().find(|r| r.mode == RootMode::Linked).unwrap();
    h.ws.dlg.confirm = Some(dialogs::Confirm::RemoveRoot { id: root.id, name: root.name, managed: false, delete_files: false });
    let c = h.ws.dlg.confirm.take().unwrap();
    h.ws.run_confirm(c);
    h.run_until("remover", |_| lib.roots().unwrap().iter().all(|r| r.mode != RootMode::Linked));
    assert_eq!(std::fs::read(&f).unwrap(), before, "o arquivo externo continua igual");
}

#[test]
fn scenario_library_has_real_synthetic_missing_trashed_duplicate_and_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let lib = synth::build_synthetic(&dir.path().join("lib"), 250, 3).unwrap();
    let sc = synth::populate_scenario(&lib, &dir.path().join("work")).unwrap();
    assert!(sc.models.len() >= 8, "primitivas + OBJ reais: {}", sc.models.len());
    assert!(sc.character.is_some());
    assert_eq!(sc.textures.len(), 4);
    assert_eq!(sc.materials.len(), 3);
    let missing = sc.linked_missing.expect("ativo ausente");
    assert_eq!(lib.require_row(&missing).unwrap().state, AssetState::Missing);
    assert_eq!(sc.trashed.len(), 1);
    assert_eq!(lib.require_row(&sc.trashed[0]).unwrap().state, AssetState::Trashed);
    assert!(lib.find_duplicates(100).unwrap().iter().any(|g| g.assets.iter().any(|a| a.name.contains("cópia"))));
    assert!(!lib.dependencies(&sc.models[0]).unwrap().is_empty());
    assert_eq!(lib.collections().unwrap().len(), 2);
    // os GLB reais têm estatísticas medidas do arquivo
    let row = lib.require_row(&sc.character.unwrap()).unwrap();
    assert!(row.tris > 1000 && row.kind == AssetKind::Character, "{row:?}");
    // a pasta de importação tem arquivos bons e quebrados
    assert!(sc.import_dir.join("lixo.glb").is_file() && sc.import_dir.join("truncado.glb").is_file());
}

#[test]
fn grid_frame_time_with_synthetic_library() {
    // N via ANIGO_BENCH_N (padrão 5 000 para o CI; 50 000 em release: `--release -- --ignored`)
    let n: usize = std::env::var("ANIGO_BENCH_N").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
    run_frame_time_bench(n, cfg!(debug_assertions));
}

/// O número do plano (T-5.10): 50 000 ativos. Rode com
/// `cargo test -p anigo-studio --release grid_frame_time_50k -- --ignored --nocapture`.
#[test]
#[ignore]
fn grid_frame_time_50k() {
    run_frame_time_bench(50_000, false);
}

fn run_frame_time_bench(n: usize, debug_build: bool) {
    let dir = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let lib = synth::build_synthetic(&dir.path().join("lib"), n, 99).unwrap();
    let build_s = t.elapsed().as_secs_f32();
    let mut h = Harness::new(lib);
    h.settle();
    let (cols, row_h) = (h.ws.cols.max(1), h.ws.row_h);
    let rows = n.div_ceil(cols);
    let max_off = (rows as f32 * row_h - h.ws.viewport_h).max(0.0);
    h.frame_ms.clear();
    // rolagem contínua do topo ao fim em 240 passos (como arrastar a barra), sem esperar as páginas
    for step in 0..=240 {
        h.ws.pending_scroll = Some(max_off * step as f32 / 240.0);
        h.frame();
        if step % 4 == 0 {
            std::thread::sleep(Duration::from_millis(2)); // dá chance aos workers, como num quadro real de 16 ms
        }
    }
    // rolagem fina (uma linha por quadro) no meio da lista
    for step in 0..120 {
        h.ws.pending_scroll = Some(max_off * 0.5 + step as f32 * row_h * 0.25);
        h.frame();
    }
    // quadros parados (nada muda): custo de manter a janela aberta
    let still_start = h.frame_ms.len();
    for _ in 0..60 {
        h.frame();
    }
    let mut moving: Vec<f64> = h.frame_ms[..still_start].to_vec();
    let mut still: Vec<f64> = h.frame_ms[still_start..].to_vec();
    moving.sort_by(|a, b| a.partial_cmp(b).unwrap());
    still.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |v: &[f64], p: f64| v[((v.len() as f64 - 1.0) * p) as usize];
    eprintln!(
        "BENCH grade {n} ativos ({} colunas, {} linhas, {}): build {build_s:.1}s | rolando: mediana {:.2} ms, p95 {:.2} ms, max {:.2} ms | parada: mediana {:.2} ms | páginas em memória {} | texturas {}",
        cols,
        rows,
        if debug_build { "DEBUG" } else { "release" },
        q(&moving, 0.5),
        q(&moving, 0.95),
        moving.last().unwrap(),
        q(&still, 0.5),
        h.ws.list.loaded_pages(),
        h.ws.tex.len(),
    );
    assert!(h.ws.list.loaded_pages() <= 48, "memória constante: {} páginas", h.ws.list.loaded_pages());
    // orçamento de CPU da UI (layout + tessellation). Debug é ~10x mais lento: o limite só pega regressões grosseiras.
    let (budget_p95, budget_med) = if debug_build { (120.0, 60.0) } else { (8.0, 4.0) };
    assert!(q(&moving, 0.95) < budget_p95, "p95 {:.2} ms acima de {budget_p95} ms", q(&moving, 0.95));
    assert!(q(&moving, 0.5) < budget_med, "mediana {:.2} ms acima de {budget_med} ms", q(&moving, 0.5));
}

#[test]
fn thumbnails_degrade_to_icons_when_the_gpu_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let glb = write_cube_glb(dir.path(), "cubo.glb", 1.0);
    let png = dir.path().join("t.png");
    image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 20, 20, 255])).save(&png).unwrap();
    let id_g = lib.import_blocking(&glb, ImportOptions::default()).unwrap().imported[0];
    let id_p = lib.import_blocking(&png, ImportOptions::default()).unwrap().imported[0];
    let (hg, hp) = (lib.require_row(&id_g).unwrap().content_hash, lib.require_row(&id_p).unwrap().content_hash);
    let cel = Arc::new(cel_thumb::CelThumbnailRenderer::unavailable("sem adaptador (teste)"));
    let chain: Arc<dyn ThumbnailRenderer> = Arc::new(ChainRenderer::new(vec![cel.clone(), default_cpu_renderer()]));
    let mut h = Harness::with_renderer(lib, cel.clone(), chain);
    h.run_until("miniaturas", |h| {
        h.frame();
        h.ws.tex.best_available(&hp).is_some() && h.ws.tex.unavailable.contains(&hg)
    });
    assert_eq!(cel.available(), Some(false));
    assert!(h.ws.tex.best_available(&hg).is_none(), "o modelo fica com o ícone do tipo, sem miniatura inventada");
    h.settle();
}

#[test]
fn real_3d_thumbnail_is_rendered_and_cached_when_a_gpu_exists() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path().join("lib")).unwrap();
    let glb = write_cube_glb(dir.path(), "cubo.glb", 1.0);
    let id = lib.import_blocking(&glb, ImportOptions::default()).unwrap().imported[0];
    let hash = lib.require_row(&id).unwrap().content_hash;
    let cel = Arc::new(cel_thumb::CelThumbnailRenderer::new());
    let chain: Arc<dyn ThumbnailRenderer> = Arc::new(ChainRenderer::new(vec![cel.clone(), default_cpu_renderer()]));
    let mut h = Harness::with_renderer(lib.clone(), cel.clone(), chain);
    h.run_until("miniatura", |h| h.ws.tex.best_available(&hash).is_some() || h.ws.tex.unavailable.contains(&hash));
    if cel.available() != Some(true) {
        eprintln!("sem GPU neste ambiente: {:?}; teste da miniatura 3D real pulado", cel.failure_reason());
        return;
    }
    let path = lib.thumbnails().cached_path(&hash, 512).expect("master de 512 no cache de disco");
    let img = image::open(path).unwrap().to_rgba8();
    let bg = img.get_pixel(0, 0).0;
    let fg = img.pixels().filter(|p| p.0 != bg).count();
    assert!(fg > 20_000, "o cubo cobre boa parte do quadro: {fg} px");
    // as quatro escalas existem em disco (64/128/256/512)
    for size in anigo_assets::THUMB_SIZES {
        assert!(lib.thumbnails().cached_path(&hash, size).is_some(), "falta o tamanho {size}");
    }
}

#[test]
fn dropping_cards_on_the_side_panel_targets_edits_the_library() {
    let (_d, lib) = synthetic(120);
    let mut h = Harness::new(lib.clone());
    h.settle();
    let ids = lib.query_ids(&Query::default(), 3).unwrap();
    let coll = lib.create_collection("Alvo").unwrap();
    h.ws.drop_on(side::DropTarget::Collection(coll, "Alvo".into()), ids.clone());
    h.run_until("coleção", |_| lib.query_ids(&Query { collection: Some(coll), ..Default::default() }, 10).unwrap().len() == 3);
    h.ws.drop_on(side::DropTarget::Tag("cena/arrastada".into()), ids.clone());
    h.run_until("tag", |_| lib.query_ids(&Query { tags: vec!["cena/arrastada".into()], ..Default::default() }, 10).unwrap().len() == 3);
    h.settle();
    let node = h.ws.facets.tags.iter().find(|n| n.path == "cena").expect("pai sintetizado na árvore");
    assert_eq!(node.total, 3, "o pai conta os descendentes");
    h.ws.drop_on(side::DropTarget::Favorites, ids.clone());
    h.run_until("favoritos", |_| ids.iter().all(|i| lib.require_row(i).unwrap().favorite));
    h.ws.drop_on(side::DropTarget::Trash, ids.clone());
    h.run_until("lixeira", |_| ids.iter().all(|i| lib.require_row(i).unwrap().state == AssetState::Trashed));
}

#[test]
fn metadata_edits_reach_the_manifest_and_the_search_index() {
    let (_d, lib) = synthetic(80);
    let mut h = Harness::new(lib.clone());
    h.settle();
    let id = h.ws.list.row(0).unwrap().id;
    h.ws.update_info(id, InfoPatch { name: Some("Nome totalmente novo".into()), description: Some("uma descricao unica zebra".into()), author: Some("Eu".into()), ..Default::default() });
    h.run_until("gravar", |_| lib.manifest(&id).unwrap().name == "Nome totalmente novo");
    let m = lib.manifest(&id).unwrap();
    assert_eq!((m.description.as_str(), m.author.as_str()), ("uma descricao unica zebra", "Eu"));
    // o índice FTS enxerga o nome e a descrição novos
    for q in ["totalmente", "zebra", "desc:unica"] {
        let found = lib.query_ids(&Query { text: Some(q.into()), ..Default::default() }, 10).unwrap();
        assert!(found.contains(&id), "busca {q:?} deveria achar o ativo editado");
    }
}
