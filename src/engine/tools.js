'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFile } = require('node:child_process');
const { EventEmitter } = require('node:events');

// Official release assets; every download is checked against the published SHA-256 list.
const SOURCES = {
  ytdlp: {
    file: 'yt-dlp.exe',
    url: 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe',
    sums: 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS',
    sumName: 'yt-dlp.exe',
  },
  ffmpeg: {
    file: 'ffmpeg.exe',
    url: 'https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip',
    sums: 'https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/checksums.sha256',
    sumName: 'ffmpeg-master-latest-win64-gpl.zip',
    zip: true,
  },
  // JavaScript engine that yt-dlp needs to solve YouTube's stream-address challenge.
  deno: {
    file: 'deno.exe',
    url: 'https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip',
    sums: 'https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip.sha256sum',
    sumsRe: /Hash\s*:\s*([0-9a-f]{64})/i, // the .sha256sum file is a PowerShell Get-FileHash printout
    zip: true,
  },
};

function onPath(exe) {
  for (const dir of (process.env.PATH || '').split(path.delimiter)) {
    const p = path.join(dir.replace(/^"|"$/g, ''), exe);
    if (dir && fs.existsSync(p)) return p;
  }
  return null;
}

class Tools extends EventEmitter {
  constructor(dir) {
    super();
    this.dir = dir;
    this.installing = null; // { name, received, total } while a download runs
  }

  /** Path of an installed tool: our own copy first, then whatever is on PATH. */
  find(name) {
    const own = path.join(this.dir, SOURCES[name].file);
    if (fs.existsSync(own)) return own;
    return onPath(SOURCES[name].file);
  }

  status() {
    return {
      ytdlp: this.find('ytdlp'),
      ffmpeg: this.find('ffmpeg'),
      deno: this.find('deno'),
      installing: this.installing,
    };
  }

  async _latestYtdlp() {
    const res = await fetch('https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest', {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'tachy-download' },
      signal: AbortSignal.timeout(15000),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return String((await res.json()).tag_name ?? '').trim();
  }

  _installedVersion(file) {
    return new Promise((resolve) =>
      execFile(file, ['--version'], { windowsHide: true, timeout: 15000 }, (err, out) => resolve(err ? '' : String(out).trim())),
    );
  }

  /**
   * YouTube changes often and an old yt-dlp stops working, so our own copy is refreshed from the
   * official release (checksum-verified) at most once per `everyMs`. A copy found on PATH is never touched.
   * Resolves with 'absent' | 'recent' | 'busy' | 'current' | 'updated'.
   */
  async checkYtdlpUpdate({ everyMs = 20 * 3600_000, busy = () => false } = {}) {
    const own = path.join(this.dir, SOURCES.ytdlp.file);
    if (!fs.existsSync(own)) return 'absent';
    const stamp = path.join(this.dir, 'ytdlp.checked');
    try {
      if (Date.now() - fs.statSync(stamp).mtimeMs < everyMs) return 'recent';
    } catch {
      // never checked yet
    }
    if (this.installing || busy()) return 'busy';
    const latest = await this._latestYtdlp();
    if (!latest) return 'current';
    fs.writeFileSync(stamp, latest);
    if ((await this._installedVersion(own)) === latest) return 'current';
    await this.install('ytdlp');
    return 'updated';
  }

  _progress(name, received, total) {
    this.installing = { name, received, total };
    this.emit('update', this.status());
  }

  async install(name) {
    const src = SOURCES[name];
    if (!src) throw new Error('Outil inconnu');
    if (this.installing) throw new Error('Une installation est déjà en cours');
    fs.mkdirSync(this.dir, { recursive: true });
    const tmp = path.join(this.dir, `${src.file}.download`);
    this._progress(name, 0, 0);
    try {
      const sums = await (await fetch(src.sums)).text();
      const line = src.sumsRe ? sums : sums.split(/\r?\n/).find((l) => l.trim().endsWith(src.sumName));
      const expected = (src.sumsRe ?? /^([0-9a-f]{64})/i).exec(line ?? '')?.[1]?.toLowerCase();
      if (!expected) throw new Error('Somme de contrôle introuvable : installation annulée');

      const res = await fetch(src.url, { redirect: 'follow' });
      if (!res.ok) throw new Error(`Téléchargement impossible (HTTP ${res.status})`);
      const total = Number(res.headers.get('content-length')) || 0;
      const hash = crypto.createHash('sha256');
      const out = fs.createWriteStream(tmp);
      let received = 0;
      const reader = res.body.getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        hash.update(value);
        if (!out.write(value)) await new Promise((r) => out.once('drain', r));
        received += value.length;
        this._progress(name, received, total);
      }
      await new Promise((r) => out.end(r));
      if (hash.digest('hex') !== expected) throw new Error('Somme de contrôle invalide : fichier rejeté');

      const target = path.join(this.dir, src.file);
      if (src.zip) {
        const outDir = path.join(this.dir, 'unzip');
        fs.rmSync(outDir, { recursive: true, force: true });
        fs.mkdirSync(outDir, { recursive: true });
        await new Promise((resolve, reject) =>
          // The Windows bsdtar by full path: a GNU tar earlier on PATH (Git for Windows) reads "C:\..." as a remote host.
          execFile(path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe'), ['-xf', tmp, '-C', outDir], { windowsHide: true }, (e) => (e ? reject(e) : resolve())),
        );
        const found = findFile(outDir, src.file);
        if (!found) throw new Error(`${src.file} absent de l’archive`);
        fs.renameSync(found, target);
        fs.rmSync(outDir, { recursive: true, force: true });
        fs.rmSync(tmp, { force: true });
      } else {
        fs.renameSync(tmp, target);
      }
    } finally {
      fs.rmSync(tmp, { force: true });
      this.installing = null;
      this.emit('update', this.status());
    }
  }
}

function findFile(dir, name) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      const r = findFile(p, name);
      if (r) return r;
    } else if (e.name.toLowerCase() === name) return p;
  }
  return null;
}

module.exports = { Tools, SOURCES };
