// `node --test` usage-error coverage for `vp-s13-scenario.mjs`'s argv
// guard (mirrors vp-s3-scenario.test.mjs; this script has no OS token, so
// the usage path itself is what is pinned). Spawns the real script rather
// than importing it -- the picker-drive/WebDriver paths need a real
// display and tauri-driver, covered by the CI dispatch, not node --test.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, 'vp-s13-scenario.mjs');

function run(...args) {
  return spawnSync('node', [SCRIPT, ...args], { encoding: 'utf8', timeout: 10_000 });
}

test('vp-s13-scenario.mjs usage error', async (t) => {
  await t.test('no arguments: exits 2 and names the usage blocker, never opening a session', () => {
    const { status, stdout } = run();
    assert.equal(status, 2);
    assert.match(stdout, /^blocker=usage: vp-s13-scenario\.mjs /m);
  });

  await t.test('missing the workspace directory: still exits 2; all three present: passes the usage gate', () => {
    assert.equal(run('http://127.0.0.1:9', '/path/to/app').status, 2);
    // Nothing listens on port 9: past the usage gate, session creation itself fails.
    const passed = run('http://127.0.0.1:9', '/path/to/app', '/path/to/workspace');
    assert.equal(passed.status, 1, passed.stdout);
    assert.doesNotMatch(passed.stdout, /^blocker=usage/m);
    assert.match(passed.stdout, /^blocker=session-create-failed: /m);
  });
});
