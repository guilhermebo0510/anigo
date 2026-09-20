# AUDITORIA TÉCNICA COMPLETA — WORKSPACE **PERSONAGEM**
### ANIGO Studio · Relatório de Conformidade para Nível Comercial / Enterprise

| Campo | Valor |
| :--- | :--- |
| **Data da auditoria** | 2026-09-20 |
| **Branch auditada** | `arena/01a0c00b-anigo` (base: `fa9ba8e`) |
| **Escopo** | Workspace Personagem (criação anatômica: corpo, rosto, cabelo, vestuário, acessórios, pintura) + motor morfológico, presets, somatótipo, manipulação tátil, undo/redo, persistência e integração com o motor Rust/MCP |
| **Método** | Inspeção estática linha a linha, compilação real (`vite build`), checagem de tipos real (`svelte-check`), execução da suíte E2E existente (`node --test`), diffs programáticos de catálogos (TS × Rust), rastreamento de estado morto, contagem de cobertura funcional |
| **Arquivos auditados** | 31 arquivos de ícones, `src/App.svelte` (3.861 linhas), `AnatomyInspector.svelte` (582), `SomatotypePad2D.svelte` (448), `LightingControls.svelte` (389), `webgpu_renderer.ts` (2.548), `tactile.ts` (413), `Viewport.svelte` (506), `morph_catalog.ts` (1.670), `character_presets.ts` (188), `history_service.ts` (196), `autosave_service.ts` (108), e 11 módulos Rust (`morph_catalog.rs`, `morph.rs`, `somatotype.rs`, `mesh.rs`, `bone_sync.rs`, `tactile.rs`, `anigo-mcp/src/main.rs`, …) |
| **Veredito** | 🔴 **NÃO APROVADO PARA NÍVEL COMERCIAL/ENTERPRISE** — 10 defeitos críticos (P0) e 12 altos (P1), incluindo quebra funcional em runtime, perda de dados e violação direta das Regras Invioláveis |

---

## 1. SUMÁRIO EXECUTIVO

A workspace Personagem tem uma *fachada* de produto profissional (design dark, 157 sliders canônicos em 18 zonas, pad de somatótipo Heath-Carter, presets de fábrica, manipulação tátil estilo Design Doll) apoiada sobre uma base que **não sustenta uso comercial**: o motor que roda no viewport é uma reimplementação paralela em TypeScript com apenas **36 dos 157 sliders** realmente conectados, enquanto o motor canônico em Rust (com 49 deltas explícitos e 108 genéricos) roda **fora do caminho interativo** (apenas no MCP/headless). Os dois motores divergem em semântica, cobertura e resultados — e nenhum deles está completo.

Pior: **as três funcionalidades mais visíveis da workspace estão quebradas em runtime hoje** — (1) clicar em qualquer preset de fábrica lança `TypeError` e dispara um fetch para `/models/anigo_base_undefined.glb`; (2) arrastar o puck do pad de somatótipo corrompe o estado (objeto onde se espera número → `NaN` → puck desaparece, leituras `%` quebradas, `genderDimorphism = NaN`); (3) a polaridade de gênero do inspetor é o inverso da do motor ( Feminino = 1.0 no inspetor, 0.0 no motor).

Além disso, **nada do que o usuário faz no Personagem entra no histórico de undo/redo nem no arquivo de projeto**: o snapshot cobre apenas luz, material e contorno. Autosave grava a cada 5 minutos um estado que não contém o personagem, e o cache de recuperação em `localStorage` é gravado e nunca lido. Em termos práticos: **o trabalho de modelagem do personagem é perdido ao salvar, ao fechar e ao desfazer.**

### 1.1 Contagem de achados

| Severidade | Qtd | Significado | Bloqueia release? |
| :--- | :--: | :--- | :---: |
| 🔴 **P0 — Crítico** | **10** | Quebra funcional em runtime, corrupção de dados, perda de trabalho, exceção não tratada | **Sim** |
| 🟠 **P1 — Alto** | **12** | Funcionalidade central ausente/inconsistente, duas fontes de verdade, violação das Regras Invioláveis | **Sim** |
| 🟡 **P2 — Médio** | **12** | Comportamento incorreto não fatal, robustez, performance, higiene de repositório | Não (corrigir no ciclo) |
| 🔵 **P3 — Melhoria** | **8** | Ergonomia de classe mundial, funcionalidades de diferencial competitivo | Não (roadmap) |
| **Total** | **42** | | |

### 1.2 Os 10 showstoppers (P0)

| # | Defeito | Evidência |
| :-- | :--- | :--- |
| P0-01 | **Presets de fábrica quebrados** (`TypeError` + GLB `undefined` + somatótipo `NaN`) | `AnatomyInspector.svelte:94-116` × `character_presets.ts:3-26` |
| P0-02 | **Pad de somatótipo corrompe o estado** (assinatura de callback incompatível → `NaN`) | `AnatomyInspector.svelte:191` × `SomatotypePad2D.svelte:76-83` |
| P0-03 | **Polaridade de gênero invertida** entre inspetor e motor | `AnatomyInspector.svelte:66,103` × `somatotype.rs:88-92` |
| P0-04 | **121 de 157 sliders (77%) são inertes** no viewport; 12 das 30 chaves dos presets não fazem nada | `webgpu_renderer.ts:1386-1672` |
| P0-05 | **Pipeline de morphs GPU é código morto**; deformação é feita em JS por evento, com churn de buffers | `webgpu_renderer.ts:1023, 1203, 2269` |
| P0-06 | **Normais nunca recalculadas** após deformação → sombreamento e contorno inverted hull errados | `webgpu_renderer.ts:1660-1664` |
| P0-07 | **Undo/Redo não cobre o Personagem** (e restaura estado errado) | `history_service.ts:8-33`, `App.svelte:477-520, 1179` |
| P0-08 | **Salvar/Autosave não preserva o personagem**; projeto carregado sem validação de schema | `autosave_service.ts:3-28`, `App.svelte:645-693, 600` |
| P0-09 | **Manipulação tátil sem clamp, ativa em todas as workspaces, com IDs inexistentes** | `App.svelte:256-274`, `tactile.ts:399-406` |
| P0-10 | **Loader glTF/GLB ingênuo e silencioso** (sem validação, sem nós/materiais, skinning zerado) | `webgpu_renderer.ts:1687-1795` |

### 1.3 Métricas duras levantadas

| Métrica | Valor atual | Alvo enterprise |
| :--- | :--- | :--- |
| Sliders no catálogo canônico | **157** em 18 zonas (documentação/UI dizem *148*) | 157, número derivado do código |
| Sliders com deformação real no viewport (TS) | **36 / 157 (22,9%)** | 157 / 157 (100%) |
| Sliders com delta explícito no motor Rust | **49 / 157 (31,2%)**; 108 caem em fallback genérico por zona | 157 com deltas distintos |
| Sliders implementados nos **dois** motores | **35** | 157 (motor único) |
| Chaves de preset inertes no viewport | **12 / 30 (40%)** | 0 |
| Erros de tipo/compilação (`svelte-check`) | **24 erros + 1 aviso** (6 erros só em componentes do Personagem) | 0 |
| Testes unitários Rust (`#[test]`) | **50** (0 em TS) | ≥ 95% de cobertura nos módulos morfológicos |
| Componentes de ícone sem API de props | **8 / 31** (5 usados na barra do Personagem) | 0 |
| Faixas de índice de vértice "mágicas" hardcoded | **13+**, duplicadas em TS e Rust | 0 (regiões nomeadas) |
| `unwrap()/expect()/panic!()` em `anigo-core` | **41 ocorrências** | 0 em caminho crítico |
| CI/CD | **Inexistente** (sem `.github`) | Obrigatório |

---

## 2. ESCOPO, MÉTODO E LIMITAÇÕES

### 2.1 O que foi executado de fato (reprodutível)

```bash
# 1. Compilação de produção — PASSOU (1 aviso)
npx vite build
#   → dist/assets/index-*.js 339,46 kB │ ✓ built in 2.49s
#   → aviso: src/App.svelte:3387 Unused CSS selector ".badge-warn"

# 2. Checagem de tipos (Svelte 5 + TS) — 24 ERROS
npx svelte-check --threshold warning --output human

# 3. Suíte E2E existente — 27 testes, 27 passam (porém são checagens estáticas superficiais)
node --experimental-strip-types --test tests/e2e/workspace_pipeline.test.ts

# 4. Diff programático dos catálogos TS × Rust (ids, zonas, ranges, arms)

# 5. Rastreamento de estado morto e de caminhos não conectados (grep + contagem de usos)
```

### 2.2 O que **não** foi possível validar neste ambiente (e precisa ser feito antes de qualquer "sprint concluída")

