'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { probe } = require('./probe');
const { sleep, uniquePath, friendlyError } = require('./util');

const MIN_SEGMENT = 1024 * 1024;
const MIN_STEAL = 2 * 1024 * 1024; // only a segment with more than this left is worth splitting
const MAX_RETRIES = 5;
const SPEED_WINDOW_MS = 3000;

class FatalError extends Error {
  constructor(message) {
    super(message);
    this.fatal = true;
  }
}

/** A refused request. 401/403/410 usually mean a signed link has expired. */
function httpError(status) {
  const err = new FatalError(`Le serveur a répondu ${status}`);
  err.expired = status === 401 || status === 403 || status === 410;
  return err;
}

function splitSegments(size, count) {
  const n = Math.max(1, Math.min(count, Math.ceil(size / MIN_SEGMENT)));
  const step = Math.floor(size / n);
  return Array.from({ length: n }, (_, i) => {
    const start = i * step;
    const end = i === n - 1 ? size - 1 : start + step - 1;
    return { start, end, pos: start };
  });
}

class Download extends EventEmitter {
  /**
   * `state` is plain JSON (it is what gets persisted); `limiter` is the shared TokenBucket.
   * status: queued | scheduled | downloading | paused | completed | error
   */
  constructor(state, limiter, ctx = {}) {
    super();
    this.ctx = ctx;
    this.state = state;
    this.limiter = limiter;
    this.abort = null;
    this.running = null;
    this.samples = [];
    this.stopStatus = 'paused';
  }

  get id() {
    return this.state.id;
  }

  get active() {
    return this.running !== null;
  }

  downloaded() {
    const { segments } = this.state;
    if (!segments) return 0;
    return segments.reduce((sum, s) => sum + (s.pos - s.start), 0);
  }

  speed() {
    const cutoff = Date.now() - SPEED_WINDOW_MS;
    this.samples = this.samples.filter(([t]) => t >= cutoff);
    if (this.samples.length === 0) return 0;
    const bytes = this.samples.reduce((sum, [, n]) => sum + n, 0);
    const span = Math.max(1000, Date.now() - this.samples[0][0]);
    return (bytes / span) * 1000;
  }

  snapshot() {
    const downloaded = this.downloaded();
    const speed = this.state.status === 'downloading' ? this.speed() : 0;
    const { size } = this.state;
    return {
      ...this.state,
      segments: this.state.segments?.map((s) => ({ ...s })) ?? null,
      downloaded,
      speed,
      eta: size && speed > 0 ? Math.ceil((size - downloaded) / speed) : null,
    };
  }

  _set(status, error = null) {
    this.state.status = status;
    this.state.error = error;
    this.emit('update');
  }

  _headers() {
    const h = {};
    const { referer, cookie, userAgent } = this.state.headers ?? {};
    if (referer) h.Referer = referer;
    if (cookie) h.Cookie = cookie;
    if (userAgent) h['User-Agent'] = userAgent;
    if (this.state.etag && !this.state.etag.startsWith('W/')) h['If-Range'] = this.state.etag;
    else if (this.state.lastModified) h['If-Range'] = this.state.lastModified;
    return h;
  }

  get partPath() {
    return path.join(this.state.dir, this.state.filename + '.part');
  }

  /** Starts (or resumes) the transfer. Resolves when it finishes, pauses or fails. */
  start() {
    if (this.running) return this.running;
    this.abort = new AbortController();
    this.stopStatus = 'paused';
    this.state.needsRefresh = false;
    this._set('downloading');
    this.running = this._run(this.abort.signal)
      .catch((err) => {
        if (this.abort.signal.aborted) {
          this._set(this.stopStatus);
        } else if (err.expired) {
          // Keeps the partial file: a fresh link for the same file resumes where it stopped.
          this.state.needsRefresh = true;
          this._set('error', `${friendlyError(err)} : le lien a peut-être expiré`);
        } else {
          this._set('error', friendlyError(err));
        }
      })
      .finally(() => {
        this.running = null;
        this.emit('settled');
      });
    return this.running;
  }

