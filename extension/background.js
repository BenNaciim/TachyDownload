/* global Sniff, importScripts */
'use strict';

// Works in Chrome, Edge and Firefox (MV3): Firefox runs this as a background script.
// The detection rules live in sniff.js: Firefox lists it before this file, Chrome's service worker imports it.
if (typeof importScripts === 'function') importScripts('sniff.js');

const DEFAULTS = { token: '', port: 6800, intercept: true, minSizeMB: 0 };
const MENU_ID = 'dm-send-link';

const store = chrome.storage.session ?? chrome.storage.local;
const tabKey = (tabId) => `media:${tabId}`;

async function getSettings() {
  return { ...DEFAULTS, ...(await chrome.storage.local.get(DEFAULTS)) };
}

async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join('; ');
  } catch {
    return '';
  }
}

function flash(text, color) {
  chrome.action.setBadgeBackgroundColor({ color });
  chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: '' }), 4000);
}

// Why the last call the user asked for failed, in a sentence for the button on the video. Background polling
// (quiet) never touches it.
const WHY = {
  notoken: () => ({ reason: 'notoken', message: 'Aucun jeton dans les options de l’extension : copiez-le depuis l’application (Paramètres → Extension navigateur).' }),
  unreachable: (port) => ({ reason: 'unreachable', message: `Tachy Download ne répond pas sur le port ${port} : lancez l’application, ou corrigez le port dans les options de l’extension.` }),
  badtoken: () => ({ reason: 'badtoken', message: 'Tachy Download a refusé le jeton : copiez-le à nouveau depuis l’application (Paramètres → Extension navigateur) dans les options de l’extension.' }),
  refused: (status) => ({ reason: 'refused', message: `Tachy Download a refusé la demande (erreur ${status}).` }),
  gone: () => ({ reason: 'gone', message: 'Cette vidéo n’est plus dans la liste de l’extension : relancez la lecture, puis réessayez.' }),
};
let failure = null;

/** Talks to the app. Resolves with the parsed answer, or null when it is closed or refuses us. */
async function callApp(pathname, payload, { quiet = false } = {}) {
  if (!quiet) failure = null;
  const settings = await getSettings();
  if (!settings.token) {
    if (!quiet) {
      failure = WHY.notoken();
      flash('?', '#b45309');
    }
    return null;
  }
  let res;
  try {
    res = await fetch(`http://127.0.0.1:${settings.port}${pathname}`, {
      method: payload ? 'POST' : 'GET',
      headers: { 'Content-Type': 'application/json', 'X-DM-Token': settings.token },
      body: payload ? JSON.stringify(payload) : undefined,
    });
  } catch {
    // App closed or port mismatch: let the browser keep its own download.
    if (!quiet) failure = WHY.unreachable(settings.port);
    return null;
  }
  if (!res.ok) {
    if (!quiet) {
      failure = res.status === 401 ? WHY.badtoken() : WHY.refused(res.status);
      flash(res.status === 401 ? 'KEY' : '!', '#b42318');
    }
    return null;
  }
  return res.json().catch(() => ({}));
}

/** Returns true only if the app accepted the download. */
async function sendToApp({ url, referrer, origin, page, filename, kind, sniffed, size }) {
  const answer = await callApp('/add', {
    url,
    size: size || 0,
    referer: referrer || '',
    origin: origin || '',
    page: page || '',
    cookie: await cookieHeader(url),
    userAgent: navigator.userAgent,
    filename: filename || '',
    kind: kind || '',
    sniffed: Boolean(sniffed),
  });
  if (answer) flash('OK', '#15803d');
  return Boolean(answer);
}

// ---- Expired links: the app reports downloads that need a fresh address ----------------

const sameName = (a, b) => a && b && decodeURIComponent(a).toLowerCase() === decodeURIComponent(b).toLowerCase();
const baseName = (p) => (p || '').split(/[\\/]/).pop();
const pagesOpened = new Map(); // task id -> { tabId, at }
const OPEN_COOLDOWN_MS = 5 * 60 * 1000;
const OPEN_TIMEOUT_MS = 45 * 1000;

async function refreshRequests() {
  return (await callApp('/refresh-requests', null, { quiet: true }))?.requests ?? [];
}

async function swapLink(target, { url, referrer, origin }) {
  const answer = await callApp('/refresh', {
    id: target.id,
    url,
    referer: referrer || '',
    origin: origin || '',
    cookie: await cookieHeader(url),
    userAgent: navigator.userAgent,
  });
  if (answer) flash('OK', '#15803d');
  return Boolean(answer);
}

