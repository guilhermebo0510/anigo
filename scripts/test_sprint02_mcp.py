import subprocess
import json
import os
import sys

print("=== Starting Sprint 02 NPR Cel-Shading & MCP Validation ===")

proc = subprocess.Popen(
    ["target/debug/anigo-mcp.exe"],
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

# 2. Tools List
tools_resp = send_req(2, "tools/list", {})
tool_names = [t["name"] for t in tools_resp.get("result", {}).get("tools", [])]
print(f"[2] Registered {len(tool_names)} tools:")
assert "anigo_set_light" in tool_names, "Missing anigo_set_light"
assert "anigo_set_material_toon" in tool_names, "Missing anigo_set_material_toon"
assert "anigo_compare_baseline" in tool_names, "Missing anigo_compare_baseline"
print("    [OK] anigo_set_light verified")
print("    [OK] anigo_set_material_toon verified")
print("    [OK] anigo_compare_baseline verified")

# 3. System Info
sys_resp = send_req(3, "tools/call", {"name": "anigo_get_system_info", "arguments": {}})
print("[3] System Info:\n", sys_resp["result"]["content"][0]["text"])

# 4. Set Directional Light & Anime Shadow Color
light_args = {
    "direction": [0.6, 0.8, 0.5],
    "intensity": 1.35,
    "color": [1.0, 0.98, 0.92],
    "ambient_intensity": 0.35,
    "shadow_color": [0.58, 0.62, 0.88]
}
light_resp = send_req(4, "tools/call", {"name": "anigo_set_light", "arguments": light_args})
print("[4] anigo_set_light response:\n", light_resp["result"]["content"][0]["text"])

# 5. Set Stylized Toon Material (1-Step Hard Anime Cel)
mat_cel_args = {
    "base_color": [0.98, 0.92, 0.86, 1.0],
    "shade_color": [0.80, 0.72, 0.85, 1.0],
    "outline_color": [0.22, 0.14, 0.20, 1.0],
    "outline_width": 0.0035,
    "shadow_threshold": 0.50,
    "shadow_smoothness": 0.015,
    "spec_intensity": 0.65,
    "spec_power": 48.0,
    "rim_intensity": 1.20,
    "rim_spread": 0.35,
    "hue_shift": -20.0,
    "toon_steps": 1.0
}
mat_resp = send_req(5, "tools/call", {"name": "anigo_set_material_toon", "arguments": mat_cel_args})
print("[5] anigo_set_material_toon (Hard Cel):\n", mat_resp["result"]["content"][0]["text"])

# 6. Render Hard Cel Frame
cel_img_path = "baselines/test_render_sprint02_cel.png"
render_cel = send_req(6, "tools/call", {"name": "anigo_render_frame", "arguments": {
    "width": 800,
    "height": 600,
    "save_path": cel_img_path
}})
print(f"[6] Rendered Hard Cel Frame: {cel_img_path}")
print("    Metrics:", render_cel["result"]["content"][0]["text"])
assert os.path.exists(cel_img_path), f"File {cel_img_path} was not created"
assert os.path.getsize(cel_img_path) > 1000, f"File {cel_img_path} is too small"

# 7. Set Stylized Toon Material (2-Tier Ghibli Soft Ramp)
mat_ghibli_args = {
    "shadow_threshold": 0.48,
    "shadow_smoothness": 0.04,
    "spec_intensity": 0.40,
    "spec_power": 24.0,
    "rim_intensity": 0.90,
    "rim_spread": 0.45,
    "hue_shift": -15.0,
    "toon_steps": 2.0
}
mat_ghibli_resp = send_req(7, "tools/call", {"name": "anigo_set_material_toon", "arguments": mat_ghibli_args})
print("[7] anigo_set_material_toon (2-Tier Ghibli):\n", mat_ghibli_resp["result"]["content"][0]["text"])

# 8. Render Ghibli Frame
ghibli_img_path = "baselines/test_render_sprint02_ghibli.png"
render_ghibli = send_req(8, "tools/call", {"name": "anigo_render_frame", "arguments": {
    "width": 800,
    "height": 600,
    "save_path": ghibli_img_path
}})
print(f"[8] Rendered Ghibli Frame: {ghibli_img_path}")
print("    Metrics:", render_ghibli["result"]["content"][0]["text"])
assert os.path.exists(ghibli_img_path), f"File {ghibli_img_path} was not created"

# 9. Test anigo_compare_baseline: Self-comparison
cmp_self = send_req(9, "tools/call", {"name": "anigo_compare_baseline", "arguments": {
    "baseline_path": cel_img_path,
    "current_image_path": cel_img_path,
    "mse_threshold": 1.0,
    "psnr_threshold": 50.0
}})
cmp_self_data = json.loads(cmp_self["result"]["content"][0]["text"])
print("[9] anigo_compare_baseline (Self-Identity):\n", json.dumps(cmp_self_data, indent=2))
assert cmp_self_data["passed"] is True, "Self comparison should pass with 100% match"
assert cmp_self_data["mse"] == 0.0, f"MSE should be 0.0, got {cmp_self_data['mse']}"
assert cmp_self_data["matching_pixel_percent"] == 100.0, "Matching percent should be 100%"

# 10. Test anigo_compare_baseline: Compare Cel vs Ghibli with diff generation
diff_path = "baselines/diff_cel_vs_ghibli.png"
cmp_diff = send_req(10, "tools/call", {"name": "anigo_compare_baseline", "arguments": {
    "baseline_path": cel_img_path,
    "current_image_path": ghibli_img_path,
    "diff_save_path": diff_path,
    "mse_threshold": 500.0,
    "psnr_threshold": 20.0
}})
cmp_diff_data = json.loads(cmp_diff["result"]["content"][0]["text"])
print("[10] anigo_compare_baseline (Cel vs Ghibli):\n", json.dumps(cmp_diff_data, indent=2))
assert os.path.exists(diff_path), f"Diff map {diff_path} was not created"
print(f"    [OK] Visual difference map generated at: {diff_path}")

# 11. Test anigo_compare_baseline: Live headless render against baseline
cmp_headless = send_req(11, "tools/call", {"name": "anigo_compare_baseline", "arguments": {
    "baseline_path": ghibli_img_path,
    "mse_threshold": 2.0,
    "psnr_threshold": 40.0
}})
cmp_headless_data = json.loads(cmp_headless["result"]["content"][0]["text"])
print("[11] anigo_compare_baseline (Direct Headless Render vs Baseline):\n", json.dumps(cmp_headless_data, indent=2))
assert cmp_headless_data["passed"] is True, "Live headless render of current scene should match its saved frame"

# 12. Test Sphere Mesh Preset with Cel-Shading
sphere_resp = send_req(12, "tools/call", {"name": "anigo_load_mesh_preset", "arguments": {"preset": "sphere"}})
print("[12] anigo_load_mesh_preset (Sphere):\n", sphere_resp["result"]["content"][0]["text"])

sphere_img_path = "baselines/test_render_sprint02_sphere.png"
render_sphere = send_req(13, "tools/call", {"name": "anigo_render_frame", "arguments": {
    "width": 800,
    "height": 600,
    "save_path": sphere_img_path
}})
print(f"[13] Rendered Sphere Cel Frame: {sphere_img_path}")
print("    Metrics:", render_sphere["result"]["content"][0]["text"])
assert os.path.exists(sphere_img_path), f"File {sphere_img_path} was not created"

proc.terminate()
print("\n=== ALL SPRINT 02 MCP TESTS PASSED WITH 100% INTEGRITY ===")
