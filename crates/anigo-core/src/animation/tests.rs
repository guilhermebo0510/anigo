use super::*;
use crate::command::{Command, CommandHistory};
use crate::pose::{bone, PoseOp};
use crate::project::ProjectState;
use glam::Quat;

fn id(s: &str) -> AnimationClipId {
    AnimationClipId::from_slug(s)
}

fn rx(deg: f32) -> [f32; 4] {
    Quat::from_rotation_x(deg.to_radians()).to_array()
}

fn ry(deg: f32) -> [f32; 4] {
    Quat::from_rotation_y(deg.to_radians()).to_array()
}

fn q_angle(a: [f32; 4], b: [f32; 4]) -> f32 {
    let d = Quat::from_xyzw(a[0], a[1], a[2], a[3]).dot(Quat::from_xyzw(b[0], b[1], b[2], b[3])).abs().clamp(0.0, 1.0);
    2.0 * d.acos().to_degrees()
}

fn chan(target: ChannelTarget, keys: Vec<Key>) -> Channel {
    Channel { target, keys }
}

fn scalar_channel(frames_values: &[(i32, f32)], interp: Interp) -> Channel {
    chan(
        ChannelTarget::Camera { prop: CameraProp::Fov },
        frames_values.iter().map(|(f, v)| Key { frame: *f, value: KeyValue::Scalar(*v), interp }).collect(),
    )
}

/// Projeto com um clip "walk" de 0..24 e algumas chaves de osso.
fn project_with_clip() -> ProjectState {
    let mut p = ProjectState::default();
    let mut clip = AnimationClip::new(id("walk"), "Walk");
    clip.range = Some([0, 24]);
    clip.channels = vec![
        chan(
            ChannelTarget::bone("LeftElbow"),
            vec![Key::new(0, KeyValue::Quat(rx(0.0))), Key::new(12, KeyValue::Quat(rx(-60.0))).with_interp(Interp::Ease), Key::new(24, KeyValue::Quat(rx(0.0)))],
        ),
        chan(ChannelTarget::Hips, vec![Key::new(0, KeyValue::Vec3([0.0, 0.0, 0.0])), Key::new(24, KeyValue::Vec3([0.0, 0.0, 1.0]))]),
    ];
    canonicalize_clip(&mut clip);
    p.animation.clips.insert(clip.clip_id.clone(), clip);
    p.animation.active_clip = Some(id("walk"));
    p
}

// ---------------------------------------------------------------- interpolação

#[test]
fn slerp_takes_the_short_path_without_flipping() {
    // q e -q são a mesma rotação: o meio entre (identidade) e (-identidade) tem que ser a identidade, sem passar pelo outro lado
    let a = [0.0, 0.0, 0.0, 1.0];
    let b = [0.0, 0.0, 0.0, -1.0];
    let m = slerp(a, b, 0.5);
    assert!(q_angle(m, a) < 1e-3, "{m:?}");
    // 170° com sinais trocados: o meio é 85°, não -95°
    let q170 = ry(170.0);
    let neg = [-q170[0], -q170[1], -q170[2], -q170[3]];
    let m = slerp([0.0, 0.0, 0.0, 1.0], neg, 0.5);
    assert!((q_angle(m, [0.0, 0.0, 0.0, 1.0]) - 85.0).abs() < 0.05, "{}", q_angle(m, [0.0, 0.0, 0.0, 1.0]));
    // continuidade: nenhum salto grande entre amostras consecutivas
    let mut prev = slerp(a, q170, 0.0);
    for i in 1..=100 {
        let cur = slerp(a, q170, i as f32 / 100.0);
        assert!(q_angle(prev, cur) < 2.0, "salto em {i}");
        prev = cur;
    }
}

#[test]
fn slerp_is_exact_at_the_ends_unit_length_and_extrapolates_by_amplifying_the_angle() {
    let a = ry(10.0);
    let b = ry(50.0);
    assert!(q_angle(slerp(a, b, 0.0), a) < 1e-3);
    assert!(q_angle(slerp(a, b, 1.0), b) < 1e-3);
    // (acos em f32 perto de 1 só resolve ~0,03 grau)
    assert!((q_angle(slerp(a, b, 0.5), ry(30.0))) < 0.05);
    // t = 2 -> 90 graus
    assert!(q_angle(slerp(a, b, 2.0), ry(90.0)) < 0.05);
    for t in [-1.0, -0.2, 0.3, 1.7, 3.0] {
        let q = slerp(a, b, t);
        let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((len - 1.0).abs() < 1e-4, "t={t} len={len}");
    }
    // quase iguais: lerp normalizado estável
    let c = ry(10.0001);
    let m = slerp(a, c, 0.5);
    assert!(m.iter().all(|v| v.is_finite()));
}

#[test]
fn step_holds_until_the_next_key_and_linear_interpolates() {
    let ch = scalar_channel(&[(0, 10.0), (10, 20.0), (20, 40.0)], Interp::Step);
    let v = |f: f32| ch.evaluate(f, Extrapolation::Constant).unwrap().as_scalar().unwrap();
    assert_eq!(v(0.0), 10.0);
    assert_eq!(v(9.99), 10.0);
    assert_eq!(v(10.0), 20.0);
    assert_eq!(v(19.5), 20.0);
    assert_eq!(v(20.0), 40.0);
    let lin = scalar_channel(&[(0, 10.0), (10, 20.0), (20, 40.0)], Interp::Linear);
    let l = |f: f32| lin.evaluate(f, Extrapolation::Constant).unwrap().as_scalar().unwrap();
    assert!((l(5.0) - 15.0).abs() < 1e-5);
    assert!((l(15.0) - 30.0).abs() < 1e-5);
}

