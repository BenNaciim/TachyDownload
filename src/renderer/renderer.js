'use strict';

const $ = (id) => document.getElementById(id);
const rowsEl = $('rows');
const appEl = $('app');
const tasks = new Map(); // id -> { snap, tr, refs }
let appState = null;
let selectedId = null;
let currentFilter = 'all';
let searchText = '';

/* icons: see icons.js, number and time formatting: see format.js */

/* ---------- theme: see appearance.js ---------- */

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


// File types come from shared/filetypes.js, the same table the engine uses to pick download folders.
function kindOf(filename) {
  const ext = FileTypes.extOf(filename);
  const type = FileTypes.typeOf(filename);
  if (type) return { kind: type.kind, label: ['pdf', 'iso', 'exe'].includes(ext) ? ext.toUpperCase() : type.label };
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
  if (segs.length === 0 && s.kind === 'ytdlp') {
    // yt-dlp cuts the video into small fragments itself: show how many are done.
    laneCount = -1;
    const f = s.frag;
    const cells = 48;
    if ($('d-map').children.length !== (f ? cells : 0)) {
      $('d-map').replaceChildren(...(f ? Array.from({ length: cells }, () => el('div', { style: 'flex:1' }, el('i'))) : []));
    }
    if (f) {
      const part = f.done / f.total;
      [...$('d-map').children].forEach((cell, i) => {
        const c = Math.max(0, Math.min(1, part * cells - i));
        cell.firstChild.style.width = `${c * 100}%`;
        cell.classList.toggle('done', c >= 1);
      });
    }
    $('d-lanes').replaceChildren(
      f
        ? el('div', { className: 'lane' }, el('span', { textContent: 'Fragments' }), el('div', { className: 'track' }, Object.assign(el('i'), { style: `width:${Math.min(100, (f.done / f.total) * 100)}%` })), el('span', { textContent: `${f.done.toLocaleString('fr-FR')} / ${f.total.toLocaleString('fr-FR')}` }))
        : el('p', { className: 'muted', textContent: 'Téléchargement géré par yt-dlp : le découpage apparaît dès que la vidéo démarre.', style: 'margin:0;font-size:12.5px' }),
    );
  } else if (segs.length === 0) {
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
// Each setting once: its name in the engine, the field that edits it and how to read that field.
const SETTING_FIELDS = [
  ['dir', 'set-dir', 'text'],
  ['connections', 'set-connections', 'number'],
  ['maxConcurrent', 'set-concurrent', 'number'],
  ['speedLimitKBps', 'set-limit', 'number'],
  ['videoMaxHeight', 'set-height', 'number'],
  ['convertToMp4', 'set-mp4', 'bool'],
  ['categories', 'set-categories', 'bool'],
  ['closeToTray', 'set-tray', 'bool'],
  ['autoStart', 'set-autostart', 'bool'],
  ['notifications', 'set-notif', 'bool'],
  ['confirmDownloads', 'set-confirm', 'bool'],
  ['autoUpdate', 'set-autoupdate', 'bool'],
  ['ytCookies', 'set-cookies', 'text'],
  ['afterAll', 'set-after', 'text'],
];
function fillSettings() {
  const s = appState.settings;
  for (const [key, id, type] of SETTING_FIELDS) {
    if (type === 'bool') $(id).checked = s[key];
    else $(id).value = String(s[key]);
  }
  $('set-sched-on').checked = s.schedule.enabled;
  $('set-sched-from').value = s.schedule.from;
  $('set-sched-to').value = s.schedule.to;
  syncAppearance();
  $('update-status').textContent = `Version installée : ${appState.version || '–'}`;
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
  syncAppearance();
});

/* ---------- appearance settings ---------- */
function markChoice(group, value) {
  for (const b of group.querySelectorAll('button')) {
    const on = b.dataset.v === value;
    b.setAttribute('aria-checked', String(on));
    b.tabIndex = on ? 0 : -1; // one tab stop per group, arrows move inside it
    b.classList.toggle('on', on);
  }
}
// Arrow keys move through a radiogroup and pick the option, as native radios do.
for (const id of ['set-theme', 'set-accent']) {
  $(id).addEventListener('keydown', (e) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    if (!step) return;
    const all = [...e.currentTarget.querySelectorAll('button')];
    const next = all[(all.indexOf(document.activeElement) + step + all.length) % all.length];
    e.preventDefault();
    next.focus();
    next.click();
  });
}
function syncAppearance() {
  markChoice($('set-theme'), getTheme());
  markChoice($('set-accent'), getLook('accent'));
  $('set-glass').checked = getGlass();
  $('set-glass').closest('label').hidden = !glassSupported;
  $('set-glass-t').value = getLook('glassT');
  $('set-glass-blur').value = getLook('glassBlur');
  $('out-glass-t').textContent = `${getLook('glassT')} %`;
  $('out-glass-blur').textContent = `${getLook('glassBlur')} px`;
  const tune = glassSupported && getGlass();
  for (const id of ['glass-tune', 'glass-tune2']) $(id).hidden = !tune;
}
$('set-theme').addEventListener('click', (e) => {
  const v = e.target.closest('button')?.dataset.v;
  if (!v) return;
  try {
    localStorage.setItem(THEME_KEY, v);
  } catch {
    // theme still applies for this session
  }
  applyTheme(v);
  syncAppearance();
});
$('set-accent').addEventListener('click', (e) => {
  const v = e.target.closest('button')?.dataset.v;
  if (!v) return;
  setLook('accent', v);
  applyLook();
  syncAppearance();
});
for (const [id, key] of [['set-glass-t', 'glassT'], ['set-glass-blur', 'glassBlur']]) {
  $(id).addEventListener('input', (e) => {
    setLook(key, Number(e.target.value));
    applyLook();
    syncAppearance();
  });
}
$('set-appearance-reset').addEventListener('click', () => {
  try {
    for (const k of Object.keys(LOOK_DEFAULTS)) localStorage.removeItem(lookKey(k));
    localStorage.removeItem(THEME_KEY);
    localStorage.removeItem(GLASS_KEY);
  } catch {
    // nothing stored to clear
  }
  applyTheme(getTheme());
  applyGlass();
  applyLook();
  syncAppearance();
});
$('set-pick').addEventListener('click', async () => {
  const dir = await call(() => window.dm.pickDir());
  if (dir) $('set-dir').value = dir;
});
$('copy-token').addEventListener('click', () => call(() => window.dm.copyToken()));
$('form-settings').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    const patch = {
      schedule: {
        enabled: $('set-sched-on').checked,
        from: $('set-sched-from').value,
        to: $('set-sched-to').value,
      },
    };
    for (const [key, id, type] of SETTING_FIELDS) {
      patch[key] = type === 'bool' ? $(id).checked : type === 'number' ? Number($(id).value) : $(id).value;
    }
    appState.settings = await window.dm.setSettings(patch);
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
