'use strict';

// Starts the real app and drives its window through the debugging protocol. Checks behaviour (attributes,
// stored choices, saved settings), not pixels.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { tmpDir, waitFor } = require('./helpers');

const ROOT = path.join(__dirname, '..');
let electronPath = null;
try {
  electronPath = require('electron');
} catch {
  // electron not installed: the tests below are skipped
}

const freePort = () =>
  new Promise((resolve) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

async function launch() {
  const userData = tmpDir();
  fs.writeFileSync(path.join(userData, 'downloads.json'), JSON.stringify({ settings: { dir: tmpDir(), connections: 5, confirmDownloads: false }, tasks: [] }));
  const port = await freePort();
  const app = spawn(electronPath, [ROOT, `--user-data-dir=${userData}`, `--remote-debugging-port=${port}`], { stdio: 'ignore' });
  let page = null;
  await waitFor(async () => {
    try {
      page = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => /index\.html/.test(t.url));
    } catch {
      return false;
    }
    return Boolean(page);
  }, 30000);
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve) => (ws.onopen = resolve));
  let n = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    pending.get(d.id)?.(d);
  };
  const ev = (expression) =>
    new Promise((resolve, reject) => {
      const id = ++n;
      pending.set(id, (d) => (d.error || d.result.exceptionDetails ? reject(new Error(JSON.stringify(d.error ?? d.result.exceptionDetails))) : resolve(d.result.result?.value)));
      ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
    });
  await waitFor(async () => (await ev('typeof appState !== "undefined" && appState !== null')) === true, 30000);
  const close = () => {
    ws.close();
    try {
      execFileSync('taskkill', ['/pid', String(app.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      app.kill();
    }
  };
  return { ev, close };
}

test('the window follows the appearance settings and the settings are saved', { skip: !electronPath && 'electron is not installed', timeout: 120000 }, async () => {
  const ui = await launch();
  try {
    // theme: a click on a choice applies it, remembers it and marks it checked
    await ui.ev(`document.getElementById('btn-settings').click()`);
    await ui.ev(`document.querySelector('#set-theme [data-v="dark"]').click()`);
    assert.equal(await ui.ev('document.documentElement.dataset.theme'), 'dark');
    assert.equal(await ui.ev(`localStorage.getItem('dm-theme')`), 'dark');
    assert.equal(await ui.ev(`document.querySelector('#set-theme [data-v="dark"]').getAttribute('aria-checked')`), 'true');
    assert.equal(await ui.ev(`document.querySelector('#set-theme [data-v="light"]').getAttribute('aria-checked')`), 'false');

    // accent: arrow keys move through the group and pick the next colour; only the checked one is a tab stop
    await ui.ev(`document.querySelector('#set-accent [data-v="violet"]').click(); document.querySelector('#set-accent [data-v="violet"]').focus()`);
    const before = await ui.ev('getComputedStyle(document.documentElement).getPropertyValue("--accent").trim()');
    await ui.ev(`document.querySelector('#set-accent [data-v="violet"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))`);
    assert.equal(await ui.ev('document.documentElement.dataset.accent'), 'blue');
    assert.notEqual(await ui.ev('getComputedStyle(document.documentElement).getPropertyValue("--accent").trim()'), before);
    assert.equal(await ui.ev(`document.activeElement.dataset.v`), 'blue', 'focus follows the choice');
    assert.equal(await ui.ev(`[...document.querySelectorAll('#set-accent button')].filter((b) => b.tabIndex === 0).length`), 1);

    // glass sliders change the CSS variables
    await ui.ev(`const r = document.getElementById('set-glass-blur'); r.value = 12; r.dispatchEvent(new Event('input', { bubbles: true }))`);
    assert.equal(await ui.ev('document.documentElement.style.getPropertyValue("--g-blur")'), '12px');

    // reset puts the defaults back
    await ui.ev(`document.getElementById('set-appearance-reset').click()`);
    assert.equal(await ui.ev('document.documentElement.dataset.accent'), 'violet');
    assert.equal(await ui.ev(`localStorage.getItem('dm-theme')`), null);

    // settings: the form shows what the engine has, and saving sends the edited value back
    assert.equal(await ui.ev(`document.getElementById('set-connections').value`), '5');
    await ui.ev(`document.getElementById('set-connections').value = '9'; document.getElementById('set-categories').checked = true; document.getElementById('form-settings').requestSubmit()`);
    await waitFor(async () => (await ui.ev('appState.settings.connections')) === 9, 10000);
    assert.equal(await ui.ev('appState.settings.categories'), true);
    await ui.ev(`document.getElementById('btn-settings').click()`);
    assert.equal(await ui.ev(`document.getElementById('set-connections').value`), '9', 'reopening shows the saved value');
  } finally {
    ui.close();
  }
});
