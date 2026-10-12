//! Saída de render do ANIGO Studio (workspace Render): lógica pura e a fila em thread de GPU dedicada.
//! A UI mora em `app::render_ui`; aqui não há egui.
//!
//! * [`settings`] — presets, bloqueio de aspecto, plano de render, nomes por template com contador;
//! * [`image_ops`] — redução do supersampling, alfa, Display P3, 8/16 bits;
//! * [`png_out`] — PNG/JPEG com alfa reto, DPI, `sRGB`/`cICP` e metadados `anigo:*`;
//! * [`aov`] — AOVs por rasterizador de software (profundidade, máscara, ID, normais) e contorno por diferença;
//! * [`job`] — pipeline de um quadro e fila (progresso, cancelar, retomar, gancho de sequência);
//! * [`history`] — histórico de saídas com miniaturas.

pub mod aov;
pub mod history;
pub mod image_ops;
pub mod job;
pub mod png_out;
pub mod settings;

#[cfg(test)]
mod job_tests;
