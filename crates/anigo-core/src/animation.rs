//! ANIGO — modelo canônico de ANIMAÇÃO (workspace Animação; ver `docs/native/ANIMACAO_UI.md` e
//! `docs/native/pesquisa/animacao.md`).
//!
//! ## Decisões
//!
//! * **Aditivo.** `ProjectState.animation` já existia (`clips`, `active_clip`, `playhead_seconds`) com `tracks` de
//!   transform por nó. Tudo o que é novo tem `serde(default)` e é omitido do JSON no padrão: projetos antigos
//!   carregam iguais, o JSON canônico/fixtures de um projeto sem animação não mudam, e os `tracks` legados
//!   continuam sendo avaliados (interpolação linear) quando nenhum canal novo cobre o mesmo alvo.
//! * **Tempo em quadros inteiros.** Cada chave está em um quadro (`i32`); `AnimationState.fps` (padrão 24) converte
//!   para segundos. O playhead de reprodução é fracionário (`f32`) só na avaliação.
//! * **Canais tipados por alvo** ([`ChannelTarget`]): rotação local de cada osso (quaternion), translação do
//!   `Root`/`Hips`, curvatura dos dedos de cada mão, transform de nós (translação/rotação/escala), sol (intensidade,
//!   direção, cor) e câmera de cena (FOV, olho, alvo). O valor de uma chave ([`KeyValue`]) precisa casar com o tipo do
//!   canal.
//! * **Interpolação por chave** ([`Interp`], da chave até a próxima): `Step`, `Linear`, `Ease*` e `Bezier` com alças
//!   normalizadas `[x1, y1, x2, y2]` (o mesmo modelo do `cubic-bezier` do CSS). `x1, x2 ∈ [0, 1]` garante monotonia no
//!   tempo; `y` fora de `[0, 1]` dá antecipação/overshoot. A curva remapeia o parâmetro `u` do segmento, então vale
//!   igual para escalar, vetor e **quaternion** (slerp pelo caminho curto, sem flip).
//! * **Loop / ping-pong / extrapolação / "animar em Ns"** ficam no clip: [`LoopMode`], [`Extrapolation`] e `hold_step`
//!   (1, 2, 3…: o quadro avaliado é `início + floor((f − início)/N)·N`, o "twos/threes" do anime).
//! * **Edições** por UM comando, [`crate::command::Command::EditAnimation`], com [`AnimEdit`]: inserir/apagar/mover chaves,
//!   interpolação, faixa do clip, fps (com retime opcional), marcadores, criar/remover/renomear clips. Todas têm inverso
//!   EXATO (`AnimEdit::inverse`), agrupáveis em `Batch`. Os canais ficam sempre ordenados por alvo e as chaves por quadro
//!   (forma canônica), o que torna o inverso independente da ordem de inserção.
//! * **Avaliação pura e determinística:** [`AnimationClip::evaluate`] devolve um [`Sample`] (valores por alvo);
//!   [`Sample::apply_to_project`] o aplica ao projeto SEM histórico (o scrub/play é "preview", como no Blender/Sequencer).

use std::collections::{BTreeMap, BTreeSet};

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::ids::{AnimationClipId, NodeId};
use crate::pose::{bone_index, mirror_bone, PoseState, BONE_NAMES, MAX_TRANSLATION};
use crate::project::{AnimationClip, AnimationState, ProjectState};

/// Taxa de quadros padrão do projeto.
pub const DEFAULT_FPS: f32 = 24.0;
pub const MIN_FPS: f32 = 1.0;
pub const MAX_FPS: f32 = 240.0;
/// Maior quadro permitido (≈ 11,5 h a 24 fps): barra valores absurdos vindos de arquivos.
pub const MAX_FRAME: i32 = 1_000_000;
pub const MAX_CLIPS: usize = 64;
pub const MAX_CHANNELS: usize = 4096;
pub const MAX_KEYS_PER_CLIP: usize = 400_000;
pub const MAX_MARKERS: usize = 1024;
pub const MAX_HOLD_STEP: u8 = 12;

// ---------------------------------------------------------------------------------------------
// Tipos de dados
// ---------------------------------------------------------------------------------------------

/// Como o clip repete fora do seu intervalo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    /// Toca uma vez; fora do intervalo vale a extrapolação das chaves.
    #[default]
    Once,
    /// Período = fim − início; o quadro `fim` equivale ao `início`.
    Loop,
    /// Vai e volta sem repetir as pontas.
    PingPong,
}

/// O que vale antes da primeira e depois da última chave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extrapolation {
    /// Mantém o valor da chave da ponta (o padrão do glTF e do Sequencer).
    #[default]
    Constant,
    /// Continua o segmento da ponta (escalar/vetor/quaternion).
    Linear,
}

/// Interpolação DA CHAVE ATÉ A PRÓXIMA.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Interp {
    /// Segura o valor da chave até a próxima (blocking, "holds" do anime).
    Step,
    #[default]
    Linear,
    /// Ease in-out suave (cubic-bezier 0.42, 0, 0.58, 1).
    Ease,
    /// Parte devagar (0.42, 0, 1, 1).
    EaseIn,
    /// Chega devagar (0, 0, 0.58, 1).
    EaseOut,
    /// Alças livres normalizadas no segmento: `[x1, y1, x2, y2]` (`x ∈ [0, 1]`).
    Bezier { c: [f32; 4] },
}

impl Interp {
    pub fn is_default(&self) -> bool {
        *self == Interp::Linear
    }

    /// Pontos de controle do easing (None para `Step`/`Linear`).
    pub fn control_points(&self) -> Option<[f32; 4]> {
        match self {
            Interp::Step | Interp::Linear => None,
            Interp::Ease => Some([0.42, 0.0, 0.58, 1.0]),
            Interp::EaseIn => Some([0.42, 0.0, 1.0, 1.0]),
            Interp::EaseOut => Some([0.0, 0.0, 0.58, 1.0]),
            Interp::Bezier { c } => Some(*c),
        }
    }

    /// Rótulo estável (i18n/telemetria).
    pub fn label(&self) -> &'static str {
        match self {
            Interp::Step => "step",
            Interp::Linear => "linear",
            Interp::Ease => "ease",
            Interp::EaseIn => "ease_in",
            Interp::EaseOut => "ease_out",
            Interp::Bezier { .. } => "bezier",
        }
    }

    /// Bézier com alças iniciais "lineares" (a curva coincide com a reta até ser arrastada).
    pub fn bezier_linear() -> Interp {
        Interp::Bezier { c: [1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0] }
    }

    /// Normaliza a Bézier (x no intervalo, valores finitos). Os demais ficam como estão.
    pub fn sanitized(self) -> Interp {
        match self {
            Interp::Bezier { c } => {
                let f = |v: f32| if v.is_finite() { v } else { 0.0 };
                Interp::Bezier { c: [f(c[0]).clamp(0.0, 1.0), f(c[1]).clamp(-4.0, 5.0), f(c[2]).clamp(0.0, 1.0), f(c[3]).clamp(-4.0, 5.0)] }
            }
            other => other,
        }
    }
}

/// Lado da mão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandSide {
    Left,
    Right,
}

/// Propriedade animável do sol (primeira luz do projeto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SunProp {
    Intensity,
    Direction,
    Color,
}

/// Propriedade animável da câmera de cena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraProp {
    /// Abertura vertical em GRAUS.
    Fov,
    Eye,
    Target,
}

/// O que um canal anima.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelTarget {
    /// Rotação local (delta sobre o repouso) de um dos 24 ossos canônicos.
    Bone { bone: String },
    /// Translação do osso `Root` (move o personagem inteiro).
    Root,
    /// Translação do osso `Hips` relativa ao repouso.
    Hips,
    /// Curvatura dos 5 dedos de uma mão.
    Hand { side: HandSide },
    NodeTranslation { node_id: NodeId },
    NodeRotation { node_id: NodeId },
    NodeScale { node_id: NodeId },
    Sun { prop: SunProp },
    Camera { prop: CameraProp },
}

/// Tipo do valor de um canal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Scalar,
    Vec3,
    Quat,
    Curl,
}

impl ChannelTarget {
    pub fn value_kind(&self) -> ValueKind {
        match self {
            ChannelTarget::Bone { .. } | ChannelTarget::NodeRotation { .. } => ValueKind::Quat,
            ChannelTarget::Root | ChannelTarget::Hips | ChannelTarget::NodeTranslation { .. } | ChannelTarget::NodeScale { .. } => ValueKind::Vec3,
            ChannelTarget::Hand { .. } => ValueKind::Curl,
            ChannelTarget::Sun { prop } => match prop {
                SunProp::Intensity => ValueKind::Scalar,
                SunProp::Direction | SunProp::Color => ValueKind::Vec3,
            },
            ChannelTarget::Camera { prop } => match prop {
                CameraProp::Fov => ValueKind::Scalar,
                CameraProp::Eye | CameraProp::Target => ValueKind::Vec3,
            },
        }
    }

