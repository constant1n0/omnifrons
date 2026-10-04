// `node --test` unit tests for `vp-s13-probe.mjs`'s pure helpers, no WebDriver
// (mirrors vp-s3-probe.test.mjs).

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { test } from 'node:test';

import {
  ATTACKS, TRAP_NAMES, buildArmScript, buildReadScript, classifyRejection,
  formatAttackLines, formatObservations, payloadDigest, summarize,
} from './vp-s13-probe.mjs';

const sha256Of = (text) => createHash('sha256').update(Buffer.from(text, 'utf8')).digest('hex');

const MALFORMED_CLASSES = ['wrong-type', 'missing-field', 'unknown-field', 'oversized', 'id-out-of-range'];
const PATH_ATTACK_CLASSES = ['traversal', 'absolute', 'nul-or-control', 'symlink-escape'];
const ALLOWED_COMMANDS = new Set(['guidance_status', 'guidance_preview', 'candidates_list', 'harness_observe']);

test('ATTACKS: every class present once, unique ids, allowed camelCase commands, oversized/symlink/id-range/unknown-field shapes', () => {
  const classes = new Set(ATTACKS.map((a) => a.class));
  const ids = new Set();
  for (const cls of [...MALFORMED_CLASSES, ...PATH_ATTACK_CLASSES]) assert.ok(classes.has(cls), `missing class ${cls}`);
  for (const attack of ATTACKS) {
    assert.ok(!ids.has(attack.id), `duplicate id ${attack.id}`);
    ids.add(attack.id);
    assert.ok(ALLOWED_COMMANDS.has(attack.command), `${attack.command} not allowed`);
    assert.ok(!('run_id' in attack.args), 'run_id must be camelCase runId');
  }
  const big = Object.values(ATTACKS.find((a) => a.class === 'oversized').args).find((v) => typeof v === 'string' && v.length > 1000);
  assert.ok(big && big.length >= 1024 * 1024);
  assert.equal(ATTACKS.find((a) => a.class === 'symlink-escape').args.file, TRAP_NAMES.guidanceFile);
  const idRange = ATTACKS.filter((a) => a.class === 'id-out-of-range');
  assert.equal(idRange.length, 2);
  for (const entry of idRange) assert.equal(entry.command, 'harness_observe');
  assert.ok(idRange.some((e) => e.args.id > 2 ** 53), 'above-2^53 id must exceed 2^53 itself, not merely MAX_SAFE_INTEGER (2^53 - 1)');
  assert.ok(idRange.some((e) => e.args.id < 0));
  const unknownField = ATTACKS.find((a) => a.class === 'unknown-field'); // spike-log.md:94-96
  assert.equal(unknownField.args.file, TRAP_NAMES.absentGuidanceFile);
  assert.equal(unknownField.baselineArgs.file, TRAP_NAMES.absentGuidanceFile);
  assert.ok('vpS13Unknown' in unknownField.args && !('vpS13Unknown' in unknownField.baselineArgs));
});

test('payloadDigest: SHA-256 and UTF-8 byte length of JSON.stringify(args), never the live JS string length', () => {
  const known = payloadDigest({});
  assert.equal(known.bytes, 2); // JSON.stringify({}) === '{}'
  // sha256sum of the literal 2-byte string "{}", computed independently of this module.
  assert.equal(known.sha256, '44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a');
  assert.deepEqual(payloadDigest({}), known, 'deterministic for the same input');

  // JSON.stringify('é') === '"é"': 3 JS chars, but 4 UTF-8 bytes since é encodes as 2 bytes.
  const nonAscii = payloadDigest('é');
  assert.equal(nonAscii.bytes, 4);
  assert.notEqual(nonAscii.bytes, '"é"'.length, 'byte length must differ from the JS string length for a non-ASCII payload');

  // Never throws, even where both serializations fail: a circular object with no prototype makes
  // JSON.stringify throw (circular) and String() throw (nothing to convert it with).
  const hostile = Object.create(null);
  hostile.self = hostile;
  assert.throws(() => JSON.stringify(hostile));
  assert.throws(() => String(hostile));
  // The fixed marker '[unserializable payload]' (24 bytes); sha256sum of that literal, computed
  // independently of this module, so a different fallback constant fails here.
  assert.deepEqual(payloadDigest(hostile), {
    bytes: 24, sha256: '89ab615c4f1d7fe299bac6a0132b1a0dbc4a67c4d274b5a68c6dfbcd4b33a789',
  });
});

