use super::*;
use crate::pose::{BONE_NAMES};

fn canonical_skeleton(rig: &PoseRig) -> SrcSkeleton {
    canonical_source_skeleton(rig)
}

fn frame_of(pose: &PoseState, rig: &PoseRig) -> SrcFrame {
    source_frame_of_pose(pose, rig)
}

fn sample_pose() -> PoseState {
    let mut p = PoseState::default();
    p.set_rotation(bone::L_SHOULDER, Quat::from_euler(glam::EulerRot::XYZ, -0.6, 0.3, 0.9));
    p.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-1.1));
    p.set_rotation(bone::R_SHOULDER, Quat::from_rotation_z(-0.5) * Quat::from_rotation_y(0.4));
    p.set_rotation(bone::R_THIGH, Quat::from_rotation_x(-0.7));
    p.set_rotation(bone::R_KNEE, Quat::from_rotation_x(0.9));
    p.set_rotation(bone::SPINE, Quat::from_rotation_y(0.25));
    p.set_rotation(bone::CHEST, Quat::from_rotation_x(0.12) * Quat::from_rotation_y(0.2));
    p.set_rotation(bone::NECK, Quat::from_rotation_z(0.15));
    p.set_rotation(bone::HEAD, Quat::from_rotation_y(-0.4));
    p.hips_translation = [0.05, -0.12, 0.2];
    p
}

fn angle(a: Quat, b: Quat) -> f32 {
    a.angle_between(b).to_degrees()
}

#[test]
fn same_rest_pose_reproduces_the_source_rotations_exactly() {
    let rig = PoseRig::default();
    let skel = canonical_skeleton(&rig);
    let map = BoneMap::from_names(&skel);
    assert_eq!(map.mapped_count(), BONE_COUNT - 1, "todos menos a raiz");
    let rt = Retargeter::new(&skel, &map, &rig).unwrap();
    assert_eq!(rt.frame_fix(), Quat::IDENTITY);
    assert!((rt.hips_scale() - 1.0).abs() < 1e-4);
    let pose = sample_pose();
    let out = rt.pose(&frame_of(&pose, &rig));
    for b in [bone::SPINE, bone::CHEST, bone::NECK, bone::HEAD, bone::L_SHOULDER, bone::L_ELBOW, bone::R_SHOULDER, bone::R_THIGH, bone::R_KNEE] {
        let a = angle(out.rotation(b), pose.rotation(b));
        assert!(a < 0.3, "{} difere {a}°", BONE_NAMES[b]);
    }
    for i in 0..3 {
        assert!((out.hips_translation[i] - pose.hips_translation[i]).abs() < 1e-4);
    }
    // a pose reconstruída leva as juntas aos mesmos lugares
    let (ja, jb) = (rig.joint_positions(&pose), rig.joint_positions(&out));
    for b in 1..BONE_COUNT {
        assert!(ja[b].distance(jb[b]) < 0.01, "{} fora do lugar: {}", BONE_NAMES[b], ja[b].distance(jb[b]));
    }
}

#[test]
fn rest_frame_gives_the_rest_pose() {
    let rig = PoseRig::default();
    let skel = canonical_skeleton(&rig);
    let rt = Retargeter::new(&skel, &BoneMap::from_names(&skel), &rig).unwrap();
    let out = rt.pose(&frame_of(&PoseState::default(), &rig));
    assert!(out.is_rest(), "{out:?}");
}

fn t_pose_skeleton(rig: &PoseRig) -> SrcSkeleton {
    canonical_source_skeleton(&t_pose_rig(rig))
}

