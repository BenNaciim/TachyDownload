'use strict';

const http = require('node:http');
const crypto = require('node:crypto');

const MAX_BODY = 64 * 1024;
const ALLOWED_ORIGIN = /^(chrome|moz|edge)-extension:\/\/[a-z0-9-]+$/i;

function sameToken(a, b) {
  const x = Buffer.from(String(a ?? ''));
  const y = Buffer.from(String(b ?? ''));
  return x.length === y.length && crypto.timingSafeEqual(x, y);
}

/**
 * Loopback API used by the browser extension. Three checks guard it:
 * the Host header (blocks DNS rebinding), the Origin (only extensions), and a
 * secret token that the user copies from the app into the extension once.
 */
function createExtensionServer({ manager, token, onAdded = () => {}, onRequest = null }) {
  let port = 0;

  const server = http.createServer((req, res) => {
    const origin = req.headers.origin;
    const send = (status, obj) => {
      const headers = { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' };
      if (origin && ALLOWED_ORIGIN.test(origin)) {
        headers['Access-Control-Allow-Origin'] = origin;
        headers.Vary = 'Origin';
      }
      res.writeHead(status, headers);
      res.end(JSON.stringify(obj));
    };

    const host = req.headers.host ?? '';
    if (host !== `127.0.0.1:${port}` && host !== `localhost:${port}`) {
      return send(403, { error: 'host' });
    }
    if (origin && !ALLOWED_ORIGIN.test(origin)) return send(403, { error: 'origin' });

    if (req.method === 'OPTIONS') {
      res.writeHead(204, {
        ...(origin ? { 'Access-Control-Allow-Origin': origin, Vary: 'Origin' } : {}),
        'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
        'Access-Control-Allow-Headers': 'Content-Type, X-DM-Token',
        'Access-Control-Max-Age': '600',
      });
      return res.end();
    }

    const url = new URL(req.url, 'http://x');
    if (req.method === 'GET' && url.pathname === '/ping') {
      return send(200, { ok: true, app: 'tachy-download' });
    }

    if (!sameToken(req.headers['x-dm-token'], token())) return send(401, { error: 'token' });

    if (req.method === 'GET' && url.pathname === '/refresh-requests') {
      return send(200, { requests: manager.refreshRequests() });
    }

    if (req.method === 'POST' && (url.pathname === '/add' || url.pathname === '/refresh')) {
      let size = 0;
      const chunks = [];
      req.on('data', (c) => {
        size += c.length;
        if (size > MAX_BODY) {
          send(413, { error: 'too large' });
          req.destroy();
        } else {
          chunks.push(c);
        }
      });
      req.on('end', () => {
        if (res.writableEnded) return;
        try {
          const body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
          const str = (v) => (typeof v === 'string' ? v.slice(0, 8192) : '');
          const common = {
            referer: str(body.referer),
            cookie: str(body.cookie),
            userAgent: str(body.userAgent),
          };
          if (url.pathname === '/refresh') {
            const task = manager.refresh(str(body.id), { url: str(body.url), ...common });
            return send(200, { id: task.id });
          }
          const opts = {
            ...common,
            filename: str(body.filename),
            kind: ['hls', 'ytdlp'].includes(body.kind) ? body.kind : undefined,
            sniffed: body.sniffed === true,
          };
          // The app may want to ask the user (destination, now or later) before adding anything.
          if (onRequest) {
            if (!/^https?:\/\//i.test(str(body.url))) throw new Error('Adresse invalide');
            const size = Number(body.size) > 0 ? Math.round(Number(body.size)) : 0;
            if (onRequest({ url: str(body.url), size, ...opts })) return send(200, { pending: true });
          }
          const task = manager.add(str(body.url), opts);
          onAdded(task);
          send(200, { id: task.id });
        } catch (err) {
          send(400, { error: err.message });
        }
      });
      return undefined;
    }
    return send(404, { error: 'not found' });
  });

  return {
    /** Tries `preferred`, then the next ports, and resolves with the one that worked. */
    listen(preferred = 6800, attempts = 10) {
      return new Promise((resolve, reject) => {
        let candidate = preferred;
        const tryListen = () => {
          server.once('error', (err) => {
            if (err.code === 'EADDRINUSE' && candidate < preferred + attempts - 1) {
              candidate++;
              tryListen();
            } else {
              reject(err);
            }
          });
          server.listen(candidate, '127.0.0.1', () => {
            port = server.address().port;
            server.removeAllListeners('error');
            resolve(port);
          });
        };
        tryListen();
      });
    },
    close: () =>
      new Promise((r) => {
        server.closeAllConnections?.();
        server.close(() => r());
      }),
    get port() {
      return port;
    },
  };
}

module.exports = { createExtensionServer };
