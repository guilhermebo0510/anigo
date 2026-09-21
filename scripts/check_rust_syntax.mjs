#!/usr/bin/env node
/**
 * ANIGO — Rust syntax gate without a Rust toolchain.
 *
 * `cargo check` remains the authority on GitHub runners (see .github/workflows/ci.yml),
 * but contributors and sandboxes without rustup still need a fast, deterministic
 * way to catch broken Rust before pushing. This script parses every `.rs` file in
 * the workspace with the tree-sitter Rust grammar (WASM) and fails on any ERROR
 * or MISSING node.
 *
 * Usage: node scripts/check_rust_syntax.mjs [paths...]
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const DEFAULT_ROOTS = ["crates", "src-tauri/src"];
const SKIP_DIRS = new Set(["target", "target_check", "node_modules", ".git"]);

function collectRustFiles(target) {
  const stats = fs.statSync(target);
  if (stats.isFile()) return target.endsWith(".rs") ? [target] : [];
  const out = [];
  for (const entry of fs.readdirSync(target, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = path.join(target, entry.name);
    if (entry.isDirectory()) out.push(...collectRustFiles(full));
    else if (entry.name.endsWith(".rs")) out.push(full);
  }
  return out.sort();
}

// Grammar limitations of tree-sitter-rust that rustc accepts. Each entry is a
// (node) predicate: matching nodes are reported as warnings, never failures.
const GRAMMAR_ALLOWLIST = [
  {
    // Rust accepts a trailing comma in a closure parameter list (`|a, b,| {}`);
    // the grammar reports a zero-width MISSING identifier inside
    // `closure_parameters` instead (mesh.rs uses this for its grid helpers).
    id: "closure-trailing-comma",
    matches: (node) => node.isMissing && node.parent?.type === "closure_parameters",
  },
];

function classify(node) {
  for (const entry of GRAMMAR_ALLOWLIST) {
    if (entry.matches(node)) return entry.id;
  }
  return null;
}

function collectErrorNodes(node, out, warnings, source) {
  if (node.type === "ERROR" || node.isMissing) {
    const record = {
      start: node.startPosition,
      end: node.endPosition,
      type: node.isMissing ? `MISSING ${node.type}` : "ERROR",
      text: source.slice(node.startIndex, Math.min(node.endIndex, node.startIndex + 120)),
    };
    const allowed = classify(node);
    if (allowed) {
      record.allowed = allowed;
      warnings.push(record);
    } else {
      out.push(record);
    }
  }
  for (const child of node.children) collectErrorNodes(child, out, warnings, source);
}

const inputs = process.argv.slice(2);
const targets = inputs.length > 0 ? inputs.map((p) => path.resolve(root, p)) : DEFAULT_ROOTS.map((p) => path.join(root, p));

const files = targets.flatMap(collectRustFiles);
if (files.length === 0) {
  console.error("[check_rust_syntax] no .rs files found");
  process.exit(2);
}

const wasmDir = path.join(root, "node_modules", "tree-sitter-wasms", "out");
const rustGrammar = path.join(wasmDir, "tree-sitter-rust.wasm");
if (!fs.existsSync(rustGrammar)) {
  console.error(
    `[check_rust_syntax] missing grammar at ${rustGrammar}\n` +
      "  install dev dependencies first: npm ci (or npm i -D tree-sitter-wasms web-tree-sitter)"
  );
  process.exit(2);
}

// web-tree-sitter ships different export shapes across versions:
//   0.22.x → module itself is the emscripten factory; `Language` appears as a
//            static once `init()` resolves.
//   0.25+  → { Parser, Language } (optionally under `default`).
const webTreeSitter = require("web-tree-sitter");
const Parser =
  typeof webTreeSitter === "function"
    ? webTreeSitter
    : webTreeSitter.Parser ?? webTreeSitter.default?.Parser ?? webTreeSitter.default;
if (!Parser || typeof Parser.init !== "function") {
  console.error("[check_rust_syntax] unsupported web-tree-sitter build (missing Parser.init)");
  process.exit(2);
}

await Parser.init();
const Language =
  webTreeSitter.Language ?? webTreeSitter.default?.Language ?? Parser.Language;
if (!Language || typeof Language.load !== "function") {
  console.error("[check_rust_syntax] unsupported web-tree-sitter build (missing Language.load)");
  process.exit(2);
}

const parser = new Parser();
parser.setLanguage(await Language.load(rustGrammar));

let failures = 0;
let warningCount = 0;
for (const file of files) {
  const source = fs.readFileSync(file, "utf8");
  const tree = parser.parse(source);
  const problems = [];
  const warnings = [];
  collectErrorNodes(tree.rootNode, problems, warnings, source);
  const rel = path.relative(root, file).replace(/\\/g, "/");
  if (problems.length === 0 && warnings.length === 0) {
    console.log(`ok   ${rel}`);
    continue;
  }
  for (const warning of warnings) {
    warningCount++;
    console.warn(
      `warn ${rel}:${warning.start.row + 1}:${warning.start.column + 1} ${warning.type} (${warning.allowed} — accepted by rustc)`
    );
  }
  if (problems.length === 0) {
    console.log(`ok   ${rel} (${warnings.length} grammar warning${warnings.length === 1 ? "" : "s"})`);
    continue;
  }
  failures++;
  console.error(`FAIL ${rel}`);
  for (const problem of problems.slice(0, 10)) {
    console.error(
      `  ${problem.start.row + 1}:${problem.start.column + 1} ${problem.type}: ${problem.text.replace(/\s+/g, " ").trim()}`
    );
  }
  if (problems.length > 10) console.error(`  ... ${problems.length - 10} more`);
}

console.log(
  `\n[check_rust_syntax] ${files.length - failures}/${files.length} files parsed cleanly` +
    (warningCount > 0 ? ` (${warningCount} allowed grammar warning${warningCount === 1 ? "" : "s"})` : "")
);
process.exit(failures === 0 ? 0 : 1);
