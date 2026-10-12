//! **ANIGO Toon Material** (`.anigomat.json`): material canonico, toon-first.
//!
//! * Superconjunto estruturado de `anigo_core::StylizedMaterial` (blocos `base`, `shade`, `ilm`, `spec`,
//!   `rim`, `matcap`, `outline`, `face_sdf`, `emission`, `normal`, `uv_anim`, `pbr_fallback`).
//! * Cada textura declara `role` + espaco de cor + (opcional) swizzle; nada e inferido do nome do arquivo.
//! * Conversores (ver [`convert`]): glTF PBR -> toon (sombra derivada), `VRMC_materials_mtoon` -> toon (1:1 nos
//!   parametros que a spec define), MTL (OBJ) e FBX -> PBR -> toon.
//! * Cores `[r,g,b]` sao **lineares** (como os fatores do glTF), a menos que o campo diga o contrario.
//!
//! O formato e versionado (`schema_ver`). Leitores devem recusar versoes maiores que a suportada.

pub mod convert;

use serde::{Deserialize, Serialize};

use anigo_core::StylizedMaterial;

use crate::error::{AssetError, Result};
use crate::paths::normalize_relative;

/// Valor do campo `schema`.
pub const MATERIAL_SCHEMA: &str = "anigo.toon-material";
/// Versao atual do formato.
pub const MATERIAL_SCHEMA_VER: u32 = 1;
/// Sufixo de arquivo.
pub const MATERIAL_SUFFIX: &str = ".anigomat.json";
/// Tamanho maximo aceito ao ler um `.anigomat.json`.
pub const MATERIAL_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    Srgb,
    Linear,
}

/// Papel da textura no material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureRole {
    BaseColor,
    ShadeColor,
    ShadeColor2,
    ShadingShift,
    Ramp,
    /// Mapa por canal (ver [`IlmBlock`]).
    Ilm,
    FaceSdf,
    MatCap,
    RimMultiply,
    OutlineWidth,
    Emission,
    Normal,
    Occlusion,
    /// Metal(B)/Rough(G) do glTF, usado so no `pbr_fallback`.
    MetalRough,
    UvAnimMask,
}

impl TextureRole {
    /// Espaco de cor padrao do papel (cor = sRGB; dados = linear).
    pub fn default_color_space(self) -> ColorSpace {
        match self {
            TextureRole::BaseColor | TextureRole::ShadeColor | TextureRole::ShadeColor2 | TextureRole::MatCap | TextureRole::Emission => ColorSpace::Srgb,
            _ => ColorSpace::Linear,
        }
    }
}

/// Referencia a textura: `uri` relativa (confinada a pasta do ativo) OU indice de imagem do glTF/GLB de origem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// `images[i]` do glTF/GLB de origem (imagens embutidas em bufferView).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gltf_image: Option<u32>,
    pub role: TextureRole,
    pub color_space: ColorSpace,
    #[serde(default)]
    pub tex_coord: u32,
    #[serde(default = "one")]
    pub scale: f32,
    /// Ex.: `"rgba"`, `"bgra"`, `"rrr1"`. `None` = identidade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swizzle: Option<String>,
}

fn one() -> f32 {
    1.0
}

