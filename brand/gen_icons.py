#!/usr/bin/env python3
"""Fonte de ícones própria do ANIGO ("ANIGO Icons") — a iconografia dos workspaces e das ferramentas.

Linguagem visual (uma só para o conjunto): traço de 1,8 px em grade de 24, pontas e cantos arredondados e **um trecho sólido em
cada ícone**, sempre do lado de baixo/direita — o "lado de sombra" do cel shading, o mesmo corte da marca. Assim o conjunto se
reconhece como ANIGO e cada ícone ganha uma silhueta própria (nada de dois círculos iguais para ferramentas diferentes).

    python brand/gen_icons.py     # gera crates/anigo-studio/assets/fonts/AnigoIcons.ttf, src/icons_anigo.rs e brand/icons_sheet.png

Só geometria própria (shapely) → contornos TrueType; nenhum ícone de terceiros.
"""
import math
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from PIL import Image, ImageDraw
from shapely import affinity
from shapely.geometry import LineString, Point, Polygon, box
from shapely.geometry.polygon import orient
from shapely.ops import unary_union

ROOT = Path(__file__).resolve().parent
STUDIO = ROOT.parent / "crates" / "anigo-studio"
SW = 1.8          # espessura do traço (grade de 24)
Q = 24            # segmentos por quarto de círculo


# ------------------------------------------------------------------------------------------------ primitivas (grade de 24)
def line(*pts, w=SW):
    """Traço com pontas arredondadas; aceita pontos (x, y) ou números soltos (x0, y0, x1, y1, ...)."""
    if pts and not isinstance(pts[0], (tuple, list)):
        pts = list(zip(pts[0::2], pts[1::2]))
    return LineString(pts).buffer(w / 2, cap_style=1, join_style=1, quad_segs=Q)


def disc(cx, cy, r):
    return Point(cx, cy).buffer(r, quad_segs=Q)


def ring(cx, cy, r, w=SW):
    return Point(cx, cy).buffer(r + w / 2, quad_segs=Q).difference(Point(cx, cy).buffer(max(r - w / 2, 0.01), quad_segs=Q))


def arc(cx, cy, r, a0, a1, w=SW, n=48):
    pts = [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)), cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]
    return line(*pts, w=w)


def poly(*pts):
    return Polygon(pts)


def outline(*pts, w=SW, closed=True):
    pts = list(pts) + ([pts[0]] if closed else [])
    return line(*pts, w=w)


def rrect_fill(x, y, w, h, r):
    r = min(r, w / 2 - 0.01, h / 2 - 0.01)
    return box(x + r, y + r, x + w - r, y + h - r).buffer(r, quad_segs=Q)


def rrect(x, y, w, h, r, sw=SW):
    return rrect_fill(x - sw / 2, y - sw / 2, w + sw, h + sw, r + sw / 2).difference(rrect_fill(x + sw / 2, y + sw / 2, w - sw, h - sw, max(r - sw / 2, 0.05)))


def U(*g):
    return unary_union([x for x in g if x is not None])


def right_half(g, x=12.0):
    return g.intersection(box(x, -50, 100, 100))


def arrow_head(tip, direction, size=3.4, spread=0.55):
    d = math.atan2(direction[1], direction[0])
    a = (tip[0] - size * math.cos(d - spread), tip[1] - size * math.sin(d - spread))
    b = (tip[0] - size * math.cos(d + spread), tip[1] - size * math.sin(d + spread))
    return Polygon([tip, a, b]).buffer(0.25, join_style=1)


def crescent(cx, cy, r, dx, dy, r2=None):
    """Lua crescente de sombra: o disco menos o disco deslocado (luz vindo de cima/esquerda)."""
    return disc(cx, cy, r).difference(disc(cx + dx, cy + dy, r2 or r))


# ------------------------------------------------------------------------------------------------ os ícones
ICONS = {}


def icon(name, code):
    def deco(fn):
        ICONS[name] = (code, fn)
        return fn
    return deco


