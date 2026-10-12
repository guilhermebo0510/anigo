//! Geometria dos nós do Cenário: primitivas geradas, ativos da Biblioteca carregados em segundo plano
//! (GLB/VRM/OBJ/FBX), cache por conteúdo (hash da revisão) e dados de desenho do Workbench.
//!
//! A ponte Biblioteca → cena é: `anigo://library/<uuid>` → `Library::row` + `resolve_files` →
//! `cel_thumb::load_core_mesh` (o mesmo leitor das miniaturas) → [`LoadedGeometry`] (malha + BVH +
//! caixa + arestas de destaque), compartilhada por todas as instâncias.

use super::bvh::MeshBvh;
use super::snap::LocalBox;
use crate::workbench::{LineInstance, MeshDrawData, SolidVertex};
use anigo_assets::{AssetId, AssetState, Library};
use anigo_core::level::PrimitiveShape;
use anigo_core::Mesh;
use crossbeam_channel::{unbounded, Receiver, Sender};
use glam::Vec3;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;

/// Cor-base do sólido do Workbench (cinza-azulado neutro).
pub const SOLID_GRAY: [f32; 4] = [0.74, 0.76, 0.80, 1.0];

/// Malha pronta para a cena.
pub struct LoadedGeometry {
    pub key: String,
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<MeshBvh>,
    pub local: LocalBox,
    pub tris: usize,
    /// Sólido + arestas de destaque para o Workbench (Sólido/Wire/overlay de profundidade).
    pub draw: Arc<MeshDrawData>,
}

impl LoadedGeometry {
    pub fn new(key: impl Into<String>, mesh: Mesh) -> Option<Arc<LoadedGeometry>> {
        if mesh.vertices.is_empty() || mesh.indices.len() < 3 {
            return None;
        }
        let bvh = Arc::new(MeshBvh::build(&mesh));
        if !bvh.bounds.is_valid() {
            return None;
        }
        let local = LocalBox { min: bvh.bounds.min, max: bvh.bounds.max };
        let draw = Arc::new(build_draw_data(&mesh));
        let tris = mesh.indices.len() / 3;
        Some(Arc::new(LoadedGeometry { key: key.into(), mesh: Arc::new(mesh), bvh, local, tris, draw }))
    }
}

/// Primitiva de 1 m (esfera/cilindro/cone de raio 0,5), centrada na origem.
pub fn primitive_mesh(shape: PrimitiveShape) -> Mesh {
    use anigo_modeling::{convert::to_core_mesh, primitives};
    let em = match shape {
        PrimitiveShape::Cube => primitives::cube(1.0),
        PrimitiveShape::Sphere => primitives::uv_sphere(0.5, 16, 28),
        PrimitiveShape::Plane => primitives::plane(1.0),
        PrimitiveShape::Cylinder => primitives::cylinder(0.5, 1.0, 32, 1, true),
        PrimitiveShape::Cone => primitives::cone(0.5, 0.0, 1.0, 32, true),
    };
    let mut mesh = to_core_mesh(&em, &Default::default()).unwrap_or_else(|_| Mesh::create_cube(1.0));
    mesh.name = shape.as_str().to_string();
    mesh
}

/// Dados de desenho do Workbench: triângulos com normal + arestas vivas (ângulo > 28° ou de borda).
pub fn build_draw_data(mesh: &Mesh) -> MeshDrawData {
    let pos = |i: u32| Vec3::from_array(mesh.vertices[i as usize].position);
    let mut solid = Vec::with_capacity(mesh.indices.len());
    for tri in mesh.indices.chunks_exact(3) {
        if tri.iter().any(|i| *i as usize >= mesh.vertices.len()) {
            continue;
        }
        let face_n = (pos(tri[1]) - pos(tri[0])).cross(pos(tri[2]) - pos(tri[0])).normalize_or_zero();
        for &i in tri {
            let v = &mesh.vertices[i as usize];
            let n = Vec3::from_array(v.normal);
            let n = if n.length_squared() > 1e-6 { n.normalize() } else { face_n };
            solid.push(SolidVertex { pos: v.position, normal: n.to_array(), color: SOLID_GRAY });
        }
    }
    MeshDrawData { solid, lines: feature_edges(mesh, 28f32.to_radians()), points: Vec::new() }
}

fn quant(p: Vec3) -> (i64, i64, i64) {
    let q = |v: f32| (v * 20_000.0).round() as i64;
    (q(p.x), q(p.y), q(p.z))
}

