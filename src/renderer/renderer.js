'use strict';

const $ = (id) => document.getElementById(id);
const rowsEl = $('rows');
const appEl = $('app');
const tasks = new Map(); // id -> { snap, tr, refs }
let appState = null;
let selectedId = null;
let currentFilter = 'all';
let searchText = '';

/* ---------- icons (constant markup, never user data) ---------- */
const ICONS = {
  plus: '<path d="M12 5v14M5 12h14"/>',
  list: '<path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"/>',
  play: '<path d="M7 4l13 8-13 8z"/>',
  pause: '<path d="M8 5v14M16 5v14"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  calendar: '<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M8 3v4M16 3v4"/>',
  check: '<path d="M5 12l5 5L20 7"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  sliders: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="M20 20l-4.5-4.5"/>',
  download: '<path d="M12 4v11M7 11l5 5 5-5M5 20h14"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  open: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  retry: '<path d="M20 12a8 8 0 1 1-2.3-5.7M20 4v4h-4"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12h10l1-12M9 7V4h6v3"/>',
  chevron: '<path d="M6 9l6 6 6-6"/>',
  film: '<rect x="3" y="6" width="13" height="12" rx="2"/><path d="M16 10l5-3v10l-5-3"/>',
  music: '<path d="M9 18V6l10-2v12"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="16" r="2"/>',
  doc: '<path d="M7 3h7l4 4v14H7z"/><path d="M14 3v4h4M10 12h5M10 16h5"/>',
  box: '<path d="M4 8l8-4 8 4v8l-8 4-8-4z"/><path d="M4 8l8 4 8-4M12 12v8"/>',
  cog: '<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M9 9l3 3-3 3M13 15h3"/>',
  image: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="M21 16l-5-5-8 8"/>',
  dots: '<circle cx="6" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="18" cy="12" r="1.5"/>',
};

function icon(name) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('class', 'i');
  s.setAttribute('viewBox', '0 0 24 24');
  s.innerHTML = ICONS[name];
  return s;
}
document.querySelectorAll('svg[data-i]').forEach((s) => {
  s.setAttribute('viewBox', '0 0 24 24');
  s.innerHTML = ICONS[s.dataset.i];
});

/* ---------- theme ---------- */
const THEME_KEY = 'dm-theme';
const getTheme = () => {
  try {
    return localStorage.getItem(THEME_KEY) || 'light';
  } catch {
    return 'light';
  }
};
function applyTheme(theme) {
  const dark = theme === 'dark' || (theme === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}
applyTheme(getTheme());

// Translucent window: only where the OS provides the acrylic material (told by the main process).
const GLASS_KEY = 'dm-glass';
let glassSupported = false;
const getGlass = () => {
  try {
    return localStorage.getItem(GLASS_KEY) !== '0';
  } catch {
    return true;
  }
};
const applyGlass = () => document.documentElement.classList.toggle('glass', glassSupported && getGlass());
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(getTheme()));

/* ---------- formatting ---------- */
const UNITS = ['o', 'Ko', 'Mo', 'Go', 'To'];
function bytesParts(n) {
  if (!Number.isFinite(n) || n <= 0) return { num: '0', unit: 'o' };
  const i = Math.min(UNITS.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return {
    num: v.toLocaleString('fr-FR', { maximumFractionDigits: v >= 100 || i === 0 ? 0 : 1 }),
    unit: UNITS[i],
  };
}
const fmtBytes = (n) => {
  const p = bytesParts(n);
  return `${p.num} ${p.unit}`;
};
const fmtSpeed = (n) => `${fmtBytes(n)}/s`;
function fmtEta(s) {
  if (s === null || s === undefined) return '';
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`;
  return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')} min`;
}
const fmtDate = (ms) =>
  new Date(ms).toLocaleString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });

const KINDS = [
  ['archive', 'ZIP', ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz']],
  ['doc', 'DOC', ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'odt', 'csv', 'epub']],
  ['video', 'VID', ['mp4', 'mkv', 'avi', 'mov', 'webm', 'wmv', 'flv', 'm4v']],
  ['audio', 'MP3', ['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a', 'opus']],
  ['image', 'IMG', ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'avif', 'tif', 'tiff']],
  ['app', 'APP', ['exe', 'msi', 'iso', 'dmg', 'apk', 'deb', 'rpm', 'img', 'msix']],
];
function kindOf(filename) {
  const ext = (/\.([A-Za-z0-9]+)(?:\.part)?$/.exec(filename)?.[1] ?? '').toLowerCase();
  for (const [kind, label, exts] of KINDS) {
    if (exts.includes(ext)) {
      return { kind, label: ext === 'pdf' ? 'PDF' : ext === 'iso' ? 'ISO' : ext === 'exe' ? 'EXE' : label };
    }
  }
  return { kind: 'other', label: ext ? ext.slice(0, 4).toUpperCase() : 'FILE' };
}

