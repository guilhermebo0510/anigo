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

async function run() {
  const res = await evalJs(`
    (() => {
      const inp = document.querySelectorAll(".lighting-controls-container input[type='range']")[0];
      const valTag = inp.parentElement.querySelector('.val-tag').textContent;
      
      // Now let's change inp.value and dispatch input
      inp.value = "180";
      inp.dispatchEvent(new Event("input", { bubbles: true }));
      
      const valTagAfterInput = inp.parentElement.querySelector('.val-tag').textContent;
      
      inp.dispatchEvent(new Event("change", { bubbles: true }));
      const valTagAfterChange = inp.parentElement.querySelector('.val-tag').textContent;
      
      return { valTag, valTagAfterInput, valTagAfterChange };
    })()
  `);
  console.log('Result:', JSON.stringify(res, null, 2));
}

run().catch(console.error);
