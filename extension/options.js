'use strict';

const DEFAULTS = { token: '', port: 6800, intercept: true, minSizeMB: 0 };
const $ = (id) => document.getElementById(id);

(async () => {
  const s = { ...DEFAULTS, ...(await chrome.storage.local.get(DEFAULTS)) };
  $('token').value = s.token;
  $('port').value = s.port;
  $('intercept').checked = s.intercept;
  $('minSize').value = s.minSizeMB;
})();

$('save').addEventListener('click', async () => {
  const next = {
    token: $('token').value.trim(),
    port: Number($('port').value) || 6800,
    intercept: $('intercept').checked,
    minSizeMB: Math.max(0, Number($('minSize').value) || 0),
  };
  await chrome.storage.local.set(next);

  const status = $('status');
  try {
    const ping = await fetch(`http://127.0.0.1:${next.port}/ping`);
    if (!ping.ok) throw new Error('ping');
    // /add with an empty body is rejected with 400 only if the token is valid; 401 means a wrong token.
    const auth = await fetch(`http://127.0.0.1:${next.port}/add`, {
      method: 'POST',
      headers: { 'X-DM-Token': next.token, 'Content-Type': 'application/json' },
      body: '{}',
    });
    status.textContent = auth.status === 401 ? 'Application trouvée, mais le jeton est refusé.' : 'Connecté à l’application.';
  } catch {
    status.textContent = 'Application introuvable : est-elle lancée, et le port est-il le bon ?';
  }
});