    pub fn bone(name: &str) -> ChannelTarget {
        ChannelTarget::Bone { bone: name.to_string() }
    }

    pub fn bone_index(index: usize) -> ChannelTarget {
        ChannelTarget::Bone { bone: BONE_NAMES[index.min(BONE_NAMES.len() - 1)].to_string() }
    }

    /// Valor neutro do canal (identidade/zero/escala 1) — o que uma chave nova guarda quando não há valor atual.
    pub fn neutral(&self) -> KeyValue {
        match self.value_kind() {
            ValueKind::Quat => KeyValue::Quat([0.0, 0.0, 0.0, 1.0]),
            ValueKind::Curl => KeyValue::Curl([0.0; 5]),
            ValueKind::Scalar => KeyValue::Scalar(match self {
                ChannelTarget::Sun { .. } => 1.0,
                ChannelTarget::Camera { .. } => 45.0,
                _ => 0.0,
            }),
            ValueKind::Vec3 => KeyValue::Vec3(match self {
                ChannelTarget::NodeScale { .. } => [1.0; 3],
                _ => [0.0; 3],
            }),
        }
    }

    /// Alvo do lado oposto e se o valor precisa ser refletido (`None` = não espelhável, ex.: nós/sol/câmera).
    pub fn mirrored(&self) -> Option<ChannelTarget> {
        match self {
            ChannelTarget::Bone { bone } => bone_index(bone).map(|i| ChannelTarget::bone_index(mirror_bone(i))),
            ChannelTarget::Root | ChannelTarget::Hips => Some(self.clone()),
            ChannelTarget::Hand { side } => Some(ChannelTarget::Hand { side: if *side == HandSide::Left { HandSide::Right } else { HandSide::Left } }),
            _ => None,
        }
    }

    /// Agrupamento da UI/dope sheet (ordem estável).
    pub fn group(&self) -> ChannelGroup {
        match self {
            ChannelTarget::Bone { .. } | ChannelTarget::Root | ChannelTarget::Hips | ChannelTarget::Hand { .. } => ChannelGroup::Character,
            ChannelTarget::NodeTranslation { .. } | ChannelTarget::NodeRotation { .. } | ChannelTarget::NodeScale { .. } => ChannelGroup::Nodes,
            ChannelTarget::Sun { .. } => ChannelGroup::Sun,
            ChannelTarget::Camera { .. } => ChannelGroup::Camera,
        }
    }

    /// Chave estável de texto (ordenação e ids de UI).
    pub fn key_string(&self) -> String {
        match self {
            ChannelTarget::Bone { bone } => format!("bone:{bone}"),
            ChannelTarget::Root => "root".into(),
            ChannelTarget::Hips => "hips".into(),
            ChannelTarget::Hand { side } => format!("hand:{}", if *side == HandSide::Left { "left" } else { "right" }),
            ChannelTarget::NodeTranslation { node_id } => format!("node_t:{node_id}"),
            ChannelTarget::NodeRotation { node_id } => format!("node_r:{node_id}"),
            ChannelTarget::NodeScale { node_id } => format!("node_s:{node_id}"),
            ChannelTarget::Sun { prop } => format!("sun:{prop:?}"),
            ChannelTarget::Camera { prop } => format!("camera:{prop:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelGroup {
    Character,
    Nodes,
    Sun,
    Camera,
}

/// Valor tipado de uma chave.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum KeyValue {
    Scalar(f32),
    Vec3([f32; 3]),
    /// Quaternion `[x, y, z, w]`.
    Quat([f32; 4]),
    /// Curvatura dos 5 dedos.
    Curl([f32; 5]),
}

impl KeyValue {
    pub fn kind(&self) -> ValueKind {
        match self {
            KeyValue::Scalar(_) => ValueKind::Scalar,
            KeyValue::Vec3(_) => ValueKind::Vec3,
            KeyValue::Quat(_) => ValueKind::Quat,
            KeyValue::Curl(_) => ValueKind::Curl,
        }
    }

    pub fn is_finite(&self) -> bool {
        match self {
            KeyValue::Scalar(v) => v.is_finite(),
            KeyValue::Vec3(v) => v.iter().all(|c| c.is_finite()),
            KeyValue::Quat(v) => v.iter().all(|c| c.is_finite()),
            KeyValue::Curl(v) => v.iter().all(|c| c.is_finite()),
        }
    }

    /// Mistura `a → b` com o parâmetro já remapeado `t` (pode extrapolar). Tipos diferentes devolvem `a`.
    pub fn mix(a: &KeyValue, b: &KeyValue, t: f32) -> KeyValue {
        match (a, b) {
            (KeyValue::Scalar(x), KeyValue::Scalar(y)) => KeyValue::Scalar(x + (y - x) * t),
            (KeyValue::Vec3(x), KeyValue::Vec3(y)) => KeyValue::Vec3([x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t]),
            (KeyValue::Quat(x), KeyValue::Quat(y)) => KeyValue::Quat(slerp(*x, *y, t)),
            (KeyValue::Curl(x), KeyValue::Curl(y)) => {
                let mut out = [0.0; 5];
                for i in 0..5 {
                    out[i] = (x[i] + (y[i] - x[i]) * t).clamp(0.0, 1.0);
                }
                KeyValue::Curl(out)
            }
            _ => *a,
        }
    }

    /// Normaliza o que precisa (quaternion unitário com `w ≥ 0`, curvas em `[0, 1]`).
    pub fn normalized(self) -> KeyValue {
        match self {
            KeyValue::Quat(q) => KeyValue::Quat(canonical_quat(q)),
            KeyValue::Curl(c) => KeyValue::Curl(c.map(|v| v.clamp(0.0, 1.0))),
            other => other,
        }
    }

    pub fn as_scalar(&self) -> Option<f32> {
        if let KeyValue::Scalar(v) = self {
            Some(*v)
        } else {
            None
        }
    }
}

/// Quaternion unitário com `w ≥ 0`; entradas degeneradas viram identidade.
pub fn canonical_quat(q: [f32; 4]) -> [f32; 4] {
    let quat = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
    if !quat.is_finite() || quat.length_squared() < 1e-12 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let quat = quat.normalize();
    let quat = if quat.w < 0.0 { -quat } else { quat };
    quat.to_array()
}

/// Slerp pelo caminho curto (nega `b` se `dot < 0`), lerp normalizado quando quase paralelos e extrapolação estável
/// para `t` fora de `[0, 1]` (o ângulo é ampliado). Sempre devolve um quaternion unitário.
pub fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let qa = Quat::from_xyzw(a[0], a[1], a[2], a[3]).normalize();
    let mut qb = Quat::from_xyzw(b[0], b[1], b[2], b[3]).normalize();
    let mut dot = qa.dot(qb);
    if dot < 0.0 {
        qb = -qb;
        dot = -dot;
    }
    if dot > 0.9995 {
        // quase iguais: lerp normalizado (estável); extrapola linearmente em 4D e renormaliza
        let q = Quat::from_xyzw(
            qa.x + (qb.x - qa.x) * t,
            qa.y + (qb.y - qa.y) * t,
            qa.z + (qb.z - qa.z) * t,
            qa.w + (qb.w - qa.w) * t,
        );
        return if q.length_squared() > 1e-12 { q.normalize().to_array() } else { qa.to_array() };
    }
    let theta0 = dot.clamp(-1.0, 1.0).acos();
    let theta = theta0 * t;
    let sin0 = theta0.sin();
    let s0 = (theta0 - theta).sin() / sin0;
    let s1 = theta.sin() / sin0;
    let q = Quat::from_xyzw(qa.x * s0 + qb.x * s1, qa.y * s0 + qb.y * s1, qa.z * s0 + qb.z * s1, qa.w * s0 + qb.w * s1);
    q.normalize().to_array()
}

/// Easing de Bézier cúbica normalizada `(0,0) (x1,y1) (x2,y2) (1,1)`: dado `u` (tempo normalizado) devolve o progresso.
/// `x1, x2` são limitados a `[0, 1]` (monotonia no tempo). Newton com bisseção de reserva: determinística.
pub fn bezier_ease(c: [f32; 4], u: f32) -> f32 {
    let (x1, y1, x2, y2) = (c[0].clamp(0.0, 1.0) as f64, c[1] as f64, c[2].clamp(0.0, 1.0) as f64, c[3] as f64);
    let u64 = u as f64;
    // fora de [0,1]: continua com a tangente das pontas (extrapolação suave e contínua)
    if u64 <= 0.0 {
        let slope = if x1 > 1e-9 { y1 / x1 } else { 0.0 };
        return (slope * u64) as f32;
    }
    if u64 >= 1.0 {
        let dx = 1.0 - x2;
        let slope = if dx > 1e-9 { (1.0 - y2) / dx } else { 0.0 };
        return (1.0 + slope * (u64 - 1.0)) as f32;
    }
    let bx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * x1 + 3.0 * (1.0 - s) * s * s * x2 + s * s * s;
    let by = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * y1 + 3.0 * (1.0 - s) * s * s * y2 + s * s * s;
    let dbx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * x1 + 6.0 * (1.0 - s) * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
    let mut s = u64;
    let mut converged = false;
    for _ in 0..8 {
        let err = bx(s) - u64;
        if err.abs() < 1e-9 {
            converged = true;
            break;
        }
        let d = dbx(s);
        if d.abs() < 1e-9 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    if !converged {
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        s = u64;
        for _ in 0..60 {
            let e = bx(s) - u64;
            if e.abs() < 1e-9 {
                break;
            }
            if e > 0.0 {
                hi = s;
            } else {
                lo = s;
            }
            s = 0.5 * (lo + hi);
        }
    }
    by(s) as f32
}

/// Uma chave: valor no quadro `frame` e interpolação até a próxima.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub frame: i32,
    pub value: KeyValue,
    #[serde(default, skip_serializing_if = "Interp::is_default")]
    pub interp: Interp,
}

impl Key {
    pub fn new(frame: i32, value: KeyValue) -> Key {
        Key { frame, value, interp: Interp::Linear }
    }
    pub fn with_interp(mut self, interp: Interp) -> Key {
        self.interp = interp;
        self
    }
}

/// Um canal: alvo + chaves em ordem ESTRITAMENTE crescente de quadro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub target: ChannelTarget,
    pub keys: Vec<Key>,
}

