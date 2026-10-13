#!/usr/bin/env python3
"""Smoke de ENTRADA REAL do Shading pela bridge (substitui o `native_shading_smoke.sh`, que depende de xdotool/Xvfb e só roda em Linux).

Os eventos de mouse/teclado são injetados DENTRO do egui (`INPUT`): o cursor do sistema nunca é tocado e funciona em qualquer SO.
  1. arrasta o slider "Intensidade" do Rim -> o material muda ao vivo e UM passo de undo é gravado ao soltar;
  2. Ctrl+Z desfaz (valor E viewport idêntica, 0 px de diferença) e Ctrl+Y refaz;
  3. arrasta de novo (outro passo) e dois Ctrl+Z voltam ao estado inicial.

O app imprime `shadetrace ...` e `shaderect <param> x= y= w= h=` (ANIGO_TRACE_SHADING=1, em PONTOS lógicos); o script lê o log.
Uso: python scripts/bridge_shading_smoke.py [--bin target/debug/anigo-studio(.exe)] [--mcp target/debug/anigo-mcp(.exe)] [--out pasta]
Não mexe em instâncias abertas: se já houver uma bridge ativa, ABORTA (não fecha o app de ninguém).
"""
import argparse, json, os, re, subprocess, sys, tempfile, time

ap = argparse.ArgumentParser()
exe = '.exe' if os.name == 'nt' else ''
ap.add_argument('--bin', default=f'target/debug/anigo-studio{exe}')
ap.add_argument('--mcp', default=f'target/debug/anigo-mcp{exe}')
ap.add_argument('--out', default=os.path.join(tempfile.gettempdir(), 'anigo_shading_smoke'))
args = ap.parse_args()
args.bin, args.mcp, args.out = (os.path.abspath(p) for p in (args.bin, args.mcp, args.out))
os.makedirs(args.out, exist_ok=True)
LOG = os.path.join(args.out, 'smoke.log')
ENV = dict(os.environ, ANIGO_MCP_ALLOWED_DIRS=args.out)


def call(action, params=None, extra=()):
    r = subprocess.run([args.mcp, 'bridge-call', action, json.dumps(params or {}), *extra], capture_output=True, text=True, encoding='utf-8', errors='replace', env=ENV)
    try:
        return json.loads(r.stdout.strip())
    except Exception:
        return {'error': (r.stderr or r.stdout).strip()}


def fail(msg):
    print('FALHA:', msg)
    print('ultimo trace:', last_trace())
    sys.exit(1)


def last_trace():
    try:
        lines = [l for l in open(LOG, encoding='utf-8', errors='replace') if l.startswith('shadetrace')]
    except OSError:
        return ''
    return lines[-1].strip() if lines else ''


def field(line, name):
    m = re.search(rf' {name}=(\S+)', line)
    return m.group(1) if m else ''


def rect(name):
    try:
        lines = [l for l in open(LOG, encoding='utf-8', errors='replace') if l.startswith(f'shaderect {name} ')]
    except OSError:
        return None
    if not lines:
        return None
    return [float(field(lines[-1], k)) for k in ('x', 'y', 'w', 'h')]


def wait(desc, cond, tmo=30):
    t = time.time()
    while time.time() - t < tmo:
        tr = last_trace()
        if tr and cond(tr):
            return tr
        time.sleep(0.25)
    fail(f'tempo esgotado: {desc}')


def inp(*steps):
    r = call('INPUT', {'steps': list(steps)})
    if 'queued' not in r:
        fail(f'INPUT recusado: {r}')
    time.sleep(0.1)
    t = time.time()
    while time.time() - t < 30:
        if call('GET_UI_STATE').get('input_pending', 0) == 0:
            return
        time.sleep(0.05)
    fail('fila de entrada travada')


def shot(name):
    path = os.path.join(args.out, name + '.png')
    call('SCREENSHOT', {'target': 'viewport'}, ['--save-png', path])
    return path


def diff_px(a, b):
    from PIL import Image, ImageChops
    ia, ib = Image.open(a).convert('RGB'), Image.open(b).convert('RGB')
    if ia.size != ib.size:
        return 10 ** 9
    d = ImageChops.difference(ia, ib).convert('L')
    return sum(d.histogram()[9:])  # pixels com diferença > ~3% de um canal


# ---- sobe o app (sem sessão do usuário, biblioteca/paletas em pasta temporária)
if call('GET_STATUS').get('engine'):
    print('ABORTADO: já existe um ANIGO com bridge ativa; feche-o (este smoke não fecha o app de ninguém).')
    sys.exit(2)
disc = os.path.join(os.environ.get('LOCALAPPDATA', os.path.expanduser('~/.local/share')), 'ANIGO', 'bridge', 'bridge.json')
if os.path.exists(disc):
    os.remove(disc)  # arquivo velho de uma execução que morreu
