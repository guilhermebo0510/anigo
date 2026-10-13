//! Arquivo de descoberta da bridge: porta efêmera + token por sessão, com permissões restritas.
//!
//! Local (ver `docs/native/pesquisa/mcp_bridge_nativa.md`, D7):
//! * `ANIGO_BRIDGE_DISCOVERY` (arquivo) sobrescreve tudo;
//! * Windows: `%LOCALAPPDATA%\ANIGO\bridge\bridge.json` (por usuário, fora de pastas sincronizadas);
//! * Linux/macOS: `$XDG_RUNTIME_DIR/anigo/bridge.json` (tmpfs 0700 do usuário), senão
//!   `~/.local/share/anigo/bridge/bridge.json`, senão `<tmp>/anigo-bridge/bridge.json`.
//!
//! Escrita atômica: arquivo temporário criado com `create_new` + modo 0600 (Unix) e `rename`.
//! Leitura: em Unix recusa arquivo legível por grupo/outros.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAX_DISCOVERY_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    /// `127.0.0.1:<porta>`
    pub addr: String,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: u64,
    pub version: String,
    pub protocol: u32,
}

/// Caminho padrão do arquivo de descoberta (não cria nada).
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ANIGO_BRIDGE_DISCOVERY").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    resolve_default(
        std::env::var_os("LOCALAPPDATA"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
        cfg!(windows),
    )
}

fn non_empty(v: Option<std::ffi::OsString>) -> Option<PathBuf> {
    v.filter(|p| !p.is_empty()).map(PathBuf::from)
}

fn resolve_default(
    localappdata: Option<std::ffi::OsString>,
    xdg_runtime: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    windows: bool,
) -> PathBuf {
    if windows {
        if let Some(base) = non_empty(localappdata) {
            return base.join("ANIGO").join("bridge").join("bridge.json");
        }
    } else if let Some(rt) = non_empty(xdg_runtime).filter(|p| p.is_absolute()) {
        return rt.join("anigo").join("bridge.json");
    }
    if let Some(home) = non_empty(home) {
        return home.join(".local").join("share").join("anigo").join("bridge").join("bridge.json");
    }
    std::env::temp_dir().join("anigo-bridge").join("bridge.json")
}

/// Grava o arquivo de descoberta de forma atômica e com permissão restrita.
pub fn write(path: &Path, d: &Discovery) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    create_private_dir(dir)?;
    let tmp = dir.join(format!(
        "{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("bridge.json"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp); // sobra de um processo com o mesmo pid
    let body = serde_json::to_vec_pretty(d).map_err(io::Error::other)?;
    let result = (|| {
        let mut f = open_private_new(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Lê (e valida) o arquivo de descoberta.
pub fn read(path: &Path) -> io::Result<Discovery> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() > MAX_DISCOVERY_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "arquivo de descoberta inválido"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} é legível por outros usuários (esperado 0600); recusado", path.display()),
            ));
        }
    }
    let mut raw = String::new();
    std::fs::File::open(path)?.take(MAX_DISCOVERY_BYTES).read_to_string(&mut raw)?;
    let d: Discovery = serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if d.token.is_empty() || !d.addr.starts_with("127.0.0.1:") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "descoberta sem token ou fora do loopback"));
    }
    Ok(d)
}

/// Remove o arquivo só se ainda for desta instância (outra instância pode tê-lo sobrescrito).
pub fn remove_if_owner(path: &Path, pid: u32, token: &str) {
    if let Ok(d) = read(path) {
        if d.pid == pid && d.token == token {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn open_private_new(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn open_private_new(path: &Path) -> io::Result<std::fs::File> {
    // Windows: sem `mode`; o arquivo herda a ACL da pasta (por usuário em %LOCALAPPDATA%).
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn sample() -> Discovery {
        Discovery { addr: "127.0.0.1:4555".into(), port: 4555, token: "abc123".into(), pid: std::process::id(), started_at: 1, version: "0.1.0".into(), protocol: 1 }
    }

    #[test]
    fn default_path_prefers_runtime_dir_then_home_and_windows_localappdata() {
        let p = resolve_default(None, Some(OsString::from("/run/user/1000")), Some(OsString::from("/home/u")), false);
        assert_eq!(p, PathBuf::from("/run/user/1000/anigo/bridge.json"));
        let p = resolve_default(None, None, Some(OsString::from("/home/u")), false);
        assert_eq!(p, PathBuf::from("/home/u/.local/share/anigo/bridge/bridge.json"));
        // XDG_RUNTIME_DIR relativo é ignorado
        let p = resolve_default(None, Some(OsString::from("rel")), Some(OsString::from("/home/u")), false);
        assert!(p.starts_with("/home/u"));
        let p = resolve_default(Some(OsString::from("C:\\Users\\u\\AppData\\Local")), None, None, true);
        assert!(p.ends_with("ANIGO/bridge/bridge.json") || p.ends_with("ANIGO\\bridge\\bridge.json"));
        let p = resolve_default(None, None, None, false);
        assert!(p.ends_with("anigo-bridge/bridge.json"));
    }

    #[test]
    fn write_read_roundtrip_and_cleanup_only_for_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("bridge.json");
        let d = sample();
        write(&path, &d).unwrap();
        assert_eq!(read(&path).unwrap(), d);
        // não deixa temporários
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
        remove_if_owner(&path, d.pid + 1, &d.token);
        assert!(path.exists(), "outro pid não apaga");
        remove_if_owner(&path, d.pid, "outro-token");
        assert!(path.exists(), "outro token não apaga");
        remove_if_owner(&path, d.pid, &d.token);
        assert!(!path.exists());
    }

    #[test]
    fn overwrite_replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.json");
        write(&path, &sample()).unwrap();
        let mut d2 = sample();
        d2.token = "novo".into();
        write(&path, &d2).unwrap();
        assert_eq!(read(&path).unwrap().token, "novo");
    }

    #[cfg(unix)]
    #[test]
    fn file_is_0600_dir_is_0700_and_loose_files_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("novo").join("bridge.json");
        write(&path, &sample()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn read_rejects_garbage_non_loopback_and_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        let mut d = sample();
        d.addr = "10.0.0.5:4555".into();
        write(&path, &d).unwrap();
        assert!(read(&path).is_err());
        let mut d = sample();
        d.token.clear();
        write(&path, &d).unwrap();
        assert!(read(&path).is_err());
        write(&path, &sample()).unwrap();
        std::fs::write(&path, "{ nao e json").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(read(&path).is_err());
        assert!(read(&dir.path().join("inexistente.json")).is_err());
    }
}
