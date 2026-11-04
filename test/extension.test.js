'use strict';

// The extension's background script, run under Node with a fake `chrome` and a fake `fetch`:
// responses go in as the browser would report them, and what ends up in the tab's list and in the call
// to the app is checked. sniff.js is loaded through importScripts, as Chrome's service worker does.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const DIR = path.join(__dirname, '..', 'extension');
const TAB_URL = 'https://site.example/serie/1';
const REF = 'https://player.example/embed/9';
const ORI = 'https://player.example';

const PAGES = {
  'https://cdn.example/v/master.txt': '#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:4,\ns0.ts\n#EXT-X-ENDLIST\n',
  'https://cdn.example/v/index.m3u8': '#EXTM3U\n#EXTINF:4,\ns0.ts\n#EXT-X-ENDLIST\n',
  'https://cdn.example/clear/manifest.mpd': '<MPD type="static" mediaPresentationDuration="PT10M"><Representation height="1080"/></MPD>',
  'https://cdn.example/drm/manifest.mpd': '<MPD><ContentProtection schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"/></MPD>',
  'https://cdn.example/blob/data': '{"not":"a stream"}',
};

const bodyOf = (text) => {
  const bytes = new TextEncoder().encode(text);
  let sent = false;
  return { getReader: () => ({ read: async () => (sent ? { done: true } : ((sent = true), { done: false, value: bytes })), cancel: async () => {} }) };
};
const hdrs = (o) => Object.entries(o).map(([name, value]) => ({ name, value }));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Loads background.js in a fresh context; `local` is what the extension's options hold.
 * `appReply(init)` stands in for the app's answer (it may throw, as a closed app does), `refuse` lists addresses
 * that answer 403 when the extension reads them itself (no Referer, no Origin of the player).
 */
function load(local = {}, { appReply = null, refuse = [] } = {}) {
  const listeners = {};
  const ev = (name) => ({ addListener: (fn, filter, extra) => ((listeners[name] ??= []).push({ fn, filter, extra }), undefined) });
  const area = (obj) => ({
    get: async (k) => (typeof k === 'string' ? { [k]: obj[k] } : Object.fromEntries(Object.entries(k).map(([a, d]) => [a, obj[a] ?? d]))),
    set: async (o) => Object.assign(obj, o),
    remove: async (k) => [].concat(k).forEach((x) => delete obj[x]),
  });
  const calls = []; // what reached the app
  const fetches = []; // every fetch the extension made itself
  const fakeFetch = async (url, init = {}) => {
    fetches.push({ url, headers: init.headers });
    if (url.startsWith('http://127.0.0.1:6800')) {
      calls.push({ url, body: JSON.parse(init.body) });
      return appReply ? appReply(init) : { ok: true, status: 200, json: async () => ({ pending: true }) };
    }
    if (refuse.includes(url)) return { ok: false, status: 403, text: async () => 'forbidden', body: bodyOf('forbidden') };
    const text = PAGES[url] ?? '';
    return { ok: true, status: 200, text: async () => text, body: bodyOf(text) };
  };
  const chrome = {
    storage: { session: area({}), local: area({ token: 'tok', port: 6800, ...local }), onChanged: ev('storage.onChanged') },
    cookies: { getAll: async () => [{ name: 'a', value: 'b' }] },
    action: { setBadgeBackgroundColor: async () => {}, setBadgeText: async () => {} },
    alarms: { create() {}, onAlarm: ev('alarm') },
    downloads: { onCreated: ev('dl') },
    runtime: { onInstalled: ev('inst'), onMessage: ev('msg'), getManifest: () => ({ version: 'test' }) },
    contextMenus: { create() {}, onClicked: ev('ctx') },
    tabs: {
      onUpdated: ev('tabs.updated'),
      onRemoved: ev('tabs.removed'),
      get: async (id) => ({ id, url: TAB_URL, title: 'Serie - Episode 1' }),
      sendMessage: async () => ({ title: 'Ma Série - S01E01' }),
    },
    webRequest: { onResponseStarted: ev('rs'), onBeforeRequest: ev('br'), onSendHeaders: ev('sh') },
  };
  const ctx = vm.createContext({ chrome, navigator: { userAgent: 'test' }, fetch: fakeFetch, URL, console, setTimeout, clearTimeout, AbortController, Uint8Array, TextEncoder, Promise, Map, Date });
  ctx.importScripts = (f) => vm.runInContext(fs.readFileSync(path.join(DIR, f), 'utf8'), ctx);
  vm.runInContext(fs.readFileSync(path.join(DIR, 'background.js'), 'utf8'), ctx);

  const respond = (requestId, url, mime, sent = { Referer: REF, Origin: ORI }, extra = {}) => {
    listeners.sh.forEach((l) => l.fn({ requestId, tabId: 7, requestHeaders: hdrs(sent) }));
    listeners.rs.forEach((l) => l.fn({ tabId: 7, frameId: 3, statusCode: 200, type: 'xmlhttprequest', requestId, url, responseHeaders: hdrs({ 'content-type': mime }), ...extra }));
  };
  const message = (msg, sender = { tab: { id: 7 }, frameId: 3 }) => new Promise((resolve) => listeners.msg[0].fn(msg, sender, resolve));
  return {
    ctx,
    listeners,
    calls,
    fetches,
    respond,
    items: async () => (await message({ type: 'list', tabId: 7 })).items,
    download: (url) => message({ type: 'download', tabId: 7, url }),
    report: () => message({ type: 'diag', tabId: 7 }),
    setDiag: (on) => listeners['storage.onChanged'][0].fn({ diag: { newValue: on } }, 'local'),
  };
}

