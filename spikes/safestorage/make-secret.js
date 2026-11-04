'use strict';
// THROWAWAY (proof S3). Runs under Electron with a temporary data dir, no window:
//   electron spikes/safestorage/make-secret.js --user-data-dir=<tmp>
// It encrypts a few SYNTHETIC secrets exactly like src/main.js does (safeStorage.encryptString(text).toString('base64'))
// and writes them next to Chromium's `Local State` (which holds the DPAPI-wrapped key), for spike-safestorage-read to open.
const { app, safeStorage } = require('electron');
const fs = require('node:fs');
const path = require('node:path');

const PLAIN = ['synthetic-secret', 'mot de passe é€ 日本 🔐', 'x'.repeat(4096)];

app.whenReady().then(() => {
  const dir = app.getPath('userData');
  const ok = safeStorage.isEncryptionAvailable();
  const secrets = ok ? PLAIN.map((plain) => ({ plain, secret: safeStorage.encryptString(plain).toString('base64') })) : [];
  fs.writeFileSync(path.join(dir, 'spike-secrets.json'), JSON.stringify({ available: ok, secrets }));
  app.quit();
});
