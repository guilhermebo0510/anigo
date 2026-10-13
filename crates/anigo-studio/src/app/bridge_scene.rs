//! Cenário pela bridge (`SCENE`): o workspace sem janela — seleção de nós, ferramenta do gizmo e espaço, snap, vista (sombreamento,
//! guias, câmera de render, vistas ortogonais), enquadrar, visibilidade/trava, Outliner (renomear, apagar, duplicar, agrupar,
//! reparentar, pastas, copiar/colar), colocar atores, transformar nós, câmeras de cena, salto no histórico e o projeto `.anigo`
//! (novo/salvar/abrir). Usa as MESMAS funções dos botões (`scene_*`); edições são passos do núcleo (`anigo_native_undo`).

use super::StudioApp;
use crate::scene::camview::SceneViewKind;
use crate::scene::gizmo::{GizmoMode, GizmoSpace};
use crate::scene::placement::DropTarget;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level::NodeTransformEntry;
use anigo_core::PrimitiveShape;
use glam::{EulerRot, Quat, Vec3};
use serde_json::{json, Map, Value};

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn bool_of(v: &Value, name: &str) -> Result<bool, BridgeError> {
    v.as_bool().ok_or_else(|| bad(format!("'{name}' deve ser booleano")))
}

fn num_in(v: &Value, name: &str, lo: f32, hi: f32) -> Result<f32, BridgeError> {
    let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(format!("'{name}' deve ser um número finito")))? as f32;
    if f < lo || f > hi {
        return Err(bad(format!("'{name}' fora de {lo}..{hi}: {f}")));
    }
    Ok(f)
}

fn vec3(v: &Value, name: &str, lim: f32) -> Result<Vec3, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| bad(format!("'{name}' deve ter 3 números")))?;
    Ok(Vec3::new(num_in(&a[0], name, -lim, lim)?, num_in(&a[1], name, -lim, lim)?, num_in(&a[2], name, -lim, lim)?))
}

fn tool_name(t: GizmoMode) -> &'static str {
    match t {
        GizmoMode::Select => "select",
        GizmoMode::Translate => "translate",
        GizmoMode::Rotate => "rotate",
        GizmoMode::Scale => "scale",
    }
}

fn view_kind_of(s: &str) -> Result<SceneViewKind, BridgeError> {
    Ok(match s {
        "perspective" => SceneViewKind::Perspective,
        "front" => SceneViewKind::Front,
        "back" => SceneViewKind::Back,
        "right" => SceneViewKind::Right,
        "left" => SceneViewKind::Left,
        "top" => SceneViewKind::Top,
        "bottom" => SceneViewKind::Bottom,
        o => return Err(bad(format!("view_kind '{o}' desconhecido (perspective, front, back, right, left, top, bottom)"))),
    })
}

impl StudioApp {
    fn scene_node_id(&self, v: &Value) -> Result<NodeId, BridgeError> {
        let s = v.as_str().ok_or_else(|| bad("id de nó deve ser texto (veja info.nodes)"))?;
        self.core.project().scene.nodes.iter().map(|n| n.node_id.clone()).find(|i| i.to_string() == s).ok_or_else(|| bad(format!("nó '{s}' não existe na cena (veja info.nodes)")))
    }

    fn scene_node_ids(&self, v: &Value) -> Result<Vec<NodeId>, BridgeError> {
        let a = v.as_array().filter(|a| a.len() <= 2000).ok_or_else(|| bad("'ids' deve ser uma lista de ids de nó"))?;
        a.iter().map(|x| self.scene_node_id(x)).collect()
    }

    /// `ids` do pedido, ou a seleção atual.
    fn scene_target_ids(&self, m: &Map<String, Value>) -> Result<Vec<NodeId>, BridgeError> {
        let ids = match m.get("ids").filter(|v| !v.is_null()) {
            Some(v) => self.scene_node_ids(v)?,
            None => self.scene_ui.selection.ids().to_vec(),
        };
        if ids.is_empty() {
            return Err(bad("informe 'ids' (ou selecione nós)"));
        }
        Ok(ids)
    }

