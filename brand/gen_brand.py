#!/usr/bin/env python3
"""Gerador da identidade visual do ANIGO (marca, ícone do app, splash, lockups).

Tudo sai de UMA geometria vetorial (shapely): os SVG são os caminhos exatos e os PNG/ICO são rasterizados dela com
super-amostragem — não há fonte tipográfica nem imagem de terceiros, então a marca é 100 % própria.

    python brand/gen_brand.py            # regrava brand/ (svg, png, ico) e as cópias usadas pelo app e pelo Tauri

Conceito: "A" sem travessa (Λ) cujo contra-forma abriga uma esfera — a bola de shading. O Λ é cortado ao meio como um
terminador de cel shading (lado claro / lado de sombra, duas cores chapadas) e a esfera leva a mesma luz. A letra "I" do
nome carrega o ponto de luz. Duas paletas: para fundos escuros e para fundos claros (o modo claro da interface).
"""
import json
import math
import os
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from shapely import affinity
from shapely.geometry import LineString, Point, Polygon, box
from shapely.ops import unary_union

ROOT = Path(__file__).resolve().parent
OUT = ROOT
SS = 4  # super-amostragem do rasterizador

# --------------------------------------------------------------------------- paletas
PAL = {
    "dark": {  # para fundo escuro
        "light": "#b9a8ff",      # lado iluminado do Λ
        "shade": "#6a50e0",      # lado de sombra do Λ
        "dot": "#ffd2a1",        # esfera: iluminada
        "dot_shade": "#f5976a",  # esfera: sombra
        "ink": "#f2f0fb",        # texto/wordmark
        "tile_a": "#2b2158",     # ícone do app: gradiente
        "tile_b": "#120e24",
        "bg": "#0f0c1b",
    },
    "light": {  # para fundo claro
        "light": "#8a6cff",
        "shade": "#4a34b5",
        "dot": "#ff9a2e",
        "dot_shade": "#d9601f",
        "ink": "#1c1633",
        "tile_a": "#ffffff",
        "tile_b": "#e9e4ff",
        "bg": "#f6f4ff",
    },
}


def hex_rgb(h, a=255):
    h = h.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


# --------------------------------------------------------------------------- geometria (grade de 1000 unidades, y para baixo)
def mark_geometry(bold=0.0):
    """Λ + esfera. `bold` engrossa para tamanhos pequenos (16–32 px)."""
    t = 190 + 40 * bold                 # espessura horizontal das pernas
    foot_l, foot_r = 150 - 18 * bold, 850 + 18 * bold
    top_y, base_y = 120 - 10 * bold, 880 + 8 * bold
    apex_l, apex_r = 430 - 8 * bold, 570 + 8 * bold
    # ponto interno do ápice: interseção das bordas internas
    dx, dy = apex_l - foot_l, top_y - base_y
    s = ((500 - (foot_l + t)) / dx)
    inner = (500, base_y + dy * s)
    lam = Polygon([(foot_l, base_y), (apex_l, top_y), (apex_r, top_y), (foot_r, base_y), (foot_r - t, base_y), inner, (foot_l + t, base_y)])
    dot_r = 74 + 14 * bold
    dot = Point(500, base_y - 135 - 4 * bold).buffer(dot_r, 64)
    return lam, dot


def cel_split(shape, x=500):
    left = shape.intersection(box(-1000, -1000, x, 2000))
    right = shape.intersection(box(x, -1000, 3000, 2000))
    return left, right


def dot_shading(dot):
    """Esfera em cel: lua crescente de sombra no lado inferior direito (mesma luz do Λ)."""
    c = dot.centroid
    r = math.sqrt(dot.area / math.pi)
    lit = dot.intersection(Point(c.x - r * 0.28, c.y - r * 0.28).buffer(r * 1.02, 64))
    shade = dot.difference(lit)
    return lit, shade


def stroke_path(points, w, cap=2, join=2):
    return LineString(points).buffer(w / 2, cap_style=cap, join_style=join, mitre_limit=8, quad_segs=32)


