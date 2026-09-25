/**
 * ANIGO contract test — render graph dinâmico (Fase 1, issue #14).
 *
 * O quadro é um DAG de passes: a topologia canônica (seis passes de anime) é
 * congelada no contrato, o snapshot do núcleo só ajusta ordem/ativação e o
 * pré-passe de profundidade, e o planejador traduz os overrides no plano que
 * headless e viewport executam igual.
 *
 * Sem GPU no ambiente de teste, a paridade é verificada nos pontos testáveis
 * fora dela (o mesmo padrão do `render_contract.test.ts`):
 *   - a seção `render_graph` do fixture (ordem canônica, nós, recursos);
 *   - a validação da seção (`missing_render_graph`/`bad_render_graph`);
 *   - o planejador puro (`planRenderGraphPasses` — mesmas regras do Rust);
 *   - os fios do comando (`set_render_settings` com `graph_*`/`depth_prepass`);
 *   - o bloco `render` do snapshot (ordem/ativação/pré-passe viajam nele);
 *   - o viewport consome o plano (referência no fonte, como os demais testes
 *     de autoridade fazem).
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";

import {
  RENDER_CONTRACT,
  RenderContractError,
  readRenderContract,
  renderGraph,
  renderGraphCanonicalOrder,
  renderPassOrder,
} from "../../src/contracts/render_contract.v1.ts";
import { decodeCoreSnapshot, type CoreSnapshotWire } from "../../src/contracts/core_snapshot.v1.ts";
import { buildCommand, type CommandIntent } from "../../src/services/command_builder.ts";
import {
  DEPTH_PREPASS_PASS,
  planRenderGraphPasses,
  type GraphOverridesInput,
} from "../../src/services/render_graph_plan.ts";
import { readRepoFile } from "./rust_contract_source.ts";

const CONTRACT_FIXTURE_PATH = "contracts/fixtures/render_contract_v1.json";
const SNAPSHOT_FIXTURE_PATH = "contracts/fixtures/core_snapshot_v1.json";
const RENDERER_SOURCE = readRepoFile("src/components/viewport/webgpu_renderer.ts");
const RUST_GRAPH = readRepoFile("crates/anigo-renderer/src/render_graph.rs");

const CANONICAL_ORDER = [
  "depth_prepass",
  "face_shadow_sdf",
  "opaque_cel",
  "hair_cloth",
  "outline",
  "postprocess",
];

function contractFixture(): any {
  return JSON.parse(readRepoFile(CONTRACT_FIXTURE_PATH));
}

function snapshotFixture(): CoreSnapshotWire {
  return JSON.parse(readRepoFile(SNAPSHOT_FIXTURE_PATH)) as CoreSnapshotWire;
}

function built(intent: CommandIntent): Record<string, unknown> {
  const result = buildCommand(intent);
  assert.equal(result.ok, true, `intent deveria virar comando: ${JSON.stringify(intent)}`);
  if (result.ok !== true) throw new Error("unreachable");
  return result.command as unknown as Record<string, unknown>;
}

function rejected(intent: CommandIntent, code: string): void {
  const result = buildCommand(intent);
  assert.equal(result.ok, false, `intent deveria ser recusada: ${JSON.stringify(intent)}`);
  if (result.ok === false) assert.equal(result.code, code, result.reason);
}

const NO_OVERRIDES: GraphOverridesInput = { order: [], disabled: [], depthPrepass: false };

describe("Render graph — a topologia canônica é congelada no contrato", () => {
  it("o fixture declara os seis passes de anime com deps e recursos", () => {
    const graph = contractFixture().render_graph;
    assert.deepEqual(graph.canonical_order, CANONICAL_ORDER);
    assert.deepEqual(Object.keys(graph.passes).sort(), [...CANONICAL_ORDER].sort());
    // Cadeia linear da especificação: cada passe depois do anterior.
    const after: Record<string, string[]> = {
      depth_prepass: [],
      face_shadow_sdf: ["depth_prepass"],
      opaque_cel: ["face_shadow_sdf"],
      hair_cloth: ["opaque_cel"],
      outline: ["hair_cloth"],
      postprocess: ["outline"],
    };
    for (const [name, deps] of Object.entries(after)) {
      assert.deepEqual(graph.passes[name].after, deps, `deps de '${name}'`);
      assert.ok(Array.isArray(graph.passes[name].reads), `reads de '${name}'`);
      assert.ok(Array.isArray(graph.passes[name].writes), `writes de '${name}'`);
    }
    // Os passes executáveis de hoje mapeiam para os pipelines reais.
    assert.equal(graph.passes.opaque_cel.executes_as, "cel");
    assert.equal(graph.passes.outline.executes_as, "outline");
    // O pré-passe nasce desligado e reusa o vértice do cel.
    assert.equal(graph.passes.depth_prepass.enabled_by_default, false);
    assert.equal(graph.passes.depth_prepass.shader, "cel_shading");
    assert.equal(graph.passes.depth_prepass.vertex_entry, "vs_main");
    assert.deepEqual(
      Object.keys(graph.resources).sort(),
      ["color_main", "depth_main", "face_shadow_mask", "post_a", "post_b"].sort()
    );
  });

  it("o TS expõe a mesma seção que o Rust congela", () => {
    assert.deepEqual(renderGraphCanonicalOrder(), CANONICAL_ORDER);
    assert.deepEqual(renderGraph().canonical_order, CANONICAL_ORDER);
    assert.equal(renderGraph(), RENDER_CONTRACT.render_graph);
  });

  it("o grafo em código (Rust) espelha os mesmos nomes do fixture", () => {
    for (const name of CANONICAL_ORDER) {
      assert.ok(
        RUST_GRAPH.includes(`"${name}"`),
        `render_graph.rs precisa declarar o passe '${name}'`
      );
    }
  });

  it("seção ausente ou inválida falha com erro explícito e recuperável", () => {
    const base = contractFixture();
    const mutate = (change: (copy: any) => void) => {
      const copy = JSON.parse(JSON.stringify(base));
      change(copy);
      return copy;
    };
    const rejects = (copy: any, code: string) => {
      assert.throws(
        () => readRenderContract(copy),
        (error: unknown) => {
          assert.ok(error instanceof RenderContractError, `esperado RenderContractError, veio ${error}`);
          const contractError = error as InstanceType<typeof RenderContractError>;
          assert.equal(contractError.code, code);
          assert.equal(contractError.recoverable, true);
          return true;
        }
      );
    };
    rejects(mutate((copy) => delete copy.render_graph), "missing_render_graph");
    rejects(mutate((copy) => (copy.render_graph.canonical_order = [])), "bad_render_graph");
    rejects(
      mutate((copy) => (copy.render_graph.canonical_order = ["outline", "outline"])),
      "bad_render_graph"
    );
    rejects(
      mutate((copy) => (copy.render_graph.canonical_order = ["bloom"])),
      "bad_render_graph"
    );
    rejects(
      mutate((copy) => (copy.render_graph.passes.outline.after = ["bloom"])),
      "bad_render_graph"
    );
    rejects(
      mutate((copy) => (copy.render_graph.passes.outline.reads = "color_main")),
      "bad_render_graph"
    );
    rejects(
      mutate((copy) => (copy.render_graph.passes.outline.writes = ["typo_target"])),
      "bad_render_graph"
    );
    // O contrato válido continua passando.
    assert.equal(readRenderContract(base).render_graph.canonical_order.length, 6);
  });
});

describe("Render graph — o planejador aplica os overrides do snapshot", () => {
  const contractOrder = renderPassOrder();

  it("sem overrides, o plano é a ordem do contrato", () => {
    assert.deepEqual(contractOrder, ["outline", "cel"]);
    const plan = planRenderGraphPasses(contractOrder, NO_OVERRIDES);
    assert.deepEqual(plan.passes, ["outline", "cel"]);
    assert.equal(plan.fellBack, false);
  });

  it("o pré-passe abre o plano quando ligado (Early-Z antes do cel)", () => {
    const plan = planRenderGraphPasses(contractOrder, {
      ...NO_OVERRIDES,
      depthPrepass: true,
    });
    assert.deepEqual(plan.passes, [DEPTH_PREPASS_PASS, "outline", "cel"]);
    assert.equal(plan.fellBack, false);
  });

  it("`disabled` explícito vence o interruptor do pré-passe", () => {
    const plan = planRenderGraphPasses(contractOrder, {
      order: [],
      disabled: [DEPTH_PREPASS_PASS],
      depthPrepass: true,
    });
    assert.deepEqual(plan.passes, ["outline", "cel"]);
  });

  it("ordem e ativação vêm do snapshot", () => {
    assert.deepEqual(
      planRenderGraphPasses(contractOrder, {
        ...NO_OVERRIDES,
        order: ["cel", "outline"],
      }).passes,
      ["cel", "outline"]
    );
    assert.deepEqual(
      planRenderGraphPasses(contractOrder, { ...NO_OVERRIDES, disabled: ["outline"] }).passes,
      ["cel"]
    );
    // Dica parcial: listados primeiro, o resto mantém o contrato.
    assert.deepEqual(
      planRenderGraphPasses(contractOrder, { ...NO_OVERRIDES, order: ["cel"] }).passes,
      ["cel", "outline"]
    );
  });

  it("nome estranho cai no plano do contrato com diagnóstico", () => {
    for (const overrides of [
      { ...NO_OVERRIDES, order: ["bloom"] },
      { ...NO_OVERRIDES, disabled: ["bloom"] },
    ]) {
      const plan = planRenderGraphPasses(contractOrder, overrides);
      assert.deepEqual(plan.passes, ["outline", "cel"]);
      assert.equal(plan.fellBack, true);
      assert.equal(plan.unknownPass, "bloom");
    }
    // Tudo desligado também cai no contrato (plano vazio não desenha).
    const empty = planRenderGraphPasses(contractOrder, {
      ...NO_OVERRIDES,
      disabled: ["outline", "cel"],
    });
    assert.deepEqual(empty.passes, ["outline", "cel"]);
    assert.equal(empty.fellBack, true);
  });
});

describe("Render graph — o comando carrega os overrides até o núcleo", () => {
  it("`render_settings` com `graph_*`/`depth_prepass` vira `set_render_settings`", () => {
    assert.deepEqual(
      built({
        kind: "render_settings",
        graph_order: ["cel", "outline"],
        graph_disabled: ["postprocess"],
        depth_prepass: true,
      }),
      {
        kind: "set_render_settings",
        graph_order: ["cel", "outline"],
        graph_disabled: ["postprocess"],
        depth_prepass: true,
      }
    );
    // Listas vazias resetam para o contrato (Some([]) no núcleo, não NoOp).
    assert.deepEqual(built({ kind: "render_settings", graph_order: [] }), {
      kind: "set_render_settings",
      graph_order: [],
    });
  });

  it("listas com nome vazio ou repetido são recusadas (mesma regra do núcleo)", () => {
    rejected({ kind: "render_settings", graph_order: ["cel", "cel"] }, "invalid_value");
    rejected({ kind: "render_settings", graph_order: ["  "] }, "invalid_value");
    rejected({ kind: "render_settings", graph_disabled: ["outline", "outline"] }, "invalid_value");
    rejected({ kind: "render_settings", depth_prepass: "sim" as unknown as boolean }, "invalid_value");
    rejected({ kind: "render_settings" }, "no_op");
  });

  it("os overrides viajam no bloco `render` do snapshot", () => {
    const render = snapshotFixture().dynamic.render;
    assert.ok(Array.isArray(render.graph_order), "graph_order viaja no snapshot");
    assert.ok(Array.isArray(render.graph_disabled), "graph_disabled viaja no snapshot");
    assert.equal(typeof render.depth_prepass, "boolean", "depth_prepass viaja no snapshot");
    const decoded = decodeCoreSnapshot(snapshotFixture());
    assert.deepEqual(decoded.state.render.graph_order, render.graph_order);
    assert.deepEqual(decoded.state.render.graph_disabled, render.graph_disabled);
    assert.equal(decoded.state.render.depth_prepass, render.depth_prepass);
  });
});

describe("Render graph — o viewport executa o plano do snapshot", () => {
  it("o viewport planeja os passes a partir do `render` do snapshot", () => {
    assert.match(RENDERER_SOURCE, /planRenderGraphPasses\(/);
    assert.match(RENDERER_SOURCE, /delivery\.state\?\.render\?\.graph_order/);
    assert.match(RENDERER_SOURCE, /delivery\.state\?\.render\?\.graph_disabled/);
    assert.match(RENDERER_SOURCE, /delivery\.state\?\.render\?\.depth_prepass/);
  });

  it("o viewport tem o pipeline só-profundidade e o passe dedicado", () => {
    assert.match(RENDERER_SOURCE, /depthPrepassPipeline/);
    assert.match(RENDERER_SOURCE, /depth_prepass/);
    // Com o pré-passe, o passe principal carrega o z-buffer em vez de limpar.
    assert.match(RENDERER_SOURCE, /depthLoadOp/);
  });
});
