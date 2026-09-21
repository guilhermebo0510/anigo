/**
 * ANIGO — contrato de exportação + paridade viewport ⇄ exportação
 * (P0 "Consolidar o renderer", §8 aceite).
 *
 * O documento canônico exige duas coisas que este teste verifica juntas:
 *
 *   * "viewport, exportação e headless usarem os mesmos contratos";
 *   * "viewport e exportação passarem teste de paridade".
 *
 * Camadas:
 *   1. contrato — `readExportManifest` valida o fixture congelado (o Rust valida
 *      o mesmo arquivo do outro lado);
 *   2. checksums — os byte layouts (`pack_vertices` de 72 B, índices, paleta) são
 *      reconstruídos aqui e comparados com os valores que o núcleo gravou no
 *      fixture: é a mesma conta dos dois lados, não uma reimplementação;
 *   3. paridade de verdade — o snapshot do núcleo vira geometria de viewport e a
 *      malha deformada é acumulada com `applyDeltasCpu` (o caminho canônico do
 *      WebGL2) e conferida contra os checksums do manifesto.
 */

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  EXPORT_FORMAT_VERSION,
  EXPORT_GENERATOR,
  EXPORT_VERTEX_STRIDE_BYTES,
  ExportManifestError,
  bufferChecksum,
  indexBytes,
  indexChecksum,
  packedPositionChecksum,
  packedVertexBytes,
  packedVertexChecksum,
  paletteBytes,
  paletteChecksum,
  readExportManifest,
  type ExportManifestWire,
} from "../../src/contracts/export_manifest.v1.ts";
import {
  decodeCoreSnapshot,
  type CoreSnapshotWire,
} from "../../src/contracts/core_snapshot.v1.ts";
import {
  applyDeltasCpu,
  viewportGeometryFromDecoded,
  type ViewportGeometry,
} from "../../src/services/viewport_mesh.ts";
import {
  EXPORT_COMMANDS,
  describeParity,
  exportCanonicalFrame,
  exportCanonicalGlb,
  exportFileName,
  type ExportArtifactResult,
} from "../../src/services/export_service.ts";
import { checkExportParity } from "../../src/services/export_service.ts";
import { renderPassOrder } from "../../src/contracts/render_contract.v1.ts";

const readRepoFile = (relative: string): string =>
  readFileSync(fileURLToPath(new URL(`../../${relative}`, import.meta.url)), "utf8");

const snapshotFixture = JSON.parse(
  readRepoFile("contracts/fixtures/core_snapshot_v1.json")
) as CoreSnapshotWire;
const manifestFixture = JSON.parse(
  readRepoFile("contracts/fixtures/export_manifest_v1.json")
) as ExportManifestWire;

/** Geometria do viewport montada pelo caminho real (snapshot → viewport). */
function viewportFixture(): ViewportGeometry {
  const decoded = decodeCoreSnapshot(snapshotFixture);
  assert.ok(decoded.geometry, "o fixture precisa trazer a geometria estática");
  return viewportGeometryFromDecoded(decoded.geometry!, decoded.morphWeights);
}

test("contrato: o fixture do manifesto é válido e determinístico", () => {
  const manifest = readExportManifest(manifestFixture);
  assert.equal(manifest.format_version, EXPORT_FORMAT_VERSION);
  assert.equal(manifest.generator, EXPORT_GENERATOR);
  assert.equal(manifest.geometry.vertex_stride_bytes, EXPORT_VERTEX_STRIDE_BYTES);
  assert.equal(manifest.deformation.authority, "core");
  assert.equal(manifest.skin.bone_count, 24);
  assert.equal(
    manifest.skin.skinned_vertices + manifest.skin.unskinned_vertices,
    manifest.geometry.vertex_count
  );
  // O fixture é gerado pelo script das fixtures: nada volátil dentro dele.
  const serialized = readRepoFile("contracts/fixtures/export_manifest_v1.json");
  assert.ok(!/timestamp|saved_at|host|generated_at/i.test(serialized));
});

