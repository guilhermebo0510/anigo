//! Pilha de modificadores não destrutivos (equivalente à Modifier Stack do
//! Blender): a malha-base fica intacta e o resultado é reavaliado a partir dela.
//!
//! * Serializável com serde (a pilha é salva no projeto): todos os parâmetros
//!   derivam `Serialize`/`Deserialize`.
//! * Cada entrada tem `enabled_viewport` e `enabled_render`; o Subsurf usa
//!   `levels_viewport` ou `levels_render` conforme o [`EvalMode`].
//! * [`ModifierStack::evaluate`] é puro. [`ModifierStack::evaluate_cached`] guarda,
//!   para cada prefixo da pilha, o resultado e um hash do estado (malha-base +
//!   modo + parâmetros de todos os modificadores até ali), de modo que alterar o
//!   último modificador só recalcula o último passo. O número de modificadores
//!   realmente aplicados é exposto em [`ModifierStack::evaluation_count`].

use super::array::{array, ArrayParams};
use super::decimate::{decimate, DecimateSettings};
use super::displace::{displace, DisplaceParams};
use super::edge_split::{edge_split, EdgeSplitParams};
use super::mirror::{mirror, MirrorParams};
use super::solidify::{solidify, SolidifyParams};
use super::weld::{weld, WeldParams};
use crate::convert::triangulate_polygon;
use crate::error::{ModelError, ModelResult};
use crate::mesh::{EditMesh, Face};
use crate::subdiv::{catmull_clark, SubdivParams};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Contexto de avaliação: visualização no editor ou render final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvalMode {
    Viewport,
    Render,
}

/// Subdivision Surface com níveis separados para viewport e render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubsurfSettings {
    pub levels_viewport: u32,
    pub levels_render: u32,
    /// Demais ajustes (`params.levels` é ignorado na pilha: valem os dois campos acima).
    pub params: SubdivParams,
}

impl Default for SubsurfSettings {
    fn default() -> Self {
        Self {
            levels_viewport: 1,
            levels_render: 2,
            params: SubdivParams::default(),
        }
    }
}

/// Triangulação de n-gons/quads (equivalente ao modificador Triangulate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriangulateParams {
    /// Só triangula faces com pelo menos esta quantidade de cantos (`>= 4`).
    pub min_vertices: u32,
    /// Mantém as normais customizadas dos cantos.
    pub keep_custom_normals: bool,
}

impl Default for TriangulateParams {
    fn default() -> Self {
        Self {
            min_vertices: 4,
            keep_custom_normals: true,
        }
    }
}

/// Triangula as faces com `min_vertices` ou mais cantos (ear clipping/diagonal
/// mais curta de [`triangulate_polygon`]). Atributos de face, UV e normais por corner
/// são herdados; o resultado mantém os vértices e as arestas marcadas.
///
/// Erros: `min_vertices < 4` ou malha inválida.
pub fn triangulate(mesh: &EditMesh, params: &TriangulateParams) -> ModelResult<EditMesh> {
    if params.min_vertices < 4 {
        return Err(ModelError::InvalidParameter(format!(
            "min_vertices deve ser >= 4, recebeu {}",
            params.min_vertices
        )));
    }
    mesh.validate()?;
    let mut out = EditMesh {
        name: mesh.name.clone(),
        verts: mesh.verts.clone(),
        faces: Vec::with_capacity(mesh.triangle_count()),
        edge_attrs: mesh.edge_attrs.clone(),
    };
    for f in &mesh.faces {
        if f.corners.len() < params.min_vertices as usize {
            out.faces.push(f.clone());
            continue;
        }
        let pts: Vec<Vec3> = f.corners.iter().map(|c| mesh.verts[c.vert as usize].co).collect();
        for t in triangulate_polygon(&pts) {
            let mut nf: Face = f.clone();
            nf.corners = vec![f.corners[t[0]], f.corners[t[1]], f.corners[t[2]]];
            if !params.keep_custom_normals {
                for c in &mut nf.corners {
                    c.normal = None;
                }
            }
            out.faces.push(nf);
        }
    }
    Ok(out)
}

