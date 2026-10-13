//! `PhysicsWorld`: a interface única. Adiciona corpos (cabelo, acessórios, roupa, rigs VRM) e
//! colliders, avança com passo fixo + acumulador, e expõe posições/rotações resultantes.
//!
//! Garantias: `step` nunca entra em pânico; `dt` inválido é ignorado (e reportado); `dt` enorme é
//! limitado; pose com NaN mantém a última pose boa; saltos grandes das raízes viram teleporte
//! (sem arrastar a roupa); `reset` reinicia tudo na pose atual.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::body::build_body_colliders;
use crate::cloth::{ClothProxyDef, ClothSim};
use crate::collider::{Collider, ColliderSet};
use crate::error::{PhysicsError, PhysicsResult};
use crate::hair::{HairGroupDef, HairSim};
use crate::spring_bone::{SpringBoneDef, SpringBoneSystem};
use crate::stepper::FixedStepper;

pub const SETUP_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldConfig {
    /// Frequência do passo fixo (Hz).
    pub step_hz: f32,
    /// `dt` máximo por chamada (s).
    pub max_dt: f32,
    /// Máximo de passos fixos por chamada.
    pub max_steps: u32,
    /// Vento (aceleração, m/s^2) somado ao de cada grupo.
    pub wind: Vec3,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { step_hz: 60.0, max_dt: 0.1, max_steps: 8, wind: Vec3::ZERO }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HairHandle(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessoryHandle(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClothHandle(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpringHandle(pub usize);

/// Relatório de um `step`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepReport {
    /// Passos fixos executados.
    pub steps: u32,
    pub paused: bool,
    /// `dt` inválido (NaN/infinito/<= 0), ignorado.
    pub invalid_dt: bool,
    /// `dt` ou backlog limitado.
    pub clamped: bool,
    /// Matrizes de osso não finitas na entrada (a última pose boa foi mantida).
    pub invalid_bones: u32,
    /// Sistemas que restauraram o estado por NaN neste passo.
    pub nan_restores: u32,
    /// Teleportes detectados neste passo.
    pub teleports: u32,
}

/// Opções para gerar os colliders do corpo (24 ossos) a partir da pose de repouso.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BodyColliderSetup {
    pub radius_scale: f32,
    pub friction: f32,
}

impl Default for BodyColliderSetup {
    fn default() -> Self {
        Self { radius_scale: 1.0, friction: 0.2 }
    }
}

/// Tudo o que o projeto grava (serde): configs de cabelo, acessórios, roupa, rigs e colliders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysicsSetup {
    pub format_version: u32,
    pub config: WorldConfig,
    pub body_colliders: Option<BodyColliderSetup>,
    pub colliders: Vec<Collider>,
    pub hair: Vec<HairGroupDef>,
    pub accessories: Vec<HairGroupDef>,
    pub cloth: Vec<ClothProxyDef>,
    pub spring_rigs: Vec<SpringBoneDef>,
}

impl Default for PhysicsSetup {
    fn default() -> Self {
        Self {
            format_version: SETUP_FORMAT_VERSION,
            config: WorldConfig::default(),
            body_colliders: None,
            colliders: Vec::new(),
            hair: Vec::new(),
            accessories: Vec::new(),
            cloth: Vec::new(),
            spring_rigs: Vec::new(),
        }
    }
}

impl PhysicsSetup {
    pub fn to_json(&self) -> PhysicsResult<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn to_json_pretty(&self) -> PhysicsResult<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> PhysicsResult<Self> {
        let setup: PhysicsSetup = serde_json::from_str(json)?;
        if setup.format_version > SETUP_FORMAT_VERSION {
            return Err(PhysicsError::BadConfig(format!(
                "format_version {} é mais novo que o suportado ({SETUP_FORMAT_VERSION})",
                setup.format_version
            )));
        }
        Ok(setup)
    }
}

#[derive(Debug, Clone)]
pub struct PhysicsWorld {
    config: WorldConfig,
    stepper: FixedStepper,
    colliders: ColliderSet,
    hair: Vec<HairSim>,
    accessories: Vec<HairSim>,
    cloth: Vec<ClothSim>,
    springs: Vec<SpringBoneSystem>,
    bones: Vec<Mat4>,
    paused: bool,
    wind: Vec3,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(WorldConfig::default())
    }
}

