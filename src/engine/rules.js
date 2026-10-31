'use strict';

const path = require('node:path');

/**
 * Filing rules, one per line: `pattern = folder`. A pattern starting with a dot or `*.` matches a file
 * extension; anything else matches a site (the host itself or any of its subdomains). The folder is
 * absolute or relative to the download folder. Lines starting with # are comments.
 */
function parseRules(text) {
  const rules = [];
  for (const line of String(text ?? '').split(/\r?\n/)) {
    const m = /^\s*([^#=][^=]*?)\s*=\s*(.+?)\s*$/.exec(line);
    if (!m) continue;
    const pattern = m[1].toLowerCase();
    const folder = m[2].replace(/^"|"$/g, '');
    const ext = /^\*?\.([a-z0-9]{1,10})$/.exec(pattern);
    rules.push(ext ? { ext: ext[1], folder } : { host: pattern.replace(/^\*?\./, ''), folder });
  }
  return rules;
}

const hostOf = (url) => {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
};

const onSite = (host, site) => Boolean(host) && (host === site || host.endsWith('.' + site));

/** The folder the first matching rule sends this file to, or null. */
function folderFor(rules, { filename, url, pageUrl }, baseDir) {
  const ext = path.extname(filename).slice(1).toLowerCase();
  const hosts = [hostOf(url), hostOf(pageUrl)];
  for (const r of rules) {
    const hit = r.ext ? r.ext === ext : hosts.some((h) => onSite(h, r.host));
    if (hit) return path.isAbsolute(r.folder) ? r.folder : path.join(baseDir, r.folder);
  }
  return null;
}

module.exports = { parseRules, folderFor };
