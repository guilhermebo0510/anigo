# Pesquisa — Workspace Animação nativo (UX de animação 3D de personagens, keyframes, formatos, física no playback)

Legenda de confiança: **[V]** = documentação oficial lida (trecho conferido nesta sessão) · **[S]** = fonte secundária (tutorial, fórum, blog, resumo de busca) ·
**[C]** = conjectura/decisão de projeto minha, não afirmada por fonte. Datas de leitura: 2026-10. Onde uma fonte não pôde ser lida (páginas do Unreal
foram lidas só por excerto de busca; a página de Maya não carregou), está marcado `[S]`.

Escopo do estudo: como os editores de referência modelam **tempo, chaves e curvas**; o que um workspace de animação de personagem *anime* precisa
(limited animation, "twos/threes", holds); como importar/exportar **glTF animation / VRMA / BVH / FBX** e **retargetar** para o esqueleto canônico de
24 ossos do ANIGO; e como a **física secundária** (cabelo/saia) entra num playback com salto no tempo.

## 1. O que o ANIGO já tem (ponto de partida)

* `ProjectState.animation` já existe com `AnimationClip { clip_id, name, duration_seconds, tracks[] }`, `AnimationTrack { node_id, keyframes[] }` e
  `TransformKeyframe { time_seconds, translation, rotation, scale }` por **nó**, mais `active_clip` e `playhead_seconds` (`crates/anigo-core/src/project.rs`).
  Não há interpolação por chave, nem canais de osso, nem taxa de quadros, nem comandos de edição — só dados. O JSON canônico escreve
  `"animation": { "clips": {}, "active_clip": null, "playhead_seconds": 0.0 }`, congelado nos fixtures.
* `PoseState`/`Command::SetPose`/`PoseRig`/`PoseAsset` (workspace Posing): a pose é "delta de rotação local por osso (quaternion), translação de
  `Root`/`Hips`, curvatura de dedos" e **já é a unidade de um keyframe** (`PoseState::blend` faz slerp/lerp; `plan_pose_change` diferencia poses).
* `CoreSession::{apply, undo, redo, begin_preview, preview, commit_preview}`: preview ao vivo fora do histórico; soltar vira UM passo de desfazer.
* `render_out::job::{FrameSource, FrameRange}`: o gancho que a Render deixou para sequências (`%04d`, retomar pulando quadros existentes).
* `anigo-physics` (`PhysicsWorld`, passo fixo de 1/60 s, `reset`, teleporte automático) e `anigo-vrm` (parser glTF/VRM, `read_accessor_f32`, `build_glb`).

## 2. Como os editores de referência modelam animação

### 2.1 Blender: Timeline, Dope Sheet, Graph Editor, NLA
* **Interpolação por chave**: Constant, Linear, Quadratic (e demais easings) e **Bézier** livre [V: manual, "Keyframes — Introduction"]. Ao criar uma
  chave o padrão é Bézier, o que gera ease-in/out nas pontas [S: discussão de usuários]. Na página de propriedades da F-Curve: Interpolation
  (Constant/Linear/Bézier), **Easing** (Automatic, Ease In, Ease Out, Ease In Out), tipo de alça (Automatic, Auto Clamped, Vector, Aligned, Free) e posição
  da alça como (Frame, Value) [V: manual, "F-Curve Properties"]. O Dope Sheet mostra a interpolação/tipo de alça por segmento; o Graph Editor edita as curvas [V].
* **Extrapolação** Constant/Linear por canal e *modifiers* (Cycles) para loop [S: resumo do manual/tutoriais; página não lida integralmente].
* **NLA**: "pilha de trilhas que funcionam como camadas… trilhas mais altas têm precedência, mas é possível misturar"; cada trilha tem *strips*, tipicamente
  *action strips* = instâncias de *actions*, e actions são "segmentos de animação nomeados e reutilizáveis" [V: manual NLA, "Introduction"]. Modos de
  mistura (Replace/Combine/Add/Subtract/Multiply), influência, repeat/scale não puderam ser lidos na página consultada [S]. **Lição**: clip = unidade
  reutilizável; instância/strip com tempo próprio; mistura é camada acima do clip. Isso fica na fase "avançado".
