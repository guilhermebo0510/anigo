//! Render graph (Fase 1, issue #14): o DAG que decide **o que roda, em que
//! ordem e sobre quais alvos**.
//!
//! Antes, a ordem dos passes era uma lista fixa de dois nomes (`outline`, `cel`)
//! lida do contrato. Com sombra facial SDF, camadas de cabelo/roupa,
//! pós-processamento e o depth pre-pass, a ordem deixa de ser um detalhe: os
//! passes ganham **dependências** (o cel precisa do z-buffer resolvido, o
//! pós-processamento precisa da cor final) e os alvos intermediários passam a
//! ser recursos com tempo de vida — e é aí que entra a economia real de VRAM:
//! dois alvos que não coexistem no tempo podem compartilhar a mesma textura.
//!
//! O módulo é deliberadamente puro (nenhum tipo do wgpu): ele descreve, valida e
//! agenda; quem cria texturas e pipelines é o renderer. É essa separação que
//! permite testar ciclo, dependência inexistente e aliasing sem GPU.
//!
//! * a ordem é **topológica** (Kahn) e determinística — desempate por
//!   `order_hint` e, depois, por nome;
//! * um ciclo é erro com **caminho nomeado**, não um `panic!`;
//! * a ativação vem da cena (`RenderGraphSettings`), então o núcleo pode
//!   desligar um passe ou trocar a ordem por snapshot;
//! * o plano de aliasing agrupa recursos com chave compatível
//!   (kind/formato/amostras/tamanho) e tempos de vida disjuntos.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tipo de nó do grafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassKind {
    /// Passe de render com alvo de cor.
    Render,
    /// Passe de compute (morphs, futuros passes de SDF).
    Compute,
    /// Cópia/transferência entre alvos.
    Copy,
}

impl PassKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Compute => "compute",
            Self::Copy => "copy",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "render" => Some(Self::Render),
            "compute" => Some(Self::Compute),
            "copy" => Some(Self::Copy),
            _ => None,
        }
    }
}

/// Um nó de execução do grafo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassNode {
    /// Nome lógico no grafo (`opaque_cel`, `depth_prepass`, …).
    pub name: String,
    pub kind: PassKind,
    /// Nome do passe no render contract (`cel`, `outline`, …). `None` quando o
    /// nó ainda não tem pipeline implementado: ele existe no DAG (com
    /// dependências e recursos) mas é executado só quando a implementação
    /// chegar — nada de trabalho falso.
    pub contract_pass: Option<String>,
    /// Nós que precisam ter rodado antes deste.
    pub depends_on: Vec<String>,
    /// Recursos escritos por este nó.
    pub writes: Vec<String>,
    /// Recursos lidos por este nó.
    pub reads: Vec<String>,
    /// Habilitação declarada pelo contrato (a cena pode desligar ainda mais).
    pub enabled: bool,
    /// Desempate determinístico da ordem topológica.
    pub order_hint: i32,
    /// `z-buffer` resolvido por este nó e consumido por aquele (early-Z).
    pub early_z_for: Option<String>,
}

impl PassNode {
    /// `true` quando o nó roda de fato (tem passe no contrato e está habilitado).
    pub fn is_executable(&self) -> bool {
        self.enabled && self.contract_pass.is_some()
    }
}

/// Classe de recurso transitório gerenciado pelo grafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// Textura de cor transitória (`TextureDimension::D2`).
    TransientColor,
    /// Alvo de profundidade/stencil.
    DepthStencil,
    /// Buffer de storage (vértices deformados pelos morphs, por exemplo).
    StorageBuffer,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TransientColor => "transient_color",
            Self::DepthStencil => "depth_stencil",
            Self::StorageBuffer => "storage_buffer",
        }
    }

    /// `true` quando o recurso pode ser aliased com outro da mesma chave.
    pub fn is_aliasable(self) -> bool {
        matches!(self, Self::TransientColor | Self::DepthStencil)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "transient_color" => Some(Self::TransientColor),
            "depth_stencil" => Some(Self::DepthStencil),
            "storage_buffer" => Some(Self::StorageBuffer),
            _ => None,
        }
    }
}

