#!/bin/bash
# Smoke de entrada REAL da Animacao: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario).
#   1. clique na regua da timeline -> o playhead vai para o quadro clicado (clicar no tempo);
#   2. tecla I -> chaves no quadro (clip criado, historico +1 ou +2); outra chave num quadro diferente;
#   3. arrastar uma chave no dope sheet -> a chave muda de quadro, UM passo de undo, durante o arrasto nada entra no historico;
#   4. Espaco -> reproduz (playing=true, o quadro avanca), Espaco de novo -> pausa;
#   5. Ctrl+Z desfaz o movimento da chave (quadro original), Ctrl+Y refaz.
# O app imprime uma linha `animtrace ...` por mudanca de estado (ANIGO_TRACE_ANIMATION=1); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup e `windowfocus` antes (o winit pode entregar a tecla antes do modificador).
# Uso: scripts/native_animation_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_animation_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:95}
mkdir -p "$OUT"
LOG="$OUT/smoke.log"
: >"$LOG"

Xvfb "$DISP" -screen 0 1600x1000x24 >/dev/null 2>&1 &
XPID=$!
sleep 1
if ! kill -0 "$XPID" 2>/dev/null; then
  echo "FALHA: nao consegui iniciar o Xvfb em $DISP"
  exit 2
fi
# daqui para a frente TUDO usa o display virtual; o display real do usuario nunca e tocado
export DISPLAY="$DISP"
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
# estado do egui (larguras de painel etc.) isolado: o smoke nao herda nem grava nada do usuario
export XDG_DATA_HOME
XDG_DATA_HOME=$(mktemp -d)

APID=""
APP_WID=""
cleanup() {
  [ -n "$APID" ] && kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME"
}
trap cleanup EXIT

last() { grep '^animtrace' "$LOG" | tail -1; }
field() { echo "$1" | sed -E "s/.* $2=([^ ]*).*/\1/"; }
shot() { import -window root "$OUT/$1.png" 2>/dev/null; }
fail() { echo "FALHA: $1"; echo "ultimo trace: $(last)"; shot falha; exit 1; }
wait_trace() { # descricao, condicao-shell que usa $T, timeout em s
  local desc="$1" cond="$2" tmo="${3:-60}" i=0
  while [ $i -lt $((tmo * 4)) ]; do
    T=$(last)
    if [ -n "$T" ] && eval "$cond"; then return 0; fi
    kill -0 "$APID" 2>/dev/null || fail "o app terminou antes de: $desc"
    sleep 0.25
    i=$((i + 1))
  done
  fail "tempo esgotado: $desc"
}
wait_stable() { # o trace nao muda por ~2 s
  local prev="" same=0 i=0
  while [ $i -lt 240 ]; do
    T=$(last)
    if [ -n "$T" ] && [ "$T" = "$prev" ]; then same=$((same + 1)); else same=0; fi
    prev="$T"
    [ $same -ge 8 ] && return 0
    kill -0 "$APID" 2>/dev/null || fail "o app terminou antes de assentar"
    sleep 0.25
    i=$((i + 1))
  done
  fail "o app nao assentou"
}
refocus() { xdotool windowfocus --sync "$APP_WID" 2>/dev/null || true; sleep 0.2; }
press() { refocus; xdotool keydown "$1"; sleep 0.15; xdotool keyup "$1"; sleep 0.3; }
ctrl_key() { refocus; xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }
# x (pixels) de um quadro e y da linha de regua, calculados do trace
frame_x() { awk -v t="$1" -v f="$2" 'BEGIN { split(t, c, ","); printf "%d", c[1] + (f - X0) * PPF * PPP }' X0="$X0" PPF="$PPF" PPP="$PPP"; }
load_geom() { # espera o layout assentar e le canvas/x0/ppf/ppp do trace
  local t canvas
  wait_stable
  t=$(last)
  canvas=$(field "$t" canvas)
  CX=$(echo "$canvas" | cut -d, -f1); CY=$(echo "$canvas" | cut -d, -f2)
  X0=$(field "$t" x0); PPF=$(field "$t" ppf); PPP=$(field "$t" ppp); RH=$(field "$t" ruler_h)
  RULER_Y=$(awk -v cy="$CY" -v rh="$RH" 'BEGIN { printf "%d", cy + rh / 2 }')
}
px_of() { awk -v cx="$CX" -v x0="$X0" -v ppf="$PPF" -v ppp="$PPP" -v f="$1" 'BEGIN { printf "%d", cx + (f - x0) * ppf * ppp }'; }
start_app() {
  : >"$LOG"
  ANIGO_TRACE_ANIMATION=1 "$BIN" --no-session --size 1440x900 "$@" >"$LOG" 2>&1 &
  APID=$!
  local WID="" i
  for i in $(seq 1 60); do
    WID=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
  done
  [ -n "$WID" ] || fail "janela do app nao encontrada no Xvfb"
  xdotool windowfocus --sync "$WID" 2>/dev/null || true
  APP_WID=$WID
  xdotool mousemove 40 700
}
stop_app() { kill "$APID" 2>/dev/null; wait "$APID" 2>/dev/null; APID=""; sleep 1; }

