'use strict';
// Minimal Chrome DevTools Protocol client for a page of Electron or WebView2 started with --remote-debugging-port.
// Needs Node >= 22 (global WebSocket). Used by the bench scripts and by the interface tests of the Tauri shell.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Page targets listed on the debugging port, or [] while the port is not open yet. */
async function pages(port) {
  try {
    const res = await fetch(`http://127.0.0.1:${port}/json`);
    return (await res.json()).filter((t) => t.type === 'page');
  } catch {
    return [];
  }
}

/** Connects to the first page whose URL contains `urlPart`; waits up to `waitMs` for it to exist. */
async function connect(port, { urlPart = '', waitMs = 30000 } = {}) {
  const t0 = Date.now();
  let target;
  while (!(target = (await pages(port)).find((t) => t.url.includes(urlPart)))) {
    if (Date.now() - t0 > waitMs) throw new Error(`no page matching "${urlPart}" on port ${port}`);
    await sleep(25);
  }
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error('cannot open the DevTools socket'));
  });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    const p = msg.id && pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    if (msg.error) p.reject(new Error(msg.error.message));
    else p.resolve(msg.result);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const n = ++id;
      pending.set(n, { resolve, reject });
      ws.send(JSON.stringify({ id: n, method, params }));
    });
  /** Evaluates `expression` in the page (promises are awaited) and returns its value. */
  const evaluate = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  const screenshot = async () => Buffer.from((await send('Page.captureScreenshot', { format: 'png' })).data, 'base64');
  return { url: target.url, send, evaluate, screenshot, close: () => ws.close() };
}

module.exports = { connect, pages, sleep };
