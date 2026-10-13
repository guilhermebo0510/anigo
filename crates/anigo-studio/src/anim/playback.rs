//! Transporte da Animação: relógio contínuo, reprodução (Espaço), passo de quadro, saltos (Home/End, chave
//! anterior/seguinte), velocidade e parada no fim. Puro (sem egui): a UI só chama `advance(dt)` a cada quadro.
//!
//! O relógio `raw` é contínuo; o quadro MOSTRADO na régua é `clip.wrap_frame(raw)` (loop/ping-pong aplicados) e o quadro
//! AVALIADO é `clip.evaluate(fps, raw)` (que ainda aplica o "animar em Ns"). Durante a reprodução o relógio pode ser
//! quantizado em quadros inteiros (`quantize`): o que se vê é o que o render da sequência produz.

use anigo_core::animation::LoopMode;
use anigo_core::project::AnimationClip;

#[derive(Debug, Clone, PartialEq)]
pub struct Transport {
    pub playing: bool,
    /// 1.0 = tempo real; negativo toca ao contrário.
    pub speed: f32,
    /// Relógio contínuo, em quadros.
    pub raw: f32,
    /// Reproduz em quadros inteiros (padrão): o quadro avaliado é o `floor` do relógio.
    pub quantize: bool,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { playing: false, speed: 1.0, raw: 0.0, quantize: true }
    }
}

/// Faixa e modo de repetição do clip ativo (ou o padrão sem clip: 1 segundo, uma vez).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: i32,
    pub end: i32,
    pub mode: LoopMode,
}

impl Span {
    pub fn of(clip: Option<&AnimationClip>, fps: f32) -> Span {
        match clip {
            Some(c) => {
                let (start, end) = c.effective_range(fps);
                Span { start, end, mode: c.loop_mode }
            }
            None => Span { start: 0, end: fps.round().max(1.0) as i32, mode: LoopMode::Once },
        }
    }

    pub fn len(&self) -> i32 {
        (self.end - self.start).max(0)
    }
}

impl Transport {
    /// Quadro mostrado na régua: loop/ping-pong aplicados ao relógio.
    pub fn displayed(&self, span: Span) -> f32 {
        let len = span.len() as f32;
        let s = span.start as f32;
        if len <= 0.0 {
            return s;
        }
        match span.mode {
            LoopMode::Once => self.raw,
            LoopMode::Loop => s + (self.raw - s).rem_euclid(len),
            LoopMode::PingPong => {
                let p = (self.raw - s).rem_euclid(2.0 * len);
                if p <= len {
                    s + p
                } else {
                    s + 2.0 * len - p
                }
            }
        }
    }

    /// Quadro (inteiro) mais próximo do mostrado: o quadro em que o usuário está para inserir chaves.
    pub fn current_frame(&self, span: Span) -> i32 {
        self.displayed(span).round() as i32
    }

    /// Relógio a entregar a `clip.evaluate`: quantizado quando está tocando e `quantize` ligado.
    pub fn eval_raw(&self) -> f32 {
        if self.playing && self.quantize {
            (self.raw + 1e-4).floor()
        } else {
            self.raw
        }
    }

    /// Vai para um quadro (para de tocar só se `stop`).
    pub fn goto(&mut self, frame: f32, stop: bool) {
        self.raw = if frame.is_finite() { frame } else { 0.0 };
        if stop {
            self.playing = false;
        }
    }

    /// Liga/desliga a reprodução. Tocar no fim da faixa (modo "uma vez") recomeça do início.
    pub fn toggle(&mut self, span: Span) {
        if self.playing {
            self.playing = false;
            return;
        }
        if span.mode == LoopMode::Once && span.len() > 0 {
            if self.speed >= 0.0 && self.raw >= span.end as f32 {
                self.raw = span.start as f32;
            } else if self.speed < 0.0 && self.raw <= span.start as f32 {
                self.raw = span.end as f32;
            }
        }
        self.playing = true;
    }

