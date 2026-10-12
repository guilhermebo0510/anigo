//! Exemplo executável: simula cabelo longo + saia no esqueleto canônico de 24 ossos, animando
//! quadril, cabeça e pernas, e imprime métricas objetivas (sem GPU, sem imagem).
//!
//!   cargo run --release -p anigo-physics --example hair_skirt            # métricas da cena
//!   cargo run --release -p anigo-physics --example hair_skirt -- bench   # partículas/ms
//!   cargo run --release -p anigo-physics --example hair_skirt -- json    # PhysicsSetup em JSON

use std::time::Instant;

use anigo_core::bone_sync::BondSyncManager;
use anigo_physics::body::{bone, canonical_body_specs};
use anigo_physics::math::{closest_point_on_segment, state_hash};
use anigo_physics::{
    cape_proxy, skirt_proxy, ChainGuide, ClothHandle, HairChainDef, HairGroupDef, HairHandle, PhysicsPreset, PhysicsSetup, PhysicsWorld,
    SpringBoneDef, SpringChainDef, SpringJointParams, SkeletonNode, SpringBoneSystem, WorldConfig,
};
use glam::{Mat4, Quat, Vec3};

const DT: f32 = 1.0 / 60.0;

fn long_hair(strands: usize, points: usize) -> Vec<ChainGuide> {
    (0..strands)
        .filter_map(|i| {
            let a = std::f32::consts::TAU * i as f32 / strands as f32;
            if a.sin().abs() > 0.8 {
                return None;
            }
            let dir = Vec3::new(a.sin(), 0.0, -a.cos());
            Some(ChainGuide::from_points(
                (0..points)
                    .map(|k| {
                        let t = k as f32 / (points - 1) as f32;
                        Vec3::new(0.0, 0.12 - 0.62 * t, 0.0) + dir * (0.09 + 0.10 * t.powf(0.6))
                    })
                    .collect(),
            ))
        })
        .collect()
}

fn hair_def(strands: usize, points: usize) -> HairGroupDef {
    HairGroupDef {
        name: "hair_long".into(),
        chains: long_hair(strands, points).into_iter().map(|g| HairChainDef { guide: g, root_bone: Some(bone::HEAD) }).collect(),
        params: PhysicsPreset::HairLong.hair_params().expect("preset"),
        xpbd: PhysicsPreset::HairLong.xpbd_config(),
        ..Default::default()
    }
}

fn skirt_def(columns: usize, rings: usize) -> anigo_physics::ClothProxyDef {
    let mut def = skirt_proxy(Vec3::new(0.0, 0.96, 0.0), 0.17, 0.27, 0.46, columns, rings, bone::HIPS as u16);
    def.params = PhysicsPreset::Skirt.cloth_params().expect("preset");
    def.params.backstop = f32::INFINITY;
    def.params.max_distance_tip = 0.6;
    def.xpbd = PhysicsPreset::Skirt.xpbd_config();
    def
}

fn animate(mgr: &mut BondSyncManager, t: f32) {
    let set = |m: &mut BondSyncManager, i: usize, q: Quat| m.joints[i].current_local_rotation = q;
    set(mgr, bone::HIPS, Quat::from_rotation_y((t * 2.0).sin() * 0.7));
    set(mgr, bone::NECK, Quat::from_euler(glam::EulerRot::YXZ, (t * 2.7).sin() * 0.6, (t * 2.1).cos() * 0.25, 0.0));
    set(mgr, bone::HEAD, Quat::from_euler(glam::EulerRot::YXZ, (t * 3.1).sin() * 0.5, (t * 1.9).sin() * 0.3, (t * 2.3).cos() * 0.1));
    set(mgr, bone::LEFT_THIGH, Quat::from_rotation_x(-((t * 3.0).sin().max(0.0)) * 0.9));
    set(mgr, bone::RIGHT_THIGH, Quat::from_rotation_x(-((t * 3.0 + std::f32::consts::PI).sin().max(0.0)) * 0.9));
}

