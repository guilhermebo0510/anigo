//! Spring bone EXATO do VRM (`VRMC_springBone-1.0`): integração de Verlet por junta, exatamente as
//! fórmulas da spec (seção "SpringBone Algorithm"):
//!
//! ```text
//! inertia   = (currentTail - prevTail) * (1 - dragForce)
//! stiffness = dt * parentWorldRot * initialLocalRot * boneAxis * stiffnessForce
//! external  = dt * gravityDir * gravityPower
//! nextTail  = currentTail + inertia + stiffness + external
//! nextTail  = headPos + normalize(nextTail - headPos) * boneLength
//! (colisão: nextTail -= dir * dist quando dist < 0; renormaliza o comprimento)
//! node.rotation = initialLocalRot * fromTo(boneAxis, normalize(inverse(parentWorld * initialLocal) * nextTail))
//! ```
//!
//! Ordem de avaliação: das raízes para os descendentes (ordenado por profundidade do nó-cabeça), e
//! antes de tudo as rotações das juntas voltam à rotação inicial (a animação não contamina a mola).
//! `center`: a inércia é avaliada no espaço do nó `center` (o movimento do personagem inteiro não
//! gera balanço); gravidade segue no mundo.
//!
//! Atenção (propriedade da spec, não bug): gravidade/rigidez são "deslocamento por segundo" somados
//! a cada passo e a deriva (`drag`) é por passo, então o "feeling" depende do passo. O ANIGO roda
//! sempre em passo fixo de 1/60 s ([`crate::FixedStepper`]); um pêndulo livre tem período
//! `2*pi*sqrt(L*dt/g)`, ou seja, gravidade efetiva `g/dt`.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::collider::{ColliderSet, ColliderShape};
use crate::error::{check_index, PhysicsError, PhysicsResult};
use crate::math::{from_to_rotation, rotation_of, safe_normalize, valid_dt};
use crate::skeleton::{Skeleton, SkeletonNode};

// ---------------------------------------------------------------------------
// Definições (serializáveis)
// ---------------------------------------------------------------------------

/// Parâmetros de uma junta (defaults da spec VRM 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpringJointParams {
    pub node: usize,
    pub hit_radius: f32,
    pub stiffness: f32,
    pub gravity_power: f32,
    pub gravity_dir: Vec3,
    pub drag_force: f32,
}

impl Default for SpringJointParams {
    fn default() -> Self {
        Self { node: 0, hit_radius: 0.0, stiffness: 1.0, gravity_power: 0.0, gravity_dir: Vec3::NEG_Y, drag_force: 0.5 }
    }
}

