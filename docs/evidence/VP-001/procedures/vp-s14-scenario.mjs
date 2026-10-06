#!/usr/bin/env node
// VP-S14 scenario orchestrator (desktop-stack-verification-plan.md:135;
// ADR 0002:67; TM-001 HAR-3/HAR-4; odd/tasks/vp-s14-executable-identity.md
// T3b-2's own Design notes, which this file follows exactly). Approves an
// executable, mutates the approved file between launches (a bytes-only
// rewrite, then a shadowing symlink), re-approves through the shadow
// target, and emits an observations-only transcript; `derive_executable_identity`
// (tools/evidence-validator) decides the result. The session is
// `runScenarioSession`'s, exactly like VP-S13. Invoked as
// `node vp-s14-scenario.mjs <baseUrl> <applicationPath> <scratchDir>`.
//
// Unlike VP-S13/VP-S6, this scenario drives no DOM: every IPC call goes
// through `vp-s14-probe.mjs`'s in-page script builders directly
// (`window.__TAURI_INTERNALS__.invoke`), never a button click -- there is
// no "pick executable"/"approve" UI step to wait for here, only the
// native chooser `executable_pick_and_probe` itself opens.
//
// `scratchDir` holds three files the shell driver (`vp-s14-linux.sh`)
// prepares before this script runs -- copies of `true`/`false`, named by
// `FIXTURE_NAMES` below (shared with the driver, which reads this object
// through node the same way VP-S13's own driver reads `TRAP_NAMES` from
// `vp-s13-probe.mjs` -- here the names live on this file itself, since
// there is no separate VP-S14 probe-style constants module the driver
// could import instead):
// - `FIXTURE_NAMES.approvedExe`: the one picked and approved, then
//   mutated in place between launches;
// - `FIXTURE_NAMES.replacementExe`: a second file whose bytes are copied
//   over the approved one's for case (b), a bytes-only rewrite;
// - `FIXTURE_NAMES.shadowTargetExe`: the target of the symlink that
//   replaces the approved path for case (c) and the re-approval (d).
//
// Case sequence (vp-s14-probe.mjs's own header has the full IPC detail):
// (a) launch the untouched approved copy; (b) overwrite its bytes in
// place, then launch; (c) replace it with a symlink to a different copy,
// then launch; (d) re-pick the same path -- now resolving through that
// symlink -- re-approve, then launch.

import { join, resolve } from 'node:path';
import { copyFileSync, statSync, symlinkSync, unlinkSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { runScenarioSession, withDeadline } from './scenario-session.mjs';
import { executeScript } from './webdriver-session.mjs';
import { CHOOSER_INPUT_STRATEGIES, driveChooserWithPath } from './vp-s6-xdotool.mjs';
import {
  buildApproveScript, buildListScript, buildPickArmScript, buildPickReadScript, buildSpawnScript, buildStopScript,
} from './vp-s14-probe.mjs';
import {
  formatCaseLines, formatDuplicateLines, formatObservations, formatRecordLines, summarize,
} from './vp-s14-observations.mjs';

/** The three fixture file names the shell driver creates inside
 * `scratchDir` before this script runs -- read by the driver through
 * node, the same way VP-S13's driver reads `TRAP_NAMES` from
 * `vp-s13-probe.mjs`. Frozen so driver and scenario can never drift on
 * them independently. */
export const FIXTURE_NAMES = Object.freeze({
  approvedExe: 'approved-exe',
  replacementExe: 'replacement-exe',
  shadowTargetExe: 'shadow-target-exe',
});

function emit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

// Chooser-close wait and the pick-resolve poll below share this bound, the
// same way VP-S13's own WORKSPACE_PICK_TIMEOUT_MS covers both halves of
// its workspace pick.
const PICK_TIMEOUT_MS = 15_000;
// executable_approve never opens a dialog; this is headroom for a loaded
// CI runner, not a dialog-driving bound.
const APPROVE_TIMEOUT_MS = 10_000;
// approvals_list is a fast no-arg call; same rationale as APPROVE_TIMEOUT_MS.
const LIST_TIMEOUT_MS = 10_000;
// Launching (or being denied) true/false should resolve almost
// immediately; generous headroom for a loaded CI runner.
const SPAWN_SETTLE_TIMEOUT_MS = 10_000;
// A best-effort harness_stop, never branched on below -- this only bounds
// how long the scenario waits before moving on to its next step.
const STOP_TIMEOUT_MS = 10_000;
// Shared read-and-check interval for every settle poll below, matching
// VP-S13's own WORKSPACE_CONFIRM_POLL_MS.
const POLL_INTERVAL_MS = 250;

const settledByPhase = (slot) => Boolean(slot) && (slot.phase === 'resolved' || slot.phase === 'rejected');
const settledByOutcome = (entry) => Boolean(entry) && (entry.outcome === 'resolved' || entry.outcome === 'rejected');

/** Re-reads the one generic `window.__vpS14` snapshot (`buildPickReadScript`)
 * until `extract(snapshot)` is settled per `isSettled`, or `timeoutMs`
 * elapses (`null` then). Never re-arms: every slot this scenario polls was
 * already armed with its own fire-and-forget `.then()`, so re-reading is
 * all that is needed here -- unlike VP-S13's workspace poll, which re-arms
 * `workspace_current` every cycle because that call has no such attached
 * callback of its own. */
async function pollUntilSettled(baseUrl, sessionId, extract, isSettled, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const snapshot = await withDeadline(executeScript(baseUrl, sessionId, buildPickReadScript(), []), `${label}-read`);
    const value = extract(snapshot);
    if (isSettled(value)) return value;
    if (Date.now() >= deadline) return null;
    await delay(POLL_INTERVAL_MS);
  }
}

