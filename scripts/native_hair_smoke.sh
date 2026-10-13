#!/bin/bash
# Smoke test de entrada real do Modo Cabelo (Xvfb PRÓPRIO + xdotool + lavapipe).
# Entra no Modo Cabelo (--scenario hair-ready: cabeça de teste, nenhuma mecha), arrasta o mouse sobre a
# cabeça (LMB) para desenhar uma mecha, confere por diferença de screenshots que a imagem mudou, desfaz
# com Ctrl+Z (keydown/key/keyup) e confere que voltou ao estado inicial; Ctrl+Shift+Z refaz.
# Só mexe no display do próprio Xvfb (nunca no do usuário): DISPLAY é sobrescrito antes de qualquer xdotool.
# uso: scripts/native_hair_smoke.sh [caminho do binário]
set -euo pipefail
BIN="${1:-./target/debug/anigo-studio}"
OUT="${ANIGO_TEST_OUT:-/tmp/anigo-hair-smoke}"
mkdir -p "$OUT"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}" WGPU_BACKEND=vulkan
export DISPLAY="${SMOKE_DISPLAY:-:96}"
# nunca reaproveita um display que já exista (poderia ser o do usuário)
DNUM="${DISPLAY#:}"
if [ -e "/tmp/.X11-unix/X$DNUM" ]; then
  echo "display $DISPLAY já existe; defina SMOKE_DISPLAY com um número livre" >&2
  exit 2
fi
Xvfb "$DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 &
XVFB=$!
APP=""
cleanup() { [ -n "$APP" ] && kill "$APP" 2>/dev/null || true; kill "$XVFB" 2>/dev/null || true; }
trap cleanup EXIT
sleep 1
"$BIN" --no-session --workspace modelagem --scenario hair-ready --size 1440x900 >"$OUT/app.log" 2>&1 &
APP=$!
sleep 8
xdotool search --class anigo-studio windowfocus || true   # sem WM, o foco do teclado precisa ser dado à mão
xdotool mousemove 1300 800            # fora do viewport: sem cursor do pincel nas capturas
sleep 1
import -window root "$OUT/before.png"
# desenha uma mecha: LMB do topo da cabeça para baixo, com passos intermediários
xdotool mousemove 530 270
sleep 0.3
xdotool mousedown 1
for p in "526 300" "520 340" "512 380" "506 420" "500 460" "494 500"; do
  xdotool mousemove $p
  sleep 0.08
done
xdotool mouseup 1
sleep 0.5
xdotool mousemove 1300 800
sleep 1
import -window root "$OUT/after.png"
# desfazer pelo teclado (o ponteiro precisa estar no viewport para os atalhos valerem)
xdotool mousemove 900 700
sleep 0.3
xdotool keydown ctrl
xdotool key z
xdotool keyup ctrl
sleep 0.5
xdotool mousemove 1300 800
sleep 1
import -window root "$OUT/undone.png"
# refazer: Ctrl+Shift+Z
xdotool mousemove 900 700
sleep 0.3
xdotool keydown ctrl
xdotool keydown shift
xdotool key z
xdotool keyup shift
xdotool keyup ctrl
sleep 0.5
xdotool mousemove 1300 800
sleep 1
import -window root "$OUT/redone.png"
python3 -I - "$OUT" <<'PY'
import sys
from PIL import Image, ImageChops
d = sys.argv[1]
def region(n):
    return Image.open(f"{d}/{n}.png").convert("RGB").crop((60, 80, 1000, 860))
def diff(a, b):
    bbox = ImageChops.difference(region(a), region(b)).point(lambda v: 255 if v > 12 else 0).convert("L")
    return bbox.histogram()[255]
changed = diff("before", "after")
restored = diff("before", "undone")
redone_vs_before = diff("before", "redone")
redone_vs_after = diff("after", "redone")  # só a seleção (guia e pontos) deixa de ser destacada após refazer
print(f"pixels alterados pelo traço: {changed}; após desfazer: {restored}; após refazer vs antes: {redone_vs_before}, vs traço: {redone_vs_after}")
if changed < 500:
    sys.exit("FALHA: o arrasto do mouse não desenhou uma mecha")
if restored > 50:
    sys.exit("FALHA: Ctrl+Z não restaurou a imagem inicial")
if redone_vs_before < 500 or redone_vs_after > 3000:
    sys.exit("FALHA: Ctrl+Shift+Z não refez a mecha")
print("OK")
PY
