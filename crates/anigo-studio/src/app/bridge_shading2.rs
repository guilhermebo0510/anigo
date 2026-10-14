//! Shading pela bridge, parte 2 (`SHADING`): o que o painel faz além de "material = patch" — opções do preview (modo, luz, fundo,
//! giro, A/B), reset por parâmetro/grupo, "ligar recursos anime", paleta (sombra derivada, giro de matiz, harmonias, paleta do projeto),
//! editor de rampa 2D, presets da Biblioteca e slots de textura (incluindo o mapa SDF facial, que é VIVO).
//! As edições de material continuam UM passo de histórico (o do núcleo para Personagem/Cenário, o da Modelagem para objetos).

use super::bridge_model::reject_unknown_keys;
use super::shading_ui::FaceMap;
use crate::shading::ball::PreviewMode;
use super::StudioApp;
use crate::shading::palette::{self, Harmony, ShadeRecipe};
use crate::shading::params::{self, Group};
use crate::shading::presets;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::npr::{RampCustom, RAMP_MAX_STOPS};
use anigo_core::StylizedMaterial;
use serde_json::{json, Map, Value};

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn num_in(v: &Value, name: &str, lo: f32, hi: f32) -> Result<f32, BridgeError> {
    let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(format!("'{name}' deve ser um número finito")))? as f32;
    if f < lo || f > hi {
        return Err(bad(format!("'{name}' fora de {lo}..{hi}: {f}")));
    }
    Ok(f)
}

fn rgb(v: &Value, name: &str) -> Result<[f32; 3], BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| bad(format!("'{name}' deve ter 3 números (RGB 0..1)")))?;
    Ok([num_in(&a[0], name, 0.0, 1.0)?, num_in(&a[1], name, 0.0, 1.0)?, num_in(&a[2], name, 0.0, 1.0)?])
}

fn group_of(s: &str) -> Result<Group, BridgeError> {
    Group::ALL.into_iter().find(|g| g.id() == s).ok_or_else(|| bad(format!("grupo '{s}' desconhecido; disponíveis: {}", Group::ALL.map(|g| g.id()).join(", "))))
}

fn harmony_of(s: &str) -> Result<Harmony, BridgeError> {
    Ok(match s {
        "complementary" => Harmony::Complementary,
        "analogous" => Harmony::Analogous,
        "triad" => Harmony::Triad,
        "tetrad" => Harmony::Tetrad,
        "split_complementary" => Harmony::SplitComplementary,
        o => return Err(bad(format!("harmonia '{o}' desconhecida (complementary, analogous, triad, tetrad, split_complementary)"))),
    })
}

fn harmony_name(h: Harmony) -> &'static str {
    match h {
        Harmony::Complementary => "complementary",
        Harmony::Analogous => "analogous",
        Harmony::Triad => "triad",
        Harmony::Tetrad => "tetrad",
        Harmony::SplitComplementary => "split_complementary",
    }
}

/// Slots de textura do painel; `face_sdf` é o único que o renderer usa de fato.
pub(crate) const TEXTURE_SLOTS: [&str; 9] = ["base", "shade", "shade2", "ramp", "ilm", "matcap", "emission", "outline_width", "face_sdf"];

impl StudioApp {
    /// `SHADING`: presets/texturas da Biblioteca precisam dela aberta (adia até lá); o resto roda direto.
    pub(crate) fn bridge_shading_ctx(&mut self, ctx: &egui::Context, params: &Value) -> Option<BridgeResult> {
        let op = params.get("op").and_then(Value::as_str).unwrap_or("");
        if matches!(op, "presets" | "texture") {
            let lib = self.bridge_library(ctx)?;
            let m = match params.as_object() {
                Some(m) => m,
                None => return Some(Err(bad("params deve ser um objeto"))),
            };
            self.shading_validate_target();
            return Some(if op == "presets" { self.shading_presets_op(&lib, m) } else { self.shading_texture_op(&lib, m) });
        }
        Some(self.bridge_shading(params))
    }

