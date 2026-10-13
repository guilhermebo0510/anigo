# Pesquisa — saída de render para anime 3D (workspace Render nativo)

**Escopo:** o que um estúdio de anime 3D precisa receber do botão "Renderizar": resolução/aspecto, antialiasing, fundo
transparente, gerenciamento de cor, formatos, camadas (AOVs) para composição, metadados, fila e vídeo. Referências: Blender
(Output/Format/View Layer), Unreal (Movie Render Queue e High Res Screenshot), Marmoset Toolbag (Render window), Clip Studio
Paint (3D → linhas e tons) e a especificação PNG 3.

Legenda de fontes: **[V]** documentação oficial lida nesta sessão; **[S]** fonte secundária (tutorial/blog/resumo de busca);
**[C]** conjectura/decisão de projeto (nada aqui é citado como fato de terceiros).
Nota de método: as páginas de documentação do Unreal renderizam por JavaScript e o `scrape` devolveu só o índice; os trechos do
MRQ/HRSS vêm do resumo de busca da própria página `dev.epicgames.com` ([V] no conteúdo, mas lido via excerto, não página inteira).

## 1. O que cada ferramenta faz

### 1.1 Blender — Output, Format, View Layer [V]
* **Format**: `Resolution X/Y`, `Percentage` (escala sobre a resolução para testes com as mesmas proporções), `Aspect X/Y`
  (pixel não quadrado), `Render Region`/`Crop to Render Region`, presets no cabeçalho do painel (TV/tela), "Swap Dimensions".
  → *Adotar*: resolução + **porcentagem de escala** + **troca de orientação** + **bloqueio de aspecto** + presets. *Não adotar agora*:
  pixel não quadrado e região de render (documentado como avançado).
* **Output**: caminho de arquivo com **`#` = preenchimento do número do quadro** (`image_##_test.png` → `image_01_test.png`; padrão
  4 dígitos), extensões automáticas, `Overwrite`, `Placeholders` ("render farm primitiva": várias máquinas sobre uma pasta compartilhada),
  *Color* (BW/RGB/RGBA), profundidade de cor, compressão, **densidade de pixels (PPI/DPI) como metadado** que não altera a qualidade.
  → *Adotar*: template de nome com contador/quadro, sobrescrever on/off, RGBA, 8/16 bits, DPI em `pHYs`.
* **Dica oficial**: "não renderize direto para vídeo; renderize sequência de imagens (PNG/EXR) e monte depois" — permite **retomar** do quadro
  que falhou e trocar o encode em segundos. → fundamenta *sequência PNG + ffmpeg só como etapa posterior opcional*.
* **Color Management** por saída: "Follow Scene" ou "Override" (display/view/look). → *Adotar*: espaço de saída do projeto (`ColorManagement.display_space`).
* **Passes (View Layer)** [V]: Combined, Depth, Mist, Position, **Normal**, Vector, UV, **Object Index / Material Index** (IDs), Cryptomatte,
  AOVs de shader. Nota oficial: **"Depth, Position, Object Index e Material Index não são anti-aliased"** (um ID médio entre dois objetos é
  um ID inexistente). Em EEVEE, DoF e motion blur só entram no Combined — passes de dados saem sem DoF.
  → *Adotar*: AOVs de dados **sem AA** (amostra única no centro do pixel) e sem DoF; máscara e linha com cobertura (AA) por serem "alpha".

### 1.2 Unreal — Movie Render Queue e High Res Screenshot [V, via excerto]
* **MRQ / Output**: *File Name Format* com **tokens** (`{sequence_name}`, `{frame_number}`…; "/" cria subpastas), *Output Resolution*,
  *Zero Pad Frame Numbers*, **Frame Number Offset** (evita quadros negativos), *Handle Frame Count*.
* **MRQ / Anti-Aliasing**: **Spatial** (renderiza o mesmo quadro N vezes com *jitter* de câmera e acumula) e **Temporal** (fatias de tempo do
  obturador); "qualquer combinação produz o mesmo padrão de offsets"; método de AA TSR/TAA/FXAA/None — com ≥ 8 amostras a recomendação é **desligar
  o AA do motor** e deixar as amostras acumuladas fazerem o trabalho. → *Adotar o equivalente espacial*: **supersampling** (renderizar a
  N× e reduzir) em vez de depender do MSAA fixo do renderer. O temporal fica para a Animação.
* **HighResShot**: `HighResShot 3840x2160` ou `Multiplier`; região de captura; `bCaptureHDR` (EXR); **buffers do G-Buffer** (BaseColor, Opacity, SceneDepth,
  WorldNormal, …) exportáveis como imagens; **máscara por *Custom Depth*** (recorta objetos escolhidos = fundo transparente seletivo). Aviso oficial:
  multiplicadores muito grandes podem travar o driver. → *Adotar*: limite de pixels efetivos, **mapa de profundidade e de normais** como AOVs,
  **máscara por objeto** (equivalente do Custom Depth).

