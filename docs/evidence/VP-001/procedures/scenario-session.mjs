// Shared WebDriver session lifecycle for `vp-s1-scenario.mjs` and
// `vp-s3-scenario.mjs`: create one session, run the scenario's own `body`,
// and delete the session, through one shared `close()`.

import { createSession, deleteSession } from './webdriver-session.mjs';

// Every WebDriver call is given this long; an unresponsive driver becomes
// a blocker naming the call, never an open-ended wait.
const CALL_DEADLINE_MS = 30_000;
// Creating the session launches the packaged app, which CALL_DEADLINE_MS
// cannot cover: CI job logs measured it at 30.2-32.6 s for the first session
// of a step and 11.4-13.6 s for VP-S1's own, and VP-S3's first run (36352840629)
// was blocked by this 30 s bound. Its cause is not established; the bound
// leaves headroom and still fits vp-s1/vp-s3-linux.sh's 300 s scenario limit.
export const SESSION_CREATE_DEADLINE_MS = 90_000;

// Exported for both scenarios and for `node --test`
// (`scenario-session.test.mjs`). The optional `ms` defaults to
// `CALL_DEADLINE_MS`; session creation passes `SESSION_CREATE_DEADLINE_MS`,
// and tests pass a few milliseconds instead of waiting 30 s. Whichever of `promise`/`deadline` settles
// first decides the race; `.finally` then always clears the deadline timer
// -- including when `promise` wins -- so a wedged WebDriver call never
// outlives its own deadline as a dangling timer. When the deadline wins
// instead, the underlying WebDriver request itself is left running:
// `webdriver-session.mjs`'s `fetch` call takes no `AbortSignal`, and wiring
// one through would change its exported functions' signatures, so this
// stays a documented gap rather than a redesign of that module.
export function withDeadline(promise, label, ms = CALL_DEADLINE_MS) {
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} exceeded ${ms} ms`)), ms);
  });
  return Promise.race([promise, deadline]).finally(() => clearTimeout(timer));
}

/**
 * Create the session, run `body(sessionId)`, and delete the session --
 * through one shared, memoized `close()`, the only place a DELETE is ever
 * issued. Resolves `0` on a clean run or `1` if any step failed.
 * `webdriver`/`deadlines` are overridable for in-process testing;
 * `deadlines` is taken whole, not merged with the defaults, since every
 * call site already supplies both fields.
 */
export async function runScenarioSession({
  baseUrl,
  applicationPath,
  emit,
  body,
  webdriver = { createSession, deleteSession },
  deadlines = { sessionCreate: SESSION_CREATE_DEADLINE_MS, call: CALL_DEADLINE_MS },
}) {
  let closePromise = null;
  let sessionPromise;

  function close() {
    if (closePromise === null) {
      closePromise = (async () => {
        let id;
        try {
          id = await sessionPromise;
        } catch {
          return; // create failed: nothing to delete
        }
        try {
          await withDeadline(webdriver.deleteSession(baseUrl, id), 'session-delete', deadlines.call);
          emit('gate', `session-closed session_id=${id}`);
        } catch (error) {
          emit('blocker', `session-close-failed: ${error.message}`);
          throw error;
        }
      })();
    }
    return closePromise;
  }

  sessionPromise = withDeadline(
    webdriver.createSession(baseUrl, applicationPath),
    'session-create',
    deadlines.sessionCreate,
  );

  let failed = false;
  try {
    const sessionId = await sessionPromise;
    emit('gate', `session-created session_id=${sessionId}`);
    try {
      await body(sessionId);
    } catch (error) {
      failed = true;
      emit('blocker', `unexpected-error: ${error.message}`);
      if (process.env.VP001_DEBUG) console.error(error);
    }
  } catch (error) {
    failed = true;
    emit('blocker', `session-create-failed: ${error.message}`);
  }

  try {
    // Always called, even after a create failure (then a no-op): the one
    // place this ever issues a DELETE.
    await close();
  } catch {
    failed = true;
  }

  return failed ? 1 : 0;
}
