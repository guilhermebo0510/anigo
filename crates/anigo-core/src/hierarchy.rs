//! Hierarquia de cena (Fase 1, issue #12): árvore de transformações
//! parent-child com propagação matricial, tipos de entidade e rejeição de
//! ciclos.
//!
//! Por que existe um módulo só para isto: numa cena de anime, acessórios
//! **seguem** partes do personagem (laço na cabeça, espada na mão, jaqueta no
//! tronco). Com uma lista plana de nós, cada um calculando apenas a própria
//! transformação local, essa vinculação é impossível de expressar. A regra é a
//! mesma de qualquer motor: a transformação mundial é a composição da cadeia
//! até a raiz,
//!
//! ```text
//! W(child) = W(parent) × T(local)
//! ```
//!
//! e a ordem de avaliação tem de ser topológica (pai antes de filho). O
//! algoritmo é o de Kahn sobre os índices dos nós, com ordem determinística
//! (ordem de declaração), o que torna o resultado reproduzível — requisito de
//! contrato: dois renderizadores (headless e viewport) têm de chegar à mesma
//! matriz.
//!
//! Um `parent_id` que aponte para um nó inexistente, para si mesmo ou que feche
//! um ciclo é **erro de dados**, não um caso a tratar silenciosamente: o
//! projeto não valida e o comando é revertido (nada de cena corrompida).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::Mat4;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::NodeId;
use crate::math::Transform;
use crate::project::NodeSlot;

/// Tipo especializado de um nó da cena (issue #12).
///
/// O tipo é *dado* do núcleo (serializado no `ProjectState`), não uma etiqueta
/// de UI: é ele que diz se o nó é raiz de personagem, osso do esqueleto, peça de
/// vestuário (que segue a armature), cabelo, acessório, luz ou câmera. A
/// contraparte TypeScript espelha exatamente estes nomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Raiz do personagem: dono da malha deformada e do material canônico.
    CharacterRoot,
    /// Osso do esqueleto humanoide (o vértice segue a paleta de skinning).
    HumanoidBone,
    /// Peça de vestuário — acompanha os ossos do corpo.
    Clothing,
    /// Mechas/cabelo — acompanham a cabeça e possuem dinâmica secundária.
    Hair,
    /// Acessório preso a um osso ou nó (laço, óculos, espada).
    Accessory,
    /// Malha solta/estática: cenário, props, blocagem.
    #[default]
    Mesh,
    /// Luz de cena.
    Light,
    /// Câmera de cena.
    Camera,
    /// Agrupador sem geometria própria (nó intermediário de organização).
    Group,
}

impl NodeKind {
    /// Todos os tipos, em ordem canônica (telemetria, UI e testes).
    pub const ALL: [Self; 9] = [
        Self::CharacterRoot,
        Self::HumanoidBone,
        Self::Clothing,
        Self::Hair,
        Self::Accessory,
        Self::Mesh,
        Self::Light,
        Self::Camera,
        Self::Group,
    ];

    /// Nome canônico em `snake_case` (o mesmo do JSON e do TypeScript).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CharacterRoot => "character_root",
            Self::HumanoidBone => "humanoid_bone",
            Self::Clothing => "clothing",
            Self::Hair => "hair",
            Self::Accessory => "accessory",
            Self::Mesh => "mesh",
            Self::Light => "light",
            Self::Camera => "camera",
            Self::Group => "group",
        }
    }

    /// Parse tolerante (case-insensitive, `-`/espaço aceitos como `_`).
    pub fn parse(value: &str) -> Option<Self> {
        // `&[char; N]` implementa `Pattern`; o array por valor, não.
        let normalized = value.trim().to_ascii_lowercase().replace(&['-', ' '], "_");
        Self::ALL.into_iter().find(|kind| kind.as_str() == normalized)
    }

    /// `true` quando o nó é vestido/preso ao corpo (acompanha a armature e
    /// participa da deformação secundária).
    pub fn is_attachment(self) -> bool {
        matches!(self, Self::Clothing | Self::Hair | Self::Accessory)
    }

    /// `true` quando o nó carrega geometria própria (relevante para culling e
    /// para o passe de pele/cabelo).
    pub fn carries_geometry(self) -> bool {
        matches!(
            self,
            Self::CharacterRoot | Self::HumanoidBone | Self::Clothing | Self::Hair | Self::Accessory | Self::Mesh
        )
    }
}