/** Reopens, in a background tab, the page of a video whose link died; the sniffer then supplies the new one. */
async function pollRefresh() {
  for (const target of await refreshRequests()) {
    if (!target.auto || !target.pageUrl) continue;
    const prev = pagesOpened.get(target.id);
    if (prev && Date.now() - prev.at < OPEN_COOLDOWN_MS) continue;
    try {
      const tab = await chrome.tabs.create({ url: target.pageUrl, active: false });
      pagesOpened.set(target.id, { tabId: tab.id, at: Date.now() });
      setTimeout(() => chrome.tabs.remove(tab.id).catch?.(() => {}), OPEN_TIMEOUT_MS);
    } catch {
      // tab could not be opened; the next poll retries after the cooldown
    }
  }
}

if (chrome.alarms) {
  chrome.alarms.create('dm-poll', { periodInMinutes: 0.5 });
  chrome.alarms.onAlarm.addListener((a) => a.name === 'dm-poll' && pollRefresh());
}

/** The kind of download the app makes of a stream: a DASH manifest goes to yt-dlp, which merges its audio and video. */
const appKind = (item) => (item.kind === 'dash' ? 'ytdlp' : item.kind);

/** A video seen on the network: if it is the replacement for a stalled one, hand it over. */
async function swapForSniffed(item, tab) {
  const own = [...pagesOpened.entries()].find(([, v]) => v.tabId === tab.id)?.[0];
  for (const target of await refreshRequests()) {
    if (!target.auto || target.kind !== appKind(item)) continue;
    const here = target.id === own || (target.pageUrl && target.pageUrl.split('#')[0] === (tab.url || '').split('#')[0]);
    const sameFile = item.kind === 'file' && target.size > 0 && item.size === target.size;
    if (!(item.kind !== 'file' ? here : here && sameFile)) continue;
    if (await swapLink(target, { url: item.url, referrer: item.referer || tab.url, origin: item.origin })) {
      pagesOpened.delete(target.id);
      if (own === target.id) chrome.tabs.remove(tab.id).catch?.(() => {});
      return true;
    }
  }
  return false;
}