/// Tamanho declarado de um recurso transitório.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSize {
    /// Mesmo tamanho do alvo de render (largura × altura da viewport).
    Viewport,
    /// Metade da viewport em cada eixo (cadeia de bloom, mip 1).
    HalfViewport,
    /// Tamanho fixo em pixels (máscaras pequenas, LUTs).
    Fixed { width: u32, height: u32 },
}

impl ResourceSize {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "viewport" => Some(Self::Viewport),
            "half_viewport" => Some(Self::HalfViewport),
            _ => value
                .strip_prefix("fixed:")
                .and_then(|rest| rest.split_once('x'))
                .and_then(|(width, height)| {
                    Some(Self::Fixed {
                        width: width.parse().ok()?,
                        height: height.parse().ok()?,
                    })
                }),
        }
    }

    /// Dimensão em pixels para uma viewport dada (nunca zero).
    pub fn resolve(self, viewport: (u32, u32)) -> (u32, u32) {
        match self {
            Self::Viewport => (viewport.0.max(1), viewport.1.max(1)),
            Self::HalfViewport => ((viewport.0 / 2).max(1), (viewport.1 / 2).max(1)),
            Self::Fixed { width, height } => (width.max(1), height.max(1)),
        }
    }

    pub fn as_str(self) -> String {
        match self {
            Self::Viewport => "viewport".to_string(),
            Self::HalfViewport => "half_viewport".to_string(),
            Self::Fixed { width, height } => format!("fixed:{width}x{height}"),
        }
    }
}

/// Recurso transitório declarado pelo contrato.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphResource {
    pub name: String,
    pub kind: ResourceKind,
    /// Formato declarado (ou nome simbólico do contrato, ex.: `targets.depth_format`).
    pub format: String,
    pub size: ResourceSize,
    pub samples: u32,
    /// Primeiro e último nó que usam o recurso (nomes; a ordem vem do grafo).
    pub first_use: Option<String>,
    pub last_use: Option<String>,
}

impl GraphResource {
    /// Bytes de um pixel do formato (fallback 4 = RGBA8).
    pub fn bytes_per_pixel(&self) -> u64 {
        match self.format.as_str() {
            "r8unorm" => 1,
            "rg8unorm" => 2,
            "rgba8unorm" | "rgba8unorm_srgb" | "bgra8unorm" | "depth24plus" | "depth32float" => 4,
            "rgba16float" => 8,
            "rgba32float" => 16,
            _ => 4,
        }
    }

    /// Bytes totais para uma viewport (amostras incluídas: MSAA multiplica a
    /// memória de verdade).
    pub fn bytes_for(&self, viewport: (u32, u32)) -> u64 {
        let (width, height) = self.size.resolve(viewport);
        let samples = self.samples.max(1) as u64;
        self.bytes_per_pixel() * width as u64 * height as u64 * samples
    }
}

/// Grupo de recursos que compartilham a mesma alocação.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasGroup {
    /// Chave de compartilhamento (kind/formato/amostras/tamanho resolvido).
    pub key: String,
    pub members: Vec<String>,
    /// Bytes reservados para o grupo (o maior membro).
    pub bytes_reserved: u64,
}

/// Um recurso com a sua janela de vida na lista agendada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceLifetime {
    pub name: String,
    pub kind: ResourceKind,
    pub first_use: usize,
    pub last_use: usize,
    pub bytes: u64,
    /// `true` quando algum nó habilitado usa o recurso (caso contrário ele nem
    /// é alocado — é o que acontece com os alvos dos passes ainda desabilitados).
    pub allocated: bool,
}

/// Plano de memória para um frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasingPlan {
    pub lifetimes: Vec<ResourceLifetime>,
    pub groups: Vec<AliasGroup>,
    /// Bytes que a aliasing economiza em relação a uma alocação por recurso.
    pub saved_bytes: u64,
    /// Total de alocações distintas que o plano pede.
    pub allocations: usize,
}

