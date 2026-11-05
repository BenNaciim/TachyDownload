'use strict';

// Under Tauri there is no preload: this script gives the page the same `window.dm` and `window.confirmApi` that
// src/preload.js and src/confirm-preload.js give it under Electron, on top of Tauri's global IPC API.
// It does nothing under Electron (window.dm already exists) and outside Tauri (no window.__TAURI__).
(() => {
  if (window.dm || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  // The Rust side refuses with the plain French message; the page strips Electron's own prefix, so add none.
  const fail = (err) => {
    throw err instanceof Error ? err : new Error(typeof err === 'string' ? err : (err && err.message) || String(err));
  };
  const call = (command, args = {}) => invoke(command, args).catch(fail);

  // Electron hands back the unsubscribe function at once; Tauri's listen() only promises one.
  const on = (event) => (callback) => {
    let unlisten = null;
    let stopped = false;
    listen(event, (e) => {
      if (!stopped) callback(e.payload);
    })
      .then((un) => {
        unlisten = un;
        if (stopped) un();
      })
      .catch((err) => console.error('dm-bridge: cannot listen to', event, err));
    return () => {
      if (stopped) return;
      stopped = true;
      if (unlisten) unlisten();
    };
  };

  window.dm = Object.freeze({
    list: () => call('dm_list'),
    state: () => call('dm_state'),
    add: (urls, opts) => call('dm_add', { urls, opts }),
    toolsStatus: () => call('dm_tools_status'),
    installTool: (name) => call('dm_tools_install', { name }),
    onTools: on('dm:tools'),
    openPage: (id) => call('dm_open_page', { id }),
    fileIcon: (id) => call('dm_file_icon', { id }),
    pause: (id) => call('dm_pause', { id }),
    resume: (id) => call('dm_resume', { id }),
    redownload: (id) => call('dm_redownload', { id }),
    remove: (id, deleteFile) => call('dm_remove', { id, deleteFile }),
    setLimit: (id, kbps) => call('dm_set_limit', { id, kbps }),
    reveal: (id) => call('dm_reveal', { id }),
    open: (id) => call('dm_open', { id }),
    setSettings: (patch) => call('dm_settings_set', { patch }),
    pickDir: () => call('dm_pick_dir'),
    copyToken: () => call('dm_copy_token'),
    copyDiagnostic: () => call('dm_copy_diagnostic'),
    checkUpdate: () => call('dm_update_check'),
    installUpdate: () => call('dm_update_install'),
    onUpdateAvail: on('dm:updateAvail'),
    onUpdateProgress: on('dm:updateProgress'),
    onUpdate: on('dm:update'),
    onRemoved: on('dm:removed'),
    onSettings: on('dm:settings'),
    onMenu: on('dm:menu'),
    logins: () => call('dm_logins_list'),
    saveLogin: (entry) => call('dm_logins_save', { entry }),
    removeLogin: (host) => call('dm_logins_remove', { host }),
  });

  window.confirmApi = Object.freeze({
    init: () => call('confirm_init'),
    category: (name) => call('confirm_category', { name }),
    pickDir: (current) => call('confirm_pick_dir', { current }),
    probe: () => call('confirm_probe'),
    answer: (answer) => call('confirm_answer', { answer }),
  });
})();
