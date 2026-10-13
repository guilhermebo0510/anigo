//! Entrada sintética da bridge (`INPUT`): ponteiro, roda e teclado injetados NO egui (gancho `raw_input_hook`), um passo
//! por quadro. Serve para o `anigo-mcp` testar o app como um usuário (orbitar, selecionar, arrastar, soltar da Biblioteca)
//! sem tocar no cursor nem no teclado do sistema operacional: os eventos nunca saem da janela do ANIGO.
//!
//! Coordenadas em pontos lógicos da janela (as mesmas dos retângulos de `GET_UI_STATE.views`).
//!
//! Passos (`steps`): `move {pos}`, `down|up {button, pos?}`, `click {pos, button?}`, `scroll {delta:[dx,dy]}`,
//! `key {key, pressed?}` (sem `pressed` = toque), `mods {mods:[ctrl,shift,alt]}` (valem até mudar), `wait {frames}` e
//! `drag {from, to, button?, frames?}` (move → pressiona → move em `frames` pedaços → solta).

use super::StudioApp;
use anigo_bridge::protocol::{BridgeError, BridgeResult};
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;

const MAX_STEPS: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Move(Pos2),
    Button { pos: Pos2, button: PointerButton, pressed: bool },
    Scroll(Vec2),
    Key { key: egui::Key, pressed: bool },
    Mods(Modifiers),
    Wait(u32),
}

#[derive(Default)]
pub(crate) struct InputQueue {
    steps: VecDeque<Step>,
    mods: Modifiers,
    pos: Pos2,
    wait: u32,
}

impl InputQueue {
    pub(crate) fn pending(&self) -> usize {
        self.steps.len() + self.wait as usize
    }

    /// Eventos do próximo quadro (um passo por quadro; `wait` segura a fila).
    pub(crate) fn next_events(&mut self, raw: &mut egui::RawInput) {
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        let Some(step) = self.steps.pop_front() else {
            return;
        };
        match step {
            Step::Move(p) => {
                self.pos = p;
                raw.events.push(Event::PointerMoved(p));
            }
            Step::Button { pos, button, pressed } => {
                self.pos = pos;
                raw.events.push(Event::PointerMoved(pos));
                raw.events.push(Event::PointerButton { pos, button, pressed, modifiers: self.mods });
            }
            Step::Scroll(delta) => {
                raw.events.push(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, phase: egui::TouchPhase::Move, modifiers: self.mods });
            }
            Step::Key { key, pressed } => {
                raw.events.push(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: self.mods });
            }
            Step::Mods(m) => {
                self.mods = m;
                raw.events.push(Event::ModifiersChanged(m));
            }
            Step::Wait(n) => self.wait = n.saturating_sub(1),
        }
    }
}

fn pos(v: &Value, name: &str) -> Result<Pos2, BridgeError> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| BridgeError::invalid_params(format!("'{name}' deve ser [x, y]")))?;
    let f = |i: usize| -> Result<f32, BridgeError> {
        a[i].as_f64()
            .filter(|f| f.is_finite() && f.abs() < 100_000.0)
            .map(|f| f as f32)
            .ok_or_else(|| BridgeError::invalid_params(format!("'{name}[{i}]' não é um número válido")))
    };
    Ok(Pos2::new(f(0)?, f(1)?))
}

fn button(m: &Map<String, Value>) -> Result<PointerButton, BridgeError> {
    match m.get("button").and_then(Value::as_str).unwrap_or("primary") {
        "primary" | "left" => Ok(PointerButton::Primary),
        "secondary" | "right" => Ok(PointerButton::Secondary),
        "middle" => Ok(PointerButton::Middle),
        other => Err(BridgeError::invalid_params(format!("button '{other}' inválido (primary|secondary|middle)"))),
    }
}

fn mods(v: Option<&Value>) -> Result<Modifiers, BridgeError> {
    let mut m = Modifiers::NONE;
    for x in v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        match x.as_str() {
            Some("ctrl") | Some("command") => {
                m.ctrl = true;
                m.command = true;
            }
            Some("shift") => m.shift = true,
            Some("alt") => m.alt = true,
            _ => return Err(BridgeError::invalid_params("mods: use ctrl|shift|alt")),
        }
    }
    Ok(m)
}

