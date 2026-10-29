'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { formatArgs, cleanYtOpts, summarizeProbe, YtdlpDownload } = require('../src/engine/ytdlp');

test('cleanYtOpts keeps only known options', () => {
  assert.deepEqual(cleanYtOpts({ audioOnly: true, height: 720, lang: 'fr', evil: '--exec' }), { audioOnly: true, height: 720, lang: 'fr' });
  assert.equal(cleanYtOpts({ height: 999, lang: '--exec calc' }), null);
  assert.equal(cleanYtOpts(null), null);
  assert.deepEqual(cleanYtOpts({ height: 0, lang: 'en-US' }), { lang: 'en-US' });
});

test('formatArgs: height and audio language drive the sort', () => {
  const a = formatArgs({ opts: { height: 720, lang: 'fr' }, maxHeight: 1080, ffmpeg: '/x/ffmpeg' });
  assert.deepEqual(a.slice(-2), ['-S', 'lang:fr,res:720,vcodec:h264,acodec:m4a']);
  const b = formatArgs({ opts: null, maxHeight: 1080, ffmpeg: '/x/ffmpeg' });
  assert.deepEqual(b.slice(-2), ['-S', 'res:1080,vcodec:h264,acodec:m4a']);
});

test('formatArgs: without ffmpeg a single file with sound is chosen', () => {
  assert.deepEqual(formatArgs({ opts: { height: 480 }, maxHeight: 1080, ffmpeg: null }), ['-f', 'b[height<=480]/b']);
});

test('formatArgs: audio only becomes mp3 with ffmpeg, stays m4a without', () => {
  const withFf = formatArgs({ opts: { audioOnly: true }, maxHeight: 1080, ffmpeg: '/x/ffmpeg' });
  assert.ok(withFf.includes('-x') && withFf.includes('mp3'));
  const without = formatArgs({ opts: { audioOnly: true }, maxHeight: 1080, ffmpeg: null });
  assert.ok(!without.includes('-x'));
  assert.ok(without[1].startsWith('ba'));
});

test('summarizeProbe lists heights high to low and audio languages once', () => {
  const p = summarizeProbe({
    title: 'T',
    duration: 90,
    formats: [
      { vcodec: 'avc1', acodec: 'none', height: 720 },
      { vcodec: 'avc1', acodec: 'none', height: 1080 },
      { vcodec: 'vp9', acodec: 'none', height: 1080 },
      { vcodec: 'none', acodec: 'mp4a', language: 'fr', format_note: 'Français, medium' },
      { vcodec: 'none', acodec: 'mp4a', language: 'fr', format_note: 'Français, low' },
      { vcodec: 'none', acodec: 'mp4a', language: 'en', format_note: 'English (default), medium' },
      { vcodec: 'none', acodec: 'mp4a' },
    ],
  });
  assert.deepEqual(p.heights, [1080, 720]);
  assert.deepEqual(p.langs, [{ code: 'fr', label: 'Français' }, { code: 'en', label: 'English' }]);
  assert.equal(p.duration, 90);
});

test('the chosen options reach the yt-dlp command line', () => {
  const state = { url: 'https://youtu.be/abc', dir: '/d', connections: 4, ytOpts: { height: 480, lang: 'fr' } };
  const ctx = { settings: () => ({ videoMaxHeight: 1080 }), ffmpeg: () => '/t/ffmpeg.exe' };
  const dl = Object.create(YtdlpDownload.prototype);
  dl.state = state;
  dl.ctx = ctx;
  const args = dl._args('/t/yt-dlp.exe');
  assert.ok(args.includes('lang:fr,res:480,vcodec:h264,acodec:m4a'));
  assert.equal(args.at(-1), 'https://youtu.be/abc');
});
