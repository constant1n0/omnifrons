// Pure VP-S29 derivation: one leg's retained transcript plus the harness's
// `--version` output become a per-leg JSON observation (harness name and
// version, loaded-file evidence, which skill path appears, a `pass` |
// `uncertain` | `fail` result with reasons). It reads no process, display or
// network; the CLI at the bottom only reads the three files it is given.
//
// Evidence rules (docs/desktop-stack-verification-plan.md, VP-S29):
//   - a user-level skill path, or the user-level skill's marker, anywhere in
//     the transcript fails the leg;
//   - `pass` needs loaded-file evidence from the harness itself AND the
//     indexed skill path in the transcript AND the bound harness version;
//   - anything else is `uncertain`, never `pass`.
// Loaded-file evidence per adapter:
//   claude-code     the `system/init` line plus a line naming CLAUDE.md or AGENTS.md
//   stream-json-cli the bridge's `system/init` line plus a line naming AGENTS.md
//   pty-cli         a normalized line naming GEMINI.md or the definition's name
//
// CLI: node vp-s29-observations.mjs <adapter> <transcript> <version-file>
//        [--manifest <file>] [--name <definition name>] [--skill <name>] [--home <dir>] [--redacted-out <file>]
//      node vp-s29-observations.mjs record --run-dir <dir> --run-id <id> --run-date <date>
//        --digests-unchanged <true|false|unknown> [--invocations <label=state,...>] --evidence-path <p> --evidence-digest <sha256>
//        [--record-id ..] [--baseline-id ..] [--build-channel ..] [--build-channel-digest ..]
//        [--exercised-artifact-digest ..] [--observed-state ..] [--operator-role ..]

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const EXPECTED_VERSIONS = {
  'claude-code': '2.1.307',
  'stream-json-cli': '0.163.0',
  'pty-cli': '0.41.2',
};

const HARNESS_NAMES = {
  'claude-code': 'Claude Code',
  'stream-json-cli': 'Codex CLI through codex-stream-bridge',
  'pty-cli': 'Gemini CLI',
};

export const SWITCH_LEG = { result: 'uncertain', reason: 'no checkpoint-and-restart flow exists' };

// A number followed, within a few words, by `file` or `files`.
const GEMINI_NOTICE_COUNT = /\b\d+\b[^\n]{0,40}\bfiles?\b/i;
const SEMVER = /\d+\.\d+\.\d+/;
const EXCERPT_LIMIT = 200;

/** Strips terminal escape sequences, carriage returns and other control
 * characters, so a PTY capture compares as plain text. */
export function normalizeText(text) {
  return text
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)/g, '')
    .replace(/\u001b[@-Z\\-_]/g, '')
    .replace(/\r/g, '')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, '');
}

// Secret-shaped values a harness may print (tokens, keys, session ids in login
// or quota errors). Conservative on purpose: a false positive costs a manual
// look, a miss leaks a credential into a committed file.
// A run that is exactly a lowercase sha256 digest is evidence, not a secret.
const SHA256_DIGEST = /^[0-9a-f]{64}$/;
const LONG_RUN = /[A-Za-z0-9+=_-]{40,}/g;

const SECRET_REDACTIONS = [
  [/(authorization:\s*)[^\r\n]*/gi, '$1<redacted>'],
  [/(bearer\s+)[A-Za-z0-9._~+/=-]+/gi, '$1<redacted>'],
  [/\b(?:sk-ant-|sk-|ghp_|gho_|xoxb-|AIza)[A-Za-z0-9_-]{8,}/g, '<redacted>'],
  [/[A-Za-z0-9+=_-]{40,}/g, (run) => (SHA256_DIGEST.test(run) ? run : '<redacted>')],
];

// What `findSecrets` reports: the redaction patterns again, plus material that
// is never scrubbed automatically (a private key block) and so needs a person.
const SECRET_DETECTORS = [
  /authorization:\s*(?!<redacted>)\S/i,
  /bearer\s+(?!<redacted>)[A-Za-z0-9._~+/=-]+/i,
  /\b(?:sk-ant-|sk-|ghp_|gho_|xoxb-|AIza)[A-Za-z0-9_-]{8,}/,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----/,
];

/** Descriptions of the secret-shaped values left in `text` (never the values). */
export function findSecrets(text) {
  const found = SECRET_DETECTORS.filter((pattern) => pattern.test(text)).map((pattern) => `matches ${pattern.source}`);
  if ((text.match(LONG_RUN) ?? []).some((run) => !SHA256_DIGEST.test(run))) found.push(`matches ${LONG_RUN.source}`);
  return found;
}

/** Replaces the HOME prefix, any other home-shaped prefix and secret-shaped
 * values with placeholders. */
