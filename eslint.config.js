'use strict';

const js = require('@eslint/js');
const globals = require('globals');

// Window scripts are plain <script> tags: they share their top-level names, so a name defined in one file
// and used in another is expected, not an error.
const shared = {
  FileTypes: 'readonly',
  getTheme: 'readonly', applyTheme: 'readonly', getLook: 'readonly', setLook: 'readonly', applyLook: 'readonly',
  LOOK_DEFAULTS: 'readonly', THEME_KEY: 'readonly', lookKey: 'readonly',
  icon: 'readonly', ICONS: 'readonly',
  bytesParts: 'readonly', fmtBytes: 'readonly', fmtSpeed: 'readonly', fmtEta: 'readonly', fmtDate: 'readonly',
};

module.exports = [
  { ignores: ['dist/**', 'node_modules/**', 'build/**', 'docs/**'] },
  js.configs.recommended,
  {
    files: ['**/*.js'],
    languageOptions: { ecmaVersion: 2023, sourceType: 'commonjs', globals: { ...globals.node } },
    rules: {
      'no-unused-vars': ['warn', { args: 'none', caughtErrors: 'none' }],
      'no-empty': ['error', { allowEmptyCatch: true }],
    },
  },
  {
    files: ['src/renderer/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser, ...shared } },
    rules: { 'no-unused-vars': 'off', 'no-redeclare': 'off' }, // names are used by the sibling scripts
  },
  {
    files: ['src/shared/*.js'],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    files: ['extension/**/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser, chrome: 'readonly', browser: 'readonly' } },
  },
];