| Item | Motivo | Como suprir |
| :--- | :--- | :--- |
| `cargo check` / `cargo test` / `cargo clippy` | Não há toolchain Rust no sandbox | CI + execução local pelo time (50 testes existentes) |
| Prova visual via `anigo-mcp` (frames reais) | Requer GPU + janela ativa; regra 1.3 exige autorização para automação de mouse/teclado | Executar protocolo da Seção 11.3 com autorização explícita |
| Inspeção perceptual das baselines (`baselines/test_render_sprint03_*_morph.png`) | Auditoria sem capacidade de visão sobre esses artefatos | Revisão humana obrigatória — **não** aceitar como evidência automática |
| Teste de desempenho (FPS/VRAM) com 157 sliders ativos | Requer hardware real | Benchmark automatizado no CI de GPU |

> ⚠️ **Nota de governança:** este relatório não contém, e não pode conter, "testes visuais" no sentido da Regra 1.1. Portanto, **nenhuma sprint da workspace Personagem pode ser declarada concluída** com base apenas neste documento.

---

## 3. INVENTÁRIO E FLUXO DE DADOS DA WORKSPACE

### 3.1 Componentes e arquivos

| Camada | Arquivo | Linhas | Papel | Estado |
| :--- | :--- | --: | :--- | :--- |
| Shell | `src/App.svelte` | 3.861 | Define as 8 workspaces, 6 ferramentas do Personagem, ~60 variáveis de estado, histórico, projeto | 🔴 Duplica estado do inspetor |
| UI | `src/components/character/AnatomyInspector.svelte` | 582 | Modelo base (M/F), 6 presets, pad 2D, busca, 18 zonas × 157 sliders | 🔴 Quebrado (P0-01/02/03) |
| UI | `src/components/character/SomatotypePad2D.svelte` | 448 | Pad baricêntrico Heath-Carter + dimorfismo | 🟠 Contrato de callback divergente |
| UI | `src/components/character/LightingControls.svelte` | 389 | Controles de luz solar/sombra | 🟡 Fora de lugar (é da workspace Iluminação) |
| Viewport | `src/components/viewport/Viewport.svelte` | 506 | Canvas, órbita/zoom/pan, raycast tátil, API exportada | 🟠 DPI do raycast incorreto |
| Motor (front) | `src/components/viewport/webgpu_renderer.ts` | 2.548 | WebGPU/WebGL2, NPR, deformação anatômica, GLB loader | 🔴 Hack de CPU + GLB ingênuo |
| Interação | `src/components/viewport/tactile.ts` | 413 | 13 cápsulas, ray→capsule, projeção tela→slider | 🔴 IDs órfãos |
| Dados | `src/services/morph_catalog.ts` | 1.670 | Catálogo dos 157 sliders (18 zonas) | 🟢 Íntegro e em sincronia com o Rust |
| Dados | `src/services/character_presets.ts` | 188 | 6 presets de fábrica | 🟠 Contrato divergente do consumidor |
| Estado | `src/services/history_service.ts` | 196 | Undo/Redo por snapshot | 🔴 Snapshot incompleto |
| Estado | `src/services/autosave_service.ts` | 108 | Autosave + cache local | 🔴 Não salva o personagem |
| Motor (Rust) | `crates/anigo-core/src/morph_catalog.rs` | 1.086 | 157 defs + geração de deltas esparsos | 🟠 108 genéricos |
| Motor (Rust) | `crates/anigo-core/src/morph.rs` | 339 | `SparseMorphDelta`, `apply_cpu` | 🟢 Correto (normaliza normais), com `assert!` |
| Motor (Rust) | `crates/anigo-core/src/somatotype.rs` | 303 | Baricêntrico + interpolação de gênero | 🟢 Correto e testado |
| Motor (Rust) | `crates/anigo-core/src/mesh.rs` | 1.605 | Base canônica 4.070 vértices isomórfica | 🟡 Skinning rígido |
| Motor (Rust) | `crates/anigo-core/src/tactile.rs` | 506 | Raycast de cápsulas | 🟢 6 testes |
| MCP | `crates/anigo-mcp/src/main.rs` | 1.418 | Ferramentas MCP de personagem | 🟠 Escala 1–12 documentada, 0–1 no código |

### 3.2 Fluxo de dados atual (e onde ele se rompe)

```
[Usuário: slider do inspetor]
   └─ AnatomyInspector.sliderValues (estado LOCAL) ─────► viewportRef.setMorphSlider(id, v)
                                                              └─ renderer.activeMorphWeights (Map)
                                                                   └─ buildGeometryBuffers()  [CPU, por evento]
                                                                        └─ applyAnatomicalDeformations() [36 ids]
                                                                             └─ destroy/create VBO+IBO ─► GPU

[Usuário: pad de somatótipo]
   └─ SomatotypePad2D.onUpdate({endomorph,…}) ─X─ AnatomyInspector.handleSomatotype(endo, meso, ecto, g)
                                                     ▲ ASSINATURA INCOMPATÍVEL → NaN (P0-02)

[Usuário: arrasto no viewport]
   └─ Viewport.onTactileDrag ─► App.morphSliders (estado LOCAL do App) ─► renderer
        X  não clampado, não reflete no inspetor, não vai pro histórico (P0-09, P1-01)

[MCP / Tauri]
   └─ eventos anigo://apply_morph_slider, reset_morphs, set_somatotype ─► App.morphSliders / somatotype*
        X  não chegam ao AnatomyInspector → UI e motor divergem (P1-01)
   └─ motor Rust (mcp/headless) roda EM PARALELO, com 49 deltas próprios (P1-02)

[Persistência]
   └─ getProjectSnapshot() ─► JSON ─► autosave/localStorage/disco
        X  sem somatótipo, gênero, morphs, proporções, cabelo, roupa, acessórios (P0-08)
```

**Conclusão estrutural:** existem **dois estados de personagem** (App × AnatomyInspector), **dois motores de deformação** (TS × Rust) e **dois modelos semânticos de somatótipo** (amplitudes independentes × baricêntrico normalizado). Enquanto isso não for unificado, qualquer correção pontual é paliativo.

---

## 4. ACHADOS CRÍTICOS (P0)

### 🔴 P0-01 — Presets de fábrica quebrados: exceção em runtime, GLB inexistente e somatótipo `NaN`

**Evidência**
- Consumidor: `src/components/character/AnatomyInspector.svelte:94-116`
  ```ts
  if (preset.base_gender !== baseGender) { handleGenderChange(preset.base_gender); }
  somatotypeEndo = preset.somatotype[0];   // ← somatotype é OBJETO, não tupla
  ...
  for (const [key, val] of Object.entries(preset.morph_parameters)) { ... }  // ← não existe
  ```
- Contrato real: `src/services/character_presets.ts:3-26` → `{ id, name, description, model, somatotype: {endo, meso, ecto}, genderDimorphism, proportions, sliders }`
- Prova de compilação: `svelte-check` →
  `AnatomyInspector.svelte:96 Property 'base_gender' does not exist on type 'CharacterPreset'`,
  `:105 Property 'morph_parameters' does not exist`, `:107 Type 'unknown' is not assignable to type 'number'`

**Reprodução:** Personagem → ferramenta *Manequim & Corpo* → clicar em qualquer chip de preset ("Shonen Hero", "Chibi 2.5c", …).

**Impacto (3 falhas em cascata)**
1. `Object.entries(undefined)` → **TypeError** não capturado (a promise do `handleGenderChange` async não é awaited nem tem `catch`) → preset aborta no meio.
2. `handleGenderChange(undefined)` → `loadCanonicalModel(undefined)` → `fetch("/models/anigo_base_undefined.glb")` → **404** → função retorna em silêncio (`if (!binBuffer || !gltf.meshes…) return`) → nenhum erro ao usuário.
3. `preset.somatotype[0]` → `undefined` → `setSomatotype(undefined, …)` → deformação com `undefined` (comparações `> 0.001` falham silenciosamente → preset visualmente nulo, mesmo quando não lança exceção).

**Correção exigida**
1. Unificar o contrato em um único tipo `CharacterPreset` com `model: "male" | "female"`, `somatotype: SomatotypeCoords`, `proportions`, `sliders: Record<SliderId, number>` e **congelar via `satisfies`**; o `CharacterPreset` deve ser a única fonte, consumida por App e inspetor.
2. Reescrever a aplicação de preset como **operação atômica**: (a) `resetAll()` para defaults canônicos; (b) troca de modelo **awaitada** com `try/catch`; (c) aplicação de somatótipo; (d) aplicação de proporções; (e) aplicação dos sliders; (f) **um único** commit no histórico; (g) toast de erro em falha.
3. Bloquear no CI: `svelte-check` com 0 erros e teste de contrato que itera todos os presets aplicando-os contra um mock de renderer e verifica que **todos** os ids existem no catálogo.

---

### 🔴 P0-02 — Pad de somatótipo corrompe o estado (`NaN` propagado ao pad e ao motor)

