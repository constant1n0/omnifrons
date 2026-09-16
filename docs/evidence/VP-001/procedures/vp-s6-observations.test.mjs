// `node --test` unit tests for `vp-s6-observations.mjs`'s pure helpers --
// written RED, before they exist (design.md Threat Matrix "Subprocess /
// process integration": pid identity compared by `(pid, starttime)` so a
// recycled pid never reads as a survivor). These exercise no process,
// filesystem, or display: `vp-s6-scenario.mjs`'s own orchestration
// (xdotool driving, WebDriver calls, `/proc` reads) is the E2E-only path,
// never covered here.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  classifyPidAfterWait,
  parsePidFile,
  parseProcStatStarttime,
} from './vp-s6-observations.mjs';

test('parsePidFile', async (t) => {
  await t.test('extracts self and descendant pids, ignoring the recorded starttimes', () => {
    assert.deepEqual(parsePidFile('self 111 999\ndescendant 222 999\n'), {
      selfPid: 111,
      descendantPid: 222,
    });
  });

  await t.test('rejects a file missing either role', () => {
    assert.throws(() => parsePidFile('self 111 999\n'), Error);
    assert.throws(() => parsePidFile(''), Error);
  });
});

test('parseProcStatStarttime', async (t) => {
  await t.test('reads the starttime field (proc(5) field 22)', () => {
    const stat =
      '4242 (vp-s6-agent) S 1 4242 4242 0 -1 4194560 100 0 0 0 0 0 0 0 20 0 1 0 999888 ...';
    assert.equal(parseProcStatStarttime(stat), 999_888);
  });

  await t.test('handles a comm field containing spaces and parens', () => {
    const stat = '77 (cmd) with ) parens) S 1 77 77 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 555000 ...';
    assert.equal(parseProcStatStarttime(stat), 555_000);
  });

  await t.test('returns null for a line with too few fields', () => {
    assert.equal(parseProcStatStarttime('1 (init) S 0 1 1'), null);
  });
});

test('classifyPidAfterWait', async (t) => {
  await t.test('gone when the pid no longer exists', () => {
    assert.equal(classifyPidAfterWait(999_888, null), 'gone');
  });

  await t.test('alive when the pid exists with the same starttime', () => {
    assert.equal(classifyPidAfterWait(999_888, 999_888), 'alive');
  });

  await t.test('recycled (never a survivor) when the pid exists with a different starttime', () => {
    assert.equal(classifyPidAfterWait(999_888, 12_345), 'recycled');
  });
});
