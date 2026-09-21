//! ANIGO — diagnóstico estruturado do renderer (P1 — Robustez, item 2).
//!
//! O headless não fala com a UI, então falha silenciosa era o modo padrão de
//! erro: um `anyhow::Error` no log ou nada. Aqui cada problema vira um
//! diagnóstico com **código estável** (o mesmo catálogo que o viewport usa,
//! congelado em `contracts/fixtures/render_contract_v1.json`), severidade,
//! contagem e detalhe.
//!
//! Uso:
//! ```ignore
//! use anigo_renderer::diagnostics;
//! diagnostics::report("device_unavailable", "nenhum adaptador wgpu disponível");
//! let summary = diagnostics::summary(); // telemetria: errors/warnings/codes
//! ```
//!
//! Um código fora do catálogo é registrado como `contract_drift` — o mesmo
//! tratamento que o lado TypeScript dá ao problema (`DiagnosticContractError`).

use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

use crate::render_contract;

/// Severidade de um diagnóstico (espelha `DiagnosticSeverity` do TypeScript).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    /// Converte a severidade declarada no contrato.
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "info" => Some(Severity::Info),
            "warning" => Some(Severity::Warning),
            "error" => Some(Severity::Error),
            _ => None,
        }
    }

    /// `true` quando o diagnóstico degrada o renderer (erro no caminho crítico).
    pub fn is_failure(self) -> bool {
        matches!(self, Severity::Error)
    }
}

/// Um problema observado (agregado por código + mensagem).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderDiagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub detail: Option<String>,
    /// Quantas vezes o mesmo problema ocorreu.
    pub count: u32,
}

/// Resumo pronto para telemetria (`get_live_telemetry`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub codes: Vec<String>,
    /// Diagnósticos descartados por limite de capacidade.
    pub dropped: u32,
    /// Há erro no caminho crítico (o renderer está degradado).
    pub degraded: bool,
}

/// Lista de diagnósticos, deduplicada por `(code, message)`.
fn sink() -> &'static Mutex<Vec<RenderDiagnostic>> {
    static SINK: OnceLock<Mutex<Vec<RenderDiagnostic>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(Vec::new()))
}

const CAPACITY: usize = 64;

/// Contador de descartes (fora do `Vec` para sobreviver ao limite).
fn dropped() -> &'static Mutex<u32> {
    static DROPPED: OnceLock<Mutex<u32>> = OnceLock::new();
    DROPPED.get_or_init(|| Mutex::new(0))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        // Um `panic!` de outro thread não pode esconder diagnósticos: o conteúdo
        // continua íntegro, então seguimos com o guard envenenado.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Registra um problema sem `panic!` e sem `unwrap`.
///
/// Devolve o diagnóstico agregado (`None` quando nem caberia na lista, mas a
/// contagem de descartes é incrementada de qualquer forma).
pub fn report(code: &str, message: impl Into<String>) -> Option<RenderDiagnostic> {
    report_with_detail(code, message, None::<String>)
}

/// Igual a [`report`], com um detalhe técnico (mensagem de erro, valor recusado).
pub fn report_with_detail(
    code: &str,
    message: impl Into<String>,
    detail: Option<impl Into<String>>,
) -> Option<RenderDiagnostic> {
    let severity = match render_contract::diagnostic_severity(code) {
        Some(severity) => severity,
        None => {
            // O catálogo é a fonte da verdade: um código desconhecido é drift do
            // contrato, não um diagnóstico inventado no meio do render.
            let mut log = lock(sink());
            let drift = log
                .iter_mut()
                .find(|entry| entry.code == "contract_drift" && entry.message.contains(code));
            if let Some(entry) = drift {
                entry.count = entry.count.saturating_add(1);
                return None;
            }
            push(
                &mut log,
                RenderDiagnostic {
                    code: "contract_drift".to_string(),
                    severity: Severity::Error,
                    message: format!("código de diagnóstico '{code}' não está no render contract"),
                    detail: Some(message.into()),
                    count: 1,
                },
            );
            return None;
        }
    };

    let message = message.into();
    let detail = detail.map(Into::into);
    let mut log = lock(sink());
    if let Some(entry) = log
        .iter_mut()
        .find(|entry| entry.code == code && entry.message == message)
    {
        entry.count = entry.count.saturating_add(1);
        if detail.is_some() {
            entry.detail = detail;
        }
        let updated = entry.clone();
        return Some(updated);
    }
    let diagnostic = RenderDiagnostic {
        code: code.to_string(),
        severity,
        message,
        detail,
        count: 1,
    };
    push(&mut log, diagnostic.clone());
    Some(diagnostic)
}

