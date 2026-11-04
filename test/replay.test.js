'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { DownloadManager } = require('../src/engine/manager');
const { createExtensionServer } = require('../src/server');
const { YtdlpDownload, headerArgs } = require('../src/engine/ytdlp');
const { tmpDir, randomBody, waitFor } = require('./helpers');

const finished = (m, id) => waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status));

/** Serves `routes` (path -> Buffer|string) and remembers what each request carried. */
function recordingServer(routes) {
  const seen = [];
  const server = http.createServer((req, res) => {
    const p = req.url.split('?')[0];
    seen.push({ path: p, referer: req.headers.referer ?? null, origin: req.headers.origin ?? null, userAgent: req.headers['user-agent'] ?? null });
    const body = routes[p];
    if (body === undefined) {
      res.writeHead(404);
      return res.end();
    }
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': Buffer.byteLength(body) });
    res.end(body);
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        base: `http://127.0.0.1:${server.address().port}`,
        seen,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

const PLAYER = 'https://player.example/embed/7';
const PLAYER_ORIGIN = 'https://player.example';

test('a file download replays the Referer and the Origin the browser sent', async () => {
  const srv = await recordingServer({ '/f.bin': randomBody(200 * 1024) });
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  try {
    const { id } = m.add(`${srv.base}/f.bin`, { referer: PLAYER, origin: PLAYER_ORIGIN });
    await finished(m, id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    assert.ok(srv.seen.length >= 2, 'the probe and the transfer');
    for (const r of srv.seen) {
      assert.equal(r.referer, PLAYER);
      assert.equal(r.origin, PLAYER_ORIGIN);
    }
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('an HLS stream replays them on the playlist and on every segment', async () => {
  const names = ['s0.ts', 's1.ts', 's2.ts'];
  const routes = { '/index.m3u8': `#EXTM3U\n#EXT-X-TARGETDURATION:4\n${names.map((n) => `#EXTINF:4,\n${n}\n`).join('')}#EXT-X-ENDLIST\n` };
  names.forEach((n) => (routes[`/${n}`] = randomBody(40_000)));
  const srv = await recordingServer(routes);
  const m = new DownloadManager({ settings: { dir: tmpDir(), connections: 2 }, tickMs: 50 });
  m.start();
  try {
    const { id } = m.add(`${srv.base}/index.m3u8`, { referer: PLAYER, origin: PLAYER_ORIGIN, filename: 'film.mp4' });
    await finished(m, id);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error);
    assert.equal(srv.seen.length, 4);
    for (const r of srv.seen) {
      assert.equal(r.referer, PLAYER, r.path);
      assert.equal(r.origin, PLAYER_ORIGIN, r.path);
    }
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('the page to reopen for a fresh link is kept apart from the Referer that is replayed', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  const a = m.add('http://127.0.0.1:9/a.bin', { referer: PLAYER, origin: PLAYER_ORIGIN, page: 'https://site.example/serie/1', sniffed: true, paused: true });
  assert.equal(m.tasks.get(a.id).state.pageUrl, 'https://site.example/serie/1');
  assert.equal(m.tasks.get(a.id).state.headers.referer, PLAYER);
  assert.equal(m.tasks.get(a.id).state.headers.origin, PLAYER_ORIGIN);
  const b = m.add('http://127.0.0.1:9/b.bin', { referer: 'https://site.example/p', paused: true });
  assert.equal(m.tasks.get(b.id).state.pageUrl, 'https://site.example/p', 'without a separate page, the Referer is the page, as before');
  assert.equal(m.tasks.get(b.id).state.headers.origin, '');
});

test('a fresh link brings its own Origin and keeps the page', () => {
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  const { id } = m.add('http://127.0.0.1:9/a.bin', { referer: PLAYER, origin: PLAYER_ORIGIN, page: 'https://site.example/serie/1', paused: true });
  m.refresh(id, { url: 'http://127.0.0.1:9/a2.bin', referer: 'https://other.example/e/1', origin: 'https://other.example' });
  const s = m.tasks.get(id).state;
  assert.equal(s.headers.referer, 'https://other.example/e/1');
  assert.equal(s.headers.origin, 'https://other.example');
  assert.equal(s.pageUrl, 'https://site.example/serie/1');
});

test('the extension API hands origin and page over, to the confirmation window and to the manager', async () => {
  const TOKEN = 'secret-token-for-tests';
  const ext = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop';
  const manager = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  const asked = [];
  let ask = true;
  const api = createExtensionServer({ manager, token: () => TOKEN, onRequest: (r) => (asked.push(r), ask) });
  const port = await api.listen(0);
  try {
    const post = (payload) =>
      fetch(`http://127.0.0.1:${port}/add`, {
        method: 'POST',
        headers: { Origin: ext, 'X-DM-Token': TOKEN, 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
    const payload = { url: 'http://127.0.0.1:9/v.bin', referer: PLAYER, origin: PLAYER_ORIGIN, page: 'https://site.example/serie/1', sniffed: true };
    assert.equal((await post(payload)).status, 200);
    assert.equal(asked[0].origin, PLAYER_ORIGIN);
    assert.equal(asked[0].page, 'https://site.example/serie/1');
    assert.equal(asked[0].referer, PLAYER);

    ask = false; // no confirmation window: straight to the manager
    const { id } = await (await post(payload)).json();
    assert.equal(manager.tasks.get(id).state.headers.origin, PLAYER_ORIGIN);
    assert.equal(manager.tasks.get(id).state.pageUrl, 'https://site.example/serie/1');
  } finally {
    await manager.shutdown();
    await api.close();
  }
});

test('yt-dlp gets the request context of a page it does not know, not of YouTube', () => {
  const headers = { referer: PLAYER, origin: PLAYER_ORIGIN, userAgent: 'Agent/1.0' };
  const unknown = headerArgs(headers, 'https://cdn.example/v/manifest.mpd');
  assert.deepEqual(unknown.slice(0, 2), ['--referer', PLAYER]);
  assert.ok(unknown.join(' ').includes(`--add-headers Origin:${PLAYER_ORIGIN}`));
  assert.ok(unknown.join(' ').includes('--user-agent Agent/1.0'));
  assert.deepEqual(headerArgs(headers, 'https://www.youtube.com/watch?v=abcdefghijk'), []);
  assert.deepEqual(headerArgs(undefined, 'https://cdn.example/v.mpd'), []);
  assert.deepEqual(headerArgs({ referer: 'not a url', origin: 'x\ny', userAgent: '' }, 'https://cdn.example/v.mpd'), [], 'nothing that is not a clean value is passed on');
});

test('the yt-dlp command line carries those headers before the address', () => {
  const dl = Object.create(YtdlpDownload.prototype);
  dl.state = { url: 'https://cdn.example/v/manifest.mpd', dir: '/d', connections: 4, headers: { referer: PLAYER, origin: PLAYER_ORIGIN, userAgent: '' } };
  dl.ctx = { settings: () => ({ videoMaxHeight: 1080 }), ffmpeg: () => null };
  const args = dl._args('/t/yt-dlp.exe');
  assert.ok(args.includes('--referer') && args[args.indexOf('--referer') + 1] === PLAYER);
  assert.equal(args.at(-1), 'https://cdn.example/v/manifest.mpd');
});
