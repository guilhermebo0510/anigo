//! Posing pela bridge (`POSE`): o workspace sem janela — seleção de ossos, rotação FK (Euler, quaternion, eixo de mundo), IK de membros,
//! quadril/raiz com foot lock e auto-ground, olhar, mãos, presets, espelhar/copiar/colar/zerar e a Biblioteca de poses; mais as opções
//! de edição (limites, simetria, snap, IK por membro…).
//!
//! Usa as MESMAS funções da UI (`crate::posing::edit`, `pose_commit_to`…): cada chamada vira UM passo de desfazer do núcleo
//! (`anigo_native_undo`), com os limites do perfil ativo, e na Animação com auto-key vira `Batch[SetPose, InsertKeys]`.
//! Tudo é validado antes de mexer (nome de osso, intervalo, vetor finito); campo desconhecido é erro.

use super::posing_ui::{character_xf, limb_index, HandSide};
use super::StudioApp;
use crate::posing::edit::{self, EditOpts, HipsMove};
use crate::posing::limits_vis::swing_twist_deg;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use anigo_core::pose::{self, bone, bone_limit, quat_to_euler_deg, Limb, LimitProfile, PoseOp, BONE_COUNT, BONE_NAMES, BONE_PARENTS};
use anigo_ik::pose_ik;
use glam::{Quat, Vec3};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn bad(msg: impl Into<String>) -> BridgeError {
    BridgeError::invalid_params(msg)
}

fn str_of<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str, BridgeError> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' é obrigatório e deve ser texto")))
}

fn opt_bool(m: &Map<String, Value>, k: &str) -> Result<Option<bool>, BridgeError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(format!("'{k}' deve ser booleano"))),
    }
}

fn num(v: &Value, name: &str) -> Result<f32, BridgeError> {
    v.as_f64().filter(|f| f.is_finite() && f.abs() < 1.0e6).map(|f| f as f32).ok_or_else(|| bad(format!("'{name}' deve ser um número finito")))
}

fn vec3(v: &Value, name: &str) -> Result<Vec3, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| bad(format!("'{name}' deve ter 3 números")))?;
    Ok(Vec3::new(num(&a[0], name)?, num(&a[1], name)?, num(&a[2], name)?))
}

fn arr<const N: usize>(v: &Value, name: &str) -> Result<[f32; N], BridgeError> {
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(|| bad(format!("'{name}' deve ter {N} números")))?;
    let mut o = [0f32; N];
    for (i, x) in a.iter().enumerate() {
        o[i] = num(x, name)?;
    }
    Ok(o)
}

/// Osso por índice (0..24) ou nome (sem diferenciar maiúsculas; também aceita `L_Elbow`-like do esqueleto: nome canônico).
fn bone_of(v: &Value) -> Result<usize, BridgeError> {
    if let Some(n) = v.as_u64() {
        return if (n as usize) < BONE_COUNT { Ok(n as usize) } else { Err(bad(format!("osso {n} fora de 0..{}", BONE_COUNT - 1))) };
    }
    let s = v.as_str().ok_or_else(|| bad("osso deve ser índice ou nome"))?;
    let norm = |x: &str| x.to_lowercase().replace(['_', ' ', '-'], "");
    BONE_NAMES.iter().position(|n| norm(n) == norm(s)).ok_or_else(|| bad(format!("osso '{s}' desconhecido; nomes: {}", BONE_NAMES.join(", "))))
}

fn bones_of(v: &Value, name: &str) -> Result<BTreeSet<usize>, BridgeError> {
    let a = v.as_array().filter(|a| a.len() <= 64).ok_or_else(|| bad(format!("'{name}' deve ser uma lista de ossos (índices ou nomes)")))?;
    a.iter().map(bone_of).collect()
}

fn limb_of(s: &str) -> Result<Limb, BridgeError> {
    Limb::ALL.into_iter().find(|l| l.key() == s).ok_or_else(|| bad(format!("membro '{s}' desconhecido: left_arm, right_arm, left_leg, right_leg")))
}

fn profile_name(p: LimitProfile) -> &'static str {
    match p {
        LimitProfile::Human => "human",
        LimitProfile::Anime => "anime",
        LimitProfile::Free => "free",
    }
}

fn side_name(s: HandSide) -> &'static str {
    match s {
        HandSide::Left => "left",
        HandSide::Right => "right",
        HandSide::Both => "both",
    }
}

