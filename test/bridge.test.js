'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const ROOT = path.join(__dirname, '..');
const BRIDGE = fs.readFileSync(path.join(ROOT, 'ui', 'renderer', 'dm-bridge.js'), 'utf8');

/** What crosses the IPC is JSON: compare payloads the way the wire sees them (and across vm realms). */
const wire = (x) => JSON.parse(JSON.stringify(x));

/** Method names an Electron preload hands to contextBridge.exposeInMainWorld(name, { ... }). */
// The surface the window scripts rely on, as the Electron preload scripts exposed it (frozen at 1.2.0).
const DM_METHODS = [
  'add', 'checkUpdate', 'copyDiagnostic', 'copyToken', 'fileIcon', 'installTool', 'installUpdate', 'list', 'logins',
  'onMenu', 'onRemoved', 'onSettings', 'onTools', 'onUpdate', 'onUpdateAvail', 'onUpdateProgress', 'open', 'openPage',
  'pause', 'pickDir', 'redownload', 'remove', 'removeLogin', 'resume', 'reveal', 'saveLogin', 'setLimit', 'setSettings',
  'state', 'toolsStatus',
];
const CONFIRM_METHODS = ['answer', 'category', 'init', 'pickDir', 'probe'];

/** Runs the bridge in a page whose `window.__TAURI__` is a recording stand-in for Tauri's global API. */
function load({ dm, invoke = async () => undefined, holdListen = false } = {}) {
  const calls = [];
  const handlers = {};
  const unlistened = [];
  const pending = [];
  const window = {
    __TAURI__: {
      core: {
        invoke: (command, args) => {
          calls.push([command, args]);
          return invoke(command, args);
        },
      },
      event: {
        listen: (event, handler) => {
          handlers[event] = handler;
          const unlisten = () => unlistened.push(event);
          if (!holdListen) return Promise.resolve(unlisten);
          return new Promise((resolve) => pending.push(() => resolve(unlisten)));
        },
      },
    },
  };
  if (dm) window.dm = dm;
  vm.runInNewContext(BRIDGE, { window, console, Error });
  return { window, calls, handlers, unlistened, settle: () => pending.splice(0).forEach((resolve) => resolve()) };
}

const tick = () => new Promise((resolve) => setImmediate(resolve));

test('does nothing under Electron, where the preload already exposes window.dm', () => {
  const electronDm = { list: () => 'electron' };
  const { window, calls } = load({ dm: electronDm });
  assert.equal(window.dm, electronDm);
  assert.equal(window.confirmApi, undefined);
  assert.deepEqual(calls, []);
});

test('does nothing outside Tauri (no window.__TAURI__): the Electron confirmation window brings its own confirmApi', () => {
  const window = {};
  vm.runInNewContext(BRIDGE, { window, console, Error });
  assert.equal(window.dm, undefined);
  assert.equal(window.confirmApi, undefined);
});

test('exposes exactly the methods the Electron preload scripts did', () => {
  const { window } = load();
  assert.deepEqual(Object.keys(window.dm).sort(), DM_METHODS);
  assert.deepEqual(Object.keys(window.confirmApi).sort(), CONFIRM_METHODS);
});

