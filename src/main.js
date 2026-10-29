'use strict';

const { app, BrowserWindow, ipcMain, dialog, shell, clipboard, Tray, Menu, nativeImage, Notification } = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const crypto = require('node:crypto');
const { DownloadManager } = require('./engine/manager');
const { createExtensionServer } = require('./server');
const { Tools } = require('./engine/tools');
const { isIdle, commandFor } = require('./afterall');
const { isPlaylistUrl } = require('./engine/ytdlp');
const { categoryFor, filenameFromUrl, sanitizeFilename } = require('./engine/util');
const { execFile } = require('node:child_process');

// The installer runs the app once with --setup-tools to fetch yt-dlp and ffmpeg: no window, no lock.
const setupTools = process.argv.includes('--setup-tools');
if (!setupTools && !app.requestSingleInstanceLock()) {
  app.quit();
}

let win = null;
let manager = null;
let api = null;
let tools = null;
let tray = null;
const startHidden = process.argv.includes('--hidden');

function loadToken(dir) {
  const file = path.join(dir, 'extension-token.txt');
  try {
    const t = fs.readFileSync(file, 'utf8').trim();
    if (t.length >= 32) return t;
  } catch {
    // first run: fall through and create one
  }
  const token = crypto.randomBytes(24).toString('hex');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, token, { mode: 0o600 });
  return token;
}

function send(channel, payload) {
  if (win && !win.isDestroyed()) win.webContents.send(channel, payload);
}

function taskPath(id) {
  const t = manager.list().find((x) => x.id === id);
  if (!t) throw new Error('Téléchargement introuvable');
  const finished = path.join(t.dir, t.filename);
  return fs.existsSync(finished) ? finished : path.join(t.dir, t.filename + '.part');
}

// Windows resolves the icon from the file extension (the program that opens it).
// Programs and installers carry their own icon, so those use the real file once it exists.
const iconCache = new Map();
const OWN_ICON_EXT = new Set(['exe', 'msi', 'scr', 'ico']);

async function fileIconFor(task) {
  let ext = path.extname(task.filename).slice(1).toLowerCase();
  if (!/^[a-z0-9]{1,10}$/.test(ext)) ext = '';
  const real = path.join(task.dir, task.filename);
  const own = task.status === 'completed' && OWN_ICON_EXT.has(ext) && fs.existsSync(real);

  let target = real;
  let key = `path:${real}`;
  if (!own) {
    // An empty stand-in with the right extension: only the extension matters for the lookup.
    const probeDir = path.join(app.getPath('userData'), 'icon-probe');
    target = path.join(probeDir, `type.${ext || 'file'}`);
    key = `ext:${ext}`;
    if (!iconCache.has(key)) {
      fs.mkdirSync(probeDir, { recursive: true });
      if (!fs.existsSync(target)) fs.writeFileSync(target, '');
    }
  }
  if (!iconCache.has(key)) {
    try {
      const img = await app.getFileIcon(target, { size: 'large' });
      iconCache.set(key, img.isEmpty() ? null : img.toDataURL());
    } catch {
      iconCache.set(key, null);
    }
  }
  return iconCache.get(key);
}

