'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFile } = require('node:child_process');
const { Download, FatalError, httpError } = require('./download');
const { sleep, settleFinished } = require('./util');

const MAX_RETRIES = 4;

function parseAttrs(line) {
  const out = {};
  const re = /([A-Z0-9-]+)=("([^"]*)"|[^,]*)/g;
  let m;
  while ((m = re.exec(line.slice(line.indexOf(':') + 1)))) out[m[1]] = m[3] ?? m[2];
  return out;
}

/** Parses an m3u8 playlist: either a master (variants) or a media playlist (segments). */
function parsePlaylist(text, baseUrl) {
  const lines = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  if (lines[0] !== '#EXTM3U') throw new FatalError('Ce n’est pas une playlist HLS valide');
  const abs = (u) => new URL(u, baseUrl).toString();

  if (lines.some((l) => l.startsWith('#EXT-X-STREAM-INF'))) {
    const variants = [];
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].startsWith('#EXT-X-STREAM-INF')) continue;
      const a = parseAttrs(lines[i]);
      const uri = lines.slice(i + 1).find((l) => !l.startsWith('#'));
      if (uri) variants.push({ url: abs(uri), bandwidth: Number(a.BANDWIDTH) || 0, resolution: a.RESOLUTION || '' });
    }
    variants.sort((x, y) => y.bandwidth - x.bandwidth);
    return { master: true, variants };
  }

  const pl = { master: false, segments: [], map: null, key: null, ended: false, mediaSequence: 0 };
  let key = null;
  for (const l of lines) {
    if (l.startsWith('#EXT-X-MEDIA-SEQUENCE:')) pl.mediaSequence = Number(l.split(':')[1]) || 0;
    else if (l === '#EXT-X-ENDLIST') pl.ended = true;
    else if (l.startsWith('#EXT-X-KEY:')) {
      const a = parseAttrs(l);
      if (a.METHOD === 'NONE') key = null;
      else if (a.METHOD === 'AES-128' && a.URI) key = { url: abs(a.URI), iv: a.IV || null };
      else throw new FatalError('Vidéo protégée (DRM) : le téléchargement n’est pas possible');
    } else if (l.startsWith('#EXT-X-MAP:')) {
      const a = parseAttrs(l);
      if (a.URI) pl.map = { url: abs(a.URI), key };
    } else if (!l.startsWith('#')) {
      pl.segments.push({ url: abs(l), key, seq: pl.mediaSequence + pl.segments.length });
    }
  }
  return pl;
}

const ivFor = (key, seq) => {
  if (key.iv) return Buffer.from(key.iv.replace(/^0x/i, '').padStart(32, '0'), 'hex');
  const iv = Buffer.alloc(16);
  iv.writeUInt32BE(seq >>> 0, 12);
  return iv;
};

/**
 * Downloads an HLS stream (unencrypted or AES-128) by fetching its segments with
 * several connections and appending them in order to one file (.ts, or .mp4 for fMP4).
 */
class HlsDownload extends Download {
  get partPath() {
    return path.join(this.state.dir, this.state.filename + '.part');
  }

  downloaded() {
    return this.state.hls?.bytes ?? 0;
  }

  snapshot() {
    const snap = super.snapshot();
    const hls = this.state.hls;
    // One lane showing overall progress; the size is an estimate until the end.
    const total = Math.max(this.state.size ?? 0, snap.downloaded, 1);
    snap.segments = [{ start: 0, end: total - 1, pos: snap.downloaded }];
    snap.hlsProgress = hls?.total ? `${hls.next} / ${hls.total} segments` : null;
    return snap;
  }

  _headers() {
    return this._baseHeaders();
  }

  async _text(url, signal) {
    const res = await fetch(url, { headers: this._baseHeaders(), redirect: 'follow', signal });
    if (res.status >= 400) throw httpError(res.status);
    return { text: await res.text(), url: res.url || url };
  }

  async _bytes(url, signal, key, seq) {
    let attempt = 0;
    for (;;) {
      try {
        const res = await fetch(url, { headers: this._baseHeaders(), redirect: 'follow', signal });
        if (res.status >= 500 || res.status === 429) throw new Error(`HTTP ${res.status}`);
        if (res.status >= 400) throw httpError(res.status);
        let buf = Buffer.from(await res.arrayBuffer());
        if (key) {
          const k = await this._key(key.url, signal);
          const d = crypto.createDecipheriv('aes-128-cbc', k, ivFor(key, seq));
          buf = Buffer.concat([d.update(buf), d.final()]);
        }
        await this.limiter.take(buf.length, signal);
        return buf;
      } catch (err) {
        if (signal.aborted || err.fatal) throw err;
        if (++attempt > MAX_RETRIES) throw err;
        await sleep(Math.min(6000, 500 * 2 ** attempt), signal);
      }
    }
  }

