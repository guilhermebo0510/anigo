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
# Iluminacao: direcao da luz (azimute/elevacao/intensidade), rigs, sombra projetada, rosto, luz presa a camera
shot iluminacao_base iluminacao iluminacao-base
shot iluminacao_luz_az_n60 iluminacao iluminacao-luz-n60-35-100
shot iluminacao_luz_az_60 iluminacao iluminacao-luz-60-35-100
shot iluminacao_luz_el_78 iluminacao iluminacao-luz-0-78-100
shot iluminacao_luz_contraluz iluminacao iluminacao-luz-170-30-120
shot iluminacao_luz_fraca iluminacao iluminacao-luz-60-35-35
shot iluminacao_rig_tres_pontos iluminacao iluminacao-rig-three_point
shot iluminacao_rig_rim iluminacao iluminacao-rig-rim
shot iluminacao_rig_golden_hour iluminacao iluminacao-rig-golden_hour
shot iluminacao_rig_noite iluminacao iluminacao-rig-night
shot iluminacao_sombra iluminacao iluminacao-sombra
shot iluminacao_sombra_suave iluminacao iluminacao-sombra-suave
shot iluminacao_rosto iluminacao iluminacao-rosto
shot iluminacao_rosto_sem_sombra iluminacao iluminacao-rosto-sem
shot iluminacao_camera iluminacao iluminacao-camera
shot iluminacao_ambiente iluminacao iluminacao-ambiente
shot iluminacao_extras iluminacao iluminacao-extras
shot render_camera render render-camera
shot render_saida render render-saida
shot render_passes render render-passes
shot render_transparente render render-transparente
shot render_fila render render-fila
shot personagem personagem character-default
shot personagem_feminino personagem character-female
# reduz o tamanho (PNG otimizado, 8 bits)
for f in "$OUT"/*.png; do convert "$f" -strip -colors 128 "PNG8:$f" 2>/dev/null || true; done
ls -la "$OUT"