chrome.downloads.onCreated.addListener(async (item) => {
  const settings = await getSettings();
  if (!settings.intercept) return;
  const url = item.finalUrl || item.url;
  if (!/^https?:\/\//i.test(url)) return;
  if (settings.minSizeMB > 0 && item.fileSize > 0 && item.fileSize < settings.minSizeMB * 1024 * 1024) return;

  // The user reopened the page and clicked download again: that is the fresh link of a stalled one.
  const name = baseName(item.filename);
  const size = item.fileSize > 0 ? item.fileSize : item.totalBytes;
  const target = (await refreshRequests()).find(
    (t) => (name && sameName(name, t.filename)) || (size > 0 && t.size === size),
  );
  const accepted = target
    ? await swapLink(target, { url, referrer: item.referrer })
    : await sendToApp({ url, referrer: item.referrer, size });
  if (accepted) {
    chrome.downloads.cancel(item.id, () => chrome.downloads.erase({ id: item.id }));
  }
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: MENU_ID,
    title: 'Télécharger avec Tachy Download',
    contexts: ['link', 'video', 'audio', 'image'],
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== MENU_ID) return;
  const url = info.linkUrl || info.srcUrl;
  if (!url || !/^https?:\/\//i.test(url)) return;
  const ok = await sendToApp({ url, referrer: info.pageUrl || tab?.url });
  if (!ok) flash('!', '#b42318');
});

// ---- Video detection (the IDM-style "download this video" feature) -------------------

function header(details, name) {
  return details.responseHeaders?.find((h) => h.name.toLowerCase() === name)?.value ?? '';
}

/** What the sniffer makes of a response (see sniff.js), plus its type and size. */
function classifyResponse(details) {
  const mime = header(details, 'content-type').split(';')[0].trim().toLowerCase();
  let size = Number(header(details, 'content-length')) || 0;
  const range = /\/(\d+)$/.exec(header(details, 'content-range'));
  if (range) size = Number(range[1]);
  return { ...Sniff.classify({ url: details.url, mime, size }), mime, size };
}

const entryFor = (details, c, kind, ext) => ({ url: details.url, kind, mime: c.mime, size: c.size, ext, frameId: details.frameId, at: Date.now() });

/** The list entry for a response that is plainly a stream or a media file, or null. */
function describe(details, c = classifyResponse(details)) {
  return ['hls', 'dash', 'file'].includes(c.kind) ? entryFor(details, c, c.kind, c.ext) : null;
}

// A response that could be anything (a playlist served as text/plain, a video as octet-stream) is read at its
// first bytes. Few probes per tab and per folder of the same host: a hundred chunks are not a hundred probes.
const PROBES_PER_TAB = 30;
const PROBES_PER_FOLDER = 2;
const probed = new Map(); // tabId -> { total, folders: Map }

function mayProbe(tabId, url) {
  let u;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  const p = probed.get(tabId) ?? { total: 0, folders: new Map() };
  const folder = u.host + u.pathname.replace(/[^/]*$/, '');
  if (p.total >= PROBES_PER_TAB || (p.folders.get(folder) ?? 0) >= PROBES_PER_FOLDER) return false;
  p.total++;
  p.folders.set(folder, (p.folders.get(folder) ?? 0) + 1);
  probed.set(tabId, p);
  return true;
}

/** The first bytes of a response, without ever reading more than asked from a server that ignores Range. */
async function readHead(url, length = 1024) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 4000);
  try {
    const res = await fetch(url, { headers: { Range: `bytes=0-${length - 1}` }, credentials: 'include', signal: ctl.signal });
    if (!res.ok || !res.body) return null;
    const reader = res.body.getReader();
    const head = new Uint8Array(length);
    let got = 0;
    while (got < length) {
      const { done, value } = await reader.read();
      if (done) break;
      const take = Math.min(value.length, length - got);
      head.set(value.subarray(0, take), got);
      got += take;
    }
    reader.cancel().catch(() => {});
    return head.subarray(0, got);
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/** Looks inside an ambiguous response; the list entry it turns out to be, or null. */
async function probeResponse(details, c) {
  if (!mayProbe(details.tabId, details.url)) return null;
  const found = Sniff.fromBytes(await readHead(details.url));
  if (!found) return null;
  if (found.kind === 'file' && c.size && c.size < Sniff.MIN_FILE_BYTES) return null;
  return entryFor(details, c, found.kind, found.kind === 'hls' ? 'm3u8' : found.kind === 'dash' ? 'mpd' : found.ext);
}

// Pages where yt-dlp finds the streams itself (the raw network requests would be useless chunks).
const VIDEO_PAGE = [
  /^https?:\/\/(www\.|m\.|music\.)?youtube\.com\/(watch|shorts\/|live\/)/i,
  /^https?:\/\/youtu\.be\/[\w-]+/i,
  /^https?:\/\/(www\.)?vimeo\.com\/\d+/i,
  /^https?:\/\/(www\.)?dailymotion\.com\/video\//i,
  /^https?:\/\/(www\.)?twitch\.tv\/videos\/\d+/i,
];
const isVideoPage = (url) => VIDEO_PAGE.some((re) => re.test(url || ''));

async function readList(tabId) {
  return (await store.get(tabKey(tabId)))[tabKey(tabId)] ?? [];
}

async function writeList(tabId, list) {
  await store.set({ [tabKey(tabId)]: list });
  const text = list.length ? String(list.length) : '';
  try {
    await chrome.action.setBadgeBackgroundColor({ tabId, color: '#3b5bdb' });
    await chrome.action.setBadgeText({ tabId, text });
  } catch {
    // tab already gone
  }
}

/** What a master playlist offers, to tell streams apart: best height, audio languages, length in seconds. */
async function describeMaster(text, base) {
  const lines = text.split(/\r?\n/);
  const langs = new Set();
  let height = 0;
  let variant = null;
  lines.forEach((l, i) => {
    if (l.startsWith('#EXT-X-MEDIA') && /TYPE=AUDIO/.test(l)) {
      const name = /NAME="([^"]*)"/.exec(l)?.[1] || /LANGUAGE="([^"]*)"/.exec(l)?.[1];
      if (name) langs.add(name);
    } else if (l.startsWith('#EXT-X-STREAM-INF')) {
      height = Math.max(height, Number(/RESOLUTION=\d+x(\d+)/.exec(l)?.[1]) || 0);
      const next = lines[i + 1]?.trim();
      if (!variant && next && !next.startsWith('#')) variant = next;
    }
  });
  let duration = 0;
  try {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), 4000);
    const body = await (await fetch(new URL(variant, base).href, { signal: ctl.signal })).text();
    clearTimeout(timer);
    for (const m of body.matchAll(/#EXTINF:([\d.]+)/g)) duration += Number(m[1]);
  } catch {
    // no length: the entry just shows less
  }
  return { height, langs: [...langs], duration: Math.round(duration) };
}

// What the browser put in Referer and Origin when it asked for a response: the download replays exactly that, which
// is what lets a host that checks them (most video hosts do) hand the stream over. Chrome only shows these two
// headers with "extraHeaders"; Firefox does not know that word.
const SEEN_FILTER = { urls: ['http://*/*', 'https://*/*'], types: ['media', 'xmlhttprequest', 'other', 'sub_frame'] };
const sentHeaders = new Map(); // requestId -> { referer, origin }
const onSendHeaders = (details) => {
  if (details.tabId < 0) return;
  const get = (n) => details.requestHeaders?.find((h) => h.name.toLowerCase() === n)?.value ?? '';
  sentHeaders.set(details.requestId, { referer: get('referer'), origin: get('origin') });
  if (sentHeaders.size > 600) sentHeaders.delete(sentHeaders.keys().next().value);
};
try {
  chrome.webRequest.onSendHeaders.addListener(onSendHeaders, SEEN_FILTER, ['requestHeaders', 'extraHeaders']);
} catch {
  chrome.webRequest.onSendHeaders.addListener(onSendHeaders, SEEN_FILTER, ['requestHeaders']);
}

/** Adds a stream seen on the network to its tab's list, unless it is live, protected, or a duplicate. */
async function keep(item, details) {
  const tab = await chrome.tabs.get(details.tabId).catch(() => null);
  if (!tab || isVideoPage(tab.url)) return undefined;
  // Read from here, a playlist may be refused (the host wants the player's Referer and Origin, which this
  // request does not carry): unreadable is not live, and it is still offered, since the app asks with the right headers.
  if (item.kind === 'hls') {
    // Variant playlists are only pieces of a master playlist; the app picks the best variant itself.
    try {
      const res = await fetch(item.url);
      const text = res.ok ? await res.text() : '';
      if (text.slice(0, 64).includes('#EXTM3U')) {
        item.master = text.includes('#EXT-X-STREAM-INF');
        item.live = !item.master && !text.includes('#EXT-X-ENDLIST');
        if (item.master) Object.assign(item, await describeMaster(text, item.url));
      } else {
        item.master = false;
      }
    } catch {
      item.master = false;
    }
  } else if (item.kind === 'dash') {
    try {
      const res = await fetch(item.url);
      if (res.ok) {
        const info = Sniff.dashInfo(await res.text());
        if (info.drm || info.live) return undefined; // protected or live: nothing the app could download
        Object.assign(item, { height: info.height, duration: info.duration });
      }
    } catch {
      // unreadable from here: still offered, yt-dlp reads it with the right headers
    }
  }
  let list = await readList(details.tabId);
  if (item.live) return undefined; // live streams cannot be downloaded
  if (await swapForSniffed(item, tab)) return undefined;
  if (item.kind === 'hls') {
    if (item.master) list = list.filter((x) => !(x.kind === 'hls' && !x.master));
    else if (list.some((x) => x.kind === 'hls' && x.master)) return undefined;
  }
  // The same video announced again under another address (new token, other CDN host) is one entry.
  if ((item.master || item.kind === 'dash') && item.duration) {
    const sig = (x) => `${x.duration}|${x.height}|${(x.langs ?? []).join(',')}`;
    list = list.filter((x) => !((x.master || x.kind === 'dash') && x.url !== item.url && sig(x) === sig(item)));
  }
  const known = list.find((x) => x.url === item.url);
  if (known) {
    known.size = Math.max(known.size, item.size);
    known.referer = item.referer || known.referer;
    known.origin = item.origin || known.origin;
    return writeList(details.tabId, list);
  }
  return writeList(details.tabId, [...list, item].slice(-25));
}

let chain = Promise.resolve(); // serialises read-modify-write on the per-tab lists
chrome.webRequest.onResponseStarted.addListener(
  (details) => {
    if (details.tabId < 0 || details.statusCode >= 400) return;
    const c = classifyResponse(details);
    if (!c.kind) return;
    const sent = sentHeaders.get(details.requestId) ?? {};
    const found = c.kind === 'probe' ? probeResponse(details, c) : Promise.resolve(describe(details, c));
    found.then((item) => {
      if (!item) return;
      item.referer = sent.referer ?? '';
      item.origin = sent.origin ?? '';
      chain = chain.then(() => keep(item, details));
    });
  },
  { urls: ['http://*/*', 'https://*/*'], types: ['media', 'xmlhttprequest', 'other'] },
  ['responseHeaders'],
);

// A player frame that loads another page (a different language or host on the same site) drops what it served before.
chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    if (details.tabId < 0) return;
    chain = chain.then(async () => {
      const list = await readList(details.tabId);
      const kept = list.filter((x) => x.frameId !== details.frameId);
      if (kept.length !== list.length) await writeList(details.tabId, kept);
    });
  },
  { urls: ['http://*/*', 'https://*/*'], types: ['sub_frame'] },
);

