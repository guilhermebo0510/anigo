//! VRM 1.0 / glTF 2.0 interoperability for ANIGO.
//!
//! The editor keeps its project state in `anigo-core`; this crate is deliberately
//! concerned only with the wire format.  It owns the GLB container boundary,
//! validates the parts of the VRM 1.0 extensions that ANIGO consumes, and can
//! decorate an existing glTF 2.0 asset with a deterministic VRM 1.0 envelope.
//!
//! No renderer or UI state is hidden in this module.  Import returns a typed
//! report and export either returns a complete GLB or an actionable error.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const GLB_MAGIC: u32 = 0x4654_6c67;
pub const GLB_VERSION: u32 = 2;
pub const JSON_CHUNK: u32 = 0x4e4f_534a;
pub const BIN_CHUNK: u32 = 0x004e_4942;
pub const VRM_EXTENSION: &str = "VRMC_vrm";
pub const MTOON_EXTENSION: &str = "VRMC_materials_mtoon";
pub const SPRING_BONE_EXTENSION: &str = "VRMC_springBone";
pub const NODE_CONSTRAINT_EXTENSION: &str = "VRMC_node_constraint";

/// Metadata used by VRM 1.0's `VRMC_vrm.meta` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmMeta {
    /// VRM `meta.name`; `title` is kept for compatibility with the first ANIGO API.
    pub title: String,
    pub version: String,
    pub author: String,
    #[serde(default)]
    pub contact_information: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
}

impl Default for VrmMeta {
    fn default() -> Self {
        Self {
            title: "ANIGO Character".to_string(),
            version: "1.0".to_string(),
            author: "ANIGO Studio".to_string(),
            contact_information: None,
            reference: None,
        }
    }
}

/// A VRM expression weight in the normalized 0..1 domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendShapeGroup {
    pub name: String,
    #[serde(default)]
    pub preset_name: String,
    pub weight: f32,
}

/// A single entry in `VRMC_vrm.humanoid.humanBones`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VrmHumanoidBone {
    pub bone: String,
    pub node: u32,
}

/// A small, serializable MToon profile. Values are kept in the same domains as
/// the VRM extension: colors are linear factors and shifts are scalar factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmMtoonMaterial {
    #[serde(rename = "material")]
    pub material_index: u32,
    #[serde(default = "default_white")]
    pub shade_color_factor: [f32; 4],
    #[serde(default)]
    pub shading_shift_factor: f32,
    #[serde(default = "default_toony")]
    pub shading_toony_factor: f32,
    #[serde(default)]
    pub outline_width_mode: String,
    #[serde(default)]
    pub outline_width_factor: f32,
    #[serde(default = "default_black")]
    pub outline_color_factor: [f32; 4],
}

fn default_white() -> [f32; 4] { [1.0, 1.0, 1.0, 1.0] }
fn default_black() -> [f32; 4] { [0.0, 0.0, 0.0, 1.0] }
fn default_toony() -> f32 { 0.9 }

impl Default for VrmMtoonMaterial {
    fn default() -> Self {
        Self {
            material_index: 0,
            shade_color_factor: default_white(),
            shading_shift_factor: 0.0,
            shading_toony_factor: default_toony(),
            outline_width_mode: "none".to_string(),
            outline_width_factor: 0.0,
            outline_color_factor: default_black(),
        }
    }
}

/// Optional export payload. The base mesh and its binary buffer remain intact;
/// only the JSON extension graph is updated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmExportOptions {
    pub meta: VrmMeta,
    #[serde(default)]
    pub humanoid: Vec<VrmHumanoidBone>,
    #[serde(default)]
    pub expressions: Vec<BlendShapeGroup>,
    #[serde(default)]
    pub mtoon_materials: Vec<VrmMtoonMaterial>,
    #[serde(default)]
    pub spring_bone: Option<Value>,
    #[serde(default, alias = "nodeConstraint")]
    pub node_constraints: Option<Value>,
}