/** Arms `executable_pick_and_probe`, drives the native "Open File" chooser
 * to `fixturePath`, and polls until the pick settles. Throws
 * `${label}-failed: …` (never resolved, or the shell's own rejection
 * message) -- `runScenarioSession` turns that into the single blocker
 * line. `label` distinguishes the original pick ('pick') from case (d)'s
 * re-pick ('repick') in that blocker text. */
async function pickFixture(baseUrl, sessionId, fixturePath, label) {
  await withDeadline(executeScript(baseUrl, sessionId, buildPickArmScript(), []), `${label}-arm`);
  // Not branched on here, same non-branching rationale as VP-S6/VP-S13's
  // own chooser calls: a chooser that never closes still falls through to
  // the settle poll below, which raises this function's own named failure.
  driveChooserWithPath('Open File', fixturePath, PICK_TIMEOUT_MS, { emit, strategies: CHOOSER_INPUT_STRATEGIES });
  const pick = await pollUntilSettled(baseUrl, sessionId, (snapshot) => snapshot.pick, settledByPhase, PICK_TIMEOUT_MS, label);
  if (!pick) throw new Error(`${label}-failed: never settled`);
  if (pick.phase === 'rejected') throw new Error(`${label}-failed: ${pick.error}`);
  return pick;
}

/** Arms `executable_approve(candidateId)` and polls until it settles.
 * Throws `${label}-failed: …` on a timeout or a rejection. `label`
 * distinguishes the original approval ('approve') from case (d)'s
 * re-approval ('reapprove'). */
async function approveCandidate(baseUrl, sessionId, candidateId, label) {
  await withDeadline(executeScript(baseUrl, sessionId, buildApproveScript(candidateId), []), `${label}-arm`);
  const approve = await pollUntilSettled(baseUrl, sessionId, (snapshot) => snapshot.approve, settledByPhase, APPROVE_TIMEOUT_MS, label);
  if (!approve) throw new Error(`${label}-failed: never settled`);
  if (approve.phase === 'rejected') throw new Error(`${label}-failed: ${approve.error}`);
  return approve;
}

/** Arms `approvals_list()` and polls until it settles, returning the
 * `approvals` array. Throws `${label}-failed: …` on a timeout or a
 * rejection. `label` is 'list-before' or 'list-after'. */
async function listApprovals(baseUrl, sessionId, label) {
  await withDeadline(executeScript(baseUrl, sessionId, buildListScript(), []), `${label}-arm`);
  const list = await pollUntilSettled(baseUrl, sessionId, (snapshot) => snapshot.list, settledByPhase, LIST_TIMEOUT_MS, label);
  if (!list) throw new Error(`${label}-failed: never settled`);
  if (list.phase === 'rejected') throw new Error(`${label}-failed: ${list.error}`);
  return list.approvals;
}

/** Arms `harness_spawn` for `approvalId` under `label` ('a'/'b'/'c'/'d')
 * and polls until that case settles -- resolved or rejected are both
 * expected outcomes here, never a scenario failure of their own; only a
 * timeout throws, as `spawn-${label}-failed: never settled`. */
async function runCase(baseUrl, sessionId, approvalId, label) {
  await withDeadline(executeScript(baseUrl, sessionId, buildSpawnScript(approvalId, label), []), `spawn-${label}-arm`);
  const entry = await pollUntilSettled(
    baseUrl, sessionId, (snapshot) => snapshot.cases && snapshot.cases[label], settledByOutcome, SPAWN_SETTLE_TIMEOUT_MS, `spawn-${label}`,
  );
  if (!entry) throw new Error(`spawn-${label}-failed: never settled`);
  return entry;
}

/** A best-effort `harness_stop`: armed and given a bounded chance to
 * settle, but never throws on a timeout or a rejection -- `summarize`
 * never consumes `window.__vpS14.stop`, so there is nothing here worth
 * failing the scenario over. */
async function stopBestEffort(baseUrl, sessionId, processId) {
  await withDeadline(executeScript(baseUrl, sessionId, buildStopScript(processId), []), 'stop-arm');
  await pollUntilSettled(baseUrl, sessionId, (snapshot) => snapshot.stop, settledByPhase, STOP_TIMEOUT_MS, 'stop');
}

