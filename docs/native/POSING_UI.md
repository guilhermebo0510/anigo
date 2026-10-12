# Workspace Posing — UI nativa (`anigo-studio`) e modelo de pose do núcleo

**Estado:** MVP + "completo" + parte do "avançado" de [`pesquisa/posing.md`](pesquisa/posing.md) implementados e validados no sandbox
(Xvfb + Vulkan por software/lavapipe, sem GPU real, sem mouse/teclado reais de um artista). Cobre a parte de Posing de T-2.11.

Legenda: **[T]** coberto por teste automático que rodou · **[S]** visto em screenshot ou em entrada real (xdotool) no Xvfb · **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais (sem mockups): a pose é parte do `ProjectState` canônico, editada por `Command::SetPose` através da `CoreSession`
(a MESMA do Personagem e do Cenário), e o personagem desenhado é o do renderer cel canônico, deformado pela paleta de skinning que a sessão calcula.

| Área | O que faz | Validação |
|---|---|---|
| **Modelo de pose** (`anigo-core/src/pose.rs`) | `PoseState` (delta de rotação local por osso, translação de `Root`/`Hips`, curvatura de dedos), `PoseRig` (bind ajustado à malha, paleta de skinning, contatos dos pés, centro de massa), limites swing-twist com 3 perfis, `PoseAsset` (`.anigopose.json`), presets de corpo e mão | [T] 25 testes: paleta identidade EXATA em repouso, rigidez SE(3) em poses aleatórias, comprimentos de osso, ausência de NaN em poses extremas, envelope convexo do LBS, JSON aditivo |
| **Comando** | `Command::SetPose { bones, root_translation, hips_translation, hands, op }` (antes de `Batch`, mesma ordem em Rust e TS); inverso exato; `PoseOp` só rotula o histórico | [T] round-trip apply/undo/redo, batch, rejeição de osso desconhecido/NaN/quaternion nulo/duplicado/no-op; contrato TS (`npm test`, `fixtures:check`) |
| **Sessão** (`anigo-session`) | `CoreSession::pose()/rig()/pose_palette()`; `sync_scene` injeta `Scene.skin.palette` e curva os dedos; rig em cache por (geometria, pesos de morph) | [T] undo volta à paleta identidade bit a bit; arrasto = preview + 1 passo |
| **IK** (`anigo-ik::pose_ik`) | `solve_limb` (two-bone, pole, dobradiça anatômica), `fabrik`/`solve_chain` (coluna), `look_at_head` (sem roll, pescoço/cabeça), `lock_feet` | [T] alcança todo alvo alcançável (4 membros × 200 alvos), comprimentos preservados, pole decide o lado da dobra, alvo inalcançável estica reto, degenerados sem NaN, limites respeitados, FABRIK converge, olhar aponta a face |
| **Viewport** | personagem cel posado em tempo real; overlay de esqueleto (ossos em losango, juntas, rótulo, ponto laranja em osso posado); clique/Shift/Ctrl/caixa selecionam; hover; órbita MMB/RMB/Alt+LMB, pan Shift+MMB, zoom na roda; enquadrar; vistas | [S] capturas `posing-*`, smoke xdotool |
| **FK** | gizmo de 3 anéis no osso ativo (mundo ou local), snap 5°/15°/45° (Ctrl inverte), HUD com o ângulo, limites aplicados ao resultado, **simetria** (X-axis mirror) | [T] `posing::edit`; [S] arrasto real do anel |
| **Limites** | perfis Humano / Anime (+25 %) / Livre; cone de swing do osso ativo (verde dentro, vermelho fora) e barras swing X/Z e torção no painel | [T] `limits_vis`, `clamp_to_limit` idempotente/contínuo; [S] `posing-limites` |
| **IK por membro** | toggle FK/IK por braço/perna; quadrado (mão/pé) arrastável no plano da tela, X/Y/Z travam o eixo; losango do **pole** arrastável; Esc cancela | [T] `drag_effector`; [S] smoke: o punho segue e os tornozelos não saem do lugar |
| **Quadril e pés** | handle do quadril (Shift = corpo inteiro), **travar pés** (IK das pernas), **pés no chão** (auto-ground), "apoiar agora" | [T] `move_hips`; [S] smoke: foot lock com entrada real |
| **Coluna (FABRIK)** | a cabeça vira handle de cadeia longa (coluna/pescoço seguem, quadril fica) | [T] `solve_chain`; [N] captura dedicada |
| **Olhar** | alvo arrastável (solta aplica pescoço+cabeça sem roll), peso, "alvo à frente", "cabeça reta" | [T] `look_at_head`; [S] `posing-gaze` |
| **Equilíbrio** | centro de massa projetado + polígono de apoio dos pés, verde/vermelho | [T] `balance`; [S] `posing-ik` |
| **Painel de osso** | Euler XYZ editável (DragValue), quaternion, barras de limite, translação do quadril/raiz, resetar osso/seleção/tudo (Alt+R / Alt+Shift+R) | [S] capturas |
| **Espelhar / copiar / colar** | `M` espelha a pose em X (ou só a seleção + pares); Ctrl+C copia (seleção ou tudo), Ctrl+V cola por nome, Ctrl+Shift+V cola espelhado | [T] `paste_pose`, `mirrored` |
| **Presets** | corpo: T, A, em pé, sentado, andando, ação (todos dentro dos limites Anime e com os pés no chão); mão: aberta, relaxada, punho, apontar, V, segurar + 5 sliders por dedo (esquerda/direita/ambas) | [T] `presets_*`; [S] `posing-presets`, `posing-mao` |
| **Biblioteca de poses** | salvar a pose atual (ou só os ossos selecionados) como ativo `Pose`; lista na Biblioteca com miniatura; aplicar, aplicar espelhado, misturar (0–150 %), aplicar só aos ossos selecionados; a grade da Biblioteca mostra a miniatura 3D cel da pose (personagem posado) | [T] `pose_bridge`, import/análise/miniatura em `anigo-assets`; [S] `posing-biblioteca` |
| **Undo/redo** | Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z e botões do topo roteados para a `CoreSession`; painel Histórico com salto (`jump_to_depth`); um arrasto = UM passo | [T] `GestureTracker`; [S] Ctrl+Z/Ctrl+Y reais |

