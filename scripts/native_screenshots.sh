#!/bin/bash
# Gera as screenshots de referência do app nativo (Xvfb + lavapipe) em docs/native/img/.
# Uso: scripts/native_screenshots.sh [diretório_saída]
set -euo pipefail
OUT=${1:-docs/native/img}
BIN=${BIN:-./target/debug/anigo-studio}
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json} XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp} WGPU_BACKEND=vulkan
mkdir -p "$OUT"
shot() { # nome workspace cenário [argumentos extras do studio]
  local n=$1 w=$2 s=$3; shift 3
  xvfb-run -a -s "-screen 0 1600x1000x24" "$BIN" --no-session --workspace "$w" ${s:+--scenario "$s"} \
    --frames 14 --size 1280x800 --screenshot "$OUT/$n.png" "$@" 2>&1 | grep -E "erro|panic" || true
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
shot shading_bola shading shading-ball
shot shading_rampa shading shading-rampa
shot shading_rim shading shading-rim
shot shading_contorno shading shading-contorno
shot shading_face_sdf shading shading-face-sdf
shot shading_face_sdf_frente shading shading-face-sdf-frente
shot shading_face_sdf_espelho shading shading-face-sdf-espelho
shot shading_ab shading shading-ab
shot shading_paleta shading shading-paleta
shot shading_presets shading shading-presets
shot shading_cenario shading shading-cenario
shot shading_modelagem shading shading-modelagem
shot posing_vazio posing posing-vazio
shot posing_fk posing posing-fk
shot posing_limites posing posing-limites
shot posing_ik posing posing-ik
shot posing_presets posing posing-presets
shot posing_mao posing posing-mao
shot posing_gaze posing posing-gaze
shot posing_biblioteca posing posing-biblioteca
# Animacao (estado do egui isolado por captura: as secoes abertas de uma nao vazam para a outra)
ashot() { XDG_DATA_HOME=$(mktemp -d) shot "$@"; }
ashot animacao_vazio animacao animacao-vazio
ashot animacao_timeline animacao animacao-timeline
ashot animacao_curvas animacao animacao-curvas
ashot animacao_passo animacao animacao-passo
ashot animacao_bezier animacao animacao-bezier
ashot animacao_onion animacao animacao-onion
ashot animacao_fisica animacao animacao-fisica
ashot animacao_clips animacao animacao-clips
ashot animacao_sequencia animacao animacao-sequencia
shot personagem personagem character-default
shot personagem_feminino personagem character-female
# T-2.12 layout dockavel: padrao de cada workspace (projeto vazio), layouts customizados carregados de arquivo (docs/native/layouts),
# arquivo corrompido (cai no padrao, sem panico), menus abertos, painel de atalhos (com e sem conflito) e "Sobre"
for w in personagem modelagem posing shading iluminacao cenario animacao render; do
  ashot "layout_padrao_$w" "$w" "layout-$w"
done
ashot layout_customizado_cenario cenario cenario-montado --layout-file docs/native/layouts/cenario_modelagem_de_cena.json
ashot layout_customizado_animacao animacao animacao-timeline --layout-file docs/native/layouts/animacao_timeline_alta.json
BAD=$(mktemp -d)/corrompido.json
printf '{{{ isto nao e json' >"$BAD"
ashot layout_corrompido_usa_padrao cenario cenario-montado --layout-file "$BAD"
ashot layout_menu_arquivo cenario layout-menu-file
ashot layout_menu_editar modelagem layout-menu-edit
ashot layout_menu_exibir cenario layout-menu-view
ashot layout_menu_janela cenario layout-menu-window
ashot layout_menu_ajuda cenario layout-menu-help
ashot layout_atalhos cenario layout-keyboard
ashot layout_atalhos_conflito cenario layout-keyboard-conflict
ashot layout_atalhos_modelagem modelagem layout-keyboard
ashot layout_sobre cenario layout-about
# reduz o tamanho (PNG otimizado, 8 bits)
for f in "$OUT"/*.png; do convert "$f" -strip -colors 128 "PNG8:$f" 2>/dev/null || true; done
ls -la "$OUT"
