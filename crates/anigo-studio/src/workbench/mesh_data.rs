//! Conversão `EditMesh` (+ seleção) → dados de desenho do Workbench (CPU, testável).

use crate::theme;
use anigo_modeling::attributes::VertexChannel;
use anigo_modeling::normals::{compute_corner_normals, NormalParams};
use anigo_modeling::{convert::triangulate_polygon, EditMesh, SelectMode, Selection, Topology};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::HashSet;

/// 40 bytes: posição, normal, cor.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct SolidVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

/// 44 bytes: segmento (a→b), cor e largura em pixels. Desenhado como quad em espaço de tela.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub color: [f32; 4],
    pub width: f32,
}

/// 16 bytes: posição + estado (0 normal, 1 selecionado, 2 ativo).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct PointInstance {
    pub pos: [f32; 3],
    pub state: f32,
}

#[derive(Debug, Clone, Default)]
pub struct MeshDrawData {
    pub solid: Vec<SolidVertex>,
    pub lines: Vec<LineInstance>,
    pub points: Vec<PointInstance>,
}

/// Overlay de normais por corner (linhas curtas a partir de cada corner).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalOverlay {
    /// Comprimento das linhas (espaço local do objeto).
    pub length: f32,
    /// Só corners de vértices selecionados (quando há seleção de vértices).
    pub selected_only: bool,
}

/// Cor das linhas de normal automática (calculada por grupos suaves/ângulo).
pub const NORMAL_AUTO_COLOR: [f32; 4] = [0.35, 0.58, 1.0, 1.0];
/// Cor das linhas de normal customizada (`Corner::normal`).
pub const NORMAL_CUSTOM_COLOR: [f32; 4] = [1.0, 0.90, 0.20, 1.0];

/// Mapa de calor 0..1: azul, ciano, verde, amarelo, vermelho (cor sRGB, alfa 1).
pub fn heat_color(t: f32) -> [f32; 4] {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    const STOPS: [[f32; 3]; 5] = [
        [0.10, 0.18, 0.75],
        [0.10, 0.75, 0.85],
        [0.25, 0.80, 0.30],
        [0.97, 0.88, 0.20],
        [0.90, 0.15, 0.12],
    ];
    let x = t * (STOPS.len() - 1) as f32;
    let i = (x.floor() as usize).min(STOPS.len() - 2);
    let f = x - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, 1.0]
}

/// Cor de visualização do canal `ch` de um vértice (mapa de calor do valor pintado).
pub fn channel_view_color(vertex_color: [f32; 4], ch: VertexChannel) -> [f32; 4] {
    heat_color(vertex_color[ch.index()])
}

#[derive(Debug, Clone)]
pub struct BuildOptions<'a> {
    pub edit_mode: bool,
    pub select_mode: SelectMode,
    pub selection: Option<&'a Selection>,
    pub active_vert: Option<u32>,
    /// Wireframe sobre o sólido em modo Objeto.
    pub object_wire: bool,
    /// Objeto ativo em modo Objeto: contorno laranja (equivalente ao outline de seleção do Blender).
    pub highlight_object: bool,
    pub base_color: [f32; 4],
    pub normals: NormalParams,
    /// Desenha normais por corner como linhas (cores: automática vs. customizada).
    pub show_normals: Option<NormalOverlay>,
    /// Pinta o sólido com o mapa de calor de um canal NPR (AO/ShadowShift/OutlineWidth/SpecRim).
    pub channel_view: Option<VertexChannel>,
}

impl Default for BuildOptions<'_> {
    fn default() -> Self {
        Self {
            edit_mode: false,
            select_mode: SelectMode::Vertex,
            selection: None,
            active_vert: None,
            object_wire: false,
            highlight_object: false,
            base_color: [0.72, 0.72, 0.76, 1.0],
            normals: NormalParams::default(),
            show_normals: None,
            channel_view: None,
        }
    }
}

fn rgba(c: egui::Color32) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

