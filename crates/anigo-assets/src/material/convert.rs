//! Conversores para o [`ToonMaterial`]: glTF PBR, `VRMC_materials_mtoon`, MTL (OBJ) e FBX.
//!
//! Estado de validacao: testado com documentos glTF/MTL **construidos nos testes** a partir das
//! propriedades da spec (nomes e valores padrao do `VRMC_materials_mtoon` 1.0). NAO foi testado com
//! arquivos reais exportados por VRoid/UniVRM/Blender.

use serde_json::Value;

use super::*;

/// Resultado de uma conversao.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub material: ToonMaterial,
    /// Perdas/ajustes (extensoes ignoradas, texturas com URI insegura, valores invalidos...).
    pub warnings: Vec<String>,
}

/// Parametros PBR genericos (glTF metal-rough, MTL, FBX) antes de virar toon.
#[derive(Debug, Clone, PartialEq)]
pub struct PbrParams {
    pub name: String,
    pub base_color: [f32; 4],
    pub base_color_texture: Option<TextureRef>,
    pub metallic: f32,
    pub roughness: f32,
    pub metal_rough_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<TextureRef>,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub emissive_texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    /// `KHR_materials_unlit`: sem sombreamento.
    pub unlit: bool,
}

impl Default for PbrParams {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0; 4],
            base_color_texture: None,
            metallic: 0.0,
            roughness: 1.0,
            metal_rough_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            unlit: false,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers de JSON (tolerantes: tipo errado = valor padrao + aviso)
// ---------------------------------------------------------------------------------------------

fn num(v: Option<&Value>, default: f32) -> f32 {
    v.and_then(Value::as_f64).filter(|f| f.is_finite()).map(|f| f as f32).unwrap_or(default)
}

fn vecn<const N: usize>(v: Option<&Value>, default: [f32; N]) -> [f32; N] {
    let Some(a) = v.and_then(Value::as_array) else { return default };
    if a.len() != N {
        return default;
    }
    let mut out = default;
    for (i, x) in a.iter().enumerate() {
        match x.as_f64().filter(|f| f.is_finite()) {
            Some(f) => out[i] = f as f32,
            None => return default,
        }
    }
    out
}

fn boolean(v: Option<&Value>, default: bool) -> bool {
    v.and_then(Value::as_bool).unwrap_or(default)
}

/// Resolve `{ "index": N, "texCoord": M, "scale"/"strength": s }` para uma [`TextureRef`].
fn tex_ref(json: &Value, info: Option<&Value>, role: TextureRole, warnings: &mut Vec<String>) -> Option<TextureRef> {
    let info = info?.as_object()?;
    let idx = info.get("index")?.as_u64()? as usize;
    let tex = json.get("textures")?.as_array()?.get(idx)?;
    // fonte: source, ou a das extensoes de textura comprimida
    let src = tex
        .get("source")
        .and_then(Value::as_u64)
        .or_else(|| tex.pointer("/extensions/KHR_texture_basisu/source").and_then(Value::as_u64))
        .or_else(|| tex.pointer("/extensions/EXT_texture_webp/source").and_then(Value::as_u64))?;
    let img = json.get("images")?.as_array()?.get(src as usize)?;
    let mut t = match img.get("uri").and_then(Value::as_str) {
        Some(u) if !u.starts_with("data:") => match crate::paths::normalize_relative(u) {
            Ok(rel) => TextureRef::from_uri(role, rel),
            Err(e) => {
                warnings.push(format!("textura {idx} ignorada: {e}"));
                return None;
            }
        },
        // data: URI ou bufferView -> imagem embutida
        _ => TextureRef::from_gltf_image(role, src as u32),
    };
    t.tex_coord = info.get("texCoord").and_then(Value::as_u64).unwrap_or(0).min(8) as u32;
    t.scale = num(info.get("scale").or_else(|| info.get("strength")), 1.0);
    if info.get("extensions").and_then(|e| e.get("KHR_texture_transform")).is_some() {
        warnings.push(format!("KHR_texture_transform na textura {idx} ignorada (offset/scale de UV nao suportados)"));
    }
    Some(t)
}

// ---------------------------------------------------------------------------------------------
// glTF PBR
// ---------------------------------------------------------------------------------------------

