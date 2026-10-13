//! Tabela de parâmetros do material toon editável no workspace Shading.
//!
//! Cada [`Param`] liga um controle da UI a UM campo real do `StylizedMaterial` (o material que o
//! renderer cel canônico consome): grupo, faixa, valor padrão (o do `StylizedMaterial::default`),
//! dependência de outro parâmetro (`gate`) e leitura/escrita. É a única fonte da verdade do painel
//! (grupos recolhíveis, reset por parâmetro/grupo, indicador "modificado"), do comando de patch e dos
//! testes que provam que cada controle muda a imagem.

use anigo_core::StylizedMaterial;

/// Valor de um parâmetro.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    F(f32),
    /// RGBA 0..1 (a UI edita o RGB; o alfa acompanha).
    C([f32; 4]),
    B(bool),
    /// Índice de uma lista de opções.
    I(u8),
}

impl Value {
    pub fn f(self) -> f32 {
        match self {
            Value::F(v) => v,
            Value::B(b) => b as u8 as f32,
            Value::I(i) => i as f32,
            Value::C(c) => c[0],
        }
    }
    pub fn c(self) -> [f32; 4] {
        match self {
            Value::C(c) => c,
            _ => [0.0, 0.0, 0.0, 1.0],
        }
    }
    pub fn b(self) -> bool {
        match self {
            Value::B(b) => b,
            Value::F(v) => v > 0.0,
            Value::I(i) => i > 0,
            Value::C(_) => true,
        }
    }
    pub fn i(self) -> u8 {
        match self {
            Value::I(i) => i,
            Value::B(b) => b as u8,
            Value::F(v) => v.max(0.0) as u8,
            Value::C(_) => 0,
        }
    }
    /// Igualdade tolerante (floats/cores a 1e-4).
    pub fn same(self, other: Value) -> bool {
        match (self, other) {
            (Value::F(a), Value::F(b)) => (a - b).abs() < 1e-4,
            (Value::C(a), Value::C(b)) => a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-4),
            (a, b) => a == b,
        }
    }
}

/// Grupos do painel (mesma ordem da UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Base,
    Ramp,
    Specular,
    Rim,
    Matcap,
    Outline,
    Face,
    Emission,
    Eye,
}

impl Group {
    pub const ALL: [Group; 9] = [Group::Base, Group::Ramp, Group::Specular, Group::Rim, Group::Matcap, Group::Outline, Group::Face, Group::Emission, Group::Eye];

    /// Chave de i18n do título.
    pub fn label_key(self) -> &'static str {
        match self {
            Group::Base => "shading.g.base",
            Group::Ramp => "shading.g.ramp",
            Group::Specular => "shading.g.specular",
            Group::Rim => "shading.g.rim",
            Group::Matcap => "shading.g.matcap",
            Group::Outline => "shading.g.outline",
            Group::Face => "shading.g.face",
            Group::Emission => "shading.g.emission",
            Group::Eye => "shading.g.eye",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Group::Base => "base",
            Group::Ramp => "ramp",
            Group::Specular => "specular",
            Group::Rim => "rim",
            Group::Matcap => "matcap",
            Group::Outline => "outline",
            Group::Face => "face",
            Group::Emission => "emission",
            Group::Eye => "eye",
        }
    }
}

/// Forma do controle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Float { min: f32, max: f32, decimals: usize },
    Color,
    Bool,
    /// Lista de rótulos (chaves de i18n).
    Choice(&'static [&'static str]),
}

/// Um parâmetro editável.
#[derive(Clone, Copy)]
pub struct Param {
    /// Id estável (também a chave de i18n: `shading.p.<id>`).
    pub id: &'static str,
    pub group: Group,
    pub kind: Kind,
    /// Chave de i18n do rótulo.
    pub label: &'static str,
    /// Id do parâmetro (bool ou float > 0) que precisa estar "ligado" para este ter efeito.
    pub gate: Option<&'static str>,
    pub get: fn(&StylizedMaterial) -> Value,
    pub set: fn(&mut StylizedMaterial, Value),
}

