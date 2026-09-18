const net = require('net');

function send(action, params = {}) {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(39090, '127.0.0.1', () => {
      client.write(JSON.stringify({ id: String(Date.now()), action, params }) + '\n');
    });
    client.on('data', (data) => {
      try {
        const json = JSON.parse(data.toString());
        resolve(json);
      } catch (e) {
        resolve(data.toString());
      }
      client.end();
    });
    client.on('error', reject);
  });
}

async function main() {
  const cmd = process.argv[2] || 'screenshot';
  if (cmd === 'reload') {
    const res = await send('RELOAD_WEBVIEW');
    console.log('Reloaded WebView:', res);
  } else if (cmd === 'dismiss_quickstart') {
    const res = await send('EVAL_JS', { script: 'const b = document.querySelector(".btn-proceed"); if (b) b.click();' });
    console.log('Dismissed QuickStart:', res);
  } else if (cmd === 'click_project') {
    const res = await send('EVAL_JS', { script: 'document.querySelector(".status-project").click()' });
    console.log('Clicked project button:', res);
  } else if (cmd === 'click_model') {
    const res = await send('EVAL_JS', { script: 'document.querySelector(".status-model").click()' });
    console.log('Clicked model button:', res);
  } else if (cmd === 'choose_sphere') {
    const res = await send('EVAL_JS', { script: 'const cards = document.querySelectorAll(".preset-card"); if (cards[1]) cards[1].click();' });
    console.log('Selected sphere preset:', res);
  } else if (cmd === 'click_undo') {
    const res = await send('EVAL_JS', { script: 'const btn = document.querySelectorAll(".hist-btn")[0]; if (btn) btn.click();' });
    console.log('Clicked Undo button:', res);
  } else if (cmd === 'click_maximize') {
    const res = await send('EVAL_JS', { script: 'const btn = document.querySelector(".win-max"); if (btn) btn.click();' });
    console.log('Clicked Maximize button:', res);
  } else if (cmd === 'click_minimize') {
    const res = await send('EVAL_JS', { script: 'const btn = document.querySelector(".win-min"); if (btn) btn.click();' });
    console.log('Clicked Minimize button:', res);
  } else if (cmd === 'close_popovers') {
    const res = await send('EVAL_JS', { script: 'const b = document.querySelector(".btn-close"); if (b) b.click();' });
    console.log('Closed popovers:', res);
  } else if (cmd === 'screenshot') {
    const filename = process.argv[3] || 'c:/ANIGO/live_screenshot_test.png';
    const res = await send('SCREENSHOT', { save_path: filename });
    console.log('Screenshot saved:', res);
  } else if (cmd === 'undo') {
    const res = await send('EVAL_JS', { script: 'window.dispatchEvent(new KeyboardEvent("keydown", {key: "z", ctrlKey: true, bubbles: true}))' });
    console.log('Undo sent:', res);
  } else if (cmd === 'redo') {
    const res = await send('EVAL_JS', { script: 'window.dispatchEvent(new KeyboardEvent("keydown", {key: "y", ctrlKey: true, bubbles: true}))' });
    console.log('Redo sent:', res);
  } else if (cmd === 'test_slider') {
    const res = await send('UI_ACTION', { action: 'set_slider', property: 'head_scale', value: 1.3 });
    console.log('Set slider head_scale to 1.3:', res);
  } else if (cmd === 'test_save_load') {
    const res = await send('EVAL_JS', {
      script: `
        (async () => {
          try {
            const path = "C:\\\\ANIGO\\\\test_project.anigo";
            const content = JSON.stringify({ preset: "sphere", test: true, timestamp: Date.now() });
            const saved = await window.__TAURI_INTERNALS__.invoke("save_project_file", { path, content });
            const loaded = await window.__TAURI_INTERNALS__.invoke("load_project_file", { path });
            console.log("SAVE_LOAD_TEST_SUCCESS:", loaded === content ? "MATCH" : "MISMATCH");
            window.__last_save_test = loaded === content;
          } catch (e) {
            console.error("SAVE_LOAD_TEST_ERROR:", e);
          }
        })()
      `
    });
    console.log('Dispatched save/load test:', res);
  }
}

main().catch(console.error);