impl StudioApp {
    pub(crate) fn bridge_pose(&mut self, params: &Value) -> BridgeResult {
        let m = params.as_object().ok_or_else(|| bad("params deve ser um objeto"))?;
        let op = str_of(m, "op")?;
        if op != "info" {
            self.bridge_reset_gestures();
        }
        let before = self.pose_now();
        let depth0 = self.core.history().undo_depth();
        self.pose_ui.message.clear();
        match op {
            "info" => return Ok(self.pose_info(m)),
            "options" => self.pose_set_options(m)?,
            "select" => self.pose_select_op(m)?,
            "bone" => self.pose_bone_op(m)?,
            "translate" => self.pose_translate_op(m)?,
            "ik" => self.pose_ik_op(m)?,
            "gaze" => self.pose_gaze_op(m)?,
            "ground" => self.pose_ground_now(),
            "mirror" => self.pose_mirror(),
            "copy" => self.pose_copy(),
            "paste" => {
                if self.pose_ui.clipboard.is_none() {
                    return Err(BridgeError::failed("a área de transferência de pose está vazia (use op=copy antes)"));
                }
                self.pose_paste(opt_bool(m, "flipped")?.unwrap_or(false));
            }
            "reset" => self.pose_reset_op(m)?,
            "preset" => self.pose_preset_op(m)?,
            "hands" => self.pose_hands_op(m)?,
            "library" => self.pose_library_op(m)?,
            other => return Err(bad(format!("POSE op '{other}' desconhecida (info, options, select, bone, translate, ik, gaze, ground, mirror, copy, paste, reset, preset, hands, library)"))),
        }
        let mut out = self.pose_info(&Map::new());
        out["changed"] = json!(self.pose_now() != before);
        out["history_steps_added"] = json!(self.core.history().undo_depth().saturating_sub(depth0));
        out["message"] = json!(self.pose_ui.message);
        Ok(out)
    }

    // ------------------------------------------------------------------------------------------------ info

    fn pose_info(&mut self, m: &Map<String, Value>) -> Value {
        let compact = m.get("compact").and_then(Value::as_bool).unwrap_or(false);
        let pose = self.pose_now();
        let rig = self.pose_rig();
        let xf = character_xf(&self.core);
        let joints = rig.joint_positions(&pose);
        let posed: BTreeSet<usize> = pose.posed_bones().into_iter().collect();
        let profile = self.pose_ui.profile;
        let bones: Vec<Value> = (0..BONE_COUNT)
            .map(|b| {
                let q = pose.rotation(b);
                let lim = bone_limit(b, profile);
                let st = swing_twist_deg(&pose, b);
                let mut v = json!({
                    "index": b, "name": BONE_NAMES[b], "parent": BONE_PARENTS[b], "posed": posed.contains(&b), "selected": self.pose_ui.selection.contains(&b),
                    "euler_deg": quat_to_euler_deg(q), "quat": q.to_array(),
                    "joint_character": joints[b].to_array(), "joint_world": xf.transform_point3(joints[b]).to_array(),
                });
                if let Some(l) = lim {
                    v["limit"] = json!({ "swing_x": l.sx, "swing_z": l.sz, "twist": l.twist, "value": st, "within": st[0] >= l.sx[0] - 0.05 && st[0] <= l.sx[1] + 0.05 && st[1] >= l.sz[0] - 0.05 && st[1] <= l.sz[1] + 0.05 && st[2] >= l.twist[0] - 0.05 && st[2] <= l.twist[1] + 0.05 });
                }
                v
            })
            .collect();
        let (com, hull, inside) = pose::balance(&rig, &pose, 0.03);
        let pu = &self.pose_ui;
        let (can_undo, can_redo) = (self.core.history().undo_depth() > 0, self.core.history().redo_depth() > 0);
        json!({
            "workspace": self.ws.key(),
            "bones": if compact { Value::Null } else { Value::Array(bones) },
            "posed_bones": posed.iter().map(|b| BONE_NAMES[*b]).collect::<Vec<_>>(),
            "root_translation": pose.root_translation, "hips_translation": pose.hips_translation,
            "hands": { "left": pose.hands.left, "right": pose.hands.right },
            "balance": { "com": com.to_array(), "inside_support": inside, "hull_points": hull.len(), "lowest_contact_y": rig.lowest_contact_y(&pose) },
            "selection": { "bones": pu.selection.iter().copied().collect::<Vec<_>>(), "active": pu.active, "names": pu.selection.iter().map(|b| BONE_NAMES[*b]).collect::<Vec<_>>() },
            "options": {
                "profile": profile_name(pu.profile), "symmetry": pu.symmetry, "snap_deg": pu.snap_deg, "space_local": pu.space_local,
                "limb_ik": Limb::ALL.iter().map(|l| (l.key().to_string(), json!(pu.limb_ik[limb_index(*l)]))).collect::<Map<String, Value>>(),
                "spine_ik": pu.spine_ik, "foot_lock": pu.foot_lock, "auto_ground": pu.auto_ground,
                "show": { "mesh": pu.show_mesh, "skeleton": pu.show_skeleton, "balance": pu.show_balance, "labels": pu.show_labels, "limits": pu.show_limits },
                "gaze": { "enabled": pu.gaze.enabled, "target": pu.gaze.target.to_array(), "weight": pu.gaze.weight },
                "library": { "blend": pu.blend, "apply_only_selected": pu.apply_only_selected, "lib_name": pu.lib_name, "only_selected": pu.lib_only_selected },
                "hand_side": side_name(pu.hand_side),
            },
            "clipboard": pu.clipboard.as_ref().map(|(_, mask)| json!({ "bones": mask.len() })),
            "pose_assets": pu.poses.iter().enumerate().map(|(i, c)| json!({ "index": i, "id": c.id.to_string(), "name": c.name, "revision": c.revision, "bones": c.asset.mask().len() })).collect::<Vec<_>>(),
            "presets": { "body": pose::presets::BODY, "hand": pose::presets::HAND },
            "limbs": Limb::ALL.iter().map(|l| l.key()).collect::<Vec<_>>(),
            "history": { "can_undo": can_undo, "can_redo": can_redo, "undo_depth": self.core.history().undo_depth() },
        })
    }

