use super::*;
use anigo_core::animation::{canonicalize_clip, Channel, ChannelTarget, Key, KeyValue};

fn rig() -> PoseRig {
    PoseRig::default()
}

fn q(x: f32, y: f32, z: f32) -> KeyValue {
    KeyValue::Quat((Quat::from_rotation_z(z) * Quat::from_rotation_y(y) * Quat::from_rotation_x(x)).to_array())
}

fn clip() -> AnimationClip {
    let mut c = AnimationClip::new(AnimationClipId::from_slug("acenar"), "Acenar");
    c.range = Some([0, 24]);
    c.channels = vec![
        Channel { target: ChannelTarget::bone("LeftShoulder"), keys: vec![Key::new(0, q(0.0, 0.0, 0.0)), Key::new(12, q(-0.4, 0.0, 1.3)), Key::new(24, q(0.0, 0.0, 0.0))] },
        Channel { target: ChannelTarget::bone("LeftElbow"), keys: vec![Key::new(0, q(0.0, 0.0, 0.0)), Key::new(12, q(-1.2, 0.0, 0.0)), Key::new(24, q(-0.2, 0.0, 0.0))] },
        Channel { target: ChannelTarget::bone("Head"), keys: vec![Key::new(0, q(0.0, 0.0, 0.0)), Key::new(24, q(0.0, 0.6, 0.0))] },
        Channel { target: ChannelTarget::bone("RightThigh"), keys: vec![Key::new(0, q(0.5, 0.0, 0.0)), Key::new(24, q(-0.5, 0.0, 0.0))] },
        Channel { target: ChannelTarget::Hips, keys: vec![Key::new(0, KeyValue::Vec3([0.0, 0.0, 0.0])), Key::new(24, KeyValue::Vec3([0.0, -0.05, 0.8]))] },
    ];
    canonicalize_clip(&mut c);
    c
}

fn deg(a: Quat, b: Quat) -> f32 {
    a.angle_between(b).to_degrees()
}

fn assert_same_motion(a: &AnimationClip, b: &AnimationClip, tol_deg: f32) {
    for f in [0.0f32, 3.0, 6.0, 12.0, 18.0, 24.0] {
        let (pa, pb) = (a.evaluate(24.0, f).apply_to_pose(&PoseState::default()), b.evaluate(24.0, f).apply_to_pose(&PoseState::default()));
        for bone in [bone::L_SHOULDER, bone::L_ELBOW, bone::HEAD, bone::R_THIGH] {
            let d = deg(pa.rotation(bone), pb.rotation(bone));
            assert!(d < tol_deg, "quadro {f} {}: {d}°", BONE_NAMES[bone]);
        }
        let dt = Vec3::from_array(pa.hips_translation).distance(Vec3::from_array(pb.hips_translation));
        assert!(dt < 0.01, "quadro {f}: quadril {dt}");
    }
}

#[test]
fn glb_export_then_import_reproduces_the_clip() {
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    assert!(glb.starts_with(b"glTF"));
    let imp = import_gltf_animations(&glb, None, &rig(), 24.0).unwrap();
    assert_eq!(imp.clips.len(), 1);
    assert_eq!(imp.clips[0].name, "Acenar");
    assert!((imp.clips[0].seconds - 1.0).abs() < 1e-5);
    assert_eq!(imp.mapped_bones, BONE_COUNT - 1);
    assert_same_motion(&c, &imp.clips[0].clip, 0.6);
    // redundância removida: bem menos chaves que 25 quadros por canal
    assert!(imp.clips[0].clip.key_count() < 30, "{}", imp.clips[0].clip.key_count());
    assert_eq!(imp.clips[0].clip.range, Some([0, 24]));
}

