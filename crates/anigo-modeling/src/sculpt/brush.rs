//! Pincéis de escultura e a aplicação de um dab ([`apply_dab`]).
//!
//! # Modelo
//! * [`Brush`]: configuração (tipo, raio, força, falloff, espaçamento, flags).
//! * [`Dab`]: uma aplicação pontual — posição/normal na superfície, raio e
//!   força **efetivos** (já com pressão), deslocamentos do traço e o índice do
//!   passe de simetria.
//! * [`SculptContext`]: estado de escultura associado a uma malha — topologia
//!   cacheada, normais de vértice atualizadas localmente, adjacência, e o
//!   cache do traço (posições originais para undo/camadas, estado do Grab).
//! * [`apply_dab`] aplica um dab: consulta vértices no BVH, calcula pesos
//!   (falloff × força × (1 − máscara) × auto-máscara × orientação), aplica a
//!   fórmula do pincel, atualiza normais locais e dá refit incremental no BVH.
//!
//! # Convenções de intensidade
//! Pincéis de deslocamento (Draw, Inflate, Crease) movem um vértice
//! `strength × radius × peso` por dab (`peso = falloff(dist/raio)`); pincéis de
//! projeção/relaxamento (Smooth, Flatten, Fill, Scrape, Clay, Pinch) movem uma
//! fração `min(strength × peso, 1)` do caminho até o alvo. Os valores padrão de
//! [`Brush::new`] são calibrados para espaçamento de 10 % do raio.
//!
//! O pincel **sempre** respeita `EVert::mask` (1 = protegido), vértices e faces
//! ocultos e (opcionalmente) a orientação para a câmera.

use crate::bvh::TriBvh;
use crate::error::{ModelError, ModelResult};
use crate::history::PositionDelta;
use crate::mesh::EditMesh;
use crate::sculpt::automask::AutoMask;
use crate::sculpt::falloff::Falloff;
use crate::sculpt::symmetry::Symmetry;
use crate::topology::Topology;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ------------------------------------------------------------------ tipos base

/// Tipo de pincel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushKind {
    /// Desloca ao longo da normal (constrói/escava).
    Draw,
    /// Puxa vértices abaixo de um plano elevado (acumula argila).
    Clay,
    /// Clay com área alongada na direção do traço (tiras).
    ClayStrips,
    /// Relaxa: média laplaciana dos vizinhos.
    Smooth,
    /// Projeta vértices no plano médio da região.
    Flatten,
    /// Eleva vértices abaixo do plano médio até ele.
    Fill,
    /// Abaixa vértices acima do plano médio até ele.
    Scrape,
    /// Puxa vértices para o centro do pincel.
    Pinch,
    /// Infla ao longo das normais dos vértices.
    Inflate,
    /// Move os vértices capturados no início do traço (falloff congelado).
    Grab,
    /// Arrasta vértices junto com o pincel (re-seleciona a cada dab).
    SnakeHook,
    /// Empurra vértices na direção do traço, sobre a superfície.
    Nudge,
    /// Pinch + Draw negativo: sulco.
    Crease,
    /// Eleva até uma altura máxima, guardando a posição original do traço.
    Layer,
    /// Grab elástico (Kelvinlet regularizado, quase incompressível).
    Elastic,
    /// Pinta máscara.
    MaskPaint,
    /// Suaviza a máscara.
    MaskSmooth,
    /// Aumenta o contraste da máscara.
    MaskSharpen,
    /// Inverte a máscara na região.
    MaskInvert,
    /// Apaga a máscara na região.
    MaskClear,
}

impl BrushKind {
    /// Pincéis que editam `EVert::mask` em vez de posições.
    pub fn is_mask(self) -> bool {
        matches!(
            self,
            BrushKind::MaskPaint
                | BrushKind::MaskSmooth
                | BrushKind::MaskSharpen
                | BrushKind::MaskInvert
                | BrushKind::MaskClear
        )
    }

    /// Pincéis cuja posição vem de projetar o mouse num plano paralelo à tela
    /// (movimento livre no espaço), em vez de seguir a superfície.
    pub fn is_grab_like(self) -> bool {
        matches!(self, BrushKind::Grab | BrushKind::Elastic | BrushKind::SnakeHook)
    }

    /// Pincéis que congelam o conjunto de vértices no início do traço.
    pub fn uses_frozen_set(self) -> bool {
        matches!(self, BrushKind::Grab | BrushKind::Elastic)
    }

    fn needs_area(self) -> bool {
        matches!(
            self,
            BrushKind::Draw
                | BrushKind::Clay
                | BrushKind::ClayStrips
                | BrushKind::Flatten
                | BrushKind::Fill
                | BrushKind::Scrape
                | BrushKind::Crease
                | BrushKind::Nudge
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            BrushKind::Draw => "draw",
            BrushKind::Clay => "clay",
            BrushKind::ClayStrips => "clay_strips",
            BrushKind::Smooth => "smooth",
            BrushKind::Flatten => "flatten",
            BrushKind::Fill => "fill",
            BrushKind::Scrape => "scrape",
            BrushKind::Pinch => "pinch",
            BrushKind::Inflate => "inflate",
            BrushKind::Grab => "grab",
            BrushKind::SnakeHook => "snake_hook",
            BrushKind::Nudge => "nudge",
            BrushKind::Crease => "crease",
            BrushKind::Layer => "layer",
            BrushKind::Elastic => "elastic",
            BrushKind::MaskPaint => "mask_paint",
            BrushKind::MaskSmooth => "mask_smooth",
            BrushKind::MaskSharpen => "mask_sharpen",
            BrushKind::MaskInvert => "mask_invert",
            BrushKind::MaskClear => "mask_clear",
        }
    }
}

/// Configuração de um pincel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub kind: BrushKind,
    /// Raio em unidades do mundo (> 0).
    pub radius: f32,
    /// Força nominal (≥ 0; pode passar de 1 em pincéis de deslocamento).
    pub strength: f32,
    pub falloff: Falloff,
    /// Espaçamento entre dabs como fração do raio (0.1 = 10 %).
    pub spacing: f32,
    /// Inverte o efeito (Draw → escava, Clay → raspa, MaskPaint → apaga…).
    pub direction_invert: bool,
    /// Ignora vértices de costas para a câmera (`Dab::view_dir`).
    pub use_frontface: bool,
    /// Usa a normal média da região (em vez da normal do ponto atingido).
    pub use_area_normal: bool,
    /// Raio de amostragem da normal/plano médio, como fração do raio do pincel.
    pub normal_radius_factor: f32,
    /// Draw/Inflate: `false` limita a altura à do pico por traço (sem
    /// acúmulo entre dabs sobrepostos); `true` soma a cada dab.
    pub accumulate: bool,
    /// Plano de Clay/Flatten/Fill/Scrape deslocado ao longo da normal
    /// (fração do raio; positivo = para fora).
    pub plane_offset: f32,
    /// Ignora vértices a mais que `fração × raio` do plano.
    pub plane_trim: Option<f32>,
    /// Clay Strips: largura da tira como fração do raio (comprimento = raio).
    pub strip_width: f32,
    /// Layer: altura máxima em unidades do mundo.
    pub layer_height: f32,
    /// Crease: fração do caminho ao centro por dab (`pinch_factor × peso`,
    /// escalada por `min(4·força, 1)`). Snake Hook: `força × peso × pinch_factor`.
    pub pinch_factor: f32,
    /// Elastic: razão de Poisson (−1, 0.5); 0.4 ≈ borracha, 0.5 = incompressível.
    pub poisson_ratio: f32,
    pub automask: AutoMask,
    // ---- traço
    /// A pressão da caneta escala a força.
    pub use_pressure_strength: bool,
    /// A pressão da caneta escala o raio.
    pub use_pressure_size: bool,
    /// Raio do estabilizador "lazy" como fração do raio do pincel (0 = desligado).
    pub stabilize_radius: f32,
    /// Suavização do traço, `[0, 1)` (0 = sem suavização).
    pub stroke_smooth: f32,
    /// Airbrush: emite dabs pelo tempo mesmo com o mouse parado.
    pub airbrush: bool,
    /// Dabs por segundo no airbrush.
    pub airbrush_rate: f32,
}

impl Brush {
    /// Pincel com valores padrão calibrados para cada tipo (espaçamento 10 %).
    pub fn new(kind: BrushKind) -> Self {
        let (strength, spacing) = match kind {
            BrushKind::Draw => (0.05, 0.1),
            BrushKind::Clay => (0.5, 0.1),
            BrushKind::ClayStrips => (0.5, 0.1),
            BrushKind::Smooth => (0.5, 0.1),
            BrushKind::Flatten => (0.5, 0.1),
            BrushKind::Fill => (0.5, 0.1),
            BrushKind::Scrape => (0.5, 0.1),
            BrushKind::Pinch => (0.3, 0.1),
            BrushKind::Inflate => (0.05, 0.1),
            BrushKind::Grab => (1.0, 0.1),
            BrushKind::SnakeHook => (1.0, 0.1),
            BrushKind::Nudge => (0.5, 0.1),
            BrushKind::Crease => (0.05, 0.1),
            BrushKind::Layer => (0.5, 0.1),
            BrushKind::Elastic => (1.0, 0.1),
            BrushKind::MaskPaint => (0.25, 0.1),
            BrushKind::MaskSmooth => (0.5, 0.1),
            BrushKind::MaskSharpen => (0.5, 0.1),
            BrushKind::MaskInvert => (0.5, 0.1),
            BrushKind::MaskClear => (0.5, 0.1),
        };
        Self {
            kind,
            radius: 0.25,
            strength,
            falloff: Falloff::Smooth,
            spacing,
            direction_invert: false,
            use_frontface: false,
            use_area_normal: true,
            normal_radius_factor: 1.0,
            accumulate: true,
            plane_offset: 0.0,
            plane_trim: None,
            strip_width: 0.5,
            layer_height: 0.05,
            pinch_factor: if kind == BrushKind::Crease { 0.5 } else { 0.0 },
            poisson_ratio: 0.4,
            automask: AutoMask::default(),
            use_pressure_strength: false,
            use_pressure_size: false,
            stabilize_radius: 0.0,
            stroke_smooth: 0.0,
            airbrush: false,
            airbrush_rate: 20.0,
        }
    }

    /// Valida todos os parâmetros.
    pub fn validate(&self) -> ModelResult<()> {
        let bad = |what: &str, v: f32| Err(ModelError::InvalidParameter(format!("{what} inválido: {v}")));
        if !(self.radius > 0.0) || !self.radius.is_finite() {
            return bad("raio do pincel", self.radius);
        }
        if !self.strength.is_finite() || self.strength < 0.0 {
            return bad("força do pincel", self.strength);
        }
        if !(self.spacing >= 0.01 && self.spacing <= 10.0) {
            return bad("espaçamento (0.01..10)", self.spacing);
        }
        if !(self.normal_radius_factor > 0.0 && self.normal_radius_factor <= 4.0) {
            return bad("normal_radius_factor (0..4]", self.normal_radius_factor);
        }
        if !self.plane_offset.is_finite() {
            return bad("plane_offset", self.plane_offset);
        }
        if let Some(t) = self.plane_trim {
            if !(t > 0.0) || !t.is_finite() {
                return bad("plane_trim", t);
            }
        }
        if !(self.strip_width > 0.0 && self.strip_width <= 4.0) {
            return bad("strip_width (0..4]", self.strip_width);
        }
        if !self.layer_height.is_finite() || self.layer_height < 0.0 {
            return bad("layer_height", self.layer_height);
        }
        if !self.pinch_factor.is_finite() || self.pinch_factor < 0.0 {
            return bad("pinch_factor", self.pinch_factor);
        }
        if !(self.poisson_ratio > -1.0 && self.poisson_ratio < 0.5) {
            return bad("razão de Poisson (-1, 0.5)", self.poisson_ratio);
        }
        if !self.stabilize_radius.is_finite() || self.stabilize_radius < 0.0 {
            return bad("stabilize_radius", self.stabilize_radius);
        }
        if !(self.stroke_smooth >= 0.0 && self.stroke_smooth < 1.0) {
            return bad("stroke_smooth [0,1)", self.stroke_smooth);
        }
        if !(self.airbrush_rate > 0.0) || !self.airbrush_rate.is_finite() {
            return bad("airbrush_rate", self.airbrush_rate);
        }
        self.falloff.validate()?;
        self.automask.validate()
    }
}

/// Uma aplicação pontual do pincel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dab {
    /// Ponto na superfície (centro da esfera de influência).
    pub center: Vec3,
    /// Normal da superfície no ponto atingido.
    pub normal: Vec3,
    /// Direção da câmera para a cena (para `use_frontface`); zero = desconhecida.
    pub view_dir: Vec3,
    /// Raio efetivo (já com pressão).
    pub radius: f32,
    /// Força efetiva (já com pressão).
    pub strength: f32,
    /// Deslocamento do centro desde o dab anterior (Nudge, Snake Hook,
    /// direção do Clay Strips).
    pub delta: Vec3,
    /// Deslocamento acumulado desde o início do traço (Grab, Elastic).
    pub total_delta: Vec3,
    /// Índice do passe de simetria (0 = original).
    pub pass: u32,
    /// Primeiro dab do traço.
    pub first: bool,
}