sandbox = os.path.join(args.out, 'dados')
os.makedirs(sandbox, exist_ok=True)
env = dict(ENV, ANIGO_TRACE_SHADING='1', ANIGO_SHADING_DIR=sandbox, ANIGO_LIBRARY_DIR=os.path.join(sandbox, 'lib'), ANIGO_ASSETS_DIR=os.path.join(sandbox, 'assets'), ANIGO_RENDER_DIR=os.path.join(sandbox, 'renders'))
flags = {'creationflags': 0x00000008 | 0x00000200} if os.name == 'nt' else {}
app = subprocess.Popen([args.bin, '--bridge', '--no-session', '--workspace', 'shading'], stdout=open(LOG, 'w'), stderr=subprocess.STDOUT, env=env, **flags)
try:
    t = time.time()
    while time.time() - t < 90 and not call('GET_STATUS').get('engine'):
        time.sleep(0.5)
    if not call('GET_STATUS').get('engine'):
        fail('o app não subiu')
    time.sleep(2)
    call('SHADING', {'op': 'view', 'patch': {'focus_group': 'rim'}})  # abre a aba do grupo e expande a seção
    t = time.time()
    while time.time() - t < 30 and not rect('rim_intensity'):
        time.sleep(0.25)
    r = rect('rim_intensity')
    if not r:
        fail('o slider do rim não apareceu (aba Contorno/Rim)')
    x, y, w, h = r
    track = w - 74
    cy = y + h / 2
    t0 = wait('trace inicial', lambda t: True)
    u0, rim0 = int(field(t0, 'undo')), float(field(t0, 'rim'))
    print(f'slider rim: x={x:.0f} y={y:.0f} w={w:.0f} h={h:.0f} (rim {rim0}, undo {u0})')
    a = shot('a_antes')

    # ---- 1. arrastar o slider
    inp({'type': 'drag', 'from': [x + track * 0.15, cy], 'to': [x + track * 0.80, cy], 'frames': 14}, {'type': 'wait', 'frames': 3})
    t1 = wait('soltar grava UM passo e fecha o gesto', lambda t: int(field(t, 'undo')) == u0 + 1 and field(t, 'gesture') == 'false')
    rim1 = float(field(t1, 'rim'))
    print(f'rim: {rim0} -> {rim1}')
    if not rim1 > rim0 + 1.0:
        fail('o rim não aumentou o bastante')
    time.sleep(0.5)
    b = shot('b_depois')
    d = diff_px(a, b)
    print('diferença da viewport antes/depois do arrasto:', d, 'px')
    if d < 10:  # o rim só aparece nas bordas da bola/personagem: poucos pixels mudam
        fail('a viewport não mudou após o arrasto')

    # ---- 2. Ctrl+Z / Ctrl+Y (com o ponteiro FORA de campos de texto)
    inp({'type': 'move', 'pos': [x - 150, cy + 150]}, {'type': 'mods', 'mods': ['ctrl']}, {'type': 'key', 'key': 'Z'}, {'type': 'mods', 'mods': []}, {'type': 'wait', 'frames': 3})
    t2 = wait('Ctrl+Z desfaz', lambda t: int(field(t, 'undo')) == u0 and field(t, 'redo') == 'true')
    if abs(float(field(t2, 'rim')) - rim0) > 1e-3:
        fail(f'desfazer não devolveu o rim ({field(t2, "rim")} != {rim0})')
    time.sleep(0.5)
    c = shot('c_desfeito')
    d = diff_px(a, c)
    print('diferença após desfazer vs. antes:', d, 'px (esperado ~0)')
    if d > 5:
        fail('desfazer não restaurou a viewport')
    inp({'type': 'mods', 'mods': ['ctrl']}, {'type': 'key', 'key': 'Y'}, {'type': 'mods', 'mods': []}, {'type': 'wait', 'frames': 3})
    t3 = wait('Ctrl+Y refaz', lambda t: int(field(t, 'undo')) == u0 + 1 and field(t, 'redo') == 'false')
    if abs(float(field(t3, 'rim')) - rim1) > 1e-3:
        fail('refazer não devolveu o rim')
    time.sleep(0.5)
    dd = shot('d_refeito')
    d = diff_px(b, dd)
    print('diferença após refazer vs. depois do arrasto:', d, 'px (esperado ~0)')
    if d > 5:
        fail('refazer não restaurou a viewport')

    # ---- 3. segundo arrasto = OUTRO passo; dois Ctrl+Z voltam ao início
    r2 = rect('rim_intensity')
    if r2 != r:
        print('aviso: o slider mudou de lugar', r, '->', r2)
        x, y, w, h = r2
        track, cy = w - 74, y + h / 2
    inp({'type': 'drag', 'from': [x + track * 0.80, cy], 'to': [x + track * 0.30, cy], 'frames': 14}, {'type': 'wait', 'frames': 3})
    wait('o segundo arrasto grava outro passo', lambda t: int(field(t, 'undo')) == u0 + 2 and field(t, 'gesture') == 'false')
    inp({'type': 'move', 'pos': [x - 150, cy + 150]})
    for _ in range(2):
        inp({'type': 'mods', 'mods': ['ctrl']}, {'type': 'key', 'key': 'Z'}, {'type': 'mods', 'mods': []}, {'type': 'wait', 'frames': 6})
    t4 = wait('dois Ctrl+Z voltam ao estado inicial', lambda t: int(field(t, 'undo')) == u0)
    if abs(float(field(t4, 'rim')) - rim0) > 1e-3:
        fail('o rim não voltou ao inicial')
    time.sleep(0.5)
    e = shot('e_inicial')
    d = diff_px(a, e)
    print('diferença após desfazer tudo vs. inicial:', d, 'px (esperado ~0)')
    if d > 5:
        fail('desfazer tudo não restaurou a viewport')
    print('OK: arrasto de slider = UM passo de undo, Ctrl+Z/Ctrl+Y com entrada injetada e viewport idêntica antes/depois')
finally:
    app.kill()