#[test]
fn vrma_export_uses_a_t_pose_skeleton_humanoid_names_and_the_extension_and_roundtrips() {
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), true).unwrap();
    let (json, _) = parse_glb(&glb).unwrap();
    // extensão e mapa humanoide
    assert_eq!(json.pointer("/extensions/VRMC_vrm_animation/specVersion").and_then(Value::as_str), Some("1.0"));
    assert!(json["extensionsUsed"].as_array().unwrap().iter().any(|e| e == "VRMC_vrm_animation"));
    let hb = json.pointer("/extensions/VRMC_vrm_animation/humanoid/humanBones").unwrap().as_object().unwrap();
    for name in ["hips", "spine", "head", "leftUpperArm", "rightLowerArm", "leftFoot", "rightToes", "leftShoulder"] {
        assert!(hb.contains_key(name), "{name}");
    }
    assert!(!hb.contains_key("leftEye"), "olhos não têm animação no VRMA");
    // os nomes dos nós são os humanoides
    let names: Vec<&str> = json["nodes"].as_array().unwrap().iter().map(|n| n["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"leftUpperArm") && !names.contains(&"Root") && !names.contains(&"Pelvis"));
    // repouso em T: o antebraço esquerdo sai na horizontal (+X) do braço
    let idx = |n: &str| names.iter().position(|x| *x == n).unwrap();
    let t = json["nodes"][idx("leftLowerArm")]["translation"].as_array().unwrap();
    let (tx, ty) = (t[0].as_f64().unwrap(), t[1].as_f64().unwrap());
    assert!(tx > 0.15 && ty.abs() < 0.02, "T-pose: ({tx}, {ty})");
    // sem escala nos ossos e sem translação fora do quadril
    for ch in json["animations"][0]["channels"].as_array().unwrap() {
        let path = ch["target"]["path"].as_str().unwrap();
        assert_ne!(path, "scale");
        if path == "translation" {
            assert_eq!(ch["target"]["node"].as_u64().unwrap() as usize, idx("hips"));
        }
    }
    // a rotação do braço no VRMA é diferente da do arquivo canônico (convertida para o repouso em T)
    let plain = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    assert_ne!(glb, plain);
    // reimportar o VRMA devolve o mesmo movimento
    let imp = import_gltf_animations(&glb, None, &rig(), 24.0).unwrap();
    assert!(imp.mapped_bones >= 20, "{}", imp.mapped_bones);
    assert_same_motion(&c, &imp.clips[0].clip, 1.0);
}

#[test]
fn gltf_json_with_a_data_uri_buffer_imports_like_the_glb() {
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    let (mut json, bin) = parse_glb(&glb).unwrap();
    let uri = format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bin));
    json["buffers"][0]["uri"] = Value::String(uri);
    let text = serde_json::to_vec(&json).unwrap();
    let imp = import_gltf_animations(&text, None, &rig(), 24.0).unwrap();
    assert_same_motion(&c, &imp.clips[0].clip, 0.6);
}

#[test]
fn external_buffers_are_confined_to_the_source_folder() {
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    let (mut json, bin) = parse_glb(&glb).unwrap();
    let dir = std::env::temp_dir().join(format!("anigo-gltf-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.bin"), &bin).unwrap();
    json["buffers"][0]["uri"] = Value::String("a.bin".into());
    let text = serde_json::to_vec(&json).unwrap();
    assert!(import_gltf_animations(&text, Some(&dir), &rig(), 24.0).is_ok());
    assert!(import_gltf_animations(&text, None, &rig(), 24.0).is_err(), "sem pasta de origem não há buffer externo");
    for evil in ["../a.bin", "/etc/passwd", "sub/../../a.bin"] {
        json["buffers"][0]["uri"] = Value::String(evil.into());
        let text = serde_json::to_vec(&json).unwrap();
        let e = import_gltf_animations(&text, Some(&dir), &rig(), 24.0).unwrap_err();
        assert!(e.to_lowercase().contains("recusado") || e.to_lowercase().contains("buffer"), "{evil}: {e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn step_animations_import_as_step_keys_without_intermediate_frames() {
    // exporta, marca todos os samplers como STEP no JSON e reimporta
    let mut c = clip();
    for ch in &mut c.channels {
        for k in &mut ch.keys {
            k.interp = Interp::Step;
        }
    }
    c.hold_step = 6; // movimento "em 6s": amostras repetidas
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    let (mut json, bin) = parse_glb(&glb).unwrap();
    for s in json["animations"][0]["samplers"].as_array_mut().unwrap() {
        s["interpolation"] = Value::String("STEP".into());
    }
    let imp = import_gltf_animations(&build_glb(&json, &bin), None, &rig(), 24.0).unwrap();
    let clip = &imp.clips[0].clip;
    for ch in &clip.channels {
        assert!(ch.keys.iter().all(|k| k.interp == Interp::Step), "{:?}", ch.target);
    }
    // só as mudanças viram chaves (6 quadros iguais = 1 chave): bem menos que 25 por canal
    let head = clip.channel(&ChannelTarget::bone("Head")).unwrap();
    assert!(head.keys.len() <= 6, "{}", head.keys.len());
}

#[test]
fn track_sampling_covers_step_linear_slerp_cubic_and_clamping() {
    let lin = Track { times: vec![0.0, 1.0], values: vec![0.0, 0.0, 0.0, 10.0, 20.0, 30.0], interp: GInterp::Linear, comps: 3 };
    assert_eq!(&lin.sample(0.5)[..3], &[5.0, 10.0, 15.0]);
    assert_eq!(&lin.sample(-3.0)[..3], &[0.0, 0.0, 0.0], "clamp antes");
    assert_eq!(&lin.sample(9.0)[..3], &[10.0, 20.0, 30.0], "clamp depois");
    let step = Track { times: vec![0.0, 1.0], values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], interp: GInterp::Step, comps: 3 };
    assert_eq!(&step.sample(0.99)[..3], &[1.0, 2.0, 3.0]);
    assert_eq!(&step.sample(1.0)[..3], &[4.0, 5.0, 6.0]);
    // quaternion: caminho curto mesmo com sinais opostos
    let a = Quat::IDENTITY;
    let b = -Quat::from_rotation_y(2.0);
    let rot = Track { times: vec![0.0, 1.0], values: [a.to_array(), b.to_array()].concat(), interp: GInterp::Linear, comps: 4 };
    let mid = rot.sample(0.5);
    assert!(deg(Quat::from_xyzw(mid[0], mid[1], mid[2], mid[3]), Quat::from_rotation_y(1.0)) < 0.1);
    // cubic spline escalar: tangentes zero = smoothstep; no meio vale a média
    let cubic = Track { times: vec![0.0, 2.0], values: vec![0.0, 0.0, 0.0, /**/ 0.0, 0.0, 0.0, /**/ 0.0, 0.0, 0.0, /**/ 0.0, 0.0, 0.0, /**/ 6.0, 6.0, 6.0, /**/ 0.0, 0.0, 0.0], interp: GInterp::Cubic, comps: 3 };
    // entradas: [in0, v0, out0, in1, v1, out1] com v1 = 6
    let cubic = Track { values: vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 6.0, 6.0, 6.0, 0.0, 0.0, 0.0], ..cubic };
    assert!((cubic.sample(1.0)[0] - 3.0).abs() < 1e-5);
    assert!((cubic.sample(0.5)[0] - 0.9375).abs() < 1e-4, "smoothstep(0.25)*6 = 0.9375: {}", cubic.sample(0.5)[0]);
    assert_eq!(cubic.sample(0.0)[0], 0.0);
    assert_eq!(cubic.sample(2.0)[0], 6.0);
}

