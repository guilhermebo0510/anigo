#!/usr/bin/env python3
"""Captura uma imagem do ANIGO para comparar com uma referência (`compare.py`).

Usa a bridge do app (`anigo-mcp bridge-call`), então o ANIGO precisa estar aberto com `--bridge`.

    python scripts/look_calibration/capture_app.py saida.png                # viewport do workspace atual
    python scripts/look_calibration/capture_app.py saida.png --workspace shading
    python scripts/look_calibration/capture_app.py saida.png --render 1920x1080   # render REAL (fila do Render), não captura de tela

Por padrão não troca o workspace e não grava na Biblioteca do usuário: o render vai para a pasta de saída informada.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time

EXE = ".exe" if os.name == "nt" else ""
MCP = os.environ.get("ANIGO_MCP_BIN", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "target", "debug", f"anigo-mcp{EXE}"))


def call(action, params=None, out_dir=None, save_png=None):
    env = dict(os.environ)
    if out_dir:
        env["ANIGO_MCP_ALLOWED_DIRS"] = out_dir
    cmd = [os.path.abspath(MCP), "bridge-call", action, json.dumps(params or {})]
    if save_png:
        cmd += ["--save-png", save_png]
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
    try:
        return json.loads(r.stdout.strip())
    except Exception:
        raise SystemExit(f"bridge-call {action} falhou: {(r.stderr or r.stdout).strip()[:300]}")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("--workspace")
    ap.add_argument("--render", metavar="LxA", help="render real nesse tamanho, ex.: 1920x1080 (precisa do workspace Render/Cenário com a câmera pronta)")
    a = ap.parse_args(argv)
    out = os.path.abspath(a.out)
    out_dir = os.path.dirname(out)
    os.makedirs(out_dir, exist_ok=True)
    try:
        alive = bool(call("GET_STATUS").get("engine"))
    except SystemExit:
        alive = False
    if not alive:
        raise SystemExit("o ANIGO não está respondendo: abra-o com `anigo-studio --bridge` (a bridge precisa estar ativa)")
    if a.workspace:
        call("SET_WORKSPACE", {"workspace": a.workspace})
        time.sleep(1.0)
    if a.render:
        w, h = (int(v) for v in a.render.lower().split("x"))
        call("APPLY_COMMAND", {"command": {"kind": "set_render_output", "patch": {"width": w, "height": h}}})
        job = call("RENDER_FRAME", {})
        t = time.time()
        while time.time() - t < 300:
            st = call("RENDER_STATUS", {"job_id": job["job_id"]})
            if st.get("status") in ("done", "failed", "cancelled"):
                break
            time.sleep(0.5)
        if st.get("status") != "done":
            raise SystemExit(f"render não terminou: {st.get('status')} {st.get('error')}")
        src = next(f["path"] for f in st["files"] if f["pass"] == "beauty")
        import shutil
        shutil.copyfile(src, out)
        print(f"render {w}x{h} -> {out}")
    else:
        call("SCREENSHOT", {"target": "viewport"}, out_dir=out_dir, save_png=out)
        print(f"viewport -> {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