C = 0xF100   # ponto de código inicial (Phosphor ocupa E000–EE82)

# ---- workspaces ---------------------------------------------------------------------------------
@icon("ws_personagem", C + 0)
def _():
    bust = Polygon([(12 + 7.6 * math.cos(math.radians(a)), 21.2 - 7.6 * math.sin(math.radians(a))) for a in range(0, 181, 4)])
    return U(ring(12, 8, 3.6), outline(*bust.exterior.coords[:-1], w=SW), right_half(bust, 12.0).buffer(-0.2))

@icon("ws_modelagem", C + 1)
def _():
    top, ur, lr, bot, ll, ul, ctr = (12, 2.9), (19.9, 7.4), (19.9, 16.6), (12, 21.1), (4.1, 16.6), (4.1, 7.4), (12, 12)
    return U(outline(top, ur, lr, bot, ll, ul), line(ctr, ul), line(ctr, ur), line(ctr, bot), poly(ur, lr, bot, ctr).buffer(-0.25))

@icon("ws_posing", C + 2)
def _():
    return U(ring(12, 4.6, 2.3), line(12, 7.4, 12, 14.2), line(5.8, 11.6, 12, 8.8, 18.2, 11.6), line(12, 14.2, 7.6, 21), line(12, 14.2, 16.4, 21),
             disc(5.8, 11.6, 1.5), disc(18.2, 11.6, 1.5), disc(12, 14.2, 1.7), disc(7.6, 21, 1.5), disc(16.4, 21, 1.5))

@icon("ws_shading", C + 3)
def _():
    return U(ring(12, 12, 8.7), crescent(12, 12, 7.6, -3.1, -3.1, 7.9))

@icon("ws_iluminacao", C + 4)
def _():
    rays = [line(12 + 7.2 * math.cos(math.radians(a)), 12 + 7.2 * math.sin(math.radians(a)), 12 + 10.2 * math.cos(math.radians(a)), 12 + 10.2 * math.sin(math.radians(a))) for a in range(0, 360, 45)]
    return U(ring(12, 12, 4.2), right_half(disc(12, 12, 4.0), 12.0), *rays)

@icon("ws_cenario", C + 5)
def _():
    shape = [(2.6, 19.6), (9, 8.2), (12.6, 14.4), (16, 9.4), (21.4, 19.6)]
    base = outline(*shape, w=SW)
    shade = U(poly((9, 8.2), (12.6, 14.4), (12.6, 19.6), (9, 19.6)), poly((16, 9.4), (21.4, 19.6), (16, 19.6)))
    return U(base, shade.buffer(-0.2))

@icon("ws_animacao", C + 6)
def _():
    def dia(cx, cy=14.6, r=3.1):
        return poly((cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy))
    marker = poly((8.9, 2.8), (15.1, 2.8), (15.1, 6.0), (12, 8.8), (8.9, 6.0)).buffer(0.25, join_style=1)
    return U(marker, line(12, 8.8, 12, 20.6, w=1.5), outline(*dia(4.9).exterior.coords[:-1]), outline(*dia(19.1).exterior.coords[:-1]), dia(12, r=3.8).buffer(0.3, join_style=1))

@icon("ws_render", C + 7)
def _():
    body = rrect(3, 7.4, 18, 12, 2.6)
    bump = outline((7.6, 7.4), (9.2, 4.4), (14.8, 4.4), (16.4, 7.4), closed=False)
    return U(body, bump, ring(12, 13.4, 3.9), disc(12, 13.4, 1.6), right_half(disc(12, 13.4, 3.4), 12.0))

@icon("ws_biblioteca", C + 8)
def _():
    book1 = rrect(3.8, 4.2, 4.6, 16, 1.4)
    book2 = rrect(9.6, 4.2, 4.6, 16, 1.4)
    book3 = affinity.rotate(rrect_fill(15.6, 4.8, 4.8, 15.6, 1.6), -13, origin=(18, 20))
    return U(book1, book2, book3, rrect_fill(9.9, 11, 4.0, 8.9, 1.0))

