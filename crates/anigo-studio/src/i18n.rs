//! Internacionalização mínima (pt-BR padrão, en). Japonês entra junto com a fonte CJK
//! embutida (o egui não faz fallback de fonte do sistema).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    PtBr,
    En,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::PtBr,
    }
}

/// Traduz `key` no idioma atual. Chave desconhecida devolve a própria chave
/// (visível na UI = bug fácil de notar; o teste abaixo impede em produção).
pub fn tr(key: &str) -> &str {
    tr_in(lang(), key)
}

pub fn tr_in(lang: Lang, key: &str) -> &str {
    for (k, pt, en) in TABLE {
        if *k == key {
            return match lang {
                Lang::PtBr => pt,
                Lang::En => en,
            };
        }
    }
    // O `&str` devolvido vive tanto quanto `key`.
    key
}

/// (chave, pt-BR, en)
pub const TABLE: &[(&str, &str, &str)] = &[
    // Workspaces
    ("workspace.personagem", "Personagem", "Character"),
    ("workspace.modelagem", "Modelagem", "Modeling"),
    ("workspace.posing", "Posing", "Posing"),
    ("workspace.shading", "Shading", "Shading"),
    ("workspace.iluminacao", "Iluminação", "Lighting"),
    ("workspace.cenario", "Cenário", "Scene"),
    ("workspace.animacao", "Animação", "Animation"),
    ("workspace.render", "Render", "Render"),
    ("workspace.biblioteca", "Biblioteca", "Library"),
    // Ferramentas — Personagem
    ("tool.body", "Anatomia e Corpo", "Anatomy & Body"),
    ("tool.face", "Rosto e Olhos", "Face & Eyes"),
    ("tool.hair", "Cabelo 3D", "3D Hair"),
    ("tool.cloth", "Vestuário", "Clothing"),
    ("tool.accessories", "Acessórios 3D", "3D Accessories"),
    ("tool.paint", "Pintura de Textura", "Texture Paint"),
    // Posing
    ("tool.rig", "Estrutura Óssea", "Skeleton"),
    ("tool.ik", "Cinemática Inversa", "Inverse Kinematics"),
    ("tool.poses_preset", "Poses do Manequim", "Mannequin Poses"),
    // Shading
    ("tool.cel_shader", "Toon Ramp e Cel-Shading", "Toon Ramp & Cel Shading"),
    ("tool.rim", "Luz de Borda", "Rim Light"),
    ("tool.outline", "Contorno", "Outline"),
    ("tool.palette", "Paleta de Sombras", "Shadow Palette"),
    ("tool.shader_ball", "Shader Ball", "Shader Ball"),
    // Iluminação
    ("tool.sun", "Iluminação Solar", "Sun Light"),
    ("tool.shadows", "Sombras Anime", "Anime Shadows"),
    ("tool.ambient", "Luz Ambiente e Céu", "Ambient & Sky"),
    // Cenário
    ("tool.stage", "Blocagem Greybox", "Greybox Blocking"),
    ("tool.props", "Elementos de Cenário", "Scene Props"),
    ("tool.environment", "Céu e Atmosfera", "Sky & Atmosphere"),
    // Animação
    ("tool.timeline", "Linha do Tempo", "Timeline"),
    ("tool.curves", "Curvas de Interpolação", "Interpolation Curves"),
    ("tool.lipsync", "Sincronia Labial", "Lip Sync"),
    // Render
    ("tool.camera", "Câmera e Lente", "Camera & Lens"),
    ("tool.passes", "Passes de Render", "Render Passes"),
    ("tool.export", "Composição e Exportação", "Compositing & Export"),
    // Biblioteca
    ("tool.browser", "Navegador de Ativos", "Asset Browser"),
    ("tool.search", "Busca e Tags", "Search & Tags"),
    ("tool.filters", "Filtros de Categoria", "Category Filters"),
    // Modelagem — modo Objeto
    ("tool.select_box", "Selecionar", "Select"),
    ("tool.move", "Mover", "Move"),
    ("tool.rotate", "Rotacionar", "Rotate"),
    ("tool.scale", "Escalar", "Scale"),
    ("tool.add_primitive", "Adicionar primitiva", "Add primitive"),
    // Modelagem — modo Edição
    ("tool.extrude", "Extrudar", "Extrude"),
    ("tool.inset", "Inset", "Inset"),
    ("tool.bevel", "Chanfro (Bevel)", "Bevel"),
    ("tool.loop_cut", "Corte em Loop", "Loop Cut"),
    ("tool.knife", "Faca (Bisect)", "Knife (Bisect)"),
    ("tool.bridge", "Ponte entre Loops", "Bridge Edge Loops"),
    ("tool.merge", "Unir (Merge)", "Merge"),
    ("tool.fill", "Preencher", "Fill"),
    ("tool.dissolve", "Dissolver", "Dissolve"),
    ("tool.smooth_verts", "Suavizar Vértices", "Smooth Vertices"),
    // Modelagem — Escultura
    ("sculpt.draw", "Desenhar", "Draw"),
    ("sculpt.clay", "Argila", "Clay"),
    ("sculpt.clay_strips", "Tiras de Argila", "Clay Strips"),
    ("sculpt.smooth", "Suavizar", "Smooth"),
    ("sculpt.flatten", "Achatar", "Flatten"),
    ("sculpt.inflate", "Inflar", "Inflate"),
    ("sculpt.pinch", "Pinçar", "Pinch"),
    ("sculpt.grab", "Agarrar", "Grab"),
    ("sculpt.snake_hook", "Gancho", "Snake Hook"),
    ("sculpt.crease", "Vinco", "Crease"),
    ("sculpt.mask", "Máscara", "Mask"),
    ("sculpt.fill", "Preencher", "Fill"),
    ("sculpt.scrape", "Raspar", "Scrape"),
    ("sculpt.nudge", "Empurrar", "Nudge"),
    ("sculpt.layer", "Camada", "Layer"),
    ("sculpt.elastic", "Agarrar Elástico", "Elastic Grab"),
    ("sculpt.mask_smooth", "Suavizar Máscara", "Smooth Mask"),
    ("sculpt.mask_sharpen", "Afiar Máscara", "Sharpen Mask"),
    ("sculpt.mask_invert", "Inverter Máscara (pincel)", "Invert Mask (brush)"),
    ("sculpt.mask_clear", "Apagar Máscara (pincel)", "Clear Mask (brush)"),
    // Modos
    ("mode.object", "Modo Objeto", "Object Mode"),
    ("mode.edit", "Modo Edição", "Edit Mode"),
    ("mode.sculpt", "Escultura", "Sculpt Mode"),
    ("select.vertex", "Vértice", "Vertex"),
    ("select.edge", "Aresta", "Edge"),
    ("select.face", "Face", "Face"),
    // Viewport
    ("viewport.solid", "Sólido", "Solid"),
    ("viewport.wireframe", "Wireframe", "Wireframe"),
    ("viewport.cel", "Cel Shading", "Cel Shading"),
    ("viewport.overlays", "Overlays", "Overlays"),
    ("viewport.xray", "Raio-X", "X-Ray"),
    // Painéis e geral
    ("panel.tools", "Ferramentas", "Tools"),
    ("panel.inspector", "Propriedades", "Properties"),
    ("panel.outliner", "Cena", "Outliner"),
    ("panel.history", "Histórico", "History"),
    ("panel.modifiers", "Modificadores", "Modifiers"),
    ("panel.mesh_stats", "Estatísticas", "Statistics"),
    ("panel.normals", "Normais", "Normals"),
    ("panel.uv", "UV", "UV"),
    ("action.undo", "Desfazer", "Undo"),
    ("action.redo", "Refazer", "Redo"),
    ("action.add", "Adicionar", "Add"),
    ("action.delete", "Excluir", "Delete"),
    ("status.ready", "Pronto", "Ready"),
    ("status.gpu", "GPU", "GPU"),
    ("lib.categories", "Categorias", "Categories"),
    ("lib.search_hint", "Buscar por nome ou tag…", "Search by name or tag…"),
    ("lib.use_in_scene", "Usar no Cenário", "Use in Scene"),
    ("lib.inspect", "Inspecionar", "Inspect"),
    ("placeholder.not_ported", "Este workspace ainda usa o app Tauri; a versão nativa entra nas próximas fases.", "This workspace still lives in the Tauri app; the native version lands in the next phases."),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WorkspaceId;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_keys_and_no_empty_strings() {
        let mut seen = HashSet::new();
        for (k, pt, en) in TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
        }
    }

    #[test]
    fn every_workspace_and_tool_label_is_translated() {
        for w in WorkspaceId::ALL {
            let key = format!("workspace.{}", w.key());
            assert_ne!(tr_in(Lang::PtBr, &key), key);
            for t in w.tools() {
                assert_ne!(tr_in(Lang::PtBr, t.label_key), t.label_key, "{}", t.label_key);
                assert_ne!(tr_in(Lang::En, t.label_key), t.label_key);
            }
        }
    }

    #[test]
    fn unknown_key_returns_itself() {
        assert_eq!(tr_in(Lang::En, "x.y"), "x.y");
    }
}
