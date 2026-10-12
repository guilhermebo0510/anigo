//! Atalhos configuráveis (T-2.12): combinações ([`Chord`]), registro de ações ([`actions`]), keymap do usuário
//! (SÓ O DELTA em relação ao padrão é persistido), detecção de conflitos por contexto, roteamento e import/export JSON.

pub mod actions;

use actions::{def, Active, ACTIONS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub use actions::{ActionDef, Area, Ctx};

/// Versão do esquema do `keymap.json`.
pub const KEYMAP_SCHEMA: u32 = 1;

/// Tecla + modificadores. `ctrl` = `COMMAND` do egui (Cmd no macOS, Ctrl nos demais).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub fn new(key: egui::Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: false }
    }

    pub fn modifiers(&self) -> egui::Modifiers {
        let mut m = egui::Modifiers::NONE;
        if self.ctrl {
            m |= egui::Modifiers::COMMAND;
        }
        if self.shift {
            m |= egui::Modifiers::SHIFT;
        }
        if self.alt {
            m |= egui::Modifiers::ALT;
        }
        m
    }

    /// `"Ctrl+Shift+Z"` — formato persistido (nomes de tecla do egui: `Z`, `1`, `Backtick`, `OpenBracket`, `Right`…).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.name());
        s
    }

    /// Texto para a UI (símbolos legíveis para pontuação).
    pub fn display(&self) -> String {
        let key = match self.key {
            egui::Key::OpenBracket => "[",
            egui::Key::CloseBracket => "]",
            egui::Key::Backtick => "`",
            egui::Key::Period => ".",
            egui::Key::Comma => ",",
            egui::Key::Minus => "-",
            egui::Key::Plus => "+",
            egui::Key::Equals => "=",
            egui::Key::Slash => "/",
            egui::Key::Backslash => "\\",
            egui::Key::Semicolon => ";",
            other => other.name(),
        };
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(key);
        s
    }

    /// Aceita `Ctrl`/`Cmd`/`Command`, `Alt`/`Option`, `Shift` em qualquer ordem e maiúsculas/minúsculas, e um nome de tecla do egui.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let mut c = Chord { key: egui::Key::A, ctrl: false, shift: false, alt: false };
        let mut key: Option<egui::Key> = None;
        for tok in text.split('+').map(str::trim) {
            if tok.is_empty() {
                return Err(format!("atalho vazio ou mal formado: '{text}'"));
            }
            match tok.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => c.ctrl = true,
                "shift" => c.shift = true,
                "alt" | "option" => c.alt = true,
                _ => {
                    if key.is_some() {
                        return Err(format!("mais de uma tecla em '{text}'"));
                    }
                    key = Some(egui::Key::from_name(tok).ok_or_else(|| format!("tecla desconhecida '{tok}' em '{text}'"))?);
                }
            }
        }
        c.key = key.ok_or_else(|| format!("falta a tecla em '{text}'"))?;
        // teclas que só são modificadores não servem de atalho
        if matches!(c.key, egui::Key::Copy | egui::Key::Cut | egui::Key::Paste) {
            return Err(format!("'{text}': Copiar/Recortar/Colar do sistema não podem ser reatribuídos"));
        }
        Ok(c)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("esquema de atalhos {0} é mais novo que o suportado ({KEYMAP_SCHEMA})")]
    FutureSchema(u64),
    #[error("o arquivo de atalhos não é um objeto JSON")]
    NotAnObject,
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

/// Dois atalhos iguais em ações distintas cujos contextos podem estar ativos juntos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub chord: Chord,
    pub a: &'static str,
    pub b: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct KeymapFile {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
}

/// Resultado de carregar/importar: o keymap e o que foi ignorado.
#[derive(Debug, Default)]
pub struct KeymapLoad {
    pub overrides: BTreeMap<String, Vec<Chord>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    /// Só o que o usuário mudou (lista vazia = ação desligada).
    overrides: BTreeMap<String, Vec<Chord>>,
    resolved: HashMap<&'static str, Vec<Chord>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_overrides(BTreeMap::new())
    }
}

fn default_chords(d: &ActionDef) -> Vec<Chord> {
    d.defaults.iter().map(|t| Chord::parse(t).unwrap_or_else(|e| panic!("atalho padrão inválido em {}: {e}", d.id))).collect()
}

