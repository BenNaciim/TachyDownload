'use strict';
// Idle memory and cold-start time of the desktop app, Electron or Tauri, with the same protocol (design §7.3).
//   node tools/bench/measure-app.js electron "dist\win-unpacked\Tachy Download.exe" [runs=3] [idleSeconds=60]
//   node tools/bench/measure-app.js tauri "target\release\tachy-download.exe"
// Per run: a fresh data dir, the app launched with a DevTools port, then
//   cold start = process spawn -> the interface has the app state (`dm.state()` resolved; `dm.list()` follows at once)
//   memory     = private working set summed over the whole process tree, `idleSeconds` after the launch,
//                window visible, no download (the same figure Task Manager shows per process)
//   idle CPU   = CPU time of the whole process tree over the second half of the idle window, as % of one core
// APPDATA points at the temp dir too, so neither app reads the user's real data (Electron's old-folder migration).
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { connect, sleep } = require('../lib/cdp');

const [kind, exe, runsArg = '3', idleArg = '60'] = process.argv.slice(2);
if (!['electron', 'tauri'].includes(kind) || !exe) {
  console.error('usage: node tools/bench/measure-app.js <electron|tauri> <exe> [runs] [idleSeconds]');
  process.exit(2);
}

const freePort = () =>
  new Promise((resolve) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

const ps = (script) =>
  JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8', maxBuffer: 1 << 26 }) || '[]');
const asArray = (v) => (Array.isArray(v) ? v : [v]);

/** Private working set (bytes) of `rootPid` and all its descendants, and how many processes that is. */
function treeMemory(rootPid) {
  const procs = asArray(ps('Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,KernelModeTime,UserModeTime | ConvertTo-Json -Compress'));
  const mem = asArray(ps('Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Select-Object IDProcess,WorkingSetPrivate | ConvertTo-Json -Compress'));
  const children = new Map();
  for (const p of procs) children.set(p.ParentProcessId, [...(children.get(p.ParentProcessId) ?? []), p.ProcessId]);
  const tree = new Set([rootPid]);
  for (const pid of tree) for (const c of children.get(pid) ?? []) tree.add(c);
  const byPid = new Map(mem.map((m) => [m.IDProcess, Number(m.WorkingSetPrivate)]));
  const cpu = new Map(procs.map((p) => [p.ProcessId, (Number(p.KernelModeTime) + Number(p.UserModeTime)) / 1e7]));
  let bytes = 0;
  let count = 0;
  let cpuSeconds = 0;
  for (const pid of tree) if (byPid.has(pid)) (bytes += byPid.get(pid)), (count += 1), (cpuSeconds += cpu.get(pid) ?? 0);
  return { bytes, count, cpuSeconds };
}

const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

async function once(idleSeconds) {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tachy-measure-'));
  const port = await freePort();
  const args = kind === 'electron' ? [`--user-data-dir=${dataDir}`, `--remote-debugging-port=${port}`] : [`--data-dir=${dataDir}`];
  const env = { ...process.env, APPDATA: dataDir, TACHY_FAKE_IDLE: '1' }; // the last one only matters to the M0 fake core
  if (kind === 'tauri') env.TACHY_DEBUG_PORT = String(port);
  const t0 = process.hrtime.bigint();
  const child = spawn(exe, args, { stdio: 'ignore', env });
  try {
    const page = await connect(port, { urlPart: 'index.html', waitMs: 60000 });
    for (;;) {
      if (await page.evaluate("typeof appState !== 'undefined' && appState !== null")) break;
      await sleep(10);
    }
    const coldMs = Number(process.hrtime.bigint() - t0) / 1e6;
    const wait = idleSeconds * 1000 - coldMs;
    if (wait > 0) await sleep(wait / 2);
    const mid = treeMemory(child.pid);
    const midAt = Date.now();
    if (wait > 0) await sleep(wait / 2);
    page.close();
    const end = treeMemory(child.pid);
    const cpuPct = ((end.cpuSeconds - mid.cpuSeconds) / ((Date.now() - midAt) / 1000)) * 100;
    return { coldMs, ...end, cpuPct };
  } finally {
    try {
      execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {}
    await sleep(1500);
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

(async () => {
  const runs = [];
  for (let i = 0; i < Number(runsArg); i++) {
    const r = await once(Number(idleArg));
    runs.push(r);
    console.log(`run ${i + 1}: cold start ${Math.round(r.coldMs)} ms, ${(r.bytes / 1048576).toFixed(1)} MiB in ${r.count} processes, idle CPU ${r.cpuPct.toFixed(1)} %`);
  }
  console.log(`MEDIAN ${kind}: cold start ${Math.round(median(runs.map((r) => r.coldMs)))} ms, memory ${(median(runs.map((r) => r.bytes)) / 1048576).toFixed(1)} MiB, idle CPU ${median(runs.map((r) => r.cpuPct)).toFixed(1)} %`);
})();
