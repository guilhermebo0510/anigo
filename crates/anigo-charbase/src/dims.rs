//! Proporções do corpo derivadas de `Params` (gênero, idade, peso, músculo, altura).
//!
//! Tudo em metros, pés em y = 0, personagem olhando para +Z, "esquerda" do personagem em +X
//! (convenção VRM). Valores de referência: feminino 1,60 m (cabeça 0,235 m, ~6,8 cabeças),
//! masculino 1,72 m (cabeça 0,245 m, ~7 cabeças): proporção "anime", não antropometria.

use crate::params::{lerp, smoothstep, Params};
use glam::Vec3;

#[derive(Debug, Clone)]
pub struct Dims {
    pub p: Params,
    /// Fator feminino 0..1.
    pub g: f32,
    /// Altura total.
    pub h: f32,
    /// Escala geral relativa a 1,60 m.
    pub sc: f32,
    pub head_h: f32,
    pub y_chin: f32,
    // alturas de referência
    pub y_crotch: f32,
    pub y_hip: f32,
    pub y_waist: f32,
    pub y_bust: f32,
    pub y_armpit: f32,
    pub y_shoulder: f32,
    pub y_knee: f32,
    pub y_ankle: f32,
}

/// Nó de perfil do tronco.
#[derive(Debug, Clone, Copy)]
pub struct TrunkKnot {
    /// altura normalizada y/H
    pub u: f32,
    pub rx: f32,
    pub rz: f32,
    pub zc: f32,
    pub n: f32,
}

impl Dims {
    pub fn new(p: &Params) -> Self {
        let p = p.sanitized();
        let g = p.gender;
        let h = lerp(1.72, 1.60, g) * (1.0 + 0.07 * p.height);
        let sc = h / 1.60;
        let head_h = lerp(0.256, 0.252, g) * (1.0 + 0.12 * p.head_size) * (1.0 + 0.5 * 0.07 * p.height);
        // a idade comprime um pouco a estatura (cifose) sem mudar a cabeça
        let h = h * (1.0 - 0.025 * p.age);
        Dims {
            p,
            g,
            h,
            sc,
            head_h,
            y_chin: h - head_h,
            y_crotch: 0.470 * h,
            y_hip: 0.545 * h,
            y_waist: 0.625 * h,
            y_bust: 0.718 * h,
            y_armpit: 0.757 * h,
            y_shoulder: lerp(0.805, 0.800, g) * h,
            y_knee: 0.272 * h,
            y_ankle: 0.045 * h,
        }
    }

    /// Curvatura do tronco para a frente (cifose do envelhecimento): deslocamento em z
    /// aplicado acima da pelve, crescendo com o quadrado da altura relativa.
    pub fn stoop(&self, y: f32) -> f32 {
        let t = ((y - self.y_hip) / (self.h - self.y_hip)).clamp(0.0, 1.0);
        self.p.age * 0.060 * self.sc * t * t
    }

