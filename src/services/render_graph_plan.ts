/**
 * ANIGO — planejador do render graph (issue #14), espelho de
 * `crates/anigo-renderer/src/render_graph.rs`.
 *
 * O quadro é um DAG de passes: o snapshot do núcleo só ajusta ordem
 * (`graph_order`), ativação (`graph_disabled`) e o pré-passe de profundidade
 * (`depth_prepass`) — nunca a topologia, que é congelada no contrato. Este
 * planejador puro traduz os overrides no plano que o viewport executa, com as
 * MESMAS regras do Rust:
 *
 * - nomes estranhos invalidam o plano inteiro (`GraphError::UnknownPass`) e o
 *   viewport cai na ordem do contrato com diagnóstico (nunca um quadro vazio);
 * - `disabled` explícito vence o `depth_prepass`;
 * - tudo desligado também cai no contrato (`GraphError::EmptyPlan`);
 * - com o pré-passe ligado, ele abre o plano (todo fragmento do cel testa
 *   contra o z-buffer já resolvido — Early-Z).
 *
 * O subgrafo executável de hoje (`outline`, `cel`) é plano: a única
 * dependência ancora o pré-passe no início, então a dica de ordem decide o
 * resto — o equivalente exato do Kahn determinístico do `from_contract`.
 */

/** O nó de pré-passe de profundidade (`GraphOverrides::DEPTH_PREPASS`). */
export const DEPTH_PREPASS_PASS = "depth_prepass";

/** Overrides do snapshot — espelho de `GraphOverrides` (Rust). */
export interface GraphOverridesInput {
  /** Ordem de execução (nomes do contrato). */
  order: string[];
  /** Passes desligados (nomes do contrato). */
  disabled: string[];
  /** Liga o pré-passe de profundidade. */
  depthPrepass: boolean;
}

/** Plano de execução do viewport. */
export interface RenderGraphPlan {
  /** Passes na ordem de execução (`depth_prepass` abre quando ligado). */
  passes: string[];
  /** `true` quando o plano caiu na ordem do contrato. */
  fellBack: boolean;
  /** Nome estranho que invalidou o plano (para o diagnóstico). */
  unknownPass?: string;
}

/**
 * Planeja a execução: valida os overrides contra os passes do contrato,
 * aplica ordem/ativação e ancora o pré-passe no início quando ligado.
 */
export function planRenderGraphPasses(
  contractOrder: readonly string[],
  overrides: GraphOverridesInput
): RenderGraphPlan {
  const known = new Set<string>([...contractOrder, DEPTH_PREPASS_PASS]);
  const unknownPass = [...overrides.order, ...overrides.disabled].find(
    (name) => !known.has(name)
  );
  if (unknownPass !== undefined) {
    return { passes: [...contractOrder], fellBack: true, unknownPass };
  }
  const disabled = new Set(overrides.disabled);
  const enabled = contractOrder.filter((name) => !disabled.has(name));
  const prepass = overrides.depthPrepass && !disabled.has(DEPTH_PREPASS_PASS);
  // Dica de ordem: listados primeiro (só os habilitados), o resto mantém a
  // ordem relativa do contrato.
  const hinted = overrides.order.filter((name) => enabled.includes(name));
  const rest = enabled.filter((name) => !hinted.includes(name));
  const passes = [...hinted, ...rest];
  if (passes.length === 0 && !prepass) {
    return { passes: [...contractOrder], fellBack: true };
  }
  return {
    passes: prepass ? [DEPTH_PREPASS_PASS, ...passes] : passes,
    fellBack: false,
  };
}