impl std::fmt::Debug for Param {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Param({})", self.id)
    }
}

impl Param {
    pub fn default_value(&self) -> Value {
        (self.get)(&StylizedMaterial::default())
    }

    pub fn is_modified(&self, m: &StylizedMaterial) -> bool {
        !(self.get)(m).same(self.default_value())
    }

    /// Limita o valor à faixa do controle (floats; cores 0..1).
    pub fn clamp(&self, v: Value) -> Value {
        match (self.kind, v) {
            (Kind::Float { min, max, .. }, Value::F(x)) => Value::F(if x.is_finite() { x.clamp(min, max) } else { self.default_value().f() }),
            (Kind::Color, Value::C(c)) => Value::C([c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), c[3].clamp(0.0, 1.0)]),
            (Kind::Choice(opts), Value::I(i)) => Value::I(i.min(opts.len().saturating_sub(1) as u8)),
            (_, v) => v,
        }
    }

    pub fn write(&self, m: &mut StylizedMaterial, v: Value) {
        (self.set)(m, self.clamp(v));
    }

    /// O gate (se houver) está ligado neste material?
    pub fn gate_on(&self, m: &StylizedMaterial) -> bool {
        match self.gate.and_then(find) {
            Some(g) => (g.get)(m).f() > 0.0 || (g.get)(m).b(),
            None => true,
        }
    }
}

macro_rules! float {
    ($id:literal, $g:ident, $min:expr, $max:expr, $dec:expr, $gate:expr, $($f:ident).+) => {
        Param {
            id: $id,
            group: Group::$g,
            kind: Kind::Float { min: $min, max: $max, decimals: $dec },
            label: concat!("shading.p.", $id),
            gate: $gate,
            get: |m| Value::F(m.$($f).+),
            set: |m, v| m.$($f).+ = v.f(),
        }
    };
}
macro_rules! color {
    ($id:literal, $g:ident, $gate:expr, $f:ident) => {
        Param {
            id: $id,
            group: Group::$g,
            kind: Kind::Color,
            label: concat!("shading.p.", $id),
            gate: $gate,
            get: |m| Value::C(m.$f),
            set: |m, v| {
                let c = v.c();
                m.$f = [c[0], c[1], c[2], m.$f[3]];
            },
        }
    };
}
macro_rules! boolean {
    ($id:literal, $g:ident, $gate:expr, $f:ident) => {
        Param {
            id: $id,
            group: Group::$g,
            kind: Kind::Bool,
            label: concat!("shading.p.", $id),
            gate: $gate,
            get: |m| Value::B(m.$f),
            set: |m, v| m.$f = v.b(),
        }
    };
}

/// Modos do matcap (`mtoon_matcap_mode`).
pub const MATCAP_MODES: &[&str] = &["shading.matcap.multiply", "shading.matcap.additive"];

