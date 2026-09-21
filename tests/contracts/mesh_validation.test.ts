/**
 * ANIGO — validação de malha antes de criar buffers (P1 — Robustez, item 3).
 *
 * A regra do item é simples: **nenhum** buffer de GPU nasce de uma malha que não
 * passou pela validação. Estes testes cobrem o validador do TypeScript, a
 * paridade do vocabulário com o render contract (o Rust lê o mesmo documento) e
 * a ordem em que as checagens acontecem em relação à criação de buffers.
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  MESH_VALIDATION_CODES,
  MeshValidationError,
  VERTEX_STRIDE_BYTES,
  assertRenderableMesh,
  meshValidationCatalogProblems,
  validateAttributeMesh,
  validateRenderableMesh,
} from "../../src/services/mesh_validation.ts";
import { RENDER_CONTRACT } from "../../src/contracts/render_contract.v1.ts";
import { readRepoFile } from "./rust_contract_source.ts";

/** Triângulo válido empacotado no stride canônico (72 B). */
function packedTriangle(): { vertices: Float32Array; indices: Uint32Array; stride: number } {
  const stride = VERTEX_STRIDE_BYTES;
  const floats = stride / 4;
  const vertices = new Float32Array(3 * floats);
  // posição fica nos 3 primeiros floats de cada vértice; w = 1.0 no último
  vertices[0] = 0;
  vertices[floats + 0] = 1;
  vertices[2 * floats + 1] = 1;
  return { vertices, indices: new Uint32Array([0, 1, 2]), stride };
}

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

test("catalogo de validacao de malha bate com o render contract", () => {
  assert.deepEqual(meshValidationCatalogProblems(), []);
  const declared = (RENDER_CONTRACT.mesh_validation?.codes ?? []).map((entry) => entry.code);
  assert.deepEqual([...MESH_VALIDATION_CODES].sort(), [...declared].sort());
  assert.equal(RENDER_CONTRACT.mesh_validation?.stride_bytes, VERTEX_STRIDE_BYTES);
  assert.equal(VERTEX_STRIDE_BYTES, 72);
});

test("malha valida passa", () => {
  const { vertices, indices, stride } = packedTriangle();
  const result = validateRenderableMesh(vertices, indices, { strideBytes: stride });
  assert.equal(result.ok, true);
  assert.equal(result.code, null);
  assert.equal(result.vertexCount, 3);
  assert.equal(result.indexCount, 3);
  assert.doesNotThrow(() => assertRenderableMesh(vertices, indices, { strideBytes: stride }));
});

test("cada codigo do contrato tem um caminho que o produz", () => {
  const { vertices, indices, stride } = packedTriangle();

  const empty = validateRenderableMesh(new Float32Array(0), new Uint32Array(0));
  assert.equal(empty.code, "EMPTY_MESH");

  const badStride = validateRenderableMesh(new Float32Array(7), indices, { strideBytes: stride });
  assert.equal(badStride.code, "BAD_VERTEX_STRIDE");

  const badTopology = validateRenderableMesh(vertices, new Uint32Array([0, 1]), { strideBytes: stride });
  assert.equal(badTopology.code, "INDEX_COUNT_MISMATCH");

  const outOfRange = validateRenderableMesh(vertices, new Uint32Array([0, 1, 9]), {
    strideBytes: stride,
  });
  assert.equal(outOfRange.code, "INDEX_OUT_OF_RANGE");

  const nanVertices = new Float32Array(vertices);
  nanVertices[1] = Number.NaN;
  const nonFinite = validateRenderableMesh(nanVertices, indices, { strideBytes: stride });
  assert.equal(nonFinite.code, "NON_FINITE_VALUE");
});

test("validador de atributos (caminho do GLB) reprova o que deve", () => {
  const positions = [0, 0, 0, 1, 0, 0, 0, 1, 0];
  const indices = [0, 1, 2];
  assert.equal(validateAttributeMesh(positions, indices).ok, true);

  assert.equal(validateAttributeMesh([], []).code, "EMPTY_MESH");
  assert.equal(validateAttributeMesh(positions, [0, 1]).code, "INDEX_COUNT_MISMATCH");
  assert.equal(validateAttributeMesh(positions, [0, 1, 7]).code, "INDEX_OUT_OF_RANGE");
  assert.equal(
    validateAttributeMesh(positions, indices, { normals: [0, 0, 0] }).code,
    "BAD_VERTEX_STRIDE"
  );
  assert.equal(
    validateAttributeMesh([0, 0, 0, Number.NaN, 0, 0, 0, 1, 0], indices).code,
    "NON_FINITE_VALUE"
  );
  const error = new MeshValidationError("NON_FINITE_VALUE", "normal com NaN");
  assert.equal(error.code, "NON_FINITE_VALUE");
  assert.match(error.message, /NON_FINITE_VALUE/);
});