### 1.3 Marmoset Toolbag — Render window [V]
* **Output › Image**: caminho (pasta ou arquivo), *Resolution* + *presets*, *Format*, **Overwrite Existing (se desligado, nomes numerados)**, *Samples*,
  **Transparency** ("salva a máscara de transparência no canal alfa onde o formato suporta"; **não** suportado em vídeo, só em sequência).
* **Render Cameras**: renderização em lote de várias câmeras habilitadas. **Render Passes**: Alpha Mask, Depth (com normalização e *dither*),
  Normals, Position, **Material ID / Object ID (cores únicas)**, Albedo, AO… → confirma a lista de AOVs "úteis" e a prática de **normalizar
  profundidade** e **cores únicas determinísticas para IDs**.
* → *Adotar*: sobrescrever off ⇒ nome com contador; lote de câmeras fica como gancho (lista de câmeras de cena + fila).

### 1.4 Clip Studio Paint — 3D para quadrinhos/anime 2D [S]
O fluxo de ilustradores: *Line Extraction → Convert layers to lines and tones* gera **uma pasta com camadas separadas de linha, tons, sombras e
fundo branco**, para colorir/retocar em 2D (tutorial no CLIP STUDIO TIPS, lido). Moral: o artista de anime/mangá quer **linha isolada**, **sombra
isolada** e **máscara por objeto** como camadas — não só a imagem final. → AOV "contorno" (RGBA só com a linha) e "máscara" são o núcleo do MVP de AOVs.
Presets de página (A4 300 dpi etc.) vêm da prática de impressão: **A4 = 210×297 mm → 2480×3508 px a 300 dpi** (conta: mm/25,4×300).

### 1.5 PNG 3ª edição (W3C) [V]
* `tEXt`: palavra-chave **1–79 bytes, Latin-1 imprimível**, sem espaços iniciais/finais/consecutivos → metadados do ANIGO como `anigo:*` precisam de
  chaves ASCII; usar `iTXt` (UTF-8) para o valor JSON.
* `cICP` (4 bytes: primárias, transferência, matriz, faixa) sinaliza o espaço de cor; **Display P3 = primárias 12 + transferência 13 (a mesma curva do sRGB)**
  [S: Chris Lilley, "cICP in PNG, explained"]; `iCCP` (perfil ICC), `sRGB` (intent), `pHYs` (unidade 1 = metro, pixels/metro). `cHRM/cICP/gAMA/iCCP/sRGB/pHYs`
  vão **antes** de `PLTE`/`IDAT`.
* Consequência: dá para marcar **sRGB** (`sRGB`) ou **Display P3** (`cICP` 12/13/0/1) no arquivo sem perfil ICC embutido; leitores só-PNG-2 ignoram `cICP`
  [C: nota de risco; testar em Windows Photos/Photoshop é validação humana].

## 2. O que o renderer atual consegue (lido em `crates/anigo-renderer`)

| Capacidade | Estado | Como usar sem quebrar o contrato congelado |
|---|---|---|
| Alvo offscreen | `Rgba8Unorm`, 8 bits, **MSAA 4× fixo no contrato** (`targets.msaa_samples`) | MSAA do `RenderState` **não** muda o renderer → AA extra = **supersampling externo** (render a N×, redução por caixa, em precisão maior que 8 bits) |
| Fundo | `scene.background_color` é a cor de limpeza; alfa 0 vira alfa no resolve | fundo transparente = limpar com `[0,0,0,0]`; o resolve do MSAA devolve **alfa pré-multiplicado** (cor×cobertura), converter para alfa reto ao gravar [medir no teste] |
| Passes | `outline`, `cel`, `dof_post` + `depth_prepass` via `GraphOverrides {order, disabled, depth_prepass}` | `RenderState.graph_disabled/graph_order/depth_prepass` já existem no núcleo (`SetRenderSettings`). `dof_post` só roda com `scene.dof.enabled` |
| DoF | passe `dof_post` (bokeh) controlado por `Scene.dof` | `DofSettings` **não** está no `ProjectState` → persistir em `RenderOutput.dof` |
| Tonemap / HDR / bloom / grain | **inexistentes** (T-3.7/T-3.10 ⬜; alvo 8 bits) | `RenderState.tonemap` é persistido mas **sem efeito**; não fingir na UI (ver §4) |
| AOVs (profundidade, ID, normal) | **não há** saída por passe; o buffer de profundidade é interno ao `RenderTargetCache` | **rasterizador por software** (CPU, determinístico) sobre as mesmas malhas/câmera → profundidade/ID/normal sem tocar em shader/contrato |
| Contorno isolado | só existe composto na cor | **diferença** entre o render com `outline` ligado e desligado (mesma câmera/AA) → RGBA só com a linha, com cobertura |
| Readback | `render_scene` devolve `RgbaImage`; limite de textura 8192 px (limites padrão do wgpu) | clamp do supersampling ao limite e ao orçamento de memória |