test('turns positional arguments into the named arguments of the Tauri commands', async () => {
  const { window, calls } = load();
  const { dm, confirmApi } = window;
  await dm.list();
  await dm.state();
  await dm.add(['https://a.test/f.zip'], { kind: 'hls' });
  await dm.toolsStatus();
  await dm.installTool('ytdlp');
  await dm.openPage('id1');
  await dm.fileIcon('id1');
  await dm.pause('id1');
  await dm.resume('id1');
  await dm.redownload('id1');
  await dm.remove('id1', true);
  await dm.setLimit('id1', 512);
  await dm.reveal('id1');
  await dm.open('id1');
  await dm.setSettings({ connections: 4 });
  await dm.pickDir();
  await dm.copyToken();
  await dm.copyDiagnostic();
  await dm.checkUpdate();
  await dm.installUpdate();
  await dm.logins();
  await dm.saveLogin({ host: 'a.test', type: 'basic', user: 'u', secret: 's' });
  await dm.removeLogin('a.test');
  await confirmApi.init();
  await confirmApi.category('Vidéos');
  await confirmApi.pickDir('C:\\Users\\x');
  await confirmApi.probe();
  await confirmApi.answer({ action: 'now', filename: 'a.zip' });
  assert.deepEqual(wire(calls), [
    ['dm_list', {}],
    ['dm_state', {}],
    ['dm_add', { urls: ['https://a.test/f.zip'], opts: { kind: 'hls' } }],
    ['dm_tools_status', {}],
    ['dm_tools_install', { name: 'ytdlp' }],
    ['dm_open_page', { id: 'id1' }],
    ['dm_file_icon', { id: 'id1' }],
    ['dm_pause', { id: 'id1' }],
    ['dm_resume', { id: 'id1' }],
    ['dm_redownload', { id: 'id1' }],
    ['dm_remove', { id: 'id1', deleteFile: true }],
    ['dm_set_limit', { id: 'id1', kbps: 512 }],
    ['dm_reveal', { id: 'id1' }],
    ['dm_open', { id: 'id1' }],
    ['dm_settings_set', { patch: { connections: 4 } }],
    ['dm_pick_dir', {}],
    ['dm_copy_token', {}],
    ['dm_copy_diagnostic', {}],
    ['dm_update_check', {}],
    ['dm_update_install', {}],
    ['dm_logins_list', {}],
    ['dm_logins_save', { entry: { host: 'a.test', type: 'basic', user: 'u', secret: 's' } }],
    ['dm_logins_remove', { host: 'a.test' }],
    ['confirm_init', {}],
    ['confirm_category', { name: 'Vidéos' }],
    ['confirm_pick_dir', { current: 'C:\\Users\\x' }],
    ['confirm_probe', {}],
    ['confirm_answer', { answer: { action: 'now', filename: 'a.zip' } }],
  ]);
});

test('resolves with what the command returned', async () => {
  const { window } = load({ invoke: async () => [{ id: 'a' }] });
  assert.deepEqual(wire(await window.dm.list()), [{ id: 'a' }]);
});

test('a refused command rejects with an Error that carries the plain French message', async () => {
  for (const refusal of ['Paramètre invalide', { message: 'Paramètre invalide' }, new Error('Paramètre invalide')]) {
    const { window } = load({ invoke: async () => Promise.reject(refusal) });
    await assert.rejects(window.dm.pause('x'), (err) => err instanceof Error && err.message === 'Paramètre invalide');
  }
});

test('event subscriptions deliver only the payload, under the dm:* event names', async () => {
  const { window, handlers } = load();
  const names = {
    onUpdate: 'dm:update',
    onRemoved: 'dm:removed',
    onSettings: 'dm:settings',
    onTools: 'dm:tools',
    onUpdateAvail: 'dm:updateAvail',
    onUpdateProgress: 'dm:updateProgress',
    onMenu: 'dm:menu',
  };
  for (const [method, event] of Object.entries(names)) {
    const seen = [];
    window.dm[method]((payload) => seen.push(payload));
    handlers[event]({ event, id: 1, payload: { n: 7 } });
    assert.deepEqual(wire(seen), [{ n: 7 }], method);
  }
});

test('an unsubscribe function comes back at once and stops delivery', async () => {
  const { window, handlers, unlistened } = load();
  const seen = [];
  const off = window.dm.onUpdate((p) => seen.push(p));
  assert.equal(typeof off, 'function');
  await tick();
  handlers['dm:update']({ payload: 1 });
  off();
  handlers['dm:update']({ payload: 2 }); // a late event must not reach the page
  off(); // twice is harmless
  assert.deepEqual(seen, [1]);
  assert.deepEqual(unlistened, ['dm:update']);
});

test('unsubscribing before Tauri has registered the listener still removes it once it has', async () => {
  const { window, handlers, unlistened, settle } = load({ holdListen: true });
  const seen = [];
  const off = window.dm.onSettings((p) => seen.push(p));
  off();
  assert.deepEqual(unlistened, []);
  settle();
  await tick();
  handlers['dm:settings']({ payload: 'late' });
  assert.deepEqual(seen, []);
  assert.deepEqual(unlistened, ['dm:settings']);
});