test("contrato: manifesto fora do formato é erro explícito e recuperável", () => {
  const base = manifestFixture as unknown as Record<string, any>;

  assert.throws(() => readExportManifest(null), ExportManifestError);
  assert.throws(() => readExportManifest({ ...base, format_version: 2 }), /format_version 2/);
  assert.throws(() => readExportManifest({ ...base, generator: "outro-gerador" }), /generator/);

  const noVertex = structuredClone(base);
  delete noVertex.geometry.vertex_count;
  assert.throws(() => readExportManifest(noVertex), /vertex_count/);

  const badChecksum = structuredClone(base);
  badChecksum.geometry.base_vertex_checksum = "nem-hex";
  assert.throws(() => readExportManifest(badChecksum), /base_vertex_checksum/);

  const badStride = structuredClone(base);
  badStride.geometry.vertex_stride_bytes = 80;
  assert.throws(() => readExportManifest(badStride), /vertex_stride_bytes/);

  const badSkin = structuredClone(base);
  badSkin.skin.skinned_vertices = 1;
  assert.throws(() => readExportManifest(badSkin), /skin cobre/);

  const badGender = structuredClone(base);
  badGender.project.base_gender = "Other";
  assert.throws(() => readExportManifest(badGender), /base_gender/);
});

test("checksums: os bytes canônicos reconstruídos batem com o núcleo", () => {
  const manifest = readExportManifest(manifestFixture);
  const payload = snapshotFixture.static_payload;

  // 1. vértices: os bytes de 72 B que o núcleo empacota saem do payload estático
  const vertexBytes = new Uint8Array(Buffer.from(payload.vertex_buffer_base64, "base64"));
  assert.equal(vertexBytes.length, payload.vertex_count * EXPORT_VERTEX_STRIDE_BYTES);
  assert.equal(bufferChecksum(vertexBytes), manifest.geometry.base_vertex_checksum);

  // ... e reconstruídos dos floats (DataView little-endian) dão o mesmo valor
  const decoded = decodeCoreSnapshot(snapshotFixture);
  assert.ok(decoded.geometry);
  assert.equal(
    packedVertexChecksum(decoded.geometry!.packedVertices, decoded.geometry!.vertexCount),
    manifest.geometry.base_vertex_checksum,
    "a reconstrução dos bytes no viewport precisa reproduzir o checksum do núcleo"
  );
  assert.deepEqual(
    Array.from(packedVertexBytes(decoded.geometry!.packedVertices, decoded.geometry!.vertexCount)),
    Array.from(vertexBytes),
    "byte a byte: o layout do viewport é o layout do núcleo"
  );

  // 2. índices
  const indexRaw = new Uint8Array(Buffer.from(payload.index_buffer_base64, "base64"));
  assert.equal(bufferChecksum(indexRaw), manifest.geometry.base_index_checksum);
  assert.equal(indexChecksum(decoded.geometry!.indices), manifest.geometry.base_index_checksum);
  assert.deepEqual(Array.from(indexBytes(decoded.geometry!.indices)), Array.from(indexRaw));

  // 3. paleta de skinning
  assert.equal(
    paletteChecksum(decoded.geometry!.skin.palette),
    manifest.skin.palette_checksum
  );
  assert.equal(paletteBytes(decoded.geometry!.skin.palette).length, 24 * 16 * 4);

  // 4. identidade da geometria
  assert.equal(manifest.geometry.topology_hash, decoded.geometry!.topologyHash);
  assert.equal(manifest.geometry.catalog_fingerprint, decoded.geometry!.catalogFingerprint);
  assert.equal(manifest.geometry.mesh_uri, decoded.geometry!.meshUri);
});

test("paridade: o manifesto confere com o que o viewport desenha", () => {
  const geometry = viewportFixture();
  const report = checkExportParity({
    manifest: manifestFixture,
    geometry,
    staticRevision: manifestFixture.project.static_revision,
    authority: "core",
  });
  assert.deepEqual(report.problems, [], describeParity(report));
  assert.ok(report.ok);
  // WebGPU: a malha deformada é somada no compute shader — o relatório diz o que
  // ficou fora em vez de fingir que conferiu tudo.
  assert.equal(report.skipped.length, 1);
  assert.match(report.skipped[0], /compute shader/);
  assert.match(describeParity(report), /não conferível/);
  // os campos realmente comparados (não um "ok" vazio)
  for (const field of [
    "geometry.vertex_count",
    "geometry.index_count",
    "geometry.triangle_count",
    "geometry.vertex_stride_bytes",
    "geometry.topology_hash",
    "geometry.catalog_fingerprint",
    "geometry.mesh_uri",
    "geometry.base_vertex_checksum",
    "geometry.base_index_checksum",
    "deformation.morph_weights",
    "skin.bone_count",
    "skin.palette_checksum",
    "project.static_revision",
    "deformation.authority",
  ]) {
    assert.ok(report.checks.includes(field), `campo não conferido: ${field}`);
  }
});