def arc_points(cx, cy, r, a0, a1, n=96):
    return [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)), cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]


def wordmark_geometry(cap=300.0, w=58.0, gap=70.0):
    """"ANIGO" desenhado em monolinha geométrica (cortes retos, sem serifa). Devolve (palavra, letras, largura)."""
    H = cap
    R = H / 2
    letters = {}
    x = 0.0
    # A: duas pernas com o topo reto e a travessa; construído como polígono (feet cortados na horizontal)
    aw = 340.0
    t = w * 1.16            # espessura horizontal da perna (≈ w na perpendicular)
    fw = w * 1.15           # largura do topo reto
    al, ar = (aw - fw) / 2, (aw + fw) / 2
    dxl, dyl = al - 0.0, 0.0 - H
    sline = ((aw / 2) - (0.0 + t)) / dxl
    inner = (aw / 2, H + dyl * sline)
    a_poly = Polygon([(0, H), (al, 0), (ar, 0), (aw, H), (aw - t, H), inner, (t, H)])
    bar_y = H * 0.68
    a_poly = unary_union([a_poly, box(aw * 0.18, bar_y - w / 2, aw * 0.82, bar_y + w / 2)]).intersection(box(0, 0, aw, H))
    letters["A"] = affinity.translate(a_poly, x, 0)
    x += aw + gap
    # N: duas hastes e a diagonal
    nw = 262.0
    dx = w * 1.45
    n_poly = unary_union([box(0, 0, w, H), box(nw - w, 0, nw, H), Polygon([(0, 0), (dx, 0), (nw, H), (nw - dx, H)])]).intersection(box(0, 0, nw, H))
    letters["N"] = affinity.translate(n_poly, x, 0)
    x += nw + gap
    # I
    letters["I"] = box(x, 0, x + w, H)
    x += w + gap
    # G: arco aberto em cima à direita + barra a meia altura, rente ao diâmetro externo
    cx, cy = x + R, R
    arc = arc_points(cx, cy, R - w / 2, -36, -360, 140)
    g = stroke_path(arc, w, cap=2, join=1)
    g = unary_union([g, box(cx + 6, cy - w / 2, cx + R, cy + w / 2), box(cx + R - w, cy - w / 2, cx + R, cy + w / 2)])
    g = g.intersection(Point(cx, cy).buffer(R, 128))  # a barra acompanha a curva externa (sem degrau)
    letters["G"] = g
    x += H + gap
    # O
    ring = Point(x + R, R).buffer(R, 96).difference(Point(x + R, R).buffer(R - w, 96))
    letters["O"] = ring
    x += H
    word = unary_union(list(letters.values()))
    return word, letters, x


# --------------------------------------------------------------------------- SVG
def poly_path(g):
    geoms = [g] if g.geom_type == "Polygon" else list(g.geoms)
    d = []
    for p in geoms:
        for ring in [p.exterior] + list(p.interiors):
            pts = list(ring.coords)
            d.append("M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in pts[:-1]) + " Z")
    return " ".join(d)


def svg_doc(w, h, body, view=None, bg=None):
    vb = view or (0, 0, w, h)
    rect = f'<rect x="{vb[0]}" y="{vb[1]}" width="{vb[2]}" height="{vb[3]}" fill="{bg}"/>' if bg else ""
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vb[0]} {vb[1]} {vb[2]} {vb[3]}">'
            f"{rect}{body}</svg>\n")


def mark_svg_body(pal, bold=0.0, mono=None, ox=0.0, oy=0.0, scale=1.0):
    lam, dot = mark_geometry(bold)
    transform = f' transform="translate({ox:.2f} {oy:.2f}) scale({scale:.5f})"'
    if mono:
        return f'<g{transform} fill="{mono}"><path fill-rule="evenodd" d="{poly_path(unary_union([lam, dot]))}"/></g>'
    left, right = cel_split(lam)
    lit, shade = dot_shading(dot)
    return (f'<g{transform}>'
            f'<path fill="{pal["light"]}" d="{poly_path(left)}"/>'
            f'<path fill="{pal["shade"]}" d="{poly_path(right)}"/>'
            f'<path fill="{pal["dot"]}" d="{poly_path(lit)}"/>'
            f'<path fill="{pal["dot_shade"]}" d="{poly_path(shade)}"/>'
            f"</g>")


