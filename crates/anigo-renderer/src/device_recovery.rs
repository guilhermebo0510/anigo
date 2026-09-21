//! ANIGO — tolerância a falhas e recuperação de perda de device wgpu (Fase 1, issue #11).
//!
//! Cenários cobertos: desconexão de monitor, troca de GPU dedicada/integrada em
//! notebooks, suspensão do SO e crash silencioso do driver gráfico. Antes,
//! qualquer um desses estados deixava o `wgpu` irrecuperável e forçava o
//! fechamento do ANIGO. Aqui a perda é interceptada:
//!
//! 1. `device.on_uncaptured_error` instala um callback que encaminha cada erro
//!    por um **canal MPSC estruturado** ([`UncapturedErrorBus`]) para o módulo
//!    de diagnósticos — nada de pânico no caminho crítico;
//! 2. [`ReconnectSchedule`] define o backoff exponencial de reconexão
//!    (500ms → 1s → 2s) usado pelo renderer (Rust e TypeScript seguem o mesmo
//!    calendário congelado);
//! 3. [`PresentationMode`] é a configuração explícita do modo de apresentação
//!    (`Fifo` = VSync, `Immediate` = menor latência, `Mailbox` quando suportado);
//! 4. o `HeadlessRenderer` expõe `recreate_device_and_swapchain()` para
//!    re-solicitar adapter/device, reinstanciar os PSOs e re-alocar os buffers
//!    essenciais a partir do snapshot canônico (o estado do personagem vive no
//!    núcleo; a GPU apenas o reapresenta).
//!
//! O lado TypeScript (`webgpu_renderer.ts`) observa a Promise `device.lost`,
//! diferencia queda esperada (`reason === "destroyed"`) de crash
//! (`reason === "unknown"`/`"connection_lost"`) e reconecta com o mesmo
//! [`ReconnectSchedule`], exibindo overlay de reconexão em vez de travar a UI.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::diagnostics;

// ---------------------------------------------------------------------------
// Modos de apresentação
// ---------------------------------------------------------------------------

/// Modo de apresentação do swapchain (issue #11: configuração explícita).
///
/// O mesmo valor atravessa as duas fronteiras: o Rust (wgpu) e o viewport
/// TypeScript (o `presentMode` do `canvas context.configure`), então a enum é
/// serializada em minúsculo e o mapeamento para cada API vive aqui — nunca um
/// literal solto no renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresentationMode {
    /// VSync ligado: a GPU aguarda o próximo vertical blank (padrão estável).
    Fifo,
    /// Menor latência: apresenta assim que o frame estiver pronto.
    Immediate,
    /// Um único buffer; apresenta sempre o frame mais recente (quando a GPU
    /// suporta; o driver resolve o fallback).
    Mailbox,
}

impl PresentationMode {
    /// Todos os modos válidos (ordem canônica para UI e testes).
    pub const ALL: [Self; 3] = [Self::Fifo, Self::Immediate, Self::Mailbox];

    /// Parse tolerante (case-insensitive, espaços ignorados).
    ///
    /// `None` quando o modo não é suportado — o chamador deve degradar para
    /// [`PresentationMode::Fifo`] e reportar, nunca pânico.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fifo" => Some(Self::Fifo),
            "immediate" => Some(Self::Immediate),
            "mailbox" => Some(Self::Mailbox),
            _ => None,
        }
    }

    /// Nome canônico em minúsculo (`"fifo"`, `"immediate"`, `"mailbox"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fifo => "fifo",
            Self::Immediate => "immediate",
            Self::Mailbox => "mailbox",
        }
    }

    /// String `presentMode` aceita pelo `GPUCanvasContext.configure` do
    /// viewport (mesmo vocabulário do WGSL contract — sem tradução).
    pub fn webgpu_present_mode(self) -> &'static str {
        self.as_str()
    }

    /// Equivalente wgpu (Rust). `Mailbox` é aceito pelo driver quando o backend
    /// suporta; caso contrário o driver aplica o próprio fallback.
    pub fn wgpu_present_mode(self) -> wgpu::PresentMode {
        match self {
            Self::Fifo => wgpu::PresentMode::Fifo,
            Self::Immediate => wgpu::PresentMode::Immediate,
            Self::Mailbox => wgpu::PresentMode::Mailbox,
        }
    }
}

