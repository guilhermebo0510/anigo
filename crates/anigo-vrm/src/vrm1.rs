//! ANIGO — extensões VRM 1.0 (lado Rust, espelho de `src/services/vrm/vrm1.ts`).
//!
//! Deserializa as quatro extensões oficiais:
//!  - `VRMC_vrm` — meta, humanoid (mapeamento de ossos), expressões;
//!  - `VRMC_materials_mtoon` — parâmetros MToon por material;
//!  - `VRMC_springBone` — schema oficial 1.0 (`springs[].joints[]`, `colliders[].shape`,
//!    `colliderGroups`, `center`) + `VRMC_springBone_extended_collider`; o formato legado
//!    do ANIGO (`secondaryRig`) ainda é lido e convertido;
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

/// Versão da spec `VRMC_springBone` (e `VRMC_springBone_extended_collider`) emitida pelo exportador.
pub const SPRING_BONE_SPEC_VERSION: &str = "1.0";

/// Junta de uma cadeia (`springs[i].joints[j]`, schema oficial `VRMC_springBone-1.0`).
///
/// Defaults da spec: `hitRadius=0`, `stiffness=1`, `gravityPower=0`,
/// `gravityDir=[0,-1,0]`, `dragForce=0.5`. A última junta da cadeia só serve de
/// "cauda" (os parâmetros dela não são usados).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vrm1SpringJoint {
    pub node: u64,
    pub hit_radius: f32,
    pub stiffness: f32,
    pub gravity_power: f32,
    pub gravity_dir: [f32; 3],
    pub drag_force: f32,
}

impl Default for Vrm1SpringJoint {
    fn default() -> Self {
        Self {
            node: 0,
            hit_radius: 0.0,
            stiffness: 1.0,
            gravity_power: 0.0,
            gravity_dir: [0.0, -1.0, 0.0],
            drag_force: 0.5,
        }
    }
}

/// Uma cadeia de molas (`springs[i]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vrm1Spring {
    pub name: Option<String>,
    pub joints: Vec<Vrm1SpringJoint>,
    /// Índices em `VRMC_springBone.colliderGroups`.
    pub collider_groups: Vec<u64>,
    /// Nó usado como raiz do "center space" (None = espaço do mundo).
    pub center: Option<u64>,
}

/// Forma de um collider (`colliders[i].shape`). `Sphere`/`Capsule` com
/// `inside=false` são a spec base; `inside=true` e `Plane` vêm de
/// `VRMC_springBone_extended_collider`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vrm1ColliderShape {
    Sphere { offset: [f32; 3], radius: f32, inside: bool },
    Capsule { offset: [f32; 3], radius: f32, tail: [f32; 3], inside: bool },
    Plane { offset: [f32; 3], normal: [f32; 3] },
}