test("paridade: a malha deformada do viewport é a malha exportada", () => {
  // Caminho canônico do WebGL2: `applyDeltasCpu` acumula os deltas **do núcleo**
  // com os pesos **do núcleo**. Se o viewport e a exportação divergissem, os
  // checksums de bytes denunciariam.
  const geometry = viewportFixture();
  const weights = new Map(manifestFixture.deformation.morph_weights.map((e) => [e.slider_id, e.weight]));
  const deformed = applyDeltasCpu(geometry, weights);
  assert.equal(
    packedVertexChecksum(deformed, geometry.vertexCount),
    manifestFixture.deformation.deformed_vertex_checksum
  );
  assert.equal(
    packedPositionChecksum(deformed, geometry.vertexCount),
    manifestFixture.deformation.deformed_position_checksum
  );

  const report = checkExportParity({
    manifest: manifestFixture,
    geometry,
    verifyDeformed: true,
  });
  assert.ok(report.ok, describeParity(report));
  assert.ok(report.checks.includes("deformation.deformed_vertex_checksum"));
  assert.ok(report.checks.includes("deformation.deformed_position_checksum"));
});

test("paridade: divergência real é detectada campo a campo", () => {
  const geometry = viewportFixture();
  const tampered = (patch: (manifest: Record<string, any>) => void): Record<string, any> => {
    const copy = structuredClone(manifestFixture) as unknown as Record<string, any>;
    patch(copy);
    return copy;
  };

  const cases: Array<[string, (manifest: Record<string, any>) => void]> = [
    [
      "vertex_count",
      (m) => {
        // mantém o manifesto autoconsistente (skin cobre os vértices) para que a
        // divergência detectada seja a do arquivo × viewport, não a do formato
        m.geometry.vertex_count += 1;
        m.skin.skinned_vertices += 1;
      },
    ],
    ["index_count", (m) => (m.geometry.index_count += 3)],
    ["topology_hash", (m) => (m.geometry.topology_hash = "0123456789abcdef")],
    ["catalog_fingerprint", (m) => (m.geometry.catalog_fingerprint = "ffffffffffffffff")],
    ["mesh_uri", (m) => (m.geometry.mesh_uri = "anigo://outra/malha")],
    ["base_vertex_checksum", (m) => (m.geometry.base_vertex_checksum = "0123456789abcdef")],
    ["base_index_checksum", (m) => (m.geometry.base_index_checksum = "0123456789abcdef")],
    ["palette_checksum", (m) => (m.skin.palette_checksum = "0123456789abcdef")],
    ["bone_count", (m) => (m.skin.bone_count = 12)],
  ];

  for (const [expectedCode, patch] of cases) {
    const report = checkExportParity({ manifest: tampered(patch), geometry });
    assert.equal(report.ok, false, `${expectedCode}: divergência não detectada`);
    assert.ok(
      report.problems.some((problem) => problem.code === expectedCode),
      `${expectedCode}: problema não reportado (${report.problems.map((p) => p.code).join(",")})`
    );
  }

  // peso de morph diferente do que o viewport desenha
  const weightDrift = checkExportParity({
    manifest: tampered((m) => {
      m.deformation.morph_weights[0].weight += 0.25;
    }),
    geometry,
  });
  assert.ok(weightDrift.problems.some((problem) => problem.code === "morph_weights"));

  // autoridade errada (alguém exportando de fora do núcleo)
  const wrongAuthority = checkExportParity({
    manifest: tampered((m) => {
      m.deformation.authority = "reference_ts";
    }),
    geometry,
  });
  assert.ok(wrongAuthority.problems.some((problem) => problem.code === "authority"));

  // revisão estática diferente da que o viewport recebeu
  const staleRevision = checkExportParity({
    manifest: manifestFixture,
    geometry,
    staticRevision: manifestFixture.project.static_revision + 1,
  });
  assert.ok(staleRevision.problems.some((problem) => problem.code === "static_revision"));

  // malha deformada divergente (caminho CPU)
  const deformedDrift = checkExportParity({
    manifest: tampered((m) => {
      m.deformation.deformed_position_checksum = "0123456789abcdef";
    }),
    geometry,
    verifyDeformed: true,
  });
  assert.ok(deformedDrift.problems.some((problem) => problem.code === "deformed_position_checksum"));

  // modo degradado: não existem dois lados para comparar
  const noGeometry = checkExportParity({ manifest: manifestFixture, geometry: null });
  assert.equal(noGeometry.ok, false);
  assert.equal(noGeometry.problems[0].code, "geometry_missing");
});

