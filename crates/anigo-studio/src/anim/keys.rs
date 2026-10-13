//! Planejadores PUROS de edição de chaves: dada uma seleção de chaves e um gesto (mover, escalar no tempo, duplicar,
//! espelhar, apagar, colar, inserir pose), devolvem a `AnimEdit` que o núcleo valida e inverte. Nada aqui toca o projeto.

use std::collections::{BTreeMap, BTreeSet};

use anigo_core::animation::{
    current_value, mirror_key, AnimEdit, ChannelTarget, Interp, Key, KeyInsert, KeyMove, KeyRef, KeyValue, MAX_FRAME,
};
use anigo_core::ids::AnimationClipId;
use anigo_core::project::{AnimationClip, ProjectState};

/// Identidade de uma chave na UI: canal + quadro.
pub type KeyId = (ChannelTarget, i32);

/// Chaves selecionadas que ainda existem no clip (descarta o que sumiu num undo).
pub fn existing(clip: &AnimationClip, sel: &BTreeSet<KeyId>) -> Vec<KeyId> {
    sel.iter().filter(|(t, f)| clip.channel(t).is_some_and(|c| c.key_at(*f).is_some())).cloned().collect()
}

fn refs(ids: &[KeyId]) -> Vec<KeyRef> {
    ids.iter().map(|(t, f)| KeyRef { target: t.clone(), frame: *f }).collect()
}

/// Mover as chaves `delta` quadros. O deslocamento é limitado para nenhuma chave sair de `0..=MAX_FRAME`.
/// Devolve a edição e o deslocamento realmente aplicado.
pub fn plan_move(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>, delta: i32) -> Option<(AnimEdit, i32)> {
    let ids = existing(clip, sel);
    let lo = ids.iter().map(|(_, f)| *f).min()?;
    let hi = ids.iter().map(|(_, f)| *f).max()?;
    let delta = delta.clamp(-lo, MAX_FRAME - hi);
    if delta == 0 {
        return None;
    }
    let moves = ids.iter().map(|(t, f)| KeyMove { target: t.clone(), frame: *f, to_frame: f + delta }).collect();
    Some((AnimEdit::MoveKeys { clip_id: clip_id.clone(), moves }, delta))
}

/// Escalar o tempo das chaves em torno de `pivot` (quadro) pelo fator `factor` (> 0). Dentro de um canal a ordem é
/// preservada e os quadros ficam estritamente crescentes (colisões de arredondamento avançam um quadro).
pub fn plan_scale(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>, pivot: f32, factor: f32) -> Option<AnimEdit> {
    if !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    let ids = existing(clip, sel);
    let mut per: BTreeMap<ChannelTarget, Vec<i32>> = BTreeMap::new();
    for (t, f) in ids {
        per.entry(t).or_default().push(f);
    }
    let mut moves = Vec::new();
    for (target, frames) in per {
        let mut prev: Option<i32> = None;
        // do menor para o maior: garante crescimento estrito; se a escala é < 1 as colisões avançam
        for f in frames {
            let mut to = (pivot + (f as f32 - pivot) * factor).round() as i32;
            if let Some(p) = prev {
                to = to.max(p + 1);
            }
            let to = to.clamp(0, MAX_FRAME);
            prev = Some(to);
            if to != f {
                moves.push(KeyMove { target: target.clone(), frame: f, to_frame: to });
            }
        }
    }
    if moves.is_empty() {
        return None;
    }
    // destinos distintos por canal (o clamp em MAX_FRAME pode repetir): descarta o excesso
    let mut seen = BTreeSet::new();
    moves.retain(|m| seen.insert((m.target.clone(), m.to_frame)));
    Some(AnimEdit::MoveKeys { clip_id: clip_id.clone(), moves })
}

/// Duplicar as chaves `delta` quadros adiante (substitui o que já estiver no destino).
pub fn plan_duplicate(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>, delta: i32) -> Option<AnimEdit> {
    let ids = existing(clip, sel);
    let mut keys = Vec::new();
    for (t, f) in ids {
        let k = *clip.channel(&t)?.key_at(f)?;
        let to = f + delta;
        if !(0..=MAX_FRAME).contains(&to) {
            continue;
        }
        keys.push(KeyInsert { target: t, key: Key { frame: to, ..k } });
    }
    if keys.is_empty() {
        None
    } else {
        Some(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys })
    }
}

