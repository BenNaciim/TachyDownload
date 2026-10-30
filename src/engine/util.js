'use strict';

const path = require('node:path');
const fs = require('node:fs');

const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

function sleep(ms, signal) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(signal.reason ?? new Error('aborted'));
    const t = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(t);
      reject(signal.reason ?? new Error('aborted'));
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

function sanitizeFilename(name) {
  let n = String(name ?? '')
    .replace(/[\\/]+/g, '_')
    .replace(/[<>:"|?*\u0000-\u001f]/g, '_')
    .replace(/^[.\s]+|[.\s]+$/g, '')
    .slice(0, 180);
  if (!n) n = 'download';
  if (RESERVED.test(n)) n = '_' + n;
  return n;
}

function safeDecode(s) {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

function filenameFromContentDisposition(header) {
  if (!header) return null;
  const star = /filename\*\s*=\s*([^']*)'[^']*'([^;]+)/i.exec(header);
  if (star) return safeDecode(star[2].trim().replace(/^"|"$/g, ''));
  const quoted = /filename\s*=\s*"((?:[^"\\]|\\.)*)"/i.exec(header);
  if (quoted) return quoted[1].replace(/\\(.)/g, '$1');
  const plain = /filename\s*=\s*([^;]+)/i.exec(header);
  return plain ? plain[1].trim() : null;
}

const CATEGORIES = [
  ['Vidéos', ['mp4', 'mkv', 'avi', 'mov', 'webm', 'wmv', 'flv', 'm4v', 'ts']],
  ['Musique', ['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a', 'opus']],
  ['Images', ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'avif', 'tif', 'tiff']],
  ['Documents', ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'odt', 'csv', 'epub']],
  ['Archives', ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz']],
  ['Programmes', ['exe', 'msi', 'msix', 'iso', 'dmg', 'apk', 'deb', 'rpm', 'img']],
];

/** Folder name for a file type: every type gets one, unknown ones go to "Autres". */
function categoryFor(filename) {
  const ext = path.extname(filename).slice(1).toLowerCase();
  return CATEGORIES.find(([, exts]) => exts.includes(ext))?.[0] ?? 'Autres';
}

function filenameFromUrl(url) {
  try {
    const last = new URL(url).pathname.split('/').filter(Boolean).pop();
    return last ? safeDecode(last) : null;
  } catch {
    return null;
  }
}

/** Returns `dir/name`, or `dir/name (1).ext` etc. when the path is taken. */
function uniquePath(dir, name, { ignorePart = false } = {}) {
  const ext = path.extname(name);
  const base = ext ? name.slice(0, -ext.length) : name;
  let candidate = path.join(dir, name);
  const taken = (p) => fs.existsSync(p) || (!ignorePart && fs.existsSync(p + '.part'));
  for (let i = 1; taken(candidate); i++) {
    candidate = path.join(dir, `${base} (${i})${ext}`);
  }
  return candidate;
}

/** Turns low-level network and disk errors into a sentence the user can act on. */
function friendlyError(err) {
  const code = err?.cause?.code ?? err?.code;
  switch (code) {
    case 'ENOTFOUND':
    case 'EAI_AGAIN':
      return 'Adresse introuvable. Vérifiez le lien et votre connexion.';
    case 'ECONNREFUSED':
      return 'Le serveur refuse la connexion.';
    case 'ETIMEDOUT':
    case 'UND_ERR_CONNECT_TIMEOUT':
    case 'UND_ERR_HEADERS_TIMEOUT':
      return 'Le serveur ne répond pas.';
    case 'ECONNRESET':
    case 'UND_ERR_SOCKET':
      return 'La connexion a été interrompue.';
    case 'ENOSPC':
      return 'Le disque est plein.';
    case 'EACCES':
    case 'EPERM':
      return 'Accès refusé au dossier de destination.';
    case 'ERR_INVALID_URL':
      return 'Le lien est invalide.';
    default:
      break;
  }
  if (err?.cause?.message === 'bad port') return 'Ce port est bloqué par mesure de sécurité.';
  if (err?.message === 'fetch failed') return 'Connexion impossible au serveur.';
  return err?.message || String(err);
}

module.exports = {
  categoryFor,
  CATEGORIES,
  friendlyError,
  sleep,
  sanitizeFilename,
  filenameFromContentDisposition,
  filenameFromUrl,
  uniquePath,
};
