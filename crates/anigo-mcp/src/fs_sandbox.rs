use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// P0-04: Filesystem sandbox
/// Allowlist: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp, ./tmp, plus env overrides.

const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024; // 50 MB
const MAX_DIM: u32 = 4096;
const MAX_PIXELS: u64 = 4096 * 4096; // 16_777_216

pub fn max_dim() -> u32 {
    MAX_DIM
}
pub fn max_pixels() -> u64 {
    MAX_PIXELS
}
pub fn max_image_bytes() -> usize {
    MAX_IMAGE_BYTES
}

/// Returns list of allowed base directories (absolute).
pub fn allowed_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Documents/ANIGO - try USERPROFILE, HOME, DOCUMENTS
    if let Ok(doc) = std::env::var("USERPROFILE") {
        dirs.push(PathBuf::from(doc).join("Documents").join("ANIGO"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join("Documents").join("ANIGO"));
        dirs.push(PathBuf::from(&home).join("ANIGO"));
    }
    if let Ok(doc_dir) = std::env::var("ANIGO_DOCUMENTS_DIR") {
        dirs.push(PathBuf::from(doc_dir));
    }

    // 2. Relative allowlist resolved to absolute via current_dir
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for rel in &[
        "baselines",
        "tmp/anigo-mcp",
        "tmp",
        "public",
        ".",
    ] {
        dirs.push(cwd.join(rel));
    }

    // 3. Env ANIGO_MCP_ALLOWED_DIRS extra (colon/semicolon separated)
    if let Ok(extra) = std::env::var("ANIGO_MCP_ALLOWED_DIRS") {
        // no Windows ':' faz parte da letra da unidade (`C:\x`): só `;` separa
        let sep = |c: char| c == ';' || (!cfg!(windows) && c == ':');
        for p in extra.split(sep) {
            if !p.trim().is_empty() {
                dirs.push(PathBuf::from(p.trim()));
            }
        }
    }

    // Deduplicate and make absolute-ish
    let mut out = Vec::new();
    for d in dirs {
        let abs = if d.is_absolute() {
            d
        } else {
            cwd.join(d)
        };
        // canonicalize if exists, otherwise keep as is
        let canon = abs.canonicalize().unwrap_or(abs);
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

/// Caminho absoluto OU com raiz explícita (`/x`, `\x`). No Windows esses últimos não têm letra de unidade e `is_absolute()`
/// dá `false`: tratá-los como relativos os REDIRECIONARIA para dentro do sandbox (e gravaria o arquivo) em vez de recusar.
fn is_rooted(p: &Path) -> bool {
    p.is_absolute() || p.has_root()
}

fn contains_parent_dir(p: &Path) -> bool {
    for comp in p.components() {
        if matches!(comp, Component::ParentDir) {
            return true;
        }
    }
    false
}

/// Canonicaliza o ANCESTRAL existente mais próximo e reanexa o resto: um destino ainda inexistente (`<permitida>/sub/x.png`)
/// ganha a mesma forma canônica (`\\?\C:\…` no Windows) das pastas permitidas, senão `starts_with` nunca casa.
fn canonical_lenient(p: &Path) -> PathBuf {
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(|q| q.to_path_buf())) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

fn inside_allowed(p: &Path, allowed: &[PathBuf]) -> bool {
    let cand = canonical_lenient(p);
    allowed.iter().any(|a| cand.starts_with(canonical_lenient(a)) || p.starts_with(a))
}

/// Validate that a path does NOT escape via ".." and is within allowlist for write.
pub fn sanitize_save_path(input: &str) -> Result<PathBuf> {
    // Env kill-switch
    if let Ok(val) = std::env::var("ANIGO_MCP_ALLOW_FS_WRITE") {
        if val == "0" || val.eq_ignore_ascii_case("false") {
            bail!("Filesystem writes are disabled by ANIGO_MCP_ALLOW_FS_WRITE=0 (attempted: {})", input);
        }
    }

    if input.trim().is_empty() {
        bail!("Empty save_path");
    }

    let p = PathBuf::from(input);

    // Reject suspicious patterns
    if input.contains('\0') {
        bail!("Invalid path: contains null byte");
    }

    // For relative paths, we will join with first allowed dir (Documents/ANIGO or baselines)
    let allowed = allowed_dirs();
    if allowed.is_empty() {
        bail!("No allowed directories configured");
    }

    let candidate: PathBuf = if is_rooted(&p) {
        // Absoluto: sem `..` (um `..` textual "dentro" da pasta permitida escaparia) e dentro da lista, comparando as formas CANÔNICAS
        if contains_parent_dir(&p) {
            bail!("Path traversal detected: {:?} contains '..'", input);
        }
        if !inside_allowed(&p, &allowed) {
            bail!("Path {:?} is not in allowlist {:?}. Allowed: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp", p, allowed);
        }
        p
    } else {
        // Relative: reject parent traversal unless it stays inside allowed after join
        if contains_parent_dir(&p) {
            // Resolve against first allowed dir and ensure it doesn't escape
            let first = &allowed[0];
            let joined = first.join(&p);
            // Use canonicalize for parent if possible
            // For security, we normalize components manually
            let mut normalized = PathBuf::new();
            for comp in joined.components() {
                match comp {
                    Component::ParentDir => { normalized.pop(); },
                    Component::CurDir => {},
                    _ => normalized.push(comp.as_os_str()),
                }
            }
            let first_canon = first.canonicalize().unwrap_or_else(|_| first.clone());
            let norm_parent = normalized.parent().unwrap_or(&normalized).to_path_buf();
            let norm_parent_canon = norm_parent.canonicalize().unwrap_or(norm_parent);
            if !norm_parent_canon.starts_with(&first_canon) && !normalized.starts_with(first) {
                bail!("Path traversal detected: {:?} escapes allowlist", input);
            }
            // Return joined path inside first allowed dir
            first.join(&p)
        } else {
            // If relative but already starts with allowed prefix like "baselines/foo.png", keep as is but resolve to absolute for write
            // Check if it starts with any allowed relative name
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let abs = cwd.join(&p);
            // Ensure abs is inside one of allowed
            let mut inside = false;
            for a in &allowed {
                if abs.starts_with(a) {
                    inside = true;
                    break;
                }
            }
            // If not inside, force it into first allowed dir
            if inside {
                p
            } else {
                // Default: place inside tmp/anigo-mcp to be safe
                // Find tmp/anigo-mcp or baselines in allowed list
                let fallback = allowed.iter().find(|d| d.ends_with("tmp/anigo-mcp") || d.ends_with("baselines")).unwrap_or(&allowed[0]);
                fallback.join(p.file_name().unwrap_or_else(|| std::ffi::OsStr::new("output.png")))
            }
        }
    };

    // Ensure parent exists (create)
    if let Some(parent) = candidate.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("Failed to create parent dir {:?}", parent))?;
    }

    Ok(candidate)
}

