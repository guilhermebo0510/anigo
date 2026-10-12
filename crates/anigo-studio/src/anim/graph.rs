//! Matemática do Graph Editor (curvas): componentes exibíveis de um canal, amostragem com desembrulho de Euler,
//! posição das alças Bézier no espaço (quadro, valor) e a conversão inversa ao arrastá-las.
//!
//! Rotações são quaternions no núcleo; aqui são MOSTRADAS como Euler XYZ em graus (somente leitura), com o ângulo
//! desembrulhado para a curva ser contínua. As alças de um segmento Bézier ficam sobre a componente de maior variação do
//! segmento (`dominant_component`): `y` normalizado é o PROGRESSO do segmento (`slerp` para rotações), então a posição
//! mostrada é `v0 + y·(v1 − v0)` naquela componente (exata para escalares/vetores, indicativa para rotações).

use anigo_core::animation::{Channel, Extrapolation, Interp, Key, KeyValue, ValueKind};
use anigo_core::pose::quat_to_euler_deg;

/// Nomes das componentes por tipo de valor.
pub fn component_names(kind: ValueKind) -> &'static [&'static str] {
    match kind {
        ValueKind::Scalar => &["v"],
        ValueKind::Vec3 | ValueKind::Quat => &["X", "Y", "Z"],
        ValueKind::Curl => &["T", "I", "M", "A", "P"],
    }
}

/// Valores das componentes exibidas de um valor de chave (Euler em graus para quaternions).
pub fn components(v: &KeyValue) -> Vec<f32> {
    match v {
        KeyValue::Scalar(x) => vec![*x],
        KeyValue::Vec3(p) => p.to_vec(),
        KeyValue::Quat(q) => quat_to_euler_deg(glam::Quat::from_xyzw(q[0], q[1], q[2], q[3])).to_vec(),
        KeyValue::Curl(c) => c.to_vec(),
    }
}

/// Desembrulha um ângulo (graus) para ficar a menos de 180° do anterior.
pub fn unwrap_deg(prev: f32, mut a: f32) -> f32 {
    while a - prev > 180.0 {
        a -= 360.0;
    }
    while a - prev < -180.0 {
        a += 360.0;
    }
    a
}

/// Euler XYZ (graus) de `q` escolhendo, entre as representações equivalentes, a mais próxima da anterior
/// (`(x+180, 180−y, z+180)` descreve a mesma rotação): a curva não pula quando |y| passa de 90°.
pub fn euler_continuous(prev: Option<[f32; 3]>, q: glam::Quat) -> [f32; 3] {
    let e1 = quat_to_euler_deg(q);
    let Some(p) = prev else { return e1 };
    let e2 = [e1[0] + 180.0, 180.0 - e1[1], e1[2] + 180.0];
    let fit = |e: [f32; 3]| -> ([f32; 3], f32) {
        let u = [unwrap_deg(p[0], e[0]), unwrap_deg(p[1], e[1]), unwrap_deg(p[2], e[2])];
        (u, (u[0] - p[0]).abs() + (u[1] - p[1]).abs() + (u[2] - p[2]).abs())
    };
    let (a, da) = fit(e1);
    let (b, db) = fit(e2);
    if da <= db {
        a
    } else {
        b
    }
}

