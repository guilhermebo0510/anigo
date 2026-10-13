//! Grupos de vértices (zonas anatômicas, máscaras, landmarks), UV e a redução para a paleta de 24 ossos.

use anigo_charbase::cage::Region;
use anigo_charbase::preview::{collapse_to_canonical24, skin_core_mesh, CANONICAL24};
use anigo_charbase::topology::{reference, ZONES};
use anigo_charbase::uv::{chart_rect, CHART_COUNT};
use anigo_charbase::{generate, Gender};
use anigo_core::morph_catalog::AnatomicalZone;
use glam::Vec3;

#[test]
fn eighteen_zones_match_the_core_enum_and_cover_the_expected_regions() {
    let t = reference();
    assert_eq!(ZONES.len(), 18);
    // o enum do core tem exatamente estas 18 zonas, na mesma ordem
    let expected = [
        AnatomicalZone::GlobalSilhouette,
        AnatomicalZone::Craniofacial,
        AnatomicalZone::Eyes,
        AnatomicalZone::Eyebrows,
        AnatomicalZone::Nose,
        AnatomicalZone::MouthLips,
        AnatomicalZone::JawChin,
        AnatomicalZone::Ears,
        AnatomicalZone::NeckTrapezius,
        AnatomicalZone::ShouldersClavicles,
        AnatomicalZone::ChestPectorals,
        AnatomicalZone::BustFemale,
        AnatomicalZone::AbdomenWaist,
        AnatomicalZone::PelvisHips,
        AnatomicalZone::Gluteus,
        AnatomicalZone::UpperLimbs,
        AnatomicalZone::HandsFingers,
        AnatomicalZone::LowerLimbs,
    ];
    for (i, z) in expected.iter().enumerate() {
        assert_eq!(ZONES[i].0, *z);
        assert!(ZONES[i].1.starts_with("zone_"));
        let n = t.zones.iter().filter(|w| w[i] > 0.0).count();
        assert!(n >= 40, "{}: só {n} vértices", ZONES[i].1);
        assert!(t.zones.iter().all(|w| (0.0..=1.0).contains(&w[i])), "{}", ZONES[i].1);
    }
    let zi = |z: AnatomicalZone| ZONES.iter().position(|(zz, _)| *zz == z).unwrap();
    for (v, w) in t.zones.iter().enumerate() {
        // as regiões grossas respeitam as zonas de membros
        match t.region[v] {
            Region::Arm => assert_eq!(w[zi(AnatomicalZone::UpperLimbs)], 1.0),
            Region::Leg | Region::Foot | Region::Toes => assert_eq!(w[zi(AnatomicalZone::LowerLimbs)], 1.0),
            Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little | Region::Hand => {
                assert_eq!(w[zi(AnatomicalZone::HandsFingers)], 1.0)
            }
            _ => {}
        }
    }
    // olhos: o anel interno da pálpebra está 100% na zona de olhos
    for s in ["L", "R"] {
        for &v in &t.loop_ids(&format!("eye_{s}_ring3")) {
            assert_eq!(t.zones[v as usize][zi(AnatomicalZone::Eyes)], 1.0);
        }
    }
    for &v in &t.loop_ids("mouth_ring3") {
        assert_eq!(t.zones[v as usize][zi(AnatomicalZone::MouthLips)], 1.0);
    }
    // o busto está no peito, não nas pernas
    let c = generate(Gender::Female);
    for (v, w) in t.zones.iter().enumerate() {
        if w[zi(AnatomicalZone::BustFemale)] > 0.5 {
            assert!((1.0..1.3).contains(&c.positions[v].y), "busto em y={}", c.positions[v].y);
            assert!(c.positions[v].z > 0.0 || t.attr[v][2] > -0.2);
        }
    }
}

