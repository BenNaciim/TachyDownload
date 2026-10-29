'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Tools } = require('../src/engine/tools');
const { tmpDir } = require('./helpers');

function fake({ latest, installed, withCopy = true }) {
  const dir = tmpDir();
  if (withCopy) fs.writeFileSync(path.join(dir, 'yt-dlp.exe'), 'x');
  const t = new Tools(dir);
  t.installs = 0;
  t._latestYtdlp = async () => latest;
  t._installedVersion = async () => installed;
  t.install = async () => {
    t.installs++;
  };
  return t;
}

test('yt-dlp: an outdated own copy is replaced, then not checked again for a while', async () => {
  const t = fake({ latest: '2026.10.26', installed: '2026.01.01' });
  assert.equal(await t.checkYtdlpUpdate(), 'updated');
  assert.equal(t.installs, 1);
  assert.equal(await t.checkYtdlpUpdate(), 'recent');
  assert.equal(t.installs, 1);
});

test('yt-dlp: an up-to-date copy is left alone', async () => {
  const t = fake({ latest: '2026.10.26', installed: '2026.10.26' });
  assert.equal(await t.checkYtdlpUpdate(), 'current');
  assert.equal(t.installs, 0);
});

test('yt-dlp: no own copy (or a download in progress) means no update', async () => {
  assert.equal(await fake({ latest: 'v', installed: 'old', withCopy: false }).checkYtdlpUpdate(), 'absent');
  const t = fake({ latest: 'v', installed: 'old' });
  assert.equal(await t.checkYtdlpUpdate({ busy: () => true }), 'busy');
  assert.equal(t.installs, 0);
});

test('yt-dlp: a failed lookup throws and leaves the copy as it is', async () => {
  const t = fake({ latest: '', installed: 'old' });
  t._latestYtdlp = async () => {
    throw new Error('offline');
  };
  await assert.rejects(t.checkYtdlpUpdate(), /offline/);
  assert.equal(t.installs, 0);
});
