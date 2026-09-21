//! ANIGO — extensões VRM 1.0 (lado Rust, espelho de `src/services/vrm/vrm1.ts`).
//!
//! Deserializa as quatro extensões oficiais:
//!  - `VRMC_vrm` — meta, humanoid (mapeamento de ossos), expressões;
//!  - `VRMC_materials_mtoon` — parâmetros MToon por material;
//!  - `VRMC_springBone` — rig secundário (molas + colisores);
//!  - `VRMC_node_constraint` — limites por nó.
//!
//! Parse tolerante a omissões opcionais (defaults da spec), rigoroso no que
//! existe (`Vrm1Error` com código estável).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::gltf::ParsedGltf;

// ---------------------------------------------------------------------------
// Tipos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1Meta {
    pub title: String,
    pub author: String,
    pub version: String,
    pub year: i64,
    pub license_name: Option<String>,
    pub contact_information: String,
    pub metadata: Vec<(String, String)>,
    pub reference: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Vrm1Humanoid {
    /// bone name → índice do nó glTF (None = mapeamento ausente).
    pub human_bones: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub struct Vrm1ExpressionBinding {
    /// Índice do morph target na malha facial.
    pub blend_shape: u64,
    pub isolated: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Vrm1Expression {
    /// 16 presets obrigatórios (name → binding).
    pub preset: Map<String, Value>,
    pub custom: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Vrm1SpringJoint {
    pub node: u64,
    pub distance: f32,
    pub hit_radius: f32,
    pub gravity_power: f32,
    pub gravity_dir: [f32; 3],
    pub spring_stiffness: f32,
    pub spring_damping: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1SpringGroup {
    pub name: String,
    pub center: Vrm1SpringJoint,
    pub joints: Vec<Vrm1SpringJoint>,
    pub collider_groups: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1SphereCollider {
    pub group: u64,
    pub node: u64,
    pub radius: f32,
    pub offset: [f32; 3],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1CapsuleCollider {
    pub group: u64,
    pub node: u64,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub radius: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1Colliders {
    pub spheres: Vec<Vrm1SphereCollider>,
    pub capsules: Vec<Vrm1CapsuleCollider>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Vrm1SpringBone {
    pub groups: Vec<Vrm1SpringGroup>,
    pub colliders: Vrm1Colliders,
}

/// MToon (VRMC_materials_mtoon) — parâmetros por material (defaults da spec).
#[derive(Debug, Clone, Serialize)]
pub struct Vrm1MToon {
    pub main_tex: Option<u64>,
    pub sub_emission_color: [f32; 4],
    pub sub_emission_texture: Option<u64>,
    pub multiply: [f32; 4],
    pub shadow_color: [f32; 4],
    pub shade_shift: f32,
    pub shade_toony: f32,
    pub light_color: [f32; 3],
    pub rim_color: [f32; 4],
    pub rim_power: f32,
    pub rim_light: bool,
    pub rim_light_color: [f32; 4],
    pub light_direction: [f32; 3],
    pub specular_color: [f32; 4],
    pub specular_power: f32,
    pub use_smooth: bool,
    pub smooth_color: [f32; 4],
    pub use_sphere: bool,
    pub sphere_mode: Vrm1SphereMode,
    pub use_mat_cap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Vrm1SphereMode {
    Normal,
    Additive,
}

impl Default for Vrm1MToon {
    fn default() -> Self {
        Self {
            main_tex: None,
            sub_emission_color: [1.0, 1.0, 1.0, 1.0],
            sub_emission_texture: None,
            multiply: [1.0, 1.0, 1.0, 1.0],
            shadow_color: [0.718, 0.831, 1.0, 1.0],
            shade_shift: 0.0,
            shade_toony: 1.0,
            light_color: [1.0, 1.0, 1.0],
            rim_color: [1.0, 1.0, 1.0, 0.0],
            rim_power: 1.0,
            rim_light: false,
            rim_light_color: [1.0, 1.0, 1.0, 1.0],
            light_direction: [0.0, 0.0, 1.0],
            specular_color: [1.0, 1.0, 1.0, 1.0],
            specular_power: 1.0,
            use_smooth: false,
            smooth_color: [0.7, 0.7, 0.7, 0.5],
            use_sphere: false,
            sphere_mode: Vrm1SphereMode::Normal,
            use_mat_cap: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Vrm1NodeConstraintBound {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Vrm1NodeConstraint {
    pub rotate_offset: Option<[f32; 3]>,
    pub rotate_limit: Option<Vrm1NodeConstraintBound>,
    pub translate_offset: Option<[f32; 3]>,
    pub translate_limit: Option<Vrm1NodeConstraintBound>,
    pub scale_offset: Option<[f32; 3]>,
    pub scale_limit: Option<Vrm1NodeConstraintBound>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ParsedVrm1 {
    pub meta: Vrm1Meta,
    pub humanoid: Vrm1Humanoid,
    pub expression: Vrm1Expression,
    pub spring_bone: Option<Vrm1SpringBone>,
    /// Índice do material → parâmetros MToon (None = material PBR padrão).
    pub materials: Vec<Option<Vrm1MToon>>,
    /// Índice do nó → constraint (None = sem constraint).
    pub node_constraints: Vec<Option<Vrm1NodeConstraint>>,
    pub warnings: Vec<String>,
}

impl ParsedVrm1 {
    /// Índice do nó para um osso humanoid (None = osso sem nó mapeado).
    pub fn human_bone_node(&self, bone: &str) -> Option<Option<u64>> {
        self.humanoid
            .human_bones
            .get(bone)
            .map(|entry| entry.get("node").and_then(Value::as_u64))
    }
}

// ---------------------------------------------------------------------------
// Erros
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Vrm1ErrorCode {
    NotVrm,
    BadVrmExtension,
    BadMeta,
    BadHumanoid,
    BadExpression,
    BadMToon,
    BadSpringBone,
    BadNodeConstraint,
    UnknownError,
}

impl Vrm1ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Vrm1ErrorCode::NotVrm => "NOT_VRM",
            Vrm1ErrorCode::BadVrmExtension => "BAD_VRM_EXTENSION",
            Vrm1ErrorCode::BadMeta => "BAD_META",
            Vrm1ErrorCode::BadHumanoid => "BAD_HUMANOID",
            Vrm1ErrorCode::BadExpression => "BAD_EXPRESSION",
            Vrm1ErrorCode::BadMToon => "BAD_MTOON",
            Vrm1ErrorCode::BadSpringBone => "BAD_SPRINGBONE",
            Vrm1ErrorCode::BadNodeConstraint => "BAD_NODE_CONSTRAINT",
            Vrm1ErrorCode::UnknownError => "UNKNOWN_ERROR",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub struct Vrm1Error {
    pub code: Vrm1ErrorCode,
    pub message: String,
}

impl std::fmt::Display for Vrm1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[vrm1 {}] {}", self.code.as_str(), self.message)
    }
}

fn fail(code: Vrm1ErrorCode, message: impl Into<String>) -> Vrm1Error {
    Vrm1Error { code, message: message.into() }
}

// ---------------------------------------------------------------------------
// Helpers de leitura tolerante
// ---------------------------------------------------------------------------

pub const VRM1_EXPRESSION_PRESETS: [&str; 16] = [
    "neutral", "angry", "sad", "happy", "relaxed", "aha", "fun", "sleepy", "surprised", "upset", "ajishii",
    "akubou", "airy", "frightened", "disgusted", "confused",
];

fn as_number(value: Option<&Value>, fallback: f64, where_: &str) -> Result<f64, Vrm1Error> {
    match value {
        None => Ok(fallback),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| fail(Vrm1ErrorCode::BadMToon, format!("{where_}: número esperado"))),
    }
}

fn as_bool(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

fn as_vec3(value: Option<&Value>, fallback: [f32; 3], where_: &str) -> Result<[f32; 3], Vrm1Error> {
    match value {
        None => Ok(fallback),
        Some(v) => {
            let Some(arr) = v.as_array() else {
                return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}: vetor de 3 esperado")));
            };
            if arr.len() != 3 {
                return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}: vetor de 3 esperado")));
            }
            let mut out = [0f32; 3];
            for (i, component) in arr.iter().enumerate() {
                out[i] = component
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| fail(Vrm1ErrorCode::BadMToon, format!("{where_}: componente não numérico")))? as f32;
            }
            Ok(out)
        }
    }
}

fn as_vec4(value: Option<&Value>, fallback: [f32; 4], where_: &str) -> Result<[f32; 4], Vrm1Error> {
    match value {
        None => Ok(fallback),
        Some(v) => {
            let Some(arr) = v.as_array() else {
                return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}: vetor de 4 esperado")));
            };
            if arr.len() != 4 {
                return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}: vetor de 4 esperado")));
            }
            let mut out = [0f32; 4];
            for (i, component) in arr.iter().enumerate() {
                out[i] = component
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| fail(Vrm1ErrorCode::BadMToon, format!("{where_}: componente não numérico")))? as f32;
            }
            Ok(out)
        }
    }
}

fn as_node(value: Option<&Value>, where_: &str) -> Result<Option<u64>, Vrm1Error> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| fail(Vrm1ErrorCode::BadHumanoid, format!("{where_}: node deve ser inteiro ou null"))),
    }
}

fn as_texture_index(value: Option<&Value>, where_: &str) -> Result<Option<u64>, Vrm1Error> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(v) => v
            .get("index")
            .and_then(Value::as_u64)
            .map(Some)
            .ok_or_else(|| fail(Vrm1ErrorCode::BadMToon, format!("{where_}: texture index inválido"))),
    }
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// `true` quando o documento declara a extensão `VRMC_vrm` (um .vrm de fato).
pub fn is_vrm_document(json: &Value) -> bool {
    let declared = |key: &str| -> bool {
        json.pointer(&format!("/asset/{key}"))
            .and_then(Value::as_array)
            .map(|arr| arr.iter().any(|v| v.as_str() == Some("VRMC_vrm")))
            .unwrap_or(false)
    };
    declared("extensionsUsed") || declared("extensionsRequired")
}

fn parse_meta(raw: Option<&Value>, warnings: &mut Vec<String>) -> Result<Vrm1Meta, Vrm1Error> {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return Err(fail(Vrm1ErrorCode::BadMeta, "VRMC_vrm.meta ausente"));
    };
    let title = raw.get("title").and_then(Value::as_str).unwrap_or("");
    let author = raw.get("author").and_then(Value::as_str).unwrap_or("");
    if title.is_empty() {
        warnings.push("VRM meta.title ausente — usando 'Untitled VRM'".to_string());
    }
    if author.is_empty() {
        warnings.push("VRM meta.author ausente".to_string());
    }
    let mut metadata: Vec<(String, String)> = Vec::new();
    if let Some(arr) = raw.get("metadata").and_then(Value::as_array) {
        for entry in arr {
            if let (Some(t), Some(v)) = (
                entry.get("type").and_then(Value::as_str),
                entry.get("value").and_then(Value::as_str),
            ) {
                metadata.push((t.to_string(), v.to_string()));
            }
        }
    }
    Ok(Vrm1Meta {
        title: if title.is_empty() { "Untitled VRM".to_string() } else { title.to_string() },
        author: author.to_string(),
        version: raw.get("version").and_then(Value::as_str).unwrap_or("1.0.0").to_string(),
        year: raw.get("year").and_then(Value::as_i64).unwrap_or(0),
        license_name: raw
            .get("license")
            .and_then(Value::as_object)
            .and_then(|l| l.get("name"))
            .and_then(Value::as_str)
            .map(String::from),
        contact_information: raw.get("contactInformation").and_then(Value::as_str).unwrap_or("").to_string(),
        metadata,
        reference: raw.get("reference").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

fn parse_humanoid(raw: Option<&Value>) -> Result<Vrm1Humanoid, Vrm1Error> {
    let Some(bones) = raw
        .and_then(Value::as_object)
        .and_then(|o| o.get("humanBones"))
        .and_then(Value::as_object)
    else {
        return Err(fail(Vrm1ErrorCode::BadHumanoid, "VRMC_vrm.humanoid.humanBones ausente"));
    };
    for (bone_name, entry) in bones {
        let Some(obj) = entry.as_object() else {
            return Err(fail(
                Vrm1ErrorCode::BadHumanoid,
                format!("humanBones.{bone_name}: entrada inválida (esperado objeto com 'node')"),
            ));
        };
        if !obj.contains_key("node") {
            return Err(fail(
                Vrm1ErrorCode::BadHumanoid,
                format!("humanBones.{bone_name}: campo 'node' ausente"),
            ));
        }
    }
    Ok(Vrm1Humanoid {
        human_bones: Map::from_iter(bones.iter().map(|(k, v)| (k.clone(), v.clone()))),
    })
}

fn parse_expression(raw: Option<&Value>) -> Result<Vrm1Expression, Vrm1Error> {
    let Some(preset_raw) = raw
        .and_then(Value::as_object)
        .and_then(|o| o.get("preset"))
        .and_then(Value::as_object)
    else {
        return Err(fail(Vrm1ErrorCode::BadExpression, "VRMC_vrm.expression.preset ausente"));
    };
    let mut preset = Map::new();
    for preset_name in VRM1_EXPRESSION_PRESETS {
        let Some(entry) = preset_raw.get(preset_name).and_then(Value::as_object) else {
            return Err(fail(
                Vrm1ErrorCode::BadExpression,
                format!("expression.preset.{preset_name} ausente (os 16 presets são obrigatórios)"),
            ));
        };
        let blend_shape = entry
            .get("blendShape")
            .and_then(Value::as_u64)
            .ok_or_else(|| fail(Vrm1ErrorCode::BadExpression, format!("expression.preset.{preset_name}.blendShape inválido")))?;
        preset.insert(
            preset_name.to_string(),
            Value::Object(Map::from_iter([
                ("blendShape".to_string(), Value::from(blend_shape)),
                ("isolated".to_string(), Value::from(entry.get("isolated").and_then(Value::as_bool).unwrap_or(false))),
            ])),
        );
    }
    let mut custom = Map::new();
    if let Some(custom_raw) = raw.and_then(Value::as_object).and_then(|o| o.get("custom")).and_then(Value::as_object) {
        for (name, entry) in custom_raw {
            let Some(obj) = entry.as_object() else {
                return Err(fail(Vrm1ErrorCode::BadExpression, format!("expression.custom.{name} inválido")));
            };
            let Some(blend_shape) = obj.get("blendShape").and_then(Value::as_u64) else {
                return Err(fail(Vrm1ErrorCode::BadExpression, format!("expression.custom.{name}.blendShape inválido")));
            };
            custom.insert(
                name.clone(),
                Value::Object(Map::from_iter([
                    ("blendShape".to_string(), Value::from(blend_shape)),
                    ("isolated".to_string(), Value::from(obj.get("isolated").and_then(Value::as_bool).unwrap_or(false))),
                ])),
            );
        }
    }
    Ok(Vrm1Expression { preset, custom })
}

fn parse_mtoon(raw: Option<&Value>, where_: &str) -> Result<Vrm1MToon, Vrm1Error> {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}: bloco VRMC_materials_mtoon inválido")));
    };
    let sphere_mode = match raw.get("sphereMode").and_then(Value::as_str) {
        None => Vrm1SphereMode::Normal,
        Some("additive") => Vrm1SphereMode::Additive,
        Some("normal") => Vrm1SphereMode::Normal,
        Some(other) => {
            return Err(fail(Vrm1ErrorCode::BadMToon, format!("{where_}.sphereMode: '{other}' inválido")))
        }
    };
    Ok(Vrm1MToon {
        main_tex: as_texture_index(raw.get("mainTex"), &format!("{where_}.mainTex"))?,
        sub_emission_color: as_vec4(raw.get("subEmission"), [1.0, 1.0, 1.0, 1.0], &format!("{where_}.subEmission"))?,
        sub_emission_texture: as_texture_index(raw.get("subEmissionTexture"), &format!("{where_}.subEmissionTexture"))?,
        multiply: as_vec4(raw.get("multiply"), [1.0, 1.0, 1.0, 1.0], &format!("{where_}.multiply"))?,
        shadow_color: as_vec4(raw.get("shadowColor"), [0.718, 0.831, 1.0, 1.0], &format!("{where_}.shadowColor"))?,
        shade_shift: as_number(raw.get("shadeShift"), 0.0, &format!("{where_}.shadeShift"))? as f32,
        shade_toony: as_number(raw.get("shadeToony"), 1.0, &format!("{where_}.shadeToony"))? as f32,
        light_color: as_vec3(raw.get("lightColor"), [1.0, 1.0, 1.0], &format!("{where_}.lightColor"))?,
        rim_color: as_vec4(raw.get("rimColor"), [1.0, 1.0, 1.0, 0.0], &format!("{where_}.rimColor"))?,
        rim_power: as_number(raw.get("rimPower"), 1.0, &format!("{where_}.rimPower"))? as f32,
        rim_light: as_bool(raw.get("rimLight"), false),
        rim_light_color: as_vec4(raw.get("rimLightColor"), [1.0, 1.0, 1.0, 1.0], &format!("{where_}.rimLightColor"))?,
        light_direction: as_vec3(raw.get("lightDirection"), [0.0, 0.0, 1.0], &format!("{where_}.lightDirection"))?,
        specular_color: as_vec4(raw.get("specularColor"), [1.0, 1.0, 1.0, 1.0], &format!("{where_}.specularColor"))?,
        specular_power: as_number(raw.get("specularPower"), 1.0, &format!("{where_}.specularPower"))? as f32,
        use_smooth: as_bool(raw.get("useSmooth"), false),
        smooth_color: as_vec4(raw.get("smoothColor"), [0.7, 0.7, 0.7, 0.5], &format!("{where_}.smoothColor"))?,
        use_sphere: as_bool(raw.get("useSphere"), false),
        sphere_mode,
        use_mat_cap: as_bool(raw.get("useMatCap"), false),
    })
}

fn parse_spring_joint(raw: Option<&Value>, where_: &str) -> Result<Vrm1SpringJoint, Vrm1Error> {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("{where_}: joint inválido")));
    };
    let node = raw
        .get("node")
        .and_then(Value::as_u64)
        .ok_or_else(|| fail(Vrm1ErrorCode::BadSpringBone, format!("{where_}.node ausente")))?;
    let distance = as_number(raw.get("distance"), 0.0, &format!("{where_}.distance"))?;
    if distance < 0.0 {
        return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("{where_}.distance deve ser >= 0")));
    }
    Ok(Vrm1SpringJoint {
        node,
        distance: distance as f32,
        hit_radius: as_number(raw.get("hitRadius"), 0.05, &format!("{where_}.hitRadius"))? as f32,
        gravity_power: as_number(raw.get("gravityPower"), 0.0, &format!("{where_}.gravityPower"))? as f32,
        gravity_dir: as_vec3(raw.get("gravityDir"), [0.0, -1.0, 0.0], &format!("{where_}.gravityDir"))?,
        spring_stiffness: as_number(raw.get("springStiffness"), 0.5, &format!("{where_}.springStiffness"))? as f32,
        spring_damping: as_number(raw.get("springDamping"), 0.5, &format!("{where_}.springDamping"))? as f32,
    })
}

