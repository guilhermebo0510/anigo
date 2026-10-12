#!/bin/bash
# Smoke da bridge nativa (T-2.13): Xvfb PROPRIO (:97), o studio com --bridge e o CLI do anigo-mcp (`bridge-call`)
# agindo como cliente de automacao. Confere COMPORTAMENTO REAL por diferenca de screenshot da viewport:
#   1. seguranca: arquivo de descoberta 0600, token errado/ausente -> 401, Origin -> 403, Host de rebinding -> 403;
#   2. um Command (set_proportions) aplicado pela bridge MUDA a viewport (UI refletiu ao vivo) e entra no historico;
#   3. Ctrl+Z (xdotool, so dentro do Xvfb) desfaz o comando do MCP: viewport volta ao estado inicial; REDO/UNDO pela bridge;
#   4. Biblioteca: LIST_ASSETS -> PLACE_ASSET no Cenario (1 passo de undo) -> UNDO remove o no;
#   5. RENDER_FRAME real (fila do workspace Render) -> PNG com o tamanho pedido;
#   6. SCREENSHOT do MCP salvo no sandbox de arquivos.
# Nunca toca o display do usuario: keydown/key/keyup explicitos e windowfocus apenas no Xvfb.
# Uso: scripts/native_bridge_smoke.sh [studio] [anigo-mcp] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
MCP=${2:-./target/debug/anigo-mcp}
OUT=${3:-/tmp/anigo_bridge_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:97}
rm -rf "$OUT"
mkdir -p "$OUT/home"
LOG="$OUT/studio.log"
: >"$LOG"

Xvfb "$DISP" -screen 0 1600x1000x24 >/dev/null 2>&1 &
XPID=$!
sleep 1
if ! kill -0 "$XPID" 2>/dev/null; then
  echo "FALHA: nao consegui iniciar o Xvfb em $DISP"
  exit 2
fi
export DISPLAY="$DISP"
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
# tudo isolado: HOME (biblioteca/renders), dados XDG, descoberta e sandbox do cliente
export HOME="$OUT/home"
export XDG_DATA_HOME="$OUT/xdg"
export ANIGO_BRIDGE_DISCOVERY="$OUT/bridge.json"
export ANIGO_MCP_ALLOWED_DIRS="$OUT"
unset ANIGO_BRIDGE_ADDR ANIGO_BRIDGE_TOKEN

"$BIN" --no-session --workspace personagem --bridge --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
}
trap cleanup EXIT

fail() { echo "FALHA: $1"; tail -5 "$LOG"; import -window root "$OUT/falha.png" 2>/dev/null; exit 1; }
call() { "$MCP" bridge-call "$@"; }
jqr() { jq -r "$1" 2>/dev/null; }
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
ctrl_key() { xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }
shot() { call SCREENSHOT "{\"target\":\"viewport\"}" --save-png "$OUT/$1.png" >/dev/null || fail "screenshot $1"; }
status_field() { call GET_STATUS | jq -r "$1"; }
wait_until() { # descricao, comando-shell, timeout em s
  local desc="$1" cond="$2" tmo="${3:-30}" i=0
  while [ $i -lt $((tmo * 4)) ]; do
    if eval "$cond"; then return 0; fi
    kill -0 "$APID" 2>/dev/null || fail "o app terminou antes de: $desc"
    sleep 0.25
    i=$((i + 1))
  done
  fail "tempo esgotado: $desc"
}

# ------------------------------------------------------------------ 0. subida e descoberta
wait_until "arquivo de descoberta" '[ -f "$OUT/bridge.json" ]' 60
PORT=$(jq -r .port "$OUT/bridge.json")
TOKEN=$(jq -r .token "$OUT/bridge.json")
MODE=$(stat -c %a "$OUT/bridge.json")
echo "bridge em 127.0.0.1:$PORT, descoberta modo $MODE"
[ "$MODE" = "600" ] || fail "arquivo de descoberta deveria ser 0600 (e $MODE)"
[ "${#TOKEN}" = "64" ] || fail "token deveria ter 64 hex"
wait_until "bridge responde (PING)" '[ "$(call PING | jq -r .pong)" = "true" ]' 60
wait_until "primeiro quadro do app" '[ "$(status_field .frame_count)" -ge 5 ] 2>/dev/null' 60
echo "adaptador: $(status_field .adapter_name) / $(status_field .backend)"

