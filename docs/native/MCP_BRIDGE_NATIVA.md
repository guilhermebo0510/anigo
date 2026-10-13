# Bridge nativa: `anigo-mcp` <-> `anigo-studio` (T-2.13)

O app nativo expõe, **só quando pedido** (`--bridge`), um servidor local pelo qual o `anigo-mcp` (servidor MCP stdio) lê o projeto e age
sobre a MESMA `CoreSession` que a UI usa. Cada mutação entra no histórico (Ctrl+Z desfaz), aparece ao vivo na janela e fica no log da sessão.
Pesquisa e decisões: [`pesquisa/mcp_bridge_nativa.md`](pesquisa/mcp_bridge_nativa.md). Captura de referência: `img/bridge_janela.png`
(barra de status mostra `Bridge 127.0.0.1:<porta>`).

```text
 cliente MCP (Claude Desktop/Code)
        | stdio JSON-RPC
   anigo-mcp  ── ferramentas antigas (Tauri, headless) + anigo_native_*  ── bridge_client.rs
        | NDJSON (ou HTTP) em 127.0.0.1:<porta efêmera>, token por sessão  (descoberta: bridge.json)
 ┌──────▼──────── anigo-studio --bridge ───────────────────────────────┐
 │ anigo-bridge::BridgeServer (threads de conexão; auth, limites)       │
 │      │ canal limitado (HostRequest{ação, params, reply, cancelled})   │
 │ StudioApp::bridge_tick() no INÍCIO de cada quadro (thread da UI)      │
 │      └─ CoreSession.apply/undo/redo · Biblioteca · fila de render ·   │
 │         workspaces · screenshot (egui)  → request_repaint             │
 └──────────────────────────────────────────────────────────────────────┘
```

## 1. Como ligar

