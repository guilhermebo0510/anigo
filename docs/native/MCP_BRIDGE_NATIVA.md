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
| `UI_ACTION` | `action` + `value` (+ `open`, `position`) | `select_tab` (troca de workspace) e as ações de UI nativas abaixo; devolve o estado da UI. Desconhecida = `-32602` |
| `INPUT` | `steps` (<=4096) | injeta ponteiro/roda/teclado DENTRO da janela do ANIGO (gancho `raw_input_hook`, um passo por quadro; nunca toca o cursor do SO). Passos: `move`, `down`/`up`, `click`, `scroll`, `key`, `mods`, `wait`, `drag`. Coordenadas em pontos lógicos (as de `GET_UI_STATE.views`; atenção: as capturas de tela são reduzidas, converta pela razão largura-lógica/largura-da-captura). Responde na hora; `GET_UI_STATE.input_pending` zera ao terminar (a ferramenta `anigo_native_input` espera por isso) |
| `GET_UI_STATE` | – | o que está na tela: workspace, modo da Modelagem, shading, `views` (layout, divisórias `ratio`, vista ativa, e por vista: tipo, `yaw/pitch/distance/target` e retângulo), `input_pending` (`object/edit/sculpt/hair`), modo de seleção, ids das ferramentas do shelf e a ativa, painéis do dock (`open`), objetos da Modelagem |

Ações de `UI_ACTION` (`crates/anigo-studio/src/app/bridge_ui.rs`; passam pelos mesmos caminhos dos cliques): `set_mode` (`object|edit|sculpt|hair`; vai para a Modelagem), `select_tool` (id lido de `GET_UI_STATE`), `toggle_panel` (id do painel; `open:true|false` força), `select_props_tab` (aba do Propriedades da Modelagem; as válidas do modo vêm em `GET_UI_STATE.props_tabs`), `set_shading` (`solid|wireframe|cel`), `set_view_layout` (`single|split_v|split_h|quad`), `set_view_kind` (`perspective|ortho|front|back|right|left|top|bottom`, `slot` 0..3 opcional = vista ativa), `set_active_view` (slot; a vista sob o ponteiro reassume a ativa no quadro seguinte), `set_camera_view` (Cenário; `true|false`), `save_session` (grava a sessão; exige `--session-file`). `set_shading`, `set_view_layout`, `set_view_kind` e `set_active_view` valem no workspace atual quando ele tem viewport 3D editável (Modelagem, Cenário, Posing/Animação, Iluminação — cada um com câmera e multi-vista próprias); nos demais (Render, Shading, Biblioteca…) valem para a Modelagem, `add_primitive` (`plane|grid|cube|quad_sphere|uv_sphere|cylinder|cone|torus|capsule|circle`, `position` opcional), `select_object` (índice), `set_select_mode` (`vertex|edge|face`). Ferramentas MCP: `anigo_native_get_ui_state` e `anigo_native_ui_action`. Estas ações mexem no estado da Modelagem (`ModelingSession`, histórico próprio), não no `ProjectState` do núcleo.

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

### Modelagem e escultura (`MODEL`, ferramentas `anigo_native_model_*`, `mesh_*`, `sculpt`)

`MODEL {op, …}` roda na thread da UI pelas MESMAS funções dos botões (`ModelingSession::edit_op/edit_mesh`, `SculptState::run_script`). Muda o estado da **Modelagem**
(`ModelingSession`, histórico PRÓPRIO — `anigo_native_model_history`, não o `anigo_native_undo` do núcleo); um operador que falha não altera nada. Vai para a Modelagem
sozinho se outro workspace estiver ativo, e as ferramentas entram no modo que precisam (Objeto/Edição/Escultura) quando há objeto ativo.

