//! Integração com o esqueleto canônico de 24 ossos (`anigo-core::BondSyncManager`): anima a
//! cabeça, o quadril e as pernas e verifica, SEM olho humano, que o cabelo e a saia seguem o
//! personagem sem atravessar os colliders do corpo e sem esticar além do limite.
//!
//! Cada cenário roda também sem colliders (controle): se o controle não penetra, o teste não
//! provaria nada e falha.

use anigo_core::bone_sync::BondSyncManager;
use anigo_physics::body::{bone, canonical_body_specs};
use anigo_physics::math::{closest_point_on_segment, state_hash};
use anigo_physics::{
    skirt_proxy, ChainGuide, ClothHandle, ColliderShape, HairChainDef, HairGroupDef, HairHandle, PhysicsPreset, PhysicsWorld,
    WorldConfig,
};
use glam::{Mat4, Quat, Vec3};

const DT: f32 = 1.0 / 60.0;

struct Rig {
    mgr: BondSyncManager,
}

impl Rig {
    fn new() -> Self {
        Self { mgr: BondSyncManager::create_canonical_humanoid() }
    }

    fn world(&self) -> Vec<Mat4> {
        self.mgr.compute_world_transforms()
    }

    fn rest(&self) -> Vec<Mat4> {
        BondSyncManager::create_canonical_humanoid().compute_world_transforms()
    }

    /// Animação de teste: quadril gira, cabeça vira e acena, pernas chutam alternadamente.
    fn animate(&mut self, t: f32) {
        let set = |m: &mut BondSyncManager, i: usize, q: Quat| m.joints[i].current_local_rotation = q;
        set(&mut self.mgr, bone::HIPS, Quat::from_rotation_y((t * 2.0).sin() * 0.7));
        set(&mut self.mgr, bone::NECK, Quat::from_euler(glam::EulerRot::YXZ, (t * 2.7).sin() * 0.6, (t * 2.1).cos() * 0.25, 0.0));
        set(&mut self.mgr, bone::HEAD, Quat::from_euler(glam::EulerRot::YXZ, (t * 3.1).sin() * 0.5, (t * 1.9).sin() * 0.3, (t * 2.3).cos() * 0.1));
        // -X gira o eixo -Y das pernas para +Z (frente)
        set(&mut self.mgr, bone::LEFT_THIGH, Quat::from_rotation_x(-((t * 3.0).sin().max(0.0)) * 0.9));
        set(&mut self.mgr, bone::RIGHT_THIGH, Quat::from_rotation_x(-((t * 3.0 + std::f32::consts::PI).sin().max(0.0)) * 0.9));
    }
}

/// Fios saindo de um anel na cabeça (espaço local da cabeça), caindo para as costas e para a frente
/// dos ombros; os lados são omitidos porque o T-pose canônico tem os braços ali.
fn long_hair(strands: usize, points: usize) -> Vec<ChainGuide> {
    (0..strands)
        .filter_map(|i| {
            let a = std::f32::consts::TAU * i as f32 / strands as f32;
            if a.sin().abs() > 0.8 {
                return None;
            }
            let dir = Vec3::new(a.sin(), 0.0, -a.cos()); // anel em torno da cabeça
            let pts = (0..points)
                .map(|k| {
                    let t = k as f32 / (points - 1) as f32;
                    let radius = 0.09 + 0.10 * t.powf(0.6);
                    Vec3::new(0.0, 0.12 - 0.62 * t, 0.0) + dir * radius
                })
                .collect();
            Some(ChainGuide::from_points(pts))
        })
        .collect()
}

fn hair_def() -> HairGroupDef {
    HairGroupDef {
        name: "long".into(),
        chains: long_hair(32, 12).into_iter().map(|g| HairChainDef { guide: g, root_bone: Some(bone::HEAD) }).collect(),
        params: PhysicsPreset::HairLong.hair_params().unwrap(),
        xpbd: PhysicsPreset::HairLong.xpbd_config(),
        ..Default::default()
    }
}

fn skirt_def() -> anigo_physics::ClothProxyDef {
    let hips_y = 0.96; // cintura natural
    let mut def = skirt_proxy(Vec3::new(0.0, hips_y, 0.0), 0.17, 0.27, 0.46, 24, 8, bone::HIPS as u16);
    def.params = PhysicsPreset::Skirt.cloth_params().unwrap();
    def.params.backstop = f32::INFINITY; // sem backstop: so os colliders seguram a saia
    def.params.max_distance_tip = 0.6;
    def.xpbd = PhysicsPreset::Skirt.xpbd_config();
    def
}

