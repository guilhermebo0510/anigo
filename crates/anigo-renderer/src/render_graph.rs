//! Render graph dinâmico (Fase 1, issue #14): o quadro como um DAG de passes.
//!
//! Antes, o renderer executava uma sequência fixa de passes (`outline`, `cel`)
//! embutida no laço de desenho. À medida que entram sombra facial SDF,
//! transparência de cabelo, pós-processamento e camadas de roupa, a ordem
//! precisa ser **planejada**: dependências explícitas, alvos transitórios com
//! tempo de vida conhecido e um plano determinístico que headless e viewport
//! executam igual.
//!
//! O desenho tem três camadas:
//!
//! * **topologia** ([`RenderGraph`]): nós ([`GraphPass`]) com dependências
//!   (`after`) e recursos lidos/escritos. O contrato congela o grafo canônico
//!   (seis passes de anime); [`RenderGraph::from_contract`] monta o
//!   subgrafo executável de hoje (passes do contrato + `depth_prepass`);
//! * **reconfiguração** ([`GraphOverrides`]): o snapshot do núcleo só ajusta
//!   ordem (`order`), ativação (`disabled`) e o pré-passe (`depth_prepass`) —
//!   nunca a topologia;
//! * **plano** ([`ExecutionPlan`]): ordem topológica (Kahn determinístico) +
//!   atribuição de slots físicos de textura com **aliasing**: recursos com
//!   tempos de vida disjuntos reutilizam o mesmo slot.
//!
//! Um plano inválido nunca derruba o quadro: [`RenderGraph::build`] devolve
//! [`GraphError`] e o chamador cai no plano do contrato com diagnóstico.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Vocabulário do grafo
// ---------------------------------------------------------------------------

/// Semântica de execução de um passe do grafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassKind {
    /// Pré-passe de profundidade: só escreve o z-buffer (Early-Z).
    DepthPrepass,
    /// Passe opaco (cel-shading, cabelo/roupa com alpha test).
    Opaque,
    /// Contorno de hull invertido (lê cor + profundidade, escreve cor).
    Outline,
    /// Pós-processamento sobre o alvo de cor.
    PostProcess,
    /// Passe de computação (fora do `RenderPass`, mesma fila).
    Compute,
    /// Passe futuro/conhecido pelo contrato mas sem semântica própria ainda.
    Custom,
}

/// Semântica de alocação de um recurso do grafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// Alvo de cor transitório (recriado por quadro, candidato a aliasing).
    TransientColor,
    /// Buffer de profundidade/stencil do quadro.
    Depth,
    /// Alvo ping-pong de pós-processamento (lê um, escreve o outro).
    PingPong,
    /// Recurso externo ao grafo (swapchain, textura lida de volta).
    External,
}

/// Um nó do DAG: o que executa, depois de quem, sobre quais recursos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphPass {
    /// Nome do passe (o mesmo do contrato e do snapshot).
    pub name: String,
    /// Semântica de execução.
    pub kind: PassKind,
    /// Dependências explícitas (nomes de passes que executam antes).
    #[serde(default)]
    pub after: Vec<String>,
    /// Recursos lidos pelo passe.
    #[serde(default)]
    pub reads: Vec<String>,
    /// Recursos escritos pelo passe.
    #[serde(default)]
    pub writes: Vec<String>,
    /// `false` tira o passe do plano (o `depth_prepass` nasce desligado).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Um recurso com tempo de vida dentro do quadro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphResource {
    /// Nome do recurso (referenciado por `reads`/`writes`).
    pub name: String,
    /// Semântica de alocação.
    pub kind: ResourceKind,
}

// ---------------------------------------------------------------------------
// Erros
// ---------------------------------------------------------------------------

/// Falha de planejamento do render graph (issue #14).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    /// Dois passes declaram o mesmo nome.
    #[error("duplicated render graph pass '{0}'")]
    DuplicatePass(String),
    /// Nome de passe que não existe no grafo (override ou dependência).
    #[error("unknown render graph pass '{0}'")]
    UnknownPass(String),
    /// Passe referencia um recurso não declarado.
    #[error("pass '{pass}' references unknown resource '{resource}'")]
    UnknownResource { pass: String, resource: String },
    /// Dependências que fecham um ciclo (`a → b → a`).
    #[error("cyclic render graph: {path}")]
    Cycle { path: String },
    /// Nada para executar (todos os passes desligados).
    #[error("render graph plan is empty (all passes disabled)")]
    EmptyPlan,
}

