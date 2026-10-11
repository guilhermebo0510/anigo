//! Auto-mascaramento: fatores por vértice (0..1) que limitam onde um pincel
//! age, derivados do estado da malha e do início do traço.
//!
//! Fatores disponíveis (multiplicam-se):
//! * máscara de escultura (`EVert::mask`, 1 = protegido);
//! * limite de normal (ângulo entre a normal do vértice e a do início do traço);
//! * face set (do face set informado ou o da face sob o início do traço) e
//!   face sets protegidos (`exclude_face_sets`);
//! * cavidade (curvatura por laplaciano normalizado);
//! * topologia (conectado ao vértice inicial; opcionalmente até uma distância
//!   geodésica sobre as arestas);
//! * fronteira (vértices de borda aberta ficam protegidos).
//!
//! Os fatores que dependem do traço (normal, face set, topologia) precisam que
//! [`AutoMask::begin_stroke`] tenha rodado (o `StrokeState` e o `apply_dab` o
//! fazem); sem isso eles são ignorados.

use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::sculpt::brush::SculptContext;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Parâmetros do auto-mask de cavidade.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CavityParams {
    /// Sensibilidade (> 0). 1 = padrão; maior detecta cavidades mais rasas.
    pub strength: f32,
    /// `false`: o pincel age só em cavidades (côncavo); `true`: só em saliências (convexo).
    pub invert: bool,
}

impl Default for CavityParams {
    fn default() -> Self {
        Self {
            strength: 1.0,
            invert: false,
        }
    }
}

/// Configuração de auto-máscara de um pincel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoMask {
    /// Inclui `(1 - mask)` em [`AutoMask::eval_vertex`]. (O pincel sempre
    /// respeita a máscara de escultura, independentemente deste campo.)
    pub use_mask: bool,
    /// Limite de ângulo (radianos, `(0, π]`) entre a normal do vértice e a
    /// normal do início do traço; além dele o fator é 0.
    pub normal_limit: Option<f32>,
    /// Largura (radianos) da transição suave que termina em `normal_limit`.
    pub normal_falloff: f32,
    /// Restringe a vértices de faces com este face set.
    pub face_set: Option<i32>,
    /// Usa o face set da face sob o início do traço (ignorado se `face_set` é `Some`).
    pub face_set_from_stroke: bool,
    /// Face sets protegidos: um vértice cujas faces pertencem **todas** a
    /// estes conjuntos não é afetado (o interior de uma região travada fica
    /// fixo; a fronteira com conjuntos livres continua editável).
    pub exclude_face_sets: Vec<i32>,
    pub cavity: Option<CavityParams>,
    /// Restringe ao componente conectado do vértice inicial.
    pub topology_connected: bool,
    /// Com `topology_connected`: distância geodésica máxima (sobre arestas) a
    /// partir do vértice inicial. `None` = componente inteiro.
    pub topology_distance: Option<f32>,
    /// Protege vértices de fronteira aberta (bordas de cortes e roupas).
    pub boundary: bool,
}

