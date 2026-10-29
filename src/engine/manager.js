'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');
const { Download } = require('./download');
const { HlsDownload } = require('./hls');
const { YtdlpDownload, isVideoPage, isPlaylistUrl, listPlaylist } = require('./ytdlp');
const { TokenBucket } = require('./tokenBucket');
const { sanitizeFilename, filenameFromUrl, categoryFor } = require('./util');

const DEFAULT_SETTINGS = {
  dir: '',
  connections: 8,
  maxConcurrent: 3,
  speedLimitKBps: 0,
  videoMaxHeight: 1080,
  convertToMp4: true,
  categories: false,
  autoStart: false,
  closeToTray: true,
  notifications: true,
  confirmDownloads: true,
  afterAll: 'none',
  schedule: { enabled: false, from: '01:00', to: '07:00' },
};

const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, Math.round(Number(n)) || lo));

function toMinutes(hhmm) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm ?? '');
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

/** True when `date` falls inside the daily window; windows may cross midnight. */
function inWindow(schedule, date = new Date()) {
  if (!schedule?.enabled) return true;
  const from = toMinutes(schedule.from);
  const to = toMinutes(schedule.to);
  if (from === null || to === null || from === to) return true;
  const now = date.getHours() * 60 + date.getMinutes();
  return from < to ? now >= from && now < to : now >= from || now < to;
}

function validateUrl(url) {
  let u;
  try {
    u = new URL(url);
  } catch {
    throw new Error('Adresse invalide');
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') {
    throw new Error('Seuls les liens http et https sont pris en charge');
  }
  return u.toString();
}

class DownloadManager extends EventEmitter {
  constructor({ storePath, settings = {}, tickMs = 1000, tools = null } = {}) {
    super();
    this.tools = tools ?? { find: () => null };
    this.storePath = storePath;
    this.settings = this._normalize({ ...DEFAULT_SETTINGS, ...settings });
    this.limiter = new TokenBucket(this.settings.speedLimitKBps * 1024);
    this.tasks = new Map();
    this.saveTimer = null;
    this.tickMs = tickMs;
    this.timer = null;
  }

  _normalize(s) {
    return {
      dir: String(s.dir || ''),
      connections: clamp(s.connections, 1, 32),
      maxConcurrent: clamp(s.maxConcurrent, 1, 10),
      speedLimitKBps: Math.max(0, Math.round(Number(s.speedLimitKBps) || 0)),
      videoMaxHeight: [360, 480, 720, 1080, 1440, 2160].includes(Number(s.videoMaxHeight)) ? Number(s.videoMaxHeight) : 1080,
      convertToMp4: s.convertToMp4 !== false,
      categories: Boolean(s.categories),
      autoStart: Boolean(s.autoStart),
      closeToTray: s.closeToTray !== false,
      notifications: s.notifications !== false,
      confirmDownloads: s.confirmDownloads !== false,
      afterAll: ['none', 'quit', 'sleep', 'shutdown'].includes(s.afterAll) ? s.afterAll : 'none',
      schedule: {
        enabled: Boolean(s.schedule?.enabled),
        from: toMinutes(s.schedule?.from) === null ? '01:00' : s.schedule.from,
        to: toMinutes(s.schedule?.to) === null ? '07:00' : s.schedule.to,
      },
    };
  }

  load() {
    if (!this.storePath || !fs.existsSync(this.storePath)) return;
    let data;
    try {
      data = JSON.parse(fs.readFileSync(this.storePath, 'utf8'));
    } catch {
      return;
    }
    if (data.settings) {
      this.settings = this._normalize({ ...this.settings, ...data.settings });
      this.limiter.setRate(this.settings.speedLimitKBps * 1024);
    }
    for (const state of data.tasks ?? []) {
      // Anything that was in flight when the app closed waits for the user.
      if (state.status === 'downloading') state.status = 'paused';
      this._register(state);
    }
  }

  start() {
    this.timer = setInterval(() => this.tick(), this.tickMs);
    this.timer.unref?.();
    this.tick();
  }

  async shutdown() {
    clearInterval(this.timer);
    await Promise.all([...this.tasks.values()].map((t) => t.stop(t.state.status === 'downloading' ? 'paused' : t.state.status)));
    this.saveNow();
  }

