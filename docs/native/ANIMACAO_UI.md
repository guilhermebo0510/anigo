# Workspace Animação — UI nativa (`anigo-studio`) e modelo de animação do núcleo

**Estado:** MVP + "completo" + parte do "avançado" de [`pesquisa/animacao.md`](pesquisa/animacao.md) implementados e validados no sandbox
(Xvfb + Vulkan por software/lavapipe, sem GPU real, sem mouse/teclado reais de um artista). Cobre a parte de Animação de T-2.11.

Legenda: **[T]** coberto por teste automático que rodou · **[S]** visto em screenshot ou em entrada real (xdotool) no Xvfb · **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais: o clip é parte do `ProjectState` canônico, editado por `Command::EditAnimation` através da `CoreSession` (a MESMA do
Personagem, Posing, Cenário e Render). Scrub e reprodução **não** são edições: o clip é avaliado no playhead e a amostra é aplicada ao projeto fora do
histórico (como o Blender), então Ctrl+Z nunca "desfaz um frame de reprodução".

| Área | O que faz | Validação |
|---|---|---|
| **Modelo** (`anigo-core/src/animation.rs`) | `AnimationClip` com canais tipados (rotação por osso, translação de `Root`/`Hips`, curvatura de dedos por mão, translação/rotação/escala de nós, sol, câmera de cena), chaves em quadros inteiros, interpolação **por chave** (degrau, linear, suave, entrada/saída suave, Bézier com alças), faixa, loop/ping-pong, extrapolação (segurar/continuar), "animar em 1s/2s/3s" (`hold_step`), marcadores; `AnimationState.fps` | [T] 29 testes em `animation/tests.rs` |
| **Avaliação pura e determinística** | `AnimationClip::evaluate(fps, quadro)`: slerp pelo caminho curto sem flip, degrau, Bézier monotônico no tempo (x em 0..1; y fora de 0..1 dá overshoot), loop/ping-pong, extrapolação, quantização do "em Ns" | [T] mesma entrada = mesmo resultado bit a bit; quaternions sempre unitários; nenhum NaN em chaves extremas |
| **Comando** | `Command::EditAnimation { edit: AnimEdit }` (antes de `Batch`, mesma ordem em Rust e TS): `PutClip`, `RemoveClip`, `SetActiveClip`, `RenameClip`, `InsertKeys`, `DeleteKeys`, `MoveKeys`, `SetInterpolation`, `SetClipRange`, `SetClipPlayback`, `SetFps`, `SetMarkers`. Inverso exato (snapshot dos clips/chaves tocados; `SetFps` com retime usa `PutClip`) | [T] apply→undo→redo em sequências aleatórias volta ao estado idêntico (propriedade); contrato TS (`npm test`, `fixtures:check`) |
| **Linha do tempo** | régua com timecode (`hh:mm:ss:ff`), zoom na roda/pan MMB, faixa do clip, playhead, marcadores; clique/arrasto na régua = scrub | [S] smoke (clicar no tempo), capturas |
| **Reprodução** | Espaço, setas (quadro), Shift+setas (±10), ↑/↓ (chave anterior/seguinte), Home/End, uma vez/repetir/vai e volta, velocidade ×0,25–×2 e reversa; por padrão toca em quadros inteiros (o que se vê = o que o render da sequência produz) | [T] `anim::playback`; [S] smoke (Espaço) |
| **Dope sheet** | chaves por osso/nó, linhas de resumo e de grupo recolhíveis, filtro "só ossos selecionados", barras de "hold" nos degraus; clique, Shift/Ctrl, caixa de seleção | [T] `anim::keys`; [S] capturas e smoke |
| **Mover/escalar/duplicar/espelhar chaves** | arrasto move (preview ao vivo, **UM** comando ao soltar); Shift+arrasto duplica; Alt+arrasto escala no tempo em torno do playhead; espelhar E/D (`LeftElbow`↔`RightElbow`, X da translação); copiar/colar/colar espelhado no playhead; Esc cancela | [T] planejadores puros; [S] smoke (arrastar chave = +1 passo, durante o arrasto o histórico não muda) |
| **Inserir chave** | `I` / botão: ossos selecionados (ou o corpo todo) com o valor vivo; `Alt+I` apaga no quadro; sem clip, o clip nasce **no mesmo passo** (`Batch[PutClip, SetActive, InsertKeys]`) | [T] um passo de undo; [S] smoke |
| **Auto-key** | amarrado à edição do Posing (gizmo FK, IK, painel numérico, presets): a edição vira `Batch[SetPose, InsertKeys]` = um passo; "Pose sem chave" quando a pose editada difere do clip | [T] `anim::autokey`; [S] cenários montam o clip por essa rota |
| **Graph editor** | curvas por canal (rotação em graus Euler contínuos), canais visíveis por olho, escala vertical automática, alças Bézier arrastáveis (um comando ao soltar), presets de interpolação | [S] `animacao-curvas` |
| **Onion skin** | fantasmas do **esqueleto** nos quadros ou chaves vizinhos (azul antes, verde depois, opacidade decrescente) | [T] `onion_ghosts`; [S] `animacao-onion` |
| **Clips** | lista, novo, duplicar, renomear, apagar, escolher o ativo; faixa, fps (com/sem "manter a duração"), extrapolação, "animar em Ns" | [S] `animacao-clips` |
| **Biblioteca de clips** | "Salvar na Biblioteca" grava `.anigoclip.json` como ativo `Animation` (mesmo nome = nova revisão; miniatura = tira de 4 poses cel); aplicar como clip novo ou inserir no playhead do clip ativo (um comando, desfaz) | [T] `clip_bridge`, `anigo-assets` `import_flow`; [S] `animacao-clips` |
| **Importar / exportar** | glTF/GLB/VRM/**VRMA** (STEP/LINEAR/CUBICSPLINE), **BVH** e `.anigoclip.json`; exporta GLB, VRMA (T-pose + nomes humanoides + `VRMC_vrm_animation`), BVH e clip. Esqueletos humanoides (VRM, Mixamo, VRoid, Rigify, Biped, CMU) mapeiam para os 24 ossos canônicos | [T] roundtrips, retarget com repouso diferente, Z-up; [N] arquivos reais de artista |
| **Física secundária** | cabelo (cadeias na cabeça) e saia (malha no quadril) de teste simulados de verdade (`anigo-physics`, passo fixo 1/60 s) contra as cápsulas do corpo; salto no tempo = reinício + pré-aquecimento; "Gravar simulação" salva as posições por quadro | [T] determinismo (mesmo `t`, mesmo hash de estado, qualquer caminho); [S] `animacao-fisica` |
| **Chaves de cena** | sol (direção/intensidade/cor), câmera de cena (FOV/posição/alvo) e transformação de um objeto da cena | [T] `current_value`, avaliação; [N] captura dedicada |
| **Renderizar sequência** | quadros PNG por quadro do clip (ou intervalo/passo), pela fila do Render (`FrameSource`/`FrameRange`), template `%04d`/`{frame:04}`, cancelar/retomar, linha `ffmpeg` sugerida (só texto) | [T] teste real na GPU headless: um PNG por quadro, poses diferentes; [S] `animacao-sequencia` |
| **Undo/redo** | Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z e os botões do topo na `CoreSession`; histórico de 400 passos | [S] smoke (Ctrl+Z/Ctrl+Y reais) |

## 2. Atalhos e interação

| Ação | Como |
|---|---|
| Ir a um quadro | clicar/arrastar na régua; campo "Quadro" |
| Reproduzir / pausar | Espaço |
| Quadro ± 1 / ± 10 | ←/→ / Shift+←/→ |
| Chave anterior / seguinte | ↓ / ↑ |
| Início / fim | Home / End |
| Inserir chave / apagar chaves aqui | I / Alt+I |
| Selecionar chaves | clique, Shift/Ctrl+clique, caixa no dope sheet, Ctrl+A (sobre a timeline) |
| Mover / duplicar / escalar chaves | arrastar / Shift+arrastar / Alt+arrastar; Esc cancela |
| Apagar chaves selecionadas | Delete ou X (sobre a timeline) |
| Copiar / colar / colar espelhado | Ctrl+C / Ctrl+V / Ctrl+Shift+V (sobre a timeline) |
| Espelhar chaves / marcador | Shift+M / M (sobre a timeline) |
| Enquadrar a faixa | F (sobre a timeline) |
| Desfazer / refazer | Ctrl+Z / Ctrl+Y ou Ctrl+Shift+Z |

Fora da timeline valem os atalhos do Posing (copiar/colar pose, espelhar, resetar). Ferramentas do shelf: **Linha do tempo** (dope sheet),
**Curvas** (Graph editor), **Clips** (lista do projeto + Biblioteca).

## 3. Modelo de dados e API

```text
ProjectState.animation : AnimationState
  fps          : f32                         // padrão 24; 1..=240
  clips        : BTreeMap<AnimationClipId, AnimationClip>   // até 64
  active_clip  : Option<AnimationClipId>
  playhead_seconds
AnimationClip (campos novos, todos #[serde(default)] + skip se padrão -> projetos e fixtures antigos não mudam)
  channels     : Vec<Channel { target: ChannelTarget, keys: Vec<Key { frame: i32, value: KeyValue, interp: Interp }> }>
  range        : Option<[i32;2]>             // None = 0..maior chave
  loop_mode    : Once | Loop | PingPong
  extrapolation: Constant | Linear
  hold_step    : u8                          // 1 | 2 | 3 ... 12 ("animar em Ns")
  markers      : Vec<Marker { frame, name }>
Interp = Step | Linear | Ease | EaseIn | EaseOut | Bezier { c: [x1,y1,x2,y2] }   // da chave até a próxima
```

* **Bézier:** cubic-bezier normalizada que remapeia o parâmetro do segmento (como o CSS); `x1,x2 ∈ [0,1]` garante tempo monotônico, `y` fora de 0..1 dá overshoot. Vale para escalar, vec3 e quaternion (slerp com o parâmetro remapeado).
* **Quadros inteiros** + fps do projeto (padrão do Blender/Maya/Unreal); `SetFps` com `retime` reescala os quadros para manter a duração em segundos.
* **Aplicar uma amostra:** `CoreSession::apply_animation_sample(&Sample)` escreve pose/nós/sol/câmera **sem histórico** (recusada enquanto há `preview` aberto); `set_playhead_seconds`.
* **Retarget:** por **direção de osso** (swing+twist em mundo, correção de quadro encaixada nas 24 rotações de eixo, translação do quadril escalada pela razão de alturas) — não pela fórmula FK do VRMA, porque o rig canônico repousa com os braços pendentes e não em T.
* **Arquivos:** `anigo-vrm::anim_io` (glTF/VRMA), `anigo_core::bvh` (cm, canais ZXY), `anigo_core::clip_asset` (`.anigoclip.json`, `anigo.clip` v1, limite de 8 MiB); `anigo-assets`: `Format::AnigoClip`, análise validada, miniatura em tira.
* **Contrato TS:** `src/contracts/commands.v1.ts` (`edit_animation`, mesma ordem de variantes), `command_scope.ts`, `command_builder.ts`; `render_contract_v1.json` e os frame goldens **não mudaram**.

## 4. Arquitetura

```text
anigo-core     animation.rs (+tests)  modelo, avaliação, AnimEdit (validate/apply/inverse)                     [T]
               retarget.rs, bvh.rs, clip_asset.rs   mapeamento de nomes, retarget por direção, BVH, ClipAsset [T]
anigo-vrm      anim_io.rs   importar/exportar glTF e VRMA                                                      [T]
anigo-assets   Format::AnigoClip, análise, miniatura em tira                                                   [T]
anigo-session  apply_animation_sample, set_playhead_seconds                                                    [T]
anigo-studio   anim/          módulos puros e testados: playback, keys, graph, autokey, secondary, sequence
               app/animation_ui/ UI: mod (estado/comandos/atalhos), timeline (régua, dope sheet, curvas, clips), panels (barra,
                                 inspetor, arquivos, sequência, Biblioteca), viewport (onion, física, HUD, trace), script (cenários)
               library_ui/    clip_bridge.rs (salvar/listar clips) + miniatura em tira em cel_thumb.rs
               i18n_animation.rs  textos pt/en (`anim.*`)
```

Ganchos em arquivos compartilhados (mínimos e aditivos): `app.rs` (estado, painéis, undo/redo, status, entrada/saída do workspace), `automation.rs` (cenários),
`i18n.rs`/`lib.rs`, `workspace.rs` (ferramentas timeline/curvas/clips), `character_ui.rs` (undo/redo roteado), `posing_ui` (envolver a edição de pose no auto-key; o viewport
é o do Posing com camadas extras), `render_ui` (`render_submit`/`render_service` visíveis ao crate).

## 5. Capturas e smoke

Cenários: `animacao-vazio`, `animacao-timeline`, `animacao-curvas`, `animacao-passo`, `animacao-bezier`, `animacao-onion`, `animacao-fisica`, `animacao-clips`,
`animacao-sequencia` (`anigo-studio --workspace animacao --scenario <nome> --screenshot x.png`). Todos montam o clip "Acenar" **pela rota do usuário** (auto-key
ligado, uma edição de pose por quadro-chave). Referências em `docs/native/img/animacao_*.png` (`scripts/native_screenshots.sh`; a trilha `native` do CI gera as mesmas e roda o smoke).
`ANIGO_ANIM_FRAME=<n>` escolhe o quadro do cenário (inspeção em tempos diferentes); `ANIGO_POSING_CAM` vale também.

**Verificação visual (inspecionada nos PNGs):**
* `animacao_passo.png` × `animacao_bezier.png`: mesmo clip, mesmo quadro 8 — com **degrau** o braço ainda está na pose do quadro 0; com **Bézier** forte (0,9 / 0 / 0,1 / 1) já está quase na pose do quadro 12.
* `animacao_sequencia_quadros.png`: render **real** (GPU headless, fila do Render) dos quadros 0/8/16/24 — as quatro poses diferem (também medido por contagem de pixels no teste).
* `animacao_onion.png` (fantasmas das chaves 0 e 24 no quadro 12), `animacao_curvas.png` (curvas e alças Bézier), `animacao_fisica.png` (cabelo e saia simulados), `animacao_clips.png`.

`scripts/native_animation_smoke.sh` (xdotool, **somente dentro de um Xvfb próprio** `:95`; nunca toca o display do usuário; estado do egui em `XDG_DATA_HOME` temporário;
o app imprime `animtrace ...` com `ANIGO_TRACE_ANIMATION=1`; teclas com modificador usam `keydown`/`key`/`keyup` explícitos e `windowfocus`):
1. clique na régua → o playhead vai ao quadro 12;
2. `I` → clip criado e 28 chaves no quadro 12 em **um** passo de undo;
3. clique na régua (quadro 30) + `I` → chaves também no 30;
4. arrasto da chave do quadro 12 até o 20 → durante o arrasto o gesto está em preview e o histórico **não muda**; ao soltar, **exatamente +1 passo** e a chave mudou de quadro;
5. Espaço → `playing=true` e o relógio avança; Espaço de novo → pausa;
6. Ctrl+Z → a chave volta ao quadro 12; Ctrl+Y → quadro 20.

## 6. Limites conhecidos e decisões

- **Onion skin é do esqueleto**, não da malha: duplicar a malha cel com transparência exigiria passes extras no renderer (contrato congelado).
- **Física:** o renderer ainda não consome cabelo/saia simulados (contrato de 24 ossos congelado, ver `FISICA.md` §7). A simulação roda de verdade sobre cadeias de **teste** (cabelo na cabeça, saia no quadril — não o cabelo do Modo Cabelo) e sai como overlay de depuração e como dado (`.physics.json`); não entra no PNG da sequência.
- **Dedos:** a curvatura dos dedos é um canal por mão (5 valores); o rig canônico não tem ossos de dedo, então dedos de arquivos glTF/BVH são ignorados na importação (listados nos avisos).
- **Retarget** cobre humanoides com nomes reconhecíveis (VRM/Mixamo/VRoid/Rigify/Biped/CMU); rigs sem ao menos 6 ossos mapeados são recusados. O resultado é uma aproximação por direção de osso, não um retarget de produção com IK de contato.
- **Importar/exportar** usa um campo de caminho digitado (o app não tem diálogo de arquivo nativo ainda); a pasta padrão de exportação é a de saída do Render (ou `ANIGO_ANIMATION_DIR`).
- **Escala de nós** e translação de nós dependem do projeto ter os nós; chaves de nó apagado são removidas na validação.
- Rotações são mostradas em Euler (graus) no inspetor e no Graph; chaves guardam quaternions (sem gimbal).
- Entrar no workspace amostra o clip ativo no playhead salvo; sair deixa a pose amostrada no projeto (é o estado derivado do playhead, não uma edição).
- Edição de pose sem auto-key fica "suja" (selo "Pose sem chave") até mover o playhead, quando o clip a substitui; é o comportamento esperado de um editor de chaves.
- As preferências (auto-key, onion, interpolação padrão, velocidade) não persistem entre execuções.

## 7. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)

- **Sensação** da timeline com mouse real: arrasto de chaves, zoom/pan, box select, alças Bézier do Graph (só exercitados por xdotool com passos discretos), e da reprodução em tempo real (o sandbox roda lavapipe: o ritmo de quadros não é o de uma GPU).
- **Gosto visual**: cores/espessuras de onion e física, densidade do dope sheet, tipografia em telas HiDPI.
- **Arquivos reais de artista:** FBX→glTF do Mixamo, `.vrma` de bibliotecas reais, BVH de captura de movimento (CMU, Rokoko) — os testes usam arquivos gerados pelo próprio código e esqueletos sintéticos com repouso diferente.
- **Qualidade do retarget** em animações longas e esqueletos fora do padrão; deriva do quadril.
- **ffmpeg:** a linha sugerida nunca foi executada pelo app (só texto); montar o MP4 é manual.
- **Física:** ajuste dos parâmetros (massa, rigidez) por um artista; desempenho com malhas de cabelo reais.
- Windows/macOS (Ctrl vs Cmd: o código usa `Modifiers::COMMAND`).
- `scripts/check_rust_syntax.mjs` não rodou no sandbox (falta `tree-sitter-wasms`: `npm ci` com dev dependencies); o `cargo check --workspace --all-targets` passou sem avisos.

## 8. Como rodar

```bash
cargo test -p anigo-core -p anigo-session -p anigo-assets -p anigo-vrm -p anigo-studio
cargo run -p anigo-studio -- --workspace animacao                                      # app
cargo run -p anigo-studio -- --workspace animacao --scenario animacao-bezier --screenshot /tmp/a.png
bash scripts/native_animation_smoke.sh ./target/debug/anigo-studio /tmp/animation_smoke  # precisa de Xvfb, xdotool, ImageMagick
```
Para o teste real de sequência na GPU headless (lavapipe): `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan cargo test -p anigo-studio the_render_queue`.
