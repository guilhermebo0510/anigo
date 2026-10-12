# Roadmap de tarefas — migração nativa + Modelagem + motor Cel

Legenda de estado: ✅ feito e validado nesta sessão · 🟡 feito, falta validação local/humana · ⬜ a fazer.
Validação: **A** automática (CI) · **B** render em software (lavapipe/Xvfb, screenshot inspecionado) · **H** humana/local (ver `VALIDACAO_LOCAL.md`).

> Descoberta de ambiente que mudou o plano: no sandbox de nuvem é possível rodar **Vulkan por software (lavapipe)** e
> **o app inteiro sob Xvfb** com `--screenshot`. Logo, boa parte da "validação visual" foi feita aqui (B); o que sobra
> para o seu PC é gosto, desempenho em GPU real, entrada nativa (mouse/teclado/IME), Windows/MSVC e DPI.

## Trilha 0 — Fundação
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-0.1 | Pesquisa (UI Rust, C/C++ vs Rust, cel shading) → `docs/native/pesquisa/*` | ✅ | — |
| T-0.2 | ADR da arquitetura nativa → `ARQUITETURA_NATIVA.md` | ✅ | — |
| T-0.3 | wgpu 24 → 30 no workspace; hashes de frame idênticos antes/depois | ✅ | A+B |
| T-0.4 | `HeadlessRenderer::{from_device, encode_scene, render_to_texture}` + cache de alvos (viewport e export no mesmo caminho) | ✅ | A+B |
| T-0.5 | Validação naga de todos os `.wgsl` em `cargo test` | ✅ | A |
| T-0.6 | Crate `anigo-session` (CoreSession compartilhada Tauri ⇄ nativo) | ✅ | A |
| T-0.7 | CI: job nativo (Xvfb + lavapipe: testes + screenshots como artefato) | ✅ | A |

## Trilha 1 — Motor de modelagem (`anigo-modeling`, Rust puro)
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-1.1 | EditMesh n-gon, topologia, seleção, primitivas, normais por corner, ponte com `anigo_core::Mesh`, undo | ✅ | A |
| T-1.2 | Operadores de edit mode (extrude, inset, bevel, loop cut, bridge, bisect, spin) | ver relatório do merge | A |
| T-1.3 | Operadores (merge, delete, dissolve, fill, triangulate, flip, smooth, transform+proporcional, misc) | ver relatório do merge | A |
| T-1.4 | Catmull-Clark com creases/UV + modificadores (mirror, solidify, array, weld, edge split, displace, decimate) + pilha serializável com cache | ✅ | A |
| T-1.5 | BVH + escultura (pincéis, simetria, máscara, automask, dyntopo, voxel remesh) | ver relatório do merge | A |
| T-1.6 | Normais anime (transferência de proxy, normais suavizadas de contorno), atributos NPR, pesos de skin, shape keys, curvas/hair cards | ✅ | A (+H p/ gosto) |
| T-1.7 | UV: costuras, LSCM(+ARAP), pack, projeções, métricas | ✅ | A (+H) |
| T-1.8 | `anigo-native`: meshopt (decimate/LOD), ufbx (FBX), xatlas (auto-atlas) — C/C++ vendorizado | ✅ | A (+H no Windows) |
| T-1.9 | Booleanas (`boolmesh` atrás de trait `BooleanBackend`), voxel remesh de alta qualidade, QuadriFlow (C++) | ⬜ | A |
| T-1.10 | Integração ao ProjectState: `Command::ReplaceAuthoredMesh` + asset store + contratos TS/Rust | ⬜ | A |

