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
| T-2.7 | Modificadores no inspector (pilha, subsurf/mirror/solidify…) e viewport avaliando a pilha | ⬜ | B |
| T-2.8 | Escultura no viewport (pincel com raycast, simetria, máscara) | ⬜ | B+H |
| T-2.9 | Editor de UV (painel 2D) e painel de Normais (transferência de proxy, sharp/seam) | ⬜ | B |
| T-2.10 | Workspace **Personagem** nativo (sliders → `Command` → `CoreSession` → cel preview) | ⬜ | B |
| T-2.11 | Workspaces Posing / Shading / Iluminação / Cenário / Animação / Render nativos | ⬜ | B+H |
| T-2.12 | Layout dockável (egui_tiles) persistido; atalhos configuráveis; menus | ⬜ | H |
| T-2.13 | Servidor local para `anigo-mcp` falar com o app nativo (loopback + token) | ⬜ | A |
| T-2.14 | Aposentar `src/` Svelte e `webgpu_renderer.ts` após paridade | ⬜ | A |

## Trilha 3 — Motor Cel (anime NPR) — ver `CEL_ENGINE_FASE1.md` quando concluído
| ID | Tarefa | Estado | Val. |
|---|---|---|---|
| T-3.1 | Corrigir `vs_dof` degenerado; DoF com profundidade 1× | ✅ (profundidade) / ver relatório (triângulo) | A+B |
| T-3.2 | Rampa 2D por material (ou remover amostra morta) | ver relatório | B |
| T-3.3 | Contorno: normais suavizadas, largura por distância/FOV + taper, cor por material | ver relatório | B+H |
| T-3.4 | Face SDF espelhado + cor de sombra do material | ver relatório | B+H |
| T-3.5 | Matemática NPR pura em Rust com goldens | ver relatório | A |
| T-3.6 | Passe de bordas (Roberts/Sobel em depth+normal+ID) para vincos | ⬜ | B+H |
| T-3.7 | HDR + tonemap neutro no passe final (cores chapadas sem desvio de matiz do ACES) | ⬜ | H |
| T-3.8 | Shadow map de 1 luz + sombra do cabelo no rosto | ⬜ | B+H |
| T-3.9 | Angel ring estável por tangente do fio; hair-through (olho/sobrancelha) | ⬜ | H |
| T-3.10 | Bloom estilizado, LUT, grain; AOVs para compositing | ⬜ | B+H |

## Trilha 4 — Qualidade/entrega
| ID | Tarefa | Estado |
|---|---|---|
| T-4.1 | `VALIDACAO_LOCAL.md` com checklist para o chat local | ✅ |
| T-4.2 | Perfil dev otimizado p/ deps (wgpu/naga/image) e medição de tempo de build no Windows | ⬜ |
| T-4.3 | Spike `dx serve --hotpatch` (Subsecond) no `anigo-studio` | ⬜ (precisa do seu PC) |
