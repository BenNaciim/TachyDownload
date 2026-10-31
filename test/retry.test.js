'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const net = require('node:net');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { tmpDir, randomBody, waitFor } = require('./helpers');

const freePort = () =>
  new Promise((resolve) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

/** Serves `body` on a fixed port, so the same address can be dead first and alive later. */
const serveOn = (port, body, status = 200) =>
  new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      if (status !== 200) {
        res.writeHead(status);
        return res.end();
      }
      res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': body.length, 'Accept-Ranges': 'bytes' });
      res.end(req.method === 'HEAD' ? undefined : body);
    });
    server.listen(port, '127.0.0.1', () => resolve({ close: () => new Promise((r) => (server.closeAllConnections?.(), server.close(r))) }));
  });

const manager = (dir, extra = {}) => {
  const m = new DownloadManager({ settings: { dir }, tickMs: 30, retryDelays: [0.1, 0.1, 0.1], ...extra });
  m.start();
  return m;
};

test('a download that lost its server tries again by itself and finishes when the server is back', async () => {
  const body = randomBody(256 * 1024);
  const port = await freePort();
  const dir = tmpDir();
  const m = manager(dir);
  let srv;
  try {
    m.add(`http://127.0.0.1:${port}/a.bin`, { filename: 'a.bin' });
    await waitFor(() => m.list()[0].status === 'error' && m.list()[0].retryAt, 10000);
    assert.equal(m.list()[0].retryCount, 1);
    srv = await serveOn(port, body);
    await waitFor(() => m.list()[0].status === 'completed', 20000);
    assert.deepEqual(fs.readFileSync(path.join(dir, 'a.bin')), body);
    assert.equal(m.list()[0].retryAt, null);
  } finally {
    await m.shutdown();
    await srv?.close();
  }
});

test('it stops trying once every delay has been used, and a manual retry starts the count over', async () => {
  const port = await freePort();
  const m = manager(tmpDir());
  try {
    const { id } = m.add(`http://127.0.0.1:${port}/b.bin`, { filename: 'b.bin' });
    await waitFor(() => m.tasks.get(id).state.retryCount === 3 && m.list()[0].status === 'error' && !m.list()[0].retryAt, 15000);
    assert.equal(m.list()[0].retryAt, null, 'no further attempt is planned');
    m.resume(id);
    assert.equal(m.tasks.get(id).state.retryCount, 0);
  } finally {
    await m.shutdown();
  }
});

test('a refusal from the server is not retried', async () => {
  const port = await freePort();
  const srv = await serveOn(port, Buffer.alloc(0), 404);
  const m = manager(tmpDir());
  try {
    const { id } = m.add(`http://127.0.0.1:${port}/c.bin`, { filename: 'c.bin' });
    await waitFor(() => m.list()[0].status === 'error', 10000);
    assert.equal(m.tasks.get(id).state.retryable, false);
    assert.ok(!m.list()[0].retryAt);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('with the setting off, nothing is retried', async () => {
  const port = await freePort();
  const m = manager(tmpDir(), { settings: { dir: tmpDir(), autoRetry: false } });
  try {
    m.add(`http://127.0.0.1:${port}/d.bin`, { filename: 'd.bin' });
    await waitFor(() => m.list()[0].status === 'error', 10000);
    await new Promise((r) => setTimeout(r, 400));
    assert.ok(!m.list()[0].retryAt);
    assert.equal(m.list()[0].status, 'error');
  } finally {
    await m.shutdown();
  }
});
