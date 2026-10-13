# Workspace Shading — UI nativa (`anigo-studio`)

**Estado:** MVP + a maior parte do "completo" da proposta de [`pesquisa/shading_materiais_anime.md`](pesquisa/shading_materiais_anime.md),
implementado e validado no sandbox (Xvfb + Vulkan por software/lavapipe, sem GPU real, sem mouse/teclado de um artista).
Cobre a parte de Shading de T-2.11. É onde o artista **liga e afina** os recursos de Fase 1 do motor cel
([`CEL_ENGINE_FASE1.md`](CEL_ENGINE_FASE1.md)), que nascem desligados.

Legenda: **[T]** coberto por teste automático que rodou; **[S]** visto em screenshot ou em entrada real (xdotool) no Xvfb; **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais (sem mockups): o material é o `StylizedMaterial` canônico do `anigo-core`, editado por `Command` através do
`CoreSession` (Personagem e Cenário) ou pelo histórico da `ModelingSession` (Modelagem), e desenhado pelo **mesmo** renderer cel do render final.

| Área | O que faz | Validação |
|---|---|---|
| **Alvos (3 fontes)** | Aba **Personagem**: materiais do projeto (`ProjectState::materials`), edição por `SetMaterialParams`. Aba **Cenário**: cada nó com malha (primitivas, ativos da Biblioteca); a raiz do personagem aponta para o material do projeto; instâncias por `SetNodeProps { overrides.material }` (patch completo e aditivo sobre o material base do nó). Aba **Modelagem**: objetos da sessão, material completo em `Document::stylized`, desfazer pelo histórico da sessão. Entrar no Shading vindo de outro workspace já abre no que estava selecionado/ativo. | [T] `shading::edit` (9 testes: arrasto = 1 passo, voltar ao valor inicial não deixa resíduo, patch no JSON só do que mudou, salvar/abrir, Modelagem sem tocar o núcleo, troca de alvo no meio do gesto, NaN recusado) [S] `shading-cenario`, `shading-modelagem` |
| **Parâmetros agrupados** | Base/Sombra (cor base, cor de sombra, limiar, suavidade, degraus, matiz da sombra, AO, 2ª sombra), **Rampa 2D**, Especular, Rim, Matcap, **Contorno por material** (largura, cor, opacidade, suavidade, bias, normais suavizadas, afinar com a distância + distância de referência + piso, taper, tinta + escurecer/saturar), **Face SDF** (liga, offset, suavidade, espelho, mapa), Emissão, Olho anime. Grupos recolhíveis; título com contagem de modificados; reset por parâmetro e por grupo; "Resetar tudo"; botão **Ligar recursos anime** (contorno suave + falloff + taper + tinta). Controles dependentes (`gate`) ficam esmaecidos com ícone quando o recurso de que dependem está desligado. | [T] `shading::params` (ids únicos, gates válidos, escrever/ler/resetar cada parâmetro, padrões dentro da faixa, `clamp` de NaN, reset de grupo só mexe no grupo) |
| **Cada controle muda a imagem** | Um teste renderiza a shader ball no renderer cel real para CADA parâmetro (valor padrão × alterado, com os gates acesos) e exige diferença de pixels; os 5 que não alteram a esfera têm a razão registrada no teste (`known_invisible`) e dica na UI. Folhas de contato em `img/shading/`. | [T] `every_panel_control_changes_the_ball_image_or_is_a_documented_limitation` [S] folhas `shading_params_*.png` |
| **Rampa 2D com editor de gradiente por linha** | Gráfico (horizontal = half-lambert deslocado pelo limiar, vertical = fator de luz) com paradas arrastáveis, duplo clique cria parada, até 6 por linha, interpolação **linear** ou **degraus**, edição numérica da parada, faixa em cinza (a linha que o shader amostra), referência da linha do contrato em cinza, volta ao contrato por linha. A linha em edição é a `ramp_row` arredondada (botões 0–3). O gradiente vai em `NprParams::ramp_custom` (aditivo, omitido no padrão); o renderer monta a textura 256×4 do material com as linhas personalizadas (as demais continuam as do contrato); **WGSL e contrato intactos**. | [T] `anigo-core::npr::ramp_custom_tests` (4), `cel_features::custom_ramp_row_changes_the_image_and_untouched_rows_stay_on_the_contract` [S] `shading-rampa`, `img/shading/shading_ramp_custom.png` |
| **Preview em tempo real** | Shader ball (esfera UV de 1 m, `u = 0,25` olha para +Z, a convenção do SDF facial), objeto/personagem do alvo (malha deformada do núcleo, geometria do nó do Cenário, objeto avaliado da Modelagem) ou ambos lado a lado; câmera orbital (LMB orbita, Shift pan, Ctrl zoom, roda), turntable, enquadrar (F), 3 fundos. **Luz**: azimute/elevação, presa à câmera (azimute relativo à câmera) ou ao mundo (0° = +Z, positivo para +X, a convenção do SDF facial); cor/intensidade/ambiente vêm da luz do projeto (o que o render final usa). Redesenha só quando algo muda (assinatura dos materiais/câmera/luz). | [S] todas as capturas |
| **A/B** | Compara "antes" (A) × "atual" (B) lado a lado, na bola e no objeto. **A = atual** fixa o antes; **Trocar** aplica A e guarda o atual como novo A; **Voltar a A** aplica A (um passo de undo cada). | [S] `shading-ab` |
| **Paleta** | Receita HSV (matiz/saturação/brilho) que **deriva a cor de sombra da base** (botão ou "Ligar": a sombra acompanha a base enquanto edita); **girar matiz** de base+sombra por arrasto; harmonias (complementar, análoga, tríade, tétrade, complementar dividida) como swatches clicáveis (clique = cor base; botão direito = sombra/contorno/rim/emissão/adicionar); **paleta do projeto** (adicionar base/sombra/harmonia, remover) salva sozinha em `<dados>/anigo-studio/palettes/projeto.anigopalette.json` e **paletas nomeadas** (salvar/carregar `.anigopalette.json`). | [T] `shading::palette` (7) [S] `shading-paleta` |
| **Presets** | **Embutidos** (7: pele, cabelo com brilho, tecido macio com rampa, metal com matcap, cel duro, ilustração macia, anime completo) e **Biblioteca**: "Salvar" grava o material atual como ativo `Material` (`.anigomat.json`, tag `preset`) com **miniatura em shader ball** renderizada pelo renderer cel; clicar aplica (um passo de undo). O `ToonMaterial` ganhou o bloco aditivo `stylized` (extras do cel + NPR + gradientes), então o preset não perde nada; vale também para materiais importados. | [T] `shading::presets` (embutidos válidos/distintos, ida e volta pelo formato, biblioteca real: salvar/listar/carregar/deduplicar), `anigo-assets::material::stylized_roundtrip_tests` (3) [S] `shading-presets` |
| **Copiar / colar** | Ctrl+C / Ctrl+V entre alvos de qualquer fonte (o nome fica o do alvo). | [T] via `shading::edit::apply` |
| **Texturas** | Seletor da Biblioteca (ativos `Texture`) por slot: base, sombra, 2ª sombra, rampa, ILM, matcap, emissão, máscara de contorno e **mapa SDF facial**. Só o **mapa SDF** é amostrado pelo renderer (R8 → `set_face_sdf_map`; há também o **mapa procedural** de demonstração); os demais slots ficam como **referência no preset** (`extras.anigo_texture_refs`) e a UI diz isso. | [T] referências no preset [S] `shading-face-sdf*` |
| **Face SDF em vários ângulos** | Capturas com a luz a 80°, 0° e −80° (com espelho) e folha 7 ângulos × (sem espelho | com espelho). | [T] `face_sdf_shadow_moves_with_the_light_and_mirrors_for_the_other_side` [S] `shading-face-sdf*` |
| **Undo/redo** | Arrasto = preview ao vivo (`CoreSession::preview` / escrita direta na Modelagem) e **um** passo ao soltar; edição discreta = um passo; Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z roteados pela aba (Modelagem → `ModelingSession`; Personagem/Cenário → `CoreSession`); trocar de alvo ou de workspace no meio do gesto o grava. | [T] `shading::edit` [S] smoke xdotool (abaixo) |