impl Default for AutoMask {
    fn default() -> Self {
        Self {
            use_mask: true,
            normal_limit: None,
            normal_falloff: 0.0,
            face_set: None,
            face_set_from_stroke: false,
            exclude_face_sets: Vec::new(),
            cavity: None,
            topology_connected: false,
            topology_distance: None,
            boundary: false,
        }
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl AutoMask {
    /// Valida os parâmetros.
    pub fn validate(&self) -> ModelResult<()> {
        if let Some(l) = self.normal_limit {
            if !(l > 0.0 && l <= std::f32::consts::PI) {
                return Err(ModelError::InvalidParameter(format!(
                    "normal_limit deve estar em (0, π]: {l}"
                )));
            }
        }
        if !self.normal_falloff.is_finite() || self.normal_falloff < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "normal_falloff inválido: {}",
                self.normal_falloff
            )));
        }
        if let Some(c) = &self.cavity {
            if !(c.strength > 0.0) || !c.strength.is_finite() {
                return Err(ModelError::InvalidParameter(format!(
                    "força de cavidade deve ser > 0: {}",
                    c.strength
                )));
            }
        }
        if let Some(d) = self.topology_distance {
            if !(d > 0.0) || !d.is_finite() {
                return Err(ModelError::InvalidParameter(format!(
                    "topology_distance deve ser > 0: {d}"
                )));
            }
        }
        Ok(())
    }

    /// `true` quando nenhum fator além da máscara está ligado (caminho rápido).
    pub fn is_trivial(&self) -> bool {
        self.normal_limit.is_none()
            && self.face_set.is_none()
            && !self.face_set_from_stroke
            && self.exclude_face_sets.is_empty()
            && self.cavity.is_none()
            && !self.topology_connected
            && !self.boundary
    }

    /// Prepara o estado do traço em `ctx` (normal inicial, face set e
    /// distâncias topológicas). `start_vert` é o vértice mais próximo do
    /// primeiro dab; `start_face` a face atingida.
    pub fn begin_stroke(
        &self,
        mesh: &EditMesh,
        ctx: &mut SculptContext,
        start_vert: Option<u32>,
        start_normal: Vec3,
        start_face: Option<u32>,
    ) {
        ctx.stroke.start_vert = start_vert;
        ctx.stroke.start_normal = Some(start_normal.normalize_or_zero()).filter(|n| *n != Vec3::ZERO);
        ctx.stroke.start_face_set = None;
        ctx.stroke.topo_dist.clear();
        if let Some(id) = self.face_set {
            ctx.stroke.start_face_set = Some(id);
        } else if self.face_set_from_stroke {
            ctx.stroke.start_face_set = start_face
                .and_then(|f| mesh.faces.get(f as usize))
                .map(|f| f.face_set);
        }
        if self.topology_connected {
            if let Some(s) = start_vert {
                ctx.stroke.topo_dist = geodesic_distances(mesh, ctx, s, self.topology_distance);
            }
        }
    }

    /// Fator final do vértice, `[0, 1]`, incluindo `(1 - mask)` se `use_mask`.
    pub fn eval_vertex(&self, mesh: &EditMesh, ctx: &SculptContext, v: u32) -> f32 {
        let f = self.eval_vertex_no_mask(mesh, ctx, v);
        if self.use_mask && f > 0.0 {
            f * (1.0 - mesh.verts.get(v as usize).map_or(0.0, |x| x.mask)).clamp(0.0, 1.0)
        } else {
            f
        }
    }

    /// Como [`Self::eval_vertex`], mas sem o termo da máscara (usado pelos
    /// pincéis de máscara e pelo `apply_dab`, que aplica a máscara à parte).
    pub fn eval_vertex_no_mask(&self, mesh: &EditMesh, ctx: &SculptContext, v: u32) -> f32 {
        if self.is_trivial() {
            return 1.0;
        }
        let vi = v as usize;
        if vi >= mesh.verts.len() || vi >= ctx.vert_normals.len() {
            return 0.0;
        }
        let mut f = 1.0f32;

        if self.boundary && ctx.boundary[vi] {
            return 0.0;
        }
        if let Some(limit) = self.normal_limit {
            if let Some(n0) = ctx.stroke.start_normal {
                let angle = ctx.vert_normals[vi].dot(n0).clamp(-1.0, 1.0).acos();
                if angle >= limit {
                    return 0.0;
                }
                if self.normal_falloff > 0.0 {
                    f *= smoothstep((limit - angle) / self.normal_falloff);
                }
            }
        }
        if !self.exclude_face_sets.is_empty() {
            let faces = &ctx.topo.vert_faces[vi];
            if !faces.is_empty()
                && faces
                    .iter()
                    .all(|&fi| self.exclude_face_sets.contains(&mesh.faces[fi as usize].face_set))
            {
                return 0.0;
            }
        }
        if let Some(id) = ctx.stroke.start_face_set {
            if self.face_set.is_some() || self.face_set_from_stroke {
                let has = ctx.topo.vert_faces[vi]
                    .iter()
                    .any(|&fi| mesh.faces[fi as usize].face_set == id);
                if !has {
                    return 0.0;
                }
            }
        }
        if self.topology_connected && !ctx.stroke.topo_dist.is_empty() {
            let d = ctx.stroke.topo_dist.get(vi).copied().unwrap_or(f32::INFINITY);
            if !d.is_finite() {
                return 0.0;
            }
        }
        if let Some(c) = &self.cavity {
            f *= cavity_factor(mesh, ctx, v, c);
        }
        f.clamp(0.0, 1.0)
    }
}