/// Falha de estrutura da árvore de cena (issue #12).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HierarchyError {
    /// Dois nós declaram o mesmo id.
    #[error("duplicated scene node id '{0}'")]
    DuplicateNodeId(String),
    /// O nó citado não existe na cena.
    #[error("unknown scene node '{node}'")]
    UnknownNode { node: String },
    /// `parent_id` aponta para o próprio nó.
    #[error("node '{node}' cannot be its own parent")]
    SelfParent { node: String },
    /// `parent_id` aponta para um nó que não existe na cena.
    #[error("node '{node}' references unknown parent '{parent}'")]
    UnknownParent { node: String, parent: String },
    /// A cadeia de pais fecha um ciclo (`a → b → c → a`).
    #[error("cyclic scene hierarchy: {path}")]
    Cycle { path: String },
}

/// Resolve as transformações mundiais de uma lista de nós **por índice**.
///
/// `ids[i]`, `parents[i]` e `locals[i]` descrevem o nó `i`; o resultado é
/// `world[i] = world[parent] × locals[i]` (raízes usam só `locals[i]`).
///
/// Uma implementação, dois consumidores: `Scene` (nós do renderer, ids
/// `String`) e a lista de `NodeSlot` do `ProjectState` (ids tipados). Duplicar
/// o algoritmo seria duplicar a chance de divergir.
pub fn resolve_indexed(
    ids: &[&str],
    parents: &[Option<&str>],
    locals: &[Mat4],
) -> Result<Vec<Mat4>, HierarchyError> {
    if ids.len() != parents.len() || ids.len() != locals.len() {
        return Err(HierarchyError::DuplicateNodeId(format!(
            "resolve_indexed: {} ids, {} parents, {} transforms",
            ids.len(),
            parents.len(),
            locals.len()
        )));
    }

    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, id) in ids.iter().enumerate() {
        if index_of.insert(id, index).is_some() {
            return Err(HierarchyError::DuplicateNodeId((*id).to_string()));
        }
    }

    // Pai resolvido para índice (ou raiz) + validações locais.
    let mut parent_index: Vec<Option<usize>> = vec![None; ids.len()];
    for (index, parent) in parents.iter().enumerate() {
        let Some(parent) = parent else { continue };
        if *parent == ids[index] {
            return Err(HierarchyError::SelfParent {
                node: ids[index].to_string(),
            });
        }
        let resolved = index_of
            .get(parent)
            .copied()
            .ok_or_else(|| HierarchyError::UnknownParent {
                node: ids[index].to_string(),
                parent: (*parent).to_string(),
            })?;
        parent_index[index] = Some(resolved);
    }

    // Filhos em ordem de declaração (determinismo do resultado).
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    for (index, parent) in parent_index.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(index);
        }
    }

    // Kahn: raízes primeiro (ordem de declaração), depois os filhos.
    let mut world: Vec<Option<Mat4>> = vec![None; ids.len()];
    let mut queue: VecDeque<usize> = (0..ids.len())
        .filter(|index| parent_index[*index].is_none())
        .collect();
    let mut resolved = 0usize;
    while let Some(index) = queue.pop_front() {
        let parent_world = parent_index[index]
            .and_then(|parent| world[parent])
            .unwrap_or(Mat4::IDENTITY);
        world[index] = Some(parent_world * locals[index]);
        resolved += 1;
        for child in &children[index] {
            queue.push_back(*child);
        }
    }

    if resolved != ids.len() {
        // Sobrou nó sem pai alcançado ⇒ existe ciclo: percorre a cadeia para
        // nomear o caminho no diagnóstico (nunca um "erro desconhecido").
        let start = world
            .iter()
            .position(|matrix| matrix.is_none())
            .unwrap_or_default();
        let mut path: Vec<&str> = Vec::new();
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            if !seen.insert(index) {
                path.push(ids[index]);
                break;
            }
            path.push(ids[index]);
            cursor = parent_index[index];
            if path.len() > ids.len() {
                break;
            }
        }
        path.reverse();
        return Err(HierarchyError::Cycle {
            path: path.join(" -> "),
        });
    }

    Ok(world
        .into_iter()
        .map(|matrix| matrix.unwrap_or(Mat4::IDENTITY))
        .collect())
}