# ------------------------------------------------------------------ 1. seguranca (HTTP e NDJSON, de fora do cliente do MCP)
H="Host: 127.0.0.1:$PORT"
BODY='{"id":"s","action":"PING"}'
code() { curl -s -o "$OUT/http_body.txt" -w '%{http_code}' --max-time 5 "$@"; }
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -d "$BODY")
[ "$C" = "401" ] || fail "sem token deveria ser 401 (veio $C)"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -H 'Authorization: Bearer errado' -d "$BODY")
[ "$C" = "401" ] || fail "token errado deveria ser 401 (veio $C)"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" -H 'Origin: http://evil.example' -d "$BODY")
[ "$C" = "403" ] || fail "com Origin deveria ser 403 (veio $C)"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" -H 'Host: evil.example:80' -d "$BODY")
[ "$C" = "403" ] || fail "Host de DNS rebinding deveria ser 403 (veio $C)"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: text/plain' -H "Authorization: Bearer $TOKEN" -d "$BODY")
[ "$C" = "415" ] || fail "text/plain deveria ser 415 (veio $C)"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" -d "$BODY")
[ "$C" = "200" ] && [ "$(jq -r .data.pong "$OUT/http_body.txt")" = "true" ] || fail "HTTP valido deveria ser 200 (veio $C)"
head -c 3000000 /dev/zero | tr '\0' 'a' >"$OUT/big.bin"
C=$(code -X POST "http://127.0.0.1:$PORT/rpc" -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" --data-binary "@$OUT/big.bin")
[ "$C" = "413" ] || echo "aviso: corpo de 3 MB deveria ser 413 (curl veio $C; pode ser RST do lado do cliente)"
echo "seguranca HTTP ok (401/401/403/403/415/200)"
# o cliente do MCP com token errado recebe mensagem acionavel
if ANIGO_BRIDGE_ADDR="127.0.0.1:$PORT" ANIGO_BRIDGE_TOKEN=errado "$MCP" bridge-call PING >"$OUT/err.txt" 2>&1; then fail "token errado deveria falhar no cliente"; fi
grep -q "rejected the token" "$OUT/err.txt" || fail "mensagem de token recusado ausente: $(cat "$OUT/err.txt")"

# ------------------------------------------------------------------ 2. um Command pela bridge muda a viewport
sleep 2
U0=$(call GET_PROJECT '{"include_document":false}' | jq -r .history.undo_depth)
echo "undo inicial: $U0"
shot a_inicio
shot a_inicio2
D=$(diff_px a_inicio a_inicio2)
echo "ruido entre duas capturas identicas: $D px"
[ "$D" -lt 300 ] || fail "viewport instavel antes do teste ($D px)"
R=$(call APPLY_COMMAND '{"command":{"kind":"set_proportions","head_scale":1.4}}') || fail "APPLY_COMMAND falhou"
echo "aplicado: $(echo "$R" | jq -r .outcome.description) / undo_depth=$(echo "$R" | jq -r .outcome.undo_depth)"
[ "$(echo "$R" | jq -r .outcome.undo_depth)" = "$((U0 + 1))" ] || fail "o comando deveria entrar no historico"
sleep 3
shot b_apos_comando
D=$(diff_px a_inicio b_apos_comando)
echo "diferenca viewport antes/depois do comando: $D px"
[ "$D" -gt 3000 ] || fail "a UI nao refletiu o comando da bridge ($D px)"
[ "$(status_field .history.undo_depth)" = "$((U0 + 1))" ] || fail "GET_STATUS deveria ver o passo de undo"

# comando invalido nao muda nada
if call APPLY_COMMAND '{"command":{"kind":"nao_existe"}}' >"$OUT/err2.txt" 2>&1; then fail "comando invalido deveria falhar"; fi
grep -q "32602" "$OUT/err2.txt" || echo "aviso: codigo -32602 ausente: $(cat "$OUT/err2.txt")"
[ "$(status_field .history.undo_depth)" = "$((U0 + 1))" ] || fail "comando invalido alterou o historico"

# ------------------------------------------------------------------ 3. Ctrl+Z do usuario desfaz o comando do MCP
WID=""
for i in $(seq 1 40); do
  WID=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
  [ -n "$WID" ] && break
  sleep 0.25
done
[ -n "$WID" ] || fail "janela do app nao encontrada no Xvfb"
xdotool windowfocus --sync "$WID" 2>/dev/null || true
xdotool mousemove 700 500
sleep 0.5
ctrl_key z
wait_until "Ctrl+Z desfaz o comando do MCP" '[ "$(status_field .history.undo_depth)" = "$U0" ]' 20
sleep 3
shot c_apos_ctrl_z
D=$(diff_px a_inicio c_apos_ctrl_z)
echo "diferenca viewport inicio/depois do Ctrl+Z: $D px"
[ "$D" -lt 600 ] || fail "Ctrl+Z nao restaurou a viewport ($D px)"
[ "$(diff_px b_apos_comando c_apos_ctrl_z)" -gt 3000 ] || fail "viewport apos Ctrl+Z deveria diferir da do comando"

# REDO e UNDO pela propria bridge
call REDO >/dev/null || fail "REDO"
sleep 3
shot d_apos_redo
D=$(diff_px b_apos_comando d_apos_redo)
echo "diferenca viewport comando/redo: $D px"
[ "$D" -lt 600 ] || fail "REDO nao refez a viewport ($D px)"
call UNDO >/dev/null || fail "UNDO"
sleep 3
shot e_apos_undo
D=$(diff_px a_inicio e_apos_undo)
[ "$D" -lt 600 ] || fail "UNDO da bridge nao restaurou a viewport ($D px)"
LOGN=$(call GET_LOG '{"limit":5}' | jq -r .total)
echo "log da sessao: $LOGN entradas (o undo nao apaga o log)"
[ "$LOGN" -ge 1 ] || fail "log vazio"

# ------------------------------------------------------------------ 4. workspace, selecao e Biblioteca -> Cenario
call SET_WORKSPACE '{"workspace":"cenario"}' >/dev/null || fail "SET_WORKSPACE"
sleep 2
[ "$(status_field .workspace)" = "cenario" ] || fail "workspace nao mudou"
call GET_SELECTION | jq -e '.scene.node_ids | type == "array"' >/dev/null || fail "GET_SELECTION"
if call SET_WORKSPACE '{"workspace":"../../etc"}' >/dev/null 2>&1; then fail "workspace invalido deveria falhar"; fi

ASSET=""
for i in $(seq 1 60); do
  ASSET=$(call LIST_ASSETS '{"kinds":["model","character"],"limit":10}' 2>/dev/null | jq -r '.assets[0].id // empty')
  [ -n "$ASSET" ] && break
  sleep 1
done
[ -n "$ASSET" ] || fail "a Biblioteca nao listou ativos (assets/models deveria ser local ligado)"
echo "ativo: $ASSET ($(call LIST_ASSETS '{"limit":1}' | jq -r '.assets[0].name'), total $(call LIST_ASSETS '{"limit":1}' | jq -r .total))"
sleep 2
shot f_cenario_antes
N0=$(status_field .node_count)
UP0=$(status_field .history.undo_depth)
call PLACE_ASSET "{\"asset_id\":\"$ASSET\",\"position\":[0.0,0.0,0.0]}" >"$OUT/place.json" || fail "PLACE_ASSET: $(cat "$OUT/place.json")"
echo "colocado: $(jq -c '{placed,node_ids}' "$OUT/place.json")"
N1=$(status_field .node_count)
[ "$N1" -gt "$N0" ] || fail "PLACE_ASSET nao criou no ($N0 -> $N1)"
[ "$(status_field .history.undo_depth)" = "$((UP0 + 1))" ] || fail "colocar o ativo deveria ser UM passo de undo"
sleep 4
shot g_cenario_depois
D=$(diff_px f_cenario_antes g_cenario_depois)
echo "diferenca viewport antes/depois de colocar o ativo: $D px"
[ "$D" -gt 1500 ] || fail "a viewport do Cenario nao mostrou o ativo ($D px)"

# ------------------------------------------------------------------ 5. render real pela fila do workspace Render
call APPLY_COMMAND '{"command":{"kind":"set_render_output","patch":{"width":480,"height":270}}}' >/dev/null || fail "set_render_output"
call RENDER_FRAME >"$OUT/render_job.json" || fail "RENDER_FRAME: $(cat "$OUT/render_job.json")"
JOB=$(jq -r .job_id "$OUT/render_job.json")
echo "render job $JOB -> $(jq -r .output_dir "$OUT/render_job.json")"
wait_until "render concluido" '[ "$(call RENDER_STATUS "{\"job_id\":$JOB}" | jq -r .status)" = "done" ]' 240
call RENDER_STATUS "{\"job_id\":$JOB,\"inline\":true}" --save-png "$OUT/render.png" >"$OUT/render_status.json" || fail "RENDER_STATUS inline"
SIZE=$(identify -format '%wx%h' "$OUT/render.png" 2>/dev/null)
echo "render: $SIZE, arquivos: $(jq -c '[.files[].path]' "$OUT/render_status.json")"
[ "$SIZE" = "480x270" ] || fail "o render deveria ter 480x270 (veio $SIZE)"
[ -s "$(jq -r '.files[0].path' "$OUT/render_status.json")" ] || fail "arquivo do render nao existe no disco"

# captura da JANELA inteira (inclui a barra de status com o indicador da bridge)
call SCREENSHOT '{"target":"window","max_width":1200}' --save-png "$OUT/h_janela.png" >/dev/null || fail "screenshot da janela"
WSIZE=$(identify -format '%wx%h' "$OUT/h_janela.png" 2>/dev/null)
echo "janela: $WSIZE (max_width 1200 reduz mantendo a proporcao)"
[ "$WSIZE" = "1200x750" ] || fail "captura da janela com max_width=1200 deveria ser 1200x750 (veio $WSIZE)"

# ------------------------------------------------------------------ 6. desfazer o que a bridge fez (colocar o ativo) e sandbox de arquivos
call UNDO >/dev/null || fail "UNDO do render output"
call UNDO >/dev/null || fail "UNDO do PLACE_ASSET"
[ "$(status_field .node_count)" = "$N0" ] || fail "UNDO deveria remover o no colocado ($(status_field .node_count) != $N0)"
if call SCREENSHOT '{}' --save-png /etc/anigo_nao.png >/dev/null 2>&1; then fail "screenshot fora do sandbox deveria falhar"; fi
[ ! -e /etc/anigo_nao.png ] || fail "arquivo fora do sandbox foi criado"
call GET_METRICS | jq -c '{requests_total,rejected_auth,rejected_host_origin,rejected_too_large,timeouts}'
echo "OK: smoke da bridge nativa passou"
exit 0
