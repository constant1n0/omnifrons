// `node --test` coverage for `vp-s29-observations.mjs`: per-leg derivation
// from hand-written transcripts, one per outcome. The transcripts are plain
// strings shaped like what each adapter retains (stream-json lines for
// `claude-code` and the Codex bridge, normalized terminal text for `pty-cli`).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  EXPECTED_VERSIONS, SWITCH_LEG, formatRecord, observeLeg, overallResult,
} from './vp-s29-observations.mjs';

const OPTIONS = { definitionName: 'Orbit Scribe', skillName: 'release-notes', home: '/home/op' };
const json = (value) => JSON.stringify(value);
const lines = (...parts) => `${parts.join('\n')}\n`;
const init = json({ type: 'system', subtype: 'init', cwd: '/work/definition', model: 'm', session_id: 's' });
const bridge = json({ type: 'system', subtype: 'bridge', codex_version: 'codex-cli 0.163.0', bridge_version: '0.1.0' });
const say = (text) => json({ type: 'assistant', message: { content: [{ type: 'text', text }] } });
const tool = (name, input) => json({ type: 'assistant', message: { content: [{ type: 'tool_use', name, input }] } });
const CLAUDE_VERSION = '2.1.307 (Claude Code)\n';
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
  assert.equal(result.harness.version, '2.1.307');
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
    transcript: '\u001b[2J\u001b[1;32mLoaded GEMINI.md\u001b[0m\r\nReadFile skills/release-notes/SKILL.md\r\n',
    versionText: GEMINI_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'pass');
  assert.equal(result.harness.name, 'Gemini CLI');
  assert.equal(result.harness.version, '0.41.2');
});

test('pty-cli accepts the definition name as loaded-file evidence', () => {
  const result = observeLeg({
    adapter: 'pty-cli',
    transcript: 'I am Orbit Scribe.\nReadFile skills/release-notes/SKILL.md\n',
    versionText: GEMINI_VERSION,
    ...OPTIONS,
  });
  assert.equal(result.result, 'pass');
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
