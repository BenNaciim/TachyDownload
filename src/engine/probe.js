'use strict';

const {
  sanitizeFilename,
  filenameFromContentDisposition,
  filenameFromUrl,
} = require('./util');

/**
 * Asks the server for the first byte only. A 206 answer proves range support and
 * gives the total size in Content-Range; a plain 200 means a single stream.
 */
async function probe(url, headers = {}, signal, fetchFn = fetch) {
  const res = await fetchFn(url, {
    headers: { ...headers, Range: 'bytes=0-0' },
    redirect: 'follow',
    signal,
  });
  try {
    if (res.status >= 400) {
      const err = new Error(`Le serveur a répondu ${res.status}`);
      err.fatal = true;
      err.expired = res.status === 401 || res.status === 403 || res.status === 410;
      err.status = res.status;
      throw err;
    }
    let size = null;
    let rangeOk = false;
    if (res.status === 206) {
      const m = /bytes\s+\d+-\d+\/(\d+)/i.exec(res.headers.get('content-range') ?? '');
      if (m) {
        size = Number(m[1]);
        rangeOk = true;
      }
    }
    if (!rangeOk) {
      const len = Number(res.headers.get('content-length'));
      size = Number.isFinite(len) && len > 0 ? len : null;
    }
    const name =
      filenameFromContentDisposition(res.headers.get('content-disposition')) ||
      filenameFromUrl(res.url || url) ||
      'download';
    return {
      finalUrl: res.url || url,
      size,
      rangeOk,
      filename: sanitizeFilename(name),
      etag: res.headers.get('etag'),
      lastModified: res.headers.get('last-modified'),
    };
  } finally {
    await res.body?.cancel().catch(() => {});
  }
}

/**
 * True when the address answers with a web page rather than a file: the link of a video page on a site
 * the app does not know by name, which only yt-dlp can turn into a video. Any failure counts as "no".
 */
async function isWebPage(url, headers = {}, fetchFn = fetch) {
  try {
    const res = await fetchFn(url, { headers: { ...headers, Range: 'bytes=0-0' }, redirect: 'follow', signal: AbortSignal.timeout(8000) });
    await res.body?.cancel().catch(() => {});
    return res.status < 400 && /^(text\/html|application\/xhtml)/i.test(res.headers.get('content-type') ?? '');
  } catch {
    return false;
  }
}

module.exports = { probe, isWebPage };
