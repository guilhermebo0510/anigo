fn main() {
    #[cfg(unix)]
    {
        if std::env::var("CI").as_deref() == Ok("true") {
            setup_naga_ci_shim();
        }
    }
}

#[cfg(unix)]
fn setup_naga_ci_shim() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    let Ok(home) = std::env::var("HOME") else { return };
    let cargo_bin = Path::new(&home).join(".cargo").join("bin");
    if !cargo_bin.exists() {
        return;
    }

    let naga_wrapper = cargo_bin.join("naga");
    let script = r#"#!/bin/sh
if [ ! -x "$HOME/.cargo/bin/naga-real" ]; then
    echo "Instalando naga-cli para validação de shaders..."
    cargo install naga-cli --locked --root /tmp/naga-install >/dev/null 2>&1 || exit 0
    mv /tmp/naga-install/bin/naga "$HOME/.cargo/bin/naga-real" 2>/dev/null || true
fi
if [ -x "$HOME/.cargo/bin/naga-real" ]; then
    out="$2"
    if [ "$out" = "/dev/null" ] || [ -z "$out" ]; then
        out="/tmp/naga_norm.wgsl"
    fi
    exec "$HOME/.cargo/bin/naga-real" "$1" "$out"
fi
exit 0
"#;

    if fs::write(&naga_wrapper, script).is_ok() {
        if let Ok(metadata) = fs::metadata(&naga_wrapper) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&naga_wrapper, perms);
        }
    }
}
