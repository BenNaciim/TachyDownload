'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFile } = require('node:child_process');

/** SHA-256 of a file, read as a stream so a large file never sits in memory. */
function fileHash(file) {
  return new Promise((resolve, reject) => {
    const h = crypto.createHash('sha256');
    fs.createReadStream(file).on('data', (c) => h.update(c)).on('error', reject).on('end', () => resolve(h.digest('hex')));
  });
}

/**
 * Looks among the other finished downloads for a file with the same content. Sizes are compared first, so
 * only files of the same size are ever read. Returns the path of the earlier copy, or null.
 */
async function findDuplicate(file, others) {
  let size;
  try {
    size = fs.statSync(file).size;
  } catch {
    return null;
  }
  if (size === 0) return null;
  const same = others.filter((p) => {
    try {
      return path.resolve(p) !== path.resolve(file) && fs.statSync(p).size === size;
    } catch {
      return false;
    }
  });
  if (same.length === 0) return null;
  const mine = await fileHash(file);
  for (const p of same) {
    if ((await fileHash(p).catch(() => null)) === mine) return p;
  }
  return null;
}

/**
 * The tar that ships with Windows 10 and later (bsdtar, which reads zip and 7z). A tar earlier in the PATH,
 * such as the one from Git, is a different program that reads "C:" as a host name, so it is not used.
 */
function systemTar() {
  const own = process.env.SystemRoot && path.join(process.env.SystemRoot, 'System32', 'tar.exe');
  return own && fs.existsSync(own) ? own : 'tar';
}

const ARCHIVE = /\.(zip|7z|tar|tgz|tar\.gz|tar\.xz|tar\.bz2)$/i;
const isArchive = (name) => ARCHIVE.test(name);

/** An archive's folder name: the file name without its archive extension. */
const archiveStem = (name) => name.replace(ARCHIVE, '');

/**
 * Unpacks an archive into a new folder named after it, next to the archive, with the tar that ships with
 * Windows 10 and later (it reads zip, 7z and the common tar flavours). It refuses paths that climb out of
 * the folder. Returns the folder it created.
 */
function extractArchive(file, { tar = systemTar(), timeoutMs = 10 * 60 * 1000 } = {}) {
  const dir = path.dirname(file);
  const stem = archiveStem(path.basename(file)) || 'archive';
  let dest = path.join(dir, stem);
  for (let i = 1; fs.existsSync(dest); i++) dest = path.join(dir, `${stem} (${i})`);
  fs.mkdirSync(dest);
  return new Promise((resolve, reject) => {
    execFile(tar, ['-xf', file, '-C', dest], { windowsHide: true, timeout: timeoutMs }, (err, _out, stderr) => {
      if (err) {
        fs.rmSync(dest, { recursive: true, force: true });
        reject(new Error(String(stderr || err.message).trim().split(/\r?\n/)[0].slice(0, 200) || 'Extraction impossible'));
      } else resolve(dest);
    });
  });
}

module.exports = { fileHash, findDuplicate, isArchive, extractArchive, systemTar };