impl Dab {
    /// Dab simples: sem movimento, câmera desconhecida, passe 0.
    pub fn new(center: Vec3, normal: Vec3, radius: f32, strength: f32) -> Self {
        Self {
            center,
            normal,
            view_dir: Vec3::ZERO,
            radius,
            strength,
            delta: Vec3::ZERO,
            total_delta: Vec3::ZERO,
            pass: 0,
            first: false,
        }
    }

    /// Dab com raio e força do `brush`.
    pub fn from_brush(brush: &Brush, center: Vec3, normal: Vec3) -> Self {
        Self::new(center, normal, brush.radius, brush.strength)
    }

    /// Valida finitude e faixas.
    pub fn validate(&self) -> ModelResult<()> {
        let finite = self.center.is_finite()
            && self.normal.is_finite()
            && self.view_dir.is_finite()
            && self.delta.is_finite()
            && self.total_delta.is_finite();
        if !finite {
            return Err(ModelError::InvalidParameter("dab com valor não finito".into()));
        }
        if !(self.radius > 0.0) || !self.radius.is_finite() {
            return Err(ModelError::InvalidParameter(format!(
                "raio do dab deve ser > 0: {}",
                self.radius
            )));
        }
        if !self.strength.is_finite() || self.strength < 0.0 {
            return Err(ModelError::InvalidParameter(format!(
                "força do dab inválida: {}",
                self.strength
            )));
        }
        Ok(())
    }
}

/// Resultado de um (ou vários) dab(s).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DabResult {
    /// Vértices com influência > 0 (ordenados).
    pub affected_verts: Vec<u32>,
    /// Vértices cuja posição mudou (ordenados).
    pub moved_verts: Vec<u32>,
    /// Vértices cuja máscara mudou (ordenados).
    pub mask_changed_verts: Vec<u32>,
    /// Normal média da região (quando o pincel a calcula).
    pub area_normal: Vec3,
    /// Centro médio da região (quando o pincel o calcula).
    pub area_center: Vec3,
    /// Maior deslocamento aplicado a um vértice.
    pub max_displacement: f32,
}

impl DabResult {
    /// Une outro resultado (listas ordenadas sem repetição).
    pub fn merge(&mut self, other: &DabResult) {
        fn union(a: &mut Vec<u32>, b: &[u32]) {
            a.extend_from_slice(b);
            a.sort_unstable();
            a.dedup();
        }
        union(&mut self.affected_verts, &other.affected_verts);
        union(&mut self.moved_verts, &other.moved_verts);
        union(&mut self.mask_changed_verts, &other.mask_changed_verts);
        self.max_displacement = self.max_displacement.max(other.max_displacement);
        if self.area_normal == Vec3::ZERO {
            self.area_normal = other.area_normal;
            self.area_center = other.area_center;
        }
    }

    /// `true` se o dab não alterou nada.
    pub fn is_noop(&self) -> bool {
        self.moved_verts.is_empty() && self.mask_changed_verts.is_empty()
    }
}

// ---------------------------------------------------------------- deltas/undo

/// Delta esparso de máscara (índice, antes, depois) para undo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaskDelta {
    pub entries: Vec<(u32, f32, f32)>,
}

impl MaskDelta {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn apply(&self, mesh: &mut EditMesh) {
        for &(i, _, after) in &self.entries {
            if let Some(v) = mesh.verts.get_mut(i as usize) {
                v.mask = after;
            }
        }
    }

    pub fn revert(&self, mesh: &mut EditMesh) {
        for &(i, before, _) in &self.entries {
            if let Some(v) = mesh.verts.get_mut(i as usize) {
                v.mask = before;
            }
        }
    }
}

/// Deltas de um traço: posições e máscara.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrokeDeltas {
    pub positions: PositionDelta,
    pub masks: MaskDelta,
}

impl StrokeDeltas {
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() && self.masks.is_empty()
    }

    pub fn apply(&self, mesh: &mut EditMesh) {
        self.positions.apply(mesh);
        self.masks.apply(mesh);
    }

    pub fn revert(&self, mesh: &mut EditMesh) {
        self.positions.revert(mesh);
        self.masks.revert(mesh);
    }
}

// ------------------------------------------------------------ contexto/escultura

#[derive(Debug, Clone)]
pub(crate) struct GrabVert {
    pub v: u32,
    pub orig: Vec3,
    /// Peso estático (falloff × máscara × auto-máscara × orientação), sem a força.
    pub w: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct GrabState {
    pub center: Vec3,
    pub radius: f32,
    pub verts: Vec<GrabVert>,
}

/// Estado do traço corrente (posições originais, máscara original, camadas,
/// conjuntos congelados do Grab e dados do auto-mask).
#[derive(Debug, Default)]
pub struct StrokeCache {
    pub active: bool,
    pub start_vert: Option<u32>,
    pub start_normal: Option<Vec3>,
    pub start_face_set: Option<i32>,
    /// Distâncias geodésicas do auto-mask de topologia (vazio = desligado).
    pub topo_dist: Vec<f32>,
    pub(crate) orig_co: Vec<Vec3>,
    pub(crate) orig_nrm: Vec<Vec3>,
    touched: Vec<bool>,
    touched_list: Vec<u32>,
    orig_mask: Vec<f32>,
    mask_touched: Vec<bool>,
    mask_list: Vec<u32>,
    pub(crate) layer: Vec<f32>,
    pub(crate) grabs: BTreeMap<u32, GrabState>,
}

impl StrokeCache {
    fn ensure(&mut self, n: usize) {
        if self.touched.len() != n {
            self.orig_co = vec![Vec3::ZERO; n];
            self.orig_nrm = vec![Vec3::Y; n];
            self.touched = vec![false; n];
            self.touched_list.clear();
            self.orig_mask = vec![0.0; n];
            self.mask_touched = vec![false; n];
            self.mask_list.clear();
            self.layer = vec![0.0; n];
        }
    }

    /// Guarda a posição/normal originais do vértice na primeira vez que ele muda.
    pub(crate) fn record_orig(&mut self, v: u32, co: Vec3, normal: Vec3) {
        let i = v as usize;
        if i < self.touched.len() && !self.touched[i] {
            self.touched[i] = true;
            self.touched_list.push(v);
            self.orig_co[i] = co;
            self.orig_nrm[i] = normal;
        }
    }

    pub(crate) fn record_mask(&mut self, v: u32, mask: f32) {
        let i = v as usize;
        if i < self.mask_touched.len() && !self.mask_touched[i] {
            self.mask_touched[i] = true;
            self.mask_list.push(v);
            self.orig_mask[i] = mask;
        }
    }

    /// Quantidade de vértices distintos movidos no traço.
    pub fn touched_count(&self) -> usize {
        self.touched_list.len()
    }
}

/// Estado de escultura de uma malha. Crie com [`SculptContext::new`]; após
/// qualquer mudança de **topologia** (dyntopo, voxel remesh, deleções) chame
/// [`SculptContext::rebuild`] (e reconstrua o BVH).
#[derive(Debug)]
pub struct SculptContext {
    pub topo: Topology,
    /// Normais de vértice (média ponderada por ângulo das normais de face).
    pub vert_normals: Vec<Vec3>,
    pub face_normals: Vec<Vec3>,
    /// Vértice em fronteira aberta (aresta com uma única face).
    pub boundary: Vec<bool>,
    adj_start: Vec<u32>,
    adj: Vec<u32>,
    pub stroke: StrokeCache,
}

impl SculptContext {
    /// Constrói topologia, adjacência e normais. Valida a malha.
    pub fn new(mesh: &EditMesh) -> ModelResult<Self> {
        mesh.validate()?;
        let topo = Topology::build(mesh);
        let mut ctx = Self {
            topo,
            vert_normals: Vec::new(),
            face_normals: Vec::new(),
            boundary: Vec::new(),
            adj_start: Vec::new(),
            adj: Vec::new(),
            stroke: StrokeCache::default(),
        };
        ctx.init_derived(mesh);
        Ok(ctx)
    }

    fn init_derived(&mut self, mesh: &EditMesh) {
        let n = mesh.verts.len();
        // adjacência CSR (vizinhos ordenados, sem repetição)
        let mut start = vec![0u32; n + 1];
        let mut adj: Vec<u32> = Vec::with_capacity(self.topo.edges.len() * 2);
        for v in 0..n {
            let mut nb: Vec<u32> = self.topo.vert_edges[v]
                .iter()
                .map(|&e| {
                    let (a, b) = self.topo.edges[e as usize].verts;
                    if a as usize == v {
                        b
                    } else {
                        a
                    }
                })
                .collect();
            nb.sort_unstable();
            nb.dedup();
            adj.extend_from_slice(&nb);
            start[v + 1] = adj.len() as u32;
        }
        self.adj_start = start;
        self.adj = adj;
        self.boundary = (0..n)
            .map(|v| {
                self.topo.vert_edges[v]
                    .iter()
                    .any(|&e| self.topo.edges[e as usize].faces.len() == 1)
            })
            .collect();
        self.face_normals = (0..mesh.faces.len() as u32).map(|f| mesh.face_normal(f)).collect();
        self.vert_normals = vec![Vec3::Y; n];
        for v in 0..n as u32 {
            self.vert_normals[v as usize] = self.compute_vertex_normal(mesh, v);
        }
        self.stroke = StrokeCache::default();
    }

    /// Reconstrói tudo (use depois de mudar a topologia). Descarta o traço em curso.
    pub fn rebuild(&mut self, mesh: &EditMesh) -> ModelResult<()> {
        mesh.validate()?;
        self.topo = Topology::build(mesh);
        self.init_derived(mesh);
        Ok(())
    }

    /// `true` se o contexto ainda descreve a malha (mesmas contagens).
    pub fn is_synced(&self, mesh: &EditMesh) -> bool {
        self.vert_normals.len() == mesh.verts.len() && self.face_normals.len() == mesh.faces.len()
    }

    pub(crate) fn check_sync(&self, mesh: &EditMesh) -> ModelResult<()> {
        if self.is_synced(mesh) {
            Ok(())
        } else {
            Err(ModelError::InvalidParameter(format!(
                "contexto de escultura desatualizado ({} vértices/{} faces no contexto, {}/{} na malha); chame rebuild",
                self.vert_normals.len(),
                self.face_normals.len(),
                mesh.verts.len(),
                mesh.faces.len()
            )))
        }
    }

    /// Vizinhos (por aresta) do vértice, ordenados.
    pub fn neighbors(&self, v: u32) -> &[u32] {
        let i = v as usize;
        if i + 1 >= self.adj_start.len() {
            return &[];
        }
        &self.adj[self.adj_start[i] as usize..self.adj_start[i + 1] as usize]
    }

    /// Vértice oculto: marcado ou todas as suas faces ocultas.
    pub fn vert_is_hidden(&self, mesh: &EditMesh, v: u32) -> bool {
        let Some(vert) = mesh.verts.get(v as usize) else {
            return true;
        };
        if vert.hidden {
            return true;
        }
        let faces = &self.topo.vert_faces[v as usize];
        !faces.is_empty() && faces.iter().all(|&f| mesh.faces[f as usize].hidden)
    }

    fn compute_vertex_normal(&self, mesh: &EditMesh, v: u32) -> Vec3 {
        let mut acc = Vec3::ZERO;
        let mut plain = Vec3::ZERO;
        for &f in &self.topo.vert_faces[v as usize] {
            let face = &mesh.faces[f as usize];
            let n = face.corners.len();
            let Some(ci) = face.corners.iter().position(|c| c.vert == v) else {
                continue;
            };
            let p = mesh.verts[v as usize].co;
            let prev = mesh.verts[face.corners[(ci + n - 1) % n].vert as usize].co;
            let next = mesh.verts[face.corners[(ci + 1) % n].vert as usize].co;
            let a = (prev - p).normalize_or_zero();
            let b = (next - p).normalize_or_zero();
            let angle = a.dot(b).clamp(-1.0, 1.0).acos();
            acc += self.face_normals[f as usize] * angle;
            plain += self.face_normals[f as usize];
        }
        acc.try_normalize()
            .or_else(|| plain.try_normalize())
            .unwrap_or_else(|| self.vert_normals.get(v as usize).copied().unwrap_or(Vec3::Y))
    }

