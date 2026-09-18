const { send, evalJs } = require('./bridge_utils.cjs');

async function run() {
  const res = await evalJs(`
    () => {
      const c = document.querySelector('.viewport-canvas');
      return {
        canvasW: c.width,
        canvasH: c.height,
        clientW: c.clientWidth,
        clientH: c.clientHeight,
        hasWebGPU: !!navigator.gpu,
      };
    }
  `);
  console.log('Canvas State:', res);
}

run().catch(console.error);