/// Espelhar: grava o reflexo (X) de cada chave selecionada na chave do lado oposto, no mesmo quadro. Calculado a
/// partir dos valores ORIGINAIS (selecionar os dois lados troca os lados).
pub fn plan_mirror(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>) -> Option<AnimEdit> {
    let mut out: BTreeMap<KeyId, KeyInsert> = BTreeMap::new();
    for (t, f) in existing(clip, sel) {
        let key = *clip.channel(&t)?.key_at(f)?;
        if let Some((to, mirrored)) = mirror_key(&t, &key) {
            out.insert((to.clone(), f), KeyInsert { target: to, key: mirrored });
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys: out.into_values().collect() })
    }
}

pub fn plan_delete(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>) -> Option<AnimEdit> {
    let ids = existing(clip, sel);
    if ids.is_empty() {
        None
    } else {
        Some(AnimEdit::DeleteKeys { clip_id: clip_id.clone(), keys: refs(&ids) })
    }
}

pub fn plan_interp(clip_id: &AnimationClipId, clip: &AnimationClip, sel: &BTreeSet<KeyId>, interp: Interp) -> Option<AnimEdit> {
    // só as chaves que ainda não têm essa interpolação
    let ids: Vec<KeyId> = existing(clip, sel).into_iter().filter(|(t, f)| clip.channel(t).and_then(|c| c.key_at(*f)).is_some_and(|k| k.interp != interp)).collect();
    if ids.is_empty() {
        None
    } else {
        Some(AnimEdit::SetInterpolation { clip_id: clip_id.clone(), keys: refs(&ids), interp })
    }
}

/// Insere chaves dos `targets` no quadro `frame` com o valor VIVO do projeto. Chaves que já existem com o mesmo valor
/// são puladas; as que existem com valor diferente mantêm a interpolação que tinham.
pub fn plan_insert(project: &ProjectState, clip_id: &AnimationClipId, clip: Option<&AnimationClip>, frame: i32, targets: &[ChannelTarget], interp: Interp) -> Option<AnimEdit> {
    let mut keys = Vec::new();
    for t in targets {
        let Some(value) = current_value(project, t) else { continue };
        let existing = clip.and_then(|c| c.channel(t)).and_then(|c| c.key_at(frame));
        match existing {
            Some(k) if values_equal(&k.value, &value) => continue,
            Some(k) => keys.push(KeyInsert { target: t.clone(), key: Key { frame, value, interp: k.interp } }),
            None => keys.push(KeyInsert { target: t.clone(), key: Key { frame, value, interp } }),
        }
    }
    if keys.is_empty() {
        None
    } else {
        Some(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys })
    }
}

/// Apaga as chaves dos `targets` no quadro `frame` (Alt+I).
pub fn plan_delete_at(clip_id: &AnimationClipId, clip: &AnimationClip, frame: i32, targets: &[ChannelTarget]) -> Option<AnimEdit> {
    let keys: Vec<KeyRef> = targets.iter().filter(|t| clip.channel(t).is_some_and(|c| c.key_at(frame).is_some())).map(|t| KeyRef { target: t.clone(), frame }).collect();
    if keys.is_empty() {
        None
    } else {
        Some(AnimEdit::DeleteKeys { clip_id: clip_id.clone(), keys })
    }
}

fn values_equal(a: &KeyValue, b: &KeyValue) -> bool {
    match (a, b) {
        (KeyValue::Quat(x), KeyValue::Quat(y)) => {
            // q e −q são a mesma rotação
            let d: f32 = x.iter().zip(y).map(|(p, q)| p * q).sum();
            d.abs() >= 1.0 - 1e-6
        }
        _ => a == b,
    }
}

/// Área de transferência de chaves: deslocamentos relativos à primeira chave copiada.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyClipboard {
    pub keys: Vec<(ChannelTarget, i32, Key)>,
}