pub fn build_draw_data(mesh: &EditMesh, opts: &BuildOptions) -> MeshDrawData {
    let mut out = MeshDrawData::default();
    let default_sel = Selection::default();
    let sel = opts.selection.unwrap_or(&default_sel);
    let select_rgba = rgba(theme::SELECT);

    // ---- sólido (não indexado: normais por corner)
    let cn = compute_corner_normals(mesh, &opts.normals);
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        let pts = mesh.face_positions(fi as u32);
        let selected_face = opts.edit_mode && sel.faces.contains(&(fi as u32));
        for [a, b, c] in triangulate_polygon(&pts) {
            for i in [a, b, c] {
                let base = match opts.channel_view {
                    Some(ch) => channel_view_color(mesh.verts[face.corners[i].vert as usize].color, ch),
                    None => opts.base_color,
                };
                let color = if selected_face { mix(base, select_rgba, 0.42) } else { base };
                out.solid.push(SolidVertex {
                    pos: pts[i].to_array(),
                    normal: cn[fi][i].to_array(),
                    color,
                });
            }
        }
    }

    // ---- arestas
    let topo = Topology::build(mesh);
    let seam = [0.95, 0.20, 0.20, 1.0];
    let sharp = [0.15, 0.85, 0.95, 1.0];
    let crease = [0.90, 0.25, 0.90, 1.0];
    for e in &topo.edges {
        let (a, b) = e.verts;
        let (pa, pb) = (mesh.co(a), mesh.co(b));
        let attr = mesh.edge_attr(a, b);
        let selected = sel.edges.contains(&e.verts);
        let line = if opts.edit_mode {
            if selected {
                Some((select_rgba, 2.6))
            } else if attr.seam {
                Some((seam, 2.0))
            } else if attr.sharp {
                Some((sharp, 2.0))
            } else if attr.crease > 0.0 {
                Some((crease, 2.0))
            } else {
                Some(([0.02, 0.02, 0.03, 0.9], 1.2))
            }
        } else if opts.highlight_object {
            Some(([select_rgba[0], select_rgba[1], select_rgba[2], 0.95], 1.6))
        } else if opts.object_wire {
            Some(([0.05, 0.05, 0.06, 0.55], 1.0))
        } else {
            None
        };
        if let Some((color, width)) = line {
            out.lines.push(LineInstance {
                a: pa.to_array(),
                b: pb.to_array(),
                color,
                width,
            });
        }
    }

    // ---- normais por corner
    if let Some(ov) = opts.show_normals {
        push_normal_lines(mesh, &cn, ov, opts, sel, &mut out.lines);
    }

    // ---- pontos
    if opts.edit_mode {
        let used = mesh.used_verts();
        for (i, v) in mesh.verts.iter().enumerate() {
            if !used[i] || v.hidden {
                continue;
            }
            let selected = sel.verts.contains(&(i as u32));
            let show = matches!(opts.select_mode, SelectMode::Vertex) || selected;
            if !show {
                continue;
            }
            let state = if opts.active_vert == Some(i as u32) && selected {
                2.0
            } else if selected {
                1.0
            } else {
                0.0
            };
            out.points.push(PointInstance {
                pos: v.co.to_array(),
                state,
            });
        }
    }
    out
}

/// Linhas de normais por corner. Corners de um mesmo vértice com a mesma normal (grupo suave)
/// geram uma só linha; customizadas e automáticas têm cores diferentes.
fn push_normal_lines(
    mesh: &EditMesh,
    cn: &[Vec<Vec3>],
    ov: NormalOverlay,
    opts: &BuildOptions,
    sel: &Selection,
    lines: &mut Vec<LineInstance>,
) {
    if !ov.length.is_finite() || ov.length <= 0.0 {
        return;
    }
    let only_sel = ov.selected_only && opts.edit_mode && !sel.verts.is_empty();
    let q = |x: f32| (x * 2048.0).round() as i32;
    let mut seen: HashSet<(u32, [i32; 3], bool)> = HashSet::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        for (ci, c) in face.corners.iter().enumerate() {
            if only_sel && !sel.verts.contains(&c.vert) {
                continue;
            }
            let n = cn[fi][ci];
            let custom = c.normal.is_some();
            if !seen.insert((c.vert, [q(n.x), q(n.y), q(n.z)], custom)) {
                continue;
            }
            let p = mesh.co(c.vert);
            lines.push(LineInstance {
                a: p.to_array(),
                b: (p + n * ov.length).to_array(),
                color: if custom { NORMAL_CUSTOM_COLOR } else { NORMAL_AUTO_COLOR },
                width: if custom { 1.8 } else { 1.3 },
            });
        }
    }
}

