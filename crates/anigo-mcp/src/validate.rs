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
}
