#!/bin/bash
# Smoke test de ENTRADA REAL (xdotool) do app nativo sob Xvfb + Vulkan por software.
# Uso: scripts/native_input_smoke.sh <saida.png> [binário]
# Sequência: Tab (modo edição) → 3 (seleção de faces) → clique na face frontal → E (extrude) → mover o mouse → clique (confirma).
set -euo pipefail
OUT=${1:-/tmp/input_smoke.png}
BIN=${2:-./target/debug/anigo-studio}
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json} XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp} WGPU_BACKEND=vulkan
xvfb-run -a -s "-screen 0 1600x1000x24" bash -c '
  '"$BIN"' --no-session --workspace modelagem --frames 700 --size 1440x900 --screenshot '"$OUT"' &
  APP=$!
  for i in $(seq 1 60); do
    WID=$(xdotool search --name "ANIGO Studio" 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.5
  done
  sleep 4
  xdotool windowfocus --sync $WID 2>/dev/null || true   # sem gerenciador de janelas o foco precisa ser explícito
  xdotool mousemove 520 420          # sobre o cubo
  sleep 0.5
  xdotool key Tab; sleep 0.5          # modo edição
  xdotool key 3; sleep 0.5            # faces
  xdotool click 1; sleep 0.8          # seleciona a face sob o cursor
  xdotool key e; sleep 0.8            # extrude + grab ao longo da normal
  xdotool mousemove 560 380; sleep 0.4
  xdotool mousemove 600 340; sleep 0.4
  xdotool click 1; sleep 1.0          # confirma
  wait $APP
'