| Ferramenta MCP | `op` | O que faz |
|---|---|---|
| `anigo_native_model_info` | `info` | modo, objetos, transform/bounds (local e mundo) do ativo, estatísticas, seleção (contagens + ids), histórico, pincel/simetria; `include_geometry` devolve posições e faces (até `limit`<=2000) para conferir numericamente |
| `anigo_native_model_object` | `object` | `add` (10 primitivas), `select`, `delete`, `duplicate`, `rename`, `set_transform` (absoluto: translation, rotation_deg XYZ, scale) |
| `anigo_native_mesh_select` | `select` | `mode` vertex/edge/face; `what`: all/none/invert/more/less/linked, `verts`/`faces`/`edges` por índice, ou `box` em **espaço de mundo**; `extend` |
| `anigo_native_mesh_op` | `edit_op` | os 35 operadores do painel (`extrude`, `inset`, `bevel`, `loop_cut`, `bisect`, `delete_*`, `merge_*`, `dissolve_*`, `bridge`, `triangulate`, `subdivide`, `smooth`…) com `params` opcionais (`distance`, `thickness`, `width`, `segments`, `cuts`…); devolve estatísticas antes/depois e a nova seleção |
| `anigo_native_mesh_transform` | `transform` | `move` (mundo), `rotate` (graus XYZ), `scale` nos vértices selecionados (pivô = centro da seleção ou `origin`) ou, no modo Objeto, no objeto (relativo). Em Edição, a **edição proporcional** vale (a do painel, `anigo_native_model_options`, ou `proportional` só nesta chamada: raio, falloff, só conectados) |
| `anigo_native_sculpt` | `sculpt` | `set_brush`, `stroke` (`path` normalizado 0..1 da vista ativa, `paths` ou `world_path` projetado pela câmera da vista; cada traço = 1 passo de undo; `dabs=0` + aviso se errou o objeto), `mask` (invert/clear), `smooth_all`, `symmetrize`, `voxel_remesh`, `dyntopo`; `stroke` aceita `frame_view` (enquadra o objeto na vista ativa antes: os pincéis só tocam o que aparece e o traço só COMEÇA se o primeiro ponto cai sobre o objeto); `voxel_remesh` aceita `shade_smooth` (o remalhar de origem chapada sai chapado: faixas numa esfera) e `voxel_smooth`; aceita `brush`, `radius_px`, `strength`, `spacing_pct`, `invert`, `mirror`, `radial` |
| `anigo_native_model_history` | `history` | `undo`/`redo` (1..100 passos) do histórico de Modelagem |

Validação em duas camadas: o `anigo-mcp` confere chaves permitidas e tamanho (<=1 MB) ANTES da rede; o app confere tipos, intervalos, índices e listas (<=100 000 índices, 32 traços de 512 pontos).
Entrada de mouse/teclado para o que não tem comando: `anigo_native_input`.

### Cabelo, pesos, malha, materiais e animação (`HAIR`, `WEIGHTS`, `MESHDATA`, `SHADING`, `ANIM`)

Mesmo desenho do `MODEL`: cada ação roda na thread da UI pelas funções dos painéis, valida chaves (campo desconhecido/typo é erro, nunca ignorado) e é transacional.
Os erros listam o que é válido (ex.: tipos de modificador, canais NPR, campos do material).

| Ferramenta MCP | Bridge | O que faz |
|---|---|---|
| `anigo_native_hair_info` | `HAIR info` | alvo, grupos, mechas (`include_strands`), pente, bounce |
| `anigo_native_hair_setup` | `HAIR target/group/bounce` | cabeça-alvo; grupos (`new`, `material` por patch, `symmetry`, `flip`…); bounce (`auto`, `set` com patch validado) |
| `anigo_native_hair_strand` | `HAIR strand` | `select`, `set_params` (por mecha, por grupo ou template), `simplify`, `duplicate`, `delete`, `move_to_group`, `insert_point`, `remove_point`, `move_point` |
| `anigo_native_hair_stroke` | `HAIR draw/comb` | desenha mecha (`world_path`, `path` na tela ou `local_path`) e penteia (`radius_px`, `strength`, `all_groups`) |
| `anigo_native_weights_info` | `WEIGHTS info` | estatísticas de pesos de pele e canais NPR; `vertices` devolve as influências |
| `anigo_native_npr_paint` | `WEIGHTS npr` | canais NPR (`ao`, `outline_width`, `shadow_shift`, `spec_rim`…): `paint`, `fill`, `gradient`, `invert`, `smooth`, `bake_ao` |
| `anigo_native_skin_weights` | `WEIGHTS skin` | `assign`, `paint`, `normalize`, `smooth`, `limit` (1..4), `auto_envelope`, `mirror`, `transfer` |
| `anigo_native_modifiers` | `MESHDATA modifiers` | `add`/`set`/`remove`/`move`/`apply` (parâmetros validados; devolve a malha avaliada) |
| `anigo_native_normals` | `MESHDATA normals` | `report`, `auto_smooth`, `bake_custom`, `clear_custom`, `smooth`, `reset`, transferência |
| `anigo_native_uv` | `MESHDATA uv` | `report`, `unwrap`, `smart_project`, `project_*`, `pack`, `flip_u/v`… |
| `anigo_native_material` | `SHADING` | fonte (`personagem`/`cenario`/`modelagem`), alvo, `set` por patch JSON parcial, `preset`, `reset`, `copy`/`paste`, `history` |
| `anigo_native_animation` | `ANIM` | clips (`new`, `select`, `rename`, `range`, `playback`), transporte (`goto`, `step`, `prev_key`, `play`/`pause`, `speed`), chaves (`insert`, `delete`, `autokey`) |