/// Arestas com ângulo diedral maior que `min_angle` (e as de borda), soldando vértices por posição.
fn feature_edges(mesh: &Mesh, min_angle: f32) -> Vec<LineInstance> {
    let mut welded: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut weld = |p: Vec3| -> u32 {
        let n = welded.len() as u32;
        *welded.entry(quant(p)).or_insert(n)
    };
    let mut edges: BTreeMap<(u32, u32), (Vec3, Vec3, Vec3, u32)> = BTreeMap::new(); // (a, b, normal da 1ª face, nº de faces)
    let mut second: BTreeMap<(u32, u32), Vec3> = BTreeMap::new();
    for tri in mesh.indices.chunks_exact(3) {
        if tri.iter().any(|i| *i as usize >= mesh.vertices.len()) {
            continue;
        }
        let p: Vec<Vec3> = tri.iter().map(|i| Vec3::from_array(mesh.vertices[*i as usize].position)).collect();
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n == Vec3::ZERO {
            continue;
        }
        let ids = [weld(p[0]), weld(p[1]), weld(p[2])];
        for k in 0..3 {
            let (a, b) = (ids[k], ids[(k + 1) % 3]);
            if a == b {
                continue;
            }
            let key = (a.min(b), a.max(b));
            match edges.get_mut(&key) {
                Some(e) => {
                    e.3 += 1;
                    second.entry(key).or_insert(n);
                }
                None => {
                    edges.insert(key, (p[k], p[(k + 1) % 3], n, 1));
                }
            }
        }
    }
    let cos_min = min_angle.cos();
    let color = [0.07, 0.07, 0.09, 0.9];
    edges
        .into_iter()
        .filter(|(key, (_, _, n1, count))| *count == 1 || *count > 2 || second.get(key).is_none_or(|n2| n1.dot(*n2) < cos_min))
        .map(|(_, (a, b, _, _))| LineInstance { a: a.to_array(), b: b.to_array(), color, width: 1.2 })
        .collect()
}

/// Estado de uma geometria no cache.
#[derive(Clone)]
pub enum Slot {
    Loading,
    Ready(Arc<LoadedGeometry>),
    Failed(String),
}

/// O que a Biblioteca diz sobre um ativo (resolvido uma vez por URI).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAsset {
    pub key: String,
    pub name: String,
    pub revision: u32,
    pub content_hash: String,
    pub size: u64,
    pub format: String,
}

type Loaded = (String, Result<Arc<LoadedGeometry>, String>);

/// Cache de geometrias do Cenário.
pub struct MeshCache {
    slots: HashMap<String, Slot>,
    resolved: HashMap<String, Result<ResolvedAsset, String>>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Sobe a cada geometria que termina de carregar (a UI recalcula o que depende dela).
    pub generation: u64,
    prims: HashMap<PrimitiveShape, Arc<LoadedGeometry>>,
}

