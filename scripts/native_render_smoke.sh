#!/bin/bash
# Smoke de entrada REAL do workspace Render: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario)
#   1. clica em "trocar orientacao" (largura <-> altura)  -> w/h trocados, +1 passo de undo;
#   2. Ctrl+Z desfaz (w/h originais) e Ctrl+Y refaz (por trace E por diferenca de screenshot da viewport);
#   3. desfaz de novo e clica em "Renderizar quadro"      -> job termina, PNG real em disco (RGBA, tamanho certo, alfa nos cantos);
#   4. renderiza de novo                                  -> segundo arquivo com os MESMOS bytes de pixels (determinismo);
#   5. enfileira 4 em rajada e cancela o mais novo pelo botao da fila -> job "cancelled", nenhum arquivo novo.
# O app imprime uma linha `rendertrace ...` por mudanca de estado (ANIGO_TRACE_RENDER=1; espacos dos caminhos viram %20); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup (o winit pode entregar a tecla antes do modificador).
# Uso: scripts/native_render_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_render_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:93}
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
# estado do egui e pasta de saida isolados: o smoke nao herda nem grava nada do usuario
export XDG_DATA_HOME
XDG_DATA_HOME=$(mktemp -d)
RDIR=$(mktemp -d)
export ANIGO_RENDER_DIR="$RDIR"

ANIGO_TRACE_RENDER=1 "$BIN" --no-session --workspace render --scenario render-transparente --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME" "$RDIR"
}
trap cleanup EXIT

last() { grep '^rendertrace' "$LOG" | tail -1; }
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
click() { xdotool mousemove "$1" "$2"; sleep 0.35; xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.4; }
ctrl_key() { xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }
# crop da imagem do preview (sem o HUD de informacoes) para comparar por diferenca
crop() { # nome-origem nome-destino
  local v x0 y0 x1 y1
  v=$(field "$(last)" img)
  x0=$(echo "$v" | cut -d, -f1); y0=$(echo "$v" | cut -d, -f2); x1=$(echo "$v" | cut -d, -f3); y1=$(echo "$v" | cut -d, -f4)
  convert "$OUT/$1.png" -crop "$((x1 - x0 - 20))x$((y1 - y0 - 50))+$((x0 + 10))+$((y0 + 40))" +repage "$OUT/$2.png"
}
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }

wait_trace "cenario do Render pronto (script concluido)" '[ "$(field "$T" script)" = "done" ] && [ "$(field "$T" render_btn)" != "0,0" ] && [ "$(field "$T" swap_btn)" != "0,0" ]' 150
sleep 1
echo "inicio: $(last)"

WID=""
for i in $(seq 1 40); do
  WID=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
  [ -n "$WID" ] && break
  sleep 0.25
done
[ -n "$WID" ] || fail "janela do app nao encontrada no Xvfb"
xdotool windowfocus --sync "$WID" 2>/dev/null || true
xdotool mousemove 500 300
sleep 0.5

