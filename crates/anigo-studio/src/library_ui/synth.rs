//! Bibliotecas SINTÉTICAS para cenários de screenshot, testes e benchmark da grade.
//!
//! Duas camadas:
//! * [`build_synthetic`] — N manifestos `asset.json` reais em disco (sem arquivo de payload) + índice
//!   reconstruído; é o mesmo gerador de `crates/anigo-assets/examples/bench_scale.rs`. Serve para medir a
//!   grade; não tem conteúdo 3D (as miniaturas desses itens são o ícone do tipo).
//! * [`populate_scenario`] — sobre isso, ativos REAIS gerados pelo próprio studio (GLB das primitivas e das
//!   bases canônicas, OBJ, PNG, materiais toon), um local ligado com um arquivo ausente, itens na lixeira,
//!   duplicados, dependências e coleções. São os que têm miniatura 3D de verdade.

use anigo_assets::*;
use std::path::{Path, PathBuf};

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
    let ext = if kind == AssetKind::Texture { "png" } else { "glb" };
    AssetManifest {
        schema: MANIFEST_SCHEMA,
        asset_id: AssetId::new(),
        name,
        kind,
        description: if r.below(4) == 0 { format!("{} {} para cenas {}", NOUN[r.below(NOUN.len())], ADJ[r.below(ADJ.len())], STYLE[r.below(STYLE.len())]) } else { String::new() },
        author: ["Ana", "Bia", "Caio", "Davi", "Eva", "Fabio"][r.below(6)].to_string(),
        license: "CC0".into(),
        source_url: String::new(),
        format: ext.into(),
        storage: StorageMode::Managed,
        primary: format!("asset.{ext}"),
        link: None,
        current_revision: 1,
        revisions: vec![RevisionInfo { rev: 1, content_hash: hash, size, created_at: now, files: vec![FileEntry { path: format!("asset.{ext}"), size, blake3: hash }], note: String::new() }],
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

/// Cria uma biblioteca em `home` com `n` ativos sintéticos (manifestos reais, sem payload).
pub fn build_synthetic(home: &Path, n: usize, seed: u64) -> Result<Library> {
    let lib = Library::open_with(home, OpenOptions { pipeline_ver: super::cel_thumb::PIPELINE_VER, ..Default::default() })?;
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ seed.wrapping_mul(0x2545F4914F6CDD1D) | 1);
    let mut hashes: Vec<ContentHash> = Vec::new();
    for i in 0..n {
        let dup = if i > 100 && rng.below(100) == 0 { Some(hashes[rng.below(hashes.len())]) } else { None };
        let m = synth_manifest(i, &mut rng, dup);
        hashes.push(m.content_hash().unwrap());
        let dir = lib.home().join(layout::managed_dir_rel(&m.asset_id));
        std::fs::create_dir_all(&dir).map_err(|e| AssetError::Internal(format!("{}: {e}", dir.display())))?;
        std::fs::write(dir.join(MANIFEST_FILE), m.to_json_pretty()?).map_err(|e| AssetError::Internal(e.to_string()))?;
    }
    if n > 0 {
        lib.rebuild_index()?;
    }
    Ok(lib)
}

// ---------------------------------------------------------------------------------------------
// Ativos reais
// ---------------------------------------------------------------------------------------------

/// O que `populate_scenario` criou (para os cenários escolherem o que selecionar).
#[derive(Debug, Default, Clone)]
pub struct ScenarioAssets {
    pub models: Vec<AssetId>,
    pub character: Option<AssetId>,
    pub textures: Vec<AssetId>,
    pub materials: Vec<AssetId>,
    pub linked_missing: Option<AssetId>,
    pub trashed: Vec<AssetId>,
    pub work_dir: PathBuf,
    /// Pasta com arquivos bons e quebrados para a tela de importação.
    pub import_dir: PathBuf,
}

fn write_png(path: &Path, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let c = f(x, y);
        image::Rgba([c[0], c[1], c[2], 255])
    });
    img.save(path).expect("png");
}

fn write_obj_pyramid(path: &Path, base: f32, height: f32) {
    let b = base / 2.0;
    let s = format!(
        "# piramide gerada pelo studio\no Piramide\nv {b} 0 {b}\nv {nb} 0 {b}\nv {nb} 0 {nb}\nv {b} 0 {nb}\nv 0 {height} 0\n\
         f 1 2 3 4\nf 1 5 2\nf 2 5 3\nf 3 5 4\nf 4 5 1\n",
        nb = -b
    );
    std::fs::write(path, s).expect("obj");
}

