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

async function evalJsAndDump(script) {
  const wrapped = `
    (async () => {
      try {
        const res = await (${script})();
        await window.__TAURI_INTERNALS__.invoke("save_project_file", {
          path: "C:\\\\ANIGO\\\\debug_eval.json",
          content: JSON.stringify(res, null, 2)
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
  await new Promise((r) => setTimeout(r, 400));
  if (fs.existsSync('c:/ANIGO/debug_eval.json')) {
    return JSON.parse(fs.readFileSync('c:/ANIGO/debug_eval.json', 'utf8'));
  }
  return null;
}

async function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function run() {
  // First, set elevation back to 45 so we have normal sunlight
  console.log('Resetting sun to elevation 45, azimuth 45, intensity 1.0...');
  await evalJsAndDump(`
    async () => {
      const inps = document.querySelectorAll(".lighting-controls-container input[type='range']");
      inps[0].value = "45";
      inps[0].dispatchEvent(new Event("input", { bubbles: true }));
      inps[0].dispatchEvent(new Event("change", { bubbles: true }));

      inps[1].value = "45";
      inps[1].dispatchEvent(new Event("input", { bubbles: true }));
      inps[1].dispatchEvent(new Event("change", { bubbles: true }));

      inps[2].value = "1.0";
      inps[2].dispatchEvent(new Event("input", { bubbles: true }));
      inps[2].dispatchEvent(new Event("change", { bubbles: true }));

      return { reset: true };
    }
  `);
  await sleep(300);

  // Now test ambient = 0.0 vs ambient = 1.5
  console.log('Setting ambient to 0.0...');
  await evalJsAndDump(`
    async () => {
      const inps = document.querySelectorAll(".lighting-controls-container input[type='range']");
      const amb = inps[5];
      amb.value = "0.0";
      amb.dispatchEvent(new Event("input", { bubbles: true }));
      amb.dispatchEvent(new Event("change", { bubbles: true }));
      return { ambVal: amb.value };
    }
  `);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_ambient_0.png' });

  console.log('Setting ambient to 1.5...');
  await evalJsAndDump(`
    async () => {
      const inps = document.querySelectorAll(".lighting-controls-container input[type='range']");
      const amb = inps[5];
      amb.value = "1.5";
      amb.dispatchEvent(new Event("input", { bubbles: true }));
      amb.dispatchEvent(new Event("change", { bubbles: true }));
      return { ambVal: amb.value };
    }
  `);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_ambient_15.png' });

  // Compare file sizes and byte difference
  const buf0 = fs.readFileSync('c:/ANIGO/test_ambient_0.png');
  const buf15 = fs.readFileSync('c:/ANIGO/test_ambient_15.png');
  console.log('ambient 0 vs 15 bytes match?', buf0.equals(buf15) ? 'IDENTICAL (FAIL!)' : 'DIFFERENT (PASS)');
}

run().catch(console.error);
