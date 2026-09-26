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
// AV1/AV2 feasibility check, bounded by a 1 s AV_TIMEOUT.
function runWithAvStub(stubBody) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s1-linux-'));
  try {
    copyFileSync(join(HERE, 'vp-s1-linux.sh'), join(dir, 'vp-s1-linux.sh'));
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${stubBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s1-scenario.mjs'), `process.stdout.write('${SCENARIO_MARKER}\\n');\n`);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const result = spawnSync('bash', [join(dir, 'vp-s1-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env: { ...process.env, VP001_AV_TIMEOUT: '1s' },
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
});