  _register(state) {
    const ctx = {
      ytdlp: () => this.tools.find('ytdlp'),
      ffmpeg: () => this.tools.find('ffmpeg'),
      deno: () => this.tools.find('deno'),
      settings: () => this.settings,
      finalDir: (name, dir) => {
        const folder = this.settings.categories && !state.ownFolder ? categoryFor(name) : null;
        return folder ? path.join(dir, folder) : dir;
      },
    };
    const Kind = { hls: HlsDownload, ytdlp: YtdlpDownload }[state.kind] ?? Download;
    const task = new Kind(state, this.limiter, ctx);
    task.on('update', () => {
      this.emit('update', task.snapshot());
      this._scheduleSave();
    });
    task.on('settled', () => {
      this.pump();
    });
    this.tasks.set(state.id, task);
    return task;
  }

  list() {
    return [...this.tasks.values()].map((t) => t.snapshot());
  }

  add(url, opts = {}) {
    const cleanUrl = validateUrl(url);
    const kind =
      opts.kind === 'ytdlp' || (!opts.kind && isVideoPage(cleanUrl))
        ? 'ytdlp'
        : opts.kind === 'hls' || /\.m3u8?$/i.test(new URL(cleanUrl).pathname)
          ? 'hls'
          : 'file';
    const isHls = kind === 'hls';
    const state = {
      id: crypto.randomUUID(),
      kind,
      ownFolder: Boolean(opts.ownFolder),
      pageUrl: /^https?:\/\//i.test(opts.referer || '') ? opts.referer : '',
      autoRefresh: Boolean(opts.sniffed), // a video the extension found: it can reopen the page itself
      needsRefresh: false,
      url: cleanUrl,
      finalUrl: null,
      dir: opts.dir || this.settings.dir,
      // Placeholder until the first request; the server's own name replaces it unless one was given.
      filename: sanitizeFilename(opts.filename || (kind === 'ytdlp' ? 'Vidéo' : filenameFromUrl(cleanUrl)) || (isHls ? 'video' : 'download')),
      filenameLocked: Boolean(opts.filename),
      size: null,
      rangeOk: false,
      segments: null,
      connections: this.settings.connections,
      etag: null,
      lastModified: null,
      headers: {
        referer: opts.referer || '',
        cookie: opts.cookie || '',
        userAgent: opts.userAgent || '',
      },
      status: 'queued',
      error: null,
      createdAt: Date.now(),
      completedAt: null,
      startAt: opts.startAt ? Number(opts.startAt) : null,
    };
    if (!state.dir) throw new Error('Aucun dossier de destination configuré');
    state.filename = this._freeName(state.dir, state.filename, isHls);
    if (state.startAt && state.startAt > Date.now()) state.status = 'scheduled';
    else if (opts.paused) state.status = 'paused'; // "later": waits in the list until the user resumes it
    const task = this._register(state);
    this.emit('update', task.snapshot());
    this._scheduleSave();
    this.pump();
    return task.snapshot();
  }

  /** Adds every video of a YouTube playlist, in its own folder named after the playlist. */
  async addPlaylist(url, opts = {}) {
    const ytdlp = this.tools.find('ytdlp');
    if (!ytdlp) throw new Error('Outils vidéo non installés : ouvrez Paramètres → Vidéos');
    const { title, items } = await listPlaylist(ytdlp, validateUrl(url));
    if (items.length === 0) throw new Error('Cette playlist est vide ou privée');
    const dir = path.join(opts.dir || this.settings.dir, sanitizeFilename(title));
    return items.map((it) =>
      this.add(it.url, { ...opts, kind: 'ytdlp', dir, filename: it.title ? `${it.title}.mp4` : undefined, ownFolder: true }),
    );
  }

  /** Two unfinished downloads must never share a .part file, so a repeated name gets a suffix. */
  _freeName(dir, filename, isHls) {
    // HLS names change extension (.ts/.mp4) later, so compare without it.
    const stem = (n) => (isHls ? n.replace(/\.(m3u8?|ts|mp4)$/i, '') : n).toLowerCase();
    const taken = new Set(
      [...this.tasks.values()]
        .filter((t) => t.state.status !== 'completed' && t.state.dir === dir)
        .map((t) => stem(t.state.filename)),
    );
    if (!taken.has(stem(filename))) return filename;
    const ext = path.extname(filename);
    const base = filename.slice(0, filename.length - ext.length);
    for (let i = 1; ; i++) {
      const candidate = `${base} (${i})${ext}`;
      if (!taken.has(stem(candidate))) return candidate;
    }
  }

