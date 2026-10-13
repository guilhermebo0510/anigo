//! Matemática pura do editor de UV: transformação UV <-> tela (zoom/pan), hit-test no espaço UV,
//! mapeamento seleção 3D <-> cantos de UV, transformações G/R/S e cores de mapa. Sem egui nem estado,
//! tudo testável sem janela.

use crate::viewport::picking::{self, ClickHit};
use anigo_modeling::uv::seams::split_islands;
use anigo_modeling::{edge_key, EdgeKey, EditMesh, SelectMode, Selection, Topology};
use glam::Vec2;
use std::collections::BTreeSet;

// ------------------------------------------------------------------------- vista (zoom/pan)

pub const ZOOM_MIN: f32 = 4.0;
pub const ZOOM_MAX: f32 = 200_000.0;

/// Vista 2D do editor: `center` é o ponto UV no centro do painel; `zoom` são pixels por unidade UV.
/// O eixo V cresce para cima (convenção de textura do Blender), então o eixo Y da tela é invertido.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvView {
    pub center: Vec2,
    pub zoom: f32,
}

impl Default for UvView {
    fn default() -> Self {
        Self { center: Vec2::splat(0.5), zoom: 256.0 }
    }
}

impl UvView {
    /// Enquadra a caixa `[lo, hi]` (UV) num painel de `size` pixels, com folga.
    pub fn frame(lo: Vec2, hi: Vec2, size: Vec2) -> Self {
        let ext = (hi - lo).max(Vec2::splat(1e-3));
        let zoom = (size.x.max(8.0) / ext.x).min(size.y.max(8.0) / ext.y) * 0.84;
        Self { center: (lo + hi) * 0.5, zoom: zoom.clamp(ZOOM_MIN, ZOOM_MAX) }
    }

    /// `origin` = centro do painel em coordenadas de tela.
    pub fn to_screen(&self, uv: Vec2, origin: Vec2) -> Vec2 {
        origin + Vec2::new((uv.x - self.center.x) * self.zoom, -(uv.y - self.center.y) * self.zoom)
    }

    pub fn to_uv(&self, screen: Vec2, origin: Vec2) -> Vec2 {
        let d = screen - origin;
        self.center + Vec2::new(d.x / self.zoom, -d.y / self.zoom)
    }

    /// Zoom em torno do cursor: o ponto UV sob `screen` não se move.
    pub fn zoom_at(&mut self, factor: f32, screen: Vec2, origin: Vec2) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let before = self.to_uv(screen, origin);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let after = self.to_uv(screen, origin);
        self.center += before - after;
    }

    /// Arrasta a vista por `delta` pixels de tela (o conteúdo acompanha o mouse).
    pub fn pan_px(&mut self, delta: Vec2) {
        self.center += Vec2::new(-delta.x, delta.y) / self.zoom;
    }
}

// ------------------------------------------------------------------------- geometria 2D

/// Ponto dentro de polígono (regra par-ímpar; independe do enrolamento, então vale para faces invertidas).
pub fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn dist_point_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 <= 1e-20 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

#[allow(dead_code)]
pub fn face_uvs(mesh: &EditMesh, f: u32) -> Vec<Vec2> {
    mesh.faces.get(f as usize).map(|face| face.corners.iter().map(|c| c.uv).collect()).unwrap_or_default()
}

fn visible(mesh: &EditMesh, f: usize) -> bool {
    mesh.faces.get(f).is_some_and(|x| !x.hidden)
}

// ------------------------------------------------------------------------- picking no espaço UV

/// Face sob o ponto UV. Em sobreposições vence a de maior índice (a que é desenhada por cima).
pub fn pick_face(mesh: &EditMesh, p: Vec2) -> Option<u32> {
    (0..mesh.faces.len()).rev().filter(|&f| visible(mesh, f)).find_map(|f| {
        let poly: Vec<Vec2> = mesh.faces[f].corners.iter().map(|c| c.uv).collect();
        point_in_polygon(p, &poly).then_some(f as u32)
    })
}

/// Vértice 3D cujo canto de UV está mais próximo de `p` (até `radius`, em unidades UV).
pub fn pick_vert(mesh: &EditMesh, p: Vec2, radius: f32) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    for face in &mesh.faces {
        if face.hidden {
            continue;
        }
        for c in &face.corners {
            let d = c.uv.distance(p);
            if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c.vert));
            }
        }
    }
    best.map(|(_, v)| v)
}