    /// Avança o relógio `dt` segundos. Devolve `true` se o quadro mudou. No modo "uma vez" para na ponta.
    pub fn advance(&mut self, dt: f32, fps: f32, span: Span) -> bool {
        if !self.playing || !dt.is_finite() || dt <= 0.0 {
            return false;
        }
        let before = self.raw;
        self.raw += dt.min(0.25) * fps.max(1.0) * self.speed;
        let (s, e) = (span.start as f32, span.end as f32);
        let len = span.len() as f32;
        if len > 0.0 {
            match span.mode {
                LoopMode::Once => {
                    if self.speed >= 0.0 && self.raw >= e {
                        self.raw = e;
                        self.playing = false;
                    } else if self.speed < 0.0 && self.raw <= s {
                        self.raw = s;
                        self.playing = false;
                    }
                }
                LoopMode::Loop => self.raw = s + (self.raw - s).rem_euclid(len),
                LoopMode::PingPong => self.raw = s + (self.raw - s).rem_euclid(2.0 * len),
            }
        } else {
            self.playing = false;
        }
        self.raw != before
    }

    /// Anda `n` quadros a partir do quadro mostrado (inteiro). "Uma vez" e ping-pong andam livremente a partir de 0
    /// (dá para ir além do fim da faixa para editar); loop dá a volta dentro da faixa.
    pub fn step(&mut self, n: i32, span: Span) {
        self.playing = false;
        let cur = self.displayed(span).round() as i32;
        let mut next = cur + n;
        if span.mode == LoopMode::Loop && span.len() > 0 {
            next = span.start + (next - span.start).rem_euclid(span.len());
        }
        self.raw = next.max(0) as f32;
    }

    pub fn home(&mut self, span: Span) {
        self.playing = false;
        self.raw = span.start as f32;
    }

    pub fn end(&mut self, span: Span) {
        self.playing = false;
        self.raw = span.end as f32;
    }

    /// Salta para a chave anterior/seguinte (quadros com alguma chave). Devolve `false` se não há.
    pub fn jump_key(&mut self, clip: &AnimationClip, span: Span, forward: bool) -> bool {
        let cur = self.displayed(span).round() as i32;
        let frames = clip.key_frames();
        let target = if forward { frames.into_iter().find(|f| *f > cur) } else { frames.into_iter().rev().find(|f| *f < cur) };
        match target {
            Some(f) => {
                self.playing = false;
                self.raw = f as f32;
                true
            }
            None => false,
        }
    }