/// Resolve as transformações mundiais da cena persistida, indexadas por id.
pub fn resolve_world_transforms(
    nodes: &[NodeSlot],
) -> Result<BTreeMap<NodeId, Mat4>, HierarchyError> {
    let ids: Vec<&str> = nodes.iter().map(|node| node.node_id.as_str()).collect();
    let parents: Vec<Option<&str>> = nodes
        .iter()
        .map(|node| node.parent_id.as_ref().map(NodeId::as_str))
        .collect();
    let locals: Vec<Mat4> = nodes
        .iter()
        .map(|node| node.transform.to_matrix())
        .collect();
    let world = resolve_indexed(&ids, &parents, &locals)?;
    Ok(nodes
        .iter()
        .zip(world)
        .map(|(node, matrix)| (node.node_id.clone(), matrix))
        .collect())
}

/// Ordem topológica dos nós (pais antes de filhos), como índices na entrada.
pub fn hierarchy_order(nodes: &[NodeSlot]) -> Result<Vec<usize>, HierarchyError> {
    let ids: Vec<&str> = nodes.iter().map(|node| node.node_id.as_str()).collect();
    let parents: Vec<Option<&str>> = nodes
        .iter()
        .map(|node| node.parent_id.as_ref().map(NodeId::as_str))
        .collect();
    let locals: Vec<Mat4> = nodes
        .iter()
        .map(|node| node.transform.to_matrix())
        .collect();
    // A resolução já faz a validação completa (ciclo, pai ausente, pai próprio);
    // a ordem sai da mesma travessia de Kahn para não existirem duas verdades.
    resolve_indexed(&ids, &parents, &locals)?;
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut roots: Vec<usize> = Vec::new();
    let index_of: BTreeMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    for (index, parent) in parents.iter().enumerate() {
        match parent.and_then(|parent| index_of.get(parent).copied()) {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
    }
    let mut order = Vec::with_capacity(nodes.len());
    let mut queue: VecDeque<usize> = roots.into();
    while let Some(index) = queue.pop_front() {
        order.push(index);
        for child in &children[index] {
            queue.push_back(*child);
        }
    }
    Ok(order)
}

/// `true` quando `parent_id` é uma raiz (sem pai declarado).
pub fn is_root(node: &NodeSlot) -> bool {
    node.parent_id.is_none()
}

/// Ancestrais de um nó, do pai imediato até a raiz (ordem de subida).
///
/// Uma cadeia defeituosa (ciclo) para na primeira repetição em vez de girar
/// para sempre — a validação é quem reporta o erro, aqui só se caminha.
pub fn ancestors_of(nodes: &[NodeSlot], id: &NodeId) -> Vec<NodeId> {
    let index_of: BTreeMap<&str, &NodeSlot> = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect();
    let mut chain = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut cursor = index_of
        .get(id.as_str())
        .and_then(|node| node.parent_id.clone());
    while let Some(parent) = cursor {
        if !seen.insert(parent.to_string()) {
            break;
        }
        cursor = index_of
            .get(parent.as_str())
            .and_then(|node| node.parent_id.clone());
        chain.push(parent);
    }
    chain
}

/// Descendentes de um nó (largura primeiro, ordem de declaração).
pub fn descendants_of(nodes: &[NodeSlot], id: &NodeId) -> Vec<NodeId> {
    let mut children: BTreeMap<&str, Vec<&NodeSlot>> = BTreeMap::new();
    for node in nodes {
        if let Some(parent) = &node.parent_id {
            children.entry(parent.as_str()).or_default().push(node);
        }
    }
    let mut out = Vec::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(id.as_str());
    while let Some(current) = queue.pop_front() {
        for child in children.get(current).into_iter().flatten() {
            out.push(child.node_id.clone());
            queue.push_back(child.node_id.as_str());
        }
    }
    out
}

/// Profundidade de um nó na árvore (raiz = 0).
///
/// Sem pânico e sem laço infinito: uma cadeia defeituosa devolve a profundidade
/// observada até o limite do número de nós.
pub fn depth_of(nodes: &[NodeSlot], id: &NodeId) -> usize {
    ancestors_of(nodes, id).len()
}

/// Valida a árvore persistida: ids únicos, pais existentes, sem ciclo.
pub fn validate_hierarchy(nodes: &[NodeSlot]) -> Result<(), HierarchyError> {
    let ids: Vec<&str> = nodes.iter().map(|node| node.node_id.as_str()).collect();
    let parents: Vec<Option<&str>> = nodes
        .iter()
        .map(|node| node.parent_id.as_ref().map(NodeId::as_str))
        .collect();
    let locals: Vec<Mat4> = nodes
        .iter()
        .map(|node| node.transform.to_matrix())
        .collect();
    resolve_indexed(&ids, &parents, &locals).map(|_| ())
}

/// Reparenta um nó **com a garantia de que nenhum ciclo nasce**.
///
/// Devolve `Err(HierarchyError::Cycle)` quando o novo pai está na subárvore do
/// próprio nó (incluindo ele mesmo). É a checagem que o comando
/// `SetNodeParent` usa antes de tocar no estado.
pub fn reparent_checked(
    nodes: &mut [NodeSlot],
    id: &NodeId,
    parent: Option<&NodeId>,
) -> Result<Option<NodeId>, HierarchyError> {
    let index = nodes
        .iter()
        .position(|node| &node.node_id == id)
        .ok_or_else(|| HierarchyError::UnknownNode {
            node: id.to_string(),
        })?;

    if let Some(parent) = parent {
        if parent == id {
            return Err(HierarchyError::SelfParent {
                node: id.to_string(),
            });
        }
        if !nodes.iter().any(|node| &node.node_id == parent) {
            return Err(HierarchyError::UnknownParent {
                node: id.to_string(),
                parent: parent.to_string(),
            });
        }
        let subtree: BTreeSet<String> = descendants_of(nodes, id)
            .into_iter()
            .map(|node| node.to_string())
            .collect();
        if subtree.contains(parent.as_str()) {
            return Err(HierarchyError::Cycle {
                path: format!("{id} -> ... -> {parent} -> {id}"),
            });
        }
    }

    let previous = nodes[index].parent_id.clone();
    nodes[index].parent_id = parent.cloned();
    Ok(previous)
}

/// Transformação local de um conjunto de `NodeSlot` na ordem de `ids`.
pub fn local_matrices(nodes: &[NodeSlot]) -> Vec<Mat4> {
    nodes.iter().map(|node| node.transform.to_matrix()).collect()
}

/// Transformação local de um `Transform` (atalho com documentação de intenção).
pub fn local_matrix(transform: Transform) -> Mat4 {
    transform.to_matrix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Transform;
    use glam::{Quat, Vec3};

    fn slot(id: &str, parent: Option<&str>, transform: Transform) -> NodeSlot {
        NodeSlot {
            node_id: NodeId::from_slug(id),
            name: id.to_string(),
            transform,
            mesh: None,
            material_id: None,
            visible: true,
            parent_id: parent.map(NodeId::from_slug),
            kind: NodeKind::Mesh,
        }
    }

    fn translation(x: f32, y: f32, z: f32) -> Transform {
        Transform {
            translation: Vec3::new(x, y, z),
            ..Default::default()
        }
    }

    fn point_of(matrix: Mat4) -> Vec3 {
        matrix.transform_point3(Vec3::ZERO)
    }

    #[test]
    fn parents_propagate_into_children() {
        let nodes = vec![
            slot("root", None, translation(1.0, 0.0, 0.0)),
            slot("child", Some("root"), translation(0.0, 2.0, 0.0)),
        ];
        let world = resolve_world_transforms(&nodes).expect("árvore válida");
        let child = world
            .get(&NodeId::from_slug("child"))
            .copied()
            .expect("filho resolvido");
        // W(child) = W(parent) × T(local) — translação soma.
        assert!((point_of(child) - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn five_level_hierarchy_matches_glam_expectation() {
        // Cadeia de 5 níveis com rotação e translação em cada passo: a matriz
        // mundial do fundo tem de ser exatamente o produto ordenado.
        let nodes = vec![
            slot("level0", None, translation(0.5, 0.0, 0.0)),
            slot("level1", Some("level0"), translation(0.0, 0.5, 0.0)),
            slot(
                "level2",
                Some("level1"),
                Transform {
                    rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                    ..Default::default()
                },
            ),
            slot("level3", Some("level2"), translation(1.0, 0.0, 0.0)),
            slot("level4", Some("level3"), translation(0.0, 0.0, 0.25)),
        ];
        let expected = Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0))
            * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)
            * Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0))
            * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.25));

        let world = resolve_world_transforms(&nodes).expect("árvore válida");
        let leaf = world
            .get(&NodeId::from_slug("level4"))
            .copied()
            .expect("folha resolvida");
        let difference: Vec<f32> = (leaf - expected)
            .to_cols_array()
            .iter()
            .map(|value| value.abs())
            .collect();
        assert!(
            difference.iter().all(|value| *value < 1e-5),
            "matriz mundial divergiu do esperado do glam: {difference:?}"
        );
        // E a profundidade da folha é 4 (raiz = 0).
        assert_eq!(depth_of(&nodes, &NodeId::from_slug("level4")), 4);
    }

    #[test]
    fn declaration_order_does_not_matter_for_resolution() {
        // O filho declarado antes do pai continua resolvendo: a ordem de
        // avaliação é topológica, não a da lista.
        let nodes = vec![
            slot("child", Some("parent"), translation(0.0, 1.0, 0.0)),
            slot("parent", None, translation(2.0, 0.0, 0.0)),
        ];
        let world = resolve_world_transforms(&nodes).expect("árvore válida");
        let child = world
            .get(&NodeId::from_slug("child"))
            .copied()
            .expect("filho resolvido");
        assert!((point_of(child) - Vec3::new(2.0, 1.0, 0.0)).length() < 1e-5);
        let order = hierarchy_order(&nodes).expect("ordem topológica");
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn cycles_are_rejected_with_a_named_path() {
        let nodes = vec![
            slot("a", Some("c"), translation(0.0, 0.0, 0.0)),
            slot("b", Some("a"), translation(0.0, 0.0, 0.0)),
            slot("c", Some("b"), translation(0.0, 0.0, 0.0)),
        ];
        let error = resolve_world_transforms(&nodes).expect_err("ciclo deve falhar");
        match error {
            HierarchyError::Cycle { path } => {
                assert!(path.contains("nod_a") && path.contains("nod_b") && path.contains("nod_c"));
            }
            other => panic!("esperava Cycle, veio {other:?}"),
        }
    }

    #[test]
    fn self_parent_and_unknown_parent_are_rejected() {
        let nodes = vec![slot("a", Some("a"), Transform::default())];
        assert_eq!(
            validate_hierarchy(&nodes),
            Err(HierarchyError::SelfParent {
                node: "nod_a".to_string()
            })
        );

        let nodes = vec![slot("a", Some("missing"), Transform::default())];
        assert_eq!(
            validate_hierarchy(&nodes),
            Err(HierarchyError::UnknownParent {
                node: "nod_a".to_string(),
                parent: "nod_missing".to_string()
            })
        );
    }

    #[test]
    fn duplicated_ids_are_rejected() {
        let nodes = vec![
            slot("a", None, Transform::default()),
            slot("a", None, Transform::default()),
        ];
        assert_eq!(
            validate_hierarchy(&nodes),
            Err(HierarchyError::DuplicateNodeId("nod_a".to_string()))
        );
    }

    #[test]
    fn descendants_and_ancestors_walk_the_tree_in_order() {
        let nodes = vec![
            slot("root", None, Transform::default()),
            slot("a", Some("root"), Transform::default()),
            slot("b", Some("root"), Transform::default()),
            slot("a1", Some("a"), Transform::default()),
        ];
        let root = NodeId::from_slug("root");
        let descendants: Vec<String> = descendants_of(&nodes, &root)
            .into_iter()
            .map(|node| node.to_string())
            .collect();
        assert_eq!(descendants, vec!["nod_a", "nod_b", "nod_a1"]);

        let leaf = NodeId::from_slug("a1");
        let ancestors: Vec<String> = ancestors_of(&nodes, &leaf)
            .into_iter()
            .map(|node| node.to_string())
            .collect();
        assert_eq!(ancestors, vec!["nod_a", "nod_root"]);
    }

    #[test]
    fn reparent_rejects_cycles_and_reports_the_previous_parent() {
        let mut nodes = vec![
            slot("root", None, Transform::default()),
            slot("child", Some("root"), Transform::default()),
            slot("grand", Some("child"), Transform::default()),
        ];
        let child = NodeId::from_slug("child");
        let grand = NodeId::from_slug("grand");
        let root = NodeId::from_slug("root");

        // Mover o avô para debaixo do neto fecharia ciclo: recusado, estado intacto.
        let error = reparent_checked(&mut nodes, &child, Some(&grand)).expect_err("ciclo");
        assert!(matches!(error, HierarchyError::Cycle { .. }));
        assert_eq!(nodes[1].parent_id.as_ref(), Some(&root));

        // Reparent legítimo devolve o pai anterior (o que a UI usa para o undo).
        let previous = reparent_checked(&mut nodes, &grand, Some(&root)).expect("reparent válido");
        assert_eq!(previous, Some(child));
        assert_eq!(nodes[2].parent_id.as_ref(), Some(&root));

        // E voltar para a raiz devolve o pai anterior de novo.
        let previous = reparent_checked(&mut nodes, &grand, None).expect("reparent para raiz");
        assert_eq!(previous, Some(root));
        assert!(nodes[2].parent_id.is_none());
    }
}
