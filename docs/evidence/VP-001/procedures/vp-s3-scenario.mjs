#!/usr/bin/env node
// VP-S3 scenario orchestrator (desktop-stack-verification-plan.md:124):
// arms the typed-IPC bridge probe's four attempts (`vp-s3-probe.mjs`),
// waits, reads the result, and emits a `key=value` transcript of
// OBSERVATIONS ONLY. Never names an outcome (mirrors `vp-s1-scenario.mjs`):
// `derive_ipc` (`tools/evidence-validator`, pure, unit-tested) is the sole
// place a result exists. Exits non-zero on any failure, like
// `vp-s1-scenario.mjs`.
//
// Unlike VP-S1, a third `<os>` argument selects the probe's per-OS bridge
// origin and documented source; it is validated before any session opens.
// The WebDriver session itself (create, this scenario's own work below,
// and delete -- including on the SIGTERM `vp-s3-linux.sh`'s
// `timeout --kill-after=10s` sends when the scenario bound fires) is owned
// by `scenario-session.mjs`'s `runScenarioSession`, shared with
// `vp-s1-scenario.mjs`; this file supplies only the `body` callback below.
// Between the location-scheme gate and the summary, every violation, then
// every recorded fetch, then every recorded attempt is emitted verbatim
// (formats: `vp-s3-probe.mjs`).
//
// Invoked as `node vp-s3-scenario.mjs <baseUrl> <applicationPath> <os>`.

import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { runScenarioSession, withDeadline } from './scenario-session.mjs';
import { executeScript } from './webdriver-session.mjs';
import {
  buildArmScript,
  buildReadScript,
  formatAttemptLines,
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

  process.exitCode = await runScenarioSession({
    baseUrl,
    applicationPath,
    emit,
    body: async (sessionId) => {
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
      for (const line of formatAttemptLines(read.attempts)) {
        process.stdout.write(`${line}\n`);
      }
      for (const line of formatObservations(summary)) {
        process.stdout.write(`${line}\n`);
      }
    },
  });
}

// Only run when executed directly, never when imported. Both sides are
// resolved as paths: a bare string comparison against a hand-built
// `file://` prefix silently ran nothing on any encoding difference.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
