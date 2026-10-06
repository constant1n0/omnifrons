// `node --test` unit tests for `vp-s14-observations.mjs`'s pure derive/
// formatting helpers (`summarize`, `formatObservations`, `formatCaseLines`,
// `formatRecordLines`), no WebDriver. The in-page script builders
// (`buildPickArmScript` and friends) have their own module and their own
// `vp-s14-probe.test.mjs`; the fixtures below are plain data shaped the
// way a driver's captured snapshots would be, never built by importing
// that sibling module.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  CASE_LABELS, formatCaseLines, formatDuplicateLines, formatObservations, formatRecordLines, summarize,
} from './vp-s14-observations.mjs';

const approval = (approvalId, canonicalPath, size, sha256, status = 'active') => (
  { approvalId, evidence: { canonicalPath, size, sha256 }, approvedAt: 0, status, revokedAt: null }
);
const resolvedCase = (processId) => ({ outcome: 'resolved', processId, code: null });
const rejectedCase = (code, message = 'denied', detail = null) => ({ outcome: 'rejected', code, message, detail });
const pendingCase = () => ({ outcome: 'pending', code: null });

const ORIGINAL_ID = 'aaaaaaaaaaaaaaaa';
const REAPPROVAL_ID = 'bbbbbbbbbbbbbbbb';
const ORIGINAL = { approvalId: ORIGINAL_ID, evidence: { canonicalPath: '/scratch/true-copy', size: 100, sha256: 'sha-true' } };
const REAPPROVAL = { approvalId: REAPPROVAL_ID, evidence: { canonicalPath: '/scratch/true-copy2', size: 100, sha256: 'sha-true' } };
const BEFORE_LIST = [approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true')];
const AFTER_LIST_PASS = [
  approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
  approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true'),
];
const allGoodInput = () => ({
  beforeList: BEFORE_LIST, afterList: AFTER_LIST_PASS, original: ORIGINAL, reapproval: REAPPROVAL,
  cases: {
    a: resolvedCase(100),
    b: rejectedCase('changed-since-approval', "the executable's content has changed since it was approved", { recordedSha256Short: 'aa', observedSha256Short: 'bb' }),
    c: rejectedCase('shadowed-path', 'the approved path now resolves somewhere else'),
    d: resolvedCase(104),
  },
  location_protocol: 'tauri:',
});

test('summarize: all-good input gives every field true except the three failure fields', () => {
  const summary = summarize(allGoodInput());
  const failureFields = ['changed_launch_allowed', 'shadow_launch_allowed', 'record_silently_rebound'];
  for (const [key, value] of Object.entries(summary)) {
    assert.equal(value, !failureFields.includes(key), key);
  }
});

test('summarize: approval_recorded requires both canonicalPath and sha256 to match what the approve call returned, not canonicalPath alone', () => {
  const input = allGoodInput();
  input.beforeList = [approval(ORIGINAL_ID, ORIGINAL.evidence.canonicalPath, ORIGINAL.evidence.size, 'sha-different-from-approve-call')];
  assert.equal(summarize(input).approval_recorded, false);
});

test('summarize: approval_recorded compares the full triple, so a size-only mismatch is not recorded', () => {
  const input = allGoodInput();
  input.beforeList = [approval(ORIGINAL_ID, ORIGINAL.evidence.canonicalPath, ORIGINAL.evidence.size + 1, ORIGINAL.evidence.sha256)];
  assert.equal(summarize(input).approval_recorded, false);
});

test('summarize: each single changed component of the original identity (path, size, or sha256 alone) is a rebind', () => {
  const { canonicalPath, size, sha256 } = ORIGINAL.evidence;
  for (const [label, changed] of [
    ['path', approval(ORIGINAL_ID, '/scratch/elsewhere', size, sha256)],
    ['size', approval(ORIGINAL_ID, canonicalPath, size + 1, sha256)],
    ['sha256', approval(ORIGINAL_ID, canonicalPath, size, 'sha-changed')],
  ]) {
    const input = allGoodInput();
    input.afterList = [changed, approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
    const summary = summarize(input);
    assert.equal(summary.record_silently_rebound, true, label);
    assert.equal(summary.original_record_unchanged, false, label);
  }
});

test('summarize: with no established original identity, a re-approval has nothing to differ from and is not reapproval_launched', () => {
  const input = allGoodInput();
  input.original = { approvalId: ORIGINAL_ID };
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: an empty or partial evidence object is no established identity, so nothing is recorded, unchanged, or rebound', () => {
  for (const [label, evidence] of [
    ['empty', {}],
    ['no size', { canonicalPath: '/scratch/true-copy', sha256: 'sha-true' }],
    ['no sha256', { canonicalPath: '/scratch/true-copy', size: 100 }],
    ['no path', { size: 100, sha256: 'sha-true' }],
  ]) {
    const hollow = { approvalId: ORIGINAL_ID, evidence, approvedAt: 0, status: 'active', revokedAt: null };
    const input = allGoodInput();
    input.original = { approvalId: ORIGINAL_ID, evidence };
    input.beforeList = [hollow];
    input.afterList = [hollow, approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
    const summary = summarize(input);
    assert.equal(summary.approval_recorded, false, label);
    assert.equal(summary.original_record_unchanged, false, label);
    assert.equal(summary.record_silently_rebound, false, `${label}: a hollow snapshot is unproven, never a rebind`);
    assert.equal(summary.reapproval_launched, false, `${label}: nothing established to differ from`);
  }
});

test('summarize: a before entry with status other than active is not approval_recorded, even with a matching identity', () => {
  const input = allGoodInput();
  input.beforeList = [approval(ORIGINAL_ID, ORIGINAL.evidence.canonicalPath, ORIGINAL.evidence.size, ORIGINAL.evidence.sha256, 'revoked')];
  assert.equal(summarize(input).approval_recorded, false);
});

test('summarize: a before or after entry with status other than active is not original_record_unchanged, even with a matching identity', () => {
  const { canonicalPath, size, sha256 } = ORIGINAL.evidence;
  for (const [label, before, after] of [
    ['before revoked', approval(ORIGINAL_ID, canonicalPath, size, sha256, 'revoked'), approval(ORIGINAL_ID, canonicalPath, size, sha256)],
    ['after revoked', approval(ORIGINAL_ID, canonicalPath, size, sha256), approval(ORIGINAL_ID, canonicalPath, size, sha256, 'revoked')],
  ]) {
    const input = allGoodInput();
    input.beforeList = [before];
    input.afterList = [after, approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
    assert.equal(summarize(input).original_record_unchanged, false, label);
  }
});

test('summarize: a revocation (status changes, identity does not) is not record_silently_rebound -- identity-only, by design', () => {
  const { canonicalPath, size, sha256 } = ORIGINAL.evidence;
  const input = allGoodInput();
  input.beforeList = [approval(ORIGINAL_ID, canonicalPath, size, sha256)];
  input.afterList = [approval(ORIGINAL_ID, canonicalPath, size, sha256, 'revoked'), approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
  const summary = summarize(input);
  assert.equal(summary.record_silently_rebound, false);
  assert.equal(summary.original_record_unchanged, false, 'the status change is still a change, just not a rebind');
});

test('summarize: an after-list entry without evidence is unproven, never record_silently_rebound', () => {
  const input = allGoodInput();
  input.afterList = [{ approvalId: ORIGINAL_ID, approvedAt: 0, status: 'active', revokedAt: null }, approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
  const summary = summarize(input);
  assert.equal(summary.record_silently_rebound, false);
  assert.equal(summary.original_record_unchanged, false);
});

test('summarize: case (b) resolved (launched instead of denied) sets changed_launch_allowed, not rewrite_denied_changed', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, b: resolvedCase(105) };
  const summary = summarize(input);
  assert.equal(summary.changed_launch_allowed, true);
  assert.equal(summary.rewrite_denied_changed, false);
  assert.equal(summary.rewrite_attempted, true, 'resolved still counts as settled');
});

test('summarize: case (c) resolved sets shadow_launch_allowed, not shadow_denied_shadowed', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, c: resolvedCase(106) };
  const summary = summarize(input);
  assert.equal(summary.shadow_launch_allowed, true);
  assert.equal(summary.shadow_denied_shadowed, false);
});

test('summarize: the original id present in afterList with a changed identity sets record_silently_rebound, not original_record_unchanged', () => {
  const input = allGoodInput();
  input.afterList = [approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'), approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')];
  const summary = summarize(input);
  assert.equal(summary.record_silently_rebound, true);
  assert.equal(summary.original_record_unchanged, false);
});

test('summarize: a typed-but-different denial code sets neither *_denied_* nor *_launch_allowed', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, b: rejectedCase('unapproved', 'this executable has not been approved') };
  const summaryB = summarize(input);
  assert.equal(summaryB.rewrite_denied_changed, false);
  assert.equal(summaryB.changed_launch_allowed, false);
  assert.equal(summaryB.rewrite_attempted, true);

  const inputC = allGoodInput();
  inputC.cases = { ...inputC.cases, c: rejectedCase('unapproved', 'this executable has not been approved') };
  const summaryC = summarize(inputC);
  assert.equal(summaryC.shadow_denied_shadowed, false);
  assert.equal(summaryC.shadow_launch_allowed, false);
});

