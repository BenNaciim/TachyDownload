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
  const inspectPort = await freePort(); // the main process, to reach the menu bar
  const app = spawn(electronPath, [ROOT, `--user-data-dir=${userData}`, `--remote-debugging-port=${port}`, `--inspect=${inspectPort}`], { stdio: 'ignore' });
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
  // Runs code in the main process (where the real menu lives) and returns its value.
  let mainWs = null;
  let mainN = 0;
  const mainPending = new Map();
  const main = async (expression) => {
    if (!mainWs) {
      const target = (await (await fetch(`http://127.0.0.1:${inspectPort}/json/list`)).json())[0];
      mainWs = new WebSocket(target.webSocketDebuggerUrl);
      await new Promise((resolve) => (mainWs.onopen = resolve));
      mainWs.onmessage = (m) => {
        const d = JSON.parse(m.data);
        mainPending.get(d.id)?.(d);
      };
    }
    return new Promise((resolve, reject) => {
      const id = ++mainN;
      mainPending.set(id, (d) => (d.error || d.result.exceptionDetails ? reject(new Error(JSON.stringify(d.error ?? d.result.exceptionDetails))) : resolve(d.result.result?.value)));
      mainWs.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
    });
  };
  const close = () => {
    ws.close();
    mainWs?.close();
    try {
      execFileSync('taskkill', ['/pid', String(app.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      app.kill();
    }
  };
  return { ev, main, close };
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

test('ticked rows can be removed together', { skip: !electronPath && 'electron is not installed', timeout: 120000 }, async () => {
  const ui = await launch();
  try {
    // scheduled far ahead: the rows exist but nothing is fetched
    const later = Date.now() + 3600e3;
    await ui.ev(`window.confirm = () => true; window.dm.add(['http://127.0.0.1:9/a.bin', 'http://127.0.0.1:9/b.bin', 'http://127.0.0.1:9/c.bin'], { startAt: ${later} })`);
    await waitFor(async () => (await ui.ev(`document.querySelectorAll('#rows .row').length`)) === 3, 10000);
    assert.equal(await ui.ev(`document.getElementById('bulk').hidden`), true, 'no bar without a selection');

    await ui.ev(`document.querySelectorAll('#rows .r-check')[0].click()`);
    await ui.ev(`document.querySelectorAll('#rows .r-check')[1].click()`);
    assert.equal(await ui.ev(`document.getElementById('bulk').hidden`), false);
    assert.equal(await ui.ev(`document.getElementById('bulk-count').textContent`), '2 sélectionnés');

    // Escape drops the selection
    await ui.ev(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
    assert.equal(await ui.ev(`document.getElementById('bulk').hidden`), true);
    assert.equal(await ui.ev(`document.querySelectorAll('#rows .r-check:checked').length`), 0);

    // select all, untick one, remove the other two
    await ui.ev(`document.getElementById('bulk-all').click()`);
    assert.equal(await ui.ev(`document.getElementById('bulk-count').textContent`), '3 sélectionnés');
    await ui.ev(`document.querySelectorAll('#rows .r-check')[0].click()`);
    await ui.ev(`document.getElementById('bulk-remove').click()`);
    await waitFor(async () => (await ui.ev(`document.querySelectorAll('#rows .row').length`)) === 1, 10000);
    assert.equal(await ui.ev(`document.getElementById('bulk').hidden`), true, 'the bar goes away with the selection');
    assert.equal(await ui.ev(`appState.settings && window.dm.list().then((l) => l.length)`), 1);
  } finally {
    ui.close();
  }
});

test('the menu bar is there and its entries drive the window', { skip: !electronPath && 'electron is not installed', timeout: 120000 }, async () => {
  const ui = await launch();
  try {
    const menu = 'process.mainModule.require("electron").Menu.getApplicationMenu()';
    const click = (label) =>
      ui.main(`(() => { const find = (items) => { for (const i of items) { if (i.label === ${JSON.stringify(label)}) return i; const r = i.submenu && find(i.submenu.items); if (r) return r; } return null; }; const item = find(${menu}.items); item.click(); return true; })()`);
    assert.deepEqual(JSON.parse(await ui.main(`JSON.stringify(${menu}.items.map((i) => i.label))`)), ['Fichier', 'Téléchargements', 'Affichage', 'Aide']);
    assert.equal(await ui.main('process.mainModule.require("electron").BrowserWindow.getAllWindows()[0].isMenuBarVisible()'), true, 'the bar is shown');

    await click('Paramètres');
    await waitFor(async () => (await ui.ev(`document.getElementById('dlg-settings').open`)) === true, 10000);
    await ui.ev(`document.getElementById('dlg-settings').close()`);

    await click('Rechercher');
    await waitFor(async () => (await ui.ev('document.activeElement.id')) === 'search', 10000);

    await click('Sombre');
    assert.equal(await ui.ev('document.documentElement.dataset.theme'), 'dark');

    await click('1 Mo/s');
    await waitFor(async () => (await ui.ev('appState.settings.speedLimitKBps')) === 1024, 10000);
    const ticked = await ui.main(`JSON.stringify(${menu}.items[1].submenu.items.find((i) => i.label === 'Limite de vitesse').submenu.items.filter((i) => i.checked).map((i) => i.label))`);
    assert.deepEqual(JSON.parse(ticked), ['1 Mo/s'], 'the menu was rebuilt with the new limit ticked');

    await click('Éteindre l’ordinateur');
    await waitFor(async () => (await ui.ev('appState && window.dm.state().then((s) => s.settings.afterAll)')) === 'shutdown', 10000);
    await click('Ne rien faire'); // never leave a shutdown armed
    await waitFor(async () => (await ui.ev('window.dm.state().then((s) => s.settings.afterAll)')) === 'none', 10000);
  } finally {
    ui.close();
  }
});

test('network and after-download settings are saved, and site logins are kept apart from the form', { skip: !electronPath && 'electron is not installed', timeout: 120000 }, async () => {
  const ui = await launch();
  try {
    await ui.ev(`document.getElementById('btn-settings').click()`);
    await ui.ev(`document.getElementById('set-proxy').value = 'ftp://nope'; document.getElementById('form-settings').requestSubmit()`);
    await waitFor(async () => /http/.test(await ui.ev(`document.getElementById('set-error').textContent`)), 10000);
    assert.equal(await ui.ev(`document.getElementById('dlg-settings').open`), true, 'a bad proxy keeps the form open');

    await ui.ev(`
      document.getElementById('set-proxy').value = 'proxy.local:3128';
      document.getElementById('set-speedsched-on').checked = true;
      document.getElementById('set-speedsched-from').value = '09:00';
      document.getElementById('set-speedsched-to').value = '17:30';
      document.getElementById('set-speedsched-limit').value = '300';
      document.getElementById('set-rules').value = '*.pdf = Documents';
      document.getElementById('set-extract').checked = true;
      document.getElementById('set-adaptive').checked = false;
      document.getElementById('form-settings').requestSubmit()`);
    await waitFor(async () => (await ui.ev('appState.settings.proxy')) === 'http://proxy.local:3128', 10000);
    assert.deepEqual(await ui.ev('appState.settings.speedSchedule'), { enabled: true, from: '09:00', to: '17:30', limitKBps: 300 });
    assert.equal(await ui.ev('appState.settings.rules'), '*.pdf = Documents');
    assert.equal(await ui.ev('appState.settings.autoExtract'), true);
    assert.equal(await ui.ev('appState.settings.adaptive'), false);

    // logins: Enter adds one without submitting the settings form, and the secret never comes back
    await ui.ev(`document.getElementById('btn-settings').click()`);
    await ui.ev(`document.getElementById('login-host').value = 'https://Example.com/login'; document.getElementById('login-user').value = 'moi'; document.getElementById('login-secret').value = 'tres-secret'; document.getElementById('login-secret').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))`);
    await waitFor(async () => (await ui.ev(`document.querySelectorAll('#login-list li').length`)) === 1, 10000);
    assert.equal(await ui.ev(`document.getElementById('dlg-settings').open`), true, 'Enter did not submit the form');
    assert.equal(await ui.ev(`document.getElementById('login-secret').value`), '', 'the field is emptied');
    assert.match(await ui.ev(`document.getElementById('login-list').textContent`), /example\.com.*moi/);
    assert.ok(!(await ui.ev(`document.body.innerText`)).includes('tres-secret'));

    await ui.ev(`document.getElementById('login-add').click()`); // empty fields
    await waitFor(async () => (await ui.ev(`document.getElementById('login-error').textContent`)) !== '', 10000);
    assert.equal(await ui.ev(`document.querySelectorAll('#login-list li').length`), 1, 'an incomplete login adds nothing');

    await ui.ev(`document.querySelector('#login-list button').click()`);
    await waitFor(async () => (await ui.ev(`document.querySelectorAll('#login-list li').length`)) === 0, 10000);
  } finally {
    ui.close();
  }
});
