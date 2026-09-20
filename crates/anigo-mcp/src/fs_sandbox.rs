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
        for p in extra.split(|c| c == ';' || c == ':') {
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

fn contains_parent_dir(p: &Path) -> bool {
    for comp in p.components() {
        if matches!(comp, Component::ParentDir) {
            return true;
        }
    }
    false
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

    let candidate: PathBuf = if p.is_absolute() {
        // Absolute must be inside allowlist
        let canon_candidate = p.canonicalize().unwrap_or_else(|_| p.clone());
        // Check parent canonicalization for non-existing file
        let parent = canon_candidate.parent().unwrap_or(Path::new("/"));
        let parent_canon = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());

        let mut ok = false;
        for a in &allowed {
            let a_canon = a.canonicalize().unwrap_or_else(|_| a.clone());
            if canon_candidate.starts_with(&a_canon) || parent_canon.starts_with(&a_canon) {
                ok = true;
                break;
            }
        }
        if !ok {
            // Also allow if absolute path is exactly inside cwd/tmp/baselines even if not canonicalized yet
            // We check if the path's prefix matches allowed after making both absolute
            let abs_input = if p.is_absolute() { p.clone() } else { std::env::current_dir().unwrap_or_default().join(&p) };
            let mut ok2 = false;
            for a in &allowed {
                if abs_input.starts_with(a) {
                    ok2 = true;
                    break;
                }
            }
            if !ok2 {
                bail!("Path {:?} is not in allowlist {:?}. Allowed: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp", p, allowed);
            }
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
    let candidate: PathBuf = if p.is_absolute() {
        // Must be inside allowlist OR be a file inside cwd
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut ok = false;
        for a in &allowed {
            if p.starts_with(a) {
                ok = true;
                break;
            }
        }
        if p.starts_with(&cwd) {
            ok = true;
        }
        if !ok {
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
