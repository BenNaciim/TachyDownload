'use strict';

/**
 * Produces the contract fixtures the Rust core is checked against (design 7.1): it drives the current JavaScript
 * engine against local servers and records what it does. Run once before the JavaScript is removed:
 *
 *   node tools/make-fixtures.js
 *
 * Output: crates/tachy-core/tests/fixtures/{settings,downloads,snapshots,api,strings}.json
 */

const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { createExtensionServer } = require('../src/server');
const { startServer, tmpDir, randomBody, waitFor } = require('../test/helpers');

const OUT = path.join(__dirname, '..', 'crates', 'tachy-core', 'tests', 'fixtures');
const write = (name, data) => fs.writeFileSync(path.join(OUT, name), JSON.stringify(data, null, 2) + '\n');

// ---- (c) settings: what the manager makes of what it is given -------------------------------------------

function settingsFixture() {
  const dir = 'C:\\Downloads';
  const inputs = [
    {},
    { connections: 99 },
    { connections: -3 },
    { connections: 'abc' },
    { maxConcurrent: 0 },
    { maxConcurrent: 50 },
    { videoMaxHeight: 5000 },
    { videoMaxHeight: 240.4 },
    { speedLimitKBps: -5 },
    { speedLimitKBps: 123.6 },
    { proxy: 'proxy.local:3128' },
    { proxy: 'socks5://x:1' },
    { proxy: '  ' },
    { speedSchedule: { enabled: true, from: '22:00', to: '06:00', limitKBps: 50 } },
    { speedSchedule: { enabled: true, from: '25:00', to: 'x', limitKBps: -1 } },
    { speedSchedule: { enabled: 'yes' } },
    { schedule: { enabled: true, from: '09:30', to: '9:5' } },
    { afterAll: 'shutdown' },
    { afterAll: 'format-the-disk' },
    { categories: 1, autoRetry: 0, adaptive: '', duplicates: null },
    { rules: '.pdf = Docs', ytCookies: 'firefox', foo: 'bar' },
    { confirmDownloads: false, notifications: false, closeToTray: false, autoUpdate: false },
  ];
  return {
    dir,
    cases: inputs.map((input) => {
      const m = new DownloadManager({ settings: { dir, ...input } });
      return { input, output: m.settings };
    }),
  };
}

// ---- (a)(b) one task per kind and status, and what the window gets for each ----------------------------

async function tasksFixture() {
  const dir = tmpDir();
  const storePath = path.join(tmpDir(), 'downloads.json');
  const fast = await startServer(randomBody(120 * 1024), { disposition: 'attachment; filename="rapport.bin"' });
  const slow = await startServer(randomBody(2 * 1024 * 1024), { bytesPerSec: 200 * 1024 });
  const gone = await startServer(Buffer.alloc(0));
  gone.setStatus = null;
  const goneServer = http.createServer((req, res) => (res.writeHead(404), res.end()));
  await new Promise((r) => goneServer.listen(0, '127.0.0.1', r));
  const goneUrl = `http://127.0.0.1:${goneServer.address().port}/missing.bin`;

  const m = new DownloadManager({ storePath, settings: { dir, connections: 3 }, tickMs: 40 });
  m.start();
  const byName = {};
  try {
    byName.completed = m.add(fast.url, { referer: 'https://example.com/page', cookie: 'a=b' }).id;
    await waitFor(() => m.list().find((t) => t.id === byName.completed).status === 'completed');
    byName.paused = m.add(slow.url, { filename: 'half.bin' }).id;
    await waitFor(() => m.list().find((t) => t.id === byName.paused).downloaded > 300 * 1024, 20000);
    await m.pause(byName.paused);
    byName.error = m.add(goneUrl, { filename: 'missing.bin' }).id;
    await waitFor(() => m.list().find((t) => t.id === byName.error).status === 'error');
    byName.scheduled = m.add(fast.url, { filename: 'later.bin', startAt: Date.UTC(2099, 0, 1) }).id;
    byName.hls = m.add('http://127.0.0.1:9/live/master.m3u8', { kind: 'hls', filename: 'stream', paused: true, page: 'https://example.com/watch', sniffed: true }).id;
    byName.ytdlp = m.add('https://www.youtube.com/watch?v=abcdefghijk', { kind: 'ytdlp', paused: true, cookie: 'x=y' }).id;
    byName.limited = m.add(fast.url, { filename: 'limited.bin', paused: true }).id;
    m.setTaskLimit(byName.limited, 250);
    m.saveNow();
  } finally {
    await m.shutdown();
    await fast.close();
    await slow.close();
    await gone.close();
    goneServer.closeAllConnections?.();
    await new Promise((r) => goneServer.close(r));
  }
  // What a fresh manager shows after loading that file is what the Rust core must show too.
  const fresh = new DownloadManager({ storePath, settings: { dir } });
  fresh.load();
  const snapshots = fresh.list();
  const stored = JSON.parse(fs.readFileSync(storePath, 'utf8'));
  return { stored, byName, snapshots, tmp: os.tmpdir() };
}

// ---- (d) API answers -------------------------------------------------------------------------------------