test('summarize: an untyped (no code) rejection also sets neither *_denied_* nor *_launch_allowed', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, b: rejectedCase(null, 'TypeError: boom') };
  const summary = summarize(input);
  assert.equal(summary.rewrite_denied_changed, false);
  assert.equal(summary.changed_launch_allowed, false);
  assert.equal(summary.rewrite_attempted, true);
});

test('summarize: a duplicated original id in beforeList is unproven -- not approval_recorded', () => {
  const input = allGoodInput();
  const { canonicalPath, size, sha256 } = ORIGINAL.evidence;
  input.beforeList = [approval(ORIGINAL_ID, canonicalPath, size, sha256), approval(ORIGINAL_ID, canonicalPath, size, sha256)];
  assert.equal(summarize(input).approval_recorded, false);
});

test('summarize: a duplicated original id in afterList is unproven -- neither original_record_unchanged nor record_silently_rebound, even when one copy is rebound', () => {
  const input = allGoodInput();
  const { canonicalPath, size, sha256 } = ORIGINAL.evidence;
  // Two entries share the original id: one unchanged, one rebound to a
  // different file. Which is "the" record is itself ambiguous, so neither
  // positive fact (unchanged) nor the failure fact (rebound) may fire.
  input.afterList = [
    approval(ORIGINAL_ID, canonicalPath, size, sha256),
    approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'),
    approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true'),
  ];
  const summary = summarize(input);
  assert.equal(summary.original_record_unchanged, false);
  assert.equal(summary.record_silently_rebound, false);
});

