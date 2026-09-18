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

async function run() {
  const action = process.argv[2];
  let params = {};
  if (process.argv[3]) {
    try {
      params = JSON.parse(process.argv[3]);
    } catch {
      params = { arg: process.argv[3] };
    }
  }
  const res = await send(action, params);
  console.log(JSON.stringify(res, null, 2));
}

run().catch(console.error);
