// `node --test` coverage for `vp-s29-linux.sh`'s pure modes (`--help`,
// `fixture`, `digests`, `compare`, `observe`, `record`) over a temp directory.
// No harness, app or network is involved. Linux-only (`sha256sum`).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, 'vp-s29-linux.sh');
const IS_ROOT = process.getuid?.() === 0;
const LINUX_ONLY = process.platform === 'linux' ? {} : { skip: 'vp-s29-linux.sh needs GNU sha256sum (Linux only)' };

const unreadableTest = process.platform !== 'linux' ? LINUX_ONLY : { skip: IS_ROOT ? 'root reads everything' : false };

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
  const env = { VP_S29_RUN_ID: 'test-run', VP_S29_RUN_DIR: runDir, VP_S29_ROOT: root, VP_S29_HOME: home };
  const result = sh(['fixture'], env);
  assert.equal(result.status, 0, result.stderr);
  return { runDir, root, home, env };
}

test('--help prints the modes and exits 0; no mode and an unknown mode exit 2', LINUX_ONLY, () => {
  const help = sh(['--help']);
  assert.equal(help.status, 0);
  for (const mode of ['fixture', 'digests', 'compare', 'observe', 'record']) assert.match(help.stdout, new RegExp(mode));
  const helpLines = help.stdout.trimEnd().split('\n');
  assert.equal(helpLines[0], 'Usage: vp-s29-linux.sh --help');
  assert.match(helpLines.at(-1), /^Exit: 0 ok;/);
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
    assert.equal(sh(['digests'], { VP_S29_RUN_DIR: dir, VP_S29_ROOT: dir }).status, 2);
    assert.equal(sh(['digests', 'a/b'], { VP_S29_RUN_DIR: dir, VP_S29_ROOT: dir }).status, 2, 'label is a plain word');
    assert.equal(sh(['digests', 'x'], { VP_S29_RUN_DIR: dir }).status, 2, 'root is required');
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
    assert.equal(sh(['observe', 'bogus', transcript, version], env).status, 1);
    assert.equal(existsSync(join(runDir, 'observation-bogus.json')), false);
  });
});

const ADAPTERS = ['claude-code', 'stream-json-cli', 'pty-cli'];
const AFTER_LABELS = ADAPTERS.flatMap((adapter) => [1, 2].map((index) => `after-${adapter}-${index}`));

function observedClaude(dir, env) {
  const transcript = join(dir, 'claude.txt');
  const version = join(dir, 'claude-version.txt');
  writeFileSync(transcript, '{"type":"system","subtype":"init"}\n{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"AGENTS.md"}},{"type":"tool_use","name":"Read","input":{"file_path":"skills/release-notes/SKILL.md"}}]}}\n');
  writeFileSync(version, '2.1.307 (Claude Code)\n');
  assert.equal(sh(['observe', 'claude-code', transcript, version], env).status, 0);
}

test('record assembles record.md and evidence-summary.txt from six per-invocation digests', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    sh(['digests', 'before'], env);
    for (const label of AFTER_LABELS) assert.equal(sh(['digests', label], env).status, 0);
    observedClaude(dir, env);

    const args = ['record', '--baseline-id', 'VP-001-BASE-01', '--run-date', '2026-11-01'];
    const ok = sh(args, env);
    assert.equal(ok.status, 0, ok.stderr);
    const record = readFileSync(join(runDir, 'record.md'), 'utf8');
    assert.match(record, /\| `result` \| uncertain \|/);
    assert.match(record, /\| `claude_code_leg` \| pass --/);
    assert.match(record, /\| `digests_unchanged` \| true \|/);
    for (const label of AFTER_LABELS) assert.match(record, new RegExp(`${label} true`));
    assert.match(record, /\| `baseline_id` \| VP-001-BASE-01 \|/);
    assert.match(record, /\| `evidence_artifact` \| [0-9a-f]{64} -- /);
    assert.ok(existsSync(join(runDir, 'evidence-summary.txt')));
    assert.ok(!record.includes(dir), 'no local path in the record');
  });
});

test('a changed intermediate digest makes the record fail even when the last one is unchanged', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    sh(['digests', 'before'], env);
    for (const label of AFTER_LABELS) {
      if (label === 'after-stream-json-cli-1') writeFileSync(join(root, 'AGENTS.md'), 'changed\n');
      if (label === 'after-stream-json-cli-2') writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
      sh(['digests', label], env);
    }
    observedClaude(dir, env);
    assert.equal(sh(['record', '--run-date', '2026-11-01'], env).status, 0);
    const record = readFileSync(join(runDir, 'record.md'), 'utf8');
    assert.match(record, /\| `result` \| fail \|/);
    assert.match(record, /\| `digests_unchanged` \| false \|/);
    assert.match(record, /after-stream-json-cli-1 false/);
    assert.match(record, /after-stream-json-cli-2 true/);
  });
});

test('a missing per-invocation digest leaves the overall value unknown, never true', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    writeFileSync(join(root, 'AGENTS.md'), 'canonical\n');
    sh(['digests', 'before'], env);
    sh(['digests', AFTER_LABELS[0]], env);
    assert.equal(sh(['record', '--run-date', '2026-11-01'], env).status, 0);
    const record = readFileSync(join(runDir, 'record.md'), 'utf8');
    assert.match(record, /\| `digests_unchanged` \| unknown/);
    assert.match(record, /after-pty-cli-2 missing/);
    assert.match(record, /\| `result` \| uncertain \|/);
  });
});

