/**
 * ANIGO contract test — Intent → Command (P0 undo/redo item 1).
 *
 * Every UI change becomes a command of the versioned contract, validated with
 * the same vocabulary the core uses (`CommandError`) *before* it is sent:
 *   - `unknown_target`   → slider/nó/material/light que não existe;
 *   - `invalid_value`    → valor fora da faixa, enum desconhecido, string vazia;
 *   - `non_finite`       → NaN/Infinity em qualquer campo numérico;
 *   - `no_op`            → comando que não mudaria nada;
 *   - `empty_batch`      → batch sem comandos.
 *
 * The catalog ranges come from Rust (`morph_catalog`), so a slider that the core
 * clamps is also clamped here — the UI cannot produce a command the core would
 * reject.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { COMMAND_KINDS, isCommandWire, type CommandWire } from "../../src/contracts/commands.v1.ts";
import { morphIdForSlider } from "../../src/contracts/project_state.v1.ts";
import {
  BUILT_COMMAND_KINDS,
  NO_OP_EPSILON,
  buildCommand,
  sliderOf,
  type CommandIntent,
} from "../../src/services/command_builder.ts";
import { assetIdForUri } from "../../src/services/project_persistence.ts";
import { CANONICAL_SLIDERS } from "../../src/services/morph_catalog.ts";
import { numericConst, readRepoFile } from "./rust_contract_source.ts";

const RUST_COMMAND = readRepoFile("crates/anigo-core/src/command.rs");
const RUST_CATALOG = readRepoFile("crates/anigo-core/src/morph_catalog.rs");

/** A slider whose range contains 1 — used by the fuzzy-value cases. */
const UNIT_SLIDER = CANONICAL_SLIDERS.find((slider) => slider.min <= 1 && slider.max >= 1);
assert.ok(UNIT_SLIDER, "o catálogo precisa ter pelo menos um slider em torno de 1");

function built(intent: CommandIntent): CommandWire {
  const result = buildCommand(intent);
  assert.equal(result.ok, true, `intent deveria virar comando: ${JSON.stringify(intent)}`);
  if (result.ok !== true) throw new Error("unreachable");
  return result.command;
}

function rejected(intent: CommandIntent, code: string): void {
  const result = buildCommand(intent);
  assert.equal(result.ok, false, `intent deveria ser recusada: ${JSON.stringify(intent)}`);
  if (result.ok === false) assert.equal(result.code, code, result.reason);
}

