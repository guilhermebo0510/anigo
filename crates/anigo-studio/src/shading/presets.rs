//! Biblioteca de presets de material: embutidos (código) e salvos (`.anigomat.json` como ativo
//! `Material` da Biblioteca, com miniatura em shader ball renderizada pelo renderer cel canônico).

use super::palette::{derive_shade, ShadeRecipe};
use anigo_assets::{AssetId, AssetKind, ImportOptions, Library, Query, ToonMaterial};
use anigo_core::StylizedMaterial;
use std::path::PathBuf;

/// Preset embutido: id estável (chave de i18n `shading.preset.<id>`) e construtor.
pub struct BuiltIn {
    pub id: &'static str,
    pub build: fn() -> StylizedMaterial,
}

fn shaded(base: [f32; 3], recipe: ShadeRecipe) -> StylizedMaterial {
    let s = derive_shade(base, recipe);
    StylizedMaterial { base_color: [base[0], base[1], base[2], 1.0], shade_color: [s[0], s[1], s[2], 1.0], ..Default::default() }
}

fn skin() -> StylizedMaterial {
    let mut m = shaded([0.98, 0.84, 0.74], ShadeRecipe { hue_shift_deg: -12.0, saturation: 1.5, value: 0.82 });
    m.name = "Pele anime".into();
    m.spec_intensity = 0.1;
    m.rim_intensity = 0.5;
    m.rim_color = [1.0, 0.82, 0.74, 1.0];
    m.outline_width = 0.004;
    m.outline_color = [0.30, 0.14, 0.12, 1.0];
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_ink_blend = 0.6;
    m
}

fn hair() -> StylizedMaterial {
    let mut m = shaded([0.30, 0.22, 0.52], ShadeRecipe { hue_shift_deg: -20.0, saturation: 1.1, value: 0.78 });
    m.name = "Cabelo com brilho".into();
    m.spec_intensity = 1.1;
    m.spec_power = 60.0;
    m.specular_size = 0.3;
    m.specular_softness = 0.02;
    m.specular_offset = 0.15;
    m.rim_intensity = 0.8;
    m.rim_color = [0.62, 0.55, 1.0, 1.0];
    m.outline_width = 0.0045;
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_taper = 0.5;
    m.npr.outline_ink_blend = 0.8;
    m
}

fn cloth() -> StylizedMaterial {
    let mut m = shaded([0.84, 0.36, 0.40], ShadeRecipe { hue_shift_deg: -18.0, saturation: 1.1, value: 0.78 });
    m.name = "Tecido macio".into();
    m.spec_intensity = 0.0;
    m.rim_intensity = 0.4;
    m.shadow_smoothness = 0.12;
    m.npr.ramp_blend = 1.0;
    m.npr.ramp_row = 2.0;
    m.npr.ramp_custom.set_stops(2, false, &[(0.0, 0.0), (0.38, 0.45), (0.5, 0.55), (0.62, 1.0)]);
    m.outline_width = 0.003;
    m.npr.outline_ink_blend = 0.6;
    m
}

fn metal() -> StylizedMaterial {
    let mut m = shaded([0.72, 0.76, 0.82], ShadeRecipe { hue_shift_deg: 8.0, saturation: 1.1, value: 0.72 });
    m.name = "Metal (matcap)".into();
    m.spec_intensity = 1.4;
    m.spec_power = 90.0;
    m.specular_size = 0.25;
    m.mtoon_matcap_enabled = true;
    m.mtoon_matcap_mode = 1;
    m.mtoon_matcap_intensity = 0.25;
    m.rim_intensity = 1.0;
    m.rim_color = [0.85, 0.92, 1.0, 1.0];
    m.outline_width = 0.004;
    m
}

fn hard_cel() -> StylizedMaterial {
    let mut m = shaded([0.55, 0.78, 0.55], ShadeRecipe { hue_shift_deg: -25.0, saturation: 1.3, value: 0.74 });
    m.name = "Cel duro (1 degrau)".into();
    m.shadow_smoothness = 0.0;
    m.spec_intensity = 0.0;
    m.rim_intensity = 0.0;
    m.outline_width = 0.006;
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_ink_blend = 0.8;
    m.npr.outline_taper = 0.4;
    m
}

fn soft_illustration() -> StylizedMaterial {
    let mut m = shaded([0.92, 0.78, 0.55], ShadeRecipe { hue_shift_deg: -10.0, saturation: 1.2, value: 0.8 });
    m.name = "Ilustração macia".into();
    m.shadow_smoothness = 0.28;
    m.toon_steps = 0.0;
    m.spec_intensity = 0.2;
    m.rim_intensity = 0.3;
    m.outline_width = 0.0;
    m
}

fn full_anime() -> StylizedMaterial {
    let mut m = shaded([0.96, 0.82, 0.74], ShadeRecipe::default());
    m.name = "Anime completo (rampa + contorno)".into();
    m.npr.ramp_blend = 1.0;
    m.npr.ramp_row = 2.0;
    m.npr.ramp_custom.set_stops(2, true, &[(0.0, 0.0), (0.32, 0.55), (0.62, 1.0)]);
    m.spec_intensity = 0.5;
    m.rim_intensity = 1.0;
    m.outline_width = 0.005;
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_distance_falloff = 0.8;
    m.npr.outline_taper = 0.5;
    m.npr.outline_ink_blend = 0.7;
    m
}

