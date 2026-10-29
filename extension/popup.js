'use strict';

const $ = (id) => document.getElementById(id);

const fmtSize = (n) =>
  !n ? '' : n >= 1024 ** 3 ? `${(n / 1024 ** 3).toFixed(1)} Go` : n >= 1024 ** 2 ? `${Math.round(n / 1024 ** 2)} Mo` : `${Math.round(n / 1024)} Ko`;

function label(item) {
  if (item.kind === 'ytdlp') return { name: 'Vidéo de cette page', meta: 'Meilleure qualité (yt-dlp)' };
  if (item.kind === 'hls') return { name: 'Flux vidéo (HLS)', meta: 'Meilleure qualité disponible' };
  const type = item.mime.startsWith('audio/') ? 'Audio' : 'Vidéo';
  return { name: `${type} ${item.ext.toUpperCase()}`, meta: fmtSize(item.size) };
}

(async () => {
  $('settings').addEventListener('click', () => chrome.runtime.openOptionsPage());
  chrome.runtime.sendMessage({ type: 'poll' }); // also look for expired links whenever the popup opens
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const { items = [] } = tab ? await chrome.runtime.sendMessage({ type: 'list', tabId: tab.id }) : {};
  $('empty').hidden = items.length > 0;

  for (const item of items) {
    const { name, meta } = label(item);
    const li = document.createElement('li');
    const info = document.createElement('div');
    info.className = 'info';
    const n = document.createElement('div');
    n.className = 'name';
    n.textContent = name;
    n.title = item.url;
    const m = document.createElement('div');
    m.className = 'meta';
    m.textContent = [meta, new URL(item.url).hostname].filter(Boolean).join(' · ');
    info.append(n, m);
    const btn = document.createElement('button');
    btn.textContent = 'Télécharger';
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      const r = await chrome.runtime.sendMessage({ type: 'download', tabId: tab.id, url: item.url });
      btn.textContent = r?.ok ? 'Envoyé' : 'Échec';
      btn.disabled = Boolean(r?.ok);
      if (!r?.ok) $('note').textContent = 'L’application est-elle lancée, et le jeton saisi dans les réglages ?';
    });
    li.append(info, btn);
    $('list').append(li);
  }
})();