impl KeyClipboard {
    pub fn copy(clip: &AnimationClip, sel: &BTreeSet<KeyId>) -> KeyClipboard {
        let ids = existing(clip, sel);
        let first = ids.iter().map(|(_, f)| *f).min().unwrap_or(0);
        let keys = ids.into_iter().filter_map(|(t, f)| clip.channel(&t).and_then(|c| c.key_at(f)).map(|k| (t, f - first, *k))).collect();
        KeyClipboard { keys }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Colar com a primeira chave em `at` (espelhado = reflexo X nos canais espelháveis).
    pub fn paste(&self, clip_id: &AnimationClipId, at: i32, mirrored: bool) -> Option<AnimEdit> {
        let mut out: BTreeMap<KeyId, KeyInsert> = BTreeMap::new();
        for (t, rel, k) in &self.keys {
            let frame = at + rel;
            if !(0..=MAX_FRAME).contains(&frame) {
                continue;
            }
            let key = Key { frame, ..*k };
            let (target, key) = if mirrored { mirror_key(t, &key).unwrap_or((t.clone(), key)) } else { (t.clone(), key) };
            out.insert((target.clone(), frame), KeyInsert { target, key });
        }
        if out.is_empty() {
            None
        } else {
            Some(AnimEdit::InsertKeys { clip_id: clip_id.clone(), keys: out.into_values().collect() })
        }
    }
}

/// Arredonda `frame` para o múltiplo de `step` a partir de `origin` ("animar em 2s/3s": chaves nos pares/múltiplos).
pub fn snap_to_step(frame: i32, origin: i32, step: i32) -> i32 {
    let step = step.max(1);
    if step == 1 {
        return frame;
    }
    origin + ((frame - origin) as f32 / step as f32).round() as i32 * step
}

/// Seleciona todas as chaves de um quadro (clique numa chave do resumo/cabeçalho de grupo).
pub fn keys_at_frame(clip: &AnimationClip, frame: i32, filter: impl Fn(&ChannelTarget) -> bool) -> BTreeSet<KeyId> {
    clip.channels.iter().filter(|c| filter(&c.target) && c.key_at(frame).is_some()).map(|c| (c.target.clone(), frame)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{canonicalize_clip, Channel, HandSide};
    use anigo_core::command::Command;

    fn id() -> AnimationClipId {
        AnimationClipId::from_slug("k")
    }

    fn quat(deg: f32) -> KeyValue {
        KeyValue::Quat(glam::Quat::from_rotation_x(deg.to_radians()).to_array())
    }

    fn project() -> ProjectState {
        let mut p = ProjectState::default();
        let mut c = AnimationClip::new(id(), "k");
        c.range = Some([0, 30]);
        c.channels = vec![
            Channel { target: ChannelTarget::bone("LeftShoulder"), keys: vec![Key::new(0, quat(0.0)), Key::new(10, quat(30.0)), Key::new(20, quat(60.0))] },
            Channel { target: ChannelTarget::bone("RightShoulder"), keys: vec![Key::new(10, quat(-30.0))] },
            Channel { target: ChannelTarget::Hips, keys: vec![Key::new(0, KeyValue::Vec3([0.0; 3])), Key::new(20, KeyValue::Vec3([0.5, 0.0, 0.0]))] },
        ];
        canonicalize_clip(&mut c);
        p.animation.clips.insert(c.clip_id.clone(), c);
        p
    }

    fn sel(items: &[(ChannelTarget, i32)]) -> BTreeSet<KeyId> {
        items.iter().cloned().collect()
    }

    fn run(p: &mut ProjectState, e: AnimEdit) -> anigo_core::command::AppliedCommand {
        Command::EditAnimation { edit: e }.apply(p).unwrap()
    }

    #[test]
    fn move_shifts_selected_keys_clamps_at_zero_and_is_undoable() {
        let mut p = project();
        let l = ChannelTarget::bone("LeftShoulder");
        let s = sel(&[(l.clone(), 10), (l.clone(), 20)]);
        let (edit, applied) = plan_move(&id(), &p.animation.clips[&id()], &s, 5).unwrap();
        assert_eq!(applied, 5);
        let before = p.clone();
        let a = run(&mut p, edit);
        let frames: Vec<i32> = p.animation.clips[&id()].channel(&l).unwrap().keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 15, 25]);
        a.inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
        // não passa de zero: a chave do quadro 0 não pode ir para trás (nada a fazer)
        let s0 = sel(&[(l.clone(), 0), (l.clone(), 10)]);
        assert!(plan_move(&id(), &p.animation.clips[&id()], &s0, -7).is_none());
        let s1 = sel(&[(l.clone(), 10), (l.clone(), 20)]);
        assert_eq!(plan_move(&id(), &p.animation.clips[&id()], &s1, -50).unwrap().1, -10, "limitado ao que cabe");
        // mover sobre uma chave não selecionada a substitui e o undo a devolve
        let s2 = sel(&[(l.clone(), 10)]);
        let (edit, _) = plan_move(&id(), &p.animation.clips[&id()], &s2, 10).unwrap();
        let before = p.clone();
        let a = run(&mut p, edit);
        assert_eq!(p.animation.clips[&id()].channel(&l).unwrap().keys.len(), 2);
        a.inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn scale_stretches_time_around_the_pivot_and_keeps_order() {
        let mut p = project();
        let l = ChannelTarget::bone("LeftShoulder");
        let all = sel(&[(l.clone(), 0), (l.clone(), 10), (l.clone(), 20)]);
        let edit = plan_scale(&id(), &p.animation.clips[&id()], &all, 0.0, 2.0).unwrap();
        run(&mut p, edit);
        let frames: Vec<i32> = p.animation.clips[&id()].channel(&l).unwrap().keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 20, 40]);
        // comprimir muito: nenhuma colisão, ordem estrita
        let all = sel(&[(l.clone(), 0), (l.clone(), 20), (l.clone(), 40)]);
        let edit = plan_scale(&id(), &p.animation.clips[&id()], &all, 0.0, 0.01).unwrap();
        run(&mut p, edit);
        let frames: Vec<i32> = p.animation.clips[&id()].channel(&l).unwrap().keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 1, 2]);
        assert!(plan_scale(&id(), &p.animation.clips[&id()], &all, 0.0, 0.0).is_none());
        assert!(plan_scale(&id(), &p.animation.clips[&id()], &all, 0.0, f32::NAN).is_none());
        assert!(plan_scale(&id(), &p.animation.clips[&id()], &all, 0.0, 1.0).is_none(), "fator 1 não muda nada");
    }

    #[test]
    fn duplicate_copies_keys_forward_with_their_interpolation() {
        let mut p = project();
        let l = ChannelTarget::bone("LeftShoulder");
        let mut c = p.animation.clips.get(&id()).unwrap().clone();
        c.channels[0].keys[1].interp = Interp::Ease;
        p.animation.clips.insert(id(), c);
        let edit = plan_duplicate(&id(), &p.animation.clips[&id()], &sel(&[(l.clone(), 10)]), 3).unwrap();
        run(&mut p, edit);
        let ch = p.animation.clips[&id()].channel(&l).unwrap();
        assert_eq!(ch.key_at(13).unwrap().interp, Interp::Ease);
        assert_eq!(ch.key_at(13).unwrap().value, ch.key_at(10).unwrap().value);
        assert!(plan_duplicate(&id(), &p.animation.clips[&id()], &sel(&[(l, 10)]), -50).is_none());
    }

    #[test]
    fn mirror_writes_the_reflection_to_the_opposite_side_and_swaps_when_both_are_selected() {
        let mut p = project();
        let (l, r) = (ChannelTarget::bone("LeftShoulder"), ChannelTarget::bone("RightShoulder"));
        // só o esquerdo: o direito do mesmo quadro é sobrescrito
        let edit = plan_mirror(&id(), &p.animation.clips[&id()], &sel(&[(l.clone(), 10)])).unwrap();
        run(&mut p, edit);
        let c = &p.animation.clips[&id()];
        let KeyValue::Quat(rq) = c.channel(&r).unwrap().key_at(10).unwrap().value else { panic!() };
        let KeyValue::Quat(lq) = c.channel(&l).unwrap().key_at(10).unwrap().value else { panic!() };
        // rotação em X: o reflexo em X (x, -y, -z, w) mantém o X
        assert!((rq[0] - lq[0]).abs() < 1e-5);
        // os dois lados selecionados trocam de valor
        let mut p = project();
        let mut c = p.animation.clips.get(&id()).unwrap().clone();
        c.channels.iter_mut().find(|ch| ch.target == r).unwrap().keys[0].value = KeyValue::Quat(glam::Quat::from_rotation_z(0.7).to_array());
        c.channels.iter_mut().find(|ch| ch.target == l).unwrap().keys[1].value = KeyValue::Quat(glam::Quat::from_rotation_y(0.4).to_array());
        p.animation.clips.insert(id(), c.clone());
        let both = sel(&[(l.clone(), 10), (r.clone(), 10)]);
        let (lv, rv) = (c.channel(&l).unwrap().key_at(10).unwrap().value, c.channel(&r).unwrap().key_at(10).unwrap().value);
        run(&mut p, plan_mirror(&id(), &c, &both).unwrap());
        let c2 = &p.animation.clips[&id()];
        let mirrored = |v: KeyValue| mirror_key(&l, &Key::new(10, v)).unwrap().1.value;
        assert_eq!(c2.channel(&r).unwrap().key_at(10).unwrap().value, mirrored(lv));
        assert_eq!(c2.channel(&l).unwrap().key_at(10).unwrap().value, mirror_key(&r, &Key::new(10, rv)).unwrap().1.value);
        // canais não espelháveis são ignorados
        assert!(plan_mirror(&id(), c2, &sel(&[(ChannelTarget::Camera { prop: anigo_core::animation::CameraProp::Fov }, 0)])).is_none());
    }

    #[test]
    fn delete_and_interp_only_touch_what_changes() {
        let mut p = project();
        let l = ChannelTarget::bone("LeftShoulder");
        let s = sel(&[(l.clone(), 0), (l.clone(), 10), (l.clone(), 77)]);
        assert_eq!(existing(&p.animation.clips[&id()], &s).len(), 2, "o quadro 77 não existe");
        let e = plan_interp(&id(), &p.animation.clips[&id()], &s, Interp::Step).unwrap();
        run(&mut p, e);
        assert!(plan_interp(&id(), &p.animation.clips[&id()], &s, Interp::Step).is_none(), "já é Step");
        let e = plan_delete(&id(), &p.animation.clips[&id()], &s).unwrap();
        run(&mut p, e);
        assert_eq!(p.animation.clips[&id()].channel(&l).unwrap().keys.len(), 1);
        assert!(plan_delete(&id(), &p.animation.clips[&id()], &BTreeSet::new()).is_none());
    }

    #[test]
    fn insert_uses_live_values_skips_unchanged_keys_and_keeps_existing_interpolation() {
        let mut p = project();
        let h = ChannelTarget::Hips;
        p.character.pose.hips_translation = [0.0, -0.1, 0.0];
        p.character.pose.set_rotation(anigo_core::pose::bone::HEAD, glam::Quat::from_rotation_y(0.5));
        let targets = vec![h.clone(), ChannelTarget::bone("Head"), ChannelTarget::Hand { side: HandSide::Left }];
        let edit = plan_insert(&p, &id(), Some(&p.animation.clips[&id()]), 7, &targets, Interp::Ease).unwrap();
        let AnimEdit::InsertKeys { ref keys, .. } = edit else { panic!() };
        assert_eq!(keys.len(), 3);
        assert!(keys.iter().all(|k| k.key.interp == Interp::Ease));
        run(&mut p, edit);
        // de novo no mesmo quadro: nada mudou, nada a inserir
        assert!(plan_insert(&p, &id(), Some(&p.animation.clips[&id()]), 7, &targets, Interp::Linear).is_none());
        // mudar o valor mantém a interpolação da chave existente
        p.character.pose.hips_translation = [0.0, -0.3, 0.0];
        let edit = plan_insert(&p, &id(), Some(&p.animation.clips[&id()]), 7, &[h], Interp::Linear).unwrap();
        let AnimEdit::InsertKeys { keys, .. } = edit else { panic!() };
        assert_eq!(keys[0].key.interp, Interp::Ease);
        // alvos que o projeto não tem (nó inexistente) são ignorados
        assert!(plan_insert(&p, &id(), None, 0, &[ChannelTarget::NodeScale { node_id: anigo_core::ids::NodeId::from_slug("zz") }], Interp::Linear).is_none());
    }

    #[test]
    fn delete_at_frame_removes_only_existing_keys() {
        let p = project();
        let c = &p.animation.clips[&id()];
        let t = [ChannelTarget::bone("LeftShoulder"), ChannelTarget::bone("RightShoulder"), ChannelTarget::bone("Head")];
        let AnimEdit::DeleteKeys { keys, .. } = plan_delete_at(&id(), c, 10, &t).unwrap() else { panic!() };
        assert_eq!(keys.len(), 2);
        assert!(plan_delete_at(&id(), c, 11, &t).is_none());
    }

    #[test]
    fn clipboard_pastes_relative_to_the_first_key_optionally_mirrored() {
        let mut p = project();
        let l = ChannelTarget::bone("LeftShoulder");
        let s = sel(&[(l.clone(), 10), (l.clone(), 20), (ChannelTarget::Hips, 20)]);
        let cb = KeyClipboard::copy(&p.animation.clips[&id()], &s);
        assert_eq!(cb.keys.len(), 3);
        assert!(cb.keys.iter().any(|(_, rel, _)| *rel == 0) && cb.keys.iter().any(|(_, rel, _)| *rel == 10));
        run(&mut p, cb.paste(&id(), 3, false).unwrap());
        let c = &p.animation.clips[&id()];
        assert!(c.channel(&l).unwrap().key_at(3).is_some() && c.channel(&l).unwrap().key_at(13).is_some());
        // colado espelhado vai para o outro lado; o quadril reflete em X
        run(&mut p, cb.paste(&id(), 22, true).unwrap());
        let c = &p.animation.clips[&id()];
        assert!(c.channel(&ChannelTarget::bone("RightShoulder")).unwrap().key_at(22).is_some());
        let KeyValue::Vec3(h) = c.channel(&ChannelTarget::Hips).unwrap().key_at(32).unwrap().value else { panic!() };
        assert_eq!(h[0], -0.5);
        assert!(KeyClipboard::default().paste(&id(), 0, false).is_none());
        assert!(KeyClipboard::default().is_empty());
        // quadros negativos são descartados
        let neg = KeyClipboard { keys: vec![(l, -50, Key::new(0, quat(0.0)))] };
        assert!(neg.paste(&id(), 10, false).is_none());
    }

    #[test]
    fn snapping_to_twos_and_threes_and_selecting_a_whole_frame() {
        assert_eq!(snap_to_step(5, 0, 2), 6);
        assert_eq!(snap_to_step(4, 0, 2), 4);
        assert_eq!(snap_to_step(8, 0, 3), 9);
        assert_eq!(snap_to_step(7, 1, 3), 7);
        assert_eq!(snap_to_step(5, 0, 1), 5);
        let p = project();
        let c = &p.animation.clips[&id()];
        let all = keys_at_frame(c, 10, |_| true);
        assert_eq!(all.len(), 2);
        let only_hips = keys_at_frame(c, 20, |t| matches!(t, ChannelTarget::Hips));
        assert_eq!(only_hips.len(), 1);
    }

    /// "Inserir chave" sem clip: o clip nasce no MESMO passo de histórico que as chaves (Batch[PutClip, SetActive, InsertKeys]).
    #[test]
    fn inserting_a_key_without_a_clip_is_one_undo_step() {
        let mut session = anigo_session::CoreSession::new();
        let cid = id();
        let fresh = AnimationClip::new(cid.clone(), "Clip 1");
        let edit = plan_insert(session.project(), &cid, Some(&fresh), 12, &anigo_core::animation::full_pose_targets(), Interp::Linear).expect("há o que gravar");
        let depth = session.history().undo_depth();
        session
            .apply(Command::Batch {
                commands: vec![
                    Command::EditAnimation { edit: AnimEdit::PutClip { clip: fresh } },
                    Command::EditAnimation { edit: AnimEdit::SetActiveClip { clip_id: Some(cid.clone()) } },
                    Command::EditAnimation { edit },
                ],
            })
            .unwrap();
        assert_eq!(session.history().undo_depth(), depth + 1);
        assert!(session.project().animation.clips[&cid].key_count() > 10);
        assert_eq!(session.project().animation.active_clip.as_ref(), Some(&cid));
        session.undo().unwrap();
        assert!(session.project().animation.clips.is_empty() && session.project().animation.active_clip.is_none());
        session.redo().unwrap();
        assert!(session.project().animation.clips[&cid].key_count() > 10);
    }
}
