//! Seleção do Cenário (estado de UI, fora do projeto) e consultas de seleção: clique por raio,
//! caixa (marquee), inverter, filhos, mesmo ativo.

use super::bvh::{Aabb, SceneBvh};
use super::camview::CamView;
use anigo_core::ids::NodeId;
use anigo_core::project::ProjectState;
use glam::Vec2;
use std::collections::BTreeSet;

/// Conjunto ordenado de nós selecionados; `active` é o último clicado (pivô "ativo").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    ids: Vec<NodeId>,
    active: Option<NodeId>,
}

impl Selection {
    pub fn ids(&self) -> &[NodeId] {
        &self.ids
    }

    pub fn active(&self) -> Option<&NodeId> {
        self.active.as_ref()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &NodeId) -> bool {
        self.ids.contains(id)
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.active = None;
    }

    pub fn set_one(&mut self, id: NodeId) {
        self.ids = vec![id.clone()];
        self.active = Some(id);
    }

    pub fn set_many(&mut self, ids: Vec<NodeId>) {
        let mut seen = BTreeSet::new();
        self.ids = ids.into_iter().filter(|i| seen.insert(i.clone())).collect();
        self.active = self.ids.last().cloned();
    }

    pub fn add(&mut self, id: NodeId) {
        if !self.ids.contains(&id) {
            self.ids.push(id.clone());
        }
        self.active = Some(id);
    }

    pub fn remove(&mut self, id: &NodeId) {
        self.ids.retain(|i| i != id);
        if self.active.as_ref() == Some(id) {
            self.active = self.ids.last().cloned();
        }
    }

    /// Ctrl/Shift+clique: alterna o nó.
    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(&id) {
            self.remove(&id);
        } else {
            self.add(id);
        }
    }

    /// Remove o que não existe mais (depois de apagar/desfazer).
    pub fn retain_existing(&mut self, state: &ProjectState) {
        self.ids.retain(|i| state.scene.node(i).is_some());
        if self.active.as_ref().is_some_and(|a| state.scene.node(a).is_none()) {
            self.active = self.ids.last().cloned();
        }
    }
}

/// `true` se o nó pode ser selecionado na viewport (visível e não bloqueado).
pub fn is_pickable(state: &ProjectState, id: &NodeId) -> bool {
    state.scene.node(id).is_some_and(|n| n.visible && !n.locked && ancestors_visible(state, id))
}

fn ancestors_visible(state: &ProjectState, id: &NodeId) -> bool {
    anigo_core::hierarchy::ancestors_of(&state.scene.nodes, id)
        .iter()
        .all(|a| state.scene.node(a).is_none_or(|n| n.visible))
}

/// Todos os nós selecionáveis (Ctrl+A).
pub fn select_all(state: &ProjectState) -> Vec<NodeId> {
    state.scene.nodes.iter().filter(|n| is_pickable(state, &n.node_id)).map(|n| n.node_id.clone()).collect()
}

/// Inverte a seleção entre os selecionáveis.
pub fn invert(state: &ProjectState, current: &Selection) -> Vec<NodeId> {
    state
        .scene
        .nodes
        .iter()
        .filter(|n| is_pickable(state, &n.node_id) && !current.contains(&n.node_id))
        .map(|n| n.node_id.clone())
        .collect()
}

/// A seleção atual mais todos os descendentes.
pub fn with_children(state: &ProjectState, current: &Selection) -> Vec<NodeId> {
    anigo_core::level::closure(state, current.ids()).into_iter().filter(|i| is_pickable(state, i)).collect()
}

/// Nós com a mesma origem (mesmo ativo da Biblioteca / mesma primitiva) que algum selecionado (Shift+E).
pub fn same_source(state: &ProjectState, current: &Selection) -> Vec<NodeId> {
    let keys: BTreeSet<String> = current.ids().iter().filter_map(|i| state.scene.node(i)).filter_map(|n| source_key(n)).collect();
    if keys.is_empty() {
        return current.ids().to_vec();
    }
    state
        .scene
        .nodes
        .iter()
        .filter(|n| is_pickable(state, &n.node_id) && source_key(n).is_some_and(|k| keys.contains(&k)))
        .map(|n| n.node_id.clone())
        .collect()
}

fn source_key(n: &anigo_core::NodeSlot) -> Option<String> {
    use anigo_core::NodeSource;
    match n.source.as_ref()? {
        NodeSource::LibraryAsset { uri, .. } => Some(format!("lib:{uri}")),
        NodeSource::Primitive { shape } => Some(format!("prim:{}", shape.as_str())),
        _ => None,
    }
}

/// Proxy de seleção de um nó: caixa mundial (de malha ou, para nós sem geometria, uma caixa pequena).
#[derive(Debug, Clone)]
pub struct Proxy {
    pub id: NodeId,
    pub bounds: Aabb,
    /// `true` quando é só o marcador de um nó sem geometria (câmera, grupo).
    pub marker: bool,
}

