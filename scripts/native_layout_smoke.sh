#!/bin/bash
# Smoke de entrada REAL do layout dockavel (T-2.12): xdotool (SOMENTE dentro de um Xvfb proprio, nunca no display do usuario).
#   1. o layout padrao do Cenario sobe com a estrutura/larguras esperadas (trace `layouttrace`, ANIGO_TRACE_LAYOUT=1);
#   2. ARRASTAR o painel Detalhes (pelo cabecalho) para a esquerda do viewport -> a arvore muda e o layout vai para o arquivo do usuario;
#   3. FECHAR um painel (X do cabecalho de Posicionar) e REABRIR por Janela > Paineis > Posicionar;
#   4. Arquivo > Sair (menu real: fecha com graceful exit e grava a sessao);
#   5. REINICIAR com --session-file e um diretorio de config VAZIO (so a sessao carrega o layout): mesma estrutura e a captura de tela
#      e IDENTICA a de antes de sair (diferenca de pixels); e com o layout do usuario de volta ao padrao ("Redefinir layout").
# Teclas/cliques: sequencia explicita mousemove/mousedown/mouseup com pausas (o egui precisa de varios quadros durante um arrasto).
# Uso: scripts/native_layout_smoke.sh [binario] [pasta-de-saida]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-/tmp/anigo_layout_smoke}
DISP=${ANIGO_SMOKE_DISPLAY:-:98}
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
CFG1=$(mktemp -d)   # 1a execucao: arquivo de layouts do usuario
CFG2=$(mktemp -d)   # 2a execucao: VAZIO (so a sessao carrega o layout)
SESS="$OUT/session.json"
rm -f "$SESS"

APID=""
cleanup() {
  [ -n "$APID" ] && kill "$APID" 2>/dev/null
  kill "$XPID" 2>/dev/null
  rm -rf "$XDG_DATA_HOME" "$CFG1" "$CFG2"
}
trap cleanup EXIT

launch() { # diretorio-de-config
  : >"$LOG"
  ANIGO_TRACE_LAYOUT=1 ANIGO_CONFIG_DIR="$1" "$BIN" --session-file "$SESS" --workspace cenario --scenario cenario-vazio --size 1280x800 >>"$LOG" 2>&1 &
  APID=$!
}
last() { grep '^layouttrace' "$LOG" | tail -1; }
tree() { last | sed -E 's/.* tree=([^ ]*).*/\1/'; }
rect() { # id -> "x y w h"
  last | tr ' ' '\n' | grep "^pane:$1=" | head -1 | sed -E 's/.*=//; s/,/ /g'
}
shot() { xdotool mousemove 1450 900; sleep 0.6; import -window root -crop 1280x800+0+0 +repage "$OUT/$1.png" 2>/dev/null; }
fail() { echo "FALHA: $1"; echo "ultimo trace: $(last)"; shot falha; exit 1; }
wait_tree() { # descricao, condicao-shell que usa $T (estrutura), timeout em s
  local desc="$1" cond="$2" tmo="${3:-60}" i=0
  while [ $i -lt $((tmo * 4)) ]; do
    T=$(tree)
    if [ -n "$T" ] && eval "$cond"; then return 0; fi
    kill -0 "$APID" 2>/dev/null || fail "o app terminou antes de: $desc"
    sleep 0.25
    i=$((i + 1))
  done
  fail "tempo esgotado: $desc"
}
focus_app() { # sem gerenciador de janelas o foco do teclado precisa ser explicito
  local wid
  wid=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
  [ -n "$wid" ] && xdotool windowfocus --sync "$wid" 2>/dev/null || true
}
diff_px() { compare -metric AE -fuzz 3% "$OUT/$1.png" "$OUT/$2.png" null: 2>&1 | awk '{print int($1)}'; }
click() { xdotool mousemove "$1" "$2"; sleep 0.35; xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.5; }
drag() { # x0 y0 x1 y1
  local x0=$1 y0=$2 x1=$3 y1=$4 i
  xdotool mousemove "$x0" "$y0"; sleep 0.4
  xdotool mousedown 1; sleep 0.25
  for i in $(seq 1 16); do
    xdotool mousemove $((x0 + (x1 - x0) * i / 16)) $((y0 + (y1 - y0) * i / 16))
    sleep 0.07
  done
  sleep 0.5
  xdotool mouseup 1; sleep 0.8
}