/// Penetração (m, >= 0) de um ponto de raio `r` nas cápsulas do corpo na pose atual.
fn body_penetration(p: Vec3, r: f32, rest: &[Mat4], now: &[Mat4]) -> f32 {
    let mut worst = 0.0f32;
    for spec in canonical_body_specs() {
        // resolve a capsula na pose atual a partir das juntas de repouso e da matriz do osso
        let inv_rest = rest[spec.bone].inverse();
        let end = |e: anigo_physics::CapsuleEnd| match e {
            anigo_physics::CapsuleEnd::Local(v) => v,
            anigo_physics::CapsuleEnd::Joint(j) => inv_rest.transform_point3(rest[j].w_axis.truncate()),
        };
        let (a, b) = (now[spec.bone].transform_point3(end(spec.from)), now[spec.bone].transform_point3(end(spec.to)));
        let (c, _) = closest_point_on_segment(p, a, b);
        worst = worst.max(spec.radius + r - (p - c).length());
    }
    worst
}

#[test]
fn os_24_ossos_batem_com_o_esqueleto_canonico() {
    let mgr = BondSyncManager::create_canonical_humanoid();
    assert_eq!(mgr.joints.len(), bone::COUNT);
    for (i, name) in bone::NAMES.iter().enumerate() {
        assert_eq!(mgr.joints[i].name, *name, "osso {i}");
    }
    let world = mgr.compute_world_transforms();
    let specs = canonical_body_specs();
    assert!(specs.len() >= 18);
    // cada capsula do corpo no repouso tem extremidades dentro da altura do personagem
    let colliders = anigo_physics::build_body_colliders(&world, 1.0, 0.2).unwrap();
    for (name, c) in &colliders {
        let ColliderShape::Capsule { a, b, radius } = c.shape else { panic!("{name}") };
        let m = world[c.bone.unwrap()];
        for p in [m.transform_point3(a), m.transform_point3(b)] {
            assert!(p.y > -0.2 && p.y < 1.8 && p.x.abs() < 1.0, "{name}: {p:?}");
        }
        assert!(radius > 0.02 && radius < 0.2);
    }
}

struct Outcome {
    worst_hair_pen: f32,
    worst_skirt_pen: f32,
    worst_hair_stretch: f32,
    worst_skirt_stretch: f32,
    hash: (u64, u64),
}

fn run_scenario(with_colliders: bool, frames: usize) -> Outcome {
    let mut rig = Rig::new();
    let rest = rig.rest();
    let mut world = PhysicsWorld::new(WorldConfig::default());
    if with_colliders {
        world.add_body_colliders(&rest, 1.0, 0.2).unwrap();
    }
    let hair = world.add_hair(&hair_def(), &rest).unwrap();
    let skirt_def = skirt_def();
    let skirt = world.add_cloth(&skirt_def, &rest).unwrap();
    let hair_radius = PhysicsPreset::HairLong.hair_params().unwrap().particle_radius;
    let skirt_radius = skirt_def.params.particle_radius;

    let mut out = Outcome { worst_hair_pen: 0.0, worst_skirt_pen: 0.0, worst_hair_stretch: 0.0, worst_skirt_stretch: 0.0, hash: (0, 0) };
    for f in 0..frames {
        rig.animate(f as f32 * DT);
        let now = rig.world();
        let report = world.step(DT, &now);
        assert_eq!(report.steps, 1);
        assert_eq!(report.nan_restores, 0);
        if f > 30 {
            // (30 quadros de aquecimento: o solver assenta eventuais sobreposicoes iniciais)
            out.worst_hair_stretch = out.worst_hair_stretch.max(world.hair(hair).unwrap().max_stretch_ratio());
            out.worst_skirt_stretch = out.worst_skirt_stretch.max(world.cloth(skirt).unwrap().max_edge_stretch());
            for (k, p) in world.hair_positions(hair).unwrap().iter().enumerate() {
                if k % 12 == 0 {
                    continue; // raiz presa à cabeça
                }
                out.worst_hair_pen = out.worst_hair_pen.max(body_penetration(*p, hair_radius, &rest, &now));
            }
            for (k, p) in world.cloth_positions(skirt).unwrap().iter().enumerate() {
                if skirt_def.fixed[k] {
                    continue;
                }
                out.worst_skirt_pen = out.worst_skirt_pen.max(body_penetration(*p, skirt_radius, &rest, &now));
            }
        }
    }
    out.hash = (state_hash(world.hair_positions(hair).unwrap()), state_hash(world.cloth_positions(skirt).unwrap()));
    out
}

