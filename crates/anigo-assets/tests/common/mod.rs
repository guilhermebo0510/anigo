//! Fixtures compartilhadas pelos testes de integracao (sem GPU, sem rede).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anigo_assets::*;
use serde_json::{json, Value};

/// Biblioteca em pasta temporaria.
pub struct Env {
    pub dir: tempfile::TempDir,
    pub lib: Library,
}

impl Env {
    pub fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().join("lib")).unwrap();
        Env { dir, lib }
    }
    /// Pasta de origem (fora da biblioteca).
    pub fn src(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join("src").join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    pub fn reopen(self) -> Env {
        let home = self.lib.home().to_path_buf();
        let dir = self.dir;
        drop(self.lib);
        let lib = Library::open(home).unwrap();
        Env { dir, lib }
    }
}

pub fn triangle_json(material: Option<Value>) -> (Value, Vec<u8>) {
    let mut bin: Vec<u8> = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.5]] {
        for c in v {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mat = material.unwrap_or_else(|| json!({"name": "Mat", "pbrMetallicRoughness": {"baseColorFactor": [0.5, 0.5, 0.5, 1.0]}}));
    let j = json!({
        "asset": {"version": "2.0", "generator": "test"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "tri"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [mat],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 2.0, 0.5]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
        "buffers": [{"byteLength": 42}]
    });
    (j, bin)
}

/// GLB valido; `seed` muda o conteudo (um extra no JSON) para gerar hashes distintos.
pub fn glb(seed: u32) -> Vec<u8> {
    let (mut j, bin) = triangle_json(None);
    j["asset"]["generator"] = json!(format!("test-{seed}"));
    anigo_vrm::gltf::build_glb(&j, &bin)
}

pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
    p
}

pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// PNG unico por `n`.
pub fn unique_png(n: u32) -> Vec<u8> {
    png(8 + (n % 5), 8, [(n % 251) as u8, ((n / 251) % 251) as u8, ((n * 7) % 251) as u8])
}

pub const CUBE_OBJ: &str = "mtllib cube.mtl\no Cube\nv -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nusemtl Red\nf 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

/// Importa e exige sucesso de um unico ativo.
pub fn import_one(lib: &Library, path: &Path, opts: ImportOptions) -> AssetId {
    let r = lib.import_blocking(path, opts).unwrap();
    assert!(r.failed.is_empty(), "falhas: {:?}", r.failed);
    assert_eq!(r.imported.len(), 1, "esperava 1 ativo: {r:?}");
    r.imported[0]
}

pub fn all_ids(lib: &Library) -> Vec<AssetId> {
    lib.query_ids(&Query { state: StateFilter::Any, ..Default::default() }, usize::MAX).unwrap()
}
