// `node --test` unit tests for `scenario-session.mjs`: the shared
// `withDeadline` helper (moved here from `vp-s1-scenario.mjs`, tests moved
// with it) and `runScenarioSession`. Everything runs in-process against a
// fake `webdriver`, a real `node:events` `EventEmitter` standing in for
// `process` as the SIGTERM source, and a fake `exit` that records its code
// instead of ending the test process -- no WebDriver session, subprocess,
// or display, mirroring every other `*-scenario.test.mjs` file.

import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { test } from 'node:test';

import {
  DEFAULT_DEADLINES,
  runScenarioSession,
  SESSION_CREATE_DEADLINE_MS,
  TERMINATION_TEARDOWN_DEADLINE_MS,
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

// Small, real (unmocked) deadlines for every test that deliberately expires
// one: tens of milliseconds keep the suite fast while still exercising the
// real withDeadline race. The one exception is the default-deadline test
// below, which pins the scenarios' own real 90 s default via mock.timers.
const FAST_DEADLINES = { sessionCreate: 300, call: 300, terminationTeardown: 50 };

/** One test's shared emit sink and SIGTERM source, plus a `run` shorthand
 * that supplies `runScenarioSession`'s common fields so each test states
 * only what it overrides (`webdriver`, `body`, `exit`, `deadlines`). */
function context() {
  const lines = [];
  const emit = (key, value) => lines.push(`${key}=${value}`);
  const signals = new EventEmitter();
  const run = (overrides) =>
    runScenarioSession({
      baseUrl: 'http://example.invalid',
      applicationPath: '/app',
      emit,
      signals,
      exit: () => assert.fail('exit must not be called'),
      ...overrides,
    });
  return { lines, emit, signals, run };
}

/** A `deleteSession` fake that counts its own calls, for tests asserting
 * "deleted at most once" without a bespoke counter each time. */
function countingDelete() {
  const state = { count: 0 };
  state.deleteSession = async () => {
    state.count += 1;
  };
  return state;
}

/** A promise plus its own resolve/reject, for tests that must control
 * exactly when a fake WebDriver call settles relative to a SIGTERM. */
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Polls `conditionFn` until true, for tests waiting on an in-flight async
 * call or a real `FAST_DEADLINES` timer. Polls on a real 1 ms tick, not a
 * bare microtask turn: `termination-teardown` is a real `setTimeout` and
 * needs actual wall-clock time regardless of how many turns are taken. */
async function waitFor(conditionFn) {
  for (let i = 0; i < 2_000; i++) {
    if (conditionFn()) return;
    await new Promise((r) => setTimeout(r, 1));
  }
  throw new Error('waitFor: condition never became true');
}

test('runScenarioSession: happy path emits session-created, the body\'s own lines, and session-closed; deletes once; resolves 0; leaves no SIGTERM listener', async () => {
  const { lines, emit, signals, run } = context();
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
  assert.equal(signals.listenerCount('SIGTERM'), 0);
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
  // The raw `createSession()` call behind `never` truly never settles, so
  // once the sessionCreate deadline wins, close() still
  // has to wait, bounded by `deadlines.call` (`DEFAULT_DEADLINES.call`
  // here), to find out whether it eventually succeeds and orphans a
  // session -- through several more `await`s than the simpler chain above.
  // A real `setImmediate` (unaffected by the `setTimeout`-only mock) fully
  // drains the microtask queue, so that wait's own timer is reliably
  // registered before it is ticked forward, however many hops it takes.
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.tick(DEFAULT_DEADLINES.call);
  const code = await resultPromise;

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    `blocker=session-create-failed: session-create exceeded ${SESSION_CREATE_DEADLINE_MS} ms`,
    `blocker=session-close-failed: late-session-create exceeded ${DEFAULT_DEADLINES.call} ms`,
  ]);
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

test('runScenarioSession: body throws a non-Error (null) -- unexpected-error: null, still one delete and session-closed; resolves 1', async () => {
  const { lines, run } = context();
  const del = countingDelete();

  const code = await run({
    webdriver: { createSession: async () => 'sess-1', deleteSession: del.deleteSession },
    body: async () => {
      throw null; // eslint-disable-line no-throw-literal -- deliberately non-Error
    },
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'blocker=unexpected-error: null',
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

test('runScenarioSession: delete never settles -- session-close-failed carrying the call deadline message; resolves 1', async () => {
  const { lines, run } = context();
  const deadlines = { ...FAST_DEADLINES, call: 30 };
  const never = new Promise(() => {});

  const code = await run({
    webdriver: { createSession: async () => 'sess-1', deleteSession: () => never },
    body: async () => {},
    deadlines,
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    `blocker=session-close-failed: session-delete exceeded ${deadlines.call} ms`,
  ]);
});

test('runScenarioSession: the sessionCreate deadline loses the race, then the raw create resolves late -- session-create-failed, then session-closed for the orphaned session; one delete; resolves 1', async () => {
  const { lines, run } = context();
  const del = countingDelete();
  const create = deferred();

  const resultPromise = run({
    webdriver: { createSession: () => create.promise, deleteSession: del.deleteSession },
    body: async () => assert.fail('body must not run once create-failed is reported'),
    deadlines: FAST_DEADLINES,
  });

  await waitFor(() => lines.includes(`blocker=session-create-failed: session-create exceeded ${FAST_DEADLINES.sessionCreate} ms`));
  create.resolve('sess-late');
  const code = await resultPromise;

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    `blocker=session-create-failed: session-create exceeded ${FAST_DEADLINES.sessionCreate} ms`,
    'gate=session-closed session_id=sess-late',
  ]);
  assert.equal(del.count, 1);
});

test('runScenarioSession: the sessionCreate deadline loses the race and the raw create never settles -- session-close-failed carrying the call deadline message; resolves 1', async () => {
  const { lines, run } = context();
  const never = new Promise(() => {});

  const code = await run({
    webdriver: {
      createSession: () => never,
      deleteSession: async () => assert.fail('delete must never be attempted with no session id'),
    },
    body: async () => assert.fail('body must not run once create-failed is reported'),
    deadlines: FAST_DEADLINES,
  });

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    `blocker=session-create-failed: session-create exceeded ${FAST_DEADLINES.sessionCreate} ms`,
    `blocker=session-close-failed: late-session-create exceeded ${FAST_DEADLINES.call} ms`,
  ]);
});

test('runScenarioSession: the sessionCreate deadline loses the race, then the raw create rejects on its own -- only session-create-failed; no delete; resolves 1', async () => {
  const { lines, run } = context();
  const del = countingDelete();
  const create = deferred();

  const resultPromise = run({
    webdriver: { createSession: () => create.promise, deleteSession: del.deleteSession },
    body: async () => assert.fail('body must not run once create-failed is reported'),
    deadlines: FAST_DEADLINES,
  });

  await waitFor(() => lines.includes(`blocker=session-create-failed: session-create exceeded ${FAST_DEADLINES.sessionCreate} ms`));
  create.reject(new Error('ECONNRESET'));
  const code = await resultPromise;

  assert.equal(code, 1);
  assert.deepEqual(lines, [`blocker=session-create-failed: session-create exceeded ${FAST_DEADLINES.sessionCreate} ms`]);
  assert.equal(del.count, 0);
});

test('runScenarioSession: a deadlines override without a terminationTeardown value still gets the real default, not the 30 s withDeadline fallback', async (t) => {
  // Real sessionCreate/call magnitudes (free under mock.timers): small ones
  // would let close() give up on its own long before any teardown bound.
  const { sessionCreate, call } = DEFAULT_DEADLINES;
  const overrides = {
    omitted: { sessionCreate, call },
    'explicitly undefined': { sessionCreate, call, terminationTeardown: undefined },
  };
  for (const [name, deadlines] of Object.entries(overrides)) {
    await t.test(name, async (st) => {
      st.mock.timers.enable({ apis: ['setTimeout'] });
      const { lines, signals, run } = context();
      const exitCodes = [];

      // Never awaited: the normal path legitimately stays pending on the
      // 90 s create, as in production, where exit() ends the process first.
      run({
        webdriver: {
          createSession: () => new Promise(() => {}),
          deleteSession: async () => assert.fail('delete must never be attempted with no session id'),
        },
        body: async () => assert.fail('body must never run once terminated'),
        exit: (code) => exitCodes.push(code),
        deadlines,
      });

      // A real setImmediate (not mocked) drains every microtask hop between steps.
      signals.emit('SIGTERM');
      await new Promise((resolve) => setImmediate(resolve));
      st.mock.timers.tick(TERMINATION_TEARDOWN_DEADLINE_MS - 1);
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(exitCodes.length, 0, 'must still be waiting 1 ms before the default termination-teardown deadline');

      st.mock.timers.tick(1);
      await new Promise((resolve) => setImmediate(resolve));

      assert.deepEqual(exitCodes, [143]);
      assert.deepEqual(lines, [
        'blocker=terminated: SIGTERM',
        `blocker=session-close-failed: termination-teardown exceeded ${TERMINATION_TEARDOWN_DEADLINE_MS} ms`,
      ]);
    });
  }
});

test('runScenarioSession: a raw create resolving in the same turn its sessionCreate deadline fires is still deleted', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { lines, run } = context();
  const del = countingDelete();
  const create = deferred();

  const resultPromise = run({
    webdriver: { createSession: () => create.promise, deleteSession: del.deleteSession },
    body: async () => assert.fail('body must not run once create-failed is reported'),
  });

  // The deadline fires and the raw create resolves before any microtask
  // runs, so the session exists before close() looks for it.
  t.mock.timers.tick(DEFAULT_DEADLINES.sessionCreate);
  create.resolve('sess-late');
  const code = await resultPromise;

  assert.equal(code, 1);
  assert.deepEqual(lines, [
    `blocker=session-create-failed: session-create exceeded ${DEFAULT_DEADLINES.sessionCreate} ms`,
    'gate=session-closed session_id=sess-late',
  ]);
  assert.equal(del.count, 1);
});

