'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn, execFile } = require('node:child_process');
const { Download, FatalError } = require('./download');
const { sanitizeFilename, settleFinished } = require('./util');

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

/** What the user may choose for one video; anything else is dropped. */
function cleanYtOpts(o) {
  if (!o || typeof o !== 'object') return null;
  const out = {};
  if (o.audioOnly === true) out.audioOnly = true;
  const h = Number(o.height);
  if ([144, 240, 360, 480, 720, 1080, 1440, 2160, 4320].includes(h)) out.height = h;
  if (typeof o.lang === 'string' && /^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})?$/.test(o.lang)) out.lang = o.lang;
  return Object.keys(out).length ? out : null;
}

/** The yt-dlp format arguments for the chosen options (pure, so it can be tested without yt-dlp). */
function formatArgs({ opts, maxHeight, ffmpeg }) {
  const o = opts ?? {};
  const lang = o.lang ? `lang:${o.lang},` : '';
  if (o.audioOnly) {
    // Converting to mp3 needs ffmpeg; without it the original audio (usually .m4a) is kept.
    return ffmpeg
      ? ['-f', 'ba/b', '-x', '--audio-format', 'mp3', '--audio-quality', '0', ...(lang ? ['-S', lang.slice(0, -1)] : [])]
      : ['-f', 'ba[ext=m4a]/ba/b', ...(lang ? ['-S', lang.slice(0, -1)] : [])];
  }
  const h = o.height || maxHeight;
  if (ffmpeg) {
    // Prefer H.264 + AAC: plays everywhere on Windows, unlike AV1/Opus in an mp4.
    return ['--merge-output-format', 'mp4', '-f', 'bv*+ba/b', '-S', `${lang}res:${h},vcodec:h264,acodec:m4a`];
  }
  return ['-f', `b[height<=${h}]/b`]; // one file with sound: no merge tool needed
}

/** Looks at a video without downloading it: title, available heights and audio languages. */
function probeVideo(ytdlp, url, { deno, cookies } = {}) {
  return new Promise((resolve, reject) => {
    const args = ['-J', '--no-playlist', '--no-warnings', '--encoding', 'utf-8'];
    if (deno) args.push('--js-runtimes', `deno:${deno}`);
    if (cookies) args.push('--cookies-from-browser', cookies);
    args.push(url);
    execFile(ytdlp, args, { windowsHide: true, timeout: 60_000, maxBuffer: 64 * 1024 * 1024, encoding: 'utf8' }, (err, stdout, stderr) => {
      if (err) {
        const line = String(stderr || err.message).split(/\r?\n/).find((l) => /ERROR/i.test(l)) || err.message;
        return reject(new Error(explain(line) || line.replace(/^ERROR:\s*(\[[^\]]+\]\s*)?/i, '').slice(0, 200)));
      }
      try {
        resolve(summarizeProbe(JSON.parse(stdout)));
      } catch {
        reject(new Error('Réponse de yt-dlp illisible'));
      }
    });
  });
}