test('classifyRejection: typed (ShellError kebab-case shape, or Tauri arg-deserialization shape) vs. untyped, never throws', () => {
  for (const message of [
    'guidance-file-invalid: the file name must be one path component with no separator', // dto.rs:1579-1581
    'invalid-request: the run id is not valid',
    'unknown-process: no process with that id is known',
    "invalid args `id` for command `harness_observe`: invalid type: string \"not-a-number\", expected u32", // tauri-2.11.5 src/error.rs:59-60
    "invalid args `kind` for command `guidance_status`: command guidance_status missing required key kind",
  ]) assert.equal(classifyRejection(message), 'typed', message);
  for (const message of ["TypeError: Cannot read properties of null (reading 'invoke')", '', 'pending', 'undefined']) {
    assert.equal(classifyRejection(message), 'untyped', message);
  }
  for (const value of [null, undefined, 42, {}]) assert.equal(classifyRejection(value), 'untyped');
});

const readFixture = (overrides = {}) => (
  { tauriInternalsPresent: true, workspaceCurrent: null, attacks: [], location_protocol: 'tauri:', ...overrides }
);
const rejected = (id, cls, command, message) => ({ id, class: cls, command, args: {}, outcome: `rejected:${message}`, response: null });
const resolved = (id, cls, command, response = null) => ({ id, class: cls, command, args: {}, outcome: 'resolved', response });
const fullyRejectedAttacks = () => ATTACKS.map((a, i) => rejected(a.id, a.class, a.command, `invalid-request: refused attack ${i}`));
const unknownFieldIndex = ATTACKS.findIndex((a) => a.class === 'unknown-field');