test('runScenarioSession: a createSession that returns a plain value or throws synchronously is handled like a promise', async (t) => {
  await t.test('plain value', async () => {
    const { lines, signals, run } = context();
    const code = await run({
      webdriver: { createSession: () => 'sess-sync', deleteSession: async () => {} },
      body: async () => {},
    });
    assert.equal(code, 0);
    assert.deepEqual(lines, ['gate=session-created session_id=sess-sync', 'gate=session-closed session_id=sess-sync']);
    assert.equal(signals.listenerCount('SIGTERM'), 0);
  });

  await t.test('synchronous throw', async () => {
    const { lines, signals, run } = context();
    const code = await run({
      webdriver: {
        createSession: () => {
          throw new Error('bad-arguments');
        },
        deleteSession: async () => assert.fail('delete must never be attempted with no session id'),
      },
      body: async () => assert.fail('body must not run when create fails'),
    });
    assert.equal(code, 1);
    assert.deepEqual(lines, ['blocker=session-create-failed: bad-arguments']);
    assert.equal(signals.listenerCount('SIGTERM'), 0);
  });
});

test('runScenarioSession: an emit that throws on the terminated line still deletes the session and reaches exit(143), with no unhandled rejection', async () => {
  const { signals, run } = context();
  const del = countingDelete();
  const exitCodes = [];
  const emittedKeys = [];

  const throwingEmit = (key, value) => {
    emittedKeys.push(`${key}=${value}`);
    if (value === 'terminated: SIGTERM') {
      throw new Error('EPIPE');
    }
  };

  run({
    emit: throwingEmit,
    webdriver: { createSession: async () => 'sess-1', deleteSession: del.deleteSession },
    body: () => new Promise(() => {}), // never resolves; the signal is what ends this run
    exit: (code) => exitCodes.push(code),
    deadlines: FAST_DEADLINES,
  });

  await waitFor(() => emittedKeys.includes('gate=session-created session_id=sess-1'));
  signals.emit('SIGTERM');
  await waitFor(() => exitCodes.length === 1);

  assert.deepEqual(exitCodes, [143]);
  // A broken transcript must not also leave the packaged app running.
  assert.equal(del.count, 1);
});

