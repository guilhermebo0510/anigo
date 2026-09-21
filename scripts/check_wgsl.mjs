#!/usr/bin/env node
/**
 * ANIGO — verificador de shader ⇄ contrato (`npm run check:wgsl`).
 *
 * O ambiente de desenvolvimento não tem GPU nem naga, então um erro de layout
 * de buffer ou um `@binding` trocado só apareceria ao abrir o app. Este script
 * fecha esse buraco comparando o que os shaders **declaram** com o que o render
 * contract **congela**:
 *
 *   1. layout WGSL de cada `struct` (alinhamento/offset/tamanho, com o
 *      arredondamento de 16 B do espaço `uniform`) contra `uniforms.<bloco>`;
 *   2. o conjunto de `@group(0) @binding(N)` de cada shader contra o bind group
 *      do passe que usa esse shader;
 *   3. entry points declarados;
 *   4. balanceamento de chaves/parênteses (pega edição truncada);
 *   5. o bloco de skinning precisa ser **byte-idêntico** entre os shaders que o
 *      usam (marcadores no contrato) — uma definição, vários usos;
 *   6. fallback GLSL: `#version 300 es`, paleta declarada e de fato usada.
 *
 * Erro aqui é drift de contrato, não estilo: o script sai com código != 0.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contractPath = path.join(root, "contracts", "fixtures", "render_contract_v1.json");

/** Tamanho/alinhamento em bytes dos tipos WGSL usados pelos shaders do ANIGO. */
export function wgslTypeSizeAlign(type) {
  const text = type.trim();
  const vector = /^vec([234])<([fiu])(32)>$/.exec(text);
  if (vector) {
    const [, width, , bits] = vector;
    const scalar = Number(bits) / 8;
    const components = Number(width);
    const align = components === 2 ? scalar * 2 : scalar * 4;
    return { align, size: scalar * components };
  }
  if (text === "f32" || text === "u32" || text === "i32") return { align: 4, size: 4 };
  const matrix = /^mat([234])x([234])<f32>$/.exec(text);
  if (matrix) {
    const columns = Number(matrix[1]);
    const rows = Number(matrix[2]);
    // SizeOf(matCxR<T>) = SizeOf(vecR<T>) * C; AlignOf = AlignOf(vecR<T>).
    const rowVector = { 2: { align: 8, size: 8 }, 3: { align: 16, size: 12 }, 4: { align: 16, size: 16 } }[rows];
    if (!rowVector) {
      throw new Error(`mat${matrix[1]}x${matrix[2]}<f32> não é válido em WGSL (linhas 2..4)`);
    }
    return { align: rowVector.align, size: rowVector.size * columns };
  }
  const array = /^array<(.+?)(?:,\s*(\d+))?>$/.exec(text);
  if (array) {
    const element = wgslTypeSizeAlign(array[1]);
    if (array[2] === undefined) {
      return { align: element.align, size: element.size, runtimeSized: true };
    }
    const count = Number(array[2]);
    const stride = roundUp(element.align, element.size);
    return { align: element.align, size: stride * (count - 1) + element.size };
  }
  throw new Error(`tipo WGSL não suportado pelo verificador: '${text}'`);
}

function roundUp(alignment, value) {
  if (alignment <= 0) return value;
  return Math.ceil(value / alignment) * alignment;
}

/**
 * Divide os membros de uma struct por vírgula **respeitando aninhamento**:
 * `array<mat4x4<f32>, 24>` tem uma vírgula dentro dos `<>` e não pode ser
 * partido em dois pedaços.
 */
export function splitWgslMembers(body) {
  const members = [];
  let depth = 0;
  let current = "";
  for (const char of body) {
    if (char === "<" || char === "(" || char === "[") depth += 1;
    else if (char === ">" || char === ")" || char === "]") depth -= 1;
    if (char === "," && depth === 0) {
      members.push(current);
      current = "";
      continue;
    }
    current += char;
  }
  members.push(current);
  return members;
}

