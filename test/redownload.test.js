'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { tmpDir, randomBody, sha, waitFor } = require('./helpers');

const finished = (m, id) => waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status));

/** A file server whose content (or answer) can change between two downloads of the same address. */
function changingServer(body) {
  const state = { body, status: 200 };
  const server = http.createServer((req, res) => {
    if (state.status !== 200) {
      res.writeHead(state.status);
      return res.end();
    }
    const data = state.body;
    let start = 0;
    let end = data.length - 1;
    let status = 200;
    const headers = { 'Content-Type': 'application/octet-stream', 'Accept-Ranges': 'bytes' };
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    if (range) {
      start = Number(range[1]);
      if (range[2] !== '') end = Math.min(Number(range[2]), data.length - 1);
      status = 206;
      headers['Content-Range'] = `bytes ${start}-${end}/${data.length}`;
    }
    headers['Content-Length'] = end - start + 1;
    res.writeHead(status, headers);
    return res.end(data.subarray(start, end + 1));
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        state,
        url: `http://127.0.0.1:${server.address().port}/files/data.bin`,
        base: `http://127.0.0.1:${server.address().port}`,
        close: () => (server.closeAllConnections?.(), new Promise((r) => server.close(r))),
      }),
    ),
  );
}

async function withFile(fn, { size = 3_500_000 } = {}) {
  const v1 = randomBody(size);
  const srv = await changingServer(v1);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 4 }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(srv.url, { filename: 'data.bin' });
    await finished(m, t.id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    await fn({ m, srv, dir, id: t.id, v1 });
  } finally {
    await m.shutdown();
    await srv.close();
  }
}

test('redownload: the finished file is replaced under the same name, in the same row', async () => {
  await withFile(async ({ m, srv, dir, id, v1 }) => {
    const v2 = randomBody(4_200_000);
    srv.state.body = v2;
    const before = m.list()[0].completedAt;
    m.redownload(id);
    assert.equal(m.list().length, 1, 'no new row');
    assert.notEqual(m.list()[0].status, 'completed', 'it starts again at once');
    await finished(m, id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.equal(done.id, id);
    assert.equal(done.filename, 'data.bin');
    assert.equal(sha(fs.readFileSync(path.join(dir, 'data.bin'))), sha(v2));
    assert.notEqual(sha(v1), sha(v2));
    assert.deepEqual(fs.readdirSync(dir), ['data.bin'], 'no copy, no leftover');
    assert.equal(done.size, v2.length);
    assert.ok(done.completedAt >= before);
  });
});

test('redownload: the old file stays while the new one fails, and a retry replaces it', async () => {
  await withFile(async ({ m, srv, dir, id, v1 }) => {
    srv.state.status = 404;
    m.redownload(id);
    await finished(m, id);
    assert.equal(m.list()[0].status, 'error');
    assert.equal(sha(fs.readFileSync(path.join(dir, 'data.bin'))), sha(v1), 'the old file is untouched');

    const v2 = randomBody(3_800_000);
    Object.assign(srv.state, { status: 200, body: v2 });
    m.resume(id);
    await finished(m, id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    assert.equal(sha(fs.readFileSync(path.join(dir, 'data.bin'))), sha(v2));
    assert.deepEqual(fs.readdirSync(dir), ['data.bin']);
  });
});

test('redownload: removing the row while it is failing leaves the old file alone', async () => {
  await withFile(async ({ m, srv, dir, id, v1 }) => {
    srv.state.status = 404;
    m.redownload(id);
    await finished(m, id);
    await m.remove(id, { deleteFile: true }); // what the "Retirer" button asks for on an unfinished download
    assert.equal(sha(fs.readFileSync(path.join(dir, 'data.bin'))), sha(v1));
  });
});

test('redownload: a file that is gone is simply downloaded again under its name', async () => {
  await withFile(async ({ m, srv, dir, id }) => {
    fs.rmSync(path.join(dir, 'data.bin'));
    const v2 = randomBody(3_100_000);
    srv.state.body = v2;
    m.redownload(id);
    await finished(m, id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    assert.deepEqual(fs.readdirSync(dir), ['data.bin']);
    assert.equal(sha(fs.readFileSync(path.join(dir, 'data.bin'))), sha(v2));
  });
});

test('redownload: refused at once when the file cannot be replaced (read-only or open elsewhere)', { skip: process.getuid?.() === 0 }, async () => {
  await withFile(async ({ m, dir, id }) => {
    const file = path.join(dir, 'data.bin');
    fs.chmodSync(file, 0o444);
    try {
      assert.throws(() => m.redownload(id), /autre programme|lecture seule/);
      assert.equal(m.list()[0].status, 'completed', 'the finished download is left as it was');
    } finally {
      fs.chmodSync(file, 0o666);
    }
  });
});

test('redownload: only a finished download can be redone', async () => {
  const srv = await changingServer(Buffer.alloc(0));
  srv.state.status = 404;
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(srv.url, { filename: 'x.bin' });
    await finished(m, t.id);
    assert.equal(m.list()[0].status, 'error');
    assert.throws(() => m.redownload(t.id), /terminé/);
    assert.throws(() => m.redownload('nope'), /introuvable/);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('redownload: a video stream (HLS) is replaced under its name too', async () => {
  const segs = (tag) => Array.from({ length: 3 }, (_, i) => Buffer.from(`${tag}-${i}-`.repeat(4000)));
  const routes = {};
  const publish = (list) => {
    list.forEach((b, i) => (routes[`/seg${i}.ts`] = b));
    routes['/v.m3u8'] = `#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n${list.map((_, i) => `#EXTINF:4,\nseg${i}.ts\n`).join('')}#EXT-X-ENDLIST\n`;
  };
  publish(segs('old'));
  const server = http.createServer((req, res) => {
    const body = routes[req.url.split('?')[0]];
    res.writeHead(body === undefined ? 404 : 200, { 'Content-Type': 'application/octet-stream' });
    res.end(body);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, convertToMp4: false }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(`http://127.0.0.1:${server.address().port}/v.m3u8`, { filename: 'clip' });
    await finished(m, t.id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    const fresh = segs('new');
    publish(fresh);
    m.redownload(t.id);
    await finished(m, t.id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.deepEqual(fs.readdirSync(dir), [done.filename]);
    assert.deepEqual(fs.readFileSync(path.join(dir, done.filename)), Buffer.concat(fresh));
  } finally {
    await m.shutdown();
    server.closeAllConnections?.();
    await new Promise((r) => server.close(r));
  }
});
