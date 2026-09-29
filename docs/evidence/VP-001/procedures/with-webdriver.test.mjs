// `node --test` coverage for `with-webdriver.sh`'s driver bootstrap: start
// the driver, wait for it to answer /status, run the wrapped command, and
// always stop the driver. Runs the real wrapper with `bash` from a scratch
// temp dir, against stub driver scripts and an in-process `node:http`
// status server -- no real tauri-driver, WebKitWebDriver, or display
// involved, mirroring `vp-s1-linux.test.mjs`'s own stub approach.
//
// Skipped on win32 (bash plus POSIX signals). Runs in CI on Linux, macOS,
// and Windows (ci.yml's `node --test docs/evidence/VP-001/procedures/*.test.mjs`
// step is unconditional), so this file's own assertions must also hold
// against macOS's bash 3.2 and BSD userland.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const NOT_WINDOWS =
  process.platform === 'win32' ? { skip: 'with-webdriver.sh needs bash and POSIX signals (not Windows)' } : {};

// A hard ceiling on every spawned wrapper, so a wedged driver or command
// fails its own test instead of hanging the suite. Comfortably above the
// two-poll, 1 s-apart readiness wait every case below sets via
// VP001_DRIVER_READY_POLLS.
const CHILD_TIMEOUT_MS = 15_000;

/** An in-process HTTP server answering every request 200 on an ephemeral
 * port, after `delayMs` -- the "driver is ready" fixture. */
function startStatusServer(delayMs = 0) {
  const server = http.createServer((_req, res) => {
    setTimeout(() => {
      res.writeHead(200);
      res.end();
    }, delayMs);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ baseUrl: `http://127.0.0.1:${port}`, close: () => new Promise((r) => server.close(r)) });
    });
  });
}

/** A base URL nothing listens on: connection-refused, the plainest "never
 * ready" case. Binds an ephemeral port, then frees it immediately. */
async function closedPortBaseUrl() {
  const server = http.createServer();
  const port = await new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve(server.address().port));
  });
  await new Promise((r) => server.close(r));
  return `http://127.0.0.1:${port}`;
}

/** Polls until `pid` is no longer a live process, or `deadlineMs` elapses.
 * The wrapper's EXIT trap only signals the driver -- it does not itself
 * wait for it to die -- so a brief poll after the wrapper exits is needed
 * before asserting the driver is gone. */
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

/** A command argv that writes an empty file at `markerPath` and exits
 * `exitCode` -- run through `with-webdriver.sh`'s `"$@"` so its own
 * pass-through exit code can be checked, and so "the command never ran"
 * can be checked by the marker's absence. */
function markerCommand(markerPath, exitCode = 0) {
  return [process.execPath, '-e', `require("fs").writeFileSync(${JSON.stringify(markerPath)}, ""); process.exit(${exitCode});`];
}

/**
 * Runs `with-webdriver.sh <command...>` from a scratch temp dir, with
 * `driverBody` standing in for `tauri-driver` (a stub script written next
 * to the real wrapper, mirroring `vp-s1-linux.test.mjs`'s `runWithAvStub`).
 * Uses async `spawn`, never `spawnSync`: the "ready" case's status server
 * runs in this very process, and a synchronous, blocking spawn would starve
 * its event loop, so the server could never answer the wrapper's `curl`
 * call (`scenario-e2e.test.mjs`'s own fake WebDriver server is async for
 * the same reason). Bounded by `CHILD_TIMEOUT_MS`, so a wedged wrapper
 * fails its own test instead of hanging the suite. Returns the temp `dir`
 * too, so a case can inspect files the stub or the command left behind
 * before cleaning up itself.
 */
async function runWrapper({ driverBody, baseUrl, command }) {
  const dir = mkdtempSync(join(tmpdir(), 'with-webdriver-'));
  copyFileSync(join(HERE, 'with-webdriver.sh'), join(dir, 'with-webdriver.sh'));
  const driverPath = join(dir, 'tauri-driver-stub.sh');
  writeFileSync(driverPath, `#!/usr/bin/env bash\n${driverBody}\n`, { mode: 0o755 });
  const child = spawn('bash', [join(dir, 'with-webdriver.sh'), ...command], {
    env: {
      ...process.env,
      VP001_TAURI_DRIVER: driverPath,
      VP001_WEBDRIVER_BASE_URL: baseUrl,
      VP001_DRIVER_READY_POLLS: '2',
    },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    stdout += chunk;
  });
  const timer = setTimeout(() => child.kill('SIGKILL'), CHILD_TIMEOUT_MS);
  const status = await new Promise((resolve) => {
    child.on('close', (code) => resolve(code));
  });
  clearTimeout(timer);
  return { status, stdout, dir };
}