async function main() {
  const [baseUrl, applicationPath, scratchDir] = process.argv.slice(2);
  if (!baseUrl || !applicationPath || !scratchDir) {
    emit('blocker', 'usage: vp-s14-scenario.mjs <baseUrl> <applicationPath> <scratchDir>');
    process.exitCode = 2;
    return;
  }

  const approvedPath = join(scratchDir, FIXTURE_NAMES.approvedExe);
  const replacementPath = join(scratchDir, FIXTURE_NAMES.replacementExe);
  const shadowTargetPath = join(scratchDir, FIXTURE_NAMES.shadowTargetExe);

  process.exitCode = await runScenarioSession({
    baseUrl,
    applicationPath,
    emit,
    body: async (sessionId) => {
      // 1. Pick and 2. approve the original candidate.
      const pick = await pickFixture(baseUrl, sessionId, approvedPath, 'pick');
      const original = await approveCandidate(baseUrl, sessionId, pick.candidateId, 'approve');
      emit('gate', `approved approval_id=${original.approvalId} basename=${FIXTURE_NAMES.approvedExe}`);

      // 3. List before any mutation.
      const beforeList = await listApprovals(baseUrl, sessionId, 'list-before');

      // 4. Case (a): launch the untouched approved copy.
      const caseA = await runCase(baseUrl, sessionId, original.approvalId, 'a');
      if (caseA.outcome === 'resolved') await stopBestEffort(baseUrl, sessionId, caseA.processId);

      // 5. Mutation (b): overwrite the approved file's bytes in place.
      const beforeStatB = statSync(approvedPath);
      copyFileSync(replacementPath, approvedPath);
      const afterStatB = statSync(approvedPath);
      // The whole point of case (b) is a bytes-only rewrite: if the copy
      // ever changed the inode or mode, the scenario would no longer be
      // testing what LaunchGate::decide's ChangedSinceApproval branch
      // claims to guard, so that assumption is checked here, not merely
      // asserted in a comment. Verified empirically (Node 22, Linux):
      // `copyFileSync` does overwrite the destination's existing inode in
      // place, but it sets the destination's mode to the SOURCE file's
      // mode, not to whatever the destination's own mode already was --
      // this check only reads as "mode preserved" because
      // `vp-s14-linux.sh` chmods every fixture to 755 beforehand, so
      // source and destination already agree.
      if (beforeStatB.ino !== afterStatB.ino || beforeStatB.mode !== afterStatB.mode) {
        throw new Error('mutation-b-invariant-broken: copyFileSync changed the inode or mode');
      }
      emit('gate', `mutated case=b before_size=${beforeStatB.size} after_size=${afterStatB.size}`);
      const caseB = await runCase(baseUrl, sessionId, original.approvalId, 'b');

      // 6. Mutation (c): replace the approved path with a symlink.
      unlinkSync(approvedPath);
      symlinkSync(shadowTargetPath, approvedPath);
      emit('gate', 'mutated case=c');
      const caseC = await runCase(baseUrl, sessionId, original.approvalId, 'c');

      // 7. Re-approval: re-pick the same path -- it now canonicalizes to
      // shadow-target-exe -- and approve the resolved candidate.
      const repick = await pickFixture(baseUrl, sessionId, approvedPath, 'repick');
      const reapproval = await approveCandidate(baseUrl, sessionId, repick.candidateId, 'reapprove');
      emit('gate', `reapproved approval_id=${reapproval.approvalId} basename=${FIXTURE_NAMES.approvedExe}`);

      // 8. Case (d): launch under the re-approval.
      const caseD = await runCase(baseUrl, sessionId, reapproval.approvalId, 'd');
      if (caseD.outcome === 'resolved') await stopBestEffort(baseUrl, sessionId, caseD.processId);

      // 9. List after every mutation and the re-approval.
      const afterList = await listApprovals(baseUrl, sessionId, 'list-after');

      // 10. location.protocol, read alongside one final generic snapshot.
      const finalRead = await withDeadline(executeScript(baseUrl, sessionId, buildPickReadScript(), []), 'final-read');

      // 11. Derive and print -- observations only, never a result.
      const cases = { a: caseA, b: caseB, c: caseC, d: caseD };
      const summary = summarize({
        beforeList, afterList, original, reapproval, cases, location_protocol: finalRead.location_protocol,
      });
      const ids = [original.approvalId, reapproval.approvalId];
      for (const line of [
        ...formatCaseLines(cases),
        ...formatRecordLines(beforeList, afterList, ids),
        ...formatDuplicateLines(beforeList, afterList, ids),
        ...formatObservations(summary),
      ]) {
        process.stdout.write(`${line}\n`);
      }
    },
  });
}

// Only run when executed directly, never when imported (resolved as
// paths, like vp-s1/vp-s3/vp-s13-scenario.mjs).
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
