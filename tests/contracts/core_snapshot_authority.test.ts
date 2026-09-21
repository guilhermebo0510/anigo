/**
 * ANIGO contract test — P0 §7 itens 4–5: a deformação saiu do TypeScript e o
 * viewport consome snapshots do núcleo.
 *
 * O que este arquivo prende:
 * 1. a geometria do viewport é *decodificada* do snapshot (nenhum delta/normal
 *    recalculado) e validada antes de ir para a GPU;
 * 2. o cliente da sessão fala com o núcleo pelos comandos Tauri declarados em
 *    `src-tauri/src/main.rs`, com cache key de revisão estática e descarte de
 *    respostas fora de ordem;
 * 3. não existe mais nenhum caminho de deformação em `src/**` (o motor de
 *    referência vive em `tests/reference/`), e o renderer não contém os
 *    símbolos da engine antiga.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  CoreBridgeError,
  CoreSessionClient,
  channelWeightOf,
  coreCommandName,
  isCoreHistoryReport,
  isCoreSnapshot,
  resolveCoreInvoker,
  type CoreInvoker,
} from "../../src/services/core_bridge.ts";
import {
  applyDeltasCpu,
  assertChannelsSorted,
  geometrySignature,
  packChannelRecords,
  packChannelRecordsWithWeights,
  viewportGeometryFromSnapshot,
  ViewportGeometryError,
} from "../../src/services/viewport_mesh.ts";
import { MORPH_CHANNEL_STRIDE_BYTES } from "../../src/contracts/core_snapshot.v1.ts";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import { readRepoFile } from "./rust_contract_source.ts";

const FIXTURE = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../contracts/fixtures/core_snapshot_v1.json", import.meta.url)), "utf8")
);
const RENDERER_SOURCE = readRepoFile("src/components/viewport/webgpu_renderer.ts");
const VIEWPORT_SOURCE = readRepoFile("src/components/viewport/Viewport.svelte");
const APP_SOURCE = readRepoFile("src/App.svelte");
const MAIN_RS = readRepoFile("src-tauri/src/main.rs");
const CORE_SESSION_RS = readRepoFile("src-tauri/src/core_session.rs");

describe("P0 itens 4–5 — geometria do viewport vem do snapshot", () => {
  it("decodifica o snapshot do contrato sem recalcular nada", () => {
    const geometry = viewportGeometryFromSnapshot(FIXTURE as never);
    assert.ok(geometry);
    assert.equal(geometry.vertexCount, FIXTURE.expected_vertex_count);
    assert.equal(geometry.indexCount, FIXTURE.expected_index_count);
    assert.equal(geometry.totalDeltas, FIXTURE.expected_total_deltas);
    assert.equal(geometry.channels.length, FIXTURE.expected_channel_count);
    assert.equal(geometry.topologyHash, FIXTURE.static_payload.topology_hash);
    assert.equal(geometry.vertices.byteLength, geometry.vertexCount * 72);
    assert.equal(geometry.indices.length, geometry.indexCount);
    // Primeira posição e primeiro delta vêm dos bytes do núcleo, bit a bit.
    const first = [
      geometry.vertices[0],
      geometry.vertices[1],
      geometry.vertices[2],
    ];
    for (let i = 0; i < 3; i++) {
      assert.ok(Math.abs(first[i] - FIXTURE.expected_first_position[i]) < 1e-6);
    }
    // O primeiro word de um delta é o `vertex_index` (u32), não um float.
    assert.equal(
      new Uint32Array(geometry.deltas.buffer)[0],
      FIXTURE.expected_first_delta_vertex_index
    );
    const firstDelta = geometry.channels[0];
    assert.equal(firstDelta.startOffset, 0);
    assert.equal(firstDelta.deltaCount, FIXTURE.morph_channels?.[0]?.delta_count ?? firstDelta.deltaCount);
    // Pesos do payload dinâmico chegam nos canais certos.
    const bySlider = new Map(geometry.channels.map((channel) => [channel.sliderId, channel.weight]));
    for (const entry of FIXTURE.dynamic.morph_weights) {
      assert.equal(bySlider.get(entry.slider_id), entry.weight);
    }
    assert.equal(geometry.activeChannelCount, FIXTURE.expected_channel_count);
  });

  it("sem a parte estática devolve null (o cliente mantém os buffers)", () => {
    const snapshot = { ...FIXTURE, static_payload: null };
    assert.equal(viewportGeometryFromSnapshot(snapshot as never), null);
  });

  it("recusa snapshot inválido com erro tipado (nunca renderiza lixo)", () => {
    assert.throws(
      () => viewportGeometryFromSnapshot({ ...FIXTURE, snapshot_version: 99 } as never),
      (error: unknown) => (error as { code?: string }).code === "UNSUPPORTED_VERSION"
    );
    const badStride = JSON.parse(JSON.stringify(FIXTURE));
    badStride.static_payload.vertex_stride_bytes = 64;
    assert.throws(
      () => viewportGeometryFromSnapshot(badStride as never),
      (error: unknown) => (error as { code?: string }).code === "BAD_VERTEX_STRIDE"
    );
  });

  it("a ordenação por vertex_index é verificada (o compute faz busca binária)", () => {
    const geometry = viewportGeometryFromSnapshot(FIXTURE as never)!;
    assertChannelsSorted(new Uint32Array(geometry.deltas.buffer), geometry.channels);
    // Dois deltas fora de ordem (5 depois 3) falham alto em vez de renderizar errado.
    assert.throws(
      () =>
        assertChannelsSorted(new Uint32Array([5, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0]), [
          { sliderId: "head_width", startOffset: 0, deltaCount: 2 },
        ]),
      ViewportGeometryError
    );
    // E um canal curto também é recusado (nenhum delta encontrado).
    assert.throws(
      () =>
        assertChannelsSorted(new Uint32Array(16), [
          { sliderId: "head_width", startOffset: 0, deltaCount: 2 },
        ]),
      ViewportGeometryError
    );
  });

  it("os pesos do frame entram nos registros de 16 B do compute", () => {
    const geometry = viewportGeometryFromSnapshot(FIXTURE as never)!;
    assert.equal(geometry.channelRecords.byteLength, geometry.channels.length * MORPH_CHANNEL_STRIDE_BYTES);
    const boosted = packChannelRecordsWithWeights(geometry.channels, new Map([["head_width", 0.75]]));
    const baseline = packChannelRecords(geometry.channels);
    assert.equal(boosted[0], 0.75);
    // Sliders sem peso no frame voltam a 0 (ausência no snapshot = default).
    assert.equal(boosted[1], 0);
    assert.ok(Math.abs(baseline[0] - 0.2) < 1e-6, `esperado ~0.2, veio ${baseline[0]}`);
    // metadados do canal (offset/delta_count) não mudam com o peso
    const words = new Uint32Array(boosted.buffer);
    assert.equal(words[1], geometry.channels[0].startOffset);
    assert.equal(words[2], geometry.channels[0].deltaCount);
  });

  it("o CPU acumula os deltas do núcleo (caminho WebGL2, sem compute)", () => {
    const geometry = viewportGeometryFromSnapshot(FIXTURE as never)!;
    // Peso 0 ⇒ a malha base volta intacta.
    const base = applyDeltasCpu(geometry, new Map());
    assert.deepEqual(Array.from(base), Array.from(geometry.vertices));
    // Peso 1 no canal head_width ⇒ base + Δ do núcleo, normal normalizada.
    const morphed = applyDeltasCpu(geometry, new Map([["head_width", 1]]));
    assert.notDeepEqual(Array.from(morphed), Array.from(geometry.vertices));
    const movedVertex = new Uint32Array(geometry.deltas.buffer)[0];
    const stride = 18;
    for (let axis = 0; axis < 3; axis++) {
      const expected = geometry.vertices[movedVertex * stride + axis] + geometry.deltas[axis + 1];
      assert.ok(Math.abs(morphed[movedVertex * stride + axis] - expected) < 1e-6);
    }
    const normalLength = Math.hypot(
      morphed[movedVertex * stride + 3],
      morphed[movedVertex * stride + 4],
      morphed[movedVertex * stride + 5]
    );
    assert.ok(Math.abs(normalLength - 1) < 1e-5, `normal normalizada (${normalLength})`);
    // Peso 2 dobra o delta (linearidade do compute canônico).
    const doubled = applyDeltasCpu(geometry, new Map([["head_width", 2]]));
    for (let axis = 0; axis < 3; axis++) {
      const single = morphed[movedVertex * stride + axis] - geometry.vertices[movedVertex * stride + axis];
      const twice = doubled[movedVertex * stride + axis] - geometry.vertices[movedVertex * stride + axis];
      assert.ok(Math.abs(twice - 2 * single) < 1e-5);
    }
  });

  it("a assinatura muda quando a topologia, o catálogo ou o peso mudam", () => {
    const geometry = viewportGeometryFromSnapshot(FIXTURE as never)!;
    const other = {
      ...geometry,
      channels: geometry.channels.map((channel) =>
        channel.sliderId === "head_width" ? { ...channel, weight: 0.9 } : channel
      ),
    };
    assert.notEqual(geometrySignature(geometry), geometrySignature(other));
    assert.equal(geometrySignature(geometry), geometrySignature({ ...geometry }));
  });
});

describe("P0 itens 4–5 — cliente da sessão canônica", () => {
  const CREDENTIALS = {
    history: {
      revision: 12,
      base_geometry_revision: 4,
      dynamic_revision: 12,
      static_revision: 4,
      can_undo: true,
      can_redo: false,
      undo_depth: 6,
      redo_depth: 0,
    },
  };

  function fakeInvoker(handlers: Record<string, (args?: Record<string, unknown>) => unknown>) {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invoker: CoreInvoker = async (command, args) => {
      calls.push({ command, args });
      const handler = handlers[command];
      if (!handler) throw new Error(`no handler for ${command}`);
      return handler(args);
    };
    return { invoker, calls };
  }

  it("os nomes de comando batem com os registrados no app shell", () => {
    for (const operation of [
      "apply",
      "undo",
      "redo",
      "snapshot",
      "history",
      "loadDocument",
      "document",
      "deformedMesh",
    ] as const) {
      assert.match(MAIN_RS, new RegExp(`async fn ${coreCommandName(operation)}\\(`));
    }
    assert.match(MAIN_RS, /core_apply_command,/);
    assert.match(MAIN_RS, /core_snapshot,/);
    // A sessão é quem possui o projeto + histórico canônicos.
    assert.match(MAIN_RS, /pub session: core_session::CoreSession/);
    assert.match(CORE_SESSION_RS, /history:\s*CommandHistory/);
    assert.match(CORE_SESSION_RS, /build_snapshot\(/);
    assert.match(CORE_SESSION_RS, /prepare_base_mesh\(/);
    // Revisão velha ⇒ a geometria volta mesmo sem pedido explícito (é o que faz
    // um undo de proporções chegar ao viewport); revisão atual ⇒ só dinâmico.
    assert.match(CORE_SESSION_RS, /let wants_static = include_static \|\| !fresh_client;/);
    assert.match(CORE_SESSION_RS, /client_static_revision == Some\(geometry\.static_revision\)/);
    // Cache de geometria por gênero/proporções/dimorfismo: peso de slider não
    // reconstrói malha (o que o teste `morph_edits_do_not_rebuild...` prova).
    assert.match(CORE_SESSION_RS, /fn geometry_key\(/);
    assert.match(CORE_SESSION_RS, /PROPORTION|proportions|gender_dimorphism/);
  });

  it("sem núcleo a ponte é explicitamente indisponível (nunca deforma)", async () => {
    assert.equal(await resolveCoreInvoker(), null);
    const client = new CoreSessionClient(null);
    assert.equal(client.available, false);
    await assert.rejects(() => client.pullSnapshot(), (error: unknown) => {
      assert.ok(error instanceof CoreBridgeError);
      assert.equal(error.code, "core_unavailable");
      return true;
    });
  });

  it("busca a parte estática uma vez e depois só a dinâmica", async () => {
    const dynamicOnly = { ...FIXTURE, static_payload: null };
    const { invoker, calls } = fakeInvoker({
      [coreCommandName("snapshot")]: (args) =>
        args?.clientStaticRevision === FIXTURE.dynamic.static_revision ? dynamicOnly : FIXTURE,
    });
    const client = new CoreSessionClient(invoker);
    const first = await client.pullSnapshot();
    assert.equal(first.includeStatic, true);
    assert.ok(first.geometry);
    assert.equal(client.static_revision, FIXTURE.dynamic.static_revision);

    const second = await client.pullSnapshot();
    assert.equal(second.includeStatic, false);
    assert.equal(second.geometry, null);
    assert.deepEqual(
      calls.map((call) => call.args?.clientStaticRevision),
      [null, FIXTURE.dynamic.static_revision]
    );
  });

  it("descarta snapshot velho que chegue fora de ordem", async () => {
    const newer = JSON.parse(JSON.stringify({ ...FIXTURE, static_payload: null }));
    newer.dynamic.dynamic_revision = 42;
    const older = JSON.parse(JSON.stringify({ ...FIXTURE, static_payload: null }));
    older.dynamic.dynamic_revision = 10;
    let response = newer;
    const { invoker } = fakeInvoker({
      [coreCommandName("snapshot")]: () => response,
    });
    const client = new CoreSessionClient(invoker);
    await client.pullSnapshot();
    assert.equal(client.dynamic_revision, 42);
    response = older;
    await assert.rejects(() => client.pullSnapshot(), (error: unknown) => {
      assert.equal((error as CoreBridgeError).code, "invalid_response");
      return true;
    });
    assert.equal(client.dropped_snapshots, 1);
    assert.equal(client.dynamic_revision, 42, "revisão boa é preservada");
  });

  it("comandos passam pelo núcleo e devolvem o outcome dele", async () => {
    const { invoker, calls } = fakeInvoker({
      [coreCommandName("apply")]: (args) => ({
        sequence: 3,
        revision: 9,
        base_geometry_revision: 2,
        description: `aplicado ${(args?.command as { kind: string }).kind}`,
        scope: "deformation",
        affected: ["mrf_head_width"],
        can_undo: true,
        can_redo: false,
        undo_depth: 3,
        redo_depth: 0,
      }),
      [coreCommandName("undo")]: () => ({
        sequence: 4,
        revision: 8,
        base_geometry_revision: 2,
        description: "Undo",
        scope: "deformation",
        affected: [],
        can_undo: false,
        can_redo: true,
        undo_depth: 2,
        redo_depth: 1,
      }),
      [coreCommandName("history")]: () => CREDENTIALS.history,
    });
    const client = new CoreSessionClient(invoker);
    const outcome = await client.applyCommand({
      kind: "set_morph_value",
      target: "height_overall",
      value: 1.7,
    });
    assert.equal(outcome.revision, 9);
    assert.equal(client.dynamic_revision, 9);
    assert.equal(calls[0].command, coreCommandName("apply"));
    assert.deepEqual((calls[0].args?.command as { kind: string }).kind, "set_morph_value");

    const undone = await client.undo();
    assert.equal(undone.revision, 8);
    assert.equal(client.dynamic_revision, 8);

    const report = await client.historyState();
    assert.ok(isCoreHistoryReport(report));
    assert.equal(report.base_geometry_revision, 4);
  });

  it("recusa comando com kind fora do contrato antes de chegar ao núcleo", async () => {
    const { invoker, calls } = fakeInvoker({});
    const client = new CoreSessionClient(invoker);
    await assert.rejects(
      () => client.applyCommand({ kind: "teleport" } as never),
      (error: unknown) => (error as CoreBridgeError).code === "rejected_by_core"
    );
    assert.equal(calls.length, 0);
  });

  it("valida a forma do snapshot/histórico vindos do núcleo", () => {
    assert.equal(isCoreSnapshot(FIXTURE), true);
    assert.equal(isCoreSnapshot({ snapshot_version: 1 }), false);
    assert.equal(isCoreSnapshot({ snapshot_version: 2, dynamic: {} }), false);
    assert.equal(isCoreHistoryReport(CREDENTIALS.history), true);
    assert.equal(isCoreHistoryReport({ revision: 1 }), false);
  });

  it("peso de canal é valor − default (mesma definição de DeformationInputs)", () => {
    const slider = CANONICAL_SLIDERS.find((entry) => entry.id === "height_overall")!;
    assert.equal(channelWeightOf(1.7, slider.defaultValue), 1.7 - slider.defaultValue);
    assert.equal(channelWeightOf(slider.defaultValue, slider.defaultValue), 0);
    assert.equal(channelWeightOf(Number.NaN, slider.defaultValue), 0);
    // E a fórmula do núcleo é literalmente a mesma (fonte Rust).
    const rust = readRepoFile("crates/anigo-core/src/project.rs");
    assert.match(rust, /let weight = value - def\.default_value;/);
  });
});

describe("P0 itens 4–5 — não existe deformação de produção em TypeScript", () => {
  it("o renderer só aplica snapshots do núcleo", () => {
    assert.match(RENDERER_SOURCE, /public applyCoreSnapshot\(delivery: CoreSnapshotDelivery\)/);
    assert.match(RENDERER_SOURCE, /viewportGeometryFromDecoded\(/);
    assert.match(RENDERER_SOURCE, /packChannelRecordsWithWeights\(/);
    assert.match(RENDERER_SOURCE, /onCoreGeometryRequired/);
    assert.equal(
      /applyAnatomicalDeformations|genericMorphDelta|recomputeNormals|buildSparseMorphSet|packChannelWeights\(/
        .test(RENDERER_SOURCE),
      false
    );
    // A malha canônica nunca é gerada localmente: o único caminho é o snapshot.
    assert.equal(/generateCanonicalMesh|createCanonicalBase/.test(RENDERER_SOURCE), false);
  });

  it("a cor de fundo do render vem do snapshot (não é mais literal)", () => {
    assert.match(RENDERER_SOURCE, /delivery\.state\?\.render\?\.background_color/);
    assert.match(RENDERER_SOURCE, /clearValue: \{[\s\S]*?this\.clearColor\[0\]/);
    assert.equal(
      /clearValue: \{ r: 0\.08, g: 0\.09, b: 0\.13/.test(RENDERER_SOURCE),
      false,
      "clear color literal removida (fonte é o snapshot)"
    );
    assert.match(RENDERER_SOURCE, /applyDeltasCpu\(this\.coreGeometry, this\.effectiveChannelWeights\(\)\)/);
  });

  it("o viewport e o shell conversam pelo snapshot (nada de setter deformante)", () => {
    assert.match(VIEWPORT_SOURCE, /renderer\.applyCoreSnapshot\(delivery\)/);
    assert.match(VIEWPORT_SOURCE, /renderer\.onCoreGeometryRequired = \(\) => \{/);
    assert.match(VIEWPORT_SOURCE, /coreSnapshotProvider/);
    assert.match(APP_SOURCE, /new CoreSessionClient\(await resolveCoreInvoker\(\)\)/);
    assert.match(APP_SOURCE, /coreSnapshotProvider=\{coreSnapshotProvider\}/);
    assert.match(APP_SOURCE, /await coreClient\.applyCommand\(command\)/);
  });

  it("a autoridade degradada é anunciada, não escondida", () => {
    assert.match(RENDERER_SOURCE, /coreAuthority = "unavailable"/);
    assert.match(RENDERER_SOURCE, /modo degradado/);
    assert.match(RENDERER_SOURCE, /getDeformationAuthority\(\)/);
    assert.match(VIEWPORT_SOURCE, /export function getDeformationAuthority\(\)/);
  });
});