/// Aresta 3D cujo segmento de UV está mais próximo de `p` (até `radius`).
pub fn pick_edge(mesh: &EditMesh, p: Vec2, radius: f32) -> Option<EdgeKey> {
    let mut best: Option<(f32, EdgeKey)> = None;
    for face in mesh.faces.iter().filter(|f| !f.hidden) {
        let n = face.corners.len();
        for i in 0..n {
            let (a, b) = (face.corners[i], face.corners[(i + 1) % n]);
            let d = dist_point_segment(p, a.uv, b.uv);
            if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, edge_key(a.vert, b.vert)));
            }
        }
    }
    best.map(|(_, e)| e)
}

/// O que há sob `p` para o modo de seleção atual (mesmo `ClickHit` do viewport 3D).
pub fn pick(mesh: &EditMesh, mode: SelectMode, p: Vec2, radius: f32) -> ClickHit {
    match mode {
        SelectMode::Vertex => pick_vert(mesh, p, radius).map_or(ClickHit::None, ClickHit::Vertex),
        SelectMode::Edge => pick_edge(mesh, p, radius).map_or(ClickHit::None, ClickHit::Edge),
        SelectMode::Face => pick_face(mesh, p).map_or(ClickHit::None, ClickHit::Face),
    }
}

/// Faces da ilha de UV que contém `f` (ilha = costuras + descontinuidades de UV).
pub fn island_faces(mesh: &EditMesh, f: u32) -> Vec<u32> {
    split_islands(mesh).into_iter().find(|i| i.faces.contains(&f)).map(|i| i.faces).unwrap_or_default()
}

/// Seleção resultante de clicar em `p`. `island`: seleciona a ilha inteira sob o cursor.
/// Devolve também o vértice ativo (para o modo vértice).
pub fn click_selection(
    mesh: &EditMesh,
    current: &Selection,
    mode: SelectMode,
    p: Vec2,
    radius: f32,
    extend: bool,
    island: bool,
) -> (Selection, Option<u32>) {
    if island {
        return match pick_face(mesh, p) {
            Some(f) => {
                let faces = island_faces(mesh, f);
                let mut base = if extend { current.clone() } else { Selection::default() };
                if mode == SelectMode::Face {
                    base.faces.extend(faces);
                    (base.flush(mesh, SelectMode::Face), None)
                } else {
                    let isl = Selection::from_faces(mesh, faces);
                    base.verts.extend(isl.verts.iter().copied());
                    base.edges.extend(isl.edges.iter().copied());
                    (base.flush(mesh, mode), None)
                }
            }
            None if extend => (current.clone(), None),
            None => (Selection::default(), None),
        };
    }
    let hit = pick(mesh, mode, p, radius);
    let sel = picking::apply_click(mesh, current, mode, hit, extend);
    let active = match hit {
        ClickHit::Vertex(v) => Some(v),
        _ => None,
    };
    (sel, active)
}

/// Seleção por caixa no espaço UV. Vértice: cantos dentro; Aresta: segmentos com os dois extremos
/// dentro; Face: centro do polígono UV dentro.
pub fn box_select(mesh: &EditMesh, mode: SelectMode, lo: Vec2, hi: Vec2) -> Selection {
    let inside = |p: Vec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
    match mode {
        SelectMode::Vertex => {
            let verts: BTreeSet<u32> = mesh
                .faces
                .iter()
                .filter(|f| !f.hidden)
                .flat_map(|f| f.corners.iter().filter(|c| inside(c.uv)).map(|c| c.vert))
                .collect();
            Selection::from_verts(verts).flush(mesh, SelectMode::Vertex)
        }
        SelectMode::Edge => {
            let mut edges: BTreeSet<EdgeKey> = BTreeSet::new();
            for face in mesh.faces.iter().filter(|f| !f.hidden) {
                let n = face.corners.len();
                for i in 0..n {
                    let (a, b) = (face.corners[i], face.corners[(i + 1) % n]);
                    if inside(a.uv) && inside(b.uv) {
                        edges.insert(edge_key(a.vert, b.vert));
                    }
                }
            }
            Selection::from_edges(edges).flush(mesh, SelectMode::Edge)
        }
        SelectMode::Face => {
            let faces = mesh.faces.iter().enumerate().filter(|(_, f)| !f.hidden && !f.corners.is_empty()).filter_map(|(i, f)| {
                let c = f.corners.iter().map(|c| c.uv).sum::<Vec2>() / f.corners.len() as f32;
                inside(c).then_some(i as u32)
            });
            Selection::from_faces(mesh, faces)
        }
    }
}