/// Escreve os GLB de primitivas e bases canônicas em `dir`. Devolve `(nome, caminho)`.
pub fn write_scenario_glbs(dir: &Path) -> Vec<(String, PathBuf)> {
    use crate::modeling::{ModelingSession, PrimitiveKind};
    std::fs::create_dir_all(dir).expect("dir");
    let mut out = Vec::new();
    for (kind, name) in [
        (PrimitiveKind::QuadSphere, "esfera_quad"),
        (PrimitiveKind::Torus, "toro_aneis"),
        (PrimitiveKind::Cone, "cone_festa"),
        (PrimitiveKind::Cylinder, "pilar_cilindro"),
        (PrimitiveKind::Capsule, "capsula_prop"),
        (PrimitiveKind::UvSphere, "orbe_uv"),
        (PrimitiveKind::Cube, "caixa_cubo"),
    ] {
        let mut s = ModelingSession::new();
        s.add_primitive(kind, glam::Vec3::ZERO);
        let mut obj = s.active_object().expect("objeto").clone();
        obj.name = name.to_string();
        if let Ok(p) = crate::library::export_object_glb(&obj, dir) {
            out.push((name.replace('_', " "), p));
        }
    }
    for (g, name) in [(anigo_core::mesh::BaseGender::Female, "base_feminina"), (anigo_core::mesh::BaseGender::Male, "base_masculina")] {
        let mesh = anigo_core::Mesh::create_canonical_base(g);
        let p = dir.join(format!("{name}.glb"));
        if mesh.to_glb_file(&p).is_ok() {
            out.push((name.replace('_', " "), p));
        }
    }
    out
}

