//! Valida TODOS os `.wgsl` de `crates/anigo-renderer/shaders/` com `naga`
//! (parse + validação completa), sem precisar de GPU. Falhas reportam
//! arquivo e erro formatado com o trecho do código.
//!
//! As capabilities são as de um device sem features opcionais
//! (`Capabilities::empty()`), que é exatamente o que o `HeadlessRenderer`
//! solicita (`required_features: Features::empty()`).

use std::fs;
use std::path::{Path, PathBuf};

use naga::valid::{Capabilities, ValidationFlags, Validator};

/// Shaders com falha CONHECIDA e aceita: `(caminho relativo a shaders/, motivo)`.
/// O teste ainda os parseia/valida, mas só reporta (não falha) — e falha se um
/// `known_issue` passar a validar, para a entrada não apodrecer.
/// Vazio hoje: todos os shaders validam.
const KNOWN_ISSUES: &[(&str, &str)] = &[];

fn shaders_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn collect_wgsl(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("não consegui ler {}: {e}", dir.display()))
        .map(|e| e.expect("entrada de diretório").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_wgsl(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            out.push(path);
        }
    }
}

/// `Ok(entry points)` ou `Err(mensagem formatada com arquivo e erro)`.
fn check_shader(path: &Path, relative: &str) -> Result<Vec<String>, String> {
    let source = fs::read_to_string(path).map_err(|e| format!("{relative}: leitura falhou: {e}"))?;
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| {
        format!("{relative}: erro de parse WGSL:\n{}", e.emit_to_string(&source))
    })?;
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("{relative}: erro de validação naga:\n{}", e.emit_to_string(&source)))?;
    Ok(module.entry_points.iter().map(|ep| ep.name.clone()).collect())
}

#[test]
fn all_wgsl_shaders_validate_with_naga() {
    let root = shaders_dir();
    let mut files = Vec::new();
    collect_wgsl(&root, &mut files);
    assert!(
        files.len() >= 6,
        "esperava ao menos 6 shaders .wgsl em {}, achei {}",
        root.display(),
        files.len()
    );

    let mut failures = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let known = KNOWN_ISSUES.iter().find(|(file, _)| *file == relative);
        match (check_shader(path, &relative), known) {
            (Ok(entry_points), None) => {
                // Sem entry point = módulo-biblioteca (funções incluídas por outros).
                let kind = if entry_points.is_empty() {
                    "(biblioteca, sem entry point)".to_string()
                } else {
                    entry_points.join(", ")
                };
                eprintln!("ok    {relative}: {kind}");
            }
            (Ok(_), Some((_, why))) => failures.push(format!(
                "{relative}: marcado como known_issue ({why}) mas agora VALIDA — remova da lista"
            )),
            (Err(error), None) => failures.push(error),
            (Err(error), Some((_, why))) => {
                eprintln!("KNOWN {relative}: {why}\n{error}");
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} shader(s) com problema:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn contract_embedded_shaders_are_the_files_on_disk() {
    // Os shaders embutidos pelo contrato (o que o renderer realmente compila)
    // são os mesmos arquivos validados acima.
    use anigo_renderer::render_contract as contract;
    let root = shaders_dir();
    for (name, embedded) in [
        ("cel_shading.wgsl", contract::CEL_SHADING_WGSL),
        ("inverted_hull.wgsl", contract::INVERTED_HULL_WGSL),
        ("morph_sparse_compute.wgsl", contract::MORPH_SPARSE_COMPUTE_WGSL),
        ("postprocess_dof.wgsl", contract::POSTPROCESS_DOF_WGSL),
    ] {
        let on_disk = fs::read_to_string(root.join(name)).unwrap();
        assert_eq!(embedded, on_disk, "{name} embutido diverge do arquivo");
    }
}