impl PbrParams {
    /// Le `materials[index]` (metal-rough + emissive/normal/occlusion + `KHR_materials_emissive_strength`,
    /// `KHR_materials_unlit` e o legado `KHR_materials_pbrSpecularGlossiness` apenas para diffuse).
    pub fn from_gltf(json: &Value, index: usize, warnings: &mut Vec<String>) -> Option<PbrParams> {
        let m = json.get("materials")?.as_array()?.get(index)?;
        let mut p = PbrParams { name: m.get("name").and_then(Value::as_str).unwrap_or("").to_string(), ..Default::default() };
        if p.name.is_empty() {
            p.name = format!("material_{index}");
        }
        if let Some(pbr) = m.get("pbrMetallicRoughness") {
            p.base_color = vecn(pbr.get("baseColorFactor"), [1.0; 4]);
            p.metallic = num(pbr.get("metallicFactor"), 1.0).clamp(0.0, 1.0);
            p.roughness = num(pbr.get("roughnessFactor"), 1.0).clamp(0.0, 1.0);
            p.base_color_texture = tex_ref(json, pbr.get("baseColorTexture"), TextureRole::BaseColor, warnings);
            p.metal_rough_texture = tex_ref(json, pbr.get("metallicRoughnessTexture"), TextureRole::MetalRough, warnings);
        } else {
            // spec: sem pbrMetallicRoughness => padroes da spec (metallic=1, roughness=1)
            p.metallic = 1.0;
        }
        if let Some(sg) = m.pointer("/extensions/KHR_materials_pbrSpecularGlossiness") {
            warnings.push("KHR_materials_pbrSpecularGlossiness (arquivada pela Khronos): so diffuse foi aproveitado".into());
            p.base_color = vecn(sg.get("diffuseFactor"), [1.0; 4]);
            p.base_color_texture = tex_ref(json, sg.get("diffuseTexture"), TextureRole::BaseColor, warnings);
            p.metallic = 0.0;
            p.roughness = (1.0 - num(sg.get("glossinessFactor"), 1.0)).clamp(0.0, 1.0);
        }
        p.normal_texture = tex_ref(json, m.get("normalTexture"), TextureRole::Normal, warnings);
        p.normal_scale = num(m.pointer("/normalTexture/scale"), 1.0);
        p.occlusion_texture = tex_ref(json, m.get("occlusionTexture"), TextureRole::Occlusion, warnings);
        p.emissive = vecn(m.get("emissiveFactor"), [0.0; 3]);
        p.emissive_texture = tex_ref(json, m.get("emissiveTexture"), TextureRole::Emission, warnings);
        p.emissive_strength = num(m.pointer("/extensions/KHR_materials_emissive_strength/emissiveStrength"), 1.0).max(0.0);
        p.alpha_mode = match m.get("alphaMode").and_then(Value::as_str) {
            Some("MASK") => AlphaMode::Mask,
            Some("BLEND") => AlphaMode::Blend,
            _ => AlphaMode::Opaque,
        };
        p.alpha_cutoff = num(m.get("alphaCutoff"), 0.5);
        p.double_sided = boolean(m.get("doubleSided"), false);
        p.unlit = m.pointer("/extensions/KHR_materials_unlit").is_some();
        for ext in m.get("extensions").and_then(Value::as_object).into_iter().flat_map(|o| o.keys()) {
            if ext.starts_with("KHR_materials_") && !matches!(ext.as_str(), "KHR_materials_emissive_strength" | "KHR_materials_unlit" | "KHR_materials_pbrSpecularGlossiness") {
                warnings.push(format!("{ext} ignorada na conversao para toon"));
            }
        }
        Some(p)
    }
}

