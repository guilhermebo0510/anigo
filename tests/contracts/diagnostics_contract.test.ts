/**
 * ANIGO — teste de contrato dos diagnósticos (P1 — Robustez, itens 1 e 2).
 *
 * O que estes testes protegem:
 *   1. o catálogo do TypeScript é o mesmo do render contract (que o Rust lê);
 *   2. severidade vem do contrato, não de um literal duplicado;
 *   3. repetição agrega e nunca infla a lista sem limite (o render loop chama
 *      `report` a 60 fps);
 *   4. código desconhecido é erro de contrato;
 *   5. o caminho crítico do renderer/viewport fala pelo canal de diagnóstico em
 *      vez de `console.warn` solto / `catch (_) {}`;
 *   6. nenhum `unwrap()`/`expect(`/`panic!(` sobra fora dos módulos de teste em
 *      `crates/anigo-renderer` e `crates/anigo-core` (P1-01).
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  DIAGNOSTIC_CODES,
  DiagnosticContractError,
  RendererDiagnostics,
  diagnosticCatalogProblems,
  isDiagnosticCode,
  severityOf,
} from "../../src/services/render_diagnostics.ts";
import { diagnosticCodeSpecs, RENDER_CONTRACT } from "../../src/contracts/render_contract.v1.ts";
import { readRepoFile } from "./rust_contract_source.ts";

/**
 * Remove `#[cfg(test)] mod <nome> { … }` (brace-matched) para que asserts de
 * teste não sejam confundidos com `panic!` de produção.
 */
function stripTestModules(source: string): string {
  const marker = /#\[cfg\(test\)\]\s*mod\s+\w+\s*\{/;
  let remaining = source;
  let result = "";
  for (;;) {
    const match = marker.exec(remaining);
    if (!match) {
      result += remaining;
      return result;
    }
    result += remaining.slice(0, match.index);
    let index = match.index + match[0].length;
    let depth = 1;
    while (index < remaining.length && depth > 0) {
      if (remaining[index] === "{") depth += 1;
      else if (remaining[index] === "}") depth -= 1;
      index += 1;
    }
    remaining = remaining.slice(index);
  }
}

test("catalogo de diagnostico bate com o render contract", () => {
  assert.deepEqual(diagnosticCatalogProblems(), []);
  const contractCodes = diagnosticCodeSpecs().map((spec) => spec.code);
  assert.deepEqual([...DIAGNOSTIC_CODES].sort(), [...contractCodes].sort());
  assert.ok(contractCodes.length >= 20, "catálogo do contrato ficou pequeno demais");
});

test("severidade vem do contrato (uma definição só para Rust e TypeScript)", () => {
  for (const spec of diagnosticCodeSpecs()) {
    assert.equal(severityOf(spec.code as (typeof DIAGNOSTIC_CODES)[number]), spec.severity, spec.code);
  }
  assert.equal(severityOf("webgpu_unavailable"), "warning");
  assert.equal(severityOf("backend_unavailable"), "error");
  assert.equal(severityOf("readback_failed"), "error");
});

test("codigo desconhecido e recusado e nada e registrado", () => {
  const diagnostics = new RendererDiagnostics();
  assert.throws(() => diagnostics.report("nao_existe" as never, "x"), DiagnosticContractError);
  assert.equal(diagnostics.entries.length, 0);
  assert.equal(isDiagnosticCode("nao_existe"), false);
  assert.equal(isDiagnosticCode("frame_skipped"), true);
});

test("repeticao agrega em vez de crescer (render loop a 60 fps)", () => {
  let now = 0;
  const diagnostics = new RendererDiagnostics({ clock: () => now });
  diagnostics.report("frame_skipped", "frame pulado");
  now = 16;
  diagnostics.report("frame_skipped", "frame pulado");
  now = 32;
  diagnostics.report("gpu_device_error", "erro de device", { detail: "OOM" });

  assert.equal(diagnostics.entries.length, 2);
  const skipped = diagnostics.of("frame_skipped")[0];
  assert.equal(skipped.count, 2);
  assert.equal(skipped.firstAt, 0);
  assert.equal(skipped.lastAt, 16);
  assert.equal(diagnostics.last?.code, "gpu_device_error");
  assert.equal(diagnostics.last?.detail, "OOM");

  const summary = diagnostics.summary();
  assert.deepEqual(summary.codes, ["frame_skipped", "gpu_device_error"]);
  assert.equal(summary.errors, 1);
  assert.equal(summary.warnings, 1);
  assert.equal(summary.degraded, true);
});

test("limite de capacidade conta o excedente em vez de perder o dado", () => {
  const diagnostics = new RendererDiagnostics({ capacity: 2, clock: () => 0 });
  const recorded = diagnostics.report("channel_missing", "primeiro");
  assert.equal(recorded.count, 1);
  diagnostics.report("snapshot_stale", "segundo");
  diagnostics.report("geometry_unavailable", "terceiro");
  assert.equal(diagnostics.entries.length, 2);
  assert.equal(diagnostics.dropped, 1);
  assert.equal(diagnostics.summary().dropped, 1);
  diagnostics.clear();
  assert.equal(diagnostics.entries.length, 0);
  assert.equal(diagnostics.dropped, 0);
});

