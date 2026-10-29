'use strict';

const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');

/**
 * Small HTTP origin for tests. `ranges: false` makes it ignore Range headers.
 * `bytesPerSec` slows the response down so a transfer can be interrupted mid-way.
 */
function startServer(body, { ranges = true, bytesPerSec = 0, disposition = null } = {}) {
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push({ url: req.url, range: req.headers.range ?? null });
    const headers = { 'Content-Type': 'application/octet-stream' };
    if (disposition) headers['Content-Disposition'] = disposition;
    let start = 0;
    let end = body.length - 1;
    let status = 200;
    const m = ranges && /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
    if (m) {
      start = Number(m[1]);
      if (m[2] !== '') end = Math.min(Number(m[2]), body.length - 1);
      status = 206;
      headers['Content-Range'] = `bytes ${start}-${end}/${body.length}`;
    }
    if (ranges) headers['Accept-Ranges'] = 'bytes';
    headers['Content-Length'] = end - start + 1;
    res.writeHead(status, headers);
    const slice = body.subarray(start, end + 1);
    if (!bytesPerSec) return res.end(slice);
    const step = Math.max(1, Math.floor(bytesPerSec / 20));
    let off = 0;
    const timer = setInterval(() => {
      if (res.destroyed) return clearInterval(timer);
      res.write(slice.subarray(off, off + step));
      off += step;
      if (off >= slice.length) {
        clearInterval(timer);
        res.end();
      }
    }, 50);
    res.on('close', () => clearInterval(timer));
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () =>
      resolve({
        url: `http://127.0.0.1:${server.address().port}/files/sample.bin`,
        requests,
        close: () => {
          server.closeAllConnections?.();
          return new Promise((r) => server.close(r));
        },
      }),
    ),
  );
}

const tmpDir = () => fs.mkdtempSync(path.join(os.tmpdir(), 'dm-test-'));
const randomBody = (n) => crypto.randomBytes(n);
const sha = (buf) => crypto.createHash('sha256').update(buf).digest('hex');
const waitFor = async (fn, ms = 15000) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error('waitFor: délai dépassé');
};

module.exports = { startServer, tmpDir, randomBody, sha, waitFor };
