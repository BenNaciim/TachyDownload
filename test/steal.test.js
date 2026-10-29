'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { tmpDir, randomBody, waitFor } = require('./helpers');

const SIZE = 8 * 1024 * 1024;

/** Range server where the first eighth of the file is served slowly and everything after it fast. */
function slowHeadServer(body, slowBytesPerSec) {
  const server = http.createServer((req, res) => {
    const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    const start = m ? Number(m[1]) : 0;
    const end = m && m[2] !== '' ? Math.min(Number(m[2]), body.length - 1) : body.length - 1;
    res.writeHead(m ? 206 : 200, {
      'Content-Type': 'application/octet-stream',
      'Accept-Ranges': 'bytes',
      'Content-Length': end - start + 1,
      ...(m ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}),
    });
    const slice = body.subarray(start, end + 1);
    if (start >= body.length / 8) return res.end(slice);
    const step = Math.floor(slowBytesPerSec / 20);
    let off = 0;
    const timer = setInterval(() => {
      if (res.destroyed) return clearInterval(timer);
      res.write(slice.subarray(off, off + step));
      off += step;
      if (off >= slice.length) {
        clearInterval(timer);
        res.end();
      }
    }, 50);
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        url: `http://127.0.0.1:${server.address().port}/f.bin`,
        close: () => new Promise((r) => (server.closeAllConnections?.(), server.close(r))),
      }),
    ),
  );
}

async function setup() {
  const body = randomBody(SIZE);
  const srv = await slowHeadServer(body, 400 * 1024);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 4 }, tickMs: 50 });
  m.start();
  return { body, srv, dir, m, close: async () => (await m.shutdown(), srv.close()) };
}

const finished = (m, id) => ['completed', 'error'].includes(m.list().find((t) => t.id === id).status);

test('a freed connection takes half of the slowest segment, and the file stays identical', async () => {
  const { body, srv, dir, m, close } = await setup();
  try {
    const t0 = Date.now();
    const { id } = m.add(srv.url);
    await waitFor(() => finished(m, id), 30000);
    const snap = m.list().find((t) => t.id === id);
    assert.equal(snap.status, 'completed', snap.error ?? '');
    assert.ok(snap.segments.length > 4, `segments were re-split (got ${snap.segments.length})`);
    // Without re-splitting the slow first segment alone needs 2 MiB / 400 KiB/s = 5 s.
    assert.ok(Date.now() - t0 < 4200, `finished in ${Date.now() - t0} ms`);
    assert.ok(fs.readFileSync(path.join(dir, snap.filename)).equals(body));
    // segments tile the file exactly: no gap, no overlap
    const segs = [...snap.segments].sort((a, b) => a.start - b.start);
    assert.equal(segs[0].start, 0);
    for (let i = 1; i < segs.length; i++) assert.equal(segs[i].start, segs[i - 1].end + 1);
    assert.equal(segs.at(-1).end, SIZE - 1);
  } finally {
    await close();
  }
});

test('pausing after a re-split and resuming still gives an identical file', async () => {
  const { body, srv, dir, m, close } = await setup();
  try {
    const { id } = m.add(srv.url);
    await waitFor(() => m.list().find((t) => t.id === id).segments?.length > 4, 20000);
    await m.pause(id);
    const mid = m.list().find((t) => t.id === id);
    assert.equal(mid.status, 'paused');
    m.resume(id);
    await waitFor(() => finished(m, id), 30000);
    const snap = m.list().find((t) => t.id === id);
    assert.equal(snap.status, 'completed', snap.error ?? '');
    assert.ok(fs.readFileSync(path.join(dir, snap.filename)).equals(body));
  } finally {
    await close();
  }
});