function summarizeProbe(info) {
  const formats = Array.isArray(info.formats) ? info.formats : [];
  const heights = [...new Set(formats.filter((f) => f.vcodec && f.vcodec !== 'none' && f.height).map((f) => f.height))].sort((a, b) => b - a);
  const langs = new Map();
  for (const f of formats) {
    if (!f.acodec || f.acodec === 'none' || !f.language) continue;
    const code = String(f.language);
    const label = String(f.format_note || '').replace(/,.*$/, '').replace(/\s*\(default\)/i, '').trim();
    if (!langs.has(code) || (!langs.get(code) && label)) langs.set(code, label);
  }
  return {
    title: String(info.title || ''),
    duration: Number(info.duration) || 0,
    heights,
    langs: [...langs].map(([code, label]) => ({ code, label: label || code })),
  };
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
  if (/not a bot|sign in to confirm/i.test(m)) return 'YouTube demande de prouver que vous n’êtes pas un robot. Dans Paramètres → Vidéos, choisissez « Cookies du navigateur » (un navigateur où vous êtes connecté à YouTube), puis réessayez.';
  if (/confirm your age|inappropriate for some users/i.test(m)) return 'Vidéo réservée aux adultes : YouTube exige une connexion.';
  if (/private video|members-only|join this channel/i.test(m)) return 'Vidéo privée ou réservée aux abonnés.';
  if (/video unavailable|this video is unavailable|has been removed|no longer available|does not exist/i.test(m)) return 'Vidéo indisponible ou supprimée.';
  if (/not available in your country|blocked it in your country/i.test(m)) return 'Vidéo non disponible dans votre pays.';
  if (/live event will begin|is live|premieres in/i.test(m)) return 'Direct ou première pas encore terminés : téléchargement impossible.';
  if (/requested format is not available|no video formats|javascript runtime|n challenge/i.test(m)) return `YouTube n’a donné aucun format téléchargeable pour cette vidéo (${m.slice(0, 160)}).`;
  if (/unable to download|getaddrinfo|timed out|network is unreachable|connection/i.test(m)) return `Problème de connexion : ${m.slice(0, 160)}`;
  return null;
}

/**
 * Folds one "DMP done total estimate fragIndex fragCount" line of yt-dlp into the task state (`ytd`, `frag`,
 * `size`) and returns the bytes gained since the previous line, for the speed. `first` marks the first line
 * of a run: it counts what an earlier run left on disk, which is not speed.
 */
function applyProgress(s, line, first = false) {
  const [, done, total, est, fragIndex, fragCount] = line.split(' ');
  const y = s.ytd;
  const cur = Number(done) || 0;
  // Next stream (audio after video) restarts near zero: keep what the finished one really weighed.
  // Parallel fragments make the figure wobble by a few KB, so only a real fall counts as a new stream.
  if (cur < y.cur * 0.5) y.base += y.cur;
  const gained = cur > y.cur && !first ? cur - y.cur : 0;
  y.cur = cur;
  const fi = Number(fragIndex);
  const fc = Number(fragCount);
  // Fragment tally for the details pane: the index wobbles with parallel fragments, so it only grows within a stream.
  s.frag = fc > 0 ? { done: Math.min(fc, Math.max(fi || 0, s.frag?.total === fc ? s.frag.done : 0)), total: fc } : (s.frag ?? null);
  // Fragmented streams (most of YouTube) give no byte total: extrapolate from the fragments done. With
  // parallel fragments the index lags far behind the bytes at the start, so wait for a few fragments
  // before trusting the ratio (otherwise the total explodes to terabytes).
  const byFragments = fi >= Math.max(5, fc * 0.03) && fc >= fi ? Math.round((cur * fc) / fi) : 0;
  y.total = Number(total) || Number(est) || byFragments;
  const downloaded = y.base + y.cur;
  s.size = y.total ? Math.max(y.base + y.total, downloaded) : 0; // unknown until a trustworthy estimate exists
  return gained;
}

