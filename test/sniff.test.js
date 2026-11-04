'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const Sniff = require('../extension/sniff.js');

const MB = 1024 * 1024;
const kind = (o) => Sniff.classify(o).kind;

test('classify: a playlist is found by its type, its extension, or an .m3u8 hidden in the query', () => {
  assert.equal(kind({ url: 'https://c.example/v/index.m3u8', mime: 'application/octet-stream', size: 0 }), 'hls');
  assert.equal(kind({ url: 'https://c.example/v/abc', mime: 'application/vnd.apple.mpegurl', size: 0 }), 'hls');
  assert.equal(kind({ url: 'https://c.example/v/abc', mime: 'application/x-mpegURL; charset=utf-8', size: 0 }), 'hls');
  assert.equal(kind({ url: 'https://c.example/get?file=/a/b/master.m3u8&t=1', mime: 'text/plain', size: 0 }), 'hls');
  assert.equal(kind({ url: 'https://c.example/get/master.m3u8/index.php', mime: 'text/html', size: 0 }), 'hls');
});

test('classify: a DASH manifest is found by its type or its extension', () => {
  assert.equal(kind({ url: 'https://c.example/v/manifest.mpd', mime: 'text/xml', size: 0 }), 'dash');
  assert.equal(kind({ url: 'https://c.example/v/abc', mime: 'application/dash+xml', size: 0 }), 'dash');
});

test('classify: a media file counts only when it is big enough, pieces of a stream never', () => {
  assert.deepEqual(Sniff.classify({ url: 'https://c.example/a.mp4', mime: 'video/mp4', size: 5 * MB }), { kind: 'file', ext: 'mp4' });
  assert.equal(kind({ url: 'https://c.example/a.mp4', mime: 'video/mp4', size: 10 * 1024 }), null, 'a thumbnail-sized clip');
  assert.equal(Sniff.classify({ url: 'https://c.example/a', mime: 'audio/mpeg', size: 5 * MB }).ext, 'mp3', 'the extension comes from the type when the address has none');
  assert.equal(kind({ url: 'https://c.example/s/1.ts', mime: 'video/mp2t', size: 2 * MB }), null);
  assert.equal(kind({ url: 'https://c.example/s/1.m4s', mime: 'video/iso.segment', size: 2 * MB }), null);
  assert.equal(kind({ url: 'https://c.example/s/9', mime: 'video/mp2t', size: 2 * MB }), null);
});

test('classify: an answer that could be anything is worth a look at its first bytes', () => {
  assert.equal(kind({ url: 'https://c.example/p/master.txt', mime: 'text/plain', size: 800 }), 'probe');
  assert.equal(kind({ url: 'https://c.example/kEHUFPxLg7UXpFz2r518/JLkLJ', mime: 'application/octet-stream', size: 0 }), 'probe');
  assert.equal(kind({ url: 'https://c.example/x', mime: '', size: 1200 }), 'probe');
  assert.equal(kind({ url: 'https://c.example/dl/film', mime: 'binary/octet-stream', size: 700 * MB }), 'probe', 'a disguised big file');
});

test('classify: what is plainly not a stream is never probed', () => {
  for (const [url, mime] of [
    ['https://c.example/app.js', 'application/octet-stream'],
    ['https://c.example/style.css', 'text/plain'],
    ['https://c.example/api/data', 'application/json'],
    ['https://c.example/page', 'text/html'],
    ['https://c.example/p.png', 'application/octet-stream'],
    ['https://c.example/a.woff2', 'application/octet-stream'],
    ['https://c.example/seg/12.ts', 'application/octet-stream'],
  ]) {
    assert.equal(kind({ url, mime, size: 800 }), null, url);
  }
  assert.equal(kind({ url: 'https://c.example/x', mime: 'application/octet-stream', size: 20 * 1024 * 1024 * 3 }), 'probe');
  assert.equal(kind({ url: 'https://c.example/x', mime: 'application/octet-stream', size: 3 * MB }), null, 'too big for a playlist, too small to be worth a guess');
});

test('classify says why it ignored a response', () => {
  assert.match(Sniff.classify({ url: 'https://c.example/p.png', mime: 'image/png', size: 5 * MB }).why, /média/);
  assert.match(Sniff.classify({ url: 'https://c.example/s.ts', mime: 'video/mp2t', size: 5 * MB }).why, /morceau/);
  assert.match(Sniff.classify({ url: 'https://c.example/a.mp4', mime: 'video/mp4', size: 100 }).why, /Ko/);
});

const bytes = (s) => Uint8Array.from(Buffer.from(s, 'latin1'));

test('fromBytes: reads what a response really is', () => {
  assert.deepEqual(Sniff.fromBytes(bytes('#EXTM3U\n#EXT-X-VERSION:3\n')), { kind: 'hls' });
  assert.deepEqual(Sniff.fromBytes(bytes('\xEF\xBB\xBF \n#EXTM3U\n')), { kind: 'hls' }, 'a byte-order mark and a blank line do not matter');
  assert.deepEqual(Sniff.fromBytes(bytes('<?xml version="1.0"?>\n<MPD xmlns="urn:mpeg:dash:schema:mpd:2011">')), { kind: 'dash' });
  assert.deepEqual(Sniff.fromBytes(bytes('\x00\x00\x00\x20ftypisom\x00\x00\x02\x00')), { kind: 'file', ext: 'mp4' });
  assert.deepEqual(Sniff.fromBytes(bytes('\x1A\x45\xDF\xA3\x01\x00\x00\x00')), { kind: 'file', ext: 'webm' });
  assert.deepEqual(Sniff.fromBytes(bytes('FLV\x01\x05\x00\x00\x00\x09')), { kind: 'file', ext: 'flv' });
  assert.deepEqual(Sniff.fromBytes(bytes('ID3\x04\x00\x00\x00\x00')), { kind: 'file', ext: 'mp3' });
  assert.deepEqual(Sniff.fromBytes(bytes('OggS\x00\x02')), { kind: 'file', ext: 'ogg' });
});

test('fromBytes: a piece of a stream is not a video to list, and nothing else is either', () => {
  const ts = Buffer.alloc(400);
  [0, 188, 376].forEach((i) => (ts[i] = 0x47));
  assert.equal(Sniff.fromBytes(Uint8Array.from(ts)), null);
  assert.equal(Sniff.fromBytes(bytes('\x00\x00\x00\x18stypmsdh')), null);
  assert.equal(Sniff.fromBytes(bytes('{"a":1}')), null);
  assert.equal(Sniff.fromBytes(bytes('<html><body>')), null);
  assert.equal(Sniff.fromBytes(new Uint8Array(0)), null);
});

test('dashInfo: length, best height, live, and protection', () => {
  const clear = '<MPD type="static" mediaPresentationDuration="PT1H2M3S"><Period><AdaptationSet><Representation height="720"/><Representation height="1080"/></AdaptationSet></Period></MPD>';
  assert.deepEqual(Sniff.dashInfo(clear), { drm: false, live: false, duration: 3723, height: 1080 });
  const live = '<MPD type="dynamic" minimumUpdatePeriod="PT2S"></MPD>';
  assert.equal(Sniff.dashInfo(live).live, true);
  const widevine = '<MPD><Period><AdaptationSet><ContentProtection schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"/></AdaptationSet></Period></MPD>';
  assert.equal(Sniff.dashInfo(widevine).drm, true);
  const cenc = '<MPD><ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/></MPD>';
  assert.equal(Sniff.dashInfo(cenc).drm, true);
  assert.equal(Sniff.dashInfo('<MPD mediaPresentationDuration="PT45.5S"/>').duration, 46);
});
