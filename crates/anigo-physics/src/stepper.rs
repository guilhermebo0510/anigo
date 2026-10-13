//! Passo fixo com acumulador: a física roda sempre em `step` fixo (determinismo), independente do
//! frame rate do app. `dt` inválido é ignorado; `dt` enorme é limitado (sem "espiral da morte").

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FixedStepper {
    /// Duração do passo fixo (s). Padrão 1/60: os parâmetros do VRM spring bone assumem 60 Hz.
    pub step: f32,
    /// `dt` máximo aceito por chamada (s); o excedente é descartado.
    pub max_dt: f32,
    /// Máximo de passos por chamada (backlog acima disso é descartado).
    pub max_steps: u32,
    acc: f64,
}

impl Default for FixedStepper {
    fn default() -> Self {
        Self { step: 1.0 / 60.0, max_dt: 0.1, max_steps: 8, acc: 0.0 }
    }
}

/// Resultado de [`FixedStepper::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepPlan {
    /// Quantos passos fixos executar.
    pub steps: u32,
    /// `dt` recebido era inválido (NaN/infinito/<= 0) e foi ignorado.
    pub invalid_dt: bool,
    /// Tempo descartado por exceder `max_dt`/`max_steps`.
    pub clamped: bool,
}

impl FixedStepper {
    pub fn new(step_hz: f32) -> Self {
        let step = if step_hz.is_finite() && step_hz > 0.0 { 1.0 / step_hz } else { 1.0 / 60.0 };
        Self { step, ..Default::default() }
    }

    /// Fração do passo já acumulada (0..1), para interpolar a apresentação.
    pub fn alpha(&self) -> f32 {
        if self.step > 0.0 {
            ((self.acc / self.step as f64) as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn reset(&mut self) {
        self.acc = 0.0;
    }

    pub fn advance(&mut self, dt: f32) -> StepPlan {
        let mut plan = StepPlan::default();
        if !(dt.is_finite() && dt > 0.0) || !(self.step.is_finite() && self.step > 0.0) {
            plan.invalid_dt = true;
            return plan;
        }
        let mut dt = dt as f64;
        if dt > self.max_dt as f64 {
            dt = self.max_dt as f64;
            plan.clamped = true;
        }
        self.acc += dt;
        let step = self.step as f64;
        // Pequena tolerância evita perder um passo por erro de arredondamento (ex.: 60 x 1/60).
        let mut n = ((self.acc / step) + 1e-9).floor() as u32;
        if n > self.max_steps {
            n = self.max_steps;
            plan.clamped = true;
            self.acc = 0.0;
        } else {
            self.acc = (self.acc - n as f64 * step).max(0.0);
        }
        plan.steps = n;
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessenta_quadros_de_um_sessenta_avos_dao_sessenta_passos() {
        let mut s = FixedStepper::default();
        let total: u32 = (0..60).map(|_| s.advance(1.0 / 60.0).steps).sum();
        assert_eq!(total, 60);
    }

    #[test]
    fn frame_rate_diferente_mesmo_numero_de_passos() {
        for fps in [30.0f32, 45.0, 60.0, 144.0, 240.0] {
            let mut s = FixedStepper::default();
            let frames = (fps * 2.0) as usize; // 2 s
            let total: u32 = (0..frames).map(|_| s.advance(1.0 / fps).steps).sum();
            assert!((total as i64 - 120).abs() <= 1, "{fps} fps -> {total} passos");
        }
    }

    #[test]
    fn dt_invalido_nao_avanca() {
        let mut s = FixedStepper::default();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let p = s.advance(dt);
            assert_eq!(p.steps, 0);
            assert!(p.invalid_dt);
        }
    }

    #[test]
    fn dt_enorme_e_limitado() {
        let mut s = FixedStepper::default();
        let p = s.advance(1000.0);
        assert!(p.clamped);
        assert!(p.steps <= s.max_steps);
        assert_eq!(s.advance(1.0 / 60.0).steps, 1, "sem backlog depois do descarte");
    }

    #[test]
    fn alpha_fica_em_0_1() {
        let mut s = FixedStepper::default();
        s.advance(0.5 / 60.0);
        assert!((s.alpha() - 0.5).abs() < 1e-3);
    }
}
