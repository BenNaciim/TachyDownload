'use strict';

/* ---------- icons (constant markup, never user data) ---------- */
const ICONS = {
  plus: '<path d="M12 5v14M5 12h14"/>',
  list: '<path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"/>',
  play: '<path d="M7 4l13 8-13 8z"/>',
  pause: '<path d="M8 5v14M16 5v14"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  calendar: '<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M8 3v4M16 3v4"/>',
  check: '<path d="M5 12l5 5L20 7"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  sliders: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="M20 20l-4.5-4.5"/>',
  download: '<path d="M12 4v11M7 11l5 5 5-5M5 20h14"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  open: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  retry: '<path d="M20 12a8 8 0 1 1-2.3-5.7M20 4v4h-4"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12h10l1-12M9 7V4h6v3"/>',
  chevron: '<path d="M6 9l6 6 6-6"/>',
  film: '<rect x="3" y="6" width="13" height="12" rx="2"/><path d="M16 10l5-3v10l-5-3"/>',
  music: '<path d="M9 18V6l10-2v12"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="16" r="2"/>',
  doc: '<path d="M7 3h7l4 4v14H7z"/><path d="M14 3v4h4M10 12h5M10 16h5"/>',
  box: '<path d="M4 8l8-4 8 4v8l-8 4-8-4z"/><path d="M4 8l8 4 8-4M12 12v8"/>',
  cog: '<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M9 9l3 3-3 3M13 15h3"/>',
  image: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="M21 16l-5-5-8 8"/>',
  dots: '<circle cx="6" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="18" cy="12" r="1.5"/>',
};

function icon(name) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('class', 'i');
  s.setAttribute('viewBox', '0 0 24 24');
  s.innerHTML = ICONS[name];
  return s;
}
document.querySelectorAll('svg[data-i]').forEach((s) => {
  s.setAttribute('viewBox', '0 0 24 24');
  s.innerHTML = ICONS[s.dataset.i];
});
