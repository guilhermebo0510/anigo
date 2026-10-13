//! Animação em glTF 2.0 / VRM Animation (VRMA): importação com retarget para os 24 ossos do ANIGO e exportação do clip.
//!
//! * **Importar**: lê as `animations` de um `.glb`/`.gltf`/`.vrma`/`.vrm` (STEP, LINEAR com slerp e CUBICSPLINE, com
//!   *clamp* fora do intervalo [V: spec]), amostra cada quadro do projeto, retarget por direção de osso
//!   (`anigo_core::retarget`) e entrega um [`AnimationClip`] por animação. O mapa de ossos vem de
//!   `VRMC_vrm_animation.humanoid.humanBones` (VRMA), do `humanoid` de um VRM 1.0/0.x, ou dos nomes dos nós.
//! * **Exportar**: GLB com o esqueleto humanoide e um canal de rotação por osso animado (+ translação do quadril),
//!   amostrado a cada quadro do clip. No modo **VRMA** o esqueleto sai em T-pose (a spec exige o repouso "VRM T-pose"),
//!   com os nomes `hips`, `leftUpperArm`… e a extensão `VRMC_vrm_animation`; o clip é convertido para esse repouso com o
//!   mesmo retargeter da importação (por isso o resultado reimportado volta igual).
//!
//! Escala, expressões e `lookAt` do VRMA não são importados (a spec proíbe escala nos ossos humanoides; expressões e
//! olhar ficam para o avançado); o aviso aparece no relatório.

use std::collections::BTreeMap;
use std::path::Path;

use anigo_core::animation::Interp;
use anigo_core::ids::AnimationClipId;
use anigo_core::pose::{bone, PoseRig, PoseState, BONE_COUNT, BONE_NAMES, BONE_PARENTS};
use anigo_core::project::AnimationClip;
use anigo_core::retarget::{canonical_source_skeleton, poses_to_clip, source_frame_of_pose, t_pose_rig, BoneMap, ClipBuild, Retargeter, SrcFrame, SrcJoint, SrcSkeleton};
use base64::Engine;
use glam::{Mat4, Quat, Vec3};
use serde_json::{json, Value};

use crate::exporter::{export_gltf, ExportAnimation, ExportAnimationTrack, ExportNode, ExportScene};
use crate::gltf::{build_glb, parse_glb, parse_gltf, read_accessor_f32, GltfError, GltfErrorCode, ParsedGltf};

/// Quadros máximos amostrados por animação (barra arquivos hostis).
pub const MAX_IMPORT_FRAMES: usize = 20_000;

/// Um clip importado.
#[derive(Debug, Clone)]
pub struct ImportedClip {
    pub name: String,
    pub clip: AnimationClip,
    /// Duração no arquivo, em segundos.
    pub seconds: f32,
}

/// Resultado de uma importação.
#[derive(Debug, Clone, Default)]
pub struct GltfAnimationImport {
    pub clips: Vec<ImportedClip>,
    pub warnings: Vec<String>,
    /// Quantos ossos canônicos foram reconhecidos (de 23).
    pub mapped_bones: usize,
}

