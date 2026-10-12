#!/bin/bash
# Gera as screenshots de referência do app nativo (Xvfb + lavapipe) em docs/native/img/.
# Uso: scripts/native_screenshots.sh [diretório_saída]
set -euo pipefail
OUT=${1:-docs/native/img}
BIN=${BIN:-./target/debug/anigo-studio}
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json} XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp} WGPU_BACKEND=vulkan
mkdir -p "$OUT"
shot() { # nome workspace cenário
  xvfb-run -a -s "-screen 0 1600x1000x24" "$BIN" --no-session --workspace "$2" ${3:+--scenario "$3"} \
    --frames 14 --size 1280x800 --screenshot "$OUT/$1.png" 2>&1 | grep -E "erro|panic" || true
}
shot modelagem_objeto modelagem ""
shot modelagem_edicao_faces modelagem edit-faces
shot modelagem_grab_modal modelagem grab-modal
shot modelagem_gizmo_mover modelagem tool-move
shot modelagem_extrude_inset modelagem ops-demo
shot modelagem_bevel_cel modelagem ops-bevel
shot modelagem_proporcional modelagem proportional
shot modelagem_modificadores_cel modelagem modifiers
shot modelagem_cel_cena modelagem cel-preview
shot biblioteca biblioteca biblioteca-grid
shot biblioteca_detalhes biblioteca biblioteca-detalhes
shot modelagem_escultura modelagem sculpt-draw
shot modelagem_escultura_mascara modelagem sculpt-mask
shot modelagem_cabelo_desenho modelagem hair-draw
shot modelagem_cabelo_grupos modelagem hair-groups
shot modelagem_cabelo_cel modelagem hair-cel
shot modelagem_uv modelagem uv-editor
shot modelagem_normais_overlay modelagem normals-overlay
shot modelagem_normais_transfer modelagem normals-transfer-clean
shot cenario_vazio cenario cenario-vazio
shot cenario_montado cenario cenario-montado
shot cenario_snap cenario cenario-snap
shot cenario_outliner cenario cenario-outliner
shot cenario_gizmo cenario cenario-gizmo
shot cenario_camera cenario cenario-camera
shot posing_vazio posing posing-vazio
shot posing_fk posing posing-fk
shot posing_limites posing posing-limites
shot posing_ik posing posing-ik
shot posing_presets posing posing-presets
shot posing_mao posing posing-mao
shot posing_gaze posing posing-gaze
shot posing_biblioteca posing posing-biblioteca
shot personagem personagem character-default
shot personagem_feminino personagem character-female
# reduz o tamanho (PNG otimizado, 8 bits)
for f in "$OUT"/*.png; do convert "$f" -strip -colors 128 "PNG8:$f" 2>/dev/null || true; done
ls -la "$OUT"
