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
import { EXPLICIT_TS_MORPH_IDS } from "../reference/morph_engine.ts";

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

  it("the reference engine is test-only and every explicit id is canonical", () => {
    // P0 §7.4: no production module may import the reference engine, so the
    // hand-authored id set is documentation for the oracle — it must still be a
    // subset of the canonical catalog (the core covers the whole catalog).
    const catalogIds = new Set(CANONICAL_SLIDERS.map((s) => s.id));
    for (const id of EXPLICIT_TS_MORPH_IDS) assert.ok(catalogIds.has(id), id);

    const rendererSrc = fs.readFileSync(
      path.join(here, "..", "..", "src", "components", "viewport", "webgpu_renderer.ts"),
      "utf8"
    );
    assert.equal(
      /applyAnatomicalDeformations|genericMorphDelta|recomputeNormals/.test(rendererSrc),
      false,
      "the renderer must not own a deformation implementation (core snapshots only)"
    );
    assert.match(rendererSrc, /applyCoreSnapshot\(/);
  });
});