test("renderer valida antes de criar buffers", () => {
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  const coreUpload = renderer.indexOf("private uploadCoreGeometry(");
  const glbLoad = renderer.indexOf("public async loadCanonicalModel(");
  assert.ok(coreUpload > 0 && glbLoad > 0);
  const coreValidation = renderer.indexOf("validateRenderableMesh(incoming.vertices, incoming.indices)");
  const glbValidation = renderer.indexOf("validateAttributeMesh(mesh.positions, rawIndices");
  assert.ok(
    coreValidation > 0 && coreValidation < coreUpload,
    "a geometria do núcleo precisa ser validada antes de uploadCoreGeometry"
  );
  const firstBuffer = renderer.indexOf("this.device.createBuffer(", coreValidation);
  assert.ok(firstBuffer > coreValidation, "nenhum buffer pode nascer antes da validação");
  assert.ok(glbValidation > glbLoad, "loadCanonicalModel precisa validar o GLB decodificado");
  assert.match(renderer, /this\.report\("geometry_rejected"/);
  assert.match(renderer, /this\.report\("mesh_invalid"/);
});

test("headless valida a malha antes do VBO", () => {
  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  const firstValidation = headless.indexOf("mesh_validation::validate_mesh(mesh)");
  const firstBuffer = headless.indexOf("let v_buffer = self.device.create_buffer_init");
  assert.ok(firstValidation > 0, "headless precisa validar a malha");
  assert.ok(firstValidation < firstBuffer, "validação precisa vir antes da criação do VBO");
  assert.equal(
    (headless.match(/mesh_validation::validate_mesh\(mesh\)/g) ?? []).length,
    2,
    "os dois caminhos de render (normal e sparse morphs) precisam validar"
  );
  assert.match(headless, /diagnostics::report_with_detail\(\s*"mesh_invalid"/);
});

test("validador do Rust sai do contrato e nao usa panic", () => {
  const rust = readRepoFile("crates/anigo-renderer/src/mesh_validation.rs");
  assert.match(rust, /\["mesh_validation"\]\["codes"\]/);
  assert.match(rust, /\["mesh_validation"\]\["stride_bytes"\]/);
  assert.match(rust, /"EMPTY_MESH"/);
  assert.match(rust, /"INDEX_OUT_OF_RANGE"/);
  assert.match(rust, /"NON_FINITE_VALUE"/);
  assert.match(rust, /"BAD_VERTEX_STRIDE"/);
  assert.match(rust, /"INDEX_COUNT_MISMATCH"/);
  const production = stripTestModules(rust);
  const hits = production.match(/\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(|todo!\(/g) ?? [];
  assert.deepEqual(hits, []);
});

test("stride do Vertex do Rust e o mesmo do contrato", () => {
  const source = readRepoFile("crates/anigo-core/src/mesh.rs");
  const vertex = /pub struct Vertex \{(?<body>[^}]*)\}/.exec(source);
  assert.ok(vertex?.groups?.body, "estrutura Vertex não encontrada em mesh.rs");
  const body = vertex.groups.body;
  // posição(3) + normal(3) + uv(2) + cor(4) + joints(4×u16) + pesos(4) = 72 B
  assert.match(body, /pub position: \[f32; 3\]/);
  assert.match(body, /pub normal: \[f32; 3\]/);
  assert.match(body, /pub uv: \[f32; 2\]/);
  assert.match(body, /pub color: \[f32; 4\]/);
  assert.match(body, /pub joints: \[u16; 4\]/);
  assert.match(body, /pub weights: \[f32; 4\]/);
  const bytes = 3 * 4 + 3 * 4 + 2 * 4 + 4 * 4 + 4 * 2 + 4 * 4;
  assert.equal(bytes, VERTEX_STRIDE_BYTES);
  const contract = readRepoFile("crates/anigo-renderer/src/mesh_validation.rs");
  assert.match(contract, /size_of::<Vertex>\(\) as u64 != stride/);
});
