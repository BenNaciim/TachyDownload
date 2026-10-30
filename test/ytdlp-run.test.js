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
async function run(mode, settings = {}) {
  const dir = tmpDir();
  const dataDir = tmpDir();
  const argsFile = path.join(dataDir, 'args.json');
  process.env.FAKE_MODE = mode;
  process.env.FAKE_ARGS_FILE = argsFile;
  const tools = { find: (n) => (n === 'ytdlp' ? process.execPath : null), ytdlpPrefix: [FAKE] };
  const m = new DownloadManager({ storePath: path.join(dataDir, 'downloads.json'), settings: { dir, ...settings }, tickMs: 50, tools });
  m.start();
  try {
    m.add(URL, { kind: 'ytdlp' });
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