impl Default for PresentationMode {
    fn default() -> Self {
        Self::Fifo
    }
}

impl std::fmt::Display for PresentationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Canal MPSC estruturado de erros de device
// ---------------------------------------------------------------------------

/// Registro estruturado de um erro de device interceptado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UncapturedErrorRecord {
    /// Origem do evento: `"uncaptured_error"`, `"validation"`, `"device_lost"`
    /// (o campo é texto estável para telemetria, não enum: um lado novo pode
    /// aparecer sem quebrar a deserialização de dados antigos).
    pub source: String,
    /// Mensagem de erro (texto do `wgpu::DeviceError`/driver).
    pub message: String,
    /// Milissegundos desde o epoch UNIX (telemetria comparável entre Rust ⇄ TS).
    pub at_ms: u64,
}

impl UncapturedErrorRecord {
    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }

    /// Erro entregue pelo callback `on_uncaptured_error` do wgpu.
    pub fn uncaptured(message: impl Into<String>) -> Self {
        Self {
            source: "uncaptured_error".to_string(),
            message: message.into(),
            at_ms: Self::now_ms(),
        }
    }

    /// Perda explícita de device (listener de `device.lost`/`poll`).
    pub fn device_lost(reason: impl Into<String>) -> Self {
        Self {
            source: "device_lost".to_string(),
            message: reason.into(),
            at_ms: Self::now_ms(),
        }
    }
}

/// Últimos erros mantidos no anel global (capacidade pequena: telemetria, não log).
const RECENT_CAPACITY: usize = 8;

fn recent_ring() -> &'static Mutex<Vec<UncapturedErrorRecord>> {
    static RING: OnceLock<Mutex<Vec<UncapturedErrorRecord>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        // Um pânico em outro thread não pode esconder erros de device: o
        // anel continua íntegro, seguimos com o guard envenenado.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Pusha no anel global (sem `panic!`, sem `unwrap`).
fn push_recent(record: &UncapturedErrorRecord) {
    let mut ring = lock(recent_ring());
    ring.push(record.clone());
    if ring.len() > RECENT_CAPACITY {
        let excess = ring.len() - RECENT_CAPACITY;
        ring.drain(..excess);
    }
}

/// Últimos erros de device interceptados (mais antigos primeiro).
pub fn recent_errors() -> Vec<UncapturedErrorRecord> {
    lock(recent_ring()).clone()
}

/// Limpa o anel (após o consumidor reconhecer os eventos).
#[cfg(test)]
pub fn clear_recent_errors() {
    lock(recent_ring()).clear();
}

/// Bus MPSC que liga o callback `device.on_uncaptured_error` ao módulo de
/// diagnósticos.
///
/// O callback roda em thread interna do wgpu e **nunca** pode bloquear: ele só
/// faz `send` em um canal (não bloqueante na prática, o receiver é
/// `unbounded`). O dreno ([`drain_to_diagnostics`]) acontece no ritmo do
/// renderer — uma vez por quadro, no `render_scene`.
pub struct UncapturedErrorBus {
    sender: std::sync::mpsc::Sender<UncapturedErrorRecord>,
    receiver: Mutex<std::sync::mpsc::Receiver<UncapturedErrorRecord>>,
}

