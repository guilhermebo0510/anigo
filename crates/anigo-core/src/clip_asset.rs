//! Ativo de clip de animação (`.anigoclip.json`): um clip de CORPO (ossos, root/quadril, mãos) por NOME de osso,
//! portátil entre projetos e personagens (o mesmo vocabulário de `.anigopose.json`). Canais de nó/sol/câmera não
//! entram: são específicos de uma cena.
//!
//! O arquivo guarda o `fps` de origem; ao aplicar num projeto com outro fps os quadros são reescalados
//! (`into_clip`/`key_inserts`), mantendo a duração em segundos.

use serde::{Deserialize, Serialize};

use crate::animation::{
    canonicalize_clip, Channel, ChannelGroup, Extrapolation, Interp, Key, KeyInsert, LoopMode, Marker, MAX_FRAME, MAX_HOLD_STEP,
};
use crate::ids::AnimationClipId;
use crate::pose::PoseState;
use crate::project::AnimationClip;

pub const CLIP_ASSET_SUFFIX: &str = ".anigoclip.json";
pub const CLIP_ASSET_FORMAT: &str = "anigo.clip";
pub const CLIP_ASSET_VERSION: u32 = 1;
/// Tamanho máximo aceito de um arquivo de clip.
pub const CLIP_ASSET_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipAsset {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// Taxa de quadros em que as chaves foram autoradas.
    pub fps: f32,
    #[serde(default)]
    pub range: Option<[i32; 2]>,
    #[serde(default)]
    pub loop_mode: LoopMode,
    #[serde(default)]
    pub extrapolation: Extrapolation,
    #[serde(default = "one")]
    pub hold_step: u8,
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClipAssetError {
    #[error("arquivo de clip grande demais ({0} bytes)")]
    TooLarge(usize),
    #[error("JSON de clip inválido: {0}")]
    Json(String),
    #[error("formato desconhecido '{0}' (esperado '{CLIP_ASSET_FORMAT}')")]
    Format(String),
    #[error("versão {0} do formato de clip não suportada (máximo {CLIP_ASSET_VERSION})")]
    Version(u32),
    #[error("clip inválido: {0}")]
    Invalid(String),
    #[error("o clip não tem chaves de corpo para salvar")]
    Empty,
}

impl ClipAsset {
    /// Cria o ativo a partir de um clip do projeto: só os canais de personagem (corpo e mãos).
    pub fn from_clip(name: impl Into<String>, clip: &AnimationClip, fps: f32) -> Result<ClipAsset, ClipAssetError> {
        let channels: Vec<Channel> = clip.channels.iter().filter(|c| c.target.group() == ChannelGroup::Character).cloned().collect();
        if channels.is_empty() {
            return Err(ClipAssetError::Empty);
        }
        let (a, b) = clip.effective_range(fps);
        Ok(ClipAsset {
            format: CLIP_ASSET_FORMAT.into(),
            version: CLIP_ASSET_VERSION,
            name: name.into(),
            fps,
            range: Some([a, b]),
            loop_mode: clip.loop_mode,
            extrapolation: clip.extrapolation,
            hold_step: clip.hold_step.max(1),
            channels,
            markers: clip.markers.clone(),
            tags: Vec::new(),
        })
    }

    pub fn key_count(&self) -> usize {
        self.channels.iter().map(|c| c.keys.len()).sum()
    }

    /// Quadros `[início, fim]` da faixa (explícita ou extensão das chaves).
    pub fn frame_range(&self) -> (i32, i32) {
        if let Some([a, b]) = self.range {
            return (a, b.max(a));
        }
        let hi = self.channels.iter().filter_map(|c| c.keys.last()).map(|k| k.frame).max().unwrap_or(0);
        (0, hi)
    }

    pub fn duration_seconds(&self) -> f32 {
        let (a, b) = self.frame_range();
        (b - a) as f32 / self.fps.max(1.0)
    }

    pub fn to_json(&self) -> Result<String, ClipAssetError> {
        serde_json::to_string_pretty(self).map_err(|e| ClipAssetError::Json(e.to_string()))
    }

    /// Lê e valida (`format`, `version`, estrutura do clip, nomes de osso, finitude, tamanho).
    pub fn from_json(raw: &[u8]) -> Result<ClipAsset, ClipAssetError> {
        if raw.len() > CLIP_ASSET_MAX_BYTES {
            return Err(ClipAssetError::TooLarge(raw.len()));
        }
        let mut a: ClipAsset = serde_json::from_slice(raw).map_err(|e| ClipAssetError::Json(e.to_string()))?;
        if a.format != CLIP_ASSET_FORMAT {
            return Err(ClipAssetError::Format(a.format));
        }
        if a.version == 0 || a.version > CLIP_ASSET_VERSION {
            return Err(ClipAssetError::Version(a.version));
        }
        if !a.fps.is_finite() || !(1.0..=240.0).contains(&a.fps) {
            return Err(ClipAssetError::Invalid(format!("fps {} fora de [1, 240]", a.fps)));
        }
        if !(1..=MAX_HOLD_STEP).contains(&a.hold_step) {
            a.hold_step = 1;
        }
        // reaproveita a validação do clip (ordem, tipos, finitude, nomes de osso)
        let mut probe = a.to_clip(AnimationClipId::from_slug("probe"), a.fps);
        if let Some(bad) = probe.channels.iter().find(|c| c.target.group() != ChannelGroup::Character) {
            return Err(ClipAssetError::Invalid(format!("canal fora do corpo: {}", bad.target.key_string())));
        }
        let keys_before = probe.key_count();
        probe.sanitize(&Default::default());
        if probe.channels.len() != a.channels.len() || probe.key_count() != keys_before {
            return Err(ClipAssetError::Invalid("canais inválidos (osso desconhecido, tipo errado ou valores não finitos)".into()));
        }
        a.channels = probe.channels;
        a.markers = probe.markers;
        if a.channels.is_empty() {
            return Err(ClipAssetError::Empty);
        }
        Ok(a)
    }

    /// Fator de reescala de quadros do fps do ativo para `target_fps`.
    fn scale(&self, target_fps: f32) -> f64 {
        target_fps as f64 / self.fps.max(1.0) as f64
    }

    fn conv(&self, frame: i32, target_fps: f32) -> i32 {
        ((frame as f64 * self.scale(target_fps)).round() as i64).clamp(0, MAX_FRAME as i64) as i32
    }

    /// Clip do projeto com o conteúdo do ativo, reescalado para `target_fps`.
    pub fn to_clip(&self, clip_id: AnimationClipId, target_fps: f32) -> AnimationClip {
        let mut clip = AnimationClip::new(clip_id, self.name.clone());
        clip.channels = self
            .channels
            .iter()
            .map(|c| {
                let mut keys: Vec<Key> = Vec::with_capacity(c.keys.len());
                for k in &c.keys {
                    let f = self.conv(k.frame, target_fps);
                    if keys.last().is_some_and(|l| l.frame == f) {
                        continue;
                    }
                    keys.push(Key { frame: f, ..*k });
                }
                Channel { target: c.target.clone(), keys }
            })
            .collect();
        clip.range = self.range.map(|[a, b]| [self.conv(a, target_fps), self.conv(b, target_fps)]);
        clip.loop_mode = self.loop_mode;
        clip.extrapolation = self.extrapolation;
        clip.hold_step = self.hold_step.clamp(1, MAX_HOLD_STEP);
        clip.markers = self.markers.iter().map(|m| Marker { frame: self.conv(m.frame, target_fps), name: m.name.clone() }).collect();
        clip.duration_seconds = self.duration_seconds();
        canonicalize_clip(&mut clip);
        clip
    }

    /// Chaves do ativo deslocadas para começar em `at_frame` (para "inserir no playhead"), no fps do projeto.
    pub fn key_inserts(&self, at_frame: i32, target_fps: f32) -> Vec<KeyInsert> {
        let first = self.channels.iter().filter_map(|c| c.keys.first()).map(|k| k.frame).min().unwrap_or(0);
        let base = self.conv(first, target_fps);
        let mut out = Vec::new();
        for c in &self.channels {
            let mut last = i32::MIN;
            for k in &c.keys {
                let f = at_frame + (self.conv(k.frame, target_fps) - base);
                if !(0..=MAX_FRAME).contains(&f) || f == last {
                    continue;
                }
                last = f;
                out.push(KeyInsert { target: c.target.clone(), key: Key { frame: f, ..*k } });
            }
        }
        out
    }

    /// `n` poses uniformemente espaçadas na faixa do clip (miniatura em tira).
    pub fn strip_poses(&self, n: usize) -> Vec<PoseState> {
        let clip = self.to_clip(AnimationClipId::from_slug("strip"), self.fps);
        let (a, b) = clip.effective_range(self.fps);
        let n = n.max(1);
        (0..n)
            .map(|i| {
                let t = if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 };
                let f = a as f32 + (b - a) as f32 * t;
                clip.evaluate(self.fps, f).apply_to_pose(&PoseState::default())
            })
            .collect()
    }

    /// Troca a interpolação de todas as chaves (útil para importações cruas).
    pub fn with_interp(mut self, interp: Interp) -> ClipAsset {
        for c in &mut self.channels {
            for k in &mut c.keys {
                k.interp = interp;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{ChannelTarget, KeyValue};
    use glam::Quat;

    fn clip() -> AnimationClip {
        let mut c = AnimationClip::new(AnimationClipId::from_slug("w"), "w");
        c.range = Some([0, 24]);
        c.channels = vec![
            Channel {
                target: ChannelTarget::bone("LeftElbow"),
                keys: vec![Key::new(0, KeyValue::Quat([0.0, 0.0, 0.0, 1.0])), Key::new(24, KeyValue::Quat(Quat::from_rotation_x(-1.2).to_array())).with_interp(Interp::Ease)],
            },
            Channel { target: ChannelTarget::Sun { prop: crate::animation::SunProp::Intensity }, keys: vec![Key::new(0, KeyValue::Scalar(1.0))] },
        ];
        canonicalize_clip(&mut c);
        c
    }

    #[test]
    fn only_body_channels_are_saved_and_the_json_roundtrips() {
        let a = ClipAsset::from_clip("Caminhada", &clip(), 24.0).unwrap();
        assert_eq!(a.channels.len(), 1, "o sol fica de fora");
        let json = a.to_json().unwrap();
        let back = ClipAsset::from_json(json.as_bytes()).unwrap();
        assert_eq!(back, a);
        assert!(json.contains("anigo.clip"));
        assert_eq!(ClipAsset::from_clip("x", &AnimationClip::new(AnimationClipId::from_slug("e"), "e"), 24.0), Err(ClipAssetError::Empty));
    }

    #[test]
    fn invalid_files_are_rejected_with_clear_errors() {
        let a = ClipAsset::from_clip("c", &clip(), 24.0).unwrap();
        let mut bad = a.clone();
        bad.format = "outro".into();
        assert!(matches!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()), Err(ClipAssetError::Format(_))));
        let mut bad = a.clone();
        bad.version = 9;
        assert!(matches!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()), Err(ClipAssetError::Version(9))));
        let mut bad = a.clone();
        bad.channels[0].target = ChannelTarget::bone("Tail");
        assert!(matches!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()), Err(ClipAssetError::Invalid(_))));
        let mut bad = a.clone();
        bad.fps = 0.0;
        assert!(matches!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()), Err(ClipAssetError::Invalid(_))));
        let mut bad = a.clone();
        bad.channels[0].keys[1].value = KeyValue::Scalar(1.0);
        assert!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()).is_err());
        assert!(matches!(ClipAsset::from_json(b"{ nope"), Err(ClipAssetError::Json(_))));
        assert!(matches!(ClipAsset::from_json(&vec![b' '; CLIP_ASSET_MAX_BYTES + 1]), Err(ClipAssetError::TooLarge(_))));
        // canal de cena não é permitido
        let mut bad = a;
        bad.channels.push(Channel { target: ChannelTarget::Sun { prop: crate::animation::SunProp::Intensity }, keys: vec![Key::new(0, KeyValue::Scalar(1.0))] });
        assert!(ClipAsset::from_json(bad.to_json().unwrap().as_bytes()).is_err());
    }

    #[test]
    fn applying_at_another_fps_rescales_frames_and_keeps_the_duration() {
        let a = ClipAsset::from_clip("c", &clip(), 24.0).unwrap();
        let c = a.to_clip(AnimationClipId::from_slug("n"), 48.0);
        assert_eq!(c.range, Some([0, 48]));
        assert_eq!(c.channels[0].keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 48]);
        assert!((c.duration_seconds - 1.0).abs() < 1e-6);
        // inserir no playhead: começa no quadro pedido e preserva os espaçamentos
        let ins = a.key_inserts(10, 24.0);
        assert_eq!(ins.iter().map(|k| k.key.frame).collect::<Vec<_>>(), vec![10, 34]);
        let ins = a.key_inserts(5, 12.0);
        assert_eq!(ins.iter().map(|k| k.key.frame).collect::<Vec<_>>(), vec![5, 17]);
    }

    #[test]
    fn strip_poses_follow_the_curve() {
        let a = ClipAsset::from_clip("c", &clip(), 24.0).unwrap();
        let strip = a.strip_poses(4);
        assert_eq!(strip.len(), 4);
        assert!(strip[0].is_rest());
        let angle = |p: &PoseState| p.rotation(crate::pose::bone::L_ELBOW).angle_between(Quat::IDENTITY);
        assert!(angle(&strip[1]) < angle(&strip[2]) && angle(&strip[2]) < angle(&strip[3]));
        assert!((angle(&strip[3]) - 1.2).abs() < 0.01);
    }
}