test("serviço: o GLB é pedido ao núcleo e o manifesto volta validado", async () => {
  const geometry = viewportFixture();
  const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
  const invoker = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    return {
      glb_path: "/tmp/canonical.glb",
      manifest_path: "/tmp/canonical.glb.manifest.json",
      glb_bytes: 123456,
      glb_checksum: "abcdef0123456789",
      manifest: manifestFixture,
    };
  };

  const result: ExportArtifactResult = await exportCanonicalGlb({
    invoker,
    path: "/tmp/canonical.glb",
    geometry,
    staticRevision: manifestFixture.project.static_revision,
    verifyDeformed: true,
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, EXPORT_COMMANDS.glb);
  assert.equal(calls[0].command, "core_export_glb");
  assert.deepEqual(calls[0].args, { path: "/tmp/canonical.glb" });
  assert.equal(result.glbBytes, 123456);
  assert.equal(result.glbChecksum, "abcdef0123456789");
  assert.equal(result.parity.ok, true, describeParity(result.parity));
  assert.match(describeParity(result.parity), /paridade ok \(\d+ campos conferidos, contrato v1\)/);

  // resposta torta do núcleo não vira sucesso silencioso
  await assert.rejects(
    () =>
      exportCanonicalGlb({
        invoker: async () => ({ manifest: manifestFixture }),
        path: "/tmp/x.glb",
        geometry,
      }),
    /glb_path/
  );
  await assert.rejects(
    () =>
      exportCanonicalGlb({
        invoker: async () => ({
          glb_path: "/tmp/x.glb",
          manifest_path: "/tmp/x.glb.manifest.json",
          glb_bytes: 1,
          glb_checksum: "abcdef0123456789",
          manifest: { format_version: 99 },
        }),
        path: "/tmp/x.glb",
        geometry,
      }),
    /format_version 99/
  );
  // sem núcleo (browser/dev server) o export não inventa um caminho paralelo
  await assert.rejects(
    () => exportCanonicalGlb({ invoker: null, path: "/tmp/x.glb", geometry }),
    /sem núcleo canônico/
  );
});

