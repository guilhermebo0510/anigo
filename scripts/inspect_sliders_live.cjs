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
  const res1 = await evalJsAndDump(`
    async () => {
      const clearBtn = document.querySelector('.clear-search');
      if (clearBtn) {
        clearBtn.click();
      }
      const input = document.querySelector('.search-input');
      if (input && input.value) {
        input.value = '';
        input.dispatchEvent(new Event('input', { bubbles: true }));
      }
      
      await new Promise(r => setTimeout(r, 100));

      const headers = Array.from(document.querySelectorAll('.zone-header')).map(h => {
        const name = h.querySelector('.zone-name')?.textContent || '';
        const badge = h.querySelector('.slider-count-badge')?.textContent || '';
        return { name, count: parseInt(badge, 10) };
      });

      return { totalAccordions: headers.length, headers };
    }
  `);

  console.log("Accordions:", JSON.stringify(res1, null, 2));

  await send('SCREENSHOT', { save_path: 'c:/ANIGO/live_all_18_accordions.png' });
  console.log("Captured live_all_18_accordions.png");
}

run().catch(console.error);
