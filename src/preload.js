'use strict';

const { contextBridge, ipcRenderer } = require('electron');

const on = (channel) => (cb) => {
  const handler = (_e, payload) => cb(payload);
  ipcRenderer.on(channel, handler);
  return () => ipcRenderer.removeListener(channel, handler);
};

contextBridge.exposeInMainWorld('dm', {
  list: () => ipcRenderer.invoke('dm:list'),
  state: () => ipcRenderer.invoke('dm:state'),
  add: (urls, opts) => ipcRenderer.invoke('dm:add', urls, opts),
  toolsStatus: () => ipcRenderer.invoke('dm:tools:status'),
  installTool: (name) => ipcRenderer.invoke('dm:tools:install', name),
  onTools: on('dm:tools'),
  openPage: (id) => ipcRenderer.invoke('dm:openPage', id),
  fileIcon: (id) => ipcRenderer.invoke('dm:fileIcon', id),
  pause: (id) => ipcRenderer.invoke('dm:pause', id),
  resume: (id) => ipcRenderer.invoke('dm:resume', id),
  remove: (id, deleteFile) => ipcRenderer.invoke('dm:remove', id, deleteFile),
  reveal: (id) => ipcRenderer.invoke('dm:reveal', id),
  open: (id) => ipcRenderer.invoke('dm:open', id),
  setSettings: (patch) => ipcRenderer.invoke('dm:settings:set', patch),
  pickDir: () => ipcRenderer.invoke('dm:pickDir'),
  copyToken: () => ipcRenderer.invoke('dm:copyToken'),
  checkUpdate: () => ipcRenderer.invoke('dm:update:check'),
  installUpdate: () => ipcRenderer.invoke('dm:update:install'),
  onUpdateAvail: on('dm:updateAvail'),
  onUpdateProgress: on('dm:updateProgress'),
  onUpdate: on('dm:update'),
  onRemoved: on('dm:removed'),
  onSettings: on('dm:settings'),
});