test("serviço: o frame de exportação traz o bloco de render do contrato", async () => {
  const geometry = viewportFixture();
  const render = {
    width: 1920,
    height: 1080,
    color_format: "Rgba8Unorm",
    depth_format: "Depth24Plus",
    msaa_samples: 4,
    render_passes: ["outline", "cel"],
    clear_source: "scene.background_color",
    clear_color: [0.08, 0.09, 0.13, 1],
    adapter_name: "test-adapter",
    backend: "Vulkan",
    render_time_ms: 1.25,
  };
  const invoker = async (command: string, args?: Record<string, unknown>) => {
    assert.equal(command, "core_export_frame");
    assert.deepEqual(args, { path: "/tmp/frame.png", width: 1920, height: 1080 });
    return {
      image_path: "/tmp/frame.png",
      manifest_path: "/tmp/frame.png.manifest.json",
      image_bytes: 4096,
      adapter_name: render.adapter_name,
      backend: render.backend,
      draw_calls: 2,
      triangle_count: 1,
      render_time_ms: render.render_time_ms,
      manifest: { ...manifestFixture, render },
    };
  };

  // A ordem dos passes que o núcleo grava no manifesto é a mesma que o viewport
  // desenha (fonte única: o render contract).
  assert.deepEqual(renderPassOrder(), ["outline", "cel"]);
  assert.deepEqual(render.render_passes, renderPassOrder());

  const result = await exportCanonicalFrame({
    invoker,
    path: "/tmp/frame.png",
    width: 1920,
    height: 1080,
    geometry,
    staticRevision: manifestFixture.project.static_revision,
  });
  assert.equal(result.imagePath, "/tmp/frame.png");
  assert.deepEqual(result.render.render_passes, ["outline", "cel"]);
  assert.equal(result.render.msaa_samples, 4);
  assert.ok(result.parity.ok, describeParity(result.parity));

  // frame sem bloco de render é resposta inválida, não sucesso
  await assert.rejects(
    () =>
      exportCanonicalFrame({
        invoker: async () => ({
          image_path: "/tmp/frame.png",
          manifest_path: "/tmp/frame.png.manifest.json",
          image_bytes: 1,
          manifest: manifestFixture,
        }),
        path: "/tmp/frame.png",
        width: 64,
        height: 64,
        geometry,
      }),
    /sem a seção de render/
  );
});

test("nome do artefato é determinístico (sem timestamp)", () => {
  assert.equal(
    exportFileName("Herói Anime.anigo", { staticRevision: 3, dynamicRevision: 7 }, "glb"),
    "heroi-anime-s3-d7.glb"
  );
  assert.equal(exportFileName("", { staticRevision: 0, dynamicRevision: 0 }, "png"), "anigo-s0-d0.png");
  assert.equal(
    exportFileName("Projeto", { staticRevision: 3, dynamicRevision: 7 }, "glb"),
    exportFileName("Projeto", { staticRevision: 3, dynamicRevision: 7 }, "glb")
  );
});

test("Rust: a exportação é do núcleo e usa o snapshot como fonte", () => {
  const exportRs = readRepoFile("crates/anigo-core/src/export.rs");
  const sessionRs = readRepoFile("src-tauri/src/core_session.rs");
  const mainRs = readRepoFile("src-tauri/src/main.rs");
  const libRs = readRepoFile("crates/anigo-core/src/lib.rs");

  assert.match(exportRs, /pub const EXPORT_FORMAT_VERSION: u32 = 1;/);
  assert.match(exportRs, /pub const EXPORT_GENERATOR: &str = "anigo-core\/export";/);
  assert.match(exportRs, /pub fn build_export_bundle\(/);
  assert.match(exportRs, /pub fn build_export_manifest\(/);
  assert.match(exportRs, /deformed_mesh\.to_glb_bytes\(\)/);
  assert.match(libRs, /pub mod export;/);

  // O GLB sai do mesmo writer glTF (com skin) e a paleta do snapshot
  assert.match(sessionRs, /pub fn export_bundle\(/);
  assert.match(sessionRs, /pub fn export_glb_to_file\(/);
  assert.match(sessionRs, /pub fn export_frame_manifest\(/);
  assert.match(sessionRs, /\.static_payload[\s\S]{0,120}?\.skin/);

  // Comandos registrados no app shell
  for (const handler of ["core_export_glb", "core_export_frame"]) {
    assert.ok(mainRs.includes(`async fn ${handler}(`), `${handler} precisa existir`);
    assert.ok(mainRs.includes(`            ${handler},`), `${handler} precisa estar no registry`);
  }
  // Issue #14: o manifesto registra o plano EXECUTADO (com o pré-passe quando
  // ligado), não a ordem estática do contrato.
  assert.match(mainRs, /render_passes: metrics\.executed_passes\.clone\(\)/);
  assert.match(mainRs, /msaa_samples: anigo_renderer::render_contract::msaa_sample_count\(\)/);
  assert.match(mainRs, /clear_source: anigo_renderer::render_contract::clear_color_source\(\)/);

  // Nenhuma segunda implementação de deformação nasceu por aqui
  assert.ok(!/apply_cpu\(/.test(mainRs), "o app shell não deforma: quem deforma é o CoreSession");
});
