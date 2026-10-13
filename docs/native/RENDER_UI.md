# Workspace Render — UI nativa (`anigo-studio`)

**Estado:** MVP + a maior parte do "completo" de [`pesquisa/render_saida.md`](pesquisa/render_saida.md), validado no sandbox
(Xvfb + Vulkan por software/lavapipe; sem GPU real, sem artista com mouse/tela de verdade). Cobre a parte de Render de T-2.11.

Legenda: **[T]** coberto por teste automático que rodou; **[S]** visto em screenshot/arquivo real no Xvfb; **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais (sem mockups): o projeto é o `ProjectState` canônico editado por `Command` na MESMA `CoreSession` dos
workspaces Personagem e Cenário; a geometria é a do Cenário (`SceneRuntime`); a imagem sai do `anigo-renderer` canônico.

| Área | O que faz | Validação |
|---|---|---|
| **Modelo de dados** (`anigo-core/src/render_output.rs`) | `RenderState.output`: resolução, escala %, supersampling 1–4×, fundo transparente, formato (PNG 8 / PNG 16 / JPEG), qualidade JPEG, DPI, AOVs e DoF. **Aditivo**: omitido do JSON canônico no padrão (fixtures e hashes congelados não mudam). Comando novo `SetRenderOutput` (último antes de `Batch`; contrato TS com 29 tipos) que também edita o espaço de cor do arquivo (`render.color.display_space`) e `near/far` da câmera de render; mudar a resolução derivada o aspecto da câmera (`camera.aspect = w/h`) e o **inverso restaura o aspecto exato**. | [T] domínio, inverso exato, round-trip no JSON, drift TS↔Rust (`npm test`: 348) |
| **Câmera de render** | Escolher uma câmera de cena (nó `Camera`) como câmera de render (`camera_command_from_node`, 1 passo de undo) e **reaplicar** depois de mover o nó; "alinhar à vista do Cenário"; enquadrar a cena/seleção (ajuste justo dos 8 cantos, respeitando o aspecto da SAÍDA); FOV ↔ focal por sensor (Super35, full-frame, APS-C, 16:9, M4/3); perspectiva/ortográfica; near/far; guias (terços, centro, margem de ação 90 %, de título 80 %, escurecer fora do quadro, informações). | [T] matemática (`framing.rs`), cenário↔render real; [S] |
| **Preview** | Viewport "câmera de render": renderer cel canônico com a câmera EXATA do projeto (inclui near/far, ortográfica), DoF e passes ligados/desligados; letterbox no aspecto da saída, máscara fora do enquadramento, margens seguras, xadrez quando o fundo é transparente, HUD (resolução, razão, focal/FOV, SS, alfa). Navegar: arrastar = órbita, Shift/botão do meio = pan, roda = dolly — cada gesto é UM passo de desfazer (`preview` + `commit_preview`). | [S]; [T] gesto = 1 passo, undo/redo |
| **Saída** | Presets (tela, **A4/A5/A3/B5 a 300 dpi**, quadrinho/webtoon, ilustração), W×H com **bloqueio de aspecto**, trocar orientação, escala %, plano de render (SS limitado à textura máxima e a 48 MP; avisa), fundo sólido (cor = `SetBackgroundColor`) ou **transparente (alfa reto)**, espaço de cor **sRGB / Display P3** (`cICP` 12/13/0/1), formato, DPI (`pHYs`), **pasta digitada** + sugestão (`Documents/ANIGO/Renders`, `ANIGO_RENDER_DIR`) com status (existe/será criada/somente leitura), **template de nome** (`{project} {camera} {w} {h} {fmt} {date} {time} {n:04} {frame:04}`, `%04d`) com **contador** que avança em vez de sobrescrever e prévia do nome resultante. | [T] settings/image_ops/png_out; [S] |
| **Passes** | Contorno e DoF ligados/desligados via `graph_disabled`/`DofSettings` do projeto (preview E render final), parâmetros do DoF (foco com "focar na seleção", abertura, focal, bokeh circular/hexagonal, raio máx.), pré-passe de profundidade. MSAA e tonemap aparecem como **somente leitura** com a causa (ver §5). | [T] DoF borra, outline desligado muda a imagem; [S] |
| **AOVs** (aditivos, desligados por padrão; um PNG extra cada) | **Contorno isolado** (RGBA, só a linha; diferença com/sem o passe `outline`, sem DoF), **profundidade** (gray16 normalizado ao conteúdo; faixa em metros no metadado), **máscara** de cobertura da geometria (AA pelo supersampling), **ID por objeto** (cor única determinística; legenda no metadado), **normais** (espaço da câmera). Profundidade/máscara/ID/normais vêm de um **rasterizador de software** (CPU, determinístico, idêntico em qualquer GPU) sobre as mesmas malhas/câmera; ID/profundidade/normais **sem antialiasing** (como no Blender). | [T] conteúdo de cada um; [S] arquivos inspecionados |
| **Render real** | "Renderizar quadro": congela cena + configuração, **thread de GPU dedicada** (device próprio; a UI continua livre), render a N×, redução por caixa em precisão >8 bits, alfa pré-multiplicado→reto, espaço de cor, PNG/JPEG com **metadados** (`anigo:project_fingerprint`, `project_id/name/schema`, câmera, configuração do render, adaptador, `Software`), gravação atômica. Os bytes **não** levam data/hora: mesmo projeto + mesmo adaptador = mesmos bytes. | [T] determinismo, alfa vs render sólido (≤4 níveis), enquadramento vs câmera do projeto, 16 bits, P3, JPEG achatado; [S] |
| **Fila** | Jobs em série com etapa/progresso, **cancelar** (descarta o quadro ao fim da etapa; nada fica no disco), **retomar** (reenfileira pulando quadros que já existem), erros por job e avisos (supersampling reduzido, AOV ignorado, ativo ausente). | [T] cancelar, retomar, falha limpa; [S] xdotool |
| **Histórico** | Por pasta de saída (`.anigo_renders.json` + `.anigo_thumbs/`), cartão com **miniatura**, tamanho, AOVs gravados, copiar caminho e **enviar à Biblioteca como ativo de imagem** (tags `render`, `autoral`; duplicado detectado por conteúdo). | [T] histórico; [S] |
| **Sequência (gancho)** | O job já aceita `FrameRange`, nome por quadro (`{frame}`/`%04d`; sem token acrescenta `_NNNN`) e `FrameSource` (a Animação só precisa devolver a `Scene` de cada quadro). Retomar pula quadros existentes. A UI mostra o intervalo **desabilitado**. | [T] com uma fonte de teste (4 quadros, cancela no 2º, retoma) |
| **ffmpeg** | Só `ffmpeg -version` ("detectar") e a linha de comando sugerida para montar `nome_%04d.png`; o ANIGO **não** executa o encode. | [T] texto; [N] com ffmpeg real |

