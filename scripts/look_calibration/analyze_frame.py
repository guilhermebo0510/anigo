#!/usr/bin/env python3
"""Mede o "look" de uma imagem cel/anime: paleta, faixas de tom, dureza da sombra, linha de contorno, realces e rim.

Serve para calibrar o shader do ANIGO contra quadros de referência: roda o MESMO analisador na referência e no render do app
(`compare.py`) e compara número a número. Não é parte do produto (só pesquisa/QA), por isso pode usar numpy/scipy/scikit-image.

    python scripts/look_calibration/analyze_frame.py quadro.png [--out pasta] [--mask mascara.png]

Máscara do personagem: usa o canal alfa, se houver; senão a cor de fundo é estimada nas bordas. Passe `--mask` (branco = personagem)
quando o fundo for complexo.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import sys

import numpy as np
from PIL import Image
from scipy import ndimage as ndi
from scipy.signal import find_peaks
from skimage import color, morphology
from sklearn.cluster import KMeans

def rm_small(arr, n):
    """remove_small_objects que aceita scikit-image novo (`max_size`) e antigo (`min_size`); remove o que tem MENOS de n px."""
    try:
        return morphology.remove_small_objects(arr, max_size=max(0, int(n) - 1))
    except TypeError:
        return morphology.remove_small_objects(arr, int(n))


def fill_small_holes(arr, n):
    try:
        return morphology.remove_small_holes(arr, max_size=max(0, int(n) - 1))
    except TypeError:
        return morphology.remove_small_holes(arr, int(n))


MAX_SIDE = 1400  # reduz imagens enormes (4K) antes de medir: as métricas são relativas à altura do personagem


# ----------------------------------------------------------------------------------------------------------- entrada

def load(path: str, mask_path: str | None = None):
    im = Image.open(path)
    has_alpha = im.mode in ("RGBA", "LA") or "transparency" in im.info
    rgba = im.convert("RGBA")
    if max(rgba.size) > MAX_SIDE:
        s = MAX_SIDE / max(rgba.size)
        rgba = rgba.resize((max(1, round(rgba.size[0] * s)), max(1, round(rgba.size[1] * s))), Image.LANCZOS)
    arr = np.asarray(rgba).astype(np.float32) / 255.0
    rgb, alpha = arr[..., :3], arr[..., 3]
    if mask_path:
        m = Image.open(mask_path).convert("L").resize(rgba.size, Image.NEAREST)
        mask = np.asarray(m) > 127
    elif has_alpha and (alpha < 0.99).any():
        mask = alpha > 0.5
    else:
        mask = estimate_mask(rgb)
    return rgb, mask


def estimate_mask(rgb: np.ndarray) -> np.ndarray:
    """Fundo = cor mediana da moldura de 2 % da imagem; personagem = o que se afasta dela, sem buracos nem poeira."""
    h, w, _ = rgb.shape
    b = max(2, int(0.02 * min(h, w)))
    border = np.concatenate([rgb[:b].reshape(-1, 3), rgb[-b:].reshape(-1, 3), rgb[:, :b].reshape(-1, 3), rgb[:, -b:].reshape(-1, 3)])
    bg = np.median(border, axis=0)
    dist = np.linalg.norm(rgb - bg, axis=2)
    mask = dist > 0.06
    mask = rm_small(mask, max(32, int(0.0005 * h * w)))
    mask = ndi.binary_closing(mask, iterations=2)
    mask = ndi.binary_fill_holes(mask)
    return fill_small_holes(mask, max(64, int(0.002 * h * w)))


# ----------------------------------------------------------------------------------------------------------- métricas

def hex_of(lab: np.ndarray) -> str:
    rgb = np.clip(color.lab2rgb(lab.reshape(1, 1, 3)).reshape(3), 0, 1)
    return "#%02x%02x%02x" % tuple(int(round(v * 255)) for v in rgb)


def palette(lab_fg: np.ndarray, k: int = 8, seed: int = 0):
    n = len(lab_fg)
    if n == 0:
        return []
    rng = np.random.default_rng(seed)
    sample = lab_fg if n <= 30000 else lab_fg[rng.choice(n, 30000, replace=False)]
    k = max(1, min(k, len(np.unique(sample.round(0), axis=0))))
    km = KMeans(n_clusters=k, n_init=4, random_state=seed).fit(sample)
    labels = km.predict(lab_fg)
    out = []
    for i in range(k):
        share = float((labels == i).mean())
        c = km.cluster_centers_[i]
        out.append({"hex": hex_of(c), "share": round(share, 4), "L": round(float(c[0]), 1), "chroma": round(float(math.hypot(c[1], c[2])), 1)})
    return sorted(out, key=lambda d: -d["share"])


def tone_bands(L_fg: np.ndarray):
    """Conta faixas de tom: picos do histograma de luminância e quão "chapado" (posterizado) é o resultado."""
    hist, edges = np.histogram(L_fg, bins=64, range=(0, 100))
    p = hist / max(1, hist.sum())
    ps = ndi.gaussian_filter1d(p, 1.0)
    peaks, props = find_peaks(ps, height=0.012, prominence=0.006, distance=2)
    centers = [float((edges[i] + edges[i + 1]) / 2) for i in peaks]
    shares = []
    for c in centers:
        shares.append(float(((L_fg > c - 4) & (L_fg < c + 4)).mean()))
    n = max(1, len(centers))
    if len(centers) >= 1:
        cs = np.array(centers)
        d = np.abs(L_fg[:, None] - cs[None, :]).min(axis=1)
        flat = float(np.clip(1.0 - d.mean() / 12.0, 0.0, 1.0))
    else:
        flat = 0.0
    return {"count": int(n), "centers_L": [round(c, 1) for c in centers], "shares": [round(s, 3) for s in shares], "flatness": round(flat, 3)}


def terminator(L: np.ndarray, inner: np.ndarray, band_centers, height_px: int):
    """Largura (px) da transição entre faixas de tom. A zona "entre platôs" (pixels que não estão em nenhuma faixa) é a transição;
    largura = área da transição / comprimento da borda (esqueleto). ~1 px = sombra dura (só o antisserrilhado); dezenas = suave."""
    if len(band_centers) < 2:
        return {"width_px": None, "width_rel": None, "length_rel": 0.0}
    cs = np.array(sorted(band_centers))
    tol = max(2.5, 0.18 * float(np.min(np.diff(cs))))
    dist = np.abs(L[..., None] - cs[None, None, :]).min(axis=2)
    trans = inner & (dist > tol) & (L > cs[0] + tol) & (L < cs[-1] - tol)
    trans = rm_small(trans, 6)
    lab, n = ndi.label(trans, structure=np.ones((3, 3)))
    if n == 0:
        return {"width_px": 0.0, "width_rel": 0.0, "length_rel": 0.0}
    skel = morphology.skeletonize(trans)
    keep = np.zeros_like(trans)
    for i in range(1, n + 1):
        comp = lab == i
        if int((skel & comp).sum()) >= max(8, 0.12 * height_px):
            keep |= comp
    if not keep.any():
        return {"width_px": 0.0, "width_rel": 0.0, "length_rel": 0.0}
    length = float((skel & keep).sum())
    width = float(keep.sum()) / max(1.0, length)
    return {"width_px": round(width, 2), "width_rel": round(width / max(1, height_px), 5), "length_rel": round(length / max(1, height_px), 3)}


def outline(rgb: np.ndarray, L: np.ndarray, fg: np.ndarray, height_px: int):
    """Linha de contorno = pixels escuros na faixa junto da silhueta. Espessura = 2 × mediana da distância no esqueleto."""
    boundary_band = ndi.binary_dilation(fg, iterations=max(3, int(0.012 * height_px))) & ~ndi.binary_erosion(fg, iterations=max(4, int(0.02 * height_px)))
    if not boundary_band.any():
        return {"px": None, "rel": None, "coverage": 0.0, "hex": None}
    Lf = L[fg]
    dark_thr = min(float(np.percentile(Lf, 12)) + 6.0, 35.0)
    dark = (L < dark_thr) & boundary_band
    dark = rm_small(dark, 8)
    if dark.sum() < 12:
        return {"px": 0.0, "rel": 0.0, "coverage": 0.0, "hex": None}
    dt = ndi.distance_transform_edt(dark)
    skel = morphology.skeletonize(dark)
    thick = float(2.0 * np.median(dt[skel])) if skel.any() else 0.0
    sil = fg & ~ndi.binary_erosion(fg)
    near_dark = ndi.binary_dilation(dark, iterations=3)
    cover = float((sil & near_dark).sum()) / max(1, int(sil.sum()))
    c = np.median(rgb[dark], axis=0)
    return {"px": round(thick, 2), "rel": round(thick / max(1, height_px), 5), "coverage": round(cover, 3), "hex": "#%02x%02x%02x" % tuple(int(round(v * 255)) for v in c)}


def highlights(L: np.ndarray, fg: np.ndarray, height_px: int):
    """Realces: manchas claras menores que ~8 % da altura (cabelo, olhos, brilho especular). Top-hat = L - abertura(L): só sobra o
    que é claro E pequeno; a faixa de luz grande do corpo (maior que a janela) não conta."""
    w = max(5, int(0.08 * height_px) | 1)
    top = L - ndi.grey_opening(L, size=(w, w))
    inner = ndi.binary_erosion(fg, iterations=max(3, int(0.02 * height_px)))
    cand = inner & (top > 8.0) & (L > float(np.percentile(L[inner], 80)))
    cand = rm_small(cand, 6)
    lab, n = ndi.label(cand)
    if n == 0:
        return {"count": 0, "area_share": 0.0, "median_area_rel": 0.0}
    areas = ndi.sum(cand, lab, range(1, n + 1))
    return {"count": int(n), "area_share": round(float(cand.sum()) / max(1, int(fg.sum())), 5), "median_area_rel": round(float(np.median(areas)) / max(1, height_px ** 2), 7)}


def rim(L: np.ndarray, fg: np.ndarray, height_px: int):
    """Rim light = o anel interno junto da silhueta é mais claro que o miolo?"""
    k = max(3, int(0.03 * height_px))
    inner = ndi.binary_erosion(fg, iterations=k)
    ring = fg & ~ndi.binary_erosion(fg, iterations=max(2, k // 3)) & ~inner & ndi.binary_dilation(~fg, iterations=k)
    ring = fg & ~inner & ~(fg & ~ndi.binary_erosion(fg, iterations=2))
    if ring.sum() < 20 or inner.sum() < 50:
        return {"ratio": None}
    return {"ratio": round(float(np.percentile(L[ring], 90)) / max(1.0, float(np.percentile(L[inner], 90))), 3)}


def shadow_shift(lab_fg: np.ndarray, bands):
    """Da faixa clara para a escura: ΔL, mudança de matiz (graus) e razão de croma — a "cor de sombra" que o ilustrador escolhe."""
    cs = bands["centers_L"]
    if len(cs) < 2:
        return {"dL": None, "dhue_deg": None, "chroma_ratio": None}
    lit, sh = max(cs), min(cs)
    a = lab_fg[np.abs(lab_fg[:, 0] - lit) < 5]
    b = lab_fg[np.abs(lab_fg[:, 0] - sh) < 5]
    if len(a) < 20 or len(b) < 20:
        return {"dL": None, "dhue_deg": None, "chroma_ratio": None}
    ma, mb = a.mean(axis=0), b.mean(axis=0)
    ha, hb = math.degrees(math.atan2(ma[2], ma[1])), math.degrees(math.atan2(mb[2], mb[1]))
    dh = (hb - ha + 180) % 360 - 180
    return {"dL": round(float(ma[0] - mb[0]), 1), "dhue_deg": round(dh, 1), "chroma_ratio": round(float(math.hypot(mb[1], mb[2]) / max(1e-3, math.hypot(ma[1], ma[2]))), 3)}


def analyze(path: str, mask_path: str | None = None, out_dir: str | None = None) -> dict:
    rgb, fg = load(path, mask_path)
    ys, xs = np.nonzero(fg)
    if len(ys) < 200:
        raise SystemExit(f"{path}: personagem não encontrado (máscara com {len(ys)} px); passe --mask")
    height_px = int(ys.max() - ys.min() + 1)
    lab = color.rgb2lab(rgb)
    L = lab[..., 0]
    lab_fg = lab[fg]
    # tom e terminador só no INTERIOR: a linha escura do contorno não é uma "faixa de tom" do sombreamento
    inner = ndi.binary_erosion(fg, iterations=max(4, int(0.02 * height_px)))
    if inner.sum() < 100:
        inner = fg
    bands = tone_bands(L[inner])
    result = {
        "file": os.path.basename(path),
        "size": [int(rgb.shape[1]), int(rgb.shape[0])],
        "character_height_px": height_px,
        "coverage": round(float(fg.mean()), 4),
        "palette": palette(lab_fg),
        "bands": bands,
        "terminator": terminator(L, inner, bands["centers_L"], height_px),
        "outline": outline(rgb, L, fg, height_px),
        "highlights": highlights(L, fg, height_px),
        "rim": rim(L, fg, height_px),
        "shadow_shift": shadow_shift(lab[inner], bands),
    }
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
        stem = os.path.splitext(os.path.basename(path))[0]
        with open(os.path.join(out_dir, stem + ".look.json"), "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)
        report_png(rgb, fg, result, os.path.join(out_dir, stem + ".look.png"))
    return result


def report_png(rgb, fg, res, path):
    """Imagem de conferência: original | máscara usada | mapa de faixas | paleta."""
    h, w, _ = rgb.shape
    scale = 420.0 / max(h, 1)
    tw, th = max(1, int(w * scale)), 420
    tile = lambda a: Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8)).resize((tw, th), Image.LANCZOS)
    orig = tile(rgb)
    maskv = tile(np.repeat(fg[..., None].astype(np.float32), 3, axis=2))
    L = color.rgb2lab(rgb)[..., 0]
    cs = np.array(res["bands"]["centers_L"] or [50.0])
    idx = np.abs(L[..., None] - cs[None, None, :]).argmin(axis=2)
    gray = (cs[idx] / 100.0)
    bandmap = tile(np.repeat(np.where(fg, gray, 0.0)[..., None], 3, axis=2))
    sheet = Image.new("RGB", (tw * 3 + 140, th), (24, 22, 36))
    for i, t in enumerate((orig, maskv, bandmap)):
        sheet.paste(t, (i * tw, 0))
    from PIL import ImageDraw
    d = ImageDraw.Draw(sheet)
    y = 8
    for p in res["palette"][:8]:
        c = tuple(int(p["hex"][i:i + 2], 16) for i in (1, 3, 5))
        d.rectangle([tw * 3 + 10, y, tw * 3 + 130, y + 36], fill=c)
        d.text((tw * 3 + 14, y + 40), f"{p['share'] * 100:.0f}%  L{p['L']:.0f}", fill=(230, 230, 240))
        y += 52
    sheet.save(path)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("image")
    ap.add_argument("--mask")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    res = analyze(a.image, a.mask, a.out)
    print(json.dumps(res, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