#[test]
fn bezier_is_monotonic_in_time_and_hits_the_endpoints() {
    for c in [[0.25, 0.1, 0.75, 0.9], [0.42, 0.0, 0.58, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.05, 0.9, 0.95, 0.1]] {
        assert!(bezier_ease(c, 0.0).abs() < 1e-5, "{c:?}");
        assert!((bezier_ease(c, 1.0) - 1.0).abs() < 1e-5, "{c:?}");
        // com y1 <= y2 dentro de [0,1] o progresso é monótono
        if c[1] <= c[3] && c[1] >= 0.0 && c[3] <= 1.0 {
            let mut prev = -1.0;
            for i in 0..=1000 {
                let y = bezier_ease(c, i as f32 / 1000.0);
                assert!(y >= prev - 1e-5, "{c:?} u={}", i as f32 / 1000.0);
                prev = y;
            }
        }
    }
    // alças "lineares" reproduzem a reta
    let lin = Interp::bezier_linear().control_points().unwrap();
    for i in 0..=20 {
        let u = i as f32 / 20.0;
        assert!((bezier_ease(lin, u) - u).abs() < 1e-4);
    }
    // ease in-out: lento nas pontas, 0.5 no meio
    assert!(bezier_ease([0.42, 0.0, 0.58, 1.0], 0.1) < 0.1);
    assert!((bezier_ease([0.42, 0.0, 0.58, 1.0], 0.5) - 0.5).abs() < 1e-4);
    assert!(bezier_ease([0.42, 0.0, 0.58, 1.0], 0.9) > 0.9);
}

#[test]
fn bezier_time_axis_is_monotonic_even_with_extreme_x_handles() {
    // a coordenada x da Bézier é monótona se x1,x2 em [0,1]: avaliar em u crescente nunca "volta no tempo"
    // (provado indiretamente: a solução de x(s)=u é única, então y(u) é função; conferimos que não há NaN e que é contínua)
    for c in [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 0.0], [0.0, 1.5, 1.0, -0.5], [5.0, 0.0, -3.0, 1.0]] {
        let mut prev = bezier_ease(c, 0.0);
        for i in 1..=2000 {
            let y = bezier_ease(c, i as f32 / 2000.0);
            assert!(y.is_finite());
            assert!((y - prev).abs() < 0.25, "descontinuidade {c:?} em {i}: {prev} -> {y}");
            prev = y;
        }
    }
}

#[test]
fn bezier_with_y_outside_unit_range_overshoots_for_anticipation() {
    let c = [0.3, -0.6, 0.7, 1.6];
    let mut min = 0.0f32;
    let mut max = 1.0f32;
    for i in 0..=200 {
        let y = bezier_ease(c, i as f32 / 200.0);
        min = min.min(y);
        max = max.max(y);
    }
    assert!(min < -0.05, "antecipação: {min}");
    assert!(max > 1.05, "overshoot: {max}");
}

#[test]
fn bezier_differs_from_step_and_linear_on_the_same_keys() {
    let mk = |i: Interp| scalar_channel(&[(0, 0.0), (12, 100.0)], i);
    let at = |ch: &Channel, f: f32| ch.evaluate(f, Extrapolation::Constant).unwrap().as_scalar().unwrap();
    let (s, l, e) = (mk(Interp::Step), mk(Interp::Linear), mk(Interp::Ease));
    assert_eq!(at(&s, 3.0), 0.0);
    assert!((at(&l, 3.0) - 25.0).abs() < 1e-4);
    assert!(at(&e, 3.0) < 25.0 - 3.0);
    assert!((at(&e, 6.0) - 50.0).abs() < 1e-3);
}

#[test]
fn quaternion_bezier_remaps_the_slerp_parameter() {
    let ch = chan(ChannelTarget::bone("Head"), vec![Key::new(0, KeyValue::Quat(ry(0.0))).with_interp(Interp::EaseIn), Key::new(10, KeyValue::Quat(ry(90.0)))]);
    let KeyValue::Quat(q) = ch.evaluate(5.0, Extrapolation::Constant).unwrap() else { panic!() };
    // ease-in: no meio do tempo a rotação ainda está abaixo de 45 graus
    assert!(q_angle(q, ry(0.0)) < 40.0, "{}", q_angle(q, ry(0.0)));
    let KeyValue::Quat(end) = ch.evaluate(10.0, Extrapolation::Constant).unwrap() else { panic!() };
    assert!(q_angle(end, ry(90.0)) < 0.05);
}