## 2. Fluxo e atalhos

* Tool shelf: **Câmera e Lente** / **Passes de Render** / **Composição e Exportação** (3 abas do painel direito); o preview é sempre pela câmera de render.
* **Renderizar quadro** (topo do painel) enfileira; a fila e o histórico ficam no painel inferior (recolhível).
* Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z: a mesma sessão do núcleo (edições numéricas, presets, trocar orientação, AOVs, passes, câmera e navegação do preview são passos de undo; campos de texto como pasta e template não).
* Navegação do preview: arrastar = órbita · Shift ou botão do meio = pan · roda = dolly (uma pausa fecha o passo de desfazer).

## 3. Arquitetura

```text
anigo-core    render_output.rs (RenderOutput, patch com inverso exato) + Command::SetRenderOutput       [T]
anigo-studio  render_out/   puro (sem egui): settings (presets, aspecto, plano, nomes), image_ops (caixa,
                            alfa, P3, 8/16 bits), png_out (PNG/JPEG + metadados), aov (rasterizador),
                            framing (enquadrar/navegar), job (pipeline de um quadro + fila em thread),
                            history (histórico + miniaturas)
              app/render_ui/ mod (estado, fila, cena de render), panels (Câmera/Passes/Saída/fila),
                            viewport (preview, guias, navegação), script (cenários de captura)
              i18n_render.rs  textos pt/en (teste: toda chave usada existe; sem duplicata/emoji)
              celview.rs    apply_render_view / clear_render_view (câmera exata, DoF e passes no preview)
```

Pipeline de um quadro (`render_out::job::render_frame`): `Scene` → `HeadlessRenderer::render_scene` a `N×` (com `GraphOverrides` do projeto) →
(contorno: 2 renders extras sem DoF) → `FloatImage::from_rgba8_box` → alfa reto → espaço de cor → PNG/JPEG; AOVs de dados pelo rasterizador.
O AA do supersampling é feito em espaço codificado (como o MSAA do renderer), não em linear.

## 4. Capturas e smoke

Cenários (`anigo-studio --workspace render --scenario <nome> --screenshot x.png`): `render-vazio`, `render-camera`, `render-saida`,
`render-passes`, `render-transparente`, `render-fila` (faz um render REAL numa pasta temporária e espera terminar). Referências em
`docs/native/img/render_*.png` (geradas por `scripts/native_screenshots.sh`; a trilha `native` do CI gera as mesmas e roda o smoke).

