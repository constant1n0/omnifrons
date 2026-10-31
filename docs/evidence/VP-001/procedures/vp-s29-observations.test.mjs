// `node --test` coverage for `vp-s29-observations.mjs`: per-leg derivation
// from hand-written transcripts, one per outcome. The transcripts are plain
// strings shaped like what each adapter retains (stream-json lines for
// `claude-code` and the Codex bridge, normalized terminal text for `pty-cli`).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  EXPECTED_VERSIONS, SWITCH_LEG, findSecrets, formatRecord, observeHarnessLogs, observeLeg, overallResult, parseHarnessLog, redact,
} from './vp-s29-observations.mjs';

const OPTIONS = { definitionName: 'Orbit Scribe', skillName: 'release-notes', home: '/home/op' };
const json = (value) => JSON.stringify(value);
const lines = (...parts) => `${parts.join('\n')}\n`;
const init = json({ type: 'system', subtype: 'init', cwd: '/work/definition', model: 'm', session_id: 's' });
const bridge = json({ type: 'system', subtype: 'bridge', codex_version: 'codex-cli 0.163.0', bridge_version: '0.1.0' });
const say = (text) => json({ type: 'assistant', message: { content: [{ type: 'text', text }] } });
const tool = (name, input) => json({ type: 'assistant', message: { content: [{ type: 'tool_use', name, input }] } });
const CLAUDE_VERSION = '2.1.316 (Claude Code)\n';
const CODEX_VERSION = 'codex-cli 0.163.0\n';
const GEMINI_VERSION = '0.41.2\n';

const claudeBase = (extra) => lines(
  init,
  tool('Read', { file_path: '/work/definition/AGENTS.md' }),
  say('I am Orbit Scribe.'),
  ...extra,
);