#[test]
fn extrapolation_constant_clamps_and_linear_continues() {
    let ch = scalar_channel(&[(10, 0.0), (20, 10.0)], Interp::Linear);
    let c = |f: f32| ch.evaluate(f, Extrapolation::Constant).unwrap().as_scalar().unwrap();
    let l = |f: f32| ch.evaluate(f, Extrapolation::Linear).unwrap().as_scalar().unwrap();
    assert_eq!(c(0.0), 0.0);
    assert_eq!(c(99.0), 10.0);
    assert!((l(30.0) - 20.0).abs() < 1e-4);
    assert!((l(0.0) + 10.0).abs() < 1e-4);
    assert_eq!(l(15.0), c(15.0));
    // uma chave só: valor constante
    let one = scalar_channel(&[(5, 7.0)], Interp::Linear);
    assert_eq!(one.evaluate(100.0, Extrapolation::Linear).unwrap().as_scalar(), Some(7.0));
    assert!(chan(ChannelTarget::Hips, vec![]).evaluate(1.0, Extrapolation::Constant).is_none());
}

// ---------------------------------------------------------------- loop / hold

#[test]
fn loop_pingpong_and_hold_map_the_clock() {
    let mut clip = AnimationClip::new(id("m"), "m");
    clip.range = Some([0, 24]);
    let fps = 24.0;
    assert_eq!(clip.map_frame(fps, 30.0), 30.0, "Once não mapeia");
    clip.loop_mode = LoopMode::Loop;
    assert_eq!(clip.map_frame(fps, 30.0), 6.0);
    assert_eq!(clip.map_frame(fps, 24.0), 0.0, "o quadro fim equivale ao início");
    assert_eq!(clip.map_frame(fps, -3.0), 21.0);
    clip.loop_mode = LoopMode::PingPong;
    assert_eq!(clip.map_frame(fps, 30.0), 18.0);
    assert_eq!(clip.map_frame(fps, 24.0), 24.0);
    assert_eq!(clip.map_frame(fps, 48.0), 0.0);
    assert_eq!(clip.map_frame(fps, 6.0), 6.0);
    // animar em 2s e 3s
    clip.loop_mode = LoopMode::Once;
    clip.hold_step = 2;
    assert_eq!(clip.map_frame(fps, 5.0), 4.0);
    assert_eq!(clip.map_frame(fps, 4.0), 4.0);
    clip.hold_step = 3;
    assert_eq!(clip.map_frame(fps, 8.9), 6.0);
    // com início diferente de zero
    clip.range = Some([10, 34]);
    assert_eq!(clip.map_frame(fps, 17.0), 16.0);
}

#[test]
fn hold_step_turns_a_smooth_curve_into_twos() {
    let mut p = project_with_clip();
    let clip = p.animation.clips.get_mut(&id("walk")).unwrap();
    let elbow = ChannelTarget::bone("LeftElbow");
    let smooth: Vec<_> = (0..=24).map(|f| clip.evaluate(24.0, f as f32).values[&elbow]).collect();
    clip.hold_step = 2;
    let twos: Vec<_> = (0..=24).map(|f| clip.evaluate(24.0, f as f32).values[&elbow]).collect();
    for f in (0..=24usize).step_by(2) {
        assert_eq!(twos[f], smooth[f], "quadro par é o da curva");
        if f + 1 <= 24 {
            assert_eq!(twos[f + 1], twos[f], "o quadro ímpar repete o par");
        }
    }
    assert_ne!(smooth[5], twos[5]);
}

#[test]
fn evaluation_is_deterministic_and_independent_of_channel_insertion_order() {
    let p = project_with_clip();
    let clip = p.animation.clips.get(&id("walk")).unwrap();
    let a = clip.evaluate(24.0, 7.3);
    let b = clip.evaluate(24.0, 7.3);
    assert_eq!(a, b);
    let mut shuffled = clip.clone();
    shuffled.channels.reverse();
    canonicalize_clip(&mut shuffled);
    assert_eq!(&shuffled, clip);
    assert_eq!(shuffled.evaluate(24.0, 7.3), a);
}

#[test]
fn legacy_node_tracks_are_still_evaluated() {
    let mut p = ProjectState::default();
    let node = crate::ids::NodeId::canonical_character();
    let mut clip = AnimationClip::new(id("old"), "old");
    clip.tracks = vec![crate::project::AnimationTrack {
        node_id: node.clone(),
        keyframes: vec![
            crate::project::TransformKeyframe { time_seconds: 0.0, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] },
            crate::project::TransformKeyframe { time_seconds: 1.0, translation: [2.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] },
        ],
    }];
    p.animation.clips.insert(clip.clip_id.clone(), clip.clone());
    p.validate().unwrap();
    let s = clip.evaluate(24.0, 12.0); // 0.5 s
    assert_eq!(s.values[&ChannelTarget::NodeTranslation { node_id: node }], KeyValue::Vec3([1.0, 0.0, 0.0]));
}

// ---------------------------------------------------------------- amostra -> projeto

