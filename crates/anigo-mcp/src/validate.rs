use anyhow::{bail, Result};

pub fn f32_finite(v: f64, name: &str) -> Result<f32> {
    let f = v as f32;
    if !f.is_finite() {
        bail!("{} must be finite, got {}", name, v);
    }
    Ok(f)
}

pub fn f32_range(v: f64, min: f32, max: f32, name: &str) -> Result<f32> {
    let f = f32_finite(v, name)?;
    if f < min || f > max {
        bail!("{} out of range [{}, {}]: {}", name, min, max, f);
    }
    Ok(f)
}

pub fn validate_vec3(arr: &[serde_json::Value], name: &str) -> Result<glam::Vec3> {
    if arr.len() != 3 {
        bail!("{} must have 3 elements", name);
    }
    let x = arr[0].as_f64().ok_or_else(|| anyhow::anyhow!("{}[0] not a number", name))?;
    let y = arr[1].as_f64().ok_or_else(|| anyhow::anyhow!("{}[1] not a number", name))?;
    let z = arr[2].as_f64().ok_or_else(|| anyhow::anyhow!("{}[2] not a number", name))?;
    let vx = f32_finite(x, &format!("{}[0]", name))?;
    let vy = f32_finite(y, &format!("{}[1]", name))?;
    let vz = f32_finite(z, &format!("{}[2]", name))?;
    Ok(glam::Vec3::new(vx, vy, vz))
}

// P1-07: Allowlist for VK codes
const ALLOWED_VK: &[u8] = &[
    0x25, 0x26, 0x27, 0x28, // arrows
    0x08, 0x09, 0x0D, 0x1B, // backspace, tab, enter, esc
    0x20, // space
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, // F1-F12
    // 0-9
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    // A-Z
    0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D,
    0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x2E, // delete
];

pub fn validate_vk(vk: u8) -> Result<()> {
    if !ALLOWED_VK.contains(&vk) {
        bail!("VK 0x{:02X} not allowed. Allowed: arrows, F1-F12, 0-9, A-Z, Backspace, Tab, Enter, Esc, Space, Delete (code -32602)", vk);
    }
    Ok(())
}

pub fn clamp_i32(v: i64, min: i32, max: i32, name: &str) -> Result<i32> {
    if v < min as i64 || v > max as i64 {
        bail!("{} out of range {}..{}: {}", name, min, max, v);
    }
    Ok(v as i32)
}


// ---------------------------------------------------------------------------------------------
// App nativo (T-2.13): validação de entrada das ferramentas `anigo_native_*`
// ---------------------------------------------------------------------------------------------

use serde_json::Value;

/// Teto do JSON de um `Command` vindo do cliente MCP (a bridge aceita até 1 MiB por quadro).
pub const MAX_COMMAND_JSON_BYTES: usize = 512 * 1024;
pub const MAX_JSON_DEPTH: usize = 32;
pub const NATIVE_WORKSPACES: &[&str] = &[
    "personagem", "modelagem", "posing", "shading", "iluminacao", "cenario", "animacao", "render", "biblioteca",
];
pub const NATIVE_ASSET_KINDS: &[&str] = &[
    "model", "character", "outfit", "hair", "accessory", "material", "texture", "pose", "animation", "hdri", "scene", "prefab",
];

/// Profundidade máxima de aninhamento de um JSON (escalares = 1).
pub fn json_depth(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(m) => 1 + m.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn has_proto_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "__proto__" || k == "constructor" || k == "prototype" {
                    return Some(k.clone());
                }
                if let Some(f) = has_proto_key(x) {
                    return Some(f);
                }
            }
            None
        }
        Value::Array(a) => a.iter().find_map(has_proto_key),
        _ => None,
    }
}

/// `Command` do contrato v1: objeto com `kind` (snake_case), limitado em tamanho e profundidade, sem chaves de
/// poluição de protótipo. A validação SEMÂNTICA (ids, faixas) é do núcleo, no `CoreSession::apply`.
pub fn validate_native_command(v: &Value) -> Result<()> {
    let obj = v.as_object().ok_or_else(|| anyhow::anyhow!("'command' must be an object like {{\"kind\": ...}} (code -32602)"))?;
    let kind = obj.get("kind").and_then(|k| k.as_str()).ok_or_else(|| anyhow::anyhow!("'command.kind' is required (string) (code -32602)"))?;
    let kind_ok = !kind.is_empty() && kind.len() <= 64 && kind.starts_with(|c: char| c.is_ascii_lowercase()) && kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !kind_ok {
        bail!("Invalid command kind {:?} (snake_case, max 64 chars) (code -32602)", kind);
    }
    if json_depth(v) > MAX_JSON_DEPTH {
        bail!("command nesting deeper than {} (code -32602)", MAX_JSON_DEPTH);
    }
    if let Some(k) = has_proto_key(v) {
        bail!("Invalid property name '{}' (prototype pollution blocked, code -32602)", k);
    }
    let size = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX);
    if size > MAX_COMMAND_JSON_BYTES {
        bail!("command JSON too large: {} > {} bytes (code -32602)", size, MAX_COMMAND_JSON_BYTES);
    }
    Ok(())
}

/// UUID textual (8-4-4-4-12 hexadecimal).
pub fn validate_uuid(s: &str, name: &str) -> Result<()> {
    let b = s.as_bytes();
    let ok = b.len() == 36
        && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() });
    if !ok {
        bail!("{} must be a UUID (8-4-4-4-12 hex), got {:?} (code -32602)", name, s.chars().take(40).collect::<String>());
    }
    Ok(())
}