// ---------------------------------------------------------------------------
// Reconfiguração via snapshot
// ---------------------------------------------------------------------------

/// Reconfiguração do plano vinda do snapshot do núcleo (espelho de
/// `RenderState::graph_*`): ordem, ativação e o pré-passe de profundidade.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphOverrides {
    /// Ordem de execução (nomes do contrato). Listados executam nessa
    /// sequência; não listados mantêm a ordem relativa do contrato. As
    /// dependências sempre vencem a dica de ordem.
    #[serde(default)]
    pub order: Vec<String>,
    /// Passes desligados (nomes do contrato). Um `disabled` explícito vence
    /// o `depth_prepass`, que é o interruptor geral do pré-passe.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Liga o pré-passe de profundidade (nó `depth_prepass` no plano).
    #[serde(default)]
    pub depth_prepass: bool,
}

impl GraphOverrides {
    /// Nome do nó de pré-passe de profundidade.
    pub const DEPTH_PREPASS: &'static str = "depth_prepass";
}

// ---------------------------------------------------------------------------
// Grafo + plano
// ---------------------------------------------------------------------------

/// Grafo acíclico dirigido de passes de render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderGraph {
    passes: Vec<GraphPass>,
    resources: Vec<GraphResource>,
}

/// Plano de execução: ordem topológica + slots físicos de textura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    /// Passes na ordem de execução.
    pub order: Vec<String>,
    /// Recurso → slot físico (aliasing: vidas disjuntas dividem o slot).
    pub slots: BTreeMap<String, usize>,
    /// Quantidade de slots físicos (texturas transitórias) do quadro.
    pub slot_count: usize,
}

impl ExecutionPlan {
    /// `true` quando o plano executa o passe nomeado.
    pub fn contains(&self, pass: &str) -> bool {
        self.order.iter().any(|name| name == pass)
    }

    /// Posição de um passe no plano (para afirmar precedência em testes).
    pub fn position(&self, pass: &str) -> Option<usize> {
        self.order.iter().position(|name| name == pass)
    }
}

/// Chave de desempate do Kahn: dica de ordem do snapshot primeiro, depois a
/// ordem de declaração do contrato (determinístico entre headless e viewport).
fn kahn_key(
    hint_pos: &BTreeMap<&str, usize>,
    passes: &[GraphPass],
    index: usize,
) -> (usize, usize) {
    (
        hint_pos
            .get(passes[index].name.as_str())
            .copied()
            .unwrap_or(usize::MAX),
        index,
    )
}

impl RenderGraph {
    /// Passes executáveis de hoje: os passes de render do contrato (na ordem
    /// declarada) mais o nó `depth_prepass` (desligado por padrão).
    ///
    /// Todo passe de render depende do pré-passe — aresta ignorada quando ele
    /// está desligado, o que ancora o pré-passe no início quando ligado.
    pub fn from_contract() -> Self {
        let mut passes: Vec<GraphPass> = crate::render_contract::render_pass_order()
            .into_iter()
            .map(|name| {
                let kind = match name {
                    "outline" => PassKind::Outline,
                    "cel" => PassKind::Opaque,
                    _ => PassKind::Custom,
                };
                let (reads, writes) = match name {
                    "outline" => (
                        vec!["color_main".to_string(), "depth_main".to_string()],
                        vec!["color_main".to_string()],
                    ),
                    _ => (
                        vec!["depth_main".to_string()],
                        vec!["color_main".to_string(), "depth_main".to_string()],
                    ),
                };
                GraphPass {
                    name: name.to_string(),
                    kind,
                    after: vec![GraphOverrides::DEPTH_PREPASS.to_string()],
                    reads,
                    writes,
                    enabled: true,
                }
            })
            .collect();
        passes.push(GraphPass {
            name: GraphOverrides::DEPTH_PREPASS.to_string(),
            kind: PassKind::DepthPrepass,
            after: Vec::new(),
            reads: Vec::new(),
            writes: vec!["depth_main".to_string()],
            enabled: false,
        });
        Self {
            passes,
            resources: Self::canonical_resources(),
        }
    }