/// Amostras `(quadro, valor)` por componente entre `f0` e `f1` com `per_frame` amostras por quadro. Quaternions viram
/// Euler contínuos (desembrulhados).
pub fn curves(ch: &Channel, ext: Extrapolation, f0: f32, f1: f32, per_frame: f32) -> Vec<Vec<[f32; 2]>> {
    let n = component_names(ch.target.value_kind()).len();
    let mut out: Vec<Vec<[f32; 2]>> = vec![Vec::new(); n];
    if ch.keys.is_empty() || f1 <= f0 {
        return out;
    }
    let steps = (((f1 - f0) * per_frame.max(0.25)).ceil() as usize).clamp(2, 4000);
    let is_quat = ch.target.value_kind() == ValueKind::Quat;
    // para o desembrulho começar contínuo, parte do começo da primeira chave quando possível
    let mut prev: Vec<f32> = Vec::new();
    for i in 0..=steps {
        let f = f0 + (f1 - f0) * i as f32 / steps as f32;
        let Some(v) = ch.evaluate(f, ext) else { continue };
        let mut vals = components(&v);
        if let (true, KeyValue::Quat(q)) = (is_quat, &v) {
            let p = if prev.len() == 3 { Some([prev[0], prev[1], prev[2]]) } else { None };
            vals = euler_continuous(p, glam::Quat::from_xyzw(q[0], q[1], q[2], q[3])).to_vec();
            prev = vals.clone();
        }
        for (c, x) in vals.into_iter().enumerate() {
            out[c].push([f, x]);
        }
    }
    out
}

/// Valores das componentes das chaves, com Euler desembrulhado ao longo das chaves do canal.
pub fn key_components(ch: &Channel) -> Vec<Vec<f32>> {
    let is_quat = ch.target.value_kind() == ValueKind::Quat;
    let mut prev: Option<[f32; 3]> = None;
    ch.keys
        .iter()
        .map(|k| {
            let mut v = components(&k.value);
            if let (true, KeyValue::Quat(q)) = (is_quat, &k.value) {
                let e = euler_continuous(prev, glam::Quat::from_xyzw(q[0], q[1], q[2], q[3]));
                prev = Some(e);
                v = e.to_vec();
            }
            v
        })
        .collect()
}