test('a stream goes to the app with the Referer and Origin the browser sent with it, and the page apart', async () => {
  const x = load();
  await sleep(20);
  assert.equal(typeof x.ctx.Sniff?.classify, 'function', 'sniff.js was loaded through importScripts');
  assert.deepEqual([...x.listeners.sh[0].extra], ['requestHeaders', 'extraHeaders'], 'Chrome shows Referer and Origin only with extraHeaders');

  x.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(40);
  const items = await x.items();
  assert.equal(items.length, 1);
  assert.equal(items[0].referer, REF);
  assert.equal(items[0].origin, ORI);

  assert.equal((await x.download(items[0].url)).ok, true);
  const body = x.calls.at(-1).body;
  assert.equal(body.referer, REF, 'the frame that asked, not the tab');
  assert.equal(body.origin, ORI);
  assert.equal(body.page, TAB_URL, 'the page is kept apart, to come back to for a fresh link');
  assert.equal(body.kind, 'hls');
  assert.equal(body.filename, 'Ma Série - S01E01');
  assert.equal(body.sniffed, true);
});

test('a playlist the extension cannot read itself (403) is still offered, not taken for a live stream', async () => {
  const x = load({}, { refuse: ['https://cdn.example/v/index.m3u8', 'https://cdn.example/clear/manifest.mpd'] });
  await sleep(20);
  x.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  x.respond('r2', 'https://cdn.example/clear/manifest.mpd', 'application/dash+xml');
  await sleep(80);
  const items = await x.items();
  assert.ok(items.some((i) => i.url.endsWith('index.m3u8') && i.kind === 'hls'), 'the app reads it with the right headers');
  assert.ok(items.some((i) => i.url.endsWith('manifest.mpd') && i.kind === 'dash'));
});

