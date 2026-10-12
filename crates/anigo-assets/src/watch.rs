//! Watcher opcional de raizes linked (feature `watch`, `notify` 8). Watchers PERDEM eventos (limites do inotify,
//! discos de rede, sleep): trate-o como um acelerador e mantenha o `scan` na abertura e um botao manual.

use std::time::Duration;

use crate::error::{AssetError, Result};
use crate::library::Library;
use crate::model::RootId;
use crate::scan::ScanReport;

/// Eventos do watcher.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchEvent {
    /// Houve atividade no disco (quantos eventos brutos no lote).
    Changed { raw_events: usize },
    /// Resultado do scan incremental disparado apos o silencio.
    Scanned(Box<ScanReport>),
    Error(String),
}

#[cfg(feature = "watch")]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Instant;

    use crate::scan::ScanOptions;
    use crossbeam_channel::{unbounded, Receiver};
    use notify::{RecursiveMode, Watcher as _};

    /// Handle do watcher; ao ser descartado, para o monitoramento.
    pub struct Watcher {
        events: Receiver<WatchEvent>,
        stop: Arc<AtomicBool>,
        _watcher: notify::RecommendedWatcher,
        thread: Option<JoinHandle<()>>,
    }

    impl Watcher {
        pub fn events(&self) -> &Receiver<WatchEvent> {
            &self.events
        }
    }

    impl Drop for Watcher {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Library {
        /// Observa a raiz linked `root`; apos `debounce` sem novos eventos roda um `scan` incremental.
        pub fn watch(&self, root: RootId, debounce: Duration) -> Result<Watcher> {
            let info = self.root(root)?;
            if info.mode != crate::model::RootMode::Linked {
                return Err(AssetError::InvalidInput("watch so vale para raizes linked".into()));
            }
            if !info.online {
                return Err(AssetError::Conflict("raiz offline".into()));
            }
            let (raw_tx, raw_rx) = unbounded::<()>();
            let (ev_tx, ev_rx) = unbounded::<WatchEvent>();
            let err_tx = ev_tx.clone();
            let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) => {
                    use notify::EventKind::*;
                    if matches!(ev.kind, Create(_) | Modify(_) | Remove(_)) {
                        let _ = raw_tx.send(());
                    }
                }
                Err(e) => {
                    let _ = err_tx.send(WatchEvent::Error(e.to_string()));
                }
            })
            .map_err(|e| AssetError::Unsupported(format!("notify: {e}")))?;
            watcher.watch(&info.path, RecursiveMode::Recursive).map_err(|e| AssetError::Unsupported(format!("notify: {e}")))?;

            let stop = Arc::new(AtomicBool::new(false));
            let stop_t = stop.clone();
            let lib = self.clone();
            let thread = std::thread::Builder::new()
                .name("anigo-watch".into())
                .spawn(move || {
                    let mut pending = 0usize;
                    let mut last = Instant::now();
                    while !stop_t.load(Ordering::Relaxed) {
                        match raw_rx.recv_timeout(Duration::from_millis(50)) {
                            Ok(()) => {
                                pending += 1;
                                last = Instant::now();
                                while raw_rx.try_recv().is_ok() {
                                    pending += 1;
                                }
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                        }
                        if pending > 0 && last.elapsed() >= debounce {
                            let n = std::mem::take(&mut pending);
                            let _ = ev_tx.send(WatchEvent::Changed { raw_events: n });
                            let job = lib.scan(root, ScanOptions::default());
                            while !job.is_finished() {
                                if stop_t.load(Ordering::Relaxed) {
                                    job.cancel();
                                }
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            match job.join() {
                                Ok(rep) => {
                                    let _ = ev_tx.send(WatchEvent::Scanned(Box::new(rep)));
                                }
                                Err(e) => {
                                    let _ = ev_tx.send(WatchEvent::Error(e.to_string()));
                                }
                            }
                            // os proprios arquivos que o scan escreveu em `home` nao estao na raiz observada
                            while raw_rx.try_recv().is_ok() {}
                        }
                    }
                })
                .map_err(|e| AssetError::Internal(format!("thread do watcher: {e}")))?;
            Ok(Watcher { events: ev_rx, stop, _watcher: watcher, thread: Some(thread) })
        }
    }
}

#[cfg(feature = "watch")]
pub use imp::Watcher;

#[cfg(not(feature = "watch"))]
mod imp {
    use super::*;
    /// Marcador: a feature `watch` esta desligada.
    pub struct Watcher {
        _p: (),
    }
    impl Watcher {
        pub fn events(&self) -> &crossbeam_channel::Receiver<WatchEvent> {
            unreachable!("Watcher nao pode ser construido sem a feature `watch`")
        }
    }
    impl Library {
        pub fn watch(&self, _root: RootId, _debounce: Duration) -> Result<Watcher> {
            Err(AssetError::Unsupported("feature `watch` desligada".into()))
        }
    }
}

#[cfg(not(feature = "watch"))]
pub use imp::Watcher;