describe("Intent → Command — toda alteração da UI vira um comando do contrato", () => {
  it("cobre os 23 tipos de comando do contrato v1", () => {
    const intents: CommandIntent[] = [
      { kind: "morph", slider_id: UNIT_SLIDER.id, value: 1.05 },
      { kind: "reset_morphs", override_count: 3 },
      { kind: "base_gender", gender: "female" },
      { kind: "somatotype", endo: 0.4, meso: 0.35, ecto: 0.25 },
      { kind: "gender_dimorphism", value: 0.5 },
      { kind: "proportions", patch: { head_scale: 1.1 } },
      { kind: "camera", eye: [0, 1.5, 3] },
      { kind: "orbit", azimuth: 0.1, elevation: 0.2 },
      { kind: "zoom", factor: 1.2 },
      { kind: "pan", dx: 3, dy: -2 },
      { kind: "light", patch: { intensity: 1.4 } },
      { kind: "material", patch: { outline_width: 2 } },
      { kind: "node_visibility", node_id: "nod_character_base", visible: false },
      { kind: "node_mesh", node_id: "nod_character_base", mesh_uri: "anigo://preset/cube" },
      {
        kind: "add_node",
        node_id: "nod_jacket",
        name: "Jaqueta",
        index: 1,
        parent_id: "nod_character_base",
        node_kind: "clothing",
      },
      { kind: "remove_node", node_id: "nod_jacket" },
      { kind: "node_parent", node_id: "nod_jacket", parent_id: "nod_character_base" },
      {
        kind: "node_transform",
        node_id: "nod_jacket",
        transform: { translation: [0, 0.2, 0] },
      },
      { kind: "preset", preset: "sphere" },
      { kind: "background_color", color: [0.1, 0.2, 0.3, 1] },
      { kind: "render_settings", msaa_samples: 8 },
      { kind: "rename_project", name: "Estudo" },
      { kind: "batch", intents: [{ kind: "zoom", factor: 1.1 }, { kind: "render_settings", tonemap: "neutral" }] },
    ];

    const produced = intents.map((intent) => built(intent).kind);
    assert.deepEqual([...produced].sort(), [...COMMAND_KINDS].sort(), "todos os kinds precisam ser construíveis");
    assert.deepEqual(
      [...BUILT_COMMAND_KINDS].sort(),
      [...COMMAND_KINDS].sort(),
      "a lista exposta pelo builder é a do contrato"
    );
    for (const intent of intents) {
      assert.ok(isCommandWire(built(intent)), "o comando construído precisa passar no guard do contrato");
    }
  });

  it("morph: usa o id estável do slider e recusa alvo/valor/finito inválidos", () => {
    const slider = UNIT_SLIDER;
    const command = built({ kind: "morph", slider_id: slider.id, value: (slider.min + slider.max) / 2 });
    assert.deepEqual(command, {
      kind: "set_morph_value",
      target: morphIdForSlider(slider.id),
      value: (slider.min + slider.max) / 2,
    });

    rejected({ kind: "morph", slider_id: "nariz_do_dragao", value: 1 }, "unknown_target");
    rejected({ kind: "morph", slider_id: slider.id, value: slider.max + 1 }, "invalid_value");
    rejected({ kind: "morph", slider_id: slider.id, value: Number.NaN }, "non_finite");
    rejected({ kind: "morph", slider_id: slider.id, value: Number.POSITIVE_INFINITY }, "non_finite");
    // Valor idêntico ao atual nunca entra no histórico (mesma tolerância do core).
    rejected({ kind: "morph", slider_id: slider.id, value: 1, current_value: 1 + NO_OP_EPSILON / 2 }, "no_op");
    const moved = built({ kind: "morph", slider_id: slider.id, value: 1.2, current_value: 1 });
    assert.equal(moved.kind, "set_morph_value");

    // As faixas do catálogo TS são as mesmas do catálogo Rust (amostra).
    assert.ok(sliderOf(slider.id), "o catálogo precisa conter o slider");
    assert.equal(sliderOf("slider_inexistente"), undefined);
    assert.ok(RUST_CATALOG.includes(`"${slider.id}"`), "o slider existe no catálogo Rust");
  });

  it("reset_morphs só é enviado quando há algo para redefinir", () => {
    assert.deepEqual(built({ kind: "reset_morphs" }), { kind: "reset_morphs" });
    rejected({ kind: "reset_morphs", override_count: 0 }, "no_op");
  });

  it("gênero, somatótipo e dimorfismo", () => {
    assert.deepEqual(built({ kind: "base_gender", gender: "female" }), {
      kind: "set_base_gender",
      gender: "Female",
    });
    rejected({ kind: "base_gender", gender: "female", current_gender: "female" }, "no_op");

    assert.deepEqual(built({ kind: "somatotype", endo: 1, meso: 0, ecto: 0 }), {
      kind: "set_somatotype",
      endomorph: 1,
      mesomorph: 0,
      ectomorph: 0,
    });
    rejected({ kind: "somatotype", endo: 0, meso: 0, ecto: 0 }, "invalid_value");
    rejected({ kind: "somatotype", endo: 1.4, meso: 0, ecto: 0 }, "invalid_value");
    rejected({ kind: "somatotype", endo: 1, meso: 0, ecto: Number.NaN }, "non_finite");
    rejected(
      { kind: "somatotype", endo: 1, meso: 0, ecto: 0, current: { endo: 1, meso: 0, ecto: 0 } },
      "no_op"
    );

    assert.deepEqual(built({ kind: "gender_dimorphism", value: 0.25 }), {
      kind: "set_gender_dimorphism",
      value: 0.25,
    });
    rejected({ kind: "gender_dimorphism", value: 1.5 }, "invalid_value");
    rejected({ kind: "gender_dimorphism", value: 0.25, current_value: 0.25 }, "no_op");
  });

  it("proporções: só campos informados viram patch", () => {
    assert.deepEqual(built({ kind: "proportions", patch: { head_scale: 1.1, leg_length: 0.9 } }), {
      kind: "set_proportions",
      head_scale: 1.1,
      leg_length: 0.9,
    });
    rejected({ kind: "proportions", patch: {} }, "no_op");
    rejected({ kind: "proportions", patch: { head_ratio: Number.NaN } }, "non_finite");
  });

  it("câmera: patch vazio é no-op e vetores precisam ter 3 componentes", () => {
    assert.deepEqual(built({ kind: "camera", fov_degrees: 45 }), { kind: "set_camera", fov_degrees: 45 });
    assert.deepEqual(built({ kind: "camera", eye: [0.4, 1.6, 3.2] }), {
      kind: "set_camera",
      eye: [0.4, 1.6, 3.2],
    });
    rejected({ kind: "camera" }, "no_op");
    rejected({ kind: "camera", eye: [1, 2] as unknown as [number, number, number] }, "invalid_value");
    rejected({ kind: "camera", fov_degrees: 200 }, "invalid_value");

    assert.deepEqual(built({ kind: "orbit", azimuth: 0.3, elevation: -0.1 }), {
      kind: "orbit_camera",
      azimuth: 0.3,
      elevation: -0.1,
    });
    rejected({ kind: "orbit", azimuth: 0, elevation: 0 }, "no_op");

    assert.deepEqual(built({ kind: "zoom", factor: 1.25 }), { kind: "zoom_camera", factor: 1.25 });
    rejected({ kind: "zoom", factor: 0 }, "invalid_value");
    rejected({ kind: "zoom", factor: 1 }, "no_op");

    assert.deepEqual(built({ kind: "pan", dx: 4, dy: -2 }), { kind: "pan_camera", dx: 4, dy: -2 });
    rejected({ kind: "pan", dx: 0, dy: 0 }, "no_op");
  });

  it("iluminação e material: alvo precisa ser um id estável e patch vazio é no-op", () => {
    assert.deepEqual(built({ kind: "light", patch: { intensity: 1.4, ambient_sky: [0.1, 0.2, 0.3] } }), {
      kind: "set_light",
      intensity: 1.4,
      ambient_sky: [0.1, 0.2, 0.3],
    });
    assert.deepEqual(built({ kind: "light", patch: { light_id: "lgt_key", intensity: 2 } }), {
      kind: "set_light",
      intensity: 2,
      light_id: "lgt_key",
    });
    rejected({ kind: "light", patch: {} }, "no_op");
    rejected({ kind: "light", patch: { light_id: "key_light" } }, "unknown_target");
    rejected({ kind: "light", patch: { intensity: Number.NaN } }, "non_finite");

    assert.deepEqual(built({ kind: "material", patch: { outline_width: 2, toon_steps: 2 } }), {
      kind: "set_material_params",
      patch: { outline_width: 2, toon_steps: 2 },
    });
    assert.deepEqual(built({ kind: "material", material_id: "mat_default_anime", patch: { hue_shift: 10 } }), {
      kind: "set_material_params",
      material_id: "mat_default_anime",
      patch: { hue_shift: 10 },
    });
    rejected({ kind: "material", patch: {} }, "no_op");
    rejected({ kind: "material", material_id: "material 1", patch: { hue_shift: 1 } }, "unknown_target");
    rejected({ kind: "material", patch: { base_color: [1, 1, 1] as unknown as [number, number, number, number] } }, "invalid_value");
  });

  it("nós, presets e malhas usam ids/asset ids do contrato", () => {
    assert.deepEqual(built({ kind: "node_visibility", node_id: "nod_character_base", visible: false }), {
      kind: "set_node_visibility",
      node_id: "nod_character_base",
      visible: false,
    });
    rejected({ kind: "node_visibility", node_id: "node", visible: false }, "unknown_target");
    rejected(
      { kind: "node_visibility", node_id: "nod_character_base", visible: true, current_visible: true },
      "no_op"
    );

    assert.deepEqual(built({ kind: "node_mesh", node_id: "nod_character_base", mesh_uri: "anigo://preset/cube" }), {
      kind: "set_node_mesh",
      node_id: "nod_character_base",
      mesh: { asset_id: assetIdForUri("anigo://preset/cube") },
    });
    assert.deepEqual(built({ kind: "node_mesh", node_id: "nod_character_base", mesh_uri: null }), {
      kind: "set_node_mesh",
      node_id: "nod_character_base",
      mesh: null,
    });
    rejected({ kind: "node_mesh", node_id: "nod_character_base", mesh_uri: "" }, "invalid_value");

    assert.deepEqual(built({ kind: "preset", preset: "mannequin" }), { kind: "load_mesh_preset", preset: "mannequin" });
    rejected({ kind: "preset", preset: "esfera" as unknown as "sphere" }, "invalid_value");
  });

  it("apresentação: cor de fundo, MSAA/tonemap e nome do projeto", () => {
    assert.deepEqual(built({ kind: "background_color", color: [0.1, 0.2, 0.3, 1] }), {
      kind: "set_background_color",
      color: [0.1, 0.2, 0.3, 1],
    });
    rejected({ kind: "background_color", color: [2, 0, 0, 1] }, "invalid_value");
    rejected({ kind: "background_color", color: [0.1, 0.2, 0.3, 1], current_color: [0.1, 0.2, 0.3, 1] }, "no_op");

    assert.deepEqual(built({ kind: "render_settings", msaa_samples: 4, tonemap: "neutral" }), {
      kind: "set_render_settings",
      msaa_samples: 4,
      tonemap: "neutral",
    });
    rejected({ kind: "render_settings" }, "no_op");
    rejected({ kind: "render_settings", msaa_samples: 3 }, "invalid_value");
    // 1, 2, 4, 8, 16 são exatamente os valores aceitos pelo core.
    for (const samples of [1, 2, 4, 8, 16]) {
      assert.deepEqual(built({ kind: "render_settings", msaa_samples: samples }), {
        kind: "set_render_settings",
        msaa_samples: samples,
      });
    }
    assert.match(RUST_COMMAND, /matches!\(samples, 1 \| 2 \| 4 \| 8 \| 16\)/, "o core aceita os mesmos MSAA");

    assert.deepEqual(built({ kind: "rename_project", name: "  Anigo  " }), { kind: "rename_project", name: "  Anigo  " });
    rejected({ kind: "rename_project", name: "   " }, "invalid_value");
    rejected({ kind: "rename_project", name: "Anigo", current_name: "Anigo" }, "no_op");
  });

  it("batch é atômico: um filho inválido invalida o batch inteiro", () => {
    assert.deepEqual(
      built({
        kind: "batch",
        intents: [
          { kind: "morph", slider_id: UNIT_SLIDER.id, value: 1.1 },
          { kind: "background_color", color: [0, 0, 0, 1] },
        ],
      }),
      {
        kind: "batch",
        commands: [
          { kind: "set_morph_value", target: morphIdForSlider(UNIT_SLIDER.id), value: 1.1 },
          { kind: "set_background_color", color: [0, 0, 0, 1] },
        ],
      }
    );
    rejected({ kind: "batch", intents: [] }, "empty_batch");
    // Nenhum comando parcial escapa: o batch só existe se todos os filhos valem.
    rejected(
      {
        kind: "batch",
        intents: [
          { kind: "zoom", factor: 1.2 },
          { kind: "morph", slider_id: "mrf_x", value: 1 },
        ],
      },
      "unknown_target"
    );
  });

  it("a tolerância de no-op e o catálogo espelham o Rust", () => {
    assert.equal(NO_OP_EPSILON, 1e-6, "mesma epsilon do core");
    // O catálogo Rust é um array de tamanho fixo; o TS precisa ter o mesmo total.
    const rustCount = /pub const ALL_MORPH_SLIDERS:\s*\[MorphSliderDef;\s*(\d+)\]/.exec(RUST_CATALOG);
    assert.ok(rustCount, "ALL_MORPH_SLIDERS (Rust) precisa declarar o tamanho");
    assert.equal(CANONICAL_SLIDERS.length, Number(rustCount?.[1]), "mesmo número de sliders dos dois lados");
    assert.ok(CANONICAL_SLIDERS.length > 100, "catálogo canônico carregado");
    assert.ok(
      CANONICAL_SLIDERS.every((slider) => morphIdForSlider(slider.id).startsWith("mrf_")),
      "todo slider gera um id estável de morph"
    );
  });
});
