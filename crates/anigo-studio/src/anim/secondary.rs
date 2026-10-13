//! Física secundária (cabelo e saia) no playback da Animação.
//!
//! O renderer ainda não consome cabelo/saia simulados (contrato de 24 ossos congelado; ver `FISICA.md` §7), então a
//! simulação roda de VERDADE sobre os ossos do clip — cadeias de cabelo na cabeça e uma saia no quadril, contra as 20
//! cápsulas do corpo — e é entregue como DADO (posições por partícula) e como overlay de depuração no viewport.
//!
//! ## Determinismo
//! * passo FIXO de 1/60 s (o `PhysicsWorld` já acumula; aqui chamamos com `dt` exato do passo, então um passo por chamada);
//! * a pose usada em cada passo é `clip.evaluate(t)` sobre o REPOUSO (a simulação é função de `(clip, t)`, não do que
//!   o usuário deixou editado à mão);
//! * ao SALTAR no tempo (scrub, loop, ir ao início, tocar do meio) a simulação é reiniciada na pose de `t − pre_roll`
//!   (zerando velocidades) e avança até `t` em passos fixos — o pré-aquecimento. Resultado: o mesmo `t` dá o mesmo estado,
//!   qualquer que seja o caminho até ele, e o cabelo não nasce "morto" nem arrastado pelo salto.
//!   Engines de produção fazem reset/teleporte para o mesmo fim [S: fóruns de Chaos Cloth / Ragdoll Dynamics].

use anigo_core::pose::{bone, PoseRig, PoseState, BONE_COUNT};
use anigo_core::project::AnimationClip;
use anigo_physics::math::state_hash;
use anigo_physics::{
    skirt_proxy, ChainGuide, ClothHandle, HairChainDef, HairGroupDef, HairHandle, PhysicsPreset, PhysicsWorld, StepReport, WorldConfig,
};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// Passo fixo da simulação (s).
pub const SIM_DT: f32 = 1.0 / 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SecondaryConfig {
    pub hair: bool,
    pub skirt: bool,
    /// Pré-aquecimento ao saltar no tempo, em segundos.
    pub pre_roll_s: f32,
    pub strands: usize,
    pub points: usize,
    pub columns: usize,
    pub rings: usize,
}

impl Default for SecondaryConfig {
    fn default() -> Self {
        SecondaryConfig { hair: true, skirt: true, pre_roll_s: 0.4, strands: 14, points: 8, columns: 16, rings: 6 }
    }
}

/// Matrizes de mundo posadas dos 24 ossos (`skin · T(bind)`): é o que o `PhysicsWorld::step` espera.
pub fn bone_world(rig: &PoseRig, pose: &PoseState) -> Vec<Mat4> {
    let skin = rig.skin_palette(pose);
    (0..BONE_COUNT).map(|i| skin[i] * Mat4::from_translation(rig.bind[i])).collect()
}

/// Matrizes de mundo no repouso.
pub fn rest_world(rig: &PoseRig) -> Vec<Mat4> {
    (0..BONE_COUNT).map(|i| Mat4::from_translation(rig.bind[i])).collect()
}

pub struct SecondarySim {
    world: PhysicsWorld,
    hair: Option<(HairHandle, usize, usize)>,
    skirt: Option<(ClothHandle, Vec<[u32; 3]>, usize)>,
    rig: PoseRig,
    cfg: SecondaryConfig,
    /// Tempo de animação (s) do último passo simulado.
    clock: Option<f64>,
    /// Quantos reinícios (saltos) aconteceram.
    pub resets: u32,
    /// Passos fixos executados desde o início.
    pub steps: u64,
    pub last_report: StepReport,
}