fn srgb_to_linear_f(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Escurece uma cor no espaco HSV (V * `factor`), mantendo matiz e saturacao.
pub(crate) fn darken(rgb: [f32; 3], factor: f32) -> [f32; 3] {
    [rgb[0] * factor, rgb[1] * factor, rgb[2] * factor]
}

/// Luminancia relativa (Rec.709) de uma cor linear.
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Fator padrao de escurecimento da sombra derivada de PBR.
pub const DERIVED_SHADE_FACTOR: f32 = 0.72;

/// PBR -> toon. A sombra e DERIVADA (heuristica): `shade = base * 0.72` (mais a rotacao de matiz do
/// renderer, `hue_shift = -15`). Com textura de cor base, a mesma imagem vira `shade.texture`
/// (papel `shade_color`) para a sombra manter o detalhe. Contorno segue o "estilo da casa" do
/// `StylizedMaterial` padrao; rim desligado; especular vem de roughness/metallic.
pub fn toon_from_pbr(p: &PbrParams, converted_from: &str) -> ToonMaterial {
    let mut m = ToonMaterial::named(if p.name.is_empty() { "material" } else { &p.name });
    m.converted_from = Some(converted_from.to_string());
    m.base.color = p.base_color;
    m.base.texture = p.base_color_texture.clone();
    m.base.alpha_mode = p.alpha_mode;
    m.base.alpha_cutoff = p.alpha_cutoff;
    m.base.double_sided = p.double_sided;
    let base_rgb = [p.base_color[0], p.base_color[1], p.base_color[2]];
    if p.unlit {
        // sem sombreamento: sombra == base
        m.shade.color = base_rgb;
        m.shade.texture = p.base_color_texture.as_ref().map(|t| TextureRef { role: TextureRole::ShadeColor, ..t.clone() });
        m.shade.hue_shift = 0.0;
        m.rim.intensity = 0.0;
        m.outline.mode = OutlineMode::None;
        m.spec.intensity = 0.0;
    } else {
        m.shade.color = darken(base_rgb, DERIVED_SHADE_FACTOR);
        m.shade.texture = p.base_color_texture.as_ref().map(|t| TextureRef { role: TextureRole::ShadeColor, ..t.clone() });
        m.rim.intensity = 0.0;
        let gloss = 1.0 - p.roughness;
        m.spec.intensity = (gloss * 0.6).clamp(0.0, 2.0);
        m.spec.power = 8.0 + gloss * gloss * 120.0;
        if p.metallic > 0.5 {
            m.spec.color = [p.base_color[0].max(0.2), p.base_color[1].max(0.2), p.base_color[2].max(0.2)];
        }
    }
    m.emission.color = p.emissive;
    m.emission.intensity = if p.emissive == [0.0; 3] && p.emissive_texture.is_none() { 0.0 } else { p.emissive_strength };
    m.emission.texture = p.emissive_texture.clone();
    m.normal.texture = p.normal_texture.clone();
    m.normal.scale = p.normal_scale;
    m.pbr_fallback = PbrFallback {
        base_color: p.base_color,
        metallic: p.metallic,
        roughness: p.roughness,
        metal_rough_texture: p.metal_rough_texture.clone(),
        occlusion_texture: p.occlusion_texture.clone(),
    };
    m.clamp_in_place();
    m
}

// ---------------------------------------------------------------------------------------------
// VRMC_materials_mtoon
// ---------------------------------------------------------------------------------------------

/// Converte `materials[index]` com `VRMC_materials_mtoon` (mapeamento 1:1 dos parametros da spec 1.0).
/// Devolve `None` se o material nao existe ou nao tem a extensao.
pub fn toon_from_mtoon(json: &Value, index: usize, warnings: &mut Vec<String>) -> Option<ToonMaterial> {
    let mat = json.get("materials")?.as_array()?.get(index)?;
    let mt = mat.pointer("/extensions/VRMC_materials_mtoon")?;
    if !mt.is_object() {
        return None;
    }
    // base: reaproveita o caminho PBR (cor, textura, alpha, emissive, normal)
    let pbr = PbrParams::from_gltf(json, index, warnings)?;
    let mut m = ToonMaterial::named(pbr.name.clone());
    m.converted_from = Some("vrm-mtoon".into());
    m.base.color = pbr.base_color;
    m.base.texture = pbr.base_color_texture.clone();
    m.base.alpha_mode = pbr.alpha_mode;
    m.base.alpha_cutoff = pbr.alpha_cutoff;
    m.base.double_sided = pbr.double_sided;
    m.emission.color = pbr.emissive;
    m.emission.intensity = if pbr.emissive == [0.0; 3] && pbr.emissive_texture.is_none() { 0.0 } else { pbr.emissive_strength };
    m.emission.texture = pbr.emissive_texture.clone();
    m.normal.texture = pbr.normal_texture.clone();
    m.normal.scale = pbr.normal_scale;
    m.pbr_fallback = PbrFallback {
        base_color: pbr.base_color,
        metallic: pbr.metallic,
        roughness: pbr.roughness,
        metal_rough_texture: pbr.metal_rough_texture.clone(),
        occlusion_texture: pbr.occlusion_texture.clone(),
    };

    // sombra
    let shift = num(mt.get("shadingShiftFactor"), 0.0).clamp(-1.0, 1.0);
    let toony = num(mt.get("shadingToonyFactor"), 0.9).clamp(0.0, 1.0);
    let gi = num(mt.get("giEqualizationFactor"), 0.9).clamp(0.0, 1.0);
    m.shade.color = vecn(mt.get("shadeColorFactor"), [0.0; 3]);
    m.shade.texture = tex_ref(json, mt.get("shadeMultiplyTexture"), TextureRole::ShadeColor, warnings);
    m.shade.shift_texture = tex_ref(json, mt.get("shadingShiftTexture"), TextureRole::ShadingShift, warnings);
    m.shade.mtoon = Some(MToonShade { shading_shift: shift, shading_toony: toony, gi_equalization: gi });
    // O renderer cel acende quando half_lambert > threshold; em MToon, shift positivo ACENDE mais.
    m.shade.threshold = (0.5 - 0.5 * shift).clamp(0.0, 1.0);
    // MToon: linearstep(-1+toony, 1-toony) em N.L (largura 2*(1-toony)); smoothstep do shader: largura 2*s em half-lambert.
    m.shade.smoothness = ((1.0 - toony) * 0.5).max(0.005);
    m.shade.steps = 1.0;
    m.shade.hue_shift = 0.0; // a cor de sombra e explicita no MToon
    // rim
    m.rim.color = vecn(mt.get("parametricRimColorFactor"), [0.0; 3]);
    m.rim.fresnel_power = num(mt.get("parametricRimFresnelPowerFactor"), 5.0).max(0.0);
    m.rim.lift = num(mt.get("parametricRimLiftFactor"), 0.0);
    m.rim.lighting_mix = num(mt.get("rimLightingMixFactor"), 1.0).clamp(0.0, 1.0);
    m.rim.multiply_texture = tex_ref(json, mt.get("rimMultiplyTexture"), TextureRole::RimMultiply, warnings);
    m.rim.intensity = if m.rim.color == [0.0; 3] { 0.0 } else { 1.0 };
    m.rim.spread = (1.0 / m.rim.fresnel_power.max(0.1)).clamp(0.05, 1.0);
    // matcap (aditivo na spec)
    m.matcap.factor = vecn(mt.get("matcapFactor"), [1.0; 3]);
    m.matcap.texture = tex_ref(json, mt.get("matcapTexture"), TextureRole::MatCap, warnings);
    m.matcap.mode = MatcapMode::Additive;
    m.matcap.intensity = if m.matcap.texture.is_some() { 1.0 } else { 0.0 };
    // contorno
    m.outline.mode = match mt.get("outlineWidthMode").and_then(Value::as_str) {
        Some("worldCoordinates") => OutlineMode::WorldCoordinates,
        Some("screenCoordinates") => OutlineMode::ScreenCoordinates,
        Some("none") | None => OutlineMode::None,
        Some(other) => {
            warnings.push(format!("outlineWidthMode {other:?} desconhecido; contorno desligado"));
            OutlineMode::None
        }
    };
    m.outline.width = num(mt.get("outlineWidthFactor"), 0.0).max(0.0);
    m.outline.width_texture = tex_ref(json, mt.get("outlineWidthMultiplyTexture"), TextureRole::OutlineWidth, warnings);
    m.outline.color = vecn(mt.get("outlineColorFactor"), [0.0; 3]);
    m.outline.lighting_mix = num(mt.get("outlineLightingMixFactor"), 1.0).clamp(0.0, 1.0);
    // uv anim
    m.uv_anim.scroll_x = num(mt.get("uvAnimationScrollXSpeedFactor"), 0.0);
    m.uv_anim.scroll_y = num(mt.get("uvAnimationScrollYSpeedFactor"), 0.0);
    m.uv_anim.rotation = num(mt.get("uvAnimationRotationSpeedFactor"), 0.0);
    m.uv_anim.mask = tex_ref(json, mt.get("uvAnimationMaskTexture"), TextureRole::UvAnimMask, warnings);
    // render
    m.transparent_with_zwrite = boolean(mt.get("transparentWithZWrite"), false);
    m.render_queue_offset = mt.get("renderQueueOffsetNumber").and_then(Value::as_i64).unwrap_or(0).clamp(-9, 9) as i32;
    // sem especular em MToon
    m.spec.intensity = 0.0;
    m.clamp_in_place();
    Some(m)
}

/// Converte `materials[index]`: MToon se houver a extensao, senao PBR.
pub fn convert_gltf_material(json: &Value, index: usize) -> Option<Converted> {
    let mut warnings = Vec::new();
    let has_mtoon = json.pointer(&format!("/materials/{index}/extensions/VRMC_materials_mtoon")).map(Value::is_object).unwrap_or(false);
    let material = if has_mtoon {
        toon_from_mtoon(json, index, &mut warnings)?
    } else {
        let p = PbrParams::from_gltf(json, index, &mut warnings)?;
        toon_from_pbr(&p, "gltf-pbr")
    };
    Some(Converted { material, warnings })
}

/// Converte todos os materiais (ate `max`).
pub fn convert_gltf_materials(json: &Value, max: usize) -> Vec<Converted> {
    let n = json.get("materials").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0).min(max);
    (0..n).filter_map(|i| convert_gltf_material(json, i)).collect()
}

