//! Ações de UI da bridge (T-2.13+): deixam o `anigo-mcp` dirigir o app como um usuário — trocar modo/ferramenta,
//! abrir/fechar painéis, criar e selecionar objetos — e ler o estado da interface (`GET_UI_STATE`).
//!
//! Tudo roda na thread da UI, pelos MESMOS caminhos que os cliques usam (`on_tool_selected`, `DockCmd`, `ModelingSession`),
//! então o que o MCP faz é o que o usuário veria. Nada aqui grava o `ProjectState` do núcleo: isso é `APPLY_COMMAND`.

use super::dock_ui::DockCmd;
use super::StudioApp;
use crate::layout::registry;
use crate::modeling::{Mode, PrimitiveKind};
use crate::workspace::WorkspaceId;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_modeling::SelectMode;
use serde_json::{json, Map, Value};

fn text<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    match m.get(k) {
        Some(Value::String(s)) => Ok(s),
        _ => Err(BridgeError::invalid_params(format!("'{k}' é obrigatório e deve ser texto"))),
    }
}

fn mode_key(m: Mode) -> &'static str {
    match m {
        Mode::Object => "object",
        Mode::Edit => "edit",
        Mode::Sculpt => "sculpt",
        Mode::Hair => "hair",
    }
}

fn parse_mode(s: &str) -> Option<Mode> {
    Some(match s {
        "object" => Mode::Object,
        "edit" => Mode::Edit,
        "sculpt" => Mode::Sculpt,
        "hair" => Mode::Hair,
        _ => return None,
    })
}

pub(super) fn parse_primitive(s: &str) -> Option<PrimitiveKind> {
    Some(match s {
        "plane" => PrimitiveKind::Plane,
        "grid" => PrimitiveKind::Grid,
        "cube" => PrimitiveKind::Cube,
        "quad_sphere" => PrimitiveKind::QuadSphere,
        "uv_sphere" => PrimitiveKind::UvSphere,
        "cylinder" => PrimitiveKind::Cylinder,
        "cone" => PrimitiveKind::Cone,
        "torus" => PrimitiveKind::Torus,
        "capsule" => PrimitiveKind::Capsule,
        "circle" => PrimitiveKind::Circle,
        _ => return None,
    })
}

const PRIMITIVES: &str = "plane|grid|cube|quad_sphere|uv_sphere|cylinder|cone|torus|capsule|circle";

