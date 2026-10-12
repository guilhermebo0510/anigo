//! Utilitários numéricos robustos (sem pânico com NaN/zero).

use glam::{Mat3, Mat4, Quat, Vec3};

/// Gravidade padrão (m/s^2), eixo Y para cima.
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Normaliza; devolve `fallback` se o vetor é nulo/não finito.
#[inline]
pub fn safe_normalize(v: Vec3, fallback: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if l2.is_finite() && l2 > 1e-20 {
        v / l2.sqrt()
    } else {
        fallback
    }
}

/// Rotação mínima que leva o vetor unitário `from` em `to` (ambos unitários).
/// Trata o caso antiparalelo (rotação de 180 graus em torno de um eixo ortogonal).
pub fn from_to_rotation(from: Vec3, to: Vec3) -> Quat {
    let d = from.dot(to).clamp(-1.0, 1.0);
    if d > 1.0 - 1e-7 {
        return Quat::IDENTITY;
    }
    if d < -1.0 + 1e-7 {
        // Antiparalelo: qualquer eixo ortogonal serve.
        let helper = if from.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let axis = safe_normalize(from.cross(helper), Vec3::Z);
        return Quat::from_axis_angle(axis, std::f32::consts::PI);
    }
    let axis = from.cross(to);
    // q = (axis, 1 + d) normalizado: meio-ângulo sem trigonometria.
    let q = Quat::from_xyzw(axis.x, axis.y, axis.z, 1.0 + d);
    let len = q.length();
    if len.is_finite() && len > 1e-12 {
        q * (1.0 / len)
    } else {
        Quat::IDENTITY
    }
}

/// Rotação (sem escala) de uma matriz afim; tolera escala não uniforme e colunas degeneradas.
pub fn rotation_of(m: &Mat4) -> Quat {
    let x = safe_normalize(m.x_axis.truncate(), Vec3::X);
    let y = m.y_axis.truncate();
    // Gram-Schmidt: garante base ortonormal mesmo com cisalhamento leve.
    let y = safe_normalize(y - x * x.dot(y), x.any_orthonormal_vector());
    let z = x.cross(y);
    let q = Quat::from_mat3(&Mat3::from_cols(x, y, z));
    if q.is_finite() {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// Escala uniforme aproximada (maior eixo) de uma matriz afim.
#[inline]
pub fn uniform_scale_of(m: &Mat4) -> f32 {
    let s = m
        .x_axis
        .truncate()
        .length()
        .max(m.y_axis.truncate().length())
        .max(m.z_axis.truncate().length());
    if s.is_finite() && s > 1e-12 {
        s
    } else {
        1.0
    }
}

/// Ponto mais próximo de `p` no segmento `[a, b]`; devolve `(ponto, t)`.
#[inline]
pub fn closest_point_on_segment(p: Vec3, a: Vec3, b: Vec3) -> (Vec3, f32) {
    let ab = b - a;
    let l2 = ab.length_squared();
    if !(l2 > 1e-20) {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    (a + ab * t, t)
}

/// Sanitiza um `dt`: devolve `None` se for inválido (`NaN`, infinito, `<= 0`).
#[inline]
pub fn valid_dt(dt: f32) -> Option<f32> {
    if dt.is_finite() && dt > 0.0 {
        Some(dt)
    } else {
        None
    }
}

/// Hash FNV-1a 64 bits dos bits de uma lista de vetores: assinatura de estado para testes de determinismo.
pub fn state_hash(points: &[Vec3]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in points {
        for c in [p.x, p.y, p.z] {
            for b in c.to_bits().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    h
}

/// Serde para limites que podem ser "sem limite" (`f32::INFINITY`): grava `null` e lê `null`/ausente
/// como infinito (JSON não tem infinito).
pub mod serde_limit {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_some(v)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
        Ok(Option::<f32>::deserialize(d)?.unwrap_or(f32::INFINITY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_to_cobre_casos_limite() {
        let cases = [
            (Vec3::X, Vec3::Y),
            (Vec3::X, Vec3::X),
            (Vec3::X, -Vec3::X),
            (Vec3::Y, -Vec3::Y),
            (Vec3::new(1.0, 2.0, 3.0).normalize(), Vec3::new(-3.0, 0.5, 1.0).normalize()),
        ];
        for (a, b) in cases {
            let q = from_to_rotation(a, b);
            assert!(q.is_normalized());
            assert!((q * a - b).length() < 1e-5, "{a:?} -> {b:?}: {:?}", q * a);
        }
    }

    #[test]
    fn rotation_of_ignora_escala() {
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.7, 1.1);
        let m = Mat4::from_scale_rotation_translation(Vec3::new(2.0, 2.0, 2.0), q, Vec3::new(1.0, 2.0, 3.0));
        let r = rotation_of(&m);
        assert!(r.angle_between(q) < 1e-5);
        assert!(rotation_of(&Mat4::ZERO).is_finite());
    }

    #[test]
    fn normalizacao_segura() {
        assert_eq!(safe_normalize(Vec3::ZERO, Vec3::Y), Vec3::Y);
        assert_eq!(safe_normalize(Vec3::splat(f32::NAN), Vec3::Y), Vec3::Y);
        assert!((safe_normalize(Vec3::new(0.0, 3.0, 4.0), Vec3::X).length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn dt_invalido_e_rejeitado() {
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(valid_dt(dt).is_none());
        }
        assert_eq!(valid_dt(0.016), Some(0.016));
    }
}
