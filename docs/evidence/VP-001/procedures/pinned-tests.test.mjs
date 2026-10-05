// `node --test` coverage for `pinned-tests.sh`'s shared `pinned` selection
// helper: run exactly the named tests (--exact), require every one of them
// to pass, and gate on cargo's own exit status too, all behind a
// `gate=<gate-prefix>-selection-failed` line a VP-001 static step can grep
// for in its own artifact. Runs the real helper sourced by `bash` from a
// scratch temp dir, against a stub `cargo` placed first on PATH -- no real
// cargo, crate, or test binary involved, mirroring `with-webdriver.test.mjs`'s
// own stub approach.
//
// Skipped on win32 (bash plus POSIX `source`). Runs in CI on Linux, macOS,
// and Windows (ci.yml's `node --test docs/evidence/VP-001/procedures/*.test.mjs`
// step is unconditional), so this file's own assertions must also hold
// against macOS's bash 3.2 and BSD userland.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const HELPER_PATH = join(HERE, 'pinned-tests.sh');
const NOT_WINDOWS = process.platform === 'win32' ? { skip: 'pinned-tests.sh needs bash and POSIX `source` (not Windows)' } : {};

// A hard ceiling on every spawned case, so a wedged helper or stub fails its
// own test instead of hanging the suite. The stub `cargo` below never
// blocks, so this is slack, not a budget that ordinarily gets used.
const CHILD_TIMEOUT_MS = 15_000;

/** Single-quotes `value` for safe embedding as one argv word in a `bash -c`
 * script, the way the fixtures below hand `pinned`'s arguments to bash. */
function shQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

/**
 * Runs `source pinned-tests.sh; pinned <...pinnedArgs>` in a fresh `bash -c`
 * process, against a stub `cargo` written into this call's own scratch
 * `dir` and placed first on PATH, so the real cargo never runs. The stub
 * always touches `markerPath` (proof it ran) and, when `recordArgv` is set,
 * records its own argv one word per line to `argvPath` -- both inside this
 * same `dir`, returned alongside it so a case can inspect them directly,
 * with one `dir` and one cleanup per case. `cargoStdout` is printed
 * verbatim and `cargoStderr` (if any) is written to the stub's own stderr
 * before it exits `exitCode`; `pinned`'s own `2>&1` merges that stderr into
 * the captured `out` it decides and echoes from, mirroring how a real
 * build failure's stderr ends up in the helper's own stdout. Uses async
 * `spawn`, never `spawnSync`, and a `CHILD_TIMEOUT_MS` kill, mirroring
 * `with-webdriver.test.mjs`'s `runWrapper`.
 */
