const net = require('net');
const fs = require('fs');

function send(action, params = {}) {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(39090, '127.0.0.1', () => {
      client.write(JSON.stringify({ id: String(Date.now()), action, params }) + '\n');
    });
    client.on('data', (data) => {
      try {
        resolve(JSON.parse(data.toString()));
      } catch (e) {
        resolve(data.toString());
      }
      client.end();
    });
    client.on('error', reject);
  });
}

async function evalJs(script) {
  if (fs.existsSync('c:/ANIGO/debug_eval.json')) {
    try { fs.unlinkSync('c:/ANIGO/debug_eval.json'); } catch (_) {}
  }
  const wrapped = `
    (async () => {
      try {
        const res = await (${script})();
        const contentStr = JSON.stringify(res === undefined ? { status: "ok" } : res, null, 2);
        await window.__TAURI_INTERNALS__.invoke("save_project_file", {
          path: "C:\\\\ANIGO\\\\debug_eval.json",
          content: contentStr
        });
      } catch (err) {
        await window.__TAURI_INTERNALS__.invoke("save_project_file", {
          path: "C:\\\\ANIGO\\\\debug_eval.json",
          content: JSON.stringify({ error: String(err), stack: err.stack }, null, 2)
        });
      }
    })()
  `;
  await send('EVAL_JS', { script: wrapped });
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 100));
    if (fs.existsSync('c:/ANIGO/debug_eval.json')) {
      try {
        return JSON.parse(fs.readFileSync('c:/ANIGO/debug_eval.json', 'utf8'));
      } catch (_) {}
    }
  }
  return null;
}

async function run() {
  console.log('1. Setting inspector width to 600px...');
  await send('UI_ACTION', { action: 'set_inspector_width', width: 600 });
  await new Promise(r => setTimeout(r, 300));
  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\test_inspector_600px.png' });

  console.log('2. Setting inspector width to 850px...');
  await send('UI_ACTION', { action: 'set_inspector_width', width: 850 });
  await new Promise(r => setTimeout(r, 300));
  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\test_inspector_850px.png' });

  console.log('3. Setting inspector width back to 320px...');
  await send('UI_ACTION', { action: 'set_inspector_width', width: 320 });
  await new Promise(r => setTimeout(r, 300));
  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\test_inspector_320px.png' });

  console.log('All visual screenshots saved successfully.');
}

run().catch(console.error);
