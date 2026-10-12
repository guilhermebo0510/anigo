//! Saída de render do projeto (workspace Render): resolução, escala, supersampling, fundo transparente,
//! formato do arquivo, AOVs e DoF.
//!
//! Tudo aqui é **dado de apresentação** do projeto: `RenderOutput` vive em `RenderState.output`, é
//! omitido do JSON canônico enquanto estiver no padrão (fixtures e hashes congelados não mudam) e é
//! editado pelo comando `SetRenderOutput` (inverso exato, undo/redo). Caminho de pasta/nome de arquivo
//! NÃO entram: são preferências da máquina, não do projeto.

use crate::project::{ColorSpace, ProjectState};
use crate::scene::DofSettings;
use serde::{Deserialize, Serialize};

/// Maior lado persistido (o renderer limita o que realmente renderiza ao limite da GPU).
pub const OUTPUT_MAX_SIDE: u32 = 16_384;
/// Menor lado aceito.
pub const OUTPUT_MIN_SIDE: u32 = 16;

/// Formato do arquivo da imagem principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// PNG 8 bits (RGBA quando o fundo é transparente, senão RGB).
    #[default]
    Png8,
    /// PNG 16 bits: a média do supersampling vira precisão real (sem faixas em gradientes).
    Png16,
    /// JPEG (sem alfa: achata sobre a cor de fundo).
    Jpeg,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 3] = [OutputFormat::Png8, OutputFormat::Png16, OutputFormat::Jpeg];

    /// Extensão do arquivo (sem ponto).
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Png8 | OutputFormat::Png16 => "png",
            OutputFormat::Jpeg => "jpg",
        }
    }

    /// Suporta canal alfa?
    pub fn has_alpha(self) -> bool {
        !matches!(self, OutputFormat::Jpeg)
    }
}

/// AOVs adicionais (cada um vira um arquivo extra). Todos desligados por padrão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AovSelection {
    /// Contorno isolado (RGBA só com a linha, com cobertura).
    pub outline: bool,
    /// Profundidade normalizada (cinza 16 bits; branco = perto).
    pub depth: bool,
    /// Máscara de cobertura (cinza; alfa de tudo que é geometria).
    pub mask: bool,
    /// ID por objeto (cor única determinística por nó; sem antialiasing).
    pub object_id: bool,
    /// Normais no espaço da câmera (RGB; sem antialiasing).
    pub normal: bool,
}

impl AovSelection {
    pub fn any(&self) -> bool {
        self.outline || self.depth || self.mask || self.object_id || self.normal
    }

    /// Precisa do rasterizador por software (profundidade/ID/normal/máscara)?
    pub fn needs_raster(&self) -> bool {
        self.depth || self.mask || self.object_id || self.normal
    }
}

/// Configurações de saída do render.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderOutput {
    pub width: u32,
    pub height: u32,
    /// Escala (%) sobre a resolução — para renders de teste com as mesmas proporções.
    pub scale_percent: u32,
    /// Supersampling (1 = desligado): renderiza a N× e reduz.
    pub supersample: u32,
    /// Fundo transparente (alfa 0 fora da geometria).
    pub transparent: bool,
    pub format: OutputFormat,
    pub jpeg_quality: u8,
    /// Densidade de pixels gravada em `pHYs` (metadado; não muda a imagem).
    pub dpi: u32,
    pub aov: AovSelection,
    pub dof: DofSettings,
}

impl Default for RenderOutput {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            scale_percent: 100,
            supersample: 1,
            transparent: false,
            format: OutputFormat::Png8,
            jpeg_quality: 92,
            dpi: 72,
            aov: AovSelection::default(),
            dof: DofSettings::default(),
        }
    }
}

