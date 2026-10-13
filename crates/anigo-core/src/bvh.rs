//! BVH (Biovision Hierarchy): leitura para o retargeter e escrita do clip ativo.
//!
//! Formato [V: spec Wisconsin CS-838]: `HIERARCHY` (`ROOT`/`JOINT`/`End Site`, `OFFSET`, `CHANNELS`), depois `MOTION`
//! (`Frames:`, `Frame Time:`) com um quadro por linha, os números "na ordem das especificações de canal conforme a
//! hierarquia foi lida". A rotação de uma junta é a concatenação das matrizes dos canais na ordem listada (a primeira é a
//! mais externa, convenção de coluna `R = R_c1 · R_c2 · R_c3`) [C: tradução da convenção de linha `v R = v YXZ` do texto].
//! Os canais de posição valem como translação LOCAL absoluta da junta (não somam ao `OFFSET`), como no importador do Blender [S].
//!
//! Limites: 512 juntas, 200 000 quadros e 64 MiB, para arquivos hostis não consumirem memória.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glam::{EulerRot, Quat, Vec3};

use crate::animation::{ChannelTarget, KeyValue};
use crate::pose::{bone, PoseRig, BONE_NAMES, BONE_PARENTS};
use crate::project::AnimationClip;
use crate::retarget::{SrcFrame, SrcJoint, SrcSkeleton};

pub const BVH_MAX_JOINTS: usize = 512;
pub const BVH_MAX_FRAMES: usize = 200_000;
pub const BVH_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chan {
    Xpos,
    Ypos,
    Zpos,
    Xrot,
    Yrot,
    Zrot,
}

/// Resultado da leitura.
#[derive(Debug, Clone, PartialEq)]
pub struct BvhMotion {
    pub skeleton: SrcSkeleton,
    pub frames: Vec<SrcFrame>,
    pub fps: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BvhError {
    #[error("arquivo BVH grande demais ({0} bytes)")]
    TooLarge(usize),
    #[error("BVH inválido: {0}")]
    Parse(String),
}

fn err<T>(msg: impl Into<String>) -> Result<T, BvhError> {
    Err(BvhError::Parse(msg.into()))
}

struct Tokens<'a> {
    it: std::iter::Peekable<std::str::SplitWhitespace<'a>>,
}

impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<&'a str, BvhError> {
        self.it.next().ok_or_else(|| BvhError::Parse("fim inesperado do arquivo".into()))
    }
    fn expect(&mut self, what: &str) -> Result<(), BvhError> {
        let t = self.next()?;
        if t.eq_ignore_ascii_case(what) {
            Ok(())
        } else {
            err(format!("esperava '{what}', achei '{t}'"))
        }
    }
    fn float(&mut self) -> Result<f32, BvhError> {
        let t = self.next()?;
        let v: f32 = t.parse().map_err(|_| BvhError::Parse(format!("número inválido '{t}'")))?;
        if !v.is_finite() {
            return err(format!("número não finito '{t}'"));
        }
        Ok(v)
    }
}

