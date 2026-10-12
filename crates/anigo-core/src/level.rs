//! Editor de cenário (Fase 1): dados extras do nó e planejadores de comandos puros.
//!
//! O nó da cena (`NodeSlot`) ganhou, de forma **aditiva** (`serde(default)` e omitido do JSON
//! enquanto estiver no padrão, então projetos e fixtures antigos continuam byte a byte iguais):
//! `locked`, `tags`, `folder`, `layer`, `source` e `overrides`. Aqui vivem:
//!
//! * os tipos desses dados ([`NodeSource`], [`NodeOverrides`], [`NodeAttrs`], [`NodePatch`]…);
//! * os **planejadores**: funções puras que, dado o estado do projeto, montam o `Command` certo
//!   (duplicar com remapeamento de ids, apagar subárvores, reparentar mantendo a pose mundial,
//!   agrupar, ocultar em lote). A UI nunca inventa a sequência de comandos — ela pede um plano.
//!
//! Tudo aqui é determinístico e testável sem GPU nem UI.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandError};
use crate::hierarchy::{self, NodeKind};
use crate::ids::{AssetId, NodeId};
use crate::math::Transform;
use crate::project::{AssetEntry, AssetKind, MeshRef, NodeSlot, ProjectState};

/// Prefixo das URIs de ativos da Biblioteca (`anigo://library/<uuid>`).
pub const LIBRARY_URI_PREFIX: &str = "anigo://library/";

/// URI canônica de um ativo da Biblioteca a partir do seu uuid.
pub fn library_uri(uuid: &str) -> String {
    format!("{LIBRARY_URI_PREFIX}{uuid}")
}

/// Extrai o uuid de uma URI de ativo da Biblioteca.
pub fn library_uuid(uri: &str) -> Option<&str> {
    uri.strip_prefix(LIBRARY_URI_PREFIX).filter(|rest| !rest.is_empty())
}

/// Primitivas geradas pelo editor (sem arquivo na Biblioteca).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveShape {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Cone,
}

impl PrimitiveShape {
    pub const ALL: [PrimitiveShape; 5] = [
        PrimitiveShape::Cube,
        PrimitiveShape::Sphere,
        PrimitiveShape::Plane,
        PrimitiveShape::Cylinder,
        PrimitiveShape::Cone,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PrimitiveShape::Cube => "cube",
            PrimitiveShape::Sphere => "sphere",
            PrimitiveShape::Plane => "plane",
            PrimitiveShape::Cylinder => "cylinder",
            PrimitiveShape::Cone => "cone",
        }
    }
}

/// Tipo de luz de um nó `Light` (dado reservado: o renderer congelado só tem a luz principal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightShape {
    Directional,
    Point,
}

/// Origem de um nó da cena: o que o gerou e, portanto, como a malha é obtida.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeSource {
    /// Ativo da Biblioteca: `uri` = `anigo://library/<uuid>`; `revision` = revisão no momento da colocação.
    LibraryAsset {
        uri: String,
        #[serde(default)]
        revision: u32,
    },
    /// Primitiva gerada (cubo, esfera…), 1 m de lado.
    Primitive { shape: PrimitiveShape },
    /// Luz (dado reservado; sem efeito no render congelado).
    Light {
        kind: LightShape,
        color: [f32; 3],
        intensity: f32,
    },
    /// Câmera de cena com lente real: distância focal e sensor em milímetros.
    Camera {
        focal_mm: f32,
        sensor_width_mm: f32,
        sensor_height_mm: f32,
    },
    /// Cartão plano com textura (dado reservado).
    Card {
        #[serde(default)]
        texture_uri: Option<String>,
        width_m: f32,
        height_m: f32,
    },
}

impl NodeSource {
    /// Câmera de cena padrão: 35 mm em sensor Super35 (24,89 × 18,66 mm).
    pub fn default_camera() -> Self {
        NodeSource::Camera {
            focal_mm: 35.0,
            sensor_width_mm: 24.89,
            sensor_height_mm: 18.66,
        }
    }

    /// URI da Biblioteca quando a origem é um ativo.
    pub fn library_uri(&self) -> Option<&str> {
        match self {
            NodeSource::LibraryAsset { uri, .. } => Some(uri),
            _ => None,
        }
    }

    /// Validação de domínio (números finitos e faixas plausíveis).
    pub fn validate(&self) -> Result<(), String> {
        let finite = |name: &str, v: f32| -> Result<(), String> {
            if v.is_finite() {
                Ok(())
            } else {
                Err(format!("{name} must be finite"))
            }
        };
        match self {
            NodeSource::LibraryAsset { uri, .. } => {
                if library_uuid(uri).is_none() {
                    return Err(format!("library asset uri '{uri}' must be anigo://library/<uuid>"));
                }
            }
            NodeSource::Primitive { .. } => {}
            NodeSource::Light { color, intensity, .. } => {
                for c in color {
                    finite("light.color", *c)?;
                }
                finite("light.intensity", *intensity)?;
                if !(0.0..=32.0).contains(intensity) {
                    return Err("light.intensity must be within [0, 32]".into());
                }
            }
            NodeSource::Camera { focal_mm, sensor_width_mm, sensor_height_mm } => {
                for (n, v) in [("focal_mm", *focal_mm), ("sensor_width_mm", *sensor_width_mm), ("sensor_height_mm", *sensor_height_mm)] {
                    finite(n, v)?;
                }
                if !(1.0..=2000.0).contains(focal_mm) {
                    return Err("camera.focal_mm must be within [1, 2000]".into());
                }
                if *sensor_width_mm <= 0.5 || *sensor_height_mm <= 0.5 || *sensor_width_mm > 200.0 || *sensor_height_mm > 200.0 {
                    return Err("camera sensor must be within (0.5, 200] mm".into());
                }
            }
            NodeSource::Card { width_m, height_m, .. } => {
                finite("card.width_m", *width_m)?;
                finite("card.height_m", *height_m)?;
                if *width_m <= 0.0 || *height_m <= 0.0 {
                    return Err("card size must be positive".into());
                }
            }
        }
        Ok(())
    }
}

/// Campo de visão vertical (radianos) de uma lente: `2·atan(sensor_h / (2·focal))`.
pub fn vertical_fov(focal_mm: f32, sensor_height_mm: f32) -> f32 {
    2.0 * (sensor_height_mm / (2.0 * focal_mm.max(0.01))).atan()
}

/// Distância focal (mm) que dá o campo de visão vertical `fov_y` em um sensor de altura `sensor_height_mm`.
pub fn focal_for_fov(fov_y: f32, sensor_height_mm: f32) -> f32 {
    sensor_height_mm / (2.0 * (fov_y * 0.5).tan().max(1e-6))
}

/// Substituições por instância (material básico): `None` = usa o material do ativo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade_color: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_width: Option<f32>,
    /// Material completo da instância (workspace Shading): patch sobre o material base do nó,
    /// aplicado DEPOIS dos três campos acima. Aditivo: omitido enquanto vazio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<crate::command::MaterialPatch>,
}

impl NodeOverrides {
    pub fn is_default(&self) -> bool {
        self.base_color.is_none() && self.shade_color.is_none() && self.outline_width.is_none() && self.material.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        for color in [self.base_color, self.shade_color].into_iter().flatten() {
            for c in color {
                if !c.is_finite() || !(0.0..=1.0).contains(&c) {
                    return Err("override colors must be finite and within [0, 1]".into());
                }
            }
        }
        if let Some(w) = self.outline_width {
            if !w.is_finite() || !(0.0..=0.05).contains(&w) {
                return Err("override outline_width must be within [0, 0.05]".into());
            }
        }
        if let Some(m) = &self.material {
            m.validate()?;
        }
        Ok(())
    }

    /// Aplica as substituições a um material já montado a partir do ativo/padrão do nó.
    pub fn apply_to_material(&self, m: &mut crate::scene::StylizedMaterial) {
        if let Some(c) = self.base_color {
            m.base_color = c;
        }
        if let Some(c) = self.shade_color {
            m.shade_color = c;
        }
        if let Some(w) = self.outline_width {
            m.outline_width = w;
        }
        if let Some(p) = &self.material {
            p.apply_to(m);
        }
    }
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

/// Atributos do nó além de nome/transform/malha — usados por `AddNode` (recriar exatamente um nó
/// removido) e pelos planejadores. Tudo no padrão = `None` no comando.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeAttrs {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<NodeSource>,
    #[serde(default, skip_serializing_if = "NodeOverrides::is_default")]
    pub overrides: NodeOverrides,
}

