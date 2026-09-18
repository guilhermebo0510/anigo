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
  await new Promise(r => setTimeout(r, 1200));
  await send('EVAL_JS', { script: `
    const btn = document.querySelector('.btn-proceed');
    if (btn) btn.click();
  ` });
  await new Promise(r => setTimeout(r, 300));
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/live_final_fresh.png' });
  console.log("Captured live_final_fresh.png");
}

run().catch(console.error);