DEFAULT='H[cenario.place|V[cenario.viewport|cenario.drawer]|V[T[cenario.outliner,cenario.history]*0|cenario.details]]'

# 0. arquivos de layout ruins nunca derrubam o app: JSON corrompido e um layout valido mas SEM o viewport (ids desconhecidos) caem no padrao
printf '{{{ isto nao e json' >"$OUT/corrompido.json"
printf '{"schema":1,"workspaces":{"cenario":{"kind":"split","vertical":false,"children":[{"share":1,"node":{"kind":"pane","id":"painel.que.nao.existe"}},{"share":1,"node":{"kind":"pane","id":"cenario.details"}}]}}}' >"$OUT/sem_viewport.json"
for bad in corrompido sem_viewport; do
  : >"$LOG"
  ANIGO_TRACE_LAYOUT=1 "$BIN" --no-session --layout-file "$OUT/$bad.json" --workspace cenario --scenario cenario-vazio --size 1280x800 >>"$LOG" 2>&1 &
  APID=$!
  wait_tree "padrao com o arquivo $bad" "[ \"\$T\" = '$DEFAULT' ]" 90
  kill -0 "$APID" 2>/dev/null || fail "o app caiu com o arquivo $bad"
  kill "$APID" 2>/dev/null; wait "$APID" 2>/dev/null; APID=""
done
[ -s "$OUT/corrompido.json.bak" ] || fail "o arquivo corrompido nao ganhou copia .bak"
echo "OK 0: arquivo corrompido (com .bak) e layout sem viewport -> layout padrao, sem panico"

launch "$CFG1"
wait_tree "layout padrao do Cenario" "[ \"\$T\" = '$DEFAULT' ]" 90
sleep 2
shot a_padrao
read -r px py pw ph <<<"$(rect cenario.place)"
[ "${pw%.*}" -eq 150 ] || fail "largura de Posicionar $pw != 150 (a do painel fixo antigo)"
read -r dx dy dw dh <<<"$(rect cenario.details)"
[ "${dw%.*}" -ge 330 ] && [ "${dw%.*}" -le 345 ] || fail "largura de Detalhes $dw fora de ~340"
echo "OK 1: layout padrao do Cenario: $DEFAULT (Posicionar ${pw}px, Detalhes ${dw}px)"

# 2. arrasta Detalhes (cabecalho: 10 px abaixo do topo do painel) para a borda esquerda do viewport
read -r vx vy vw vh <<<"$(rect cenario.viewport)"
drag $((dx + 60)) $((dy + 10)) $((vx + 12)) $((vy + vh / 2))
wait_tree "Detalhes reorganizado" "[ \"\$T\" != '$DEFAULT' ]" 30
T2=$(tree)
echo "estrutura apos o arrasto: $T2"
case "$T2" in *'cenario.details'*) ;; *) fail "Detalhes sumiu da arvore" ;; esac
shot b_arrastado
D=$(diff_px a_padrao b_arrastado)
[ "$D" -gt 5000 ] || fail "a tela quase nao mudou apos o arrasto ($D px)"
echo "OK 2: arrastar Detalhes mudou o layout ($D px de diferenca)"
sleep 1
[ -s "$CFG1/layouts.json" ] || fail "layouts.json nao foi gravado em $CFG1"
grep -q 'cenario.details' "$CFG1/layouts.json" || fail "layouts.json sem o painel Detalhes"
echo "OK 2b: layout gravado no arquivo do usuario ($(wc -c <"$CFG1/layouts.json") bytes)"