# ---- Personagem ---------------------------------------------------------------------------------
@icon("t_body", C + 10)
def _():
    torso = rrect(8.4, 8.2, 7.2, 7.4, 2.4)
    return U(ring(12, 4.4, 2.3), torso, rrect_fill(12, 8.8, 3.2, 6.2, 1.2), line(8.4, 9.6, 5, 14.6), line(15.6, 9.6, 19, 14.6), line(10, 15.6, 9.2, 21.4), line(14, 15.6, 14.8, 21.4))

@icon("t_face", C + 11)
def _():
    return U(ring(12, 12, 8.7), disc(8.8, 10.4, 1.25), disc(15.2, 10.4, 1.25), arc(12, 11.4, 4.3, 30, 150, w=1.6), crescent(12, 12, 8.0, -1.9, 0.0, 8.0).intersection(box(12.5, 3, 30, 22)))

@icon("t_hair", C + 12)
def _():
    top = [(12 + 7.4 * math.cos(math.radians(a)), 12.4 - 7.4 * math.sin(math.radians(a))) for a in range(180, -1, -6)]
    hem = [(19.4, 20.4), (17.0, 18.0), (14.6, 20.4), (12.0, 18.0), (9.4, 20.4), (7.0, 18.0), (4.6, 20.4)]
    shape = [(4.6, 20.4), (4.6, 12.4)] + top + [(19.4, 20.4)] + hem[1:-1]
    wig = Polygon(shape)
    return U(outline(*shape, w=SW), line(12, 5.0, 9.4, 12.2, 10.6, 17.6, w=1.5), right_half(wig, 13.0).buffer(-0.15))

@icon("t_cloth", C + 13)
def _():
    shirt = [(8.2, 3.6), (3.6, 6.2), (2.8, 10.6), (6.2, 11.4), (6.2, 20.4), (17.8, 20.4), (17.8, 11.4), (21.2, 10.6), (20.4, 6.2), (15.8, 3.6)]
    return U(outline(*shirt), arc(12, 3.6, 3.7, 0, 180, w=SW), right_half(Polygon(shirt), 12.2).buffer(-0.2))

@icon("t_accessories", C + 14)
def _():
    return U(ring(7.4, 13.2, 3.8), ring(16.6, 13.2, 3.8), arc(12, 12.4, 1.6, 200, 340, w=1.6), line(3.7, 12.6, 2.4, 9.4), line(20.3, 12.6, 21.6, 9.4),
             crescent(7.4, 13.2, 3.1, -1.4, -1.4, 3.1), crescent(16.6, 13.2, 3.1, -1.4, -1.4, 3.1))

@icon("t_paint", C + 15)
def _():
    handle = line(19.8, 4.2, 12.6, 11.4, w=3.2)
    ferrule = line(12.6, 11.4, 10.8, 13.2, w=4.4)
    tip = U(disc(8.2, 15.8, 3.3), poly((10.6, 14.6), (6.0, 18.8), (6.8, 13.4)))
    return U(handle, ferrule, tip, disc(4.6, 20.4, 1.3))

# ---- Posing -------------------------------------------------------------------------------------
@icon("t_rig", C + 20)
def _():
    shape = U(line(7.2, 16.8, 16.8, 7.2, w=3.2), disc(5.4, 15.4, 2.2), disc(8.6, 18.6, 2.2), disc(15.4, 5.4, 2.2), disc(18.6, 8.6, 2.2))
    return shape.difference(line(8.0, 14.0, 14.0, 8.0, w=0.9))

@icon("t_ik", C + 21)
def _():
    return U(ring(5.6, 18.4, 2.3), ring(11.8, 12.2, 2.3), line(7.3, 16.7, 10.1, 13.9), line(13.5, 10.5, 15.6, 8.4), ring(18.2, 5.8, 3.3), disc(18.2, 5.8, 1.3),
             disc(5.6, 18.4, 1.0))

