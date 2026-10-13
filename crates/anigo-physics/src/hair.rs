//! Cabelo, rabos, fitas e acessórios pendurados: cadeias de partículas XPBD presas a um osso.
//!
//! Cada fio é uma [`ChainGuide`] (pontos no espaço local do osso-raiz, raiz primeiro, tipo neutro
//! que `anigo-hair` preenche a partir de `anigo-modeling::curve`). A simulação usa:
//!
//! * distância entre partículas consecutivas (compliance ~0: o fio não estica),
//! * flexão (distância pulando uma partícula),
//! * **restauração de ângulo** pai->filho em direção à pose animada, com rigidez que DECRESCE da
//!   raiz à ponta (`stiffness_root` -> `stiffness_tip`, curva `t^power`),
//! * tether (limite de alongamento total a partir da raiz),
//! * colisão das partículas (raio) com colliders do corpo (cabeça, ombros) em cápsulas/esferas.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::binding::ChainFrames;
use crate::collider::ColliderSet;
use crate::error::{check_index, PhysicsError, PhysicsResult};
use crate::math::{from_to_rotation, rotation_of, safe_normalize};
use crate::xpbd::{XpbdConfig, XpbdSystem};

/// Compliance máxima ("rigidez 0"): praticamente livre.
pub const SOFT_COMPLIANCE: f32 = 1e-2;

/// Mapeia rigidez artística `0..=1` (estilo VRoid) para compliance XPBD (m/N): `1 -> 0` (rígido),
/// `0 -> SOFT_COMPLIANCE`. Curva quadrática: resolução fina perto do lado rígido.
pub fn compliance_from_stiffness(stiffness: f32) -> f32 {
    let s = if stiffness.is_finite() { stiffness.clamp(0.0, 1.0) } else { 0.0 };
    SOFT_COMPLIANCE * (1.0 - s) * (1.0 - s)
}

/// Fio neutro: pontos no espaço LOCAL do osso-raiz (raiz primeiro).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChainGuide {
    pub points: Vec<Vec3>,
    /// Raio de colisão por ponto (m); vazio = `HairParams::particle_radius`.
    pub radii: Vec<f32>,
}

impl ChainGuide {
    pub fn from_points(points: Vec<Vec3>) -> Self {
        Self { points, radii: Vec::new() }
    }

    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }
}

/// Parâmetros "de animador" de um grupo de fios.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HairParams {
    /// Rigidez (0..1) na raiz e na ponta; interpolada por `t^stiffness_power`.
    pub stiffness_root: f32,
    pub stiffness_tip: f32,
    pub stiffness_power: f32,
    /// Compliance da distância entre partículas (m/N). ~0 = fio que não estica.
    pub stretch_compliance: f32,
    /// Compliance da flexão (distância pulando uma partícula). Menor = fio mais "duro".
    pub bending_compliance: f32,
    /// Quantas partículas iniciais acompanham o osso rigidamente (>= 1).
    pub fixed_count: usize,
    pub mass: f32,
    /// Fator de massa na ponta (1 = igual); lerp raiz->ponta.
    pub tip_mass_scale: f32,
    /// Amortecimento (1/s) na raiz e na ponta.
    pub damping_root: f32,
    pub damping_tip: f32,
    /// Escala da gravidade (cabelo de anime costuma ser "mais leve" que 1).
    pub gravity_scale: f32,
    pub particle_radius: f32,
    /// Alongamento máximo do total a partir da raiz (fração; tether). < 0 desliga.
    pub tether_stretch: f32,
}