**Evidência**
- Emissor: `SomatotypePad2D.svelte:76-83`, `113-121`, `126-133`, `139-146` → `onUpdate?.({ endomorph, mesomorph, ectomorph, genderDimorphism, isContinuous })` (**objeto único**).
- Receptor: `AnatomyInspector.svelte:191` → `onUpdate={handleSomatotype}` onde `handleSomatotype(endo: number, meso: number, ecto: number, gender: number)` (`:79`).
- Prova: `svelte-check` → `AnatomyInspector.svelte:191 Error: Type '(endo: number, meso: number, ecto: number, gender: number) => void' is not assignable to type '(params: {...}) => void'`.

**Comportamento resultante (cadeia exata)**
1. `somatotypeEndo = {endomorph:…}` (objeto), `meso/ecto/genderDimorphism = undefined`.
2. `viewportRef.setSomatotype(objeto, undefined, undefined)` → guardas `if (endo > 0.001)` etc. falham → **somatótipo não deforma nada**.
3. `viewportRef.setGenderDimorphism(undefined)` → `Math.max(0, Math.min(1, undefined))` → **`genderDimorphism = NaN`** armazenado no renderer (sem guarda `Number.isFinite`).
4. O valor volta via `bind:endomorph` para o pad → `coordsToSvg(e, m, ec)` → `sum = NaN || 1.0` → `x = NaN` → `<circle cx={NaN}>` → **puck desaparece/salta para a origem** e os cartões de leitura exibem `NaN%`.

**Correção exigida**
1. Tipar o callback: `onUpdate?: (p: SomatotypeUpdate) => void` e **não** usar parâmetros posicionais; o pad e o inspetor devem compartilhar a mesma interface exportada.
2. Sanitizar todas as fronteiras numéricas (`Number.isFinite` + `clamp`), no pad, no inspetor e nos setters do renderer.
3. Teste unitário (TS): simular `pointerdown/move/up` no pad e afirmar que (a) os três componentes são finitos, (b) somam 1.0 ± 1e-6, (c) o callback emite exatamente o tipo esperado, (d) o puck permanece dentro do triângulo.

---

### 🔴 P0-03 — Polaridade de gênero invertida entre inspetor e motor

**Evidência**
- Inspetor: `AnatomyInspector.svelte:66` → `genderDimorphism = gender === "female" ? 1.0 : 0.0;` e `:103` → `… preset.base_gender === "female" ? 1.0 : 0.0`.
- Motor Rust: `somatotype.rs:88-92` → documenta `0.0 = Feminino, 1.0 = Masculino` e interpola `(1-g)*female + g*male`.
- App/MCP: `App.svelte:881` (`anigo://set_character_model`) → `genderDimorphism = p.model_type === "female" ? 0.0 : 1.0`.
- Presets: `character_presets.ts` → femininos com `genderDimorphism: 0.0`.
- Pad: `SomatotypePad2D.svelte:234-239` rotula `> 0.65` como **Masculino**.

**Impacto:** clicar em "Feminino Base" faz o motor interpolar para o corpo **masculino** e o pad exibir "Masculino 100%"; presets femininos aplicados pelo inspetor produzem corpo masculino. É uma inversão silenciosa que invalida qualquer prova visual de gênero.

**Correção exigida**
1. Criar tipo nominal `type GenderDimorphism = number` com **constante única** `GENDER_FEMALE = 0.0`, `GENDER_MALE = 1.0`, `GENDER_ANDROGYNOUS = 0.5` em módulo compartilhado (`services/character_state.ts`), usado por App, inspetor, pad, presets e bridge MCP.
2. Unificar o helper `genderToDimorphism(g: "male" | "female"): GenderDimorphism`.
3. Teste de round-trip: `toGender(fromDimorphism(x))` para x ∈ {0, 0.25, 0.5, 0.75, 1} e verificação de que a malha interpolada em g=0 é bit-a-bit igual à base feminina (já coberto em `somatotype.rs:262`).

---

### 🔴 P0-04 — 121 de 157 sliders (77%) são inertes no viewport; 40% das chaves dos presets não fazem nada

**Evidência (medição programática)**
- IDs efetivamente lidos pelo motor do viewport: **36** (`webgpu_renderer.ts:1386-1672`, extração de `activeMorphWeights.get("…")`).
- Inertes: **121**.
- IDs com delta explícito no Rust: **49**; **108** caem no fallback genérico (`morph_catalog.rs:780-818`).
- Interseção dos dois motores: **35**.
- Exclusivos do Rust (inertes no viewport): `deltoid_muscle_volume`, `latissimus_dorsi_flare`, `ribcage_width`, `outer_thigh_sweep`, `forearm_brachioradialis`, `flank_love_handles`, `cheek_fullness_upper`, `jaw_bigonial_width`, `chin_cleft_dimple`, `nose_tip_sharpness`, `ear_flare_angle`, `pectoral_lower_cut`, `gluteus_posterior_shelf`, `forehead_roundness`.
- Chaves de preset inertes no viewport: **12 de 30** (`deltoid_muscle_volume`, `latissimus_dorsi_flare`, `cheek_fullness_upper`, `triceps_bulk`, `jaw_bigonial_width`, `flank_love_handles`, `submental_fullness`, `nose_tip_sharpness`, `ear_scale_uniform`, `outer_thigh_sweep`, `quadriceps_definition`, `clavicle_bone_relief`).
- Zonas **100% inertes no viewport**: Proporções Globais & Silhueta (8 — inclui os 3 sliders de somatótipo), Sobrancelhas (6), Boca & Lábios (8), Ombros/Clavículas/Dorsal (8), Mãos & Dedos (8) = **38 sliders**; parcialmente inertes: Olhos (3/12), Nariz (2/8), Orelhas (1/5), Pelve (1/9), Tórax (1/6).

**Impacto comercial:** o principal argumento de venda da workspace ("157 sliders anatômicos") é falso na prática para 77% dos controles. Um artista que arrasta "Espessura da Sobrancelha" ou "Largura dos Dedos" **não vê nada acontecer** e conclui que o software está quebrado.

**Correção exigida**
1. **Proibir UI de slider sem deformação:** todo slider exposto no inspetor deve ter delta verificável. Implementar os 121 restantes no motor canônico (ver P1-03/P0-05) e, até lá, **ocultar/marcar como "em implementação"** os não suportados (nunca exibir controle morto — isso é exatamente o que a Regra 2 proíbe).
2. Teste de contrato no CI: para cada id do catálogo, aplicar `valor = max` e afirmar que ao menos N vértices se deslocaram mais de 1e-4 e que o delta é **distinto** do delta de qualquer outro slider da mesma zona (mata o fallback genérico disfarçado).

---

### 🔴 P0-05 — Pipeline de morphs em GPU é código morto; a deformação real é um hack de CPU por evento

**Evidência**
- O compute shader `cs_accumulate_morphs` (WGSL) existe e é compilado: `webgpu_renderer.ts:587-745`.
- `uploadSparseMorphData()` (`:1023`) **nunca é chamado** em lugar nenhum do repositório.
- Consequência: `morphBindGroup` é sempre `null`; o bloco em `:2269-2271` nunca executa; `dispatchSparseMorphs` nunca roda.
- Todo o caminho real: `setMorphSlider/setSomatotype/setGenderDimorphism` (`:1088-1108`) → `buildGeometryBuffers()` (`:1203`) → `generateMannequinData()` → `applyAnatomicalDeformations()` (`:1386`) → `destroy()` + `createBuffer()` de VBO e IBO a **cada evento `input`**.

**Impacto**
- A deformação roda em JavaScript single-thread (4.070 vértices × dezenas de condicionais) a cada pixel de movimento do slider; em um arrasto de 120 Hz são ~120 rebuilds/s, cada um recriando buffers de GPU (churn de alocação no driver).
- Descumpre o objetivo central da Sprint 03 (Sub-Sprint 3.3: *"Compute Shaders WebGPU em WGSL processando deltas esparsos indexados"*) e o teto de "< 3,5 MB de VRAM".
- Divergência garantida com o motor Rust (que faz o mesmo trabalho com deltas esparsos de verdade).

**Correção exigida**
1. **Ligar o pipeline real:** gerar o `SparseMorphSet` (já existe em Rust, `morph_catalog.rs:830+`), serializar e enviar ao front (comando Tauri ou WASM), chamar `uploadSparseMorphData()` na carga do modelo e `dispatchSparseMorphs()` a cada frame com pesos ativos (já suportado, com busca binária no shader: `:663-675`).
2. Manter um único par VBO/IBO pré-alocado no tamanho máximo e usar `writeBuffer` (fim do `destroy/create` por evento).
3. Dirty-flag + throttle em `requestAnimationFrame` (máx. 1 rebuild por frame).
4. Benchmark de aceitação: ≥ 120 FPS com 40 sliders ativos simultâneos e pico de VRAM medido < 3,5 MB (relatório anexado à sprint).

---

### 🔴 P0-06 — Normais nunca são recalculadas após a deformação

