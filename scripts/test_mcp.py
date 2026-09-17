import subprocess
import json
import os

print("Testing anigo-mcp executable...")

proc = subprocess.Popen(
    ["target/debug/anigo-mcp.exe"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True
)

requests = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "anigo_get_system_info", "arguments": {}}},
    {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "anigo_inspect_scene", "arguments": {}}},
    {"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "anigo_render_frame", "arguments": {"width": 640, "height": 480, "save_path": "baselines/test_render_sprint01.png"}}}
]

for req in requests:
    req_str = json.dumps(req)
    proc.stdin.write(req_str + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        stderr_output = proc.stderr.read()
        print("Subprocess closed unexpectedly. Stderr:", stderr_output)
        break
    data = json.loads(line)
    req_id = req["id"]
    print(f"=== RESPONSE FOR ID {req_id} ===")
    if req_id == 1:
        print("Initialized Server:", data.get("result", {}).get("serverInfo"))
    elif req_id == 2:
        tools = [t["name"] for t in data.get("result", {}).get("tools", [])]
        print("Discovered Tools:", tools)
    elif req_id == 3:
        print("GPU System Info:\n", data["result"]["content"][0]["text"])
    elif req_id == 4:
        print("Scene Summary:\n", data["result"]["content"][0]["text"])
    elif req_id == 5:
        print("Render Result:\n", data["result"]["content"][0]["text"])
        has_image = len(data["result"]["content"]) > 1 and data["result"]["content"][1].get("type") == "image"
        print("Image payload attached:", has_image)

proc.terminate()
print("\nTest completed successfully!")