@icon("t_pose", C + 22)
def _():
    return U(disc(12, 4.8, 2.4), line(12, 8.4, 12, 14.4), line(12, 9.8, 5.2, 5.6), line(12, 9.8, 18.8, 5.6), line(12, 14.4, 7, 21.4), line(12, 14.4, 17, 21.4), disc(12, 14.4, 1.6))

# ---- Shading ------------------------------------------------------------------------------------
@icon("t_cel_shader", C + 30)
def _():
    return U(ring(12, 12, 8.7), right_half(disc(12, 12, 8.2), 12.0))

@icon("t_rim", C + 31)
def _():
    return U(ring(12, 12, 8.2, 1.4), arc(12, 12, 8.2, -85, 15, w=3.6), disc(12, 12, 0.0001))

@icon("t_outline", C + 32)
def _():
    return U(ring(12, 12, 8.4, 2.6), disc(12, 12, 4.4))

@icon("t_palette", C + 33)
def _():
    blob = Point(12, 12.6).buffer(1, quad_segs=Q)
    blob = affinity.scale(blob, 9.6, 8.2)
    body = blob.difference(blob.buffer(-SW))
    hole = disc(15.6, 15.4, 1.9)
    return U(body.difference(hole.buffer(0.9)), hole.buffer(0.0).difference(hole.buffer(-0.01)), disc(7.6, 11.2, 1.5), disc(11.2, 7.9, 1.5), disc(15.8, 8.4, 1.5), disc(18.2, 12.2, 1.5))

@icon("t_shader_ball", C + 34)
def _():
    ball = disc(12, 10.4, 6.8).difference(disc(9.7, 8.0, 1.7))
    return U(ball, line(4.2, 20.2, 19.8, 20.2, w=1.6))

# ---- Iluminação ---------------------------------------------------------------------------------
@icon("t_sun", C + 40)
def _():
    rays = [line(8 + 5.6 * math.cos(math.radians(a)), 8 + 5.6 * math.sin(math.radians(a)), 8 + 7.6 * math.cos(math.radians(a)), 8 + 7.6 * math.sin(math.radians(a)), w=1.6) for a in (180, 225, 270)]
    return U(ring(8, 8, 3.2), right_half(disc(8, 8, 3.0), 8.0), *rays, line(11.8, 11.8, 20.2, 20.2), arrow_head((20.8, 20.8), (1, 1), 4.2))

@icon("t_shadows", C + 41)
def _():
    obj = rrect(3.6, 3.6, 8.4, 8.4, 1.8)
    cast = poly((12.6, 12.6), (21, 9.6), (21.4, 17.6), (16.6, 21.2), (12.6, 21.2)).buffer(-0.3, join_style=1)
    return U(obj, cast)

@icon("t_ambient", C + 42)
def _():
    def dome(r):
        return Polygon([(12 + r * math.cos(math.radians(a)), 17.4 - r * math.sin(math.radians(a))) for a in range(0, 181, 5)])
    return U(outline(*dome(8.8).exterior.coords[:-1]), dome(4.4), line(2.8, 20.4, 21.2, 20.4, w=1.6))

@icon("t_rigs", C + 43)
def _():
    return U(ring(4.8, 6.2, 2.4), ring(19.2, 6.2, 2.4), ring(12, 3.8, 2.0, 1.5), disc(12, 17.4, 3.4), line(6.4, 8.0, 9.6, 14.6, w=1.5), line(17.6, 8.0, 14.4, 14.6, w=1.5), line(12, 6.0, 12, 13.6, w=1.5), line(4.4, 21.4, 19.6, 21.4, w=1.6))