impl Default for VrmExportOptions {
    fn default() -> Self {
        Self {
            meta: VrmMeta::default(),
            humanoid: vec![
                VrmHumanoidBone { bone: "hips".to_string(), node: 0 },
                VrmHumanoidBone { bone: "spine".to_string(), node: 0 },
                VrmHumanoidBone { bone: "head".to_string(), node: 0 },
            ],
            expressions: Vec::new(),
            mtoon_materials: Vec::new(),
            spring_bone: None,
            node_constraints: None,
        }
    }
}

/// GLB payload after the container has been checked.
#[derive(Debug, Clone, PartialEq)]
pub struct GlbDocument {
    pub json: Value,
    pub bin: Vec<u8>,
}

/// Import result used by the asset browser and the core bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmImportSummary {
    pub meta: VrmMeta,
    pub spec_version: String,
    pub node_count: u32,
    pub mesh_count: u32,
    pub material_count: u32,
    pub humanoid_bones: Vec<VrmHumanoidBone>,
    pub expressions: Vec<BlendShapeGroup>,
    pub has_mtoon: bool,
    pub has_spring_bone: bool,
    pub has_node_constraint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VrmIssueSeverity { Error, Warning }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmIssue {
    pub severity: VrmIssueSeverity,
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrmValidationReport {
    pub valid: bool,
    pub is_vrm: bool,
    pub spec_version: Option<String>,
    pub issues: Vec<VrmIssue>,
    pub node_count: u32,
    pub mesh_count: u32,
    pub material_count: u32,
}

impl VrmValidationReport {
    pub fn errors(&self) -> usize {
        self.issues.iter().filter(|issue| issue.severity == VrmIssueSeverity::Error).count()
    }

    pub fn warnings(&self) -> usize {
        self.issues.iter().filter(|issue| issue.severity == VrmIssueSeverity::Warning).count()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VrmError {
    #[error("GLB is truncated (need at least {needed} bytes, got {actual})")]
    Truncated { needed: usize, actual: usize },
    #[error("invalid GLB magic 0x{0:08x}")]
    InvalidMagic(u32),
    #[error("unsupported GLB version {0}; expected 2")]
    UnsupportedVersion(u32),
    #[error("GLB declares {declared} bytes but contains {actual}")]
    InvalidLength { declared: usize, actual: usize },
    #[error("GLB chunk at offset {offset} exceeds the container")]
    InvalidChunk { offset: usize },
    #[error("GLB does not contain a JSON chunk")]
    MissingJson,
    #[error("GLB does not contain a BIN chunk")]
    MissingBinary,
    #[error("GLB JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("the source asset is not valid glTF 2.0: {0}")]
    InvalidGltf(String),
    #[error("the asset is not a valid VRM 1.0 document: {0}")]
    InvalidVrm(String),
    #[error("could not serialize GLB JSON: {0}")]
    SerializeJson(String),
    #[error("numeric value is outside the supported range at {path}")]
    InvalidNumber { path: String },
}

/// Reads and validates the binary GLB container without interpreting geometry.
pub fn parse_glb(bytes: &[u8]) -> Result<GlbDocument, VrmError> {
    if bytes.len() < 12 {
        return Err(VrmError::Truncated { needed: 12, actual: bytes.len() });
    }
    let magic = read_u32(bytes, 0)?;
    if magic != GLB_MAGIC { return Err(VrmError::InvalidMagic(magic)); }
    let version = read_u32(bytes, 4)?;
    if version != GLB_VERSION { return Err(VrmError::UnsupportedVersion(version)); }
    let declared = read_u32(bytes, 8)? as usize;
    if declared < 20 || declared != bytes.len() {
        return Err(VrmError::InvalidLength { declared, actual: bytes.len() });
    }

    let mut offset = 12usize;
    let mut json_bytes: Option<&[u8]> = None;
    let mut bin_bytes: Option<&[u8]> = None;
    while offset < declared {
        if declared - offset < 8 { return Err(VrmError::InvalidChunk { offset }); }
        let chunk_len = read_u32(bytes, offset)? as usize;
        let chunk_type = read_u32(bytes, offset + 4)?;
        let data_start = offset + 8;
        let data_end = data_start.checked_add(chunk_len).ok_or(VrmError::InvalidChunk { offset })?;
        if data_end > declared { return Err(VrmError::InvalidChunk { offset }); }
        match chunk_type {
            JSON_CHUNK if json_bytes.is_none() => json_bytes = Some(&bytes[data_start..data_end]),
            BIN_CHUNK if bin_bytes.is_none() => bin_bytes = Some(&bytes[data_start..data_end]),
            _ => {}
        }
        offset = data_end;
    }

    let json_bytes = json_bytes.ok_or(VrmError::MissingJson)?;
    let json_text = std::str::from_utf8(json_bytes)
        .map_err(|error| VrmError::InvalidJson(error.to_string()))?
        .trim_end_matches('\0')
        .trim_end();
    let json: Value = serde_json::from_str(json_text).map_err(|error| VrmError::InvalidJson(error.to_string()))?;
    Ok(GlbDocument { json, bin: bin_bytes.unwrap_or(&[]).to_vec() })
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, VrmError> {
    let end = offset.checked_add(4).ok_or(VrmError::Truncated { needed: offset + 4, actual: bytes.len() })?;
    if end > bytes.len() { return Err(VrmError::Truncated { needed: end, actual: bytes.len() }); }
    Ok(u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]))
}

fn padded_len(length: usize) -> usize { (length + 3) & !3 }

/// Writes a deterministic GLB. JSON is padded with spaces and BIN with zeroes,
/// as required by the glTF 2.0 container specification.
pub fn write_glb(document: &GlbDocument) -> Result<Vec<u8>, VrmError> {
    let mut json_bytes = serde_json::to_vec(&document.json).map_err(|error| VrmError::SerializeJson(error.to_string()))?;
    let json_len = padded_len(json_bytes.len());
    json_bytes.resize(json_len, b' ');
    let bin_len = padded_len(document.bin.len());
    let total = 12usize
        .checked_add(8).and_then(|n| n.checked_add(json_len))
        .and_then(|n| n.checked_add(8)).and_then(|n| n.checked_add(bin_len))
        .ok_or(VrmError::InvalidLength { declared: usize::MAX, actual: 0 })?;
    let total_u32 = u32::try_from(total).map_err(|_| VrmError::InvalidLength { declared: total, actual: 0 })?;
    let json_u32 = u32::try_from(json_len).map_err(|_| VrmError::InvalidLength { declared: json_len, actual: 0 })?;
    let bin_u32 = u32::try_from(bin_len).map_err(|_| VrmError::InvalidLength { declared: bin_len, actual: 0 })?;

    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    output.extend_from_slice(&GLB_VERSION.to_le_bytes());
    output.extend_from_slice(&total_u32.to_le_bytes());
    output.extend_from_slice(&json_u32.to_le_bytes());
    output.extend_from_slice(&JSON_CHUNK.to_le_bytes());
    output.extend_from_slice(&json_bytes);
    output.extend_from_slice(&bin_u32.to_le_bytes());
    output.extend_from_slice(&BIN_CHUNK.to_le_bytes());
    output.extend_from_slice(&document.bin);
    output.resize(total, 0);
    Ok(output)
}

/// Validates an arbitrary glTF 2.0 GLB. It does not require VRM extensions.
pub fn validate_gltf(bytes: &[u8]) -> Result<VrmValidationReport, VrmError> {
    let document = parse_glb(bytes)?;
    Ok(validate_json(&document.json, false))
}

/// Validates a VRM 1.0 GLB and returns all issues in one pass.
pub fn validate_vrm(bytes: &[u8]) -> Result<VrmValidationReport, VrmError> {
    let document = parse_glb(bytes)?;
    Ok(validate_json(&document.json, true))
}

fn validate_json(json: &Value, require_vrm: bool) -> VrmValidationReport {
    let mut issues = Vec::new();
    let asset = json.get("asset");
    if asset.and_then(|v| v.get("version")).and_then(Value::as_str) != Some("2.0") {
        issue(&mut issues, VrmIssueSeverity::Error, "GLTF_VERSION", "/asset/version", "asset.version must be '2.0'");
    }
    let nodes = array_len(json, "nodes");
    let meshes = array_len(json, "meshes");
    let materials = array_len(json, "materials");
    if nodes == 0 { issue(&mut issues, VrmIssueSeverity::Error, "NO_NODES", "/nodes", "VRM must contain at least one node"); }
    if meshes == 0 { issue(&mut issues, VrmIssueSeverity::Warning, "NO_MESHES", "/meshes", "document has no mesh; it can be metadata-only but cannot display a character"); }
    if json.get("scenes").and_then(Value::as_array).is_none() {
        issue(&mut issues, VrmIssueSeverity::Error, "NO_SCENES", "/scenes", "glTF must declare scenes");
    }

    let extensions = json.get("extensions").and_then(Value::as_object);
    let vrm = extensions.and_then(|map| map.get(VRM_EXTENSION));
    let spec_version = vrm.and_then(|value| value.get("specVersion")).and_then(Value::as_str).map(ToOwned::to_owned);
    if require_vrm && vrm.is_none() {
        issue(&mut issues, VrmIssueSeverity::Error, "MISSING_VRM_EXTENSION", "/extensions/VRMC_vrm", "VRM 1.0 requires the VRMC_vrm extension");
    }
    if require_vrm && spec_version.as_deref() != Some("1.0") {
        issue(&mut issues, VrmIssueSeverity::Error, "VRM_SPEC_VERSION", "/extensions/VRMC_vrm/specVersion", "VRMC_vrm.specVersion must be '1.0'");
    }

    if let Some(vrm) = vrm {
        validate_meta(vrm, &mut issues);
        validate_humanoid(vrm, nodes, &mut issues);
        validate_expressions(vrm, &mut issues);
    }
    validate_extension_versions(extensions, &mut issues);
    let is_vrm = vrm.is_some() && spec_version.as_deref() == Some("1.0");
    let valid = !issues.iter().any(|entry| entry.severity == VrmIssueSeverity::Error) && (!require_vrm || is_vrm);
    VrmValidationReport { valid, is_vrm, spec_version, issues, node_count: nodes, mesh_count: meshes, material_count: materials }
}

fn array_len(json: &Value, field: &str) -> u32 { json.get(field).and_then(Value::as_array).map(|array| array.len() as u32).unwrap_or(0) }

fn issue(issues: &mut Vec<VrmIssue>, severity: VrmIssueSeverity, code: &str, path: &str, message: &str) {
    issues.push(VrmIssue { severity, code: code.to_string(), path: path.to_string(), message: message.to_string() });
}

fn validate_meta(vrm: &Value, issues: &mut Vec<VrmIssue>) {
    let Some(meta) = vrm.get("meta").and_then(Value::as_object) else {
        issue(issues, VrmIssueSeverity::Error, "MISSING_META", "/extensions/VRMC_vrm/meta", "VRM 1.0 requires metadata");
        return;
    };
    for field in ["name", "version"] {
        if meta.get(field).and_then(Value::as_str).map(str::is_empty).unwrap_or(true) {
            issue(issues, VrmIssueSeverity::Error, "META_FIELD", &format!("/extensions/VRMC_vrm/meta/{field}"), "field must be a non-empty string");
        }
    }
    if meta.get("authors").and_then(Value::as_array).map(|items| items.is_empty()).unwrap_or(true) {
        issue(issues, VrmIssueSeverity::Error, "META_AUTHORS", "/extensions/VRMC_vrm/meta/authors", "authors must contain at least one author");
    }
}

fn validate_humanoid(vrm: &Value, node_count: u32, issues: &mut Vec<VrmIssue>) {
    let Some(humanoid) = vrm.get("humanoid").and_then(Value::as_object) else {
        issue(issues, VrmIssueSeverity::Error, "MISSING_HUMANOID", "/extensions/VRMC_vrm/humanoid", "VRM 1.0 requires a humanoid mapping");
        return;
    };
    let Some(bones) = humanoid.get("humanBones").and_then(Value::as_array) else {
        issue(issues, VrmIssueSeverity::Error, "MISSING_HUMAN_BONES", "/extensions/VRMC_vrm/humanoid/humanBones", "humanBones must be an array");
        return;
    };
    let mut names = std::collections::BTreeSet::new();
    for (index, bone) in bones.iter().enumerate() {
        let path = format!("/extensions/VRMC_vrm/humanoid/humanBones/{index}");
        let name = bone.get("bone").and_then(Value::as_str);
        let node = bone.get("node").and_then(Value::as_u64);
        if name.is_none() || node.is_none() { issue(issues, VrmIssueSeverity::Error, "HUMANOID_ENTRY", &path, "each human bone needs bone and node"); continue; }
        if !names.insert(name.unwrap_or_default()) { issue(issues, VrmIssueSeverity::Error, "DUPLICATE_HUMANOID_BONE", &path, "human bone names must be unique"); }
        if node.unwrap_or(0) >= u64::from(node_count) { issue(issues, VrmIssueSeverity::Error, "HUMANOID_NODE", &format!("{path}/node"), "node index is outside the nodes array"); }
    }
    for required in ["hips", "spine", "head"] {
        if !names.contains(required) { issue(issues, VrmIssueSeverity::Warning, "INCOMPLETE_HUMANOID", "/extensions/VRMC_vrm/humanoid/humanBones", &format!("recommended humanoid bone '{required}' is missing")); }
    }
}

fn validate_expressions(vrm: &Value, issues: &mut Vec<VrmIssue>) {
    let Some(expressions) = vrm.get("expressions") else { return; };
    let Some(object) = expressions.as_object() else { issue(issues, VrmIssueSeverity::Error, "EXPRESSIONS_OBJECT", "/extensions/VRMC_vrm/expressions", "expressions must be an object"); return; };
    for group in ["preset", "custom"] {
        if let Some(map) = object.get(group) {
            if !map.is_object() { issue(issues, VrmIssueSeverity::Error, "EXPRESSIONS_GROUP", &format!("/extensions/VRMC_vrm/expressions/{group}"), "expression group must be an object"); }
        }
    }
}

fn validate_extension_versions(extensions: Option<&Map<String, Value>>, issues: &mut Vec<VrmIssue>) {
    let Some(extensions) = extensions else { return; };
    for name in [MTOON_EXTENSION, SPRING_BONE_EXTENSION, NODE_CONSTRAINT_EXTENSION] {
        if let Some(value) = extensions.get(name) {
            let version = value.get("specVersion").and_then(Value::as_str);
            if version != Some("1.0") {
                issue(issues, VrmIssueSeverity::Error, "EXTENSION_VERSION", &format!("/extensions/{name}/specVersion"), "ANIGO supports the VRM 1.0 extension version only");
            }
        }
    }
}

/// Imports a VRM and refuses to return a scene when validation has errors.
pub fn import_vrm(bytes: &[u8]) -> Result<(GlbDocument, VrmImportSummary), VrmError> {
    let document = parse_glb(bytes)?;
    let report = validate_json(&document.json, true);
    if !report.valid { return Err(VrmError::InvalidVrm(format_validation_errors(&report))); }
    let summary = summary_from_json(&document.json, &report);
    Ok((document, summary))
}

fn format_validation_errors(report: &VrmValidationReport) -> String {
    report.issues.iter().filter(|issue| issue.severity == VrmIssueSeverity::Error).map(|issue| format!("{} {}: {}", issue.code, issue.path, issue.message)).collect::<Vec<_>>().join("; ")
}

fn meta_from_json(vrm: &Value) -> VrmMeta {
    let meta = vrm.get("meta").and_then(Value::as_object);
    let author = meta.and_then(|object| object.get("authors")).and_then(Value::as_array).and_then(|items| items.first()).and_then(Value::as_str).unwrap_or("ANIGO Studio");
    VrmMeta {
        title: meta.and_then(|object| object.get("name")).and_then(Value::as_str).unwrap_or("ANIGO Character").to_string(),
        version: meta.and_then(|object| object.get("version")).and_then(Value::as_str).unwrap_or("1.0").to_string(),
        author: author.to_string(),
        contact_information: meta.and_then(|object| object.get("contactInformation")).and_then(Value::as_str).map(ToOwned::to_owned),
        reference: meta.and_then(|object| object.get("references")).and_then(Value::as_array).and_then(|items| items.first()).and_then(Value::as_str).map(ToOwned::to_owned),
    }
}

fn humanoid_from_json(vrm: &Value) -> Vec<VrmHumanoidBone> {
    vrm.get("humanoid").and_then(|value| value.get("humanBones")).and_then(Value::as_array).into_iter().flatten().filter_map(|bone| Some(VrmHumanoidBone { bone: bone.get("bone")?.as_str()?.to_string(), node: u32::try_from(bone.get("node")?.as_u64()?).ok()? })).collect()
}

fn expressions_from_json(vrm: &Value) -> Vec<BlendShapeGroup> {
    let preset = vrm.get("expressions").and_then(|value| value.get("preset")).and_then(Value::as_object);
    preset.into_iter().flat_map(|map| map.iter()).filter_map(|(name, expression)| Some(BlendShapeGroup { name: name.clone(), preset_name: name.clone(), weight: expression.get("overrideBlink").and_then(Value::as_f64).unwrap_or(0.0) as f32 })).collect()
}

fn summary_from_json(json: &Value, report: &VrmValidationReport) -> VrmImportSummary {
    let vrm = json.get("extensions").and_then(|value| value.get(VRM_EXTENSION)).unwrap_or(&Value::Null);
    VrmImportSummary {
        meta: meta_from_json(vrm),
        spec_version: report.spec_version.clone().unwrap_or_else(|| "1.0".to_string()),
        node_count: report.node_count,
        mesh_count: report.mesh_count,
        material_count: report.material_count,
        humanoid_bones: humanoid_from_json(vrm),
        expressions: expressions_from_json(vrm),
        has_mtoon: json.get("extensions").and_then(Value::as_object).map(|map| map.contains_key(MTOON_EXTENSION)).unwrap_or(false),
        has_spring_bone: json.get("extensions").and_then(Value::as_object).map(|map| map.contains_key(SPRING_BONE_EXTENSION)).unwrap_or(false),
        has_node_constraint: json.get("extensions").and_then(Value::as_object).map(|map| map.contains_key(NODE_CONSTRAINT_EXTENSION)).unwrap_or(false),
    }
}

/// Adds VRM 1.0 extensions to an existing glTF 2.0 GLB.
///
/// This is intentionally an additive operation: buffers, accessors, meshes,
/// skinning and textures are preserved byte-for-byte. That makes export safe
/// for assets produced by other DCCs and makes round-trip checks deterministic.
pub fn export_vrm_glb(base_glb: &[u8], options: &VrmExportOptions) -> Result<Vec<u8>, VrmError> {
    let mut document = parse_glb(base_glb)?;
    let base_report = validate_json(&document.json, false);
    if !base_report.valid { return Err(VrmError::InvalidGltf(format_validation_errors(&base_report))); }
    let node_count = base_report.node_count;
    let humanoid = options.humanoid.iter().map(|bone| json!({ "bone": bone.bone, "node": bone.node })).collect::<Vec<_>>();
    for bone in &options.humanoid {
        if bone.node >= node_count { return Err(VrmError::InvalidVrm(format!("humanoid bone '{}' references node {} but the asset has {} nodes", bone.bone, bone.node, node_count))); }
    }
    let preset_expressions = options.expressions.iter().map(|expression| (expression.name.clone(), json!({ "overrideBlink": expression.weight.clamp(0.0, 1.0) }))).collect::<Map<_, _>>();
    let mut vrm = document.json.get("extensions").and_then(|value| value.get(VRM_EXTENSION)).cloned().unwrap_or_else(|| json!({}));
    let vrm_object = vrm.as_object_mut().ok_or_else(|| VrmError::InvalidVrm("VRMC_vrm must be an object".to_string()))?;
    vrm_object.insert("specVersion".to_string(), Value::String("1.0".to_string()));
    vrm_object.insert("meta".to_string(), json!({
        "name": options.meta.title.clone(),
        "version": options.meta.version.clone(),
        "authors": [options.meta.author.clone()],
        "contactInformation": options.meta.contact_information.clone(),
        "references": options.meta.reference.clone().map(|reference| vec![reference]).unwrap_or_default(),
    }));
    vrm_object.insert("humanoid".to_string(), json!({ "humanBones": humanoid }));
    if !preset_expressions.is_empty() { vrm_object.insert("expressions".to_string(), json!({ "preset": preset_expressions, "custom": {} })); }
    // Read the material count before borrowing the root extensions mutably.
    // Rust's borrow checker correctly prevents inspecting `document.json` while
    // the extension map is being edited.
    let material_count = document.json.get("materials").and_then(Value::as_array).map(|items| items.len()).unwrap_or(0);
    let extensions = document.json.as_object_mut().ok_or_else(|| VrmError::InvalidGltf("glTF root must be an object".to_string()))?.entry("extensions").or_insert_with(|| json!({}));
    let extensions_object = extensions.as_object_mut().ok_or_else(|| VrmError::InvalidGltf("extensions must be an object".to_string()))?;
    extensions_object.insert(VRM_EXTENSION.to_string(), Value::Object(vrm_object.clone()));

    if !options.mtoon_materials.is_empty() {
        let materials = material_count;
        let mut mtoon_map = Map::new();
        mtoon_map.insert("specVersion".to_string(), Value::String("1.0".to_string()));
        mtoon_map.insert("materials".to_string(), Value::Array(options.mtoon_materials.iter().filter_map(|profile| {
            if usize::try_from(profile.material_index).ok()? >= materials { return None; }
            Some(json!({
                "material": profile.material_index,
                "shadeColorFactor": profile.shade_color_factor,
                "shadingShiftFactor": profile.shading_shift_factor,
                "shadingToonyFactor": profile.shading_toony_factor,
                "outlineWidthMode": profile.outline_width_mode,
                "outlineWidthFactor": profile.outline_width_factor,
                "outlineColorFactor": profile.outline_color_factor,
            }))
        }).collect()));
        extensions_object.insert(MTOON_EXTENSION.to_string(), Value::Object(mtoon_map));
    }
    if let Some(spring_bone) = &options.spring_bone { extensions_object.insert(SPRING_BONE_EXTENSION.to_string(), with_spec_version(spring_bone.clone())); }
    if let Some(constraints) = &options.node_constraints { extensions_object.insert(NODE_CONSTRAINT_EXTENSION.to_string(), with_spec_version(constraints.clone())); }
    add_extension_name(&mut document.json, VRM_EXTENSION);
    if !options.mtoon_materials.is_empty() { add_extension_name(&mut document.json, MTOON_EXTENSION); }
    if options.spring_bone.is_some() { add_extension_name(&mut document.json, SPRING_BONE_EXTENSION); }
    if options.node_constraints.is_some() { add_extension_name(&mut document.json, NODE_CONSTRAINT_EXTENSION); }
    let result = write_glb(&document)?;
    let report = validate_vrm(&result)?;
    if !report.valid { return Err(VrmError::InvalidVrm(format_validation_errors(&report))); }
    Ok(result)
}

fn with_spec_version(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() { object.entry("specVersion").or_insert_with(|| Value::String("1.0".to_string())); }
    value
}

fn add_extension_name(json: &mut Value, name: &str) {
    let root = match json.as_object_mut() { Some(root) => root, None => return };
    let used = root.entry("extensionsUsed").or_insert_with(|| Value::Array(Vec::new()));
    if let Some(items) = used.as_array_mut() {
        if !items.iter().any(|item| item.as_str() == Some(name)) { items.push(Value::String(name.to_string())); }
    }
}

/// Convenience alias used by command adapters.
pub fn validate_vrm_file(bytes: &[u8]) -> Result<VrmValidationReport, VrmError> { validate_vrm(bytes) }

/// Stateless exporter façade for integrations that prefer an object API.
pub struct VrmExporter;
impl VrmExporter {
    pub fn export(base_glb: &[u8], options: &VrmExportOptions) -> Result<Vec<u8>, VrmError> { export_vrm_glb(base_glb, options) }
}

/// Stateless importer façade for integrations that prefer an object API.
pub struct VrmImporter;
impl VrmImporter {
    pub fn import(bytes: &[u8]) -> Result<(GlbDocument, VrmImportSummary), VrmError> { import_vrm(bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_glb() -> Vec<u8> {
        let json = json!({
            "asset": { "version": "2.0", "generator": "test" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{ "name": "Root" }],
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
            "materials": [{}],
            "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3" }],
            "bufferViews": [{ "buffer": 0, "byteLength": 12 }],
            "buffers": [{ "byteLength": 12 }]
        });
        write_glb(&GlbDocument { json, bin: vec![0; 12] }).expect("test fixture is representable")
    }

    fn options() -> VrmExportOptions {
        VrmExportOptions {
            meta: VrmMeta { title: "Aoi".into(), version: "1.0.0".into(), author: "ANIGO".into(), ..Default::default() },
            humanoid: vec![
                VrmHumanoidBone { bone: "hips".into(), node: 0 },
                VrmHumanoidBone { bone: "spine".into(), node: 0 },
                VrmHumanoidBone { bone: "head".into(), node: 0 },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn glb_container_round_trip_preserves_binary() {
        let source = base_glb();
        let parsed = parse_glb(&source).expect("fixture should parse");
        let output = write_glb(&parsed).expect("fixture should write");
        assert_eq!(parse_glb(&output).expect("round trip").bin, vec![0; 12]);
    }

    #[test]
    fn exports_and_imports_vrm_1() {
        let output = export_vrm_glb(&base_glb(), &options()).expect("VRM should export");
        let report = validate_vrm(&output).expect("export should parse");
        assert!(report.valid, "issues: {:?}", report.issues);
        let (_, summary) = import_vrm(&output).expect("export should import");
        assert_eq!(summary.meta.title, "Aoi");
        assert_eq!(summary.humanoid_bones.len(), 3);
    }

    #[test]
    fn rejects_humanoid_node_outside_asset() {
        let mut payload = options();
        payload.humanoid[0].node = 99;
        let error = export_vrm_glb(&base_glb(), &payload).expect_err("bad node must fail");
        assert!(error.to_string().contains("references node"));
    }

    #[test]
    fn rejects_wrong_vrm_spec_version() {
        let mut document = parse_glb(&base_glb()).expect("fixture should parse");
        document.json["extensions"] = json!({ "VRMC_vrm": { "specVersion": "0.0" } });
        let bytes = write_glb(&document).expect("fixture should write");
        let report = validate_vrm(&bytes).expect("validation should return a report");
        assert!(!report.valid);
        assert!(report.issues.iter().any(|issue| issue.code == "VRM_SPEC_VERSION"));
    }
}
