'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { tmpDir, randomBody, waitFor } = require('./helpers');

const BASIC = 'Basic ' + Buffer.from('moi:secret').toString('base64');

/** An origin that wants an Authorization header (and a cookie when `cookie` is set). */
function startPrivateServer(body, { cookie = null } = {}) {
  const seen = [];
  const server = http.createServer((req, res) => {
    seen.push({ auth: req.headers.authorization ?? null, cookie: req.headers.cookie ?? null });
    const ok = (cookie ? req.headers.cookie === cookie : req.headers.authorization === BASIC);
    if (!ok) {
      res.writeHead(401, { 'WWW-Authenticate': 'Basic' });
      return res.end();
    }
    const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    const start = Number(m?.[1] ?? 0);
    const end = m?.[2] ? Math.min(Number(m[2]), body.length - 1) : body.length - 1;
    res.writeHead(206, { 'Content-Range': `bytes ${start}-${end}/${body.length}`, 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes' });
    return res.end(body.subarray(start, end + 1));
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        url: `http://127.0.0.1:${server.address().port}/secret.bin`,
        seen,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

test('a site login is sent with every request to that site', async () => {
  const body = randomBody(200 * 1024);
  const srv = await startPrivateServer(body);
  const dir = tmpDir();
  const asked = [];
  const m = new DownloadManager({
    settings: { dir },
    tickMs: 30,
    siteAuth: (url) => {
      asked.push(url);
      return new URL(url).hostname === '127.0.0.1' ? { Authorization: BASIC } : null;
    },
  });
  m.start();
  try {
    const { id } = m.add(srv.url, { filename: 's.bin' });
    await waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id).status));
    const snap = m.list().find((t) => t.id === id);
    assert.equal(snap.status, 'completed', snap.error ?? '');
    assert.deepEqual(fs.readFileSync(path.join(dir, 's.bin')), body);
    assert.ok(srv.seen.every((r) => r.auth === BASIC), 'the probe and every segment carried the login');
    assert.ok(asked.every((u) => u.startsWith('http://127.0.0.1')), 'the lookup always gets the address being requested');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('without a login the download stops with a message that says what to do', async () => {
  const srv = await startPrivateServer(randomBody(10 * 1024));
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 30 });
  m.start();
  try {
    const { id } = m.add(srv.url, { filename: 's.bin' });
    await waitFor(() => m.list().find((t) => t.id === id).status === 'error');
    assert.match(m.list().find((t) => t.id === id).error, /Sites connectés/);
    assert.ok(srv.seen.every((r) => r.auth === null), 'nothing was sent');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('a site cookie works like a login, and the one from the browser wins when both exist', async () => {
  const body = randomBody(50 * 1024);
  const srv = await startPrivateServer(body, { cookie: 'session=browser' });
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 30, siteAuth: () => ({ Cookie: 'session=saved' }) });
  m.start();
  try {
    const first = m.add(srv.url, { filename: 'a.bin' });
    await waitFor(() => m.list().find((t) => t.id === first.id).status === 'error');
    const second = m.add(srv.url, { filename: 'b.bin', cookie: 'session=browser' });
    await waitFor(() => m.list().find((t) => t.id === second.id).status === 'completed');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'b.bin')), body);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
