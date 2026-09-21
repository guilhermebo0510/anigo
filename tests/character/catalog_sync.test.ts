/**
 * ANIGO P0-04 contract — TS catalog ≡ Rust catalog (ids, order, zones, ranges).
 * Parses crates/anigo-core/src/morph_catalog.rs textually (no toolchain needed).
 * Run: node --experimental-strip-types --test tests/character/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import { EXPLICIT_TS_MORPH_IDS } from "../../src/services/morph_engine.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const rustPath = path.join(here, "..", "..", "crates", "anigo-core", "src", "morph_catalog.rs");

interface RustDef {
  id: string;
  zone: string;
  min: number;
  def: number;
  max: number;
}

function parseRustCatalog(): RustDef[] {
  const text = fs.readFileSync(rustPath, "utf8");
  const defs: RustDef[] = [];
  const re =
    /MorphSliderDef\s*\{\s*id:\s*"([^"]+)"[^}]*?zone:\s*AnatomicalZone::(\w+)[^}]*?min:\s*([-\d.]+)[^}]*?default_value:\s*([-\d.]+)[^}]*?max:\s*([-\d.]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    defs.push({
      id: m[1],
      zone: m[2],
      min: Number(m[3]),
      def: Number(m[4]),
      max: Number(m[5]),
    });
  }
  return defs;
}

describe("P0-04 TS ≡ Rust catalog contract", () => {
  it("Rust catalog parses to 157 defs", () => {
    const rust = parseRustCatalog();
    assert.equal(rust.length, 157);
  });

  it("TS catalog holds 157 sliders in 18 zones", () => {
    assert.equal(CANONICAL_SLIDERS.length, 157);
    assert.equal(new Set(CANONICAL_SLIDERS.map((s) => s.zoneKey)).size, 18);
  });

  it("ids match in the same order", () => {
    const rust = parseRustCatalog();
    const diffs: string[] = [];
    for (let i = 0; i < 157; i++) {
      if (rust[i].id !== CANONICAL_SLIDERS[i].id) {
        diffs.push(`[${i}] rust=${rust[i].id} ts=${CANONICAL_SLIDERS[i].id}`);
      }
    }
    assert.deepEqual(diffs, []);
  });

  it("zones match for every slider", () => {
    const rust = parseRustCatalog();
    const diffs: string[] = [];
    for (let i = 0; i < 157; i++) {
      if (rust[i].zone !== CANONICAL_SLIDERS[i].zoneKey) {
        diffs.push(`${rust[i].id}: rust=${rust[i].zone} ts=${CANONICAL_SLIDERS[i].zoneKey}`);
      }
    }
    assert.deepEqual(diffs, []);
  });

  it("min/default/max match within float tolerance", () => {
    const rust = parseRustCatalog();
    const diffs: string[] = [];
    for (let i = 0; i < 157; i++) {
      const r = rust[i];
      const t = CANONICAL_SLIDERS[i];
      const close = (a: number, b: number) => Math.abs(a - b) < 1e-6;
      if (!close(r.min, t.min) || !close(r.def, t.defaultValue) || !close(r.max, t.max)) {
        diffs.push(`${r.id}: rust=[${r.min},${r.def},${r.max}] ts=[${t.min},${t.defaultValue},${t.max}]`);
      }
    }
    assert.deepEqual(diffs, []);
  });

  it("explicit TS morph set matches the renderer's 36 hand-authored ids", () => {
    // Guard against silent drift between morph_engine and webgpu_renderer:
    // the renderer test below scans the source for activeMorphWeights.get("…").
    const rendererSrc = fs.readFileSync(
      path.join(here, "..", "..", "src", "components", "viewport", "webgpu_renderer.ts"),
      "utf8"
    );
    const used = new Set<string>();
    const re = /activeMorphWeights\.get\("([a-z_0-9]+)"\)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(rendererSrc)) !== null) used.add(m[1]);
    assert.equal(used.size, EXPLICIT_TS_MORPH_IDS.size);
    const missing = [...used].filter((id) => !EXPLICIT_TS_MORPH_IDS.has(id));
    const extra = [...EXPLICIT_TS_MORPH_IDS].filter((id) => !used.has(id));
    assert.deepEqual([missing, extra], [[], []]);
    // And every explicit id exists in the catalog.
    const catalogIds = new Set(CANONICAL_SLIDERS.map((s) => s.id));
    for (const id of EXPLICIT_TS_MORPH_IDS) assert.ok(catalogIds.has(id), id);
  });
});
