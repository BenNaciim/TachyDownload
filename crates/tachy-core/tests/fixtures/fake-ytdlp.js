'use strict';

// Stand-in for yt-dlp.exe: prints what the real one prints for a few situations, chosen by FAKE_MODE.
const fs = require('node:fs');
const path = require('node:path');

const args = process.argv.slice(2);
if (args.includes('--flat-playlist')) {
  // A playlist listing: "playlist title<TAB>url<TAB>title" per video. FAKE_PLAYLIST=empty lists nothing.
  if (process.env.FAKE_PLAYLIST !== 'empty') {
    for (const [id, title] of [['aaaaaaaaaaa', 'Premier'], ['bbbbbbbbbbb', 'Second']]) {
      process.stdout.write(['Ma liste', `https://www.youtube.com/watch?v=${id}`, title].join(String.fromCharCode(9)) + '\n');
    }
  }
  process.exit(0);
}
if (args.includes('-J')) {
  // The probe for a plain-file site: one http file, nothing to merge.
  if (process.env.FAKE_DIRECT_URL) {
    process.stdout.write(JSON.stringify({ url: process.env.FAKE_DIRECT_URL, protocol: 'https', ext: 'mp4', title: 'Direct clip', http_headers: { 'User-Agent': 'fake' } }));
    process.exit(0);
  }
  if (process.env.FAKE_PROBE) {
    process.stdout.write(JSON.stringify({ title: 'Clip', duration: 61, formats: [{ vcodec: 'avc1', acodec: 'none', height: 1080 }, { vcodec: 'avc1', acodec: 'none', height: 720 }, { vcodec: 'none', acodec: 'mp4a', language: 'fr', format_note: 'Français, low' }] }));
    process.exit(0);
  }
  if (process.env.FAKE_MODE === 'fail') {
    process.stderr.write('ERROR: [youtube] abcdefghijk: Sign in to confirm you’re not a bot\n');
    process.exit(1);
  }
  process.stderr.write('ERROR: no probe here\n');
  process.exit(1);
}
const dir = args[args.indexOf('-P') + 1];
if (process.env.FAKE_ARGS_FILE) fs.writeFileSync(process.env.FAKE_ARGS_FILE, JSON.stringify(args));
const mode = process.env.FAKE_MODE || 'ok';
// The file is named the way the -o template says, as the real one does (title "Episode", id "abcdefghijk").
const template = args.includes('-o') ? args[args.indexOf('-o') + 1] : '%(title).150B [%(id)s].%(ext)s';
const real = path.join(dir, template.replace('%(title).150B', 'Episode').replace('%(id)s', 'abcdefghijk').replace('%(ext)s', 'mp4').replace(/%%/g, '%'));
const out = (l) => process.stdout.write(`${l}\n`);

if (mode === 'fail') {
  process.stderr.write('ERROR: [youtube] abcdefghijk: Sign in to confirm you’re not a bot\n');
  process.exit(1);
}

out('DMT Episode');
out('DMP 1024 NA 2097152.0 1 8');
out('DMP 1048576 NA 2097152.0 4 8');
if (mode === 'nofile') {
  out(`DMF ${path.join(dir, 'ghost.mp4')}`);
  process.exit(0);
}
fs.mkdirSync(dir, { recursive: true });
// Like the real one: a file already there is "already downloaded" and left alone, unless told to overwrite.
if (!fs.existsSync(real) || args.includes('--force-overwrites')) fs.writeFileSync(real, Buffer.alloc(2 * 1024 * 1024, Number(process.env.FAKE_FILL || 7)));
// "renamed": the printed path lost a character on the way, as happened with the Windows code page.
out(`DMF ${mode === 'renamed' ? path.join(dir, 'Episode ? [abcdefghijk].mp4') : real}`);
