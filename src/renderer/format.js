'use strict';

/* ---------- formatting ---------- */
const UNITS = ['o', 'Ko', 'Mo', 'Go', 'To'];
function bytesParts(n) {
  if (!Number.isFinite(n) || n <= 0) return { num: '0', unit: 'o' };
  const i = Math.min(UNITS.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return {
    num: v.toLocaleString('fr-FR', { maximumFractionDigits: v >= 100 || i === 0 ? 0 : 1 }),
    unit: UNITS[i],
  };
}
const fmtBytes = (n) => {
  const p = bytesParts(n);
  return `${p.num} ${p.unit}`;
};
const fmtSpeed = (n) => `${fmtBytes(n)}/s`;
function fmtEta(s) {
  if (s === null || s === undefined) return '';
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`;
  return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')} min`;
}
const fmtDate = (ms) =>
  new Date(ms).toLocaleString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