#[test]
fn hostile_or_unsupported_files_fail_cleanly() {
    let r = rig();
    assert!(import_gltf_animations(b"", None, &r, 24.0).is_err());
    assert!(import_gltf_animations(b"glTF\x02\x00\x00\x00", None, &r, 24.0).is_err());
    assert!(import_gltf_animations(b"{ nope", None, &r, 24.0).is_err());
    assert!(import_gltf_animations(br#"{"asset":{"version":"2.0"},"nodes":[{}]}"#, None, &r, 24.0).is_err());
    // glTF sem esqueleto humanoide reconhecível
    let json = br#"{"asset":{"version":"2.0"},"nodes":[{"name":"Cube"},{"name":"Foo"}],"animations":[{"channels":[],"samplers":[]}]}"#;
    let e = import_gltf_animations(json, None, &r, 24.0).unwrap_err();
    assert!(e.contains("humanoide"), "{e}");
    // um clip sem canais de corpo não exporta
    let empty = AnimationClip::new(AnimationClipId::from_slug("e"), "e");
    assert!(export_gltf_animation(&empty, 24.0, &r, false).is_err());
    // truncar um GLB válido no meio não derruba o processo
    let glb = export_gltf_animation(&clip(), 24.0, &r, false).unwrap();
    for cut in [20usize, 100, glb.len() / 2, glb.len() - 3] {
        let _ = import_gltf_animations(&glb[..cut], None, &r, 24.0);
    }
    // tempos fora de ordem e contagens erradas são ignorados com aviso, sem panic
    let (mut json, bin) = parse_glb(&glb).unwrap();
    let times_acc = json["animations"][0]["samplers"][0]["input"].as_u64().unwrap() as usize;
    json["accessors"][times_acc]["count"] = Value::from(3);
    let _ = import_gltf_animations(&build_glb(&json, &bin), None, &r, 24.0);
}

#[test]
fn a_humanoid_map_in_a_vrm_0x_document_is_honored() {
    // nomes dos nós sem pistas (n0..n23); o mapa vem de extensions.VRM.humanoid.humanBones (VRM 0.x)
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    let (mut json, bin) = parse_glb(&glb).unwrap();
    let mut human = Vec::new();
    for (i, n) in json["nodes"].as_array_mut().unwrap().iter_mut().enumerate() {
        let canonical = n["name"].as_str().unwrap().to_string();
        n["name"] = Value::String(format!("n{i}"));
        if let Some(h) = (0..BONE_COUNT).find(|b| BONE_NAMES[*b] == canonical).and_then(vrm_humanoid_name) {
            human.push(json!({ "bone": h, "node": i }));
        }
    }
    json["extensions"] = json!({ "VRM": { "humanoid": { "humanBones": human } } });
    let imp = import_gltf_animations(&build_glb(&json, &bin), None, &rig(), 24.0).unwrap();
    assert!(imp.mapped_bones >= 20);
    // o canônico "LeftShoulder" (braço) virou leftUpperArm -> braço de novo
    assert_same_motion(&c, &imp.clips[0].clip, 1.0);
}

#[test]
fn skeleton_joints_unused_by_the_map_are_ignored_and_scale_is_reported() {
    let c = clip();
    let glb = export_gltf_animation(&c, 24.0, &rig(), false).unwrap();
    let (mut json, bin) = parse_glb(&glb).unwrap();
    json["nodes"][0]["scale"] = json!([2.0, 2.0, 2.0]);
    let imp = import_gltf_animations(&build_glb(&json, &bin), None, &rig(), 24.0).unwrap();
    assert!(imp.warnings.iter().any(|w| w.contains("escala")), "{:?}", imp.warnings);
}
