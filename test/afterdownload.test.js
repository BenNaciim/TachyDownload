'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { DownloadManager } = require('../src/engine/manager');
const { parseRules, folderFor } = require('../src/engine/rules');
const { isArchive, extractArchive, systemTar } = require('../src/engine/postprocess');
const { startServer, tmpDir, randomBody, waitFor } = require('./helpers');

const done = (m, id) => m.list().find((t) => t.id === id);
const finished = (m, id) => waitFor(() => ['completed', 'error'].includes(done(m, id).status), 20000);

test('filing rules: one per line, by extension or by site, relative or absolute folder', () => {
  const rules = parseRules('# comment\n*.pdf = Documents\n.iso=D:\\Images\nexample.com = "Sites\\Example"\n\nnot a rule\n');
  assert.deepEqual(rules, [
    { ext: 'pdf', folder: 'Documents' },
    { ext: 'iso', folder: 'D:\\Images' },
    { host: 'example.com', folder: 'Sites\\Example' },
  ]);
  const base = path.join(path.parse(process.cwd()).root, 'dl');
  assert.equal(folderFor(rules, { filename: 'a.PDF', url: 'http://x.org/a.PDF' }, base), path.join(base, 'Documents'));
  assert.equal(folderFor(rules, { filename: 'a.iso', url: 'http://x.org/a.iso' }, base), 'D:\\Images');
  assert.equal(folderFor(rules, { filename: 'a.bin', url: 'https://cdn.example.com/a.bin' }, base), path.join(base, 'Sites\\Example'), 'subdomains match');
  assert.equal(folderFor(rules, { filename: 'a.bin', url: 'https://notexample.com/a.bin' }, base), null, 'a lookalike host does not');
  assert.equal(folderFor(rules, { filename: 'a.bin', url: 'http://x.org/a.bin', pageUrl: 'https://www.example.com/page' }, base), path.join(base, 'Sites\\Example'), 'the page it came from counts');
  assert.equal(folderFor([], { filename: 'a.bin', url: 'http://x.org/a.bin' }, base), null);
});

test('a rule sends the finished file to its folder, before the type folders', async () => {
  const body = randomBody(80 * 1024);
  const srv = await startServer(body);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, categories: true, rules: '127.0.0.1 = Mon site\n.bin = Binaires' }, tickMs: 30 });
  m.start();
  try {
    const a = m.add(srv.url, { filename: 'a.bin' });
    await finished(m, a.id);
    assert.deepEqual(fs.readFileSync(path.join(dir, 'Mon site', 'a.bin')), body, 'the first matching rule wins');

    const b = m.add(srv.url, { filename: 'b.bin', ownFolder: true });
    await finished(m, b.id);
    assert.ok(fs.existsSync(path.join(dir, 'b.bin')), 'a folder chosen by hand is left alone');

    m.setSettings({ rules: '' });
    const c = m.add(srv.url, { filename: 'c.bin' });
    await finished(m, c.id);
    assert.ok(fs.existsSync(path.join(dir, 'Autres', 'c.bin')), 'no rule: the type folder again');
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('a file identical to one already received is flagged, a different one of the same size is not', async () => {
  const body = randomBody(60 * 1024);
  const other = randomBody(60 * 1024);
  const s1 = await startServer(body);
  const s2 = await startServer(other);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir }, tickMs: 30 });
  m.start();
  try {
    const a = m.add(s1.url, { filename: 'one.bin' });
    await finished(m, a.id);
    const b = m.add(s1.url, { filename: 'two.bin' });
    await finished(m, b.id);
    await waitFor(() => done(m, b.id).duplicateOf);
    assert.equal(done(m, b.id).duplicateOf, path.join(dir, 'one.bin'));
    assert.equal(done(m, a.id).duplicateOf, undefined, 'the first copy is the original');

    const c = m.add(s2.url, { filename: 'three.bin' });
    await finished(m, c.id);
    await new Promise((r) => setTimeout(r, 200));
    assert.equal(done(m, c.id).duplicateOf, undefined, 'same size, other content');

    m.setSettings({ duplicates: false });
    const d = m.add(s1.url, { filename: 'four.bin' });
    await finished(m, d.id);
    await new Promise((r) => setTimeout(r, 200));
    assert.equal(done(m, d.id).duplicateOf, undefined, 'detection off');
  } finally {
    await m.shutdown();
    await s1.close();
    await s2.close();
  }
});