impl UncapturedErrorBus {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            sender,
            receiver: Mutex::new(receiver),
        }
    }

    /// Instala o callback no device wgpu. O closure clona o `sender` — o
    /// device pode viver mais ou menos que o bus, e cada clone é um
    /// produtor legítimo do canal (MPSC de verdade).
    pub fn install_on(&self, device: &wgpu::Device) {
        let sender = self.sender.clone();
        device.on_uncaptured_error(Some(Box::new(move |error: &wgpu::DeviceError| {
            let _ = sender.send(UncapturedErrorRecord::uncaptured(error.to_string()));
        })));
    }

    /// Entrada de teste/simulação: injeta um registro no **mesmo** canal que o
    /// callback do device usa (a aceitação #1 — falha forçada sem crash — é
    /// verificada por aqui em ambiente sem GPU).
    pub fn inject(&self, record: UncapturedErrorRecord) {
        let _ = self.sender.send(record);
    }

    /// Quantos erros ainda aguardam dreno (telemetria rápida).
    pub fn pending(&self) -> usize {
        lock(&self.receiver).try_iter().count()
    }

    /// Lê todos os erros pendentes e os encaminha ao módulo de diagnósticos
    /// (código `gpu_device_error`, o mesmo que o contrato congela). Devolve a
    /// quantidade registrada.
    pub fn drain_to_diagnostics(&self) -> usize {
        let mut count = 0;
        let receiver = lock(&self.receiver);
        for record in receiver.iter() {
            push_recent(&record);
            diagnostics::report_with_detail(
                "gpu_device_error",
                "erro de device wgpu interceptado (nenhum crash)",
                Some(format!("[{}] {}", record.source, record.message)),
            );
            count += 1;
        }
        count
    }
}

impl Default for UncapturedErrorBus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Backoff exponencial de reconexão
// ---------------------------------------------------------------------------

/// Calendário de tentativas de reconexão (issue #11: 500ms, 1s, 2s).
///
/// `delay(n) = min(base * 2^n, max)` — o mesmo calendário que o TypeScript
/// (`webgpu_renderer.ts`) segue ao observar `device.lost`, para que a UI e o
/// headless reajam no mesmo ritmo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconnectSchedule {
    pub base_ms: u64,
    pub max_ms: u64,
    pub max_attempts: u32,
}

impl Default for ReconnectSchedule {
    fn default() -> Self {
        Self {
            base_ms: 500,
            max_ms: 2000,
            max_attempts: 3,
        }
    }
}

impl ReconnectSchedule {
    /// Atraso antes da tentativa `attempt` (0-indexada).
    ///
    /// `None` quando o calendário se esgotou — o chamador deve reportar
    /// `device_unavailable` e parar de tentar (o retry manual é do usuário).
    pub fn delay_for_attempt(&self, attempt: u32) -> Option<Duration> {
        if attempt >= self.max_attempts {
            return None;
        }
        // `base * 2^attempt` com saturação: o excedente vira o teto.
        let shifted = self
            .base_ms
            .checked_mul(1u64 << attempt.min(63))
            .and_then(|value| value.checked_min(self.max_ms))
            .unwrap_or(self.max_ms);
        Some(Duration::from_millis(shifted))
    }

    /// Atrasos de todas as tentativas (a sequência congelada do issue: 500/1000/2000).
    pub fn delays(&self) -> Vec<Duration> {
        (0..self.max_attempts).filter_map(|attempt| self.delay_for_attempt(attempt)).collect()
    }

    /// `true` após `max_attempts` tentativas.
    pub fn is_exhausted(&self, attempt: u32) -> bool {
        attempt >= self.max_attempts
    }

    /// Orçamento total do backoff (soma dos atrasos) — aparece na notificação
    /// de recuperação como teto do tempo de reconexão.
    pub fn total_budget(&self) -> Duration {
        self.delays().iter().copied().sum()
    }
}

// ---------------------------------------------------------------------------
// Relatório de recriação
// ---------------------------------------------------------------------------

