//! Lógica pura (sem egui/GPU) do workspace Personagem: tradução controle → `Command` do
//! núcleo, catálogo de sliders por região, presets canônicos e persistência do projeto.
//!
//! Regra de arquitetura: a UI nunca altera o personagem diretamente; tudo o que este módulo
//! produz são `anigo_core::Command` que o `CoreSession` valida, aplica e desfaz.

pub mod macros;
pub mod persist;
pub mod presets;
pub mod sliders;

use anigo_core::command::{Command, CommandError};
use anigo_core::project::ProjectState;

/// Compõe `cmds` em um único comando, descartando os que não teriam efeito sobre o estado
/// (sem efeito = o núcleo recusaria o lote inteiro com `NoOp`). Cada comando é validado e
/// aplicado em sequência sobre uma cópia, então o resultado é válido por construção.
///
/// * `Ok(None)`: nada a fazer;
/// * `Ok(Some(cmd))`: um comando (ou `Batch` se houver mais de um passo);
/// * `Err`: algum comando é inválido de verdade (valor fora do domínio, alvo desconhecido).
pub fn compose(state: &ProjectState, cmds: Vec<Command>) -> Result<Option<Command>, CommandError> {
    let mut scratch = state.clone();
    let mut kept: Vec<Command> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        let before = scratch.clone();
        match cmd.apply(&mut scratch) {
            Ok(_) => {
                if scratch != before {
                    kept.push(cmd);
                }
            }
            Err(CommandError::NoOp(_)) => {}
            Err(e) => return Err(e),
        }
    }
    if scratch == *state {
        // o lote inteiro se anula (ex.: zerar morphs e reaplicar os mesmos valores)
        return Ok(None);
    }
    Ok(match kept.len() {
        0 => None,
        1 => kept.pop(),
        _ => Some(Command::Batch { commands: kept }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::ids::MorphId;

    fn morph(id: &str, v: f32) -> Command {
        Command::SetMorphValue { target: MorphId::for_slider(id), value: v }
    }

    #[test]
    fn compose_drops_noops_and_wraps_in_batch() {
        let state = ProjectState::default();
        // head_width já está no padrão (1.0): NoOp; eye_scale_uniform muda
        let cmd = compose(&state, vec![morph("head_width", 1.0), morph("eye_scale_uniform", 1.2)])
            .unwrap()
            .unwrap();
        assert!(matches!(cmd, Command::SetMorphValue { .. }), "um só passo útil vira comando simples: {cmd:?}");
        let both = compose(&state, vec![morph("head_width", 1.1), morph("eye_scale_uniform", 1.2)])
            .unwrap()
            .unwrap();
        assert!(matches!(both, Command::Batch { ref commands } if commands.len() == 2));
        assert!(both.validate(&state).is_ok());
    }

    #[test]
    fn compose_reports_nothing_to_do_and_real_errors() {
        let state = ProjectState::default();
        assert!(compose(&state, vec![morph("head_width", 1.0)]).unwrap().is_none());
        assert!(compose(&state, vec![morph("head_width", 99.0)]).is_err());
    }
}