// A new page in the same tab starts a fresh list.
chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === 'loading' && changeInfo.url) {
    chain = chain.then(() => writeList(tabId, []));
    dchain = dchain.then(() => store.remove(dbgKey(tabId)));
    probed.delete(tabId);
  }
  const url = changeInfo.url || tab?.url;
  if (isVideoPage(url) && (changeInfo.url || changeInfo.status === 'complete')) {
    chrome.action.setBadgeBackgroundColor({ tabId, color: '#3b5bdb' }).catch?.(() => {});
    chrome.action.setBadgeText({ tabId, text: '1' }).catch?.(() => {});
  }
});
chrome.tabs.onRemoved.addListener((tabId) => {
  probed.delete(tabId);
  chain = chain.then(() => store.remove([tabKey(tabId), dbgKey(tabId)]));
});

// ---- Diagnostic (opt-in, off by default) ---------------------------------------------------
// Records what the browser saw on a tab (frames, media-ish requests, the Referer/Origin it sent) and what the
// sniffer made of each, so a stream that is not offered can be explained. Nothing leaves the browser unless the
// user copies the report from the popup.

const DIAG_MAX = 400;
const DIAG_GROUP_MAX = 4; // the same kind of request repeated (hundreds of chunks) is counted, not listed
const dbgKey = (tabId) => `dbg:${tabId}`;
let diagOn = false;
chrome.storage.local.get({ diag: false }).then((s) => {
  diagOn = Boolean(s.diag);
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && changes.diag) diagOn = Boolean(changes.diag.newValue);
});

