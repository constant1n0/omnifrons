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
//        --digests-unchanged <true|false> --evidence-path <p> --evidence-digest <sha256>
//        [--record-id ..] [--baseline-id ..] [--build-channel ..] [--observed-state ..] [--operator-role ..]

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

/** Replaces the HOME prefix, and any other home-shaped prefix, with a placeholder. */
export function redact(text, home) {
  let out = text;
  if (home) out = out.split(home.replace(/\/+$/, '')).join('<home>');
  return out.replace(/\/(?:home|Users)\/[^/\s"'\\]+/g, '<home>');
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
  adapter, transcript, versionText, definitionName, skillName, home = '', userSkillMarker = 'USER-LEVEL-SKILL-MARKER',
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

  // Loaded-file evidence.
  let evidenceLines = [];
  if (adapter === 'claude-code') {
    evidenceLines = findLines(lines, /(?:CLAUDE|AGENTS)\.md/);
    if (!hasSystem('init')) evidenceLines = [];
  } else if (adapter === 'stream-json-cli') {
    evidenceLines = findLines(lines, /AGENTS\.md/);
    if (!hasSystem('init')) evidenceLines = [];
  } else {
    const name = definitionName ? new RegExp(escapeRegex(definitionName), 'i') : null;
    evidenceLines = lines.filter((line) => /GEMINI\.md/.test(line) || (name !== null && name.test(line)));
  }
  const loadedFound = evidenceLines.length > 0;
  if (!loadedFound && lines.length > 0) {
    reasons.push({
      'claude-code': 'no system/init line together with a line naming CLAUDE.md or AGENTS.md',
      'stream-json-cli': 'no system/init line together with a line naming AGENTS.md',
      'pty-cli': 'no normalized line names GEMINI.md or the definition',
    }[adapter]);
  }

  // Skill paths.
  const name = escapeRegex(skillName);
  const userPath = new RegExp(`\\.(?:claude|agents|gemini|codex)/skills/${name}/SKILL\\.md`);
  const indexedPath = new RegExp(`(?<!\\.(?:claude|agents|gemini|codex)/)skills/${name}/SKILL\\.md`);
  const userLines = lines.filter((line) => userPath.test(line) || line.includes(userSkillMarker));
  const indexedLines = findLines(lines, indexedPath);
  const userLevel = userLines.length > 0;
  const indexed = indexedLines.length > 0;
  if (userLevel) reasons.push('the transcript names the user-level skill (path or marker)');
  else if (!indexed && lines.length > 0) reasons.push('the transcript names no skill path');

  let result = 'pass';
  if (userLevel) result = 'fail';
  else if (!loadedFound || !indexed || !versionMatchesBinding || (adapter === 'stream-json-cli' && bridge === null)) {
    result = 'uncertain';
  }
  if (lines.length === 0 && result === 'pass') result = 'uncertain';

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
  runId, recordId, baselineId, buildChannel, observedState, runDate, operatorRole,
  legs, digestsUnchanged, evidencePath, evidenceDigest,
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
    ['observed_state', observedState ?? placeholder('observed_state token')],
    ['build_channel', buildChannel ?? placeholder('build channel of the exercised app')],
    ['variant_scan', 'absent'],
    ['procedure_ref', 'docs/evidence/VP-001/procedures/vp-s29-linux.sh'],
    ...LEG_ROWS.map(([adapter, key]) => [key, legCell(legs[adapter])]),
    ['switch_leg', `${SWITCH_LEG.result} -- ${SWITCH_LEG.reason}`],
    ['digests_unchanged', unchanged],
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
    } catch {
      legs[adapter] = null;
    }
  }
  const unchanged = get('--digests-unchanged');
  process.stdout.write(formatRecord({
    runId: get('--run-id'),
    recordId: get('--record-id'),
    baselineId: get('--baseline-id'),
    buildChannel: get('--build-channel'),
    observedState: get('--observed-state'),
    runDate: get('--run-date'),
    operatorRole: get('--operator-role'),
    legs,
    digestsUnchanged: unchanged === 'true' ? true : unchanged === 'false' ? false : null,
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
      const observation = observeLeg({
        adapter,
        transcript,
        versionText: readFileSync(versionPath, 'utf8'),
        definitionName: argValue(args, '--name') ?? manifest.definitionName ?? '',
        skillName: argValue(args, '--skill') ?? manifest.skillName ?? 'release-notes',
        home: argValue(args, '--home') ?? manifest.local?.home ?? process.env.VP029_HOME ?? '',
        userSkillMarker: manifest.userSkillMarker,
      });
      const redactedOut = argValue(args, '--redacted-out');
      if (redactedOut) {
        const home = argValue(args, '--home') ?? manifest.local?.home ?? process.env.VP029_HOME ?? '';
        writeFileSync(redactedOut, redact(normalizeText(transcript), home));
      }
      process.stdout.write(`${JSON.stringify(observation, null, 2)}\n`);
    } catch (error) {
      process.stderr.write(`vp-s29-observations.mjs: ${error.message}\n`);
      process.exit(1);
    }
  }
}
