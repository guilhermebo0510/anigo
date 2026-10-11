//! Conversão `EditMesh` (+ seleção) → dados de desenho do Workbench (CPU, testável).

use crate::theme;
use anigo_modeling::normals::{compute_corner_normals, NormalParams};
use anigo_modeling::{convert::triangulate_polygon, EditMesh, SelectMode, Selection, Topology};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

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

/// Como [`build_draw_data`], mas o SÓLIDO vem da malha avaliada (pilha de modificadores) e as
/// arestas/pontos de edição ficam na malha base (a "cage"), como no Blender.
pub fn build_draw_data_eval(base: &EditMesh, evaluated: &EditMesh, opts: &BuildOptions) -> MeshDrawData {
    let mut out = build_draw_data(base, opts);
    let solid_only = BuildOptions {
        edit_mode: false,
        selection: None,
        object_wire: false,
        highlight_object: false,
        ..opts.clone()
    };
    out.solid = build_draw_data(evaluated, &solid_only).solid;
    out
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
        let color = if opts.edit_mode && sel.faces.contains(&(fi as u32)) {
            mix(opts.base_color, select_rgba, 0.42)
        } else {
            opts.base_color
        };
        for [a, b, c] in triangulate_polygon(&pts) {
            for i in [a, b, c] {
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
}
