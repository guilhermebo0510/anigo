//! Textos do workspace Animação (pt-BR / en). Chaves `anim.*` (e `tool.clips`); o teste confere que toda chave usada no
//! código existe aqui, que não há duplicata, texto vazio nem emoji, e que as chaves compostas em tempo de execução
//! (canais de sol/câmera, interpolações, modos de repetição) existem.

/// (chave, pt-BR, en)
pub const ANIMATION_TABLE: &[(&str, &str, &str)] = &[
    ("tool.clips", "Clips", "Clips"),
    // abas do inspetor
    ("anim.tab.clip", "Clip", "Clip"),
    ("anim.tab.bone", "Osso", "Bone"),
    // transporte
    ("anim.t.start", "Ir ao início", "Go to start"),
    ("anim.t.end", "Ir ao fim", "Go to end"),
    ("anim.t.prev_key", "Chave anterior", "Previous key"),
    ("anim.t.next_key", "Próxima chave", "Next key"),
    ("anim.t.prev_frame", "Quadro anterior", "Previous frame"),
    ("anim.t.next_frame", "Próximo quadro", "Next frame"),
    ("anim.t.play", "Reproduzir / pausar", "Play / pause"),
    ("anim.t.space", "Espaço", "Space"),
    ("anim.t.frame", "Quadro", "Frame"),
    ("anim.t.speed", "Velocidade de reprodução", "Playback speed"),
    ("anim.loop.once", "Uma vez", "Once"),
    ("anim.loop.loop", "Repetir", "Loop"),
    ("anim.loop.pingpong", "Vai e volta", "Ping-pong"),
    ("anim.twos", "Animar em", "Animate on"),
    ("anim.twos_tip", "Animar em 1s/2s/3s: as chaves caem na grade e o clip segura cada pose por N quadros (estilo anime)", "Animate on ones/twos/threes: keys snap to the grid and the clip holds each pose for N frames (anime style)"),
    ("anim.quantize", "Tocar em quadros inteiros", "Play on whole frames"),
    ("anim.quantize_tip", "Durante a reprodução mostra exatamente os quadros que o render da sequência produz", "During playback shows exactly the frames the sequence render produces"),
    // chaves
    ("anim.key.insert", "Inserir chave", "Insert key"),
    ("anim.key.insert_tip", "Insere chaves no quadro atual com o valor vivo (ossos selecionados, ou o corpo todo)", "Inserts keys at the current frame with the live value (selected bones, or the whole body)"),
    ("anim.key.delete_here", "Apagar chaves aqui", "Delete keys here"),
    ("anim.key.nothing", "Nada novo para gravar neste quadro", "Nothing new to key on this frame"),
    ("anim.key.copied", "Chaves copiadas", "Keys copied"),
    ("anim.key.mirror_none", "Nenhuma chave espelhável na seleção", "No mirrorable key in the selection"),
    ("anim.keys", "chaves", "keys"),
    ("anim.autokey_on", "Auto-key", "Auto-key"),
    ("anim.autokey_tip", "Auto-key: toda edição de pose neste quadro vira chave (um passo de desfazer)", "Auto-key: every pose edit on this frame becomes a key (one undo step)"),
    ("anim.unkeyed", "Pose sem chave", "Unkeyed pose"),
    ("anim.unkeyed_tip", "A pose foi editada e não está gravada no clip neste quadro. Insira uma chave (I) ou ligue o auto-key.", "The pose was edited and is not stored in the clip on this frame. Insert a key (I) or turn on auto-key."),
    ("anim.drag.cancelled", "Arrasto cancelado", "Drag cancelled"),
    // interpolação
    ("anim.interp", "Interpolação", "Interpolation"),
    ("anim.interp_tip", "Interpolação das chaves novas", "Interpolation of new keys"),
    ("anim.interp.step", "Degrau", "Step"),
    ("anim.interp.linear", "Linear", "Linear"),
    ("anim.interp.ease", "Suave", "Ease"),
    ("anim.interp.ease_in", "Entrada suave", "Ease in"),
    ("anim.interp.ease_out", "Saída suave", "Ease out"),
    ("anim.interp.bezier", "Bézier", "Bezier"),
    ("anim.handles", "Alças Bézier (x1 y1 x2 y2)", "Bezier handles (x1 y1 x2 y2)"),
    ("anim.handles_preset", "Interpolação predefinida: escolha Bézier para editar as alças", "Preset interpolation: choose Bezier to edit the handles"),
    // onion skin
    ("anim.onion", "Onion skin", "Onion skin"),
    ("anim.onion_tip", "Fantasmas do esqueleto em quadros ou chaves vizinhos (azul antes, verde depois)", "Skeleton ghosts on neighbouring frames or keys (blue before, green after)"),
    ("anim.onion.before", "Antes", "Before"),
    ("anim.onion.after", "Depois", "After"),
    ("anim.onion.step", "Passo (quadros)", "Step (frames)"),
    ("anim.onion.keys", "Nas chaves vizinhas", "On neighbouring keys"),
    ("anim.onion.keys_tip", "Usa as chaves anterior e seguinte em vez de quadros a distância fixa", "Uses the previous and next keys instead of frames at a fixed distance"),
    ("anim.onion.note", "Os fantasmas são o esqueleto avaliado no outro quadro; a malha não é duplicada.", "Ghosts are the skeleton evaluated on the other frame; the mesh is not duplicated."),
    ("anim.only_selected_tip", "Mostrar só os canais dos ossos selecionados", "Show only the channels of the selected bones"),
    // seleção de chaves
    ("anim.sel.title", "Chaves selecionadas", "Selected keys"),
    ("anim.sel.none", "Nenhuma chave selecionada", "No key selected"),
    ("anim.sel.help", "Clique numa chave ou arraste uma caixa no dope sheet. Arraste para mover, Shift+arrasto duplica, Alt+arrasto escala no tempo em torno do playhead.", "Click a key or drag a box in the dope sheet. Drag to move, Shift+drag duplicates, Alt+drag scales in time around the playhead."),
    ("anim.sel.count", "chave(s) selecionada(s)", "key(s) selected"),
    ("anim.sel.duplicate", "Duplicar", "Duplicate"),
    ("anim.sel.duplicate_tip", "Duplica as chaves para frente", "Duplicates the keys forward"),
    ("anim.sel.mirror", "Espelhar", "Mirror"),
    ("anim.sel.mirror_tip", "Espelha as chaves para o lado oposto (E/D) mantendo o quadro", "Mirrors the keys to the opposite side (L/R) keeping the frame"),
    ("anim.sel.delete", "Apagar", "Delete"),
    ("anim.sel.copy", "Copiar", "Copy"),
    ("anim.sel.paste", "Colar no playhead", "Paste at playhead"),
    ("anim.sel.paste_mirrored", "Colar espelhado", "Paste mirrored"),
    ("anim.sel.shift", "quadros", "frames"),
    ("anim.sel.shift_tip", "Desloca as chaves selecionadas", "Shifts the selected keys"),
    // clip
    ("anim.clip.title", "Clip", "Clip"),
    ("anim.clip.new", "Novo clip", "New clip"),
    ("anim.clip.rename", "Renomear", "Rename"),
    ("anim.clip.duplicate", "Duplicar clip", "Duplicate clip"),
    ("anim.clip.delete", "Apagar clip", "Delete clip"),
    ("anim.clip.default_name", "Clip", "Clip"),
    ("anim.clip.copy_suffix", "cópia", "copy"),
    ("anim.clips", "Clips do projeto", "Project clips"),
    ("anim.clips_n", "clip(s)", "clip(s)"),
    ("anim.no_clip", "Sem clip", "No clip"),
    ("anim.no_clip_hint", "Sem clip: insira uma chave (I) ou crie um clip com +. Com o auto-key ligado, editar a pose já cria o clip.", "No clip: insert a key (I) or create a clip with +. With auto-key on, editing the pose already creates the clip."),
    ("anim.empty_clip", "Clip sem chaves: posicione o personagem e insira uma chave (I)", "Clip without keys: pose the character and insert a key (I)"),
    ("anim.range", "Faixa", "Range"),
    ("anim.range.start", "início", "start"),
    ("anim.range.end", "fim", "end"),
    ("anim.range.auto", "Faixa automática (até a última chave)", "Automatic range (up to the last key)"),
    ("anim.range.auto_tip", "Desligue para fixar o início e o fim da reprodução e do render", "Turn off to fix the start and end of playback and render"),
    ("anim.range.fit_keys", "Ajustar às chaves", "Fit to keys"),
    ("anim.fps", "Quadros por segundo", "Frames per second"),
    ("anim.fps.retime", "Manter a duração", "Keep duration"),
    ("anim.fps.retime_tip", "Ao mudar o fps, reescala os quadros das chaves para manter a duração em segundos", "When changing fps, rescales key frames to keep the duration in seconds"),
    ("anim.extrap", "Fora da faixa", "Outside range"),
    ("anim.extrap.constant", "Segurar", "Hold"),
    ("anim.extrap.linear", "Continuar", "Continue"),
    // cena (sol, câmera, nós)
    ("anim.scene.title", "Cena: sol, câmera, objetos", "Scene: sun, camera, objects"),
    ("anim.scene.note", "Grava chaves do estado atual da cena no quadro atual: edite o sol, a câmera ou o objeto nos outros workspaces, volte e insira a chave.", "Keys the scene's current state on the current frame: edit the sun, camera or object in the other workspaces, come back and insert the key."),
    ("anim.scene.key_sun", "Chave do sol", "Key sun"),
    ("anim.scene.key_sun_tip", "Direção, intensidade e cor do sol", "Sun direction, intensity and color"),
    ("anim.scene.key_camera", "Chave da câmera", "Key camera"),
    ("anim.scene.key_camera_tip", "Campo de visão, posição e alvo da câmera da cena", "Scene camera field of view, position and target"),
    ("anim.scene.key_node", "Chave do objeto", "Key object"),
    ("anim.scene.key_node_tip", "Posição, rotação e escala do objeto escolhido", "Position, rotation and scale of the chosen object"),
    // marcadores
    ("anim.markers", "Marcadores", "Markers"),
    ("anim.marker.toggle", "Marcador no quadro atual", "Marker on current frame"),
    ("anim.marker.toggle_tip", "Cria ou remove um marcador no quadro atual (M)", "Creates or removes a marker on the current frame (M)"),
    ("anim.marker.go", "Ir ao marcador", "Go to marker"),
    ("anim.marker.default", "Marcador", "Marker"),
    // dope sheet / curvas
    ("anim.dope", "Dope sheet", "Dope sheet"),
    ("anim.curves", "Curvas", "Curves"),
    ("anim.row.summary", "Resumo", "Summary"),
    ("anim.group.character", "Personagem", "Character"),
    ("anim.group.nodes", "Objetos da cena", "Scene objects"),
    ("anim.group.sun", "Sol", "Sun"),
    ("anim.group.camera", "Câmera", "Camera"),
    ("anim.ch.root", "Raiz (posição)", "Root (position)"),
    ("anim.ch.hips", "Quadril (posição)", "Hips (position)"),
    ("anim.ch.hand_l", "Dedos da mão E", "Left hand fingers"),
    ("anim.ch.hand_r", "Dedos da mão D", "Right hand fingers"),
    ("anim.ch.position", "posição", "position"),
    ("anim.ch.rotation", "rotação", "rotation"),
    ("anim.ch.scale", "escala", "scale"),
    ("anim.ch.sun.intensity", "intensidade", "intensity"),
    ("anim.ch.sun.direction", "direção", "direction"),
    ("anim.ch.sun.color", "cor", "color"),
    ("anim.ch.cam.fov", "campo de visão", "field of view"),
    ("anim.ch.cam.eye", "posição", "position"),
    ("anim.ch.cam.target", "alvo", "target"),
    // histórico (descrições)
    ("anim.h.batch", "Pose e chaves", "Pose and keys"),
    ("anim.h.insert", "Inserir chaves", "Insert keys"),
    ("anim.h.delete", "Apagar chaves", "Delete keys"),
    ("anim.h.move", "Mover chaves", "Move keys"),
    ("anim.h.interp", "Interpolação", "Interpolation"),
    ("anim.h.clip", "Clip", "Clip"),
    ("anim.h.range", "Faixa do clip", "Clip range"),
    ("anim.h.playback", "Reprodução do clip", "Clip playback"),
    ("anim.h.fps", "Quadros por segundo", "Frames per second"),
    ("anim.h.markers", "Marcadores", "Markers"),
    // física
    ("anim.phys.title", "Física secundária", "Secondary physics"),
    ("anim.phys.note", "Cabelo e saia de teste simulados de verdade sobre os ossos do clip (passo fixo de 1/60 s). O renderer ainda não os consome: aparecem como overlay e saem como dado.", "Test hair and skirt truly simulated on the clip bones (fixed 1/60 s step). The renderer does not consume them yet: they show as an overlay and come out as data."),
    ("anim.phys.enable", "Simular no viewport", "Simulate in the viewport"),
    ("anim.phys.hair", "Cabelo", "Hair"),
    ("anim.phys.skirt", "Saia", "Skirt"),
    ("anim.phys.preroll", "Pré-aquecimento", "Pre-roll"),
    ("anim.phys.preroll_tip", "Ao saltar no tempo a simulação reinicia na pose de t menos este tempo e avança até t", "When jumping in time the simulation restarts at the pose of t minus this time and advances to t"),
    ("anim.phys.particles", "partículas", "particles"),
    ("anim.phys.steps", "passo(s)", "step(s)"),
    ("anim.phys.resets", "reinício(s)", "reset(s)"),
    ("anim.phys.bake", "Gravar simulação do clip", "Bake clip simulation"),
    ("anim.phys.bake_tip", "Simula a faixa inteira e grava as posições por quadro em um arquivo .physics.json", "Simulates the whole range and writes the per-frame positions to a .physics.json file"),
    ("anim.phys.baked", "Simulação gravada", "Simulation baked"),
    ("anim.phys.frames", "quadros", "frames"),
    // arquivos
    ("anim.io.title", "Importar e exportar", "Import and export"),
    ("anim.io.note", "glTF/GLB/VRM/VRMA, BVH e clips do ANIGO. Esqueletos humanoides são mapeados para os 24 ossos canônicos e reorientados por direção de osso.", "glTF/GLB/VRM/VRMA, BVH and ANIGO clips. Humanoid skeletons are mapped to the 24 canonical bones and retargeted by bone direction."),
    ("anim.io.path_hint", "Caminho do arquivo (ou pasta para exportar)", "File path (or folder to export to)"),
    ("anim.io.import", "Importar", "Import"),
    ("anim.io.import_tip", "Importa o arquivo do caminho como clip novo", "Imports the file at the path as a new clip"),
    ("anim.io.export_glb", "glTF", "glTF"),
    ("anim.io.export_vrma", "VRMA", "VRMA"),
    ("anim.io.export_bvh", "BVH", "BVH"),
    ("anim.io.export_clip", "Clip ANIGO", "ANIGO clip"),
    ("anim.io.imported", "Importado", "Imported"),
    ("anim.io.exported", "Exportado", "Exported"),
    ("anim.io.error", "Erro", "Error"),
    ("anim.io.too_big", "arquivo grande demais", "file too large"),
    ("anim.io.unsupported", "formato não suportado (use .glb, .gltf, .vrma, .bvh ou .anigoclip.json)", "unsupported format (use .glb, .gltf, .vrma, .bvh or .anigoclip.json)"),
    ("anim.io.no_humanoid", "esqueleto humanoide não reconhecido", "humanoid skeleton not recognised"),
    ("anim.io.nothing", "o arquivo não tem animação", "the file has no animation"),
    ("anim.io.warnings", "aviso(s)", "warning(s)"),
    // sequência de render
    ("anim.seq.title", "Renderizar sequência", "Render sequence"),
    ("anim.seq.note", "Um quadro PNG por quadro do clip, pela fila do Render (mesma cena, câmera e saída).", "One PNG per clip frame, through the Render queue (same scene, camera and output)."),
    ("anim.seq.custom", "Intervalo personalizado", "Custom range"),
    ("anim.seq.step", "passo", "step"),
    ("anim.seq.folder", "Pasta", "Folder"),
    ("anim.seq.template", "Nome", "Name"),
    ("anim.seq.template_tip", "Use %04d (ou {frame:04}) para o número do quadro", "Use %04d (or {frame:04}) for the frame number"),
    ("anim.seq.render", "Renderizar sequência", "Render sequence"),
    ("anim.seq.frames", "quadros", "frames"),
    ("anim.seq.cancel", "Cancelar", "Cancel"),
    ("anim.seq.open_render", "Abrir o Render", "Open Render"),
    ("anim.seq.queued", "Sequência enfileirada", "Sequence queued"),
    ("anim.seq.st.queued", "Na fila", "Queued"),
    ("anim.seq.st.running", "Renderizando", "Rendering"),
    ("anim.seq.st.done", "Concluído", "Done"),
    ("anim.seq.st.failed", "Falhou", "Failed"),
    ("anim.seq.st.cancelled", "Cancelado", "Cancelled"),
    // Biblioteca de clips
    ("anim.lib.title", "Clips da Biblioteca", "Library clips"),
    ("anim.lib.name_hint", "Nome do clip", "Clip name"),
    ("anim.lib.save", "Salvar na Biblioteca", "Save to Library"),
    ("anim.lib.save_tip", "Guarda o clip ativo como ativo da Biblioteca (mesmo nome = nova revisão)", "Stores the active clip as a Library asset (same name = new revision)"),
    ("anim.lib.refresh", "Recarregar", "Reload"),
    ("anim.lib.empty", "Nenhum clip na Biblioteca ainda.", "No clips in the Library yet."),
    ("anim.lib.error", "Biblioteca", "Library"),
    ("anim.lib.apply_new", "Como clip novo", "As new clip"),
    ("anim.lib.apply_new_tip", "Cria um clip novo no projeto com este clip", "Creates a new project clip from this clip"),
    ("anim.lib.apply_at", "No playhead", "At playhead"),
    ("anim.lib.apply_at_tip", "Insere as chaves deste clip a partir do quadro atual do clip ativo", "Inserts this clip's keys starting at the active clip's current frame"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn has_emoji(s: &str) -> bool {
        s.chars().any(|c| {
            let u = c as u32;
            (0x1F000..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) || (0x2B00..=0x2BFF).contains(&u) || u == 0xFE0F
        })
    }

    #[test]
    fn keys_are_unique_non_empty_and_free_of_emoji() {
        let mut seen = HashSet::new();
        for (k, pt, en) in ANIMATION_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "{k} vazio");
            assert!(k.starts_with("anim.") || *k == "tool.clips", "{k}");
            assert!(!has_emoji(pt) && !has_emoji(en), "{k} tem emoji");
        }
    }

    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/animation_ui/mod.rs"),
            include_str!("app/animation_ui/panels.rs"),
            include_str!("app/animation_ui/timeline.rs"),
            include_str!("app/animation_ui/viewport.rs"),
            include_str!("app/animation_ui/script.rs"),
            include_str!("library_ui/clip_bridge.rs"),
        ];
        let known: HashSet<&str> = ANIMATION_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"anim.") {
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
    fn composed_keys_exist() {
        let known: HashSet<&str> = ANIMATION_TABLE.iter().map(|(k, _, _)| *k).collect();
        for k in ["intensity", "direction", "color"] {
            assert!(known.contains(format!("anim.ch.sun.{k}").as_str()), "sol {k}");
        }
        for k in ["fov", "eye", "target"] {
            assert!(known.contains(format!("anim.ch.cam.{k}").as_str()), "câmera {k}");
        }
        for k in ["step", "linear", "ease", "ease_in", "ease_out", "bezier"] {
            assert!(known.contains(format!("anim.interp.{k}").as_str()), "interpolação {k}");
        }
        for k in ["once", "loop", "pingpong"] {
            assert!(known.contains(format!("anim.loop.{k}").as_str()), "repetição {k}");
        }
        // todo osso tem nome (vem das chaves do Posing)
        for b in anigo_core::pose::BONE_NAMES {
            assert!(crate::i18n::tr(&format!("pose.bone.{b}")) != format!("pose.bone.{b}"), "{b}");
        }
    }

    #[test]
    fn the_table_translates_in_both_languages() {
        use crate::i18n::{tr_in, Lang};
        assert_eq!(tr_in(Lang::En, "anim.key.insert"), "Insert key");
        assert_eq!(tr_in(Lang::PtBr, "anim.key.insert"), "Inserir chave");
        assert_eq!(tr_in(Lang::En, "anim.nao_existe"), "anim.nao_existe");
    }
}
