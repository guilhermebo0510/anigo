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

async function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function run() {
  // Go to Shading -> cel_shader
  await evalJs(`
    document.querySelectorAll(".tab-button")[2].click();
    setTimeout(() => {
      document.querySelectorAll(".contextual-tools .tool-btn")[0].click();
    }, 100);
  `);
  await sleep(300);

  // Set specIntensity to 0.0
  console.log('Setting specIntensity = 0.0 on mannequin...');
  await evalJs(`
    (() => {
      const inps = document.querySelectorAll(".right-inspector input[type='range']");
      // inps[2] is specIntensity
      inps[2].value = "0.0";
      inps[2].dispatchEvent(new Event("input", { bubbles: true }));
      inps[2].dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_spec_0.png' });

  // Set specIntensity to 2.5
  console.log('Setting specIntensity = 2.5 on mannequin...');
  await evalJs(`
    (() => {
      const inps = document.querySelectorAll(".right-inspector input[type='range']");
      inps[2].value = "2.5";
      inps[2].dispatchEvent(new Event("input", { bubbles: true }));
      inps[2].dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_spec_25.png' });

  const b0 = fs.readFileSync('c:/ANIGO/test_spec_0.png');
  const b25 = fs.readFileSync('c:/ANIGO/test_spec_25.png');
  console.log('specIntensity 0 vs 2.5 bytes match?', b0.equals(b25) ? 'IDENTICAL (SLIDER DOES NOTHING ON MANNEQUIN!)' : 'DIFFERENT');
}

run().catch(console.error);
