#!/usr/bin/env node
// VP-S1 scenario orchestrator (desktop-stack-verification-plan.md:122):
// arms the three CSP attempts (`vp-s1-probe.mjs`), waits, reads the
// result, and emits a `key=value` transcript of OBSERVATIONS ONLY. Never
// names an outcome (mirrors `vp-s6-scenario.mjs`): `derive_csp`
// (`tools/evidence-validator`, pure, unit-tested) is the sole place a
// result exists. Unlike `vp-s6-scenario.mjs`, this exits non-zero on any
// failure -- a deliberate, differently-shaped signal from VP-S6's own
// exit-0 discipline.
//
// The WebDriver session itself (create, this scenario's own work below,
// and delete -- including on the SIGTERM `vp-s1-linux.sh`'s
// `timeout --kill-after=10s` sends when the scenario bound fires) is owned
// by `scenario-session.mjs`'s `runScenarioSession`, shared with
// `vp-s3-scenario.mjs`; this file supplies only the `body` callback below.
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

import { runScenarioSession, withDeadline } from './scenario-session.mjs';
import { executeScript } from './webdriver-session.mjs';
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

  process.exitCode = await runScenarioSession({
    baseUrl,
    applicationPath,
    emit,
    body: async (sessionId) => {
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
    },
  });
}

// Only run when executed directly, never when imported. Both sides are
// resolved as paths: a bare string comparison against a hand-built
// `file://` prefix silently ran nothing on any encoding difference.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
