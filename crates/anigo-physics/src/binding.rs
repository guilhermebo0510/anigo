//! Vínculos "malha de render <-> simulação":
//!
//! * [`ChainFrames`]/[`ChainBinding`]: vértices de um card de cabelo presos a uma cadeia de
//!   partículas (posição ao longo da cadeia + offset em um frame de transporte paralelo).
//! * [`ProxyBinding`]: vértices da roupa presos a 3 vértices da malha-proxy por coordenadas
//!   baricêntricas + offset no frame do triângulo (modelo `.mhclo` do MakeHuman).
//!
//! Os dois são determinísticos, serializáveis e exatos no repouso (reproduzem a malha original).

use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{PhysicsError, PhysicsResult};
use crate::math::{closest_point_on_segment, from_to_rotation, safe_normalize};

// ---------------------------------------------------------------------------
// Cadeias
// ---------------------------------------------------------------------------

/// Frames (um por segmento) por transporte paralelo, ancorados em um eixo de referência.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainFrames {
    /// Rotação do frame de cada segmento: coluna X = tangente, Y = normal, Z = binormal.
    pub frames: Vec<Quat>,
}

impl ChainFrames {
    /// `points`: partículas (>= 2). `reference`: eixo (no mesmo espaço) que define a normal inicial
    /// (normalmente um eixo do osso-raiz já girado para o mundo). Devolve `None` se a cadeia é
    /// degenerada (menos de 2 pontos ou todos coincidentes).
    pub fn compute(points: &[Vec3], reference: Vec3) -> Option<Self> {
        if points.len() < 2 || points.iter().any(|p| !p.is_finite()) {
            return None;
        }
        let mut frames = Vec::with_capacity(points.len() - 1);
        let mut prev_t = Vec3::ZERO;
        let mut prev_n = Vec3::ZERO;
        let mut last_t = None;
        for w in points.windows(2) {
            let d = w[1] - w[0];
            let t = if d.length_squared() > 1e-16 { d.normalize() } else { last_t.unwrap_or(Vec3::Y) };
            let n = match last_t {
                None => {
                    let r = if reference.is_finite() && reference.length_squared() > 1e-12 { reference } else { Vec3::Z };
                    let mut n = r - t * r.dot(t);
                    if n.length_squared() < 1e-10 {
                        n = t.any_orthonormal_vector();
                    }
                    n.normalize()
                }
                Some(_) => {
                    let q = from_to_rotation(prev_t, t);
                    (q * prev_n).normalize()
                }
            };
            let b = t.cross(n);
            let q = Quat::from_mat3(&Mat3::from_cols(t, n, b));
            frames.push(q.normalize());
            prev_t = t;
            prev_n = n;
            last_t = Some(t);
        }
        Some(Self { frames })
    }

    /// Frame interpolado no segmento `seg` em `t` (nlerp com o frame do segmento seguinte).
    pub fn frame_at(&self, seg: usize, t: f32) -> Quat {
        let a = self.frames[seg.min(self.frames.len() - 1)];
        let b = self.frames[(seg + 1).min(self.frames.len() - 1)];
        let b = if a.dot(b) < 0.0 { -b } else { b };
        let q = a * (1.0 - t) + b * t;
        if q.length_squared() > 1e-12 {
            q.normalize()
        } else {
            a
        }
    }
}

/// Vértice preso à cadeia `chain` no segmento `segment`, parâmetro `t` (0..1) e offset local.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChainBinding {
    pub chain: u32,
    pub segment: u32,
    pub t: f32,
    /// Offset no frame (tangente, normal, binormal) interpolado.
    pub offset: Vec3,
}