#[test]
fn sample_applies_pose_nodes_sun_and_camera_without_history_and_reports_changes() {
    let mut p = project_with_clip();
    let node = crate::ids::NodeId::canonical_character();
    let mut clip = AnimationClip::new(id("all"), "all");
    clip.channels = vec![
        chan(ChannelTarget::bone("LeftElbow"), vec![Key::new(0, KeyValue::Quat(rx(0.0))), Key::new(10, KeyValue::Quat(rx(-90.0)))]),
        chan(ChannelTarget::NodeTranslation { node_id: node.clone() }, vec![Key::new(0, KeyValue::Vec3([0.0; 3])), Key::new(10, KeyValue::Vec3([1.0, 0.0, 0.0]))]),
        chan(ChannelTarget::Sun { prop: SunProp::Intensity }, vec![Key::new(0, KeyValue::Scalar(1.0)), Key::new(10, KeyValue::Scalar(2.0))]),
        chan(ChannelTarget::Camera { prop: CameraProp::Fov }, vec![Key::new(0, KeyValue::Scalar(30.0)), Key::new(10, KeyValue::Scalar(60.0))]),
        chan(ChannelTarget::Hand { side: HandSide::Left }, vec![Key::new(0, KeyValue::Curl([0.0; 5])), Key::new(10, KeyValue::Curl([1.0; 5]))]),
    ];
    canonicalize_clip(&mut clip);
    p.animation.clips.insert(clip.clip_id.clone(), clip.clone());
    p.validate().unwrap();
    let before = p.clone();
    let s = clip.evaluate(24.0, 5.0);
    assert!(s.apply_to_project(&mut p), "muda o projeto");
    assert!(q_angle(p.character.pose.rotation(bone::L_ELBOW).to_array(), rx(-45.0)) < 0.1);
    assert!((p.character_node().unwrap().transform.translation.x - 0.5).abs() < 1e-5);
    assert!((p.scene.lights[0].light.intensity - 1.5).abs() < 1e-5);
    assert!((p.scene.camera.camera.fov_y.to_degrees() - 45.0).abs() < 1e-3);
    assert!((p.character.pose.hands.left[0] - 0.5).abs() < 1e-5);
    assert!(!s.apply_to_project(&mut p), "aplicar a mesma amostra de novo não muda nada");
    assert_ne!(p, before);
    p.validate().unwrap();
}

#[test]
fn sample_only_overrides_keyed_bones() {
    let mut base = PoseState::default();
    base.set_rotation(bone::HEAD, Quat::from_rotation_y(0.4));
    let p = project_with_clip();
    let s = p.animation.clips[&id("walk")].evaluate(24.0, 12.0);
    let posed = s.apply_to_pose(&base);
    assert!(posed.rotation(bone::HEAD).dot(Quat::from_rotation_y(0.4)).abs() > 0.9999, "osso sem canal fica como estava");
    assert!(q_angle(posed.rotation(bone::L_ELBOW).to_array(), rx(-60.0)) < 0.1);
    assert!((posed.hips_translation[2] - 0.5).abs() < 1e-5);
}

// ---------------------------------------------------------------- comandos: inversos exatos

fn run_and_invert(p: &mut ProjectState, cmd: Command) {
    let before = p.clone();
    let applied = cmd.apply(p).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    let after = p.clone();
    assert_ne!(before, after, "{cmd:?} deveria mudar algo");
    applied.inverse.apply(p).unwrap_or_else(|e| panic!("inverso de {cmd:?}: {e}"));
    assert_eq!(*p, before, "inverso de {cmd:?} não restaurou o estado");
    // refazer devolve o estado depois
    cmd.apply(p).unwrap();
    assert_eq!(*p, after, "refazer {cmd:?}");
}

fn edit(e: AnimEdit) -> Command {
    Command::EditAnimation { edit: e }
}

fn r(target: ChannelTarget, frame: i32) -> KeyRef {
    KeyRef { target, frame }
}

