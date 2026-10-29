'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { DownloadManager, inWindow } = require('../src/engine/manager');
const { splitSegments } = require('../src/engine/download');
const {
  sanitizeFilename,
  filenameFromContentDisposition,
} = require('../src/engine/util');
const { startServer, tmpDir, randomBody, sha, waitFor } = require('./helpers');

const MB = 1024 * 1024;

function makeManager(dir, settings = {}) {
  const m = new DownloadManager({ settings: { dir, ...settings }, tickMs: 50 });
  m.start();
  return m;
}

const finished = (m, id) =>
  waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status));

test('splitSegments covers every byte exactly once', () => {
  for (const [size, n] of [[10 * MB + 7, 8], [3 * MB, 8], [1, 4], [5 * MB, 1]]) {
    const segs = splitSegments(size, n);
    assert.equal(segs[0].start, 0);
    assert.equal(segs.at(-1).end, size - 1);
    for (let i = 1; i < segs.length; i++) assert.equal(segs[i].start, segs[i - 1].end + 1);
  }
  assert.equal(splitSegments(3 * MB, 8).length, 3, 'no segment smaller than 1 MiB');
});

test('filename helpers', () => {
  assert.equal(sanitizeFilename('../../etc/passwd'), '.._.._etc_passwd'.replace(/^\.+/, ''));
  assert.equal(sanitizeFilename('CON.txt'), '_CON.txt');
  assert.equal(sanitizeFilename('a:b*c?.zip'), 'a_b_c_.zip');
  assert.equal(filenameFromContentDisposition('attachment; filename="r\u00e9sum\u00e9.pdf"'), 'r\u00e9sum\u00e9.pdf');
  assert.equal(
    filenameFromContentDisposition("attachment; filename*=UTF-8''%E2%82%AC%20rates.txt"),
    '\u20ac rates.txt',
  );
});

test('inWindow handles windows that cross midnight', () => {
  const s = { enabled: true, from: '22:00', to: '06:00' };
  assert.equal(inWindow(s, new Date(2026, 0, 1, 23, 30)), true);
  assert.equal(inWindow(s, new Date(2026, 0, 1, 5, 59)), true);
  assert.equal(inWindow(s, new Date(2026, 0, 1, 12, 0)), false);
  assert.equal(inWindow({ ...s, enabled: false }, new Date(2026, 0, 1, 12, 0)), true);
});

