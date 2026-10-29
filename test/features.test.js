'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { DownloadManager } = require('../src/engine/manager');
const { categoryFor } = require('../src/engine/util');
const { isVideoPage, isPlaylistUrl } = require('../src/engine/ytdlp');
const { Tools } = require('../src/engine/tools');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

const finished = (m, id) =>
  waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status), 30000);

const ffmpegPath = new Tools(tmpDir()).find('ffmpeg');

test('categoryFor maps extensions to folders', () => {
  assert.equal(categoryFor('a.PDF'), 'Documents');
  assert.equal(categoryFor('a.mkv'), 'Vidéos');
  assert.equal(categoryFor('a.iso'), 'Programmes');
  assert.equal(categoryFor('a.xyz'), null);
});

test('video page detection', () => {
  assert.ok(isVideoPage('https://www.youtube.com/watch?v=abc'));
  assert.ok(isVideoPage('https://youtu.be/abc'));
  assert.ok(!isVideoPage('https://example.com/watch?v=abc'));
  assert.ok(!isVideoPage('https://www.youtube.com/'));
});

test('playlist detection: only real playlist pages, not a video that happens to carry list=', () => {
  assert.ok(isPlaylistUrl('https://www.youtube.com/playlist?list=PLabc_123-x'));
  assert.ok(!isPlaylistUrl('https://www.youtube.com/watch?v=abc&list=PLabc'));
});

test('addPlaylist without yt-dlp explains what to install', async () => {
  const m = new DownloadManager({ settings: { dir: tmpDir() } });
  await assert.rejects(m.addPlaylist('https://www.youtube.com/playlist?list=PLabc'), /Outils vidéo/);
});

test('categories on: finished file goes into its type folder; off: stays put', async () => {
  const body = randomBody(200 * 1024);
  const srv = await startServer(body, { disposition: 'attachment; filename="rapport.pdf"' });
  for (const [on, expectedDir] of [[true, 'Documents'], [false, '']]) {
    const dir = tmpDir();
    const m = new DownloadManager({ settings: { dir, categories: on }, tickMs: 50 });
    m.start();
    try {
      const t = m.add(`${srv.url}?c=${on}`);
      await finished(m, t.id);
      const done = m.list()[0];
      assert.equal(done.status, 'completed', done.error);
      assert.equal(done.dir, path.join(dir, expectedDir));
      assert.ok(fs.existsSync(path.join(done.dir, 'rapport.pdf')));
      assert.ok(!fs.existsSync(path.join(dir, 'rapport.pdf.part')));
    } finally {
      await m.shutdown();
    }
  }
  await srv.close();
});

test('yt-dlp task without the tool fails with an actionable message', async () => {
  const m = new DownloadManager({ settings: { dir: tmpDir() }, tickMs: 50 });
  m.start();
  try {
    const t = m.add('https://www.youtube.com/watch?v=jNQXAC9IJ2w');
    assert.equal(t.kind, 'ytdlp');
    await finished(m, t.id);
    assert.match(m.list()[0].error, /Outils vidéo/);
  } finally {
    await m.shutdown();
  }
});

test('HLS with ffmpeg: segments are remuxed into a real mp4', { skip: !ffmpegPath }, async () => {
  const src = tmpDir();
  execFileSync(
    ffmpegPath,
    ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc=duration=4:size=160x120:rate=10', '-f', 'lavfi', '-i', 'sine=duration=4',
      '-c:v', 'libx264', '-c:a', 'aac', '-f', 'hls', '-hls_time', '1', '-hls_playlist_type', 'vod', path.join(src, 'v.m3u8')],
  );
  const server = http.createServer((req, res) => {
    const f = path.join(src, path.basename(req.url.split('?')[0]));
    if (!fs.existsSync(f)) { res.writeHead(404); return res.end(); }
    res.writeHead(200);
    res.end(fs.readFileSync(f));
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 50, tools: { find: (n) => (n === 'ffmpeg' ? ffmpegPath : null) } });
  m.start();
  try {
    const t = m.add(`http://127.0.0.1:${server.address().port}/v.m3u8`, { filename: 'clip' });
    await finished(m, t.id);
    const done = m.list()[0];
    assert.equal(done.status, 'completed', done.error);
    assert.equal(done.filename, 'clip.mp4');
    const file = path.join(dir, 'clip.mp4');
    assert.equal(fs.readFileSync(file).subarray(4, 8).toString(), 'ftyp');
    const probe = execFileSync(ffmpegPath, ['-v', 'error', '-i', file, '-f', 'null', '-'], { encoding: 'utf8' });
    assert.equal(probe, ''); // decodes without errors
    assert.deepEqual(fs.readdirSync(dir), ['clip.mp4']); // no leftovers
  } finally {
    await m.shutdown();
    server.closeAllConnections?.();
    server.close();
  }
});