impl Vrm1ColliderShape {
    /// `true` quando a forma precisa de `VRMC_springBone_extended_collider`.
    pub fn is_extended(&self) -> bool {
        match self {
            Vrm1ColliderShape::Sphere { inside, .. } | Vrm1ColliderShape::Capsule { inside, .. } => *inside,
            Vrm1ColliderShape::Plane { .. } => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vrm1Collider {
    pub node: u64,
    pub shape: Vrm1ColliderShape,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vrm1ColliderGroup {
    pub name: Option<String>,
    /// Índices em `VRMC_springBone.colliders`.
    pub colliders: Vec<u64>,
}

/// `VRMC_springBone` no schema oficial 1.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vrm1SpringBone {
    pub spec_version: String,
    pub colliders: Vec<Vrm1Collider>,
    pub collider_groups: Vec<Vrm1ColliderGroup>,
    pub springs: Vec<Vrm1Spring>,
}

impl Default for Vrm1SpringBone {
    fn default() -> Self {
        Self {
            spec_version: SPRING_BONE_SPEC_VERSION.to_string(),
            colliders: Vec::new(),
            collider_groups: Vec::new(),
            springs: Vec::new(),
        }
    }
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

#[allow(dead_code)]
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
        // A spec glTF põe `extensionsUsed/Required` na raiz; aceita também `asset.*` (arquivos antigos do ANIGO).
        json.pointer(&format!("/{key}"))
            .or_else(|| json.pointer(&format!("/asset/{key}")))
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

// ---------------------------------------------------------------------------
// VRMC_springBone (schema oficial 1.0 + leitura tolerante do formato legado)
// ---------------------------------------------------------------------------

fn sb_fail(message: impl Into<String>) -> Vrm1Error {
    fail(Vrm1ErrorCode::BadSpringBone, message)
}

fn sb_f32(value: Option<&Value>, fallback: f32, where_: &str) -> Result<f32, Vrm1Error> {
    match value {
        None | Some(Value::Null) => Ok(fallback),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && n.abs() < f32::MAX as f64)
            .map(|n| n as f32)
            .ok_or_else(|| sb_fail(format!("{where_}: número finito esperado"))),
    }
}

fn sb_vec3(value: Option<&Value>, fallback: [f32; 3], where_: &str) -> Result<[f32; 3], Vrm1Error> {
    match value {
        None | Some(Value::Null) => Ok(fallback),
        Some(v) => {
            let arr = v
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| sb_fail(format!("{where_}: vetor de 3 números esperado")))?;
            let mut out = [0f32; 3];
            for (i, component) in arr.iter().enumerate() {
                out[i] = sb_f32(Some(component), 0.0, where_)?;
            }
            Ok(out)
        }
    }
}

fn sb_node(value: Option<&Value>, where_: &str) -> Result<u64, Vrm1Error> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| sb_fail(format!("{where_}: índice de nó (inteiro >= 0) obrigatório")))
}

fn sb_index_list(value: Option<&Value>, where_: &str) -> Result<Vec<u64>, Vrm1Error> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| v.as_u64().ok_or_else(|| sb_fail(format!("{where_}: índice inteiro >= 0 esperado"))))
            .collect(),
        Some(_) => Err(sb_fail(format!("{where_}: lista de índices esperada"))),
    }
}

fn sb_name(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(String::from)
}

/// `parent[i]` de cada nó a partir de `nodes[*].children` (índices inválidos são ignorados).
fn node_parents(nodes: &[Value]) -> Vec<Option<usize>> {
    let mut parents = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children.iter().filter_map(Value::as_u64) {
                if let Some(slot) = parents.get_mut(child as usize) {
                    *slot = Some(i);
                }
            }
        }
    }
    parents
}

fn is_ancestor(parents: &[Option<usize>], ancestor: usize, node: usize) -> bool {
    let mut cursor = parents.get(node).copied().flatten();
    let mut guard = 0usize;
    while let Some(p) = cursor {
        if p == ancestor {
            return true;
        }
        guard += 1;
        if guard > parents.len() {
            return false; // ciclo malformado
        }
        cursor = parents.get(p).copied().flatten();
    }
    false
}

