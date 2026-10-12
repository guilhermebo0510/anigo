#!/bin/bash
# Screenshots dos cenarios da Biblioteca (Xvfb + Vulkan por software/lavapipe). Cada cenario monta uma biblioteca
# temporaria SINTETICA (N ativos + ativos reais gerados pelo proprio studio) e espera as miniaturas 3D antes de
# capturar. Uso: scripts/native_library_screens.sh [binario] [pasta-de-saida] [cenarios...]
set -u
BIN=${1:-./target/debug/anigo-studio}
OUT=${2:-native-artifacts}
shift 2 2>/dev/null || true
SCENARIOS=${*:-"biblioteca-grid biblioteca-lista biblioteca-detalhes biblioteca-3d biblioteca-busca biblioteca-import biblioteca-gestao biblioteca-duplicados biblioteca-lixeira biblioteca-ausentes"}
mkdir -p "$OUT"
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
fail=0
for sc in $SCENARIOS; do
  png="$OUT/studio_$sc.png"
  rm -f "$png"
  # o ponteiro do Xvfb comeca no centro (sobre a janela e gera hover/tooltip); estaciona-o fora da janela do app
  timeout 240 xvfb-run -a -s "-screen 0 1600x1000x24" bash -c 'xdotool mousemove 1599 999 2>/dev/null; exec "$@"' _ "$BIN" \
    --no-session --workspace biblioteca --scenario "$sc" --frames 14 --size 1440x900 \
    --screenshot "$png" 2>"$OUT/studio_$sc.log"
  if [ -s "$png" ]; then
    echo "OK   $sc -> $png"
  else
    echo "FALHA $sc (veja $OUT/studio_$sc.log)"
    fail=1
  fi
done
exit $fail
