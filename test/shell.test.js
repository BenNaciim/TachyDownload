'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createLogins, cleanHost } = require('../src/logins');
const { overallProgress } = require('../src/progress');
const { menuTemplate } = require('../src/menu');
const { DownloadManager } = require('../src/engine/manager');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

// A stand-in for safeStorage: reversible, and clearly not the plain text.
const crypt = {
  encrypt: (t) => Buffer.from([...t].reverse().join('')).toString('base64'),
  decrypt: (b) => [...Buffer.from(b, 'base64').toString()].reverse().join(''),
};

test('a site is read out of an address, a bare host or a host with a port', () => {
  assert.equal(cleanHost('https://Example.com/some/page?x=1'), 'example.com');
  assert.equal(cleanHost('example.com'), 'example.com');
  assert.equal(cleanHost('files.example.com:8080'), 'files.example.com');
  assert.equal(cleanHost('*.example.com'), 'example.com');
  assert.throws(() => cleanHost(''), /site/);
  assert.throws(() => cleanHost('exa mple'), /invalide/);
});

test('logins become the right headers, for that site and its subdomains only', () => {
  const dir = tmpDir();
  const l = createLogins({ file: path.join(dir, 'logins.json'), crypt });
  l.save({ host: 'https://example.com/login', type: 'basic', user: 'moi', secret: 'pa:ss' });
  l.save({ host: 'api.other.org', type: 'bearer', secret: 'tok123' });
  l.save({ host: 'cdn.net', type: 'cookie', secret: 'sid=abc' });

  assert.equal(l.headersFor('https://example.com/a.zip').Authorization, 'Basic ' + Buffer.from('moi:pa:ss').toString('base64'));
  assert.ok(l.headersFor('https://files.example.com/a.zip'), 'a subdomain is covered');
  assert.equal(l.headersFor('https://notexample.com/a.zip'), null, 'a lookalike is not');
  assert.equal(l.headersFor('https://example.com.evil.io/a.zip'), null);
  assert.deepEqual(l.headersFor('https://api.other.org/x'), { Authorization: 'Bearer tok123' });
  assert.deepEqual(l.headersFor('http://cdn.net/x'), { Cookie: 'sid=abc' });
  assert.equal(l.headersFor('not a url'), null);

  // the more specific entry wins
  l.save({ host: 'vip.example.com', type: 'bearer', secret: 'vip' });
  assert.deepEqual(l.headersFor('https://vip.example.com/x'), { Authorization: 'Bearer vip' });
  assert.ok(l.headersFor('https://example.com/x').Authorization.startsWith('Basic'));
});

test('the list never shows a secret, and the file on disk holds none in clear', () => {
  const file = path.join(tmpDir(), 'logins.json');
  const l = createLogins({ file, crypt });
  const after = l.save({ host: 'example.com', type: 'basic', user: 'moi', secret: 'motdepasse-unique' });
  assert.deepEqual(after, [{ host: 'example.com', type: 'basic', user: 'moi' }]);
  assert.ok(!fs.readFileSync(file, 'utf8').includes('motdepasse-unique'));

  // a fresh start reads the same entries back
  const again = createLogins({ file, crypt });
  assert.equal(again.list().length, 1);
  assert.ok(again.headersFor('https://example.com/x').Authorization);

  // saving the same site again replaces it; removing works
  again.save({ host: 'example.com', type: 'bearer', secret: 'new' });
  assert.deepEqual(again.list(), [{ host: 'example.com', type: 'bearer', user: '' }]);
  assert.deepEqual(again.remove('https://example.com'), []);
  assert.equal(again.headersFor('https://example.com/x'), null);
});

test('incomplete or unknown logins are refused', () => {
  const l = createLogins({ file: path.join(tmpDir(), 'l.json'), crypt });
  assert.throws(() => l.save({ host: 'a.com', type: 'basic', user: '', secret: 'x' }), /utilisateur/);
  assert.throws(() => l.save({ host: 'a.com', type: 'basic', user: 'u', secret: '' }), /mot de passe/);
  assert.throws(() => l.save({ host: 'a.com', type: 'cookie', secret: '' }), /cookie/);
  assert.throws(() => l.save({ host: 'a.com', type: 'magic', secret: 'x' }), /inconnu/);
  assert.throws(() => l.save({ host: '', type: 'bearer', secret: 'x' }), /site/);
  assert.deepEqual(l.list(), []);
});

test('a secret that cannot be decrypted (another Windows account) simply gives no header', () => {
  const file = path.join(tmpDir(), 'l.json');
  createLogins({ file, crypt }).save({ host: 'a.com', type: 'bearer', secret: 'x' });
  const broken = createLogins({ file, crypt: { encrypt: crypt.encrypt, decrypt: () => { throw new Error('cannot decrypt'); } } });
  assert.equal(broken.headersFor('https://a.com/f'), null);
  assert.equal(broken.list().length, 1, 'the entry is still listed so it can be removed');
});

