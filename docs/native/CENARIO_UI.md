# Workspace Cenário — UI nativa (`anigo-studio`)

**Estado:** Fase 1 do §11.1 de [`pesquisa/ue5_level_editor.md`](pesquisa/ue5_level_editor.md) implementada e validada no sandbox
(Xvfb + Vulkan por software/lavapipe, sem GPU real, sem mouse/teclado reais de um artista).
Cobre a parte de Cenário de T-2.11. A modelagem continua com atalhos estilo Blender; **só o Cenário usa o esquema do UE**.

Legenda: **[T]** coberto por teste automático que rodou; **[S]** visto em screenshot ou em entrada real (xdotool) no Xvfb; **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais (sem mockups): a cena é a `SceneState` canônica do `anigo-core`, editada por `Command` através do `CoreSession`;
os ativos vêm de `anigo_assets::Library`.

| Área | O que faz | Validação |
|---|---|---|
| **Modelo de dados** (`anigo-core/src/level.rs`, `project.rs`, `command.rs`) | `NodeSlot` ganhou `locked`, `tags`, `folder`, `layer`, `source` (`LibraryAsset{uri anigo://library/<uuid>, revision}` / `Primitive` / `Light` / `Camera{focal, sensor}` / `Card`) e `overrides` (cor-base, sombra, contorno). Campos aditivos e omitidos quando vazios: o JSON canônico e os fixtures **não mudam**. Novos comandos (antes de `Batch`, mesma ordem em Rust e TS): `SetNodesTransform`, `DuplicateNodes` (com remapeamento de ids), `SetNodeProps`, `ReplaceNodesAsset`, `PatchAssets`; `AddNode` aceita `attrs`. Planejadores puros: duplicar, apagar, ocultar, bloquear, reparentar mantendo o mundo, agrupar, desagrupar, copiar/colar (entre projetos), colocar ativo/primitiva/câmera. | [T] inversos exatos, fuzz de undo/redo, copiar/colar entre projetos, JSON aditivo, contrato TS (`npm test`, `fixtures:check`) |
| **Ponte Biblioteca→cena** | `anigo://library/<uuid>` resolvido por `Library::row` + `resolve_files`; GLB/OBJ/FBX carregados em thread de fundo (`MeshCache`, chave `lib:<uuid>@<hash12>`), nó com `MeshRef`; `sanitize` repara o registro de ativos a partir de `source`. | [T] [S] |
| **Content Drawer** | Painel inferior do próprio workspace que reutiliza `library_ui` (busca, filtros, grade). Arrastar um card para a viewport (`dnd_release_payload::<AssetDragPayload>`) coloca o ativo; duplo clique coloca no centro. Ghost (caixa tracejada + nome) durante o arrasto, base do ativo no ponto atingido (compensação da caixa), alinhamento opcional à normal, **um único passo de undo** (inclusive o registro do ativo). | [S] xdotool arrastou um card real: +1 nó, 1 passo de undo |
| **Viewport** | Orbit (MMB / Alt+LMB), pan (Shift+MMB), zoom (roda), **fly** (RMB + WASD/QE, roda = velocidade), F foca a seleção, vistas ortográficas (Alt+G/H/J/K: perspectiva/frente/topo/direita), sombreamento Cel/Solid/Wire (Alt+4/3/2), grade métrica, vista de jogo (G), vista pela câmera da cena. Renderiza com `CelPreview` (renderer canônico) + overlay do `Workbench` com profundidade. | [S] |
| **Seleção** | Raycast com **BVH por ativo** (espaço local) + varredura de AABBs de mundo por instância; clique, Ctrl/Shift (alterna/soma), marquee, Ctrl+A, inverter (Ctrl+I), filhos, mesmo ativo (Shift+E), contorno da seleção (OBB), ocultar/bloquear. | [T] picking, marquee, BVH; [S] |
| **Gizmo** | W/E/R (e Q/Espaço): eixo, plano, uniforme; Mundo/Local (Ctrl+\`); tamanho constante em pixels; pivô de multisseleção; **Alt+arrastar duplica**; o arrasto é estado transitório (`preview`) e vira **um** comando ao soltar (`commit_preview`); Esc cancela. | [T] algoritmos; [S] arrasto real com xdotool |
| **Snap** | Barra superior: grade de posição / rotação / escala (valores e liga/desliga), absoluto × incremental, Ctrl inverte durante o arrasto, **End** cola no chão, **Ctrl+End** origem na grade, **Z** surface snapping (com alinhamento à normal opcional), `[` `]` e Shift+`[` `]` mudam os passos. **Snap de vértice (V) é Fase 2** (só avisa). | [T] algoritmos puros (`scene/snap.rs`); [S] movimento em múltiplos de 0,5 m com entrada real |
| **Outliner** | Árvore virtualizada com pastas lógicas, olho e cadeado por linha, busca com a sintaxe do UE (`-x`, `+x`, `"frase"`), renomear (duplo clique / F2), arrastar para reparentar (mantém o mundo) ou mover para pasta, F foca, menu de contexto. | [T] filtro/linhas; [S] |
| **Details** | Nome, visível/bloqueado, Transform numérico (multivalor, reset por linha, trava de proporção), ativo da Biblioteca (revisão, triângulos, "Substituir", "Atualizar" quando a revisão ficou velha), material por instância (overrides), tags, pasta, lente (focal mm + sensor + FOV) para câmeras de cena, e — sem seleção — luz principal estilizada (azimute, elevação, cor, intensidade, ambiente) e câmera de render. | [S] |
| **Place Actors** | Cubo, Esfera, Plano, Cilindro, Cone, Câmera de cena, Grupo (clicar ou arrastar). | [S] |
| **Câmera de cena** | Nó `Camera{focal_mm, sensor}`; "Usar como câmera de render" ↔ `SetCamera`; "Alinhar à vista"; guias de composição (terços, centro, ação/título seguros) desenhadas na vista pela câmera. | [T] matemática da lente; [S] |
| **Undo/Redo e documento** | Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z + botões do topo, **painel Histórico com salto** (`jump_to_depth`), Ctrl+C/X/V (entre cenas), salvar/abrir cena (diálogo no painel), autosave/restauração via sessão (`character_document`). | [T] roteamento, salto; [S] Ctrl+Z/Y reais |

## 2. Atalhos (esquema do UE, só no Cenário)

| Ação | Atalho |
|---|---|
| Selecionar / Mover / Rotacionar / Escalar | Q / W / E / R (Espaço alterna) |
| Mundo ↔ Local | Ctrl+\` |
| Orbit / Pan / Zoom | MMB ou Alt+LMB / Shift+MMB / roda |
| Fly | segurar RMB + W A S D Q E; roda = velocidade; Shift acelera |
| Focar seleção | F |
| Vistas | Alt+G perspectiva, Alt+H frente, Alt+J topo, Alt+K direita |
| Sombreamento | Alt+4 Cel, Alt+3 Solid, Alt+2 Wire |
| Vista de jogo | G |
| Duplicar | Ctrl+W ou Alt+arrastar o gizmo |
| Apagar | Del |
| Ocultar / mostrar tudo | H / Ctrl+H |
| Selecionar tudo / inverter / mesmo ativo | Ctrl+A / Ctrl+I / Shift+E |
| Agrupar / desagrupar | Ctrl+G / Shift+G |
| Copiar / recortar / colar | Ctrl+C / Ctrl+X / Ctrl+V |
| Desfazer / refazer | Ctrl+Z / Ctrl+Y ou Ctrl+Shift+Z |
| Renomear | F2 (ou duplo clique no Outliner) |
| Colar no chão / origem na grade | End / Ctrl+End |
| Surface snapping | Z |
| Passo da grade / da rotação | `[` `]` / Shift+`[` `]` |
| Inverter snap durante o arrasto | segurar Ctrl |
| Content Drawer | Ctrl+Espaço |
| Limpar seleção / cancelar arrasto | Esc |

## 3. Arquitetura

```text
anigo-core   level.rs (planejadores puros) + Command/CommandHistory (inversos)         [T]
anigo-session CoreSession: preview / commit_preview / cancel_preview, jump_to_depth     [T]
anigo-studio  scene/        módulos puros e testados: camview, snap, gizmo, bvh, selection,
                            outliner, geometry (MeshCache assíncrono), runtime, placement
              app/scene_ui/ UI: mod (estado), actions (comandos), viewport (entrada, gizmo,
                            overlays, atalhos), panels (Outliner, Histórico, Details, Place Actors,
                            Content Drawer), script (cenários de captura)
              celview.rs    SceneOwner::Level + sync_level + render_if
              library_ui/   drawer_ui reaproveita a grade da Biblioteca
```

`render_if(size, dirty)` evita redesenhar quando nada mudou; `Workbench::instance` compartilha os buffers de um ativo entre instâncias.

## 4. Capturas e smoke

Cenários: `cenario-vazio`, `cenario-montado`, `cenario-snap`, `cenario-outliner`, `cenario-gizmo`, `cenario-camera`
(`anigo-studio --workspace cenario --scenario <nome> --screenshot x.png`). Referências em `docs/native/img/cenario_*.png`,
geradas por `scripts/native_screenshots.sh`; a trilha `native` do CI gera as mesmas como artefato e roda
`scripts/native_scene_smoke.sh`.

`scripts/native_scene_smoke.sh` (xdotool, **somente dentro de um Xvfb próprio** `:79`; nunca toca o display do usuário; estado do egui em `XDG_DATA_HOME` temporário):
1. espera o cenário `cenario-vazio` terminar (`ANIGO_TRACE_SCENE=1` imprime `scenetrace ...` por mudança de estado);
2. arrasta o primeiro card do Content Drawer até a viewport → exige +1 nó, selecionado, **exatamente +1 passo de undo**, geometria carregada;
3. espera o hover do eixo X do gizmo, arrasta com snap → exige deslocamento em múltiplos de 0,5 m e +1 passo de undo;
4. Ctrl+Z → estado e **screenshot da viewport idênticos** ao anterior ao movimento (0 px de diferença); Ctrl+Y → idênticos ao movido;
5. dois Ctrl+Z voltam ao estado inicial (o drop inteiro é um passo).

Modificadores no xdotool: sequência explícita `keydown Control_L; key z; keyup Control_L` (o winit pode entregar a tecla antes do modificador) e `windowfocus` (sem gerenciador de janelas o foco é explícito).
O smoke achou um defeito real que os testes unitários não pegavam: Ctrl+Z/Ctrl+Y do Cenário estavam roteados para a `ModelingSession`
em vez do `CoreSession` (corrigido em `route_undo_redo`/`can_undo_redo`, com teste).

## 5. Limites conhecidos e decisões

- **Renderer sem instancing nem cache de malha**: `CelPreview` reenvia os vértices de cada nó a cada quadro. Mitigação: `render_if` + buffers compartilhados do `Workbench`. Cenas com centenas de malhas pesadas podem ficar lentas; o orçamento de <5 ms de raycast vale para o picking, não para o render. **[N]** com GPU real.
- **Luzes extras não foram implementadas**: o `render_contract` está congelado (uma luz principal). "Luz direcional / ponto" aparece **desabilitada**, com tooltip explicando. A luz principal é editável (azimute, elevação, cor, intensidade, ambiente).
- **Snap de vértice (V)** é Fase 2; a tecla só mostra um aviso.
- A geometria dos nós da Biblioteca usa **sempre a revisão atual** do ativo; se a revisão mudou, o Details mostra "A Biblioteca tem a revisão N" e o botão de atualizar (`ReplaceNodesAsset`).
- **Pastas** valem só para nós na raiz (pastas lógicas, sem hierarquia de cena).
- O **histórico de undo não é restaurado** entre execuções; só o documento (via sessão).
- **Guias de composição** só aparecem na vista pela câmera da cena.
- O **ghost do arrasto** é uma caixa/contorno opaco (não uma malha translúcida).
- Dimensões dos ativos sintéticos de teste (toro grande demais perto do manequim) são do gerador de cenário, não do editor.

## 6. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)

- **Sensação** de fly, gizmo e snap (velocidades, tamanho do gizmo, limiar de arrasto, atrito do snap, Alt+arrastar) — só foi exercitado por xdotool com passos discretos.
- **Gosto visual**: o cel fica claro/"branco" demais com os materiais padrão dos nós; paleta do viewport e do gizmo.
- **Drag-and-drop real** (mouse físico, e arrastar arquivos do SO para o Content Drawer): validado só com eventos sintéticos do X no Xvfb.
- Desempenho com GPU real e cenas grandes (centenas de instâncias); lavapipe é muito mais lento e esconde problemas de banda.
- Windows/macOS (Ctrl vs Cmd: o código usa `Modifiers::COMMAND`, mas só Linux foi exercitado).
- Ativos reais de terceiros (VRoid/Blender/Sketchfab/Quixel) como cenário completo; FBX com texturas.
- Persistência entre execuções do layout dos painéis (o egui guarda larguras; um valor antigo muito largo persiste — apague `~/.local/share/anigo-studio/app.ron` se o painel direito parecer largo demais).

## 7. Como rodar

```bash
cargo test -p anigo-core -p anigo-session -p anigo-studio
cargo run -p anigo-studio -- --workspace cenario                    # app
cargo run -p anigo-studio -- --workspace cenario --scenario cenario-montado --screenshot /tmp/c.png
bash scripts/native_scene_smoke.sh ./target/debug/anigo-studio /tmp/scene_smoke   # precisa de Xvfb, xdotool, ImageMagick
```
