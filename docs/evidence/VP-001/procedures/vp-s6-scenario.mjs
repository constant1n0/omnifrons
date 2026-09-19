#!/usr/bin/env node
// VP-S6 scenario orchestrator (design.md D5/D7, "Honesty machinery"): drives
// the retained packaged artifact's own GUI end to end through `tauri-driver`
// -- approve the fixture agent, pick a scratch workspace, select the
// adapter and approval, Start, observe the fixture and its
// breakaway-attempting descendant, Stop through the UI, a bounded wait,
// then re-enumerate -- and emits a `key=value` transcript of OBSERVATIONS
// ONLY, one per line.
//
// This script never names an outcome (design.md Honesty machinery #1):
// `derive` (`tools/evidence-validator/src/derive.rs`, pure, unit-tested) is
// the sole place a `result`/`observed_state` exists. `vp-s6-linux.sh` runs
// the AV1/AV2 artifact-identity gate before invoking this script; identity
// is therefore always gated by the time this script's own session exists.
//
// Invoked as `node vp-s6-scenario.mjs`, configured entirely through
// environment variables (never spliced into a script body -- the same
// no-path-interpolation discipline `vp-s6-linux.sh`'s AV1/AV2 gate already
// uses):
//   VP001_PROCEDURES_DIR    directory holding webdriver-session.mjs
//   VP001_WEBDRIVER_BASE_URL  tauri-driver's own HTTP endpoint
//   VP001_APPLICATION_PATH  the AV1/AV2-gated artifact to launch
//   VP001_FIXTURE_PATH      the vp-s6-agent binary to approve and launch
//   VP001_WORKSPACE_DIR     an existing, empty scratch directory to pick
//
// The `stream-json-cli` adapter is used unconditionally: its
// `argv_template` is empty (`crates/omnifrons-adapters/src/line_agent.rs`
// (read-only)), so it imposes nothing on the fixture beyond the
// `StdinThenClose` contract the fixture already ignores.

import { readFileSync, existsSync } from 'node:fs';

import {
  clickElement,
  createSession,
  deleteSession,
  executeScript,
  findElement,
  getElementText,
  refreshPage,
  sendKeysToElement,
} from './webdriver-session.mjs';
import { classifyPidAfterWait, parsePidFile, parseProcStatStarttime } from './vp-s6-observations.mjs';
import { driveChooserWithPath, WORKSPACE_CHOOSER_STRATEGIES } from './vp-s6-xdotool.mjs';

// -- Bounded polling (design.md Honesty machinery #3: no retry, no loop
// beyond one bounded wait per observation) --

const POLL_INTERVAL_MS = 250;

async function pollUntil(checkFn, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    // Deliberately sequential: each poll must observe the current DOM state
    // before deciding whether to wait and try again. A rejection (e.g. "no
    // such element" while the page is still rendering, right after Start or
    // a refresh) means "not ready yet", not a hard failure -- it is
    // swallowed the same as a falsy result, and the bound below is what
    // turns a genuinely stuck check into a blocker.
    let result;
    try {
      result = await checkFn();
    } catch {
      result = null;
    }
    if (result) return result;
    if (Date.now() >= deadline) return null;
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
  }
}

/** Poll for an element to appear, bounded by `timeoutMs`; `null` on timeout. */
function waitForElement(baseUrl, sessionId, using, value, timeoutMs) {
  return pollUntil(() => findElement(baseUrl, sessionId, using, value), timeoutMs);
}

