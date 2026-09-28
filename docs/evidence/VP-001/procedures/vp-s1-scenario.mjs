#!/usr/bin/env node
// VP-S1 scenario orchestrator (desktop-stack-verification-plan.md:122):
// opens one WebDriver session, arms the three CSP attempts
// (`vp-s1-probe.mjs`), waits, reads the result, and emits a `key=value`
// transcript of OBSERVATIONS ONLY. Never names an outcome (mirrors
// `vp-s6-scenario.mjs`): `derive_csp` (`tools/evidence-validator`, pure,
// unit-tested) is the sole place a result exists. Unlike `vp-s6-scenario.mjs`,
// this exits non-zero on any failure -- a deliberate, differently-shaped
// signal from VP-S6's own exit-0 discipline.
//
// Between the location-scheme gate and the summary observations, this also
// emits one `observation=violation ...` line per captured
// `securitypolicyviolation` event, verbatim and in event order
// (`vp-s1-probe.mjs`'s `formatViolationLines` -- see that file's header for
// the line format and its encoding).
//
// Invoked as `node vp-s1-scenario.mjs <baseUrl> <applicationPath>`.

import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { SESSION_CREATE_DEADLINE_MS, withDeadline } from './scenario-session.mjs';
import { createSession, deleteSession, executeScript } from './webdriver-session.mjs';
import {
  buildArmScript,
  buildReadScript,
  formatObservations,
  formatViolationLines,
  summarize,
} from './vp-s1-probe.mjs';

function emit(key, value) {
  process.stdout.write(`${key}=${value}\n`);
}

// Violation events and the fetch settle asynchronously -- this bounded
// wait, not a poll, gives them time before buildReadScript runs. An event
// not yet fired reads back as its own honest absence, not a blocker.
const SETTLE_MS = 2_000;

async function main() {
  const [baseUrl, applicationPath] = process.argv.slice(2);
  if (!baseUrl || !applicationPath) {
    emit('blocker', 'usage: vp-s1-scenario.mjs <baseUrl> <applicationPath>');
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
    const summary = summarize(read);

    emit('gate', `location-scheme value=${summary.location_scheme}`);
    for (const line of formatViolationLines(read.violations)) {
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