## Trilha 2 — App nativo (`anigo-studio`)
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-2.1 | Shell: 9 workspaces, tema, i18n pt/en, tool shelf, inspector, status, sessão persistida, CLI de screenshot | ✅ | A+B |
| T-2.2 | Workbench wgpu (grid, sólido, arestas, pontos, MSAA) + hot reload de shader | ✅ | A+B |
| T-2.3 | Viewport: câmera orbital, picking, box select, seleção v/e/f, G/R/S modal com eixo/numérico/snap, gizmo | ✅ | A+B / H (sensação) |
| T-2.4 | Preview **Cel Shading** com o renderer canônico + overlays | ✅ | B |
| T-2.5 | Biblioteca (Asset Browser em grade, busca/categorias, medidas reais dos .glb) | ✅ | B |
| T-2.6 | Ligar operadores do motor ao Edit Mode (E, I, Ctrl+B, Ctrl+R, X, M, F…) + painel de operadores | ver relatório do merge | A+B |
| T-2.7 | Modificadores no inspector (pilha, subsurf/mirror/solidify…) e viewport avaliando a pilha | 🟡 | B |
| T-2.8 | Escultura no viewport (pincel com raycast, simetria, máscara) | 🟡 | B+H |
| T-2.9 | Editor de UV (painel 2D) e painel de Normais (transferência de proxy, sharp/seam) | 🟡 | B |
| T-2.10 | Workspace **Personagem** nativo (sliders → `Command` → `CoreSession` → cel preview) | 🟡 | B |
| T-2.11 | Workspaces Posing / Shading / Iluminação / Cenário / Animação / Render nativos — **Cenário (Fase 1 do UE5, ver `CENARIO_UI.md`): 🟡** viewport/fly, Content Drawer da Biblioteca com drag-and-drop, seleção BVH, gizmo W/E/R com snap, Outliner/Details/Histórico, câmera de cena, undo/redo via CoreSession; pendente: luzes extras (render_contract congelado), snap de vértice, validar sensação com GPU real. **Iluminação (`ILUMINACAO_UI.md`): 🟡** gizmo de direção arrastável (esfera + disco do sol), sol (az/el/cor/Kelvin/intensidade), ambiente céu/chão, tinta/dureza da sombra, rim, 7 rigs, luz presa à câmera, sombra projetada (T-3.8), tudo por comandos do núcleo com undo/redo e smoke xdotool; pendente: luzes extras (contrato), tonemap/HDR (T-3.7), gosto/sensação/GPU real. **Render (ver `RENDER_UI.md`): 🟡** câmera de render (câmera de cena, lente, near/far, guias, enquadrar), saída (presets A4 300 dpi/quadrinho/ilustração, aspecto bloqueado, supersampling, alfa, sRGB/Display P3, PNG 8/16/JPEG), passes e AOVs (contorno isolado, profundidade, máscara, ID, normais), preview com máscara/margens seguras, render REAL em thread de GPU com fila (cancelar/retomar), histórico com miniaturas, gancho de sequência; pendente: EXR/HDR/tonemap/bloom/grain (T-3.7/T-3.10), tiles >8192 px, integração da timeline, validar em GPU real. **Shading (ver `SHADING_UI.md`, pesquisa em `pesquisa/shading_materiais_anime.md`): 🟡** alvos de Personagem/Cenário/Modelagem com undo, parâmetros agrupados com reset, editor de gradiente da rampa 2D (por linha, renderer por material), contorno por material, face SDF com mapa e espelho, preview em shader ball + objeto + A/B, paleta (HSV/harmonias), presets `.anigomat.json` na Biblioteca com miniatura 3D; pendente: texturas por material no renderer, alpha/recorte, material-pai com overrides (`AddMaterial`), gosto/sensação com GPU real. **Posing (`POSING_UI.md`): 🟡** `Command::SetPose` + `PoseState`/`PoseRig` no núcleo (aditivo, TS em paridade), limites swing-twist, IK (two-bone/pole, FABRIK, look-at, foot lock), esqueleto/gizmo FK/handles IK na viewport, espelhar/copiar/colar, presets de corpo e mão, ativo `.anigopose.json` na Biblioteca, undo/redo, smoke xdotool; pendente: sensação com GPU real, dedos (deformação CPU, sem ossos de dedo), pose fora do snapshot TS/GLB, aplicar card pela Biblioteca. Animação: ⬜ | B+H |
| T-2.12 | Layout dockável (egui_tiles) persistido; atalhos configuráveis; menus | ⬜ | H |
| T-2.13 | Servidor local para `anigo-mcp` falar com o app nativo (loopback + token) | ⬜ | A |
| T-2.14 | Aposentar `src/` Svelte e `webgpu_renderer.ts` após paridade | ⬜ | A |
| T-2.15 | Base de personagem Gen-0 (`anigo-charbase`): mesh anime M/F só quads, VRM 55 ossos + twist/busto, pesos, 64 morphs esparsos, contrato de topologia, GLB/VRM em `assets/models_v2` (ver `PERSONAGEM_BASE.md`). Pendente: esculpir rosto/corpo, renderer a N ossos, migração do CoreSession | 🟡 | A+H |