**Evidência:** `webgpu_renderer.ts:1660-1664` escreve apenas `v.pos[0..2] = x, y, z`. `v.normal` é mantido do base mesh. O motor Rust faz o contrário corretamente (`morph.rs:230-241`).

**Impacto**
- **Sombreamento toon errado:** o terminador de sombra (banda NPR) é definido por `dot(N, L)`; com normais obsoletas, ombros/quadris/busto deformados mantêm a iluminação da malha original.
- **Contorno inverted hull descola:** a extrusão do contorno é feita ao longo da normal (`outlineExtrusion`); com normais desatualizadas o traço se separa da silhueta — artefato imediatamente visível e inaceitável em produção.
- Divergência de cor/contorno entre o viewport (errado) e as provas do MCP/headless (certas).

**Correção exigida:** recalcular normais após acumular deltas — por compute pass de normais (recomendado, aproveitando o índice) ou por acumulação de `delta_normal` + normalização (como o Rust). Teste numérico: após aplicar `deltoid_muscle_volume = max`, ≥ 95% das normais afetadas devem divergir das originais em > 1e-3, e `|N| = 1 ± 1e-5`.

---

### 🔴 P0-07 — Undo/Redo não cobre a workspace Personagem (e restaura estado parcial)

**Evidência**
- `HistoryStateSnapshot` (`history_service.ts:8-33`) contém apenas: `preset, headScale, headRatio, outlineWidth, shadowThreshold, lightDir, lightIntensity, shadowColor` + opcionais de luz/material/contorno/cores + `activeWorkspace/activeTool/projectName`.
- **Ausentes:** `somatotype(endo/meso/ecto)`, `genderDimorphism`, `morphSliders` (157), `shoulderWidth/legLength/armLength/neckLength`, `hair*`, `cloth*`, `accessory*`, pintura.
- `applySnapshot` (`App.svelte:477-520`) restaura somente esses campos e ainda chama `viewportRef.switchPreset(...)` → **reconstrói a malha do zero** a cada Ctrl+Z.
- `updateProportions(record = false)` (`App.svelte:1179`) **nunca é chamado com `record = true`** (todas as chamadas em `:813, 1002-1007` usam o default) → o parâmetro está morto e **nenhuma alteração de proporção entra no histórico**.
- `AnatomyInspector` **nunca** chama `recordHistory` (não recebe callback) → somatótipo, gênero e os 157 sliders não são desfazíveis.

**Impacto:** o usuário modela o personagem por 20 minutos, pressiona Ctrl+Z esperando desfazer o último slider, e o sistema reverte **a iluminação** (ou nada) e deixa o corpo como está — o efeito percebido é "undo aleatório". A Regra R2 e o critério de aceitação "*Undo/Redo opera sem regressões nos parâmetros do estúdio*" não são atendidos.

**Correção exigida**
1. Substituir o snapshot manual por um **estado de personagem único e serializável** (`CharacterState`), com histórico por *command pattern* (ação + inverso) ou diff profundo automático — nunca uma lista manual de campos.
2. `recordHistory` por gesto: `pointerdown` abre transação, `pointermove` atualiza (coalescência por gesto), `pointerup` commiteia com descrição legível e i18n.
3. Testes: aplicar 10 mutações distintas de personagem → desfazer 10× → estado final deve ser **idêntico** ao inicial (deep equal), e refazer 10× → igual ao estado pós-mutações.

---

### 🔴 P0-08 — Salvar/Autosave não preserva o personagem; abertura de projeto sem validação

**Evidência**
- `ProjectStateSnapshot` (`autosave_service.ts:3-28`) e `getProjectSnapshot()` (`App.svelte:645-693`) têm exatamente os mesmos campos do histórico → **nenhum dado de personagem**.
- `handleOpenProject` (`App.svelte:600-612`) faz `applySnapshot(JSON.parse(rawJson))` **sem checar `version`, sem migração e sem validação de schema**; `version: "0.1.0"` é gravada e nunca lida.
- Autosave (`autosave_service.ts:47-52`) roda a cada 5 min **independentemente de haver alteração** (não consulta `isProjectDirty`) e grava o cache em `localStorage["anigo_autosave_cache"]` que **nunca é lido** em lugar nenhum (recuperação é write-only).
- Falhas são `console.error` (`:86`) — o usuário não é informado.

**Impacto:** perda total do trabalho de modelagem (o ativo de maior valor da sessão) ao salvar/fechar/abrir;ausência de recuperação após queda; risco de corromper projetos antigos após qualquer mudança de schema.

**Correção exigida**
1. Snapshot de projeto versionado (`schemaVersion`), com **todos** os parâmetros de personagem, e carga via validador (zod/valibot ou `serde` no Rust) + migrations encadeadas.
2. Autosave condicional a `isProjectDirty`, com debounce (ex.: 30 s de inatividade) e **restauração automática** do cache ao iniciar (com prompt "Deseja recuperar a sessão não salva?").
3. Erros de E/S propagados à UI (toast + log estruturado).
4. Teste de round-trip: salvar → carregar → deep equal, para um projeto com 50 morphs alterados, somatótipo não-default e gênero 0.37.

---

### 🔴 P0-09 — Manipulação tátil: sem clamp, sem histórico, ativa em todas as workspaces, com IDs inexistentes

**Evidência**
- `App.svelte:256-274` (`handleTactileDrag`): `morphSliders[k] = (morphSliders[k] ?? 0) + delta` — **sem clamp** a `[min, max]` do catálogo e sem validar que o id existe.
- `Viewport.svelte:47-54` dispara o raycast em **qualquer** clique esquerdo simples: o gate é apenas `e.button === 0 && !alt && !ctrl && !shift` — não há checagem de `activeWorkspace === "personagem"` nem de `activeTool === "body"`. Logo, arrastar sobre o corpo nas workspaces **Shading, Iluminação, Cenário, Animação e Render** deforma o personagem.
- `tactile.ts:399-406`: os ids `calf_gastrocnemius_volume` e `ankle_achilles_definition` **não existem** no catálogo (a zona LowerLimbs tem `calf_circumference` e `gastrocnemius_height`) → arrastar a panturrilha grava valores órfãos invisíveis na UI.
- Não existe callback de fim de arrasto → nenhum commit no histórico (agravado pelo P0-07).

**Impacto:** valores fora de faixa (ex.: `head_width = 12.7` quando o máximo é 1.35) que a UI não consegue representar (o `<input range>` satura, mas o motor usa o valor real), deformações acidentais fora da workspace de modelagem e perda de rastreabilidade.

**Correção exigida**
1. Gate explícito: tátil ativo **somente** em `personagem` + ferramenta `body` (e futuro `face`), com cursor e highlight de cápsula correspondentes.
2. `clampCatalog(id, value)` central, aplicado em **todo** caminho de escrita (slider, tátil, MCP, preset).
3. Tipo literal `SliderId` derivado do catálogo, para que ids inválidos **não compilem** (elimina classes de bug como os dois ids órfãos).
4. Callbacks `onTactileDragStart/End` para abrir/fechar transação de histórico com descrição i18n.

---

### 🔴 P0-10 — Loader glTF/GLB ingênuo, silencioso e que destrói o rig

**Evidência:** `webgpu_renderer.ts:1687-1795`
- `magic` e `version` são lidos (`:1694-1695`) e **nunca validados** (`0x46546C67` e `2`).
- Assume exatamente 2 chunks e BIN logo após o JSON (`:1703-1711`), sem varrer chunks por tipo.
- `readFloatArray` (`:1728-1739`) assume `componentType = 5126 (FLOAT)`; ignora `normalized`, `byteStride` de acessores interleaved e tipos quantizados (`UNSIGNED_SHORT` normalizado é comum em VRM).
- Usa apenas `gltf.meshes[0].primitives[0]` (`:1716`) — modelos com múltiplos primitivos (corpo, rosto, cabelo — padrão em VRM) são **truncados**.
- Ignora `nodes`, `scenes` e transformações (TRS/matriz) → modelo sempre na origem, sem escala/rotação do autor.
- Ignora materiais, texturas e `skins`; grava `joints: [0,0,0,0]`, `weights: [1,0,0,0]` para **todos** os vértices (`:1782-1785`) → **skinning destruído**.
- Falhas terminam em `return` silencioso (`:1714`) → o usuário vê um **cubo** (fallback de `generateMannequinData`, `:1683`) sem qualquer mensagem.

**Impacto:** a base canônica carrega sem rig (inviabiliza Posing/Animação — Sprints 04/13-16), nenhum GLB/VRM de terceiros carrega corretamente, e o modo de falha é invisível. Inaceitável para interoperabilidade VRM 1.0 (Sprint 11).