// A video found by yt-dlp has no extension until it is saved: it still belongs with the videos.
function kindOfTask(snap) {
  const k = kindOf(snap.filename);
  if (k.kind === 'other' && (snap.kind === 'ytdlp' || snap.kind === 'hls')) {
    return snap.ytOpts?.audioOnly ? { kind: 'audio', label: 'AUDIO' } : { kind: 'video', label: 'VIDÉO' };
  }
  return k;
}

// The tile first shows the coloured label, then swaps in the icon of the program
// Windows uses to open this file type (main process: app.getFileIcon).
const OWN_ICON_EXT = new Set(['exe', 'msi', 'scr', 'ico']);
const iconPromises = new Map();
function setTile(tile, snap) {
  const k = kindOfTask(snap);
  const ext = (/\.([A-Za-z0-9]+)(?:\.part)?$/.exec(snap.filename)?.[1] ?? '').toLowerCase();
  const key = OWN_ICON_EXT.has(ext) && snap.status === 'completed' ? `${ext}:${snap.id}` : `ext:${ext}:${k.kind}`;
  if (tile.dataset.key === key) return;
  tile.dataset.key = key;
  tile.className = `tile t-${k.kind}`;
  tile.textContent = k.label;
  if (!ext) return; // no type known yet: the coloured label is clearer than Windows' blank icon
  if (!iconPromises.has(key)) {
    iconPromises.set(key, window.dm.fileIcon(snap.id).catch(() => null));
  }
  iconPromises.get(key).then((url) => {
    if (!url || tile.dataset.key !== key) return;
    tile.className = 'tile has-icon';
    tile.textContent = '';
    const img = document.createElement('img');
    img.src = url;
    img.alt = '';
    tile.append(img);
  });
}

const STATUS_LABEL = {
  queued: 'En attente',
  scheduled: 'Planifié',
  downloading: 'En cours',
  paused: 'En pause',
  completed: 'Terminé',
  error: 'Échec',
};

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  Object.assign(node, props);
  node.append(...children);
  return node;
}

async function call(fn) {
  try {
    return await fn();
  } catch (err) {
    alert(String(err.message || err).replace(/^Error invoking remote method '[^']+': (Error: )?/, ''));
    return undefined;
  }
}

function iconButton(name, label, onClick, cls = '') {
  const b = el('button', { type: 'button', className: `icon-btn ${cls}`.trim(), title: label });
  b.setAttribute('aria-label', label);
  b.append(icon(name));
  b.addEventListener('click', (e) => {
    e.stopPropagation();
    onClick();
  });
  return b;
}

function textButton(name, label, onClick, cls = '') {
  const b = el('button', { type: 'button', className: cls });
  b.append(icon(name), ` ${label}`);
  b.style.display = 'inline-flex';
  b.style.alignItems = 'center';
  b.style.gap = '6px';
  b.addEventListener('click', onClick);
  return b;
}

/* ---------- filters ---------- */
const GROUPS = {
  active: (s) => s === 'downloading',
  waiting: (s) => s === 'queued' || s === 'paused' || s === 'error',
  scheduled: (s) => s === 'scheduled',
  done: (s) => s === 'completed',
  all: () => true,
};
const TYPE_TITLES = {
  video: 'Vidéos',
  audio: 'Musique',
  doc: 'Documents',
  archive: 'Fichiers compressés',
  app: 'Exécutables',
  image: 'Images',
  other: 'Autres fichiers',
};
const TITLES = {
  all: 'Tous les téléchargements',
  active: 'En cours',
  waiting: 'En attente',
  scheduled: 'Planifiés',
  done: 'Terminés',
};

function matches(snap) {
  if (currentFilter.startsWith('type:')) {
    if (kindOfTask(snap).kind !== currentFilter.slice(5)) return false;
  } else if (!GROUPS[currentFilter](snap.status)) return false;
  if (!searchText) return true;
  const q = searchText.toLowerCase();
  return snap.filename.toLowerCase().includes(q) || snap.url.toLowerCase().includes(q);
}

function refreshList() {
  let visible = 0;
  const counts = { all: 0, active: 0, waiting: 0, scheduled: 0, done: 0 };
  const typeCounts = Object.fromEntries(Object.keys(TYPE_TITLES).map((k) => [k, 0]));
  for (const { snap, tr } of tasks.values()) {
    counts.all++;
    for (const g of ['active', 'waiting', 'scheduled', 'done']) if (GROUPS[g](snap.status)) counts[g]++;
    typeCounts[kindOfTask(snap).kind]++;
    const show = matches(snap);
    tr.hidden = !show;
    if (snap.id === selectedId) drawer.hidden = !show; // the open details follow their row
    if (show) visible++;
  }
  const setCount = (id, n) => {
    const b = $(id);
    b.textContent = n;
    b.classList.toggle('zero', n === 0);
  };
  for (const [g, n] of Object.entries(counts)) setCount(`c-${g}`, n);
  for (const [k, n] of Object.entries(typeCounts)) setCount(`c-type-${k}`, n);
  $('list-title').textContent = currentFilter.startsWith('type:') ? TYPE_TITLES[currentFilter.slice(5)] : TITLES[currentFilter];
  const empty = visible === 0;
  $('empty').hidden = !empty;
  rowsEl.hidden = empty;
  const noTasks = tasks.size === 0;
  $('empty-title').textContent = noTasks ? 'Aucun téléchargement' : 'Rien à afficher';
  $('empty-text').textContent = noTasks
    ? 'Collez un lien avec « Ajouter », ou laissez l’extension du navigateur vous envoyer vos téléchargements.'
    : 'Aucun téléchargement ne correspond à ce filtre ou à cette recherche.';
  $('empty-add').hidden = !noTasks;
  $('chip-active').textContent = `${counts.active} actif${counts.active > 1 ? 's' : ''}`;
  $('chip-waiting').textContent = `${counts.waiting} en attente`;
}

document.querySelectorAll('.nav-item[data-filter]').forEach((btn) =>
  btn.addEventListener('click', () => {
    currentFilter = btn.dataset.filter;
    document.querySelectorAll('.nav-item[data-filter]').forEach((b) => b.classList.toggle('on', b === btn));
    refreshList();
  }),
);
// The file-type section can be folded away; the choice is remembered.
const TYPES_KEY = 'dm-types-open';
function setTypesOpen(open) {
  $('nav-types').hidden = !open;
  $('types-toggle').setAttribute('aria-expanded', String(open));
  $('types-toggle').classList.toggle('closed', !open);
  try {
    localStorage.setItem(TYPES_KEY, open ? '1' : '0');
  } catch {
    // simply not remembered
  }
}
$('types-toggle').addEventListener('click', () => setTypesOpen($('nav-types').hidden));
try {
  if (localStorage.getItem(TYPES_KEY) === '0') setTypesOpen(false);
} catch {
  // default: open
}
$('search').addEventListener('input', (e) => {
  searchText = e.target.value.trim();
  refreshList();
});

/* ---------- rows ---------- */
const SPARK_W = 190;
const SPARK_H = 36;
function svgEl(tag, attrs) {
  const n = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, String(v));
  return n;
}

