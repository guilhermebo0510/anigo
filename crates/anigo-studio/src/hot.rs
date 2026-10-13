//! Hot reload de shaders: observa o arquivo WGSL no disco e avisa o app.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct ShaderWatcher {
    // O watcher precisa viver tanto quanto o canal.
    _watcher: RecommendedWatcher,
    rx: Receiver<()>,
    pub path: PathBuf,
}

impl ShaderWatcher {
    /// Observa `path` (arquivo). Erro se o arquivo não existe ou o backend de notificação falha.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(format!("shader não encontrado: {}", path.display()));
        }
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if ev.kind.is_modify() || ev.kind.is_create() {
                    let _ = tx.send(());
                }
            }
        })
        .map_err(|e| e.to_string())?;
        // observa o diretório pai: editores que salvam por rename quebram o watch do arquivo
        let dir = path.parent().unwrap_or(Path::new("."));
        watcher.watch(dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string())?;
        Ok(Self {
            _watcher: watcher,
            rx,
            path,
        })
    }

    /// Há mudança pendente? (drena o canal; várias notificações viram uma).
    pub fn take_changed(&self) -> bool {
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }
        changed
    }

    pub fn read(&self) -> Result<String, String> {
        std::fs::read_to_string(&self.path).map_err(|e| e.to_string())
    }
}

/// Caminho do shader do workbench no checkout (dev). `None` fora do repositório.
pub fn dev_shader_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ANIGO_SHADER_DIR") {
        let p = Path::new(&dir).join("workbench.wgsl");
        return p.is_file().then_some(p);
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/workbench/workbench.wgsl");
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn detects_file_modification() {
        let dir = std::env::temp_dir().join(format!("anigo_hot_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.wgsl");
        std::fs::write(&file, "// v1").unwrap();
        let w = ShaderWatcher::new(&file).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&file, "// v2").unwrap();
        let start = Instant::now();
        let mut seen = false;
        while start.elapsed() < Duration::from_secs(5) {
            if w.take_changed() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen, "mudança de arquivo não foi observada");
        assert_eq!(w.read().unwrap(), "// v2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(ShaderWatcher::new("/nonexistent/x.wgsl").is_err());
    }

    #[test]
    fn dev_shader_path_resolves_in_repo() {
        assert!(dev_shader_path().is_some());
    }
}
