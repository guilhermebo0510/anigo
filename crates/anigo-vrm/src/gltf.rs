//! ANIGO — parser glTF 2.0 (lado Rust, espelho de `src/services/vrm/gltf2.ts`).
//!
//! O TS é a via primária (viewport + testes em Node); este módulo garante que
//! o core Rust tenha a MESMA capacidade de interoperabilidade para os comandos
//! Tauri (`validate_model`, `import_model`, `export_model`) e para o headless.
//!
//! Disciplina idêntica do contrato: erros com **códigos estáveis**
//! (`GltfError`), sem falha silenciosa.
//!
//! Nota de ambiente: sem `cargo` neste sandbox — a validação de sintaxe é via
//! `scripts/check_rust_syntax.mjs`; `cargo test` roda na CI (issue #59).

use serde::Serialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Constantes da spec
// ---------------------------------------------------------------------------

/// Component types da spec glTF 2.0 (3.14.1): 5120 BYTE, 5121 UNSIGNED_BYTE,
/// 5122 UNSIGNED_SHORT, 5123 UNSIGNED_INT, 5125 DOUBLE, 5126 FLOAT.
pub const COMPONENT_U8: u64 = 5120;
pub const COMPONENT_U8N: u64 = 5121;
pub const COMPONENT_U16: u64 = 5122;
pub const COMPONENT_U32: u64 = 5123;
pub const COMPONENT_F64: u64 = 5125;
pub const COMPONENT_F32: u64 = 5126;

const GLB_MAGIC: u32 = 0x4654_6c67; // "glTF"
const GLB_VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4e4f_534a; // "JSON"
const CHUNK_BIN: u32 = 0x004e_4942; // "BIN\0"

// ---------------------------------------------------------------------------
// Erros (códigos estáveis — paridade com o TS)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GltfErrorCode {
    BadAsset,
    UnsupportedVersion,
    MissingBuffer,
    BufferLengthMismatch,
    BadBufferIndex,
    BadBufferViewIndex,
    BufferViewOutOfRange,
    BadAccessorIndex,
    AccessorOutOfRange,
    BadAccessorComponent,
    BadAccessorType,
    BadAccessorCount,
    SparseOutOfRange,
    BadImageIndex,
    BadNodeIndex,
    BadSkinIndex,
    BadSceneIndex,
    NodeGraphCycle,
    MissingPosition,
    MissingBin,
    BadJson,
    UnknownError,
}

impl GltfErrorCode {
    /// Código serializável (idêntico ao que o TS emite).
    pub fn as_str(&self) -> &'static str {
        match self {
            GltfErrorCode::BadAsset => "BAD_ASSET",
            GltfErrorCode::UnsupportedVersion => "UNSUPPORTED_VERSION",
            GltfErrorCode::MissingBuffer => "MISSING_BUFFER",
            GltfErrorCode::BufferLengthMismatch => "BUFFER_LENGTH_MISMATCH",
            GltfErrorCode::BadBufferIndex => "BAD_BUFFER_INDEX",
            GltfErrorCode::BadBufferViewIndex => "BAD_BUFFERVIEW_INDEX",
            GltfErrorCode::BufferViewOutOfRange => "BUFFERVIEW_OUT_OF_RANGE",
            GltfErrorCode::BadAccessorIndex => "BAD_ACCESSOR_INDEX",
            GltfErrorCode::AccessorOutOfRange => "ACCESSOR_OUT_OF_RANGE",
            GltfErrorCode::BadAccessorComponent => "BAD_ACCESSOR_COMPONENT",
            GltfErrorCode::BadAccessorType => "BAD_ACCESSOR_TYPE",
            GltfErrorCode::BadAccessorCount => "BAD_ACCESSOR_COUNT",
            GltfErrorCode::SparseOutOfRange => "SPARSE_OUT_OF_RANGE",
            GltfErrorCode::BadImageIndex => "BAD_IMAGE_INDEX",
            GltfErrorCode::BadNodeIndex => "BAD_NODE_INDEX",
            GltfErrorCode::BadSkinIndex => "BAD_SKIN_INDEX",
            GltfErrorCode::BadSceneIndex => "BAD_SCENE_INDEX",
            GltfErrorCode::NodeGraphCycle => "NODE_GRAPH_CYCLE",
            GltfErrorCode::MissingPosition => "MISSING_POSITION",
            GltfErrorCode::MissingBin => "MISSING_BIN",
            GltfErrorCode::BadJson => "BAD_JSON",
            GltfErrorCode::UnknownError => "UNKNOWN_ERROR",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub struct GltfError {
    pub code: GltfErrorCode,
    pub message: String,
}

impl GltfError {
    pub fn new(code: GltfErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for GltfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[gltf {}] {}", self.code.as_str(), self.message)
    }
}

fn fail(code: GltfErrorCode, message: impl Into<String>) -> GltfError {
    GltfError::new(code, message)
}

// ---------------------------------------------------------------------------
// Modelo do documento (acesso por `Value` — mantém extensions/extras cruas
// para roundtrip sem perdas)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParsedGltf {
    pub json: Value,
    pub buffers: Vec<Vec<u8>>,
    pub warnings: Vec<String>,
}

fn arr<'a>(json: &'a Value, key: &str) -> &'a [Value] {
    json.get(key).and_then(Value::as_array).map(|v| v.as_slice()).unwrap_or(&[])
}