Pesos e hair escrevem no documento da **Modelagem** (histórico `anigo_native_model_history`); material de Personagem/Cenário e animação entram no histórico do núcleo (`anigo_native_undo`).
`HAIR export` grava na Biblioteca real do usuário: não usar em testes.

### Fila do Render e Biblioteca (`RENDER`, `LIBRARY`)

Mesmo desenho: as MESMAS funções dos botões (`render_enqueue/cancel/resume`, backend `anigo-assets`), validação antes de gravar (todo-ou-nada), campo desconhecido é erro. O sandbox de arquivos
mora no `anigo-mcp` (pastas e caminhos precisam ser ABSOLUTOS e estar em Documents/ANIGO, ./baselines, ./tmp, cwd ou `ANIGO_MCP_ALLOWED_DIRS`); o app recebe só o caminho já validado.

| Ferramenta MCP | Bridge | O que faz |
|---|---|---|
| `anigo_native_render_info` | `RENDER info` | preferências, estado da pasta, PRÓXIMOS nomes de arquivo, saída/AOVs/passes do projeto, câmera de render, fila (progresso, etapa, arquivos, erros), histórico de imagens, ffmpeg |
| `anigo_native_render_prefs` | `RENDER prefs` | `patch` de pasta (`dir`), template (`{project}_{n:04}`, sem separadores), contador, sobrescrever, travar aspecto, sensor, guias, fila aberta |
| `anigo_native_render_camera` | `RENDER camera` | câmera de cena vinculada (`source`/`reapply`), `from_view` (câmera do Cenário), `frame` (cena ou seleção, no aspecto da SAÍDA) |
| `anigo_native_render_job` | `RENDER job` | `cancel`, `resume` (cancelado/falho), `remove`, `clear`; enfileirar segue em `anigo_native_render_frame` |
| `anigo_native_render_history` | `RENDER history` | `list`, `path`, `to_library` (envia a imagem à Biblioteca REAL do usuário) |
| `anigo_native_library_browse` | `LIBRARY info/query/filter/select/detail/edit_in_modeling` | contagens, locais, coleções, árvore de tags, jobs; consulta completa (`Query` do backend); filtros da UI; seleção; detalhe; abrir na Modelagem |
| `anigo_native_library_edit` | `LIBRARY meta/collections/tags` | favorito, nota, nome, descrição, autor, licença, tipo e tags; coleções manuais e inteligentes; renomear/cor/apagar/limpar tags |
| `anigo_native_library_trash` | `LIBRARY trash` | `trash`/`restore` (reversíveis), `purge`/`empty` (DEFINITIVOS: exigem `confirm=true`) |
| `anigo_native_library_import` | `LIBRARY import` | arquivos/pastas com opções (modo, tags, duplicados, coleção, nova revisão); `wait=true` só para arquivos pequenos (<=64 MB) |
| `anigo_native_library_roots` | `LIBRARY roots` | listar/atualizar/adicionar/escanear/remover locais (remover NUNCA apaga arquivos) |
| `anigo_native_library_maintenance` | `LIBRARY maintenance` | duplicados, ausentes, verificar (`fix` só corrige o índice), reindexar, cancelar imports |

Teste ponta a ponta (com `ANIGO_LIBRARY_DIR`, `ANIGO_ASSETS_DIR` e `ANIGO_RENDER_DIR` apontando para uma pasta temporária, para NÃO tocar a Biblioteca do usuário): app real + render de verdade na GPU.
O `to_library` e as importações gravam na Biblioteca configurada; nos testes, isole-a com essas variáveis.

### Posing e Iluminação (`POSE`, `LIGHT`)

