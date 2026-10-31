'use strict';

const { ProxyAgent, fetch: undiciFetch } = require('undici');

/** A proxy address as typed by the user: "host:port" is read as http://host:port. Empty means no proxy. */
function normalizeProxy(value) {
  const raw = String(value ?? '').trim();
  if (!raw) return '';
  let u;
  try {
    u = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `http://${raw}`);
  } catch {
    throw new Error('Adresse du proxy invalide');
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') throw new Error('Le proxy doit être de type http ou https');
  if (!u.hostname) throw new Error('Adresse du proxy invalide');
  return u.toString().replace(/\/$/, '');
}

/** The fetch every download uses: the plain one, or one that goes through the proxy. */
function makeFetch(proxy) {
  if (!proxy) return fetch;
  const dispatcher = new ProxyAgent(proxy);
  return (url, init) => undiciFetch(url, { ...init, dispatcher });
}

module.exports = { normalizeProxy, makeFetch };