/// Fator de cavidade de um vértice. Sinal = componente do laplaciano ao longo
/// da normal, normalizada pelo comprimento médio das arestas: positivo =
/// côncavo (o vértice está abaixo da média dos vizinhos).
pub fn cavity_signal(mesh: &EditMesh, ctx: &SculptContext, v: u32) -> f32 {
    let nb = ctx.neighbors(v);
    if nb.is_empty() {
        return 0.0;
    }
    let co = mesh.verts[v as usize].co;
    let mut avg = Vec3::ZERO;
    let mut len = 0.0;
    for &u in nb {
        let p = mesh.verts[u as usize].co;
        avg += p;
        len += p.distance(co);
    }
    avg /= nb.len() as f32;
    len /= nb.len() as f32;
    if len <= 1e-12 {
        return 0.0;
    }
    (avg - co).dot(ctx.vert_normals[v as usize]) / len
}

fn cavity_factor(mesh: &EditMesh, ctx: &SculptContext, v: u32, c: &CavityParams) -> f32 {
    // 8: escala empírica — uma cavidade moderada (curvatura típica de uma
    // dobra de tecido) dá sinal ~0.1–0.2 e deve saturar o fator.
    let s = cavity_signal(mesh, ctx, v) * c.strength * 8.0;
    smoothstep(if c.invert { -s } else { s })
}