impl PhysicsWorld {
    pub fn new(config: WorldConfig) -> Self {
        let mut stepper = FixedStepper::new(config.step_hz);
        if config.max_dt.is_finite() && config.max_dt > 0.0 {
            stepper.max_dt = config.max_dt;
        }
        stepper.max_steps = config.max_steps.clamp(1, 32);
        Self {
            wind: config.wind,
            config,
            stepper,
            colliders: ColliderSet::new(),
            hair: Vec::new(),
            accessories: Vec::new(),
            cloth: Vec::new(),
            springs: Vec::new(),
            bones: Vec::new(),
            paused: false,
        }
    }

    /// Constrói o mundo a partir de um `PhysicsSetup` (projeto) e da pose de repouso dos ossos.
    pub fn from_setup(setup: &PhysicsSetup, rest_bones: &[Mat4]) -> PhysicsResult<Self> {
        let mut world = Self::new(setup.config);
        world.ingest_bones(rest_bones, &mut StepReport::default());
        if let Some(body) = setup.body_colliders {
            world.add_body_colliders(rest_bones, body.radius_scale, body.friction)?;
        }
        for c in &setup.colliders {
            world.add_collider(*c).ok_or_else(|| PhysicsError::BadConfig("collider inválido no setup".into()))?;
        }
        for h in &setup.hair {
            world.add_hair(h, rest_bones)?;
        }
        for h in &setup.accessories {
            world.add_accessory(h, rest_bones)?;
        }
        for c in &setup.cloth {
            world.add_cloth(c, rest_bones)?;
        }
        for s in &setup.spring_rigs {
            world.add_spring_rig(s)?;
        }
        Ok(world)
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    // ----- construção ------------------------------------------------------------------------

    /// Adiciona um collider; `None` se a forma é inválida.
    pub fn add_collider(&mut self, collider: Collider) -> Option<usize> {
        self.colliders.add(collider)
    }

    /// Cápsulas do corpo nos 24 ossos canônicos. Devolve os índices dos colliders criados.
    pub fn add_body_colliders(&mut self, rest_bones: &[Mat4], radius_scale: f32, friction: f32) -> PhysicsResult<Vec<usize>> {
        let built = build_body_colliders(rest_bones, radius_scale, friction)?;
        Ok(built.into_iter().filter_map(|(_, c)| self.colliders.add(c)).collect())
    }

    pub fn colliders(&self) -> &ColliderSet {
        &self.colliders
    }

    pub fn colliders_mut(&mut self) -> &mut ColliderSet {
        &mut self.colliders
    }

    pub fn add_hair(&mut self, def: &HairGroupDef, bone_world: &[Mat4]) -> PhysicsResult<HairHandle> {
        let mut sim = HairSim::new(def, bone_world)?;
        sim.system_mut().config.wind += self.wind;
        self.hair.push(sim);
        Ok(HairHandle(self.hair.len() - 1))
    }

    pub fn add_accessory(&mut self, def: &HairGroupDef, bone_world: &[Mat4]) -> PhysicsResult<AccessoryHandle> {
        let mut sim = HairSim::new(def, bone_world)?;
        sim.system_mut().config.wind += self.wind;
        self.accessories.push(sim);
        Ok(AccessoryHandle(self.accessories.len() - 1))
    }

    pub fn add_cloth(&mut self, def: &ClothProxyDef, rest_bones: &[Mat4]) -> PhysicsResult<ClothHandle> {
        let mut sim = ClothSim::new(def, rest_bones)?;
        sim.system_mut().config.wind += self.wind;
        self.cloth.push(sim);
        Ok(ClothHandle(self.cloth.len() - 1))
    }

    pub fn add_spring_rig(&mut self, def: &SpringBoneDef) -> PhysicsResult<SpringHandle> {
        self.springs.push(SpringBoneSystem::new(def)?);
        Ok(SpringHandle(self.springs.len() - 1))
    }

    // ----- leitura ---------------------------------------------------------------------------

    pub fn hair(&self, h: HairHandle) -> Option<&HairSim> {
        self.hair.get(h.0)
    }

    pub fn hair_mut(&mut self, h: HairHandle) -> Option<&mut HairSim> {
        self.hair.get_mut(h.0)
    }

    pub fn accessory(&self, h: AccessoryHandle) -> Option<&HairSim> {
        self.accessories.get(h.0)
    }

    pub fn accessory_mut(&mut self, h: AccessoryHandle) -> Option<&mut HairSim> {
        self.accessories.get_mut(h.0)
    }

    pub fn cloth(&self, h: ClothHandle) -> Option<&ClothSim> {
        self.cloth.get(h.0)
    }

    pub fn cloth_mut(&mut self, h: ClothHandle) -> Option<&mut ClothSim> {
        self.cloth.get_mut(h.0)
    }

    pub fn spring_rig(&self, h: SpringHandle) -> Option<&SpringBoneSystem> {
        self.springs.get(h.0)
    }

    pub fn spring_rig_mut(&mut self, h: SpringHandle) -> Option<&mut SpringBoneSystem> {
        self.springs.get_mut(h.0)
    }

    /// Posições (mundo) das partículas de um grupo de cabelo.
    pub fn hair_positions(&self, h: HairHandle) -> Option<&[Vec3]> {
        self.hair(h).map(|s| s.positions())
    }

    /// Rotações swing por segmento de um fio (ver [`HairSim::segment_swings`]).
    pub fn hair_segment_swings(&self, h: HairHandle, chain: usize) -> Option<Vec<Quat>> {
        self.hair(h)?.segment_swings(chain)
    }

    pub fn cloth_positions(&self, h: ClothHandle) -> Option<&[Vec3]> {
        self.cloth(h).map(|s| s.positions())
    }

    /// Fração (0..1) do próximo passo fixo já acumulada, para interpolar a apresentação.
    pub fn alpha(&self) -> f32 {
        self.stepper.alpha()
    }

    // ----- controle --------------------------------------------------------------------------

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Vento uniforme (aceleração m/s^2) para todos os grupos futuros e existentes.
    pub fn set_wind(&mut self, wind: Vec3) {
        if !wind.is_finite() {
            return;
        }
        let delta = wind - self.wind;
        self.wind = wind;
        for s in self.hair.iter_mut().chain(self.accessories.iter_mut()) {
            s.system_mut().config.wind += delta;
        }
        for s in self.cloth.iter_mut() {
            s.system_mut().config.wind += delta;
        }
    }

    fn ingest_bones(&mut self, bone_world: &[Mat4], report: &mut StepReport) {
        if bone_world.len() > self.bones.len() {
            self.bones.resize(bone_world.len(), Mat4::IDENTITY);
        }
        for (i, m) in bone_world.iter().enumerate() {
            if m.is_finite() {
                self.bones[i] = *m;
            } else {
                report.invalid_bones += 1;
            }
        }
    }

    fn apply_pose(&mut self) {
        self.colliders.update(&self.bones);
        for s in self.hair.iter_mut().chain(self.accessories.iter_mut()) {
            s.set_pose(&self.bones);
        }
        for s in self.cloth.iter_mut() {
            s.set_pose(&self.bones);
        }
    }

    /// Reinicia tudo na pose dada (teleporte/troca de pose/carregar projeto): sem velocidades, sem
    /// "puxão". Seguro a qualquer momento.
    pub fn reset(&mut self, bone_world: &[Mat4]) {
        let mut report = StepReport::default();
        self.ingest_bones(bone_world, &mut report);
        self.apply_pose();
        for s in self.hair.iter_mut().chain(self.accessories.iter_mut()) {
            s.reset();
        }
        for s in self.cloth.iter_mut() {
            s.reset();
        }
        for s in self.springs.iter_mut() {
            s.reset();
        }
        self.stepper.reset();
    }

    fn counters(&self) -> (u32, u32) {
        let mut nan = 0;
        let mut tele = 0;
        for s in self.hair.iter().chain(self.accessories.iter()) {
            nan += s.system().stats().nan_restores;
            tele += s.system().stats().teleports;
        }
        for s in &self.cloth {
            nan += s.system().stats().nan_restores;
            tele += s.system().stats().teleports;
        }
        for s in &self.springs {
            nan += s.stats().nan_resets;
        }
        (nan, tele)
    }

    /// Avança `dt` segundos com a pose dos ossos no mundo (`bone_world`, mesma indexação dos
    /// `root_bone`/`bone` das definições). Nunca entra em pânico.
    pub fn step(&mut self, dt: f32, bone_world: &[Mat4]) -> StepReport {
        let mut report = StepReport::default();
        if self.paused {
            report.paused = true;
            return report;
        }
        let plan = self.stepper.advance(dt);
        report.invalid_dt = plan.invalid_dt;
        report.clamped = plan.clamped;
        if plan.invalid_dt {
            return report;
        }
        self.ingest_bones(bone_world, &mut report);
        if plan.steps == 0 {
            return report;
        }
        let (nan0, tele0) = self.counters();
        self.apply_pose();
        let h = self.stepper.step;
        for _ in 0..plan.steps {
            for s in self.hair.iter_mut().chain(self.accessories.iter_mut()) {
                s.step(h, &self.colliders);
            }
            for s in self.cloth.iter_mut() {
                s.step(h, &self.colliders);
            }
            for s in self.springs.iter_mut() {
                s.step(h);
            }
        }
        report.steps = plan.steps;
        let (nan1, tele1) = self.counters();
        report.nan_restores = nan1 - nan0;
        report.teleports = tele1 - tele0;
        report
    }

    /// Número total de partículas XPBD (cabelo + acessórios + roupa).
    pub fn particle_count(&self) -> usize {
        self.hair.iter().chain(self.accessories.iter()).map(|s| s.particle_count()).sum::<usize>()
            + self.cloth.iter().map(|s| s.vertex_count()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloth::{cape_proxy, skirt_proxy};
    use crate::collider::ColliderShape;
    use crate::hair::{demo_strands, HairChainDef};
    use crate::preset::PhysicsPreset;
    use crate::spring_bone::{SpringChainDef, SpringJointParams};
    use crate::skeleton::SkeletonNode;
    use crate::math::state_hash;

    const DT: f32 = 1.0 / 60.0;

    fn bones_at(head: Vec3, hips: Vec3, yaw: f32) -> Vec<Mat4> {
        vec![
            Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), hips), // 0 hips
            Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), head), // 1 head
        ]
    }

    fn demo_setup() -> PhysicsSetup {
        let hair = HairGroupDef {
            name: "long".into(),
            chains: demo_strands(6, 7, 0.4, 0.09, 0.0).into_iter().map(|g| HairChainDef { guide: g, root_bone: Some(1) }).collect(),
            params: PhysicsPreset::HairLong.hair_params().unwrap(),
            xpbd: PhysicsPreset::HairLong.xpbd_config(),
            ..Default::default()
        };
        let accessory = HairGroupDef {
            name: "ribbon".into(),
            chains: vec![HairChainDef {
                guide: crate::hair::ChainGuide::from_points((0..5).map(|k| Vec3::new(0.1, 0.1 - 0.05 * k as f32, -0.08)).collect()),
                root_bone: Some(1),
            }],
            params: PhysicsPreset::Accessory.hair_params().unwrap(),
            xpbd: PhysicsPreset::Accessory.xpbd_config(),
            ..Default::default()
        };
        let mut skirt = skirt_proxy(Vec3::new(0.0, 0.9, 0.0), 0.19, 0.30, 0.45, 16, 5, 0);
        skirt.params = PhysicsPreset::Skirt.cloth_params().unwrap();
        let spring = SpringBoneDef {
            nodes: vec![
                SkeletonNode::new("root", None, Vec3::new(0.0, 1.5, 0.0)),
                SkeletonNode::new("a", Some(0), Vec3::ZERO),
                SkeletonNode::new("b", Some(1), Vec3::new(0.0, -0.2, 0.0)),
            ],
            chains: vec![SpringChainDef {
                name: "tail".into(),
                joints: vec![SpringJointParams { node: 1, stiffness: 0.5, gravity_power: 1.0, ..Default::default() }, SpringJointParams::new(2)],
                ..Default::default()
            }],
            ..Default::default()
        };
        PhysicsSetup {
            hair: vec![hair],
            accessories: vec![accessory],
            cloth: vec![skirt],
            spring_rigs: vec![spring],
            colliders: vec![Collider::on_bone(1, ColliderShape::Sphere { center: Vec3::ZERO, radius: 0.1 })],
            ..Default::default()
        }
    }

    fn rest() -> Vec<Mat4> {
        bones_at(Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 0.9, 0.0), 0.0)
    }

    #[test]
    fn setup_serde_ida_e_volta_e_constroi_o_mundo() {
        let setup = demo_setup();
        let json = setup.to_json().unwrap();
        let back = PhysicsSetup::from_json(&json).unwrap();
        assert_eq!(back, setup, "serde preserva tudo (inclusive limites infinitos como null)");
        let world = PhysicsWorld::from_setup(&back, &rest()).unwrap();
        assert!(world.particle_count() > 100);
        // JSON de versão futura é rejeitado; JSON mínimo usa defaults
        let mut future = serde_json::to_value(&setup).unwrap();
        future["format_version"] = serde_json::json!(999);
        assert!(PhysicsSetup::from_json(&future.to_string()).is_err());
        let minimal = PhysicsSetup::from_json("{}").unwrap();
        assert_eq!(minimal, PhysicsSetup::default());
        assert!(PhysicsSetup::from_json("{ nao e json").is_err());
    }

    #[test]
    fn setup_com_capa_serializa_backstop_infinito() {
        let mut setup = PhysicsSetup::default();
        let mut cape = cape_proxy(Vec3::new(-0.2, 1.4, -0.1), Vec3::new(0.2, 1.4, -0.1), 0.8, 5, 4, 0);
        cape.params = PhysicsPreset::Cape.cloth_params().unwrap();
        setup.cloth.push(cape);
        let json = setup.to_json().unwrap();
        assert!(json.contains("\"backstop\":null"));
        let back = PhysicsSetup::from_json(&json).unwrap();
        assert!(back.cloth[0].params.backstop.is_infinite());
        assert_eq!(back, setup);
    }

    #[test]
    fn mundo_completo_roda_estavel_e_segue_o_personagem() {
        let mut world = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
        let hair = HairHandle(0);
        let skirt = ClothHandle(0);
        let mut worst_hair = 0.0f32;
        let mut worst_skirt = 0.0f32;
        for f in 0..600 {
            let t = f as f32 * DT;
            let bones = bones_at(Vec3::new((t * 2.0).sin() * 0.4, 1.5, 0.0), Vec3::new((t * 2.0).sin() * 0.4, 0.9, 0.0), (t * 3.0).sin());
            let r = world.step(DT, &bones);
            assert_eq!(r.steps, 1);
            assert_eq!(r.nan_restores, 0);
            worst_hair = worst_hair.max(world.hair(hair).unwrap().max_stretch_ratio());
            worst_skirt = worst_skirt.max(world.cloth(skirt).unwrap().max_edge_stretch());
        }
        eprintln!("mundo completo 10 s: estiramento max cabelo {worst_hair:.4}, saia {worst_skirt:.4}, particulas {}", world.particle_count());
        assert!(worst_hair < 0.10);
        assert!(worst_skirt < 0.08);
        assert!(world.hair_positions(hair).unwrap().iter().all(|p| p.is_finite()));
        assert!(world.hair_segment_swings(hair, 0).is_some());
        assert!(world.hair_segment_swings(hair, 99).is_none());
        assert!(world.hair(HairHandle(5)).is_none());
        assert!(world.spring_rig(SpringHandle(0)).unwrap().tail_world(0).unwrap().is_finite());
    }

    #[test]
    fn independe_do_frame_rate() {
        // 2 s de simulacao a 30, 60 e 144 fps: o numero de passos fixos e o resultado sao equivalentes.
        let mut finals = Vec::new();
        for fps in [30.0f32, 60.0, 144.0] {
            let mut world = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
            let mut steps = 0;
            for f in 0..(fps * 2.0) as usize {
                let t = f as f32 / fps;
                let bones = bones_at(Vec3::new((t * 2.0).sin() * 0.3, 1.5, 0.0), Vec3::new(0.0, 0.9, 0.0), 0.0);
                steps += world.step(1.0 / fps, &bones).steps;
            }
            assert!((steps as i64 - 120).abs() <= 1, "{fps} fps -> {steps} passos");
            finals.push(*world.hair_positions(HairHandle(0)).unwrap().last().unwrap());
        }
        eprintln!("ponta do cabelo apos 2 s: {finals:?}");
        // a pose é entregue em frequências diferentes, então tolera-se alguns cm (não é bit a bit)
        assert!((finals[0] - finals[1]).length() < 0.08, "{finals:?}");
        assert!((finals[2] - finals[1]).length() < 0.05, "{finals:?}");
    }

    #[test]
    fn determinismo_mesmo_input_mesmo_hash() {
        let run = || {
            let mut world = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
            for f in 0..240 {
                let t = f as f32 * DT;
                let bones = bones_at(Vec3::new((t * 3.0).sin() * 0.2, 1.5, 0.0), Vec3::new(0.0, 0.9, 0.0), t.sin());
                world.step(DT, &bones);
            }
            (
                state_hash(world.hair_positions(HairHandle(0)).unwrap()),
                state_hash(world.cloth_positions(ClothHandle(0)).unwrap()),
                state_hash(world.accessory(AccessoryHandle(0)).unwrap().positions()),
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn pausa_dt_invalido_pose_nan_e_reset() {
        let mut world = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
        for _ in 0..30 {
            world.step(DT, &rest());
        }
        let before = world.hair_positions(HairHandle(0)).unwrap().to_vec();
        // pausa
        world.set_paused(true);
        let r = world.step(DT, &bones_at(Vec3::new(5.0, 1.5, 0.0), Vec3::new(5.0, 0.9, 0.0), 0.0));
        assert!(r.paused && r.steps == 0);
        assert_eq!(world.hair_positions(HairHandle(0)).unwrap(), &before[..]);
        world.set_paused(false);
        // dt invalido
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let r = world.step(dt, &rest());
            assert!(r.invalid_dt && r.steps == 0);
        }
        assert_eq!(world.hair_positions(HairHandle(0)).unwrap(), &before[..]);
        // pose com NaN: mantem a ultima boa e conta
        let mut bad = rest();
        bad[1] = Mat4::from_translation(Vec3::splat(f32::NAN));
        let r = world.step(DT, &bad);
        assert_eq!(r.invalid_bones, 1);
        assert!(world.hair_positions(HairHandle(0)).unwrap().iter().all(|p| p.is_finite()));
        // dt enorme e limitado
        let r = world.step(1000.0, &rest());
        assert!(r.clamped && r.steps <= 8);
        // reset teleporta sem explodir
        let far = bones_at(Vec3::new(900.0, 1.5, 900.0), Vec3::new(900.0, 0.9, 900.0), 2.0);
        world.reset(&far);
        let root = world.hair_positions(HairHandle(0)).unwrap()[0];
        assert!(root.is_finite() && (root.x - 900.0).abs() < 0.2);
        for _ in 0..120 {
            world.step(DT, &far);
        }
        assert!(world.hair(HairHandle(0)).unwrap().max_stretch_ratio() < 0.1);
        assert!(world.cloth(ClothHandle(0)).unwrap().max_edge_stretch() < 0.1);
    }

    #[test]
    fn salto_grande_da_pose_vira_teleporte_automatico() {
        let mut world = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
        for _ in 0..30 {
            world.step(DT, &rest());
        }
        let far = bones_at(Vec3::new(50.0, 1.5, 0.0), Vec3::new(50.0, 0.9, 0.0), 0.0);
        let r = world.step(DT, &far);
        assert!(r.teleports >= 1, "{r:?}");
        for _ in 0..60 {
            world.step(DT, &far);
        }
        assert!(world.hair(HairHandle(0)).unwrap().max_stretch_ratio() < 0.1);
    }

    #[test]
    fn construcao_invalida_nao_causa_panic() {
        let mut world = PhysicsWorld::default();
        assert!(world.add_collider(Collider::fixed(ColliderShape::Sphere { center: Vec3::splat(f32::NAN), radius: 1.0 })).is_none());
        assert!(world.add_hair(&HairGroupDef { chains: vec![HairChainDef { guide: Default::default(), root_bone: None }], ..Default::default() }, &[]).is_err());
        assert!(world.add_cloth(&ClothProxyDef::default(), &[]).is_err());
        assert!(world.add_spring_rig(&SpringBoneDef { chains: vec![SpringChainDef { joints: vec![SpringJointParams::new(3)], ..Default::default() }], ..Default::default() }).is_err());
        assert!(world.add_body_colliders(&[Mat4::IDENTITY; 3], 1.0, 0.2).is_err());
        assert!(world.add_body_colliders(&[Mat4::IDENTITY; 24], f32::NAN, 0.2).is_err());
        world.set_wind(Vec3::splat(f32::NAN));
        // mundo vazio roda
        for _ in 0..10 {
            world.step(DT, &[]);
        }
        world.reset(&[]);
        let r = world.step(DT, &[Mat4::IDENTITY]);
        assert_eq!(r.steps, 1);
    }

    #[test]
    fn vento_desloca_cabelo_e_roupa() {
        let mut calm = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
        let mut windy = PhysicsWorld::from_setup(&demo_setup(), &rest()).unwrap();
        windy.set_wind(Vec3::new(6.0, 0.0, 0.0));
        for _ in 0..240 {
            calm.step(DT, &rest());
            windy.step(DT, &rest());
        }
        let a = calm.hair_positions(HairHandle(0)).unwrap();
        let b = windy.hair_positions(HairHandle(0)).unwrap();
        let mean_dx = a.iter().zip(b).map(|(p, q)| q.x - p.x).sum::<f32>() / a.len() as f32;
        eprintln!("deslocamento medio em X do cabelo com vento: {mean_dx:.4} m");
        assert!(mean_dx > 0.02);
        windy.set_wind(Vec3::ZERO);
        for _ in 0..600 {
            windy.step(DT, &rest());
        }
        let c = windy.hair_positions(HairHandle(0)).unwrap();
        let back = a.iter().zip(c).map(|(p, q)| (*p - *q).length()).fold(0.0, f32::max);
        assert!(back < 0.03, "sem vento volta ao repouso: {back}");
    }
}
