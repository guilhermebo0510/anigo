# Pesquisa: servidor local ("bridge") para o `anigo-mcp` falar com o app nativo (T-2.13, out/2026)

Legenda: [V] = lido na fonte nesta sessão; [S] = visto só em snippet/resumo de busca; [C] = conhecimento prévio/inferência, NÃO verificado agora. Estado em 2026-10-12.

## 1. O que existe hoje (lido no repositório) [V]

* `crates/anigo-mcp` é um servidor MCP **stdio** (JSON-RPC por linha, `protocolVersion 2024-11-05`) que, para agir no app, fala com `src-tauri/src/bridge.rs` por **TCP em `127.0.0.1:39090`**, protocolo **NDJSON**: requisição `{"id","action","params","token"}` -> resposta `{"id","success","data"|"error"}`; ações em CAIXA ALTA (`PING`, `GET_STATUS`, `SCREENSHOT`, `ORBIT`, `SET_LIGHT`, `UI_ACTION`...).
* Segurança atual do bridge Tauri: token (env `ANIGO_BRIDGE_TOKEN` ou aleatório lido de `/dev/urandom`, com **fallback fraco** baseado em pid+tempo quando não há `/dev/urandom`), arquivo de lock em `%TEMP%/anigo-bridge-<pid>.json` **sem permissões restritas**, quadro máximo 1 MiB, rate limit 30 req/s por IP (= global, tudo é loopback), timeout de leitura 5 s, fecha a conexão na 1ª falha de auth. Não há checagem de `Host`/`Origin` (não era HTTP). Porta FIXA 39090.
* Cliente (`crates/anigo-mcp/src/bridge_client.rs`): uma conexão por comando, timeouts 800 ms/2 s, id monotônico, resposta limitada a 1 MiB (**pequeno demais para screenshot em base64**), token vem de `ANIGO_BRIDGE_TOKEN` (não lê arquivo de descoberta).
* `anigo-session::CoreSession` já oferece `apply`/`apply_json`/`undo`/`redo`/`snapshot`/`history_report`/`history().export_log()`; o app nativo guarda UMA `CoreSession` (`StudioApp::core`) compartilhada por Personagem/Cenário/Iluminação/Render/Posing. Toda mutação do usuário passa por ela => a bridge reaproveita o MESMO caminho (undo/redo, log, `dynamic_revision`).
* O app nativo não tem runtime async: threads + `crossbeam-channel` (ex.: `render_out::RenderService`, `LibraryWorkspace`).

## 2. Protocolo MCP atual e o que ele exige de um servidor local

* **Spec** [V]: <https://modelcontextprotocol.io/specification/2025-06-18/basic/transports> (lida inteira) define dois transportes padrão, **stdio** e **Streamable HTTP**; "Clients SHOULD support stdio whenever possible"; transportes customizados são permitidos desde que preservem JSON-RPC e o ciclo de vida. A página "latest" (2026-07-28) existe, mas o scrape devolveu só a visão geral, então o texto normativo vigente da versão mais nova é [S]; nada indica remoção do stdio.
* **Aviso de segurança do Streamable HTTP** [V] (2025-06-18): "Servers MUST validate the `Origin` header on all incoming connections to prevent DNS rebinding attacks"; "When running locally, servers SHOULD bind only to localhost (127.0.0.1) rather than 0.0.0.0"; "Servers SHOULD implement proper authentication for all connections".
* **Autorização** [V]: opcional; implementações STDIO "SHOULD NOT" seguir o fluxo OAuth e "retrieve credentials from the environment"; transportes alternativos "MUST follow established security best practices for their protocol". Token em `Authorization: Bearer`, nunca na query string; 401 para token inválido, 403 para permissão.
* **Security best practices** (versão latest) [S, via resumo do scrape]: servidores destinados a rodar localmente "SHOULD use the stdio transport to limit access exclusively to the MCP client, or restrict HTTP transport access by requiring an authorization token and using unix domain sockets or restricted IPC mechanisms"; posse de um "state handle" não é autenticação.
* **DNS rebinding** [S] (GitHub Blog "Localhost dangers: CORS and DNS rebinding"; SWM "A website can read your local MCP server"; blog jsmon): o navegador acredita falar com `evil.example`, mas depois do TTL o nome resolve para 127.0.0.1; a política de mesma origem deixa passar. Mitigações unânimes: (1) rejeitar `Host` que não seja nome de loopback (`127.0.0.1`, `localhost`, `[::1]`) — um pedido rebinded sempre carrega o `Host` do atacante; (2) validar `Origin` quando presente (comparação EXATA, nunca `HasPrefix`/`endsWith`, que o artigo GitHub mostra ser explorável); (3) exigir token local — um processo malicioso já na máquina forja `Host: 127.0.0.1`, só o token o barra; (4) continuar fazendo bind em 127.0.0.1.
* **Arquivo de descoberta com permissão restrita** [V parcial]: `OpenOptions::create_new(true)` cria atomicamente e falha se existir (inclusive symlink pendurado); em Unix `OpenOptionsExt::mode(0o600)` define as permissões na criação (sem janela de corrida do `write` + `chmod`) [V, docs.rs/std]. No Windows não há `mode`; o arquivo herda a ACL da pasta -> usar `%LOCALAPPDATA%` (por usuário), não `Documents` (pode sincronizar com OneDrive) nem `%TEMP%` compartilhado por convenção [C].