/** Why the sniffer did or did not keep a response, in words. */
function diagVerdict(details) {
  const c = classifyResponse(details);
  if (['hls', 'dash', 'file'].includes(c.kind)) return `RECONNU (${c.kind}${c.size ? '' : ', taille inconnue'})`;
  if (c.kind === 'probe') return 'à examiner : réponse ambiguë, ses premiers octets sont lus';
  return `ignoré : ${c.why}`;
}

let dchain = Promise.resolve();
chrome.webRequest.onResponseStarted.addListener((details) => {
  if (!diagOn || details.tabId < 0) return;
  const sent = sentHeaders.get(details.requestId) ?? {};
  let host = '';
  try {
    host = new URL(details.url).host;
  } catch {
    // keeps an empty host
  }
  const mime = header(details, 'content-type');
  const entry = {
    at: new Date().toISOString().slice(11, 19),
    type: details.type,
    status: details.statusCode,
    frame: details.frameId,
    url: details.url,
    mime,
    length: header(details, 'content-length'),
    range: header(details, 'content-range'),
    from: details.initiator || details.documentUrl || '',
    referer: sent.referer ?? '',
    origin: sent.origin ?? '',
    verdict: diagVerdict(details),
    group: `${host}|${details.type}|${mime.split(';')[0]}|${/\.([a-z0-9]{1,5})$/i.exec(details.url.split(/[?#]/)[0])?.[1] ?? ''}`,
    more: 0,
  };
  dchain = dchain.then(async () => {
    const key = dbgKey(details.tabId);
    const list = (await store.get(key))[key] ?? [];
    const same = list.filter((x) => x.group === entry.group);
    if (same.length >= DIAG_GROUP_MAX) same[same.length - 1].more++;
    else list.push(entry);
    await store.set({ [key]: list.slice(-DIAG_MAX) });
  });
}, SEEN_FILTER, ['responseHeaders']);

