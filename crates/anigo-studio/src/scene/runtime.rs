//! Estado derivado da cena para a viewport: matrizes mundiais, geometria de cada nó, BVH de instâncias,
//! proxies de seleção. É recalculado quando o projeto muda (nunca é fonte da verdade).

use super::bvh::{Aabb, Instance, SceneBvh};
use super::geometry::{LoadedGeometry, MeshCache, Slot};
use super::gizmo::NodeXf;
use super::selection::Proxy;
use anigo_assets::Library;
use anigo_core::hierarchy::NodeKind;
use anigo_core::ids::{AssetId, NodeId};
use anigo_core::project::{NodeSlot, ProjectState, URI_PRESET_CUBE, URI_PRESET_SPHERE};
use anigo_core::{NodeSource, PrimitiveShape};
use glam::{Mat4, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// O que a viewport sabe da geometria de um nó.
#[derive(Clone)]
pub enum GeomState {
    /// Nó sem malha (grupo, câmera de cena…).
    None,
    Ready(Arc<LoadedGeometry>),
    /// Carregando em segundo plano (ou Biblioteca ainda abrindo).
    Loading,
    /// Ativo ausente/ilegível: a viewport mostra um marcador.
    Missing(String),
}

/// Caixa do marcador de nós sem geometria / com ativo ausente (metros).
pub const MARKER_HALF: f32 = 0.18;
pub const MISSING_HALF: f32 = 0.3;

#[derive(Default)]
pub struct SceneRuntime {
    pub key: Option<(u64, u64, u64, u64, u64)>,
    pub world: BTreeMap<NodeId, Mat4>,
    pub geom: BTreeMap<NodeId, GeomState>,
    /// Nós efetivamente ocultos (próprios ou por ancestral).
    pub hidden: BTreeSet<NodeId>,
    pub bvh: SceneBvh,
    pub proxies: Vec<Proxy>,
    pub bounds: Option<Aabb>,
}

fn is_preset(slot: &NodeSlot, uri: &str) -> bool {
    slot.mesh.as_ref().is_some_and(|m| m.asset_id == AssetId::for_uri(uri))
}

impl SceneRuntime {
    /// Recalcula tudo a partir do projeto. `character` é a malha deformada do manequim (cacheada fora).
    pub fn rebuild(&mut self, project: &ProjectState, cache: &mut MeshCache, lib: Option<&Library>, character: Option<&Arc<LoadedGeometry>>) {
        self.world = project.scene.world_transforms().unwrap_or_default();
        self.geom.clear();
        self.hidden.clear();
        // visibilidade efetiva (pai antes de filho)
        let order = project.scene.hierarchy_order().unwrap_or_default();
        for &i in &order {
            let n = &project.scene.nodes[i];
            let parent_hidden = n.parent_id.as_ref().is_some_and(|p| self.hidden.contains(p));
            if !n.visible || parent_hidden {
                self.hidden.insert(n.node_id.clone());
            }
        }
        let mut instances = Vec::new();
        let mut proxies = Vec::new();
        let mut total: Option<Aabb> = None;
        for n in &project.scene.nodes {
            let state = self.geometry_of(n, cache, lib, character);
            self.geom.insert(n.node_id.clone(), state.clone());
            if self.hidden.contains(&n.node_id) {
                continue;
            }
            let world = self.world.get(&n.node_id).copied().unwrap_or(Mat4::IDENTITY);
            let origin = world.transform_point3(Vec3::ZERO);
            let mut grow = |b: Aabb| total = Some(total.map_or(b, |t| t.union(&b)));
            match state {
                GeomState::Ready(g) => {
                    if let Some(inst) = Instance::new(n.node_id.clone(), world, g.bvh.clone()) {
                        proxies.push(Proxy { id: n.node_id.clone(), bounds: inst.world_bounds, marker: false });
                        grow(inst.world_bounds);
                        instances.push(inst);
                    }
                }
                other => {
                    let half = if matches!(other, GeomState::Missing(_) | GeomState::Loading) { MISSING_HALF } else { MARKER_HALF };
                    let b = Aabb { min: origin - Vec3::splat(half), max: origin + Vec3::splat(half) };
                    proxies.push(Proxy { id: n.node_id.clone(), bounds: b, marker: true });
                    grow(b);
                }
            }
        }
        self.bvh = SceneBvh::new(instances);
        self.proxies = proxies;
        self.bounds = total;
    }

    fn geometry_of(&self, n: &NodeSlot, cache: &mut MeshCache, lib: Option<&Library>, character: Option<&Arc<LoadedGeometry>>) -> GeomState {
        if n.kind == NodeKind::CharacterRoot {
            return character.map_or(GeomState::Loading, |g| GeomState::Ready(g.clone()));
        }
        match &n.source {
            Some(NodeSource::Primitive { shape }) => GeomState::Ready(cache.primitive(*shape)),
            Some(NodeSource::LibraryAsset { uri, .. }) => {
                let Some(lib) = lib else { return GeomState::Loading };
                match cache.request_library(lib, uri) {
                    Err(e) => GeomState::Missing(e),
                    Ok(r) => match cache.slot(&r.key) {
                        Some(Slot::Ready(g)) => GeomState::Ready(g.clone()),
                        Some(Slot::Failed(e)) => GeomState::Missing(e.clone()),
                        _ => GeomState::Loading,
                    },
                }
            }
            Some(_) => GeomState::None,
            None => {
                if is_preset(n, URI_PRESET_CUBE) {
                    GeomState::Ready(cache.primitive(PrimitiveShape::Cube))
                } else if is_preset(n, URI_PRESET_SPHERE) {
                    GeomState::Ready(cache.primitive(PrimitiveShape::Sphere))
                } else {
                    GeomState::None
                }
            }
        }
    }

    pub fn world_of(&self, id: &NodeId) -> Mat4 {
        self.world.get(id).copied().unwrap_or(Mat4::IDENTITY)
    }

    pub fn position_of(&self, id: &NodeId) -> Vec3 {
        self.world_of(id).transform_point3(Vec3::ZERO)
    }

    /// Caixa mundial de um nó (malha ou marcador).
    pub fn bounds_of(&self, id: &NodeId) -> Option<Aabb> {
        self.proxies.iter().find(|p| &p.id == id).map(|p| p.bounds)
    }

    /// União das caixas dos nós (e dos descendentes).
    pub fn bounds_of_many(&self, ids: &[NodeId]) -> Option<Aabb> {
        let mut out: Option<Aabb> = None;
        for id in ids {
            if let Some(b) = self.bounds_of(id) {
                out = Some(out.map_or(b, |o| o.union(&b)));
            }
        }
        out
    }

    /// Estado inicial dos nós para um arrasto: só as raízes do conjunto (filhos acompanham o pai).
    pub fn xf_for(&self, project: &ProjectState, ids: &[NodeId]) -> Vec<NodeXf> {
        anigo_core::level::top_level(project, ids)
            .into_iter()
            .filter_map(|id| {
                let slot = project.scene.node(&id)?;
                if slot.locked {
                    return None;
                }
                let parent_world = slot.parent_id.as_ref().map_or(Mat4::IDENTITY, |p| self.world_of(p));
                Some(NodeXf { start_local: slot.transform, start_world: self.world_of(&id), parent_world, id })
            })
            .collect()
    }

    /// Raios do snap ao chão: desloca verticalmente a união das caixas de `ids` até pousar em algo
    /// (ignora os próprios `ids` e descendentes).
    pub fn ground_snap(&self, project: &ProjectState, ids: &[NodeId]) -> Option<f32> {
        let moving: BTreeSet<NodeId> = anigo_core::level::closure(project, ids).into_iter().collect();
        let b = self.bounds_of_many(ids)?;
        let pts = super::snap::base_points(b.min, b.max);
        Some(super::snap::ground_snap_delta(&pts, 0.0, |o, d| self.bvh.raycast(o, d, 1e6, |id| moving.contains(id)).map(|h| h.t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::Command;
    use anigo_core::level::{plan_place_primitive, NodePatch};
    use anigo_core::Transform;

    fn put(st: &mut ProjectState, shape: PrimitiveShape, at: Vec3, scale: f32) -> NodeId {
        let t = Transform { translation: at, scale: Vec3::splat(scale), ..Default::default() };
        let (cmd, id) = plan_place_primitive(st, shape, "Obj", t, &Default::default());
        cmd.apply(st).unwrap();
        id
    }

    fn rebuilt(st: &ProjectState) -> (SceneRuntime, MeshCache) {
        let mut cache = MeshCache::new();
        let mut rt = SceneRuntime::default();
        let ch = LoadedGeometry::new("char", anigo_core::Mesh::create_cube(1.8)).unwrap();
        rt.rebuild(st, &mut cache, None, Some(&ch));
        (rt, cache)
    }

    #[test]
    fn rebuild_resolves_geometry_world_matrices_and_proxies() {
        let mut st = ProjectState::default();
        let a = put(&mut st, PrimitiveShape::Cube, Vec3::new(2.0, 0.5, 0.0), 1.0);
        let (rt, _) = rebuilt(&st);
        assert!(matches!(rt.geom[&a], GeomState::Ready(_)));
        assert!(matches!(rt.geom[&NodeId::canonical_character()], GeomState::Ready(_)));
        assert_eq!(rt.bvh.instances.len(), 2);
        let b = rt.bounds_of(&a).unwrap();
        assert!((b.center() - Vec3::new(2.0, 0.5, 0.0)).length() < 1e-4);
        assert!(rt.bounds.is_some());
    }

    #[test]
    fn hidden_nodes_and_their_children_leave_the_bvh_but_keep_their_matrices() {
        let mut st = ProjectState::default();
        let p = put(&mut st, PrimitiveShape::Cube, Vec3::ZERO, 1.0);
        let c = put(&mut st, PrimitiveShape::Sphere, Vec3::new(1.0, 0.0, 0.0), 1.0);
        Command::SetNodeParent { node_id: c.clone(), parent_id: Some(p.clone()) }.apply(&mut st).unwrap();
        Command::SetNodeVisibility { node_id: p.clone(), visible: false }.apply(&mut st).unwrap();
        let (rt, _) = rebuilt(&st);
        assert!(rt.hidden.contains(&p) && rt.hidden.contains(&c));
        assert!(rt.bvh.instances.iter().all(|i| i.id != p && i.id != c));
        assert!(rt.world.contains_key(&c));
    }

    #[test]
    fn nodes_without_geometry_get_marker_proxies() {
        let mut st = ProjectState::default();
        let (cmd, cam) = anigo_core::level::plan_place_camera(&st, "Câmera", Transform::from_translation(Vec3::new(0.0, 1.5, 5.0)), NodeSource::default_camera(), &Default::default());
        cmd.apply(&mut st).unwrap();
        let (rt, _) = rebuilt(&st);
        let p = rt.proxies.iter().find(|p| p.id == cam).unwrap();
        assert!(p.marker);
        assert!(matches!(rt.geom[&cam], GeomState::None));
    }

    #[test]
    fn library_nodes_without_a_library_are_loading_not_crashing() {
        let mut st = ProjectState::default();
        let uri = anigo_core::library_uri("0190a1b2-0000-7000-8000-00000000beef");
        let (cmds, id) = anigo_core::level::plan_place_library_asset(&st, &uri, 1, "Mesa", Transform::default(), None, None, None, &Default::default());
        Command::Batch { commands: cmds }.apply(&mut st).unwrap();
        let (rt, _) = rebuilt(&st);
        assert!(matches!(rt.geom[&id], GeomState::Loading));
        assert!(rt.proxies.iter().any(|p| p.id == id && p.marker), "marcador clicável enquanto carrega");
    }

    #[test]
    fn ground_snap_lowers_a_floating_cube_onto_a_table_and_ignores_itself() {
        let mut st = ProjectState::default();
        // mesa: cubo de 1 m de lado a 1 m de altura de topo (centro y=0.5)
        let table = put(&mut st, PrimitiveShape::Cube, Vec3::new(5.0, 0.5, 5.0), 1.0);
        // cubo pequeno flutuando em cima da mesa (base em y=3)
        let top = put(&mut st, PrimitiveShape::Cube, Vec3::new(5.0, 3.25, 5.0), 0.5);
        let (rt, _) = rebuilt(&st);
        let dy = rt.ground_snap(&st, &[top.clone()]).unwrap();
        assert!((dy + 2.0).abs() < 0.01, "desce de y=3 até o topo da mesa (y=1): {dy}");
        // sozinho no ar sobre o piso: desce até y=0
        let lone = {
            let mut s2 = ProjectState::default();
            let id = put(&mut s2, PrimitiveShape::Cube, Vec3::new(-5.0, 4.5, 0.0), 1.0);
            let (rt2, _) = rebuilt(&s2);
            let d = rt2.ground_snap(&s2, &[id]).unwrap();
            d
        };
        assert!((lone + 4.0).abs() < 1e-3, "{lone}");
        let _ = (table, NodePatch::default());
    }

    #[test]
    fn xf_for_skips_locked_and_nested_nodes() {
        let mut st = ProjectState::default();
        let p = put(&mut st, PrimitiveShape::Cube, Vec3::ZERO, 1.0);
        let c = put(&mut st, PrimitiveShape::Cube, Vec3::X, 1.0);
        let l = put(&mut st, PrimitiveShape::Cube, Vec3::Z, 1.0);
        Command::SetNodeParent { node_id: c.clone(), parent_id: Some(p.clone()) }.apply(&mut st).unwrap();
        anigo_core::level::plan_set_locked(&st, &[l.clone()], true).unwrap().apply(&mut st).unwrap();
        let (rt, _) = rebuilt(&st);
        let xf = rt.xf_for(&st, &[p.clone(), c, l]);
        assert_eq!(xf.len(), 1, "só o pai se move; o filho acompanha; o bloqueado fica");
        assert_eq!(xf[0].id, p);
    }
}
