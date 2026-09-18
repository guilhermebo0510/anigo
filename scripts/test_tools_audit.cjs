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
  // Test Rim Light Tool
  console.log('--- Testing Rim Light Tool (tool index 1 in Shading) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[1].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_rim.png' });

  // Test Outline Tool (tool index 2 in Shading)
  console.log('--- Testing Outline Tool (tool index 2 in Shading) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[2].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_outline.png' });

  // Test Palette Tool (tool index 3 in Shading)
  console.log('--- Testing Palette Tool (tool index 3 in Shading) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[3].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_palette.png' });

  // Test Shader Ball Tool (tool index 4 in Shading)
  console.log('--- Testing Shader Ball Tool (tool index 4 in Shading) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[4].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_shader_ball.png' });

  // Now switch to Iluminação tab!
  console.log('--- Switching to Iluminacao Workspace ---');
  await evalJs(`document.querySelectorAll(".tab-button")[3].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_iluminacao_0.png' });

  // Check tools in Iluminação: index 0 (sun), 1 (shadows), 2 (ambient)
  console.log('--- Testing Iluminacao tool 1 (shadows) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[1].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_iluminacao_1.png' });

  console.log('--- Testing Iluminacao tool 2 (ambient) ---');
  await evalJs(`document.querySelectorAll(".contextual-tools .tool-btn")[2].click();`);
  await sleep(300);
  await send('SCREENSHOT', { save_path: 'c:/ANIGO/test_tool_iluminacao_2.png' });
}

run().catch(console.error);