function buildRow(id) {
  const tile = el('div', { className: 'tile' });
  const name = el('div', { className: 'r-name' });
  const sub = el('div', { className: 'r-sub' });
  const barFill = el('i');
  const bar = el('div', { className: 'bar' }, barFill);
  const pill = el('span', { className: 'pill' });
  const speedNum = el('b');
  const speedUnit = el('small');
  const speed = el('div', { className: 'r-speed' }, speedNum, speedUnit);
  const eta = el('div', { className: 'r-eta' });
  const right = el('div', { className: 'r-right' }, pill, speed, eta);
  const pct = el('div', { className: 'r-pct' });
  // Live speed trace of this download alone: a smoothed line that fades into the past and a pulsing head.
  const spark = svgEl('svg', { viewBox: `0 0 ${SPARK_W} ${SPARK_H}`, width: SPARK_W, height: SPARK_H, class: 'r-spark', 'aria-hidden': 'true' });
  const sparkArea = svgEl('path', { class: 'sp-area' });
  const sparkLine = svgEl('path', { class: 'sp-line' });
  const sparkRing = svgEl('circle', { class: 'sp-ring', r: 3 });
  const sparkHead = svgEl('circle', { class: 'sp-head', r: 2.6 });
  spark.append(svgEl('line', { class: 'sp-base', x1: 0, y1: SPARK_H - 4, x2: SPARK_W, y2: SPARK_H - 4 }), sparkArea, sparkLine, sparkRing, sparkHead);
  const actions = el('div', { className: 'r-actions' });
  const tr = el('div', { className: 'row', tabIndex: 0 }, tile, el('div', { style: 'min-width:0' }, name, sub, bar), spark, pct, right, actions);
  tr.setAttribute('role', 'listitem');
  tr.addEventListener('click', () => select(id));
  tr.addEventListener('keydown', (e) => {
    if ((e.key === 'Enter' || e.key === ' ') && e.target === tr) {
      e.preventDefault();
      select(id);
    }
  });
  return { tr, refs: { tile, name, sub, barFill, pill, speed, speedNum, speedUnit, pct, eta, actions, sparkArea, sparkLine, sparkRing, sparkHead, hist: new Array(60).fill(0), state: '' } };
}

