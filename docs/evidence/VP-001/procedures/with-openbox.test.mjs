// `node --test` coverage for `with-openbox.sh`'s window-manager bootstrap:
// start openbox, wait for it to take the window-manager selection, run the
// wrapped command, and always stop openbox. Runs the real wrapper with
// `bash` from a scratch temp dir, against stub `openbox` and `xprop`
// executables placed on PATH -- the script calls both by their bare names,
// never through an env override, so PATH is the only seam (with-webdriver.sh
// stubs `tauri-driver` through an env var instead, because it does support
// that override; see with-webdriver.test.mjs).
//
// Skipped on win32 (bash plus POSIX signals). Runs in CI on Linux and
// macOS (ci.yml's `node --test docs/evidence/VP-001/procedures/*.test.mjs`
// step is unconditional), so this file's own assertions must also hold
// against macOS's bash 3.2 and BSD userland.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const NOT_WINDOWS =
  process.platform === 'win32'
    ? { skip: 'with-openbox.sh needs bash, POSIX signals, and fake openbox/xprop binaries on PATH (not Windows)' }
    : {};

// A hard ceiling on every spawned wrapper, so a wedged openbox or command
// fails its own test instead of hanging the suite. Comfortably above the
// slowest case below, each of which is bounded by its own readyPolls x
// VP001_WM_POLL_INTERVAL (1 s by default here) plus process start-up --
// never restated as a fixed count here, so this bound cannot quietly drift
// out of sync with whatever a case actually sets.
const CHILD_TIMEOUT_MS = 15_000;

/** A command argv that writes an empty file at `markerPath` and exits
 * `exitCode` -- run through `with-openbox.sh`'s `"$@"` so its own
 * pass-through exit code can be checked, and so "the command never ran"
 * can be checked by the marker's absence (mirrors with-webdriver.test.mjs's
 * own `markerCommand`). */
function markerCommand(markerPath, exitCode = 0) {
  return [process.execPath, '-e', `require("fs").writeFileSync(${JSON.stringify(markerPath)}, ""); process.exit(${exitCode});`];
}

/** A command argv that writes its own received arguments (JSON-encoded,
 * exactly as `process.argv.slice(1)` sees them -- verified empirically:
 * `node -e <script> a b` hands the eval'd code `process.argv = [node, a,
 * b]`, with no extra placeholder slot for the `-e` script itself) to
 * `markerPath`, then exits 0. Lets a test check that with-openbox.sh's
 * `"$@"` passes a wrapped command's own arguments through unchanged,
 * including ones a naive split or re-quote would mangle. */
function argvMarkerCommand(markerPath, extraArgs) {
  return [
    process.execPath,
    '-e',
    `require("fs").writeFileSync(${JSON.stringify(markerPath)}, JSON.stringify(process.argv.slice(1)));`,
    ...extraArgs,
  ];
}

/** Polls until `pid` is no longer a live process, or `deadlineMs` elapses.
 * The wrapper's EXIT trap only signals openbox -- it does not itself wait
 * for it to die -- so a brief poll after the wrapper exits is needed before
 * asserting openbox is gone (mirrors with-webdriver.test.mjs's own
 * `eventuallyDead`). */
async function eventuallyDead(pid, deadlineMs = 5_000) {
  const deadline = Date.now() + deadlineMs;
  for (;;) {
    try {
      process.kill(pid, 0);
    } catch {
      return true; // ESRCH: the process is gone
    }
    if (Date.now() > deadline) return false;
    await new Promise((r) => setTimeout(r, 20));
  }
}

// Real xprop exits 0 for `-root _NET_SUPPORTING_WM_CHECK` even when that
// property is absent -- which is exactly what made the inline openbox/xprop
// readiness loops this change removes from .github/workflows/tauri-build.yml
// vacuous: they checked xprop's exit status alone and so could never tell
// ready from not-ready. These fakes model the real output, not just a
// plausible exit code, so with-openbox.sh's own output-based check (its
// header comment) is exercised against the same ambiguity the real binary
// presents.
const XPROP_NOT_READY_OUTPUT = '_NET_SUPPORTING_WM_CHECK:  no such atom on any window.';
const XPROP_READY_OUTPUT = '_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0x200001';

/**
 * Runs `with-openbox.sh <command...>` from a scratch temp dir, with fake
 * `openbox` and `xprop` executables written into a bin dir prepended to
 * PATH -- `openboxBody` and `xpropBody` stand in for the real binaries.
 * `readyPolls` and `pollInterval` set VP001_WM_READY_POLLS/
 * VP001_WM_POLL_INTERVAL; either may be omitted (`undefined`) to leave the
 * script's own default, or set to a deliberately invalid string. Uses
 * async `spawn`, never `spawnSync`, bounded by `CHILD_TIMEOUT_MS`, so a
 * wedged wrapper fails its own test instead of hanging the suite. Returns
 * the temp `dir` too, so a case can inspect files the stub or the command
 * left behind before cleaning up itself.
 */