impl RenderOutput {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Proporção da imagem (largura/altura).
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }

    /// Tamanho final do arquivo: resolução × escala (arredondado, mínimo 1 px).
    pub fn target_size(&self) -> (u32, u32) {
        let f = |v: u32| (((v as u64 * self.scale_percent as u64) + 50) / 100).clamp(1, OUTPUT_MAX_SIDE as u64 * 4) as u32;
        (f(self.width), f(self.height))
    }

    /// Valida domínio (sem estado).
    pub fn validate(&self) -> Result<(), String> {
        check_side("width", self.width)?;
        check_side("height", self.height)?;
        check_u32("scale_percent", self.scale_percent, 10, 400)?;
        check_u32("supersample", self.supersample, 1, 4)?;
        check_u32("jpeg_quality", self.jpeg_quality as u32, 1, 100)?;
        check_u32("dpi", self.dpi, 36, 1200)?;
        validate_dof(&self.dof)
    }

    /// Normalização leniente de dados externos (clamp; nunca falha).
    pub fn sanitize(&mut self) {
        self.width = self.width.clamp(OUTPUT_MIN_SIDE, OUTPUT_MAX_SIDE);
        self.height = self.height.clamp(OUTPUT_MIN_SIDE, OUTPUT_MAX_SIDE);
        self.scale_percent = self.scale_percent.clamp(10, 400);
        self.supersample = self.supersample.clamp(1, 4);
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        self.dpi = self.dpi.clamp(36, 1200);
        if validate_dof(&self.dof).is_err() {
            self.dof = DofSettings::default();
        }
    }
}

fn check_side(field: &str, v: u32) -> Result<(), String> {
    check_u32(field, v, OUTPUT_MIN_SIDE, OUTPUT_MAX_SIDE)
}

fn check_u32(field: &str, v: u32, lo: u32, hi: u32) -> Result<(), String> {
    if v < lo || v > hi {
        Err(format!("{field} must be within [{lo}, {hi}], found {v}"))
    } else {
        Ok(())
    }
}

fn check_f32(field: &str, v: f32, lo: f32, hi: f32) -> Result<(), String> {
    if !v.is_finite() || v < lo || v > hi {
        Err(format!("{field} must be a finite value within [{lo}, {hi}], found {v}"))
    } else {
        Ok(())
    }
}

/// Faixas do DoF (as mesmas que a UI oferece).
pub fn validate_dof(d: &DofSettings) -> Result<(), String> {
    check_f32("dof.focus_distance", d.focus_distance, 0.05, 1000.0)?;
    check_f32("dof.f_number", d.f_number, 0.5, 64.0)?;
    check_f32("dof.focal_mm", d.focal_mm, 8.0, 800.0)?;
    check_f32("dof.max_radius_px", d.max_radius_px, 0.0, 64.0)?;
    check_u32("dof.bokeh_shape", d.bokeh_shape, 0, 1)
}

/// Patch parcial de [`RenderOutput`] (+ espaço de cor de saída e aspecto da câmera de render).
///
/// `camera_aspect` existe para o **inverso** ser exato: quando só `width`/`height` mudam, o aspecto da
/// câmera de render acompanha (`largura/altura`); o inverso restaura o valor anterior literalmente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RenderOutputPatch {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale_percent: Option<u32>,
    pub supersample: Option<u32>,
    pub transparent: Option<bool>,
    pub format: Option<OutputFormat>,
    pub jpeg_quality: Option<u8>,
    pub dpi: Option<u32>,
    pub aov: Option<AovSelection>,
    pub dof: Option<DofSettings>,
    /// Espaço de cor do arquivo (`render.color.display_space`): sRGB ou Display P3.
    pub display_space: Option<ColorSpace>,
    pub camera_aspect: Option<f32>,
    /// Planos near/far da câmera de render (`scene.camera.camera.z_near/z_far`), em metros.
    pub camera_near: Option<f32>,
    pub camera_far: Option<f32>,
}