# --------------------------------------------------------------------------- raster
class Canvas:
    def __init__(self, w, h, bg=None):
        self.w, self.h = w, h
        self.img = Image.new("RGBA", (w * SS, h * SS), bg or (0, 0, 0, 0))
        self.d = ImageDraw.Draw(self.img)

    def fill(self, geom, color, ox=0.0, oy=0.0, s=1.0):
        geoms = [geom] if geom.geom_type == "Polygon" else list(geom.geoms)
        for p in geoms:
            ext = [((x * s + ox) * SS, (y * s + oy) * SS) for x, y in p.exterior.coords]
            self.d.polygon(ext, fill=color)
            for r in p.interiors:
                self.d.polygon([((x * s + ox) * SS, (y * s + oy) * SS) for x, y in r.coords], fill=(0, 0, 0, 0))

    def fill_composite(self, geom, color, ox=0.0, oy=0.0, s=1.0):
        """Preenche respeitando buracos (anel do O) sem apagar o que já foi desenhado atrás."""
        layer = Image.new("L", self.img.size, 0)
        ld = ImageDraw.Draw(layer)
        geoms = [geom] if geom.geom_type == "Polygon" else list(geom.geoms)
        for p in geoms:
            ld.polygon([((x * s + ox) * SS, (y * s + oy) * SS) for x, y in p.exterior.coords], fill=255)
            for r in p.interiors:
                ld.polygon([((x * s + ox) * SS, (y * s + oy) * SS) for x, y in r.coords], fill=0)
        self.img.paste(Image.new("RGBA", self.img.size, color), (0, 0), layer)

    def result(self):
        return self.img.resize((self.w, self.h), Image.LANCZOS)


def draw_mark(cv, pal, size, ox, oy, bold=0.0, mono=None):
    s = size / 1000.0
    lam, dot = mark_geometry(bold)
    if mono:
        cv.fill_composite(unary_union([lam, dot]), hex_rgb(mono), ox, oy, s)
        return
    left, right = cel_split(lam)
    lit, shade = dot_shading(dot)
    cv.fill_composite(left, hex_rgb(pal["light"]), ox, oy, s)
    cv.fill_composite(right, hex_rgb(pal["shade"]), ox, oy, s)
    cv.fill_composite(lit, hex_rgb(pal["dot"]), ox, oy, s)
    cv.fill_composite(shade, hex_rgb(pal["dot_shade"]), ox, oy, s)


def squircle(n=5.0, size=1000.0, inset=0.0, steps=400):
    r = size / 2 - inset
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s_ = math.cos(t), math.sin(t)
        pts.append((size / 2 + r * math.copysign(abs(c) ** (2 / n), c), size / 2 + r * math.copysign(abs(s_) ** (2 / n), s_)))
    return Polygon(pts)


def tile_image(size, pal, bold=0.0, padding=0.0):
    """Ícone do app: tile arredondado com degradê vertical + marca cel."""
    cv = Canvas(size, size)
    shape = squircle(5.0, 1000.0, inset=1000 * padding)
    layer = Image.new("L", cv.img.size, 0)
    ImageDraw.Draw(layer).polygon([(x / 1000 * size * SS, y / 1000 * size * SS) for x, y in shape.exterior.coords], fill=255)
    a, b = np.array(hex_rgb(pal["tile_a"])[:3], float), np.array(hex_rgb(pal["tile_b"])[:3], float)
    grad = np.zeros((size * SS, size * SS, 4), np.uint8)
    t = np.linspace(0, 1, size * SS)[:, None]
    # degradê diagonal suave (de cima/esquerda para baixo/direita)
    xx = np.linspace(0, 1, size * SS)[None, :]
    k = np.clip(0.65 * t + 0.35 * xx, 0, 1)
    for c in range(3):
        grad[..., c] = (a[c] * (1 - k) + b[c] * k).astype(np.uint8)
    grad[..., 3] = 255
    cv.img.paste(Image.fromarray(grad, "RGBA"), (0, 0), layer)
    # marca centralizada (ocupa ~64 % do tile), levemente acima do centro óptico
    m = size * 0.64
    draw_mark(cv, pal, m, (size - m) / 2, (size - m) / 2 - size * 0.015, bold=bold)
    return cv.result()


