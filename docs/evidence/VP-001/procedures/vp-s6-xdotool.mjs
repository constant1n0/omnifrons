// `xdotool`-driven native-dialog interaction for the VP-S6 scenario
// (design.md D7). This module makes no assumption either way about a window
// manager being present on the target display -- `--onlyvisible` reads the X
// server's own `map_state` directly (needs no WM), and `windowfocus` below is
// `XSetInputFocus`, also WM-independent. But the CI scenario step's own Xvfb
// display now runs a minimal window manager (openbox, wired in
// tauri-build.yml's "VP-001 VP-S6 scenario" step only, not the earlier
// feasibility-gate step, which never drives a chooser); a local invocation
// of this script against a bare Xvfb with no WM is expected to work the same
// way this module always has, but is no longer how CI itself exercises it.
// No repository test drives the X11 interaction itself (`vp-s6-scenario.mjs`'s
// own live run under Xvfb is its only exercise); the pure decision/formatting
// helpers below have no such exemption and are covered by
// `vp-s6-xdotool.test.mjs`.
//
// Hardened after CI run 35252892166 (pinned `ubuntu-24.04`, filed as
// VP-001-VP-S6-01, `uncertain`): the transcript shows `gate=f1-session-
// created` immediately followed by `Gtk-CRITICAL gtk_widget_event: assertion
// 'WIDGET_REALIZED_FOR_EVENT (widget, event)' failed` then
// `blocker=workspace-chooser-timeout`. Diagnosis: the old `findWindowByName`
// trusted the *first* id `xdotool search` reported, which can exist before
// GTK realizes its widgets -- sending keys then trips that assertion, and on
// a bare Xvfb a dropped keystroke is silent. Slower/loaded CI widens a race
// this much faster local machine never hits.
//
// Run 35440083284 (after that hardening, `x11-utils` present) showed the
// stability-polled window genuinely `map_state=IsViewable` and still tripped
// the same assertion on every one of its three attempts: waiting longer for
// the window to be viewable was not enough. Working theory: with no window
// manager on the display, nothing ever delivers the WM-mediated focus-in GTK
// expects before it realizes a dialog's focus widget, so a keystroke can
// still reach an unrealized widget even once `xwininfo` calls the window
// viewable. Local testing on this change (both with and without openbox
// running on a scratch Xvfb display, several runs each) never reproduced the
// CI assertion either way -- a materially faster, unloaded machine, matching
// this file's own "Slower/loaded CI widens a race" note above. Openbox is
// added to CI as a targeted hardening against the theorized mechanism, not a
// locally verified fix for it. `sendChooserInput` below still uses
// `windowfocus`, not `windowactivate`: re-verified with openbox running
// (several passing local runs, see the branch's own commit), it kept working
// exactly as before, and swapping it for `windowactivate` showed no
// measurable difference in reliability, so there is no local evidence to
// prefer it over the mechanism already verified below.

import { execFileSync } from 'node:child_process';

// A window id must be reported, visible, and unchanged for this many
// consecutive polls before anything is sent to it (guards the CI race
// above). `Ctrl+L`'s location-bar popup is unnamed and AT-SPI is
// unavailable here (same transcript: "AT-SPI: Error retrieving
// accessibility bus address"), so its settle wait polls the X window tree
// for a new entry instead -- a proxy, not a hard requirement, since the
// per-attempt close verification below is the real correctness check.
const STABILITY_REQUIRED_POLLS = 3;
const STABILITY_POLL_INTERVAL_MS = 100;
const LOCATION_BAR_SETTLE_TIMEOUT_MS = 1_000;
const LOCATION_BAR_POLL_INTERVAL_MS = 50;
const TYPE_DELAY_MS = 20;
// The location-bar's completion navigates the browser view as each path
// segment is typed; for a *file* target that leaves a moment of internal
// GTK state settling before `Return`, with no externally observable signal
// to poll for -- dropping this entirely reproduced a
// `gtk_tree_model_get_iter_first` assertion storm and a chooser that never
// closed (verified empirically), so it stays a fixed pause.
const POST_TYPE_SETTLE_MS = 300;
const DIALOG_CLOSE_TIMEOUT_MS = 3_000;
const DIALOG_CLOSE_POLL_INTERVAL_MS = 150;
// A retry here is UI-input robustness, not a retry of the scenario: the
// scenario still runs exactly once, `derive()` still decides the outcome
// exactly once, and a chooser that never closes still surfaces as the same
// disclosed `blocker=` its caller already names.
const MAX_DIALOG_ATTEMPTS = 3;
// Distinguishes "xwininfo could not tell us" from "xwininfo says the window
// is not viewable": the first must not be read as the second.
const MAP_STATE_PROBE_UNAVAILABLE = 'probe-unavailable';
const DIAGNOSTIC_MAX_LINES = 60;
const DIAGNOSTIC_MAX_LINE_LENGTH = 200;