impl StudioApp {
    /// `UI_ACTION` nativo. `select_tab` fica em `bridge_host` (compatibilidade); o resto vem para cá.
    pub(crate) fn bridge_ui_action(&mut self, action: &str, m: &Map<String, Value>) -> BridgeResult {
        match action {
            "set_mode" => {
                let want = text(m, "value")?;
                let mode = parse_mode(want).ok_or_else(|| BridgeError::invalid_params("set_mode: value deve ser object|edit|sculpt|hair"))?;
                if self.ws != WorkspaceId::Modelagem {
                    self.switch_workspace(WorkspaceId::Modelagem);
                }
                self.modeling.set_mode(mode);
                // o shelf mostra as ferramentas do modo novo: volta para a primeira e sincroniza pincel/ferramenta
                self.tool_idx.insert(self.ws, 0);
                if let Some(t) = self.shelf_tools().first().map(|t| t.id) {
                    self.on_tool_selected(t);
                }
                if self.modeling.mode != mode {
                    return Err(BridgeError::failed(format!("não foi possível entrar em '{want}': {}", self.modeling.status)));
                }
                Ok(self.bridge_ui_state())
            }
            "select_tool" => {
                let id = text(m, "value")?;
                let tools = self.shelf_tools();
                let idx = tools.iter().position(|t| t.id == id).ok_or_else(|| {
                    let ids: Vec<&str> = tools.iter().map(|t| t.id).collect();
                    BridgeError::invalid_params(format!("ferramenta '{id}' não existe neste workspace/modo; disponíveis: {}", ids.join(", ")))
                })?;
                self.tool_idx.insert(self.ws, idx);
                self.on_tool_selected(id);
                if self.ws == WorkspaceId::Modelagem && self.modeling.mode == Mode::Sculpt {
                    self.sculpt_sync_tool();
                }
                Ok(self.bridge_ui_state())
            }
            "toggle_panel" => {
                let id = text(m, "value")?;
                let spec = registry::panels(self.ws)
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| BridgeError::invalid_params(format!("painel '{id}' não existe no workspace '{}'", self.ws.key())))?;
                if spec.kind == registry::PanelKind::Viewport {
                    return Err(BridgeError::invalid_params("o painel central (viewport) não fecha"));
                }
                let open_now = self.dock_panel_open(self.ws, spec.id);
                let want = m.get("open").and_then(Value::as_bool).unwrap_or(!open_now);
                if want != open_now {
                    self.dock.cmds.push(DockCmd::Toggle(spec.id));
                }
                // o dock aplica o comando no próximo quadro: devolve já o estado pretendido
                let mut st = self.bridge_ui_state();
                if let Some(arr) = st["panels"].as_array_mut() {
                    for p in arr.iter_mut().filter(|p| p["id"] == spec.id) {
                        p["open"] = json!(want);
                    }
                }
                Ok(st)
            }
            "add_primitive" => {
                let k = text(m, "value")?;
                let kind = parse_primitive(k).ok_or_else(|| BridgeError::invalid_params(format!("add_primitive: value deve ser {PRIMITIVES}")))?;
                let at = match m.get("position") {
                    Some(v) => super::bridge_host::vec3(v, "position")?,
                    None => glam::Vec3::ZERO,
                };
                if self.ws != WorkspaceId::Modelagem {
                    self.switch_workspace(WorkspaceId::Modelagem);
                }
                if self.modeling.mode != Mode::Object {
                    self.modeling.set_mode(Mode::Object);
                }
                let idx = self.modeling.add_primitive(kind, at);
                let mut st = self.bridge_ui_state();
                st["created_index"] = json!(idx);
                Ok(st)
            }
            "select_object" => {
                let idx = m.get("value").and_then(Value::as_u64).ok_or_else(|| BridgeError::invalid_params("select_object: value deve ser o índice (inteiro)"))? as usize;
                if idx >= self.modeling.doc.objects.len() {
                    return Err(BridgeError::invalid_params(format!("select_object: índice {idx} fora do intervalo (0..{})", self.modeling.doc.objects.len())));
                }
                if self.modeling.mode != Mode::Object {
                    self.modeling.set_mode(Mode::Object);
                }
                self.modeling.set_active(idx);
                Ok(self.bridge_ui_state())
            }
            "select_props_tab" => {
                let key = text(m, "value")?;
                let set = self.current_tab_set().ok_or_else(|| BridgeError::invalid_params(format!("o workspace '{}' não tem abas de propriedades", self.ws.key())))?;
                let tab = super::props_tabs::PropTab::from_key(key).filter(|t| set.tabs().contains(t)).ok_or_else(|| {
                    let ks: Vec<&str> = set.tabs().iter().map(|t| t.key()).collect();
                    BridgeError::invalid_params(format!("aba '{key}' não existe em '{}'; disponíveis: {}", self.ws.key(), ks.join(", ")))
                })?;
                self.set_prop_tab(set, tab);
                Ok(self.bridge_ui_state())
            }
            "save_session" => {
                // grava a sessão (layout, vistas, câmeras…) no arquivo de `--session-file`; sem ele a sessão vai para o
                // armazenamento do eframe ao fechar, e aí não há o que gravar daqui
                let path = self.session_path.clone().ok_or_else(|| BridgeError::failed("save_session exige o app iniciado com --session-file"))?;
                let state = self.session_state();
                state.save_to(&path).map_err(|e| BridgeError::failed(format!("falha ao salvar a sessão: {e}")))?;
                Ok(json!({ "saved": path.display().to_string() }))
            }
            "set_camera_view" => {
                if self.ws != WorkspaceId::Cenario {
                    return Err(BridgeError::invalid_params("set_camera_view só existe no Cenário"));
                }
                self.scene_ui.camera_view = m.get("value").and_then(Value::as_bool).ok_or_else(|| BridgeError::invalid_params("set_camera_view: value deve ser true|false"))?;
                Ok(self.bridge_ui_state())
            }
            "set_shading" => {
                let v = text(m, "value")?;
                if self.ws == WorkspaceId::Cenario {
                    self.scene_ui.shading = match v {
                        "solid" => crate::app::scene_ui::SceneShading::Solid,
                        "wireframe" => crate::app::scene_ui::SceneShading::Wire,
                        "cel" => crate::app::scene_ui::SceneShading::Cel,
                        _ => return Err(BridgeError::invalid_params("set_shading: value deve ser solid|wireframe|cel")),
                    };
                } else {
                    self.view_shading = match v {
                        "solid" => super::ViewShading::Solid,
                        "wireframe" => super::ViewShading::Wireframe,
                        "cel" => super::ViewShading::Cel,
                        _ => return Err(BridgeError::invalid_params("set_shading: value deve ser solid|wireframe|cel")),
                    };
                }
                Ok(self.bridge_ui_state())
            }
            "set_view_layout" => {
                let key = text(m, "value")?;
                let layout = super::multiview::ViewLayout::from_key(key)
                    .ok_or_else(|| BridgeError::invalid_params("set_view_layout: value deve ser single|split_v|split_h|quad"))?;
                // vale para o workspace atual se ele tem vistas; em qualquer outro (Render, Shading, Biblioteca…), para a Modelagem
                if self.view_parts(self.ws).is_none() {
                    self.switch_workspace(WorkspaceId::Modelagem);
                }
                let (mv, cam) = self.bridge_view_parts();
                mv.set_layout(cam, layout);
                Ok(self.bridge_ui_state())
            }
            "set_view_kind" => {
                let key = text(m, "value")?;
                let kind = super::multiview::ViewKind::from_key(key)
                    .ok_or_else(|| BridgeError::invalid_params("set_view_kind: value deve ser perspective|ortho|front|back|right|left|top|bottom"))?;
                let (mv, cam) = self.bridge_view_parts();
                let slot = match m.get("slot") {
                    None | Some(Value::Null) => mv.active,
                    Some(v) => v.as_u64().filter(|s| (*s as usize) < super::multiview::MAX_VIEWS).ok_or_else(|| BridgeError::invalid_params("set_view_kind: slot deve ser 0..3"))? as usize,
                };
                if !mv.visible().contains(&slot) {
                    return Err(BridgeError::invalid_params(format!("a vista {slot} não está visível no layout '{}'", mv.layout.key())));
                }
                kind.apply(mv.camera_mut(cam, slot));
                Ok(self.bridge_ui_state())
            }
            "set_active_view" => {
                let slot = m.get("value").and_then(Value::as_u64).filter(|s| (*s as usize) < super::multiview::MAX_VIEWS).ok_or_else(|| BridgeError::invalid_params("set_active_view: value deve ser 0..3"))? as usize;
                let (mv, cam) = self.bridge_view_parts();
                if !mv.visible().contains(&slot) {
                    return Err(BridgeError::invalid_params(format!("a vista {slot} não está visível no layout '{}'", mv.layout.key())));
                }
                mv.set_active(cam, slot);
                Ok(self.bridge_ui_state())
            }
            "set_select_mode" => {
                let sm = match text(m, "value")? {
                    "vertex" => SelectMode::Vertex,
                    "edge" => SelectMode::Edge,
                    "face" => SelectMode::Face,
                    _ => return Err(BridgeError::invalid_params("set_select_mode: value deve ser vertex|edge|face")),
                };
                self.modeling.set_select_mode(sm);
                Ok(self.bridge_ui_state())
            }
            other => Err(BridgeError::invalid_params(format!(
                "UI_ACTION '{other}' desconhecida. Suportadas: select_tab, set_mode, select_tool, toggle_panel, select_props_tab, save_session, set_camera_view, set_shading, set_view_layout, set_view_kind, set_active_view, add_primitive, select_object, set_select_mode"
            ))),
        }
    }

    /// Multi-vista e câmera ativa do workspace atual quando ele tem viewport 3D editável; nos demais, a da Modelagem.
    fn bridge_view_parts(&mut self) -> (&mut super::multiview::MultiView, &mut crate::viewport::OrbitCamera) {
        let ws = if self.view_parts(self.ws).is_some() { self.ws } else { WorkspaceId::Modelagem };
        self.view_parts_mut(ws).expect("workspace com vistas")
    }

    /// `GET_UI_STATE`: o que está na tela agora (workspace, modo, ferramentas e a ativa, painéis abertos, objetos).
    pub(crate) fn bridge_ui_state(&self) -> Value {
        let tools = self.shelf_tools();
        let active = *self.tool_idx.get(&self.ws).unwrap_or(&0);
        let panels: Vec<Value> = registry::panels(self.ws)
            .iter()
            .map(|p| json!({
                "id": p.id,
                "central": p.kind == registry::PanelKind::Viewport,
                "open": p.kind == registry::PanelKind::Viewport || self.dock_panel_open(self.ws, p.id),
            }))
            .collect();
        let objects: Vec<Value> = self
            .modeling
            .doc
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| json!({ "index": i, "name": o.name, "active": self.modeling.doc.active == Some(i) }))
            .collect();
        let views = {
                let (mv, active_cam) = self.view_parts(self.ws).unwrap_or((&self.mview, &self.camera));
                json!({
                    "layout": mv.layout.key(),
                    "active": mv.active,
                    "ratio": mv.ratio,
                    "slots": mv.visible().into_iter().map(|slot| {
                        let cam = if slot == mv.active { active_cam } else { &mv.cams[slot] };
                        let r = mv.rects[slot];
                        json!({ "slot": slot, "kind": super::multiview::ViewKind::of(cam).key(), "yaw": cam.yaw, "pitch": cam.pitch, "distance": cam.distance, "target": [cam.target.x, cam.target.y, cam.target.z], "rect": [r.min.x, r.min.y, r.width(), r.height()] })
                    }).collect::<Vec<_>>(),
                })
            
        };
        let shading = if self.ws == WorkspaceId::Cenario {
                match self.scene_ui.shading {
                    crate::app::scene_ui::SceneShading::Solid => "solid",
                    crate::app::scene_ui::SceneShading::Wire => "wireframe",
                    crate::app::scene_ui::SceneShading::Cel => "cel",
                }
            } else {
                match self.view_shading {
                    super::ViewShading::Solid => "solid",
                    super::ViewShading::Wireframe => "wireframe",
                    super::ViewShading::Cel => "cel",
                }
            };
        json!({
            "workspace": self.ws.key(),
            "input_pending": self.input.pending(),
            "mode": mode_key(self.modeling.mode),
            "shading": shading,
            "select_mode": format!("{:?}", self.modeling.select_mode).to_lowercase(),
            "tools": tools.iter().map(|t| t.id).collect::<Vec<_>>(),
            "active_tool": tools.get(active).map(|t| t.id),
            "props_tab": self.current_tab_set().map(|s| { let shown = self.shown_tabs(s); let cur = self.prop_tab(s); (if shown.contains(&cur) { cur } else { shown[0] }).key() }),
            "props_tabs": self.current_tab_set().map(|s| self.shown_tabs(s).iter().map(|t| t.key()).collect::<Vec<_>>()).unwrap_or_default(),
            "views": views,
            "panels": panels,
            "objects": objects,
            "status": self.modeling.status,
        })
    }
}
