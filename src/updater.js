'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');

const REPO = 'BenNaciim/TachyDownload';
const API = 'https://api.github.com';
const INSTALLER = /^Tachy-Download-Setup-.+\.exe$/i;

/** 1 when a is newer than b, -1 when older, 0 when equal ("v1.2.3" and "1.2.3" are the same). */
function compareVersions(a, b) {
  const parts = (v) => String(v).replace(/^v/i, '').split('-')[0].split('.').map((n) => Number.parseInt(n, 10) || 0);
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d) return d > 0 ? 1 : -1;
  }
  return 0;
}

/**
 * Looks for a newer release. Resolves to null when there is none, when the repository has no public
 * release (a private repository answers 404), or when the release carries no installer.
 */
async function checkForUpdate({ current, repo = REPO, api = API, fetchImpl = fetch } = {}) {
  const res = await fetchImpl(`${api}/repos/${repo}/releases/latest`, {
    headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'TachyDownload-Updater' },
    signal: AbortSignal.timeout(15_000),
  });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`GitHub a répondu ${res.status}`);
  const rel = await res.json();
  if (!rel || rel.draft || rel.prerelease || !rel.tag_name) return null;
  if (compareVersions(rel.tag_name, current) <= 0) return null;
  const asset = (rel.assets ?? []).find((a) => INSTALLER.test(a.name ?? ''));
  if (!asset?.browser_download_url) return null;
  const digest = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest ?? '');
  return {
    version: String(rel.tag_name).replace(/^v/i, ''),
    url: asset.browser_download_url,
    size: Number(asset.size) || 0,
    sha256: digest ? digest[1].toLowerCase() : null,
    page: rel.html_url || `https://github.com/${repo}/releases`,
  };
}

/**
 * Downloads the installer and checks its SHA-256 against the one GitHub published for that file.
 * Without that checksum nothing is run: the caller can still send the user to the release page.
 */
async function downloadUpdate(info, dir, { onProgress, fetchImpl = fetch, allowHost = /(^|\.)(github\.com|githubusercontent\.com)$/i } = {}) {
  const u = new URL(info.url);
  if (u.protocol !== 'https:' && u.hostname !== '127.0.0.1') throw new Error('Adresse de mise à jour refusée');
  if (u.hostname !== '127.0.0.1' && !allowHost.test(u.hostname)) throw new Error('Adresse de mise à jour refusée');
  if (!info.sha256) throw new Error('Somme de contrôle absente : installation automatique refusée');

  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `Tachy-Download-Setup-${info.version.replace(/[^\w.-]/g, '')}.exe`);
  const part = `${file}.part`;
  const res = await fetchImpl(info.url, { headers: { 'User-Agent': 'TachyDownload-Updater' }, redirect: 'follow' });
  if (!res.ok || !res.body) throw new Error(`Téléchargement impossible (${res.status})`);
  const total = Number(res.headers.get('content-length')) || info.size || 0;
  const hash = crypto.createHash('sha256');
  let got = 0;
  const body = Readable.fromWeb(res.body);
  body.on('data', (chunk) => {
    hash.update(chunk);
    got += chunk.length;
    onProgress?.(got, total);
  });
  try {
    await pipeline(body, fs.createWriteStream(part));
    if (hash.digest('hex') !== info.sha256) throw new Error('Somme de contrôle incorrecte : fichier rejeté');
    fs.renameSync(part, file);
  } catch (err) {
    fs.rmSync(part, { force: true });
    throw err;
  }
  return file;
}

module.exports = { checkForUpdate, downloadUpdate, compareVersions, REPO };
