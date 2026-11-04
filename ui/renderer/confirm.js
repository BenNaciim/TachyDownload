'use strict';

const $ = (id) => document.getElementById(id);

// Theme, accent colour and glass come from appearance.js, the same as the main window.

const fmtSize = (n) => {
  if (n >= 1073741824) return `${(n / 1073741824).toFixed(2).replace('.', ',')} Go`;
  if (n >= 1048576) return `${(n / 1048576).toFixed(1).replace('.', ',')} Mo`;
  return `${Math.max(1, Math.round(n / 1024))} Ko`;
};

let info = null;
let dirTouched = false;

async function updateCategory() {
  if (!info || dirTouched || info.kind === 'ytdlp') {
    $('cat').textContent = '';
    return;
  }
  const cat = await window.confirmApi.category($('name').value);
  $('cat').textContent = cat ? `Classé automatiquement dans le sous-dossier « ${cat} »` : '';
}

function ytChoice() {
  if (info?.kind !== 'ytdlp' || $('yt-grid').hidden) return undefined;
  const audioOnly = $('yt-kind').value === 'audio';
  const height = Number($('yt-q').value) || 0;
  const lang = $('yt-lang-wrap').hidden ? '' : $('yt-lang').value;
  return { audioOnly, height: audioOnly ? 0 : height, lang };
}

function answer(action) {
  if (!info) return;
  const name = $('name').value.trim();
  window.confirmApi.answer({
    action,
    filename: info.kind === 'ytdlp' ? '' : name,
    dir: dirTouched ? $('dir').value : '',
    yt: ytChoice(),
  });
}

const qualityLabel = (h) => (h >= 4320 ? '8K' : h >= 2160 ? '4K (2160p)' : `${h}p`);

function fillChoices(p) {
  if (p.unavailable) {
    // Nothing to choose from: the download still works with the default quality.
    $('yt-status').textContent = p.error ? `${p.error} Le téléchargement utilisera la qualité par défaut.` : 'Qualité par défaut.';
    return;
  }
  $('yt-status').textContent = [p.title, p.duration ? `${Math.floor(p.duration / 60)} min ${String(Math.round(p.duration % 60)).padStart(2, '0')} s` : ''].filter(Boolean).join(' · ');
  const q = $('yt-q');
  q.replaceChildren(new Option(`Meilleure (limite : ${qualityLabel(p.maxHeight)})`, '0'));
  for (const h of p.heights.filter((x) => x >= 144)) q.add(new Option(qualityLabel(h), String(h)));
  if (!p.ffmpeg) {
    $('yt-kind').querySelector('[value="audio"]').textContent = 'Audio seul (.m4a)';
  }
  if (p.langs.length > 1) {
    const l = $('yt-lang');
    l.replaceChildren(new Option('Par défaut', ''));
    for (const { code, label } of p.langs) l.add(new Option(label === code ? code : `${label} (${code})`, code));
    $('yt-lang-wrap').hidden = false;
  }
  $('yt-grid').hidden = false;
}

(async () => {
  info = await window.confirmApi.init();
  if (!info) return;
  const label = { ytdlp: 'Vidéo de la page', hls: 'Flux vidéo (HLS)', file: 'Fichier' }[info.kind];
  $('title').textContent = info.kind === 'file' ? 'Télécharger ce fichier ?' : 'Télécharger cette vidéo ?';
  $('meta').textContent = [label, info.size ? fmtSize(info.size) : 'Taille inconnue', info.host].filter(Boolean).join(' · ');
  if (info.icon) $('ico').src = info.icon;
  else $('ico').hidden = true;
  $('url').value = info.url;
  $('url').title = info.url;
  $('name').value = info.kind === 'ytdlp' ? 'Choisi automatiquement (titre de la vidéo)' : info.filename;
  $('name').disabled = info.kind === 'ytdlp';
  $('dir').value = info.dir;
  updateCategory();

  if (info.kind === 'file' && info.sniffed && info.size > 0 && info.size < 2 * 1048576) {
    $('warn').textContent = `Ce fichier ne pèse que ${fmtSize(info.size)} : ce n’est peut-être pas la vidéo complète (publicité, aperçu). Vérifiez avant de télécharger.`;
    $('warn').hidden = false;
  }
  $('name').focus();
  $('name').select();
  if (info.kind === 'ytdlp') {
    $('yt').hidden = false;
    window.confirmApi.probe().then(fillChoices, () => fillChoices({ unavailable: true }));
  }
})();

$('yt-kind').addEventListener('change', () => {
  $('yt-q-wrap').hidden = $('yt-kind').value === 'audio';
});

let timer = null;
$('name').addEventListener('input', () => {
  clearTimeout(timer);
  timer = setTimeout(updateCategory, 200);
});
$('pick').addEventListener('click', async () => {
  const dir = await window.confirmApi.pickDir($('dir').value);
  if (dir) {
    $('dir').value = dir;
    dirTouched = true;
    updateCategory();
  }
});
$('form').addEventListener('submit', (e) => {
  e.preventDefault();
  answer('now');
});
$('later').addEventListener('click', () => answer('later'));
$('cancel').addEventListener('click', () => answer('cancel'));
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') answer('cancel');
});