    /// Recalcula só o necessário depois que `moved` mudaram de posição:
    /// normais das faces incidentes e dos vértices dessas faces.
    pub fn refresh_normals(&mut self, mesh: &EditMesh, moved: &[u32]) {
        if moved.is_empty() {
            return;
        }
        let mut faces: Vec<u32> = Vec::new();
        for &v in moved {
            if let Some(list) = self.topo.vert_faces.get(v as usize) {
                faces.extend_from_slice(list);
            }
        }
        faces.sort_unstable();
        faces.dedup();
        for &f in &faces {
            self.face_normals[f as usize] = mesh.face_normal(f);
        }
        let mut verts: Vec<u32> = moved.to_vec();
        for &f in &faces {
            verts.extend(mesh.faces[f as usize].verts());
        }
        verts.sort_unstable();
        verts.dedup();
        for v in verts {
            self.vert_normals[v as usize] = self.compute_vertex_normal(mesh, v);
        }
    }

    /// Inicia um traço (limpa o cache e prepara o auto-mask). Erro se já há um
    /// traço ativo ou se o contexto está desatualizado.
    pub fn begin_stroke(
        &mut self,
        mesh: &EditMesh,
        brush: &Brush,
        start_vert: Option<u32>,
        start_normal: Vec3,
        start_face: Option<u32>,
    ) -> ModelResult<()> {
        self.check_sync(mesh)?;
        if self.stroke.active {
            return Err(ModelError::InvalidParameter(
                "já existe um traço ativo; chame end_stroke antes".into(),
            ));
        }
        brush.validate()?;
        self.stroke.ensure(mesh.verts.len());
        self.stroke.grabs.clear();
        self.stroke.active = true;
        brush
            .automask
            .begin_stroke(mesh, self, start_vert, start_normal, start_face);
        Ok(())
    }