fn parse_spring_bone(raw: Option<&Value>, node_count: usize, warnings: &mut Vec<String>) -> Result<Vrm1SpringBone, Vrm1Error> {
    let Some(rig) = raw
        .and_then(Value::as_object)
        .and_then(|o| o.get("secondaryRig"))
        .and_then(Value::as_object)
    else {
        return Err(fail(Vrm1ErrorCode::BadSpringBone, "VRMC_springBone.secondaryRig ausente"));
    };
    let check_node = |node: u64, where_: &str| {
        if node as usize >= node_count {
            warnings.push(format!("springBone {where_}.node={node} fora da faixa — mola será ignorada em runtime"));
        }
    };
    let mut groups: Vec<Vrm1SpringGroup> = Vec::new();
    if let Some(groups_raw) = rig.get("groups").and_then(Value::as_array) {
        for (i, group_raw) in groups_raw.iter().enumerate() {
            let Some(group_raw) = group_raw.as_object() else {
                return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("groups[{i}] inválido")));
            };
            let center = parse_spring_joint(group_raw.get("center"), &format!("groups[{i}].center"))?;
            check_node(center.node, &format!("groups[{i}].center"));
            let mut joints: Vec<Vrm1SpringJoint> = Vec::new();
            if let Some(joints_raw) = group_raw.get("joints").and_then(Value::as_array) {
                for (j, joint_raw) in joints_raw.iter().enumerate() {
                    let joint = parse_spring_joint(Some(joint_raw), &format!("groups[{i}].joints[{j}]"))?;
                    check_node(joint.node, &format!("groups[{i}].joints[{j}]"));
                    joints.push(joint);
                }
            }
            let collider_groups: Vec<u64> = group_raw
                .get("colliderGroups")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            groups.push(Vrm1SpringGroup {
                name: group_raw.get("name").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("group_{i}")),
                center,
                joints,
                collider_groups,
            });
        }
    }
    let mut colliders = Vrm1Colliders::default();
    let colliders_value = rig.get("colliders");
    let colliders_raw: Vec<&Value> = match colliders_value {
        Some(Value::Array(arr)) => arr.iter().collect(),
        Some(v) if v.is_object() => vec![v],
        _ => vec![],
    };
    for (i, collider_raw) in colliders_raw.iter().enumerate() {
        let Some(collider_raw) = collider_raw.as_object() else {
            return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("colliders[{i}] inválido")));
        };
        if let Some(spheres) = collider_raw.get("spheres").and_then(Value::as_array) {
            for (j, sphere_raw) in spheres.iter().enumerate() {
                let Some(sphere_raw) = sphere_raw.as_object() else {
                    return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("colliders[{i}].spheres[{j}] inválido")));
                };
                let node = sphere_raw.get("node").and_then(Value::as_u64).unwrap_or(0);
                check_node(node, &format!("colliders[{i}].spheres[{j}]"));
                colliders.spheres.push(Vrm1SphereCollider {
                    group: sphere_raw.get("group").and_then(Value::as_u64).unwrap_or(0),
                    node,
                    radius: sphere_raw.get("radius").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    offset: as_vec3(sphere_raw.get("offset"), [0.0, 0.0, 0.0], &format!("colliders[{i}].spheres[{j}].offset"))?,
                });
            }
        }
        if let Some(capsules) = collider_raw.get("capsules").and_then(Value::as_array) {
            for (j, capsule_raw) in capsules.iter().enumerate() {
                let Some(capsule_raw) = capsule_raw.as_object() else {
                    return Err(fail(Vrm1ErrorCode::BadSpringBone, format!("colliders[{i}].capsules[{j}] inválido")));
                };
                let node = capsule_raw.get("node").and_then(Value::as_u64).unwrap_or(0);
                check_node(node, &format!("colliders[{i}].capsules[{j}]"));
                colliders.capsules.push(Vrm1CapsuleCollider {
                    group: capsule_raw.get("group").and_then(Value::as_u64).unwrap_or(0),
                    node,
                    from: as_vec3(capsule_raw.get("from"), [0.0, 0.0, 0.0], &format!("colliders[{i}].capsules[{j}].from"))?,
                    to: as_vec3(capsule_raw.get("to"), [0.0, 0.0, 0.0], &format!("colliders[{i}].capsules[{j}].to"))?,
                    radius: capsule_raw.get("radius").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                });
            }
        }
    }
    Ok(Vrm1SpringBone { groups, colliders })
}