    pub(crate) fn shading_ui_json(&self) -> Value {
        let u = &self.shading_ui;
        json!({
            "mode": match u.mode { PreviewMode::Ball => "ball", PreviewMode::Object => "object", PreviewMode::Both => "both" },
            "background": u.bg, "turntable": u.turntable,
            "light": { "azimuth_deg": u.rig.azimuth_deg, "elevation_deg": u.rig.elevation_deg, "follows_camera": u.rig.follows_camera },
            "focus_group": u.focus_group.map(|g| g.id()),
            "compare": { "on": u.compare, "has_a": u.snapshot_a.is_some() },
            "palette": {
                "open": u.palette_open, "link_shade": u.link_shade, "harmony": harmony_name(u.harmony), "colors": u.palette.colors,
                "recipe": { "hue_shift_deg": u.recipe.hue_shift_deg, "saturation": u.recipe.saturation, "value": u.recipe.value },
                "dir": u.palette_dir.display().to_string(),
            },
            "ramp_selected_stop": u.ramp_sel,
            "textures": { "slots": TEXTURE_SLOTS, "chosen": u.tex_refs.iter().map(|(k, (id, n))| json!({ "slot": k, "asset_id": id.to_string(), "name": n })).collect::<Vec<_>>(),
                          "face_map": match &u.face_map { FaceMap::None => json!("none"), FaceMap::Procedural => json!("procedural"), FaceMap::Library(id, n) => json!({ "asset_id": id.to_string(), "name": n }) }, "face_map_error": u.face_map_error },
            "message": u.message,
        })
    }

    fn shading_full(&self) -> Value {
        let mut v = self.shading_info_pub();
        v["ui"] = self.shading_ui_json();
        v
    }