test('a damaged logins file starts empty instead of crashing', () => {
  const file = path.join(tmpDir(), 'l.json');
  fs.writeFileSync(file, '{ not json');
  assert.deepEqual(createLogins({ file, crypt }).list(), []);
});

test('taskbar progress: nothing running, unknown sizes, and a real fraction', () => {
  assert.deepEqual(overallProgress([]), { mode: 'none', value: -1 });
  assert.equal(overallProgress([{ status: 'paused', size: 10, downloaded: 5 }, { status: 'completed', size: 10, downloaded: 10 }]).mode, 'none');
  assert.equal(overallProgress([{ status: 'downloading', size: null, downloaded: 500 }]).mode, 'indeterminate');
  const p = overallProgress([
    { status: 'downloading', size: 1000, downloaded: 250 },
    { status: 'downloading', size: 3000, downloaded: 1750 },
    { status: 'downloading', size: null, downloaded: 99999 },
    { status: 'queued', size: 5000, downloaded: 0 },
  ]);
  assert.equal(p.mode, 'normal');
  assert.equal(p.value, 0.5, 'the weight of each file follows its size; unsized and waiting ones are left out');
  assert.equal(overallProgress([{ status: 'downloading', size: 100, downloaded: 400 }]).value, 1, 'never above 100 %');
});

const flatten = (items, out = []) => {
  for (const i of items) {
    out.push(i);
    if (i.submenu) flatten(i.submenu, out);
  }
  return out;
};

test('the menu bar offers the main actions and each entry calls the right thing', () => {
  const calls = [];
  const actions = new Proxy({}, { get: (_t, name) => (...a) => calls.push([name, ...a]) });
  const tpl = menuTemplate({ settings: { speedLimitKBps: 1024, afterAll: 'quit' }, actions });
  assert.deepEqual(tpl.map((m) => m.label), ['Fichier', 'Téléchargements', 'Affichage', 'Aide']);

  const all = flatten(tpl);
  const byLabel = (l) => all.find((i) => i.label === l);
  byLabel('Ajouter un lien…').click();
  byLabel('Paramètres').click();
  byLabel('Rechercher').click();
  byLabel('Sombre').click();
  byLabel('Tout mettre en pause').click();
  byLabel('Tout reprendre').click();
  byLabel('Retirer les téléchargements terminés de la liste').click();
  byLabel('512 Ko/s').click();
  byLabel('Éteindre l’ordinateur').click();
  assert.deepEqual(calls, [
    ['ui', 'add'],
    ['ui', 'settings'],
    ['ui', 'search'],
    ['ui', 'theme', 'dark'],
    ['pauseAll'],
    ['resumeAll'],
    ['clearFinished'],
    ['setLimit', 512],
    ['setAfterAll', 'shutdown'],
  ]);

  // the choice in force is ticked
  assert.equal(byLabel('1 Mo/s').checked, true);
  assert.equal(byLabel('512 Ko/s').checked, false);
  assert.equal(byLabel('Quitter l’application').checked, true);
  assert.equal(byLabel('Ne rien faire').checked, false);

  // developer tools only when asked for
  assert.ok(!all.some((i) => i.role === 'toggleDevTools'));
  assert.ok(flatten(menuTemplate({ settings: { speedLimitKBps: 0, afterAll: 'none' }, actions, devTools: true })).some((i) => i.role === 'toggleDevTools'));
});

test('every shortcut in the menu is used only once', () => {
  const keys = flatten(menuTemplate({ settings: { speedLimitKBps: 0, afterAll: 'none' }, actions: {} })).map((i) => i.accelerator).filter(Boolean);
  assert.equal(new Set(keys).size, keys.length);
});

test('finished downloads can be taken off the list in one go, their files stay', async () => {
  const body = randomBody(40 * 1024);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 30 });
  m.start();
  try {
    const a = m.add(srv.url, { filename: 'a.bin' });
    const b = m.add(srv.url, { filename: 'b.bin' });
    await waitFor(() => m.list().every((t) => t.status === 'completed'));
    const paused = m.add(srv.url, { filename: 'c.bin', paused: true });
    assert.equal(await m.clearCompleted(), 2);
    assert.deepEqual(m.list().map((t) => t.id), [paused.id]);
    assert.ok(fs.existsSync(path.join(dir, 'a.bin')) && fs.existsSync(path.join(dir, 'b.bin')));
    assert.ok(a.id && b.id);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
