'use strict';

// The parts of the Tauri shell the page cannot see: the confirmation window opened for the extension, the close
// button and the notification area, the single instance, the clean exit and the installer's --setup-tools. Needs the
// built app (see app.js); skipped without it. The pure logic (menu layout, progress, tooltips) is tested in Rust.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { startServer, randomBody, sha, waitFor } = require('./helpers');
const { BIN, skip, launch } = require('./app');

const EXT_ORIGIN = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop';

/** What the browser extension sends to the app. */
async function postAdd(ui, payload) {
  const port = await ui.ev('appState.port');
  const token = fs.readFileSync(path.join(ui.userData, 'extension-token.txt'), 'utf8').trim();
  const res = await fetch(`http://127.0.0.1:${port}/add`, {
    method: 'POST',
    headers: { Origin: EXT_ORIGIN, 'X-DM-Token': token, 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  return { status: res.status, body: await res.json() };
}

// The answer closes the window it comes from, so its reply never arrives: not awaited.
const answered = (page, code) => void page.ev(code).catch(() => {});

const list = (ui) => ui.ev('window.dm.list()');

test('the extension asks first: the confirmation window shows the file and the answer is what is downloaded', { skip, timeout: 60000 }, async () => {
  const body = randomBody(300 * 1024);
  const origin = await startServer(body);
  const ui = await launch({ settings: { confirmDownloads: true, categories: false } });
  try {
    const sent = await postAdd(ui, { url: origin.url, referer: 'https://site.example/p' });
    assert.deepEqual(sent.body, { pending: true });
    const confirm = await ui.pageOf(/confirm\.html/);
    await waitFor(async () => (await confirm.ev('typeof confirmApi !== "undefined"')) === true, 10000);
    const init = await confirm.ev('confirmApi.init()');
    assert.equal(init.url, origin.url);
    assert.equal(init.filename, 'sample.bin', 'named after the address');
    assert.equal(init.host, '127.0.0.1');
    assert.equal(init.kind, 'file');
    assert.equal(init.dir, ui.dir);
    assert.match(init.icon, /^data:image\/png;base64,/, 'the icon of the file type');
    assert.deepEqual(await list(ui), [], 'nothing is queued before the answer');

    answered(confirm, `confirmApi.answer({ action: 'now', filename: 'chosen.bin', dir: '' })`);
    await waitFor(async () => (await list(ui)).some((t) => t.status === 'completed'), 20000);
    const [task] = await list(ui);
    assert.equal(task.filename, 'chosen.bin', 'the name the user typed');
    assert.equal(sha(fs.readFileSync(path.join(ui.dir, 'chosen.bin'))), sha(body));
    await waitFor(async () => !(await ui.targets()).some((t) => /confirm\.html/.test(t.url)), 10000);
  } finally {
    ui.close();
    await origin.close();
  }
});

test('a confirmation window answered "later" queues the download paused, and "cancel" queues nothing', { skip, timeout: 60000 }, async () => {
  const origin = await startServer(randomBody(50 * 1024));
  const ui = await launch({ settings: { confirmDownloads: true } });
  try {
    await postAdd(ui, { url: origin.url });
    const later = await ui.pageOf(/confirm\.html/);
    await waitFor(async () => (await later.ev('typeof confirmApi !== "undefined"')) === true, 10000);
    answered(later, `confirmApi.answer({ action: 'later', filename: '', dir: '' })`);
    await waitFor(async () => (await list(ui)).length === 1, 10000);
    assert.equal((await list(ui))[0].status, 'paused');

    await postAdd(ui, { url: origin.url + '?second' });
    const cancel = await ui.pageOf(/confirm\.html/);
    await waitFor(async () => (await cancel.ev('typeof confirmApi !== "undefined"')) === true, 10000);
    answered(cancel, `confirmApi.answer({ action: 'cancel' })`);
    await waitFor(async () => !(await ui.targets()).some((t) => /confirm\.html/.test(t.url)), 10000);
    assert.equal((await list(ui)).length, 1, 'cancelling adds nothing');
  } finally {
    ui.close();
    await origin.close();
  }
});

test('closing the confirmation window without an answer cancels the request', { skip, timeout: 60000 }, async () => {
  const origin = await startServer(randomBody(50 * 1024));
  const ui = await launch({ settings: { confirmDownloads: true } });
  try {
    await postAdd(ui, { url: origin.url });
    await ui.pageOf(/confirm\.html/);
    ui.win('close-confirm');
    await waitFor(async () => !(await ui.targets()).some((t) => /confirm\.html/.test(t.url)), 10000);
    assert.deepEqual(await list(ui), []);
    // the next request opens its own window: the closed one did not leave anything behind
    await postAdd(ui, { url: origin.url + '?again' });
    const again = await ui.pageOf(/confirm\.html/);
    await waitFor(async () => (await again.ev('typeof confirmApi !== "undefined"')) === true, 10000);
    assert.equal((await again.ev('confirmApi.init()')).url, origin.url + '?again');
  } finally {
    ui.close();
    await origin.close();
  }
});

test('without confirmation the extension adds straight to the list', { skip, timeout: 60000 }, async () => {
  const origin = await startServer(randomBody(80 * 1024));
  const ui = await launch({ settings: { confirmDownloads: false } });
  try {
    const sent = await postAdd(ui, { url: origin.url });
    assert.equal(sent.status, 200);
    assert.ok(sent.body.id, 'the answer carries the id of the new download');
    await waitFor(async () => (await list(ui)).some((t) => t.status === 'completed'), 20000);
    assert.deepEqual(await ui.targets().then((t) => t.map((x) => x.url.split('/').pop())), ['index.html'], 'no window opened');
  } finally {
    ui.close();
    await origin.close();
  }
});

test('the close button hides the window to the notification area and a second launch brings it back', { skip, timeout: 60000 }, async () => {
  const ui = await launch();
  try {
    assert.equal(ui.win('visible'), 'True');
    ui.win('close');
    await waitFor(async () => ui.win('visible') === 'False', 10000);
    assert.equal(await ui.ev('window.dm.list().then((l) => l.length)'), 0, 'the app keeps running');
    // the same app started again: it hands over to the running one and exits
    const second = spawn(BIN, ['--data-dir', ui.userData], { stdio: 'ignore' });
    const code = await new Promise((resolve) => second.once('exit', resolve));
    assert.equal(code, 0);
    await waitFor(async () => ui.win('visible') === 'True', 10000);
  } finally {
    ui.close();
  }
});

test('with the notification area turned off, closing the window ends the app and saves the list', { skip, timeout: 60000 }, async () => {
  const ui = await launch({ settings: { closeToTray: false } });
  try {
    const later = Date.now() + 3600e3;
    await ui.ev(`window.dm.add(['http://127.0.0.1:9/kept.bin'], { startAt: ${later} })`);
    await waitFor(async () => (await list(ui)).length === 1, 10000);
    ui.win('close');
    await Promise.race([ui.exited, new Promise((_, reject) => setTimeout(() => reject(new Error('the app did not exit')), 20000))]);
    const saved = JSON.parse(fs.readFileSync(path.join(ui.userData, 'downloads.json'), 'utf8'));
    assert.equal(saved.tasks.length, 1);
    assert.equal(saved.tasks[0].filename, 'kept.bin');
    assert.equal(saved.settings.closeToTray, false);
  } finally {
    ui.close();
  }
});

test('--setup-tools opens no window and exits when every tool is already there', { skip, timeout: 60000 }, async () => {
  const data = fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'dm-setup-'));
  const tools = path.join(data, 'tools');
  fs.mkdirSync(tools);
  for (const f of ['yt-dlp.exe', 'deno.exe', 'ffmpeg.exe']) fs.writeFileSync(path.join(tools, f), 'x');
  const app = spawn(BIN, ['--setup-tools', '--data-dir', data], { stdio: 'ignore' });
  const code = await Promise.race([new Promise((resolve) => app.once('exit', resolve)), new Promise((resolve) => setTimeout(() => resolve('timeout'), 30000))]);
  if (code === 'timeout') app.kill();
  assert.equal(code, 0);
  assert.ok(!fs.existsSync(path.join(data, 'extension-token.txt')), 'nothing of the normal start-up ran');
});
