//! Textos do workspace Shading (pt-BR / en). Chaves `shading.*`; o teste confere que toda chave usada
//! no código existe aqui, que não há duplicata nem texto vazio e que todo parâmetro/preset tem rótulo.

/// (chave, pt-BR, en)
pub const SHADING_TABLE: &[(&str, &str, &str)] = &[
    ("shading.targets", "Materiais", "Materials"),
    ("shading.src.character", "Personagem", "Character"),
    ("shading.src.scene", "Cenário", "Scene"),
    ("shading.src.modeling", "Modelagem", "Modeling"),
    ("shading.search", "Buscar", "Search"),
    ("shading.empty_character", "O projeto não tem materiais.", "The project has no materials."),
    ("shading.empty_scene", "Nenhum objeto com material na cena. Coloque objetos no workspace Cenário.", "No object with a material in the scene. Place objects in the Scene workspace."),
    ("shading.empty_modeling", "Nenhum objeto na Modelagem. Crie um no workspace Modelagem.", "No object in Modeling. Create one in the Modeling workspace."),
    ("shading.no_target", "Escolha um material na lista à esquerda.", "Pick a material from the list on the left."),
    ("shading.cmd.material", "Material", "Material"),
    ("shading.copy", "Copiar", "Copy"),
    ("shading.paste", "Colar", "Paste"),
    ("shading.copied", "Material copiado", "Material copied"),
    ("shading.pasted", "Material colado", "Material pasted"),
    ("shading.save", "Salvar", "Save"),
    ("shading.on", "ligado", "on"),
    ("shading.reset_param", "Voltar ao padrão", "Reset to default"),
    ("shading.reset_group", "Voltar o grupo ao padrão", "Reset the group to default"),
    ("shading.reset_all", "Resetar tudo", "Reset all"),
    ("shading.reset_all_tip", "Devolve o material inteiro ao padrão do renderer (um passo de desfazer).", "Returns the whole material to the renderer defaults (one undo step)."),
    ("shading.gate_off", "Este controle só tem efeito com o recurso acima ligado.", "This control only has an effect while the feature above is on."),
    ("shading.anime_on", "Ligar recursos anime", "Enable anime features"),
    ("shading.anime_on_tip", "Liga contorno com normais suavizadas, afinar com a distância, taper e tinta de linha por material (hoje desligados por padrão).", "Turns on smoothed-normal outline, distance thinning, taper and per-material line ink (off by default)."),
    ("shading.lib_opening", "Abrindo a Biblioteca…", "Opening the Library…"),
    ("shading.g.base", "Base / Sombra", "Base / Shadow"),
    ("shading.g.ramp", "Rampa 2D", "2D Ramp"),
    ("shading.g.specular", "Especular", "Specular"),
    ("shading.g.rim", "Rim (luz de borda)", "Rim light"),
    ("shading.g.matcap", "Matcap", "Matcap"),
    ("shading.g.outline", "Contorno (por material)", "Outline (per material)"),
    ("shading.g.face", "Face SDF", "Face SDF"),
    ("shading.g.emission", "Emissão", "Emission"),
    ("shading.g.eye", "Olho anime", "Anime eye"),
    ("shading.g.textures", "Texturas", "Textures"),
    ("shading.g.base.hint", "Cor base, cor de sombra, limiar e suavidade da sombra. Suavidade 0 = cel duro; mais alto = ilustração.", "Base color, shade color, shadow threshold and softness. Softness 0 = hard cel; higher = illustration."),
    ("shading.g.ramp.hint", "A rampa decide o fator de luz por linha (0 contínua, 1 um degrau, 2 dois, 3 três). Edite o gradiente de cada linha abaixo.", "The ramp decides the light factor per row (0 continuous, 1 one step, 2 two, 3 three). Edit each row's gradient below."),
    ("shading.g.specular.hint", "Brilho em degrau (angel ring): tamanho e suavidade controlam a forma.", "Stepped highlight (angel ring): size and softness shape it."),
    ("shading.g.rim.hint", "Fresnel estilizado somado à luz; funciona melhor com a luz atrás do objeto.", "Stylized Fresnel added to the light; works best with the light behind the object."),
    ("shading.g.matcap.hint", "Sem textura de matcap o renderer usa um matcap branco neutro: o efeito é multiplicar/somar brilho uniforme.", "Without a matcap texture the renderer uses a neutral white matcap: the effect is a uniform brightness multiply/add."),
    ("shading.g.outline.hint", "Casco invertido por material. Normais suavizadas fecham frestas em arestas duras; falloff afina a linha ao longe; taper dá peso de traço; tinta deriva a cor da linha da base.", "Inverted hull per material. Smoothed normals close gaps on hard edges; falloff thins far lines; taper gives stroke weight; ink derives the line color from the base."),
    ("shading.g.face.hint", "Sombra facial por mapa SDF: a sombra é desenhada pelo mapa, não pelo terminador. Precisa de um mapa (procedural ou da Biblioteca) e da luz girando em torno do rosto.", "Face shadow from an SDF map: the map draws the shadow, not the terminator. Needs a map (procedural or from the Library) and the light moving around the face."),
    ("shading.g.emission.hint", "Brilho próprio somado depois da iluminação.", "Self-glow added after lighting."),
    ("shading.g.eye.hint", "Highlights desenhados à mão do olho anime. O parallax da íris precisa de textura (não suportada).", "Hand-drawn anime eye highlights. Iris parallax needs a texture (unsupported)."),
    ("shading.p.base_color", "Cor base", "Base color"),
    ("shading.p.shade_color", "Cor de sombra", "Shade color"),
    ("shading.p.shade_color.tip", "Cor da região na sombra. O renderer a multiplica pela luz ambiente: use tons claros.", "Color of the shadowed area. The renderer multiplies it by ambient light: use light tones."),
    ("shading.p.shadow_threshold", "Limiar da sombra", "Shadow threshold"),
    ("shading.p.shadow_threshold.tip", "Onde (half-lambert) começa a sombra. 0,5 = no terminador.", "Where (half-lambert) the shadow starts. 0.5 = at the terminator."),
    ("shading.p.shadow_smoothness", "Suavidade da sombra", "Shadow softness"),
    ("shading.p.shadow_smoothness.tip", "Largura da penumbra. 0 = cel duro.", "Penumbra width. 0 = hard cel."),
    ("shading.p.toon_steps", "Degraus (0 = contínuo)", "Steps (0 = continuous)"),
    ("shading.p.hue_shift", "Matiz da sombra (°)", "Shadow hue shift (°)"),
    ("shading.p.hue_shift.tip", "Gira o matiz da sombra (negativo = frio).", "Rotates the shadow hue (negative = cool)."),
    ("shading.p.ao_intensity", "Intensidade do AO", "AO intensity"),
    ("shading.p.ao_intensity.tip", "Só age com o canal de AO da malha (cor de vértice R).", "Only acts with the mesh AO channel (vertex color R)."),
    ("shading.p.mtoon_shade_toony", "Quantizar textura de sombra", "Quantize shade texture"),
    ("shading.p.mtoon_shade_toony.tip", "MToon: quantiza a textura de sombra. Sem textura de sombra no renderer, não muda a imagem.", "MToon: quantizes the shade texture. Without a shade texture in the renderer it does not change the image."),
    ("shading.p.mtoon_second_shade_shift", "2ª sombra: deslocamento", "2nd shade: shift"),
    ("shading.p.mtoon_second_shade_shift.tip", "Só com a textura da 2ª sombra (não suportada pelo renderer ainda).", "Only with the 2nd shade texture (not yet supported by the renderer)."),
    ("shading.p.mtoon_second_shade_softness", "2ª sombra: suavidade", "2nd shade: softness"),
    ("shading.p.mtoon_second_shade_softness.tip", "Só com a textura da 2ª sombra (não suportada pelo renderer ainda).", "Only with the 2nd shade texture (not yet supported by the renderer)."),
    ("shading.p.ramp_blend", "Mistura da rampa", "Ramp blend"),
    ("shading.p.ramp_blend.tip", "0 = rampa desligada (fator de luz analítico). 1 = a rampa decide.", "0 = ramp off (analytic light factor). 1 = the ramp decides."),
    ("shading.p.ramp_row", "Linha da rampa", "Ramp row"),
    ("shading.p.ramp_row.tip", "0 contínua, 1 um degrau, 2 dois, 3 três; fração interpola.", "0 continuous, 1 one step, 2 two, 3 three; fractions interpolate."),
    ("shading.p.spec_intensity", "Intensidade", "Intensity"),
    ("shading.p.specular_color", "Cor do brilho", "Highlight color"),
    ("shading.p.spec_power", "Potência", "Power"),
    ("shading.p.specular_size", "Tamanho", "Size"),
    ("shading.p.specular_softness", "Suavidade", "Softness"),
    ("shading.p.specular_offset", "Deslocamento", "Offset"),
    ("shading.p.rim_intensity", "Intensidade", "Intensity"),
    ("shading.p.rim_spread", "Propagação", "Spread"),
    ("shading.p.rim_spread.tip", "Largura angular do rim.", "Angular width of the rim."),
    ("shading.p.rim_color", "Cor do rim", "Rim color"),
    ("shading.p.mtoon_matcap_enabled", "Matcap", "Matcap"),
    ("shading.p.mtoon_matcap_mode", "Modo", "Mode"),
    ("shading.p.mtoon_matcap_intensity", "Intensidade", "Intensity"),
    ("shading.p.outline_width", "Largura (NDC)", "Width (NDC)"),
    ("shading.p.outline_width.tip", "0 desliga o contorno. 0,004 ≈ 2 px a 512 px de altura.", "0 turns the outline off. 0.004 ≈ 2 px at 512 px height."),
    ("shading.p.outline_color", "Cor da linha", "Line color"),
    ("shading.p.outline_opacity", "Opacidade", "Opacity"),
    ("shading.p.outline_smoothness", "Suavidade da borda", "Edge smoothness"),
    ("shading.p.outline_depth_bias", "Bias de profundidade", "Depth bias"),
    ("shading.p.outline_depth_bias.tip", "Na esfera isolada não aparece; ajuda onde o contorno cruza outras malhas.", "Not visible on an isolated sphere; helps where the outline crosses other meshes."),
    ("shading.p.outline_smooth_normals", "Normais suavizadas", "Smoothed normals"),
    ("shading.p.outline_smooth_normals.tip", "Fecha as frestas do casco em arestas duras.", "Closes hull gaps on hard edges."),
    ("shading.p.outline_distance_falloff", "Afinar com a distância", "Thin with distance"),
    ("shading.p.outline_distance_falloff.tip", "0 = largura constante em pixels; 1 = proporcional ao tamanho na tela.", "0 = constant pixel width; 1 = proportional to on-screen size."),
    ("shading.p.outline_ref_distance", "Distância de referência (m)", "Reference distance (m)"),
    ("shading.p.outline_ref_distance.tip", "Distância em que a largura é a nominal.", "Distance at which the width is the nominal one."),
    ("shading.p.outline_min_width_scale", "Piso da largura", "Width floor"),
    ("shading.p.outline_min_width_scale.tip", "Impede a linha de sumir ao longe.", "Keeps the line from vanishing far away."),
    ("shading.p.outline_taper", "Taper (peso de traço)", "Taper (stroke weight)"),
    ("shading.p.outline_taper.tip", "Afina a linha onde a superfície encara a câmera.", "Thins the line where the surface faces the camera."),
    ("shading.p.outline_ink_blend", "Tinta da base", "Base ink"),
    ("shading.p.outline_ink_blend.tip", "0 = cor da linha; 1 = cor derivada da base (escurecida e saturada).", "0 = line color; 1 = color derived from the base (darkened and saturated)."),
    ("shading.p.outline_ink_darken", "Escurecer a tinta", "Darken ink"),
    ("shading.p.outline_ink_saturation", "Saturar a tinta", "Saturate ink"),
    ("shading.p.face_sdf_enabled", "Sombra facial SDF", "SDF face shadow"),
    ("shading.p.face_shadow_offset", "Deslocamento do limiar", "Threshold offset"),
    ("shading.p.face_shadow_smoothness", "Suavidade", "Softness"),
    ("shading.p.face_sdf_mirror_center", "Espelhar (centro u)", "Mirror (center u)"),
    ("shading.p.face_sdf_mirror_center.tip", "0 = sem espelho. >0 = u do eixo de simetria: a sombra vale para os dois lados da luz (0,25 na esfera UV).", "0 = no mirroring. >0 = u of the symmetry axis: the shadow works on both light sides (0.25 on the UV sphere)."),
    ("shading.p.mtoon_emission_intensity", "Intensidade", "Intensity"),
    ("shading.p.mtoon_emission_intensity.tip", "Ligar a intensidade liga o slot de emissão do renderer.", "Raising the intensity turns on the renderer's emission slot."),
    ("shading.p.mtoon_emission_color", "Cor da emissão", "Emission color"),
    ("shading.p.eye_enabled", "Olho anime", "Anime eye"),
    ("shading.p.eye_depth_scale", "Profundidade da íris", "Iris depth"),
    ("shading.p.eye_depth_scale.tip", "Sem textura de íris não muda a imagem.", "Without an iris texture it does not change the image."),
    ("shading.p.eye_highlight_intensity", "Highlights", "Highlights"),
    ("shading.matcap.multiply", "Multiplicar", "Multiply"),
    ("shading.matcap.additive", "Somar", "Add"),
    ("shading.ramp.row", "Linha", "Row"),
    ("shading.ramp.customize", "Editar gradiente", "Edit gradient"),
    ("shading.ramp.customize_tip", "Cria um gradiente próprio para esta linha a partir da linha do contrato.", "Creates a custom gradient for this row from the contract row."),
    ("shading.ramp.to_contract", "Voltar ao contrato", "Back to contract"),
    ("shading.ramp.to_contract_tip", "Remove o gradiente próprio desta linha.", "Removes this row's custom gradient."),
    ("shading.ramp.interp", "Interpolação", "Interpolation"),
    ("shading.ramp.linear", "Linear", "Linear"),
    ("shading.ramp.steps", "Degraus", "Steps"),
    ("shading.ramp.pos", "Posição", "Position"),
    ("shading.ramp.val", "Valor", "Value"),
    ("shading.ramp.remove", "Remover parada", "Remove stop"),
    ("shading.ramp.add", "Nova parada", "New stop"),
    ("shading.ramp.hint", "Arraste uma parada; duplo clique no gráfico cria outra.", "Drag a stop; double-click the plot to add another."),
    ("shading.ramp.axes", "Horizontal: luz (half-lambert, laranja = limiar). Vertical: fator de luz.", "Horizontal: light (half-lambert, orange = threshold). Vertical: light factor."),
    ("shading.ramp.rows_hint", "linha exata", "exact row"),
    ("shading.ramp.off_hint", "A rampa está desligada (mistura = 0): ligue a mistura para ver o gradiente.", "The ramp is off (blend = 0): raise the blend to see the gradient."),
    ("shading.face.map", "Mapa SDF facial", "Face SDF map"),
    ("shading.face.none", "Nenhum (neutro: não altera a imagem)", "None (neutral: does not change the image)"),
    ("shading.face.procedural", "Procedural (demonstração)", "Procedural (demo)"),
    ("shading.face.use_procedural", "Usar mapa procedural", "Use procedural map"),
    ("shading.face.procedural_tip", "Gera um mapa de demonstração (varredura lateral + nariz). Não é arte final.", "Generates a demo map (lateral sweep + nose). Not final art."),
    ("shading.face.clear", "Limpar mapa", "Clear map"),
    ("shading.face.mirror_on", "Espelhar (0,25)", "Mirror (0.25)"),
    ("shading.face.mirror_tip", "Espelha o mapa quando a luz vem do lado oposto ao autorado.", "Mirrors the map when the light comes from the side opposite to the authored one."),
    ("shading.tex.hint", "Escolha texturas da Biblioteca. O renderer cel só amostra o mapa SDF facial; os demais slots ficam registrados no preset (.anigomat.json) e entram quando o renderer os suportar.", "Pick Library textures. The cel renderer only samples the face SDF map; the other slots are recorded in the preset (.anigomat.json) for when the renderer supports them."),
    ("shading.tex.base", "Textura base", "Base texture"),
    ("shading.tex.shade", "Textura de sombra", "Shade texture"),
    ("shading.tex.shade2", "Textura da 2ª sombra", "2nd shade texture"),
    ("shading.tex.ramp", "Textura de rampa", "Ramp texture"),
    ("shading.tex.ilm", "Mapa ILM", "ILM map"),
    ("shading.tex.matcap", "Matcap", "Matcap"),
    ("shading.tex.emission", "Emissão", "Emission"),
    ("shading.tex.outline_width", "Máscara de contorno", "Outline mask"),
    ("shading.tex.alpha", "Transparência e recorte (alpha mode / cutoff) não existem no material do renderer ainda: ficam fora desta versão.", "Transparency and cutout (alpha mode / cutoff) do not exist in the renderer material yet: left out of this version."),
    ("shading.tex.live", "usado no render", "used in render"),
    ("shading.tex.ref_only", "só referência (não renderizada)", "reference only (not rendered)"),
    ("shading.tex.choose", "Escolher da Biblioteca…", "Pick from Library…"),
    ("shading.tex.clear", "Limpar", "Clear"),
    ("shading.tex.none_in_library", "Nenhuma textura na Biblioteca.", "No textures in the Library."),
    ("shading.view.ball", "Bola", "Ball"),
    ("shading.view.object", "Objeto", "Object"),
    ("shading.view.both", "Ambos", "Both"),
    ("shading.light.follow", "Luz na câmera", "Light on camera"),
    ("shading.light.follow_tip", "Ligada: o azimute é relativo à câmera. Desligada: relativo ao mundo (0° = +Z, positivo para +X).", "On: azimuth is camera-relative. Off: world-relative (0° = +Z, positive toward +X)."),
    ("shading.light.az", "Azimute", "Azimuth"),
    ("shading.light.az_tip", "Ângulo da luz em torno do objeto. Com a face SDF, varra de 0° a 180° e de 0° a -180°.", "Light angle around the object. With the face SDF, sweep 0° to 180° and 0° to -180°."),
    ("shading.light.el", "Elevação", "Elevation"),
    ("shading.bg", "Fundo do preview", "Preview background"),
    ("shading.ab.tip", "Comparar antes (A) e depois (B) lado a lado.", "Compare before (A) and after (B) side by side."),
    ("shading.ab.before", "A (antes)", "A (before)"),
    ("shading.ab.after", "B (atual)", "B (current)"),
    ("shading.ab.set_a", "A = atual", "A = current"),
    ("shading.ab.set_a_tip", "Fixa o material atual como o 'antes' (A).", "Pins the current material as the 'before' (A)."),
    ("shading.ab.swap", "Trocar", "Swap"),
    ("shading.ab.swap_tip", "Aplica A e guarda o atual como novo A (um passo de desfazer).", "Applies A and keeps the current one as the new A (one undo step)."),
    ("shading.ab.revert", "Voltar a A", "Revert to A"),
    ("shading.ab.revert_tip", "Aplica o material A ao alvo (um passo de desfazer).", "Applies material A to the target (one undo step)."),
    ("shading.presets", "Presets de material", "Material presets"),
    ("shading.presets.builtin", "Embutidos", "Built-in"),
    ("shading.presets.library", "Biblioteca", "Library"),
    ("shading.presets.empty", "Nenhum preset salvo. Dê um nome e salve o material atual: ele vira um ativo .anigomat.json da Biblioteca.", "No saved presets. Name and save the current material: it becomes a .anigomat.json asset in the Library."),
    ("shading.preset_name", "Nome do preset", "Preset name"),
    ("shading.save_preset_tip", "Salva o material atual como ativo (.anigomat.json) da Biblioteca, com miniatura em shader ball.", "Saves the current material as a Library asset (.anigomat.json) with a shader ball thumbnail."),
    ("shading.preset_saved", "Preset salvo", "Preset saved"),
    ("shading.preset.skin", "Pele anime", "Anime skin"),
    ("shading.preset.hair", "Cabelo com brilho", "Glossy hair"),
    ("shading.preset.cloth", "Tecido macio", "Soft cloth"),
    ("shading.preset.metal", "Metal (matcap)", "Metal (matcap)"),
    ("shading.preset.hard_cel", "Cel duro", "Hard cel"),
    ("shading.preset.soft", "Ilustração macia", "Soft illustration"),
    ("shading.preset.full_anime", "Anime completo", "Full anime"),
    ("shading.palette", "Paleta", "Palette"),
    ("shading.derive_shade", "Derivar sombra", "Derive shade"),
    ("shading.derive_shade_tip", "Calcula a cor de sombra a partir da base em HSV (matiz, saturação e brilho da receita da Paleta).", "Computes the shade color from the base in HSV (hue, saturation and value from the Palette recipe)."),
    ("shading.link_shade", "Ligar", "Link"),
    ("shading.link_shade_tip", "A cor de sombra acompanha a base enquanto você a edita.", "The shade color follows the base while you edit it."),
    ("shading.palette.derive", "Sombra a partir da base (HSV)", "Shade from base (HSV)"),
    ("shading.palette.hue", "Matiz (°)", "Hue (°)"),
    ("shading.palette.sat", "Saturação ×", "Saturation ×"),
    ("shading.palette.val", "Brilho ×", "Value ×"),
    ("shading.palette.derive_hint", "Mexer nos controles já atualiza a sombra.", "Moving the controls already updates the shade."),
    ("shading.palette.turn", "Girar matiz (°)", "Hue-shift (°)"),
    ("shading.palette.harmony", "Harmonias da cor base", "Base color harmonies"),
    ("shading.harmony.complementary", "Complementar", "Complementary"),
    ("shading.harmony.analogous", "Análoga", "Analogous"),
    ("shading.harmony.triad", "Tríade", "Triad"),
    ("shading.harmony.tetrad", "Tétrade", "Tetrad"),
    ("shading.harmony.split", "Complementar dividida", "Split complementary"),
    ("shading.palette.swatch_tip", "Clique: usar como cor base. Botão direito: outras funções.", "Click: use as base color. Right-click: more actions."),
    ("shading.palette.project", "Paleta do projeto", "Project palette"),
    ("shading.palette.empty", "vazia", "empty"),
    ("shading.palette.add_base", "Base", "Base"),
    ("shading.palette.add_shade", "Sombra", "Shade"),
    ("shading.palette.add_harmony", "Harmonia", "Harmony"),
    ("shading.palette.add", "Adicionar à paleta", "Add to palette"),
    ("shading.palette.remove", "Remover da paleta", "Remove from palette"),
    ("shading.palette.name", "Nome", "Name"),
    ("shading.palette.saved", "Paleta salva", "Palette saved"),
    ("shading.palette.saved_list", "Paletas salvas (clique para carregar):", "Saved palettes (click to load):"),
    ("shading.palette.load_tip", "Substitui a paleta do projeto por esta.", "Replaces the project palette with this one."),
    ("shading.palette.use_base", "Usar como cor base", "Use as base color"),
    ("shading.palette.use_shade", "Usar como cor de sombra", "Use as shade color"),
    ("shading.palette.use_outline", "Usar como cor do contorno", "Use as outline color"),
    ("shading.palette.use_rim", "Usar como cor do rim", "Use as rim color"),
    ("shading.palette.use_emission", "Usar como emissão", "Use as emission"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicates_no_empty_strings_and_no_clash_with_other_tables() {
        let others: HashSet<&str> = crate::i18n::TABLE
            .iter()
            .map(|(k, _, _)| *k)
            .chain(crate::i18n_lib::LIB_TABLE.iter().map(|(k, _, _)| *k))
            .chain(crate::i18n_scene::SCENE_TABLE.iter().map(|(k, _, _)| *k))
            .chain(crate::i18n_hair::HAIR_TABLE.iter().map(|(k, _, _)| *k))
            .collect();
        let mut seen = HashSet::new();
        for (k, pt, en) in SHADING_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
            assert!(k.starts_with("shading."));
            assert!(!others.contains(k), "{k} já existe em outra tabela");
        }
    }

    /// Toda chave `"shading.*"` escrita no código do workspace precisa existir na tabela.
    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/shading_ui/mod.rs"),
            include_str!("app/shading_ui/panels.rs"),
            include_str!("app/shading_ui/ramp.rs"),
            include_str!("app/shading_ui/script.rs"),
            include_str!("app/shading_ui/sources.rs"),
            include_str!("app/shading_ui/viewport.rs"),
            include_str!("shading/params.rs"),
            include_str!("shading/palette.rs"),
        ];
        let known: HashSet<&str> = SHADING_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"shading.") {
                let tail = &rest[i + 1..];
                let Some(end) = tail.find('"') else { break };
                let key = &tail[..end];
                // prefixos de chaves montadas em tempo de execução terminam em '.'
                if !key.ends_with('.') && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_') && !known.contains(key) {
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
    fn every_param_group_and_preset_has_a_label_in_both_languages() {
        use crate::i18n::{tr_in, Lang};
        let known: HashSet<&str> = SHADING_TABLE.iter().map(|(k, _, _)| *k).collect();
        for p in crate::shading::params::PARAMS {
            assert!(known.contains(p.label), "rótulo ausente: {}", p.label);
            if let crate::shading::params::Kind::Choice(opts) = p.kind {
                for o in opts {
                    assert!(known.contains(o), "opção ausente: {o}");
                }
            }
        }
        for g in crate::shading::params::Group::ALL {
            assert!(known.contains(g.label_key()), "grupo ausente: {}", g.label_key());
        }
        for h in crate::shading::palette::Harmony::ALL {
            assert!(known.contains(h.label_key()), "harmonia ausente: {}", h.label_key());
        }
        for b in crate::shading::presets::BUILT_IN {
            let key = format!("shading.preset.{}", b.id);
            assert!(known.contains(key.as_str()), "preset ausente: {key}");
        }
        for s in crate::app::shading_source_keys() {
            assert!(known.contains(s), "aba ausente: {s}");
        }
        for (k, pt, en) in SHADING_TABLE {
            assert_eq!(tr_in(Lang::PtBr, k), *pt);
            assert_eq!(tr_in(Lang::En, k), *en);
        }
    }
}