impl Default for HairParams {
    fn default() -> Self {
        Self {
            stiffness_root: 0.97,
            stiffness_tip: 0.55,
            stiffness_power: 1.0,
            stretch_compliance: 1e-7,
            bending_compliance: 2e-5,
            fixed_count: 1,
            mass: 1.0,
            tip_mass_scale: 1.0,
            damping_root: 8.0,
            damping_tip: 3.0,
            gravity_scale: 0.7,
            particle_radius: 0.012,
            tether_stretch: 0.03,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HairChainDef {
    pub guide: ChainGuide,
    /// Osso a que a raiz está presa (índice na lista de matrizes de osso). `None` = espaço do mundo.
    pub root_bone: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HairGroupDef {
    pub name: String,
    pub chains: Vec<HairChainDef>,
    pub params: HairParams,
    pub xpbd: XpbdConfig,
    /// Colliders considerados (índices no `ColliderSet` do mundo); `None` = todos.
    pub collider_filter: Option<Vec<usize>>,
}

impl Default for HairGroupDef {
    fn default() -> Self {
        Self {
            name: "hair".into(),
            chains: Vec::new(),
            params: HairParams::default(),
            xpbd: XpbdConfig { substeps: 6, ..Default::default() },
            collider_filter: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChainRange {
    start: usize,
    len: usize,
    root_bone: Option<usize>,
}

/// Simulação de um grupo de fios.
#[derive(Debug, Clone)]
pub struct HairSim {
    sys: XpbdSystem,
    ranges: Vec<ChainRange>,
    /// Pontos de repouso, no espaço local do osso-raiz (mesma ordem das partículas).
    rest_local: Vec<Vec3>,
    /// Eixo do osso a usar como referência dos frames (espaço local do osso), por cadeia.
    ref_axis_local: Vec<Vec3>,
    params: HairParams,
    last_good_bones: Vec<Mat4>,
}

impl HairSim {
    /// Constrói a partir da pose de osso atual (`bone_world`), que define o repouso no mundo.
    pub fn new(def: &HairGroupDef, bone_world: &[Mat4]) -> PhysicsResult<Self> {
        let p = def.params;
        for (what, v) in [
            ("stiffness_root", p.stiffness_root),
            ("stiffness_tip", p.stiffness_tip),
            ("stiffness_power", p.stiffness_power),
            ("stretch_compliance", p.stretch_compliance),
            ("bending_compliance", p.bending_compliance),
            ("mass", p.mass),
            ("tip_mass_scale", p.tip_mass_scale),
            ("damping_root", p.damping_root),
            ("damping_tip", p.damping_tip),
            ("gravity_scale", p.gravity_scale),
            ("particle_radius", p.particle_radius),
            ("tether_stretch", p.tether_stretch),
        ] {
            if !v.is_finite() {
                return Err(PhysicsError::BadValue { what: format!("HairParams.{what}") });
            }
        }
        if p.mass <= 0.0 || p.stiffness_power <= 0.0 || p.stretch_compliance < 0.0 || p.bending_compliance < 0.0 {
            return Err(PhysicsError::BadConfig("HairParams: mass/stiffness_power > 0 e compliances >= 0".into()));
        }
        let mut sys = XpbdSystem::new(def.xpbd);
        sys.config.gravity *= p.gravity_scale;
        sys.collider_filter = def.collider_filter.clone();
        let mut ranges = Vec::new();
        let mut rest_local = Vec::new();
        let mut ref_axis_local = Vec::new();
        for (ci, chain) in def.chains.iter().enumerate() {
            let pts = &chain.guide.points;
            if pts.len() < 2 {
                return Err(PhysicsError::BadConfig(format!("fio {ci}: precisa de >= 2 pontos")));
            }
            if pts.iter().any(|v| !v.is_finite()) {
                return Err(PhysicsError::BadValue { what: format!("fio {ci}: ponto não finito") });
            }
            let bone_m = match chain.root_bone {
                Some(b) => {
                    check_index("hair.root_bone", b, bone_world.len())?;
                    if !bone_world[b].is_finite() {
                        return Err(PhysicsError::BadValue { what: format!("fio {ci}: osso não finito") });
                    }
                    bone_world[b]
                }
                None => Mat4::IDENTITY,
            };
            let total = chain.guide.length().max(1e-6);
            let start = sys.len();
            let fixed = p.fixed_count.clamp(1, pts.len() - 1);
            let mut acc = 0.0f32;
            for (k, pt) in pts.iter().enumerate() {
                if k > 0 {
                    acc += (*pt - pts[k - 1]).length();
                }
                let t = acc / total;
                let mass = if k < fixed { 0.0 } else { p.mass * (1.0 + (p.tip_mass_scale - 1.0) * t) };
                let radius = chain.guide.radii.get(k).copied().filter(|r| r.is_finite() && *r >= 0.0).unwrap_or(p.particle_radius);
                let idx = sys
                    .add_particle(bone_m.transform_point3(*pt), mass, radius)
                    .ok_or_else(|| PhysicsError::BadValue { what: format!("fio {ci}: posição no mundo") })?;
                let damping = p.damping_root + (p.damping_tip - p.damping_root) * t;
                sys.damping_scale[idx] = (damping / sys.config.damping.max(1e-6)).max(0.0);
                rest_local.push(*pt);
            }
            // restrições
            for k in 1..pts.len() {
                sys.add_distance_auto(start + k - 1, start + k, p.stretch_compliance);
            }
            for k in 2..pts.len() {
                sys.add_distance_auto(start + k - 2, start + k, p.bending_compliance);
            }
            acc = 0.0;
            for k in 1..pts.len() {
                acc += (pts[k] - pts[k - 1]).length();
                let t = acc / total;
                let s = p.stiffness_root + (p.stiffness_tip - p.stiffness_root) * t.powf(p.stiffness_power);
                if k >= fixed {
                    sys.add_direction(start + k - 1, start + k, compliance_from_stiffness(s));
                }
                if p.tether_stretch >= 0.0 && k >= fixed {
                    sys.add_tether(start + k, start + fixed - 1, 0.0, acc * (1.0 + p.tether_stretch), 0.0);
                }
            }
            // eixo de referência: o eixo do osso menos paralelo à primeira tangente
            let t0 = safe_normalize(pts[1] - pts[0], Vec3::Y);
            let axis = [Vec3::X, Vec3::Y, Vec3::Z].into_iter().min_by(|a, b| a.dot(t0).abs().total_cmp(&b.dot(t0).abs())).unwrap_or(Vec3::Z);
            ref_axis_local.push(axis);
            ranges.push(ChainRange { start, len: pts.len(), root_bone: chain.root_bone });
        }
        sys.settle_here();
        Ok(Self { sys, ranges, rest_local, ref_axis_local, params: p, last_good_bones: bone_world.to_vec() })
    }

    pub fn params(&self) -> &HairParams {
        &self.params
    }

    pub fn system(&self) -> &XpbdSystem {
        &self.sys
    }

    pub fn system_mut(&mut self) -> &mut XpbdSystem {
        &mut self.sys
    }

    pub fn chain_count(&self) -> usize {
        self.ranges.len()
    }

    pub fn particle_count(&self) -> usize {
        self.sys.len()
    }

    pub fn positions(&self) -> &[Vec3] {
        &self.sys.pos
    }

    pub fn chain_positions(&self, chain: usize) -> Option<&[Vec3]> {
        let r = self.ranges.get(chain)?;
        Some(&self.sys.pos[r.start..r.start + r.len])
    }

    /// Atualiza os alvos animados a partir da pose dos ossos. Matriz não finita mantém a última boa.
    pub fn set_pose(&mut self, bone_world: &[Mat4]) {
        for (i, m) in bone_world.iter().enumerate() {
            if m.is_finite() {
                if let Some(slot) = self.last_good_bones.get_mut(i) {
                    *slot = *m;
                }
            }
        }
        for r in &self.ranges {
            let m = r.root_bone.and_then(|b| self.last_good_bones.get(b)).copied().unwrap_or(Mat4::IDENTITY);
            for k in 0..r.len {
                let i = r.start + k;
                self.sys.targets[i] = m.transform_point3(self.rest_local[i]);
            }
        }
    }

    pub fn step(&mut self, dt: f32, colliders: &ColliderSet) {
        self.sys.step(dt, colliders);
    }

    /// Teleporte: tudo volta para a pose animada atual, sem velocidade.
    pub fn reset(&mut self) {
        self.sys.reset_to_targets();
    }

    /// Eixo de referência (mundo) da cadeia, a partir do osso-raiz atual.
    fn reference_world(&self, chain: usize) -> Vec3 {
        let r = self.ranges[chain];
        let m = r.root_bone.and_then(|b| self.last_good_bones.get(b)).copied().unwrap_or(Mat4::IDENTITY);
        rotation_of(&m) * self.ref_axis_local[chain]
    }

    /// Eixos de referência (mundo) de todas as cadeias, na ordem das cadeias.
    pub fn reference_axes(&self) -> Vec<Vec3> {
        (0..self.ranges.len()).map(|c| self.reference_world(c)).collect()
    }

    /// Rotações "swing" por segmento (mundo): quanto a direção simulada do segmento `k` (partícula
    /// `k` -> `k+1`) se afastou da direção animada. Para acionar ossos: `nova_rot_mundo =
    /// swing * rot_mundo_animada`. Cadeia inexistente = `None`.
    pub fn segment_swings(&self, chain: usize) -> Option<Vec<Quat>> {
        let r = self.ranges.get(chain)?;
        let mut out = Vec::with_capacity(r.len - 1);
        for k in 0..r.len - 1 {
            let i = r.start + k;
            let animated = safe_normalize(self.sys.targets[i + 1] - self.sys.targets[i], Vec3::Y);
            let current = safe_normalize(self.sys.pos[i + 1] - self.sys.pos[i], animated);
            out.push(from_to_rotation(animated, current));
        }
        Some(out)
    }

    /// Frames de transporte paralelo da cadeia simulada (para vincular/deformar cards).
    pub fn chain_frames(&self, chain: usize) -> Option<ChainFrames> {
        let pts = self.chain_positions(chain)?;
        ChainFrames::compute(pts, self.reference_world(chain))
    }

    /// Maior alongamento relativo entre partículas consecutivas (métrica de qualidade).
    pub fn max_stretch_ratio(&self) -> f32 {
        let mut worst = 0.0f32;
        for r in &self.ranges {
            for k in 1..r.len {
                let rest = (self.rest_local[r.start + k] - self.rest_local[r.start + k - 1]).length();
                if rest > 1e-9 {
                    let cur = (self.sys.pos[r.start + k] - self.sys.pos[r.start + k - 1]).length();
                    worst = worst.max((cur - rest) / rest);
                }
            }
        }
        worst
    }
}

/// Gera fios retos pendurados para testes/exemplos: `count` fios saindo de um anel em torno do
/// topo da cabeça, cada um com `points` pontos e `length` metros, no espaço local do osso.
pub fn demo_strands(count: usize, points: usize, length: f32, ring_radius: f32, root_height: f32) -> Vec<ChainGuide> {
    (0..count)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / count.max(1) as f32;
            let root = Vec3::new(ring_radius * a.cos(), root_height, ring_radius * a.sin());
            let out = Vec3::new(a.cos(), 0.0, a.sin());
            let pts = (0..points.max(2))
                .map(|k| {
                    let t = k as f32 / (points.max(2) - 1) as f32;
                    // sai um pouco para fora e cai
                    root + out * (0.03 * (1.0 - (1.0 - t).powi(2))) + Vec3::new(0.0, -length * t, 0.0)
                })
                .collect();
            ChainGuide::from_points(pts)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collider::{Collider, ColliderShape};

    const DT: f32 = 1.0 / 60.0;

    fn one_chain_def(params: HairParams) -> HairGroupDef {
        HairGroupDef {
            chains: vec![HairChainDef {
                guide: ChainGuide::from_points((0..8).map(|k| Vec3::new(0.0, -(k as f32) * 0.05, 0.0)).collect()),
                root_bone: Some(0),
            }],
            params,
            ..Default::default()
        }
    }

    fn bone(pos: Vec3, rot: Quat) -> Mat4 {
        Mat4::from_rotation_translation(rot, pos)
    }

    #[test]
    fn repouso_permanece_no_repouso_sem_movimento() {
        let mut hair = HairSim::new(&one_chain_def(HairParams { gravity_scale: 0.0, ..Default::default() }), &[bone(Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY)]).unwrap();
        let set = ColliderSet::new();
        let rest = hair.positions().to_vec();
        for _ in 0..300 {
            hair.step(DT, &set);
        }
        for (a, b) in rest.iter().zip(hair.positions()) {
            assert!((*a - *b).length() < 1e-4);
        }
    }

    #[test]
    fn rigidez_decrescente_ponta_balanca_mais_que_a_raiz() {
        // O osso gira rapido; a ponta deve defasar mais que o meio, que defasa mais que a raiz.
        let params = HairParams { stiffness_root: 0.98, stiffness_tip: 0.2, gravity_scale: 0.0, ..Default::default() };
        let mut hair = HairSim::new(&one_chain_def(params), &[bone(Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY)]).unwrap();
        let set = ColliderSet::new();
        let mut max_dev = [0.0f32; 8];
        for f in 0..180 {
            let t = f as f32 * DT;
            let rot = Quat::from_rotation_z((t * 9.0).sin() * 0.5);
            hair.set_pose(&[bone(Vec3::new(0.0, 1.5, 0.0), rot)]);
            hair.step(DT, &set);
            for k in 0..8 {
                max_dev[k] = max_dev[k].max((hair.positions()[k] - hair.system().targets[k]).length());
            }
        }
        eprintln!("desvio maximo ao alvo por partícula: {max_dev:.4?}");
        assert!(max_dev[0] < 1e-6, "raiz presa");
        assert!(max_dev[7] > max_dev[4] && max_dev[4] > max_dev[2], "ponta > meio > raiz");
        assert!(hair.max_stretch_ratio() < 0.05, "fio nao estica: {}", hair.max_stretch_ratio());
    }

    #[test]
    fn fio_pendurado_por_gravidade_assenta_proximo_da_vertical() {
        // Fio horizontal no repouso, gravidade ligada: rigidez da ponta baixa -> cai.
        let def = HairGroupDef {
            chains: vec![HairChainDef {
                guide: ChainGuide::from_points((0..8).map(|k| Vec3::new(k as f32 * 0.05, 0.0, 0.0)).collect()),
                root_bone: None,
            }],
            params: HairParams { stiffness_root: 0.3, stiffness_tip: 0.0, gravity_scale: 1.0, ..Default::default() },
            ..Default::default()
        };
        let mut hair = HairSim::new(&def, &[]).unwrap();
        let set = ColliderSet::new();
        for _ in 0..600 {
            hair.step(DT, &set);
        }
        let tip = hair.positions()[7];
        eprintln!("ponta do fio horizontal apos 10 s: {tip:?}");
        assert!(tip.y < -0.2, "a ponta caiu: {tip:?}");
        assert!(hair.max_stretch_ratio() < 0.05);
    }

    #[test]
    fn fios_nao_atravessam_a_cabeca() {
        // Cabeça = esfera r=0.1 em (0,1.5,0); fios saem do alto e caem. Balança forte do osso.
        // Roda COM colisor e SEM (controle): o controle precisa penetrar, senão o teste não prova nada.
        let head = Vec3::new(0.0, 1.5, 0.0);
        let mut set = ColliderSet::new();
        set.add(Collider::fixed(ColliderShape::Sphere { center: head, radius: 0.1 }));
        let mut results = Vec::new();
        for with_collider in [true, false] {
            let strands = demo_strands(12, 8, 0.35, 0.09, 0.0);
            let def = HairGroupDef {
                chains: strands.into_iter().map(|g| HairChainDef { guide: g, root_bone: Some(0) }).collect(),
                params: HairParams { stiffness_root: 0.9, stiffness_tip: 0.1, gravity_scale: 1.0, ..Default::default() },
                ..Default::default()
            };
            let mut hair = HairSim::new(&def, &[bone(head, Quat::IDENTITY)]).unwrap();
            let empty = ColliderSet::new();
            let mut worst = 0.0f32;
            for f in 0..360 {
                let t = f as f32 * DT;
                let rot = Quat::from_euler(glam::EulerRot::YXZ, (t * 6.0).sin() * 0.9, (t * 4.0).cos() * 0.4, 0.0);
                hair.set_pose(&[bone(head, rot)]);
                hair.step(DT, if with_collider { &set } else { &empty });
                for (k, p) in hair.positions().iter().enumerate() {
                    if k % 8 == 0 {
                        continue; // raiz fixa (nasce na superficie)
                    }
                    worst = worst.max(0.1 + hair.params().particle_radius - (*p - head).length());
                }
            }
            results.push((worst, hair.max_stretch_ratio()));
        }
        eprintln!(
            "cabeca: COM colisor pen {:.5} m / estiramento {:.4}; SEM colisor (controle) pen {:.5} m",
            results[0].0, results[0].1, results[1].0
        );
        assert!(results[1].0 > 0.01, "o controle precisa penetrar: {}", results[1].0);
        assert!(results[0].0 < 3e-3, "penetracao {}", results[0].0);
        assert!(results[0].1 < 0.08);
    }

    #[test]
    fn teleporte_do_osso_nao_explode() {
        let mut hair = HairSim::new(&one_chain_def(HairParams::default()), &[bone(Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY)]).unwrap();
        let set = ColliderSet::new();
        for _ in 0..60 {
            hair.step(DT, &set);
        }
        hair.set_pose(&[bone(Vec3::new(300.0, 1.5, -80.0), Quat::from_rotation_y(2.0))]);
        hair.step(DT, &set);
        assert!(hair.positions().iter().all(|p| p.is_finite()));
        for _ in 0..120 {
            hair.step(DT, &set);
        }
        let root = hair.positions()[0];
        assert!((root - Vec3::new(300.0, 1.5, -80.0)).length() < 1e-3);
        assert!(hair.max_stretch_ratio() < 0.08, "{}", hair.max_stretch_ratio());
        assert!(hair.system().stats().teleports >= 1);
    }

    #[test]
    fn swings_zero_no_repouso_e_nao_zero_apos_balanco() {
        let mut hair = HairSim::new(&one_chain_def(HairParams::default()), &[bone(Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY)]).unwrap();
        let set = ColliderSet::new();
        for q in hair.segment_swings(0).unwrap() {
            assert!(q.angle_between(Quat::IDENTITY) < 1e-4);
        }
        for f in 0..30 {
            hair.set_pose(&[bone(Vec3::new((f as f32 * 0.05).sin(), 1.5, 0.0), Quat::IDENTITY)]);
            hair.step(DT, &set);
        }
        let max_angle = hair.segment_swings(0).unwrap().iter().map(|q| q.angle_between(Quat::IDENTITY)).fold(0.0, f32::max);
        assert!(max_angle > 0.01, "o fio defasou do osso: {max_angle}");
        assert!(hair.segment_swings(9).is_none());
        let frames = hair.chain_frames(0).unwrap();
        assert_eq!(frames.frames.len(), 7);
    }

    #[test]
    fn construcao_invalida_e_rejeitada() {
        let bones = [Mat4::IDENTITY];
        let mut d = one_chain_def(HairParams::default());
        d.chains[0].guide.points.truncate(1);
        assert!(HairSim::new(&d, &bones).is_err());
        let mut d = one_chain_def(HairParams::default());
        d.chains[0].guide.points[2] = Vec3::splat(f32::NAN);
        assert!(HairSim::new(&d, &bones).is_err());
        let mut d = one_chain_def(HairParams::default());
        d.chains[0].root_bone = Some(5);
        assert!(HairSim::new(&d, &bones).is_err());
        let d = one_chain_def(HairParams { mass: 0.0, ..Default::default() });
        assert!(HairSim::new(&d, &bones).is_err());
        let d = one_chain_def(HairParams { stiffness_root: f32::NAN, ..Default::default() });
        assert!(HairSim::new(&d, &bones).is_err());
        assert!(HairSim::new(&one_chain_def(HairParams::default()), &[Mat4::from_translation(Vec3::splat(f32::NAN))]).is_err());
        // grupo vazio ok
        let mut empty = HairSim::new(&HairGroupDef::default(), &bones).unwrap();
        empty.step(DT, &ColliderSet::new());
        assert_eq!(empty.particle_count(), 0);
    }

    #[test]
    fn mapeamento_de_rigidez_e_monotono() {
        assert_eq!(compliance_from_stiffness(1.0), 0.0);
        assert!((compliance_from_stiffness(0.0) - SOFT_COMPLIANCE).abs() < 1e-9);
        let mut prev = f32::INFINITY;
        for i in 0..=10 {
            let c = compliance_from_stiffness(i as f32 / 10.0);
            assert!(c <= prev);
            prev = c;
        }
        assert_eq!(compliance_from_stiffness(f32::NAN), SOFT_COMPLIANCE);
        assert_eq!(compliance_from_stiffness(5.0), 0.0);
    }
}
