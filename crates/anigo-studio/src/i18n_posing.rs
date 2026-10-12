//! Textos do workspace Posing (pt-BR / en). Chaves `pose.*`; o teste confere que toda chave usada no código
//! existe aqui, que não há duplicata nem texto vazio e que os 24 ossos têm nome nos dois idiomas.

/// (chave, pt-BR, en)
pub const POSING_TABLE: &[(&str, &str, &str)] = &[
    // ossos (nomes canônicos VRM)
    ("pose.bone.Root", "Raiz", "Root"),
    ("pose.bone.Hips", "Quadril", "Hips"),
    ("pose.bone.Pelvis", "Pelve", "Pelvis"),
    ("pose.bone.Spine", "Coluna", "Spine"),
    ("pose.bone.Chest", "Peito", "Chest"),
    ("pose.bone.UpperChest", "Peito superior", "Upper chest"),
    ("pose.bone.Neck", "Pescoço", "Neck"),
    ("pose.bone.Head", "Cabeça", "Head"),
    ("pose.bone.LeftClavicle", "Clavícula E", "Left clavicle"),
    ("pose.bone.LeftShoulder", "Ombro E", "Left shoulder"),
    ("pose.bone.LeftElbow", "Cotovelo E", "Left elbow"),
    ("pose.bone.LeftWrist", "Punho E", "Left wrist"),
    ("pose.bone.RightClavicle", "Clavícula D", "Right clavicle"),
    ("pose.bone.RightShoulder", "Ombro D", "Right shoulder"),
    ("pose.bone.RightElbow", "Cotovelo D", "Right elbow"),
    ("pose.bone.RightWrist", "Punho D", "Right wrist"),
    ("pose.bone.LeftThigh", "Coxa E", "Left thigh"),
    ("pose.bone.LeftKnee", "Joelho E", "Left knee"),
    ("pose.bone.LeftAnkle", "Tornozelo E", "Left ankle"),
    ("pose.bone.LeftToes", "Dedos do pé E", "Left toes"),
    ("pose.bone.RightThigh", "Coxa D", "Right thigh"),
    ("pose.bone.RightKnee", "Joelho D", "Right knee"),
    ("pose.bone.RightAnkle", "Tornozelo D", "Right ankle"),
    ("pose.bone.RightToes", "Dedos do pé D", "Right toes"),
    // membros
    ("pose.limb.left_arm", "Braço esquerdo", "Left arm"),
    ("pose.limb.right_arm", "Braço direito", "Right arm"),
    ("pose.limb.left_leg", "Perna esquerda", "Left leg"),
    ("pose.limb.right_leg", "Perna direita", "Right leg"),
    // operações (histórico)
    ("pose.history", "Histórico", "History"),
    ("pose.history.pose", "Pose", "Pose"),
    ("pose.history.initial", "Estado inicial", "Initial state"),
    ("pose.op.edit", "edição", "edit"),
    ("pose.op.mirror", "espelhar", "mirror"),
    ("pose.op.reset", "resetar", "reset"),
    ("pose.op.apply", "aplicar da Biblioteca", "apply from Library"),
    ("pose.op.paste", "colar", "paste"),
    ("pose.op.preset", "preset", "preset"),
    ("pose.op.ik", "IK", "IK"),
    ("pose.op.gaze", "olhar", "gaze"),
    ("pose.op.hand", "mãos", "hands"),
    ("pose.op.ground", "pés no chão", "ground feet"),
    // HUD
    ("pose.hud.rotate", "girar", "rotate"),
    ("pose.hud.move_body", "mover o corpo", "move body"),
    ("pose.hud.move_hips", "mover o quadril", "move hips"),
    ("pose.hud.hips", "quadril", "hips"),
    ("pose.hud.gaze", "olhar para o alvo", "look at target"),
    // barra do viewport
    ("pose.view.skeleton", "Esqueleto", "Skeleton"),
    ("pose.view.skeleton_tip", "Mostrar ossos e juntas (clique/caixa seleciona)", "Show bones and joints (click/box selects)"),
    ("pose.view.mesh", "Malha", "Mesh"),
    ("pose.view.limits", "Limites", "Limits"),
    ("pose.view.limits_tip", "Cone de limites do osso ativo (verde = dentro, vermelho = fora)", "Limit cone of the active bone (green = inside, red = outside)"),
    ("pose.view.balance", "Equilíbrio", "Balance"),
    ("pose.view.balance_tip", "Centro de massa projetado e base de apoio dos pés", "Projected center of mass and foot support polygon"),
    ("pose.space.world", "Mundo", "World"),
    ("pose.space.local", "Local", "Local"),
    ("pose.symmetry", "Simetria", "Symmetry"),
    ("pose.symmetry_tip", "Editar um osso edita o do outro lado (espelho em X)", "Editing a bone also edits its opposite-side bone (X mirror)"),
    ("pose.snap", "Snap", "Snap"),
    ("pose.snap_off", "Livre", "Free"),
    // painel: estrutura óssea
    ("pose.tree", "Ossos", "Bones"),
    ("pose.tree.filter", "Filtrar ossos…", "Filter bones…"),
    ("pose.select_hint", "Clique numa junta no viewport (ou na lista) para selecionar; arraste uma caixa para várias; Shift/Ctrl soma.", "Click a joint in the viewport (or the list) to select; drag a box for several; Shift/Ctrl adds."),
    ("pose.mirror", "Espelhar", "Mirror"),
    ("pose.mirror_tip", "Espelha a pose em X (M). Com ossos selecionados, só eles e seus pares.", "Mirrors the pose in X (M). With selected bones, only them and their counterparts."),
    ("pose.copy", "Copiar", "Copy"),
    ("pose.copy_tip", "Copia a pose dos ossos selecionados (todos, se nada selecionado) — Ctrl+C", "Copies the pose of the selected bones (all if none) — Ctrl+C"),
    ("pose.paste", "Colar", "Paste"),
    ("pose.paste_tip", "Cola por nome de osso — Ctrl+V", "Pastes by bone name — Ctrl+V"),
    ("pose.paste_flipped", "Colar espelhado", "Paste flipped"),
    ("pose.reset_selection", "Resetar seleção", "Reset selection"),
    ("pose.reset_selection_tip", "Volta os ossos selecionados ao repouso (Alt+R); sem seleção, todos", "Returns the selected bones to rest (Alt+R); all if none selected"),
    ("pose.reset_all", "Resetar tudo", "Reset all"),
    ("pose.reset_bone", "Resetar osso", "Reset bone"),
    ("pose.copied", "pose copiada", "pose copied"),
    ("pose.clipboard_empty", "nada copiado ainda", "nothing copied yet"),
    ("pose.limits", "Limites", "Limits"),
    ("pose.limits.human", "Humano", "Human"),
    ("pose.limits.anime", "Anime", "Anime"),
    ("pose.limits.free", "Livre", "Free"),
    ("pose.limits_tip", "Humano = amplitudes normativas; Anime = +25 %; Livre = sem limites", "Human = normative ranges; Anime = +25 %; Free = no limits"),
    ("pose.translation", "Posição", "Position"),
    ("pose.swing_x", "Swing X", "Swing X"),
    ("pose.swing_z", "Swing Z", "Swing Z"),
    ("pose.twist", "Torção", "Twist"),
    ("pose.no_limit", "Sem limites neste osso/perfil.", "No limits for this bone/profile."),
    // painel: IK
    ("pose.ik.hint", "Membros em IK mostram um quadrado (mão/pé) e um losango (pole = para onde o cotovelo/joelho aponta).", "Limbs in IK show a square (hand/foot) and a diamond (pole = where the elbow/knee points)."),
    ("pose.ik.limbs", "FK / IK por membro", "FK / IK per limb"),
    ("pose.ik.all", "Todos em IK", "All IK"),
    ("pose.ik.none", "Todos em FK", "All FK"),
    ("pose.ik.drag_hint", "Arraste o quadrado no viewport; segure X, Y ou Z para travar o eixo; Esc cancela.", "Drag the square in the viewport; hold X, Y or Z to lock the axis; Esc cancels."),
    ("pose.ik.spine", "Cabeça arrasta a coluna (FABRIK)", "Head drags the spine (FABRIK)"),
    ("pose.ik.spine_tip", "A cabeça vira um handle de IK de cadeia longa: quadril fica, coluna e pescoço seguem", "The head becomes a long-chain IK handle: hips stay, spine and neck follow"),
    ("pose.feet", "Pés e quadril", "Feet and hips"),
    ("pose.foot_lock", "Travar pés", "Foot lock"),
    ("pose.foot_lock_tip", "Ao mover o quadril, os tornozelos ficam onde estavam (IK das pernas)", "When moving the hips, the ankles stay where they were (leg IK)"),
    ("pose.auto_ground", "Pés no chão (auto)", "Ground feet (auto)"),
    ("pose.auto_ground_tip", "Depois de cada edição, o ponto mais baixo dos pés toca o chão", "After each edit, the lowest foot point touches the floor"),
    ("pose.ground_now", "Apoiar no chão agora", "Ground now"),
    ("pose.hips", "Quadril", "Hips"),
    ("pose.root", "Corpo (raiz)", "Body (root)"),
    ("pose.gaze", "Olhar", "Gaze"),
    ("pose.gaze.show", "Alvo do olhar", "Gaze target"),
    ("pose.gaze.tip", "Mostra o alvo arrastável; soltar aplica cabeça e pescoço", "Shows the draggable target; releasing applies head and neck"),
    ("pose.gaze.weight", "Peso", "Weight"),
    ("pose.gaze.apply", "Olhar o alvo", "Look at target"),
    ("pose.gaze.front", "Alvo à frente", "Target ahead"),
    ("pose.gaze.center", "Cabeça reta", "Straight head"),
    ("pose.balance", "Equilíbrio", "Balance"),
    ("pose.balance.show", "Mostrar centro de massa", "Show center of mass"),
    ("pose.balance.ok", "dentro da base de apoio", "inside the support base"),
    ("pose.balance.out", "fora da base de apoio", "outside the support base"),
    ("pose.balance.note", "Indicador (frações de massa tipo Dempster, não validadas); nunca bloqueia a edição.", "Indicator (Dempster-like mass fractions, not validated); never blocks editing."),
    // painel: presets, mãos, Biblioteca
    ("pose.presets.body", "Poses de corpo", "Body poses"),
    ("pose.preset.t", "T-pose", "T-pose"),
    ("pose.preset.a", "A-pose", "A-pose"),
    ("pose.preset.standing", "Em pé", "Standing"),
    ("pose.preset.sitting", "Sentado", "Sitting"),
    ("pose.preset.walking", "Andando", "Walking"),
    ("pose.preset.action", "Ação", "Action"),
    ("pose.presets.hands", "Mãos", "Hands"),
    ("pose.hand.left", "Esquerda", "Left"),
    ("pose.hand.right", "Direita", "Right"),
    ("pose.hand.both", "Ambas", "Both"),
    ("pose.hand.open", "Aberta", "Open"),
    ("pose.hand.relaxed", "Relaxada", "Relaxed"),
    ("pose.hand.fist", "Punho", "Fist"),
    ("pose.hand.point", "Apontar", "Point"),
    ("pose.hand.v", "V", "V"),
    ("pose.hand.hold", "Segurar", "Hold"),
    ("pose.hand.note", "Dedos: deformação CPU nesta fase (o esqueleto canônico não tem ossos de dedo).", "Fingers: CPU deformation at this stage (the canonical skeleton has no finger bones)."),
    ("pose.finger.thumb", "Polegar", "Thumb"),
    ("pose.finger.index", "Indicador", "Index"),
    ("pose.finger.middle", "Médio", "Middle"),
    ("pose.finger.ring", "Anelar", "Ring"),
    ("pose.finger.little", "Mínimo", "Little"),
    ("pose.lib", "Biblioteca de poses", "Pose library"),
    ("pose.lib.name", "Nome da pose", "Pose name"),
    ("pose.lib.default_name", "Pose sem nome", "Untitled pose"),
    ("pose.lib.save", "Salvar na Biblioteca", "Save to Library"),
    ("pose.lib.save_tip", "Grava a pose atual como ativo .anigopose.json (mesmo nome = nova revisão)", "Saves the current pose as a .anigopose.json asset (same name = new revision)"),
    ("pose.lib.only_selected", "Só os ossos selecionados", "Only the selected bones"),
    ("pose.lib.assets", "Poses salvas", "Saved poses"),
    ("pose.lib.refresh", "Atualizar lista", "Refresh list"),
    ("pose.lib.blend", "Mistura", "Blend"),
    ("pose.lib.apply_selected", "Aplicar só aos ossos selecionados", "Apply only to the selected bones"),
    ("pose.lib.empty", "Nenhuma pose na Biblioteca ainda.", "No poses in the Library yet."),
    ("pose.lib.bones", "ossos", "bones"),
    ("pose.lib.apply", "Aplicar", "Apply"),
    ("pose.lib.apply_flipped", "Espelhado", "Flipped"),
    ("pose.lib.error", "Biblioteca", "Library"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn keys_are_unique_and_non_empty() {
        let mut seen = HashSet::new();
        for (k, pt, en) in POSING_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "{k} vazio");
            assert!(k.starts_with("pose."), "{k}");
        }
    }

    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/posing_ui/mod.rs"),
            include_str!("app/posing_ui/panels.rs"),
            include_str!("app/posing_ui/viewport.rs"),
            include_str!("app/posing_ui/script.rs"),
        ];
        let known: HashSet<&str> = POSING_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"pose.") {
                let tail = &rest[i + 1..];
                let Some(end) = tail.find('"') else { break };
                let key = &tail[..end];
                // chaves compostas em tempo de execução terminam com '.' ou contêm '{'
                if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') && !key.ends_with('.') && !known.contains(key) {
                    missing.push(key.to_string());
                }
                rest = &tail[end..];
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "chaves sem tradução: {missing:?}");
    }

    #[test]
    fn composed_keys_exist_for_every_bone_limb_op_preset_and_hand() {
        use anigo_core::pose::{presets, Limb, PoseOp, BONE_NAMES};
        let known: HashSet<&str> = POSING_TABLE.iter().map(|(k, _, _)| *k).collect();
        for b in BONE_NAMES {
            assert!(known.contains(format!("pose.bone.{b}").as_str()), "osso {b}");
        }
        for l in Limb::ALL {
            assert!(known.contains(format!("pose.limb.{}", l.key()).as_str()), "{l:?}");
        }
        for op in [PoseOp::Edit, PoseOp::Mirror, PoseOp::Reset, PoseOp::Apply, PoseOp::Paste, PoseOp::Preset, PoseOp::Ik, PoseOp::Gaze, PoseOp::Hand, PoseOp::Ground] {
            assert!(known.contains(format!("pose.op.{}", op.label()).as_str()), "{op:?}");
        }
        for k in presets::BODY {
            assert!(known.contains(format!("pose.preset.{k}").as_str()), "{k}");
        }
        for k in presets::HAND {
            assert!(known.contains(format!("pose.hand.{k}").as_str()), "{k}");
        }
    }

    #[test]
    fn labels_resolve_in_both_languages() {
        for (k, pt, en) in POSING_TABLE {
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::PtBr, k), *pt);
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::En, k), *en);
        }
    }
}