  async _key(url, signal) {
    this._keys ??= new Map();
    if (!this._keys.has(url)) {
      const res = await fetch(url, { headers: this._baseHeaders(), signal });
      if (res.status >= 400) throw new FatalError('Clé de déchiffrement inaccessible');
      const k = Buffer.from(await res.arrayBuffer());
      if (k.length !== 16) throw new FatalError('Vidéo protégée (DRM) : le téléchargement n’est pas possible');
      this._keys.set(url, k);
    }
    return this._keys.get(url);
  }

  async _run(signal) {
    const s = this.state;
    fs.mkdirSync(s.dir, { recursive: true });

    let { text, url } = await this._text(s.url, signal);
    let pl = parsePlaylist(text, url);
    if (pl.master) {
      if (pl.variants.length === 0) throw new FatalError('Playlist sans flux vidéo');
      ({ text, url } = await this._text(pl.variants[0].url, signal));
      pl = parsePlaylist(text, url);
      if (pl.master) throw new FatalError('Playlist HLS non prise en charge');
    }
    if (!pl.ended) throw new FatalError('Les directs ne sont pas pris en charge');
    if (pl.segments.length === 0) throw new FatalError('Playlist vide');

    const items = pl.map ? [{ ...pl.map, seq: 0, init: true }, ...pl.segments] : pl.segments;
    const fmp4 = Boolean(pl.map) || /\.(m4s|mp4|cmfv|cmfa)(\?|$)/i.test(items[items.length - 1].url);
    s.filename = s.filename.replace(/\.(m3u8?|ts|mp4|mkv|webm)$/i, '') + (fmp4 ? '.mp4' : '.ts');

    const hls = (s.hls ??= { next: 0, bytes: 0, total: items.length });
    if (hls.total !== items.length) Object.assign(hls, { next: 0, bytes: 0, total: items.length });
    this.samples = [];

    const fh = await fs.promises.open(this.partPath, fs.existsSync(this.partPath) && hls.next > 0 ? 'r+' : 'w');
    const ticker = setInterval(() => this.emit('update'), 250);
    try {
      await fh.truncate(hls.bytes); // drop a partially written tail from an earlier run
      const window = Math.max(1, s.connections);
      const inflight = new Map();
      let fetchIdx = hls.next;
      const launch = () => {
        while (inflight.size < window && fetchIdx < items.length) {
          const i = fetchIdx++;
          const it = items[i];
          const p = this._bytes(it.url, signal, it.key, it.seq);
          p.catch(() => {});
          inflight.set(i, p);
        }
      };
      launch();
      while (hls.next < items.length) {
        const buf = await inflight.get(hls.next);
        inflight.delete(hls.next);
        let off = 0;
        while (off < buf.length) {
          const { bytesWritten } = await fh.write(buf, off, buf.length - off, hls.bytes + off);
          off += bytesWritten;
        }
        hls.bytes += buf.length;
        hls.next++;
        this.samples.push([Date.now(), buf.length]);
        s.size = Math.round((hls.bytes / hls.next) * items.length);
        launch();
      }
      await fh.sync();
    } finally {
      clearInterval(ticker);
      await fh.close().catch(() => {});
    }
    if (signal.aborted) return;

    // With ffmpeg available, the raw pieces become a regular .mp4 (stream copy, no re-encoding).
    const ffmpeg = this.ctx.ffmpeg?.();
    if (ffmpeg && this.ctx.settings?.().convertToMp4 !== false) {
      s.phase = 'convert';
      this.emit('update');
      const converted = this.partPath + '.mp4';
      try {
        await new Promise((resolve, reject) =>
          execFile(
            ffmpeg,
            ['-y', '-v', 'error', '-i', this.partPath, '-c', 'copy', '-f', 'mp4', '-movflags', '+faststart', converted],
            { windowsHide: true, signal },
            (e) => (e ? reject(e) : resolve()),
          ),
        );
        const rawPart = this.partPath;
        s.filename = s.filename.replace(/\.(ts|mp4)$/i, '') + '.mp4';
        fs.renameSync(converted, this.partPath); // partPath follows the new name
        if (rawPart !== this.partPath) fs.rmSync(rawPart, { force: true });
      } catch (err) {
        fs.rmSync(converted, { force: true });
        if (signal.aborted) throw err;
        // Conversion failed: keep the raw file rather than lose the download.
      } finally {
        s.phase = null;
      }
    }

    const finalPath = settleFinished(this.ctx, s, this.partPath, s.filename);
    s.size = fs.statSync(finalPath).size;
    s.hls.bytes = s.size;
    this._set('completed');
  }
}

module.exports = { HlsDownload, parsePlaylist };