pub const BUILT_IN: &[BuiltIn] = &[
    BuiltIn { id: "skin", build: skin },
    BuiltIn { id: "hair", build: hair },
    BuiltIn { id: "cloth", build: cloth },
    BuiltIn { id: "metal", build: metal },
    BuiltIn { id: "hard_cel", build: hard_cel },
    BuiltIn { id: "soft", build: soft_illustration },
    BuiltIn { id: "full_anime", build: full_anime },
];

// ───────────────────────────── Biblioteca ─────────────────────────────────

/// Linha de um preset salvo na Biblioteca.
#[derive(Debug, Clone)]
pub struct PresetRow {
    pub id: AssetId,
    pub name: String,
    pub hash: anigo_assets::ContentHash,
}

fn safe_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let s = s.trim_matches('_');
    if s.is_empty() { "material".into() } else { s.chars().take(60).collect() }
}

/// Salva `m` como ativo `Material` da Biblioteca (`<nome>.anigomat.json`, tag `preset`). Devolve o id
/// (se já existir um ativo com o mesmo conteúdo, o existente).
pub fn save_preset(lib: &Library, name: &str, m: &StylizedMaterial) -> Result<AssetId, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("nome vazio".into());
    }
    let mut tm = ToonMaterial::from_stylized(m);
    tm.name = name.to_string();
    tm.converted_from = None;
    let bytes = tm.to_json().map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join(format!("anigo-preset-{}-{}", std::process::id(), anigo_core::ids::fnv1a64(name)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file: PathBuf = dir.join(format!("{}{}", safe_file_name(name), anigo_assets::MATERIAL_SUFFIX));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let opts = ImportOptions { name: Some(name.to_string()), kind: Some(AssetKind::Material), tags: vec!["preset".into(), "shading".into()], author: Some("ANIGO Studio".into()), ..Default::default() };
    let rep = lib.import_blocking(&file, opts);
    let _ = std::fs::remove_dir_all(&dir);
    let rep = rep.map_err(|e| e.to_string())?;
    if let Some(id) = rep.imported.first().or(rep.revised.first()) {
        return Ok(*id);
    }
    if let Some((_, id)) = rep.duplicates.first() {
        return Ok(*id);
    }
    if let Some((p, e)) = rep.failed.first() {
        return Err(format!("{}: {e}", p.display()));
    }
    Err("nada foi importado".into())
}

/// Presets (ativos `Material`) da Biblioteca, mais recentes primeiro.
pub fn list_presets(lib: &Library) -> Vec<PresetRow> {
    let q = Query { kinds: vec![AssetKind::Material], limit: 200, ..Default::default() };
    lib.query(&q)
        .map(|p| p.items.into_iter().map(|r| PresetRow { id: r.id, name: r.name, hash: r.content_hash }).collect())
        .unwrap_or_default()
}

/// Carrega o `.anigomat.json` do ativo e converte para o material do renderer.
pub fn load_preset(lib: &Library, id: &AssetId) -> Result<StylizedMaterial, String> {
    let files = lib.resolve_files(id).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&files.primary).map_err(|e| format!("{}: {e}", files.primary.display()))?;
    if meta.len() > anigo_assets::material::MATERIAL_MAX_BYTES as u64 {
        return Err("material grande demais".into());
    }
    let bytes = std::fs::read(&files.primary).map_err(|e| e.to_string())?;
    let tm = ToonMaterial::from_json(&bytes).map_err(|e| e.to_string())?;
    Ok(tm.to_stylized())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_presets_are_valid_distinct_and_survive_the_toon_material_format() {
        let mut seen = std::collections::HashSet::new();
        for p in BUILT_IN {
            assert!(seen.insert(p.id));
            let m = (p.build)();
            // faixas válidas segundo a tabela de parâmetros
            for prm in super::super::params::PARAMS {
                if let super::super::params::Kind::Float { min, max, .. } = prm.kind {
                    let v = (prm.get)(&m).f();
                    assert!(v >= min - 1e-5 && v <= max + 1e-5, "{}: {} = {v} fora de {min}..{max}", p.id, prm.id);
                }
            }
            assert_ne!(m, StylizedMaterial::default(), "{} não pode ser o padrão", p.id);
            // ida e volta pelo .anigomat.json sem perder nada (nome incluído)
            let tm = ToonMaterial::from_stylized(&m);
            let back = ToonMaterial::from_json(&tm.to_json().unwrap()).unwrap().to_stylized();
            assert_eq!(back, m, "{} mudou ao passar pelo .anigomat.json", p.id);
        }
        assert!(BUILT_IN.len() >= 6);
    }

    #[test]
    fn saving_listing_and_loading_a_preset_through_a_real_library() {
        let dir = std::env::temp_dir().join(format!("anigo-presets-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        let mut m = (BUILT_IN[0].build)();
        m.name = "Minha pele".into();
        let id = save_preset(&lib, "Minha pele", &m).unwrap();
        let rows = list_presets(&lib);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].name, "Minha pele");
        let loaded = load_preset(&lib, &id).unwrap();
        assert_eq!(loaded, m);
        // salvar o mesmo conteúdo de novo não duplica
        let again = save_preset(&lib, "Minha pele", &m).unwrap();
        assert_eq!(again, id);
        assert_eq!(list_presets(&lib).len(), 1);
        assert!(save_preset(&lib, "  ", &m).is_err());
        drop(lib);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
