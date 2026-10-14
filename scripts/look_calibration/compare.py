#!/usr/bin/env python3
"""Compara o look de uma referência (quadro de anime) com o de um render do ANIGO, métrica a métrica.

    python scripts/look_calibration/compare.py referencia.png render.png [--mask-ref m.png] [--mask-render m.png] [--out pasta]

Imprime uma tabela (referência | render | diferença) e uma nota de 0 a 100 (100 = mesmo look). Cada linha diz também PARA ONDE mexer no
shader (ex.: "sombra mais dura: reduza shadow_smoothness"). A nota é só um guia para calibrar; olho humano decide.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze_frame import analyze  # noqa: E402


def get(d, *path, default=None):
    for p in path:
        if d is None:
            return default
        d = d.get(p) if isinstance(d, dict) else None
    return default if d is None else d


# (nome, caminho, peso, escala de "uma unidade de diferença", dica quando render > ref, dica quando render < ref)
METRICS = [
    ("faixas de tom", ("bands", "count"), 2.0, 1.0, "menos degraus: reduza toon_steps", "mais degraus: aumente toon_steps"),
    ("posterização", ("bands", "flatness"), 1.5, 0.15, "mais chapado que a referência: aumente shadow_smoothness", "mais liso que a referência: reduza shadow_smoothness/ao_intensity"),
    ("dureza da sombra (px)", ("terminator", "width_px"), 2.0, 3.0, "sombra mais suave que a referência: reduza shadow_smoothness", "sombra mais dura que a referência: aumente shadow_smoothness"),
    ("linha (rel. à altura)", ("outline", "rel"), 2.0, 0.002, "linha mais grossa: reduza outline_width", "linha mais fina: aumente outline_width"),
    ("cobertura da linha", ("outline", "coverage"), 1.0, 0.15, "linha em mais lugares: reduza outline_depth_bias", "falta linha na silhueta: aumente outline_opacity"),
    ("realces (área)", ("highlights", "area_share"), 1.0, 0.01, "realce maior: reduza specular_size/spec_intensity", "realce menor: aumente specular_size/spec_intensity"),
    ("rim (razão)", ("rim", "ratio"), 1.0, 0.1, "rim forte demais: reduza rim_intensity", "rim fraco: aumente rim_intensity"),
    ("sombra: ΔL", ("shadow_shift", "dL"), 1.5, 8.0, "sombra mais escura que a referência: clareie shade_color", "sombra mais clara que a referência: escureça shade_color"),
    ("sombra: Δmatiz (°)", ("shadow_shift", "dhue_deg"), 1.0, 12.0, "sombra gira para matiz maior: reduza hue_shift", "sombra gira para matiz menor: aumente hue_shift"),
    ("sombra: razão de croma", ("shadow_shift", "chroma_ratio"), 1.0, 0.2, "sombra mais saturada que a referência: dessature shade_color", "sombra menos saturada: sature shade_color"),
]


def palette_distance(a, b):
    """Distância média entre as cores dominantes (RGB 0..255), ponderada pela fatia de cada cor da referência."""
    def rgb(h):
        return tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))
    if not a or not b:
        return None
    total, acc = 0.0, 0.0
    for pa in a[:6]:
        ca = rgb(pa["hex"])
        best = min(math.dist(ca, rgb(pb["hex"])) for pb in b[:6])
        acc += best * pa["share"]
        total += pa["share"]
    return acc / total if total else None


def compare(ref: dict, cand: dict):
    rows, score_num, score_den = [], 0.0, 0.0
    for name, path, w, unit, hint_hi, hint_lo in METRICS:
        r, c = get(ref, *path), get(cand, *path)
        if r is None or c is None:
            rows.append((name, r, c, None, "sem dado"))
            continue
        diff = c - r
        err = min(1.0, abs(diff) / (3.0 * unit))  # 3 unidades de diferença = erro total
        score_num += w * err
        score_den += w
        hint = "" if abs(diff) < unit * 0.5 else (hint_hi if diff > 0 else hint_lo)
        rows.append((name, r, c, diff, hint))
    pd = palette_distance(ref.get("palette"), cand.get("palette"))
    if pd is not None:
        err = min(1.0, pd / 90.0)
        score_num += 3.0 * err
        score_den += 3.0
        hint = "" if pd < 25 else "paleta diferente: compare base_color/shade_color (veja a faixa de cores no relatório)"
        rows.append(("paleta (dist. RGB média)", 0.0, round(pd, 1), round(pd, 1), hint))
    score = 100.0 * (1.0 - score_num / score_den) if score_den else 0.0
    return rows, score


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reference")
    ap.add_argument("render")
    ap.add_argument("--mask-ref")
    ap.add_argument("--mask-render")
    ap.add_argument("--out")
    ap.add_argument("--fail-below", type=float, help="sai com código 1 se a nota ficar abaixo disto (uso em CI)")
    a = ap.parse_args(argv)
    ref = analyze(a.reference, a.mask_ref, a.out)
    cand = analyze(a.render, a.mask_render, a.out)
    rows, score = compare(ref, cand)
    fmt = lambda v: "-" if v is None else (f"{v:.3f}" if isinstance(v, float) else str(v))
    print(f"{'métrica':28s} {'referência':>12s} {'render':>12s} {'diferença':>10s}  ajuste")
    for name, r, c, d, hint in rows:
        print(f"{name:28s} {fmt(r):>12s} {fmt(c):>12s} {fmt(d):>10s}  {hint}")
    print(f"\nnota do look: {score:.0f}/100")
    if a.out:
        with open(os.path.join(a.out, "compare.json"), "w", encoding="utf-8") as f:
            json.dump({"score": round(score, 1), "reference": ref, "render": cand, "rows": [list(r) for r in rows]}, f, ensure_ascii=False, indent=2)
    if a.fail_below is not None and score < a.fail_below:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