    /// Encerra o traço e devolve os deltas (posições e máscara) para undo.
    /// Sem traço ativo devolve deltas vazios.
    pub fn end_stroke(&mut self, mesh: &EditMesh) -> StrokeDeltas {
        let mut out = StrokeDeltas::default();
        let mut touched = std::mem::take(&mut self.stroke.touched_list);
        touched.sort_unstable();
        for &v in &touched {
            let i = v as usize;
            if let Some(vert) = mesh.verts.get(i) {
                let before = self.stroke.orig_co[i];
                if before != vert.co {
                    out.positions.entries.push((v, before, vert.co));
                }
            }
            self.stroke.touched[i] = false;
            self.stroke.layer[i] = 0.0;
        }
        touched.clear();
        self.stroke.touched_list = touched;
        let mut ml = std::mem::take(&mut self.stroke.mask_list);
        ml.sort_unstable();
        for &v in &ml {
            let i = v as usize;
            if let Some(vert) = mesh.verts.get(i) {
                let before = self.stroke.orig_mask[i];
                if before != vert.mask {
                    out.masks.entries.push((v, before, vert.mask));
                }
            }
            self.stroke.mask_touched[i] = false;
        }
        ml.clear();
        self.stroke.mask_list = ml;
        self.stroke.grabs.clear();
        self.stroke.topo_dist.clear();
        self.stroke.active = false;
        self.stroke.start_vert = None;
        self.stroke.start_normal = None;
        self.stroke.start_face_set = None;
        out
    }
}

// --------------------------------------------------------------- utilidades

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Kelvinlet "grab" regularizado normalizado (de Goes & James 2017): campo de
/// deslocamento em `r` (relativo ao centro) para uma força `f`, com
/// escala `eps` e razão de Poisson `nu`. No centro devolve exatamente `f`.
pub fn kelvinlet(r: Vec3, f: Vec3, eps: f32, nu: f32) -> Vec3 {
    let a = 1.0f32;
    let b = a / (4.0 * (1.0 - nu));
    let re = (r.length_squared() + eps * eps).sqrt();
    let re3 = re * re * re;
    let re5 = re3 * re * re;
    let rf = r.dot(f);
    let u = f * ((a - b) / re) + r * (b * rf / re3) + (f / re3 - r * (3.0 * rf / re5)) * (a * eps * eps * 0.5);
    u * (eps / (1.5 * a - b))
}

struct Cand {
    v: u32,
    /// falloff × (1 − máscara, se aplicável) × auto-máscara.
    w: f32,
    co: Vec3,
}

/// Região do dab: candidatos com peso e informações de área.
struct Region {
    cands: Vec<Cand>,
    area_normal: Vec3,
    area_center: Vec3,
    /// Normal usada pelo pincel (área ou do ponto atingido).
    sculpt_normal: Vec3,
}

fn safe_normal(n: Vec3, fallback: Vec3) -> Vec3 {
    n.try_normalize()
        .or_else(|| fallback.try_normalize())
        .unwrap_or(Vec3::Y)
}

fn facing_ok(ctx: &SculptContext, brush: &Brush, dab: &Dab, v: u32) -> bool {
    !brush.use_frontface || dab.view_dir == Vec3::ZERO || ctx.vert_normals[v as usize].dot(dab.view_dir) <= 0.0
}

fn build_region(
    mesh: &EditMesh,
    bvh: &TriBvh,
    ctx: &SculptContext,
    brush: &Brush,
    dab: &Dab,
    apply_mask: bool,
    need_area: bool,
    shape: &dyn Fn(Vec3) -> Option<f32>,
) -> Region {
    let radius = dab.radius;
    let area_r = radius * brush.normal_radius_factor;
    let query_r = radius.max(if need_area { area_r } else { 0.0 });
    let raw: Vec<u32> = bvh
        .query_sphere(dab.center, query_r)
        .into_iter()
        .filter(|&v| !ctx.vert_is_hidden(mesh, v) && facing_ok(ctx, brush, dab, v))
        .collect();

    let (mut area_normal, mut area_center) = (dab.normal, dab.center);
    if need_area {
        let mut sn = Vec3::ZERO;
        let mut sc = Vec3::ZERO;
        let mut cnt = 0u32;
        let r2 = area_r * area_r;
        for &v in &raw {
            let co = mesh.verts[v as usize].co;
            if co.distance_squared(dab.center) <= r2 {
                sn += ctx.vert_normals[v as usize];
                sc += co;
                cnt += 1;
            }
        }
        if cnt > 0 {
            area_center = sc / cnt as f32;
            area_normal = safe_normal(sn, dab.normal);
        } else {
            area_normal = safe_normal(dab.normal, Vec3::Y);
        }
    }
    let sculpt_normal = if brush.use_area_normal {
        area_normal
    } else {
        safe_normal(dab.normal, area_normal)
    };

    let r2 = radius * radius;
    let mut cands = Vec::with_capacity(raw.len());
    for &v in &raw {
        let co = mesh.verts[v as usize].co;
        if co.distance_squared(dab.center) > r2 {
            continue;
        }
        let Some(t) = shape(co) else { continue };
        let mut w = brush.falloff.eval(t);
        if w <= 0.0 {
            continue;
        }
        w *= brush.automask.eval_vertex_no_mask(mesh, ctx, v);
        if apply_mask {
            w *= (1.0 - mesh.verts[v as usize].mask).clamp(0.0, 1.0);
        }
        if w <= 0.0 {
            continue;
        }
        cands.push(Cand { v, w, co });
    }
    Region {
        cands,
        area_normal,
        area_center,
        sculpt_normal,
    }
}

/// Aplica as novas posições, registrando originais, normais locais e refit.
fn commit_positions(
    mesh: &mut EditMesh,
    bvh: &mut TriBvh,
    ctx: &mut SculptContext,
    updates: &[(u32, Vec3)],
    res: &mut DabResult,
) -> ModelResult<()> {
    let mut moved: Vec<u32> = Vec::new();
    for &(v, p) in updates {
        if !p.is_finite() {
            continue;
        }
        let cur = mesh.verts[v as usize].co;
        if cur == p {
            continue;
        }
        let n = ctx.vert_normals[v as usize];
        ctx.stroke.record_orig(v, cur, n);
        mesh.verts[v as usize].co = p;
        res.max_displacement = res.max_displacement.max(cur.distance(p));
        moved.push(v);
    }
    if !moved.is_empty() {
        ctx.refresh_normals(mesh, &moved);
        bvh.refit_verts(mesh, &moved)?;
    }
    res.moved_verts = moved;
    Ok(())
}

fn commit_masks(mesh: &mut EditMesh, ctx: &mut SculptContext, updates: &[(u32, f32)], res: &mut DabResult) {
    let mut changed = Vec::new();
    for &(v, m) in updates {
        let m = m.clamp(0.0, 1.0);
        let cur = mesh.verts[v as usize].mask;
        if (cur - m).abs() < 1e-7 {
            continue;
        }
        ctx.stroke.record_mask(v, cur);
        mesh.verts[v as usize].mask = m;
        changed.push(v);
    }
    res.mask_changed_verts = changed;
}

#[derive(Clone, Copy, PartialEq)]
enum PlaneMode {
    Both,
    /// Só vértices abaixo do plano sobem até ele.
    Below,
    /// Só vértices acima do plano descem até ele.
    Above,
}

/// Média dos vizinhos de `v` para o Smooth. Em vértice de fronteira aberta só
/// desliza ao longo da borda (média dos dois vizinhos de borda) e apenas se a
/// borda é aproximadamente reta; cantos e junções ficam fixos.
fn neighbor_average(mesh: &EditMesh, ctx: &SculptContext, v: u32) -> Option<Vec3> {
    if ctx.boundary[v as usize] {
        let mut nb: Vec<u32> = Vec::with_capacity(2);
        for &e in &ctx.topo.vert_edges[v as usize] {
            let info = &ctx.topo.edges[e as usize];
            if info.faces.len() == 1 {
                nb.push(if info.verts.0 == v { info.verts.1 } else { info.verts.0 });
            }
        }
        if nb.len() != 2 {
            return None;
        }
        let p = mesh.verts[v as usize].co;
        let a = mesh.verts[nb[0] as usize].co;
        let b = mesh.verts[nb[1] as usize].co;
        let (da, db) = ((a - p).normalize_or_zero(), (b - p).normalize_or_zero());
        if da.dot(db) > -0.5 {
            return None; // canto agudo: preserva
        }
        return Some((a + b) * 0.5);
    }
    let nb = ctx.neighbors(v);
    if nb.is_empty() {
        return None;
    }
    let sum: Vec3 = nb.iter().map(|&u| mesh.verts[u as usize].co).sum();
    Some(sum / nb.len() as f32)
}

fn neighbor_mask_average(mesh: &EditMesh, ctx: &SculptContext, v: u32) -> Option<f32> {
    let nb = ctx.neighbors(v);
    if nb.is_empty() {
        return None;
    }
    Some(nb.iter().map(|&u| mesh.verts[u as usize].mask).sum::<f32>() / nb.len() as f32)
}

// ------------------------------------------------------------------ apply_dab

/// Aplica um dab de `brush` em `mesh`.
///
/// * `bvh` precisa ter sido construído para esta malha (mesmas contagens); é
///   atualizado por refit incremental quando posições mudam.
/// * `ctx` precisa estar sincronizado ([`SculptContext::is_synced`]). Se não
///   há traço ativo, um é iniciado implicitamente a partir do dab (use
///   [`SculptContext::end_stroke`] para obter os deltas de undo e reiniciar).
/// * `dab.radius`/`dab.strength` são os valores **efetivos** (o `Brush::radius`
///   e `Brush::strength` não são lidos aqui; veja [`Dab::from_brush`]).
///
/// Valida tudo antes de mutar: em `Err` a malha, o BVH e o contexto ficam intactos.
pub fn apply_dab(
    mesh: &mut EditMesh,
    bvh: &mut TriBvh,
    dab: &Dab,
    brush: &Brush,
    ctx: &mut SculptContext,
) -> ModelResult<DabResult> {
    brush.validate()?;
    dab.validate()?;
    ctx.check_sync(mesh)?;
    if bvh.vert_count() != mesh.verts.len() || bvh.face_count() != mesh.faces.len() {
        return Err(ModelError::InvalidParameter(
            "BVH desatualizado em relação à malha; reconstrua".into(),
        ));
    }
    if !ctx.stroke.active {
        let start = bvh.closest_point(dab.center).map(|h| {
            let f = h.face;
            let best = mesh.faces[f as usize]
                .verts()
                .min_by(|&a, &b| {
                    mesh.co(a)
                        .distance_squared(dab.center)
                        .total_cmp(&mesh.co(b).distance_squared(dab.center))
                })
                .unwrap_or(0);
            (best, f)
        });
        ctx.begin_stroke(mesh, brush, start.map(|s| s.0), dab.normal, start.map(|s| s.1))?;
    }

    let kind = brush.kind;
    let mut res = DabResult::default();

    // Grab/Elastic: depois da captura, só reposiciona o conjunto congelado.
    if kind.uses_frozen_set() && ctx.stroke.grabs.contains_key(&dab.pass) {
        apply_frozen(mesh, bvh, ctx, brush, dab, &mut res)?;
        return Ok(res);
    }

    let mask_brush = kind.is_mask();
    let radius = dab.radius;
    let inv_r = 1.0 / radius;
    let center = dab.center;

    // Forma do pincel: esfera (padrão) ou tira (Clay Strips).
    let shape_sphere = move |co: Vec3| Some(co.distance(center) * inv_r);
    let region = if kind == BrushKind::ClayStrips {
        // a base da tira depende da normal da região: duas fases
        let pre = build_region(mesh, bvh, ctx, brush, dab, true, true, &shape_sphere);
        let n = pre.sculpt_normal;
        let along = dab.delta - n * dab.delta.dot(n);
        let sx = along.try_normalize().unwrap_or_else(|| {
            let h = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
            n.cross(h).normalize_or_zero()
        });
        let sy = n.cross(sx).normalize_or_zero();
        let width = brush.strip_width;
        let shape_strip = move |co: Vec3| {
            let l = co - center;
            let u = l.dot(sx) * inv_r;
            let v = l.dot(sy) * inv_r / width;
            let t = (u.powi(4) + v.powi(4)).sqrt().sqrt();
            (t <= 1.0).then_some(t)
        };
        build_region(mesh, bvh, ctx, brush, dab, true, true, &shape_strip)
    } else {
        build_region(mesh, bvh, ctx, brush, dab, !mask_brush, kind.needs_area(), &shape_sphere)
    };

    res.area_normal = region.area_normal;
    res.area_center = region.area_center;
    res.affected_verts = region.cands.iter().map(|c| c.v).collect();
    if region.cands.is_empty() {
        return Ok(res);
    }
    let s = dab.strength;
    let dir = if brush.direction_invert { -1.0f32 } else { 1.0 };
    let n = region.sculpt_normal;

    match kind {
        BrushKind::Draw => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                let amount = dir * radius * s * c.w;
                if brush.accumulate {
                    upd.push((c.v, c.co + n * amount));
                } else {
                    let vn = ctx.vert_normals[c.v as usize];
                    ctx.stroke.record_orig(c.v, c.co, vn);
                    let orig = ctx.stroke.orig_co[c.v as usize];
                    let cur_h = (c.co - orig).dot(n);
                    let new_h = if dir > 0.0 { cur_h.max(amount) } else { cur_h.min(amount) };
                    upd.push((c.v, c.co + n * (new_h - cur_h)));
                }
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Inflate => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                let vn = ctx.vert_normals[c.v as usize];
                let amount = dir * radius * s * c.w;
                if brush.accumulate {
                    upd.push((c.v, c.co + vn * amount));
                } else {
                    ctx.stroke.record_orig(c.v, c.co, vn);
                    let orig = ctx.stroke.orig_co[c.v as usize];
                    let cur_h = (c.co - orig).dot(vn);
                    let new_h = if dir > 0.0 { cur_h.max(amount) } else { cur_h.min(amount) };
                    upd.push((c.v, c.co + vn * (new_h - cur_h)));
                }
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Clay | BrushKind::ClayStrips => {
            let shift = dir * radius * (0.15 * s.min(1.0) + brush.plane_offset);
            let mode = if dir > 0.0 { PlaneMode::Below } else { PlaneMode::Above };
            let upd = plane_updates(&region, n, shift, mode, brush, radius, s);
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Flatten => {
            let upd = plane_updates(&region, n, brush.plane_offset * radius, PlaneMode::Both, brush, radius, s);
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Fill | BrushKind::Scrape => {
            let below = (kind == BrushKind::Fill) == (dir > 0.0);
            let mode = if below { PlaneMode::Below } else { PlaneMode::Above };
            let upd = plane_updates(&region, n, brush.plane_offset * radius, mode, brush, radius, s);
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Smooth => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                if let Some(avg) = neighbor_average(mesh, ctx, c.v) {
                    let f = (s * c.w).min(1.0);
                    upd.push((c.v, c.co + (avg - c.co) * f));
                }
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Pinch => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                let f = (s * c.w).min(1.0) * dir;
                upd.push((c.v, c.co + (center - c.co) * f));
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Crease => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                let sink = -dir * radius * s * c.w;
                let pinch = (brush.pinch_factor * c.w * (s * 4.0).min(1.0)).min(1.0);
                upd.push((c.v, c.co + n * sink + (center - c.co) * pinch));
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Nudge => {
            let tangent = dab.delta - n * dab.delta.dot(n);
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                upd.push((c.v, c.co + tangent * (s * c.w).min(1.0)));
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::SnakeHook => {
            let mut upd = Vec::with_capacity(region.cands.len());
            for c in &region.cands {
                let f = (s * c.w).min(1.0);
                let pinch = f * brush.pinch_factor;
                upd.push((c.v, c.co + dab.delta * f + (center - c.co) * pinch));
            }
            commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
        }
        BrushKind::Layer => {
            let height = brush.layer_height;
            if height > 0.0 {
                let mut upd = Vec::with_capacity(region.cands.len());
                for c in &region.cands {
                    let vn = ctx.vert_normals[c.v as usize];
                    ctx.stroke.record_orig(c.v, c.co, vn);
                    let i = c.v as usize;
                    let f = ctx.stroke.layer[i];
                    let nf = (f + dir * (s * c.w).min(1.0) * (1.05 - f.abs())).clamp(-1.0, 1.0);
                    ctx.stroke.layer[i] = nf;
                    let orig = ctx.stroke.orig_co[i];
                    let onrm = ctx.stroke.orig_nrm[i];
                    upd.push((c.v, orig + onrm * (height * nf)));
                }
                commit_positions(mesh, bvh, ctx, &upd, &mut res)?;
            }
        }
        BrushKind::Grab | BrushKind::Elastic => {
            // captura: guarda o conjunto com pesos estáticos; o movimento vem nos próximos dabs
            let verts = region
                .cands
                .iter()
                .map(|c| GrabVert {
                    v: c.v,
                    orig: c.co,
                    w: c.w,
                })
                .collect();
            ctx.stroke.grabs.insert(
                dab.pass,
                GrabState {
                    center,
                    radius,
                    verts,
                },
            );
            apply_frozen(mesh, bvh, ctx, brush, dab, &mut res)?;
        }
        BrushKind::MaskPaint => {
            let upd: Vec<(u32, f32)> = region
                .cands
                .iter()
                .map(|c| (c.v, mesh.verts[c.v as usize].mask + dir * s * c.w))
                .collect();
            commit_masks(mesh, ctx, &upd, &mut res);
        }
        BrushKind::MaskClear => {
            let upd: Vec<(u32, f32)> = region
                .cands
                .iter()
                .map(|c| {
                    let m = mesh.verts[c.v as usize].mask;
                    (c.v, m * (1.0 - (s * c.w).min(1.0)))
                })
                .collect();
            commit_masks(mesh, ctx, &upd, &mut res);
        }
        BrushKind::MaskInvert => {
            let upd: Vec<(u32, f32)> = region
                .cands
                .iter()
                .map(|c| {
                    let m = mesh.verts[c.v as usize].mask;
                    let f = (s * c.w).min(1.0);
                    (c.v, m + ((1.0 - m) - m) * f)
                })
                .collect();
            commit_masks(mesh, ctx, &upd, &mut res);
        }
        BrushKind::MaskSmooth => {
            let upd: Vec<(u32, f32)> = region
                .cands
                .iter()
                .filter_map(|c| {
                    let m = mesh.verts[c.v as usize].mask;
                    let avg = neighbor_mask_average(mesh, ctx, c.v)?;
                    Some((c.v, m + (avg - m) * (s * c.w).min(1.0)))
                })
                .collect();
            commit_masks(mesh, ctx, &upd, &mut res);
        }
        BrushKind::MaskSharpen => {
            let upd: Vec<(u32, f32)> = region
                .cands
                .iter()
                .map(|c| {
                    let m = mesh.verts[c.v as usize].mask;
                    (c.v, m + (smoothstep(m) - m) * (s * c.w).min(1.0))
                })
                .collect();
            commit_masks(mesh, ctx, &upd, &mut res);
        }
    }
    Ok(res)
}

/// Novas posições dos pincéis de plano (Clay, Flatten, Fill, Scrape).
fn plane_updates(
    region: &Region,
    n: Vec3,
    shift: f32,
    mode: PlaneMode,
    brush: &Brush,
    radius: f32,
    strength: f32,
) -> Vec<(u32, Vec3)> {
    let p0 = region.area_center + n * shift;
    let mut upd = Vec::with_capacity(region.cands.len());
    for c in &region.cands {
        let dist = (c.co - p0).dot(n);
        if let Some(trim) = brush.plane_trim {
            if dist.abs() > trim * radius {
                continue;
            }
        }
        let applies = match mode {
            PlaneMode::Both => true,
            PlaneMode::Below => dist < 0.0,
            PlaneMode::Above => dist > 0.0,
        };
        if !applies {
            continue;
        }
        let f = (strength * c.w).min(1.0);
        upd.push((c.v, c.co - n * (dist * f)));
    }
    upd
}

/// Grab/Elastic: reposiciona o conjunto congelado a partir das posições originais.
fn apply_frozen(
    mesh: &mut EditMesh,
    bvh: &mut TriBvh,
    ctx: &mut SculptContext,
    brush: &Brush,
    dab: &Dab,
    res: &mut DabResult,
) -> ModelResult<()> {
    let Some(state) = ctx.stroke.grabs.get(&dab.pass) else {
        return Ok(());
    };
    let s = dab.strength;
    let mut upd: Vec<(u32, Vec3)> = Vec::with_capacity(state.verts.len());
    // ε do Kelvinlet = metade do raio de captura
    let eps = (state.radius * 0.5).max(1e-6);
    for g in &state.verts {
        let disp = match brush.kind {
            BrushKind::Elastic => kelvinlet(g.orig - state.center, dab.total_delta, eps, brush.poisson_ratio) * (s * g.w),
            _ => dab.total_delta * (s * g.w),
        };
        upd.push((g.v, g.orig + disp));
    }
    res.affected_verts = state.verts.iter().map(|g| g.v).collect();
    commit_positions(mesh, bvh, ctx, &upd, res)
}

/// Aplica um dab em todos os passes de `sym` (espelho/radial) e junta os resultados.
pub fn apply_dab_symmetric(
    mesh: &mut EditMesh,
    bvh: &mut TriBvh,
    dab: &Dab,
    brush: &Brush,
    sym: &Symmetry,
    ctx: &mut SculptContext,
) -> ModelResult<DabResult> {
    sym.validate()?;
    let mut total = DabResult::default();
    for d in sym.symmetric_dabs(dab) {
        let r = apply_dab(mesh, bvh, &d, brush, ctx)?;
        total.merge(&r);
    }
    Ok(total)
}

// ---------------------------------------------------------- máscara na malha toda

fn mask_all(mesh: &mut EditMesh, f: impl Fn(f32) -> f32) -> MaskDelta {
    let mut d = MaskDelta::default();
    for (i, v) in mesh.verts.iter_mut().enumerate() {
        let new = f(v.mask).clamp(0.0, 1.0);
        if new != v.mask {
            d.entries.push((i as u32, v.mask, new));
            v.mask = new;
        }
    }
    d
}

/// Inverte a máscara de todos os vértices (equivale a Mask → Invert do Blender).
pub fn mask_invert_all(mesh: &mut EditMesh) -> MaskDelta {
    mask_all(mesh, |m| 1.0 - m)
}

/// Zera a máscara (Mask → Clear).
pub fn mask_clear_all(mesh: &mut EditMesh) -> MaskDelta {
    mask_all(mesh, |_| 0.0)
}

/// Preenche a máscara com `value` (limitado a `[0, 1]`; NaN vira erro).
pub fn mask_fill_all(mesh: &mut EditMesh, value: f32) -> ModelResult<MaskDelta> {
    if !value.is_finite() {
        return Err(ModelError::InvalidParameter(format!("valor de máscara inválido: {value}")));
    }
    Ok(mask_all(mesh, |_| value))
}

// ---------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bvh::test_util::Rng;
    use crate::primitives;
    use crate::sculpt::automask::AutoMask;

    struct Rig {
        mesh: EditMesh,
        bvh: TriBvh,
        ctx: SculptContext,
    }

    impl Rig {
        fn new(mesh: EditMesh) -> Rig {
            let bvh = TriBvh::build(&mesh);
            let ctx = SculptContext::new(&mesh).unwrap();
            Rig { mesh, bvh, ctx }
        }
        fn dab(&mut self, d: &Dab, b: &Brush) -> DabResult {
            apply_dab(&mut self.mesh, &mut self.bvh, d, b, &mut self.ctx).unwrap()
        }
        fn vert_at(&self, p: Vec3) -> u32 {
            (0..self.mesh.vert_count() as u32)
                .min_by(|&a, &b| {
                    self.mesh
                        .co(a)
                        .distance_squared(p)
                        .total_cmp(&self.mesh.co(b).distance_squared(p))
                })
                .unwrap()
        }
    }

    fn flat() -> Rig {
        Rig::new(primitives::grid(20, 20, 2.0, 2.0))
    }

    fn brush(kind: BrushKind, radius: f32, strength: f32) -> Brush {
        let mut b = Brush::new(kind);
        b.radius = radius;
        b.strength = strength;
        b
    }

    fn up_dab(rig: &Rig, b: &Brush) -> Dab {
        let c = rig.mesh.co(rig.vert_at(Vec3::ZERO));
        Dab::from_brush(b, c, Vec3::Y)
    }

    fn laplacian_energy(m: &EditMesh) -> f32 {
        let topo = Topology::build(m);
        let mut e = 0.0;
        for v in 0..m.vert_count() as u32 {
            let nb = topo.vert_neighbors(v);
            if nb.is_empty() || topo.is_boundary_vert(v) {
                continue;
            }
            let avg = nb.iter().map(|&u| m.co(u)).sum::<Vec3>() / nb.len() as f32;
            e += (avg - m.co(v)).length_squared();
        }
        e
    }

    // ------------------------------------------------------------- Brush/Dab

    #[test]
    fn brush_defaults_are_valid_for_every_kind() {
        let kinds = [
            BrushKind::Draw,
            BrushKind::Clay,
            BrushKind::ClayStrips,
            BrushKind::Smooth,
            BrushKind::Flatten,
            BrushKind::Fill,
            BrushKind::Scrape,
            BrushKind::Pinch,
            BrushKind::Inflate,
            BrushKind::Grab,
            BrushKind::SnakeHook,
            BrushKind::Nudge,
            BrushKind::Crease,
            BrushKind::Layer,
            BrushKind::Elastic,
            BrushKind::MaskPaint,
            BrushKind::MaskSmooth,
            BrushKind::MaskSharpen,
            BrushKind::MaskInvert,
            BrushKind::MaskClear,
        ];
        let mut names = std::collections::BTreeSet::new();
        for k in kinds {
            let b = Brush::new(k);
            b.validate().unwrap();
            assert_eq!(b.kind, k);
            assert!(names.insert(k.name()));
        }
        assert_eq!(names.len(), 20);
        assert!(BrushKind::MaskPaint.is_mask() && !BrushKind::Draw.is_mask());
        assert!(BrushKind::Grab.is_grab_like() && BrushKind::SnakeHook.is_grab_like());
        assert!(!BrushKind::Nudge.is_grab_like());
        assert!(BrushKind::Elastic.uses_frozen_set() && !BrushKind::SnakeHook.uses_frozen_set());
    }

    #[test]
    fn brush_validate_rejects_bad_values() {
        let ok = Brush::new(BrushKind::Draw);
        let cases: Vec<Box<dyn Fn(&mut Brush)>> = vec![
            Box::new(|b| b.radius = 0.0),
            Box::new(|b| b.radius = f32::NAN),
            Box::new(|b| b.strength = -1.0),
            Box::new(|b| b.strength = f32::INFINITY),
            Box::new(|b| b.spacing = 0.0),
            Box::new(|b| b.spacing = 11.0),
            Box::new(|b| b.normal_radius_factor = 0.0),
            Box::new(|b| b.plane_offset = f32::NAN),
            Box::new(|b| b.plane_trim = Some(0.0)),
            Box::new(|b| b.strip_width = 0.0),
            Box::new(|b| b.layer_height = -1.0),
            Box::new(|b| b.pinch_factor = -0.1),
            Box::new(|b| b.poisson_ratio = 0.5),
            Box::new(|b| b.stabilize_radius = -1.0),
            Box::new(|b| b.stroke_smooth = 1.0),
            Box::new(|b| b.airbrush_rate = 0.0),
            Box::new(|b| b.falloff = Falloff::Custom(vec![])),
            Box::new(|b| b.automask.normal_limit = Some(-1.0)),
        ];
        for (i, c) in cases.iter().enumerate() {
            let mut b = ok.clone();
            c(&mut b);
            assert!(b.validate().is_err(), "caso {i}");
        }
    }

    #[test]
    fn brush_serde_roundtrip() {
        let mut b = Brush::new(BrushKind::Clay);
        b.falloff = Falloff::Custom(vec![1.0, 0.5, 0.0]);
        b.plane_trim = Some(0.5);
        let j = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Brush>(&j).unwrap(), b);
    }

    #[test]
    fn dab_validate() {
        assert!(Dab::new(Vec3::ZERO, Vec3::Y, 1.0, 0.5).validate().is_ok());
        assert!(Dab::new(Vec3::NAN, Vec3::Y, 1.0, 0.5).validate().is_err());
        assert!(Dab::new(Vec3::ZERO, Vec3::Y, 0.0, 0.5).validate().is_err());
        assert!(Dab::new(Vec3::ZERO, Vec3::Y, 1.0, -0.1).validate().is_err());
        assert!(Dab::new(Vec3::ZERO, Vec3::Y, 1.0, f32::NAN).validate().is_err());
        let mut d = Dab::new(Vec3::ZERO, Vec3::Y, 1.0, 0.5);
        d.delta = Vec3::INFINITY;
        assert!(d.validate().is_err());
    }

    // ------------------------------------------------------------------ Draw

    #[test]
    fn draw_moves_center_by_strength_radius_normal_weight() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let res = rig.dab(&d, &b);
        assert!(res.moved_verts.contains(&c));
        let y = rig.mesh.co(c).y;
        assert!((y - 0.2 * 0.5 * 1.0).abs() < 1e-6, "y={y}");
        // vértices a distância t: deslocamento = s·r·falloff(t)
        for &v in &res.moved_verts {
            let p0 = d.center;
            let dist = Vec3::new(rig.mesh.co(v).x - p0.x, 0.0, rig.mesh.co(v).z - p0.z).length();
            let want = 0.2 * 0.5 * b.falloff.eval(dist / 0.5);
            assert!((rig.mesh.co(v).y - want).abs() < 1e-5, "v{v} dist {dist}");
            assert!(rig.mesh.co(v).x.is_finite());
        }
        // fora do raio nada mexe
        for v in 0..rig.mesh.vert_count() as u32 {
            let dist = rig.mesh.co(v).distance(Vec3::new(d.center.x, rig.mesh.co(v).y, d.center.z));
            if dist > 0.5 + 1e-5 {
                assert_eq!(rig.mesh.co(v).y, 0.0);
            }
        }
    }

    #[test]
    fn draw_invert_digs_and_zero_strength_is_noop() {
        let mut rig = flat();
        let mut b = brush(BrushKind::Draw, 0.5, 0.2);
        b.direction_invert = true;
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        rig.dab(&d, &b);
        assert!((rig.mesh.co(c).y + 0.1).abs() < 1e-6);
        let mut rig = flat();
        let before = rig.mesh.clone();
        let mut z = brush(BrushKind::Draw, 0.5, 0.0);
        z.strength = 0.0;
        let res = rig.dab(&up_dab(&rig, &z), &z);
        assert!(res.is_noop());
        assert_eq!(rig.mesh, before);
    }

    #[test]
    fn draw_without_area_normal_follows_hit_normal() {
        let mut rig = flat();
        let mut b = brush(BrushKind::Draw, 0.4, 0.5);
        b.use_area_normal = false;
        let c = rig.mesh.co(rig.vert_at(Vec3::ZERO));
        let d = Dab::from_brush(&b, c, Vec3::new(1.0, 1.0, 0.0).normalize());
        let v = rig.vert_at(c);
        let before = rig.mesh.co(v);
        rig.dab(&d, &b);
        let disp = rig.mesh.co(v) - before;
        assert!(disp.normalize().dot(d.normal) > 0.999);
        assert!((disp.length() - 0.5 * 0.4).abs() < 1e-6);
    }

    #[test]
    fn draw_non_accumulating_caps_height_while_accumulating_grows() {
        let run = |accumulate: bool| {
            let mut rig = flat();
            let mut b = brush(BrushKind::Draw, 0.5, 0.2);
            b.accumulate = accumulate;
            let d = up_dab(&rig, &b);
            let c = rig.vert_at(d.center);
            for _ in 0..10 {
                rig.dab(&d, &b);
            }
            rig.mesh.co(c).y
        };
        let acc = run(true);
        let capped = run(false);
        // o peso usa a distância 3D ao centro do dab, que cresce conforme o vértice sobe
        let falloff = Falloff::Smooth;
        let mut want = 0.0f32;
        for _ in 0..10 {
            want += 0.1 * falloff.eval(want / 0.5);
        }
        assert!((acc - want).abs() < 1e-4, "acc={acc} want={want}");
        assert!(acc > capped * 3.0);
        assert!((capped - 0.1).abs() < 1e-5, "capped={capped}");
    }

    #[test]
    fn mask_protects_vertices_fully_and_partially() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let neighbors = rig.ctx.neighbors(c).to_vec();
        rig.mesh.verts[c as usize].mask = 1.0;
        rig.mesh.verts[neighbors[0] as usize].mask = 0.5;
        // ctx lê máscara direto da malha, mas o BVH/ctx não precisam ser refeitos
        let res = rig.dab(&d, &b);
        assert_eq!(rig.mesh.co(c).y, 0.0, "mascara 1 não se move");
        assert!(!res.moved_verts.contains(&c));
        let n0 = neighbors[0];
        let dist = rig.mesh.co(n0).distance(Vec3::new(d.center.x, rig.mesh.co(n0).y, d.center.z));
        let full = 0.2 * 0.5 * b.falloff.eval(dist / 0.5);
        assert!((rig.mesh.co(n0).y - 0.5 * full).abs() < 1e-6, "metade protegida");
    }

    #[test]
    fn hidden_vertices_and_hidden_faces_are_skipped() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let nb = rig.ctx.neighbors(c)[0];
        rig.mesh.verts[nb as usize].hidden = true;
        for &f in &rig.ctx.topo.vert_faces[c as usize].clone() {
            rig.mesh.faces[f as usize].hidden = true;
        }
        rig.dab(&d, &b);
        assert_eq!(rig.mesh.co(nb).y, 0.0, "vértice oculto");
        assert_eq!(rig.mesh.co(c).y, 0.0, "todas as faces ocultas");
    }