/// Erros de construção/validação do grafo.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("render graph tem dois nós com o nome '{node}'")]
    DuplicatePass { node: String },
    #[error("nó '{node}' depende de '{depends_on}', que não existe no grafo")]
    UnknownDependency { node: String, depends_on: String },
    #[error("recurso '{resource}' usado por '{node}' não está declarado no grafo")]
    UnknownResource { node: String, resource: String },
    #[error("ciclo no render graph: {path}")]
    Cycle { path: String },
}

/// O grafo: nós + recursos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderGraph {
    pub nodes: Vec<PassNode>,
    pub resources: Vec<GraphResource>,
}

/// Ajustes vindos da cena (```RenderGraphSettings``` do núcleo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphSettings {
    /// Nós desligados pela cena (além dos que o contrato já declara desligados).
    pub disabled: BTreeSet<String>,
    /// Ordem explícita pedida pela cena — quando presente, os nomes listados
    /// saem naquela ordem e o resto do grafo mantém a ordem topológica
    /// determinística depois deles.
    pub order: Vec<String>,
}

impl RenderGraph {
    pub fn new(nodes: Vec<PassNode>, resources: Vec<GraphResource>) -> Self {
        Self { nodes, resources }
    }

    /// Nó por nome.
    pub fn node(&self, name: &str) -> Option<&PassNode> {
        self.nodes.iter().find(|node| node.name == name)
    }

