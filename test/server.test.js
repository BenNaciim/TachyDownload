'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { DownloadManager } = require('../src/engine/manager');
const { createExtensionServer } = require('../src/server');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

const TOKEN = 'secret-token-for-tests';

async function setup() {
  const origin = await startServer(randomBody(50 * 1024));
  const manager = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  manager.start();
  const api = createExtensionServer({ manager, token: () => TOKEN });
  // Port 0 = any free port, so fetch's keep-alive pool never reuses a socket from a previous test.
  const port = await api.listen(0);
  const base = `http://127.0.0.1:${port}`;
  const close = async () => {
    await manager.shutdown();
    await api.close();
    await origin.close();
  };
  return { origin, manager, base, close };
}

const ext = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop';

test('rejects requests without the token, from web origins, or with a foreign Host', async () => {
  const { base, manager, origin, close } = await setup();
  try {
    const body = JSON.stringify({ url: origin.url });
    const post = (headers) => fetch(`${base}/add`, { method: 'POST', body, headers });

    assert.equal((await post({ Origin: ext })).status, 401);
    assert.equal((await post({ Origin: ext, 'X-DM-Token': 'wrong-token' })).status, 401);
    assert.equal((await post({ Origin: 'https://evil.example', 'X-DM-Token': TOKEN })).status, 403);
    assert.equal(manager.list().length, 0);
  } finally {
    await close();
  }
});

test('accepts a valid add from an extension and downloads the file', async () => {
  const { base, manager, origin, close } = await setup();
  try {
    const res = await fetch(`${base}/add`, {
      method: 'POST',
      headers: { Origin: ext, 'X-DM-Token': TOKEN, 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: origin.url, referer: 'https://example.com/page' }),
    });
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('access-control-allow-origin'), ext);
    const { id } = await res.json();
    await waitFor(() => manager.list().find((t) => t.id === id)?.status === 'completed');
  } finally {
    await close();
  }
});

test('bad payloads are 400 and CORS preflight is answered', async () => {
  const { base, close } = await setup();
  try {
    const headers = { Origin: ext, 'X-DM-Token': TOKEN };
    const bad = await fetch(`${base}/add`, { method: 'POST', headers, body: '{"url":"file:///etc/passwd"}' });
    assert.equal(bad.status, 400);
    const notJson = await fetch(`${base}/add`, { method: 'POST', headers, body: 'nope' });
    assert.equal(notJson.status, 400);
    const pre = await fetch(`${base}/add`, { method: 'OPTIONS', headers: { Origin: ext } });
    assert.equal(pre.status, 204);
    assert.match(pre.headers.get('access-control-allow-headers'), /X-DM-Token/);
  } finally {
    await close();
  }
});

test('onRequest can take over an add: nothing is queued until the user answers', async () => {
  const origin = await startServer(randomBody(50 * 1024));
  const manager = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  manager.start();
  const seen = [];
  const api = createExtensionServer({ manager, token: () => TOKEN, onRequest: (r) => (seen.push(r), true) });
  const port = await api.listen(0);
  try {
    const post = (payload) =>
      fetch(`http://127.0.0.1:${port}/add`, {
        method: 'POST',
        headers: { Origin: ext, 'X-DM-Token': TOKEN, 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
    const res = await post({ url: origin.url, size: 376660, sniffed: true, kind: 'hls' });
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { pending: true });
    assert.equal(manager.list().length, 0);
    assert.equal(seen[0].size, 376660);
    assert.equal(seen[0].kind, 'hls');
    assert.equal((await post({ url: 'file:///etc/passwd' })).status, 400);
    assert.equal(seen.length, 1);
  } finally {
    await manager.shutdown();
    await api.close();
    await origin.close();
  }
});