* Auto-keying e tempo: *Auto Keying* grava chaves ao editar; scrub do playhead muda o estado da cena **sem entrar no undo** (comportamento do Blender
  [S/C]: o estado de cena é função de `frame_current` + curvas; mudanças não-chaveadas são perdidas ao trocar de quadro). Copiamos esse modelo.

### 2.2 Unreal Sequencer + Control Rig
* Tracks contêm **Sections**; chaves vivem em seções; seções têm intervalo e "Pre/Post-Infinity" (**Constant** padrão, **Cycle**, **Cycle with Offset**,
  **Oscillate**, **Linear**) [V: Epic, "Creating Animation Keyframes"]. Isso mapeia exatamente em `LoopMode` (Once/Loop/PingPong) e extrapolação do ANIGO.
* **Auto Key** é um botão da toolbar: mexer numa propriedade cria chave; "Enter" cria chaves nas tracks selecionadas; "Rekey" move a chave para o playhead;
  "Snap to Frame" arredonda ao fps do projeto; "Reduce Keys" com tolerância; seleção por caixa; setas movem 1 quadro; "Key Interpolation" aplica tipo de
  tangente a uma seção [V: Epic, mesma página + "Sequencer Cinematic Toolbar"]. Barra tem seletor de FPS e opção de travar ao fps [V].
* **Restore Pre-Animated State** (volta ao estado editor) [V]; "When Finished: Keep/Restore State" [V] — informa a nossa decisão de **não** persistir a
  avaliação como edição.
* Control Rig (animar o rig em vez dos ossos) = nosso Posing FK/IK; o IK do Posing é "gravado como FK" — ao keyar, gravamos FK (decisão consistente com o Posing).

### 2.3 Maya: Time Slider / Graph Editor
* Time Slider com **faixa de animação** (início/fim) e **faixa de reprodução** contida nela; unidades de tempo (fps) do projeto; loop Once/Loop/Oscillate;
  auto-key; ticks de chave [S: excertos da ajuda da Autodesk; página principal não carregou]. Graph Editor: tipos de tangente (spline, linear, flat, stepped…)
  [S/C]. Adotamos: fps é do **projeto**, e há **faixa do clip** (range) separada do playhead.

### 2.4 Cascadeur, Clip Studio Paint (3D/timeline), Mixamo
* **Ghosts** (Cascadeur): modos "nenhum", "quadros anteriores", e variantes com quadros seguintes/chaves [S: excerto da ajuda do Cascadeur] → *onion skin* 3D
  = renderizar poses vizinhas semi-transparentes. No ANIGO, como o renderer cel não tem passe translúcido (contrato congelado), os fantasmas são
  **silhuetas de esqueleto/juntas coloridas** sobrepostas (anterior frio, seguinte quente) [C — decisão por restrição de renderer].
* **Mixamo**: anima personagens humanoides, exporta FBX/BVH com ossos `mixamorig:*`; uso comum é retargeting por Humanoid (Unity Mecanim) ou
  "Humanoid Avatar" que mapeia por papel do osso, não por nome [S: fóruns Unity/Reallusion]. → mapa **nome → osso canônico** com normalização
  (`mixamorig:LeftArm` → `LeftShoulder` no nosso vocabulário VRM-like), prefixos e *fuzzy* de sufixos [C].
* Clip Studio/pegada 2D: a ideia de linha do tempo por **células/quadros inteiros** com "exposure" (segurar quadros) — base do nosso "animar em 2s/3s".

## 3. Keyframes e interpolação — decisões técnicas

* **Quaternions: slerp pelo caminho curto**. glTF 2.0: LINEAR em rotação *deve* ser slerp [V: spec glTF 2.0, "animation.sampler"]; o pseudo-código do tutorial
  Khronos nega o quaternion se `dot < 0` (caminho curto) e cai em lerp normalizado se `dot > 0.9995` [V: Khronos glTF-Tutorials, "Animations"]. Fazemos igual
  (testes: sem flip entre chaves com sinais opostos).