    /// Valida a topologia: nomes únicos, dependências existentes, recursos
    /// declarados. Um grafo válido é pré-requisito para agendar.
    pub fn validate(&self) -> Result<(), GraphError> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.name.as_str()) {
                return Err(GraphError::DuplicatePass {
                    node: node.name.clone(),
                });
            }
        }
        let known: BTreeSet<&str> = seen.clone();
        for node in &self.nodes {
            for dependency in &node.depends_on {
                if !known.contains(dependency.as_str()) {
                    return Err(GraphError::UnknownDependency {
                        node: node.name.clone(),
                        depends_on: dependency.clone(),
                    });
                }
            }
            let resources: BTreeSet<&str> = self
                .resources
                .iter()
                .map(|resource| resource.name.as_str())
                .collect();
            for resource in node.reads.iter().chain(node.writes.iter()) {
                if !resources.contains(resource.as_str()) {
                    return Err(GraphError::UnknownResource {
                        node: node.name.clone(),
                        resource: resource.clone(),
                    });
                }
            }
        }
        // O ciclo aparece na ordenação; validar antes devolve o caminho cedo.
        self.topological_order().map(|_| ())
    }

    /// Ordem topológica determinística (Kahn).
    ///
    /// Desempate: `order_hint` e, depois, nome — dois frames do mesmo projeto
    /// nunca divergem na ordem (o que quebraria a paridade headless ⇄ viewport).
    pub fn topological_order(&self) -> Result<Vec<String>, GraphError> {
        self.validate_topology()?;
        let mut indegree: BTreeMap<&str, usize> = self
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), 0usize))
            .collect();
        // Grau de entrada = número de dependências **distintas** do próprio nó
        // (uma dependência repetida é uma aresta só).
        for node in &self.nodes {
            let unique: BTreeSet<&str> = node.depends_on.iter().map(String::as_str).collect();
            if let Some(entry) = indegree.get_mut(node.name.as_str()) {
                *entry = unique.len();
            }
        }
        let mut ready: Vec<&PassNode> = self
            .nodes
            .iter()
            .filter(|node| indegree[node.name.as_str()] == 0)
            .collect();
        let mut ordered: Vec<String> = Vec::with_capacity(self.nodes.len());
        let mut emitted: BTreeSet<&str> = BTreeSet::new();
        while !ready.is_empty() {
            // Menor `order_hint`; empate pelo nome (estável e determinístico).
            ready.sort_by(|left, right| {
                left.order_hint
                    .cmp(&right.order_hint)
                    .then_with(|| left.name.cmp(&right.name))
            });
            let next = ready.remove(0);
            ordered.push(next.name.clone());
            emitted.insert(next.name.as_str());
            for node in &self.nodes {
                if !node.depends_on.iter().any(|dep| dep == &next.name) {
                    continue;
                }
                let entry = indegree
                    .get_mut(node.name.as_str())
                    .expect("todo nó tem grau de entrada");
                *entry -= 1;
                if *entry == 0 && !emitted.contains(node.name.as_str()) {
                    ready.push(node);
                }
            }
        }
        if ordered.len() != self.nodes.len() {
            // Os nós que sobraram formam (pelo menos) um ciclo: reportar o
            // caminho é o que transforma um deadlock em diagnóstico.
            let remaining: Vec<&str> = self
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .filter(|name| !emitted.contains(name))
                .collect();
            return Err(GraphError::Cycle {
                path: self.cycle_path(&remaining),
            });
        }
        Ok(ordered)
    }

    /// Nós habilitados, na ordem de execução (topológica ou a pedida pela cena).
    pub fn schedule(&self, settings: &GraphSettings) -> Result<Vec<String>, GraphError> {
        let enabled: BTreeSet<&str> = self
            .nodes
            .iter()
            .filter(|node| node.enabled && node.contract_pass.is_some())
            .map(|node| node.name.as_str())
            .filter(|name| !settings.disabled.contains(*name))
            .collect();
        let topological = self.topological_order()?;
        let mut schedule: Vec<String> = topological
            .iter()
            .filter(|name| enabled.contains(name.as_str()))
            .cloned()
            .collect();
        if !settings.order.is_empty() {
            // A ordem pedida pela cena respeita as dependências: um nó só entra
            // depois de tudo em que ele depende já ter entrado.
            let mut prioritized: Vec<String> = Vec::with_capacity(schedule.len());
            let mut placed: BTreeSet<String> = BTreeSet::new();
            // `fn` aninhada (e não closure): uma closure não pode chamar a si
            // mesma em Rust, e a recursão é o que garante que a ordem pedida
            // nunca coloque um nó antes das dependências dele.
            fn push_with_dependencies(
                name: &str,
                prioritized: &mut Vec<String>,
                placed: &mut BTreeSet<String>,
                enabled: &BTreeSet<&str>,
                all: &RenderGraph,
            ) {
                if placed.contains(name) || !enabled.contains(name) {
                    return;
                }
                if let Some(node) = all.node(name) {
                    for dependency in &node.depends_on {
                        push_with_dependencies(dependency, prioritized, placed, enabled, all);
                    }
                }
                placed.insert(name.to_string());
                prioritized.push(name.to_string());
            }
            for name in &settings.order {
                push_with_dependencies(name, &mut prioritized, &mut placed, &enabled, self);
            }
            for name in &schedule {
                if !placed.contains(name.as_str()) {
                    placed.insert(name.clone());
                    prioritized.push(name.clone());
                }
            }
            schedule = prioritized;
        }
        Ok(schedule)
    }

    /// Plano de memória do frame: janelas de vida + agrupamento por aliasing.
    pub fn aliasing_plan(&self, schedule: &[String], viewport: (u32, u32)) -> AliasingPlan {
        let index_of = |name: &str| schedule.iter().position(|entry| entry == name);
        let mut lifetimes: Vec<ResourceLifetime> = self
            .resources
            .iter()
            .map(|resource| {
                let first = resource
                    .first_use
                    .as_deref()
                    .and_then(index_of)
                    .unwrap_or(usize::MAX);
                let last = resource
                    .last_use
                    .as_deref()
                    .and_then(index_of)
                    .unwrap_or(usize::MIN);
                let allocated = first != usize::MAX && last != usize::MIN && first <= last;
                ResourceLifetime {
                    name: resource.name.clone(),
                    kind: resource.kind,
                    first_use: if allocated { first } else { 0 },
                    last_use: if allocated { last } else { 0 },
                    bytes: resource.bytes_for(viewport),
                    allocated,
                }
            })
            .collect();
        lifetimes.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.name.cmp(&right.name))
        });

        let resource_of = |name: &str| self.resources.iter().find(|entry| entry.name == name);
        let mut groups: Vec<(AliasGroup, Vec<(usize, usize)>, String)> = Vec::new();
        let mut saved_bytes = 0u64;
        let mut allocations = 0usize;
        for lifetime in lifetimes.iter().filter(|entry| entry.allocated) {
            allocations += 1;
            let Some(resource) = resource_of(&lifetime.name) else {
                continue;
            };
            if !resource.kind.is_aliasable() {
                continue;
            }
            let (width, height) = resource.size.resolve(viewport);
            let key = format!(
                "{}|{}|{width}x{height}|{}spp",
                resource.kind.as_str(),
                resource.format,
                resource.samples.max(1)
            );
            let interval = (lifetime.first_use, lifetime.last_use);
            let mut placed = false;
            for (group, intervals, group_key) in groups.iter_mut() {
                if *group_key != key {
                    continue;
                }
                // Só compartilha se as janelas não se sobrepõem (nem encostam:
                // o último uso e o primeiro uso no mesmo índice seriam
                // leituras concorrentes).
                let overlaps = intervals
                    .iter()
                    .any(|(first, last)| interval.0 <= *last && *first <= interval.1);
                if overlaps {
                    continue;
                }
                intervals.push(interval);
                group.members.push(lifetime.name.clone());
                group.bytes_reserved = group.bytes_reserved.max(lifetime.bytes);
                saved_bytes += lifetime.bytes;
                placed = true;
                break;
            }
            if !placed {
                groups.push((
                    AliasGroup {
                        key: key.clone(),
                        members: vec![lifetime.name.clone()],
                        bytes_reserved: lifetime.bytes,
                    },
                    vec![interval],
                    key,
                ));
            }
        }
        AliasingPlan {
            lifetimes,
            groups,
            saved_bytes,
            allocations,
        }
    }

    /// Validação interna sem a checagem de ciclo (a ordenação chama isto antes
    /// de tentar montar o grafo de graus).
    fn validate_topology(&self) -> Result<(), GraphError> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.name.as_str()) {
                return Err(GraphError::DuplicatePass {
                    node: node.name.clone(),
                });
            }
        }
        for node in &self.nodes {
            for dependency in &node.depends_on {
                if !seen.contains(dependency.as_str()) {
                    return Err(GraphError::UnknownDependency {
                        node: node.name.clone(),
                        depends_on: dependency.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Caminho do ciclo entre os nós que sobraram (para a mensagem de erro).
    fn cycle_path(&self, remaining: &[&str]) -> String {
        let start = remaining.first().copied().unwrap_or("?");
        let mut path: Vec<String> = vec![start.to_string()];
        let mut current = start.to_string();
        let mut guard = 0;
        while guard < self.nodes.len() + 1 {
            guard += 1;
            let Some(node) = self.node(&current) else {
                break;
            };
            match node.depends_on.first().and_then(|dep| self.node(dep)) {
                Some(next) if remaining.contains(&next.name.as_str()) => {
                    if next.name == path[0] {
                        path.push(next.name.clone());
                        break;
                    }
                    path.push(next.name.clone());
                    current = next.name.clone();
                }
                _ => break,
            }
        }
        path.join(" → ")
    }
}

/// Constrói o grafo a partir do bloco `render_graph` do render contract.
///
/// O resultado é memoizado: o grafo do contrato é imutável e cada frame pergunta
/// por ele (ordem + aliasing), então ele é construído uma vez por processo.
pub fn graph_from_contract() -> Result<RenderGraph, GraphError> {
    static CACHED: std::sync::OnceLock<Result<RenderGraph, GraphError>> =
        std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| build_graph_from_contract())
        .clone()
}

fn build_graph_from_contract() -> Result<RenderGraph, GraphError> {
    let spec = crate::render_contract::render_graph_spec();
    let mut nodes = Vec::new();
    for node in spec["nodes"].as_array().cloned().unwrap_or_default() {
        let name = node["name"].as_str().unwrap_or_default().to_string();
        nodes.push(PassNode {
            name,
            kind: node["kind"]
                .as_str()
                .and_then(PassKind::parse)
                .unwrap_or(PassKind::Render),
            contract_pass: node["contract_pass"].as_str().map(str::to_string),
            depends_on: string_list(&node["depends_on"]),
            writes: string_list(&node["writes"]),
            reads: string_list(&node["reads"]),
            enabled: node["enabled"].as_bool().unwrap_or(false),
            order_hint: node["order_hint"].as_i64().unwrap_or(0) as i32,
            early_z_for: node["early_z_for"].as_str().map(str::to_string),
        });
    }
    let mut resources = Vec::new();
    for resource in spec["resources"].as_array().cloned().unwrap_or_default() {
        resources.push(GraphResource {
            name: resource["name"].as_str().unwrap_or_default().to_string(),
            kind: resource["kind"]
                .as_str()
                .and_then(ResourceKind::parse)
                .unwrap_or(ResourceKind::TransientColor),
            format: resource["format"].as_str().unwrap_or("rgba8unorm").to_string(),
            size: resource["size"]
                .as_str()
                .and_then(ResourceSize::parse)
                .unwrap_or(ResourceSize::Viewport),
            samples: resource["samples"].as_u64().unwrap_or(1) as u32,
            first_use: resource["first_use"].as_str().map(str::to_string),
            last_use: resource["last_use"].as_str().map(str::to_string),
        });
    }
    Ok(RenderGraph::new(nodes, resources))
}

fn string_list(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, deps: &[&str], order_hint: i32) -> PassNode {
        PassNode {
            name: name.to_string(),
            kind: PassKind::Render,
            contract_pass: Some(name.to_string()),
            depends_on: deps.iter().map(|dep| dep.to_string()).collect(),
            writes: Vec::new(),
            reads: Vec::new(),
            enabled: true,
            order_hint,
            early_z_for: None,
        }
    }

    fn resource(
        name: &str,
        kind: ResourceKind,
        format: &str,
        first: &str,
        last: &str,
    ) -> GraphResource {
        GraphResource {
            name: name.to_string(),
            kind,
            format: format.to_string(),
            size: ResourceSize::Viewport,
            samples: 1,
            first_use: Some(first.to_string()),
            last_use: Some(last.to_string()),
        }
    }

    #[test]
    fn topological_order_is_deterministic_and_respects_dependencies() {
        let graph = RenderGraph::new(
            vec![
                node("opaque_cel", &["depth_prepass"], 20),
                node("depth_prepass", &[], 10),
                node("inverted_hull_outline", &["opaque_cel"], 40),
            ],
            vec![],
        );
        let order = graph.topological_order().expect("grafo válido");
        assert_eq!(
            order,
            vec!["depth_prepass", "opaque_cel", "inverted_hull_outline"]
        );
        // Rodar duas vezes devolve exatamente o mesmo resultado.
        assert_eq!(graph.topological_order().unwrap(), order);
    }

    #[test]
    fn order_hint_breaks_ties_and_ignores_declaration_order() {
        let graph = RenderGraph::new(
            vec![node("late", &[], 30), node("early", &[], 1), node("middle", &[], 20)],
            vec![],
        );
        assert_eq!(
            graph.topological_order().unwrap(),
            vec!["early", "middle", "late"]
        );
    }

    #[test]
    fn cycles_are_rejected_with_a_named_path() {
        let graph = RenderGraph::new(
            vec![
                node("a", &["c"], 0),
                node("b", &["a"], 1),
                node("c", &["b"], 2),
            ],
            vec![],
        );
        match graph.topological_order() {
            Err(GraphError::Cycle { path }) => {
                assert!(path.contains("a") && path.contains("b") && path.contains("c"), "{path}");
            }
            other => panic!("esperava Cycle, veio {other:?}"),
        }
        assert!(matches!(
            graph.validate(),
            Err(GraphError::Cycle { .. })
        ));
    }

    #[test]
    fn self_dependency_and_unknown_dependency_are_rejected() {
        let self_loop = RenderGraph::new(vec![node("a", &["a"], 0)], vec![]);
        assert!(matches!(
            self_loop.topological_order(),
            Err(GraphError::Cycle { .. })
        ));

        let dangling = RenderGraph::new(vec![node("a", &["ghost"], 0)], vec![]);
        assert_eq!(
            dangling.validate(),
            Err(GraphError::UnknownDependency {
                node: "a".to_string(),
                depends_on: "ghost".to_string()
            })
        );
    }

    #[test]
    fn duplicated_names_and_unknown_resources_are_rejected() {
        let duplicated = RenderGraph::new(vec![node("a", &[], 0), node("a", &[], 1)], vec![]);
        assert_eq!(
            duplicated.validate(),
            Err(GraphError::DuplicatePass {
                node: "a".to_string()
            })
        );

        let mut with_resource = node("cel", &[], 0);
        with_resource.writes = vec!["color".to_string()];
        let unknown = RenderGraph::new(vec![with_resource], vec![]);
        assert_eq!(
            unknown.validate(),
            Err(GraphError::UnknownResource {
                node: "cel".to_string(),
                resource: "color".to_string()
            })
        );
    }

    #[test]
    fn schedule_follows_scene_settings() {
        let graph = RenderGraph::new(
            vec![
                node("depth_prepass", &[], 0),
                node("opaque_cel", &["depth_prepass"], 10),
                node("inverted_hull_outline", &["opaque_cel"], 20),
            ],
            vec![],
        );
        let default = graph.schedule(&GraphSettings::default()).unwrap();
        assert_eq!(
            default,
            vec!["depth_prepass", "opaque_cel", "inverted_hull_outline"]
        );

        // Desligar o pre-pass tira só ele do plano.
        let mut settings = GraphSettings::default();
        settings.disabled.insert("depth_prepass".to_string());
        assert_eq!(
            graph.schedule(&settings).unwrap(),
            vec!["opaque_cel", "inverted_hull_outline"]
        );

        // A ordem pedida pela cena vale, mas nunca antes das dependências.
        let settings = GraphSettings {
            disabled: BTreeSet::new(),
            order: vec!["inverted_hull_outline".to_string()],
        };
        assert_eq!(
            graph.schedule(&settings).unwrap(),
            vec!["depth_prepass", "opaque_cel", "inverted_hull_outline"]
        );

        let settings = GraphSettings {
            disabled: BTreeSet::new(),
            order: vec!["opaque_cel".to_string(), "depth_prepass".to_string()],
        };
        assert_eq!(
            graph.schedule(&settings).unwrap(),
            vec!["depth_prepass", "opaque_cel", "inverted_hull_outline"],
            "as dependências entram antes, mesmo quando a cena pede o contrário"
        );
    }

    #[test]
    fn passes_without_pipeline_are_declared_but_never_scheduled() {
        let mut planned = node("post_process", &["opaque_cel"], 50);
        planned.contract_pass = None;
        planned.enabled = false;
        let graph = RenderGraph::new(vec![node("opaque_cel", &[], 10), planned], vec![]);
        assert_eq!(graph.schedule(&GraphSettings::default()).unwrap(), vec!["opaque_cel"]);
    }

    #[test]
    fn aliasing_shares_allocations_only_when_lifetimes_do_not_overlap() {
        let graph = RenderGraph::new(
            vec![
                node("depth_prepass", &[], 0),
                node("hair_and_cloth", &["depth_prepass"], 10),
                node("post_process", &["hair_and_cloth"], 20),
            ],
            vec![
                resource("oit_accum", ResourceKind::TransientColor, "rgba16float", "hair_and_cloth", "hair_and_cloth"),
                resource("bloom_a", ResourceKind::TransientColor, "rgba16float", "post_process", "post_process"),
                resource("bloom_b", ResourceKind::TransientColor, "rgba16float", "post_process", "post_process"),
                resource("depth", ResourceKind::DepthStencil, "depth24plus", "depth_prepass", "post_process"),
            ],
        );
        let schedule = graph.schedule(&GraphSettings::default()).unwrap();
        let plan = graph.aliasing_plan(&schedule, (1920, 1080));

        // oit_accum e bloom_a não coexistem → dividem a alocação.
        let shared = plan
            .groups
            .iter()
            .find(|group| group.members.contains(&"oit_accum".to_string()))
            .expect("grupo do oit_accum");
        assert!(shared.members.contains(&"bloom_a".to_string()));
        assert!(!shared.members.contains(&"bloom_b".to_string()), "ping-pong coexiste");

        // A economia é exatamente o tamanho de um dos membros reusados.
        let expected = 1920u64 * 1080 * 8;
        assert_eq!(plan.saved_bytes, expected);
        // A profundidade nunca entra num grupo de cor (chave diferente).
        let depth_group = plan
            .groups
            .iter()
            .find(|group| group.members.contains(&"depth".to_string()))
            .expect("grupo da profundidade");
        assert_eq!(depth_group.members.len(), 1);
    }

    #[test]
    fn resources_without_an_enabled_user_are_not_allocated() {
        let mut planned = node("post_process", &["opaque_cel"], 20);
        planned.contract_pass = None;
        planned.enabled = false;
        let graph = RenderGraph::new(
            vec![node("opaque_cel", &[], 10), planned],
            vec![
                resource("color", ResourceKind::TransientColor, "rgba8unorm", "opaque_cel", "opaque_cel"),
                resource("bloom", ResourceKind::TransientColor, "rgba16float", "post_process", "post_process"),
            ],
        );
        let plan = graph.aliasing_plan(&graph.schedule(&GraphSettings::default()).unwrap(), (800, 600));
        let bloom = plan.lifetimes.iter().find(|entry| entry.name == "bloom").unwrap();
        assert!(!bloom.allocated, "sem usuário habilitado não há alocação");
        let color = plan.lifetimes.iter().find(|entry| entry.name == "color").unwrap();
        assert!(color.allocated);
        assert_eq!(plan.allocations, 1);
        assert_eq!(plan.saved_bytes, 0);
    }

    #[test]
    fn the_contract_graph_is_valid_and_matches_the_issue_order() {
        let graph = graph_from_contract().expect("o contrato precisa ter um grafo válido");
        graph.validate().expect("grafo do contrato válido");

        let order = graph.schedule(&GraphSettings::default()).unwrap();
        assert_eq!(
            order,
            vec!["depth_prepass", "opaque_cel", "inverted_hull_outline"],
            "a ordem dos passes habilitados segue o DAG"
        );

        // A ordem **declarada** é a do issue, mesmo para os passes que ainda não
        // têm pipeline: o DAG já está montado para recebê-los.
        let declared = graph
            .topological_order()
            .expect("ordem topológica do contrato");
        let expected = [
            "sparse_morph",
            "depth_prepass",
            "face_shadow_sdf",
            "opaque_cel",
            "hair_and_cloth",
            "inverted_hull_outline",
            "post_process",
        ];
        assert_eq!(declared, expected.iter().map(|name| name.to_string()).collect::<Vec<_>>());

        // O pre-pass declara o early-Z do cel — é a ligação que prova a intenção
        // de redução de overdraw.
        let prepass = graph.node("depth_prepass").expect("pre-pass no grafo");
        assert_eq!(prepass.early_z_for.as_deref(), Some("opaque_cel"));
        let cel = graph.node("opaque_cel").expect("cel no grafo");
        assert!(cel.depends_on.contains(&"depth_prepass".to_string()));
    }
}
