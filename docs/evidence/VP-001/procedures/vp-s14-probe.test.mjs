// `node --test` unit tests for `vp-s14-probe.mjs`'s pure in-page script
// builders, no WebDriver (mirrors vp-s13-probe.test.mjs). The derive/
// formatting side (`summarize`, `formatObservations`, `formatCaseLines`,
// `formatRecordLines`) has its own module and its own
// `vp-s14-observations.test.mjs`.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  buildApproveScript, buildListScript, buildPickArmScript, buildPickReadScript,
  buildSpawnScript, buildStopScript,
} from './vp-s14-probe.mjs';

/** Runs `script` against a fake `window`; `invoke(cmd, args)` stands in for
 * `window.__TAURI_INTERNALS__.invoke`, `transformCallback(cb)` for
 * `window.__TAURI_INTERNALS__.transformCallback` (default: returns 1,
 * never calling `cb`). Omitting `__TAURI_INTERNALS__` entirely (pass
 * `null`) simulates a page where it was never injected. */
function runArmed(script, invoke, transformCallback) {
  const fakeWindow = invoke === null
    ? {}
    : { __TAURI_INTERNALS__: { invoke, transformCallback: transformCallback ?? (() => 1) } };
  new Function('window', script)(fakeWindow);
  return fakeWindow;
}
async function settleMicrotasks(fakeWindow) {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return fakeWindow;
}

const CHANGED_SINCE_APPROVAL_ERROR = {
  code: 'changed-since-approval',
  message: "the executable's content has changed since it was approved",
  detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
};
const SHADOWED_PATH_ERROR = { code: 'shadowed-path', message: 'the approved path now resolves somewhere else' };

test('buildPickArmScript / buildPickReadScript', async (t) => {
  await t.test('resolved: records candidateId/evidence, invokes with no args', async () => {
    const seen = [];
    const invoke = (cmd, args) => { seen.push([cmd, args]); return Promise.resolve({ candidateId: 7, evidence: { canonicalPath: '/scratch/true', size: 10, sha256: 'sha-a' } }); };
    const fakeWindow = await settleMicrotasks(runArmed(buildPickArmScript(), invoke));
    assert.deepEqual(seen, [['executable_pick_and_probe', undefined]]);
    assert.deepEqual(fakeWindow.__vpS14.pick, { phase: 'resolved', candidateId: 7, evidence: { canonicalPath: '/scratch/true', size: 10, sha256: 'sha-a' } });
  });
  await t.test("rejected: the shell's {code, message} shape collapses to one string", async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildPickArmScript(), () => Promise.reject({ code: 'no-candidate', message: 'no file was selected' })));
    assert.deepEqual(fakeWindow.__vpS14.pick, { phase: 'rejected', error: 'no-candidate: no file was selected' });
  });
  await t.test('a synchronous throw settles as rejected, never an uncaught exception', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildPickArmScript(), () => { throw new Error('boom-sync'); }));
    assert.equal(fakeWindow.__vpS14.pick.phase, 'rejected');
    assert.equal(fakeWindow.__vpS14.pick.error, 'Error: boom-sync');
  });
  await t.test('no __TAURI_INTERNALS__: stays pending, nothing invoked', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildPickArmScript(), null));
    assert.deepEqual(fakeWindow.__vpS14.pick, { phase: 'pending' });
  });
  await t.test('buildPickReadScript: empty snapshot when never armed, full deep-cloned snapshot after', () => {
    assert.deepEqual(new Function('window', 'location', buildPickReadScript())({}, { protocol: 'tauri:' }), { location_protocol: 'tauri:' });
    const fakeWindow = { __vpS14: { pick: { phase: 'resolved', candidateId: 1 } } };
    const snapshot = new Function('window', 'location', buildPickReadScript())(fakeWindow, { protocol: 'tauri:' });
    assert.deepEqual(snapshot, { pick: { phase: 'resolved', candidateId: 1 }, location_protocol: 'tauri:' });
    snapshot.pick.candidateId = 999; // deep clone: mutating the read-back snapshot must not touch the live state
    assert.equal(fakeWindow.__vpS14.pick.candidateId, 1);
  });
});

