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
  return await send('EVAL_JS', { script });
}

async function screenshot(path) {
  return await send('SCREENSHOT', { save_path: path });
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function main() {
  const op = process.argv[2] || 'status';
  if (op === 'status') {
    const st = await send('GET_STATUS');
    console.log('Status:', JSON.stringify(st, null, 2));
  } else if (op === 'shading') {
    console.log('Switching to Shading tab...');
    await evalJs('document.querySelectorAll(".tab-button")[2].click();');
    await sleep(300);
    await screenshot('c:/ANIGO/test_shading_tab.png');
    console.log('Screenshot saved to test_shading_tab.png');
  } else if (op === 'iluminacao') {
    console.log('Switching to Iluminacao tab...');
    await evalJs('const c = document.querySelector(".btn-close"); if(c) c.click();');
    await sleep(200);
    await evalJs('document.querySelectorAll(".tab-button")[3].click();');
    await sleep(400);
    await screenshot('c:/ANIGO/test_iluminacao_tab.png');
    console.log('Screenshot saved to test_iluminacao_tab.png');
  } else if (op === 'biblioteca') {
    console.log('Switching to Biblioteca tab...');
    await evalJs('const c = document.querySelector(".btn-close"); if(c) c.click();');
    await sleep(200);
    await evalJs('document.querySelectorAll(".tab-button")[7].click();');
    await sleep(400);
    await screenshot('c:/ANIGO/test_biblioteca_tab.png');
    console.log('Screenshot saved to test_biblioteca_tab.png');
  } else if (op === 'biblioteca-collapse') {
    console.log('Collapsing inspector in Biblioteca...');
    await evalJs('const c = document.querySelector(".btn-close"); if(c) c.click();');
    await sleep(200);
    await evalJs('document.querySelectorAll(".tab-button")[7].click();');
    await sleep(400);
    await evalJs('const b = document.querySelector(".btn-toggle-inspector"); if (b) b.click();');
    await sleep(400);
    await screenshot('c:/ANIGO/test_biblioteca_collapsed.png');
    console.log('Screenshot saved to test_biblioteca_collapsed.png');
  } else if (op === 'reload') {
    console.log('Reloading webview...');
    await evalJs('location.reload();');
    await sleep(1500);
    console.log('Reloaded!');
  } else if (op === 'select-asset') {
    console.log('Switching to Biblioteca...');
    await evalJs('document.querySelectorAll(".tab-button")[7].click();');
    await sleep(400);
    console.log('Selecting second asset (Ren Stylized Male)...');
    await evalJs('document.querySelectorAll(".asset-card")[1].click();');
    await sleep(400);
    await screenshot('c:/ANIGO/test_biblioteca_selected_ren.png');
    console.log('Screenshot saved to test_biblioteca_selected_ren.png');
  }
}

main().catch(console.error);