    // ------------------------------------------------------------------------------------------------ opções e seleção

    /// Patch de opções (todo-ou-nada).
    fn pose_set_options(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let patch = m.get("patch").and_then(Value::as_object).ok_or_else(|| bad("'patch' é obrigatório (profile, symmetry, snap_deg, space_local, limb_ik, spine_ik, foot_lock, auto_ground, show, gaze, blend, apply_only_selected, hand_side, lib_name, lib_only_selected)"))?;
        let pu = &self.pose_ui;
        let (mut profile, mut symmetry, mut snap, mut local) = (pu.profile, pu.symmetry, pu.snap_deg, pu.space_local);
        let (mut limb_ik, mut spine, mut foot, mut ground) = (pu.limb_ik, pu.spine_ik, pu.foot_lock, pu.auto_ground);
        let mut show = (pu.show_mesh, pu.show_skeleton, pu.show_balance, pu.show_labels, pu.show_limits);
        let mut gaze = pu.gaze;
        let (mut blend, mut only_sel, mut side, mut name, mut lib_sel) = (pu.blend, pu.apply_only_selected, pu.hand_side, pu.lib_name.clone(), pu.lib_only_selected);
        let b = |v: &Value, n: &str| v.as_bool().ok_or_else(|| bad(format!("'{n}' deve ser booleano")));
        for (k, v) in patch {
            match k.as_str() {
                "profile" => {
                    profile = match v.as_str() {
                        Some("human") => LimitProfile::Human,
                        Some("anime") => LimitProfile::Anime,
                        Some("free") => LimitProfile::Free,
                        _ => return Err(bad("'profile' deve ser human|anime|free")),
                    }
                }
                "symmetry" => symmetry = b(v, k)?,
                "snap_deg" => snap = if v.is_null() { None } else { Some(num(v, k).and_then(|d| if d > 0.0 && d <= 90.0 { Ok(d) } else { Err(bad("'snap_deg' deve ser null ou 0..90")) })?) },
                "space_local" => local = b(v, k)?,
                "spine_ik" => spine = b(v, k)?,
                "foot_lock" => foot = b(v, k)?,
                "auto_ground" => ground = b(v, k)?,
                "limb_ik" => {
                    let o = v.as_object().ok_or_else(|| bad("'limb_ik' deve ser um objeto {left_arm: bool, …}"))?;
                    for (lk, lv) in o {
                        limb_ik[limb_index(limb_of(lk)?)] = b(lv, lk)?;
                    }
                }
                "show" => {
                    let o = v.as_object().ok_or_else(|| bad("'show' deve ser um objeto {mesh, skeleton, balance, labels, limits}"))?;
                    for (sk, sv) in o {
                        let x = b(sv, sk)?;
                        match sk.as_str() {
                            "mesh" => show.0 = x,
                            "skeleton" => show.1 = x,
                            "balance" => show.2 = x,
                            "labels" => show.3 = x,
                            "limits" => show.4 = x,
                            o => return Err(bad(format!("show.{o} desconhecido (mesh, skeleton, balance, labels, limits)"))),
                        }
                    }
                }
                "gaze" => {
                    let o = v.as_object().ok_or_else(|| bad("'gaze' deve ser um objeto {enabled, target, weight}"))?;
                    for (gk, gv) in o {
                        match gk.as_str() {
                            "enabled" => gaze.enabled = b(gv, gk)?,
                            "target" => gaze.target = vec3(gv, "gaze.target")?,
                            "weight" => gaze.weight = num(gv, gk).and_then(|w| if (0.0..=1.0).contains(&w) { Ok(w) } else { Err(bad("'gaze.weight' deve ser 0..1")) })?,
                            o => return Err(bad(format!("gaze.{o} desconhecido (enabled, target, weight)"))),
                        }
                    }
                }
                "blend" => blend = num(v, k).and_then(|w| if (0.0..=1.5).contains(&w) { Ok(w) } else { Err(bad("'blend' deve ser 0..1.5")) })?,
                "apply_only_selected" => only_sel = b(v, k)?,
                "hand_side" => {
                    side = match v.as_str() {
                        Some("left") => HandSide::Left,
                        Some("right") => HandSide::Right,
                        Some("both") => HandSide::Both,
                        _ => return Err(bad("'hand_side' deve ser left|right|both")),
                    }
                }
                "lib_name" => name = v.as_str().filter(|s| s.chars().count() <= 120 && !s.contains('\0')).ok_or_else(|| bad("'lib_name' deve ser texto (até 120 caracteres)"))?.to_string(),
                "lib_only_selected" => lib_sel = b(v, k)?,
                o => return Err(bad(format!("campo desconhecido 'patch.{o}'; disponíveis: apply_only_selected, auto_ground, blend, foot_lock, gaze, hand_side, lib_name, lib_only_selected, limb_ik, profile, show, snap_deg, space_local, spine_ik, symmetry"))),
            }
        }
        let pu = &mut self.pose_ui;
        (pu.profile, pu.symmetry, pu.snap_deg, pu.space_local) = (profile, symmetry, snap, local);
        (pu.limb_ik, pu.spine_ik, pu.foot_lock, pu.auto_ground) = (limb_ik, spine, foot, ground);
        (pu.show_mesh, pu.show_skeleton, pu.show_balance, pu.show_labels, pu.show_limits) = show;
        pu.gaze = gaze;
        (pu.blend, pu.apply_only_selected, pu.hand_side, pu.lib_name, pu.lib_only_selected) = (blend, only_sel, side, name, lib_sel);
        Ok(())
    }

