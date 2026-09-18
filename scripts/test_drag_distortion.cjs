const { send, evalJs } = require('./bridge_utils.cjs');

async function run() {
  const attachResult = await evalJs(`
    () => {
      window.__dragDiagnostics = {
        errors: [],
        framesDuringDrag: 0,
        renderErrors: 0,
        aspectDeltas: [],
      };

      // Listen for window/tauri unhandled errors
      window.addEventListener('error', (e) => {
        window.__dragDiagnostics.errors.push(e.message);
      });

      return { attached: true };
    }
  `);
  console.log('Attached diagnostics:', attachResult);

  // Now simulate a fast continuous drag (120 movements in 200ms)
  const dragSim = await evalJs(`
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

      const canvas = document.querySelector('.viewport-canvas');
      const container = document.querySelector('.viewport-container');

      const deltas = [];

      for (let i = 0; i < 60; i++) {
        const curX = startX - (i * 8); // Drag 480px
        window.dispatchEvent(new PointerEvent('pointermove', {
          clientX: curX,
          clientY: startY,
          button: 0,
          bubbles: true,
          pointerId: 1
        }));

        // Check immediately after dispatch:
        const cW = canvas.clientWidth;
        const cH = canvas.clientHeight;
        const bW = canvas.width;
        const bH = canvas.height;
        const displayAspect = cW / Math.max(cH, 1);
        const bufferAspect = bW / Math.max(bH, 1);
        deltas.push({
          step: i,
          cW, cH, bW, bH,
          displayAspect: Number(displayAspect.toFixed(3)),
          bufferAspect: Number(bufferAspect.toFixed(3)),
          ratio: Number((displayAspect / bufferAspect).toFixed(3))
        });

        // wait 5ms between moves (fast mouse swipe)
        await new Promise(r => setTimeout(r, 5));
      }

      window.dispatchEvent(new PointerEvent('pointerup', {
        clientX: startX - 480,
        clientY: startY,
        button: 0,
        bubbles: true,
        pointerId: 1
      }));

      return { deltas: deltas.filter(d => d.ratio !== 1.0) };
    }
  `);

  console.log('Mismatched deltas during drag:');
  console.log(JSON.stringify(dragSim?.deltas?.slice(0, 15), null, 2));
  console.log('Total steps with distortion:', dragSim?.deltas?.length);
}

run().catch(console.error);