/// Todos os parâmetros, na ordem de exibição dentro de cada grupo.
pub const PARAMS: &[Param] = &[
    // ── Base / Sombra ────────────────────────────────────────────────────────
    color!("base_color", Base, None, base_color),
    color!("shade_color", Base, None, shade_color),
    float!("shadow_threshold", Base, 0.0, 1.0, 2, None, shadow_threshold),
    float!("shadow_smoothness", Base, 0.0, 0.5, 3, None, shadow_smoothness),
    float!("toon_steps", Base, 0.0, 3.0, 2, None, toon_steps),
    float!("hue_shift", Base, -180.0, 180.0, 0, None, hue_shift),
    float!("ao_intensity", Base, 0.0, 1.0, 2, None, ao_intensity),
    boolean!("mtoon_shade_toony", Base, None, mtoon_shade_toony),
    float!("mtoon_second_shade_shift", Base, -1.0, 1.0, 2, None, mtoon_second_shade_shift),
    float!("mtoon_second_shade_softness", Base, 0.0, 1.0, 2, None, mtoon_second_shade_softness),
    // ── Rampa 2D ─────────────────────────────────────────────────────────────
    float!("ramp_blend", Ramp, 0.0, 1.0, 2, None, npr.ramp_blend),
    float!("ramp_row", Ramp, 0.0, 3.0, 2, Some("ramp_blend"), npr.ramp_row),
    // ── Especular ────────────────────────────────────────────────────────────
    float!("spec_intensity", Specular, 0.0, 2.0, 2, None, spec_intensity),
    color!("specular_color", Specular, Some("spec_intensity"), specular_color),
    float!("spec_power", Specular, 4.0, 128.0, 0, Some("spec_intensity"), spec_power),
    float!("specular_size", Specular, 0.0, 1.0, 2, Some("spec_intensity"), specular_size),
    float!("specular_softness", Specular, 0.0, 1.0, 2, Some("spec_intensity"), specular_softness),
    float!("specular_offset", Specular, -1.0, 1.0, 2, Some("spec_intensity"), specular_offset),
    // ── Rim ─────────────────────────────────────────────────────────────────
    float!("rim_intensity", Rim, 0.0, 3.0, 2, None, rim_intensity),
    float!("rim_spread", Rim, 0.05, 1.0, 2, Some("rim_intensity"), rim_spread),
    color!("rim_color", Rim, Some("rim_intensity"), rim_color),
    // ── Matcap ──────────────────────────────────────────────────────────────
    boolean!("mtoon_matcap_enabled", Matcap, None, mtoon_matcap_enabled),
    Param {
        id: "mtoon_matcap_mode",
        group: Group::Matcap,
        kind: Kind::Choice(MATCAP_MODES),
        label: "shading.p.mtoon_matcap_mode",
        gate: Some("mtoon_matcap_enabled"),
        get: |m| Value::I(m.mtoon_matcap_mode),
        set: |m, v| m.mtoon_matcap_mode = v.i(),
    },
    float!("mtoon_matcap_intensity", Matcap, 0.0, 2.0, 2, Some("mtoon_matcap_enabled"), mtoon_matcap_intensity),
    // ── Contorno (por material) ─────────────────────────────────────────────
    float!("outline_width", Outline, 0.0, 0.02, 4, None, outline_width),
    color!("outline_color", Outline, Some("outline_width"), outline_color),
    float!("outline_opacity", Outline, 0.0, 1.0, 2, Some("outline_width"), outline_opacity),
    float!("outline_smoothness", Outline, 0.0, 1.0, 2, Some("outline_width"), outline_smoothness),
    float!("outline_depth_bias", Outline, 0.0, 0.01, 4, Some("outline_width"), outline_depth_bias),
    float!("outline_smooth_normals", Outline, 0.0, 1.0, 2, Some("outline_width"), npr.outline_smooth_normals),
    float!("outline_distance_falloff", Outline, 0.0, 1.0, 2, Some("outline_width"), npr.outline_distance_falloff),
    float!("outline_ref_distance", Outline, 0.05, 50.0, 2, Some("outline_distance_falloff"), npr.outline_ref_distance),
    float!("outline_min_width_scale", Outline, 0.0, 1.0, 2, Some("outline_distance_falloff"), npr.outline_min_width_scale),
    float!("outline_taper", Outline, 0.0, 1.0, 2, Some("outline_width"), npr.outline_taper),
    float!("outline_ink_blend", Outline, 0.0, 1.0, 2, Some("outline_width"), npr.outline_ink_blend),
    float!("outline_ink_darken", Outline, 0.0, 1.0, 2, Some("outline_ink_blend"), npr.outline_ink_darken),
    float!("outline_ink_saturation", Outline, 0.0, 3.0, 2, Some("outline_ink_blend"), npr.outline_ink_saturation),
    // ── Face SDF ────────────────────────────────────────────────────────────
    boolean!("face_sdf_enabled", Face, None, face_sdf_enabled),
    float!("face_shadow_offset", Face, -0.25, 0.25, 3, Some("face_sdf_enabled"), face_shadow_offset),
    float!("face_shadow_smoothness", Face, 0.0, 0.3, 3, Some("face_sdf_enabled"), face_shadow_smoothness),
    float!("face_sdf_mirror_center", Face, 0.0, 1.0, 2, Some("face_sdf_enabled"), npr.face_sdf_mirror_center),
    // ── Emissão ─────────────────────────────────────────────────────────────
    // A emissão do renderer só soma quando o slot de emissão está ligado (o neutro 1×1 branco faz o papel
    // de "sem textura"): ligar a intensidade liga o slot, zerar desliga.
    Param {
        id: "mtoon_emission_intensity",
        group: Group::Emission,
        kind: Kind::Float { min: 0.0, max: 4.0, decimals: 2 },
        label: "shading.p.mtoon_emission_intensity",
        gate: None,
        get: |m| Value::F(m.mtoon_emission_intensity),
        set: |m, v| {
            m.mtoon_emission_intensity = v.f();
            m.mtoon_emission_texture_enabled = v.f() > 0.0;
        },
    },
    color!("mtoon_emission_color", Emission, Some("mtoon_emission_intensity"), mtoon_emission_color),
    // ── Olho anime ──────────────────────────────────────────────────────────
    boolean!("eye_enabled", Eye, None, eye_enabled),
    float!("eye_depth_scale", Eye, 0.0, 0.2, 3, Some("eye_enabled"), eye_depth_scale),
    float!("eye_highlight_intensity", Eye, 0.0, 2.0, 2, Some("eye_enabled"), eye_highlight_intensity),
];

