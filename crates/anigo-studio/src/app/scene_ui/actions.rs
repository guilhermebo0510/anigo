//! Operações do Cenário sobre a seleção: cada uma monta o `Command` certo (planejadores do núcleo) e o
//! aplica como UM passo de desfazer.

use super::{PlaceKind, StudioApp};
use crate::i18n::tr;
use crate::scene::camview::{frame_box, CamView};
use crate::scene::gizmo::{apply_translate, NewLocal};
use crate::scene::placement::{drop_target, place_asset, row_offsets, DropTarget};
use crate::scene::snap::{snap_value, LocalBox};
use anigo_assets::AssetId;
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level::{self, NodePatch, PlanError, PrimitiveShape};
use anigo_core::{NodeSource, NodeTransformEntry, Transform};
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

/// Entradas `SetNodesTransform` a partir de novos locais calculados pelos solvers do gizmo.
pub(crate) fn entries_from(new: &[NewLocal], project: &anigo_core::ProjectState) -> Vec<NodeTransformEntry> {
    new.iter()
        .filter_map(|n| {
            let cur = project.scene.node(&n.id)?.transform;
            let e = NodeTransformEntry {
                node_id: n.id.clone(),
                translation: n.translation.filter(|t| *t != cur.translation).map(|t| t.to_array()),
                rotation: n.rotation.filter(|r| !r.abs_diff_eq(cur.rotation, 1e-7)).map(|r| r.to_array()),
                scale: n.scale.filter(|s| *s != cur.scale).map(|s| s.to_array()),
            };
            (e.translation.is_some() || e.rotation.is_some() || e.scale.is_some()).then_some(e)
        })
        .collect()
}

impl StudioApp {
    fn scene_plan(&mut self, plan: Result<Command, PlanError>) -> bool {
        match plan {
            Ok(cmd) => self.scene_apply(cmd),
            Err(PlanError::Nothing) => false,
            Err(PlanError::Refused(msg)) => {
                self.scene_say(msg);
                false
            }
        }
    }

    pub(crate) fn scene_delete(&mut self) {
        let ids = self.scene_editable_selection();
        if ids.is_empty() {
            return;
        }
        let plan = level::plan_delete(self.core.project(), &ids);
        if self.scene_plan(plan) {
            self.scene_ui.selection.clear();
        }
    }

    pub(crate) fn scene_duplicate(&mut self) {
        let ids = self.scene_editable_selection();
        let plan = level::plan_duplicate(self.core.project(), &ids, None);
        match plan {
            Ok((cmd, map)) => {
                if self.scene_apply(cmd) {
                    let roots = level::top_level(self.core.project(), &map.values().cloned().collect::<Vec<_>>());
                    self.scene_ui.selection.set_many(roots);
                }
            }
            Err(PlanError::Refused(m)) => self.scene_say(m),
            Err(PlanError::Nothing) => {}
        }
    }

    pub(crate) fn scene_group(&mut self) {
        let ids = self.scene_editable_selection();
        match level::plan_group(self.core.project(), &ids) {
            Ok((cmd, gid)) => {
                if self.scene_apply(cmd) {
                    self.scene_ui.selection.set_one(gid);
                }
            }
            Err(PlanError::Refused(m)) => self.scene_say(m),
            Err(PlanError::Nothing) => {}
        }
    }

    pub(crate) fn scene_ungroup(&mut self) {
        let groups: Vec<NodeId> = self
            .scene_editable_selection()
            .into_iter()
            .filter(|i| self.core.project().scene.node(i).is_some_and(|n| n.kind == anigo_core::hierarchy::NodeKind::Group))
            .collect();
        let mut commands = Vec::new();
        let mut state = self.core.project().clone();
        let mut children = Vec::new();
        for g in groups {
            children.extend(state.scene.children_of(Some(&g)).iter().map(|n| n.node_id.clone()));
            if let Ok(cmd) = level::plan_ungroup(&state, &g) {
                if cmd.apply(&mut state).is_ok() {
                    commands.push(cmd);
                }
            }
        }
        if commands.is_empty() {
            return;
        }
        let cmd = if commands.len() == 1 { commands.remove(0) } else { Command::Batch { commands } };
        if self.scene_apply(cmd) {
            self.scene_ui.selection.set_many(children);
        }
    }

