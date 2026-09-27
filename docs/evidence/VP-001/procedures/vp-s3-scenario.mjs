#!/usr/bin/env node
// VP-S3 scenario orchestrator (desktop-stack-verification-plan.md:124):
// opens one WebDriver session, arms the typed-IPC bridge probe's four
// attempts (`vp-s3-probe.mjs`), waits, reads the result, and emits a
// `key=value` transcript of OBSERVATIONS ONLY. Never names an outcome
// (mirrors `vp-s1-scenario.mjs`): `derive_ipc` (`tools/evidence-validator`,
// pure, unit-tested) is the sole place a result exists. Exits non-zero on
// any failure, like `vp-s1-scenario.mjs`.
//
// Unlike VP-S1, a third `<os>` argument selects the probe's per-OS bridge
// origin and documented source; it is validated before any session opens.
// Between the location-scheme gate and the summary, every violation and then
// every recorded fetch is emitted verbatim (formats: `vp-s3-probe.mjs`).
//
// Invoked as `node vp-s3-scenario.mjs <baseUrl> <applicationPath> <os>`.

import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { SESSION_CREATE_DEADLINE_MS, withDeadline } from './vp-s1-scenario.mjs';
import { createSession, deleteSession, executeScript } from './webdriver-session.mjs';
import {
  buildArmScript,
  buildReadScript,
  formatFetchLines,
  formatObservations,
  formatViolationLines,
  summarize,
} from './vp-s3-probe.mjs';

function emit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

// The probe's supported OS tokens (`bridgeOriginFor`'s domain), checked here
// before a WebDriver session opens rather than by the probe's own throw after.
const SUPPORTED_OS_VALUES = ['linux', 'macos', 'windows'];

/** `true` iff `os` is one of vp-s3-probe.mjs's supported OS tokens. Pure; never throws. */
export function isSupportedOs(os) {
  return SUPPORTED_OS_VALUES.includes(os);
}

// Both IPC invokes and the artifact: image load settle asynchronously, like
// VP-S1's violation events. An attempt still unsettled when the read runs is
// recorded as 'pending', never as a blocker. VP-S1's 2 s bound is kept until
// evidence shows it is too short.
const SETTLE_MS = 2_000;

async function main() {
  const [baseUrl, applicationPath, os] = process.argv.slice(2);
  if (!baseUrl || !applicationPath || !os || !isSupportedOs(os)) {
    emit(
      'blocker',
      `usage: vp-s3-scenario.mjs <baseUrl> <applicationPath> <os> (os: ${SUPPORTED_OS_VALUES.join('|')})`,
    );
    process.exitCode = 2;
    return;
  }

  let sessionId;
  try {
    sessionId = await withDeadline(createSession(baseUrl, applicationPath), 'session-create', SESSION_CREATE_DEADLINE_MS);
  } catch (error) {
    emit('blocker', `session-create-failed: ${error.message}`);
    process.exitCode = 1;
    return;
  }
  emit('gate', `session-created session_id=${sessionId}`);

  let blocker = null;
  try {
    await withDeadline(executeScript(baseUrl, sessionId, buildArmScript(), []), 'arm');
    emit('gate', 'armed');

    await delay(SETTLE_MS);

    const read = await withDeadline(executeScript(baseUrl, sessionId, buildReadScript(), []), 'read');
    const summary = summarize(read, os);

    emit('gate', `location-scheme value=${summary.location_scheme}`);
    for (const line of formatViolationLines(read.violations)) {
      process.stdout.write(`${line}\n`);
    }
    for (const line of formatFetchLines(read.fetches)) {
      process.stdout.write(`${line}\n`);
    }
    for (const line of formatObservations(summary)) {
      process.stdout.write(`${line}\n`);
    }
  } catch (error) {
    blocker = `unexpected-error: ${error.message}`;
    if (process.env.VP001_DEBUG) console.error(error);
  } finally {
    if (blocker !== null) {
      emit('blocker', blocker);
    }
    // A failed teardown is the one failure that leaves the packaged app
    // alive, so it is reported as its own blocker, never as a close.
    try {
      await withDeadline(deleteSession(baseUrl, sessionId), 'session-delete');
      emit('gate', `session-closed session_id=${sessionId}`);
    } catch (error) {
      blocker ??= `session-close-failed: ${error.message}`;
      emit('blocker', `session-close-failed: ${error.message}`);
      if (process.env.VP001_DEBUG) console.error(error);
    }
  }

  if (blocker !== null) {
    process.exitCode = 1;
  }
}

// Only run when executed directly, never when imported. Both sides are
// resolved as paths: a bare string comparison against a hand-built
// `file://` prefix silently ran nothing on any encoding difference.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