fn gerr(e: GltfError) -> String {
    e.to_string()
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn f32s(v: Option<&Value>, n: usize) -> Option<Vec<f32>> {
    let a = v?.as_array()?;
    if a.len() != n {
        return None;
    }
    a.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()
}

/// Lê as animações de `bytes` (GLB, VRM, VRMA ou `.gltf` JSON). `base_dir` resolve buffers externos de um `.gltf`.
pub fn import_gltf_animations(bytes: &[u8], base_dir: Option<&Path>, rig: &PoseRig, target_fps: f32) -> Result<GltfAnimationImport, String> {
    let target_fps = target_fps.clamp(1.0, 240.0);
    let (json, bin) = if bytes.starts_with(b"glTF") {
        parse_glb(bytes).map_err(gerr)?
    } else {
        (serde_json::from_slice::<Value>(bytes).map_err(|e| format!("JSON inválido: {e}"))?, Vec::new())
    };
    let base = base_dir.map(Path::to_path_buf);
    let parsed: ParsedGltf = parse_gltf(json, |_, uri| match uri {
        None => Ok(bin.clone()),
        Some(u) if u.starts_with("data:") => {
            let (_, data) = u.split_once(',').ok_or_else(|| GltfError::new(GltfErrorCode::MissingBuffer, "data URI inválida"))?;
            base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| GltfError::new(GltfErrorCode::MissingBuffer, format!("base64 inválido: {e}")))
        }
        Some(u) => {
            let dir = base.as_ref().ok_or_else(|| GltfError::new(GltfErrorCode::MissingBuffer, format!("buffer externo '{u}' sem pasta de origem")))?;
            // sem caminhos absolutos nem `..`: o buffer precisa estar dentro da pasta do arquivo
            let rel = Path::new(u);
            if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err(GltfError::new(GltfErrorCode::MissingBuffer, format!("caminho de buffer recusado '{u}'")));
            }
            std::fs::read(dir.join(rel)).map_err(|e| GltfError::new(GltfErrorCode::MissingBuffer, format!("{u}: {e}")))
        }
    })
    .map_err(gerr)?;
    let json = &parsed.json;
    let mut warnings = parsed.warnings.clone();

    // ---- nós -> juntas (ordem topológica)
    let nodes = arr(json, "nodes");
    if nodes.is_empty() {
        return Err("o arquivo não tem nós".into());
    }
    let mut parent_of: Vec<Option<usize>> = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in arr(n, "children") {
            if let Some(c) = c.as_u64().map(|c| c as usize).filter(|c| *c < nodes.len() && *c != i) {
                parent_of[c] = Some(i);
            }
        }
    }
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; nodes.len()];
    fn visit(i: usize, nodes: &[Value], seen: &mut [bool], order: &mut Vec<usize>, depth: usize) {
        if seen[i] || depth > 256 {
            return;
        }
        seen[i] = true;
        order.push(i);
        for c in nodes[i].get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(c) = c.as_u64().map(|c| c as usize).filter(|c| *c < nodes.len()) {
                visit(c, nodes, seen, order, depth + 1);
            }
        }
    }
    for i in 0..nodes.len() {
        if parent_of[i].is_none() {
            visit(i, nodes, &mut seen, &mut order, 0);
        }
    }
    for i in 0..nodes.len() {
        visit(i, nodes, &mut seen, &mut order, 0); // ciclos / órfãos
    }
    let mut joint_of: Vec<usize> = vec![usize::MAX; nodes.len()];
    let mut skel = SrcSkeleton::default();
    let mut scaled = false;
    for (j, &n) in order.iter().enumerate() {
        joint_of[n] = j;
        let node = &nodes[n];
        let (mut t, mut r) = (Vec3::ZERO, Quat::IDENTITY);
        if let Some(m) = f32s(node.get("matrix"), 16) {
            let (s, rr, tt) = Mat4::from_cols_slice(&m).to_scale_rotation_translation();
            if (s - Vec3::ONE).abs().max_element() > 1e-3 {
                scaled = true;
            }
            t = tt;
            r = rr;
        } else {
            if let Some(v) = f32s(node.get("translation"), 3) {
                t = Vec3::new(v[0], v[1], v[2]);
            }
            if let Some(v) = f32s(node.get("rotation"), 4) {
                r = Quat::from_xyzw(v[0], v[1], v[2], v[3]);
                r = if r.is_finite() && r.length_squared() > 1e-12 { r.normalize() } else { Quat::IDENTITY };
            }
            if let Some(v) = f32s(node.get("scale"), 3) {
                if (Vec3::new(v[0], v[1], v[2]) - Vec3::ONE).abs().max_element() > 1e-3 {
                    scaled = true;
                }
            }
        }
        skel.joints.push(SrcJoint {
            name: node.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            parent: parent_of[n].map(|p| joint_of[p]).filter(|p| *p != usize::MAX),
            rest_t: if t.is_finite() { t } else { Vec3::ZERO },
            rest_r: r,
        });
    }
    if scaled {
        warnings.push("há escala nos nós do esqueleto; ela é ignorada (a spec VRMA proíbe escala nos ossos humanoides)".into());
    }
    skel.validate()?;

    // ---- mapa de ossos
    let map = humanoid_map(json, &joint_of).unwrap_or_else(|| BoneMap::from_names(&skel));
    if map.mapped_count() < 6 {
        return Err(format!("não encontrei um esqueleto humanoide no arquivo (ossos reconhecidos: {} de 23)", map.mapped_count()));
    }
    let rt = Retargeter::new(&skel, &map, rig)?;
    warnings.extend(rt.warnings.iter().cloned());

    // ---- animações
    let anims = arr(json, "animations");
    if anims.is_empty() {
        return Err("o arquivo não tem animações".into());
    }
    let mut out = GltfAnimationImport { mapped_bones: map.mapped_count(), ..Default::default() };
    let rest_rot: Vec<Quat> = skel.joints.iter().map(|j| j.rest_r).collect();
    for (ai, anim) in anims.iter().enumerate() {
        let name = anim.get("name").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("Animação {}", ai + 1));
        let samplers = arr(anim, "samplers");
        let mut rot_tracks: BTreeMap<usize, Track> = BTreeMap::new();
        let mut pos_tracks: BTreeMap<usize, Track> = BTreeMap::new();
        let (mut all_step, mut any) = (true, false);
        let mut ignored = std::collections::BTreeSet::new();
        for ch in arr(anim, "channels") {
            let (Some(si), Some(node)) = (ch.get("sampler").and_then(Value::as_u64), ch.pointer("/target/node").and_then(Value::as_u64)) else { continue };
            let path = ch.pointer("/target/path").and_then(Value::as_str).unwrap_or("");
            let Some(sampler) = samplers.get(si as usize) else { continue };
            let node = node as usize;
            if node >= nodes.len() {
                continue;
            }
            let comps = match path {
                "rotation" => 4,
                "translation" => 3,
                other => {
                    ignored.insert(other.to_string());
                    continue;
                }
            };
            let interp = match sampler.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR") {
                "STEP" => GInterp::Step,
                "CUBICSPLINE" => GInterp::Cubic,
                _ => GInterp::Linear,
            };
            let (Some(inp), Some(outp)) = (sampler.get("input").and_then(Value::as_u64), sampler.get("output").and_then(Value::as_u64)) else { continue };
            let times = read_accessor_f32(&parsed, inp as usize).map_err(gerr)?;
            let values = read_accessor_f32(&parsed, outp as usize).map_err(gerr)?;
            let per = if interp == GInterp::Cubic { 3 * comps } else { comps };
            if times.is_empty() || values.len() != times.len() * per || times.iter().any(|t| !t.is_finite()) || values.iter().any(|v| !v.is_finite()) {
                warnings.push(format!("{name}: canal com dados inconsistentes ignorado"));
                continue;
            }
            if times.windows(2).any(|w| w[1] <= w[0]) {
                warnings.push(format!("{name}: tempos fora de ordem ignorados"));
                continue;
            }
            any = true;
            all_step &= interp == GInterp::Step;
            let track = Track { times, values, interp, comps };
            let j = joint_of[node];
            if path == "rotation" {
                rot_tracks.insert(j, track);
            } else {
                pos_tracks.insert(j, track);
            }
        }
        if !ignored.is_empty() {
            warnings.push(format!("{name}: canais ignorados ({})", ignored.into_iter().collect::<Vec<_>>().join(", ")));
        }
        if !any {
            warnings.push(format!("{name}: sem canais de rotação ou translação"));
            continue;
        }
        let seconds = rot_tracks.values().chain(pos_tracks.values()).filter_map(|t| t.times.last().copied()).fold(0.0f32, f32::max);
        let n_frames = ((seconds * target_fps).ceil() as usize + 1).max(1);
        if n_frames > MAX_IMPORT_FRAMES {
            return Err(format!("{name}: {n_frames} quadros a {target_fps} fps passa do limite de {MAX_IMPORT_FRAMES}"));
        }
        let mut poses: Vec<PoseState> = Vec::with_capacity(n_frames);
        for f in 0..n_frames {
            let t = (f as f32 / target_fps).min(seconds);
            let mut frame = SrcFrame { rot: rest_rot.clone(), pos: BTreeMap::new() };
            for (j, tr) in &rot_tracks {
                let v = tr.sample(t);
                let q = Quat::from_xyzw(v[0], v[1], v[2], v[3]);
                frame.rot[*j] = if q.length_squared() > 1e-12 { q.normalize() } else { Quat::IDENTITY };
            }
            for (j, tr) in &pos_tracks {
                let v = tr.sample(t);
                frame.pos.insert(*j, Vec3::new(v[0], v[1], v[2]));
            }
            poses.push(rt.pose(&frame));
        }
        let id = AnimationClipId::from_label(&name);
        let opts = ClipBuild { interp: if all_step { Interp::Step } else { Interp::Linear }, ..Default::default() };
        let mut clip = poses_to_clip(&poses, target_fps, target_fps, id, &name, opts);
        clip.duration_seconds = seconds;
        out.clips.push(ImportedClip { name, clip, seconds });
    }
    if out.clips.is_empty() {
        return Err("nenhuma animação utilizável no arquivo".into());
    }
    out.warnings = warnings;
    Ok(out)
}

