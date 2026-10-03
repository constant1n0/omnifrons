// `node --test` coverage for `vp-s13-linux.sh` (mirrors vp-s3-linux.test.mjs):
// exit contract, trap workspace and canary, and the outside snapshot. Stubs
// stand in for `vp-s6-linux.sh` and `vp-s13-scenario.mjs`; the real
// `vp-s13-probe.mjs` supplies TRAP_NAMES. Linux-only (GNU timeout/stat).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const DIGEST = 'a'.repeat(64);
const SCENARIO_MARKER = 'stub=scenario-ran';
const LINUX_ONLY =
  process.platform === 'linux' ? {} : { skip: 'vp-s13-linux.sh needs GNU timeout/stat/mapfile (Linux only)' };
const AV_PASS = 'echo "gate=av-stub-pass"; exit 0';

// Follows the guidance-file symlink and compares its target to the env
// canary, never echoing it (concatenation, as in vp-s3-linux.test.mjs).
const DEFAULT_SCENARIO_BODY = `
import { readFileSync, readlinkSync } from 'node:fs';
import { TRAP_NAMES } from './vp-s13-probe.mjs';
const workspaceDir = process.argv[4];
const canary = process.env.VP001_CANARY ?? '';
const target = readlinkSync(workspaceDir + '/' + TRAP_NAMES.guidanceFile);
const content = readFileSync(target, 'utf8');
process.stdout.write('${SCENARIO_MARKER} canary_matches=' + (content === canary) + '\\n');
`;

// A stand-in escape: writes into the outside dir, workspaceDir's sibling.
const ESCAPE_SCENARIO_BODY = `
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
const workspaceDir = process.argv[4];
writeFileSync(join(dirname(workspaceDir), 'outside', 'escaped.txt'), 'escaped');
process.stdout.write('${SCENARIO_MARKER} wrote-outside=true\\n');
`;

// Runs vp-s13-linux.sh with `stubBody` for vp-s6-linux.sh's AV1/AV2 check
// (bounded 1 s) and `scenarioBody` for vp-s13-scenario.mjs.
function runWithAvStub(stubBody, scenarioBody = DEFAULT_SCENARIO_BODY, scenarioTimeout = '30s', probeBody = null) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s13-linux-'));
  try {
    copyFileSync(join(HERE, 'vp-s13-linux.sh'), join(dir, 'vp-s13-linux.sh'));
    copyFileSync(join(HERE, 'leg-status.sh'), join(dir, 'leg-status.sh'));
    if (probeBody === null) copyFileSync(join(HERE, 'vp-s13-probe.mjs'), join(dir, 'vp-s13-probe.mjs'));
    else writeFileSync(join(dir, 'vp-s13-probe.mjs'), `${probeBody}\n`);
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${stubBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s13-scenario.mjs'), `${scenarioBody}\n`);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const result = spawnSync('bash', [join(dir, 'vp-s13-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env: { ...process.env, VP001_AV_TIMEOUT: '1s', VP001_SCENARIO_TIMEOUT: scenarioTimeout },
      timeout: 30_000,
    });
    return { status: result.status, stdout: result.stdout };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('vp-s13-linux.sh', LINUX_ONLY, async (t) => {
  // One run per fixture, inside the Linux gate, shared across assertions below.
  const passingRun = runWithAvStub(AV_PASS);
  const escapingRun = runWithAvStub(AV_PASS, ESCAPE_SCENARIO_BODY);
  const rejectedRun = runWithAvStub('echo "gate=av-stub-rejected"; exit 1');
  const usageErrorRun = runWithAvStub('exit 2');
  const scenarioBlockerRun = runWithAvStub(AV_PASS, 'process.exitCode = 1;');
  const scenarioTimeoutRun = runWithAvStub(AV_PASS, 'setTimeout(() => {}, 30_000);', '1s');
  const badNamesRun = runWithAvStub(AV_PASS, undefined, '30s', "export const TRAP_NAMES = { guidanceFile: '../AGENTS.md', absentGuidanceFile: 'x.md' };");
  const brokenProbeRun = runWithAvStub(AV_PASS, undefined, '30s', "throw new Error('probe import failed');");

  await t.test('TRAP_NAMES that are missing or not one plain name stop the run with a named gate, before any trap or scenario', () => {
    for (const run of [badNamesRun, brokenProbeRun]) {
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=trap-names-invalid'), run.stdout);
      assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
    }
  });

  await t.test('AV1/AV2 pass: prepares the trap workspace, runs the scenario, exits 0, and emits a length-only canary gate', () => {
    assert.equal(passingRun.status, 0);
    assert.ok(passingRun.stdout.includes(SCENARIO_MARKER), passingRun.stdout);
    const line = passingRun.stdout.split('\n').find((candidate) => candidate.startsWith('gate=canary-created'));
    assert.ok(line, passingRun.stdout);
    assert.match(line, /^gate=canary-created length=\d+$/);
    assert.ok(passingRun.stdout.includes('canary_matches=true'), passingRun.stdout);
  });

  await t.test('AV1/AV2 failure paths: a rejection keeps its gate/exit 1 and skips canary/scenario; a usage error keeps exit 2', () => {
    assert.equal(rejectedRun.status, 1);
    assert.ok(rejectedRun.stdout.includes('gate=av-stub-rejected'));
    assert.ok(!rejectedRun.stdout.includes('gate=canary-created'));
    assert.ok(!rejectedRun.stdout.includes(SCENARIO_MARKER));
    assert.equal(usageErrorRun.status, 2);
    assert.ok(!usageErrorRun.stdout.includes(SCENARIO_MARKER));
  });

  await t.test('outside snapshot: unmodified on a no-op scenario, modified once one writes outside; no raw /tmp path either way', () => {
    assert.ok(passingRun.stdout.includes('gate=outside-snapshot-taken'), passingRun.stdout);
    assert.ok(passingRun.stdout.includes('observation=outside_target_modified value=false'), passingRun.stdout);
    assert.ok(!passingRun.stdout.includes('/tmp/'), passingRun.stdout);
    assert.equal(escapingRun.status, 0);
    assert.ok(escapingRun.stdout.includes('wrote-outside=true'), escapingRun.stdout);
    assert.ok(escapingRun.stdout.includes('observation=outside_target_modified value=true'), escapingRun.stdout);
    assert.ok(!escapingRun.stdout.includes('/tmp/'), escapingRun.stdout);
  });

  await t.test("the scenario's own blocker keeps exit 1 (outside snapshot still ran); outliving SCENARIO_TIMEOUT is a timeout", () => {
    assert.equal(scenarioBlockerRun.status, 1);
    assert.ok(!scenarioBlockerRun.stdout.includes('gate=scenario-'));
    assert.ok(scenarioBlockerRun.stdout.includes('gate=outside-snapshot-taken'));
    assert.equal(scenarioTimeoutRun.status, 1);
    assert.ok(scenarioTimeoutRun.stdout.includes('gate=scenario-timeout'));
  });
});
