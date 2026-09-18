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
  console.log('--- Switching to Shading ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[2].click()' });
  await sleep(800);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/workspace_shading.png' });
  console.log('Saved workspace_shading.png');

  console.log('--- Switching to Iluminação ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[3].click()' });
  await sleep(800);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/workspace_iluminacao.png' });
  console.log('Saved workspace_iluminacao.png');

  console.log('--- Switching to Biblioteca ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[7].click()' });
  await sleep(800);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/workspace_biblioteca.png' });
  console.log('Saved workspace_biblioteca.png');

  console.log('--- Switching back to Personagem ---');
  await send('EVAL_JS', { script: 'document.querySelectorAll(".tab-button")[0].click()' });
  await sleep(800);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/workspace_back_personagem.png' });
  console.log('Saved workspace_back_personagem.png');

  console.log('All workspace visual validation captures completed successfully!');
}

main().catch(console.error);