`scripts/native_render_smoke.sh` (xdotool, **somente num Xvfb próprio** `:93`; `XDG_DATA_HOME` e pasta de saída temporários):
1. clica em "trocar orientação" → `w/h` trocados e +1 passo de undo (trace `rendertrace`), preview muda (383 706 px);
2. Ctrl+Z → estado **e screenshot idênticos** ao inicial (0 px); Ctrl+Y → idênticos ao trocado (0 px);
3. clica em "Renderizar quadro" → PNG real: 1920×1080 RGBA, cantos com alfa 0, geometria opaca, metadados `anigo:*`;
4. renderiza de novo → **segundo arquivo com os mesmos bytes** (`cmp`) e o contador avançou (nada sobrescrito);
5. enfileira mais dois e cancela pelo botão da fila → job `cancelled`.

Modificadores no xdotool: `keydown Control_L; key z; keyup Control_L` e `windowfocus` (sem gerenciador de janelas o foco é explícito).

## 5. Limites conhecidos e decisões

* **MSAA** é 4× fixo no contrato do renderer; o campo `msaa_samples` do projeto não o muda. A UI mostra "MSAA 4× (fixo no contrato)" e oferece **supersampling** (AA real).
* **Tonemap/HDR/EXR**: o alvo do renderer é `rgba8unorm`; um tonemap depois dele só escureceria realces já saturados. O `tonemap` do projeto aparece só para leitura (T-3.7). **EXR e AOVs em float ficam para quando o renderer tiver alvo HDR.**
* **Bloom, grain e LUT** são passes novos no renderer (T-3.10; contrato congelado): aparecem desabilitados como "planejados".
* **Display P3** só reencoda a paleta (autorada em sRGB) e marca `cICP`; não surgem cores novas. Leitores sem suporte a PNG 3 podem ignorar a marca; JPEG sai convertido mas **sem marcação**.
* **AOVs** usam a pose de repouso (como o preview do Cenário) e não têm DoF; a máscara é só da geometria (sem o contorno); o contorno isolado tem cobertura binária por amostra (suave com SS ≥ 2). Nenhum shader/contrato foi alterado: **nada precisou de fixtures regeneradas** (`npm run fixtures:check` verde).
* **Resolução máxima**: lado ≤ limite de textura da GPU (8192 nos limites padrão do wgpu) e ≤ 48 MP no alvo de render; o supersampling é reduzido automaticamente (aviso) — 4K a 3× vira 2×. Sem render em tiles (exigiria projeção fora de centro no renderer).
* O **DoF do preview** usa o raio em pixels do preview (menor que o do render final): a aparência é aproximada.
* A fila é **em memória**: fechar o app perde jobs em andamento (o histórico em disco fica). O render é cancelável **entre etapas** (uma submissão de GPU em curso termina).
* O histórico mostra a miniatura só da imagem principal; os AOVs aparecem como "+ outline, depth…".
* Pasta/template/contador são preferências da sessão (`session.json`), não do projeto.

## 6. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)

* Tempo e memória de 4K×SS2 / A4 300 dpi×SS2 em GPU real (aqui: lavapipe); o limite de 48 MP é conservador mas não foi medido fora do sandbox.
* Gosto do AA por média em espaço codificado, do contorno isolado em composição real (Photoshop/Clip Studio/After Effects) e das normais/profundidade como mapas de composição.
* Leitura dos PNGs com `cICP` (Display P3) em Windows Photos, Photoshop, Clip Studio, Safari/Chrome/Firefox; PNG 16 bits nesses programas.
* Sensação de órbita/pan/dolly do preview (velocidades, inversão de eixos) com mouse/trackpad de verdade; DPI/escala de tela diferentes de 1×.
* `ffmpeg` real do sistema (só há teste do texto sugerido) e a integração com a timeline da Animação (gancho `FrameSource`).
* Windows/macOS (nomes de arquivo reservados são tratados, mas só Linux foi exercitado); caminhos longos e pastas de rede.

## 7. Como rodar

```bash
cargo test -p anigo-core -p anigo-session -p anigo-studio
cargo run -p anigo-studio -- --workspace render                     # app
cargo run -p anigo-studio -- --workspace render --scenario render-fila --screenshot /tmp/r.png
ANIGO_RENDER_DIR=/tmp/saida ANIGO_SCENARIO_KEEP=1 cargo run -p anigo-studio -- --workspace render --scenario render-fila --screenshot /tmp/r.png  # mantém os PNGs
bash scripts/native_render_smoke.sh ./target/debug/anigo-studio /tmp/render_smoke   # precisa de Xvfb, xdotool, ImageMagick, python3-pil
ANIGO_TEST_OUT=/tmp/out cargo test -p anigo-studio render_out::job_tests             # copia os PNGs dos testes (nomes ocultos .tmp*)
```
