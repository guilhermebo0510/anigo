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
  return fs.readFileSync(path.join(REPO_ROOT, relativePath), "utf8").replace(/\r\n/g, "\n");
}

/**
 * Removes comments while preserving string literals and byte offsets.
 *
 * A naive `indexOf("//")` would truncate string literals that contain URLs
 * (`"anigo://base/…"`), so the scanner tracks strings and nested block comments
 * the way Rust does. It does not track char literals or lifetimes: a `//` split
 * across two adjacent char literals would be misread, which cannot happen in the
 * sources this reads.
 */
export function stripComments(source: string): string {
  const out: string[] = [];
  let index = 0;
  let blockDepth = 0;

  while (index < source.length) {
    const char = source[index];
    const next = source[index + 1];

    if (blockDepth > 0) {
      if (char === "/" && next === "*") {
        blockDepth++;
        out.push("  ");
        index += 2;
      } else if (char === "*" && next === "/") {
        blockDepth--;
        out.push("  ");
        index += 2;
      } else {
        out.push(char === "\n" ? "\n" : " ");
        index++;
      }
      continue;
    }

    if (char === "/" && next === "/") {
      while (index < source.length && source[index] !== "\n") {
        out.push(" ");
        index++;
      }
      continue;
    }

    if (char === "/" && next === "*") {
      blockDepth++;
      out.push("  ");
      index += 2;
      continue;
    }

    if (char === '"') {
      out.push(char);
      index++;
      while (index < source.length) {
        const current = source[index];
        out.push(current);
        index++;
        if (current === "\\") {
          if (index < source.length) {
            out.push(source[index]);
            index++;
          }
          continue;
        }
        if (current === '"') break;
      }
      continue;
    }

    out.push(char);
    index++;
  }

  return out.join("");
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

/** Splits a body on top-level commas, ignoring commas inside `<> [ ] ( ) { }`. */
function splitTopLevel(body: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = "";
  for (const char of body) {
    if ("<([{".includes(char)) depth++;
    else if (">)]}".includes(char)) depth = Math.max(0, depth - 1);
    if (char === "," && depth === 0) {
      parts.push(current);
      current = "";
      continue;
    }
    current += char;
  }
  parts.push(current);
  return parts.map((part) => part.trim()).filter((part) => part.length > 0);
}

/** `#[serde(default)] light_id: Option<LightId>` → `light_id`. */
function fieldNameOf(part: string): string | null {
  const withoutAttributes = part.replace(/#\[[^\]]*\]/g, " ").trim();
  const match = /^(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:/.exec(withoutAttributes);
  return match ? match[1] : null;
}

/**
 * Field names of a struct (fields may be `pub` or not) or of one enum variant
 * (`Variant { a, b }` — the wire shape of internally tagged enums).
 *
 * Works for both multi-line and single-line declarations, because the variant
 * formatting in this repository is not uniform.
 */
export function fieldsOf(source: string, name: string, variant?: string): string[] {
  let body = bracedBody(source, variant ? "enum" : "struct", name);
  if (variant) {
    const variantMatch = new RegExp(`\\b${variant}\\s*\\{`).exec(body);
    if (!variantMatch) throw new Error(`variant ${variant} not found in ${name}`);
    body = balancedBody(body, variantMatch.index + variantMatch[0].length - 1);
  }
  const fields = splitTopLevel(body)
    .map(fieldNameOf)
    .filter((field): field is string => field !== null);
  if (fields.length === 0) throw new Error(`${variant ?? name} parsed to zero fields`);
  return fields;
}

/**
 * Fields of every variant of an enum, in declaration order.
 *
 * Unit variants map to an empty array, so callers never have to know which
 * variants carry data.
 */
export function enumVariantFields(source: string, name: string): Map<string, string[]> {
  const body = bracedBody(source, "enum", name);
  const out = new Map<string, string[]>();
  for (const variant of enumVariants(source, name)) {
    const match = new RegExp(`\\b${variant}\\s*\\{`).exec(body);
    if (!match) {
      out.set(variant, []);
      continue;
    }
    const variantBody = balancedBody(body, match.index + match[0].length - 1);
    out.set(
      variant,
      splitTopLevel(variantBody)
        .map(fieldNameOf)
        .filter((field): field is string => field !== null)
    );
  }
  return out;
}

/** Numeric const: `pub const NAME: u32 = 72;`. */
export function numericConst(source: string, name: string): number {
  const match = new RegExp(`const\\s+${name}\\s*:\\s*[a-z0-9]+\\s*=\\s*([0-9_.]+)`, "i").exec(
    stripComments(source)
  );
  if (!match) throw new Error(`const ${name} not found`);
  return Number(match[1].replace(/_/g, ""));
}

/** String const: `pub const NAME: &str = "value";` (also matches `&'static str`). */
export function stringConst(source: string, name: string): string {
  const match = new RegExp(
    `const\\s+${name}\\s*:\\s*&(?:'static\\s+)?str\\s*=\\s*"([^"]*)"`
  ).exec(stripComments(source));
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

/**
 * Members of a TypeScript string-literal union:
 * `export type ChangeScope = "a" | "b";`
 */
export function tsStringUnion(source: string, name: string): string[] {
  const match = new RegExp(`type\\s+${name}\\s*=([^;]+);`).exec(source);
  if (!match) throw new Error(`TS union ${name} not found`);
  const members = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (members.length === 0) throw new Error(`TS union ${name} parsed to zero members`);
  return members;
}

/** Members of a TypeScript `as const` array of string literals. */
export function tsStringArrayConst(source: string, name: string): string[] {
  const match = new RegExp(`const\\s+${name}\\s*=\\s*\\[([\\s\\S]*?)\\]\\s*as const`).exec(source);
  if (!match) throw new Error(`TS array const ${name} not found`);
  const members = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (members.length === 0) throw new Error(`TS array const ${name} parsed to zero members`);
  return members;
}

/** `PascalCase` → `snake_case` (matches serde's `rename_all = "snake_case"`). */
export function pascalToSnake(name: string): string {
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase();
}

/**
 * Fields of a struct/variant annotated `#[serde(default…)]`.
 *
 * TypeScript models those as optional properties (`field?`), so the drift test
 * can assert that "optional on the wire" and "defaulted in Rust" are the same
 * set of fields.
 */
export function defaultedFields(source: string, name: string, variant?: string): string[] {
  let body = bracedBody(source, variant ? "enum" : "struct", name);
  if (variant) {
    const variantMatch = new RegExp(`\\b${variant}\\s*\\{`).exec(body);
    if (!variantMatch) throw new Error(`variant ${variant} not found in ${name}`);
    body = balancedBody(body, variantMatch.index + variantMatch[0].length - 1);
  }
  return splitTopLevel(body)
    .filter((part) => part.includes("serde(default"))
    .map(fieldNameOf)
    .filter((field): field is string => field !== null);
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
