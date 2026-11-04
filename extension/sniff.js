'use strict';

// Pure helpers of the extension's stream detection: what a network response is, from its address, its type
// and its size, then from its first bytes. No browser API in here, so the tests can run it under Node.
// The background page loads it as a plain script (Firefox), through importScripts (Chrome).
(() => {
  const MIN_FILE_BYTES = 150 * 1024; // skips tracking pixels, short UI sounds, thumbnails
  const SEGMENT_EXT = /\.(ts|m4s|cmfv|cmfa)$/i; // pieces of a stream, never listed on their own
  const MEDIA_MIME = /^(video|audio)\//i;
  const MEDIA_EXT = /\.(mp4|m4v|webm|mkv|mov|flv|mp3|m4a|aac|ogg|opus|wav)$/i;
  const STATIC_EXT = /\.(js|mjs|css|json|html?|png|jpe?g|gif|webp|avif|svg|ico|woff2?|ttf|otf|eot|map|wasm|xml)$/i;
  // Types a server gives to anything it has no better name for: playlists and videos are often served that way.
  const VAGUE_MIME = new Set(['', 'text/plain', 'application/octet-stream', 'binary/octet-stream', 'application/binary']);
  const MAX_PLAYLIST_BYTES = 512 * 1024;
  const MIN_HIDDEN_FILE_BYTES = 16 * 1024 * 1024;
  const MIME_EXT = { mpeg: 'mp3', 'x-m4v': 'm4v', 'x-matroska': 'mkv', quicktime: 'mov', 'x-flv': 'flv', 'mp4a-latm': 'm4a', 'x-m4a': 'm4a', 'x-wav': 'wav', 'x-ms-wmv': 'wmv' };

  const decode = (s) => {
    try {
      return decodeURIComponent(s);
    } catch {
      return s;
    }
  };

  function extensionOf(path, mime) {
    const fromPath = /\.([a-z0-9]{2,4})$/i.exec(path)?.[1]?.toLowerCase();
    if (fromPath && MEDIA_EXT.test(`.${fromPath}`)) return fromPath;
    const sub = (mime.split('/')[1] || '').toLowerCase();
    return MIME_EXT[sub] ?? (/^[a-z0-9]{2,4}$/.test(sub) ? sub : 'mp4');
  }

  /**
   * What a response is, going by its address, its type and its size (0 when unknown):
   * { kind: 'hls' | 'dash' | 'file', ext } for a stream or a media file, { kind: 'probe' } when only its first bytes
   * can tell, { kind: null, why } for everything else.
   */
  function classify({ url, mime = '', size = 0 }) {
    let path = '';
    let query = '';
    try {
      const u = new URL(url);
      path = u.pathname;
      query = decode(u.search);
    } catch {
      return { kind: null, why: 'adresse illisible' };
    }
    const type = String(mime).split(';')[0].trim().toLowerCase();
    if (SEGMENT_EXT.test(path) || type === 'video/mp2t') return { kind: null, why: 'morceau de flux (.ts, .m4s)' };
    if (/mpegurl/.test(type) || /\.m3u8?(\/|$)/i.test(path) || /\.m3u8?(?=$|&|#)/i.test(query)) return { kind: 'hls', ext: 'm3u8' };
    if (/dash\+xml/.test(type) || /\.mpd(\/|$)/i.test(path)) return { kind: 'dash', ext: 'mpd' };
    if (MEDIA_MIME.test(type) || MEDIA_EXT.test(path)) {
      if (size && size < MIN_FILE_BYTES) return { kind: null, why: `média de moins de ${MIN_FILE_BYTES / 1024} Ko` };
      return { kind: 'file', ext: extensionOf(path, type) };
    }
    const couldBePlaylist = size === 0 || size <= MAX_PLAYLIST_BYTES;
    if (VAGUE_MIME.has(type) && !STATIC_EXT.test(path) && (couldBePlaylist || size >= MIN_HIDDEN_FILE_BYTES)) return { kind: 'probe' };
    return { kind: null, why: 'ni type MIME ni extension de média' };
  }

  const MP3_SYNC = (b) => b[0] === 0xff && (b[1] & 0xe0) === 0xe0 && (b[1] & 0x06) !== 0;

  /** What a response really is, from its first bytes (a Uint8Array): the same answers as classify(), or null. */
  function fromBytes(b) {
    if (!b || b.length === 0) return null;
    const at = (i, s) => i + s.length <= b.length && [...s].every((c, k) => b[i + k] === c.charCodeAt(0));
    let i = b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf ? 3 : 0; // byte-order mark
    while (i < b.length && [0x20, 0x09, 0x0a, 0x0d].includes(b[i])) i++;
    if (at(i, '#EXTM3U')) return { kind: 'hls' };
    if (b[i] === 0x3c) {
      const head = String.fromCharCode(...b.subarray(i, i + 1024));
      return /<MPD[\s>]/.test(head) ? { kind: 'dash' } : null;
    }
    if (at(4, 'ftyp') || at(4, 'moov') || at(4, 'mdat')) return { kind: 'file', ext: 'mp4' };
    if (b[0] === 0x1a && b[1] === 0x45 && b[2] === 0xdf && b[3] === 0xa3) return { kind: 'file', ext: 'webm' };
    if (at(0, 'FLV') && b[3] === 1) return { kind: 'file', ext: 'flv' };
    if (at(0, 'ID3') || MP3_SYNC(b)) return { kind: 'file', ext: 'mp3' };
    if (at(0, 'OggS')) return { kind: 'file', ext: 'ogg' };
    return null; // including the pieces of a stream (MPEG-TS, fragmented MP4): they are not videos of their own
  }

  const isoSeconds = (iso) => {
    const m = /^P(?:(\d+)D)?(?:T(?:(\d+(?:\.\d+)?)H)?(?:(\d+(?:\.\d+)?)M)?(?:(\d+(?:\.\d+)?)S)?)?$/.exec(iso || '');
    if (!m) return 0;
    return Math.round(Number(m[1] || 0) * 86400 + Number(m[2] || 0) * 3600 + Number(m[3] || 0) * 60 + Number(m[4] || 0));
  };

  /** What a DASH manifest says: whether it is protected or live, how long it is, its best height. */
  function dashInfo(text) {
    const t = String(text ?? '');
    let height = 0;
    for (const m of t.matchAll(/\bheight="(\d+)"/g)) height = Math.max(height, Number(m[1]));
    return {
      drm: /<ContentProtection/i.test(t), // any protection scheme: the key is not ours to get
      live: /\btype="dynamic"/.test(t),
      duration: isoSeconds(/mediaPresentationDuration="([^"]+)"/.exec(t)?.[1]),
      height,
    };
  }

  const api = { MIN_FILE_BYTES, classify, fromBytes, dashInfo };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else globalThis.Sniff = api;
})();
