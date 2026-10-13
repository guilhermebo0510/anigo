#!/bin/bash
# Smoke test de entrada real do Modo Escultura (Xvfb + xdotool + lavapipe).
# Entra em escultura (--scenario sculpt-ready), arrasta o mouse sobre a esfera e verifica por diferença de
# screenshots que a malha mudou; depois aciona Desfazer (Ctrl+Z) e verifica que a imagem volta ao estado inicial.
# uso: scripts/native_sculpt_smoke.sh [caminho do binário]
set -euo pipefail
BIN="${1:-./target/debug/anigo-studio}"
OUT="${ANIGO_TEST_OUT:-/tmp/anigo-sculpt-smoke}"
mkdir -p "$OUT"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}" WGPU_BACKEND=vulkan
export DISPLAY="${SMOKE_DISPLAY:-:97}"
Xvfb "$DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 &
XVFB=$!
APP=""
cleanup() { [ -n "$APP" ] && kill "$APP" 2>/dev/null || true; kill "$XVFB" 2>/dev/null || true; }
trap cleanup EXIT
sleep 1
"$BIN" --no-session --workspace modelagem --scenario sculpt-ready --size 1440x900 >"$OUT/app.log" 2>&1 &
APP=$!
sleep 8
xdotool search --class anigo-studio windowfocus || true   # sem WM, o foco do teclado precisa ser dado à mão
xdotool mousemove 1300 800            # fora do viewport: sem círculo do pincel nas capturas
sleep 1
import -window root "$OUT/before.png"
# arrasta sobre a esfera (LMB), com passos intermediários
xdotool mousemove 430 450
sleep 0.3
xdotool mousedown 1
for x in 450 480 510 540 570 600 630 660 690; do xdotool mousemove "$x" 450; sleep 0.08; done
xdotool mouseup 1
sleep 0.5
xdotool mousemove 1300 800
sleep 1
import -window root "$OUT/after.png"
# Desfazer por Ctrl+Z (ação global do keymap). Não clicar no botão da barra por coordenada fixa:
# a posição dele muda com o layout (barra de menus, largura da janela) e o clique errava em silêncio.
xdotool mousemove 600 450   # ponteiro sobre o app (a janela mede 1280 px; 1300,800 já é fora dela)
sleep 0.3
xdotool keydown ctrl; sleep 0.1; xdotool key z; sleep 0.1; xdotool keyup ctrl   # modificador segurado: `key ctrl+z` chega rápido demais
sleep 0.5
xdotool mousemove 1300 800
sleep 1
import -window root "$OUT/undone.png"
python3 -I - "$OUT" <<'PY'
import sys
from PIL import Image, ImageChops
d = sys.argv[1]
def region(n):
    return Image.open(f"{d}/{n}.png").convert("RGB").crop((60, 80, 1100, 870))
def diff(a, b):
    bbox = ImageChops.difference(region(a), region(b)).point(lambda v: 255 if v > 12 else 0).convert("L")
    return sum(1 for v in bbox.getdata() if v)
changed = diff("before", "after")
restored = diff("before", "undone")
print(f"pixels alterados pelo traço: {changed}; após desfazer: {restored}")
if changed < 500:
    sys.exit("FALHA: o arrasto do mouse não alterou a malha")
if restored > 50:
    sys.exit("FALHA: desfazer não restaurou a imagem inicial")
print("OK")
PY