pub fn accessor_list(json: &Value) -> &[Value] {
    arr(json, "accessors")
}

/// Contagem de componentes por tipo (spec 3.14.3); 0 = inválido.
pub fn accessor_component_count(tpe: &str) -> u64 {
    match tpe {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        _ => 0,
    }
}

/// Bytes por componente (spec 3.14.1); 0 = inválido.
fn component_bytes(component_type: u64) -> u64 {
    match component_type {
        COMPONENT_U8 | COMPONENT_U8N => 1,
        COMPONENT_U16 => 2,
        COMPONENT_U32 | COMPONENT_F32 => 4,
        COMPONENT_F64 => 8,
        _ => 0,
    }
}

fn accessor_byte_size(tpe: &str, component_type: u64) -> u64 {
    accessor_component_count(tpe).saturating_mul(component_bytes(component_type))
}

fn accessor_field_tpe(accessor: &Value) -> &str {
    accessor.get("type").and_then(Value::as_str).unwrap_or("")
}

fn accessor_field_component(accessor: &Value) -> u64 {
    accessor.get("componentType").and_then(Value::as_u64).unwrap_or(0)
}

fn accessor_field_count(accessor: &Value) -> u64 {
    accessor.get("count").and_then(Value::as_u64).unwrap_or(0)
}

fn in_range(idx: Option<&Value>, len: usize, where_: &str) -> Result<(), GltfError> {
    match idx {
        None => Ok(()),
        Some(Value::Null) => Ok(()),
        Some(v) => match v.as_u64() {
            Some(n) if (n as usize) < len => Ok(()),
            _ => Err(fail(GltfErrorCode::UnknownError, format!("{where_} aponta para índice inválido"))),
        },
    }
}

// ---------------------------------------------------------------------------
// Validação estrutural
// ---------------------------------------------------------------------------