test('runScenarioSession: SIGTERM during body -- terminated blocker, exactly one delete, session-closed, exit(143); a late body rejection emits no unexpected-error', async () => {
  const { lines, signals, run } = context();
  const del = countingDelete();
  const exitCodes = [];
  const body = deferred();

  const resultPromise = run({
    webdriver: { createSession: async () => 'sess-1', deleteSession: del.deleteSession },
    body: () => body.promise,
    exit: (code) => exitCodes.push(code),
    deadlines: FAST_DEADLINES,
  });

  await waitFor(() => lines.includes('gate=session-created session_id=sess-1'));
  signals.emit('SIGTERM');
  await waitFor(() => exitCodes.length === 1);

  // The body's own promise rejects only after the signal was already
  // handled -- this must produce no `unexpected-error` line. Awaiting the
  // result lets the normal path's catch (and its re-entrant, memoized
  // `close()` call) run to completion before asserting.
  body.reject(new Error('late-rejection-after-signal'));
  await resultPromise;

  assert.deepEqual(exitCodes, [143]);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'blocker=terminated: SIGTERM',
    'gate=session-closed session_id=sess-1',
  ]);
  assert.equal(del.count, 1);
});

test('runScenarioSession: SIGTERM during create, create resolving inside the window -- one delete for that id, no session-created gate, body never called, exit(143)', async () => {
  const { lines, signals, run } = context();
  const del = countingDelete();
  const create = deferred();
  const exitCodes = [];

  const resultPromise = run({
    webdriver: { createSession: () => create.promise, deleteSession: del.deleteSession },
    body: async () => assert.fail('body must never run once terminated'),
    exit: (code) => exitCodes.push(code),
    deadlines: FAST_DEADLINES,
  });

  // Fire the signal before create ever settles, then let the still-pending
  // creation resolve inside the termination-teardown window.
  signals.emit('SIGTERM');
  await waitFor(() => lines.includes('blocker=terminated: SIGTERM'));
  create.resolve('sess-late');
  await waitFor(() => exitCodes.length === 1);

  assert.deepEqual(exitCodes, [143]);
  assert.deepEqual(lines, ['blocker=terminated: SIGTERM', 'gate=session-closed session_id=sess-late']);
  assert.equal(del.count, 1);
  await resultPromise;
});

