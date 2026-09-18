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

async function evalJs(script) {
  return await send('EVAL_JS', { script });
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function run() {
  console.log('--- Inspecting DOM in Shading Tab ---');
  // First make sure we are in shading tab
  await evalJs(`
    (() => {
      document.querySelectorAll(".tab-button")[2].click();
    })()
  `);
  await sleep(300);

  // Check what inputs are present in the DOM
  const inputsInfo = await evalJs(`
    (() => {
      const inputs = Array.from(document.querySelectorAll("input"));
      window.__inputs_debug = inputs.map((inp, idx) => ({
        idx,
        type: inp.type,
        min: inp.min,
        max: inp.max,
        step: inp.step,
        value: inp.value,
        parentClass: inp.parentElement ? inp.parentElement.className : '',
        label: inp.parentElement ? (inp.parentElement.querySelector('.label') ? inp.parentElement.querySelector('.label').textContent : '') : ''
      }));
      return window.__inputs_debug;
    })()
  `);
  console.log('Inputs found in DOM:');

  const check = await evalJs(`window.__inputs_debug`);
  console.log(JSON.stringify(check, null, 2));

  // Now test dispatching an input event on the first range input (Ponto de Corte Sombra)
  console.log('--- Testing slider change on shadow threshold ---');
  const res = await evalJs(`
    (() => {
      const range = document.querySelectorAll("input[type='range']")[0];
      if (!range) return { error: "No range input found" };
      const oldVal = range.value;
      range.value = "0.85";
      range.dispatchEvent(new Event("input", { bubbles: true }));
      range.dispatchEvent(new Event("change", { bubbles: true }));
      return { oldVal, newVal: range.value };
    })()
  `);
  console.log('Slider change dispatched:', res);
  await sleep(300);

  const status = await send('GET_STATUS');
  console.log('LiveWindowState shadow_threshold:', status.data ? status.data.shadow_threshold : status);

  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_slider_thresh_085.png' });
}

run().catch(console.error);