/// For reading, we allow same allowlist but also allow absolute if file exists and is not sensitive.
/// We still block reading of sensitive files like .ssh, /etc/passwd, etc.
pub fn sanitize_read_path(input: &str) -> Result<PathBuf> {
    if input.trim().is_empty() {
        bail!("Empty read path");
    }
    if input.contains('\0') {
        bail!("Invalid path");
    }

    let p = PathBuf::from(input);
    let input_lower = input.to_lowercase();

    // Block sensitive files
    let blocked_substrings = [
        ".ssh",
        "id_rsa",
        ".env",
        "/etc/passwd",
        "/etc/shadow",
        "system32",
        "windows\\system32",
    ];
    for blocked in &blocked_substrings {
        if input_lower.contains(blocked) {
            bail!("Reading path {:?} is blocked for security (contains {})", input, blocked);
        }
    }

    let allowed = allowed_dirs();
    let candidate: PathBuf = if is_rooted(&p) {
        // Dentro da lista OU do diretório atual (formas canônicas); `..` nunca
        if contains_parent_dir(&p) {
            bail!("Path traversal detected in read path: {:?}", input);
        }
        let cwd = std::env::current_dir().unwrap_or_default();
        if !inside_allowed(&p, &allowed) && !inside_allowed(&p, &[cwd]) {
            bail!("Read path {:?} is not in allowlist {:?} nor current directory", p, allowed);
        }
        p
    } else {
        if contains_parent_dir(&p) {
            bail!("Path traversal detected in read path: {:?}", input);
        }
        // Resolve relative against cwd and check
        let cwd = std::env::current_dir().unwrap_or_default();
        let abs = cwd.join(&p);
        let mut ok = false;
        for a in &allowed {
            if abs.starts_with(a) {
                ok = true;
                break;
            }
        }
        if abs.starts_with(&cwd) {
            ok = true;
        }
        if !ok {
            bail!("Read path {:?} not in allowlist", input);
        }
        p
    };

    // Check file size limit before reading (will be checked again in load)
    if candidate.exists() {
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.len() as usize > MAX_IMAGE_BYTES {
                bail!("File {:?} too large: {} bytes > {} bytes limit", candidate, meta.len(), MAX_IMAGE_BYTES);
            }
        }
    }

    Ok(candidate)
}

