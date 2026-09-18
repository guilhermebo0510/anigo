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

async function run() {
  const dump = await evalJsAndDump(`
    async () => {
      const input = document.querySelector('.search-input');
      const valBefore = input ? input.value : 'no-input';
      const clearBtn = document.querySelector('.clear-search');
      if (clearBtn) clearBtn.click();
      return { valBefore, hadClearBtn: !!clearBtn, valAfter: input ? input.value : '' };
    }
  `);
  console.log("Dump:", dump);
}

run().catch(console.error);