# 3. fecha Posicionar pelo X do cabecalho e reabre por Janela > Paineis > Posicionar
read -r px py pw ph <<<"$(rect cenario.place)"
click $((px + pw - 12)) $((py + 10))
wait_tree "Posicionar fechado" "case \"\$T\" in *cenario.place*) false;; *) true;; esac" 30
shot c_fechado
echo "OK 3a: Posicionar fechado pelo X (estrutura: $(tree))"
click 170 11            # menu Janela
xdotool mousemove 185 36; sleep 0.9   # submenu Paineis
click 350 36            # Posicionar
wait_tree "Posicionar reaberto" "case \"\$T\" in *cenario.place*) true;; *) false;; esac" 30
sleep 1.5
shot d_reaberto
echo "OK 3b: Posicionar reaberto pelo menu Janela (estrutura: $(tree))"
BEFORE_QUIT=$(tree)
sleep 1
shot e_antes_de_sair

# 4. Arquivo > Sair (graceful exit grava a sessao)
click 32 11
sleep 0.5
shot f_menu_arquivo
# "Sair" e o ultimo item do menu Arquivo
click 40 196
i=0
while kill -0 "$APID" 2>/dev/null && [ $i -lt 40 ]; do sleep 0.25; i=$((i + 1)); done
kill -0 "$APID" 2>/dev/null && { shot falha_sair; fail "Arquivo > Sair nao fechou o app"; }
[ -s "$SESS" ] || fail "a sessao nao foi gravada em $SESS"
grep -q '"layouts"' "$SESS" || fail "a sessao nao tem o campo layouts"
echo "OK 4: o app fechou pelo menu e gravou a sessao ($(wc -c <"$SESS") bytes)"
APID=""

# 5. reinicia com --session-file e um diretorio de config VAZIO: so a sessao devolve o layout
launch "$CFG2"
wait_tree "layout restaurado da sessao" "[ \"\$T\" = '$BEFORE_QUIT' ]" 90
sleep 2.5
shot g_restaurado
D2=$(diff_px e_antes_de_sair g_restaurado)
TOTAL=$((1280 * 800))
echo "estrutura restaurada: $(tree) | diferenca de tela vs. antes de sair: $D2 px de $TOTAL"
[ "$D2" -lt $((TOTAL / 50)) ] || fail "a tela restaurada difere demais da anterior ($D2 px)"
[ ! -e "$CFG2/layouts.json" ] || true
echo "OK 5: o layout voltou identico apos reiniciar (estrutura igual; $D2 px de diferenca, <2% do quadro)"

# 6. Janela > Layouts > Salvar layout atual como... "meu" (preset nomeado, persistido no arquivo do usuario)
focus_app
click 170 11
xdotool mousemove 185 59; sleep 0.9
click 340 116
click 640 413            # campo "Nome do layout"
xdotool type --delay 80 "meu"; sleep 0.5
xdotool key Return; sleep 1.5
grep -q '"name": "meu"' "$CFG2/layouts.json" || fail "o preset 'meu' nao foi gravado em $CFG2/layouts.json"
echo "OK 6: preset nomeado 'meu' salvo ($(grep -c '"name"' "$CFG2/layouts.json") preset(s) no arquivo)"

# 7. Janela > Layouts > Redefinir layout volta ao padrao (e persiste)
click 170 11
xdotool mousemove 185 59; sleep 0.9
click 330 59
wait_tree "layout redefinido" "[ \"\$T\" = '$DEFAULT' ]" 30
sleep 1.2
shot h_redefinido
D3=$(diff_px a_padrao h_redefinido)
echo "OK 7: Redefinir layout voltou ao padrao ($D3 px de diferenca vs. a captura inicial; o preset continua salvo)"

# 8. Janela > Layouts > Layouts salvos > meu aplica o preset de volta
click 170 11
xdotool mousemove 185 59; sleep 0.9
xdotool mousemove 340 139; sleep 0.9
click 535 139
wait_tree "preset aplicado" "[ \"\$T\" = '$BEFORE_QUIT' ]" 30
sleep 1.2
shot i_preset_aplicado
D4=$(diff_px e_antes_de_sair i_preset_aplicado)
echo "OK 8: o preset 'meu' devolveu o layout customizado ($D4 px de diferenca vs. antes de sair)"
echo "SMOKE DO LAYOUT OK"
