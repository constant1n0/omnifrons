// `node --test` coverage for `vp-s29-linux.sh`'s pure modes (`--help`,
// `fixture`, `digests`, `compare`, `observe`, `record`) over a temp directory.
// No harness, app or network is involved. Linux-only (`sha256sum`).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, 'vp-s29-linux.sh');
const LINUX_ONLY = process.platform === 'linux' ? {} : { skip: 'vp-s29-linux.sh needs GNU sha256sum (Linux only)' };

function withDir(run) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s29-linux-'));
  try {
    return run(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function sh(args, env = {}) {
  const result = spawnSync('bash', [SCRIPT, ...args], {
    encoding: 'utf8', env: { PATH: process.env.PATH, ...env }, timeout: 30_000,
  });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

function prepared(dir) {
  const runDir = join(dir, 'run');
  const root = join(dir, 'definition');
  const home = join(dir, 'home');
  const env = { VP029_RUN_ID: 'test-run', VP029_RUN_DIR: runDir, VP029_ROOT: root, VP029_HOME: home };
  const result = sh(['fixture'], env);
  assert.equal(result.status, 0, result.stderr);
  return { runDir, root, home, env };
}

test('--help prints the modes and exits 0; no mode and an unknown mode exit 2', LINUX_ONLY, () => {
  const help = sh(['--help']);
  assert.equal(help.status, 0);
  for (const mode of ['fixture', 'digests', 'compare', 'observe', 'record']) assert.match(help.stdout, new RegExp(mode));
  assert.equal(sh([]).status, 2);
  assert.equal(sh(['launch']).status, 2);
});

test('fixture writes the definition root, the user-level skills under HOME and fixture.json', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, home } = prepared(dir);
    assert.ok(existsSync(join(root, 'skills', 'release-notes', 'SKILL.md')));
    assert.ok(existsSync(join(home, '.claude', 'skills', 'release-notes', 'SKILL.md')));
    const manifest = JSON.parse(readFileSync(join(runDir, 'private', 'fixture.json'), 'utf8'));
    assert.equal(manifest.skillName, 'release-notes');
    assert.equal(readFileSync(join(runDir, 'private', '.gitignore'), 'utf8'), '*\n!.gitignore\n');
    assert.equal(existsSync(join(runDir, 'fixture.json')), false);
  });
});

test('digests then compare: unchanged exits 0; an edited SKILL.md, a new GEMINI.md or a removal exit 1', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    assert.equal(sh(['digests', 'before'], env).status, 0);
    const before = readFileSync(join(runDir, 'digests-before.txt'), 'utf8');
    assert.match(before, /^[0-9a-f]{64} {2}AGENTS\.md$/m);
    assert.match(before, /^[0-9a-f]{64} {2}skills\/release-notes\/SKILL\.md$/m);
    assert.match(before, /^absent {2}GEMINI\.md$/m);
    assert.match(before, /^absent {2}\.agents\/skills$/m);
    assert.match(before, /^absent {2}\.gemini\/skills$/m);
    assert.doesNotMatch(before, new RegExp(dir), 'no absolute path in a digest file');

    mkdirSync(join(root, '.omnifrons'));
    writeFileSync(join(root, '.omnifrons', 'state.json'), '{}');
    assert.equal(sh(['digests', 'after'], env).status, 0);
    const same = sh(['compare', join(runDir, 'digests-before.txt'), join(runDir, 'digests-after.txt')]);
    assert.equal(same.status, 0, '.omnifrons changes are not digested');

    writeFileSync(join(root, 'skills', 'release-notes', 'SKILL.md'), 'edited\n');
    sh(['digests', 'edited'], env);
    const edited = sh(['compare', join(runDir, 'digests-before.txt'), join(runDir, 'digests-edited.txt')]);
    assert.equal(edited.status, 1);
    assert.match(edited.stdout, /SKILL\.md/);

    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    writeFileSync(join(root, 'GEMINI.md'), 'wrapper\n');
    sh(['digests', 'wrapper'], env);
    assert.equal(sh(['compare', join(runDir, 'digests-before.txt'), join(runDir, 'digests-wrapper.txt')]).status, 1);

    rmSync(join(root, 'AGENTS.md'));
    sh(['digests', 'gone'], env);
    assert.equal(sh(['compare', join(runDir, 'digests-before.txt'), join(runDir, 'digests-gone.txt')]).status, 1);
  });
});

test('compare and digests refuse bad arguments with exit 2', LINUX_ONLY, () => {
  withDir((dir) => {
    assert.equal(sh(['compare', join(dir, 'nope-a'), join(dir, 'nope-b')]).status, 2);
    assert.equal(sh(['digests'], { VP029_RUN_DIR: dir, VP029_ROOT: dir }).status, 2);
    assert.equal(sh(['digests', 'a/b'], { VP029_RUN_DIR: dir, VP029_ROOT: dir }).status, 2, 'label is a plain word');
    assert.equal(sh(['digests', 'x'], { VP029_RUN_DIR: dir }).status, 2, 'root is required');
  });
});

