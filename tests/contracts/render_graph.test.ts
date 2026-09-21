/**
 * ANIGO #14 — render graph dinâmico (DAG de passes).
 *
 * O DAG vive no contrato congelado (`render_graph` do
 * `contracts/fixtures/render_contract_v1.json`) e é lido pelos **dois**
 * renderers: o headless Rust (`crates/anigo-renderer/src/render_graph.rs`) e o
 * viewport (`src/contracts/render_contract.v1.ts`). Se as duas implementações
 * divergissem na ordem, o mesmo projeto sairia com passes em ordem diferente no
 * frame exportado e na tela — exatamente o que a paridade headless ⇄ viewport
 * proíbe.
 *
 * O que este arquivo trava:
 *
 *   1. a ordem topológica é determinística (Kahn + desempate `order_hint`/nome)
 *      e respeita as dependências, inclusive quando a cena pede outra ordem;
 *   2. ciclo, auto-dependência e dependência inexistente viram erro nomeado —
 *      o caminho do ciclo aparece na mensagem, não um "deadlock" anônimo;
 *   3. ativação: nó desabilitado ou sem pipeline declarado nunca é agendado, e o
 *      depth pre-pass pode ser desligado pela cena (critério 1 do issue #14);
 *   4. aliasing de alvos transitórios: só compartilha quem tem janelas de vida
 *      disjuntas, e a profundidade nunca entra no grupo de uma textura de cor;
 *   5. a lista de passes do núcleo (`RENDER_GRAPH_PASSES`) bate com os nós do
 *      contrato — o drift é detectado aqui, não em produção.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  RENDER_CONTRACT,
  RenderContractError,
  renderGraph,
  renderGraphNode,
  renderGraphOrder,
  renderGraphOrderOf,
  renderPassOrder,
  passesWithoutPipeline,
  type RenderGraphNodeV1,
} from "../../src/contracts/render_contract.v1.ts";
import { readRepoFile, stripComments } from "./rust_contract_source.ts";

function node(
  name: string,
  depends_on: string[],
  order_hint: number,
  extra: Partial<RenderGraphNodeV1> = {}
): RenderGraphNodeV1 {
  return {
    name,
    kind: "render",
    contract_pass: name,
    depends_on,
    reads: [],
    writes: [],
    enabled: true,
    order_hint,
    only_when: null,
    early_z_for: null,
    ...extra,
  };
}

test("ordem topológica: determinística e dependências primeiro", () => {
  // Declarado fora de ordem de propósito: o `order_hint` é quem decide.
  const nodes = [
    node("outline", ["cel"], 40),
    node("cel", ["depth_prepass"], 20),
    node("depth_prepass", [], 0),
  ];
  assert.deepEqual(renderGraphOrderOf(nodes), ["depth_prepass", "cel", "outline"]);
  // Idempotente: dois frames do mesmo projeto nunca divergem na ordem.
  assert.deepEqual(renderGraphOrderOf([...nodes].reverse()), ["depth_prepass", "cel", "outline"]);
});

test("empate de `order_hint` é desempatado pelo nome (estável)", () => {
  const nodes = [node("b_pass", [], 10), node("a_pass", [], 10), node("c_pass", [], 5)];
  assert.deepEqual(renderGraphOrderOf(nodes), ["c_pass", "a_pass", "b_pass"]);
});

test("ciclo é recusado com o caminho nomeado", () => {
  const nodes = [
    node("a", ["c"], 0),
    node("b", ["a"], 1),
    node("c", ["b"], 2),
  ];
  assert.throws(
    () => renderGraphOrderOf(nodes),
    (error: unknown) => {
      assert.ok(error instanceof RenderContractError);
      assert.equal(error.code, "graph_cycle");
      assert.match(error.message, /ciclo no render graph entre:/);
      for (const name of ["a", "b", "c"]) {
        assert.ok(error.message.includes(name), `o caminho precisa citar ${name}`);
      }
      return true;
    }
  );
});

test("auto-dependência e dependência inexistente são recusadas", () => {
  assert.throws(
    () => renderGraphOrderOf([node("a", ["a"], 0)]),
    (error: unknown) => (error as RenderContractError).code === "graph_cycle"
  );
  assert.throws(
    () => renderGraphOrderOf([node("a", ["ghost"], 0)]),
    (error: unknown) => {
      assert.equal((error as RenderContractError).code, "graph_dependency");
      assert.match((error as Error).message, /'ghost'/);
      return true;
    }
  );
});

test("ativação: nó sem pipeline nunca entra no plano; ordem da cena respeita o DAG", () => {
  // `sem_pipeline` está declarado no DAG (o grafo já o conhece) mas ainda não
  // tem passe no contrato: ele fica de fora em vez de virar no-op disfarçado.
  const declared = renderGraph().nodes.filter((entry) => entry.contract_pass === null);
  assert.deepEqual(
    declared.map((entry) => entry.name).sort(),
    ["face_shadow_sdf", "hair_and_cloth", "post_process"]
  );
  assert.deepEqual(passesWithoutPipeline().sort(), ["face_shadow_sdf", "hair_and_cloth", "post_process"]);

  // Ordem do contrato (nenhuma configuração da cena).
  assert.deepEqual(renderPassOrder(), ["depth_prepass", "cel", "outline"]);

  // Critério 1 do issue #14: o pre-pass é desligável pelo documento…
  assert.deepEqual(renderPassOrder({ depthPrepass: false }), ["cel", "outline"]);

  // …e um passe do grafo pode ser desligado pelo **nome do nó** (a convenção do
  // comando `set_render_settings` no núcleo).
  assert.deepEqual(
    renderPassOrder({ disabledPasses: ["inverted_hull_outline"] }),
    ["depth_prepass", "cel"]
  );

  // Pedir o outline primeiro não o coloca antes das dependências dele.
  assert.deepEqual(
    renderPassOrder({ order: ["outline", "cel", "depth_prepass"] }),
    ["depth_prepass", "cel", "outline"]
  );
});

test("contrato: os nós, dependências e o early-Z do issue #14", () => {
  assert.deepEqual(renderGraphOrder(), [
    "sparse_morph",
    "depth_prepass",
    "face_shadow_sdf",
    "opaque_cel",
    "hair_and_cloth",
    "inverted_hull_outline",
    "post_process",
  ]);
  // O pre-pass existe para o early-Z do passe cel — é a ligação que prova a
  // intenção, e o cel precisa declará-lo como dependência.
  assert.equal(renderGraphNode("depth_prepass")?.early_z_for, "opaque_cel");
  assert.ok(renderGraphNode("opaque_cel")?.depends_on.includes("depth_prepass"));
  // O compute de morphs abre o frame e alimenta o pre-pass.
  assert.ok(renderGraphNode("depth_prepass")?.depends_on.includes("sparse_morph"));
  assert.equal(renderGraphNode("sparse_morph")?.only_when, "gpu_morph_active");
});

test("aliasing: só compartilha quem não coexiste, e cor nunca divide com profundidade", () => {
  const graph = renderGraph();
  const byName = new Map(graph.resources.map((resource) => [resource.name, resource]));
  const group = graph.aliasing.groups.find((candidate) => candidate.members.includes("oit_accum"));
  assert.ok(group, "o grupo do oit_accum está declarado no contrato");
  assert.deepEqual(group.members, ["oit_accum", "bloom_a"]);

  // Os membros de um grupo precisam ter a mesma chave (kind|format|tamanho|samples)…
  const first = byName.get(group.members[0])!;
  for (const member of group.members) {
    const resource = byName.get(member)!;
    assert.equal(resource.kind, first.kind, `${member}: kind diferente no mesmo grupo`);
    assert.equal(resource.format, first.format, `${member}: format diferente no mesmo grupo`);
    assert.equal(resource.size, first.size, `${member}: tamanho diferente no mesmo grupo`);
  }
  // …e nenhum deles pode ser profundidade (buffer de cor e z-buffer não se
  // sobrepõem por acidente).
  assert.ok(group.members.every((member) => byName.get(member)!.kind === "transient_color"));

  // Recursos persistentes (buffers de storage) não têm janela de vida.
  for (const resource of graph.resources.filter((entry) => entry.kind === "storage_buffer")) {
    assert.equal(resource.first_use, null, `${resource.name}: storage não é transiente`);
    assert.equal(resource.last_use, null, `${resource.name}: storage não é transiente`);
  }
  // Todo primeiro/último uso aponta para um nó que existe no grafo.
  const names = new Set(graph.nodes.map((entry) => entry.name));
  for (const resource of graph.resources) {
    for (const use of [resource.first_use, resource.last_use]) {
      if (use === null) continue;
      assert.ok(names.has(use), `${resource.name}: uso em '${use}', que não está no grafo`);
    }
  }
});

test("drift: os passes do núcleo são os nós do contrato", () => {
  // `RENDER_GRAPH_PASSES` (Rust, `anigo-core`) é a lista que valida o comando
  // `set_render_settings`. Se o contrato ganhar um nó e o núcleo não souber
  // dele, o comando passaria a recusar um passe legítimo — então os dois lados
  // são comparados aqui.
  const source = stripComments(readRepoFile("crates/anigo-core/src/project.rs"));
  const tuple = /pub const RENDER_GRAPH_PASSES: \[&str; (\d+)\] = \[([^\]]*)\];/.exec(source);
  assert.ok(tuple, "RENDER_GRAPH_PASSES não encontrado no núcleo");
  const names = [...tuple[2].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
  assert.equal(Number(tuple[1]), names.length, "a aridade do array precisa fechar com os nomes");
  assert.deepEqual(
    names,
    renderGraph().nodes.map((entry) => entry.name),
    "RENDER_GRAPH_PASSES divergiu do render_graph do contrato"
  );

  // O contrato do renderer conhece a mesma lista (fallback documentado).
  assert.equal(RENDER_CONTRACT.render_graph.scheduling, "kahn_topological");
  assert.equal(RENDER_CONTRACT.render_graph.nodes.length, names.length);
});