export function redact(text, home) {
  let out = text;
  const prefix = home ? home.replace(/\/+$/, '') : '';
  if (prefix) out = out.split(prefix).join('<home>');
  out = out.replace(/\/(?:home|Users)\/[^/\s"'\\]+/g, '<home>');
  for (const [pattern, replacement] of SECRET_REDACTIONS) out = out.replace(pattern, replacement);
  return out;
}

const excerptOf = (line, home) => {
  const clean = redact(line.trim(), home);
  return clean.length > EXCERPT_LIMIT ? `${clean.slice(0, EXCERPT_LIMIT)}...` : clean;
};

function parseJsonLine(line) {
  if (!line.startsWith('{')) return null;
  try {
    const value = JSON.parse(line);
    return value && typeof value === 'object' ? value : null;
  } catch {
    return null;
  }
}

const escapeRegex = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The `system` lines and `tool_use` blocks of a stream-json transcript, as
 * strings; every other line (assistant text, results, plain text) is left out. */
function harnessMaterial(lines, parsed) {
  const out = [];
  parsed.forEach((value, index) => {
    if (!value) return;
    if (value.type === 'system') out.push(lines[index]);
    const content = value.message?.content;
    if (Array.isArray(content)) {
      for (const block of content) {
        if (block && block.type === 'tool_use') out.push(JSON.stringify(block));
      }
    }
  });
  return out;
}

function findLines(lines, pattern) {
  return lines.filter((line) => pattern.test(line));
}

/**
 * Derives one leg's observation.
 * @param {{adapter: string, transcript: string, versionText: string,
 *   definitionName: string, skillName: string, home?: string,
 *   userSkillMarker?: string}} input
 */
export function observeLeg({
  adapter, transcript, versionText, skillName, home = '', userSkillMarker = 'USER-LEVEL-SKILL-MARKER',
}) {
  if (!Object.hasOwn(EXPECTED_VERSIONS, adapter)) throw new Error(`unknown adapter: ${adapter}`);
  const reasons = [];
  const lines = normalizeText(transcript).split('\n').map((line) => line.trim()).filter((line) => line !== '');
  if (lines.length === 0) reasons.push('the transcript is empty');
  const parsed = lines.map(parseJsonLine);
  const hasSystem = (subtype) => parsed.some((value) => value && value.type === 'system' && value.subtype === subtype);

  // Harness identity.
  const version = SEMVER.exec(versionText)?.[0] ?? null;
  const expectedVersion = EXPECTED_VERSIONS[adapter];
  let versionMatchesBinding = version === expectedVersion;
  if (version === null) reasons.push('the --version output carries no version');
  else if (!versionMatchesBinding) reasons.push(`the harness version ${version} is not the bound ${expectedVersion}`);
  let bridge = null;
  if (adapter === 'stream-json-cli') {
    const bridgeLine = parsed.find((value) => value && value.type === 'system' && value.subtype === 'bridge');
    if (bridgeLine) {
      bridge = {
        codexVersion: SEMVER.exec(String(bridgeLine.codex_version ?? ''))?.[0] ?? null,
        bridgeVersion: typeof bridgeLine.bridge_version === 'string' ? bridgeLine.bridge_version : null,
      };
      if (bridge.codexVersion !== null && bridge.codexVersion !== expectedVersion) {
        versionMatchesBinding = false;
        reasons.push(`the bridge reports Codex ${bridge.codexVersion}, not the bound ${expectedVersion}`);
      }
    } else {
      reasons.push('the transcript has no retained bridge version line (system/bridge)');
    }
  }

  // Material that comes from the harness, not the model's prose: `system`
  // lines and `tool_use` blocks (a read names its file in its input). An
  // assistant `text` block never counts. A PTY capture has no such structure.
  const jsonAdapter = adapter !== 'pty-cli';
  const material = jsonAdapter ? harnessMaterial(lines, parsed) : lines;

  // Loaded-file evidence.
  let evidenceLines = [];
  if (adapter === 'claude-code') {
    evidenceLines = findLines(material, /(?:CLAUDE|AGENTS)\.md/);
    if (!hasSystem('init')) evidenceLines = [];
  } else if (adapter === 'stream-json-cli') {
    evidenceLines = findLines(material, /AGENTS\.md/);
    if (!hasSystem('init')) evidenceLines = [];
  } else {
    // Only Gemini CLI's own context-file notice counts: a line naming GEMINI.md
    // together with a count of files ("Loaded 1 GEMINI.md file", "1 context file
    // (GEMINI.md)"). The model saying who it is, or merely mentioning the file, is prose.
    evidenceLines = lines.filter((line) => /GEMINI\.md/.test(line) && GEMINI_NOTICE_COUNT.test(line));
  }
  const loadedFound = evidenceLines.length > 0;
  if (!loadedFound && lines.length > 0) {
    reasons.push({
      'claude-code': 'no system/init line together with a line naming CLAUDE.md or AGENTS.md',
      'stream-json-cli': 'no system/init line together with a line naming AGENTS.md',
      'pty-cli': 'no normalized line is the CLI notice counting GEMINI.md files',
    }[adapter]);
  }

  // Skill paths.
  const skillPattern = escapeRegex(skillName);
  const userPath = new RegExp(`\\.(?:claude|agents|gemini|codex)/skills/${skillPattern}/SKILL\\.md`);
  const indexedPath = new RegExp(`(?<!\\.(?:claude|agents|gemini|codex)/)skills/${skillPattern}/SKILL\\.md`);
  const userLines = lines.filter((line) => userPath.test(line) || line.includes(userSkillMarker));
  const indexedLines = findLines(material, indexedPath);
  const userLevel = userLines.length > 0;
  const indexed = indexedLines.length > 0;
  if (userLevel) reasons.push('the transcript names the user-level skill (path or marker)');
  else if (!indexed && lines.length > 0) reasons.push('the transcript names no skill path');

  let result = 'pass';
  if (userLevel) result = 'fail';
  else if (!loadedFound || !indexed || !versionMatchesBinding || (adapter === 'stream-json-cli' && bridge === null)) {
    result = 'uncertain';
  }

  return {
    adapter,
    harness: {
      name: HARNESS_NAMES[adapter], version, expectedVersion, versionMatchesBinding, bridge,
    },
    loadedFileEvidence: { found: loadedFound, excerpts: evidenceLines.slice(0, 3).map((line) => excerptOf(line, home)) },
    skillPaths: {
      indexed,
      userLevel,
      excerpts: [...new Set([...userLines, ...indexedLines])].slice(0, 4).map((line) => excerptOf(line, home)),
    },
    result,
    reasons,
  };
}

const LEG_ROWS = [
  ['claude-code', 'claude_code_leg'],
  ['stream-json-cli', 'codex_leg'],
  ['pty-cli', 'gemini_leg'],
];

const cell = (text) => String(text).replace(/\|/g, '/').replace(/\s+/g, ' ').trim();

/** `fail` on any failed leg or any changed digest; otherwise `uncertain`,
 * because the switch leg is always `uncertain` (SWITCH_LEG). */
export function overallResult(legs, digestsUnchanged) {
  if (digestsUnchanged === false) return 'fail';
  if (Object.values(legs).some((observation) => observation && observation.result === 'fail')) return 'fail';
  return 'uncertain';
}

function legCell(observation) {
  if (!observation) return 'uncertain -- the leg was not run or has no observation';
  const { harness, loadedFileEvidence, skillPaths, reasons } = observation;
  const facts = [
    `${harness.name} ${harness.version ?? 'unknown version'}`,
    `loaded-file evidence ${loadedFileEvidence.found}`,
    `indexed skill path ${skillPaths.indexed}`,
    `user-level skill path ${skillPaths.userLevel}`,
  ];
  if (reasons.length > 0) facts.push(reasons.join('; '));
  return cell(`${observation.result} -- ${facts.join('; ')}`);
}

/** Renders the run's scenario record (README.md "Scenario record") for the
 * operator to review and append to records.md. A value the script cannot
 * know is left as an `<operator: ...>` placeholder, never invented. */
export function formatRecord({
  runId, recordId, baselineId, buildChannel, buildChannelDigest, exercisedArtifactDigest, observedState,
  runDate, operatorRole, legs, digestsUnchanged, invocationDigests = [], evidencePath, evidenceDigest,
}) {
  const placeholder = (what) => `<operator: ${what}>`;
  const versions = LEG_ROWS.map(([adapter]) => {
    const observation = legs[adapter];
    if (!observation) return `${HARNESS_NAMES[adapter]} not run`;
    const bridge = observation.harness.bridge;
    const via = bridge ? ` (bridge reports Codex ${bridge.codexVersion ?? 'unknown'}, bridge ${bridge.bridgeVersion ?? 'unknown'})` : '';
    return `${observation.harness.name} ${observation.harness.version ?? 'unknown'}${via}`;
  }).join('; ');
  const unchanged = digestsUnchanged === null ? 'unknown -- digests were not compared' : String(digestsUnchanged);
  const rows = [
    ['record_id', recordId ?? placeholder('next free VP-001-VP-S29-NN id')],
    ['kind', 'scenario'],
    ['scenario_id', 'VP-S29'],
    ['baseline_id', baselineId ?? placeholder('pinned baseline id')],
    ['result', overallResult(legs, digestsUnchanged)],
    ['observed_state', observedState ?? 'uncertain'],
    ['build_channel', buildChannel ?? 'packaged-ci'],
    ...(buildChannelDigest && exercisedArtifactDigest
      ? [['build_channel_digest', buildChannelDigest], ['exercised_artifact_digest', exercisedArtifactDigest]]
      : []),
    ['variant_scan', 'absent'],
    ['procedure_ref', 'docs/evidence/VP-001/procedures/vp-s29-linux.sh'],
    ...LEG_ROWS.map(([adapter, key]) => [key, legCell(legs[adapter])]),
    ['switch_leg', `${SWITCH_LEG.result} -- ${SWITCH_LEG.reason}`],
    ['digests_unchanged', unchanged],
    ...(invocationDigests.length > 0
      ? [['digests_by_invocation', invocationDigests.map(({ label, state }) => `${label} ${state}`).join('; ')]]
      : []),
    ['harness_versions', versions],
    ['run_date', runDate],
    ['operator_role', operatorRole ?? placeholder('role of the person who ran it')],
    ['evidence_artifact', `${evidenceDigest} -- SHA-256 of the retained redacted file ${evidencePath}`],
  ];
  return [
    `## VP-001 VP-S29 scenario -- local operator run ${runId} (${runDate})`,
    '',
    'A local operator run with user-installed, credentialed harnesses. The switch leg is `uncertain` until a',
    'checkpoint-and-restart flow exists, so this run does not discharge the ADR-0005 gate.',
    '',
    '| field | value |',
    '| --- | --- |',
    ...rows.map(([key, value]) => `| \`${key}\` | ${cell(value)} |`),
    '',
  ].join('\n');
}

function argValue(args, flag) {
  const at = args.indexOf(flag);
  return at >= 0 ? args[at + 1] : undefined;
}

function recordMain(args) {
  const runDir = argValue(args, '--run-dir');
  const get = (flag) => argValue(args, flag) ?? null;
  const legs = {};
  for (const [adapter] of LEG_ROWS) {
    try {
      legs[adapter] = JSON.parse(readFileSync(`${runDir}/observation-${adapter}.json`, 'utf8'));
    } catch (error) {
      if (error.code !== 'ENOENT') throw new Error(`observation-${adapter}.json: ${error.message}`);
      legs[adapter] = null;
    }
  }
  const unchanged = get('--digests-unchanged');
  process.stdout.write(formatRecord({
    runId: get('--run-id'),
    recordId: get('--record-id'),
    baselineId: get('--baseline-id'),
    buildChannel: get('--build-channel'),
    buildChannelDigest: get('--build-channel-digest'),
    exercisedArtifactDigest: get('--exercised-artifact-digest'),
    observedState: get('--observed-state'),
    runDate: get('--run-date'),
    operatorRole: get('--operator-role'),
    legs,
    digestsUnchanged: unchanged === 'true' ? true : unchanged === 'false' ? false : null,
    invocationDigests: (get('--invocations') ?? '').split(',').filter((pair) => pair !== '').map((pair) => {
      const at = pair.lastIndexOf('=');
      return { label: pair.slice(0, at), state: pair.slice(at + 1) };
    }),
    evidencePath: get('--evidence-path'),
    evidenceDigest: get('--evidence-digest'),
  }));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args[0] === 'record') {
    try {
      recordMain(args);
    } catch (error) {
      process.stderr.write(`vp-s29-observations.mjs: ${error.message}\n`);
      process.exit(1);
    }
  } else {
    const [adapter, transcriptPath, versionPath] = args;
    if (!adapter || !transcriptPath || !versionPath) {
      process.stderr.write('usage: vp-s29-observations.mjs <adapter> <transcript> <version-file> [--manifest f] [--name N] [--skill S] [--home H] [--redacted-out f]\n');
      process.exit(2);
    }
    try {
      const manifestPath = argValue(args, '--manifest');
      const manifest = manifestPath ? JSON.parse(readFileSync(manifestPath, 'utf8')) : {};
      const transcript = readFileSync(transcriptPath, 'utf8');
      const home = argValue(args, '--home') ?? manifest.local?.home ?? process.env.VP_S29_HOME ?? '';
      const observation = observeLeg({
        adapter,
        transcript,
        versionText: readFileSync(versionPath, 'utf8'),
        skillName: argValue(args, '--skill') ?? manifest.skillName ?? 'release-notes',
        home,
        userSkillMarker: manifest.userSkillMarker,
      });
      const observationText = `${JSON.stringify(observation, null, 2)}\n`;
      const redactedText = redact(normalizeText(transcript), home);
      if (findSecrets(redactedText).length > 0 || findSecrets(observationText).length > 0) {
        process.stderr.write('vp-s29-observations.mjs: a secret-shaped value remains after redaction; review the transcript by hand\n');
        process.exit(1);
      }
      const redactedOut = argValue(args, '--redacted-out');
      if (redactedOut) writeFileSync(redactedOut, redactedText);
      process.stdout.write(observationText);
    } catch (error) {
      process.stderr.write(`vp-s29-observations.mjs: ${error.message}\n`);
      process.exit(1);
    }
  }
}
