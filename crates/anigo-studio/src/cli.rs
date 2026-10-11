//! Argumentos de linha de comando (sem dependência externa).
//!
//! ```text
//! anigo-studio [--workspace <chave>] [--screenshot <png>] [--frames <n>] [--size <LxA>]
//!              [--scenario <nome>] [--session-file <json>] [--no-session] [--lang pt|en]
//! ```

use crate::i18n::Lang;
use crate::workspace::WorkspaceId;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cli {
    pub workspace: Option<WorkspaceId>,
    pub screenshot: Option<String>,
    /// Quadros a esperar antes do screenshot (deixa a UI estabilizar).
    pub frames: u32,
    pub size: Option<(f32, f32)>,
    pub scenario: Option<String>,
    pub session_file: Option<String>,
    pub no_session: bool,
    pub lang: Option<Lang>,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut cli = Cli {
        frames: 8,
        ..Default::default()
    };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} exige um valor"));
        match arg.as_str() {
            "--workspace" => {
                let v = value("--workspace")?;
                cli.workspace = Some(WorkspaceId::from_key(&v).ok_or_else(|| format!("workspace desconhecido: {v}"))?);
            }
            "--screenshot" => cli.screenshot = Some(value("--screenshot")?),
            "--frames" => {
                cli.frames = value("--frames")?.parse().map_err(|_| "--frames precisa ser inteiro".to_string())?;
            }
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size usa o formato LARGURAxALTURA")?;
                cli.size = Some((
                    w.parse().map_err(|_| "largura inválida".to_string())?,
                    h.parse().map_err(|_| "altura inválida".to_string())?,
                ));
            }
            "--scenario" => cli.scenario = Some(value("--scenario")?),
            "--session-file" => cli.session_file = Some(value("--session-file")?),
            "--no-session" => cli.no_session = true,
            "--lang" => {
                cli.lang = Some(match value("--lang")?.as_str() {
                    "pt" | "pt-BR" => Lang::PtBr,
                    "en" => Lang::En,
                    other => return Err(format!("idioma desconhecido: {other}")),
                });
            }
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Cli, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_all_flags() {
        let c = p(&[
            "--workspace", "modelagem", "--screenshot", "/tmp/a.png", "--frames", "3", "--size", "1280x720",
            "--scenario", "edit-face", "--no-session", "--lang", "en",
        ])
        .unwrap();
        assert_eq!(c.workspace, Some(WorkspaceId::Modelagem));
        assert_eq!(c.screenshot.as_deref(), Some("/tmp/a.png"));
        assert_eq!((c.frames, c.size), (3, Some((1280.0, 720.0))));
        assert!(c.no_session);
        assert_eq!(c.lang, Some(Lang::En));
    }

    #[test]
    fn rejects_bad_input_with_messages() {
        assert!(p(&["--workspace", "x"]).unwrap_err().contains("desconhecido"));
        assert!(p(&["--frames"]).unwrap_err().contains("valor"));
        assert!(p(&["--size", "100"]).is_err());
        assert!(p(&["--wat"]).is_err());
    }

    #[test]
    fn defaults_wait_a_few_frames() {
        assert_eq!(p(&[]).unwrap().frames, 8);
    }
}