impl TextureRef {
    pub fn from_uri(role: TextureRole, uri: impl Into<String>) -> Self {
        Self { uri: Some(uri.into()), gltf_image: None, role, color_space: role.default_color_space(), tex_coord: 0, scale: 1.0, swizzle: None }
    }
    pub fn from_gltf_image(role: TextureRole, image: u32) -> Self {
        Self { uri: None, gltf_image: Some(image), role, color_space: role.default_color_space(), tex_coord: 0, scale: 1.0, swizzle: None }
    }
    fn validate(&self) -> Result<()> {
        match (&self.uri, &self.gltf_image) {
            (Some(u), None) => {
                normalize_relative(u)?;
            }
            (None, Some(_)) => {}
            _ => return Err(AssetError::InvalidInput("textura precisa de exatamente um entre 'uri' e 'gltf_image'".into())),
        }
        if !self.scale.is_finite() {
            return Err(AssetError::InvalidInput("scale de textura nao finito".into()));
        }
        if let Some(s) = &self.swizzle {
            if s.len() != 4 || !s.chars().all(|c| matches!(c, 'r' | 'g' | 'b' | 'a' | '0' | '1')) {
                return Err(AssetError::InvalidInput(format!("swizzle invalido: {s:?}")));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    Opaque,
    /// Preferido para cabelo/cilios (cutoff).
    Mask,
    Blend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BaseBlock {
    /// RGBA linear.
    pub color: [f32; 4],
    pub texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for BaseBlock {
    fn default() -> Self {
        Self { color: [0.98, 0.92, 0.85, 1.0], texture: None, alpha_mode: AlphaMode::Opaque, alpha_cutoff: 0.5, double_sided: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadeBlock {
    pub color: [f32; 3],
    pub texture: Option<TextureRef>,
    pub color2: Option<[f32; 3]>,
    pub texture2: Option<TextureRef>,
    /// Limiar do half-lambert abaixo do qual entra sombra (renderer cel: `shadow_threshold`).
    pub threshold: f32,
    /// Largura da transicao (renderer: `shadow_smoothness`).
    pub smoothness: f32,
    /// 0 = continuo, 1 = cel duro, 2/3 = bandas (renderer: `toon_steps`).
    pub steps: f32,
    /// Rotacao de matiz da sombra, em graus.
    pub hue_shift: f32,
    pub second_shift: f32,
    pub second_softness: f32,
    pub ramp: Option<TextureRef>,
    pub shift_texture: Option<TextureRef>,
    /// Valores originais do MToon preservados para exportar de volta sem perda (`None` se nao veio de MToon).
    pub mtoon: Option<MToonShade>,
}

/// Parametros de sombra do MToon, como na spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MToonShade {
    pub shading_shift: f32,
    pub shading_toony: f32,
    pub gi_equalization: f32,
}

impl Default for ShadeBlock {
    fn default() -> Self {
        Self {
            color: [0.82, 0.73, 0.78],
            texture: None,
            color2: None,
            texture2: None,
            threshold: 0.5,
            smoothness: 0.02,
            steps: 1.0,
            hue_shift: -15.0,
            second_shift: 0.0,
            second_softness: 0.05,
            ramp: None,
            shift_texture: None,
            mtoon: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IlmChannel {
    None,
    SpecMask,
    ShadowThreshold,
    SpecSize,
    OutlineMask,
}

/// Mapa ILM (Guilty Gear/Genshin): cada canal da textura carrega um papel configuravel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IlmBlock {
    pub texture: TextureRef,
    pub r: IlmChannel,
    pub g: IlmChannel,
    pub b: IlmChannel,
    pub a: IlmChannel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecBlock {
    pub color: [f32; 3],
    pub intensity: f32,
    pub power: f32,
    pub size: f32,
    pub softness: f32,
    pub offset: f32,
}

impl Default for SpecBlock {
    fn default() -> Self {
        Self { color: [1.0; 3], intensity: 0.40, power: 32.0, size: 0.45, softness: 0.05, offset: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RimBlock {
    pub color: [f32; 3],
    pub intensity: f32,
    pub spread: f32,
    pub fresnel_power: f32,
    pub lift: f32,
    pub lighting_mix: f32,
    pub multiply_texture: Option<TextureRef>,
}

impl Default for RimBlock {
    fn default() -> Self {
        Self { color: [0.576, 0.773, 0.992], intensity: 0.80, spread: 0.40, fresnel_power: 5.0, lift: 0.0, lighting_mix: 1.0, multiply_texture: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatcapMode {
    Multiply,
    Additive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatcapBlock {
    pub texture: Option<TextureRef>,
    pub factor: [f32; 3],
    pub intensity: f32,
    pub mode: MatcapMode,
}

impl Default for MatcapBlock {
    fn default() -> Self {
        Self { texture: None, factor: [1.0; 3], intensity: 0.0, mode: MatcapMode::Multiply }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlineMode {
    None,
    /// Largura em unidades de mundo (metros).
    WorldCoordinates,
    /// Largura relativa a altura da tela (fracao).
    ScreenCoordinates,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutlineBlock {
    pub mode: OutlineMode,
    pub width: f32,
    pub width_texture: Option<TextureRef>,
    pub color: [f32; 3],
    pub lighting_mix: f32,
    pub opacity: f32,
    pub smoothness: f32,
    pub depth_bias: f32,
}

impl Default for OutlineBlock {
    fn default() -> Self {
        Self {
            mode: OutlineMode::ScreenCoordinates,
            width: 0.0035,
            width_texture: None,
            color: [0.25, 0.15, 0.20],
            lighting_mix: 1.0,
            opacity: 1.0,
            smoothness: 0.0,
            depth_bias: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceSdfBlock {
    pub texture: TextureRef,
    /// Vetores frente/direita da cabeca no espaco do objeto.
    pub front: [f32; 3],
    pub right: [f32; 3],
    pub offset: f32,
    pub smoothness: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmissionBlock {
    pub color: [f32; 3],
    pub intensity: f32,
    pub texture: Option<TextureRef>,
}

impl Default for EmissionBlock {
    fn default() -> Self {
        Self { color: [0.0; 3], intensity: 0.0, texture: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalConvention {
    /// +Y para cima (glTF, Blender).
    OpenGl,
    /// Y invertido (Unreal/DirectX): inverter o canal G ao importar.
    DirectX,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalBlock {
    pub texture: Option<TextureRef>,
    pub scale: f32,
    pub convention: NormalConvention,
}

impl Default for NormalBlock {
    fn default() -> Self {
        Self { texture: None, scale: 1.0, convention: NormalConvention::OpenGl }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UvAnimBlock {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub rotation: f32,
    pub mask: Option<TextureRef>,
}

/// Parametros PBR preservados para exportar a mundos PBR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PbrFallback {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub metal_rough_texture: Option<TextureRef>,
    pub occlusion_texture: Option<TextureRef>,
}

impl Default for PbrFallback {
    fn default() -> Self {
        Self { base_color: [1.0; 4], metallic: 0.0, roughness: 1.0, metal_rough_texture: None, occlusion_texture: None }
    }
}

/// Parametros do `StylizedMaterial` do renderer cel que o formato estruturado nao modela (ficam fora dos
/// blocos acima) e o grupo NPR (rampa 2D, contorno de alta qualidade, espelho do SDF facial). Opcional e
/// aditivo: ausente em arquivos antigos; quando presente sobrescreve o que `to_stylized` derivou dos blocos.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StylizedExtras {
    pub ao_intensity: f32,
    pub mtoon_matcap_enabled: bool,
    pub mtoon_shade_toony: bool,
    pub face_sdf_enabled: bool,
    pub face_shadow_offset: f32,
    pub face_shadow_smoothness: f32,
    pub eye_enabled: bool,
    pub eye_depth_scale: f32,
    pub eye_highlight_intensity: f32,
    pub gaze_tracking_enabled: bool,
    pub gaze_saccade_amplitude: f32,
    pub gaze_damping: f32,
    pub npr: anigo_core::NprParams,
}

impl Default for StylizedExtras {
    fn default() -> Self {
        Self::from_stylized(&StylizedMaterial::default())
    }
}

impl StylizedExtras {
    pub fn from_stylized(s: &StylizedMaterial) -> Self {
        Self {
            ao_intensity: s.ao_intensity,
            mtoon_matcap_enabled: s.mtoon_matcap_enabled,
            mtoon_shade_toony: s.mtoon_shade_toony,
            face_sdf_enabled: s.face_sdf_enabled,
            face_shadow_offset: s.face_shadow_offset,
            face_shadow_smoothness: s.face_shadow_smoothness,
            eye_enabled: s.eye_enabled,
            eye_depth_scale: s.eye_depth_scale,
            eye_highlight_intensity: s.eye_highlight_intensity,
            gaze_tracking_enabled: s.gaze_tracking_enabled,
            gaze_saccade_amplitude: s.gaze_saccade_amplitude,
            gaze_damping: s.gaze_damping,
            npr: s.npr,
        }
    }

    fn apply_to(&self, s: &mut StylizedMaterial) {
        s.ao_intensity = self.ao_intensity;
        s.mtoon_matcap_enabled = self.mtoon_matcap_enabled;
        s.mtoon_shade_toony = self.mtoon_shade_toony;
        s.face_sdf_enabled = self.face_sdf_enabled;
        s.face_shadow_offset = self.face_shadow_offset;
        s.face_shadow_smoothness = self.face_shadow_smoothness;
        s.eye_enabled = self.eye_enabled;
        s.eye_depth_scale = self.eye_depth_scale;
        s.eye_highlight_intensity = self.eye_highlight_intensity;
        s.gaze_tracking_enabled = self.gaze_tracking_enabled;
        s.gaze_saccade_amplitude = self.gaze_saccade_amplitude;
        s.gaze_damping = self.gaze_damping;
        s.npr = self.npr.sanitized();
    }

    fn all_finite(&self) -> bool {
        [self.ao_intensity, self.face_shadow_offset, self.face_shadow_smoothness, self.eye_depth_scale, self.eye_highlight_intensity, self.gaze_saccade_amplitude, self.gaze_damping]
            .iter()
            .all(|v| v.is_finite())
    }
}

/// O material canonico.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToonMaterial {
    pub schema: String,
    pub schema_ver: u32,
    pub name: String,
    /// `"gltf-pbr"`, `"vrm-mtoon"`, `"obj-mtl"`, `"fbx"`, `"stylized"` ou `None` se autoral.
    pub converted_from: Option<String>,
    pub base: BaseBlock,
    pub shade: ShadeBlock,
    pub ilm: Option<IlmBlock>,
    pub spec: SpecBlock,
    pub rim: RimBlock,
    pub matcap: MatcapBlock,
    pub outline: OutlineBlock,
    pub face_sdf: Option<FaceSdfBlock>,
    pub emission: EmissionBlock,
    pub normal: NormalBlock,
    pub uv_anim: UvAnimBlock,
    pub pbr_fallback: PbrFallback,
    pub transparent_with_zwrite: bool,
    pub render_queue_offset: i32,
    /// Extras do renderer cel e grupo NPR (aditivo; ver [`StylizedExtras`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stylized: Option<StylizedExtras>,
    /// Dados sem equivalente (extensoes desconhecidas etc.), preservados.
    pub extras: serde_json::Value,
}

impl Default for ToonMaterial {
    fn default() -> Self {
        Self {
            schema: MATERIAL_SCHEMA.into(),
            schema_ver: MATERIAL_SCHEMA_VER,
            name: "ANIGO Toon Material".into(),
            converted_from: None,
            base: BaseBlock::default(),
            shade: ShadeBlock::default(),
            ilm: None,
            spec: SpecBlock::default(),
            rim: RimBlock::default(),
            matcap: MatcapBlock::default(),
            outline: OutlineBlock::default(),
            face_sdf: None,
            emission: EmissionBlock::default(),
            normal: NormalBlock::default(),
            uv_anim: UvAnimBlock::default(),
            pbr_fallback: PbrFallback::default(),
            transparent_with_zwrite: false,
            render_queue_offset: 0,
            stylized: None,
            extras: serde_json::Value::Null,
        }
    }
}

fn finite(name: &str, vals: &[f32]) -> Result<()> {
    if vals.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(AssetError::InvalidInput(format!("valor nao finito em '{name}'")))
    }
}

impl ToonMaterial {
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// Texturas referenciadas, na ordem dos blocos.
    pub fn textures(&self) -> Vec<&TextureRef> {
        fn opt<'a>(v: &mut Vec<&'a TextureRef>, t: &'a Option<TextureRef>) {
            if let Some(t) = t {
                v.push(t)
            }
        }
        let mut v: Vec<&TextureRef> = Vec::new();
        opt(&mut v, &self.base.texture);
        opt(&mut v, &self.shade.texture);
        opt(&mut v, &self.shade.texture2);
        opt(&mut v, &self.shade.ramp);
        opt(&mut v, &self.shade.shift_texture);
        if let Some(i) = &self.ilm {
            v.push(&i.texture);
        }
        opt(&mut v, &self.rim.multiply_texture);
        opt(&mut v, &self.matcap.texture);
        opt(&mut v, &self.outline.width_texture);
        if let Some(f) = &self.face_sdf {
            v.push(&f.texture);
        }
        opt(&mut v, &self.emission.texture);
        opt(&mut v, &self.normal.texture);
        opt(&mut v, &self.uv_anim.mask);
        opt(&mut v, &self.pbr_fallback.metal_rough_texture);
        opt(&mut v, &self.pbr_fallback.occlusion_texture);
        v
    }

    /// Valida schema, numeros finitos e URIs de textura (confinadas). Nao altera valores.
    pub fn validate(&self) -> Result<()> {
        if self.schema != MATERIAL_SCHEMA {
            return Err(AssetError::parse("anigomat", format!("schema {:?} inesperado", self.schema)));
        }
        if self.schema_ver == 0 || self.schema_ver > MATERIAL_SCHEMA_VER {
            return Err(AssetError::Unsupported(format!("anigomat schema_ver {} (suportado ate {MATERIAL_SCHEMA_VER})", self.schema_ver)));
        }
        if self.name.chars().count() > 256 {
            return Err(AssetError::InvalidInput("nome do material longo demais".into()));
        }
        finite("base.color", &self.base.color)?;
        finite("base.alpha_cutoff", &[self.base.alpha_cutoff])?;
        finite("shade", &[self.shade.color[0], self.shade.color[1], self.shade.color[2], self.shade.threshold, self.shade.smoothness, self.shade.steps, self.shade.hue_shift, self.shade.second_shift, self.shade.second_softness])?;
        if let Some(c2) = self.shade.color2 {
            finite("shade.color2", &c2)?;
        }
        finite("spec", &[self.spec.intensity, self.spec.power, self.spec.size, self.spec.softness, self.spec.offset, self.spec.color[0], self.spec.color[1], self.spec.color[2]])?;
        finite("rim", &[self.rim.intensity, self.rim.spread, self.rim.fresnel_power, self.rim.lift, self.rim.lighting_mix, self.rim.color[0], self.rim.color[1], self.rim.color[2]])?;
        finite("matcap", &[self.matcap.intensity, self.matcap.factor[0], self.matcap.factor[1], self.matcap.factor[2]])?;
        finite("outline", &[self.outline.width, self.outline.lighting_mix, self.outline.opacity, self.outline.smoothness, self.outline.depth_bias, self.outline.color[0], self.outline.color[1], self.outline.color[2]])?;
        finite("emission", &[self.emission.intensity, self.emission.color[0], self.emission.color[1], self.emission.color[2]])?;
        finite("normal.scale", &[self.normal.scale])?;
        finite("uv_anim", &[self.uv_anim.scroll_x, self.uv_anim.scroll_y, self.uv_anim.rotation])?;
        finite("pbr_fallback", &[self.pbr_fallback.metallic, self.pbr_fallback.roughness, self.pbr_fallback.base_color[0], self.pbr_fallback.base_color[1], self.pbr_fallback.base_color[2], self.pbr_fallback.base_color[3]])?;
        if let Some(f) = &self.face_sdf {
            finite("face_sdf", &[f.offset, f.smoothness, f.front[0], f.front[1], f.front[2], f.right[0], f.right[1], f.right[2]])?;
        }
        if let Some(m) = &self.shade.mtoon {
            finite("shade.mtoon", &[m.shading_shift, m.shading_toony, m.gi_equalization])?;
        }
        if let Some(x) = &self.stylized {
            if !x.all_finite() {
                return Err(AssetError::InvalidInput("valor nao finito em 'stylized'".into()));
            }
        }
        for t in self.textures() {
            t.validate()?;
        }
        Ok(())
    }

    /// Limita cada parametro a sua faixa util (para entradas de terceiros que passaram em `validate`).
    pub fn clamp_in_place(&mut self) {
        let c01 = |v: &mut f32| *v = v.clamp(0.0, 1.0);
        for v in &mut self.base.color {
            c01(v);
        }
        c01(&mut self.base.alpha_cutoff);
        for v in &mut self.shade.color {
            c01(v);
        }
        c01(&mut self.shade.threshold);
        self.shade.smoothness = self.shade.smoothness.clamp(0.0, 1.0);
        self.shade.steps = self.shade.steps.clamp(0.0, 8.0);
        self.shade.hue_shift = self.shade.hue_shift.clamp(-180.0, 180.0);
        self.spec.intensity = self.spec.intensity.clamp(0.0, 2.0);
        self.spec.power = self.spec.power.clamp(1.0, 512.0);
        self.rim.intensity = self.rim.intensity.clamp(0.0, 3.0);
        self.rim.spread = self.rim.spread.clamp(0.05, 1.0);
        self.rim.fresnel_power = self.rim.fresnel_power.clamp(0.0, 100.0);
        c01(&mut self.outline.opacity);
        self.outline.width = self.outline.width.clamp(0.0, 1.0);
        self.pbr_fallback.metallic = self.pbr_fallback.metallic.clamp(0.0, 1.0);
        self.pbr_fallback.roughness = self.pbr_fallback.roughness.clamp(0.0, 1.0);
    }

    /// Serializa como JSON legivel.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// Le e valida (limite de 4 MiB). Nunca entra em `panic` com bytes arbitrarios.
    pub fn from_json(bytes: &[u8]) -> Result<ToonMaterial> {
        if bytes.len() > MATERIAL_MAX_BYTES {
            return Err(AssetError::LimitExceeded { what: "anigomat.json", limit: MATERIAL_MAX_BYTES as u64, actual: bytes.len() as u64 });
        }
        let mut m: ToonMaterial = serde_json::from_slice(bytes).map_err(|e| AssetError::parse("anigomat", e.to_string()))?;
        m.validate()?;
        m.clamp_in_place();
        Ok(m)
    }

    // -------------------------------------------------------------------------------------
    // StylizedMaterial (anigo-core)
    // -------------------------------------------------------------------------------------

    /// Converte para o material que o renderer cel consome. Perde o que o `StylizedMaterial` nao modela
    /// (rampa, ILM, mascaras...), que continuam no `.anigomat.json`.
    pub fn to_stylized(&self) -> StylizedMaterial {
        let mut s = StylizedMaterial { name: self.name.clone(), ..Default::default() };
        s.base_color = self.base.color;
        s.shade_color = [self.shade.color[0], self.shade.color[1], self.shade.color[2], 1.0];
        s.shadow_threshold = self.shade.threshold;
        s.shadow_smoothness = self.shade.smoothness;
        s.toon_steps = self.shade.steps;
        s.hue_shift = self.shade.hue_shift;
        s.mtoon_second_shade_shift = self.shade.second_shift;
        s.mtoon_second_shade_softness = self.shade.second_softness;
        s.mtoon_shade_toony = self.shade.steps >= 0.5;
        s.mtoon_main_texture_enabled = self.base.texture.is_some();
        s.mtoon_shade_texture_enabled = self.shade.texture.is_some();
        s.mtoon_second_shade_texture_enabled = self.shade.texture2.is_some();
        s.spec_intensity = self.spec.intensity;
        s.spec_power = self.spec.power;
        s.specular_color = [self.spec.color[0], self.spec.color[1], self.spec.color[2], 1.0];
        s.specular_size = self.spec.size;
        s.specular_softness = self.spec.softness;
        s.specular_offset = self.spec.offset;
        s.rim_intensity = self.rim.intensity;
        s.rim_spread = self.rim.spread;
        s.rim_color = [self.rim.color[0], self.rim.color[1], self.rim.color[2], 1.0];
        s.outline_color = [self.outline.color[0], self.outline.color[1], self.outline.color[2], 1.0];
        s.outline_width = if self.outline.mode == OutlineMode::None { 0.0 } else { self.outline.width };
        s.outline_opacity = self.outline.opacity;
        s.outline_smoothness = self.outline.smoothness;
        s.outline_depth_bias = self.outline.depth_bias;
        s.mtoon_emission_color = [self.emission.color[0], self.emission.color[1], self.emission.color[2], StylizedMaterial::default().mtoon_emission_color[3]];
        s.mtoon_emission_intensity = self.emission.intensity;
        s.mtoon_emission_texture_enabled = self.emission.texture.is_some();
        s.mtoon_matcap_enabled = self.matcap.texture.is_some();
        s.mtoon_matcap_intensity = self.matcap.intensity;
        s.mtoon_matcap_mode = match self.matcap.mode {
            MatcapMode::Multiply => 0,
            MatcapMode::Additive => 1,
        };
        if let Some(f) = &self.face_sdf {
            s.face_sdf_enabled = true;
            s.face_shadow_offset = f.offset;
            s.face_shadow_smoothness = f.smoothness;
        }
        if let Some(x) = &self.stylized {
            x.apply_to(&mut s);
        }
        s
    }

    /// Cria um material canonico a partir do material do renderer (sem texturas).
    pub fn from_stylized(s: &StylizedMaterial) -> ToonMaterial {
        let mut m = ToonMaterial { name: s.name.clone(), converted_from: Some("stylized".into()), ..Default::default() };
        m.base.color = s.base_color;
        m.shade.color = [s.shade_color[0], s.shade_color[1], s.shade_color[2]];
        m.shade.threshold = s.shadow_threshold;
        m.shade.smoothness = s.shadow_smoothness;
        m.shade.steps = s.toon_steps;
        m.shade.hue_shift = s.hue_shift;
        m.shade.second_shift = s.mtoon_second_shade_shift;
        m.shade.second_softness = s.mtoon_second_shade_softness;
        m.spec = SpecBlock {
            color: [s.specular_color[0], s.specular_color[1], s.specular_color[2]],
            intensity: s.spec_intensity,
            power: s.spec_power,
            size: s.specular_size,
            softness: s.specular_softness,
            offset: s.specular_offset,
        };
        m.rim.intensity = s.rim_intensity;
        m.rim.spread = s.rim_spread;
        m.rim.color = [s.rim_color[0], s.rim_color[1], s.rim_color[2]];
        m.outline.color = [s.outline_color[0], s.outline_color[1], s.outline_color[2]];
        m.outline.mode = if s.outline_width > 0.0 { OutlineMode::ScreenCoordinates } else { OutlineMode::None };
        m.outline.width = s.outline_width;
        m.outline.opacity = s.outline_opacity;
        m.outline.smoothness = s.outline_smoothness;
        m.outline.depth_bias = s.outline_depth_bias;
        m.emission.color = [s.mtoon_emission_color[0], s.mtoon_emission_color[1], s.mtoon_emission_color[2]];
        m.emission.intensity = s.mtoon_emission_intensity;
        m.matcap.intensity = s.mtoon_matcap_intensity;
        m.matcap.mode = if s.mtoon_matcap_mode == 1 { MatcapMode::Additive } else { MatcapMode::Multiply };
        // os extras só entram quando `to_stylized` NÃO reproduziria sozinho esses campos
        let extras = StylizedExtras::from_stylized(s);
        m.stylized = (StylizedExtras::from_stylized(&m.to_stylized()) != extras).then_some(extras);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_through_stylized() {
        let t = ToonMaterial::default();
        let s = t.to_stylized();
        let d = StylizedMaterial::default();
        // O padrao canonico coincide com o padrao do renderer nos campos que o modelam.
        assert_eq!(s.base_color, d.base_color);
        assert_eq!(s.shade_color, d.shade_color);
        assert_eq!(s.shadow_threshold, d.shadow_threshold);
        assert_eq!(s.shadow_smoothness, d.shadow_smoothness);
        assert_eq!(s.toon_steps, d.toon_steps);
        assert_eq!(s.hue_shift, d.hue_shift);
        assert_eq!(s.spec_intensity, d.spec_intensity);
        assert_eq!(s.spec_power, d.spec_power);
        assert_eq!(s.specular_size, d.specular_size);
        assert_eq!(s.rim_intensity, d.rim_intensity);
        assert_eq!(s.rim_spread, d.rim_spread);
        assert_eq!(s.rim_color, d.rim_color);
        assert_eq!(s.outline_color, d.outline_color);
        assert_eq!(s.outline_width, d.outline_width);
        let back = ToonMaterial::from_stylized(&s);
        assert_eq!(back.base.color, t.base.color);
        assert_eq!(back.shade.color, t.shade.color);
        assert_eq!(back.outline.width, t.outline.width);
        assert_eq!(back.spec, t.spec);
    }

    #[test]
    fn json_roundtrip_with_textures() {
        let mut m = ToonMaterial::named("Pele");
        m.base.texture = Some(TextureRef::from_uri(TextureRole::BaseColor, "tex/pele.png"));
        m.shade.ramp = Some(TextureRef::from_gltf_image(TextureRole::Ramp, 3));
        m.ilm = Some(IlmBlock {
            texture: TextureRef { swizzle: Some("rgba".into()), ..TextureRef::from_uri(TextureRole::Ilm, "tex/ilm.png") },
            r: IlmChannel::SpecMask,
            g: IlmChannel::ShadowThreshold,
            b: IlmChannel::SpecSize,
            a: IlmChannel::OutlineMask,
        });
        let bytes = m.to_json().unwrap();
        let back = ToonMaterial::from_json(&bytes).unwrap();
        assert_eq!(back, m);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"schema\": \"anigo.toon-material\""));
        assert!(text.contains("\"role\": \"ilm\""));
        assert!(text.contains("\"color_space\": \"srgb\""));
    }

    #[test]
    fn rejects_hostile_json() {
        assert!(ToonMaterial::from_json(b"").is_err());
        assert!(ToonMaterial::from_json(b"{").is_err());
        assert!(ToonMaterial::from_json(b"\xff\xff\xff").is_err());
        assert!(ToonMaterial::from_json(b"42").is_err());
        assert!(ToonMaterial::from_json(b"null").is_err());
        // schema errado
        assert!(ToonMaterial::from_json(br#"{"schema":"outra.coisa"}"#).is_err());
        // versao futura
        let mut m = ToonMaterial::default();
        m.schema_ver = 99;
        assert!(matches!(ToonMaterial::from_json(&serde_json::to_vec(&m).unwrap()), Err(AssetError::Unsupported(_))));
        // uri que escapa
        let mut m = ToonMaterial::default();
        m.base.texture = Some(TextureRef::from_uri(TextureRole::BaseColor, "../../etc/passwd"));
        assert!(matches!(ToonMaterial::from_json(&serde_json::to_vec(&m).unwrap()), Err(AssetError::UnsafePath(_))));
        let mut m = ToonMaterial::default();
        m.base.texture = Some(TextureRef::from_uri(TextureRole::BaseColor, "file:///etc/passwd"));
        assert!(ToonMaterial::from_json(&serde_json::to_vec(&m).unwrap()).is_err());
        // textura sem fonte / com duas fontes / swizzle ruim
        let mut m = ToonMaterial::default();
        m.base.texture = Some(TextureRef { uri: None, gltf_image: None, ..TextureRef::from_gltf_image(TextureRole::BaseColor, 0) });
        assert!(m.validate().is_err());
        let mut m = ToonMaterial::default();
        m.base.texture = Some(TextureRef { uri: Some("a.png".into()), ..TextureRef::from_gltf_image(TextureRole::BaseColor, 0) });
        assert!(m.validate().is_err());
        let mut m = ToonMaterial::default();
        m.base.texture = Some(TextureRef { swizzle: Some("xyz".into()), ..TextureRef::from_uri(TextureRole::BaseColor, "a.png") });
        assert!(m.validate().is_err());
        // NaN
        let mut m = ToonMaterial::default();
        m.shade.threshold = f32::NAN;
        assert!(m.validate().is_err());
        // grande demais
        assert!(matches!(ToonMaterial::from_json(&vec![b' '; MATERIAL_MAX_BYTES + 1]), Err(AssetError::LimitExceeded { .. })));
    }

    #[test]
    fn from_json_clamps_ranges_and_fills_defaults() {
        let m = ToonMaterial::from_json(br#"{"schema":"anigo.toon-material","schema_ver":1,"shade":{"threshold":7.0,"steps":99.0},"rim":{"intensity":50.0}}"#).unwrap();
        assert_eq!(m.shade.threshold, 1.0);
        assert_eq!(m.shade.steps, 8.0);
        assert_eq!(m.rim.intensity, 3.0);
        assert_eq!(m.outline.width, OutlineBlock::default().width); // faltando -> default
    }

    #[test]
    fn textures_listing_and_stylized_flags() {
        let mut m = ToonMaterial::default();
        assert!(m.textures().is_empty());
        m.base.texture = Some(TextureRef::from_uri(TextureRole::BaseColor, "a.png"));
        m.matcap.texture = Some(TextureRef::from_uri(TextureRole::MatCap, "m.png"));
        m.face_sdf = Some(FaceSdfBlock { texture: TextureRef::from_uri(TextureRole::FaceSdf, "f.png"), front: [0.0, 0.0, 1.0], right: [1.0, 0.0, 0.0], offset: 0.1, smoothness: 0.05 });
        assert_eq!(m.textures().len(), 3);
        let s = m.to_stylized();
        assert!(s.mtoon_main_texture_enabled && s.mtoon_matcap_enabled && s.face_sdf_enabled);
        assert!(!s.mtoon_shade_texture_enabled);
        assert_eq!(s.face_shadow_offset, 0.1);
        m.outline.mode = OutlineMode::None;
        assert_eq!(m.to_stylized().outline_width, 0.0);
    }

    #[test]
    fn color_space_defaults() {
        assert_eq!(TextureRole::BaseColor.default_color_space(), ColorSpace::Srgb);
        assert_eq!(TextureRole::Normal.default_color_space(), ColorSpace::Linear);
        assert_eq!(TextureRole::FaceSdf.default_color_space(), ColorSpace::Linear);
        assert_eq!(TextureRole::Ilm.default_color_space(), ColorSpace::Linear);
        assert_eq!(TextureRole::Emission.default_color_space(), ColorSpace::Srgb);
    }
}

#[cfg(test)]
mod stylized_roundtrip_tests {
    use super::*;

    fn tweaked() -> StylizedMaterial {
        let mut s = StylizedMaterial { name: "Roupa".into(), ..Default::default() };
        s.base_color = [0.2, 0.4, 0.8, 1.0];
        s.shade_color = [0.1, 0.2, 0.5, 1.0];
        s.outline_color = [0.05, 0.05, 0.1, 1.0];
        s.outline_width = 0.007;
        s.rim_intensity = 1.7;
        s.spec_power = 77.0;
        s.hue_shift = 22.0;
        s.toon_steps = 2.0;
        s.ao_intensity = 0.3;
        s.mtoon_matcap_enabled = true;
        s.mtoon_matcap_intensity = 0.6;
        s.mtoon_matcap_mode = 1;
        s.mtoon_emission_intensity = 0.4;
        s.face_sdf_enabled = true;
        s.face_shadow_offset = 0.05;
        s.face_shadow_smoothness = 0.04;
        s.eye_enabled = true;
        s.eye_depth_scale = 0.2;
        s.npr.ramp_blend = 1.0;
        s.npr.ramp_row = 2.0;
        s.npr.outline_taper = 0.5;
        s.npr.face_sdf_mirror_center = 0.5;
        s.npr.ramp_custom.set_stops(2, true, &[(0.0, 0.0), (0.4, 0.6), (0.8, 1.0)]);
        s
    }

    #[test]
    fn stylized_material_survives_the_anigomat_json_roundtrip_including_npr() {
        let s = tweaked();
        let m = ToonMaterial::from_stylized(&s);
        assert!(m.stylized.is_some());
        let bytes = m.to_json().unwrap();
        let back = ToonMaterial::from_json(&bytes).unwrap();
        assert_eq!(back, m);
        let s2 = back.to_stylized();
        assert_eq!(s2, s, "preset deve devolver exatamente o material do renderer");
    }

    #[test]
    fn default_material_has_no_stylized_block_and_old_files_still_load() {
        let m = ToonMaterial::from_stylized(&StylizedMaterial::default());
        assert!(m.stylized.is_none());
        let text = String::from_utf8(m.to_json().unwrap()).unwrap();
        assert!(!text.contains("\"stylized\":"), "aditivo: omitido no padrao");
        let old = ToonMaterial::from_json(br#"{"schema":"anigo.toon-material","schema_ver":1}"#).unwrap();
        assert!(old.stylized.is_none());
        assert_eq!(old.to_stylized().npr, anigo_core::NprParams::default());
    }

    #[test]
    fn hostile_stylized_block_is_rejected_or_clamped() {
        let mut m = ToonMaterial::default();
        let mut x = StylizedExtras::default();
        x.ao_intensity = f32::NAN;
        m.stylized = Some(x);
        assert!(m.validate().is_err());
        let mut m = ToonMaterial::default();
        let mut x = StylizedExtras::default();
        x.npr.ramp_blend = 9.0;
        x.npr.ramp_custom.count = [9, 9, 9, 9];
        m.stylized = Some(x);
        m.validate().unwrap();
        let s = m.to_stylized();
        assert_eq!(s.npr.ramp_blend, 1.0);
        assert!(s.npr.ramp_custom.is_default(), "contagem invalida volta ao contrato");
    }
}
