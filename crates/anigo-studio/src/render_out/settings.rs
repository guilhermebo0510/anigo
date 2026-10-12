//! Configurações puras da saída de render: presets de resolução, bloqueio de aspecto, plano de render
//! (supersampling limitado pela GPU/memória) e nomes de arquivo por template com contador.
//! Sem egui, sem GPU: tudo testável.

use anigo_core::RenderOutput;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------------------------------------
// Presets
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetGroup {
    Screen,
    Print,
    Comic,
    Illustration,
}

impl PresetGroup {
    pub const ALL: [PresetGroup; 4] = [PresetGroup::Screen, PresetGroup::Print, PresetGroup::Comic, PresetGroup::Illustration];

    pub fn key(self) -> &'static str {
        match self {
            PresetGroup::Screen => "render.preset_group.screen",
            PresetGroup::Print => "render.preset_group.print",
            PresetGroup::Comic => "render.preset_group.comic",
            PresetGroup::Illustration => "render.preset_group.illustration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Chave de i18n do nome.
    pub key: &'static str,
    pub group: PresetGroup,
    pub w: u32,
    pub h: u32,
    pub dpi: u32,
}

const fn preset(key: &'static str, group: PresetGroup, w: u32, h: u32, dpi: u32) -> Preset {
    Preset { key, group, w, h, dpi }
}

/// Tamanhos em pixels. Impressão: `mm / 25,4 × dpi` arredondado (A4 = 210×297 mm → 2480×3508 a 300 dpi).
pub const PRESETS: &[Preset] = &[
    preset("render.preset.test", PresetGroup::Screen, 640, 360, 72),
    preset("render.preset.hd720", PresetGroup::Screen, 1280, 720, 72),
    preset("render.preset.fhd", PresetGroup::Screen, 1920, 1080, 72),
    preset("render.preset.qhd", PresetGroup::Screen, 2560, 1440, 72),
    preset("render.preset.uhd4k", PresetGroup::Screen, 3840, 2160, 72),
    preset("render.preset.square", PresetGroup::Screen, 1080, 1080, 72),
    preset("render.preset.portrait45", PresetGroup::Screen, 1080, 1350, 72),
    preset("render.preset.vertical916", PresetGroup::Screen, 1080, 1920, 72),
    preset("render.preset.a4_portrait", PresetGroup::Print, 2480, 3508, 300),
    preset("render.preset.a4_landscape", PresetGroup::Print, 3508, 2480, 300),
    preset("render.preset.a5_portrait", PresetGroup::Print, 1748, 2480, 300),
    preset("render.preset.b5_portrait", PresetGroup::Print, 2079, 2953, 300),
    preset("render.preset.a3_portrait", PresetGroup::Print, 3508, 4961, 300),
    preset("render.preset.panel43", PresetGroup::Comic, 1600, 1200, 150),
    preset("render.preset.panel_wide", PresetGroup::Comic, 2400, 1000, 150),
    preset("render.preset.panel_square", PresetGroup::Comic, 1200, 1200, 150),
    preset("render.preset.webtoon", PresetGroup::Comic, 800, 1280, 72),
    preset("render.preset.illus_square", PresetGroup::Illustration, 3000, 3000, 300),
    preset("render.preset.illus_poster", PresetGroup::Illustration, 2400, 3600, 300),
    preset("render.preset.illus_wide", PresetGroup::Illustration, 3840, 2160, 300),
];

/// Primeiro preset cujo tamanho coincide com `(w, h)` (para rotular o combo).
pub fn preset_for(w: u32, h: u32) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.w == w && p.h == h)
}

// ------------------------------------------------------------------------------------------------
// Aspecto
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Width,
    Height,
}

/// Aplica uma edição de `axis` mantendo a proporção `aspect` (largura/altura). Devolve `(w, h)` dentro de `[min, max]`.
pub fn lock_aspect(axis: Axis, value: u32, aspect: f32, min: u32, max: u32) -> (u32, u32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let v = value.clamp(min, max);
    let (mut w, mut h) = match axis {
        Axis::Width => (v, (v as f32 / aspect).round() as u32),
        Axis::Height => ((v as f32 * aspect).round() as u32, v),
    };
    if w > max || h > max || w < min || h < min {
        // a outra dimensão estourou o limite: ela fica no limite e a editada recua para continuar proporcional
        match axis {
            Axis::Width => {
                h = h.clamp(min, max);
                w = ((h as f32 * aspect).round() as u32).clamp(min, max);
            }
            Axis::Height => {
                w = w.clamp(min, max);
                h = ((w as f32 / aspect).round() as u32).clamp(min, max);
            }
        }
    }
    (w, h)
}