impl RenderOutputPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Valida os valores presentes (sem estado).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(v) = self.width {
            check_side("width", v)?;
        }
        if let Some(v) = self.height {
            check_side("height", v)?;
        }
        if let Some(v) = self.scale_percent {
            check_u32("scale_percent", v, 10, 400)?;
        }
        if let Some(v) = self.supersample {
            check_u32("supersample", v, 1, 4)?;
        }
        if let Some(v) = self.jpeg_quality {
            check_u32("jpeg_quality", v as u32, 1, 100)?;
        }
        if let Some(v) = self.dpi {
            check_u32("dpi", v, 36, 1200)?;
        }
        if let Some(d) = &self.dof {
            validate_dof(d)?;
        }
        if let Some(s) = self.display_space {
            if !matches!(s, ColorSpace::Srgb | ColorSpace::DisplayP3) {
                return Err("display_space must be srgb or display_p3".into());
            }
        }
        if let Some(a) = self.camera_aspect {
            check_f32("camera_aspect", a, 0.05, 20.0)?;
        }
        if let Some(n) = self.camera_near {
            check_f32("camera_near", n, 0.001, 1000.0)?;
        }
        if let Some(f) = self.camera_far {
            check_f32("camera_far", f, 0.1, 100_000.0)?;
        }
        Ok(())
    }

    /// Validação com o estado: near < far depois de aplicar.
    pub fn validate_against(&self, state: &ProjectState) -> Result<(), String> {
        self.validate()?;
        let cam = &state.scene.camera.camera;
        let near = self.camera_near.unwrap_or(cam.z_near);
        let far = self.camera_far.unwrap_or(cam.z_far);
        if near >= far {
            return Err(format!("camera near ({near}) must be smaller than far ({far})"));
        }
        Ok(())
    }

    /// Aplica ao projeto (assume `validate` ok).
    pub fn apply(&self, state: &mut ProjectState) {
        let o = &mut state.render.output;
        let dims_changed = self.width.is_some_and(|w| w != o.width) || self.height.is_some_and(|h| h != o.height);
        if let Some(v) = self.width {
            o.width = v;
        }
        if let Some(v) = self.height {
            o.height = v;
        }
        if let Some(v) = self.scale_percent {
            o.scale_percent = v;
        }
        if let Some(v) = self.supersample {
            o.supersample = v;
        }
        if let Some(v) = self.transparent {
            o.transparent = v;
        }
        if let Some(v) = self.format {
            o.format = v;
        }
        if let Some(v) = self.jpeg_quality {
            o.jpeg_quality = v;
        }
        if let Some(v) = self.dpi {
            o.dpi = v;
        }
        if let Some(v) = self.aov {
            o.aov = v;
        }
        if let Some(v) = self.dof {
            o.dof = v;
        }
        let aspect = o.aspect();
        if let Some(s) = self.display_space {
            state.render.color.display_space = s;
        }
        match self.camera_aspect {
            Some(a) => state.scene.camera.camera.aspect = a,
            None if dims_changed => state.scene.camera.camera.aspect = aspect,
            None => {}
        }
        if let Some(n) = self.camera_near {
            state.scene.camera.camera.z_near = n;
        }
        if let Some(f) = self.camera_far {
            state.scene.camera.camera.z_far = f;
        }
    }

    /// `true` quando aplicar muda algo (senão o comando é `NoOp`).
    pub fn would_change(&self, state: &ProjectState) -> bool {
        let mut probe = state.clone();
        self.apply(&mut probe);
        probe.render != state.render || probe.scene.camera.camera != state.scene.camera.camera
    }

    /// Patch que desfaz este (valores atuais dos campos tocados + aspecto exato da câmera).
    pub fn inverse(&self, state: &ProjectState) -> RenderOutputPatch {
        let o = &state.render.output;
        let touches_dims = self.width.is_some() || self.height.is_some();
        RenderOutputPatch {
            width: touches_dims.then_some(o.width),
            height: touches_dims.then_some(o.height),
            scale_percent: self.scale_percent.map(|_| o.scale_percent),
            supersample: self.supersample.map(|_| o.supersample),
            transparent: self.transparent.map(|_| o.transparent),
            format: self.format.map(|_| o.format),
            jpeg_quality: self.jpeg_quality.map(|_| o.jpeg_quality),
            dpi: self.dpi.map(|_| o.dpi),
            aov: self.aov.map(|_| o.aov),
            dof: self.dof.map(|_| o.dof),
            display_space: self.display_space.map(|_| state.render.color.display_space),
            camera_aspect: (touches_dims || self.camera_aspect.is_some()).then_some(state.scene.camera.camera.aspect),
            camera_near: self.camera_near.map(|_| state.scene.camera.camera.z_near),
            camera_far: self.camera_far.map(|_| state.scene.camera.camera.z_far),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_canonical_hd_frame_and_is_omitted_from_json() {
        let o = RenderOutput::default();
        assert!(o.is_default());
        assert_eq!((o.width, o.height), (1920, 1080));
        assert_eq!(o.target_size(), (1920, 1080));
        let project = ProjectState::default();
        let json = project.to_canonical_json().unwrap();
        assert!(!json.contains("\"output\""), "o padrão não pode aparecer no JSON canônico");
        // e o aspecto canônico da câmera é o do HD (a derivação 1920/1080 não muda o padrão)
        assert_eq!(o.aspect(), project.scene.camera.camera.aspect);
    }

    #[test]
    fn non_default_output_round_trips_through_the_project_json() {
        let mut p = ProjectState::default();
        p.render.output.width = 2480;
        p.render.output.height = 3508;
        p.render.output.dpi = 300;
        p.render.output.aov.outline = true;
        let json = p.to_canonical_json().unwrap();
        assert!(json.contains("\"output\""));
        let back = ProjectState::from_json(&json).unwrap();
        assert_eq!(back.render.output, p.render.output);
    }

    #[test]
    fn scale_changes_the_file_size_not_the_frame() {
        let o = RenderOutput { scale_percent: 50, ..Default::default() };
        assert_eq!(o.target_size(), (960, 540));
        let o = RenderOutput { width: 2480, height: 3508, scale_percent: 33, ..Default::default() };
        assert_eq!(o.target_size(), (818, 1158));
    }

    #[test]
    fn patch_follows_the_camera_aspect_and_inverse_is_exact() {
        let mut p = ProjectState::default();
        p.scene.camera.camera.aspect = 1.234; // valor "estranho" vindo de um projeto antigo
        let before = p.clone();
        let patch = RenderOutputPatch { width: Some(1000), height: Some(500), ..Default::default() };
        patch.validate().unwrap();
        assert!(patch.would_change(&p));
        let inv = patch.inverse(&p);
        patch.apply(&mut p);
        assert_eq!((p.render.output.width, p.render.output.height), (1000, 500));
        assert!((p.scene.camera.camera.aspect - 2.0).abs() < 1e-6);
        inv.apply(&mut p);
        assert_eq!(p, before, "o inverso devolve até o aspecto original da câmera");
    }

    #[test]
    fn invalid_values_are_rejected_and_sanitize_clamps() {
        assert!(RenderOutputPatch { width: Some(3), ..Default::default() }.validate().is_err());
        assert!(RenderOutputPatch { supersample: Some(9), ..Default::default() }.validate().is_err());
        assert!(RenderOutputPatch { display_space: Some(ColorSpace::LinearSrgb), ..Default::default() }.validate().is_err());
        let mut o = RenderOutput { width: 1, height: 99_999, supersample: 0, ..Default::default() };
        o.sanitize();
        assert_eq!((o.width, o.height, o.supersample), (OUTPUT_MIN_SIDE, OUTPUT_MAX_SIDE, 1));
        assert!(o.validate().is_ok());
    }
}
