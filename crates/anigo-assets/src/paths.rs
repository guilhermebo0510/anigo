//! Confinamento de URIs/caminhos relativos: tudo que vem de arquivos de terceiros
//! (glTF `uri`, MTL `map_Kd`, FBX texturas, `.anigomat.json`) passa por aqui.

use std::path::{Component, Path, PathBuf};

use crate::error::{AssetError, Result};

/// Decodifica `%XX`. Retorna `None` se a sequencia for invalida ou o resultado nao for UTF-8.
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = (*b.get(i + 1)? as char).to_digit(16)?;
            let lo = (*b.get(i + 2)? as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Normaliza um caminho relativo vindo de fora para uma forma canonica com `/`, sem `.`/`..`.
///
/// Rejeita: vazio, NUL/controle, absoluto (`/x`, `C:\x`, `\\srv`), esquema (`file:`, `http:`, `data:`...),
/// qualquer componente `..` (inclusive apos decodificar `%2e%2e`) e barras invertidas disfarcadas.
pub fn normalize_relative(uri: &str) -> Result<String> {
    let bad = |why: &str| AssetError::UnsafePath(format!("{why}: {uri:?}"));
    if uri.is_empty() {
        return Err(bad("vazio"));
    }
    if uri.len() > 4096 {
        return Err(bad("longo demais"));
    }
    // esquema (`scheme:`) ou drive (`C:`)
    if let Some(colon) = uri.find(':') {
        let head = &uri[..colon];
        if !head.is_empty() && head.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return Err(bad("esquema/drive nao permitido"));
        }
    }
    let decoded = percent_decode(uri).ok_or_else(|| bad("percent-encoding invalido"))?;
    if decoded.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(bad("caracter de controle"));
    }
    let unified = decoded.replace('\\', "/");
    if unified.starts_with('/') {
        return Err(bad("caminho absoluto"));
    }
    let mut parts: Vec<&str> = Vec::new();
    for comp in unified.split('/') {
        match comp {
            "" | "." => continue,
            ".." => return Err(bad("componente '..'")),
            c => {
                if c.contains(':') {
                    return Err(bad("':' em componente"));
                }
                parts.push(c)
            }
        }
    }
    if parts.is_empty() {
        return Err(bad("sem componentes"));
    }
    Ok(parts.join("/"))
}

/// Junta `base` + `rel` (ja normalizado por [`normalize_relative`]) e confere que o resultado
/// continua dentro de `base` mesmo apos resolver symlinks, quando o arquivo existe.
pub fn confine(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel = normalize_relative(rel)?;
    let joined = base.join(&rel);
    // Defesa em profundidade contra componentes inesperados.
    for c in Path::new(&rel).components() {
        if !matches!(c, Component::Normal(_)) {
            return Err(AssetError::UnsafePath(rel));
        }
    }
    if joined.exists() {
        let real_base = base.canonicalize().map_err(|e| AssetError::io(base, e))?;
        let real = joined.canonicalize().map_err(|e| AssetError::io(&joined, e))?;
        if !real.starts_with(&real_base) {
            return Err(AssetError::UnsafePath(format!("{rel} escapa de {} (symlink?)", base.display())));
        }
    }
    Ok(joined)
}

/// Caminho relativo (com `/`) de `path` dentro de `base`, ou `None` se estiver fora.
pub(crate) fn relative_to(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative() {
        assert_eq!(normalize_relative("tex/a.png").unwrap(), "tex/a.png");
        assert_eq!(normalize_relative("./tex//a.png").unwrap(), "tex/a.png");
        assert_eq!(normalize_relative("my%20tex.png").unwrap(), "my tex.png");
        assert_eq!(normalize_relative("a\\b.png").unwrap(), "a/b.png");
    }

    #[test]
    fn rejects_traversal_and_absolute() {
        for bad in [
            "../x.png",
            "a/../../x",
            "a/..",
            "/etc/passwd",
            "\\\\srv\\share\\x",
            "C:\\x.png",
            "c:/x.png",
            "file:///etc/passwd",
            "http://evil/x.png",
            "data:image/png;base64,AAAA",
            "%2e%2e/x.png",
            "a/%2E%2E/b",
            "%2fetc/passwd",
            "a%00b",
            "",
            ".",
            "a\nb",
            "%zz",
            "%4",
        ] {
            assert!(normalize_relative(bad).is_err(), "deveria rejeitar {bad:?}");
        }
    }

    #[test]
    fn confine_blocks_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        let base = dir.path().join("asset");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("ok.txt"), "ok").unwrap();
        assert!(confine(&base, "ok.txt").is_ok());
        assert!(confine(&base, "missing.txt").is_ok()); // inexistente: so valida sintaxe
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), base.join("link")).unwrap();
            assert!(matches!(confine(&base, "link/secret.txt"), Err(AssetError::UnsafePath(_))));
        }
    }

    #[test]
    fn relative_to_works() {
        let base = Path::new("/a/b");
        assert_eq!(relative_to(base, Path::new("/a/b/c/d.txt")).unwrap(), "c/d.txt");
        assert!(relative_to(base, Path::new("/a/x")).is_none());
        assert!(relative_to(base, base).is_none());
    }
}