/// Valida a estrutura do documento e resolve os buffers.
///
/// `buffer_resolver(i, uri)`: bytes do `buffers[i]` (GLB BIN para buffers sem
/// `uri`, ou fonte externa para `.gltf`).
pub fn parse_gltf(
    json: Value,
    buffer_resolver: impl Fn(usize, Option<&str>) -> Result<Vec<u8>, GltfError>,
) -> Result<ParsedGltf, GltfError> {
    let warnings: Vec<String> = Vec::new();
    let version = json
        .get("asset")
        .and_then(|a| a.get("version"))
        .and_then(Value::as_str)
        .ok_or_else(|| fail(GltfErrorCode::BadAsset, "campo asset.version ausente"))?;
    let major = version.split('.').next().unwrap_or("0");
    if major != "2" {
        return Err(fail(
            GltfErrorCode::UnsupportedVersion,
            format!("asset.version {version} (esperado major 2)"),
        ));
    }
    if let Some(required) = json.pointer("/asset/extensionsRequired").and_then(Value::as_array) {
        for ext in required.iter().filter_map(Value::as_str) {
            if ext == "KHR_draco_mesh_compression" {
                return Err(fail(
                    GltfErrorCode::BadAsset,
                    format!("extensão obrigatória não suportada: {ext}"),
                ));
            }
        }
    }

    let buffer_specs = arr(&json, "buffers");
    let mut buffers: Vec<Vec<u8>> = Vec::with_capacity(buffer_specs.len());
    for (i, spec) in buffer_specs.iter().enumerate() {
        let uri = spec.get("uri").and_then(Value::as_str);
        let declared = spec.get("byteLength").and_then(Value::as_u64).unwrap_or(0);
        let bytes = buffer_resolver(i, uri)?;
        if bytes.len() as u64 != declared {
            return Err(fail(
                GltfErrorCode::BufferLengthMismatch,
                format!("buffers[{i}]: byteLength={declared} mas a fonte tem {} bytes", bytes.len()),
            ));
        }
        buffers.push(bytes);
    }

    let views = arr(&json, "bufferViews");
    for (i, view) in views.iter().enumerate() {
        let start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
        let len = view.get("byteLength").and_then(Value::as_u64).unwrap_or(0);
        match view.get("buffer").and_then(Value::as_u64) {
            Some(b) if (b as usize) < buffers.len() => {
                let buffer_len = buffers[b as usize].len() as u64;
                if start > buffer_len || start.wrapping_add(len) > buffer_len {
                    return Err(fail(
                        GltfErrorCode::BufferViewOutOfRange,
                        format!("bufferViews[{i}] ultrapassa o buffer (start {start} + len {len})"),
                    ));
                }
            }
            _ => {
                return Err(fail(
                    GltfErrorCode::BadBufferIndex,
                    format!("bufferViews[{i}].buffer fora da faixa"),
                ))
            }
        }
    }

    let accessors = arr(&json, "accessors");
    for (i, accessor) in accessors.iter().enumerate() {
        if accessor.get("bufferView").is_none() && accessor.get("sparse").is_none() {
            return Err(fail(
                GltfErrorCode::BadAccessorIndex,
                format!("accessors[{i}] sem bufferView nem sparse"),
            ));
        }
        if let Some(bv) = accessor.get("bufferView").and_then(Value::as_u64) {
            if bv as usize >= views.len() {
                return Err(fail(
                    GltfErrorCode::BadBufferViewIndex,
                    format!("accessors[{i}].bufferView fora da faixa"),
                ));
            }
        }
        let tpe = accessor_field_tpe(accessor);
        if accessor_component_count(tpe) == 0 {
            return Err(fail(
                GltfErrorCode::BadAccessorType,
                format!("accessors[{i}].type='{tpe}' inválido"),
            ));
        }
        let comp = accessor_field_component(accessor);
        if component_bytes(comp) == 0 {
            return Err(fail(
                GltfErrorCode::BadAccessorComponent,
                format!("accessors[{i}].componentType={comp} inválido"),
            ));
        }
        if accessor_field_count(accessor) == u64::MAX {
            return Err(fail(GltfErrorCode::BadAccessorCount, format!("accessors[{i}].count inválido")));
        }
    }

    let textures = arr(&json, "textures");
    for (i, texture) in textures.iter().enumerate() {
        in_range(texture.get("sampler"), arr(&json, "samplers").len(), &format!("textures[{i}].sampler"))?;
        in_range(texture.get("source"), arr(&json, "images").len(), &format!("textures[{i}].source"))?;
    }
    let meshes = arr(&json, "meshes");
    for (i, mesh) in meshes.iter().enumerate() {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            return Err(fail(GltfErrorCode::BadAccessorIndex, format!("meshes[{i}] sem primitivas")));
        };
        if primitives.is_empty() {
            return Err(fail(GltfErrorCode::BadAccessorIndex, format!("meshes[{i}] sem primitivas")));
        }
        for (p_i, prim) in primitives.iter().enumerate() {
            let Some(attributes) = prim.get("attributes").and_then(Value::as_object) else {
                return Err(fail(
                    GltfErrorCode::MissingPosition,
                    format!("meshes[{i}].primitives[{p_i}] sem attributes"),
                ));
            };
            if attributes.get("POSITION").is_none() {
                return Err(fail(
                    GltfErrorCode::MissingPosition,
                    format!("meshes[{i}].primitives[{p_i}] sem POSITION"),
                ));
            }
            for (semantic, idx) in attributes {
                in_range(Some(idx), accessors.len(), &format!("meshes[{i}].primitives[{p_i}].{semantic}"))?;
            }
            in_range(
                prim.get("indices"),
                accessors.len(),
                &format!("meshes[{i}].primitives[{p_i}].indices"),
            )?;
            in_range(
                prim.get("material"),
                arr(&json, "materials").len(),
                &format!("meshes[{i}].primitives[{p_i}].material"),
            )?;
            if let Some(targets) = prim.get("targets").and_then(Value::as_array) {
                for target in targets {
                    if let Some(object) = target.as_object() {
                        for idx in object.values() {
                            in_range(
                                Some(idx),
                                accessors.len(),
                                &format!("meshes[{i}].primitives[{p_i}].targets"),
                            )?;
                        }
                    }
                }
            }
        }
    }
    let nodes = arr(&json, "nodes");
    for (i, node) in nodes.iter().enumerate() {
        in_range(node.get("mesh"), meshes.len(), &format!("nodes[{i}].mesh"))?;
        in_range(node.get("skin"), arr(&json, "skins").len(), &format!("nodes[{i}].skin"))?;
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                in_range(Some(child), nodes.len(), &format!("nodes[{i}].children"))?;
            }
        }
    }
    for (i, skin) in arr(&json, "skins").iter().enumerate() {
        let Some(joints) = skin.get("joints").and_then(Value::as_array) else {
            return Err(fail(GltfErrorCode::BadSkinIndex, format!("skins[{i}].joints vazio")));
        };
        if joints.is_empty() {
            return Err(fail(GltfErrorCode::BadSkinIndex, format!("skins[{i}].joints vazio")));
        }
        for joint in joints {
            in_range(Some(joint), nodes.len(), &format!("skins[{i}].joints"))?;
        }
        in_range(
            skin.get("inverseBindMatrices"),
            accessors.len(),
            &format!("skins[{i}].inverseBindMatrices"),
        )?;
    }
    in_range(json.get("scene"), arr(&json, "scenes").len(), "scene")?;

    detect_node_cycles(nodes)?;

    Ok(ParsedGltf { json, buffers, warnings })
}

