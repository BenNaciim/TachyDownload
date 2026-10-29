'use strict';

// Usage: npm run release -- <patch|minor|major|x.y.z> [--dry-run] [--notes "text"]
//
// Bumps the version (package.json, package-lock.json, extension/manifest.json), runs the tests,
// builds the installer, commits "Release vX.Y.Z", tags it, pushes, and creates the GitHub release
// with the installer attached. Needs git, npm and the GitHub CLI (`gh`, signed in via `gh auth login`).

const fs = require('node:fs');
const path = require('node:path');
const { execFileSync, spawnSync } = require('node:child_process');

const root = path.join(__dirname, '..');
const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const notesAt = args.indexOf('--notes');
const customNotes = notesAt >= 0 ? args[notesAt + 1] : '';
const bump = args.find((a, i) => !a.startsWith("--") && (notesAt < 0 || i !== notesAt + 1));

const fail = (msg) => {
  console.error(`\nError: ${msg}`);
  process.exit(1);
};
const run = (cmd, argv, opts = {}) => {
  const r = spawnSync(cmd, argv, { cwd: root, stdio: 'inherit', shell: cmd === 'npm' && process.platform === 'win32', ...opts });
  if (r.status !== 0) fail(`"${[cmd, ...argv].join(' ')}" failed`);
};
const out = (cmd, argv) => execFileSync(cmd, argv, { cwd: root, encoding: 'utf8', shell: false }).trim();
const step = (msg) => console.log(`\n== ${msg}`);

function nextVersion(current, how) {
  if (/^\d+\.\d+\.\d+$/.test(how ?? '')) return how;
  const [maj, min, pat] = current.split('.').map(Number);
  if (how === 'major') return `${maj + 1}.0.0`;
  if (how === 'minor') return `${maj}.${min + 1}.0`;
  if (how === 'patch') return `${maj}.${min}.${pat + 1}`;
  return fail('give patch, minor, major or an exact version such as 1.2.0');
}

const readJson = (f) => JSON.parse(fs.readFileSync(path.join(root, f), 'utf8'));
const writeJson = (f, data) => fs.writeFileSync(path.join(root, f), `${JSON.stringify(data, null, 2)}\n`);

const pkg = readJson('package.json');
const version = nextVersion(pkg.version, bump);
const tag = `v${version}`;

step(`Release ${pkg.version} -> ${version}`);

// ---- checks before touching anything ----
if (out('git', ['status', '--porcelain'])) fail('the working tree has uncommitted changes: commit or stash them first');
if (out('git', ['branch', '--show-current']) !== 'main') fail('release from the main branch');
if (out('git', ['tag', '--list', tag])) fail(`the tag ${tag} already exists`);
const hasGh = spawnSync('gh', ['--version'], { shell: false }).status === 0;
if (!hasGh) fail('the GitHub CLI is missing: install it from https://cli.github.com then run "gh auth login"');
if (spawnSync('gh', ['auth', 'status'], { shell: false, stdio: 'ignore' }).status !== 0) {
  fail('not signed in to GitHub: run "gh auth login"');
}

// ---- release notes: commits since the previous tag ----
const previous = out('git', ['tag', '--list', 'v*', '--sort=-v:refname']).split(/\r?\n/).find(Boolean);
const range = previous ? `${previous}..HEAD` : 'HEAD';
const commits = out('git', ['log', range, '--no-merges', '--format=- %s']);
const notes = customNotes || (commits ? `## Changes\n\n${commits}\n` : 'Maintenance release.');
console.log(`\nNotes:\n${notes}`);

if (dryRun) {
  console.log(`\nDry run: nothing changed. A real run would bump to ${version}, test, build, commit, tag ${tag}, push and publish.`);
  process.exit(0);
}

step('Updating version files');
pkg.version = version;
writeJson('package.json', pkg);
const lockFile = path.join(root, 'package-lock.json');
if (fs.existsSync(lockFile)) {
  const lock = readJson('package-lock.json');
  lock.version = version;
  if (lock.packages?.['']) lock.packages[''].version = version;
  writeJson('package-lock.json', lock);
}
const manifest = readJson('extension/manifest.json');
manifest.version = version;
writeJson('extension/manifest.json', manifest);

const undo = () => run('git', ['checkout', '--', 'package.json', 'package-lock.json', 'extension/manifest.json']);
try {
  step('Running the tests');
  run('npm', ['test']);
  step('Building the installer');
  fs.rmSync(path.join(root, 'dist'), { recursive: true, force: true });
  run('npm', ['run', 'dist']);
} catch (err) {
  undo();
  throw err;
}

const installer = path.join(root, 'dist', `Tachy-Download-Setup-${version}.exe`);
if (!fs.existsSync(installer)) {
  undo();
  fail(`the installer was not produced: ${installer}`);
}

step('Committing, tagging and pushing');
run('git', ['add', 'package.json', 'package-lock.json', 'extension/manifest.json']);
run('git', ['commit', '-m', `Release ${tag}`]);
run('git', ['tag', '-a', tag, '-m', `Tachy Download ${version}`]);
run('git', ['push', 'origin', 'main']);
run('git', ['push', 'origin', tag]);

step('Creating the GitHub release');
const notesFile = path.join(root, 'dist', 'release-notes.md');
fs.writeFileSync(notesFile, notes);
run('gh', ['release', 'create', tag, installer, '--title', `Tachy Download ${version}`, '--notes-file', notesFile, '--verify-tag']);

console.log(`\nDone: ${tag} is published. Installed copies will offer the update on their next check.`);