#[test]
fn a_t_pose_source_is_retargeted_by_direction_not_by_blind_fk() {
    let rig = PoseRig::default();
    let skel = t_pose_skeleton(&rig);
    let rt = Retargeter::new(&skel, &BoneMap::from_names(&skel), &rig).unwrap();
    // 1) quadro de repouso da fonte = braços na horizontal -> o destino também levanta os braços na horizontal
    let rest = rt.pose(&SrcFrame { rot: skel.joints.iter().map(|j| j.rest_r).collect(), pos: BTreeMap::new() });
    let j = rig.joint_positions(&rest);
    let (sh, wr) = (j[bone::L_SHOULDER], j[bone::L_WRIST]);
    let dir = (wr - sh).normalize();
    assert!(dir.x > 0.97 && dir.y.abs() < 0.2, "braço E na horizontal: {dir:?}");
    // 2) a fonte baixa os braços 90° (rotação em Z): o destino volta a ter os braços pendurados (locais ~ identidade)
    let mut down = SrcFrame { rot: skel.joints.iter().map(|j| j.rest_r).collect(), pos: BTreeMap::new() };
    down.rot[bone::L_SHOULDER] = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    down.rot[bone::R_SHOULDER] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let hang = rt.pose(&down);
    for b in [bone::L_SHOULDER, bone::R_SHOULDER, bone::L_ELBOW, bone::R_ELBOW] {
        assert!(angle(hang.rotation(b), Quat::IDENTITY) < 4.0, "{} deveria ficar em repouso: {}°", BONE_NAMES[b], angle(hang.rotation(b), Quat::IDENTITY));
    }
    // 3) levantar o braço 90° acima da horizontal na fonte = braço para cima no destino
    let mut up = SrcFrame { rot: skel.joints.iter().map(|j| j.rest_r).collect(), pos: BTreeMap::new() };
    up.rot[bone::L_SHOULDER] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let pose = rt.pose(&up);
    let j = rig.joint_positions(&pose);
    let dir = (j[bone::L_WRIST] - j[bone::L_SHOULDER]).normalize();
    assert!(dir.y > 0.95, "braço para cima: {dir:?}");
}

#[test]
fn a_z_up_source_is_realigned_from_its_rest_pose() {
    let rig = PoseRig::default();
    let mut skel = canonical_skeleton(&rig);
    // eixos do Blender: Z para cima (gira -90° em torno de X: Y -> -Z… aqui Y(cima) -> Z)
    let z_up = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    skel.joints[bone::ROOT].rest_r = z_up;
    let map = BoneMap::from_names(&skel);
    let rt = Retargeter::new(&skel, &map, &rig).unwrap();
    assert!(angle(rt.frame_fix(), z_up.inverse()) < 1.0, "a correção desfaz a rotação do arquivo");
    let pose = sample_pose();
    let mut frame = frame_of(&pose, &rig);
    // a translação do quadril da fonte vive nos eixos da fonte
    frame.pos.insert(bone::HIPS, rig.offset(bone::HIPS) + Vec3::from_array(pose.hips_translation));
    frame.rot[bone::ROOT] = z_up; // a raiz do arquivo carrega a rotação de eixos também na animação
    let out = rt.pose(&frame);
    for b in [bone::L_SHOULDER, bone::L_ELBOW, bone::R_THIGH, bone::HEAD, bone::CHEST] {
        assert!(angle(out.rotation(b), pose.rotation(b)) < 0.5, "{}", BONE_NAMES[b]);
    }
    // hips: delta em eixos da fonte -> ANIGO
    let expect = z_up.inverse() * (z_up * Vec3::from_array(pose.hips_translation));
    assert!(Vec3::from_array(out.hips_translation).distance(expect) < 1e-3, "{:?} vs {expect:?} (scale {})", out.hips_translation, rt.hips_scale());
}