## Trilha 3 — Motor Cel (anime NPR) — ver `CEL_ENGINE_FASE1.md` (Fase 1 concluída; padrões conservadores, afinar com artista)
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-3.1 | Corrigir `vs_dof` degenerado; DoF com profundidade 1× | ✅ | A+B |
| T-3.2 | Rampa 2D por material (ou remover amostra morta) | ✅ | B |
| T-3.3 | Contorno: normais suavizadas, largura por distância/FOV + taper, cor por material | 🟡 (gosto/parâmetros) | B+H |
| T-3.4 | Face SDF espelhado + cor de sombra do material | 🟡 (gosto/parâmetros) | B+H |
| T-3.5 | Matemática NPR pura em Rust com goldens | ✅ (paridade WGSL↔Rust) | A |
| T-3.6 | Passe de bordas (Roberts/Sobel em depth+normal+ID) para vincos | ⬜ | B+H |
| T-3.7 | HDR + tonemap neutro no passe final (cores chapadas sem desvio de matiz do ACES) | ⬜ | H |
| T-3.8 | Shadow map de 1 luz + sombra do cabelo no rosto — aditivo, **desligado por padrão** (`StylizedLight.shadow`, passe de profundidade da luz, PCF + degrau estilizado, bindings 18–20 do cel; goldens intactos; ver `ILUMINACAO_UI.md` §3) | 🟡 (bias/dureza/gosto, GPU real e cabelo real) | B+H |
| T-3.9 | Angel ring estável por tangente do fio; hair-through (olho/sobrancelha) | ⬜ | H |
| T-3.10 | Bloom estilizado, LUT, grain; AOVs para compositing | ⬜ | B+H |

## Trilha 4 — Qualidade/entrega
| ID | Tarefa | Estado |
|---|---|---|
| T-4.1 | `VALIDACAO_LOCAL.md` com checklist para o chat local | ✅ |
| T-4.2 | Perfil dev otimizado p/ deps (wgpu/naga/image) e medição de tempo de build no Windows | ⬜ |
| T-4.3 | Spike `dx serve --hotpatch` (Subsecond) no `anigo-studio` | ⬜ (precisa do seu PC) |