* **STEP** mantém o valor da chave até a próxima [V: glTF]; **CUBICSPLINE** usa hermite com tangentes `vt = (2t³−3t²+1)vk + td(t³−2t²+t)bk + (−2t³+3t²)vk+1 + td(t³−t²)ak+1`
  [V: glTF 2.0 spec, Apêndice C] — usado só na **importação** (convertemos para chaves amostradas + Bézier).
* **Fora do intervalo**: glTF exige *clamp* nas extremidades [V]; é nossa extrapolação padrão (Constant), com Linear opcional [C].
* **Bézier por segmento**: em vez de alças livres em (frame, valor) como no Blender, cada chave guarda `Interp::Bezier { c: [x1,y1,x2,y2] }` normalizado no segmento
  (equivalente a `cubic-bezier` do CSS / "temporal ease" de After Effects) [C]. Vantagens: serve igual para escalar, vetor e **quaternion** (o parâmetro `u` do
  slerp é remapeado por `ease(u)`), garante **monotonia no tempo** se `x1,x2 ∈ [0,1]`, `y` fora de `[0,1]` dá *anticipation/overshoot* (muito usado em anime),
  e o inverso é exato. Presets Ease/EaseIn/EaseOut são Béziers fixos (0.42,0,0.58,1 etc.) [C, valores = convenção CSS `ease-in-out`].
* **Euler vs quaternion**: o Graph Editor mostra rotações como **curvas Euler XYZ derivadas** (com *unwrap* para continuidade), somente leitura; a edição de
  valores é no gizmo (Posing). Chaves guardam quaternions → sem gimbal lock e sem "flip" [C, alinhado ao "Quaternion (WXYZ)" do Blender].
* **Tempo inteiro**: chaves em **quadros inteiros** (`i32`) e `fps` do projeto; segundos = quadro/fps. Retime ao mudar o fps é opção (escalar quadros) com inverso
  por snapshot [C]. Timecode `HH:MM:SS:FF` (não-drop-frame; 23.976/29.97 NDF aproximados e avisados) [C].
* **Anime / limited animation**: anime é majoritariamente **em 3s** (8 desenhos/s a 24 fps), vai para 2s em ação rápida e 1s em picos [S: Reddit r/animation;
  Animétudes "framerate modulation"; iD Tech]: "animar em 2s" = 12 poses novas/s, "em 3s" = 8/s; o ritmo varia por movimento ("modulation": 2s num trecho, 3s noutro)
  [S: animetudes.com]. Daí duas ferramentas: (a) **interpolação Step por chave** (blocking / pose-to-pose, "holds"), (b) **`hold_step` do clip** (1/2/3): a
  avaliação usa `floor((f − início)/N)·N + início`, transformando QUALQUER curva suave num movimento "em Ns" sem perder as chaves originais — reversível e determinístico.
  *Smears* e quadros extras ficam como chaves normais com Step de 1–2 quadros [C].
* **Loop / ping-pong**: Once (clamp), Loop (período = fim − início; o quadro `fim` equivale a `início`), PingPong (vai e volta sem repetir as pontas) [V: Sequencer
  Cycle/Oscillate; C: fórmulas].
* **Auto-key**: quando ligado, editar a pose (arrasto de gizmo, IK, presets, colar) grava chaves **apenas dos canais alterados** no quadro atual, no MESMO passo de
  desfazer da edição (`Batch[SetPose, EditAnimation::InsertKeys]`) [C, equivalente ao Auto Key do Sequencer/Blender]. Desligado, a edição é só uma pose temporária
  (perdida ao mover o playhead) e a UI avisa "mudança sem chave" [C].

## 4. Formatos e retargeting