impl Keymap {
    pub fn with_overrides(overrides: BTreeMap<String, Vec<Chord>>) -> Self {
        let mut km = Keymap { overrides, resolved: HashMap::new() };
        // ids desconhecidos (de versões futuras/antigas) são descartados
        km.overrides.retain(|id, _| def(id).is_some());
        km.rebuild();
        km
    }

    fn rebuild(&mut self) {
        self.resolved.clear();
        for d in ACTIONS {
            let chords = self.overrides.get(d.id).cloned().unwrap_or_else(|| default_chords(d));
            self.resolved.insert(d.id, chords);
        }
    }

    /// Atalhos em vigor da ação (vazio se desligada ou id desconhecido).
    pub fn chords(&self, id: &str) -> &[Chord] {
        match self.resolved.get(id) {
            Some(c) => c,
            None => {
                debug_assert!(false, "ação de atalho desconhecida: {id}");
                &[]
            }
        }
    }

    pub fn defaults_of(id: &str) -> Vec<Chord> {
        def(id).map(default_chords).unwrap_or_default()
    }

    pub fn is_modified(&self, id: &str) -> bool {
        self.overrides.contains_key(id)
    }

    pub fn modified_count(&self) -> usize {
        self.overrides.len()
    }

    /// Define os atalhos da ação; igual ao padrão = remove o override (o delta continua mínimo).
    pub fn set(&mut self, id: &str, chords: Vec<Chord>) {
        let Some(d) = def(id) else { return };
        let mut uniq: Vec<Chord> = Vec::new();
        for c in chords {
            if !uniq.contains(&c) {
                uniq.push(c);
            }
        }
        if uniq == default_chords(d) {
            self.overrides.remove(id);
        } else {
            self.overrides.insert(id.to_string(), uniq);
        }
        self.rebuild();
    }

    pub fn add(&mut self, id: &str, chord: Chord) {
        let mut cur = self.chords(id).to_vec();
        if !cur.contains(&chord) {
            cur.push(chord);
        }
        self.set(id, cur);
    }

    pub fn reset(&mut self, id: &str) {
        self.overrides.remove(id);
        self.rebuild();
    }

    pub fn reset_all(&mut self) {
        self.overrides.clear();
        self.rebuild();
    }

    /// Texto do atalho principal para menus/dicas (`""` se a ação está desligada).
    pub fn label(&self, id: &str) -> String {
        self.chords(id).first().map(|c| c.display()).unwrap_or_default()
    }

    // ------------------------------------------------------------------------------ conflitos e roteamento

    fn ctxs_overlap(a: &ActionDef, b: &ActionDef) -> bool {
        a.ctxs.iter().any(|x| b.ctxs.iter().any(|y| x.overlaps(y)))
    }

