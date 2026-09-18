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
  if (fs.existsSync('c:/ANIGO/debug_eval.json')) {
    try { fs.unlinkSync('c:/ANIGO/debug_eval.json'); } catch (_) {}
  }
  const wrapped = `
    (async () => {
      try {
        const res = await (${script})();
        const contentStr = JSON.stringify(res === undefined ? { status: "ok" } : res, null, 2);
        await window.__TAURI_INTERNALS__.invoke("save_project_file", {
          path: "C:\\\\ANIGO\\\\debug_eval.json",
          content: contentStr
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
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 100));
    if (fs.existsSync('c:/ANIGO/debug_eval.json')) {
      try {
        return JSON.parse(fs.readFileSync('c:/ANIGO/debug_eval.json', 'utf8'));
      } catch (_) {}
    }
  }
  return null;
}

async function run() {
  const setup = await evalJs(`
    () => {
      window.__resizeLog = [];
      const canvas = document.querySelector('.viewport-canvas');
      const container = document.querySelector('.viewport-container');

      window.__monitorInterval = setInterval(() => {
        if (!window.__resizeLog) return;
        const box = container.getBoundingClientRect();
        const aspectBuffer = canvas.width / Math.max(canvas.height, 1);
        const aspectDisplay = canvas.clientWidth / Math.max(canvas.clientHeight, 1);
        window.__resizeLog.push({
          time: performance.now(),
          canvasW: canvas.width,
          canvasH: canvas.height,
          canvasClientW: canvas.clientWidth,
          canvasClientH: canvas.clientHeight,
          containerW: box.width,
          containerH: box.height,
          aspectBuffer: Number(aspectBuffer.toFixed(4)),
          aspectDisplay: Number(aspectDisplay.toFixed(4)),
          diff: Number(Math.abs(aspectBuffer - aspectDisplay).toFixed(4))
        });
      }, 16);
      return { started: true };
    }
  `);
  console.log('Setup monitor:', setup);

  console.log('Dragging resizer continuously across 60 frames...');
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

      for (let i = 1; i <= 60; i++) {
        const delta = Math.sin(i / 5) * 200;
        window.dispatchEvent(new PointerEvent('pointermove', {
          clientX: startX - delta,
          clientY: startY,
          button: 0,
          bubbles: true,
          pointerId: 1
        }));
        await new Promise(r => setTimeout(r, 16));
      }

      window.dispatchEvent(new PointerEvent('pointerup', {
        clientX: startX,
        clientY: startY,
        button: 0,
        bubbles: true,
        pointerId: 1
      }));
      return { dragged: true };
    }
  `);

  await new Promise(r => setTimeout(r, 300));

  const logData = await evalJs(`
    () => {
      clearInterval(window.__monitorInterval);
      const log = window.__resizeLog || [];
      const mismatches = log.filter(e => e.diff > 0.02);
      return {
        totalSamples: log.length,
        mismatchCount: mismatches.length,
        sampleMismatches: mismatches.slice(0, 5),
        firstSamples: log.slice(0, 3),
        lastSamples: log.slice(-3)
      };
    }
  `);

  console.log('Results:', JSON.stringify(logData, null, 2));
}

run().catch(console.error);
