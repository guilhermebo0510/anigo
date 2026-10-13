#!/bin/bash
# Smoke de entrada REAL da Biblioteca: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario)
# rola a grade com a roda do mouse, clica num card, Ctrl+clica noutro e Ctrl+A. O app imprime uma linha
# `libtrace ...` por mudanca de estado (ANIGO_TRACE_LIB=1); o script confere os valores.
# Uso: scripts/native_library_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_library_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:78}
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

ANIGO_TRACE_LIB=1 ANIGO_SCENARIO_N=${ANIGO_SCENARIO_N:-6000} "$BIN" --no-session --workspace biblioteca \
  --scenario biblioteca-grid --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
}
trap cleanup EXIT

last() { grep '^libtrace' "$LOG" | tail -1; }
field() { echo "$1" | sed -E "s/.* $2=([^ ]*).*/\1/"; }
fail() { echo "FALHA: $1"; echo "ultimo trace: $(last)"; import -window root "$OUT/falha.png" 2>/dev/null; exit 1; }
# espera uma condicao sobre o ultimo trace (campo, operador numerico ou igualdade textual)
wait_trace() { # descricao, comando-shell que usa $T, timeout em s
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

wait_trace "grade carregada e assentada" '[ "$(field "$T" settled)" = "true" ] && [ "$(field "$T" total)" -ge 1000 ]' 150
echo "inicio: $(last)"
T=$(last)
TOTAL=$(field "$T" total)
COLS=$(field "$T" cols)

# sem gerenciador de janelas o foco do teclado precisa ser explicito (como em native_input_smoke.sh)
WID=""
for i in $(seq 1 40); do
  WID=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
  [ -n "$WID" ] && break
  sleep 0.25
done
[ -n "$WID" ] || fail "janela do app nao encontrada no Xvfb"
xdotool windowfocus --sync "$WID" 2>/dev/null || true
# o cenario deixa 3 cards selecionados: Esc limpa
xdotool mousemove 700 500
sleep 0.5
xdotool key Escape
wait_trace "Esc limpa a selecao" '[ "$(field "$T" sel)" = "0" ]' 20

# rolagem com a roda do mouse
FIRST0=$(field "$(last)" first)
for i in $(seq 1 40); do xdotool click 5; sleep 0.03; done
wait_trace "a roda rolou a grade" '[ "$(field "$T" first)" -gt "$FIRST0" ]' 20
FIRST1=$(field "$(last)" first)
echo "rolou: primeiro item visivel $FIRST0 -> $FIRST1"
wait_trace "paginas novas carregadas e sem estouro de memoria" '[ "$(field "$T" settled)" = "true" ] && [ "$(field "$T" loaded_pages)" -le 48 ]' 30

# clique num card
T=$(last)
XY=$(field "$T" card0)
CX=${XY%,*}
CY=${XY#*,}
xdotool mousemove "$CX" "$CY"
sleep 0.4
xdotool click 1
wait_trace "clique seleciona um card e carrega os detalhes" '[ "$(field "$T" sel)" = "1" ] && [ "$(field "$T" detail)" != "-" ]' 30
DETAIL=$(field "$(last)" detail)
echo "clicou no card $CX,$CY -> detalhes de: $DETAIL"
import -window root "$OUT/depois_do_clique.png" 2>/dev/null

# Ctrl+clique no card vizinho (mesma linha, uma coluna a direita; se so ha uma coluna, o da linha de baixo)
if [ "$COLS" -ge 2 ]; then NX=$((CX + 165)); NY=$CY; else NX=$CX; NY=$((CY + 195)); fi
xdotool mousemove "$NX" "$NY"
sleep 0.3
xdotool keydown ctrl click 1 keyup ctrl
wait_trace "Ctrl+clique soma ao selecionado" '[ "$(field "$T" sel)" = "2" ]' 20

# Ctrl+A seleciona tudo (a faixa e resolvida na thread de consulta, sem carregar tudo na UI)
xdotool mousemove 700 500
sleep 0.3
# 'key ctrl+a' envia tudo de uma vez e o winit pode entregar a tecla antes do modificador: sequencia explicita
xdotool keydown Control_L; sleep 0.2; xdotool key a; sleep 0.2; xdotool keyup Control_L
wait_trace "Ctrl+A seleciona todos os $TOTAL itens" '[ "$(field "$T" sel)" = "'"$TOTAL"'" ]' 30
import -window root "$OUT/selecionar_tudo.png" 2>/dev/null
echo "OK: rolagem, clique, Ctrl+clique e Ctrl+A funcionaram com entrada real no Xvfb ($TOTAL itens)"
