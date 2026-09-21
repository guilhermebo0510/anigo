import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  exportVrmGlb,
  gltfJsonToGlb,
  parseGlbJson,
  validateVrmGlb,
  writeGlbJson,
  summarizeVrmJson,
  type VrmExportOptions,
} from "../../src/services/vrm_interop.ts";

function baseGlb(): ArrayBuffer {
  return writeGlbJson({
    json: {
      asset: { version: "2.0" }, scene: 0, scenes: [{ nodes: [0] }], nodes: [{ name: "Root" }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }], materials: [{}],
      accessors: [{ bufferView: 0, componentType: 5126, count: 1, type: "VEC3" }],
      bufferViews: [{ buffer: 0, byteLength: 12 }], buffers: [{ byteLength: 12 }],
    },
    bin: new Uint8Array(12),
  });
}
const options: VrmExportOptions = {
  meta: { title: "Aoi", version: "1.0.0", author: "ANIGO" },
  humanoid: [{ bone: "hips", node: 0 }, { bone: "spine", node: 0 }, { bone: "head", node: 0 }],
  expressions: [{ name: "happy", weight: 1 }],
};

describe("VRM 1.0 interoperability contract", () => {
  it("wraps a glTF 2.0 GLB without changing its binary chunk", () => {
    const source = baseGlb();
    const result = exportVrmGlb(source, options);
    const report = validateVrmGlb(result);
    assert.equal(report.valid, true);
    const parsedSource = parseGlbJson(source);
    const parsedResult = parseGlbJson(result);
    assert.deepEqual([...parsedResult.bin], [...parsedSource.bin]);
    const summary = summarizeVrmJson(parsedResult.json);
    assert.equal(summary.meta.title, "Aoi");
    assert.equal(summary.humanoidBones.length, 3);
    assert.equal(summary.expressions[0]?.name, "happy");
  });

  it("converts JSON glTF with an embedded buffer into the shared GLB path", () => {
    const json = {
      asset: { version: "2.0" }, scene: 0, scenes: [{ nodes: [0] }], nodes: [{ name: "Root" }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }], materials: [{}],
      accessors: [{ bufferView: 0, componentType: 5126, count: 1, type: "VEC3" }],
      bufferViews: [{ buffer: 0, byteLength: 4 }],
      buffers: [{ byteLength: 4, uri: "data:application/octet-stream;base64,AQIDBA==" }],
    };
    const glb = gltfJsonToGlb(JSON.stringify(json));
    const document = parseGlbJson(glb);
    assert.deepEqual([...document.bin], [1, 2, 3, 4]);
    assert.deepEqual(document.json.buffers, [{ byteLength: 4 }]);
  });

  it("reports malformed VRM data instead of accepting it", () => {
    const document = parseGlbJson(baseGlb());
    document.json.extensions = { VRMC_vrm: { specVersion: "0.0" } };
    const report = validateVrmGlb(writeGlbJson(document));
    assert.equal(report.valid, false);
    assert.ok(report.issues.some((entry) => entry.code === "VRM_SPEC_VERSION"));
  });
});
