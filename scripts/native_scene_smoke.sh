#!/bin/bash
# Smoke de entrada REAL do Cenario: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario)
#   1. arrasta o primeiro card do Content Drawer (Biblioteca) para a viewport  -> +1 no, selecionado, UM passo de undo;
#   2. arrasta o eixo X do gizmo de mover com snap de grade                    -> deslocamento em multiplo de 0.5 m, +1 passo;
#   3. Ctrl+Z desfaz o movimento e Ctrl+Y refaz (por trace E por diferenca de screenshot da viewport).
# O app imprime uma linha `scenetrace ...` por mudanca de estado (ANIGO_TRACE_SCENE=1); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup (o winit pode entregar a tecla antes do modificador).
# Uso: scripts/native_scene_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_scene_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:79}
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

ANIGO_TRACE_SCENE=1 "$BIN" --no-session --workspace cenario --scenario cenario-vazio --size 1440x900 >"$LOG" 2>&1 &
APID=$!
cleanup() {
  kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME"
}
trap cleanup EXIT

last() { grep '^scenetrace' "$LOG" | tail -1; }
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
# crop da viewport (sem a faixa inferior do HUD e sem as barras) para comparar por diferenca
crop() { # nome-origem nome-destino
  local v x0 y0 x1 y1
  v=$(field "$(last)" view)
  x0=$(echo "$v" | cut -d, -f1); y0=$(echo "$v" | cut -d, -f2); x1=$(echo "$v" | cut -d, -f3); y1=$(echo "$v" | cut -d, -f4)
  convert "$OUT/$1.png" -crop "$((x1 - x0 - 20))x$((y1 - y0 - 150))+$((x0 + 10))+$((y0 + 110))" +repage "$OUT/$2.png"
}
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
# acao em passos: o egui precisa de varios quadros durante um arrasto
drag() { # x0 y0 x1 y1
  local x0=$1 y0=$2 x1=$3 y1=$4 i
  xdotool mousemove "$x0" "$y0"; sleep 0.4
  xdotool mousedown 1; sleep 0.25
  for i in $(seq 1 14); do
    xdotool mousemove $((x0 + (x1 - x0) * i / 14)) $((y0 + (y1 - y0) * i / 14)); sleep 0.07
  done
  sleep 0.4
  xdotool mouseup 1; sleep 0.4
}
ctrl_key() { xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }

wait_trace "cenario carregado (manequim pronto e drawer visivel)" '[ "$(field "$T" nodes)" -ge 1 ] && [ "$(field "$T" loaded)" -ge 1 ] && [ "$(field "$T" card0)" != "0,0" ] && [ "$(field "$T" script)" = "done" ]' 150
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
xdotool mousemove 800 300
sleep 0.5

