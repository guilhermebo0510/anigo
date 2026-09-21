#!/usr/bin/env node
/**
 * ANIGO — cross-file call audit (semantic smoke test without cargo).
 *
 * `check_rust_syntax.mjs` proves the Rust parses; this script proves the
 * *names* used in the workspace exist somewhere in the workspace. It is a
 * cheap substitute for `cargo check` in sandboxes that have no toolchain:
 *
 *   1. parses every `.rs` file with tree-sitter,
 *   2. collects every declared name (functions, methods, struct fields, enum
 *      variants, consts, macro rules),
 *   3. collects every member access (`receiver.name` and `receiver.name(...)`),
 *   4. reports the members that are not declared anywhere and are not part of
 *      the std/glam/wgpu surface listed in `EXTERNAL_MEMBERS`.
 *
 * A hit is a *lead*, not a verdict: it can be a trait method, a macro expansion
 * or an external crate API missing from the allowlist. It cannot see types, so
 * it will not catch a method called on the wrong type.
 *
 * Usage: node scripts/audit_calls.mjs [--all] [paths...]
 *        --all  also print members that are not method-like (no underscore)
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const showAll = args.includes("--all");
const inputs = args.filter((a) => !a.startsWith("--"));

const DEFAULT_ROOTS = ["crates", "src-tauri/src"];
const SKIP_DIRS = new Set(["target", "target_check", "node_modules", ".git"]);

// Members provided by std / external crates used across the workspace. Kept
// deliberately small: everything else must be declared in the workspace.
const EXTERNAL_MEMBERS = new Set([
  // std / core
  "abs", "acos", "add", "all", "any", "as_bytes", "as_deref", "as_f64", "as_i64", "as_mut",
  "as_ref", "as_slice", "as_str", "as_u32", "as_u64", "asin", "atan2", "buffer", "bytes",
  "canonicalize", "capacity", "ceil", "chars", "checked_add", "checked_sub", "chunks",
  "chunks_exact", "clamp", "clone", "collect", "concat", "contains", "contains_key", "copied",
  "copy_from_slice", "cos", "count", "count_ones", "dedup", "default", "deref", "display",
  "div_ceil", "drain", "ends_with", "entry", "enumerate", "eq", "err", "exp", "extend",
  "extend_from_slice", "file_name", "filter", "filter_map", "find", "find_map", "first",
  "flat_map", "floor", "fmt", "fold", "format", "from_iter", "from_utf8", "get", "get_mut",
  "hash", "insert", "into_boxed_str", "into_bytes", "into_iter", "into_string", "into_vec",
  "is_char_boundary", "is_empty", "is_err", "is_finite", "is_infinite", "is_nan", "is_none",
  "is_ok", "is_some", "is_some_and", "iter", "iter_mut", "join", "key", "keys", "last", "len",
  "lines", "ln", "log2", "map", "max", "min", "mul", "ne", "next", "ok", "ok_or", "ok_or_else",
  "or_else", "parse", "partition", "pop", "position", "powf", "powi", "push", "push_str",
  "read_to_end", "read_to_string", "remove", "repeat", "replace", "reserve", "retain", "rev",
  "rev_iter", "round", "rposition", "rsplit", "rsplit_once", "signum", "sin", "skip", "slice",
  "sort", "sort_by", "sort_by_key", "split", "split_at", "split_once", "split_whitespace",
  "sqrt", "starts_with", "step_by", "strip_prefix", "strip_suffix", "sum", "take", "tan",
  "to_ascii_lowercase", "to_ascii_uppercase", "to_be_bytes", "to_bits", "to_lowercase",
  "to_owned", "to_string", "to_uppercase", "to_vec", "total_cmp", "truncate", "unwrap",
  "unwrap_or", "unwrap_or_default", "unwrap_or_else", "values", "values_mut", "with_capacity",
  "wrapping_add", "wrapping_mul", "wrapping_sub", "write_all", "zip",
  // glam
  "to_array", "from_array", "normalize", "normalize_or_zero", "length", "length_squared",
  "dot", "cross", "distance", "lerp", "inverse", "transpose", "to_cols_array", "from_cols_array",
  "to_scale_rotation_translation", "from_scale_rotation_translation", "project_point3",
  "transform_point3", "transform_vector3", "is_finite_mask", "x", "y", "z", "w", "truncate",
  "perp", "any_orthonormal_vector", "to_euler", "from_euler", "look_at_lh", "look_at_rh",
  "perspective_lh", "perspective_rh", "orthographic_lh", "orthographic_rh", "inverse_proj",
  "mul_mat4", "as_vec3", "from_rotation_x", "from_rotation_y", "from_rotation_z",
  "from_translation", "from_scale", "from_axis_angle", "from_quat", "from_mat4", "to_quat",
  "rotation_x", "rotation_y", "rotation_z", "scale", "translation", "quat", "to_simd",
  // wgpu / bytemuck / pollster / image / anyhow / thiserror / tokio / serde_json
  "as_bytes_of", "cast_slice", "map_async", "poll", "get_mapped_range", "unmap", "create_view",
  "create_buffer", "create_texture", "write_buffer", "create_command_encoder", "queue",
  "submit", "create_bind_group", "create_bind_group_layout", "create_compute_pipeline",
  "create_render_pipeline", "create_pipeline_layout", "create_shader_module", "create_sampler",
  "create_buffer_init", "create_texture_with_data", "set_bind_group", "set_pipeline",
  "draw_indexed", "draw", "begin_render_pass", "begin_compute_pass", "set_vertex_buffer",
  "set_index_buffer", "dispatch_workgroups", "copy_texture_to_buffer", "copy_buffer_to_buffer",
  "copy_buffer_to_texture", "as_secs_f64", "as_millis", "save", "save_buffer", "to_rgba8",
  "to_rgb8", "pixels", "dimensions", "context", "with_context", "kind", "source", "root_cause",
  "get_info", "get_limits", "features", "limits", "request_adapter", "request_device",
  "get_current_texture", "present", "create_surface", "enter", "block_on", "sleep", "spawn",
  "bind", "accept", "read", "write", "flush", "shutdown", "send", "recv", "interval", "tick",
  "get_or_insert_with", "get_or_init", "lock", "read_lock", "write_lock", "try_lock", "emit",
  "emit_to", "get_webview_window", "show", "hide", "close", "destroy", "set_title", "set_size",
  "inner_size", "outer_size", "outer_position", "is_maximized", "maximize", "unmaximize",
  "minimize", "set_focus", "request_user_attention", "eval", "url", "as_ref", "type_id",
  "serialize", "deserialize", "from_str", "from_value", "to_value", "to_vec", "is_null",
  "as_object", "as_array", "as_object_mut", "as_array_mut", "as_bool", "as_f32", "as_u16",
  "as_u8", "as_i32", "as_i128", "as_u128", "as_f64_mut", "take", "insert", "shift_remove",
  "get_mut", "contains_key", "entry", "or_insert", "or_insert_with", "or_default", "retain",
  "iter", "keys", "values", "len", "is_empty", "clear", "extend", "remove", "append", "split_off",
  "first_key_value", "last_key_value", "range", "to_owned", "into_values", "into_keys",
]);

function collectFiles(target) {
  const stats = fs.statSync(target);
  if (stats.isFile()) return target.endsWith(".rs") ? [target] : [];
  const out = [];
  for (const entry of fs.readdirSync(target, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = path.join(target, entry.name);
    if (entry.isDirectory()) out.push(...collectFiles(full));
    else if (entry.name.endsWith(".rs")) out.push(full);
  }
  return out.sort();
}

const wasmDir = path.join(root, "node_modules", "tree-sitter-wasms", "out");
const grammar = path.join(wasmDir, "tree-sitter-rust.wasm");
if (!fs.existsSync(grammar)) {
  console.error(`[audit_calls] missing grammar at ${grammar} — run npm ci first`);
  process.exit(2);
}

const webTreeSitter = require("web-tree-sitter");
const Parser =
  typeof webTreeSitter === "function"
    ? webTreeSitter
    : webTreeSitter.Parser ?? webTreeSitter.default?.Parser ?? webTreeSitter.default;
await Parser.init();
const Language = webTreeSitter.Language ?? webTreeSitter.default?.Language ?? Parser.Language;
const parser = new Parser();
parser.setLanguage(await Language.load(grammar));

const targets = inputs.length > 0 ? inputs.map((p) => path.resolve(root, p)) : DEFAULT_ROOTS.map((p) => path.join(root, p));
const files = targets.flatMap(collectFiles);
if (files.length === 0) {
  console.error("[audit_calls] no .rs files found");
  process.exit(2);
}

const declared = new Set();
const used = new Map(); // name -> [{file, row, col}]

function attrText(node, source) {
  const prev = node.previousSibling;
  return prev ? source.slice(prev.startIndex, prev.endIndex) : "";
}

for (const file of files) {
  const source = fs.readFileSync(file, "utf8");
  const tree = parser.parse(source);
  const rel = path.relative(root, file).replace(/\\/g, "/");

  (function walk(node, inMacroRules) {
    switch (node.type) {
      case "function_item":
      case "function_signature_item":
      case "struct_item":
      case "enum_item":
      case "union_item":
      case "const_item":
      case "static_item":
      case "type_item":
      case "trait_item":
      case "mod_item": {
        const name = node.childForFieldName("name");
        if (name) declared.add(source.slice(name.startIndex, name.endIndex));
        break;
      }
      case "field_declaration": {
        const name = node.childForFieldName("name");
        if (name) declared.add(source.slice(name.startIndex, name.endIndex));
        break;
      }
      case "enum_variant": {
        const name = node.childForFieldName("name");
        if (name) declared.add(source.slice(name.startIndex, name.endIndex));
        break;
      }
      case "macro_definition": {
        const name = node.childForFieldName("name");
        if (name) declared.add(source.slice(name.startIndex, name.endIndex));
        break;
      }
      case "macro_invocation": {
        const macro = node.childForFieldName("macro");
        if (macro) declared.add(source.slice(macro.startIndex, macro.endIndex));
        break;
      }
      case "field_expression": {
        const field = node.childForFieldName("field");
        if (field) {
          const name = source.slice(field.startIndex, field.endIndex);
          if (!used.has(name)) used.set(name, []);
          const list = used.get(name);
          if (list.length < 5) {
            list.push({ file: rel, row: field.startPosition.row + 1, col: field.startPosition.column + 1 });
          }
        }
        break;
      }
      default:
        break;
    }
    for (const child of node.children) walk(child, inMacroRules);
  })(tree.rootNode, false);
}

const missing = [];
for (const [name, sites] of [...used.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (declared.has(name) || EXTERNAL_MEMBERS.has(name)) continue;
  if (!showAll && !name.includes("_")) continue;
  missing.push({ name, sites });
}

if (missing.length === 0) {
  console.log(`[audit_calls] ${files.length} files, ${used.size} member names — every member is declared or allowlisted`);
  process.exit(0);
}

console.warn(`[audit_calls] ${missing.length} undeclared member name(s) — each hit needs manual review:\n`);
for (const { name, sites } of missing) {
  console.warn(`  ${name}`);
  for (const site of sites) console.warn(`      ${site.file}:${site.row}:${site.col}`);
}
console.log(`\n[audit_calls] ${files.length} files scanned (no cargo available — this is a name-level heuristic)`);
process.exit(1);
