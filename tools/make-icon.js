'use strict';

// Draws the app icon (dark rounded square with an orange speed gauge) and writes
// build/icon.png and build/icon.ico. No dependencies: PNG and ICO are encoded by hand.
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');

const SIZES = [256, 64, 48, 32, 24, 16];

function insideRoundedRect(x, y, w, h, r) {
  if (x < 0 || y < 0 || x > w || y > h) return false;
  const cx = Math.min(Math.max(x, r), w - r);
  const cy = Math.min(Math.max(y, r), h - r);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
}

function insideTriangle(px, py, [ax, ay], [bx, by], [cx, cy]) {
  const d = (x1, y1, x2, y2, x3, y3) => (x1 - x3) * (y2 - y3) - (x2 - x3) * (y1 - y3);
  const d1 = d(px, py, ax, ay, bx, by);
  const d2 = d(px, py, bx, by, cx, cy);
  const d3 = d(px, py, cx, cy, ax, ay);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

const distToSegment = (px, py, [ax, ay], [bx, by]) => {
  const dx = bx - ax, dy = by - ay;
  const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
};

const CX = 128, CY = 172.8, R = 76.8, W = 22.4; // speed gauge, 256-unit design space
const polar = (deg) => [CX + R * Math.cos((deg * Math.PI) / 180), CY - R * Math.sin((deg * Math.PI) / 180)];
const ON_ARC = (x, y, fromDeg, toDeg) => {
  // part of the ring between two angles measured over the top (180 = left, 0 = right)
  if (Math.abs(Math.hypot(x - CX, y - CY) - R) > W / 2) return false;
  const phi = (Math.atan2(CY - y, x - CX) * 180) / Math.PI;
  if (phi < Math.min(fromDeg, toDeg) || phi > Math.max(fromDeg, toDeg)) {
    const [ex, ey] = polar(fromDeg);
    const [fx, fy] = polar(toDeg);
    return Math.hypot(x - ex, y - ey) <= W / 2 || Math.hypot(x - fx, y - fy) <= W / 2;
  }
  return true;
};
const capped = (x, y, fromDeg, toDeg) =>
  ON_ARC(x, y, fromDeg, toDeg) ||
  Math.hypot(x - polar(fromDeg)[0], y - polar(fromDeg)[1]) <= W / 2 ||
  Math.hypot(x - polar(toDeg)[0], y - polar(toDeg)[1]) <= W / 2;

/** Colour at (x, y) in a 256-unit design space, or null when transparent. */
function sample(x, y) {
  if (!insideRoundedRect(x, y, 256, 256, 61)) return null;
  const orange = [251, 146, 60];
  // needle, hub and chevron
  if (distToSegment(x, y, [CX, CY], [158.4, 121.6]) <= 5.6 || Math.hypot(x - CX, y - CY) <= 14.4) return [255, 255, 255];
  if (distToSegment(x, y, [105.6, 208], [128, 230.4]) <= 5.6 || distToSegment(x, y, [128, 230.4], [150.4, 208]) <= 5.6) return orange;
  // gauge: grey track, orange progress (from the left up to 60 degrees)
  if (y <= CY + 0.001 || Math.hypot(x - polar(0)[0], y - polar(0)[1]) <= W / 2 || Math.hypot(x - polar(180)[0], y - polar(180)[1]) <= W / 2) {
    if (capped(x, y, 60, 180)) return orange;
    if (capped(x, y, 0, 180)) return [55, 65, 81];
  }
  // thin orange edge, then the dark body with a soft light at the top
  if (!insideRoundedRect(x - 4, y - 4, 248, 248, 57)) return [142, 93, 49];
  const lift = Math.round(14 * (1 - y / 256));
  return [31 + lift, 41 + lift, 55 + lift];
}

function render(size) {
  const ss = 4; // supersampling for smooth edges
  const px = Buffer.alloc(size * size * 4);
  for (let j = 0; j < size; j++) {
    for (let i = 0; i < size; i++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sj = 0; sj < ss; sj++) {
        for (let si = 0; si < ss; si++) {
          const c = sample(((i + (si + 0.5) / ss) / size) * 256, ((j + (sj + 0.5) / ss) / size) * 256);
          if (c) { r += c[0]; g += c[1]; b += c[2]; a++; }
        }
      }
      const o = (j * size + i) * 4;
      if (a) {
        px[o] = Math.round(r / a);
        px[o + 1] = Math.round(g / a);
        px[o + 2] = Math.round(b / a);
        px[o + 3] = Math.round((a / (ss * ss)) * 255);
      }
    }
  }
  return px;
}

function png(size, rgba) {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(zlib.crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function ico(images) {
  const head = Buffer.alloc(6);
  head.writeUInt16LE(1, 2); // type: icon
  head.writeUInt16LE(images.length, 4);
  let offset = 6 + 16 * images.length;
  const entries = images.map(({ size, data }) => {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4); // planes
    e.writeUInt16LE(32, 6); // bits per pixel
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    return e;
  });
  return Buffer.concat([head, ...entries, ...images.map((i) => i.data)]);
}

const outDir = path.join(__dirname, '..', 'build');
fs.mkdirSync(outDir, { recursive: true });
const images = SIZES.map((size) => ({ size, data: png(size, render(size)) }));
fs.writeFileSync(path.join(outDir, 'icon.png'), images[0].data);
fs.writeFileSync(path.join(outDir, 'icon.ico'), ico(images));
console.log('wrote build/icon.png and build/icon.ico', images.map((i) => `${i.size}px`).join(' '));
