#!/bin/bash
# smoke xdotool: clica numa face no editor de UV e verifica a seleção 3D
set -u
BIN=./target/debug/anigo-studio
LOG=/tmp/uvu_click.log
OUT=${1:-/tmp/uvu_click.png}
Xvfb :77 -screen 0 1600x1000x24 >/dev/null 2>&1 &
XPID=$!
sleep 1
export DISPLAY=:77
ANIGO_TRACE_UV=2 $BIN --no-session --workspace modelagem --scenario uv-click --frames 260 --size 1440x900 --screenshot "$OUT" >"$LOG" 2>&1 &
APID=$!
# espera a posição da face 2 no editor
for i in $(seq 1 60); do grep -q "uvtrace" "$LOG" && break; sleep 0.5; done
LINE=$(grep uvtrace "$LOG" | tail -1)
echo "antes: $LINE"
XY=$(echo "$LINE" | sed -E 's/.*face2_px=([0-9]+),([0-9]+).*/\1 \2/')
set -- $XY
xdotool mousemove "$1" "$2"
sleep 1
xdotool click 1
sleep 1.5
LINE2=$(grep uvtrace "$LOG" | tail -1)
echo "depois: $LINE2"
# arrasta a face selecionada 60 px para a direita (move UVs com undo)
xdotool mousedown 1
for d in 10 20 30 40 50 60; do xdotool mousemove_relative -- 10 0; sleep 0.2; done
xdotool mouseup 1
sleep 1.5
LINE3=$(grep uvtrace "$LOG" | tail -1)
echo "apos arrasto: $LINE3"
wait $APID
kill $XPID 2>/dev/null
if echo "$LINE2" | grep -q "sel_faces={2}"; then echo "OK: clique no editor de UV selecionou a face 2 no 3D"; else echo "FALHA"; exit 1; fi