**Correção exigida**
1. Substituir por loader glTF 2.0 completo e validado: preferencialmente `gltf` crate no Rust (comando Tauri que devolve buffers prontos) ou, no front, um loader auditado (`@gltf-transform/core` + validador Khronos).
2. Propagar `Result` até a UI: erro de parse/404/magic inválido must exibir diálogo técnico (arquivo, motivo, ação).
3. Preservar **todos** os primitivos (multi-submesh), nós/transformações, `skins` (joints + weights reais com até 4 influências), materiais e texturas.
4. Testes: carregar os GLBs do repositório + uma suíte de ativos Khronos (`glTF-Sample-Assets`) e afirmar contagem de vértices/índices, presença de skin e igualdade de bounds.

---

## 5. ACHADOS ALTOS (P1)

### 🟠 P1-01 — Duas fontes de verdade para o estado do personagem (App × AnatomyInspector)
`App.svelte:216-273` (`morphSliders`, `somatotypeEndo/Meso/Ecto`, `genderDimorphism`, `activeCharacterPreset`) e `AnatomyInspector.svelte:25-33` (cópias locais). Eventos MCP (`anigo://apply_morph_slider`, `anigo://reset_morphs`, `anigo://set_somatotype`) e o arrasto tátil escrevem **só no App** → os sliders do inspetor não se movem; as edições do inspetor não chegam ao estado do App. `handleSomatotypeUpdate` (`App.svelte:234`) e `handleCharacterPreset` (`App.svelte:276`) têm **0 usos** (código órfão). **Correção:** store único em Svelte 5 (`$state` em módulo + contexto), com bind de duas vias e nenhuma duplicação; remover os órfãos.

### 🟠 P1-02 — Dois motores divergentes com semânticas incompatíveis
Rust (MCP/headless) usa **somatótipo baricêntrico normalizado** (`somatotype.rs:41-53`, soma = 1) e 49 deltas; o TS usa **amplitudes independentes** (`webgpu_renderer.ts:1405-1407, 1630-1658`, cada componente é um multiplicador direto) com 36 deltas (35 em comum). O mesmo número, por exemplo `(0.33, 0.34, 0.33)`, significa coisas diferentes nos dois lados. **Consequência:** as provas visuais do MCP jamais reproduzem o que o usuário vê — o que anula a validação exigida pela Regra 1.1. **Correção:** motor único (Rust) como dono da matemática; o front envia comandos e recebe buffers; contrato numérico documentado em ADR.

### 🟠 P1-03 — 108 de 157 sliders no Rust caem em fallback genérico por zona
`morph_catalog.rs:780-818`: deltas idênticos de "inflar ao longo da normal" por zona → `nose_alar_width` produz exatamente o mesmo delta de `nose_bridge_height`; `fingernail_style_anime` infla a mão inteira. É **implementação nominal**, não funcional — viola frontalmente a Regra 2 (tolerância zero a provisórios). **Correção:** delta real por slider (máscaras por região/UV/osso), com o teste de "delta distinto" do P0-04.

### 🟠 P1-04 — 13+ faixas de índice de vértice "mágicas" duplicadas em TS e Rust
TS (`webgpu_renderer.ts:1419, 1500, 1519, 1566, 1585, 1587, 1591, 1606, 1611, 1615, 1623`) e Rust (`morph_catalog.rs`: `425..544`, `544..1069`, `1069..1444`, `1444..1678`, `1444..2536`, `1678..1912`, `1912..2146`, `2146..2536`, `2536..2926`, `2926..3160`, `3160..3550`, `3550..4070`, `2536..4070`). Qualquer mudança de topologia — ou a carga de um GLB com **outra ordem de vértices** — destrói **silenciosamente** todas as deformações e produz artefatos grotescos (ex.: o delta de "busto" aplicado no joelho). **Correção:** regiões nomeadas (`VertexRegion`) derivadas de grupos/ossos/UVs, com teste de integridade que valida o mapeamento região→faixa contra a malha carregada.

### 🟠 P1-05 — Ferramentas *Cabelo*, *Vestuário*, *Acessórios* e *Pintura* são maquetes inertes
`App.svelte:1673-1816`: sliders com `bind:value` em estado local que **nunca** chegam ao motor; botão "Gerar Nova Mecha Guiada" (`:1701`) **sem `onclick`**; sliders de pincel com `value="16"`/`"0.8"`/`"0.5"` **hardcoded** (nem estado têm, `:1792-1810`); botões de máscara com `class="selected"` estático (`:1814-1816`). **Viola diretamente a Regra 2.1/2.3.** **Correção:** implementar de verdade (Sprints 06/07/09/10) **ou** remover as ferramentas da barra até existirem; se mantidas como preview, desabilitar explícita e visualmente (`disabled` + badge "Em desenvolvimento"), nunca simular funcionamento.

### 🟠 P1-06 — A ferramenta "Rosto & Olhos" não existe
`App.svelte:1642` → `{#if activeTool === "body" || activeTool === "face"}` renderiza **o mesmo painel**. `eyeScale`, `chinWidth`, `jawWidth` (`App.svelte:302-306`) são escritos por `handleSliderUpdate` (`:332-334`) e nunca lidos; `eyeTilt` (`:306`) tem **0 usos**. As 6 zonas faciais (Eyes 12, Eyebrows 6, Nose 8, MouthLips 8, JawChin 10, Ears 5 = 49 sliders) ficam enterradas no acordeão genérico. **Correção:** painel facial dedicado com as 6 zonas, mini-preview de rosto e sincronia bidirecional + remoção do estado morto.

### 🟠 P1-07 — Emojis na UI e 8 componentes de ícone sem API (5 deles na barra do Personagem)
- `AnatomyInspector.svelte:197` usa **🔍** (emoji, terminantemente proibido pela Regra 2.2) quando existe `src/components/icons/SearchIcon.svelte`; também ♂/♀ (`:144, :156, :232`), ↺/✕/▼/▶ (`:229, :246, :213`) como glifos tipográficos.
- **8** componentes de ícone não declaram `$props()`: `BoneIcon`, `BotIcon`, `BrushIcon`, `ScissorsIcon`, `ShirtIcon`, `SmileIcon`, `SunIcon`, `UserIcon` — são SVG com `width="18"` fixo. Como `App.svelte:1537` faz `<ToolIcon size={18} />` (e `:2400-2408` passa `size={34}`), o tamanho é **ignorado** e o TS acusa `Type 'number' is not assignable to type 'never'` — **17 dos 24 erros** do `svelte-check` vêm exatamente disso. Os 5 primeiros da lista são justamente os ícones das ferramentas do Personagem (Corpo, Rosto, Cabelo, Vestuário, Pintura).
- **Correção:** padronizar todos os ícones com `{ size = 18, strokeWidth = 2, ...rest }`, usar `SearchIcon`/`CheckIcon`/`ChevronDown` existentes no lugar de glifos, e adicionar teste que instancia todos os ícones com props.

### 🟠 P1-08 — Zero internacionalização no conteúdo do Personagem
Os 157 nomes de slider, 18 zonas, 6 presets (nome + descrição) e todo o chrome do inspetor/pad ("PRESETS DE FÁBRICA", "Resetar Sliders", "Filtrar 148 sliders…", "Músculo/Gordura/Magreza", "Masculino/Feminino/Andrógino") estão **hardcoded em português**, enquanto o app mantém 3 dicionários completos e consistentes (171 chaves cada, verificados: 0 divergências entre pt-BR/en/ja). **Correção:** extrair para i18n com chaves estáveis (`morph.slider.<id>`, `morph.zone.<key>`, `character.preset.<id>.name/.desc`, `character.pad.*`) e usar o catálogo do Rust como fonte de ids; bloquear strings literais em componentes via lint.

### 🟠 P1-09 — Acessibilidade e ergonomia aquém do padrão profissional
- Nenhum `<input type="range">` do inspetor tem `<label>`/`aria-label`/`aria-valuetext` (`AnatomyInspector.svelte:222-231`).
- Botão de reset por slider é apenas o glifo "↺" (`:243-250`), sem rótulo acessível.
- O pad SVG tem `role="application"` (`SomatotypePad2D.svelte:173-180`) mas **não é operável por teclado**; não há `tabindex`, nem setas/PageUp, nem `aria-valuetext` com os três componentes.
- Acordeões sem `aria-controls`/`aria-expanded` consistentes; sem `focus-visible` explícito; sem indicação de valor não-default por teclado/leitores de tela.
- **Correção:** rotular todos os controles, tornar o pad navegável por teclado, anunciar valores com unidades, garantir contraste AA e ordem de tabulação lógica.

### 🟠 P1-10 — `resetAllSliders` define somatótipo `(0, 0, 0)`, fora do domínio válido
`AnatomyInspector.svelte:126-129` → `setSomatotype(0, 0, 0)`. O domínio do pad é baricêntrico (soma = 1); o default do Rust é `1/3` cada (`somatotype.rs:22-29`); o default do pad/App é `0.33/0.34/0.33`. Três "neutros" diferentes e um delo matematicamente inválido. **Correção:** constante única `NEUTRAL_SOMATOTYPE` + normalização na fronteira do motor + teste de que todo somatótipo aceito soma 1.0.

