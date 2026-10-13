#!/bin/bash
# Smoke de entrada REAL do Posing: xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario).
#   Fase 1 (FK, pose em repouso):
#     1. clique numa junta (cotovelo E) -> selecionada e ativa;
#     2. arrasto do anel Y do gizmo FK -> a pose sai do repouso, UM passo de undo, a viewport muda (diferenca de screenshot);
#     3. Ctrl+Z volta ao repouso exato (trace + 0 px de diferenca), Ctrl+Y refaz;
#     4. Esc no meio de um arrasto descarta o gesto (sem passo de undo);
#     5. caixa de selecao (marquee) seleciona joelho e tornozelo.
#   Fase 2 (IK, cenario posing-ik com braco/pernas em IK e pes travados):
#     6. arrasto do efetuador do braco E -> o punho segue, os tornozelos NAO saem do lugar, UM passo de undo;
#     7. Ctrl+Z restaura; arrasto do handle do quadril com "travar pes" -> o quadril desce e os tornozelos ficam (IK das pernas).
# O app imprime uma linha `posetrace ...` por mudanca de estado (ANIGO_TRACE_POSING=1); o script confere os valores.
# Teclas com modificador: sequencia explicita keydown/key/keyup (o winit pode entregar a tecla antes do modificador) e `windowfocus`.
# Uso: scripts/native_posing_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_posing_smoke}
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

last() { grep '^posetrace' "$LOG" | tail -1; }
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
wait_stable() { # o trace nao muda por ~2 s (cena assentada e personagem enquadrado)
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
  fail "a cena nao assentou"
}
# crop da viewport (sem as barras) para comparar por diferenca
crop() { # nome-origem nome-destino
  local v x0 y0 w h
  v=$(field "$(last)" rects)
  x0=$(echo "$v" | cut -d, -f1); y0=$(echo "$v" | cut -d, -f2); w=$(echo "$v" | cut -d, -f3); h=$(echo "$v" | cut -d, -f4)
  convert "$OUT/$1.png" -crop "$((w - 20))x$((h - 200))+$((x0 + 10))+$((y0 + 110))" +repage "$OUT/$2.png"
}
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
drag_to() { # x0 y0 x1 y1  (pressiona em (x0,y0), anda em passos ate (x1,y1) e solta)
  local x0=$1 y0=$2 x1=$3 y1=$4 i
  xdotool mousemove "$x0" "$y0"; sleep 0.4
  xdotool mousedown 1; sleep 0.3
  for i in $(seq 1 14); do
    xdotool mousemove $((x0 + (x1 - x0) * i / 14)) $((y0 + (y1 - y0) * i / 14)); sleep 0.07
  done
  sleep 0.4
  xdotool mouseup 1; sleep 0.5
}
# `import -window root` (screenshot) tira o foco do app: sem foco o egui ignora o teclado; por isso refoca antes de cada tecla
refocus() { xdotool windowfocus --sync "$APP_WID" 2>/dev/null || true; sleep 0.2; }
ctrl_key() { refocus; xdotool keydown Control_L; sleep 0.2; xdotool key "$1"; sleep 0.2; xdotool keyup Control_L; }
xy() { echo "${1%,*} ${1#*,}"; }
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { d = a - b; if (d < 0) d = -d; print (d <= t) ? 1 : 0 }'; }
vec_close() { # "x,y,z" "x,y,z" tolerancia -> 1 se cada componente difere menos que tol
  awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN { split(a, p, ","); split(b, q, ","); ok = 1; for (i = 1; i <= 3; i++) { d = p[i] - q[i]; if (d < 0) d = -d; if (d > t) ok = 0 } print ok }'
}
start_app() { # argumentos extras do app
  : >"$LOG"
  ANIGO_TRACE_POSING=1 "$BIN" --no-session --size 1440x900 "$@" >"$LOG" 2>&1 &
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

# =========================================================================== fase 1: FK
start_app --workspace posing
wait_trace "workspace Posing com o esqueleto na viewport" '[ "$(field "$T" j10)" != "$T" ] && [ "$(field "$T" rest)" = "true" ]' 150
wait_stable
T=$(last)
echo "inicio: $T"
U0=$(field "$T" undo)
[ "$(field "$T" active)" = "-" ] || fail "nada devia estar selecionado no inicio"

# ---- 1. clique numa junta (ombro E: gira livremente em Z dentro do limite humano)
read -r EX EY <<<"$(xy "$(field "$T" j9)")"
xdotool mousemove "$EX" "$EY"; sleep 0.4
xdotool click 1; sleep 0.6
wait_trace "clicar na junta do ombro E a seleciona e a torna ativa" '[ "$(field "$T" active)" = "LeftShoulder" ] && [ "$(field "$T" sel)" = "9" ]' 20
xdotool mousemove 600 400; sleep 1.2   # cursor neutro: o hover (destaque/rotulo) nao entra na comparacao
shot a_antes
crop a_antes a_antes_vp

# ---- 2. arrastar o anel Z do gizmo FK (abducao do braco)
T=$(last)
read -r CX CY <<<"$(xy "$(field "$T" j9)")"
read -r PX PY <<<"$(xy "$(field "$T" ring2)")"
# vetor radial e tangente (60 px) calculados com awk
read -r TX TY <<<"$(awk -v cx="$CX" -v cy="$CY" -v px="$PX" -v py="$PY" 'BEGIN { rx = px - cx; ry = py - cy; n = sqrt(rx*rx + ry*ry); if (n < 1) n = 1; printf "%d %d", px + (-ry / n) * 60, py + (rx / n) * 60 }')"
xdotool mousemove "$((PX - 6))" "$((PY - 4))"; sleep 0.3
xdotool mousemove "$PX" "$PY"; sleep 0.3
xdotool mousemove "$((PX + 1))" "$PY"; sleep 0.3
wait_trace "o anel Z do gizmo FK fica sob o cursor (hover)" '[ "$(field "$T" hover)" = "Ring(Z)" ]' 20
xdotool mousedown 1; sleep 0.3
wait_trace "pressionar o anel inicia o arrasto" '[ "$(field "$T" drag)" = "true" ] && [ "$(field "$T" prev)" = "true" ]' 20
for i in $(seq 1 14); do xdotool mousemove $((PX + 1 + (TX - PX) * i / 14)) $((PY + (TY - PY) * i / 14)); sleep 0.08; done
sleep 0.4
wait_trace "durante o arrasto nada entra no historico" '[ "$(field "$T" undo)" = "'"$U0"'" ] && [ "$(field "$T" rest)" = "false" ]' 20
xdotool mouseup 1; sleep 0.5
wait_trace "soltar o anel grava UM passo de undo com o ombro girado" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" drag)" = "false" ] && [ "$(field "$T" prev)" = "false" ] && echo "$T" | grep -q "LeftShoulder:"' 30
xdotool mousemove 600 400; sleep 1.2
POSE1=$(field "$(last)" pose)
echo "girou: $POSE1"
shot b_girado
crop b_girado b_girado_vp
D=$(diff_px a_antes_vp b_girado_vp)
echo "diferenca viewport antes/depois do arrasto: $D px"
[ "$D" -gt 150 ] || fail "a viewport nao mudou apos girar o osso ($D px)"

