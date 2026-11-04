'use strict';
// Local file server for the throughput bench: a body of SIZE_MB made of one repeated block, `Range` supported.
// It runs in its own process so its CPU is not counted against the client being measured.
//   node tools/bench/server.js     prints "PORT <n>" once it listens, then serves until killed
//   GET /__stats                   {"connections": <TCP connections accepted>, "requests": <requests served>}
// Used by tools/bench/bench-js.js (current engine) and by the Rust bench against the same server.
const http = require('node:http');

const SIZE_MB = Number(process.env.SIZE_MB || 1024);
const SIZE = SIZE_MB * 1024 * 1024;
const chunk = Buffer.alloc(4 << 20, 0x5a);

let connections = 0;
let requests = 0;

const server = http.createServer(async (req, res) => {
  if (req.url === '/__stats') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ connections, requests }));
  }
  requests += 1;
  let wake;
  res.once('close', () => wake?.());
  let start = 0;
  let end = SIZE - 1;
  let status = 200;
  const h = { 'Content-Type': 'application/octet-stream', 'Accept-Ranges': 'bytes' };
  const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '');
  if (m) {
    start = Number(m[1]);
    if (m[2] !== '') end = Math.min(Number(m[2]), SIZE - 1);
    status = 206;
    h['Content-Range'] = `bytes ${start}-${end}/${SIZE}`;
  }
  h['Content-Length'] = end - start + 1;
  res.writeHead(status, h);
  if (req.method === 'HEAD') return res.end();
  let left = end - start + 1;
  while (left > 0 && !res.destroyed) {
    const n = Math.min(left, chunk.length);
    left -= n;
    if (!res.write(n === chunk.length ? chunk : chunk.subarray(0, n))) await new Promise((r) => ((wake = r), res.once('drain', r)));
  }
  return res.destroyed ? undefined : res.end();
});
server.on('connection', () => (connections += 1));

server.listen(Number(process.env.PORT || 0), '127.0.0.1', () => {
  console.log('PORT ' + server.address().port);
});