fn parse_official_shape(shape: &Value, where_: &str) -> Result<Vrm1ColliderShape, Vrm1Error> {
    let obj = shape
        .as_object()
        .ok_or_else(|| sb_fail(format!("{where_}: objeto esperado")))?;
    let nonneg = |v: f32, what: &str| -> Result<f32, Vrm1Error> {
        if v < 0.0 {
            Err(sb_fail(format!("{where_}.{what} deve ser >= 0")))
        } else {
            Ok(v)
        }
    };
    if let Some(sphere) = obj.get("sphere").filter(|v| !v.is_null()) {
        let w = format!("{where_}.sphere");
        return Ok(Vrm1ColliderShape::Sphere {
            offset: sb_vec3(sphere.get("offset"), [0.0; 3], &format!("{w}.offset"))?,
            radius: nonneg(sb_f32(sphere.get("radius"), 0.0, &format!("{w}.radius"))?, "sphere.radius")?,
            inside: sphere.get("inside").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    if let Some(capsule) = obj.get("capsule").filter(|v| !v.is_null()) {
        let w = format!("{where_}.capsule");
        return Ok(Vrm1ColliderShape::Capsule {
            offset: sb_vec3(capsule.get("offset"), [0.0; 3], &format!("{w}.offset"))?,
            radius: nonneg(sb_f32(capsule.get("radius"), 0.0, &format!("{w}.radius"))?, "capsule.radius")?,
            tail: sb_vec3(capsule.get("tail"), [0.0; 3], &format!("{w}.tail"))?,
            inside: capsule.get("inside").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    if let Some(plane) = obj.get("plane").filter(|v| !v.is_null()) {
        let w = format!("{where_}.plane");
        let normal = sb_vec3(plane.get("normal"), [0.0, 0.0, 1.0], &format!("{w}.normal"))?;
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if len < 1e-6 {
            return Err(sb_fail(format!("{w}.normal não pode ser o vetor nulo")));
        }
        return Ok(Vrm1ColliderShape::Plane {
            offset: sb_vec3(plane.get("offset"), [0.0; 3], &format!("{w}.offset"))?,
            normal: [normal[0] / len, normal[1] / len, normal[2] / len],
        });
    }
    Err(sb_fail(format!("{where_}: shape deve ter exatamente um de sphere/capsule/plane")))
}

fn parse_official_joint(raw: &Value, where_: &str) -> Result<Vrm1SpringJoint, Vrm1Error> {
    let obj = raw.as_object().ok_or_else(|| sb_fail(format!("{where_}: objeto esperado")))?;
    let defaults = Vrm1SpringJoint::default();
    let joint = Vrm1SpringJoint {
        node: sb_node(obj.get("node"), &format!("{where_}.node"))?,
        hit_radius: sb_f32(obj.get("hitRadius"), defaults.hit_radius, &format!("{where_}.hitRadius"))?,
        stiffness: sb_f32(obj.get("stiffness"), defaults.stiffness, &format!("{where_}.stiffness"))?,
        gravity_power: sb_f32(obj.get("gravityPower"), defaults.gravity_power, &format!("{where_}.gravityPower"))?,
        gravity_dir: sb_vec3(obj.get("gravityDir"), defaults.gravity_dir, &format!("{where_}.gravityDir"))?,
        drag_force: sb_f32(obj.get("dragForce"), defaults.drag_force, &format!("{where_}.dragForce"))?,
    };
    if joint.hit_radius < 0.0 {
        return Err(sb_fail(format!("{where_}.hitRadius deve ser >= 0")));
    }
    if joint.stiffness < 0.0 {
        return Err(sb_fail(format!("{where_}.stiffness deve ser >= 0")));
    }
    if !(0.0..=1.0).contains(&joint.drag_force) {
        return Err(sb_fail(format!("{where_}.dragForce deve estar em [0, 1]")));
    }
    Ok(joint)
}

fn parse_official_spring_bone(
    raw: &serde_json::Map<String, Value>,
    nodes: &[Value],
    warnings: &mut Vec<String>,
) -> Result<Vrm1SpringBone, Vrm1Error> {
    let spec_version = match raw.get("specVersion").and_then(Value::as_str) {
        Some(v) => v.to_string(),
        None => {
            warnings.push("VRMC_springBone.specVersion ausente (obrigatório na spec) — assumindo \"1.0\"".into());
            SPRING_BONE_SPEC_VERSION.to_string()
        }
    };
    if spec_version != SPRING_BONE_SPEC_VERSION {
        warnings.push(format!("VRMC_springBone.specVersion=\"{spec_version}\" não é \"1.0\" — lendo como 1.0"));
    }
    let node_count = nodes.len();
    let parents = node_parents(nodes);
    let check_node = |node: u64, where_: &str, warnings: &mut Vec<String>| {
        if node as usize >= node_count {
            warnings.push(format!("springBone {where_}.node={node} fora da faixa de nodes — será ignorado em runtime"));
        }
    };

    let mut colliders: Vec<Vrm1Collider> = Vec::new();
    if let Some(list) = raw.get("colliders").and_then(Value::as_array) {
        for (i, entry) in list.iter().enumerate() {
            let w = format!("colliders[{i}]");
            let obj = entry.as_object().ok_or_else(|| sb_fail(format!("{w}: objeto esperado")))?;
            let node = sb_node(obj.get("node"), &format!("{w}.node"))?;
            check_node(node, &w, warnings);
            let extended = obj
                .get("extensions")
                .and_then(|e| e.get("VRMC_springBone_extended_collider"))
                .and_then(|e| e.get("shape"))
                .filter(|v| !v.is_null());
            let shape = match (extended, obj.get("shape").filter(|v| !v.is_null())) {
                (Some(ext), _) => parse_official_shape(ext, &format!("{w}.extensions.VRMC_springBone_extended_collider.shape"))?,
                (None, Some(base)) => parse_official_shape(base, &format!("{w}.shape"))?,
                (None, None) => return Err(sb_fail(format!("{w}.shape obrigatório"))),
            };
            colliders.push(Vrm1Collider { node, shape });
        }
    }

    let mut collider_groups: Vec<Vrm1ColliderGroup> = Vec::new();
    if let Some(list) = raw.get("colliderGroups").and_then(Value::as_array) {
        for (i, entry) in list.iter().enumerate() {
            let w = format!("colliderGroups[{i}]");
            let obj = entry.as_object().ok_or_else(|| sb_fail(format!("{w}: objeto esperado")))?;
            let mut indices = sb_index_list(obj.get("colliders"), &format!("{w}.colliders"))?;
            let before = indices.len();
            indices.retain(|c| (*c as usize) < colliders.len());
            if indices.len() != before {
                warnings.push(format!("springBone {w}.colliders referencia collider inexistente — referência descartada"));
            }
            collider_groups.push(Vrm1ColliderGroup { name: sb_name(obj.get("name")), colliders: indices });
        }
    }

    let mut springs: Vec<Vrm1Spring> = Vec::new();
    let mut seen_joint_nodes: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    if let Some(list) = raw.get("springs").and_then(Value::as_array) {
        for (i, entry) in list.iter().enumerate() {
            let w = format!("springs[{i}]");
            let obj = entry.as_object().ok_or_else(|| sb_fail(format!("{w}: objeto esperado")))?;
            let joints_raw = obj
                .get("joints")
                .and_then(Value::as_array)
                .ok_or_else(|| sb_fail(format!("{w}.joints obrigatório")))?;
            let mut joints = Vec::with_capacity(joints_raw.len());
            for (j, joint_raw) in joints_raw.iter().enumerate() {
                let jw = format!("{w}.joints[{j}]");
                let joint = parse_official_joint(joint_raw, &jw)?;
                check_node(joint.node, &jw, warnings);
                if let Some(prev) = seen_joint_nodes.insert(joint.node, i) {
                    warnings.push(format!(
                        "springBone {jw}.node={} já pertence à cadeia springs[{prev}] (cadeias duplicadas são proibidas pela spec)",
                        joint.node
                    ));
                }
                joints.push(joint);
            }
            if joints.len() < 2 {
                warnings.push(format!("springBone {w}.joints tem {} junta(s); a spec exige >= 2 — cadeia sem efeito", joints.len()));
            }
            for pair in joints.windows(2) {
                if (pair[1].node as usize) < node_count
                    && (pair[0].node as usize) < node_count
                    && !is_ancestor(&parents, pair[0].node as usize, pair[1].node as usize)
                {
                    warnings.push(format!(
                        "springBone {w}: nó {} não é descendente do nó {} — a spec exige joints[n] ancestral de joints[n+1]",
                        pair[1].node, pair[0].node
                    ));
                }
            }
            let mut groups = sb_index_list(obj.get("colliderGroups"), &format!("{w}.colliderGroups"))?;
            let before = groups.len();
            groups.retain(|g| (*g as usize) < collider_groups.len());
            if groups.len() != before {
                warnings.push(format!("springBone {w}.colliderGroups referencia grupo inexistente — referência descartada"));
            }
            let center = match obj.get("center") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let node = sb_node(Some(v), &format!("{w}.center"))?;
                    check_node(node, &format!("{w}.center"), warnings);
                    Some(node)
                }
            };
            springs.push(Vrm1Spring { name: sb_name(obj.get("name")), joints, collider_groups: groups, center });
        }
    }
    Ok(Vrm1SpringBone { spec_version: SPRING_BONE_SPEC_VERSION.to_string(), colliders, collider_groups, springs })
}

/// Formato legado do ANIGO (`secondaryRig.groups[]/colliders[]{spheres,capsules}`), anterior ao
/// schema oficial. Convertido para o modelo novo para não perder arquivos já gerados.
fn parse_legacy_spring_bone(
    rig: &serde_json::Map<String, Value>,
    node_count: usize,
    warnings: &mut Vec<String>,
) -> Result<Vrm1SpringBone, Vrm1Error> {
    warnings.push(
        "VRMC_springBone em formato legado (secondaryRig) convertido para o schema oficial 1.0; reexporte para gravar o formato novo"
            .into(),
    );
    let check_node = |node: u64, where_: &str, warnings: &mut Vec<String>| {
        if node as usize >= node_count {
            warnings.push(format!("springBone {where_}.node={node} fora da faixa — será ignorado em runtime"));
        }
    };
    let legacy_joint = |raw: &Value, where_: &str| -> Result<Vrm1SpringJoint, Vrm1Error> {
        let obj = raw.as_object().ok_or_else(|| sb_fail(format!("{where_}: joint inválido")))?;
        let distance = sb_f32(obj.get("distance"), 0.0, &format!("{where_}.distance"))?;
        if distance < 0.0 {
            return Err(sb_fail(format!("{where_}.distance deve ser >= 0")));
        }
        Ok(Vrm1SpringJoint {
            node: sb_node(obj.get("node"), &format!("{where_}.node"))?,
            hit_radius: sb_f32(obj.get("hitRadius"), 0.05, &format!("{where_}.hitRadius"))?.max(0.0),
            gravity_power: sb_f32(obj.get("gravityPower"), 0.0, &format!("{where_}.gravityPower"))?,
            gravity_dir: sb_vec3(obj.get("gravityDir"), [0.0, -1.0, 0.0], &format!("{where_}.gravityDir"))?,
            stiffness: sb_f32(obj.get("springStiffness"), 0.5, &format!("{where_}.springStiffness"))?.max(0.0),
            drag_force: sb_f32(obj.get("springDamping"), 0.5, &format!("{where_}.springDamping"))?.clamp(0.0, 1.0),
        })
    };

    // Colliders: lista (ou objeto único) de {spheres[], capsules[]}; `group` é um id livre.
    let collider_objs: Vec<&Value> = match rig.get("colliders") {
        Some(Value::Array(arr)) => arr.iter().collect(),
        Some(v) if v.is_object() => vec![v],
        _ => vec![],
    };
    let mut colliders: Vec<Vrm1Collider> = Vec::new();
    let mut collider_group_ids: Vec<u64> = Vec::new(); // id legado de cada collider (mesmo índice)
    for (i, entry) in collider_objs.iter().enumerate() {
        let obj = entry.as_object().ok_or_else(|| sb_fail(format!("colliders[{i}] inválido")))?;
        if let Some(spheres) = obj.get("spheres").and_then(Value::as_array) {
            for (j, s) in spheres.iter().enumerate() {
                let w = format!("colliders[{i}].spheres[{j}]");
                let so = s.as_object().ok_or_else(|| sb_fail(format!("{w} inválido")))?;
                let node = so.get("node").and_then(Value::as_u64).unwrap_or(0);
                check_node(node, &w, warnings);
                colliders.push(Vrm1Collider {
                    node,
                    shape: Vrm1ColliderShape::Sphere {
                        offset: sb_vec3(so.get("offset"), [0.0; 3], &format!("{w}.offset"))?,
                        radius: sb_f32(so.get("radius"), 0.0, &format!("{w}.radius"))?.max(0.0),
                        inside: false,
                    },
                });
                collider_group_ids.push(so.get("group").and_then(Value::as_u64).unwrap_or(0));
            }
        }
        if let Some(capsules) = obj.get("capsules").and_then(Value::as_array) {
            for (j, c) in capsules.iter().enumerate() {
                let w = format!("colliders[{i}].capsules[{j}]");
                let co = c.as_object().ok_or_else(|| sb_fail(format!("{w} inválido")))?;
                let node = co.get("node").and_then(Value::as_u64).unwrap_or(0);
                check_node(node, &w, warnings);
                colliders.push(Vrm1Collider {
                    node,
                    shape: Vrm1ColliderShape::Capsule {
                        offset: sb_vec3(co.get("from"), [0.0; 3], &format!("{w}.from"))?,
                        tail: sb_vec3(co.get("to"), [0.0; 3], &format!("{w}.to"))?,
                        radius: sb_f32(co.get("radius"), 0.0, &format!("{w}.radius"))?.max(0.0),
                        inside: false,
                    },
                });
                collider_group_ids.push(co.get("group").and_then(Value::as_u64).unwrap_or(0));
            }
        }
    }

    let mut springs: Vec<Vrm1Spring> = Vec::new();
    let mut referenced_ids: Vec<u64> = collider_group_ids.clone();
    let mut spring_group_ids: Vec<Vec<u64>> = Vec::new();
    if let Some(groups) = rig.get("groups").and_then(Value::as_array) {
        for (i, entry) in groups.iter().enumerate() {
            let obj = entry.as_object().ok_or_else(|| sb_fail(format!("groups[{i}] inválido")))?;
            let center = match obj.get("center") {
                None | Some(Value::Null) => None,
                Some(raw) => {
                    let joint = legacy_joint(raw, &format!("groups[{i}].center"))?;
                    check_node(joint.node, &format!("groups[{i}].center"), warnings);
                    Some(joint.node)
                }
            };
            let mut joints = Vec::new();
            if let Some(list) = obj.get("joints").and_then(Value::as_array) {
                for (j, raw) in list.iter().enumerate() {
                    let w = format!("groups[{i}].joints[{j}]");
                    let joint = legacy_joint(raw, &w)?;
                    check_node(joint.node, &w, warnings);
                    joints.push(joint);
                }
            }
            let ids: Vec<u64> = obj
                .get("colliderGroups")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            referenced_ids.extend(ids.iter().copied());
            spring_group_ids.push(ids);
            springs.push(Vrm1Spring {
                name: Some(sb_name(obj.get("name")).unwrap_or_else(|| format!("group_{i}"))),
                joints,
                collider_groups: Vec::new(),
                center,
            });
        }
    }

    // Ids legados -> índices densos de `colliderGroups` (ordem crescente de id: determinístico).
    referenced_ids.sort_unstable();
    referenced_ids.dedup();
    let mut collider_groups: Vec<Vrm1ColliderGroup> = Vec::new();
    for id in &referenced_ids {
        let members: Vec<u64> = collider_group_ids
            .iter()
            .enumerate()
            .filter(|(_, g)| *g == id)
            .map(|(idx, _)| idx as u64)
            .collect();
        collider_groups.push(Vrm1ColliderGroup { name: Some(format!("group_{id}")), colliders: members });
    }
    for (spring, ids) in springs.iter_mut().zip(spring_group_ids) {
        spring.collider_groups = ids
            .iter()
            .filter_map(|id| referenced_ids.binary_search(id).ok().map(|p| p as u64))
            .collect();
    }
    Ok(Vrm1SpringBone { spec_version: SPRING_BONE_SPEC_VERSION.to_string(), colliders, collider_groups, springs })
}

/// Lê um bloco `VRMC_springBone` (oficial ou legado). `nodes` é `gltf.nodes` (para validar
/// índices e a ordem ancestral das juntas).
pub fn parse_spring_bone(raw: Option<&Value>, nodes: &[Value], warnings: &mut Vec<String>) -> Result<Vrm1SpringBone, Vrm1Error> {
    let Some(obj) = raw.and_then(Value::as_object) else {
        return Err(sb_fail("VRMC_springBone inválido (objeto esperado)"));
    };
    if let Some(rig) = obj.get("secondaryRig").and_then(Value::as_object) {
        return parse_legacy_spring_bone(rig, nodes.len(), warnings);
    }
    if obj.contains_key("secondaryRig") {
        return Err(sb_fail("VRMC_springBone.secondaryRig inválido"));
    }
    parse_official_spring_bone(obj, nodes, warnings)
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
        let nodes: &[Value] = model.json.get("nodes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        Some(parse_spring_bone(raw.as_ref(), nodes, &mut warnings)?)
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
    let mtoon_default = Vrm1MToon::default();
    let mtoon = mtoon.unwrap_or(&mtoon_default);
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