test("formato de log identifica codigo e contexto", () => {
  const diagnostics = new RendererDiagnostics();
  const diagnostic = diagnostics.report("model_load_failed", "falha ao carregar", {
    detail: "404",
    context: { gender: "male" },
  });
  const line = RendererDiagnostics.format(diagnostic);
  assert.match(line, /model_load_failed/);
  assert.match(line, /404/);
  assert.match(line, /male/);
});

test("renderer usa o canal de diagnostico em vez de falha silenciosa", () => {
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  for (const code of [
    "webgpu_unavailable",
    "backend_unavailable",
    "gpu_device_error",
    "compute_init_failed",
    "channel_upload_failed",
    "context_configure_failed",
    "frame_skipped",
    "model_load_failed",
    "geometry_unavailable",
    "channel_missing",
  ]) {
    assert.match(renderer, new RegExp(`this\\.report\\("${code}"`), `código '${code}' não é reportado`);
  }
  // Os avisos migrados não podem voltar como log solto.
  assert.doesNotMatch(renderer, /console\.warn\("\[ANIGO 3D\] WebGPU initialization failed/);
  assert.doesNotMatch(renderer, /console\.warn\("\[ANIGO 3D\] Compute pipeline initialization fallback/);
  assert.doesNotMatch(renderer, /console\.warn\("\[ANIGO 3D\] core geometry hook failed/);
  assert.doesNotMatch(renderer, /console\.error\("\[ANIGO\]\[GPU\] uncaptured error/);
  // `catch (_) {}` só pode sobrar em limpeza de recursos (destroy/dispose).
  const silentCatches = renderer.match(/catch \(_\) \{\}/g) ?? [];
  assert.ok(
    silentCatches.length <= 20,
    `catch silencioso voltou ao caminho crítico (${silentCatches.length} ocorrências)`
  );
});

test("viewport e shell propagam o diagnostico do renderer", () => {
  const viewport = readRepoFile("src/components/viewport/Viewport.svelte");
  assert.match(viewport, /renderer\.onDiagnostic\s*=/);
  assert.match(viewport, /getDiagnostics\(\)/);
  assert.match(viewport, /diagnostics_errors/);
  const app = readRepoFile("src/App.svelte");
  assert.match(app, /onDiagnostic=\{handleViewportDiagnostic\}/);
  assert.match(app, /refreshViewportDiagnostics/);
  assert.match(app, /reportDiagnostic/);
});

test("codigos de diagnostico do Rust saem do contrato", () => {
  const rust = readRepoFile("crates/anigo-renderer/src/render_contract.rs");
  assert.match(rust, /fn diagnostic_codes\(\)/);
  assert.match(rust, /fn diagnostic_severity\(/);
  assert.match(rust, /\["diagnostics"\]\["codes"\]/);
  const sink = readRepoFile("crates/anigo-renderer/src/diagnostics.rs");
  assert.match(sink, /pub fn report\(/);
  assert.match(sink, /pub fn summary\(\)/);
  assert.match(sink, /render_contract::diagnostic_severity\(code\)/);
  assert.doesNotMatch(sink, /\.unwrap\(\)/);
});

test("nenhum panic fora dos modulos de teste no caminho critico do Rust", () => {
  const files = [
    "crates/anigo-renderer/src/render_contract.rs",
    "crates/anigo-renderer/src/headless.rs",
    "crates/anigo-renderer/src/diagnostics.rs",
    "crates/anigo-renderer/src/uniforms.rs",
    "crates/anigo-core/src/mesh.rs",
    "crates/anigo-core/src/snapshot.rs",
    "crates/anigo-core/src/command.rs",
    "crates/anigo-core/src/project.rs",
  ];
  for (const file of files) {
    const production = stripTestModules(readRepoFile(file));
    const hits = production.match(/\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(|todo!\(/g) ?? [];
    assert.deepEqual(hits, [], `${file} ainda tem panic/expect no código de produção`);
  }
});

test("o render contract declara os diagnosticos que o headless emite", () => {
  const diagnostics = (RENDER_CONTRACT as { diagnostics?: { codes: Array<{ code: string }> } })
    .diagnostics;
  assert.ok(diagnostics, "render contract sem seção `diagnostics`");
  const codes = new Set(diagnostics.codes.map((spec) => spec.code));
  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  const emitted = [...headless.matchAll(/diagnostics::report(?:_with_detail)?\(\s*"([a-z_]+)"/g)].map(
    (match) => match[1]
  );
  assert.ok(emitted.length > 0, "headless não emite diagnóstico nenhum");
  for (const code of emitted) {
    assert.ok(codes.has(code), `headless emite '${code}', que não está no contrato`);
  }
});
