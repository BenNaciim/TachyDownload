'use strict';

// Stand-in for yt-dlp.exe: prints what the real one prints for a few situations, chosen by FAKE_MODE.
const fs = require('node:fs');
const path = require('node:path');

const args = process.argv.slice(2);
const dir = args[args.indexOf('-P') + 1];
if (process.env.FAKE_ARGS_FILE) fs.writeFileSync(process.env.FAKE_ARGS_FILE, JSON.stringify(args));
const mode = process.env.FAKE_MODE || 'ok';
const real = path.join(dir, 'Episode [abcdefghijk].mp4');
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
fs.writeFileSync(real, Buffer.alloc(2 * 1024 * 1024, 7));
// "renamed": the printed path lost a character on the way, as happened with the Windows code page.
out(`DMF ${mode === 'renamed' ? path.join(dir, 'Episode ? [abcdefghijk].mp4') : real}`);
