'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { formatDiag, explain } = require('../src/engine/ytdlp');
const { tmpDir, waitFor } = require('./helpers');

const URL = 'https://www.youtube.com/watch?v=abcdefghijk';
const FAKE = path.join(__dirname, 'fake-ytdlp.js');

/** Runs one video through the real engine with the stand-in yt-dlp and returns what happened. */
async function run(mode, settings = {}, taskLimit = 0, add = {}) {
  const dir = tmpDir();
  const dataDir = tmpDir();
  add.before?.(dir);
  const argsFile = path.join(dataDir, 'args.json');
  process.env.FAKE_MODE = mode;
  process.env.FAKE_ARGS_FILE = argsFile;
  const tools = { find: (n) => (n === 'ytdlp' ? process.execPath : null), ytdlpPrefix: [FAKE] };
  const m = new DownloadManager({ storePath: path.join(dataDir, 'downloads.json'), settings: { dir, ...settings }, tickMs: 50, tools });
  m.start();
  try {
    const { id } = m.add(add.url ?? URL, { kind: 'ytdlp', paused: Boolean(taskLimit), ...add.opts });
    if (taskLimit) {
      m.setTaskLimit(id, taskLimit);
      m.resume(id);
    }
    await waitFor(() => ['completed', 'error'].includes(m.list()[0].status), 20000);
    const diag = path.join(dataDir, 'ytdlp-last.log');
    return {
      task: m.list()[0],
      dir,
      args: JSON.parse(fs.readFileSync(argsFile, 'utf8')),
      diag: fs.existsSync(diag) ? fs.readFileSync(diag, 'utf8') : null,
    };
  } finally {
    await m.shutdown();
    delete process.env.FAKE_MODE;
    delete process.env.FAKE_ARGS_FILE;
  }
}

test('a normal run completes, keeps the file and always asks for a single video', async () => {
  const r = await run('ok');
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.equal(r.task.filename, 'Episode [abcdefghijk].mp4');
  assert.ok(fs.existsSync(path.join(r.dir, r.task.filename)));
  assert.equal(r.task.size, 2 * 1024 * 1024);
  for (const flag of ['--no-playlist', '--no-simulate', '--progress', '--encoding']) {
    assert.ok(r.args.includes(flag), `${flag} is passed to yt-dlp`);
  }
  assert.match(r.diag, /Code de sortie : 0/, 'a diagnostic is written even on success');
});

const MANIFEST = { url: 'https://cdn.example/v/manifest.mpd', opts: { sniffed: true, filename: 'Série - S01E04', referer: 'https://player.example/e/1', origin: 'https://player.example' } };

test('a manifest found by the extension is named after its page, and gets the headers the browser sent', async () => {
  const r = await run('ok', {}, 0, MANIFEST);
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.equal(r.task.filename, 'Série - S01E04.mp4');
  assert.ok(fs.existsSync(path.join(r.dir, 'Série - S01E04.mp4')));
  assert.equal(r.args[r.args.indexOf('-o') + 1], 'Série - S01E04.%(ext)s');
  assert.equal(r.args[r.args.indexOf('--referer') + 1], 'https://player.example/e/1');
  assert.ok(r.args.includes('Origin:https://player.example'));
});

test('a manifest named after its page never overwrites a file that is already there', async () => {
  const r = await run('ok', {}, 0, { ...MANIFEST, before: (dir) => fs.writeFileSync(path.join(dir, 'Série - S01E04.mp4'), 'older') });
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.equal(r.task.filename, 'Série - S01E04 (1).mp4');
  assert.equal(fs.readFileSync(path.join(r.dir, 'Série - S01E04.mp4'), 'utf8'), 'older');
});

test('a "%" in the page title is not read as a yt-dlp template field', async () => {
  const r = await run('ok', {}, 0, { ...MANIFEST, opts: { ...MANIFEST.opts, filename: '100% vrai' } });
  assert.equal(r.args[r.args.indexOf('-o') + 1], '100%% vrai.%(ext)s');
  assert.equal(r.task.filename, '100% vrai.mp4');
});

test('a printed path that lost a character is found again by the video id', async () => {
  const r = await run('renamed');
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.equal(r.task.filename, 'Episode [abcdefghijk].mp4');
});