  /** Stops the transfer but keeps the partial file; `next` is the status left behind. */
  stop(next = 'paused') {
    if (!this.running) {
      if (this.state.status !== 'completed') this._set(next);
      return Promise.resolve();
    }
    this.stopStatus = next;
    this.abort.abort();
    return this.running;
  }

  async _prepare(signal) {
    const s = this.state;
    if (s.size === undefined || s.size === null || !s.finalUrl) {
      const info = await probe(s.url, this._baseHeaders(), signal);
      s.finalUrl = info.finalUrl;
      s.size = info.size;
      s.rangeOk = info.rangeOk;
      s.etag = info.etag;
      s.lastModified = info.lastModified;
      if (!s.filenameLocked) {
        s.filename = path.basename(uniquePath(s.dir, info.filename));
      }
    }
    fs.mkdirSync(s.dir, { recursive: true });

    const partExists = fs.existsSync(this.partPath);
    const resumable =
      s.segments && partExists && (!s.rangeOk || s.size === null
        ? false
        : fs.statSync(this.partPath).size === s.size);
    if (!resumable) {
      s.segments = s.rangeOk && s.size
        ? splitSegments(s.size, s.connections)
        : [{ start: 0, end: s.size ? s.size - 1 : Number.MAX_SAFE_INTEGER, pos: 0 }];
      this.samples = [];
      fs.writeFileSync(this.partPath, '');
      if (s.size && s.rangeOk) fs.truncateSync(this.partPath, s.size);
    }
  }

  _baseHeaders() {
    const { referer, cookie, userAgent } = this.state.headers ?? {};
    const h = {};
    if (referer) h.Referer = referer;
    if (cookie) h.Cookie = cookie;
    if (userAgent) h['User-Agent'] = userAgent;
    return h;
  }

  async _run(signal) {
    await this._prepare(signal);
    const s = this.state;
    const fh = await fs.promises.open(this.partPath, 'r+');
    const ticker = setInterval(() => this.emit('update'), 250);
    try {
      const pending = s.segments.filter((seg) => seg.pos <= seg.end);
      // A connection that finishes its segment takes over half of the slowest one still running,
      // so every connection stays busy until the very end.
      const running = new Set();
      const runOne = async (seg) => {
        running.add(seg);
        try {
          await (s.rangeOk ? this._runRangeSegment(seg, fh, signal) : this._runStream(seg, fh, signal));
        } finally {
          running.delete(seg);
        }
        const next = s.rangeOk && !signal.aborted ? this._steal(running) : null;
        return next ? runOne(next) : undefined;
      };
      try {
        await Promise.all(pending.map(runOne));
      } catch (err) {
        if (err.changed && !signal.aborted) {
          // The remote file changed under us: the partial data is useless.
          s.segments = null;
          s.size = null;
          s.finalUrl = null;
          s.etag = null;
          s.lastModified = null;
          await fh.close();
          return this._run(signal);
        }
        throw err;
      }
      if (s.size && this.downloaded() !== s.size) {
        throw new Error('Taille finale inattendue');
      }
      await fh.sync();
    } finally {
      clearInterval(ticker);
      await fh.close().catch(() => {});
    }
    if (signal.aborted) return;
    // Our own .part file reserves the name, so only a finished file can collide here.
    const finalDir = this.ctx.finalDir?.(s.filename, s.dir) ?? s.dir;
    fs.mkdirSync(finalDir, { recursive: true });
    const finalPath = uniquePath(finalDir, s.filename, { ignorePart: true });
    fs.renameSync(this.partPath, finalPath);
    s.dir = finalDir;
    s.filename = path.basename(finalPath);
    s.completedAt = Date.now();
    this._set('completed');
  }