## 2. Atalhos e interação

| Ação | Como |
|---|---|
| Selecionar junta / somar / caixa | clique / Shift ou Ctrl+clique / arrastar no vazio |
| Girar osso (FK) | arrastar um dos 3 anéis do gizmo do osso ativo |
| Mover mão/pé (IK) | arrastar o quadrado; segurar X, Y ou Z trava o eixo; o losango é o pole |
| Mover o quadril | ferramenta "Cinemática inversa": arrastar o losango roxo; Shift = corpo inteiro |
| Cancelar um arrasto | Esc |
| Órbita / pan / zoom | MMB, RMB ou Alt+LMB / Shift+MMB / roda |
| Espelhar pose | M |
| Copiar / colar / colar espelhado | Ctrl+C / Ctrl+V / Ctrl+Shift+V |
| Selecionar tudo | Ctrl+A |
| Resetar seleção / tudo | Alt+R / Alt+Shift+R |
| Enquadrar | F ou botão |
| Desfazer / refazer | Ctrl+Z / Ctrl+Y ou Ctrl+Shift+Z |

Ferramentas do shelf: **Estrutura óssea** (FK, árvore, painel do osso), **Cinemática inversa** (membros, pés, quadril, olhar, equilíbrio, coluna),
**Poses do manequim** (presets de corpo, mãos, Biblioteca de poses).

## 3. Modelo de dados e API (para a Animação e outros consumidores)

### 3.1 No projeto

```text
ProjectState.character.pose : PoseState        // #[serde(default, skip_serializing_if = "PoseState::is_rest")]
  rotations        : BTreeMap<String, [f32;4]>  // nome canônico do osso -> quaternion XYZW (delta local sobre o repouso); só ossos != identidade
  root_translation : [f32;3]                    // move o personagem inteiro
  hips_translation : [f32;3]                    // move o corpo a partir do quadril
  hands            : { left, right : [f32;5] }  // curvatura 0..1 (polegar, indicador, médio, anelar, mínimo)
```

