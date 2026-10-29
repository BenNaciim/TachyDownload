'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn, execFile } = require('node:child_process');
const { Download, FatalError } = require('./download');
const { uniquePath, sanitizeFilename } = require('./util');

/** Pages that yt-dlp handles and that the browser cannot give as a plain file. */
const VIDEO_PAGE = [
  /^https?:\/\/(www\.|m\.|music\.)?youtube\.com\/(watch|shorts\/|live\/)/i,
  /^https?:\/\/youtu\.be\/[\w-]+/i,
  /^https?:\/\/(www\.)?vimeo\.com\/\d+/i,
  /^https?:\/\/(www\.)?dailymotion\.com\/video\//i,
  /^https?:\/\/(www\.)?twitch\.tv\/videos\/\d+/i,
];
const isVideoPage = (url) => VIDEO_PAGE.some((re) => re.test(url));

const isPlaylistUrl = (url) => /^https?:\/\/(www\.|m\.|music\.)?youtube\.com\/playlist\?(.*&)?list=[\w-]+/i.test(url);

/** Lists a playlist without downloading it: [{ url, title }] plus the playlist's own title. */
function listPlaylist(ytdlp, url, limit = 200) {
  return new Promise((resolve, reject) => {
    execFile(
      ytdlp,
      ['--flat-playlist', '--no-warnings', '--playlist-end', String(limit), '--print', '%(playlist_title)s\t%(url)s\t%(title)s', url],
      { windowsHide: true, timeout: 90_000, maxBuffer: 16 * 1024 * 1024, encoding: 'utf8' },
      (err, stdout, stderr) => {
        if (err) return reject(new Error(String(stderr || err.message).split(/\r?\n/).find((l) => l.trim())?.replace(/^ERROR:\s*(\[[^\]]+\]\s*)?/i, '') || 'Playlist illisible'));
        const rows = stdout.split(/\r?\n/).filter(Boolean).map((l) => l.split('\t'));
        const items = rows.filter((r) => /^https?:\/\//.test(r[1] ?? '')).map((r) => ({ url: r[1], title: r[2] || '' }));
        resolve({ title: rows[0]?.[0] && rows[0][0] !== 'NA' ? rows[0][0] : 'Playlist', items });
      },
    );
  });
}

function killTree(pid) {
  return new Promise((resolve) =>
    execFile('taskkill', ['/pid', String(pid), '/T', '/F'], { windowsHide: true }, () => resolve()),
  );
}

/** A download handled by the yt-dlp program: it finds the streams, we show progress and keep the file. */
/** Turns yt-dlp's English messages into something actionable; null when the cause is unknown. */
function explain(msg) {
  const m = String(msg || '');
  if (/not a bot|sign in to confirm/i.test(m)) return 'YouTube demande de prouver que vous n’êtes pas un robot. Réessayez plus tard ou depuis une autre connexion.';
  if (/confirm your age|inappropriate for some users/i.test(m)) return 'Vidéo réservée aux adultes : YouTube exige une connexion.';
  if (/private video|members-only|join this channel/i.test(m)) return 'Vidéo privée ou réservée aux abonnés.';
  if (/video unavailable|this video is unavailable|has been removed|no longer available|does not exist/i.test(m)) return 'Vidéo indisponible ou supprimée.';
  if (/not available in your country|blocked it in your country/i.test(m)) return 'Vidéo non disponible dans votre pays.';
  if (/live event will begin|is live|premieres in/i.test(m)) return 'Direct ou première pas encore terminés : téléchargement impossible.';
  if (/requested format is not available|no video formats|javascript runtime|n challenge/i.test(m)) return `YouTube n’a donné aucun format téléchargeable pour cette vidéo (${m.slice(0, 160)}).`;
  if (/unable to download|getaddrinfo|timed out|network is unreachable|connection/i.test(m)) return `Problème de connexion : ${m.slice(0, 160)}`;
  return null;
}

class YtdlpDownload extends Download {
  downloaded() {
    const y = this.state.ytd;
    return y ? y.base + y.cur : 0;
  }

