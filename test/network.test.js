'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const net = require('node:net');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { normalizeProxy } = require('../src/engine/net');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

test('a proxy address is cleaned up, and an unusable one is refused', () => {
  assert.equal(normalizeProxy(''), '');
  assert.equal(normalizeProxy('  '), '');
  assert.equal(normalizeProxy('proxy.local:3128'), 'http://proxy.local:3128');
  assert.equal(normalizeProxy('http://user:pw@10.0.0.1:8080/'), 'http://user:pw@10.0.0.1:8080');
  assert.throws(() => normalizeProxy('socks5://127.0.0.1:1080'), /http ou https/);
  assert.throws(() => normalizeProxy('http://'), /invalide/);
});

test('the limit in force is the tighter of the fixed limit and the time-of-day limit', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir(), speedLimitKBps: 0, speedSchedule: { enabled: true, from: '08:00', to: '18:00', limitKBps: 200 } } });
  const at = (h) => new Date(2026, 0, 5, h, 0);
  assert.equal(m.effectiveLimit(at(10)), 200, 'inside the window');
  assert.equal(m.effectiveLimit(at(20)), 0, 'outside the window: no limit at all');
  m.setSettings({ speedLimitKBps: 100 });
  assert.equal(m.effectiveLimit(at(10)), 100, 'the fixed limit is lower');
  assert.equal(m.effectiveLimit(at(20)), 100);
  m.setSettings({ speedLimitKBps: 500 });
  assert.equal(m.effectiveLimit(at(10)), 200, 'the window limit is lower');
  m.setSettings({ speedSchedule: { enabled: false, from: '08:00', to: '18:00', limitKBps: 200 } });
  assert.equal(m.effectiveLimit(at(10)), 500, 'schedule off');
  // a window that crosses midnight
  m.setSettings({ speedLimitKBps: 0, speedSchedule: { enabled: true, from: '22:00', to: '06:00', limitKBps: 50 } });
  assert.equal(m.effectiveLimit(at(23)), 50);
  assert.equal(m.effectiveLimit(at(3)), 50);
  assert.equal(m.effectiveLimit(at(12)), 0);
});

test('the shared bucket follows the limit in force', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir(), speedLimitKBps: 300 } });
  assert.equal(m.limiter.rate, 300 * 1024);
  m.setSettings({ speedLimitKBps: 0 });
  assert.equal(m.limiter.rate, 0);
});

/** A tiny HTTP proxy: plain requests are forwarded, CONNECT is tunnelled. Counts what went through. */
function startProxy() {
  const seen = { forwarded: 0, tunnelled: 0 };
  const server = http.createServer((req, res) => {
    seen.forwarded++;
    const target = new URL(req.url);
    const up = http.request({ host: target.hostname, port: target.port, path: target.pathname + target.search, method: req.method, headers: req.headers }, (r) => {
      res.writeHead(r.statusCode, r.headers);
      r.pipe(res);
    });
    req.pipe(up);
  });
  server.on('connect', (req, client, head) => {
    seen.tunnelled++;
    const [host, port] = req.url.split(':');
    const up = net.connect(Number(port), host, () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      up.write(head);
      up.pipe(client);
      client.pipe(up);
    });
    up.on('error', () => client.destroy());
    client.on('error', () => up.destroy());
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        url: `127.0.0.1:${server.address().port}`,
        seen,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

test('downloads go through the proxy when one is set', async () => {
  const body = randomBody(300 * 1024);
  const srv = await startServer(body);
  const proxy = await startProxy();
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, proxy: proxy.url }, tickMs: 30 });
  m.start();
  try {
    assert.equal(m.settings.proxy, `http://${proxy.url}`);
    const { id } = m.add(srv.url, { filename: 'p.bin' });
    await waitFor(() => m.list().find((t) => t.id === id).status === 'completed');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'p.bin')), body);
    assert.ok(proxy.seen.forwarded + proxy.seen.tunnelled > 0, 'the proxy carried the traffic');

    // without the proxy the traffic no longer goes through it
    m.setSettings({ proxy: '' });
    const before = proxy.seen.forwarded + proxy.seen.tunnelled;
    const second = m.add(srv.url, { filename: 'q.bin' });
    await waitFor(() => m.list().find((t) => t.id === second.id).status === 'completed');
    assert.equal(proxy.seen.forwarded + proxy.seen.tunnelled, before);
  } finally {
    await m.shutdown();
    await srv.close();
    await proxy.close();
  }
});

test('an invalid proxy is refused when saving settings and the old one stays', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir(), proxy: 'proxy.local:3128' } });
  assert.throws(() => m.setSettings({ proxy: 'ftp://x' }), /http ou https/);
  assert.equal(m.settings.proxy, 'http://proxy.local:3128');
});

/** Origin that answers 429 as soon as more than `max` requests are in flight at once. */
function startPickyServer(body, max) {
  let inflight = 0;
  let refused = 0;
  const server = http.createServer((req, res) => {
    const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    const start = Number(m?.[1] ?? 0);
    const end = m?.[2] ? Math.min(Number(m[2]), body.length - 1) : body.length - 1;
    const isProbe = start === 0 && end === 0;
    if (!isProbe && inflight >= max) {
      refused++;
      res.writeHead(429);
      return res.end();
    }
    inflight++;
    res.on('close', () => inflight--);
    res.writeHead(206, { 'Content-Type': 'application/octet-stream', 'Content-Range': `bytes ${start}-${end}/${body.length}`, 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes' });
    const slice = body.subarray(start, end + 1);
    const step = Math.max(1, Math.ceil(slice.length / 8));
    let off = 0;
    const timer = setInterval(() => {
      if (res.destroyed) return clearInterval(timer);
      res.write(slice.subarray(off, off + step));
      off += step;
      if (off >= slice.length) {
        clearInterval(timer);
        res.end();
      }
    }, 40);
    return undefined;
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        url: `http://127.0.0.1:${server.address().port}/big.bin`,
        refused: () => refused,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

test('a server that refuses too many connections ends up with fewer of them, and the file is still complete', { timeout: 60000 }, async () => {
  const body = randomBody(6 * 1024 * 1024);
  const srv = await startPickyServer(body, 2);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 6 }, tickMs: 30 });
  m.start();
  try {
    const { id } = m.add(srv.url, { filename: 'big.bin' });
    await waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id).status), 50000);
    const snap = m.list().find((t) => t.id === id);
    assert.equal(snap.status, 'completed', snap.error ?? '');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'big.bin')), body);
    assert.ok(srv.refused() > 0, 'the server did push back');
    assert.ok(m.tasks.get(id).allowed <= 2, `connections allowed fell to ${m.tasks.get(id).allowed}`);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('with the adaptive setting off the number of connections is left alone', { timeout: 60000 }, async () => {
  const body = randomBody(3 * 1024 * 1024);
  const srv = await startPickyServer(body, 2);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 3, adaptive: false }, tickMs: 30 });
  m.start();
  try {
    const { id } = m.add(srv.url, { filename: 'big.bin' });
    await waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id).status), 50000);
    assert.equal(m.tasks.get(id).allowed, 3);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
