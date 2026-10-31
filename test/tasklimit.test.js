'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

const SIZE = 700 * 1024;

/** Time one download takes with the given own limit (KB/s, 0 for none) and shared limit. */
async function timeIt(own, shared = 0) {
  const body = randomBody(SIZE);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, speedLimitKBps: shared, connections: 2 }, tickMs: 30 });
  m.start();
  try {
    const { id } = m.add(srv.url, { filename: 'f.bin', paused: true });
    m.setTaskLimit(id, own);
    const t0 = Date.now();
    m.resume(id);
    await waitFor(() => m.list()[0].status === 'completed', 30000);
    assert.deepEqual(fs.readFileSync(path.join(dir, 'f.bin')), body);
    return { ms: Date.now() - t0, snap: m.list()[0] };
  } finally {
    await m.shutdown();
    await srv.close();
  }
}

test('a download with its own limit is slowed down, an unlimited one is not', async () => {
  const free = await timeIt(0);
  const slow = await timeIt(200); // 200 KB/s: the first second is free, the other ~500 KB take ~2.5 s
  assert.ok(slow.ms > 1800, `limited run took ${slow.ms} ms`);
  assert.ok(slow.ms > free.ms * 3, `limited ${slow.ms} ms against ${free.ms} ms`);
  assert.equal(slow.snap.speedLimitKBps, 200, 'the limit is part of the task and gets saved');
});

test('the shared limit still applies when it is lower than the own limit', async () => {
  const r = await timeIt(5000, 200);
  assert.ok(r.ms > 1800, `took ${r.ms} ms`);
});

test('setTaskLimit cleans its input and 0 removes the limit', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir() } });
  const { id } = m.add('http://127.0.0.1:9/x.bin', { filename: 'x.bin', paused: true });
  assert.equal(m.setTaskLimit(id, 123.7).speedLimitKBps, 124);
  assert.equal(m.setTaskLimit(id, -5).speedLimitKBps, 0);
  assert.equal(m.setTaskLimit(id, 'abc').speedLimitKBps, 0);
  assert.throws(() => m.setTaskLimit('nope', 10), /introuvable/);
});
