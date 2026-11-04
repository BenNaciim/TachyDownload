'use strict';

// "Download this video" button shown over a playing video, like IDM's floating bar.
// The list of streams comes from the background script (it sees the network requests).
(() => {
  const api = typeof browser !== 'undefined' ? browser : chrome;
  let host = null;
  let items = [];
  let fetchedAt = 0;

  const ask = (msg) =>
    new Promise((resolve) => {
      try {
        api.runtime.sendMessage(msg, (r) => resolve(api.runtime.lastError ? null : r));
      } catch {
        resolve(null);
      }
    });

  /** " · 1080p · 54 min · Français" for a stream, or its host when nothing more is known. */
  function streamInfo(it) {
    if (it.kind === 'ytdlp') return '';
    const bits = [];
    if (it.height) bits.push(`${it.height}p`);
    if (it.duration) bits.push(it.duration >= 3600 ? `${Math.floor(it.duration / 3600)} h ${String(Math.round((it.duration % 3600) / 60)).padStart(2, '0')}` : `${Math.max(1, Math.round(it.duration / 60))} min`);
    if (it.langs?.length) bits.push(it.langs.join(', '));
    if (!bits.length) bits.push(new URL(it.url).hostname);
    return ` · ${bits.join(' · ')}`;
  }

  function ensureUi() {
    if (host) return host;
    host = document.createElement('div');
    host.style.cssText = 'all:initial;position:fixed;z-index:2147483647;display:none';
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `
      <style>
        .bar{display:flex;align-items:center;gap:8px;background:#101a33;color:#fff;border-radius:10px;padding:5px 12px 5px 5px;cursor:pointer;box-shadow:0 2px 12px rgba(0,0,0,.4);border:1px solid rgba(255,255,255,.18);font:600 13px system-ui,sans-serif;max-width:300px}
        .bar:hover{background:#18264a}
        .bar.err{background:#7a1426;border-color:#ff8da0;cursor:default}
        .bar.ok{background:#0b5d4b}
        svg{flex:none;display:block}
        .txt{display:flex;flex-direction:column;line-height:1.25;min-width:0}
        .app{font-weight:500;font-size:10.5px;opacity:.7}
        .label{white-space:normal}
        .menu{margin-top:6px;background:#fff;color:#1a2140;border-radius:8px;box-shadow:0 4px 18px rgba(0,0,0,.35);overflow:hidden;font:13px system-ui,sans-serif;max-width:300px}
        .menu button{display:block;width:100%;text-align:left;background:none;color:inherit;border:0;cursor:pointer;font:500 13px system-ui,sans-serif;padding:8px 12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
        .menu button:hover{background:#e8edff}
      </style>
      <div class="bar" id="main" role="button" tabindex="0">
        <svg width="30" height="30" viewBox="0 0 24 24" aria-hidden="true">
          <rect width="24" height="24" rx="6" fill="#0d1428"/>
          <path d="M5.2 16.6a7.4 7.4 0 1 1 13.6 0" fill="none" stroke="#ff7a1a" stroke-width="2.2" stroke-linecap="round"/>
          <path d="M12 13.6 15.8 8.6" stroke="#fff" stroke-width="1.9" stroke-linecap="round"/>
          <circle cx="12" cy="13.6" r="1.7" fill="#fff"/>
          <path d="M8.6 19.2 12 21.4l3.4-2.2" fill="none" stroke="#ff7a1a" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <span class="txt"><span class="app">Tachy Download</span><span class="label" id="label">Télécharger cette vidéo</span></span>
      </div>
      <div class="menu" id="menu" hidden></div>`;
    const menu = root.getElementById('menu');
    const main = root.getElementById('main');
    const label = root.getElementById('label');
    const send = async (url) => {
      label.textContent = 'Envoi…';
      const r = await ask({ type: 'download', url });
      main.classList.toggle('ok', Boolean(r?.ok));
      main.classList.toggle('err', !r?.ok);
      // No answer at all means this page still runs the script of an extension that was reloaded or updated since.
      label.textContent = r?.ok
        ? 'Envoyé à Tachy Download ✓'
        : r === null
          ? 'L’extension a été rechargée ou mise à jour : rechargez cette page (F5), puis réessayez.'
          : r.message || 'Envoi impossible : réessayez, ou rechargez la page.';
      menu.hidden = true;
      setTimeout(() => {
        main.classList.remove('ok', 'err');
        label.textContent = 'Télécharger cette vidéo';
      }, r?.ok ? 3000 : 9000);
    };
    main.addEventListener('click', () => {
      if (main.classList.contains('err')) return undefined;
      if (items.length === 1) return send(items[0].url);
      const size = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1).replace('.', ',')} Mo` : `${Math.max(1, Math.round(n / 1024))} Ko`);
      menu.replaceChildren(
        ...items.map((it) => {
          const b = document.createElement('button');
          b.textContent = `${it.kind === 'ytdlp' ? 'Vidéo de la page' : it.kind === 'hls' ? 'Flux vidéo (HLS)' : it.kind === 'dash' ? 'Flux vidéo (DASH)' : it.ext.toUpperCase()}${it.size ? ` · ${size(it.size)}` : ''}${streamInfo(it)}`;
          b.addEventListener('click', () => send(it.url));
          return b;
        }),
      );
      menu.hidden = !menu.hidden;
      return undefined;
    });
    document.documentElement.append(host);
    return host;
  }

  async function onEnter(video) {
    const r = video.getBoundingClientRect();
    if (r.width < 240 || r.height < 135) return;
    if (Date.now() - fetchedAt > 1500) {
      fetchedAt = Date.now();
      items = (await ask({ type: 'list', page: true }))?.items ?? [];
    }
    if (items.length === 0) return;
    const ui = ensureUi();
    ui.style.left = `${Math.max(4, r.left + 10)}px`;
    ui.style.top = `${Math.max(4, r.top + 10)}px`;
    ui.style.display = 'block';
  }

  // A name for the file made from the page: its heading, plus the season and episode picked in the page.
  function pageTitle() {
    const text = (el) => el?.textContent?.replace(/\s+/g, ' ').trim() ?? '';
    const meta = document.querySelector('meta[property="og:title"],meta[name="og:title"]')?.content?.trim() ?? '';
    const heading = text(document.querySelector('h1'));
    const base = [heading, meta, document.title].find((t) => t.length > 3 && t.length < 160) ?? document.title;
    const picked = [...document.querySelectorAll('select')]
      .map((s) => text(s.selectedOptions?.[0]))
      .filter((t) => t && t.length < 40 && /saison|season|[ée]pisode|\bs\d+|\be\d+/i.test(t) && !base.toLowerCase().includes(t.toLowerCase()));
    return [base, ...picked].join(' - ');
  }
  if (window === window.top) {
    api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
      if (msg?.type !== 'pageTitle') return undefined;
      sendResponse({ title: pageTitle() });
      return undefined;
    });
  }

  let hideTimer = null;
  document.addEventListener(
    'mouseover',
    (e) => {
      if (host && e.composedPath().includes(host)) {
        clearTimeout(hideTimer);
        return;
      }
      // Players lay their own controls over the video: look at what lies under the pointer, not only at the target.
      const video = e.target instanceof HTMLVideoElement ? e.target : document.elementsFromPoint(e.clientX, e.clientY).find((el) => el instanceof HTMLVideoElement);
      if (video) {
        clearTimeout(hideTimer);
        onEnter(video);
      } else if (host) {
        clearTimeout(hideTimer);
        hideTimer = setTimeout(() => (host.style.display = 'none'), 1800);
      }
    },
    true,
  );
})();
