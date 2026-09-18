import subprocess
import json
import os
import sys

print("=== Starting Sprint 03 Anatomical Morphs & MCP Automated Validation ===")

# Locate anigo-mcp executable
exe_path = os.path.join("target", "debug", "anigo-mcp.exe")
if not os.path.exists(exe_path):
    print(f"Error: {exe_path} not found. Please build with `cargo build -p anigo-mcp` first.")
    sys.exit(1)

proc = subprocess.Popen(
    [exe_path],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1
)

def send_req(req_id, method, params):
    payload = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        err = proc.stderr.read()
        print("MCP process exited unexpectedly:", err)
        sys.exit(1)
    return json.loads(line)

os.makedirs("baselines", exist_ok=True)

# 1. Initialize
init_resp = send_req(1, "initialize", {})
print("[1] Initialized:", init_resp.get("result", {}).get("serverInfo"))

# 2. Verify registered tools list
tools_resp = send_req(2, "tools/list", {})
tool_names = [t["name"] for t in tools_resp.get("result", {}).get("tools", [])]
print(f"[2] Registered {len(tool_names)} tools:")
sprint03_required_tools = [
    "anigo_set_character_model",
    "anigo_set_somatotype",
    "anigo_apply_morph_slider",
    "anigo_inspect_mesh_integrity",
    "anigo_get_active_morphs",
    "anigo_reset_morphs",
    "anigo_render_frame"
]
for tool in sprint03_required_tools:
    assert tool in tool_names, f"Required Sprint 03 tool '{tool}' is missing!"
    print(f"    [OK] {tool} verified")

# 3. Inspect baseline mesh integrity
integrity_resp = send_req(3, "tools/call", {"name": "anigo_inspect_mesh_integrity", "arguments": {}})
raw_text = integrity_resp["result"]["content"][0]["text"]
report = json.loads(raw_text)
print("[3] Baseline Mesh Integrity:\n", json.dumps(report, indent=2))
assert report["is_valid"] is True, f"Baseline mesh is invalid: {report}"
assert report["degenerate_triangle_count"] == 0, f"Found degenerate triangles: {report['degenerate_triangle_count']}"
assert report["inverted_normal_count"] == 0, f"Found inverted normals: {report['inverted_normal_count']}"
assert report["nan_or_inf_count"] == 0, f"Found NaN/Inf coordinates: {report['nan_or_inf_count']}"
assert report["vertex_count"] == 4070, f"Expected 4070 vertices, got {report['vertex_count']}"
assert report["triangle_count"] == 6880, f"Expected 6880 triangles, got {report['triangle_count']}"

# 4. Swap to canonical Female base model
model_resp = send_req(4, "tools/call", {
    "name": "anigo_set_character_model",
    "arguments": {"model_type": "female"}
})
print("[4] anigo_set_character_model ('female'):\n", model_resp["result"]["content"][0]["text"])
model_data = json.loads(model_resp["result"]["content"][0]["text"])
assert model_data["status"] == "success"
assert model_data["vertex_count"] == 4070
assert model_data["triangle_count"] == 6880
assert model_data["isomorphic"] is True

# 5. Inspect female baseline integrity
fem_integrity = send_req(5, "tools/call", {"name": "anigo_inspect_mesh_integrity", "arguments": {}})
fem_report = json.loads(fem_integrity["result"]["content"][0]["text"])
print("[5] Female Mesh Integrity:\n", json.dumps(fem_report, indent=2))
assert fem_report["is_valid"] is True
assert fem_report["degenerate_triangle_count"] == 0
assert fem_report["inverted_normal_count"] == 0

# 6. Apply anatomical morph sliders across multiple zones
sliders_to_apply = [
    ("bust_volume_cup", 1.20),
    ("waist_pinch_width", 0.80),
    ("hip_trochanteric_flare", 1.30),
    ("jaw_v_line_taper", 0.85),
    ("eye_canthal_tilt", 8.0),
    ("nose_bridge_depth", 1.25),
    ("cheek_fullness_upper", 0.40)
]
for idx, (slider_id, val) in enumerate(sliders_to_apply):
    resp = send_req(10 + idx, "tools/call", {
        "name": "anigo_apply_morph_slider",
        "arguments": {"slider_id": slider_id, "value": val}
    })
    res_data = json.loads(resp["result"]["content"][0]["text"])
    assert res_data["status"] == "success"
    print(f"    [Morph] Applied '{slider_id}' = {res_data['applied_value']} ({res_data['zone']})")

# 7. Query active morphs
active_resp = send_req(20, "tools/call", {"name": "anigo_get_active_morphs", "arguments": {}})
active_data = json.loads(active_resp["result"]["content"][0]["text"])
print(f"[7] Active Morphs Count: {active_data['active_count']}")
assert active_data["active_count"] >= len(sliders_to_apply), f"Expected >= {len(sliders_to_apply)} active morphs"

