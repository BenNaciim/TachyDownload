'use strict';

// Works in Chrome, Edge and Firefox (MV3): Firefox runs this as a background script.
const DEFAULTS = { token: '', port: 6800, intercept: true, minSizeMB: 0 };
const MENU_ID = 'dm-send-link';

const MEDIA_MIME = /^(video|audio)\/|mpegurl/i;
const MEDIA_EXT = /\.(mp4|m4v|webm|mkv|mov|flv|mp3|m4a|aac|ogg|opus|wav|m3u8)$/i;
const SEGMENT_EXT = /\.(ts|m4s|cmfv|cmfa)$/i; // pieces of a stream, never listed on their own
const MIN_FILE_BYTES = 150 * 1024; // skips tracking pixels, short UI sounds, thumbnails

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

/** Talks to the app. Resolves with the parsed answer, or null when it is closed or refuses us. */
async function callApp(pathname, payload, { quiet = false } = {}) {
  const settings = await getSettings();
  if (!settings.token) {
    if (!quiet) flash('?', '#b45309');
    return null;
  }
  try {
    const res = await fetch(`http://127.0.0.1:${settings.port}${pathname}`, {
      method: payload ? 'POST' : 'GET',
      headers: { 'Content-Type': 'application/json', 'X-DM-Token': settings.token },
      body: payload ? JSON.stringify(payload) : undefined,
    });
    if (!res.ok) {
      if (!quiet) flash(res.status === 401 ? 'KEY' : '!', '#b42318');
      return null;
    }
    return await res.json();
  } catch {
    // App closed or port mismatch: let the browser keep its own download.
    return null;
  }
}

