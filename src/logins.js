'use strict';

const fs = require('node:fs');
const path = require('node:path');

const TYPES = ['basic', 'bearer', 'cookie'];

/** The host out of whatever the user typed: "example.com", "https://example.com/page", "Example.com:8080". */
function cleanHost(value) {
  const raw = String(value ?? '').trim().replace(/^((?:[a-z][a-z0-9+.-]*:\/\/)?)\*\./i, '$1'); // "*.example.com" means the site and its subdomains
  if (!raw) throw new Error('Indiquez le site');
  let host = '';
  try {
    host = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `http://${raw}`).hostname.toLowerCase();
  } catch {
    throw new Error('Adresse du site invalide');
  }
  if (!host || !/^[a-z0-9.-]+$/.test(host) && !host.includes(':')) throw new Error('Adresse du site invalide');
  return host.replace(/^\*?\./, '');
}

const onSite = (host, site) => host === site || host.endsWith('.' + site);

/**
 * Logins kept per site. Secrets are stored encrypted (`crypt` is Electron's safeStorage in the app, which
 * ties them to the Windows account) and never leave this module except as request headers for that site.
 */
function createLogins({ file, crypt }) {
  let entries = [];
  try {
    const data = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (Array.isArray(data)) entries = data.filter((e) => e && typeof e.host === 'string' && TYPES.includes(e.type) && typeof e.secret === 'string');
  } catch {
    // no file yet, or a damaged one: start empty
  }

  const persist = () => {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const tmp = file + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(entries), { mode: 0o600 });
    fs.renameSync(tmp, file);
  };

  const list = () => entries.map(({ host, type, user }) => ({ host, type, user: user ?? '' })).sort((a, b) => a.host.localeCompare(b.host));

  return {
    list,

    save({ host, type, user = '', secret }) {
      if (!TYPES.includes(type)) throw new Error('Type de connexion inconnu');
      const h = cleanHost(host);
      const u = String(user ?? '').trim();
      const s = String(secret ?? '');
      if (!s) throw new Error(type === 'cookie' ? 'Collez le cookie' : type === 'bearer' ? 'Collez le jeton' : 'Indiquez le mot de passe');
      if (type === 'basic' && !u) throw new Error('Indiquez le nom d’utilisateur');
      if (u.length > 256 || s.length > 4096) throw new Error('Valeur trop longue');
      entries = entries.filter((e) => e.host !== h);
      entries.push({ host: h, type, user: type === 'basic' ? u : '', secret: crypt.encrypt(s) });
      persist();
      return list();
    },

    remove(host) {
      const h = cleanHost(host);
      entries = entries.filter((e) => e.host !== h);
      persist();
      return list();
    },

    /** Request headers for this address, or null when no login is kept for its site. */
    headersFor(url) {
      let host;
      try {
        host = new URL(url).hostname.toLowerCase();
      } catch {
        return null;
      }
      // The most specific site wins: login.example.com before example.com.
      const e = entries.filter((x) => onSite(host, x.host)).sort((a, b) => b.host.length - a.host.length)[0];
      if (!e) return null;
      let secret;
      try {
        secret = crypt.decrypt(e.secret);
      } catch {
        return null; // saved under another Windows account: unreadable here
      }
      if (e.type === 'basic') return { Authorization: 'Basic ' + Buffer.from(`${e.user}:${secret}`).toString('base64') };
      if (e.type === 'bearer') return { Authorization: 'Bearer ' + secret };
      return { Cookie: secret };
    },
  };
}

module.exports = { createLogins, cleanHost, TYPES };
