'use strict';
// Throughput bench of the current (Node) engine: how fast it pulls a file from the local server, and at what CPU cost.
//   node tools/bench/bench-js.js                      1 GiB, 1 / 8 / 16 connections
//   SIZE_MB=256 CONNS=8 node tools/bench/bench-js.js
// `curl` (C) on the same server gives the ceiling of what one stream can do on this machine.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const SIZE_MB = Number(process.env.SIZE_MB || 1024);
const SIZE = SIZE_MB * 1024 * 1024;

(async () => {
  const srv = spawn(process.execPath, [path.join(__dirname, 'server.js')], {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: { ...process.env, SIZE_MB: String(SIZE_MB) },
  });
  const port = await new Promise((resolve) =>
    srv.stdout.on('data', (d) => {
      const m = /PORT (\d+)/.exec(String(d));
      if (m) resolve(Number(m[1]));
    }),
  );
  const { DownloadManager } = require('../../src/engine/manager');
  const base = fs.mkdtempSync(path.join(os.tmpdir(), 'tachy-bench-'));
  const rows = [];
  for (const connections of (process.env.CONNS || '1,8,16').split(',').map(Number)) {
    const dir = fs.mkdtempSync(path.join(base, 'run-'));
    const m = new DownloadManager({ settings: { dir, connections }, tickMs: 100 });
    m.start();
    const cpu0 = process.cpuUsage();
    const t0 = process.hrtime.bigint();
    const t = m.add(`http://127.0.0.1:${port}/big.bin`, { filename: `big-${connections}.bin` });
    const samples = [];
    let final;
    for (;;) {
      await new Promise((r) => setTimeout(r, 100));
      const secs = Number(process.hrtime.bigint() - t0) / 1e9;
      const s = m.list().find((x) => x.id === t.id);
      samples.push([secs, s.downloaded ?? 0]);
      if (s.status === 'completed' || s.status === 'error') {
        final = s;
        break;
      }
      if (secs > 150) {
        final = { status: 'timeout' };
        break;
      }
    }
    const wall = Number(process.hrtime.bigint() - t0) / 1e9;
    const cpu = process.cpuUsage(cpu0);
    // steady rate between 10 % and 90 % of the file, so setup and finalisation are left out
    const a = samples.find(([, b]) => b >= SIZE * 0.1);
    const z = samples.find(([, b]) => b >= SIZE * 0.9);
    const steady = a && z && z[0] > a[0] ? (z[1] - a[1]) / (z[0] - a[0]) / 1e6 : NaN;
    rows.push({
      connexions: connections,
      statut: final.status,
      'durée (s)': +wall.toFixed(2),
      'débit moyen (Mo/s)': +(SIZE / wall / 1e6).toFixed(0),
      'débit stable (Mo/s)': Number.isNaN(steady) ? 'n/a' : +steady.toFixed(0),
      'CPU user (s)': +(cpu.user / 1e6).toFixed(2),
      'CPU noyau (s)': +(cpu.system / 1e6).toFixed(2),
      'CPU (% d’un cœur)': +(((cpu.user + cpu.system) / 1e6 / wall) * 100).toFixed(0),
    });
    await m.shutdown();
  }
  console.table(rows);
  console.log('RSS max du processus moteur (Mo):', Math.round(process.resourceUsage().maxRSS / 1024));
  // Reference ceiling: curl, same server, one stream, to NUL (no disk) then to a file
  for (const [label, out] of [['curl -> NUL (pas de disque)', 'NUL'], ['curl -> fichier', path.join(base, 'curl.bin')]]) {
    const r = spawnSync('curl.exe', ['-s', '-o', out, '-w', '%{speed_download} %{time_total}', `http://127.0.0.1:${port}/big.bin`], { encoding: 'utf8' });
    const [speed, secs] = String(r.stdout).trim().split(' ').map(Number);
    console.log(label, Number.isFinite(speed) ? `${Math.round(speed / 1e6)} Mo/s en ${secs.toFixed(2)} s` : `échec: ${r.stderr || r.error}`);
  }
  srv.kill();
  fs.rmSync(base, { recursive: true, force: true });
})();