test('summarize: a missing record in afterList gives neither original_record_unchanged nor record_silently_rebound', () => {
  const input = allGoodInput();
  input.afterList = [approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true')]; // original id absent
  const summary = summarize(input);
  assert.equal(summary.original_record_unchanged, false);
  assert.equal(summary.record_silently_rebound, false);
});

test('summarize: a re-approval reusing the original id does not count as reapproval_launched', () => {
  const input = allGoodInput();
  input.reapproval = { approvalId: ORIGINAL_ID, evidence: ORIGINAL.evidence };
  const summary = summarize(input);
  assert.equal(summary.reapproval_launched, false);
});

test('summarize: a re-approval with the same identity as the original does not count as reapproval_launched', () => {
  const input = allGoodInput();
  input.reapproval = { approvalId: REAPPROVAL_ID, evidence: ORIGINAL.evidence };
  input.afterList = [approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'), approval(REAPPROVAL_ID, '/scratch/true-copy', 100, 'sha-true')];
  const summary = summarize(input);
  assert.equal(summary.reapproval_launched, false);
});

test('summarize: a re-approval record that is already revoked in afterList is not reapproval_launched', () => {
  const input = allGoodInput();
  input.afterList = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true', 'revoked'),
  ];
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: a process id above u32 (4294967296) is not a launched process', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, a: resolvedCase(4294967296) };
  assert.equal(summarize(input).unchanged_launch_allowed, false);
  input.cases = { ...input.cases, a: resolvedCase(4294967295) };
  assert.equal(summarize(input).unchanged_launch_allowed, true, 'the u32 maximum itself is valid');
});

test('summarize: a re-approval entry with hollow evidence does not count as reapproval_launched, though a null identity never equals the original', () => {
  const input = allGoodInput();
  // The after-list entry for the re-approval id carries no evidence at all:
  // `identityOf` reads it as no established identity, not as "different".
  input.afterList = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    { approvalId: REAPPROVAL_ID, approvedAt: 0, status: 'active', revokedAt: null },
  ];
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: a re-approval entry with partial evidence (missing sha256) does not count as reapproval_launched', () => {
  const input = allGoodInput();
  input.afterList = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    { approvalId: REAPPROVAL_ID, evidence: { canonicalPath: '/scratch/true-copy2', size: 100 }, approvedAt: 0, status: 'active', revokedAt: null },
  ];
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: a re-approval call whose own identity does not match its after-list record does not count as reapproval_launched', () => {
  const input = allGoodInput();
  // `reapproval` (the approve-call result) claims one identity; the record
  // actually written to the list carries a different one. The record must
  // carry what was approved, mirroring approval_recorded's own check.
  input.reapproval = { approvalId: REAPPROVAL_ID, evidence: { canonicalPath: '/scratch/not-what-was-approved', size: 1, sha256: 'sha-mismatch' } };
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: unsettled cases (d still pending) do not count as reapproval_launched even with a differing reapproval identity', () => {
  const input = allGoodInput();
  input.cases = { ...input.cases, d: pendingCase() };
  assert.equal(summarize(input).reapproval_launched, false);
});