impl Default for NodeAttrs {
    fn default() -> Self {
        Self {
            visible: true,
            locked: false,
            tags: Vec::new(),
            folder: None,
            layer: None,
            source: None,
            overrides: NodeOverrides::default(),
        }
    }
}

impl NodeAttrs {
    pub fn is_default(&self) -> bool {
        *self == NodeAttrs::default()
    }

    /// Atributos de um nó existente (`None` quando tudo está no padrão).
    pub fn of(slot: &NodeSlot) -> Option<NodeAttrs> {
        let attrs = NodeAttrs {
            visible: slot.visible,
            locked: slot.locked,
            tags: slot.tags.clone(),
            folder: slot.folder.clone(),
            layer: slot.layer.clone(),
            source: slot.source.clone(),
            overrides: slot.overrides.clone(),
        };
        (!attrs.is_default()).then_some(attrs)
    }

    pub fn apply_to(&self, slot: &mut NodeSlot) {
        slot.visible = self.visible;
        slot.locked = self.locked;
        slot.tags = self.tags.clone();
        slot.folder = self.folder.clone();
        slot.layer = self.layer.clone();
        slot.source = self.source.clone();
        slot.overrides = self.overrides.clone();
    }
}

/// Patch parcial das propriedades editáveis de um nó (como `MaterialPatch`).
///
/// `folder`/`layer`: `Some("")` limpa o valor (o JSON não distingue "ausente" de "nulo" aqui).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodePatch {
    pub name: Option<String>,
    pub locked: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
    pub layer: Option<String>,
    pub overrides: Option<NodeOverrides>,
    pub source: Option<NodeSource>,
}

impl NodePatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.locked.is_none()
            && self.tags.is_none()
            && self.folder.is_none()
            && self.layer.is_none()
            && self.overrides.is_none()
            && self.source.is_none()
    }

    /// Validação de domínio dos valores (independe do estado).
    pub fn validate(&self) -> Result<(), CommandError> {
        let bad = |field: &str, detail: String| CommandError::InvalidValue { field: field.to_string(), detail };
        if let Some(name) = &self.name {
            if name.trim().is_empty() {
                return Err(bad("name", "node name must not be empty".into()));
            }
            if name.chars().count() > 120 {
                return Err(bad("name", "node name is limited to 120 characters".into()));
            }
        }
        if let Some(tags) = &self.tags {
            validate_tags(tags).map_err(|d| bad("tags", d))?;
        }
        for (field, value) in [("folder", &self.folder), ("layer", &self.layer)] {
            if let Some(v) = value {
                if v.chars().count() > 120 {
                    return Err(bad(field, "limited to 120 characters".into()));
                }
            }
        }
        if let Some(o) = &self.overrides {
            o.validate().map_err(|d| bad("overrides", d))?;
        }
        if let Some(s) = &self.source {
            s.validate().map_err(|d| bad("source", d))?;
        }
        Ok(())
    }

    /// Aplica o patch ao nó.
    pub fn apply_to(&self, slot: &mut NodeSlot) {
        if let Some(name) = &self.name {
            slot.name = name.trim().to_string();
        }
        if let Some(locked) = self.locked {
            slot.locked = locked;
        }
        if let Some(tags) = &self.tags {
            slot.tags = tags.clone();
        }
        if let Some(folder) = &self.folder {
            slot.folder = (!folder.is_empty()).then(|| folder.clone());
        }
        if let Some(layer) = &self.layer {
            slot.layer = (!layer.is_empty()).then(|| layer.clone());
        }
        if let Some(o) = &self.overrides {
            slot.overrides = o.clone();
        }
        if let Some(s) = &self.source {
            slot.source = Some(s.clone());
        }
    }

    /// Patch que restaura os campos que este patch toca, lidos de `slot` (estado ANTES de aplicar).
    pub fn inverse_of(&self, slot: &NodeSlot) -> NodePatch {
        NodePatch {
            name: self.name.as_ref().map(|_| slot.name.clone()),
            locked: self.locked.map(|_| slot.locked),
            tags: self.tags.as_ref().map(|_| slot.tags.clone()),
            folder: self.folder.as_ref().map(|_| slot.folder.clone().unwrap_or_default()),
            layer: self.layer.as_ref().map(|_| slot.layer.clone().unwrap_or_default()),
            overrides: self.overrides.as_ref().map(|_| slot.overrides.clone()),
            // `source` nunca volta a `None` por patch: o inverso restaura o anterior quando existia.
            source: self.source.as_ref().and_then(|_| slot.source.clone()),
        }
    }

    /// `true` quando aplicar o patch não mudaria nada em `slot`.
    pub fn is_noop_for(&self, slot: &NodeSlot) -> bool {
        let mut probe = slot.clone();
        self.apply_to(&mut probe);
        probe == *slot
    }
}

/// Tags: não vazias, ≤ 64 caracteres, sem repetição, no máximo 64.
pub fn validate_tags(tags: &[String]) -> Result<(), String> {
    if tags.len() > 64 {
        return Err("at most 64 tags per node".into());
    }
    let mut seen = BTreeSet::new();
    for tag in tags {
        if tag.trim().is_empty() {
            return Err("tags must not be empty".into());
        }
        if tag.chars().count() > 64 {
            return Err("tags are limited to 64 characters".into());
        }
        if !seen.insert(tag.to_lowercase()) {
            return Err(format!("duplicated tag '{tag}'"));
        }
    }
    Ok(())
}

/// Uma entrada de `SetNodesTransform` (patch de TRS **local** de um nó).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTransformEntry {
    pub node_id: NodeId,
    #[serde(default)]
    pub translation: Option<[f32; 3]>,
    #[serde(default)]
    pub rotation: Option<[f32; 4]>,
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
}

impl NodeTransformEntry {
    /// Entrada que fixa a transformação local completa.
    pub fn full(node_id: NodeId, t: Transform) -> Self {
        Self {
            node_id,
            translation: Some(t.translation.to_array()),
            rotation: Some(t.rotation.to_array()),
            scale: Some(t.scale.to_array()),
        }
    }
}

/// Remapeamento de id de `DuplicateNodes`: `source` → `copy` (com o nome da cópia).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeCopy {
    pub source: NodeId,
    pub copy: NodeId,
    pub name: String,
}

/// Uma entrada de `ReplaceNodesAsset`: troca malha/origem preservando a transformação.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeAssetEntry {
    pub node_id: NodeId,
    #[serde(default)]
    pub mesh: Option<MeshRef>,
    #[serde(default)]
    pub source: Option<NodeSource>,
    #[serde(default)]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Planejadores
// ---------------------------------------------------------------------------------------------

/// Erro de planejamento (mensagem curta para a barra de status).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("nothing to do")]
    Nothing,
    #[error("{0}")]
    Refused(String),
}

impl From<CommandError> for PlanError {
    fn from(e: CommandError) -> Self {
        PlanError::Refused(e.to_string())
    }
}