    pub(crate) fn shading_ext_op(&mut self, op: &str, m: &Map<String, Value>) -> BridgeResult {
        let depth = self.core.history().undo_depth();
        match op {
            "view" => self.shading_view_op(m)?,
            "param" => return self.shading_param_op(m),
            "palette" => return self.shading_palette_op(m),
            "ramp" => self.shading_ramp_op(m)?,
            "compare" => self.shading_compare_op(m)?,
            _ => return Err(bad(format!("SHADING op '{op}' desconhecida"))),
        }
        let mut out = self.shading_full();
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth));
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------------ preview

    fn shading_view_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (mode, background, turntable, light, focus_group, palette_open, link_shade, recipe, harmony, frame)"))?;
        let u = &self.shading_ui;
        let (mut mode, mut bg, mut turn, mut rig) = (u.mode, u.bg, u.turntable, u.rig);
        let (mut focus, mut pal_open, mut link, mut recipe, mut harmony) = (u.focus_group, u.palette_open, u.link_shade, u.recipe, u.harmony);
        let mut frame = false;
        for (k, v) in patch {
            let b = |n: &str| v.as_bool().ok_or_else(|| bad(format!("'{n}' deve ser booleano")));
            match k.as_str() {
                "mode" => {
                    mode = match v.as_str() {
                        Some("ball") => PreviewMode::Ball,
                        Some("object") => PreviewMode::Object,
                        Some("both") => PreviewMode::Both,
                        _ => return Err(bad("'mode' deve ser ball|object|both")),
                    };
                    frame = true;
                }
                "background" => bg = v.as_u64().filter(|n| *n < super::shading_ui::BACKGROUNDS.len() as u64).ok_or_else(|| bad("'background' deve ser 0..2"))? as u8,
                "turntable" => turn = b(k)?,
                "light" => {
                    let o = v.as_object().ok_or_else(|| bad("'light' deve ser {azimuth_deg, elevation_deg, follows_camera}"))?;
                    for (lk, lv) in o {
                        match lk.as_str() {
                            "azimuth_deg" => rig.azimuth_deg = num_in(lv, lk, -180.0, 180.0)?,
                            "elevation_deg" => rig.elevation_deg = num_in(lv, lk, -80.0, 89.0)?,
                            "follows_camera" => rig.follows_camera = lv.as_bool().ok_or_else(|| bad("'follows_camera' deve ser booleano"))?,
                            o => return Err(bad(format!("light.{o} desconhecido (azimuth_deg, elevation_deg, follows_camera)"))),
                        }
                    }
                }
                "focus_group" => focus = if v.is_null() { None } else { Some(group_of(v.as_str().ok_or_else(|| bad("'focus_group' deve ser texto ou null"))?)?) },
                "palette_open" => pal_open = b(k)?,
                "link_shade" => link = b(k)?,
                "harmony" => harmony = harmony_of(v.as_str().ok_or_else(|| bad("'harmony' deve ser texto"))?)?,
                "recipe" => {
                    let o = v.as_object().ok_or_else(|| bad("'recipe' deve ser {hue_shift_deg, saturation, value}"))?;
                    for (rk, rv) in o {
                        match rk.as_str() {
                            "hue_shift_deg" => recipe.hue_shift_deg = num_in(rv, rk, -180.0, 180.0)?,
                            "saturation" => recipe.saturation = num_in(rv, rk, 0.0, 3.0)?,
                            "value" => recipe.value = num_in(rv, rk, 0.0, 2.0)?,
                            o => return Err(bad(format!("recipe.{o} desconhecido (hue_shift_deg, saturation, value)"))),
                        }
                    }
                }
                "frame" => frame = b(k)?,
                other => return Err(bad(format!("campo desconhecido 'patch.{other}'; disponíveis: background, focus_group, frame, harmony, light, link_shade, mode, palette_open, recipe, turntable"))),
            }
        }
        let u = &mut self.shading_ui;
        (u.mode, u.bg, u.turntable, u.rig) = (mode, bg, turn, rig);
        (u.focus_group, u.palette_open, u.link_shade, u.recipe, u.harmony) = (focus, pal_open, link, recipe, harmony);
        if frame {
            u.framed = false;
        }
        Ok(())
    }

    fn shading_compare_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let cur = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t));
        match str_of(m, "action")? {
            "on" => {
                if self.shading_ui.snapshot_a.is_none() {
                    self.shading_ui.snapshot_a = cur;
                }
                self.shading_ui.compare = true;
                self.shading_ui.framed = false;
            }
            "off" => {
                self.shading_ui.compare = false;
                self.shading_ui.framed = false;
            }
            "set_a" => self.shading_ui.snapshot_a = Some(cur.ok_or_else(|| BridgeError::failed("não há alvo de material"))?),
            "swap" => {
                let a = self.shading_ui.snapshot_a.clone().ok_or_else(|| BridgeError::failed("não há material A (use set_a)"))?;
                let cur = cur.ok_or_else(|| BridgeError::failed("não há alvo de material"))?;
                let (t, _) = self.shading_target_material()?;
                self.shading_apply_strict(&t, a)?;
                self.shading_ui.snapshot_a = Some(cur);
            }
            "revert" => {
                let a = self.shading_ui.snapshot_a.clone().ok_or_else(|| BridgeError::failed("não há material A (use set_a)"))?;
                let (t, _) = self.shading_target_material()?;
                self.shading_apply_strict(&t, a)?;
            }
            o => return Err(bad(format!("compare.action '{o}' desconhecida (on, off, set_a, swap, revert)"))),
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ parâmetros

    fn shading_param_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        if action == "list" {
            let group = match m.get("group").and_then(Value::as_str) {
                Some(g) => Some(group_of(g)?),
                None => None,
            };
            let list: Vec<Value> = params::PARAMS.iter().filter(|p| group.is_none_or(|g| p.group == g)).map(|p| json!({ "id": p.id, "group": p.group.id(), "gate": p.gate })).collect();
            return Ok(json!({ "params": list, "groups": Group::ALL.map(|g| g.id()) }));
        }
        let depth = self.core.history().undo_depth();
        let (t, mut mat) = self.shading_target_material()?;
        match action {
            "reset_param" => {
                let id = str_of(m, "id")?;
                if params::find(id).is_none() {
                    return Err(bad(format!("parâmetro '{id}' desconhecido (use param action=list)")));
                }
                params::reset_param(&mut mat, id);
            }
            "reset_group" => params::reset_group(&mut mat, group_of(str_of(m, "group")?)?),
            "anime_features" => params::enable_anime_features(&mut mat),
            o => return Err(bad(format!("param.action '{o}' desconhecida (list, reset_param, reset_group, anime_features)"))),
        }
        self.shading_apply_strict(&t, mat)?;
        let mut out = self.shading_full();
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth));
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------------ paleta

    fn shading_palette_op(&mut self, m: &Map<String, Value>) -> BridgeResult {
        let action = str_of(m, "action")?;
        let depth = self.core.history().undo_depth();
        let mut extra = json!({});
        match action {
            "derive_shade" => {
                // recipe opcional só desta chamada; senão a do painel
                let mut r: ShadeRecipe = self.shading_ui.recipe;
                if let Some(o) = m.get("recipe").and_then(Value::as_object) {
                    for (k, v) in o {
                        match k.as_str() {
                            "hue_shift_deg" => r.hue_shift_deg = num_in(v, k, -180.0, 180.0)?,
                            "saturation" => r.saturation = num_in(v, k, 0.0, 3.0)?,
                            "value" => r.value = num_in(v, k, 0.0, 2.0)?,
                            o => return Err(bad(format!("recipe.{o} desconhecido (hue_shift_deg, saturation, value)"))),
                        }
                    }
                }
                let (t, mut mat) = self.shading_target_material()?;
                let s = palette::derive_shade([mat.base_color[0], mat.base_color[1], mat.base_color[2]], r);
                mat.shade_color = [s[0], s[1], s[2], mat.shade_color[3]];
                self.shading_apply_strict(&t, mat)?;
            }
            "hue_turn" => {
                let deg = num_in(m.get("deg").ok_or_else(|| bad("'deg' é obrigatório (-180..180)"))?, "deg", -180.0, 180.0)?;
                let (t, mut mat) = self.shading_target_material()?;
                for c in [&mut mat.base_color, &mut mat.shade_color] {
                    let n = palette::hue_shift([c[0], c[1], c[2]], deg);
                    *c = [n[0], n[1], n[2], c[3]];
                }
                self.shading_apply_strict(&t, mat)?;
            }
            "harmony" => {
                let base = match m.get("color") {
                    Some(c) => rgb(c, "color")?,
                    None => {
                        let (_, mat) = self.shading_target_material()?;
                        [mat.base_color[0], mat.base_color[1], mat.base_color[2]]
                    }
                };
                let h = match m.get("harmony").and_then(Value::as_str) {
                    Some(h) => harmony_of(h)?,
                    None => self.shading_ui.harmony,
                };
                let colors = palette::harmony_colors(base, h);
                if m.get("add_to_palette").and_then(Value::as_bool).unwrap_or(false) {
                    for c in &colors {
                        self.shading_ui.palette.add(*c);
                    }
                    self.shading_save_project_palette();
                }
                extra["harmony"] = json!({ "kind": harmony_name(h), "colors": colors });
            }
            "add" => {
                let c = rgb(m.get("color").ok_or_else(|| bad("'color' é obrigatório ([r,g,b] 0..1)"))?, "color")?;
                extra["added"] = json!(self.shading_ui.palette.add(c));
                self.shading_save_project_palette();
            }
            "remove" => {
                let i = m.get("index").and_then(Value::as_u64).map(|n| n as usize).filter(|i| *i < self.shading_ui.palette.colors.len()).ok_or_else(|| bad("'index' fora da paleta (veja ui.palette.colors)"))?;
                self.shading_ui.palette.colors.remove(i);
                self.shading_save_project_palette();
            }
            "apply" => {
                // cor da paleta -> base ou sombra do material (UM passo)
                let i = m.get("index").and_then(Value::as_u64).map(|n| n as usize).filter(|i| *i < self.shading_ui.palette.colors.len()).ok_or_else(|| bad("'index' fora da paleta (veja ui.palette.colors)"))?;
                let c = self.shading_ui.palette.colors[i];
                let (t, mut mat) = self.shading_target_material()?;
                match m.get("to").and_then(Value::as_str).unwrap_or("base") {
                    "base" => {
                        mat.base_color = [c[0], c[1], c[2], mat.base_color[3]];
                        if self.shading_ui.link_shade {
                            let s = palette::derive_shade(c, self.shading_ui.recipe);
                            mat.shade_color = [s[0], s[1], s[2], mat.shade_color[3]];
                        }
                    }
                    "shade" => mat.shade_color = [c[0], c[1], c[2], mat.shade_color[3]],
                    o => return Err(bad(format!("to '{o}' desconhecido (base, shade)"))),
                }
                self.shading_apply_strict(&t, mat)?;
            }
            "save" => {
                self.shading_save_project_palette();
                extra["saved_in"] = json!(self.shading_ui.palette_dir.display().to_string());
            }
            "reload" => self.shading_load_project_palette(),
            "list" => {}
            o => return Err(bad(format!("palette.action '{o}' desconhecida (list, derive_shade, hue_turn, harmony, add, remove, apply, save, reload)"))),
        }
        let mut out = self.shading_full();
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth));
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------------ rampa 2D

    fn shading_ramp_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        let row = |m: &Map<String, Value>| -> Result<usize, BridgeError> {
            match m.get("row") {
                Some(v) => v.as_u64().filter(|r| *r < 4).map(|r| r as usize).ok_or_else(|| bad("'row' deve ser 0..3")),
                None => {
                    let (_, mat) = self.shading_target_material()?;
                    Ok((mat.npr.ramp_row.round().clamp(0.0, 3.0)) as usize)
                }
            }
        };
        if action == "select_stop" {
            self.shading_ui.ramp_sel = match m.get("index") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().filter(|i| (*i as usize) < RAMP_MAX_STOPS).ok_or_else(|| bad("'index' deve ser 0..5 ou null"))? as usize),
            };
            return Ok(());
        }
        let (t, mut mat) = self.shading_target_material()?;
        let r = row(m)?;
        match action {
            "select_row" => mat.npr.ramp_row = r as f32,
            "customize" => {
                let (constant, stops) = RampCustom::contract_stops(r);
                mat.npr.ramp_custom.set_stops(r, constant, &stops);
                if mat.npr.ramp_blend <= 0.0 {
                    mat.npr.ramp_blend = 1.0;
                }
                mat.npr.ramp_row = r as f32;
            }
            "set_stops" => {
                let arr = m.get("stops").and_then(Value::as_array).filter(|a| (2..=RAMP_MAX_STOPS).contains(&a.len())).ok_or_else(|| bad(format!("'stops' deve ter 2..{RAMP_MAX_STOPS} pares [posição, valor]")))?;
                let mut stops = Vec::new();
                for s in arr {
                    let p = s.as_array().filter(|p| p.len() == 2).ok_or_else(|| bad("cada parada deve ser [posição 0..1, valor 0..1]"))?;
                    stops.push((num_in(&p[0], "posição", 0.0, 1.0)?, num_in(&p[1], "valor", 0.0, 1.0)?));
                }
                let constant = m.get("constant").and_then(Value::as_bool).unwrap_or(mat.npr.ramp_custom.constant[r]);
                mat.npr.ramp_custom.set_stops(r, constant, &stops);
                if mat.npr.ramp_blend <= 0.0 {
                    mat.npr.ramp_blend = 1.0;
                }
            }
            "clear_row" => mat.npr.ramp_custom.clear_row(r),
            o => return Err(bad(format!("ramp.action '{o}' desconhecida (select_row, customize, set_stops, clear_row, select_stop)"))),
        }
        self.shading_apply_strict(&t, mat)?;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ presets e texturas (Biblioteca)

    fn shading_presets_op(&mut self, lib: &anigo_assets::Library, m: &Map<String, Value>) -> BridgeResult {
        let depth = self.core.history().undo_depth();
        let mut extra = json!({});
        match str_of(m, "action")? {
            "list" => {}
            "save" => {
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() || name.chars().count() > 120 {
                    return Err(bad("'name' deve ter 1..120 caracteres"));
                }
                let (_, mat) = self.shading_target_material()?;
                let id = presets::save_preset(lib, &name, &mat, &self.shading_texture_refs()).map_err(BridgeError::failed)?;
                self.shading_ui.preset_rows_for = None;
                extra["saved"] = json!({ "asset_id": id.to_string(), "name": name });
            }
            "apply" => {
                let rows = presets::list_presets(lib);
                let row = match (m.get("asset_id").and_then(Value::as_str), m.get("name").and_then(Value::as_str)) {
                    (Some(id), None) => rows.iter().find(|r| r.id.to_string() == id),
                    (None, Some(n)) => rows.iter().find(|r| r.name == n),
                    _ => return Err(bad("informe 'asset_id' OU 'name' do preset (veja action=list)")),
                }
                .ok_or_else(|| bad("preset não existe na Biblioteca (veja action=list)"))?;
                let (t, cur) = self.shading_target_material()?;
                let (mut nm, refs) = presets::load_preset_with_refs(lib, &row.id).map_err(BridgeError::failed)?;
                nm.name = cur.name;
                self.shading_apply_strict(&t, nm)?;
                for (slot, uri) in refs {
                    if let Some(aid) = uri.strip_prefix("anigo://library/").and_then(|u| u.parse::<anigo_assets::AssetId>().ok()) {
                        let name = lib.row(&aid).ok().flatten().map(|r| r.name).unwrap_or_else(|| uri.clone());
                        self.shading_ui.tex_refs.insert(slot, (aid, name));
                    }
                }
            }
            o => return Err(bad(format!("presets.action '{o}' desconhecida (list, save, apply)"))),
        }
        let rows = presets::list_presets(lib);
        let mut out = self.shading_full();
        out["library_presets"] = json!(rows.iter().map(|r| json!({ "asset_id": r.id.to_string(), "name": r.name })).collect::<Vec<_>>());
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth));
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }

    fn shading_texture_op(&mut self, lib: &anigo_assets::Library, m: &Map<String, Value>) -> BridgeResult {
        let slot_of = |m: &Map<String, Value>| -> Result<&'static str, BridgeError> {
            let s = str_of(m, "slot")?;
            TEXTURE_SLOTS.iter().find(|t| **t == s).copied().ok_or_else(|| bad(format!("slot '{s}' desconhecido; disponíveis: {}", TEXTURE_SLOTS.join(", "))))
        };
        let mut extra = json!({});
        match str_of(m, "action")? {
            "list" => {
                let text = m.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty() && t.len() <= 256).map(str::to_string);
                let q = anigo_assets::Query { kinds: vec![anigo_assets::AssetKind::Texture], text, limit: 60, ..Default::default() };
                let page = lib.query(&q).map_err(|e| BridgeError::failed(e.to_string()))?;
                extra["textures"] = json!(page.items.iter().map(|r| json!({ "asset_id": r.id.to_string(), "name": r.name })).collect::<Vec<_>>());
            }
            "choose" => {
                let slot = slot_of(m)?;
                let id: anigo_assets::AssetId = str_of(m, "asset_id")?.parse().map_err(|e: anigo_assets::AssetError| bad(e.to_string()))?;
                let row = lib.require_row(&id).map_err(|e| bad(e.to_string()))?;
                if row.kind != anigo_assets::AssetKind::Texture {
                    return Err(bad(format!("o ativo '{}' não é uma textura ({})", row.name, row.kind.as_str())));
                }
                if slot == "face_sdf" {
                    self.shading_ui.face_map_error = None;
                    self.shading_install_face_map(FaceMap::Library(id, row.name.clone()));
                    if let Some(e) = &self.shading_ui.face_map_error {
                        return Err(BridgeError::failed(format!("mapa SDF facial: {e}")));
                    }
                }
                self.shading_ui.tex_refs.insert(slot.to_string(), (id, row.name));
            }
            "clear" => {
                let slot = slot_of(m)?;
                self.shading_ui.tex_refs.remove(slot);
                if slot == "face_sdf" {
                    self.shading_install_face_map(FaceMap::None);
                }
            }
            "face_procedural" => {
                // mapa SDF facial procedural + liga `face_sdf_enabled` (UM passo), como o botão
                self.shading_ui.face_map_error = None;
                self.shading_install_face_map(FaceMap::Procedural);
                if let Some(e) = &self.shading_ui.face_map_error {
                    return Err(BridgeError::failed(format!("mapa SDF facial: {e}")));
                }
                let (t, mut mat) = self.shading_target_material()?;
                mat.face_sdf_enabled = true;
                self.shading_apply_strict(&t, mat)?;
            }
            o => return Err(bad(format!("texture.action '{o}' desconhecida (list, choose, clear, face_procedural)"))),
        }
        let mut out = self.shading_full();
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }
}