## 3. Decisões

| # | Decisão | Motivo |
|---|---|---|
| D1 | `anigo-mcp` continua sendo O servidor MCP (stdio). A bridge nativa é um **canal de controle privado** (transporte customizado local), NÃO um endpoint MCP. | Spec recomenda stdio para servidores locais; evita sessões/SSE/OAuth sem benefício; os clientes de IA (Claude Desktop/Code) já lançam o `anigo-mcp` como subprocesso. |
| D2 | **Mesmo envelope NDJSON** do cliente existente (`id/action/params/token`), mesmas ações antigas onde fizer sentido (`PING`, `GET_STATUS`, `ORBIT`, `ZOOM`, `PAN`, `SET_CAMERA`, `SET_LIGHT`, `SCREENSHOT`), mais ações novas em CAIXA ALTA. Resposta idêntica (`success/data/error`) + campo ADITIVO `code`. | Compatibilidade total com `bridge_client.rs`; o cliente só precisa descobrir porta/token e aceitar respostas maiores. |
| D3 | A MESMA porta também aceita **HTTP/1.1 mínimo** (`POST /rpc` com o mesmo envelope no corpo; `GET /health`): detecta pelo 1º byte (`{` = NDJSON, letra = HTTP). Em HTTP: `Host` exato (`127.0.0.1:<porta>`, `localhost:<porta>`, `[::1]:<porta>`), **qualquer `Origin` é recusado** (clientes de automação não enviam `Origin`; navegador sempre envia em POST cross-origin), `Content-Type: application/json` obrigatório (POST "não simples" forçaria preflight; `OPTIONS` -> 405 sem CORS), `Content-Length` obrigatório e limitado, `Transfer-Encoding`/`Content-Length` duplicado -> 400, `Connection: close`, token por `Authorization: Bearer` (ou no corpo). | Atende "validação de Host/Origin" e permite `curl`/testes sem cliente especial. |
| D4 | **Sem dependência HTTP/async**: parser próprio de ~200 linhas, estrito, em `std::net` + threads. | `hyper`/`axum` exigiriam tokio no app; `tiny_http` não valida Host/Origin por nós, e precisamos de 1 endpoint. Superfície minúscula e testada (auth, Host, Origin, corpo grande, cabeçalhos malformados, slowloris). Sem proxy no caminho => sem risco de request smuggling (sem keep-alive em HTTP; `Transfer-Encoding` recusado). |
| D5 | Bind **só `127.0.0.1`**, porta efêmera (0) por padrão (`--bridge-port` opcional). | Spec [V]; porta fixa 39090 é previsível e disputável. |
| D6 | Token de **256 bits** via `getrandom` (CSPRNG do SO: `getrandom(2)` no Linux, `BCryptGenRandom` no Windows); **sem fallback fraco** — se o SO não der entropia, a bridge não sobe. Comparação em tempo constante. Falha de auth fecha a conexão e atrasa 100 ms. | Corrige o fallback fraco do bridge Tauri. |
| D7 | Arquivo de descoberta `bridge.json` (`{addr, port, token, pid, started_at, version, protocol}`) em: Linux `$XDG_RUNTIME_DIR/anigo/` (tmpfs, 0700 por usuário) senão `~/.local/share/anigo/bridge/`; Windows `%LOCALAPPDATA%\ANIGO\bridge\`; `ANIGO_BRIDGE_DISCOVERY` e `--bridge-discovery` sobrescrevem. Criação atômica (arquivo temporário `create_new` + modo 0600 + `rename`), pasta 0700 (Unix); removido ao desligar se o `pid` ainda for o nosso. | Permissão restrita sem janela de corrida; fora de pastas sincronizadas. |
| D8 | Limites: corpo/linha de requisição 1 MiB; resposta 32 MiB (screenshots/snapshots); no máximo 32 conexões simultâneas; 5 s para cabeçalhos HTTP; 30 s de ociosidade em NDJSON; rate limit global por token-bucket (padrão 100 req/s, rajada 200); timeout por ação na ponte UI (10 s; screenshot 20 s). | Mesma ordem de grandeza do bridge Tauri, ajustada ao novo payload. |
| D9 | **Desligada por padrão**; liga com `--bridge` (CLI) ou `ANIGO_BRIDGE=1`. Aparece na barra de status quando ativa (indicador honesto, com porta). | Princípio de menor privilégio; "nunca age de surpresa". |
| D10 | Mutações **sempre** na thread da UI: a thread de conexão envia `HostRequest{action,params,reply,cancelled}` por canal `crossbeam` limitado; a UI drena no início do quadro, aplica (`CoreSession::apply`/`undo`/`redo` pelo mesmo roteamento do usuário), responde e chama `request_repaint`. Se a UI não responder no prazo, o pedido é marcado `cancelled` e a UI **descarta** (nunca aplica tarde). | Nenhum estado compartilhado novo, sem locks na sessão, mutação ao vivo visível e desfazível com Ctrl+Z. |
| D11 | Segurança do conteúdo: `Command` validado por `serde` (`kind` com tag) + `CoreSession::apply` (validação do núcleo); nada de caminhos de arquivo arbitrários vindos da bridge (render grava na pasta de saída configurada pelo usuário; screenshots voltam em base64 e quem grava é o `anigo-mcp` no sandbox `fs_sandbox`). | A bridge não vira primitiva de escrita de arquivos. |

## 4. Modelo de ameaças (resumo)

| Ameaça | Mitigação |
|---|---|
| Página web chamando `fetch("http://127.0.0.1:porta")` | `Origin` presente => 403; sem token => 401; porta efêmera. |
| DNS rebinding | `Host` exato com a porta; token. |
| Ataque cross-protocol (formulário HTML falando NDJSON) | 1º byte decide o modo; NDJSON exige JSON válido + token por requisição; primeira linha HTTP cai no parser HTTP e esbarra em `Origin`. |
| Outro usuário da máquina lendo o token | arquivo 0600 em pasta 0700 (Unix); `%LOCALAPPDATA%` (Windows, [C] herda ACL do perfil). |
| Processo do MESMO usuário | Fora de escopo (mesmo limite do próprio `anigo-mcp`); o token só impede que seja trivial. |
| Força bruta do token | 256 bits; fecha conexão + 100 ms na falha; rate limit global. |
| DoS (corpos grandes, muitas conexões, slowloris) | limites D8; cabeçalhos com teto de 16 KiB/64 campos; timeouts. |
| Mutação tardia/fantasma | `cancelled` no pedido (D10). |

## 5. Proposta de escopo

* **MVP (feito nesta tarefa)**: servidor (NDJSON + HTTP mínimo), segurança D1–D11, ações: `PING`, `GET_METRICS`, `GET_PROJECT`, `GET_SNAPSHOT`, `APPLY_COMMAND`, `UNDO`, `REDO`, `GET_LOG`, `GET_WORKSPACE`/`SET_WORKSPACE`, `GET_SELECTION`, `LIST_ASSETS`, `PLACE_ASSET`, `RENDER_FRAME`/`RENDER_STATUS`, `SCREENSHOT`, `GET_STATUS`, e as ações legadas de câmera/luz como `Command`s; ferramentas novas no `anigo-mcp` (mantendo as antigas) e subcomando `bridge-call` para CLI/CI.
* **Completo (parcial)**: testes de protocolo/auth/concorrência/integração, smoke Xvfb, docs. Fica de fora: seleção por escrita (`SET_SELECTION`), `jump_to_depth`, render de sequência, eventos empurrados (notificações).
* **Avançado (não feito)**: notificações servidor->cliente (SSE ou conexão persistente) para telemetria/ progresso de render; Unix domain socket / named pipe (elimina a superfície TCP; spec [S] recomenda); ACL explícita no Windows; endpoint MCP Streamable HTTP nativo no próprio studio (dispensaria o subprocesso); multi-instância com descoberta por pid; sandbox de processo para o `anigo-mcp`.

## 6. Divisão nuvem x local

* **Nuvem/CI (validado aqui)**: protocolo, auth, limites, concorrência, integração com `CoreSession` real, smoke sob Xvfb + lavapipe (screenshot antes/depois, Ctrl+Z).
* **Local (humano/GPU real/Windows)**: discovery e permissões no Windows (`%LOCALAPPDATA%`/ACL), comportamento com janela minimizada/oclusa (o loop de quadros do eframe pode pausar; pedidos expiram com erro claro), latência com GPU real, clientes MCP reais (Claude Desktop) configurados com o subprocesso, firewall/antivírus bloqueando loopback.