Mesmo desenho dos anteriores: as MESMAS funções da UI (`posing::edit`, `pose_commit_to`, `SetLight`/`SetMaterialParams`, `rig_command`), validação antes de mexer, campo desconhecido é erro.
Toda edição é UM passo de desfazer do núcleo (`anigo_native_undo`); luz + material no mesmo `op=set` viram um `Batch` (um passo). As respostas trazem `changed` e `history_steps_added`.

| Ferramenta MCP | Bridge | O que faz |
|---|---|---|
| `anigo_native_pose_info` | `POSE info` | 24 ossos (Euler, quaternion, junta em espaço do personagem e do mundo, limite + valor atual), translações, dedos, equilíbrio (CoM, base de apoio, contato mais baixo), seleção, TODAS as opções, área de transferência, poses da Biblioteca, nomes dos presets |
| `anigo_native_pose_options` | `POSE options` | perfil de limites (human/anime/free), simetria, snap, IK por membro, foot lock, auto-ground, olhar, mistura, visibilidade; todo-ou-nada |
| `anigo_native_pose_select` | `POSE select` | ossos por índice ou nome (set/add/remove/toggle, tudo, nada, ativo) |
| `anigo_native_pose_bone` | `POSE bone` | `set_euler`, `set_quat`, `rotate_world` (eixo em MUNDO), `reset`; respeitam limites e simetria |
| `anigo_native_pose_edit` | `POSE translate/ik/gaze/ground/mirror/copy/paste/reset/preset/hands` | quadril/raiz (foot lock), IK de braço/perna (alvo em mundo ou no personagem, pole), olhar, apoiar no chão, espelhar, copiar/colar (espelhado), zerar, presets de corpo e de mão, dedos |
| `anigo_native_pose_library` | `POSE library` | listar, salvar (grava na Biblioteca REAL; mesmo nome = nova revisão) e aplicar poses (mistura, espelhada, só selecionados) |
| `anigo_native_light_info` | `LIGHT info` | sol (direção, az/el, cor, K, intensidade), tinta/saturação da sombra, ambiente, sombra projetada, terminador e rim, luz presa, rigs |
| `anigo_native_light_set` | `LIGHT set` | az/el ou vetor, cor ou kelvin, intensidade, sombra (tinta, saturação, projetada), ambiente, `material` (limiar, penumbra, rim) |
| `anigo_native_light_control` | `LIGHT rig/reset_sun/follow/view` | rigs prontos, resetar o sol, luz presa à câmera (+ gravar a direção), gizmo e enquadrar |

Teste ponta a ponta com o app real: `scratchpad/t_pose_light.py` (Biblioteca isolada por `ANIGO_LIBRARY_DIR`, pois `pose_library save` grava nela).

### Animação (fina), Shading, Cenário, Personagem e App (`ANIM`, `SHADING`, `SCENE`, `CHAR`, `APP`)

Cobertura do que cada workspace faz na UI além de "aplicar um `Command`". Mesmas funções dos botões, validação antes de mexer (campo desconhecido e valor fora da faixa são erro, nunca viram o padrão), uma edição = UM passo de desfazer.
Caminhos de arquivo passam pelo sandbox do `anigo-mcp` (absolutos, dentro de Documents/ANIGO, ./baselines, ./tmp, cwd ou `ANIGO_MCP_ALLOWED_DIRS`).

