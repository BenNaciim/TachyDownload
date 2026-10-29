const fs = require('fs');
const path = require('path');

// Retire le fichier de licences Chromium (5 Mo), inutile au fonctionnement.
exports.default = async function afterPack(context) {
  const file = path.join(context.appOutDir, 'LICENSES.chromium.html');
  fs.rmSync(file, { force: true });
};