#[test]
fn every_edit_has_an_exact_inverse() {
    let mut p = project_with_clip();
    let w = id("walk");
    let elbow = ChannelTarget::bone("LeftElbow");
    // inserir: substitui uma chave existente (12) e cria outra + um canal novo
    run_and_invert(
        &mut p,
        edit(AnimEdit::InsertKeys {
            clip_id: w.clone(),
            keys: vec![
                KeyInsert { target: elbow.clone(), key: Key::new(12, KeyValue::Quat(rx(-30.0))).with_interp(Interp::Step) },
                KeyInsert { target: elbow.clone(), key: Key::new(6, KeyValue::Quat(rx(-10.0))) },
                KeyInsert { target: ChannelTarget::bone("Head"), key: Key::new(3, KeyValue::Quat(ry(20.0))) },
            ],
        }),
    );
    // apagar (inclui esvaziar um canal -> some)
    run_and_invert(&mut p, edit(AnimEdit::DeleteKeys { clip_id: w.clone(), keys: vec![r(elbow.clone(), 12), r(ChannelTarget::Hips, 0), r(ChannelTarget::Hips, 24)] }));
    // mover: 12 -> 18 (livre) e 24 -> 0 (sobrescreve a chave 0, que o inverso precisa recolocar)
    run_and_invert(
        &mut p,
        edit(AnimEdit::MoveKeys {
            clip_id: w.clone(),
            moves: vec![KeyMove { target: elbow.clone(), frame: 6, to_frame: 18 }, KeyMove { target: elbow.clone(), frame: 24, to_frame: 0 }],
        }),
    );
    // mover com troca (swap)
    run_and_invert(
        &mut p,
        edit(AnimEdit::MoveKeys {
            clip_id: w.clone(),
            moves: vec![KeyMove { target: elbow.clone(), frame: 0, to_frame: 18 }, KeyMove { target: elbow.clone(), frame: 18, to_frame: 0 }],
        }),
    );
    // interpolação: chaves com interpolações diferentes viram um só valor e o inverso restaura cada uma
    run_and_invert(
        &mut p,
        edit(AnimEdit::SetInterpolation { clip_id: w.clone(), keys: vec![r(elbow.clone(), 0), r(elbow.clone(), 18), r(ChannelTarget::bone("Head"), 3)], interp: Interp::Bezier { c: [0.2, 0.0, 0.8, 1.4] } }),
    );
    run_and_invert(&mut p, edit(AnimEdit::SetClipRange { clip_id: w.clone(), range: Some([4, 40]) }));
    run_and_invert(&mut p, edit(AnimEdit::SetClipRange { clip_id: w.clone(), range: None }));
    run_and_invert(&mut p, edit(AnimEdit::SetClipPlayback { clip_id: w.clone(), loop_mode: Some(LoopMode::PingPong), extrapolation: Some(Extrapolation::Linear), hold_step: Some(3) }));
    run_and_invert(&mut p, edit(AnimEdit::SetMarkers { clip_id: w.clone(), markers: vec![Marker { frame: 12, name: "pé no chão".into() }, Marker { frame: 3, name: "início".into() }] }));
    run_and_invert(&mut p, edit(AnimEdit::RenameClip { clip_id: w.clone(), name: "Caminhada".into() }));
    // fps sem retime e com retime (colisões de quadro no retime para baixo)
    run_and_invert(&mut p, edit(AnimEdit::SetFps { fps: 30.0, retime: false }));
    run_and_invert(&mut p, edit(AnimEdit::SetFps { fps: 12.0, retime: true }));
    run_and_invert(&mut p, edit(AnimEdit::SetFps { fps: 60.0, retime: true }));
    // clips: criar, ativar, remover o ativo
    let other = AnimationClip::new(id("run"), "Run");
    run_and_invert(&mut p, edit(AnimEdit::PutClip { clip: other }));
    run_and_invert(&mut p, edit(AnimEdit::SetActiveClip { clip_id: Some(id("run")) }));
    run_and_invert(&mut p, edit(AnimEdit::RemoveClip { clip_id: w.clone() }));
    p.validate().unwrap();
}

#[test]
fn retime_scales_frames_and_keeps_the_duration_in_seconds() {
    let mut p = project_with_clip();
    edit(AnimEdit::SetFps { fps: 48.0, retime: true }).apply(&mut p).unwrap();
    let clip = &p.animation.clips[&id("walk")];
    assert_eq!(clip.range, Some([0, 48]));
    assert_eq!(clip.channel(&ChannelTarget::bone("LeftElbow")).unwrap().keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 24, 48]);
    assert_eq!(p.animation.fps, 48.0);
    // para baixo com colisão: a primeira chave do quadro vence
    let mut clip = AnimationClip::new(id("c"), "c");
    clip.channels = vec![scalar_channel(&[(0, 1.0), (1, 2.0), (2, 3.0), (3, 4.0)], Interp::Linear)];
    p.animation.clips.insert(clip.clip_id.clone(), clip);
    edit(AnimEdit::SetFps { fps: 24.0, retime: true }).apply(&mut p).unwrap();
    let ks = &p.animation.clips[&id("c")].channels[0].keys;
    assert!(ks.windows(2).all(|w| w[0].frame < w[1].frame), "estritamente crescente");
    p.validate().unwrap();
}

#[test]
fn random_edit_sequences_always_invert_exactly() {
    // gerador congruencial: determinístico
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = move |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let mut p = project_with_clip();
    let w = id("walk");
    let targets = [ChannelTarget::bone("LeftElbow"), ChannelTarget::bone("Head"), ChannelTarget::Hips, ChannelTarget::bone("RightKnee")];
    let mut done = 0;
    for _ in 0..400 {
        let t = targets[rnd(targets.len() as u64) as usize].clone();
        let frame = rnd(30) as i32;
        let value = match t.value_kind() {
            ValueKind::Quat => KeyValue::Quat(ry(rnd(180) as f32)),
            _ => KeyValue::Vec3([rnd(10) as f32, 0.0, 0.0]),
        };
        let interp = [Interp::Linear, Interp::Step, Interp::Ease, Interp::Bezier { c: [0.3, 0.1, 0.6, 0.9] }][rnd(4) as usize];
        let which = rnd(4);
        let existing = p.animation.clips[&w].channel(&t).map(|ch| ch.keys[rnd(ch.keys.len() as u64) as usize].frame);
        let cmd = match which {
            0 => edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: t.clone(), key: Key { frame, value, interp } }] }),
            1 => {
                let Some(k) = existing else { continue };
                edit(AnimEdit::DeleteKeys { clip_id: w.clone(), keys: vec![r(t.clone(), k)] })
            }
            2 => {
                let Some(k) = existing else { continue };
                edit(AnimEdit::MoveKeys { clip_id: w.clone(), moves: vec![KeyMove { target: t.clone(), frame: k, to_frame: frame }] })
            }
            _ => {
                let Some(k) = existing else { continue };
                edit(AnimEdit::SetInterpolation { clip_id: w.clone(), keys: vec![r(t.clone(), k)], interp })
            }
        };
        let before = p.clone();
        match cmd.apply(&mut p) {
            Ok(applied) => {
                let after = p.clone();
                applied.inverse.apply(&mut p).unwrap();
                assert_eq!(p, before, "{cmd:?}");
                cmd.apply(&mut p).unwrap();
                assert_eq!(p, after);
                done += 1;
            }
            Err(crate::command::CommandError::NoOp(_)) => assert_eq!(p, before),
            Err(e) => panic!("{cmd:?}: {e}"),
        }
    }
    assert!(done > 150, "poucas edições efetivas: {done}");
    p.validate().unwrap();
}

