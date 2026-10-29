'use strict';

const $ = (id) => document.getElementById(id);

try {
  const theme = localStorage.getItem('dm-theme') || 'light';
  const dark = theme === 'dark' || (theme === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
} catch {
  document.documentElement.dataset.theme = 'light';
}

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

function answer(action) {
  if (!info) return;
  const name = $('name').value.trim();
  window.confirmApi.answer({
    action,
    filename: info.kind === 'ytdlp' ? '' : name,
    dir: dirTouched ? $('dir').value : '',
  });
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
})();

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