T=$(last)
N0=$(field "$T" nodes)
U0=$(field "$T" undo)
LOADED0=$(field "$T" loaded)
XY=$(field "$T" card0); CX=${XY%,*}; CY=${XY#*,}
V=$(field "$T" view)
VX0=$(echo "$V" | cut -d, -f1); VY0=$(echo "$V" | cut -d, -f2); VX1=$(echo "$V" | cut -d, -f3); VY1=$(echo "$V" | cut -d, -f4)
# alvo do soltar: piso, a esquerda e abaixo do centro da viewport (longe do manequim)
DX=$(((VX0 + VX1) / 2 - 150)); DY=$(((VY0 + VY1) / 2 + 80))
shot a_antes
crop a_antes a_antes_vp

# ---- 1. arrastar o card da Biblioteca para a viewport
drag "$CX" "$CY" "$DX" "$DY"
wait_trace "soltar o card cria UM no, selecionado, com UM passo de undo" '[ "$(field "$T" nodes)" = "'$((N0 + 1))'" ] && [ "$(field "$T" sel)" = "1" ] && [ "$(field "$T" undo)" = "'$((U0 + 1))'" ]' 30
wait_trace "a geometria do ativo soltado carregou" '[ "$(field "$T" loaded)" -gt '"$LOADED0"' ] && [ "$(field "$T" gizmo_x)" != "-" ]' 60
sleep 1
echo "soltou: $(last)"
shot b_depois_do_drop
crop b_depois_do_drop b_depois_do_drop_vp
D=$(diff_px a_antes_vp b_depois_do_drop_vp)
echo "diferenca viewport antes/depois do drop: $D px"
[ "$D" -gt 800 ] || fail "a viewport nao mudou apos o drop ($D px)"

# ---- 2. mover pelo eixo X do gizmo (snap de grade 0.5 m ligado por padrao)
T=$(last)
echo "antes de mover: $T"
POS0=$(field "$T" active)
X0=$(echo "$POS0" | cut -d, -f1)
GXY=$(field "$T" gizmo_x); GX=${GXY%,*}; GY=${GXY#*,}
# aproxima o cursor aos poucos e espera o app marcar a alca sob o cursor (hover) antes de pressionar
xdotool mousemove "$((GX - 15))" "$((GY + 6))"; sleep 0.3
xdotool mousemove "$GX" "$GY"; sleep 0.3
xdotool mousemove "$((GX + 1))" "$GY"; sleep 0.3
wait_trace "alca X do gizmo sob o cursor (hover)" '[ "$(field "$T" hover)" = "Axis(X)" ]' 20
xdotool mousedown 1; sleep 0.3
wait_trace "pressionar a alca inicia o arrasto do gizmo" '[ "$(field "$T" drag)" = "true" ]' 20
for i in $(seq 1 14); do xdotool mousemove $((GX + 1 + 70 * i / 14)) "$GY"; sleep 0.08; done
sleep 0.4
xdotool mouseup 1; sleep 0.4
wait_trace "o arrasto do gizmo vira UM comando (undo +1)" '[ "$(field "$T" undo)" = "'$((U0 + 2))'" ] && [ "$(field "$T" active)" != "'"$POS0"'" ]' 30
sleep 0.5
POS1=$(field "$(last)" active)
X1=$(echo "$POS1" | cut -d, -f1)
echo "moveu: $POS0 -> $POS1"
# o deslocamento tem de ser multiplo do passo da grade (0.5 m) e nao nulo
OK=$(awk -v a="$X0" -v b="$X1" 'BEGIN { d = b - a; k = d / 0.5; r = k - int(k + (k < 0 ? -0.5 : 0.5)); if (r < 0) r = -r; print (d != 0 && r < 0.01) ? 1 : 0 }')
[ "$OK" = "1" ] || fail "o deslocamento X ($X0 -> $X1) nao e multiplo de 0.5 m"
shot c_movido
crop c_movido c_movido_vp
D=$(diff_px b_depois_do_drop_vp c_movido_vp)
echo "diferenca viewport antes/depois de mover: $D px"
[ "$D" -gt 300 ] || fail "a viewport nao mudou apos mover ($D px)"

# ---- 3. Ctrl+Z / Ctrl+Y
xdotool mousemove "$((VX0 + 200))" "$((VY0 + 200))"; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z desfaz o movimento" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" redo)" = "1" ] && [ "$(field "$T" active)" = "'"$POS0"'" ]' 20
sleep 1
shot d_desfeito
crop d_desfeito d_desfeito_vp
D=$(diff_px b_depois_do_drop_vp d_desfeito_vp)
echo "diferenca viewport apos desfazer vs. antes de mover: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "desfazer nao restaurou a viewport ($D px)"

ctrl_key y
wait_trace "Ctrl+Y refaz o movimento" '[ "$(field "$T" undo)" = "'$((U0 + 2))'" ] && [ "$(field "$T" redo)" = "0" ] && [ "$(field "$T" active)" = "'"$POS1"'" ]' 20
sleep 1
shot e_refeito
crop e_refeito e_refeito_vp
D=$(diff_px c_movido_vp e_refeito_vp)
echo "diferenca viewport apos refazer vs. movido: $D px (esperado ~0)"
[ "$D" -lt 300 ] || fail "refazer nao restaurou a viewport ($D px)"

# Ctrl+Z duas vezes: remove o no colocado (o drop inteiro e um unico passo)
ctrl_key z; sleep 0.4; ctrl_key z
wait_trace "dois Ctrl+Z voltam ao estado inicial (o drop e UM passo)" '[ "$(field "$T" nodes)" = "'"$N0"'" ] && [ "$(field "$T" undo)" = "'"$U0"'" ]' 20
echo "OK: drop de card da Biblioteca, gizmo com snap, Ctrl+Z/Ctrl+Y com entrada real no Xvfb"