test('buildApproveScript', async (t) => {
  await t.test('invokes with exactly { candidateId }, records approvalId/evidence on resolve', async () => {
    const seen = [];
    const invoke = (cmd, args) => { seen.push([cmd, args]); return Promise.resolve({ approvalId: 'aaaaaaaaaaaaaaaa', evidence: { canonicalPath: '/scratch/true', size: 10, sha256: 'sha-a' } }); };
    const fakeWindow = await settleMicrotasks(runArmed(buildApproveScript(42), invoke));
    assert.deepEqual(seen, [['executable_approve', { candidateId: 42 }]]);
    assert.deepEqual(fakeWindow.__vpS14.approve, { phase: 'resolved', approvalId: 'aaaaaaaaaaaaaaaa', evidence: { canonicalPath: '/scratch/true', size: 10, sha256: 'sha-a' } });
  });
  await t.test('rejected: collapses to one string', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildApproveScript(42), () => Promise.reject({ code: 'no-candidate', message: 'no candidate' })));
    assert.deepEqual(fakeWindow.__vpS14.approve, { phase: 'rejected', error: 'no-candidate: no candidate' });
  });
  await t.test('a synchronous throw settles as rejected', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildApproveScript(42), () => { throw 'boom'; }));
    assert.deepEqual(fakeWindow.__vpS14.approve, { phase: 'rejected', error: 'boom' });
  });
  await t.test('no __TAURI_INTERNALS__: stays pending', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildApproveScript(42), null));
    assert.deepEqual(fakeWindow.__vpS14.approve, { phase: 'pending' });
  });
});

test('buildListScript', async (t) => {
  await t.test('invokes approvals_list with no args, records the array', async () => {
    const seen = [];
    const invoke = (cmd, args) => { seen.push([cmd, args]); return Promise.resolve([{ approvalId: 'x' }]); };
    const fakeWindow = await settleMicrotasks(runArmed(buildListScript(), invoke));
    assert.deepEqual(seen, [['approvals_list', undefined]]);
    assert.deepEqual(fakeWindow.__vpS14.list, { phase: 'resolved', approvals: [{ approvalId: 'x' }] });
  });
  await t.test('a non-array resolved value is coerced to an empty array, never thrown', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildListScript(), () => Promise.resolve(undefined)));
    assert.deepEqual(fakeWindow.__vpS14.list, { phase: 'resolved', approvals: [] });
  });
  await t.test('rejected: collapses to one string', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildListScript(), () => Promise.reject('approval-store-unavailable: the approval store is unavailable')));
    assert.deepEqual(fakeWindow.__vpS14.list, { phase: 'rejected', error: 'approval-store-unavailable: the approval store is unavailable' });
  });
});

