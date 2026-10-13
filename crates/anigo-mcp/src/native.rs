//! Ferramentas MCP do app NATIVO (T-2.13): `anigo_native_*`. Falam com a bridge do `anigo-studio --bridge`
//! (`crates/anigo-bridge`) pelo mesmo `LiveBridgeClient` das ferramentas antigas; toda mutação é um `Command`
//! do núcleo aplicado na MESMA sessão do usuário (desfazível com Ctrl+Z na janela).
//!
//! Entrada validada em `validate.rs`; arquivos só no sandbox de `fs_sandbox`; contadores em `metrics.rs`.

use crate::bridge_client::LiveBridgeClient;
use crate::fs_sandbox::sanitize_save_path;
use crate::metrics::McpMetrics;
use crate::validate;
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

/// Texto devolvido ao modelo é truncado acima disto (use `save_path` para o resto).
const MAX_TEXT_CHARS: usize = 120_000;

fn ann(read_only: bool, destructive: bool, idempotent: bool) -> Value {
    json!({ "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": false })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], annotations: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
        "annotations": annotations,
    })
}

/// Definições das ferramentas nativas (acrescentadas ao catálogo existente).
pub fn definitions() -> Vec<Value> {
    let save = json!({ "type": "string", "description": "Optional local path to save the result (allowlisted: Documents/ANIGO, ./baselines, ./tmp/anigo-mcp, ANIGO_MCP_ALLOWED_DIRS)" });
    vec![
        tool(
            "anigo_native_get_status",
            "Native ANIGO Studio (started with --bridge): active workspace, real frame time, GPU adapter, project name, camera, history report (undo/redo depth, revisions) and bridge metrics.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_get_project",
            "Reads the canonical project (ProjectState) of the native studio session. With include_document=false (default) returns only the history report (revisions, undo/redo depth, fingerprint); with true returns the full canonical document JSON (can be large; use save_path).",
            json!({ "include_document": { "type": "boolean", "default": false }, "save_path": save }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_get_snapshot",
            "Reads the CoreSnapshot (the canonical hand-off to renderers: dynamic weights/camera/lights/materials; geometry only if include_static=true — large). Prefer anigo_native_get_project for editing decisions.",
            json!({ "include_static": { "type": "boolean", "default": false }, "save_path": save }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_apply_command",
            "Applies ONE core Command (contract v1 JSON, e.g. {\"kind\":\"rename_project\",\"name\":\"X\"}, {\"kind\":\"set_base_gender\",\"gender\":\"female\"}, {\"kind\":\"batch\",\"commands\":[...]} for atomic groups) through the same CoreSession as the user: it is validated by the core, enters the undo history, and shows live in the window. Optional if_revision (history revision from a previous call) rejects the command if someone else changed the project meanwhile.",
            json!({
                "command": { "type": "object", "description": "Command JSON with a snake_case 'kind' (see anigo-core Command)", "properties": { "kind": { "type": "string" } }, "required": ["kind"] },
                "if_revision": { "type": "integer", "minimum": 0, "description": "Optimistic concurrency: only apply if history.revision equals this" }
            }), &["command"], ann(false, false, false),
        ),
        tool(
            "anigo_native_undo",
            "Undoes the last accepted command of the native session (same history as Ctrl+Z in the core-backed workspaces: Personagem, Cenário, Iluminação, Render, Posing).",
            json!({}), &[], ann(false, true, false),
        ),
        tool(
            "anigo_native_redo",
            "Redoes the last undone command of the native session.",
            json!({}), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_get_log",
            "Lists the session command log (the log is the session: undo does not remove entries). Page with from/limit (limit<=500); include_commands=true returns each Command JSON; sequence=N reads one entry with its command.",
            json!({
                "from": { "type": "integer", "minimum": 0 }, "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 },
                "include_commands": { "type": "boolean", "default": false }, "sequence": { "type": "integer", "minimum": 1 }
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_list_assets",
            "Lists Library assets (id, name, kind, format, triangles, bones...). Filters: text (Library search syntax), kinds (model, character, outfit, hair, accessory, material, texture, pose, animation, hdri, scene, prefab), limit<=500, after (cursor from a previous page).",
            json!({
                "text": { "type": "string", "maxLength": 256 },
                "kinds": { "type": "array", "items": { "type": "string", "enum": validate::NATIVE_ASSET_KINDS }, "maxItems": 16 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 100 }, "after": { "type": "string", "maxLength": 512 }
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_place_asset",
            "Places Library asset(s) in the scene as ONE undoable step (like double-click in the Content Drawer). Optional world position [x,y,z]; default is the center of the scene viewport.",
            json!({
                "asset_id": { "type": "string", "description": "Library asset UUID" },
                "asset_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 32 },
                "slot": { "type": "integer", "minimum": 0, "maximum": 3, "description": "set_view_kind only: which view (default: the active one)" },
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
            }), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_render_frame",
            "Queues a still render with the project's current render settings in the native studio's render queue (real final-quality pipeline) and, with wait=true (default), waits for it. Returns file paths; inline=true also returns the PNG image.",
            json!({
                "wait": { "type": "boolean", "default": true },
                "timeout_seconds": { "type": "integer", "minimum": 5, "maximum": 300, "default": 90 },
                "inline": { "type": "boolean", "default": false }, "save_path": save
            }), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_screenshot",
            "Screenshot of the native studio: the active workspace's 3D viewport (default) or the whole window. Returns a PNG image; max_width downsizes to save tokens.",
            json!({
                "target": { "type": "string", "enum": ["viewport", "window"], "default": "viewport" },
                "max_width": { "type": "integer", "minimum": 64, "maximum": 4096 }, "save_path": save
            }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_set_workspace",
            "Switches the active workspace of the native studio.",
            json!({ "workspace": { "type": "string", "enum": validate::NATIVE_WORKSPACES } }), &["workspace"], ann(false, false, true),
        ),
        tool(
            "anigo_native_get_ui_state",
            "Reads what is on screen in the native studio: active workspace, modeling mode (object|edit|sculpt|hair), selection mode, the tool ids available in the left shelf and the active one, every dock panel with open=true/false, and the modeling objects (index, name, active). Use it before ui_action to learn valid tool and panel ids.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_ui_action",
            "Drives the native studio UI through the same code paths as clicks. Actions: set_mode (value: object|edit|sculpt|hair; switches to Modelagem), select_tool (value: tool id from get_ui_state), toggle_panel (value: panel id; optional open:boolean to force), select_props_tab (value: Properties tab key from get_ui_state.props_tabs of the current workspace: Modelagem brush|symmetry|mesh|modifiers|view|history…, Personagem tool|project|history, Posing tool|history, Iluminação rigs|sun|shadows|advanced, Animação clip|bone, Shading shade|highlight|edge|face|textures), save_session (only with --session-file), set_camera_view (Cenário; value true|false), set_shading (value: solid|wireframe|cel), set_view_layout (value: single|split_v|split_h|quad), set_view_kind (value: perspective|ortho|front|back|right|left|top|bottom; optional slot 0..3, default the active view), set_active_view (value: slot 0..3), add_primitive (value: plane|grid|cube|quad_sphere|uv_sphere|cylinder|cone|torus|capsule|circle; optional position [x,y,z]; Modelagem object mode), select_object (value: object index), set_select_mode (value: vertex|edge|face). Returns the resulting UI state.",
            json!({
                "action": { "type": "string", "enum": ["set_mode", "select_tool", "toggle_panel", "select_props_tab", "save_session", "set_camera_view", "set_shading", "set_view_layout", "set_view_kind", "set_active_view", "add_primitive", "select_object", "set_select_mode"] },
                "value": { "description": "String id/kind for most actions; integer index for select_object" },
                "open": { "type": "boolean", "description": "toggle_panel only: true opens, false closes (default: toggles)" },
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "add_primitive only" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_input",
            "Injects synthetic pointer/wheel/keyboard events INSIDE the ANIGO window (never the OS cursor), one step per frame, to test the UI like a user. Coordinates are logical points of the window (same as the rects in get_ui_state.views). Steps: move{pos}, down|up{button,pos}, click{pos,button?}, scroll{delta:[dx,dy]}, key{key,pressed?}, mods{mods:[ctrl|shift|alt]} (hold until changed), wait{frames}, drag{from,to,button?,frames?}. Waits until the queue is empty (timeout_seconds) and returns the resulting UI state.",
            json!({
                "steps": { "type": "array", "maxItems": 256, "items": { "type": "object", "properties": { "type": { "type": "string", "enum": ["move", "down", "up", "click", "scroll", "key", "mods", "wait", "drag"] } }, "required": ["type"] } },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 60, "default": 15 }
            }), &["steps"], ann(false, false, false),
        ),
        tool(
            "anigo_native_model_info",
            "Reads the Modelagem (mesh modeling) state of the native studio: mode (object|edit|sculpt|hair), selection mode, every object (index, name, vertex/face counts), the active object's transform and bounds (local and world), mesh stats, the current selection (counts + ids), undo availability and the active sculpt brush/symmetry. include_geometry=true also returns vertex positions and face vertex lists (capped by limit<=2000) so you can verify an edit numerically.",
            json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "default": 100 }, "include_geometry": { "type": "boolean", "default": false } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_model_object",
            "Modelagem objects (same code paths as the UI; each change is one undo step of the MODELING history, see anigo_native_model_history). action: add (kind: plane|grid|cube|quad_sphere|uv_sphere|cylinder|cone|torus|capsule|circle, optional position [x,y,z]), select (index), delete, duplicate, rename (name), set_transform (any of translation [x,y,z] meters, rotation_deg [rx,ry,rz] XYZ Euler, scale [sx,sy,sz] > 0 — absolute values for the ACTIVE object). Switches to Object mode when needed.",
            json!({
                "action": { "type": "string", "enum": ["add", "select", "delete", "duplicate", "rename", "set_transform"] },
                "kind": { "type": "string" }, "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "index": { "type": "integer", "minimum": 0 }, "name": { "type": "string", "maxLength": 128 },
                "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_mesh_select",
            "Edit-mode selection on the active object (enters Edit mode). mode: vertex|edge|face (optional switch). what: all|none|invert|more|less|linked, or an explicit set: verts (indices), faces (indices), edges (edges:[[a,b],…]), or box (world-space {min:[x,y,z],max:[x,y,z]}: vertices inside; edges/faces only when all their vertices are inside). extend=true adds to the current selection. Returns the selection (counts + ids up to limit).",
            json!({
                "mode": { "type": "string", "enum": ["vertex", "edge", "face"] },
                "what": { "type": "string", "enum": ["all", "none", "invert", "more", "less", "linked", "verts", "edges", "faces", "box"] },
                "indices": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "maxItems": 100000 },
                "edges": { "type": "array", "items": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 }, "maxItems": 100000 },
                "box": { "type": "object", "properties": { "min": { "type": "array", "items": { "type": "number" } }, "max": { "type": "array", "items": { "type": "number" } } }, "required": ["min", "max"] },
                "extend": { "type": "boolean", "default": false }, "limit": { "type": "integer", "minimum": 1, "maximum": 2000 }
            }), &["what"], ann(false, false, true),
        ),
        tool(
            "anigo_native_mesh_op",
            "Runs ONE Edit-mode operator on the active object's selection — the same operators as the Operators panel (enters Edit mode). name: extrude, extrude_individual, inset, inset_individual, bevel, loop_cut, bisect, delete_verts, delete_edges, delete_faces, delete_only_faces, merge_center, merge_by_distance, dissolve_verts, dissolve_edges, dissolve_faces, limited_dissolve, fill_hole, grid_fill, bridge, triangulate, tris_to_quads, flip_normals, recalc_outside, smooth, relax, subdivide, poke, mark_sharp, clear_sharp, mark_seam, clear_seam, set_crease, hide_selected, reveal_all. params (all optional): distance, thickness, depth, outset, even, width, segments, profile, cuts, slide, plane_axis (0|1|2), plane_offset, clear_inner, clear_outer, fill, factor, iterations, merge_distance, dissolve_angle_deg, subdiv_cuts, subdiv_smooth, crease, poke_offset, lock_boundary. A failing operator changes nothing and returns the reason (e.g. empty selection). Returns mesh stats before/after and the new selection.",
            json!({
                "name": { "type": "string" },
                "params": { "type": "object", "additionalProperties": { "type": ["number", "boolean"] } }
            }), &["name"], ann(false, false, false),
        ),
        tool(
            "anigo_native_mesh_transform",
            "Numeric transform. In Edit mode it moves/rotates/scales the SELECTED vertices (pivot: the selection center, or pivot=\"origin\" for the world origin); in Object mode it transforms the active object relative to its current transform. kind=move: value=[dx,dy,dz] in world meters; rotate: value=[rx,ry,rz] degrees (XYZ Euler, world axes); scale: value=[sx,sy,sz] factors > 0 (or one number for uniform). One undo step. In Edit mode PROPORTIONAL editing applies when enabled (model_options, or a per-call `proportional` {enabled, radius, falloff, connected_only}): vertices within the radius follow with a falloff weight.",
            json!({
                "kind": { "type": "string", "enum": ["move", "rotate", "scale"] },
                "value": { "description": "[x,y,z] numbers (scale also accepts a single number)" },
                "pivot": { "type": "string", "enum": ["selection", "origin"] },
                "proportional": { "type": "object", "description": "per-call override of the proportional-editing options (see model_options)" }
            }), &["kind", "value"], ann(false, false, false),
        ),
        tool(
            "anigo_native_sculpt",
            "Sculpt mode driver (enters Sculpt mode on the active object). action: set_brush (brush + params), stroke, mask (what: invert|clear), smooth_all (iterations, smooth_strength), symmetrize (axis x|y|z, from positive|negative), voxel_remesh (optional voxel_size, voxel_smooth 0..10 Taubin iterations, shade_smooth=true to leave the faces SMOOTH-shaded: the remesh otherwise comes out flat-shaded, which shows as bands on a sphere), dyntopo (enabled, detail_px). Brush ids: draw, clay, clay_strips, smooth, flatten, fill, scrape, pinch, inflate, grab, snake_hook, nudge, crease, layer, elastic, mask, mask_smooth, mask_sharpen, mask_invert, mask_clear. Any action also accepts brush, radius_px (6..600, screen pixels), strength (0..2), spacing_pct, invert, mirror [x,y,z] booleans, radial [x,y,z] counts. stroke: path = [[u,v],…] in NORMALIZED coordinates of the active viewport (0,0 = top-left, 1,1 = bottom-right), or paths (up to 32 strokes), or world_path = [[x,y,z],…] projected through the active view's camera (best for 'sculpt exactly here'); each stroke is one undo step. Returns dabs applied and mesh stats; dabs=0 means the stroke missed the object: brushes only hit what is VISIBLE in the active view, so pass frame_view=true to frame the active object in that view first.",
            json!({
                "action": { "type": "string", "enum": ["set_brush", "stroke", "mask", "smooth_all", "symmetrize", "voxel_remesh", "dyntopo"] },
                "brush": { "type": "string" }, "radius_px": { "type": "number", "minimum": 6, "maximum": 600 }, "strength": { "type": "number", "minimum": 0, "maximum": 2 },
                "spacing_pct": { "type": "number", "minimum": 1, "maximum": 1000 }, "invert": { "type": "boolean" },
                "mirror": { "type": "array", "items": { "type": "boolean" }, "minItems": 3, "maxItems": 3 },
                "radial": { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 32 }, "minItems": 3, "maxItems": 3 },
                "path": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "maxItems": 512 },
                "paths": { "type": "array", "items": { "type": "array" }, "maxItems": 32 },
                "world_path": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "maxItems": 512 },
                "what": { "type": "string", "enum": ["invert", "clear"] }, "iterations": { "type": "integer", "minimum": 1, "maximum": 50 },
                "smooth_strength": { "type": "number", "minimum": 0, "maximum": 1 }, "axis": { "type": "string", "enum": ["x", "y", "z"] },
                "from": { "type": "string", "enum": ["positive", "negative"] }, "voxel_size": { "type": "number", "minimum": 0 },
                "enabled": { "type": "boolean" }, "detail_px": { "type": "number", "minimum": 2, "maximum": 200 },
                "frame_view": { "type": "boolean" }, "shade_smooth": { "type": "boolean" }, "voxel_smooth": { "type": "integer", "minimum": 0, "maximum": 10 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_model_history",
            "Undo/redo of the MODELING history (Modelagem objects, mesh edits, sculpt strokes). It is separate from anigo_native_undo, which acts on the core project (Personagem, Cenário…). steps 1..100.",
            json!({ "action": { "type": "string", "enum": ["undo", "redo"] }, "steps": { "type": "integer", "minimum": 1, "maximum": 100, "default": 1 } }), &["action"], ann(false, true, false),
        ),
        tool(
            "anigo_native_hair_info",
            "Reads the hair state of the native studio's Modelagem (Hair mode tool): head target, groups (id, name, strand count, symmetry, material), the selected strand/point, the new-strand template params, stroke and comb settings, bounce (physics) groups. include_strands=true lists each strand of every group (guide points in head-local space, length, params; capped by limit<=500).",
            json!({ "include_strands": { "type": "boolean", "default": false }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_hair_setup",
            "Hair groups and scene setup (enters Hair mode). op=target: make the active object (or object_index) the head. op=group + action: new (name), activate, rename (name), duplicate, flip (mirror copy), delete, symmetry (on), visible (on), material (patch: base_color/shade_color/highlight_color [r,g,b] 0..1, root_ao, normal_bias, name, uv_layout length_v|length_u) — group defaults to the active one. op=bounce + action: info, auto (groups 1..30, bones 1..16), set (index, patch: name, bones, fixed_point, stiffness, drag, gravity, hit_radius). op=regenerate rebuilds the hair meshes. op=export writes each group as a GLB into the user's LIBRARY (persistent side effect, like the Output button).",
            json!({
                "op": { "type": "string", "enum": ["target", "group", "bounce", "regenerate", "export"] },
                "action": { "type": "string" }, "object_index": { "type": "integer", "minimum": 0 }, "group": { "type": "integer", "minimum": 0 },
                "name": { "type": "string", "maxLength": 128 }, "on": { "type": "boolean" }, "index": { "type": "integer", "minimum": 0 },
                "groups": { "type": "integer", "minimum": 1, "maximum": 30 }, "bones": { "type": "integer", "minimum": 1, "maximum": 16 },
                "patch": { "type": "object" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_hair_strand",
            "Strand editing in Hair mode. action: select (group+strand, optional point), set_params (params = partial JSON merged over the strand's params; apply=\"strand\"|\"group\"|\"template\" — template changes what NEW strands get), simplify (epsilon), duplicate, delete, move_to_group (to_group), insert_point (after), remove_point (index), move_point (index + pos in head-local space or world_pos). Without group+strand the selected strand is used. params shape: {section:{kind: flat|arc|triangle|tube, width, thickness, twist_deg, curl_deg}, tip_taper 0..1, taper_power, resolution 1..64, flare, clump: null | {count, spread, tip_spread, noise, twist, points, length_variation, seed, strand_width}}. Unknown keys are rejected.",
            json!({
                "action": { "type": "string", "enum": ["select", "set_params", "simplify", "duplicate", "delete", "move_to_group", "insert_point", "remove_point", "move_point"] },
                "group": { "type": "integer", "minimum": 0 }, "strand": { "type": "integer", "minimum": 0 }, "point": { "type": "integer", "minimum": 0 },
                "params": { "type": "object" }, "apply": { "type": "string", "enum": ["strand", "group", "template"] }, "epsilon": { "type": "number" },
                "to_group": { "type": "integer", "minimum": 0 }, "after": { "type": "integer", "minimum": 0 }, "index": { "type": "integer", "minimum": 0 },
                "pos": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "world_pos": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_hair_stroke",
            "Draws or combs hair like the mouse does (enters Hair mode; needs the head target). op=draw: one stroke becomes a strand (and its mirror if the group has symmetry), using path = [[u,v],…] normalized 0..1 of the ACTIVE viewport, world_path = [[x,y,z],…] projected through the active view, or local_path = points in head-local space; optional group, params (template patch, same shape as hair_strand.params) and stroke ({resample_step, smooth_iters, rdp_epsilon, lift_root, lift_tip, max_points, min_length}). The stroke must touch the head. op=comb: same path forms plus radius_px (10..400), strength (0.02..1), all_groups; returns segments_combed.",
            json!({
                "op": { "type": "string", "enum": ["draw", "comb"] },
                "path": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "maxItems": 512 },
                "world_path": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "maxItems": 512 },
                "local_path": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "maxItems": 512 },
                "group": { "type": "integer", "minimum": 0 }, "params": { "type": "object" }, "stroke": { "type": "object" },
                "radius_px": { "type": "number" }, "strength": { "type": "number" }, "all_groups": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_weights_info",
            "Reads per-vertex weights of the ACTIVE object: stats for the four NPR channels (ao, shadow_shift, outline_width, spec_rim: min/mean/max) and skin weights (weighted vertices, max/mean influences, normalization error, vertices per joint). vertices=[ids] (<=500) also returns each vertex's position, NPR values and skin influences.",
            json!({ "vertices": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "maxItems": 500 } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_npr_paint",
            "NPR vertex-attribute painting on the active object (the 'Paint attribute' tool): channel ao|shadow_shift|outline_width|spec_rim, values 0..1. action: paint (spherical brush: center in object space or world_center, radius in object/world units, falloff smooth|linear|sphere|root|sharp|constant, value, blend mix|add|subtract|multiply, strength 0..1), fill (verts or use_selection), smooth (iterations, factor), gradient (p0, p1, v0, v1 in object space), invert, bake_ao (samples, max_dist, bias, seed, strength — writes the AO channel). Each call is one undo step of the MODELING history.",
            json!({
                "action": { "type": "string", "enum": ["paint", "fill", "smooth", "gradient", "invert", "bake_ao"] },
                "channel": { "type": "string", "enum": ["ao", "shadow_shift", "outline_width", "spec_rim"] },
                "center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "world_center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "radius": { "type": "number" }, "falloff": { "type": "string" }, "value": { "type": "number" }, "blend": { "type": "string" }, "strength": { "type": "number" },
                "verts": { "type": "array", "items": { "type": "integer", "minimum": 0 } }, "use_selection": { "type": "boolean" },
                "iterations": { "type": "integer" }, "factor": { "type": "number" },
                "p0": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "p1": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "v0": { "type": "number" }, "v1": { "type": "number" },
                "samples": { "type": "integer" }, "max_dist": { "type": "number" }, "bias": { "type": "number" }, "seed": { "type": "integer" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_skin_weights",
            "Skin-weight editing on the active object (up to 4 influences per vertex; joint = integer index). action: assign (joint, weight, vertex | vertices | use_selection, normalize), paint (spherical brush: center/world_center, radius, falloff, joint, value, blend, strength, normalize), normalize (all vertices), limit (max 1..4), smooth (verts|use_selection, iterations, factor), transfer (from_object index: copies weights from ANOTHER object by closest surface point, optional max_distance), auto_envelope (bones: [{joint, head, tail, radius_head, radius_tail}], falloff, space object|world), mirror (axis, from positive|negative, joint_map [[from,to],…], tolerance). One undo step of the MODELING history each; invalid input changes nothing. Read results with anigo_native_weights_info.",
            json!({
                "action": { "type": "string", "enum": ["assign", "paint", "normalize", "limit", "smooth", "transfer", "auto_envelope", "mirror"] },
                "joint": { "type": "integer", "minimum": 0, "maximum": 65535 }, "weight": { "type": "number" }, "normalize": { "type": "boolean" },
                "vertex": { "type": "integer", "minimum": 0 }, "vertices": { "type": "array", "items": { "type": "integer", "minimum": 0 } }, "verts": { "type": "array", "items": { "type": "integer", "minimum": 0 } }, "use_selection": { "type": "boolean" },
                "center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "world_center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "radius": { "type": "number" }, "falloff": { "type": ["string", "number"], "description": "paint: smooth|linear|sphere|root|sharp|constant; auto_envelope: number >= 0 (distance beyond the capsule as a fraction of its radius)" }, "value": { "type": "number" }, "blend": { "type": "string" }, "strength": { "type": "number" },
                "max": { "type": "integer", "minimum": 1, "maximum": 4 }, "iterations": { "type": "integer" }, "factor": { "type": "number" },
                "from_object": { "type": "integer", "minimum": 0 }, "max_distance": { "type": "number" },
                "bones": { "type": "array", "items": { "type": "object" }, "maxItems": 256 }, "space": { "type": "string", "enum": ["object", "world"] },
                "axis": { "type": "string", "enum": ["x", "y", "z"] }, "from": { "type": "string", "enum": ["positive", "negative"] },
                "joint_map": { "type": "array", "items": { "type": "array", "items": { "type": "integer" } } }, "tolerance": { "type": "number" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_modifiers",
            "Non-destructive modifier stack of the active object. action: list, add (kind: subsurf|mirror|solidify|array|weld|edge_split|displace|decimate|triangulate, optional params = partial JSON over the type's defaults — see list for the shape — and name), set (index, params, enabled_viewport, enabled_render, name), remove (index), move (from, to), apply (bakes the evaluated result into the base mesh and clears the stack). A stack that would fail to evaluate is rejected before anything changes. Returns the stack and the evaluated mesh size.",
            json!({
                "action": { "type": "string", "enum": ["list", "add", "set", "remove", "move", "apply"] },
                "kind": { "type": "string" }, "params": { "type": "object" }, "name": { "type": "string", "maxLength": 64 },
                "index": { "type": "integer", "minimum": 0 }, "from": { "type": "integer", "minimum": 0 }, "to": { "type": "integer", "minimum": 0 },
                "enabled_viewport": { "type": "boolean" }, "enabled_render": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_normals",
            "Normals tools on the active object. action: report (smoothness metrics, auto-smooth angle), auto_smooth (angle_deg 0..180), bake_custom (freeze the computed normals as custom), clear_custom, reset, average (mode face_area|corner_angle|uniform), smooth (iterations, factor), transfer (from_object index, mapping closest_surface|closest_vertex|closest_corner, mix 0..1, max_distance). reset/average/smooth/transfer accept use_selection to restrict to the selected faces/vertices (Edit mode selection); otherwise the whole mesh. One undo step each.",
            json!({
                "action": { "type": "string", "enum": ["report", "auto_smooth", "bake_custom", "clear_custom", "reset", "average", "smooth", "transfer"] },
                "angle_deg": { "type": "number" }, "use_selection": { "type": "boolean" }, "mode": { "type": "string" },
                "iterations": { "type": "integer" }, "factor": { "type": "number" }, "from_object": { "type": "integer", "minimum": 0 },
                "mapping": { "type": "string" }, "mix": { "type": "number" }, "max_distance": { "type": "number" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_uv",
            "UV tools on the active object (enters Edit mode; selection scopes the faces like the UV panel). action: report (islands, stretch, overlaps, texel density…), mark_seam, clear_seam, clear_all_seams, seams_by_angle, seam_path, auto_seams_character, seams_from_sharp, unwrap, minimize_stretch, pack, normalize_density, project_planar, project_box, project_cylinder, project_sphere, smart_project, project_from_view, flip_u, flip_v, rotate_90. params (all optional, remembered like the panel): tex_size, seam_angle_deg, char_angle_deg, method lscm|lscm_arap, margin_px, correct_aspect, aspect, fill_holes, stretch_iters, pack_margin_px, pack_rotate, planar_axis 0|1|2, box_cross, smart_angle_deg. Returns the UV report after the action.",
            json!({ "action": { "type": "string" }, "params": { "type": "object" } }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_material",
            "Toon material editor (the Shading workspace without the window). op: info (source, targets list, the current target's full material JSON, built-in presets), source (value personagem|cenario|modelagem), target (index from info.targets), set (patch = partial JSON over the material, e.g. {\"shadow_threshold\":0.6,\"base_color\":[1,0.8,0.7,1],\"outline_width\":0.004}; unknown fields are rejected and listed), preset (id from info.presets; keeps the target's name), reset, copy, paste, history (action undo|redo — acts on the history of the current source). Each edit is ONE undo step; the core validates values.",
            json!({
                "op": { "type": "string", "enum": ["info", "source", "target", "set", "preset", "reset", "copy", "paste", "history"] },
                "value": { "type": "string" }, "index": { "type": "integer", "minimum": 0 }, "patch": { "type": "object" }, "id": { "type": "string" },
                "action": { "type": "string", "enum": ["undo", "redo"] }, "fields_only": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_animation",
            "Animation workspace driver (switches to Animação). op=info: fps, current frame, playing, clips (id, name, keys, range, loop), the active clip's channels and key frames, auto-key. op=clip + action: new, duplicate, select, rename (name), remove, playback (loop_mode once|loop|ping_pong, extrapolation constant|linear, hold_step 1..12), range ([start,end] or null), fps (fps, retime) — clip id optional (default active). op=transport + action: goto (frame), step (n), play, pause, home, end, speed (value -4..4), next_key, prev_key. op=key + action: insert (frame?, bones? — indices or names; default: selected bones, else the whole body; records the CURRENT pose), delete (same targeting), autokey (on). Edits are core history steps (anigo_native_undo). For per-key edits (move, interpolation, markers, whole clips) use anigo_native_apply_command with kind edit_animation.",
            json!({
                "op": { "type": "string", "enum": ["info", "clip", "transport", "key"] }, "action": { "type": "string" }, "id": { "type": "string" },
                "name": { "type": "string", "maxLength": 128 }, "frame": { "type": "number", "minimum": 0 }, "n": { "type": "integer" }, "value": { "type": "number" },
                "loop_mode": { "type": "string" }, "extrapolation": { "type": "string" }, "hold_step": { "type": "integer", "minimum": 1, "maximum": 12 },
                "range": { "type": ["array", "null"], "items": { "type": "integer" } }, "fps": { "type": "number" }, "retime": { "type": "boolean" },
                "bones": { "type": "array", "items": {} }, "on": { "type": "boolean" }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_render_info",
            "Reads the Render workspace's queue state: output preferences (folder, name template, counter, overwrite, lock_aspect, sensor, guides), the folder status and the NEXT file names it would write, the project's output settings and enabled AOVs, pass graph, the render camera (name, linked scene camera, scene cameras, eye/target/fov), the job queue (status, progress, stage, files, notes, errors) and the history of rendered images (beauty entries; limit<=500), ffmpeg detection and the ffmpeg command line for a sequence. To enqueue a still use anigo_native_render_frame; to change resolution/passes/lens use anigo_native_apply_command (set_render_output, set_render_settings, set_camera).",
            json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_render_prefs",
            "Edits the Render preferences (machine-level, not project data; all-or-nothing): patch = { dir (ABSOLUTE output folder inside the MCP sandbox: Documents/ANIGO, ./baselines, ./tmp, ANIGO_MCP_ALLOWED_DIRS; empty = suggested folder), template (1..200 chars, tokens {project} {camera} {w} {h} {fmt} {pass} {n:04} {date} {time}, no path separators), counter (1..999999), overwrite, lock_aspect, sensor (index, see render_info.sensors), queue_open, guides: {thirds, center, action_safe, title_safe, dim_outside, info} }. Unknown fields are rejected. Returns the new render_info.",
            json!({ "patch": { "type": "object" } }), &["patch"], ann(false, false, true),
        ),
        tool(
            "anigo_native_render_camera",
            "The render camera, like the Render > Camera panel. action: source (node_id of a scene camera, or project=true for the project camera; linking applies it as ONE core undo step), reapply (re-read the linked scene camera), from_view (copy the Cenario viewport camera, eye/target/fov), frame (selection=false frames the whole scene, true the selected nodes; keeps the look direction and uses the OUTPUT aspect). Returns render_info + history_changed.",
            json!({
                "action": { "type": "string", "enum": ["source", "reapply", "from_view", "frame"] },
                "node_id": { "type": "string", "maxLength": 128 }, "project": { "type": "boolean" }, "selection": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_render_job",
            "Render queue control. action: cancel (job_id; a running job finishes its current stage and discards the frame), resume (job_id of a cancelled/failed job; sequences skip frames already on disk), remove (job_id of a finished job), clear (drops all finished jobs). Enqueue with anigo_native_render_frame. Returns render_info.",
            json!({ "action": { "type": "string", "enum": ["cancel", "resume", "remove", "clear"] }, "job_id": { "type": "integer", "minimum": 0 } }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_render_history",
            "Rendered images recorded in the output folder. action: list (render_info with the history; limit<=500), path (file -> absolute path, exists), to_library (file -> imports the image into the user's Library with tags render/autoral; persistent side effect on the real Library).",
            json!({ "action": { "type": "string", "enum": ["list", "path", "to_library"] }, "file": { "type": "string", "maxLength": 260 }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_library_browse",
            "Asset Library browser. op=info: home, UI filters, selection, counts (all/favorites/trash/missing/duplicates), kinds, roots (locations), collections, tag tree, background jobs (import progress + log, scan, verify, rebuild). op=query: query = {text (FTS: words, \"phrases\", -exclude, name:x tag:x kind:x), kinds[model|character|outfit|hair|accessory|material|texture|pose|animation|hdri|scene|prefab], tags[] (ALL), any_tags[] (ANY), collection, favorite, min_rating, formats[], root, storage[managed|linked], min_tris, max_tris, state[active|trashed|missing|corrupt|any], sort{by:name|imported_at|modified_at|size|rating|tris|relevance, desc}, limit<=500, after (cursor)} -> assets + total + next. op=filter: patch = {section: all|favorites|trash|missing|{collection:id}|{tag:path}|{root:id}, text, kinds, min_rating, sort} changes what the Library workspace SHOWS (same as the sidebar/search box) and returns the first page. op=select: asset_ids | all_matching=true | none=true (the UI selection). op=detail: asset_id -> row, manifest, tags, collections, dependencies, used_by, files. op=edit_in_modeling: asset_id -> opens the model as an editable object in Modelagem (switches workspace). To place an asset in the scene use anigo_native_place_asset.",
            json!({
                "op": { "type": "string", "enum": ["info", "query", "filter", "select", "detail", "edit_in_modeling"] },
                "query": { "type": "object" }, "patch": { "type": "object" }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
                "asset_id": { "type": "string", "maxLength": 64 }, "asset_ids": { "type": "array", "items": { "type": "string", "maxLength": 64 }, "maxItems": 500 },
                "all_matching": { "type": "boolean" }, "none": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_library_edit",
            "Edits Library metadata (asset_ids, or the UI selection when omitted; validated before anything is written). op=meta: patch = {favorite, rating 0..5, name (ONE asset), description, author, license, source_url, kind, tags_add[], tags_remove[], tags_set[]} (tags are hierarchical paths like \"personagem/cabelo\"). op=collections + action: list, create (name), create_smart (name, query as in library_browse), update_smart (collection_id, query), rename (collection_id, name), delete (collection_id; assets are kept), add/remove (collection_id, asset_ids). op=tags + action: list, rename (path, new), delete (path), color (path, color), prune (drops unused tags). Unknown fields are rejected.",
            json!({
                "op": { "type": "string", "enum": ["meta", "collections", "tags"] }, "asset_ids": { "type": "array", "items": { "type": "string", "maxLength": 64 }, "maxItems": 500 },
                "patch": { "type": "object" }, "action": { "type": "string" }, "collection_id": { "type": "integer" }, "name": { "type": "string", "maxLength": 120 },
                "query": { "type": "object" }, "path": { "type": "string", "maxLength": 200 }, "new": { "type": "string", "maxLength": 200 }, "color": { "type": "string", "maxLength": 16 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_library_trash",
            "Library trash. action: trash (asset_ids or the UI selection; assets that others depend on are reported in `blocked` unless force=true; reversible), restore (asset_ids), purge (asset_ids already in the trash; PERMANENT, needs confirm=true; managed files are deleted, linked files outside the Library are never touched), empty (empties the trash; PERMANENT, needs confirm=true; older_than_secs optional).",
            json!({
                "action": { "type": "string", "enum": ["trash", "restore", "purge", "empty"] }, "asset_ids": { "type": "array", "items": { "type": "string", "maxLength": 64 }, "maxItems": 500 },
                "force": { "type": "boolean" }, "confirm": { "type": "boolean" }, "older_than_secs": { "type": "integer", "minimum": 0 }
            }), &["action"], ann(false, true, false),
        ),
        tool(
            "anigo_native_library_import",
            "Imports files or folders into the Library (GLB/glTF/VRM, OBJ, FBX, images...). paths = absolute files/folders inside the MCP read sandbox (Documents/ANIGO, ./baselines, ./tmp, ANIGO_MCP_ALLOWED_DIRS, cwd). options = {mode managed (copy, default) | linked, recursive, kind, name (one path only), tags[], description, author, license, source_url, duplicates skip|allow, collection (id), root (id), replace (asset id: import as a NEW REVISION)}. wait=false (default) queues a background job - follow it with library_browse op=info (jobs.import, jobs.import_log); wait=true imports small FILES (<=64 MB total) synchronously and returns the imported ids. cancel=true cancels the running import queue.",
            json!({
                "paths": { "type": "array", "items": { "type": "string", "maxLength": 1024 }, "maxItems": 64 }, "options": { "type": "object" },
                "wait": { "type": "boolean" }, "cancel": { "type": "boolean" }
            }), &[], ann(false, false, false),
        ),
        tool(
            "anigo_native_library_roots",
            "Library locations (roots). action: list, refresh (re-check online/offline), add (path inside the sandbox; mode linked (default, read-only reference; scanned right away unless scan=false) | managed; name), remove (root_id; ONLY unregisters - files are never deleted; root 1 is the Library itself), scan (root_id, or all linked roots when omitted; background - see library_browse op=info).",
            json!({
                "action": { "type": "string", "enum": ["list", "refresh", "add", "remove", "scan"] }, "root_id": { "type": "integer", "minimum": 1 },
                "path": { "type": "string", "maxLength": 1024 }, "mode": { "type": "string", "enum": ["linked", "managed"] }, "name": { "type": "string", "maxLength": 120 }, "scan": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_library_maintenance",
            "Library health. action: duplicates (groups with the same content; limit<=500), missing (assets whose file is gone/offline), verify (background; rehash=true by default, fix=true applies safe index corrections and NEVER deletes files; results in library_browse op=info jobs.last_verify), rebuild (background: rebuilds the index from the manifests), cancel_imports.",
            json!({
                "action": { "type": "string", "enum": ["duplicates", "missing", "verify", "rebuild", "cancel_imports"] }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
                "rehash": { "type": "boolean" }, "fix": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_pose_info",
            "Reads the Posing state (character pose, 25 bones): per bone index, name, parent, local Euler XYZ (deg) and quaternion, whether it is posed, joint position in CHARACTER and WORLD space and the joint limit with the current swing/twist and `within`; hips/root translation, finger curls, balance (center of mass, inside support polygon, lowest contact y), the bone selection, ALL editing options (limit profile, symmetry, snap, per-limb IK, foot lock, auto-ground, gaze, library blend), clipboard, the pose assets of the Library and the preset names. compact=true omits the per-bone list.",
            json!({ "compact": { "type": "boolean" } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_pose_options",
            "Editing options of the Posing tools (UI state, not undoable; all-or-nothing). patch = { profile: human|anime|free (joint limits), symmetry (editing a bone also edits its mirrored bone), snap_deg (null or 0..90), space_local, limb_ik {left_arm,right_arm,left_leg,right_leg: bool}, spine_ik, foot_lock, auto_ground, show {mesh,skeleton,balance,labels,limits}, gaze {enabled,target [x,y,z] character space,weight 0..1}, blend 0..1.5, apply_only_selected, hand_side left|right|both, lib_name, lib_only_selected }. Unknown fields are rejected.",
            json!({ "patch": { "type": "object" } }), &["patch"], ann(false, false, true),
        ),
        tool(
            "anigo_native_pose_select",
            "Bone selection of the Posing workspace (also the mask of copy/reset/mirror/library). bones = indices or names (see pose_info); mode set (default) | add | remove | toggle; all=true selects every bone except Root; none=true clears; active = which selected bone is the active one.",
            json!({
                "bones": { "type": "array", "items": { "type": ["integer", "string"] }, "maxItems": 64 }, "mode": { "type": "string", "enum": ["set", "add", "remove", "toggle"] },
                "all": { "type": "boolean" }, "none": { "type": "boolean" }, "active": { "type": ["integer", "string"] }
            }), &[], ann(false, false, true),
        ),
        tool(
            "anigo_native_pose_bone",
            "FK rotation of ONE bone (default: the active bone), subject to the joint-limit profile and symmetry from pose_options. ONE core undo step each (anigo_native_undo). action: set_euler (euler_deg [x,y,z] local degrees), set_quat (quat [x,y,z,w] local), rotate_world (axis [x,y,z] in WORLD space, angle_deg — like the gizmo ring), reset (bone back to rest). Returns pose_info + changed + history_steps_added (changed=false when a limit clamps the edit to the current value).",
            json!({
                "action": { "type": "string", "enum": ["set_euler", "set_quat", "rotate_world", "reset"] }, "bone": { "type": ["integer", "string"] },
                "euler_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "quat": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                "axis": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "angle_deg": { "type": "number" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_pose_edit",
            "Whole-body pose edits (each ONE core undo step; with Animation auto-key they also insert keys). op: translate (part hips|root; value [x,y,z] absolute metres OR delta; hips honours foot_lock/auto_ground, overridable here), ik (limb left_arm|right_arm|left_leg|right_leg; target [x,y,z] and optional pole, space world (default) | character), gaze (action apply (target, weight 0..1: head+neck look-at) | center | front), ground (rest the lowest contact point on the floor), mirror (selected bones, or the whole pose), copy, paste (flipped), reset (scope selection | all | bones + bones), preset (body: t|a|standing|sitting|walking|action), hands (side left|right|both + preset open|fist|point|v|hold|relaxed, or curls [thumb,index,middle,ring,little] 0..1).",
            json!({
                "op": { "type": "string", "enum": ["translate", "ik", "gaze", "ground", "mirror", "copy", "paste", "reset", "preset", "hands"] },
                "part": { "type": "string", "enum": ["hips", "root"] }, "value": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "foot_lock": { "type": "boolean" }, "auto_ground": { "type": "boolean" }, "limb": { "type": "string", "enum": ["left_arm", "right_arm", "left_leg", "right_leg"] },
                "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "pole": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "space": { "type": "string", "enum": ["world", "character"] },
                "action": { "type": "string", "enum": ["apply", "center", "front"] }, "weight": { "type": "number", "minimum": 0, "maximum": 1 },
                "flipped": { "type": "boolean" }, "scope": { "type": "string", "enum": ["selection", "all", "bones"] },
                "bones": { "type": "array", "items": { "type": ["integer", "string"] }, "maxItems": 64 }, "body": { "type": "string" },
                "side": { "type": "string", "enum": ["left", "right", "both"] }, "preset": { "type": "string" },
                "curls": { "type": "array", "items": { "type": "number", "minimum": 0, "maximum": 1 }, "minItems": 5, "maxItems": 5 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_pose_library",
            "Pose assets of the Library. action: list (refreshes pose_info.pose_assets), save (name; only_selected=true saves just the selected bones; same name = NEW REVISION; writes to the user's REAL Library), apply (index OR name from pose_info.pose_assets; blend 0..1.5, flipped, only_selected). Apply is ONE core undo step.",
            json!({
                "action": { "type": "string", "enum": ["list", "save", "apply"] }, "name": { "type": "string", "maxLength": 120 }, "index": { "type": "integer", "minimum": 0 },
                "only_selected": { "type": "boolean" }, "flipped": { "type": "boolean" }, "blend": { "type": "number", "minimum": 0, "maximum": 1.5 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_light_info",
            "Reads the Lighting state: sun (direction, azimuth/elevation deg, colour, temperature K, intensity), shadow tint/saturation, ambient hemisphere (intensity, sky, ground), cast-shadow settings, the character's terminator and rim material values, the camera-follow state, the last rig, the rig catalogue and history flags.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_light_set",
            "Edits the lighting like the Iluminacao sliders; ONE core undo step even when light AND material change (Batch). patch = { azimuth_deg -180..180, elevation_deg -90..90 (or direction [x,y,z]), color [r,g,b] 0..1 OR kelvin 1000..40000, intensity 0..10, shadow_color, shadow_saturation 0..2, ambient_intensity 0..2, ambient_sky, ambient_ground, shadow {enabled, strength 0..1, hardness 0..1, bias 0..0.25, normal_bias 0..6, resolution 512|1024|2048|4096, distance 0=auto}, material {shadow_threshold, shadow_smoothness, rim_intensity 0..3, rim_spread, rim_color [r,g,b(,a)]} }. Unknown fields are rejected; with the light locked to the camera azimuth/elevation set the RELATIVE angles instead. Returns light_info + changed + history_steps_added.",
            json!({ "patch": { "type": "object" } }), &["patch"], ann(false, false, false),
        ),
        tool(
            "anigo_native_light_control",
            "Other Lighting actions. op=rig (id: three_point|… see light_info.rigs; one undo step: sun + ambient + shadow tint + rim), reset_sun, follow (on: lock the light to the camera; rel_az/rel_el relative angles; bake=true writes the camera-relative direction into the project), view (show_gizmo, frame=true frames the character).",
            json!({
                "op": { "type": "string", "enum": ["rig", "reset_sun", "follow", "view"] }, "id": { "type": "string", "maxLength": 64 }, "on": { "type": "boolean" },
                "rel_az": { "type": "number" }, "rel_el": { "type": "number" }, "bake": { "type": "boolean" }, "show_gizmo": { "type": "boolean" }, "frame": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_model_options",
            "Modeling options (UI state, all-or-nothing): patch = { proportional {enabled, radius 0.01..100, falloff smooth|sphere|root|inverse_square|sharp|linear|constant|random, connected_only}, xray, snap_enabled }. The proportional settings are the ones mesh_transform uses in Edit mode (like the O key), unless that call passes its own `proportional`.",
            json!({ "patch": { "type": "object" } }), &["patch"], ann(false, false, true),
        ),
        tool(
            "anigo_native_anim_keys",
            "Fine key editing on the ACTIVE clip (each ONE core undo step). Keys are addressed as {bone: \"LeftElbow\", frame} or {target: {kind:\"root\"|\"hips\"|\"hand\"|\"bone\"|\"sun\"|\"camera\"|\"node_translation\"…, …}, frame} exactly as shown in animation info. action: select (items[] | all | frame_range [a,b] (+ optional bone) | none; mode set|add), set_interp (interp step|linear|ease|ease_in|ease_out|bezier, c [x1,y1,x2,y2] for bezier) on the selection, move (delta frames), duplicate (delta), mirror (L<->R bones), delete, copy, paste (frame, mirrored), set_value (bone|target + frame + value: scalar | [x,y,z] | curls[5] | euler_deg [x,y,z] or quat [x,y,z,w] for rotations; optional interp). Returns the animation info incl. selected_keys.",
            json!({
                "action": { "type": "string", "enum": ["select", "set_interp", "move", "duplicate", "mirror", "delete", "copy", "paste", "set_value"] },
                "items": { "type": "array", "items": { "type": "object" }, "maxItems": 5000 }, "all": { "type": "boolean" }, "none": { "type": "boolean" },
                "frame_range": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "mode": { "type": "string", "enum": ["set", "add"] },
                "bone": { "type": "string", "maxLength": 64 }, "target": { "type": "object" }, "frame": { "type": "integer", "minimum": 0 },
                "interp": { "type": "string", "enum": ["step", "linear", "ease", "ease_in", "ease_out", "bezier"] }, "c": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                "delta": { "type": "integer" }, "mirrored": { "type": "boolean" }, "value": {}, "euler_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "quat": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_anim_more",
            "Other Animation workspace features. op=marker (action list|toggle|add|rename|remove; frame, name), scene_keys (what sun|camera|node + node_id, frame; keys the scene's sun / scene camera / a node's TRS at a frame), onion (patch {enabled, before 0..6, after 0..6, step 1..12, on_keys}), physics (action enable {on} | config {patch {hair, skirt, pre_roll_s}} | bake {dir: absolute folder} -> writes <clip>.physics.json), file (action import|export; path ABSOLUTE; kind glb|vrma|bvh|clip; BVH/GLB/VRMA/clip-json), library (action list|save {name}|apply {index|name, at_playhead}; save writes to the Library), options (patch {interp (default for NEW keys) step|linear|ease|bezier, quantize, only_selected_bones, retime}), sequence (patch {custom, start, end, step, template} and render=true: queues a sequence render of the clip through the Render queue — follow it with render_info/render_status).",
            json!({
                "op": { "type": "string", "enum": ["marker", "scene_keys", "onion", "physics", "file", "library", "options", "sequence"] },
                "action": { "type": "string" }, "frame": { "type": "integer", "minimum": 0 }, "name": { "type": "string", "maxLength": 128 },
                "what": { "type": "string", "enum": ["sun", "camera", "node"] }, "node_id": { "type": "string", "maxLength": 128 }, "patch": { "type": "object" },
                "on": { "type": "boolean" }, "dir": { "type": "string", "maxLength": 1024 }, "path": { "type": "string", "maxLength": 1024 },
                "kind": { "type": "string", "enum": ["glb", "vrma", "bvh", "clip"] }, "index": { "type": "integer", "minimum": 0 }, "at_playhead": { "type": "boolean" },
                "render": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_material_view",
            "Shading preview/UI options (not undoable). op=view: patch {mode ball|object|both, background 0..2, turntable, light {azimuth_deg, elevation_deg, follows_camera}, focus_group base|ramp|specular|rim|matcap|outline|face|emission|eye (opens that tab), palette_open, link_shade, recipe {hue_shift_deg, saturation, value}, harmony, frame}. op=compare (A/B): action on|off|set_a|swap|revert (swap/revert apply material A = ONE undo step).",
            json!({
                "op": { "type": "string", "enum": ["view", "compare"] }, "patch": { "type": "object" },
                "action": { "type": "string", "enum": ["on", "off", "set_a", "swap", "revert"] }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_material_tools",
            "Shading panel tools beyond a raw material patch (each edit = ONE history step of the current source). op=param: action list {group} | reset_param {id} | reset_group {group} | anime_features. op=palette: action derive_shade {recipe} (shade colour from the base, HSV recipe) | hue_turn {deg} (rotates base+shade hue) | harmony {color, harmony, add_to_palette} | add {color} | remove {index} | apply {index, to base|shade} | save | reload | list (project palette persisted in the shading palettes folder). op=ramp (2D ramp editor): action select_row {row 0..3} | customize {row} (copies the contract ramp into editable stops) | set_stops {row, stops [[pos,val]…2..6], constant} | clear_row {row} | select_stop {index}.",
            json!({
                "op": { "type": "string", "enum": ["param", "palette", "ramp"] }, "action": { "type": "string" }, "group": { "type": "string" }, "id": { "type": "string", "maxLength": 64 },
                "recipe": { "type": "object" }, "deg": { "type": "number" }, "color": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "harmony": { "type": "string" }, "add_to_palette": { "type": "boolean" },
                "index": { "type": "integer", "minimum": 0 }, "to": { "type": "string", "enum": ["base", "shade"] }, "row": { "type": "integer", "minimum": 0, "maximum": 3 },
                "stops": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "maxItems": 6 }, "constant": { "type": "boolean" }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_material_library",
            "Shading <-> Library. op=presets: action list | save {name} (saves the current material AND its texture refs as a Material asset; same name = new revision; writes to the Library) | apply {asset_id | name} (ONE undo step; keeps the target's name). op=texture: action list {text} | choose {slot, asset_id} | clear {slot} | face_procedural; slots base, shade, shade2, ramp, ilm, matcap, emission, outline_width (bookkeeping saved with presets) and face_sdf (LIVE: needs the preview renderer).",
            json!({
                "op": { "type": "string", "enum": ["presets", "texture"] }, "action": { "type": "string" }, "name": { "type": "string", "maxLength": 120 },
                "asset_id": { "type": "string", "maxLength": 64 }, "slot": { "type": "string", "maxLength": 32 }, "text": { "type": "string", "maxLength": 256 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_scene_info",
            "Reads the Cenario (scene) workspace: nodes (id, name, kind, parent, visible, locked, folder, tags, local TRS in degrees, world bounds), selection, gizmo tool/space, snap settings, view (shading, guides, camera, orthographic kind), clipboard, scene cameras, history depth, project path. limit<=2000 nodes.",
            json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 2000 } }), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_scene_select",
            "Scene node selection (UI state; hidden/locked nodes cannot be picked). ids = node ids from scene_info; mode set (default)|add|remove|toggle; or what all|invert|children|same; or none=true.",
            json!({ "ids": { "type": "array", "items": { "type": "string", "maxLength": 128 }, "maxItems": 2000 }, "mode": { "type": "string", "enum": ["set", "add", "remove", "toggle"] }, "what": { "type": "string", "enum": ["all", "invert", "children", "same"] }, "none": { "type": "boolean" } }), &[], ann(false, false, true),
        ),
        tool(
            "anigo_native_scene_edit",
            "Scene editing, like the Cenario UI (edits are core undo steps). op=tool (tool select|translate|rotate|scale, space world|local), snap (patch {grid_on, grid_m, rot_on, rot_deg, scale_on, scale_step, surface_on, align_to_normal, surface_offset, absolute, scale_lock}), view (patch {shading cel|solid|wire, game_view, camera_view, speed, view_kind perspective|front|back|right|left|top|bottom, guides {thirds,center,safe,frame}}; frame all|selection), visibility (action hide|show|toggle|lock|unlock|unhide_all; ids or selection), outliner (action rename {id,name} | delete | duplicate | group | ungroup | reparent {ids, parent|null} | folder {ids, folder} | copy | cut | paste | snap_floor | snap_origin_grid; ids or selection), place (kind cube|sphere|plane|cylinder|cone|camera|group, position [x,y,z]; Library assets: anigo_native_place_asset), transform (id + translation [x,y,z] / rotation_deg [x,y,z] / scale: ABSOLUTE LOCAL values), camera (action use_as_render_camera|align_to_view, id), history (depth: jump to that undo depth).",
            json!({
                "op": { "type": "string", "enum": ["tool", "snap", "view", "visibility", "outliner", "place", "transform", "camera", "history"] },
                "tool": { "type": "string", "enum": ["select", "translate", "rotate", "scale"] }, "space": { "type": "string", "enum": ["world", "local"] }, "patch": { "type": "object" },
                "frame": { "type": "string", "enum": ["all", "selection"] }, "action": { "type": "string" }, "ids": { "type": "array", "items": { "type": "string", "maxLength": 128 }, "maxItems": 2000 }, "id": { "type": "string", "maxLength": 128 },
                "name": { "type": "string", "maxLength": 120 }, "parent": { "type": ["string", "null"] }, "folder": { "type": "string", "maxLength": 120 }, "kind": { "type": "string" },
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, "scale": {}, "depth": { "type": "integer", "minimum": 0 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_scene_project",
            "The .anigo project file. action: info, save (path ABSOLUTE inside the MCP sandbox; extension .anigo added), new (DISCARDS the current project: needs discard_current=true), open (path; DISCARDS the current project: needs discard_current=true; the undo history restarts).",
            json!({ "action": { "type": "string", "enum": ["info", "save", "new", "open"] }, "path": { "type": "string", "maxLength": 1024 }, "discard_current": { "type": "boolean" } }), &["action"], ann(false, true, false),
        ),
        tool(
            "anigo_native_char_info",
            "Reads the Personagem workspace: base gender, dimorphism, somatotype, macro proportions (+ranges), active preset, preset ids, every MODIFIED morph slider (value, default, range), somatotype shortcuts, preview camera, history flags.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_char_sliders",
            "The 157 morph sliders. action: list (region body|face, query text, only_modified) | set (values {slider_id: value}, validated against each slider's min/max; ONE undo step; values already at the target are skipped) | reset (ids [] or all=true).",
            json!({
                "action": { "type": "string", "enum": ["list", "set", "reset"] }, "region": { "type": "string", "enum": ["body", "face"] }, "query": { "type": "string", "maxLength": 64 },
                "only_modified": { "type": "boolean" }, "values": { "type": "object" }, "ids": { "type": "array", "items": { "type": "string", "maxLength": 64 }, "maxItems": 157 }, "all": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_char_edit",
            "Other character edits (core undo steps). op=proportions (patch {height, head_scale, head_ratio, shoulders, legs, arms, neck, torso}, ranges in char_info), gender (gender male|female, OR dimorphism 0..1), somatotype (endomorph/mesomorph/ectomorph 0..1, or shortcut key from char_info), preset (id), reset_all, view (patch {preset front|side|back|face, yaw, pitch, turntable, search, frame}).",
            json!({
                "op": { "type": "string", "enum": ["proportions", "gender", "somatotype", "preset", "reset_all", "view"] }, "patch": { "type": "object" }, "gender": { "type": "string", "enum": ["male", "female"] },
                "dimorphism": { "type": "number", "minimum": 0, "maximum": 1 }, "endomorph": { "type": "number" }, "mesomorph": { "type": "number" }, "ectomorph": { "type": "number" },
                "shortcut": { "type": "string", "maxLength": 32 }, "id": { "type": "string", "maxLength": 64 }
            }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_app_info",
            "App-level state: active workspace, language, undo/redo availability of the ACTIVE workspace (scope core|modelagem|shading, recent history labels of each), keymap (path, modified count, conflicts), dock layout presets, GPU adapter/backend (read-only).",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_app_history",
            "Undo/redo of the ACTIVE workspace, like Edit > Undo/Redo (Modelagem and Shading have their OWN histories; every other workspace uses the core one). action undo|redo|list, steps 1..100.",
            json!({ "action": { "type": "string", "enum": ["undo", "redo", "list"] }, "steps": { "type": "integer", "minimum": 1, "maximum": 100 } }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_app_keymap",
            "Keyboard shortcuts. action: list (query, only_modified; chords, defaults, conflicts) | set (id, chords [\"Ctrl+Shift+Z\"…<=4]) | add (id, chord) | reset (id) | reset_all | export (path ABSOLUTE; all) | import (path ABSOLUTE). Changes are saved to the user's keymap file.",
            json!({
                "action": { "type": "string", "enum": ["list", "set", "add", "reset", "reset_all", "export", "import"] }, "id": { "type": "string", "maxLength": 64 }, "query": { "type": "string", "maxLength": 64 },
                "only_modified": { "type": "boolean" }, "chords": { "type": "array", "items": { "type": "string", "maxLength": 32 }, "maxItems": 4 }, "chord": { "type": "string", "maxLength": 32 },
                "path": { "type": "string", "maxLength": 1024 }, "all": { "type": "boolean" }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_app_layout",
            "Dock layouts of the ACTIVE workspace (Window menu). action: reset_current | reset_all | save_preset {name} | apply_preset {name} | delete_preset {name} | export {path} | import {path}. The command runs on the next UI frame: read app_info afterwards.",
            json!({
                "action": { "type": "string", "enum": ["reset_current", "reset_all", "save_preset", "apply_preset", "delete_preset", "export", "import"] },
                "name": { "type": "string", "maxLength": 60 }, "path": { "type": "string", "maxLength": 1024 }
            }), &["action"], ann(false, false, false),
        ),
        tool(
            "anigo_native_app_control",
            "Misc app actions. op=lang (value pt|en), frame (frame all/selection in Modelagem or Cenario), export_active_to_library (exports the active Modelagem object as a GLB authored asset into the REAL Library: persistent side effect).",
            json!({ "op": { "type": "string", "enum": ["lang", "frame", "export_active_to_library"] }, "value": { "type": "string", "enum": ["pt", "en"] } }), &["op"], ann(false, false, false),
        ),
        tool(
            "anigo_native_ping",
            "Cheap probe answered OUTSIDE the UI thread: pong, bridge protocol/version and ui_drawing (false when the ANIGO window is minimized/hidden and therefore not drawing frames — every other native tool would then fail after ~2.5 s with a clear message; restore the window). ui_idle_ms = time since the last drawn frame.",
            json!({}), &[], ann(true, false, true),
        ),
        tool(
            "anigo_native_get_selection",
            "Reads the current selection state: scene node ids, modeling selection counts, posing bones and Library selection.",
            json!({}), &[], ann(true, false, true),
        ),
    ]
}

pub fn is_native_tool(name: &str) -> bool {
    name.starts_with("anigo_native_")
}

fn text(v: Value) -> Value {
    json!({ "type": "text", "text": v })
}

/// JSON como texto, truncado de forma explícita (o modelo sabe que faltou).
fn json_text(v: &Value) -> Result<Value> {
    let mut s = serde_json::to_string_pretty(v)?;
    if s.chars().count() > MAX_TEXT_CHARS {
        let cut: String = s.chars().take(MAX_TEXT_CHARS).collect();
        s = format!("{cut}\n... [truncated: result larger than {MAX_TEXT_CHARS} characters; pass save_path to get all of it]");
    }
    Ok(json!({ "type": "text", "text": s }))
}

async fn call_bridge(client: &LiveBridgeClient, metrics: &McpMetrics, action: &str, params: Value) -> Result<Value> {
    metrics.bridge_commands_sent.fetch_add(1, Relaxed);
    match client.send_command(action, params).await {
        Ok(v) => Ok(v),
        Err(e) => {
            metrics.bridge_commands_failed.fetch_add(1, Relaxed);
            Err(e)
        }
    }
}

fn save_bytes(metrics: &McpMetrics, path: &str, bytes: &[u8]) -> Result<std::path::PathBuf> {
    let p = match sanitize_save_path(path) {
        Ok(p) => p,
        Err(e) => {
            metrics.fs_sandbox_rejections.fetch_add(1, Relaxed);
            return Err(e);
        }
    };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    }
    std::fs::write(&p, bytes).with_context(|| format!("writing {p:?}"))?;
    metrics.bytes_written_to_fs.fetch_add(bytes.len() as u64, Relaxed);
    Ok(p)
}

fn opt_str<'a>(args: &'a Value, k: &str) -> Result<Option<&'a str>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => anyhow::bail!("'{}' must be a string (code -32602)", k),
    }
}

fn opt_u64(args: &Value, k: &str, min: u64, max: u64) -> Result<Option<u64>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| anyhow::anyhow!("'{}' must be a non-negative integer (code -32602)", k))?;
            if n < min || n > max {
                anyhow::bail!("'{}' out of range {}..{}: {} (code -32602)", k, min, max, n);
            }
            Ok(Some(n))
        }
    }
}

fn opt_bool(args: &Value, k: &str) -> Result<Option<bool>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => anyhow::bail!("'{}' must be a boolean (code -32602)", k),
    }
}

/// Recusa propriedades desconhecidas (schema `additionalProperties:false`).
fn only_keys(args: &Value, allowed: &[&str]) -> Result<()> {
    match args {
        Value::Null => Ok(()),
        Value::Object(m) => {
            for k in m.keys() {
                if !allowed.contains(&k.as_str()) {
                    anyhow::bail!("Unknown argument '{}'. Allowed: {} (code -32602)", k, if allowed.is_empty() { "(none)".to_string() } else { allowed.join(", ") });
                }
            }
            Ok(())
        }
        _ => anyhow::bail!("arguments must be an object (code -32602)"),
    }
}

/// Imagem MCP a partir de um `png_base64` da bridge (+ salva no sandbox quando pedido).
fn image_result(metrics: &McpMetrics, label: String, b64: &str, save_path: Option<&str>) -> Result<Vec<Value>> {
    let mut note = label;
    if let Some(path) = save_path {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).context("invalid base64 image from bridge")?;
        let p = save_bytes(metrics, path, &bytes)?;
        note.push_str(&format!("\nSaved to: {p:?}"));
    }
    metrics.native_images_received.fetch_add(1, Relaxed);
    Ok(vec![text(Value::String(note)), json!({ "type": "image", "data": b64, "mimeType": "image/png" })])
}

/// `None` = não é uma ferramenta nativa.
pub async fn call(name: &str, args: &Value, client: &LiveBridgeClient, metrics: &McpMetrics) -> Option<Result<Vec<Value>>> {
    if !is_native_tool(name) {
        return None;
    }
    Some(dispatch(name, args, client, metrics).await)
}

async fn dispatch(name: &str, args: &Value, client: &LiveBridgeClient, m: &McpMetrics) -> Result<Vec<Value>> {
    match name {
        "anigo_native_get_status" => {
            only_keys(args, &[])?;
            let status = call_bridge(client, m, "GET_STATUS", json!({})).await?;
            let sel = call_bridge(client, m, "GET_SELECTION", json!({})).await.ok();
            Ok(vec![json_text(&json!({ "status": status, "selection": sel }))?])
        }
        "anigo_native_get_project" => {
            only_keys(args, &["include_document", "save_path"])?;
            let include = opt_bool(args, "include_document")?.unwrap_or(false);
            let data = call_bridge(client, m, "GET_PROJECT", json!({ "include_document": include })).await?;
            finish_json(m, &data, opt_str(args, "save_path")?)
        }
        "anigo_native_get_snapshot" => {
            only_keys(args, &["include_static", "save_path"])?;
            let include = opt_bool(args, "include_static")?.unwrap_or(false);
            let data = call_bridge(client, m, "GET_SNAPSHOT", json!({ "include_static": include })).await?;
            finish_json(m, &data, opt_str(args, "save_path")?)
        }
        "anigo_native_apply_command" => {
            only_keys(args, &["command", "if_revision"])?;
            let cmd = args.get("command").ok_or_else(|| anyhow::anyhow!("Missing 'command' (code -32602)"))?;
            validate::validate_native_command(cmd)?;
            let mut params = json!({ "command": cmd });
            if let Some(r) = opt_u64(args, "if_revision", 0, u64::MAX)? {
                params["if_revision"] = json!(r);
            }
            let data = call_bridge(client, m, "APPLY_COMMAND", params).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_undo" | "anigo_native_redo" => {
            only_keys(args, &[])?;
            let action = if name.ends_with("undo") { "UNDO" } else { "REDO" };
            let data = call_bridge(client, m, action, json!({})).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_get_log" => {
            only_keys(args, &["from", "limit", "include_commands", "sequence"])?;
            let mut p = serde_json::Map::new();
            if let Some(v) = opt_u64(args, "from", 0, u32::MAX as u64)? {
                p.insert("from".into(), json!(v));
            }
            if let Some(v) = opt_u64(args, "limit", 1, 500)? {
                p.insert("limit".into(), json!(v));
            }
            if let Some(v) = opt_u64(args, "sequence", 1, u64::MAX)? {
                p.insert("sequence".into(), json!(v));
            }
            if let Some(v) = opt_bool(args, "include_commands")? {
                p.insert("include_commands".into(), json!(v));
            }
            let data = call_bridge(client, m, "GET_LOG", Value::Object(p)).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_list_assets" => {
            only_keys(args, &["text", "kinds", "limit", "after"])?;
            let mut p = serde_json::Map::new();
            if let Some(t) = opt_str(args, "text")? {
                validate::validate_text(t, "text", 256)?;
                p.insert("text".into(), json!(t));
            }
            if let Some(k) = args.get("kinds").filter(|v| !v.is_null()) {
                let arr = k.as_array().filter(|a| a.len() <= 16).ok_or_else(|| anyhow::anyhow!("'kinds' must be an array of up to 16 strings (code -32602)"))?;
                let mut kinds = Vec::new();
                for i in arr {
                    let s = i.as_str().ok_or_else(|| anyhow::anyhow!("'kinds' must contain strings (code -32602)"))?;
                    validate::validate_native_asset_kind(s)?;
                    kinds.push(json!(s));
                }
                p.insert("kinds".into(), Value::Array(kinds));
            }
            if let Some(v) = opt_u64(args, "limit", 1, 500)? {
                p.insert("limit".into(), json!(v));
            }
            if let Some(a) = opt_str(args, "after")? {
                validate::validate_text(a, "after", 512)?;
                p.insert("after".into(), json!(a));
            }
            let data = call_bridge(client, m, "LIST_ASSETS", Value::Object(p)).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_place_asset" => {
            only_keys(args, &["asset_id", "asset_ids", "position"])?;
            let mut p = serde_json::Map::new();
            let mut n = 0;
            if let Some(id) = opt_str(args, "asset_id")? {
                validate::validate_uuid(id, "asset_id")?;
                p.insert("asset_id".into(), json!(id));
                n += 1;
            }
            if let Some(list) = args.get("asset_ids").filter(|v| !v.is_null()) {
                let arr = list.as_array().filter(|a| a.len() <= 32).ok_or_else(|| anyhow::anyhow!("'asset_ids' must be an array of up to 32 UUIDs (code -32602)"))?;
                let mut ids = Vec::new();
                for i in arr {
                    let s = i.as_str().ok_or_else(|| anyhow::anyhow!("'asset_ids' must contain strings (code -32602)"))?;
                    validate::validate_uuid(s, "asset_ids[]")?;
                    ids.push(json!(s));
                }
                n += ids.len();
                p.insert("asset_ids".into(), Value::Array(ids));
            }
            if n == 0 {
                anyhow::bail!("Missing 'asset_id' or 'asset_ids' (code -32602)");
            }
            if let Some(pos) = args.get("position").filter(|v| !v.is_null()) {
                let arr = pos.as_array().ok_or_else(|| anyhow::anyhow!("'position' must be [x,y,z] (code -32602)"))?;
                let v = validate::validate_vec3(arr, "position").map_err(|e| anyhow::anyhow!("{e} (code -32602)"))?;
                if v.abs().max_element() > 1.0e6 {
                    anyhow::bail!("position out of range (code -32602)");
                }
                p.insert("position".into(), json!([v.x, v.y, v.z]));
            }
            let data = call_bridge(client, m, "PLACE_ASSET", Value::Object(p)).await?;
            m.native_commands_applied.fetch_add(1, Relaxed);
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_set_workspace" => {
            only_keys(args, &["workspace"])?;
            let ws = opt_str(args, "workspace")?.ok_or_else(|| anyhow::anyhow!("Missing 'workspace' (code -32602)"))?;
            validate::validate_native_workspace(ws)?;
            let data = call_bridge(client, m, "SET_WORKSPACE", json!({ "workspace": ws })).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_get_ui_state" => {
            only_keys(args, &[])?;
            let data = call_bridge(client, m, "GET_UI_STATE", json!({})).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_ui_action" => {
            only_keys(args, &["action", "value", "open", "position", "slot"])?;
            let action = opt_str(args, "action")?.ok_or_else(|| anyhow::anyhow!("Missing 'action' (code -32602)"))?;
            const ACTIONS: &[&str] = &["set_mode", "select_tool", "toggle_panel", "select_props_tab", "save_session", "set_camera_view", "set_shading", "set_view_layout", "set_view_kind", "set_active_view", "add_primitive", "select_object", "set_select_mode"];
            if !ACTIONS.contains(&action) {
                anyhow::bail!("'action' must be one of {} (code -32602)", ACTIONS.join(", "));
            }
            let value = args
                .get("value")
                .cloned()
                .or_else(|| (action == "save_session").then(|| json!(true)))
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' (code -32602)"))?;
            match &value {
                Value::String(v) if v.len() <= 128 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) => {}
                Value::Number(n) if n.is_u64() => {}
                Value::Bool(_) => {}
                _ => anyhow::bail!("'value' must be a short id string ([A-Za-z0-9_.-]), a non-negative integer or a boolean (code -32602)"),
            }
            let mut p = serde_json::Map::new();
            p.insert("action".into(), json!(action));
            p.insert("value".into(), value);
            if let Some(o) = opt_bool(args, "open")? {
                p.insert("open".into(), json!(o));
            }
            if let Some(slot) = args.get("slot") {
                if !slot.as_u64().is_some_and(|n| n <= 3) {
                    anyhow::bail!("'slot' must be an integer 0..3 (code -32602)");
                }
                p.insert("slot".into(), slot.clone());
            }
            if let Some(pos) = args.get("position") {
                let ok = pos.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(|x| x.as_f64().is_some_and(|f| f.is_finite() && f.abs() <= 1.0e6)));
                if !ok {
                    anyhow::bail!("'position' must be [x,y,z] finite numbers (code -32602)");
                }
                p.insert("position".into(), pos.clone());
            }
            let data = call_bridge(client, m, "UI_ACTION", Value::Object(p)).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_input" => {
            only_keys(args, &["steps", "timeout_seconds"])?;
            let steps = args.get("steps").and_then(Value::as_array).ok_or_else(|| anyhow::anyhow!("Missing 'steps' (code -32602)"))?;
            if steps.is_empty() || steps.len() > 256 {
                anyhow::bail!("'steps' must have 1..=256 entries (code -32602)");
            }
            let timeout = opt_u64(args, "timeout_seconds", 1, 60)?.unwrap_or(15);
            call_bridge(client, m, "INPUT", json!({ "steps": steps })).await?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            loop {
                let st = call_bridge(client, m, "GET_UI_STATE", json!({})).await?;
                if st["input_pending"].as_u64().unwrap_or(0) == 0 {
                    return Ok(vec![json_text(&st)?]);
                }
                if std::time::Instant::now() > deadline {
                    anyhow::bail!("input queue still has {} pending steps after {timeout}s (code -32002)", st["input_pending"]);
                }
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
        }
        "anigo_native_model_info" => model_call(client, m, "info", args, &["limit", "include_geometry"]).await,
        "anigo_native_model_object" => model_call(client, m, "object", args, &["action", "kind", "position", "index", "name", "translation", "rotation_deg", "scale"]).await,
        "anigo_native_mesh_select" => model_call(client, m, "select", args, &["mode", "what", "indices", "edges", "box", "extend", "limit"]).await,
        "anigo_native_mesh_op" => model_call(client, m, "edit_op", args, &["name", "params"]).await,
        "anigo_native_mesh_transform" => model_call(client, m, "transform", args, &["kind", "value", "pivot", "proportional"]).await,
        "anigo_native_sculpt" => {
            model_call(
                client,
                m,
                "sculpt",
                args,
                &["action", "brush", "radius_px", "strength", "spacing_pct", "invert", "mirror", "radial", "path", "paths", "world_path", "what", "iterations", "smooth_strength", "axis", "from", "voxel_size", "enabled", "detail_px", "frame_view", "shade_smooth", "voxel_smooth"],
            )
            .await
        }
        "anigo_native_model_history" => model_call(client, m, "history", args, &["action", "steps"]).await,
        "anigo_native_hair_info" => op_call(client, m, "HAIR", Some("info"), args, &["include_strands", "limit"], &[]).await,
        "anigo_native_hair_setup" => op_call(client, m, "HAIR", None, args, &["op", "action", "object_index", "group", "name", "on", "index", "groups", "bones", "patch"], &["target", "group", "bounce", "regenerate", "export"]).await,
        "anigo_native_hair_strand" => op_call(client, m, "HAIR", Some("strand"), args, &["action", "group", "strand", "point", "params", "apply", "epsilon", "to_group", "after", "index", "pos", "world_pos"], &[]).await,
        "anigo_native_hair_stroke" => op_call(client, m, "HAIR", None, args, &["op", "path", "world_path", "local_path", "group", "params", "stroke", "radius_px", "strength", "all_groups"], &["draw", "comb"]).await,
        "anigo_native_weights_info" => op_call(client, m, "WEIGHTS", Some("info"), args, &["vertices"], &[]).await,
        "anigo_native_npr_paint" => op_call(client, m, "WEIGHTS", Some("npr"), args, &["action", "channel", "center", "world_center", "radius", "falloff", "value", "blend", "strength", "verts", "use_selection", "iterations", "factor", "p0", "p1", "v0", "v1", "samples", "max_dist", "bias", "seed"], &[]).await,
        "anigo_native_skin_weights" => op_call(client, m, "WEIGHTS", Some("skin"), args, &["action", "joint", "weight", "normalize", "vertex", "vertices", "verts", "use_selection", "center", "world_center", "radius", "falloff", "value", "blend", "strength", "max", "iterations", "factor", "from_object", "max_distance", "bones", "space", "axis", "from", "joint_map", "tolerance"], &[]).await,
        "anigo_native_modifiers" => op_call(client, m, "MESHDATA", Some("modifiers"), args, &["action", "kind", "params", "name", "index", "from", "to", "enabled_viewport", "enabled_render"], &[]).await,
        "anigo_native_normals" => op_call(client, m, "MESHDATA", Some("normals"), args, &["action", "angle_deg", "use_selection", "mode", "iterations", "factor", "from_object", "mapping", "mix", "max_distance"], &[]).await,
        "anigo_native_uv" => op_call(client, m, "MESHDATA", Some("uv"), args, &["action", "params"], &[]).await,
        "anigo_native_material" => op_call(client, m, "SHADING", None, args, &["op", "value", "index", "patch", "id", "action", "fields_only"], &["info", "source", "target", "set", "preset", "reset", "copy", "paste", "history"]).await,
        "anigo_native_animation" => op_call(client, m, "ANIM", None, args, &["op", "action", "id", "name", "frame", "n", "value", "loop_mode", "extrapolation", "hold_step", "range", "fps", "retime", "bones", "on", "limit"], &["info", "clip", "transport", "key"]).await,
        "anigo_native_render_info" => op_call(client, m, "RENDER", Some("info"), args, &["limit"], &[]).await,
        "anigo_native_render_prefs" => {
            only_keys(args, &["patch"])?;
            let mut a = args.clone();
            if let Some(d) = a.get("patch").and_then(|p| p.get("dir")).and_then(Value::as_str).map(str::to_string) {
                if !d.trim().is_empty() {
                    let safe = sandboxed_abs_dir(m, &d, true)?;
                    a["patch"]["dir"] = json!(safe);
                }
            }
            op_call(client, m, "RENDER", Some("prefs"), &a, &["patch"], &[]).await
        }
        "anigo_native_render_camera" => op_call(client, m, "RENDER", Some("camera"), args, &["action", "node_id", "project", "selection"], &[]).await,
        "anigo_native_render_job" => op_call(client, m, "RENDER", Some("job"), args, &["action", "job_id"], &[]).await,
        "anigo_native_render_history" => op_call(client, m, "RENDER", Some("history"), args, &["action", "file", "limit"], &[]).await,
        "anigo_native_library_browse" => op_call(client, m, "LIBRARY", None, args, &["op", "query", "patch", "limit", "asset_id", "asset_ids", "all_matching", "none"], &["info", "query", "filter", "select", "detail", "edit_in_modeling"]).await,
        "anigo_native_library_edit" => op_call(client, m, "LIBRARY", None, args, &["op", "asset_ids", "patch", "action", "collection_id", "name", "query", "path", "new", "color"], &["meta", "collections", "tags"]).await,
        "anigo_native_library_trash" => op_call(client, m, "LIBRARY", Some("trash"), args, &["action", "asset_ids", "force", "confirm", "older_than_secs"], &[]).await,
        "anigo_native_library_import" => {
            only_keys(args, &["paths", "options", "wait", "cancel"])?;
            let mut a = args.clone();
            if let Some(list) = a.get("paths").and_then(Value::as_array).cloned() {
                let mut safe = Vec::new();
                for p in list {
                    let p = p.as_str().ok_or_else(|| anyhow::anyhow!("'paths' must contain strings (code -32602)"))?;
                    safe.push(json!(sandboxed_abs_dir(m, p, false)?));
                }
                a["paths"] = Value::Array(safe);
            }
            op_call(client, m, "LIBRARY", Some("import"), &a, &["paths", "options", "wait", "cancel"], &[]).await
        }
        "anigo_native_library_roots" => {
            only_keys(args, &["action", "root_id", "path", "mode", "name", "scan"])?;
            let mut a = args.clone();
            if let Some(p) = a.get("path").and_then(Value::as_str).map(str::to_string) {
                let writes = a.get("mode").and_then(Value::as_str) == Some("managed");
                a["path"] = json!(sandboxed_abs_dir(m, &p, writes)?);
            }
            op_call(client, m, "LIBRARY", Some("roots"), &a, &["action", "root_id", "path", "mode", "name", "scan"], &[]).await
        }
        "anigo_native_library_maintenance" => op_call(client, m, "LIBRARY", Some("maintenance"), args, &["action", "limit", "rehash", "fix"], &[]).await,
        "anigo_native_pose_info" => op_call(client, m, "POSE", Some("info"), args, &["compact"], &[]).await,
        "anigo_native_pose_options" => op_call(client, m, "POSE", Some("options"), args, &["patch"], &[]).await,
        "anigo_native_pose_select" => op_call(client, m, "POSE", Some("select"), args, &["bones", "mode", "all", "none", "active"], &[]).await,
        "anigo_native_pose_bone" => op_call(client, m, "POSE", Some("bone"), args, &["action", "bone", "euler_deg", "quat", "axis", "angle_deg"], &[]).await,
        "anigo_native_pose_edit" => op_call(client, m, "POSE", None, args, &["op", "part", "value", "delta", "foot_lock", "auto_ground", "limb", "target", "pole", "space", "action", "weight", "flipped", "scope", "bones", "body", "side", "preset", "curls"], &["translate", "ik", "gaze", "ground", "mirror", "copy", "paste", "reset", "preset", "hands"]).await,
        "anigo_native_pose_library" => op_call(client, m, "POSE", Some("library"), args, &["action", "name", "index", "only_selected", "flipped", "blend"], &[]).await,
        "anigo_native_light_info" => op_call(client, m, "LIGHT", Some("info"), args, &[], &[]).await,
        "anigo_native_light_set" => op_call(client, m, "LIGHT", Some("set"), args, &["patch"], &[]).await,
        "anigo_native_light_control" => op_call(client, m, "LIGHT", None, args, &["op", "id", "on", "rel_az", "rel_el", "bake", "show_gizmo", "frame"], &["rig", "reset_sun", "follow", "view"]).await,
        "anigo_native_model_options" => model_call(client, m, "options", args, &["patch"]).await,
        "anigo_native_anim_keys" => op_call(client, m, "ANIM", Some("keys"), args, &["action", "items", "all", "none", "frame_range", "mode", "bone", "target", "frame", "interp", "c", "delta", "mirrored", "value", "euler_deg", "quat"], &[]).await,
        "anigo_native_anim_more" => {
            only_keys(args, &["op", "action", "frame", "name", "what", "node_id", "patch", "on", "dir", "path", "kind", "index", "at_playhead", "render"])?;
            let mut a = args.clone();
            sandbox_arg(m, &mut a, "dir", true)?;
            let writes = a.get("action").and_then(Value::as_str) == Some("export");
            sandbox_arg(m, &mut a, "path", writes)?;
            op_call(client, m, "ANIM", None, &a, &["op", "action", "frame", "name", "what", "node_id", "patch", "on", "dir", "path", "kind", "index", "at_playhead", "render"], &["marker", "scene_keys", "onion", "physics", "file", "library", "options", "sequence"]).await
        }
        "anigo_native_material_view" => op_call(client, m, "SHADING", None, args, &["op", "patch", "action"], &["view", "compare"]).await,
        "anigo_native_material_tools" => op_call(client, m, "SHADING", None, args, &["op", "action", "group", "id", "recipe", "deg", "color", "harmony", "add_to_palette", "index", "to", "row", "stops", "constant"], &["param", "palette", "ramp"]).await,
        "anigo_native_material_library" => op_call(client, m, "SHADING", None, args, &["op", "action", "name", "asset_id", "slot", "text"], &["presets", "texture"]).await,
        "anigo_native_scene_info" => op_call(client, m, "SCENE", Some("info"), args, &["limit"], &[]).await,
        "anigo_native_scene_select" => op_call(client, m, "SCENE", Some("select"), args, &["ids", "mode", "what", "none"], &[]).await,
        "anigo_native_scene_edit" => op_call(client, m, "SCENE", None, args, &["op", "tool", "space", "patch", "frame", "action", "ids", "id", "name", "parent", "folder", "kind", "position", "translation", "rotation_deg", "scale", "depth"], &["tool", "snap", "view", "visibility", "outliner", "place", "transform", "camera", "history"]).await,
        "anigo_native_scene_project" => {
            only_keys(args, &["action", "path", "discard_current"])?;
            let mut a = args.clone();
            let writes = a.get("action").and_then(Value::as_str) == Some("save");
            sandbox_arg(m, &mut a, "path", writes)?;
            op_call(client, m, "SCENE", Some("project"), &a, &["action", "path", "discard_current"], &[]).await
        }
        "anigo_native_char_info" => op_call(client, m, "CHAR", Some("info"), args, &[], &[]).await,
        "anigo_native_char_sliders" => op_call(client, m, "CHAR", Some("sliders"), args, &["action", "region", "query", "only_modified", "values", "ids", "all"], &[]).await,
        "anigo_native_char_edit" => op_call(client, m, "CHAR", None, args, &["op", "patch", "gender", "dimorphism", "endomorph", "mesomorph", "ectomorph", "shortcut", "id"], &["proportions", "gender", "somatotype", "preset", "reset_all", "view"]).await,
        "anigo_native_app_info" => op_call(client, m, "APP", Some("info"), args, &[], &[]).await,
        "anigo_native_app_history" => op_call(client, m, "APP", Some("history"), args, &["action", "steps"], &[]).await,
        "anigo_native_app_keymap" => {
            only_keys(args, &["action", "id", "query", "only_modified", "chords", "chord", "path", "all"])?;
            let mut a = args.clone();
            let writes = a.get("action").and_then(Value::as_str) == Some("export");
            sandbox_arg(m, &mut a, "path", writes)?;
            op_call(client, m, "APP", Some("keymap"), &a, &["action", "id", "query", "only_modified", "chords", "chord", "path", "all"], &[]).await
        }
        "anigo_native_app_layout" => {
            only_keys(args, &["action", "name", "path"])?;
            let mut a = args.clone();
            let writes = a.get("action").and_then(Value::as_str) == Some("export");
            sandbox_arg(m, &mut a, "path", writes)?;
            op_call(client, m, "APP", Some("layout"), &a, &["action", "name", "path"], &[]).await
        }
        "anigo_native_app_control" => op_call(client, m, "APP", None, args, &["op", "value"], &["lang", "frame", "export_active_to_library"]).await,
        "anigo_native_ping" => op_call_plain(client, m, "PING", args).await,
        "anigo_native_get_selection" => {
            only_keys(args, &[])?;
            let data = call_bridge(client, m, "GET_SELECTION", json!({})).await?;
            Ok(vec![json_text(&data)?])
        }
        "anigo_native_screenshot" => {
            only_keys(args, &["target", "max_width", "save_path"])?;
            let mut p = serde_json::Map::new();
            if let Some(t) = opt_str(args, "target")? {
                if t != "viewport" && t != "window" {
                    anyhow::bail!("'target' must be 'viewport' or 'window' (code -32602)");
                }
                p.insert("target".into(), json!(t));
            }
            if let Some(w) = opt_u64(args, "max_width", 64, 4096)? {
                p.insert("max_width".into(), json!(w));
            }
            let save = opt_str(args, "save_path")?;
            if let Some(path) = save {
                // valida o destino ANTES de gastar uma captura
                sanitize_save_path(path).inspect_err(|_| {
                    m.fs_sandbox_rejections.fetch_add(1, Relaxed);
                })?;
            }
            let data = call_bridge(client, m, "SCREENSHOT", Value::Object(p)).await?;
            let b64 = data.get("png_base64").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("bridge returned no image"))?;
            let label = format!(
                "Native studio screenshot: {}x{} px, target={}, workspace={}",
                data["width"], data["height"], data["target"].as_str().unwrap_or("?"), data["workspace"].as_str().unwrap_or("?")
            );
            image_result(m, label, b64, save)
        }
        "anigo_native_render_frame" => render_frame(args, client, m).await,
        other => anyhow::bail!("Unknown tool: {} (code -32601)", other),
    }
}

/// Ferramentas de modelagem: valida as chaves (e o tamanho) ANTES de qualquer rede e repassa para a ação `MODEL` da bridge.
async fn model_call(client: &LiveBridgeClient, m: &McpMetrics, op: &str, args: &Value, allowed: &[&str]) -> Result<Vec<Value>> {
    only_keys(args, allowed)?;
    if serde_json::to_string(args).map(|t| t.len()).unwrap_or(usize::MAX) > 1_000_000 {
        anyhow::bail!("arguments too large (max 1 MB) (code -32602)");
    }
    let mut p = args.as_object().cloned().unwrap_or_default();
    p.insert("op".into(), json!(op));
    let data = call_bridge(client, m, "MODEL", Value::Object(p)).await?;
    Ok(vec![json_text(&data)?])
}

/// Ferramentas de cabelo/pesos/malha/material/animação: valida chaves, `op` (quando a ferramenta tem vários) e tamanho ANTES de
/// qualquer rede e repassa para a ação da bridge. `fixed_op` = a ferramenta já é uma `op`; senão o chamador escolhe entre `ops`.
/// Caminho ABSOLUTO dentro do sandbox de arquivos (`write` = pasta que o app vai criar/gravar; senão, leitura). O app só recebe o
/// caminho já validado: o sandbox mora aqui, no `anigo-mcp`.
fn sandboxed_abs_dir(m: &McpMetrics, path: &str, write: bool) -> Result<String> {
    if !std::path::Path::new(path).is_absolute() {
        anyhow::bail!("path must be ABSOLUTE (code -32602): {path:?}");
    }
    let r = if write { sanitize_save_path(path) } else { crate::fs_sandbox::sanitize_read_path(path) };
    match r {
        Ok(p) => Ok(p.display().to_string()),
        Err(e) => {
            m.fs_sandbox_rejections.fetch_add(1, Relaxed);
            Err(e)
        }
    }
}

/// Troca o valor de `key` (caminho de arquivo/pasta) pelo caminho já validado no sandbox (`write` = o app vai gravar).
fn sandbox_arg(m: &McpMetrics, args: &mut Value, key: &str, write: bool) -> Result<()> {
    if let Some(p) = args.get(key).and_then(Value::as_str).map(str::to_string) {
        args[key] = json!(sandboxed_abs_dir(m, &p, write)?);
    }
    Ok(())
}

async fn op_call_plain(client: &LiveBridgeClient, m: &McpMetrics, action: &str, args: &Value) -> Result<Vec<Value>> {
    only_keys(args, &[])?;
    let data = call_bridge(client, m, action, json!({})).await?;
    Ok(vec![json_text(&data)?])
}

async fn op_call(client: &LiveBridgeClient, m: &McpMetrics, bridge_action: &str, fixed_op: Option<&str>, args: &Value, allowed: &[&str], ops: &[&str]) -> Result<Vec<Value>> {
    only_keys(args, allowed)?;
    if serde_json::to_string(args).map(|t| t.len()).unwrap_or(usize::MAX) > 1_000_000 {
        anyhow::bail!("arguments too large (max 1 MB) (code -32602)");
    }
    let mut p = args.as_object().cloned().unwrap_or_default();
    match fixed_op {
        Some(op) => {
            p.insert("op".into(), json!(op));
        }
        None => {
            let op = p.get("op").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("Missing 'op' (one of: {}) (code -32602)", ops.join(", ")))?;
            if !ops.contains(&op) {
                anyhow::bail!("'op' must be one of: {} (code -32602)", ops.join(", "));
            }
        }
    }
    let data = call_bridge(client, m, bridge_action, Value::Object(p)).await?;
    Ok(vec![json_text(&data)?])
}

fn finish_json(m: &McpMetrics, data: &Value, save: Option<&str>) -> Result<Vec<Value>> {
    let mut out = vec![json_text(data)?];
    if let Some(path) = save {
        let bytes = serde_json::to_vec_pretty(data)?;
        let p = save_bytes(m, path, &bytes)?;
        out.push(text(Value::String(format!("Saved to: {p:?} ({} bytes)", bytes.len()))));
    }
    Ok(out)
}

async fn render_frame(args: &Value, client: &LiveBridgeClient, m: &McpMetrics) -> Result<Vec<Value>> {
    only_keys(args, &["wait", "timeout_seconds", "inline", "save_path"])?;
    let wait = opt_bool(args, "wait")?.unwrap_or(true);
    let timeout_s = opt_u64(args, "timeout_seconds", 5, 300)?.unwrap_or(90);
    let inline = opt_bool(args, "inline")?.unwrap_or(false);
    let save = opt_str(args, "save_path")?;
    if let Some(path) = save {
        sanitize_save_path(path).inspect_err(|_| {
            m.fs_sandbox_rejections.fetch_add(1, Relaxed);
        })?;
    }
    if (inline || save.is_some()) && !wait {
        anyhow::bail!("inline/save_path require wait=true (code -32602)");
    }
    let queued = call_bridge(client, m, "RENDER_FRAME", json!({})).await?;
    if !wait {
        return Ok(vec![json_text(&queued)?]);
    }
    let job = queued.get("job_id").and_then(Value::as_u64).ok_or_else(|| anyhow::anyhow!("bridge returned no job_id"))?;
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    loop {
        let st = call_bridge(client, m, "RENDER_STATUS", json!({ "job_id": job })).await?;
        match st.get("status").and_then(Value::as_str) {
            Some("done") => {
                m.frames_rendered.fetch_add(1, Relaxed);
                if inline || save.is_some() {
                    let full = call_bridge(client, m, "RENDER_STATUS", json!({ "job_id": job, "inline": true })).await?;
                    if let Some(b64) = full.get("png_base64").and_then(Value::as_str) {
                        let label = format!("Render job {job} done:\n{}", serde_json::to_string_pretty(&full["files"])?);
                        return image_result(m, label, b64, save);
                    }
                    anyhow::bail!("render finished but the image could not be returned inline: {}", full["inline_error"].as_str().unwrap_or("unknown"));
                }
                return Ok(vec![json_text(&st)?]);
            }
            Some("failed") | Some("cancelled") => {
                anyhow::bail!("Render job {} {}: {}", job, st["status"].as_str().unwrap_or("?"), st["error"].as_str().unwrap_or("(no details)"));
            }
            _ => {}
        }
        if Instant::now() > deadline {
            anyhow::bail!("Render job {job} still running after {timeout_s}s (progress {:.0}%); poll with wait=false is not available — raise timeout_seconds", st["progress"].as_f64().unwrap_or(0.0) * 100.0);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed_and_names_are_unique_and_prefixed() {
        let defs = definitions();
        let mut names = std::collections::HashSet::new();
        for d in &defs {
            let n = d["name"].as_str().unwrap();
            assert!(is_native_tool(n), "{n}");
            assert!(names.insert(n.to_string()), "duplicado {n}");
            assert!(d["description"].as_str().unwrap().len() > 20);
            assert_eq!(d["inputSchema"]["type"], "object");
            assert_eq!(d["inputSchema"]["additionalProperties"], false);
            assert!(d["annotations"]["readOnlyHint"].is_boolean());
        }
        assert!(defs.len() >= 13);
    }

    #[test]
    fn only_keys_rejects_unknown_arguments() {
        assert!(only_keys(&json!({}), &[]).is_ok());
        assert!(only_keys(&Value::Null, &[]).is_ok());
        assert!(only_keys(&json!({"a": 1}), &["a"]).is_ok());
        assert!(only_keys(&json!({"b": 1}), &["a"]).is_err());
        assert!(only_keys(&json!([1]), &["a"]).is_err());
    }

    #[test]
    fn typed_argument_helpers_enforce_types_and_ranges() {
        let a = json!({"s": "x", "n": 5, "b": true, "neg": -1});
        assert_eq!(opt_str(&a, "s").unwrap(), Some("x"));
        assert!(opt_str(&a, "n").is_err());
        assert_eq!(opt_u64(&a, "n", 1, 10).unwrap(), Some(5));
        assert!(opt_u64(&a, "n", 6, 10).is_err());
        assert!(opt_u64(&a, "neg", 0, 10).is_err());
        assert_eq!(opt_u64(&a, "zz", 0, 1).unwrap(), None);
        assert_eq!(opt_bool(&a, "b").unwrap(), Some(true));
        assert!(opt_bool(&a, "s").is_err());
    }

    #[test]
    fn long_json_is_truncated_explicitly() {
        let v = json!({"blob": "a".repeat(MAX_TEXT_CHARS * 2)});
        let t = json_text(&v).unwrap();
        let s = t["text"].as_str().unwrap();
        assert!(s.contains("[truncated"));
        assert!(s.chars().count() < MAX_TEXT_CHARS + 300);
        let small = json_text(&json!({"a": 1})).unwrap();
        assert!(!small["text"].as_str().unwrap().contains("truncated"));
    }

    #[tokio::test]
    async fn validation_happens_before_any_network_call() {
        // cliente apontando para uma porta fechada: se a validação deixasse passar, o erro seria de conexão
        let client = LiveBridgeClient::with_token("127.0.0.1:1", "t");
        let m = McpMetrics::new();
        for (tool, args) in [
            ("anigo_native_apply_command", json!({"command": {"kind": "Bad Kind"}})),
            ("anigo_native_apply_command", json!({"command": "x"})),
            ("anigo_native_apply_command", json!({})),
            ("anigo_native_apply_command", json!({"command": {"kind": "x"}, "extra": 1})),
            ("anigo_native_set_workspace", json!({"workspace": "../x"})),
            ("anigo_native_place_asset", json!({"asset_id": "nao-uuid"})),
            ("anigo_native_place_asset", json!({})),
            ("anigo_native_place_asset", json!({"asset_id": "0197f3a2-1b2c-7d3e-8f40-0123456789ab", "position": [1, 2]})),
            ("anigo_native_list_assets", json!({"kinds": ["exe"]})),
            ("anigo_native_list_assets", json!({"limit": 5000})),
            ("anigo_native_get_log", json!({"limit": 0})),
            ("anigo_native_screenshot", json!({"target": "tela"})),
            ("anigo_native_screenshot", json!({"max_width": 10})),
            ("anigo_native_render_frame", json!({"wait": false, "inline": true})),
            ("anigo_native_undo", json!({"x": 1})),
        ] {
            let err = call(tool, &args, &client, &m).await.expect("é nativa").unwrap_err().to_string();
            assert!(err.contains("-32602"), "{tool} {args}: {err}");
        }
        assert_eq!(m.bridge_commands_sent.load(Relaxed), 0, "nada deveria ter chegado à rede");
        assert!(call("anigo_ping", &json!({}), &client, &m).await.is_none());
    }

    #[tokio::test]
    async fn unreachable_bridge_counts_a_failed_command() {
        let client = LiveBridgeClient::with_token("127.0.0.1:1", "t");
        let m = McpMetrics::new();
        let err = call("anigo_native_get_selection", &json!({}), &client, &m).await.unwrap().unwrap_err().to_string();
        assert!(err.contains("not running") || err.contains("timeout"), "{err}");
        assert_eq!((m.bridge_commands_sent.load(Relaxed), m.bridge_commands_failed.load(Relaxed)), (1, 1));
    }

    #[test]
    fn save_path_outside_the_sandbox_is_rejected_and_counted() {
        let m = McpMetrics::new();
        let e = save_bytes(&m, "/etc/anigo_nope.png", b"x").unwrap_err().to_string();
        assert!(!e.is_empty());
        assert_eq!(m.fs_sandbox_rejections.load(Relaxed), 1);
        assert_eq!(m.bytes_written_to_fs.load(Relaxed), 0);
    }
}