/// Nome único na cena (`"Cubo"`, `"Cubo 2"`, `"Cubo 3"`…).
pub fn unique_name(state: &ProjectState, base: &str) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "Node" } else { base };
    let taken: BTreeSet<&str> = state.scene.nodes.iter().map(|n| n.name.as_str()).collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    // "Cubo 2" → base "Cubo"
    let stem = match base.rsplit_once(' ') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) && !tail.is_empty() => head,
        _ => base,
    };
    for n in 2u32.. {
        let candidate = format!("{stem} {n}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Id de nó livre derivado de um rótulo (`nod_cubo`, `nod_cubo_2`…), levando em conta `also_taken`.
pub fn unique_node_id(state: &ProjectState, label: &str, also_taken: &BTreeSet<NodeId>) -> NodeId {
    let first = NodeId::from_label(label);
    let taken = |id: &NodeId| state.scene.node(id).is_some() || also_taken.contains(id);
    if !taken(&first) {
        return first;
    }
    for n in 2u32.. {
        let candidate = NodeId::from_label(&format!("{label} {n}"));
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Nós de `ids` cujo nenhum ancestral também está em `ids` (as "raízes" do conjunto), na ordem da cena.
pub fn top_level(state: &ProjectState, ids: &[NodeId]) -> Vec<NodeId> {
    let set: BTreeSet<&NodeId> = ids.iter().collect();
    let by_id: BTreeMap<&NodeId, &NodeSlot> = state.scene.nodes.iter().map(|n| (&n.node_id, n)).collect();
    state
        .scene
        .nodes
        .iter()
        .filter(|n| set.contains(&n.node_id))
        .filter(|n| {
            let mut cursor = n.parent_id.as_ref();
            let mut guard = 0;
            while let Some(p) = cursor {
                if set.contains(p) {
                    return false;
                }
                cursor = by_id.get(p).and_then(|s| s.parent_id.as_ref());
                guard += 1;
                if guard > state.scene.nodes.len() {
                    break;
                }
            }
            true
        })
        .map(|n| n.node_id.clone())
        .collect()
}

/// `ids` mais todos os descendentes, na ordem da cena (sem repetição).
pub fn closure(state: &ProjectState, ids: &[NodeId]) -> Vec<NodeId> {
    let mut wanted: BTreeSet<NodeId> = ids.iter().cloned().collect();
    for id in ids {
        for d in hierarchy::descendants_of(&state.scene.nodes, id) {
            wanted.insert(d);
        }
    }
    state.scene.nodes.iter().filter(|n| wanted.contains(&n.node_id)).map(|n| n.node_id.clone()).collect()
}

fn is_duplicable(kind: NodeKind) -> bool {
    !matches!(kind, NodeKind::CharacterRoot | NodeKind::HumanoidBone)
}

/// Plano de duplicação: copia `ids` (com subárvores), remapeando ids e pais.
///
/// Retorna o comando e o mapa `origem → cópia`. `offset` (translação local) vai só nas raízes copiadas.
/// O personagem (`CharacterRoot`) não se duplica: ele é único no projeto.
pub fn plan_duplicate(
    state: &ProjectState,
    ids: &[NodeId],
    offset: Option<[f32; 3]>,
) -> Result<(Command, BTreeMap<NodeId, NodeId>), PlanError> {
    let roots = top_level(state, ids);
    if roots.is_empty() {
        return Err(PlanError::Nothing);
    }
    let all = closure(state, &roots);
    let mut taken: BTreeSet<NodeId> = BTreeSet::new();
    let mut names: BTreeSet<String> = state.scene.nodes.iter().map(|n| n.name.clone()).collect();
    let mut copies = Vec::new();
    let mut map = BTreeMap::new();
    for id in &all {
        let slot = state.scene.node(id).ok_or(PlanError::Nothing)?;
        if !is_duplicable(slot.kind) {
            // o personagem e os ossos ficam de fora (e tudo que está sob eles)
            continue;
        }
        let copy_id = unique_node_id(state, &slot.name, &taken);
        taken.insert(copy_id.clone());
        let name = {
            let stem = match slot.name.rsplit_once(' ') {
                Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head.to_string(),
                _ => slot.name.clone(),
            };
            let mut n = 2u32;
            loop {
                let candidate = format!("{stem} {n}");
                if !names.contains(&candidate) {
                    names.insert(candidate.clone());
                    break candidate;
                }
                n += 1;
            }
        };
        map.insert(id.clone(), copy_id.clone());
        copies.push(NodeCopy { source: id.clone(), copy: copy_id, name });
    }
    if copies.is_empty() {
        return Err(PlanError::Refused("the character cannot be duplicated".into()));
    }
    Ok((Command::DuplicateNodes { copies, offset }, map))
}

/// Plano de remoção: apaga as raízes de `ids` (cada `RemoveNode` leva a subárvore).
/// Recusa apagar o projeto inteiro (o documento precisa de ao menos um nó).
pub fn plan_delete(state: &ProjectState, ids: &[NodeId]) -> Result<Command, PlanError> {
    let roots = top_level(state, ids);
    if roots.is_empty() {
        return Err(PlanError::Nothing);
    }
    let doomed = closure(state, &roots);
    if doomed.len() >= state.scene.nodes.len() {
        return Err(PlanError::Refused("the scene must keep at least one node".into()));
    }
    let commands: Vec<Command> = roots.into_iter().map(|node_id| Command::RemoveNode { node_id }).collect();
    Ok(if commands.len() == 1 { commands.into_iter().next().unwrap_or(Command::Batch { commands: vec![] }) } else { Command::Batch { commands } })
}

/// Plano de visibilidade em lote (só os nós que realmente mudam).
pub fn plan_set_visible(state: &ProjectState, ids: &[NodeId], visible: bool) -> Result<Command, PlanError> {
    let commands: Vec<Command> = ids
        .iter()
        .filter_map(|id| state.scene.node(id))
        .filter(|n| n.visible != visible)
        .map(|n| Command::SetNodeVisibility { node_id: n.node_id.clone(), visible })
        .collect();
    batch_or_single(commands)
}

/// Plano de bloqueio em lote.
pub fn plan_set_locked(state: &ProjectState, ids: &[NodeId], locked: bool) -> Result<Command, PlanError> {
    let commands: Vec<Command> = ids
        .iter()
        .filter_map(|id| state.scene.node(id))
        .filter(|n| n.locked != locked)
        .map(|n| Command::SetNodeProps {
            node_id: n.node_id.clone(),
            patch: NodePatch { locked: Some(locked), ..Default::default() },
        })
        .collect();
    batch_or_single(commands)
}

fn batch_or_single(mut commands: Vec<Command>) -> Result<Command, PlanError> {
    match commands.len() {
        0 => Err(PlanError::Nothing),
        1 => Ok(commands.remove(0)),
        _ => Ok(Command::Batch { commands }),
    }
}

/// Reparenta `node` sob `parent` (ou na raiz) **mantendo a pose mundial**: o local é recalculado
/// com a inversa do novo pai. Um comando só (`Batch`), um passo de undo.
pub fn plan_reparent_keep_world(state: &ProjectState, node: &NodeId, parent: Option<&NodeId>) -> Result<Command, PlanError> {
    let slot = state.scene.node(node).ok_or_else(|| PlanError::Refused(format!("unknown node '{node}'")))?;
    if slot.parent_id.as_ref() == parent {
        return Err(PlanError::Nothing);
    }
    let world = state.scene.world_transforms().map_err(|e| PlanError::Refused(e.to_string()))?;
    let own_world = world.get(node).copied().unwrap_or(Mat4::IDENTITY);
    let parent_world = parent.and_then(|p| world.get(p)).copied().unwrap_or(Mat4::IDENTITY);
    let local = parent_world.inverse() * own_world;
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    let mut commands = vec![Command::SetNodeParent { node_id: node.clone(), parent_id: parent.cloned() }];
    let new_t = Transform { translation, rotation: rotation.normalize(), scale };
    if new_t != slot.transform && new_t.translation.is_finite() && new_t.scale.is_finite() {
        commands.push(Command::SetNodeTransform {
            node_id: node.clone(),
            translation: Some(new_t.translation.to_array()),
            rotation: Some(new_t.rotation.to_array()),
            scale: Some(new_t.scale.to_array()),
        });
    }
    batch_or_single(commands)
}

/// Agrupa `ids` sob um novo nó `Group` posicionado na mediana das posições mundiais.
/// Os filhos mantêm a pose mundial.
pub fn plan_group(state: &ProjectState, ids: &[NodeId]) -> Result<(Command, NodeId), PlanError> {
    let roots = top_level(state, ids);
    if roots.len() < 2 {
        return Err(PlanError::Refused("select at least two objects to group".into()));
    }
    if roots.iter().any(|id| state.scene.node(id).is_some_and(|n| !is_duplicable(n.kind))) {
        return Err(PlanError::Refused("the character cannot be grouped".into()));
    }
    let world = state.scene.world_transforms().map_err(|e| PlanError::Refused(e.to_string()))?;
    let mut center = Vec3::ZERO;
    for id in &roots {
        center += world.get(id).copied().unwrap_or(Mat4::IDENTITY).transform_point3(Vec3::ZERO);
    }
    center /= roots.len() as f32;
    // o grupo entra sob o pai comum quando todos têm o mesmo; senão na raiz
    let first_parent = state.scene.node(&roots[0]).and_then(|n| n.parent_id.clone());
    let common = roots.iter().all(|id| state.scene.node(id).map(|n| n.parent_id.clone()) == Some(first_parent.clone()));
    let group_parent = if common { first_parent } else { None };
    let group_parent_world = group_parent.as_ref().and_then(|p| world.get(p)).copied().unwrap_or(Mat4::IDENTITY);
    let group_local_pos = group_parent_world.inverse().transform_point3(center);
    let group_world = Mat4::from_translation(center);

    let gid = unique_node_id(state, "Grupo", &BTreeSet::new());
    let gname = unique_name(state, "Grupo");
    let mut commands = vec![Command::AddNode {
        node_id: gid.clone(),
        name: gname,
        parent_id: group_parent,
        node_kind: NodeKind::Group,
        transform: Transform::from_translation(group_local_pos),
        mesh: None,
        material_id: None,
        index: state.scene.nodes.len() as u32,
        attrs: None,
    }];
    for id in &roots {
        let own_world = world.get(id).copied().unwrap_or(Mat4::IDENTITY);
        let local = group_world.inverse() * own_world;
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        commands.push(Command::SetNodeParent { node_id: id.clone(), parent_id: Some(gid.clone()) });
        commands.push(Command::SetNodeTransform {
            node_id: id.clone(),
            translation: Some(translation.to_array()),
            rotation: Some(rotation.normalize().to_array()),
            scale: Some(scale.to_array()),
        });
    }
    Ok((Command::Batch { commands }, gid))
}

/// Remove o grupo `group`, devolvendo os filhos ao pai do grupo com a pose mundial preservada.
pub fn plan_ungroup(state: &ProjectState, group: &NodeId) -> Result<Command, PlanError> {
    let slot = state.scene.node(group).ok_or_else(|| PlanError::Refused(format!("unknown node '{group}'")))?;
    if slot.kind != NodeKind::Group {
        return Err(PlanError::Refused("not a group".into()));
    }
    let children: Vec<NodeId> = state.scene.children_of(Some(group)).iter().map(|n| n.node_id.clone()).collect();
    let mut commands = Vec::new();
    for child in &children {
        if let Ok(c) = plan_reparent_keep_world(state, child, slot.parent_id.as_ref()) {
            commands.push(c);
        }
    }
    // o grupo (agora sem filhos, no estado final) é removido por último; como o plano é um Batch
    // validado sequencialmente, `RemoveNode` já enxerga o grupo vazio.
    commands.push(Command::RemoveNode { node_id: group.clone() });
    batch_or_single(commands)
}

/// Nós copiados (Ctrl+C): subárvores completas, com as raízes já em coordenadas de MUNDO e o registro
/// dos ativos que elas usam (a colagem funciona mesmo depois de trocar de projeto).
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardNodes {
    pub nodes: Vec<NodeSlot>,
    pub assets: Vec<AssetEntry>,
}

/// Copia `ids` (raízes do conjunto + descendentes). O personagem não se copia.
pub fn clipboard_from(state: &ProjectState, ids: &[NodeId]) -> Option<ClipboardNodes> {
    let roots: Vec<NodeId> = top_level(state, ids)
        .into_iter()
        .filter(|id| state.scene.node(id).is_some_and(|n| is_duplicable(n.kind)))
        .collect();
    if roots.is_empty() {
        return None;
    }
    let world = state.scene.world_transforms().ok()?;
    let mut nodes = Vec::new();
    for id in closure(state, &roots) {
        let mut slot = state.scene.node(&id)?.clone();
        if roots.contains(&id) {
            let (scale, rotation, translation) = world.get(&id).copied().unwrap_or(Mat4::IDENTITY).to_scale_rotation_translation();
            slot.transform = Transform { translation, rotation: rotation.normalize(), scale };
            slot.parent_id = None;
        }
        nodes.push(slot);
    }
    let mut registry: BTreeMap<AssetId, AssetEntry> = BTreeMap::new();
    for n in &nodes {
        if let Some(entry) = n.mesh.as_ref().and_then(|m| state.assets.get(&m.asset_id)) {
            registry.insert(entry.asset_id.clone(), entry.clone());
        }
    }
    Some(ClipboardNodes { nodes, assets: registry.into_values().collect() })
}

/// Plano de colagem: cria cópias com ids/nomes novos (e registra os ativos que faltam) em UM comando.
/// Devolve também os ids das raízes coladas (a nova seleção).
pub fn plan_paste(state: &ProjectState, clip: &ClipboardNodes, offset: [f32; 3]) -> Result<(Command, Vec<NodeId>), PlanError> {
    if clip.nodes.is_empty() {
        return Err(PlanError::Nothing);
    }
    let mut taken: BTreeSet<NodeId> = BTreeSet::new();
    let mut names: BTreeSet<String> = state.scene.nodes.iter().map(|n| n.name.clone()).collect();
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut commands = Vec::new();
    let missing: Vec<AssetEntry> = clip.assets.iter().filter(|a| state.assets.get(&a.asset_id) != Some(a)).cloned().collect();
    if !missing.is_empty() {
        commands.push(Command::PatchAssets { put: missing, remove: vec![] });
    }
    let mut roots = Vec::new();
    for slot in &clip.nodes {
        let new_id = unique_node_id(state, &slot.name, &taken);
        taken.insert(new_id.clone());
        map.insert(slot.node_id.clone(), new_id);
    }
    // pais antes de filhos (a área de transferência guarda a ordem da cena, que não é topológica)
    let depth_in_clip = |slot: &NodeSlot| -> usize {
        let mut d = 0;
        let mut cursor = slot.parent_id.as_ref();
        while let Some(p) = cursor {
            if !map.contains_key(p) || d > clip.nodes.len() {
                break;
            }
            d += 1;
            cursor = clip.nodes.iter().find(|n| &n.node_id == p).and_then(|n| n.parent_id.as_ref());
        }
        d
    };
    let mut ordered: Vec<&NodeSlot> = clip.nodes.iter().collect();
    ordered.sort_by_key(|n| depth_in_clip(n));
    for slot in ordered {
        let new_id = map[&slot.node_id].clone();
        let parent = slot.parent_id.as_ref().and_then(|p| map.get(p)).cloned();
        let mut transform = slot.transform;
        if parent.is_none() {
            transform.translation += Vec3::from_array(offset);
            roots.push(new_id.clone());
        }
        let stem = match slot.name.rsplit_once(' ') {
            Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head.to_string(),
            _ => slot.name.clone(),
        };
        let name = if names.contains(&slot.name) {
            let mut n = 2u32;
            loop {
                let candidate = format!("{stem} {n}");
                if !names.contains(&candidate) {
                    break candidate;
                }
                n += 1;
            }
        } else {
            slot.name.clone()
        };
        names.insert(name.clone());
        commands.push(Command::AddNode {
            node_id: new_id,
            name,
            parent_id: parent,
            node_kind: slot.kind,
            transform,
            mesh: slot.mesh.clone(),
            material_id: slot.material_id.clone(),
            index: (state.scene.nodes.len() + taken.len()) as u32 + 1000,
            attrs: NodeAttrs::of(slot),
        });
    }
    Ok((Command::Batch { commands }, roots))
}

/// Registro do ativo da Biblioteca no projeto (uma entrada por URI; `content_hash` = blake3 da revisão).
pub fn library_asset_entry(uri: &str, byte_len: Option<u64>, content_hash: Option<String>) -> AssetEntry {
    let mut entry = AssetEntry::from_uri(AssetKind::Mesh, uri);
    entry.byte_len = byte_len;
    entry.content_hash = content_hash;
    entry
}

/// Plano de colocação de um ativo da Biblioteca: registra o ativo (se preciso) e cria o nó, num `Batch`.
#[allow(clippy::too_many_arguments)]
pub fn plan_place_library_asset(
    state: &ProjectState,
    uri: &str,
    revision: u32,
    name: &str,
    transform: Transform,
    parent: Option<NodeId>,
    byte_len: Option<u64>,
    content_hash: Option<String>,
    taken: &BTreeSet<NodeId>,
) -> (Vec<Command>, NodeId) {
    let asset_id = AssetId::for_uri(uri);
    let mut commands = Vec::new();
    let needs_register = match state.assets.get(&asset_id) {
        None => true,
        Some(existing) => existing.content_hash != content_hash && content_hash.is_some(),
    };
    if needs_register {
        commands.push(Command::PatchAssets {
            put: vec![library_asset_entry(uri, byte_len, content_hash)],
            remove: vec![],
        });
    }
    let node_id = unique_node_id(state, name, taken);
    commands.push(Command::AddNode {
        node_id: node_id.clone(),
        name: unique_name(state, name),
        parent_id: parent,
        node_kind: NodeKind::Mesh,
        transform,
        mesh: Some(MeshRef { asset_id, primitive_index: 0 }),
        material_id: None,
        index: (state.scene.nodes.len() + taken.len()) as u32,
        attrs: Some(NodeAttrs {
            source: Some(NodeSource::LibraryAsset { uri: uri.to_string(), revision }),
            ..Default::default()
        }),
    });
    (commands, node_id)
}

/// Comando para criar uma primitiva (ou câmera de cena) na cena.
pub fn plan_place_primitive(state: &ProjectState, shape: PrimitiveShape, label: &str, transform: Transform, taken: &BTreeSet<NodeId>) -> (Command, NodeId) {
    let node_id = unique_node_id(state, label, taken);
    (
        Command::AddNode {
            node_id: node_id.clone(),
            name: unique_name(state, label),
            parent_id: None,
            node_kind: NodeKind::Mesh,
            transform,
            mesh: None,
            material_id: None,
            index: (state.scene.nodes.len() + taken.len()) as u32,
            attrs: Some(NodeAttrs { source: Some(NodeSource::Primitive { shape }), ..Default::default() }),
        },
        node_id,
    )
}

/// Comando para criar uma câmera de cena.
pub fn plan_place_camera(state: &ProjectState, label: &str, transform: Transform, source: NodeSource, taken: &BTreeSet<NodeId>) -> (Command, NodeId) {
    let node_id = unique_node_id(state, label, taken);
    (
        Command::AddNode {
            node_id: node_id.clone(),
            name: unique_name(state, label),
            parent_id: None,
            node_kind: NodeKind::Camera,
            transform,
            mesh: None,
            material_id: None,
            index: (state.scene.nodes.len() + taken.len()) as u32,
            attrs: Some(NodeAttrs { source: Some(source), ..Default::default() }),
        },
        node_id,
    )
}

/// Comando `SetCamera` que faz a câmera de render do projeto olhar por um nó câmera de cena
/// (posição/direção do nó + FOV da lente).
pub fn camera_command_from_node(state: &ProjectState, node: &NodeId) -> Result<Command, PlanError> {
    let slot = state.scene.node(node).ok_or_else(|| PlanError::Refused(format!("unknown node '{node}'")))?;
    let Some(NodeSource::Camera { focal_mm, sensor_height_mm, .. }) = &slot.source else {
        return Err(PlanError::Refused("not a scene camera".into()));
    };
    let world = state.scene.world_transforms().map_err(|e| PlanError::Refused(e.to_string()))?;
    let m = world.get(node).copied().unwrap_or(Mat4::IDENTITY);
    let eye = m.transform_point3(Vec3::ZERO);
    let forward = m.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
    let up = m.transform_vector3(Vec3::Y).normalize_or_zero();
    if forward == Vec3::ZERO || up == Vec3::ZERO {
        return Err(PlanError::Refused("degenerate camera transform".into()));
    }
    let focus = state.scene.camera.camera.target.distance(state.scene.camera.camera.eye).max(1.0);
    Ok(Command::SetCamera {
        eye: Some(eye.to_array()),
        target: Some((eye + forward * focus).to_array()),
        up: Some(up.to_array()),
        fov_degrees: Some(vertical_fov(*focal_mm, *sensor_height_mm).to_degrees()),
        projection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::MaterialId;

    fn base() -> ProjectState {
        ProjectState::default()
    }

    fn add_cube(state: &mut ProjectState, label: &str, at: Vec3) -> NodeId {
        let (cmd, id) = plan_place_primitive(state, PrimitiveShape::Cube, label, Transform::from_translation(at), &BTreeSet::new());
        cmd.apply(state).expect("add cube");
        id
    }

    #[test]
    fn unique_names_and_ids_do_not_collide() {
        let mut s = base();
        let a = add_cube(&mut s, "Cubo", Vec3::ZERO);
        let b = add_cube(&mut s, "Cubo", Vec3::X);
        let c = add_cube(&mut s, "Cubo", Vec3::Y);
        assert_ne!(a, b);
        assert_ne!(b, c);
        let names: Vec<&str> = s.scene.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Cubo") && names.contains(&"Cubo 2") && names.contains(&"Cubo 3"));
        assert_eq!(unique_name(&s, "Cubo 2"), "Cubo 4");
    }

    #[test]
    fn duplicate_remaps_ids_parents_and_undoes_exactly() {
        let mut s = base();
        let g = add_cube(&mut s, "Pai", Vec3::new(1.0, 0.0, 0.0));
        let c = add_cube(&mut s, "Filho", Vec3::new(0.0, 1.0, 0.0));
        Command::SetNodeParent { node_id: c.clone(), parent_id: Some(g.clone()) }.apply(&mut s).unwrap();
        let before = s.clone();
        let (cmd, map) = plan_duplicate(&s, &[g.clone()], Some([2.0, 0.0, 0.0])).unwrap();
        let applied = cmd.apply(&mut s).unwrap();
        assert_eq!(s.scene.nodes.len(), before.scene.nodes.len() + 2);
        let g2 = map.get(&g).unwrap();
        let c2 = map.get(&c).unwrap();
        assert_eq!(s.scene.node(c2).unwrap().parent_id.as_ref(), Some(g2), "o filho copiado aponta para o pai copiado");
        assert_eq!(s.scene.node(g2).unwrap().transform.translation.x, 3.0, "offset só na raiz copiada");
        assert_eq!(s.scene.node(c2).unwrap().transform.translation, Vec3::new(0.0, 1.0, 0.0));
        assert!(s.scene.node(g2).unwrap().source.is_some(), "a origem viaja na cópia");
        applied.inverse.apply(&mut s).unwrap();
        assert_eq!(s, before, "undo da duplicação restaura o estado exato");
    }

    #[test]
    fn copy_paste_round_trips_across_projects_and_undoes_in_one_step() {
        let mut a = base();
        let uri = library_uri("0190a1b2-0000-7000-8000-0000000000aa");
        let (cmds, mesa) = plan_place_library_asset(&a, &uri, 3, "Mesa", Transform { translation: Vec3::new(2.0, 0.0, 1.0), scale: Vec3::splat(1.5), ..Default::default() }, None, Some(9), Some("h".into()), &BTreeSet::new());
        Command::Batch { commands: cmds }.apply(&mut a).unwrap();
        let pai = add_cube(&mut a, "Pai", Vec3::new(4.0, 0.0, 0.0));
        Command::SetNodeParent { node_id: mesa.clone(), parent_id: Some(pai.clone()) }.apply(&mut a).unwrap();
        let world_mesa = a.scene.world_transforms().unwrap()[&mesa];
        let clip = clipboard_from(&a, &[mesa.clone(), pai.clone()]).unwrap();
        assert_eq!(clip.nodes.len(), 2);
        assert_eq!(clip.assets.len(), 1, "o registro do ativo viaja junto");
        // colar em OUTRO projeto (sem o ativo registrado)
        let mut b = base();
        let before = b.clone();
        let (cmd, roots) = plan_paste(&b, &clip, [0.0, 0.0, 0.0]).unwrap();
        let applied = cmd.apply(&mut b).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(b.scene.nodes.len(), before.scene.nodes.len() + 2);
        assert!(b.assets.contains_key(&AssetId::for_uri(&uri)));
        let pasted_mesa = b.scene.nodes.iter().find(|n| n.source.as_ref().and_then(NodeSource::library_uri) == Some(uri.as_str())).unwrap();
        let w = b.scene.world_transforms().unwrap()[&pasted_mesa.node_id];
        assert!(w.abs_diff_eq(world_mesa, 1e-4), "a pose mundial é preservada na colagem");
        applied.inverse.apply(&mut b).unwrap();
        assert_eq!(b, before, "um undo desfaz a colagem inteira");
        // o personagem não vai para a área de transferência
        assert!(clipboard_from(&a, &[NodeId::canonical_character()]).is_none());
        // colar duas vezes gera ids e nomes distintos
        let (c1, _) = plan_paste(&b, &clip, [0.5, 0.0, 0.0]).unwrap();
        c1.apply(&mut b).unwrap();
        let (c2, _) = plan_paste(&b, &clip, [0.5, 0.0, 0.0]).unwrap();
        c2.apply(&mut b).unwrap();
        let mut ids: Vec<_> = b.scene.nodes.iter().map(|n| n.node_id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), b.scene.nodes.len());
    }

    #[test]
    fn the_character_cannot_be_duplicated() {
        let s = base();
        let r = plan_duplicate(&s, &[NodeId::canonical_character()], None);
        assert!(matches!(r, Err(PlanError::Refused(_))));
    }

    #[test]
    fn delete_removes_subtrees_and_refuses_to_empty_the_scene() {
        let mut s = base();
        let a = add_cube(&mut s, "A", Vec3::ZERO);
        let b = add_cube(&mut s, "B", Vec3::X);
        Command::SetNodeParent { node_id: b.clone(), parent_id: Some(a.clone()) }.apply(&mut s).unwrap();
        let before = s.clone();
        let cmd = plan_delete(&s, &[a.clone(), b.clone()]).unwrap();
        let applied = cmd.apply(&mut s).unwrap();
        assert!(s.scene.node(&a).is_none() && s.scene.node(&b).is_none());
        applied.inverse.apply(&mut s).unwrap();
        assert_eq!(s, before);
        let all: Vec<NodeId> = s.scene.nodes.iter().map(|n| n.node_id.clone()).collect();
        assert!(matches!(plan_delete(&s, &all), Err(PlanError::Refused(_))));
    }

    #[test]
    fn deleting_a_hidden_locked_node_restores_all_attributes_on_undo() {
        let mut s = base();
        let a = add_cube(&mut s, "A", Vec3::ZERO);
        Command::SetNodeVisibility { node_id: a.clone(), visible: false }.apply(&mut s).unwrap();
        Command::SetNodeProps {
            node_id: a.clone(),
            patch: NodePatch { locked: Some(true), tags: Some(vec!["cenario".into()]), folder: Some("Props".into()), ..Default::default() },
        }
        .apply(&mut s)
        .unwrap();
        let before = s.clone();
        let applied = Command::RemoveNode { node_id: a.clone() }.apply(&mut s).unwrap();
        applied.inverse.apply(&mut s).unwrap();
        assert_eq!(s, before);
        let slot = s.scene.node(&a).unwrap();
        assert!(!slot.visible && slot.locked && slot.tags == ["cenario"] && slot.folder.as_deref() == Some("Props"));
    }

    #[test]
    fn reparent_keeps_the_world_pose() {
        let mut s = base();
        let p = add_cube(&mut s, "P", Vec3::new(5.0, 0.0, 0.0));
        Command::SetNodeTransform { node_id: p.clone(), translation: None, rotation: Some(glam::Quat::from_rotation_y(0.7).to_array()), scale: Some([2.0, 2.0, 2.0]) }
            .apply(&mut s)
            .unwrap();
        let c = add_cube(&mut s, "C", Vec3::new(1.0, 2.0, 3.0));
        let world_before = s.scene.world_transforms().unwrap()[&c];
        let cmd = plan_reparent_keep_world(&s, &c, Some(&p)).unwrap();
        let applied = cmd.apply(&mut s).unwrap();
        let world_after = s.scene.world_transforms().unwrap()[&c];
        assert!(world_before.abs_diff_eq(world_after, 1e-4), "{world_before:?} vs {world_after:?}");
        applied.inverse.apply(&mut s).unwrap();
        assert_eq!(s.scene.node(&c).unwrap().parent_id, None);
    }

    #[test]
    fn group_and_ungroup_preserve_world_poses() {
        let mut s = base();
        let a = add_cube(&mut s, "A", Vec3::new(1.0, 0.0, 0.0));
        let b = add_cube(&mut s, "B", Vec3::new(3.0, 0.0, 2.0));
        let wa = s.scene.world_transforms().unwrap()[&a];
        let wb = s.scene.world_transforms().unwrap()[&b];
        let (cmd, gid) = plan_group(&s, &[a.clone(), b.clone()]).unwrap();
        let applied = cmd.apply(&mut s).unwrap();
        assert_eq!(s.scene.node(&a).unwrap().parent_id.as_ref(), Some(&gid));
        let w = s.scene.world_transforms().unwrap();
        assert!(w[&a].abs_diff_eq(wa, 1e-4) && w[&b].abs_diff_eq(wb, 1e-4));
        // desagrupar
        let un = plan_ungroup(&s, &gid).unwrap();
        un.apply(&mut s).unwrap();
        assert!(s.scene.node(&gid).is_none());
        let w = s.scene.world_transforms().unwrap();
        assert!(w[&a].abs_diff_eq(wa, 1e-4) && w[&b].abs_diff_eq(wb, 1e-4));
        // o agrupar inteiro desfaz num passo
        let mut t = base();
        let a = add_cube(&mut t, "A", Vec3::ZERO);
        let b = add_cube(&mut t, "B", Vec3::X);
        let before = t.clone();
        let (cmd, _) = plan_group(&t, &[a, b]).unwrap();
        let ap = cmd.apply(&mut t).unwrap();
        ap.inverse.apply(&mut t).unwrap();
        assert_eq!(t, before);
        let _ = applied;
    }

    #[test]
    fn place_library_asset_registers_once_and_undoes_in_one_step() {
        let mut s = base();
        let uri = library_uri("0190a1b2-0000-7000-8000-000000000001");
        let before = s.clone();
        let (cmds, id) = plan_place_library_asset(&s, &uri, 2, "Mesa", Transform::from_translation(Vec3::new(1.0, 0.0, 1.0)), None, Some(1234), Some("abc".into()), &BTreeSet::new());
        assert_eq!(cmds.len(), 2, "registro + nó");
        let batch = Command::Batch { commands: cmds };
        let applied = batch.apply(&mut s).unwrap();
        let slot = s.scene.node(&id).unwrap();
        assert_eq!(slot.source.as_ref().and_then(NodeSource::library_uri), Some(uri.as_str()));
        assert!(s.assets.contains_key(&AssetId::for_uri(&uri)));
        // segundo ativo igual: não registra de novo
        let (cmds2, _) = plan_place_library_asset(&s, &uri, 2, "Mesa", Transform::default(), None, Some(1234), Some("abc".into()), &BTreeSet::new());
        assert_eq!(cmds2.len(), 1);
        applied.inverse.apply(&mut s).unwrap();
        assert_eq!(s, before, "um undo desfaz registro e nó");
    }

    #[test]
    fn node_patch_roundtrips_and_detects_noop() {
        let mut s = base();
        let a = add_cube(&mut s, "A", Vec3::ZERO);
        let slot = s.scene.node(&a).unwrap().clone();
        let patch = NodePatch { name: Some("Novo".into()), tags: Some(vec!["x".into(), "y".into()]), folder: Some("Pasta".into()), ..Default::default() };
        assert!(!patch.is_noop_for(&slot));
        let inv = patch.inverse_of(&slot);
        let mut changed = slot.clone();
        patch.apply_to(&mut changed);
        assert_eq!(changed.name, "Novo");
        assert_eq!(changed.folder.as_deref(), Some("Pasta"));
        inv.apply_to(&mut changed);
        assert_eq!(changed, slot);
        assert!(NodePatch::default().is_empty());
        assert!(NodePatch { tags: Some(vec!["a".into(), "A".into()]), ..Default::default() }.validate().is_err());
        assert!(NodePatch { name: Some("  ".into()), ..Default::default() }.validate().is_err());
    }

    #[test]
    fn lens_math_matches_known_values() {
        // full-frame 24 mm de altura, 50 mm → ~26,99 graus
        let fov = vertical_fov(50.0, 24.0).to_degrees();
        assert!((fov - 26.99).abs() < 0.05, "{fov}");
        assert!((focal_for_fov(vertical_fov(35.0, 18.66), 18.66) - 35.0).abs() < 1e-3);
    }

    #[test]
    fn scene_camera_node_drives_the_render_camera() {
        let mut s = base();
        let (cmd, id) = plan_place_camera(
            &s,
            "Câmera",
            Transform { translation: Vec3::new(0.0, 1.5, 4.0), ..Default::default() },
            NodeSource::default_camera(),
            &BTreeSet::new(),
        );
        cmd.apply(&mut s).unwrap();
        let set = camera_command_from_node(&s, &id).unwrap();
        set.apply(&mut s).unwrap();
        let cam = &s.scene.camera.camera;
        assert!((cam.eye - Vec3::new(0.0, 1.5, 4.0)).length() < 1e-5);
        assert!((cam.fov_y.to_degrees() - vertical_fov(35.0, 18.66).to_degrees()).abs() < 1e-3);
        assert!(cam.target.z < cam.eye.z, "olha para -Z");
        let _ = MaterialId::canonical_default();
    }

    // ---------------------------------------------------------------------------------------
    // Fuzz de undo: N comandos aleatórios → desfazer tudo → estado idêntico; refazer tudo → idem.
    // ---------------------------------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            // xorshift64*
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.unit()
        }
    }

    /// Igualdade exata, exceto rotações (aceitam 1e-6: `apply` renormaliza o quaternion).
    fn same_state(a: &ProjectState, b: &ProjectState) -> Result<(), String> {
        if a.scene.nodes.len() != b.scene.nodes.len() {
            return Err(format!("{} vs {} nodes", a.scene.nodes.len(), b.scene.nodes.len()));
        }
        for (x, y) in a.scene.nodes.iter().zip(&b.scene.nodes) {
            let mut x2 = x.clone();
            let mut y2 = y.clone();
            if !x.transform.rotation.abs_diff_eq(y.transform.rotation, 1e-6) {
                return Err(format!("rotation of {} differs", x.node_id));
            }
            x2.transform.rotation = glam::Quat::IDENTITY;
            y2.transform.rotation = glam::Quat::IDENTITY;
            if x2 != y2 {
                return Err(format!("node {} differs:\n{x2:?}\n{y2:?}", x.node_id));
            }
        }
        if a.assets != b.assets {
            return Err("asset registries differ".into());
        }
        Ok(())
    }

    fn random_command(state: &ProjectState, rng: &mut Rng, serial: &mut u32) -> Option<Command> {
        let ids: Vec<NodeId> = state.scene.nodes.iter().map(|n| n.node_id.clone()).collect();
        let pick = |rng: &mut Rng| ids[rng.below(ids.len())].clone();
        let pick_many = |rng: &mut Rng, max: usize| -> Vec<NodeId> {
            let n = 1 + rng.below(max);
            let mut out = Vec::new();
            for _ in 0..n {
                let id = ids[rng.below(ids.len())].clone();
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            out
        };
        *serial += 1;
        match rng.below(14) {
            0 | 1 => {
                let shape = PrimitiveShape::ALL[rng.below(PrimitiveShape::ALL.len())];
                let t = Transform {
                    translation: Vec3::new(rng.range(-10.0, 10.0), rng.range(0.0, 3.0), rng.range(-10.0, 10.0)),
                    rotation: glam::Quat::from_rotation_y(rng.range(0.0, 6.0)),
                    scale: Vec3::splat(rng.range(0.5, 2.0)),
                };
                Some(plan_place_primitive(state, shape, shape.as_str(), t, &BTreeSet::new()).0)
            }
            2 => {
                let uri = library_uri(&format!("0190a1b2-0000-7000-8000-{:012}", rng.below(5)));
                let (cmds, _) = plan_place_library_asset(
                    state,
                    &uri,
                    1,
                    "Ativo",
                    Transform::from_translation(Vec3::new(rng.range(-5.0, 5.0), 0.0, rng.range(-5.0, 5.0))),
                    None,
                    Some(10),
                    Some(format!("h{}", rng.below(2))),
                    &BTreeSet::new(),
                );
                Some(Command::Batch { commands: cmds })
            }
            3 | 4 => {
                let entries: Vec<NodeTransformEntry> = pick_many(rng, 4)
                    .into_iter()
                    .map(|id| NodeTransformEntry {
                        node_id: id,
                        translation: Some([rng.range(-8.0, 8.0), rng.range(0.0, 2.0), rng.range(-8.0, 8.0)]),
                        rotation: (rng.below(2) == 0).then(|| glam::Quat::from_rotation_y(rng.range(0.0, 6.0)).to_array()),
                        scale: (rng.below(3) == 0).then_some([1.5, 1.5, 1.5]),
                    })
                    .collect();
                Some(Command::SetNodesTransform { entries })
            }
            5 => plan_duplicate(state, &pick_many(rng, 3), (rng.below(2) == 0).then_some([1.0, 0.0, 1.0])).ok().map(|p| p.0),
            6 => plan_delete(state, &pick_many(rng, 2)).ok(),
            7 => {
                let node = pick(rng);
                let parent = if rng.below(3) == 0 { None } else { Some(pick(rng)) };
                plan_reparent_keep_world(state, &node, parent.as_ref()).ok()
            }
            8 => plan_group(state, &pick_many(rng, 3)).ok().map(|p| p.0),
            9 => plan_set_visible(state, &pick_many(rng, 3), rng.below(2) == 0).ok(),
            10 => plan_set_locked(state, &pick_many(rng, 3), rng.below(2) == 0).ok(),
            11 => Some(Command::SetNodeProps {
                node_id: pick(rng),
                patch: NodePatch {
                    name: (rng.below(2) == 0).then(|| format!("Nome {serial}")),
                    tags: (rng.below(2) == 0).then(|| vec![format!("t{}", rng.below(4)), format!("u{}", rng.below(4))]),
                    folder: (rng.below(2) == 0).then(|| if rng.below(2) == 0 { String::new() } else { format!("Pasta{}", rng.below(3)) }),
                    overrides: (rng.below(3) == 0).then(|| NodeOverrides { base_color: Some([rng.unit(), 0.5, 0.5, 1.0]), ..Default::default() }),
                    ..Default::default()
                },
            }),
            12 => {
                // trocar o asset de um nó de ativo
                let lib_nodes: Vec<&NodeSlot> = state
                    .scene
                    .nodes
                    .iter()
                    .filter(|n| n.source.as_ref().and_then(NodeSource::library_uri).is_some())
                    .collect();
                let node = *lib_nodes.get(rng.below(lib_nodes.len().max(1)))?;
                let uri = library_uri(&format!("0190a1b2-0000-7000-8000-{:012}", rng.below(5)));
                let id = AssetId::for_uri(&uri);
                if !state.assets.contains_key(&id) {
                    return None;
                }
                Some(Command::ReplaceNodesAsset {
                    entries: vec![NodeAssetEntry {
                        node_id: node.node_id.clone(),
                        mesh: Some(MeshRef { asset_id: id, primitive_index: 0 }),
                        source: Some(NodeSource::LibraryAsset { uri, revision: 2 }),
                        name: Some(format!("Trocado {serial}")),
                    }],
                })
            }
            _ => {
                let id = pick(rng);
                Some(Command::SetNodeVisibility { node_id: id.clone(), visible: !state.scene.node(&id)?.visible })
            }
        }
    }

    #[test]
    fn fuzz_random_scene_commands_undo_all_then_redo_all() {
        for seed in [1u64, 7, 42, 2026] {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut state = base();
            let initial = state.clone();
            let mut history = crate::command::CommandHistory::new(2000);
            let mut serial = 0u32;
            let mut accepted = 0;
            for _ in 0..400 {
                if let Some(cmd) = random_command(&state, &mut rng, &mut serial) {
                    if history.execute(&mut state, cmd).is_ok() {
                        accepted += 1;
                    }
                }
            }
            assert!(accepted > 150, "seed {seed}: só {accepted} comandos aceitos — o gerador está fraco");
            state.validate().expect("o estado final é um projeto válido");
            let after = state.clone();
            let mut undone = 0;
            while history.can_undo() {
                history.undo(&mut state).unwrap_or_else(|e| panic!("seed {seed}: undo {undone} falhou: {e}"));
                state.validate().unwrap_or_else(|e| panic!("seed {seed}: estado inválido após undo {undone}: {e}"));
                undone += 1;
            }
            assert_eq!(undone, accepted);
            same_state(&state, &initial).unwrap_or_else(|e| panic!("seed {seed}: após desfazer tudo: {e}"));
            while history.can_redo() {
                history.redo(&mut state).unwrap_or_else(|e| panic!("seed {seed}: redo falhou: {e}"));
            }
            same_state(&state, &after).unwrap_or_else(|e| panic!("seed {seed}: após refazer tudo: {e}"));
        }
    }

    #[test]
    fn fuzz_every_intermediate_undo_restores_the_previous_state() {
        let mut rng = Rng(0xDEAD_BEEF_1234_5679);
        let mut state = base();
        let mut history = crate::command::CommandHistory::new(2000);
        let mut serial = 0u32;
        for step in 0..250 {
            let Some(cmd) = random_command(&state, &mut rng, &mut serial) else { continue };
            let before = state.clone();
            if history.execute(&mut state, cmd.clone()).is_err() {
                assert_eq!(state, before, "comando recusado não pode alterar o estado");
                continue;
            }
            let mid = state.clone();
            history.undo(&mut state).unwrap();
            same_state(&state, &before).unwrap_or_else(|e| panic!("passo {step} ({cmd:?}): {e}"));
            history.redo(&mut state).unwrap();
            same_state(&state, &mid).unwrap_or_else(|e| panic!("passo {step} redo: {e}"));
        }
    }

    #[test]
    fn new_fields_are_additive_and_absent_from_default_json() {
        let s = base();
        let json = s.to_canonical_json().unwrap();
        let scene_json = serde_json::to_string(&s.scene).unwrap();
        for key in ["locked", "tags", "folder", "layer", "source", "overrides"] {
            assert!(!scene_json.contains(key), "campo novo {key} vazou para o JSON da cena padrão");
        }
        // um nó do JSON antigo (sem os campos) continua carregando e idêntico
        let back = ProjectState::from_json(&json).unwrap();
        assert_eq!(back, s);
        // e um comando antigo (AddNode sem `attrs`) continua válido e serializa igual
        let cmd = Command::AddNode {
            node_id: NodeId::from_slug("x"),
            name: "X".into(),
            parent_id: None,
            node_kind: NodeKind::Mesh,
            transform: Transform::default(),
            mesh: None,
            material_id: None,
            index: 1,
            attrs: None,
        };
        let text = cmd.to_json().unwrap();
        assert!(!text.contains("attrs"), "{text}");
        assert_eq!(Command::from_json(&text).unwrap(), cmd);
    }

    #[test]
    fn scene_commands_survive_a_json_round_trip_with_stable_kinds() {
        let mut s = base();
        let a = add_cube(&mut s, "A", Vec3::ZERO);
        let (dup, _) = plan_duplicate(&s, &[a.clone()], Some([1.0, 0.0, 0.0])).unwrap();
        let cmds = vec![
            dup,
            Command::SetNodesTransform { entries: vec![NodeTransformEntry::full(a.clone(), Transform::from_translation(Vec3::ONE))] },
            Command::SetNodeProps { node_id: a.clone(), patch: NodePatch { locked: Some(true), ..Default::default() } },
            Command::PatchAssets { put: vec![library_asset_entry(&library_uri("abc"), Some(1), None)], remove: vec![] },
            Command::ReplaceNodesAsset { entries: vec![NodeAssetEntry { node_id: a, mesh: None, source: None, name: None }] },
        ];
        let kinds = ["duplicate_nodes", "set_nodes_transform", "set_node_props", "patch_assets", "replace_nodes_asset"];
        for (cmd, kind) in cmds.into_iter().zip(kinds) {
            let json = cmd.to_json().unwrap();
            assert!(json.contains(&format!("\"kind\":\"{kind}\"")), "{json}");
            assert_eq!(Command::from_json(&json).unwrap(), cmd);
        }
    }

    #[test]
    fn library_nodes_are_repaired_on_load_instead_of_turning_into_the_mannequin() {
        let mut s = base();
        let uri = library_uri("0190a1b2-0000-7000-8000-00000000abcd");
        let (cmds, id) = plan_place_library_asset(&s, &uri, 1, "Cadeira", Transform::default(), None, None, None, &BTreeSet::new());
        Command::Batch { commands: cmds }.apply(&mut s).unwrap();
        // simula um arquivo salvo sem o registro do ativo
        let mut broken = s.clone();
        broken.assets.remove(&AssetId::for_uri(&uri));
        let json = serde_json::to_string(&broken).unwrap();
        let loaded = ProjectState::from_json(&json).unwrap();
        let node = loaded.scene.node(&id).unwrap();
        assert_eq!(node.mesh.as_ref().map(|m| &m.asset_id), Some(&AssetId::for_uri(&uri)));
        assert!(loaded.assets.contains_key(&AssetId::for_uri(&uri)));
    }

    #[test]
    fn history_exposes_descriptions_and_a_limit() {
        let mut s = base();
        let mut h = crate::command::CommandHistory::new(3);
        for i in 0..5 {
            let (cmd, _) = plan_place_primitive(&s, PrimitiveShape::Cube, "Cubo", Transform::from_translation(Vec3::splat(i as f32)), &BTreeSet::new());
            h.execute(&mut s, cmd).unwrap();
        }
        assert_eq!(h.undo_descriptions().len(), 3, "o limite descarta os passos mais antigos");
        h.undo(&mut s).unwrap();
        assert_eq!(h.redo_descriptions().len(), 1);
        h.set_limit(1);
        assert_eq!(h.undo_descriptions().len(), 1);
        assert_eq!(h.limit(), 1);
    }

    #[test]
    fn source_validation_rejects_nonsense() {
        assert!(NodeSource::default_camera().validate().is_ok());
        assert!(NodeSource::Camera { focal_mm: 0.1, sensor_width_mm: 24.0, sensor_height_mm: 18.0 }.validate().is_err());
        assert!(NodeSource::LibraryAsset { uri: "file:///x".into(), revision: 1 }.validate().is_err());
        assert!(NodeSource::Primitive { shape: PrimitiveShape::Cone }.validate().is_ok());
    }
}

#[cfg(test)]
mod override_material_tests {
    use super::*;
    use crate::command::MaterialPatch;
    use crate::scene::StylizedMaterial;

    #[test]
    fn default_overrides_serialize_to_nothing_and_old_json_still_reads() {
        let o = NodeOverrides::default();
        assert!(o.is_default());
        assert_eq!(serde_json::to_string(&o).unwrap(), "{}");
        let back: NodeOverrides = serde_json::from_str("{\"outline_width\":0.01}").unwrap();
        assert_eq!(back.outline_width, Some(0.01));
        assert!(back.material.is_none());
    }

    #[test]
    fn material_patch_override_applies_after_the_basic_fields_and_validates() {
        let mut o = NodeOverrides { base_color: Some([0.1, 0.1, 0.1, 1.0]), ..Default::default() };
        o.material = Some(MaterialPatch { base_color: Some([0.9, 0.2, 0.2, 1.0]), rim_intensity: Some(2.0), ..Default::default() });
        assert!(!o.is_default());
        o.validate().unwrap();
        let mut m = StylizedMaterial::default();
        o.apply_to_material(&mut m);
        assert_eq!(m.base_color, [0.9, 0.2, 0.2, 1.0], "o patch completo vence o campo básico");
        assert_eq!(m.rim_intensity, 2.0);
        let json = serde_json::to_string(&o).unwrap();
        let back: NodeOverrides = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
        o.material = Some(MaterialPatch { spec_power: Some(f32::NAN), ..Default::default() });
        assert!(o.validate().is_err());
    }
}