#[test]
fn hips_translation_scales_with_the_character_height() {
    let rig = PoseRig::default();
    let mut tall = rig.clone();
    for p in tall.bind.iter_mut() {
        *p *= 2.0;
    }
    tall.sole_y *= 2.0;
    let skel = canonical_skeleton(&tall);
    let rt = Retargeter::new(&skel, &BoneMap::from_names(&skel), &rig).unwrap();
    assert!((rt.hips_scale() - 0.5).abs() < 0.02, "{}", rt.hips_scale());
    let mut f = SrcFrame { rot: skel.joints.iter().map(|j| j.rest_r).collect(), pos: BTreeMap::new() };
    f.pos.insert(bone::HIPS, tall.offset(bone::HIPS) + Vec3::new(0.0, 0.0, 1.0));
    let out = rt.pose(&f);
    assert!((out.hips_translation[2] - 0.5).abs() < 0.02, "{:?}", out.hips_translation);
}

#[test]
fn missing_intermediate_bones_do_not_break_the_chain() {
    let rig = PoseRig::default();
    // fonte sem UpperChest nem Pelvis nem Clavículas
    let full = canonical_skeleton(&rig);
    let mut skel = SrcSkeleton::default();
    let mut remap: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, j) in full.joints.iter().enumerate() {
        if [bone::UPPER_CHEST, bone::PELVIS, bone::L_CLAVICLE, bone::R_CLAVICLE].contains(&i) {
            continue;
        }
        // pai ausente: sobe até um presente (e acumula o offset local para manter as posições de mundo)
        let mut parent = j.parent;
        let mut t = j.rest_t;
        while let Some(p) = parent {
            if remap.contains_key(&p) {
                break;
            }
            t += full.joints[p].rest_t;
            parent = full.joints[p].parent;
        }
        remap.insert(i, skel.joints.len());
        skel.joints.push(SrcJoint { name: j.name.clone(), parent: parent.and_then(|p| remap.get(&p).copied()), rest_t: t, rest_r: Quat::IDENTITY });
    }
    let map = BoneMap::from_names(&skel);
    assert!(map.0[bone::UPPER_CHEST].is_none());
    let rt = Retargeter::new(&skel, &map, &rig).unwrap();
    let mut frame = SrcFrame { rot: vec![Quat::IDENTITY; skel.joints.len()], pos: BTreeMap::new() };
    frame.rot[remap[&bone::CHEST]] = Quat::from_rotation_y(0.4);
    frame.rot[remap[&bone::L_SHOULDER]] = Quat::from_rotation_x(-0.8);
    let out = rt.pose(&frame);
    assert!(out.validate().is_ok());
    assert!(angle(out.rotation(bone::CHEST), Quat::from_rotation_y(0.4)) < 1.0);
    assert!(angle(out.rotation(bone::NECK), Quat::IDENTITY) < 1.0, "o pescoço herda, não gira sozinho: o delta do peito já foi aplicado ao pai");
    assert!(rt.warnings.is_empty() || rt.warnings.iter().all(|w| w.contains("não encontrados")));
}

#[test]
fn a_skeleton_without_hips_is_refused() {
    let skel = SrcSkeleton { joints: vec![SrcJoint { name: "Foo".into(), parent: None, rest_t: Vec3::ZERO, rest_r: Quat::IDENTITY }] };
    assert!(Retargeter::new(&skel, &BoneMap::from_names(&skel), &PoseRig::default()).is_err());
    let bad = SrcSkeleton { joints: vec![SrcJoint { name: "a".into(), parent: Some(1), rest_t: Vec3::ZERO, rest_r: Quat::IDENTITY }, SrcJoint { name: "b".into(), parent: None, rest_t: Vec3::ZERO, rest_r: Quat::IDENTITY }] };
    assert!(bad.validate().is_err());
}