    fn pose_select_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let mode = m.get("mode").and_then(Value::as_str).unwrap_or("set");
        if opt_bool(m, "none")?.unwrap_or(false) {
            self.pose_ui.clear_selection();
            return Ok(());
        }
        let set: BTreeSet<usize> = if opt_bool(m, "all")?.unwrap_or(false) {
            (0..BONE_COUNT).filter(|b| *b != bone::ROOT).collect()
        } else {
            bones_of(m.get("bones").ok_or_else(|| bad("informe 'bones', 'all' ou 'none'"))?, "bones")?
        };
        let pu = &mut self.pose_ui;
        match mode {
            "set" => {
                pu.selection = set.clone();
                pu.active = set.iter().next_back().copied();
            }
            "add" => {
                for b in &set {
                    pu.selection.insert(*b);
                    pu.active = Some(*b);
                }
            }
            "remove" => {
                for b in &set {
                    pu.selection.remove(b);
                }
                if pu.active.is_some_and(|a| !pu.selection.contains(&a)) {
                    pu.active = pu.selection.iter().next_back().copied();
                }
            }
            "toggle" => {
                for b in &set {
                    pu.toggle(*b);
                }
            }
            o => return Err(bad(format!("mode '{o}' desconhecido (set, add, remove, toggle)"))),
        }
        if let Some(a) = m.get("active").filter(|v| !v.is_null()) {
            let a = bone_of(a)?;
            if !pu.selection.contains(&a) {
                return Err(bad("'active' precisa estar na seleção"));
            }
            pu.active = Some(a);
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ edição

    fn edit_opts(&self) -> EditOpts {
        self.pose_ui.edit_opts()
    }

    fn pose_bone_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let action = str_of(m, "action")?;
        let b = match m.get("bone") {
            Some(v) => bone_of(v)?,
            None => self.pose_ui.active.ok_or_else(|| bad("informe 'bone' (ou selecione um osso ativo)"))?,
        };
        if b == bone::ROOT && action != "reset" {
            return Err(bad("o osso Root não gira: use op=translate para mover o personagem"));
        }
        let now = self.pose_now();
        let opts = self.edit_opts();
        let target = match action {
            "set_euler" => edit::set_bone_euler(&now, b, arr::<3>(m.get("euler_deg").ok_or_else(|| bad("'euler_deg' é obrigatório ([x,y,z] graus)"))?, "euler_deg")?, &opts),
            "set_quat" => {
                let q = arr::<4>(m.get("quat").ok_or_else(|| bad("'quat' é obrigatório ([x,y,z,w])"))?, "quat")?;
                let q = Quat::from_array(q);
                if q.length_squared() < 1e-8 {
                    return Err(bad("'quat' não pode ser zero"));
                }
                edit::set_bone_local(&now, b, q.normalize(), &opts)
            }
            "rotate_world" => {
                let axis = vec3(m.get("axis").ok_or_else(|| bad("'axis' é obrigatório (vetor de mundo)"))?, "axis")?;
                if axis.length_squared() < 1e-12 {
                    return Err(bad("'axis' não pode ser zero"));
                }
                let angle = num(m.get("angle_deg").ok_or_else(|| bad("'angle_deg' é obrigatório"))?, "angle_deg")?.to_radians();
                // o eixo vem em MUNDO; a pose vive no espaço do personagem
                let xf = character_xf(&self.core);
                let axis_char = (xf.inverse().transform_vector3(axis)).normalize_or_zero();
                let rig = self.pose_rig();
                edit::rotate_bone_world(&rig, &now, b, axis_char, angle, &opts)
            }
            "reset" => {
                let mut sel: BTreeSet<usize> = [b].into_iter().collect();
                if self.pose_ui.symmetry {
                    sel.insert(pose::mirror_bone(b));
                }
                edit::reset_bones(&now, &sel)
            }
            o => return Err(bad(format!("bone.action '{o}' desconhecida (set_euler, set_quat, rotate_world, reset)"))),
        };
        self.pose_commit_to(target, if action == "reset" { PoseOp::Reset } else { PoseOp::Edit });
        Ok(())
    }

