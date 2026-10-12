#!/bin/bash
# Smoke de entrada REAL do Iluminacao: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario)
#   1. arrasta o DISCO DO SOL (gizmo de direcao) na viewport  -> azimute/elevacao mudam e entra EXATAMENTE UM passo de undo;
#   2. Ctrl+Z restaura a direcao e a viewport (0 px de diferenca), Ctrl+Y refaz;
#   3. clica o botao do rig "golden hour" no painel          -> a luz esquenta (R > B), o rim muda, UM passo de undo;
#   4. Ctrl+Z desfaz o rig inteiro (luz + rim juntos).
# O app imprime uma linha `lighttrace ...` por mudanca de estado (ANIGO_TRACE_LIGHT=1); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup (o winit pode entregar a tecla antes do modificador).
# Uso: scripts/native_lighting_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_lighting_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:92}
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
# estado do egui isolado: o smoke nao herda nem grava nada do usuario
export XDG_DATA_HOME
XDG_DATA_HOME=$(mktemp -d)

ANIGO_TRACE_LIGHT=1 "$BIN" --no-session --workspace iluminacao --scenario iluminacao-base --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME"
}
trap cleanup EXIT

last() { grep '^lighttrace' "$LOG" | tail -1; }
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
# recorte do miolo da viewport (sem toolbar no topo nem as linhas do HUD embaixo) para comparar por diferenca
crop() { # nome-origem nome-destino
  local v x0 y0 x1 y1
  v=$(field "$(last)" view)
  x0=$(echo "$v" | cut -d, -f1); y0=$(echo "$v" | cut -d, -f2); x1=$(echo "$v" | cut -d, -f3); y1=$(echo "$v" | cut -d, -f4)
  convert "$OUT/$1.png" -crop "$((x1 - x0 - 20))x$((y1 - y0 - 170))+$((x0 + 10))+$((y0 + 60))" +repage "$OUT/$2.png"
}
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
# arrasto em passos: o egui precisa de varios quadros durante um arrasto
drag() { # x0 y0 x1 y1
  local x0=$1 y0=$2 x1=$3 y1=$4 i
  xdotool mousemove "$((x0 - 12))" "$((y0 + 4))"; sleep 0.3
  xdotool mousemove "$x0" "$y0"; sleep 0.4
  xdotool mousedown 1; sleep 0.3
  for i in $(seq 1 16); do
    xdotool mousemove $((x0 + (x1 - x0) * i / 16)) $((y0 + (y1 - y0) * i / 16)); sleep 0.07
  done
  sleep 0.4
  xdotool mouseup 1; sleep 0.5
}
ctrl_key() { xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }
# a diferenca absoluta entre dois angulos em graus (azimute dá a volta)
angdiff() { awk -v a="$1" -v b="$2" 'BEGIN { d = a - b; while (d > 180) d -= 360; while (d < -180) d += 360; if (d < 0) d = -d; print d }'; }
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { d = a - b; if (d < 0) d = -d; print (d <= t) ? 1 : 0 }'; }

wait_trace "cenario montado (script pronto, disco do sol visivel e botoes de rig medidos)" '[ "$(field "$T" script)" = "done" ] && [ "$(field "$T" disk)" != "-" ] && [ "$(field "$T" rigs)" != "-" ]' 150
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
xdotool mousemove 700 500
sleep 0.5

T=$(last)
U0=$(field "$T" undo)
AZ0=$(field "$T" az); EL0=$(field "$T" el)
DXY=$(field "$T" disk); DX=${DXY%,*}; DY=${DXY#*,}
shot a_antes
crop a_antes a_antes_vp

# ---- 1. arrastar o disco do sol: azimute/elevacao mudam, UM passo de undo
drag "$DX" "$DY" "$((DX - 170))" "$((DY - 70))"
wait_trace "soltar o disco grava UM comando (undo +1) com a direcao nova" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" drag)" = "false" ]' 30
sleep 1
T=$(last)
AZ1=$(field "$T" az); EL1=$(field "$T" el)
echo "arrastou: az $AZ0 -> $AZ1, el $EL0 -> $EL1"
DAZ=$(angdiff "$AZ1" "$AZ0")
DEL=$(near "$EL1" "$EL0" 4)
OK=$(awk -v a="$DAZ" 'BEGIN { print (a > 10) ? 1 : 0 }')
[ "$OK" = "1" ] || [ "$DEL" = "0" ] || fail "a direcao da luz quase nao mudou (az $AZ0 -> $AZ1, el $EL0 -> $EL1)"
shot b_arrastado
crop b_arrastado b_arrastado_vp
D=$(diff_px a_antes_vp b_arrastado_vp)
echo "diferenca viewport antes/depois de arrastar o sol: $D px"
[ "$D" -gt 1500 ] || fail "a imagem nao mudou apos arrastar o sol ($D px)"