#[test]
fn names_from_many_conventions_map_to_the_same_bones() {
    let cases: &[(&str, Option<usize>)] = &[
        // VRM humanoid
        ("hips", Some(bone::HIPS)),
        ("spine", Some(bone::SPINE)),
        ("chest", Some(bone::CHEST)),
        ("upperChest", Some(bone::UPPER_CHEST)),
        ("neck", Some(bone::NECK)),
        ("head", Some(bone::HEAD)),
        ("leftShoulder", Some(bone::L_CLAVICLE)),
        ("leftUpperArm", Some(bone::L_SHOULDER)),
        ("leftLowerArm", Some(bone::L_ELBOW)),
        ("leftHand", Some(bone::L_WRIST)),
        ("rightUpperLeg", Some(bone::R_THIGH)),
        ("rightLowerLeg", Some(bone::R_KNEE)),
        ("rightFoot", Some(bone::R_ANKLE)),
        ("rightToes", Some(bone::R_TOES)),
        // Mixamo
        ("mixamorig:Hips", Some(bone::HIPS)),
        ("mixamorig:Spine1", Some(bone::CHEST)),
        ("mixamorig:Spine2", Some(bone::UPPER_CHEST)),
        ("mixamorig:LeftShoulder", Some(bone::L_CLAVICLE)),
        ("mixamorig:LeftArm", Some(bone::L_SHOULDER)),
        ("mixamorig:LeftForeArm", Some(bone::L_ELBOW)),
        ("mixamorig:LeftHand", Some(bone::L_WRIST)),
        ("mixamorig:RightUpLeg", Some(bone::R_THIGH)),
        ("mixamorig:RightLeg", Some(bone::R_KNEE)),
        ("mixamorig:LeftToeBase", Some(bone::L_TOES)),
        ("mixamorig:LeftHandThumb1", None),
        // VRoid
        ("J_Bip_C_Hips", Some(bone::HIPS)),
        ("J_Bip_L_UpperArm", Some(bone::L_SHOULDER)),
        ("J_Bip_R_LowerLeg", Some(bone::R_KNEE)),
        ("J_Bip_L_Shoulder", Some(bone::L_CLAVICLE)),
        // Rigify / Blender
        ("DEF-upper_arm.L", Some(bone::L_SHOULDER)),
        ("DEF-forearm.R", Some(bone::R_ELBOW)),
        ("DEF-thigh.L", Some(bone::L_THIGH)),
        ("DEF-shin.R", Some(bone::R_KNEE)),
        ("DEF-foot.L", Some(bone::L_ANKLE)),
        ("DEF-toe.R", Some(bone::R_TOES)),
        // 3ds Max Biped
        ("Bip01 Pelvis", Some(bone::HIPS)),
        ("Bip01 L Thigh", Some(bone::L_THIGH)),
        ("Bip01 R Calf", Some(bone::R_KNEE)),
        ("Bip01 L UpperArm", Some(bone::L_SHOULDER)),
        ("Bip01 R Forearm", Some(bone::R_ELBOW)),
        // CMU
        ("lhumerus", Some(bone::L_SHOULDER)),
        ("rfemur", Some(bone::R_THIGH)),
        ("ltibia", Some(bone::L_KNEE)),
        ("lclavicle", Some(bone::L_CLAVICLE)),
        ("lowerback", Some(bone::SPINE)),
        ("thorax", Some(bone::UPPER_CHEST)),
        // ruído
        ("Armature", None),
        ("Root", None),
        ("Cube.001", None),
        ("", None),
    ];
    for (name, expect) in cases {
        assert_eq!(canonical_bone_for_name(name), *expect, "{name:?}");
    }
}

#[test]
fn canonical_vocabulary_is_mapped_literally() {
    // "LeftShoulder" é o BRAÇO no vocabulário do ANIGO (e a clavícula no Mixamo/VRM): o arquivo canônico vence
    let rig = PoseRig::default();
    let skel = canonical_skeleton(&rig);
    let map = BoneMap::from_names(&skel);
    assert_eq!(map.0[bone::L_SHOULDER], Some(bone::L_SHOULDER));
    assert_eq!(map.0[bone::L_CLAVICLE], Some(bone::L_CLAVICLE));
    assert_eq!(map.0[bone::R_WRIST], Some(bone::R_WRIST));
}

