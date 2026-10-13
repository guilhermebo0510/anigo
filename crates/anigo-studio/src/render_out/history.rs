//! Histórico de saídas de render: `<pasta>/.anigo_renders.json` + miniaturas em `<pasta>/.anigo_thumbs/`.
//! Mora na pasta de saída (viaja com as imagens); entradas cujo arquivo sumiu são ignoradas ao carregar.

use super::png_out::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const HISTORY_FILE: &str = ".anigo_renders.json";
pub const THUMB_DIR: &str = ".anigo_thumbs";
/// Maior lado da miniatura.
pub const THUMB_SIDE: u32 = 192;
/// Teto de entradas guardadas (as mais antigas saem).
pub const HISTORY_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Nome do arquivo na pasta de saída.
    pub file: String,
    /// `beauty`, `outline`, `depth`, `mask`, `id`, `normal`.
    pub pass: String,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    pub project: String,
    pub camera: String,
    /// Data/hora UTC do render (só aqui: o PNG não leva relógio).
    pub when: String,
    pub fingerprint: String,
    #[serde(default)]
    pub frame: Option<u32>,
    /// Nome da miniatura em `.anigo_thumbs/`.
    #[serde(default)]
    pub thumb: Option<String>,
}

impl HistoryEntry {
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.file)
    }

    pub fn thumb_path(&self, dir: &Path) -> Option<PathBuf> {
        self.thumb.as_ref().map(|t| dir.join(THUMB_DIR).join(t))
    }
}

#[derive(Serialize, Deserialize, Default)]
struct HistoryFile {
    version: u32,
    entries: Vec<HistoryEntry>,
}

/// Carrega as entradas cujo arquivo ainda existe, da mais nova para a mais antiga.
pub fn load(dir: &Path) -> Vec<HistoryEntry> {
    let Ok(raw) = std::fs::read_to_string(dir.join(HISTORY_FILE)) else { return Vec::new() };
    let Ok(file) = serde_json::from_str::<HistoryFile>(&raw) else { return Vec::new() };
    let mut v: Vec<HistoryEntry> = file.entries.into_iter().filter(|e| e.path(dir).is_file()).collect();
    v.reverse();
    v
}

/// Acrescenta entradas (leitura-modificação-escrita atômica).
pub fn append(dir: &Path, new: Vec<HistoryEntry>) -> Result<(), String> {
    let path = dir.join(HISTORY_FILE);
    let mut file: HistoryFile = std::fs::read_to_string(&path).ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    file.version = 1;
    for e in new {
        // refazer o mesmo arquivo substitui a entrada antiga
        file.entries.retain(|o| !(o.file == e.file));
        file.entries.push(e);
    }
    if file.entries.len() > HISTORY_LIMIT {
        let cut = file.entries.len() - HISTORY_LIMIT;
        file.entries.drain(..cut);
    }
    let json = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
    write_atomic(&path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &str) -> HistoryEntry {
        HistoryEntry { file: file.into(), pass: "beauty".into(), w: 10, h: 10, bytes: 1, project: "p".into(), camera: "c".into(), when: "2026-10-12 10:00:00".into(), fingerprint: "00".into(), frame: None, thumb: None }
    }

    #[test]
    fn history_round_trips_newest_first_and_drops_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"x").unwrap();
        std::fs::write(dir.path().join("b.png"), b"x").unwrap();
        append(dir.path(), vec![entry("a.png")]).unwrap();
        append(dir.path(), vec![entry("b.png"), entry("gone.png")]).unwrap();
        let h = load(dir.path());
        assert_eq!(h.iter().map(|e| e.file.as_str()).collect::<Vec<_>>(), ["b.png", "a.png"]);
        // refazer o mesmo arquivo não duplica
        append(dir.path(), vec![entry("a.png")]).unwrap();
        assert_eq!(load(dir.path()).len(), 2);
        assert!(load(Path::new("/definitely/not/here")).is_empty());
    }
}