/** Returns true only if the app accepted the download. */
async function sendToApp({ url, referrer, filename, kind, sniffed, size }) {
  const answer = await callApp('/add', {
    url,
    size: size || 0,
    referer: referrer || '',
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

async function swapLink(target, { url, referrer }) {
  const answer = await callApp('/refresh', {
    id: target.id,
    url,
    referer: referrer || '',
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

/** A video seen on the network: if it is the replacement for a stalled one, hand it over. */
async function swapForSniffed(item, tab) {
  const own = [...pagesOpened.entries()].find(([, v]) => v.tabId === tab.id)?.[0];
  for (const target of await refreshRequests()) {
    if (!target.auto || target.kind !== item.kind) continue;
    const here = target.id === own || (target.pageUrl && target.pageUrl.split('#')[0] === (tab.url || '').split('#')[0]);
    const sameFile = item.kind === 'file' && target.size > 0 && item.size === target.size;
    if (!(item.kind === 'hls' ? here : here && sameFile)) continue;
    if (await swapLink(target, { url: item.url, referrer: tab.url })) {
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

function describe(details) {
  const url = details.url;
  let path;
  try {
    path = new URL(url).pathname;
  } catch {
    return null;
  }
  if (SEGMENT_EXT.test(path)) return null;
  const mime = header(details, 'content-type').split(';')[0].trim().toLowerCase();
  const isHls = /mpegurl/.test(mime) || /\.m3u8?$/i.test(path);
  if (!isHls && !MEDIA_MIME.test(mime) && !MEDIA_EXT.test(path)) return null;
  if (!isHls && /^video\/mp2t$/.test(mime)) return null; // transport-stream chunk

  let size = Number(header(details, 'content-length')) || 0;
  const range = /\/(\d+)$/.exec(header(details, 'content-range'));
  if (range) size = Number(range[1]);
  if (!isHls && size && size < MIN_FILE_BYTES) return null;

  const ext = (/\.([a-z0-9]{2,4})$/i.exec(path)?.[1] ?? (mime.split('/')[1] || 'mp4')).toLowerCase();
  return { url, kind: isHls ? 'hls' : 'file', mime, size, ext: isHls ? 'm3u8' : ext, at: Date.now() };
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

let chain = Promise.resolve(); // serialises read-modify-write on the per-tab lists
chrome.webRequest.onResponseStarted.addListener(
  (details) => {
    if (details.tabId < 0 || details.statusCode >= 400) return;
    const item = describe(details);
    if (!item) return;
    chain = chain.then(async () => {
      const tab = await chrome.tabs.get(details.tabId).catch(() => null);
      if (!tab || isVideoPage(tab.url)) return undefined;
      if (item.kind === 'hls') {
        // Variant playlists are only pieces of a master playlist; the app picks the best variant itself.
        try {
          const text = await (await fetch(item.url)).text();
          item.master = text.includes('#EXT-X-STREAM-INF');
          item.live = !item.master && !text.includes('#EXT-X-ENDLIST');
        } catch {
          item.master = false;
        }
      }
      let list = await readList(details.tabId);
      if (item.live) return undefined; // live streams cannot be downloaded
      if (await swapForSniffed(item, tab)) return undefined;
      if (item.kind === 'hls') {
        if (item.master) list = list.filter((x) => !(x.kind === 'hls' && !x.master));
        else if (list.some((x) => x.kind === 'hls' && x.master)) return undefined;
      }
      const known = list.find((x) => x.url === item.url);
      if (known) {
        known.size = Math.max(known.size, item.size);
        return writeList(details.tabId, list);
      }
      return writeList(details.tabId, [...list, item].slice(-25));
    });
  },
  { urls: ['http://*/*', 'https://*/*'], types: ['media', 'xmlhttprequest', 'other'] },
  ['responseHeaders'],
);

// A new page in the same tab starts a fresh list.
chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === 'loading' && changeInfo.url) chain = chain.then(() => writeList(tabId, []));
  const url = changeInfo.url || tab?.url;
  if (isVideoPage(url) && (changeInfo.url || changeInfo.status === 'complete')) {
    chrome.action.setBadgeBackgroundColor({ tabId, color: '#3b5bdb' }).catch?.(() => {});
    chrome.action.setBadgeText({ tabId, text: '1' }).catch?.(() => {});
  }
});
chrome.tabs.onRemoved.addListener((tabId) => {
  chain = chain.then(() => store.remove(tabKey(tabId)));
});

const cleanTitle = (t) =>
  (t || 'video')
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 100);

async function listFor(tabId) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (tab && isVideoPage(tab.url)) {
    return [{ url: tab.url, kind: 'ytdlp', mime: '', size: 0, ext: '', at: Date.now() }];
  }
  // Biggest first: the main video is almost always larger than ads, previews and trailers.
  // A stream playlist has no size to compare and is normally the main content, so it goes first.
  const weight = (x) => (x.kind === 'hls' ? Number.MAX_SAFE_INTEGER : x.size);
  return [...(await readList(tabId))].sort((a, b) => weight(b) - weight(a));
}

async function downloadItem(tabId, url) {
  const item = (await listFor(tabId)).find((x) => x.url === url);
  if (!item) return false;
  const tab = await chrome.tabs.get(tabId);
  if (item.kind === 'ytdlp') return sendToApp({ url: item.url, referrer: tab.url, kind: 'ytdlp' });
  const base = cleanTitle(tab.title);
  return sendToApp({
    url: item.url,
    referrer: tab.url,
    filename: item.kind === 'hls' ? base : `${base}.${item.ext}`,
    kind: item.kind,
    sniffed: true,
    size: item.size,
  });
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  const tabId = msg.tabId ?? sender.tab?.id;
  (async () => {
    if (msg.type === 'list') return { items: await listFor(tabId) };
    if (msg.type === 'download') return { ok: await downloadItem(tabId, msg.url) };
    if (msg.type === 'poll') {
      await pollRefresh();
      return { ok: true };
    }
    return {};
  })().then(sendResponse, () => sendResponse({ error: true }));
  return true; // answer asynchronously
});