impl Channel {
    /// Índice da chave EXATAMENTE em `frame`.
    pub fn key_index(&self, frame: i32) -> Option<usize> {
        self.keys.binary_search_by_key(&frame, |k| k.frame).ok()
    }

    pub fn key_at(&self, frame: i32) -> Option<&Key> {
        self.key_index(frame).map(|i| &self.keys[i])
    }

    /// Avalia o canal em `frame` (fracionário). `None` se não há chaves.
    pub fn evaluate(&self, frame: f32, extrapolation: Extrapolation) -> Option<KeyValue> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        let first = &self.keys[0];
        let last = &self.keys[n - 1];
        if n == 1 {
            return Some(first.value.normalized());
        }
        if frame <= first.frame as f32 {
            if extrapolation == Extrapolation::Linear && frame < first.frame as f32 {
                return Some(segment_value(first, &self.keys[1], frame, true));
            }
            return Some(first.value.normalized());
        }
        if frame >= last.frame as f32 {
            if extrapolation == Extrapolation::Linear && frame > last.frame as f32 {
                return Some(segment_value(&self.keys[n - 2], last, frame, true));
            }
            return Some(last.value.normalized());
        }
        // i = último índice com keys[i].frame <= frame
        let i = match self.keys.binary_search_by(|k| (k.frame as f32).partial_cmp(&frame).unwrap_or(std::cmp::Ordering::Less)) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Some(segment_value(&self.keys[i], &self.keys[i + 1], frame, false))
    }
}

/// Valor entre `a` e `b` (`a.frame < b.frame`) em `frame`. `linear_extrap` força interpolação linear para extrapolar.
fn segment_value(a: &Key, b: &Key, frame: f32, linear_extrap: bool) -> KeyValue {
    let span = (b.frame - a.frame) as f32;
    let u = (frame - a.frame as f32) / span;
    let eased = if linear_extrap {
        u
    } else {
        match a.interp {
            Interp::Step => return a.value.normalized(),
            Interp::Linear => u,
            other => match other.control_points() {
                Some(c) => bezier_ease(c, u),
                None => u,
            },
        }
    };
    KeyValue::mix(&a.value, &b.value, eased)
}

/// Marcador (nome em um quadro).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub frame: i32,
    pub name: String,
}

// ---------------------------------------------------------------------------------------------
// Clip: campos novos e avaliação
// ---------------------------------------------------------------------------------------------

/// Parâmetros de reprodução de um clip (inverso exato do `SetClipPlayback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClipPlayback {
    pub loop_mode: LoopMode,
    pub extrapolation: Extrapolation,
    pub hold_step: u8,
}

impl AnimationClip {
    /// Clip vazio com os campos novos no padrão.
    pub fn new(clip_id: AnimationClipId, name: impl Into<String>) -> AnimationClip {
        AnimationClip {
            clip_id,
            name: name.into(),
            duration_seconds: 0.0,
            tracks: Vec::new(),
            channels: Vec::new(),
            range: None,
            loop_mode: LoopMode::Once,
            extrapolation: Extrapolation::Constant,
            hold_step: 1,
            markers: Vec::new(),
        }
    }

    pub fn playback(&self) -> ClipPlayback {
        ClipPlayback { loop_mode: self.loop_mode, extrapolation: self.extrapolation, hold_step: self.hold_step.max(1) }
    }

    /// Menor e maior quadro de chave de todos os canais.
    pub fn key_extent(&self) -> Option<(i32, i32)> {
        let mut out: Option<(i32, i32)> = None;
        for ch in &self.channels {
            if let (Some(a), Some(b)) = (ch.keys.first(), ch.keys.last()) {
                out = Some(match out {
                    Some((lo, hi)) => (lo.min(a.frame), hi.max(b.frame)),
                    None => (a.frame, b.frame),
                });
            }
        }
        out
    }

    /// Faixa efetiva `(início, fim)` em quadros: a explícita, senão `0..=` (maior chave ou `duration_seconds·fps`).
    pub fn effective_range(&self, fps: f32) -> (i32, i32) {
        if let Some([a, b]) = self.range {
            return (a, b.max(a));
        }
        let dur = (self.duration_seconds * fps).round() as i32;
        let key_end = self.key_extent().map(|(_, hi)| hi).unwrap_or(0);
        let end = dur.max(key_end);
        (0, if end > 0 { end } else { fps.round() as i32 })
    }

    pub fn channel(&self, target: &ChannelTarget) -> Option<&Channel> {
        self.channels.binary_search_by(|c| c.target.cmp(target)).ok().map(|i| &self.channels[i])
    }

    pub fn key_count(&self) -> usize {
        self.channels.iter().map(|c| c.keys.len()).sum()
    }

    /// Quadros que têm alguma chave (únicos, em ordem).
    pub fn key_frames(&self) -> Vec<i32> {
        let set: BTreeSet<i32> = self.channels.iter().flat_map(|c| c.keys.iter().map(|k| k.frame)).collect();
        set.into_iter().collect()
    }

    /// Mapeia o quadro bruto do relógio para o quadro de avaliação: loop/ping-pong e "animar em Ns".
    pub fn map_frame(&self, fps: f32, raw: f32) -> f32 {
        let (start, end) = self.effective_range(fps);
        let (s, e) = (start as f32, end as f32);
        let len = e - s;
        let mut f = raw;
        if len > 0.0 {
            match self.loop_mode {
                LoopMode::Once => {}
                LoopMode::Loop => f = s + (raw - s).rem_euclid(len),
                LoopMode::PingPong => {
                    let p = (raw - s).rem_euclid(2.0 * len);
                    f = if p <= len { s + p } else { s + 2.0 * len - p };
                }
            }
        }
        let hold = self.hold_step.max(1) as f32;
        if hold > 1.0 {
            f = s + ((f - s) / hold).floor() * hold;
        }
        f
    }

    /// Avalia o clip no quadro bruto `raw` (fracionário): aplica loop/hold e avalia cada canal. Os `tracks` legados
    /// preenchem os alvos de nó que nenhum canal cobre. Determinístico.
    pub fn evaluate(&self, fps: f32, raw: f32) -> Sample {
        let f = self.map_frame(fps, raw);
        let mut values = BTreeMap::new();
        for ch in &self.channels {
            if let Some(v) = ch.evaluate(f, self.extrapolation) {
                values.insert(ch.target.clone(), v);
            }
        }
        // legado: tracks por nó (tempo em segundos, interpolação linear)
        let seconds = f / fps.max(MIN_FPS);
        for track in &self.tracks {
            if track.keyframes.is_empty() {
                continue;
            }
            let mut kfs: Vec<&crate::project::TransformKeyframe> = track.keyframes.iter().filter(|k| k.time_seconds.is_finite()).collect();
            if kfs.is_empty() {
                continue;
            }
            kfs.sort_by(|a, b| a.time_seconds.partial_cmp(&b.time_seconds).unwrap_or(std::cmp::Ordering::Equal));
            let (a, b, t) = legacy_segment(&kfs, seconds);
            let tr = KeyValue::mix(&KeyValue::Vec3(a.translation), &KeyValue::Vec3(b.translation), t);
            let rot = KeyValue::mix(&KeyValue::Quat(a.rotation), &KeyValue::Quat(b.rotation), t);
            let sc = KeyValue::mix(&KeyValue::Vec3(a.scale), &KeyValue::Vec3(b.scale), t);
            values.entry(ChannelTarget::NodeTranslation { node_id: track.node_id.clone() }).or_insert(tr);
            values.entry(ChannelTarget::NodeRotation { node_id: track.node_id.clone() }).or_insert(rot);
            values.entry(ChannelTarget::NodeScale { node_id: track.node_id.clone() }).or_insert(sc);
        }
        Sample { frame: f, values }
    }

