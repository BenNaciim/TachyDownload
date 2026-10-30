'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { DownloadManager } = require('../src/engine/manager');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

const COOKIE = 'session=secret-value';
const done = (m, id) => waitFor(() => ['completed', 'error'].includes(m.list().find((t) => t.id === id)?.status));

test('the cookie is used for the download, then forgotten once it is finished', async () => {
  const srv = await startServer(randomBody(512 * 1024));
  const dir = tmpDir();
  const storePath = path.join(tmpDir(), 'downloads.json');
  const m = new DownloadManager({ storePath, settings: { dir }, tickMs: 50 });
  m.start();
  try {
    const { id } = m.add(srv.url, { cookie: COOKIE });
    assert.ok(JSON.stringify(m.tasks.get(id).state).includes(COOKIE), 'kept while the download may need to resume');
    assert.ok(!JSON.stringify(m.list()).includes(COOKIE), 'never sent to the window');
    await done(m, id);
    assert.equal(m.list().find((t) => t.id === id).status, 'completed');
    m.saveNow();
    assert.ok(!fs.readFileSync(storePath, 'utf8').includes(COOKIE), 'gone from the file once completed');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('a video taken by yt-dlp never stores a cookie, and old finished tasks are cleaned at load', async () => {
  const storePath = path.join(tmpDir(), 'downloads.json');
  fs.writeFileSync(
    storePath,
    JSON.stringify({
      settings: { dir: tmpDir() },
      tasks: [
        { id: 'old', kind: 'file', url: 'http://127.0.0.1/a.bin', dir: tmpDir(), filename: 'a.bin', status: 'completed', headers: { referer: '', cookie: COOKIE, userAgent: '' } },
        { id: 'half', kind: 'file', url: 'http://127.0.0.1/b.bin', dir: tmpDir(), filename: 'b.bin', status: 'paused', headers: { referer: '', cookie: COOKIE, userAgent: '' } },
      ],
    }),
  );
  const m = new DownloadManager({ storePath, settings: { dir: tmpDir() }, tickMs: 50 });
  m.load();
  assert.equal(m.tasks.get('old').state.headers.cookie, '', 'finished: cleaned');
  assert.equal(m.tasks.get('half').state.headers.cookie, COOKIE, 'unfinished: kept so it can resume');
  const { id } = m.add('https://www.youtube.com/watch?v=abcdefghijk', { kind: 'ytdlp', cookie: COOKIE, paused: true });
  assert.equal(m.tasks.get(id).state.headers.cookie, '');
  await m.shutdown();
});