    /// Perfil do tronco (rings de 16 pontos) com gênero, peso, músculo e idade aplicados.
    pub fn trunk_knots(&self) -> Vec<TrunkKnot> {
        let g = self.g;
        let w = self.p.weight;
        let m = self.p.muscle;
        let a = self.p.age;
        let sc = self.sc;
        // (u, rx_m, rx_f, rz_m, rz_f, zc_m, zc_f, n)
        let raw: [(f32, f32, f32, f32, f32, f32, f32, f32); 13] = [
            (0.440, 0.140, 0.140, 0.090, 0.088, -0.004, -0.004, 2.3),
            (0.470, 0.152, 0.150, 0.096, 0.092, -0.005, -0.005, 2.4),
            (0.545, 0.150, 0.166, 0.099, 0.106, -0.006, -0.012, 2.5),
            (0.585, 0.148, 0.150, 0.094, 0.098, -0.003, -0.006, 2.3),
            (0.625, 0.136, 0.108, 0.088, 0.080, 0.000, 0.000, 2.1),
            (0.680, 0.152, 0.122, 0.098, 0.088, 0.002, 0.003, 2.2),
            (0.718, 0.166, 0.133, 0.106, 0.098, 0.004, 0.007, 2.3),
            (0.757, 0.172, 0.138, 0.100, 0.090, 0.000, 0.000, 2.3),
            (0.793, 0.208, 0.176, 0.094, 0.082, -0.004, -0.004, 2.4),
            (0.8075, 0.150, 0.122, 0.082, 0.074, -0.005, -0.005, 2.2),
            (0.821, 0.052, 0.046, 0.048, 0.044, -0.008, -0.008, 2.0),
            (0.834, 0.035, 0.031, 0.032, 0.029, -0.010, -0.010, 2.0),
            (0.848, 0.029, 0.026, 0.028, 0.025, -0.011, -0.011, 2.0),
        ];
        let mut out = Vec::with_capacity(raw.len() + 3);
        for (u, rxm, rxf, rzm, rzf, zcm, zcf, n) in raw {
            let mut rx = lerp(rxm, rxf, g) * sc;
            let mut rz = lerp(rzm, rzf, g) * sc;
            let mut zc = lerp(zcm, zcf, g) * sc;
            // peso: barriga/cintura/quadril/coxa cheios; magro: o contrário (metade do efeito)
            let wk = if w >= 0.0 { w } else { 0.45 * w };
            let belly = 1.0 - ((u - 0.60) / 0.10).powi(2); // pico na cintura-umbigo
            let belly = belly.clamp(0.0, 1.0);
            let hips = (1.0 - ((u - 0.52) / 0.09).powi(2)).clamp(0.0, 1.0);
            let chest = (1.0 - ((u - 0.70) / 0.10).powi(2)).clamp(0.0, 1.0);
            rx *= 1.0 + wk * (0.26 * belly + 0.07 * hips + 0.08 * chest);
            rz *= 1.0 + wk * (0.42 * belly + 0.11 * hips + 0.14 * chest);
            zc += wk * 0.022 * belly * sc;
            // músculo: ombros, peito e dorsal; cintura mais fina (V)
            let sh = smoothstep(0.70, 0.79, u) * (1.0 - smoothstep(0.80, 0.83, u));
            let mk = m;
            rx *= 1.0 + mk * (0.10 * sh + 0.05 * chest - 0.03 * belly);
            rz *= 1.0 + mk * (0.07 * chest + 0.03 * sh);
            // idade: ombros caem/estreitam, barriga e costas arredondam, peito desce
            rx *= 1.0 - a * (0.045 * sh) + a * 0.04 * belly;
            rz *= 1.0 + a * (0.07 * belly - 0.02 * chest);
            let y = u * self.h;
            zc += self.stoop(y) * 0.55;
            out.push(TrunkKnot { u, rx, rz, zc, n });
        }
        out
    }
}

/// Interpolação Catmull-Rom dos nós do tronco em `u` (clamp nas pontas).
pub fn trunk_at(knots: &[TrunkKnot], u: f32) -> TrunkKnot {
    let n = knots.len();
    if u <= knots[0].u {
        return knots[0];
    }
    if u >= knots[n - 1].u {
        return knots[n - 1];
    }
    let mut i = 0;
    while i + 1 < n && knots[i + 1].u < u {
        i += 1;
    }
    let (k1, k2) = (knots[i], knots[i + 1]);
    let k0 = if i == 0 { k1 } else { knots[i - 1] };
    let k3 = if i + 2 >= n { k2 } else { knots[i + 2] };
    let t = (u - k1.u) / (k2.u - k1.u);
    let cr = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        // Catmull-Rom com tangentes escaladas pelo intervalo
        let t2 = t * t;
        let t3 = t2 * t;
        0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    TrunkKnot {
        u,
        rx: cr(k0.rx, k1.rx, k2.rx, k3.rx).max(0.005),
        rz: cr(k0.rz, k1.rz, k2.rz, k3.rz).max(0.005),
        zc: cr(k0.zc, k1.zc, k2.zc, k3.zc),
        n: cr(k0.n, k1.n, k2.n, k3.n).clamp(1.8, 3.0),
    }
}

/// Ponto de uma superelipse no plano XZ: `theta` medido de +Z (frente) para +X.
pub fn superellipse(rx: f32, rz: f32, n: f32, theta: f32) -> (f32, f32) {
    let (s, c) = theta.sin_cos();
    let e = 2.0 / n;
    let x = rx * s.signum() * s.abs().powf(e);
    let z = rz * c.signum() * c.abs().powf(e);
    (x, z)
}

pub fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}