fn parse_node_constraint(raw: Option<&Value>, node_index: usize) -> Result<Vrm1NodeConstraint, Vrm1Error> {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return Err(fail(Vrm1ErrorCode::BadNodeConstraint, format!("nodes[{node_index}]: constraint inválido")));
    };
    let bound = |key: &str| -> Result<Option<Vrm1NodeConstraintBound>, Vrm1Error> {
        let Some(value) = raw.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let Some(value) = value.as_object() else {
            return Err(fail(Vrm1ErrorCode::BadNodeConstraint, format!("{key}: bound inválido")));
        };
        Ok(Some(Vrm1NodeConstraintBound {
            min: as_vec3(value.get("min"), [0.0, 0.0, 0.0], &format!("{key}.min"))?,
            max: as_vec3(value.get("max"), [0.0, 0.0, 0.0], &format!("{key}.max"))?,
        }))
    };
    let offset = |key: &str| -> Result<Option<[f32; 3]>, Vrm1Error> {
        match raw.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(as_vec3(Some(v), [0.0, 0.0, 0.0], key)?)),
        }
    };
    Ok(Vrm1NodeConstraint {
        rotate_offset: offset("rotateOffset")?,
        rotate_limit: bound("rotateLimit")?,
        translate_offset: offset("translateOffset")?,
        translate_limit: bound("translateLimit")?,
        scale_offset: offset("scaleOffset")?,
        scale_limit: bound("scaleLimit")?,
    })
}

