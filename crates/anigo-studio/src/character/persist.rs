//! Persistência do projeto do personagem: `.anigo` = documento canônico do núcleo
//! (`ProjectState` em JSON). A UI só decide o caminho; quem valida é o núcleo.

use anigo_session::CoreSession;
use std::path::{Path, PathBuf};

/// Extensão do projeto.
pub const EXTENSION: &str = "anigo";

/// Garante a extensão `.anigo` (caminho digitado sem extensão ganha uma).
pub fn normalize_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("caminho vazio".to_string());
    }
    let mut p = PathBuf::from(trimmed);
    if p.is_dir() {
        return Err(format!("\"{}\" é uma pasta", p.display()));
    }
    if p.extension().is_none() {
        p.set_extension(EXTENSION);
    }
    Ok(p)
}

/// Grava o documento canônico da sessão (escrita atômica: arquivo temporário + rename).
pub fn save_project(core: &CoreSession, path: &Path) -> Result<(), String> {
    let json = core.document()?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("não foi possível criar a pasta: {e}"))?;
        }
    }
    let tmp = path.with_extension("anigo.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("falha ao gravar: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("falha ao finalizar a gravação: {e}"))
}

/// Abre um `.anigo` na sessão. Documento inválido NÃO altera a sessão (o núcleo valida
/// antes de trocar o estado); o histórico de desfazer recomeça no projeto aberto.
pub fn load_project(core: &mut CoreSession, path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("não foi possível ler \"{}\": {e}", path.display()))?;
    core.load_document(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::Command;
    use anigo_core::ids::MorphId;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anigo_persist_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_then_load_restores_an_identical_project_state() {
        let dir = tmp_dir("rt");
        let path = dir.join("nested").join("heroi.anigo");
        let mut a = CoreSession::new();
        a.apply(Command::SetMorphValue { target: MorphId::for_slider("jaw_v_line_taper"), value: 0.7 }).unwrap();
        a.apply(Command::SetSomatotype { endomorph: 0.1, mesomorph: 0.7, ectomorph: 0.2 }).unwrap();
        a.apply(Command::RenameProject { name: "Heroi".into() }).unwrap();
        save_project(&a, &path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("anigo.tmp").exists(), "escrita atômica não deixa temporário");

        let mut b = CoreSession::new();
        load_project(&mut b, &path).unwrap();
        assert_eq!(b.project(), a.project(), "ProjectState idêntico");
        assert_eq!(b.project().content_fingerprint(), a.project().content_fingerprint());
        assert!(!b.history().can_undo(), "histórico recomeça no projeto aberto");
        // a malha deformada também é a mesma
        assert_eq!(b.deformed_mesh().unwrap().vertices.len(), a.deformed_mesh().unwrap().vertices.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_or_missing_files_do_not_touch_the_session() {
        let dir = tmp_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = CoreSession::new();
        s.apply(Command::RenameProject { name: "Intacto".into() }).unwrap();
        let before = s.project().clone();

        let junk = dir.join("lixo.anigo");
        std::fs::write(&junk, "{ isto não é um projeto").unwrap();
        assert!(load_project(&mut s, &junk).is_err());
        assert!(load_project(&mut s, &dir.join("nao_existe.anigo")).is_err());
        assert_eq!(s.project(), &before);
        assert!(s.history().can_undo(), "falha de abertura preserva o histórico");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_normalization() {
        assert_eq!(normalize_path("  /tmp/x  ").unwrap(), PathBuf::from("/tmp/x.anigo"));
        assert_eq!(normalize_path("/tmp/x.anigo").unwrap(), PathBuf::from("/tmp/x.anigo"));
        assert_eq!(normalize_path("/tmp/x.json").unwrap(), PathBuf::from("/tmp/x.json"));
        assert!(normalize_path("   ").is_err());
        assert!(normalize_path(&std::env::temp_dir().to_string_lossy()).is_err(), "pasta recusada");
    }
}