# ---- Transformações (Modelagem e Cenário) -------------------------------------------------------
@icon("t_select", C + 50)
def _():
    corners = U(line(3.4, 8.4, 3.4, 3.4, 8.4, 3.4), line(15.6, 3.4, 20.6, 3.4, 20.6, 8.4), line(3.4, 15.6, 3.4, 20.6, 8.4, 20.6))
    cur = poly((9.2, 8.6), (9.2, 20.2), (12.2, 17.2), (14.6, 21.4), (16.8, 20.2), (14.4, 16.0), (18.6, 16.0)).buffer(0.0)
    return U(corners, cur)

@icon("t_move", C + 51)
def _():
    return U(line(12, 4.6, 12, 19.4), line(4.6, 12, 19.4, 12), arrow_head((12, 2.4), (0, -1)), arrow_head((12, 21.6), (0, 1)), arrow_head((2.4, 12), (-1, 0)), arrow_head((21.6, 12), (1, 0)),
             disc(12, 12, 2.0))

@icon("t_rotate", C + 52)
def _():
    return U(arc(12, 12, 7.8, 30, 335, w=SW), arrow_head((12 + 7.8 * math.cos(math.radians(352)), 12 + 7.8 * math.sin(math.radians(352))), (-math.sin(math.radians(352)), math.cos(math.radians(352))), 4.4, 0.6), disc(12, 12, 2.1))

@icon("t_scale", C + 53)
def _():
    return U(rrect(3.6, 12.6, 7.8, 7.8, 1.6), rrect_fill(5.6, 14.6, 3.8, 3.8, 0.8), line(11.6, 12.4, 19.8, 4.2), arrow_head((21.0, 3.0), (1, -1), 4.4), line(14.6, 3.6, 20.6, 3.6, 20.6, 9.6, w=1.6))

@icon("t_add_cube", C + 54)
def _():
    top, ur, lr, bot, ll, ul, ctr = (10, 4.6), (16.9, 8.6), (16.9, 16.6), (10, 20.6), (3.1, 16.6), (3.1, 8.6), (10, 12.6)
    cube = U(outline(top, ur, lr, bot, ll, ul), line(ctr, ul), line(ctr, ur), line(ctr, bot), poly(ur, lr, bot, ctr).buffer(-0.25))
    plus = U(line(18.8, 2.4, 18.8, 9.6, w=1.9), line(15.2, 6.0, 22.4, 6.0, w=1.9))
    return U(cube.difference(disc(18.8, 6.0, 5.2)), plus)

# ---- Animação -----------------------------------------------------------------------------------
@icon("t_timeline", C + 60)
def _():
    ticks = [line(x, 17.0, x, 19.6, w=1.4) for x in (5.0, 8.5, 15.5, 19.0)]
    return U(line(3.2, 19.8, 20.8, 19.8, w=1.6), *ticks, poly((8.6, 3.8), (15.4, 3.8), (15.4, 8.0), (12, 11.6), (8.6, 8.0)).buffer(0.2, join_style=1), line(12, 11.6, 12, 19.8, w=1.6))

@icon("t_curves", C + 61)
def _():
    pts = []
    for i in range(0, 41):
        t = i / 40
        p0, p1, p2, p3 = (3.6, 19.2), (11.2, 19.2), (12.8, 4.8), (20.4, 4.8)
        x = (1 - t) ** 3 * p0[0] + 3 * (1 - t) ** 2 * t * p1[0] + 3 * (1 - t) * t * t * p2[0] + t ** 3 * p3[0]
        y = (1 - t) ** 3 * p0[1] + 3 * (1 - t) ** 2 * t * p1[1] + 3 * (1 - t) * t * t * p2[1] + t ** 3 * p3[1]
        pts.append((x, y))
    return U(line(*pts, w=SW), disc(3.6, 19.2, 2.0), disc(20.4, 4.8, 2.0), line(3.6, 19.2, 8.4, 19.2, w=1.2), line(20.4, 4.8, 15.6, 4.8, w=1.2), ring(8.8, 19.2, 1.1, 1.2), ring(15.2, 4.8, 1.1, 1.2))

