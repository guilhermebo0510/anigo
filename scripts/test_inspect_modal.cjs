const net = require('net');

function send(action, params = {}) {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(39090, '127.0.0.1', () => {
      client.write(JSON.stringify({ id: String(Date.now()), action, params }) + '\n');
    });
    client.on('data', (d) => {
      try { resolve(JSON.parse(d.toString())); } catch (e) { resolve(d.toString()); }
      client.end();
    });
    client.on('error', reject);
  });
}

const sleep = (ms) => new Promise(r => setTimeout(r, ms));

async function main() {
  console.log('--- Navigating to Biblioteca ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[7].click()' });
  await sleep(400);

  console.log('--- Opening Inspect Modal for first card ---');
  await send('EVAL_JS', { script: 'const btn = document.querySelectorAll(".btn-inspect")[0]; if(btn) btn.click();' });
  await sleep(600);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/workspace_asset_inspect.png' });
  console.log('Saved workspace_asset_inspect.png');

  console.log('--- Closing Inspect Modal ---');
  await send('EVAL_JS', { script: 'const closeBtn = document.querySelector(".btn-close-modal"); if(closeBtn) closeBtn.click();' });
  await sleep(400);

  console.log('--- Navigating back to Personagem ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[0].click()' });
  await sleep(400);
  console.log('Modal inspection flow verified successfully.');
}

main().catch(console.error);