**Compatibilidade:** em repouso o campo não é escrito, então o JSON canônico, os fixtures (`npm run fixtures:check`) e o `render_contract` não mudam;
projetos antigos carregam como repouso; `sanitize()` descarta ossos desconhecidos/valores não finitos; `validate()` recusa o resto.

Nomes de osso (ordem fixa, = paleta de skinning de 24 ossos): `Root, Hips, Pelvis, Spine, Chest, UpperChest, Neck, Head, LeftClavicle, LeftShoulder,
LeftElbow, LeftWrist, RightClavicle, RightShoulder, RightElbow, RightWrist, LeftThigh, LeftKnee, LeftAnkle, LeftToes, RightThigh, RightKnee, RightAnkle, RightToes`
(`anigo_core::pose::BONE_NAMES`/`bone::*`; `POSE_BONE_NAMES` no TS, com teste de drift). A esquerda do personagem é **+X**; ele olha para **+Z**.

### 3.2 Comando

`Command::SetPose { bones: Vec<PoseBoneEntry{bone, rotation}>, root_translation?, hips_translation?, hands?, op }` — cada entrada traz a rotação **final** do
osso (identidade = repouso), portanto o inverso é exato e o comando é idempotente. Escopo `Deformation` (não invalida a geometria base; a paleta é avaliada).
`op` (`PoseOp`: edit, mirror, reset, apply, paste, preset, ik, gaze, hand, ground) só rotula o histórico. TS: `{ kind: "set_pose", bones?, root_translation?, … }`.

Mirror, reset, aplicar ativo, colar, preset, IK e olhar **não são comandos**: são *planejadores puros* que devolvem a pose desejada;
`anigo_core::pose::plan_pose_change(&from, &to, op) -> Option<Command>` transforma a diferença em UM `SetPose` (só os ossos que mudaram).

### 3.3 Arrasto ao vivo (sem poluir o histórico)

```rust
let mut g = GestureTracker::begin(session.pose().clone(), PoseOp::Edit);   // anigo-studio::posing
session.begin_preview();
loop { let target = /* pose desejada deste quadro, calculada a partir de g.base */;
       if let Some(cmd) = g.step(session.pose(), target) { session.preview(cmd)?; } }   // diff contra a pose AO VIVO
match g.finish() { Some(cmd) => session.commit_preview(cmd)?, None => session.cancel_preview() }  // UM passo de undo
```

### 3.4 Avaliar uma pose (keyframes)

```rust
let rig = session.rig()?;                          // PoseRig ajustado à malha deformada (cache)
let pose = PoseState::blend(&key_a, &key_b, t);    // slerp por osso + lerp de translação/dedos; t pode extrapolar
let palette = rig.palette_floats(&pose);           // 24x16 floats prontos para Scene.skin.palette
let joints = rig.joint_positions(&pose);           // posições de mundo das juntas
```

Úteis para a Animação: `PoseState::{blend, masked, overlay, additive, differing_bones, mirrored}`, `PoseAsset::{from_mask, apply_to}`,
`PoseRig::{skin_palette, joint_positions, world_rotations, grounded, lowest_contact_y, center_of_mass}`, `anigo_ik::pose_ik::*` e
`anigo_core::pose::{clamp_pose, bone_limit}` para limitar uma pose interpolada. **Auto-key:** o Posing não grava keyframes; o gancho é
o `CommandOutcome` de cada `SetPose` (escopo `Deformation`, `affected` = ossos tocados) — a Animação pode observá-lo para gravar a pose atual.
A pose NÃO entra no payload de snapshot do TS ainda (`snapshot.skin` continua identidade); o caminho nativo usa `Scene.skin`.

### 3.5 Rig ajustado à malha (por que não é o esqueleto T)

