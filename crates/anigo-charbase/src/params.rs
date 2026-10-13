//! Parâmetros do gerador. Tudo que muda a *forma* (e nunca a topologia) passa por aqui:
//! gênero, idade, peso, músculo, altura e o conjunto mínimo de rosto. Cada morph de macro
//! é a diferença entre duas gerações com um parâmetro movido (ver `morphs`).

use serde::{Deserialize, Serialize};

/// Gênero do contrato: `Blend` interpola continuamente (0 = masculino, 1 = feminino).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Gender {
    Male,
    Female,
    Blend(f32),
}

impl Gender {
    /// Fator feminino em `[0, 1]`.
    pub fn female_factor(self) -> f32 {
        match self {
            Gender::Male => 0.0,
            Gender::Female => 1.0,
            Gender::Blend(t) => {
                if t.is_finite() {
                    t.clamp(0.0, 1.0)
                } else {
                    0.5
                }
            }
        }
    }
}

/// Parâmetros de rosto (todos em `[-1, 1]`, 0 = base). Cada um tem um morph `face_<nome>_pos/neg`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceParams {
    pub face_width: f32,
    pub face_length: f32,
    pub eye_size: f32,
    pub eye_width: f32,
    pub eye_height: f32,
    /// Inclinação do canto externo: positivo = tsurime (sobe), negativo = tareme (cai).
    pub eye_tilt: f32,
    pub eye_distance: f32,
    pub nose_size: f32,
    pub nose_height: f32,
    pub mouth_width: f32,
    pub mouth_height: f32,
    pub lip_fullness: f32,
    pub jaw_width: f32,
    pub chin_length: f32,
    pub chin_point: f32,
    pub ear_size: f32,
    /// Orelha pontuda/élfica.
    pub ear_point: f32,
    pub cheek_fullness: f32,
}

impl Default for FaceParams {
    fn default() -> Self {
        Self {
            face_width: 0.0,
            face_length: 0.0,
            eye_size: 0.0,
            eye_width: 0.0,
            eye_height: 0.0,
            eye_tilt: 0.0,
            eye_distance: 0.0,
            nose_size: 0.0,
            nose_height: 0.0,
            mouth_width: 0.0,
            mouth_height: 0.0,
            lip_fullness: 0.0,
            jaw_width: 0.0,
            chin_length: 0.0,
            chin_point: 0.0,
            ear_size: 0.0,
            ear_point: 0.0,
            cheek_fullness: 0.0,
        }
    }
}

/// Nomes estáveis dos parâmetros de rosto, na ordem dos campos.
pub const FACE_PARAM_NAMES: [&str; 18] = [
    "face_width",
    "face_length",
    "eye_size",
    "eye_width",
    "eye_height",
    "eye_tilt",
    "eye_distance",
    "nose_size",
    "nose_height",
    "mouth_width",
    "mouth_height",
    "lip_fullness",
    "jaw_width",
    "chin_length",
    "chin_point",
    "ear_size",
    "ear_point",
    "cheek_fullness",
];

impl FaceParams {
    pub fn get(&self, i: usize) -> f32 {
        match i {
            0 => self.face_width,
            1 => self.face_length,
            2 => self.eye_size,
            3 => self.eye_width,
            4 => self.eye_height,
            5 => self.eye_tilt,
            6 => self.eye_distance,
            7 => self.nose_size,
            8 => self.nose_height,
            9 => self.mouth_width,
            10 => self.mouth_height,
            11 => self.lip_fullness,
            12 => self.jaw_width,
            13 => self.chin_length,
            14 => self.chin_point,
            15 => self.ear_size,
            16 => self.ear_point,
            _ => self.cheek_fullness,
        }
    }

    pub fn set(&mut self, i: usize, v: f32) {
        match i {
            0 => self.face_width = v,
            1 => self.face_length = v,
            2 => self.eye_size = v,
            3 => self.eye_width = v,
            4 => self.eye_height = v,
            5 => self.eye_tilt = v,
            6 => self.eye_distance = v,
            7 => self.nose_size = v,
            8 => self.nose_height = v,
            9 => self.mouth_width = v,
            10 => self.mouth_height = v,
            11 => self.lip_fullness = v,
            12 => self.jaw_width = v,
            13 => self.chin_length = v,
            14 => self.chin_point = v,
            15 => self.ear_size = v,
            16 => self.ear_point = v,
            _ => self.cheek_fullness = v,
        }
    }
}

/// Parâmetros completos da geração. `Params::default()` é o personagem neutro (jovem adulto).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// 0 = masculino, 1 = feminino.
    pub gender: f32,
    /// 0 = jovem adulto, 0.5 = adulto, 1 = idoso. O limite inferior é ADULTO (nunca criança/adolescente).
    pub age: f32,
    /// -1 magro, 0 base, +1 gordo.
    pub weight: f32,
    /// -1 flácido/delgado, 0 base, +1 musculoso.
    pub muscle: f32,
    /// -1 baixo, 0 base, +1 alto (±8 % da altura).
    pub height: f32,
    /// -1 cabeça menor, +1 maior (±12 % da cabeça, no estilo anime).
    pub head_size: f32,
    pub face: FaceParams,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            gender: 1.0,
            age: 0.0,
            weight: 0.0,
            muscle: 0.0,
            height: 0.0,
            head_size: 0.0,
            face: FaceParams::default(),
        }
    }
}

impl Params {
    pub fn for_gender(g: Gender) -> Self {
        Self {
            gender: g.female_factor(),
            ..Self::default()
        }
    }

    /// Valores finitos e dentro das faixas (qualquer entrada malformada é saneada, nunca entra em pânico).
    pub fn sanitized(&self) -> Self {
        fn c(v: f32, lo: f32, hi: f32) -> f32 {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                0.0
            }
        }
        let mut p = *self;
        p.gender = c(self.gender, 0.0, 1.0);
        p.age = c(self.age, 0.0, 1.0);
        p.weight = c(self.weight, -1.0, 1.0);
        p.muscle = c(self.muscle, -1.0, 1.0);
        p.height = c(self.height, -1.0, 1.0);
        p.head_size = c(self.head_size, -1.0, 1.0);
        for i in 0..FACE_PARAM_NAMES.len() {
            let v = c(self.face.get(i), -1.0, 1.0);
            p.face.set(i, v);
        }
        p
    }
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