    pub(crate) fn bridge_scene(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if op == "info" {
            self.scene_refresh();
            return Ok(self.scene_info(m));
        }
        self.bridge_reset_gestures();
        if self.ws != crate::workspace::WorkspaceId::Cenario {
            self.switch_workspace(crate::workspace::WorkspaceId::Cenario);
        }
        self.scene_refresh();
        let depth = self.core.history().undo_depth();
        let nodes0 = self.core.project().scene.nodes.len();
        self.scene_ui.message.clear();
        match op {
            "select" => self.scene_select_op(m)?,
            "tool" => self.scene_tool_op(m)?,
            "snap" => self.scene_snap_op(m)?,
            "view" => self.scene_view_op(m)?,
            "visibility" => self.scene_visibility_op(m)?,
            "outliner" => self.scene_outliner_op(m)?,
            "place" => self.scene_place_op(m)?,
            "transform" => self.scene_transform_op(m)?,
            "camera" => self.scene_camera_op(m)?,
            "history" => {
                let d = m.get("depth").and_then(Value::as_u64).filter(|d| *d <= 100_000).ok_or_else(|| bad("'depth' é obrigatório (0 = início; veja info.history.undo_depth)"))? as usize;
                self.core.jump_to_depth(d).map_err(BridgeError::failed)?;
            }
            "project" => self.scene_project_op(m)?,
            other => return Err(bad(format!("SCENE op '{other}' desconhecida (info, select, tool, snap, view, visibility, outliner, place, transform, camera, history, project)"))),
        }
        self.scene_refresh();
        let mut out = self.scene_info(&Map::new());
        out["history_steps_added"] = json!(self.core.history().undo_depth() as i64 - depth as i64);
        out["nodes_added"] = json!(self.core.project().scene.nodes.len() as i64 - nodes0 as i64);
        out["message"] = json!(self.scene_ui.message);
        Ok(out)
    }

    fn scene_info(&self, m: &Map<String, Value>) -> Value {
        let limit = m.get("limit").and_then(Value::as_u64).map(|n| (n as usize).min(2000)).unwrap_or(300);
        let p = self.core.project();
        let su = &self.scene_ui;
        let nodes: Vec<Value> = p
            .scene
            .nodes
            .iter()
            .take(limit)
            .map(|n| {
                let (rx, ry, rz) = n.transform.rotation.to_euler(EulerRot::XYZ);
                json!({
                    "id": n.node_id.to_string(), "name": n.name, "kind": format!("{:?}", n.kind), "parent": n.parent_id.as_ref().map(|i| i.to_string()),
                    "visible": n.visible, "locked": n.locked, "folder": n.folder, "tags": n.tags, "selected": su.selection.contains(&n.node_id),
                    "translation": n.transform.translation.to_array(), "rotation_deg": [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()], "scale": n.transform.scale.to_array(),
                    "bounds": su.runtime.bounds_of(&n.node_id).map(|b| json!([b.min.to_array(), b.max.to_array()])),
                })
            })
            .collect();
        let s = &su.snap;
        json!({
            "workspace": self.ws.key(), "project": p.name, "node_count": p.scene.nodes.len(), "nodes": nodes, "nodes_truncated": p.scene.nodes.len() > limit,
            "selection": { "ids": su.selection.ids().iter().map(|i| i.to_string()).collect::<Vec<_>>(), "active": su.selection.active().map(|i| i.to_string()) },
            "tool": tool_name(su.tool), "space": match su.space { GizmoSpace::World => "world", GizmoSpace::Local => "local" },
            "snap": { "grid_on": s.grid_on, "grid_m": s.grid_m, "rot_on": s.rot_on, "rot_deg": s.rot_deg, "scale_on": s.scale_on, "scale_step": s.scale_step, "surface_on": s.surface_on, "align_to_normal": s.align_to_normal, "surface_offset": s.surface_offset, "absolute": s.absolute },
            "scale_lock": su.scale_lock,
            "view": { "shading": format!("{:?}", su.shading).to_lowercase(), "game_view": su.game_view, "camera_view": su.camera_view, "speed": su.speed, "view_kind": format!("{:?}", su.view_kind).to_lowercase(),
                      "guides": { "thirds": su.guides.thirds, "center": su.guides.center, "safe": su.guides.safe, "frame": su.guides.frame },
                      "camera": { "target": su.camera.target.to_array(), "yaw": su.camera.yaw, "pitch": su.camera.pitch, "distance": su.camera.distance } },
            "clipboard": su.clipboard.is_some(), "last_path": su.last_path,
            "history": { "undo_depth": self.core.history().undo_depth(), "redo_depth": self.core.history().redo_depth() },
            "scene_cameras": p.scene.nodes.iter().filter(|n| matches!(n.source, Some(anigo_core::NodeSource::Camera { .. }))).map(|n| json!({ "id": n.node_id.to_string(), "name": n.name })).collect::<Vec<_>>(),
            "message": su.message,
        })
    }