/// Converte a lista de passos do pedido em passos primitivos.
fn parse(steps: &[Value]) -> Result<Vec<Step>, BridgeError> {
    let mut out = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let m = s.as_object().ok_or_else(|| BridgeError::invalid_params(format!("steps[{i}] deve ser um objeto")))?;
        let ty = m.get("type").and_then(Value::as_str).ok_or_else(|| BridgeError::invalid_params(format!("steps[{i}].type é obrigatório")))?;
        let need = |k: &str| m.get(k).ok_or_else(|| BridgeError::invalid_params(format!("steps[{i}] ({ty}) exige '{k}'")));
        match ty {
            "move" => out.push(Step::Move(pos(need("pos")?, "pos")?)),
            "down" | "up" => {
                let p = match m.get("pos") {
                    Some(v) => pos(v, "pos")?,
                    None => return Err(BridgeError::invalid_params(format!("steps[{i}] ({ty}) exige 'pos'"))),
                };
                out.push(Step::Button { pos: p, button: button(m)?, pressed: ty == "down" });
            }
            "click" => {
                let p = pos(need("pos")?, "pos")?;
                let b = button(m)?;
                out.push(Step::Move(p));
                out.push(Step::Button { pos: p, button: b, pressed: true });
                out.push(Step::Button { pos: p, button: b, pressed: false });
            }
            "scroll" => {
                let d = pos(need("delta")?, "delta")?;
                out.push(Step::Scroll(Vec2::new(d.x, d.y)));
            }
            "key" => {
                let name = need("key")?.as_str().ok_or_else(|| BridgeError::invalid_params("key deve ser texto"))?;
                let key = egui::Key::from_name(name).ok_or_else(|| BridgeError::invalid_params(format!("tecla desconhecida: {name}")))?;
                match m.get("pressed").and_then(Value::as_bool) {
                    Some(p) => out.push(Step::Key { key, pressed: p }),
                    None => {
                        out.push(Step::Key { key, pressed: true });
                        out.push(Step::Key { key, pressed: false });
                    }
                }
            }
            "mods" => out.push(Step::Mods(mods(m.get("mods"))?)),
            "wait" => {
                let n = m.get("frames").and_then(Value::as_u64).filter(|n| *n <= 600).ok_or_else(|| BridgeError::invalid_params("wait: frames deve ser 0..600"))?;
                out.push(Step::Wait(n as u32));
            }
            "drag" => {
                let (from, to) = (pos(need("from")?, "from")?, pos(need("to")?, "to")?);
                let b = button(m)?;
                let n = m.get("frames").and_then(Value::as_u64).filter(|n| (1..=200).contains(n)).unwrap_or(8) as u32;
                out.push(Step::Move(from));
                out.push(Step::Wait(1));
                out.push(Step::Button { pos: from, button: b, pressed: true });
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    out.push(Step::Move(from + (to - from) * t));
                }
                out.push(Step::Wait(1));
                out.push(Step::Button { pos: to, button: b, pressed: false });
            }
            other => return Err(BridgeError::invalid_params(format!("steps[{i}]: tipo '{other}' desconhecido"))),
        }
    }
    Ok(out)
}

impl StudioApp {
    /// `INPUT`: enfileira os passos (executados um por quadro). Responde logo; `GET_UI_STATE.input_pending` zera ao terminar.
    pub(crate) fn bridge_input(&mut self, params: &Value) -> BridgeResult {
        let steps = params.get("steps").and_then(Value::as_array).ok_or_else(|| BridgeError::invalid_params("INPUT exige 'steps' (lista)"))?;
        if steps.is_empty() || steps.len() > MAX_STEPS {
            return Err(BridgeError::invalid_params(format!("INPUT: 'steps' deve ter de 1 a {MAX_STEPS} passos")));
        }
        let parsed = parse(steps)?;
        if self.input.steps.len() + parsed.len() > MAX_STEPS * 4 {
            return Err(BridgeError::failed("fila de entrada cheia; espere terminar (GET_UI_STATE.input_pending)"));
        }
        let n = parsed.len();
        self.input.steps.extend(parsed);
        Ok(json!({ "queued": n, "pending": self.input.pending() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: Value) -> Vec<Event> {
        let mut q = InputQueue::default();
        q.steps.extend(parse(steps.as_array().unwrap()).unwrap());
        let mut all = Vec::new();
        for _ in 0..200 {
            let mut raw = egui::RawInput::default();
            q.next_events(&mut raw);
            all.extend(raw.events);
            if q.pending() == 0 {
                break;
            }
        }
        all
    }

    #[test]
    fn click_is_move_press_release() {
        let ev = run(json!([{"type":"click","pos":[10.0,20.0]}]));
        // o ponteiro vai até o ponto ANTES de apertar, e solta no mesmo lugar
        assert!(matches!(ev.first(), Some(Event::PointerMoved(p)) if *p == Pos2::new(10.0, 20.0)));
        let presses: Vec<bool> = ev.iter().filter_map(|e| if let Event::PointerButton { pressed, .. } = e { Some(*pressed) } else { None }).collect();
        assert_eq!(presses, [true, false]);
    }

    #[test]
    fn drag_moves_in_pieces_and_ends_at_the_target() {
        let ev = run(json!([{"type":"drag","from":[0.0,0.0],"to":[80.0,40.0],"button":"middle","frames":4}]));
        let last_move = ev.iter().rev().find_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None }).unwrap();
        assert_eq!(last_move, Pos2::new(80.0, 40.0));
        let downs = ev.iter().filter(|e| matches!(e, Event::PointerButton { pressed: true, button: PointerButton::Middle, .. })).count();
        assert_eq!(downs, 1);
    }

    #[test]
    fn modifiers_persist_until_changed() {
        let ev = run(json!([{"type":"mods","mods":["alt"]},{"type":"click","pos":[1.0,1.0]}]));
        assert!(ev.iter().any(|e| matches!(e, Event::PointerButton { modifiers, .. } if modifiers.alt)));
    }

    #[test]
    fn invalid_steps_are_rejected() {
        for bad in [json!([{"type":"warp"}]), json!([{"type":"click"}]), json!([{"type":"click","pos":[1.0]}]), json!([{"type":"key","key":"NopeKey"}]), json!([{"type":"down","pos":[0,0],"button":"x"}]), json!([{"type":"wait","frames":100000}])] {
            assert!(parse(bad.as_array().unwrap()).is_err(), "{bad}");
        }
    }
}
