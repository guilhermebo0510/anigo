//! Infra de trabalhos em background: thread + canal de eventos (crossbeam) + cancelamento.
//!
//! Sem `tokio`: a carga e CPU/disco e a UI (egui) e sincrona. A UI faz `job.events().try_iter()` a cada
//! quadro e chama `job.cancel()` quando o usuario cancela. `join()` nunca entra em `panic`: um `panic` no
//! worker vira `AssetError::Internal`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::error::{AssetError, Result};

/// Token de cancelamento cooperativo (barato de clonar).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(AssetError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Lado do worker: emite eventos e consulta cancelamento.
pub struct JobCtx<E> {
    tx: Sender<E>,
    pub cancel: CancelToken,
}

impl<E> Clone for JobCtx<E> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone(), cancel: self.cancel.clone() }
    }
}

impl<E> JobCtx<E> {
    /// Emite um evento; ignora silenciosamente se o `Job` foi descartado (ninguem ouvindo).
    pub fn emit(&self, e: E) {
        let _ = self.tx.send(e);
    }
}

/// Handle de um trabalho em background com eventos `E` e resultado final `R`.
pub struct Job<E, R> {
    rx: Receiver<E>,
    cancel: CancelToken,
    handle: Option<JoinHandle<Result<R>>>,
}

impl<E: Send + 'static, R: Send + 'static> Job<E, R> {
    /// Inicia `work` em uma thread dedicada (`name` aparece em debuggers).
    pub(crate) fn spawn<F>(name: &str, work: F) -> Job<E, R>
    where
        F: FnOnce(JobCtx<E>) -> Result<R> + Send + 'static,
    {
        let (tx, rx) = unbounded();
        let cancel = CancelToken::new();
        let ctx = JobCtx { tx, cancel: cancel.clone() };
        let handle = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || match catch_unwind(AssertUnwindSafe(|| work(ctx))) {
                Ok(r) => r,
                Err(p) => Err(AssetError::Internal(panic_message(&p))),
            })
            .expect("falha ao criar thread de trabalho");
        Job { rx, cancel, handle: Some(handle) }
    }

    /// Canal de eventos (nao bloqueante com `try_iter`).
    pub fn events(&self) -> &Receiver<E> {
        &self.rx
    }

    /// Pede cancelamento; o worker para no proximo ponto de checagem.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Token clonavel para cancelar de outra thread.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().map(|h| h.is_finished()).unwrap_or(true)
    }

    /// Espera o termino e devolve o resultado. Eventos nao consumidos sao descartados.
    pub fn join(mut self) -> Result<R> {
        match self.handle.take() {
            Some(h) => match h.join() {
                Ok(r) => r,
                Err(p) => Err(AssetError::Internal(panic_message(&p))),
            },
            None => Err(AssetError::Internal("job ja finalizado".into())),
        }
    }
}

impl<E, R> Drop for Job<E, R> {
    fn drop(&mut self) {
        // Descartar o handle sem `join` cancela o trabalho (nao deixa threads orfas processando).
        if self.handle.is_some() {
            self.cancel.cancel();
        }
    }
}

pub(crate) fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        format!("panic: {s}")
    } else if let Some(s) = p.downcast_ref::<String>() {
        format!("panic: {s}")
    } else {
        "panic desconhecido".to_string()
    }
}

/// Executa `f` capturando `panic` e convertendo em `AssetError::Internal`.
pub(crate) fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => Err(AssetError::Internal(panic_message(&p))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_runs_and_reports() {
        let job: Job<u32, u32> = Job::spawn("t", |ctx| {
            for i in 0..3 {
                ctx.emit(i);
            }
            Ok(42)
        });
        let r = job.join().unwrap();
        assert_eq!(r, 42);
    }

    #[test]
    fn job_events_arrive() {
        let job: Job<u32, ()> = Job::spawn("t", |ctx| {
            ctx.emit(1);
            ctx.emit(2);
            Ok(())
        });
        let first = job.events().recv().unwrap();
        let second = job.events().recv().unwrap();
        assert_eq!((first, second), (1, 2));
        job.join().unwrap();
    }

    #[test]
    fn panic_becomes_error() {
        let job: Job<(), ()> = Job::spawn("t", |_| -> Result<()> { panic!("boom") });
        match job.join() {
            Err(AssetError::Internal(m)) => assert!(m.contains("boom")),
            other => panic!("esperava Internal, veio {other:?}"),
        }
    }

    #[test]
    fn cancel_is_observed() {
        let job: Job<(), u32> = Job::spawn("t", |ctx| {
            let mut n = 0;
            while !ctx.cancel.is_cancelled() {
                n += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(n)
        });
        job.cancel();
        assert!(job.join().is_ok());
    }

    #[test]
    fn guard_catches_panic() {
        let r: Result<()> = guard(|| panic!("x"));
        assert!(matches!(r, Err(AssetError::Internal(_))));
    }
}
