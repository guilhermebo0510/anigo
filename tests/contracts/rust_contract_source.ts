/**
 * ANIGO contract tests — textual readers for the Rust source of truth.
 *
 * The tests run with `node --test` (no cargo), so the Rust side of every
 * contract is read as text: constants, enum variants, struct fields. A drift
 * between the two languages therefore fails `npm test` long before CI runs
 * `cargo check`.
 *
 * Keep the readers strict — a reader that silently returns an empty list would
 * turn a contract test into a no-op.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(here, "..", "..");

export function readRepoFile(relativePath: string): string {
  return fs.readFileSync(path.join(REPO_ROOT, relativePath), "utf8");
}

/** Removes `//` line comments and `/* */` block comments (keeps line count). */
export function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, " "))
    .split("\n")
    .map((line) => {
      const index = line.indexOf("//");
      return index === -1 ? line : line.slice(0, index);
    })
    .join("\n");
}

/** Body of the balanced `{ ... }` starting at `openIndex`. */
export function balancedBody(source: string, openIndex: number): string {
  let depth = 0;
  for (let i = openIndex; i < source.length; i++) {
    const char = source[i];
    if (char === "{") depth++;
    else if (char === "}") {
      depth--;
      if (depth === 0) return source.slice(openIndex + 1, i);
    }
  }
  throw new Error("unbalanced braces");
}

/** Extracts the body of `enum|struct|trait <Name>` with balanced braces. */
export function bracedBody(source: string, kind: string, name: string): string {
  const cleaned = stripComments(source);
  const header = new RegExp(`\\b${kind}\\s+${name}\\b`).exec(cleaned);
  if (!header) throw new Error(`${kind} ${name} not found`);
  const start = cleaned.indexOf("{", header.index);
  if (start === -1) throw new Error(`${kind} ${name} has no body`);
  return balancedBody(cleaned, start);
}

/** Enum variant names in declaration order (`PascalCase`). */
export function enumVariants(source: string, name: string): string[] {
  const body = bracedBody(source, "enum", name);
  const variants: string[] = [];
  for (const line of body.split("\n")) {
    const match = /^\s*([A-Z][A-Za-z0-9_]*)\s*(?:[{(,]|$)/.exec(line);
    if (match) variants.push(match[1]);
  }
  if (variants.length === 0) throw new Error(`enum ${name} parsed to zero variants`);
  return variants;
}

/** `pub` field names of an enum variant (`Variant { a, b }`) or struct. */
export function fieldsOf(source: string, name: string, variant?: string): string[] {
  let body = bracedBody(source, variant ? "enum" : "struct", name);
  if (variant) {
    const variantMatch = new RegExp(`\\b${variant}\\s*\\{`).exec(body);
    if (!variantMatch) throw new Error(`variant ${variant} not found in ${name}`);
    body = balancedBody(body, variantMatch.index + variantMatch[0].length - 1);
  }
  const fields: string[] = [];
  for (const line of body.split("\n")) {
    const match = /^\s*pub\s+([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (match) fields.push(match[1]);
  }
  return fields;
}

/** All field names of a struct, whether `pub` or not. */
export function structFieldNames(source: string, name: string): string[] {
  const body = bracedBody(source, "struct", name);
  const fields: string[] = [];
  for (const line of body.split("\n")) {
    const match = /^\s*(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (match) fields.push(match[1]);
  }
  if (fields.length === 0) throw new Error(`struct ${name} parsed to zero fields`);
  return fields;
}

/** Numeric const: `pub const NAME: u32 = 72;`. */
export function numericConst(source: string, name: string): number {
  const match = new RegExp(`const\\s+${name}\\s*:\\s*[a-z0-9]+\\s*=\\s*([0-9_.]+)`, "i").exec(
    stripComments(source)
  );
  if (!match) throw new Error(`const ${name} not found`);
  return Number(match[1].replace(/_/g, ""));
}

/** String const: `pub const NAME: &str = "value";`. */
export function stringConst(source: string, name: string): string {
  const match = new RegExp(`const\\s+${name}\\s*:\\s*&'?static\\s+str\\s*=\\s*"([^"]*)"`).exec(
    stripComments(source)
  );
  if (!match) throw new Error(`const ${name} not found`);
  return match[1];
}

/** Field names declared in a TypeScript `interface`/`type` object literal. */
export function tsInterfaceFields(source: string, name: string): string[] {
  const match = new RegExp(`(?:interface|type)\\s+${name}\\b[^{]*\\{`).exec(source);
  if (!match) throw new Error(`TS type ${name} not found`);
  const body = (() => {
    let depth = 0;
    for (let i = match.index + match[0].length - 1; i < source.length; i++) {
      if (source[i] === "{") depth++;
      else if (source[i] === "}") {
        depth--;
        if (depth === 0) return source.slice(match.index + match[0].length, i);
      }
    }
    throw new Error(`TS type ${name} is not balanced`);
  })();

  const fields: string[] = [];
  for (const line of body.split("\n")) {
    const field = /^\s*(?:\/\*\*[\s\S]*?\*\/\s*)?([a-zA-Z_][a-zA-Z0-9_]*)\??\s*:/.exec(line);
    if (field) fields.push(field[1]);
  }
  if (fields.length === 0) throw new Error(`TS type ${name} parsed to zero fields`);
  return fields;
}

/** `PascalCase` → `snake_case` (matches serde's `rename_all = "snake_case"`). */
export function pascalToSnake(name: string): string {
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase();
}

export function sorted(values: readonly string[]): string[] {
  return [...values].sort();
}

export function diff(expected: readonly string[], actual: readonly string[]): string {
  const missing = expected.filter((value) => !actual.includes(value));
  const extra = actual.filter((value) => !expected.includes(value));
  return [
    missing.length ? `missing: ${missing.join(", ")}` : "",
    extra.length ? `extra: ${extra.join(", ")}` : "",
    missing.length || extra.length ? "" : "identical",
  ]
    .filter(Boolean)
    .join(" | ");
}