# ---- 2. Ctrl+Z / Ctrl+Y
xdotool mousemove 700 520; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z restaura a direcao" '[ "$(field "$T" undo)" = "'"$U0"'" ] && [ "$(field "$T" redo)" = "1" ]' 20
sleep 1
T=$(last)
[ "$(near "$(field "$T" az)" "$AZ0" 0.6)" = "1" ] && [ "$(near "$(field "$T" el)" "$EL0" 0.6)" = "1" ] || fail "desfazer nao restaurou az/el ($(field "$T" az), $(field "$T" el) vs $AZ0, $EL0)"
shot c_desfeito
crop c_desfeito c_desfeito_vp
D=$(diff_px a_antes_vp c_desfeito_vp)
echo "diferenca viewport apos desfazer vs. antes: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "desfazer nao restaurou a viewport ($D px)"

ctrl_key y
wait_trace "Ctrl+Y refaz a direcao" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" redo)" = "0" ]' 20
sleep 1
shot d_refeito
crop d_refeito d_refeito_vp
D=$(diff_px b_arrastado_vp d_refeito_vp)
echo "diferenca viewport apos refazer vs. arrastado: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "refazer nao restaurou a viewport ($D px)"

# ---- 3. rig "golden hour" por clique no botao do painel
T=$(last)
COLOR0=$(field "$T" color)
RIM0=$(field "$T" rim)
RIGS=$(field "$T" rigs)
GXY=$(echo "$RIGS" | tr ';' '\n' | grep '^golden_hour:' | cut -d: -f2)
[ -n "$GXY" ] || fail "botao do rig golden_hour nao medido ($RIGS)"
GX=${GXY%,*}; GY=${GXY#*,}
xdotool mousemove "$((GX - 10))" "$GY"; sleep 0.3
xdotool mousemove "$GX" "$GY"; sleep 0.4
xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.5
wait_trace "clicar o rig aplica UM passo de undo e marca o rig" '[ "$(field "$T" undo)" = "'$((U0 + 2))'" ] && [ "$(field "$T" rig)" = "golden_hour" ]' 30
sleep 1
T=$(last)
COLOR1=$(field "$T" color)
RIM1=$(field "$T" rim)
echo "rig: cor $COLOR0 -> $COLOR1, rim $RIM0 -> $RIM1, az=$(field "$T" az) el=$(field "$T" el)"
R=$(echo "$COLOR1" | cut -d, -f1); B=$(echo "$COLOR1" | cut -d, -f3)
OK=$(awk -v r="$R" -v b="$B" 'BEGIN { print (r - b > 0.3) ? 1 : 0 }')
[ "$OK" = "1" ] || fail "golden hour deveria esquentar a luz (R=$R B=$B)"
[ "$RIM0" != "$RIM1" ] || fail "o rig deveria mudar o rim do personagem ($RIM0 -> $RIM1)"
shot e_golden
crop e_golden e_golden_vp
D=$(diff_px d_refeito_vp e_golden_vp)
echo "diferenca viewport antes/depois do rig: $D px"
[ "$D" -gt 3000 ] || fail "a imagem nao mudou apos o rig ($D px)"

# ---- 4. Ctrl+Z desfaz o rig inteiro (luz e rim juntos)
xdotool mousemove 700 520; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z desfaz o rig inteiro" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" rim)" = "'"$RIM0"'" ] && [ "$(field "$T" color)" = "'"$COLOR0"'" ]' 20
sleep 1
shot f_rig_desfeito
crop f_rig_desfeito f_rig_desfeito_vp
D=$(diff_px d_refeito_vp f_rig_desfeito_vp)
echo "diferenca viewport apos desfazer o rig vs. antes dele: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "desfazer o rig nao restaurou a viewport ($D px)"
echo "OK: arrastar o disco do sol, Ctrl+Z/Ctrl+Y e rig por clique com entrada real no Xvfb"