### 🟠 P1-11 — Raycast tátil com DPI quebrado
`Viewport.svelte:50-52` passa coordenadas **CSS**; `webgpu_renderer.ts:2506-2517` converte usando `this.canvas.width/height`, que são **pixels de dispositivo** (o renderer mantém `cssWidth/cssHeight` corretos em `:197-198`, mas não os usa aqui). Em DPI 1.5×/2.0× o NDC é calculado com metade/um terço do valor real → o raio erra progressivamente em direção às bordas. `projectTactileDelta` (`:2521-2529`) divide deltas CSS pela largura do dispositivo → **sensibilidade do arrasto cortada pela metade em 2.0×**. **Correção:** usar `cssWidth/cssHeight` (ou `getBoundingClientRect`) em ambos; teste de picking com DPI 1.0/1.5/2.0 comparando o segmento atingido.

### 🟠 P1-12 — Dimorfismo de gênero contínuo não existe no viewport
`webgpu_renderer.ts:120` declara `genderDimorphism`, `:1097-1101` armazena (e **reconstrói a geometria**), mas **nenhuma deformação o utiliza** (grep: apenas declaração e setter). A troca de gênero é feita por **troca discreta de arquivo GLB** (`anigo_base_male.glb`/`anigo_base_female.glb`) — o que é exatamente o oposto do requisito de Sub-Sprint 3.1/3.5 (*"interpolação de gênero contínua"*, implementada e testada no Rust, `somatotype.rs:118-186`). **Correção:** implementar a interpolação isomórfica no caminho interativo (GPU: lerp de dois buffers de base + pesos; ou via motor Rust), com o slider 0↔1 produzindo transição contínua e teste de que g=0 ≡ base feminina e g=1 ≡ base masculina.

---

## 6. ACHADOS MÉDIOS (P2)

| ID | Achado | Evidência | Correção exigida |
| :-- | :--- | :--- | :--- |
| **P2-01** | 41 `unwrap()/expect()/panic!()` em `anigo-core` (mesh.rs 19, morph_catalog 5, bone_sync 8, somatotype 3, tactile 4, scene 2); `assert_eq!` em caminho de produção (`morph.rs:199`); indexação sem checagem (`morph_catalog.rs:926` → `base_mesh.vertices[0]` pânico se vazio) | Regra 2.3 exige zero unwrap em caminho crítico | `Result`/`thiserror` em todas as APIs públicas; `first().ok_or(…)`; `#[deny(clippy::unwrap_used)]` no CI |
| **P2-02** | `build_canonical_sparse_morph_set` é O(sliders × vértices) = **157 × 4.070 ≈ 639 mil avaliações**, executado em `MorphCatalog::new()` e em **cada** `set_gender()` (`morph_catalog.rs:846-867`) — sem cache | Travamento perceptível ao trocar gênero | Pré-computar e serializar (cache versionado em disco), geração lazy por slider, medição < 16 ms |
| **P2-03** | Divergência documental de contagem: spec, UI e comentários dizem **148**; o catálogo tem **157** (`SPRINT_03.md:57`, `AnatomyInspector.svelte:201`, `morph_catalog.rs:2-3`, descrição da ferramenta MCP `"148+"`) | Ruído de governança | Número **sempre** derivado de `CANONICAL_SLIDERS.length`; proibir literais |
| **P2-04** | Carga do modelo sem tratamento: `loadCanonicalModel` é `async` sem `await`/`catch` no chamador (`AnatomyInspector.svelte:69-71`), sem estado de carregamento, sem retry, falha silenciosa | Usuário vê cubo sem saber por quê | `try/catch` + estado `{idle, loading, ready, error}` + indicador na UI + retry |
| **P2-05** | `switchWorkspace`/`selectTool` sem validação: `selectTool` (`App.svelte:1039-1050`) aceita ids inexistentes (ex.: `"clothing"`, `"morphs"` vindos do MCP) → inspetor **em branco** (a cadeia `{#if}` termina sem `{:else}`, `:2823`); `switchWorkspace` sempre reseta para a ferramenta padrão | Estado inválido silencioso | Validar contra o catálogo de ferramentas, cair no default com log, persistir última ferramenta por workspace |
| **P2-06** | `applySnapshot` não restaura `activeWorkspace`/`activeTool` (embora os armazene) e `handleNewProject` (`App.svelte:624-644`) reseta apenas 7 variáveis — nenhuma de personagem | "Novo projeto" deixa o corpo do projeto anterior | Reset central a partir do estado canônico (`createDefaultCharacterState()`) |
| **P2-07** | Estado morto/duplicado: `eyeTilt` (0 usos), `eyeScale/chinWidth/jawWidth` (só escritos), `hairStrands` (só UI), `handleSomatotypeUpdate` e `handleCharacterPreset` (0 usos), import não usado de `SomatotypePad2D` (`App.svelte:44`) | Débito e confusão de manutenção | Remover; habilitar `svelte-check` com `noUnusedLocals` e ESLint `no-unused-vars` no CI |
| **P2-08** | Skinning rígido na malha canônica: `Vertex::with_skinning(..., [joint,0,0,0], [1,0,0,0])` (`mesh.rs:400-407`) → 1 influência por vértice, sem suavização | LBS da Sprint 04 vai cisalhar nas emendas entre regiões | Gerar até 4 influências por vértice (distância às cápsulas/ossos), normalizar pesos, teste de soma = 1 |
| **P2-09** | Observabilidade ausente: ~20 pontos com `catch(() => {})`/`console.error` silenciosos (`App.svelte:539, 583, 605, …; webgpu_renderer.ts:214`) | Falhas invisíveis em campo | Logger estruturado (níveis + contexto), toast de erro, comando de "exportar diagnóstico" |
| **P2-10** | Higiene de repositório: ZIP de 1,7 MB versionado (`modelos/feminino/animebasemesh (1).zip`), GLBs duplicados em `assets/models` **e** `public/models` (744 KB cada par), sem Git LFS, sem `.github/` (CI) | Repositório pesado e sem portão de qualidade | Remover ZIP e duplicatas, Git LFS para binários, CI com `cargo fmt/clippy/test` + `svelte-check` + `vite build` + `node --test` |
| **P2-11** | `historyService.undo/redo` sem `try/finally`: se `applySnapshot` lançar, `isExecutingHistory` fica `true` e o histórico **trava para sempre** (`history_service.ts:104-142`) | Falha catastrófica silenciosa | `try/finally` + telemetria do erro |
| **P2-12** | Coalescência do histórico por relógio (500 ms, `history_service.ts:70-79`): um arrasto longo gera vários passos de undo | UX de undo imprevisível | Coalescer por **sessão de gesto** (pointerdown→pointerup), não por tempo |

---

## 7. MELHORIAS (P3) — para "melhor do mercado", além de correto

| ID | Melhoria | Justificativa competitiva |
| :-- | :--- | :--- |
| **P3-01** | Filtros avançados no inspetor: "somente alterados", por zona, por dimorfismo (M/F), favoritos, busca por id **e** nome **e** tags semânticas | VRoid/CC4 têm busca limitada; 157 sliders exigem triagem |
| **P3-02** | Presets de usuário + export/import de morfologia (`.anigomorph` JSON) e compartilhamento | Ecossistema/mercado de presets (diferencial The Sims 4/CC) |
| **P3-03** | Randomizador controlado, mistura (blend) entre presets, simetria L/R e espelhamento de sliders unilaterais | Fluxo de concept art acelerado |
| **P3-04** | Perfis exigidos pela spec e ainda genéricos: busto **copa A–G** e **4 perfis de glúteos** (Sprint 03 §4.12/§4.15) | Requisito de spec pendente |
| **P3-05** | Campos numéricos editáveis + unidades reais (cm, mm, %) + step adaptativo + duplo-clique para reset | Precisão para produção (Design Doll tem) |
| **P3-06** | Highlight da região anatômica no viewport ao focar/passar o mouse no slider, com "custo" (nº de vértices afetados) | Feedback espacial essencial com 157 controles |
| **P3-07** | Comparação A/B (antes/depois) e captura de prova integrada ao inspetor (salva baseline automaticamente) | Evidência de validação contínua (Regra 1.1) |
| **P3-08** | Documentação viva da workspace: ADR do motor de morphs, contrato de sliders, matriz de cobertura publicada | Manutenibilidade enterprise e onboarding |

---

## 8. MATRIZ DE CONFORMIDADE COM `ANIGO_INVIOLABLE_RULES.md`

