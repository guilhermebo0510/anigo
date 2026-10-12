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

fn index() -> &'static std::collections::HashMap<&'static str, (&'static str, &'static str)> {
    static IDX: std::sync::OnceLock<std::collections::HashMap<&'static str, (&'static str, &'static str)>> = std::sync::OnceLock::new();
    IDX.get_or_init(|| {
        let mut m = std::collections::HashMap::with_capacity(TABLE.len() + crate::i18n_lib::LIB_TABLE.len());
        for (k, pt, en) in TABLE.iter().chain(crate::i18n_lib::LIB_TABLE.iter()) {
            m.entry(*k).or_insert((*pt, *en));
        }
        m
    })
}

pub fn tr_in(lang: Lang, key: &str) -> &str {
    if let Some((pt, en)) = index().get(key) {
        return match lang {
            Lang::PtBr => pt,
            Lang::En => en,
        };
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
    // Personagem — zonas anatômicas
    ("zone.global_silhouette", "Proporções globais e silhueta", "Global proportions & silhouette"),
    ("zone.craniofacial", "Crânio e estrutura facial", "Skull & facial structure"),
    ("zone.eyes", "Olhos e órbitas", "Eyes & orbits"),
    ("zone.eyebrows", "Sobrancelhas", "Eyebrows"),
    ("zone.nose", "Nariz", "Nose"),
    ("zone.mouth_lips", "Boca e lábios", "Mouth & lips"),
    ("zone.jaw_chin", "Mandíbula, queixo e linha V", "Jaw, chin & V-line"),
    ("zone.ears", "Orelhas", "Ears"),
    ("zone.neck_trapezius", "Pescoço e trapézio", "Neck & trapezius"),
    ("zone.shoulders_clavicles", "Ombros, clavículas e dorsal", "Shoulders, clavicles & back"),
    ("zone.chest_pectorals", "Tórax e peitorais", "Chest & pectorals"),
    ("zone.bust_female", "Busto", "Bust"),
    ("zone.abdomen_waist", "Abdômen, cintura e flancos", "Abdomen, waist & flanks"),
    ("zone.pelvis_hips", "Pelve e quadril", "Pelvis & hips"),
    ("zone.gluteus", "Glúteos", "Glutes"),
    ("zone.upper_limbs", "Braços, antebraços e cotovelos", "Arms, forearms & elbows"),
    ("zone.hands_fingers", "Mãos e dedos", "Hands & fingers"),
    ("zone.lower_limbs", "Pernas, joelhos e pés", "Legs, knees & feet"),
    // Personagem — proporções e somatótipo
    ("prop.height", "Altura", "Height"),
    ("prop.head_scale", "Tamanho da cabeça", "Head size"),
    ("prop.head_ratio", "Cabeças de altura", "Heads tall"),
    ("prop.shoulders", "Ombros", "Shoulders"),
    ("prop.torso", "Tronco", "Torso"),
    ("prop.legs", "Pernas", "Legs"),
    ("prop.arms", "Braços", "Arms"),
    ("prop.neck", "Pescoço", "Neck"),
    ("somatotype.athletic", "Atlético", "Athletic"),
    ("somatotype.stocky", "Corpulento", "Stocky"),
    ("somatotype.slender", "Esguio", "Slender"),
    ("somatotype.balanced", "Equilibrado", "Balanced"),
    ("somatotype.endo", "Endomorfo", "Endomorph"),
    ("somatotype.meso", "Mesomorfo", "Mesomorph"),
    ("somatotype.ecto", "Ectomorfo", "Ectomorph"),
    // Personagem — presets
    ("preset.shonen_hero", "Herói Shonen", "Shonen Hero"),
    ("preset.shonen_hero.desc", "Proporção heróica 6,5 cabeças, atlética, com V-taper e queixo em V.", "Heroic 6.5-head athletic build with V-taper and a V-line jaw."),
    ("preset.shojo_idol", "Ídolo Shojo", "Shojo Idol"),
    ("preset.shojo_idol.desc", "Silhueta delicada de 6,2 cabeças, olhos expressivos e cintura marcada.", "Delicate 6.2-head silhouette, expressive eyes and a defined waist."),
    ("preset.muscular_berserker", "Berserker Musculoso", "Muscular Berserker"),
    ("preset.muscular_berserker.desc", "Físico hipertrofiado de 7,5 cabeças, trapézio maciço e mandíbula potente.", "Hypertrophic 7.5-head physique, massive trapezius and a strong jaw."),
    ("preset.plus_size", "Plus Size", "Plus Size"),
    ("preset.plus_size.desc", "Morfologia curvilínea e encorpada, quadril proeminente e volume suave.", "Curvy, full-bodied morphology with prominent hips and soft volume."),
    ("preset.chibi", "Chibi 2,5 cabeças", "Chibi 2.5 heads"),
    ("preset.chibi.desc", "Super-deformed de 2,5 cabeças: crânio grande, olhos gigantes, traços fofos.", "Super-deformed 2.5 heads: big skull, giant eyes, cute features."),
    ("preset.heroic_8_5c", "Heroico 8,5 cabeças", "Heroic 8.5 heads"),
    ("preset.heroic_8_5c.desc", "Proporção fashion heroica: pernas longas, ombros largos, porte esguio.", "Heroic fashion proportion: long legs, broad shoulders, slender bearing."),
    // Personagem — painel e viewport
    ("char.project", "Projeto", "Project"),
    ("char.save", "Salvar", "Save"),
    ("char.open", "Abrir", "Open"),
    ("char.reset_all", "Resetar tudo", "Reset all"),
    ("char.gender", "Gênero", "Gender"),
    ("char.male", "Masculino", "Male"),
    ("char.female", "Feminino", "Female"),
    ("char.dimorphism", "Dimorfismo", "Dimorphism"),
    ("char.somatotype", "Somatótipo", "Somatotype"),
    ("char.proportions", "Proporções", "Proportions"),
    ("char.presets", "Presets de personagem", "Character presets"),
    ("char.apply", "Aplicar", "Apply"),
    ("char.search_hint", "Buscar slider…", "Search sliders…"),
    ("char.no_results", "Nenhum slider encontrado.", "No sliders found."),
    ("char.reset_slider", "Voltar ao padrão", "Reset to default"),
    ("char.female_only", "Específico do corpo feminino", "Specific to the female body"),
    ("char.male_only", "Específico do corpo masculino", "Specific to the male body"),
    ("char.modified", "alterado(s)", "changed"),
    ("char.history", "Histórico", "History"),
    ("char.history_steps", "passos de desfazer", "undo steps"),
    ("char.history_next", "Próximo a desfazer", "Next undo"),
    ("char.history_empty", "Nada a desfazer", "Nothing to undo"),
    ("char.turntable", "Turntable", "Turntable"),
    ("char.light_camera", "Luz presa à câmera", "Light follows camera"),
    ("char.frame", "Enquadrar", "Frame"),
    ("char.view_front", "Frente", "Front"),
    ("char.view_side", "Lado", "Side"),
    ("char.view_back", "Costas", "Back"),
    ("char.view_face", "Rosto", "Face"),
    ("char.no_preview", "Preview cel indisponível: o renderer canônico não iniciou nesta GPU.", "Cel preview unavailable: the canonical renderer did not start on this GPU."),
    ("char.dialog.save", "Salvar projeto", "Save project"),
    ("char.dialog.open", "Abrir projeto", "Open project"),
    ("char.dialog.path", "Caminho do arquivo (.anigo)", "File path (.anigo)"),
    ("char.dialog.cancel", "Cancelar", "Cancel"),
    ("char.dialog.hint", "Digite o caminho completo; o seletor de arquivos nativo ainda não está disponível.", "Type the full path; the native file picker is not available yet."),
    ("char.saved", "Projeto salvo", "Project saved"),
    ("char.opened", "Projeto aberto", "Project opened"),
    ("char.error", "Erro", "Error"),
    ("char.undone", "Desfeito", "Undone"),
    ("char.redone", "Refeito", "Redone"),
    ("char.batch", "Preset / lote de alterações", "Preset / batch of changes"),
    ("char.soon", "Em breve", "Coming soon"),
    ("char.soon.hair", "O editor de cabelo 3D nativo ainda não existe. O núcleo já guarda estes parâmetros no projeto, mas não há comando para editá-los nem malha de cabelo no preview.", "The native 3D hair editor does not exist yet. The core already stores these parameters in the project, but there is no command to edit them and no hair mesh in the preview."),
    ("char.soon.cloth", "O vestuário nativo ainda não existe. O núcleo guarda os parâmetros de tecido abaixo, sem comando de edição nem malha de roupa no preview.", "Native clothing does not exist yet. The core stores the cloth parameters below, with no edit command and no garment mesh in the preview."),
    ("char.soon.accessories", "Os acessórios 3D nativos ainda não existem. O núcleo guarda o encaixe abaixo, sem comando de edição.", "Native 3D accessories do not exist yet. The core stores the attachment below, with no edit command."),
    ("char.soon.paint", "A pintura de textura nativa depende do editor de UV e do sistema de camadas, ainda não portados.", "Native texture painting depends on the UV editor and layer system, which are not ported yet."),
    ("char.soon.stored", "Guardado no projeto (somente leitura)", "Stored in the project (read-only)"),
    ("hair.volume", "Volume", "Volume"),
    ("hair.thickness", "Espessura", "Thickness"),
    ("hair.curvature", "Curvatura", "Curvature"),
    ("hair.strands", "Mechas", "Strands"),
    ("cloth.layer", "Camada", "Layer"),
    ("cloth.tension", "Tensão", "Tension"),
    ("cloth.rigidity", "Rigidez", "Rigidity"),
    ("cloth.gravity", "Gravidade", "Gravity"),
    ("accessory.socket", "Encaixe", "Socket"),
    ("accessory.scale", "Escala", "Scale"),
    ("accessory.offset", "Deslocamento", "Offset"),
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