O `BondSyncManager` está em T-pose e é mais baixo que a malha canônica; a malha tem os braços **pendurados**. O `PoseRig` guarda posições de bind dos 24 ossos
derivadas das constantes do gerador da malha (`Mesh::create_canonical_base`), interpoladas por gênero e ajustadas à malha deformada (cada junta é arrastada pelos
deslocamentos dos 24 vértices mais próximos entre a malha de referência e a final: proporções e morphs). Rotações de bind = identidade: o delta de rotação
é lido nos eixos do mundo quando os pais estão em repouso (X lateral, Y vertical, Z frente). Paleta: `skin[i] = skin[pai] · T(b_i)·R_i·T(−b_i)`.

### 3.6 Ativo de pose (`.anigopose.json`)

```json
{ "format": "anigo.pose", "version": 1, "name": "Saudação",
  "bones": ["LeftShoulder", "LeftElbow"],            // máscara; vazio = todos
  "pose": { "rotations": { "LeftElbow": [x,y,z,w] }, "hands": { … } },
  "rig_hint": { "head_height": 1.57, "hips_height": 0.9 }, "tags": [] }
```

`anigo-assets`: `Format::AnigoPose` (extensão `.anigopose.json`), análise valida o formato (limite de 256 KiB, nomes de osso, finitude), ativo `AssetKind::Pose`,
miniatura CPU de boneco de palitos (`PoseStickRenderer`) e, no studio, miniatura 3D cel do personagem posado (`CelThumbnailRenderer::render_pose`).
Aplicar = por nome de osso, só os ossos da máscara (e opcionalmente só os selecionados), com espelhar e mistura.

## 4. Arquitetura

```text
anigo-core     pose.rs (PoseState, PoseRig, limites, PoseAsset, presets, finger_curl_mesh) + Command::SetPose            [T]
anigo-ik       pose_ik.rs (solve_limb, fabrik/solve_chain, look_at_head, lock_feet)                                      [T]
anigo-session  CoreSession::{pose, rig, pose_palette}; sync_scene injeta paleta + dedos                                  [T]
anigo-assets   Format::AnigoPose, análise, PoseStickRenderer                                                              [T]
anigo-studio   posing/        módulos puros e testados: edit, gesture, pick, limits_vis
               app/posing_ui/ UI: mod (estado/ações), viewport (entrada, gizmo, overlays, render), panels, script (cenários)
               library_ui/    pose_bridge.rs (salvar/listar ativos de pose) + miniatura 3D de pose em cel_thumb.rs
               i18n_posing.rs textos pt/en (`pose.*`)
```

## 5. Capturas e smoke

Cenários: `posing-vazio`, `posing-fk`, `posing-limites`, `posing-ik`, `posing-presets`, `posing-mao`, `posing-gaze`, `posing-biblioteca`
(`anigo-studio --workspace posing --scenario <nome> --screenshot x.png`). Referências em `docs/native/img/posing_*.png`, geradas por
`scripts/native_screenshots.sh`; a trilha `native` do CI gera as mesmas como artefato e roda `scripts/native_posing_smoke.sh`.
`ANIGO_POSING_CAM="tx,ty,tz,distância,yaw,pitch"` sobrescreve a câmera de um cenário (inspeção de detalhes).

`scripts/native_posing_smoke.sh` (xdotool, **somente dentro de um Xvfb próprio** `:93`; nunca toca o display do usuário; estado do egui em `XDG_DATA_HOME` temporário;
o app imprime `posetrace ...` com `ANIGO_TRACE_POSING=1`):
1. clique no ombro E → selecionado e ativo;
2. arrasto do anel Z do gizmo → pose fora do repouso, **exatamente +1 passo de undo**, nada no histórico durante o arrasto, viewport muda;
3. Ctrl+Z → repouso, **screenshot da viewport idêntico (0 px)**; Ctrl+Y → idêntico ao girado;
4. Esc no meio do arrasto → pose igual, sem passo novo (e soltar o botão depois não cria passo);
5. caixa de seleção → joelho E + tornozelo E;
6. cenário `posing-ik`: arrasto do efetuador do braço E → o punho se move, os tornozelos **não**, +1 passo; Ctrl+Z restaura;
7. arrasto do handle do quadril com "travar pés" → o quadril desce e os tornozelos ficam (tolerância 3 mm), +1 passo.

