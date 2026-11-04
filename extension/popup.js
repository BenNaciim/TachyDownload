'use strict';

const $ = (id) => document.getElementById(id);

const fmtSize = (n) =>
  !n ? '' : n >= 1024 ** 3 ? `${(n / 1024 ** 3).toFixed(1)} Go` : n >= 1024 ** 2 ? `${Math.round(n / 1024 ** 2)} Mo` : `${Math.round(n / 1024)} Ko`;

function label(item) {
  if (item.kind === 'ytdlp') return { name: 'Vidéo de cette page', meta: 'Meilleure qualité (yt-dlp)' };
  if (item.kind === 'hls') return { name: 'Flux vidéo (HLS)', meta: 'Meilleure qualité disponible' };
  if (item.kind === 'dash') return { name: 'Flux vidéo (DASH)', meta: [item.height && `${item.height}p`, 'yt-dlp'].filter(Boolean).join(' · ') };
  const type = item.mime.startsWith('audio/') ? 'Audio' : 'Vidéo';
  return { name: `${type} ${item.ext.toUpperCase()}`, meta: fmtSize(item.size) };
}

(async () => {
  $('settings').addEventListener('click', () => chrome.runtime.openOptionsPage());
  chrome.runtime.sendMessage({ type: 'poll' }); // also look for expired links whenever the popup opens
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const { items = [] } = tab ? await chrome.runtime.sendMessage({ type: 'list', tabId: tab.id }) : {};
  $('empty').hidden = items.length > 0;

  // Diagnostic: record what the browser sees on the page, then copy it for whoever looks into a missing video.
  const { diag } = await chrome.storage.local.get({ diag: false });
  $('diag').checked = diag;
  $('copy').hidden = !diag;
  $('diag').addEventListener('change', async () => {
    const on = $('diag').checked;
    await chrome.storage.local.set({ diag: on });
    $('copy').hidden = !on;
    $('note').textContent = on ? 'Diagnostic actif : rechargez la page, lancez la vidéo, puis cliquez sur « Copier le diagnostic ».' : '';
  });
  $('copy').addEventListener('click', async () => {
    const r = tab ? await chrome.runtime.sendMessage({ type: 'diag', tabId: tab.id }) : null;
    if (!r?.text) {
      $('note').textContent = 'Rien à copier.';
      return;
    }
    try {
      await navigator.clipboard.writeText(r.text);
      $('note').textContent = `Copié : ${r.count} requêtes. Collez-le dans la conversation.`;
    } catch {
      $('report').value = r.text;
      $('report').hidden = false;
      $('report').select();
      $('note').textContent = 'Copie automatique impossible : sélectionnez le texte ci-dessous (Ctrl+A, Ctrl+C).';
    }
  });

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
      if (!r?.ok) $('note').textContent = r?.message ?? 'Envoi impossible : l’application est-elle lancée ?';
    });
    li.append(info, btn);
    $('list').append(li);
  }
})();
