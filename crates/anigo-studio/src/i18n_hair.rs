//! Textos (pt-BR / en) da ferramenta de cabelo (Modo Cabelo da Modelagem). Tabela própria para não
//! conflitar com `i18n.rs`; `i18n::tr_in` consulta esta tabela depois da principal.

/// (chave, pt-BR, en)
pub const HAIR_TABLE: &[(&str, &str, &str)] = &[
    ("mode.hair", "Cabelo", "Hair"),
    // ferramentas do shelf
    ("hair.draw", "Desenhar mecha", "Draw strand"),
    ("hair.edit", "Editar mecha e pontos", "Edit strand and points"),
    ("hair.comb", "Pentear", "Comb"),
    // seções da mecha
    ("hair.section.flat", "Fita plana", "Flat ribbon"),
    ("hair.section.arc", "Fita curva", "Curved ribbon"),
    ("hair.section.triangle", "Mecha triangular", "Triangular lock"),
    ("hair.section.tube", "Mecha tubular", "Tube lock"),
    // painel
    ("hairtool.tools", "Ferramentas", "Tools"),
    ("hairtool.hint.draw", "Arraste sobre a cabeça: o traço vira uma mecha (a direção do traço é raiz → ponta).", "Drag over the head: the stroke becomes a strand (stroke direction is root → tip)."),
    ("hairtool.hint.edit", "Clique numa mecha para selecionar; arraste os pontos; Ctrl+clique na mecha insere um ponto.", "Click a strand to select it; drag its points; Ctrl+click on the strand inserts a point."),
    ("hairtool.hint.comb", "Arraste: as mechas sob o pincel giram na direção do movimento (comprimento preservado).", "Drag: strands under the brush turn toward the motion (length is preserved)."),
    ("hairtool.target", "Cabeça (alvo)", "Head (target)"),
    ("hairtool.no_target", "Selecione um objeto-cabeça no modo Objeto e volte ao modo Cabelo.", "Select a head object in Object mode and return to Hair mode."),
    ("hairtool.use_active", "Usar o objeto ativo como cabeça", "Use active object as head"),
    ("hairtool.groups", "Grupos de cabelo", "Hair groups"),
    ("hairtool.no_groups", "Nenhum grupo ainda: o primeiro traço cria o Grupo 1.", "No groups yet: the first stroke creates Group 1."),
    ("hairtool.group", "Grupo", "Group"),
    ("hairtool.group_new", "Novo", "New"),
    ("hairtool.group_duplicate", "Duplicar", "Duplicate"),
    ("hairtool.group_flip", "Espelhar", "Flip"),
    ("hairtool.group_flip_tip", "Cria um grupo novo com as mechas espelhadas em X (Shift+M)", "Creates a new group with the strands mirrored in X (Shift+M)"),
    ("hairtool.symmetry", "Simetria X", "X symmetry"),
    ("hairtool.move_to", "Mover mecha para o grupo…", "Move strand to group…"),
    ("hairtool.material", "Material do grupo", "Group material"),
    ("hairtool.color_base", "Cor base", "Base color"),
    ("hairtool.color_shade", "Cor da sombra", "Shade color"),
    ("hairtool.color_highlight", "Cor do brilho", "Highlight color"),
    ("hairtool.root_ao", "AO da raiz", "Root AO"),
    ("hairtool.normal_bias", "Normais para fora da cabeça", "Normals away from head"),
    ("hairtool.uv_length_v", "UV: comprimento em V", "UV: length along V"),
    ("hairtool.uv_length_u", "UV: comprimento em U (tangente = fio)", "UV: length along U (tangent = strand)"),
    ("hairtool.strand_selected", "Mecha selecionada", "Selected strand"),
    ("hairtool.strand_new", "Parâmetros das mechas novas", "New strand parameters"),
    ("hairtool.width", "Largura", "Width"),
    ("hairtool.thickness", "Espessura", "Thickness"),
    ("hairtool.twist", "Torção", "Twist"),
    ("hairtool.curl", "Curvatura da seção", "Section curl"),
    ("hairtool.tip_taper", "Afinar ponta", "Tip taper"),
    ("hairtool.taper_power", "Expoente do afinamento", "Taper exponent"),
    ("hairtool.flare", "Bojo", "Flare"),
    ("hairtool.resolution", "Resolução", "Resolution"),
    ("hairtool.clump", "Mecha composta (clump)", "Clump of fibres"),
    ("hairtool.clump_count", "Fios", "Fibres"),
    ("hairtool.clump_spread", "Espalhamento", "Spread"),
    ("hairtool.clump_noise", "Ondulação", "Waviness"),
    ("hairtool.clump_width", "Largura do fio", "Fibre width"),
    ("hairtool.apply_group", "Aplicar ao grupo", "Apply to group"),
    ("hairtool.apply_group_tip", "Copia estes parâmetros para todas as mechas do grupo ativo", "Copies these parameters to every strand of the active group"),
    ("hairtool.strand_duplicate", "Duplicar", "Duplicate"),
    ("hairtool.strand_delete", "Excluir", "Delete"),
    ("hairtool.simplify", "Suavizar pontos", "Smooth points"),
    ("hairtool.simplify_tip", "Reduz pontos de controle (Ramer–Douglas–Peucker)", "Reduces control points (Ramer–Douglas–Peucker)"),
    ("hairtool.points", "pontos · comprimento", "points · length"),
    ("hairtool.drawing", "Desenho e pente", "Drawing and comb"),
    ("hairtool.lift_root", "Elevação na raiz", "Root lift"),
    ("hairtool.lift_tip", "Elevação na ponta", "Tip lift"),
    ("hairtool.smooth", "Suavização do traço", "Stroke smoothing"),
    ("hairtool.rdp", "Tolerância dos pontos", "Point tolerance"),
    ("hairtool.step", "Passo da reamostragem", "Resample step"),
    ("hairtool.comb_radius", "Raio do pente (px)", "Comb radius (px)"),
    ("hairtool.comb_strength", "Força do pente", "Comb strength"),
    ("hairtool.comb_all", "Pentear todos os grupos visíveis", "Comb all visible groups"),
    ("hairtool.physics", "Física (bounce)", "Physics (bounce)"),
    ("hairtool.physics_note", "Gera cadeias neutras (HairChainSpec) para o motor de física; elas são derivadas das mechas atuais.", "Generates neutral chains (HairChainSpec) for the physics engine; they are derived from the current strands."),
    ("hairtool.bounce_groups", "Grupos", "Groups"),
    ("hairtool.bones", "Ossos", "Bones"),
    ("hairtool.auto_bounce", "Auto-gerar grupos de bounce", "Auto-generate bounce groups"),
    ("hairtool.bounce_created", "grupos de bounce criados", "bounce groups created"),
    ("hairtool.fixed_point", "Ponto fixo", "Fixed point"),
    ("hairtool.stiffness", "Rigidez", "Stiffness"),
    ("hairtool.drag", "Arrasto", "Drag"),
    ("hairtool.gravity", "Gravidade", "Gravity"),
    ("hairtool.hit_radius", "Raio de colisão", "Hit radius"),
    ("hairtool.chains", "cadeias de física", "physics chains"),
    ("hairtool.export_chains", "Exportar cadeias (JSON)", "Export chains (JSON)"),
    ("hairtool.chains_saved", "Cadeias salvas em", "Chains saved to"),
    ("hairtool.no_chains", "Sem cadeias: gere os grupos de bounce primeiro.", "No chains: generate the bounce groups first."),
    ("hairtool.output", "Saída", "Output"),
    ("hairtool.export_glb", "Exportar grupos para a Biblioteca (GLB)", "Export groups to the Library (GLB)"),
    ("hairtool.regenerate", "Regenerar malhas", "Regenerate meshes"),
    ("hairtool.output_note", "Cada grupo é um objeto da cena (um GLB por grupo). A cor do material não vai no GLB.", "Each group is a scene object (one GLB per group). Material color is not written to the GLB."),
    ("hairtool.nothing_to_export", "Nenhum grupo de cabelo para exportar", "No hair group to export"),
    ("hairtool.exported", "Grupos de cabelo exportados:", "Hair groups exported:"),
    ("hairtool.stroke_cancelled", "Traço cancelado", "Stroke cancelled"),
    ("hairtool.start_on_head", "Comece o traço sobre a cabeça", "Start the stroke on the head"),
    ("hairtool.strands", "mechas", "strands"),
    ("hairtool.shortcuts", "Atalhos", "Shortcuts"),
    ("hairtool.keys.1", "D desenhar · V editar · C pentear · Esc cancela o traço", "D draw · V edit · C comb · Esc cancels the stroke"),
    ("hairtool.keys.2", "Shift+D duplica · X/Del remove ponto · Shift+X remove mecha", "Shift+D duplicates · X/Del removes point · Shift+X removes strand"),
    ("hairtool.keys.3", "M simetria X do grupo · Shift+M espelha o grupo", "M group X symmetry · Shift+M flips the group"),
    ("hairtool.keys.4", "[ ] raio do pente · Ctrl+clique na mecha insere ponto", "[ ] comb radius · Ctrl+click on a strand inserts a point"),
    ("hairtool.keys.5", "Ctrl+Z desfaz · Ctrl+Shift+Z refaz · Alt+arrastar / botão do meio orbita", "Ctrl+Z undo · Ctrl+Shift+Z redo · Alt+drag / middle button orbits"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicates_no_empty_and_no_clash_with_the_main_table() {
        let mut seen = std::collections::HashSet::new();
        let main: std::collections::HashSet<&str> = crate::i18n::TABLE.iter().map(|t| t.0).collect();
        for (k, pt, en) in HAIR_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!main.contains(k), "chave {k} já existe na tabela principal");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
        }
    }

    #[test]
    fn every_hair_tool_label_and_section_is_translated() {
        use crate::i18n::{tr_in, Lang};
        for t in crate::app::inspector::MODELAGEM_HAIR {
            for l in [Lang::PtBr, Lang::En] {
                assert_ne!(tr_in(l, t.label_key), t.label_key);
            }
        }
        for k in anigo_modeling::hair::SectionKind::ALL {
            assert_ne!(tr_in(Lang::En, k.key()), k.key());
        }
    }
}