#[test]
fn cabelo_e_saia_seguem_o_esqueleto_canonico_sem_atravessar_nem_esticar() {
    let frames = 720; // 12 s
    let with = run_scenario(true, frames);
    let without = run_scenario(false, frames);
    eprintln!(
        "COM colisores: penetracao cabelo {:.5} m, saia {:.5} m; estiramento cabelo {:.4}, saia {:.4}",
        with.worst_hair_pen, with.worst_skirt_pen, with.worst_hair_stretch, with.worst_skirt_stretch
    );
    eprintln!("SEM colisores (controle): penetracao cabelo {:.5} m, saia {:.5} m", without.worst_hair_pen, without.worst_skirt_pen);
    // o controle precisa provar que a animacao empurra cabelo e saia contra o corpo
    assert!(without.worst_hair_pen > 0.02, "controle do cabelo nao penetrou: {}", without.worst_hair_pen);
    assert!(without.worst_skirt_pen > 0.02, "controle da saia nao penetrou: {}", without.worst_skirt_pen);
    // com colisores: tolerancia de poucos mm
    assert!(with.worst_hair_pen < 0.006, "cabelo atravessou o corpo: {}", with.worst_hair_pen);
    assert!(with.worst_skirt_pen < 0.006, "saia atravessou o corpo: {}", with.worst_skirt_pen);
    // sem esticar
    assert!(with.worst_hair_stretch < 0.06, "cabelo esticou {}", with.worst_hair_stretch);
    // saia justa (hem 0.27 m) empurrada por uma coxa a ~50 graus: ~7,5% no pior quadro
    assert!(with.worst_skirt_stretch < 0.10, "saia esticou {}", with.worst_skirt_stretch);
}

#[test]
fn simulacao_do_personagem_e_deterministica() {
    let a = run_scenario(true, 240);
    let b = run_scenario(true, 240);
    assert_eq!(a.hash, b.hash);
}

#[test]
fn teleporte_do_personagem_nao_arrasta_cabelo_nem_saia() {
    let mut rig = Rig::new();
    let rest = rig.rest();
    let mut world = PhysicsWorld::default();
    world.add_body_colliders(&rest, 1.0, 0.2).unwrap();
    let hair = world.add_hair(&hair_def(), &rest).unwrap();
    let skirt = world.add_cloth(&skirt_def(), &rest).unwrap();
    for f in 0..120 {
        rig.animate(f as f32 * DT);
        world.step(DT, &rig.world());
    }
    // move o personagem inteiro 40 m (raiz): salto imediato
    rig.mgr.joints[bone::ROOT].current_local_position = Vec3::new(40.0, 0.0, -25.0);
    let far = rig.world();
    let report = world.step(DT, &far);
    assert!(report.teleports >= 1, "{report:?}");
    for f in 0..180 {
        rig.animate(f as f32 * DT);
        world.step(DT, &rig.world());
    }
    assert!(world.hair(hair).unwrap().max_stretch_ratio() < 0.08);
    assert!(world.cloth(skirt).unwrap().max_edge_stretch() < 0.08);
    let waist = world.cloth_positions(skirt).unwrap()[0];
    assert!((waist.x - 40.0).abs() < 0.5 && (waist.z + 25.0).abs() < 0.5, "{waist:?}");
}

#[test]
fn escala_do_corpo_muda_o_raio_dos_colliders() {
    let rig = Rig::new();
    let rest = rig.rest();
    let thin = anigo_physics::build_body_colliders(&rest, 0.8, 0.2).unwrap();
    let wide = anigo_physics::build_body_colliders(&rest, 1.4, 0.2).unwrap();
    for ((_, a), (_, b)) in thin.iter().zip(&wide) {
        let (ColliderShape::Capsule { radius: ra, .. }, ColliderShape::Capsule { radius: rb, .. }) = (a.shape, b.shape) else { panic!() };
        assert!((rb / ra - 1.4 / 0.8).abs() < 1e-4);
    }
    assert!(anigo_physics::build_body_colliders(&rest, -1.0, 0.2).is_err());
    assert!(anigo_physics::build_body_colliders(&rest[..5], 1.0, 0.2).is_err());
}

#[test]
fn mundo_e_handles_expoem_posicoes_por_vertice() {
    let rig = Rig::new();
    let rest = rig.rest();
    let mut world = PhysicsWorld::default();
    let hair = world.add_hair(&hair_def(), &rest).unwrap();
    assert_eq!(hair, HairHandle(0));
    let skirt = world.add_cloth(&skirt_def(), &rest).unwrap();
    assert_eq!(skirt, ClothHandle(0));
    assert_eq!(world.hair_positions(hair).unwrap().len(), long_hair(32, 12).len() * 12);
    assert_eq!(world.cloth_positions(skirt).unwrap().len(), 24 * 9);
    // swings por segmento (rotacoes por osso do fio)
    assert_eq!(world.hair_segment_swings(hair, 0).unwrap().len(), 11);
}
