'use strict';

/**
 * Overall progress of what is being downloaded, for the taskbar button: { mode, value } where mode is
 * 'none' (nothing running), 'indeterminate' (sizes unknown) or 'normal' with value between 0 and 1.
 */
function overallProgress(tasks) {
  const running = tasks.filter((t) => t.status === 'downloading');
  if (running.length === 0) return { mode: 'none', value: -1 };
  let total = 0;
  let got = 0;
  for (const t of running) {
    if (t.size > 0) {
      total += t.size;
      got += Math.min(t.downloaded ?? 0, t.size);
    }
  }
  if (total === 0) return { mode: 'indeterminate', value: 2 };
  // A video with no known size alongside sized ones only shifts the bar a little: the bar stays honest
  // about what is known.
  return { mode: 'normal', value: Math.min(1, got / total) };
}

module.exports = { overallProgress };
