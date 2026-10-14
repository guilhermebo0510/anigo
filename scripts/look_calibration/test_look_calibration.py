"""Testes do analisador de look com esferas cel sintéticas de parâmetros CONHECIDOS (sombra dura/suave, linha de 4 px, realce).
    python -m unittest scripts/look_calibration/test_look_calibration.py -v
"""
import os
import sys
import tempfile
import unittest

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze_frame import analyze  # noqa: E402
from compare import compare  # noqa: E402

LIT = np.array([250, 226, 205], np.float32)
SHADE = np.array([196, 160, 186], np.float32)
LINE = np.array([40, 28, 52], np.float32)
BG = np.array([205, 215, 235], np.float32)


def sphere(size=420, radius=150, soft_px=0.0, line_px=4, highlight=True, bands=2):
    """Esfera iluminada da esquerda-cima. `soft_px` = largura da transição entre luz e sombra (0 = dura)."""
    yy, xx = np.mgrid[0:size, 0:size].astype(np.float32)
    cx = cy = size / 2
    dx, dy = xx - cx, yy - cy
    r = np.hypot(dx, dy)
    inside = r <= radius
    nz = np.sqrt(np.clip(1 - (r / radius) ** 2, 0, 1))
    ndl = (-0.55 * dx / radius) + (0.55 * dy / radius * -1) + 0.62 * nz  # luz vindo da esquerda/cima/frente
    img = np.ones((size, size, 3), np.float32) * BG
    # distância (em px) até o terminador, positiva do lado iluminado
    thr = 0.18
    dist_px = (ndl - thr) * radius * 1.6
    if soft_px <= 0:
        t = (dist_px > 0).astype(np.float32)
    else:
        t = np.clip(dist_px / soft_px + 0.5, 0, 1)
        t = t * t * (3 - 2 * t)
    if bands == 3:
        t = np.where(dist_px > 0, np.where(dist_px > 60, 1.0, 0.62), 0.0).astype(np.float32)
    col = SHADE[None, None, :] * (1 - t[..., None]) + LIT[None, None, :] * t[..., None]
    img = np.where(inside[..., None], col, img)
    if line_px > 0:
        ring = (r > radius - line_px) & (r <= radius)
        img[ring] = LINE
    if highlight:
        hx, hy = cx - 0.45 * radius, cy - 0.45 * radius
        hl = ((xx - hx) / 12.0) ** 2 + ((yy - hy) / 7.0) ** 2 <= 1
        img[hl & inside] = (255, 255, 255)
    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))


def run(img):
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "s.png")
        img.save(p)
        return analyze(p)


class LookTests(unittest.TestCase):
    def test_hard_cel_has_two_flat_bands_a_sharp_terminator_and_the_drawn_outline(self):
        r = run(sphere(soft_px=0, line_px=4))
        self.assertEqual(r["bands"]["count"], 2, r["bands"])
        self.assertGreater(r["bands"]["flatness"], 0.8)
        self.assertIsNotNone(r["terminator"]["width_px"])
        self.assertLess(r["terminator"]["width_px"], 3.0, r["terminator"])
        self.assertTrue(2.0 <= r["outline"]["px"] <= 7.0, r["outline"])
        self.assertGreater(r["outline"]["coverage"], 0.8)
        self.assertGreaterEqual(r["highlights"]["count"], 1)

    def test_soft_shading_is_measured_as_a_wider_terminator(self):
        hard = run(sphere(soft_px=0))["terminator"]["width_px"]
        soft = run(sphere(soft_px=40))["terminator"]["width_px"]
        self.assertIsNotNone(hard)
        self.assertIsNotNone(soft)
        self.assertGreater(soft, hard * 3, (hard, soft))

    def test_three_tone_cel_counts_three_bands(self):
        r = run(sphere(bands=3))
        self.assertGreaterEqual(r["bands"]["count"], 3, r["bands"])

    def test_outline_thickness_tracks_the_drawn_width(self):
        thin = run(sphere(line_px=2))["outline"]["px"]
        thick = run(sphere(line_px=8))["outline"]["px"]
        self.assertGreater(thick, thin * 2, (thin, thick))

    def test_palette_finds_the_lit_color_and_the_shadow_hue_shifts(self):
        r = run(sphere(soft_px=0, highlight=False))
        top = r["palette"][0]["hex"]
        rgb = np.array([int(top[i:i + 2], 16) for i in (1, 3, 5)])
        self.assertLess(np.linalg.norm(rgb - LIT), 40, top)
        self.assertGreater(r["shadow_shift"]["dL"], 8.0)
        self.assertIsNotNone(r["shadow_shift"]["dhue_deg"])

    def test_identical_images_score_100_and_different_looks_score_less(self):
        a = run(sphere(soft_px=0))
        same, score = compare(a, a)
        self.assertGreaterEqual(score, 99.0)
        _, worse = compare(a, run(sphere(soft_px=60, line_px=0, highlight=False)))
        self.assertLess(worse, score - 15, (score, worse))


if __name__ == "__main__":
    unittest.main()
