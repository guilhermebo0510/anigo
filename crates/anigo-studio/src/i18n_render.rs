//! Textos do workspace Render (pt-BR / en). Chaves `render.*`; o teste confere que toda chave usada no código
//! existe aqui, que não há duplicata nem texto vazio e que nenhuma colide com as outras tabelas.

/// (chave, pt-BR, en)
pub const RENDER_TABLE: &[(&str, &str, &str)] = &[
    // botão principal e fila
    ("render.btn.render", "Renderizar quadro", "Render frame"),
    ("render.btn.render_tip", "Renderiza a cena pela câmera de render, offscreen, numa thread de GPU própria (a interface continua livre).", "Renders the scene through the render camera, offscreen, on a dedicated GPU thread (the UI stays responsive)."),
    ("render.queue", "Fila de render", "Render queue"),
    ("render.queue.busy", "Render em andamento: veja o progresso na fila abaixo.", "Render in progress: see the queue below."),
    ("render.queue.cancel", "Cancelar (descarta o quadro em andamento ao fim da etapa)", "Cancel (drops the frame in progress at the end of the current stage)"),
    ("render.queue.clear", "Limpar concluídos", "Clear finished"),
    ("render.queue.empty", "Nenhum render nesta sessão. Ajuste a câmera e a saída e clique em Renderizar quadro.", "No render in this session. Set up the camera and output, then click Render frame."),
    ("render.queue.files", "arquivo(s)", "file(s)"),
    ("render.queue.hide", "Esconder a fila", "Hide the queue"),
    ("render.queue.resume", "Retomar (sequências pulam os quadros já gravados)", "Resume (sequences skip frames already written)"),
    ("render.queue.waiting", "na fila", "queued"),
    ("render.queued", "Render enfileirado.", "Render queued."),
    ("render.done", "Render concluído", "Render finished"),
    ("render.failed", "Falha no render", "Render failed"),
    ("render.cancelled", "Render cancelado", "Render cancelled"),
    ("render.note.skipped", "Quadro já existia no disco: pulado.", "Frame already on disk: skipped."),
    ("render.stage.prepare", "preparando", "preparing"),
    ("render.stage.render", "renderizando", "rendering"),
    ("render.stage.outline", "contorno isolado", "isolated outline"),
    ("render.stage.reduce", "reduzindo", "downsampling"),
    ("render.stage.aov", "AOVs", "AOVs"),
    ("render.stage.encode", "codificando", "encoding"),
    ("render.stage.write", "gravando", "writing"),
    ("render.err.dir", "Pasta de saída", "Output folder"),
    ("render.err.empty", "Não há geometria visível para renderizar.", "There is no visible geometry to render."),
    ("render.err.loading", "Ainda carregando a geometria dos ativos: aguarde um instante.", "Asset geometry is still loading: wait a moment."),
    ("render.warn.missing_assets", "ativo(s) ausente(s) na Biblioteca foram ignorados no render", "missing Library asset(s) were left out of the render"),
    ("render.cmd.output", "Saída de render", "Render output"),
    ("render.lib.error", "Biblioteca", "Library"),
    ("render.contract", "fixo no contrato do renderer", "fixed by the renderer contract"),
    // histórico
    ("render.history", "Histórico de saídas", "Output history"),
    ("render.history.empty", "As imagens renderizadas aparecem aqui, com miniatura, para esta pasta de saída.", "Rendered images show up here, with thumbnails, for this output folder."),
    ("render.history.to_library", "Enviar para a Biblioteca como ativo de imagem", "Send to the Library as an image asset"),
    ("render.history.copy_path", "Copiar o caminho do arquivo", "Copy the file path"),
    ("render.history.copied", "Caminho copiado.", "Path copied."),
    // câmera
    ("render.camera.source", "Câmera de render", "Render camera"),
    ("render.camera.project", "Câmera livre do projeto", "Free project camera"),
    ("render.camera.none_hint", "Coloque câmeras de cena no workspace Cenário (Colocar atores > Câmera de cena) para escolhê-las aqui.", "Place scene cameras in the Scene workspace (Place actors > Scene camera) to pick them here."),
    ("render.camera.reapply", "Reaplicar a câmera de cena", "Re-apply the scene camera"),
    ("render.camera.reapply_tip", "Copia de novo posição, direção e FOV do nó de câmera (use depois de movê-lo no Cenário).", "Copies the camera node's position, direction and FOV again (use it after moving the node in the Scene)."),
    ("render.camera.from_view", "Alinhar à vista do Cenário", "Align to the Scene view"),
    ("render.camera.from_view_tip", "Copia posição, alvo e FOV da câmera de edição do Cenário.", "Copies position, target and FOV from the Scene editing camera."),
    ("render.camera.frame_all", "Enquadrar a cena", "Frame the scene"),
    ("render.camera.frame_sel", "Enquadrar a seleção", "Frame the selection"),
    ("render.camera.frame_tip", "Move a câmera mantendo a direção de olhar até a cena caber no enquadramento (respeita o aspecto da saída).", "Moves the camera, keeping its look direction, until the scene fits the frame (respects the output aspect)."),
    ("render.lens", "Lente", "Lens"),
    ("render.lens.sensor", "Sensor", "Sensor"),
    ("render.lens.fov", "FOV vertical", "Vertical FOV"),
    ("render.lens.focal", "Focal", "Focal"),
    ("render.lens.focal_tip", "A focal é calculada com a altura do sensor escolhido; o projeto guarda só o FOV.", "The focal length uses the chosen sensor height; the project stores only the FOV."),
    ("render.lens.projection", "Projeção", "Projection"),
    ("render.lens.perspective", "Perspectiva", "Perspective"),
    ("render.lens.ortho", "Ortográfica", "Orthographic"),
    ("render.lens.near", "Near", "Near"),
    ("render.lens.far", "Far", "Far"),
    ("render.lens.aspect", "Aspecto da câmera = saída", "Camera aspect = output"),
    ("render.guides", "Guias de composição", "Composition guides"),
    ("render.guide.thirds", "Regra dos terços", "Rule of thirds"),
    ("render.guide.center", "Centro", "Center"),
    ("render.guide.action_safe", "Margem segura de ação (90 %)", "Action safe area (90 %)"),
    ("render.guide.title_safe", "Margem segura de título (80 %)", "Title safe area (80 %)"),
    ("render.guide.dim", "Escurecer fora do enquadramento", "Dim outside the frame"),
    ("render.guide.info", "Informações (resolução, lente, SS)", "Info (resolution, lens, SS)"),
    ("render.guide.action", "ação", "action"),
    ("render.guide.title", "título", "title"),
    ("render.info.alpha", "alfa", "alpha"),
    ("render.nav.hint", "Arrastar: orbitar  \u{b7}  Shift ou botão do meio: pan  \u{b7}  Roda: aproximar", "Drag: orbit  \u{b7}  Shift or middle button: pan  \u{b7}  Wheel: dolly"),
    // passes
    ("render.passes.graph", "Passes do render graph", "Render graph passes"),
    ("render.passes.graph_hint", "Ligar/desligar tira o passe do plano de execução (graph_disabled do projeto) no preview e no render final.", "Toggling removes the pass from the execution plan (the project's graph_disabled) in the preview and the final render."),
    ("render.pass.outline", "Contorno (inverted hull)", "Outline (inverted hull)"),
    ("render.pass.outline_tip", "Desligar remove a linha de contorno da imagem principal.", "Turning it off removes the outline from the main image."),
    ("render.pass.cel", "Cel shading", "Cel shading"),
    ("render.pass.cel_tip", "O passe principal não pode ser desligado.", "The main pass cannot be turned off."),
    ("render.pass.dof", "Profundidade de campo (bokeh)", "Depth of field (bokeh)"),
    ("render.pass.dof_tip", "Passe de pós-processamento do renderer; vale no preview e no render final.", "The renderer's post-process pass; applies to the preview and the final render."),
    ("render.pass.prepass", "Pré-passe de profundidade", "Depth prepass"),
    ("render.pass.prepass_tip", "Z-buffer antes do cel (rejeição antecipada). Não muda a imagem.", "Z-buffer before the cel pass (early rejection). Does not change the image."),
    ("render.dof.focus", "Foco", "Focus"),
    ("render.dof.focus_sel", "Focar na seleção (ou no centro da cena)", "Focus on the selection (or the scene center)"),
    ("render.dof.fnumber", "Abertura", "Aperture"),
    ("render.dof.focal", "Focal", "Focal"),
    ("render.dof.bokeh", "Bokeh", "Bokeh"),
    ("render.dof.round", "Circular", "Round"),
    ("render.dof.hex", "Hexagonal", "Hexagonal"),
    ("render.dof.max_radius", "Raio máx.", "Max radius"),
    ("render.passes.aov", "Passes adicionais (AOVs)", "Extra passes (AOVs)"),
    ("render.passes.aov_hint", "Cada AOV ligado grava um PNG extra ao lado da imagem principal. Desligados por padrão.", "Each enabled AOV writes an extra PNG next to the main image. Off by default."),
    ("render.aov.outline", "Contorno isolado (RGBA)", "Isolated outline (RGBA)"),
    ("render.aov.outline_tip", "Só a linha, com cobertura (diferença entre o render com e sem o passe de contorno, sem DoF).", "Only the line, with coverage (difference between the render with and without the outline pass, no DoF)."),
    ("render.aov.depth", "Profundidade (16 bits)", "Depth (16 bit)"),
    ("render.aov.depth_tip", "Cinza 16 bits normalizado ao conteúdo: branco = perto. A faixa em metros vai nos metadados (anigo:depth_range_m). Sem antialiasing.", "16-bit gray normalised to the content: white = near. The range in meters is in the metadata (anigo:depth_range_m). No antialiasing."),
    ("render.aov.mask", "Máscara de cobertura", "Coverage mask"),
    ("render.aov.mask_tip", "Alfa da geometria (sem o contorno), com bordas suaves pelo supersampling.", "Geometry alpha (without the outline), with soft edges from the supersampling."),
    ("render.aov.id", "ID por objeto", "Object ID"),
    ("render.aov.id_tip", "Cor única e determinística por nó, sem antialiasing; a legenda id → cor vai nos metadados (anigo:object_ids).", "Unique, deterministic color per node, no antialiasing; the id → color legend is in the metadata (anigo:object_ids)."),
    ("render.aov.normal", "Normais (espaço da câmera)", "Normals (camera space)"),
    ("render.aov.normal_tip", "RGB = n \u{d7} 0,5 + 0,5; sem antialiasing.", "RGB = n \u{d7} 0.5 + 0.5; no antialiasing."),
    ("render.aov.outline_off", "O passe de contorno está desligado: o AOV de contorno será ignorado.", "The outline pass is off: the outline AOV will be skipped."),
    ("render.aov.note", "Profundidade, máscara, ID e normais vêm de um rasterizador de software (idênticos em qualquer GPU) e usam a pose de repouso, como o preview do Cenário.", "Depth, mask, ID and normals come from a software rasterizer (identical on any GPU) and use the rest pose, like the Scene preview."),
    ("render.passes.post", "Pós-processamento", "Post-processing"),
    ("render.post.bloom", "Bloom estilizado", "Stylized bloom"),
    ("render.post.grain", "Grain", "Grain"),
    ("render.post.lut", "LUT de cor", "Color LUT"),
    ("render.post.note", "Planejados (T-3.10): exigem passes novos no renderer, que tem contrato congelado.", "Planned (T-3.10): they need new renderer passes, and the renderer contract is frozen."),
    // saída
    ("render.res", "Resolução", "Resolution"),
    ("render.res.lock", "Bloquear a proporção", "Lock the aspect ratio"),
    ("render.res.swap", "Trocar orientação (largura \u{2194} altura)", "Swap orientation (width \u{2194} height)"),
    ("render.res.scale", "Escala", "Scale"),
    ("render.res.file", "Arquivo", "File"),
    ("render.res.target", "render", "render"),
    ("render.res.ss_clamped", "Supersampling reduzido pelo limite da GPU:", "Supersampling reduced by the GPU limit:"),
    ("render.preset.custom", "Personalizado", "Custom"),
    ("render.preset_group.screen", "Tela", "Screen"),
    ("render.preset_group.print", "Impressão (300 dpi)", "Print (300 dpi)"),
    ("render.preset_group.comic", "Quadrinho / webtoon", "Comic / webtoon"),
    ("render.preset_group.illustration", "Ilustração", "Illustration"),
    ("render.preset.test", "Teste rápido 360p", "Quick test 360p"),
    ("render.preset.hd720", "HD 720p", "HD 720p"),
    ("render.preset.fhd", "Full HD 1080p", "Full HD 1080p"),
    ("render.preset.qhd", "QHD 1440p", "QHD 1440p"),
    ("render.preset.uhd4k", "4K UHD", "4K UHD"),
    ("render.preset.square", "Quadrado 1:1", "Square 1:1"),
    ("render.preset.portrait45", "Retrato 4:5", "Portrait 4:5"),
    ("render.preset.vertical916", "Vertical 9:16", "Vertical 9:16"),
    ("render.preset.a4_portrait", "A4 retrato", "A4 portrait"),
    ("render.preset.a4_landscape", "A4 paisagem", "A4 landscape"),
    ("render.preset.a5_portrait", "A5 retrato", "A5 portrait"),
    ("render.preset.b5_portrait", "B5 (ISO) retrato", "B5 (ISO) portrait"),
    ("render.preset.a3_portrait", "A3 retrato", "A3 portrait"),
    ("render.preset.panel43", "Quadro 4:3", "Panel 4:3"),
    ("render.preset.panel_wide", "Quadro panorâmico", "Wide panel"),
    ("render.preset.panel_square", "Quadro quadrado", "Square panel"),
    ("render.preset.webtoon", "Webtoon (largura 800)", "Webtoon (width 800)"),
    ("render.preset.illus_square", "Ilustração quadrada", "Square illustration"),
    ("render.preset.illus_poster", "Ilustração pôster 2:3", "Poster illustration 2:3"),
    ("render.preset.illus_wide", "Ilustração 16:9", "Illustration 16:9"),
    ("render.aa", "Antialiasing", "Antialiasing"),
    ("render.aa.ss", "Supersampling", "Supersampling"),
    ("render.aa.off", "Desligado", "Off"),
    ("render.aa.ss_tip", "Renderiza a N\u{d7} e reduz por média (precisão acima de 8 bits).", "Renders at N\u{d7} and box-filters down (more than 8 bits of precision)."),
    ("render.bg", "Fundo", "Background"),
    ("render.bg.transparent", "Fundo transparente (alfa)", "Transparent background (alpha)"),
    ("render.bg.transparent_tip", "Grava alfa reto (não pré-multiplicado). JPEG não tem alfa: achata sobre a cor de fundo.", "Writes straight (not premultiplied) alpha. JPEG has no alpha: it flattens over the background color."),
    ("render.bg.color", "Cor de fundo", "Background color"),
    ("render.bg.color_unused", "(sem efeito com alfa)", "(unused with alpha)"),
    ("render.color", "Gerenciamento de cor", "Color management"),
    ("render.color.space", "Espaço de saída", "Output space"),
    ("render.color.srgb_tip", "PNG marcado como sRGB (chunk sRGB).", "PNG tagged as sRGB (sRGB chunk)."),
    ("render.color.p3_tip", "Reencoda de sRGB para Display P3 e marca o PNG com cICP (12/13/0/1). A paleta cel é autorada em sRGB: não surgem cores novas. Leitores sem PNG 3 podem ignorar a marca.", "Re-encodes sRGB to Display P3 and tags the PNG with cICP (12/13/0/1). The cel palette is authored in sRGB: no new colors appear. Readers without PNG 3 may ignore the tag."),
    ("render.color.tonemap", "Tonemap", "Tonemap"),
    ("render.color.tonemap_none", "nenhum (renderer de 8 bits; HDR em T-3.7)", "none (8-bit renderer; HDR is T-3.7)"),
    ("render.color.tonemap_tip", "Um tonemap sobre 8 bits já saturados só escureceria realces e estragaria cores chapadas do cel; por isso não é aplicado.", "A tonemap over already clamped 8-bit values would only darken highlights and ruin flat cel colors; that is why it is not applied."),
    ("render.format", "Formato", "Format"),
    ("render.format.png16_tip", "PNG 16 bits: a média do supersampling vira precisão real (sem faixas em gradientes).", "16-bit PNG: the supersampling average becomes real precision (no banding in gradients)."),
    ("render.format.quality", "Qualidade", "Quality"),
    ("render.format.jpeg_alpha", "JPEG não tem alfa: o fundo transparente será achatado sobre a cor de fundo.", "JPEG has no alpha: the transparent background will be flattened over the background color."),
    ("render.format.dpi", "Densidade (dpi)", "Density (dpi)"),
    ("render.format.dpi_tip", "Metadado do arquivo (pHYs): não muda os pixels.", "File metadata (pHYs): does not change the pixels."),
    ("render.files", "Arquivos de saída", "Output files"),
    ("render.files.dir", "Pasta de saída (digite o caminho)", "Output folder (type the path)"),
    ("render.files.suggest", "Sugerida", "Suggested"),
    ("render.files.dir_ok", "A pasta existe e é gravável.", "The folder exists and is writable."),
    ("render.files.dir_create", "A pasta não existe: será criada ao renderizar.", "The folder does not exist: it will be created when rendering."),
    ("render.files.dir_file", "O caminho é um arquivo, não uma pasta.", "The path is a file, not a folder."),
    ("render.files.dir_ro", "A pasta é somente leitura.", "The folder is read-only."),
    ("render.files.template", "Nome do arquivo (template)", "File name (template)"),
    ("render.files.template_tip", "Tokens: {project} {camera} {w} {h} {fmt} {date} {time} {n:04} (contador) {frame:04} (quadro). %04d também vale.", "Tokens: {project} {camera} {w} {h} {fmt} {date} {time} {n:04} (counter) {frame:04} (frame). %04d works too."),
    ("render.files.counter", "Próximo número", "Next number"),
    ("render.files.overwrite", "Sobrescrever", "Overwrite"),
    ("render.files.overwrite_tip", "Desligado: o contador avança até um nome livre; nada é sobrescrito.", "Off: the counter advances to a free name; nothing is overwritten."),
    ("render.sequence", "Sequência de quadros e vídeo", "Frame sequence and video"),
    ("render.sequence.hint", "Gancho para a Animação: a fila já aceita intervalo de quadros, nome por {frame}/%04d e retomar pulando quadros existentes. Falta só a timeline fornecer a cena de cada quadro.", "Hook for Animation: the queue already supports a frame range, naming by {frame}/%04d and resuming by skipping existing frames. Only the timeline is missing to provide each frame's scene."),
    ("render.sequence.range", "Quadros", "Frames"),
    ("render.sequence.step", "passo", "step"),
    ("render.sequence.tokens", "Template de sequência: cena_%04d (ou {frame:04}). Sem token de quadro, _NNNN é acrescentado.", "Sequence template: scene_%04d (or {frame:04}). Without a frame token, _NNNN is appended."),
    ("render.video", "Vídeo (ffmpeg)", "Video (ffmpeg)"),
    ("render.video.detect", "Detectar ffmpeg", "Detect ffmpeg"),
    ("render.video.missing", "ffmpeg não encontrado no PATH.", "ffmpeg not found in PATH."),
    ("render.video.note", "Só detecta e sugere a linha de comando para montar a sequência PNG; o ANIGO não executa o encode.", "It only detects ffmpeg and suggests the command to assemble the PNG sequence; ANIGO does not run the encode."),
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
        for (k, pt, en) in RENDER_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
            assert!(k.starts_with("render."));
            assert!(!others.contains(k), "{k} já existe em outra tabela");
            // sem emoji na UI: só símbolos tipográficos comuns
            for c in pt.chars().chain(en.chars()) {
                assert!(!matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF if c != '\u{2194}'), "emoji em {k}: {c}");
            }
        }
    }

    /// Toda chave `"render.*"` escrita no código do workspace (e do preview) precisa existir na tabela.
    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/render_ui/mod.rs"),
            include_str!("app/render_ui/panels.rs"),
            include_str!("app/render_ui/viewport.rs"),
            include_str!("app/render_ui/script.rs"),
            include_str!("render_out/settings.rs"),
            include_str!("app/scene_ui/mod.rs"),
        ];
        let known: HashSet<&str> = RENDER_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"render.") {
                let tail = &rest[i + 1..];
                let end = tail.find('"').unwrap_or(0);
                let key = &tail[..end];
                if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') && !known.contains(key) {
                    missing.push(key.to_string());
                }
                rest = &rest[i + 1 + end..];
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "chaves sem tradução: {missing:?}");
    }

    #[test]
    fn labels_resolve_in_both_languages() {
        for (k, pt, en) in RENDER_TABLE {
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::PtBr, k), *pt);
            assert_eq!(crate::i18n::tr_in(crate::i18n::Lang::En, k), *en);
        }
    }

    /// Todo preset da tabela de resoluções tem nome traduzido.
    #[test]
    fn every_preset_and_group_has_a_label() {
        let known: HashSet<&str> = RENDER_TABLE.iter().map(|(k, _, _)| *k).collect();
        for p in crate::render_out::settings::PRESETS {
            assert!(known.contains(p.key), "preset sem nome: {}", p.key);
        }
        for g in crate::render_out::settings::PresetGroup::ALL {
            assert!(known.contains(g.key()), "grupo sem nome: {}", g.key());
        }
    }
}