impl Default for MeshCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshCache {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self { slots: HashMap::new(), resolved: HashMap::new(), tx, rx, wake: None, generation: 0, prims: HashMap::new() }
    }

    /// Função chamada pelas threads de carga ao terminar (acordar a UI).
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake = Some(wake);
    }

    pub fn primitive(&mut self, shape: PrimitiveShape) -> Arc<LoadedGeometry> {
        self.prims
            .entry(shape)
            .or_insert_with(|| {
                LoadedGeometry::new(format!("prim:{}", shape.as_str()), primitive_mesh(shape))
                    .unwrap_or_else(|| LoadedGeometry::new("prim:fallback", Mesh::create_cube(1.0)).expect("cubo válido"))
            })
            .clone()
    }

    /// Esquece o que foi resolvido na Biblioteca (um import novo pode ter criado uma revisão).
    pub fn forget_resolutions(&mut self) {
        self.resolved.clear();
    }

    /// Resolve `anigo://library/<uuid>` e dispara a carga da geometria (idempotente).
    pub fn request_library(&mut self, lib: &Library, uri: &str) -> Result<ResolvedAsset, String> {
        if let Some(r) = self.resolved.get(uri) {
            return r.clone();
        }
        let r = self.resolve_and_load(lib, uri);
        self.resolved.insert(uri.to_string(), r.clone());
        r
    }

    fn resolve_and_load(&mut self, lib: &Library, uri: &str) -> Result<ResolvedAsset, String> {
        let uuid = anigo_core::library_uuid(uri).ok_or_else(|| format!("URI de ativo inválida: {uri}"))?;
        let id = AssetId::from_str(uuid).map_err(|e| e.to_string())?;
        let row = lib.row(&id).map_err(|e| e.to_string())?.ok_or_else(|| "ativo não está na Biblioteca".to_string())?;
        if row.state != AssetState::Active {
            return Err("ativo na lixeira ou ausente".to_string());
        }
        if !crate::library_ui::cel_thumb::is_3d_format(&row.format) {
            return Err(format!("formato {} não é um modelo 3D", row.format));
        }
        let hash = row.content_hash.to_hex();
        let key = format!("lib:{uuid}@{}", &hash[..12.min(hash.len())]);
        let resolved = ResolvedAsset { key: key.clone(), name: row.name.clone(), revision: row.revision, content_hash: hash, size: row.size, format: row.format.clone() };
        if !self.slots.contains_key(&key) {
            let files = lib.resolve_files(&id).map_err(|e| e.to_string())?;
            if !files.exists {
                self.slots.insert(key.clone(), Slot::Failed("arquivo do ativo ausente".into()));
                return Ok(resolved);
            }
            self.slots.insert(key.clone(), Slot::Loading);
            let (tx, wake, k, format, path) = (self.tx.clone(), self.wake.clone(), key.clone(), row.format.clone(), files.primary);
            std::thread::Builder::new()
                .name("anigo-scene-load".into())
                .spawn(move || {
                    let r = crate::library_ui::cel_thumb::load_core_mesh(&path, &format).and_then(|m| LoadedGeometry::new(k.clone(), m).ok_or_else(|| "malha vazia".to_string()));
                    let _ = tx.send((k, r));
                    if let Some(w) = wake {
                        w();
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(resolved)
    }

    /// Estado de uma chave de geometria.
    pub fn slot(&self, key: &str) -> Option<&Slot> {
        self.slots.get(key)
    }

    pub fn ready(&self, key: &str) -> Option<Arc<LoadedGeometry>> {
        match self.slots.get(key) {
            Some(Slot::Ready(g)) => Some(g.clone()),
            _ => None,
        }
    }

    /// Resolução já feita de um URI (sem tocar na Biblioteca).
    pub fn resolved(&self, uri: &str) -> Option<&Result<ResolvedAsset, String>> {
        self.resolved.get(uri)
    }

    /// Recolhe as cargas que terminaram. `true` se algo mudou.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for (key, r) in self.rx.try_iter().collect::<Vec<_>>() {
            self.slots.insert(key, match r {
                Ok(g) => Slot::Ready(g),
                Err(e) => Slot::Failed(e),
            });
            self.generation += 1;
            changed = true;
        }
        changed
    }

    /// Espera (bloqueando a UI, no máximo `timeout`) a geometria `key` ficar pronta.
    pub fn wait_for(&mut self, key: &str, timeout: std::time::Duration) -> Option<Arc<LoadedGeometry>> {
        let start = std::time::Instant::now();
        loop {
            self.poll();
            match self.slots.get(key) {
                Some(Slot::Ready(g)) => return Some(g.clone()),
                Some(Slot::Failed(_)) | None => return None,
                Some(Slot::Loading) => {}
            }
            if start.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    pub fn loading_count(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Loading)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_primitive_has_valid_unit_geometry() {
        let mut cache = MeshCache::new();
        for shape in PrimitiveShape::ALL {
            let g = cache.primitive(shape);
            assert!(g.tris >= 2, "{shape:?}");
            let size = g.local.max - g.local.min;
            match shape {
                PrimitiveShape::Plane => assert!(size.y < 1e-4 && size.x > 0.99, "{size:?}"),
                _ => assert!(size.min_element() > 0.9 && size.max_element() < 1.1, "{shape:?} {size:?}"),
            }
            assert!(!g.draw.solid.is_empty());
            // primitivas são cacheadas (mesmo Arc)
            assert!(Arc::ptr_eq(&g, &cache.primitive(shape)));
        }
    }

    #[test]
    fn feature_edges_of_a_cube_are_its_twelve_edges() {
        let cube = primitive_mesh(PrimitiveShape::Cube);
        let lines = feature_edges(&cube, 28f32.to_radians());
        assert_eq!(lines.len(), 12, "{}", lines.len());
        // esfera lisa: quase nenhuma aresta viva
        let sphere = primitive_mesh(PrimitiveShape::Sphere);
        let n = feature_edges(&sphere, 28f32.to_radians()).len();
        assert!(n < 80, "{n}");
        // plano: 4 arestas de borda
        assert_eq!(feature_edges(&primitive_mesh(PrimitiveShape::Plane), 0.5).len(), 4);
    }

    #[test]
    fn loaded_geometry_rejects_empty_meshes() {
        assert!(LoadedGeometry::new("x", Mesh::new("x", vec![], vec![])).is_none());
    }

    #[test]
    fn missing_library_assets_fail_with_a_message_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("anigo_scene_geo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).expect("biblioteca");
        let mut cache = MeshCache::new();
        let uri = anigo_core::library_uri("0190a1b2-0000-7000-8000-000000000042");
        let err = cache.request_library(&lib, &uri).unwrap_err();
        assert!(!err.is_empty());
        assert!(cache.request_library(&lib, "anigo://base/x").is_err());
        assert!(cache.request_library(&lib, &anigo_core::library_uri("não-é-uuid")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
