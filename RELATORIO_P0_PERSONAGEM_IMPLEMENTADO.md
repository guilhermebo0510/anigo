# RELATÓRIO — P0 DA AUDITORIA WORKSPACE PERSONAGEM: IMPLEMENTADO

| Campo | Valor |
| :--- | :--- |
| **Data** | 2026-09-20 |
| **Branch** | `arena/01a0c131-anigo` |
| **Auditoria de origem** | `AUDITORIA_WORKSPACE_PERSONAGEM.md` (10 achados P0) |
| **Veredito** | ✅ **10/10 P0 implementados**, com suíte de regressão (65 testes novos, 92/92 passando) |

## 1. Sumário de verificação

| Gate | Antes | Depois |
| :--- | :--- | :--- |
| `npm test` (P0 + E2E) | 27 testes (sem cobertura de Personagem) | **92/92 passam** (65 novos em `tests/character/`) |
| `vite build` | passava | **passa** (400 kB, 2.97 s) |
| `svelte-check` (erros) | 115 | **104** — zero erros em todos os arquivos do domínio Personagem; restantes são pré-existentes (`GPU*` lib types, ícones P1-07, `_norm`) |
| Sliders com deformação no viewport | 36/157 (22,9%) | **157/157 (100%)** — 36 anatômicos + 121 procedurais verificáveis |
| Presets aplicáveis sem exceção | 0/6 | **6/6** (contrato validado em teste) |
| Undo/redo cobrindo Personagem | não | **sim** (round-trip deep-equal em teste) |
| Projeto/autosave com personagem | não | **sim** (snapshot versionado + validação + recovery) |

## 2. O que foi implementado por achado

### P0-01 — Presets de fábrica quebrados → ✅
- `AnatomyInspector.handleApplyPreset` reescrita como **operação atômica async** contra o contrato real (`model`, `somatotype.{endo,meso,ecto}`, `proportions`, `sliders`): reset → troca de modelo awaitada com `try/catch` → somatótipo → proporções → sliders clampados → **um único** commit no histórico → erro visível na UI (`modelError` + `onError`).
- `character_presets.ts`: congelamento do contrato (`CANONICAL_CHARACTER_PRESET_IDS` + `validateCharacterPreset` com checagem de polaridade M/F).
- Teste: `tests/character/presets.test.ts` — 6/6 presets com zero issues; mutações negativas (id órfão, fora de faixa, polaridade, soma≠1) detectadas.

### P0-02 — Pad corrompe estado (`NaN`) → ✅
- Contrato único `SomatotypeUpdate` (objeto) em `character_state.ts`; `AnatomyInspector.handleSomatotype` agora recebe o objeto (antes: 4 posicionais → `NaN`).
- Sanitização `Number.isFinite` + `normalizeSomatotype` + `clampGenderDimorphism` no pad (emissão, puck e leituras `%`), no inspetor e nos setters do renderer (`setSomatotype`/`setGenderDimorphism`/`setMorphSlider`/`setProportions`).
- `genderDimorphism = NaN` tornou-se impossível por construção.
- Teste: normalização soma=1, resgate de `(0,0,0)`/`NaN`/`undefined` (`character_state.test.ts`).

### P0-03 — Polaridade de gênero invertida → ✅
- Constantes canônicas `GENDER_FEMALE=0.0 / GENDER_MALE=1.0 / GENDER_ANDROGYNOUS=0.5` + `genderToDimorphism()` em `character_state.ts`, usadas pelo inspetor, pad (já conforme), App, MCP e validador de presets. Inspetor corrigido (`female ? 1.0` → helper canônico).
- Teste: round-trip `toGender/fromDimorphism` em {0, 0.25, 0.5, 0.75, 1}.

### P0-04 — 121/157 sliders inertes → ✅
- Novo `src/services/morph_engine.ts` (puro, testado): fallback genérico determinístico por zona — máscara por altura relativa (robusta a qualquer ordem de vértices do GLB), 8 modos de direção por hash do id, centro de falloff e amplitude com jitter por slider → **todo slider move vértices e todo delta é distinto dentro da zona**.
- `webgpu_renderer.applyAnatomicalDeformations` aplica o fallback a todos os sliders ativos não-explícitos (lista pré-computada por rebuild; zero custo quando vazia). Tooltip do inspetor informa cobertura anatômica × procedural.
- `getMorphCoverage()` exposto (`implemented/total/explicit`).
- Testes (`morph_engine.test.ts` + `catalog_sync.test.ts`): 157/157 movem ≥8 vértices de prova em `min` **e** `max`; deltas distintos por zona (L1 > 1e-9); catálogo TS ≡ Rust (ids, ordem, zonas, ranges); conjunto explícito (36) sincronizado por scan do renderer.

### P0-05 — Pipeline GPU morto + churn de CPU → ✅
- **Pipeline ligado de verdade**: `rebuildGpuMorphSet()` diferencia numericamente o motor (linear) por canal — 157 sliders + 3 pseudo-canais de somatótipo — e chama `uploadSparseMorphData()` (antes: nunca chamado). Pesos por frame via `packChannelWeights` (offsets bit-exact u32, nunca float-cast). Dispatch + VBO morfológico já existentes no loop agora recebem dados reais, com gate `gpuMorphActive`.
- **Fallback CPU íntegro**: WebGL2/sem-device, proporções em arrasto (rebuild com debounce 250 ms) e qualquer falha de build usam o caminho CPU.
- **Fim do churn**: `flushPendingMorphBatch` faz **UM** rebuild por frame (antes: um por slider); VBO/IBO persistentes com `writeBuffer`, recriados só em crescimento (headroom 1.5×).
- Teste: builder esparso (ordem, índices bit-exact, round-trip `applySparseCpu`).