test('downloads with parallel segments and the bytes match', async () => {
  const body = randomBody(6 * MB + 123);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = makeManager(dir, { connections: 4 });
  try {
    const { id } = m.add(srv.url);
    await finished(m, id);
    const t = m.list()[0];
    assert.equal(t.status, 'completed', t.error);
    assert.equal(t.filename, 'sample.bin');
    assert.equal(sha(fs.readFileSync(path.join(dir, 'sample.bin'))), sha(body));
    const ranged = srv.requests.filter((r) => r.range && r.range !== 'bytes=0-0');
    assert.equal(ranged.length, 4, 'one ranged request per segment');
    assert.equal(fs.existsSync(path.join(dir, 'sample.bin.part')), false);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('Content-Disposition names the file and duplicates get a suffix', async () => {
  const body = randomBody(200 * 1024);
  const srv = await startServer(body, { disposition: 'attachment; filename="report.bin"' });
  const dir = tmpDir();
  const m = makeManager(dir);
  try {
    const a = m.add(srv.url);
    await finished(m, a.id);
    const b = m.add(srv.url);
    await finished(m, b.id);
    const names = m.list().map((t) => t.filename).sort();
    assert.deepEqual(names, ['report (1).bin', 'report.bin']);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('pause then resume continues from the saved offsets', async () => {
  const body = randomBody(4 * MB);
  const srv = await startServer(body, { bytesPerSec: 2 * MB });
  const dir = tmpDir();
  const m = makeManager(dir, { connections: 2 });
  try {
    const { id } = m.add(srv.url);
    await waitFor(() => (m.list()[0].downloaded ?? 0) > 300 * 1024);
    await m.pause(id);
    const paused = m.list()[0];
    assert.equal(paused.status, 'paused');
    assert.ok(paused.downloaded > 0 && paused.downloaded < body.length);
    const before = paused.downloaded;
    const requestsBefore = srv.requests.length;
    m.resume(id);
    await finished(m, id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.equal(sha(fs.readFileSync(path.join(dir, 'sample.bin'))), sha(body));
    // Only the requests issued after the resume count: they must ask for the
    // missing bytes, not the whole file again.
    const asked = srv.requests.slice(requestsBefore).reduce((n, r) => {
      const [a, b] = r.range.slice(6).split('-').map(Number);
      return n + (b - a + 1);
    }, 0);
    assert.ok(asked <= body.length - before, `re-requested ${asked} bytes, ${body.length - before} were missing`);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('resumes after the manager restarts (state persisted on disk)', async () => {
  const body = randomBody(4 * MB);
  const srv = await startServer(body, { bytesPerSec: 2 * MB });
  const dir = tmpDir();
  const storePath = path.join(tmpDir(), 'downloads.json');
  const m1 = new DownloadManager({ storePath, settings: { dir, connections: 2 }, tickMs: 50 });
  m1.start();
  const { id } = m1.add(srv.url);
  await waitFor(() => m1.list()[0].downloaded > 300 * 1024);
  await m1.shutdown();

  const m2 = new DownloadManager({ storePath, tickMs: 50 });
  m2.load();
  m2.start();
  try {
    assert.equal(m2.list()[0].status, 'paused');
    assert.ok(m2.list()[0].downloaded > 0);
    m2.resume(id);
    await finished(m2, id);
    assert.equal(m2.list()[0].status, 'completed', m2.list()[0].error);
    assert.equal(sha(fs.readFileSync(path.join(dir, 'sample.bin'))), sha(body));
  } finally {
    await m2.shutdown();
    await srv.close();
  }
});

test('falls back to a single stream when the server ignores Range', async () => {
  const body = randomBody(2 * MB + 5);
  const srv = await startServer(body, { ranges: false });
  const dir = tmpDir();
  const m = makeManager(dir, { connections: 8 });
  try {
    const { id } = m.add(srv.url);
    await finished(m, id);
    const t = m.list()[0];
    assert.equal(t.status, 'completed', t.error);
    assert.equal(t.rangeOk, false);
    assert.equal(t.segments.length, 1);
    assert.equal(sha(fs.readFileSync(path.join(dir, 'sample.bin'))), sha(body));
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('speed limit throttles the transfer', async () => {
  const body = randomBody(600 * 1024);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = makeManager(dir, { connections: 2, speedLimitKBps: 200 });
  try {
    const t0 = Date.now();
    const { id } = m.add(srv.url);
    await finished(m, id);
    const secs = (Date.now() - t0) / 1000;
    assert.equal(m.list()[0].status, 'completed');
    // 600 KiB at 200 KiB/s with a one-second burst: about two seconds.
    assert.ok(secs > 1.5, `finished too fast for the limit: ${secs}s`);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('queue respects maxConcurrent', async () => {
  const body = randomBody(1 * MB);
  const srv = await startServer(body, { bytesPerSec: 1 * MB });
  const dir = tmpDir();
  const m = makeManager(dir, { maxConcurrent: 2, connections: 1 });
  try {
    const ids = [1, 2, 3, 4].map(() => m.add(srv.url).id);
    let peak = 0;
    await waitFor(() => {
      const list = m.list();
      peak = Math.max(peak, list.filter((t) => t.status === 'downloading').length);
      return list.every((t) => t.status === 'completed');
    }, 30000);
    assert.equal(peak, 2);
    assert.equal(new Set(m.list().map((t) => t.filename)).size, ids.length);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('scheduled download waits for its start time', async () => {
  const body = randomBody(100 * 1024);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = makeManager(dir);
  try {
    const { id } = m.add(srv.url, { startAt: Date.now() + 700 });
    assert.equal(m.list()[0].status, 'scheduled');
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(m.list()[0].status, 'scheduled');
    await finished(m, id);
    assert.equal(m.list()[0].status, 'completed');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('rejects non-http links and reports HTTP errors', async () => {
  const dir = tmpDir();
  const m = makeManager(dir);
  try {
    assert.throws(() => m.add('file:///C:/Windows/system.ini'), /http/);
    assert.throws(() => m.add('not a url'), /invalide/);
    const body = Buffer.alloc(10);
    const srv = await startServer(body);
    const { id } = m.add(srv.url.replace('/files/sample.bin', '/x'));
    // The test server answers every path, so use a closed port instead.
    await srv.close();
    // A port that was free a moment ago and now has nobody listening: a real refusal.
    const probe = net.createServer();
    await new Promise((r) => probe.listen(0, '127.0.0.1', r));
    const closedPort = probe.address().port;
    await new Promise((r) => probe.close(r));
    const dead = m.add(`http://127.0.0.1:${closedPort}/nothing`);
    await finished(m, dead.id);
    const failed = m.list().find((t) => t.id === dead.id);
    assert.equal(failed.status, 'error');
    assert.equal(failed.error, 'Le serveur refuse la connexion.');
    void id;
  } finally {
    await m.shutdown();
  }
});

test('add with paused:true waits in the list instead of starting ("later")', async () => {
  const srv = await startServer(randomBody(50 * 1024));
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  try {
    const { id } = m.add(srv.url, { paused: true });
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(m.list().find((t) => t.id === id).status, 'paused');
    m.resume(id);
    await waitFor(() => m.list().find((t) => t.id === id).status === 'completed');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
