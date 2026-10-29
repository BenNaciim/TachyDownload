'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { checkForUpdate, downloadUpdate, compareVersions } = require('../src/updater');

const installer = Buffer.from('MZ-pretend-installer'.repeat(5000));
const sha = crypto.createHash('sha256').update(installer).digest('hex');

function mockGithub(release) {
  const srv = http.createServer((q, s) => {
    if (q.url.endsWith('/releases/latest')) {
      if (!release) return s.writeHead(404).end('{}');
      const port = srv.address().port;
      return s.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify(release(port)));
    }
    if (q.url === '/dl/setup.exe') return s.writeHead(200, { 'Content-Length': installer.length }).end(installer);
    return s.writeHead(404).end();
  });
  return new Promise((r) => srv.listen(0, '127.0.0.1', () => r(srv)));
}

const rel = (over = {}) => (port) => ({
  tag_name: 'v1.2.0',
  html_url: 'https://github.com/x/y/releases/tag/v1.2.0',
  assets: [
    { name: 'notes.txt', browser_download_url: `http://127.0.0.1:${port}/dl/notes.txt`, size: 1 },
    { name: 'Tachy-Download-Setup-1.2.0.exe', browser_download_url: `http://127.0.0.1:${port}/dl/setup.exe`, size: installer.length, digest: `sha256:${sha}` },
  ],
  ...over,
});

test('compareVersions orders numerically, ignoring a leading v', () => {
  assert.equal(compareVersions('v1.10.0', '1.9.9'), 1);
  assert.equal(compareVersions('1.0.0', 'v1.0.0'), 0);
  assert.equal(compareVersions('1.0', '1.0.1'), -1);
});

test('a newer release with an installer is offered, with its checksum', async () => {
  const srv = await mockGithub(rel());
  try {
    const info = await checkForUpdate({ current: '1.0.0', api: `http://127.0.0.1:${srv.address().port}` });
    assert.equal(info.version, '1.2.0');
    assert.equal(info.sha256, sha);
    assert.ok(info.url.endsWith('/dl/setup.exe'));
  } finally {
    srv.close();
  }
});

test('no update when current, when private (404), or when it is a draft or prerelease', async () => {
  for (const [release, current] of [[rel(), '1.2.0'], [null, '1.0.0'], [rel({ draft: true }), '1.0.0'], [rel({ prerelease: true }), '1.0.0']]) {
    const srv = await mockGithub(release);
    try {
      assert.equal(await checkForUpdate({ current, api: `http://127.0.0.1:${srv.address().port}` }), null);
    } finally {
      srv.close();
    }
  }
});

test('the installer is downloaded and verified', async () => {
  const srv = await mockGithub(rel());
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dm-upd-'));
  try {
    const info = await checkForUpdate({ current: '1.0.0', api: `http://127.0.0.1:${srv.address().port}` });
    let last = 0;
    const file = await downloadUpdate(info, dir, { onProgress: (n) => (last = n) });
    assert.equal(fs.readFileSync(file).length, installer.length);
    assert.equal(last, installer.length);
    assert.deepEqual(fs.readdirSync(dir), [path.basename(file)]);
  } finally {
    srv.close();
  }
});

test('a wrong checksum or a missing one is refused and leaves nothing behind', async () => {
  const srv = await mockGithub(rel());
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dm-upd-'));
  try {
    const info = await checkForUpdate({ current: '1.0.0', api: `http://127.0.0.1:${srv.address().port}` });
    await assert.rejects(downloadUpdate({ ...info, sha256: 'a'.repeat(64) }, dir), /incorrecte/);
    await assert.rejects(downloadUpdate({ ...info, sha256: null }, dir), /absente/);
    assert.deepEqual(fs.readdirSync(dir), []);
  } finally {
    srv.close();
  }
});

test('an installer from an unexpected host is refused', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dm-upd-'));
  await assert.rejects(downloadUpdate({ url: 'https://evil.example.com/a.exe', version: '9.9.9', sha256: sha }, dir), /refusée/);
  await assert.rejects(downloadUpdate({ url: 'http://github.com/a.exe', version: '9.9.9', sha256: sha }, dir), /refusée/);
});