/// Prende `vertices` (repouso) ao ponto mais próximo das cadeias de repouso. `rest_chains[i]`
/// são as posições das partículas da cadeia `i` no mesmo espaço dos vértices; `reference[i]` é o
/// eixo de referência de cada cadeia.
pub fn bind_to_chains(vertices: &[Vec3], rest_chains: &[&[Vec3]], reference: &[Vec3]) -> PhysicsResult<Vec<ChainBinding>> {
    if rest_chains.len() != reference.len() {
        return Err(PhysicsError::BadConfig("rest_chains e reference com tamanhos diferentes".into()));
    }
    let frames: Vec<ChainFrames> = rest_chains
        .iter()
        .zip(reference)
        .map(|(c, r)| ChainFrames::compute(c, *r).ok_or_else(|| PhysicsError::BadConfig("cadeia degenerada".into())))
        .collect::<PhysicsResult<_>>()?;
    let mut out = Vec::with_capacity(vertices.len());
    for v in vertices {
        if !v.is_finite() {
            return Err(PhysicsError::BadValue { what: "vértice do card".into() });
        }
        let mut best = (f32::INFINITY, 0usize, 0usize, 0.0f32, Vec3::ZERO);
        for (ci, chain) in rest_chains.iter().enumerate() {
            for (si, w) in chain.windows(2).enumerate() {
                let (c, t) = closest_point_on_segment(*v, w[0], w[1]);
                let d2 = (*v - c).length_squared();
                if d2 < best.0 {
                    best = (d2, ci, si, t, c);
                }
            }
        }
        let (_, ci, si, t, c) = best;
        let q = frames[ci].frame_at(si, t);
        out.push(ChainBinding { chain: ci as u32, segment: si as u32, t, offset: q.inverse() * (*v - c) });
    }
    Ok(out)
}

