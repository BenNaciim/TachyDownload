'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { DownloadManager } = require('../src/engine/manager');
const { parsePlaylist } = require('../src/engine/hls');
const { tmpDir, randomBody, waitFor } = require('./helpers');

const finished = (m, id) =>
  waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status));

/** Serves `routes` (path -> Buffer|string); `slowMs` delays segment answers. */
function hlsServer(routes, { slowMs = 0 } = {}) {
  const hits = [];
  const server = http.createServer((req, res) => {
    const p = req.url.split('?')[0];
    hits.push(p);
    const body = routes[p];
    if (body === undefined) {
      res.writeHead(404);
      return res.end();
    }
    const send = () => {
      res.writeHead(200, { 'Content-Type': 'application/octet-stream' });
      res.end(body);
    };
    if (slowMs && p.startsWith('/seg')) setTimeout(send, slowMs);
    else send();
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        base: `http://127.0.0.1:${server.address().port}`,
        hits,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

const media = (names, extra = '') =>
  `#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n${extra}${names.map((n) => `#EXTINF:4,\n${n}\n`).join('')}#EXT-X-ENDLIST\n`;

test('parsePlaylist: master variants sorted by bandwidth, relative urls resolved', () => {
  const pl = parsePlaylist(
    '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080\nhigh/index.m3u8\n',
    'http://h/a/master.m3u8',
  );
  assert.equal(pl.master, true);
  assert.equal(pl.variants[0].url, 'http://h/a/high/index.m3u8');
});

test('HLS: master playlist, best variant, segments joined in order', async () => {
  const segs = Array.from({ length: 9 }, () => randomBody(50_000));
  const routes = {
    '/master.m3u8': '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900\nhigh.m3u8\n',
    '/low.m3u8': media(['seg-low.ts']),
    '/seg-low.ts': Buffer.from('nope'),
    '/high.m3u8': media(segs.map((_, i) => `seg${i}.ts`)),
  };
  segs.forEach((b, i) => (routes[`/seg${i}.ts`] = b));
  const srv = await hlsServer(routes, { slowMs: 20 });
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 4 }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(`${srv.base}/master.m3u8`, { filename: 'film.mp4' });
    assert.equal(t.kind, 'hls');
    await finished(m, t.id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.equal(done.filename, 'film.ts');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'film.ts')), Buffer.concat(segs));
    assert.ok(!srv.hits.includes('/seg-low.ts'));
    assert.equal(done.downloaded, done.size);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('HLS: AES-128 segments are decrypted (explicit IV and sequence IV)', async () => {
  const key = crypto.randomBytes(16);
  const plain = [randomBody(30_000), randomBody(30_001)];
  const enc = (buf, iv) => {
    const c = crypto.createCipheriv('aes-128-cbc', key, iv);
    return Buffer.concat([c.update(buf), c.final()]);
  };
  const seqIv = Buffer.alloc(16);
  seqIv.writeUInt32BE(7, 12); // media sequence 7 -> first segment
  const seq1 = Buffer.alloc(16);
  seq1.writeUInt32BE(8, 12);
  const routes = {
    '/k.bin': key,
    '/v.m3u8': media(['seg0.ts', 'seg1.ts'], '#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI="k.bin"\n'),
    '/seg0.ts': enc(plain[0], seqIv),
    '/seg1.ts': enc(plain[1], seq1),
  };
  const srv = await hlsServer(routes);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(`${srv.base}/v.m3u8`);
    await finished(m, t.id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.deepEqual(fs.readFileSync(path.join(dir, done.filename)), Buffer.concat(plain));
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('HLS: DRM and live streams are refused with a clear message', async () => {
  const srv = await hlsServer({
    '/drm.m3u8': media(['seg0.ts'], '#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://x"\n'),
    '/live.m3u8': '#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg0.ts\n',
    '/seg0.ts': Buffer.from('x'),
  });
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  try {
    const a = m.add(`${srv.base}/drm.m3u8`);
    const b = m.add(`${srv.base}/live.m3u8`);
    await finished(m, a.id);
    await finished(m, b.id);
    const list = m.list();
    assert.match(list.find((t) => t.id === a.id).error, /DRM/);
    assert.match(list.find((t) => t.id === b.id).error, /directs/);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('HLS: pause then resume continues without re-downloading finished segments', async () => {
  const segs = Array.from({ length: 12 }, () => randomBody(20_000));
  const routes = { '/v.m3u8': media(segs.map((_, i) => `seg${i}.ts`)) };
  segs.forEach((b, i) => (routes[`/seg${i}.ts`] = b));
  const srv = await hlsServer(routes, { slowMs: 120 });
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, connections: 2 }, tickMs: 50 });
  m.start();
  try {
    const t = m.add(`${srv.base}/v.m3u8`, { filename: 'clip' });
    await waitFor(() => (m.list()[0].hlsProgress ?? '').match(/^[3-9] /));
    await m.pause(t.id);
    const before = srv.hits.filter((h) => h.startsWith('/seg')).length;
    assert.equal(m.list()[0].status, 'paused');
    m.resume(t.id);
    await finished(m, t.id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.deepEqual(fs.readFileSync(path.join(dir, done.filename)), Buffer.concat(segs));
    const total = srv.hits.filter((h) => h.startsWith('/seg')).length;
    assert.ok(total - before <= 12 - 3 + 2, `refetched too much: ${total - before}`);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