/// Razão simplificada para exibir ("16:9"); proporções sem razão pequena viram `0.71:1`.
pub fn aspect_label(w: u32, h: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(w.max(1), h.max(1)).max(1);
    let (a, b) = (w / g, h / g);
    if a <= 32 && b <= 32 {
        format!("{a}:{b}")
    } else {
        format!("{:.2}:1", w as f32 / h.max(1) as f32)
    }
}

// ------------------------------------------------------------------------------------------------
// Plano de render
// ------------------------------------------------------------------------------------------------

/// Orçamento padrão de pixels do alvo de render (já com supersampling): ~48 MP.
pub const DEFAULT_MAX_PIXELS: u64 = 48_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPlan {
    /// Tamanho do arquivo final.
    pub target: (u32, u32),
    /// Supersampling pedido e o efetivo (limitado pela textura máxima / orçamento de pixels).
    pub ss_requested: u32,
    pub ss: u32,
    /// Tamanho do alvo de render (`target × ss`).
    pub render_size: (u32, u32),
}

impl RenderPlan {
    pub fn clamped(&self) -> bool {
        self.ss < self.ss_requested
    }
}

/// Resolve o tamanho do arquivo e o supersampling que cabe em `max_dim` (lado) e `max_pixels`.
pub fn plan(output: &RenderOutput, max_dim: u32, max_pixels: u64) -> Result<RenderPlan, String> {
    let (tw, th) = output.target_size();
    if tw > max_dim || th > max_dim {
        return Err(format!("{tw}×{th} excede o limite de textura da GPU ({max_dim} px por lado)"));
    }
    if tw as u64 * th as u64 > max_pixels {
        return Err(format!("{tw}×{th} excede o orçamento de {} MP", max_pixels / 1_000_000));
    }
    let requested = output.supersample.clamp(1, 4);
    let mut ss = requested;
    while ss > 1 && (tw * ss > max_dim || th * ss > max_dim || (tw as u64 * ss as u64) * (th as u64 * ss as u64) > max_pixels) {
        ss -= 1;
    }
    Ok(RenderPlan { target: (tw, th), ss_requested: requested, ss, render_size: (tw * ss, th * ss) })
}

// ------------------------------------------------------------------------------------------------
// Nomes de arquivo
// ------------------------------------------------------------------------------------------------

/// Template padrão: `{project}_{n:04}`.
pub const DEFAULT_TEMPLATE: &str = "{project}_{n:04}";

/// Valores dos tokens do template.
#[derive(Debug, Clone, Default)]
pub struct NameTokens {
    pub project: String,
    pub camera: String,
    pub w: u32,
    pub h: u32,
    pub fmt: String,
    pub pass: String,
    /// Contador (render avulso).
    pub n: u32,
    /// Número do quadro (sequência da Animação); `None` em render avulso.
    pub frame: Option<u32>,
    pub date: String,
    pub time: String,
}

/// Expande `{project} {camera} {w} {h} {fmt} {pass} {n} {frame} {date} {time}`; `{n:04}`/`{frame:04}` preenchem com zeros;
/// `%04d` (estilo printf/Blender) vale `{frame:04}` quando há quadro e `{n:04}` caso contrário.
pub fn expand_template(template: &str, t: &NameTokens) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                if let Some(end) = chars[i + 1..].iter().position(|&c| c == '}') {
                    let body: String = chars[i + 1..i + 1 + end].iter().collect();
                    out.push_str(&token_value(&body, t));
                    i += end + 2;
                    continue;
                }
                out.push('{');
            }
            '%' => {
                let mut j = i + 1;
                let mut width = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    width.push(chars[j]);
                    j += 1;
                }
                if j < chars.len() && chars[j] == 'd' {
                    out.push_str(&pad(t.frame.unwrap_or(t.n), &width));
                    i = j + 1;
                    continue;
                }
                out.push('%');
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn pad(v: u32, width: &str) -> String {
    let w = width.parse::<usize>().unwrap_or(0).min(9);
    format!("{v:0w$}")
}