# 8. Inspect morphed female mesh integrity (must remain 100% valid)
morphed_integrity = send_req(21, "tools/call", {"name": "anigo_inspect_mesh_integrity", "arguments": {}})
morphed_report = json.loads(morphed_integrity["result"]["content"][0]["text"])
print("[8] Morphed Female Mesh Integrity:\n", json.dumps(morphed_report, indent=2))
assert morphed_report["is_valid"] is True, f"Morphed mesh failed integrity: {morphed_report}"
assert morphed_report["degenerate_triangle_count"] == 0
assert morphed_report["inverted_normal_count"] == 0
assert morphed_report["nan_or_inf_count"] == 0

# 9. Apply Heath-Carter Somatotype (Athletic/Mesomorphic Female)
somato_resp = send_req(22, "tools/call", {
    "name": "anigo_set_somatotype",
    "arguments": {"endo": 2.5, "meso": 5.5, "ecto": 3.0}
})
somato_data = json.loads(somato_resp["result"]["content"][0]["text"])
print("[9] Heath-Carter Somatotype Applied:\n", json.dumps(somato_data, indent=2))
assert somato_data["status"] == "success"

# 10. Render Female Morphed Frame offscreen with real WebGPU hardware
female_render_path = "baselines/test_render_sprint03_female_morph.png"
render_fem = send_req(23, "tools/call", {
    "name": "anigo_render_frame",
    "arguments": {
        "width": 800,
        "height": 600,
        "save_path": female_render_path
    }
})
print(f"[10] Rendered Morphed Female: {female_render_path}")
print("     Metrics:", render_fem["result"]["content"][0]["text"])
assert os.path.exists(female_render_path), f"Rendered frame {female_render_path} does not exist"
assert os.path.getsize(female_render_path) > 1000, f"Rendered frame {female_render_path} is unexpectedly small"

# 11. Switch to Male Berserker and apply muscular sliders
model_male = send_req(24, "tools/call", {
    "name": "anigo_set_character_model",
    "arguments": {"model_type": "male"}
})
somato_male = send_req(25, "tools/call", {
    "name": "anigo_set_somatotype",
    "arguments": {"endo": 1.2, "meso": 8.5, "ecto": 1.5}
})
male_sliders = [
    ("deltoid_muscle_volume", 0.90),
    ("biceps_peak_volume", 0.85),
    ("pectoral_muscle_bulk", 0.90),
    ("latissimus_dorsi_flare", 0.70),
    ("abs_sixpack_definition", 0.75),
    ("trapezius_bulk", 0.80),
    ("jaw_bigonial_width", 1.25)
]
for idx, (slider_id, val) in enumerate(male_sliders):
    send_req(30 + idx, "tools/call", {
        "name": "anigo_apply_morph_slider",
        "arguments": {"slider_id": slider_id, "value": val}
    })

# 12. Inspect male integrity
male_integrity = send_req(40, "tools/call", {"name": "anigo_inspect_mesh_integrity", "arguments": {}})
male_report = json.loads(male_integrity["result"]["content"][0]["text"])
print("[12] Muscular Berserker Male Mesh Integrity:\n", json.dumps(male_report, indent=2))
assert male_report["is_valid"] is True
assert male_report["degenerate_triangle_count"] == 0
assert male_report["inverted_normal_count"] == 0

# 13. Render Male Berserker Frame offscreen
male_render_path = "baselines/test_render_sprint03_male_morph.png"
render_male = send_req(41, "tools/call", {
    "name": "anigo_render_frame",
    "arguments": {
        "width": 800,
        "height": 600,
        "save_path": male_render_path
    }
})
print(f"[13] Rendered Muscular Berserker Male: {male_render_path}")
print("     Metrics:", render_male["result"]["content"][0]["text"])
assert os.path.exists(male_render_path), f"Rendered frame {male_render_path} does not exist"
assert os.path.getsize(male_render_path) > 1000, f"Rendered frame {male_render_path} is unexpectedly small"

# 14. Reset Morphs
reset_resp = send_req(42, "tools/call", {"name": "anigo_reset_morphs", "arguments": {}})
reset_data = json.loads(reset_resp["result"]["content"][0]["text"])
print("[14] Reset Morphs:\n", json.dumps(reset_data, indent=2))
assert reset_data["status"] == "success"
assert reset_data["active_morphs_count"] == 0

reset_active = send_req(43, "tools/call", {"name": "anigo_get_active_morphs", "arguments": {}})
reset_active_data = json.loads(reset_active["result"]["content"][0]["text"])
assert reset_active_data["active_count"] == 0, f"Expected 0 active morphs after reset, got {reset_active_data['active_count']}"

# Final clean exit
try:
    proc.terminate()
    proc.wait(timeout=2)
except Exception:
    proc.kill()

print("\n=== SPRINT 03 MCP & MORPHOLOGY VALIDATION COMPLETED SUCCESSFULLY (14/14 PASS) ===")
