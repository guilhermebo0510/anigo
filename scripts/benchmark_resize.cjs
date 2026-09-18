const { send, evalJs } = require('./bridge_utils.cjs');

async function run() {
  const res = await evalJs(`
    () => {
      const canvas = document.querySelector('.viewport-canvas');
      const renderer = window.__anigo_renderer || null; // let's see if renderer is exposed

      const times = [];
      const startW = canvas.width;
      const startH = canvas.height;

      // Let's measure setting canvas.width repeatedly
      const t0 = performance.now();
      for (let i = 0; i < 60; i++) {
        const tStep0 = performance.now();
        canvas.width = startW - i;
        canvas.height = startH;
        times.push(performance.now() - tStep0);
      }
      const totalTime = performance.now() - t0;
      canvas.width = startW;
      canvas.height = startH;

      return {
        totalTimeMs: totalTime,
        avgStepMs: totalTime / 60,
        maxStepMs: Math.max(...times),
        minStepMs: Math.min(...times)
      };
    }
  `);
  console.log('Benchmark canvas.width change:', res);
}

run().catch(console.error);