fn body_penetration(p: Vec3, r: f32, rest: &[Mat4], now: &[Mat4]) -> f32 {
    let mut worst = 0.0f32;
    for spec in canonical_body_specs() {
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

fn scene(seconds: f32) {
    let mut mgr = BondSyncManager::create_canonical_humanoid();
    let rest = mgr.compute_world_transforms();
    let mut world = PhysicsWorld::new(WorldConfig::default());
    world.add_body_colliders(&rest, 1.0, 0.2).expect("colliders do corpo");
    let hair_d = hair_def(32, 12);
    let skirt_d = skirt_def(24, 8);
    let hair: HairHandle = world.add_hair(&hair_d, &rest).expect("cabelo");
    let skirt: ClothHandle = world.add_cloth(&skirt_d, &rest).expect("saia");
    let hair_r = PhysicsPreset::HairLong.hair_params().expect("preset").particle_radius;
    let skirt_r = skirt_d.params.particle_radius;

    println!("== cena: cabelo longo ({} fios x 12) + saia (24 x 8) no esqueleto canonico de 24 ossos ==", hair_d.chains.len());
    println!("particulas XPBD: {}", world.particle_count());
    println!("{:>6} {:>12} {:>12} {:>14} {:>14} {:>10}", "t(s)", "cabelo_str%", "saia_str%", "cabelo_pen(mm)", "saia_pen(mm)", "ms/passo");

    let frames = (seconds / DT) as usize;
    let (mut worst_hs, mut worst_ss, mut worst_hp, mut worst_sp) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut total_ms = 0.0f64;
    let mut window_ms = 0.0f64;
    for f in 0..frames {
        animate(&mut mgr, f as f32 * DT);
        let now = mgr.compute_world_transforms();
        let t0 = Instant::now();
        world.step(DT, &now);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        total_ms += ms;
        window_ms += ms;
        if f > 30 {
            worst_hs = worst_hs.max(world.hair(hair).expect("h").max_stretch_ratio());
            worst_ss = worst_ss.max(world.cloth(skirt).expect("s").max_edge_stretch());
            for (k, p) in world.hair_positions(hair).expect("h").iter().enumerate() {
                if k % 12 != 0 {
                    worst_hp = worst_hp.max(body_penetration(*p, hair_r, &rest, &now));
                }
            }
            for (k, p) in world.cloth_positions(skirt).expect("s").iter().enumerate() {
                if !skirt_d.fixed[k] {
                    worst_sp = worst_sp.max(body_penetration(*p, skirt_r, &rest, &now));
                }
            }
        }
        if (f + 1) % 120 == 0 {
            println!(
                "{:>6.1} {:>12.2} {:>12.2} {:>14.2} {:>14.2} {:>10.3}",
                (f + 1) as f32 * DT,
                worst_hs * 100.0,
                worst_ss * 100.0,
                worst_hp * 1000.0,
                worst_sp * 1000.0,
                window_ms / 120.0
            );
            window_ms = 0.0;
        }
    }
    println!(
        "resumo: estiramento max cabelo {:.2}% / saia {:.2}%; penetracao max cabelo {:.2} mm / saia {:.2} mm; media {:.3} ms por passo",
        worst_hs * 100.0,
        worst_ss * 100.0,
        worst_hp * 1000.0,
        worst_sp * 1000.0,
        total_ms / frames as f64
    );
    println!(
        "hash do estado final (determinismo): cabelo {:016x} saia {:016x}",
        state_hash(world.hair_positions(hair).expect("h")),
        state_hash(world.cloth_positions(skirt).expect("s"))
    );
}

fn bench() {
    println!("== benchmark (build release recomendado) ==");
    let mgr = BondSyncManager::create_canonical_humanoid();
    let rest = mgr.compute_world_transforms();
    let frames = 600usize;

    let case = |name: &str, strands: usize, points: usize, columns: usize, rings: usize, bodies: bool| {
        let mut world = PhysicsWorld::new(WorldConfig::default());
        if bodies {
            world.add_body_colliders(&rest, 1.0, 0.2).expect("colliders");
        }
        if strands > 0 {
            world.add_hair(&hair_def(strands, points), &rest).expect("cabelo");
        }
        if columns > 0 {
            world.add_cloth(&skirt_def(columns, rings), &rest).expect("saia");
        }
        let particles = world.particle_count();
        let mut m = BondSyncManager::create_canonical_humanoid();
        let t0 = Instant::now();
        for f in 0..frames {
            animate(&mut m, f as f32 * DT);
            world.step(DT, &m.compute_world_transforms());
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("{name:<44} {particles:>6} particulas  {ms:>8.3} ms/passo  {:>9.0} particulas/ms", particles as f64 / ms);
    };
    case("cabelo curto (~50 fios x 6), com colisores", 90, 6, 0, 0, true);
    case("cabelo longo (~50 fios x 12), com colisores", 90, 12, 0, 0, true);
    case("cabelo longo (~50 fios x 12), sem colisores", 90, 12, 0, 0, false);
    case("saia 24 x 8, com colisores", 0, 0, 24, 8, true);
    case("saia 48 x 16 (alta resolucao), com colisores", 0, 0, 48, 16, true);
    case("cena completa (cabelo ~50x12 + saia 24x8)", 90, 12, 24, 8, true);

    // spring bone VRM: 200 cadeias de 6 juntas
    let mut nodes = vec![SkeletonNode::new("root", None, Vec3::new(0.0, 1.5, 0.0))];
    let mut chains = Vec::new();
    for c in 0..200 {
        let first = nodes.len();
        for k in 0..7 {
            let parent = if k == 0 { 0 } else { nodes.len() - 1 };
            nodes.push(SkeletonNode::new(format!("c{c}_{k}"), Some(parent), Vec3::new(0.0, -0.05, 0.0)));
        }
        chains.push(SpringChainDef {
            joints: (first..first + 7).map(|n| SpringJointParams { node: n, stiffness: 0.5, gravity_power: 1.0, ..Default::default() }).collect(),
            ..Default::default()
        });
    }
    let mut spring = SpringBoneSystem::new(&SpringBoneDef { nodes, chains, ..Default::default() }).expect("rig");
    let pairs = spring.pair_count();
    let t0 = Instant::now();
    for f in 0..frames {
        spring.skeleton_mut().set_local_translation(0, Vec3::new((f as f32 * 0.05).sin() * 0.2, 1.5, 0.0));
        spring.step(DT);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("{:<44} {pairs:>6} juntas       {ms:>8.3} ms/passo  {:>9.0} juntas/ms", "spring bone VRM (200 cadeias x 6 ossos)", pairs as f64 / ms);

    // capa
    let cape = cape_proxy(Vec3::new(-0.2, 1.4, -0.1), Vec3::new(0.2, 1.4, -0.1), 0.8, 16, 16, 0);
    let mut world = PhysicsWorld::default();
    world.add_cloth(&cape, &[Mat4::IDENTITY]).expect("capa");
    let particles = world.particle_count();
    let t0 = Instant::now();
    for f in 0..frames {
        world.step(DT, &[Mat4::from_translation(Vec3::new((f as f32 * 0.03).sin(), 0.0, 0.0))]);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("{:<44} {particles:>6} particulas  {ms:>8.3} ms/passo  {:>9.0} particulas/ms", "capa 16 x 16", particles as f64 / ms);
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("bench") => bench(),
        Some("json") => {
            let mgr = BondSyncManager::create_canonical_humanoid();
            let _ = mgr;
            let setup = PhysicsSetup {
                body_colliders: Some(Default::default()),
                hair: vec![hair_def(32, 12)],
                cloth: vec![skirt_def(24, 8), {
                    let mut cape = cape_proxy(Vec3::new(-0.2, 1.4, -0.1), Vec3::new(0.2, 1.4, -0.1), 0.8, 8, 8, bone::UPPER_CHEST as u16);
                    cape.params = PhysicsPreset::Cape.cloth_params().expect("preset");
                    cape
                }],
                ..Default::default()
            };
            println!("{}", setup.to_json_pretty().expect("json"));
        }
        _ => scene(12.0),
    }
}