test('summarize: a resolved case with a processId that is NaN, Infinity, negative, fractional, a string, or null is not unchanged_launch_allowed', () => {
  for (const [label, badProcessId] of [
    ['NaN', NaN], ['Infinity', Infinity], ['negative', -1], ['fractional', 1.5], ['string', '100'], ['null', null],
  ]) {
    const input = allGoodInput();
    input.cases = { ...input.cases, a: { outcome: 'resolved', processId: badProcessId, code: null } };
    assert.equal(summarize(input).unchanged_launch_allowed, false, label);
  }
});

test('summarize: case (d) resolved with a processId that is NaN, negative, or fractional is not reapproval_launched', () => {
  for (const [label, badProcessId] of [['NaN', NaN], ['negative', -1], ['fractional', 1.5]]) {
    const input = allGoodInput();
    input.cases = { ...input.cases, d: { outcome: 'resolved', processId: badProcessId, code: null } };
    assert.equal(summarize(input).reapproval_launched, false, label);
  }
});

test('summarize: location_scheme_is_app_protocol is strictly tauri:, never e.g. https:', () => {
  assert.equal(summarize({ ...allGoodInput(), location_protocol: 'https:' }).location_scheme_is_app_protocol, false);
});

test('summarize: empty/malformed input never throws; every field reads false', () => {
  for (const input of [undefined, null, {}, { cases: null }, { beforeList: 'not-a-list', cases: { a: null } }]) {
    const summary = summarize(input);
    for (const [key, value] of Object.entries(summary)) assert.equal(value, false, `${key} for ${JSON.stringify(input)}`);
  }
});

test('formatObservations: stable declared order, mirroring ExecutableIdentityObservations minus the two gating fields', () => {
  const lines = formatObservations(summarize(allGoodInput()));
  const keys = lines.map((line) => line.slice('observation='.length).split(' value=')[0]);
  assert.deepEqual(keys, [
    'location_scheme_is_app_protocol', 'approval_recorded', 'unchanged_launch_allowed',
    'rewrite_attempted', 'rewrite_denied_changed', 'shadow_attempted', 'shadow_denied_shadowed',
    'original_record_unchanged', 'reapproval_launched', 'changed_launch_allowed',
    'shadow_launch_allowed', 'record_silently_rebound',
  ]);
  assert.equal(lines[0], 'observation=location_scheme_is_app_protocol value=true');
});

test('formatCaseLines: fixed label order, exact strings, process_id only on a resolved case, pending default for a missing label', () => {
  const cases = {
    a: { outcome: 'resolved', code: null, processId: 42 },
    b: { outcome: 'rejected', code: 'changed-since-approval' },
    // c intentionally missing
    d: { outcome: 'resolved', code: null }, // resolved but no numeric processId: no process_id suffix
  };
  const lines = formatCaseLines(cases);
  assert.equal(CASE_LABELS.length, 4);
  assert.deepEqual(lines, [
    'observation=case label="a" outcome="resolved" code=null process_id=42',
    'observation=case label="b" outcome="rejected" code="changed-since-approval"',
    'observation=case label="c" outcome="pending" code=null',
    'observation=case label="d" outcome="resolved" code=null',
  ]);
});

test('formatCaseLines: a resolved case with a processId that is NaN, negative, fractional, or a string omits process_id entirely', () => {
  for (const [label, badProcessId] of [['NaN', NaN], ['negative', -1], ['fractional', 1.5], ['string', '42']]) {
    const lines = formatCaseLines({ a: { outcome: 'resolved', code: null, processId: badProcessId } });
    assert.equal(lines[0], 'observation=case label="a" outcome="resolved" code=null', label);
  }
});

