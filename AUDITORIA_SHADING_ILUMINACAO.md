# AUDITORIA TÉCNICA COMPLETA — WORKSPACES **SHADING** + **ILUMINAÇÃO**
### ANIGO Studio · Relatório de Conformidade para Nível Comercial / Enterprise

| Campo | Valor |
| :--- | :--- |
| **Data da auditoria** | 2026-09-20 |
| **Branch auditada** | `arena/01a0c022-anigo` (base: `7ac1369`) |
| **Escopo** | Workspace **Shading** (ferramentas `cel_shader`, `rim`, `outline`, `palette`, `shader_ball`) e **Iluminação** (`sun`, `shadows`, `ambient`) — shader NPR cel-shading, contorno inverted hull, sistema de luz toon, paleta/hue-shift, rampa toon, integração viewport ↔ motor Rust ↔ MCP, persistência, undo/redo, configuração gráfica e testes |
| **Método** | Inspeção estática linha a linha dos 4 caminhos de shading; **parse binário real dos GLB canônicos** (leitura de accessors/bufferViews para provar o conteúdo dos canais de vertex color); **simulação numérica do shader em JS** (rampa, threshold, cor de sombra, saturação de intensidade, crossfade); **diff programático** entre as 4 cópias de shader; **diff da tabela de rampa TS × Rust**; execução real da suíte E2E existente (`node --test`); rastreamento de estado morto (grep de usos por parâmetro); leitura das specs `SPRINT_02`/`SPRINT_05`/`SPRINT_18` e das Regras Invioláveis |
| **Arquivos auditados** | `src/components/viewport/webgpu_renderer.ts` (2.548 linhas, sendo ~700 de shader inline), `src/App.svelte` (3.861), `src/components/viewport/Viewport.svelte` (506), `src/components/character/LightingControls.svelte` (389), `src/components/viewport/tactile.ts` (413), `src/components/settings/SettingsModal.svelte` (1.349), `shaders/*.wgsl` (3), `crates/anigo-renderer/shaders/*.wgsl` (3, cópia), `crates/anigo-renderer/src/headless.rs` (1.429), `crates/anigo-renderer/src/uniforms.rs` (133), `crates/anigo-core/src/scene.rs` (208), `crates/anigo-core/src/math.rs` (176), `crates/anigo-core/src/mesh.rs` (1.605), `src-tauri/src/main.rs` (428), `src-tauri/src/bridge.rs` (718), `crates/anigo-mcp/src/main.rs` (1.418), `src/services/history_service.ts` (196), `src/services/autosave_service.ts` (108), `tests/e2e/workspace_pipeline.test.ts` (1.133), `public/models/*.glb` (2 × 377 kB) |
| **Veredito** | 🔴 **NÃO APROVADO PARA NÍVEL COMERCIAL/ENTERPRISE** — 11 defeitos críticos (P0), 15 altos (P1), 16 médios (P2) e 13 melhorias de estado da arte (P3). O pipeline de shading **renderiza hoje uma imagem matematicamente diferente da que o artista configura**, e a imagem exportada é **diferente da imagem do viewport** |

---

## 1. SUMÁRIO EXECUTIVO

A workspace de Shading/Iluminação tem aparência de produto profissional: 5 ferramentas de shading, 3 de iluminação, 24 controles, 8 presets, telemetria ao vivo, ponte MCP e um shader NPR com rampa toon em textura, hue-shift matemático, especular anisotrópico com jitter e rim Fresnel. **A arquitetura de apresentação é boa; a matemática e o plumbing por trás dela não são.**

Três classes de defeito dominam o quadro:

1. **O shader não recebe os dados que o artista autora.** O loader TypeScript procura o atributo glTF padrão `COLOR_0`, mas os GLB canônicos gravam o canal de shading como `_ANIGO_COLOR` (verificado por parse binário: 4.070 vértices com R∈{0,95;1,0}, G∈{0,48;0,50}, B∈{1,0;1,2}, A∈{0,85;1,0}). O loader cai no fallback `r=g=b=a=1` — e como o canal G é lido pelo shader como *shadow shift* centrado em 0,5, **o limiar de sombra de toda a malha é deslocado em +0,15** (0,50 configurado → 0,65 efetivo, ≈27° de arco N·L). AO, máscara de contorno e máscara de especular/rim são igualmente descartadas. O motor Rust (`mesh.rs:1155`) lê `_ANIGO_COLOR` corretamente — ou seja, **os dois motores shadingam a mesma malha de forma diferente**.

2. **A cor que o artista escolhe não é a cor que aparece.** O mesmo hex (`shadowColorHex`) alimenta simultaneamente `material.shade_color` e `light.shadow_color`, e o shader multiplica os dois (`cel_shading.wgsl:160`). Resultado medido: a sombra escolhida `#9995be` é renderizada como `#262b44` — **28,5% da luminância selecionada**. Somado a isso, o slider "Intensidade" da luz (0,0–3,0) **satura em 1,02**: acima disso o shader normaliza pelo canal máximo e a luz deixa de clarear (os presets "Meio-Dia" 1,2 e "Golden Hour" 1,4 já nascem clipados). Dois controles centrais da workspace estão, na prática, quebrados.

3. **Existem quatro implementações do mesmo shader, e elas divergem.** WGSL em `shaders/` (usado pelo Rust), WGSL em `crates/anigo-renderer/shaders/` (cópia byte-a-byte menos uma quebra de linha), WGSL inline no `webgpu_renderer.ts` e GLSL inline no fallback WebGL2. Derivas medidas: o outline do viewport lê `params.z/w` (depth bias e opacidade) enquanto o arquivo `.wgsl` fixa `0.0003` e ignora a opacidade; o jitter especular usa `uv.x` no WGSL e `v_pos.x` no GLSL; a tabela da rampa toon difere entre TS e Rust em **81/256 texels (linha 2) e 76/256 (linha 3), com |Δ| até 127**; o export é `Rgba8UnormSrgb` e o canvas é `bgra8unorm`; a ordem dos passes é invertida; a cor de fundo é `(0,08;0,09;0,13)` no viewport e `(0,12;0,13;0,16)` no Rust. **O que se vê não é o que se exporta** — violação direta de WYSIWYG e da Regra Inviolável nº 2.

Além disso: **não existe anti-aliasing em nenhum backend** (a configuração "MSAA 4x" é decorativa e nunca é aplicada) — o que viola textualmente o *Definition of Done* da Sprint 02 ("transição nítida de anime **sem serrilhado**"); **dois controles da UI são mortos** (`rimColor`, `outlineSmoothness`); **a telemetria enviada ao MCP contém valores fabricados** (156 triângulos, câmera e luz fixas) competindo com a telemetria real no mesmo estado; **8+ parâmetros de shading não entram no undo/redo nem no arquivo de projeto**; e a suíte de testes (27 testes, todos verdes) **não executa uma única linha do renderer** — ela valida um harness-mock com clamps que não existem no aplicativo.

### 1.1 Contagem de achados

| Severidade | Qtd | Significado | Bloqueia release? |
| :--- | :--: | :--- | :---: |
| 🔴 **P0 — Crítico** | **11** | Imagem incorreta em runtime, controle morto/enganoso, quebra de WYSIWYG, perda de dados, tela preta silenciosa | **Sim** |
| 🟠 **P1 — Alto** | **15** | Ausência de gestão de cor, features centrais da spec inexistentes, duas fontes de verdade, sem tipagem estrita/CI, vazamento de recursos | **Sim** |
| 🟡 **P2 — Médio** | **16** | Artefatos de profundidade/blend, acoplamento de parâmetros, métricas enganosas, robustez do loader, performance | Não (corrigir no ciclo) |
| 🔵 **P3 — Melhoria** | **13** | Estado da arte NPR (HDR/tonemap, SDF facial, shadow map, light rigs, editor de rampa, passes de render) | Não (roadmap) |
| **Total** | **55** | | |

### 1.2 Os 11 showstoppers (P0)

| # | Defeito | Evidência |
| :-- | :--- | :--- |
| P0-01 | **Canal de shading `_ANIGO_COLOR` descartado** → AO, shadow-shift, máscara de contorno e de especular/rim perdidos; limiar de sombra deslocado em **+0,15** | `webgpu_renderer.ts:1725`, `1756-1768` × `mesh.rs:1155` × parse binário dos GLB |
| P0-02 | **Cor de sombra aplicada duas vezes** (`shade_color × light.shadow_color`, mesmo hex) → `#9995be` vira `#262b44` (28,5% da luminância) | `App.svelte:1225`, `1251` × `cel_shading.wgsl:160` |
| P0-03 | **Slider "Intensidade" da luz morto acima de 1,02** (normalização por canal máximo, sem HDR/tonemap); presets 1,2 e 1,4 clipados | `cel_shading.wgsl:165-170`, `LightingControls.svelte:124-140` |
| P0-04 | **4 cópias do shader com deriva comprovada** (depth bias/opacidade do outline, jitter, tabela de rampa, ordem de passes) | `shaders/*.wgsl` × `crates/anigo-renderer/shaders/*.wgsl` × `webgpu_renderer.ts:266-511` × `763-930` |
| P0-05 | **Export/headless ≠ viewport**: mesh proxy de caixas, câmera independente, sRGB, rampa diferente, spec color/softness/offset fixos, opacidade/bias de contorno ignorados, `shadow_saturation` inexistente | `main.rs:105-121`, `uniforms.rs:66,79`, `headless.rs:279,326,608,661-666,744-746` |
| P0-06 | **Fallback WebGL2 inatingível** após falha do WebGPU (mesmo canvas) **+ erros de shader/pipeline nunca observados** → tela preta silenciosa, sem diagnóstico | `webgpu_renderer.ts:211-262`, `755-756` |
| P0-07 | **Zero anti-aliasing** (MSAA/FXAA inexistentes); setting "MSAA 4x" nunca aplicada → viola DoD da Sprint 02 | `webgpu_renderer.ts:528-575` (pipelines sem `multisample` → `sampleCount = 1`), `App.svelte:681-699`, `SettingsModal.svelte:17,32,540-557` |
| P0-08 | **Telemetria fabricada** (156 triângulos, câmera/luz fixas) em fonte concorrente com a telemetria real, no mesmo `LiveWindowState` consumido pelo MCP | `App.svelte:1341-1366` × `Viewport.svelte:287-320` × `bridge.rs:119-146` |
| P0-09 | **Controles mortos na UI**: `rimColor` (shader usa `shadow_color`), `outlineSmoothness` (nunca lido), banda de 3 degraus inacessível | `App.svelte:346,2014-2018,2058-2068`, `webgpu_renderer.ts:179,1198`, `cel_shading.wgsl:209` |
| P0-10 | **Undo/Redo e Save/Autosave não preservam o shading** (8+ parâmetros ausentes) e gravam câmera com valor fixo | `history_service.ts:7-34`, `autosave_service.ts:1-29`, `App.svelte:643-680` |
| P0-11 | **Normais nunca recalculadas após deformação** → terminador de sombra e contorno inverted hull errados em qualquer morph/somatótipo | `webgpu_renderer.ts:1386-1672` (copia `v.normal` sem recalcular) |

### 1.3 Métricas duras levantadas

| Métrica | Valor medido | Alvo enterprise |
| :--- | :--- | :--- |
| Cópias do shader de shading | **4** (2 WGSL arquivo + 1 WGSL inline + 1 GLSL inline) | **1** fonte canônica |
| Deriva de shader medida | outline `params.z/w` só no TS; jitter `uv.x` × `v_pos.x`; rampa **81 e 76 texels** divergentes; ordem de passes invertida | 0 |
| Parâmetros de shading/lighting expostos na UI | **27** | 27 |
| …que chegam corretamente ao shader do viewport | **19** | 27 |
| …que chegam ao export/headless Rust | **12** | 27 |
| …que entram no arquivo de projeto / autosave | **17** | 27 |
| …que entram no undo/redo | **17** | 27 |
| …controláveis via MCP | **14** | 27 |
| Controles de UI **totalmente mortos** | **3** (`rimColor`, `outlineSmoothness`, `antiAliasing`) | 0 |
| Deslocamento do limiar de sombra por fallback de vertex color | **+0,150** (≈27° de arco N·L) | 0,000 |
| Erro de luminância da sombra escolhida × renderizada | **−71,5%** (`#9995be` → `#262b44`) | < 2% |
| Faixa útil do slider de intensidade da luz | **0,00 → 1,02** de 0,0 → 3,0 (66% morto) | 100% |
| Anti-aliasing ativo | **nenhum** (WebGPU `sampleCount=1`, Rust `MultisampleState::default()`) | MSAA 4x + pós-AA |
| Gestão de cor (sRGB↔linear, tonemap) | **inexistente** no viewport; sRGB só no export | pipeline completo |
| Campos públicos mutáveis no renderer (sem validação) | **46** | 0 (estado encapsulado + schema) |
| `any` explícitos em App/Viewport/renderer | **26** | 0 |
| `tsconfig.json` / typecheck / lint / CI | **inexistentes** (sem `.github`) | obrigatórios |
| Testes que exercitam o renderer/shading | **0** de 27 (harness-mock) | ≥ 60 (unit + parity + golden) |
| `unwrap()/expect()` em `mesh.rs` / `headless.rs` | **9 / 2** | 0 em caminho crítico |
| Erros silenciosos (`catch(_){}`, `.catch(()=>{})`) | **5 no renderer + 13 no App** | 0 sem log/diagnóstico |
| Vértices / triângulos da malha canônica | **4.070 vértices / 6.880 triângulos** (telemetria declara **156**) | valor real |
| Juntas autoradas no GLB × usadas | **18 juntas, weights soma 1,0** × **0 usadas** (sem skinning, sem matriz de modelo) | 100% |
| `SceneNode.transform` aplicado no render | **0** (não há matriz de modelo em nenhum dos 2 motores) | 100% |

---

## 2. ESCOPO, MÉTODO E LIMITAÇÕES

### 2.1 O que foi executado de fato (reprodutível)