  /** Downloads stopped by an expired link, for the extension to find a fresh one. */
  refreshRequests() {
    return [...this.tasks.values()]
      .filter((t) => t.state.needsRefresh && t.state.status === 'error')
      .map((t) => ({
        id: t.state.id,
        filename: t.state.filename,
        size: t.state.size,
        kind: t.state.kind,
        pageUrl: t.state.pageUrl,
        auto: t.state.autoRefresh,
      }));
  }

  /** Swaps in a fresh link for the same file; what is already on disk is kept and the transfer resumes. */
  refresh(id, opts = {}) {
    const t = this._get(id);
    const s = t.state;
    if (t.active || s.status === 'completed') throw new Error('Ce téléchargement n’a pas besoin d’un nouveau lien');
    s.url = validateUrl(opts.url);
    s.finalUrl = null; // forces a new probe on the new address
    s.etag = null;
    s.lastModified = null;
    s.filenameLocked = true; // the .part file already carries its final name
    for (const [k, v] of [['referer', opts.referer], ['cookie', opts.cookie], ['userAgent', opts.userAgent]]) {
      if (v) s.headers[k] = v;
    }
    s.needsRefresh = false;
    s.error = null;
    s.status = 'queued';
    this.emit('update', t.snapshot());
    this._scheduleSave();
    this.pump();
    return t.snapshot();
  }

  _get(id) {
    const t = this.tasks.get(id);
    if (!t) throw new Error('Téléchargement introuvable');
    return t;
  }

  pause(id) {
    const t = this._get(id);
    if (t.state.status === 'completed') return;
    return t.stop('paused');
  }

  resume(id) {
    const t = this._get(id);
    if (['paused', 'error', 'scheduled'].includes(t.state.status)) {
      t.state.status = 'queued';
      t.state.error = null;
      t.state.startAt = null;
      this.emit('update', t.snapshot());
      this._scheduleSave();
      this.pump();
    }
  }

  async remove(id, { deleteFile = false } = {}) {
    const t = this._get(id);
    await t.stop('paused');
    this.tasks.delete(id);
    if (deleteFile) {
      const target =
        t.state.status === 'completed' ? path.join(t.state.dir, t.state.filename) : t.partPath;
      fs.rmSync(target, { force: true });
      fs.rmSync(t.partPath, { force: true });
    }
    this.emit('removed', id);
    this._scheduleSave();
    this.pump();
  }

  setSettings(patch) {
    const next = this._normalize({ ...this.settings, ...patch });
    this.settings = next;
    this.limiter.setRate(next.speedLimitKBps * 1024);
    this._scheduleSave();
    this.emit('settings', this.settings);
    this.tick();
    return this.settings;
  }

  tick() {
    const now = Date.now();
    for (const t of this.tasks.values()) {
      const s = t.state;
      if (s.status === 'scheduled' && s.startAt && s.startAt <= now) {
        s.status = 'queued';
        this.emit('update', t.snapshot());
      }
    }
    if (!inWindow(this.settings.schedule)) {
      for (const t of this.tasks.values()) {
        if (t.active) t.stop('queued');
      }
    }
    this.pump();
  }

  pump() {
    if (!inWindow(this.settings.schedule)) return;
    let active = [...this.tasks.values()].filter((t) => t.active).length;
    for (const t of this.tasks.values()) {
      if (active >= this.settings.maxConcurrent) break;
      if (t.state.status === 'queued' && !t.active) {
        t.start();
        active++;
      }
    }
  }

  _scheduleSave() {
    if (!this.storePath || this.saveTimer) return;
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      this.saveNow();
    }, 1000);
    this.saveTimer.unref?.();
  }

  saveNow() {
    if (!this.storePath) return;
    clearTimeout(this.saveTimer);
    this.saveTimer = null;
    const payload = {
      settings: this.settings,
      tasks: [...this.tasks.values()].map((t) => t.state),
    };
    const tmp = this.storePath + '.tmp';
    fs.mkdirSync(path.dirname(this.storePath), { recursive: true });
    fs.writeFileSync(tmp, JSON.stringify(payload));
    fs.renameSync(tmp, this.storePath);
  }
}

module.exports = { DownloadManager, inWindow, DEFAULT_SETTINGS };