| | |
|---|---|
| `anigo-studio --bridge` | liga (padrão: desligada). `--bridge-port <n>` fixa a porta; `--bridge-discovery <arquivo>` escolhe onde gravar a descoberta (ambos já implicam `--bridge`) |
| `ANIGO_BRIDGE=1` | equivalente a `--bridge` |
| descoberta | `bridge.json` = `{addr, port, token, pid, started_at, version, protocol}`. Linux: `$XDG_RUNTIME_DIR/anigo/` (senão `~/.local/share/anigo/bridge/`); Windows: `%LOCALAPPDATA%\ANIGO\bridge\`; `ANIGO_BRIDGE_DISCOVERY` sobrescreve. Modo 0600 numa pasta 0700 (Unix), criação atômica, removida ao fechar o app |
| cliente | o `anigo-mcp` lê a descoberta a cada chamada (sobrevive a reinício do studio). `ANIGO_BRIDGE_ADDR` (+ `ANIGO_BRIDGE_TOKEN`) fixa o destino; sem arquivo cai no Tauri legado `127.0.0.1:39090` |

Configuração MCP (exemplo): `{"command": "anigo-mcp"}`; o studio precisa estar aberto com `--bridge`. Sem o studio, as ferramentas `anigo_native_*` respondem
"ANIGO Studio is not running…" e as antigas seguem como antes.

## 2. Protocolo

Uma porta, dois dialetos, escolhidos pelo 1º byte da conexão.

**NDJSON** (o do `bridge_client.rs`; uma linha por mensagem, conexão reutilizável):
`-> {"id":"mcp-1","action":"APPLY_COMMAND","params":{...},"token":"<hex>"}` /
`<- {"id":"mcp-1","success":true,"data":{...}}` ou `{"id":..,"success":false,"error":"...","code":-32602}`. O campo `code` é aditivo.

**HTTP/1.1 mínimo**: `POST /rpc` (mesmo envelope no corpo, `Content-Type: application/json`, `Content-Length`, token em `Authorization: Bearer` ou no corpo)
e `GET /health` (sem token; só `{ok, engine, protocol}`). `Connection: close` sempre. Status: 200 (inclusive erro de ação, `success:false`), 400/411/413/414/415/431,
401 (+`WWW-Authenticate: Bearer`), 403 (Host/Origin), 404, 405, 408, 429, 503.

Códigos de erro: `-32001` não autorizado, `-32002` a UI não respondeu no prazo (a ação **não** foi aplicada), `-32003` fila da UI cheia, `-32004` rate limit,
`-32005` grande demais, `-32006` encerrando, `-32600` requisição inválida, `-32601` ação desconhecida, `-32602` parâmetros inválidos, `-32000` a ação falhou (ex.: o núcleo recusou o comando).

## 3. Ações

Todas em CAIXA ALTA; `params` é um objeto (ou ausente).

| Ação | Parâmetros | Resposta (`data`) |
|---|---|---|
| `PING` | – | `{pong, engine, protocol, version}` |
| `GET_METRICS` | – | contadores da bridge (requisições, rejeições por auth/Host/tamanho/rate, timeouts, bytes) |
| `GET_STATUS` | – | workspace ativo, `fps`/`frame_time_ms` reais (`null` se não medido), adaptador/backend, nome do projeto, câmera, `node_count`, `history` (revisões, undo/redo, impressão digital), jobs de render, métricas |
| `GET_PROJECT` | `include_document` (padrão `true`) | `{history, project}`: o `ProjectState` canônico (JSON do `to_canonical_json`) |
| `GET_SNAPSHOT` | `include_static` (`false`), `client_static_revision` | `{snapshot, history}`: o `CoreSnapshot` (geometria só se pedida: grande) |
| `APPLY_COMMAND` | `command` (objeto `{"kind": ...}` do contrato v1), `if_revision` opcional | `{outcome, history}`. Validado por `serde` + `CoreSession::apply`; inválido = `-32602`/`-32000` e estado intacto. `{"kind":"batch","commands":[..]}` = um passo atômico |
| `UNDO` / `REDO` | – | `{outcome, history}` (age na `CoreSession`, qualquer que seja o workspace ativo) |
| `GET_LOG` | `from`, `limit` (<=500), `include_commands`, ou `sequence` | `{total, entries[], next}` ou `{entry}` com o `Command` completo. O log é a sessão: undo não apaga entradas |
| `GET_WORKSPACE` / `SET_WORKSPACE` | `workspace`: `personagem modelagem posing shading iluminacao cenario animacao render biblioteca` | `{workspace, previous?}` |
| `GET_SELECTION` | – | cena (ids, ativo), modelagem (modo, objeto ativo, contagens), posing (ossos), Biblioteca (ids) |
| `LIST_ASSETS` | `text`, `kinds[]`, `limit` (<=500), `after` (cursor) | `{total, next, assets[{id,name,kind,format,size,tris,verts,bones,has_skin,has_vrm,revision,available}]}` |
| `PLACE_ASSET` | `asset_id` ou `asset_ids[]` (UUID, <=32), `position` `[x,y,z]` | `{placed, node_ids, history}`: UM passo de undo (o mesmo do duplo clique no Content Drawer) |
| `RENDER_FRAME` | – | `{job_id, label, status:"queued", output_dir}`: enfileira um still com as configurações atuais (fila e thread de GPU do workspace Render) |
| `RENDER_STATUS` | `job_id` (padrão: o último), `inline` | `{status: queued/running/done/failed/cancelled, progress, files[{pass,path,width,height,bytes}], error, notes}`; com `inline:true` e `done`, `png_base64` (<=12 MiB) |
| `SCREENSHOT` | `target`: `viewport` (padrão, região da viewport 3D do workspace ativo) ou `window`; `max_width` (16..8192) | `{png_base64, width, height, target, workspace, pixels_per_point}` |
| `ORBIT` `ZOOM` `PAN` `SET_CAMERA` `SET_LIGHT` | os campos dos `Command`s homônimos | atalhos legados: viram `orbit_camera`/`zoom_camera`/`pan_camera`/`set_camera`/`set_light` do núcleo (mudam a câmera do **projeto** e a luz-chave, os mesmos dados dos workspaces Render/Iluminação) |
| `UI_ACTION` | `{"action":"select_tab","value":ws}` | só a troca de aba tem equivalente; o resto responde `-32602` |

Pedidos de leitura/escrita da Biblioteca e do render que dependem de carga assíncrona (Biblioteca abrindo, ativos carregando) ficam **adiados** na UI
até 12 s e então respondem com `-32002`.

### Ferramentas MCP novas (`anigo_native_*`, `crates/anigo-mcp/src/native.rs`)

`get_status`, `get_project` (`include_document`, `save_path`), `get_snapshot`, `apply_command` (`command`, `if_revision`), `undo`, `redo`, `get_log`, `list_assets`,
`place_asset`, `render_frame` (`wait`, `timeout_seconds`, `inline`, `save_path`), `screenshot` (`target`, `max_width`, `save_path`), `set_workspace`, `get_selection`.
As ferramentas antigas continuam no catálogo e iguais. Entrada validada em `validate.rs` (comando: objeto com `kind` snake_case, <=512 KiB, profundidade <=32, sem chaves de
poluição de protótipo; UUIDs; listas de workspaces/tipos; textos sem controle) **antes** de qualquer chamada de rede; arquivos só no sandbox (`fs_sandbox`: `save_path` é validado antes de gastar a
captura; `ANIGO_MCP_ALLOWED_DIRS` amplia); respostas de texto acima de 120 k caracteres são truncadas de forma explícita (use `save_path`);
métricas em `anigo_get_metrics` (`bridge_commands_sent/failed`, `native_commands_applied`, `native_images_received`, `frames_rendered`, `bytes_written_to_fs`, `fs_sandbox_rejections`).

CLI (sem GPU, sem MCP; usado pelo smoke e para depurar): `anigo-mcp bridge-info`, `anigo-mcp bridge-call <ACAO> [JSON|@arquivo] [--save-png <png>] [--timeout <s>]`.
Exemplo: `anigo-mcp bridge-call APPLY_COMMAND '{"command":{"kind":"set_proportions","head_scale":1.4}}'`.

## 4. Segurança (resumo; ameaças em `pesquisa/mcp_bridge_nativa.md` §4)

* Desligada por padrão; indicador visível na barra de status com a porta.
* Bind **só em 127.0.0.1**, porta efêmera; conexões não-loopback são descartadas.
* Token de 256 bits do CSPRNG do SO (`getrandom`), sem fallback fraco (sem entropia = a bridge não sobe); comparação em tempo constante; **autenticação antes do rate limit**
  (um cliente sem token não consome o orçamento); falha fecha a conexão e atrasa 100 ms; o cliente nunca imprime o token inteiro.
* HTTP: `Host` exato (`127.0.0.1:<porta>`, `localhost:<porta>`, `[::1]:<porta>`) contra DNS rebinding; **qualquer `Origin` => 403**; `Content-Type: application/json` obrigatório;
  sem CORS; `Transfer-Encoding`/cabeçalhos duplicados/obs-fold recusados; token na query string nunca é lido.
* Limites: 1 MiB por requisição, 32 MiB por resposta, 32 conexões, 100 req/s (rajada 200) globais, 5 s para cabeçalhos/linha (slowloris), 30 s de ociosidade, 10 s de corpo,
  fila de 64 pedidos para a UI, prazo por ação (10–20 s). Um pedido que estoura o prazo é marcado `cancelled` e a UI o **descarta** (nunca aplica tarde).
* A bridge não grava arquivos arbitrários: o render vai para a pasta de saída configurada pelo usuário; screenshots voltam em base64 e quem grava é o `anigo-mcp`, no sandbox.

## 5. Testes

| O quê | Onde | Resultado nesta máquina |
|---|---|---|
| protocolo, HTTP estrito, descoberta (0600/0700, atômica), canal da UI (timeout/cancelamento/fila cheia), `core_ops` | `crates/anigo-bridge/src` (25 unitários) | verdes |
| auth (token errado/ausente/vazio/de outro tamanho), Host/Origin, 413/431/408/415/411, linhas lentas, ociosidade, limite de conexões, rate limit, resposta grande, ciclo de vida, **24 clientes x 15 comandos + a "UI" aplicando os seus** (nenhum comando perdido; log = bridge + usuário; desfaz tudo em ordem) | `crates/anigo-bridge/tests/server.rs` (23) | verdes (3 execuções seguidas) |
| cliente do `anigo-mcp` + ferramentas `anigo_native_*` contra a bridge com uma `CoreSession` real: aplicar + undo + redo + log, erro tipado, `if_revision`, resposta > 1 MiB, descoberta que segue um reinício, 16 clientes paralelos, screenshot e sandbox | `crates/anigo-mcp/tests/native_bridge.rs` (8) + unitários de `cli`/`native`/`validate` | verdes |
| studio: recorte/redução do screenshot, atalhos legados, validação de parâmetros, flags da CLI | `crates/anigo-studio/src/app/bridge_host.rs`, `cli.rs` | verdes |
| **UI real sob Xvfb :97 + lavapipe**: seguranca HTTP por `curl`; `set_proportions` pela bridge muda a viewport (8 371 px de diferença); **Ctrl+Z (xdotool) a restaura (0 px)**; REDO/UNDO pela bridge; Biblioteca -> `PLACE_ASSET` no Cenário (+1 nó, 1 passo de undo, 108 480 px) -> UNDO remove; `RENDER_FRAME` real 480x270; captura da janela; sandbox | `scripts/native_bridge_smoke.sh` (também no job `native` do CI) | passou (~38 s) |

## 6. O que NÃO foi validado / limites

* **Windows**: nada rodou lá. Falta conferir o caminho `%LOCALAPPDATA%\ANIGO\bridge\bridge.json` e que a ACL herdada do perfil impede outros usuários de lê-lo (no Unix o modo 0600 é testado; no Windows não há `mode`, é herança de ACL **[C]**), antivírus/firewall com loopback, `rename` sobre arquivo aberto.
* **Janela minimizada/oculta/oclusa**: o loop de quadros do eframe pode pausar; `request_repaint` costuma acordá-lo, mas o pedido pode expirar com `-32002` ("a UI não respondeu"; nada é aplicado).
* **GPU real** e latências reais (no lavapipe um quadro leva ~140 ms; o smoke usa folgas largas). `fps` vem do `stable_dt` do egui.
* **Cliente MCP real** (Claude Desktop/Code) configurado com o subprocesso `anigo-mcp`: só o CLI e os testes foram usados.
* `UNDO`/`REDO` da bridge agem na `CoreSession`. No workspace Modelagem o Ctrl+Z do usuário age na `ModelingSession` (histórico separado) e o Shading tem histórico próprio: a bridge não os controla.
* `SET_SELECTION`, `jump_to_depth`, render de sequência/animação, eventos empurrados (progresso, telemetria) e `anigo_screenshot_window`/mouse/teclado nativos (que usam Win32) **não** existem na bridge nativa; os atalhos legados mudam a câmera do projeto, não a câmera de edição do viewport.
* A mutação por `PLACE_ASSET` espera até 6 s por ativo na thread da UI (comportamento herdado do Content Drawer); ativos muito grandes congelam a janela por esse tempo.
* Um pedido atendido no instante exato do estouro de prazo pode ter sido aplicado mesmo com o cliente recebendo `-32002` (janela de microssegundos; `if_revision` ajuda a reconciliar).
* Observado no smoke: `set_base_gender` aplicado pela bridge entra no histórico, mas a viewport do Personagem não mudou (0 px de diferença), enquanto `set_proportions`/`set_gender_dimorphism` mudam; a causa (preview do manequim, não da bridge) não foi investigada, por isso o smoke usa `set_proportions`.
* Sem unix socket/named pipe e sem TLS (loopback + token; um processo do MESMO usuário que leia o `bridge.json` tem o mesmo poder que o `anigo-mcp`).