/// Une duas seleções (Shift + caixa).
pub fn merge_selection(a: &Selection, b: &Selection) -> Selection {
    let mut s = a.clone();
    s.verts.extend(b.verts.iter().copied());
    s.edges.extend(b.edges.iter().copied());
    s.faces.extend(b.faces.iter().copied());
    s
}

// ------------------------------------------------------------------------- seleção 3D -> cantos

/// Um canto de UV é "selecionado" quando a seleção 3D o cobre: no modo Face, quando a face está
/// selecionada; nos demais, quando o vértice está (seleção sincronizada, como o UV Sync do Blender —
/// um vértice numa costura seleciona as duas cópias de UV).
pub fn corner_selected(sel: &Selection, mode: SelectMode, face: u32, vert: u32) -> bool {
    match mode {
        SelectMode::Face => sel.faces.contains(&face),
        _ => sel.verts.contains(&vert),
    }
}

/// Todos os cantos selecionados `(face, índice do canto)`.
pub fn selected_corners(mesh: &EditMesh, sel: &Selection, mode: SelectMode) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        for (ci, c) in face.corners.iter().enumerate() {
            if corner_selected(sel, mode, fi as u32, c.vert) {
                out.push((fi as u32, ci));
            }
        }
    }
    out
}

/// Caixa envolvente dos UVs dos cantos dados.
pub fn corners_bounds(mesh: &EditMesh, corners: &[(u32, usize)]) -> Option<(Vec2, Vec2)> {
    let mut it = corners.iter().filter_map(|&(f, c)| mesh.faces.get(f as usize)?.corners.get(c).map(|c| c.uv));
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Arestas selecionadas para marcar/limpar costura: as da seleção ou, na falta, as totalmente selecionadas.
pub fn selected_edges(mesh: &EditMesh, sel: &Selection) -> Vec<EdgeKey> {
    if !sel.edges.is_empty() {
        return sel.edges.iter().copied().collect();
    }
    let topo = Topology::build(mesh);
    sel.edges_fully_selected(&topo)
}

/// Faces cuja operação de UV deve valer: as faces selecionadas (se houver) ou `None` = malha toda.
pub fn scope_faces(sel: &Selection) -> Option<Vec<u32>> {
    (!sel.faces.is_empty()).then(|| sel.faces.iter().copied().collect())
}

// ------------------------------------------------------------------------- transformações G/R/S

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XformKind {
    Grab,
    Rotate,
    Scale,
}

impl XformKind {
    pub fn label(self) -> &'static str {
        match self {
            XformKind::Grab => "Mover UV",
            XformKind::Rotate => "Girar UV",
            XformKind::Scale => "Escalar UV",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisLock {
    #[default]
    None,
    U,
    V,
}

/// Transformação interativa dos UVs: `start` é o ponto UV do mouse ao iniciar, `pivot` o centro.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform {
    pub kind: XformKind,
    pub pivot: Vec2,
    pub start: Vec2,
    pub lock: AxisLock,
    /// Valor digitado: deslocamento (Mover), graus (Girar) ou fator (Escalar).
    pub numeric: Option<f32>,
}

impl Xform {
    pub fn new(kind: XformKind, pivot: Vec2, start: Vec2) -> Self {
        Self { kind, pivot, start, lock: AxisLock::None, numeric: None }
    }

    /// Alterna o travamento de eixo (apertar a mesma tecla de novo libera).
    pub fn toggle_lock(&mut self, lock: AxisLock) {
        self.lock = if self.lock == lock { AxisLock::None } else { lock };
    }

    fn grab_delta(&self, cur: Vec2) -> Vec2 {
        if let Some(n) = self.numeric {
            return match self.lock {
                AxisLock::V => Vec2::new(0.0, n),
                _ => Vec2::new(n, 0.0),
            };
        }
        let d = cur - self.start;
        match self.lock {
            AxisLock::None => d,
            AxisLock::U => Vec2::new(d.x, 0.0),
            AxisLock::V => Vec2::new(0.0, d.y),
        }
    }

    pub fn angle(&self, cur: Vec2) -> f32 {
        if let Some(n) = self.numeric {
            return n.to_radians();
        }
        let (a, b) = (self.start - self.pivot, cur - self.pivot);
        if a.length_squared() < 1e-12 || b.length_squared() < 1e-12 {
            return 0.0;
        }
        b.y.atan2(b.x) - a.y.atan2(a.x)
    }

    pub fn scale_factors(&self, cur: Vec2) -> Vec2 {
        let f = match self.numeric {
            Some(n) => n,
            None => {
                let (a, b) = ((self.start - self.pivot).length(), (cur - self.pivot).length());
                if a < 1e-6 {
                    1.0
                } else {
                    b / a
                }
            }
        };
        match self.lock {
            AxisLock::None => Vec2::splat(f),
            AxisLock::U => Vec2::new(f, 1.0),
            AxisLock::V => Vec2::new(1.0, f),
        }
    }

    /// Novo UV de `p` com o mouse em `cur`.
    pub fn map(&self, cur: Vec2, p: Vec2) -> Vec2 {
        match self.kind {
            XformKind::Grab => p + self.grab_delta(cur),
            XformKind::Rotate => {
                let (s, c) = self.angle(cur).sin_cos();
                let r = p - self.pivot;
                self.pivot + Vec2::new(r.x * c - r.y * s, r.x * s + r.y * c)
            }
            XformKind::Scale => self.pivot + (p - self.pivot) * self.scale_factors(cur),
        }
    }

    /// Texto para a barra de status durante a operação.
    pub fn describe(&self, cur: Vec2) -> String {
        let lock = match self.lock {
            AxisLock::None => "",
            AxisLock::U => "  [eixo U]",
            AxisLock::V => "  [eixo V]",
        };
        let body = match self.kind {
            XformKind::Grab => {
                let d = self.grab_delta(cur);
                format!("ΔU {:+.4}  ΔV {:+.4}", d.x, d.y)
            }
            XformKind::Rotate => format!("Ângulo {:+.1}°", self.angle(cur).to_degrees()),
            XformKind::Scale => {
                let f = self.scale_factors(cur);
                format!("Fator U {:.3}  V {:.3}", f.x, f.y)
            }
        };
        format!("{}: {body}{lock}  (Enter confirma · Esc cancela · X/Y travam eixo · digite um valor)", self.kind.label())
    }
}

/// Reflexão dos UVs em torno de `pivot` ao longo do eixo U (`horizontal`) ou V.
pub fn mirror_point(p: Vec2, pivot: Vec2, horizontal: bool) -> Vec2 {
    if horizontal {
        Vec2::new(2.0 * pivot.x - p.x, p.y)
    } else {
        Vec2::new(p.x, 2.0 * pivot.y - p.y)
    }
}

/// Rotação de 90 graus anti-horária em torno de `pivot`.
pub fn rotate90_point(p: Vec2, pivot: Vec2) -> Vec2 {
    let r = p - pivot;
    pivot + Vec2::new(-r.y, r.x)
}

// ------------------------------------------------------------------------- cores

/// Cor (RGB) por ilha: matiz espaçado pela razão áurea (cores vizinhas bem distintas).
pub fn island_rgb(i: u32) -> [u8; 3] {
    let h = (i as f32 * 0.618_034).fract();
    hsv_to_rgb(h, 0.5, 0.78)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let (i, f) = (h6.floor() as i32, h6.fract());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8]
}

fn lerp3(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

/// Rampa de estiramento anisotrópico (`face_stretch`, 1 = sem distorção): azul → verde → amarelo → vermelho.
/// Valor não finito (UV colapsado) devolve magenta.
pub fn stretch_rgb(s: f32) -> [u8; 3] {
    if !s.is_finite() {
        return [220, 40, 200];
    }
    const STOPS: [(f32, [u8; 3]); 4] = [(1.0, [48, 104, 224]), (1.25, [60, 190, 110]), (1.6, [236, 214, 64]), (2.2, [232, 56, 48])];
    let s = s.max(1.0);
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if s <= b {
            return lerp3(ca, cb, (s - a) / (b - a));
        }
    }
    STOPS[3].1
}

/// Rampa de densidade de texels relativa à média (`ratio = d / média`): azul (pouca) → verde (média) → vermelho (muita).
pub fn density_rgb(ratio: f32) -> [u8; 3] {
    if !ratio.is_finite() || ratio <= 0.0 {
        return [90, 90, 96];
    }
    let t = (ratio.log2() / 1.5).clamp(-1.0, 1.0);
    if t < 0.0 {
        lerp3([60, 190, 110], [48, 104, 224], -t)
    } else {
        lerp3([60, 190, 110], [232, 56, 48], t)
    }
}

/// Imagem de checker `cells x cells` (`px` pixels de lado), em tons de cinza azulado; a célula do canto
/// inferior esquerdo (u=0, v=0) é marcada para indicar a orientação.
pub fn checker_pixels(cells: usize, px: usize) -> Vec<[u8; 3]> {
    let cells = cells.max(1);
    let mut out = Vec::with_capacity(px * px);
    for y in 0..px {
        for x in 0..px {
            let (cx, cy) = (x * cells / px, y * cells / px);
            let light = (cx + cy) % 2 == 0;
            // y cresce para baixo na imagem; v=0 é a última linha de células
            let origin_cell = cx == 0 && cy == cells - 1;
            out.push(match (origin_cell, light) {
                (true, true) => [214, 120, 108],
                (true, false) => [150, 78, 70],
                (false, true) => [190, 192, 204],
                (false, false) => [112, 114, 130],
            });
        }
    }
    out
}

// ------------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{Mode, ModelingSession, PrimitiveKind};
    use anigo_modeling::primitives;
    use glam::Vec3;

    fn quad_uv_mesh() -> EditMesh {
        // dois quads lado a lado: face 0 em UV [0,.5]x[0,1], face 1 em [.5,1]x[0,1]; vértices 1 e 4 compartilhados
        let mut m = EditMesh::new("duas");
        for z in 0..2 {
            for x in 0..3 {
                m.add_vert(Vec3::new(x as f32, 0.0, z as f32));
            }
        }
        m.add_face(&[0, 1, 4, 3]).expect("face 0");
        m.add_face(&[1, 2, 5, 4]).expect("face 1");
        for f in &mut m.faces {
            for c in &mut f.corners {
                let p = m.verts[c.vert as usize].co;
                c.uv = Vec2::new(p.x * 0.5, p.z);
            }
        }
        m
    }

    #[test]
    fn view_roundtrip_and_v_axis_points_up() {
        let v = UvView { center: Vec2::new(0.5, 0.5), zoom: 300.0 };
        let origin = Vec2::new(400.0, 200.0);
        let s = v.to_screen(Vec2::new(0.5, 0.5), origin);
        assert!((s - origin).length() < 1e-4);
        // V maior = mais alto na tela (y menor)
        assert!(v.to_screen(Vec2::new(0.5, 1.0), origin).y < v.to_screen(Vec2::new(0.5, 0.0), origin).y);
        for uv in [Vec2::new(0.1, 0.9), Vec2::new(-2.0, 3.5), Vec2::ZERO] {
            let back = v.to_uv(v.to_screen(uv, origin), origin);
            assert!((back - uv).length() < 1e-5);
        }
    }

    #[test]
    fn zoom_keeps_point_under_cursor_fixed() {
        let mut v = UvView::default();
        let origin = Vec2::new(300.0, 150.0);
        let cursor = Vec2::new(420.0, 90.0);
        let uv_before = v.to_uv(cursor, origin);
        v.zoom_at(2.5, cursor, origin);
        assert!((v.to_uv(cursor, origin) - uv_before).length() < 1e-5);
        assert!((v.zoom - 640.0).abs() < 1e-3);
        v.zoom_at(1e9, cursor, origin);
        assert_eq!(v.zoom, ZOOM_MAX);
        let before = v;
        v.zoom_at(f32::NAN, cursor, origin);
        assert_eq!(v, before);
    }

    #[test]
    fn pan_moves_content_with_the_mouse() {
        let mut v = UvView::default();
        let origin = Vec2::ZERO;
        let p = Vec2::new(0.25, 0.75);
        let s0 = v.to_screen(p, origin);
        v.pan_px(Vec2::new(30.0, -12.0));
        let s1 = v.to_screen(p, origin);
        assert!((s1 - s0 - Vec2::new(30.0, -12.0)).length() < 1e-4);
    }

    #[test]
    fn frame_fits_the_unit_square_in_the_panel() {
        let size = Vec2::new(800.0, 300.0);
        let v = UvView::frame(Vec2::ZERO, Vec2::ONE, size);
        let origin = size * 0.5;
        let (a, b) = (v.to_screen(Vec2::ZERO, origin), v.to_screen(Vec2::ONE, origin));
        for p in [a, b] {
            assert!(p.x >= 0.0 && p.x <= size.x && p.y >= 0.0 && p.y <= size.y, "{p:?}");
        }
        // limitado pela altura
        assert!(((a.y - b.y).abs() - size.y * 0.84).abs() < 1.0);
    }

    #[test]
    fn polygon_hit_test_handles_concave_and_flipped() {
        let l = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.4), Vec2::new(0.4, 0.4), Vec2::new(0.4, 1.0), Vec2::new(0.0, 1.0)];
        assert!(point_in_polygon(Vec2::new(0.2, 0.8), &l));
        assert!(point_in_polygon(Vec2::new(0.8, 0.2), &l));
        assert!(!point_in_polygon(Vec2::new(0.8, 0.8), &l), "o recorte do L está fora");
        let rev: Vec<Vec2> = l.iter().rev().copied().collect();
        assert!(point_in_polygon(Vec2::new(0.2, 0.8), &rev));
        assert!(!point_in_polygon(Vec2::new(0.5, 0.5), &l[..2]));
    }

    #[test]
    fn pick_face_in_uv_space_prefers_topmost() {
        let m = quad_uv_mesh();
        assert_eq!(pick_face(&m, Vec2::new(0.25, 0.5)), Some(0));
        assert_eq!(pick_face(&m, Vec2::new(0.75, 0.5)), Some(1));
        assert_eq!(pick_face(&m, Vec2::new(1.5, 0.5)), None);
        // sobrepõe a face 1 sobre a 0: ganha a de maior índice
        let mut o = m.clone();
        for c in &mut o.faces[1].corners {
            c.uv -= Vec2::new(0.5, 0.0);
        }
        assert_eq!(pick_face(&o, Vec2::new(0.25, 0.5)), Some(1));
        // face oculta não é pegável
        o.faces[1].hidden = true;
        assert_eq!(pick_face(&o, Vec2::new(0.25, 0.5)), Some(0));
    }

    #[test]
    fn pick_vert_and_edge_respect_radius() {
        let m = quad_uv_mesh();
        let v = pick_vert(&m, Vec2::new(0.51, 0.01), 0.05).expect("canto");
        assert_eq!(v, 1, "canto (0.5, 0) é o vértice 1");
        assert_eq!(pick_vert(&m, Vec2::new(0.3, 0.5), 0.05), None);
        let e = pick_edge(&m, Vec2::new(0.25, 0.02), 0.05).expect("aresta inferior da face 0");
        let f0 = &m.faces[0];
        let bottom: Vec<u32> = f0.corners.iter().filter(|c| c.uv.y < 0.5).map(|c| c.vert).collect();
        assert_eq!(e, edge_key(bottom[0], bottom[1]));
        assert_eq!(pick_edge(&m, Vec2::new(0.25, 0.5), 0.05), None);
    }

    #[test]
    fn click_selection_by_mode_and_extend() {
        let m = quad_uv_mesh();
        let none = Selection::default();
        let (s, _) = click_selection(&m, &none, SelectMode::Face, Vec2::new(0.75, 0.5), 0.02, false, false);
        assert_eq!(s.faces.iter().copied().collect::<Vec<_>>(), vec![1]);
        let (s2, _) = click_selection(&m, &s, SelectMode::Face, Vec2::new(0.25, 0.5), 0.02, true, false);
        assert_eq!(s2.faces.len(), 2);
        let (s3, _) = click_selection(&m, &s2, SelectMode::Face, Vec2::new(0.25, 0.5), 0.02, true, false);
        assert_eq!(s3.faces.len(), 1, "Shift alterna");
        let (empty, _) = click_selection(&m, &s2, SelectMode::Face, Vec2::new(5.0, 5.0), 0.02, false, false);
        assert!(empty.is_empty());
        let (sv, active) = click_selection(&m, &none, SelectMode::Vertex, Vec2::new(0.0, 0.0), 0.02, false, false);
        assert_eq!(sv.verts.len(), 1);
        assert!(active.is_some_and(|a| sv.verts.contains(&a)));
    }

    #[test]
    fn island_click_selects_whole_island() {
        // face 0 e 1 separadas em UV (costura implícita por descontinuidade): duas ilhas
        let mut m = quad_uv_mesh();
        for c in &mut m.faces[1].corners {
            c.uv += Vec2::new(2.0, 0.0);
        }
        let (s, _) = click_selection(&m, &Selection::default(), SelectMode::Face, Vec2::new(0.25, 0.5), 0.02, false, true);
        assert_eq!(s.faces.iter().copied().collect::<Vec<_>>(), vec![0]);
        // costura de verdade: grid 3x1 sem descontinuidade de UV -> uma ilha só
        let mut g = primitives::grid(3, 1, 3.0, 1.0);
        for f in &mut g.faces {
            for c in &mut f.corners {
                c.uv = Vec2::new(g.verts[c.vert as usize].co.x / 3.0 + 0.5, g.verts[c.vert as usize].co.z + 0.5);
            }
        }
        let p = g.faces[0].corners.iter().map(|c| c.uv).sum::<Vec2>() / 4.0;
        let (s, _) = click_selection(&g, &Selection::default(), SelectMode::Face, p, 0.02, false, true);
        assert_eq!(s.faces.len(), 3);
    }

    #[test]
    fn box_select_by_mode() {
        let m = quad_uv_mesh();
        let f = box_select(&m, SelectMode::Face, Vec2::new(-0.1, -0.1), Vec2::new(0.6, 1.1));
        assert_eq!(f.faces.iter().copied().collect::<Vec<_>>(), vec![0]);
        let v = box_select(&m, SelectMode::Vertex, Vec2::new(-0.1, -0.1), Vec2::new(0.6, 1.1));
        assert_eq!(v.verts.len(), 4, "só os vértices do quad 0 (os compartilhados já estão nele)");
        assert_eq!(v.faces.len(), 1, "flush preenche a face totalmente coberta");
        let e = box_select(&m, SelectMode::Edge, Vec2::new(-0.1, -0.1), Vec2::new(0.6, 0.1));
        assert_eq!(e.edges.len(), 1);
        assert!(box_select(&m, SelectMode::Face, Vec2::new(3.0, 3.0), Vec2::new(4.0, 4.0)).is_empty());
    }

    #[test]
    fn selection_sync_3d_to_uv_corners() {
        let m = primitives::cube(2.0);
        let sel = Selection::from_faces(&m, [0u32, 3]);
        let c = selected_corners(&m, &sel, SelectMode::Face);
        assert_eq!(c.len(), 8);
        assert!(c.iter().all(|(f, _)| *f == 0 || *f == 3));
        // modo vértice: todos os cantos dos vértices selecionados (inclusive em outras faces)
        let vsel = Selection::from_verts([0u32]);
        let vc = selected_corners(&m, &vsel, SelectMode::Vertex);
        assert_eq!(vc.len(), 3, "um vértice de cubo pertence a 3 faces");
        assert!(selected_corners(&m, &Selection::default(), SelectMode::Face).is_empty());
        let (lo, hi) = corners_bounds(&m, &c).expect("caixa");
        assert!(lo.x <= hi.x && lo.y <= hi.y);
    }

    #[test]
    fn uv_click_updates_3d_selection_in_session() {
        // cubo -> unwrap com costuras -> clicar no centro UV de uma face seleciona essa face na sessão 3D
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        s.set_mode(Mode::Edit);
        s.set_select_mode(SelectMode::Face);
        let mesh = s.active_object().map(|o| o.mesh.clone()).expect("objeto");
        let target = 3u32;
        let center = face_uvs(&mesh, target).iter().copied().sum::<Vec2>() / 4.0;
        // o primitivo já vem com UVs por face (cada face em [0,1]^2 sobreposta): vence o maior índice
        let expect = pick_face(&mesh, center).expect("face sob o ponto");
        let (sel, active) = click_selection(&mesh, &s.selection, SelectMode::Face, center, 0.02, false, false);
        s.set_selection(sel, active);
        assert_eq!(s.selection.faces.iter().copied().collect::<Vec<_>>(), vec![expect]);
        // a seleção volta para o editor: os cantos selecionados são exatamente os da face
        let corners = selected_corners(&mesh, &s.selection, s.select_mode);
        assert_eq!(corners.len(), 4);
        assert!(corners.iter().all(|(f, _)| *f == expect));
    }

    #[test]
    fn grab_rotate_scale_math() {
        let pivot = Vec2::new(0.5, 0.5);
        let mut x = Xform::new(XformKind::Grab, pivot, Vec2::new(0.5, 0.5));
        let p = Vec2::new(0.2, 0.2);
        assert!((x.map(Vec2::new(0.7, 0.9), p) - Vec2::new(0.4, 0.6)).length() < 1e-6);
        x.toggle_lock(AxisLock::U);
        assert!((x.map(Vec2::new(0.7, 0.9), p) - Vec2::new(0.4, 0.2)).length() < 1e-6);
        x.toggle_lock(AxisLock::U);
        assert_eq!(x.lock, AxisLock::None);
        x.numeric = Some(0.25);
        x.lock = AxisLock::V;
        assert!((x.map(Vec2::ZERO, p) - Vec2::new(0.2, 0.45)).length() < 1e-6);

        let mut r = Xform::new(XformKind::Rotate, pivot, Vec2::new(1.0, 0.5));
        let rotated = r.map(Vec2::new(0.5, 1.0), Vec2::new(1.0, 0.5));
        assert!((rotated - Vec2::new(0.5, 1.0)).length() < 1e-5, "90° anti-horário: {rotated:?}");
        r.numeric = Some(180.0);
        assert!((r.map(Vec2::ZERO, Vec2::new(1.0, 0.5)) - Vec2::new(0.0, 0.5)).length() < 1e-5);

        let mut s = Xform::new(XformKind::Scale, pivot, Vec2::new(1.0, 0.5));
        let scaled = s.map(Vec2::new(1.5, 0.5), Vec2::new(1.0, 1.0));
        assert!((scaled - Vec2::new(1.5, 1.5)).length() < 1e-5);
        s.lock = AxisLock::U;
        s.numeric = Some(2.0);
        assert!((s.map(Vec2::ZERO, Vec2::new(1.0, 1.0)) - Vec2::new(1.5, 1.0)).length() < 1e-5);
        // início sobre o pivô: não explode
        let degenerate = Xform::new(XformKind::Scale, pivot, pivot);
        assert!(degenerate.map(Vec2::new(2.0, 2.0), Vec2::ONE).is_finite());
        assert!(x.describe(Vec2::ZERO).contains("Mover UV"));
    }

    #[test]
    fn mirror_and_rotate90() {
        let pivot = Vec2::new(0.5, 0.5);
        assert!((mirror_point(Vec2::new(0.1, 0.2), pivot, true) - Vec2::new(0.9, 0.2)).length() < 1e-6);
        assert!((mirror_point(Vec2::new(0.1, 0.2), pivot, false) - Vec2::new(0.1, 0.8)).length() < 1e-6);
        let mut p = Vec2::new(1.0, 0.5);
        for _ in 0..4 {
            p = rotate90_point(p, pivot);
        }
        assert!((p - Vec2::new(1.0, 0.5)).length() < 1e-5);
    }

    #[test]
    fn color_ramps_are_monotonic_and_robust() {
        assert_eq!(stretch_rgb(1.0), [48, 104, 224]);
        assert_eq!(stretch_rgb(0.2), stretch_rgb(1.0));
        assert_eq!(stretch_rgb(50.0), [232, 56, 48]);
        assert_eq!(stretch_rgb(f32::INFINITY), [220, 40, 200]);
        assert_eq!(stretch_rgb(f32::NAN), [220, 40, 200]);
        assert!(stretch_rgb(1.3)[0] > stretch_rgb(1.1)[0], "avança em direção ao amarelo/vermelho");
        assert_eq!(density_rgb(1.0), [60, 190, 110]);
        assert!(density_rgb(0.25)[2] > density_rgb(1.0)[2]);
        assert!(density_rgb(4.0)[0] > density_rgb(1.0)[0]);
        assert_eq!(density_rgb(0.0), density_rgb(f32::NAN));
        assert_ne!(island_rgb(0), island_rgb(1));
    }

    #[test]
    fn checker_marks_origin_cell_and_alternates() {
        let px = checker_pixels(8, 64);
        assert_eq!(px.len(), 64 * 64);
        let at = |x: usize, y: usize| px[y * 64 + x];
        assert_ne!(at(4, 4), at(12, 4), "células vizinhas alternam");
        assert_eq!(at(4, 4), at(12, 12));
        // célula (u=0, v=0) = canto inferior esquerdo da imagem
        let o = at(2, 62);
        assert!(o[0] > o[1] + 40, "célula de origem avermelhada: {o:?}");
    }

    #[test]
    fn selected_edges_and_scope_helpers() {
        let m = primitives::cube(2.0);
        let sel = Selection::from_edges([edge_key(0, 1)]);
        assert_eq!(selected_edges(&m, &sel), vec![edge_key(0, 1)]);
        let vsel = Selection::from_verts([0u32, 1]);
        let e = selected_edges(&m, &vsel);
        assert_eq!(e.len(), 1, "só uma aresta liga os dois vértices");
        assert_eq!(scope_faces(&Selection::default()), None);
        assert_eq!(scope_faces(&Selection::from_faces(&m, [2u32])), Some(vec![2]));
    }
}