## 2. Atalhos

| Ação | Atalho |
|---|---|
| Desfazer / refazer | Ctrl+Z / Ctrl+Y ou Ctrl+Shift+Z (também os botões do cabeçalho do painel) |
| Copiar / colar material | Ctrl+C / Ctrl+V |
| Reenquadrar o preview | F (ou o botão de enquadrar) |
| Orbitar / pan / zoom | LMB arrastando / Shift+LMB / Ctrl+LMB ou roda |
| Ferramentas do shelf | `cel_shader` abre Base/Sombra, `rim` o Rim, `outline` o Contorno, `palette` a Paleta, `shader_ball` volta ao modo Bola |
| Rampa | arrastar parada; duplo clique no gráfico cria parada |

## 3. Arquitetura

```text
anigo-core    npr.rs  RampCustom (gradiente por linha, puro/Copy, bytes da textura) + NprParams::ramp_custom
              command.rs  MaterialPatch::{from_material, diff, validate}  (patch completo, mínimo e finito)
              level.rs    NodeOverrides.material (patch completo da instância) + apply_to_material
anigo-assets  material/mod.rs  ToonMaterial.stylized (StylizedExtras: extras do cel + NPR), to_stylized/from_stylized sem perda
anigo-renderer headless.rs  rampa 2D por material (cache por RampCustom::cache_key; mesmo formato/sampler do contrato)
anigo-studio  shading/      módulos puros e testados: params (tabela única), palette, ball (shader ball, layout, luz),
                            presets (embutidos + Biblioteca), edit (leitura/preview/commit/undo sem UI), visual_tests
              app/shading_ui/ UI: mod (estado, atalhos, entrada), sources (alvos, seleção, mensagens), panels (esquerda/direita,
                            paleta, texturas, presets), ramp (editor de gradiente), viewport (preview, A/B, luz), script (capturas)
              celview.rs    CelPreview: mapa SDF facial; sync_scene usa Document::stylized
              library_ui/cel_thumb.rs  miniatura de ativo Material = shader ball no renderer cel (PIPELINE_VER 3)
              i18n_shading.rs pt/en (teste: toda chave usada existe, todo parâmetro/grupo/preset tem rótulo nos dois idiomas)
```