    /// Todos os conflitos no keymap em vigor (cada par uma vez).
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut out = Vec::new();
        for (i, a) in ACTIONS.iter().enumerate() {
            for b in &ACTIONS[i + 1..] {
                if !Self::ctxs_overlap(a, b) {
                    continue;
                }
                for c in self.chords(a.id) {
                    if self.chords(b.id).contains(c) {
                        out.push(Conflict { chord: *c, a: a.id, b: b.id });
                    }
                }
            }
        }
        out
    }

    /// Ações que conflitariam se `chord` fosse dado a `id` (para avisar ANTES/depois de capturar).
    pub fn conflicts_with(&self, id: &str, chord: &Chord) -> Vec<&'static str> {
        let Some(me) = def(id) else { return Vec::new() };
        ACTIONS.iter().filter(|o| o.id != id && Self::ctxs_overlap(me, o) && self.chords(o.id).contains(chord)).map(|o| o.id).collect()
    }

    /// Quais ações dispararia `chord` no contexto `active` (teste de roteamento; na prática 0 ou 1 sem conflitos).
    pub fn route(&self, chord: &Chord, active: &Active) -> Vec<&'static str> {
        ACTIONS.iter().filter(|d| d.ctxs.iter().any(|c| c.active_in(active)) && self.chords(d.id).contains(chord)).map(|d| d.id).collect()
    }

    // ------------------------------------------------------------------------------ egui

    /// Consome o primeiro atalho da ação que foi pressionado neste quadro (mesma semântica de `consume_key` do egui:
    /// shift/alt extras são ignorados, então os handlers continuam checando o mais específico primeiro).
    pub fn consume(&self, ctx: &egui::Context, id: &str) -> bool {
        let chords = self.chords(id);
        if chords.is_empty() {
            return false;
        }
        ctx.input_mut(|i| chords.iter().any(|c| i.consume_key(c.modifiers(), c.key)))
    }

    // ------------------------------------------------------------------------------ JSON

    /// `all = false`: só o delta (o que o usuário mudou). `all = true`: o keymap inteiro.
    pub fn to_json(&self, all: bool) -> String {
        let mut bindings = BTreeMap::new();
        for d in ACTIONS {
            if all || self.is_modified(d.id) {
                bindings.insert(d.id.to_string(), self.chords(d.id).iter().map(Chord::to_text).collect());
            }
        }
        serde_json::to_string_pretty(&KeymapFile { schema: KEYMAP_SCHEMA, bindings }).expect("keymap serializa")
    }

    /// Lê um JSON de atalhos tolerante: ids desconhecidos e combinações inválidas viram avisos, o resto entra.
    /// Entradas iguais ao padrão são normalizadas (não viram override).
    pub fn parse(raw: &str) -> Result<KeymapLoad, KeymapError> {
        let v: serde_json::Value = serde_json::from_str(raw)?;
        let obj = v.as_object().ok_or(KeymapError::NotAnObject)?;
        let schema = obj.get("schema").and_then(|s| s.as_u64()).unwrap_or(0);
        if schema > KEYMAP_SCHEMA as u64 {
            return Err(KeymapError::FutureSchema(schema));
        }
        let mut out = KeymapLoad::default();
        let Some(b) = obj.get("bindings").and_then(|b| b.as_object()) else {
            return Ok(out);
        };
        for (id, val) in b {
            let Some(d) = def(id) else {
                out.warnings.push(format!("ação desconhecida ignorada: {id}"));
                continue;
            };
            let Some(list) = val.as_array() else {
                out.warnings.push(format!("{id}: esperado uma lista de atalhos"));
                continue;
            };
            let mut chords = Vec::new();
            let mut bad = false;
            for t in list {
                match t.as_str().map(Chord::parse) {
                    Some(Ok(c)) => {
                        if !chords.contains(&c) {
                            chords.push(c)
                        }
                    }
                    Some(Err(e)) => {
                        out.warnings.push(format!("{id}: {e}"));
                        bad = true;
                    }
                    None => {
                        out.warnings.push(format!("{id}: atalho que não é texto"));
                        bad = true;
                    }
                }
            }
            if bad {
                continue; // não aplica uma lista parcial: mantém o padrão dessa ação
            }
            if chords != default_chords(d) {
                out.overrides.insert(id.clone(), chords);
            }
        }
        Ok(out)
    }

    pub fn from_json(raw: &str) -> Result<(Keymap, Vec<String>), KeymapError> {
        let l = Self::parse(raw)?;
        Ok((Keymap::with_overrides(l.overrides), l.warnings))
    }

    pub fn overrides(&self) -> &BTreeMap<String, Vec<Chord>> {
        &self.overrides
    }

    pub fn save_to(&self, path: &std::path::Path, all: bool) -> Result<(), KeymapError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_json(all))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Carrega o arquivo do usuário. Ausente = padrão; ilegível = padrão + cópia `.bak` + aviso. Nunca entra em pânico.
    pub fn load_file(path: &std::path::Path) -> (Keymap, Vec<String>) {
        let raw = match std::fs::read_to_string(path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Keymap::default(), Vec::new()),
            Err(e) => {
                let _ = std::fs::copy(path, path.with_extension("json.bak"));
                return (Keymap::default(), vec![format!("atalhos ilegíveis ({}): {e}", path.display())]);
            }
        };
        match Self::from_json(&raw) {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::copy(path, path.with_extension("json.bak"));
                (Keymap::default(), vec![format!("atalhos ignorados ({}): {e}", path.display())])
            }
        }
    }
}

