//! Ferramentas de paleta do workspace Shading: derivar a cor de sombra da cor base (HSV), hue-shift,
//! harmonias de cor e paletas salvas (arquivo JSON). Tudo em sRGB 0..1, como os campos do material.

use anigo_core::npr::{hsv_to_rgb, rgb_to_hsv};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Como a sombra deriva da base (HSV): matiz rotacionado, saturação e valor multiplicados.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadeRecipe {
    /// Rotação de matiz em graus (negativo = para o frio/azulado, o padrão do anime).
    pub hue_shift_deg: f32,
    /// Multiplicador da saturação da base.
    pub saturation: f32,
    /// Multiplicador do valor (brilho) da base.
    pub value: f32,
}

impl Default for ShadeRecipe {
    fn default() -> Self {
        Self { hue_shift_deg: -15.0, saturation: 1.15, value: 0.78 }
    }
}

/// Cor de sombra derivada de `base` pela receita (HSV, resultado limitado a 0..1).
pub fn derive_shade(base: [f32; 3], r: ShadeRecipe) -> [f32; 3] {
    let [h, s, v] = rgb_to_hsv([base[0].clamp(0.0, 1.0), base[1].clamp(0.0, 1.0), base[2].clamp(0.0, 1.0)]);
    let h2 = (h + r.hue_shift_deg / 360.0).rem_euclid(1.0);
    let out = hsv_to_rgb([h2, (s * r.saturation).clamp(0.0, 1.0), (v * r.value).clamp(0.0, 1.0)]);
    [out[0].clamp(0.0, 1.0), out[1].clamp(0.0, 1.0), out[2].clamp(0.0, 1.0)]
}

/// Gira o matiz de uma cor (graus), mantendo saturação e valor.
pub fn hue_shift(c: [f32; 3], deg: f32) -> [f32; 3] {
    let [h, s, v] = rgb_to_hsv(c);
    hsv_to_rgb([(h + deg / 360.0).rem_euclid(1.0), s, v])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harmony {
    Complementary,
    Analogous,
    Triad,
    Tetrad,
    SplitComplementary,
}

impl Harmony {
    pub const ALL: [Harmony; 5] = [Harmony::Complementary, Harmony::Analogous, Harmony::Triad, Harmony::Tetrad, Harmony::SplitComplementary];

    pub fn label_key(self) -> &'static str {
        match self {
            Harmony::Complementary => "shading.harmony.complementary",
            Harmony::Analogous => "shading.harmony.analogous",
            Harmony::Triad => "shading.harmony.triad",
            Harmony::Tetrad => "shading.harmony.tetrad",
            Harmony::SplitComplementary => "shading.harmony.split",
        }
    }

    /// Rotações de matiz (graus) em relação à base, sem incluir a própria base.
    pub fn offsets(self) -> &'static [f32] {
        match self {
            Harmony::Complementary => &[180.0],
            Harmony::Analogous => &[-30.0, 30.0],
            Harmony::Triad => &[120.0, 240.0],
            Harmony::Tetrad => &[90.0, 180.0, 270.0],
            Harmony::SplitComplementary => &[150.0, 210.0],
        }
    }
}

/// Cores da harmonia (a base vem primeiro).
pub fn harmony_colors(base: [f32; 3], h: Harmony) -> Vec<[f32; 3]> {
    let mut out = vec![base];
    out.extend(h.offsets().iter().map(|d| hue_shift(base, *d)));
    out
}

/// Luminância relativa aproximada (Rec. 709 sobre sRGB) — para ordenar/mostrar contraste.
pub fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn to_hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

pub fn from_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([p(0)?, p(2)?, p(4)?])
}

/// Paleta nomeada (salva em `<pasta>/<nome>.anigopalette.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Palette {
    pub name: String,
    pub colors: Vec<[f32; 3]>,
}

pub const PALETTE_SUFFIX: &str = ".anigopalette.json";
pub const MAX_SWATCHES: usize = 64;

fn sanitize_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "paleta".into()
    } else {
        s.chars().take(60).collect()
    }
}

impl Palette {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), colors: Vec::new() }
    }

    /// Acrescenta uma cor (sem repetir, no máximo [`MAX_SWATCHES`]). `true` se entrou.
    pub fn add(&mut self, c: [f32; 3]) -> bool {
        let c = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
        if self.colors.len() >= MAX_SWATCHES || self.colors.iter().any(|x| x.iter().zip(c.iter()).all(|(a, b)| (a - b).abs() < 1.0 / 512.0)) {
            return false;
        }
        self.colors.push(c);
        true
    }

    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}{PALETTE_SUFFIX}", sanitize_name(&self.name)))
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = self.path_in(dir);
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn load(path: &Path) -> Result<Palette, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > 1024 * 1024 {
            return Err("paleta grande demais".into());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut p: Palette = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        p.colors.truncate(MAX_SWATCHES);
        for c in &mut p.colors {
            for v in c.iter_mut() {
                *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            }
        }
        Ok(p)
    }
}