/** The text the user copies from the popup. */
async function diagReport(tabId) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  const key = dbgKey(tabId);
  const entries = (await store.get(key))[key] ?? [];
  const offered = await listFor(tabId, undefined, { page: false });
  const m = chrome.runtime.getManifest();
  const lines = [
    `Tachy Download – diagnostic de l’extension ${m.version}`,
    `Navigateur : ${navigator.userAgent}`,
    `Page : ${tab?.url ?? '?'}`,
    `Proposé dans la fenêtre de l’extension : ${offered.length ? offered.map((x) => `${x.kind} ${x.url}`).join(' | ') : '(rien)'}`,
    `Requêtes enregistrées : ${entries.length}`,
    '',
  ];
  for (const e of entries) {
    lines.push(
      `[${e.at}] ${e.type} ${e.status} cadre=${e.frame} type=${e.mime || '-'} taille=${e.length || '-'}${e.range ? ` plage=${e.range}` : ''}${e.more ? ` (+${e.more} semblables)` : ''}`,
      `  ${e.url}`,
      `  demandée par=${e.from || '-'}  Referer envoyé=${e.referer || '-'}  Origin envoyé=${e.origin || '-'}`,
      `  → ${e.verdict}`,
    );
  }
  return { text: lines.join('\n'), count: entries.length };
}

const cleanTitle = (t) =>
  (t || 'video')
    // eslint-disable-next-line no-control-regex -- control characters are not allowed in a file name
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 100);

async function listFor(tabId, frameId, { page = false } = {}) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (tab && isVideoPage(tab.url)) {
    return [{ url: tab.url, kind: 'ytdlp', mime: '', size: 0, ext: '', at: Date.now() }];
  }
  // Biggest first: the main video is almost always larger than ads, previews and trailers.
  // A stream playlist has no size to compare and is normally the main content, so it goes first.
  const weight = (x) => (x.kind === 'hls' || x.kind === 'dash' ? Number.MAX_SAFE_INTEGER : x.size);
  const all = await readList(tabId);
  // The video being pointed at lives in one frame: offer what that frame played, when it played anything.
  const own = all.filter((x) => x.frameId === frameId);
  // Most recent first among streams: the one just started is the one being watched.
  const found = [...(own.length ? own : all)].sort((a, b) => weight(b) - weight(a) || b.at - a.at);
  // A video is playing here but no stream was seen (the player hides it, or it has not started yet): the app
  // can still try the page itself with yt-dlp, which knows many more sites.
  if (!found.length && page && tab && /^https?:/i.test(tab.url || '')) {
    return [{ url: tab.url, kind: 'ytdlp', mime: '', size: 0, ext: '', at: Date.now() }];
  }
  return found;
}

async function downloadItem(tabId, url) {
  failure = null;
  const item = (await listFor(tabId, undefined, { page: true })).find((x) => x.url === url);
  if (!item) {
    failure = WHY.gone();
    return false;
  }
  const tab = await chrome.tabs.get(tabId);
  if (item.kind === 'ytdlp') return sendToApp({ url: item.url, referrer: tab.url, page: tab.url, kind: 'ytdlp' });
  const fromPage = await Promise.race([
    chrome.tabs.sendMessage(tabId, { type: 'pageTitle' }, { frameId: 0 }).then((r) => r?.title, () => ''),
    new Promise((resolve) => setTimeout(() => resolve(''), 1500)),
  ]);
  const base = cleanTitle(fromPage || tab.title);
  return sendToApp({
    url: item.url,
    // What the browser sent with this very stream (the player's frame, not always the page): the host checks it.
    referrer: item.referer || tab.url,
    origin: item.origin,
    page: tab.url, // where to come back for a fresh link when this one expires
    filename: item.kind === 'file' ? `${base}.${item.ext}` : base,
    kind: appKind(item),
    sniffed: true,
    size: item.size,
  });
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  const tabId = msg.tabId ?? sender.tab?.id;
  (async () => {
    if (msg.type === 'list') return { items: await listFor(tabId, sender.frameId, { page: Boolean(msg.page) }) };
    if (msg.type === 'download') {
      const ok = await downloadItem(tabId, msg.url);
      return ok ? { ok } : { ok, ...failure };
    }
    if (msg.type === 'diag') return diagReport(tabId);
    if (msg.type === 'poll') {
      await pollRefresh();
      return { ok: true };
    }
    return {};
  })().then(sendResponse, () => sendResponse({ error: true }));
  return true; // answer asynchronously
});
