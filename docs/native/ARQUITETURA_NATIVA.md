# ANIGO Nativo — arquitetura de UI e viewport 100 % Rust (ADR-001)

**Status:** proposta técnica implementada parcialmente (esqueleto + motor); validação visual local pendente
(ver `VALIDACAO_LOCAL.md`). **Data:** out/2026. **Substitui parcialmente** a seção 1 de
`ARQUITETURA_CANONICA_ANIGO.md` (a camada "Tauri + Svelte" passa a ser transitória).

> Correção de premissa: o app atual **não** usa React. É Tauri 2 + **Svelte 5** + viewport WebGPU escrito
> em TypeScript (`src/components/viewport/webgpu_renderer.ts`, 3,2 k linhas), mais um renderer wgpu em Rust
> (`anigo-renderer`) usado só para render headless/exportação. Ou seja, hoje há **duas** implementações do
> mesmo pipeline (TS no viewport, Rust no headless), mantidas em paridade por contrato congelado
> (`render_contract_v1.json`) e testes de hash. A migração elimina essa duplicação.

## 1. Decisão

| Camada | Hoje | Alvo |
|---|---|---|
| UI | Svelte 5 (webview) | **egui 0.36** (Rust, imediato). Layout atual por painéis redimensionáveis; docking livre (`egui_tiles`) é a tarefa T-2.12 |
| Viewport | WebGPU em TypeScript | **wgpu 30 em Rust**, o MESMO `anigo-renderer` do headless (um único pipeline) |
| Modelagem | inexistente | **`anigo-modeling`** (Rust puro) + UI do workspace "Modelagem" |
| C/C++ | nenhum | **`anigo-native`**: apenas libs prontas e vendorizadas (meshoptimizer, ufbx, xatlas); resto em Rust |
| Shell/janela | Tauri | **winit via eframe** (Tauri fica como alternativa até paridade; ver §6) |
| Automação | `anigo-mcp` | mantém; passa a falar com o app nativo por IPC local (§7) |

### Por que egui (e não Slint / iced / Bevy / manter Tauri)
Resumo da pesquisa (`pesquisa/ui_rust_hot_reload.md`, fontes e versões lá):
* Único candidato que reúne **viewport wgpu compartilhando o mesmo `Device`**, **docking maduro** (egui_tiles/egui_dock), AccessKit e releases regulares.
* Slint: wgpu `unstable-*`, sem docking, licença com atribuição. iced: sem acessibilidade, wgpu parado. Bevy: exigiria trocar o renderer próprio. Dioxus-Native/Xilem/GPUI/Makepad/Floem: alfa, sem release ou sem wgpu.
* Tauri + wgpu nativo: **não existe** solução estável (camada nativa sempre sobre o DOM, flicker com transparência, input em duas camadas). Serve só de ponte.
* Custo aceito: egui quebra API 2–4×/ano (fixar versão, atualizar em janela planejada); sem menus nativos nem bidi; fonte com CJK precisa ser embutida.

### Por que C++ quase não aparece
A pesquisa de bibliotecas (`pesquisa/cpp_vs_rust_modelagem.md`) concluiu que, para o escopo anime, **Rust puro resolve agora**:
subdivisão, UV (LSCM), voxel remesh, escultura, BVH, normais. C/C++ entra onde é **barato e comprovado**:
`meshopt` (decimate/LOD), `ufbx` (import FBX), `xatlas` (auto-atlas) — as três vendorizadas, compilam só com `cc`.
Fica para "C++ depois" (atrás de feature/trait): OpenSubdiv GPU, Manifold (booleanas robustas — o `manifold-csg-sys`
faz `git clone` no build, então só roda no PC do usuário), QuadriFlow (retopo automática), OpenUSD.
Critério de entrada de uma lib C++: (1) ganho mensurável sobre Rust puro, (2) licença compatível, (3) wrapper seguro
sem ponteiro cru exposto, (4) feature desligável, (5) build documentado para MSVC. Ver `docs/native/CPP_FFI.md`.

## 2. Diagrama

```text
┌───────────────────────────── anigo-studio (bin, eframe/winit) ─────────────────────────────┐
│  Barra de workspaces (9) │ Tool shelf │  Viewport (textura wgpu)  │ Inspector │ Outliner   │
│  egui_tiles (layout persistido)        overlays de edit mode (egui painter / pipeline próprio)│
└──────────┬──────────────────────────────────────┬─────────────────────────────────────────┘
           │ comandos tipados                      │ SceneSnapshot / EditMesh evaluated
┌──────────▼───────────────┐             ┌─────────▼────────────────┐
│ anigo-core               │             │ anigo-modeling           │
│ ProjectState + Commands  │◄───────────►│ EditMesh, ops, modifiers │
│ (undo/redo, autosave)    │ MeshRef/    │ sculpt, normais, UV      │
└──────────┬───────────────┘ assets      └─────────┬────────────────┘
           │                                       │ to_core_mesh (72 B/vértice)
┌──────────▼───────────────────────────────────────▼──────┐   ┌───────────────────────┐
│ anigo-renderer (wgpu 30) — shaders WGSL canônicos        │   │ anigo-native (C/C++)  │
│ encode_scene(target) usado por viewport E headless/export│   │ meshopt · ufbx · xatlas│
└──────────────────────────────────────────────────────────┘   └───────────────────────┘
```

