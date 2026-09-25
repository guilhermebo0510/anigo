/**
 * ANIGO — P1 "Robustez" (§7): itens 5, 6 e 7.
 *
 * Itens 1 e 2 (fim do panic/silêncio + diagnóstico) estão em
 * `diagnostics_contract.test.ts`; item 3 (validação antes de buffers) em
 * `mesh_validation.test.ts`.
 *
 * Aqui ficam os invariantes que precisam dos **dois** lados:
 *   * 5 — quando uma meta deforma posição sem delta de normal, quem recalcula é
 *     o núcleo (dono da topologia) e a delta de normal é o que os consumidores
 *     aplicam; WGSL e o caminho WebGL2 no CPU normalizam sob a mesma condição.
 *   * 6 — telemetria só com dado medido (defaults explicitamente "não medido",
 *     diagnóstico nativo junto do diagnóstico do viewport).
 *   * 7 — CI rodando build, testes, contratos congelados e Rust.
 */
import test from "node:test";
import assert from "node:assert/strict";

import { readRepoFile } from "./rust_contract_source.ts";
import { applyDeltasCpu, type ViewportGeometry } from "../../src/services/viewport_mesh.ts";
import { VERTEX_STRIDE_BYTES } from "../../src/services/mesh_validation.ts";

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

test("item 5 — nucleo recalcula normais das metas que nao trazem delta", () => {
  const morph = readRepoFile("crates/anigo-core/src/morph.rs");
  assert.match(morph, /pub fn has_normal_deltas\(&self\) -> bool/);
  assert.match(morph, /pub fn targets_without_normal_deltas\(&self\) -> Vec<usize>/);
  assert.match(morph, /pub fn complete_normal_deltas\(&mut self, base_vertices: &\[Vertex\], indices: &\[u32\]\) -> u32/);
  assert.match(morph, /fn vertex_normals\(positions: &\[\[f32; 3\]\], indices: &\[u32\]\)/);
  // área-ponderada: a normal da face entra sem normalizar
  assert.match(morph, /ab\[1\] \* ac\[2\] - ab\[2\] \* ac\[1\]/);
  // normalização sob a mesma condição do WGSL/CPU
  assert.match(morph, /length_sq > 1e-12/);
  // o builder canônico completa as metas antes de entregar
  const catalog = readRepoFile("crates/anigo-core/src/morph_catalog.rs");
  assert.match(catalog, /complete_normal_deltas\(&base_mesh\.vertices, &base_mesh\.indices\)/);
  // `apply_cpu` não derruba mais o processo com tamanhos incompatíveis
  const production = stripTestModules(morph);
  assert.doesNotMatch(production, /assert_eq!\(\s*base_vertices\.len\(\)/);
});

test("item 5 — consumidores normalizam sob a mesma condicao", () => {
  const wgsl = readRepoFile("crates/anigo-renderer/shaders/morph_sparse_compute.wgsl");
  assert.match(wgsl, /delta_nx/, "o compute precisa somar as deltas de normal");
  assert.match(wgsl, /n_sq > 1e-12/, "o compute precisa normalizar a normal acumulada");
  const cpu = readRepoFile("src/services/viewport_mesh.ts");
  assert.match(cpu, /squared > 1e-12/, "o caminho CPU usa o mesmo limiar do compute");
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /applyCoreDeltasToGlBuffer/, "WebGL2 aplica as deltas do núcleo no CPU");
});

