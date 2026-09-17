// `xdotool`-driven native-dialog interaction for the VP-S6 scenario
// (design.md D7): this bare Xvfb has no window manager, so the usual
// "activate the active/focused window" idioms are unavailable or
// unreliable here. Exercises real child processes and a real X display --
// no repository test drives this; `vp-s6-scenario.mjs`'s own live run is
// this module's only exercise (design.md's Testing Strategy: "the live
// browser-driving path only ... is evidence machinery, not a repository
// test").

import { execFileSync } from 'node:child_process';

/**
 * Find a top-level window by its exact `WM_NAME`, retrying until
 * `timeoutMs` elapses (the native dialog's window is not mapped the
 * instant its opening button is clicked). Deliberately not
 * `xdotool getwindowfocus`/`getactivewindow`: this bare Xvfb has no window
 * manager, so `_NET_ACTIVE_WINDOW` is unavailable, and `XGetInputFocus`
 * itself never resolves past the `PointerRoot` placeholder here regardless
 * of which window GTK actually mapped -- searching by name and then
 * `windowfocus`ing that exact id (an `XSetInputFocus` call, not a query)
 * is the one reliable path found empirically in this environment.
 */
export function findWindowByName(name, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const output = execFileSync('xdotool', ['search', '--name', name], { encoding: 'utf8' }).trim();
      const id = output.split('\n')[0];
      if (id) return id;
    } catch {
      /* no matching window yet */
    }
    if (Date.now() >= deadline) {
      throw new Error(`window not found within ${timeoutMs}ms: ${name}`);
    }
    execFileSync('sleep', ['0.2']);
  }
}

/**
 * Drive a native GTK chooser already known to be open under
 * `windowTitle`: `Ctrl+L` opens its location bar, `path` is typed into it,
 * `Return` navigates to (and, for an exact directory/file match, selects
 * and closes) it.
 */
export function driveChooserWithPath(windowTitle, path, timeoutMs) {
  const windowId = findWindowByName(windowTitle, timeoutMs);
  execFileSync('xdotool', ['windowfocus', windowId]);
  execFileSync('xdotool', ['key', 'ctrl+l']);
  // `Ctrl+L` opens the location bar as its own short-lived popup, which
  // needs a moment to map and take its own input focus before it can
  // receive keystrokes -- verified empirically in this environment: typing
  // immediately after `key ctrl+l` with no pause raced ahead of it.
  execFileSync('sleep', ['0.3']);
  execFileSync('xdotool', ['type', '--delay', '20', path]);
  execFileSync('sleep', ['0.3']);
  execFileSync('xdotool', ['key', 'Return']);
}