/// Lê um arquivo BVH.
pub fn parse_bvh(text: &str) -> Result<BvhMotion, BvhError> {
    if text.len() > BVH_MAX_BYTES {
        return Err(BvhError::TooLarge(text.len()));
    }
    let mut t = Tokens { it: text.split_whitespace().peekable() };
    t.expect("HIERARCHY")?;
    let mut joints: Vec<SrcJoint> = Vec::new();
    let mut channels: Vec<Vec<Chan>> = Vec::new();
    let kind = t.next()?;
    if !kind.eq_ignore_ascii_case("ROOT") {
        return err("a hierarquia começa com ROOT");
    }
    parse_joint(&mut t, None, &mut joints, &mut channels, 0)?;
    // ROOTs adicionais (raros): lê enquanto vier ROOT
    while t.it.peek().is_some_and(|p| p.eq_ignore_ascii_case("ROOT")) {
        t.next()?;
        parse_joint(&mut t, None, &mut joints, &mut channels, 0)?;
    }
    t.expect("MOTION")?;
    t.expect("Frames:")?;
    let n_frames: usize = t.next()?.parse().map_err(|_| BvhError::Parse("Frames inválido".into()))?;
    if n_frames > BVH_MAX_FRAMES {
        return err(format!("quadros demais ({n_frames})"));
    }
    t.expect("Frame")?;
    t.expect("Time:")?;
    let frame_time = t.float()?;
    if frame_time <= 0.0 {
        return err("Frame Time precisa ser positivo");
    }
    let fps = (1.0 / frame_time).clamp(1.0, 1000.0);
    let total_channels: usize = channels.iter().map(Vec::len).sum();
    let mut frames = Vec::with_capacity(n_frames);
    for _ in 0..n_frames {
        let mut f = SrcFrame { rot: joints.iter().map(|j| j.rest_r).collect(), pos: BTreeMap::new() };
        for (ji, chans) in channels.iter().enumerate() {
            if chans.is_empty() {
                continue;
            }
            let mut p = joints[ji].rest_t;
            let mut has_pos = false;
            let mut q = Quat::IDENTITY;
            for c in chans {
                let v = t.float()?;
                match c {
                    Chan::Xpos => {
                        p.x = v;
                        has_pos = true;
                    }
                    Chan::Ypos => {
                        p.y = v;
                        has_pos = true;
                    }
                    Chan::Zpos => {
                        p.z = v;
                        has_pos = true;
                    }
                    Chan::Xrot => q *= Quat::from_rotation_x(v.to_radians()),
                    Chan::Yrot => q *= Quat::from_rotation_y(v.to_radians()),
                    Chan::Zrot => q *= Quat::from_rotation_z(v.to_radians()),
                }
            }
            if chans.iter().any(|c| matches!(c, Chan::Xrot | Chan::Yrot | Chan::Zrot)) {
                f.rot[ji] = q.normalize();
            }
            if has_pos {
                f.pos.insert(ji, p);
            }
        }
        let _ = total_channels;
        frames.push(f);
    }
    let skeleton = SrcSkeleton { joints };
    skeleton.validate().map_err(BvhError::Parse)?;
    Ok(BvhMotion { skeleton, frames, fps })
}