test('with-webdriver.sh', NOT_WINDOWS, async (t) => {
  await t.test('ready: waits for /status, runs the command, passes its exit code through, and stops the driver', async () => {
    const statusServer = await startStatusServer();
    let dir;
    try {
      dir = mkdtempSync(join(tmpdir(), 'with-webdriver-marker-'));
      const markerPath = join(dir, 'command.ran');
      const pidFile = join(dir, 'driver.pid');
      const run = await runWrapper({
        // Stays alive (like a real tauri-driver would) so liveness alone
        // never explains readiness; the stub's own pid -- unchanged by
        // `exec`, which replaces the process image but keeps its pid -- is
        // recorded so the driver's teardown can be checked afterward.
        driverBody: `echo "$$" > ${JSON.stringify(pidFile)}\nexec sleep 30`,
        baseUrl: statusServer.baseUrl,
        command: markerCommand(markerPath, 3),
      });
      try {
        assert.equal(run.status, 3, run.stdout); // the wrapped command's own exit code, unchanged
        assert.ok(existsSync(markerPath), 'the command should have run');
        assert.ok(!run.stdout.includes('gate=driver-not-ready'));
        assert.ok(!run.stdout.includes('gate=driver-exited'));
        const driverPid = Number(readFileSync(pidFile, 'utf8').trim());
        assert.ok(await eventuallyDead(driverPid), 'the driver should be stopped once the wrapper exits');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
      await statusServer.close();
    }
  });

  await t.test('driver exits at once: named gate=driver-exited with its own status, and the command never runs', async () => {
    const baseUrl = await closedPortBaseUrl(); // never reached: liveness is checked before any status poll
    let dir;
    try {
      dir = mkdtempSync(join(tmpdir(), 'with-webdriver-marker-'));
      const markerPath = join(dir, 'command.ran');
      const run = await runWrapper({
        driverBody: 'exit 7',
        baseUrl,
        command: markerCommand(markerPath),
      });
      try {
        assert.equal(run.status, 1, run.stdout);
        assert.ok(run.stdout.includes('gate=driver-exited status=7'), run.stdout);
        assert.ok(!run.stdout.includes('gate=driver-not-ready'), run.stdout);
        assert.ok(!existsSync(markerPath), 'the command should never have run');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('never ready: named gate=driver-not-ready, the command never runs, and the driver is stopped', async () => {
    const baseUrl = await closedPortBaseUrl();
    let dir;
    try {
      dir = mkdtempSync(join(tmpdir(), 'with-webdriver-marker-'));
      const markerPath = join(dir, 'command.ran');
      const pidFile = join(dir, 'driver.pid');
      const run = await runWrapper({
        driverBody: `echo "$$" > ${JSON.stringify(pidFile)}\nexec sleep 30`,
        baseUrl,
        command: markerCommand(markerPath),
      });
      try {
        assert.equal(run.status, 1, run.stdout);
        assert.ok(run.stdout.includes('gate=driver-not-ready'), run.stdout);
        assert.ok(!run.stdout.includes('gate=driver-exited'), run.stdout);
        assert.ok(!existsSync(markerPath), 'the command should never have run');
        const driverPid = Number(readFileSync(pidFile, 'utf8').trim());
        assert.ok(await eventuallyDead(driverPid), 'the driver should be stopped once the wrapper gives up');
      } finally {
        rmSync(run.dir, { recursive: true, force: true });
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  await t.test('a /status that accepts but never answers still ends in gate=driver-not-ready, well inside the child bound', async () => {
    // Without a per-poll bound, one such poll would block the wrapper for good.
    const server = http.createServer(() => {});
    const port = await new Promise((resolve) => {
      server.listen(0, '127.0.0.1', () => resolve(server.address().port));
    });
    try {
      const run = await runWrapper({
        driverBody: 'exec sleep 30',
        baseUrl: `http://127.0.0.1:${port}`,
        command: [process.execPath, '-e', 'process.exit(0)'],
      });
      rmSync(run.dir, { recursive: true, force: true });
      assert.equal(run.status, 1, run.stdout);
      // A wrapper blocked for good would be SIGKILLed by the child bound: status null, not 1.
      assert.ok(run.stdout.includes('gate=driver-not-ready'), run.stdout);
    } finally {
      await new Promise((r) => server.close(r).closeAllConnections());
    }
  });

  await t.test('a driver that dies while another listener answers /status is named driver-exited, and the command never runs', async () => {
    // Alive at the first poll, dead (0.3 s) before the other listener's answer (0.6 s) arrives.
    const statusServer = await startStatusServer(600);
    const dir = mkdtempSync(join(tmpdir(), 'with-webdriver-marker-'));
    try {
      const markerPath = join(dir, 'command.ran');
      const run = await runWrapper({ driverBody: 'sleep 0.3\nexit 9', baseUrl: statusServer.baseUrl, command: markerCommand(markerPath) });
      rmSync(run.dir, { recursive: true, force: true });
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=driver-exited status=9'), run.stdout);
      assert.ok(!existsSync(markerPath), 'the command should never have run');
    } finally {
      rmSync(dir, { recursive: true, force: true });
      await statusServer.close();
    }
  });
});