/// Resultado de uma `recreate_device_and_swapchain` bem-sucedida (telemetria
/// da notificação de rodapé: evento + tempo de recuperação).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecreationReport {
    pub adapter_name: String,
    pub backend: String,
    /// Tempo total de recriação (ms) — o número que a UI mostra ao usuário.
    pub duration_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::test_guard;

    // ── PresentationMode ────────────────────────────────────────────────

    #[test]
    fn presentation_mode_parsing_is_case_insensitive_and_tolerant() {
        assert_eq!(PresentationMode::parse("fifo"), Some(PresentationMode::Fifo));
        assert_eq!(PresentationMode::parse("IMMEDIATE"), Some(PresentationMode::Immediate));
        assert_eq!(PresentationMode::parse("  Mailbox "), Some(PresentationMode::Mailbox));
        assert_eq!(PresentationMode::parse("vsync"), None);
        assert_eq!(PresentationMode::parse(""), None);
        assert_eq!(PresentationMode::parse("double_buffer"), None);
    }

    #[test]
    fn presentation_mode_maps_to_webgpu_and_wgpu_identically() {
        for mode in PresentationMode::ALL {
            // O mesmo nome serve às duas APIs (contrato compartilhado).
            assert_eq!(mode.webgpu_present_mode(), mode.as_str());
            assert_eq!(
                mode.wgpu_present_mode(),
                match mode {
                    PresentationMode::Fifo => wgpu::PresentMode::Fifo,
                    PresentationMode::Immediate => wgpu::PresentMode::Immediate,
                    PresentationMode::Mailbox => wgpu::PresentMode::Mailbox,
                }
            );
        }
        assert_eq!(PresentationMode::default(), PresentationMode::Fifo);
        let serialized = serde_json::to_string(&PresentationMode::Immediate).unwrap();
        assert_eq!(serialized, "\"immediate\"");
        let parsed: PresentationMode = serde_json::from_str("\"mailbox\"").unwrap();
        assert_eq!(parsed, PresentationMode::Mailbox);
    }

    // ── ReconnectSchedule ───────────────────────────────────────────────

    #[test]
    fn reconnect_schedule_is_the_frozen_500_1000_2000_sequence() {
        let schedule = ReconnectSchedule::default();
        assert_eq!(
            schedule.delays(),
            vec![
                Duration::from_millis(500),
                Duration::from_millis(1000),
                Duration::from_millis(2000)
            ]
        );
        assert_eq!(schedule.total_budget(), Duration::from_millis(4500));
        assert!(!schedule.is_exhausted(2));
        assert!(schedule.is_exhausted(3));
        assert_eq!(schedule.delay_for_attempt(3), None);
        assert_eq!(schedule.delay_for_attempt(99), None);
    }

    #[test]
    fn reconnect_schedule_caps_at_max_ms() {
        let schedule = ReconnectSchedule {
            base_ms: 500,
            max_ms: 1500,
            max_attempts: 5,
        };
        let delays: Vec<u64> = schedule
            .delays()
            .iter()
            .map(|d| d.as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![500, 1000, 1500, 1500, 1500]);
    }

    // ── UncapturedErrorBus (canal MPSC) ─────────────────────────────────

    #[test]
    fn error_bus_forwards_injected_errors_to_diagnostics() {
        let _guard = test_guard();
        diagnostics::clear();
        clear_recent_errors();

        let bus = UncapturedErrorBus::new();
        assert_eq!(bus.pending(), 0);

        // A falha forçada entra pelo mesmo canal do callback `on_uncaptured_error`.
        bus.inject(UncapturedErrorRecord::uncaptured(
            "simulated driver crash (device lost)",
        ));
        bus.inject(UncapturedErrorRecord::device_lost("connection_lost"));
        assert_eq!(bus.pending(), 2);

        let drained = bus.drain_to_diagnostics();
        assert_eq!(drained, 2);
        assert_eq!(bus.pending(), 0);

        // O módulo de diagnósticos registrou pelo código estável do contrato.
        let summary = diagnostics::summary();
        assert!(summary.codes.contains(&"gpu_device_error".to_string()));
        let records = recent_errors();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].source, "uncaptured_error");
        assert!(records[0].message.contains("simulated driver crash"));
        assert_eq!(records[1].source, "device_lost");

        diagnostics::clear();
        clear_recent_errors();
    }

    #[test]
    fn error_bus_ring_is_bounded() {
        clear_recent_errors();
        let bus = UncapturedErrorBus::new();
        for index in 0..(RECENT_CAPACITY + 4) {
            bus.inject(UncapturedErrorRecord::uncaptured(format!("erro {index}")));
        }
        bus.drain_to_diagnostics();
        assert_eq!(recent_errors().len(), RECENT_CAPACITY);
        // O mais antigo saiu, o mais recente ficou.
        assert!(recent_errors()
            .last()
            .unwrap()
            .message
            .contains(&format!("erro {}", RECENT_CAPACITY + 3)));
        diagnostics::clear();
        clear_recent_errors();
    }
}