/** The newest finished file in `dir` written since `since` whose name holds "[id].", or null. */
function findByVideoId(dir, id, since) {
  if (!id) return null;
  let best = null;
  try {
    for (const f of fs.readdirSync(dir)) {
      if (!f.includes(`[${id}].`) || /\.(part|ytdl|temp)$|\.part-/i.test(f)) continue;
      const full = path.join(dir, f);
      const t = fs.statSync(full).mtimeMs;
      if (t < since) continue;
      if (!best || t > best.t) best = { full, t };
    }
  } catch {}
  return best?.full ?? null;
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
    const maxHeight = Number(cfg.videoMaxHeight) || 1080;
    const opts = s.ytOpts ?? {};
    const args = [
      '--newline', '--progress', '--no-colors', // --print implies --quiet, which would silence the progress lines '--encoding', 'utf-8', '--no-playlist', '--no-mtime', '--no-simulate',
      '--progress-template', 'download:DMP %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.fragment_index)s %(progress.fragment_count)s',
      '--print', 'before_dl:DMT %(title)s',
      '--print', 'after_move:DMF %(filepath)s',
      '-P', s.dir,
      '-o', '%(title).150B [%(id)s].%(ext)s',
      '--concurrent-fragments', String(Math.max(1, s.connections)),
    ];
    if (ffmpeg) args.push('--ffmpeg-location', path.dirname(ffmpeg));
    args.push(...formatArgs({ opts, maxHeight, ffmpeg }));
    // Without a JS engine YouTube hides the H.264 formats and may hide all of them.
    const deno = this.ctx.deno?.();
    if (deno) args.push('--js-runtimes', `deno:${deno}`);
    if (cfg.ytCookies) args.push('--cookies-from-browser', cfg.ytCookies); // a signed-in browser passes YouTube's bot check
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
    // yt-dlp counts what an earlier run already left on disk, so every run starts its own tally from zero
    // (a base kept from a paused run would be added on top and inflate the size).
    s.ytd = { base: 0, cur: 0, total: 0 };
    s.frag = null;
    let firstReport = true;
    const startedAt = Date.now();
    this.samples = [];

    // Without these, yt-dlp prints non-ASCII titles through the Windows code page and mangles them (accents, full-width colons).
    const env = { ...process.env, PYTHONUTF8: '1', PYTHONIOENCODING: 'utf-8' };
    const proc = spawn(ytdlp, this._args(ytdlp), { windowsHide: true, env });
    const onAbort = () => killTree(proc.pid);
    signal.addEventListener('abort', onAbort, { once: true });
    const ticker = setInterval(() => this.emit('update'), 250);

    let finalFile = null;
    let lastErr = '';
    const tail = []; // last messages yt-dlp printed, for when it fails without an ERROR line
    let buf = '';
    const onLine = (line) => {
      if (line.startsWith('DMP ')) {
        const gained = applyProgress(s, line, firstReport);
        firstReport = false;
        if (gained > 0) this.samples.push([Date.now(), gained]);
      } else if (line.startsWith('DMT ')) {
        s.filename = sanitizeFilename(line.slice(4).trim() || 'video') + (s.ytOpts?.audioOnly ? (this.ctx.ffmpeg?.() ? '.mp3' : '.m4a') : '.mp4');
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
    // The printed path can still differ from the real one (characters lost on the way): find the file by the video id in its name.
    if (code === 0 && !(finalFile && fs.existsSync(finalFile))) {
      const id = /\[([\w-]{11})\]\.[A-Za-z0-9]+$/.exec(finalFile ?? '')?.[1] ?? /[?&]v=([\w-]{11})|youtu\.be\/([\w-]{11})/.exec(s.url)?.slice(1).find(Boolean);
      finalFile = findByVideoId(finalFile ? path.dirname(finalFile) : s.dir, id, startedAt - 5000) ?? finalFile;
    }
    if (code !== 0 || !finalFile || !fs.existsSync(finalFile)) {
      const detail = lastErr || tail.join(' · ');
      throw new FatalError(explain(detail) || detail || `yt-dlp n’a pas pu télécharger cette vidéo (code ${code})`);
    }
    // yt-dlp already wrote it in place: only a move to another folder needs a free name.
    const target = settleFinished(this.ctx, s, finalFile, path.basename(finalFile));
    s.size = fs.statSync(target).size;
    s.ytd.base = 0;
    s.ytd.cur = s.size;
    s.ytd.total = 0;
    s.frag = null;
    this._set('completed');
  }
}

module.exports = { YtdlpDownload, applyProgress, isVideoPage, isPlaylistUrl, listPlaylist, probeVideo, summarizeProbe, formatArgs, cleanYtOpts };
