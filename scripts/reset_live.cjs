const net = require('net');

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

async function run() {
  const script = `
    const clearBtn = document.querySelector('.clear-search');
    if (clearBtn) clearBtn.click();
    const el = document.querySelector('.inspector-content');
    if (el) el.scrollTop = 0;
  `;
  await send('EVAL_JS', { script });
  await new Promise(r => setTimeout(r, 200));
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/live_final_ready.png' });
  console.log("Saved live_final_ready.png");
}

run().catch(console.error);