pub fn validate_render_dims(width: u32, height: u32) -> Result<()> {
    if width < 64 || height < 64 {
        bail!("Invalid dimensions {}x{}: minimum is 64x64 (code -32602)", width, height);
    }
    if width > MAX_DIM || height > MAX_DIM {
        bail!("Invalid dimensions {}x{}: maximum is {}x{} (code -32602)", width, height, MAX_DIM, MAX_DIM);
    }
    let pixels = width as u64 * height as u64;
    if pixels > MAX_PIXELS {
        bail!("Invalid dimensions {}x{}: {} MP exceeds limit {} MP (code -32602)", width, height, pixels / 1_000_000, MAX_PIXELS / 1_000_000);
    }
    Ok(())
}

pub fn validate_tolerance_channel_diff(v: i32) -> Result<()> {
    if v < 0 || v > 255 {
        bail!("tolerance_channel_diff out of range 0..255: {}", v);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_dims_rejects_too_small() {
        assert!(validate_render_dims(32, 32).is_err());
        assert!(validate_render_dims(64, 64).is_ok());
    }

    #[test]
    fn render_dims_rejects_too_large() {
        assert!(validate_render_dims(99999, 1).is_err());
        assert!(validate_render_dims(4096, 4096).is_ok());
        assert!(validate_render_dims(4097, 4096).is_err());
    }

    #[test]
    fn render_dims_rejects_oversized_pixel_count() {
        // 4096 x 4096 = 16 MP, ok. 5000 x 5000 = 25 MP, rejeitado.
        assert!(validate_render_dims(5000, 5000).is_err());
    }

    #[test]
    fn tolerance_channel_diff_range() {
        assert!(validate_tolerance_channel_diff(0).is_ok());
        assert!(validate_tolerance_channel_diff(255).is_ok());
        assert!(validate_tolerance_channel_diff(-1).is_err());
        assert!(validate_tolerance_channel_diff(256).is_err());
    }

    #[test]
    fn sanitize_read_path_blocks_etc_passwd() {
        assert!(sanitize_read_path("/etc/passwd").is_err());
    }

    #[test]
    fn sanitize_read_path_blocks_id_rsa() {
        assert!(sanitize_read_path("/home/user/.ssh/id_rsa").is_err());
    }

    #[test]
    fn sanitize_read_path_rejects_traversal() {
        assert!(sanitize_read_path("../../etc/passwd").is_err());
        assert!(sanitize_save_path("../../../etc/passwd").is_err());
    }

    #[test]
    fn sanitize_read_path_rejects_null_byte() {
        assert!(sanitize_read_path("foo\0bar.png").is_err());
    }

    #[test]
    fn contains_parent_dir_detects_dotdot() {
        assert!(contains_parent_dir(Path::new("../etc")));
        assert!(!contains_parent_dir(Path::new("baselines/a.png")));
    }

    #[test]
    fn rooted_paths_without_a_drive_are_rejected_not_redirected_into_the_sandbox() {
        // no Windows `/etc/x` não é `is_absolute()`: antes virava `baselines/x` e GRAVAVA o arquivo
        assert!(is_rooted(Path::new("/etc/anigo_nope.png")));
        assert!(!is_rooted(Path::new("baselines/a.png")));
        assert!(sanitize_save_path("/etc/anigo_nope.png").is_err());
        assert!(sanitize_save_path("/anigo_raiz_nope/x.png").is_err());
        assert!(sanitize_read_path("/anigo_raiz_nope/x.png").is_err());
    }

    #[test]
    fn nonexistent_subfolders_of_an_allowed_dir_are_accepted_and_dotdot_escapes_are_not() {
        // sem mexer em variáveis de ambiente (os testes rodam em paralelo): o diretório atual já é permitido
        let base = std::env::current_dir().unwrap().join("target").join(format!("anigo_sb_{}", std::process::id()));
        let ok = base.join("novo").join("fundo").join("x.png");
        assert!(sanitize_save_path(&ok.display().to_string()).is_ok(), "subpasta ainda inexistente dentro da permitida");
        assert!(sanitize_read_path(&base.join("qualquer.glb").display().to_string()).is_ok());
        let esc = base.join("a").join("..").join("..").join("fora.png");
        assert!(sanitize_save_path(&esc.display().to_string()).is_err(), "`..` não escapa");
        assert!(sanitize_read_path(&esc.display().to_string()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_killswitch() {
        // Ensure that when ANIGO_MCP_ALLOW_FS_WRITE=0, writes are rejected.
        // We set the var, call, then unset it.
        unsafe { std::env::set_var("ANIGO_MCP_ALLOW_FS_WRITE", "0"); }
        assert!(sanitize_save_path("tmp/out.png").is_err());
        unsafe { std::env::remove_var("ANIGO_MCP_ALLOW_FS_WRITE"); }
    }
}