/// Paletas salvas na pasta (ordem alfabética).
pub fn list_palettes(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(PALETTE_SUFFIX).map(|n| (n.to_string(), e.path()))
        })
        .collect();
    v.sort();
    v
}

/// Pasta das paletas do projeto: `ANIGO_SHADING_DIR/palettes` ou `<dados locais>/anigo-studio/palettes`.
pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ANIGO_SHADING_DIR") {
        return PathBuf::from(d).join("palettes");
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("anigo-studio").join("palettes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 2e-3)
    }

    #[test]
    fn derived_shade_is_darker_and_cooler_for_light_skin() {
        let skin = [0.98, 0.85, 0.75];
        let sh = derive_shade(skin, ShadeRecipe::default());
        assert!(luma(sh) < luma(skin) - 0.1);
        let (hb, hs) = (rgb_to_hsv(skin)[0], rgb_to_hsv(sh)[0]);
        // matiz da pele ~0.07; -15° = -0.0417 → ~0.03 (mais para o vermelho/frio)
        assert!((hb - hs - 15.0 / 360.0).abs() < 0.01, "{hb} {hs}");
        assert!(rgb_to_hsv(sh)[1] > rgb_to_hsv(skin)[1], "saturação sobe");
    }

    #[test]
    fn identity_recipe_returns_the_base_and_extremes_stay_in_range() {
        let base = [0.2, 0.6, 0.4];
        assert!(near(derive_shade(base, ShadeRecipe { hue_shift_deg: 0.0, saturation: 1.0, value: 1.0 }), base));
        let extreme = derive_shade([1.0, 0.0, 0.0], ShadeRecipe { hue_shift_deg: 720.0, saturation: 9.0, value: 9.0 });
        assert!(extreme.iter().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(derive_shade([0.0; 3], ShadeRecipe::default()), [0.0; 3]);
    }

    #[test]
    fn hue_shift_full_turn_is_identity_and_half_turn_is_complementary() {
        let c = [0.9, 0.3, 0.2];
        assert!(near(hue_shift(c, 360.0), c));
        assert!(near(hue_shift([1.0, 0.0, 0.0], 180.0), [0.0, 1.0, 1.0]));
        assert!(near(hue_shift([1.0, 0.0, 0.0], 120.0), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn harmonies_have_the_right_size_and_keep_saturation_and_value() {
        let base = [0.8, 0.4, 0.2];
        for h in Harmony::ALL {
            let cols = harmony_colors(base, h);
            assert_eq!(cols.len(), 1 + h.offsets().len());
            assert_eq!(cols[0], base);
            for c in &cols[1..] {
                let (a, b) = (rgb_to_hsv(base), rgb_to_hsv(*c));
                assert!((a[1] - b[1]).abs() < 1e-3 && (a[2] - b[2]).abs() < 1e-3);
            }
        }
        assert_eq!(harmony_colors(base, Harmony::Tetrad).len(), 4);
    }

    #[test]
    fn hex_roundtrip_and_rejects_garbage() {
        assert_eq!(to_hex([1.0, 0.0, 0.5]), "#ff0080");
        assert!(near(from_hex("#ff0080").unwrap(), [1.0, 0.0, 0.502]));
        assert_eq!(from_hex("ff0080"), from_hex("#ff0080"));
        for bad in ["", "#12", "#gggggg", "#1234567", "é12345"] {
            assert!(from_hex(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn palette_saves_loads_and_dedups() {
        let dir = std::env::temp_dir().join(format!("anigo-pal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut p = Palette::new("Pele / Cabelo 1");
        assert!(p.add([0.9, 0.7, 0.6]));
        assert!(!p.add([0.9, 0.7, 0.6]), "repetida");
        assert!(p.add([0.1, 0.1, 0.3]));
        let path = p.save(&dir).unwrap();
        assert!(path.file_name().unwrap().to_string_lossy().ends_with(PALETTE_SUFFIX));
        assert!(!path.to_string_lossy().contains(' '));
        let back = Palette::load(&path).unwrap();
        assert_eq!(back, p);
        assert_eq!(list_palettes(&dir).len(), 1);
        // hostil
        std::fs::write(dir.join("ruim.anigopalette.json"), b"{\"name\":\"x\",\"colors\":[[2.0,-1.0,0.5]]}").unwrap();
        let bad = Palette::load(&dir.join("ruim.anigopalette.json")).unwrap();
        assert_eq!(bad.colors[0], [1.0, 0.0, 0.5]);
        std::fs::write(dir.join("lixo.anigopalette.json"), b"nao e json").unwrap();
        assert!(Palette::load(&dir.join("lixo.anigopalette.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_is_capped() {
        let mut p = Palette::new("x");
        for i in 0..200 {
            p.add([i as f32 / 200.0, 0.0, 0.0]);
        }
        assert_eq!(p.colors.len(), MAX_SWATCHES);
    }
}
