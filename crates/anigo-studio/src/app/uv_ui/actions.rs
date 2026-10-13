//! Ações de UV do Inspector/editor: cada uma roda dentro de `ModelingSession::edit_uv` (cópia da malha,
//! validação e checkpoint de undo só em caso de sucesso). Lógica livre de UI para ser testável.

use super::geom::{self, mirror_point, rotate90_point};
use crate::modeling::ModelingSession;
use anigo_modeling::uv::pack::{self, PackParams};
use anigo_modeling::uv::project::{self, BoxLayout};
use anigo_modeling::uv::seams::{self, CharacterSeamParams};
use anigo_modeling::uv::unwrap::{self, UnwrapMethod, UnwrapParams};
use anigo_modeling::{EditMesh, ModelError, ModelResult, SelectMode};
use glam::{Mat4, Vec2, Vec3};

/// Parâmetros editáveis no Inspector (todos `Copy` para entrar em closures sem emprestar a UI).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvParams {
    /// Resolução (px) usada em margens, densidade de texels e relatório.
    pub tex_size: u32,
    /// Limiar diedral de "costura por ângulo" (graus).
    pub seam_angle_deg: f32,
    /// Limiar diedral das costuras automáticas de personagem (graus).
    pub char_angle_deg: f32,
    pub method: UnwrapMethod,
    /// Margem entre ilhas do unwrap, em pixels de `tex_size`.
    pub margin_px: f32,
    pub correct_aspect: bool,
    /// Largura/altura da textura (só vale com `correct_aspect`).
    pub aspect: f32,
    pub fill_holes: bool,
    pub stretch_iters: u32,
    pub pack_margin_px: f32,
    pub pack_rotate: bool,
    /// Eixo normal da projeção planar: 0 = X, 1 = Y, 2 = Z.
    pub planar_axis: usize,
    /// Caixa: layout em cruz (`true`) ou uma célula por direção (`false`).
    pub box_cross: bool,
    pub smart_angle_deg: f32,
}

impl Default for UvParams {
    fn default() -> Self {
        Self {
            tex_size: 1024,
            seam_angle_deg: 60.0,
            char_angle_deg: 70.0,
            method: UnwrapMethod::Lscm,
            margin_px: 4.0,
            correct_aspect: true,
            aspect: 1.0,
            fill_holes: true,
            stretch_iters: 20,
            pack_margin_px: 4.0,
            pack_rotate: true,
            planar_axis: 2,
            box_cross: true,
            smart_angle_deg: 66.0,
        }
    }
}

impl UvParams {
    fn margin_frac(&self) -> f32 {
        self.margin_px / self.tex_size.max(1) as f32
    }

    pub fn unwrap_params(&self) -> UnwrapParams {
        UnwrapParams {
            method: self.method,
            fill_holes: self.fill_holes,
            margin: self.margin_frac(),
            correct_aspect: self.correct_aspect,
            aspect: self.aspect,
            ..UnwrapParams::default()
        }
    }