T=$(last)
W0=$(field "$T" w); H0=$(field "$T" h); U0=$(field "$T" undo)
XY=$(field "$T" swap_btn); SX=${XY%,*}; SY=${XY#*,}
shot a_antes
crop a_antes a_antes_vp

# ---- 1. trocar orientacao
click "$SX" "$SY"
wait_trace "trocar orientacao troca w/h e cria UM passo de undo" '[ "$(field "$T" w)" = "'"$H0"'" ] && [ "$(field "$T" h)" = "'"$W0"'" ] && [ "$(field "$T" undo)" = "'$((U0 + 1))'" ]' 30
sleep 1
shot b_trocado
crop b_trocado b_trocado_vp
D=$(diff_px a_antes_vp b_trocado_vp)
echo "diferenca do preview antes/depois de trocar: $D px"
[ "$D" -gt 2000 ] || fail "o preview nao mudou de enquadramento ($D px)"

# ---- 2. Ctrl+Z / Ctrl+Y
xdotool mousemove 500 300; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z desfaz" '[ "$(field "$T" w)" = "'"$W0"'" ] && [ "$(field "$T" h)" = "'"$H0"'" ] && [ "$(field "$T" redo)" = "1" ]' 20
sleep 1
shot c_desfeito
crop c_desfeito c_desfeito_vp
D=$(diff_px a_antes_vp c_desfeito_vp)
echo "diferenca apos desfazer vs. inicio: $D px (esperado ~0)"
[ "$D" -lt 400 ] || fail "desfazer nao restaurou o preview ($D px)"
ctrl_key y
wait_trace "Ctrl+Y refaz" '[ "$(field "$T" w)" = "'"$H0"'" ] && [ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" redo)" = "0" ]' 20
sleep 1
shot d_refeito
crop d_refeito d_refeito_vp
D=$(diff_px b_trocado_vp d_refeito_vp)
echo "diferenca apos refazer vs. trocado: $D px (esperado ~0)"
[ "$D" -lt 400 ] || fail "refazer nao restaurou o preview ($D px)"
ctrl_key z
wait_trace "Ctrl+Z volta ao tamanho original" '[ "$(field "$T" w)" = "'"$W0"'" ]' 20

# ---- 3. render real
T=$(last)
XY=$(field "$T" render_btn); RX=${XY%,*}; RY=${XY#*,}
click "$RX" "$RY"
wait_trace "o job de render termina com arquivos" '[ "$(field "$T" job)" = "done" ] && [ "$(field "$T" files)" != "-" ]' 180
F1=$(field "$(last)" files | cut -d, -f1 | sed "s/%20/ /g")
echo "render 1: $F1"
[ -s "$F1" ] || fail "arquivo de saida ausente: $F1"
python3 - "$F1" "$W0" "$H0" <<'PY' || fail "o PNG renderizado nao confere (tamanho/alfa)"
import sys
from PIL import Image
im = Image.open(sys.argv[1])
w, h = int(sys.argv[2]), int(sys.argv[3])
assert im.size == (w, h), f"tamanho {im.size} != {(w, h)}"
assert im.mode == "RGBA", f"modo {im.mode}"
px = im.load()
assert px[0, 0][3] == 0 and px[w - 1, h - 1][3] == 0, "cantos deveriam ser transparentes"
opaque = sum(1 for y in range(0, h, 4) for x in range(0, w, 4) if px[x, y][3] > 200)
assert opaque > 200, f"pouca geometria opaca: {opaque}"
assert any(k.startswith("anigo:") for k in im.info), "metadados anigo:* ausentes"
print("PNG ok:", im.size, im.mode, "amostras opacas:", opaque)
PY

# ---- 4. determinismo: o mesmo projeto renderizado de novo tem os mesmos pixels
N1=$(ls "$RDIR"/*.png 2>/dev/null | wc -l)
click "$RX" "$RY"
wait_trace "o segundo render termina" '[ "$(field "$T" jobs)" = "2" ] && [ "$(field "$T" job)" = "done" ]' 180
F2=$(field "$(last)" files | cut -d, -f1 | sed "s/%20/ /g")
[ -s "$F2" ] && [ "$F1" != "$F2" ] || fail "o segundo render nao gerou um arquivo novo ($F2)"
cmp -s "$F1" "$F2" || fail "mesmo projeto, bytes diferentes entre $F1 e $F2"
echo "determinismo: $(basename "$F1") == $(basename "$F2") (mesmos bytes)"
N2=$(ls "$RDIR"/*.png 2>/dev/null | wc -l)
[ "$N2" -eq $((N1 + 1)) ] || fail "o contador deveria criar um arquivo novo (antes $N1, depois $N2)"

# ---- 5. cancelar pela fila (o job mais novo fica atras dos outros e e cancelado antes de comecar).
# O botao de cancelar aponta para o job MAIS NOVO ainda em fila/rodando. Com so 2 jobs enfileirados por
# cliques separados (~1 s cada) a corrida era perdida em runner rapido: os dois renders terminavam antes do
# clique e o botao apontava para nada. Rajada de 4 cliques: o ultimo so comeca depois de 3 renders.
xdotool mousemove "$RX" "$RY"; sleep 0.35
xdotool click --repeat 4 --delay 120 1
wait_trace "quatro jobs novos enfileirados" '[ "$(field "$T" jobs)" = "6" ]' 30
T=$(last)
XY=$(field "$T" cancel_btn); CX=${XY%,*}; CY=${XY#*,}
[ "$CX" != "0" ] || fail "botao de cancelar da fila nao apareceu"
click "$CX" "$CY"
wait_trace "o job cancelado fica 'cancelled'" '[ "$(field "$T" job)" = "cancelled" ]' 60
echo "OK: trocar orientacao, Ctrl+Z/Ctrl+Y, render real (alfa, tamanho, metadados), determinismo e cancelar com entrada real no Xvfb"