    /// Valida estrutura e valores do clip (sem olhar o resto do projeto).
    pub fn validate_shape(&self) -> Result<(), String> {
        if !self.duration_seconds.is_finite() || self.duration_seconds < 0.0 {
            return Err("duration_seconds inválida".into());
        }
        if let Some([a, b]) = self.range {
            if !(0..=MAX_FRAME).contains(&a) || !(0..=MAX_FRAME).contains(&b) || b < a {
                return Err(format!("faixa do clip inválida [{a}, {b}]"));
            }
        }
        if !(1..=MAX_HOLD_STEP).contains(&self.hold_step) {
            return Err(format!("hold_step {} fora de 1..={MAX_HOLD_STEP}", self.hold_step));
        }
        if self.channels.len() > MAX_CHANNELS {
            return Err(format!("canais demais ({})", self.channels.len()));
        }
        if self.key_count() > MAX_KEYS_PER_CLIP {
            return Err("chaves demais no clip".into());
        }
        if self.markers.len() > MAX_MARKERS {
            return Err("marcadores demais".into());
        }
        for m in &self.markers {
            if !(0..=MAX_FRAME).contains(&m.frame) || m.name.chars().count() > 64 {
                return Err(format!("marcador inválido '{}'@{}", m.name, m.frame));
            }
        }
        let mut prev: Option<&ChannelTarget> = None;
        for ch in &self.channels {
            if let Some(p) = prev {
                if p >= &ch.target {
                    return Err(format!("canais fora de ordem ou repetidos: {}", ch.target.key_string()));
                }
            }
            prev = Some(&ch.target);
            validate_target(&ch.target)?;
            if ch.keys.is_empty() {
                return Err(format!("canal vazio: {}", ch.target.key_string()));
            }
            let mut last = i32::MIN;
            for k in &ch.keys {
                if !(0..=MAX_FRAME).contains(&k.frame) {
                    return Err(format!("quadro {} fora de 0..={MAX_FRAME} em {}", k.frame, ch.target.key_string()));
                }
                if k.frame <= last {
                    return Err(format!("chaves fora de ordem em {}", ch.target.key_string()));
                }
                last = k.frame;
                if k.value.kind() != ch.target.value_kind() {
                    return Err(format!("tipo de valor não casa com {}", ch.target.key_string()));
                }
                if !k.value.is_finite() {
                    return Err(format!("valor não finito em {}@{}", ch.target.key_string(), k.frame));
                }
                validate_value(&ch.target, &k.value)?;
                if let Interp::Bezier { c } = k.interp {
                    if c.iter().any(|v| !v.is_finite()) || c[0] < 0.0 || c[0] > 1.0 || c[2] < 0.0 || c[2] > 1.0 {
                        return Err(format!("alças Bézier inválidas em {}@{}", ch.target.key_string(), k.frame));
                    }
                }
            }
        }
        Ok(())
    }

    /// Reparo de um clip lido de arquivo: ordena, remove duplicatas/valores inválidos/canais vazios/alvos desconhecidos.
    pub fn sanitize(&mut self, known_nodes: &BTreeSet<NodeId>) {
        if !self.duration_seconds.is_finite() || self.duration_seconds < 0.0 {
            self.duration_seconds = 0.0;
        }
        if let Some([a, b]) = self.range {
            let a = a.clamp(0, MAX_FRAME);
            let b = b.clamp(a, MAX_FRAME);
            self.range = Some([a, b]);
        }
        self.hold_step = self.hold_step.clamp(1, MAX_HOLD_STEP);
        let mut by_target: BTreeMap<ChannelTarget, BTreeMap<i32, Key>> = BTreeMap::new();
        for ch in std::mem::take(&mut self.channels) {
            if validate_target(&ch.target).is_err() {
                continue;
            }
            let node_ok = match &ch.target {
                ChannelTarget::NodeTranslation { node_id } | ChannelTarget::NodeRotation { node_id } | ChannelTarget::NodeScale { node_id } => known_nodes.contains(node_id),
                _ => true,
            };
            if !node_ok {
                continue;
            }
            let entry = by_target.entry(ch.target.clone()).or_default();
            for mut k in ch.keys {
                if !(0..=MAX_FRAME).contains(&k.frame) || k.value.kind() != ch.target.value_kind() || !k.value.is_finite() {
                    continue;
                }
                k.value = clamp_value(&ch.target, k.value.normalized());
                k.interp = k.interp.sanitized();
                entry.insert(k.frame, k);
            }
        }
        self.channels = by_target
            .into_iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(target, m)| Channel { target, keys: m.into_values().collect() })
            .take(MAX_CHANNELS)
            .collect();
        self.markers.retain(|m| (0..=MAX_FRAME).contains(&m.frame));
        self.markers.truncate(MAX_MARKERS);
        for m in &mut self.markers {
            m.name = m.name.chars().take(64).collect();
        }
    }
}

fn legacy_segment<'a>(kfs: &[&'a crate::project::TransformKeyframe], seconds: f32) -> (&'a crate::project::TransformKeyframe, &'a crate::project::TransformKeyframe, f32) {
    let first = kfs[0];
    let last = kfs[kfs.len() - 1];
    if kfs.len() == 1 || seconds <= first.time_seconds {
        return (first, first, 0.0);
    }
    if seconds >= last.time_seconds {
        return (last, last, 0.0);
    }
    let mut i = 0;
    while i + 1 < kfs.len() && kfs[i + 1].time_seconds <= seconds {
        i += 1;
    }
    let (a, b) = (kfs[i], kfs[i + 1]);
    let span = (b.time_seconds - a.time_seconds).max(1e-9);
    (a, b, ((seconds - a.time_seconds) / span).clamp(0.0, 1.0))
}

fn validate_target(t: &ChannelTarget) -> Result<(), String> {
    if let ChannelTarget::Bone { bone } = t {
        if bone_index(bone).is_none() {
            return Err(format!("osso desconhecido '{bone}'"));
        }
    }
    Ok(())
}