async function runWrapper({ openboxBody, xpropBody, command, readyPolls = '3', pollInterval }) {
  const dir = mkdtempSync(join(tmpdir(), 'with-openbox-'));
  const binDir = join(dir, 'bin');
  mkdirSync(binDir);
  copyFileSync(join(HERE, 'with-openbox.sh'), join(dir, 'with-openbox.sh'));
  writeFileSync(join(binDir, 'openbox'), `#!/usr/bin/env bash\n${openboxBody}\n`, { mode: 0o755 });
  writeFileSync(join(binDir, 'xprop'), `#!/usr/bin/env bash\n${xpropBody}\n`, { mode: 0o755 });
  const env = {
    ...process.env,
    PATH: `${binDir}${delimiter}${process.env.PATH}`,
  };
  if (readyPolls !== undefined) env.VP001_WM_READY_POLLS = readyPolls;
  if (pollInterval !== undefined) env.VP001_WM_POLL_INTERVAL = pollInterval;
  const child = spawn('bash', [join(dir, 'with-openbox.sh'), ...command], { env, stdio: ['ignore', 'pipe', 'pipe'] });
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
  return { status, stdout, stderr, dir };
}

test('with-openbox.sh', NOT_WINDOWS, async (t) => {
  await t.test('ready on the first poll: prints gate=wm-ready, runs the command, passes its exit code through, and stops openbox', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const markerPath = join(dir, 'command.ran');
      const pidFile = join(dir, 'openbox.pid');
      const run = await runWrapper({
        // Stays alive (like a real openbox would) so liveness alone never
        // explains readiness; the stub's own pid -- unchanged by `exec`,
        // which replaces the process image but keeps its pid -- is
        // recorded so openbox's teardown can be checked afterward.
        openboxBody: `echo "$$" > ${JSON.stringify(pidFile)}\nexec sleep 30`,
        xpropBody: `echo ${JSON.stringify(XPROP_READY_OUTPUT)}`,
        command: markerCommand(markerPath, 0),
      });
      try {
        assert.equal(run.status, 0, run.stdout); // the wrapped command's own exit code, unchanged
        assert.ok(run.stdout.includes('gate=wm-ready'), run.stdout);
        assert.ok(existsSync(markerPath), 'the command should have run');
        assert.ok(!run.stdout.includes('gate=wm-not-ready'), run.stdout);
        assert.ok(!run.stdout.includes('gate=wm-exited'), run.stdout);
        const pid = Number(readFileSync(pidFile, 'utf8').trim());
        assert.ok(await eventuallyDead(pid), 'openbox should be stopped once the wrapper exits');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('ready on a later poll: still runs the command, passes a non-zero exit code through, and stops openbox', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const markerPath = join(dir, 'command.ran');
      const counterFile = join(dir, 'xprop.calls');
      const pidFile = join(dir, 'openbox.pid');
      const run = await runWrapper({
        openboxBody: `echo "$$" > ${JSON.stringify(pidFile)}\nexec sleep 30`,
        // Reports not-ready the first call, ready from the second call on
        // -- always exiting 0, like the real binary -- so readiness
        // genuinely comes from a later poll's output, not the first one.
        xpropBody:
          `count=0\n[[ -f ${JSON.stringify(counterFile)} ]] && count=$(cat ${JSON.stringify(counterFile)})\n` +
          `count=$((count + 1))\necho "$count" > ${JSON.stringify(counterFile)}\n` +
          `if [[ "$count" -ge 2 ]]; then echo ${JSON.stringify(XPROP_READY_OUTPUT)}; else echo ${JSON.stringify(XPROP_NOT_READY_OUTPUT)}; fi`,
        command: markerCommand(markerPath, 5),
        readyPolls: '3',
      });
      try {
        assert.equal(run.status, 5, run.stdout); // the wrapped command's own exit code, unchanged
        assert.ok(run.stdout.includes('gate=wm-ready'), run.stdout);
        assert.ok(existsSync(markerPath), 'the command should have run');
        const pid = Number(readFileSync(pidFile, 'utf8').trim());
        assert.ok(await eventuallyDead(pid), 'openbox should be stopped once the wrapper exits, even on a non-zero command exit');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('never ready: named gate=wm-not-ready, the command never runs, and openbox is stopped', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const markerPath = join(dir, 'command.ran');
      const pidFile = join(dir, 'openbox.pid');
      const run = await runWrapper({
        openboxBody: `echo "$$" > ${JSON.stringify(pidFile)}\nexec sleep 30`,
        xpropBody: `echo ${JSON.stringify(XPROP_NOT_READY_OUTPUT)}`,
        command: markerCommand(markerPath),
        readyPolls: '2',
      });
      try {
        assert.equal(run.status, 1, run.stdout);
        assert.ok(run.stdout.includes('gate=wm-not-ready'), run.stdout);
        assert.ok(!run.stdout.includes('gate=wm-exited'), run.stdout);
        assert.ok(!existsSync(markerPath), 'the command should never have run');
        const pid = Number(readFileSync(pidFile, 'utf8').trim());
        assert.ok(await eventuallyDead(pid), 'openbox should be stopped once the wrapper gives up');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('openbox exits before ready: named gate=wm-exited with its own status, and the command never runs', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const markerPath = join(dir, 'command.ran');
      const run = await runWrapper({
        openboxBody: 'exit 7',
        xpropBody: `echo ${JSON.stringify(XPROP_NOT_READY_OUTPUT)}`,
        command: markerCommand(markerPath),
      });
      try {
        assert.equal(run.status, 1, run.stdout);
        assert.ok(run.stdout.includes('gate=wm-exited status=7'), run.stdout);
        assert.ok(!run.stdout.includes('gate=wm-not-ready'), run.stdout);
        assert.ok(!existsSync(markerPath), 'the command should never have run');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('usage error: no command given prints the usage line on stderr, exits 2, and never starts openbox', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const openboxStarted = join(dir, 'openbox.started');
      const run = await runWrapper({
        // Would prove it ran by touching a marker -- never reached, since
        // the usage check must fail before `openbox &`.
        openboxBody: `touch ${JSON.stringify(openboxStarted)}\nexec sleep 30`,
        xpropBody: `echo ${JSON.stringify(XPROP_READY_OUTPUT)}`,
        command: [],
      });
      try {
        assert.equal(run.status, 2, run.stdout + run.stderr);
        assert.ok(run.stderr.includes('usage: with-openbox.sh <command> [args...]'), run.stderr);
        assert.ok(!existsSync(openboxStarted), 'openbox should never have started');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test("the wrapped command's own arguments pass through unchanged, including one with a space and one empty argument", async () => {
    const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
    try {
      const argvPath = join(dir, 'argv.json');
      const extraArgs = ['has space', ''];
      const run = await runWrapper({
        openboxBody: 'exec sleep 30',
        xpropBody: `echo ${JSON.stringify(XPROP_READY_OUTPUT)}`,
        command: argvMarkerCommand(argvPath, extraArgs),
      });
      try {
        assert.equal(run.status, 0, run.stdout + run.stderr);
        assert.deepStrictEqual(JSON.parse(readFileSync(argvPath, 'utf8')), extraArgs);
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('an invalid VP001_WM_READY_POLLS (0, or non-numeric) is a usage error: exit 2, before openbox ever starts', async () => {
    for (const invalidPolls of ['0', 'abc']) {
      const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
      try {
        const openboxStarted = join(dir, 'openbox.started');
        const run = await runWrapper({
          openboxBody: `touch ${JSON.stringify(openboxStarted)}\nexec sleep 30`,
          xpropBody: `echo ${JSON.stringify(XPROP_READY_OUTPUT)}`,
          command: markerCommand(join(dir, 'command.ran')),
          readyPolls: invalidPolls,
        });
        try {
          assert.equal(run.status, 2, `polls=${invalidPolls}: ${run.stdout}${run.stderr}`);
          assert.ok(run.stderr.includes('VP001_WM_READY_POLLS'), `polls=${invalidPolls}: ${run.stderr}`);
          assert.ok(!existsSync(openboxStarted), `polls=${invalidPolls}: openbox should never have started`);
        } finally {
          rmSync(run.dir, { recursive: true, force: true });
        }
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    }
  });

  await t.test('an invalid VP001_WM_POLL_INTERVAL (0, or non-numeric) is a usage error: exit 2, before openbox ever starts', async () => {
    for (const invalidInterval of ['0', '0.0', 'abc']) {
      const dir = mkdtempSync(join(tmpdir(), 'with-openbox-marker-'));
      try {
        const openboxStarted = join(dir, 'openbox.started');
        const run = await runWrapper({
          openboxBody: `touch ${JSON.stringify(openboxStarted)}\nexec sleep 30`,
          xpropBody: `echo ${JSON.stringify(XPROP_READY_OUTPUT)}`,
          command: markerCommand(join(dir, 'command.ran')),
          pollInterval: invalidInterval,
        });
        try {
          assert.equal(run.status, 2, `interval=${invalidInterval}: ${run.stdout}${run.stderr}`);
          assert.ok(run.stderr.includes('VP001_WM_POLL_INTERVAL'), `interval=${invalidInterval}: ${run.stderr}`);
          assert.ok(!existsSync(openboxStarted), `interval=${invalidInterval}: openbox should never have started`);
        } finally {
          rmSync(run.dir, { recursive: true, force: true });
        }
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    }
  });
});