impl SpringJointParams {
    pub fn new(node: usize) -> Self {
        Self { node, ..Default::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SpringChainDef {
    pub name: String,
    pub joints: Vec<SpringJointParams>,
    pub collider_groups: Vec<usize>,
    pub center: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpringColliderDef {
    pub node: usize,
    pub shape: ColliderShape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SpringColliderGroupDef {
    pub name: String,
    pub colliders: Vec<usize>,
}

/// Rig completo: nós + cadeias + colliders. É o equivalente em memória de `VRMC_springBone`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SpringBoneDef {
    pub nodes: Vec<SkeletonNode>,
    pub chains: Vec<SpringChainDef>,
    pub colliders: Vec<SpringColliderDef>,
    pub collider_groups: Vec<SpringColliderGroupDef>,
}

// ---------------------------------------------------------------------------
// Sistema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Pair {
    head: usize,
    tail: usize,
    params: SpringJointParams,
    /// Direção do tail no espaço local do head (unitária).
    bone_axis: Vec3,
    /// Posição do tail no espaço local do head, em repouso.
    local_tail: Vec3,
    /// Comprimento (mundo) head -> tail em repouso.
    bone_length: f32,
    initial_rot: Quat,
    initial_local: Mat4,
    /// Tails no espaço do `center` (ou do mundo se não há center).
    prev_tail: Vec3,
    cur_tail: Vec3,
    center: Option<usize>,
    colliders: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpringStats {
    /// Juntas cujo estado ficou não finito e foi reiniciado.
    pub nan_resets: u32,
    pub steps: u64,
}

#[derive(Debug, Clone)]
pub struct SpringBoneSystem {
    skeleton: Skeleton,
    pairs: Vec<Pair>,
    colliders: ColliderSet,
    warnings: Vec<String>,
    stats: SpringStats,
}

impl SpringBoneSystem {
    pub fn new(def: &SpringBoneDef) -> PhysicsResult<Self> {
        let skeleton = Skeleton::new(def.nodes.clone())?;
        let n = skeleton.len();
        let mut warnings = Vec::new();

        let mut colliders = ColliderSet::new();
        let mut collider_map: Vec<Option<usize>> = Vec::new();
        for (i, c) in def.colliders.iter().enumerate() {
            check_index("collider.node", c.node, n)?;
            match colliders.add(crate::collider::Collider { shape: c.shape, bone: Some(c.node), ..Default::default() }) {
                Some(id) => {
                    collider_map.push(Some(id));
                }
                None => {
                    warnings.push(format!("collider {i} inválido (NaN/raio negativo) descartado"));
                    collider_map.push(None);
                }
            }
        }
        for g in &def.collider_groups {
            for &c in &g.colliders {
                check_index("colliderGroup.colliders", c, def.colliders.len())?;
            }
        }

        let mut pairs = Vec::new();
        for (ci, chain) in def.chains.iter().enumerate() {
            if let Some(center) = chain.center {
                check_index("chain.center", center, n)?;
            }
            for g in &chain.collider_groups {
                check_index("chain.colliderGroups", *g, def.collider_groups.len())?;
            }
            for j in &chain.joints {
                check_index("chain.joint.node", j.node, n)?;
                if !(j.hit_radius.is_finite() && j.stiffness.is_finite() && j.gravity_power.is_finite()
                    && j.gravity_dir.is_finite() && j.drag_force.is_finite())
                {
                    return Err(PhysicsError::BadValue { what: format!("chain {ci} joint {} parâmetros", j.node) });
                }
            }
            let mut collider_idx: Vec<usize> = chain
                .collider_groups
                .iter()
                .flat_map(|g| def.collider_groups[*g].colliders.iter().copied())
                .filter_map(|c| collider_map[c])
                .collect();
            collider_idx.sort_unstable();
            collider_idx.dedup();

            for w in chain.joints.windows(2) {
                let (head, tail) = (w[0].node, w[1].node);
                if !skeleton.is_ancestor(head, tail) {
                    warnings.push(format!(
                        "chain {ci}: nó {tail} não é descendente do nó {head} — par ignorado (a spec exige ancestralidade)"
                    ));
                    continue;
                }
                let rest_head = skeleton.rest_world(head);
                let rest_tail = skeleton.rest_world(tail);
                let local_tail = rest_head.inverse().transform_point3(rest_tail.w_axis.truncate());
                let bone_length = (rest_tail.w_axis.truncate() - rest_head.w_axis.truncate()).length();
                if !(bone_length.is_finite() && bone_length > 1e-8) || !local_tail.is_finite() {
                    warnings.push(format!("chain {ci}: osso {head}->{tail} com comprimento nulo — par ignorado"));
                    continue;
                }
                let initial_rot = skeleton.rest_local_rotation(head);
                let initial_local = skeleton.rest_local_matrix(head);
                pairs.push(Pair {
                    head,
                    tail,
                    params: w[0],
                    bone_axis: safe_normalize(local_tail, Vec3::Y),
                    local_tail,
                    bone_length,
                    initial_rot,
                    initial_local,
                    prev_tail: Vec3::ZERO,
                    cur_tail: Vec3::ZERO,
                    center: chain.center,
                    colliders: collider_idx.clone(),
                });
            }
        }
        // Pais antes dos filhos (entre cadeias também): ordem estável por profundidade do head.
        pairs.sort_by_key(|p| skeleton.depth(p.head));
        let mut sys = Self { skeleton, pairs, colliders, warnings, stats: SpringStats::default() };
        sys.reset();
        Ok(sys)
    }

    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    /// Para aplicar a pose animada dos nós antes de `step`.
    pub fn skeleton_mut(&mut self) -> &mut Skeleton {
        &mut self.skeleton
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn stats(&self) -> SpringStats {
        self.stats
    }

    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// `(head, tail)` do par `i` (na ordem de avaliação).
    pub fn pair_nodes(&self, i: usize) -> Option<(usize, usize)> {
        self.pairs.get(i).map(|p| (p.head, p.tail))
    }

    pub fn pair_bone_length(&self, i: usize) -> Option<f32> {
        self.pairs.get(i).map(|p| p.bone_length)
    }

    /// Posição simulada do tail do par `i` no MUNDO.
    pub fn tail_world(&self, i: usize) -> Option<Vec3> {
        let p = self.pairs.get(i)?;
        Some(match p.center {
            Some(c) => self.skeleton.world(c).transform_point3(p.cur_tail),
            None => p.cur_tail,
        })
    }

    /// Rotação local simulada de um nó (a mesma que um player VRM aplicaria).
    pub fn node_rotation(&self, node: usize) -> Quat {
        self.skeleton.local_rotation(node)
    }

    fn rest_tail_world(&self, p: &Pair) -> Vec3 {
        let parent_world = self.skeleton.parent(p.head).map(|q| self.skeleton.world(q)).unwrap_or(Mat4::IDENTITY);
        (parent_world * p.initial_local).transform_point3(p.local_tail)
    }

    /// Reinicia o estado (teleporte): rotações voltam ao inicial e os tails vão para o repouso
    /// relativo à pose atual dos pais. Seguro para chamar a qualquer momento.
    pub fn reset(&mut self) {
        for p in &self.pairs {
            self.skeleton.set_local_rotation(p.head, p.initial_rot);
        }
        self.skeleton.update_world();
        for i in 0..self.pairs.len() {
            let tail = self.rest_tail_world(&self.pairs[i]);
            let stored = match self.pairs[i].center {
                Some(c) => self.skeleton.world(c).inverse().transform_point3(tail),
                None => tail,
            };
            self.pairs[i].prev_tail = stored;
            self.pairs[i].cur_tail = stored;
        }
    }

    /// Um passo de simulação (a spec assume passo fixo; use [`crate::FixedStepper`]).
    /// `dt` inválido é ignorado. Valores enormes de `dt` não explodem: o comprimento do osso é
    /// sempre restaurado.
    pub fn step(&mut self, dt: f32) {
        let Some(dt) = valid_dt(dt) else { return };
        self.stats.steps += 1;

        // 1) Rotações das juntas voltam ao inicial; FK completa; colliders no mundo.
        for p in &self.pairs {
            self.skeleton.set_local_rotation(p.head, p.initial_rot);
        }
        self.skeleton.update_world();
        self.colliders.update(self.skeleton.world_matrices());

        // 2) Avalia cada par, raiz -> folha.
        for i in 0..self.pairs.len() {
            self.step_pair(i, dt);
        }
    }

    fn step_pair(&mut self, i: usize, dt: f32) {
        let p = self.pairs[i].clone();
        let sk = &self.skeleton;
        let head_world = sk.world(p.head);
        let head_pos = head_world.w_axis.truncate();
        let parent_world = sk.parent(p.head).map(|q| sk.world(q)).unwrap_or(Mat4::IDENTITY);
        let parent_rot = rotation_of(&parent_world);

        let center_world = p.center.map(|c| sk.world(c));
        let to_world = |v: Vec3| center_world.map(|m| m.transform_point3(v)).unwrap_or(v);
        let cur = to_world(p.cur_tail);
        let prev = to_world(p.prev_tail);

        let gravity_dir = safe_normalize(p.params.gravity_dir, Vec3::NEG_Y);
        let drag = p.params.drag_force.clamp(0.0, 1.0);
        let inertia = (cur - prev) * (1.0 - drag);
        let stiffness_dir = parent_rot * p.initial_rot * p.bone_axis;
        let stiffness = stiffness_dir * (dt * p.params.stiffness.max(0.0));
        let external = gravity_dir * (dt * p.params.gravity_power);

        let constrain = |t: Vec3| head_pos + safe_normalize(t - head_pos, stiffness_dir) * p.bone_length;
        let mut next = constrain(cur + inertia + stiffness + external);

        for &ci in &p.colliders {
            if let (Some(c), Some(shape)) = (self.colliders.get(ci), self.colliders.world_shapes().get(ci)) {
                if !c.enabled {
                    continue;
                }
                let (gap, dir) = shape.gap(next, p.params.hit_radius.max(0.0));
                if gap < 0.0 {
                    next = constrain(next - dir * gap);
                }
            }
        }

        if !next.is_finite() {
            // Estado corrompido: volta ao repouso (nunca propaga NaN para a pose).
            self.stats.nan_resets += 1;
            let tail = self.rest_tail_world(&p);
            let stored = center_world.map(|m| m.inverse().transform_point3(tail)).unwrap_or(tail);
            self.pairs[i].prev_tail = stored;
            self.pairs[i].cur_tail = stored;
            self.skeleton.set_local_rotation(p.head, p.initial_rot);
            self.skeleton.update_subtree(p.head);
            return;
        }

        let to_stored = |v: Vec3| center_world.map(|m| m.inverse().transform_point3(v)).unwrap_or(v);
        self.pairs[i].prev_tail = p.cur_tail;
        self.pairs[i].cur_tail = to_stored(next);

        // Aplica a rotação: direção do tail no espaço do head em repouso.
        let local_next = (parent_world * p.initial_local).inverse().transform_point3(next);
        let to = safe_normalize(local_next, p.bone_axis);
        let rotation = p.initial_rot * from_to_rotation(p.bone_axis, to);
        self.skeleton.set_local_rotation(p.head, rotation);
        self.skeleton.update_subtree(p.head);
    }
}

// ---------------------------------------------------------------------------
// Testes (casos analíticos)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::state_hash;
    use crate::stepper::FixedStepper;

    const DT: f32 = 1.0 / 60.0;

    /// root(0) -> head(1) (mesma posição) -> tail(2) em `tail_rest`.
    fn single_bone(tail_rest: Vec3, params: SpringJointParams) -> SpringBoneDef {
        SpringBoneDef {
            nodes: vec![
                SkeletonNode::new("root", None, Vec3::ZERO),
                SkeletonNode::new("head", Some(0), Vec3::ZERO),
                SkeletonNode::new("tail", Some(1), tail_rest),
            ],
            chains: vec![SpringChainDef {
                name: "c".into(),
                joints: vec![SpringJointParams { node: 1, ..params }, SpringJointParams::new(2)],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn tail_fk(sys: &SpringBoneSystem) -> Vec3 {
        sys.skeleton().world_position(2)
    }

    #[test]
    fn repouso_sem_forcas_nada_se_move() {
        let def = single_bone(Vec3::new(0.0, -0.4, 0.0), SpringJointParams { stiffness: 1.0, gravity_power: 0.0, ..Default::default() });
        let mut sys = SpringBoneSystem::new(&def).unwrap();
        for _ in 0..600 {
            sys.step(DT);
        }
        assert!((tail_fk(&sys) - Vec3::new(0.0, -0.4, 0.0)).length() < 1e-5);
        assert!(sys.node_rotation(1).angle_between(Quat::IDENTITY) < 1e-5);
    }

    #[test]
    fn pendulo_livre_periodo_bate_com_a_teoria() {
        // stiffness 0, drag 0, gravidade 1: g_efetivo = 1/dt; T = 2*pi*sqrt(L*dt/g).
        let l = 0.5f32;
        let theta0 = 0.05f32;
        let tail = Vec3::new(l * theta0.sin(), -l * theta0.cos(), 0.0);
        let params = SpringJointParams { stiffness: 0.0, drag_force: 0.0, gravity_power: 1.0, ..Default::default() };
        let mut sys = SpringBoneSystem::new(&single_bone(tail, params)).unwrap();
        let expected_t = 2.0 * std::f32::consts::PI * (l * DT / 1.0).sqrt();
        // Cruzamentos de x por zero, de + para - e de - para +: meio período entre cruzamentos.
        let mut crossings = Vec::new();
        let mut prev_x = sys.tail_world(0).unwrap().x;
        for step in 1..=400 {
            sys.step(DT);
            let x = sys.tail_world(0).unwrap().x;
            if prev_x.signum() != x.signum() {
                // interpolação linear do instante do cruzamento
                let frac = prev_x / (prev_x - x);
                crossings.push((step as f32 - 1.0 + frac) * DT);
            }
            prev_x = x;
        }
        assert!(crossings.len() >= 8, "poucas oscilações: {}", crossings.len());
        let span = crossings[crossings.len() - 1] - crossings[0];
        let half_periods = (crossings.len() - 1) as f32;
        let measured_t = 2.0 * span / half_periods;
        let err = (measured_t - expected_t).abs() / expected_t;
        eprintln!("pendulo: T medido {measured_t:.5}s, teorico {expected_t:.5}s, erro {:.3}%", err * 100.0);
        assert!(err < 0.02, "periodo medido {measured_t} vs teorico {expected_t}");
        // comprimento preservado e amplitude conservada (drag 0): energia aproximadamente constante
        let l_now = (tail_fk(&sys) - sys.skeleton().world_position(1)).length();
        assert!((l_now - l).abs() < 1e-5);
    }

    #[test]
    fn repouso_sob_gravidade_equilibrio_analitico() {
        // Equilíbrio: tail na direção normalize(s*e + g*gd). e = +X, gd = -Y.
        let l = 0.4f32;
        for (s, g) in [(1.0f32, 1.0f32), (2.0, 1.0), (0.5, 1.5)] {
            let params = SpringJointParams { stiffness: s, gravity_power: g, drag_force: 0.5, ..Default::default() };
            let mut sys = SpringBoneSystem::new(&single_bone(Vec3::new(l, 0.0, 0.0), params)).unwrap();
            for _ in 0..2000 {
                sys.step(DT);
            }
            let expected = Vec3::new(s, -g, 0.0).normalize() * l;
            let got = tail_fk(&sys);
            eprintln!("equilibrio s={s} g={g}: esperado {expected:?}, obtido {got:?}");
            assert!((got - expected).length() < 1e-4, "s={s} g={g}: {got:?} vs {expected:?}");
            // a rotação do nó leva +X na direção de equilíbrio
            let dir = sys.node_rotation(1) * Vec3::X;
            assert!((dir - expected.normalize()).length() < 1e-4);
        }
    }

    #[test]
    fn drag_dissipa_a_energia_do_pendulo() {
        let l = 0.5f32;
        let tail = Vec3::new(l * 0.3f32.sin(), -l * 0.3f32.cos(), 0.0);
        let params = SpringJointParams { stiffness: 0.0, drag_force: 0.1, gravity_power: 1.0, ..Default::default() };
        let mut sys = SpringBoneSystem::new(&single_bone(tail, params)).unwrap();
        let mut max_first = 0.0f32;
        let mut max_last = 0.0f32;
        for step in 0..600 {
            sys.step(DT);
            let x = sys.tail_world(0).unwrap().x.abs();
            if step < 60 {
                max_first = max_first.max(x);
            }
            if step >= 540 {
                max_last = max_last.max(x);
            }
        }
        eprintln!("amplitude inicial {max_first:.4}, final {max_last:.6}");
        assert!(max_last < 0.1 * max_first);
    }

    #[test]
    fn comprimento_do_osso_e_preservado_com_animacao_violenta() {
        // Cadeia de 6 ossos pendurada em um root que balança/gira.
        let mut nodes = vec![SkeletonNode::new("root", None, Vec3::new(0.0, 2.0, 0.0))];
        for k in 1..=6 {
            nodes.push(SkeletonNode::new(format!("n{k}"), Some(k - 1), Vec3::new(0.0, -0.1, 0.0)));
        }
        let joints: Vec<SpringJointParams> = (1..=6)
            .map(|n| SpringJointParams { node: n, stiffness: 0.6, gravity_power: 1.0, drag_force: 0.4, ..Default::default() })
            .collect();
        let def = SpringBoneDef { nodes, chains: vec![SpringChainDef { joints, ..Default::default() }], ..Default::default() };
        let mut sys = SpringBoneSystem::new(&def).unwrap();
        let mut max_len_err = 0.0f32;
        let mut max_fk_err = 0.0f32;
        for step in 0..1200 {
            let t = step as f32 * DT;
            let q = Quat::from_euler(glam::EulerRot::XYZ, (t * 7.0).sin() * 1.2, t * 3.0, (t * 5.0).cos());
            sys.skeleton_mut().set_local(0, Vec3::new((t * 4.0).sin(), 2.0, (t * 2.0).cos()), q);
            sys.step(DT);
            for i in 0..sys.pair_count() {
                let (h, tl) = sys.pair_nodes(i).unwrap();
                let hp = sys.skeleton().world_position(h);
                let tp = sys.skeleton().world_position(tl);
                max_len_err = max_len_err.max(((tp - hp).length() - sys.pair_bone_length(i).unwrap()).abs());
                // a rotação aplicada reproduz o tail simulado
                max_fk_err = max_fk_err.max((tp - sys.tail_world(i).unwrap()).length());
            }
        }
        eprintln!("erro max de comprimento {max_len_err:.2e}, erro FK vs tail {max_fk_err:.2e}");
        assert!(max_len_err < 2e-5);
        assert!(max_fk_err < 5e-4);
    }

    #[test]
    fn estabilidade_com_dt_grande() {
        let params = SpringJointParams { stiffness: 3.0, gravity_power: 5.0, drag_force: 0.2, ..Default::default() };
        let mut sys = SpringBoneSystem::new(&single_bone(Vec3::new(0.0, -0.4, 0.0), params)).unwrap();
        for dt in [0.1, 0.5, 1.0, 10.0, 1e4, 1e8] {
            for _ in 0..20 {
                sys.step(dt);
                let t = sys.tail_world(0).unwrap();
                assert!(t.is_finite(), "dt={dt}");
                let len = (tail_fk(&sys) - sys.skeleton().world_position(1)).length();
                assert!((len - 0.4).abs() < 1e-3, "dt={dt}: comprimento {len}");
            }
        }
        // dt invalido e ignorado
        let before = sys.tail_world(0).unwrap();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            sys.step(dt);
        }
        assert_eq!(sys.tail_world(0).unwrap(), before);
    }

    #[test]
    fn colisao_com_esfera_o_osso_para_na_superficie() {
        // Osso horizontal que cai por gravidade; esfera na trajetória.
        let l = 0.5f32;
        let params = SpringJointParams { stiffness: 0.3, gravity_power: 1.0, drag_force: 0.5, hit_radius: 0.02, ..Default::default() };
        let mut def = single_bone(Vec3::new(l, 0.0, 0.0), params);
        // Esfera exatamente sobre o arco do tail, 55 graus abaixo da horizontal.
        let ang = 55f32.to_radians();
        let center = Vec3::new(l * ang.cos(), -l * ang.sin(), 0.0);
        def.nodes.push(SkeletonNode::new("col", Some(0), center));
        def.colliders.push(SpringColliderDef { node: 3, shape: ColliderShape::Sphere { center: Vec3::ZERO, radius: 0.08 } });
        def.collider_groups.push(SpringColliderGroupDef { name: "g".into(), colliders: vec![0] });
        def.chains[0].collider_groups = vec![0];

        let mut with = SpringBoneSystem::new(&def).unwrap();
        let mut without_def = def.clone();
        without_def.chains[0].collider_groups.clear();
        let mut without = SpringBoneSystem::new(&without_def).unwrap();
        let mut worst_pen = 0.0f32;
        for _ in 0..1500 {
            with.step(DT);
            without.step(DT);
            let d = (with.tail_world(0).unwrap() - center).length() - 0.08 - 0.02;
            worst_pen = worst_pen.max(-d);
        }
        let d_final = (with.tail_world(0).unwrap() - center).length() - 0.08 - 0.02;
        let free = without.tail_world(0).unwrap();
        let blocked = with.tail_world(0).unwrap();
        eprintln!("colisao: penetracao maxima {worst_pen:.5} m, folga final {d_final:.5} m, y livre {:.3}, y bloqueado {:.3}", free.y, blocked.y);
        assert!(worst_pen < 0.01, "penetracao maxima {worst_pen}");
        assert!(d_final > -1e-3, "folga final {d_final}");
        assert!(free.y < blocked.y - 0.05, "sem colisor o osso cai mais");
    }

    #[test]
    fn center_space_ignora_o_movimento_do_personagem() {
        // O mesmo osso, com e sem `center`, quando o root inteiro se desloca (sem girar).
        let params = SpringJointParams { stiffness: 0.2, gravity_power: 0.0, drag_force: 0.1, ..Default::default() };
        let mut def = single_bone(Vec3::new(0.0, -0.4, 0.0), params);
        let mut with_center = def.clone();
        with_center.chains[0].center = Some(0);
        let mut a = SpringBoneSystem::new(&def).unwrap();
        let mut b = SpringBoneSystem::new(&with_center).unwrap();
        let mut dev_a = 0.0f32;
        let mut dev_b = 0.0f32;
        for step in 0..120 {
            let x = (step as f32 * DT * 6.0).sin() * 2.0; // anda 2 m para os lados
            a.skeleton_mut().set_local_translation(0, Vec3::new(x, 0.0, 0.0));
            b.skeleton_mut().set_local_translation(0, Vec3::new(x, 0.0, 0.0));
            a.step(DT);
            b.step(DT);
            dev_a = dev_a.max((a.tail_world(0).unwrap() - a.skeleton().world_position(1) - Vec3::new(0.0, -0.4, 0.0)).length());
            dev_b = dev_b.max((b.tail_world(0).unwrap() - b.skeleton().world_position(1) - Vec3::new(0.0, -0.4, 0.0)).length());
        }
        eprintln!("desvio sem center {dev_a:.4}, com center {dev_b:.2e}");
        assert!(dev_a > 0.02, "sem center o osso balanca com o personagem");
        assert!(dev_b < 1e-4, "com center a translacao pura nao gera balanco");
        def.chains.clear();
    }

    #[test]
    fn cadeias_ordenadas_raiz_para_folha_e_juntas_invalidas_sao_puladas() {
        let mut nodes = vec![SkeletonNode::new("root", None, Vec3::ZERO)];
        for k in 1..=3 {
            nodes.push(SkeletonNode::new(format!("n{k}"), Some(k - 1), Vec3::new(0.0, -0.2, 0.0)));
        }
        nodes.push(SkeletonNode::new("solto", Some(0), Vec3::X));
        // cadeia com junta fora de ordem (4 nao e descendente de 3) -> par ignorado + aviso
        let def = SpringBoneDef {
            nodes,
            chains: vec![SpringChainDef {
                joints: vec![SpringJointParams::new(1), SpringJointParams::new(2), SpringJointParams::new(3), SpringJointParams::new(4)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let sys = SpringBoneSystem::new(&def).unwrap();
        assert_eq!(sys.pair_count(), 2);
        assert!(sys.warnings().iter().any(|w| w.contains("descendente")));
        assert_eq!(sys.pair_nodes(0), Some((1, 2)));
        assert_eq!(sys.pair_nodes(1), Some((2, 3)));
    }

    #[test]
    fn entradas_malformadas_nao_causam_panic() {
        let base = single_bone(Vec3::new(0.0, -0.4, 0.0), SpringJointParams::default());
        let mut d = base.clone();
        d.chains[0].joints[0].node = 99;
        assert!(SpringBoneSystem::new(&d).is_err());
        let mut d = base.clone();
        d.chains[0].center = Some(99);
        assert!(SpringBoneSystem::new(&d).is_err());
        let mut d = base.clone();
        d.chains[0].joints[0].stiffness = f32::NAN;
        assert!(SpringBoneSystem::new(&d).is_err());
        let mut d = base.clone();
        d.chains[0].collider_groups = vec![3];
        assert!(SpringBoneSystem::new(&d).is_err());
        let mut d = base.clone();
        d.collider_groups.push(SpringColliderGroupDef { name: "x".into(), colliders: vec![5] });
        assert!(SpringBoneSystem::new(&d).is_err());
        let mut d = base.clone();
        d.nodes[2].translation = Vec3::ZERO; // osso de comprimento zero
        let sys = SpringBoneSystem::new(&d).unwrap();
        assert_eq!(sys.pair_count(), 0);
        // Pose animada com NaN e ignorada
        let mut sys = SpringBoneSystem::new(&base).unwrap();
        sys.skeleton_mut().set_local(0, Vec3::splat(f32::NAN), Quat::IDENTITY);
        sys.step(DT);
        assert!(sys.tail_world(0).unwrap().is_finite());
        // Rig vazio funciona
        let mut empty = SpringBoneSystem::new(&SpringBoneDef::default()).unwrap();
        empty.step(DT);
        empty.reset();
    }

    #[test]
    fn determinismo_mesmo_estado_apos_n_passos() {
        let params = SpringJointParams { stiffness: 0.7, gravity_power: 1.0, drag_force: 0.3, ..Default::default() };
        let run = || {
            let mut sys = SpringBoneSystem::new(&single_bone(Vec3::new(0.3, -0.2, 0.0), params)).unwrap();
            let mut stepper = FixedStepper::default();
            let mut hashes = Vec::new();
            for frame in 0..300 {
                let t = frame as f32 / 75.0;
                sys.skeleton_mut().set_local(0, Vec3::new(t.sin(), 0.0, 0.0), Quat::from_rotation_y(t));
                for _ in 0..stepper.advance(1.0 / 75.0).steps {
                    sys.step(DT);
                }
                hashes.push(state_hash(&[sys.tail_world(0).unwrap(), tail_fk(&sys)]));
            }
            hashes
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn reset_teleporte_nao_explode() {
        let params = SpringJointParams { stiffness: 0.5, gravity_power: 1.0, drag_force: 0.3, ..Default::default() };
        let mut sys = SpringBoneSystem::new(&single_bone(Vec3::new(0.0, -0.4, 0.0), params)).unwrap();
        for _ in 0..60 {
            sys.step(DT);
        }
        // teleporta o personagem 1000 m; sem reset o tail e puxado, com reset fica no repouso
        sys.skeleton_mut().set_local_translation(0, Vec3::new(1000.0, 0.0, 0.0));
        sys.reset();
        let tail = sys.tail_world(0).unwrap();
        assert!((tail - Vec3::new(1000.0, -0.4, 0.0)).length() < 1e-3);
        sys.step(DT);
        assert!((sys.tail_world(0).unwrap() - tail).length() < 0.05, "sem salto apos o reset");
    }
}
