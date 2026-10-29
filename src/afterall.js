'use strict';

const ACTIONS = ['none', 'quit', 'sleep', 'shutdown'];

/** True when nothing is running or waiting to run (paused items are the user's choice). */
function isIdle(tasks) {
  return !tasks.some((t) => ['downloading', 'queued', 'scheduled'].includes(t.status));
}

/** The system command behind an end-of-queue action; `quit` is handled by the app itself. */
function commandFor(action) {
  if (action === 'sleep') return { cmd: 'rundll32.exe', args: ['powrprof.dll,SetSuspendState', '0', '1', '0'] };
  if (action === 'shutdown') {
    return { cmd: 'shutdown', args: ['/s', '/t', '60', '/c', 'Tachy Download : téléchargements terminés'] };
  }
  return null;
}

module.exports = { ACTIONS, isIdle, commandFor };