Hooks nos arquivos compartilhados (mínimos e aditivos): `app.rs` (campo, painéis/viewport/atalhos, roteamento de undo, status, entrada), `app/automation.rs`
(despacho do cenário), `i18n.rs` (encadeia a tabela), `workspace.rs` (nenhum), `library_ui/cel_thumb.rs` (miniatura de material), `scene_ui` (`node_material` delega
a `shading::edit` e o Details mostra uma nota quando o nó tem material completo do Shading), `scripts/native_screenshots.sh` e o job `native` do CI.

## 4. Capturas e smoke

Cenários (`anigo-studio --workspace shading --scenario <nome> --screenshot x.png`): `shading-ball`, `shading-rampa`, `shading-rim`, `shading-contorno`,
`shading-face-sdf` (luz 80°), `shading-face-sdf-frente` (0°), `shading-face-sdf-espelho` (−80° com espelho), `shading-ab`, `shading-paleta`, `shading-presets`,
`shading-cenario`, `shading-modelagem`. Referências em `docs/native/img/shading_*.png` (geradas por `scripts/native_screenshots.sh`; o job `native` do CI
gera as mesmas como artefato). Folhas de contato dos testes visuais em `docs/native/img/shading/` (com `ANIGO_TEST_OUT=<pasta> cargo test -p anigo-studio shading::visual`).

`scripts/native_shading_smoke.sh` (xdotool, **somente dentro de um Xvfb próprio** `:94`; nunca toca o display do usuário; estado do egui em `XDG_DATA_HOME` temporário):
1. espera o cenário `shading-smoke` terminar (`ANIGO_TRACE_SHADING=1` imprime `shadetrace ...` por mudança e `shaderect <param> x= y= w= h=` com a posição dos sliders);
2. arrasta o slider **Intensidade do Rim** (mouse real: down, 14 passos, up) → o rim sobe (0,5 → 2,15), a viewport muda (≈13 000 px), `gesture=true` durante o arrasto e **exatamente +1 passo de undo** ao soltar;
3. Ctrl+Z → rim e **screenshot da viewport idênticos** ao anterior (0 px de diferença); Ctrl+Y → idênticos ao pós-arrasto (0 px);
4. um segundo arrasto no mesmo slider = outro passo (+2); dois Ctrl+Z voltam ao estado inicial (0 px).

Teclas com modificador: sequência explícita `keydown Control_L; key z; keyup Control_L` e `windowfocus` (sem gerenciador de janelas o foco é explícito).

## 5. Limites conhecidos e decisões

- **Texturas por material não são amostradas pelo renderer** (slots MToon ancoram o neutro 1×1): base/sombra/2ª sombra/ILM/matcap/emissão/rampa-textura ficam só como
  referência no preset. O **matcap** sem textura usa o matcap branco neutro (o efeito é multiplicar/somar brilho uniforme). A emissão só soma com o slot ligado: a UI liga
  o slot quando a intensidade passa de 0 (o neutro faz o papel de "sem textura").
