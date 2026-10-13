//! Auto-key: transforma um `SetPose` (edição do Posing) em `Batch[SetPose, EditAnimation(InsertKeys)]` — a pose e as
//! chaves entram no histórico como UM passo. Só os canais que o comando tocou ganham chave, no quadro atual, com a
//! interpolação padrão (as chaves que já existem mantêm a sua). Sem clip ativo, o primeiro auto-key cria um.

use anigo_core::animation::{AnimEdit, ChannelTarget, HandSide, Interp, Key, KeyInsert, KeyValue};
use anigo_core::command::Command;
use anigo_core::ids::AnimationClipId;
use anigo_core::pose::{bone_index, HandsPose};
use anigo_core::project::{AnimationClip, ProjectState};

/// Chaves que o `SetPose` `cmd` implica (valores FINAIS que o comando grava), ignorando o que o clip já tem igual.
pub fn keys_of_pose_command(project: &ProjectState, clip: Option<&AnimationClip>, cmd: &Command, frame: i32, interp: Interp) -> Vec<KeyInsert> {
    let Command::SetPose { bones, root_translation, hips_translation, hands, .. } = cmd else { return Vec::new() };
    let mut out: Vec<KeyInsert> = Vec::new();
    let mut push = |target: ChannelTarget, value: KeyValue| {
        let existing = clip.and_then(|c| c.channel(&target)).and_then(|c| c.key_at(frame));
        let key_interp = existing.map(|k| k.interp).unwrap_or(interp);
        if existing.is_some_and(|k| same(&k.value, &value)) {
            return;
        }
        out.push(KeyInsert { target, key: Key { frame, value, interp: key_interp } });
    };
    for e in bones {
        if bone_index(&e.bone).is_some() {
            push(ChannelTarget::bone(&e.bone), KeyValue::Quat(e.rotation).normalized());
        }
    }
    if let Some(t) = root_translation {
        push(ChannelTarget::Root, KeyValue::Vec3(*t));
    }
    if let Some(t) = hips_translation {
        push(ChannelTarget::Hips, KeyValue::Vec3(*t));
    }
    if let Some(h) = hands {
        let cur = project.character.pose.hands;
        let side = |s: HandSide, new: [f32; 5], old: [f32; 5], push: &mut dyn FnMut(ChannelTarget, KeyValue)| {
            let keyed = clip.and_then(|c| c.channel(&ChannelTarget::Hand { side: s })).is_some();
            if new != old || keyed {
                push(ChannelTarget::Hand { side: s }, KeyValue::Curl(new));
            }
        };
        let HandsPose { left, right } = *h;
        side(HandSide::Left, left, cur.left, &mut push);
        side(HandSide::Right, right, cur.right, &mut push);
    }
    out
}

fn same(a: &KeyValue, b: &KeyValue) -> bool {
    match (a, b) {
        (KeyValue::Quat(x), KeyValue::Quat(y)) => x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>().abs() >= 1.0 - 1e-6,
        _ => a == b,
    }
}

/// Id novo e livre para um clip criado pelo auto-key.
pub fn fresh_clip_id(project: &ProjectState, name: &str) -> AnimationClipId {
    let base = AnimationClipId::from_label(name);
    if !project.animation.clips.contains_key(&base) {
        return base;
    }
    for n in 2..1000 {
        let id = AnimationClipId::from_label(&format!("{name} {n}"));
        if !project.animation.clips.contains_key(&id) {
            return id;
        }
    }
    base
}