    /// Grafo canônico de anime (os seis passes da especificação): pré-passe de
    /// profundidade → sombra facial SDF → cel opaco → cabelo/roupa → contorno →
    /// pós-processamento.
    ///
    /// É a topologia congelada no contrato (`render_graph` do fixture): este
    /// construtor a espelha em código e o teste de drift garante que os dois
    /// nunca divergem.
    pub fn anime_default() -> Self {
        let pass = |name: &str,
                    kind: PassKind,
                    after: &[&str],
                    reads: &[&str],
                    writes: &[&str]| GraphPass {
            name: name.to_string(),
            kind,
            after: after.iter().map(|dep| dep.to_string()).collect(),
            reads: reads.iter().map(|res| res.to_string()).collect(),
            writes: writes.iter().map(|res| res.to_string()).collect(),
            enabled: true,
        };
        Self {
            passes: vec![
                pass("depth_prepass", PassKind::DepthPrepass, &[], &[], &["depth_main"]),
                pass(
                    "face_shadow_sdf",
                    PassKind::Custom,
                    &["depth_prepass"],
                    &["depth_main"],
                    &["face_shadow_mask"],
                ),
                pass(
                    "opaque_cel",
                    PassKind::Opaque,
                    &["face_shadow_sdf"],
                    &["depth_main"],
                    &["color_main", "depth_main"],
                ),
                pass(
                    "hair_cloth",
                    PassKind::Opaque,
                    &["opaque_cel"],
                    &["color_main", "depth_main"],
                    &["color_main"],
                ),
                pass(
                    "outline",
                    PassKind::Outline,
                    &["hair_cloth"],
                    &["color_main", "depth_main"],
                    &["color_main"],
                ),
                pass(
                    "postprocess",
                    PassKind::PostProcess,
                    &["outline"],
                    &["color_main"],
                    &["color_main"],
                ),
            ],
            resources: Self::canonical_resources(),
        }
    }

    /// Recursos canônicos do contrato (`color_main`, `depth_main`,
    /// `face_shadow_mask`, `post_a`, `post_b`).
    fn canonical_resources() -> Vec<GraphResource> {
        [
            ("color_main", ResourceKind::TransientColor),
            ("depth_main", ResourceKind::Depth),
            ("face_shadow_mask", ResourceKind::TransientColor),
            ("post_a", ResourceKind::PingPong),
            ("post_b", ResourceKind::PingPong),
        ]
        .into_iter()
        .map(|(name, kind)| GraphResource {
            name: name.to_string(),
            kind,
        })
        .collect()
    }

    /// Nomes dos passes na ordem de declaração.
    pub fn pass_names(&self) -> Vec<String> {
        self.passes.iter().map(|pass| pass.name.clone()).collect()
    }

    /// Planeja a execução: valida, aplica os overrides do snapshot, ordena
    /// topologicamente e atribui slots físicos com aliasing.
    pub fn build(&self, overrides: &GraphOverrides) -> Result<ExecutionPlan, GraphError> {
        // Nomes únicos + índice de declaração (desempate determinístico).
        let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, pass) in self.passes.iter().enumerate() {
            if index_of.insert(pass.name.as_str(), index).is_some() {
                return Err(GraphError::DuplicatePass(pass.name.clone()));
            }
        }
        let resources: BTreeSet<&str> = self
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        for pass in &self.passes {
            for name in pass.reads.iter().chain(pass.writes.iter()) {
                if !resources.contains(name.as_str()) {
                    return Err(GraphError::UnknownResource {
                        pass: pass.name.clone(),
                        resource: name.clone(),
                    });
                }
            }
            for dep in &pass.after {
                if !index_of.contains_key(dep.as_str()) {
                    return Err(GraphError::UnknownPass(dep.clone()));
                }
            }
        }

        // Overrides referenciam passes do grafo — nome estranho é erro de
        // dados (o chamador cai no plano do contrato com diagnóstico).
        for name in overrides.order.iter().chain(overrides.disabled.iter()) {
            if !index_of.contains_key(name.as_str()) {
                return Err(GraphError::UnknownPass(name.clone()));
            }
        }
        let disabled: BTreeSet<&str> = overrides.disabled.iter().map(String::as_str).collect();

