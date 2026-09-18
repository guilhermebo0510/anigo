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
  // Ensure we are in Iluminação
  await evalJs(`document.querySelectorAll(".tab-button")[3].click();`);
  await sleep(200);

  // Get all range inputs in LightingControls
  const ranges = await evalJs(`
    (() => {
      const inputs = Array.from(document.querySelectorAll(".lighting-controls-container input[type='range']"));
      return inputs.map((inp, idx) => ({
        idx,
        min: inp.min,
        max: inp.max,
        val: inp.value
      }));
    })()
  `);
  console.log('Lighting range inputs:', ranges);

  // Test 1: Change Azimuth to 270 (slider 0)
  console.log('Setting Azimuth to 270...');
  await evalJs(`
    (() => {
      const inp = document.querySelectorAll(".lighting-controls-container input[type='range']")[0];
      inp.value = "270";
      inp.dispatchEvent(new Event("input", { bubbles: true }));
      inp.dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(200);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_light_azimuth_270.png' });

  // Test 2: Change Elevation to -30 (slider 1)
  console.log('Setting Elevation to -30...');
  await evalJs(`
    (() => {
      const inp = document.querySelectorAll(".lighting-controls-container input[type='range']")[1];
      inp.value = "-30";
      inp.dispatchEvent(new Event("input", { bubbles: true }));
      inp.dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(200);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_light_elevation_minus30.png' });

  // Test 3: Change Sun Intensity to 2.5 (slider 2)
  console.log('Setting Intensity to 2.5...');
  await evalJs(`
    (() => {
      const inp = document.querySelectorAll(".lighting-controls-container input[type='range']")[2];
      inp.value = "2.5";
      inp.dispatchEvent(new Event("input", { bubbles: true }));
      inp.dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(200);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_light_intensity_25.png' });

  // Test 4: Change Ambient to 1.2 (slider 5)
  console.log('Setting Ambient to 1.2...');
  await evalJs(`
    (() => {
      const inps = document.querySelectorAll(".lighting-controls-container input[type='range']");
      // Let's log how many sliders there are
      console.log("Total sliders in container:", inps.length);
      const amb = inps[4] || inps[inps.length - 1];
      amb.value = "1.2";
      amb.dispatchEvent(new Event("input", { bubbles: true }));
      amb.dispatchEvent(new Event("change", { bubbles: true }));
    })()
  `);
  await sleep(200);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_light_ambient_12.png' });
}

run().catch(console.error);