  _args(ytdlp) {
    const s = this.state;
    const cfg = this.ctx.settings?.() ?? {};
    const ffmpeg = this.ctx.ffmpeg?.();
    const h = Number(cfg.videoMaxHeight) || 1080;
    const args = [
      '--newline', '--no-colors', '--encoding', 'utf-8', '--no-playlist', '--no-mtime', '--no-simulate',
      '--progress-template', 'download:DMP %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s',
      '--print', 'before_dl:DMT %(title)s',
      '--print', 'after_move:DMF %(filepath)s',
      '-P', s.dir,
      '-o', '%(title).150B [%(id)s].%(ext)s',
      '--concurrent-fragments', String(Math.max(1, s.connections)),
    ];
    if (ffmpeg) {
      args.push('--ffmpeg-location', path.dirname(ffmpeg), '--merge-output-format', 'mp4');
      // Prefer H.264 + AAC: plays everywhere on Windows, unlike AV1/Opus in an mp4.
      args.push('-f', 'bv*+ba/b', '-S', `res:${h},vcodec:h264,acodec:m4a`);
    } else {
      args.push('-f', `b[height<=${h}]/b`); // one file with sound: no merge tool needed
    }
    // Without a JS engine YouTube hides the H.264 formats and may hide all of them.
    const deno = this.ctx.deno?.();
    if (deno) args.push('--js-runtimes', `deno:${deno}`);
    if (cfg.speedLimitKBps > 0) args.push('--limit-rate', `${cfg.speedLimitKBps}K`);
    args.push(s.url);
    return args;
  }

  async _run(signal) {
    const ytdlp = this.ctx.ytdlp?.();
    if (!ytdlp) {
      throw new FatalError('Outils vidéo non installés : ouvrez Paramètres → Outils vidéo');
    }
    const s = this.state;
    fs.mkdirSync(s.dir, { recursive: true });
    s.ytd ??= { base: 0, cur: 0, total: 0 };
    s.ytd.cur = 0;
    this.samples = [];

    const proc = spawn(ytdlp, this._args(ytdlp), { windowsHide: true });
    const onAbort = () => killTree(proc.pid);
    signal.addEventListener('abort', onAbort, { once: true });
    const ticker = setInterval(() => this.emit('update'), 250);

    let finalFile = null;
    let lastErr = '';
    const tail = []; // last messages yt-dlp printed, for when it fails without an ERROR line
    let buf = '';
    const onLine = (line) => {
      if (line.startsWith('DMP ')) {
        const [, done, total, est] = line.split(' ');
        const y = s.ytd;
        const cur = Number(done) || 0;
        if (cur < y.cur) y.base += y.total; // next stream (audio after video) restarts at zero
        if (cur > y.cur) this.samples.push([Date.now(), cur - y.cur]);
        y.cur = cur;
        y.total = Number(total) || Number(est) || cur;
        s.size = Math.max(y.base + y.total, this.downloaded());
      } else if (line.startsWith('DMT ')) {
        s.filename = sanitizeFilename(line.slice(4).trim() || 'video') + '.mp4';
      } else if (line.startsWith('DMF ')) {
        finalFile = line.slice(4).trim();
      } else if (/^ERROR:/i.test(line)) {
        lastErr = line.replace(/^ERROR:\s*(\[[^\]]+\]\s*)?/i, '');
      } else if (line && !line.startsWith('[download]')) {
        tail.push(line.replace(/^WARNING:\s*/i, ''));
        if (tail.length > 4) tail.shift();
      }
    };
    proc.stdout.on('data', (d) => {
      buf += d.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        onLine(buf.slice(0, i).trim());
        buf = buf.slice(i + 1);
      }
    });
    proc.stderr.on('data', (d) => d.toString('utf8').split(/\r?\n/).forEach((l) => l && onLine(l.trim())));

    const code = await new Promise((resolve, reject) => {
      proc.once('error', reject);
      proc.once('close', resolve);
    }).finally(() => {
      clearInterval(ticker);
      signal.removeEventListener('abort', onAbort);
    });

    if (signal.aborted) return;
    if (code !== 0 || !finalFile || !fs.existsSync(finalFile)) {
      const detail = lastErr || tail.join(' · ');
      throw new FatalError(explain(detail) || detail || `yt-dlp n’a pas pu télécharger cette vidéo (code ${code})`);
    }
    const name = path.basename(finalFile);
    const finalDir = this.ctx.finalDir?.(name, s.dir) ?? path.dirname(finalFile);
    fs.mkdirSync(finalDir, { recursive: true });
    // yt-dlp already wrote it in place: only a move to another folder needs a free name.
    const inPlace = path.resolve(finalDir, name) === path.resolve(finalFile);
    const target = inPlace ? finalFile : uniquePath(finalDir, name, { ignorePart: true });
    if (!inPlace) fs.renameSync(finalFile, target);
    s.dir = finalDir;
    s.filename = path.basename(target);
    s.size = fs.statSync(target).size;
    s.ytd.base = 0;
    s.ytd.cur = s.size;
    s.ytd.total = 0;
    s.completedAt = Date.now();
    this._set('completed');
  }
}

module.exports = { YtdlpDownload, isVideoPage, isPlaylistUrl, listPlaylist };
