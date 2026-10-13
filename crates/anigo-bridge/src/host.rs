//! Ponte entre as threads de conexão e a thread "dona" do estado (a UI do studio).
//!
//! A conexão cria um [`HostRequest`] com um canal de resposta e o envia por um canal LIMITADO; a UI chama
//! [`HostQueue::drain`] no início do quadro, executa e responde ([`Reply::send`]). A UI é acordada com a
//! função `wake` (ex.: `ctx.request_repaint()`).
//!
//! Se a UI não responder no prazo da ação, o pedido é marcado `cancelled` e o cliente recebe `-32002`; quem
//! drena DEVE ignorar pedidos cancelados (nunca aplicar uma mutação tarde demais).

use crate::protocol::{code, BridgeError, BridgeResult, Request};
use crate::server::Backend;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

pub struct Reply(Sender<BridgeResult>);

impl Reply {
    /// Responde (ignora silenciosamente se o cliente já desistiu).
    pub fn send(self, result: BridgeResult) {
        let _ = self.0.send(result);
    }
}

pub struct HostRequest {
    pub id: String,
    pub action: String,
    pub params: Value,
    pub reply: Reply,
    pub cancelled: Arc<AtomicBool>,
}

impl HostRequest {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Relaxed)
    }
}

/// Lado da UI.
pub struct HostQueue {
    rx: Receiver<HostRequest>,
}

impl HostQueue {
    /// Até `max` pedidos pendentes, descartando os já cancelados (o cliente já recebeu o timeout).
    pub fn drain(&self, max: usize) -> Vec<HostRequest> {
        let mut out = Vec::new();
        while out.len() < max {
            match self.rx.try_recv() {
                Ok(r) if r.is_cancelled() => {}
                Ok(r) => out.push(r),
                Err(_) => break,
            }
        }
        out
    }

    pub fn pending(&self) -> usize {
        self.rx.len()
    }
}

/// Prazo de cada ação (tempo para a UI responder).
pub fn action_timeout(action: &str) -> Duration {
    match action {
        "SCREENSHOT" => Duration::from_secs(20),
        "PLACE_ASSET" | "RENDER_FRAME" | "LIST_ASSETS" => Duration::from_secs(20),
        "GET_SNAPSHOT" | "GET_PROJECT" => Duration::from_secs(15),
        _ => Duration::from_secs(10),
    }
}

/// Lado das conexões (implementa [`Backend`]).
pub struct ChannelBackend {
    tx: Sender<HostRequest>,
    wake: Arc<dyn Fn() + Send + Sync>,
    timeout: fn(&str) -> Duration,
}

/// Cria o par (backend para o servidor, fila para a UI). `capacity` limita pedidos pendentes.
pub fn channel(capacity: usize, wake: Arc<dyn Fn() + Send + Sync>) -> (ChannelBackend, HostQueue) {
    let (tx, rx) = bounded(capacity.max(1));
    (ChannelBackend { tx, wake, timeout: action_timeout }, HostQueue { rx })
}

impl ChannelBackend {
    /// Troca a política de prazos (testes).
    pub fn with_timeout(mut self, f: fn(&str) -> Duration) -> Self {
        self.timeout = f;
        self
    }
}

impl Backend for ChannelBackend {
    fn call(&self, req: &Request) -> BridgeResult {
        let (rtx, rrx) = bounded(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let hr = HostRequest {
            id: req.id_string(),
            action: req.action.clone(),
            params: req.params.clone(),
            reply: Reply(rtx),
            cancelled: Arc::clone(&cancelled),
        };
        match self.tx.try_send(hr) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(BridgeError::new(code::BUSY, "fila da UI cheia; tente novamente")),
            Err(TrySendError::Disconnected(_)) => return Err(BridgeError::new(code::SHUTTING_DOWN, "o app está encerrando")),
        }
        (self.wake)();
        let limit = (self.timeout)(&req.action);
        match rrx.recv_timeout(limit) {
            Ok(r) => r,
            Err(_) => {
                cancelled.store(true, Relaxed);
                Err(BridgeError::new(
                    code::UI_TIMEOUT,
                    format!("a UI não respondeu em {} s (janela minimizada/oculta ou ocupada?); a ação não foi aplicada", limit.as_secs()),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(action: &str) -> Request {
        serde_json::from_value(json!({ "id": "1", "action": action })).unwrap()
    }

    #[test]
    fn request_reaches_the_host_and_the_reply_comes_back() {
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = Arc::clone(&woken);
        let (backend, queue) = channel(4, Arc::new(move || {
            w.fetch_add(1, Relaxed);
        }));
        let h = std::thread::spawn(move || backend.call(&req("PING2")));
        let mut got = Vec::new();
        for _ in 0..200 {
            got = queue.drain(8);
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].action, "PING2");
        assert!(woken.load(Relaxed) >= 1);
        got.remove(0).reply.send(Ok(json!({"ok": true})));
        assert_eq!(h.join().unwrap().unwrap()["ok"], true);
    }

    #[test]
    fn timeout_marks_cancelled_and_the_late_request_is_dropped() {
        let (backend, queue) = channel(4, Arc::new(|| {}));
        let backend = backend.with_timeout(|_| Duration::from_millis(60));
        let err = backend.call(&req("APPLY_COMMAND")).unwrap_err();
        assert_eq!(err.code, code::UI_TIMEOUT);
        // a UI "acorda tarde": o pedido cancelado nunca chega
        assert!(queue.drain(8).is_empty(), "mutação tardia deve ser descartada");
    }

    #[test]
    fn full_queue_is_busy_and_closed_queue_is_shutting_down() {
        let (backend, queue) = channel(1, Arc::new(|| {}));
        let backend = Arc::new(backend.with_timeout(|_| Duration::from_millis(80)));
        let b2 = Arc::clone(&backend);
        let first = std::thread::spawn(move || b2.call(&req("A")));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(backend.call(&req("B")).unwrap_err().code, code::BUSY);
        let _ = first.join();
        drop(queue);
        assert_eq!(backend.call(&req("C")).unwrap_err().code, code::SHUTTING_DOWN);
    }
}