/// Seleção por clique: raio contra a geometria + contra os marcadores dos nós sem malha.
pub fn pick(scene: &SceneBvh, proxies: &[Proxy], view: &CamView, mouse: Vec2, mut pickable: impl FnMut(&NodeId) -> bool) -> Option<NodeId> {
    let (o, d) = view.ray(mouse);
    let blocked: BTreeSet<NodeId> = scene.instances.iter().map(|i| i.id.clone()).filter(|id| !pickable(id)).collect();
    let mut best: Option<(f32, NodeId)> = scene.raycast(o, d, 1e9, |id| blocked.contains(id)).map(|h| (h.t, h.id));
    let inv = glam::Vec3::new(
        if d.x.abs() < 1e-12 { 1e12 } else { 1.0 / d.x },
        if d.y.abs() < 1e-12 { 1e12 } else { 1.0 / d.y },
        if d.z.abs() < 1e-12 { 1e12 } else { 1.0 / d.z },
    );
    for p in proxies.iter().filter(|p| p.marker) {
        if !pickable(&p.id) {
            continue;
        }
        if let Some(t) = p.bounds.ray_entry(o, inv, best.as_ref().map_or(1e9, |b| b.0)) {
            if best.as_ref().is_none_or(|b| t < b.0) {
                best = Some((t, p.id.clone()));
            }
        }
    }
    best.map(|b| b.1)
}

/// Retângulo de tela (mín, máx) de uma caixa mundial; `None` se qualquer canto fica atrás da câmera
/// E nenhum fica à frente.
pub fn project_box(b: &Aabb, view: &CamView) -> Option<(Vec2, Vec2)> {
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    let mut any = false;
    for c in b.corners() {
        if let Some((p, _)) = view.project(c) {
            lo = lo.min(p);
            hi = hi.max(p);
            any = true;
        }
    }
    any.then_some((lo, hi))
}