fn validate_value(t: &ChannelTarget, v: &KeyValue) -> Result<(), String> {
    match (t, v) {
        (_, KeyValue::Quat(q)) => {
            let len2: f32 = q.iter().map(|c| c * c).sum();
            if !(0.25..=4.0).contains(&len2) {
                return Err(format!("quaternion não unitário em {}", t.key_string()));
            }
        }
        (ChannelTarget::Root | ChannelTarget::Hips, KeyValue::Vec3(p)) => {
            if p.iter().any(|c| c.abs() > MAX_TRANSLATION) {
                return Err(format!("translação fora de ±{MAX_TRANSLATION} m em {}", t.key_string()));
            }
        }
        (_, KeyValue::Curl(c)) => {
            if c.iter().any(|v| !(-1e-4..=1.0 + 1e-4).contains(v)) {
                return Err("curvatura de dedo fora de [0, 1]".into());
            }
        }
        (ChannelTarget::Camera { prop: CameraProp::Fov }, KeyValue::Scalar(f)) => {
            if !(1.0..=170.0).contains(f) {
                return Err(format!("FOV {f} fora de 1..=170 graus"));
            }
        }
        (ChannelTarget::Sun { prop: SunProp::Intensity }, KeyValue::Scalar(i)) => {
            if !(0.0..=100.0).contains(i) {
                return Err(format!("intensidade do sol {i} fora de 0..=100"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Limita um valor às faixas válidas do alvo (repara; usado na avaliação e na importação).
pub fn clamp_value(t: &ChannelTarget, v: KeyValue) -> KeyValue {
    match (t, v) {
        (ChannelTarget::Root | ChannelTarget::Hips, KeyValue::Vec3(p)) => KeyValue::Vec3(p.map(|c| c.clamp(-MAX_TRANSLATION, MAX_TRANSLATION))),
        (ChannelTarget::Camera { prop: CameraProp::Fov }, KeyValue::Scalar(f)) => KeyValue::Scalar(f.clamp(1.0, 170.0)),
        (ChannelTarget::Sun { prop: SunProp::Intensity }, KeyValue::Scalar(i)) => KeyValue::Scalar(i.clamp(0.0, 100.0)),
        (_, other) => other,
    }
}

// ---------------------------------------------------------------------------------------------
// Amostra e aplicação ao projeto
// ---------------------------------------------------------------------------------------------

/// Resultado de avaliar um clip: valor por alvo (só os alvos que o clip anima).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// Quadro de avaliação (depois de loop/hold).
    pub frame: f32,
    pub values: BTreeMap<ChannelTarget, KeyValue>,
}

impl Sample {
    /// Sobrepõe os canais de personagem desta amostra sobre `base` (os ossos que o clip não anima ficam como estão).
    pub fn apply_to_pose(&self, base: &PoseState) -> PoseState {
        let mut pose = base.clone();
        for (target, value) in &self.values {
            match (target, value) {
                (ChannelTarget::Bone { bone }, KeyValue::Quat(q)) => {
                    if let Some(i) = bone_index(bone) {
                        pose.set_rotation(i, Quat::from_xyzw(q[0], q[1], q[2], q[3]));
                    }
                }
                (ChannelTarget::Root, KeyValue::Vec3(p)) => pose.root_translation = p.map(|c| c.clamp(-MAX_TRANSLATION, MAX_TRANSLATION)),
                (ChannelTarget::Hips, KeyValue::Vec3(p)) => pose.hips_translation = p.map(|c| c.clamp(-MAX_TRANSLATION, MAX_TRANSLATION)),
                (ChannelTarget::Hand { side }, KeyValue::Curl(c)) => {
                    let c = c.map(|v| v.clamp(0.0, 1.0));
                    match side {
                        HandSide::Left => pose.hands.left = c,
                        HandSide::Right => pose.hands.right = c,
                    }
                }
                _ => {}
            }
        }
        pose
    }

    /// `true` se a amostra mexe em algum canal do personagem (pose).
    pub fn has_character(&self) -> bool {
        self.values.keys().any(|t| t.group() == ChannelGroup::Character)
    }

    /// Aplica a amostra ao projeto SEM histórico (pose, transform de nós, sol e câmera de cena). Devolve `true` se
    /// algo mudou (evita bumpar revisões quando a amostra é igual à anterior).
    pub fn apply_to_project(&self, project: &mut ProjectState) -> bool {
        let mut changed = false;
        if self.has_character() {
            let next = self.apply_to_pose(&project.character.pose);
            if next != project.character.pose {
                project.character.pose = next;
                changed = true;
            }
        }
        for (target, value) in &self.values {
            match (target, value) {
                (ChannelTarget::NodeTranslation { node_id }, KeyValue::Vec3(p)) => {
                    if let Some(n) = project.scene.node_mut(node_id) {
                        let v = Vec3::from_array(*p);
                        if n.transform.translation != v {
                            n.transform.translation = v;
                            changed = true;
                        }
                    }
                }
                (ChannelTarget::NodeRotation { node_id }, KeyValue::Quat(q)) => {
                    if let Some(n) = project.scene.node_mut(node_id) {
                        let v = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
                        if n.transform.rotation != v {
                            n.transform.rotation = v;
                            changed = true;
                        }
                    }
                }
                (ChannelTarget::NodeScale { node_id }, KeyValue::Vec3(p)) => {
                    if let Some(n) = project.scene.node_mut(node_id) {
                        let v = Vec3::from_array(*p);
                        if n.transform.scale != v {
                            n.transform.scale = v;
                            changed = true;
                        }
                    }
                }
                (ChannelTarget::Sun { prop }, v) => {
                    if let Some(slot) = project.scene.lights.first_mut() {
                        let l = &mut slot.light;
                        match (prop, v) {
                            (SunProp::Intensity, KeyValue::Scalar(x)) => {
                                if l.intensity != *x {
                                    l.intensity = *x;
                                    changed = true;
                                }
                            }
                            (SunProp::Direction, KeyValue::Vec3(d)) => {
                                let dir = Vec3::from_array(*d);
                                let dir = if dir.length_squared() > 1e-12 { dir.normalize().to_array() } else { l.direction };
                                if l.direction != dir {
                                    l.direction = dir;
                                    changed = true;
                                }
                            }
                            (SunProp::Color, KeyValue::Vec3(c)) => {
                                if l.color != *c {
                                    l.color = *c;
                                    changed = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                (ChannelTarget::Camera { prop }, v) => {
                    let cam = &mut project.scene.camera.camera;
                    match (prop, v) {
                        (CameraProp::Fov, KeyValue::Scalar(deg)) => {
                            let r = deg.to_radians();
                            if cam.fov_y != r {
                                cam.fov_y = r;
                                changed = true;
                            }
                        }
                        (CameraProp::Eye, KeyValue::Vec3(p)) => {
                            let v = Vec3::from_array(*p);
                            if cam.eye != v {
                                cam.eye = v;
                                changed = true;
                            }
                        }
                        (CameraProp::Target, KeyValue::Vec3(p)) => {
                            let v = Vec3::from_array(*p);
                            if cam.target != v {
                                cam.target = v;
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        changed
    }
}

/// Valor ATUAL de um alvo no projeto (para criar chaves a partir do estado vivo).
pub fn current_value(project: &ProjectState, target: &ChannelTarget) -> Option<KeyValue> {
    let pose = &project.character.pose;
    Some(match target {
        ChannelTarget::Bone { bone } => {
            let i = bone_index(bone)?;
            KeyValue::Quat(canonical_quat(pose.rotation(i).to_array()))
        }
        ChannelTarget::Root => KeyValue::Vec3(pose.root_translation),
        ChannelTarget::Hips => KeyValue::Vec3(pose.hips_translation),
        ChannelTarget::Hand { side } => KeyValue::Curl(match side {
            HandSide::Left => pose.hands.left,
            HandSide::Right => pose.hands.right,
        }),
        ChannelTarget::NodeTranslation { node_id } => KeyValue::Vec3(project.scene.node(node_id)?.transform.translation.to_array()),
        ChannelTarget::NodeRotation { node_id } => KeyValue::Quat(canonical_quat(project.scene.node(node_id)?.transform.rotation.to_array())),
        ChannelTarget::NodeScale { node_id } => KeyValue::Vec3(project.scene.node(node_id)?.transform.scale.to_array()),
        ChannelTarget::Sun { prop } => {
            let l = &project.scene.lights.first()?.light;
            match prop {
                SunProp::Intensity => KeyValue::Scalar(l.intensity),
                SunProp::Direction => KeyValue::Vec3(l.direction),
                SunProp::Color => KeyValue::Vec3(l.color),
            }
        }
        ChannelTarget::Camera { prop } => {
            let c = &project.scene.camera.camera;
            match prop {
                CameraProp::Fov => KeyValue::Scalar(c.fov_y.to_degrees()),
                CameraProp::Eye => KeyValue::Vec3(c.eye.to_array()),
                CameraProp::Target => KeyValue::Vec3(c.target.to_array()),
            }
        }
    })
}

/// Alvos de pose de corpo inteiro: 24 ossos + translação do root/quadril + as duas mãos (pose-to-pose do anime).
pub fn full_pose_targets() -> Vec<ChannelTarget> {
    let mut v: Vec<ChannelTarget> = (0..BONE_NAMES.len()).map(ChannelTarget::bone_index).collect();
    v.push(ChannelTarget::Root);
    v.push(ChannelTarget::Hips);
    v.push(ChannelTarget::Hand { side: HandSide::Left });
    v.push(ChannelTarget::Hand { side: HandSide::Right });
    v
}

/// Alvos que um `SetPose` toca (ossos, translações e mãos informados) — o auto-key grava só estes.
pub fn targets_of_pose_edit(bones: &[String], root: bool, hips: bool, hands: bool) -> Vec<ChannelTarget> {
    let mut v: Vec<ChannelTarget> = bones.iter().filter(|b| bone_index(b).is_some()).map(|b| ChannelTarget::bone(b)).collect();
    if root {
        v.push(ChannelTarget::Root);
    }
    if hips {
        v.push(ChannelTarget::Hips);
    }
    if hands {
        v.push(ChannelTarget::Hand { side: HandSide::Left });
        v.push(ChannelTarget::Hand { side: HandSide::Right });
    }
    v
}

/// Lê o valor de um alvo de personagem numa `PoseState` (para criar chaves a partir de uma pose que não é a do projeto).
pub fn pose_value(pose: &PoseState, target: &ChannelTarget) -> Option<KeyValue> {
    Some(match target {
        ChannelTarget::Bone { bone } => KeyValue::Quat(canonical_quat(pose.rotation(bone_index(bone)?).to_array())),
        ChannelTarget::Root => KeyValue::Vec3(pose.root_translation),
        ChannelTarget::Hips => KeyValue::Vec3(pose.hips_translation),
        ChannelTarget::Hand { side } => KeyValue::Curl(if *side == HandSide::Left { pose.hands.left } else { pose.hands.right }),
        _ => return None,
    })
}

/// Chaves de `pose` nos `targets` do quadro `frame` (alvos que não são de personagem são ignorados).
pub fn keys_from_pose(pose: &PoseState, targets: &[ChannelTarget], frame: i32, interp: Interp) -> Vec<KeyInsert> {
    targets
        .iter()
        .filter_map(|t| pose_value(pose, t).map(|v| KeyInsert { target: t.clone(), key: Key { frame, value: v, interp } }))
        .collect()
}

/// Espelha uma chave: devolve o alvo oposto e a chave refletida no plano X (`q' = (x, −y, −z, w)`, `tx → −tx`).
pub fn mirror_key(target: &ChannelTarget, key: &Key) -> Option<(ChannelTarget, Key)> {
    let to = target.mirrored()?;
    let value = match (&key.value, target) {
        (KeyValue::Quat(q), ChannelTarget::Bone { .. }) => KeyValue::Quat(canonical_quat([q[0], -q[1], -q[2], q[3]])),
        (KeyValue::Vec3(p), ChannelTarget::Root | ChannelTarget::Hips) => KeyValue::Vec3([-p[0], p[1], p[2]]),
        (v, _) => *v,
    };
    Some((to, Key { frame: key.frame, value, interp: key.interp }))
}

/// Converte quadro → segundos.
pub fn frame_to_seconds(frame: f32, fps: f32) -> f32 {
    frame / fps.max(MIN_FPS)
}

/// Converte segundos → quadro (fracionário).
pub fn seconds_to_frame(seconds: f32, fps: f32) -> f32 {
    seconds * fps.max(MIN_FPS)
}

/// Timecode `HH:MM:SS:FF` (sem drop-frame; a base de FF é o fps arredondado).
pub fn timecode(frame: i32, fps: f32) -> String {
    let base = fps.round().max(1.0) as i64;
    let total = frame.max(0) as i64;
    let ff = total % base;
    let secs = total / base;
    format!("{:02}:{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60, ff)
}

// ---------------------------------------------------------------------------------------------
// Edições (comando EditAnimation)
// ---------------------------------------------------------------------------------------------

/// Chave a inserir/substituir no canal `target` (cria o canal se não existir).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyInsert {
    pub target: ChannelTarget,
    pub key: Key,
}

/// Referência a uma chave existente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRef {
    pub target: ChannelTarget,
    pub frame: i32,
}

/// Move a chave de `frame` para `to_frame` no mesmo canal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMove {
    pub target: ChannelTarget,
    pub frame: i32,
    pub to_frame: i32,
}

/// Erro de uma edição de animação.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimError {
    /// Nada mudaria.
    NoOp(String),
    Invalid(String),
    Unknown { kind: &'static str, target: String },
}

impl std::fmt::Display for AnimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnimError::NoOp(m) => write!(f, "command has no effect: {m}"),
            AnimError::Invalid(m) => write!(f, "{m}"),
            AnimError::Unknown { kind, target } => write!(f, "unknown {kind} '{target}'"),
        }
    }
}

/// Uma edição atômica da animação do projeto. Cada variante tem inverso exato ([`AnimEdit::inverse`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AnimEdit {
    /// Cria o clip ou SUBSTITUI o clip de mesmo id por inteiro (também é o inverso de várias edições).
    PutClip { clip: AnimationClip },
    /// Remove o clip (e desativa se era o ativo).
    RemoveClip { clip_id: AnimationClipId },
    /// Define o clip ativo (`None` = nenhum).
    SetActiveClip {
        #[serde(default)]
        clip_id: Option<AnimationClipId>,
    },
    RenameClip { clip_id: AnimationClipId, name: String },
    /// Insere/substitui chaves.
    InsertKeys { clip_id: AnimationClipId, keys: Vec<KeyInsert> },
    DeleteKeys { clip_id: AnimationClipId, keys: Vec<KeyRef> },
    MoveKeys { clip_id: AnimationClipId, moves: Vec<KeyMove> },
    /// Muda a interpolação (da chave até a próxima) das chaves indicadas.
    SetInterpolation { clip_id: AnimationClipId, keys: Vec<KeyRef>, interp: Interp },
    /// Faixa de reprodução do clip em quadros (`None` = automática: 0..maior chave).
    SetClipRange {
        clip_id: AnimationClipId,
        #[serde(default)]
        range: Option<[i32; 2]>,
    },
    /// Loop/ping-pong, extrapolação e "animar em Ns" (só o que vier preenchido muda).
    SetClipPlayback {
        clip_id: AnimationClipId,
        #[serde(default)]
        loop_mode: Option<LoopMode>,
        #[serde(default)]
        extrapolation: Option<Extrapolation>,
        #[serde(default)]
        hold_step: Option<u8>,
    },
    /// Taxa de quadros do projeto. Com `retime`, os quadros de todos os clips são escalados (a duração em segundos se mantém).
    SetFps {
        fps: f32,
        #[serde(default)]
        retime: bool,
    },
    SetMarkers { clip_id: AnimationClipId, markers: Vec<Marker> },
}

impl AnimEdit {
    /// Descrição curta (histórico). Em inglês estável; a UI traduz pelo prefixo.
    pub fn description(&self) -> String {
        match self {
            AnimEdit::PutClip { clip } => format!("Animation clip {}", clip.name),
            AnimEdit::RemoveClip { .. } => "Remove animation clip".to_string(),
            AnimEdit::SetActiveClip { .. } => "Active animation clip".to_string(),
            AnimEdit::RenameClip { name, .. } => format!("Rename animation clip {name}"),
            AnimEdit::InsertKeys { keys, .. } => format!("Insert {} keys", keys.len()),
            AnimEdit::DeleteKeys { keys, .. } => format!("Delete {} keys", keys.len()),
            AnimEdit::MoveKeys { moves, .. } => format!("Move {} keys", moves.len()),
            AnimEdit::SetInterpolation { keys, interp, .. } => format!("Interpolation {} ({} keys)", interp.label(), keys.len()),
            AnimEdit::SetClipRange { .. } => "Animation range".to_string(),
            AnimEdit::SetClipPlayback { .. } => "Animation playback".to_string(),
            AnimEdit::SetFps { fps, .. } => format!("Frame rate {fps}"),
            AnimEdit::SetMarkers { .. } => "Animation markers".to_string(),
        }
    }

    /// Id do clip afetado (telemetria).
    pub fn clip_id(&self) -> Option<&AnimationClipId> {
        match self {
            AnimEdit::PutClip { clip } => Some(&clip.clip_id),
            AnimEdit::RemoveClip { clip_id }
            | AnimEdit::RenameClip { clip_id, .. }
            | AnimEdit::InsertKeys { clip_id, .. }
            | AnimEdit::DeleteKeys { clip_id, .. }
            | AnimEdit::MoveKeys { clip_id, .. }
            | AnimEdit::SetInterpolation { clip_id, .. }
            | AnimEdit::SetClipRange { clip_id, .. }
            | AnimEdit::SetClipPlayback { clip_id, .. }
            | AnimEdit::SetMarkers { clip_id, .. } => Some(clip_id),
            AnimEdit::SetActiveClip { clip_id } => clip_id.as_ref(),
            AnimEdit::SetFps { .. } => None,
        }
    }

    /// Valida contra o projeto (alvos de nó existentes, clip existente, valores) e recusa edições sem efeito.
    pub fn validate(&self, project: &ProjectState) -> Result<(), AnimError> {
        let nodes: BTreeSet<&NodeId> = project.scene.nodes.iter().map(|n| &n.node_id).collect();
        let check_target = |t: &ChannelTarget| -> Result<(), AnimError> {
            validate_target(t).map_err(|_| AnimError::Unknown { kind: "bone", target: t.key_string() })?;
            match t {
                ChannelTarget::NodeTranslation { node_id } | ChannelTarget::NodeRotation { node_id } | ChannelTarget::NodeScale { node_id } => {
                    if !nodes.contains(node_id) {
                        return Err(AnimError::Unknown { kind: "scene node", target: node_id.to_string() });
                    }
                }
                _ => {}
            }
            Ok(())
        };
        match self {
            AnimEdit::PutClip { clip } => {
                clip.validate_shape().map_err(AnimError::Invalid)?;
                for ch in &clip.channels {
                    check_target(&ch.target)?;
                }
                for tr in &clip.tracks {
                    if !nodes.contains(&tr.node_id) {
                        return Err(AnimError::Unknown { kind: "scene node", target: tr.node_id.to_string() });
                    }
                }
                if !project.animation.clips.contains_key(&clip.clip_id) && project.animation.clips.len() >= MAX_CLIPS {
                    return Err(AnimError::Invalid(format!("no máximo {MAX_CLIPS} clips")));
                }
            }
            AnimEdit::InsertKeys { keys, .. } => {
                if keys.is_empty() {
                    return Err(AnimError::NoOp("no keys".into()));
                }
                let mut seen = BTreeSet::new();
                for k in keys {
                    check_target(&k.target)?;
                    if k.key.value.kind() != k.target.value_kind() {
                        return Err(AnimError::Invalid(format!("value type does not match channel {}", k.target.key_string())));
                    }
                    if !k.key.value.is_finite() {
                        return Err(AnimError::Invalid("non-finite key value".into()));
                    }
                    if !(0..=MAX_FRAME).contains(&k.key.frame) {
                        return Err(AnimError::Invalid(format!("frame {} outside 0..={MAX_FRAME}", k.key.frame)));
                    }
                    validate_value(&k.target, &k.key.value).map_err(AnimError::Invalid)?;
                    if let Interp::Bezier { c } = k.key.interp {
                        if c.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&c[0]) || !(0.0..=1.0).contains(&c[2]) {
                            return Err(AnimError::Invalid("invalid bezier handles".into()));
                        }
                    }
                    if !seen.insert((k.target.clone(), k.key.frame)) {
                        return Err(AnimError::Invalid(format!("key {}@{} listed twice", k.target.key_string(), k.key.frame)));
                    }
                }
            }
            AnimEdit::SetFps { fps, .. } => {
                if !fps.is_finite() || !(MIN_FPS..=MAX_FPS).contains(fps) {
                    return Err(AnimError::Invalid(format!("fps must be within [{MIN_FPS}, {MAX_FPS}]")));
                }
            }
            AnimEdit::SetClipPlayback { hold_step: Some(h), .. } if !(1..=MAX_HOLD_STEP).contains(h) => {
                return Err(AnimError::Invalid(format!("hold_step must be within 1..={MAX_HOLD_STEP}")));
            }
            AnimEdit::SetClipRange { range: Some([a, b]), .. } if !(0..=MAX_FRAME).contains(a) || !(0..=MAX_FRAME).contains(b) || b < a => {
                return Err(AnimError::Invalid(format!("invalid range [{a}, {b}]")));
            }
            AnimEdit::SetMarkers { markers, .. } => {
                if markers.len() > MAX_MARKERS {
                    return Err(AnimError::Invalid("too many markers".into()));
                }
                for m in markers {
                    if !(0..=MAX_FRAME).contains(&m.frame) || m.name.chars().count() > 64 {
                        return Err(AnimError::Invalid(format!("invalid marker '{}'", m.name)));
                    }
                }
            }
            AnimEdit::RenameClip { name, .. } if name.trim().is_empty() || name.chars().count() > 80 => {
                return Err(AnimError::Invalid("clip name must have 1..=80 characters".into()));
            }
            _ => {}
        }
        // o resto (alvos inexistentes, sem efeito…) sai de um ensaio sobre uma cópia
        let mut scratch = project.animation.clone();
        self.apply(&mut scratch)?;
        if scratch == project.animation {
            return Err(AnimError::NoOp("animation already up to date".into()));
        }
        Ok(())
    }

    /// Aplica a edição ao estado de animação (sem olhar o resto do projeto). Mantém a forma canônica.
    pub fn apply(&self, anim: &mut AnimationState) -> Result<(), AnimError> {
        let clip_mut = |anim: &mut AnimationState, id: &AnimationClipId| -> Result<(), AnimError> {
            if anim.clips.contains_key(id) {
                Ok(())
            } else {
                Err(AnimError::Unknown { kind: "animation clip", target: id.to_string() })
            }
        };
        match self {
            AnimEdit::PutClip { clip } => {
                let mut clip = clip.clone();
                canonicalize_clip(&mut clip);
                anim.clips.insert(clip.clip_id.clone(), clip);
            }
            AnimEdit::RemoveClip { clip_id } => {
                clip_mut(anim, clip_id)?;
                anim.clips.remove(clip_id);
                if anim.active_clip.as_ref() == Some(clip_id) {
                    anim.active_clip = None;
                }
            }
            AnimEdit::SetActiveClip { clip_id } => {
                if let Some(id) = clip_id {
                    clip_mut(anim, id)?;
                }
                anim.active_clip = clip_id.clone();
            }
            AnimEdit::RenameClip { clip_id, name } => {
                clip_mut(anim, clip_id)?;
                if let Some(c) = anim.clips.get_mut(clip_id) {
                    c.name = name.trim().to_string();
                }
            }
            AnimEdit::InsertKeys { clip_id, keys } => {
                clip_mut(anim, clip_id)?;
                let clip = anim.clips.get_mut(clip_id).ok_or_else(|| AnimError::Unknown { kind: "animation clip", target: clip_id.to_string() })?;
                for ins in keys {
                    let mut key = ins.key;
                    key.value = clamp_value(&ins.target, key.value.normalized());
                    key.interp = key.interp.sanitized();
                    insert_key(clip, &ins.target, key);
                }
            }
            AnimEdit::DeleteKeys { clip_id, keys } => {
                clip_mut(anim, clip_id)?;
                let clip = anim.clips.get_mut(clip_id).ok_or_else(|| AnimError::Unknown { kind: "animation clip", target: clip_id.to_string() })?;
                for r in keys {
                    if !remove_key(clip, &r.target, r.frame) {
                        return Err(AnimError::Unknown { kind: "key", target: format!("{}@{}", r.target.key_string(), r.frame) });
                    }
                }
            }
            AnimEdit::MoveKeys { clip_id, moves } => {
                clip_mut(anim, clip_id)?;
                let clip = anim.clips.get_mut(clip_id).ok_or_else(|| AnimError::Unknown { kind: "animation clip", target: clip_id.to_string() })?;
                apply_moves(clip, moves)?;
            }
            AnimEdit::SetInterpolation { clip_id, keys, interp } => {
                clip_mut(anim, clip_id)?;
                let clip = anim.clips.get_mut(clip_id).ok_or_else(|| AnimError::Unknown { kind: "animation clip", target: clip_id.to_string() })?;
                let interp = interp.sanitized();
                for r in keys {
                    let ch = channel_mut(clip, &r.target).ok_or_else(|| AnimError::Unknown { kind: "channel", target: r.target.key_string() })?;
                    let i = ch.key_index(r.frame).ok_or_else(|| AnimError::Unknown { kind: "key", target: format!("{}@{}", r.target.key_string(), r.frame) })?;
                    ch.keys[i].interp = interp;
                }
            }
            AnimEdit::SetClipRange { clip_id, range } => {
                clip_mut(anim, clip_id)?;
                if let Some(c) = anim.clips.get_mut(clip_id) {
                    c.range = *range;
                }
            }
            AnimEdit::SetClipPlayback { clip_id, loop_mode, extrapolation, hold_step } => {
                clip_mut(anim, clip_id)?;
                if let Some(c) = anim.clips.get_mut(clip_id) {
                    if let Some(l) = loop_mode {
                        c.loop_mode = *l;
                    }
                    if let Some(e) = extrapolation {
                        c.extrapolation = *e;
                    }
                    if let Some(h) = hold_step {
                        c.hold_step = *h;
                    }
                }
            }
            AnimEdit::SetFps { fps, retime } => {
                let old = anim.fps;
                if *retime && (old - *fps).abs() > 1e-6 {
                    for clip in anim.clips.values_mut() {
                        retime_clip(clip, old, *fps);
                    }
                }
                anim.fps = *fps;
            }
            AnimEdit::SetMarkers { clip_id, markers } => {
                clip_mut(anim, clip_id)?;
                if let Some(c) = anim.clips.get_mut(clip_id) {
                    let mut m = markers.clone();
                    m.sort_by(|a, b| a.frame.cmp(&b.frame).then_with(|| a.name.cmp(&b.name)));
                    c.markers = m;
                }
            }
        }
        Ok(())
    }

    /// Edições (em ordem de aplicação) que restauram EXATAMENTE o estado `anim` (anterior a esta edição).
    pub fn inverse(&self, anim: &AnimationState) -> Result<Vec<AnimEdit>, AnimError> {
        let get = |id: &AnimationClipId| anim.clips.get(id).ok_or_else(|| AnimError::Unknown { kind: "animation clip", target: id.to_string() });
        Ok(match self {
            AnimEdit::PutClip { clip } => match anim.clips.get(&clip.clip_id) {
                Some(old) => vec![AnimEdit::PutClip { clip: old.clone() }],
                None => vec![AnimEdit::RemoveClip { clip_id: clip.clip_id.clone() }],
            },
            AnimEdit::RemoveClip { clip_id } => {
                let old = get(clip_id)?.clone();
                let mut v = vec![AnimEdit::PutClip { clip: old }];
                if anim.active_clip.as_ref() == Some(clip_id) {
                    v.push(AnimEdit::SetActiveClip { clip_id: Some(clip_id.clone()) });
                }
                v
            }
            AnimEdit::SetActiveClip { .. } => vec![AnimEdit::SetActiveClip { clip_id: anim.active_clip.clone() }],
            AnimEdit::RenameClip { clip_id, .. } => vec![AnimEdit::RenameClip { clip_id: clip_id.clone(), name: get(clip_id)?.name.clone() }],
            AnimEdit::InsertKeys { clip_id, keys } => {
                let clip = get(clip_id)?;
                let mut restore = Vec::new();
                let mut remove = Vec::new();
                for ins in keys {
                    match clip.channel(&ins.target).and_then(|c| c.key_at(ins.key.frame)) {
                        Some(old) => restore.push(KeyInsert { target: ins.target.clone(), key: *old }),
                        None => remove.push(KeyRef { target: ins.target.clone(), frame: ins.key.frame }),
                    }
                }
                let mut v = Vec::new();
                if !restore.is_empty() {
                    v.push(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys: restore });
                }
                if !remove.is_empty() {
                    v.push(AnimEdit::DeleteKeys { clip_id: clip_id.clone(), keys: remove });
                }
                v
            }
            AnimEdit::DeleteKeys { clip_id, keys } => {
                let clip = get(clip_id)?;
                let mut restore = Vec::new();
                for r in keys {
                    let k = clip.channel(&r.target).and_then(|c| c.key_at(r.frame)).ok_or_else(|| AnimError::Unknown { kind: "key", target: format!("{}@{}", r.target.key_string(), r.frame) })?;
                    restore.push(KeyInsert { target: r.target.clone(), key: *k });
                }
                vec![AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys: restore }]
            }
            AnimEdit::MoveKeys { clip_id, moves } => {
                let clip = get(clip_id)?;
                // chaves não movidas que o destino sobrescreve
                let moving: BTreeSet<(ChannelTarget, i32)> = moves.iter().map(|m| (m.target.clone(), m.frame)).collect();
                let mut overwritten = Vec::new();
                for m in moves {
                    if moving.contains(&(m.target.clone(), m.to_frame)) {
                        continue;
                    }
                    if let Some(k) = clip.channel(&m.target).and_then(|c| c.key_at(m.to_frame)) {
                        overwritten.push(KeyInsert { target: m.target.clone(), key: *k });
                    }
                }
                let back: Vec<KeyMove> = moves.iter().map(|m| KeyMove { target: m.target.clone(), frame: m.to_frame, to_frame: m.frame }).collect();
                let mut v = vec![AnimEdit::MoveKeys { clip_id: clip_id.clone(), moves: back }];
                if !overwritten.is_empty() {
                    v.push(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys: overwritten });
                }
                v
            }
            AnimEdit::SetInterpolation { clip_id, keys, .. } => {
                let clip = get(clip_id)?;
                let mut groups: Vec<(Interp, Vec<KeyRef>)> = Vec::new();
                for r in keys {
                    let k = clip.channel(&r.target).and_then(|c| c.key_at(r.frame)).ok_or_else(|| AnimError::Unknown { kind: "key", target: format!("{}@{}", r.target.key_string(), r.frame) })?;
                    match groups.iter_mut().find(|(i, _)| *i == k.interp) {
                        Some((_, v)) => v.push(r.clone()),
                        None => groups.push((k.interp, vec![r.clone()])),
                    }
                }
                groups.into_iter().map(|(interp, keys)| AnimEdit::SetInterpolation { clip_id: clip_id.clone(), keys, interp }).collect()
            }
            AnimEdit::SetClipRange { clip_id, .. } => vec![AnimEdit::SetClipRange { clip_id: clip_id.clone(), range: get(clip_id)?.range }],
            AnimEdit::SetClipPlayback { clip_id, loop_mode, extrapolation, hold_step } => {
                let c = get(clip_id)?;
                vec![AnimEdit::SetClipPlayback {
                    clip_id: clip_id.clone(),
                    loop_mode: loop_mode.map(|_| c.loop_mode),
                    extrapolation: extrapolation.map(|_| c.extrapolation),
                    hold_step: hold_step.map(|_| c.hold_step),
                }]
            }
            AnimEdit::SetFps { retime, .. } => {
                let mut v = vec![AnimEdit::SetFps { fps: anim.fps, retime: false }];
                if *retime {
                    for clip in anim.clips.values() {
                        v.push(AnimEdit::PutClip { clip: clip.clone() });
                    }
                }
                v
            }
            AnimEdit::SetMarkers { clip_id, .. } => vec![AnimEdit::SetMarkers { clip_id: clip_id.clone(), markers: get(clip_id)?.markers.clone() }],
        })
    }
}

fn channel_mut<'a>(clip: &'a mut AnimationClip, target: &ChannelTarget) -> Option<&'a mut Channel> {
    clip.channels.binary_search_by(|c| c.target.cmp(target)).ok().map(move |i| &mut clip.channels[i])
}

/// Forma canônica: canais ordenados e sem duplicata, chaves ordenadas por quadro (a última de um quadro repetido vence),
/// canais vazios removidos, marcadores ordenados.
pub fn canonicalize_clip(clip: &mut AnimationClip) {
    let mut by_target: BTreeMap<ChannelTarget, BTreeMap<i32, Key>> = BTreeMap::new();
    for ch in std::mem::take(&mut clip.channels) {
        let e = by_target.entry(ch.target).or_default();
        for k in ch.keys {
            e.insert(k.frame, k);
        }
    }
    clip.channels = by_target.into_iter().filter(|(_, m)| !m.is_empty()).map(|(target, m)| Channel { target, keys: m.into_values().collect() }).collect();
    clip.markers.sort_by(|a, b| a.frame.cmp(&b.frame).then_with(|| a.name.cmp(&b.name)));
}

fn insert_key(clip: &mut AnimationClip, target: &ChannelTarget, key: Key) {
    let idx = match clip.channels.binary_search_by(|c| c.target.cmp(target)) {
        Ok(i) => i,
        Err(i) => {
            clip.channels.insert(i, Channel { target: target.clone(), keys: Vec::new() });
            i
        }
    };
    let ch = &mut clip.channels[idx];
    match ch.keys.binary_search_by_key(&key.frame, |k| k.frame) {
        Ok(i) => ch.keys[i] = key,
        Err(i) => ch.keys.insert(i, key),
    }
}

fn remove_key(clip: &mut AnimationClip, target: &ChannelTarget, frame: i32) -> bool {
    let Ok(ci) = clip.channels.binary_search_by(|c| c.target.cmp(target)) else { return false };
    let ch = &mut clip.channels[ci];
    let Some(ki) = ch.key_index(frame) else { return false };
    ch.keys.remove(ki);
    if ch.keys.is_empty() {
        clip.channels.remove(ci);
    }
    true
}

fn apply_moves(clip: &mut AnimationClip, moves: &[KeyMove]) -> Result<(), AnimError> {
    // agrupa por canal
    let mut per: BTreeMap<ChannelTarget, Vec<&KeyMove>> = BTreeMap::new();
    for m in moves {
        per.entry(m.target.clone()).or_default().push(m);
    }
    for (target, list) in per {
        let mut dests = BTreeSet::new();
        let mut srcs = BTreeSet::new();
        for m in &list {
            if !(0..=MAX_FRAME).contains(&m.to_frame) {
                return Err(AnimError::Invalid(format!("frame {} outside 0..={MAX_FRAME}", m.to_frame)));
            }
            if !dests.insert(m.to_frame) {
                return Err(AnimError::Invalid(format!("two keys move to frame {} in {}", m.to_frame, target.key_string())));
            }
            if !srcs.insert(m.frame) {
                return Err(AnimError::Invalid(format!("key {}@{} moved twice", target.key_string(), m.frame)));
            }
        }
        let ch = channel_mut(clip, &target).ok_or_else(|| AnimError::Unknown { kind: "channel", target: target.key_string() })?;
        let mut taken = Vec::new();
        for m in &list {
            let i = ch.key_index(m.frame).ok_or_else(|| AnimError::Unknown { kind: "key", target: format!("{}@{}", target.key_string(), m.frame) })?;
            taken.push((m.to_frame, ch.keys[i]));
        }
        ch.keys.retain(|k| !srcs.contains(&k.frame));
        for (to, mut key) in taken {
            key.frame = to;
            match ch.keys.binary_search_by_key(&to, |k| k.frame) {
                Ok(i) => ch.keys[i] = key,
                Err(i) => ch.keys.insert(i, key),
            }
        }
    }
    Ok(())
}

fn retime_clip(clip: &mut AnimationClip, old_fps: f32, new_fps: f32) {
    let scale = new_fps as f64 / old_fps.max(MIN_FPS) as f64;
    let conv = |f: i32| -> i32 { ((f as f64 * scale).round() as i64).clamp(0, MAX_FRAME as i64) as i32 };
    for ch in &mut clip.channels {
        let mut out: Vec<Key> = Vec::with_capacity(ch.keys.len());
        for k in &ch.keys {
            let f = conv(k.frame);
            if out.last().is_some_and(|l| l.frame == f) {
                continue; // colisão: vale a primeira
            }
            out.push(Key { frame: f, ..*k });
        }
        ch.keys = out;
    }
    if let Some([a, b]) = clip.range {
        clip.range = Some([conv(a), conv(b).max(conv(a))]);
    }
    for m in &mut clip.markers {
        m.frame = conv(m.frame);
    }
}

#[cfg(test)]
mod tests;