## Trilha 5 — Biblioteca de ativos de alto nível (pedido do usuário)
Meta: dezenas de milhares de assets bem geridos, com miniaturas, inspirada no Content Browser/Fab (Unreal), Quixel Bridge/Megascans,
Blender Asset Browser e DAMs (Eagle/Billfish/Bridge). Formato interno próprio (modelagem ANIGO) **mais** importação de GLB/glTF
(e FBX/OBJ/VRM), com padrão de materiais interoperável (Blender/Unreal/Unity) escolhido para **anime** (ver pesquisa T-5.1).
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-5.0 | Biblioteca nativa básica: grade, busca, categorias, medidas reais dos .glb, "Editar na Modelagem", exportar objeto autoral p/ GLB | ✅ (MVP) — **substituído** pela UI de T-5.7 sobre `anigo-assets` (o scan antigo virou local ligado; "Editar" e "Exportar" continuam, agora via `Library`) | B |
| T-5.1 | Pesquisa dos líderes (UE Asset Registry/DDC, Megascans/Bridge/Fab, Blender Asset Libraries, DAMs) + formatos de material (glTF PBR, VRM/MToon, MaterialX, OpenPBR, UsdPreviewSurface) → `docs/native/pesquisa/biblioteca_assets.md` | em andamento | — |
| T-5.2 | Crate `anigo-assets`: registry SQLite (WAL + FTS5, migrações versionadas), modelo de dados (asset, versão, tag hierárquica, coleção manual/inteligente, localização, dependências) | 🟡 backend pronto e testado (A); UI e validação local pendentes — ver `BIBLIOTECA_BACKEND.md` | A |
| T-5.3 | Gestão de **locais**: biblioteca gerenciada (cópia interna) vs ligada (referência), múltiplas raízes, mover/relink de ausentes, scan incremental (mtime+tamanho+hash), watcher | 🟡 backend pronto e testado (A); watcher validado só no inotify do sandbox | A |
| T-5.4 | Pipeline de **import** em background (progresso/cancelar/retomar): GLB/glTF completo (cenas, hierarquia, skin, animações, materiais, KHR_*), FBX (ufbx), OBJ, VRM; dedup por hash de conteúdo (blake3); limites de segurança | 🟡 GLB/glTF/VRM/OBJ/FBX(ufbx)/imagens; sem Draco/meshopt/KTX2; validado só com arquivos sintéticos | A |
| T-5.5 | **Material canônico ANIGO** (toon-first, compatível com glTF PBR + VRMC_materials_mtoon) + conversores de import/export | 🟡 formato + conversores (glTF PBR, MToon, MTL, FBX) + `to_stylized`; falta B (render) e arquivos reais VRoid | A+B |
| T-5.6 | **Miniaturas**: geração em lote com o renderer cel offscreen (3D) e `image` (texturas), tamanhos 64/128/256/512, cache em disco por hash + LRU, invalidação | 🟡 parte CPU no backend; **render 3D cel (GLB/VRM/OBJ/FBX) no studio** (`library_ui/cel_thumb.rs`, fila serial, fallback sem GPU, lavapipe/Xvfb); falta validar em GPU real, texturas/multi-material nas miniaturas | A+B |
| T-5.7 | UI **grade virtualizada** (10⁴–10⁵ itens): grade/lista, zoom de cards, busca FTS com operadores, filtros facetados, tags, coleções, favoritos/rating, painel de metadados, preview 3D, multi-seleção, arrastar p/ cena | 🟡 implementada e testada no sandbox (ver `BIBLIOTECA_UI.md`); arrastar p/ viewport só como gancho (`AssetDragPayload`); falta validar rolagem em GPU real e gosto visual (H) | B+H |
| T-5.8 | **Gestão**: renomear/mover/duplicar, excluir com lixeira (restaurar/purge), visualizador de referências/dependências, duplicados, ausentes, verificação de integridade, uso de disco, limpeza de cache | 🟡 UI pronta (lixeira, restaurar/purge com confirmação, duplicar, dependências/usado-por, duplicados com resolução, ausentes/relink em lote, verify, rebuild, uso de disco, limpar cache); falta uso com bibliotecas reais | A+B |
| T-5.9 | Pacotes `.anigopack` (zip: asset + texturas + metadata + thumbnails) e exportação GLB/VRM | ⬜ | A |
| T-5.10 | Benchmark de escala: 50 k assets (índice, busca < 50 ms, scroll 60 FPS, memória) | 🟡 backend medido (50 k/200 k) em `BIBLIOTECA_BENCH.md`; **CPU da UI por quadro** com 50 k medida em `BIBLIOTECA_UI.md` (headless); scroll 60 FPS em GPU real continua (H) | A+H |
| T-5.11 | C/C++ onde compensar: KTX2/BasisU (basis_universal), LODs de preview (meshopt já em `anigo-native`) | ⬜ | A |
| T-5.12 | Interface `AssetSource` para fontes remotas (estilo Fab/Sketchfab) — só o contrato | ⬜ | A |