test('observe writes observation-<adapter>.json and redacts HOME', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, home, env } = prepared(dir);
    const transcript = join(dir, 'gemini.txt');
    const version = join(dir, 'gemini-version.txt');
    writeFileSync(transcript, `Loaded GEMINI.md\nReadFile ${home}/.gemini/skills/release-notes/SKILL.md\n`);
    writeFileSync(version, '0.41.2\n');
    const result = sh(['observe', 'pty-cli', transcript, version], env);
    assert.equal(result.status, 0, result.stderr);
    const written = readFileSync(join(runDir, 'observation-pty-cli.json'), 'utf8');
    assert.equal(JSON.parse(written).result, 'fail');
    assert.ok(!written.includes(home), 'HOME is redacted from the retained observation');
    assert.ok(!readFileSync(join(runDir, 'transcript-pty-cli.txt'), 'utf8').includes(home), 'and from the retained transcript');
    assert.equal(existsSync(join(runDir, 'observation-bogus.json')), false);
    assert.equal(sh(['observe', 'bogus', transcript, version], env).status, 1);
  });
});

test('record assembles record.md and evidence-summary.txt; a changed digest makes it fail', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    sh(['digests', 'before'], env);
    sh(['digests', 'after'], env);
    const transcript = join(dir, 'claude.txt');
    const version = join(dir, 'claude-version.txt');
    writeFileSync(transcript, '{"type":"system","subtype":"init"}\n{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"AGENTS.md"}},{"type":"tool_use","name":"Read","input":{"file_path":"skills/release-notes/SKILL.md"}}]}}\n');
    writeFileSync(version, '2.1.307 (Claude Code)\n');
    assert.equal(sh(['observe', 'claude-code', transcript, version], env).status, 0);

    const args = ['record', '--baseline-id', 'VP-001-BASE-01', '--run-date', '2026-11-01'];
    const ok = sh(args, env);
    assert.equal(ok.status, 0, ok.stderr);
    const record = readFileSync(join(runDir, 'record.md'), 'utf8');
    assert.match(record, /\| `result` \| uncertain \|/);
    assert.match(record, /\| `claude_code_leg` \| pass --/);
    assert.match(record, /\| `digests_unchanged` \| true \|/);
    assert.match(record, /\| `baseline_id` \| VP-001-BASE-01 \|/);
    assert.match(record, /\| `evidence_artifact` \| [0-9a-f]{64} -- /);
    assert.ok(existsSync(join(runDir, 'evidence-summary.txt')));
    assert.ok(!record.includes(dir), 'no local path in the record');

    writeFileSync(join(root, 'AGENTS.md'), 'changed\n');
    sh(['digests', 'after'], env);
    assert.equal(sh(args, env).status, 0);
    assert.match(readFileSync(join(runDir, 'record.md'), 'utf8'), /\| `result` \| fail \|/);
  });
});

test('record without --run-date or with a missing run directory exits 2', LINUX_ONLY, () => {
  withDir((dir) => {
    assert.equal(sh(['record'], { VP029_RUN_ID: 'x', VP029_RUN_DIR: join(dir, 'missing') }).status, 2);
  });
});

test('digests show a host project-skill folder that appears during a run', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    sh(['digests', 'before'], env);
    mkdirSync(join(root, '.agents', 'skills'), { recursive: true });
    sh(['digests', 'after'], env);
    assert.match(readFileSync(join(runDir, 'digests-after.txt'), 'utf8'), /^present {2}\.agents\/skills$/m);
    assert.equal(sh(['compare', join(runDir, 'digests-before.txt'), join(runDir, 'digests-after.txt')]).status, 1);
  });
});

test('record takes the channel, the two digests and a named evidence file', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, env } = prepared(dir);
    writeFileSync(join(runDir, 'bundle.txt'), 'evidence\n');
    const base = ['record', '--run-date', '2026-11-01'];
    const digest = 'd'.repeat(64);
    const ok = sh([...base, '--build-channel-digest', digest, '--exercised-artifact-digest', digest, '--evidence-artifact', 'bundle.txt'], env);
    assert.equal(ok.status, 0, ok.stderr);
    const record = readFileSync(join(runDir, 'record.md'), 'utf8');
    assert.match(record, /\| `build_channel` \| packaged-ci \|/);
    assert.match(record, /\| `observed_state` \| uncertain \|/);
    assert.match(record, new RegExp(`\\| \`build_channel_digest\` \\| ${digest} \\|`));
    assert.match(record, /retained redacted file docs\/evidence\/VP-001\/artifacts\/vp-s29\/test-run\/bundle\.txt/);
    assert.equal(sh([...base, '--build-channel-digest', digest], env).status, 2, 'both digests or neither');
    assert.equal(sh([...base, '--build-channel-digest', 'xyz', '--exercised-artifact-digest', 'xyz'], env).status, 2);
    assert.equal(sh([...base, '--evidence-artifact', '../escape.txt'], env).status, 2);
    assert.equal(sh([...base, '--evidence-artifact', 'missing.txt'], env).status, 2);
  });
});