Invariantes herdadas (não mudam): `ProjectState` é a fonte única; toda alteração persistente é `Command`; shaders
canônicos em `crates/anigo-renderer/shaders`; viewport e render final usam o mesmo caminho.

## 3. Workspaces (9)

`Personagem · Modelagem · Posing · Shading · Iluminação · Cenário · Animação · Render · Biblioteca`

* **Personagem** (estilo VRoid): sliders sobre bases (já existe no core: morphs, somatótipo, proporções).
* **Modelagem** (novo, estilo Blender enxuto): Object / Edit / Sculpt / Normais / UV / Curvas-cabelo. Independente das demais; o resultado vira asset na Biblioteca.
* **Cenário** (estilo Unreal): coloca assets da Biblioteca na cena (outliner + gizmos + snapping).
* **Animação / Render**: timeline, câmeras, passes; consomem personagem + cenário.
* Workspaces são *vistas* sobre o mesmo `ProjectState`; não há cópia de dados entre elas.

## 4. Hot reload (fluxo de trabalho)

1. **Shaders/assets** (menos de 1 s, sem recompilar): `notify` observa `crates/anigo-renderer/shaders/*.wgsl`; em debug o app lê do disco
   (em release usa `include_str!` — hash do contrato continua válido); o shader novo é validado em *error scope* e só substitui o pipeline se passar (erro vira overlay).
2. **UI e lógica do app** (segundos, estado preservado): experimento com `dx serve --hotpatch` (Subsecond). É experimental → tratado como **spike**, não requisito; fallback é o item 3.
3. **Qualquer crate** : `bacon`/`cargo watch -x run` com perfil dev otimizado para deps (`opt-level=2` em wgpu/naga/image/glam) + **restauração de sessão** (`session.json`: projeto, câmera, workspace, seleção, layout) → todo reinício parece um hot reload.
4. Cloud/CI: o app roda sob `Xvfb` + Vulkan por software (lavapipe) com `--screenshot` e `--scenario <nome>`, gerando PNGs
   para revisão (job `native` do CI). `xdotool` dá entrada real de mouse/teclado no Xvfb para smoke tests.

### Dois renderers, um propósito cada
* **Workbench** (`anigo-studio/src/workbench`): ferramenta de edição — grid, sólido com luz de estúdio, arestas/pontos em quads de tela,
  MSAA. É o "Workbench" do Blender: não precisa ser fiel ao render final, precisa ser rápido e legível.
* **Cel preview** (`anigo-studio/src/celview.rs`): o **mesmo** `anigo-renderer` do render final/export (`render_to_texture` no `Device`
  do egui), com os overlays do Workbench por cima. A regra "o que se vê é o que se renderiza" continua valendo para o visual cel.

## 5. Plano de migração (strangler)

0. ✔ Pesquisa, ADR, motor de modelagem.
1. ✔ wgpu 24→30 + `encode_scene`/`render_to_texture` em alvo externo (hashes de frame idênticos).
2. ✔ `anigo-studio` casca: janela, 9 workspaces, tool shelf, inspector, viewport (Workbench + preview Cel), câmera orbit, sessão.
3. ✔/🟡 Workspace Modelagem: primitivas, seleção v/e/f, G/R/S, gizmo, ~45 operadores, edição proporcional, modificadores, normais, UV, escultura
   (ver `ROADMAP_TAREFAS.md` para o estado de cada item).
4. Paridade Personagem (sliders → `Command` do core → viewport).
5. Biblioteca/Cenário/Animação/Render nativos. Aposentar `src/` Svelte e `webgpu_renderer.ts` quando os testes de paridade passarem.

Enquanto 5 não termina, o app Tauri continua compilando e testado em CI (nenhum arquivo dele foi removido).

## 6. Se for preciso manter o Tauri como visualizador
Não recomendado (pesquisa §3). Se ainda assim desejado: renderizar `anigo-renderer` headless e enviar frames ao
canvas (latência ~100–300 ms, input difícil) — é o caminho que o app atual já suporta (`core_export_frame`).
Fluxo de hot reload do Tauri atual (Vite HMR) permanece funcionando para quem editar o Svelte.

## 7. Automação / MCP
`anigo-mcp` fala com o app Tauri (`bridge.rs`, porta 39090) **e**, desde a T-2.13, com o app nativo: `anigo-studio --bridge` sobe o servidor local do crate `anigo-bridge`
(loopback, porta efêmera, token por sessão em `bridge.json`) e aplica os pedidos na thread da UI pela mesma `CoreSession` (comandos tipados do core, undo/redo, screenshot, render).
Mantém a regra de que telemetria reflete valores reais. Detalhes, segurança e limites: [`MCP_BRIDGE_NATIVA.md`](MCP_BRIDGE_NATIVA.md).

## 8. Integração do modelador com o ProjectState
Malha autoral = `AssetEntry{kind: Mesh, uri: "anigo://authored/<id>"}` + payload `EditMesh` (JSON canônico).
Cada confirmação (sair do Edit Mode, fim de traço de escultura, aplicar modificador) emite
`Command::ReplaceAuthoredMesh{asset_id, before_hash, after_hash}`; os blobs ficam no log/asset store, o
undo troca o ponteiro de asset (O(1), sem refazer a operação). Dentro de uma sessão de edição o
undo fino é do `SnapshotHistory`/`PositionDelta` do modelador. A validação do comando checa hash e `EditMesh::validate`.
O renderer recebe `to_core_mesh(modifier_stack.evaluate(base))` — mesmo contrato de vértice de 72 B.
