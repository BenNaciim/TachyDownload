'use strict';

/*
 * Theme, accent colour and glass look: remembered in localStorage and applied to the page. Loaded by the
 * main window and by the confirmation window, so both always look the same. Plain script: its top-level
 * names are shared with the script that follows it.
 */

const THEME_KEY = 'dm-theme';
const getTheme = () => {
  try {
    return localStorage.getItem(THEME_KEY) || 'light';
  } catch {
    return 'light';
  }
};
function applyTheme(theme) {
  const dark = theme === 'dark' || (theme === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(getTheme()));

// Look of the glass and the accent colour, applied as CSS variables.
const LOOK_DEFAULTS = { accent: 'violet', glassT: 30, glassBlur: 26 };
const lookKey = (k) => `dm-look-${k}`;
function getLook(k) {
  try {
    const v = localStorage.getItem(lookKey(k));
    if (v !== null) return k === 'accent' ? v : Number(v);
  } catch {
    // falls back to the default
  }
  return LOOK_DEFAULTS[k];
}
function setLook(k, v) {
  try {
    localStorage.setItem(lookKey(k), String(v));
  } catch {
    // preference simply not remembered
  }
}
function applyLook() {
  const root = document.documentElement;
  const t = Math.min(100, Math.max(0, getLook('glassT'))) / 100;
  // Higher transparency = lighter fill. Light glass is a white veil, dark glass a faint white veil over a tint.
  root.style.setProperty('--g-a1', (0.92 - 0.8 * t).toFixed(3));
  root.style.setProperty('--g-a2', ((0.92 - 0.8 * t) * 0.6).toFixed(3));
  root.style.setProperty('--gd-a1', (0.28 - 0.26 * t).toFixed(3));
  root.style.setProperty('--gd-a2', ((0.28 - 0.26 * t) * 0.35).toFixed(3));
  root.style.setProperty('--gd-s1', (0.86 - 0.8 * t).toFixed(3));
  root.style.setProperty('--gd-s2', (0.96 - 0.8 * t).toFixed(3));
  root.style.setProperty('--g-blur', `${Math.min(60, Math.max(0, getLook('glassBlur')))}px`);
  root.dataset.accent = getLook('accent');
}

applyTheme(getTheme());
applyLook();