test('exit code 0 without any file fails with a diagnostic instead of a silent success', async () => {
  const r = await run('nofile');
  assert.equal(r.task.status, 'error');
  assert.match(r.task.error, /code 0|n’a pas pu/);
  assert.match(r.diag, /introuvable/);
  assert.match(r.diag, /ghost\.mp4/);
});

test('a known YouTube refusal is explained and the diagnostic keeps the raw message', async () => {
  const r = await run('fail');
  assert.equal(r.task.status, 'error');
  assert.match(r.task.error, /robot/);
  assert.match(r.diag, /Sign in to confirm/);
  assert.match(r.diag, /Code de sortie : 1/);
});

test('with type folders on, the video lands in its folder', async () => {
  const r = await run('ok', { categories: true });
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.ok(fs.existsSync(path.join(r.dir, 'Vidéos', 'Episode [abcdefghijk].mp4')));
});

test('the lower of the shared and the own speed limit goes to yt-dlp', async () => {
  const r = await run('ok', { speedLimitKBps: 500 }, 200);
  assert.equal(r.task.status, 'completed', r.task.error);
  assert.equal(r.args[r.args.indexOf('--limit-rate') + 1], '200K');
  const s = await run('ok', { speedLimitKBps: 100 }, 400);
  assert.equal(s.args[s.args.indexOf('--limit-rate') + 1], '100K');
});

test('formatDiag lists what was run and how it ended', () => {
  const text = formatDiag({ url: 'u', args: ['-a', 'b'], code: 1, finalFile: null, fileFound: false, error: 'boom', lines: ['x', 'y'], lastProgress: '1 2', at: new Date(0) });
  assert.match(text, /Arguments : -a b/);
  assert.match(text, /Fichier annoncé : \(aucun\) — introuvable/);
  assert.match(text, /--- sortie de yt-dlp ---\nx\ny/);
});

test('known yt-dlp messages become actions, unknown ones stay null', () => {
  assert.match(explain('HTTP Error 403: Forbidden'), /403/);
  assert.match(explain('Postprocessing: ffprobe and ffmpeg not found'), /ffmpeg/);
  assert.match(explain('[Errno 28] No space left on device'), /Disque plein/);
  assert.match(explain("Sign in to confirm you're not a bot"), /robot/);
  assert.equal(explain('something nobody has seen'), null);
});

test('a page yt-dlp does not know sends the user to the extension, not to a dead end', () => {
  const msg = explain('ERROR: Unsupported URL: https://site.example/serie/1');
  assert.match(msg, /extension/i, 'says what to do instead');
  assert.match(msg, /lecture/i, 'the video has to be playing for the extension to see it');
  assert.doesNotMatch(msg, /YouTube/);
});

test('a refused stream is explained with its likely cause, for a site that is not YouTube', () => {
  const msg = explain('HTTP Error 403: Forbidden', 'https://cdn.example/v/manifest.mpd');
  assert.doesNotMatch(msg, /YouTube/);
  assert.match(msg, /403/);
  assert.match(explain('HTTP Error 403: Forbidden', 'https://www.youtube.com/watch?v=abcdefghijk'), /YouTube/);
  assert.match(explain('HTTP Error 403: Forbidden'), /YouTube/, 'without an address, as before');
});

test('a plain-file site is fetched by the app itself over several connections', async () => {
  const { startServer } = require('./helpers');
  const body = Buffer.alloc(5 * 1024 * 1024, 9);
  const srv = await startServer(body);
  const dir = tmpDir();
  const dataDir = tmpDir();
  process.env.FAKE_DIRECT_URL = srv.url;
  const tools = { find: (n) => (n === 'ytdlp' ? process.execPath : null), ytdlpPrefix: [FAKE] };
  const m = new DownloadManager({ storePath: path.join(dataDir, 'downloads.json'), settings: { dir, connections: 4 }, tickMs: 50, tools });
  m.start();
  try {
    const { id } = m.add('https://videos.example.org/watch/42', { kind: 'ytdlp' });
    await waitFor(() => ['completed', 'error'].includes(m.list()[0].status), 20000);
    const t = m.list()[0];
    assert.equal(t.status, 'completed', t.error ?? '');
    assert.equal(t.filename, 'Direct clip.mp4');
    assert.deepEqual(fs.readFileSync(path.join(dir, 'Direct clip.mp4')), body);
    assert.ok(t.segments?.length > 1 || t.direct, 'the app split the file itself');
    assert.ok(id);
  } finally {
    await m.shutdown();
    await srv.close();
    delete process.env.FAKE_DIRECT_URL;
  }
});

