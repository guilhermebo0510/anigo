/**
 * ANIGO contract test — Command v1 (Rust `command.rs` ⇄ TypeScript).
 *
 * The UI may only change persistent state by sending commands to the core
 * (ARQUITETURA_CANONICA_ANIGO §2.5). That makes the command vocabulary a wire
 * contract: a renamed field or an added variant on one side must fail here
 * instead of silently producing a command the core rejects (or, worse, a
 * command the UI believes it sent).
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  COMMAND_CONTRACT_VERSION,
  COMMAND_KINDS,
  isCommandWire,
  type CommandWire,
  type MaterialPatchWire,
} from "../../src/contracts/commands.v1.ts";
import {
  bracedBody,
  defaultedFields,
  diff,
  enumVariantFields,
  enumVariants,
  fieldsOf,
  pascalToSnake,
  readRepoFile,
  sorted,
  tsInterfaceFields,
  tsStringArrayConst,
  tsStringUnion,
} from "./rust_contract_source.ts";

const RUST_COMMAND = readRepoFile("crates/anigo-core/src/command.rs");
const TS_COMMAND = readRepoFile("src/contracts/commands.v1.ts");

/** Extracts `{ kind: "x"; field?: T; … }` members of the `CommandWire` union. */
function parseCommandWireMembers(source: string): Map<string, { fields: string[]; optional: string[] }> {
  const start = source.indexOf("export type CommandWire =");
  assert.ok(start !== -1, "CommandWire union not found");
  const end = source.indexOf("export const COMMAND_KINDS", start);
  assert.ok(end > start, "COMMAND_KINDS must follow the CommandWire union");
  const slice = source.slice(start, end);

  const members = new Map<string, { fields: string[]; optional: string[] }>();
  let depth = 0;
  let memberStart = -1;
  for (let i = 0; i < slice.length; i++) {
    const char = slice[i];
    if (char === "{") {
      depth++;
      if (depth === 1) memberStart = i;
    } else if (char === "}") {
      depth--;
      if (depth === 0 && memberStart !== -1) {
        const member = slice.slice(memberStart + 1, i);
        const kind = /kind:\s*"([^"]+)"/.exec(member)?.[1];
        assert.ok(kind, `union member without a kind: ${member.slice(0, 60)}`);
        const fields: string[] = [];
        const optional: string[] = [];
        for (const part of member.split(";")) {
          const match = /^\s*([a-zA-Z_][a-zA-Z0-9_]*)(\?)?\s*:/.exec(part);
          if (!match || match[1] === "kind") continue;
          fields.push(match[1]);
          if (match[2]) optional.push(match[1]);
        }
        assert.equal(members.has(kind), false, `duplicate command kind ${kind}`);
        members.set(kind, { fields, optional });
        memberStart = -1;
      }
    }
  }
  assert.ok(members.size > 0, "CommandWire union parsed to zero members");
  return members;
}

const wireMembers = parseCommandWireMembers(TS_COMMAND);

describe("Command v1 — vocabulary matches the Rust enum", () => {
  it("every Rust variant has exactly one snake_case kind, in the same order", () => {
    const rustKinds = enumVariants(RUST_COMMAND, "Command").map(pascalToSnake);
    assert.deepEqual(
      [...COMMAND_KINDS],
      rustKinds,
      `command kinds drifted: ${diff(rustKinds, [...COMMAND_KINDS])}`
    );
    assert.deepEqual([...wireMembers.keys()], rustKinds, "the union must cover every kind");
    assert.equal(COMMAND_KINDS.length, 19);
  });

  it("the snake_case array literal matches the exported tuple", () => {
    assert.deepEqual(tsStringArrayConst(TS_COMMAND, "COMMAND_KINDS"), [...COMMAND_KINDS]);
  });

  it("each command carries the same fields as its Rust variant", () => {
    const rustVariants = enumVariantFields(RUST_COMMAND, "Command");
    assert.equal(rustVariants.size, COMMAND_KINDS.length);
    for (const [variant, rustFields] of rustVariants) {
      const kind = pascalToSnake(variant);
      const member = wireMembers.get(kind);
      assert.ok(member, `CommandWire is missing ${kind}`);
      assert.deepEqual(
        sorted(member.fields),
        sorted(rustFields),
        `${kind} drifted: ${diff(rustFields, member.fields)}`
      );
    }
    // Unit variants exist on the Rust side and stay field-less here.
    assert.deepEqual(rustVariants.get("ResetMorphs"), []);
    assert.deepEqual(wireMembers.get("reset_morphs")?.fields, []);
  });

  it("optional wire fields are the `#[serde(default)]` fields in Rust", () => {
    const variants = ["SetProportions", "SetCamera", "SetLight", "SetRenderSettings", "SetMaterialParams", "SetNodeMesh"];
    for (const variant of variants) {
      const kind = pascalToSnake(variant);
      const member = wireMembers.get(kind);
      assert.ok(member);
      assert.deepEqual(
        sorted(member.optional),
        sorted(defaultedFields(RUST_COMMAND, "Command", variant)),
        `${kind}: optional fields must match the Rust serde defaults`
      );
    }
  });

  it("small enums used by commands are mirrored", () => {
    assert.deepEqual(tsStringUnion(TS_COMMAND, "ChangeScope"), [
      "base_geometry",
      "deformation",
      "shading",
      "camera",
      "presentation",
      "project",
    ]);
    assert.deepEqual(
      tsStringUnion(TS_COMMAND, "ChangeScope"),
      enumVariants(RUST_COMMAND, "ChangeScope").map(pascalToSnake)
    );
    assert.deepEqual(
      tsStringUnion(TS_COMMAND, "MeshPresetWire"),
      enumVariants(RUST_COMMAND, "MeshPreset").map(pascalToSnake)
    );
    assert.deepEqual(tsStringUnion(TS_COMMAND, "TonemapOperatorWire"), ["none", "reinhard", "neutral"]);
  });
});