test('claude-code passes with loaded-file evidence and only the indexed skill path', () => {
  const result = observeLeg({
    adapter: 'claude-code',
    transcript: claudeBase([tool('Read', { file_path: '/work/definition/skills/release-notes/SKILL.md' })]),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'pass');
  assert.equal(result.harness.name, 'Claude Code');
  assert.equal(result.harness.version, '2.1.316');
  assert.equal(result.harness.versionMatchesBinding, true);
  assert.equal(result.loadedFileEvidence.found, true);
  assert.equal(result.skillPaths.indexed, true);
  assert.equal(result.skillPaths.userLevel, false);
});

test('a user-level skill path in the transcript fails the leg', () => {
  const result = observeLeg({
    adapter: 'claude-code',
    transcript: claudeBase([
      tool('Read', { file_path: '/home/op/.claude/skills/release-notes/SKILL.md' }),
      tool('Read', { file_path: '/work/definition/skills/release-notes/SKILL.md' }),
    ]),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'fail');
  assert.equal(result.skillPaths.userLevel, true);
  assert.ok(result.reasons.some((reason) => /user-level/.test(reason)));
  assert.ok(result.skillPaths.excerpts.every((line) => !line.includes('/home/op')), 'HOME is redacted');
});

test('the user-level marker in the output fails the leg even without a path', () => {
  const result = observeLeg({
    adapter: 'claude-code',
    transcript: claudeBase([say('Applied USER-LEVEL-SKILL-MARKER style.')]),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'fail');
});

test('claude-code with no skill path is uncertain', () => {
  const result = observeLeg({ adapter: 'claude-code', transcript: claudeBase([]), versionText: CLAUDE_VERSION, ...OPTIONS });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.skillPaths.indexed, false);
  assert.ok(result.reasons.some((reason) => /skill path/.test(reason)));
});

test('claude-code with init but no line naming CLAUDE.md or AGENTS.md is uncertain', () => {
  const result = observeLeg({
    adapter: 'claude-code',
    transcript: lines(init, say('I am Orbit Scribe.'), tool('Read', { file_path: 'skills/release-notes/SKILL.md' })),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.loadedFileEvidence.found, false);
});

test('a version other than the binding is uncertain, never pass', () => {
  const result = observeLeg({
    adapter: 'claude-code',
    transcript: claudeBase([tool('Read', { file_path: 'skills/release-notes/SKILL.md' })]),
    versionText: '2.2.0 (Claude Code)\n',
    ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.harness.versionMatchesBinding, false);
  assert.equal(result.harness.expectedVersion, EXPECTED_VERSIONS['claude-code']);
});

test('stream-json-cli through the bridge passes when the transcript names AGENTS.md', () => {
  const result = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(
      init,
      bridge,
      tool('command_execution', { command: 'cat AGENTS.md' }),
      tool('command_execution', { command: 'cat skills/release-notes/SKILL.md' }),
      say('Orbit Scribe here.'),
      json({ type: 'result', subtype: 'success' }),
    ),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'pass');
  assert.equal(result.harness.name, 'Codex CLI through codex-stream-bridge');
  assert.equal(result.harness.bridge.codexVersion, '0.163.0');
  assert.equal(result.harness.bridge.bridgeVersion, '0.1.0');
});

test('the bridge with only init is uncertain', () => {
  const result = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(init, bridge, say('Orbit Scribe here.'), tool('command_execution', { command: 'cat skills/release-notes/SKILL.md' })),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.loadedFileEvidence.found, false);
});

test('the bridge without its retained version line is uncertain', () => {
  const result = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(init, tool('command_execution', { command: 'cat AGENTS.md skills/release-notes/SKILL.md' })),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.harness.bridge, null);
  assert.ok(result.reasons.some((reason) => /bridge/.test(reason)));
});

test('pty-cli passes on a normalized line naming GEMINI.md and the indexed path', () => {
  const result = observeLeg({
    adapter: 'pty-cli',
    transcript: '\u001b[2J\u001b[1;32mUsing: 1 GEMINI.md file\u001b[0m\r\nReadFile skills/release-notes/SKILL.md\r\n',
    versionText: GEMINI_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'pass');
  assert.equal(result.harness.name, 'Gemini CLI');
  assert.equal(result.harness.version, '0.41.2');
});

test('pty-cli does not accept the model saying who it is as loaded-file evidence', () => {
  const result = observeLeg({
    adapter: 'pty-cli',
    transcript: 'I am Orbit Scribe.\nReadFile skills/release-notes/SKILL.md\n',
    versionText: GEMINI_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.loadedFileEvidence.found, false);
  assert.equal(result.skillPaths.indexed, true);
});

test('pty-cli needs the CLI notice that counts GEMINI.md files, not any mention', () => {
  const notice = (line) => observeLeg({
    adapter: 'pty-cli', transcript: `${line}\nReadFile skills/release-notes/SKILL.md\n`, versionText: GEMINI_VERSION, ...OPTIONS,
  });
  assert.equal(notice('Loaded 1 GEMINI.md file').result, 'pass');
  assert.equal(notice('1 context file (GEMINI.md)').result, 'pass');
  assert.equal(notice('I read GEMINI.md earlier').result, 'uncertain');
  assert.equal(notice('Loaded GEMINI.md').result, 'uncertain');
});

test('pty-cli with neither evidence nor a skill path is uncertain; an empty transcript too', () => {
  const none = observeLeg({ adapter: 'pty-cli', transcript: 'hello\n', versionText: GEMINI_VERSION, ...OPTIONS });
  assert.equal(none.result, 'uncertain');
  const empty = observeLeg({ adapter: 'pty-cli', transcript: '', versionText: GEMINI_VERSION, ...OPTIONS });
  assert.equal(empty.result, 'uncertain');
  assert.ok(empty.reasons.some((reason) => /empty/.test(reason)));
});

test('an unknown adapter is rejected', () => {
  assert.throws(() => observeLeg({ adapter: 'other', transcript: 'x', versionText: '1.0.0', ...OPTIONS }), /unknown adapter/);
});

test('the switch leg is a constant uncertain', () => {
  assert.deepEqual(SWITCH_LEG, { result: 'uncertain', reason: 'no checkpoint-and-restart flow exists' });
});

const leg = (adapter, result, reasons = []) => ({
  adapter,
  harness: { name: adapter, version: '1.0.0', expectedVersion: '1.0.0', versionMatchesBinding: true, bridge: null },
  loadedFileEvidence: { found: result === 'pass', excerpts: [] },
  skillPaths: { indexed: result === 'pass', userLevel: result === 'fail', excerpts: [] },
  result,
  reasons,
});

test('the overall result is uncertain while the switch leg is, fail on any failed leg or changed digest', () => {
  const legs = { 'claude-code': leg('claude-code', 'pass'), 'stream-json-cli': leg('stream-json-cli', 'pass'), 'pty-cli': leg('pty-cli', 'pass') };
  assert.equal(overallResult(legs, true), 'uncertain');
  assert.equal(overallResult(legs, false), 'fail');
  assert.equal(overallResult({ ...legs, 'pty-cli': leg('pty-cli', 'fail') }, true), 'fail');
  assert.equal(overallResult({ ...legs, 'pty-cli': null }, true), 'uncertain');
});

test('formatRecord renders one two-column table with every leg, the switch leg and placeholders', () => {
  const text = formatRecord({
    runId: 'run-1',
    recordId: null,
    baselineId: 'VP-001-BASE-01',
    buildChannel: null,
    buildChannelDigest: 'b'.repeat(64),
    exercisedArtifactDigest: 'b'.repeat(64),
    observedState: null,
    runDate: '2026-11-01',
    operatorRole: 'maintainer, local operator run',
    legs: {
      'claude-code': leg('claude-code', 'pass'),
      'stream-json-cli': leg('stream-json-cli', 'uncertain', ['the transcript names no skill path | x']),
      'pty-cli': null,
    },
    digestsUnchanged: true,
    evidencePath: 'docs/evidence/VP-001/artifacts/vp-s29/run-1/evidence-summary.txt',
    evidenceDigest: 'a'.repeat(64),
  });
  assert.match(text, /^## VP-001 VP-S29 scenario -- local operator run run-1 \(2026-11-01\)\n/);
  assert.match(text, /\| `scenario_id` \| VP-S29 \|/);
  assert.match(text, /\| `result` \| uncertain \|/);
  assert.match(text, /\| `baseline_id` \| VP-001-BASE-01 \|/);
  assert.match(text, /\| `record_id` \| <operator: [^>]+> \|/);
  assert.match(text, /\| `build_channel` \| packaged-ci \|/);
  assert.match(text, /\| `observed_state` \| uncertain \|/);
  assert.match(text, new RegExp(`\\| \`build_channel_digest\` \\| ${'b'.repeat(64)} \\|`));
  assert.match(text, new RegExp(`\\| \`exercised_artifact_digest\` \\| ${'b'.repeat(64)} \\|`));
  assert.match(text, /\| `claude_code_leg` \| pass --/);
  assert.match(text, /\| `codex_leg` \| uncertain -- .*names no skill path \/ x/);
  assert.match(text, /\| `gemini_leg` \| uncertain -- the leg was not run or has no observation/);
  assert.match(text, /\| `switch_leg` \| uncertain -- no checkpoint-and-restart flow exists \|/);
  assert.match(text, /\| `digests_unchanged` \| true/);
  assert.match(text, new RegExp(`\\| \`evidence_artifact\` \\| ${'a'.repeat(64)} -- `));
  const rows = text.split('\n').filter((line) => line.startsWith('| `'));
  assert.ok(rows.every((row) => row.split(' | ').length === 2), 'every row has exactly two cells');
});

test('formatRecord omits both digest rows when none is given and cites the named evidence file', () => {
  const text = formatRecord({
    runId: 'run-2', recordId: 'R', baselineId: 'B', buildChannel: 'packaged-ci', observedState: 'uncertain',
    runDate: '2026-11-01', operatorRole: 'r', legs: {}, digestsUnchanged: null,
    evidencePath: 'docs/evidence/VP-001/artifacts/vp-s29/run-2/bundle.txt', evidenceDigest: 'c'.repeat(64),
  });
  assert.doesNotMatch(text, /build_channel_digest|exercised_artifact_digest/);
  assert.match(text, /retained redacted file docs\/evidence\/VP-001\/artifacts\/vp-s29\/run-2\/bundle\.txt/);
});

test('an assistant text mention of AGENTS.md is not loaded-file evidence; a tool_use read is', () => {
  const textOnly = observeLeg({
    adapter: 'claude-code',
    transcript: lines(init, say('I read AGENTS.md at startup.'), tool('Read', { file_path: 'skills/release-notes/SKILL.md' })),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(textOnly.result, 'uncertain');
  assert.equal(textOnly.loadedFileEvidence.found, false);
  const read = observeLeg({
    adapter: 'claude-code',
    transcript: lines(init, tool('Read', { file_path: 'AGENTS.md' }), tool('Read', { file_path: 'skills/release-notes/SKILL.md' })),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(read.result, 'pass');
});

test('the same rule holds for the bridge, and for the indexed skill path named only in prose', () => {
  const textOnly = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(init, bridge, say('AGENTS.md says so.'), tool('command_execution', { command: 'cat skills/release-notes/SKILL.md' })),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(textOnly.result, 'uncertain');
  const proseSkill = observeLeg({
    adapter: 'claude-code',
    transcript: lines(init, tool('Read', { file_path: 'AGENTS.md' }), say('I used skills/release-notes/SKILL.md')),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(proseSkill.skillPaths.indexed, false);
  assert.equal(proseSkill.result, 'uncertain');
  const proseUser = observeLeg({
    adapter: 'claude-code',
    transcript: lines(init, tool('Read', { file_path: 'AGENTS.md' }), say('I used .claude/skills/release-notes/SKILL.md')),
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(proseUser.result, 'fail');
});

test('redact scrubs secret-shaped values and findSecrets reports what is left', () => {
  const key = `sk-ant-${'Ab3dEf6h'.repeat(4)}`;
  const text = [
    `Authorization: Bearer abc.def-ghi`, `token ${key}`, `gh ghp_${'a'.repeat(30)}`, `slack xoxb-123-456-${'z'.repeat(12)}`,
    `google AIza${'B'.repeat(35)}`, `hex ${'0123456789abcdef'.repeat(3)}`, 'path /work/definition/skills/release-notes/SKILL.md',
  ].join('\n');
  const out = redact(text, '');
  assert.doesNotMatch(out, /abc\.def|sk-ant|ghp_|xoxb|AIza|0123456789abcdef/);
  assert.match(out, /<redacted>/);
  assert.match(out, /\/work\/definition\/skills\/release-notes\/SKILL\.md/, 'ordinary paths survive');
  assert.deepEqual(findSecrets(out), []);
  assert.ok(findSecrets('-----BEGIN RSA PRIVATE KEY-----').length > 0);
  assert.ok(findSecrets(`raw ${key}`).length > 0);
});

test('redact with an empty or slash-only HOME prefix never splits the text', () => {
  assert.equal(redact('abc', '/'), 'abc');
  assert.equal(redact('abc', '//'), 'abc');
  assert.equal(redact('abc', ''), 'abc');
  assert.equal(redact('/home/op/x', '/home/op/'), '<home>/x');
});

test('a sha256 digest survives redaction and the detector, while base64-like runs do not', () => {
  const digest = 'ab'.repeat(32);
  const kept = redact(`sha256 ${digest}`, '');
  assert.equal(kept, `sha256 ${digest}`);
  assert.deepEqual(findSecrets(kept), []);
  const signature = 'Zm9vYmFy'.repeat(6);
  assert.equal(redact(`signature ${signature}`, ''), 'signature <redacted>');
  assert.ok(findSecrets(`signature ${signature}`).length > 0);
  assert.ok(findSecrets(`${digest}${'c'.repeat(10)}`).length > 0, 'a longer run is not a digest');
  assert.ok(findSecrets(`x${digest}`).length > 0, 'neither is a digest with a neighbour');
  const observation = JSON.stringify({ digest, other: { sha256: 'f'.repeat(64) } }, null, 2);
  assert.deepEqual(findSecrets(observation), []);
});

test('formatRecord lists each invocation digest result next to the overall value', () => {
  const text = formatRecord({
    runId: 'run-3', recordId: 'R', baselineId: 'B', runDate: '2026-11-01', operatorRole: 'r', legs: {},
    digestsUnchanged: false, evidencePath: 'p', evidenceDigest: 'e'.repeat(64),
    invocationDigests: [{ label: 'after-a-1', state: 'true' }, { label: 'after-a-2', state: 'false' }, { label: 'after-b-1', state: 'missing' }],
  });
  assert.match(text, /\| `digests_unchanged` \| false \|/);
  assert.match(text, /\| `digests_by_invocation` \| after-a-1 true; after-a-2 false; after-b-1 missing \|/);
  assert.match(text, /\| `result` \| fail \|/);
});

// Harness logs (src-tauri/src/harness_log.rs): one JSON object per line.
const record = (value) => JSON.stringify(value);
const header = (adapter = 'claude-code', runId = 'run-1') => record({
  kind: 'header', schema: 'omnifrons.harness-log/1', runId, adapter, startedAt: '2026-11-01T10:00:00Z',
});
const out = (text, continued = false, droppedBefore = 0) => record({ kind: 'stdout', text, continued, droppedBefore });
const err = (text) => record({ kind: 'stderr', text, continued: false, droppedBefore: 0 });
const state = (code = 0) => record({ kind: 'state', state: 'exited', code });
const log = (...parts) => `${parts.join('\n')}\n`;
const claudeLog = (...extra) => log(
  header('claude-code'), out(init), out(tool('Read', { file_path: 'AGENTS.md' })),
  out(tool('Read', { file_path: 'skills/release-notes/SKILL.md' })), ...extra, state(),
);

test('parseHarnessLog reassembles continued fragments and keeps stderr, state and drops apart', () => {
  const long = 'x'.repeat(30);
  const parsed = parseHarnessLog(log(
    header('claude-code', 'run-9'), out('first'), out(long.slice(0, 10), true), out(long.slice(10, 20), true), out(long.slice(20)),
    err('warn'), out('after', false, 3), record({ kind: 'truncated', framesDropped: 7 }), state(1),
  ));
  assert.deepEqual(parsed.stdoutLines, ['first', long, 'after']);
  assert.deepEqual(parsed.stderrLines, ['warn']);
  assert.equal(parsed.header.runId, 'run-9');
  assert.equal(parsed.header.adapter, 'claude-code');
  assert.equal(parsed.header.startedAt, '2026-11-01T10:00:00Z');
  assert.equal(parsed.droppedBefore, 3);
  assert.equal(parsed.truncatedFrames, 7);
  assert.deepEqual(parsed.state, { kind: 'state', state: 'exited', code: 1 });
});

test('parseHarnessLog flushes a fragment left open at the end and rejects bad logs', () => {
  assert.deepEqual(parseHarnessLog(log(header(), out('part', true))).stdoutLines, ['part']);
  assert.throws(() => parseHarnessLog(log(out('no header'))), /header/);
  assert.throws(() => parseHarnessLog(`${header()}\n{not json\n`), /line 2/);
  assert.throws(() => parseHarnessLog(log(record({ kind: 'header', schema: 'other/9', runId: 'r', adapter: 'a', startedAt: 't' }))), /schema/);
});

test('observeHarnessLogs passes a complete claude-code log and carries the log summary', () => {
  const result = observeHarnessLogs({ adapter: 'claude-code', logs: [claudeLog()], versionText: CLAUDE_VERSION, ...OPTIONS });
  assert.equal(result.result, 'pass');
  assert.equal(result.log.logs.length, 1);
  assert.deepEqual(result.log.logs[0].state, { kind: 'state', state: 'exited', code: 0 });
  assert.equal(result.log.incomplete, false);
  assert.equal(result.log.logs[0].runId, 'run-1');
});

test('a log whose header names another adapter fails the leg', () => {
  const result = observeHarnessLogs({
    adapter: 'claude-code', logs: [claudeLog().replace('"adapter":"claude-code"', '"adapter":"pty-cli"')], versionText: CLAUDE_VERSION, ...OPTIONS,
  });
  assert.equal(result.result, 'fail');
  assert.ok(result.reasons.some((reason) => /adapter/.test(reason)));
});

test('a truncated marker or any dropped frame makes the leg at most uncertain', () => {
  const truncated = observeHarnessLogs({
    adapter: 'claude-code', logs: [claudeLog(record({ kind: 'truncated', framesDropped: 4 }))], versionText: CLAUDE_VERSION, ...OPTIONS,
  });
  assert.equal(truncated.result, 'uncertain');
  assert.equal(truncated.log.incomplete, true);
  assert.ok(truncated.reasons.some((reason) => /incomplete log/.test(reason)));
  const dropped = observeHarnessLogs({
    adapter: 'claude-code',
    logs: [log(header(), out(init), out(tool('Read', { file_path: 'AGENTS.md' }), false, 2), out(tool('Read', { file_path: 'skills/release-notes/SKILL.md' })), state())],
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(dropped.result, 'uncertain');
  assert.equal(dropped.log.droppedBefore, 2);
});

test('a user-level skill path still fails even in an incomplete log', () => {
  const result = observeHarnessLogs({
    adapter: 'claude-code',
    logs: [claudeLog(out(tool('Read', { file_path: '/home/op/.claude/skills/release-notes/SKILL.md' })), record({ kind: 'truncated', framesDropped: 1 }))],
    versionText: CLAUDE_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'fail');
});

test('the two runs of a leg combine; a pty-cli log uses its text frames', () => {
  const first = log(header('claude-code', 'run-1'), out(init), out(tool('Read', { file_path: 'AGENTS.md' })), state());
  const second = log(header('claude-code', 'run-2'), out(init), out(tool('Read', { file_path: 'skills/release-notes/SKILL.md' })), state());
  const both = observeHarnessLogs({ adapter: 'claude-code', logs: [first, second], versionText: CLAUDE_VERSION, ...OPTIONS });
  assert.equal(both.result, 'pass');
  assert.deepEqual(both.log.logs.map((entry) => entry.runId), ['run-1', 'run-2']);
  const pty = observeHarnessLogs({
    adapter: 'pty-cli',
    logs: [log(header('pty-cli'), out('\u001b[1mLoaded 1 GEMINI.md file\u001b[0m'), out('ReadFile skills/release-notes/SKILL.md'), state())],
    versionText: GEMINI_VERSION,
    ...OPTIONS,
  });
  assert.equal(pty.result, 'pass');
});

test('a log with no final state line is incomplete, and says so', () => {
  const noState = log(header(), out(init), out(tool('Read', { file_path: 'AGENTS.md' })), out(tool('Read', { file_path: 'skills/release-notes/SKILL.md' })));
  assert.equal(parseHarnessLog(noState).state, null);
  const result = observeHarnessLogs({ adapter: 'claude-code', logs: [noState], versionText: CLAUDE_VERSION, ...OPTIONS });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.log.incomplete, true);
  assert.ok(result.reasons.some((reason) => /log has no final state/.test(reason)));
});

test('a log ending in a continued fragment is incomplete: the log ends mid-line', () => {
  const parsed = parseHarnessLog(log(header(), out('part', true), state()));
  assert.equal(parsed.endsMidLine, true);
  assert.equal(parseHarnessLog(log(header(), out('whole'), state())).endsMidLine, false);
  const result = observeHarnessLogs({
    adapter: 'claude-code', logs: [claudeLog(out('{"type":"assi', true))], versionText: CLAUDE_VERSION, ...OPTIONS,
  });
  assert.equal(result.result, 'uncertain');
  assert.ok(result.reasons.some((reason) => /log ends mid-line/.test(reason)));
});

test('drops recorded on the state line count towards the dropped total', () => {
  const lossy = log(
    header(), out(init), out(tool('Read', { file_path: 'AGENTS.md' })), out(tool('Read', { file_path: 'skills/release-notes/SKILL.md' })),
    record({ kind: 'state', droppedBefore: 2, state: 'exited', code: 0 }),
  );
  assert.equal(parseHarnessLog(lossy).droppedBefore, 2);
  const result = observeHarnessLogs({ adapter: 'claude-code', logs: [lossy], versionText: CLAUDE_VERSION, ...OPTIONS });
  assert.equal(result.result, 'uncertain');
  assert.equal(result.log.droppedBefore, 2);
});

test('a failed command execution naming the indexed skill path is not a read', () => {
  const failed = tool('command_execution', {
    command: "/usr/bin/zsh -lc 'cat skills/release-notes/SKILL.md'", status: 'failed', exit_code: '1', aggregated_output: 'bwrap: execvp: No such file or directory',
  });
  const result = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(init, bridge, failed, say('The local release-notes template could not be read.')),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.skillPaths.indexed, false);
  assert.equal(result.result, 'uncertain');
  assert.ok(result.reasons.some((reason) => /access to the indexed skill path failed/.test(reason)));
  assert.ok(result.skillPaths.excerpts.some((line) => /skills\/release-notes\/SKILL\.md/.test(line)));
});

test('a completed command execution naming the indexed skill path still counts as a read', () => {
  const completed = tool('command_execution', { command: 'cat skills/release-notes/SKILL.md', status: 'completed', exit_code: '0' });
  const result = observeLeg({
    adapter: 'stream-json-cli',
    transcript: lines(init, bridge, completed),
    versionText: CODEX_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.skillPaths.indexed, true);
  assert.ok(!result.reasons.some((reason) => /access to the indexed skill path failed/.test(reason)));
});