start_app --workspace animacao
wait_trace "workspace Animacao com a timeline desenhada" '[ "$(field "$T" ppf)" != "$T" ] && [ "$(field "$(echo "$T")" canvas)" != "0,0,0,0" ]' 150
wait_stable
T=$(last)
echo "inicio: $T"
[ "$(field "$T" clip)" = "-" ] || fail "nao devia haver clip no inicio"
U0=$(field "$T" undo)
load_geom
echo "geometria: canvas=$CX,$CY x0=$X0 ppf=$PPF ppp=$PPP ruler_h=$RH"

# ---- 1. clicar no tempo: quadro 12
X12=$(px_of 12)
xdotool mousemove "$X12" "$RULER_Y"; sleep 0.4
xdotool click 1; sleep 0.6
wait_trace "clicar na regua leva o playhead ao quadro 12" '[ "$(field "$T" frame)" = "12" ]' 20
echo "clicou no tempo: frame=$(field "$(last)" frame)"

# ---- 2. tecla I: chaves no quadro 12 (corpo inteiro)
xdotool mousemove 700 330; sleep 0.3
press i
wait_trace "I cria o clip e insere chaves no quadro 12" '[ "$(field "$T" clip)" != "-" ] && [ "$(field "$T" keys)" -gt 10 ]' 30
T=$(last)
K12=$(field "$T" keys)
U1=$(field "$T" undo)
echo "chaves no quadro 12: $K12 (undo $U0 -> $U1)"
[ "$U1" -gt "$U0" ] || fail "inserir chave nao entrou no historico"

# ---- 3. outra chave: quadro 30 (clicar no tempo, I)
load_geom
X30=$(px_of 30)
xdotool mousemove "$X30" "$RULER_Y"; sleep 0.4
xdotool click 1; sleep 0.6
wait_trace "playhead no quadro 30" '[ "$(field "$T" frame)" = "30" ]' 20
xdotool mousemove 700 330; sleep 0.3
press i
wait_trace "I insere chaves no quadro 30 (mesmo valor: o clip ja o tem, ou novas chaves)" '[ "$(field "$T" keys)" -ge "'"$K12"'" ]' 20
sleep 0.8
T=$(last)
KEYS_BEFORE=$(field "$T" ch0keys)
U2=$(field "$T" undo)
echo "chaves do primeiro canal: $KEYS_BEFORE (undo=$U2)"
case "$KEYS_BEFORE" in *12*) ;; *) fail "o primeiro canal devia ter chave no quadro 12";; esac

# ---- 4. arrastar a chave do quadro 12 do primeiro canal ate o quadro 20
load_geom
T=$(last)
ROW0_Y=$(field "$T" row0 | sed -E 's/.*@//')
KX=$(px_of 12)
TX=$(px_of 20)
shot a_antes
xdotool mousemove "$KX" "$ROW0_Y"; sleep 0.4
xdotool mousedown 1; sleep 0.3
for i in $(seq 1 12); do xdotool mousemove $((KX + (TX - KX) * i / 12)) "$ROW0_Y"; sleep 0.07; done
sleep 0.3
wait_trace "durante o arrasto o gesto esta em preview e nada novo entra no historico" '[ "$(field "$T" prev)" = "true" ] && [ "$(field "$T" undo)" = "'"$U2"'" ]' 20
xdotool mouseup 1; sleep 0.6
wait_trace "soltar grava UM passo de undo e a chave mudou de quadro" '[ "$(field "$T" undo)" = "'$((U2 + 1))'" ] && [ "$(field "$T" prev)" = "false" ] && [ "$(field "$T" ch0keys)" != "'"$KEYS_BEFORE"'" ]' 30
T=$(last)
KEYS_AFTER=$(field "$T" ch0keys)
echo "arrastou a chave: $KEYS_BEFORE -> $KEYS_AFTER"
case "$KEYS_AFTER" in *20*) ;; *) fail "a chave devia estar no quadro 20 ($KEYS_AFTER)";; esac
xdotool mousemove 700 330; sleep 0.8
shot b_arrastado

# ---- 5. Espaco: reproduz e pausa
press space
wait_trace "Espaco inicia a reproducao" '[ "$(field "$T" playing)" = "true" ]' 20
F0=$(field "$(last)" raw)
sleep 1.2
F1=$(field "$(last)" raw)
echo "reproduzindo: raw $F0 -> $F1"
shot c_tocando
press space
wait_trace "Espaco de novo pausa" '[ "$(field "$T" playing)" = "false" ]' 20
awk -v a="$F0" -v b="$F1" 'BEGIN { exit !(b != a) }' || fail "o relogio nao avancou durante a reproducao"

# ---- 6. Ctrl+Z / Ctrl+Y
xdotool mousemove 700 330; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z desfaz o movimento da chave" '[ "$(field "$T" ch0keys)" = "'"$KEYS_BEFORE"'" ] && [ "$(field "$T" redo)" -ge 1 ]' 20
ctrl_key y
wait_trace "Ctrl+Y refaz o movimento da chave" '[ "$(field "$T" ch0keys)" = "'"$KEYS_AFTER"'" ]' 20
echo "OK: clicar no tempo, I (insere chaves), arrastar chave (um passo, preview no arrasto), Espaco (tocar/pausar), Ctrl+Z/Ctrl+Y com entrada real no Xvfb"
stop_app