fn push(log: &mut Vec<RenderDiagnostic>, diagnostic: RenderDiagnostic) {
    if log.len() >= CAPACITY {
        let mut dropped = lock(dropped());
        *dropped = dropped.saturating_add(1);
        return;
    }
    log.push(diagnostic);
}

/// Diagnósticos na ordem em que apareceram.
pub fn entries() -> Vec<RenderDiagnostic> {
    lock(sink()).clone()
}

/// Último diagnóstico registrado.
pub fn last() -> Option<RenderDiagnostic> {
    lock(sink()).last().cloned()
}

/// Resumo para telemetria/status.
pub fn summary() -> DiagnosticSummary {
    let log = lock(sink());
    let errors = log
        .iter()
        .filter(|entry| entry.severity.is_failure())
        .count();
    let warnings = log
        .iter()
        .filter(|entry| entry.severity == Severity::Warning)
        .count();
    DiagnosticSummary {
        total: log.len(),
        errors,
        warnings,
        codes: log.iter().map(|entry| entry.code.clone()).collect(),
        dropped: *lock(dropped()),
        degraded: errors > 0,
    }
}

/// Limpa o histórico (após o consumidor reconhecer os avisos).
pub fn clear() {
    lock(sink()).clear();
    *lock(dropped()) = 0;
}

/// Registra o estado do render contract (um drift é um erro, não um aviso).
///
/// Chamado na criação do headless: se o contrato não carregou, todo o resto do
/// quadro está suspeito e isso precisa aparecer na telemetria.
pub fn report_contract_health() -> bool {
    if render_contract::contract_is_valid() {
        return true;
    }
    for problem in render_contract::diagnostics() {
        report_with_detail(
            "contract_drift",
            "render contract com problema; usando defaults seguros",
            Some(problem),
        );
    }
    false
}

/// Serializa os testes que **observam** o coletor global de diagnósticos.
///
/// O coletor é global por design (telemetria do processo). Sem exclusão mútua,
/// um teste que afirma uma contagem (`warnings == 1`) corre em paralelo com
/// qualquer outro que reporte — inclusive os de `headless`, que reportam
/// `device_unavailable` quando o runner não tem adaptador — e a contagem vira
/// sorte de escalonamento.
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_code_records_a_diagnostic_with_contract_severity() {
        let _guard = test_guard();
        clear();
        let diagnostic = report("geometry_unavailable", "modo degradado no teste");
        let diagnostic = diagnostic.expect("diagnóstico registrado");
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(summary().warnings, 1);
        assert!(!summary().degraded);
        clear();
    }

    #[test]
    fn errors_degrade_the_summary() {
        let _guard = test_guard();
        clear();
        report("device_unavailable", "sem adaptador no teste");
        let summary = summary();
        assert_eq!(summary.errors, 1);
        assert!(summary.degraded);
        assert_eq!(summary.codes, vec!["device_unavailable".to_string()]);
        clear();
    }

    #[test]
    fn repeated_reports_aggregate_instead_of_growing() {
        let _guard = test_guard();
        clear();
        report("frame_skipped", "frame pulado no teste");
        report("frame_skipped", "frame pulado no teste");
        let entries = entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].count, 2);
        clear();
    }

    #[test]
    fn unknown_code_is_contract_drift() {
        let _guard = test_guard();
        clear();
        assert!(report("codigo_que_nao_existe", "teste").is_none());
        let entries = entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "contract_drift");
        assert!(entries[0].message.contains("codigo_que_nao_existe"));
        clear();
    }

    #[test]
    fn every_contract_code_has_a_severity() {
        let codes = render_contract::diagnostic_codes();
        assert!(codes.len() >= 20, "catálogo de diagnóstico muito curto");
        for (code, severity) in codes {
            assert!(
                render_contract::diagnostic_severity(code).is_some(),
                "código '{code}' sem severidade"
            );
            assert!(
                Severity::from_str(severity).is_some(),
                "severidade de '{code}' inválida"
            );
        }
        assert!(render_contract::diagnostic_severity("webgpu_unavailable").is_some());
        assert!(render_contract::diagnostic_severity("nao_existe").is_none());
    }
}