@icon("t_clips", C + 62)
def _():
    back = line(7.2, 5.2, 20.4, 5.2, 20.4, 16.4, w=SW)
    front = rrect(3.4, 8.2, 14.6, 11.8, 2.2)
    return U(back, front, poly((8.4, 11.4), (14.4, 14.1), (8.4, 16.8)).buffer(0.25, join_style=1))

# ---- Render -------------------------------------------------------------------------------------
@icon("t_lens", C + 70)
def _():
    spokes = [line(12 + 3.6 * math.cos(math.radians(a)), 12 + 3.6 * math.sin(math.radians(a)), 12 + 8.0 * math.cos(math.radians(a + 22)), 12 + 8.0 * math.sin(math.radians(a + 22)), w=1.5) for a in range(0, 360, 60)]
    return U(ring(12, 12, 8.8), *spokes, disc(12, 12, 2.1))

@icon("t_passes", C + 71)
def _():
    top = [(12, 3.4), (21, 8.4), (12, 13.4), (3, 8.4)]
    return U(outline(*top), right_half(Polygon(top), 12.0).buffer(-0.2), line(3, 12.6, 12, 17.6, 21, 12.6), line(3, 16.8, 12, 21.8, 21, 16.8))

@icon("t_export", C + 72)
def _():
    return U(line(4.4, 13.4, 4.4, 20, 19.6, 20, 19.6, 13.4), line(12, 16.2, 12, 4.2), line(7.6, 8.4, 12, 4.0, 16.4, 8.4), disc(12, 16.2, 1.5))

# ---- Biblioteca ---------------------------------------------------------------------------------
@icon("t_grid", C + 80)
def _():
    return U(rrect(3.6, 3.6, 7.2, 7.2, 1.8), rrect(13.2, 3.6, 7.2, 7.2, 1.8), rrect(3.6, 13.2, 7.2, 7.2, 1.8), rrect_fill(13.2, 13.2, 7.2, 7.2, 1.8))

@icon("t_search", C + 81)
def _():
    return U(ring(10.2, 10.2, 6.2), line(14.8, 14.8, 20.6, 20.6, w=2.4), right_half(disc(10.2, 10.2, 5.4), 10.2).intersection(box(0, 10.2, 30, 30)))

@icon("t_filter", C + 82)
def _():
    funnel = [(3.6, 4.6), (20.4, 4.6), (14.0, 12.4), (14.0, 19.4), (10.0, 17.4), (10.0, 12.4)]
    return U(outline(*funnel), poly((10.0, 12.4), (14.0, 12.4), (14.0, 19.4), (10.0, 17.4)).buffer(-0.1))


# ------------------------------------------------------------------------------------------------ fonte TrueType
FONT_K = 42.0   # unidades de fonte por unidade da grade de 24: a arte dos ícones ocupa ~2/3 da grade, o Phosphor ocupa quase toda


def to_font_units(geom):
    # centro da grade (12, 12) -> centro da caixa de arte do Phosphor (512, 448); y para cima
    g = affinity.translate(geom, -12, -12)
    g = affinity.scale(g, FONT_K, -FONT_K, origin=(0, 0))
    return affinity.translate(g, 512, 448)


def glyph_from(geom):
    pen = TTGlyphPen(None)
    g = to_font_units(geom).buffer(0)
    geoms = [g] if g.geom_type == "Polygon" else list(g.geoms)
    for p in geoms:
        if p.is_empty:
            continue
        p = orient(p, sign=-1.0)  # TrueType: contorno externo horário, furos anti-horário (y para cima)
        for ring_ in [p.exterior] + list(p.interiors):
            pts = [(round(x), round(y)) for x, y in ring_.coords[:-1]]
            clean = [pts[0]] + [q for a, q in zip(pts, pts[1:]) if q != a]
            if len(clean) < 3:
                continue
            pen.moveTo(clean[0])
            for q in clean[1:]:
                pen.lineTo(q)
            pen.closePath()
    return pen.glyph()


