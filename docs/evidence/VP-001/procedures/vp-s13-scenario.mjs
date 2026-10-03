#!/usr/bin/env node
// VP-S13 scenario orchestrator (desktop-stack-verification-plan.md:134):
// selects the trap workspace through the native picker (vp-s6-xdotool.mjs's
// chooser driving, as VP-S6 does), arms vp-s13-probe.mjs's attack corpus,
// and emits an observations-only transcript; `derive_ipc_boundary` decides
// the result. The session is `runScenarioSession`'s. The canary arrives via
// VP001_CANARY, never argv or print. Invoked as
// `node vp-s13-scenario.mjs <baseUrl> <applicationPath> <workspaceDir>`.

import { basename, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { runScenarioSession, withDeadline } from './scenario-session.mjs';
import { executeScript } from './webdriver-session.mjs';
import { driveChooserWithPath, WORKSPACE_CHOOSER_STRATEGIES } from './vp-s6-xdotool.mjs';
import { ATTACKS, buildArmScript, buildReadScript, formatAttackLines, formatObservations, summarize } from './vp-s13-probe.mjs';

function emit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

// Fire-and-forget: `workspace_pick` blocks on the dialog; `workspace_current` confirms below.
const WORKSPACE_PICK_ARM_SCRIPT = `
  if (window.__TAURI_INTERNALS__) {
    window.__TAURI_INTERNALS__.invoke('workspace_pick').catch(function () {});
  }
`;

// Re-armed every poll: the workspace may not be registered the instant this fires.
const WORKSPACE_CURRENT_ARM_SCRIPT = `
  window.__vpS13WorkspaceCurrent = { phase: 'pending' };
  window.__TAURI_INTERNALS__.invoke('workspace_current').then(
    function (value) { window.__vpS13WorkspaceCurrent = { phase: 'resolved', displayPath: value && value.displayPath }; },
    function (error) { window.__vpS13WorkspaceCurrent = { phase: 'rejected', message: String((error && error.message) || error) }; },
  );
`;

const WORKSPACE_CURRENT_READ_SCRIPT = `return window.__vpS13WorkspaceCurrent || { phase: 'unarmed' };`;

const WORKSPACE_PICK_TIMEOUT_MS = 15_000; // chooser-close wait and workspace_current confirm poll share this bound
const WORKSPACE_CONFIRM_POLL_MS = 250;

// 2.5x VP-S3's 2 s bound: this corpus also round-trips a ~1 MiB string. An
// attempt still unsettled when the read runs is 'pending', not a blocker.
const SETTLE_MS = 5_000;

/** Bounded, re-arming poll for `workspace_current` to resolve to
 * `expectedBasename`. `{ ok, basename }` or `{ ok: false, reason }` --
 * never throws, never a raw path. */
async function confirmWorkspaceSelected(baseUrl, sessionId, expectedBasename, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastPhase = 'unarmed';
  let lastBasename = null;
  for (;;) {
    await withDeadline(executeScript(baseUrl, sessionId, WORKSPACE_CURRENT_ARM_SCRIPT, []), 'workspace-current-arm');
    await delay(WORKSPACE_CONFIRM_POLL_MS);
    const read = await withDeadline(executeScript(baseUrl, sessionId, WORKSPACE_CURRENT_READ_SCRIPT, []), 'workspace-current-read');
    lastPhase = (read && read.phase) || 'unknown';
    lastBasename = read && typeof read.displayPath === 'string' ? basename(read.displayPath) : null;
    if (lastPhase === 'resolved' && lastBasename === expectedBasename) {
      return { ok: true, basename: lastBasename };
    }
    if (Date.now() >= deadline) {
      const seen = lastBasename ? ` basename=${lastBasename}` : '';
      return { ok: false, reason: `workspace_current never confirmed basename=${expectedBasename} (phase=${lastPhase}${seen})` };
    }
  }
}

async function main() {
  const [baseUrl, applicationPath, workspaceDir] = process.argv.slice(2);
  if (!baseUrl || !applicationPath || !workspaceDir) {
    emit('blocker', 'usage: vp-s13-scenario.mjs <baseUrl> <applicationPath> <workspaceDir>');
    process.exitCode = 2;
    return;
  }

  const expectedBasename = basename(workspaceDir);
  const canary = process.env.VP001_CANARY ?? '';

  process.exitCode = await runScenarioSession({
    baseUrl,
    applicationPath,
    emit,
    body: async (sessionId) => {
      await withDeadline(executeScript(baseUrl, sessionId, WORKSPACE_PICK_ARM_SCRIPT, []), 'workspace-pick-arm');

      // Not branched on: a chooser that never closes falls through below.
      driveChooserWithPath('Select Folder', workspaceDir, WORKSPACE_PICK_TIMEOUT_MS, {
        emit,
        strategies: WORKSPACE_CHOOSER_STRATEGIES,
      });

      const confirmed = await confirmWorkspaceSelected(baseUrl, sessionId, expectedBasename, WORKSPACE_PICK_TIMEOUT_MS);
      // Thrown, not emitted here: runScenarioSession reports it as the one
      // blocker line and still deletes the session.
      if (!confirmed.ok) throw new Error(`workspace-select-failed: ${confirmed.reason}`);
      emit('gate', `workspace-selected basename=${confirmed.basename}`);

      await withDeadline(executeScript(baseUrl, sessionId, buildArmScript(ATTACKS, { canary }), []), 'arm');
      emit('gate', 'armed');

      await delay(SETTLE_MS);

      const read = await withDeadline(executeScript(baseUrl, sessionId, buildReadScript(), []), 'read');
      const summary = summarize(read, { expectedWorkspaceBasename: expectedBasename });

      emit('gate', `location-scheme value=${read.location_protocol ?? 'unknown'}`);
      for (const line of [...formatAttackLines(read.attacks), ...formatObservations(summary)]) {
        process.stdout.write(`${line}\n`);
      }
    },
  });
}

// Only run when executed directly, never when imported (resolved as
// paths, like vp-s1/vp-s3-scenario.mjs).
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