test('summarize', () => {
  const confined = summarize(readFixture({
    workspaceCurrent: { displayPath: '/home/user/trap-workspace', workArea: 'ok' }, attacks: fullyRejectedAttacks(),
  }), { expectedWorkspaceBasename: 'trap-workspace' });
  assert.deepEqual(confined, {
    location_scheme_is_app_protocol: true, workspace_selected: true, malformed_payloads_sent: true,
    path_attacks_sent: true, every_attack_rejected: true, every_rejection_typed: true,
    attack_accepted: false, canary_leaked: false,
  });

  // An accepted or pending attack is never counted as rejected.
  const accepted = fullyRejectedAttacks();
  accepted[0] = resolved(accepted[0].id, accepted[0].class, accepted[0].command);
  const acceptedSummary = summarize(readFixture({ attacks: accepted }), {});
  assert.equal(acceptedSummary.attack_accepted, true);
  assert.equal(acceptedSummary.every_attack_rejected, false);
  const pending = fullyRejectedAttacks();
  pending[0] = { ...pending[0], outcome: 'pending', response: null };
  assert.equal(summarize(readFixture({ attacks: pending }), {}).attack_accepted, false);
  assert.equal(summarize(readFixture({ attacks: pending }), {}).every_attack_rejected, false);

  // An untyped rejection (a crash, not a catalogue code) flips every_rejection_typed, still rejected.
  const crash = fullyRejectedAttacks();
  crash[0] = rejected(crash[0].id, crash[0].class, crash[0].command, 'TypeError: boom');
  const crashSummary = summarize(readFixture({ attacks: crash }), {});
  assert.equal(crashSummary.every_rejection_typed, false);
  assert.equal(crashSummary.every_attack_rejected, true);

  // unknown-field: identical -> tolerated; differing -> attack_accepted (BestEffort); pending baseline -> not sent.
  const pairedCase = (outcome, baselineOutcome) => ({ id: ATTACKS[unknownFieldIndex].id, class: 'unknown-field', outcome, baselineOutcome });
  const identical = fullyRejectedAttacks();
  identical[unknownFieldIndex] = pairedCase('resolved', 'resolved');
  const identicalSummary = summarize(readFixture({ attacks: identical }), {});
  assert.equal(identicalSummary.attack_accepted, false);
  assert.equal(identicalSummary.every_attack_rejected, true, 'excluded from the rejection-based facts');
  assert.equal(identicalSummary.malformed_payloads_sent, true);
  const differing = fullyRejectedAttacks();
  differing[unknownFieldIndex] = pairedCase('resolved', 'rejected:invalid-request: no such file');
  assert.equal(summarize(readFixture({ attacks: differing }), {}).attack_accepted, true);
  const differingResponse = fullyRejectedAttacks();
  differingResponse[unknownFieldIndex] = { ...pairedCase('resolved', 'resolved'), response: '{"a":1}', baselineResponse: '{"a":2}' };
  assert.equal(summarize(readFixture({ attacks: differingResponse }), {}).attack_accepted, true, 'both resolved, different bodies');
  // When both full bodies survive the retention bound, the comparison rests on them, not
  // on the capped response -- so equal capped prefixes with differing full bodies still flip the fact.
  const differingFull = fullyRejectedAttacks();
  differingFull[unknownFieldIndex] = {
    ...pairedCase('resolved', 'resolved'), response: 'same', baselineResponse: 'same',
    responseFull: 'full-a', baselineResponseFull: 'full-b',
  };
  assert.equal(summarize(readFixture({ attacks: differingFull }), {}).attack_accepted, true, 'equal capped prefixes, differing full bodies');
  const sameFull = fullyRejectedAttacks();
  sameFull[unknownFieldIndex] = {
    ...pairedCase('resolved', 'resolved'), response: 'same', baselineResponse: 'same',
    responseFull: 'full-a', baselineResponseFull: 'full-a',
  };
  const sameFullSummary = summarize(readFixture({ attacks: sameFull }), {});
  assert.equal(sameFullSummary.attack_accepted, false, 'both full bodies retained and equal: no effect');
  assert.equal(sameFullSummary.malformed_payloads_sent, true, 'and the pair is established');
  // A body dropped for exceeding the bound still carries its length: one side dropped and the other
  // retained, or both dropped at different lengths, means the bodies differ despite equal prefixes.
  const sameCapped = { ...pairedCase('resolved', 'resolved'), response: 'same', baselineResponse: 'same' };
  const oneDropped = fullyRejectedAttacks();
  oneDropped[unknownFieldIndex] = { ...sameCapped, responseFull: null, responseFullLength: 70_000, baselineResponseFull: 'same' };
  assert.equal(summarize(readFixture({ attacks: oneDropped }), {}).attack_accepted, true, 'one side over the bound, the other within it');
  const bothDroppedApart = fullyRejectedAttacks();
  bothDroppedApart[unknownFieldIndex] = {
    ...sameCapped, responseFull: null, responseFullLength: 70_000, baselineResponseFull: null, baselineResponseFullLength: 70_001,
  };
  assert.equal(summarize(readFixture({ attacks: bothDroppedApart }), {}).attack_accepted, true, 'both over the bound, different lengths');
  // Both dropped at the same length with equal prefixes cannot be compared: the pair is not
  // established (malformed_payloads_sent=false, so Uncertain), never read as "no effect".
  const bothDroppedEqual = fullyRejectedAttacks();
  bothDroppedEqual[unknownFieldIndex] = {
    ...sameCapped, responseFull: null, responseFullLength: 70_000, baselineResponseFull: null, baselineResponseFullLength: 70_000,
  };
  const undecidable = summarize(readFixture({ attacks: bothDroppedEqual }), {});
  assert.equal(undecidable.malformed_payloads_sent, false, 'an undecidable pair is not an established check');
  assert.equal(undecidable.attack_accepted, false, 'nor a demonstrated effect');
  const pendingBaseline = fullyRejectedAttacks();
  pendingBaseline[unknownFieldIndex] = pairedCase('resolved', 'pending');
  assert.equal(summarize(readFixture({ attacks: pendingBaseline }), {}).malformed_payloads_sent, false);

  // Missing a required malformed or path-attack class renders the matching *_sent fact false.
  const withoutOversized = fullyRejectedAttacks().filter((a) => a.class !== 'oversized');
  assert.equal(summarize(readFixture({ attacks: withoutOversized }), {}).malformed_payloads_sent, false);
  const withoutSymlink = fullyRejectedAttacks().filter((a) => a.class !== 'symlink-escape');
  assert.equal(summarize(readFixture({ attacks: withoutSymlink }), {}).path_attacks_sent, false);

  // canary_leaked is any recorded canaryHit, per attack or on the workspace_current read-back.
  const leaked = fullyRejectedAttacks();
  leaked[0] = { ...leaked[0], canaryHit: true };
  assert.equal(summarize(readFixture({ attacks: leaked }), {}).canary_leaked, true);
  assert.equal(summarize(readFixture({ attacks: fullyRejectedAttacks() }), {}).canary_leaked, false);
  assert.equal(summarize(readFixture({ workspaceCurrentCanaryHit: true }), {}).canary_leaked, true);

  // workspace_selected compares the displayPath basename, never a bare path-equality check.
  const read = readFixture({ workspaceCurrent: { displayPath: 'C:\\Users\\demo\\trap-workspace', workArea: 'ok' } });
  assert.equal(summarize(read, { expectedWorkspaceBasename: 'trap-workspace' }).workspace_selected, true);
  assert.equal(summarize(read, { expectedWorkspaceBasename: 'other' }).workspace_selected, false);
  assert.equal(summarize(readFixture({ workspaceCurrent: null }), { expectedWorkspaceBasename: 'trap-workspace' }).workspace_selected, false);

  // Empty/malformed input: every field false, except the two vacuous-on-empty facts.
  const empty = summarize({}, {});
  for (const key of Object.keys(empty)) {
    assert.equal(empty[key], key === 'every_attack_rejected' || key === 'every_rejection_typed', key);
  }
  assert.equal(summarize(null, undefined).location_scheme_is_app_protocol, false);
  assert.equal(summarize(null, undefined).workspace_selected, false);
});

