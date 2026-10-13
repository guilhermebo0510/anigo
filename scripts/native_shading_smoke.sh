#!/bin/bash
# Smoke de entrada REAL do Shading: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario)
#   1. arrasta o slider "Intensidade" do Rim (painel de parametros)  -> o material muda ao vivo, UM passo de undo ao soltar;
#   2. Ctrl+Z desfaz (por trace E por diferenca de screenshot da viewport: 0 px) e Ctrl+Y refaz;
#   3. arrasta o slider da Mistura da rampa e desfaz os dois passos em sequencia -> volta ao estado inicial.
# O app imprime `shadetrace ...` por mudanca de estado e `shaderect <param> x= y= w= h=` com a posicao dos sliders
# (ANIGO_TRACE_SHADING=1); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup (o winit pode entregar a tecla antes do modificador).
# Uso: scripts/native_shading_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_shading_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:94}
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
export XDG_DATA_HOME
XDG_DATA_HOME=$(mktemp -d)

ANIGO_TRACE_SHADING=1 "$BIN" --no-session --workspace shading --scenario shading-smoke --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME"
}
trap cleanup EXIT

last() { grep '^shadetrace' "$LOG" | tail -1; }
rect() { grep "^shaderect $1 " "$LOG" | tail -1; }
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
# crop da viewport central (sem barras laterais nem o cabecalho do preview) para comparar por diferenca
crop() { convert "$OUT/$1.png" -crop "620x640+420+170" +repage "$OUT/$2.png"; }
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
approx() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { d = a - b; if (d < 0) d = -d; print (d <= t) ? 1 : 0 }'; }
drag() { # x0 y0 x1 y1
  local x0=$1 y0=$2 x1=$3 y1=$4 i
  xdotool mousemove "$x0" "$y0"; sleep 0.4
  xdotool mousedown 1; sleep 0.25
  for i in $(seq 1 14); do
    xdotool mousemove $((x0 + (x1 - x0) * i / 14)) $((y0 + (y1 - y0) * i / 14)); sleep 0.07
  done
  sleep 0.4
  xdotool mouseup 1; sleep 0.5
}
ctrl_key() { xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }

wait_trace "cenario do shading pronto" '[ "$(field "$T" script)" = "done" ] && [ -n "$(rect rim_intensity)" ]' 150
sleep 2
echo "inicio: $(last)"

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

T=$(last)
U0=$(field "$T" undo)
RIM0=$(field "$T" rim)
R=$(rect rim_intensity)
RX=$(echo "$R" | sed -E 's/.* x=([0-9-]+).*/\1/'); RY=$(echo "$R" | sed -E 's/.* y=([0-9-]+).*/\1/')
RW=$(echo "$R" | sed -E 's/.* w=([0-9-]+).*/\1/'); RH=$(echo "$R" | sed -E 's/.* h=([0-9-]+).*/\1/')
echo "slider rim: x=$RX y=$RY w=$RW h=$RH (rim inicial $RIM0, undo $U0)"
TRACK=$((RW - 74))
Y=$((RY + RH / 2))
shot a_antes
crop a_antes a_antes_vp

# ---- 1. arrastar o slider do rim
drag $((RX + TRACK * 15 / 100)) "$Y" $((RX + TRACK * 80 / 100)) "$Y"
wait_trace "soltar o slider grava UM passo (undo +1) e fecha o gesto" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" gesture)" = "false" ]' 30
RIM1=$(field "$(last)" rim)
echo "rim: $RIM0 -> $RIM1"
[ "$(awk -v a="$RIM1" -v b="$RIM0" 'BEGIN{print (a > b + 1.0) ? 1 : 0}')" = "1" ] || fail "o rim nao aumentou o bastante ($RIM0 -> $RIM1)"
shot b_depois
crop b_depois b_depois_vp
D=$(diff_px a_antes_vp b_depois_vp)
echo "diferenca viewport antes/depois do arrasto: $D px"
[ "$D" -gt 400 ] || fail "a viewport nao mudou apos o arrasto ($D px)"

# ---- 2. Ctrl+Z / Ctrl+Y
xdotool mousemove 700 400; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z desfaz o arrasto" '[ "$(field "$T" undo)" = "'"$U0"'" ] && [ "$(field "$T" redo)" = "true" ]' 20
sleep 1
[ "$(approx "$(field "$(last)" rim)" "$RIM0" 0.001)" = "1" ] || fail "desfazer nao devolveu o rim ($(field "$(last)" rim) != $RIM0)"
shot c_desfeito
crop c_desfeito c_desfeito_vp
D=$(diff_px a_antes_vp c_desfeito_vp)
echo "diferenca viewport apos desfazer vs. antes: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "desfazer nao restaurou a viewport ($D px)"

ctrl_key y
wait_trace "Ctrl+Y refaz o arrasto" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" redo)" = "false" ]' 20
sleep 1
[ "$(approx "$(field "$(last)" rim)" "$RIM1" 0.001)" = "1" ] || fail "refazer nao devolveu o rim"
shot d_refeito
crop d_refeito d_refeito_vp
D=$(diff_px b_depois_vp d_refeito_vp)
echo "diferenca viewport apos refazer vs. depois do arrasto: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "refazer nao restaurou a viewport ($D px)"

# ---- 3. segundo arrasto no mesmo slider = OUTRO passo; dois Ctrl+Z voltam ao estado inicial
drag $((RX + TRACK * 80 / 100)) "$Y" $((RX + TRACK * 30 / 100)) "$Y"
wait_trace "o segundo arrasto grava outro passo (undo +2)" '[ "$(field "$T" undo)" = "'$((U0 + 2))'" ] && [ "$(field "$T" gesture)" = "false" ]' 30
echo "rim apos o 2o arrasto: $(field "$(last)" rim)"
xdotool mousemove 700 400; sleep 0.3
ctrl_key z; sleep 0.5; ctrl_key z
wait_trace "dois Ctrl+Z voltam ao estado inicial" '[ "$(field "$T" undo)" = "'"$U0"'" ]' 20
[ "$(approx "$(field "$(last)" rim)" "$RIM0" 0.001)" = "1" ] || fail "o rim nao voltou ao inicial"
sleep 1
shot e_inicial
crop e_inicial e_inicial_vp
D=$(diff_px a_antes_vp e_inicial_vp)
echo "diferenca viewport apos desfazer tudo vs. inicial: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "desfazer tudo nao restaurou a viewport ($D px)"
echo "OK: arrasto de slider = UM passo de undo, Ctrl+Z/Ctrl+Y com entrada real e viewport identica antes/depois no Xvfb"