/// Distâncias geodésicas (soma de comprimentos de aresta) a partir de `start`
/// por Dijkstra; `INFINITY` para vértices inalcançáveis ou além de `max_dist`.
pub fn geodesic_distances(mesh: &EditMesh, ctx: &SculptContext, start: u32, max_dist: Option<f32>) -> Vec<f32> {
    let n = mesh.verts.len();
    let mut dist = vec![f32::INFINITY; n];
    if start as usize >= n {
        return dist;
    }
    let limit = max_dist.unwrap_or(f32::INFINITY);
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    dist[start as usize] = 0.0;
    heap.push(Reverse((0f32.to_bits(), start)));
    while let Some(Reverse((bits, u))) = heap.pop() {
        let d = f32::from_bits(bits);
        if d > dist[u as usize] {
            continue;
        }
        let pu = mesh.verts[u as usize].co;
        for &w in ctx.neighbors(u) {
            let nd = d + pu.distance(mesh.verts[w as usize].co);
            if nd <= limit && nd < dist[w as usize] {
                dist[w as usize] = nd;
                // floats não negativos preservam a ordem como u32
                heap.push(Reverse((nd.to_bits(), w)));
            }
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::sculpt::brush::Brush;
    use crate::sculpt::brush::BrushKind;

    fn setup(mesh: &EditMesh) -> SculptContext {
        SculptContext::new(mesh).unwrap()
    }

    #[test]
    fn default_is_trivial_and_returns_one_or_mask() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let ctx = setup(&m);
        let a = AutoMask::default();
        assert!(a.is_trivial());
        assert_eq!(a.eval_vertex_no_mask(&m, &ctx, 3), 1.0);
        assert_eq!(a.eval_vertex(&m, &ctx, 3), 1.0);
        m.verts[3].mask = 0.25;
        assert!((a.eval_vertex(&m, &ctx, 3) - 0.75).abs() < 1e-6);
        assert_eq!(a.eval_vertex_no_mask(&m, &ctx, 3), 1.0);
        let off = AutoMask {
            use_mask: false,
            ..Default::default()
        };
        assert_eq!(off.eval_vertex(&m, &ctx, 3), 1.0);
        // índice inválido não entra em pânico quando há fatores ligados
        let b = AutoMask {
            boundary: true,
            ..Default::default()
        };
        assert_eq!(b.eval_vertex(&m, &ctx, 9999), 0.0);
    }

    #[test]
    fn validate_rejects_bad_parameters() {
        assert!(AutoMask::default().validate().is_ok());
        for bad in [
            AutoMask {
                normal_limit: Some(0.0),
                ..Default::default()
            },
            AutoMask {
                normal_limit: Some(4.0),
                ..Default::default()
            },
            AutoMask {
                normal_falloff: -1.0,
                ..Default::default()
            },
            AutoMask {
                normal_falloff: f32::NAN,
                ..Default::default()
            },
            AutoMask {
                cavity: Some(CavityParams {
                    strength: 0.0,
                    invert: false,
                }),
                ..Default::default()
            },
            AutoMask {
                topology_distance: Some(-1.0),
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn normal_limit_masks_by_angle_with_falloff() {
        let m = primitives::quad_sphere(1.0, 8);
        let mut ctx = setup(&m);
        let top = (0..m.vert_count() as u32)
            .max_by(|&a, &b| m.co(a).y.total_cmp(&m.co(b).y))
            .unwrap();
        let a = AutoMask {
            normal_limit: Some(60f32.to_radians()),
            normal_falloff: 20f32.to_radians(),
            ..Default::default()
        };
        a.begin_stroke(&m, &mut ctx, Some(top), Vec3::Y, None);
        assert!((a.eval_vertex(&m, &ctx, top) - 1.0).abs() < 1e-5);
        let bottom = (0..m.vert_count() as u32)
            .min_by(|&a, &b| m.co(a).y.total_cmp(&m.co(b).y))
            .unwrap();
        assert_eq!(a.eval_vertex(&m, &ctx, bottom), 0.0);
        // plateau até limite - falloff, zero a partir do limite, sempre em [0, 1]
        for v in (0..m.vert_count() as u32).step_by(3) {
            let f = a.eval_vertex(&m, &ctx, v);
            let ang = ctx.vert_normals[v as usize].dot(Vec3::Y).clamp(-1.0, 1.0).acos();
            if ang <= 40f32.to_radians() {
                assert!((f - 1.0).abs() < 1e-5, "dentro do plateau (ang {ang})");
            }
            if ang >= 60f32.to_radians() {
                assert_eq!(f, 0.0);
            }
            assert!((0.0..=1.0).contains(&f));
        }
        // a transição é monótona em função do ângulo
        let mut samples: Vec<(f32, f32)> = (0..m.vert_count() as u32)
            .map(|v| {
                let ang = ctx.vert_normals[v as usize].dot(Vec3::Y).clamp(-1.0, 1.0).acos();
                (ang, a.eval_vertex(&m, &ctx, v))
            })
            .collect();
        samples.sort_by(|x, y| x.0.total_cmp(&y.0));
        assert!(samples.windows(2).all(|w| w[1].1 <= w[0].1 + 1e-5));
    }

    #[test]
    fn normal_limit_without_stroke_is_ignored() {
        let m = primitives::quad_sphere(1.0, 4);
        let ctx = setup(&m);
        let a = AutoMask {
            normal_limit: Some(0.1),
            ..Default::default()
        };
        assert_eq!(a.eval_vertex(&m, &ctx, 0), 1.0);
    }

    #[test]
    fn face_set_restricts_to_set_vertices() {
        let mut m = primitives::quad_sphere(1.0, 4);
        let topo = crate::topology::Topology::build(&m);
        // face set 5 nas faces com y > 0.3
        for f in 0..m.face_count() as u32 {
            if m.face_center(f).y > 0.3 {
                m.faces[f as usize].face_set = 5;
            }
        }
        let mut ctx = SculptContext::new(&m).unwrap();
        let a = AutoMask {
            face_set: Some(5),
            ..Default::default()
        };
        a.begin_stroke(&m, &mut ctx, None, Vec3::Y, None);
        for v in 0..m.vert_count() as u32 {
            let touches = topo.vert_faces[v as usize].iter().any(|&f| m.faces[f as usize].face_set == 5);
            assert_eq!(a.eval_vertex(&m, &ctx, v) > 0.0, touches);
        }
        // do traço: pega o face set da face inicial
        let b = AutoMask {
            face_set_from_stroke: true,
            ..Default::default()
        };
        let low_face = (0..m.face_count() as u32).find(|&f| m.faces[f as usize].face_set == 0).unwrap();
        b.begin_stroke(&m, &mut ctx, None, Vec3::Y, Some(low_face));
        let hi_vert = m.faces.iter().find(|f| f.face_set == 5).unwrap().corners[0].vert;
        let only_hi = !topo.vert_faces[hi_vert as usize]
            .iter()
            .any(|&f| m.faces[f as usize].face_set == 0);
        if only_hi {
            assert_eq!(b.eval_vertex(&m, &ctx, hi_vert), 0.0);
        }
        // face inválida => sem restrição de face set (ignora)
        b.begin_stroke(&m, &mut ctx, None, Vec3::Y, Some(99999));
        assert_eq!(b.eval_vertex(&m, &ctx, hi_vert), 1.0);
    }

    #[test]
    fn excluded_face_sets_protect_region_interiors_only() {
        let mut m = primitives::quad_sphere(1.0, 6);
        for f in 0..m.face_count() as u32 {
            if m.face_center(f).y > 0.0 {
                m.faces[f as usize].face_set = 4; // hemisfério superior travado
            }
        }
        let ctx = SculptContext::new(&m).unwrap();
        let a = AutoMask {
            exclude_face_sets: vec![4],
            ..Default::default()
        };
        assert!(!a.is_trivial());
        let topo = &ctx.topo;
        let (mut locked, mut free, mut border) = (0, 0, 0);
        for v in 0..m.vert_count() as u32 {
            let sets: Vec<i32> = topo.vert_faces[v as usize].iter().map(|&f| m.faces[f as usize].face_set).collect();
            let f = a.eval_vertex(&m, &ctx, v);
            if sets.iter().all(|&s| s == 4) {
                assert_eq!(f, 0.0);
                locked += 1;
            } else {
                assert_eq!(f, 1.0);
                if sets.iter().any(|&s| s == 4) {
                    border += 1;
                } else {
                    free += 1;
                }
            }
        }
        assert!(locked > 0 && free > 0 && border > 0);
        // conjunto inexistente não protege nada
        let none = AutoMask {
            exclude_face_sets: vec![99],
            ..Default::default()
        };
        assert!((0..m.vert_count() as u32).all(|v| none.eval_vertex(&m, &ctx, v) == 1.0));
    }

    #[test]
    fn topology_connected_isolates_islands() {
        let mut m = primitives::quad_sphere(1.0, 3);
        let other = primitives::quad_sphere(1.0, 3);
        let (vo, _) = m.append(&other);
        for v in vo as usize..m.verts.len() {
            m.verts[v].co += Vec3::new(0.5, 0.0, 0.0); // sobrepostas no espaço
        }
        let mut ctx = SculptContext::new(&m).unwrap();
        let a = AutoMask {
            topology_connected: true,
            ..Default::default()
        };
        a.begin_stroke(&m, &mut ctx, Some(0), Vec3::Y, None);
        for v in 0..m.vert_count() as u32 {
            let f = a.eval_vertex(&m, &ctx, v);
            if v < vo {
                assert_eq!(f, 1.0);
            } else {
                assert_eq!(f, 0.0, "ilha desconectada");
            }
        }
        // sem vértice inicial o fator topológico é ignorado
        a.begin_stroke(&m, &mut ctx, None, Vec3::Y, None);
        assert_eq!(a.eval_vertex(&m, &ctx, vo), 1.0);
    }

    #[test]
    fn topology_distance_limits_geodesic_reach() {
        let m = primitives::grid(10, 10, 10.0, 10.0);
        let mut ctx = setup(&m);
        let center = (0..m.vert_count() as u32)
            .min_by(|&a, &b| m.co(a).length().total_cmp(&m.co(b).length()))
            .unwrap();
        let a = AutoMask {
            topology_connected: true,
            topology_distance: Some(2.5),
            ..Default::default()
        };
        a.begin_stroke(&m, &mut ctx, Some(center), Vec3::Y, None);
        let d = &ctx.stroke.topo_dist;
        assert_eq!(d[center as usize], 0.0);
        let mut reached = 0;
        for v in 0..m.vert_count() as u32 {
            let f = a.eval_vertex(&m, &ctx, v);
            // distância geodésica sobre arestas (Manhattan em grade) <= 2.5 ⇒ passa
            let p = m.co(v) - m.co(center);
            let manhattan = p.x.abs() + p.z.abs();
            if manhattan <= 2.5 + 1e-4 {
                assert_eq!(f, 1.0, "v{v} manhattan {manhattan}");
                reached += 1;
            } else {
                assert_eq!(f, 0.0, "v{v} manhattan {manhattan}");
            }
        }
        assert!(reached > 5);
        assert!(geodesic_distances(&m, &ctx, 9999, None).iter().all(|d| d.is_infinite()));
    }

    #[test]
    fn boundary_protects_open_borders() {
        let m = primitives::grid(4, 4, 4.0, 4.0);
        let ctx = setup(&m);
        let a = AutoMask {
            boundary: true,
            ..Default::default()
        };
        let topo = crate::topology::Topology::build(&m);
        for v in 0..m.vert_count() as u32 {
            let b = topo.is_boundary_vert(v);
            assert_eq!(a.eval_vertex(&m, &ctx, v), if b { 0.0 } else { 1.0 });
        }
        // esfera fechada: nada protegido
        let s = primitives::quad_sphere(1.0, 4);
        let ctx = setup(&s);
        assert!((0..s.vert_count() as u32).all(|v| a.eval_vertex(&s, &ctx, v) == 1.0));
    }

    /// Plano com um buraco-cone (cavidade) e um morro, para testar curvatura.
    fn bumpy_grid() -> EditMesh {
        let mut m = primitives::grid(16, 16, 8.0, 8.0);
        for v in m.verts.iter_mut() {
            let (x, z) = (v.co.x, v.co.z);
            // cavidade em x<0, morro em x>0
            let pit = -0.6 * (-((x + 2.0).powi(2) + z.powi(2)) / 0.8).exp();
            let hill = 0.6 * (-((x - 2.0).powi(2) + z.powi(2)) / 0.8).exp();
            v.co.y = pit + hill;
        }
        m
    }

    #[test]
    fn cavity_distinguishes_pits_from_hills_and_flats() {
        let m = bumpy_grid();
        let ctx = setup(&m);
        let pit = AutoMask {
            cavity: Some(CavityParams::default()),
            ..Default::default()
        };
        let hill = AutoMask {
            cavity: Some(CavityParams {
                invert: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let near = |x: f32, z: f32| {
            (0..m.vert_count() as u32)
                .min_by(|&a, &b| {
                    let da = (m.co(a).x - x).powi(2) + (m.co(a).z - z).powi(2);
                    let db = (m.co(b).x - x).powi(2) + (m.co(b).z - z).powi(2);
                    da.total_cmp(&db)
                })
                .unwrap()
        };
        let in_pit = near(-2.0, 0.0);
        let on_hill = near(2.0, 0.0);
        let flat = near(0.0, 3.0);
        assert!(pit.eval_vertex(&m, &ctx, in_pit) > 0.5, "{}", pit.eval_vertex(&m, &ctx, in_pit));
        assert!(pit.eval_vertex(&m, &ctx, on_hill) < 0.05);
        assert!(hill.eval_vertex(&m, &ctx, on_hill) > 0.5);
        assert!(hill.eval_vertex(&m, &ctx, in_pit) < 0.05);
        assert!(pit.eval_vertex(&m, &ctx, flat) < 0.01);
        assert!(hill.eval_vertex(&m, &ctx, flat) < 0.01);
        assert!(cavity_signal(&m, &ctx, in_pit) > 0.0 && cavity_signal(&m, &ctx, on_hill) < 0.0);
    }

    #[test]
    fn cavity_on_sphere_is_convex() {
        let m = primitives::quad_sphere(1.0, 8);
        let ctx = setup(&m);
        for v in (0..m.vert_count() as u32).step_by(5) {
            assert!(cavity_signal(&m, &ctx, v) < 0.0, "esfera é convexa");
        }
    }

    #[test]
    fn automask_integrates_with_brush_defaults() {
        // o pincel carrega um AutoMask válido por padrão
        let b = Brush::new(BrushKind::Draw);
        assert!(b.automask.validate().is_ok());
        assert!(b.automask.is_trivial());
    }
}
