//! Textos do workspace Cenário (pt-BR / en). Chaves `scene.*`; o teste confere que toda chave usada
//! no código existe aqui e que não há duplicata nem texto vazio.

/// (chave, pt-BR, en)
pub const SCENE_TABLE: &[(&str, &str, &str)] = &[
    // painéis
    ("scene.outliner", "Outliner", "Outliner"),
    ("scene.history", "Histórico", "History"),
    ("scene.details", "Detalhes", "Details"),
    ("scene.place", "Colocar atores", "Place actors"),
    ("scene.place.hint", "Clique ou arraste para a viewport.", "Click or drag into the viewport."),
    ("scene.place.shapes", "Formas", "Shapes"),
    ("scene.place.other", "Outros", "Other"),
    ("scene.drawer", "Content Drawer (Biblioteca)", "Content Drawer (Library)"),
    ("scene.drawer.hint", "Arraste um card para a viewport; duplo clique coloca no centro.", "Drag a card into the viewport; double-click places it at the center."),
    ("scene.drawer.reload", "Recarregar ativos da Biblioteca", "Reload Library assets"),
    ("scene.ok", "OK", "OK"),
    ("scene.on", "ligado", "on"),
    ("scene.off", "desligado", "off"),
    ("scene.loading", "carregando", "loading"),
    ("scene.asset_missing", "ativo ausente", "asset missing"),
    // atores
    ("scene.actor.cube", "Cubo", "Cube"),
    ("scene.actor.sphere", "Esfera", "Sphere"),
    ("scene.actor.plane", "Plano", "Plane"),
    ("scene.actor.cylinder", "Cilindro", "Cylinder"),
    ("scene.actor.cone", "Cone", "Cone"),
    ("scene.actor.camera", "Câmera de cena", "Scene camera"),
    ("scene.actor.group", "Grupo", "Group"),
    ("scene.actor.light", "Luz direcional / ponto", "Directional / point light"),
    ("scene.actor.light_tip", "Luzes extras exigem mudar o contrato de render congelado (decisão do projeto). A luz principal estilizada é editada em Detalhes > Cena.", "Extra lights require changing the frozen render contract (a project decision). The main stylized light is edited in Details > Scene."),
    // outliner
    ("scene.outliner.search", "Buscar (-x exclui, +x exato, \"frase\")", "Search (-x excludes, +x exact, \"phrase\")"),
    ("scene.outliner.search_help", "Termos parciais (todos precisam casar) em nome, tags, pasta e tipo. -termo exclui; +termo exige o nome exato; \"duas palavras\" casa a frase.", "Partial terms (all must match) over name, tags, folder and type. -term excludes; +term requires the exact name; \"two words\" matches the phrase."),
    ("scene.outliner.empty", "Nenhum objeto corresponde.", "No object matches."),
    ("scene.outliner.new_folder", "Mover a seleção para a pasta:", "Move the selection to folder:"),
    ("scene.outliner.folder_tip", "Pastas (organização do Outliner; Pai/Filho aninha)", "Folders (Outliner organization; Parent/Child nests)"),
    ("scene.outliner.drop_root", "Soltar aqui: volta à raiz", "Drop here: back to the root"),
    // histórico
    ("scene.history.initial", "Estado inicial", "Initial state"),
    ("scene.history.steps", "passos", "steps"),
    ("scene.history.redoable", "refazíveis", "redoable"),
    // menus
    ("scene.menu.focus", "Focar", "Focus"),
    ("scene.menu.duplicate", "Duplicar", "Duplicate"),
    ("scene.menu.copy", "Copiar", "Copy"),
    ("scene.menu.paste", "Colar", "Paste"),
    ("scene.menu.delete", "Apagar", "Delete"),
    ("scene.menu.snap_floor", "Snap ao chão", "Snap to floor"),
    ("scene.menu.hide", "Ocultar", "Hide"),
    ("scene.menu.lock", "Bloquear", "Lock"),
    ("scene.menu.unlock", "Desbloquear", "Unlock"),
    ("scene.menu.group", "Agrupar", "Group"),
    ("scene.menu.ungroup", "Desagrupar", "Ungroup"),
    ("scene.menu.rename", "Renomear", "Rename"),
    ("scene.menu.select_children", "Selecionar filhos", "Select children"),
    ("scene.menu.select_same", "Selecionar o mesmo ativo", "Select same asset"),
    ("scene.menu.unparent", "Tirar do pai", "Unparent"),
    // details
    ("scene.details.objects", "objetos selecionados", "objects selected"),
    ("scene.details.visible", "Visível", "Visible"),
    ("scene.details.locked", "Bloqueado", "Locked"),
    ("scene.details.all_locked", "Todos os selecionados estão bloqueados.", "All selected objects are locked."),
    ("scene.details.transform", "Transform", "Transform"),
    ("scene.details.location", "Posição", "Location"),
    ("scene.details.rotation", "Rotação", "Rotation"),
    ("scene.details.scale", "Escala", "Scale"),
    ("scene.details.scale_lock", "Travar proporção", "Lock proportions"),
    ("scene.details.scale_lock_tip", "Escala uniforme: editar um eixo escala os três.", "Uniform scale: editing one axis scales all three."),
    ("scene.details.relative", "relativo ao pai", "relative to parent"),
    ("scene.details.parent", "Pai:", "Parent:"),
    ("scene.details.asset", "Ativo da Biblioteca", "Library asset"),
    ("scene.details.revision", "revisão", "revision"),
    ("scene.details.stale", "A Biblioteca tem a revisão", "The Library has revision"),
    ("scene.details.update", "Atualizar", "Update"),
    ("scene.details.tris", "triângulos", "triangles"),
    ("scene.details.replace", "Substituir pelo selecionado na Biblioteca", "Replace with the Library selection"),
    ("scene.details.replace_tip", "Troca o ativo dos objetos selecionados mantendo posição, rotação e escala.", "Swaps the asset of the selected objects keeping position, rotation and scale."),
    ("scene.details.lens", "Lente da câmera", "Camera lens"),
    ("scene.details.material", "Material (por instância)", "Material (per instance)"),
    ("scene.details.organize", "Organização", "Organization"),
    ("scene.details.tags", "Tags", "Tags"),
    ("scene.details.add_tag", "adicionar tag e Enter", "add tag and Enter"),
    ("scene.details.folder", "Pasta", "Folder"),
    ("scene.material.base", "Cor", "Color"),
    ("scene.material.shade", "Sombra", "Shade"),
    ("scene.material.outline", "Contorno", "Outline"),
    ("scene.material.reset", "Voltar ao material do ativo", "Back to the asset material"),
    // lente
    ("scene.lens.focal", "Focal", "Focal"),
    ("scene.lens.use_as_render", "Usar como câmera de render", "Use as render camera"),
    ("scene.lens.use_as_render_tip", "Define a câmera do projeto (posição, direção e FOV da lente).", "Sets the project camera (position, direction and lens FOV)."),
    ("scene.lens.align_to_view", "Alinhar à vista", "Align to view"),
    ("scene.lens.align_tip", "Leva a câmera de cena para a posição e direção da viewport.", "Moves the scene camera to the viewport position and direction."),
    // cena
    ("scene.settings", "Cena", "Scene"),
    ("scene.settings.hint", "Nada selecionado: luz principal e câmera de render do projeto.", "Nothing selected: the project's main light and render camera."),
    ("scene.settings.units", "Unidade", "Unit"),
    ("scene.settings.meters", "metros", "meters"),
    ("scene.settings.centimeters", "centímetros", "centimeters"),
    ("scene.settings.objects", "objetos", "objects"),
    ("scene.settings.pickable", "selecionáveis na viewport", "pickable in the viewport"),
    ("scene.light.main", "Luz principal (estilizada)", "Main light (stylized)"),
    ("scene.light.azimuth", "Azimute", "Azimuth"),
    ("scene.light.elevation", "Elevação", "Elevation"),
    ("scene.light.color", "Cor", "Color"),
    ("scene.light.intensity", "Intensidade", "Intensity"),
    ("scene.light.ambient", "Ambiente", "Ambient"),
    ("scene.camera.render", "Câmera de render", "Render camera"),
    ("scene.camera.from_view", "Definir pela vista atual", "Set from current view"),
    ("scene.camera.from_view_tip", "Grava a vista da viewport como câmera de render (um passo de desfazer).", "Stores the viewport view as the render camera (one undo step)."),
    // vista
    ("scene.view.perspective", "Perspectiva", "Perspective"),
    ("scene.view.front", "Frente", "Front"),
    ("scene.view.back", "Trás", "Back"),
    ("scene.view.right", "Direita", "Right"),
    ("scene.view.left", "Esquerda", "Left"),
    ("scene.view.top", "Topo", "Top"),
    ("scene.view.bottom", "Base", "Bottom"),
    ("scene.view.focus", "Focar seleção", "Frame selection"),
    ("scene.view.game", "Game view (esconde overlays)", "Game view (hides overlays)"),
    ("scene.view.through_camera", "Ver pela câmera da cena", "Look through the scene camera"),
    ("scene.view.speed", "Velocidade da câmera", "Camera speed"),
    ("scene.view.speed_tip", "Escala a velocidade do voo (RMB+WASD). RMB + roda também ajusta.", "Scales fly speed (RMB+WASD). RMB + wheel also adjusts it."),
    ("scene.space.world", "Espaço: mundo", "Space: world"),
    ("scene.space.local", "Espaço: local", "Space: local"),
    ("scene.guides", "Guias de composição", "Composition guides"),
    ("scene.guides.thirds", "Regra dos terços", "Rule of thirds"),
    ("scene.guides.center", "Centro", "Center"),
    ("scene.guides.safe", "Áreas seguras", "Safe areas"),
    ("scene.guides.frame", "Moldura do quadro", "Frame border"),
    // snap
    ("scene.snap.grid", "Snap de posição (grade)", "Position snap (grid)"),
    ("scene.snap.grid_size", "Tamanho da grade", "Grid size"),
    ("scene.snap.rotation", "Snap de rotação", "Rotation snap"),
    ("scene.snap.rot_size", "Passo de rotação", "Rotation step"),
    ("scene.snap.scale", "Snap de escala", "Scale snap"),
    ("scene.snap.scale_size", "Passo de escala", "Scale step"),
    ("scene.snap.surface", "Surface snapping", "Surface snapping"),
    ("scene.snap.align_normal", "Alinhar à normal da superfície", "Align to surface normal"),
    ("scene.snap.offset", "Distância", "Offset"),
    ("scene.snap.absolute", "Absoluto", "Absolute"),
    ("scene.snap.incremental", "Incremental", "Incremental"),
    ("scene.snap.abs_tip", "Absoluto: a pose final cai na grade. Incremental: o deslocamento cai na grade.", "Absolute: the final pose lands on the grid. Incremental: the displacement lands on the grid."),
    ("scene.snap.ctrl_tip", "Segurar Ctrl inverte temporariamente o snap. End: pousar no chão. Z: surface snapping.", "Holding Ctrl temporarily inverts snapping. End: snap to floor. Z: surface snapping."),
    ("scene.hud.move", "Mover", "Move"),
    ("scene.hud.rotate", "Girar", "Rotate"),
    ("scene.hud.scale", "Escalar", "Scale"),
    ("scene.hud.grid", "grade", "grid"),
    // mensagens
    ("scene.msg.copied", "objeto(s) copiado(s)", "object(s) copied"),
    ("scene.msg.nothing_to_copy", "Nada para copiar (o personagem não se copia).", "Nothing to copy (the character cannot be copied)."),
    ("scene.msg.clipboard_empty", "A área de transferência está vazia.", "The clipboard is empty."),
    ("scene.msg.already_on_floor", "Já está no chão.", "Already on the floor."),
    ("scene.msg.library_not_ready", "A Biblioteca ainda está abrindo.", "The Library is still opening."),
    ("scene.msg.load_failed", "não foi possível carregar a malha", "could not load the mesh"),
    ("scene.msg.nothing_placeable", "Nenhum ativo 3D colocável no que foi solto.", "No placeable 3D asset in what was dropped."),
    ("scene.msg.select_asset_in_drawer", "Selecione um ativo no Content Drawer primeiro.", "Select an asset in the Content Drawer first."),
    ("scene.msg.vertex_snap_phase2", "Snap de vértice (V) é Fase 2; use grade, rotação, escala, End (chão) e Z (superfície).", "Vertex snap (V) is Phase 2; use grid, rotation, scale, End (floor) and Z (surface)."),
    // comandos (histórico / status)
    ("scene.cmd.transform", "Transformar", "Transform"),
    ("scene.cmd.duplicate", "Duplicar", "Duplicate"),
    ("scene.cmd.delete", "Apagar", "Delete"),
    ("scene.cmd.add", "Criar objeto", "Add object"),
    ("scene.cmd.rename", "Renomear", "Rename"),
    ("scene.cmd.lock", "Bloquear/desbloquear", "Lock/unlock"),
    ("scene.cmd.properties", "Propriedades", "Properties"),
    ("scene.cmd.reparent", "Reparentar", "Reparent"),
    ("scene.cmd.visibility", "Visibilidade", "Visibility"),
    ("scene.cmd.replace", "Substituir ativo", "Replace asset"),
    ("scene.cmd.assets", "Registro de ativos", "Asset registry"),
    ("scene.cmd.camera", "Câmera", "Camera"),
    ("scene.cmd.light", "Luz", "Light"),
    ("scene.cmd.background", "Fundo", "Background"),
    ("scene.cmd.batch", "Várias alterações", "Several changes"),
    // arquivo
    ("scene.dialog.save", "Salvar cena", "Save scene"),
    ("scene.dialog.open", "Abrir cena", "Open scene"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicates_no_empty_strings_and_no_clash_with_other_tables() {
        let others: HashSet<&str> = crate::i18n::TABLE.iter().map(|(k, _, _)| *k).chain(crate::i18n_lib::LIB_TABLE.iter().map(|(k, _, _)| *k)).collect();
        let mut seen = HashSet::new();
        for (k, pt, en) in SCENE_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
            assert!(k.starts_with("scene."));
            assert!(!others.contains(k), "{k} já existe em outra tabela");
        }
    }

    /// Toda chave `"scene.*"` escrita no código do workspace precisa existir na tabela.
    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/scene_ui/mod.rs"),
            include_str!("app/scene_ui/actions.rs"),
            include_str!("app/scene_ui/panels.rs"),
            include_str!("app/scene_ui/viewport.rs"),
            include_str!("app/scene_ui/script.rs"),
            include_str!("scene/camview.rs"),
        ];
        let known: HashSet<&str> = SCENE_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"scene.") {
                let tail = &rest[i + 1..];
                let Some(end) = tail.find('"') else { break };
                let key = &tail[..end];
                if key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_') && !known.contains(key) {
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
    fn scene_labels_resolve_in_both_languages() {
        for (k, pt, en) in SCENE_TABLE {
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::PtBr, k), *pt);
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::En, k), *en);
        }
    }
}