| Regra | Status | Evidência / Observação |
| :--- | :---: | :--- |
| **1.1** Validação visual obrigatória via `anigo-mcp` | ⚠️ **Não atendida para esta workspace** | Existem ferramentas MCP de personagem (`main.rs:458-510`) e baselines `test_render_sprint03_*_morph.png`, mas **não há evidência** de que os 157 sliders, os 6 presets e o pad tenham sido exercitados e inspecionados. Os presets estavam quebrados até esta auditoria (P0-01), logo não podem ter sido validados |
| **1.2** Módulos backend com suíte `cargo test` + relatório | ⚠️ **Parcial** | 50 testes Rust (tactile 6, mesh 12, morph 3, morph_catalog 4, somatotype 2), **0 para o caminho TS**. Não executável neste ambiente (sem toolchain) |
| **1.3** Autorização para automação de mouse/teclado | ✅ N/A | Nenhuma ação desse tipo foi executada nesta auditoria |
| **2.1** Proibido mockup de viewport | ✅ **Conforme** | Canvas WebGPU real com WGSL (com fallback WebGL2 legítimo) |
| **2.2** Proibido emojis na UI | ❌ **Violado** | `AnatomyInspector.svelte:197` usa 🔍; glifos ♀♂↺✕▼▶ em `:144, :156, :213, :229, :232, :246` |
| **2.3** Código limpo/robusto — Rust sem `unwrap` crítico | ❌ **Violado** | 41 ocorrências; `assert_eq!` em produção (`morph.rs:199`) |
| **2.3** Frontend TS estrito, componentes modulares | ❌ **Violado** | 24 erros de `svelte-check`; 8 ícones fora do padrão; estado duplicado |
| **2** Tolerância zero a provisórios | ❌ **Violado** | 121 sliders inertes (P0-04); 108 deltas genéricos (P1-03); 4 ferramentas-maquete (P1-05); pipeline GPU morto (P0-05) |
| **3** Pesquisa de estado da arte | ⚠️ **Indício contrário** | A implementação usa faixas de índice mágicas e constantes chutadas, não Green Coordinates/SDF/XPBD citados na spec |
| **4** Checklist de autoavaliação (7 perguntas) | ❌ **Reprovado** | Perguntas 1, 2, 4 e 6 respondem "não" para a workspace Personagem |

---

## 9. COBERTURA FUNCIONAL × ESPECIFICAÇÃO (SPRINT 03)

*Contagem exata obtida por extração programática dos ids lidos em `webgpu_renderer.ts` e dos `match` arms explícitos em `morph_catalog.rs`, cruzados com o catálogo canônico.*

| # | Zona (spec) | Sliders | Deformação no viewport (TS) | Delta explícito no Rust | Status |
| --: | :--- | --: | --: | --: | :--- |
| 1 | Proporções Globais & Silhueta | 8 | **0** | **0** | 🔴 |
| 2 | Crânio & Craniofacial | 11 | 6 | 8 | 🟠 |
| 3 | Olhos & Órbitas | 12 | 3 | 3 | 🔴 |
| 4 | Sobrancelhas | 6 | **0** | **0** | 🔴 |
| 5 | Nariz | 8 | 2 | 3 | 🔴 |
| 6 | Boca & Lábios | 8 | **0** | **0** | 🔴 |
| 7 | Mandíbula, Queixo e Linha V | 10 | 4 | 6 | 🟠 |
| 8 | Orelhas | 5 | 1 | 2 | 🔴 |
| 9 | Pescoço & Trapézio | 7 | 3 | 3 | 🟠 |
| 10 | Ombros, Clavículas e Dorsal | 8 | **0** | 2 | 🔴 |
| 11 | Tórax & Peitorais | 6 | 1 | 3 | 🔴 |
| 12 | Busto Feminino | 9 | 3 | 3 | 🔴 |
| 13 | Abdômen, Cintura e Flancos | 11 | 3 | 4 | 🟠 |
| 14 | Pelve & Quadris | 9 | 1 | 1 | 🔴 |
| 15 | Glúteos | 9 | 2 | 3 | 🔴 |
| 16 | Membros Superiores | 10 | 2 | 3 | 🟠 |
| 17 | Mãos & Dedos | 8 | **0** | **0** | 🔴 |
| 18 | Membros Inferiores | 12 | 5 | 5 | 🟠 |
| | **Total** | **157** | **36 (22,9%)** | **49 (31,2%)** | 🔴 |

**Destaques:** 4 zonas estão **100% inertes nos dois motores** — Proporções Globais (inclui os 3 sliders de somatótipo e os 4 de proporção/régua de cabeças, que na spec são `BoneDelta`), Sobrancelhas, Boca & Lábios e Mãos & Dedos — totalizando **30 sliders sem qualquer implementação real**. Ombros/Clavículas/Dorsal tem 0 no viewport (e `deltoid_muscle_volume`/`latissimus_dorsi_flare`, usados pelos presets, só existem no Rust).

**Observações:** (a) os 3 sliders de somatótipo (`somatotype_*`) e os de proporção global (`height_overall`, `head_to_body_ratio`, `torso_to_limb_ratio`, `spine_s_curvature`) são `BoneDelta`/`Dual` na spec e **não têm implementação esquelética** em nenhum dos motores (BOND) — hoje são inertes ou aproximados por escala de vértice; (b) a spec pede 148 sliders e o código tem 157 (P2-03).

---

## 10. PLANO DE EXECUÇÃO — O QUE PRECISA SER FEITO

> Princípio: **primeiro unificar, depois completar.** Implementar os 121 sliders faltantes sobre a arquitetura atual (dois estados, dois motores, sem histórico) apenas multiplicaria o débito.