```bash
# 1. Suíte E2E existente — 27 testes, 27 passam, 0 tocam no renderer
node --experimental-strip-types --test tests/e2e/workspace_pipeline.test.ts
#   → # tests 27 | # pass 27 | # fail 0 | duration 217ms

# 2. Parse binário real dos GLB canônicos (accessors, bufferViews, canais de cor)
node /tmp/glb_deep.cjs        # → 4.070 vértices, atributos: JOINTS_0, NORMAL, POSITION,
                              #   TEXCOORD_0, WEIGHTS_0, _ANIGO_COLOR  (NÃO existe COLOR_0)
                              #   _ANIGO_COLOR: R{0.95,1.0} G{0.48,0.50} B{1.0,1.2} A{0.85,1.0}
                              #   425 vértices com G=0.48 (cabeça); 2.626 com A=0.85
                              #   male y∈[0,1.795] | female y∈[0,1.655] (mesmas faixas de índice!)

# 3. Diff programático das 4 cópias de shader
diff shaders/cel_shading.wgsl crates/anigo-renderer/shaders/cel_shading.wgsl   # → idênticos (1 newline)
diff shaders/inverted_hull.wgsl crates/anigo-renderer/shaders/inverted_hull.wgsl
python3 (extração dos template literals do TS) + diff -u                        # → deriva em params.z/w

# 4. Simulação numérica do shader em JS (mesmas equações, mesmos defaults da UI)
node /tmp/verify.cjs          # → tabela de rampa TS×Rust, threshold efetivo, cor de sombra,
                              #   curva de intensidade, pesos do crossfade, piso de ambiente

# 5. Rastreamento de estado morto por parâmetro
grep -n "outlineSmoothness" src/components/viewport/webgpu_renderer.ts  # → 3 hits: só escrita, nunca lida
grep -rn "rimColor" src                                                 # → 3 hits: estado + input, nunca enviado
grep -rn "shadow_saturation" crates src-tauri                           # → 0 hits (não existe no Rust)
grep -rn "uploadSparseMorphData" src                                    # → 0 chamadores (código morto)
```

### 2.2 O que **não** foi possível validar neste ambiente (e precisa ser feito antes de qualquer "sprint concluída")

* **Frame real em GPU**: o sandbox não tem WebGPU/Vulkan nem toolchain Rust (`cargo` ausente), logo **nenhum frame foi renderizado nem inspecionado visualmente**. Pela Regra Inviolável nº 1, a validação visual via `anigo-mcp` (`anigo_render_frame` + `anigo_compare_baseline` + `anigo_screenshot_window`) é **obrigatória e ainda não existe para esta workspace** — não há baselines de shading versionadas (`baselines/` só tem PNGs das Sprints 01–03).
* **Compilação/typecheck**: sem `node_modules` e sem `tsconfig.json`, não foi possível rodar `vite build`/`svelte-check`. A ausência de typecheck é, ela própria, um achado (P1-14).
* **Comportamento de drivers específicos** (Dawn no WebView2 Windows, Metal/ANGLE): as falhas de `presentMode: "immediate"` e de `getContext` duplicado foram identificadas por leitura de especificação, não por execução.

> Todas as conclusões numéricas deste relatório derivam de **código lido + dados binários reais + simulação das mesmas equações**, e são reproduzíveis com os comandos acima.

---

## 3. INVENTÁRIO E ARQUITETURA ATUAL

### 3.1 Superfície de código da workspace

| Camada | Arquivo | Linhas | Papel |
| :--- | :--- | :--: | :--- |
| UI Shading | `src/App.svelte` (blocos `cel_shader`/`rim`/`outline`/`palette`/`shader_ball`, ~1810–2360) | ~550 | 20 controles + 12 botões de preset inline |
| UI Iluminação | `src/components/character/LightingControls.svelte` | 389 | 6 controles + 8 presets, `activeTool`-driven |
| Estado/orquestração | `src/App.svelte` (`updateLighting`, `updateMaterial`, `handleOutlineChange`, `handleShadowThresholdChange`, `handleToonSmoothnessChange`, `reportLiveTelemetry`, snapshots) | ~400 | única "camada de domínio" — não existe módulo de shading |
| Façade | `src/components/viewport/Viewport.svelte` | 506 | 20 métodos `export function` repassando ao renderer + 10 listeners de bridge |
| Renderer | `src/components/viewport/webgpu_renderer.ts` | 2.548 | WebGPU + WebGL2 + geradores de geometria + loader GLB + câmera + morphs |
| Shader (viewport) | inline no renderer (`celShaderCode`, `outlineShaderCode`, `morphComputeCode`, `vsCel/fsCel/vsOutline/fsOutline`) | ~700 | **cópias 3 e 4** |
| Shader (arquivo) | `shaders/cel_shading.wgsl`, `shaders/inverted_hull.wgsl`, `shaders/morph_sparse_compute.wgsl` | 397 | **não usados por ninguém no frontend** |
| Shader (Rust) | `crates/anigo-renderer/shaders/*.wgsl` | 398 | **cópia** usada por `include_str!` |
| Motor offscreen | `crates/anigo-renderer/src/headless.rs` | 1.429 | 2 funções de render quase duplicadas (589 e 895) |
| Contratos | `crates/anigo-renderer/src/uniforms.rs`, `crates/anigo-core/src/scene.rs` | 341 | `MaterialUniform`/`LightUniform`/`StylizedMaterial`/`StylizedLight` |
| IPC | `src-tauri/src/main.rs` (`set_light_params`, `set_material_toon_params`, `render_viewport_frame`) | 428 | comandos Tauri |
| Automação | `src-tauri/src/bridge.rs`, `crates/anigo-mcp/src/main.rs` | 2.136 | WebSocket + 32 ferramentas MCP |
| Persistência | `history_service.ts`, `autosave_service.ts` | 304 | undo/redo e autosave |
| Config gráfica | `SettingsModal.svelte` | 1.349 | FPS cap, DPI, vsync, **AA (morto)** |

### 3.2 Os quatro motores de shading que coexistem hoje

```
                 ┌──────────────────────── 1. VIEWPORT (interativo) ───────────────────────┐
 UI (sliders) ──▶ App.svelte state ──▶ Viewport.svelte façade ──▶ renderer.<46 campos> ──▶ │
                 │  hexToRgb (sem sRGB→linear)              WGSL inline + GLSL inline      │
                 └────────────────────────────────────────────────────────────────────────┘
                                                   ▲
                 ┌────────── 2. RUST HEADLESS (export/MCP) ──────────┐   deriva de shader,
 invoke() ─────▶ │ main.rs set_*_params ─▶ Scene ─▶ headless.rs      │   rampa, sRGB, ordem
                 │ uniforms.rs (spec_color/params3 HARDCODED)        │   de passes, mesh proxy
                 └───────────────────────────────────────────────────┘
                                                   ▲
                 ┌────────── 3. MCP (processo separado) ─────────────┐  cena própria,
 anigo_* ──────▶ │ anigo-mcp/main.rs: state.scene + state.renderer   │  defaults próprios,
                 │  └─▶ bridge WebSocket ─▶ eventos p/ a janela      │  sem clamp
                 └───────────────────────────────────────────────────┘
                                                   ▲
                 ┌────────── 4. EVENTOS DE BRIDGE (2 consumidores) ──┐
 anigo://set_* ─▶│ App.svelte (snake_case → state → updateMaterial)  │  mesmos eventos,
                 │ Viewport.svelte (snake_case → setMaterialParams   │  formatos distintos,
                 │   que espera camelCase → NO-OP silencioso)        │  corrida de estado
                 └───────────────────────────────────────────────────┘
```

Não há **nenhum** ponto de sincronização garantido entre 1, 2 e 3. Cada um tem seu próprio default (renderer: `shadowSaturation 1.15`/`baseColor (0.98,0.92,0.85)`; App: `shadowSaturation 1.1`/`baseColorHex #faeae0`; Rust `StylizedMaterial::default()`: `shade_color (0.82,0.73,0.78)`; `LiveWindowState::default()`: `fps 120`, `triangle_count 156`, `webgpu_active true`).

### 3.3 Fluxo de um único slider — onde ele se rompe

Exemplo: **"Saturação" da sombra** (`shadowSaturation`, faixa 0,0–2,5, default App 1,1 / renderer 1,15 / Rust **inexistente**):

1. `LightingControls.svelte:174` → `notifyChange(true)` → `App.svelte:2373-2384` → `updateLighting()` **+** `updateMaterial()`.
2. `updateLighting` (`App.svelte:1217`) → `viewportRef.setLight(..., shadowSaturation)` → renderer grava em `light.shadow_color.w` ✅ chega ao WGSL (`cel_shading.wgsl:161`).
3. `updateMaterial` (`App.svelte:1249`) → `setMaterialParams({shadowSaturation})` → grava o **mesmo** campo por um segundo caminho ⚠️ (duas escritas, uma fonte).
4. `invoke("set_light_params", {shadow_saturation})` → **o comando Rust não declara esse parâmetro** (`main.rs:128-152`) → descartado em silêncio ❌.
5. `invoke("set_material_toon_params", {shadow_saturation})` → **também não declara** (`main.rs:153-186`) ❌.
6. Projeto/autosave: presente ✅. Undo/redo: presente ✅. MCP `anigo_set_light`: **ausente do schema** ❌.
7. Export: `headless.rs:661-666` escreve `shadow_color[3] = 1.0` **fixo** → saturação sempre 1,0 ❌.

**Resultado:** o slider funciona apenas no viewport interativo; desaparece no export, no MCP e no motor Rust. Esse padrão se repete (ver matriz §4).

---

## 4. MATRIZ DE COBERTURA DOS PARÂMETROS (a evidência central)

Legenda: ✅ chega e é aplicado · ⚠️ chega mas diverge/parcial · ❌ ausente ou morto