// ---------------------------------------------------------------------------
// API principal
// ---------------------------------------------------------------------------

/// Extrai a camada VRM 1.0 de um documento glTF já parseado.
/// Lança `Vrm1Error` quando o documento não é VRM 1.0 ou está malformado.
pub fn parse_vrm1(model: &ParsedGltf) -> Result<ParsedVrm1, Vrm1Error> {
    if !is_vrm_document(&model.json) {
        return Err(fail(Vrm1ErrorCode::NotVrm, "documento não declara a extensão VRMC_vrm (não é um .vrm)"));
    }
    let mut warnings = model.warnings.clone();
    let Some(vrm_raw) = model.json.pointer("/extensions/VRMC_vrm").and_then(Value::as_object) else {
        return Err(fail(Vrm1ErrorCode::BadVrmExtension, "extensões glTF sem bloco VRMC_vrm"));
    };

    let meta = parse_meta(vrm_raw.get("meta"), &mut warnings)?;
    let humanoid = parse_humanoid(vrm_raw.get("humanoid"))?;
    let expression = parse_expression(vrm_raw.get("expression"))?;
    let spring_bone = if vrm_raw.get("secondary").is_some() || model.json.pointer("/extensions/VRMC_springBone").is_some() {
        let raw = vrm_raw
            .get("secondary")
            .cloned()
            .or_else(|| model.json.pointer("/extensions/VRMC_springBone").cloned());
        let node_count = model.json.get("nodes").and_then(Value::as_array).map(|n| n.len()).unwrap_or(0);
        Some(parse_spring_bone(raw.as_ref(), node_count, &mut warnings)?)
    } else {
        None
    };

    let mut materials: Vec<Option<Vrm1MToon>> = Vec::new();
    if let Some(materials_raw) = model.json.get("materials").and_then(Value::as_array) {
        for material in materials_raw {
            let mtoon_raw = material.pointer("/extensions/VRMC_materials_mtoon");
            match mtoon_raw {
                Some(raw) => materials.push(Some(parse_mtoon(Some(raw), "material")?)),
                None => materials.push(None),
            }
        }
    }

    let mut node_constraints: Vec<Option<Vrm1NodeConstraint>> = Vec::new();
    if let Some(nodes) = model.json.get("nodes").and_then(Value::as_array) {
        for (i, node) in nodes.iter().enumerate() {
            match node.pointer("/extensions/VRMC_node_constraint") {
                Some(raw) => node_constraints.push(Some(parse_node_constraint(Some(raw), i)?)),
                None => node_constraints.push(None),
            }
        }
    }

    Ok(ParsedVrm1 {
        meta,
        humanoid,
        expression,
        spring_bone,
        materials,
        node_constraints,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Mapeamento MToon → material anime do ANIGO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MToonMaterialMapping {
    pub base_color: [f32; 4],
    pub shade_color: [f32; 4],
    pub shadow_threshold: f32,
    pub toon_steps: f32,
    pub rim_color: [f32; 4],
    pub rim_intensity: f32,
    pub rim_spread: f32,
    pub specular_color: [f32; 4],
    pub spec_intensity: f32,
    pub spec_power: f32,
    pub emission_color: [f32; 4],
    pub emission_intensity: f32,
    pub main_texture: Option<u64>,
    pub use_sphere: bool,
    pub use_mat_cap: bool,
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Convenção VRoid: base ← baseColorFactor × multiply; shade ← base × shadowColor;
/// threshold ← 0.5 + shadeShift × 0.5; toon_steps ← 2 quando shadeToony < 0.5.
pub fn map_mtoon_to_anime_material(material: &Value, mtoon: Option<&Vrm1MToon>) -> MToonMaterialMapping {
    let base_factor = material
        .pointer("/pbrMetallicRoughness/baseColorFactor")
        .and_then(Value::as_array)
        .map(|arr| {
            [
                arr.get(0).and_then(Value::as_f64).unwrap_or(1.0) as f32,
                arr.get(1).and_then(Value::as_f64).unwrap_or(1.0) as f32,
                arr.get(2).and_then(Value::as_f64).unwrap_or(1.0) as f32,
                arr.get(3).and_then(Value::as_f64).unwrap_or(1.0) as f32,
            ]
        })
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let emission_intensity = if mtoon.is_some() { 1.0 } else { 0.0 };
    let mtoon = mtoon.unwrap_or(&Vrm1MToon::default());
    let base_color = [
        base_factor[0] * mtoon.multiply[0],
        base_factor[1] * mtoon.multiply[1],
        base_factor[2] * mtoon.multiply[2],
        base_factor[3] * mtoon.multiply[3],
    ];
    let shade_color = [
        base_color[0] * mtoon.shadow_color[0],
        base_color[1] * mtoon.shadow_color[1],
        base_color[2] * mtoon.shadow_color[2],
        base_color[3] * mtoon.shadow_color[3],
    ];
    MToonMaterialMapping {
        base_color,
        shade_color,
        shadow_threshold: clamp01(0.5 + mtoon.shade_shift * 0.5),
        toon_steps: if mtoon.shade_toony < 0.5 { 2.0 } else { 1.0 },
        rim_color: mtoon.rim_color,
        rim_intensity: mtoon.rim_power * if mtoon.rim_light { 1.0 } else { 0.8 },
        rim_spread: clamp01(1.0 / mtoon.rim_power.max(0.1)),
        specular_color: mtoon.specular_color,
        spec_intensity: clamp01(mtoon.specular_power * 0.5),
        spec_power: (mtoon.specular_power * 24.0).max(2.0),
        emission_color: mtoon.sub_emission_color,
        emission_intensity,
        main_texture: mtoon.main_tex,
        use_sphere: mtoon.use_sphere,
        use_mat_cap: mtoon.use_mat_cap,
    }
}