async function runPinned({ pinnedArgs, cargoStdout = '', cargoStderr = '', exitCode = 0, recordArgv = false }) {
  const dir = mkdtempSync(join(tmpdir(), 'pinned-tests-'));
  const markerPath = join(dir, 'cargo.ran');
  const argvPath = recordArgv ? join(dir, 'cargo.argv') : undefined;

  const lines = [`touch ${JSON.stringify(markerPath)}`];
  if (argvPath) {
    lines.push(`printf '%s\\n' "$@" > ${JSON.stringify(argvPath)}`);
  }
  if (cargoStderr) {
    lines.push(`printf '%s\\n' ${JSON.stringify(cargoStderr)} >&2`);
  }
  if (cargoStdout) {
    lines.push(`cat <<'CARGO_STUB_EOF'\n${cargoStdout}\nCARGO_STUB_EOF`);
  }
  lines.push(`exit ${exitCode}`);
  writeFileSync(join(dir, 'cargo'), `#!/usr/bin/env bash\n${lines.join('\n')}\n`, { mode: 0o755 });

  const script = `source ${shQuote(HELPER_PATH)}; pinned ${pinnedArgs.map(shQuote).join(' ')}`;
  const child = spawn('bash', ['-c', script], {
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}` },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    stdout += chunk;
  });
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
  const timer = setTimeout(() => child.kill('SIGKILL'), CHILD_TIMEOUT_MS);
  const status = await new Promise((resolve) => {
    child.on('close', (code) => resolve(code));
  });
  clearTimeout(timer);
  return { status, stdout, stderr, dir, markerPath, argvPath };
}

test('pinned-tests.sh', NOT_WINDOWS, async (t) => {
  await t.test('every named test passes: returns 0, and cargo\'s output is echoed', async () => {
    const run = await runPinned({
      cargoStdout: 'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
      pinnedArgs: ['mygate', '-p', 'demo', '--test', 'foo', '--', 'test_a', 'test_b'],
    });
    try {
      assert.equal(run.status, 0, run.stdout + run.stderr);
      assert.ok(existsSync(run.markerPath), 'cargo should have run');
      assert.ok(run.stdout.includes('test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s'), run.stdout);
      assert.ok(!run.stdout.includes('gate=mygate-selection-failed'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('fewer passed than names (a renamed test): returns 1 with the exact gate line', async () => {
    // The realistic shape under --exact: a renamed name matches nothing, so
    // it is filtered out, not failed -- cargo itself still exits 0.
    const run = await runPinned({
      cargoStdout: 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s',
      pinnedArgs: ['mygate', '-p', 'demo', '--test', 'foo', '--', 'test_a', 'test_b'],
    });
    try {
      assert.equal(run.status, 1, run.stdout + run.stderr);
      assert.ok(run.stdout.includes('gate=mygate-selection-failed expected=2 passed=1 status=0'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('a summation case: two "test result: ok." lines that sum to the expected count return 0', async () => {
    // cargo prints one "test result" line per test binary it runs; `-p
    // <pkg>` with no narrowing `--test`/`--lib` can match several of them
    // in one invocation, each reporting only the names that landed in it.
    const run = await runPinned({
      cargoStdout: [
        'running 2 tests',
        'test test_a ... ok',
        'test test_b ... ok',
        '',
        'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
        '',
        'running 1 test',
        'test test_c ... ok',
        '',
        'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
      ].join('\n'),
      pinnedArgs: ['mygate', '-p', 'demo', '--', 'test_a', 'test_b', 'test_c'],
    });
    try {
      assert.equal(run.status, 0, run.stdout + run.stderr);
      assert.ok(!run.stdout.includes('gate=mygate-selection-failed'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('a summation case: two "test result: ok." lines that fall short of the expected count return 1', async () => {
    const run = await runPinned({
      cargoStdout: [
        'running 2 tests',
        'test test_a ... ok',
        'test test_b ... ok',
        '',
        'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
        '',
        'running 0 tests',
        '',
        'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s',
      ].join('\n'),
      pinnedArgs: ['mygate', '-p', 'demo', '--', 'test_a', 'test_b', 'test_c'],
    });
    try {
      assert.equal(run.status, 1, run.stdout + run.stderr);
      assert.ok(run.stdout.includes('gate=mygate-selection-failed expected=3 passed=2 status=0'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('cargo exits non-zero: returns 1 with the gate line carrying status=', async () => {
    const run = await runPinned({
      cargoStderr: 'error[E0433]: failed to resolve',
      exitCode: 101,
      pinnedArgs: ['mygate', '-p', 'demo', '--test', 'foo', '--', 'test_a', 'test_b'],
    });
    try {
      assert.equal(run.status, 1, run.stdout + run.stderr);
      // stderr is merged by pinned's own `2>&1` before the pass/fail
      // decision, so the build failure's cause surfaces in stdout, the
      // text an uploaded artifact actually keeps.
      assert.ok(run.stdout.includes('error[E0433]: failed to resolve'), run.stdout);
      assert.ok(run.stdout.includes('gate=mygate-selection-failed expected=2 passed=0 status=101'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('usage error: no `--` among the arguments returns 2, and cargo never runs', async () => {
    const run = await runPinned({
      cargoStdout: 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
      pinnedArgs: ['mygate', '-p', 'demo'],
    });
    try {
      assert.equal(run.status, 2, run.stdout + run.stderr);
      assert.ok(run.stderr.includes('usage:'), run.stderr);
      assert.ok(!existsSync(run.markerPath), 'cargo should never have run');
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('usage error: no test names after `--` returns 2, and cargo never runs', async () => {
    const run = await runPinned({
      cargoStdout: 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
      pinnedArgs: ['mygate', '-p', 'demo', '--'],
    });
    try {
      assert.equal(run.status, 2, run.stdout + run.stderr);
      assert.ok(run.stderr.includes('usage:'), run.stderr);
      assert.ok(!existsSync(run.markerPath), 'cargo should never have run');
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('the stub records its argv: it receives `<args> -- --exact <names>` in order', async () => {
    const run = await runPinned({
      cargoStdout: 'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
      pinnedArgs: ['mygate', '-p', 'demo', '--test', 'foo', '--release', '--', 'name_one', 'name_two'],
      recordArgv: true,
    });
    try {
      assert.equal(run.status, 0, run.stdout + run.stderr);
      const argv = readFileSync(run.argvPath, 'utf8').split('\n').filter((line) => line.length > 0);
      assert.deepEqual(argv, ['test', '-p', 'demo', '--test', 'foo', '--release', '--', '--exact', 'name_one', 'name_two']);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });

  await t.test('the gate prefix is used verbatim, not a fixed string', async () => {
    const run = await runPinned({
      cargoStdout: 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s',
      pinnedArgs: ['static-identity', '-p', 'omnifrons-app', '--test', 'launch_gate', '--', 'some_renamed_test'],
    });
    try {
      assert.equal(run.status, 1, run.stdout + run.stderr);
      assert.ok(run.stdout.includes('gate=static-identity-selection-failed expected=1 passed=0 status=0'), run.stdout);
      assert.ok(!run.stdout.includes('gate=static-boundary-selection-failed'), run.stdout);
    } finally {
      rmSync(run.dir, { recursive: true, force: true });
    }
  });
});