Achados de entrada real: `import -window root` (screenshot) tira o foco do app e o egui passa a ignorar o teclado — o smoke refoca (`xdotool windowfocus`) antes de cada tecla.

## 6. Limites conhecidos e decisões

- **Dedos:** o esqueleto canônico (24 ossos, paleta congelada em 1536 B) não tem ossos de dedo. A curvatura vive no modelo/biblioteca e se vê por uma deformação **CPU** dos
  35 vértices de cada dedo da malha canônica (`finger_curl_mesh`, antes do skinning). Quando o rig ganhar ossos de dedo o mesmo dado passa a girar ossos.
- **Malha:** a malha canônica é feita de partes independentes (cabeça, pescoço, tronco, pelve, membros) com costuras sobrepostas; poses fortes (agachamento, ombro > 90°)
  mostram as emendas e o colapso de axila próprios do LBS com 2 influências. Não é o gerador de pesos nem a malha que este workspace pode mudar (regra do repo).
- **Limites** são swing-twist (sem gimbal) com valores por osso `[C]` repartidos para somar as amplitudes normativas `[V]` (ver pesquisa); o cotovelo tem folga lateral (±12°)
  por causa da inclinação de repouso do braço na malha; o IK dos membros alinha a dobradiça (joelho/cotovelo giram só em X) e a torção vai para o osso da raiz.
- **FK com vários ossos selecionados:** o gizmo gira o osso ATIVO; a seleção múltipla vale para copiar, resetar, espelhar, salvar na Biblioteca e aplicar só aos selecionados.
- **IK não é persistente:** o resultado é gravado como FK (como "Apply IK" do Blender); os toggles FK/IK, o pole e o alvo do olhar são estado de UI, não do projeto.
- **Equilíbrio** usa frações de massa tipo Dempster `[S/C]` e é só um indicador.
- A pose **não** vai no snapshot do TS (`snapshot.skin` fica identidade); exportação GLB posada não foi feita (`export_bundle` exporta o repouso).
- Preferências (perfil de limites, simetria, snap) não são persistidas entre execuções; a pose em si persiste com o projeto (`character_document`).
- O Posing posa o personagem **onde ele está** no projeto (transformação do nó do personagem aplicada ao overlay e aos handles).

## 7. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)

- **Sensação** do gizmo FK, do IK ao vivo e do foot lock (tamanho do gizmo, limiar de arrasto, velocidade) — só exercitado por xdotool com passos discretos.
- **Gosto visual**: cores/espessuras do esqueleto e dos cones, aparência dos pesos de skin em poses extremas (candy-wrapper), dedos curvados, cel "branco demais".
- **Valores de limites por osso e frações de massa** — `[C]/[S]` até um artista/biomecânico revisar.
- Desempenho com GPU real: a cada passo de arrasto a sessão reaplica os morphs na CPU e reenvia a malha (≈ ms em CPU; lavapipe esconde o custo de GPU).
- Windows/macOS (Ctrl vs Cmd: o código usa `Modifiers::COMMAND`).
- Aplicar um card de pose a partir do workspace **Biblioteca** (detalhes do card) e arrastar card para a viewport: não implementados; a pose só é aplicada pela lista "Biblioteca de poses" do painel do Posing.

## 8. Como rodar

```bash
cargo test -p anigo-core -p anigo-session -p anigo-ik -p anigo-assets -p anigo-studio
cargo run -p anigo-studio -- --workspace posing                                         # app
cargo run -p anigo-studio -- --workspace posing --scenario posing-ik --screenshot /tmp/p.png
bash scripts/native_posing_smoke.sh ./target/debug/anigo-studio /tmp/posing_smoke       # precisa de Xvfb, xdotool, ImageMagick
```