impl SecondarySim {
    pub fn new(rig: &PoseRig, cfg: SecondaryConfig) -> Result<SecondarySim, String> {
        let rest = rest_world(rig);
        let mut world = PhysicsWorld::new(WorldConfig::default());
        world.add_body_colliders(&rest, 1.0, 0.2).map_err(|e| e.to_string())?;
        let head = rig.bind[bone::HEAD];
        let _ = head;
        let hair = if cfg.hair {
            let chains: Vec<HairChainDef> = (0..cfg.strands)
                .filter_map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / cfg.strands as f32;
                    // sem fios sobre o rosto (+Z): deixa uma abertura de 70° na frente
                    if a.cos() > 0.82 && a.sin().abs() < 0.57 {
                        return None;
                    }
                    let dir = Vec3::new(a.sin(), 0.0, a.cos());
                    let pts: Vec<Vec3> = (0..cfg.points)
                        .map(|k| {
                            let t = k as f32 / (cfg.points - 1).max(1) as f32;
                            Vec3::new(0.0, 0.12 - 0.5 * t, 0.0) + dir * (0.09 + 0.07 * t.powf(0.6))
                        })
                        .collect();
                    Some(HairChainDef { guide: ChainGuide::from_points(pts), root_bone: Some(bone::HEAD) })
                })
                .collect();
            let n = chains.len();
            let def = HairGroupDef {
                name: "cabelo_teste".into(),
                chains,
                params: PhysicsPreset::HairLong.hair_params().ok_or("preset de cabelo indisponível")?,
                xpbd: PhysicsPreset::HairLong.xpbd_config(),
                ..Default::default()
            };
            let h = world.add_hair(&def, &rest).map_err(|e| e.to_string())?;
            Some((h, n, cfg.points))
        } else {
            None
        };
        let skirt = if cfg.skirt {
            let waist = rig.bind[bone::HIPS] + Vec3::new(0.0, 0.03, 0.0);
            let mut def = skirt_proxy(waist, 0.17, 0.27, 0.42, cfg.columns, cfg.rings, bone::HIPS as u16);
            def.params = PhysicsPreset::Skirt.cloth_params().ok_or("preset de saia indisponível")?;
            def.params.backstop = f32::INFINITY;
            def.params.max_distance_tip = 0.6;
            def.xpbd = PhysicsPreset::Skirt.xpbd_config();
            let tris = def.triangles.clone();
            let cols = cfg.columns;
            let h = world.add_cloth(&def, &rest).map_err(|e| e.to_string())?;
            Some((h, tris, cols))
        } else {
            None
        };
        Ok(SecondarySim { world, hair, skirt, rig: rig.clone(), cfg, clock: None, resets: 0, steps: 0, last_report: StepReport::default() })
    }

    pub fn config(&self) -> SecondaryConfig {
        self.cfg
    }

    pub fn particle_count(&self) -> usize {
        self.world.particle_count()
    }

    /// Pose do clip no instante `t` (segundos), sobre o repouso.
    fn pose_at(clip: &AnimationClip, fps: f32, t: f64) -> PoseState {
        clip.evaluate(fps, (t * fps as f64) as f32).apply_to_pose(&PoseState::default())
    }

    /// Reinicia na pose de `t` (sem velocidades) e esquece o relógio.
    fn reset_at(&mut self, clip: &AnimationClip, fps: f32, t: f64) {
        let bones = bone_world(&self.rig, &Self::pose_at(clip, fps, t));
        self.world.reset(&bones);
        self.clock = Some(t);
        self.resets += 1;
    }

    /// Leva a simulação ao instante `t` (segundos de animação). `contiguous` = o tempo veio andando para a frente a
    /// partir do último `sync` (reprodução); falso = salto, e a simulação reinicia com pré-aquecimento.
    pub fn sync(&mut self, clip: &AnimationClip, fps: f32, t: f32, contiguous: bool) {
        let t = t.max(0.0) as f64;
        let step = SIM_DT as f64;
        let continues = contiguous && self.clock.is_some_and(|c| t >= c - 1e-9 && t - c < 0.5);
        if !continues {
            let t0 = (t - self.cfg.pre_roll_s as f64).max(0.0);
            self.reset_at(clip, fps, t0);
        }
        let mut c = self.clock.unwrap_or(t);
        let mut guard = 0;
        while c + step <= t + 1e-9 && guard < 4000 {
            c += step;
            let bones = bone_world(&self.rig, &Self::pose_at(clip, fps, c));
            self.last_report = self.world.step(SIM_DT, &bones);
            self.steps += 1;
            guard += 1;
        }
        self.clock = Some(c);
    }

    /// Fios de cabelo (cada um uma polilinha, no espaço do personagem).
    pub fn hair_chains(&self) -> Vec<Vec<Vec3>> {
        let Some((h, n, len)) = self.hair else { return Vec::new() };
        let Some(p) = self.world.hair_positions(h) else { return Vec::new() };
        (0..n).map(|i| p[i * len..((i + 1) * len).min(p.len())].to_vec()).collect()
    }

    /// Arestas da saia (pares de pontos), sem repetição.
    pub fn skirt_edges(&self) -> Vec<(Vec3, Vec3)> {
        let Some((h, tris, _)) = &self.skirt else { return Vec::new() };
        let Some(p) = self.world.cloth_positions(*h) else { return Vec::new() };
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for t in tris {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (a.min(b), a.max(b));
                if seen.insert(key) {
                    if let (Some(pa), Some(pb)) = (p.get(a as usize), p.get(b as usize)) {
                        out.push((*pa, *pb));
                    }
                }
            }
        }
        out
    }

    /// Hash do estado de todas as partículas (teste de determinismo).
    pub fn state_hash(&self) -> u64 {
        let mut h = 0u64;
        if let Some((hh, _, _)) = self.hair {
            if let Some(p) = self.world.hair_positions(hh) {
                h ^= state_hash(p);
            }
        }
        if let Some((ch, _, _)) = &self.skirt {
            if let Some(p) = self.world.cloth_positions(*ch) {
                h = h.rotate_left(17) ^ state_hash(p);
            }
        }
        h
    }

    /// Todas as posições (cabelo depois saia) — o "dado" da simulação.
    pub fn positions(&self) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        if let Some((h, _, _)) = self.hair {
            out.extend(self.world.hair_positions(h).unwrap_or(&[]).iter().map(|v| v.to_array()));
        }
        if let Some((h, _, _)) = &self.skirt {
            out.extend(self.world.cloth_positions(*h).unwrap_or(&[]).iter().map(|v| v.to_array()));
        }
        out
    }
}