describe("Command v1 — payload shapes", () => {
  it("MaterialPatch mirrors the Rust patch field for field", () => {
    const rustFields = fieldsOf(RUST_COMMAND, "MaterialPatch");
    const tsFields = tsInterfaceFields(TS_COMMAND, "MaterialPatchWire");
    assert.deepEqual(
      sorted(tsFields),
      sorted(rustFields),
      `MaterialPatchWire drifted: ${diff(rustFields, tsFields)}`
    );
    // The patch is the material vocabulary: keep it complete.
    // 42 = 21 baseline + 12 MToon (#18) + 3 SDF facial (#17) + 6 olho anime (#43).
    assert.equal(tsFields.length, 42);
    // Every field is optional in Rust (`Option<T>`) and the container itself is
    // `#[serde(default)]`, which is what makes the TS `?` fields safe to omit.
    assert.match(RUST_COMMAND, /#\[serde\(default\)\]\s*pub struct MaterialPatch/);
    const patchBody = bracedBody(RUST_COMMAND, "struct", "MaterialPatch");
    for (const field of rustFields) {
      assert.match(
        patchBody,
        new RegExp(`(?:pub\\s+)?${field}\\s*:\\s*Option<`),
        `${field} must stay an Option so a partial patch is valid`
      );
    }
  });

  it("CommandOutcome mirrors the Rust outcome struct", () => {
    const rustFields = fieldsOf(RUST_COMMAND, "CommandOutcome");
    const tsFields = tsInterfaceFields(TS_COMMAND, "CommandOutcomeWire");
    assert.deepEqual(
      sorted(tsFields),
      sorted(rustFields),
      `CommandOutcomeWire drifted: ${diff(rustFields, tsFields)}`
    );
    assert.equal(COMMAND_CONTRACT_VERSION, 1);
  });

  it("MeshRef keeps the asset reference shape", () => {
    const rustSource = readRepoFile("crates/anigo-core/src/project.rs");
    const rustFields = fieldsOf(rustSource, "MeshRef");
    const tsFields = tsInterfaceFields(TS_COMMAND, "MeshRefWire");
    assert.deepEqual(
      sorted(tsFields),
      sorted(rustFields),
      `MeshRefWire drifted: ${diff(rustFields, tsFields)}`
    );
    // `primitive_index` is defaulted on the Rust side, therefore optional here.
    assert.deepEqual(defaultedFields(rustSource, "MeshRef"), ["primitive_index"]);
  });

  it("the frozen wire example serializes byte-for-byte", () => {
    // Same string the Rust test asserts in crates/anigo-core/src/command.rs.
    const command: CommandWire = { kind: "set_morph_value", target: "mrf_head_width", value: 1.2 };
    assert.equal(JSON.stringify(command), '{"kind":"set_morph_value","target":"mrf_head_width","value":1.2}');
    const rustTest = /\{\\?"kind\\?":\\?"set_morph_value\\?".{0,80}\}/.exec(RUST_COMMAND);
    assert.ok(rustTest, "the Rust test module must keep the frozen wire example");
    assert.ok(
      rustTest[0].includes("mrf_head_width"),
      "the frozen example must use a stable morph id"
    );
  });

  it("the material patch example is accepted as a partial update", () => {
    const patch: MaterialPatchWire = { outline_width: 2.5, toon_steps: 3 };
    const command: CommandWire = { kind: "set_material_params", patch };
    assert.equal(JSON.stringify(command), '{"kind":"set_material_params","patch":{"outline_width":2.5,"toon_steps":3}}');
  });
});

describe("Command v1 — structural validation before sending", () => {
  it("accepts every kind and rejects unknown or malformed payloads", () => {
    for (const kind of COMMAND_KINDS) {
      assert.equal(isCommandWire({ kind }), true, `${kind} must be recognized`);
    }
    assert.equal(isCommandWire({ kind: "delete_everything" }), false);
    assert.equal(isCommandWire({}), false);
    assert.equal(isCommandWire(null), false);
    assert.equal(isCommandWire("set_morph_value"), false);
    assert.equal(isCommandWire([{ kind: "reset_morphs" }]), false);
  });

  it("nested batch commands are still commands", () => {
    const batch: CommandWire = {
      kind: "batch",
      commands: [
        { kind: "set_morph_value", target: "mrf_head_width", value: 1.1 },
        { kind: "reset_morphs" },
      ],
    };
    assert.equal(isCommandWire(batch), true);
    assert.equal(batch.commands.every(isCommandWire), true);
  });
});