| # | Controle na UI | WGSL viewport | GLSL fallback | WGSL export (Rust) | Rust `StylizedMaterial`/`Light` | Projeto/Autosave | Undo/Redo | MCP |
| :- | :--- | :-: | :-: | :-: | :-: | :-: | :-: | :-: |
| 1 | Corte (`shadowThreshold`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 2 | Suavidade (`toonSmoothness`) | ⚠️ P1-03 | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ |
| 3 | Bandas (`toonSteps`) | ⚠️ P0-09 (3 degraus inacessível) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 4 | Especular (`specIntensity`) | ⚠️ P2-07 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 5 | Tamanho (`specExponent`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 6 | Brilho · Suavidade (`specSoftness`) | ✅ | ✅ | ❌ fixo 0,05 | ❌ | ❌ | ❌ | ❌ |
| 7 | Brilho · Offset Y (`specOffset`) | ✅ | ✅ | ❌ fixo 0,0 | ❌ | ❌ | ❌ | ❌ |
| 8 | Brilho · Cor (`specColorHex`) | ✅ | ✅ | ❌ fixo branco | ❌ (`uniforms.rs:66`) | ❌ | ❌ | ❌ |
| 9 | Rim · Intensidade | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 10 | Rim · Espalhamento | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 11 | **Rim · Cor (`rimColor`)** | ❌ **morto** | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| 12 | Cor Base (`baseColorHex`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 13 | Cor Sombra (`shadowColorHex`) | ⚠️ **P0-02 dupla aplicação** | ⚠️ | ⚠️ | ⚠️ | ✅ | ✅ | ✅ |
| 14 | Hue Shift | ⚠️ P1-02 (HSV, faixa ±60 × ±180) | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ⚠️ sem clamp |
| 15 | Saturação da sombra | ✅ | ✅ | ❌ fixo 1,0 | ❌ (não existe) | ✅ | ✅ | ❌ fora do schema |
| 16 | Contorno · Espessura | ⚠️ P1-13 (3 conversões de unidade) | ⚠️ | ✅ | ✅ | ✅ | ✅ | ⚠️ heurística `>0.05` |
| 17 | Contorno · Cor | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 18 | Contorno · Opacidade | ✅ (`params.w`) | ✅ | ❌ (`inverted_hull.wgsl:62` ignora) | ❌ | ❌ | ❌ | ❌ |
| 19 | **Contorno · Suavização** | ❌ **morto** (só gravado) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| 20 | Contorno · Profundidade Z | ⚠️ default 0,0 → P2-01 | ✅ | ❌ fixo 0,0003 | ❌ | ❌ | ❌ | ❌ |
| 21 | Azimute / Elevação solar | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 22 | **Intensidade solar** | ⚠️ **P0-03 satura em 1,02** | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ✅ |
| 23 | Cor do Sol | ⚠️ P1-01 (sem sRGB→linear) | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ✅ |
| 24 | Luz Ambiente | ⚠️ P2-04 (piso 0,2; não é ambiente real) | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ✅ |
| 25 | Cor de fundo | ❌ sem UI; `(0,08;0,09;0,13)` | ❌ | ❌ `(0,12;0,13;0,16)` | ✅ (`scene.rs:119`) | ❌ | ❌ | ❌ |
| 26 | **Anti-Aliasing (Settings)** | ❌ **morto** | ⚠️ `antialias:true` | ❌ | ❌ | ❌ | ❌ | ❌ |
| 27 | Transform do objeto (posição/rotação/escala) | ❌ sem matriz de modelo | ❌ | ❌ | ✅ (`scene.rs:77`) | ❌ | ❌ | ❌ |
| — | **Total ✅ plenos** | **19/27** | **17/27** | **12/27** | **13/27** | **17/27** | **17/27** | **14/27** |

---

## 5. ACHADOS CRÍTICOS (P0)

> Formato de cada item: **Evidência → O que acontece → Impacto → Correção especificada → Critério de aceite.**

### 🔴 P0-01 — O canal de shading autorado (`_ANIGO_COLOR`) é descartado pelo loader do viewport

**Evidência.**
```ts
// webgpu_renderer.ts:1725
const colData = prim.attributes.COLOR_0 !== undefined ? getAccessorData(prim.attributes.COLOR_0) : null;
// webgpu_renderer.ts:1756-1768  (fallback quando colData === null)
let r=1,g=1,b=1,a=1;
...
color: [r, g, b, a],
```
```rust
// crates/anigo-core/src/mesh.rs:1154-1155  (motor Rust faz o certo)
let mut colors = vec![[1.0, 0.5, 1.0, 1.0]; vert_count];
let color_attr = attrs.get("_ANIGO_COLOR").or_else(|| attrs.get("COLOR_0"));
```
Parse binário real de `public/models/anigo_base_male.glb`:
```
atributos: JOINTS_0, NORMAL, POSITION, TEXCOORD_0, WEIGHTS_0, _ANIGO_COLOR   ← COLOR_0 NÃO EXISTE
_ANIGO_COLOR (4.070 vértices): R ∈ {0.95, 1.00} | G ∈ {0.48, 0.50} | B ∈ {1.00, 1.20} | A ∈ {0.85, 1.00}
  425 vértices (cabeça) com G = 0.48      → shadow shift autorado = −0.006
  2.626 vértices com A = 0.85             → máscara de especular/rim atenuada
  vértices com B = 1.20                   → contorno mais espesso (silhueta/cabelo)
  vértices com R = 0.95                   → AO leve
```

**O que acontece.** O loader não encontra `COLOR_0` e usa `[1,1,1,1]` para os 4.070 vértices. O shader interpreta esses canais como:
```wgsl
// cel_shading.wgsl:117-119
let shadow_shift = (in.anime_attr.g - 0.5) * 0.3;      // G = 1.0 → +0.150  (neutro seria 0.5 → 0.0)
let threshold    = material.params.x + shadow_shift;   // 0.50 configurado → 0.650 efetivo
let ao           = in.anime_attr.r;                    // 1.0 → AO autorado perdido
// inverted_hull.wgsl:49
let thickness    = outline.params.x * in.color.b;      // 1.0 → variação 1.2 perdida
// cel_shading.wgsl:197, 205
... * in.anime_attr.a                                  // 1.0 → máscara 0.85 perdida
```

**Impacto (medido).**
* O terminador de sombra desloca **+0,150** em espaço half-Lambert ≈ **27° de arco N·L**: a sombra ocupa muito mais área do que o slider "Corte" indica; o valor exibido na UI mente.
* A faixa útil do slider "Corte" (0,05–0,95) vira 0,20–1,10; como `half_lambert ≤ 1,0`, **os 9,5% superiores do slider são mortos** (malha 100% em sombra, sem resposta).
* Perda total de **AO estilizado**, da **máscara de especular/rim** (specular e rim aparecem em 100% do corpo, inclusive onde o artista atenuou para 0,85) e da **variação de espessura de contorno** (linha uniforme, sem ênfase de silhueta).
* O viewport e o export Rust shadingam a **mesma malha de forma diferente** (o Rust lê o canal corretamente) → além do P0-05.

**Correção especificada.**
1. Criar um **único loader glTF/GLB** compartilhado (ver P0-04/P1-14): `src/engine/gltf/` (TS) espelhando a semântica de `mesh.rs`, ou — preferencial para enterprise — expor o loader Rust via comando Tauri e consumir o mesmo `Vertex` binário nos dois lados.
2. Resolução de atributo de cor na ordem: `_ANIGO_COLOR` → `COLOR_0` → neutro canônico.
3. **Neutro canônico obrigatório: `[1.0, 0.5, 1.0, 1.0]`** (nunca `[1,1,1,1]`), pois G é um *bias* centrado em 0,5. Definir essa constante **uma vez** (`ANIME_ATTR_NEUTRAL`) e usá-la em TS, Rust e nos geradores de primitivas (hoje `generateSphereData`/`appendCube` já usam `[1,0.5,1,1]` — inconsistente com o fallback do loader).
4. Suportar `componentType` 5121/5123/5126 com normalização correta (o loader TS assume 5126 sempre: `readFloatArray` lê `getFloat32` mesmo para dados `UNSIGNED_BYTE`/`UNSIGNED_SHORT` — ver P2-12).
5. Validar e logar: se o atributo de cor estiver ausente, emitir aviso estruturado (`[ANIGO][GLTF] mesh sem canal de shading; usando neutro`) e refletir no painel de diagnóstico.
6. Documentar o contrato do canal no repositório (`docs/shading_vertex_channels.md`): R = AO, G = shadow shift (neutro 0,5, escala ×0,3), B = multiplicador de espessura de contorno (0 = sem contorno), A = máscara de especular/rim.

**Critério de aceite.**
* `anigo_base_male.glb` carrega com `G∈{0.48,0.50}`, `B∈{1.0,1.2}`, `A∈{0.85,1.0}`, `R∈{0.95,1.0}` — verificável por teste unitário que lê o GLB e confere os 4 canais.
* Com `Corte = 0.50` e luz a 45°/45°, o terminador de sombra fica em `N·L = 2·0.50−1 = 0` (±0,006 na cabeça) — validado por teste de pixel (linha de terminador na coluna esperada).
* Contorno visivelmente mais espesso nos vértices com B=1,2; especular/rim atenuados nos 2.626 vértices com A=0,85 — validado por diff contra baseline.
* Zero diferença de imagem entre viewport e export para a mesma cena (P0-05).

---

### 🔴 P0-02 — A cor de sombra é aplicada duas vezes (e o resultado não é o que o artista escolheu)

**Evidência.**
```ts
// App.svelte:1224-1225 (updateLighting)
const sunRgb    = hexToRgb(sunColor);
const shadowRgb = hexToRgb(shadowColorHex);          // → light.shadow_color
// App.svelte:1250-1251 (updateMaterial)
const baseRgb  = hexToRgb(baseColorHex);
const shadeRgb = hexToRgb(shadowColorHex);           // → material.shade_color  (MESMO hex!)
```
```wgsl
// cel_shading.wgsl:160-162
let raw_shadow_color   = material.shade_color.rgb * light.shadow_color.rgb;   // c × c = c²
let shadow_sat         = max(light.shadow_color.w, 0.0);
let hue_shifted_shadow = apply_hue_shift(raw_shadow_color, hue_shift_rad, shadow_sat);
```

**O que acontece (simulação com os defaults reais da UI).**
```
escolhido no color picker : #9995be = rgb(0.600, 0.584, 0.745)   luminância 0.599
shade_color × shadow_color: rgb(0.360, 0.341, 0.555) = #5c578e   (elevado ao quadrado)
após hue-shift −15° / sat 1.15 : rgb(0.309, 0.349, 0.555)
após ambient_term (0.2+0.35·0.8 = 0.480) : rgb(0.148, 0.168, 0.266) = #262b44
luminância renderizada    : 0.171  →  28,5% da luminância escolhida
```

**Impacto.**
* O controle mais subjetivo e importante da paleta anime ("Cor da Sombra") **não corresponde ao resultado**: sombras ~3,5× mais escuras e com matiz deslocado. Nenhum artista consegue trabalhar assim; presets ficam imprevisíveis.
* O bug é **não-linear**: cores claras (0,9) quase não mudam (0,81), cores médias colapsam (0,6 → 0,36). Por isso passa despercebido em testes superficiais.
* O mesmo hex alimenta dois conceitos distintos (tinta do material × tinta da luz), o que impede iluminação colorida independente (ex.: sombra lavanda + luz de recorte azul) — requisito de qualquer pipeline NPR comercial.
* `ambient_term` multiplica a cor da sombra, então o slider "Ambiente" altera a **saturação/luminância da tinta** em vez de adicionar luz ambiente (P2-04).

**Correção especificada.**
1. Separar o modelo de dados em **dois campos independentes**: `material.shadeColor` (tinta de sombra do material, default claro/neutro) e `light.shadowTint` (tinta da luz, default **branco neutro `[1,1,1]`**).
2. UI: no painel *palette* expor "Cor da Sombra do Material"; no painel *shadows* expor "Tint da Luz de Sombra" (com default branco e preset "Neutro"). Nunca derivar um do outro.
3. Shader: manter `shade_color × light_shadow_tint` (composição fisicamente plausível de tinta × luz), mas com o tint neutro por padrão o resultado é **exatamente** a cor escolhida.
4. Adicionar **modo de composição** selecionável (`multiply` | `replace` | `lerp`) no material — estúdios anime costumam preferir `replace` (cor de sombra absoluta) para consistência de paleta.
5. Teste unitário de contrato: `shade(hex) com tint=branco, ambient=1.0, hue=0, sat=1.0` ⇒ pixel renderizado = `hex` (ΔE < 1).

**Critério de aceite.** Escolher `#9995be` com tint neutro produz sombra `#9995be` (±1/255 por canal, medido em framebuffer linear→sRGB); presets de paleta reproduzem as cores anunciadas; os dois campos persistem separados em projeto/undo/MCP.

---

### 🔴 P0-03 — O slider de intensidade da luz satura em 1,02 (66% da faixa é decorativa)

**Evidência.**
```wgsl
// cel_shading.wgsl:165-170
let intensity = light.direction.w;
var lit_color = material.base_color.rgb * light.color.rgb * intensity;
let max_lit = max(max(lit_color.r, lit_color.g), lit_color.b);
if (max_lit > 1.0) { lit_color = lit_color / max_lit; }   // ← normalização destrutiva
```
```svelte
<!-- LightingControls.svelte:124-135 -->
<input type="range" min="0.0" max="3.0" step="0.05" bind:value={lightIntensity} />
<!-- presets: Meio-Dia 1.2 · Golden Hour 1.4 · Luar 0.85 · High-Key 1.0 -->
```

**O que acontece (simulação com `baseColor #faeae0` × `sunColor #fff8e7`).**
```
produto base×sol = rgb(0.980, 0.892, 0.796); canal máx 0.9804
⇒ para I > 1.0200 o shader divide pelo máximo e o canal máx fica PINADO em 1.000

 I=0.50 → rgb(0.490, 0.446, 0.398)   I=1.02 → rgb(1.000, 0.910, 0.812)
 I=1.00 → rgb(0.980, 0.892, 0.796)   I=1.20 → rgb(1.000, 0.910, 0.812)  ← idêntico
 I=1.40 → rgb(1.000, 0.910, 0.812)   I=2.00 → rgb(1.000, 0.910, 0.812)  ← idêntico
 I=3.00 → rgb(1.000, 0.910, 0.812)
```

**Impacto.**
* **66% do slider (1,02 → 3,0) não produz mudança alguma de brilho.** Os presets "Meio-Dia" (1,2) e "Golden Hour" (1,4) — os dois mais vendidos de qualquer pacote de luz anime — renderizam **exatamente iguais** entre si e iguais a 1,02.
* Não existe headroom para luz de estúdio, backlight forte, exposição de cena ou "high-key" — impossível atingir o look de referência (Genshin/Guilty Gear usam HDR + tonemap).
* A normalização também **dessatura** a cor da luz (saturação cai de 0,185 → 0,188 e o matiz é achatado): aumentar a intensidade *muda a cor*, não o brilho. Comportamento não-físico e não-artístico.
* O mesmo clamp aparece no GLSL (`webgpu_renderer.ts:856-858`), logo o fallback reproduz o defeito.

**Correção especificada.**
1. **Pipeline HDR real**: render target `rgba16float` (WebGPU) / `RGBA16F` + `EXT_color_buffer_half_float` (WebGL2) com resolve para o canvas; `Rgba16Float` também no headless Rust.
2. **Tone mapping estilizado** em passe final: curva paramétrica (Reinhard modificado ou ACES simplificado) + `exposure` (EV) + `whitePoint`, todos expostos na UI de Iluminação e persistidos.
3. Remover `if (max_lit > 1.0) lit_color /= max_lit;`. A proteção contra blowout deve vir do tonemap, não de um clamp que destrói a crominância.
4. Renomear o controle para **"Intensidade (EV)"** ou manter 0–3 como multiplicador linear, mas com resposta monotônica real em toda a faixa (verificável por teste).
5. Recalibrar presets de luz com valores que façam diferença mensurável (ΔL* > 3 entre presets consecutivos).

**Critério de aceite.** Varredura de I de 0,0 a 3,0 em passo 0,05 produz luminância **estritamente crescente** em toda a faixa (teste automatizado sobre framebuffer); nenhum canal clipa antes do tonemap; presets distintos produzem imagens distintas (MSE > limiar).

---

### 🔴 P0-04 — Quatro cópias do shader, com deriva funcional comprovada

**Evidência (diff programático).**
```
shaders/cel_shading.wgsl                 ≡ crates/anigo-renderer/shaders/cel_shading.wgsl   (idênticos, 1 newline)
shaders/inverted_hull.wgsl               ≈ crates/anigo-renderer/shaders/inverted_hull.wgsl (idênticos, 1 newline)
webgpu_renderer.ts:266-451  (WGSL inline) ≠ shaders/cel_shading.wgsl                        (comentários/estrutura)
webgpu_renderer.ts:455-511  (WGSL inline) ≠ shaders/inverted_hull.wgsl                      (DERIVA FUNCIONAL)
webgpu_renderer.ts:763-930  (GLSL inline) ≠ WGSL                                            (DERIVA FUNCIONAL)
shaders/*.wgsl                            → NÃO são lidos por nenhum código do frontend (grep: 0 fetch/import)
```
Derivas funcionais medidas:

| Ponto | WGSL arquivo (Rust/export) | WGSL inline (viewport) | GLSL inline (fallback) |
| :--- | :--- | :--- | :--- |
| Depth bias do contorno | `clip_pos.z += 0.0003 * w` **fixo** (`inverted_hull.wgsl:54`) | `outline.params.z` (**0,0** por default da UI) | `u_outline_depth_bias` |
| Opacidade do contorno | `return outline.color` (`:62`) — **ignora** | `outline.color.a * outline.params.w` | `u_outline_color.a * u_outline_opacity` |
| Aspect ratio | `max(params.y, 0.001)` | `params.y` **sem guarda** → div por 0 se aspect 0 | `u_aspect` sem guarda |
| Jitter especular | `world_position.y*35 + uv.x*20` | idem | `v_pos.y*35 + **v_pos.x***20` ← **outra variável** |
| Rampa toon | textura 256×4, linhas 2/3 com níveis `0/102/255` e `0/77/179/255` | mesma amostragem, níveis `0/128/255` e `0/89/179/255` | **não existe textura** (só analítico) |
| Ordem dos passes | **outline → cel** (`headless.rs:809-825`) | **cel → outline** (`webgpu_renderer.ts:2300-2310`) | cel → outline |
| Formato de cor | `Rgba8UnormSrgb` | `getPreferredCanvasFormat()` = `bgra8unorm` | `RGBA8` (sRGB por default do framebuffer) |
| Fundo | `(0,12; 0,13; 0,16)` (`scene.rs:119`) | `(0,08; 0,09; 0,13)` (`:2277`) | `(0,08; 0,09; 0,13)` |
| Profundidade | `Depth32Float` | `depth24plus` | `DEPTH_COMPONENT24` |
| Código morto | — | — | `float toon = u_coord;` (`:839`) nunca usado |

Diferença da **tabela de rampa** (TS × Rust), medida texel a texel:
```
linha 0 (contínuo) : 0/256 divergentes
linha 1 (1 degrau) : 0/256 divergentes
linha 2 (2 degraus): 81/256 divergentes, |Δ|máx = 127  (TS 0/128/255 × Rust 0/102/255, cortes 89/166 × 85/135)
linha 3 (3 degraus): 76/256 divergentes, |Δ|máx = 90   (TS 0/89/179/255 × Rust 0/77/179/255, cortes 64/128/191 × 64/120/180)
```

**Impacto.** Quatro comportamentos visuais para o mesmo conjunto de parâmetros; correções aplicadas em um lugar não chegam aos outros (já aconteceu: opacidade e bias de contorno existem só no viewport); o modo "2 Degraus (Ghibli)" e o "3 Degraus" produzem bandas em posições diferentes no viewport e no export; o fallback WebGL2 tem *look* próprio (sem rampa em textura, jitter em outro eixo). Manutenção impossível e regressões silenciosas garantidas.

**Correção especificada.**
1. **Fonte única**: `shaders/` vira o diretório canônico. Frontend importa com `import celShader from "../../../shaders/cel_shading.wgsl?raw"` (Vite suporta nativamente); Rust mantém `include_str!("../shaders/...")` apontando para **o mesmo arquivo** (mover `crates/anigo-renderer/shaders` → symlink/`build.rs` que copia do diretório raiz, ou path relativo `../../../shaders`). Eliminar a cópia duplicada.
2. **Fallback WebGL2**: eliminar o GLSL escrito à mão. Opções, em ordem de preferência: (a) compilar WGSL→GLSL ES 3.0 no build com `naga`/`tint` e embutir o resultado gerado (paridade garantida por construção); (b) usar `webgpu` polyfill; (c) se mantiver GLSL manual, gerar ambos a partir de um único `.h` de funções compartilhadas e ter **teste de paridade de pixels** obrigatório.
3. **Tabela de rampa**: gerar por uma função única e compartilhada (ex.: `shaders/toon_ramp.ts` + `toon_ramp.rs` gerados de uma mesma especificação JSON, ou um único arquivo `.json` lido pelos dois) e adicionar teste que compara os 1.024 valores.
4. **Ordem de passes e estado de pipeline** (depth format, blend, cull, sample count, background) declarados **uma vez** num `RenderConfig` consumido pelos dois motores.
5. Lint de repositório: proibir `createShaderModule({ code: \`...\` })` com literal inline (regra ESLint customizada ou teste estático que falha se encontrar template literal com `@vertex`/`@fragment`).

**Critério de aceite.** `grep -rn "createShaderModule" src` retorna apenas imports de arquivo; os 1.024 valores de rampa são idênticos em TS e Rust (teste unitário); o mesmo `RenderConfig` produz a mesma imagem no viewport, no fallback e no export (MSE < 1,0 em cena de referência).

---

### 🔴 P0-05 — O que se exporta não é o que se vê (WYSIWYG quebrado em 8 eixos)

**Evidência.**
```rust
// src-tauri/src/main.rs:105-121 — export usa MESH PROXY de caixas, nunca o GLB canônico
"sphere" => Mesh::create_uv_sphere(0.8, 32, 64),
"cube"   => Mesh::create_cube(1.0),
_        => Mesh::create_mannequin_proxy(),      // 13 caixas (mesh.rs:246-273)

// crates/anigo-renderer/src/uniforms.rs:66 e :79 — comentários que confessam a provisoriedade
specular_color: [1.0, 1.0, 1.0, 1.0], // Core material doesn't have specular_color yet, default to white
params3: [0.05, 0.0, 0.0, 0.0],       // Default spec softness and offset

// crates/anigo-renderer/src/headless.rs:744-746 — opacidade/bias de contorno zerados
params: [mat.outline_width, aspect, 0.0, 0.0],
// headless.rs:665 — saturação de sombra fixa
shadow_color: [r, g, b, 1.0],
```
* `render_viewport_frame` (`main.rs:34-67`) **nunca é chamado pelo frontend** (grep: 0 ocorrências em `src/`); o painel *Render → Composição & Exportação* (`App.svelte:2795-2823`) é uma maquete: botões de resolução sem `onclick`, checkboxes com `checked` fixo, botão "Renderizar Imagem Atual" sem handler.
* A câmera do export vem de `scene.camera` (mutada só por `camera_orbit/zoom/pan` do Rust, com clamps e semântica próprios: elevação ±89°, zoom 0,2–50, pan em unidades de mundo) — **não** da câmera real do viewport (elevação livre, zoom 0,2–40, pan calibrado em pixels de tela). O snapshot de projeto grava `cameraEye: [0, 1.5, 3.5]` **fixo** (`App.svelte:659-660`).

**Impacto.** Exportação/offscreen é inutilizável para produção: geometria diferente (caixas vs. malha canônica de 6.880 triângulos), cor diferente (sRGB vs. linear-ish), rampa diferente, especular/rim/contorno diferentes, câmera diferente, e 8 parâmetros de shading ignorados. Qualquer estúdio que usar o ANIGO para entregar frames descobre a divergência no primeiro render — quebra de confiança fatal. Viola a Regra Inviolável nº 2 ("zero implementação provisória") e o DoD da Sprint 02.

**Correção especificada.**
1. **Uma cena, dois consumidores.** O estado canônico de cena (malha carregada, transform, material, luz, câmera) vive num único `SceneStore` (TS) serializável; o Rust recebe a **cena inteira** (ou um handle de cena persistido) antes de cada render offscreen — nunca mantém uma cena paralela com defaults próprios.
2. `load_mesh_preset` deve carregar **o mesmo GLB canônico** (o loader Rust já existe em `mesh.rs` e lê `_ANIGO_COLOR`), não um proxy de caixas. O proxy pode existir como *fallback* explícito e nomeado (`"proxy_debug"`), jamais como default.
3. Completar `StylizedMaterial`/`StylizedLight` com **todos** os campos: `specular_color`, `spec_softness`, `spec_offset`, `rim_color`, `outline_opacity`, `outline_smoothness`, `outline_depth_bias`, `shadow_saturation`, `background_color`, `exposure`, `toon_ramp_id`. Remover os dois comentários "yet" de `uniforms.rs`.
4. Sincronizar câmera: o viewport publica `camera {eye,target,up,fov,near,far}` no `SceneStore`; o export usa exatamente esses valores (e o mesmo `RenderConfig` de formato de cor/profundidade/passes).
5. Implementar o painel *Render* de verdade: resolução (com `render_scale`), passes (beauty/line/shadow/depth/normal/mask), formato (PNG 8/16-bit, EXR), caminho de destino, fila de render, preview do frame e **comparação A/B com o viewport**.
6. Alternativa pragmática de curto prazo (Fase 0): exportar a partir do **próprio canvas WebGPU** (`copyExternalImageToTexture`/`copyTextureToBuffer` + `readPixels` no GL) usando o mesmo pipeline do viewport — garante paridade imediata enquanto o offscreen Rust é unificado.

**Critério de aceite.** Para uma cena de referência fixa, `MSE(viewport_png, export_png) < 1,0` e `PSNR > 45 dB` (usando `anigo_compare_baseline`), com todos os 27 parâmetros em valores não-default; painel Render funcional e testado; `render_viewport_frame` coberto por teste E2E.

---

### 🔴 P0-06 — Fallback WebGL2 é inatingível depois de uma falha WebGPU, e erros de GPU nunca são observados

**Evidência.**
```ts
// webgpu_renderer.ts:224-262
this.context = this.canvas.getContext("webgpu");
if (this.context) {
  this.context.configure({ ... });      // ← canvas fica permanentemente vinculado a "webgpu"
  this.buildShadersAndPipelines();      // se isto falhar...
  ...
} catch (e) { console.warn("...switching to WebGL2 fallback:", e); }
// webgpu_renderer.ts:755-756
const gl = this.canvas.getContext("webgl2", { antialias: true, alpha: false });   // ← retorna null
if (!gl) { console.error("[ANIGO 3D] WebGL2 not supported..."); return false; }   // → viewport preto
```
* Um `HTMLCanvasElement` só pode ter **um** tipo de contexto: após `getContext("webgpu")`, `getContext("webgl2")` retorna `null` (HTML Living Standard, *same origin of context*). Logo o caminho de fallback só funciona se o WebGPU falhar **antes** de `getContext("webgpu")`.
* Erros de compilação de shader e de criação de pipeline no WebGPU são **assíncronos** (error scope / `device.lost`), não exceções: `createShaderModule` não lança. Não há `getCompilationInfo()`, `pushErrorScope/popErrorScope`, `device.addEventListener('uncapturederror')` nem `device.lost.then(...)` em todo o repositório (grep: 0 ocorrências).
* `this.backend` inicia como `"webgpu"` (`:83`); se `initWebGL2()` retornar `false`, `backend` continua `"webgpu"` com `device`/`vertexBuffer` possivelmente nulos → `renderWebGPU` sai em silêncio a cada frame.

**Impacto.** Em qualquer máquina onde o WGSL não compile (driver antigo, limite de recurso, WebView2 sem flag), o usuário vê **uma tela preta sem nenhuma mensagem**, sem fallback e sem log acionável. Para um produto comercial isso é o pior modo de falha possível: suporte sem diagnóstico, e a promessa de "nunca é uma tela preta" (comentário no `:256`) é falsa.

**Correção especificada.**
1. **Probe antes de vincular**: criar um `canvas` *offscreen* (`document.createElement("canvas")`) para testar `navigator.gpu.requestAdapter/requestDevice` + compilação do shader; só então chamar `getContext("webgpu")` no canvas visível. Se o probe falhar, o canvas visível ainda está limpo para `getContext("webgl2")`.
2. Alternativa mais robusta: **dois canvas empilhados** (WebGPU e WebGL2), exibindo apenas o ativo; ou `OffscreenCanvas` + `ImageBitmap` para o fallback.
3. **Diagnóstico obrigatório de shader**: após `createShaderModule`, chamar `await module.getCompilationInfo()` e tratar `error`/`warning` (log estruturado + toast + relatório copiável). Envolver criação de pipelines em `device.pushErrorScope("validation")` / `await device.popErrorScope()`.
4. `device.lost.then(info => ...)` com **recovery**: re-inicializar (novo device, novos buffers) até 3 vezes com backoff; se falhar, degradar para WebGL2 (com o canvas já limpo) e, por fim, para um estado de erro visível com CTA de suporte.
5. `device.addEventListener("uncapturederror", ...)` + `webglcontextlost`/`webglcontextrestored` no canvas + fila circular de erros exibível no painel de diagnóstico.
6. `initialize()` deve retornar um **resultado tipado** (`{backend, diagnostics[], degraded: boolean}`) e nunca deixar `backend` inconsistente com o estado real; estado `error` deve renderizar um painel explicativo (não canvas preto).

**Critério de aceite.** Teste com WGSL deliberadamente inválido: o app (a) reporta o erro de compilação com linha/coluna na UI, (b) cai no fallback WebGL2 e renderiza a cena, (c) registra em log estruturado. Simulação de `device.lost` recupera sem reload. Nenhum caminho produz canvas preto sem mensagem.

---

### 🔴 P0-07 — Não existe anti-aliasing (e a configuração "MSAA 4x" é decorativa)

**Evidência.**
```ts
// webgpu_renderer.ts:520-570 — pipelines SEM campo multisample (default sampleCount = 1)
this.celPipeline = this.device.createRenderPipeline({ ... primitive: {...}, depthStencil: {...} });
// SettingsModal.svelte:17,32,540-557 — a UI promete três modos
antiAliasing: "msaa4x" | "fxaa" | "none";   antiAliasing: "msaa4x",   // DEFAULT
// App.svelte:681-699 — handleSaveStudioSettings aplica fpsCap, dpiScale, vsync... e ignora antiAliasing
```
```rust
// headless.rs:300 e 347 — export também sem AA
multisample: wgpu::MultisampleState::default(),   // sample_count = 1
```
DoD Sprint 02: *"Transição nítida de anime **sem serrilhado**"* — não atendido. Ironicamente o único backend com AA é o fallback WebGL2 (`antialias: true`), ou seja, **o caminho degradado tem qualidade de silhueta melhor que o caminho principal**.

**Impacto.** Silhuetas e contornos inverted hull serrilhados em 1080p/4K — o defeito mais visível de qualquer renderer NPR, e justamente o que mais aparece em screenshots de venda. Inconsistência de qualidade entre backends e entre viewport/export. Setting que não faz nada destrói a confiança na aplicação inteira.

**Correção especificada.**
1. **MSAA 4x no WebGPU**: criar `colorTexture` (`rgba8unorm`/`rgba16float`, `sampleCount: 4`, `TEXTURE_BINDING|RENDER_ATTACHMENT`) + `resolveTarget` no `colorAttachments`; pipelines com `multisample: { count: 4 }`; depth também 4x. No WebGL2 já existe `antialias: true` (validar que o contexto não é `preserveDrawingBuffer:false` com custo indevido).
2. **Pós-AA opcional** (FXAA 3.11 ou SMAA 1.x) como passe fullscreen para quando MSAA não estiver disponível (limites de `sampleCount` do adaptador) ou quando o usuário escolher; consultar `adapter.limits.maxColorAttachmentSamples`/`maxSampleCount` e degradar com aviso.
3. Aplicar a configuração: `handleSaveStudioSettings` → `renderer.setAntiAliasing(mode)` → **recriar pipelines e attachments** (com `try/catch` e diagnóstico), e persistir a escolha.
4. Espelhar no Rust (`MultisampleState { count: 4, .. }` + resolve) para que o export tenha o mesmo AA — e expor `render_samples` no painel Render (1/2/4/8).
5. O contorno inverted hull se beneficia duplamente (a extrusão gera geometria fina); validar com baseline de silhueta.

**Critério de aceite.** Modo `msaa4x` reduz a contagem de pixels de aresta serrilhada em ≥ 70% vs. `none` (métrica de gradiente em teste de imagem); os três modos produzem imagens distintas e são persistidos; viewport e export com o mesmo AA.

---

### 🔴 P0-08 — Telemetria fabricada em duas fontes concorrentes alimenta o MCP

**Evidência.**
```ts
// App.svelte:1341-1366 — valores hardcoded, enviados a CADA input de slider
draw_calls: 2,
triangle_count: currentPreset === "mannequin" ? 156 : currentPreset === "sphere" ? 2592 : 12,
camera_eye: [0, 1.5, 3.5], camera_target: [0, 1, 0],
light_direction: [0.577, 0.577, 0.577], shadow_color: [0.65, 0.68, 0.85],
```
```ts
// Viewport.svelte:300-330 — telemetria REAL, enviada a cada 500 ms, para o MESMO estado
camera_eye: renderer?.eye, light_direction: renderer?.lightDir, triangle_count: m.triangles, ...
```
```rust
// bridge.rs:118-145 — Default fabricado: fps 120.0, frame_time 0.5, triangle_count 156,
//                     adapter "WebGPU Native Hardware", webgpu_active: true
// anigo-mcp/main.rs:1113-1139 — anigo_render_frame com sync_live usa camera_eye/camera_target
//                                 lidos DESSA telemetria para posicionar a câmera do export
```

**Impacto.**
* O MCP (`anigo_get_live_telemetry`, `anigo_inspect_scene`, `anigo_render_frame sync_live=true`) recebe **última escrita ganha** entre valores reais e fabricados → relatórios de validação visual (obrigatórios pela Regra Inviolável nº 1) podem ser gerados sobre dados falsos. `156 triângulos` quando a malha real tem **6.880**.
* `sync_live` pode posicionar a câmera do export em `[0,1.5,3.5]` fixo em vez da câmera real → o agente "valida" um frame que não corresponde ao que o usuário vê.
* `webgpu_active: true` e `fps: 120` como **default** significam que um motor sem GPU reporta saúde plena.
* `reportLiveTelemetry()` é chamado em cada `oninput` de slider → IPC + `import()` dinâmico por evento (tempestade de IPC, ver P1-10).

**Correção especificada.**
1. **Fonte única**: apenas o renderer publica telemetria (via callback → store). Remover `reportLiveTelemetry()` do App (ou reduzi-lo a leitura do store, sem inventar campos).
2. `LiveWindowState::default()` → valores sentinelas explícitos (`fps: 0.0`, `triangle_count: 0`, `adapter_name: "uninitialized"`, `webgpu_active: false`) + campo `last_update_ms`; o MCP deve **recusar** sincronizar com telemetria obsoleta (> 2 s) e reportar o motivo.
3. Completar `LiveWindowState` com o estado real de shading (todos os 27 parâmetros) para permitir verificação de ida-e-volta (`set` → `get` → confere).
4. Throttle único (máx. 4 Hz) + envio por diferença (delta) e um `invoke` por lote; remover o `import()` dinâmico repetido (importar `@tauri-apps/api/core` uma vez no bootstrap).
5. `frameTimeMs` deve ser tempo de quadro real (delta de `requestAnimationFrame` ou timestamp GPU), não o CPU-submit (P2-08).

**Critério de aceite.** `anigo_get_live_telemetry` retorna `triangle_count = 6880` com a malha canônica; `camera_eye` bate com `renderer.eye` (Δ < 1e-4); desconectar o viewport faz a telemetria marcar `stale/uninitialized` (nunca valores saudáveis fictícios); ≤ 4 IPC/s em repouso e durante drag de slider.

---

### 🔴 P0-09 — Controles de UI mortos: `rimColor`, `outlineSmoothness` e a banda de 3 degraus

**Evidência.**
```svelte
<!-- App.svelte:346 -->   let rimColor = $state("#93c5fd");
<!-- App.svelte:2014-2018 -->  <input type="color" bind:value={rimColor} oninput={() => updateMaterial(true, true)} />
<!-- updateMaterial (1249-1277) NUNCA envia rimColor -->
```
```wgsl
// cel_shading.wgsl:209 — o rim é tingido pela COR DE SOMBRA DA LUZ, não por rimColor
let with_rim = lit_highlighted + (light.shadow_color.rgb * rim_term);
```
```ts
// webgpu_renderer.ts:179 / 1198 — outlineSmoothness só é gravado, nunca lido
public outlineSmoothness: number = 0.0;
if (params.outlineSmoothness !== undefined) this.outlineSmoothness = params.outlineSmoothness;
// App.svelte:2058-2068 — slider "Suavização" 0–100% ligado a esse campo
```
```svelte
<!-- App.svelte:1925-1946 — apenas 3 botões: 1.0, 2.0, 0.0 -->
<!-- cel_shading.wgsl:144-151 — o ramo "3 Degraus (High-Key Multi-band)" exige toon_steps >= 2.5 -->
```
O contrato de testes declara `rimColor` como propriedade do inspetor (`workspace_pipeline.test.ts:63,183`) — o teste passa sem que o controle exista de fato.

**Impacto.** O usuário move um color picker de Rim Light e **nada acontece**; move "Suavização" do contorno e **nada acontece**; o shader tem uma banda de 3 degraus implementada que **ninguém alcança pela UI**. Regra de produto: *todo controle visível deve ter efeito observável*. Controles mortos são o tipo de defeito que elimina um software de uma avaliação comercial em minutos.

**Correção especificada.**
1. **`rimColor`**: adicionar `rim_color: vec4<f32>` ao `MaterialUniform` (novo `params4` ou campo próprio, mantendo alinhamento 16 B), usar no shader (`with_rim = lit + rim_color.rgb * rim_term`), expor em `StylizedMaterial`, IPC, MCP, projeto, undo. Default `#93c5fd` (o valor já presente no estado).
2. **`outlineSmoothness`**: implementar de verdade — opções: (a) *smoothstep* de espessura com derivada de tela (`fwidth`) para AA de contorno; (b) alpha-to-coverage com MSAA; (c) passe de suavização da silhueta (blur do mask de contorno). Escolher (a)+(b), documentar e expor no export. **Se não for implementado na sprint, remover o slider** — nunca deixar controle inerte.
3. **Bandas**: expor 4 modos (`0 = gradiente`, `1 = cel`, `2 = Ghibli`, `3 = high-key`) como segmento de controle, com preview de rampa ao lado; alinhar as posições de banda entre textura e analítico (P1-03).
4. Auditoria automática de "controles mortos": teste que varre os `bind:value` do inspetor e verifica que cada estado aparece em (i) `setMaterialParams`/`setLight`, (ii) `getProjectSnapshot`, (iii) `getHistorySnapshot`. Falha de build se houver estado de UI não propagado.

**Critério de aceite.** Alterar `rimColor` muda a cor da borda (ΔE > 10 em pixels de silhueta); alterar `outlineSmoothness` muda a transição do contorno (métrica de gradiente); `toon_steps = 3` acessível e produz 3 bandas distintas; teste de "controles mortos" verde.

---

### 🔴 P0-10 — Undo/Redo, Save e Autosave não preservam o shading (e gravam câmera falsa)

**Evidência.**
```ts
// history_service.ts:7-34 — HistoryStateSnapshot: 17 campos, sem
//   specSoftness, specOffset, specColorHex, rimColor, outlineOpacity,
//   outlineSmoothness, outlineDepthBias, cameraEye/Target/up/fov, focalLength, somatotype/morphs
// autosave_service.ts:1-29 — ProjectStateSnapshot: mesmas ausências
// App.svelte:659-660 — câmera gravada com valores FIXOS
cameraEye: [0, 1.5, 3.5],
cameraTarget: [0, 1, 0],
```
* `applySnapshot` (`App.svelte:477-527`) só restaura o que existe no snapshot: após um undo, `specColor`, `specSoftness`, `specOffset`, opacidade/bias de contorno **permanecem no valor novo** → estado misto, inconsistente com o que o usuário desfez.
* Projeto salvo → fechado → reaberto: perde cor de especular, suavidade/offset de brilho, opacidade/suavização/profundidade de contorno, câmera, FOV/distância focal, e todo o estado de personagem.
* `version: "0.1.0"` sem esquema de migração; `load_project_file` → `JSON.parse` → `applySnapshot` **sem validação** (um arquivo corrompido ou com hex inválido produz `NaN` nos uniforms → tela preta, ver P1-01).

**Impacto.** Perda silenciosa de trabalho artístico — o defeito mais grave para um DCC comercial. Undo que "meia-volta" o estado é pior que não ter undo, porque o usuário confia nele.

**Correção especificada.**
1. Definir **um único schema canônico e versionado** (`AnigoProject v1`) que contenha `ShadingProfile`, `LightRig`, `CameraState`, `CharacterState`, `RenderSettings` — ver Anexo B. Usar o mesmo tipo em: estado da UI, histórico, projeto, autosave, IPC Tauri e MCP.
2. Histórico: snapshot **completo** (derivado do store, não listado à mão) + `description` i18n + coalescência por *gesto* (não por 500 ms arbitrários).
3. `cameraEye/cameraTarget` lidos do renderer (`renderer.eye/target/up/fov`) no momento do snapshot; restauração aplica no renderer.
4. Validação de entrada com biblioteca de schema (ex.: `zod`/`valibot` ou validador próprio tipado): rejeitar/coagir valores fora de faixa, hex inválido, arrays de tamanho errado; **nunca** deixar `NaN` chegar aos uniforms (ver P1-01). Migração explícita `v0 → v1`.
5. Autosave com rotação (manter N últimos), checksum, e recuperação de crash no boot ("Projeto recuperado de HH:MM").
6. Teste de round-trip: para cada um dos 27 parâmetros, `set(valor não-default) → save → load → get === valor` e `set → undo → get === anterior`.

**Critério de aceite.** 27/27 parâmetros sobrevivem a undo/redo e a save/load (teste parametrizado); arquivo inválido produz erro amigável e estado íntegro (nunca `NaN`/tela preta); câmera real restaurada.

---

### 🔴 P0-11 — Normais nunca são recalculadas após deformação → sombreamento e contorno errados

**Evidência.**
```ts
// webgpu_renderer.ts:1386-1672 (applyAnatomicalDeformations)
const vertices = baseVertices.map(v => ({ ..., normal: [v.normal[0], v.normal[1], v.normal[2]], ... }));
for (let i = 0; i < vertices.length; i++) {
  ...  x += Math.sign(x) * 0.038 * f * hipFlare;   // desloca posições
  ...  z += 0.055 * intensity * bustCup;           // desloca posições
  v.pos[0] = x; v.pos[1] = y; v.pos[2] = z;        // ← normal intocada
}
```
Não há recálculo de normais em nenhum lugar do renderer (grep `normal =` / `recomputeNormals`: 0 ocorrências), nem no `morph_sparse_compute.wgsl` para a deformação de CPU (o compute soma `delta_n*` autorados, mas esse caminho é código morto — P1-09).

**Impacto.** Todo morph, somatótipo ou proporção altera a superfície **mantendo as normais da malha base**: o terminador de sombra (`N·L`), o fresnel de rim (`V·N`), o especular anisotrópico (`N·H`) e a extrusão do contorno (que usa `in.normal`) ficam **geometricamente incorretos**. Em deformações grandes (busto, glúteo, coxa, mandíbula) isso aparece como manchas de sombra, rim em lugar errado e contorno que "vaza" ou some. É um defeito de shading puro, ainda que a causa esteja no módulo de personagem.

**Correção especificada.**
1. **Recalcular normais por vértice após qualquer deformação**: acumular normais de face ponderadas por ângulo (ou por área) nas malhas indexadas e renormalizar. Implementação canônica em `src/engine/mesh/normals.ts` com testes unitários (esfera → normal = posição normalizada; cubo → normais de face).
2. Para o caminho GPU (quando o compute for religado, P1-09): recalcular normais **no shader de compute** a partir dos deltas (já existe `delta_n*` no formato `SparseMorphDelta`) ou por passe de vizinhança; nunca interpolar normais sem renormalizar.
3. **Normais de contorno separadas** (Sprint 05 exige): campo `outline_normal` com normais suavizadas/averaged (k-NN ou média por aresta dentro de um raio elipsoidal) para que o inverted hull tenha espessura uniforme em regiões côncavas (axilas, virilha, pescoço, dobra do joelho). Sem isso o DoD da Sprint 05 ("contorno com espessura uniforme") não é atingível.
4. Custo: recalcular 4.070 vértices + 6.880 faces é ~1–2 ms em JS; deve rodar **fora da thread principal** (Worker) ou na GPU, e só quando a deformação mudar (dirty flag), nunca por evento de `oninput` (P1-09).

**Critério de aceite.** Após aplicar `bust_volume_cup = 1.0` e `gluteus_volume_overall = 1.0`, as normais são ortogonais à superfície deformada (teste: para cada vértice, `|dot(normal, média das faces adjacentes)| > 0.98`); o terminador de sombra acompanha a nova geometria (validação visual via MCP); contorno com variação de espessura < 15% ao longo da silhueta.

---

## 6. ACHADOS ALTOS (P1)

### 🟠 P1-01 — Ausência total de gestão de cor (sRGB ↔ linear) e de validação de entrada
`hexToRgb` (`App.svelte:414-423`) devolve valores sRGB 0–1 **direto para os uniforms**, o shader faz aritmética (multiplicação, HSV, mix) nesses valores e escreve num canvas `bgra8unorm` **sem codificação sRGB**; o Rust escreve em `Rgba8UnormSrgb` (**com** codificação). Consequências: (a) mistura de cores fisicamente errada (soma/multiplicação em espaço não-linear → sombras sujas, gradientes com banding, rim acinzentado); (b) viewport e export com gamma diferente (P0-05); (c) `parseInt(hex,16)` sem validação — um projeto com `"cor_sombra": "#XYZ"` ou `null` produz `NaN`, que se propaga para **todos** os pixels (tela preta, sem erro). **Correção:** pipeline linear (converter sRGB→linear na entrada de todas as cores; trabalhar em linear; tonemap; codificar sRGB na saída — ou usar formato de canvas sRGB e manter o shader em linear), validação de schema em toda entrada externa (UI, projeto, IPC, MCP) com coerção e log, e teste de round-trip `hex → linear → shader → sRGB → hex` (Δ ≤ 1).

### 🟠 P1-02 — Hue-shift em HSV (não-perceptual) e faixas inconsistentes entre camadas
`apply_hue_shift` (`cel_shading.wgsl:95-100`) rotaciona o matiz em **HSV**, onde a roda não é perceptualmente uniforme: −15° em torno de azul produz deslocamento visual muito diferente de −15° em torno de amarelo; além disso `rgb_to_hsv`/`hsv_to_rgb` perdem informação em cores de baixa saturação (o `e = 1e-10` estabiliza divisão, mas o matiz de cinzas é arbitrário → sombras neutras ganham tinta aleatória). Faixas divergentes: UI ±60° (`LightingControls.svelte:170`), comentário do renderer `degrees (-60 to +60)` (`:190`), doc do Rust `-180 to +180` (`scene.rs:47`), schema MCP `-180.0 to +180.0` (`anigo-mcp/main.rs:284`) e **nenhum clamp em lugar nenhum**. **Correção:** migrar para **OKLab/OKLCH** (rotação de matiz perceptualmente uniforme, padrão da indústria em 2024+), faixa canônica única (−180…+180) com clamp validado nas 4 camadas, e preservação de crominância para sombras dessaturadas (não rotacionar se `C < ε`).

### 🟠 P1-03 — O slider "Suavidade" troca o *modelo* de shading (crossfade rampa-textura × analítico)
```wgsl
// cel_shading.wgsl:153
let toon_factor = mix(ramp_sample.r, analytical_toon, clamp((smoothness - 0.015) * 15.0, 0.0, 1.0));
```
Medido: `smoothness ≤ 0,015` → 100% textura; `0,02` (default) → 92,5% textura; `0,05` → 47,5%/52,5%; `≥ 0,0817` → 100% analítico. Como as **posições de banda diferem** entre os dois modelos (textura linha 2: ±0,150 e nível médio 0,500; analítico 2-degraus: ±0,140 e `0,45·s1 + 0,55·s2`; linha 3: ±0,250 vs ±0,200), arrastar "Suavidade" faz o terminador **migrar de posição e mudar de carácter** — o controle não é ortogonal. **Correção:** um único modelo de bandas paramétrico (nº de bandas, posição, largura de penumbra, níveis) implementado **ou** por textura **ou** por fórmula, com a textura gerada *a partir* dos mesmos parâmetros (o que também habilita o editor de rampa, P3-06). "Suavidade" controla apenas a largura da penumbra; "Corte" apenas a posição; "Bandas" apenas a quantização.

### 🟠 P1-04 — Rampa toon procedural, hardcoded, não editável e não exportável
A rampa 256×4 é gerada em código (`webgpu_renderer.ts:1835-1860`, `headless.rs:84-104`) com 4 linhas fixas. Não há: UI para editar a curva, presets de rampa por material, import/export (PNG/JSON), interpolação entre rampas, nem suporte a rampas 2D (hue-vs-luma, usadas em produções anime). O próprio DoD da Sprint 02 fala em "amostragem Toon Ramp 1D/2D". **Correção:** asset de rampa versionado (`assets/ramps/*.png|json`) + editor de curva no painel `cel_shader` + campo `ramp_id` no material + carregamento no renderer e no Rust a partir do mesmo asset + teste de paridade dos 1.024 valores.

### 🟠 P1-05 — Sistema de luz é uma única direcional: sem shadow map, sem ambiente real, sem SDF facial (Sprint 05/18 não implementadas)
Não existe sombra projetada de nenhum tipo (só o terminador N·L), não há hemisfério/IBL (o "Ambiente" apenas escala a cor da sombra, com piso 0,2 — P2-04), não há luz de preenchimento/recorte como fontes reais, e **`shaders/face_sdf.wgsl` não existe** (exigido pela Sprint 05: "mapas SDF de sombra facial estilo Genshin Impact", com ferramenta MCP `anigo_set_face_light_angle` — também inexistente; grep: 0 ocorrências). Consequência: sombras de nariz/bochecha quebram em qualquer ângulo de luz, exatamente o que a Sprint 05 proíbe. **Correção (roadmap P3-02/03/05):** shadow map estilizado (1 direcional + resolução configurável, filtragem hard/soft anime, bias normal/constant), shadow catcher no plano do chão, SDF facial (bake em textura + shader de amostragem com ângulo de luz), luz hemisférica céu/chão + IBL estilizado, e light rigs nomeados (key/fill/rim/back/kicker).

### 🟠 P1-06 — Sem normais customizadas para o contorno (Sprint 05) → espessura não uniforme
O inverted hull extrude ao longo da **normal geométrica** (`inverted_hull.wgsl:47-56`). Em regiões côncavas e em bordas duras (cubos, queixo, ombro, dedos) isso produz espessura irregular e "buracos" de contorno — o DoD exige "contorno com espessura uniforme". Não há transferência k-NN de normais elipsoidais nem edição de normais (a Sprint 05 pede os dois). **Correção:** (a) canal adicional `outline_normal` no vértice (stride 72 → 84 B ou reuso de atributo), (b) ferramenta de bake por média ponderada/k-NN com raio elipsoidal por zona anatômica, (c) UI de pintura/visualização de normais de contorno, (d) teste de uniformidade de espessura em silhueta.

### 🟠 P1-07 — Dois consumidores para o mesmo evento de bridge, com formatos incompatíveis
`anigo://set_material_toon` é tratado em `App.svelte:822-845` (snake_case → estado → `updateMaterial`) **e** em `Viewport.svelte:407-411` (`renderer.setMaterialParams(event.payload)` — que espera **camelCase**, logo é um **no-op silencioso**). O mesmo para `set_light`, `load_preset`, `set_proportions`, `set_outline`, `recenter_camera`. Além da duplicação, há **corrida**: os dois handlers escrevem no renderer em ordem indefinida, e o do Viewport bypassa o estado da UI (sliders ficam mostrando valor antigo). **Correção:** um único consumidor (o App/store); o Viewport nunca escuta bridge; todo payload externo passa por **um** validador/normalizador (`normalizeBridgePayload`) que converte snake↔camel, clamp e loga campos desconhecidos; teste de contrato por evento.

### 🟠 P1-08 — MCP não cobre o shading e sobrescreve o estado da UI com defaults próprios
`anigo_set_material_toon` expõe 12 campos; ficam de fora `spec_color`, `spec_softness`, `spec_offset`, `rim_color`, `outline_opacity`, `outline_smoothness`, `outline_depth_bias`, `shadow_saturation`; `anigo_set_light` não expõe `shadow_saturation`. Nenhum valor é **clampado/validado** (`shadow_threshold: 5.0`, `outline_width: -1`, `hue_shift: 1e9` são aceitos e vão direto ao shader). Pior: o handler devolve **o material inteiro** ao app (`main.rs:752-769`) — se o MCP alterar só `base_color`, o app recebe `spec_intensity: 0.4` (default da cena do MCP) e **reverte silenciosamente** o que o artista ajustou. **Correção:** schema MCP completo (27 parâmetros) + validação/clamp por faixa canônica (uma única tabela compartilhada TS/Rust) + envio **apenas dos campos alterados** (patch semantics) + `anigo_get_shading_profile` para leitura de volta + testes de round-trip.

### 🟠 P1-09 — Deformação reconstrói a malha na main thread a cada `oninput`, com *churn* de buffers GPU; o pipeline de morph em compute é código morto
Cada slider chama `setMorphSlider`/`setSomatotype`/`setProportions` → `buildGeometryBuffers()` (`:1203`), que (a) roda `applyAnatomicalDeformations` sobre 4.070 vértices **sincronamente na thread principal**, (b) `destroy()` + `createBuffer()` + `writeBuffer()` de VBO/IBO a cada evento (`:1223-1240`), (c) adiciona o pedestal a cada rebuild. Enquanto isso, `uploadSparseMorphData` (`:1023`) **não tem nenhum chamador** no repositório e `dispatchSparseMorphs` só roda se `morphVertexCount > 0` (`:2269`) → o pipeline compute de morphs é código morto; se fosse ligado hoje, o `activeVbo` passaria a ser o buffer de morph **sem** as deformações de CPU (a base enviada é a malha canônica), causando snap visual. **Correção:** (a) dirty-flag + debounce/coalescência em `requestAnimationFrame`, (b) buffers persistentes com `writeBuffer` (sem destroy/create) e reaproveitamento por tamanho, (c) mover deformação para Worker ou para o compute shader (religando o pipeline com a base correta e validando `morphVertexCount === vertexCount`), (d) benchmark obrigatório: ≤ 4 ms de CPU por frame durante drag de slider, 0 alocações de buffer GPU por frame.

### 🟠 P1-10 — Sem pause/resume do render loop; viewport oculto continua renderizando
Entrar em *Biblioteca* esconde o viewport com `display:none` (`App.svelte:3205-3207`) mas o loop `requestAnimationFrame` continua renderizando a 120 fps um canvas invisível (não há `pause()`, `resume()`, nem listener de `visibilitychange`; o harness de teste até afirma que `isRenderLoopPaused = true`, mas isso **não existe no código**). `reportLiveTelemetry` dispara IPC por evento de slider. **Correção:** API `pause()/resume()` no renderer; `IntersectionObserver`/`visibilitychange`/mudança de workspace para pausar; renderização *on-demand* (invalidar → 1 frame) quando nada muda (economia real de bateria/GPU); throttle de IPC; logging estruturado com níveis.

### 🟠 P1-11 — Configurações do Studio não persistem; DPI ignora `devicePixelRatio`
`SettingsModal` nunca recebe `initialSettings` (`App.svelte:2894-2900`) e não grava em disco/`localStorage` → a cada boot voltam os defaults (inclusive AA). `dpiMultiplier` é um multiplicador absoluto (1,0/1,5/2,0) que **ignora o `devicePixelRatio` real**: num notebook 2×, "1.0x" renderiza em resolução CSS → imagem borrada; e `setDpiScale` usa `cssWidth/cssHeight` **armazenados** (potencialmente stale) em vez de `canvas.clientWidth`. **Correção:** persistência de settings (arquivo de configuração do Studio via Tauri, com schema e migração), modo "Auto (DPR)" como default, `resize()` lendo dimensões reais, e aviso quando a resolução efetiva exceder o limite do adaptador.

### 🟠 P1-12 — Zero internacionalização nos painéis de Shading/Iluminação
Existem 171 chaves por idioma e `t()` é usado em toolbar/status/settings, mas **nenhuma chave** cobre os painéis auditados: "TOON RAMP", "Corte", "Suavidade", "BANDAS (STEPS)", "BRILHO", "CORES BASE", "RIM LIGHT", "CONTORNO", "HUE SHIFT", "PRESETS HARMONIA", "CALIBRAÇÃO", "RESUMO", "PRESETS NPR", "DIREÇÃO", "Azimute", "Elevação", "Cor do Sol", "Matiz", "Saturação", nomes dos 12 presets, além dos textos de `LightingControls.svelte`. Usuários `en`/`ja` veem metade do aplicativo em português. **Correção:** extrair 100% das strings para chaves (`shading.*`, `light.*`, `preset.*`), completar `en_US`/`ja_JP`, e adicionar teste de paridade de chaves entre os 3 dicionários + teste estático que falha se houver texto literal em `{#if activeTool}` blocks.

### 🟠 P1-13 — Presets hardcoded inline e unidades de contorno convertidas por heurística em 3 lugares
Os 12 presets (4 solares, 4 de sombra, 4 NPR) são blocos `onclick={() => { hueShift = -18; shadowSaturation = 1.25; ... }}` no template (`App.svelte:2155-2360`), com números mágicos, sem i18n, sem persistência, sem ícone/preview e sem descrição de undo específica. A espessura de contorno é convertida por heurísticas distintas: `renderer.setOutlineWidth` faz `width > 0.05 ? width * 0.001 : width` (`:1127`), `updateMaterial` faz `outlineWidth * 0.001` (`:1271`), e o handler MCP faz `p.outline_width > 0.05 ? p.outline_width : p.outline_width * 1000` (`App.svelte:841`). **Correção:** catálogo tipado de presets (`src/services/shading_presets.ts` com `id`, `labelKey`, `preview`, `profile: ShadingProfile`, `lightRig: LightRig`), aplicação atômica com uma única entrada de histórico ("Aplicar preset Ghibli Suave"), persistência de presets do usuário; **uma** unidade canônica para contorno (milímetros de tela ou fração de NDC) com conversão feita **uma única vez** na fronteira UI↔motor e teste de round-trip.

### 🟠 P1-14 — Sem TypeScript estrito, sem lint, sem CI, e a suíte de testes não testa o produto
Não existe `tsconfig.json` (logo `strict` nunca é aplicado), não existe ESLint/Prettier configurado, não existe `.github/` (nenhum workflow), não há `test` em `package.json` e nenhum framework de teste frontend; há 26 `any` explícitos (`viewportRef: any`, `handleMetrics(m: any)`, `assetBrowserRef: any`, `accData: any`, 10 `event: any` no Viewport). A suíte `tests/e2e/workspace_pipeline.test.ts` (27 testes verdes) valida um **`StudioPipelineHarness` auto-contido** cujos clamps (`headScale [0.5,2]`, `shadowThreshold [0,1]`, `lightElevation [0,90]`, `cameraFov [10,120]`) **não existem no aplicativo real**, e cujo contrato (`inspectorProperties: ["rimColor","toonBands","shadowSoftness","shadowBias","ambientSkyColor","ambientGroundColor"]`, `outlineWidth: 0.015`, ícone `SlidersIcon` para `shader_ball`) **diverge do código** (App usa `LayersIcon`, `outlineWidth: 3.5`). Testes verdes sem tocar no produto = falsa garantia de qualidade. **Correção:** `tsconfig.json` com `strict: true` (+ `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`) e `svelte-check` no CI com zero erros; ESLint + Prettier; Vitest para unidades (cor, rampa, packing de vértices, math de câmera, validação de schema); Playwright para E2E real (carregar o app, mover slider, ler pixel do canvas); `cargo test/clippy/fmt` no CI; GitHub Actions com gates; reescrever a suíte atual para importar o código real (ou deletá-la e substituir por testes que exercitam o produto).

### 🟠 P1-15 — Vazamento de recursos em `destroy()` e raycast tátil quebrado quando DPI ≠ 1
`destroy()` (`:2531-2548`) libera 6 buffers + depth + rampa, mas **não**: pipelines (`celPipeline`, `outlinePipeline`, `morphPipeline`), bind groups, `morphHeaderBuffer/morphBaseBuffer/morphDeltasBuffer/morphChannelsBuffer/morphedVertexBuffer`, `toonRampSampler`, `device.destroy()`, `context.unconfigure()`, nem os recursos GL (`glVao/glVbo/glIbo`, `deleteProgram`). Em Svelte, trocar de workspace/remontar o componente vaza VRAM e contexts (a Regra Inviolável nº 2.3 exige "ausência de leaks de VRAM"). Separadamente, `raycastTactile` (`:2507`) e `projectTactileDelta` (`:2521`) passam `canvas.width/height` (**pixels de dispositivo**) para funções que recebem `screenX/screenY` em **pixels CSS** (vindos de `getBoundingClientRect`) → com `dpiMultiplier` 1,5 ou 2,0 o raio é calculado com NDC errado e a manipulação tátil seleciona o segmento anatômico errado. **Correção:** `destroy()` completo e idempotente (com `Device.destroy()` e `unconfigure`), contadores de recursos em modo debug, teste de montagem/desmontagem repetida com `device.limits`/heap estável; converter coordenadas de ponteiro para o espaço do backing store em **um** único lugar (`toCanvasSpace(cssX, cssY)`) usado por raycast, zoom-at-cursor e pan.

---

## 7. ACHADOS MÉDIOS (P2)

| # | Achado | Evidência | Correção |
| :-- | :--- | :--- | :--- |
| P2-01 | **Contorno com depth bias 0,0 e depth write ligado** → backfaces extrudadas coplanares passam no `less-equal` e **sangram sobre a superfície** (z-fighting/escurecimento interno); o export usa 0,0003 fixo | `webgpu_renderer.ts:180,550-556,579-585`, `App.svelte:354`, `inverted_hull.wgsl:54` | Bias default calibrado + `depthWriteEnabled: false` no passe de contorno (ou `depthBias`/`slopeScaledDepthBias` do pipeline) + ordem de passes idêntica nos 2 motores |
| P2-02 | **Ordem dos passes invertida** entre viewport (cel→outline) e export (outline→cel) → oclusões de silhueta diferentes | `webgpu_renderer.ts:2300-2310` × `headless.rs:809-825` | Definir a ordem canônica no `RenderConfig` compartilhado (outline primeiro, sem depth write, é o padrão da indústria) |
| P2-03 | **Blend habilitado no passe opaco** e `alphaMode: "premultiplied"` sem pré-multiplicar no shader → custo de banda e composição incorreta se `base_color.a < 1` | `webgpu_renderer.ts:540-546`, `:230-236`, `cel_shading.wgsl:212` | Desligar blend para geometria opaca; se houver alpha, pré-multiplicar explicitamente e usar `alphaMode` coerente |
| P2-04 | **"Luz Ambiente" não é luz ambiente**: só escala a cor da sombra, com piso 0,2 e teto 1,5 inalcançável (máx 1,4) | `cel_shading.wgsl:172-173`; simulação §Anexo A.6 | Termo ambiente real: `ambient = skyColor·hemisphere(N) + groundColor·(1−hemisphere)` somado ao cel, com intensidade 0–2 sem piso mágico |
| P2-05 | **AO multiplicado no fim** (escurece também especular e rim) e sem influência no limiar; o asset só tem 2 níveis (0,95/1,0) | `cel_shading.wgsl:156,210` | AO aplicado ao termo difuso/ambiente (não ao highlight), com intensidade configurável; considerar AO no threshold para sombras de contato |
| P2-06 | **Jitter especular com constantes mágicas** (`y*35 + uv.x*20 + offset*10`, `sin()*0.08`) → banding dependente do eixo Y do mundo, não controlável, e **diferente no GLSL** (`v_pos.x`) | `cel_shading.wgsl:190-191` × `webgpu_renderer.ts:876-877` | Substituir por *highlight mask* autorado (textura/canal) ou por anel anisotrópico paramétrico (posição, largura, intensidade) exposto na UI; unificar nos 2 backends |
| P2-07 | **`spec_cutoff` deriva de `spec_intensity`** (`0.65 − 0.12·i`) → um controle altera tamanho **e** posição do highlight (acoplamento não documentado) | `cel_shading.wgsl:194` | Separar `spec_size` (expoente/cutoff) de `spec_intensity` (ganho), ambos explícitos no material |
| P2-08 | **Métricas enganosas**: `frameTimeMs` mede só o CPU submit; `drawCalls` fixo em 2; `triangles` inclui o pedestal; `adapter.info.device` está depreciado na spec | `webgpu_renderer.ts:2380-2400` | Timestamps GPU (`GPUQuerySet` "timestamp") com fallback para delta de rAF; contagem real de draw calls/triângulos por objeto; `adapter.info` com feature-detect |
| P2-09 | **Frame pacing por skip de rAF** (`elapsed < interval − 1.0`) → jitter e tearing perceptível; `setFpsCap` não reseta `lastFrameTimestamp` | `webgpu_renderer.ts:2179-2195` | Usar `presentMode` + limite por timestamp de apresentação; resetar o timer ao mudar o cap; oferecer modo "ilimitado/vsync" explícito |
| P2-10 | **Double-render em frames de resize** (`frame → checkAndApplyResize → resize → render` e depois `render`) | `webgpu_renderer.ts:1924-1926`, `2188-2192` | `resize()` apenas invalida; o loop renderiza uma vez |
| P2-11 | **`setVsync` engole falha** de `presentMode: "immediate"` (não suportado em várias plataformas) em `catch(_){}` → o toggle parece funcionar e não funciona | `webgpu_renderer.ts:1940-1955` | Consultar capacidades, degradar com aviso na UI, logar; nunca `catch` vazio |
| P2-12 | **Loader GLB ingênuo**: sem `resp.ok`, sem validar magic/version/`chunk0Type`/`chunk1Type` (lidos e ignorados → variáveis mortas), só 1 primitive/1 mesh, `componentType` de cor/normal assumido 5126, sem cache/abort/progresso, `joints/weights` **descartados** (rig de 18 juntas autorado e inutilizado), erro só em `console.error` | `webgpu_renderer.ts:1687-1795` | Loader validado e completo (multi-mesh/primitive, 5121/5123/5126 normalizados, skins, materiais, `extras`), com `AbortController`, cache por URL, progresso, e erros tipados surfacados na UI |
| P2-13 | **Regiões anatômicas por faixas de índice mágicas** (`425/544/1069/1444/2536/4070`) com âncoras Y **masculinas** aplicadas ao mesh feminino (male `y∈[0;1,795]`, female `y∈[0;1,655]`, mesmas faixas de índice) → morphs femininos caem fora da anatomia (ex.: `forehead_height` exige `y>1,64`, que cobre 0,155 m da cabeça masculina e **0,015 m** da feminina) | `webgpu_renderer.ts:1386-1672` + parse binário dos GLB | Zonas nomeadas no próprio asset (`extras.anigo_zones` ou atributo de vértice com ID de zona + âncoras normalizadas por altura/landmarks), eliminando índices mágicos e escalando por gênero |
| P2-14 | **Sem matriz de modelo**: `SceneNode.transform` (posição/rotação/escala) é **ignorado** nos dois motores; o vertex shader trata espaço de objeto como mundo (`out.world_position = in.position`) | `cel_shading.wgsl:65-72`, `headless.rs` (sem model matrix), `scene.rs:77` | Adicionar `model`/`normal_matrix` ao `CameraUniform` (ou UBO por objeto), aplicar em posição e normal; habilita cenários, múltiplos objetos e rotação do modelo contra a luz |
| P2-15 | **`index.html lang="en"`** com app pt-BR; `console.log` de marketing ("initialized successfully at 120+ FPS"); 6 `alert/confirm/prompt` nativos em vez de diálogos do Studio; textos de erro em português hardcoded no Rust | `index.html:2`, `webgpu_renderer.ts:245`, `App.svelte:569,618,2922,2934`, `main.rs:275,283` | `lang` dinâmico via i18n; logging estruturado (nível + contexto); diálogos próprios do Studio; mensagens de erro localizadas e tipadas |
| P2-16 | **Sem estados de UI para loading/erro/vazio** no viewport (carregamento do GLB, adapter indisponível, contexto perdido, shader inválido) | `webgpu_renderer.ts:211-262`, `Viewport.svelte:340-360` | Overlays de estado (skeleton/spinner, painel de erro com diagnóstico copiável, empty state com CTA), i18n |

---

## 8. MELHORIAS DE ESTADO DA ARTE (P3) — diferencial competitivo

| # | Melhoria | Referência de indústria | Valor |
| :-- | :--- | :--- | :--- |
| P3-01 | Pipeline HDR + tonemap estilizado + exposição/white balance | UE5 / DaVinci | headroom de luz, presets que funcionam, export consistente |
| P3-02 | Shadow map anime (hard/soft) + contact shadow + shadow catcher | Genshin Impact, Guilty Gear Strive | sombras projetadas reais, personagem integrado ao chão |
| P3-03 | SDF face shadow + ângulo de luz facial dedicado (`face_sdf.wgsl`, MCP `anigo_set_face_light_angle`) | Genshin (SIGGRAPH/GDC) | sombra de nariz/bochecha sem quebra em 360° (DoD Sprint 05) |
| P3-04 | Editor de Toon Ramp (curva 1D/2D, import/export PNG, presets por material) | VRoid Studio / Unity URP Toon | controle artístico real sobre a assinatura visual |
| P3-05 | Light rigs nomeados (key/fill/rim/back/kicker) + gizmo 3D de luz arrastável no viewport + disco solar | Blender 4 / Maya | edição de luz direta e WYSIWYG; fim dos sliders de azimute/elevação às cegas |
| P3-06 | Ambiente hemisférico céu/chão + IBL estilizado + HDRI anime | Unreal / Marmoset | iluminação global coerente (Sprint 18) |
| P3-07 | Materiais por objeto/malha + biblioteca de materiais (hoje: 1 material global para tudo) | Substance/VRM 1.0 MToon | pele/cabelo/olho/tecido com shading próprio — pré-requisito de qualidade anime |
| P3-08 | Pós-processamento: bloom, DoF, aberração cromática sutil, grain, color grading LUT | DaVinci Resolve | acabamento de frame final |
| P3-09 | Split-view A/B + overlay de referência + "safe areas" | ferramentas de calibração de estúdio | calibrar shading contra referência de produção |
| P3-10 | Color picker profissional (OKLCH, harmonias, eyedropper, swatches persistidos) | Figma / Blender | precisão e reuso de paleta |
| P3-11 | Passes de render isolados (beauty/line/shadow/depth/normal/mask) + EXR/PNG 16-bit + fila de render | Nuke/After Effects workflow | integração com pipeline de estúdio (Sprint 22) |
| P3-12 | GPU-driven morph + skinning no viewport (religar o compute existente) e timestamps GPU | Unreal Nanite-style budgeting | 120 fps reais com 157 sliders ativos |
| P3-13 | Regressão visual automatizada (golden images + MSE/PSNR via `anigo_compare_baseline`) no CI | padrão de motores AAA | impede que qualquer um dos bugs acima volte |

---

## 9. PLANO DE CORREÇÃO (6 fases, com dependências e esforço)

> Estimativas em dias-homem de engenharia sênior, já incluindo testes. Fase 0 e 1 são **bloqueadoras de release**.

### Fase 0 — Hotfix de integridade visual (2–3 dias) · *dependência: nenhuma*
1. Ler `_ANIGO_COLOR` (com fallback `COLOR_0`) e usar neutro canônico `[1; 0,5; 1; 1]` — **P0-01**.
2. Separar `shadeColor` × `shadowTint` (tint default branco) — **P0-02**.
3. Remover a normalização destrutiva de intensidade (clamp temporário em 0–2 **com curva monotônica**, até a Fase 3 trazer HDR) — **P0-03**.
4. Unificar depth bias/opacidade do outline e ordem de passes entre TS e Rust — **P0-04 (parcial)**, **P2-01/02**.
5. Ligar `rimColor` no shader (novo campo no `MaterialUniform`) e remover/ligar `outlineSmoothness` — **P0-09**.
6. Corrigir as coordenadas do raycast tátil (CSS→backing store) — **P1-15 (parcial)**.
7. Remover a telemetria fabricada do App — **P0-08**.

**Gate de saída:** a imagem do viewport muda de forma mensurável e correta para cada um dos 27 controles; `Corte=0.50` coloca o terminador em `N·L=0`; cor de sombra escolhida = cor renderizada (Δ ≤ 1/255).

### Fase 1 — Fonte única de verdade: dados, shaders e estado (5–7 dias) · *depende da Fase 0*
1. `shaders/` canônico; imports `?raw` no Vite; `include_str!` no Rust apontando para o mesmo arquivo; deletar a cópia em `crates/anigo-renderer/shaders/` — **P0-04**.
2. Tabela de rampa gerada de uma especificação única (JSON) + teste dos 1.024 valores — **P0-04**.
3. `RenderConfig` compartilhado (formato de cor, depth, passes, MSAA, background, ordem) — **P0-04/05**.
4. `ShadingProfile` + `LightRig` + `CameraState` + `AnigoProject v1` (Anexo B) usados por UI, histórico, projeto, autosave, IPC e MCP; validação de schema com clamp canônico em **uma** tabela — **P0-10, P1-08, P1-13**.
5. Unificar o consumo de eventos de bridge (um só handler + normalizador) — **P1-07**.
6. Completar `StylizedMaterial`/`StylizedLight`/`LiveWindowState` no Rust; remover defaults fabricados — **P0-05, P0-08**.
7. `tsconfig.json` strict + `svelte-check` + ESLint/Prettier + GitHub Actions (build, typecheck, `cargo test/clippy/fmt`, vitest) — **P1-14**.

**Gate de saída:** zero duplicação de shader; os 27 parâmetros round-trip em UI→projeto→undo→IPC→MCP→export; CI vermelho se qualquer parâmetro não propagar.

### Fase 2 — Paridade viewport ↔ export e robustez de inicialização (4–6 dias) · *depende da Fase 1*
1. Export consumindo a **mesma** cena/malha/câmera/material do viewport (ou export via canvas como ponte) — **P0-05**.
2. Painel *Render* funcional (resolução, escala, passes, formato, destino, preview, A/B) — **P0-05**.
3. Probe de capacidades + fallback real + `getCompilationInfo` + error scopes + `device.lost` com recovery + UI de diagnóstico — **P0-06**.
4. `destroy()` completo/idempotente + teste de montagem repetida sem vazamento — **P1-15**.
5. Pause/resume + render on-demand + throttle de IPC — **P1-10**.
6. Loader GLB validado e completo (multi-primitive, componentTypes, skins, cache, abort, progresso, erros tipados) — **P2-12**.

**Gate de saída:** `MSE(viewport, export) < 1,0` na cena de referência com parâmetros não-default; falha de WebGPU produz fallback funcional + diagnóstico visível; nenhum leak em 50 ciclos de montagem.

### Fase 3 — Qualidade de imagem: cor, AA, HDR (6–9 dias) · *depende da Fase 1*
1. MSAA 4x (+ resolve) no WebGPU e no Rust; FXAA/SMAA como pós-passe; aplicar a setting e persistir — **P0-07, P1-11**.
2. Pipeline HDR (`rgba16float`) + tonemap estilizado + exposição/white point — **P0-03**.
3. Gestão de cor sRGB↔linear de ponta a ponta (entrada, aritmética, saída) — **P1-01**.
4. Hue-shift em OKLab/OKLCH com faixa única e clamp — **P1-02**.
5. Modelo de bandas unificado (fim do crossfade textura×analítico) — **P1-03**.
6. Recálculo de normais pós-deformação + normais de contorno (canal dedicado) — **P0-11, P1-06**.
7. Performance: deformação fora da main thread (Worker/GPU), buffers persistentes, religar o compute de morphs com a base correta — **P1-09**.

**Gate de saída:** varredura de intensidade monotônica em 0–3; ΔE < 1 entre cor escolhida e renderizada; redução ≥ 70% de pixels serrilhados com MSAA; terminador de sombra geometricamente correto após morphs extremos; ≤ 4 ms de CPU/frame durante drag.

### Fase 4 — Sistema de iluminação de nível produção (10–15 dias) · *depende da Fase 3*
1. Sombra projetada estilizada (shadow map + bias + filtragem anime) e shadow catcher — **P1-05, P3-02**.
2. SDF facial (`shaders/face_sdf.wgsl`) + bake + MCP `anigo_set_face_light_angle` — **P1-05, P3-03** (DoD Sprint 05).
3. Ambiente hemisférico céu/chão + IBL estilizado; "Ambiente" vira luz ambiente real — **P2-04, P3-06**.
4. Light rigs (key/fill/rim/back/kicker) + gizmo 3D arrastável + disco solar — **P3-05**.
5. Editor de rampa toon + assets de rampa versionados — **P1-04, P3-04**.
6. Materiais por objeto + matriz de modelo + biblioteca de materiais — **P2-14, P3-07**.
7. Passes isolados + EXR/16-bit — **P3-11**.

### Fase 5 — Testes, i18n, documentação e regressão visual (5–7 dias, paralelo)
1. Substituir a suíte mock por testes que exercitam o produto (Vitest + Playwright com leitura de pixel do canvas) — **P1-14**.
2. Harness de paridade: mesmo perfil de shading → render TS e render Rust → MSE/PSNR no CI — **P0-04/05**.
3. Golden images versionadas em `baselines/shading/` + `anigo_compare_baseline` como gate — Regra Inviolável nº 1.
4. i18n completo dos painéis + teste de paridade de chaves — **P1-12**.
5. Catálogo de presets tipado, i18n, persistível, com preview — **P1-13**.
6. `docs/shading_model.md` (equações, canais de vértice, faixas canônicas, unidades) e `docs/lighting_rig.md`.
7. Protocolo de validação visual via MCP documentado e executado (frames de prova inspecionados criticamente: silhueta, terminador, AA, normais) — Regra Inviolável nº 1.

---

## 10. ESTRATÉGIA DE TESTES OBRIGATÓRIA PARA ESTA WORKSPACE

| Camada | O quê | Ferramenta | Gate |
| :-- | :--- | :--- | :--- |
| Unit (TS) | `hexToRgb/rgbToHex` round-trip; sRGB↔linear; OKLCH hue-shift; geração da rampa; packing de vértices (offsets 0/12/24/32/48/56, stride 72); math de câmera/orbit/zoom/pan; validação de schema e clamps | Vitest | ≥ 95% de cobertura dos módulos de shading |
| Unit (Rust) | `MaterialUniform`/`LightUniform` (tamanho/offsets/round-trip); loader GLB (`_ANIGO_COLOR`, componentTypes); rampa idêntica à TS; `StylizedMaterial` completo | `cargo test` | 0 `unwrap` em caminho crítico; `clippy -D warnings` |
| Contract | Tabela canônica de faixas/unidades compartilhada TS↔Rust↔MCP; teste que falha se um parâmetro existir na UI e não no schema (e vice-versa) | Vitest + `cargo test` | 27/27 parâmetros nos 2 lados |
| Parity | Mesmo `ShadingProfile` → frame do viewport (Playwright + `readPixels`) × frame do Rust → MSE/PSNR/diff por canal | Playwright + `anigo_compare_baseline` | MSE < 1,0; PSNR > 45 dB |
| Golden | 12 imagens de referência (4 presets de luz × 3 bandas) em `baselines/shading/` | `anigo_compare_baseline` | sem regressão não intencional |
| Visual MCP | Frames de prova inspecionados criticamente (silhueta, terminador, AA, normais, contorno) com relatório descritivo | `anigo-mcp` (Regra Inviolável nº 1) | obrigatório para concluir a sprint |
| E2E UI | Mover cada slider e verificar mudança de pixel na região esperada; presets; undo/redo; save/load; troca de workspace | Playwright | 0 controles mortos |
| Resiliência | WGSL inválido, adapter ausente, `device.lost`, contexto perdido, GLB 404/corrompido, projeto inválido, hex inválido | Vitest + Playwright (mocks) | nunca tela preta silenciosa |
| Performance | FPS, frame time GPU, CPU por frame durante drag, alocações de buffer GPU, VRAM após 50 ciclos de montagem | Playwright + `GPUQuerySet` | 120 fps em referência; 0 alocação/frame em regime; 0 leak |

---

## 11. DEFINITION OF DONE — WORKSPACES SHADING & ILUMINAÇÃO

A workspace só pode ser declarada pronta quando **todos** os itens abaixo forem verdadeiros e demonstrados por evidência (teste automatizado ou frame MCP inspecionado):

1. Os **27 controles** produzem efeito observável, monotônico e documentado; **zero controles mortos**.
2. A cor escolhida pelo artista é a cor renderizada (Δ ≤ 1/255 em condição neutra); a intensidade da luz responde em toda a faixa.
3. Os 4 canais de vertex color autorados (`_ANIGO_COLOR`) são lidos e aplicados; o neutro canônico é `[1; 0,5; 1; 1]`.
4. Existe **uma única** fonte de shader; viewport, fallback e export produzem a mesma imagem (MSE < 1,0).
5. Anti-aliasing funcional e configurável (MSAA 4x default) nos dois motores — silhueta e contorno sem serrilhado.
6. Pipeline com gestão de cor correta (linear + tonemap + sRGB) e hue-shift perceptual (OKLCH).
7. Undo/redo, save, autosave, IPC e MCP cobrem 100% dos parâmetros, com schema versionado, validado e migrável.
8. Falhas de GPU/shader/asset produzem diagnóstico visível e degradação graciosa — **nunca** tela preta silenciosa.
9. Sem vazamento de VRAM/contexts em 50 ciclos de montagem; sem alocação de buffer GPU por frame em regime.
10. TypeScript estrito sem erros, lint limpo, CI verde (build + typecheck + vitest + cargo test/clippy/fmt + parity + golden).
11. i18n completo (pt-BR/en/ja) dos painéis e presets, com teste de paridade de chaves.
12. Documentação técnica do modelo de shading (equações, canais, faixas, unidades) e do light rig.
13. Validação visual via MCP executada e relatada (Regra Inviolável nº 1), com baselines versionadas.

---

## 12. ANEXOS

### Anexo A — Números brutos das simulações (reproduzíveis)

**A.1 Paridade da tabela de rampa toon (TS × Rust), 256 texels por linha**
```
linha 0 (contínuo)  : 0/256 divergentes   níveis TS 0..255        níveis RS 0..255
linha 1 (1 degrau)  : 0/256 divergentes   níveis TS {0,255}       níveis RS {0,255}
linha 2 (2 degraus) : 81/256 divergentes  níveis TS {0,128,255}   níveis RS {0,102,255}   |Δ|máx 127
                      cortes TS em u=0.35/0.65 (cols 89/166) × cortes RS cols 85/135
linha 3 (3 degraus) : 76/256 divergentes  níveis TS {0,89,179,255} níveis RS {0,77,179,255} |Δ|máx 90
                      cortes TS cols 64/128/191 × cortes RS cols 64/120/180
```

**A.2 Limiar de sombra efetivo (fallback de vertex color)**
```
G autorado  = 0.50 (corpo) / 0.48 (cabeça, 425 vértices)  → shift 0.000 / −0.006
G do loader = 1.00 (fallback COLOR_0 ausente, 4.070 vértices) → shift +0.150
UI "Corte" 0.50 → efetivo 0.650  (Δ = +0.150 em half-Lambert ≈ 27.0° de arco N·L)
UI faixa 0.05–0.95 → efetiva 0.20–1.10; half_lambert ≤ 1.0 ⇒ 9.5% superiores do slider mortos
```

**A.3 Cor de sombra: escolhida × renderizada (defaults reais da UI)**
```
escolhida            #9995be = rgb(0.600, 0.584, 0.745)   L = 0.599
shade×light (c²)     #5c578e = rgb(0.360, 0.341, 0.555)
+ hue −15°, sat 1.15         rgb(0.309, 0.349, 0.555)
× ambient_term 0.480 #262b44 = rgb(0.148, 0.168, 0.266)   L = 0.171  →  28.5% da escolhida
com shade_color neutro (branco): rgb(0.269, 0.282, 0.358)  →  comportamento esperado
```

**A.4 Resposta do slider de intensidade (base `#faeae0` × sol `#fff8e7`)**
```
I = 0.50 → rgb(0.490, 0.446, 0.398)   max 0.490
I = 1.00 → rgb(0.980, 0.892, 0.796)   max 0.980
I = 1.02 → rgb(1.000, 0.910, 0.812)   max 1.000  ← ponto de saturação
I = 1.20 → rgb(1.000, 0.910, 0.812)   idêntico
I = 1.40 → rgb(1.000, 0.910, 0.812)   idêntico   (preset "Golden Hour")
I = 2.00 → rgb(1.000, 0.910, 0.812)   idêntico
I = 3.00 → rgb(1.000, 0.910, 0.812)   idêntico
```

**A.5 Peso do crossfade rampa-textura × analítico (`clamp((s−0.015)·15,0,1)`)**
```
s = 0.001 → textura 100.0% / analítico  0.0%
s = 0.015 → textura 100.0% / analítico  0.0%
s = 0.020 → textura  92.5% / analítico  7.5%   ← default da UI
s = 0.050 → textura  47.5% / analítico 52.5%
s = 0.082 → textura   0.0% / analítico 100.0%
s = 0.250 → textura   0.0% / analítico 100.0%
```

**A.6 Termo "ambiente" (`clamp(0.2 + a·0.8, 0.05, 1.5)`)**
```
a = 0.00 → 0.200 (piso hardcoded: sombra nunca fica preta)
a = 0.35 → 0.480 (default)
a = 1.00 → 1.000
a = 1.50 → 1.400 (o clamp superior 1.5 é inalcançável)
```

**A.7 Malha canônica (parse binário dos GLB)**
```
male   : 4.070 vértices, 20.640 índices (6.880 triângulos), y ∈ [0.000, 1.795], 18 juntas, weights Σ=1.000
female : 4.070 vértices, 20.640 índices (6.880 triângulos), y ∈ [0.000, 1.655], 18 juntas, weights Σ=1.000
atributos: POSITION, NORMAL, TEXCOORD_0, JOINTS_0, WEIGHTS_0, _ANIGO_COLOR   (COLOR_0 ausente; skins: 0)
faixas de índice usadas como "zonas" no TS: 0–425 cabeça · 425–544 pescoço · 544–1069 torso ·
  1069–1444 pélvis · 1444–2536 braços · 2536–4070 pernas  (idênticas para male e female,
  embora a altura difira 7.8% → âncoras Y masculinas aplicadas ao mesh feminino)
telemetria declara: 156 triângulos   ← 44× menor que o real
```

### Anexo B — Modelo de dados canônico proposto (fonte única de verdade)

```ts
// src/engine/shading/types.ts  — espelhado 1:1 em crates/anigo-core/src/shading.rs
export interface RgbLinear { r: number; g: number; b: number }          // sempre linear, 0..1 (HDR permitido >1)
export interface RgbaLinear extends RgbLinear { a: number }

export type ToonBands = 0 | 1 | 2 | 3;                                   // 0 = gradiente contínuo
export type ShadeBlend = "multiply" | "replace" | "lerp";
export type AntiAliasing = "none" | "msaa2x" | "msaa4x" | "fxaa" | "smaa";

export interface ShadingProfile {
  version: 1;
  baseColor: RgbaLinear;
  shadeColor: RgbaLinear;              // tinta de sombra do MATERIAL (P0-02)
  bands: ToonBands;
  shadowThreshold: number;             // 0.00..1.00 (posição do terminador em half-Lambert)
  shadowSoftness: number;              // 0.000..0.250 (largura da penumbra — sem crossfade, P1-03)
  rampId: string;                      // asset de rampa (P1-04)
  hueShiftDeg: number;                 // −180..+180 em OKLCH (P1-02)
  shadowSaturation: number;            // 0.00..2.50
  shadeBlend: ShadeBlend;
  aoIntensity: number;                 // 0.00..1.00 (P2-05)
  specular: { color: RgbaLinear; intensity: number; size: number; softness: number; offsetY: number };
  rim:        { color: RgbaLinear; intensity: number; spread: number };   // P0-09
  outline:    { color: RgbaLinear; widthPx: number; opacity: number; smoothness: number;
                depthBias: number; useOutlineNormals: boolean };           // P1-06
}

export interface LightRig {
  version: 1;
  key:   { azimuthDeg: number; elevationDeg: number; intensity: number; color: RgbLinear;
           shadowTint: RgbLinear; castShadow: boolean };                  // P0-02 / P1-05
  fill?: { azimuthDeg: number; elevationDeg: number; intensity: number; color: RgbLinear };
  rim?:  { azimuthDeg: number; elevationDeg: number; intensity: number; color: RgbLinear };
  ambient: { intensity: number; skyColor: RgbLinear; groundColor: RgbLinear };   // P2-04
  exposure: number;                    // EV, −3..+3 (P0-03)
  tonemap: "aces" | "reinhard" | "linear";
  background: RgbaLinear;              // P2-14 / P0-05
}

export interface CameraState { eye: Vec3; target: Vec3; up: Vec3; fovDeg: number; near: number; far: number; focalMm: number }

export interface RenderSettings { resolution: [number, number]; scale: number; antiAliasing: AntiAliasing;
                                  samples: 1|2|4|8; passes: RenderPassId[]; format: "png8"|"png16"|"exr" }

export const PARAM_RANGES = { /* única tabela de faixas, consumida por UI, validador, IPC e MCP (P1-08) */ } as const;
```

**Layout de uniform proposto (alinhamento 16 B, idêntico em WGSL e `#[repr(C)]`):**
```
CameraUniform  : view_proj mat4x4 (64) | camera_pos vec4 (16) | model mat4x4 (64) | normal_mat mat3x3→vec4x3 (48)   = 192 B
LightUniform   : direction·intensity (16) | color·ambient (16) | shadow_tint·saturation (16)
                 | fill_dir·intensity (16) | rim_dir·intensity (16) | ambient_sky·exp (16) | ambient_gnd·tonemap (16) = 112 B
MaterialUniform: base (16) | shade (16) | spec_color (16) | rim_color (16) | outline_color (16)
                 | params  (threshold, softness, spec_intensity, spec_size) (16)
                 | params2 (rim_intensity, rim_spread, hue_shift_rad, bands) (16)
                 | params3 (spec_softness, spec_offset, ao_intensity, shade_blend) (16)
                 | params4 (outline_width, outline_opacity, outline_smoothness, outline_depth_bias) (16) = 160 B
```

### Anexo C — Checklist de verificação por achado (usar no PR de correção)

```
[ ] P0-01 _ANIGO_COLOR lido; neutro [1;0.5;1;1]; teste lê o GLB e confere os 4 canais
[ ] P0-02 shadeColor ≠ shadowTint; tint default branco; Δ ≤ 1/255 na cor de sombra
[ ] P0-03 intensidade monotônica em 0..3; HDR + tonemap; presets distinguíveis
[ ] P0-04 1 fonte de shader; rampa idêntica TS/Rust; RenderConfig compartilhado; lint contra shader inline
[ ] P0-05 MSE(viewport, export) < 1.0; painel Render funcional; cena única
[ ] P0-06 probe + fallback real + compilationInfo + errorScope + device.lost + UI de erro
[ ] P0-07 MSAA 4x nos 2 motores; setting aplicada e persistida; −70% de pixels serrilhados
[ ] P0-08 telemetria única e real; defaults sentinelas; stale detectado; ≤ 4 IPC/s
[ ] P0-09 rimColor ligado; outlineSmoothness ligado (ou removido); 4 bandas acessíveis; teste anti-controle-morto
[ ] P0-10 schema v1; 27/27 parâmetros em undo/save; câmera real; validação sem NaN
[ ] P0-11 normais recalculadas; normais de contorno; teste de ortogonalidade
[ ] P1-01..P1-15 e P2-01..P2-16 conforme §6/§7
[ ] CI verde: typecheck strict, lint, vitest, cargo test/clippy/fmt, parity, golden
[ ] Validação visual via MCP executada e relatada (Regra Inviolável nº 1)
```

---

### Nota final

Nenhum dos defeitos acima é de difícil solução isoladamente — a maioria é **consequência de uma única decisão arquitetural**: ter quatro implementações paralelas do mesmo motor de shading (WGSL arquivo, WGSL inline, GLSL inline, Rust headless) e quatro fontes de estado (App, renderer, cena Rust, cena MCP), sem contrato compartilhado e sem testes que atravessem essas fronteiras. **Enquanto essa duplicação existir, cada correção aplicada em um lugar criará uma nova divergência em outro.** Por isso a Fase 1 (fonte única de verdade) é o investimento de maior retorno do plano: ela torna as Fases 2–5 mecânicas e permanentes.

*Relatório gerado em 2026-09-20 · branch `arena/01a0c022-anigo` · 55 achados (11 P0 · 15 P1 · 16 P2 · 13 P3).*