/// Embrulha `cmd` (um `SetPose`) com as chaves do auto-key. Devolve `cmd` intacto quando não há nada a gravar
/// (comando que não é pose, ou chaves idênticas às que já existem).
pub fn wrap_pose_command(project: &ProjectState, cmd: Command, frame: i32, interp: Interp, new_clip_name: &str) -> Command {
    if !matches!(cmd, Command::SetPose { .. }) {
        return cmd;
    }
    let active = project.animation.active_clip.clone().filter(|id| project.animation.clips.contains_key(id));
    let clip = active.as_ref().and_then(|id| project.animation.clips.get(id));
    let keys = keys_of_pose_command(project, clip, &cmd, frame, interp);
    if keys.is_empty() {
        return cmd;
    }
    let mut commands = Vec::new();
    let id = match active {
        Some(id) => id,
        None => {
            let id = fresh_clip_id(project, new_clip_name);
            let mut clip = AnimationClip::new(id.clone(), new_clip_name);
            clip.duration_seconds = 0.0;
            commands.push(Command::EditAnimation { edit: AnimEdit::PutClip { clip } });
            commands.push(Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(id.clone()) } });
            id
        }
    };
    commands.push(cmd);
    commands.push(Command::EditAnimation { edit: AnimEdit::InsertKeys { clip_id: id, keys } });
    Command::Batch { commands }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::CommandHistory;
    use anigo_core::pose::{bone, PoseBoneEntry, PoseOp};
    use glam::Quat;

    fn set_pose(bone: &str, deg: f32) -> Command {
        Command::SetPose {
            bones: vec![PoseBoneEntry { bone: bone.into(), rotation: Quat::from_rotation_x(deg.to_radians()).to_array() }],
            root_translation: None,
            hips_translation: None,
            hands: None,
            op: PoseOp::Edit,
        }
    }

    #[test]
    fn first_autokey_creates_the_clip_and_everything_is_one_undo_step() {
        let mut p = ProjectState::default();
        let mut h = CommandHistory::new(50);
        let cmd = wrap_pose_command(&p, set_pose("LeftElbow", -70.0), 12, Interp::Linear, "Clip 1");
        let Command::Batch { ref commands } = cmd else { panic!("{cmd:?}") };
        assert_eq!(commands.len(), 4, "criar clip, ativar, pose, chaves");
        h.execute(&mut p, cmd).unwrap();
        assert_eq!(h.undo_depth(), 1);
        let id = p.animation.active_clip.clone().unwrap();
        let clip = &p.animation.clips[&id];
        assert_eq!(clip.name, "Clip 1");
        assert_eq!(clip.channel(&ChannelTarget::bone("LeftElbow")).unwrap().keys[0].frame, 12);
        assert!(!p.character.pose.is_rest());
        h.undo(&mut p).unwrap();
        assert!(p.character.pose.is_rest() && p.animation.clips.is_empty() && p.animation.active_clip.is_none());
        h.redo(&mut p).unwrap();
        assert_eq!(p.animation.clips[&id].key_count(), 1);
    }

    #[test]
    fn later_edits_reuse_the_active_clip_and_only_key_the_touched_channels() {
        let mut p = ProjectState::default();
        let mut h = CommandHistory::new(50);
        let first = wrap_pose_command(&p, set_pose("LeftElbow", -70.0), 0, Interp::Linear, "Clip 1");
        h.execute(&mut p, first).unwrap();
        let cmd = wrap_pose_command(&p, set_pose("Head", 20.0), 24, Interp::Ease, "Clip 1");
        let Command::Batch { ref commands } = cmd else { panic!() };
        assert_eq!(commands.len(), 2, "pose + chaves");
        h.execute(&mut p, cmd).unwrap();
        let clip = &p.animation.clips[p.animation.active_clip.as_ref().unwrap()];
        assert_eq!(clip.channels.len(), 2);
        assert_eq!(clip.channel(&ChannelTarget::bone("Head")).unwrap().keys[0].interp, Interp::Ease);
        assert!(clip.channel(&ChannelTarget::bone("RightKnee")).is_none());
        // editar de novo o MESMO osso no MESMO quadro mantém a interpolação da chave
        let cmd = wrap_pose_command(&p, set_pose("Head", 35.0), 24, Interp::Step, "Clip 1");
        h.execute(&mut p, cmd).unwrap();
        let clip = &p.animation.clips[p.animation.active_clip.as_ref().unwrap()];
        assert_eq!(clip.channel(&ChannelTarget::bone("Head")).unwrap().keys[0].interp, Interp::Ease);
    }

    #[test]
    fn translation_and_hands_become_their_own_channels_and_unchanged_hands_are_skipped() {
        let p = ProjectState::default();
        let cmd = Command::SetPose {
            bones: vec![],
            root_translation: Some([0.0, 0.0, 1.0]),
            hips_translation: Some([0.0, -0.1, 0.0]),
            hands: Some(HandsPose { left: [1.0; 5], right: [0.0; 5] }),
            op: PoseOp::Edit,
        };
        let keys = keys_of_pose_command(&p, None, &cmd, 3, Interp::Linear);
        let targets: Vec<_> = keys.iter().map(|k| k.target.clone()).collect();
        assert!(targets.contains(&ChannelTarget::Root) && targets.contains(&ChannelTarget::Hips) && targets.contains(&ChannelTarget::Hand { side: HandSide::Left }));
        assert!(!targets.contains(&ChannelTarget::Hand { side: HandSide::Right }), "a direita não mudou e não tem canal");
    }

    #[test]
    fn nothing_to_key_returns_the_command_untouched() {
        let p = ProjectState::default();
        // não é pose
        let rename = Command::RenameProject { name: "x".into() };
        assert_eq!(wrap_pose_command(&p, rename.clone(), 0, Interp::Linear, "c"), rename);
        // pose sem ossos conhecidos
        let cmd = set_pose("Tail", 10.0);
        assert_eq!(wrap_pose_command(&p, cmd.clone(), 0, Interp::Linear, "c"), cmd);
    }

    #[test]
    fn fresh_ids_do_not_collide() {
        let mut p = ProjectState::default();
        let a = fresh_clip_id(&p, "Clip 1");
        p.animation.clips.insert(a.clone(), AnimationClip::new(a.clone(), "Clip 1"));
        let b = fresh_clip_id(&p, "Clip 1");
        assert_ne!(a, b);
        let _ = bone::HIPS;
    }
}