def build_font(path):
    order = [".notdef"] + [f"uni{c:04X}" for _, (c, _) in ICONS.items()]
    glyphs = {".notdef": TTGlyphPen(None).glyph()}
    cmap = {}
    for name, (code, fn) in ICONS.items():
        gn = f"uni{code:04X}"
        glyphs[gn] = glyph_from(fn())
        cmap[code] = gn
    fb = FontBuilder(1024, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({g: (1024, 0) for g in order})
    fb.setupHorizontalHeader(ascent=960, descent=-64)
    fb.setupNameTable({"familyName": "ANIGO Icons", "styleName": "Regular", "uniqueFontIdentifier": "ANIGO Icons 1.0", "fullName": "ANIGO Icons", "psName": "ANIGOIcons-Regular", "version": "Version 1.0"})
    fb.setupOS2(sTypoAscender=960, sTypoDescender=-64, usWinAscent=960, usWinDescent=64, sxHeight=500, sCapHeight=700)
    fb.setupPost()
    path.parent.mkdir(parents=True, exist_ok=True)
    fb.save(str(path))


def write_rust(path):
    lines = ["//! Gerado por `brand/gen_icons.py` — NÃO editar à mão. Ícones próprios do ANIGO (fonte `AnigoIcons.ttf`, pontos de código",
             "//! F100–F1FF). Cada ícone tem um trecho sólido do lado de baixo/direita (o \"lado de sombra\" do cel shading).", ""]
    for name, (code, _) in ICONS.items():
        lines.append(f"pub const {name.upper()}: &str = \"\\u{{{code:X}}}\";")
    lines.append("")
    lines.append(f"/// Todos os ícones (nome, glifo) — usado pelos testes e pela folha de contato.")
    lines.append("pub const ALL: &[(&str, &str)] = &[")
    for name in ICONS:
        lines.append(f"    (\"{name}\", {name.upper()}),")
    lines.append("];")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


# ------------------------------------------------------------------------------------------------ folha de contato
def raster(geom, px, color=(238, 236, 250, 255), bg=None):
    ss = 8
    n = px * ss
    img = Image.new("RGBA", (n, n), bg or (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    from PIL import ImageChops
    layer = Image.new("L", (n, n), 0)
    s = n / 24.0
    geoms = [geom] if geom.geom_type == "Polygon" else list(geom.geoms)
    for p in geoms:
        one = Image.new("L", (n, n), 0)
        od = ImageDraw.Draw(one)
        od.polygon([(x * s, y * s) for x, y in p.exterior.coords], fill=255)
        for r in p.interiors:
            od.polygon([(x * s, y * s) for x, y in r.coords], fill=0)
        layer = ImageChops.lighter(layer, one)
    img.paste(Image.new("RGBA", (n, n), color), (0, 0), layer)
    return img.resize((px, px), Image.LANCZOS)


def contact_sheet(path):
    names = list(ICONS)
    cols = 12
    cell = 96
    rows = math.ceil(len(names) / cols)
    sheet = Image.new("RGBA", (cols * cell, rows * (cell + 56)), (24, 22, 36, 255))
    for i, name in enumerate(names):
        geom = ICONS[name][1]()
        x, y = (i % cols) * cell, (i // cols) * (cell + 56)
        big = raster(geom, 64)
        sheet.paste(big, (x + 16, y + 8), big)
        for k, px in enumerate((16, 24)):
            small = raster(geom, px)
            sheet.paste(small, (x + 10 + k * 40, y + 78), small)
        ImageDraw.Draw(sheet).text((x + 4, y + 108 - 6 + 0), name.replace("t_", "").replace("ws_", "ws ")[:14], fill=(150, 146, 178, 255))
    sheet.convert("RGB").save(path)


if __name__ == "__main__":
    build_font(STUDIO / "assets" / "fonts" / "AnigoIcons.ttf")
    write_rust(STUDIO / "src" / "icons_anigo.rs")
    contact_sheet(ROOT / "icons_sheet.png")
    print(len(ICONS), "ícones")