    fn pose_translate_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let what = str_of(m, "part")?;
        let now = self.pose_now();
        let cur = if what == "root" { now.root_translation } else { now.hips_translation };
        let new = match (m.get("value"), m.get("delta")) {
            (Some(v), None) => vec3(v, "value")?,
            (None, Some(d)) => Vec3::from_array(cur) + vec3(d, "delta")?,
            _ => return Err(bad("informe 'value' (absoluto, metros) OU 'delta'")),
        };
        if new.abs().max_element() > 10.0 {
            return Err(bad("translação fora de ±10 m"));
        }
        let target = match what {
            "hips" => {
                let rig = self.pose_rig();
                let foot_lock = opt_bool(m, "foot_lock")?.unwrap_or(self.pose_ui.foot_lock);
                let auto_ground = opt_bool(m, "auto_ground")?.unwrap_or(self.pose_ui.auto_ground);
                let mv = HipsMove { delta: new - Vec3::from_array(cur), whole_body: false, foot_lock, auto_ground };
                edit::move_hips(&rig, &now, mv, &self.edit_opts())
            }
            "root" => {
                let mut t = now.clone();
                t.root_translation = new.to_array();
                t
            }
            o => return Err(bad(format!("part '{o}' desconhecido (hips, root)"))),
        };
        self.pose_commit_to(target, PoseOp::Edit);
        Ok(())
    }

    /// Ponto: `space` = `world` (padrão) ou `character`.
    fn to_character(&self, m: &Map<String, Value>, v: &Value, name: &str) -> Result<Vec3, BridgeError> {
        let p = vec3(v, name)?;
        match m.get("space").and_then(Value::as_str).unwrap_or("world") {
            "world" => Ok(character_xf(&self.core).inverse().transform_point3(p)),
            "character" => Ok(p),
            o => Err(bad(format!("space '{o}' desconhecido (world, character)"))),
        }
    }

    fn pose_ik_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let limb = limb_of(str_of(m, "limb")?)?;
        let target = self.to_character(m, m.get("target").ok_or_else(|| bad("'target' é obrigatório ([x,y,z])"))?, "target")?;
        let pole = match m.get("pole").filter(|v| !v.is_null()) {
            Some(p) => Some(self.to_character(m, p, "pole")?),
            None => None,
        };
        let rig = self.pose_rig();
        let now = self.pose_now();
        let t = edit::drag_effector(&rig, &now, limb, target, pole, &self.edit_opts());
        self.pose_commit_to(t, PoseOp::Ik);
        Ok(())
    }

    fn pose_gaze_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let rig = self.pose_rig();
        let now = self.pose_now();
        match str_of(m, "action")? {
            "apply" => {
                let target = match m.get("target").filter(|v| !v.is_null()) {
                    Some(t) => self.to_character(m, t, "target")?,
                    None => self.pose_ui.gaze.target,
                };
                let weight = match m.get("weight") {
                    Some(w) => num(w, "weight").and_then(|w| if (0.0..=1.0).contains(&w) { Ok(w) } else { Err(bad("'weight' deve ser 0..1")) })?,
                    None => self.pose_ui.gaze.weight,
                };
                self.pose_ui.gaze.target = target;
                self.pose_ui.gaze.weight = weight;
                let t = pose_ik::look_at_head(&rig, &now, target, weight, 0.35, self.pose_ui.profile).pose;
                self.pose_commit_to(t, PoseOp::Gaze);
            }
            "center" => {
                let mut t = now;
                t.set_rotation(bone::NECK, Quat::IDENTITY);
                t.set_rotation(bone::HEAD, Quat::IDENTITY);
                self.pose_commit_to(t, PoseOp::Gaze);
            }
            "front" => {
                let eye = pose_ik::eye_center(&rig, &now);
                let f = pose_ik::face_direction(&rig, &now);
                self.pose_ui.gaze.target = eye + f;
                self.pose_ui.gaze.enabled = true;
            }
            o => return Err(bad(format!("gaze.action '{o}' desconhecida (apply, center, front)"))),
        }
        Ok(())
    }

    fn pose_reset_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        match m.get("scope").and_then(Value::as_str).unwrap_or("selection") {
            "all" => self.pose_reset_all(),
            "selection" => self.pose_reset_selection(),
            "bones" => {
                let set = bones_of(m.get("bones").ok_or_else(|| bad("'bones' é obrigatório com scope=bones"))?, "bones")?;
                let t = edit::reset_bones(&self.pose_now(), &set);
                self.pose_commit_to(t, PoseOp::Reset);
            }
            o => return Err(bad(format!("scope '{o}' desconhecido (selection, all, bones)"))),
        }
        Ok(())
    }

    fn pose_preset_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let key = str_of(m, "body")?;
        if !pose::presets::BODY.contains(&key) {
            return Err(bad(format!("preset de corpo '{key}' desconhecido; disponíveis: {}", pose::presets::BODY.join(", "))));
        }
        self.pose_apply_body_preset(key);
        Ok(())
    }

    fn pose_hands_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        let side = match m.get("side").and_then(Value::as_str).unwrap_or("both") {
            "left" => HandSide::Left,
            "right" => HandSide::Right,
            "both" => HandSide::Both,
            o => return Err(bad(format!("side '{o}' desconhecido (left, right, both)"))),
        };
        let now = self.pose_now();
        let mut t = now.clone();
        let curls = if let Some(key) = m.get("preset").and_then(Value::as_str) {
            pose::presets::hand(key).ok_or_else(|| bad(format!("preset de mão '{key}' desconhecido; disponíveis: {}", pose::presets::HAND.join(", "))))?
        } else {
            let c = arr::<5>(m.get("curls").ok_or_else(|| bad("informe 'preset' ou 'curls' ([polegar, indicador, médio, anelar, mínimo] 0..1)"))?, "curls")?;
            if c.iter().any(|v| !(0.0..=1.0).contains(v)) {
                return Err(bad("'curls' deve estar em 0..1"));
            }
            c
        };
        if matches!(side, HandSide::Left | HandSide::Both) {
            t.hands.left = curls;
        }
        if matches!(side, HandSide::Right | HandSide::Both) {
            t.hands.right = curls;
        }
        self.pose_commit_to(t, PoseOp::Hand);
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ biblioteca de poses

    fn pose_library_op(&mut self, m: &Map<String, Value>) -> Result<(), BridgeError> {
        match str_of(m, "action")? {
            "list" => self.pose_refresh_library(),
            "save" => {
                let name = str_of(m, "name")?.trim().to_string();
                if name.is_empty() || name.chars().count() > 120 {
                    return Err(bad("'name' deve ter 1..120 caracteres"));
                }
                self.pose_ui.lib_name = name;
                if let Some(o) = opt_bool(m, "only_selected")? {
                    self.pose_ui.lib_only_selected = o;
                }
                self.pose_ui.preset_status.clear(); // só uma gravação bem-sucedida preenche o status
                self.pose_save_to_library();
                if self.pose_ui.preset_status.is_empty() {
                    return Err(BridgeError::failed(self.pose_ui.message.clone()));
                }
            }
            "apply" => {
                self.pose_refresh_library();
                let idx = match (m.get("index").and_then(Value::as_u64), m.get("name").and_then(Value::as_str)) {
                    (Some(i), None) => i as usize,
                    (None, Some(n)) => self.pose_ui.poses.iter().position(|c| c.name == n).ok_or_else(|| bad(format!("pose '{n}' não existe na Biblioteca (veja info.pose_assets)")))?,
                    _ => return Err(bad("informe 'index' OU 'name' da pose (veja info.pose_assets)")),
                };
                if idx >= self.pose_ui.poses.len() {
                    return Err(bad(format!("índice {idx} fora de 0..{}", self.pose_ui.poses.len().saturating_sub(1))));
                }
                if let Some(b) = m.get("blend") {
                    self.pose_ui.blend = num(b, "blend").and_then(|w| if (0.0..=1.5).contains(&w) { Ok(w) } else { Err(bad("'blend' deve ser 0..1.5")) })?;
                }
                if let Some(o) = opt_bool(m, "only_selected")? {
                    self.pose_ui.apply_only_selected = o;
                }
                self.pose_apply_asset(idx, opt_bool(m, "flipped")?.unwrap_or(false));
            }
            o => return Err(bad(format!("library.action '{o}' desconhecida (list, save, apply)"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bones_are_found_by_index_or_loose_name() {
        assert_eq!(bone_of(&json!(bone::HEAD)).unwrap(), bone::HEAD);
        assert_eq!(bone_of(&json!("head")).unwrap(), bone::HEAD);
        let name = BONE_NAMES[bone::L_ELBOW];
        assert_eq!(bone_of(&json!(name.to_uppercase())).unwrap(), bone::L_ELBOW);
        assert!(bone_of(&json!("cauda")).is_err());
        assert!(bone_of(&json!(99)).is_err());
        assert!(bone_of(&json!(1.5)).is_err());
        assert!(bones_of(&json!("head"), "b").is_err());
    }

    #[test]
    fn vectors_must_be_finite_and_sized() {
        assert!(vec3(&json!([0, 1, 2]), "v").is_ok());
        assert!(vec3(&json!([0, 1]), "v").is_err());
        assert!(vec3(&json!([0, 1, "x"]), "v").is_err());
        assert!(arr::<5>(&json!([0, 0, 0, 0]), "c").is_err());
        assert!(num(&json!(1e9), "n").is_err());
    }

    #[test]
    fn limbs_and_profiles_have_stable_names() {
        for l in Limb::ALL {
            assert_eq!(limb_of(l.key()).unwrap(), l);
        }
        assert!(limb_of("tail").is_err());
        assert_eq!(profile_name(LimitProfile::Anime), "anime");
        assert_eq!(side_name(HandSide::Both), "both");
    }
}
