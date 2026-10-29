'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('confirmApi', {
  init: () => ipcRenderer.invoke('dm:confirm:init'),
  category: (name) => ipcRenderer.invoke('dm:confirm:category', name),
  pickDir: (current) => ipcRenderer.invoke('dm:confirm:pickDir', current),
  answer: (a) => ipcRenderer.invoke('dm:confirm:answer', a),
});