function registerIpc(token) {
  const str = (v, max = 8192) => {
    if (typeof v !== 'string' || v.length > max) throw new Error('Paramètre invalide');
    return v;
  };

  ipcMain.handle('dm:list', () => manager.list());
  ipcMain.handle('dm:state', () => ({
    settings: manager.settings,
    port: api.port,
    token,
    glass: glassSupported(),
  }));

  ipcMain.handle('dm:add', async (_e, urls, opts) => {
    if (!Array.isArray(urls) || urls.length === 0 || urls.length > 200) {
      throw new Error('Liste de liens invalide');
    }
    const startAt = Number(opts?.startAt) || null;
    const kind = opts?.kind === 'ytdlp' ? 'ytdlp' : undefined;
    const added = [];
    const errors = [];
    for (const raw of urls) {
      try {
        if (isPlaylistUrl(raw)) added.push(...(await manager.addPlaylist(str(raw), { startAt })));
        else added.push(manager.add(str(raw), { startAt, kind }));
      } catch (err) {
        errors.push({ url: String(raw).slice(0, 200), error: err.message });
      }
    }
    return { added: added.length, errors };
  });

  ipcMain.handle('dm:openPage', (_e, id) => {
    const task = manager.list().find((x) => x.id === str(id, 100));
    if (!task?.pageUrl || !/^https?:\/\//i.test(task.pageUrl)) throw new Error('Page d’origine inconnue');
    return shell.openExternal(task.pageUrl);
  });

  ipcMain.handle('dm:fileIcon', (_e, id) => {
    const task = manager.list().find((x) => x.id === str(id, 100));
    return task ? fileIconFor(task) : null;
  });

  ipcMain.handle('dm:tools:status', () => tools.status());
  ipcMain.handle('dm:tools:install', async (_e, name) => {
    if (name !== 'ytdlp' && name !== 'ffmpeg') throw new Error('Outil inconnu');
    await tools.install(name);
    return tools.status();
  });

  ipcMain.handle('dm:pause',(_e, id) => manager.pause(str(id, 100)));
  ipcMain.handle('dm:resume', (_e, id) => manager.resume(str(id, 100)));
  ipcMain.handle('dm:remove', (_e, id, deleteFile) =>
    manager.remove(str(id, 100), { deleteFile: deleteFile === true }),
  );
  ipcMain.handle('dm:reveal', (_e, id) => shell.showItemInFolder(taskPath(str(id, 100))));
  ipcMain.handle('dm:open', async (_e, id) => {
    const p = taskPath(str(id, 100));
    if (p.endsWith('.part')) throw new Error('Le téléchargement n’est pas terminé');
    const err = await shell.openPath(p);
    if (err) throw new Error(err);
  });

  ipcMain.handle('dm:settings:set', (_e, patch) => {
    if (typeof patch !== 'object' || patch === null) throw new Error('Paramètres invalides');
    const { dir, connections, maxConcurrent, speedLimitKBps, schedule, videoMaxHeight, convertToMp4, categories, autoStart, closeToTray, notifications, confirmDownloads, afterAll } = patch;
    const next = manager.setSettings({
      dir, connections, maxConcurrent, speedLimitKBps, schedule, videoMaxHeight, convertToMp4, categories, autoStart, closeToTray,
      notifications, confirmDownloads, afterAll,
    });
    applyAutoStart();
    return next;
  });
  ipcMain.handle('dm:pickDir', async () => {
    const r = await dialog.showOpenDialog(win, {
      properties: ['openDirectory', 'createDirectory'],
      defaultPath: manager.settings.dir,
    });
    return r.canceled ? null : r.filePaths[0];
  });
  ipcMain.handle('dm:copyToken', () => clipboard.writeText(token));

  const entryOf = (e) => confirms.get(e.sender.id);
  ipcMain.handle('dm:confirm:init', async (e) => {
    const entry = entryOf(e);
    if (!entry) return null;
    const { req } = entry;
    let host = '';
    try {
      host = new URL(req.url).hostname;
    } catch {
      // the server already validated the address
    }
    return {
      url: req.url,
      kind: req.kind,
      size: req.size,
      sniffed: Boolean(req.sniffed),
      filename: entry.initialName,
      dir: manager.settings.dir,
      host,
      icon: await fileIconFor({ filename: entry.initialName || 'file.bin', dir: '', status: 'queued' }),
    };
  });
  ipcMain.handle('dm:confirm:category', (e, name) =>
    entryOf(e) && manager.settings.categories && typeof name === 'string' ? categoryFor(name) : null,
  );
  ipcMain.handle('dm:confirm:pickDir', async (e, current) => {
    const entry = entryOf(e);
    if (!entry) return null;
    const r = await dialog.showOpenDialog(entry.win, {
      properties: ['openDirectory', 'createDirectory'],
      defaultPath: typeof current === 'string' && current ? current : manager.settings.dir,
    });
    return r.canceled ? null : r.filePaths[0];
  });
  ipcMain.handle('dm:confirm:answer', (e, a) => {
    const entry = entryOf(e);
    if (!entry || entry.done || typeof a !== 'object' || a === null) return undefined;
    return finishConfirm(entry, ['now', 'later'].includes(a.action) ? a : { action: 'cancel' });
  });
}

const lastStatus = new Map();

/** Reacts when a download finishes or fails: desktop notification, then the end-of-queue action. */
function onStatusChange(snap) {
  const prev = lastStatus.get(snap.id);
  lastStatus.set(snap.id, snap.status);
  if (!prev || prev === snap.status || !['completed', 'error'].includes(snap.status)) return;

  const visible = win && win.isVisible() && win.isFocused();
  if (manager.settings.notifications && !visible && Notification.isSupported()) {
    const n = new Notification({
      title: snap.status === 'completed' ? 'Téléchargement terminé' : 'Téléchargement échoué',
      body: snap.status === 'completed' ? snap.filename : `${snap.filename} : ${snap.error ?? ''}`,
      icon: path.join(__dirname, '..', 'build', 'icon.png'),
    });
    n.on('click', () => {
      if (snap.status === 'completed') shell.showItemInFolder(path.join(snap.dir, snap.filename));
      else showWindow();
    });
    n.show();
  }

  const action = manager.settings.afterAll;
  if (action !== 'none' && isIdle(manager.list())) runAfterAll(action);
}

async function runAfterAll(action) {
  manager.setSettings({ afterAll: 'none' }); // one-shot: never fires twice by surprise
  manager.saveNow();
  if (action === 'quit') {
    app.isQuitting = true;
    return app.quit();
  }
  const command = commandFor(action);
  if (!command) return undefined;
  execFile(command.cmd, command.args, { windowsHide: true }, () => {});
  if (action === 'shutdown') {
    const r = await dialog.showMessageBox({
      type: 'warning',
      title: 'Tachy Download',
      message: 'Les téléchargements sont terminés : arrêt de l’ordinateur dans 60 secondes.',
      buttons: ['Annuler l’arrêt', 'OK'],
      defaultId: 0,
      cancelId: 1,
    });
    if (r.response === 0) execFile('shutdown', ['/a'], { windowsHide: true }, () => {});
  }
  return undefined;
}

// Acrylic backgrounds need Windows 11 22H2 (build 22621) or later.
function glassSupported() {
  const build = Number(/^10\.0\.(\d+)/.exec(require('node:os').release())?.[1] ?? 0);
  return process.platform === 'win32' && build >= 22621;
}

function createWindow() {
  win = new BrowserWindow({
    width: 1200,
    height: 720,
    minWidth: 760,
    minHeight: 460,
    title: 'Tachy Download',
    icon: path.join(__dirname, '..', 'build', 'icon.png'),
    backgroundColor: glassSupported() ? '#00000000' : '#f1f5fd',
    ...(glassSupported() ? { backgroundMaterial: 'acrylic' } : {}),
    show: !startHidden,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });
  win.setMenuBarVisibility(false);
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  win.loadFile(path.join(__dirname, 'renderer', 'index.html'));
  // The close button keeps downloads running in the notification area (unless turned off).
  win.on('close', (e) => {
    if (!app.isQuitting && manager?.settings.closeToTray && tray) {
      e.preventDefault();
      win.hide();
    }
  });
  win.on('closed', () => {
    win = null;
  });
}

function showWindow() {
  if (!win) createWindow();
  if (win.isMinimized()) win.restore();
  win.show();
  win.focus();
}

// ---- "Download this?" window: destination, now or later (like IDM's confirmation) ----------

const confirms = new Map(); // webContents id -> { req, win, initialName, done }

/** Opens the confirmation window for a download handed over by the browser. */
function askAboutDownload(req) {
  const kind = req.kind ?? 'file';
  const filename = sanitizeFilename(req.filename || (kind === 'ytdlp' ? '' : filenameFromUrl(req.url)) || (kind === 'hls' ? 'video' : 'download'));
  const offset = (confirms.size % 6) * 28;
  const w = new BrowserWindow({
    width: 520,
    height: 470,
    x: undefined,
    y: undefined,
    resizable: false,
    minimizable: false,
    maximizable: false,
    title: 'Tachy Download',
    icon: path.join(__dirname, '..', 'build', 'icon.png'),
    autoHideMenuBar: true,
    show: false,
    alwaysOnTop: true,
    webPreferences: {
      preload: path.join(__dirname, 'confirm-preload.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });
  w.setMenuBarVisibility(false);
  w.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  w.webContents.on('will-navigate', (e) => e.preventDefault());
  const id = w.webContents.id;
  const entry = { req: { ...req, kind }, win: w, initialName: filename, done: false };
  confirms.set(id, entry);
  w.once('ready-to-show', () => {
    const [x, y] = w.getPosition();
    w.setPosition(x + offset, y + offset);
    w.show();
    w.focus();
  });
  w.on('closed', () => confirms.delete(id)); // closing without answering cancels
  w.loadFile(path.join(__dirname, 'renderer', 'confirm.html'));
  return true;
}

async function finishConfirm(entry, a) {
  entry.done = true;
  entry.win.close();
  if (a.action === 'cancel') return;
  const { req } = entry;
  const opts = { referer: req.referer, cookie: req.cookie, userAgent: req.userAgent, kind: req.kind === 'file' ? undefined : req.kind, sniffed: req.sniffed, paused: a.action === 'later' };
  const name = typeof a.filename === 'string' ? a.filename.trim().slice(0, 255) : '';
  if (req.kind !== 'ytdlp' && name && (name !== entry.initialName || req.filename)) opts.filename = name;
  if (typeof a.dir === 'string' && a.dir && path.isAbsolute(a.dir)) {
    opts.dir = a.dir;
    opts.ownFolder = true;
  }
  try {
    if (isPlaylistUrl(req.url)) await manager.addPlaylist(req.url, opts);
    else manager.add(req.url, opts);
  } catch (err) {
    dialog.showErrorBox('Tachy Download', err.message);
  }
}

function applyAutoStart() {
  // A development run must not register electron.exe as a startup program.
  if (!app.isPackaged) return;
  app.setLoginItemSettings({ openAtLogin: Boolean(manager.settings.autoStart), args: ['--hidden'] });
}

function createTray() {
  const icon = nativeImage.createFromPath(path.join(__dirname, '..', 'build', 'icon.png')).resize({ width: 16, height: 16 });
  tray = new Tray(icon);
  const menu = () =>
    Menu.buildFromTemplate([
      { label: 'Ouvrir Tachy Download', click: showWindow },
      { type: 'separator' },
      { label: 'Tout mettre en pause', click: () => manager.list().forEach((t) => ['downloading', 'queued'].includes(t.status) && manager.pause(t.id)) },
      { label: 'Tout reprendre', click: () => manager.list().forEach((t) => ['paused', 'error'].includes(t.status) && manager.resume(t.id)) },
      { type: 'separator' },
      { label: 'Quitter', click: () => { app.isQuitting = true; app.quit(); } },
    ]);
  tray.setContextMenu(menu());
  tray.on('click', showWindow);
  const refresh = () => {
    const all = manager.list();
    const active = all.filter((t) => t.status === 'downloading');
    const speed = active.reduce((n, t) => n + t.speed, 0);
    tray.setToolTip(
      active.length
        ? `Tachy Download : ${active.length} en cours, ${(speed / 1048576).toFixed(1).replace('.', ',')} Mo/s`
        : 'Tachy Download',
    );
  };
  setInterval(refresh, 2000).unref();
  refresh();
}

app.on('second-instance', () => showWindow());

/** Downloads the video tools (checksum-verified) and exits. Never fails the installer: offline just skips it. */
async function runSetupTools() {
  const t = new Tools(path.join(app.getPath('userData'), 'tools'));
  setTimeout(() => app.exit(0), 8 * 60 * 1000).unref();
  for (const name of ['ytdlp', 'ffmpeg']) {
    if (t.find(name)) continue;
    try {
      await t.install(name);
    } catch {
      // the settings page still offers the install button
    }
  }
  app.exit(0);
}

/** The app used to be called "Download Manager": bring its settings, list, token and tools along once. */
function migrateOldData() {
  const dest = app.getPath('userData');
  const old = path.join(app.getPath('appData'), 'Download Manager');
  try {
    if (fs.existsSync(path.join(dest, 'extension-token.txt')) || !fs.existsSync(path.join(old, 'extension-token.txt'))) return;
    fs.cpSync(old, dest, { recursive: true, force: false });
    if (app.isPackaged) app.setLoginItemSettings({ openAtLogin: false, name: 'Download Manager' }); // stale startup entry
  } catch {
    // starting fresh is acceptable
  }
}

app.whenReady().then(async () => {
  migrateOldData();
  if (setupTools) return runSetupTools();
  const dataDir = app.getPath('userData');
  const token = loadToken(dataDir);

  tools = new Tools(path.join(dataDir, 'tools'));
  tools.on('update', (st) => send('dm:tools', st));

  // Keep our yt-dlp fresh: a check shortly after launch, then every few hours while the app runs.
  const keepYtdlpFresh = () =>
    tools
      .checkYtdlpUpdate({ busy: () => manager.list().some((t) => t.kind === 'ytdlp' && t.status === 'downloading') })
      .catch(() => {}); // offline or rate-limited: try again later
  setTimeout(keepYtdlpFresh, 20_000).unref();
  setInterval(keepYtdlpFresh, 6 * 3600_000).unref();

  manager = new DownloadManager({
    storePath: path.join(dataDir, 'downloads.json'),
    settings: { dir: app.getPath('downloads'), categories: true },
    tools,
  });
  manager.load();
  if (!manager.settings.dir) manager.settings.dir = app.getPath('downloads');

  api = createExtensionServer({
    manager,
    token: () => token,
    onRequest: (req) => manager.settings.confirmDownloads && askAboutDownload(req),
  });
  try {
    await api.listen(6800);
  } catch (err) {
    dialog.showErrorBox('Tachy Download', `Le serveur local n’a pas pu démarrer : ${err.message}`);
  }

  manager.on('update', (snap) => {
    send('dm:update', snap);
    onStatusChange(snap);
  });
  manager.on('removed', (id) => send('dm:removed', id));
  manager.on('settings', (s) => send('dm:settings', s));
  manager.start();

  registerIpc(token);
  createWindow();
  createTray();
  applyAutoStart();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

let quitting = false;
app.on('before-quit', (e) => {
  app.isQuitting = true;
  if (quitting || !manager) return;
  e.preventDefault();
  quitting = true;
  Promise.allSettled([manager.shutdown(), api?.close()]).finally(() => app.quit());
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