pub fn validate_native_workspace(s: &str) -> Result<()> {
    if !NATIVE_WORKSPACES.contains(&s) {
        bail!("Unknown workspace '{}'. Allowed: {} (code -32602)", s, NATIVE_WORKSPACES.join(", "));
    }
    Ok(())
}

pub fn validate_native_asset_kind(s: &str) -> Result<()> {
    if !NATIVE_ASSET_KINDS.contains(&s) {
        bail!("Unknown asset kind '{}'. Allowed: {} (code -32602)", s, NATIVE_ASSET_KINDS.join(", "));
    }
    Ok(())
}

/// Texto livre curto, sem caracteres de controle.
pub fn validate_text(s: &str, name: &str, max: usize) -> Result<()> {
    if s.chars().count() > max {
        bail!("{} too long (max {} chars) (code -32602)", name, max);
    }
    if s.chars().any(|c| c.is_control()) {
        bail!("{} contains control characters (code -32602)", name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_finite_rejects_nan() {
        assert!(f32_finite(f64::NAN, "x").is_err());
        assert!(f32_finite(f64::INFINITY, "x").is_err());
        assert!(f32_finite(f64::NEG_INFINITY, "x").is_err());
        assert!(f32_finite(0.0, "x").is_ok());
    }

    #[test]
    fn f32_range_rejects_out_of_bounds() {
        assert!(f32_range(0.5, 0.0, 1.0, "v").is_ok());
        assert!(f32_range(-0.001, 0.0, 1.0, "v").is_err());
        assert!(f32_range(1.001, 0.0, 1.0, "v").is_err());
    }

    #[test]
    fn validate_vk_allows_arrows_blocks_win() {
        assert!(validate_vk(0x26).is_ok()); // up arrow
        assert!(validate_vk(0x41).is_ok()); // 'A'
        assert!(validate_vk(0x0D).is_ok()); // Enter
        assert!(validate_vk(0x5B).is_err()); // Win key
        assert!(validate_vk(0x12).is_err()); // Alt
        assert!(validate_vk(0x11).is_err()); // Ctrl
    }

    #[test]
    fn validate_vec3_accepts_triplets() {
        let v = vec![serde_json::json!(1.0), serde_json::json!(2.0), serde_json::json!(3.0)];
        assert!(validate_vec3(&v, "p").is_ok());
        let v2 = vec![serde_json::json!(1.0), serde_json::json!(2.0)];
        assert!(validate_vec3(&v2, "p").is_err());
    }

    #[test]
    fn clamp_i32_enforces_range() {
        assert!(clamp_i32(50, 0, 100, "x").is_ok());
        assert!(clamp_i32(-1, 0, 100, "x").is_err());
        assert!(clamp_i32(101, 0, 100, "x").is_err());
    }

    #[test]
    fn native_command_shape_is_validated() {
        use serde_json::json;
        assert!(validate_native_command(&json!({"kind": "rename_project", "name": "x"})).is_ok());
        assert!(validate_native_command(&json!({"kind": "batch", "commands": [{"kind": "reset_morphs"}]})).is_ok());
        for bad in [
            json!(null), json!("x"), json!([1]), json!({}), json!({"kind": 3}), json!({"kind": ""}), json!({"kind": "Rename"}),
            json!({"kind": "a b"}), json!({"kind": "9x"}), json!({"kind": "x".repeat(65)}),
            json!({"kind": "x", "__proto__": {"a": 1}}), json!({"kind": "x", "n": {"constructor": 1}}), json!({"kind": "x", "l": [{"prototype": 1}]}),
        ] {
            assert!(validate_native_command(&bad).is_err(), "deveria recusar {bad}");
        }
        let mut deep = json!({"kind": "x"});
        for _ in 0..40 {
            deep = json!({"kind": "x", "inner": deep});
        }
        assert!(validate_native_command(&deep).is_err());
        let huge = json!({"kind": "x", "blob": "a".repeat(MAX_COMMAND_JSON_BYTES + 1)});
        assert!(validate_native_command(&huge).is_err());
    }

    #[test]
    fn json_depth_counts_nesting() {
        use serde_json::json;
        assert_eq!(json_depth(&json!(1)), 1);
        assert_eq!(json_depth(&json!({"a": [1, {"b": 2}]})), 4);
        assert_eq!(json_depth(&json!([])), 1);
    }

    #[test]
    fn uuid_workspace_kind_and_text_validators() {
        assert!(validate_uuid("0197f3a2-1b2c-7d3e-8f40-0123456789ab", "id").is_ok());
        for bad in ["", "not-a-uuid", "0197f3a21b2c7d3e8f400123456789ab", "0197f3a2-1b2c-7d3e-8f40-0123456789a", "0197f3a2-1b2c-7d3e-8f40-0123456789zz", "0197f3a2_1b2c_7d3e_8f40_0123456789ab"] {
            assert!(validate_uuid(bad, "id").is_err(), "{bad}");
        }
        assert!(validate_native_workspace("cenario").is_ok());
        assert!(validate_native_workspace("modelagem").is_ok());
        assert!(validate_native_workspace("../etc").is_err());
        assert!(validate_native_asset_kind("model").is_ok());
        assert!(validate_native_asset_kind("exe").is_err());
        assert!(validate_text("cabelo azul", "text", 20).is_ok());
        assert!(validate_text(&"a".repeat(21), "text", 20).is_err());
        assert!(validate_text("a\u{0}b", "text", 20).is_err());
        assert!(validate_text("a\nb", "text", 20).is_err());
    }
}