let hasTar = true;
try {
  execFileSync(systemTar(), ['--version'], { stdio: 'ignore' });
} catch {
  hasTar = false;
}

test('archives are recognised by their extension', () => {
  for (const n of ['a.zip', 'A.ZIP', 'b.7z', 'c.tar', 'd.tar.gz', 'e.tgz', 'f.tar.xz']) assert.ok(isArchive(n), n);
  for (const n of ['a.zipx', 'b.rar', 'c.txt', 'zip']) assert.ok(!isArchive(n), n);
});

test('a finished zip is unpacked into its own folder when asked to', { skip: !hasTar && 'tar is not available' }, async () => {
  const work = tmpDir();
  fs.writeFileSync(path.join(work, 'hello.txt'), 'bonjour');
  fs.mkdirSync(path.join(work, 'sub'));
  fs.writeFileSync(path.join(work, 'sub', 'deep.txt'), 'profond');
  execFileSync(systemTar(), ['-a', '-cf', path.join(work, 'pack.zip'), '-C', work, 'hello.txt', 'sub']);
  const zip = fs.readFileSync(path.join(work, 'pack.zip'));
  const srv = await startServer(zip);
  const dir = tmpDir();
  const m = new DownloadManager({ settings: { dir, autoExtract: true }, tickMs: 30 });
  m.start();
  try {
    const a = m.add(srv.url, { filename: 'pack.zip' });
    await finished(m, a.id);
    await waitFor(() => done(m, a.id).extractedTo);
    const out = done(m, a.id).extractedTo;
    assert.equal(out, path.join(dir, 'pack'));
    assert.equal(fs.readFileSync(path.join(out, 'hello.txt'), 'utf8'), 'bonjour');
    assert.equal(fs.readFileSync(path.join(out, 'sub', 'deep.txt'), 'utf8'), 'profond');
    assert.ok(fs.existsSync(path.join(dir, 'pack.zip')), 'the archive itself is kept');

    // a second download of the same name does not merge into the first folder
    const b = m.add(srv.url, { filename: 'pack.zip' });
    await finished(m, b.id);
    await waitFor(() => done(m, b.id).extractedTo);
    assert.notEqual(done(m, b.id).extractedTo, out);

    // setting off: nothing is unpacked
    m.setSettings({ autoExtract: false });
    const c = m.add(srv.url, { filename: 'other.zip' });
    await finished(m, c.id);
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(done(m, c.id).extractedTo, undefined);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});

test('a broken archive reports the error and leaves no empty folder behind', { skip: !hasTar && 'tar is not available' }, async () => {
  const dir = tmpDir();
  const file = path.join(dir, 'broken.zip');
  fs.writeFileSync(file, randomBody(5000));
  await assert.rejects(extractArchive(file), /./);
  assert.ok(!fs.existsSync(path.join(dir, 'broken')), 'the half-made folder is removed');

  const srv = await startServer(randomBody(5000));
  const m = new DownloadManager({ settings: { dir, autoExtract: true }, tickMs: 30 });
  m.start();
  try {
    const a = m.add(srv.url, { filename: 'bad.zip' });
    await finished(m, a.id);
    await waitFor(() => done(m, a.id).extractError);
    assert.equal(done(m, a.id).status, 'completed', 'the download itself still counts as done');
    assert.equal(done(m, a.id).extracting, false);
  } finally {
    await m.shutdown();
    await srv.close();
  }
});