function raw(port, { method = 'GET', path: p = '/', headers = {}, body = null, host = `127.0.0.1:${port}` }) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: { ...headers, Host: host, Connection: 'close' } }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, text: Buffer.concat(chunks).toString('utf8') }));
    });
    req.on('error', reject);
    req.end(body);
  });
}

async function apiFixture() {
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  const TOKEN = 'tok'.repeat(16);
  const api = createExtensionServer({ manager: m, token: () => TOKEN });
  const port = await api.listen(0);
  const ext = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop';
  const json = { 'Content-Type': 'application/json' };
  const cases = [
    { name: 'ping', req: { path: '/ping' } },
    { name: 'ping from an extension', req: { path: '/ping', headers: { Origin: ext } } },
    { name: 'ping from a web page', req: { path: '/ping', headers: { Origin: 'https://evil.example' } } },
    { name: 'foreign host', req: { path: '/ping', host: 'evil.example' } },
    { name: 'localhost host', req: { path: '/ping', host: `localhost:${port}` } },
    { name: 'no token', req: { method: 'POST', path: '/add', headers: { ...json, Origin: ext }, body: '{"url":"http://x/"}' } },
    { name: 'wrong token', req: { method: 'POST', path: '/add', headers: { ...json, 'X-DM-Token': 'nope' }, body: '{"url":"http://x/"}' } },
    { name: 'unknown route', req: { path: '/nope', headers: { 'X-DM-Token': TOKEN } } },
    { name: 'refresh requests', req: { path: '/refresh-requests', headers: { 'X-DM-Token': TOKEN, Origin: ext } } },
    { name: 'add a file link', req: { method: 'POST', path: '/add', headers: { ...json, 'X-DM-Token': TOKEN }, body: '{"url":"file:///etc/passwd"}' } },
    { name: 'add garbage', req: { method: 'POST', path: '/add', headers: { ...json, 'X-DM-Token': TOKEN }, body: '{"url":"not a url"}' } },
    { name: 'add without url', req: { method: 'POST', path: '/add', headers: { ...json, 'X-DM-Token': TOKEN }, body: '{}' } },
    { name: 'refresh unknown id', req: { method: 'POST', path: '/refresh', headers: { ...json, 'X-DM-Token': TOKEN }, body: '{"id":"nope","url":"http://x/"}' } },
    { name: 'preflight', req: { method: 'OPTIONS', path: '/add', headers: { Origin: ext } } },
    { name: 'preflight from a web page', req: { method: 'OPTIONS', path: '/add', headers: { Origin: 'https://evil.example' } } },
  ];
  const out = [];
  try {
    for (const c of cases) {
      const r = await raw(port, c.req);
      let body = null;
      try {
        body = r.text ? JSON.parse(r.text) : null;
      } catch {
        body = r.text;
      }
      out.push({
        name: c.name,
        req: { ...c.req, host: c.req.host?.replace(String(port), '{port}') ?? null },
        status: r.status,
        body,
        headers: Object.fromEntries(
          ['access-control-allow-origin', 'access-control-allow-methods', 'access-control-allow-headers', 'access-control-max-age', 'vary', 'cache-control', 'content-type'].filter((h) => r.headers[h] !== undefined).map((h) => [h, r.headers[h]]),
        ),
      });
    }
  } finally {
    await api.close();
    await m.shutdown();
  }
  return { token: TOKEN, cases: out };
}

// ---- (f) user-visible French strings of the engine -------------------------------------------------------

function stringsFixture() {
  const files = ['util', 'manager', 'download', 'probe', 'net'].map((f) => path.join(__dirname, '..', 'src', 'engine', `${f}.js`));
  files.push(path.join(__dirname, '..', 'src', 'server.js'));
  const found = new Set();
  for (const file of files) {
    const src = fs.readFileSync(file, 'utf8');
    const rx = /'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\.)*)`/g;
    let m;
    while ((m = rx.exec(src))) {
      const lit = m[1] ?? m[2] ?? m[3];
      for (const part of lit.split(/\$\{[^}]*\}/)) {
        const s = part.replace(/\\'/g, "'").replace(/\\"/g, '"').replace(/\\n/g, '\n');
        // a sentence a person reads: several words, with an accent or an apostrophe typical of French text
        const looksFrench = /^[A-ZÀ-Ý]/.test(s) || /[À-ÿ’]/.test(s) || /\b(le|la|les|du|un|une|de|des|ce|cette|pas|est|avec|sur|dans)\b/i.test(s);
        if (s.length >= 12 && /\s/.test(s) && looksFrench && !/^[\\/^(]|=>|\{|\}|\$|^Content-|^bytes /.test(s.trim())) {
          found.add(s);
        }
      }
    }
  }
  return [...found].sort();
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  write('settings.json', settingsFixture());
  const t = await tasksFixture();
  write('downloads.json', t.stored);
  write('snapshots.json', { byName: t.byName, snapshots: t.snapshots });
  write('api.json', await apiFixture());
  write('strings.json', stringsFixture());
  console.log('fixtures written to', OUT);
  process.exit(0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