  /**
   * Splits the running segment with the most bytes left: it keeps the first half and a new segment
   * takes the second. Done synchronously, so the victim's position cannot move in between.
   */
  _steal(running) {
    let victim = null;
    for (const seg of running) {
      const left = seg.end - seg.pos + 1;
      if (left >= MIN_STEAL && (!victim || left > victim.end - victim.pos + 1)) victim = seg;
    }
    if (!victim) return null;
    const mid = victim.pos + Math.floor((victim.end - victim.pos + 1) / 2);
    const fresh = { start: mid, end: victim.end, pos: mid };
    victim.end = mid - 1;
    const segs = this.state.segments;
    segs.splice(segs.indexOf(victim) + 1, 0, fresh);
    this.emit('update');
    return fresh;
  }

  async _write(fh, chunk, position) {
    let off = 0;
    while (off < chunk.length) {
      const { bytesWritten } = await fh.write(chunk, off, chunk.length - off, position + off);
      off += bytesWritten;
    }
  }

  async _runRangeSegment(seg, fh, signal) {
    const s = this.state;
    let attempt = 0;
    while (seg.pos <= seg.end) {
      const before = seg.pos;
      try {
        const res = await fetch(s.finalUrl, {
          headers: { ...this._headers(), Range: `bytes=${seg.pos}-${seg.end}` },
          redirect: 'follow',
          signal,
        });
        if (res.status === 200) {
          await res.body?.cancel().catch(() => {});
          const err = new FatalError('Le fichier distant a changé');
          err.changed = true;
          throw err;
        }
        if (res.status !== 206) {
          await res.body?.cancel().catch(() => {});
          if (res.status >= 500 || res.status === 429) throw new Error(`HTTP ${res.status}`);
          throw httpError(res.status);
        }
        const reader = res.body.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          // seg.end can shrink while we read: another connection took the tail of this segment.
          const room = seg.end - seg.pos + 1;
          if (room <= 0) {
            await reader.cancel().catch(() => {});
            break;
          }
          const chunk = value.length > room ? value.subarray(0, room) : value;
          await this.limiter.take(chunk.length, signal);
          const at = seg.pos;
          await this._write(fh, chunk, at);
          const kept = Math.max(0, Math.min(chunk.length, seg.end + 1 - at)); // end may have moved during the write
          seg.pos = at + kept;
          this.samples.push([Date.now(), kept]);
          if (seg.pos > seg.end) {
            await reader.cancel().catch(() => {});
            break;
          }
        }
        if (seg.pos <= seg.end) throw new Error('Flux interrompu');
      } catch (err) {
        if (signal.aborted || err.fatal) throw err;
        if (seg.pos > before) attempt = 0;
        if (++attempt > MAX_RETRIES) throw err;
        await sleep(Math.min(8000, 500 * 2 ** attempt), signal);
      }
    }
  }

  /** Server without range support: one sequential stream, restarted from zero on failure. */
  async _runStream(seg, fh, signal) {
    const s = this.state;
    let attempt = 0;
    for (;;) {
      seg.pos = 0;
      this.samples = [];
      try {
        const res = await fetch(s.finalUrl, {
          headers: this._baseHeaders(),
          redirect: 'follow',
          signal,
        });
        if (res.status >= 400) {
          await res.body?.cancel().catch(() => {});
          if (res.status >= 500 || res.status === 429) throw new Error(`HTTP ${res.status}`);
          throw httpError(res.status);
        }
        const reader = res.body.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          await this.limiter.take(value.length, signal);
          await this._write(fh, value, seg.pos);
          seg.pos += value.length;
          this.samples.push([Date.now(), value.length]);
        }
        if (!s.size) {
          s.size = seg.pos;
          seg.end = seg.pos - 1;
        } else if (seg.pos !== s.size) {
          throw new Error('Flux interrompu');
        }
        return;
      } catch (err) {
        if (signal.aborted || err.fatal) throw err;
        if (++attempt > MAX_RETRIES) throw err;
        await fh.truncate(0);
        await sleep(Math.min(8000, 500 * 2 ** attempt), signal);
      }
    }
  }
}

module.exports = { Download, splitSegments, FatalError, httpError };