#[allow(dead_code)]
fn _keep(_: &StylizedMaterial) {
    let _ = reject_unknown_keys;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_harmonies_and_slots_have_stable_names() {
        for g in Group::ALL {
            assert_eq!(group_of(g.id()).unwrap(), g);
        }
        assert!(group_of("zzz").is_err());
        for h in ["complementary", "analogous", "triad", "tetrad", "split_complementary"] {
            assert_eq!(harmony_name(harmony_of(h).unwrap()), h);
        }
        assert!(harmony_of("x").is_err());
        assert!(TEXTURE_SLOTS.contains(&"face_sdf"));
    }

    #[test]
    fn numbers_and_colors_are_range_checked() {
        assert!(num_in(&json!(0.5), "x", 0.0, 1.0).is_ok());
        assert!(num_in(&json!(2.0), "x", 0.0, 1.0).is_err());
        assert!(rgb(&json!([0.0, 0.5, 1.0]), "c").is_ok());
        assert!(rgb(&json!([0.0, 0.5, 1.5]), "c").is_err());
    }

    #[test]
    fn reset_param_and_group_restore_defaults() {
        let mut m = StylizedMaterial { shadow_threshold: 0.9, rim_intensity: 2.5, ..Default::default() };
        params::reset_param(&mut m, "shadow_threshold");
        assert_eq!(m.shadow_threshold, StylizedMaterial::default().shadow_threshold);
        assert_eq!(m.rim_intensity, 2.5, "o resto não muda");
        params::reset_group(&mut m, Group::Rim);
        assert_eq!(m.rim_intensity, StylizedMaterial::default().rim_intensity);
        assert!(params::find("nao_existe").is_none());
    }
}