/** Extrai `struct Nome { ... }` do fonte WGSL. */
export function parseWgslStructs(source) {
  const structs = new Map();
  const pattern = /struct\s+([A-Za-z_]\w*)\s*\{([^}]*)\}\s*;/g;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    const members = [];
    // Comentários saem **antes** da divisão por vírgula: um comentário de
    // membro pode conter vírgulas (`// r: AO, g: shadow shift, ...`).
    const body = match[2].replace(/\/\/[^\n]*/g, "");
    for (const rawMember of splitWgslMembers(body)) {
      const clean = rawMember.trim();
      if (clean.length === 0) continue;
      const attributes = {};
      let rest = clean;
      const attributePattern = /@(align|size|location|builtin|interpolate)\((?:"?[\w.]+"?|[\d.]+)\)/g;
      for (const attribute of rest.matchAll(attributePattern)) {
        const [key, value] = /@(\w+)\((.+)\)/.exec(attribute[0]).slice(1);
        attributes[key] = /^\d+$/.test(value) ? Number(value) : value.replace(/"/g, "");
      }
      rest = rest.replace(attributePattern, "").trim();
      const typeMatch = /^([A-Za-z_][\w<>,\s]*?)\s*:\s*(.+)$/.exec(rest);
      if (!typeMatch) throw new Error(`membro WGSL não reconhecido: '${clean}'`);
      members.push({
        name: typeMatch[1],
        type: typeMatch[2].trim(),
        align: attributes.align,
        sizeAttribute: attributes.size,
        location: attributes.location,
      });
    }
    structs.set(name, members);
  }
  return structs;
}

/** Layout de uma struct WGSL (offsets absolutos, alinhamento e tamanho). */
export function layoutWgslStruct(members, structs, addressSpace = "storage_read") {
  const layout = [];
  let cursor = 0;
  let structAlign = 1;
  for (const member of members) {
    let { align, size } = wgslTypeSizeAlign(member.type);
    // `array<T>` (runtime-sized) não é membro de struct; se aparecer, é o
    // elemento que interessa (usado em `var<storage> x: array<T>`).
    if (member.align !== undefined) align = member.align;
    if (member.sizeAttribute !== undefined) size = member.sizeAttribute;
    structAlign = Math.max(structAlign, align);
    const offset = roundUp(align, cursor);
    layout.push({ name: member.name, type: member.type, offset, size, align });
    cursor = offset + size;
  }
  if (addressSpace === "uniform") structAlign = roundUp(16, structAlign);
  return { fields: layout, align: structAlign, size: roundUp(structAlign, cursor) };
}

/** Declarações `@group(g) @binding(b) var<...> nome: tipo;` de um shader WGSL. */
export function parseWgslBindings(source) {
  const bindings = [];
  const pattern =
    /@group\((\d+)\)\s*@binding\((\d+)\)\s*var(?:<([^>]*)>)?\s+([A-Za-z_]\w*)\s*:\s*([^;]+);/g;
  for (const match of source.matchAll(pattern)) {
    bindings.push({
      group: Number(match[1]),
      binding: Number(match[2]),
      addressSpace: (match[3] ?? "").trim(),
      name: match[4],
      type: match[5].trim(),
    });
  }
  return bindings;
}

function balanced(source, file, problems) {
  const pairs = [
    ["{", "}"],
    ["(", ")"],
    ["[", "]"],
  ];
  for (const [open, close] of pairs) {
    let depth = 0;
    for (const char of source) {
      if (char === open) depth += 1;
      else if (char === close) depth -= 1;
      if (depth < 0) break;
    }
    if (depth !== 0) {
      problems.push(`${file}: '${open}${close}' desbalanceado (diferença ${depth})`);
    }
  }
}

function extractBlock(source, begin, end, file, problems) {
  const start = source.indexOf(begin);
  const stop = source.indexOf(end);
  if (start < 0 || stop < 0 || stop < start) {
    problems.push(`${file}: bloco de skinning sem os marcadores '${begin}'/'${end}'`);
    return null;
  }
  return source.slice(start + begin.length, stop).trim();
}