fn token_value(body: &str, t: &NameTokens) -> String {
    let (name, spec) = match body.split_once(':') {
        Some((n, s)) => (n.trim(), s.trim()),
        None => (body.trim(), ""),
    };
    match name {
        "project" => t.project.clone(),
        "camera" => t.camera.clone(),
        "w" => t.w.to_string(),
        "h" => t.h.to_string(),
        "fmt" => t.fmt.clone(),
        "pass" => t.pass.clone(),
        "date" => t.date.clone(),
        "time" => t.time.clone(),
        "n" => pad(t.n, spec),
        "frame" => pad(t.frame.unwrap_or(0), spec),
        other => format!("_{other}_"),
    }
}

fn has_printf(template: &str) -> bool {
    let b = template.as_bytes();
    (0..b.len()).any(|i| {
        b[i] == b'%' && {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            j < b.len() && b[j] == b'd'
        }
    })
}

/// `true` se o template tem contador (`{n}` ou `%d`).
pub fn template_has_counter(template: &str) -> bool {
    template.contains("{n}") || template.contains("{n:") || has_printf(template)
}

/// Nome de arquivo seguro (sem separadores, caracteres reservados do Windows nem pontos/espaços nas pontas).
pub fn sanitize_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '{' | '}' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == ' ');
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "render".to_string();
    }
    let upper = out.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")) && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.push('_');
    }
    out
}

/// Arquivos de uma saída: imagem principal + um por AOV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    /// Nome-base (sem extensão) já com o contador resolvido.
    pub stem: String,
    pub n: u32,
    pub main: PathBuf,
    pub aovs: Vec<(String, PathBuf)>,
}

/// Aloca os nomes. Sem `overwrite`, avança o contador até que NENHUM arquivo do conjunto exista (se o template não tem
/// contador, acrescenta `_NNNN` a partir da primeira colisão). Em sequência (`tokens.frame`), o nome é função do quadro:
/// quem chama decide pular quadros que já existem (retomar).
pub fn allocate(dir: &Path, template: &str, tokens: &NameTokens, ext: &str, aovs: &[&str], overwrite: bool, start_n: u32) -> Allocation {
    let has_counter = template_has_counter(template);
    let start = start_n.max(1);
    let mut n = start;
    loop {
        let mut t = tokens.clone();
        t.n = n;
        t.pass = "beauty".into();
        let mut stem = sanitize_file_name(&expand_template(template, &t));
        if !has_counter && t.frame.is_none() && n > start {
            stem = format!("{stem}_{n:04}");
        }
        let main = dir.join(format!("{stem}.{ext}"));
        let aov_paths: Vec<(String, PathBuf)> = aovs.iter().map(|a| (a.to_string(), dir.join(format!("{stem}_{a}.png")))).collect();
        let exists = main.exists() || aov_paths.iter().any(|(_, p)| p.exists());
        if overwrite || tokens.frame.is_some() || !exists || n > start.saturating_add(100_000) {
            return Allocation { stem, n, main, aovs: aov_paths };
        }
        n += 1;
    }
}

/// Nomes de um quadro de sequência: o nome é função do quadro (`{frame}`/`%04d`); sem token de quadro no template,
/// acrescenta `_{frame:04}`. Nunca consulta o disco (quem chama decide pular os que já existem).
pub fn allocate_frame(dir: &Path, template: &str, tokens: &NameTokens, ext: &str, aovs: &[&str]) -> Allocation {
    let frame = tokens.frame.unwrap_or(0);
    let mut t = tokens.clone();
    t.pass = "beauty".into();
    t.n = frame;
    let mut stem = sanitize_file_name(&expand_template(template, &t));
    if !(template.contains("{frame") || has_printf(template)) {
        stem = format!("{stem}_{frame:04}");
    }
    let main = dir.join(format!("{stem}.{ext}"));
    let aov_paths = aovs.iter().map(|a| (a.to_string(), dir.join(format!("{stem}_{a}.png")))).collect();
    Allocation { stem, n: frame, main, aovs: aov_paths }
}

