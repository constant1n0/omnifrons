// Shared WebDriver session lifecycle for `vp-s1-scenario.mjs` and
// `vp-s3-scenario.mjs`: create one session, run the scenario's own `body`,
// and delete the session -- on the normal path, and also on the SIGTERM
// that both scripts' `timeout --kill-after=10s` sends when the scenario's
// bound fires. Without a handler, that SIGTERM skipped the session delete
// and the packaged app outlived the scenario.

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
// Both scripts' `timeout --kill-after=10s` leaves 10 s between SIGTERM and
// SIGKILL; teardown -- deleting the session and printing its own line --
// must finish inside that, or SIGKILL lands with the packaged app still up.
export const TERMINATION_TEARDOWN_DEADLINE_MS = 8_000;

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

/** `error.message`, or `String(error)` when the caught value is not an
 * `Error` (e.g. `throw null`/`throw undefined` from `body`). Every catch
 * site below formats a message from whatever was thrown or rejected with,
 * and must not itself throw doing so. */
function messageOf(error) {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Create the session, run `body(sessionId)`, and delete the session --
 * exactly once, on the normal path or on SIGTERM. Resolves `0`/`1`; a
 * terminated run instead calls `exit` (default `process.exit(143)`), which
 * never returns in production, but tests inject a fake that records the
 * code so this can still settle and be asserted. `signals`/`webdriver`/
 * `deadlines` are likewise overridable for in-process testing; `deadlines`
 * is taken whole, not merged with the defaults, since every call site
 * already supplies all three fields.
 */
export async function runScenarioSession({
  baseUrl,
  applicationPath,
  emit,
  body,
  signals = process,
  exit = (code) => process.exit(code),
  webdriver = { createSession, deleteSession },
  deadlines = {
    sessionCreate: SESSION_CREATE_DEADLINE_MS,
    call: CALL_DEADLINE_MS,
    terminationTeardown: TERMINATION_TEARDOWN_DEADLINE_MS,
  },
}) {
  // Re-checked after every later await, since a termination can land at any one.
  let terminated = false;
  // Memoized: the normal path and the SIGTERM handler can both reach this,
  // and must always await the very same delete attempt so a session is
  // deleted at most once, regardless of which one gets there first.
  let closePromise = null;
  // Lets the handler below tell "close() itself already emitted its own
  // session-close-failed line" apart from "the bounding deadline elapsed
  // first, and close() has not emitted anything yet".
  let closeEmittedFailure = false;
  let sessionPromise; // declared before the listener below reads it

  function close() {
    if (closePromise === null) {
      closePromise = (async () => {
        let id;
        try {
          id = await sessionPromise;
        } catch {
          return; // create failed, or never finished: nothing to delete
        }
        try {
          await withDeadline(webdriver.deleteSession(baseUrl, id), 'session-delete', deadlines.call);
          emit('gate', `session-closed session_id=${id}`);
        } catch (error) {
          closeEmittedFailure = true;
          emit('blocker', `session-close-failed: ${messageOf(error)}`);
          throw error;
        }
      })();
    }
    return closePromise;
  }

  async function onSigterm() {
    terminated = true;
    emit('blocker', 'terminated: SIGTERM');
    try {
      // Bounds the whole close() attempt, including a session still mid-creation.
      await withDeadline(close(), 'termination-teardown', deadlines.terminationTeardown);
    } catch (error) {
      // Report here only when close() itself never got to fail on its own
      // (the deadline elapsed first); otherwise it already emitted its line.
      if (!closeEmittedFailure) {
        emit('blocker', `session-close-failed: ${messageOf(error)}`);
      }
    }
    exit(143);
  }

  // Installed before `createSession` is called: a first attempt registered
  // this afterward, so a SIGTERM landing during the 30-90 s session-create
  // leaked the session. Only SIGTERM is handled -- `timeout` sends exactly
  // that -- and `once` removes itself after firing, with no removal needed here.
  signals.once('SIGTERM', onSigterm);

  sessionPromise = withDeadline(
    webdriver.createSession(baseUrl, applicationPath),
    'session-create',
    deadlines.sessionCreate,
  );

  let failed = false;
  try {
    const sessionId = await sessionPromise;
    // Once terminated, the handler above owns the transcript's last word:
    // no session-created gate for a late-resolving session, and no body.
    if (!terminated) {
      emit('gate', `session-created session_id=${sessionId}`);
      try {
        await body(sessionId);
      } catch (error) {
        failed = true;
        // A body rejecting after termination (its session usually already
        // gone) is not a fresh error on top of the SIGTERM already reported.
        if (!terminated) {
          emit('blocker', `unexpected-error: ${messageOf(error)}`);
          if (process.env.VP001_DEBUG) console.error(error);
        }
      }
    }
  } catch (error) {
    failed = true;
    if (!terminated) {
      emit('blocker', `session-create-failed: ${messageOf(error)}`);
    }
  }

  try {
    // Always called, even after a create failure (then a no-op): the one
    // place, shared with the SIGTERM handler, that ever issues a DELETE.
    await close();
  } catch {
    failed = true;
  }

  signals.off('SIGTERM', onSigterm);
  return failed ? 1 : 0;
}