Por que rasterizador por software e não um shader novo: shader/uniform novos entram no `render_contract_v1.json` (hashes FNV de cada WGSL, paridade com o
viewport TS) — custo alto e risco para os goldens por um benefício que a CPU resolve **bit a bit igual em qualquer GPU** (AOVs determinísticos entre
lavapipe e GPU real). Se um dia o desempenho exigir, o rasterizador vira shader aditivo atrás de um passe desligado por padrão (fase 3).

## 3. Proposta de escopo em fases

### MVP (implementado)
1. **Câmera de render**: escolher a câmera de cena (nós `Camera`) como câmera de render (`camera_command_from_node`), FOV ↔ focal (sensor), near/far (novo no `SetCamera`),
   guias (terços, centro, ação/título seguros), enquadrar (cena inteira / seleção) respeitando o aspecto de saída.
2. **Configurações**: resolução com presets (1080p, 4K, A4/A5/B5 300 dpi, quadrinho, ilustração, redes sociais), bloqueio de aspecto, troca de orientação, escala %, supersampling 1–4×,
   fundo sólido/transparente, espaço de cor de saída (sRGB / Display P3 por `cICP`), passes ligados/desligados (`graph_disabled`), DoF, formato (PNG 8/16 bits, JPEG), DPI.
   Tudo em comandos do núcleo com undo/redo (`SetRenderOutput`).
3. **Preview** da câmera de render: máscara de enquadramento, margens seguras, resolução/aspecto, xadrez quando transparente.
4. **Render real** de um quadro: thread de GPU dedicada (device próprio), readback, redução, PNG com alfa reto e metadados (versão, hash do projeto, câmera, configurações), nome por template + contador,
   pasta por caminho digitado + sugestão, **fila** com progresso/cancelar/retomar e **histórico com miniaturas** (ativo de imagem opcional na Biblioteca).

### Completo (implementado na medida do que cabe)
5. **AOVs aditivos, desligados por padrão**: contorno isolado, profundidade (normalizada), máscara (alfa de cobertura), ID por objeto (cores únicas determinísticas, sem AA), normais (espaço da câmera, sem AA).
6. Gancho de **sequência**: o job já carrega `frames` e o template tem `{frame}`/`%04d`; "retomar" pula quadros já gravados. A fonte de quadros (Animação) só precisa implementar `FrameSource`.
7. Detectar `ffmpeg` (apenas `ffmpeg -version`) e mostrar a linha de comando sugerida para montar a sequência; **não** executa o encode.

### Avançado (documentado, não implementado)
* EXR/float (exige alvo de render > 8 bits; T-3.7), tonemap neutro real, bloom/grain/LUT como passes de pós (T-3.10), região de render, pixel não quadrado,
  render em tiles (câmera fora de centro → exige mudar a projeção do renderer) para além de 8192 px, lote de múltiplas câmeras (estilo Marmoset), Cryptomatte, temporal AA/motion blur,
  vídeo direto (`ffmpeg` como processo filho) e fila persistente entre execuções.

## 4. Decisões e limites honestos
* **MSAA** continua 4× (contrato). A UI mostra "MSAA 4× (contrato)" e oferece **supersampling**, que é AA de verdade e determinístico.
* **Tonemap** não é oferecido como controle ativo: com alvo de 8 bits o "neutro" aplicado depois só escureceria realces já saturados e destruiria cores chapadas do cel. A UI mostra o valor do projeto como somente leitura, com a causa (T-3.7).
* **Display P3**: a paleta cel é autorada em sRGB; converter para P3 só **reencoda** os valores num contêiner de gamut maior (sem cores novas) e marca `cICP`. Útil para pipeline P3; não "expande" a cor.
* **Determinismo**: os bytes do PNG não levam data/hora (a data vai só no histórico); mesmo projeto + mesmo adaptador ⇒ mesmos bytes (testado). Entre GPUs diferentes os pixels do renderer divergem (goldens valem só para lavapipe), os AOVs por software não.
* **Divisão nuvem × local**: nuvem (lavapipe/Xvfb) valida pipeline, bytes, alfa, enquadramento, AOVs e fila; **local** valida GPU real (tempo, memória em 4K×SS3), leitores de PNG com `cICP`, gosto do AA e do contorno isolado em composição real (Photoshop/Clip Studio) e o `ffmpeg` do sistema.

## 5. Fontes
* Blender Manual — Output / Format / Passes (docs.blender.org/manual/en/latest/render/…) [V].
* Unreal Engine — *Cinematic Rendering Image Quality Settings* e *Taking Screenshots* (dev.epicgames.com) [V via excerto].
* Marmoset Toolbag — *Render Window* (docs.marmoset.co/docs/render-object/) [V].
* CLIP STUDIO TIPS — "How to use 3d models as background" (Line Extraction → lines and tones) [S].
* W3C — *Portable Network Graphics (PNG) Specification, Third Edition* (cICP, tEXt, iCCP, sRGB, pHYs, ordem de chunks) [V]; Chris Lilley, "cICP in PNG, explained" [S].