/// Um modificador da pilha.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Modifier {
    Subsurf(SubsurfSettings),
    Mirror(MirrorParams),
    Solidify(SolidifyParams),
    Array(ArrayParams),
    Weld(WeldParams),
    EdgeSplit(EdgeSplitParams),
    Displace(DisplaceParams),
    Decimate(DecimateSettings),
    Triangulate(TriangulateParams),
}

impl Modifier {
    /// Nome do tipo para a UI.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Modifier::Subsurf(_) => "Subdivision Surface",
            Modifier::Mirror(_) => "Mirror",
            Modifier::Solidify(_) => "Solidify",
            Modifier::Array(_) => "Array",
            Modifier::Weld(_) => "Weld",
            Modifier::EdgeSplit(_) => "Edge Split",
            Modifier::Displace(_) => "Displace",
            Modifier::Decimate(_) => "Decimate",
            Modifier::Triangulate(_) => "Triangulate",
        }
    }

    /// Aplica este modificador a `mesh` (sem mutar a entrada).
    pub fn apply(&self, mesh: &EditMesh, mode: EvalMode) -> ModelResult<EditMesh> {
        match self {
            Modifier::Subsurf(s) => {
                let mut p = s.params.clone();
                p.levels = match mode {
                    EvalMode::Viewport => s.levels_viewport,
                    EvalMode::Render => s.levels_render,
                };
                catmull_clark(mesh, &p)
            }
            Modifier::Mirror(p) => mirror(mesh, p),
            Modifier::Solidify(p) => solidify(mesh, p),
            Modifier::Array(p) => array(mesh, p),
            Modifier::Weld(p) => weld(mesh, p.distance, p.only_loose),
            Modifier::EdgeSplit(p) => edge_split(mesh, p.angle, p.use_sharp),
            Modifier::Displace(p) => displace(mesh, p),
            Modifier::Decimate(s) => decimate(mesh, s),
            Modifier::Triangulate(p) => triangulate(mesh, p),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Entrada da pilha: um modificador com nome e chaves de visibilidade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifierEntry {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled_viewport: bool,
    #[serde(default = "default_true")]
    pub enabled_render: bool,
    pub modifier: Modifier,
}

impl ModifierEntry {
    /// Entrada habilitada nos dois contextos, nomeada pelo tipo.
    pub fn new(modifier: Modifier) -> Self {
        Self {
            name: modifier.kind_name().to_string(),
            enabled_viewport: true,
            enabled_render: true,
            modifier,
        }
    }

    /// Define o nome (builder).
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn enabled_in(&self, mode: EvalMode) -> bool {
        match mode {
            EvalMode::Viewport => self.enabled_viewport,
            EvalMode::Render => self.enabled_render,
        }
    }
}

// ----------------------------------------------------------------- hashing

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
}

/// Impressão digital completa da malha (diferente de `content_hash`, cobre
/// também UV, normais, cor, skin, máscara e todos os atributos).
fn fingerprint(mesh: &EditMesh) -> u64 {
    let mut h = Fnv::new();
    h.u32(mesh.verts.len() as u32);
    for v in &mesh.verts {
        for c in v.co.to_array() {
            h.f32(c);
        }
        for c in v.color {
            h.f32(c);
        }
        for j in v.joints {
            h.bytes(&j.to_le_bytes());
        }
        for w in v.weights {
            h.f32(w);
        }
        h.f32(v.mask);
        h.f32(v.crease);
        h.bytes(&[v.hidden as u8]);
    }
    h.u32(mesh.faces.len() as u32);
    for f in &mesh.faces {
        h.u32(f.corners.len() as u32);
        for c in &f.corners {
            h.u32(c.vert);
            h.f32(c.uv.x);
            h.f32(c.uv.y);
            match c.normal {
                Some(n) => {
                    h.bytes(&[1]);
                    for k in n.to_array() {
                        h.f32(k);
                    }
                }
                None => h.bytes(&[0]),
            }
        }
        h.bytes(&f.material.to_le_bytes());
        h.bytes(&[f.smooth as u8, f.hidden as u8]);
        h.bytes(&f.face_set.to_le_bytes());
    }
    for ((a, b), e) in &mesh.edge_attrs {
        h.u32(*a);
        h.u32(*b);
        h.bytes(&[e.sharp as u8, e.seam as u8]);
        h.f32(e.crease);
        h.f32(e.bevel_weight);
    }
    h.0
}

fn entry_hash(prev: u64, entry: &ModifierEntry) -> u64 {
    let mut h = Fnv(prev);
    // Debug é determinístico (floats com ida-e-volta exata) e não falha com NaN
    h.bytes(format!("{:?}", entry.modifier).as_bytes());
    h.bytes(&[0xff]);
    h.0
}

fn with_context(e: ModelError, name: &str) -> ModelError {
    match e {
        ModelError::InvalidParameter(s) => ModelError::InvalidParameter(format!("modificador '{name}': {s}")),
        ModelError::UnsupportedTopology(s) => ModelError::UnsupportedTopology(format!("modificador '{name}': {s}")),
        other => other,
    }
}

#[derive(Debug, Clone, Default)]
struct EvalCache {
    /// (hash do prefixo, resultado) por passo ativo.
    steps: Vec<(u64, EditMesh)>,
    applied: u64,
    hits: u64,
}

impl PartialEq for EvalCache {
    /// O cache não faz parte da identidade da pilha.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Pilha ordenada de modificadores.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModifierStack {
    pub modifiers: Vec<ModifierEntry>,
    #[serde(skip)]
    cache: EvalCache,
}

impl ModifierStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Anexa um modificador ao fim da pilha.
    pub fn push(&mut self, entry: ModifierEntry) {
        self.modifiers.push(entry);
    }

    /// Insere na posição `index` (`index <= len`).
    pub fn insert(&mut self, index: usize, entry: ModifierEntry) -> ModelResult<()> {
        if index > self.modifiers.len() {
            return Err(ModelError::InvalidParameter(format!(
                "índice {index} fora da pilha (tamanho {})",
                self.modifiers.len()
            )));
        }
        self.modifiers.insert(index, entry);
        Ok(())
    }

    /// Remove e devolve a entrada `index`.
    pub fn remove(&mut self, index: usize) -> ModelResult<ModifierEntry> {
        if index >= self.modifiers.len() {
            return Err(ModelError::InvalidParameter(format!(
                "índice {index} fora da pilha (tamanho {})",
                self.modifiers.len()
            )));
        }
        Ok(self.modifiers.remove(index))
    }

    /// Move a entrada `from` para a posição `to` (reordenar na UI).
    pub fn move_entry(&mut self, from: usize, to: usize) -> ModelResult<()> {
        let n = self.modifiers.len();
        if from >= n || to >= n {
            return Err(ModelError::InvalidParameter(format!(
                "movimento {from} -> {to} fora da pilha (tamanho {n})"
            )));
        }
        let e = self.modifiers.remove(from);
        self.modifiers.insert(to, e);
        Ok(())
    }

    /// Avalia a pilha sobre `base` sem usar cache. Aplica, na ordem, os
    /// modificadores habilitados para `mode`; devolve um clone de `base` se
    /// nenhum estiver ativo. Em erro devolve o erro do modificador (com o nome
    /// da entrada no texto) e `base` permanece intacta.
    pub fn evaluate(&self, base: &EditMesh, mode: EvalMode) -> ModelResult<EditMesh> {
        let mut current: Option<EditMesh> = None;
        for entry in self.modifiers.iter().filter(|e| e.enabled_in(mode)) {
            let input = current.as_ref().unwrap_or(base);
            let next = entry
                .modifier
                .apply(input, mode)
                .map_err(|e| with_context(e, &entry.name))?;
            current = Some(next);
        }
        Ok(current.unwrap_or_else(|| base.clone()))
    }

    /// Como [`Self::evaluate`], mas reaproveita os resultados dos prefixos que não
    /// mudaram desde a última chamada (mesma malha-base, mesmo modo e mesmos
    /// parâmetros). Alterar o último modificador recalcula só o último passo.
    /// Em caso de erro, os passos anteriores ao que falhou continuam em cache.
    pub fn evaluate_cached(&mut self, base: &EditMesh, mode: EvalMode) -> ModelResult<EditMesh> {
        let ModifierStack { modifiers, cache } = self;
        let active: Vec<&ModifierEntry> = modifiers.iter().filter(|e| e.enabled_in(mode)).collect();
        if active.is_empty() {
            cache.steps.clear();
            return Ok(base.clone());
        }
        let mut h = Fnv::new();
        h.bytes(&fingerprint(base).to_le_bytes());
        h.bytes(&[mode as u8]);
        let mut running = h.0;
        for (i, entry) in active.iter().enumerate() {
            running = entry_hash(running, entry);
            if i < cache.steps.len() && cache.steps[i].0 == running {
                cache.hits += 1;
                continue;
            }
            cache.steps.truncate(i);
            let result = {
                let input = if i == 0 { base } else { &cache.steps[i - 1].1 };
                entry
                    .modifier
                    .apply(input, mode)
                    .map_err(|e| with_context(e, &entry.name))?
            };
            cache.applied += 1;
            cache.steps.push((running, result));
        }
        cache.steps.truncate(active.len());
        Ok(cache.steps[active.len() - 1].1.clone())
    }

    /// Quantas aplicações de modificador foram realmente executadas por
    /// [`Self::evaluate_cached`] desde a criação (ou desde [`Self::reset_counters`]).
    pub fn evaluation_count(&self) -> u64 {
        self.cache.applied
    }

    /// Quantos passos foram reaproveitados do cache.
    pub fn cache_hit_count(&self) -> u64 {
        self.cache.hits
    }

    /// Zera os contadores (mantém o cache).
    pub fn reset_counters(&mut self) {
        self.cache.applied = 0;
        self.cache.hits = 0;
    }

    /// Descarta o cache (por exemplo, depois de desserializar ou de trocar a malha-base).
    pub fn clear_cache(&mut self) {
        self.cache.steps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::EdgeAttr;
    use crate::modifiers::array::ArrayFit;
    use crate::modifiers::decimate::DecimateParams;
    use crate::modifiers::displace::{DisplaceDirection, DisplaceSource};
    use crate::primitives::{self, assert_outward};
    use crate::topology::Topology;

    fn entry(m: Modifier) -> ModifierEntry {
        ModifierEntry::new(m)
    }

    fn subsurf(vp: u32, rd: u32) -> Modifier {
        Modifier::Subsurf(SubsurfSettings {
            levels_viewport: vp,
            levels_render: rd,
            params: SubdivParams::default(),
        })
    }

    fn everything_stack() -> ModifierStack {
        let mut s = ModifierStack::new();
        s.push(entry(Modifier::Weld(WeldParams::default())));
        s.push(entry(Modifier::Mirror(MirrorParams {
            axes: [true, false, true],
            joint_remap: Some(vec![0, 2, 1]),
            ..MirrorParams::default()
        })));
        s.push(entry(Modifier::Solidify(SolidifyParams::default())));
        s.push(entry(Modifier::Array(ArrayParams {
            start_cap: Some(primitives::cube(0.1)),
            fit: Some(ArrayFit::Length(2.0)),
            ..ArrayParams::default()
        })));
        s.push(entry(Modifier::EdgeSplit(EdgeSplitParams::default())));
        s.push(entry(Modifier::Displace(DisplaceParams {
            texture: DisplaceSource::Noise { scale: 0.3, seed: 7 },
            direction: DisplaceDirection::Normal,
            vertex_weights: Some(vec![0.5, 1.0]),
            ..DisplaceParams::default()
        })));
        s.push(entry(Modifier::Decimate(DecimateSettings::Collapse {
            ratio: 0.5,
            params: DecimateParams::default(),
        })));
        s.push(entry(Modifier::Decimate(DecimateSettings::Planar { angle: 0.1 })));
        s.push(entry(subsurf(1, 3)));
        let mut tri = entry(Modifier::Triangulate(TriangulateParams::default())).named("Tris");
        tri.enabled_render = false;
        s.push(tri);
        s
    }

    #[test]
    fn serde_roundtrip_of_full_stack() {
        let s = everything_stack();
        let json = serde_json::to_string(&s).unwrap();
        let back: ModifierStack = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        assert_eq!(back.modifiers.len(), 10);
        // pretty também
        let pretty = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(serde_json::from_str::<ModifierStack>(&pretty).unwrap(), s);
        // campos de visibilidade ausentes assumem "habilitado"
        let minimal = r#"{"modifiers":[{"name":"w","modifier":{"Weld":{}}}]}"#;
        let m: ModifierStack = serde_json::from_str(minimal).unwrap();
        assert!(m.modifiers[0].enabled_viewport && m.modifiers[0].enabled_render);
        assert_eq!(m.modifiers[0].modifier, Modifier::Weld(WeldParams::default()));
        // o cache nunca é serializado
        assert!(!json.contains("steps"));
        // JSON inválido falha sem pânico
        assert!(serde_json::from_str::<ModifierStack>("{\"modifiers\":[{\"name\":1}]}").is_err());
    }

    #[test]
    fn evaluate_applies_in_order_and_respects_modes() {
        let base = primitives::cube(2.0);
        let mut s = ModifierStack::new();
        s.push(entry(subsurf(1, 2)));
        let vp = s.evaluate(&base, EvalMode::Viewport).unwrap();
        let rd = s.evaluate(&base, EvalMode::Render).unwrap();
        assert_eq!(vp.face_count(), 24);
        assert_eq!(rd.face_count(), 96);
        // desabilitar em um contexto
        s.modifiers[0].enabled_render = false;
        assert_eq!(s.evaluate(&base, EvalMode::Render).unwrap(), base);
        assert_eq!(s.evaluate(&base, EvalMode::Viewport).unwrap().face_count(), 24);
        // ordem importa: Mirror depois do Subsurf != Subsurf depois do Mirror
        let mut half = primitives::cube(2.0);
        half.transform_positions(|p| p + Vec3::X);
        let mut a = ModifierStack::new();
        a.push(entry(Modifier::Mirror(MirrorParams::default())));
        a.push(entry(subsurf(1, 1)));
        let mut b = ModifierStack::new();
        b.push(entry(subsurf(1, 1)));
        b.push(entry(Modifier::Mirror(MirrorParams::default())));
        let ra = a.evaluate(&half, EvalMode::Viewport).unwrap();
        let rb = b.evaluate(&half, EvalMode::Viewport).unwrap();
        assert_ne!(ra.content_hash(), rb.content_hash());
        assert!(Topology::build(&ra).is_closed_manifold());
        // pilha vazia devolve clone
        assert_eq!(ModifierStack::new().evaluate(&base, EvalMode::Render).unwrap(), base);
    }

    #[test]
    fn evaluate_error_names_the_modifier_and_leaves_base_intact() {
        let base = primitives::cube(2.0);
        let before = base.clone();
        let mut s = ModifierStack::new();
        s.push(entry(subsurf(1, 1)));
        s.push(
            entry(Modifier::Solidify(SolidifyParams {
                thickness: 0.0,
                ..SolidifyParams::default()
            }))
            .named("Casca"),
        );
        let err = s.evaluate(&base, EvalMode::Viewport).unwrap_err();
        match err {
            ModelError::InvalidParameter(msg) => assert!(msg.contains("Casca"), "{msg}"),
            other => panic!("erro inesperado {other:?}"),
        }
        assert_eq!(base, before);
        // modificador desabilitado não é avaliado (e não falha)
        s.modifiers[1].enabled_viewport = false;
        assert!(s.evaluate(&base, EvalMode::Viewport).is_ok());
        assert!(s.evaluate(&base, EvalMode::Render).is_err());
    }

    #[test]
    fn cache_reuses_unchanged_prefix_and_counts_evaluations() {
        let base = primitives::quad_sphere(1.0, 6);
        let mut s = ModifierStack::new();
        s.push(entry(Modifier::Mirror(MirrorParams::default())));
        s.push(entry(subsurf(1, 2)));
        s.push(entry(Modifier::Displace(DisplaceParams {
            strength: 0.1,
            texture: DisplaceSource::Noise { scale: 0.5, seed: 1 },
            ..DisplaceParams::default()
        })));
        assert_eq!(s.evaluation_count(), 0);
        let first = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 3);
        // segunda avaliação idêntica não recalcula nada
        let second = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 3);
        assert_eq!(s.cache_hit_count(), 3);
        assert_eq!(first, second);
        assert_eq!(first, s.evaluate(&base, EvalMode::Viewport).unwrap(), "cache == avaliação pura");

        // muda só o último: 1 recomputação
        if let Modifier::Displace(d) = &mut s.modifiers[2].modifier {
            d.strength = 0.2;
        }
        let third = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 4);
        assert_ne!(third.content_hash(), second.content_hash());
        assert_eq!(third, s.evaluate(&base, EvalMode::Viewport).unwrap());

        // muda o do meio: 2 recomputações (meio + último)
        s.modifiers[1].modifier = subsurf(2, 2);
        let fourth = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 6);
        assert!(fourth.face_count() > third.face_count());

        // muda o primeiro: tudo
        if let Modifier::Mirror(m) = &mut s.modifiers[0].modifier {
            m.merge_distance = 0.5;
        }
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 9);

        // nada mudou: sem recomputar
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 9);

        // trocar de modo invalida (os níveis do Subsurf podem diferir)
        s.evaluate_cached(&base, EvalMode::Render).unwrap();
        assert_eq!(s.evaluation_count(), 12);
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 15);

        // contadores e limpeza
        s.reset_counters();
        assert_eq!(s.evaluation_count(), 0);
        s.clear_cache();
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 3);
    }

    #[test]
    fn cache_is_invalidated_by_any_base_mesh_attribute_change() {
        let base = primitives::quad_sphere(1.0, 4);
        let mut s = ModifierStack::new();
        s.push(entry(subsurf(1, 1)));
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 1);
        let mut variants: Vec<EditMesh> = Vec::new();
        let mut m = base.clone();
        m.faces[0].corners[0].uv.x += 0.25; // só UV (content_hash não vê)
        variants.push(m);
        let mut m = base.clone();
        m.verts[0].color[1] = 0.1;
        variants.push(m);
        let mut m = base.clone();
        m.verts[1].mask = 0.5;
        variants.push(m);
        let mut m = base.clone();
        m.faces[2].corners[1].normal = Some(Vec3::Y);
        variants.push(m);
        let mut m = base.clone();
        m.edge_attrs.insert(
            (0, 1),
            EdgeAttr {
                bevel_weight: 0.3,
                ..EdgeAttr::default()
            },
        );
        variants.push(m);
        let mut m = base.clone();
        m.verts[2].joints[1] = 4;
        variants.push(m);
        for (i, v) in variants.iter().enumerate() {
            let before = s.evaluation_count();
            s.evaluate_cached(v, EvalMode::Viewport).unwrap();
            assert_eq!(s.evaluation_count(), before + 1, "variante {i} deveria invalidar");
            // voltar para a base original também recalcula (cache guarda só o último estado)
            s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        }
    }

    #[test]
    fn cache_handles_toggle_reorder_insert_remove() {
        let base = primitives::quad_sphere(1.0, 4);
        let mut s = ModifierStack::new();
        s.push(entry(Modifier::Weld(WeldParams::default())));
        s.push(entry(subsurf(1, 1)));
        s.push(entry(Modifier::Triangulate(TriangulateParams::default())));
        let a = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert!(a.faces.iter().all(|f| f.len() == 3));
        assert_eq!(s.evaluation_count(), 3);
        // desabilitar o último: prefixo reaproveitado, nada recalculado
        s.modifiers[2].enabled_viewport = false;
        let b = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 3);
        assert!(b.faces.iter().all(|f| f.len() == 4));
        // reabilitar: o cache do passo 3 foi descartado, recalcula 1
        s.modifiers[2].enabled_viewport = true;
        let c = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 4);
        assert_eq!(a, c);
        // reordenar: Triangulate antes do Subsurf muda tudo a partir da posição 1
        s.move_entry(2, 1).unwrap();
        let d = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 6);
        assert!(d.faces.iter().all(|f| f.len() == 4), "subsurf por último gera quads");
        // inserir no meio e remover
        s.insert(1, entry(Modifier::Weld(WeldParams::default()))).unwrap();
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 9);
        let removed = s.remove(1).unwrap();
        assert_eq!(removed.modifier.kind_name(), "Weld");
        s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        // volta ao estado anterior, mas só o último passo guardado coincide com ... tudo recalcula
        assert!(s.evaluation_count() >= 10);
        // erros de índice
        assert!(s.insert(99, entry(subsurf(1, 1))).is_err());
        assert!(s.remove(99).is_err());
        assert!(s.move_entry(0, 99).is_err());
        assert!(s.move_entry(99, 0).is_err());
        // tudo desabilitado: devolve a base e limpa o cache
        for e in &mut s.modifiers {
            e.enabled_viewport = false;
        }
        assert_eq!(s.evaluate_cached(&base, EvalMode::Viewport).unwrap(), base);
    }

    #[test]
    fn cached_evaluation_error_keeps_valid_prefix() {
        let base = primitives::quad_sphere(1.0, 4);
        let mut s = ModifierStack::new();
        s.push(entry(subsurf(1, 1)));
        s.push(entry(Modifier::Solidify(SolidifyParams {
            thickness: f32::NAN,
            ..SolidifyParams::default()
        })));
        let err = s.evaluate_cached(&base, EvalMode::Viewport).unwrap_err();
        assert!(matches!(err, ModelError::InvalidParameter(_)));
        assert_eq!(s.evaluation_count(), 1, "o passo válido foi calculado e guardado");
        // corrige o parâmetro: só o passo que falhou é recalculado
        s.modifiers[1].modifier = Modifier::Solidify(SolidifyParams {
            thickness: 0.05,
            ..SolidifyParams::default()
        });
        let ok = s.evaluate_cached(&base, EvalMode::Viewport).unwrap();
        assert_eq!(s.evaluation_count(), 2);
        assert!(Topology::build(&ok).is_closed_manifold());
    }

    #[test]
    fn triangulate_modifier() {
        let c = primitives::cube(2.0);
        let t = triangulate(&c, &TriangulateParams::default()).unwrap();
        assert_eq!(t.face_count(), 12);
        assert_eq!(t.vert_count(), 8);
        assert!(Topology::build(&t).is_closed_manifold());
        assert_outward(&t);
        // min_vertices = 5 não toca em quads
        let q = triangulate(
            &c,
            &TriangulateParams {
                min_vertices: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(q, c);
        // n-gon: tampa do cilindro
        let cyl = primitives::cylinder(1.0, 2.0, 10, 1, true);
        let tc = triangulate(
            &cyl,
            &TriangulateParams {
                min_vertices: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(tc.faces.iter().all(|f| f.len() <= 4));
        assert_eq!(tc.face_count(), 10 + 2 * 8);
        assert!(Topology::build(&tc).is_closed_manifold());
        // normais customizadas
        let mut n = c.clone();
        n.faces[0].corners[0].normal = Some(Vec3::Y);
        let keep = triangulate(&n, &TriangulateParams::default()).unwrap();
        assert!(keep.faces.iter().any(|f| f.corners.iter().any(|k| k.normal.is_some())));
        let drop = triangulate(
            &n,
            &TriangulateParams {
                keep_custom_normals: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(drop.faces.iter().all(|f| f.corners.iter().all(|k| k.normal.is_none())));
        // erro
        assert!(triangulate(&c, &TriangulateParams { min_vertices: 3, ..Default::default() }).is_err());
        let mut broken = c.clone();
        broken.faces[0].corners[0].vert = 77;
        assert!(triangulate(&broken, &TriangulateParams::default()).is_err());
    }

    #[test]
    fn every_modifier_applies_through_the_stack() {
        let base = primitives::quad_sphere(1.0, 6);
        for (name, m) in [
            ("subsurf", subsurf(1, 1)),
            ("mirror", Modifier::Mirror(MirrorParams::default())),
            ("solidify", Modifier::Solidify(SolidifyParams::default())),
            ("array", Modifier::Array(ArrayParams::default())),
            ("weld", Modifier::Weld(WeldParams::default())),
            ("edge_split", Modifier::EdgeSplit(EdgeSplitParams::default())),
            ("displace", Modifier::Displace(DisplaceParams::default())),
            ("decimate", Modifier::Decimate(DecimateSettings::default())),
            ("triangulate", Modifier::Triangulate(TriangulateParams::default())),
        ] {
            let mut s = ModifierStack::new();
            s.push(entry(m));
            let out = s.evaluate(&base, EvalMode::Viewport).unwrap_or_else(|e| panic!("{name}: {e}"));
            out.validate().unwrap();
            assert!(out.vert_count() > 0, "{name}");
        }
    }

    #[test]
    fn character_pipeline_half_head_to_subdivided_symmetric_mesh() {
        // metade de uma cabeça (corte no plano x = 0) -> mirror (merge) -> subsurf -> solidify
        let mut head = primitives::quad_sphere(0.5, 6);
        head.transform_positions(|p| Vec3::new(p.x * 0.9, p.y * 1.1, p.z));
        let mut stack = ModifierStack::new();
        stack.push(entry(Modifier::Mirror(MirrorParams {
            bisect: [true, false, false],
            ..MirrorParams::default()
        })));
        stack.push(entry(subsurf(1, 2)));
        let vp = stack.evaluate_cached(&head, EvalMode::Viewport).unwrap();
        let rd = stack.evaluate_cached(&head, EvalMode::Render).unwrap();
        assert!(Topology::build(&vp).is_closed_manifold());
        assert!(Topology::build(&rd).is_closed_manifold());
        assert_eq!(rd.face_count(), vp.face_count() * 4);
        assert_outward(&rd);
        // simetria exata em X
        for v in &rd.verts {
            let r = Vec3::new(-v.co.x, v.co.y, v.co.z);
            assert!(rd.verts.iter().any(|w| (w.co - r).length() < 1e-4));
        }
        // pilha salva e restaurada gera o mesmo resultado
        let json = serde_json::to_string(&stack).unwrap();
        let restored: ModifierStack = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.evaluate(&head, EvalMode::Render).unwrap().content_hash(), rd.content_hash());
    }

    #[test]
    fn determinism_and_large_stack() {
        let base = primitives::quad_sphere(1.0, 24);
        let mut s = ModifierStack::new();
        s.push(entry(Modifier::Mirror(MirrorParams::default())));
        s.push(entry(subsurf(1, 1)));
        let t = std::time::Instant::now();
        let a = s.evaluate(&base, EvalMode::Viewport).unwrap();
        let b = s.evaluate(&base, EvalMode::Viewport).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        assert!(t.elapsed().as_secs_f32() < 30.0);
    }
}
