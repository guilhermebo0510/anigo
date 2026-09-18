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
  console.log('1. Initial state...');
  let info = await evalJs(`
    () => {
      const canvas = document.querySelector('.viewport-canvas');
      const container = document.querySelector('.viewport-container');
      const inspector = document.querySelector('.right-inspector');
      return {
        canvasWidth: canvas.width,
        canvasHeight: canvas.height,
        canvasClientW: canvas.clientWidth,
        canvasClientH: canvas.clientHeight,
        containerW: container.clientWidth,
        containerH: container.clientHeight,
        inspectorW: inspector.clientWidth,
      };
    }
  `);
  console.log('Initial:', info);

  console.log('2. Simulating drag on resizer from 320 to 700...');
  await evalJs(`
    async () => {
      const resizer = document.querySelector('.inspector-resizer');
      const rect = resizer.getBoundingClientRect();
      const startX = rect.left + rect.width / 2;
      const startY = rect.top + rect.height / 2;

      resizer.dispatchEvent(new PointerEvent('pointerdown', {
        clientX: startX,
        clientY: startY,
        button: 0,
        bubbles: true,
        pointerId: 1
      }));

      for (let i = 1; i <= 30; i++) {
        const curX = startX - (i * 10);
        window.dispatchEvent(new PointerEvent('pointermove', {
          clientX: curX,
          clientY: startY,
          button: 0,
          bubbles: true,
          pointerId: 1
        }));
        await new Promise(r => setTimeout(r, 20));
      }
    }
  `);

  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\drag_midway.png' });

  info = await evalJs(`
    () => {
      const canvas = document.querySelector('.viewport-canvas');
      const container = document.querySelector('.viewport-container');
      const inspector = document.querySelector('.right-inspector');
      return {
        canvasWidth: canvas.width,
        canvasHeight: canvas.height,
        canvasClientW: canvas.clientWidth,
        canvasClientH: canvas.clientHeight,
        containerW: container.clientWidth,
        containerH: container.clientHeight,
        inspectorW: inspector.clientWidth,
      };
    }
  `);
  console.log('Midway during drag:', info);

  await evalJs(`
    () => {
      window.dispatchEvent(new PointerEvent('pointerup', {
        clientX: 500,
        clientY: 500,
        button: 0,
        bubbles: true,
        pointerId: 1
      }));
    }
  `);

  await new Promise(r => setTimeout(r, 500));
  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\drag_after_500ms.png' });

  info = await evalJs(`
    () => {
      const canvas = document.querySelector('.viewport-canvas');
      const container = document.querySelector('.viewport-container');
      const inspector = document.querySelector('.right-inspector');
      return {
        canvasWidth: canvas.width,
        canvasHeight: canvas.height,
        canvasClientW: canvas.clientWidth,
        canvasClientH: canvas.clientHeight,
        containerW: container.clientWidth,
        containerH: container.clientHeight,
        inspectorW: inspector.clientWidth,
      };
    }
  `);
  console.log('After 500ms:', info);

  await new Promise(r => setTimeout(r, 1500));
  await send('SCREENSHOT', { save_path: 'C:\\ANIGO\\drag_after_2000ms.png' });
}

run().catch(console.error);