#[test]
fn canonical_form_is_independent_of_the_order_edits_were_made() {
    let mut a = project_with_clip();
    let mut b = project_with_clip();
    let w = id("walk");
    let t = ChannelTarget::bone("Head");
    let k = |f: i32| KeyInsert { target: t.clone(), key: Key::new(f, KeyValue::Quat(ry(f as f32))) };
    for f in [5, 1, 9] {
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![k(f)] }).apply(&mut a).unwrap();
    }
    for f in [9, 5, 1] {
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![k(f)] }).apply(&mut b).unwrap();
    }
    assert_eq!(a, b);
}

#[test]
fn invalid_and_noop_edits_are_rejected_and_leave_the_project_intact() {
    let mut p = project_with_clip();
    let before = p.clone();
    let w = id("walk");
    let bad: Vec<Command> = vec![
        // tipo errado para o canal
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::bone("Head"), key: Key::new(1, KeyValue::Scalar(1.0)) }] }),
        // osso desconhecido
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::bone("Tail"), key: Key::new(1, KeyValue::Quat(rx(0.0))) }] }),
        // nó inexistente
        edit(AnimEdit::InsertKeys {
            clip_id: w.clone(),
            keys: vec![KeyInsert { target: ChannelTarget::NodeTranslation { node_id: crate::ids::NodeId::from_slug("nope") }, key: Key::new(1, KeyValue::Vec3([0.0; 3])) }],
        }),
        // não finito, quadro negativo, duplicata na mesma chamada, Bézier com x fora de [0,1]
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::Hips, key: Key::new(1, KeyValue::Vec3([f32::NAN, 0.0, 0.0])) }] }),
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::Hips, key: Key::new(-1, KeyValue::Vec3([0.0; 3])) }] }),
        edit(AnimEdit::InsertKeys {
            clip_id: w.clone(),
            keys: vec![KeyInsert { target: ChannelTarget::Hips, key: Key::new(2, KeyValue::Vec3([0.0; 3])) }, KeyInsert { target: ChannelTarget::Hips, key: Key::new(2, KeyValue::Vec3([1.0; 3])) }],
        }),
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::Hips, key: Key::new(2, KeyValue::Vec3([0.0; 3])).with_interp(Interp::Bezier { c: [2.0, 0.0, 0.5, 1.0] }) }] }),
        // chave inexistente, clip inexistente, fps absurdo, hold fora da faixa, faixa invertida
        edit(AnimEdit::DeleteKeys { clip_id: w.clone(), keys: vec![r(ChannelTarget::Hips, 7)] }),
        edit(AnimEdit::DeleteKeys { clip_id: id("ghost"), keys: vec![r(ChannelTarget::Hips, 0)] }),
        edit(AnimEdit::SetFps { fps: 0.0, retime: false }),
        edit(AnimEdit::SetFps { fps: f32::NAN, retime: false }),
        edit(AnimEdit::SetClipPlayback { clip_id: w.clone(), loop_mode: None, extrapolation: None, hold_step: Some(0) }),
        edit(AnimEdit::SetClipRange { clip_id: w.clone(), range: Some([10, 5]) }),
        edit(AnimEdit::MoveKeys { clip_id: w.clone(), moves: vec![KeyMove { target: ChannelTarget::Hips, frame: 0, to_frame: -4 }] }),
        edit(AnimEdit::RenameClip { clip_id: w.clone(), name: "  ".into() }),
        // FOV fora da faixa
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::Camera { prop: CameraProp::Fov }, key: Key::new(0, KeyValue::Scalar(400.0)) }] }),
    ];
    for cmd in bad {
        assert!(cmd.apply(&mut p).is_err(), "{cmd:?} deveria ser recusado");
        assert_eq!(p, before, "{cmd:?} deixou rastros");
    }
    // sem efeito: reinserir a mesma chave, mover para o mesmo quadro, mesma interpolação, mesmo fps
    let same = [
        edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys: vec![KeyInsert { target: ChannelTarget::bone("LeftElbow"), key: Key::new(0, KeyValue::Quat(rx(0.0))) }] }),
        edit(AnimEdit::MoveKeys { clip_id: w.clone(), moves: vec![KeyMove { target: ChannelTarget::Hips, frame: 0, to_frame: 0 }] }),
        edit(AnimEdit::SetInterpolation { clip_id: w.clone(), keys: vec![r(ChannelTarget::Hips, 0)], interp: Interp::Linear }),
        edit(AnimEdit::SetFps { fps: 24.0, retime: false }),
        edit(AnimEdit::SetClipRange { clip_id: w.clone(), range: Some([0, 24]) }),
    ];
    for cmd in same {
        let e = cmd.apply(&mut p).unwrap_err();
        assert!(matches!(e, crate::command::CommandError::NoOp(_)), "{cmd:?}: {e}");
    }
    assert_eq!(p, before);
}