/// Mapa de ossos declarado no arquivo: VRMA, VRM 1.0 ou VRM 0.x.
fn humanoid_map(json: &Value, joint_of: &[usize]) -> Option<BoneMap> {
    let mut entries: Vec<(String, usize)> = Vec::new();
    let ext = json.get("extensions")?;
    if let Some(hb) = ext.pointer("/VRMC_vrm_animation/humanoid/humanBones").or_else(|| ext.pointer("/VRMC_vrm/humanoid/humanBones")) {
        for (name, v) in hb.as_object()? {
            if let Some(n) = v.get("node").and_then(Value::as_u64) {
                entries.push((name.clone(), n as usize));
            }
        }
    } else if let Some(list) = ext.pointer("/VRM/humanoid/humanBones").and_then(Value::as_array) {
        for v in list {
            if let (Some(name), Some(n)) = (v.get("bone").and_then(Value::as_str), v.get("node").and_then(Value::as_u64)) {
                entries.push((name.to_string(), n as usize));
            }
        }
    }
    if entries.is_empty() {
        return None;
    }
    let mut map = BoneMap::empty();
    for (name, node) in entries {
        let Some(b) = anigo_core::retarget::canonical_bone_for_name(&name) else { continue };
        if b == bone::ROOT {
            continue;
        }
        if let Some(j) = joint_of.get(node).copied().filter(|j| *j != usize::MAX) {
            map.0[b] = Some(j);
        }
    }
    Some(map)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GInterp {
    Step,
    Linear,
    Cubic,
}

struct Track {
    times: Vec<f32>,
    values: Vec<f32>,
    interp: GInterp,
    comps: usize,
}

impl Track {
    /// Valor no tempo `t` (clamp fora do intervalo; slerp para rotações; hermite para CUBICSPLINE).
    fn sample(&self, t: f32) -> [f32; 4] {
        let c = self.comps;
        let stride = if self.interp == GInterp::Cubic { 3 * c } else { c };
        let value_at = |k: usize| -> &[f32] {
            let o = k * stride + if self.interp == GInterp::Cubic { c } else { 0 };
            &self.values[o..o + c]
        };
        let pack = |s: &[f32]| -> [f32; 4] {
            let mut o = [0.0, 0.0, 0.0, 1.0];
            o[..c].copy_from_slice(s);
            o
        };
        let n = self.times.len();
        if n == 1 || t <= self.times[0] {
            return pack(value_at(0));
        }
        if t >= self.times[n - 1] {
            return pack(value_at(n - 1));
        }
        let k = self.times.partition_point(|x| *x <= t) - 1;
        let (t0, t1) = (self.times[k], self.times[k + 1]);
        let u = (t - t0) / (t1 - t0);
        match self.interp {
            GInterp::Step => pack(value_at(k)),
            GInterp::Linear => {
                let (a, b) = (value_at(k), value_at(k + 1));
                if c == 4 {
                    anigo_core::animation::slerp([a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]], u)
                } else {
                    let mut o = [0.0, 0.0, 0.0, 1.0];
                    for i in 0..c {
                        o[i] = a[i] + (b[i] - a[i]) * u;
                    }
                    o
                }
            }
            GInterp::Cubic => {
                let td = t1 - t0;
                let (u2, u3) = (u * u, u * u * u);
                let mut o = [0.0, 0.0, 0.0, 1.0];
                for i in 0..c {
                    let vk = self.values[k * stride + c + i];
                    let bk = self.values[k * stride + 2 * c + i]; // tangente de saída de k
                    let vk1 = self.values[(k + 1) * stride + c + i];
                    let ak1 = self.values[(k + 1) * stride + i]; // tangente de entrada de k+1
                    o[i] = (2.0 * u3 - 3.0 * u2 + 1.0) * vk + td * (u3 - 2.0 * u2 + u) * bk + (-2.0 * u3 + 3.0 * u2) * vk1 + td * (u3 - u2) * ak1;
                }
                if c == 4 {
                    let q = Quat::from_xyzw(o[0], o[1], o[2], o[3]);
                    return if q.length_squared() > 1e-12 { q.normalize().to_array() } else { [0.0, 0.0, 0.0, 1.0] };
                }
                o
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Exportação
// ---------------------------------------------------------------------------------------------

/// Nome do osso humanoide do VRM para um osso canônico (`None` = fora do humanoide VRM).
pub fn vrm_humanoid_name(b: usize) -> Option<&'static str> {
    use bone::*;
    Some(match b {
        HIPS => "hips",
        SPINE => "spine",
        CHEST => "chest",
        UPPER_CHEST => "upperChest",
        NECK => "neck",
        HEAD => "head",
        L_CLAVICLE => "leftShoulder",
        L_SHOULDER => "leftUpperArm",
        L_ELBOW => "leftLowerArm",
        L_WRIST => "leftHand",
        R_CLAVICLE => "rightShoulder",
        R_SHOULDER => "rightUpperArm",
        R_ELBOW => "rightLowerArm",
        R_WRIST => "rightHand",
        L_THIGH => "leftUpperLeg",
        L_KNEE => "leftLowerLeg",
        L_ANKLE => "leftFoot",
        L_TOES => "leftToes",
        R_THIGH => "rightUpperLeg",
        R_KNEE => "rightLowerLeg",
        R_ANKLE => "rightFoot",
        R_TOES => "rightToes",
        _ => return None,
    })
}

/// Exporta o clip como GLB. `vrma = true` gera um VRM Animation (esqueleto em T-pose, nomes humanoides e a extensão).
pub fn export_gltf_animation(clip: &AnimationClip, fps: f32, rig: &PoseRig, vrma: bool) -> Result<Vec<u8>, String> {
    let fps = fps.clamp(1.0, 240.0);
    let (start, end) = clip.effective_range(fps);
    let frames = (end - start + 1).max(1) as usize;
    if frames > MAX_IMPORT_FRAMES {
        return Err(format!("{frames} quadros passa do limite de {MAX_IMPORT_FRAMES}"));
    }
    let dst_rig = if vrma { t_pose_rig(rig) } else { rig.clone() };
    // poses por quadro (loop/hold do clip valem: exporta o que o playback mostra)
    let src_poses: Vec<PoseState> = (0..frames).map(|i| clip.evaluate(fps, (start + i as i32) as f32).apply_to_pose(&PoseState::default())).collect();
    let poses: Vec<PoseState> = if vrma {
        let skel = canonical_source_skeleton(rig);
        let map = BoneMap::from_names(&skel);
        let rt = Retargeter::new(&skel, &map, &dst_rig)?;
        src_poses.iter().map(|p| rt.pose(&source_frame_of_pose(p, rig))).collect()
    } else {
        src_poses
    };

    // ---- nós
    let included: Vec<usize> = (0..BONE_COUNT).filter(|b| !vrma || (*b != bone::ROOT && *b != bone::PELVIS)).collect();
    let node_of = |b: usize| included.iter().position(|x| *x == b);
    let mut nodes: Vec<ExportNode> = Vec::new();
    for &b in &included {
        let local = match BONE_PARENTS[b].filter(|p| node_of(*p).is_some()) {
            Some(_) => dst_rig.offset(b),
            None => dst_rig.bind[b],
        };
        nodes.push(ExportNode {
            name: Some(if vrma { vrm_humanoid_name(b).unwrap_or(BONE_NAMES[b]).to_string() } else { BONE_NAMES[b].to_string() }),
            translation: Some(local.to_array()),
            rotation: Some([0.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        });
    }
    for &b in &included {
        if let Some(p) = BONE_PARENTS[b].and_then(node_of) {
            let me = node_of(b).unwrap_or(0) as u64;
            nodes[p].children.push(me);
        }
    }
    let root_nodes: Vec<u64> = included.iter().enumerate().filter(|(_, b)| BONE_PARENTS[**b].and_then(node_of).is_none()).map(|(i, _)| i as u64).collect();

    // ---- trilhas
    let times: Vec<f32> = (0..frames).map(|i| i as f32 / fps).collect();
    let mut tracks: Vec<ExportAnimationTrack> = Vec::new();
    for &b in &included {
        if poses.iter().all(|p| p.rotation(b).angle_between(Quat::IDENTITY) < 1e-5) {
            continue;
        }
        let mut values: Vec<f32> = Vec::with_capacity(frames * 4);
        let mut prev: Option<Quat> = None;
        for p in &poses {
            let mut q = p.rotation(b);
            if prev.is_some_and(|pq| pq.dot(q) < 0.0) {
                q = -q; // continuidade de hemisfério: o slerp do leitor toma o caminho curto
            }
            prev = Some(q);
            values.extend_from_slice(&q.to_array());
        }
        tracks.push(ExportAnimationTrack { node: node_of(b).unwrap_or(0) as u64, path: "rotation".into(), times: times.clone(), values, step: false });
    }
    if poses.iter().any(|p| p.hips_translation.iter().any(|c| c.abs() > 1e-6)) {
        let hips_node = node_of(bone::HIPS).unwrap_or(0);
        let base = nodes[hips_node].translation.unwrap_or([0.0; 3]);
        let mut values = Vec::with_capacity(frames * 3);
        for p in &poses {
            for i in 0..3 {
                values.push(base[i] + p.hips_translation[i]);
            }
        }
        tracks.push(ExportAnimationTrack { node: hips_node as u64, path: "translation".into(), times: times.clone(), values, step: false });
    }
    if tracks.is_empty() {
        return Err("o clip não tem canais de corpo para exportar".into());
    }
    let scene = ExportScene {
        name: Some(clip.name.clone()),
        root_nodes,
        nodes,
        animations: vec![ExportAnimation { name: Some(clip.name.clone()), tracks }],
        ..Default::default()
    };
    let (mut json, bin) = export_gltf(&scene, None);
    if vrma {
        let mut human = serde_json::Map::new();
        for &b in &included {
            if let Some(n) = vrm_humanoid_name(b) {
                human.insert(n.to_string(), json!({ "node": node_of(b).unwrap_or(0) }));
            }
        }
        if let Some(obj) = json.as_object_mut() {
            obj.entry("extensionsUsed").or_insert_with(|| json!([])).as_array_mut().map(|a| a.push(json!("VRMC_vrm_animation")));
            obj.insert("extensions".into(), json!({ "VRMC_vrm_animation": { "specVersion": "1.0", "humanoid": { "humanBones": Value::Object(human) } } }));
        }
    }
    Ok(build_glb(&json, &bin))
}

#[cfg(test)]
mod tests;