/** Read `/proc/<pid>/stat`'s starttime, or `null` if the pid no longer exists. */
function readProcStatStarttimeOrNull(pid) {
  let contents;
  try {
    contents = readFileSync(`/proc/${pid}/stat`, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
  return parseProcStatStarttime(contents);
}

function emit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

const ELEMENT_READY_TIMEOUT_MS = 10_000;
const CHOOSER_TIMEOUT_MS = 15_000;
const START_TIMEOUT_MS = 15_000;
const PID_FILE_TIMEOUT_MS = 15_000;
const STOP_CONFIRM_TIMEOUT_MS = 15_000;
const POST_STOP_WAIT_MS = 5_000;

const AGENT_SECTION = "//section[@aria-label='Agent']";
const STATE_BADGE = `${AGENT_SECTION}//p[starts-with(normalize-space(.), 'State:')]/strong`;

const SELECT_CONTROLS_SCRIPT = `
  const adapterSelect = document.querySelector('#agent-adapter');
  adapterSelect.value = arguments[0];
  adapterSelect.dispatchEvent(new Event('change', { bubbles: true }));
  const approvalSelect = document.querySelector('#agent-approval');
  if (approvalSelect.options.length === 0) { return 'no-approval-option'; }
  approvalSelect.selectedIndex = approvalSelect.options.length - 1;
  approvalSelect.dispatchEvent(new Event('change', { bubbles: true }));
  return approvalSelect.value;
`;

async function readBadge(baseUrl, sessionId) {
  const element = await findElement(baseUrl, sessionId, 'xpath', STATE_BADGE);
  return getElementText(baseUrl, sessionId, element);
}

async function approveFixture(baseUrl, sessionId, fixturePath) {
  const pickButton = await waitForElement(
    baseUrl,
    sessionId,
    'xpath',
    "//section[@aria-label='Executable approval']//button[text()='Pick executable']",
    ELEMENT_READY_TIMEOUT_MS,
  );
  if (pickButton === null) {
    return { ok: false, blocker: 'pick-executable-button-not-found' };
  }
  await clickElement(baseUrl, sessionId, pickButton);
  // The return value is intentionally not branched on here: a `false` (the
  // chooser never closed) still falls through to the candidate-panel wait
  // below, which times out on its own and raises this function's existing
  // `executable-chooser-timeout` blocker -- driveChooserWithPath only adds
  // robustness and `dialog_attempt`/`dialog_diagnostic` transcript lines,
  // it never invents a new outcome.
  driveChooserWithPath('Open File', fixturePath, CHOOSER_TIMEOUT_MS, { emit });

  const candidatePanel = await waitForElement(
    baseUrl,
    sessionId,
    'xpath',
    "//div[@aria-label='Candidate evidence']",
    CHOOSER_TIMEOUT_MS,
  );
  if (candidatePanel === null) {
    return { ok: false, blocker: 'executable-chooser-timeout' };
  }

  const digestElement = await findElement(
    baseUrl,
    sessionId,
    'xpath',
    "//div[@aria-label='Candidate evidence']//p[contains(., 'SHA-256 (short)')]/span",
  );
  const sha256Short = await getElementText(baseUrl, sessionId, digestElement);

  const confirmInput = await findElement(baseUrl, sessionId, 'css selector', '#approval-confirm-input');
  await sendKeysToElement(baseUrl, sessionId, confirmInput, sha256Short);

  const approveButton = await findElement(
    baseUrl,
    sessionId,
    'xpath',
    "//div[@aria-label='Candidate evidence']//button[text()='Approve']",
  );
  await clickElement(baseUrl, sessionId, approveButton);

  const approved = await waitForElement(
    baseUrl,
    sessionId,
    'xpath',
    "//ul[@aria-label='Approvals list']/li",
    CHOOSER_TIMEOUT_MS,
  );
  return approved === null ? { ok: false, blocker: 'approval-confirmation-timeout' } : { ok: true };
}

async function pickWorkspace(baseUrl, sessionId, workspaceDir) {
  const pickButton = await waitForElement(
    baseUrl,
    sessionId,
    'xpath',
    `${AGENT_SECTION}//button[text()='Pick workspace']`,
    ELEMENT_READY_TIMEOUT_MS,
  );
  if (pickButton === null) {
    return { ok: false, blocker: 'pick-workspace-button-not-found' };
  }
  await clickElement(baseUrl, sessionId, pickButton);
  // Same non-branching rationale as approveFixture's own call above.
  // `WORKSPACE_CHOOSER_STRATEGIES` (vp-s6-xdotool.mjs) tries `bookmark-jump`
  // first: it needs no location entry and no typing, unlike this file's own
  // "Open File" chooser below, which must land on one specific file.
  driveChooserWithPath('Select Folder', workspaceDir, CHOOSER_TIMEOUT_MS, {
    emit,
    strategies: WORKSPACE_CHOOSER_STRATEGIES,
  });

  const confirmed = await waitForElement(
    baseUrl,
    sessionId,
    'xpath',
    `${AGENT_SECTION}//p[starts-with(normalize-space(.), 'Workspace:')]`,
    CHOOSER_TIMEOUT_MS,
  );
  return confirmed === null ? { ok: false, blocker: 'workspace-chooser-timeout' } : { ok: true };
}

async function main() {
  const baseUrl = process.env.VP001_WEBDRIVER_BASE_URL;
  const applicationPath = process.env.VP001_APPLICATION_PATH;
  const fixturePath = process.env.VP001_FIXTURE_PATH;
  const workspaceDir = process.env.VP001_WORKSPACE_DIR;

  const sessionId = await createSession(baseUrl, applicationPath);
  emit('gate', `f1-session-created session_id=${sessionId}`);

  // Observations, defaulted honestly to "not proven" -- only ever flipped
  // true by an actual, bounded, positive check below (design.md Honesty
  // machinery #2: any absent or unreadable observation yields `uncertain`).
  const observations = {
    identity_gated: true, // AV1/AV2 already ran in vp-s6-linux.sh before this script was invoked.
    descendant_alive_before_stop: false,
    stop_confirmed: false,
    all_pids_proven_gone: false,
    any_pid_alive_after_wait: false,
    enumeration_unreadable: false,
  };
  let blocker = null;

  try {
    const workspaceResult = await pickWorkspace(baseUrl, sessionId, workspaceDir);
    if (!workspaceResult.ok) {
      blocker = workspaceResult.blocker;
      return;
    }

    const approvalResult = await approveFixture(baseUrl, sessionId, fixturePath);
    if (!approvalResult.ok) {
      blocker = approvalResult.blocker;
      return;
    }

    // AgentPanel's own approvals list fetches once on mount, with no
    // refresh on a new approval (this change's own Slice 3a finding) --
    // reload so the freshly approved executable appears in #agent-approval.
    await refreshPage(baseUrl, sessionId);

    // The reloaded page needs a moment to remount before its controls
    // exist again -- wait for the adapter select rather than assuming the
    // refresh already settled.
    const adapterSelectReady = await waitForElement(
      baseUrl,
      sessionId,
      'css selector',
      '#agent-adapter',
      ELEMENT_READY_TIMEOUT_MS,
    );
    if (adapterSelectReady === null) {
      blocker = 'adapter-select-not-found-after-refresh';
      return;
    }

    const selectedApproval = await executeScript(baseUrl, sessionId, SELECT_CONTROLS_SCRIPT, [
      'stream-json-cli',
    ]);
    if (selectedApproval === 'no-approval-option') {
      blocker = 'no-approval-option-after-refresh';
      return;
    }
    emit('gate', `controls-selected approval_id=${selectedApproval}`);

    const startButton = await waitForElement(
      baseUrl,
      sessionId,
      'xpath',
      `${AGENT_SECTION}//button[text()='Start']`,
      ELEMENT_READY_TIMEOUT_MS,
    );
    if (startButton === null) {
      blocker = 'start-button-not-found';
      return;
    }
    await clickElement(baseUrl, sessionId, startButton);

    const runningBadge = await pollUntil(async () => {
      const badge = await readBadge(baseUrl, sessionId);
      return badge === 'running' ? badge : null;
    }, START_TIMEOUT_MS);
    if (runningBadge === null) {
      blocker = 'start-timeout';
      return;
    }
    emit('observation', 'started value=true');

    const pidFilePath = `${workspaceDir}/vp-s6-agent.pids`;
    const pidFileAppeared = await pollUntil(() => (existsSync(pidFilePath) ? true : null), PID_FILE_TIMEOUT_MS);
    if (pidFileAppeared === null) {
      blocker = 'pid-file-timeout';
      return;
    }

    const { selfPid, descendantPid } = parsePidFile(readFileSync(pidFilePath, 'utf8'));
    emit('observation', `self_pid=${selfPid} descendant_pid=${descendantPid}`);

    const selfBaselineStarttime = readProcStatStarttimeOrNull(selfPid);
    const descendantBaselineStarttime = readProcStatStarttimeOrNull(descendantPid);
    observations.descendant_alive_before_stop = descendantBaselineStarttime !== null;

    const stopButton = await waitForElement(
      baseUrl,
      sessionId,
      'xpath',
      `${AGENT_SECTION}//button[text()='Stop']`,
      ELEMENT_READY_TIMEOUT_MS,
    );
    if (stopButton === null) {
      blocker = 'stop-button-not-found';
      return;
    }
    await clickElement(baseUrl, sessionId, stopButton);

    const terminalBadge = await pollUntil(async () => {
      const badge = await readBadge(baseUrl, sessionId);
      return badge !== 'running' && badge !== 'idle' ? badge : null;
    }, STOP_CONFIRM_TIMEOUT_MS);
    observations.stop_confirmed = terminalBadge !== null;
    emit('gate', `stop-badge value=${terminalBadge ?? 'unconfirmed'}`);

    // Bounded wait (design.md's sequence: "Stop -> bounded wait ->
    // re-enumerate"), giving the SIGTERM-then-SIGKILL escalation
    // (`crates/omnifrons-supervisor/src/lib.rs` (read-only)) time to run
    // its course before this script looks again.
    await new Promise((resolve) => setTimeout(resolve, POST_STOP_WAIT_MS));

    let unreadable = false;
    let selfClass;
    let descendantClass;
    try {
      selfClass = classifyPidAfterWait(selfBaselineStarttime, readProcStatStarttimeOrNull(selfPid));
    } catch {
      unreadable = true;
      selfClass = 'unreadable';
    }
    try {
      descendantClass = classifyPidAfterWait(
        descendantBaselineStarttime,
        readProcStatStarttimeOrNull(descendantPid),
      );
    } catch {
      unreadable = true;
      descendantClass = 'unreadable';
    }
    emit('observation', `self_after_wait=${selfClass} descendant_after_wait=${descendantClass}`);

    observations.enumeration_unreadable = unreadable;
    observations.any_pid_alive_after_wait = selfClass === 'alive' || descendantClass === 'alive';
    observations.all_pids_proven_gone =
      !unreadable &&
      ['gone', 'recycled'].includes(selfClass) &&
      ['gone', 'recycled'].includes(descendantClass);
  } catch (error) {
    blocker = `unexpected-error: ${error.message}`;
    if (process.env.VP001_DEBUG) console.error(error);
  } finally {
    for (const [key, value] of Object.entries(observations)) {
      emit('observation', `${key} value=${value}`);
    }
    if (blocker !== null) {
      emit('blocker', blocker);
    }
    await deleteSession(baseUrl, sessionId).catch(() => {});
    emit('gate', `session-closed session_id=${sessionId}`);
  }
}

// Only run when executed directly (`node vp-s6-scenario.mjs`), never when
// imported -- `vp-s6-scenario.test.mjs` imports this module for its pure
// helpers alone and must never trigger a live WebDriver session.
if (import.meta.url === `file://${process.argv[1]}`) {
  await main();
}