/** Downloads a video, then asks for it again; the stand-in yt-dlp fills the second copy with another byte. */
async function runTwice(add = {}) {
  const dir = tmpDir();
  const dataDir = tmpDir();
  const argsFile = path.join(dataDir, 'args.json');
  Object.assign(process.env, { FAKE_MODE: 'ok', FAKE_ARGS_FILE: argsFile, FAKE_FILL: '7' });
  const tools = { find: (n) => (n === 'ytdlp' ? process.execPath : null), ytdlpPrefix: [FAKE] };
  const m = new DownloadManager({ storePath: path.join(dataDir, 'downloads.json'), settings: { dir }, tickMs: 50, tools });
  m.start();
  try {
    const { id } = m.add(add.url ?? URL, { kind: 'ytdlp', ...add.opts });
    await waitFor(() => ['completed', 'error'].includes(m.list()[0].status), 20000);
    assert.equal(m.list()[0].status, 'completed', m.list()[0].error ?? '');
    process.env.FAKE_FILL = '9';
    m.redownload(id);
    await waitFor(() => ['completed', 'error'].includes(m.list()[0].status), 20000);
    // No arguments file when the app fetched the plain file itself: yt-dlp only answered the probe.
    return { task: m.list()[0], dir, files: fs.readdirSync(dir), args: fs.existsSync(argsFile) ? JSON.parse(fs.readFileSync(argsFile, 'utf8')) : [] };
  } finally {
    await m.shutdown();
    for (const k of ['FAKE_MODE', 'FAKE_ARGS_FILE', 'FAKE_FILL']) delete process.env[k];
  }
}

test('a video downloaded again is overwritten in place: yt-dlp is told to, and the name stays', async () => {
  const r = await runTwice();
  assert.equal(r.task.status, 'completed', r.task.error ?? '');
  assert.ok(r.args.includes('--force-overwrites'), 'without it yt-dlp answers "already downloaded" and does nothing');
  assert.equal(r.task.filename, 'Episode [abcdefghijk].mp4');
  assert.deepEqual(r.files, ['Episode [abcdefghijk].mp4']);
  assert.equal(fs.readFileSync(path.join(r.dir, r.task.filename))[0], 9, 'it is the new copy');
});

test('a video named after its page keeps that name when downloaded again', async () => {
  const r = await runTwice(MANIFEST);
  assert.equal(r.task.status, 'completed', r.task.error ?? '');
  assert.deepEqual(r.files, ['Série - S01E04.mp4']);
  assert.equal(fs.readFileSync(path.join(r.dir, 'Série - S01E04.mp4'))[0], 9);
});

test('a plain-file video fetched by the app keeps its name when downloaded again', async () => {
  const { startServer } = require('./helpers');
  const srv = await startServer(Buffer.alloc(3 * 1024 * 1024, 5));
  process.env.FAKE_DIRECT_URL = srv.url;
  try {
    const r = await runTwice({ url: 'https://videos.example.org/watch/42' });
    assert.equal(r.task.status, 'completed', r.task.error ?? '');
    assert.deepEqual(r.files, ['Direct clip.mp4'], 'no "(1)" copy beside it');
  } finally {
    await srv.close();
    delete process.env.FAKE_DIRECT_URL;
  }
});

test('only a single plain http file counts as a direct download', () => {
  const { directFile } = require('../src/engine/ytdlp');
  assert.deepEqual(directFile({ url: 'https://cdn.x/v.mp4', protocol: 'https', ext: 'mp4', title: 'T' }), { url: 'https://cdn.x/v.mp4', ext: 'mp4', title: 'T', userAgent: '' });
  assert.equal(directFile({ url: 'https://cdn.x/v.m3u8', protocol: 'm3u8_native', ext: 'mp4' }), null);
  assert.equal(directFile({ url: 'https://cdn.x/v.mp4', protocol: 'https', ext: 'mp4', requested_formats: [{}, {}] }), null);
  assert.equal(directFile({ url: 'https://cdn.x/v.mp4', protocol: 'https', ext: 'mp4', fragments: [{}] }), null);
  assert.equal(directFile({ protocol: 'https', ext: 'mp4' }), null);
});