/// Data/hora UTC (aaaa-mm-dd, hh-mm-ss) a partir de segundos Unix — sem dependências de calendário.
pub fn utc_date_time(unix_secs: u64) -> (String, String) {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}-{:02}-{:02}", rem / 3600, (rem % 3600) / 60, rem % 60))
}

/// Pasta de saída sugerida: `Documents/ANIGO/Renders` (`ANIGO_RENDER_DIR` sobrescreve).
pub fn suggested_output_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANIGO_RENDER_DIR") {
        return PathBuf::from(dir);
    }
    crate::library::studio_data_dir().join("Renders")
}

/// Garante que `dir` existe e é gravável (cria e escreve/remove um arquivo de teste).
pub fn check_output_dir(dir: &Path) -> Result<(), String> {
    if dir.as_os_str().is_empty() {
        return Err("caminho vazio".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let probe = dir.join(format!(".anigo_write_test_{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(|e| format!("{}: sem permissão de escrita ({e})", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> NameTokens {
        NameTokens { project: "Meu Projeto".into(), camera: "Cam 1".into(), w: 1920, h: 1080, fmt: "png".into(), date: "2026-10-12".into(), time: "10-30-00".into(), ..Default::default() }
    }

    #[test]
    fn presets_use_the_print_math_and_are_unique() {
        let a4 = PRESETS.iter().find(|p| p.key.ends_with("a4_portrait")).unwrap();
        assert_eq!((a4.w, a4.h, a4.dpi), (2480, 3508, 300));
        for p in PRESETS.iter().filter(|p| p.group == PresetGroup::Print) {
            assert_eq!(p.dpi, 300);
        }
        let px = |mm: f32| (mm / 25.4 * 300.0).round() as u32;
        assert_eq!((px(210.0), px(297.0)), (2480, 3508));
        assert_eq!((px(148.0), px(210.0)), (1748, 2480));
        assert_eq!((px(176.0), px(250.0)), (2079, 2953));
        assert_eq!((px(297.0), px(420.0)), (3508, 4961));
        let mut keys: Vec<_> = PRESETS.iter().map(|p| p.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRESETS.len());
        assert_eq!(preset_for(1920, 1080).map(|p| p.key), Some("render.preset.fhd"));
        for p in PRESETS {
            assert!(p.w >= 16 && p.h >= 16 && p.w <= 16384 && p.h <= 16384);
        }
    }

    #[test]
    fn aspect_lock_keeps_proportion_and_respects_limits() {
        assert_eq!(lock_aspect(Axis::Width, 1280, 16.0 / 9.0, 16, 16384), (1280, 720));
        assert_eq!(lock_aspect(Axis::Height, 1350, 0.8, 16, 16384), (1080, 1350));
        let (w, h) = lock_aspect(Axis::Width, 16384, 0.5, 16, 16384);
        assert_eq!((w, h), (8192, 16384), "a altura estourou: fica no limite e a largura acompanha");
        assert_eq!(lock_aspect(Axis::Width, 2, 1.0, 16, 100), (16, 16));
        assert_eq!(aspect_label(1920, 1080), "16:9");
        assert_eq!(aspect_label(2480, 3508), "0.71:1");
        assert_eq!(aspect_label(1080, 1350), "4:5");
    }

    #[test]
    fn plan_clamps_supersampling_to_the_gpu_limits() {
        let hd = RenderOutput { supersample: 4, ..Default::default() };
        let p = plan(&hd, 8192, DEFAULT_MAX_PIXELS).unwrap();
        assert_eq!((p.ss, p.render_size, p.clamped()), (4, (7680, 4320), false));
        let uhd = RenderOutput { width: 3840, height: 2160, supersample: 3, ..Default::default() };
        let p = plan(&uhd, 8192, DEFAULT_MAX_PIXELS).unwrap();
        assert_eq!(p.ss, 2, "3×4K = 11520 px passa de 8192");
        assert!(p.clamped());
        let big = RenderOutput { width: 9000, height: 3000, ..Default::default() };
        assert!(plan(&big, 8192, DEFAULT_MAX_PIXELS).is_err());
        let half = RenderOutput { scale_percent: 50, supersample: 2, ..Default::default() };
        assert_eq!(plan(&half, 8192, DEFAULT_MAX_PIXELS).unwrap().render_size, (1920, 1080));
    }

    #[test]
    fn templates_expand_tokens_counters_and_printf() {
        let mut t = tokens();
        t.n = 7;
        assert_eq!(expand_template("{project}_{n:04}", &t), "Meu Projeto_0007");
        assert_eq!(expand_template("{camera}-{w}x{h}.{fmt}", &t), "Cam 1-1920x1080.png");
        assert_eq!(expand_template("shot_%04d", &t), "shot_0007");
        t.frame = Some(12);
        assert_eq!(expand_template("shot_%04d_{frame:03}", &t), "shot_0012_012");
        assert_eq!(expand_template("{date} {time}", &t), "2026-10-12 10-30-00");
        assert!(template_has_counter("a_{n:04}") && template_has_counter("a_%04d") && template_has_counter("{n}"));
        assert!(!template_has_counter("a_{project}") && !template_has_counter("100%"));
    }

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(sanitize_file_name("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_file_name("  ..  "), "render");
        assert_eq!(sanitize_file_name("CON"), "CON_");
        assert_eq!(sanitize_file_name("{x}"), "_x_");
        assert!(sanitize_file_name(&"x".repeat(500)).chars().count() <= 121);
    }

    #[test]
    fn allocation_advances_the_counter_instead_of_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let t = tokens();
        let a = allocate(dir.path(), "{project}_{n:04}", &t, "png", &["outline"], false, 1);
        assert_eq!(a.main.file_name().unwrap().to_string_lossy(), "Meu Projeto_0001.png");
        assert_eq!(a.aovs[0].1.file_name().unwrap().to_string_lossy(), "Meu Projeto_0001_outline.png");
        std::fs::write(&a.main, b"x").unwrap();
        let b = allocate(dir.path(), "{project}_{n:04}", &t, "png", &["outline"], false, 1);
        assert_eq!(b.n, 2);
        std::fs::write(&b.aovs[0].1, b"x").unwrap();
        let c = allocate(dir.path(), "{project}_{n:04}", &t, "png", &["outline"], false, 2);
        assert_eq!(c.n, 3, "um AOV existente também bloqueia o nome");
        let d = allocate(dir.path(), "{project}_{n:04}", &t, "png", &[], true, 1);
        assert_eq!(d.n, 1, "sobrescrever mantém o nome");
        std::fs::write(dir.path().join("fixo.png"), b"x").unwrap();
        let e = allocate(dir.path(), "fixo", &t, "png", &[], false, 1);
        assert_eq!(e.main.file_name().unwrap().to_string_lossy(), "fixo_0002.png");
        let mut tf = t.clone();
        tf.frame = Some(5);
        std::fs::write(dir.path().join("seq_0005.png"), b"x").unwrap();
        let f = allocate(dir.path(), "seq_%04d", &tf, "png", &[], false, 1);
        assert_eq!(f.main.file_name().unwrap().to_string_lossy(), "seq_0005.png");
    }

    #[test]
    fn sequence_frames_get_deterministic_names() {
        let dir = Path::new("/tmp/x");
        let mut t = tokens();
        t.frame = Some(7);
        assert_eq!(allocate_frame(dir, "cena_%04d", &t, "png", &[]).main, dir.join("cena_0007.png"));
        assert_eq!(allocate_frame(dir, "cena_{frame:03}", &t, "png", &["depth"]).aovs[0].1, dir.join("cena_007_depth.png"));
        assert_eq!(allocate_frame(dir, "cena", &t, "png", &[]).main, dir.join("cena_0007.png"));
    }

    #[test]
    fn utc_date_time_matches_known_instants() {
        assert_eq!(utc_date_time(0), ("1970-01-01".into(), "00-00-00".into()));
        assert_eq!(utc_date_time(1_791_800_000).0, "2026-10-12");
        assert_eq!(utc_date_time(951_782_400).0, "2000-02-29");
    }

    #[test]
    fn output_dir_check_creates_and_probes() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a").join("b");
        assert!(check_output_dir(&sub).is_ok());
        assert!(sub.is_dir());
        assert!(check_output_dir(Path::new("")).is_err());
    }
}