#[test]
fn autokey_batch_is_a_single_undo_step_that_restores_pose_and_keys() {
    let mut p = ProjectState::default();
    let mut h = CommandHistory::new(100);
    let w = id("auto");
    h.execute(&mut p, edit(AnimEdit::PutClip { clip: AnimationClip::new(w.clone(), "Auto") })).unwrap();
    let base_depth = h.undo_depth();
    let pose_cmd = Command::SetPose {
        bones: vec![crate::pose::PoseBoneEntry { bone: "LeftElbow".into(), rotation: rx(-80.0) }],
        root_translation: None,
        hips_translation: None,
        hands: None,
        op: PoseOp::Edit,
    };
    let after_pose = {
        let mut tmp = p.clone();
        pose_cmd.apply(&mut tmp).unwrap();
        tmp.character.pose.clone()
    };
    let keys = keys_from_pose(&after_pose, &[ChannelTarget::bone("LeftElbow")], 12, Interp::Linear);
    let batch = Command::Batch { commands: vec![pose_cmd, edit(AnimEdit::InsertKeys { clip_id: w.clone(), keys })] };
    h.execute(&mut p, batch).unwrap();
    assert_eq!(h.undo_depth(), base_depth + 1, "um passo só");
    assert!(!p.character.pose.is_rest());
    assert_eq!(p.animation.clips[&w].key_count(), 1);
    h.undo(&mut p).unwrap();
    assert!(p.character.pose.is_rest());
    assert_eq!(p.animation.clips[&w].key_count(), 0);
    h.redo(&mut p).unwrap();
    assert_eq!(p.animation.clips[&w].key_count(), 1);
    assert!(!p.character.pose.is_rest());
}

#[test]
fn mirror_keys_swap_sides_and_reflect_values() {
    let key = Key::new(4, KeyValue::Quat(Quat::from_euler(glam::EulerRot::XYZ, 0.2, 0.5, -0.3).to_array()));
    let (to, m) = mirror_key(&ChannelTarget::bone("LeftShoulder"), &key).unwrap();
    assert_eq!(to, ChannelTarget::bone("RightShoulder"));
    let KeyValue::Quat(q) = key.value else { panic!() };
    let KeyValue::Quat(mq) = m.value else { panic!() };
    // mesma regra de PoseState::mirrored: (x, -y, -z, w)
    let mut pose = PoseState::default();
    pose.set_rotation(bone::L_SHOULDER, Quat::from_xyzw(q[0], q[1], q[2], q[3]));
    let mirrored = pose.mirrored();
    assert!(mirrored.rotation(bone::R_SHOULDER).dot(Quat::from_xyzw(mq[0], mq[1], mq[2], mq[3])).abs() > 0.99999);
    // espelhar duas vezes volta ao original
    let (back_t, back) = mirror_key(&to, &m).unwrap();
    assert_eq!(back_t, ChannelTarget::bone("LeftShoulder"));
    let KeyValue::Quat(bq) = back.value else { panic!() };
    assert!(q_angle(bq, q) < 1e-3);
    // translação do quadril: x vira -x; mãos trocam de lado; nós/sol/câmera não espelham
    let (t, hm) = mirror_key(&ChannelTarget::Hips, &Key::new(0, KeyValue::Vec3([0.3, 1.0, 2.0]))).unwrap();
    assert_eq!((t, hm.value), (ChannelTarget::Hips, KeyValue::Vec3([-0.3, 1.0, 2.0])));
    let (t, _) = mirror_key(&ChannelTarget::Hand { side: HandSide::Left }, &Key::new(0, KeyValue::Curl([0.5; 5]))).unwrap();
    assert_eq!(t, ChannelTarget::Hand { side: HandSide::Right });
    assert!(mirror_key(&ChannelTarget::Sun { prop: SunProp::Intensity }, &Key::new(0, KeyValue::Scalar(1.0))).is_none());
}

#[test]
fn timecode_and_conversions() {
    assert_eq!(timecode(0, 24.0), "00:00:00:00");
    assert_eq!(timecode(25, 24.0), "00:00:01:01");
    assert_eq!(timecode(24 * 3600 + 24 * 61 + 5, 24.0), "01:01:01:05");
    assert_eq!(timecode(61, 30.0), "00:00:02:01");
    assert!((frame_to_seconds(48.0, 24.0) - 2.0).abs() < 1e-6);
    assert!((seconds_to_frame(0.5, 24.0) - 12.0).abs() < 1e-6);
}

// ---------------------------------------------------------------- compatibilidade de dados

#[test]
fn default_project_json_has_no_new_animation_fields() {
    let p = ProjectState::default();
    let json = serde_json::to_value(&p.animation).unwrap();
    assert_eq!(json, serde_json::json!({ "clips": {}, "active_clip": null, "playhead_seconds": 0.0 }));
    // clip antigo (sem os campos novos) continua carregando e re-serializa igual
    let old = serde_json::json!({ "clip_id": "ani_x", "name": "x", "duration_seconds": 2.0, "tracks": [] });
    let clip: AnimationClip = serde_json::from_value(old.clone()).unwrap();
    assert_eq!((clip.hold_step, clip.loop_mode, clip.range), (1, LoopMode::Once, None));
    assert_eq!(serde_json::to_value(&clip).unwrap(), old);
}

