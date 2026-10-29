'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { createExtensionServer } = require('../src/server');
const { tmpDir, randomBody, waitFor } = require('./helpers');

/** Origin whose links only work with the current ?sig=; `expire()` invalidates it. */
function signedServer(body) {
  let valid = 'one';
  const hits = [];
  const server = http.createServer((req, res) => {
    const u = new URL(req.url, 'http://x');
    hits.push({ sig: u.searchParams.get('sig'), range: req.headers.range ?? null });
    if (u.searchParams.get('sig') !== valid) {
      res.writeHead(403);
      return res.end();
    }
    const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    const start = m ? Number(m[1]) : 0;
    const end = m && m[2] !== '' ? Math.min(Number(m[2]), body.length - 1) : body.length - 1;
    const headers = { 'Content-Type': 'application/octet-stream', 'Accept-Ranges': 'bytes', 'Content-Length': end - start + 1 };
    if (m) headers['Content-Range'] = `bytes ${start}-${end}/${body.length}`;
    res.writeHead(m ? 206 : 200, headers);
    const slice = body.subarray(start, end + 1);
    // Slow enough to be cut in the middle.
    let off = 0;
    const timer = setInterval(() => {
      if (res.destroyed) return clearInterval(timer);
      res.write(slice.subarray(off, off + 64 * 1024));
      off += 64 * 1024;
      if (off >= slice.length) { clearInterval(timer); res.end(); }
    }, 25);
    res.on('close', () => clearInterval(timer));
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        base: `http://127.0.0.1:${server.address().port}/f.bin`,
        hits,
        expire: (next) => { valid = next; },
        close: () => { server.closeAllConnections?.(); return new Promise((r) => server.close(r)); },
      }),
    ),
  );
}

test('expired link: download stops with needsRefresh, a fresh link resumes and the file is intact', async () => {
  const body = randomBody(6 * 1024 * 1024);
  const srv = await signedServer(body);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 2 }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(`${srv.base}?sig=one`, { filename: 'big.bin', referer: 'http://page.example/watch' });
    await waitFor(() => m.list()[0].downloaded > 1024 * 1024);
    srv.expire('two'); // the link dies mid-transfer; next request is refused
    await m.pause(t.id);
    m.resume(t.id);
    await waitFor(() => m.list()[0].status === 'error', 30000);
    const failed = m.list()[0];
    assert.equal(failed.needsRefresh, true);
    assert.match(failed.error, /403/);
    assert.equal(failed.pageUrl, 'http://page.example/watch');
    assert.equal(m.refreshRequests()[0].id, t.id);
    const kept = failed.downloaded;
    assert.ok(kept > 0, 'partial data is kept');

    m.refresh(t.id, { url: `${srv.base}?sig=two` });
    assert.equal(m.refreshRequests().length, 0);
    await waitFor(() => ['completed', 'error'].includes(m.list()[0].status), 30000);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.deepEqual(fs.readFileSync(path.join(dir, 'big.bin')), body);
    // It resumed: the first ranged request on the new link does not start at byte 0.
    const fresh = srv.hits.filter((h) => h.sig === 'two' && h.range);
    assert.ok(fresh.some((h) => !h.range.startsWith('bytes=0-')), 'resumed from saved offsets');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('API: /refresh-requests lists them, /refresh swaps the link (token required)', async () => {
  const body = randomBody(300 * 1024);
  const srv = await signedServer(body);
  srv.expire('two');
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 50 });
  m.start();
  const api = createExtensionServer({ manager: m, token: () => 'tok'.repeat(16) });
  const port = await api.listen(0);
  const H = { 'X-DM-Token': 'tok'.repeat(16), 'Content-Type': 'application/json' };
  try {
    const t = m.add(`${srv.base}?sig=one`, { filename: 'a.bin', referer: 'http://page.example/x', sniffed: true });
    await waitFor(() => m.list()[0].status === 'error');
    const noToken = await fetch(`http://127.0.0.1:${port}/refresh-requests`);
    assert.equal(noToken.status, 401);
    const list = await (await fetch(`http://127.0.0.1:${port}/refresh-requests`, { headers: H })).json();
    assert.deepEqual(list.requests.map((r) => [r.id, r.auto, r.pageUrl]), [[t.id, true, 'http://page.example/x']]);

    const bad = await fetch(`http://127.0.0.1:${port}/refresh`, { method: 'POST', headers: H, body: JSON.stringify({ id: 'nope', url: 'http://x/' }) });
    assert.equal(bad.status, 400);
    const ok = await fetch(`http://127.0.0.1:${port}/refresh`, { method: 'POST', headers: H, body: JSON.stringify({ id: t.id, url: `${srv.base}?sig=two`, cookie: 'a=b' }) });
    assert.equal(ok.status, 200);
    await waitFor(() => m.list()[0].status === 'completed');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'a.bin')), body);
    assert.equal(m.list()[0].headers.cookie, 'a=b');
  } finally {
    await api.close();
    await m.shutdown();
    await srv.close();
  }
});
