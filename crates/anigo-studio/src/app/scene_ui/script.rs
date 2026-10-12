//! Cenários de captura do Cenário (`--scenario cenario-*`): montam uma cena DE VERDADE (comandos do
//! núcleo, ativos reais da Biblioteca sintética gerada pelo studio) e levam a UI a um estado conhecido.
//! O screenshot só sai quando a cena assentou (carga de malhas, miniaturas, jobs).

use super::{SceneShading, StudioApp};
use crate::scene::gizmo::GizmoMode;
use crate::scene::placement::DropTarget;
use crate::scene::runtime::GeomState;
use anigo_assets::AssetId;
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level::{self, NodePatch, PrimitiveShape};
use anigo_core::{NodeSource, NodeTransformEntry, Transform};
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

#[allow(dead_code)]
pub const SCENE_SCENARIOS: &[&str] = &["cenario-vazio", "cenario-montado", "cenario-snap", "cenario-outliner", "cenario-gizmo", "cenario-camera"];

pub(crate) fn is_scene_scenario(name: &str) -> bool {
    name.starts_with("cenario-")
}

pub(crate) struct Script {
    pub name: String,
    stage: u32,
    stage_frame: u64,
    frame: u64,
    done: bool,
}

impl Script {
    pub fn is_done(&self) -> bool {
        self.done
    }
}

fn floor_target(x: f32, z: f32) -> DropTarget {
    DropTarget { point: Vec3::new(x, 0.0, z), normal: Vec3::Y, on_geometry: false }
}

