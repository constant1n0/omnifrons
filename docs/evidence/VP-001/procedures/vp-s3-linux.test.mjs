// `node --test` coverage for `vp-s3-linux.sh`'s exit contract around the
// reused AV1/AV2 leg (mirrors `vp-s1-linux.test.mjs`). The script runs from
// a scratch copy of this directory in which `vp-s6-linux.sh` and
// `vp-s3-scenario.mjs` are stubs: no AppImage, WebDriver session, or display
// is involved. Linux-only, like the script itself (GNU `timeout`); skipped
// elsewhere.

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
const LINUX_ONLY = process.platform === 'linux' ? {} : { skip: 'vp-s3-linux.sh needs GNU timeout (Linux only)' };

const ECHO_ARGV = `process.stdout.write('${SCENARIO_MARKER} argv=' + process.argv.slice(2).join(',') + '\\n');`;

// Runs vp-s3-linux.sh with `stubBody` standing in for vp-s6-linux.sh's
// AV1/AV2 feasibility check (bounded to 1 s) and `scenarioBody` for
// vp-s3-scenario.mjs, bounded by `scenarioTimeout`: generous by default, so
// a slow Node start never reads as a wedge; only the timeout case shortens it.
// The default scenario stub echoes its argv, to check `linux` as its third.
function runWithAvStub(stubBody, scenarioBody = ECHO_ARGV, scenarioTimeout = '30s') {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s3-linux-'));
  try {
    copyFileSync(join(HERE, 'vp-s3-linux.sh'), join(dir, 'vp-s3-linux.sh'));
    copyFileSync(join(HERE, 'leg-status.sh'), join(dir, 'leg-status.sh'));
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${stubBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s3-scenario.mjs'), `${scenarioBody}\n`);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const result = spawnSync('bash', [join(dir, 'vp-s3-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env: { ...process.env, VP001_AV_TIMEOUT: '1s', VP001_SCENARIO_TIMEOUT: scenarioTimeout },
      timeout: 30_000,
    });
    return { status: result.status, stdout: result.stdout };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('vp-s3-linux.sh AV1/AV2 leg exit contract', LINUX_ONLY, async (t) => {
  await t.test('a passing leg runs the scenario with linux as its third argument and exits 0', () => {
    const { status, stdout } = runWithAvStub('echo "gate=av-stub-pass"; exit 0');
    assert.equal(status, 0);
    const marker = stdout.split('\n').find((line) => line.startsWith(SCENARIO_MARKER));
    assert.ok(marker, stdout);
    const argv = marker.slice(`${SCENARIO_MARKER} argv=`.length).split(',');
    assert.equal(argv[2], 'linux');
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
});

test('vp-s3-linux.sh scenario leg exit contract', LINUX_ONLY, async (t) => {
  const AV_PASS = 'echo "gate=av-stub-pass"; exit 0';

  await t.test("the scenario's own blocker keeps its exit 1 and names no wedge", () => {
    const { status, stdout } = runWithAvStub(AV_PASS, 'process.exitCode = 1;');
    assert.equal(status, 1);
    assert.ok(!stdout.includes('gate=scenario-'));
  });

  await t.test('a scenario that outlives SCENARIO_TIMEOUT is named a timeout and exits 1', () => {
    const { status, stdout } = runWithAvStub(AV_PASS, 'setTimeout(() => {}, 30_000);', '1s');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-timeout'));
  });

  await t.test('a SIGKILLed scenario is named a kill, never a timeout', () => {
    const { status, stdout } = runWithAvStub(AV_PASS, "process.kill(process.pid, 'SIGKILL');");
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-killed'));
    assert.ok(!stdout.includes('gate=scenario-timeout'));
  });

  await t.test('a SIGTERM from outside the bound is named terminated, never a timeout or a kill', () => {
    // A Node process with no SIGTERM handler dies by signal (status 143),
    // unlike the scripts' own `timeout --kill-after` SIGTERM, which
    // scenario-session.mjs's handler catches to delete the session first.
    const { status, stdout } = runWithAvStub(AV_PASS, "process.kill(process.pid, 'SIGTERM');");
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-terminated'));
    assert.ok(!stdout.includes('gate=scenario-timeout'));
    assert.ok(!stdout.includes('gate=scenario-killed'));
  });

  await t.test('a status outside 0/1/2/124/137/143 is named unexpected, with its raw status', () => {
    const { status, stdout } = runWithAvStub(AV_PASS, 'process.exitCode = 127;');
    assert.equal(status, 1);
    assert.ok(stdout.includes('gate=scenario-unexpected-exit status=127'));
  });
});