    // ------------------------------------------------------------------------------------------------ seleção, ferramenta, snap

    fn scene_select_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        if m.get("none").and_then(Value::as_bool).unwrap_or(false) {
            self.scene_ui.selection.clear();
            return Ok(());
        }
        if let Some(what) = m.get("what").and_then(Value::as_str) {
            match what {
                "all" => self.scene_select_all(),
                "invert" => self.scene_invert_selection(),
                "children" => self.scene_select_children(),
                "same" => self.scene_select_same(),
                o => return Err(bad(format!("what '{o}' desconhecido (all, invert, children, same)"))),
            }
            return Ok(());
        }
        let ids = self.scene_node_ids(m.get("ids").ok_or_else(|| bad("informe 'ids', 'what' ou 'none'"))?)?;
        let p = self.core.project();
        for id in &ids {
            if !crate::scene::selection::is_pickable(p, id) {
                return Err(bad(format!("o nó {id} está oculto ou travado (não pode ser selecionado)")));
            }
        }
        match m.get("mode").and_then(Value::as_str).unwrap_or("set") {
            "set" => self.scene_ui.selection.set_many(ids),
            "add" => ids.into_iter().for_each(|i| self.scene_ui.selection.add(i)),
            "remove" => ids.iter().for_each(|i| self.scene_ui.selection.remove(i)),
            "toggle" => ids.into_iter().for_each(|i| self.scene_ui.selection.toggle(i)),
            o => return Err(bad(format!("mode '{o}' desconhecido (set, add, remove, toggle)"))),
        }
        Ok(())
    }

    fn scene_tool_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        if let Some(t) = m.get("tool") {
            let t = match t.as_str() {
                Some("select") => GizmoMode::Select,
                Some("translate") => GizmoMode::Translate,
                Some("rotate") => GizmoMode::Rotate,
                Some("scale") => GizmoMode::Scale,
                _ => return Err(bad("'tool' deve ser select|translate|rotate|scale")),
            };
            self.set_scene_tool(t);
        }
        if let Some(s) = m.get("space") {
            self.scene_ui.space = match s.as_str() {
                Some("world") => GizmoSpace::World,
                Some("local") => GizmoSpace::Local,
                _ => return Err(bad("'space' deve ser world|local")),
            };
        }
        if m.get("tool").is_none() && m.get("space").is_none() {
            return Err(bad("informe 'tool' e/ou 'space'"));
        }
        Ok(())
    }

    fn scene_snap_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (grid_on, grid_m, rot_on, rot_deg, scale_on, scale_step, surface_on, align_to_normal, surface_offset, absolute, scale_lock)"))?;
        let mut s = self.scene_ui.snap;
        let mut lock = self.scene_ui.scale_lock;
        for (k, v) in patch {
            match k.as_str() {
                "grid_on" => s.grid_on = bool_of(v, k)?,
                "rot_on" => s.rot_on = bool_of(v, k)?,
                "scale_on" => s.scale_on = bool_of(v, k)?,
                "surface_on" => s.surface_on = bool_of(v, k)?,
                "align_to_normal" => s.align_to_normal = bool_of(v, k)?,
                "absolute" => s.absolute = bool_of(v, k)?,
                "scale_lock" => lock = bool_of(v, k)?,
                "grid_m" => s.grid_m = num_in(v, k, 0.001, 100.0)?,
                "rot_deg" => s.rot_deg = num_in(v, k, 0.1, 180.0)?,
                "scale_step" => s.scale_step = num_in(v, k, 0.001, 10.0)?,
                "surface_offset" => s.surface_offset = num_in(v, k, 0.0, 2.0)?,
                other => return Err(bad(format!("campo desconhecido 'patch.{other}'"))),
            }
        }
        self.scene_ui.snap = s;
        self.scene_ui.scale_lock = lock;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ vista

    fn scene_view_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        use super::scene_ui::SceneShading;
        let patch = m.get("patch").and_then(Value::as_object);
        if let Some(patch) = patch {
            let su = &self.scene_ui;
            let (mut shading, mut game, mut camv, mut guides, mut speed) = (su.shading, su.game_view, su.camera_view, su.guides, su.speed);
            let mut kind: Option<SceneViewKind> = None;
            for (k, v) in patch {
                match k.as_str() {
                    "shading" => {
                        shading = match v.as_str() {
                            Some("cel") => SceneShading::Cel,
                            Some("solid") => SceneShading::Solid,
                            Some("wire") => SceneShading::Wire,
                            _ => return Err(bad("'shading' deve ser cel|solid|wire")),
                        }
                    }
                    "game_view" => game = bool_of(v, k)?,
                    "camera_view" => camv = bool_of(v, k)?,
                    "speed" => speed = num_in(v, k, 0.1, 16.0)?,
                    "view_kind" => kind = Some(view_kind_of(v.as_str().ok_or_else(|| bad("'view_kind' deve ser texto"))?)?),
                    "guides" => {
                        let o = v.as_object().ok_or_else(|| bad("'guides' deve ser {thirds, center, safe, frame}"))?;
                        for (gk, gv) in o {
                            match gk.as_str() {
                                "thirds" => guides.thirds = bool_of(gv, gk)?,
                                "center" => guides.center = bool_of(gv, gk)?,
                                "safe" => guides.safe = bool_of(gv, gk)?,
                                "frame" => guides.frame = bool_of(gv, gk)?,
                                o => return Err(bad(format!("guides.{o} desconhecido (thirds, center, safe, frame)"))),
                            }
                        }
                    }
                    other => return Err(bad(format!("campo desconhecido 'patch.{other}' (shading, game_view, camera_view, speed, view_kind, guides)"))),
                }
            }
            let su = &mut self.scene_ui;
            (su.shading, su.game_view, su.camera_view, su.guides, su.speed) = (shading, game, camv, guides, speed);
            if let Some(k) = kind {
                self.scene_set_view(k);
            }
        }
        if let Some(f) = m.get("frame").and_then(Value::as_str) {
            match f {
                "all" => {
                    self.scene_ui.selection.clear();
                    self.scene_focus();
                }
                "selection" => {
                    if self.scene_ui.selection.ids().is_empty() {
                        return Err(BridgeError::failed("não há seleção para enquadrar"));
                    }
                    self.scene_focus();
                }
                o => return Err(bad(format!("frame '{o}' desconhecido (all, selection)"))),
            }
        } else if patch.is_none() {
            return Err(bad("informe 'patch' e/ou 'frame'"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ edição

    fn scene_visibility_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        if action == "unhide_all" {
            self.scene_unhide_all();
            return Ok(());
        }
        let ids = self.scene_target_ids(m)?;
        match action {
            "hide" | "show" => {
                let plan = anigo_core::level::plan_set_visible(self.core.project(), &ids, action == "show");
                match plan {
                    Ok(cmd) => {
                        self.scene_apply(cmd);
                        if action == "hide" {
                            for i in &ids {
                                self.scene_ui.selection.remove(i);
                            }
                        }
                    }
                    Err(e) => return Err(BridgeError::failed(format!("{e:?}"))),
                }
            }
            "toggle" => ids.iter().for_each(|i| self.scene_toggle_visible(i)),
            "lock" => self.scene_set_locked(&ids, true),
            "unlock" => self.scene_set_locked(&ids, false),
            o => return Err(bad(format!("visibility.action '{o}' desconhecida (hide, show, toggle, lock, unlock, unhide_all)"))),
        }
        Ok(())
    }

    fn scene_outliner_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        // ações sobre a seleção aceitam `ids` (viram a seleção antes)
        if matches!(action, "delete" | "duplicate" | "group" | "copy" | "cut" | "snap_floor" | "snap_origin_grid") {
            if let Some(v) = m.get("ids").filter(|v| !v.is_null()) {
                let ids = self.scene_node_ids(v)?;
                self.scene_ui.selection.set_many(ids);
            }
            if self.scene_ui.selection.ids().is_empty() {
                return Err(bad("selecione nós (SCENE op=select) ou informe 'ids'"));
            }
        }
        match action {
            "rename" => {
                let id = self.scene_node_id(m.get("id").ok_or_else(|| bad("'id' é obrigatório"))?)?;
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() || name.chars().count() > 120 {
                    return Err(bad("'name' deve ter 1..120 caracteres"));
                }
                self.scene_rename(&id, &name);
            }
            "delete" => self.scene_delete(),
            "duplicate" => self.scene_duplicate(),
            "group" => self.scene_group(),
            "ungroup" => {
                if let Some(v) = m.get("ids").filter(|v| !v.is_null()) {
                    let ids = self.scene_node_ids(v)?;
                    self.scene_ui.selection.set_many(ids);
                }
                self.scene_ungroup();
            }
            "reparent" => {
                let ids = self.scene_target_ids(m)?;
                let parent = match m.get("parent").filter(|v| !v.is_null()) {
                    Some(p) => Some(self.scene_node_id(p)?),
                    None => None,
                };
                self.scene_reparent(&ids, parent.as_ref());
            }
            "folder" => {
                let ids = self.scene_target_ids(m)?;
                let f = str_of(m, "folder")?;
                if f.chars().count() > 120 || f.contains(['\0', '\\']) {
                    return Err(bad("'folder' inválida (máx. 120 caracteres; use / para aninhar)"));
                }
                self.scene_move_to_folder(&ids, f);
            }
            "copy" => self.scene_copy(),
            "cut" => self.scene_cut(),
            "paste" => {
                if self.scene_ui.clipboard.is_none() {
                    return Err(BridgeError::failed("a área de transferência da cena está vazia (copy/cut antes)"));
                }
                self.scene_paste();
            }
            "snap_floor" => self.scene_snap_to_floor(),
            "snap_origin_grid" => self.scene_snap_origin_to_grid(),
            o => return Err(bad(format!("outliner.action '{o}' desconhecida (rename, delete, duplicate, group, ungroup, reparent, folder, copy, cut, paste, snap_floor, snap_origin_grid)"))),
        }
        Ok(())
    }

    fn scene_place_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        use super::scene_ui::PlaceKind;
        let kind = match str_of(m, "kind")? {
            "cube" => PlaceKind::Primitive(PrimitiveShape::Cube),
            "sphere" => PlaceKind::Primitive(PrimitiveShape::Sphere),
            "plane" => PlaceKind::Primitive(PrimitiveShape::Plane),
            "cylinder" => PlaceKind::Primitive(PrimitiveShape::Cylinder),
            "cone" => PlaceKind::Primitive(PrimitiveShape::Cone),
            "camera" => PlaceKind::Camera,
            "group" => PlaceKind::Group,
            o => return Err(bad(format!("kind '{o}' desconhecido (cube, sphere, plane, cylinder, cone, camera, group); ativos da Biblioteca: anigo_native_place_asset"))),
        };
        let at = match m.get("position").filter(|v| !v.is_null()) {
            Some(p) => Some(DropTarget { point: vec3(p, "position", 10_000.0)?, normal: Vec3::Y, on_geometry: false }),
            None => None,
        };
        let n = self.core.project().scene.nodes.len();
        self.scene_place_actor(kind, at);
        if self.core.project().scene.nodes.len() == n {
            return Err(BridgeError::failed(self.scene_ui.message.clone()));
        }
        Ok(())
    }

    /// Transform LOCAL absoluto de um nó (cada campo é opcional): translation, rotation_deg (XYZ), scale.
    fn scene_transform_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let id = self.scene_node_id(m.get("id").ok_or_else(|| bad("'id' é obrigatório"))?)?;
        let node = self.core.project().scene.node(&id).cloned().ok_or_else(|| bad("nó não existe"))?;
        if node.locked {
            return Err(BridgeError::failed("o nó está travado (use visibility unlock)"));
        }
        let translation = m.get("translation").map(|v| vec3(v, "translation", 100_000.0)).transpose()?.map(|v| v.to_array());
        let rotation = m
            .get("rotation_deg")
            .map(|v| vec3(v, "rotation_deg", 3600.0))
            .transpose()?
            .map(|e| Quat::from_euler(EulerRot::XYZ, e.x.to_radians(), e.y.to_radians(), e.z.to_radians()).to_array());
        let scale = match m.get("scale") {
            None => None,
            Some(Value::Number(_)) => Some(Vec3::splat(num_in(&m["scale"], "scale", 0.001, 1000.0)?).to_array()),
            Some(v) => {
                let s = vec3(v, "scale", 1000.0)?;
                if s.min_element() <= 0.0 {
                    return Err(bad("'scale' deve ser > 0"));
                }
                Some(s.to_array())
            }
        };
        if translation.is_none() && rotation.is_none() && scale.is_none() {
            return Err(bad("informe translation, rotation_deg e/ou scale"));
        }
        if !self.scene_apply(Command::SetNodesTransform { entries: vec![NodeTransformEntry { node_id: id, translation, rotation, scale }] }) && !self.scene_ui.message.is_empty() && self.scene_ui.message.contains(&crate::i18n::tr("char.error").to_string()) {
            return Err(BridgeError::failed(self.scene_ui.message.clone()));
        }
        Ok(())
    }

    fn scene_camera_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let id = self.scene_node_id(m.get("id").ok_or_else(|| bad("'id' é obrigatório"))?)?;
        match str_of(m, "action")? {
            "use_as_render_camera" => {
                if !matches!(self.core.project().scene.node(&id).and_then(|n| n.source.as_ref()), Some(anigo_core::NodeSource::Camera { .. })) {
                    return Err(bad("o nó não é uma câmera de cena"));
                }
                self.scene_use_as_render_camera(&id);
            }
            "align_to_view" => self.scene_align_node_to_view(&id),
            o => return Err(bad(format!("camera.action '{o}' desconhecida (use_as_render_camera, align_to_view)"))),
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ projeto

    fn scene_project_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        use crate::character::persist;
        let action = str_of(m, "action")?;
        let discard = m.get("discard_current").and_then(Value::as_bool).unwrap_or(false);
        let path_arg = |m: &Map<String, Value>| -> Result<std::path::PathBuf, BridgeError> {
            let raw = str_of(m, "path")?;
            let p = std::path::Path::new(raw.trim());
            if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err(bad("'path' deve ser um caminho absoluto sem '..'"));
            }
            persist::normalize_path(raw).map_err(bad)
        };
        match action {
            "info" => {}
            "new" => {
                if !discard {
                    return Err(bad("novo projeto DESCARTA o atual: confirme com discard_current=true (salve antes com action=save)"));
                }
                self.file_new_project();
            }
            "save" => {
                let path = path_arg(m)?;
                self.core.cancel_preview();
                persist::save_project(&self.core, &path).map_err(BridgeError::failed)?;
                self.scene_ui.last_path = path.display().to_string();
            }
            "open" => {
                if !discard {
                    return Err(bad("abrir DESCARTA o projeto atual: confirme com discard_current=true (salve antes com action=save)"));
                }
                let path = path_arg(m)?;
                self.core.cancel_preview();
                persist::load_project(&mut self.core, &path).map_err(BridgeError::failed)?;
                self.core.set_history_limit(400);
                self.scene_ui.selection.clear();
                self.scene_ui.session = None;
                self.scene_ui.cache.forget_resolutions();
                self.scene_ui.runtime.key = None;
                self.scene_ui.framed = false;
                self.char_ui.framed = false;
                self.scene_ui.last_path = path.display().to_string();
            }
            o => return Err(bad(format!("project.action '{o}' desconhecida (info, new, save, open)"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ranges_are_validated() {
        assert_eq!(tool_name(GizmoMode::Rotate), "rotate");
        assert!(view_kind_of("top").is_ok());
        assert!(view_kind_of("cima").is_err());
        assert!(vec3(&json!([1, 2, 3]), "v", 10.0).is_ok());
        assert!(vec3(&json!([1, 2, 30]), "v", 10.0).is_err());
        assert!(vec3(&json!([1, 2]), "v", 10.0).is_err());
        assert!(num_in(&json!(0.0), "x", 0.1, 2.0).is_err());
    }
}