export function checkContract(contract, readShader, options = {}) {
  const problems = [];
  const structs = new Map();
  const shaderSources = new Map();

  for (const shader of contract.shaders) {
    const source = readShader(shader.path);
    shaderSources.set(shader.name, { shader, source });
    balanced(source, shader.path, problems);
    if (shader.language === "wgsl") {
      for (const [name, members] of parseWgslStructs(source)) {
        structs.set(name, members);
      }
    }
  }

  // 1) layout dos blocos de uniform/storage contra o contrato
  for (const [blockName, block] of Object.entries(contract.uniforms)) {
    if (!block.struct) {
      problems.push(`uniforms.${blockName}: bloco sem campo 'struct' (nome da struct WGSL)`);
      continue;
    }
    const members = structs.get(block.struct);
    if (!members) {
      problems.push(`uniforms.${blockName}: struct '${block.struct}' não existe em nenhum shader WGSL`);
      continue;
    }
    const layout = layoutWgslStruct(members, structs, block.address_space);
    if (layout.size !== block.size) {
      problems.push(
        `uniforms.${blockName}: WGSL calcula ${layout.size} B, contrato declara ${block.size} B (struct ${block.struct})`
      );
    }
    if (block.address_space === "vertex_and_storage_read") {
      // O bloco tem duas vistas (vertex buffer de 72 B e array no storage); o
      // contrato só garante o tamanho do elemento, os campos são agrupamentos.
      continue;
    }
    for (const field of block.fields) {
      const member = layout.fields.find((entry) => entry.name === field.name);
      if (!member) {
        problems.push(
          `uniforms.${blockName}.${field.name}: não é membro da struct ${block.struct} (membros: ${layout.fields
            .map((entry) => entry.name)
            .join(", ")})`
        );
        continue;
      }
      if (member.offset !== field.offset || member.size !== field.size) {
        problems.push(
          `uniforms.${blockName}.${field.name}: WGSL offset/size ${member.offset}/${member.size}, contrato ${field.offset}/${field.size}`
        );
      }
    }
  }

  // 2) bindings + entry points por passe
  const bindGroups = new Map(contract.bind_groups.map((group) => [group.name, group]));
  for (const pass of contract.passes) {
    const entry = shaderSources.get(pass.shader);
    if (!entry) {
      problems.push(`passe '${pass.name}': shader '${pass.shader}' não está no contrato`);
      continue;
    }
    const { shader, source } = entry;
    const available = new Set(shader.entry_points ? Object.values(shader.entry_points) : []);
    for (const [stage, fn] of [
      ["vertex", pass.vertex_entry],
      ["fragment", pass.fragment_entry],
      ["compute", pass.compute_entry],
    ]) {
      if (!fn) continue;
      if (!available.has(fn)) {
        problems.push(`passe '${pass.name}': entry point '${fn}' (${stage}) não está declarado no shader`);
      }
      if (!new RegExp(`fn\\s+${fn}\\s*\\(`).test(source)) {
        problems.push(`passe '${pass.name}': 'fn ${fn}(' não aparece em ${shader.path}`);
      }
    }
    if (shader.language !== "wgsl") continue;
    const group = bindGroups.get(pass.bind_group);
    if (!group) {
      problems.push(`passe '${pass.name}': bind group '${pass.bind_group}' não existe`);
      continue;
    }
    const declared = parseWgslBindings(source)
      .filter((binding) => binding.group === group.group)
      .map((binding) => binding.binding)
      .sort((a, b) => a - b);
    const stages = pass.kind === "compute" ? ["compute"] : ["vertex", "fragment"];
    const expectedSet = [
      ...new Set(
        group.entries
          .filter((entry) => entry.stages.some((stage) => stages.includes(stage)))
          .map((entry) => entry.binding)
      ),
    ].sort((a, b) => a - b);
    const missing = expectedSet.filter((binding) => !declared.includes(binding));
    const extra = declared.filter((binding) => !expectedSet.includes(binding));
    if (missing.length > 0) {
      problems.push(`${shader.path}: faltam @binding ${missing.join(", ")} exigidos pelo bind group '${group.name}'`);
    }
    if (extra.length > 0) {
      problems.push(`${shader.path}: declara @binding ${extra.join(", ")} que o bind group '${group.name}' não tem`);
    }
  }

  // 3) bloco de skinning idêntico entre os shaders que o usam
  const skinning = contract.skinning;
  if (skinning?.block_markers?.length === 2) {
    const [begin, end] = skinning.block_markers;
    const blocks = new Map();
    for (const { shader, source } of shaderSources.values()) {
      if (shader.language !== "wgsl") continue;
      if (!source.includes(begin)) continue;
      const block = extractBlock(source, begin, end, shader.path, problems);
      if (block) blocks.set(shader.name, block);
    }
    if (blocks.size > 0) {
      const [firstName, firstBlock] = [...blocks.entries()][0];
      for (const [name, block] of blocks) {
        if (block !== firstBlock) {
          problems.push(
            `skinning: bloco de '${name}' difere de '${firstName}' — a definição precisa ser byte-idêntica nos dois shaders`
          );
        }
      }
      for (const [name, block] of blocks) {
        for (const required of ["matrices[", "weights", "joints"]) {
          if (!block.includes(required)) {
            problems.push(`skinning: bloco de '${name}' não usa '${required}'`);
          }
        }
      }
    } else {
      problems.push("skinning: nenhum shader WGSL declara o bloco de skinning");
    }
  }

  // 3b) Blocos compartilhados de shading (Fase 2 #17 face SDF, #43 olho
  // anime): a definição canônica vive em um shader library e é copiada
  // byte-idêntica para o shader que compila no passe — mesma regra do
  // skinning: uma definição, vários usos.
  for (const [sectionName, section] of Object.entries({
    face_sdf: contract.face_sdf,
    anime_eye: contract.anime_eye,
  })) {
    if (!section?.block_markers?.length === 2) {
      problems.push(`contrato sem '${sectionName}.block_markers' (bloco compartilhado)`);
      continue;
    }
    const [begin, end] = section.block_markers;
    const blocks = new Map();
    for (const { shader, source } of shaderSources.values()) {
      if (shader.language !== "wgsl") continue;
      if (!section.shared_by?.includes(shader.name)) continue;
      if (!source.includes(begin)) {
        problems.push(`${shader.path}: shader '${shader.name}' listado em ${sectionName}.shared_by sem o bloco`);
        continue;
      }
      const block = extractBlock(source, begin, end, shader.path, problems);
      if (block) blocks.set(shader.name, block);
    }
    if (blocks.size > 0) {
      const [firstName, firstBlock] = [...blocks.entries()][0];
      for (const [name, block] of blocks) {
        if (block !== firstBlock) {
          problems.push(
            `${sectionName}: bloco de '${name}' difere de '${firstName}' — a definição precisa ser byte-idêntica`
          );
        }
      }
      for (const fn of section.entry_functions ?? []) {
        if (!blocks.get(firstName)?.includes(`fn ${fn}(`)) {
          problems.push(`${sectionName}: bloco canônico sem a função '${fn}'`);
        }
      }
    } else {
      problems.push(`${sectionName}: nenhum shader declara o bloco compartilhado`);
    }
  }

  // 4) paleta dos shaders de fallback (GLSL) — mesmo contrato, outra linguagem
  const paletteUniform = "u_bones";
  for (const shader of contract.shaders) {
    if (shader.language !== "glsl") continue;
    const source = shaderSources.get(shader.name).source;
    if (!source.includes("#version 300 es")) {
      problems.push(`${shader.path}: fallback WebGL2 precisa declarar '#version 300 es'`);
    }
    if (!shader.name.endsWith("_vertex")) continue;
    if (!new RegExp(`uniform\\s+mat4\\s+${paletteUniform}\\s*\\[\\s*${skinning?.joint_count ?? 24}\\s*\\]\\s*;`).test(source)) {
      problems.push(`${shader.path}: falta 'uniform mat4 ${paletteUniform}[${skinning?.joint_count ?? 24}];'`);
    }
    if (!source.includes(`${paletteUniform}[`)) {
      problems.push(`${shader.path}: paleta declarada mas não aplicada`);
    }
  }

  return problems;
}

function main() {
  const contract = JSON.parse(fs.readFileSync(contractPath, "utf8"));
  const readShader = (relative) => {
    const file = path.join(root, relative);
    if (!fs.existsSync(file)) throw new Error(`shader não encontrado: ${relative}`);
    return fs.readFileSync(file, "utf8").replace(/\r\n/g, "\n");
  };
  let problems;
  try {
    problems = checkContract(contract, readShader);
  } catch (error) {
    console.error(`[check:wgsl] falha ao analisar: ${error.message}`);
    process.exit(1);
  }
  if (problems.length > 0) {
    console.error(`[check:wgsl] ${problems.length} problema(s) de contrato:\n`);
    for (const problem of problems) console.error(`  - ${problem}`);
    process.exit(1);
  }
  const wgsl = contract.shaders.filter((shader) => shader.language === "wgsl").length;
  const glsl = contract.shaders.filter((shader) => shader.language === "glsl").length;
  console.log(
    `[check:wgsl] ${wgsl} shader(s) WGSL e ${glsl} GLSL conferem com o render contract ` +
      `(${Object.keys(contract.uniforms).length} blocos, ${contract.bind_groups.length} bind groups, skinning ${contract.skinning?.joint_count} ossos)`
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main();
}