function subline(s) {
  const pct = s.size ? Math.floor((s.downloaded / s.size) * 100) : null;
  switch (s.status) {
    case 'downloading':
      if (s.phase === 'convert') return 'Conversion en .mp4…';
      if (s.kind === 'ytdlp' && !s.size) return 'Recherche de la vidéo…';
      return s.size ? `${pct} % · ${fmtBytes(s.downloaded)} sur ${fmtBytes(s.size)}` : `${fmtBytes(s.downloaded)} reçus`;
    case 'paused':
      return pct === null ? 'En pause' : `En pause à ${pct} % · ${fmtBytes(s.downloaded)} sur ${fmtBytes(s.size)}`;
    case 'completed':
      return `${s.size ? fmtBytes(s.size) : fmtBytes(s.downloaded)} · terminé ${s.completedAt ? fmtDate(s.completedAt) : ''}`.trim();
    case 'error':
      if (s.needsRefresh) {
        return s.autoRefresh
          ? 'Lien expiré · l’extension cherche une nouvelle adresse…'
          : 'Lien expiré · ouvrez la page et relancez le téléchargement';
      }
      return s.error || 'Le téléchargement a échoué';
    case 'scheduled':
      return s.startAt ? `Démarre ${fmtDate(s.startAt)}` : 'Planifié';
    default:
      return 'En attente de son tour';
  }
}

function barFraction(s) {
  if (s.status === 'completed') return 1;
  return s.size ? Math.min(1, s.downloaded / s.size) : 0;
}

function render(snap) {
  let entry = tasks.get(snap.id);
  if (!entry) {
    entry = { snap, ...buildRow(snap.id) };
    tasks.set(snap.id, entry);
    rowsEl.prepend(entry.tr);
  }
  entry.snap = snap;
  const r = entry.refs;
  setTile(r.tile, snap);
  r.name.textContent = snap.filename;
  r.name.title = snap.filename;
  r.sub.textContent = subline(snap);
  r.sub.title = r.sub.textContent;
  r.barFill.style.width = `${barFraction(snap) * 100}%`;
  entry.tr.className = `row ${snap.status}${snap.id === selectedId ? ' selected' : ''}`;

  const running = snap.status === 'downloading';
  // Same reading as the total speed at the top: a big number and a small unit.
  const sp = bytesParts(snap.speed);
  r.speedNum.textContent = running ? sp.num : '';
  r.speedUnit.textContent = running ? ` ${sp.unit}/s` : '';
  r.pct.textContent = snap.size && (running || snap.status === 'paused') ? `${Math.floor(barFraction(snap) * 100)} %` : '';
  r.eta.textContent = running && snap.eta !== null ? `reste ${fmtEta(snap.eta)}` : '';
  r.pill.hidden = running;
  r.pill.className = `pill p-${snap.status}`;
  r.pill.textContent = STATUS_LABEL[snap.status];

  if (r.state !== snap.status) {
    r.state = snap.status;
    const btns = [];
    if (snap.status === 'downloading' || snap.status === 'queued') {
      btns.push(iconButton('pause', 'Mettre en pause', () => call(() => window.dm.pause(snap.id))));
    }
    if (snap.status === 'paused' || snap.status === 'scheduled') {
      btns.push(iconButton('play', 'Reprendre', () => call(() => window.dm.resume(snap.id))));
    }
    if (snap.status === 'error') {
      btns.push(iconButton('retry', 'Réessayer', () => call(() => window.dm.resume(snap.id))));
      if (snap.needsRefresh && snap.pageUrl) {
        btns.push(iconButton('open', 'Ouvrir la page pour obtenir un nouveau lien', () => call(() => window.dm.openPage(snap.id))));
      }
    }
    if (snap.status === 'completed') {
      btns.push(iconButton('open', 'Ouvrir le fichier', () => call(() => window.dm.open(snap.id))));
      btns.push(iconButton('folder', 'Afficher dans le dossier', () => call(() => window.dm.reveal(snap.id))));
    }
    btns.push(
      iconButton('x', 'Retirer de la liste', () => call(() => window.dm.remove(snap.id, snap.status !== 'completed'))),
    );
    r.actions.replaceChildren(...btns);
  }

  refreshList();
  if (snap.id === selectedId) renderDrawer(snap);
}

/* ---------- hero (live speed) ---------- */
const history = new Array(90).fill(0);

function drawHero(total) {
  const peak = Math.max(...history);
  const max = Math.max(peak, 1024 * 1024);
  const pts = history.map((v, i) => `${((i * 360) / (history.length - 1)).toFixed(1)},${(76 - (v / max) * 68).toFixed(1)}`);
  $('g-line').setAttribute('points', pts.join(' '));
  $('g-area').setAttribute('points', `0,80 ${pts.join(' ')} 360,80`);
  const p = total > 0 ? bytesParts(total) : { num: '0', unit: 'Mo' };
  $('hero-num').textContent = p.num;
  $('hero-unit').textContent = `${p.unit}/s`;
  $('hero-peak').textContent = peak > 0 ? `Pic : ${fmtSpeed(peak)} · 90 dernières secondes` : '90 dernières secondes';
  const limit = appState?.settings?.speedLimitKBps ?? 0;
  $('chip-limit').textContent = limit > 0 ? `Limite : ${fmtSpeed(limit * 1024)}` : 'Sans limite';
}