- **Transparência e recorte (`alpha_mode`, `alpha_cutoff`) não existem** no material do renderer: não há grupo para isso (a UI avisa na seção Texturas).
- **Não alteram a shader ball** (documentado no teste e na dica de cada um): 2ª sombra (precisa da textura da 2ª sombra), "quantizar textura de sombra", intensidade do AO
  (depende do canal R de vértice, que a esfera não tem), bias de profundidade do contorno (só age contra outra malha) e os controles de íris do olho (precisam de textura).
- **O mapa SDF facial é global ao renderer do preview** (uma textura para todos os materiais com `face_sdf_enabled`), não por material; ele persiste ao trocar de alvo e
  também afeta o preview do Personagem enquanto o mapa estiver instalado. A referência do mapa escolhido não vai para o projeto (só para o preset, se salvo).
- **A cor de sombra é multiplicada pela luz ambiente no shader** (herdado do motor): tons escuros de sombra viram quase preto. Os presets embutidos usam sombras claras.
- A **paleta do projeto** é um arquivo (`projeto.anigopalette.json`), não um campo do `ProjectState` (não existe `AddMaterial`/paleta no `Command`): ela não viaja dentro do `.anigo`.
- **Material por instância no Cenário** vive em `NodeSlot.overrides.material`; não existem `AddMaterial`/`SetNodeMaterial` (um projeto tem 1 material canônico + overrides). Isso
  também significa que não há "material compartilhado entre nós" (cada instância tem seu patch).
- A tela de **Modelagem** guarda o material em `Document::stylized` (por id de objeto); apagar o objeto deixa a entrada órfã (inofensiva) e o histórico é por snapshot do documento.
- **Undo é por origem**: com a aba Modelagem ativa, Ctrl+Z fala com a sessão da Modelagem; nas outras, com o núcleo. Não existe um histórico único entre as duas sessões.
- A rampa 2D personalizada tem **até 6 paradas por linha** e 4 linhas (limite do formato 256×4 do contrato); interpolação linear ou em degraus (sem curvas de Bézier).

## 6. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)

- **Gosto**: defaults, os 7 presets, os valores do botão "Ligar recursos anime", a receita HSV padrão da sombra (matiz −15°, saturação ×1,15, brilho ×0,78), harmonias e as cores do preview.
  O look do motor é claro/lavado (ACES + ambiente): cores de meio-tom foram usadas nas capturas para os efeitos ficarem legíveis; as de pele clara estouram em branco.
- **Sensação** do arrasto dos sliders, do editor de gradiente (alça de 16 px, duplo clique para criar parada), dos seletores de cor (cada arrasto no popup = 1 passo) e da orbita do preview — só foi
  exercitado por xdotool com passos discretos (um único slider).
- **Desempenho com GPU real**: a bola + objeto + A/B compartilham uma cena por quadro (o renderer reenvia os vértices de cada nó; sem instancing); lavapipe é mais lento e esconde problemas de banda.
  O renderer cria uma textura 256×4 por gradiente distinto (cache de 64).
- **Entrada real** de Ctrl+C/Ctrl+V, do seletor de cor e do duplo clique na rampa; Ctrl vs Cmd em macOS.
- **Mapas SDF reais** de artista (Genshin/VRoid): o fluxo da Biblioteca foi exercitado só em teste/capturas com o mapa procedural; mapas de rosto reais podem precisar do espelho/offset.
- **Material da Biblioteca com texturas reais** (o VRoid/MToon com texturas): o preset guarda só os parâmetros que o `StylizedMaterial` modela + extras do cel.
- Windows/macOS (o caminho da paleta usa `XDG_DATA_HOME`/`HOME`; no Windows cai em `temp_dir`).
- Cor: a comparação sRGB/linear contra referências de anime e a leitura dos swatches (o swatch mostra o sRGB gravado; o renderer clareia).

## 7. Como rodar

```bash
cargo test -p anigo-core -p anigo-session -p anigo-assets -p anigo-renderer -p anigo-studio
ANIGO_TEST_OUT=/tmp/shading cargo test -p anigo-studio shading::visual      # folhas de contato PNG
cargo run -p anigo-studio -- --workspace shading                              # app
cargo run -p anigo-studio -- --workspace shading --scenario shading-rampa --screenshot /tmp/s.png
bash scripts/native_shading_smoke.sh ./target/debug/anigo-studio /tmp/shading_smoke   # precisa de Xvfb, xdotool, ImageMagick
```