/// Componente em que as alças do segmento `a → b` são desenhadas: a de maior variação.
pub fn dominant_component(a: &KeyValue, b: &KeyValue) -> usize {
    let (ca, cb) = (components(a), components(b));
    let mut best = (0usize, -1.0f32);
    for i in 0..ca.len().min(cb.len()) {
        let mut d = (cb[i] - ca[i]).abs();
        if matches!(a, KeyValue::Quat(_)) {
            d = d.min(360.0 - d.min(360.0)).max(d.min(180.0));
        }
        if d > best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// Pontos de controle do segmento `a → b` em (quadro, valor) da componente `comp`: `(P1, P2)`.
/// `None` se a interpolação de `a` não for uma curva (Step/Linear).
pub fn handle_points(a: &Key, b: &Key, comp: usize, a_val: f32, b_val: f32) -> Option<([f32; 2], [f32; 2])> {
    let c = a.interp.control_points()?;
    let span = (b.frame - a.frame) as f32;
    let _ = comp;
    Some(([a.frame as f32 + c[0] * span, a_val + c[1] * (b_val - a_val)], [a.frame as f32 + c[2] * span, a_val + c[3] * (b_val - a_val)]))
}

/// Qual alça se arrasta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Alça de saída (anexada à chave `a`).
    Out,
    /// Alça de entrada (anexada à chave `b`).
    In,
}

/// Novo `[x1, y1, x2, y2]` ao arrastar uma alça para `(frame, value)` (espaço do gráfico). `x` é limitado a `[0, 1]`
/// (monotonia no tempo); `y` livre em `[−4, 5]` (antecipação/overshoot). Se `a_val == b_val` o `y` fica como está.
pub fn drag_handle(c: [f32; 4], which: Handle, a: &Key, b: &Key, a_val: f32, b_val: f32, frame: f32, value: f32) -> [f32; 4] {
    let span = (b.frame - a.frame) as f32;
    let x = if span.abs() > 1e-6 { ((frame - a.frame as f32) / span).clamp(0.0, 1.0) } else { 0.0 };
    let dv = b_val - a_val;
    let y = if dv.abs() > 1e-6 { ((value - a_val) / dv).clamp(-4.0, 5.0) } else { 0.0 };
    let mut out = c;
    match which {
        Handle::Out => {
            out[0] = x;
            if dv.abs() > 1e-6 {
                out[1] = y;
            }
        }
        Handle::In => {
            out[2] = x;
            if dv.abs() > 1e-6 {
                out[3] = y;
            }
        }
    }
    out
}

/// Bézier equivalente à interpolação atual (para "converter em Bézier" e arrastar a partir dela).
pub fn as_bezier(i: Interp) -> Interp {
    match i {
        Interp::Bezier { .. } => i,
        Interp::Step | Interp::Linear => Interp::bezier_linear(),
        other => Interp::Bezier { c: other.control_points().unwrap_or([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]) },
    }
}

/// Faixa vertical `(min, max)` que cobre as curvas dadas (com folga), nunca degenerada.
pub fn value_range(curves: &[Vec<[f32; 2]>]) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for c in curves {
        for p in c {
            if p[1].is_finite() {
                lo = lo.min(p[1]);
                hi = hi.max(p[1]);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (-1.0, 1.0);
    }
    let pad = ((hi - lo) * 0.12).max(0.05);
    (lo - pad, hi + pad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{ChannelTarget, KeyValue};
    use glam::Quat;

    fn quat_channel(angles_deg: &[(i32, f32)]) -> Channel {
        Channel {
            target: ChannelTarget::bone("Head"),
            keys: angles_deg.iter().map(|(f, a)| Key::new(*f, KeyValue::Quat(Quat::from_rotation_y(a.to_radians()).to_array()))).collect(),
        }
    }

    #[test]
    fn euler_curves_are_continuous_across_the_plus_minus_180_wrap() {
        // 0 -> 350 graus pelo caminho curto a cada chave: o Euler cru saltaria de +170 para -170
        let ch = quat_channel(&[(0, 0.0), (10, 120.0), (20, 240.0), (30, 350.0)]);
        let curves = curves(&ch, Extrapolation::Constant, 0.0, 30.0, 2.0);
        assert_eq!(curves.len(), 3);
        // a curva Y é a mais ativa e não tem saltos de ~360
        let mut prev = curves[1][0][1];
        for p in &curves[1] {
            assert!((p[1] - prev).abs() < 30.0, "salto de {} para {}", prev, p[1]);
            prev = p[1];
        }
    }

    #[test]
    fn key_components_are_unwrapped_along_the_channel() {
        let ch = quat_channel(&[(0, 0.0), (10, 100.0), (20, 200.0)]);
        let comps = key_components(&ch);
        // 100 e 200 graus em Y passam do limite de ±90° do Euler: a representação equivalente mantém a curva contínua
        assert!((comps[1][1] - 100.0).abs() < 0.5 && (comps[2][1] - 200.0).abs() < 0.5, "{comps:?}");
        assert!(comps[2][0].abs() < 0.5 && comps[2][2].abs() < 0.5);
        for w in comps.windows(2) {
            assert!((w[1][1] - w[0][1]).abs() < 180.0);
        }
        assert_eq!(unwrap_deg(170.0, -170.0), 190.0);
        assert_eq!(unwrap_deg(-170.0, 170.0), -190.0);
        assert_eq!(unwrap_deg(0.0, 90.0), 90.0);
    }

    #[test]
    fn scalar_and_vector_channels_report_their_components() {
        assert_eq!(component_names(ValueKind::Scalar), &["v"]);
        assert_eq!(component_names(ValueKind::Curl).len(), 5);
        assert_eq!(components(&KeyValue::Scalar(2.0)), vec![2.0]);
        assert_eq!(components(&KeyValue::Vec3([1.0, 2.0, 3.0])), vec![1.0, 2.0, 3.0]);
        assert_eq!(components(&KeyValue::Curl([0.1; 5])).len(), 5);
        let ch = Channel { target: ChannelTarget::Hips, keys: vec![Key::new(0, KeyValue::Vec3([0.0; 3])), Key::new(10, KeyValue::Vec3([1.0, 5.0, 2.0]))] };
        let c = curves(&ch, Extrapolation::Constant, 0.0, 10.0, 1.0);
        assert_eq!(c.len(), 3);
        assert!((c[1].last().unwrap()[1] - 5.0).abs() < 1e-4);
        assert_eq!(dominant_component(&ch.keys[0].value, &ch.keys[1].value), 1);
        assert!(curves(&Channel { target: ChannelTarget::Hips, keys: vec![] }, Extrapolation::Constant, 0.0, 1.0, 1.0)[0].is_empty());
    }

    #[test]
    fn handles_sit_on_the_curve_and_dragging_them_updates_the_bezier() {
        let a = Key::new(0, KeyValue::Scalar(0.0)).with_interp(Interp::Bezier { c: [0.25, 0.1, 0.75, 0.9] });
        let b = Key::new(20, KeyValue::Scalar(10.0));
        let (p1, p2) = handle_points(&a, &b, 0, 0.0, 10.0).unwrap();
        assert_eq!(p1, [5.0, 1.0]);
        assert_eq!(p2, [15.0, 9.0]);
        // arrastar a alça de saída para (frame 10, valor 8): x = 0.5, y = 0.8
        let c = drag_handle([0.25, 0.1, 0.75, 0.9], Handle::Out, &a, &b, 0.0, 10.0, 10.0, 8.0);
        assert_eq!(c, [0.5, 0.8, 0.75, 0.9]);
        // x é limitado a [0,1]; y pode passar de 1 (overshoot) mas é limitado a [-4,5]
        let c = drag_handle(c, Handle::In, &a, &b, 0.0, 10.0, 99.0, 1000.0);
        assert_eq!((c[2], c[3]), (1.0, 5.0));
        let c = drag_handle(c, Handle::In, &a, &b, 0.0, 10.0, -5.0, -1000.0);
        assert_eq!((c[2], c[3]), (0.0, -4.0));
        // segmento plano (a_val == b_val): o y não muda
        let c = drag_handle([0.3, 0.4, 0.6, 0.7], Handle::Out, &a, &b, 5.0, 5.0, 10.0, 99.0);
        assert_eq!((c[0], c[1]), (0.5, 0.4));
        // Step/Linear não têm alças
        assert!(handle_points(&Key::new(0, KeyValue::Scalar(0.0)), &b, 0, 0.0, 1.0).is_none());
        // o ponto arrastado reproduz a curva do núcleo no meio do segmento
        let ch = Channel { target: ChannelTarget::Camera { prop: anigo_core::animation::CameraProp::Fov }, keys: vec![a, b] };
        let mid = ch.evaluate(10.0, Extrapolation::Constant).unwrap().as_scalar().unwrap();
        assert!((mid - 10.0 * anigo_core::animation::bezier_ease([0.25, 0.1, 0.75, 0.9], 0.5)).abs() < 1e-4);
    }

    #[test]
    fn converting_to_bezier_preserves_the_look_and_ranges_are_never_degenerate() {
        assert_eq!(as_bezier(Interp::Linear), Interp::bezier_linear());
        assert_eq!(as_bezier(Interp::Step), Interp::bezier_linear());
        assert_eq!(as_bezier(Interp::Ease), Interp::Bezier { c: [0.42, 0.0, 0.58, 1.0] });
        let keep = Interp::Bezier { c: [0.1, 0.2, 0.3, 0.4] };
        assert_eq!(as_bezier(keep), keep);
        let (lo, hi) = value_range(&[vec![[0.0, 3.0], [1.0, 3.0]]]);
        assert!(hi > lo);
        assert_eq!(value_range(&[]), (-1.0, 1.0));
        let (lo, hi) = value_range(&[vec![[0.0, -2.0], [1.0, 6.0]]]);
        assert!(lo < -2.0 && hi > 6.0);
    }
}