def lockup_image(height, pal, mono=None, stacked=False, bg=None):
    word, letters, ww = wordmark_geometry()
    H = 300.0
    if not stacked:
        mark_h = 2.0 * H           # marca com o dobro da altura das capitulares
        gap = 0.55 * H
        total_w = mark_h + gap + ww
        total_h = mark_h
        scale = height / total_h
        w = int(math.ceil(total_w * scale))
        cv = Canvas(w, height, hex_rgb(bg) if bg else None)
        draw_mark(cv, pal, mark_h * scale, 0, 0, mono=mono)
        ox = (mark_h + gap) * scale
        oy = (mark_h - H) / 2 * scale + 0.0
        cv.fill_composite(word, hex_rgb(mono or pal["ink"]), ox, oy, scale)
        return cv.result()
    mark_h = 3.0 * H
    gap = 0.6 * H
    total_w = max(mark_h, ww)
    total_h = mark_h + gap + H
    scale = height / total_h
    w = int(math.ceil(total_w * scale))
    cv = Canvas(w, height, hex_rgb(bg) if bg else None)
    draw_mark(cv, pal, mark_h * scale, (total_w - mark_h) / 2 * scale, 0, mono=mono)
    cv.fill_composite(word, hex_rgb(mono or pal["ink"]), (total_w - ww) / 2 * scale, (mark_h + gap) * scale, scale)
    return cv.result()