#[test]
fn masks_and_landmarks_follow_the_f1_3_table() {
    let t = reference();
    for name in [
        "mask_face_normal_proxy", "mask_outline_weight", "mask_shadow_threshold_offset", "mask_hide_under_cloth_torso", "mask_hide_under_cloth_arms",
        "mask_hide_under_cloth_legs", "mask_hide_under_cloth_feet", "mask_hide_under_cloth_hands", "mask_skin_sss", "lash_upper", "lash_lower",
        "hair_scalp_region", "cloth_attach_waist", "cloth_attach_shoulder", "cloth_attach_neck", "cloth_attach_wrist", "cloth_attach_ankle",
    ] {
        let m = t.masks.get(name).unwrap_or_else(|| panic!("máscara ausente: {name}"));
        assert_eq!(m.len(), t.n_verts, "{name}");
        assert!(m.iter().all(|x| (0.0..=1.0).contains(x) && x.is_finite()), "{name}");
        if name != "mask_shadow_threshold_offset" {
            assert!(m.iter().any(|x| *x > 0.0), "{name} vazia");
        }
    }
    // o couro cabeludo cobre o topo da cabeça e não os pés nem o rosto frontal baixo
    let c = generate(Gender::Female);
    let scalp = &t.masks["hair_scalp_region"];
    let top = t.landmark("lm_head_top").unwrap().eval(&c.positions);
    let chin = t.landmark("lm_chin").unwrap().eval(&c.positions);
    for (v, s) in scalp.iter().enumerate() {
        if *s > 0.5 {
            assert!(c.positions[v].y > chin.y + 0.04 && c.positions[v].y <= top.y + 1e-3, "couro cabeludo fora da cabeça em y={}", c.positions[v].y);
        }
    }
    // landmarks: localizadores afins dentro da caixa do corpo
    assert!(t.landmarks.len() >= 40);
    let (mn, mx) = c.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    for (name, l) in &t.landmarks {
        assert!((l.weight_sum() - 1.0).abs() < 1e-4, "{name}: soma {}", l.weight_sum());
        let p = l.eval(&c.positions);
        assert!(p.cmpge(mn - Vec3::splat(1e-3)).all() && p.cmple(mx + Vec3::splat(1e-3)).all(), "{name} fora do corpo: {p:?}");
    }
    for n in [
        "lm_eye_center_L", "lm_eye_center_R", "lm_nose_tip", "lm_mouth_corner_L", "lm_mouth_corner_R", "lm_chin", "lm_ear_top_L", "lm_ear_center_L", "lm_head_top",
        "lm_neck_base", "lm_shoulder_L", "lm_elbow_L", "lm_wrist_L", "lm_hip_L", "lm_knee_L", "lm_ankle_L", "lm_waist",
    ] {
        assert!(t.landmark(n).is_some(), "{n}");
    }
    // simetria dos landmarks
    let l = t.landmark("lm_mouth_corner_L").unwrap().eval(&c.positions);
    let r = t.landmark("lm_mouth_corner_R").unwrap().eval(&c.positions);
    assert!(l.x > 0.0 && (l.x + r.x).abs() < 1e-5 && (l.y - r.y).abs() < 1e-5);
}

