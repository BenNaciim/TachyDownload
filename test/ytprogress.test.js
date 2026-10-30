'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { applyProgress } = require('../src/engine/ytdlp');

const fresh = () => ({ ytd: { base: 0, cur: 0, total: 0 }, frag: null, size: null });
const feed = (s, lines, first = true) => lines.map((l, i) => applyProgress(s, `DMP ${l}`, first && i === 0));

// Lines in the shape yt-dlp really printed for a long YouTube video: no byte total, an estimate that
// wanders, and a fragment index that wobbles with 16 fragments in flight.
test('a long fragmented video keeps a sane size when the byte count wobbles', () => {
  const s = fresh();
  feed(s, [
    '271244144 NA 3815647496.33 215 3008',
    '271911792 NA 3683838307.17 221 3008',
    '1715471624 NA 3648800885.23 1415 3008',
    '1714791056 NA 3653381169.64 1413 3008', // a few KB lower: not a new stream
    '1716008200 NA 3645288152.58 1415 3008',
  ]);
  assert.equal(s.ytd.base, 0);
  assert.equal(s.size, 3645288152.58, 'the last estimate from yt-dlp');
  assert.ok(s.size > 3.5e9 && s.size < 3.8e9, `size ${s.size}`);
  assert.deepEqual(s.frag, { done: 1415, total: 3008 }, 'fragment count never goes back');
});

test('at the start the fragment index lags: the total stays unknown rather than exploding', () => {
  const s = fresh();
  feed(s, ['130048 NA NA 0 120', '307168 NA NA 1 120', '1034144 NA NA 2 120']);
  assert.equal(s.size, 0, 'unknown until a few fragments are done');
  feed(s, ['6000000 NA NA 6 120'], false);
  assert.equal(s.size, Math.round((6000000 * 120) / 6));
});

test('the byte total or yt-dlp estimate wins over our own extrapolation', () => {
  const s = fresh();
  feed(s, ['1024 62914560 NA NA NA']);
  assert.equal(s.size, 62914560);
  const t = fresh();
  feed(t, ['1000000 NA 36000000.0 0 120']);
  assert.equal(t.size, 36000000);
});

test('an audio stream starting after the video adds what the video really weighed', () => {
  const s = fresh();
  feed(s, ['500000000 500000000 NA NA NA']);
  feed(s, ['1000 80000000 NA NA NA'], false); // second stream restarts near zero
  assert.equal(s.ytd.base, 500000000);
  assert.equal(s.size, 580000000);
  assert.equal(s.ytd.base + s.ytd.cur, 500001000);
});

test('the first line of a resumed run is not counted as speed, the next ones are', () => {
  const s = fresh();
  const [first, second, third] = feed(s, ['271244144 NA 3815647496 215 3008', '271911792 NA 3683838307 221 3008', '271900000 NA 3683838307 221 3008']);
  assert.equal(first, 0);
  assert.equal(second, 271911792 - 271244144);
  assert.equal(third, 0, 'a dip is not negative speed');
});

test('a size never falls below what is already downloaded', () => {
  const s = fresh();
  feed(s, ['900 NA 500 NA NA']);
  assert.equal(s.size, 900);
});