    #[test]
    fn frontface_ignores_back_facing_vertices() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 8));
        let mut b = brush(BrushKind::Draw, 3.0, 0.1);
        b.use_frontface = true;
        let mut d = Dab::from_brush(&b, Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        d.view_dir = -Vec3::Y; // câmera acima olhando para baixo
        let orig = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(!res.moved_verts.is_empty());
        for v in 0..rig.mesh.vert_count() as u32 {
            let faces_up = orig.co(v).normalize().dot(Vec3::Y);
            let moved = rig.mesh.co(v) != orig.co(v);
            if faces_up > 0.1 {
                // voltado para a câmera e dentro do raio
                assert!(moved || orig.co(v).distance(d.center) > 3.0, "v{v}");
            }
            if orig.verts[v as usize].co.y < -0.5 {
                assert!(!moved, "costas não podem mover");
            }
        }
        // sem frontface o fundo também se move
        let mut rig2 = Rig::new(primitives::quad_sphere(1.0, 8));
        let mut b2 = b.clone();
        b2.use_frontface = false;
        rig2.dab(&d, &b2);
        let bottom = rig2.vert_at(Vec3::new(0.0, -1.0, 0.0));
        assert_ne!(rig2.mesh.co(bottom), orig.co(bottom));
    }

    // ----------------------------------------------------------- Smooth etc.

    fn noisy_grid(seed: u64) -> Rig {
        let mut m = primitives::grid(24, 24, 2.0, 2.0);
        let mut rng = Rng(seed);
        for v in m.verts.iter_mut() {
            v.co.y = rng.range(-0.05, 0.05);
        }
        Rig::new(m)
    }

    #[test]
    fn smooth_reduces_laplacian_variance_and_keeps_boundary() {
        let mut rig = noisy_grid(1);
        let e0 = laplacian_energy(&rig.mesh);
        let boundary_before: Vec<(u32, Vec3)> = (0..rig.mesh.vert_count() as u32)
            .filter(|&v| rig.ctx.boundary[v as usize])
            .map(|v| (v, rig.mesh.co(v)))
            .collect();
        let b = brush(BrushKind::Smooth, 3.0, 0.8);
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        let mut last = e0;
        for _ in 0..5 {
            rig.dab(&d, &b);
            let e = laplacian_energy(&rig.mesh);
            assert!(e < last, "energia deve cair monotonicamente: {e} !< {last}");
            last = e;
        }
        assert!(last < e0 * 0.2, "reduz bastante: {e0} -> {last}");
        // fronteira só desliza ao longo da borda: a coordenada normal à borda não muda
        for (v, p) in boundary_before {
            let q = rig.mesh.co(v);
            let on_x_edge = (p.x.abs() - 1.0).abs() < 1e-5;
            let on_z_edge = (p.z.abs() - 1.0).abs() < 1e-5;
            assert!(on_x_edge || on_z_edge);
            if on_x_edge {
                assert!((q.x - p.x).abs() < 1e-5);
            }
            if on_z_edge {
                assert!((q.z - p.z).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn smooth_respects_mask_and_is_order_independent() {
        let mut a = noisy_grid(2);
        for v in 0..a.mesh.vert_count() {
            if v % 3 == 0 {
                a.mesh.verts[v].mask = 1.0;
            }
        }
        let before = a.mesh.clone();
        let b = brush(BrushKind::Smooth, 3.0, 1.0);
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        a.dab(&d, &b);
        for v in 0..a.mesh.vert_count() {
            if v % 3 == 0 {
                assert_eq!(a.mesh.verts[v].co, before.verts[v].co);
            }
        }
        // Jacobi: resultado independe da ordem dos vértices (compara com malha espelhada em índice)
        let mut c = noisy_grid(2);
        for v in 0..c.mesh.vert_count() {
            if v % 3 == 0 {
                c.mesh.verts[v].mask = 1.0;
            }
        }
        c.dab(&d, &b);
        assert_eq!(a.mesh.content_hash(), c.mesh.content_hash());
    }

    #[test]
    fn inflate_moves_along_vertex_normals_and_preserves_symmetry() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 8));
        let before = rig.mesh.clone();
        let b = brush(BrushKind::Inflate, 0.6, 0.1);
        let d = Dab::from_brush(&b, Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        let res = rig.dab(&d, &b);
        assert!(res.moved_verts.len() > 4);
        // direção ≈ normal do vértice (esfera: radial)
        for &v in &res.moved_verts {
            let disp = rig.mesh.co(v) - before.co(v);
            assert!(disp.normalize().dot(before.co(v).normalize()) > 0.99);
        }
        // simetria X, Z preservada
        let mm_x = crate::sculpt::symmetry::build_mirror_map(&rig.mesh, 0, 1e-4).unwrap();
        let mm_z = crate::sculpt::symmetry::build_mirror_map(&rig.mesh, 2, 1e-4).unwrap();
        assert!(mm_x.is_complete() && mm_z.is_complete());
        // o topo foi inflado
        let top = rig.vert_at(Vec3::Y);
        assert!(rig.mesh.co(top).length() > 1.05);
    }

    #[test]
    fn symmetric_dab_gives_symmetric_result_on_sphere() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 10));
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let c = Vec3::new(0.6, 0.5, 0.3).normalize();
        let d = Dab::from_brush(&b, c, c);
        let sym = Symmetry::mirror_x();
        let res = apply_dab_symmetric(&mut rig.mesh, &mut rig.bvh, &d, &b, &sym, &mut rig.ctx).unwrap();
        assert!(res.moved_verts.len() > 8);
        let mm = crate::sculpt::symmetry::build_mirror_map(&rig.mesh, 0, 1e-3).unwrap();
        assert!(mm.is_complete());
        for v in 0..rig.mesh.vert_count() as u32 {
            let m = mm.mirror_of(v).unwrap();
            let mut p = rig.mesh.co(v);
            p.x = -p.x;
            assert!((rig.mesh.co(m) - p).length() < 1e-3, "v{v}");
        }
        // sem simetria o resultado é assimétrico
        let mut rig2 = Rig::new(primitives::quad_sphere(1.0, 10));
        rig2.dab(&d, &b);
        let mm2 = crate::sculpt::symmetry::build_mirror_map(&rig2.mesh, 0, 1e-3).unwrap();
        assert!(!mm2.is_complete());
    }

    #[test]
    fn symmetric_dab_validates_symmetry() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let d = up_dab(&rig, &b);
        let bad = Symmetry {
            tolerance: f32::NAN,
            ..Default::default()
        };
        let before = rig.mesh.clone();
        assert!(apply_dab_symmetric(&mut rig.mesh, &mut rig.bvh, &d, &b, &bad, &mut rig.ctx).is_err());
        assert_eq!(rig.mesh, before);
    }

    // ------------------------------------------------------- plano: Clay etc

    fn bumpy() -> Rig {
        let mut m = primitives::grid(30, 30, 2.0, 2.0);
        let mut rng = Rng(9);
        for v in m.verts.iter_mut() {
            v.co.y = rng.range(-0.08, 0.08);
        }
        Rig::new(m)
    }

    fn y_variance(m: &EditMesh, within: f32) -> f32 {
        let ys: Vec<f32> = m
            .verts
            .iter()
            .filter(|v| Vec3::new(v.co.x, 0.0, v.co.z).length() < within)
            .map(|v| v.co.y)
            .collect();
        let mean = ys.iter().sum::<f32>() / ys.len() as f32;
        ys.iter().map(|y| (y - mean) * (y - mean)).sum::<f32>() / ys.len() as f32
    }

    #[test]
    fn flatten_reduces_height_variance() {
        let mut rig = bumpy();
        let v0 = y_variance(&rig.mesh, 0.3);
        let b = brush(BrushKind::Flatten, 0.5, 0.8);
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        for _ in 0..4 {
            rig.dab(&d, &b);
        }
        let v1 = y_variance(&rig.mesh, 0.3);
        assert!(v1 < v0 * 0.2, "{v0} -> {v1}");
    }

    #[test]
    fn fill_only_raises_and_scrape_only_lowers() {
        let rig0 = bumpy();
        let b_fill = brush(BrushKind::Fill, 0.6, 0.7);
        let d = Dab::from_brush(&b_fill, Vec3::ZERO, Vec3::Y);
        let mut rig = Rig::new(rig0.mesh.clone());
        let res = rig.dab(&d, &b_fill);
        assert!(!res.moved_verts.is_empty());
        for &v in &res.moved_verts {
            assert!(rig.mesh.co(v).y > rig0.mesh.co(v).y, "fill só sobe");
        }
        let b_scrape = brush(BrushKind::Scrape, 0.6, 0.7);
        let mut rig = Rig::new(rig0.mesh.clone());
        let res = rig.dab(&d, &b_scrape);
        assert!(!res.moved_verts.is_empty());
        for &v in &res.moved_verts {
            assert!(rig.mesh.co(v).y < rig0.mesh.co(v).y, "scrape só desce");
        }
        // invertidos trocam o comportamento
        let mut inv = b_fill.clone();
        inv.direction_invert = true;
        let mut rig = Rig::new(rig0.mesh.clone());
        let res = rig.dab(&d, &inv);
        for &v in &res.moved_verts {
            assert!(rig.mesh.co(v).y < rig0.mesh.co(v).y);
        }
    }

    #[test]
    fn clay_builds_up_material_and_invert_removes_it() {
        let mut rig = flat();
        let b = brush(BrushKind::Clay, 0.5, 0.8);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        for _ in 0..5 {
            rig.dab(&d, &b);
        }
        assert!(rig.mesh.co(c).y > 0.02, "clay eleva: {}", rig.mesh.co(c).y);
        assert!(rig.mesh.verts.iter().all(|v| v.co.y >= -1e-6), "clay nunca afunda");
        let mut inv = b.clone();
        inv.direction_invert = true;
        let mut rig = flat();
        for _ in 0..5 {
            rig.dab(&d, &inv);
        }
        assert!(rig.mesh.co(c).y < -0.02);
        assert!(rig.mesh.verts.iter().all(|v| v.co.y <= 1e-6));
    }

    #[test]
    fn plane_trim_and_offset_affect_plane_brushes() {
        let mut rig = bumpy();
        let mut b = brush(BrushKind::Flatten, 0.6, 1.0);
        b.plane_offset = 0.5; // plano 0.3 acima
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        let orig = rig.mesh.clone();
        let c = rig.vert_at(Vec3::ZERO);
        let res = rig.dab(&d, &b);
        assert!(rig.mesh.co(c).y > 0.25, "centro projetado no plano elevado");
        for &v in &res.moved_verts {
            assert!(rig.mesh.co(v).y > orig.co(v).y, "todos sobem em direção ao plano");
        }
        // trim pequeno ignora vértices distantes do plano (todos, por causa do offset)
        let mut rig = bumpy();
        b.plane_trim = Some(0.1);
        let before = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(res.moved_verts.is_empty());
        assert_eq!(rig.mesh, before);
    }

    #[test]
    fn clay_strips_covers_a_strip_along_the_stroke_direction() {
        let mut rig = flat();
        let mut b = brush(BrushKind::ClayStrips, 0.6, 1.0);
        b.strip_width = 0.3;
        let mut d = up_dab(&rig, &b);
        d.delta = Vec3::X * 0.1;
        let res = rig.dab(&d, &b);
        assert!(!res.affected_verts.is_empty());
        let mut max_x = 0.0f32;
        let mut max_z = 0.0f32;
        for &v in &res.affected_verts {
            let p = rig.mesh.co(v) - d.center;
            max_x = max_x.max(p.x.abs());
            max_z = max_z.max(p.z.abs());
        }
        assert!(max_x > 0.4, "comprimento ao longo do traço: {max_x}");
        assert!(max_z <= 0.6 * 0.3 + 1e-4, "largura limitada: {max_z}");
        // sem delta ainda funciona (base arbitrária)
        let mut rig = flat();
        d.delta = Vec3::ZERO;
        assert!(!rig.dab(&d, &b).affected_verts.is_empty());
    }

    // -------------------------------------------------------- Pinch / Crease

    #[test]
    fn pinch_pulls_toward_center_and_invert_spreads() {
        let mut rig = flat();
        let b = brush(BrushKind::Pinch, 0.6, 0.5);
        let d = up_dab(&rig, &b);
        let orig = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(!res.moved_verts.is_empty());
        for &v in &res.moved_verts {
            let before = orig.co(v).distance(d.center);
            let after = rig.mesh.co(v).distance(d.center);
            assert!(after <= before + 1e-6, "v{v}");
        }
        let mut inv = b.clone();
        inv.direction_invert = true;
        let mut rig = flat();
        let res = rig.dab(&d, &inv);
        for &v in &res.moved_verts {
            assert!(rig.mesh.co(v).distance(d.center) >= orig.co(v).distance(d.center) - 1e-6);
        }
    }

    #[test]
    fn crease_digs_a_groove_and_pinches() {
        let mut rig = flat();
        let b = brush(BrushKind::Crease, 0.5, 0.2);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let orig = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(rig.mesh.co(c).y < -0.05, "centro afunda: {}", rig.mesh.co(c).y);
        // vértices periféricos se aproximam do centro no plano XZ
        let mut closer = 0;
        for &v in &res.moved_verts {
            let before = Vec3::new(orig.co(v).x - d.center.x, 0.0, orig.co(v).z - d.center.z).length();
            let after = Vec3::new(rig.mesh.co(v).x - d.center.x, 0.0, rig.mesh.co(v).z - d.center.z).length();
            if after < before - 1e-6 {
                closer += 1;
            }
        }
        assert!(closer > 3, "pinch moveu {closer}");
        // invertido vira crista
        let mut inv = b.clone();
        inv.direction_invert = true;
        let mut rig = flat();
        rig.dab(&d, &inv);
        assert!(rig.mesh.co(c).y > 0.05);
    }

    // ------------------------------------------------------------------ Nudge

    #[test]
    fn nudge_moves_along_delta_and_drops_normal_component() {
        let mut rig = flat();
        let b = brush(BrushKind::Nudge, 0.5, 1.0);
        let mut d = up_dab(&rig, &b);
        d.delta = Vec3::new(0.1, 0.3, 0.0); // componente Y deve ser descartada
        let c = rig.vert_at(d.center);
        let before = rig.mesh.co(c);
        rig.dab(&d, &b);
        let disp = rig.mesh.co(c) - before;
        assert!((disp.x - 0.1).abs() < 1e-5 && disp.y.abs() < 1e-6 && disp.z.abs() < 1e-6, "{disp:?}");
    }

    // ---------------------------------------------------------------- Grab

    #[test]
    fn grab_freezes_set_and_follows_total_delta_idempotently() {
        let mut rig = flat();
        let b = brush(BrushKind::Grab, 0.5, 1.0);
        let mut d = up_dab(&rig, &b);
        d.first = true;
        let orig = rig.mesh.clone();
        let c = rig.vert_at(d.center);
        // primeiro dab: sem movimento
        let r0 = rig.dab(&d, &b);
        assert!(r0.moved_verts.is_empty());
        assert!(!r0.affected_verts.is_empty());
        let set: Vec<u32> = r0.affected_verts.clone();
        // move 0.3 em +Y: centro acompanha por completo
        d.first = false;
        d.total_delta = Vec3::new(0.0, 0.3, 0.0);
        let r1 = rig.dab(&d, &b);
        assert_eq!(r1.affected_verts, set);
        assert!((rig.mesh.co(c).y - 0.3).abs() < 1e-6);
        for &v in &set {
            let dist = Vec3::new(orig.co(v).x - d.center.x, 0.0, orig.co(v).z - d.center.z).length();
            let want = 0.3 * b.falloff.eval(dist / 0.5);
            assert!((rig.mesh.co(v).y - want).abs() < 1e-5);
        }
        // fora do conjunto nada muda
        for v in 0..rig.mesh.vert_count() as u32 {
            if !set.contains(&v) {
                assert_eq!(rig.mesh.co(v), orig.co(v));
            }
        }
        // idempotente: repetir o mesmo dab não acumula
        let h = rig.mesh.content_hash();
        rig.dab(&d, &b);
        assert_eq!(rig.mesh.content_hash(), h);
        // continua a seguir mesmo quando o centro do dab "foge" (falloff congelado)
        let mut d2 = d;
        d2.center += Vec3::new(5.0, 0.0, 0.0);
        d2.total_delta = Vec3::new(0.0, 0.1, 0.0);
        rig.dab(&d2, &b);
        assert!((rig.mesh.co(c).y - 0.1).abs() < 1e-6);
        // fim do traço devolve delta que desfaz tudo exatamente
        let deltas = rig.ctx.end_stroke(&rig.mesh);
        deltas.revert(&mut rig.mesh);
        for v in 0..rig.mesh.vert_count() {
            assert_eq!(rig.mesh.verts[v].co.to_array().map(f32::to_bits), orig.verts[v].co.to_array().map(f32::to_bits));
        }
    }

    #[test]
    fn grab_respects_mask_and_symmetry_passes_are_independent() {
        let mut rig = flat();
        let b = brush(BrushKind::Grab, 0.5, 1.0);
        let d0 = up_dab(&rig, &b);
        let c = rig.vert_at(d0.center);
        rig.mesh.verts[c as usize].mask = 1.0;
        let mut d = d0;
        d.first = true;
        rig.dab(&d, &b);
        d.first = false;
        d.total_delta = Vec3::Y * 0.2;
        rig.dab(&d, &b);
        assert_eq!(rig.mesh.co(c).y, 0.0, "protegido pela máscara");
        // dois passes: cada um captura o seu conjunto
        let mut rig = Rig::new(primitives::grid(20, 20, 2.0, 2.0));
        let sym = Symmetry::mirror_x();
        let vp = rig.vert_at(Vec3::new(0.5, 0.0, 0.0));
        let vm = rig.vert_at(Vec3::new(-0.5, 0.0, 0.0));
        let mut dd = Dab::from_brush(&b, Vec3::new(0.5, 0.0, 0.0), Vec3::Y);
        dd.first = true;
        apply_dab_symmetric(&mut rig.mesh, &mut rig.bvh, &dd, &b, &sym, &mut rig.ctx).unwrap();
        dd.first = false;
        dd.total_delta = Vec3::new(0.2, 0.2, 0.0);
        apply_dab_symmetric(&mut rig.mesh, &mut rig.bvh, &dd, &b, &sym, &mut rig.ctx).unwrap();
        let (p, m) = (rig.mesh.co(vp), rig.mesh.co(vm));
        assert!((p.y - m.y).abs() < 1e-5 && (p.y - 0.2).abs() < 1e-5);
        assert!((p.x - 0.7).abs() < 1e-5, "passe original: {p:?}");
        assert!((m.x + 0.7).abs() < 1e-5, "passe espelhado: {m:?}");
    }

    #[test]
    fn elastic_moves_center_fully_and_decays_smoothly() {
        let mut rig = flat();
        let b = brush(BrushKind::Elastic, 0.6, 1.0);
        let mut d = up_dab(&rig, &b);
        d.first = true;
        let orig = rig.mesh.clone();
        let c = rig.vert_at(d.center);
        rig.dab(&d, &b);
        d.first = false;
        d.total_delta = Vec3::new(0.0, 0.25, 0.0);
        let res = rig.dab(&d, &b);
        assert!(!res.moved_verts.is_empty());
        let disp_c = (rig.mesh.co(c) - orig.co(c)).length();
        assert!((disp_c - 0.25).abs() < 1e-4, "centro segue a força: {disp_c}");
        // decai monotonicamente com a distância (ao longo de uma linha)
        let mut prev = f32::INFINITY;
        for i in 0..8 {
            let v = rig.vert_at(Vec3::new(i as f32 * 0.07, 0.0, 0.0));
            let dsp = (rig.mesh.co(v) - orig.co(v)).length();
            assert!(dsp <= prev + 1e-5, "i={i}: {dsp} > {prev}");
            prev = dsp;
        }
        // kelvinlet no centro devolve a força e é finito longe
        let f = Vec3::new(0.1, 0.2, 0.3);
        assert!((kelvinlet(Vec3::ZERO, f, 0.3, 0.4) - f).length() < 1e-6);
        assert!(kelvinlet(Vec3::splat(100.0), f, 0.3, 0.4).length() < 0.01);
        assert!(kelvinlet(Vec3::new(0.2, 0.0, 0.0), f, 0.3, 0.49).is_finite());
    }

    // ------------------------------------------------------------ Snake hook

    #[test]
    fn snake_hook_drags_vertices_with_the_brush() {
        let mut rig = flat();
        let b = brush(BrushKind::SnakeHook, 0.4, 1.0);
        let mut d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let start = rig.mesh.co(c);
        // arrasta o centro do dab 0.05 por vez em +Y (para fora do plano)
        for i in 0..10 {
            d.center = start + Vec3::Y * (0.05 * i as f32);
            d.delta = Vec3::Y * 0.05;
            rig.dab(&d, &b);
        }
        assert!(rig.mesh.co(c).y > 0.3, "vértice acompanhou: {}", rig.mesh.co(c).y);
        // vértice que acompanhou nunca passa do pincel
        assert!(rig.mesh.co(c).y <= 0.5 + 1e-4);
        rig.mesh.validate().unwrap();
    }

    #[test]
    fn snake_hook_pinch_pulls_neighbors_toward_the_hook() {
        let run = |pinch: f32| {
            let mut rig = flat();
            let mut b = brush(BrushKind::SnakeHook, 0.5, 0.5);
            b.pinch_factor = pinch;
            let d = up_dab(&rig, &b);
            let res = rig.dab(&d, &b);
            res.moved_verts.len()
        };
        assert_eq!(run(0.0), 0, "sem delta e sem pinch nada se move");
        assert!(run(0.8) > 0);
    }

    // ------------------------------------------------------------------ Layer

    #[test]
    fn layer_caps_height_and_restarts_per_stroke() {
        let mut rig = flat();
        let mut b = brush(BrushKind::Layer, 0.5, 0.8);
        b.layer_height = 0.1;
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let mut prev = 0.0;
        for _ in 0..60 {
            rig.dab(&d, &b);
            let y = rig.mesh.co(c).y;
            assert!(y >= prev - 1e-7 && y <= 0.1 + 1e-6, "y={y}");
            prev = y;
        }
        assert!((prev - 0.1).abs() < 1e-4, "satura na altura: {prev}");
        // nenhum vértice passa da altura
        assert!(rig.mesh.verts.iter().all(|v| v.co.y <= 0.1 + 1e-6));
        // novo traço: a altura é relativa ao estado atual (deslocamento original guardado)
        let deltas = rig.ctx.end_stroke(&rig.mesh);
        assert!(!deltas.positions.is_empty());
        for _ in 0..60 {
            rig.dab(&d, &b);
        }
        assert!((rig.mesh.co(c).y - 0.2).abs() < 1e-3, "segundo traço soma: {}", rig.mesh.co(c).y);
        // invertido escava até -altura
        let mut rig = flat();
        let mut inv = b.clone();
        inv.direction_invert = true;
        for _ in 0..60 {
            rig.dab(&d, &inv);
        }
        assert!((rig.mesh.co(c).y + 0.1).abs() < 1e-3);
    }

    #[test]
    fn layer_with_zero_height_is_noop() {
        let mut rig = flat();
        let mut b = brush(BrushKind::Layer, 0.5, 0.8);
        b.layer_height = 0.0;
        let d = up_dab(&rig, &b);
        let before = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(res.is_noop());
        assert_eq!(rig.mesh, before);
    }

    // ------------------------------------------------------------------- Mask

    #[test]
    fn mask_paint_sets_weighted_values_and_ignores_existing_mask() {
        let mut rig = flat();
        let b = brush(BrushKind::MaskPaint, 0.5, 0.6);
        let d = up_dab(&rig, &b);
        let c = rig.vert_at(d.center);
        let res = rig.dab(&d, &b);
        assert!(res.moved_verts.is_empty());
        assert!(!res.mask_changed_verts.is_empty());
        assert!((rig.mesh.verts[c as usize].mask - 0.6).abs() < 1e-6);
        // posições intactas
        assert!(rig.mesh.verts.iter().all(|v| v.co.y == 0.0));
        // dabs repetidos saturam em 1
        for _ in 0..5 {
            rig.dab(&d, &b);
        }
        assert_eq!(rig.mesh.verts[c as usize].mask, 1.0);
        // invertido apaga
        let mut er = b.clone();
        er.direction_invert = true;
        for _ in 0..5 {
            rig.dab(&d, &er);
        }
        assert_eq!(rig.mesh.verts[c as usize].mask, 0.0);
    }

    #[test]
    fn mask_then_draw_prevents_movement() {
        let mut rig = flat();
        let paint = brush(BrushKind::MaskPaint, 0.5, 1.0);
        let d = up_dab(&rig, &paint);
        rig.dab(&d, &paint);
        let draw = brush(BrushKind::Draw, 0.5, 0.5);
        let c = rig.vert_at(d.center);
        rig.dab(&d, &draw);
        assert_eq!(rig.mesh.co(c).y, 0.0, "centro com máscara 1 não se move");
    }

    #[test]
    fn mask_clear_invert_smooth_sharpen() {
        let mut rig = flat();
        let paint = brush(BrushKind::MaskPaint, 0.6, 1.0);
        let d = up_dab(&rig, &paint);
        rig.dab(&d, &paint);
        let c = rig.vert_at(d.center);
        let edge = rig.vert_at(d.center + Vec3::new(0.35, 0.0, 0.0));
        let m_edge = rig.mesh.verts[edge as usize].mask;
        assert!(m_edge > 0.0 && m_edge < 1.0);

        // sharpen: valores abaixo de 0.5 descem, acima sobem
        let mut sharp = brush(BrushKind::MaskSharpen, 0.6, 1.0);
        sharp.falloff = Falloff::Constant;
        let mut r2 = Rig::new(rig.mesh.clone());
        r2.dab(&d, &sharp);
        for v in 0..r2.mesh.vert_count() {
            let (a, b) = (rig.mesh.verts[v].mask, r2.mesh.verts[v].mask);
            if a < 0.5 {
                assert!(b <= a + 1e-6);
            } else {
                assert!(b >= a - 1e-6);
            }
        }
        // smooth: variação máxima diminui
        let tv = |m: &EditMesh| {
            let topo = Topology::build(m);
            topo.edges
                .iter()
                .map(|e| (m.verts[e.verts.0 as usize].mask - m.verts[e.verts.1 as usize].mask).abs())
                .fold(0.0f32, f32::max)
        };
        let mut smooth = brush(BrushKind::MaskSmooth, 0.6, 1.0);
        smooth.falloff = Falloff::Constant;
        let mut r3 = Rig::new(rig.mesh.clone());
        let before = tv(&r3.mesh);
        r3.dab(&d, &smooth);
        assert!(tv(&r3.mesh) < before);
        // invert: centro (1) vira ~0 com força 1 e falloff constante
        let mut inv = brush(BrushKind::MaskInvert, 0.6, 1.0);
        inv.falloff = Falloff::Constant;
        let mut r4 = Rig::new(rig.mesh.clone());
        r4.dab(&d, &inv);
        assert!(r4.mesh.verts[c as usize].mask < 1e-6);
        // clear: zera
        let mut clr = brush(BrushKind::MaskClear, 0.6, 1.0);
        clr.falloff = Falloff::Constant;
        let mut r5 = Rig::new(rig.mesh.clone());
        r5.dab(&d, &clr);
        assert!(r5.mesh.verts[c as usize].mask < 1e-6);
        assert!(r5.mesh.verts[edge as usize].mask < 1e-6);
    }

    #[test]
    fn whole_mesh_mask_operations_with_undo_deltas() {
        let mut m = primitives::quad_sphere(1.0, 4);
        for (i, v) in m.verts.iter_mut().enumerate() {
            v.mask = (i % 5) as f32 / 4.0;
        }
        let orig = m.clone();
        let d = mask_invert_all(&mut m);
        for (a, b) in orig.verts.iter().zip(&m.verts) {
            assert!((a.mask + b.mask - 1.0).abs() < 1e-6);
        }
        d.revert(&mut m);
        assert_eq!(m, orig);
        d.apply(&mut m);
        let dc = mask_clear_all(&mut m);
        assert!(m.verts.iter().all(|v| v.mask == 0.0));
        dc.revert(&mut m);
        let inv = orig.verts.iter().map(|v| 1.0 - v.mask);
        assert!(m.verts.iter().zip(inv).all(|(v, e)| (v.mask - e).abs() < 1e-6));
        let df = mask_fill_all(&mut m, 2.0).unwrap();
        assert!(m.verts.iter().all(|v| v.mask == 1.0));
        assert!(!df.is_empty());
        assert!(mask_fill_all(&mut m, f32::NAN).is_err());
        assert!(mask_fill_all(&mut m, 1.0).unwrap().is_empty(), "sem mudança");
    }

    // ---------------------------------------------------------- Auto-mask etc

    #[test]
    fn automask_boundary_and_normal_limit_gate_the_brush() {
        // fronteira
        let mut rig = Rig::new(primitives::grid(10, 10, 2.0, 2.0));
        let mut b = brush(BrushKind::Draw, 3.0, 0.2);
        b.automask = AutoMask {
            boundary: true,
            ..Default::default()
        };
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        let res = rig.dab(&d, &b);
        assert!(!res.moved_verts.is_empty());
        for v in 0..rig.mesh.vert_count() as u32 {
            if rig.ctx.boundary[v as usize] {
                assert_eq!(rig.mesh.co(v).y, 0.0);
            }
        }
        // limite de normal numa esfera: só a calota superior
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 8));
        let mut b = brush(BrushKind::Inflate, 3.0, 0.1);
        b.automask = AutoMask {
            normal_limit: Some(50f32.to_radians()),
            ..Default::default()
        };
        let orig = rig.mesh.clone();
        let d = Dab::from_brush(&b, Vec3::Y, Vec3::Y);
        rig.dab(&d, &b);
        let mut moved = 0;
        for v in 0..rig.mesh.vert_count() as u32 {
            let ang = orig.co(v).normalize().dot(Vec3::Y).clamp(-1.0, 1.0).acos();
            let did = rig.mesh.co(v) != orig.co(v);
            if ang > 60f32.to_radians() {
                assert!(!did, "v{v} fora do cone se moveu");
            }
            if did {
                moved += 1;
            }
        }
        assert!(moved > 0);
    }

    #[test]
    fn locked_face_sets_keep_their_interior_fixed() {
        let mut m = primitives::grid(20, 20, 2.0, 2.0);
        for f in 0..m.face_count() as u32 {
            if m.face_center(f).x > 0.0 {
                m.faces[f as usize].face_set = 2;
            }
        }
        let mut rig = Rig::new(m);
        let mut b = brush(BrushKind::Draw, 3.0, 0.1);
        b.automask.exclude_face_sets = vec![2];
        let orig = rig.mesh.clone();
        let d = Dab::from_brush(&b, Vec3::ZERO, Vec3::Y);
        rig.dab(&d, &b);
        let moved_free = rig.mesh.verts.iter().zip(&orig.verts).filter(|(a, b)| a.co != b.co && a.co.x < -0.01).count();
        assert!(moved_free > 10);
        for (a, o) in rig.mesh.verts.iter().zip(&orig.verts) {
            if o.co.x > 0.01 {
                assert_eq!(a.co, o.co, "interior do face set travado não se move");
            }
        }
    }

    // ---------------------------------------------------- erros e invariantes

    #[test]
    fn invalid_inputs_leave_everything_intact() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        let good = up_dab(&rig, &b);
        let before = rig.mesh.clone();
        let mut bad_b = b.clone();
        bad_b.radius = f32::NAN;
        assert!(apply_dab(&mut rig.mesh, &mut rig.bvh, &good, &bad_b, &mut rig.ctx).is_err());
        let mut bad_d = good;
        bad_d.radius = 0.0;
        assert!(apply_dab(&mut rig.mesh, &mut rig.bvh, &bad_d, &b, &mut rig.ctx).is_err());
        assert_eq!(rig.mesh, before);
        assert!(!rig.ctx.stroke.active, "erro de validação não inicia traço");

        // contexto desatualizado
        rig.mesh.add_vert(Vec3::ZERO);
        let snapshot = rig.mesh.clone();
        let e = apply_dab(&mut rig.mesh, &mut rig.bvh, &good, &b, &mut rig.ctx);
        assert!(matches!(e, Err(ModelError::InvalidParameter(_))));
        assert_eq!(rig.mesh, snapshot);
        // depois do rebuild volta a funcionar
        rig.bvh = TriBvh::build(&rig.mesh);
        rig.ctx.rebuild(&rig.mesh).unwrap();
        assert!(apply_dab(&mut rig.mesh, &mut rig.bvh, &good, &b, &mut rig.ctx).is_ok());

        // BVH desatualizado (ctx ok)
        let mut rig = flat();
        let other = TriBvh::build(&primitives::cube(1.0));
        let mut stale = other;
        let before = rig.mesh.clone();
        assert!(apply_dab(&mut rig.mesh, &mut stale, &good, &b, &mut rig.ctx).is_err());
        assert_eq!(rig.mesh, before);
    }

    #[test]
    fn begin_stroke_twice_is_an_error_and_end_without_stroke_is_empty() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.5, 0.2);
        assert!(rig.ctx.end_stroke(&rig.mesh).is_empty());
        rig.ctx.begin_stroke(&rig.mesh, &b, None, Vec3::Y, None).unwrap();
        assert!(rig.ctx.begin_stroke(&rig.mesh, &b, None, Vec3::Y, None).is_err());
        rig.ctx.end_stroke(&rig.mesh);
        assert!(rig.ctx.begin_stroke(&rig.mesh, &b, None, Vec3::Y, None).is_ok());
    }

    #[test]
    fn normals_and_bvh_stay_in_sync_after_dabs() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 10));
        let b = brush(BrushKind::Draw, 0.5, 0.3);
        let mut rng = Rng(5);
        for _ in 0..12 {
            let c = rng.unit_vec();
            rig.dab(&Dab::from_brush(&b, c, c), &b);
        }
        // normais locais ≈ recalculadas do zero
        let fresh = SculptContext::new(&rig.mesh).unwrap();
        for v in 0..rig.mesh.vert_count() {
            assert!(
                (rig.ctx.vert_normals[v] - fresh.vert_normals[v]).length() < 1e-4,
                "normal de v{v} dessincronizada"
            );
        }
        for f in 0..rig.mesh.face_count() {
            assert!((rig.ctx.face_normals[f] - fresh.face_normals[f]).length() < 1e-5);
        }
        // normais próximas das de compute_vertex_normals (mesma ponderação por ângulo)
        let reference = crate::normals::compute_vertex_normals(&rig.mesh);
        let close = (0..reference.len())
            .filter(|&v| reference[v].dot(rig.ctx.vert_normals[v]) > 0.98)
            .count();
        assert!(close as f32 > 0.97 * reference.len() as f32);
        // BVH: posições em cache coerentes com a malha e consultas coerentes
        assert!(rig.bvh.bounds_are_consistent());
        for i in 0..rig.bvh.tri_count() as u32 {
            let tr = rig.bvh.tri_ref(i).unwrap();
            let pos = rig.bvh.triangle(i);
            for k in 0..3 {
                assert_eq!(pos[k], rig.mesh.co(tr.verts[k]), "triângulo {i} desatualizado");
            }
        }
        for _ in 0..50 {
            let o = rng.unit_vec() * 3.0;
            let dir = -o.normalize();
            let first = rig.bvh.raycast(o, dir, 100.0).map(|h| h.t);
            let all = rig.bvh.raycast_all(o, dir, 100.0);
            assert_eq!(first, all.first().map(|h| h.t));
        }
    }

    #[test]
    fn stroke_deltas_restore_positions_and_masks_exactly() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 10));
        let orig = rig.mesh.clone();
        let mut rng = Rng(21);
        let draw = brush(BrushKind::Draw, 0.4, 0.3);
        let paint = brush(BrushKind::MaskPaint, 0.4, 0.5);
        let smooth = brush(BrushKind::Smooth, 0.5, 0.7);
        for i in 0..30 {
            let c = rng.unit_vec();
            let b = match i % 3 {
                0 => &draw,
                1 => &paint,
                _ => &smooth,
            };
            rig.dab(&Dab::from_brush(b, c, c), b);
        }
        assert!(rig.ctx.stroke.touched_count() > 0);
        let deltas = rig.ctx.end_stroke(&rig.mesh);
        assert!(!deltas.positions.is_empty() && !deltas.masks.is_empty());
        // só vértices tocados aparecem
        assert!(deltas.positions.entries.len() < rig.mesh.vert_count());
        let edited = rig.mesh.clone();
        deltas.revert(&mut rig.mesh);
        assert_eq!(rig.mesh, orig);
        deltas.apply(&mut rig.mesh);
        assert_eq!(rig.mesh, edited);
        // cache limpo para o próximo traço
        assert!(!rig.ctx.stroke.active);
        assert_eq!(rig.ctx.stroke.touched_count(), 0);
        // equivalente a PositionDelta::between
        let before: Vec<Vec3> = orig.verts.iter().map(|v| v.co).collect();
        let after: Vec<Vec3> = edited.verts.iter().map(|v| v.co).collect();
        assert_eq!(PositionDelta::between(&before, &after), deltas.positions);
    }

    #[test]
    fn dabs_are_deterministic() {
        let run = || {
            let mut rig = Rig::new(primitives::quad_sphere(1.0, 8));
            let mut rng = Rng(3);
            let kinds = [BrushKind::Draw, BrushKind::Smooth, BrushKind::Clay, BrushKind::Pinch, BrushKind::Inflate];
            for i in 0..40 {
                let c = rng.unit_vec();
                let b = brush(kinds[i % kinds.len()], 0.4, 0.3);
                rig.dab(&Dab::from_brush(&b, c, c), &b);
            }
            rig.mesh.content_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn dab_far_from_mesh_is_noop() {
        let mut rig = flat();
        let b = brush(BrushKind::Draw, 0.3, 0.5);
        let d = Dab::from_brush(&b, Vec3::new(50.0, 0.0, 0.0), Vec3::Y);
        let before = rig.mesh.clone();
        let res = rig.dab(&d, &b);
        assert!(res.is_noop() && res.affected_verts.is_empty());
        assert_eq!(rig.mesh, before);
    }

    #[test]
    fn dab_result_merge_unions_sorted() {
        let mut a = DabResult {
            affected_verts: vec![1, 3],
            moved_verts: vec![1],
            max_displacement: 0.1,
            ..Default::default()
        };
        let b = DabResult {
            affected_verts: vec![2, 3],
            moved_verts: vec![3],
            mask_changed_verts: vec![9],
            max_displacement: 0.3,
            area_normal: Vec3::Y,
            ..Default::default()
        };
        a.merge(&b);
        assert_eq!(a.affected_verts, vec![1, 2, 3]);
        assert_eq!(a.moved_verts, vec![1, 3]);
        assert_eq!(a.mask_changed_verts, vec![9]);
        assert_eq!(a.max_displacement, 0.3);
        assert_eq!(a.area_normal, Vec3::Y);
    }

    #[test]
    fn works_on_meshes_with_ngons_and_triangles() {
        // cilindro com tampas n-gon, UV sphere com polos triangulares
        for m in [primitives::cylinder(0.5, 1.0, 12, 4, true), primitives::uv_sphere(1.0, 8, 12)] {
            let mut rig = Rig::new(m);
            for kind in [
                BrushKind::Draw,
                BrushKind::Smooth,
                BrushKind::Clay,
                BrushKind::Flatten,
                BrushKind::Crease,
                BrushKind::Inflate,
            ] {
                let b = brush(kind, 0.6, 0.3);
                let c = rig.mesh.co(0);
                let n = rig.ctx.vert_normals[0];
                rig.dab(&Dab::from_brush(&b, c, n), &b);
                rig.mesh.validate().unwrap();
                assert!(rig.mesh.verts.iter().all(|v| v.co.is_finite()));
            }
        }
    }

    #[test]
    fn perf_200_draw_dabs_on_large_sphere() {
        let mut rig = Rig::new(primitives::quad_sphere(1.0, 64));
        assert!(rig.mesh.face_count() > 24_000);
        let b = brush(BrushKind::Draw, 0.12, 0.05);
        let mut rng = Rng(77);
        let t0 = std::time::Instant::now();
        let mut moved = 0usize;
        for _ in 0..200 {
            let c = rng.unit_vec();
            let hit = rig.bvh.raycast(c * 3.0, -c, 10.0).unwrap();
            let r = rig.dab(&Dab::from_brush(&b, hit.point, hit.normal), &b);
            moved += r.moved_verts.len();
        }
        let el = t0.elapsed();
        println!(
            "[sculpt perf] 200 dabs Draw em quad_sphere(1,64) ({} faces): {:?} ({} vértices movidos)",
            rig.mesh.face_count(),
            el,
            moved
        );
        assert!(moved > 0);
        assert!(el.as_secs() < 120, "desempenho absurdo: {el:?}");
    }
}