impl StudioApp {
    pub(crate) fn scene_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Cenario;
        self.scene_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false });
    }

    fn set_node_transform(&mut self, id: &NodeId, t: Transform) {
        self.scene_apply(Command::SetNodesTransform { entries: vec![NodeTransformEntry::full(id.clone(), t)] });
    }

    fn add_primitive(&mut self, shape: PrimitiveShape, label: &str, t: Transform) -> NodeId {
        let (cmd, id) = level::plan_place_primitive(self.core.project(), shape, label, t, &BTreeSet::new());
        self.scene_apply(cmd);
        id
    }

    /// Monta a sala: piso, parede, pilares, mesa e adereços da Biblioteca, personagem e câmera.
    pub(crate) fn build_set(&mut self, full: bool) {
        // piso e parede de fundo (primitivas)
        let floor = self.add_primitive(PrimitiveShape::Plane, "Piso", Transform { scale: Vec3::new(18.0, 1.0, 12.0), ..Default::default() });
        self.scene_apply(Command::SetNodeProps { node_id: floor, patch: NodePatch { overrides: Some(anigo_core::NodeOverrides { base_color: Some([0.50, 0.58, 0.70, 1.0]), shade_color: Some([0.28, 0.32, 0.46, 1.0]), outline_width: None, ..Default::default() }), ..Default::default() } });
        self.add_primitive(PrimitiveShape::Cube, "Parede", Transform { translation: Vec3::new(0.0, 2.0, -5.0), scale: Vec3::new(14.0, 4.0, 0.3), ..Default::default() });
        // personagem ao centro-esquerda
        self.set_node_transform(&NodeId::canonical_character(), Transform { translation: Vec3::new(-1.5, 0.0, 1.5), rotation: Quat::from_rotation_y(0.5), scale: Vec3::ONE });
        let Some(assets) = self.scene_ui.script_assets.clone() else { return };
        let ids: Vec<AssetId> = assets.models.clone();
        // ativos reais da Biblioteca, colocados com a mesma rota do arrastar-e-soltar (snap de grade ativo)
        let plan: [(usize, f32, f32, f32); 6] = [(3, -4.0, -3.5, 1.0), (3, 4.0, -3.5, 1.0), (6, 2.0, 0.5, 1.6), (0, 4.5, 2.5, 1.0), (1, -4.5, 1.0, 1.0), (4, 0.0, -2.0, 1.0)];
        let take = if full { plan.len() } else { 3 };
        for (i, x, z, scale) in plan.into_iter().take(take) {
            if let Some(id) = ids.get(i) {
                if self.scene_place_library(&[*id], Some(floor_target(x, z))) {
                    if (scale - 1.0).abs() > 1e-3 {
                        if let Some(n) = self.scene_ui.selection.active().cloned() {
                            if let Some(t) = self.core.project().scene.node(&n).map(|s| s.transform) {
                                // a escala preserva a base no chão: sobe a origem na mesma proporção
                                let lift = t.translation.y * scale;
                                self.set_node_transform(&n, Transform { scale: Vec3::splat(scale), translation: Vec3::new(t.translation.x, lift, t.translation.z), ..t });
                            }
                        }
                    }
                }
            }
        }
        self.scene_ui.selection.clear();
        self.scene_ui.framed = false;
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn scene_script_tick(&mut self) {
        let Some(mut run) = self.scene_ui.script.take() else { return };
        if run.done {
            self.scene_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let ready = self.lib_ws.as_ref().is_some_and(|w| w.is_ready());
        let settled = self.lib_ws.as_ref().is_some_and(|w| w.is_settled());
        let waited = run.frame.saturating_sub(run.stage_frame);
        let mut next = false;
        match run.stage {
            0 => {
                if ready {
                    if let Some(w) = self.lib_ws.as_mut() {
                        w.prepare_scene_drawer();
                    }
                    self.scene_ui.drawer_open = true;
                    self.scene_ui.place_open = true;
                    self.core.set_history_limit(400);
                    next = true;
                }
            }
            1 => {
                if settled && waited > 3 {
                    next = true;
                }
            }
            2 => {
                let name = run.name.clone();
                match name.as_str() {
                    "cenario-vazio" => {
                        self.scene_ui.selection.clear();
                    }
                    "cenario-montado" => {
                        self.build_set(true);
                        self.scene_ui.selection.clear();
                    }
                    "cenario-snap" => {
                        self.build_set(true);
                        self.scene_ui.snap.surface_on = true;
                        self.scene_ui.snap.grid_m = 0.5;
                        self.scene_ui.snap.rot_deg = 15.0;
                        self.scene_ui.snap.scale_on = true;
                        self.scene_ui.tool = GizmoMode::Translate;
                        // seleciona um objeto e mostra a prévia (fantasma) de um ativo sobre a mesa, como no arrastar da Biblioteca
                        if let Some(n) = self.core.project().scene.nodes.iter().find(|n| matches!(n.source, Some(NodeSource::LibraryAsset { .. }))).map(|n| n.node_id.clone()) {
                            self.scene_ui.selection.set_one(n);
                        }
                    }
                    "cenario-outliner" => {
                        self.build_set(true);
                        let nodes: Vec<(NodeId, String)> = self.core.project().scene.nodes.iter().map(|n| (n.node_id.clone(), n.name.clone())).collect();
                        let pillars: Vec<NodeId> = nodes.iter().filter(|(_, n)| n.starts_with("Pilar")).map(|(i, _)| i.clone()).collect();
                        if pillars.len() >= 2 {
                            self.scene_ui.selection.set_many(pillars.clone());
                            self.scene_group();
                            if let Some(g) = self.scene_ui.selection.active().cloned() {
                                self.scene_apply(Command::SetNodeProps { node_id: g.clone(), patch: NodePatch { name: Some("Pilares".into()), folder: Some("Cenário/Estrutura".into()), ..Default::default() } });
                            }
                        }
                        if let Some((id, _)) = nodes.iter().find(|(_, n)| n.starts_with("Parede")) {
                            self.scene_apply(Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { folder: Some("Cenário/Estrutura".into()), tags: Some(vec!["estrutura".into(), "fundo".into()]), locked: Some(true), ..Default::default() } });
                        }
                        if let Some((id, _)) = nodes.iter().find(|(_, n)| n.starts_with("Piso")) {
                            self.scene_apply(Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { folder: Some("Cenário/Estrutura".into()), locked: Some(true), ..Default::default() } });
                        }
                        if let Some((id, _)) = nodes.iter().find(|(_, n)| n.starts_with("Orbe") || n.starts_with("Esfera")) {
                            self.scene_apply(Command::SetNodeVisibility { node_id: id.clone(), visible: false });
                        }
                        if let Some((id, _)) = nodes.iter().find(|(_, n)| n.starts_with("Caixa")) {
                            self.scene_ui.selection.set_one(id.clone());
                        }
                    }
                    "cenario-gizmo" => {
                        self.build_set(true);
                        self.scene_ui.tool = GizmoMode::Translate;
                        let pick = self.core.project().scene.nodes.iter().find(|n| n.name.starts_with("Caixa")).map(|n| n.node_id.clone());
                        if let Some(n) = pick {
                            self.scene_ui.selection.set_one(n);
                        }
                        self.scene_ui.camera.yaw = 0.7;
                        self.scene_ui.camera.pitch = 0.32;
                    }
                    "cenario-camera" => {
                        self.build_set(false);
                        let (cmd, cam) = level::plan_place_camera(self.core.project(), "Câmera principal", Transform { translation: Vec3::new(3.0, 1.6, 6.5), rotation: Quat::from_rotation_y(0.18), scale: Vec3::ONE }, NodeSource::Camera { focal_mm: 35.0, sensor_width_mm: 24.89, sensor_height_mm: 18.66 }, &BTreeSet::new());
                        self.scene_apply(cmd);
                        self.scene_use_as_render_camera(&cam);
                        self.scene_ui.selection.set_one(cam);
                        self.scene_ui.camera_view = true;
                        self.scene_ui.guides.thirds = true;
                        self.scene_ui.guides.safe = true;
                        self.scene_ui.guides.frame = true;
                    }
                    _ => {}
                }
                if name != "cenario-camera" {
                    self.scene_ui.shading = SceneShading::Cel;
                }
                next = true;
            }
            3 => {
                // espera as malhas carregarem e a UI assentar
                let loading = self.scene_ui.runtime.geom.values().any(|g| matches!(g, GeomState::Loading)) || self.scene_ui.cache.loading_count() > 0;
                if !loading && settled && waited > 6 {
                    // o fantasma do cenário de snap precisa da malha pronta: monta agora
                    if run.name == "cenario-snap" {
                        self.script_ghost();
                    }
                    next = true;
                }
            }
            _ => {
                if waited > 5 {
                    run.done = true;
                }
            }
        }
        if next {
            run.stage += 1;
            run.stage_frame = run.frame;
        }
        if run.frame > 3500 {
            run.done = true;
        }
        self.scene_ui.script = Some(run);
    }

    /// Fantasma de um ativo sobre a mesa (a mesma estrutura que o arrastar da Biblioteca produz).
    fn script_ghost(&mut self) {
        let Some(id) = self.scene_ui.script_assets.as_ref().and_then(|a| a.models.get(5).copied()) else { return };
        let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) else { return };
        let uri = anigo_core::library_uri(&id.to_string());
        let Ok(r) = self.scene_ui.cache.request_library(&lib, &uri) else { return };
        let Some(g) = self.scene_ui.cache.wait_for(&r.key, std::time::Duration::from_secs(5)) else { return };
        let snap = self.scene_ui.snap;
        // sobre a "caixa" (mesa) do cenário: topo a ~1.6 m
        let target = DropTarget { point: Vec3::new(2.0, 1.6, 0.5), normal: Vec3::Y, on_geometry: true };
        let t = crate::scene::placement::place_asset(&target, &g.local, 0.0, Vec3::ONE, &snap);
        self.scene_ui.ghost_pinned = true;
        self.scene_ui.ghost = Some(super::Ghost { items: vec![super::GhostItem { geom: Some(g), transform: t, name: r.name, loading: false, error: None }] });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_recognised_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for n in SCENE_SCENARIOS {
            assert!(is_scene_scenario(n) && seen.insert(*n));
        }
        assert!(!is_scene_scenario("biblioteca-grid"));
        assert!(SCENE_SCENARIOS.contains(&"cenario-montado"));
    }
}