/// Parâmetro por id.
pub fn find(id: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.id == id)
}

/// Parâmetros de um grupo, em ordem.
pub fn of_group(g: Group) -> impl Iterator<Item = &'static Param> {
    PARAMS.iter().filter(move |p| p.group == g)
}

/// Quantos parâmetros do grupo diferem do padrão.
pub fn modified_in_group(m: &StylizedMaterial, g: Group) -> usize {
    of_group(g).filter(|p| p.is_modified(m)).count()
}

/// Gradiente próprio da rampa conta como modificação do grupo Rampa.
pub fn group_modified_count(m: &StylizedMaterial, g: Group) -> usize {
    let mut n = modified_in_group(m, g);
    if g == Group::Ramp && !m.npr.ramp_custom.is_default() {
        n += 1;
    }
    n
}

/// Devolve o grupo ao padrão (mantém tudo que não é dele).
pub fn reset_group(m: &mut StylizedMaterial, g: Group) {
    for p in of_group(g) {
        (p.set)(m, p.default_value());
    }
    if g == Group::Ramp {
        m.npr.ramp_custom = anigo_core::RampCustom::DEFAULT;
    }
}

/// Devolve um parâmetro ao padrão.
pub fn reset_param(m: &mut StylizedMaterial, id: &str) {
    if let Some(p) = find(id) {
        (p.set)(m, p.default_value());
    }
}