// Catmull-Rom spline through the samples, as cubic Béziers: a soft trace instead of a jagged one.
function smoothPath(pts) {
  const clampY = (y) => Math.min(SPARK_H - 4, Math.max(2, y));
  let d = `M${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[i - 1] ?? pts[i];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[i + 2] ?? p2;
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, clampY(p1[1] + (p2[1] - p0[1]) / 6)];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, clampY(p2[1] - (p3[1] - p1[1]) / 6)];
    d += ` C${c1[0].toFixed(1)},${c1[1].toFixed(1)} ${c2[0].toFixed(1)},${c2[1].toFixed(1)} ${p2[0].toFixed(1)},${p2[1].toFixed(1)}`;
  }
  return d;
}

function drawSpark(r) {
  const max = Math.max(...r.hist, 256 * 1024);
  const plotW = SPARK_W - 8; // leave room for the head
  const pts = r.hist.map((v, i) => [(i * plotW) / (r.hist.length - 1), SPARK_H - 6 - (v / max) * (SPARK_H - 14)]);
  const line = smoothPath(pts);
  const last = pts[pts.length - 1];
  r.sparkLine.setAttribute('d', line);
  r.sparkArea.setAttribute('d', `${line} L${last[0].toFixed(1)},${SPARK_H} L0,${SPARK_H} Z`);
  for (const c of [r.sparkRing, r.sparkHead]) {
    c.setAttribute('cx', last[0].toFixed(1));
    c.setAttribute('cy', last[1].toFixed(1));
  }
}

setInterval(() => {
  let total = 0;
  for (const { snap, refs } of tasks.values()) {
    const running = snap.status === 'downloading';
    if (running) total += snap.speed;
    if (running) {
      refs.hist.push(snap.speed);
      refs.hist.shift();
      drawSpark(refs);
    } else if (refs.hist.some((v) => v > 0)) {
      refs.hist.fill(0); // the trace starts afresh when the download resumes
    }
  }
  history.push(total);
  history.shift();
  drawHero(total);
}, 1000);

/* ---------- detail drawer ---------- */
const drawer = $('drawer');
let drawerSig = '';
let laneCount = -1;
const laneSpeed = new Map(); // `${task}:${segment}` -> { pos, t, bps }

// Accordion: one download's details open right under its row; a second click folds them away.
function select(id) {
  if (id === selectedId) return closeDrawer();
  selectedId = id;
  drawerSig = '';
  laneCount = -1;
  const entry = tasks.get(id);
  entry.tr.after(drawer);
  drawer.hidden = false;
  drawer.classList.remove('unfold');
  void drawer.offsetWidth; // restart the unfolding animation
  drawer.classList.add('unfold');
  for (const e of tasks.values()) e.tr.classList.toggle('selected', e.snap.id === id);
  renderDrawer(entry.snap);
  entry.tr.scrollIntoView({ block: 'start', behavior: 'smooth' });
  return undefined;
}

function closeDrawer() {
  selectedId = null;
  drawer.hidden = true;
  for (const e of tasks.values()) e.tr.classList.remove('selected');
}
$('drawer-close').addEventListener('click', closeDrawer);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && selectedId && !document.querySelector('dialog[open]')) closeDrawer();
});

function infoRow(dl, label, value) {
  dl.append(el('dt', { textContent: label }), el('dd', { textContent: value, title: value }));
}

function renderDrawer(s) {
  setTile($('d-tile'), s);
  $('d-name').textContent = s.filename;
  $('d-status').className = `pill p-${s.status}`;
  $('d-status').textContent = STATUS_LABEL[s.status];

  const frac = barFraction(s);
  $('d-pct').textContent = s.size || s.status === 'completed' ? `${Math.floor(frac * 100)} %` : '–';
  $('d-sizes').textContent = `${fmtBytes(s.downloaded)} sur ${s.size ? fmtBytes(s.size) : 'taille inconnue'}`;
  $('d-bar').className = `bar big s-${s.status}`;
  $('d-bar').firstChild.style.width = `${frac * 100}%`;
  const running = s.status === 'downloading';
  $('d-speed').textContent = running ? fmtSpeed(s.speed) : '–';
  $('d-eta').textContent = running && s.eta !== null ? fmtEta(s.eta) : '–';

  const segs = s.segments ?? [];
  if (segs.length === 0) {
    laneCount = -1;
    $('d-map').replaceChildren();
    $('d-lanes').replaceChildren(el('p', { className: 'muted', textContent: 'Les connexions apparaîtront au démarrage.', style: 'margin:0;font-size:12.5px' }));
  } else {
    if (laneCount !== segs.length) {
      laneCount = segs.length;
      $('d-lanes').replaceChildren(
        ...segs.map((_, i) => {
          const fill = el('i');
          return el(
            'div',
            { className: 'lane' },
            el('span', { textContent: s.rangeOk ? `Segment ${i + 1}` : 'Flux unique' }),
            el('div', { className: 'track' }, fill),
            el('span'),
          );
        }),
      );
    }
    const now = Date.now();
    const mapCells = $('d-map').children;
    if (mapCells.length !== segs.length) {
      $('d-map').replaceChildren(...segs.map((seg) => el('div', { style: `flex:${Math.max(1, s.size ? seg.end - seg.start + 1 : 1)}` }, el('i'))));
    }
    [...$('d-lanes').children].forEach((lane, i) => {
      const seg = segs[i];
      const total = s.size ? seg.end - seg.start + 1 : 0;
      const f = s.status === 'completed' ? 1 : total ? Math.min(1, (seg.pos - seg.start) / total) : 0;
      lane.querySelector('i').style.width = `${f * 100}%`;
      lane.classList.toggle('done', f >= 1);
      mapCells[i].firstChild.style.width = `${f * 100}%`;
      mapCells[i].classList.toggle('done', f >= 1);

      // Per-connection speed, measured between two updates at least 0.7 s apart.
      const key = `${s.id}:${i}`;
      const prev = laneSpeed.get(key);
      if (!prev || seg.pos < prev.pos) laneSpeed.set(key, { pos: seg.pos, t: now, bps: 0 });
      else if (now - prev.t >= 700) laneSpeed.set(key, { pos: seg.pos, t: now, bps: ((seg.pos - prev.pos) * 1000) / (now - prev.t) });
      const bps = running && f < 1 ? laneSpeed.get(key).bps : 0;
      lane.lastChild.textContent = total ? `${Math.floor(f * 100)} %${bps > 0 ? ` · ${fmtSpeed(bps)}` : ''}` : '';
      lane.lastChild.title = total ? `${fmtBytes(Math.max(0, seg.pos - seg.start))} sur ${fmtBytes(total)}` : '';
    });
  }

  const sig = `${s.status}|${s.filename}|${s.size}|${s.rangeOk}|${s.dir}|${segs.length}`;
  if (sig !== drawerSig) {
    drawerSig = sig;
    const dl = $('d-info');
    dl.replaceChildren();
    infoRow(dl, 'Adresse', s.url);
    infoRow(dl, 'Dossier', s.dir);
    infoRow(dl, 'Reprise', s.finalUrl ? (s.rangeOk ? 'Possible' : 'Non prise en charge par le serveur') : 'Vérifiée au démarrage');
    infoRow(dl, 'Taille', s.size ? `${fmtBytes(s.size)} (${s.size.toLocaleString('fr-FR')} octets)` : 'Inconnue');
    infoRow(dl, 'Connexions', String(segs.length || s.connections));
    if (s.rangeOk && segs.length && s.size) infoRow(dl, 'Découpage', `${segs.length} segments d’environ ${fmtBytes(Math.ceil(s.size / segs.length))}`);
    infoRow(dl, 'Méthode', { hls: 'Flux vidéo (HLS), segments téléchargés puis assemblés', ytdlp: 'Vidéo de la page (yt-dlp)' }[s.kind] ?? (s.rangeOk ? 'Segments parallèles' : 'Flux unique'));
    infoRow(dl, 'Ajouté', fmtDate(s.createdAt));

    const acts = [];
    if (s.status === 'downloading' || s.status === 'queued') acts.push(textButton('pause', 'Pause', () => call(() => window.dm.pause(s.id))));
    if (s.status === 'paused' || s.status === 'scheduled') acts.push(textButton('play', 'Reprendre', () => call(() => window.dm.resume(s.id)), 'primary'));
    if (s.status === 'error') acts.push(textButton('retry', 'Réessayer', () => call(() => window.dm.resume(s.id)), 'primary'));
    if (s.status === 'error' && s.needsRefresh && s.pageUrl) {
      acts.push(textButton('open', 'Ouvrir la page', () => call(() => window.dm.openPage(s.id))));
    }
    if (s.status === 'completed') {
      acts.push(textButton('open', 'Ouvrir', () => call(() => window.dm.open(s.id)), 'primary'));
      acts.push(textButton('folder', 'Dossier', () => call(() => window.dm.reveal(s.id))));
    }
    acts.push(textButton('x', 'Retirer', () => call(() => window.dm.remove(s.id, s.status !== 'completed'))));
    if (s.status === 'completed') {
      acts.push(
        textButton('trash', 'Supprimer le fichier', () => {
          if (confirm(`Supprimer « ${s.filename} » du disque ?`)) call(() => window.dm.remove(s.id, true));
        }, 'danger'),
      );
    }
    $('d-actions').replaceChildren(...acts);
  }
}

/* ---------- add dialog ---------- */
const dlgAdd = $('dlg-add');
const openAdd = () => {
  $('add-error').textContent = '';
  dlgAdd.showModal();
  $('add-urls').focus();
};
$('btn-add').addEventListener('click', openAdd);
$('empty-add').addEventListener('click', openAdd);
$('add-cancel').addEventListener('click', () => dlgAdd.close());
$('add-x').addEventListener('click', () => dlgAdd.close());
$('add-schedule').addEventListener('change', (e) => {
  $('add-when-wrap').hidden = !e.target.checked;
  if (e.target.checked && !$('add-when').value) {
    const d = new Date(Date.now() + 3600_000);
    d.setMinutes(d.getMinutes() - d.getTimezoneOffset());
    $('add-when').value = d.toISOString().slice(0, 16);
  }
});
$('form-add').addEventListener('submit', async (e) => {
  e.preventDefault();
  const urls = $('add-urls').value.split(/\s+/).filter(Boolean);
  if (urls.length === 0) return;
  let startAt = null;
  if ($('add-schedule').checked) {
    startAt = new Date($('add-when').value).getTime();
    if (!Number.isFinite(startAt)) {
      $('add-error').textContent = 'Date de départ invalide';
      return;
    }
  }
  try {
    const res = await window.dm.add(urls, { startAt, kind: $('add-video').checked ? 'ytdlp' : undefined });
    if (res.errors.length) {
      $('add-error').textContent = res.errors.map((x) => `${x.url} : ${x.error}`).join(' · ');
      $('add-urls').value = res.errors.map((x) => x.url).join('\n');
      return;
    }
    $('add-urls').value = '';
    dlgAdd.close();
  } catch (err) {
    $('add-error').textContent = String(err.message || err).replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
  }
});

/* ---------- settings dialog ---------- */
const dlgSet = $('dlg-settings');
function fillSettings() {
  const s = appState.settings;
  $('set-dir').value = s.dir;
  $('set-connections').value = s.connections;
  $('set-concurrent').value = s.maxConcurrent;
  $('set-limit').value = s.speedLimitKBps;
  $('set-sched-on').checked = s.schedule.enabled;
  $('set-sched-from').value = s.schedule.from;
  $('set-sched-to').value = s.schedule.to;
  $('set-theme').value = getTheme();
  $('set-glass').checked = getGlass();
  $('set-glass').closest('label').hidden = !glassSupported;
  $('set-height').value = String(s.videoMaxHeight);
  $('set-mp4').checked = s.convertToMp4;
  $('set-cookies').value = s.ytCookies;
  $('set-categories').checked = s.categories;
  $('set-tray').checked = s.closeToTray;
  $('set-autostart').checked = s.autoStart;
  $('set-notif').checked = s.notifications;
  $('set-confirm').checked = s.confirmDownloads;
  $('set-autoupdate').checked = s.autoUpdate;
  $('update-status').textContent = `Version installée : ${appState.version || '–'}`;
  $('set-after').value = s.afterAll;
  window.dm.toolsStatus().then(drawTools);
  $('info-port').textContent = appState.port || 'indisponible';
  $('info-token').value = appState.token;
}
$('btn-settings').addEventListener('click', () => {
  $('set-error').textContent = '';
  fillSettings();
  dlgSet.showModal();
  dlgSet.focus(); // not the close cross, which would show a focus ring
});
function drawTools(st) {
  const line = (id, label, path, key) => {
    const busy = st.installing?.name === key;
    const pct = busy && st.installing.total ? ` ${Math.floor((st.installing.received / st.installing.total) * 100)} %` : '';
    $(id).textContent = busy ? `${label} : téléchargement…${pct}` : path ? `${label} : installé` : `${label} : non installé`;
    $('inst-' + key).hidden = Boolean(path) || Boolean(st.installing);
  };
  line('tool-ytdlp', 'yt-dlp (YouTube et autres sites)', st.ytdlp, 'ytdlp');
  line('tool-ffmpeg', 'ffmpeg (fusion audio/vidéo)', st.ffmpeg, 'ffmpeg');
  line('tool-deno', 'Deno (moteur JavaScript pour YouTube)', st.deno, 'deno');
}
window.dm.onTools(drawTools);
for (const key of ['ytdlp', 'deno', 'ffmpeg']) {
  $('inst-' + key).addEventListener('click', () =>
    window.dm.installTool(key).then(drawTools, (err) => {
      $('set-error').textContent = String(err.message || err).replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
      window.dm.toolsStatus().then(drawTools);
    }),
  );
}
/* ---------- app updates ---------- */
let updateDismissed = '';
function showUpdate(u) {
  const bar = $('update-bar');
  if (!u || u.version === updateDismissed) {
    bar.hidden = true;
    $('install-update').hidden = !u;
    return;
  }
  $('update-text').textContent = `La version ${u.version} de Tachy Download est disponible.`;
  $('update-go').textContent = u.canInstall ? 'Installer et redémarrer' : 'Voir la page de téléchargement';
  $('update-go').disabled = false;
  bar.hidden = false;
  $('install-update').textContent = $('update-go').textContent;
  $('install-update').hidden = false;
}
window.dm.onUpdateAvail(showUpdate);
window.dm.onUpdateProgress(({ got, total }) => {
  $('update-go').textContent = total ? `Téléchargement… ${Math.floor((got / total) * 100)} %` : 'Téléchargement…';
});
$('update-go').addEventListener('click', async () => {
  const active = [...tasks.values()].filter((e) => e.snap.status === 'downloading').length;
  if (active && !confirm(`${active} téléchargement(s) en cours seront mis en pause puis pourront être repris. Continuer ?`)) return;
  $('update-go').disabled = true;
  try {
    await window.dm.installUpdate();
  } catch (err) {
    $('update-text').textContent = String(err.message || err).replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
    $('update-go').disabled = false;
  }
});
$('update-x').addEventListener('click', () => {
  updateDismissed = $('update-text').textContent.match(/\d+(\.\d+)+/)?.[0] ?? '';
  $('update-bar').hidden = true;
});
$('install-update').addEventListener('click', () => {
  dlgSet.close();
  $('update-go').click();
});
$('check-update').addEventListener('click', async () => {
  $('update-status').textContent = 'Recherche en cours…';
  try {
    const r = await window.dm.checkUpdate();
    updateDismissed = '';
    $('update-status').textContent = r.update
      ? `Version ${r.update.version} disponible (installée : ${r.current}).`
      : `Vous avez la dernière version (${r.current}).`;
    showUpdate(r.update);
  } catch (err) {
    $('update-status').textContent = `Recherche impossible : ${String(err.message || err).replace(/^Error invoking remote method '[^']+': (Error: )?/, '')}`;
  }
});
$('set-cancel').addEventListener('click', () => dlgSet.close());
$('set-x').addEventListener('click', () => dlgSet.close());
$('set-glass').addEventListener('change', (e) => {
  try {
    localStorage.setItem(GLASS_KEY, e.target.checked ? '1' : '0');
  } catch {
    // preference simply not remembered
  }
  applyGlass();
});
$('set-theme').addEventListener('change', (e) => {
  try {
    localStorage.setItem(THEME_KEY, e.target.value);
  } catch {
    // theme still applies for this session
  }
  applyTheme(e.target.value);
});
$('set-pick').addEventListener('click', async () => {
  const dir = await call(() => window.dm.pickDir());
  if (dir) $('set-dir').value = dir;
});
$('copy-token').addEventListener('click', () => call(() => window.dm.copyToken()));
$('form-settings').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    appState.settings = await window.dm.setSettings({
      dir: $('set-dir').value,
      connections: Number($('set-connections').value),
      maxConcurrent: Number($('set-concurrent').value),
      speedLimitKBps: Number($('set-limit').value),
      videoMaxHeight: Number($('set-height').value),
      convertToMp4: $('set-mp4').checked,
      categories: $('set-categories').checked,
      closeToTray: $('set-tray').checked,
      autoStart: $('set-autostart').checked,
      notifications: $('set-notif').checked,
      confirmDownloads: $('set-confirm').checked,
      autoUpdate: $('set-autoupdate').checked,
      ytCookies: $('set-cookies').value,
      afterAll: $('set-after').value,
      schedule: {
        enabled: $('set-sched-on').checked,
        from: $('set-sched-from').value,
        to: $('set-sched-to').value,
      },
    });
    drawHero(history[history.length - 1]);
    dlgSet.close();
  } catch (err) {
    $('set-error').textContent = String(err.message || err);
  }
});

/* ---------- toolbar ---------- */
$('btn-pause-all').addEventListener('click', () => {
  for (const { snap } of tasks.values()) {
    if (snap.status === 'downloading' || snap.status === 'queued') call(() => window.dm.pause(snap.id));
  }
});
$('btn-resume-all').addEventListener('click', () => {
  for (const { snap } of tasks.values()) {
    if (snap.status === 'paused' || snap.status === 'error') call(() => window.dm.resume(snap.id));
  }
});

/* ---------- boot ---------- */
window.dm.onUpdate(render);
window.dm.onRemoved((id) => {
  tasks.get(id)?.tr.remove();
  tasks.delete(id);
  if (id === selectedId) closeDrawer();
  refreshList();
});
window.dm.onSettings((s) => {
  if (appState) appState.settings = s;
  drawHero(history[history.length - 1]);
});

(async () => {
  appState = await window.dm.state();
  showUpdate(appState.update);
  glassSupported = Boolean(appState.glass);
  applyGlass();
  const list = await window.dm.list();
  list.sort((a, b) => a.createdAt - b.createdAt).forEach(render);
  refreshList();
  drawHero(0);
})();