    pub(crate) fn scene_set_hidden(&mut self, hide: bool) {
        let ids: Vec<NodeId> = self.scene_ui.selection.ids().to_vec();
        let plan = level::plan_set_visible(self.core.project(), &ids, !hide);
        if self.scene_plan(plan) && hide {
            self.scene_ui.selection.clear();
        }
    }

    pub(crate) fn scene_unhide_all(&mut self) {
        let all: Vec<NodeId> = self.core.project().scene.nodes.iter().map(|n| n.node_id.clone()).collect();
        let plan = level::plan_set_visible(self.core.project(), &all, true);
        self.scene_plan(plan);
    }

    pub(crate) fn scene_set_locked(&mut self, ids: &[NodeId], locked: bool) {
        let plan = level::plan_set_locked(self.core.project(), ids, locked);
        self.scene_plan(plan);
    }

    pub(crate) fn scene_toggle_visible(&mut self, id: &NodeId) {
        let visible = self.core.project().scene.node(id).is_none_or(|n| n.visible);
        self.scene_apply(Command::SetNodeVisibility { node_id: id.clone(), visible: !visible });
    }

    pub(crate) fn scene_rename(&mut self, id: &NodeId, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.scene_apply(Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { name: Some(name.to_string()), ..Default::default() } });
    }

    /// Foca (tecla F): enquadra a seleção ou, sem seleção, a cena inteira.
    pub(crate) fn scene_focus(&mut self) {
        let ids: Vec<NodeId> = self.scene_ui.selection.ids().to_vec();
        let b = if ids.is_empty() { self.scene_ui.runtime.bounds } else { self.scene_ui.runtime.bounds_of_many(&anigo_core::level::closure(self.core.project(), &ids)) };
        if let Some(b) = b {
            frame_box(&mut self.scene_ui.camera, b.min, b.max);
            self.scene_ui.camera_view = false;
        }
    }

    /// End: pousa a seleção no chão/superfície abaixo (menor deslocamento dos 5 pontos da base).
    pub(crate) fn scene_snap_to_floor(&mut self) {
        let ids = self.scene_editable_selection();
        if ids.is_empty() {
            return;
        }
        let Some(dy) = self.scene_ui.runtime.ground_snap(self.core.project(), &ids) else { return };
        if dy.abs() < 1e-5 {
            self.scene_say(tr("scene.msg.already_on_floor"));
            return;
        }
        let xf = self.scene_ui.runtime.xf_for(self.core.project(), &ids);
        let entries = entries_from(&apply_translate(&xf, Vec3::new(0.0, dy, 0.0)), self.core.project());
        if !entries.is_empty() {
            self.scene_apply(Command::SetNodesTransform { entries });
        }
    }

    /// Ctrl+End: leva a origem de cada nó para a grade de posição.
    pub(crate) fn scene_snap_origin_to_grid(&mut self) {
        let ids = self.scene_editable_selection();
        let step = self.scene_ui.snap.grid_m;
        let xf = self.scene_ui.runtime.xf_for(self.core.project(), &ids);
        let mut new = Vec::new();
        for n in &xf {
            let p = n.world_pos();
            let target = Vec3::new(snap_value(p.x, step), snap_value(p.y, step), snap_value(p.z, step));
            new.extend(apply_translate(std::slice::from_ref(n), target - p));
        }
        let entries = entries_from(&new, self.core.project());
        if !entries.is_empty() {
            self.scene_apply(Command::SetNodesTransform { entries });
        }
    }

    pub(crate) fn scene_copy(&mut self) {
        let ids: Vec<NodeId> = self.scene_ui.selection.ids().to_vec();
        match level::clipboard_from(self.core.project(), &ids) {
            Some(c) => {
                let n = c.nodes.len();
                self.scene_ui.clipboard = Some(c);
                self.scene_say(format!("{} {}", n, tr("scene.msg.copied")));
            }
            None => self.scene_say(tr("scene.msg.nothing_to_copy")),
        }
    }

    pub(crate) fn scene_cut(&mut self) {
        self.scene_copy();
        self.scene_delete();
    }

    pub(crate) fn scene_paste(&mut self) {
        let Some(clip) = self.scene_ui.clipboard.clone() else {
            self.scene_say(tr("scene.msg.clipboard_empty"));
            return;
        };
        match level::plan_paste(self.core.project(), &clip, [0.0, 0.0, 0.0]) {
            Ok((cmd, roots)) => {
                if self.scene_apply(cmd) {
                    self.scene_ui.selection.set_many(roots);
                }
            }
            Err(PlanError::Refused(m)) => self.scene_say(m),
            Err(PlanError::Nothing) => {}
        }
    }

    pub(crate) fn scene_select_all(&mut self) {
        let ids = crate::scene::selection::select_all(self.core.project());
        self.scene_ui.selection.set_many(ids);
    }

    pub(crate) fn scene_invert_selection(&mut self) {
        let ids = crate::scene::selection::invert(self.core.project(), &self.scene_ui.selection);
        self.scene_ui.selection.set_many(ids);
    }

    pub(crate) fn scene_select_children(&mut self) {
        let ids = crate::scene::selection::with_children(self.core.project(), &self.scene_ui.selection);
        self.scene_ui.selection.set_many(ids);
    }

    pub(crate) fn scene_select_same(&mut self) {
        let ids = crate::scene::selection::same_source(self.core.project(), &self.scene_ui.selection);
        self.scene_ui.selection.set_many(ids);
    }

    // ---------------------------------------------------------------- colocar

    /// Raio pelo centro da viewport (alvo dos botões de "Colocar atores" e do duplo clique no drawer).
    fn scene_center_target(&self) -> DropTarget {
        let su = &self.scene_ui;
        let view = CamView::from_orbit(&su.camera, su.view_size);
        let (o, d) = view.ray(su.view_size * 0.5);
        drop_target(&su.runtime.bvh, o, d, 0.0, su.camera.distance.clamp(3.0, 14.0), |_| false)
    }

    pub(crate) fn scene_place_actor(&mut self, kind: PlaceKind, at: Option<DropTarget>) {
        let target = at.unwrap_or_else(|| self.scene_center_target());
        let snap = self.scene_ui.snap;
        let project = self.core.project().clone();
        let (cmd, id) = match kind {
            PlaceKind::Primitive(shape) => {
                let geom = self.scene_ui.cache.primitive(shape);
                let t = place_asset(&target, &geom.local, 0.0, Vec3::ONE, &snap);
                level::plan_place_primitive(&project, shape, &tr(prim_label_key(shape)), t, &BTreeSet::new())
            }
            PlaceKind::Camera => {
                let eye = Vec3::new(target.point.x, target.point.y.max(0.0) + 1.5, target.point.z);
                // olha para o alvo atual da câmera de edição
                let look = self.scene_ui.camera.target;
                let dir = (look - eye).normalize_or_zero();
                let rot = if dir == Vec3::ZERO { Quat::IDENTITY } else { Quat::from_rotation_arc(Vec3::NEG_Z, dir) };
                level::plan_place_camera(&project, &tr("scene.actor.camera"), Transform { translation: eye, rotation: rot, scale: Vec3::ONE }, NodeSource::default_camera(), &BTreeSet::new())
            }
            PlaceKind::Group => {
                let t = Transform::from_translation(target.point);
                let id = level::unique_node_id(&project, &tr("scene.actor.group"), &BTreeSet::new());
                (
                    Command::AddNode {
                        node_id: id.clone(),
                        name: level::unique_name(&project, &tr("scene.actor.group")),
                        parent_id: None,
                        node_kind: anigo_core::hierarchy::NodeKind::Group,
                        transform: t,
                        mesh: None,
                        material_id: None,
                        index: project.scene.nodes.len() as u32,
                        attrs: None,
                    },
                    id,
                )
            }
        };
        if self.scene_apply(cmd) {
            self.scene_ui.selection.set_one(id);
        }
    }

    /// Coloca ativos da Biblioteca em `target` como UM passo de desfazer. `true` se algum foi criado.
    pub(crate) fn scene_place_library(&mut self, ids: &[AssetId], target: Option<DropTarget>) -> bool {
        let target = target.unwrap_or_else(|| self.scene_center_target());
        let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) else {
            self.scene_say(tr("scene.msg.library_not_ready"));
            return false;
        };
        let snap = self.scene_ui.snap;
        // resolve (e espera a carga de) cada ativo antes de decidir as posições
        let mut ready = Vec::new();
        let mut failures = Vec::new();
        for id in ids {
            let uri = anigo_core::library_uri(&id.to_string());
            match self.scene_ui.cache.request_library(&lib, &uri) {
                Err(e) => failures.push(e),
                Ok(r) => match self.scene_ui.cache.wait_for(&r.key, std::time::Duration::from_secs(6)) {
                    Some(g) => ready.push((uri, r, g)),
                    None => failures.push(format!("{}: {}", r.name, tr("scene.msg.load_failed"))),
                },
            }
        }
        if ready.is_empty() {
            self.scene_say(failures.first().cloned().unwrap_or_else(|| tr("scene.msg.nothing_placeable").to_string()));
            return false;
        }
        let widths: Vec<f32> = ready.iter().map(|(_, _, g)| (g.local.max.x - g.local.min.x).max(0.2)).collect();
        let offsets = row_offsets(&widths, 0.3);
        let project = self.core.project().clone();
        let mut taken = BTreeSet::new();
        let mut commands = Vec::new();
        let mut placed = Vec::new();
        for ((uri, r, g), dx) in ready.iter().zip(offsets) {
            let mut tgt = target;
            if ready.len() > 1 {
                tgt.point.x += dx;
            }
            let t = place_asset(&tgt, &g.local, 0.0, Vec3::ONE, &snap);
            let (mut cmds, nid) = level::plan_place_library_asset(&project, uri, r.revision, &r.name, t, None, Some(r.size), Some(r.content_hash.clone()), &taken);
            // o registro do ativo só entra uma vez no lote
            if commands.iter().any(|c: &Command| matches!(c, Command::PatchAssets { put, .. } if put.iter().any(|e| e.uri == *uri))) {
                cmds.retain(|c| !matches!(c, Command::PatchAssets { .. }));
            }
            taken.insert(nid.clone());
            placed.push(nid);
            commands.extend(cmds);
        }
        let cmd = if commands.len() == 1 { commands.remove(0) } else { Command::Batch { commands } };
        if self.scene_apply(cmd) {
            self.scene_ui.selection.set_many(placed);
            if !failures.is_empty() {
                self.scene_say(failures.join("; "));
            }
            true
        } else {
            false
        }
    }

    /// "Substituir selecionados por…": troca o ativo dos nós selecionados pelo ativo selecionado na Biblioteca.
    pub(crate) fn scene_replace_with_library_selection(&mut self) {
        let Some(id) = self.lib_ws.as_ref().and_then(|w| w.selected_asset_ids().into_iter().next()) else {
            self.scene_say(tr("scene.msg.select_asset_in_drawer"));
            return;
        };
        let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) else { return };
        let uri = anigo_core::library_uri(&id.to_string());
        let resolved = match self.scene_ui.cache.request_library(&lib, &uri) {
            Ok(r) => r,
            Err(e) => {
                self.scene_say(e);
                return;
            }
        };
        let targets: Vec<NodeId> = self.scene_editable_selection();
        if targets.is_empty() {
            return;
        }
        let mut commands = vec![Command::PatchAssets { put: vec![level::library_asset_entry(&uri, Some(resolved.size), Some(resolved.content_hash.clone()))], remove: vec![] }];
        commands.push(Command::ReplaceNodesAsset {
            entries: targets
                .iter()
                .map(|n| anigo_core::NodeAssetEntry {
                    node_id: n.clone(),
                    mesh: Some(anigo_core::MeshRef { asset_id: anigo_core::AssetId::for_uri(&uri), primitive_index: 0 }),
                    source: Some(NodeSource::LibraryAsset { uri: uri.clone(), revision: resolved.revision }),
                    name: Some(level::unique_name(self.core.project(), &resolved.name)),
                })
                .collect(),
        });
        // se o registro já está certo, o PatchAssets seria um no-op e recusaria o lote: tira-o
        if self.core.project().assets.get(&anigo_core::AssetId::for_uri(&uri)).is_some_and(|e| e.content_hash.as_deref() == Some(resolved.content_hash.as_str())) {
            commands.remove(0);
        }
        let cmd = if commands.len() == 1 { commands.remove(0) } else { Command::Batch { commands } };
        self.scene_apply(cmd);
    }

    /// Atualiza nós de ativos desatualizados para a revisão atual da Biblioteca.
    pub(crate) fn scene_update_stale(&mut self, ids: &[NodeId]) {
        let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) else { return };
        let mut entries = Vec::new();
        let mut puts = Vec::new();
        for id in ids {
            let Some(NodeSource::LibraryAsset { uri, revision }) = self.core.project().scene.node(id).and_then(|n| n.source.clone()) else { continue };
            let Ok(r) = self.scene_ui.cache.request_library(&lib, &uri) else { continue };
            if r.revision == revision {
                continue;
            }
            puts.push(level::library_asset_entry(&uri, Some(r.size), Some(r.content_hash.clone())));
            entries.push(anigo_core::NodeAssetEntry {
                node_id: id.clone(),
                mesh: Some(anigo_core::MeshRef { asset_id: anigo_core::AssetId::for_uri(&uri), primitive_index: 0 }),
                source: Some(NodeSource::LibraryAsset { uri, revision: r.revision }),
                name: None,
            });
        }
        if entries.is_empty() {
            return;
        }
        let cmd = Command::Batch { commands: vec![Command::PatchAssets { put: puts, remove: vec![] }, Command::ReplaceNodesAsset { entries }] };
        self.scene_apply(cmd);
    }


    /// Reparenta mantendo a pose mundial (arrastar no Outliner).
    pub(crate) fn scene_reparent(&mut self, ids: &[NodeId], parent: Option<&NodeId>) {
        let roots = level::top_level(self.core.project(), ids);
        let mut commands = Vec::new();
        let mut state = self.core.project().clone();
        for n in roots {
            if state.scene.node(&n).is_some_and(|s| s.locked) {
                continue;
            }
            match level::plan_reparent_keep_world(&state, &n, parent) {
                Ok(c) => {
                    if let Err(e) = c.apply(&mut state) {
                        self.scene_say(e.to_string());
                        return;
                    }
                    commands.push(c);
                }
                Err(PlanError::Refused(m)) => {
                    self.scene_say(m);
                    return;
                }
                Err(PlanError::Nothing) => {}
            }
        }
        if commands.is_empty() {
            return;
        }
        let cmd = if commands.len() == 1 { commands.remove(0) } else { Command::Batch { commands } };
        self.scene_apply(cmd);
    }

    /// Move nós para uma pasta do Outliner (`""` = tira da pasta).
    pub(crate) fn scene_move_to_folder(&mut self, ids: &[NodeId], folder: &str) {
        let roots = level::top_level(self.core.project(), ids);
        let commands: Vec<Command> = roots
            .into_iter()
            .filter(|n| self.core.project().scene.node(n).is_some_and(|s| s.folder.as_deref().unwrap_or("") != folder))
            .map(|n| Command::SetNodeProps { node_id: n, patch: NodePatch { folder: Some(folder.to_string()), ..Default::default() } })
            .collect();
        if commands.is_empty() {
            return;
        }
        let cmd = if commands.len() == 1 { commands.into_iter().next().unwrap_or(Command::Batch { commands: vec![] }) } else { Command::Batch { commands } };
        self.scene_apply(cmd);
    }

    /// Define a câmera de render do projeto a partir de um nó câmera de cena.
    pub(crate) fn scene_use_as_render_camera(&mut self, id: &NodeId) {
        match level::camera_command_from_node(self.core.project(), id) {
            Ok(c) => {
                self.scene_apply(c);
            }
            Err(e) => self.scene_say(e.to_string()),
        }
    }

    /// Leva o nó câmera de cena para a posição/direção da câmera de edição ("Move Object to Camera").
    pub(crate) fn scene_align_node_to_view(&mut self, id: &NodeId) {
        let cam = self.scene_ui.camera;
        let rot = Quat::from_rotation_arc(Vec3::NEG_Z, cam.forward());
        // preserva a inclinação lateral zero: usa look_to com Y global
        let m = glam::Mat4::look_to_rh(cam.eye(), cam.forward(), Vec3::Y).inverse();
        let (_, r, t) = m.to_scale_rotation_translation();
        let _ = rot;
        let parent_world = self.core.project().scene.node(id).and_then(|n| n.parent_id.clone()).map(|p| self.scene_ui.runtime.world_of(&p)).unwrap_or(glam::Mat4::IDENTITY);
        let local = parent_world.inverse() * glam::Mat4::from_rotation_translation(r, t);
        let (_, lr, lt) = local.to_scale_rotation_translation();
        let entry = NodeTransformEntry { node_id: id.clone(), translation: Some(lt.to_array()), rotation: Some(lr.normalize().to_array()), scale: None };
        self.scene_apply(Command::SetNodesTransform { entries: vec![entry] });
    }

    pub(crate) fn scene_set_view(&mut self, view: crate::scene::camview::SceneViewKind) {
        crate::scene::camview::set_view(&mut self.scene_ui.camera, view);
        self.scene_ui.view_kind = view;
        self.scene_ui.camera_view = false;
        if view != crate::scene::camview::SceneViewKind::Perspective {
            self.scene_focus_keep_direction();
        }
    }

    /// Aplica um tipo de vista (perspectiva/ortogonal/alinhadas) à câmera de edição atual. As alinhadas enquadram a cena
    /// mantendo a direção, como `scene_set_view`.
    pub(crate) fn scene_apply_view_kind(&mut self, kind: crate::app::multiview::ViewKind) {
        use crate::app::multiview::ViewKind as K;
        kind.apply(&mut self.scene_ui.camera);
        self.scene_ui.camera_view = false;
        self.scene_ui.view_kind = match kind {
            K::Front => crate::scene::camview::SceneViewKind::Front,
            K::Back => crate::scene::camview::SceneViewKind::Back,
            K::Right => crate::scene::camview::SceneViewKind::Right,
            K::Left => crate::scene::camview::SceneViewKind::Left,
            K::Top => crate::scene::camview::SceneViewKind::Top,
            K::Bottom => crate::scene::camview::SceneViewKind::Bottom,
            K::Perspective | K::Ortho => crate::scene::camview::SceneViewKind::Perspective,
        };
        if !matches!(kind, K::Perspective | K::Ortho) {
            self.scene_focus_keep_direction();
        }
    }

    fn scene_focus_keep_direction(&mut self) {
        if let Some(b) = self.scene_ui.runtime.bounds {
            frame_box(&mut self.scene_ui.camera, b.min, b.max);
        }
    }


    pub(crate) fn scene_cycle_tool(&mut self) {
        let t = self.scene_ui.tool.cycle();
        self.set_scene_tool(t);
    }

    pub(crate) fn scene_prim_shapes() -> [PrimitiveShape; 5] {
        PrimitiveShape::ALL
    }


}

pub(crate) fn prim_label_key(shape: PrimitiveShape) -> &'static str {
    match shape {
        PrimitiveShape::Cube => "scene.actor.cube",
        PrimitiveShape::Sphere => "scene.actor.sphere",
        PrimitiveShape::Plane => "scene.actor.plane",
        PrimitiveShape::Cylinder => "scene.actor.cylinder",
        PrimitiveShape::Cone => "scene.actor.cone",
    }
}

#[allow(dead_code)]
pub(crate) fn local_box_of(g: &crate::scene::geometry::LoadedGeometry) -> LocalBox {
    g.local
}