| Ferramenta MCP | Bridge | O que faz |
|---|---|---|
| `anigo_native_model_options` | `MODEL options` | edição proporcional (raio, falloff, só conectados), raio X, snap |
| `anigo_native_anim_keys` | `ANIM keys` | seleção de chaves (itens, tudo, faixa), interpolação (step/linear/ease/Bézier com alças), mover, duplicar, espelhar, copiar/colar, apagar, valor da chave (Euler/quat/vetor/dedos) |
| `anigo_native_anim_more` | `ANIM marker/scene_keys/onion/physics/file/library/options/sequence` | marcadores; chaves de sol/câmera/nó; onion skin; física secundária (+bake); importar/exportar BVH/GLB/VRMA/clip; Biblioteca de clips; opções; **sequência de render** do clip pela fila do Render |
| `anigo_native_material_view` | `SHADING view/compare` | preview (bola/objeto, luz, fundo, giro, foco de grupo, paleta), comparador A/B |
| `anigo_native_material_tools` | `SHADING param/palette/ramp` | reset de parâmetro/grupo, "recursos anime", sombra derivada, giro de matiz, harmonias, paleta do projeto, editor de rampa 2D |
| `anigo_native_material_library` | `SHADING presets/texture` | presets da Biblioteca (salvar/aplicar), slots de textura (incl. mapa SDF facial vivo) |
| `anigo_native_scene_info` / `scene_select` / `scene_edit` / `scene_project` | `SCENE` | nós e seleção; gizmo, snap, vista/guias/ortogonais, enquadrar; visibilidade e trava; Outliner (renomear, apagar, duplicar, agrupar, reparentar, pastas, copiar/colar); colocar atores; transformar nós; câmeras de cena; salto no histórico; projeto `.anigo` (novo/abrir **exigem `discard_current=true`**) |
| `anigo_native_char_info` / `char_sliders` / `char_edit` | `CHAR` | os 157 sliders (lista/define/reseta), proporções, gênero/dimorfismo, somatótipo, presets, resetar tudo, câmera do preview |
| `anigo_native_app_info` / `app_history` / `app_keymap` / `app_layout` / `app_control` | `APP` | idioma; desfazer/refazer do workspace ATIVO (núcleo, Modelagem ou Shading); atalhos (listar/trocar/restaurar/importar/exportar); layouts do dock (presets, importar/exportar; rodam no próximo quadro); enquadrar; exportar o objeto ativo para a Biblioteca |

Smoke de entrada do Shading **sem xdotool** (roda em qualquer SO): `python scripts/bridge_shading_smoke.py` — arrasta o slider do Rim com mouse injetado, confere UM passo de undo, Ctrl+Z/Ctrl+Y e a viewport idêntica (0 px) antes/depois.

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

* **Windows** (validado nesta máquina): o `bridge.json` fica em `%LOCALAPPDATA%\ANIGOridgeridge.json` e agora a ACL é restrita ao usuário atual (`icacls /inheritance:r`; antes herdava SYSTEM, Administradores e outras contas do perfil). Melhor esforço: se o `icacls` falhar vale a ACL por usuário do perfil. Ainda sem teste: antivírus/firewall com loopback e `rename` sobre arquivo aberto.
* **Janela minimizada/oculta** (testado): o eframe para de desenhar quadros e a UI não drena a fila. Antes o cliente esperava 10–20 s; agora, se nenhum quadro é desenhado em ~2,5 s, a ação falha com `-32002` e a causa ("a janela do ANIGO está minimizada ou oculta (restaure-a)"), sem aplicar nada. `PING` (e `anigo_native_ping`) é respondido FORA da UI e traz `ui_drawing`/`ui_idle_ms`, então serve de sonda mesmo minimizado. Uma UI viva mas ocupada continua com o prazo inteiro da ação.
* **GPU real** e latências reais (no lavapipe um quadro leva ~140 ms; o smoke usa folgas largas). `fps` vem do `stable_dt` do egui.
* **Cliente MCP real** (Claude Desktop/Code) configurado com o subprocesso `anigo-mcp`: só o CLI e os testes foram usados.
* `UNDO`/`REDO` da bridge agem na `CoreSession`. No workspace Modelagem o Ctrl+Z do usuário age na `ModelingSession` (histórico separado) e o Shading tem histórico próprio: a bridge não os controla.
* Eventos empurrados (progresso, telemetria) e `anigo_screenshot_window`/mouse/teclado nativos (que usam Win32) **não** existem na bridge nativa (o progresso se lê por polling: `render_info`, `library_browse info`); os atalhos legados mudam a câmera do projeto, não a câmera de edição do viewport. A seleção de nós do Cenário (`scene_select`), o salto no histórico (`scene_edit history`) e a sequência de render (`anim_more sequence`) agora existem.
* A mutação por `PLACE_ASSET` espera até 6 s por ativo na thread da UI (comportamento herdado do Content Drawer); ativos muito grandes congelam a janela por esse tempo.
* Um pedido atendido no instante exato do estouro de prazo pode ter sido aplicado mesmo com o cliente recebendo `-32002` (janela de microssegundos; `if_revision` ajuda a reconciliar).
* `set_base_gender` pela bridge: a observação antiga de "viewport não muda" NÃO se reproduz hoje (`anigo_native_char_edit gender` muda ~50 mil pixels da viewport do Personagem; conferido por diferença de captura).
* Sem unix socket/named pipe e sem TLS (loopback + token; um processo do MESMO usuário que leia o `bridge.json` tem o mesmo poder que o `anigo-mcp`).