test('a playlist that really is live is still left out', async () => {
  PAGES['https://cdn.example/live/index.m3u8'] = '#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\ns0.ts\n#EXTINF:4,\ns1.ts\n';
  const x = load();
  await sleep(20);
  x.respond('r1', 'https://cdn.example/live/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(60);
  assert.ok(!(await x.items()).some((i) => i.url.includes('/live/')));
});

test('a failed send says why, instead of blaming the token every time', async () => {
  const closed = () => {
    throw new TypeError('Failed to fetch');
  };
  const answer = (status) => () => ({ ok: status < 400, status, json: async () => ({}) });
  const cases = [
    ['no token in the options', { token: '' }, undefined, 'notoken'],
    ['the app is closed or the port is wrong', {}, closed, 'unreachable'],
    ['the app refuses the token', {}, answer(401), 'badtoken'],
    ['the app refuses the request', {}, answer(400), 'refused'],
  ];
  for (const [what, local, appReply, reason] of cases) {
    const x = load(local, { appReply });
    await sleep(20);
    x.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
    await sleep(40);
    const r = await x.download('https://cdn.example/v/index.m3u8');
    assert.equal(r.ok, false, what);
    assert.equal(r.reason, reason, what);
    assert.ok(r.message.length > 20, `${what}: a sentence to show`);
  }
  const x = load();
  await sleep(20);
  const gone = await x.download('https://cdn.example/never-seen.m3u8');
  assert.equal(gone.reason, 'gone', 'the stream is no longer in the list');
  const refused = await load({}, { appReply: answer(401) });
  await sleep(20);
  refused.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(40);
  assert.match((await refused.download('https://cdn.example/v/index.m3u8')).message, /jeton/i, 'a refused token names the token');
  const unreachable = load({}, { appReply: closed });
  await sleep(20);
  unreachable.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(40);
  assert.doesNotMatch((await unreachable.download('https://cdn.example/v/index.m3u8')).message, /jeton/i, 'a closed app is not a token problem');
});

test('a successful send carries no failure', async () => {
  const x = load();
  await sleep(20);
  x.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(40);
  const r = await x.download('https://cdn.example/v/index.m3u8');
  assert.equal(r.ok, true);
  assert.equal(r.reason, undefined);
});

test('a request that carried no Referer falls back on the tab, as before', async () => {
  const x = load();
  await sleep(20);
  x.respond('r1', 'https://cdn.example/other/clip.mp4', 'video/mp4', {}, { responseHeaders: hdrs({ 'content-type': 'video/mp4', 'content-length': String(5 * 1024 * 1024) }) });
  await sleep(40);
  const clip = (await x.items()).find((i) => i.url.endsWith('clip.mp4'));
  assert.ok(clip, 'a 5 MB mp4 is offered');
  await x.download(clip.url);
  const body = x.calls.at(-1).body;
  assert.equal(body.referer, TAB_URL);
  assert.equal(body.origin, '');
  assert.equal(body.filename, 'Ma Série - S01E01.mp4');
});

test('a playlist served as plain text is found by reading its first bytes, and nothing else is probed needlessly', async () => {
  const x = load();
  await sleep(20);
  x.respond('r1', 'https://cdn.example/v/master.txt', 'text/plain');
  await sleep(60);
  assert.ok(x.fetches.some((f) => f.url.endsWith('master.txt') && /bytes=0-1023/.test(f.headers?.Range)), 'a ranged probe was made');
  assert.ok((await x.items()).some((i) => i.url.endsWith('master.txt') && i.kind === 'hls'), 'offered as a stream');

  const before = x.fetches.length;
  x.respond('r2', 'https://cdn.example/app.js', 'application/octet-stream');
  x.respond('r3', 'https://cdn.example/blob/data', 'application/octet-stream');
  await sleep(60);
  assert.ok(!x.fetches.slice(before).some((f) => f.url.endsWith('app.js')), 'a script is not probed');
  assert.ok(!(await x.items()).some((i) => i.url.endsWith('/blob/data')), 'JSON found inside is not offered');
});

test('the same folder of a host is probed only twice, so a hundred disguised chunks cost two requests', async () => {
  const x = load();
  await sleep(20);
  for (let i = 0; i < 20; i++) x.respond(`c${i}`, `https://cdn.example/chunks/${i}`, 'application/octet-stream');
  await sleep(80);
  assert.equal(x.fetches.filter((f) => f.url.includes('/chunks/')).length, 2);
});

test('a DASH manifest goes to yt-dlp under the page name; a protected one is not offered', async () => {
  const x = load();
  await sleep(20);
  x.respond('r1', 'https://cdn.example/drm/manifest.mpd', 'application/dash+xml');
  x.respond('r2', 'https://cdn.example/clear/manifest.mpd', 'application/dash+xml');
  await sleep(80);
  const items = await x.items();
  assert.ok(!items.some((i) => i.url.includes('/drm/')), 'protected: nothing the app could download');
  const dash = items.find((i) => i.url.includes('/clear/'));
  assert.deepEqual([dash.kind, dash.height, dash.duration], ['dash', 1080, 600]);
  await x.download(dash.url);
  const body = x.calls.at(-1).body;
  assert.equal(body.kind, 'ytdlp');
  assert.equal(body.referer, REF);
  assert.equal(body.origin, ORI);
  assert.equal(body.filename, 'Ma Série - S01E01');
});

test('the diagnostic is off unless asked for, then lists each request with what the browser sent and the verdict', async () => {
  const off = load();
  await sleep(20);
  off.respond('r1', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  await sleep(40);
  assert.equal((await off.report()).count, 0, 'nothing recorded by default');

  const x = load({ diag: true });
  await sleep(20);
  x.respond('a', 'https://cdn.example/v/index.m3u8', 'application/vnd.apple.mpegurl');
  x.respond('b', 'https://cdn.example/p/page.png', 'image/png', { Referer: REF });
  for (let i = 0; i < 10; i++) x.respond(`c${i}`, `https://cdn.example/seg/${i}.ts`, 'video/mp2t', {});
  await sleep(60);
  const r = await x.report();
  assert.ok(r.text.includes(`Referer envoyé=${REF}  Origin envoyé=${ORI}`));
  assert.ok(r.text.includes('Origin envoyé=-'), 'the second request had none');
  assert.ok(r.text.includes('RECONNU (hls'));
  assert.ok(r.text.includes('ignoré : ni type MIME ni extension de média'));
  assert.ok(r.text.includes('+6 semblables'), 'ten chunks: four listed, six counted');
  assert.equal(r.count, 2 + 4);

  x.setDiag(false);
  x.respond('z', 'https://cdn.example/after-off.m3u8', 'application/x-mpegURL');
  await sleep(30);
  const after = await x.report();
  assert.equal(after.count, r.count, 'switched off: nothing more is recorded');
  assert.ok(!after.text.split('\n').some((l) => l.startsWith('[') && l.includes('after-off')), 'not among the recorded requests');
});