/// Caixa envolvente dos dados sólidos (para enquadrar a câmera).
pub fn solid_bounds(data: &MeshDrawData) -> Option<(Vec3, Vec3)> {
    let first = Vec3::from_array(data.solid.first()?.pos);
    Some(data.solid.iter().fold((first, first), |(lo, hi), v| {
        let p = Vec3::from_array(v.pos);
        (lo.min(p), hi.max(p))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;

    #[test]
    fn vertex_layouts_have_expected_strides() {
        assert_eq!(std::mem::size_of::<SolidVertex>(), 40);
        assert_eq!(std::mem::size_of::<LineInstance>(), 44);
        assert_eq!(std::mem::size_of::<PointInstance>(), 16);
    }

    #[test]
    fn cube_object_mode_has_solid_only() {
        let m = primitives::cube(2.0);
        let d = build_draw_data(&m, &BuildOptions::default());
        assert_eq!(d.solid.len(), 12 * 3);
        assert!(d.lines.is_empty() && d.points.is_empty());
        let (lo, hi) = solid_bounds(&d).unwrap();
        assert!((hi - lo - Vec3::splat(2.0)).length() < 1e-5);
    }

    #[test]
    fn edit_mode_draws_edges_and_points_and_colors_selection() {
        let m = primitives::cube(2.0);
        let sel = Selection::from_faces(&m, [0u32]);
        let d = build_draw_data(
            &m,
            &BuildOptions {
                edit_mode: true,
                selection: Some(&sel),
                select_mode: SelectMode::Face,
                ..Default::default()
            },
        );
        assert_eq!(d.lines.len(), 12);
        let selected_edges = d.lines.iter().filter(|l| l.width > 2.5).count();
        assert_eq!(selected_edges, 4);
        // modo face: só vértices selecionados aparecem
        assert_eq!(d.points.len(), 4);
        assert!(d.points.iter().all(|p| p.state == 1.0));
        // a face selecionada tem cor diferente
        let tinted = d.solid.iter().filter(|v| v.color != BuildOptions::default().base_color).count();
        assert_eq!(tinted, 6);
    }

    #[test]
    fn vertex_mode_shows_all_points_with_active() {
        let m = primitives::cube(2.0);
        let sel = Selection::from_verts([3u32]);
        let d = build_draw_data(
            &m,
            &BuildOptions {
                edit_mode: true,
                selection: Some(&sel),
                active_vert: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(d.points.len(), 8);
        assert_eq!(d.points.iter().filter(|p| p.state == 2.0).count(), 1);
    }

    #[test]
    fn seam_and_sharp_edges_get_special_colors() {
        let mut m = primitives::cube(2.0);
        m.set_seam(0, 1, true);
        m.set_sharp(1, 2, true);
        let d = build_draw_data(&m, &BuildOptions { edit_mode: true, ..Default::default() });
        let seams = d.lines.iter().filter(|l| l.color[0] > 0.9 && l.color[1] < 0.3).count();
        let sharps = d.lines.iter().filter(|l| l.color[2] > 0.9 && l.color[0] < 0.2).count();
        assert_eq!((seams, sharps), (1, 1));
    }

    #[test]
    fn hidden_faces_are_skipped() {
        let mut m = primitives::cube(2.0);
        m.faces[0].hidden = true;
        let d = build_draw_data(&m, &BuildOptions::default());
        assert_eq!(d.solid.len(), 10 * 3); // 5 quads restantes = 10 triângulos
    }

    // ------------------------------------------------------------ normais e canais

    fn normal_lines(d: &MeshDrawData) -> Vec<&LineInstance> {
        d.lines
            .iter()
            .filter(|l| l.color == NORMAL_AUTO_COLOR || l.color == NORMAL_CUSTOM_COLOR)
            .collect()
    }

    fn overlay(length: f32) -> Option<NormalOverlay> {
        Some(NormalOverlay { length, selected_only: false })
    }

    #[test]
    fn flat_cube_draws_one_normal_per_corner_along_face_normals() {
        let m = primitives::cube(2.0);
        let d = build_draw_data(&m, &BuildOptions { show_normals: overlay(0.5), ..Default::default() });
        let lines = normal_lines(&d);
        assert_eq!(lines.len(), 24, "6 faces x 4 corners, todas distintas num cubo chapado");
        for l in lines {
            let (a, b) = (Vec3::from_array(l.a), Vec3::from_array(l.b));
            assert!(((b - a).length() - 0.5).abs() < 1e-5);
            // a normal de um cubo aponta ao longo de um eixo e para fora (a posição do corner tem o mesmo sinal)
            let dir = (b - a).normalize();
            assert!((dir.abs().max_element() - 1.0).abs() < 1e-5);
            assert!(a.dot(dir) > 0.0);
            assert_eq!(l.color, NORMAL_AUTO_COLOR);
        }
    }

    #[test]
    fn smooth_groups_share_one_normal_line() {
        let mut m = primitives::cube(2.0);
        for f in &mut m.faces {
            f.smooth = true;
        }
        let params = NormalParams { auto_smooth_angle: None, ..NormalParams::default() };
        let d = build_draw_data(&m, &BuildOptions { show_normals: overlay(0.3), normals: params, ..Default::default() });
        assert_eq!(normal_lines(&d).len(), 8, "sem corte por ângulo: uma normal por vértice");
        let hard = build_draw_data(&m, &BuildOptions { show_normals: overlay(0.3), ..Default::default() });
        assert_eq!(normal_lines(&hard).len(), 24, "30° mantém as quinas de 90° duras");
    }

    #[test]
    fn custom_normals_have_their_own_color_and_direction() {
        let mut m = primitives::cube(2.0);
        let custom = Vec3::new(1.0, 1.0, 0.0).normalize();
        m.faces[0].corners[0].normal = Some(custom);
        let d = build_draw_data(&m, &BuildOptions { show_normals: overlay(1.0), ..Default::default() });
        let lines = normal_lines(&d);
        let customs: Vec<_> = lines.iter().filter(|l| l.color == NORMAL_CUSTOM_COLOR).collect();
        assert_eq!(customs.len(), 1);
        assert_eq!(lines.len() - customs.len(), 23);
        let dir = (Vec3::from_array(customs[0].b) - Vec3::from_array(customs[0].a)).normalize();
        assert!((dir - custom).length() < 1e-5);
    }

    #[test]
    fn normal_overlay_off_by_default_and_respects_hidden_and_selection() {
        let mut m = primitives::cube(2.0);
        let d = build_draw_data(&m, &BuildOptions::default());
        assert!(normal_lines(&d).is_empty());
        // vértice 0 selecionado + só selecionados: apenas os corners daquele vértice (3 faces -> 3 normais)
        let sel = Selection::from_verts([0u32]);
        let d = build_draw_data(
            &m,
            &BuildOptions {
                edit_mode: true,
                selection: Some(&sel),
                show_normals: Some(NormalOverlay { length: 0.4, selected_only: true }),
                ..Default::default()
            },
        );
        assert_eq!(normal_lines(&d).len(), 3);
        m.faces[0].hidden = true;
        let d = build_draw_data(&m, &BuildOptions { show_normals: overlay(0.4), ..Default::default() });
        assert_eq!(normal_lines(&d).len(), 20);
        // comprimento inválido não gera nada
        let d = build_draw_data(&m, &BuildOptions { show_normals: overlay(f32::NAN), ..Default::default() });
        assert!(normal_lines(&d).is_empty());
    }

    #[test]
    fn heat_color_maps_endpoints_and_is_monotone_in_red() {
        let lo = heat_color(0.0);
        let hi = heat_color(1.0);
        assert!(lo[2] > lo[0] && lo[2] > 0.5, "0 = azul");
        assert!(hi[0] > hi[2] && hi[0] > 0.7, "1 = vermelho");
        assert_eq!(heat_color(-3.0), lo);
        assert_eq!(heat_color(7.0), hi);
        assert_eq!(heat_color(f32::NAN), lo);
        assert!(heat_color(0.5)[1] > 0.6, "meio = verde");
    }

    #[test]
    fn channel_view_colors_each_vertex_by_the_selected_channel() {
        use VertexChannel::*;
        let vc = [0.0, 0.5, 1.0, 0.25];
        assert_eq!(channel_view_color(vc, AmbientOcclusion), heat_color(0.0));
        assert_eq!(channel_view_color(vc, ShadowShift), heat_color(0.5));
        assert_eq!(channel_view_color(vc, OutlineWidth), heat_color(1.0));
        assert_eq!(channel_view_color(vc, SpecRimMask), heat_color(0.25));

        let mut m = primitives::cube(2.0);
        m.verts[0].color[VertexChannel::AmbientOcclusion.index()] = 0.0;
        let d = build_draw_data(&m, &BuildOptions { channel_view: Some(AmbientOcclusion), ..Default::default() });
        let red = heat_color(1.0);
        let blue = heat_color(0.0);
        assert!(d.solid.iter().any(|v| v.color == blue && Vec3::from_array(v.pos) == m.verts[0].co));
        assert!(d.solid.iter().any(|v| v.color == red));
        // sem o modo de visualização, a cor base permanece
        let plain = build_draw_data(&m, &BuildOptions::default());
        assert!(plain.solid.iter().all(|v| v.color == BuildOptions::default().base_color));
    }
}