test('formatObservations / formatAttackLines', () => {
  const lines = formatObservations(summarize(readFixture({ attacks: fullyRejectedAttacks() }), {}));
  const keys = lines.map((line) => line.slice('observation='.length).split(' value=')[0]);
  assert.deepEqual(keys, [
    'location_scheme_is_app_protocol', 'workspace_selected', 'malformed_payloads_sent',
    'path_attacks_sent', 'every_attack_rejected', 'every_rejection_typed', 'attack_accepted', 'canary_leaked',
  ]);

  const judgedArgs = { kind: 'guidance', file: '/etc/passwd' };
  const pairedArgs = { kind: 'guidance', file: 'x', vpS13Unknown: true };
  const pairedBaselineArgs = { kind: 'guidance', file: 'x' };
  const attacks = [
    { id: 'a-1', class: 'traversal', command: 'guidance_status', args: judgedArgs, outcome: 'rejected:guidance-file-invalid: nope' },
    {
      id: 'a-2', class: 'unknown-field', command: 'guidance_preview', args: pairedArgs, baselineArgs: pairedBaselineArgs,
      outcome: 'resolved', baselineOutcome: 'resolved', responseFull: '{"ok":true}', baselineResponseFull: '{"ok":true}',
    },
    {
      id: 'a-3', class: 'unknown-field', command: 'guidance_preview', args: pairedArgs, baselineArgs: pairedBaselineArgs,
      outcome: 'resolved', baselineOutcome: 'resolved', responseFull: null, baselineResponseFull: 'short-baseline-body',
    },
  ];
  const attackLines = formatAttackLines(attacks);
  const judgedPayload = payloadDigest(judgedArgs);
  assert.equal(attackLines[0],
    'observation=attack index=0 id="a-1" class="traversal" command="guidance_status" outcome="rejected:guidance-file-invalid: nope"'
    + ` payload_bytes=${judgedPayload.bytes} payload_sha256=${judgedPayload.sha256}`);

  const pairedPayload = payloadDigest(pairedArgs);
  const pairedBaselinePayload = payloadDigest(pairedBaselineArgs);
  assert.equal(attackLines[1],
    'observation=attack index=1 id="a-2" class="unknown-field" command="guidance_preview" outcome="resolved" baseline="resolved"'
    + ` payload_bytes=${pairedPayload.bytes} payload_sha256=${pairedPayload.sha256}`
    + ` baseline_payload_bytes=${pairedBaselinePayload.bytes} baseline_payload_sha256=${pairedBaselinePayload.sha256}`
    + ` response_sha256=${sha256Of('{"ok":true}')} baseline_response_sha256=${sha256Of('{"ok":true}')}`);

  // A full body dropped for exceeding the retention bound (responseFull: null) gives response_sha256=null;
  // its paired counterpart that survived still gets a real digest.
  assert.equal(attackLines[2],
    'observation=attack index=2 id="a-3" class="unknown-field" command="guidance_preview" outcome="resolved" baseline="resolved"'
    + ` payload_bytes=${pairedPayload.bytes} payload_sha256=${pairedPayload.sha256}`
    + ` baseline_payload_bytes=${pairedBaselinePayload.bytes} baseline_payload_sha256=${pairedBaselinePayload.sha256}`
    + ` response_sha256=null baseline_response_sha256=${sha256Of('short-baseline-body')}`);

  assert.deepEqual(formatAttackLines([]), []);
  const nullPayload = payloadDigest(undefined);
  assert.equal(formatAttackLines([null])[0],
    `observation=attack index=0 id=null class=null command=null outcome=null payload_bytes=${nullPayload.bytes} payload_sha256=${nullPayload.sha256}`);
});