// -- Pure helpers (unit-tested in vp-s6-xdotool.test.mjs) -------------------

/** Parse `xdotool search` stdout into an ordered list of window ids. */
export function parseSearchIds(output) {
  return output
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** Parse `xwininfo -id <id>`'s "Map State" line, or `null` if absent
 * (for example the window no longer exists and `xwininfo` errored). */
export function parseMapState(xwininfoOutput) {
  const match = xwininfoOutput.match(/Map State:\s*(\S+)/);
  return match ? match[1] : null;
}

/**
 * Decide whether a window `xdotool search --onlyvisible` reported may be
 * driven, given what `xwininfo` could tell us about it.
 *
 * `xwininfo` ships in `x11-utils`, which is NOT installed by default on a
 * GitHub `ubuntu-24.04` runner: CI run 35437413086 spent its whole chooser
 * budget rejecting a window that was there, because every map-state probe
 * failed with `ENOENT` and the caller read that as "not viewable yet".
 * A corroborating tool that is absent must weaken the check, never fail it
 * closed against the only signal we do have -- but the weaker check is
 * disclosed in the transcript rather than passed off as the strong one.
 */
export function isWindowDrivable(mapState) {
  if (mapState === MAP_STATE_PROBE_UNAVAILABLE) return { drivable: true, degraded: true };
  return { drivable: mapState === 'IsViewable', degraded: false };
}

/** Count window ids in an `xwininfo -root -tree` snapshot -- a coarse but
 * genuinely observable proxy for "a new window appeared". The negative
 * lookbehind matters: geometry strings (`WIDTHxHEIGHT+X+Y`, e.g.
 * "400x300+50+50") routinely contain a literal "0x" whenever a dimension
 * ends in zero, which a bare `/0x[0-9a-fA-F]+/` would miscount. */
export function countWindowIdsInTree(treeOutput) {
  const matches = treeOutput.match(/(?<![0-9a-fA-F])0x[0-9a-fA-F]+/g);
  return matches ? matches.length : 0;
}

/** Advance the window-id stability streak: a `null` candidate (not found,
 * or found but not viewable) resets it; the same id extends it; a
 * different id restarts it at 1. */
export function nextStabilityState(previous, candidateId) {
  if (!candidateId) return { id: null, streak: 0 };
  if (previous.id === candidateId) return { id: candidateId, streak: previous.streak + 1 };
  return { id: candidateId, streak: 1 };
}

/** Redact absolute home-directory-rooted paths before they reach the
 * transcript -- a runner's own username/work directory is exactly the kind
 * of provenance detail `docs/evidence/VP-001/README.md` requires stay out
 * of anything committed. */
export function redactProvenance(text) {
  return text
    .replace(/\/home\/[^\s"'):]+/g, '<home>')
    .replace(/\/Users\/[^\s"'):]+/g, '<home>')
    .replace(/\/root(?:\/[^\s"'):]*)?/g, '<root>');
}

/** Collapse multi-line text to at most `maxLines` lines, each truncated to
 * `maxLineLength`, joined with `" | "` so the result stays one transcript
 * line (the transcript is one `key=value` line each). */
export function compactLines(text, maxLines, maxLineLength) {
  return text
    .split('\n')
    .slice(0, maxLines)
    .map((line) => (line.length > maxLineLength ? `${line.slice(0, maxLineLength)}...` : line))
    .join(' | ');
}

/** Format the final-failure diagnostic lines: window/attempts/map-state,
 * plus a redacted, compacted window-tree snapshot -- everything a future CI
 * failure needs to be debuggable from the transcript alone. */
export function formatDiagnostics({ windowTitle, windowId, attempts, mapState, tree }) {
  return [
    `window_title=${windowTitle} window_id=${windowId} attempts=${attempts} map_state=${mapState}`,
    `tree=${compactLines(redactProvenance(tree), DIAGNOSTIC_MAX_LINES, DIAGNOSTIC_MAX_LINE_LENGTH)}`,
  ];
}

// -- Impure X11/process plumbing (no repository test) -----------------------

function xdo(args) {
  return execFileSync('xdotool', args, { encoding: 'utf8' });
}

function sleepMs(ms) {
  execFileSync('sleep', [(ms / 1000).toString()]);
}

// `--onlyvisible` checks the X server's own `map_state`, set directly by
// the client that owns the window -- unlike `WM_STATE`, it needs no window
// manager to be meaningful, which is why it works on this bare Xvfb.
function searchVisibleWindowIds(name) {
  try {
    return parseSearchIds(xdo(['search', '--onlyvisible', '--name', name]));
  } catch {
    return [];
  }
}

function windowMapState(id) {
  try {
    return parseMapState(execFileSync('xwininfo', ['-id', id], { encoding: 'utf8' }));
  } catch (error) {
    // `xwininfo` missing entirely is a different fact from a window that
    // vanished, and the caller must not conflate them.
    return error?.code === 'ENOENT' ? MAP_STATE_PROBE_UNAVAILABLE : null;
  }
}

function windowTreeSnapshot() {
  try {
    return execFileSync('xwininfo', ['-root', '-tree'], { encoding: 'utf8' });
  } catch (error) {
    return `<xwininfo -root -tree unavailable: ${error.message}>`;
  }
}

/** Wait for a chooser window named `name` to be reported, `IsViewable`, and
 * unchanged for `STABILITY_REQUIRED_POLLS` consecutive polls. Returns the
 * window id, or `null` on timeout. Replaces "trust the first id ever
 * reported", which raced GTK's own widget realization in CI run
 * 35252892166. */
function waitForUsableWindow(name, timeoutMs, emit = defaultEmit) {
  const deadline = Date.now() + timeoutMs;
  let state = { id: null, streak: 0 };
  let disclosedDegradedProbe = false;
  for (;;) {
    const ids = searchVisibleWindowIds(name);
    const candidateId = ids.length > 0 ? ids[0] : null;
    let viewable = false;
    if (candidateId !== null) {
      const { drivable, degraded } = isWindowDrivable(windowMapState(candidateId));
      viewable = drivable;
      if (degraded && !disclosedDegradedProbe) {
        // Say which check actually ran, rather than letting the transcript
        // imply the stronger one did.
        emit('dialog_note', 'xwininfo-unavailable: viewability from xdotool --onlyvisible only');
        disclosedDegradedProbe = true;
      }
    }
    state = nextStabilityState(state, viewable ? candidateId : null);
    if (state.streak >= STABILITY_REQUIRED_POLLS) return state.id;
    if (Date.now() >= deadline) return null;
    sleepMs(STABILITY_POLL_INTERVAL_MS);
  }
}

/** Bounded wait for `id` to stop being a viewable window (destroyed or
 * unmapped) -- the observable confirmation that a `Return` actually landed,
 * instead of assuming success once keys have been sent. */
function waitForWindowGone(id, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const state = windowMapState(id);
    if (state === null || state !== 'IsViewable') return true;
    if (Date.now() >= deadline) return false;
    sleepMs(DIALOG_CLOSE_POLL_INTERVAL_MS);
  }
}

function waitForWindowCountAtLeast(minCount, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (countWindowIdsInTree(windowTreeSnapshot()) >= minCount) return true;
    if (Date.now() >= deadline) return false;
    sleepMs(LOCATION_BAR_POLL_INTERVAL_MS);
  }
}

/** Send one `Ctrl+L` / type-path / `Return` sequence to `windowId`.
 *
 * `--window`-targeted `key`/`type` (`XSendEvent` straight to that window,
 * bypassing XTEST) was tried here first, expecting it to beat focus-
 * dependent global input with no window manager present. Verification
 * disproved that: targeted at the "Open File" chooser it reliably produced
 * `gtk_tree_model_get_iter_first: assertion 'GTK_IS_TREE_MODEL
 * (tree_model)' failed` and the dialog never closed, while `windowfocus`
 * (`XSetInputFocus`) plus *global* `key`/`type` -- the original module's own
 * mechanism -- passed both choosers on the exact same artifact. Kept for
 * that reason, not out of inertia.
 */
/**
 * The three attempts are three DIFFERENT strategies, not the same one
 * three times: CI runs 35252892166 and 35440083284 both burned all three
 * identical attempts against the same failure, which taught us nothing
 * about the mechanism. Each attempt now varies one thing, so a single run
 * says which input path a runner accepts.
 *
 * - `focus-ctrl-l`: what has always worked on a developer machine.
 * - `activate-ctrl-l`: `windowactivate` asks the window manager to focus
 *   the window, which `windowfocus`'s bare `XSetInputFocus` never does; it
 *   is only meaningful now that the scenario display runs one.
 * - `focus-type-path`: skips `Ctrl+L` entirely. A GTK file chooser opens
 *   its location entry on its own when typing begins with `/`, so this
 *   attempt tests whether the accelerator, rather than the keystrokes, is
 *   what a runner drops.
 */
const CHOOSER_INPUT_STRATEGIES = ['focus-ctrl-l', 'activate-ctrl-l', 'focus-type-path'];

function typePathAndConfirm(path) {
  xdo(['type', '--clearmodifiers', '--delay', String(TYPE_DELAY_MS), path]);
  sleepMs(POST_TYPE_SETTLE_MS);
  xdo(['key', '--clearmodifiers', 'Return']);
}

function openLocationBar() {
  const baselineWindowCount = countWindowIdsInTree(windowTreeSnapshot());
  xdo(['key', '--clearmodifiers', 'ctrl+l']);
  waitForWindowCountAtLeast(baselineWindowCount + 1, LOCATION_BAR_SETTLE_TIMEOUT_MS);
}

function sendChooserInput(windowId, path, strategy) {
  switch (strategy) {
    case 'activate-ctrl-l':
      xdo(['windowactivate', '--sync', windowId]);
      openLocationBar();
      typePathAndConfirm(path);
      return;
    case 'focus-type-path':
      xdo(['windowfocus', windowId]);
      // No `Ctrl+L`: the leading `/` is what opens the location entry.
      typePathAndConfirm(path);
      return;
    default:
      xdo(['windowfocus', windowId]);
      openLocationBar();
      typePathAndConfirm(path);
  }
}

function defaultEmit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

function emitDiagnostics(emit, { windowTitle, windowId, attempts }) {
  const mapState = windowId ? windowMapState(windowId) ?? 'window-gone' : 'no-window-found';
  const tree = windowTreeSnapshot();
  for (const line of formatDiagnostics({ windowTitle, windowId: windowId ?? 'none', attempts, mapState, tree })) {
    emit('dialog_diagnostic', line);
  }
}

/**
 * Drive a native GTK chooser already known to be open under `windowTitle`:
 * `Ctrl+L` opens its location bar, `path` is typed into it, `Return`
 * navigates to (and, for an exact match, selects and closes) it. Verified
 * rather than hoped: each attempt polls for the chooser window to actually
 * disappear before being trusted, retrying the whole keystroke sequence up
 * to `MAX_DIALOG_ATTEMPTS` times and emitting `dialog_attempt=<n>` each try
 * so the transcript shows exactly what was tried.
 *
 * Returns `true` once the chooser is confirmed gone, `false` otherwise --
 * callers keep raising their own disclosed `blocker=` from their existing
 * follow-up check; this function never fabricates success. On `false`,
 * `dialog_diagnostic=...` lines describe the window tree, the target
 * window's last known map state, and the attempt count.
 *
 * `emit(key, value)` defaults to writing `key=value` straight to stdout;
 * `vp-s6-scenario.mjs` passes its own `emit` so these lines interleave with
 * its other observations in one stream.
 */
export function driveChooserWithPath(windowTitle, path, timeoutMs, { emit = defaultEmit } = {}) {
  const windowId = waitForUsableWindow(windowTitle, timeoutMs, emit);
  if (windowId === null) {
    emitDiagnostics(emit, { windowTitle, windowId: null, attempts: 0 });
    return false;
  }

  // `VP001_CHOOSER_STRATEGY` pins every attempt to one strategy. It exists so
  // each strategy can be exercised deliberately on a developer machine --
  // otherwise only the first would ever run locally, and the other two would
  // reach a runner having never executed anywhere. It is never set in CI.
  const forced = process.env.VP001_CHOOSER_STRATEGY;
  for (let attempt = 1; attempt <= MAX_DIALOG_ATTEMPTS; attempt += 1) {
    const strategy = forced ?? (CHOOSER_INPUT_STRATEGIES[attempt - 1] ?? CHOOSER_INPUT_STRATEGIES[0]);
    // The strategy is named in the transcript so a failed run reports which
    // input paths a runner refused, not merely how many times it refused.
    emit('dialog_attempt', `${attempt} strategy=${strategy}`);
    try {
      sendChooserInput(windowId, path, strategy);
    } catch (error) {
      emit('dialog_attempt_error', `${strategy}: ${redactProvenance(String(error?.message ?? error))}`);
    }
    if (waitForWindowGone(windowId, DIALOG_CLOSE_TIMEOUT_MS)) {
      emit('dialog_closed', strategy);
      return true;
    }
  }

  emitDiagnostics(emit, { windowTitle, windowId, attempts: MAX_DIALOG_ATTEMPTS });
  return false;
}