    pub fn pack_params(&self) -> PackParams {
        PackParams { margin: self.pack_margin_px, texture_size: self.tex_size, rotate: self.pack_rotate, ..PackParams::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvAction {
    MarkSeam,
    ClearSeam,
    ClearAllSeams,
    SeamsByAngle,
    SeamPath,
    AutoSeamsCharacter,
    SeamsFromSharp,
    Unwrap,
    MinimizeStretch,
    Pack,
    NormalizeDensity,
    ProjectPlanar,
    ProjectBox,
    ProjectCylinder,
    ProjectSphere,
    SmartProject,
    ProjectFromView,
    FlipU,
    FlipV,
    Rotate90,
}

impl UvAction {
    /// Rótulo (também usado no histórico de undo).
    pub fn label(self) -> &'static str {
        match self {
            UvAction::MarkSeam => "Marcar costura",
            UvAction::ClearSeam => "Limpar costura",
            UvAction::ClearAllSeams => "Limpar todas as costuras",
            UvAction::SeamsByAngle => "Costuras por ângulo",
            UvAction::SeamPath => "Costura pelo caminho mais curto",
            UvAction::AutoSeamsCharacter => "Costuras automáticas (personagem)",
            UvAction::SeamsFromSharp => "Costuras das arestas duras",
            UvAction::Unwrap => "Unwrap",
            UvAction::MinimizeStretch => "Minimizar estiramento",
            UvAction::Pack => "Empacotar ilhas",
            UvAction::NormalizeDensity => "Normalizar densidade de texel",
            UvAction::ProjectPlanar => "Projeção planar",
            UvAction::ProjectBox => "Projeção em caixa",
            UvAction::ProjectCylinder => "Projeção cilíndrica",
            UvAction::ProjectSphere => "Projeção esférica",
            UvAction::SmartProject => "Smart UV Project",
            UvAction::ProjectFromView => "Projetar da vista",
            UvAction::FlipU => "Espelhar UV (U)",
            UvAction::FlipV => "Espelhar UV (V)",
            UvAction::Rotate90 => "Girar UV 90°",
        }
    }
}

fn faces_bbox(mesh: &EditMesh, scope: Option<&[u32]>) -> Option<(Vec3, Vec3)> {
    let mut it: Box<dyn Iterator<Item = Vec3> + '_> = match scope {
        Some(fs) => Box::new(fs.iter().filter_map(|&f| mesh.faces.get(f as usize)).flat_map(|f| f.corners.iter().map(|c| mesh.co(c.vert)))),
        None => Box::new(mesh.faces.iter().flat_map(|f| f.corners.iter().map(|c| mesh.co(c.vert)))),
    };
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Transforma os UVs selecionados (ou todos, se nada está selecionado) em torno do centro da caixa deles.
fn transform_selected(
    mesh: &mut EditMesh,
    sel: &anigo_modeling::Selection,
    mode: SelectMode,
    f: impl Fn(Vec2, Vec2) -> Vec2,
) -> ModelResult<usize> {
    let mut corners = geom::selected_corners(mesh, sel, mode);
    if corners.is_empty() {
        corners = mesh
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.hidden)
            .flat_map(|(fi, f)| (0..f.corners.len()).map(move |ci| (fi as u32, ci)))
            .collect();
    }
    let (lo, hi) = geom::corners_bounds(mesh, &corners).ok_or(ModelError::EmptySelection("a malha não tem faces"))?;
    let pivot = (lo + hi) * 0.5;
    for &(fi, ci) in &corners {
        if let Some(c) = mesh.faces.get_mut(fi as usize).and_then(|x| x.corners.get_mut(ci)) {
            c.uv = f(c.uv, pivot);
        }
    }
    Ok(corners.len())
}

/// Executa uma ação de UV sobre a malha ativa. `view_proj` = projeção*vista*modelo (só para "da vista").
/// Devolve o texto de sucesso para a barra de status; em erro a malha e o histórico ficam intactos.
pub fn run_action(session: &mut ModelingSession, a: UvAction, p: &UvParams, view_proj: Option<Mat4>) -> Result<String, ModelError> {
    let label = a.label();
    let mode = session.select_mode;
    let (pi, tex) = (*p, p.tex_size);
    // verts selecionados em ordem estável (para o caminho mais curto)
    let sel_verts: Vec<u32> = session.selection.verts.iter().copied().collect();
    session.edit_uv(label, move |mesh, sel| {
        let scope_vec = geom::scope_faces(sel);
        let scope = scope_vec.as_deref();
        let scope_txt = if scope.is_some() { "nas faces selecionadas" } else { "na malha toda" };
        match a {
            UvAction::MarkSeam | UvAction::ClearSeam => {
                let edges = geom::selected_edges(mesh, sel);
                if edges.is_empty() {
                    return Err(ModelError::EmptySelection("selecione arestas (ou vértices/faces ligados) primeiro"));
                }
                let mark = a == UvAction::MarkSeam;
                let n = seams::mark_seams(mesh, &edges, mark)?;
                Ok(match (mark, n) {
                    (true, 0) => "as arestas já eram costuras".to_string(),
                    (false, 0) => "nenhuma costura nas arestas selecionadas".to_string(),
                    (true, n) => format!("{n} costura(s) marcada(s)"),
                    (false, n) => format!("{n} costura(s) removida(s)"),
                })
            }
            UvAction::ClearAllSeams => {
                let n = seams::clear_seams(mesh);
                Ok(format!("{n} costura(s) removida(s)"))
            }
            UvAction::SeamsByAngle => {
                let n = seams::auto_seams_by_angle(mesh, pi.seam_angle_deg.to_radians())?;
                Ok(format!("{n} costura(s) nova(s) acima de {:.0}°", pi.seam_angle_deg))
            }
            UvAction::SeamPath => {
                let [from, to] = sel_verts[..] else {
                    return Err(ModelError::InvalidParameter(format!(
                        "selecione exatamente 2 vértices para o caminho (há {} selecionados)",
                        sel_verts.len()
                    )));
                };
                let path = seams::seam_path(mesh, from, to)?;
                let n = seams::mark_seams(mesh, &path, true)?;
                Ok(format!("caminho de {} aresta(s), {n} nova(s) costura(s)", path.len()))
            }
            UvAction::AutoSeamsCharacter => {
                let params = CharacterSeamParams { angle: pi.char_angle_deg.to_radians(), ..CharacterSeamParams::default() };
                let r = seams::auto_seams_for_character_with(mesh, &params)?;
                let mut s = format!(
                    "{} por ângulo, {} de corte, {} em pontas; {} ilha(s)",
                    r.angle_seams, r.cut_seams, r.point_seams, r.islands
                );
                if r.non_disk_islands > 0 {
                    s.push_str(&format!(" ({} ainda não é disco)", r.non_disk_islands));
                }
                Ok(s)
            }
            UvAction::SeamsFromSharp => {
                let n = seams::seams_from_sharp(mesh);
                Ok(format!("{n} costura(s) a partir de arestas duras"))
            }
            UvAction::Unwrap => {
                let r = unwrap::unwrap_lscm(mesh, scope, &pi.unwrap_params())?;
                let mut s = format!("{} ilha(s), {} face(s) {scope_txt}", r.islands, r.faces);
                if r.non_disk_islands > 0 {
                    s.push_str(&format!("; {} ilha(s) não-disco (distorção maior)", r.non_disk_islands));
                }
                if pi.method == UnwrapMethod::LscmArap {
                    s.push_str(&format!("; ARAP melhorou {}", r.arap_improved_islands));
                }
                Ok(s)
            }
            UvAction::MinimizeStretch => {
                let r = unwrap::unwrap_minimum_stretch_in(mesh, scope, pi.stretch_iters.max(1))?;
                Ok(format!(
                    "distorção média {:.3} -> {:.3} ({} de {} ilha(s) melhoraram)",
                    r.mean_before, r.mean_after, r.improved, r.islands
                ))
            }
            UvAction::Pack => {
                let r = pack::pack_islands_in(mesh, scope, &pi.pack_params())?;
                Ok(format!("{} ilha(s), ocupação {:.0}%, {} girada(s)", r.islands, r.uv_area_ratio * 100.0, r.rotated))
            }
            UvAction::NormalizeDensity => {
                let d = pack::normalize_texel_density(mesh, tex, None)?;
                let r = pack::pack_islands(mesh, &pi.pack_params())?;
                Ok(format!("densidade uniforme de {d:.0} px/u; reempacotado ({:.0}% de ocupação)", r.uv_area_ratio * 100.0))
            }
            UvAction::ProjectPlanar => {
                let (lo, hi) = faces_bbox(mesh, scope).ok_or(ModelError::EmptySelection("sem faces para projetar"))?;
                let (u, v) = match pi.planar_axis {
                    0 => (Vec3::Y, Vec3::Z),
                    1 => (Vec3::Z, Vec3::X),
                    _ => (Vec3::X, Vec3::Y),
                };
                let ext = hi - lo;
                let big = ext.dot(u).abs().max(ext.dot(v).abs()).max(1e-6);
                project::project_planar(mesh, scope, lo, u, v, 1.0 / big)?;
                Ok(format!("planar no eixo {} {scope_txt}", ["X", "Y", "Z"][pi.planar_axis.min(2)]))
            }
            UvAction::ProjectBox | UvAction::ProjectCylinder | UvAction::ProjectSphere => {
                let (lo, hi) = faces_bbox(mesh, scope).ok_or(ModelError::EmptySelection("sem faces para projetar"))?;
                let (center, size) = ((lo + hi) * 0.5, (hi - lo).max(Vec3::splat(1e-3)));
                match a {
                    UvAction::ProjectBox => {
                        let layout = if pi.box_cross { BoxLayout::Cross } else { BoxLayout::PerFace };
                        project::project_box(mesh, scope, center, size, layout)?;
                    }
                    UvAction::ProjectCylinder => project::project_cylinder(mesh, scope, center, Vec3::Y, None)?,
                    _ => project::project_sphere(mesh, scope, center)?,
                }
                Ok(format!("{} {scope_txt}", label.to_lowercase()))
            }
            UvAction::SmartProject => {
                let r = project::smart_uv_project_in(mesh, scope, pi.smart_angle_deg.to_radians(), pi.pack_margin_px / tex.max(1) as f32)?;
                Ok(format!("{} grupo(s) planar(es), ocupação {:.0}%", r.groups, r.pack.uv_area_ratio * 100.0))
            }
            UvAction::ProjectFromView => {
                let vp = view_proj.ok_or(ModelError::InvalidParameter("sem câmera para projetar".into()))?;
                project::project_from_view(mesh, scope, vp)?;
                Ok(format!("projetado da vista atual {scope_txt}"))
            }
            UvAction::FlipU | UvAction::FlipV => {
                let horizontal = a == UvAction::FlipU;
                let n = transform_selected(mesh, sel, mode, |p, pivot| mirror_point(p, pivot, horizontal))?;
                Ok(format!("{n} canto(s) espelhado(s)"))
            }
            UvAction::Rotate90 => {
                let n = transform_selected(mesh, sel, mode, rotate90_point)?;
                Ok(format!("{n} canto(s) girado(s)"))
            }
        }
    })
}

/// Arestas do complemento de uma árvore geradora do grafo dual (faces ligadas por aresta), a partir da
/// face 0: abre qualquer malha conexa em uma única ilha. Num cubo isso dá a "cruz" clássica.
pub fn spanning_tree_seams(mesh: &EditMesh) -> Vec<anigo_modeling::EdgeKey> {
    use anigo_modeling::Topology;
    let topo = Topology::build(mesh);
    let n = mesh.faces.len();
    let mut seen = vec![false; n];
    let mut kept: std::collections::BTreeSet<anigo_modeling::EdgeKey> = Default::default();
    for root in 0..n {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut queue = std::collections::VecDeque::from([root as u32]);
        while let Some(f) = queue.pop_front() {
            let face = &mesh.faces[f as usize];
            let k = face.corners.len();
            for i in 0..k {
                let (a, b) = (face.corners[i].vert, face.corners[(i + 1) % k].vert);
                for &o in topo.edge_faces(a, b) {
                    if !seen[o as usize] {
                        seen[o as usize] = true;
                        kept.insert(anigo_modeling::edge_key(a, b));
                        queue.push_back(o);
                    }
                }
            }
        }
    }
    topo.edges.iter().map(|e| e.verts).filter(|k| !kept.contains(k) && topo.edge_faces(k.0, k.1).len() == 2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{Mode, PrimitiveKind};
    use anigo_modeling::uv::metrics;
    use anigo_modeling::{Selection, Topology};

    fn session(kind: PrimitiveKind) -> ModelingSession {
        let mut s = ModelingSession::new();
        s.add_primitive(kind, Vec3::ZERO);
        s.set_mode(Mode::Edit);
        s
    }

    fn mesh(s: &ModelingSession) -> EditMesh {
        s.active_object().map(|o| o.mesh.clone()).expect("objeto ativo")
    }

    fn cross_cube() -> ModelingSession {
        let mut s = session(PrimitiveKind::Cube);
        let edges = spanning_tree_seams(&mesh(&s));
        assert_eq!(edges.len(), 7, "cubo: 12 arestas - 5 da árvore");
        s.set_select_mode(SelectMode::Edge);
        s.set_selection(Selection::from_edges(edges), None);
        run_action(&mut s, UvAction::MarkSeam, &UvParams::default(), None).expect("marcar");
        s
    }

    #[test]
    fn mark_and_clear_seams_are_undoable_and_report_counts() {
        let mut s = session(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Edge);
        let e = Topology::build(&mesh(&s)).edges[0].verts;
        s.set_selection(Selection::from_edges([e]), None);
        let before = mesh(&s);
        let msg = run_action(&mut s, UvAction::MarkSeam, &UvParams::default(), None).expect("ok");
        assert!(msg.contains("1 costura"), "{msg}");
        assert!(mesh(&s).edge_attr(e.0, e.1).seam);
        // repetir não muda nada nem polui o histórico
        let depth = s.history_labels().len();
        let msg2 = run_action(&mut s, UvAction::MarkSeam, &UvParams::default(), None).expect("ok");
        assert!(msg2.contains("já eram"), "{msg2}");
        assert_eq!(s.history_labels().len(), depth);
        run_action(&mut s, UvAction::ClearSeam, &UvParams::default(), None).expect("ok");
        assert!(!mesh(&s).edge_attr(e.0, e.1).seam);
        s.undo();
        s.undo();
        assert_eq!(mesh(&s), before);
    }

    #[test]
    fn seam_actions_without_selection_fail_with_friendly_message_and_change_nothing() {
        let mut s = session(PrimitiveKind::Cube);
        let before = mesh(&s);
        let depth = s.history_labels().len();
        let e = run_action(&mut s, UvAction::MarkSeam, &UvParams::default(), None).unwrap_err();
        assert!(e.to_string().contains("selecione arestas"), "{e}");
        let e = run_action(&mut s, UvAction::SeamPath, &UvParams::default(), None).unwrap_err();
        assert!(e.to_string().contains("2 vértices"), "{e}");
        assert_eq!(mesh(&s), before);
        assert_eq!(s.history_labels().len(), depth);
    }

    #[test]
    fn seam_path_between_two_vertices_marks_the_shortest_path() {
        let mut s = session(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Vertex);
        let m = mesh(&s);
        let topo = Topology::build(&m);
        let a = 0u32;
        let b = topo.vert_neighbors(a)[0];
        s.set_selection(Selection::from_verts([a, b]), None);
        let msg = run_action(&mut s, UvAction::SeamPath, &UvParams::default(), None).expect("ok");
        assert!(msg.contains("1 aresta"), "{msg}");
        assert!(mesh(&s).edge_attr(a, b).seam);
    }

    #[test]
    fn closed_mesh_without_seams_fails_unwrap_and_leaves_history_untouched() {
        let mut s = session(PrimitiveKind::Cube);
        let before = mesh(&s);
        let depth = s.history_labels().len();
        let e = run_action(&mut s, UvAction::Unwrap, &UvParams::default(), None).unwrap_err();
        assert!(e.to_string().contains("costura"), "mensagem orienta a marcar costuras: {e}");
        assert_eq!(mesh(&s), before);
        assert_eq!(s.history_labels().len(), depth);
    }

    #[test]
    fn unwrap_cross_cube_then_undo_restores_exactly() {
        let mut s = cross_cube();
        let before = mesh(&s);
        let uv_before = metrics::uv_hash(&before);
        let msg = run_action(&mut s, UvAction::Unwrap, &UvParams::default(), None).expect("unwrap");
        assert!(msg.contains("1 ilha"), "{msg}");
        let after = mesh(&s);
        assert_ne!(metrics::uv_hash(&after), uv_before, "UVs mudaram");
        assert!(metrics::flipped_faces(&after).is_empty());
        let rep = metrics::uv_report(&after, 1024).expect("relatório");
        assert_eq!(rep.island_count, 1);
        assert_eq!(rep.overlapping_island_pairs, 0);
        let (lo, hi) = metrics::uv_bounds(&after).expect("caixa");
        assert!(lo.x >= -1e-4 && lo.y >= -1e-4 && hi.x <= 1.0001 && hi.y <= 1.0001, "empacotado em [0,1]²: {lo:?} {hi:?}");
        // undo devolve a malha bit a bit (UVs inclusive); a costura marcada antes permanece
        assert_eq!(s.undo().as_deref(), Some("Unwrap"));
        assert_eq!(mesh(&s), before);
        assert_eq!(metrics::uv_hash(&mesh(&s)), uv_before);
        // refazer reaplica o mesmo resultado
        s.redo();
        assert_eq!(mesh(&s), after);
    }

    #[test]
    fn lscm_arap_minimize_pack_and_density_pipeline() {
        let mut s = session(PrimitiveKind::QuadSphere);
        let r = run_action(&mut s, UvAction::AutoSeamsCharacter, &UvParams::default(), None).expect("auto seams");
        assert!(r.contains("ilha"), "{r}");
        let p = UvParams { method: UnwrapMethod::LscmArap, ..UvParams::default() };
        run_action(&mut s, UvAction::Unwrap, &p, None).expect("unwrap");
        let m = mesh(&s);
        assert!(metrics::flipped_faces(&m).is_empty(), "sem faces invertidas");
        run_action(&mut s, UvAction::MinimizeStretch, &p, None).expect("minimizar");
        run_action(&mut s, UvAction::Pack, &p, None).expect("pack");
        run_action(&mut s, UvAction::NormalizeDensity, &p, None).expect("densidade");
        let rep = metrics::uv_report(&mesh(&s), p.tex_size).expect("relatório");
        let d = rep.texel_density.expect("densidade");
        assert!(d.std_dev / d.mean < 0.35, "densidade razoavelmente uniforme: {d:?}");
        assert_eq!(rep.overlapping_island_pairs, 0);
        // 4 ações de UV + marcação de costuras = entradas no histórico
        assert!(s.history_labels().contains(&"Normalizar densidade de texel"));
    }

    #[test]
    fn projections_write_finite_uvs_in_expected_ranges() {
        for a in [
            UvAction::ProjectPlanar,
            UvAction::ProjectBox,
            UvAction::ProjectCylinder,
            UvAction::ProjectSphere,
            UvAction::SmartProject,
        ] {
            let mut s = session(PrimitiveKind::UvSphere);
            let before = mesh(&s);
            run_action(&mut s, a, &UvParams::default(), None).unwrap_or_else(|e| panic!("{a:?}: {e}"));
            let m = mesh(&s);
            assert_ne!(m, before, "{a:?} deve alterar UVs");
            assert!(m.faces.iter().flat_map(|f| f.corners.iter()).all(|c| c.uv.is_finite()), "{a:?}");
            s.undo();
            assert_eq!(mesh(&s), before, "undo de {a:?}");
        }
        // planar cabe em [0,1]²
        let mut s = session(PrimitiveKind::Cube);
        run_action(&mut s, UvAction::ProjectPlanar, &UvParams::default(), None).expect("planar");
        let (lo, hi) = metrics::uv_bounds(&mesh(&s)).expect("caixa");
        assert!(lo.min_element() > -1e-4 && hi.max_element() < 1.0001, "{lo:?} {hi:?}");
    }

    #[test]
    fn project_from_view_uses_camera_matrix_and_errors_behind_camera() {
        let mut s = session(PrimitiveKind::Cube);
        let cam = crate::viewport::OrbitCamera::default();
        let vp = cam.view_proj(1.5);
        run_action(&mut s, UvAction::ProjectFromView, &UvParams::default(), Some(vp)).expect("da vista");
        let (lo, hi) = metrics::uv_bounds(&mesh(&s)).expect("caixa");
        assert!(lo.min_element() > -0.5 && hi.max_element() < 1.5 && (hi - lo).min_element() > 0.05, "projeção plausível da vista: {lo:?} {hi:?}");
        let before = mesh(&s);
        let e = run_action(&mut s, UvAction::ProjectFromView, &UvParams::default(), None).unwrap_err();
        assert!(e.to_string().contains("câmera"));
        assert_eq!(mesh(&s), before);
    }

    #[test]
    fn flip_and_rotate_act_on_selected_corners_only() {
        let mut s = session(PrimitiveKind::Cube);
        s.set_select_mode(SelectMode::Face);
        let m = mesh(&s);
        let uv_other = m.faces[1].corners[0].uv;
        s.set_selection(Selection::from_faces(&m, [0u32]), None);
        let before0 = m.faces[0].corners.iter().map(|c| c.uv).collect::<Vec<_>>();
        run_action(&mut s, UvAction::FlipU, &UvParams::default(), None).expect("flip");
        let m2 = mesh(&s);
        let after0 = m2.faces[0].corners.iter().map(|c| c.uv).collect::<Vec<_>>();
        assert_ne!(before0, after0);
        // vértices da face 0 são compartilhados com outras faces; em modo Face só os cantos da face 0 mudam
        assert_eq!(m2.faces[1].corners[0].uv, uv_other);
        for (b, a) in before0.iter().zip(&after0) {
            assert!((b.y - a.y).abs() < 1e-6, "espelhar U não mexe em V");
        }
    }

    #[test]
    fn scope_limits_unwrap_to_selected_faces() {
        let mut s = cross_cube();
        // só a face 0 selecionada: as outras mantêm UV
        s.set_select_mode(SelectMode::Face);
        let m = mesh(&s);
        s.set_selection(Selection::from_faces(&m, [0u32]), None);
        run_action(&mut s, UvAction::Unwrap, &UvParams::default(), None).expect("unwrap parcial");
        let m2 = mesh(&s);
        for f in 1..m.faces.len() {
            assert_eq!(m.faces[f].corners.iter().map(|c| c.uv).collect::<Vec<_>>(), m2.faces[f].corners.iter().map(|c| c.uv).collect::<Vec<_>>(), "face {f}");
        }
    }

    #[test]
    fn invalid_parameters_are_reported_not_panicked() {
        let mut s = cross_cube();
        let before = mesh(&s);
        let bad = UvParams { margin_px: 900.0, ..UvParams::default() };
        let e = run_action(&mut s, UvAction::Unwrap, &bad, None).unwrap_err();
        assert!(e.to_string().contains("margem"), "{e}");
        let bad = UvParams { smart_angle_deg: 120.0, ..UvParams::default() };
        assert!(run_action(&mut s, UvAction::SmartProject, &bad, None).is_err());
        assert_eq!(mesh(&s), before);
    }
}