### Fase 0 — Estabilização (1–2 dias) · *impede regressão enquanto se refatora*
1. Corrigir **P0-01** (contrato + aplicação atômica de preset com `try/catch`), **P0-02** (assinatura + sanitização `Number.isFinite`), **P0-03** (constante única de gênero) — são correções localizadas de altíssimo impacto.
2. Adicionar `try/finally` no histórico (P2-11), guardas `Number.isFinite` nos setters do renderer, e clamp central `clampCatalog()` (P0-09 parcial).
3. Portão de qualidade no CI (`.github/workflows/ci.yml`): `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `svelte-check --threshold error`, `vite build`, `node --test` — **o merge é bloqueado com qualquer erro** (P2-10). Instalar `svelte-check` como devDependency **com pnpm** (`pnpm add -D svelte-check` + commit do `pnpm-lock.yaml`); não usar npm, para não introduzir `package-lock.json` divergente.

### Fase 1 — Unificação arquitetural (3–5 dias) · *pré-requisito de todo o resto*
4. **Estado único de personagem** (`services/character_state.svelte.ts`): somatótipo, gênero, proporções, 157 sliders, modelo base, cabelo/roupa/acessórios; consumido por App, inspetor, pad, presets, MCP e persistência (P1-01).
5. **Motor único:** expor o motor Rust ao front (comando Tauri `apply_morph_state` + `get_morph_buffers`, ou WASM) e **desligar** `applyAnatomicalDeformations` do TS (P1-02, P0-05). O TS passa a apenas enviar comandos e desenhar buffers.
6. **Pipeline GPU real:** `uploadSparseMorphData()` + `dispatchSparseMorphs()` por frame, buffers persistentes com `writeBuffer`, dirty-flag + RAF (P0-05).
7. **Recálculo de normais** pós-deformação (compute pass de normais) (P0-06).

### Fase 2 — Persistência e histórico (2–3 dias)
8. Snapshot de projeto **versionado** com todos os parâmetros de personagem + validador + migrations (P0-08).
9. Histórico por **command pattern** sobre o estado único, com transações por gesto (P0-07, P2-12).
10. Autosave condicional a dirty + restauração do cache ao iniciar + erros visíveis (P0-08, P2-09).

### Fase 3 — Cobertura morfológica real (5–8 dias) · *maior esforço*
11. Deltas **reais e distintos** para os 157 sliders no motor Rust, com máscaras anatômicas (P1-03, P0-04).
12. Substituir as faixas de índice mágicas por **regiões nomeadas** validadas por teste de integridade (P1-04).
13. Camada **BOND** (bone deltas) para os 4 sliders de proporção global, com recálculo de bind pose (spec Sub-Sprint 3.4) — hoje inexistente.
14. **Dimorfismo contínuo** no caminho interativo (P1-12).
15. Portão de cobertura no CI: **todo slider exposto precisa ter delta não-nulo e distinto** — sem isso, o slider não pode ser exibido (P0-04).

### Fase 4 — Ferramentas da workspace (4–6 dias)
16. Painel **Rosto & Olhos** dedicado (49 sliders, 6 zonas) + preview facial (P1-06).
17. **Decisão explícita** sobre Cabelo/Vestuário/Acessórios/Pintura: implementar (Sprints 06/07/09/10) **ou** remover da barra; nunca manter maquete (P1-05).
18. Manipulação tátil com gate por workspace/ferramenta, clamp e transação de histórico (P0-09).
19. Correção de DPI no raycast (P1-11) + highlight de cápsula no hover.

### Fase 5 — Qualidade de produto (3–4 dias)
20. i18n completo do conteúdo de personagem (P1-08); ícones padronizados e sem emojis (P1-07); acessibilidade de todos os controles e teclado no pad (P1-09).
21. Robustez Rust: eliminar `unwrap/panic` de caminhos críticos (P2-01); cache do `SparseMorphSet` (P2-02); skinning com pesos suaves (P2-08).
22. Higiene: remover estado morto (P2-07), ZIP/duplicatas do Git + LFS (P2-10), loader glTF completo com erros na UI (P0-10).

### Fase 6 — Validação e diferenciação (2–3 dias + roadmap)
23. Protocolo MCP de validação visual (Seção 11.3) com screenshots de prova para: 157 sliders (antes/depois por zona), 6 presets, pad nos 3 vértices + centro, gênero 0/0.5/1, tátil em 13 segmentos.
24. Benchmark publicado: FPS, tempo de frame, VRAM, tempo de carga do modelo (P0-05).
25. Melhorias P3-01…P3-08 conforme roadmap.

**Estimativa total:** ~20–31 dias de engenharia focada, mais o protocolo de validação visual obrigatório.

---

## 11. SUÍTE DE TESTES EXIGIDA (hoje inexistente em sua maior parte)

### 11.1 Testes que precisam existir e passar

| Camada | Teste | Critério de aceitação |
| :--- | :--- | :--- |
| **Contrato de dados** | Catálogo TS ≡ catálogo Rust (ids, ordem, min/max/default) | 0 divergências (hoje: ✅ ids batem, mas o `step` só existe no TS) |
| **Contrato de dados** | Todo id referenciado (tátil, presets, MCP) existe no catálogo | 0 órfãos (hoje: **2**) |
| **Cobertura** | Todo slider do catálogo produz delta não-nulo e **distinto** dos demais da zona | 157/157 (hoje: 36 no TS, 49 no Rust) |
| **Unidade (TS)** | Pad 2D: ida-e-volta baricêntrico, clamp fora do triângulo, emissão do callback | soma = 1 ± 1e-6; nenhum `NaN` |
| **Unidade (TS)** | `clampCatalog`, `hexToRgb/rgbToHex`, `SomatotypeState` | 100% dos limites cobertos |
| **Integração** | Aplicar cada um dos 6 presets → todos os sliders dentro de `[min,max]`, somatótipo soma 1, modelo correto carregado | 6/6 sem exceção (hoje: 6/6 quebram) |
| **Integração** | Undo/Redo: 10 mutações de personagem → 10 undo → deep equal ao inicial | 100% |
| **Persistência** | Save → Load round-trip com 50 morphs alterados | Deep equal, com `schemaVersion` |
| **Integridade** | Nenhum `NaN/Inf` nas posições/normais após 157 sliders no máximo simultâneos | 0 ocorrências |
| **Render (GPU)** | Normais recalculadas: ≥95% das normais da região afetada mudam >1e-3 e `|N| = 1 ± 1e-5` | 100% |
| **E2E** | Troca de workspace/tool, undo/redo, autosave, abrir/salvar projeto | Fluxo completo sem erro |
| **Performance** | 40 sliders ativos a 120 FPS; rebuild < 8 ms; VRAM de morphs < 3,5 MB | Medido e publicado |

### 11.2 Infraestrutura mínima
- CI (`.github/workflows/ci.yml`) com matriz: Rust (`fmt`, `clippy -D warnings`, `test`) × Node (`svelte-check`, `vite build`, `node --test`).
- `svelte-check` configurado como **erro** (hoje há 24 erros e o build passa, porque `vite build` não checa tipos).
- Testes de GPU em runner dedicado (ou suíte headless via `anigo-renderer` + `cargo test`).

### 11.3 Protocolo de validação visual (Regra 1.1) — executar **após** as Fases 0–3
1. Subir `anigo-mcp` + janela nativa; capturar baseline neutra.
2. Para cada uma das 18 zonas: aplicar `min`, `default`, `max` em cada slider, capturar frame e anexar ao relatório com inspeção crítica (silhueta, terminador, contorno, normais).
3. Pad de somatótipo: 3 vértices + centro + 4 pontos externos (clamp).
4. Gênero: 0.0 / 0.25 / 0.5 / 0.75 / 1.0.
5. Presets: 6 capturas comparadas ao esperado da spec.
6. Tátil: 13 segmentos × arrasto horizontal/vertical.
7. Persistência: salvar, fechar, reabrir e comparar frame (deve ser idêntico).
> Cada item exige **inspeção humana crítica documentada** — automação sem inspeção não satisfaz a Regra 1.1.

---

## 12. INVENTÁRIO DE DÉBITO TÉCNICO (para limpeza imediata)

| Tipo | Item | Local |
| :--- | :--- | :--- |
| Código morto (funções com 0 usos) | `handleSomatotypeUpdate`, `handleCharacterPreset` | `App.svelte:234, 276` |
| Import não utilizado | `SomatotypePad2D` | `App.svelte:44` |
| Estado morto | `eyeTilt` (0 usos), `eyeScale`, `chinWidth`, `jawWidth` (só escritos) | `App.svelte:302-306` |
| Inertes (nunca chegam ao motor) | `hairVolume/Thickness/Curvature/Strands`, `clothLayer/Tension/Rigidity/Gravity`, `activeSocket`, `accessoryScale/OffsetXYZ` | `App.svelte:309-327`, painéis `:1673-1786` |
| Parâmetro morto | `updateProportions(record = false)` — nunca chamado com `true` | `App.svelte:1179` |
| CSS órfão | `.badge-warn` | `App.svelte:3387` |
| Pipeline morto | `uploadSparseMorphData` + `dispatchSparseMorphs` (bind group sempre nulo) | `webgpu_renderer.ts:1023, 2269` |
| Cache write-only | `localStorage["anigo_autosave_cache"]` nunca lido | `autosave_service.ts:76` |
| Arquivos versionados indevidamente | `modelos/feminino/animebasemesh (1).zip` (1,7 MB), GLBs duplicados em `assets/` e `public/` | repo |
| Componente fora de lugar | `LightingControls.svelte` (workspace Iluminação) dentro de `components/character/` | pasta |
| Divergências de domínio | 3 valores diferentes de "somatótipo neutro" (0,0,0 / 0.33,0.34,0.33 / 1/3 cada) | `AnatomyInspector:126`, `App:216-218`, `somatotype.rs:22` |

---

## 13. CONCLUSÃO

A workspace Personagem **parece** pronta e **não está**. Em termos de engenharia de produto, o estado atual é de um protótipo de fachada: a interface promete 157 controles anatômicos, presets de fábrica, somatótipo contínuo e manipulação tátil, mas o caminho até a GPU sustenta apenas 36 controles, com presets que lançam exceção, um pad que produz `NaN`, gênero invertido, normais desatualizadas, undo que ignora o corpo e um arquivo de projeto que não salva o personagem.

**Recomendação formal:** não declarar a Sprint 03 (nem qualquer sprint dependente) como concluída; tratar a Fase 0 como *hotfix* imediato e as Fases 1–3 como **pré-requisito de release comercial**. Com as Fases 0–5 executadas e o protocolo de validação visual (Seção 11.3) cumprido, a workspace atinge o patamar "Padrão Ouro" definido em `ANIGO_INVIOLABLE_RULES.md` e o objetivo declarado de superar VRoid/Design Doll/Clip Studio Paint 3D.

---

### ANEXO A — Comandos para reproduzir esta auditoria

```bash
# Dependências (pnpm, conforme pnpm-lock.yaml do repositório)
pnpm install

# Build de produção (passa, mas NÃO checa tipos)
npx vite build

# Checagem de tipos — hoje: 24 erros + 1 aviso
pnpm add -D svelte-check          # ainda não é dependência do projeto
npx svelte-check --threshold warning --output human

# E2E existente (27 passam; são checagens estáticas superficiais)
node --experimental-strip-types --test tests/e2e/workspace_pipeline.test.ts

# Suíte Rust (requer toolchain; 50 testes)
cargo test --workspace

# Diff de cobertura de sliders (TS × Rust) e contagem de arms/fallback
node -e "/* extrai ids de activeMorphWeights.get('…') em webgpu_renderer.ts
           e de MorphSliderDef em morph_catalog.rs, compara com o catálogo */"
```

### ANEXO B — Artefatos a revisar manualmente (não validáveis neste ambiente)
- `baselines/test_render_sprint03_male_morph.png` e `..._female_morph.png` — verificar se mostram o manequim anatômico real ou o fallback de cubo.
- `SPRINTS_PADRAO_OURO/SPRINT_03.md` (spec) × `src/services/morph_catalog.ts` (157 ids) — fechar a divergência 148 × 157.
- `RELATORIO_INVESTIGACAO_SPRINT_03.md` (68 KB) — conferir se há achados já conhecidos e não resolvidos que se somam a este relatório.