#[test]
fn project_with_animation_roundtrips_through_the_canonical_json() {
    let mut p = project_with_clip();
    {
        let c = p.animation.clips.get_mut(&id("walk")).unwrap();
        c.loop_mode = LoopMode::Loop;
        c.hold_step = 2;
        c.markers.push(Marker { frame: 3, name: "m".into() });
        c.channels[0].keys[1].interp = Interp::Bezier { c: [0.1, 0.2, 0.9, 1.3] };
    }
    p.animation.fps = 30.0;
    p.validate().unwrap();
    let json = p.to_canonical_json().unwrap();
    let back = ProjectState::from_json(&json).unwrap();
    assert_eq!(back.animation, p.animation);
    assert_eq!(back.to_canonical_json().unwrap(), json, "serialização estável");
}

#[test]
fn sanitize_repairs_unordered_duplicate_invalid_and_foreign_channels() {
    let mut p = ProjectState::default();
    let mut clip = AnimationClip::new(id("dirty"), "dirty");
    clip.hold_step = 0;
    clip.channels = vec![
        chan(ChannelTarget::bone("Head"), vec![Key::new(5, KeyValue::Quat(ry(10.0))), Key::new(2, KeyValue::Quat(ry(20.0))), Key::new(5, KeyValue::Quat([0.0, 0.0, 0.0, 0.0])), Key::new(-3, KeyValue::Quat(ry(0.0)))]),
        chan(ChannelTarget::bone("Wings"), vec![Key::new(0, KeyValue::Quat(ry(0.0)))]),
        chan(ChannelTarget::NodeScale { node_id: crate::ids::NodeId::from_slug("ghost") }, vec![Key::new(0, KeyValue::Vec3([1.0; 3]))]),
        chan(ChannelTarget::Hips, vec![Key::new(0, KeyValue::Scalar(1.0))]),
        chan(ChannelTarget::Root, vec![Key::new(0, KeyValue::Vec3([f32::INFINITY, 0.0, 0.0]))]),
    ];
    p.animation.clips.insert(clip.clip_id.clone(), clip);
    p.animation.fps = f32::NAN;
    let p = p.sanitize();
    p.validate().unwrap();
    let c = &p.animation.clips[&id("dirty")];
    assert_eq!(c.hold_step, 1);
    assert_eq!(c.channels.len(), 1, "só o canal da cabeça sobrevive");
    assert_eq!(c.channels[0].keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![2, 5]);
    assert_eq!(p.animation.fps, DEFAULT_FPS);
}

#[test]
fn validate_rejects_a_hand_edited_unordered_clip_and_dangling_node_channels() {
    let mut p = project_with_clip();
    p.animation.clips.get_mut(&id("walk")).unwrap().channels[0].keys.swap(0, 1);
    assert!(p.validate().is_err());
    let mut p = project_with_clip();
    p.animation.clips.get_mut(&id("walk")).unwrap().channels.push(chan(
        ChannelTarget::NodeRotation { node_id: crate::ids::NodeId::from_slug("ghost") },
        vec![Key::new(0, KeyValue::Quat([0.0, 0.0, 0.0, 1.0]))],
    ));
    canonicalize_clip(p.animation.clips.get_mut(&id("walk")).unwrap());
    assert!(matches!(p.validate(), Err(crate::project::ProjectError::DanglingReference { .. })));
}

#[test]
fn key_helpers_cover_targets_and_pose_capture() {
    let mut pose = PoseState::default();
    pose.set_rotation(bone::HEAD, Quat::from_rotation_y(0.3));
    pose.hips_translation = [0.0, -0.1, 0.0];
    pose.hands.left = [0.5; 5];
    let keys = keys_from_pose(&pose, &full_pose_targets(), 7, Interp::Step);
    assert_eq!(keys.len(), 24 + 4);
    assert!(keys.iter().all(|k| k.key.frame == 7 && k.key.interp == Interp::Step));
    let head = keys.iter().find(|k| k.target == ChannelTarget::bone("Head")).unwrap();
    assert!(matches!(head.key.value, KeyValue::Quat(_)));
    let targets = targets_of_pose_edit(&["LeftElbow".to_string(), "Nope".to_string()], true, false, true);
    assert_eq!(targets.len(), 1 + 1 + 2, "osso desconhecido é ignorado");
    // current_value lê o projeto vivo
    let mut p = ProjectState::default();
    p.character.pose = pose;
    assert_eq!(current_value(&p, &ChannelTarget::Hips), Some(KeyValue::Vec3([0.0, -0.1, 0.0])));
    assert!(current_value(&p, &ChannelTarget::NodeScale { node_id: crate::ids::NodeId::from_slug("zzz") }).is_none());
}

#[test]
fn effective_range_falls_back_to_keys_then_duration_then_one_second() {
    let mut c = AnimationClip::new(id("r"), "r");
    assert_eq!(c.effective_range(24.0), (0, 24));
    c.duration_seconds = 2.0;
    assert_eq!(c.effective_range(24.0), (0, 48));
    c.channels = vec![scalar_channel(&[(0, 0.0), (100, 1.0)], Interp::Linear)];
    assert_eq!(c.effective_range(24.0), (0, 100));
    c.range = Some([5, 9]);
    assert_eq!(c.effective_range(24.0), (5, 9));
}
