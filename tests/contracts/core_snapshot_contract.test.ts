/**
 * ANIGO contract test — CoreSnapshot v1 (Rust core ⇄ TypeScript renderer).
 *
 * Two independent obligations are checked here:
 *  1. the TypeScript decoder reproduces `contracts/fixtures/core_snapshot_v1.json`,
 *     which is the same fixture the Rust test module decodes;
 *  2. the TypeScript contract types and constants still match the Rust source of
 *     truth (parsed textually, so this runs without a Rust toolchain).
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  MORPH_CHANNEL_STRIDE_BYTES,
  MORPH_DELTA_STRIDE_BYTES,
  MORPH_WEIGHT_EPSILON,
  SNAPSHOT_FORMAT_VERSION,
  SnapshotContractError,
  VERTEX_STRIDE_BYTES,
  authorityFor,
  coverageIsComplete,
  coverageRatio,
  decodeCoreSnapshot,
  decodeStaticGeometry,
  missingSliders,
  type CoreSnapshotWire,
} from "../../src/contracts/core_snapshot.v1.ts";
import {
  bracedBody,
  diff,
  enumVariants,
  fieldsOf,
  numericConst,
  pascalToSnake,
  readRepoFile,
  sorted,
  tsInterfaceFields,
} from "./rust_contract_source.ts";

const RUST_SNAPSHOT = readRepoFile("crates/anigo-core/src/snapshot.rs");
const RUST_MESH = readRepoFile("crates/anigo-core/src/mesh.rs");
const RUST_PROJECT = readRepoFile("crates/anigo-core/src/project.rs");
const FIXTURE = JSON.parse(readRepoFile("contracts/fixtures/core_snapshot_v1.json")) as CoreSnapshotWire;

function clone(): CoreSnapshotWire {
  return JSON.parse(JSON.stringify(FIXTURE)) as CoreSnapshotWire;
}

describe("CoreSnapshot v1 — constants match the Rust contract", () => {
  it("format version and binary strides are identical", () => {
    assert.equal(SNAPSHOT_FORMAT_VERSION, numericConst(RUST_SNAPSHOT, "SNAPSHOT_FORMAT_VERSION"));
    assert.equal(VERTEX_STRIDE_BYTES, numericConst(RUST_SNAPSHOT, "VERTEX_STRIDE_BYTES"));
    assert.equal(MORPH_DELTA_STRIDE_BYTES, numericConst(RUST_SNAPSHOT, "MORPH_DELTA_STRIDE_BYTES"));
    assert.equal(
      MORPH_CHANNEL_STRIDE_BYTES,
      numericConst(RUST_SNAPSHOT, "MORPH_CHANNEL_STRIDE_BYTES")
    );
    // The 72-byte vertex layout is stated in both crates and in the shader.
    assert.equal(VERTEX_STRIDE_BYTES, 72);
    assert.equal(MORPH_DELTA_STRIDE_BYTES, 32);
    assert.equal(MORPH_CHANNEL_STRIDE_BYTES, 16);
    // Sparse threshold documented as 1e-6 on the Rust side.
    assert.equal(MORPH_WEIGHT_EPSILON, 1e-6);
  });

  it("snapshot payload structs expose the same fields on both sides", () => {
    // [Rust struct, TypeScript type] — the wire names differ only by the
    // `Wire` suffix, except for the coverage report which is a shared type.
    const pairs: Array<[string, string]> = [
      ["CoreSnapshot", "CoreSnapshotWire"],
      ["StaticGeometryPayload", "StaticGeometryPayloadWire"],
      ["DynamicStatePayload", "DynamicStatePayloadWire"],
      ["MorphChannelDescriptor", "MorphChannelDescriptorWire"],
      ["MorphWeight", "MorphWeightWire"],
      ["CameraSnapshot", "CameraSnapshotWire"],
      ["LightSnapshot", "LightSnapshotWire"],
      ["MaterialSnapshot", "MaterialSnapshotWire"],
      ["NodeSnapshot", "NodeSnapshotWire"],
      ["RenderSnapshot", "RenderSnapshotWire"],
      ["DeformationCoverage", "DeformationCoverage"],
    ];
    const tsSource = readRepoFile("src/contracts/core_snapshot.v1.ts");
    for (const [rustName, tsName] of pairs) {
      const rustFields = fieldsOf(RUST_SNAPSHOT, rustName);
      const tsFields = tsInterfaceFields(tsSource, tsName);
      assert.deepEqual(
        sorted(tsFields),
        sorted(rustFields),
        `${tsName} drifted from Rust ${rustName}: ${diff(rustFields, tsFields)}`
      );
      if (tsName.endsWith("Wire")) {
        assert.equal(
          tsName,
          `${rustName}Wire`,
          "wire type names must keep the frozen `Wire` convention"
        );
      }
    }
  });

  it("nested wire structs are mirrored too", () => {
    const tsSource = readRepoFile("src/contracts/core_snapshot.v1.ts");
    assert.deepEqual(
      sorted(tsInterfaceFields(tsSource, "ColorManagementWire")),
      sorted(fieldsOf(RUST_PROJECT, "ColorManagement")),
      "ColorManagementWire drifted from Rust ColorManagement"
    );
  });

  it("coverage completeness rule is the same predicate on both sides", () => {
    const rustBody = bracedBody(RUST_SNAPSHOT, "fn", "is_complete");
    const rustFlags = ["bakes_proportions", "bakes_gender", "somatotype_via_macro_sliders"].filter(
      (flag) => rustBody.includes(flag)
    );
    assert.equal(rustFlags.length, 3, "Rust is_complete must require the three bake flags");
    assert.ok(rustBody.includes("total_sliders"), "Rust is_complete must require a non-empty catalog");
    assert.ok(
      rustBody.includes("sliders_with_geometry"),
      "Rust is_complete must compare covered vs total sliders"
    );

    const tsSource = readRepoFile("src/contracts/core_snapshot.v1.ts");
    const tsBody = bracedBody(tsSource, "function", "coverageIsComplete");
    for (const flag of rustFlags) {
      assert.ok(tsBody.includes(flag), `coverageIsComplete is missing '${flag}'`);
    }
    assert.ok(tsBody.includes("total_sliders"));
    assert.ok(tsBody.includes("sliders_with_geometry"));

    // Behavioural equivalence on the boundary cases.
    const complete = {
      total_sliders: 157,
      sliders_with_geometry: 157,
      morph_targets: 157,
      bakes_proportions: true,
      bakes_gender: true,
      somatotype_via_macro_sliders: true,
      proportion_policy_version: 1,
      somatotype_policy_version: 1,
    };
    assert.equal(coverageIsComplete(complete), true);
    assert.equal(missingSliders(complete), 0);
    assert.equal(coverageRatio(complete), 1);
    assert.equal(authorityFor(complete), "core");

    for (const flag of ["bakes_proportions", "bakes_gender", "somatotype_via_macro_sliders"] as const) {
      assert.equal(coverageIsComplete({ ...complete, [flag]: false }), false, `${flag} must gate`);
    }
    assert.equal(coverageIsComplete({ ...complete, sliders_with_geometry: 156 }), false);
    assert.equal(authorityFor({ ...complete, sliders_with_geometry: 156 }), "reference_ts");
    assert.equal(missingSliders({ ...complete, sliders_with_geometry: 150 }), 7);
  });

  it("authority wire values are the snake_case variants of the Rust enum", () => {
    const variants = enumVariants(RUST_SNAPSHOT, "DeformationAuthority").map(pascalToSnake);
    assert.deepEqual(variants, ["core", "reference_ts"]);
    const tsSource = readRepoFile("src/contracts/core_snapshot.v1.ts");
    assert.ok(tsSource.includes('export type DeformationAuthority = "core" | "reference_ts";'));
  });

  it("base gender wire values match BaseGender", () => {
    assert.deepEqual(enumVariants(RUST_MESH, "BaseGender"), ["Male", "Female"]);
    assert.equal(FIXTURE.static_payload?.base_gender, "Male");
  });

  it("color management enums match ColorSpace and TonemapOperator", () => {
    const tsSource = readRepoFile("src/contracts/core_snapshot.v1.ts");
    assert.deepEqual(enumVariants(RUST_PROJECT, "ColorSpace").map(pascalToSnake), [
      "srgb",
      "linear_srgb",
      "display_p3",
    ]);
    assert.deepEqual(enumVariants(RUST_PROJECT, "TonemapOperator").map(pascalToSnake), [
      "none",
      "reinhard",
      "neutral",
    ]);
    assert.ok(tsSource.includes('"srgb" | "linear_srgb" | "display_p3"'));
    assert.ok(tsSource.includes('"none" | "reinhard" | "neutral"'));
  });
});

describe("CoreSnapshot v1 — the fixture decodes to the documented values", () => {
  it("decodes geometry, channels and weights", () => {
    const decoded = decodeCoreSnapshot(clone());
    assert.equal(decoded.snapshotVersion, SNAPSHOT_FORMAT_VERSION);
    assert.equal(decoded.dynamicRevision, 7);
    assert.equal(decoded.staticRevision, 3);

    const geometry = decoded.geometry;
    assert.ok(geometry, "the fixture carries a static payload");
    assert.equal(geometry.vertexCount, FIXTURE.expected_vertex_count);
    assert.equal(geometry.indexCount, FIXTURE.expected_index_count);
    assert.equal(geometry.vertexStrideBytes, VERTEX_STRIDE_BYTES);
    assert.equal(geometry.channels.length, FIXTURE.expected_channel_count);
    assert.equal(geometry.totalDeltas, FIXTURE.expected_total_deltas);

    // Vertex 0 keeps the position it was packed with (f32 decode).
    const first = FIXTURE.expected_first_position;
    const second = FIXTURE.expected_second_position;
    for (let axis = 0; axis < 3; axis++) {
      assert.ok(
        Math.abs(geometry.positions[axis] - first[axis]) < 1e-6,
        `vertex 0 axis ${axis}: expected ${first[axis]}, decoded ${geometry.positions[axis]}`
      );
      assert.ok(
        Math.abs(geometry.positions[3 + axis] - second[axis]) < 1e-6,
        `vertex 1 axis ${axis}: expected ${second[axis]}, decoded ${geometry.positions[3 + axis]}`
      );
    }
    assert.deepEqual(Array.from(geometry.indices), [0, 1, 2]);

    // Deltas are sparse and indexed by the vertex they move. f32 rounding is
    // expected: the fixture states decimal values, the buffer holds f32.
    const [dx, dy, dz] = FIXTURE.expected_first_delta_position;
    assert.equal(geometry.deltaWords[0], FIXTURE.expected_first_delta_vertex_index);
    const deltas = [geometry.deltas[1], geometry.deltas[2], geometry.deltas[3]];
    [dx, dy, dz].forEach((expected, axis) => {
      assert.ok(
        Math.abs(deltas[axis] - expected) < 1e-6,
        `delta axis ${axis}: expected ${expected}, decoded ${deltas[axis]}`
      );
    });

    // Channels are contiguous and ordered (the shader binary-searches them).
    assert.equal(geometry.channels[0].start_offset, 0);
    assert.equal(geometry.channels[1].start_offset, geometry.channels[0].delta_count);
    assert.equal(geometry.topologyHash, FIXTURE.static_payload?.topology_hash);
    assert.equal(geometry.catalogFingerprint, FIXTURE.static_payload?.catalog_fingerprint);
  });

  it("decodes weights sparsely and reports the degraded authority", () => {
    const decoded = decodeCoreSnapshot(clone());
    assert.equal(decoded.morphWeights.size, FIXTURE.dynamic.morph_weights.length);
    assert.equal(decoded.morphWeights.get("head_width"), 0.2);
    assert.equal(decoded.morphValues.get("head_width"), 1.2);
    assert.equal(decoded.morphWeights.get("jaw_v_line_taper"), 0.1);
    assert.equal(decoded.authority, "reference_ts");
    assert.equal(decoded.coverage.total_sliders, 157);
    assert.equal(decoded.coverage.bakes_gender, false);
    assert.equal(coverageIsComplete(decoded.coverage), false);
  });

  it("skips non-finite weights instead of poisoning the GPU buffer", () => {
    const snapshot = clone();
    snapshot.dynamic.morph_weights[0].weight = Number.NaN;
    const decoded = decodeCoreSnapshot(snapshot);
    assert.equal(decoded.morphWeights.has("head_width"), false);
    assert.equal(decoded.morphValues.has("head_width"), false);
    assert.equal(decoded.morphWeights.get("jaw_v_line_taper"), 0.1);
  });
});

describe("CoreSnapshot v1 — malformed payloads fail loudly", () => {
  function expectCode(code: string, run: () => unknown) {
    assert.throws(run, (error: unknown) => {
      assert.ok(error instanceof SnapshotContractError, `expected SnapshotContractError, got ${error}`);
      assert.equal(error.code, code);
      return true;
    });
  }

  it("rejects an unsupported snapshot version", () => {
    const snapshot = clone();
    snapshot.snapshot_version = 2;
    expectCode("UNSUPPORTED_VERSION", () => decodeCoreSnapshot(snapshot));
  });

  it("rejects a static payload from another format version", () => {
    const snapshot = clone();
    snapshot.static_payload!.format_version = 99;
    expectCode("UNSUPPORTED_VERSION", () => decodeStaticGeometry(snapshot.static_payload!));
  });

  it("rejects a wrong vertex stride", () => {
    const snapshot = clone();
    snapshot.static_payload!.vertex_stride_bytes = 64;
    expectCode("BAD_VERTEX_STRIDE", () => decodeStaticGeometry(snapshot.static_payload!));
  });

  it("rejects a buffer that is not a multiple of the stride", () => {
    const snapshot = clone();
    snapshot.static_payload!.morph_deltas_base64 = Buffer.from([1, 2, 3, 4, 5]).toString("base64");
    expectCode("BAD_STRIDE", () => decodeStaticGeometry(snapshot.static_payload!));
  });

  it("rejects header/buffer count mismatches", () => {
    const vertexMismatch = clone();
    vertexMismatch.static_payload!.vertex_count = 4;
    expectCode("VERTEX_COUNT_MISMATCH", () => decodeStaticGeometry(vertexMismatch.static_payload!));

    const indexMismatch = clone();
    indexMismatch.static_payload!.index_count = 9;
    expectCode("INDEX_COUNT_MISMATCH", () => decodeStaticGeometry(indexMismatch.static_payload!));

    const deltaMismatch = clone();
    deltaMismatch.static_payload!.morph_total_deltas = 2;
    expectCode("DELTA_COUNT_MISMATCH", () => decodeStaticGeometry(deltaMismatch.static_payload!));
  });

  it("rejects a channel table that is not contiguous", () => {
    const snapshot = clone();
    snapshot.static_payload!.morph_channels[1].start_offset = 5;
    expectCode("NON_CONTIGUOUS_CHANNELS", () => decodeStaticGeometry(snapshot.static_payload!));
  });

  it("rejects a channel that runs past the delta buffer", () => {
    const snapshot = clone();
    snapshot.static_payload!.morph_channels[1].delta_count = 99;
    snapshot.static_payload!.morph_channels[1].start_offset = 2;
    expectCode("CHANNEL_OUT_OF_RANGE", () => decodeStaticGeometry(snapshot.static_payload!));
  });

  it("accepts a snapshot without a static payload (dynamic-only refresh)", () => {
    const snapshot = clone();
    snapshot.static_payload = null;
    const decoded = decodeCoreSnapshot(snapshot);
    assert.equal(decoded.geometry, null);
    assert.equal(decoded.dynamicRevision, 7);
    assert.equal(decoded.morphWeights.size, 2);
  });
});