test('formatRecordLines: only basename ever appears, full sha256, one line per (id, phase) actually present, ids deduplicated', () => {
  const beforeList = [approval(ORIGINAL_ID, '/scratch/vp-s14-xyz123/true-copy', 100, 'sha-true')];
  const afterList = [
    approval(ORIGINAL_ID, '/scratch/vp-s14-xyz123/true-copy', 100, 'sha-true'),
    approval(REAPPROVAL_ID, '/scratch/vp-s14-xyz123/true-copy2', 100, 'sha-true'),
  ];
  const lines = formatRecordLines(beforeList, afterList, [ORIGINAL_ID, REAPPROVAL_ID, ORIGINAL_ID]);
  assert.deepEqual(lines, [
    'observation=record phase="before" approval_id="aaaaaaaaaaaaaaaa" basename="true-copy" size=100 sha256="sha-true" status="active"',
    'observation=record phase="after" approval_id="aaaaaaaaaaaaaaaa" basename="true-copy" size=100 sha256="sha-true" status="active"',
    'observation=record phase="after" approval_id="bbbbbbbbbbbbbbbb" basename="true-copy2" size=100 sha256="sha-true" status="active"',
  ]);
  for (const line of lines) {
    assert.ok(!line.includes('/scratch'), 'no full path, ever');
    assert.ok(!line.includes('vp-s14-xyz123'), 'no scratch-directory name, ever');
  }
});

test('formatRecordLines: an id absent from a phase emits no line for that phase; empty/malformed inputs never throw', () => {
  const beforeList = [approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true')];
  assert.deepEqual(formatRecordLines(beforeList, [], [ORIGINAL_ID]), [
    'observation=record phase="before" approval_id="aaaaaaaaaaaaaaaa" basename="true-copy" size=100 sha256="sha-true" status="active"',
  ]);
  assert.deepEqual(formatRecordLines(null, undefined, null), []);
  assert.deepEqual(formatRecordLines(beforeList, [], ['not-on-record']), []);
});

test('formatRecordLines: a duplicated id in a phase emits no line for that phase -- ambiguous, not fabricated', () => {
  const duplicated = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'),
  ];
  assert.deepEqual(formatRecordLines(duplicated, [], [ORIGINAL_ID]), []);
});

test('formatDuplicateLines: a duplicated id in beforeList emits one record-duplicate line for that phase, with the exact count', () => {
  const duplicated = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'),
  ];
  assert.deepEqual(formatDuplicateLines(duplicated, [], [ORIGINAL_ID]), [
    'observation=record-duplicate phase="before" approval_id="aaaaaaaaaaaaaaaa" count=2',
  ]);
});

test('formatDuplicateLines: a duplicated id in afterList (the re-approval id, not just the original) emits a line for that phase', () => {
  const duplicated = [
    approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true'),
    approval(REAPPROVAL_ID, '/scratch/other-file', 999, 'sha-different'),
  ];
  assert.deepEqual(formatDuplicateLines([], duplicated, [ORIGINAL_ID, REAPPROVAL_ID]), [
    'observation=record-duplicate phase="after" approval_id="bbbbbbbbbbbbbbbb" count=2',
  ]);
});

test('formatDuplicateLines: three copies of the same id report count=3, not just "duplicated"', () => {
  const triplicated = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'),
    approval(ORIGINAL_ID, '/scratch/third-file', 1, 'sha-third'),
  ];
  assert.deepEqual(formatDuplicateLines(triplicated, [], [ORIGINAL_ID]), [
    'observation=record-duplicate phase="before" approval_id="aaaaaaaaaaaaaaaa" count=3',
  ]);
});

test('formatDuplicateLines: no duplicates in either phase emits nothing', () => {
  const beforeList = [approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true')];
  const afterList = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(REAPPROVAL_ID, '/scratch/true-copy2', 100, 'sha-true'),
  ];
  assert.deepEqual(formatDuplicateLines(beforeList, afterList, [ORIGINAL_ID, REAPPROVAL_ID]), []);
});

test('formatDuplicateLines: ids are deduplicated before checking, so passing the same id twice never doubles its line', () => {
  const duplicated = [
    approval(ORIGINAL_ID, '/scratch/true-copy', 100, 'sha-true'),
    approval(ORIGINAL_ID, '/scratch/other-file', 999, 'sha-different'),
  ];
  assert.deepEqual(formatDuplicateLines(duplicated, [], [ORIGINAL_ID, ORIGINAL_ID]), [
    'observation=record-duplicate phase="before" approval_id="aaaaaaaaaaaaaaaa" count=2',
  ]);
});

test('formatDuplicateLines: empty/malformed inputs never throw and report no duplicates', () => {
  assert.deepEqual(formatDuplicateLines(null, undefined, null), []);
  assert.deepEqual(formatDuplicateLines('not-a-list', 'not-a-list', [ORIGINAL_ID]), []);
  assert.deepEqual(formatDuplicateLines([], [], ['not-on-record']), []);
});