test('buildSpawnScript', async (t) => {
  await t.test('invokes harness_spawn with the verified kind shape (tag field "type", not "kind") and a Channel-compatible onFrame', async () => {
    const seen = [];
    const invoke = (cmd, args) => { seen.push([cmd, args]); return Promise.resolve(321); };
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'a'), invoke, () => 55));
    assert.equal(seen.length, 1);
    const [cmd, args] = seen[0];
    assert.equal(cmd, 'harness_spawn');
    assert.deepEqual(args.kind, { type: 'approved', approvalId: 'cafecafecafecafe' });
    assert.equal(args.onFrame.id, 55);
    assert.equal(args.onFrame.toJSON(), '__CHANNEL__:55');
    assert.equal(args.onFrame['__TAURI_TO_IPC_KEY__'](), '__CHANNEL__:55');
    assert.deepEqual(fakeWindow.__vpS14.cases.a, { outcome: 'resolved', code: null, frameCount: 0, processId: 321 });
  });
  await t.test('rejected with the typed changed-since-approval shape: code, message and detail all captured', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'b'), () => Promise.reject(CHANGED_SINCE_APPROVAL_ERROR)));
    assert.deepEqual(fakeWindow.__vpS14.cases.b, {
      outcome: 'rejected', code: 'changed-since-approval', frameCount: 0,
      message: CHANGED_SINCE_APPROVAL_ERROR.message, detail: CHANGED_SINCE_APPROVAL_ERROR.detail,
    });
  });
  await t.test('rejected with the typed shadowed-path shape, which carries no detail at all, gives detail: null', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'c'), () => Promise.reject(SHADOWED_PATH_ERROR)));
    assert.deepEqual(fakeWindow.__vpS14.cases.c, {
      outcome: 'rejected', code: 'shadowed-path', frameCount: 0, message: SHADOWED_PATH_ERROR.message, detail: null,
    });
  });
  await t.test('an untyped rejection gives code: null, message as the stringified value', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'b'), () => Promise.reject('spawn failed')));
    assert.deepEqual(fakeWindow.__vpS14.cases.b, { outcome: 'rejected', code: null, frameCount: 0, message: 'spawn failed', detail: null });
  });
  await t.test('a synchronous throw settles as rejected, never an uncaught exception', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'd'), () => { throw new Error('boom-sync'); }));
    assert.equal(fakeWindow.__vpS14.cases.d.outcome, 'rejected');
    assert.equal(fakeWindow.__vpS14.cases.d.message, 'Error: boom-sync');
  });
  await t.test('frames delivered on the channel are counted, an end marker is not counted, order is irrelevant to the count', async () => {
    let capturedCallback;
    const transformCallback = (cb) => { capturedCallback = cb; return 1; };
    const fakeWindow = runArmed(buildSpawnScript('cafecafecafecafe', 'a'), () => new Promise(() => {}), transformCallback);
    await settleMicrotasks(fakeWindow);
    capturedCallback({ index: 0, message: { stream: 'stdout' } });
    capturedCallback({ index: 2, message: { stream: 'stdout' } }); // out of order: still one more frame
    capturedCallback({ index: 1, message: { stream: 'stderr' } });
    capturedCallback({ index: 3, end: true }); // not a frame
    assert.equal(fakeWindow.__vpS14.cases.a.frameCount, 3);
  });
  await t.test('distinct labels coexist in the same cases object', async () => {
    const fakeWindow = runArmed(buildSpawnScript('cafecafecafecafe', 'a'), () => Promise.resolve(1));
    new Function('window', buildSpawnScript('deadbeefdeadbeef', 'b'))(fakeWindow);
    await settleMicrotasks(fakeWindow);
    assert.ok(fakeWindow.__vpS14.cases.a);
    assert.ok(fakeWindow.__vpS14.cases.b);
  });
  await t.test('a re-armed label is never overwritten by the earlier spawn: its late settlement and frames stay on the old record', async () => {
    const pending = [];
    const callbacks = [];
    const invoke = () => new Promise((resolve) => { pending.push(resolve); });
    const transformCallback = (cb) => { callbacks.push(cb); return callbacks.length; };
    const fakeWindow = runArmed(buildSpawnScript('cafecafecafecafe', 'a'), invoke, transformCallback);
    const firstRecord = fakeWindow.__vpS14.cases.a;
    new Function('window', buildSpawnScript('cafecafecafecafe', 'a'))(fakeWindow);
    pending[1](2); // the re-armed spawn settles first
    await settleMicrotasks(fakeWindow);
    pending[0](1); // the earlier spawn settles late
    callbacks[0]({ message: 'late frame', index: 0 }); // and its channel still delivers
    await settleMicrotasks(fakeWindow);
    assert.equal(fakeWindow.__vpS14.cases.a.processId, 2, 'the current record keeps the re-armed spawn\'s result');
    assert.equal(fakeWindow.__vpS14.cases.a.frameCount, 0, 'no frame from the earlier channel is counted on it');
    assert.equal(firstRecord.processId, 1, 'the late settlement lands on the record it was armed with');
    assert.equal(firstRecord.frameCount, 1);
  });
  await t.test('no __TAURI_INTERNALS__: stays pending, transformCallback is never reached', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildSpawnScript('cafecafecafecafe', 'a'), null));
    assert.deepEqual(fakeWindow.__vpS14.cases.a, { outcome: 'pending', code: null, frameCount: 0 });
  });
});

test('buildStopScript', async (t) => {
  await t.test('invokes harness_stop with { id, deadlineMs }, defaulting deadlineMs to 5000', async () => {
    const seen = [];
    const invoke = (cmd, args) => { seen.push([cmd, args]); return Promise.resolve({ state: 'exited', code: 0 }); };
    const fakeWindow = await settleMicrotasks(runArmed(buildStopScript(9), invoke));
    assert.deepEqual(seen, [['harness_stop', { id: 9, deadlineMs: 5000 }]]);
    assert.deepEqual(fakeWindow.__vpS14.stop, { phase: 'resolved', outcome: { state: 'exited', code: 0 } });
  });
  await t.test('a custom deadline is embedded verbatim', async () => {
    const seen = [];
    await settleMicrotasks(runArmed(buildStopScript(9, 1000), (cmd, args) => { seen.push(args); return Promise.resolve({}); }));
    assert.equal(seen[0].deadlineMs, 1000);
  });
  await t.test('a rejection because the process already exited is tolerated: it settles, never throws', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildStopScript(9), () => Promise.reject({ code: 'unknown-process', message: 'no process with that id is known' })));
    assert.deepEqual(fakeWindow.__vpS14.stop, { phase: 'rejected', error: 'unknown-process: no process with that id is known' });
  });
});