        // Conjunto habilitado: flag do nó, menos os desligados, mais o
        // pré-passe quando o snapshot pede (salvo desligado explícito).
        let mut enabled: Vec<bool> = self.passes.iter().map(|pass| pass.enabled).collect();
        for (index, pass) in self.passes.iter().enumerate() {
            if disabled.contains(pass.name.as_str()) {
                enabled[index] = false;
            }
        }
        if overrides.depth_prepass {
            if let Some(index) = index_of.get(GraphOverrides::DEPTH_PREPASS) {
                if !disabled.contains(GraphOverrides::DEPTH_PREPASS) {
                    enabled[*index] = true;
                }
            }
        }
        if !enabled.iter().any(|on| *on) {
            return Err(GraphError::EmptyPlan);
        }

        // Kahn determinístico: entre os prontos, vence a dica de ordem do
        // snapshot; o resto mantém a ordem de declaração do contrato.
        let hint_pos: BTreeMap<&str, usize> = overrides
            .order
            .iter()
            .enumerate()
            .map(|(position, name)| (name.as_str(), position))
            .collect();
        let mut pending_deps: Vec<usize> = vec![0; self.passes.len()];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); self.passes.len()];
        for (index, pass) in self.passes.iter().enumerate() {
            if !enabled[index] {
                continue;
            }
            for dep in &pass.after {
                let dep_index = index_of[dep.as_str()];
                if !enabled[dep_index] {
                    continue;
                }
                pending_deps[index] += 1;
                dependents[dep_index].push(index);
            }
        }
        // Fila de prontos ordenada pela chave (dica, declaração).
        let mut ready: Vec<usize> = (0..self.passes.len())
            .filter(|index| enabled[*index] && pending_deps[*index] == 0)
            .collect();
        ready.sort_by_key(|index| kahn_key(&hint_pos, &self.passes, *index));
        let mut order: Vec<usize> = Vec::new();
        while let Some(index) = ready.first().copied() {
            ready.remove(0);
            order.push(index);
            let mut newly_ready: Vec<usize> = Vec::new();
            for dependent in &dependents[index] {
                pending_deps[*dependent] -= 1;
                if pending_deps[*dependent] == 0 {
                    newly_ready.push(*dependent);
                }
            }
            newly_ready.sort_by_key(|index| kahn_key(&hint_pos, &self.passes, *index));
            for candidate in newly_ready {
                let position = ready
                    .iter()
                    .position(|ready_index| {
                        kahn_key(&hint_pos, &self.passes, candidate)
                            < kahn_key(&hint_pos, &self.passes, *ready_index)
                    })
                    .unwrap_or(ready.len());
                ready.insert(position, candidate);
            }
        }

        let enabled_count = enabled.iter().filter(|on| **on).count();
        if order.len() != enabled_count {
            return Err(GraphError::Cycle {
                path: self.cycle_path(&enabled, &index_of),
            });
        }

        // Aliasing de texturas transitórias: cada recurso vive do primeiro ao
        // último passo que o toca; vidas disjuntas dividem o mesmo slot físico
        // (first-fit pela ordem de primeiro uso — determinístico).
        let mut live: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (step, index) in order.iter().enumerate() {
            let pass = &self.passes[*index];
            for name in pass.reads.iter().chain(pass.writes.iter()) {
                live.entry(name.as_str())
                    .and_modify(|range| {
                        range.0 = range.0.min(step);
                        range.1 = range.1.max(step);
                    })
                    .or_insert((step, step));
            }
        }
        let mut by_first_use: Vec<(&str, (usize, usize))> = live.into_iter().collect();
        by_first_use.sort_by_key(|(_, range)| (range.0, range.1));
        let mut slots: BTreeMap<String, usize> = BTreeMap::new();
        let mut slot_lives: Vec<(usize, usize)> = Vec::new();
        for (name, range) in by_first_use {
            let slot = slot_lives
                .iter()
                .position(|held| held.1 < range.0 || range.1 < held.0)
                .unwrap_or_else(|| {
                    slot_lives.push(range);
                    slot_lives.len() - 1
                });
            // O slot passa a cobrir a união das vidas que o dividem.
            slot_lives[slot] = (
                slot_lives[slot].0.min(range.0),
                slot_lives[slot].1.max(range.1),
            );
            slots.insert(name.to_string(), slot);
        }

        Ok(ExecutionPlan {
            order: order
                .iter()
                .map(|index| self.passes[*index].name.clone())
                .collect(),
            slots,
            slot_count: slot_lives.len(),
        })
    }

    /// Caminho do ciclo para o diagnóstico (nunca um "erro desconhecido").
    fn cycle_path(&self, enabled: &[bool], index_of: &BTreeMap<&str, usize>) -> String {
        // Reexecuta Kahn sem consumir: quem sobra com dependência pendente
        // está (ou depende de quem está) no ciclo; caminha pelas arestas.
        let mut pending: Vec<usize> = vec![0; self.passes.len()];
        for (index, pass) in self.passes.iter().enumerate() {
            if !enabled[index] {
                continue;
            }
            for dep in &pass.after {
                let dep_index = index_of[dep.as_str()];
                if enabled[dep_index] {
                    pending[index] += 1;
                }
            }
        }
        let mut queue: VecDeque<usize> = (0..self.passes.len())
            .filter(|index| enabled[*index] && pending[index] == 0)
            .collect();
        let mut removed = vec![false; self.passes.len()];
        while let Some(index) = queue.pop_front() {
            removed[index] = true;
            for (dependent, pass) in self.passes.iter().enumerate() {
                if !enabled[dependent] || removed[dependent] {
                    continue;
                }
                if pass.after.iter().any(|dep| dep == &self.passes[index].name) {
                    pending[dependent] -= 1;
                    if pending[dependent] == 0 {
                        queue.push_back(dependent);
                    }
                }
            }
        }
        let start = (0..self.passes.len())
            .find(|index| enabled[*index] && !removed[*index])
            .unwrap_or_default();
        let mut path: Vec<&str> = Vec::new();
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            if !seen.insert(index) {
                path.push(self.passes[index].name.as_str());
                break;
            }
            path.push(self.passes[index].name.as_str());
            cursor = self.passes[index]
                .after
                .iter()
                .filter_map(|dep| index_of.get(dep.as_str()).copied())
                .find(|dep| enabled[*dep] && !removed[*dep]);
            if path.len() > self.passes.len() {
                break;
            }
        }
        path.reverse();
        path.join(" -> ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, after: &[&str]) -> GraphPass {
        GraphPass {
            name: name.to_string(),
            kind: PassKind::Custom,
            after: after.iter().map(|dep| dep.to_string()).collect(),
            reads: Vec::new(),
            writes: Vec::new(),
            enabled: true,
        }
    }

    fn resource(name: &str) -> GraphResource {
        GraphResource {
            name: name.to_string(),
            kind: ResourceKind::TransientColor,
        }
    }

    #[test]
    fn contract_plan_matches_today_execution_order() {
        // Sem overrides, o plano é a ordem do contrato (outline → cel): o
        // grafo não muda um pixel do comportamento atual.
        let plan = RenderGraph::from_contract()
            .build(&GraphOverrides::default())
            .expect("plano do contrato");
        assert_eq!(plan.order, vec!["outline".to_string(), "cel".to_string()]);
        assert!(!plan.contains(GraphOverrides::DEPTH_PREPASS));
        // Cor e profundidade convivem no mesmo passo: slots distintos.
        assert_ne!(
            plan.slots.get("color_main"),
            plan.slots.get("depth_main")
        );
        assert!(plan.slot_count >= 2);
    }

    #[test]
    fn depth_prepass_is_anchored_first_when_enabled() {
        // Aceitação #2 (estrutura): com o pré-passe ligado, ele abre o plano
        // — todo fragmento do cel testa contra o z-buffer já resolvido.
        let plan = RenderGraph::from_contract()
            .build(&GraphOverrides {
                depth_prepass: true,
                ..GraphOverrides::default()
            })
            .expect("plano com pré-passe");
        assert_eq!(
            plan.order,
            vec![
                "depth_prepass".to_string(),
                "outline".to_string(),
                "cel".to_string()
            ]
        );
        assert!(plan.position("depth_prepass") < plan.position("cel"));
        assert!(plan.position("depth_prepass") < plan.position("outline"));

        // Desligado explícito vence o interruptor geral.
        let plan = RenderGraph::from_contract()
            .build(&GraphOverrides {
                disabled: vec!["depth_prepass".to_string()],
                depth_prepass: true,
                ..GraphOverrides::default()
            })
            .expect("plano sem pré-passe");
        assert!(!plan.contains("depth_prepass"));
    }

    #[test]
    fn snapshot_reorders_and_disables_passes() {
        // Aceitação #1: ordem e ativação vêm do snapshot do núcleo.
        let plan = RenderGraph::from_contract()
            .build(&GraphOverrides {
                order: vec!["cel".to_string(), "outline".to_string()],
                ..GraphOverrides::default()
            })
            .expect("ordem do snapshot");
        assert_eq!(plan.order, vec!["cel".to_string(), "outline".to_string()]);

        let plan = RenderGraph::from_contract()
            .build(&GraphOverrides {
                disabled: vec!["outline".to_string()],
                ..GraphOverrides::default()
            })
            .expect("só o cel");
        assert_eq!(plan.order, vec!["cel".to_string()]);

        // Tudo desligado não é plano — é erro (o chamador cai no contrato).
        let error = RenderGraph::from_contract()
            .build(&GraphOverrides {
                disabled: vec!["outline".to_string(), "cel".to_string()],
                ..GraphOverrides::default()
            })
            .expect_err("plano vazio");
        assert_eq!(error, GraphError::EmptyPlan);
    }

    #[test]
    fn unknown_names_and_resources_are_data_errors() {
        let graph = RenderGraph::from_contract();
        assert_eq!(
            graph
                .build(&GraphOverrides {
                    order: vec!["bloom".to_string()],
                    ..GraphOverrides::default()
                })
                .expect_err("passe desconhecido"),
            GraphError::UnknownPass("bloom".to_string())
        );
        assert_eq!(
            graph
                .build(&GraphOverrides {
                    disabled: vec!["bloom".to_string()],
                    ..GraphOverrides::default()
                })
                .expect_err("passe desconhecido"),
            GraphError::UnknownPass("bloom".to_string())
        );

        let broken = RenderGraph {
            passes: vec![GraphPass {
                name: "cel".to_string(),
                kind: PassKind::Opaque,
                after: Vec::new(),
                reads: vec!["color_main".to_string()],
                writes: vec!["typo_target".to_string()],
                enabled: true,
            }],
            resources: vec![resource("color_main")],
        };
        assert_eq!(
            broken
                .build(&GraphOverrides::default())
                .expect_err("recurso desconhecido"),
            GraphError::UnknownResource {
                pass: "cel".to_string(),
                resource: "typo_target".to_string(),
            }
        );

        let duplicated = RenderGraph {
            passes: vec![decl("cel", &[]), decl("cel", &[])],
            resources: Vec::new(),
        };
        assert_eq!(
            duplicated
                .build(&GraphOverrides::default())
                .expect_err("passe duplicado"),
            GraphError::DuplicatePass("cel".to_string())
        );
    }

    #[test]
    fn cycles_are_rejected_with_a_named_path() {
        // Aceitação #4: o DAG é validado — ciclo vira erro nomeado.
        let cyclic = RenderGraph {
            passes: vec![
                decl("depth_prepass", &["outline"]),
                decl("outline", &["cel"]),
                decl("cel", &["depth_prepass"]),
            ],
            resources: Vec::new(),
        };
        let error = cyclic
            .build(&GraphOverrides::default())
            .expect_err("ciclo precisa falhar");
        match error {
            GraphError::Cycle { path } => {
                assert!(path.contains("depth_prepass"));
                assert!(path.contains("outline"));
                assert!(path.contains("cel"));
            }
            other => panic!("esperava Cycle, veio {other:?}"),
        }
    }

    #[test]
    fn disjoint_lifetimes_share_a_texture_slot() {
        // Aliasing: a máscara de sombra morre antes do pós-processar usar o
        // rascunho — os dois dividem um slot; cor e profundidade convivem e
        // ficam em slots distintos.
        let pass = |name: &str, after: &[&str], reads: &[&str], writes: &[&str]| GraphPass {
            name: name.to_string(),
            kind: PassKind::Custom,
            after: after.iter().map(|dep| dep.to_string()).collect(),
            reads: reads.iter().map(|res| res.to_string()).collect(),
            writes: writes.iter().map(|res| res.to_string()).collect(),
            enabled: true,
        };
        let graph = RenderGraph {
            passes: vec![
                pass("depth_prepass", &[], &[], &["depth_main"]),
                pass(
                    "face_shadow_sdf",
                    &["depth_prepass"],
                    &["depth_main"],
                    &["face_shadow_mask"],
                ),
                pass(
                    "opaque_cel",
                    &["face_shadow_sdf"],
                    &["depth_main", "face_shadow_mask"],
                    &["color_main", "depth_main"],
                ),
                pass("outline", &["opaque_cel"], &["color_main", "depth_main"], &["color_main"]),
                // O pós lê a profundidade (DOF/neblina usam o z-buffer): com
                // `depth_main` vivo até o fim, o first-fit devolve o
                // `post_a` ao slot que a máscara de sombra liberou.
                pass(
                    "postprocess",
                    &["outline"],
                    &["color_main", "post_a", "depth_main"],
                    &["color_main"],
                ),
            ],
            resources: vec![
                resource("color_main"),
                resource("depth_main"),
                resource("face_shadow_mask"),
                resource("post_a"),
            ],
        };
        let plan = graph.build(&GraphOverrides::default()).expect("plano");
        assert_eq!(
            plan.order,
            vec![
                "depth_prepass",
                "face_shadow_sdf",
                "opaque_cel",
                "outline",
                "postprocess"
            ]
        );
        // `face_shadow_mask` (passos 1–2) e `post_a` (passo 4): vidas
        // disjuntas, mesmo slot físico.
        assert_eq!(plan.slots.get("face_shadow_mask"), plan.slots.get("post_a"));
        // `color_main` e `depth_main` convivem: slots distintos.
        assert_ne!(plan.slots.get("color_main"), plan.slots.get("depth_main"));
        assert!(plan.slot_count < 4, "aliasing economiza um slot");
    }

    #[test]
    fn canonical_anime_graph_matches_the_frozen_contract() {
        // O grafo em código espelha `render_graph` do fixture: deriva contra
        // deriva, não contra a memória de quem escreveu.
        let contract: serde_json::Value =
            serde_json::from_str(crate::render_contract::CONTRACT_JSON).expect("contrato JSON");
        let frozen = &contract["render_graph"];
        assert!(frozen.is_object(), "contrato declara `render_graph`");

        let canonical: Vec<String> = frozen["canonical_order"]
            .as_array()
            .expect("canonical_order")
            .iter()
            .map(|name| name.as_str().expect("nome").to_string())
            .collect();
        let graph = RenderGraph::anime_default();
        assert_eq!(graph.pass_names(), canonical);

        let frozen_passes = frozen["passes"].as_object().expect("passes");
        for pass in &graph.passes {
            let frozen_pass = frozen_passes
                .get(pass.name.as_str())
                .unwrap_or_else(|| panic!("contrato declara o passe '{}'", pass.name));
            let kind = frozen_pass["kind"].as_str().expect("kind");
            let expected = match pass.kind {
                PassKind::DepthPrepass => "depth_prepass",
                PassKind::Opaque => "opaque",
                PassKind::Outline => "outline",
                PassKind::PostProcess => "postprocess",
                PassKind::Compute => "compute",
                PassKind::Custom => "custom",
            };
            assert_eq!(kind, expected, "kind de '{}'", pass.name);
            let after: Vec<String> = frozen_pass["after"]
                .as_array()
                .expect("after")
                .iter()
                .map(|dep| dep.as_str().expect("dep").to_string())
                .collect();
            assert_eq!(after, pass.after, "deps de '{}'", pass.name);
            for list in ["reads", "writes"] {
                let frozen_list: Vec<String> = frozen_pass[list]
                    .as_array()
                    .unwrap_or_else(|| panic!("'{list}' de '{}'", pass.name))
                    .iter()
                    .map(|name| name.as_str().expect("recurso").to_string())
                    .collect();
                let coded = if list == "reads" { &pass.reads } else { &pass.writes };
                assert_eq!(frozen_list, *coded, "'{list}' de '{}'", pass.name);
            }
        }

        // E o plano canônico executa na ordem da especificação.
        let plan = graph
            .build(&GraphOverrides {
                depth_prepass: true,
                ..GraphOverrides::default()
            })
            .expect("plano canônico");
        assert_eq!(plan.order, canonical);
    }
}