/// Posições atuais dos vértices dado o estado atual das cadeias. Vínculo inválido -> `None` nessa posição.
pub fn apply_chain_bindings(bindings: &[ChainBinding], chains: &[&[Vec3]], reference: &[Vec3]) -> Vec<Option<Vec3>> {
    let frames: Vec<Option<ChainFrames>> = chains.iter().zip(reference).map(|(c, r)| ChainFrames::compute(c, *r)).collect();
    bindings
        .iter()
        .map(|b| {
            let ci = b.chain as usize;
            let si = b.segment as usize;
            let chain = chains.get(ci)?;
            let fr = frames.get(ci)?.as_ref()?;
            if si + 1 >= chain.len() || !b.t.is_finite() {
                return None;
            }
            let c = chain[si].lerp(chain[si + 1], b.t.clamp(0.0, 1.0));
            Some(c + fr.frame_at(si, b.t.clamp(0.0, 1.0)) * b.offset)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Proxy de roupa
// ---------------------------------------------------------------------------

/// Vértice da roupa preso a um triângulo da proxy: `p = u*P0 + v*P1 + w*P2 + F*offset`, onde
/// `F = (t, b, n)` é o frame do triângulo (t = aresta P0->P1 normalizada, n = normal, b = n x t).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProxyBinding {
    pub tri: [u32; 3],
    pub bary: [f32; 3],
    pub offset: Vec3,
}

fn tri_frame(p0: Vec3, p1: Vec3, p2: Vec3) -> Option<Mat3> {
    let e0 = p1 - p0;
    let n = e0.cross(p2 - p0);
    if !(n.length_squared() > 1e-18) || !e0.is_finite() {
        return None;
    }
    let n = n.normalize();
    let t = safe_normalize(e0, Vec3::X);
    let b = n.cross(t);
    Some(Mat3::from_cols(t, b, n))
}

/// Ponto mais próximo no triângulo; devolve `(ponto, baricêntricas)`.
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    // Ericson, Real-Time Collision Detection.
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

/// Cria os vínculos: cada vértice da roupa em repouso se prende ao triângulo mais próximo da
/// proxy (em repouso). `O(V*T)`; a proxy tem centenas de triângulos.
pub fn bind_to_proxy(vertices: &[Vec3], proxy_positions: &[Vec3], triangles: &[[u32; 3]]) -> PhysicsResult<Vec<ProxyBinding>> {
    for t in triangles {
        for &i in t {
            if i as usize >= proxy_positions.len() {
                return Err(PhysicsError::BadIndex { what: "proxy.triangle", index: i as usize, len: proxy_positions.len() });
            }
        }
    }
    if proxy_positions.iter().any(|p| !p.is_finite()) {
        return Err(PhysicsError::BadValue { what: "proxy.positions".into() });
    }
    let valid: Vec<usize> =
        (0..triangles.len()).filter(|&k| tri_frame(proxy_positions[triangles[k][0] as usize], proxy_positions[triangles[k][1] as usize], proxy_positions[triangles[k][2] as usize]).is_some()).collect();
    if valid.is_empty() {
        return Err(PhysicsError::BadMesh("proxy sem triângulos não degenerados".into()));
    }
    let mut out = Vec::with_capacity(vertices.len());
    for v in vertices {
        if !v.is_finite() {
            return Err(PhysicsError::BadValue { what: "vértice da roupa".into() });
        }
        let mut best = (f32::INFINITY, 0usize, Vec3::ZERO, [0.0f32; 3]);
        for &k in &valid {
            let t = triangles[k];
            let (c, bary) = closest_on_triangle(*v, proxy_positions[t[0] as usize], proxy_positions[t[1] as usize], proxy_positions[t[2] as usize]);
            let d2 = (*v - c).length_squared();
            if d2 < best.0 {
                best = (d2, k, c, bary);
            }
        }
        let (_, k, c, bary) = best;
        let t = triangles[k];
        let frame = tri_frame(proxy_positions[t[0] as usize], proxy_positions[t[1] as usize], proxy_positions[t[2] as usize])
            .unwrap_or(Mat3::IDENTITY);
        out.push(ProxyBinding { tri: t, bary, offset: frame.transpose() * (*v - c) });
    }
    Ok(out)
}

/// Posição atual de cada vértice da roupa a partir das posições atuais da proxy. Vínculo com
/// índice inválido ou triângulo degenerado agora devolve `None` para aquele vértice.
pub fn apply_proxy_bindings(bindings: &[ProxyBinding], proxy_positions: &[Vec3]) -> Vec<Option<Vec3>> {
    bindings.iter().map(|b| apply_proxy_binding(b, proxy_positions)).collect()
}

pub fn apply_proxy_binding(b: &ProxyBinding, proxy_positions: &[Vec3]) -> Option<Vec3> {
    let p0 = *proxy_positions.get(b.tri[0] as usize)?;
    let p1 = *proxy_positions.get(b.tri[1] as usize)?;
    let p2 = *proxy_positions.get(b.tri[2] as usize)?;
    let base = p0 * b.bary[0] + p1 * b.bary[1] + p2 * b.bary[2];
    let frame = tri_frame(p0, p1, p2)?;
    let p = base + frame * b.offset;
    p.is_finite().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_proxy() -> (Vec<Vec3>, Vec<[u32; 3]>) {
        // quadrado 3x3 de vertices no plano XY (z=0), 2 triangulos por celula
        let mut pos = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                pos.push(Vec3::new(x as f32 * 0.1, y as f32 * 0.1, 0.0));
            }
        }
        let mut tris = Vec::new();
        for y in 0..2u32 {
            for x in 0..2u32 {
                let i = y * 3 + x;
                tris.push([i, i + 1, i + 3]);
                tris.push([i + 1, i + 4, i + 3]);
            }
        }
        (pos, tris)
    }

    #[test]
    fn proxy_binding_reproduz_o_repouso_exatamente() {
        let (pos, tris) = grid_proxy();
        // vertices da roupa: acima, abaixo, fora da proxy e exatamente sobre vertices
        let verts = vec![
            Vec3::new(0.05, 0.05, 0.02),
            Vec3::new(0.12, 0.03, -0.01),
            Vec3::new(0.2, 0.2, 0.05),
            Vec3::new(-0.05, 0.1, 0.03),
            Vec3::new(0.1, 0.1, 0.0),
        ];
        let binds = bind_to_proxy(&verts, &pos, &tris).unwrap();
        let out = apply_proxy_bindings(&binds, &pos);
        for (v, o) in verts.iter().zip(out) {
            assert!((o.unwrap() - *v).length() < 1e-5, "{v:?} -> {o:?}");
        }
        for b in &binds {
            assert!((b.bary.iter().sum::<f32>() - 1.0).abs() < 1e-5);
            assert!(b.bary.iter().all(|w| *w >= -1e-6));
        }
    }

    #[test]
    fn proxy_binding_acompanha_movimento_rigido_e_escala() {
        let (pos, tris) = grid_proxy();
        let verts = vec![Vec3::new(0.05, 0.05, 0.02), Vec3::new(0.15, 0.12, -0.03)];
        let binds = bind_to_proxy(&verts, &pos, &tris).unwrap();
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.8, 1.2);
        let tr = Vec3::new(3.0, -1.0, 2.0);
        let moved: Vec<Vec3> = pos.iter().map(|p| q * *p + tr).collect();
        let out = apply_proxy_bindings(&binds, &moved);
        for (v, o) in verts.iter().zip(out) {
            assert!((o.unwrap() - (q * *v + tr)).length() < 1e-5);
        }
    }

    #[test]
    fn proxy_binding_entradas_invalidas() {
        let (pos, tris) = grid_proxy();
        assert!(bind_to_proxy(&[Vec3::ZERO], &pos, &[[0, 1, 99]]).is_err());
        assert!(bind_to_proxy(&[Vec3::splat(f32::NAN)], &pos, &tris).is_err());
        assert!(bind_to_proxy(&[Vec3::ZERO], &pos, &[[0, 0, 0]]).is_err(), "so triangulos degenerados");
        let mut bad = pos.clone();
        bad[2] = Vec3::splat(f32::INFINITY);
        assert!(bind_to_proxy(&[Vec3::ZERO], &bad, &tris).is_err());
        let binds = bind_to_proxy(&[Vec3::new(0.05, 0.05, 0.0)], &pos, &tris).unwrap();
        // proxy menor que o vinculo -> None, nao panic
        assert_eq!(apply_proxy_bindings(&binds, &pos[..2]), vec![None]);
        // triangulo colapsado -> None
        let collapsed = vec![Vec3::ZERO; 9];
        assert_eq!(apply_proxy_bindings(&binds, &collapsed), vec![None]);
    }

    fn arc(n: usize, radius: f32, sweep: f32) -> Vec<Vec3> {
        (0..n)
            .map(|k| {
                let a = sweep * k as f32 / (n - 1) as f32;
                Vec3::new(radius * a.sin(), -radius * a.cos() + radius, 0.0)
            })
            .collect()
    }

    #[test]
    fn card_preso_a_cadeia_reproduz_o_repouso_e_segue_movimento_rigido() {
        let chain = arc(8, 0.5, 1.2);
        let verts: Vec<Vec3> = (0..8).flat_map(|k| [chain[k] + Vec3::new(0.0, 0.0, 0.03), chain[k] + Vec3::new(0.0, 0.0, -0.03)]).collect();
        let binds = bind_to_chains(&verts, &[&chain], &[Vec3::Z]).unwrap();
        let out = apply_chain_bindings(&binds, &[&chain], &[Vec3::Z]);
        for (v, o) in verts.iter().zip(&out) {
            assert!((o.unwrap() - *v).length() < 2e-4, "{v:?} vs {o:?}");
        }
        // movimento rigido: cadeia e eixo de referencia giram juntos
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.7, 0.3, -1.1);
        let tr = Vec3::new(0.2, 1.0, -0.4);
        let moved: Vec<Vec3> = chain.iter().map(|p| q * *p + tr).collect();
        let out = apply_chain_bindings(&binds, &[&moved], &[q * Vec3::Z]);
        for (v, o) in verts.iter().zip(&out) {
            assert!((o.unwrap() - (q * *v + tr)).length() < 3e-4);
        }
    }

    #[test]
    fn card_acompanha_a_dobra_da_cadeia() {
        let straight: Vec<Vec3> = (0..6).map(|k| Vec3::new(0.0, -(k as f32) * 0.1, 0.0)).collect();
        let verts = vec![Vec3::new(0.0, -0.45, 0.04), Vec3::new(0.0, -0.45, -0.04)];
        let binds = bind_to_chains(&verts, &[&straight], &[Vec3::Z]).unwrap();
        // dobra a cadeia em arco no plano XY (Z preservado): largura do card em Z deve se manter
        let bent = arc(6, 0.4, 1.0);
        let out = apply_chain_bindings(&binds, &[&bent], &[Vec3::Z]);
        let a = out[0].unwrap();
        let b = out[1].unwrap();
        assert!(((a - b).length() - 0.08).abs() < 1e-4, "largura do card preservada: {}", (a - b).length());
        assert!((a.z - 0.04).abs() < 1e-4 && (b.z + 0.04).abs() < 1e-4);
    }

    #[test]
    fn cadeia_degenerada_e_invalida() {
        assert!(ChainFrames::compute(&[Vec3::ZERO], Vec3::Z).is_none());
        assert!(ChainFrames::compute(&[Vec3::ZERO, Vec3::splat(f32::NAN)], Vec3::Z).is_none());
        // todos os pontos coincidentes: ainda produz frames finitos (tangente de fallback)
        let f = ChainFrames::compute(&[Vec3::ZERO, Vec3::ZERO, Vec3::ZERO], Vec3::Z).unwrap();
        assert!(f.frames.iter().all(|q| q.is_finite()));
        assert!(bind_to_chains(&[Vec3::ZERO], &[&[Vec3::ZERO]], &[Vec3::Z]).is_err());
        assert!(bind_to_chains(&[Vec3::ZERO], &[], &[Vec3::Z]).is_err());
        let chain = vec![Vec3::ZERO, Vec3::Y];
        let binds = bind_to_chains(&[Vec3::X], &[&chain], &[Vec3::Z]).unwrap();
        assert_eq!(apply_chain_bindings(&binds, &[], &[]), vec![None]);
    }
}
