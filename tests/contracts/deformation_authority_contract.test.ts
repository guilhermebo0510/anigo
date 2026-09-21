/**
 * ANIGO contract test — deformation authority (ARQUITETURA §2.2/§4.1).
 *
 * Rule: only the Rust core deforms the canonical mesh. This test pins the gate
 * that decides who is allowed to, the degraded mode the client must announce,
 * and the fact that the policy versions reported by the payload come from the
 * core's own constants.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  DeformationAuthorityError,
  decideAuthority,
  describeAuthority,
  referenceFallbackAllowed,
  requireReferenceDeformation,
  type RuntimeDeformationAuthority,
} from "../../src/services/deformation_authority.ts";
import {
  coverageIsComplete,
  type DeformationCoverage,
} from "../../src/contracts/core_snapshot.v1.ts";
import { readdirSync, readFileSync } from "node:fs";
import {
  enumVariants,
  numericConst,
  pascalToSnake,
  readRepoFile,
} from "./rust_contract_source.ts";

const RUST_SNAPSHOT = readRepoFile("crates/anigo-core/src/snapshot.rs");
const RUST_DEFORMATION = readRepoFile("crates/anigo-core/src/deformation.rs");

const COMPLETE: DeformationCoverage = {
  total_sliders: 157,
  sliders_with_geometry: 157,
  morph_targets: 157,
  bakes_proportions: true,
  bakes_gender: true,
  somatotype_via_macro_sliders: true,
  proportion_policy_version: 1,
  somatotype_policy_version: 1,
};

const DEGRADED: DeformationCoverage = { ...COMPLETE, sliders_with_geometry: 120, bakes_gender: false };

describe("Deformation authority — wire vocabulary", () => {
  it("the authority values are the Rust enum variants", () => {
    assert.deepEqual(enumVariants(RUST_SNAPSHOT, "DeformationAuthority").map(pascalToSnake), [
      "core",
      "reference_ts",
    ]);
  });

  it("policy versions reported in the payload come from the core constants", () => {
    assert.equal(COMPLETE.proportion_policy_version, numericConst(RUST_DEFORMATION, "PROPORTION_POLICY_VERSION"));
    assert.equal(COMPLETE.somatotype_policy_version, numericConst(RUST_DEFORMATION, "SOMATOTYPE_POLICY_VERSION"));
    assert.equal(COMPLETE.proportion_policy_version, 1);
    assert.equal(COMPLETE.somatotype_policy_version, 1);
  });
});

describe("Deformation authority — who may deform", () => {
  it("grants authority to the core when the model is complete", () => {
    const decision = decideAuthority({ coreAvailable: true, coverage: COMPLETE, production: true });
    assert.equal(decision.authority, "core");
    assert.equal(decision.degraded, false);
    assert.equal(decision.coreAuthority, "core");
    assert.equal(coverageIsComplete(COMPLETE), true);
    // And the reference engine must refuse to run at all.
    assert.throws(
      () => requireReferenceDeformation(decision.authority, "applyAnatomicalDeformations"),
      (error: unknown) => {
        assert.ok(error instanceof DeformationAuthorityError);
        assert.equal(error.code, "DEFORMATION_AUTHORITY_VIOLATION");
        assert.match(error.message, /applyAnatomicalDeformations/);
        return true;
      }
    );
  });

  it("keeps the reference engine in development previews, marked as degraded", () => {
    const preview = decideAuthority({ coreAvailable: false, coverage: null, production: false });
    assert.equal(preview.authority, "reference_ts");
    assert.equal(preview.degraded, true);
    assert.match(preview.reason, /[Rr]eferência|referência/);
    requireReferenceDeformation(preview.authority, "webgpu_renderer.uploadSparseMorphData");
  });

  it("still marks an incomplete core model as degraded in development", () => {
    const decision = decideAuthority({ coreAvailable: true, coverage: DEGRADED, production: false });
    assert.equal(decision.authority, "reference_ts");
    assert.equal(decision.degraded, true);
    assert.equal(decision.coreAuthority, "reference_ts");
    assert.match(decision.reason, /120\/157/);
  });

  it("never falls back silently in production", () => {
    const noCore = decideAuthority({ coreAvailable: false, coverage: null, production: true });
    assert.equal(noCore.authority, "unavailable");
    assert.equal(noCore.degraded, true);
    assert.match(noCore.reason, /malha base canônica|canônica/);
    assert.throws(
      () => requireReferenceDeformation(noCore.authority, "applyAnatomicalDeformations"),
      DeformationAuthorityError
    );

    const incomplete = decideAuthority({ coreAvailable: true, coverage: DEGRADED, production: true });
    assert.equal(incomplete.authority, "unavailable");
    assert.equal(incomplete.degraded, true);
    assert.throws(() => requireReferenceDeformation(incomplete.authority, "anywhere"));
  });

  it("waits for the first snapshot instead of guessing", () => {
    const decision = decideAuthority({ coreAvailable: true, coverage: null, production: false });
    assert.equal(decision.authority, "reference_ts");
    assert.equal(decision.degraded, true);
    assert.match(decision.reason, /snapshot/i);
  });

  it("allows an explicit opt-in for the reference engine in production", () => {
    assert.equal(referenceFallbackAllowed({ production: true }), false);
    assert.equal(referenceFallbackAllowed({ production: true, allowReferenceFallback: true }), true);
    assert.equal(referenceFallbackAllowed({ production: false }), true);
    const decision = decideAuthority({
      coreAvailable: false,
      coverage: null,
      production: true,
      allowReferenceFallback: true,
    });
    assert.equal(decision.authority, "reference_ts");
    assert.equal(decision.degraded, true, "an explicit fallback is still not the canonical model");
  });

  it("reports diagnostics for the UI and telemetry", () => {
    const decision = decideAuthority({ coreAvailable: true, coverage: COMPLETE, production: true });
    const diagnostics = describeAuthority(decision, 42, COMPLETE);
    assert.equal(diagnostics.authority, "core");
    assert.equal(diagnostics.degraded, false);
    assert.equal(diagnostics.coreRevision, 42);
    assert.equal(diagnostics.coverage?.total_sliders, 157);
    assert.ok(diagnostics.reason.length > 0);
  });
});

describe("Deformation authority — the gate is the only way in", () => {
  it("only the reference authority passes the gate", () => {
    const allowed: RuntimeDeformationAuthority[] = ["reference_ts"];
    const blocked: RuntimeDeformationAuthority[] = ["core", "unavailable"];
    for (const authority of allowed) {
      requireReferenceDeformation(authority, "gate test");
    }
    for (const authority of blocked) {
      assert.throws(() => requireReferenceDeformation(authority, "gate test"), DeformationAuthorityError);
    }
  });

  it("the gate is the only door and production never imports the reference engine", () => {
    // The reference deformation is only reachable through the authority gate:
    // every call site must prove it went through `requireReferenceDeformation`.
    const source = readRepoFile("src/services/deformation_authority.ts");
    assert.match(source, /export function requireReferenceDeformation\(/);
    assert.match(source, /DEFORMATION_AUTHORITY_VIOLATION/);
    // The module owns the decision, so it must not import the reference engine.
    assert.equal(
      /from\s+"\.\.\/services\/morph_engine|from\s+"\.\/morph_engine/.test(source),
      false,
      "the authority module must not depend on the reference deformation engine"
    );

    // P0 §7.4: the reference engine left `src/**` altogether.
    const production = new URL("../../src/", import.meta.url);
    const offenders: string[] = [];
    const walk = (dir: URL) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const child = new URL(entry.name + (entry.isDirectory() ? "/" : ""), dir);
        if (entry.isDirectory()) walk(child);
        else if (/\.(ts|svelte)$/.test(entry.name)) {
          // Comentários podem citar o motor de referência; imports não podem.
          const code = readFileSync(child, "utf8")
            .split("\n")
            .filter((line) => !/^\s*(\/\/|\/\*|\*)/.test(line))
            .join("\n");
          const importsEngine = /from\s+["'][^"']*morph_engine/.test(code);
          const ownsDeformation = /applyAnatomicalDeformations|genericMorphDelta|recomputeNormals/.test(code);
          if (importsEngine || ownsDeformation) {
            offenders.push(child.pathname.replace(/.*\/src\//, "src/"));
          }
        }
      }
    };
    walk(production);
    assert.deepEqual(offenders, [], "production modules must not import the reference engine");
  });
});