/// Material com os recursos de Fase 1 do cel LIGADOS num visual "anime" (usado pelo botão
/// "Ligar recursos anime" e pelos testes): rampa 2D, contorno com normais suavizadas, falloff,
/// taper e tinta por material.
pub fn enable_anime_features(m: &mut StylizedMaterial) {
    m.outline_width = m.outline_width.max(0.004);
    m.npr.outline_smooth_normals = 1.0;
    m.npr.outline_distance_falloff = 0.8;
    m.npr.outline_taper = 0.5;
    m.npr.outline_ink_blend = 0.7;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_gates_exist() {
        let mut seen = HashSet::new();
        for p in PARAMS {
            assert!(seen.insert(p.id), "id repetido {}", p.id);
            if let Some(g) = p.gate {
                assert!(find(g).is_some(), "gate {g} de {} não existe", p.id);
                assert_ne!(g, p.id);
            }
            assert!(p.label.starts_with("shading.p."));
        }
    }

    #[test]
    fn every_param_reads_what_it_writes_and_defaults_match_the_core_default() {
        let d = StylizedMaterial::default();
        for p in PARAMS {
            let dv = (p.get)(&d);
            assert!(!p.is_modified(&d), "{} deveria ser o padrão", p.id);
            // escreve um valor diferente e relê
            let alt = alt_value(p);
            let mut m = d.clone();
            p.write(&mut m, alt);
            let back = (p.get)(&m);
            assert!(back.same(p.clamp(alt)), "{}: escreveu {alt:?} e leu {back:?}", p.id);
            assert!(!back.same(dv), "{}: o valor alternativo igual ao padrão", p.id);
            assert!(p.is_modified(&m));
            // reset volta ao padrão e só mexe nesse campo
            let mut r = m.clone();
            reset_param(&mut r, p.id);
            assert_eq!(r, d, "reset de {} deixou resíduo", p.id);
        }
    }

    #[test]
    fn defaults_are_inside_the_declared_ranges() {
        let d = StylizedMaterial::default();
        for p in PARAMS {
            if let Kind::Float { min, max, .. } = p.kind {
                let v = (p.get)(&d).f();
                assert!(v >= min - 1e-6 && v <= max + 1e-6, "{}: padrão {v} fora de {min}..{max}", p.id);
            }
        }
    }

    #[test]
    fn clamp_repairs_nan_and_out_of_range() {
        let p = find("rim_intensity").unwrap();
        assert_eq!(p.clamp(Value::F(99.0)), Value::F(3.0));
        assert_eq!(p.clamp(Value::F(f32::NAN)), p.default_value());
        let c = find("base_color").unwrap();
        assert_eq!(c.clamp(Value::C([2.0, -1.0, 0.5, 1.0])), Value::C([1.0, 0.0, 0.5, 1.0]));
        let mut m = StylizedMaterial::default();
        c.write(&mut m, Value::C([0.1, 0.2, 0.3, 0.0]));
        assert_eq!(m.base_color[3], 1.0, "o alfa da cor é preservado");
    }

    #[test]
    fn reset_group_only_touches_its_group() {
        let mut m = StylizedMaterial::default();
        for p in PARAMS {
            p.write(&mut m, alt_value(p));
        }
        m.npr.ramp_custom.set_stops(1, true, &[(0.0, 0.0), (0.5, 1.0)]);
        let before = m.clone();
        reset_group(&mut m, Group::Rim);
        assert_eq!(group_modified_count(&m, Group::Rim), 0);
        assert!(group_modified_count(&m, Group::Outline) > 0);
        assert_eq!(m.rim_color, StylizedMaterial::default().rim_color);
        assert_eq!(m.outline_width, before.outline_width);
        reset_group(&mut m, Group::Ramp);
        assert!(m.npr.ramp_custom.is_default());
        assert_eq!(group_modified_count(&m, Group::Ramp), 0);
    }

    /// Um valor claramente diferente do padrão, dentro da faixa.
    pub(crate) fn alt_value(p: &Param) -> Value {
        match p.kind {
            Kind::Float { min, max, .. } => {
                let d = p.default_value().f();
                let hi = max - (max - min) * 0.2;
                let lo = min + (max - min) * 0.2;
                Value::F(if (d - hi).abs() > (max - min) * 0.25 { hi } else { lo })
            }
            Kind::Color => {
                let d = p.default_value().c();
                Value::C([1.0 - d[0], d[1] * 0.5, (d[2] + 0.5).min(1.0), d[3]])
            }
            Kind::Bool => Value::B(!p.default_value().b()),
            Kind::Choice(opts) => Value::I(if p.default_value().i() == 0 { (opts.len() - 1) as u8 } else { 0 }),
        }
    }
}

#[cfg(test)]
pub(crate) use tests::alt_value;
