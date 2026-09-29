// `node --test` coverage for `vp-s1-linux.sh`'s exit contract around the
// reused AV1/AV2 leg. The script runs from a scratch copy of this directory
// in which `vp-s6-linux.sh` and `vp-s1-scenario.mjs` are stubs: no AppImage,
// WebDriver session, or display is involved. Linux-only, like the script
// itself (GNU `timeout`); skipped elsewhere.

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
const LINUX_ONLY = process.platform === 'linux' ? {} : { skip: 'vp-s1-linux.sh needs GNU timeout (Linux only)' };

// Runs vp-s1-linux.sh with `stubBody` standing in for vp-s6-linux.sh's
// AV1/AV2 feasibility check, bounded by a 1 s AV_TIMEOUT. `leg-status.sh` is
// copied alongside the script, exactly as `procedures_dir` expects to find
// it. `scenarioBody`/`scenarioTimeout` default to a stub that only emits
// SCENARIO_MARKER, bounded to a generous 30 s so a slow Node start never
// reads as a wedge; only the scenario-leg tests below override them.
function runWithAvStub(stubBody, { scenarioBody = `process.stdout.write('${SCENARIO_MARKER}\\n');`, scenarioTimeout = '30s' } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s1-linux-'));
  try {
    copyFileSync(join(HERE, 'vp-s1-linux.sh'), join(dir, 'vp-s1-linux.sh'));
    copyFileSync(join(HERE, 'leg-status.sh'), join(dir, 'leg-status.sh'));
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${stubBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s1-scenario.mjs'), `${scenarioBody}\n`);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const result = spawnSync('bash', [join(dir, 'vp-s1-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env: { ...process.env, VP001_AV_TIMEOUT: '1s', VP001_SCENARIO_TIMEOUT: scenarioTimeout },
      timeout: 30_000,
    });
    return { status: result.status, stdout: result.stdout };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('vp-s1-linux.sh AV1/AV2 leg exit contract', LINUX_ONLY, async (t) => {
  await t.test('a passing leg runs the scenario and exits 0', () => {
    const { status, stdout } = runWithAvStub('echo "gate=av-stub-pass"; exit 0');
    assert.equal(status, 0);
    assert.ok(stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a rejection keeps its own gate line and exit 1, and never runs the scenario', () => {
    const { status, stdout } = runWithAvStub('echo "gate=av-stub-rejected"; exit 1');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=av-stub-rejected'));
    assert.ok(!stdout.includes('gate=av-feasibility-check-'));
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a usage error keeps its own exit 2', () => {
    const { status, stdout } = runWithAvStub('exit 2');
    assert.equal(status, 2);
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a leg that outlives AV_TIMEOUT is named a timeout and exits 1', () => {
    const { status, stdout } = runWithAvStub('exec sleep 30');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=av-feasibility-check-timeout'));
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a SIGKILL the bound cannot attribute is named a kill, never a timeout', () => {
    const { status, stdout } = runWithAvStub('kill -KILL $$');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=av-feasibility-check-killed'));
    assert.ok(!stdout.includes('gate=av-feasibility-check-timeout'));
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a SIGTERM from outside the bound is named terminated, never a timeout or a kill', () => {
    const { status, stdout } = runWithAvStub('kill -TERM $$');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=av-feasibility-check-terminated'));
    assert.ok(!stdout.includes('gate=av-feasibility-check-timeout'));
    assert.ok(!stdout.includes('gate=av-feasibility-check-killed'));
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });

  await t.test('a status outside 0/1/2/124/137/143 is named unexpected, with its raw status', () => {
    const { status, stdout } = runWithAvStub('exit 127');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=av-feasibility-check-unexpected-exit status=127'));
    assert.ok(!stdout.includes(SCENARIO_MARKER));
  });
});

test('vp-s1-linux.sh scenario leg exit contract', LINUX_ONLY, async (t) => {
  const AV_PASS = 'echo "gate=av-stub-pass"; exit 0';

  await t.test("the scenario's own blocker keeps its exit 1 and names no wedge", () => {
    const { status, stdout } = runWithAvStub(AV_PASS, { scenarioBody: 'process.exitCode = 1;' });
    assert.equal(status, 1);
    assert.ok(!stdout.includes('gate=scenario-'));
  });

  await t.test('a scenario that outlives SCENARIO_TIMEOUT is named a timeout and exits 1', () => {
    const { status, stdout } = runWithAvStub(AV_PASS, { scenarioBody: 'setTimeout(() => {}, 30_000);', scenarioTimeout: '1s' });
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-timeout'));
  });

  await t.test('a SIGKILLed scenario is named a kill, never a timeout', () => {
    const { status, stdout } = runWithAvStub(AV_PASS, { scenarioBody: "process.kill(process.pid, 'SIGKILL');" });
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-killed'));
    assert.ok(!stdout.includes('gate=scenario-timeout'));
  });
});

test('map_leg_status maps an unmapped status directly, without spawning a driver', LINUX_ONLY, () => {
  const result = spawnSync(
    'bash',
    ['-c', 'source "$1"; map_leg_status scenario 125; echo "rc=$?"', 'bash', join(HERE, 'leg-status.sh')],
    { encoding: 'utf8' },
  );
  assert.ok(result.stdout.includes('gate=scenario-unexpected-exit status=125'), result.stdout);
  assert.ok(result.stdout.includes('rc=1'), result.stdout);
});