/// DFS iterativo com estados (branco/cinza/preto) para detectar ciclos.
fn detect_node_cycles(nodes: &[Value]) -> Result<(), GltfError> {
    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;
    let mut color = vec![WHITE; nodes.len()];
    for start in 0..nodes.len() {
        if color[start] != WHITE {
            continue;
        }
        // stack: (node, índice do próximo filho a visitar)
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        color[start] = GRAY;
        while let Some((idx, mut next_child)) = stack.pop() {
            let children = nodes[idx]
                .get("children")
                .and_then(Value::as_array)
                .map(|c| c.iter().filter_map(Value::as_u64).filter(|c| (*c as usize) < nodes.len()).collect::<Vec<_>>())
                .unwrap_or_default();
            let advanced = loop {
                if next_child >= children.len() {
                    break false;
                }
                let child = children[next_child] as usize;
                next_child += 1;
                match color[child] {
                    WHITE => {
                        color[child] = GRAY;
                        stack.push((idx, next_child));
                        stack.push((child, 0));
                        break true;
                    }
                    GRAY => {
                        return Err(fail(
                            GltfErrorCode::NodeGraphCycle,
                            format!("ciclo detectado em nodes[{idx}] → nodes[{child}]"),
                        ))
                    }
                    BLACK => continue,
                    _ => unreachable!(),
                }
            };
            if !advanced {
                color[idx] = BLACK;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// GLB container
// ---------------------------------------------------------------------------

/// Parse do contêiner GLB → (JSON, bytes do chunk BIN).
pub fn parse_glb(bytes: &[u8]) -> Result<(Value, Vec<u8>), GltfError> {
    if bytes.len() < 20 {
        return Err(fail(
            GltfErrorCode::BadJson,
            format!("buffer tem {} bytes, precisa >= 20", bytes.len()),
        ));
    }
    let dv = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    if dv(0) != GLB_MAGIC {
        return Err(fail(GltfErrorCode::BadJson, format!("magic 0x{:08x} != glTF", dv(0))));
    }
    if dv(4) != GLB_VERSION {
        return Err(fail(GltfErrorCode::UnsupportedVersion, format!("GLB version {}", dv(4))));
    }
    let length = dv(8) as usize;
    if length > bytes.len() || length < 20 {
        return Err(fail(
            GltfErrorCode::BadJson,
            format!("declara {length} bytes, buffer tem {}", bytes.len()),
        ));
    }
    let mut json_bytes: Option<&[u8]> = None;
    let mut bin: Option<&[u8]> = None;
    let mut offset = 12usize;
    while offset + 8 <= length {
        let chunk_len = dv(offset) as usize;
        let chunk_type = dv(offset + 4);
        let data_start = offset + 8;
        let data_end = data_start.saturating_add(chunk_len);
        if data_end > bytes.len() {
            break;
        }
        if chunk_type == CHUNK_JSON && json_bytes.is_none() {
            json_bytes = Some(&bytes[data_start..data_end]);
        } else if chunk_type == CHUNK_BIN && bin.is_none() {
            bin = Some(&bytes[data_start..data_end]);
        }
        offset = data_end;
    }
    let json_bytes = json_bytes.ok_or_else(|| fail(GltfErrorCode::BadJson, "sem chunk JSON"))?;
    let json: Value = serde_json::from_slice(json_bytes)
        .map_err(|e| fail(GltfErrorCode::BadJson, format!("JSON inválido: {e}")))?;
    Ok((json, bin.unwrap_or(&[]).to_vec()))
}

/// Resolve buffers de um GLB: sem `uri` → chunk BIN (a spec permite no máximo
/// um); com `uri` → data-URI ou `external`.
pub fn resolve_glb_buffers(
    json: &Value,
    bin: &[u8],
    external: impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<(Vec<Vec<u8>>, Vec<String>), GltfError> {
    let specs = arr(json, "buffers");
    let mut uriless = 0usize;
    let mut warnings: Vec<String> = Vec::new();
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(specs.len());
    for spec in specs {
        match spec.get("uri").and_then(Value::as_str) {
            None => {
                uriless += 1;
                out.push(bin.to_vec());
            }
            Some(u) => {
                if let Some(bytes) = external(u) {
                    out.push(bytes);
                } else {
                    return Err(fail(
                        GltfErrorCode::MissingBuffer,
                        format!("buffers: URI externa '{u}' não resolvida"),
                    ));
                }
            }
        }
    }
    if uriless > 1 {
        warnings.push(format!(
            "GLB com {uriless} buffers sem uri — a spec permite no máximo um no chunk BIN"
        ));
    }
    Ok((out, warnings))
}

/// Monta um GLB (header 12 B + JSON chunk + BIN chunk) com padding canônico
/// (JSON→0x20, BIN→0x00). Determinístico para o mesmo JSON/bin.
pub fn build_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let json_text = serde_json::to_string(json).unwrap_or_else(|_| "{}".to_string());
    let json_bytes = json_text.as_bytes();
    let pad_json = (4 - (json_bytes.len() % 4)) % 4;
    let pad_bin = (4 - (bin.len() % 4)) % 4;
    let has_bin = !bin.is_empty() || pad_bin > 0;
    let total = 12 + 8 + json_bytes.len() + pad_json + if has_bin { 8 + bin.len() + pad_bin } else { 0 };
    let mut out = vec![0u8; total];
    out[0..4].copy_from_slice(&GLB_MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&GLB_VERSION.to_le_bytes());
    out[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    let mut cursor = 12usize;
    out[cursor..cursor + 4].copy_from_slice(&((json_bytes.len() + pad_json) as u32).to_le_bytes());
    out[cursor + 4..cursor + 8].copy_from_slice(&CHUNK_JSON.to_le_bytes());
    cursor += 8;
    out[cursor..cursor + json_bytes.len()].copy_from_slice(json_bytes);
    for i in 0..pad_json {
        out[cursor + json_bytes.len() + i] = 0x20;
    }
    cursor += json_bytes.len() + pad_json;
    if has_bin {
        out[cursor..cursor + 4].copy_from_slice(&((bin.len() + pad_bin) as u32).to_le_bytes());
        out[cursor + 4..cursor + 8].copy_from_slice(&CHUNK_BIN.to_le_bytes());
        cursor += 8;
        out[cursor..cursor + bin.len()].copy_from_slice(bin);
        for i in 0..pad_bin {
            out[cursor + bin.len() + i] = 0x00;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Leitura de accessors (paridade com o TS)
// ---------------------------------------------------------------------------

/// Região bruta do accessor: (bytes, stride do view, offset inicial).
fn accessor_region(parsed: &ParsedGltf, accessor: &Value) -> Result<(Vec<u8>, Option<u64>, u64), GltfError> {
    let offset = accessor.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
    match accessor.get("bufferView").and_then(Value::as_u64) {
        None => {
            if accessor.get("sparse").is_none() {
                return Err(fail(GltfErrorCode::BadAccessorIndex, "accessor sem bufferView nem sparse"));
            }
            let size = accessor_byte_size(accessor_field_tpe(accessor), accessor_field_component(accessor))
                * accessor_field_count(accessor);
            Ok((vec![0u8; size as usize], None, 0))
        }
        Some(bv) => {
            let views = arr(&parsed.json, "bufferViews");
            if bv as usize >= views.len() {
                return Err(fail(GltfErrorCode::BadBufferViewIndex, "accessor.bufferView fora da faixa"));
            }
            let view = &views[bv as usize];
            let buffer = view.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
            if buffer >= parsed.buffers.len() {
                return Err(fail(GltfErrorCode::BadBufferIndex, "accessor aponta para buffer inexistente"));
            }
            let start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) + offset;
            let element = accessor_byte_size(accessor_field_tpe(accessor), accessor_field_component(accessor));
            let stride = view.get("byteStride").and_then(Value::as_u64).unwrap_or(element);
            let count = accessor_field_count(accessor);
            let end = start.saturating_add(count.saturating_sub(1).saturating_mul(stride)).saturating_add(element);
            if end > parsed.buffers[buffer].len() as u64 {
                return Err(fail(
                    GltfErrorCode::AccessorOutOfRange,
                    format!(
                        "accessor ultrapassa o buffer (end {end} > {})",
                        parsed.buffers[buffer].len()
                    ),
                ));
            }
            Ok((
                parsed.buffers[buffer].clone(),
                view.get("byteStride").and_then(Value::as_u64),
                start,
            ))
        }
    }
}

fn decode_scalar(bytes: &[u8], offset: u64, component_type: u64, normalized: bool) -> Result<f32, GltfError> {
    let needed = component_bytes(component_type) as usize;
    if offset as usize + needed > bytes.len() {
        return Err(fail(GltfErrorCode::AccessorOutOfRange, "leitura de componente fora do buffer"));
    }
    let base = offset as usize;
    Ok(match component_type {
        COMPONENT_U8 => bytes[base] as f32,
        COMPONENT_U8N => {
            if normalized {
                bytes[base] as f32 / 255.0
            } else {
                bytes[base] as f32
            }
        }
        COMPONENT_U16 => {
            let v = u16::from_le_bytes([bytes[base], bytes[base + 1]]);
            if normalized {
                v as f32 / 65535.0
            } else {
                v as f32
            }
        }
        COMPONENT_U32 => {
            u32::from_le_bytes([bytes[base], bytes[base + 1], bytes[base + 2], bytes[base + 3]]) as f32
        }
        COMPONENT_F64 => {
            f64::from_le_bytes([
                bytes[base],
                bytes[base + 1],
                bytes[base + 2],
                bytes[base + 3],
                bytes[base + 4],
                bytes[base + 5],
                bytes[base + 6],
                bytes[base + 7],
            ]) as f32
        }
        COMPONENT_F32 => {
            f32::from_le_bytes([bytes[base], bytes[base + 1], bytes[base + 2], bytes[base + 3]])
        }
        _ => {
            return Err(fail(
                GltfErrorCode::BadAccessorComponent,
                format!("componentType {component_type} não decodificável"),
            ))
        }
    })
}

/// Lê um accessor como `Vec<f32>` (count × componentes). Paridade com o TS:
/// stride (interleaved), quantização normalizada e sparse.
pub fn read_accessor_f32(parsed: &ParsedGltf, accessor_index: usize) -> Result<Vec<f32>, GltfError> {
    let accessors = accessor_list(&parsed.json);
    let Some(accessor) = accessors.get(accessor_index) else {
        return Err(fail(
            GltfErrorCode::BadAccessorIndex,
            format!("accessor {accessor_index} fora da faixa"),
        ));
    };
    let comps = accessor_component_count(accessor_field_tpe(accessor)) as usize;
    if comps == 0 {
        return Err(fail(
            GltfErrorCode::BadAccessorType,
            format!("type '{}' inválido", accessor_field_tpe(accessor)),
        ));
    }
    let component_type = accessor_field_component(accessor);
    let scalar_bytes = component_bytes(component_type) as usize;
    let count = accessor_field_count(accessor) as usize;
    let normalized = accessor.get("normalized").and_then(Value::as_bool).unwrap_or(false);
    let (bytes, view_stride, start) = accessor_region(parsed, accessor)?;
    let element_bytes = scalar_bytes * comps;
    let stride = view_stride.unwrap_or(element_bytes as u64) as usize;
    if stride < element_bytes {
        return Err(fail(
            GltfErrorCode::AccessorOutOfRange,
            format!("byteStride {stride} menor que o tamanho do accessor"),
        ));
    }
    let mut out = vec![0f32; count * comps];
    for i in 0..count {
        let base = start + (i as u64).saturating_mul(stride as u64);
        for c in 0..comps {
            out[i * comps + c] = decode_scalar(&bytes, base + (c * scalar_bytes) as u64, component_type, normalized)?;
        }
    }
    if accessor.get("sparse").is_some() {
        apply_sparse(parsed, accessor, &mut out, comps, scalar_bytes)?;
    }
    Ok(out)
}

fn apply_sparse(parsed: &ParsedGltf, accessor: &Value, out: &mut [f32], comps: usize, scalar_bytes: usize) -> Result<(), GltfError> {
    let Some(sparse) = accessor.get("sparse").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(indices_spec) = sparse.get("indices") else {
        return Ok(());
    };
    let Some(values_spec) = sparse.get("values") else {
        return Ok(());
    };
    let views = arr(&parsed.json, "bufferViews");
    let index_bv = indices_spec.get("bufferView").and_then(Value::as_u64).unwrap_or(0) as usize;
    if index_bv >= views.len() {
        return Err(fail(GltfErrorCode::SparseOutOfRange, "sparse.indices.bufferView fora da faixa"));
    }
    let index_view = &views[index_bv];
    let index_buffer = index_view.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
    if index_buffer >= parsed.buffers.len() {
        return Err(fail(GltfErrorCode::SparseOutOfRange, "sparse.indices.buffer fora da faixa"));
    }
    let index_bytes = &parsed.buffers[index_buffer];
    let index_start =
        index_view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)
            + indices_spec.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
    let index_component = indices_spec.get("componentType").and_then(Value::as_u64).unwrap_or(0);
    let index_count = indices_spec.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let index_stride = index_view.get("byteStride").and_then(Value::as_u64).unwrap_or(0) as usize;
    let value_bv = values_spec.get("bufferView").and_then(Value::as_u64).unwrap_or(0) as usize;
    if value_bv >= views.len() {
        return Err(fail(GltfErrorCode::SparseOutOfRange, "sparse.values.bufferView fora da faixa"));
    }
    let value_view = &views[value_bv];
    let value_buffer = value_view.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
    if value_buffer >= parsed.buffers.len() {
        return Err(fail(GltfErrorCode::SparseOutOfRange, "sparse.values.buffer fora da faixa"));
    }
    let value_bytes = &parsed.buffers[value_buffer];
    let value_start =
        value_view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0)
            + values_spec.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
    let accessor_count = accessor_field_count(accessor) as usize;
    let component_type = accessor_field_component(accessor);
    let normalized = accessor.get("normalized").and_then(Value::as_bool).unwrap_or(false);
    for i in 0..index_count {
        let index_offset = index_start + (i as u64).saturating_mul(index_stride as u64);
        let vertex_index = match index_component {
            COMPONENT_U8 => index_bytes.get(index_offset as usize).copied().unwrap_or(0) as usize,
            COMPONENT_U16 => index_bytes
                .get(index_offset as usize..index_offset as usize + 2)
                .and_then(|w| w.try_into().ok())
                .map(u16::from_le_bytes)
                .unwrap_or(0) as usize,
            _ => index_bytes
                .get(index_offset as usize..index_offset as usize + 4)
                .and_then(|w| w.try_into().ok())
                .map(u32::from_le_bytes)
                .unwrap_or(0) as usize,
        };
        if vertex_index >= accessor_count {
            return Err(fail(GltfErrorCode::SparseOutOfRange, format!("sparse índice {vertex_index} fora da faixa")));
        }
        for c in 0..comps {
            out[vertex_index * comps + c] = decode_scalar(
                value_bytes,
                value_start + ((i * comps + c) * scalar_bytes) as u64,
                component_type,
                normalized,
            )?;
        }
    }
    Ok(())
}

/// Lê um accessor de índices como `Vec<u32>` (uint8/uint16/uint32).
pub fn read_accessor_indices(parsed: &ParsedGltf, accessor_index: usize) -> Result<Vec<u32>, GltfError> {
    let accessors = accessor_list(&parsed.json);
    let Some(accessor) = accessors.get(accessor_index) else {
        return Err(fail(
            GltfErrorCode::BadAccessorIndex,
            format!("accessor {accessor_index} fora da faixa"),
        ));
    };
    let component_type = accessor_field_component(accessor);
    if component_type != COMPONENT_U8 && component_type != COMPONENT_U16 && component_type != COMPONENT_U32 {
        return Err(fail(
            GltfErrorCode::BadAccessorComponent,
            format!("índices exigem uint8/uint16/uint32 (recebido {component_type})"),
        ));
    }
    let count = accessor_field_count(accessor) as usize;
    let (bytes, view_stride, start) = accessor_region(parsed, accessor)?;
    let stride = view_stride.unwrap_or(component_bytes(component_type)) as usize;
    let mut out = vec![0u32; count];
    for i in 0..count {
        let off = start + (i as u64).saturating_mul(stride as u64);
        out[i] = match component_type {
            COMPONENT_U8 => bytes.get(off as usize).copied().unwrap_or(0) as u32,
            COMPONENT_U16 => bytes
                .get(off as usize..off as usize + 2)
                .and_then(|w| w.try_into().ok())
                .map(u16::from_le_bytes)
                .unwrap_or(0) as u32,
            _ => bytes
                .get(off as usize..off as usize + 4)
                .and_then(|w| w.try_into().ok())
                .map(u32::from_le_bytes)
                .unwrap_or(0),
        };
    }
    Ok(out)
}

/// Contagem de mips para uma textura: `floor(log2(max(w,h)))+1` (issue #26).
pub fn mip_level_count(width: u32, height: u32) -> u32 {
    let max_side = width.max(height);
    if max_side <= 1 {
        return 1;
    }
    // floor(log2(n)) = bit_length(n) - 1; mips = floor(log2) + 1 = bit_length.
    u32::BITS - max_side.leading_zeros()
}

// ---------------------------------------------------------------------------
// Sumário de importação (paridade com o TS)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportSummary {
    pub title: String,
    pub gltf_version: String,
    pub vertex_count: u64,
    pub index_count: u64,
    pub triangle_count: u64,
    pub mesh_count: usize,
    pub node_count: usize,
    pub material_count: usize,
    pub skin_count: usize,
    pub animation_count: usize,
    pub texture_count: usize,
    pub morph_target_count: u64,
    pub is_vrm: bool,
    pub vrm_extensions: Vec<String>,
}

/// Resumo estrutural (o que a UI/Tauri mostram após validar).
pub fn summarize_import(json: &Value) -> ImportSummary {
    let accessors = accessor_list(json);
    let meshes = arr(json, "meshes");
    let mut vertex_count = 0u64;
    let mut index_count = 0u64;
    let mut morph_target_count = 0u64;
    for mesh in meshes {
        for prim in mesh.get("primitives").and_then(Value::as_array).into_iter().flatten() {
            if let Some(pos) = prim.pointer("/attributes/POSITION").and_then(Value::as_u64) {
                if let Some(accessor) = accessors.get(pos as usize) {
                    vertex_count += accessor_field_count(accessor);
                }
            }
            if let Some(idx) = prim.get("indices").and_then(Value::as_u64) {
                if let Some(accessor) = accessors.get(idx as usize) {
                    index_count += accessor_field_count(accessor);
                }
            }
            if let Some(targets) = prim.get("targets").and_then(Value::as_array) {
                morph_target_count += targets.len() as u64;
            }
        }
    }
    let used: Vec<String> = json
        .pointer("/asset/extensionsUsed")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    let is_vrm = used.iter().any(|e| e == "VRMC_vrm");
    let title = if is_vrm {
        json.pointer("/extensions/VRMC_vrm/meta/title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    ImportSummary {
        title,
        gltf_version: json.pointer("/asset/version").and_then(Value::as_str).unwrap_or("").to_string(),
        vertex_count,
        index_count,
        triangle_count: index_count / 3,
        mesh_count: meshes.len(),
        node_count: arr(json, "nodes").len(),
        material_count: arr(json, "materials").len(),
        skin_count: arr(json, "skins").len(),
        animation_count: arr(json, "animations").len(),
        texture_count: arr(json, "textures").len(),
        morph_target_count,
        is_vrm,
        vrm_extensions: used.into_iter().filter(|e| e.starts_with("VRMC_")).collect(),
    }
}
