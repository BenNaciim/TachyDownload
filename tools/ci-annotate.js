'use strict';

// Turns the failing tests of a node:test run (TAP output saved in a file) into a GitHub annotation, so the cause
// shows on the run's page without opening the log. Usage: node tools/ci-annotate.js test-output.txt
const fs = require('node:fs');

const file = process.argv[2];
let text = '';
try {
  text = fs.readFileSync(file, 'utf8');
} catch {
  console.log(`::error title=npm test::no output was saved (${file})`);
  process.exit(0);
}
const blocks = [];
let current = null;
for (const line of text.split(/\r?\n/)) {
  if (/^\s*not ok /.test(line)) {
    current = [line.trim()];
    blocks.push(current);
  } else if (current && /^\s*\.\.\.\s*$/.test(line)) {
    current = null;
  } else if (current && /^\s*(error|failureType|location|duration_ms):/.test(line)) {
    current.push(line.trim());
  }
}
const message = (blocks.length ? blocks.map((b) => b.join('\n')).join('\n\n') : text.slice(-3000)).slice(0, 3500);
const encoded = message.replace(/%/g, '%25').replace(/\r/g, '%0D').replace(/\n/g, '%0A');
console.log(`::error title=npm test, ${blocks.length} failing::${encoded}`);