test('runScenarioSession: SIGTERM during create, create never settling -- session-close-failed carrying the termination-teardown deadline message, exit(143)', async () => {
  const { lines, signals, run } = context();
  const never = new Promise(() => {});
  const exitCodes = [];

  const resultPromise = run({
    webdriver: {
      createSession: () => never,
      deleteSession: async () => assert.fail('delete must never be attempted with no session id'),
    },
    body: async () => assert.fail('body must never run once terminated'),
    exit: (code) => exitCodes.push(code),
    deadlines: FAST_DEADLINES,
  });

  signals.emit('SIGTERM');
  await waitFor(() => exitCodes.length === 1);

  assert.deepEqual(exitCodes, [143]);
  assert.deepEqual(lines, [
    'blocker=terminated: SIGTERM',
    `blocker=session-close-failed: termination-teardown exceeded ${FAST_DEADLINES.terminationTeardown} ms`,
  ]);
  await resultPromise;
});

test('runScenarioSession: SIGTERM while the normal path\'s close() is already in flight -- still exactly one delete call and one session-closed line', async () => {
  const { lines, signals, run } = context();
  let deleteCalls = 0;
  const deleteCall = deferred();
  const exitCodes = [];

  const resultPromise = run({
    webdriver: {
      createSession: async () => 'sess-1',
      deleteSession: () => {
        deleteCalls += 1;
        return deleteCall.promise;
      },
    },
    body: async () => {},
    exit: (code) => exitCodes.push(code),
    deadlines: FAST_DEADLINES,
  });

  // Wait until the normal path's own close() has already issued the
  // DELETE and is waiting on it, then fire the signal.
  await waitFor(() => deleteCalls === 1);
  signals.emit('SIGTERM');
  await waitFor(() => lines.includes('blocker=terminated: SIGTERM'));
  deleteCall.resolve();
  await waitFor(() => exitCodes.length === 1);

  assert.equal(deleteCalls, 1);
  assert.deepEqual(lines, [
    'gate=session-created session_id=sess-1',
    'blocker=terminated: SIGTERM',
    'gate=session-closed session_id=sess-1',
  ]);
  await resultPromise;
});
