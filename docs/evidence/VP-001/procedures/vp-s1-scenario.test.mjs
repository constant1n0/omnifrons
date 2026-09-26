// `node --test` unit tests for `vp-s1-scenario.mjs`'s exported `withDeadline`
// helper. No WebDriver session, process, or display -- `main`'s own
// orchestration is the E2E-only path, never covered here (mirrors
// `vp-s1-probe.test.mjs`'s own scope note). Importing this module never
// runs `main`: its direct-execution guard compares `process.argv[1]`
// against this module's own URL, which the test runner's own argv never
// matches.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { withDeadline } from './vp-s1-scenario.mjs';

test('withDeadline', async (t) => {
  await t.test('resolves with the wrapped promise\'s own value when it settles well before the deadline', async () => {
    const value = await withDeadline(Promise.resolve('ok'), 'test-call', 1_000);
    assert.equal(value, 'ok');
  });

  await t.test('rejects with the wrapped promise\'s own error when it rejects before the deadline', async () => {
    await assert.rejects(withDeadline(Promise.reject(new Error('boom')), 'test-call', 1_000), /boom/);
  });

  await t.test('rejects naming the label and the bound once the deadline elapses before the wrapped promise settles', async () => {
    const neverSettles = new Promise(() => {});
    await assert.rejects(withDeadline(neverSettles, 'slow-call', 10), /slow-call exceeded 10 ms/);
  });

  await t.test('clears the deadline timer once the wrapped promise settles first, instead of leaving it pending', async () => {
    const originalClearTimeout = globalThis.clearTimeout;
    let clearedArg;
    globalThis.clearTimeout = (handle) => {
      clearedArg = handle;
      return originalClearTimeout(handle);
    };
    try {
      const result = await withDeadline(Promise.resolve('done'), 'fast-call', 5_000);
      assert.equal(result, 'done');
      assert.notEqual(clearedArg, undefined);
    } finally {
      globalThis.clearTimeout = originalClearTimeout;
    }
  });
});