fn parse_joint(t: &mut Tokens<'_>, parent: Option<usize>, joints: &mut Vec<SrcJoint>, channels: &mut Vec<Vec<Chan>>, depth: usize) -> Result<(), BvhError> {
    if depth > 64 {
        return err("hierarquia profunda demais");
    }
    if joints.len() >= BVH_MAX_JOINTS {
        return err("juntas demais");
    }
    let name = t.next()?.to_string();
    t.expect("{")?;
    t.expect("OFFSET")?;
    let offset = Vec3::new(t.float()?, t.float()?, t.float()?);
    let me = joints.len();
    joints.push(SrcJoint { name, parent, rest_t: offset, rest_r: Quat::IDENTITY });
    channels.push(Vec::new());
    loop {
        let tok = t.next()?;
        if tok.eq_ignore_ascii_case("CHANNELS") {
            let n: usize = t.next()?.parse().map_err(|_| BvhError::Parse("CHANNELS inválido".into()))?;
            if n > 6 {
                return err("CHANNELS > 6");
            }
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                let c = t.next()?;
                list.push(match c.to_ascii_lowercase().as_str() {
                    "xposition" => Chan::Xpos,
                    "yposition" => Chan::Ypos,
                    "zposition" => Chan::Zpos,
                    "xrotation" => Chan::Xrot,
                    "yrotation" => Chan::Yrot,
                    "zrotation" => Chan::Zrot,
                    other => return err(format!("canal desconhecido '{other}'")),
                });
            }
            channels[me] = list;
        } else if tok.eq_ignore_ascii_case("JOINT") {
            parse_joint(t, Some(me), joints, channels, depth + 1)?;
        } else if tok.eq_ignore_ascii_case("End") {
            t.expect("Site")?;
            t.expect("{")?;
            t.expect("OFFSET")?;
            let _ = (t.float()?, t.float()?, t.float()?);
            t.expect("}")?;
        } else if tok == "}" {
            return Ok(());
        } else {
            return err(format!("token inesperado '{tok}'"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Escrita
// ---------------------------------------------------------------------------------------------

/// Escreve o clip (canais de corpo) como BVH, amostrado a cada quadro da faixa. Centímetros, Y para cima, frente +Z,
/// esquerda do personagem +X; ordem de canais `Zrotation Xrotation Yrotation` (quadril também com posição).
pub fn write_bvh(clip: &AnimationClip, fps: f32, rig: &PoseRig) -> String {
    const CM: f32 = 100.0;
    let (start, end) = clip.effective_range(fps);
    // hierarquia a partir do Hips (o Root canônico não é escrito)
    let mut out = String::new();
    out.push_str("HIERARCHY\n");
    fn children(b: usize) -> Vec<usize> {
        (0..BONE_PARENTS.len()).filter(|i| BONE_PARENTS[*i] == Some(b) && *i != bone::ROOT).collect()
    }
    fn end_offset(b: usize) -> Option<Vec3> {
        match b {
            bone::HEAD => Some(Vec3::new(0.0, 0.2, 0.0)),
            bone::L_WRIST | bone::R_WRIST => Some(Vec3::new(0.0, -0.09, 0.0)),
            bone::L_TOES | bone::R_TOES => Some(Vec3::new(0.0, 0.0, 0.05)),
            bone::PELVIS => Some(Vec3::new(0.0, -0.05, 0.0)),
            _ => None,
        }
    }
    let mut order: Vec<usize> = Vec::new();
    fn write_joint(out: &mut String, rig: &PoseRig, b: usize, depth: usize, order: &mut Vec<usize>) {
        let pad = "  ".repeat(depth);
        let off = if b == bone::HIPS { rig.bind[b] } else { rig.offset(b) } * 100.0;
        let kind = if depth == 0 { "ROOT" } else { "JOINT" };
        let _ = writeln!(out, "{pad}{kind} {}\n{pad}{{", BONE_NAMES[b]);
        let _ = writeln!(out, "{pad}  OFFSET {:.5} {:.5} {:.5}", off.x, off.y, off.z);
        if depth == 0 {
            let _ = writeln!(out, "{pad}  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation");
        } else {
            let _ = writeln!(out, "{pad}  CHANNELS 3 Zrotation Xrotation Yrotation");
        }
        order.push(b);
        let kids = children(b);
        for k in &kids {
            write_joint(out, rig, *k, depth + 1, order);
        }
        if kids.is_empty() {
            if let Some(e) = end_offset(b) {
                let e = e * 100.0;
                let _ = writeln!(out, "{pad}  End Site\n{pad}  {{\n{pad}    OFFSET {:.5} {:.5} {:.5}\n{pad}  }}", e.x, e.y, e.z);
            }
        }
        let _ = writeln!(out, "{pad}}}");
    }
    write_joint(&mut out, rig, bone::HIPS, 0, &mut order);
    let frames = (end - start + 1).max(1);
    let _ = writeln!(out, "MOTION\nFrames: {frames}\nFrame Time: {:.7}", 1.0 / fps.max(1.0));
    for f in start..=end.max(start) {
        let pose = clip.evaluate(fps, f as f32).apply_to_pose(&crate::pose::PoseState::default());
        let mut vals: Vec<f32> = Vec::new();
        for (i, b) in order.iter().enumerate() {
            if i == 0 {
                let p = rig.bind[bone::HIPS] + Vec3::from_array(pose.hips_translation);
                vals.extend([p.x * CM, p.y * CM, p.z * CM]);
            }
            let (z, x, y) = pose.rotation(*b).to_euler(EulerRot::ZXY);
            vals.extend([z.to_degrees(), x.to_degrees(), y.to_degrees()]);
        }
        let line: Vec<String> = vals.iter().map(|v| format!("{v:.5}")).collect();
        let _ = writeln!(out, "{}", line.join(" "));
    }
    out
}

/// Canais de um clip que o exportador BVH escreve (para o aviso "x canais ficaram de fora").
pub fn bvh_exportable(target: &ChannelTarget) -> bool {
    match target {
        ChannelTarget::Bone { bone } => bone != "Root",
        ChannelTarget::Hips => true,
        _ => false,
    }
}

/// `true` se o valor de um canal é de rotação ou translação do quadril.
pub fn is_motion_value(v: &KeyValue) -> bool {
    matches!(v, KeyValue::Quat(_) | KeyValue::Vec3(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{canonicalize_clip, Channel, Key};
    use crate::ids::AnimationClipId;
    use crate::retarget::{BoneMap, Retargeter};

    const SAMPLE: &str = "HIERARCHY
ROOT Hips
{
  OFFSET 0 100 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation
  JOINT Spine
  {
    OFFSET 0 10 0
    CHANNELS 3 Zrotation Xrotation Yrotation
    End Site
    {
      OFFSET 0 10 0
    }
  }
}
MOTION
Frames: 3
Frame Time: 0.0416667
0 100 0  0 0 0  0 0 0
0 101 5  0 0 0  90 0 0
2 102 0  0 0 0  0 0 90
";

    #[test]
    fn parses_hierarchy_channels_and_motion() {
        let m = parse_bvh(SAMPLE).unwrap();
        assert_eq!(m.skeleton.joints.len(), 2, "End Site não é junta");
        assert_eq!(m.skeleton.joints[1].parent, Some(0));
        assert_eq!(m.skeleton.joints[0].rest_t, Vec3::new(0.0, 100.0, 0.0));
        assert_eq!(m.frames.len(), 3);
        assert!((m.fps - 24.0).abs() < 0.01);
        // posição absoluta do quadril por quadro
        assert_eq!(m.frames[1].pos[&0], Vec3::new(0.0, 101.0, 5.0));
        // Zrotation 90 na coluna
        let q = m.frames[1].rot[1];
        assert!(q.angle_between(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)) < 1e-3);
        let q = m.frames[2].rot[1];
        assert!(q.angle_between(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)) < 1e-3);
    }

    #[test]
    fn rotation_order_follows_the_channel_list() {
        // Zrotation Xrotation Yrotation: R = Rz * Rx * Ry
        let text = SAMPLE.replace("0 0 0  90 0 0", "0 0 0  30 40 50");
        let m = parse_bvh(&text).unwrap();
        let expect = Quat::from_rotation_z(30f32.to_radians()) * Quat::from_rotation_x(40f32.to_radians()) * Quat::from_rotation_y(50f32.to_radians());
        assert!(m.frames[1].rot[1].angle_between(expect) < 1e-3);
        // ordem diferente: Yrotation Xrotation
        let alt = SAMPLE.replace("JOINT Spine\n  {\n    OFFSET 0 10 0\n    CHANNELS 3 Zrotation Xrotation Yrotation", "JOINT Spine\n  {\n    OFFSET 0 10 0\n    CHANNELS 3 Yrotation Xrotation Zrotation").replace("0 0 0  90 0 0", "0 0 0  30 40 50");
        let m = parse_bvh(&alt).unwrap();
        let expect = Quat::from_rotation_y(30f32.to_radians()) * Quat::from_rotation_x(40f32.to_radians()) * Quat::from_rotation_z(50f32.to_radians());
        assert!(m.frames[1].rot[1].angle_between(expect) < 1e-3, "{:?} vs {:?} / {:?}", m.frames[1].rot, expect, m.skeleton);
    }

    #[test]
    fn malformed_and_hostile_files_are_rejected_without_panicking() {
        for bad in [
            "",
            "HIERARCHY",
            "HIERARCHY ROOT A {",
            "HIERARCHY ROOT A { OFFSET 0 0 x }",
            "HIERARCHY ROOT A { OFFSET 0 0 0 CHANNELS 3 Foo Bar Baz } MOTION Frames: 1 Frame Time: 0.1 0 0 0",
            "HIERARCHY ROOT A { OFFSET 0 0 0 CHANNELS 3 Xrotation Yrotation Zrotation } MOTION Frames: 2 Frame Time: 0.1 1 2 3",
            "HIERARCHY ROOT A { OFFSET 0 0 0 CHANNELS 3 Xrotation Yrotation Zrotation } MOTION Frames: 1 Frame Time: 0 1 2 3",
            "HIERARCHY ROOT A { OFFSET 0 0 0 CHANNELS 7 a b c d e f g }",
            "HIERARCHY ROOT A { OFFSET 0 0 0 CHANNELS 3 Xrotation Yrotation Zrotation } MOTION Frames: 999999999 Frame Time: 0.1",
            "HIERARCHY ROOT A { OFFSET 0 0 nan CHANNELS 0 } MOTION Frames: 0 Frame Time: 0.1",
        ] {
            assert!(parse_bvh(bad).is_err(), "{bad:?}");
        }
        let deep = format!("HIERARCHY ROOT A {{ OFFSET 0 0 0 {} {} MOTION Frames: 0 Frame Time: 0.1", "JOINT B { OFFSET 0 0 0 ".repeat(100), "}".repeat(100));
        assert!(parse_bvh(&deep).is_err());
        assert!(matches!(parse_bvh(&" ".repeat(BVH_MAX_BYTES + 1)), Err(BvhError::TooLarge(_))));
    }

    fn walking_clip() -> AnimationClip {
        let mut c = AnimationClip::new(AnimationClipId::from_slug("w"), "w");
        c.range = Some([0, 12]);
        let q = |x: f32, y: f32| KeyValue::Quat((Quat::from_rotation_y(y) * Quat::from_rotation_x(x)).to_array());
        c.channels = vec![
            Channel { target: ChannelTarget::bone("LeftThigh"), keys: vec![Key::new(0, q(0.6, 0.1)), Key::new(6, q(-0.5, 0.0)), Key::new(12, q(0.6, 0.1))] },
            Channel { target: ChannelTarget::bone("LeftShoulder"), keys: vec![Key::new(0, q(-0.8, 0.0)), Key::new(12, q(0.4, 0.3))] },
            Channel { target: ChannelTarget::Hips, keys: vec![Key::new(0, KeyValue::Vec3([0.0, 0.0, 0.0])), Key::new(12, KeyValue::Vec3([0.1, -0.05, 0.6]))] },
        ];
        canonicalize_clip(&mut c);
        c
    }

    #[test]
    fn exporting_a_clip_and_importing_it_back_reproduces_the_motion() {
        let rig = PoseRig::default();
        let clip = walking_clip();
        let text = write_bvh(&clip, 24.0, &rig);
        assert!(text.starts_with("HIERARCHY\nROOT Hips"));
        let m = parse_bvh(&text).unwrap();
        assert_eq!(m.frames.len(), 13);
        assert!((m.fps - 24.0).abs() < 0.01);
        let map = BoneMap::from_names(&m.skeleton);
        assert_eq!(map.mapped_count(), BONE_NAMES.len() - 1, "tudo menos a raiz (não escrita)");
        let rt = Retargeter::new(&m.skeleton, &map, &rig).unwrap();
        // a escala do quadril compensa as unidades (cm -> m)
        for f in [0usize, 3, 6, 9, 12] {
            let expect = clip.evaluate(24.0, f as f32).apply_to_pose(&crate::pose::PoseState::default());
            let got = rt.pose(&m.frames[f]);
            for b in [bone::L_THIGH, bone::L_SHOULDER, bone::HEAD] {
                let a = got.rotation(b).angle_between(expect.rotation(b)).to_degrees();
                assert!(a < 0.2, "quadro {f} {}: {a}°", BONE_NAMES[b]);
            }
            let d = Vec3::from_array(got.hips_translation).distance(Vec3::from_array(expect.hips_translation));
            assert!(d < 0.01, "quadro {f}: translação do quadril {d}");
        }
    }

    #[test]
    fn export_lists_only_body_channels() {
        assert!(bvh_exportable(&ChannelTarget::bone("Head")));
        assert!(bvh_exportable(&ChannelTarget::Hips));
        assert!(!bvh_exportable(&ChannelTarget::Sun { prop: crate::animation::SunProp::Color }));
        assert!(!bvh_exportable(&ChannelTarget::Hand { side: crate::animation::HandSide::Left }));
        assert!(is_motion_value(&KeyValue::Quat([0.0, 0.0, 0.0, 1.0])));
    }
}