/// Nós cujo retângulo projetado intersecta o retângulo `[a, b]` (marquee).
pub fn marquee(proxies: &[Proxy], view: &CamView, a: Vec2, b: Vec2, mut pickable: impl FnMut(&NodeId) -> bool) -> Vec<NodeId> {
    let (lo, hi) = (a.min(b), a.max(b));
    proxies
        .iter()
        .filter(|p| pickable(&p.id))
        .filter(|p| {
            project_box(&p.bounds, view).is_some_and(|(plo, phi)| plo.x <= hi.x && phi.x >= lo.x && plo.y <= hi.y && phi.y >= lo.y)
        })
        .map(|p| p.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::bvh::{Instance, MeshBvh};
    use crate::viewport::OrbitCamera;
    use anigo_core::command::Command;
    use anigo_core::level::{plan_place_primitive, PrimitiveShape};
    use anigo_core::{Mesh, Transform};
    use glam::{Mat4, Vec3};
    use std::sync::Arc;

    fn id(s: &str) -> NodeId {
        NodeId::from_slug(s)
    }

    #[test]
    fn selection_set_toggle_and_active() {
        let mut s = Selection::default();
        s.set_one(id("a"));
        s.add(id("b"));
        assert_eq!(s.ids(), &[id("a"), id("b")]);
        assert_eq!(s.active(), Some(&id("b")));
        s.toggle(id("b"));
        assert_eq!(s.ids(), &[id("a")]);
        assert_eq!(s.active(), Some(&id("a")), "o ativo cai para o último restante");
        s.toggle(id("c"));
        s.set_many(vec![id("x"), id("y"), id("x")]);
        assert_eq!(s.len(), 2, "sem repetidos");
        s.clear();
        assert!(s.is_empty() && s.active().is_none());
    }

    fn scene_with(n: usize) -> (ProjectState, Vec<NodeId>) {
        let mut st = ProjectState::default();
        let mut ids = vec![];
        for i in 0..n {
            let (cmd, nid) = plan_place_primitive(&st, if i % 2 == 0 { PrimitiveShape::Cube } else { PrimitiveShape::Sphere }, "Obj", Transform::from_translation(Vec3::new(i as f32 * 2.0, 0.0, 0.0)), &Default::default());
            cmd.apply(&mut st).unwrap();
            ids.push(nid);
        }
        (st, ids)
    }

    #[test]
    fn select_all_skips_hidden_and_locked_and_invert_complements() {
        let (mut st, ids) = scene_with(4);
        Command::SetNodeVisibility { node_id: ids[0].clone(), visible: false }.apply(&mut st).unwrap();
        anigo_core::level::plan_set_locked(&st, &[ids[1].clone()], true).unwrap().apply(&mut st).unwrap();
        let all = select_all(&st);
        assert!(!all.contains(&ids[0]) && !all.contains(&ids[1]));
        assert!(all.contains(&ids[2]) && all.contains(&ids[3]));
        let mut sel = Selection::default();
        sel.set_one(ids[2].clone());
        let inv = invert(&st, &sel);
        assert!(!inv.contains(&ids[2]) && inv.contains(&ids[3]));
    }

    #[test]
    fn same_source_finds_every_instance_of_the_asset() {
        let (st, ids) = scene_with(5);
        let mut sel = Selection::default();
        sel.set_one(ids[0].clone());
        let same = same_source(&st, &sel);
        // cubos: índices 0, 2, 4
        assert_eq!(same.len(), 3);
        assert!(same.contains(&ids[2]) && same.contains(&ids[4]) && !same.contains(&ids[1]));
    }

    #[test]
    fn children_selection_includes_descendants() {
        let (mut st, ids) = scene_with(3);
        Command::SetNodeParent { node_id: ids[1].clone(), parent_id: Some(ids[0].clone()) }.apply(&mut st).unwrap();
        Command::SetNodeParent { node_id: ids[2].clone(), parent_id: Some(ids[1].clone()) }.apply(&mut st).unwrap();
        let mut sel = Selection::default();
        sel.set_one(ids[0].clone());
        let got = with_children(&st, &sel);
        assert_eq!(got.len(), 3);
    }

    fn view() -> CamView {
        let cam = OrbitCamera { target: Vec3::new(2.0, 0.5, 0.0), yaw: 0.0, pitch: 0.2, distance: 9.0, ..Default::default() };
        CamView::from_orbit(&cam, Vec2::new(900.0, 600.0))
    }

    #[test]
    fn click_pick_returns_the_front_object_and_respects_locks() {
        let bvh = Arc::new(MeshBvh::build(&Mesh::create_cube(1.0)));
        let a = Instance::new(id("a"), Mat4::from_translation(Vec3::new(2.0, 0.5, 2.0)), bvh.clone()).unwrap();
        let b = Instance::new(id("b"), Mat4::from_translation(Vec3::new(2.0, 0.5, -2.0)), bvh).unwrap();
        let scene = SceneBvh::new(vec![a, b]);
        let v = view();
        let (px, _) = v.project(Vec3::new(2.0, 0.5, 2.0)).unwrap();
        assert_eq!(pick(&scene, &[], &v, px, |_| true), Some(id("a")));
        // "a" bloqueado: o clique atravessa e pega o de trás (ou nada, se não alinhar)
        let got = pick(&scene, &[], &v, px, |i| i != &id("a"));
        assert_ne!(got, Some(id("a")));
        // clique no vazio
        assert_eq!(pick(&scene, &[], &v, Vec2::new(5.0, 5.0), |_| true), None);
    }

    #[test]
    fn marker_nodes_are_pickable_by_their_box() {
        let v = view();
        let proxy = Proxy { id: id("cam"), bounds: Aabb { min: Vec3::new(1.85, 0.35, -0.15), max: Vec3::new(2.15, 0.65, 0.15) }, marker: true };
        let (px, _) = v.project(Vec3::new(2.0, 0.5, 0.0)).unwrap();
        assert_eq!(pick(&SceneBvh::default(), &[proxy], &v, px, |_| true), Some(id("cam")));
    }

    #[test]
    fn marquee_selects_by_projected_box_intersection() {
        let v = view();
        let mk = |name: &str, x: f32| Proxy { id: id(name), bounds: Aabb { min: Vec3::new(x - 0.5, 0.0, -0.5), max: Vec3::new(x + 0.5, 1.0, 0.5) }, marker: false };
        let proxies = [mk("left", -2.0), mk("mid", 2.0), mk("right", 6.0)];
        let (pm, _) = v.project(Vec3::new(2.0, 0.5, 0.0)).unwrap();
        let hit = marquee(&proxies, &v, pm - Vec2::splat(20.0), pm + Vec2::splat(20.0), |_| true);
        assert_eq!(hit, vec![id("mid")]);
        let (pl, _) = v.project(Vec3::new(-2.0, 0.5, 0.0)).unwrap();
        let (pr, _) = v.project(Vec3::new(6.0, 0.5, 0.0)).unwrap();
        let all = marquee(&proxies, &v, pl - Vec2::splat(40.0), pr + Vec2::splat(40.0), |_| true);
        assert_eq!(all.len(), 3);
        // o retângulo pode ser desenhado em qualquer direção
        let rev = marquee(&proxies, &v, pm + Vec2::splat(20.0), pm - Vec2::splat(20.0), |_| true);
        assert_eq!(rev, vec![id("mid")]);
        assert!(marquee(&proxies, &v, pm - Vec2::splat(20.0), pm + Vec2::splat(20.0), |_| false).is_empty());
    }
}
