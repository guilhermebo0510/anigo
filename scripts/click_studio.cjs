const net = require('net');
const client = net.createConnection(39090, '127.0.0.1', () => {
  client.write(JSON.stringify({
    id: 'eval_close',
    action: 'EVAL_JS',
    params: { script: 'const btn = document.querySelector(".btn-proceed"); if (btn) btn.click();' }
  }) + '\n');
});
client.on('data', (d) => {
  console.log('RESULT:', d.toString());
  client.end();
});