#[test]
fn the_shallowest_joint_wins_when_two_candidates_match() {
    let skel = SrcSkeleton {
        joints: vec![
            SrcJoint { name: "Hips".into(), parent: None, rest_t: Vec3::ZERO, rest_r: Quat::IDENTITY },
            SrcJoint { name: "LeftHand".into(), parent: Some(0), rest_t: Vec3::X, rest_r: Quat::IDENTITY },
            SrcJoint { name: "LeftWrist".into(), parent: Some(1), rest_t: Vec3::X, rest_r: Quat::IDENTITY },
        ],
    };
    assert_eq!(BoneMap::from_names(&skel).0[bone::L_WRIST], Some(1));
}

// ---------------------------------------------------------------- poses -> clip

fn swinging(n: usize) -> Vec<PoseState> {
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let mut p = PoseState::default();
            p.set_rotation(bone::L_SHOULDER, Quat::from_rotation_x(-1.0 * t)); // linear no ângulo
            p.set_rotation(bone::HEAD, Quat::from_rotation_y((t * std::f32::consts::TAU).sin() * 0.8)); // curvo
            p.hips_translation = [0.0, 0.0, t];
            p
        })
        .collect()
}

#[test]
fn poses_become_a_clip_resampled_to_the_target_fps_with_redundant_keys_removed() {
    let poses = swinging(61); // 2 s a 30 fps
    let id = AnimationClipId::from_slug("imp");
    let full = poses_to_clip(&poses, 30.0, 24.0, id.clone(), "Imp", ClipBuild { reduce: false, ..Default::default() });
    assert_eq!(full.range, Some([0, 48]), "2 s a 24 fps");
    assert!((full.duration_seconds - 2.0).abs() < 1e-5);
    assert_eq!(full.channel(&ChannelTarget::bone("Head")).unwrap().keys.len(), 49);
    let reduced = poses_to_clip(&poses, 30.0, 24.0, id, "Imp", ClipBuild::default());
    let keys = |t: ChannelTarget| reduced.channel(&t).unwrap().keys.len();
    assert_eq!(keys(ChannelTarget::bone("LeftShoulder")), 2, "movimento linear = 2 chaves");
    assert_eq!(keys(ChannelTarget::Hips), 2);
    let head = keys(ChannelTarget::bone("Head"));
    assert!(head > 4 && head < 49, "curva senoidal mantém só as chaves necessárias: {head}");
    assert!(reduced.channel(&ChannelTarget::bone("Pelvis")).is_none(), "osso parado não vira canal");
    // o clip reduzido reproduz o original dentro da tolerância
    for f in 0..=48 {
        let a = full.evaluate(24.0, f as f32).apply_to_pose(&PoseState::default());
        let b = reduced.evaluate(24.0, f as f32).apply_to_pose(&PoseState::default());
        assert!(angle(a.rotation(bone::HEAD), b.rotation(bone::HEAD)) < 0.6, "quadro {f}");
        assert!(Vec3::from_array(a.hips_translation).distance(Vec3::from_array(b.hips_translation)) < 0.004);
    }
}

#[test]
fn reduce_keys_keeps_the_endpoints_and_the_extremes() {
    let key = |f: i32, v: f32| Key { frame: f, value: KeyValue::Vec3([v, 0.0, 0.0]), interp: Interp::Linear };
    let keys = vec![key(0, 0.0), key(1, 0.25), key(2, 0.5), key(3, 1.0), key(4, 0.5), key(5, 0.0)];
    let r = reduce_keys(&keys, 1.0, 0.01);
    assert_eq!(r.first().unwrap().frame, 0);
    assert_eq!(r.last().unwrap().frame, 5);
    assert!(r.iter().any(|k| k.frame == 3), "o pico fica");
    assert!(r.len() <= 4);
    assert_eq!(reduce_keys(&keys[..2], 1.0, 0.01).len(), 2);
    assert!(poses_to_clip(&[], 30.0, 24.0, AnimationClipId::from_slug("e"), "e", ClipBuild::default()).channels.is_empty());
}