| Formato | Papel | Fonte |
|---|---|---|
| **glTF 2.0 animation** | canais `translation/rotation/scale/weights` por nó; samplers STEP/LINEAR/CUBICSPLINE; tempos estritamente crescentes; clamp fora do intervalo | [V] spec |
| **VRMA** (`VRMC_vrm_animation-1.0`) | glTF + `humanoid.humanBones` (osso humanoide → nó); **proíbe escala** e **translação fora de Hips**; rest "VRM T-pose"; `lookAt`/`expressions` | [V] spec |
| **BVH** | `HIERARCHY` (`ROOT/JOINT/End Site`, `OFFSET`, `CHANNELS`), `MOTION` (`Frames:`, `Frame Time:`); números por quadro "na ordem dos CHANNELS conforme o parse da hierarquia"; rotações concatenadas na ordem dos canais | [V] spec Wisconsin |
| **FBX** | via `ufbx` (já em `anigo-native`/`anigo-assets`) | [S] CPP_FFI.md — fase avançada |

**Retargeting**. A spec VRMA dá a fórmula FK entre T-poses: `Normalized = W·L⁻¹·localA·W⁻¹` e `localB = L_B·W_B⁻¹·Normalized·W_B`, com `W` = rotação de mundo
de repouso e `L` = local de repouso; ossos ausentes no destino propagam a rotação aos filhos; a translação do Hips escala por `y_dst / y_src` [V: how_to_transform_human_pose]. Isso supõe
que ambos os rigs repousam em T-pose. O rig canônico do ANIGO repousa com **braços pendurados** e todas as rotações de repouso = identidade (ver `POSING_UI.md`) → a fórmula
FK cega leva os braços para a direção errada. Decisão [C]: retarget **por direção de osso no mundo**: para cada osso, `d_dst = (rotação de mundo animada da fonte)·(direção de repouso da fonte)`;
a rotação de mundo do destino é o *swing* mínimo `dir_repouso_destino → d_dst` mais a *torção* da fonte em torno do eixo do osso; depois `local = pai⁻¹ · mundo`. Com repousos iguais reduz-se
à fórmula da spec. Hips escalam por `y_dst/y_src` [V]. Mapeamento de nomes: VRM humanoid (`leftUpperArm`→`LeftShoulder` no vocabulário do ANIGO, que usa ombro/cotovelo/punho),
Mixamo (`mixamorig:*`), CMU/BVH genéricos (heurística por nome) [C]. Dedos: o rig canônico não tem ossos de dedo (curvatura 0–1): ignorados na importação; exportamos só os 24 ossos.

## 5. Física secundária no playback (cabelo/saia)

* **Passo fixo** com acumulador e *clamp* de quadro longo contra a "espiral da morte" [V: Gaffer On Games, "Fix Your Timestep": `accumulator += frameTime; while (accumulator >= dt) {…}`;
  `if (frameTime > 0.25) frameTime = 0.25`; interpolar o resto com `alpha = accumulator/dt`]. `PhysicsWorld::step` já faz isso a 60 Hz (FISICA.md §3.6).