# ---- 3. Ctrl+Z / Ctrl+Y
xdotool mousemove 600 400; sleep 0.3
ctrl_key z
wait_trace "Ctrl+Z volta ao repouso (undo 0, redo 1)" '[ "$(field "$T" undo)" = "'"$U0"'" ] && [ "$(field "$T" redo)" = "1" ] && [ "$(field "$T" rest)" = "true" ]' 20
sleep 1
shot c_desfeito
crop c_desfeito c_desfeito_vp
D=$(diff_px a_antes_vp c_desfeito_vp)
echo "diferenca viewport apos desfazer vs. repouso: $D px (esperado ~0)"
[ "$D" -lt 120 ] || fail "desfazer nao restaurou a viewport ($D px)"
ctrl_key y
wait_trace "Ctrl+Y refaz a pose" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" redo)" = "0" ] && [ "$(field "$T" pose)" = "'"$POSE1"'" ]' 20
sleep 1
shot d_refeito
crop d_refeito d_refeito_vp
D=$(diff_px b_girado_vp d_refeito_vp)
echo "diferenca viewport apos refazer vs. girado: $D px (esperado ~0)"
[ "$D" -lt 120 ] || fail "refazer nao restaurou a viewport ($D px)"

# ---- 4. Esc no meio de um arrasto descarta o gesto
T=$(last)
read -r PX PY <<<"$(xy "$(field "$T" ring0)")"
xdotool mousemove "$((PX - 6))" "$((PY - 4))"; sleep 0.3
xdotool mousemove "$PX" "$PY"; sleep 0.3
xdotool mousemove "$((PX + 1))" "$PY"; sleep 0.3
wait_trace "o anel X fica sob o cursor" '[ "$(field "$T" hover)" = "Ring(X)" ]' 20
xdotool mousedown 1; sleep 0.3
for i in $(seq 1 8); do xdotool mousemove "$((PX + 1 + i * 6))" "$((PY - i * 4))"; sleep 0.08; done
wait_trace "arrasto do anel X em curso" '[ "$(field "$T" drag)" = "true" ]' 20
refocus; xdotool key Escape; sleep 0.5
wait_trace "Esc cancela o gesto (pose igual, sem passo novo)" '[ "$(field "$T" drag)" = "false" ] && [ "$(field "$T" prev)" = "false" ] && [ "$(field "$T" pose)" = "'"$POSE1"'" ] && [ "$(field "$T" undo)" = "'$((U0 + 1))'" ]' 20
xdotool mouseup 1; sleep 0.5
T=$(last)
[ "$(field "$T" undo)" = "$((U0 + 1))" ] || fail "soltar o botao depois do Esc nao pode criar passo de undo"