    /// Velocidade: múltiplos de 0.25, entre -4 e 4 (0 nunca).
    pub fn set_speed(&mut self, speed: f32) {
        let s = if speed.is_finite() { speed.clamp(-4.0, 4.0) } else { 1.0 };
        self.speed = if s.abs() < 0.05 { 0.05 } else { s };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{Channel, ChannelTarget, Key, KeyValue};
    use anigo_core::ids::AnimationClipId;

    fn span(mode: LoopMode) -> Span {
        Span { start: 0, end: 24, mode }
    }

    fn clip_with_keys(frames: &[i32]) -> AnimationClip {
        let mut c = AnimationClip::new(AnimationClipId::from_slug("t"), "t");
        c.range = Some([0, 24]);
        c.channels = vec![Channel { target: ChannelTarget::Hips, keys: frames.iter().map(|f| Key::new(*f, KeyValue::Vec3([*f as f32, 0.0, 0.0]))).collect() }];
        c
    }

    #[test]
    fn once_stops_at_the_end_and_play_again_restarts() {
        let mut t = Transport::default();
        t.toggle(span(LoopMode::Once));
        assert!(t.playing);
        // 1 s a 24 fps = 24 quadros
        for _ in 0..62 {
            t.advance(1.0 / 60.0, 24.0, span(LoopMode::Once));
        }
        assert!(!t.playing, "parou");
        assert_eq!(t.raw, 24.0);
        t.toggle(span(LoopMode::Once));
        assert!(t.playing);
        assert_eq!(t.raw, 0.0, "tocar no fim recomeça");
    }

    #[test]
    fn loop_wraps_and_pingpong_reflects_on_the_ruler() {
        let mut t = Transport { playing: true, raw: 23.0, ..Default::default() };
        t.advance(2.0 / 24.0, 24.0, span(LoopMode::Loop));
        assert!((t.raw - 1.0).abs() < 1e-4, "{}", t.raw);
        assert!(t.playing);
        let mut p = Transport { playing: true, raw: 23.0, ..Default::default() };
        p.advance(3.0 / 24.0, 24.0, span(LoopMode::PingPong));
        assert!((p.displayed(span(LoopMode::PingPong)) - 22.0).abs() < 1e-4, "{}", p.displayed(span(LoopMode::PingPong)));
        for _ in 0..500 {
            p.advance(1.0 / 60.0, 24.0, span(LoopMode::PingPong));
            let d = p.displayed(span(LoopMode::PingPong));
            assert!((0.0..=24.0).contains(&d), "{d}");
            assert!(p.raw < 48.0 + 1e-3, "o relógio não cresce sem limite");
        }
    }

    #[test]
    fn reverse_playback_stops_at_the_start() {
        let mut t = Transport { playing: true, raw: 3.0, speed: -1.0, ..Default::default() };
        t.advance(0.5, 24.0, span(LoopMode::Once));
        assert_eq!((t.raw, t.playing), (0.0, false));
        t.toggle(span(LoopMode::Once));
        assert_eq!(t.raw, 24.0, "ao contrário recomeça do fim");
    }

    #[test]
    fn speed_scales_the_clock_and_long_frames_are_capped() {
        let mut t = Transport { playing: true, speed: 2.0, ..Default::default() };
        t.advance(0.1, 24.0, span(LoopMode::Loop));
        assert!((t.raw - 4.8).abs() < 1e-4);
        let mut t = Transport { playing: true, ..Default::default() };
        t.advance(10.0, 24.0, span(LoopMode::Loop));
        assert!(t.raw <= 6.0 + 1e-3, "um quadro lento de 10 s conta no máximo 0,25 s: {}", t.raw);
        t.set_speed(0.0);
        assert!(t.speed > 0.0);
        t.set_speed(99.0);
        assert_eq!(t.speed, 4.0);
        t.set_speed(f32::NAN);
        assert_eq!(t.speed, 1.0);
    }

    #[test]
    fn quantized_playback_evaluates_whole_frames() {
        let mut t = Transport { playing: true, raw: 5.7, ..Default::default() };
        assert_eq!(t.eval_raw(), 5.0);
        t.quantize = false;
        assert_eq!(t.eval_raw(), 5.7);
        t.playing = false;
        t.quantize = true;
        assert_eq!(t.eval_raw(), 5.7, "parado (scrub) avalia o valor exato");
    }

    #[test]
    fn stepping_clamps_in_once_and_wraps_in_loop_modes() {
        let mut t = Transport::default();
        t.step(-1, span(LoopMode::Once));
        assert_eq!(t.raw, 0.0);
        t.step(5, span(LoopMode::Once));
        assert_eq!(t.raw, 5.0);
        t.raw = 24.0;
        t.step(1, span(LoopMode::Once));
        assert_eq!(t.raw, 25.0, "dá para passar do fim da faixa");
        t.raw = 24.0;
        t.step(1, span(LoopMode::Loop));
        assert_eq!(t.raw, 1.0);
        t.raw = 0.0;
        t.step(-1, span(LoopMode::Loop));
        assert_eq!(t.raw, 23.0);
        t.home(span(LoopMode::Once));
        assert_eq!(t.raw, 0.0);
        t.end(span(LoopMode::Once));
        assert_eq!(t.raw, 24.0);
    }

    #[test]
    fn jumping_between_keys() {
        let clip = clip_with_keys(&[0, 6, 12, 20]);
        let sp = Span::of(Some(&clip), 24.0);
        let mut t = Transport { raw: 7.0, playing: true, ..Default::default() };
        assert!(t.jump_key(&clip, sp, true));
        assert_eq!((t.raw, t.playing), (12.0, false));
        assert!(t.jump_key(&clip, sp, false));
        assert_eq!(t.raw, 6.0);
        t.raw = 20.0;
        assert!(!t.jump_key(&clip, sp, true));
        t.raw = 0.0;
        assert!(!t.jump_key(&clip, sp, false));
    }

    #[test]
    fn span_defaults_without_a_clip() {
        let s = Span::of(None, 30.0);
        assert_eq!((s.start, s.end, s.mode), (0, 30, LoopMode::Once));
        let mut t = Transport::default();
        t.goto(f32::NAN, true);
        assert_eq!(t.raw, 0.0);
        assert_eq!(Transport::default().current_frame(s), 0);
    }
}