test('record without --run-date or with a missing run directory exits 2', LINUX_ONLY, () => {
  withDir((dir) => {
    assert.equal(sh(['record'], { VP_S29_RUN_ID: 'x', VP_S29_RUN_DIR: join(dir, 'missing') }).status, 2);
    const existing = join(dir, 'existing');
    mkdirSync(existing);
    const env = { VP_S29_RUN_ID: 'x', VP_S29_RUN_DIR: existing };
    const noDate = sh(['record'], env);
    assert.equal(noDate.status, 2);
    assert.match(noDate.stderr, /--run-date/);
    assert.equal(sh(['record', '--run-date', '1 Nov 2026'], env).status, 2);
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

test('digests list a symlinked or dangling SKILL.md as a symlink entry and never fail on it', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    mkdirSync(join(root, 'skills', 'other'));
    symlinkSync(join(dir, 'does-not-exist'), join(root, 'skills', 'other', 'SKILL.md'));
    const result = sh(['digests', 'before'], env);
    assert.equal(result.status, 0, result.stderr);
    const digests = readFileSync(join(runDir, 'digests-before.txt'), 'utf8');
    assert.match(digests, /^symlink {2}skills\/other\/SKILL\.md -> .*does-not-exist {2}dangling$/m);
    assert.match(digests, /^[0-9a-f]{64} {2}skills\/release-notes\/SKILL\.md$/m);
  });
});

test('a failing digests run leaves no partial digest file', unreadableTest, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    chmodSync(join(root, 'skills', 'release-notes', 'SKILL.md'), 0o000);
    const result = sh(['digests', 'before'], env);
    assert.notEqual(result.status, 0);
    assert.equal(existsSync(join(runDir, 'digests-before.txt')), false);
  });
});

test('compare separates a diff tool error (2) from a difference (1)', unreadableTest, () => {
  withDir((dir) => {
    const a = join(dir, 'a.txt');
    const b = join(dir, 'b.txt');
    writeFileSync(a, 'one\n');
    writeFileSync(b, 'one\n');
    chmodSync(b, 0o000);
    const result = sh(['compare', a, b]);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /diff failed/);
  });
});

test('record leaves no record.md when it fails, and names a corrupt observation file', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, env } = prepared(dir);
    writeFileSync(join(runDir, 'observation-claude-code.json'), '{not json');
    const result = sh(['record', '--run-date', '2026-11-01'], env);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /observation-claude-code\.json/);
    assert.equal(existsSync(join(runDir, 'record.md')), false);
    assert.deepEqual(readdirSync(runDir).filter((name) => name.includes('record')), []);
  });
});

test('observe exits 1 and keeps nothing when the transcript still holds a secret-shaped value', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, env } = prepared(dir);
    const transcript = join(dir, 'gemini.txt');
    const version = join(dir, 'gemini-version.txt');
    writeFileSync(version, '0.41.2\n');
    writeFileSync(transcript, `Loaded GEMINI.md\nkey sk-ant-${'a1B2c3D4'.repeat(4)}\n`);
    const scrubbed = sh(['observe', 'pty-cli', transcript, version], env);
    assert.equal(scrubbed.status, 0, scrubbed.stderr);
    assert.doesNotMatch(readFileSync(join(runDir, 'transcript-pty-cli.txt'), 'utf8'), /sk-ant-/);
    writeFileSync(transcript, 'Loaded GEMINI.md\n-----BEGIN PRIVATE KEY-----\nabc\n');
    const refused = sh(['observe', 'pty-cli', transcript, version], env);
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /secret-shaped/);
    assert.equal(existsSync(join(runDir, 'transcript-pty-cli.txt')), false);
    assert.equal(existsSync(join(runDir, 'observation-pty-cli.json')), false);
  });
});

test('digests fail and leave no file when a skills subdirectory cannot be listed', unreadableTest, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    const hidden = join(root, 'skills', 'hidden');
    mkdirSync(hidden);
    chmodSync(hidden, 0o000);
    try {
      const result = sh(['digests', 'before'], env);
      assert.notEqual(result.status, 0);
      assert.equal(existsSync(join(runDir, 'digests-before.txt')), false);
    } finally {
      chmodSync(hidden, 0o755);
    }
  });
});

test('a symlinked SKILL.md records its target and the target content, and a dangling one says so', LINUX_ONLY, () => {
  withDir((dir) => {
    const { runDir, root, env } = prepared(dir);
    mkdirSync(join(root, 'skills', 'linked'));
    writeFileSync(join(root, 'skills', 'linked', 'real-a.md'), 'content a\n');
    writeFileSync(join(root, 'skills', 'linked', 'real-b.md'), 'content a\n');
    const link = join(root, 'skills', 'linked', 'SKILL.md');
    const take = (label) => {
      assert.equal(sh(['digests', label], env).status, 0);
      return readFileSync(join(runDir, `digests-${label}.txt`), 'utf8');
    };
    symlinkSync('real-a.md', link);
    const first = take('one');
    assert.match(first, /^symlink {2}skills\/linked\/SKILL\.md -> real-a\.md {2}[0-9a-f]{64}$/m);
    writeFileSync(join(root, 'skills', 'linked', 'real-a.md'), 'content changed\n');
    const changed = take('two');
    assert.notEqual(changed, first, 'rewritten target content shows');
    unlinkSync(link);
    symlinkSync('real-b.md', link);
    writeFileSync(join(root, 'skills', 'linked', 'real-a.md'), 'content a\n');
    const retargeted = take('three');
    assert.match(retargeted, /-> real-b\.md/);
    assert.notEqual(retargeted, first, 'a retargeted link shows even with equal content');
    unlinkSync(link);
    symlinkSync('missing.md', link);
    assert.match(take('four'), /^symlink {2}skills\/linked\/SKILL\.md -> missing\.md {2}dangling$/m);
  });
});