test("item 5 — applyDeltasCpu aplica e normaliza normais", () => {
  const floatsPerVertex = VERTEX_STRIDE_BYTES / 4;
  const vertices = new Float32Array(2 * floatsPerVertex);
  vertices[3] = 0;
  vertices[4] = 0;
  vertices[5] = 1; // normal base do vértice 0 = +Z
  vertices[floatsPerVertex + 3] = 0;
  vertices[floatsPerVertex + 4] = 1;
  vertices[floatsPerVertex + 5] = 0;

  // Uma delta no vértice 0 com peso 1 (a delta em si é vazia, então a normal
  // de base precisa sobreviver — o que se testa aqui é o caminho do CPU).
  const deltas = new Float32Array(8);
  const geometry = {
    vertices,
    indices: new Uint32Array([0, 1, 0]),
    deltas,
    channels: [{ target: "head_width", sliderId: "head_width", weight: 0, startOffset: 0, deltaCount: 1 }],
    vertexCount: 2,
    indexCount: 3,
    triangleCount: 1,
    channelRecords: new Float32Array(4),
    totalDeltas: 1,
    activeChannelCount: 1,
    topologyHash: "0",
    catalogFingerprint: "0",
    staticRevision: 0,
    baseGender: "Male",
    meshUri: "anigo://base/anigo_base_male.glb",
  } as unknown as ViewportGeometry;

  const out = applyDeltasCpu(geometry, new Map([["head_width", 1]]));
  const length = Math.hypot(out[3], out[4], out[5]);
  assert.ok(Math.abs(length - 1) < 1e-6, "normal precisa sair normalizada");
  // Deltas vazias ⇒ normal de base preservada.
  assert.ok(Math.abs(out[3] - 0) < 1e-6);
  assert.ok(Math.abs(out[5] - 1) < 1e-6);
});

test("item 6 — telemetria so com dado medido", () => {
  const bridge = readRepoFile("src-tauri/src/bridge.rs");
  for (const field of [
    "diagnostics_errors",
    "diagnostics_warnings",
    "diagnostic_codes",
    "deformation_authority",
    "core_static_revision",
    "core_dynamic_revision",
    "morph_channels",
    "vertex_count",
    "native_diagnostics_errors",
    "native_diagnostic_codes",
    "native_contract_valid",
  ]) {
    assert.match(bridge, new RegExp(`pub ${field}:`), `campo de telemetria '${field}' ausente`);
  }
  // Defaults explícitos de "não medido" (antes: 120 fps / 156 triângulos inventados).
  assert.match(bridge, /fps: 0\.0/);
  assert.match(bridge, /frame_time_ms: 0\.0/);
  assert.match(bridge, /triangle_count: 0/);
  assert.match(bridge, /adapter_name: "unknown"\.into\(\)/);
  assert.match(bridge, /deformation_authority: "unknown"\.into\(\)/);
});

test("item 6 — telemetria nativa e do viewport nao se sobrescrevem", () => {
  const main = readRepoFile("src-tauri/src/main.rs");
  assert.match(main, /anigo_renderer::diagnostics_summary\(\)/);
  assert.match(main, /state\.native_diagnostics_errors/);
  assert.match(main, /state\.native_contract_valid = anigo_renderer::render_contract::contract_is_valid\(\)/);
  const viewport = readRepoFile("src/components/viewport/Viewport.svelte");
  assert.match(viewport, /diagnostics_errors: diagnostics\.errors/);
  assert.match(viewport, /deformation_authority: getDeformationAuthority\(\)\.authority/);
  assert.match(viewport, /vertex_count: getDeformationAuthority\(\)\.vertexCount/);
  // FPS/tempo de frame vêm do laço de render, não de constante.
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /frameTimeMs: elapsed/);
  assert.match(renderer, /fps: currentFps/);
});

test("item 7 — CI cobre build, testes, contratos e Rust", () => {
  const ci = readRepoFile(".github/workflows/ci.yml");
  assert.match(ci, /npm test/);
  assert.match(ci, /npm run fixtures:check/);
  assert.match(ci, /npm run build/);
  assert.match(ci, /scripts\/check_rust_syntax\.mjs/);
  assert.match(ci, /pull_request:/);
  assert.match(ci, /branches: \[main, arena\/\*\*\]/);
  // job com toolchain Rust
  assert.match(ci, /cargo check --workspace --all-targets/);
  assert.match(ci, /cargo test -p anigo-core/);
  assert.match(ci, /cargo test -p anigo-renderer/);
  assert.match(ci, /dtolnay\/rust-toolchain@stable/);
  // shaders de produção validados (o sandbox não tem naga; o CI tem).
  // O passo valida um arquivo por vez: o 2º positional do naga-cli é o
  // ARQUIVO DE SAÍDA ("naga a.wgsl b.wgsl" sobrescreve b com a normalizado!)
  assert.match(ci, /for f in crates\/anigo-renderer\/shaders\/cel_shading\.wgsl/);
  assert.match(ci, /naga "\$f" \/dev\/null > \/tmp\/naga-log\.txt/);
});