/// Arquivo padrão dos atalhos do usuário.
pub fn default_keymap_path() -> std::path::PathBuf {
    crate::layout::config_dir().join("keymap.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WorkspaceId;
    use egui::Key;
    use std::collections::HashSet;

    fn chord(t: &str) -> Chord {
        Chord::parse(t).unwrap()
    }

    #[test]
    fn chord_text_roundtrips_for_every_default() {
        for d in ACTIONS {
            for t in d.defaults {
                let c = Chord::parse(t).unwrap_or_else(|e| panic!("{}: {e}", d.id));
                assert_eq!(&c.to_text(), t, "{} não é o texto canônico", d.id);
                assert_eq!(Chord::parse(&c.to_text()).unwrap(), c);
            }
        }
        // sinônimos e maiúsculas
        assert_eq!(chord("cmd+shift+z"), chord("Ctrl+Shift+Z"));
        assert_eq!(chord("Shift+Ctrl+Z"), chord("Ctrl+Shift+Z"));
        assert_eq!(chord("Option+h"), chord("Alt+H"));
        assert_eq!(chord("["), Chord::new(Key::OpenBracket));
    }

    #[test]
    fn invalid_chords_are_rejected_with_a_reason() {
        for bad in ["", "Ctrl+", "Ctrl+Shift", "A+B", "Ctrl+Nada", "Ctrl+Copy", "++"] {
            assert!(Chord::parse(bad).is_err(), "{bad:?} deveria falhar");
        }
    }

    #[test]
    fn registry_ids_are_unique_and_every_action_has_a_default_or_is_explicitly_unbound() {
        let mut seen = HashSet::new();
        for d in ACTIONS {
            assert!(seen.insert(d.id), "id repetido {}", d.id);
            assert!(!d.ctxs.is_empty(), "{} sem contexto", d.id);
            assert!(!d.defaults.is_empty(), "{} sem atalho padrão", d.id);
            let mut u = HashSet::new();
            for t in d.defaults {
                assert!(u.insert(Chord::parse(t).unwrap()), "{} repete o atalho {t}", d.id);
            }
        }
    }

    #[test]
    fn the_default_keymap_has_no_conflicts() {
        let km = Keymap::default();
        let c = km.conflicts();
        assert!(c.is_empty(), "padrões em conflito: {:?}", c.iter().map(|c| format!("{} x {} em {}", c.a, c.b, c.chord.display())).collect::<Vec<_>>());
    }

    #[test]
    fn defaults_reproduce_the_blender_and_unreal_schemes() {
        let km = Keymap::default();
        assert_eq!(km.chords("modeling.grab"), &[chord("G")]);
        assert_eq!(km.chords("modeling.edit.loop_cut"), &[chord("Ctrl+R")]);
        assert_eq!(km.chords("modeling.toggle_edit"), &[chord("Tab")]);
        assert_eq!(km.chords("scene.tool.select"), &[chord("Q")]);
        assert_eq!(km.chords("scene.tool.move"), &[chord("W")]);
        assert_eq!(km.chords("scene.tool.rotate"), &[chord("E")]);
        assert_eq!(km.chords("scene.tool.scale"), &[chord("R")]);
        assert_eq!(km.chords("scene.duplicate"), &[chord("Ctrl+W")]);
        assert_eq!(km.chords("edit.redo"), &[chord("Ctrl+Shift+Z"), chord("Ctrl+Y")]);
        assert_eq!(km.chords("anim.play"), &[chord("Space")]);
        assert_eq!(km.label("edit.undo"), "Ctrl+Z");
    }

    #[test]
    fn conflicts_are_detected_per_context_not_globally() {
        let mut km = Keymap::default();
        // mesmo atalho em workspaces diferentes: sem conflito (R é girar no Cenário e na Modelagem por padrão)
        assert!(km.chords("scene.tool.scale").contains(&chord("R")) && km.chords("modeling.rotate").contains(&chord("R")));
        assert!(km.conflicts().is_empty());
        // dois comandos do Cenário na mesma tecla: conflito
        km.set("scene.tool.scale", vec![chord("W")]);
        let c = km.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].a, c[0].b), ("scene.tool.move", "scene.tool.scale"));
        assert_eq!(km.conflicts_with("scene.tool.move", &chord("W")), vec!["scene.tool.scale"]);
        // um atalho GLOBAL colide com qualquer contexto
        let mut km = Keymap::default();
        km.set("edit.undo", vec![chord("Q")]);
        assert!(km.conflicts().iter().any(|c| c.a == "edit.undo" && c.b == "scene.tool.select"));
        // modos diferentes da Modelagem não colidem (X = excluir em Objeto e em Edição por padrão)
        assert!(Keymap::default().conflicts().is_empty());
        // áreas diferentes no Animação (M = marcador na timeline x espelhar pose fora dela)
        let km = Keymap::default();
        assert!(km.chords("anim.marker").contains(&chord("M")) && km.chords("pose.mirror").contains(&chord("M")));
        assert!(km.conflicts().is_empty());
        // ...mas virar "Any" colidiria
        let mut km = Keymap::default();
        km.set("anim.key_insert", vec![chord("M")]);
        assert!(km.conflicts().iter().any(|c| (c.a == "anim.marker" || c.b == "anim.marker") && c.chord == chord("M")));
    }

    #[test]
    fn routing_depends_on_workspace_mode_and_area() {
        let km = Keymap::default();
        let cen = Active { ws: WorkspaceId::Cenario, mode: None, area: Area::Viewport };
        assert_eq!(km.route(&chord("Q"), &cen), vec!["scene.tool.select"]);
        assert_eq!(km.route(&chord("W"), &cen), vec!["scene.tool.move"]);
        assert_eq!(km.route(&chord("Ctrl+Z"), &cen), vec!["edit.undo"], "global em qualquer lugar");
        let mo = Active { ws: WorkspaceId::Modelagem, mode: Some("object"), area: Area::Viewport };
        assert!(km.route(&chord("Q"), &mo).is_empty(), "Q não é nada na Modelagem");
        assert_eq!(km.route(&chord("X"), &mo), vec!["modeling.object.delete"]);
        let me = Active { mode: Some("edit"), ..mo };
        assert_eq!(km.route(&chord("X"), &me), vec!["modeling.edit.delete"]);
        let mh = Active { mode: Some("hair"), ..mo };
        assert_eq!(km.route(&chord("X"), &mh), vec!["hair.delete"]);
        assert!(km.route(&chord("G"), &mh).is_empty(), "G (grab) não vale no modo cabelo");
        let an_tl = Active { ws: WorkspaceId::Animacao, mode: None, area: Area::Timeline };
        let an_vp = Active { area: Area::Viewport, ..an_tl };
        assert_eq!(km.route(&chord("M"), &an_tl), vec!["anim.marker"]);
        assert_eq!(km.route(&chord("M"), &an_vp), vec!["pose.mirror"]);
        assert_eq!(km.route(&chord("Space"), &an_tl), vec!["anim.play"]);
    }

    #[test]
    fn overrides_replace_defaults_and_only_the_delta_is_saved() {
        let mut km = Keymap::default();
        km.set("scene.tool.move", vec![chord("T")]);
        assert_eq!(km.chords("scene.tool.move"), &[chord("T")]);
        assert!(km.is_modified("scene.tool.move") && km.modified_count() == 1);
        let j = km.to_json(false);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["bindings"].as_object().unwrap().len(), 1);
        assert_eq!(v["bindings"]["scene.tool.move"], serde_json::json!(["T"]));
        // voltar ao padrão remove o override
        km.set("scene.tool.move", vec![chord("W")]);
        assert_eq!(km.modified_count(), 0);
        // desligar uma ação
        km.set("scene.hide", vec![]);
        assert!(km.chords("scene.hide").is_empty() && km.is_modified("scene.hide"));
        assert_eq!(km.label("scene.hide"), "");
        km.reset("scene.hide");
        assert_eq!(km.chords("scene.hide"), &[chord("H")]);
        // add não duplica
        km.add("scene.hide", chord("H"));
        km.add("scene.hide", chord("Shift+H"));
        assert_eq!(km.chords("scene.hide"), &[chord("H"), chord("Shift+H")]);
        km.reset_all();
        assert_eq!(km.modified_count(), 0);
    }

    #[test]
    fn export_import_roundtrip_full_and_delta() {
        let mut km = Keymap::default();
        km.set("scene.tool.move", vec![chord("T"), chord("Ctrl+T")]);
        km.set("modeling.edit.extrude", vec![chord("Shift+E")]);
        for all in [false, true] {
            let (back, warns) = Keymap::from_json(&km.to_json(all)).unwrap();
            assert!(warns.is_empty(), "{warns:?}");
            for d in ACTIONS {
                assert_eq!(back.chords(d.id), km.chords(d.id), "{} (all={all})", d.id);
            }
            assert_eq!(back.modified_count(), 2, "o export completo também normaliza para o delta");
        }
        assert!(km.to_json(true).len() > km.to_json(false).len());
    }

    #[test]
    fn import_is_tolerant_but_not_silent() {
        let raw = r#"{"schema":1,"bindings":{
            "scene.tool.move":["T"],
            "acao.que.nao.existe":["A"],
            "scene.hide":["Ctrl+Nada"],
            "scene.rename":"F3",
            "scene.focus":[7]}}"#;
        let (km, warns) = Keymap::from_json(raw).unwrap();
        assert_eq!(km.chords("scene.tool.move"), &[chord("T")]);
        assert_eq!(km.chords("scene.hide"), &[chord("H")], "lista inválida mantém o padrão");
        assert_eq!(km.chords("scene.rename"), &[chord("F2")]);
        assert_eq!(warns.len(), 4, "{warns:?}");
        // arquivo escrito à mão sem schema vale; futuro é recusado; lixo é recusado
        assert!(Keymap::from_json(r#"{"bindings":{}}"#).is_ok());
        assert!(matches!(Keymap::from_json(r#"{"schema":9,"bindings":{}}"#), Err(KeymapError::FutureSchema(9))));
        assert!(Keymap::from_json("não é json").is_err());
        assert!(matches!(Keymap::from_json("[]"), Err(KeymapError::NotAnObject)));
    }

    #[test]
    fn corrupted_keymap_file_falls_back_with_backup() {
        let dir = std::env::temp_dir().join(format!("anigo_keymap_corrupt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("keymap.json");
        std::fs::write(&p, "{{{ quebrado").unwrap();
        let (km, warns) = Keymap::load_file(&p);
        assert_eq!(km.modified_count(), 0);
        assert_eq!(warns.len(), 1);
        assert!(dir.join("keymap.json.bak").exists());
        let (km, warns) = Keymap::load_file(&dir.join("ausente.json"));
        assert!(km.modified_count() == 0 && warns.is_empty());
        // ida e volta em arquivo
        let mut km = Keymap::default();
        km.set("anim.play", vec![chord("P")]);
        km.save_to(&p, false).unwrap();
        assert_eq!(Keymap::load_file(&p).0.chords("anim.play"), &[chord("P")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn key_event(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    fn run_with<R>(events: Vec<egui::Event>, f: impl FnOnce(&egui::Context) -> R) -> R {
        let ctx = egui::Context::default();
        let mut out = None;
        let mut f = Some(f);
        let input = egui::RawInput { events, ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            if let Some(f) = f.take() {
                out = Some(f(ui.ctx()));
            }
        });
        full.textures_delta.clear();
        out.unwrap()
    }

    #[test]
    fn consume_uses_the_resolved_chords_and_consumes_once() {
        let mut km = Keymap::default();
        // padrão: W move
        let (a, b) = run_with(vec![key_event(Key::W, egui::Modifiers::NONE)], |ctx| (km.consume(ctx, "scene.tool.move"), km.consume(ctx, "scene.tool.move")));
        assert!(a && !b, "consome uma única vez");
        // com W trocado para T: W não dispara mais, T dispara
        km.set("scene.tool.move", vec![chord("T")]);
        let r = run_with(vec![key_event(Key::W, egui::Modifiers::NONE)], |ctx| km.consume(ctx, "scene.tool.move"));
        assert!(!r);
        let r = run_with(vec![key_event(Key::T, egui::Modifiers::NONE)], |ctx| km.consume(ctx, "scene.tool.move"));
        assert!(r);
        // Ctrl+Shift+Z e Ctrl+Y são Refazer; Ctrl+Z puro não
        let km = Keymap::default();
        let ctrl_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        assert!(run_with(vec![key_event(Key::Z, ctrl_shift)], |ctx| km.consume(ctx, "edit.redo")));
        assert!(run_with(vec![key_event(Key::Y, egui::Modifiers::COMMAND)], |ctx| km.consume(ctx, "edit.redo")));
        assert!(!run_with(vec![key_event(Key::Z, egui::Modifiers::COMMAND)], |ctx| km.consume(ctx, "edit.redo")));
        assert!(run_with(vec![key_event(Key::Z, egui::Modifiers::COMMAND)], |ctx| km.consume(ctx, "edit.undo")));
        // ação desligada nunca dispara
        let mut km = Keymap::default();
        km.set("edit.undo", vec![]);
        assert!(!run_with(vec![key_event(Key::Z, egui::Modifiers::COMMAND)], |ctx| km.consume(ctx, "edit.undo")));
    }

    #[test]
    fn every_action_is_translated_and_has_no_emoji() {
        use crate::i18n::{tr_in, Lang};
        for d in ACTIONS {
            for l in [Lang::PtBr, Lang::En] {
                let k = d.label_key();
                let t = tr_in(l, &k);
                assert_ne!(t, k, "ação sem tradução ({l:?}): {}", d.id);
                assert!(t.chars().all(|c| (c as u32) < 0x2190 || c == '\u{2026}'), "símbolo/emoji em {k}");
            }
        }
    }
}
