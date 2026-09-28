// `node --test` unit tests for `scenario-session.mjs`: the shared
// `withDeadline` helper (moved here from `vp-s1-scenario.mjs`, tests moved
// with it) and `runScenarioSession`'s normal-path lifecycle. Everything
// here runs in-process against a fake `webdriver` -- no WebDriver session,
// subprocess, or display, mirroring every other `*-scenario.test.mjs` file.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  runScenarioSession,
  SESSION_CREATE_DEADLINE_MS,
  withDeadline,
} from './scenario-session.mjs';

test('SESSION_CREATE_DEADLINE_MS leaves at least twice the slowest session creation measured on CI (32.6 s)', () => {
  assert.ok(SESSION_CREATE_DEADLINE_MS >= 2 * 32_600, String(SESSION_CREATE_DEADLINE_MS));
});

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

// -- runScenarioSession --

/** A `deleteSession` fake that counts its own calls, for tests asserting
 * "deleted at most once" without a bespoke counter each time. */
function countingDelete() {
  const state = { count: 0 };
  state.deleteSession = async () => {
    state.count += 1;
  };
  return state;
}

/** One test's shared emit sink, plus a `run` shorthand that supplies
 * `runScenarioSession`'s common fields so each test states only what it
 * overrides (`webdriver`, `body`, `deadlines`). */
function context() {
  const lines = [];
  const emit = (key, value) => lines.push(`${key}=${value}`);
  const run = (overrides) =>
    runScenarioSession({
      baseUrl: 'http://example.invalid',
      applicationPath: '/app',
      emit,
      ...overrides,
    });
  return { lines, emit, run };
}

test('runScenarioSession: happy path emits session-created, the body\'s own lines, and session-closed; deletes once; resolves 0', async () => {
  const { lines, emit, run } = context();
  const del = countingDelete();

  const code = await run({
    webdriver: { createSession: async () => 'sess-1', deleteSession: del.deleteSession },
    body: async (sessionId) => emit('gate', `armed session_id=${sessionId}`),
  });

  assert.equal(code, 0);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'gate=armed session_id=sess-1',
    'gate=session-closed session_id=sess-1',
  ]);
  assert.equal(del.count, 1);
});

test('runScenarioSession: create rejects -- session-create-failed, no delete, resolves 1', async () => {
  const { lines, run } = context();
  const del = countingDelete();

  const code = await run({
    webdriver: {
      createSession: async () => {
        throw new Error('ECONNREFUSED');
      },
      deleteSession: del.deleteSession,
    },
    body: async () => assert.fail('body must not run when create fails'),
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, ['blocker=session-create-failed: ECONNREFUSED']);
  assert.equal(del.count, 0);
});

test('runScenarioSession: default deadlines -- a never-settling create is still pending 1 ms before SESSION_CREATE_DEADLINE_MS and fails exactly on it', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { lines, run } = context();
  const never = new Promise(() => {});

  // `deadlines` intentionally omitted below: this pins the default the
  // scenarios actually call with, not just the exported constant.
  const resultPromise = run({
    webdriver: { createSession: () => never, deleteSession: async () => {} },
    body: async () => assert.fail('body must not run when create never settles'),
  });

  let settled = false;
  resultPromise.then(() => {
    settled = true;
  });

  t.mock.timers.tick(SESSION_CREATE_DEADLINE_MS - 1);
  // Flush microtasks a few times so a premature settle shows up here,
  // rather than being masked by the final `await` below.
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(settled, false, 'must still be waiting 1 ms before the deadline');

  t.mock.timers.tick(1);
  const code = await resultPromise;

  assert.equal(code, 1);
  assert.deepEqual(lines, [`blocker=session-create-failed: session-create exceeded ${SESSION_CREATE_DEADLINE_MS} ms`]);
});

test('runScenarioSession: body throws -- unexpected-error, then one delete and session-closed; resolves 1', async () => {
  const { lines, run } = context();
  const del = countingDelete();

  const code = await run({
    webdriver: { createSession: async () => 'sess-1', deleteSession: del.deleteSession },
    body: async () => {
      throw new Error('script-failed');
    },
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'blocker=unexpected-error: script-failed',
    'gate=session-closed session_id=sess-1',
  ]);
  assert.equal(del.count, 1);
});

test('runScenarioSession: delete rejects -- session-close-failed; resolves 1', async () => {
  const { lines, run } = context();

  const code = await run({
    webdriver: {
      createSession: async () => 'sess-1',
      deleteSession: async () => {
        throw new Error('DELETE failed with HTTP 500');
      },
    },
    body: async () => {},
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'blocker=session-close-failed: DELETE failed with HTTP 500',
  ]);
});