// ---------------------------------------------------------------------------------------------
// MTL (OBJ)
// ---------------------------------------------------------------------------------------------

fn mtl_path(rest: &str, warnings: &mut Vec<String>, role: TextureRole) -> Option<TextureRef> {
    // pula opcoes "-x a b c" (numeros apos a opcao) e junta o resto como nome (pode ter espacos)
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() && toks[i].starts_with('-') && toks[i].len() > 1 && !toks[i].chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false) {
        i += 1;
        while i < toks.len() && toks[i].parse::<f32>().is_ok() {
            i += 1;
        }
    }
    let name = toks.get(i..)?.join(" ");
    if name.is_empty() {
        return None;
    }
    match crate::paths::normalize_relative(&name) {
        Ok(rel) => Some(TextureRef::from_uri(role, rel)),
        Err(e) => {
            warnings.push(format!("textura MTL ignorada: {e}"));
            None
        }
    }
}

fn parse_f3(s: &str) -> Option<[f32; 3]> {
    let v: Vec<f32> = s.split_whitespace().take(3).map(|t| t.parse::<f32>().ok()).collect::<Option<Vec<_>>>()?;
    match v.len() {
        1 => Some([v[0]; 3]),
        3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// Le um `.mtl`. Cores `Kd`/`Ks`/`Ke` do MTL sao tratadas como sRGB e convertidas para linear.
/// `Ns` (brilho) vira `roughness = sqrt(2 / (Ns + 2))`. Limites: `max_lines`, `max_materials`.
pub fn parse_mtl(text: &str, max_lines: usize, max_materials: usize, warnings: &mut Vec<String>) -> Vec<PbrParams> {
    let mut out: Vec<PbrParams> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if n >= max_lines {
            warnings.push("MTL truncado pelo limite de linhas".into());
            break;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (line, ""),
        };
        if key.eq_ignore_ascii_case("newmtl") {
            if out.len() >= max_materials {
                warnings.push("MTL com materiais demais; o restante foi ignorado".into());
                break;
            }
            out.push(PbrParams { name: rest.chars().take(256).collect(), ..Default::default() });
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        match key.to_ascii_lowercase().as_str() {
            "kd" => {
                if let Some(c) = parse_f3(rest) {
                    cur.base_color[0] = srgb_to_linear_f(c[0].clamp(0.0, 1.0));
                    cur.base_color[1] = srgb_to_linear_f(c[1].clamp(0.0, 1.0));
                    cur.base_color[2] = srgb_to_linear_f(c[2].clamp(0.0, 1.0));
                }
            }
            "ke" => {
                if let Some(c) = parse_f3(rest) {
                    cur.emissive = [srgb_to_linear_f(c[0].clamp(0.0, 1.0)), srgb_to_linear_f(c[1].clamp(0.0, 1.0)), srgb_to_linear_f(c[2].clamp(0.0, 1.0))];
                }
            }
            "ns" => {
                if let Ok(ns) = rest.split_whitespace().next().unwrap_or("").parse::<f32>() {
                    if ns.is_finite() {
                        cur.roughness = (2.0 / (ns.clamp(0.0, 1000.0) + 2.0)).sqrt().clamp(0.0, 1.0);
                    }
                }
            }
            "d" => {
                if let Ok(d) = rest.split_whitespace().next().unwrap_or("").parse::<f32>() {
                    if d.is_finite() {
                        cur.base_color[3] = d.clamp(0.0, 1.0);
                        if d < 1.0 {
                            cur.alpha_mode = AlphaMode::Blend;
                        }
                    }
                }
            }
            "tr" => {
                if let Ok(t) = rest.split_whitespace().next().unwrap_or("").parse::<f32>() {
                    if t.is_finite() {
                        cur.base_color[3] = (1.0 - t).clamp(0.0, 1.0);
                        if t > 0.0 {
                            cur.alpha_mode = AlphaMode::Blend;
                        }
                    }
                }
            }
            "map_kd" => cur.base_color_texture = mtl_path(rest, warnings, TextureRole::BaseColor),
            "map_ke" => cur.emissive_texture = mtl_path(rest, warnings, TextureRole::Emission),
            "map_bump" | "bump" | "norm" | "map_kn" => cur.normal_texture = mtl_path(rest, warnings, TextureRole::Normal),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// FBX
// ---------------------------------------------------------------------------------------------

/// Material FBX (cor base + textura) -> PBR. O caminho da textura do FBX e reduzido ao nome do arquivo
/// (FBX costuma guardar caminhos absolutos da maquina do autor).
pub fn pbr_from_fbx_parts(name: &str, base_color: [f32; 4], texture_path: Option<&str>, warnings: &mut Vec<String>) -> PbrParams {
    let mut p = PbrParams { name: name.to_string(), base_color, ..Default::default() };
    p.roughness = 0.8;
    p.metallic = 0.0;
    if let Some(tp) = texture_path {
        let file = tp.rsplit(['/', '\\']).next().unwrap_or("");
        if file.is_empty() {
            return p;
        }
        match crate::paths::normalize_relative(file) {
            Ok(rel) => p.base_color_texture = Some(TextureRef::from_uri(TextureRole::BaseColor, rel)),
            Err(e) => warnings.push(format!("textura FBX ignorada: {e}")),
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(material: Value) -> Value {
        json!({
            "asset": {"version": "2.0"},
            "images": [{"uri": "tex/base.png"}, {"uri": "../../evil.png"}, {"bufferView": 0, "mimeType": "image/png"}, {"uri": "data:image/png;base64,AAAA"}],
            "textures": [{"source": 0}, {"source": 1}, {"source": 2}, {"source": 3}],
            "materials": [material]
        })
    }

    #[test]
    fn pbr_basic_conversion() {
        let j = doc(json!({
            "name": "Pele",
            "pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.6, 0.5, 1.0], "baseColorTexture": {"index": 0}, "metallicFactor": 0.0, "roughnessFactor": 0.5},
            "normalTexture": {"index": 2, "scale": 0.7},
            "emissiveFactor": [0.1, 0.0, 0.0],
            "alphaMode": "MASK", "alphaCutoff": 0.3, "doubleSided": true,
            "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": 4.0}, "KHR_materials_clearcoat": {}}
        }));
        let c = convert_gltf_material(&j, 0).unwrap();
        let m = &c.material;
        assert_eq!(m.name, "Pele");
        assert_eq!(m.converted_from.as_deref(), Some("gltf-pbr"));
        assert_eq!(m.base.color, [0.8, 0.6, 0.5, 1.0]);
        assert_eq!(m.base.texture.as_ref().unwrap().uri.as_deref(), Some("tex/base.png"));
        assert_eq!(m.base.texture.as_ref().unwrap().color_space, ColorSpace::Srgb);
        assert_eq!(m.shade.texture.as_ref().unwrap().role, TextureRole::ShadeColor);
        assert_eq!(m.normal.texture.as_ref().unwrap().gltf_image, Some(2));
        assert_eq!(m.normal.texture.as_ref().unwrap().color_space, ColorSpace::Linear);
        assert_eq!(m.normal.scale, 0.7);
        assert_eq!(m.base.alpha_mode, AlphaMode::Mask);
        assert_eq!(m.base.alpha_cutoff, 0.3);
        assert!(m.base.double_sided);
        assert_eq!(m.emission.intensity, 4.0);
        assert!(c.warnings.iter().any(|w| w.contains("KHR_materials_clearcoat")));
        // sombra derivada mais escura que a base, com o mesmo matiz
        assert!(luminance(m.shade.color) < luminance([0.8, 0.6, 0.5]));
        assert!((m.shade.color[0] / m.shade.color[1] - 0.8 / 0.6).abs() < 1e-4);
        assert_eq!(m.shade.hue_shift, -15.0);
        // pbr_fallback preserva os fatores
        assert_eq!(m.pbr_fallback.roughness, 0.5);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn pbr_unsafe_and_embedded_textures() {
        let j = doc(json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": 1}}, "emissiveTexture": {"index": 3}}));
        let c = convert_gltf_material(&j, 0).unwrap();
        assert!(c.material.base.texture.is_none(), "uri com .. deve ser descartada");
        assert!(c.warnings.iter().any(|w| w.contains("ignorada")));
        // data: URI vira imagem embutida (indice), nao uri
        assert_eq!(c.material.emission.texture.as_ref().unwrap().gltf_image, Some(3));
    }

    #[test]
    fn pbr_defaults_follow_the_gltf_spec() {
        // glTF: sem pbrMetallicRoughness => metallic 1 / roughness 1 (padroes da spec)
        let j = doc(json!({}));
        let mut w = vec![];
        let p = PbrParams::from_gltf(&j, 0, &mut w).unwrap();
        assert_eq!(p.metallic, 1.0);
        assert_eq!(p.roughness, 1.0);
        assert_eq!(p.base_color, [1.0; 4]);
        assert!(PbrParams::from_gltf(&j, 5, &mut w).is_none());
        // tipos errados nao derrubam
        let j = doc(json!({"pbrMetallicRoughness": {"baseColorFactor": "x", "metallicFactor": [1], "roughnessFactor": null}, "emissiveFactor": [1, "a", 2], "alphaMode": 7}));
        let p = PbrParams::from_gltf(&j, 0, &mut w).unwrap();
        assert_eq!(p.base_color, [1.0; 4]);
        assert_eq!(p.emissive, [0.0; 3]);
        assert_eq!(p.alpha_mode, AlphaMode::Opaque);
    }

    #[test]
    fn unlit_has_no_shade_difference() {
        let j = doc(json!({"pbrMetallicRoughness": {"baseColorFactor": [0.5, 0.5, 0.5, 1.0]}, "extensions": {"KHR_materials_unlit": {}}}));
        let m = convert_gltf_material(&j, 0).unwrap().material;
        assert_eq!(m.shade.color, [0.5, 0.5, 0.5]);
        assert_eq!(m.outline.mode, OutlineMode::None);
    }

    #[test]
    fn spec_gloss_uses_diffuse_only() {
        let j = doc(json!({"extensions": {"KHR_materials_pbrSpecularGlossiness": {"diffuseFactor": [0.2, 0.3, 0.4, 1.0], "glossinessFactor": 0.75}}}));
        let c = convert_gltf_material(&j, 0).unwrap();
        assert_eq!(c.material.base.color, [0.2, 0.3, 0.4, 1.0]);
        assert!((c.material.pbr_fallback.roughness - 0.25).abs() < 1e-6);
        assert!(c.warnings.iter().any(|w| w.contains("arquivada")));
    }

    fn mtoon_doc() -> Value {
        json!({
            "asset": {"version": "2.0"},
            "images": [{"uri": "tex/body.png"}, {"uri": "tex/shade.png"}, {"uri": "tex/matcap.png"}, {"uri": "tex/rim.png"}, {"uri": "tex/ow.png"}],
            "textures": [{"source": 0}, {"source": 1}, {"source": 2}, {"source": 3}, {"source": 4}],
            "materials": [{
                "name": "Roupa",
                "pbrMetallicRoughness": {"baseColorFactor": [1,1,1,1], "baseColorTexture": {"index": 0}},
                "emissiveFactor": [0.0, 0.0, 0.0],
                "extensions": {"VRMC_materials_mtoon": {
                    "specVersion": "1.0",
                    "transparentWithZWrite": true,
                    "renderQueueOffsetNumber": 3,
                    "shadeColorFactor": [0.5, 0.4, 0.45],
                    "shadeMultiplyTexture": {"index": 1},
                    "shadingShiftFactor": 0.2,
                    "shadingToonyFactor": 0.8,
                    "giEqualizationFactor": 0.5,
                    "matcapFactor": [0.9, 0.9, 0.9],
                    "matcapTexture": {"index": 2},
                    "parametricRimColorFactor": [1.0, 0.8, 0.8],
                    "parametricRimFresnelPowerFactor": 2.0,
                    "parametricRimLiftFactor": 0.1,
                    "rimMultiplyTexture": {"index": 3},
                    "rimLightingMixFactor": 0.5,
                    "outlineWidthMode": "screenCoordinates",
                    "outlineWidthFactor": 0.004,
                    "outlineWidthMultiplyTexture": {"index": 4},
                    "outlineColorFactor": [0.1, 0.0, 0.2],
                    "outlineLightingMixFactor": 0.7,
                    "uvAnimationScrollXSpeedFactor": 0.5,
                    "uvAnimationScrollYSpeedFactor": -0.25,
                    "uvAnimationRotationSpeedFactor": 1.5
                }}
            }]
        })
    }

    #[test]
    fn mtoon_maps_one_to_one() {
        let c = convert_gltf_material(&mtoon_doc(), 0).unwrap();
        let m = &c.material;
        assert_eq!(m.converted_from.as_deref(), Some("vrm-mtoon"));
        assert_eq!(m.name, "Roupa");
        assert_eq!(m.shade.color, [0.5, 0.4, 0.45]);
        assert_eq!(m.shade.texture.as_ref().unwrap().uri.as_deref(), Some("tex/shade.png"));
        let mt = m.shade.mtoon.as_ref().unwrap();
        assert_eq!((mt.shading_shift, mt.shading_toony, mt.gi_equalization), (0.2, 0.8, 0.5));
        // shift positivo acende mais => threshold cai abaixo de 0.5
        assert!((m.shade.threshold - 0.4).abs() < 1e-6);
        assert!((m.shade.smoothness - 0.1).abs() < 1e-6);
        assert_eq!(m.shade.hue_shift, 0.0);
        assert_eq!(m.rim.color, [1.0, 0.8, 0.8]);
        assert_eq!(m.rim.fresnel_power, 2.0);
        assert_eq!(m.rim.lift, 0.1);
        assert_eq!(m.rim.lighting_mix, 0.5);
        assert_eq!(m.rim.multiply_texture.as_ref().unwrap().uri.as_deref(), Some("tex/rim.png"));
        assert_eq!(m.matcap.mode, MatcapMode::Additive);
        assert_eq!(m.matcap.factor, [0.9; 3]);
        assert_eq!(m.matcap.texture.as_ref().unwrap().role, TextureRole::MatCap);
        assert_eq!(m.outline.mode, OutlineMode::ScreenCoordinates);
        assert_eq!(m.outline.width, 0.004);
        assert_eq!(m.outline.color, [0.1, 0.0, 0.2]);
        assert_eq!(m.outline.lighting_mix, 0.7);
        assert_eq!(m.outline.width_texture.as_ref().unwrap().uri.as_deref(), Some("tex/ow.png"));
        assert_eq!((m.uv_anim.scroll_x, m.uv_anim.scroll_y, m.uv_anim.rotation), (0.5, -0.25, 1.5));
        assert!(m.transparent_with_zwrite);
        assert_eq!(m.render_queue_offset, 3);
        assert_eq!(m.spec.intensity, 0.0);
        m.validate().unwrap();
        // chega ao renderer
        let s = m.to_stylized();
        assert!(s.mtoon_main_texture_enabled && s.mtoon_shade_texture_enabled && s.mtoon_matcap_enabled);
        assert_eq!(s.outline_width, 0.004);
    }

    #[test]
    fn mtoon_defaults_match_the_spec_when_absent() {
        let j = json!({"asset": {"version": "2.0"}, "materials": [{"extensions": {"VRMC_materials_mtoon": {"specVersion": "1.0"}}}]});
        let m = convert_gltf_material(&j, 0).unwrap().material;
        let mt = m.shade.mtoon.unwrap();
        assert_eq!((mt.shading_shift, mt.shading_toony, mt.gi_equalization), (0.0, 0.9, 0.9));
        assert_eq!(m.shade.color, [0.0; 3]);
        assert_eq!(m.rim.fresnel_power, 5.0);
        assert_eq!(m.rim.lighting_mix, 1.0);
        assert_eq!(m.outline.mode, OutlineMode::None);
        assert_eq!(m.outline.lighting_mix, 1.0);
        assert_eq!(m.matcap.factor, [1.0; 3]);
        assert!(!m.transparent_with_zwrite);
        assert_eq!(m.render_queue_offset, 0);
    }

    #[test]
    fn mtoon_survives_garbage_values() {
        let j = json!({"asset": {"version": "2.0"}, "materials": [{"extensions": {"VRMC_materials_mtoon": {
            "shadingToonyFactor": "x", "shadeColorFactor": [1, 2], "outlineWidthMode": "weird", "renderQueueOffsetNumber": 1e9,
            "rimMultiplyTexture": {"index": 99}, "matcapTexture": 5, "shadingShiftFactor": 1e30
        }}}]});
        let c = convert_gltf_material(&j, 0).unwrap();
        let m = c.material;
        assert_eq!(m.outline.mode, OutlineMode::None);
        assert!(m.render_queue_offset.abs() <= 9);
        assert!(m.rim.multiply_texture.is_none());
        assert!(m.shade.threshold.is_finite());
        assert!(m.validate().is_ok());
        assert!(c.warnings.iter().any(|w| w.contains("weird")));
    }

    #[test]
    fn convert_all_and_extension_dispatch() {
        let mut j = mtoon_doc();
        j["materials"].as_array_mut().unwrap().push(json!({"name": "Plain"}));
        let all = convert_gltf_materials(&j, 100);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].material.converted_from.as_deref(), Some("vrm-mtoon"));
        assert_eq!(all[1].material.converted_from.as_deref(), Some("gltf-pbr"));
        assert_eq!(convert_gltf_materials(&j, 1).len(), 1);
        assert!(convert_gltf_materials(&json!({}), 10).is_empty());
        assert!(convert_gltf_materials(&json!({"materials": "x"}), 10).is_empty());
    }

    #[test]
    fn mtl_parsing() {
        let mtl = "# comentario\nnewmtl Pele\nKd 0.8 0.6 0.5\nKe 0 0 0\nNs 100\nd 0.5\nmap_Kd -s 1 1 1 tex/pele cor.png\nmap_Bump -bm 0.5 normal.png\nnewmtl Evil\nmap_Kd ../../etc/passwd\nTr 0.25\nnewmtl Vazio\n";
        let mut w = vec![];
        let v = parse_mtl(mtl, 1000, 100, &mut w);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].name, "Pele");
        assert!((v[0].base_color[0] - srgb_to_linear_f(0.8)).abs() < 1e-6);
        assert_eq!(v[0].base_color[3], 0.5);
        assert_eq!(v[0].alpha_mode, AlphaMode::Blend);
        assert!((v[0].roughness - (2.0f32 / 102.0).sqrt()).abs() < 1e-6);
        assert_eq!(v[0].base_color_texture.as_ref().unwrap().uri.as_deref(), Some("tex/pele cor.png"));
        assert_eq!(v[0].normal_texture.as_ref().unwrap().uri.as_deref(), Some("normal.png"));
        assert!(v[1].base_color_texture.is_none());
        assert_eq!(v[1].base_color[3], 0.75);
        assert!(w.iter().any(|x| x.contains("ignorada")));
        // limites
        let mut w = vec![];
        assert_eq!(parse_mtl("newmtl a\nnewmtl b\nnewmtl c\n", 1000, 2, &mut w).len(), 2);
        let mut w = vec![];
        assert_eq!(parse_mtl("newmtl a\nKd 1 0 0\nnewmtl b\n", 2, 100, &mut w).len(), 1);
        // lixo
        let mut w = vec![];
        assert!(parse_mtl("\u{0}\u{1} Kd\nKd\nnewmtl\nKd x y z\nNs nan\n", 100, 100, &mut w).len() <= 1);
        let mut w = vec![];
        let v = parse_mtl("newmtl a\nKd 1e999 0 0\nNs inf\nd -4\n", 100, 100, &mut w);
        assert!(v[0].base_color.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn pbr_to_toon_is_always_valid() {
        let p = PbrParams { base_color: [2.0, -1.0, 0.5, 3.0], roughness: 7.0, metallic: -3.0, ..Default::default() };
        let m = toon_from_pbr(&p, "test");
        assert!(m.validate().is_ok());
        assert!(m.base.color.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn fbx_texture_paths_are_reduced_to_file_name() {
        let mut w = vec![];
        let p = pbr_from_fbx_parts("m", [1.0; 4], Some("C:\\Users\\x\\tex\\pele.png"), &mut w);
        assert_eq!(p.base_color_texture.unwrap().uri.as_deref(), Some("pele.png"));
        let p = pbr_from_fbx_parts("m", [1.0; 4], Some("/home/a/b/"), &mut w);
        assert!(p.base_color_texture.is_none());
    }
}