* **Determinismo no scrub/pulo**: simulação é *estado* — o resultado em `t` depende da história. Para ser função de `(clip, t)`: ao **saltar** (scrub, loop, ir ao início, tocar do
  meio) fazemos `reset` na pose do quadro `t − pre_roll` e **simulamos de volta até `t`** a 1/60 s (pré-aquecimento). Durante o playback contínuo só avançamos os passos novos. Com
  `pre_roll` fixo (padrão 0,5 s) e passo fixo o resultado é idêntico entre execuções (teste de hash). Teleportes grandes são tratados pelo próprio mundo (`teleports`) [FISICA.md].
  Engines de produção expõem "Reset/Teleport" para evitar cabelo arrastado após pulos de câmera/personagem [S: fóruns Unreal Chaos Cloth / Ragdoll Dynamics: "se você resetar, os
  'danglies' começam com velocidade zero e demoram a assentar" → daí o pré-aquecimento em vez de reset seco] .
* **Estado do renderer**: o cabelo/saia do ANIGO ainda não entram no renderer (decisão A/B pendente, FISICA.md §7). Entrega: a simulação roda de verdade sobre os ossos do clip
  (cadeias/ saia de teste + colliders do corpo) e aparece como **overlay de depuração** (partículas e segmentos) no viewport; dados exportáveis (posições por quadro).

## 6. Proposta de escopo em fases

**MVP (obrigatório, tudo ligado a dados reais)**
1. Núcleo aditivo: `AnimationState.fps`, `AnimationClip.{channels, range, loop_mode, hold_step, markers…}`, canais tipados (osso/root/hips/mãos/nó/luz/câmera), `Interp`,
   `evaluate`, `Command::EditAnimation(AnimEdit::{InsertKeys, DeleteKeys, MoveKeys, SetInterpolation, SetClipRange, SetFps, …})` com inversos exatos; TS em paridade.
2. Sessão: amostragem ao vivo fora do histórico (`apply_animation_sample`), reamostrar após undo/redo.
3. UI: timeline com régua/timecode, scrub, play (Espaço), setas/Home/End, loop/ping-pong, velocidade; dope sheet por osso/nó com seleção por clique/caixa; mover/escalar/duplicar/
   espelhar chaves (arrastar = preview, soltar = um comando); inserir chave (I); auto-key do Posing.

**Completo (o que couber)**
4. Graph Editor com alças Bézier arrastáveis; onion skin; marcadores; "animar em 2s/3s"; clips múltiplos (duplicar/renomear); clip como ativo da Biblioteca
   (`.anigoclip.json`, miniatura em tira) e aplicar um clip de um card.
5. Importar/exportar glTF animation e VRMA; BVH importar/exportar; retarget por direção.
6. Física no playback (pré-aquecimento determinístico + overlay de depuração).
7. "Renderizar sequência" → `FrameSource`/`FrameRange` da Render (PNG `%04d`).

**Avançado (não prometido nesta entrega)**
8. NLA (strips, mistura Replace/Add, influência, time-warp), curvas de retime, bake de root motion, FBX via `ufbx`, motion paths, keys de IK/constraints, expressões e lip-sync,
   *auto in-between* assistido (estilo Cascadeur), interpolação de apresentação >60 Hz para a física.

## 7. Divisão nuvem × local
* **Nuvem (feito/validável aqui, Xvfb + lavapipe)**: todo o núcleo e testes (avaliação, comandos, inversos, importadores com arquivos sintéticos), a UI inteira, PNGs de poses em
  tempos diferentes (scrub/playback/step vs Bézier), smoke xdotool no Xvfb próprio (`:95`), sequência de render real em lavapipe.
* **Local/humano**: sensação do scrub e da reprodução em tempo real (60/144 Hz) em GPU real; gosto de easings e do onion skin; importar arquivos reais (Mixamo, VRMA do VRoid Hub,
  BVH CMU/Mixamo) e conferir o retarget; sincronia áudio/vídeo (não há áudio); tempo de render 4K de sequências; ffmpeg real; Windows/macOS.

## Fontes
* Blender Manual (5.2 LTS): Keyframes — Introduction; F-Curve Properties; NLA — Introduction. [V]
* Khronos: glTF 2.0 Specification (animation, sampler, Apêndice C); glTF-Tutorials "Animations". [V]
* vrm-c/vrm-specification: `VRMC_vrm_animation-1.0/README.md` (excerto) e `how_to_transform_human_pose.md`. [V]
* Biovision BVH (Wisconsin CS-838): `BVH.html`. [V]
* Gaffer On Games: "Fix Your Timestep!". [V]
* Epic: "Creating Animation Keyframes in Unreal Engine"; "Sequencer Cinematic Toolbar". [V — excerto de busca com o texto das páginas]
* Autodesk Maya (Time Slider preferences, excerto). [S] · Cascadeur "Ghosts" (excerto). [S]
* Animétudes "Animation and subjectivity: towards a theory of framerate modulation"; iD Tech "animating on ones, twos, threes"; r/animation. [S]
* Fóruns Unity/Reallusion (Mixamo + Mecanim), Unreal Chaos Cloth / Ragdoll Dynamics (teleport/reset). [S]