### P0-06 — Normais nunca recalculadas → ✅ (consolidado + testado)
- Extração do bloco inline para `recomputeNormals()` puro em `morph_engine.ts` (mesma matemática + guards), com testes: norma unitária ±1e-5, reação à deformação, imunidade a triângulos degenerados/índices inválidos. Caminho GPU acumula `delta_normal` por canal (normais recalculadas também no compute).

### P0-07 — Undo/Redo não cobre Personagem → ✅
- `HistoryStateSnapshot.character: CharacterState` (somatótipo, gênero, 157 esparsos, proporções, hair/cloth/accessory, preset, baseGender).
- Inspetor emite `onCharacterChange` (contínuo) + `onCharacterCommit` (por gesto: `onchange` dos sliders, `isContinuous=false` do pad, preset atômico, tátil `pointerup`).
- `App.applyCharacterState()` restaura inspetor + viewport + espelhos (reset-total-then-apply dos 157, swap de modelo quando necessário); `updateProportions(true)` nos 7 call sites de bridge; `try/finally` no `historyService` (P2-11 junto).
- Teste: 10 mutações → 10 undos → deep-equal inicial; 10 redos → deep-equal final.

### P0-08 — Salvar/Autosave perde o personagem → ✅
- `ProjectStateSnapshot.character` + `schemaVersion: 2`; `parseProjectSnapshot()` valida (JSON, raiz, versão futura rejeitada, migração v1→v2, sanitização de todos os numéricos).
- Autosave **condicional a dirty** (`isDirtyFn`), erros propagados à UI (`onSaveError` → alert), cache `localStorage` **lido no boot com prompt de recuperação** (só quando mais novo que o último save limpo).
- `handleNewProject` reseta o personagem (P2-06 junto); erros de save manual agora alertam (antes: `console.error` silencioso).
- Teste: round-trip com 50 morphs + soma não-default + gênero 0.37; payloads hostis sanitizados.

### P0-09 — Tátil sem clamp, global e com IDs órfãos → ✅
- IDs órfãos corrigidos (`calf_circumference`, `gastrocnemius_height`); `assertTactileIdsValid()` + teste de contrato (zero órfãos nos 13 segmentos).
- Gate explícito: `tactileEnabled={personagem && (body|face)}` (antes: ativo nas 8 workspaces); `onTactileDragStart/End` → **1 entrada de histórico por gesto**.
- `clampCatalog` central em **todos** os caminhos de escrita (slider, tátil, MCP, preset, renderer); base do tátil corrigida (`?? 0.0` → default do catálogo); ids desconhecidos rejeitados com warn (renderer + App + MCP).

### P0-10 — Loader GLB ingênuo e silencioso → ✅
- Novo `src/services/gltf_loader.ts` (puro, testado): valida magic/versão/chunks (scan por tipo), decodifica `BYTE/UBYTE/SHORT/USHORT/UINT/FLOAT` com `normalized` e `byteStride` (VRM), **todos** os meshes/primitivos com merge e offsets, transforms de nós (matriz/TRS, hierarquia de cena), `JOINTS_0/WEIGHTS_0` reais e normalizados (rig preservado), canal `_ANIGO_COLOR → COLOR_0 → neutro`.
- Renderer: erros viram exceção + `onModelLoadError` → alert (nunca cubo silencioso); `AbortError` tratado; log informativo (verts/índices/primitivos/skin).
- Testes: GLB sintético construído no teste (transform, merge, cores U8, skin), magic/versão/truncado rejeitados.

## 3. Arquivos

**Novos (puros, 100% cobertos por teste):** `src/services/character_state.ts`, `src/services/morph_engine.ts`, `src/services/gltf_loader.ts`; `tests/character/*.test.ts` (7 arquivos, 65 testes); `tests/register.mjs` + `tests/resolve-ts.mjs` (hook ESM p/ `node --test`); `package.json` script `test`.

**Modificados:** `AnatomyInspector.svelte`, `SomatotypePad2D.svelte`, `Viewport.svelte`, `tactile.ts`, `webgpu_renderer.ts`, `App.svelte`, `history_service.ts`, `autosave_service.ts`, `character_presets.ts`, `morph_catalog.ts` (comentário), `settings_persist.ts` (declaração global redundante removida).

## 4. Escopo deliberadamente não incluído (P1/P2/P3)

- **P1-02** (semântica baricêntrica vs amplitude no TS) e **P1-03/P1-04** (deltas Rust distintos, regiões nomeadas no Rust): sem toolchain Rust no ambiente; nenhum `.rs` tocado para não quebrar o build nativo. O fallback procedural do viewport é a cobertura interativa; a paridade Rust permanece como Fase 3 da auditoria.
- **P1-12** (dimorfismo contínuo no viewport — hoje: troca discreta de GLB + escalar guardado): estrutura pronta (canal GPU + polaridade canônica), interpolação isomórfica pendente.
- **P1-05/06/07/08/09**, **P2** (exceto P2-03/P2-04-parcial/P2-06/P2-11 incluídos de carona) e **P3**: fora do P0.
- **Regra 1.1 (validação visual via MCP)**: exige GPU + janela + inspeção humana; protocolo da Seção 11.3 da auditoria deve rodar após este P0 — sem ele, nenhuma sprint pode ser declarada concluída.

## 5. Como reproduzir

```bash
npm install
npm test          # 92/92 (65 P0 + 27 E2E existentes)
npm run build     # vite build — passa
npx svelte-check --threshold error  # 104 erros, todos pré-existentes fora do domínio P0
```