#[test]
fn uv_charts_are_bounded_disjoint_and_not_degenerate() {
    let t = reference();
    let c = generate(Gender::Female);
    assert_eq!(t.uv.len(), t.faces.len());
    // retângulos dos charts dentro de [0,1] e sem sobreposição
    for i in 0..CHART_COUNT {
        let a = chart_rect(i);
        assert!(a[0] >= 0.0 && a[1] >= 0.0 && a[2] <= 1.0 && a[3] <= 1.0 && a[0] < a[2] && a[1] < a[3]);
        for j in i + 1..CHART_COUNT {
            let b = chart_rect(j);
            let overlap = a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
            assert!(!overlap, "charts {i} e {j} se sobrepõem");
        }
    }
    let mut ratios: Vec<Vec<f32>> = vec![Vec::new(); CHART_COUNT];
    for (fi, f) in t.faces.iter().enumerate() {
        let ch = t.chart_of_face[fi] as usize;
        let rect = chart_rect(ch);
        for k in 0..4 {
            let uv = t.uv[fi][k];
            assert!(uv[0].is_finite() && uv[1].is_finite());
            assert!(uv[0] >= rect[0] - 1e-5 && uv[0] <= rect[2] + 1e-5 && uv[1] >= rect[1] - 1e-5 && uv[1] <= rect[3] + 1e-5, "face {fi} fora do retângulo do chart {ch}");
        }
        let area_uv = {
            let q = &t.uv[fi];
            let s = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]));
            (s(q[0], q[1], q[2]) + s(q[0], q[2], q[3])).abs()
        };
        let p: Vec<Vec3> = f.iter().map(|&i| c.positions[i as usize]).collect();
        let area_w = 0.5 * ((p[2] - p[0]).cross(p[3] - p[1])).length();
        if area_w > 1e-9 {
            ratios[ch].push(area_uv / area_w);
        }
    }
    // distorção limitada: a razão área UV/área do mundo de quase todas as faces fica a < 30x da mediana do chart
    for (ch, r) in ratios.iter_mut().enumerate() {
        if r.is_empty() {
            continue;
        }
        r.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = r[r.len() / 2].max(1e-12);
        let bad = r.iter().filter(|x| **x < med / 30.0 || **x > med * 30.0).count();
        assert!(bad as f32 <= 0.03 * r.len() as f32 + 3.0, "chart {ch}: {bad}/{} faces muito distorcidas", r.len());
    }
}

#[test]
fn collapse_to_canonical24_keeps_the_render_contract_palette() {
    let t = reference();
    let c = generate(Gender::Female);
    let w24 = collapse_to_canonical24(&c.skeleton, &t.weights);
    assert_eq!(w24.len(), t.n_verts);
    let idx = |n: &str| CANONICAL24.iter().position(|x| *x == n).unwrap() as u16;
    for (v, w) in w24.iter().enumerate() {
        let sum: f32 = w.iter().map(|x| x.1).sum();
        assert!((sum - 1.0).abs() < 1e-4, "vértice {v}");
        assert!(w.iter().all(|x| (x.0 as usize) < 24 && x.1 >= 0.0));
        // nunca usa Root nem Pelvis (o esqueleto novo não tem equivalentes)
        assert!(w.iter().filter(|x| x.1 > 0.0).all(|x| x.0 != idx("Root") && x.0 != idx("Pelvis")));
    }
    // dedos -> pulso; twist -> osso pai; olhos e mandíbula -> cabeça
    let finger_vertex = t.mark("finger_L_index_ring5")[0] as usize;
    assert_eq!(w24[finger_vertex][0].0, idx("LeftWrist"));
    let ear = t.mark("ear_L_center")[0] as usize;
    assert_eq!(w24[ear][0].0, idx("Head"));
    // braço esquerdo só usa ossos esquerdos do núcleo
    for (v, r) in t.region.iter().enumerate() {
        if *r == Region::Arm && c.positions[v].x > 0.45 {
            for x in w24[v].iter().filter(|x| x.1 > 0.0) {
                assert!([idx("LeftClavicle"), idx("LeftShoulder"), idx("LeftElbow"), idx("LeftWrist")].contains(&x.0), "{}", CANONICAL24[x.0 as usize]);
            }
        }
    }
    // a malha de render usa o formato de 72 B do núcleo, sem tocar no contrato
    let mesh = skin_core_mesh(&c);
    assert_eq!(mesh.vertices.len(), t.n_verts);
    assert_eq!(std::mem::size_of::<anigo_core::mesh::Vertex>(), 72);
    assert!(mesh.vertices.iter().all(|v| v.joints.iter().all(|&j| j < 24)));
    assert_eq!(anigo_core::bone_sync::CANONICAL_JOINT_COUNT, 24);
}