def splash_image(w, h, pal):
    """Splash 16:9: fundo com a rampa de cel (faixas) + a marca e o nome; o app sobrepõe versão e progresso."""
    bg = hex_rgb(pal["bg"])
    cv = Canvas(w, h, bg)
    # faixas de luz diagonais, muito sutis, como as bandas de uma rampa toon
    glow = Image.new("RGBA", cv.img.size, (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    cx, cy = w * 0.5 * SS, h * 0.42 * SS
    for i, (rad, a) in enumerate([(0.70, 9), (0.50, 13), (0.31, 18)]):
        r = rad * h * SS * 1.25
        gd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=hex_rgb(pal["light"], a))
    cv.img = Image.alpha_composite(cv.img, glow)
    cv.d = ImageDraw.Draw(cv.img)
    mark = h * 0.42
    draw_mark(cv, pal, mark, (w - mark) / 2, h * 0.14)
    word, _, ww = wordmark_geometry()
    s = (h * 0.075) / 300.0
    cv.fill_composite(word, hex_rgb(pal["ink"]), (w - ww * s) / 2, h * 0.14 + mark + h * 0.05, s)
    return cv.result()


# --------------------------------------------------------------------------- ICO
def write_ico(path, images):
    """ICO com PNGs embutidos (Vista+). `images` = lista de PIL RGBA, qualquer tamanho <= 256."""
    blobs = []
    for im in images:
        import io
        b = io.BytesIO()
        im.save(b, "PNG")
        blobs.append((im.size[0], im.size[1], b.getvalue()))
    head = struct.pack("<HHH", 0, 1, len(blobs))
    off = 6 + 16 * len(blobs)
    entries, data = b"", b""
    for w, h, blob in blobs:
        entries += struct.pack("<BBBBHHII", 0 if w >= 256 else w, 0 if h >= 256 else h, 0, 0, 1, 32, len(blob), off + len(data))
        data += blob
    Path(path).write_bytes(head + entries + data)


# --------------------------------------------------------------------------- saída
def main():
    for sub in ("svg", "png", "ico", "splash"):
        (OUT / sub).mkdir(parents=True, exist_ok=True)
    word, letters, ww = wordmark_geometry()
    H = 300.0

    # ---- SVG
    for name, pal in PAL.items():
        (OUT / "svg" / f"mark_{name}.svg").write_text(svg_doc(512, 512, mark_svg_body(pal, scale=0.512), (0, 0, 512, 512)), encoding="utf-8")
        wm = f'<path fill="{pal["ink"]}" fill-rule="evenodd" d="{poly_path(word)}"/>'
        (OUT / "svg" / f"wordmark_{name}.svg").write_text(svg_doc(int(ww * 0.5), 150, f'<g transform="scale(0.5)">{wm}</g>', (0, 0, int(ww * 0.5), 150)), encoding="utf-8")
        mark_h = 2 * H
        gap = 0.55 * H
        total_w = mark_h + gap + ww
        body = mark_svg_body(pal, scale=mark_h / 1000.0) + f'<g transform="translate({mark_h + gap:.2f} {(mark_h - H) / 2:.2f})">{wm}</g>'
        (OUT / "svg" / f"lockup_{name}.svg").write_text(svg_doc(int(total_w / 2), int(mark_h / 2), body, (0, 0, round(total_w, 2), round(mark_h, 2))), encoding="utf-8")
    # mono
    lam, dot = mark_geometry()
    for nm, col in (("white", "#ffffff"), ("black", "#000000")):
        body = f'<path fill="{col}" fill-rule="evenodd" d="{poly_path(unary_union([lam, dot]))}"/>'
        (OUT / "svg" / f"mark_mono_{nm}.svg").write_text(svg_doc(512, 512, f'<g transform="scale(0.512)">{body}</g>', (0, 0, 512, 512)), encoding="utf-8")
    # ícone do app em SVG (tile + marca), versão escura
    pal = PAL["dark"]
    tile = squircle(5.0, 1000.0)
    defs = (f'<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{pal["tile_a"]}"/>'
            f'<stop offset="1" stop-color="{pal["tile_b"]}"/></linearGradient></defs>')
    body = f'{defs}<path fill="url(#g)" d="{poly_path(tile)}"/>' + mark_svg_body(pal, ox=180, oy=165, scale=0.64)
    (OUT / "svg" / "app_icon_dark.svg").write_text(svg_doc(1024, 1024, body, (0, 0, 1000, 1000)), encoding="utf-8")

    # ---- PNG: ícone do app em todos os tamanhos (escuro e claro); tamanhos pequenos usam a marca mais gorda
    sizes = [16, 20, 24, 32, 40, 48, 64, 72, 96, 128, 150, 192, 256, 310, 512, 1024]
    for variant in ("dark", "light"):
        pal = PAL[variant]
        for s in sizes:
            bold = 1.0 if s <= 24 else 0.6 if s <= 32 else 0.25 if s <= 48 else 0.0
            tile_image(s, pal, bold=bold, padding=0.0 if s >= 48 else 0.02).save(OUT / "png" / f"app_icon_{variant}_{s}.png")
    # ICO (Windows): 16,24,32,48,64,128,256 — escuro (é o que vai no exe e no atalho)
    ico = [Image.open(OUT / "png" / f"app_icon_dark_{s}.png").convert("RGBA") for s in (16, 24, 32, 48, 64, 128, 256)]
    write_ico(OUT / "ico" / "anigo.ico", ico)
    write_ico(OUT / "ico" / "anigo_light.ico", [Image.open(OUT / "png" / f"app_icon_light_{s}.png").convert("RGBA") for s in (16, 24, 32, 48, 64, 128, 256)])

    # ---- PNG: marcas soltas (fundo transparente), lockups e wordmark
    for variant in ("dark", "light"):
        pal = PAL[variant]
        for s in (64, 128, 256, 512, 1024):
            cv = Canvas(s, s)
            draw_mark(cv, pal, s, 0, 0, bold=0.3 if s <= 64 else 0.0)
            cv.result().save(OUT / "png" / f"mark_{variant}_{s}.png")
        for h in (48, 96, 192, 384):
            lockup_image(h, pal).save(OUT / "png" / f"lockup_{variant}_h{h}.png")
        lockup_image(512, pal, stacked=True).save(OUT / "png" / f"lockup_stacked_{variant}_h512.png")
    for nm, col in (("white", "#ffffff"), ("black", "#000000")):
        cv = Canvas(512, 512)
        draw_mark(cv, PAL["dark"], 512, 0, 0, mono=col)
        cv.result().save(OUT / "png" / f"mark_mono_{nm}_512.png")
        lockup_image(192, PAL["dark"], mono=col).save(OUT / "png" / f"lockup_mono_{nm}_h192.png")

    # ---- splash
    for variant in ("dark", "light"):
        splash_image(1920, 1080, PAL[variant]).convert("RGB").save(OUT / "splash" / f"splash_{variant}_1920x1080.png")
        splash_image(960, 540, PAL[variant]).convert("RGB").save(OUT / "splash" / f"splash_{variant}_960x540.png")

    # ---- cópias para o app nativo e para o Tauri legado
    app_assets = ROOT.parent / "crates" / "anigo-studio" / "assets" / "brand"
    app_assets.mkdir(parents=True, exist_ok=True)
    for nm, src in (("icon_256.png", "app_icon_dark_256.png"), ("icon_64.png", "app_icon_dark_64.png"),
                    ("mark_dark_256.png", "mark_dark_256.png"), ("mark_light_256.png", "mark_light_256.png"),
                    ("lockup_dark_h48.png", "lockup_dark_h48.png"), ("lockup_light_h48.png", "lockup_light_h48.png"),
                    ("lockup_dark_h96.png", "lockup_dark_h96.png"), ("lockup_light_h96.png", "lockup_light_h96.png"),
                    ("lockup_dark_h192.png", "lockup_dark_h192.png"), ("lockup_light_h192.png", "lockup_light_h192.png")):
        (app_assets / nm).write_bytes((OUT / "png" / src).read_bytes())
    (app_assets / "splash_dark.png").write_bytes((OUT / "splash" / "splash_dark_1920x1080.png").read_bytes())
    (app_assets / "splash_light.png").write_bytes((OUT / "splash" / "splash_light_1920x1080.png").read_bytes())
    (app_assets / "anigo.ico").write_bytes((OUT / "ico" / "anigo.ico").read_bytes())
    tauri = ROOT.parent / "src-tauri" / "icons"
    if tauri.exists():
        for nm, s in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
            (tauri / nm).write_bytes((OUT / "png" / f"app_icon_dark_{s}.png").read_bytes())
        (tauri / "icon.ico").write_bytes((OUT / "ico" / "anigo.ico").read_bytes())

    # ---- folha de contato (para revisão humana)
    sheet = Image.new("RGBA", (1800, 1000), hex_rgb("#1b1830"))
    x = 40
    for s in (16, 24, 32, 48, 64, 128, 256):
        im = Image.open(OUT / "png" / f"app_icon_dark_{s}.png")
        sheet.paste(im, (x, 40), im)
        x += s + 30
    x = 40
    for s in (16, 24, 32, 48, 64, 128, 256):
        im = Image.open(OUT / "png" / f"app_icon_light_{s}.png")
        sheet.paste(im, (x, 330), im)
        x += s + 30
    ld = Image.open(OUT / "png" / "lockup_dark_h192.png")
    sheet.paste(ld, (40, 640), ld)
    lightbg = Image.new("RGBA", (880, 260), hex_rgb(PAL["light"]["bg"]))
    ll = Image.open(OUT / "png" / "lockup_light_h192.png")
    lightbg.paste(ll, (30, 30), ll)
    sheet.paste(lightbg, (900, 640))
    sheet.convert("RGB").save(OUT / "contact_sheet.png")
    print("marca gerada em", OUT)


if __name__ == "__main__":
    main()
