'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { isIdle, commandFor } = require('../src/afterall');
const { DownloadManager } = require('../src/engine/manager');

test('isIdle ignores finished, failed and paused items only', () => {
  const t = (status) => ({ status });
  assert.equal(isIdle([t('completed'), t('error'), t('paused')]), true);
  for (const s of ['downloading', 'queued', 'scheduled']) assert.equal(isIdle([t('completed'), t(s)]), false);
  assert.equal(isIdle([]), true);
});

test('commandFor builds the system commands, quit is handled by the app', () => {
  assert.equal(commandFor('quit'), null);
  assert.equal(commandFor('none'), null);
  assert.deepEqual(commandFor('shutdown').args.slice(0, 3), ['/s', '/t', '60']);
  assert.equal(commandFor('sleep').cmd, 'rundll32.exe');
});

test('afterAll and notifications settings are validated', () => {
  const m = new DownloadManager({ settings: { dir: 'x', afterAll: 'format-disk', notifications: false } });
  assert.equal(m.settings.afterAll, 'none');
  assert.equal(m.settings.notifications, false);
  assert.equal(m.setSettings({ afterAll: 'shutdown' }).afterAll, 'shutdown');
});