function runArmed(script, invoke, workspaceValue = null) {
  const fakeWindow = {
    __TAURI_INTERNALS__: { invoke: (cmd, args) => (cmd === 'workspace_current' ? Promise.resolve(workspaceValue) : invoke(args)) },
  };
  new Function('window', script)(fakeWindow);
  return fakeWindow;
}
async function settleMicrotasks(fakeWindow) {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return fakeWindow;
}

test('buildArmScript', async (t) => {
  const tiny = [{ id: 'x-1', class: 'traversal', command: 'guidance_status', args: { kind: 'guidance', file: '../x' } }];

  await t.test('embeds the corpus and a quote-carrying canary safely (JSON-encoded); empty/absent canary is ""; guards __TAURI_INTERNALS__, caps at 512, reads workspace_current first', () => {
    const script = buildArmScript(tiny, { canary: 'ca"ry' });
    assert.match(script, /if \(window\.__TAURI_INTERNALS__\)/);
    assert.match(script, /"guidance_status"/);
    assert.match(script, /var CANARY = "ca\\"ry";/);
    assert.match(script, /try \{[\s\S]*window\.__TAURI_INTERNALS__\.invoke\(command, args\)[\s\S]*\} catch/);
    assert.match(script, /MAX_TEXT_LENGTH = 512/);
    assert.match(script, /\.slice\(0, MAX_TEXT_LENGTH\)/);
    const corpusIndex = script.indexOf('__vpS13 = {');
    assert.ok(corpusIndex >= 0 && script.indexOf("invoke('workspace_current')") > corpusIndex);
    assert.match(buildArmScript(tiny), /var CANARY = "";/);
  });
  await t.test("settles a rejection with the shell's {code, message} shape", async () => {
    const rejectIt = () => Promise.reject({ code: 'guidance-file-invalid', message: 'the file name must be one path component with no separator' });
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(tiny, {}), rejectIt));
    assert.equal(fakeWindow.__vpS13.attacks[0].outcome, 'rejected:guidance-file-invalid: the file name must be one path component with no separator');
  });
  await t.test('detects a canary beyond the 512-character cap on a resolved value, and on the workspace_current read-back', async () => {
    const script = buildArmScript(tiny, { canary: 'leak-me' });
    const padded = 'x'.repeat(600) + 'leak-me';
    const fakeWindow = await settleMicrotasks(runArmed(script, () => Promise.resolve({ content: padded }), { displayPath: '/leak-me/ws' }));
    assert.equal(fakeWindow.__vpS13.attacks[0].canaryHit, true);
    assert.ok(!fakeWindow.__vpS13.attacks[0].response.includes('leak-me'), 'the capped response must not itself carry the late canary');
    assert.equal(fakeWindow.__vpS13.workspaceCurrentCanaryHit, true);
  });
  await t.test('invokes and records both outcomes for a paired baselineArgs attack', async () => {
    const paired = [{
      id: 'u-1', class: 'unknown-field', command: 'guidance_preview',
      args: { kind: 'guidance', file: 'x', extra: true }, baselineArgs: { kind: 'guidance', file: 'x' },
    }];
    const seen = [];
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(paired, {}), (args) => { seen.push(args); return Promise.resolve('ok'); }));
    assert.equal(seen.length, 2);
    assert.equal(fakeWindow.__vpS13.attacks[0].outcome, 'resolved');
    assert.equal(fakeWindow.__vpS13.attacks[0].baselineOutcome, 'resolved');
  });
  await t.test('a paired attack retains the full uncapped response body within the retention bound (65,536 UTF-16 units), and null plus the original length above it', async () => {
    const paired = [{
      id: 'u-2', class: 'unknown-field', command: 'guidance_preview',
      args: { kind: 'guidance', file: 'x', extra: true }, baselineArgs: { kind: 'guidance', file: 'x' },
    }];
    // Resolved values are JSON-stringified before capping/retention (rawValue), same as `response`.
    const small = 'y'.repeat(600); // over the 512-char cap, well under the full-body bound
    const smallRaw = JSON.stringify(small);
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(paired, {}), () => Promise.resolve(small)));
    assert.equal(fakeWindow.__vpS13.attacks[0].responseFull, smallRaw);
    assert.equal(fakeWindow.__vpS13.attacks[0].baselineResponseFull, smallRaw);
    assert.equal(fakeWindow.__vpS13.attacks[0].response.length, 512, 'the capped field stays bounded even though the full body is retained separately');
    assert.equal(fakeWindow.__vpS13.attacks[0].responseFullLength, undefined, 'length is only recorded once the full body is dropped');

    const big = 'z'.repeat(64 * 1024 + 1);
    const bigRaw = JSON.stringify(big);
    const fakeWindowBig = await settleMicrotasks(runArmed(buildArmScript(paired, {}), () => Promise.resolve(big)));
    assert.equal(fakeWindowBig.__vpS13.attacks[0].responseFull, null);
    assert.equal(fakeWindowBig.__vpS13.attacks[0].responseFullLength, bigRaw.length);
    assert.equal(fakeWindowBig.__vpS13.attacks[0].baselineResponseFull, null);
    assert.equal(fakeWindowBig.__vpS13.attacks[0].baselineResponseFullLength, bigRaw.length);
  });
  await t.test('the retention bound is inclusive: a body of exactly 65,536 UTF-16 units is kept, 65,537 is dropped', async () => {
    const paired = [{
      id: 'u-5', class: 'unknown-field', command: 'guidance_preview',
      args: { kind: 'guidance', file: 'x', extra: true }, baselineArgs: { kind: 'guidance', file: 'x' },
    }];
    const atBound = 'z'.repeat(64 * 1024 - 2); // JSON.stringify adds two quotes: 65,536 units
    assert.equal(JSON.stringify(atBound).length, 64 * 1024);
    const kept = (await settleMicrotasks(runArmed(buildArmScript(paired, {}), () => Promise.resolve(atBound)))).__vpS13.attacks[0];
    assert.equal(kept.responseFull, JSON.stringify(atBound));
    assert.equal(kept.responseFullLength, undefined);
    const overBound = 'z'.repeat(64 * 1024 - 1); // 65,537 units once quoted
    const dropped = (await settleMicrotasks(runArmed(buildArmScript(paired, {}), () => Promise.resolve(overBound)))).__vpS13.attacks[0];
    assert.equal(dropped.responseFull, null);
    assert.equal(dropped.responseFullLength, 64 * 1024 + 1);
  });
  await t.test('a paired invoke resolving to undefined still retains a string body, never throwing in the page', async () => {
    const paired = [{
      id: 'u-3', class: 'unknown-field', command: 'guidance_preview',
      args: { kind: 'guidance', file: 'x', extra: true }, baselineArgs: { kind: 'guidance', file: 'x' },
    }];
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(paired, {}), () => Promise.resolve(undefined)));
    assert.equal(fakeWindow.__vpS13.attacks[0].responseFull, 'undefined');
    assert.equal(fakeWindow.__vpS13.attacks[0].baselineResponseFull, 'undefined');
    assert.equal(fakeWindow.__vpS13.attacks[0].response, 'undefined');
  });
  await t.test('a rejected paired side keeps a null full body and no length; the resolved side keeps its body', async () => {
    const paired = [{
      id: 'u-4', class: 'unknown-field', command: 'guidance_preview',
      args: { kind: 'guidance', file: 'x', extra: true }, baselineArgs: { kind: 'guidance', file: 'x' },
    }];
    const invoke = (args) => ('extra' in args ? Promise.reject('invalid-request: no') : Promise.resolve('ok'));
    const record = (await settleMicrotasks(runArmed(buildArmScript(paired, {}), invoke))).__vpS13.attacks[0];
    assert.equal(record.outcome, 'rejected:invalid-request: no');
    assert.equal(record.responseFull, null);
    assert.equal('responseFullLength' in record, false, 'a length means dropped for size, never rejected');
    assert.equal(record.baselineResponseFull, '"ok"');
  });
  await t.test('a non-paired attack never retains a full response body', async () => {
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(tiny, {}), () => Promise.resolve('ok')));
    assert.equal('responseFull' in fakeWindow.__vpS13.attacks[0], false);
  });
  await t.test("a synchronous throw from invoke settles as 'rejected:...', never an uncaught exception", async () => {
    const throwSync = () => { throw new Error('boom-sync'); };
    const fakeWindow = await settleMicrotasks(runArmed(buildArmScript(tiny, {}), throwSync));
    assert.equal(fakeWindow.__vpS13.attacks[0].outcome, 'rejected:Error: boom-sync');
  });
  await t.test('no __TAURI_INTERNALS__: attacks stay pending, tauriInternalsPresent is false, workspace_current is never invoked', async () => {
    const fakeWindow = {};
    new Function('window', buildArmScript(tiny, {}))(fakeWindow);
    await settleMicrotasks(fakeWindow);
    assert.equal(fakeWindow.__vpS13.tauriInternalsPresent, false);
    assert.equal(fakeWindow.__vpS13.attacks[0].outcome, 'pending');
    assert.equal(fakeWindow.__vpS13.workspaceCurrent, null);
  });
  await t.test('a rejected workspace_current carrying the canary in its error text sets workspaceCurrentCanaryHit', async () => {
    const fakeWindow = {
      __TAURI_INTERNALS__: {
        invoke: (cmd) => (cmd === 'workspace_current'
          ? Promise.reject({ code: 'invalid-request', message: 'path contains leak-me' })
          : Promise.resolve('ok')),
      },
    };
    new Function('window', buildArmScript(tiny, { canary: 'leak-me' }))(fakeWindow);
    await settleMicrotasks(fakeWindow);
    assert.equal(fakeWindow.__vpS13.workspaceCurrent, null);
    assert.equal(fakeWindow.__vpS13.workspaceCurrentCanaryHit, true);
  });
});

test('buildReadScript returns an empty snapshot, never throws, when the probe was never armed', () => {
  const script = buildReadScript();
  assert.match(script, /window\.__vpS13 \? JSON\.parse\(JSON\.stringify\(window\.__vpS13\)\) : \{\}/);
  assert.deepEqual(new Function('window', 'location', script)({}, { protocol: 'tauri:' }), { location_protocol: 'tauri:' });
});
