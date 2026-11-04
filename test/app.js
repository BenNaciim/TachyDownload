'use strict';

// Starts the built Tachy Download (Tauri) with a throwaway data folder and drives its window through the web view's
// debugging port. Shared by the end-to-end tests: ui.test.js (the page) and shell.test.js (the windows, the tray's
// close behaviour, the extension's server).
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { tmpDir, waitFor } = require('./helpers');

const ROOT = path.join(__dirname, '..');
// The built app (cargo build -p tachy-download); without it the end-to-end tests are skipped.
const BIN = process.env.TACHY_BIN || path.join(ROOT, 'target', 'debug', 'tachy-download.exe');
const skip = !fs.existsSync(BIN) && 'the app is not built (cargo build -p tachy-download)';
const WIN = path.join(__dirname, 'win.ps1');
const win = (...args) => execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WIN, ...args], { encoding: 'utf8' }).trim();

const freePort = () =>
  new Promise((resolve) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

/** Evaluates code in one page of the web view (CDP `Runtime.evaluate`) and returns its value. */
async function connect(target) {
  const ws = new WebSocket(target.webSocketDebuggerUrl);
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
  return { ev, close: () => ws.close() };
}

/**
 * Launches the app. `settings` are merged over the test defaults in `downloads.json`; `args` go to the command line.
 * Returns the main page (`ev`), the native menu (`menu`), the other pages (`pageOf`), the process and its data.
 */
async function launch({ settings = {}, args = [] } = {}) {
  const userData = tmpDir();
  const dir = tmpDir();
  fs.writeFileSync(path.join(userData, 'downloads.json'), JSON.stringify({ settings: { dir, connections: 5, confirmDownloads: false, ...settings }, tasks: [] }));
  const port = await freePort();
  const app = spawn(BIN, ['--data-dir', userData, ...args], { stdio: 'ignore', env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` } });
  const exited = new Promise((resolve) => app.once('exit', resolve));
  const targets = async () => {
    try {
      return (await (await fetch(`http://127.0.0.1:${port}/json`, { signal: AbortSignal.timeout(2000) })).json()).filter((t) => t.type === 'page');
    } catch {
      return [];
    }
  };
  let page = null;
  await waitFor(async () => (page = (await targets()).find((t) => /index\.html/.test(t.url))), 30000);
  const main = await connect(page);
  const { ev } = main;
  await waitFor(async () => (await ev('typeof appState !== "undefined" && appState !== null')) === true, 30000);
  const open = [main];
  const close = () => {
    for (const c of open) c.close();
    try {
      execFileSync('taskkill', ['/pid', String(app.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      app.kill();
    }
  };
  // A page other than the main one (a confirmation window), by part of its address.
  const pageOf = async (re) => {
    let t = null;
    await waitFor(async () => (t = (await targets()).find((x) => re.test(x.url))), 15000);
    const c = await connect(t);
    open.push(c);
    return c;
  };
  // The native menu bar: what it shows, and choosing an entry.
  // (PowerShell writes a one-entry list as the entry itself and an empty one as {}, so lists are put back here.)
  const list = (x) => (Array.isArray(x) ? x : x && Object.keys(x).length ? [x] : []).map((i) => ({ ...i, items: list(i.items) }));
  const menu = { dump: () => list(JSON.parse(win('dump'))), click: (label) => win('click', label) };
  return { ev, menu, win, pageOf, targets, close, userData, dir, app, exited, port };
}

module.exports = { BIN, skip, win, launch, freePort };