# ---- 5. caixa de selecao (marquee): joelho E e tornozelo E
T=$(last)
read -r KX KY <<<"$(xy "$(field "$T" j17)")"
read -r AX AY <<<"$(xy "$(field "$T" j18)")"
drag_to "$((KX + 90))" "$((KY - 25))" "$((AX - 45))" "$((AY + 25))"
wait_trace "a caixa seleciona joelho E (17) e tornozelo E (18)" '[ "$(field "$T" sel)" = "17,18" ]' 20
echo "marquee: $(field "$(last)" sel)"
stop_app

# =========================================================================== fase 2: IK
start_app --workspace posing --scenario posing-ik
wait_trace "cenario posing-ik pronto" '[ "$(field "$T" script)" = "done" ] && [ "$(field "$T" ik)" = "1111" ]' 200
wait_stable
T=$(last)
echo "ik inicio: $T"
U0=$(field "$T" undo)
LW0=$(field "$T" lwrist); LA0=$(field "$T" lankle); RA0=$(field "$T" rankle); H0=$(field "$T" hips)

# ---- 6. arrastar o efetuador do braco esquerdo (eff0)
read -r PX PY <<<"$(xy "$(field "$T" eff0)")"
xdotool mousemove "$((PX - 5))" "$((PY - 5))"; sleep 0.3
xdotool mousemove "$PX" "$PY"; sleep 0.3
xdotool mousemove "$((PX + 1))" "$PY"; sleep 0.3
wait_trace "o efetuador do braco E fica sob o cursor" '[ "$(field "$T" hover)" = "Effector(LeftArm)" ]' 20
xdotool mousedown 1; sleep 0.3
wait_trace "pressionar o efetuador inicia o arrasto de IK" '[ "$(field "$T" drag)" = "true" ]' 20
for i in $(seq 1 14); do xdotool mousemove $((PX + 1 - 70 * i / 14)) $((PY - 45 * i / 14)); sleep 0.08; done
sleep 0.4
xdotool mouseup 1; sleep 0.5
wait_trace "soltar o efetuador grava UM passo e o punho se moveu" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" drag)" = "false" ] && [ "$(field "$T" lwrist)" != "'"$LW0"'" ]' 30
sleep 0.6
T=$(last)
LW1=$(field "$T" lwrist)
echo "punho: $LW0 -> $LW1"
[ "$(vec_close "$LW0" "$LW1" 0.04)" = "0" ] || fail "o punho quase nao se moveu"
[ "$(vec_close "$(field "$T" lankle)" "$LA0" 0.002)" = "1" ] || fail "o tornozelo E saiu do lugar durante o IK do braco"
[ "$(vec_close "$(field "$T" rankle)" "$RA0" 0.002)" = "1" ] || fail "o tornozelo D saiu do lugar durante o IK do braco"
ctrl_key z
wait_trace "Ctrl+Z restaura o punho" '[ "$(field "$T" undo)" = "'"$U0"'" ] && [ "$(field "$T" lwrist)" = "'"$LW0"'" ]' 20

# ---- 7. quadril com "travar pes": desce e os tornozelos ficam
T=$(last)
read -r HX HY <<<"$(xy "$(field "$T" j1)")"
xdotool mousemove "$((HX - 5))" "$((HY - 5))"; sleep 0.3
xdotool mousemove "$HX" "$HY"; sleep 0.3
xdotool mousemove "$((HX + 1))" "$HY"; sleep 0.3
wait_trace "o handle do quadril fica sob o cursor" '[ "$(field "$T" hover)" = "Hips" ]' 20
xdotool mousedown 1; sleep 0.3
for i in $(seq 1 14); do xdotool mousemove $((HX + 1)) $((HY + 36 * i / 14)); sleep 0.08; done
sleep 0.4
xdotool mouseup 1; sleep 0.5
wait_trace "mover o quadril grava UM passo" '[ "$(field "$T" undo)" = "'$((U0 + 1))'" ] && [ "$(field "$T" drag)" = "false" ] && [ "$(field "$T" hips)" != "'"$H0"'" ]' 30
sleep 0.6
T=$(last)
H1=$(field "$T" hips)
echo "quadril: $H0 -> $H1"
[ "$(vec_close "$(field "$T" lankle)" "$LA0" 0.003)" = "1" ] || fail "foot lock: o tornozelo E saiu do lugar ($LA0 -> $(field "$T" lankle))"
[ "$(vec_close "$(field "$T" rankle)" "$RA0" 0.003)" = "1" ] || fail "foot lock: o tornozelo D saiu do lugar ($RA0 -> $(field "$T" rankle))"
echo "OK: clique/marquee de juntas, gizmo FK (anel) com UM passo de undo, Esc cancela, Ctrl+Z/Ctrl+Y, IK de braco e foot lock com entrada real no Xvfb"
stop_app
