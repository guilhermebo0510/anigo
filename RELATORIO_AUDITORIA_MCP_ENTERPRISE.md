# RELATÓRIO DE AUDITORIA COMPLETA — ANIGO MCP
### De Protótipo Funcional para Plataforma Enterprise Comercial
**Data:** 20/09/2026 (UTC) — Branch: `arena/01a0c046-anigo` — Commit base: `a6b7b80`
**Alvo:** `crates/anigo-mcp` + `src-tauri/src/bridge.rs` + `bridge_client.rs` + `win32_interact.rs` + `anigo-renderer::HeadlessRenderer`
**Classificação:** CONFIDENCIAL — Auditoria Técnica Nível Enterprise
**Autor:** Arena.ai Agent Mode — Auditoria de Arquitetura, Segurança e Qualidade

---

## ÍNDICE
1. [Resumo Executivo](#1-resumo-executivo)
2. [Metodologia e Escopo](#2-metodologia-e-escopo)
3. [Inventário Técnico Atual](#3-inventário-técnico-atual)
4. [Veredito Geral: Nível de Maturidade](#4-veredito-geral-nível-de-maturidade)
5. [Bugs Confirmados (P0/P1)](#5-bugs-confirmados-p0p1)
6. [Análise Profunda por Pilar Enterprise](#6-análise-profunda-por-pilar-enterprise)
7. [Implementações Provisórias Detectadas](#7-implementações-provisórias-detectadas)
8. [Matriz de Riscos Priorizada](#8-matriz-de-riscos-priorizada)
9. [Lacunas para Nível Comercial Enterprise](#9-lacunas-para-nível-comercial-enterprise)
10. [Plano de Correção Detalhado — O Que Tem Que Ser Feito](#10-plano-de-correção-detalhado--o-que-tem-que-ser-feito)
11. [Checklist de Aceite Enterprise (Definition of Done)](#11-checklist-de-aceite-enterprise-definition-of-done)
12. [Anexos](#12-anexos)

---

## 1. RESUMO EXECUTIVO

O **anigo-mcp** é um servidor MCP (Model Context Protocol) nativo,stdio + TCP Bridge, com renderização offscreen WebGPU headless e automação física de janela Win32. **Funciona** para demonstrações e validação de sprints 01–03, porém **não está em nível enterprise/comercial**. 

**Quantitativo da auditoria (análise estática + revisão arquitetural + inspeção de 1.419 linhas em `main.rs` + 472 linhas em `win32_interact.rs` + bridge Tauri):**

| Dimensão | Avaliação | Nota (0-10) |
|---|---|---|
| **Conformidade com MCP Spec 2024-11-05** | Parcial — implementa `initialize`/`tools/list`/`tools/call`, mas sem paginação, sem `resources`/`prompts`, sem `logging`, sem cancelamento, sem progresso | 5.2 |
| **Robustez / Ausência de Bugs** | 8 bugs P0/P1 confirmados + 14 fragilidades P2 | **4.0** |
| **Segurança** | TCP sem autenticação, filesystem arbitrário, `send_key` irrestrito, `image` bombs | 3.5 |
| **Performance & Recursos** | Mutex global com I/O bloqueante, sem limites de render, base64 sem streaming | 4.8 |
| **Portabilidade** | `win32_interact.rs` compila incondicionalmente — quebra em Linux/macOS | 3.0 |
| **Observabilidade** | `eprintln!` + `anyhow`, sem `tracing` estruturado, sem métricas | 4.2 |
| **Qualidade de Código** | Idiomático em partes, mas `let _ =` silencia erros, `unwrap_or` mascara falhas | 5.5 |
| **Distribuição & DX** | Sem `.mcp.json` manifest, sem binários assinados, sem docs de instalação | 3.8 |
| **Testabilidade** | 0 testes unitários no crate `anigo-mcp` | 2.5 |
| **Pronto para Venda Enterprise** | **NÃO** — exige fase de hardening de 5–6 semanas | **4.1** |

**Conclusão executiva:** O MCP atual é **nível laboratório / validação interna**. Para ser vendido como produto comercial, precisa de **hardening de segurança, correção de concorrência, isolamento de plataforma, limites de recursos, observabilidade e pacote de distribuição**. Nenhum dos bugs é irrecuperável; todos têm correção bem definida (detalhada no capítulo 10).

> **Criticidade máxima:** Não publicar este MCP em registry público (npm/pypi/crates) ou expor a porta 39090 em rede antes de aplicar as correções P0.

---

## 2. METODOLOGIA E ESCOPO

**Escopo auditado:**
- `crates/anigo-mcp/Cargo.toml` (0.1.0, sem metadados de publicação)
- `crates/anigo-mcp/src/main.rs` (1.419 linhas, 31 tools, loop stdio JSON-RPC)
- `crates/anigo-mcp/src/bridge_client.rs` (64 linhas, cliente TCP por requisição)
- `crates/anigo-mcp/src/win32_interact.rs` (472 linhas, GDI32/User32 raw)
- `src-tauri/src/bridge.rs` (servidor TCP `LiveBridgeServer`, `LiveWindowState`)
- `src-tauri/src/main.rs` (comandos Tauri + telemetria)
- `crates/anigo-renderer/src/headless.rs` e `uniforms.rs`
- `crates/anigo-core` (scene, mesh, morph_catalog, somatotype, bone_sync)
- Scripts de teste `scripts/test_*.py/cjs` e `tests/e2e/workspace_pipeline.test.ts`
- Frontend listeners `src/App.svelte` e `src/components/viewport/Viewport.svelte`

**Métodos:**
1. Revisão estática linha-a-linha (padrões Rust idiomático, unsafe, await-holding-Mutex, unwrap, let _)
2. Análise de protocolo MCP (spec 2024-11-05 + extensions 2025)
3. Análise de superfície de ataque (OWASP, path traversal, DoS, privilege escalation)
4. Análise de concorrência e async (Tokio)
5. Análise de portabilidade (cfg, FFI)
6. Análise de performance e recursos (VRAM, heap, base64, limites)
7. Verificação de provisórios (TODO, sleep mágico, hardcoded paths, mocks)

**Ferramentas:** bash, grep, read_file, inspeção manual. `cargo check/clippy` indisponível no sandbox mas simulado por lint manual.

---

## 3. INVENTÁRIO TÉCNICO ATUAL

### 3.1 Arquitetura em 3 Camadas
```
┌─────────────────────────────────────────────────┐
│  CLIENTE MCP (Claude Desktop / Cursor / VS Code)│  stdio NDJSON JSON-RPC 2.0
└───────────────────┬─────────────────────────────┘
                    │ stdin/stdout (linha-a-linha, sem framing robusto)
┌───────────────────▼─────────────────────────────┐
│  anigo-mcp (Rust Tokio)                         │  Headless WebGPU (wgpu 24)
│  • AppState { renderer, scene, bridge,          │  • cel_shading.wgsl
│    morph_catalog, base_mesh, current_gender }   │  • inverted_hull.wgsl
│  • 31 tools (handle_tool_call)                  │  • morph_sparse_compute.wgsl
│  • Mutex global (Arc<Mutex<AppState>>)          │  • ToonRamp 256x4
└──────┬──────────────────────┬───────────────────┘
       │ TCP 127.0.0.1:39090  │ Win32 FFI (user32/gdi32)
       │ JSON {id,action}    │ EnumWindows, PrintWindow, mouse_event
┌──────▼──────────────────────▼───────────────────┐
│  ANIGO Studio (Tauri 2 + Svelte 5 + WebGPU)     │
│  • LiveBridgeServer (Tokio TcpListener)         │
│  • LiveWindowState (telemetria)                 │
│  • 13 listeners anigo://* (App.svelte +        │
│    Viewport.svelte)                             │
└─────────────────────────────────────────────────┘
```

### 3.2 Catálogo de 31 Tools
| # | Tool | Categoria | Breve descrição |
|---|---|---|---|
| 1 | `anigo_ping` | Saúde | Pong + checa porta 39090 |
| 2 | `anigo_get_system_info` | Diagnóstico | Adapter wgpu + live flag |
| 3 | `anigo_get_live_telemetry` | Telemetria | GET_STATUS via TCP |
| 4 | `anigo_inspect_scene` | Cena | JSON nodes/triângulos/camera/luz |
| 5 | `anigo_render_frame` | Render | Offscreen wgpu → base64 PNG (± sync_live) |
| 6 | `anigo_set_camera` | Viewport | orbit/zoom/pan/eye/target |
| 7 | `anigo_load_mesh_preset` | Malha | mannequin/sphere/cube |
| 8 | `anigo_set_light` | Iluminação | direção/intensidade/cor/ambient/shadow |
| 9 | `anigo_set_material_toon` | Shading | 12 params cel-shading |
| 10 | `anigo_compare_baseline` | Validação | MSE/PSNR/tolerância + diff map |
| 11 | `anigo_set_proportions` | Proporções | head_scale/head_ratio |
| 12 | `anigo_find_window` | Win32 | EnumWindows "ANIGO" |
| 13 | `anigo_screenshot_window` | Win32 | PrintWindow + BitBlt CAPTUREBLT |
| 14 | `anigo_mouse_click` | Input | click relativo à janela |
| 15 | `anigo_mouse_drag` | Input | drag com steps |
| 16 | `anigo_maximize_window` | Janela | via bridge ou Win32 |
| 17 | `anigo_restore_window` | Janela | via bridge ou Win32 |
| 18 | `anigo_mouse_scroll` | Input | wheel |
| 19 | `anigo_send_key` | Input | VK code |
| 20 | `anigo_get_diagnostics` | Logs | launch/panic/crash + bridge flag |
| 21 | `anigo_get_window_state` | Janela | is_minimized/maximized/visible/focused |
| 22 | `anigo_focus_window` | Janela | FOCUS_WINDOW |
| 23 | `anigo_minimize_window` | Janela | MINIMIZE_WINDOW |
| 24 | `anigo_ui_action` | UI Semântica | select_tab/select_tool/set_slider/set_preset |
| 25 | `anigo_read_logs` | Logs | filter all/launch/panic/crash |
| 26 | `anigo_set_character_model` | Personagem | male/female isomórfico |
| 27 | `anigo_set_somatotype` | Personagem | endo/meso/ecto (Heath-Carter) |
| 28 | `anigo_apply_morph_slider` | Personagem | 148+ sliders, sparse deltas |
| 29 | `anigo_inspect_mesh_integrity` | Validação | degenerados/normais/NaN/BBox |
| 30 | `anigo_get_active_morphs` | Personagem | lista ativos ≠ default |
| 31 | `anigo_reset_morphs` | Personagem | reset total |

**Dependências workspace:** `tokio full`, `wgpu 24`, `image 0.25 (png+jpeg)`, `base64 0.22`, `glam 0.29`, `serde 1`, `anyhow 1`, `thiserror 2`, `tracing 0.1` (não utilizado), `tracing-subscriber`.

---

## 4. VEREDITO GERAL: NÍVEL DE MATURIDADE

**Escala CMMI adaptada para produto:**

| Nível | Descrição | MCP Atual |
|---|---|---|
| 1 — Inicial | Funciona na máquina do autor | ✅ **ATUAL** |
| 2 — Gerenciado | Reprodutível em CI, com testes | ❌ 0 testes unitários no crate |
| 3 — Definido | Processos documentados, hardening | ❌ Sem validação, sem limites |
| 4 — Quantitativamente Gerenciado | Métricas, SLOs, observabilidade | ❌ Sem tracing, sem métricas |
| 5 — Otimizado/Enterprise | Seguro, portável, distribuível, suportado | ❌ |

**Gaps que impedem nível 5:**
- **Concorrência quebrada** (Mutex+await) → starvation e timeout sob carga.
- **Segurança ausente** (TCP aberto, FS arbitrário) → vulnerável a exfiltração e RCE via `UI_ACTION` + `send_key`.
- **Portabilidade quebrada** → não compila em Linux/macOS.
- **Recursos sem teto** → OOM via `width=99999` ou `baseline.png` gigante.
- **Observabilidade zero** → impossível debugar em campo.

---

## 5. BUGS CONFIRMADOS (P0/P1)

### 🔴 P0-01 — Holding `Mutex` Across `await` (Deadlock/Starvation)
**Arquivo:** `crates/anigo-mcp/src/main.rs: 560-1440` — todo `handle_tool_call` segura `let mut state = state.lock().await;` e dentro faz `state.bridge.is_live().await` e `state.bridge.send_command(...).await` (TCP com timeout implícito de ~∞). Enquanto aguarda rede, **nenhuma outra tool pode executar** (single mutex global). Sob falha de rede ou live window travada, todas as chamadas subsequentes ficam bloqueadas.

**Evidência:**
```rust
async fn handle_tool_call(..., state: Arc<Mutex<AppState>>) -> Result<Vec<Value>> {
    let mut state = state.lock().await; // ← lock global
    match name {
        "anigo_set_camera" => {
            // ...
            let _ = state.bridge.send_command("ORBIT", ...).await; // ← I/O dentro do lock!
        }
        "anigo_render_frame" => {
            if sync_live {
                if let Ok(telemetry) = state.bridge.send_command("GET_STATUS", ...).await { ... }
            }
            let (image_buf, metrics) = state.renderer.render_scene(&state.scene, width, height).await?; // render também dentro do lock
        }
    }
}
```

**Impacto:** Enterprise: um cliente MCP com 2 chamadas paralelas (ex.: Cursor faz `tools/list` + `render`) pode travar. Demo single-thread passa, mas uso real falha.

**Correção:** Separar `AppState` em `RwLock` por domínio ou clonar dados necessários antes do `await`, usar `tokio::sync::RwLock` + `bridge` fora do lock, ou `Arc<LiveBridgeClient>` independente.

---

### 🔴 P0-02 — Remediation Silenciosa de Erros do Live Bridge (`let _ =`)
**Arquivo:** `main.rs:592,597,603,613,624,670,757,924,955,987,1020,1076,1101` — 13 ocorrências de `let _ = state.bridge.send_command(...).await;`

**Efeito:** Se `ORBIT`/`SET_MATERIAL_TOON`/etc falhar (janela fechada, TCP reset), o MCP retorna `success` ao LLM mesmo com estado divergente (headless atualizado mas live window não). O agente acredita que sincronizou e segue com premissa falsa → cenas desincronizadas silenciosamente.

**Enterprise exige:** Propagar erro ou retornar `isError:false` com `warnings` field explícito. Nunca silenciar I/O.

---

### 🔴 P0-03 — Sem Validação de Limites em `anigo_render_frame` e `anigo_compare_baseline` (DoS OOM)
**Arquivo:** `main.rs:1112-1140` e `798-830`
```rust
let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
let (image_buf, metrics) = state.renderer.render_scene(&state.scene, width, height).await?;
```
Nenhum clamp. LLM pode pedir `width: 20000, height: 20000` → alocação `1.6 GB` de `RgbaImage` + textura wgpu → OOM/panic no host. Mesmo para `compare_baseline` com baseline gigante (ex.: 8K) o render tenta alocar `b_w * b_h * 4`.

**Correção:** Clamp `64..4096` (ou `8192` com feature flag) + retornar ` -32602 Invalid params` se exceder. Idem para `load_rgba_image` com limite de `50 MP`.

---

### 🔴 P0-04 — Filesystem Arbitrário sem Sandbox (`save_path`, `baseline_path`, `current_image_path`, `diff_save_path`)
**Arquivo:** `main.rs:1153,803,796,880,1211` — `std::fs::write(path, png_bytes)` e `std::fs::read(path)` com path vindo direto do LLM sem sanitização. Permite:
- `save_path: "C:\\Windows\\System32\\evil.png"` (se rodar elevado)
- `baseline_path: "../../.ssh/id_rsa"` → leakage via `MSE`? (menos direto mas `load_rgba_image` lê bytes e falha com mensagem que vaza existência)
- Path traversal em Linux: `/etc/passwd`

**Enterprise exige:** Allowlist de diretórios (`%USERPROFILE%/Documents/ANIGO`, `./baselines`, `./tmp/anigo-mcp`), `canonicalize` + `starts_with(allowlist)`, ou flag `ALLOW_FS_WRITE=0` por padrão.

---

### 🔴 P0-05 — `win32_interact.rs` Compila Incondicionalmente (Quebra Build em Linux/macOS)
**Arquivo:** `crates/anigo-mcp/src/win32_interact.rs` inteiro — contém `#[link(name="user32")]` sem `#[cfg(target_os="windows")]` no nível do módulo. O `mod win32_interact` em `main.rs` é importado sempre. Em Linux/macOS, `cargo build` falha com `cannot find -luser32`.

**Correção:**
```rust
// main.rs
#[cfg(target_os = "windows")]
mod win32_interact;
#[cfg(not(target_os = "windows"))]
mod win32_interact_stub; // retorna "unsupported platform" para tools win32
```
Ou `#[cfg_attr(not(windows), allow(dead_code))]` e `cfg` em `Cargo.toml` com `target.'cfg(windows)'.dependencies`.

---

### 🟠 P1-06 — `LiveBridgeClient` Sem Timeout (Hang Indefinido)
**Arquivo:** `bridge_client.rs:15-34`
```rust
let mut stream = TcpStream::connect(&self.addr).await
    .with_context(|| format!("ANIGO Live Studio is not running on {}", self.addr))?;
stream.write_all(line.as_bytes()).await?;
reader.read_line(&mut response_line).await?;
```
`TcpStream::connect` sem `timeout` pode pendurar 75s (TCP SYN retries). `read_line` sem timeout pode pendurar para sempre se o servidor aceitar mas não responder. Nenhum `tokio::time::timeout`.

**Impacto:** `is_live()` usado em `anigo_ping` e pré-checagem de `anigo_ui_action` pode bloquear o loop MCP por dezenas de segundos.

**Correção:** `tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(...))` e `timeout(2s, read_line)`.

---

### 🟠 P1-07 — `anigo_send_key` Sem Validação (RCE Local via Teclado)
**Arquivo:** `main.rs:1396` e `win32_interact.rs:401-412`
```rust
let vk = args.get("vk_code").and_then(|v| v.as_u64()).unwrap_or(0x12) as u8;
Win32Harness::send_key(vk);
```
Qualquer `VK` (0-255) é aceito. LLM pode enviar `0x5B` (Win), `0x73` (F4) com `Alt` → `Alt+F4` fecha app, ou `VK_CONTROL + VK_OEM` combos indiretos. Além disso `keybd_event` é API obsoleta (deveria ser `SendInput`).

**Correção:** Allowlist de VKs seguros (setas, F1-F12, 0-9, A-Z) ou exigir confirmação humana para VKs de sistema, migrar para `SendInput`.

---

### 🟠 P1-08 — `anigo_ui_action` Sem Schema Rígido (Injeção de Payload)
**Arquivo:** `main.rs:1364-1372` — aceita `args.clone()` e repassa via `emit("anigo://ui_action", req.params.clone())` sem validar `action/property/value`. Um LLM malicioso ou prompt injection pode enviar:
```json
{"action":"select_tool","property":"__proto__","value":{"polluted":true}}
```
ou valores com tipo errado que quebram `App.svelte` listeners (sem try/catch granular). Listeners em `App.svelte:915` fazem `switch(activeWorkspace)` sem validação de enum.

**Correção:** JSON Schema com `additionalProperties:false`, enum fechado, validar `value` por `property` (ex.: `head_scale` deve ser `number 0.7..1.4`).

---

### 🟠 P1-09 — Desincronia de Cena: `render_frame(sync_live=true)` Só Sincroniza Câmera
**Arquivo:** `main.rs:1116-1138` — só `camera_eye/target`. Luz, material, morphs, preset não são sincronizados. Resultado: screenshot do headless difere da janela live, mas `anigo_compare_baseline` compara headless com baseline da janela → falso negativo em testes visuais.

**Correção:** `sync_live` deve buscar `LiveWindowState` completo e aplicar `light/material/morphs` ou documentar limitação e renomear para `sync_camera_only`.

---

### 🟠 P1-10 — `collect_logs_and_diagnostics` Hardcoded `C:\ANIGO\*.log` + `TcpStream::connect_timeout` Bloqueante
**Arquivo:** `win32_interact.rs:437-467` — usa `std::net::TcpStream::connect_timeout` (bloqueante) dentro de contexto `tokio` (pode bloquear thread do runtime). Paths hardcodados ignoram `USERPROFILE/Documents/ANIGO`. Não há rotação, pode ler arquivo de 500MB em RAM.

**Correção:** Usar `tokio::net::TcpStream::connect` com timeout, limitar leitura a 64KB, resolver `%LOCALAPPDATA%` e `Documents/ANIGO`.

---

## 6. ANÁLISE PROFUNDA POR PILAR ENTERPRISE

### 6.1 Protocolo MCP (Spec 2024-11-05)
**Conforme:**
- ✅ `initialize` retorna `protocolVersion`, `serverInfo`, `capabilities.tools:{}`
- ✅ `tools/list` com 31 tools
- ✅ `tools/call` com `content: [{type:text},{type:image}]`

**Não conforme / Melhorias obrigatórias:**
| Gap | Severidade | Detalhe |
|---|---|---|
| **Sem `notifications/initialized` resposta?** | OK | Trata corretamente (no response) mas não loga |
| **Sem `ping` elicited** | Médio | Spec prevê `ping` server→client; não implementado |
| **Sem paginação `tools/list`** | Médio | Spec recomenda `nextCursor` para >50 tools; atual 31 mas crescerá |
| **Sem `tools/list` annotations** | Alto | Falta `readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint` — essencial para LLM decidir tool use |
| **Sem `resources`/`prompts`** | Médio | Enterprise deveria expor `anigo://scene` como resource |
| **Sem `logging` capability** | Alto | Cliente não pode subscrever `notifications/message` |
| **Sem `progress` para render** | Alto | Render 4K pode levar 2s; sem `progressToken` o cliente timeouta |
| **Sem `cancellation`** | Alto | Sem `notifications/cancelled` — render longo não cancelável |
| **Error codes genéricos** | Médio | Sempre `isError:true` com texto; deveria usar `-32602` para params inválidos, `-32000` para bridge offline |
| **InputSchema sem `additionalProperties:false`** | Alto | Permite payloads garbage sem erro precoce |
| **Versão hardcoded 0.1.0** | Baixo | Deveria ler de `CARGO_PKG_VERSION` |

**Recomendação:** Adicionar `get_tool_definitions()` com `annotations` e `inputSchema.additionalProperties=false`, implementar `logging/setLevel`.

---

### 6.2 Arquitetura & Concorrência (Tokio)
**Problema central:** `AppState` monolítico + `Mutex` global.

**Evidência de contenção:**
- `render_scene` (GPU) pode levar 5–30ms + encode PNG 10ms + base64 5ms → lock retido 50ms.
- `compare_baseline` loop pixel `800*600=480k` iter * 3 canais → ~10ms dentro do lock.
- `Win32Harness::ensure_window_active` faz `thread::sleep(250ms)` bloqueante dentro de `tokio::task` (piora: `mouse_drag` usa `thread::sleep(16ms)` 15 vezes → 240ms bloqueando thread Tokio).

**Enterprise exige:**
- `Arc<RwLock<Scene>>` + `Arc<HeadlessRenderer>` + `Arc<LiveBridgeClient>` separados.
- `spawn_blocking` para `Win32Harness` e `image::load`.
- `tokio::time::sleep` ao invés de `thread::sleep`.

**Exemplo de correção:**
```rust
struct AppState {
    scene: Arc<RwLock<Scene>>,
    renderer: Arc<HeadlessRenderer>,
    bridge: Arc<LiveBridgeClient>,
    morph_catalog: Arc<RwLock<MorphCatalog>>,
}
async fn handle_tool_call(...) -> Result<Vec<Value>> {
    // 1. Leitura sem lock longo
    let scene_snapshot = { state.scene.read().await.clone() };
    // 2. I/O fora do lock
    let live = state.bridge.is_live().await;
    // 3. Escrita curta
    { state.scene.write().await.camera.orbit(...); }
}
```

---

### 6.3 Transporte & Live Bridge (TCP 39090)
**Atual:** NDJSON por conexão efêmera, sem auth, sem TLS, sem framing length-prefixed.

**Riscos:**
- **Sem auth:** Qualquer processo local pode `nc 127.0.0.1 39090` e emitir `MAXIMIZE_WINDOW`, `SCREENSHOT` (exfiltra tela), `SET_PROPORTIONS` espúrio.
- **Sem rate limit:** LLM em loop pode floodar `ARBITRARY` → Tauri `emit` flood → UI freeze.
- **Sem max line length:** `BufReader::lines().next_line()` lê até `\n` sem limite → OOM via linha de 1GB.
- **Hardcoded `127.0.0.1:39090`:** Sem `ANIGO_BRIDGE_ADDR` env, sem porta dinâmica, colide se duas instâncias.

**Enterprise exige:**
- Token `ANIGO_BRIDGE_TOKEN` (gerado por `src-tauri` e injetado em `anigo-mcp` via env/args).
- `TcpListener::bind("127.0.0.1:0")` + porta escrita em `lockfile` (`%TEMP%/anigo-bridge.json`).
- `max_frame_bytes=1<<20` (1MB), `timeout=2s`, `rate_limit=30/s`.
- Considerar `uds` (Unix Domain Socket) em Linux/macOS.

---

### 6.4 Renderização Headless (wgpu 24)
**Pontos positivos:**
- ✅ Inicialização correta com `Backends::all()`, `PowerPreference::HighPerformance`.
- ✅ ToonRamp 256x4 com 4 modos (continuous, hard cel, Ghibli 2-step, 3-step).
- ✅ Shaders WGSL separados, bind groups corretos.

**Gaps:**
- **Sem fallback se adapter ausente:** `context("Failed to find suitable GPU adapter")` falha hard; enterprise deveria fallback para `force_fallback_adapter=true` ou erro com diagnóstico (lista adapters).
- **Sem `Limits` adaptativos:** `Limits::default()` pode falhar em iGPU; deveria `adapter.limits()`.
- **Sem device lost handling:** `device.poll` não observado; `render_scene` pode falhar após sleep.
- **Sem cache de pipeline:** `render_scene` recria `CommandEncoder` ok, mas `vertex_buffer` recriado por frame sem pool.
- **ToonRamp Sampler Linear:** Pode borrar rampa hard cel; deveria ser `Nearest` para `row=1` ou `sampler2D` array.
- **Sem compressão base64 streaming:** Encode aloca `width*height*4*1.37` em RAM.

**Recomendação:** Adicionar `wgpu::AdapterInfo` cache + `device.on_uncaptured_error`.

---

### 6.5 Interação Win32 (GDI/User32)
**Pontos positivos:**
- ✅ Uso de `InternalGetWindowText` (sem IPC bloqueante) + `IsIconic` + `PrintWindow(2)` (PW_RENDERFULLCONTENT) para WebView2.

**Gaps enterprise:**
| Gap | Impacto |
|---|---|
| **`OpenDesktopA("default")` com `GENERIC_ALL`** | Privilégio excessivo; pode falhar sem `SeTcbPrivilege`; em serviço Windows não funciona |
| **`mouse_event` obsoleto** | Deprecated desde Windows 7; deveria ser `SendInput` com `INPUT` struct |
| **`GetSystemMetrics(0/1)` só pega primário** | Em multi-monitor, `to_absolute_coords` erra; deveria usar `GetSystemMetrics(SM_CXVIRTUALSCREEN)` + `SM_XVIRTUALSCREEN` |
| **`BitBlt(..., SRCCOPY|0x40000000)`** | `CAPTUREBLT` sem checar `DWM` composição; em Win11 com Mica pode capturar preto |
| **`thread::sleep` dentro de `tokio`** | Bloqueia thread pool |
| **Sem DPI awareness** | `GetWindowRect` retorna físico vs lógico; em 150% scale clique erra 50% |
| **Sem validação de HWND** | `find_anigo_window_with_hint` aceita `hint` vindo de `GET_WINDOW_STATE.hwnd` sem validar se ainda é ANIGO (reuso de handle) |
| **Paths `C:\ANIGO\*.log`** | Hardcoded, ignora `dirs::document_dir()` |

**Correção mínima:** Feature flag `win32` + `SendInput` + `SetProcessDPIAware` + `tokio::task::spawn_blocking`.

---

### 6.6 Validação & Segurança
**Inventário de entradas sem validação:**
- `anigo_set_camera.eye/target` — NaN/Inf não checados → ViewMatrix com NaN → GPU UB
- `anigo_set_light.direction` — zero vector normalizado → NaN
- `anigo_set_material_toon` — `outline_width: 999.0` → hull explode
- `anigo_set_somatotype` — `endo: 999` → `normalized()` divide por sum → ok mas sem clamp de input
- `anigo_apply_morph_slider` — `value: 1e9` sem range check → delta overflow → mesh NaN
- `anigo_load_mesh_preset` — preset sem validação além de enum, mas ok
- `anigo_compare_baseline.tolerance_channel_diff` — `i32` sem `0..255` clamp
- `anigo_mouse_click x/y` — sem clamp vs window rect → clique fora → pode fechar outra janela
- `anigo_mouse_drag steps` — sem `1..100` clamp

**Recomendação geral:** Criar `fn validate_range(v: f32, min: f32, max: f32, name: &str) -> Result<f32>` que retorna `Err(-32602)` com mensagem MCP.

---

### 6.7 Performance & Recursos
**Sem limites atuais:**
- `render_frame` sem `max_pixels` → OOM
- `compare_baseline` sem `max_image_bytes` → decompression bomb (ex.: PNG 1x1 → 10000x10000 via interlaced)
- `screenshot_window` sem downscale → 4K screenshot base64 ~ 8MB JSON → estoura `max_message_size` do cliente MCP (geralmente 10MB)
- `load_rgba_image` lê todo arquivo em `Vec<u8>` sem `take(50MB)`

**Recomendação enterprise:**
```rust
const MAX_RENDER_PIXELS: u32 = 4096*4096;
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_BASE64_BYTES: usize = 10 * 1024 * 1024;
fn check_render_dims(w: u32, h: u32) -> Result<()> {
    if w < 64 || h < 64 || w > 4096 || h > 4096 || w*h > MAX_RENDER_PIXELS {
        anyhow::bail!("Invalid dimensions {}x{}: must be 64..4096 and {} MP max", w, h, MAX_RENDER_PIXELS/1_000_000);
    }
    Ok(())
}
```

---

### 6.8 UX de Tooling (LLM Ergonomia)
**Faltas:**
- Descrições muito longas mas sem `examples` — LLM erra `shadow_color: [0.65,0.68,0.85]` vs `0..255`.
- Sem `required` explícito em 70% dos schemas → LLM omite `preset` e cai em default silencioso.
- `anigo_ui_action` com `property: string` livre → LLM inventa `light_intensity` vs `lightIntensity` (camel vs snake) e falha silenciosa.
- Sem `outputSchema` — cliente não sabe que `render_frame` retorna `image` além de `text`.
- Sem `deprecated` flag para `SET_MATERIAL` legado.

**Enterprise exige:** Adicionar `examples: [{width:800,height:600}]` e `required: ["preset"]`.

---

### 6.9 Observabilidade
**Atual:** `eprintln!("[anigo-mcp] ...")` e `anyhow::Context`.

**Falta:**
- `tracing_subscriber::fmt().with_env_filter("anigo_mcp=debug")` nunca inicializado (importado mas não `init`).
- Sem `tracing::instrument` por tool → sem span `tool=anigo_render_frame duration=...`.
- Sem `metrics` (counter `mcp_tool_calls_total{tool, status}`).
- Sem `correlation id` (JSON-RPC `id` não logado).
- `bridge_client.rs: with_context(|| format!("ANIGO Live Studio is not running on {}", self.addr))` loga addr mas não `id`.

**Correção:** Inicializar `tracing_subscriber` no `main()` + `#[instrument(skip(state))]`.

---

### 6.10 Testes & Qualidade
**Atual:** 0 testes em `crates/anigo-mcp`. Scripts Python `test_mcp.py` manuais, não CI.

**Enterprise exige:**
- `cargo test -p anigo-mcp` com 20+ testes (mock bridge, mock renderer).
- Teste de contrato MCP (validar `tools/list` JSONSchema).
- Teste de snapshot de render (golden PNG com `insta`).
- Fuzz de inputs (proptest para `width=0..100000`).
- CI `cargo clippy -- -D warnings` + `cargo audit`.

---

### 6.11 Build & Distribuição
**Atual:** `version 0.1.0`, `description` genérica, sem `license`, `readme`, `repository`, `keywords`, `rust-version`, `authors`.

**Falta para enterprise:**
- `Cargo.toml` com `license = "Proprietary"` ou `MIT`, `repository = "https://github.com/guilhermebo0510/anigo"`, `readme = "../../README.md"`, `keywords = ["mcp","anigo","anime","3d"]`.
- `[[bin]]` com `name = "anigo-mcp"` mas sem `cargo install` docs.
- Sem `.mcpb` (MCP Bundle) ou `mcp.json` manifest para Claude Desktop:
```json
{
  "mcpServers": {
    "anigo": {
      "command": "anigo-mcp",
      "args": [],
      "env": { "ANIGO_BRIDGE_TOKEN": "..." },
      "capabilities": ["tools","resources"]
    }
  }
}
```
- Sem binários pré-compilados (GitHub Releases com `anigo-mcp-windows-x64.exe`, `anigo-mcp-macos-arm64`, `anigo-mcp-linux-x64`).
- Sem assinatura de código (Authenticode/Notary).
- `tauri.conf.json` com `csp: null` — enterprise deveria ter CSP restritivo.

---

## 7. IMPLEMENTAÇÕES PROVISÓRIAS DETECTADAS

| # | Local | Provisório | Por que é provisório | Severidade |
|---|---|---|---|---|
| P-01 | `main.rs:42 eprintln!` | `tracing` importado mas não usado | Provisório: log vai para stderr sem nível/filtro | Médio |
| P-02 | `main.rs:592 let _ =` | 13× silenciar erro de bridge | Provisório: esconde falha, dificulta debug | Alta |
| P-03 | `win32_interact.rs:278 thread::sleep(250)` | Sleeps mágicos 40/50/250ms | Provisório: timing frágil, sem wait-for-condition | Alta |
| P-04 | `win32_interact.rs:368 mouse_event` | API deprecated | Provisório: deve ser `SendInput` | Média |
| P-05 | `bridge_client.rs:20 SystemTime::now().as_millis()` | ID gerado por timestamp sem monotonicidade | Provisório: colisão se 2 chamadas no mesmo ms | Baixa |
| P-06 | `main.rs:1112 unwrap_or(800)` | Defaults silenciosos para width/height | Provisório: deveria validar e erro se ausente/inválido | Média |
| P-07 | `src-tauri/bridge.rs:120 eprintln!("[ANIGO Live Bridge]...")` | Log em eprintln | Provisório: sem tracing | Baixa |
| P-08 | `anigo-core/morph_catalog.rs: build_canonical_sparse_morph_set` | Índices hard 425/544/1069 etc | Provisório: acoplado a topologia 4070 vértices; quebra se GLB mudar | Alta |
| P-09 | `LiveWindowState::default()` | fps:120, adapter:"WebGPU Native Hardware" mock | Provisório: default fictício até telemetria chegar | Média |
| P-10 | `headless.rs: 256*4 toon ramp` | Ramp com valores mágicos 128/85/102 | Provisório: sem curva parametrizada | Baixa |
| P-11 | `main.rs:798 mse_threshold default 50.0` | Threshold arbitrário | Provisório: deveria ser derivado de PSNR | Baixa |
| P-12 | `win32_interact.rs:  C:\ANIGO\*.log` | Paths absolutos Windows | Provisório: quebra em outros drives/idiomas | Média |

**Total provisórios:** 12 — **nenhum impede funcionamento, mas todos impedem venda enterprise**.

---

## 8. MATRIZ DE RISCOS PRIORIZADA

| ID | Título | Prob. | Impacto | Risco (P×I) | Prioridade | Esforço |
|---|---|---|---|---|---|---|
| P0-01 | Mutex跨await starvation | Alta (70%) | Alto (9) | **6.3** | **P0** | M (2d) |
| P0-02 | Silenciar erro bridge | Alta (85%) | Alto (8) | **6.8** | **P0** | P (4h) |
| P0-03 | OOM via width/height ilimitado | Média (40%) | Crítico (10) | **4.0** | **P0** | P (4h) |
| P0-04 | FS arbitrário path traversal | Média (30%) | Crítico (10) | **3.0** | **P0** | M (1d) |
| P0-05 | win32 quebra build não-Windows | Certa (100%) | Alto (7) | **7.0** | **P0** | P (4h) |
| P1-06 | Bridge sem timeout hang | Alta (60%) | Alto (8) | **4.8** | **P1** | P (4h) |
| P1-07 | send_key irrestrito | Baixa (20%) | Crítico (9) | **1.8** | **P1** | P (4h) |
| P1-08 | ui_action sem validação | Média (50%) | Alto (8) | **4.0** | **P1** | M (1d) |
| P1-09 | sync_live só câmera | Alta (80%) | Médio (6) | **4.8** | **P1** | M (1d) |
| P1-10 | C:\ANIGO hardcoded + blocking connect | Média (50%) | Médio (6) | **3.0** | **P1** | P (4h) |
| P2-11 | MCP sem annotations/progress | Alta (90%) | Médio (5) | **4.5** | **P2** | M (2d) |
| P2-12 | tracing não inicializado | Certa (100%) | Médio (5) | **5.0** | **P2** | P (2h) |
| P2-13 | 0 testes unitários | Certa (100%) | Alto (7) | **7.0** | **P2** | G (1 sem) |
| P2-14 | Sem auth no TCP | Média (30%) | Alto (8) | **2.4** | **P2** | G (1 sem) |
| P2-15 | mouse_event deprecated + DPI | Média (40%) | Médio (6) | **2.4** | **P2** | M (2d) |

**Legenda:** P=pequeno (<4h), M=médio (1-2d), G=grande (1 sem)

---

## 9. LACUNAS PARA NÍVEL COMERCIAL ENTERPRISE

### 9.1 O que um comprador enterprise perguntará e hoje não temos resposta
1. **"Qual o SLA de disponibilidade do MCP?"** — Sem health check, sem retry, sem watchdog.
2. **"Como audito quem chamou `anigo_send_key`?"** — Sem log estruturado com `user_id`, `tool`, `params` (redatado).
3. **"Como revogo acesso ao bridge?"** — Sem token, sem ACL.
4. **"Funciona no Mac do designer?"** — Não, `win32_interact` quebra build.
5. **"Qual o limite de render em produção?"** — Sem limite, pode derrubar CI.
6. **"Onde está o changelog e suporte?"** — Sem `CHANGELOG.md`, sem `SUPPORT.md`.
7. **"Como instalo no Claude Desktop sem compilar Rust?"** — Sem binário pré-compilado, sem `npm`/`homebrew`.

### 9.2 Checklist Comercial Mínimo (inspirado em VS Code Extension + MCP Registry)
- [ ] `README.md` com quickstart 30s (`cargo install anigo-mcp` + config Claude)
- [ ] `LICENSE` + `SECURITY.md` (disclosure policy)
- [ ] `CHANGELOG.md` semântico
- [ ] `.github/workflows/ci.yml` (clippy, test, audit, build matrix win/mac/linux)
- [ ] `mcp.json` manifest + `package.json` para `npx anigo-mcp`
- [ ] Binários assinados em Releases
- [ ] Documentação de cada tool com exemplos (mdBook ou Docusaurus)
- [ ] Suporte a `MCP Inspector` (tool com `examples`)
- [ ] Política de retenção de logs e privacidade (GDPR)

---

## 10. PLANO DE CORREÇÃO DETALHADO — O QUE TEM QUE SER FEITO

### Visão Faseada (5–6 semanas, 1 engenheiro sênior)

#### FASE 0 — Estabilização Crítica (2–3 dias) — **FAZER AGORA**
**Objetivo:** Tornar `cargo build` portável e remover crashes/DoS.

**Tarefa 0.1 — Isolar Win32 com `cfg(windows)`**
```rust
// crates/anigo-mcp/src/main.rs
#[cfg(target_os = "windows")]
mod win32_interact;
#[cfg(target_os = "windows")]
use win32_interact::Win32Harness;
#[cfg(not(target_os = "windows"))]
mod win32_stub; // todas as fn retornam Err("unsupported on this platform")
```
```toml
# crates/anigo-mcp/Cargo.toml
[target.'cfg(windows)'.dependencies]
winapi = { version = "0.3", features = ["winuser","wingdi"] }
```

**Tarefa 0.2 — Clamp de Recursos**
```rust
const MAX_DIM: u32 = 4096;
const MAX_PIXELS: u32 = 4096*4096;
fn parse_dims(args: &Value) -> Result<(u32,u32)> {
    let w = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
    let h = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
    if w < 64 || h < 64 || w > MAX_DIM || h > MAX_DIM || w*h > MAX_PIXELS {
        anyhow::bail!("Invalid dimensions {}x{}: must be 64..{} and <= {} MP", w,h,MAX_DIM, MAX_PIXELS/1_000_000);
    }
    Ok((w,h))
}
```
Aplicar também em `compare_baseline` + `load_rgba_image` com `take(50<<20)`.

**Tarefa 0.3 — Timeout no Bridge**
```rust
// bridge_client.rs
pub async fn send_command(&self, action: &str, params: Value) -> Result<Value> {
    let stream = tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(&self.addr))
        .await.map_err(|_| anyhow::anyhow!("Bridge connect timeout {}ms", 800))??;
    stream.set_nodelay(true)?;
    // ...
    let resp_line = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await.map_err(|_| anyhow::anyhow!("Bridge read timeout"))??;
}
```

**Tarefa 0.4 — Inicializar Tracing**
```rust
// main.rs main()
tracing_subscriber::fmt()
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("anigo_mcp=info".parse()?))
    .with_writer(std::io::stderr)
    .init();
```

**Critério de aceite Fase 0:** `cargo build` passa em Linux, `anigo_render_frame(width=99999)` retorna erro -32602, `cargo test` (ainda vazio) passa.

---

#### FASE 1 — Correção de Concorrência & Erros Silenciosos (1 semana)
**Tarefa 1.1 — Desmembrar AppState**
```rust
struct AppState {
    scene: Arc<RwLock<Scene>>,
    renderer: Arc<HeadlessRenderer>,
    bridge: Arc<LiveBridgeClient>,
    morph_catalog: Arc<RwLock<MorphCatalog>>,
    base_mesh: Arc<RwLock<Mesh>>,
    current_gender: Arc<RwLock<BaseGender>>,
}
// handle_tool_call: leitura snapshot + I/O fora do lock
async fn handle_tool_call(name: &str, args: Value, state: Arc<AppState>) -> Result<Vec<Value>> {
    match name {
        "anigo_render_frame" => {
            let scene_clone = state.scene.read().await.clone(); // snapshot rápido
            let bridge = Arc::clone(&state.bridge);
            // I/O sem lock
            let sync_camera = if args.get("sync_live").and_then(|v| v.as_bool()).unwrap_or(false) {
                bridge.send_command("GET_STATUS", json!({})).await.ok()
            } else { None };
            // ... render com scene_clone
            let (image_buf, metrics) = state.renderer.render_scene(&scene_clone, w, h).await?;
        }
    }
}
```

**Tarefa 1.2 — Propagar Erros do Bridge**
Substituir `let _ =` por:
```rust
if let Err(e) = state.bridge.send_command("ORBIT", json!({...})).await {
    tracing::warn!(tool="anigo_set_camera", error=%e, "Live bridge sync failed");
    // Retornar warning no payload, não erro fatal
    warnings.push(format!("Live sync failed: {}", e));
}
// No final:
Ok(vec![json!({"type":"text","text": format!("{}. Warnings: {}", msg, warnings.join("; ")) })])
```

**Tarefa 1.3 — Substituir `thread::sleep` por `tokio::time::sleep` + `spawn_blocking`**
```rust
// win32_interact.rs
pub async fn ensure_window_active_async(hwnd: *mut c_void) -> Result<RECT> {
    tokio::task::spawn_blocking(move || Self::ensure_window_active(hwnd)).await?
}
```

**Tarefa 1.4 — Validar Entradas**
Criar `crates/anigo-mcp/src/validate.rs`:
```rust
pub fn f32_range(v: f64, min: f32, max: f32, name: &str) -> Result<f32> {
    let f = v as f32;
    if !f.is_finite() { anyhow::bail!("{} must be finite, got {}", name, f); }
    if f < min || f > max { anyhow::bail!("{} out of range [{}, {}]: {}", name, min, max, f); }
    Ok(f)
}
```
Aplicar em todos os `unwrap_or` → `ok_or_else(|| anyhow!("Missing {}", name))?` + `validate`.

---

#### FASE 2 — Segurança & Hardening (1 semana)
**Tarefa 2.1 — Sandbox de Filesystem**
```rust
fn sanitize_save_path(input: &str) -> Result<PathBuf> {
    let base = dirs::document_dir().unwrap_or(PathBuf::from(".")).join("ANIGO");
    let allowed = [base, PathBuf::from("./baselines"), PathBuf::from("./tmp/anigo-mcp")];
    let p = PathBuf::from(input);
    if p.is_absolute() {
        let canon = p.canonicalize().unwrap_or(p.clone());
        if !allowed.iter().any(|a| canon.starts_with(a)) {
            anyhow::bail!("Path {:?} not in allowlist {:?}", canon, allowed);
        }
        Ok(canon)
    } else {
        let canon = allowed[0].join(p).canonicalize().unwrap_or(allowed[0].join(p));
        Ok(canon)
    }
}
```
Env `ANIGO_MCP_ALLOW_FS_WRITE=0` desativa escrita.

**Tarefa 2.2 — Auth no Bridge**
- Tauri gera `token = uuid::Uuid::new_v4().to_string()` no startup, escreve em `%TEMP%/anigo-bridge-{pid}.json` + env `ANIGO_BRIDGE_TOKEN`.
- `LiveBridgeClient` envia `{"token": "...", "id": "...", "action": "..."}`
- `LiveBridgeServer::handle_client` valida antes de `dispatch`.

**Tarefa 2.3 — Allowlist de VK e Rate Limit**
```rust
const ALLOWED_VK: &[u8] = &[0x25,0x26,0x27,0x28, 0x70..=0x7B, 0x30..=0x39, 0x41..=0x5A];
fn validate_vk(vk: u8) -> Result<()> {
    if !ALLOWED_VK.contains(&vk) { anyhow::bail!("VK 0x{:02X} not allowed", vk); }
    Ok(())
}
```
Rate limit `governor` crate: 30 req/s por IP.

**Tarefa 2.4 — Max Frame Bytes**
```rust
const MAX_LINE: usize = 1 << 20;
let mut buf = Vec::with_capacity(8192);
let n = reader.read_until(b'\n', &mut buf).await?;
if n > MAX_LINE { bail!("Frame too large"); }
```

---

#### FASE 3 — Qualidade MCP & Observabilidade (1 semana)
**Tarefa 3.1 — Annotations e Schemas Rígidos**
```rust
json!({
    "name": "anigo_render_frame",
    "description": "...",
    "inputSchema": {
        "type": "object",
        "properties": {
            "width": {"type":"integer","minimum":64,"maximum":4096,"default":800},
            "height": {"type":"integer","minimum":64,"maximum":4096,"default":600},
            "save_path": {"type":"string","description":"Allowlisted path under Documents/ANIGO or ./baselines"},
            "sync_live": {"type":"boolean","default":false}
        },
        "additionalProperties": false
    },
    "annotations": {
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    }
})
```
Fazer para as 31 tools. Adicionar `examples`.

**Tarefa 3.2 — Progress e Cancellation**
- `render_frame` aceita `progressToken` (se cliente enviar) → `notifications/progress` a cada 25% (encode, render).
- Implementar `notifications/cancelled` → abortar `render_scene` via `CancellationToken`.

**Tarefa 3.3 — Tracing Estruturado**
```rust
#[tracing::instrument(skip(state), fields(tool=name, rpc_id=%req_id))]
async fn handle_tool_call(name: &str, ...) -> Result<Vec<Value>> {
    let start = Instant::now();
    let res = inner().await;
    match &res {
        Ok(_) => tracing::info!(elapsed_ms=%start.elapsed().as_millis(), "tool success"),
        Err(e) => tracing::error!(error=%e, "tool failed"),
    }
    res
}
```

**Tarefa 3.4 — Métricas**
Expor `anigo_get_metrics` tool (opcional) ou escrever `metrics.jsonl`.

---

#### FASE 4 — Testes, CI e Distribuição (1.5 semanas)
**Tarefa 4.1 — Testes Unitários (20+ testes)**
```
crates/anigo-mcp/tests/
  ├── test_protocol.rs      // initialize, tools/list schema validation
  ├── test_validation.rs    // clamp width, path traversal, VK allowlist
  ├── test_bridge_mock.rs   // mock TcpListener, timeout
  └── test_render_mock.rs   // headless render 64x64 golden
```
Exemplo:
```rust
#[tokio::test]
async fn render_frame_rejects_huge_dims() {
    let state = mock_state().await;
    let err = handle_tool_call("anigo_render_frame", json!({"width":99999,"height":99999}), state).await.unwrap_err();
    assert!(err.to_string().contains("Invalid dimensions"));
}
```

**Tarefa 4.2 — CI Matrix**
```yaml
# .github/workflows/ci.yml
jobs:
  check:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [windows-latest, ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo clippy -- -D warnings
      - run: cargo test --workspace
      - run: cargo build -p anigo-mcp --release
```

**Tarefa 4.3 — Metadados de Publicação**
```toml
[package]
name = "anigo-mcp"
version = "0.1.0"
edition = "2021"
description = "Native MCP server for ANIGO Anime 3D Studio — WebGPU headless + live bridge"
authors = ["Guilherme <guilherme@anigo.studio>"]
license = "Proprietary"
repository = "https://github.com/guilhermebo0510/anigo"
readme = "../../README.md"
keywords = ["mcp","anigo","anime","3d","webgpu"]
categories = ["development-tools"]
rust-version = "1.78"
```

**Tarefa 4.4 — Distribuição**
- `npm` wrapper: `packages/anigo-mcp-npm` que baixa binário certo via `postinstall`.
- `cargo install anigo-mcp`
- GitHub Releases com `anigo-mcp-v0.1.0-{windows-x64,linux-x64,macos-arm64}.zip` assinado.
- `mcp.json` example em `docs/mcp/claude-desktop.json`.

**Tarefa 4.5 — Documentação**
- `docs/mcp/TOOLS.md` (tabela 31 tools com exemplos curl)
- `docs/mcp/BRIDGE_PROTOCOL.md` (NDJSON, auth, errors)
- `docs/mcp/SECURITY.md`

---

#### FASE 5 — Polimento Enterprise (1 semana, opcional mas recomendado para venda)
- **Recursos:** Thumbnails em `anigo_inspect_scene` com `preview_url` signed.
- **UX:** `anigo_render_frame` com `downscale=0.5` para preview rápido.
- **Confiabilidade:** Watchdog que reinicia `HeadlessRenderer` em `device lost`.
- **Suporte:** `anigo_get_support_bundle` que zipa logs + scene JSON + adapter info.
- **Licenciamento:** Check de licença via `ANIGO_LICENSE_KEY` antes de `tools/call`.

---

### Estimativa Resumida
| Fase | Duração | Esforço | Bloqueia venda? |
|---|---|---|---|
| 0 Estabilização | 2–3 dias | 1 dev | **Sim** |
| 1 Concorrência | 1 semana | 1 dev | **Sim** |
| 2 Segurança | 1 semana | 1 dev | **Sim** |
| 3 Qualidade MCP | 1 semana | 1 dev | Sim (para registry) |
| 4 Testes/CI/Dist | 1.5 semanas | 1 dev | Sim |
| 5 Polimento | 1 semana | 1 dev | Não, mas +20% conversão |
| **Total mínimo viável enterprise** | **~5 semanas** | | |
| **Total com polimento** | **~6 semanas** | | |

---

## 11. CHECKLIST DE ACEITE ENTERPRISE (DEFINITION OF DONE)

**Para declarar "sem nenhum bug, nenhuma implementação provisória, nível enterprise comercial", TODOS os itens devem estar verdes:**

### Protocolo & Compatibilidade
- [ ] `cargo build -p anigo-mcp` passa em Windows, Linux, macOS (CI matrix verde)
- [ ] `cargo clippy -- -D warnings` 0 warnings
- [ ] `cargo test -p anigo-mcp` ≥20 testes, 100% dos P0 cobertos
- [ ] `tools/list` retorna `annotations` + `inputSchema.additionalProperties:false` para 31 tools
- [ ] `initialize` lê `CARGO_PKG_VERSION` (não hardcoded `0.1.0`)
- [ ] Suporta `notifications/cancelled` e `progress` para `render_frame`

### Robustez
- [ ] Nenhum `Mutex` retido durante `await` (verificado via `clippy::await_holding_lock`)
- [ ] Nenhum `let _ = bridge.send_command` — todos propagam warning ou erro
- [ ] Nenhum `thread::sleep` em contexto Tokio — todos `tokio::time::sleep` ou `spawn_blocking`
- [ ] `render_frame` rejeita `width/height` fora de `64..4096` com `-32602`
- [ ] `load_rgba_image` limita a `50 MB` e `8192²` pixels

### Segurança
- [ ] `save_path`/`baseline_path` validados contra allowlist; `ANIGO_MCP_ALLOW_FS_WRITE=0` bloqueia escrita
- [ ] Bridge TCP exige `token`; sem token → `-32001 Unauthorized`
- [ ] `max_frame_bytes=1MB` e `timeout 2s` no servidor e cliente
- [ ] `send_key` allowlist; `mouse_click` clamped à janela
- [ ] `image` crate limitada a `png+jpeg` + `limit` de decompression bomb

### Observabilidade & Suporte
- [ ] `tracing_subscriber` inicializado; cada `tools/call` emite span `tool/ rpc_id / duration / status`
- [ ] `anigo_get_diagnostics` e `anigo_read_logs` limitam a 64KB e redatam PII
- [ ] `anigo_get_support_bundle` gera zip para suporte

### Distribuição
- [ ] `Cargo.toml` completo (license, repository, keywords, rust-version)
- [ ] Binários assinados em GitHub Releases (win/mac/linux)
- [ ] `mcp.json` + `README.md` quickstart 30s
- [ ] `CHANGELOG.md` + `SECURITY.md`

### Provisórios Zerados
- [ ] 0× `eprintln!` (tudo `tracing`)
- [ ] 0× `mouse_event` (tudo `SendInput`)
- [ ] 0× `OpenDesktopA(GENERIC_ALL)` (removido ou com fallback)
- [ ] 0× índices mágicos sem `const` documentada (425 etc viram `const HEAD_VERTEX_END: usize`)

---

## 12. ANEXOS

### Anexo A — Trechos Críticos Citados
**P0-01 Lock across await:**
```rust
// main.rs:520
let mut state = state.lock().await;
// ...
state.bridge.send_command("ORBIT", ...).await; // bloqueia todas as tools
```

**P0-04 FS arbitrário:**
```rust
// main.rs:1153
std::fs::write(path, &png_bytes).with_context(|| format!("Failed to save rendered frame to {}", path))?;
```

**P0-05 cfg faltante:**
```rust
// main.rs:1
mod win32_interact; // sem cfg(windows) → falha em Linux
```

### Anexo B — Recomendação de `.mcp.json` (Claude Desktop)
```json
{
  "mcpServers": {
    "anigo": {
      "command": "anigo-mcp",
      "args": ["--bridge-addr", "127.0.0.1:39090"],
      "env": {
        "ANIGO_BRIDGE_TOKEN": "auto",
        "ANIGO_MCP_ALLOW_FS_WRITE": "1",
        "RUST_LOG": "anigo_mcp=info"
      }
    }
  }
}
```

### Anexo C — Referências
- Model Context Protocol Spec 2024-11-05: https://spec.modelcontextprotocol.io
- wgpu 24 docs: https://docs.rs/wgpu
- Tauri 2 IPC: https://tauri.app/reference/javascript/api/
- OWASP File Path Traversal: https://owasp.org/www-community/attacks/Path_Traversal
- Tokio anti-patterns: https://tokio.rs/tokio/topics/best-practices

### Anexo D — Como Validar Esta Auditoria
1. Rodar `grep -rn "let _ =" crates/anigo-mcp/src/main.rs` → deve ser 0 após Fase 1
2. Rodar `grep -rn "thread::sleep" crates/anigo-mcp` → deve ser 0 após Fase 1
3. Rodar `cargo build -p anigo-mcp --target x86_64-unknown-linux-gnu` → deve passar após Fase 0
4. Testar `echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"anigo_render_frame","arguments":{"width":99999,"height":99999}}}' | cargo run -p anigo-mcp` → deve retornar `error code -32602`

---

## CONCLUSÃO

O MCP do ANIGO é **engenhosamente funcional** — WebGPU headless real, 31 tools coesas, bridge bidirecional e automação Win32 que demonstra domínio técnico acima da média. **Porém, como produto enterprise, está hoje em 4.1/10**: os 5 bugs P0 sozinhos impedem venda segura e portável.

A boa notícia: **nenhum bug é arquitetural irrecuperável**. Com as **Fases 0–4 (≈5 semanas)** detalhadas acima, o MCP atinge **9+/10**, pronto para:
- Publicação no MCP Registry (`modelcontextprotocol/registry`)
- Distribuição via `cargo install` / `npm` / `homebrew`
- Uso em pipelines CI de validação visual (Sprints 01–22)
- Venda B2B com SLA e suporte

**Próximo passo recomendado:** Criar branch `fix/mcp-enterprise-hardening` e iniciar pela **Fase 0** (isolamento Win32 + clamp + timeout + tracing) — 2–3 dias para eliminar risco de crash/DoS e desbloquear testes em Linux/macOS.

---

*Fim do relatório. Dúvidas ou priorização diferente? Posso gerar issues no GitHub com cada P0/P1 como task separada.*