/// Simulação gravada quadro a quadro (para outras ferramentas ou o futuro caminho A do renderer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsBake {
    pub format: String,
    pub fps: f32,
    pub first_frame: i32,
    pub hair_chains: usize,
    pub hair_points: usize,
    pub frames: Vec<Vec<[f32; 3]>>,
}

/// Grava a simulação de `first..=last` (quadros): pré-aquece no primeiro quadro e segue contíguo.
pub fn bake(sim: &mut SecondarySim, clip: &AnimationClip, fps: f32, first: i32, last: i32) -> PhysicsBake {
    let mut frames = Vec::new();
    for f in first..=last.max(first) {
        sim.sync(clip, fps, f as f32 / fps, f > first);
        frames.push(sim.positions());
    }
    let (chains, points) = sim.hair.map(|(_, n, l)| (n, l)).unwrap_or((0, 0));
    PhysicsBake { format: "anigo.physics_bake".into(), fps, first_frame: first, hair_chains: chains, hair_points: points, frames }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{Channel, ChannelTarget, Key, KeyValue};
    use anigo_core::ids::AnimationClipId;
    use glam::Quat;

    /// Clip em que o quadril gira e anda (o cabelo e a saia balançam).
    fn dancing_clip() -> AnimationClip {
        let mut c = AnimationClip::new(AnimationClipId::from_slug("dance"), "dance");
        c.range = Some([0, 48]);
        let yaw = |d: f32| KeyValue::Quat(Quat::from_rotation_y(d.to_radians()).to_array());
        c.channels = vec![
            Channel { target: ChannelTarget::Hips, keys: vec![Key::new(0, KeyValue::Vec3([0.0; 3])), Key::new(24, KeyValue::Vec3([0.4, 0.0, 0.0])), Key::new(48, KeyValue::Vec3([0.0; 3]))] },
            Channel { target: ChannelTarget::bone("Hips"), keys: vec![Key::new(0, yaw(0.0)), Key::new(12, yaw(60.0)), Key::new(24, yaw(-60.0)), Key::new(48, yaw(0.0))] },
            Channel { target: ChannelTarget::bone("Head"), keys: vec![Key::new(0, yaw(0.0)), Key::new(20, yaw(50.0)), Key::new(48, yaw(0.0))] },
        ];
        anigo_core::animation::canonicalize_clip(&mut c);
        c
    }

    fn sim() -> SecondarySim {
        SecondarySim::new(&PoseRig::default(), SecondaryConfig::default()).unwrap()
    }

    #[test]
    fn the_sim_builds_with_hair_chains_and_a_skirt() {
        let s = sim();
        assert!(s.particle_count() > 100, "{}", s.particle_count());
        assert!(s.hair_chains().len() >= 8);
        assert!(s.hair_chains().iter().all(|c| c.len() == 8));
        assert!(!s.skirt_edges().is_empty());
        let off = SecondarySim::new(&PoseRig::default(), SecondaryConfig { hair: false, skirt: false, ..Default::default() }).unwrap();
        assert!(off.hair_chains().is_empty() && off.skirt_edges().is_empty());
    }

    #[test]
    fn the_same_time_gives_the_same_state_whatever_the_path_to_it() {
        let clip = dancing_clip();
        let fps = 24.0;
        // A: salta direto para t=1,25 s
        let mut a = sim();
        a.sync(&clip, fps, 1.25, false);
        // B: passa por outros lugares antes (scrub bagunçado) e depois salta para o mesmo t
        let mut b = sim();
        for t in [0.2, 1.9, 0.7, 1.4] {
            b.sync(&clip, fps, t, false);
        }
        b.sync(&clip, fps, 1.25, false);
        assert_eq!(a.state_hash(), b.state_hash(), "saltar para o mesmo instante é determinístico");
        assert!(a.resets == 1 && b.resets == 5);
        // C: outra instância (outro processo, em tese): idem
        let mut c = sim();
        c.sync(&clip, fps, 1.25, false);
        assert_eq!(a.state_hash(), c.state_hash());
        assert_eq!(a.positions(), c.positions());
    }

    #[test]
    fn playback_steps_are_independent_of_the_ui_frame_rate() {
        let clip = dancing_clip();
        let fps = 24.0;
        // 144 Hz de UI e 30 Hz de UI chegam ao mesmo tempo de animação com o mesmo número de passos fixos
        let mut fast = sim();
        let mut slow = sim();
        fast.sync(&clip, fps, 0.0, false);
        slow.sync(&clip, fps, 0.0, false);
        let mut t = 0.0f32;
        while t < 1.0 {
            t += 1.0 / 144.0;
            fast.sync(&clip, fps, t, true);
        }
        let mut t = 0.0f32;
        while t < 1.0 {
            t += 1.0 / 30.0;
            slow.sync(&clip, fps, t, true);
        }
        assert!((fast.steps as i64 - slow.steps as i64).abs() <= 2, "{} vs {}", fast.steps, slow.steps);
        assert_eq!(fast.resets, 1, "reprodução contínua não reinicia");
        // as posições finais são próximas (a diferença é só o fim do último passo parcial)
        let (pa, pb) = (fast.positions(), slow.positions());
        let worst = pa.iter().zip(&pb).map(|(x, y)| Vec3::from_array(*x).distance(Vec3::from_array(*y))).fold(0.0f32, f32::max);
        assert!(worst < 0.08, "as duas execuções divergem {worst} m");
    }

    #[test]
    fn hair_actually_moves_with_the_body_and_stays_finite_and_attached() {
        let clip = dancing_clip();
        let mut s = sim();
        s.sync(&clip, 24.0, 0.0, false);
        let still = s.positions();
        s.sync(&clip, 24.0, 0.6, false);
        let moved = s.positions();
        assert!(still.iter().zip(&moved).any(|(a, b)| Vec3::from_array(*a).distance(Vec3::from_array(*b)) > 0.03), "a dança move as partículas");
        assert!(moved.iter().all(|p| p.iter().all(|c| c.is_finite())));
        // as raízes dos fios acompanham a cabeça: ficam perto do osso Head
        let rig = PoseRig::default();
        let pose = SecondarySim::pose_at(&clip, 24.0, s.clock.unwrap());
        let head = rig.joint_positions(&pose)[bone::HEAD];
        for chain in s.hair_chains() {
            assert!(chain[0].distance(head) < 0.45, "raiz longe da cabeça: {}", chain[0].distance(head));
        }
        // sem estiramento absurdo: o comprimento total do fio não passa de 1,3x o de repouso
        let rest = SecondarySim::new(&rig, SecondaryConfig::default()).unwrap();
        for (a, b) in rest.hair_chains().iter().zip(s.hair_chains().iter()) {
            let len = |c: &Vec<Vec3>| c.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
            assert!(len(b) < len(a) * 1.3, "{} vs {}", len(b), len(a));
        }
    }

    #[test]
    fn warm_up_removes_the_dead_start_after_a_jump() {
        let clip = dancing_clip();
        // sem pré-aquecimento o cabelo nasce parado em t=1 s; com ele já traz a inércia do movimento
        let mut warm = SecondarySim::new(&PoseRig::default(), SecondaryConfig { pre_roll_s: 0.5, ..Default::default() }).unwrap();
        let mut cold = SecondarySim::new(&PoseRig::default(), SecondaryConfig { pre_roll_s: 0.0, ..Default::default() }).unwrap();
        warm.sync(&clip, 24.0, 1.0, false);
        cold.sync(&clip, 24.0, 1.0, false);
        assert_ne!(warm.state_hash(), cold.state_hash());
        // o relatório do último passo não registra erros
        assert_eq!(warm.last_report.nan_restores, 0);
        assert_eq!(warm.last_report.invalid_bones, 0);
    }

    #[test]
    fn bake_records_every_frame_and_is_deterministic() {
        let clip = dancing_clip();
        let mut a = sim();
        let b1 = bake(&mut a, &clip, 24.0, 0, 11);
        let mut b = sim();
        let b2 = bake(&mut b, &clip, 24.0, 0, 11);
        assert_eq!(b1, b2);
        assert_eq!(b1.frames.len(), 12);
        assert!(b1.hair_chains >= 8 && b1.hair_points == 8);
        assert_eq!(b1.frames[0].len(), a.positions().len());
        let json = serde_json::to_string(&b1).unwrap();
        assert_eq!(serde_json::from_str::<PhysicsBake>(&json).unwrap(), b1);
    }
}