/// Popula uma biblioteca (já com os sintéticos) com ativos reais, ligados, lixeira, duplicados e coleções.
pub fn populate_scenario(lib: &Library, work: &Path) -> Result<ScenarioAssets> {
    let mut sc = ScenarioAssets { work_dir: work.to_path_buf(), ..Default::default() };
    let src = work.join("fontes");
    let glbs = write_scenario_glbs(&src.join("glb"));
    let mut hair_idx = 0usize;
    for (i, (name, path)) in glbs.iter().enumerate() {
        let is_char = name.starts_with("base");
        let mut tags = vec!["estilo/cel/noir".to_string(), "origem/propria".to_string()];
        if is_char {
            tags.push("categoria/personagem".into());
        } else {
            tags.push("categoria/prop".into());
        }
        let rep = lib.import_blocking(
            path,
            ImportOptions {
                name: Some(capitalize(name)),
                tags,
                author: Some("ANIGO".into()),
                license: Some("CC0".into()),
                description: Some(format!("{} gerado pelo próprio studio para o cenário de captura.", capitalize(name))),
                kind: if is_char { Some(AssetKind::Character) } else if i % 3 == 0 { Some(AssetKind::Accessory) } else { Some(AssetKind::Model) },
                ..Default::default()
            },
        )?;
        for id in rep.imported {
            lib.set_rating(&id, (3 + (i % 3)) as u8)?;
            if i % 2 == 0 {
                lib.set_favorite(&id, true)?;
            }
            if is_char {
                sc.character.get_or_insert(id);
            } else {
                sc.models.push(id);
            }
            hair_idx += 1;
        }
    }
    let _ = hair_idx;
    // OBJ
    let objs = src.join("obj");
    std::fs::create_dir_all(&objs).ok();
    write_obj_pyramid(&objs.join("piramide.obj"), 1.4, 1.8);
    write_obj_pyramid(&objs.join("obelisco.obj"), 0.6, 3.0);
    let r = lib.import_blocking(&objs, ImportOptions { tags: vec!["formato/obj".into()], ..Default::default() })?;
    sc.models.extend(r.imported);
    // texturas
    let texdir = src.join("tex");
    std::fs::create_dir_all(&texdir).ok();
    write_png(&texdir.join("xadrez.png"), 128, 128, |x, y| if (x / 16 + y / 16) % 2 == 0 { [235, 235, 240] } else { [60, 60, 90] });
    write_png(&texdir.join("gradiente_por_do_sol.png"), 128, 128, |x, y| [(255 - y * 2).min(255) as u8, (80 + x).min(255) as u8, (160 + y).min(255) as u8]);
    write_png(&texdir.join("pele_base.png"), 128, 128, |x, y| [(236 - y / 8) as u8, (196 - y / 6 + x / 16) as u8, (170 - x / 8) as u8]);
    write_png(&texdir.join("ruido_listras.png"), 128, 128, |x, y| {
        let v = (((x * 3 + y * 5) % 64) * 4) as u8;
        [v, 40, 255 - v]
    });
    let r = lib.import_blocking(&texdir, ImportOptions { tags: vec!["categoria/textura".into()], ..Default::default() })?;
    sc.textures = r.imported;
    // materiais toon canônicos
    let matdir = src.join("mat");
    std::fs::create_dir_all(&matdir).ok();
    for (name, base, shade) in [("pele_anime", [1.0, 0.82, 0.74], [0.85, 0.58, 0.56]), ("roupa_azul", [0.25, 0.45, 0.95], [0.12, 0.2, 0.55]), ("cabelo_rosa", [1.0, 0.45, 0.7], [0.65, 0.2, 0.45])] {
        let mut m = ToonMaterial::default();
        m.name = capitalize(&name.replace('_', " "));
        m.base.color = [base[0], base[1], base[2], 1.0];
        m.shade.color = shade;
        std::fs::write(matdir.join(format!("{name}{MATERIAL_SUFFIX}")), m.to_json()?).ok();
    }
    let r = lib.import_blocking(&matdir, ImportOptions { tags: vec!["categoria/material".into()], ..Default::default() })?;
    sc.materials = r.imported;
    // dependência: o primeiro modelo "usa" a textura xadrez e um material
    if let (Some(m), Some(t)) = (sc.models.first(), sc.textures.first()) {
        let _ = lib.add_dependency(m, t, "texture");
    }
    if let (Some(m), Some(mat)) = (sc.models.first(), sc.materials.first()) {
        let _ = lib.add_dependency(m, mat, "material");
    }
    // duplicado real: o mesmo GLB importado de novo (política Allow)
    if let Some((name, path)) = glbs.first() {
        let _ = lib.import_blocking(path, ImportOptions { name: Some(format!("{} (cópia)", capitalize(name))), duplicates: DuplicatePolicy::Allow, ..Default::default() })?;
    }
    // local ligado: 3 arquivos, um é apagado e o scan o marca como ausente
    let ext = work.join("externo");
    std::fs::create_dir_all(&ext).ok();
    for (name, path) in glbs.iter().take(2) {
        let _ = std::fs::copy(path, ext.join(format!("{}.glb", name.replace(' ', "_"))));
    }
    write_png(&ext.join("referencia_cenario.png"), 96, 96, |x, y| [(x * 2) as u8, (y * 2) as u8, 140]);
    std::fs::write(ext.join("anotacoes.txt"), b"nao e um ativo").ok();
    let root = lib.add_root(&ext, RootMode::Linked, Some("Projeto externo"))?;
    lib.scan_blocking(root, ScanOptions::default())?;
    let victim = ext.join("referencia_cenario.png");
    let id = lib.find_by_path(root.0, "referencia_cenario.png")?;
    std::fs::remove_file(&victim).ok();
    lib.scan_blocking(root, ScanOptions::default())?;
    sc.linked_missing = id;
    // lixeira
    if let Some(t) = sc.textures.get(1) {
        lib.trash(&[*t], TrashOptions::default())?;
        sc.trashed.push(*t);
    }
    // coleções
    let col = lib.create_collection("Props da cena")?;
    lib.add_to_collection(col, &sc.models.iter().take(4).copied().collect::<Vec<_>>())?;
    lib.create_smart_collection("Bem avaliados do estilo noir", &Query { tags: vec!["estilo/cel/noir".into()], min_rating: Some(4), ..Default::default() })?;
    // pasta de importação com arquivos bons e quebrados (a UI de import mostra os erros reais do backend)
    let imp = work.join("para_importar");
    std::fs::create_dir_all(&imp).ok();
    if let Some((_, p)) = glbs.get(3) {
        let _ = std::fs::copy(p, imp.join("pilar_novo.glb"));
    }
    write_png(&imp.join("textura_nova.png"), 64, 64, |x, y| [(x * 4) as u8, 90, (y * 4) as u8]);
    if let Some((_, p)) = glbs.get(4) {
        if let Ok(bytes) = std::fs::read(p) {
            std::fs::write(imp.join("truncado.glb"), &bytes[..bytes.len() / 3]).ok();
        }
    }
    std::fs::write(imp.join("lixo.glb"), b"isto nao e um glb de verdade, so texto").ok();
    std::fs::write(imp.join("lixo.fbx"), b"Kaydara FBX Binary  \x00\x1a\x00 corrompido").ok();
    sc.import_dir = imp;
    Ok(sc)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
