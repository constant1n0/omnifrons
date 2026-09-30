// End-to-end tests for `vp-s1-scenario.mjs` and `vp-s3-scenario.mjs`: each
// script is run as a real child process, `stdout` collected in full,
// against an in-process fake WebDriver endpoint (`node:http`, no real
// `tauri-driver` or packaged app). `scenario-session.test.mjs` already
// exercises `runScenarioSession` and `withDeadline` in-process; this file is
// the one place that exercises the scripts themselves -- argv parsing, the
// shared session lifecycle wired to a real spawn, and the two probes' own
// format functions applied to a fake `read` result. The scripts' own
// `key=value` transcript is what `derive_csp`/`derive_ipc`
// (`tools/evidence-validator`) parse, so its exact shape is the contract
// pinned below with `assert.deepEqual`, never a substring check.
//
// The fake's `read` result for each scenario carries a realistic
// `securitypolicyviolation` event (and, for VP-S3, a fetch entry), so the
// violation/fetch/observation formatting paths all appear in the
// transcript, not just the empty case. Every expected line below is
// computed by hand against `vp-s1-probe.mjs`/`vp-s3-probe.mjs`'s own logic,
// never by calling `summarize`/`formatObservations`/... from this file --
// that would test the probe against itself.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import http from 'node:http';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const VP_S1_SCRIPT = join(__dirname, 'vp-s1-scenario.mjs');
const VP_S3_SCRIPT = join(__dirname, 'vp-s3-scenario.mjs');

// A hard ceiling on every spawned child, so a wedged script fails its own
// test instead of hanging the suite. Comfortably above both scenarios' own
// real 2 s SETTLE_MS wait between 'armed' and the read.
const CHILD_TIMEOUT_MS = 15_000;

/**
 * An in-process fake WebDriver endpoint on 127.0.0.1:0, one per test.
 * Answers `POST /session`, `POST /session/<id>/execute/sync` (arm, then
 * read), and `DELETE /session/<id>`, and records every request as
 * `<METHOD> <path>` so a test can assert the DELETE count. `behavior`
 * selects one misbehavior ('create-500', 'exec-500', 'delete-500', 'hang');
 * anything else is the happy path. `readResult` is returned verbatim from
 * the second `execute/sync` call (the probe's read); the first always
 * answers `null` -- the arm script's own return value is never used by the
 * scenario scripts.
 */
function startFakeWebDriver({ behavior = 'ok', readResult = {} } = {}) {
  const requests = [];
  let execCount = 0;
  const server = http.createServer((req, res) => {
    req.on('data', () => {}); // drain the request body; its content is unused
    req.on('end', () => {
      requests.push(`${req.method} ${req.url}`);
      const send = (status, value) => {
        res.writeHead(status, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ value }));
      };

      if (req.method === 'POST' && req.url === '/session') {
        if (behavior === 'create-500') {
          return send(500, { error: 'session not created', message: 'create failed' });
        }
        return send(200, { sessionId: 'sess-1', capabilities: {} });
      }

      if (req.method === 'POST' && req.url.endsWith('/execute/sync')) {
        execCount += 1;
        if (behavior === 'hang') return; // never respond -- the SIGTERM case
        if (behavior === 'exec-500' && execCount === 2) {
          return send(500, { error: 'javascript error', message: 'read failed' });
        }
        return send(200, execCount === 1 ? null : readResult);
      }

      if (req.method === 'DELETE') {
        if (behavior === 'delete-500') {
          return send(500, { error: 'unknown error', message: 'delete failed' });
        }
        return send(200, null);
      }

      send(404, { error: 'unknown command', message: req.url });
    });
  });

  return new Promise((resolveFake) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolveFake({
        baseUrl: `http://127.0.0.1:${port}`,
        requests,
        deleteCount: () => requests.filter((r) => r.startsWith('DELETE')).length,
        // Also drops a connection held by a still-pending request (the 'hang' case).
        close: () => new Promise((resolveClose) => server.close(resolveClose).closeAllConnections()),
      });
    });
  });
}

/**
 * Spawns `script` with `args`, buffering complete stdout lines as they
 * arrive so a caller can wait on one mid-run (the SIGTERM case) as well as
 * on exit. `stdin` is unused; `stderr` is inherited so an unexpected
 * script-side failure is visible in the test run's own output, not silently
 * swallowed.
 */
function spawnScript(script, args) {
  const child = spawn(process.execPath, [script, ...args], { stdio: ['ignore', 'pipe', 'inherit'] });
  const lines = [];
  let buffer = '';
  child.stdout.setEncoding('utf8'); // never split a multi-byte character across chunks
  child.stdout.on('data', (chunk) => {
    buffer += chunk;
    const parts = buffer.split('\n');
    buffer = parts.pop();
    lines.push(...parts);
  });
  const closed = new Promise((resolveClosed) => {
    child.on('close', (code, signal) => {
      if (buffer !== '') lines.push(buffer); // a final line without its newline is still reported
      resolveClosed({ code, signal });
    });
  });
  return { child, lines, closed };
}

/** Runs `script` to completion against the fake endpoint, bounded by `CHILD_TIMEOUT_MS`. */
async function run(script, args) {
  const { child, lines, closed } = spawnScript(script, args);
  const timer = setTimeout(() => child.kill('SIGKILL'), CHILD_TIMEOUT_MS);
  try {
    const { code } = await closed;
    return { lines, code };
  } finally {
    clearTimeout(timer);
  }
}

/** Polls `lines` until it contains `line`, for the SIGTERM case, which must
 * act at a precise point in the transcript rather than after exit. */
async function waitForLine(lines, line) {
  const deadline = Date.now() + CHILD_TIMEOUT_MS;
  while (!lines.includes(line)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for line: ${line}`);
    await new Promise((r) => setTimeout(r, 10));
  }
}

// -- VP-S1 fixtures --

const VP_S1_READ_RESULT = {
  violations: [
    {
      effectiveDirective: 'script-src-elem',
      violatedDirective: 'script-src-elem',
      blockedURI: 'inline',
      disposition: 'enforce',
      originalPolicy: "default-src 'none'; script-src 'self'",
      sourceFile: 'tauri://localhost/index.html',
      lineNumber: 12,
      sample: 'window.__vpS1',
    },
  ],
  attempts: [],
  location: { protocol: 'tauri:', host: 'localhost' },
  metaCsp: null,
  inlineRan: false,
  externalFetchSettled: 'rejected:TypeError',
  iframeDocumentUrl: null,
};

const VP_S1_OK_LINES = [
  'gate=session-created session_id=sess-1',
  'gate=armed',
  'gate=location-scheme value=tauri:',
  `observation=violation index=0 effectiveDirective="script-src-elem" violatedDirective="script-src-elem" blockedURI="inline" disposition="enforce" originalPolicy="default-src 'none'; script-src 'self'" sourceFile="tauri://localhost/index.html" lineNumber=12 sample="window.__vpS1"`,
  'observation=location_scheme value=tauri:',
  'observation=meta_csp_present value=false',
  "observation=policy_dump value=default-src 'none'; script-src 'self'",
  'observation=inline_script_violation value=true',
  'observation=inline_script_ran value=false',
  'observation=external_fetch_violation value=false',
  'observation=external_fetch_settled value=rejected:TypeError',
  'observation=framed_context_violation value=false',
  'observation=iframe_document_url value=null',
  'observation=violation_count value=1',
  'observation=third_party_surface_exercised value=false',
  'gate=session-closed session_id=sess-1',
];

const VP_S1_EXEC_FAIL_LINES = [
  'gate=session-created session_id=sess-1',
  'gate=armed',
  'blocker=unexpected-error: read failed',
  'gate=session-closed session_id=sess-1',
];

const VP_S1_DELETE_FAIL_LINES = [...VP_S1_OK_LINES.slice(0, -1), 'blocker=session-close-failed: delete failed'];
const VP_S1_CREATE_FAIL_LINES = ['blocker=session-create-failed: create failed'];
const VP_S1_USAGE_LINE = 'blocker=usage: vp-s1-scenario.mjs <baseUrl> <applicationPath>';

// -- VP-S3 fixtures --

const VP_S3_READ_RESULT = {
  violations: [
    {
      effectiveDirective: 'connect-src',
      violatedDirective: 'connect-src',
      blockedURI: 'ipc://localhost/shell_health',
      disposition: 'enforce',
      originalPolicy: "default-src 'none'; connect-src ipc:",
      sourceFile: 'tauri://localhost/index.html',
      lineNumber: 7,
      sample: '',
    },
  ],
  fetches: [{ url: 'ipc://localhost/shell_health', outcome: 'resolved:200' }],
  attempts: [
    { name: 'registered-call', outcome: 'resolved' },
    { name: 'unregistered-call', outcome: 'rejected:TypeError' },
    { name: 'artifact-load', outcome: 'error' },
    { name: 'policy-probe', outcome: 'resolved:0' },
  ],
  tauriInternalsPresent: true,
  consoleWarnFallbackSeen: false,
  postMessageWrapped: false,
  postMessageCallCount: 0,
  location_protocol: 'tauri:',
};

const VP_S3_OK_LINES = [
  'gate=session-created session_id=sess-1',
  'gate=armed',
  'gate=location-scheme value=tauri:',
  `observation=violation index=0 effectiveDirective="connect-src" violatedDirective="connect-src" blockedURI="ipc://localhost/shell_health" disposition="enforce" originalPolicy="default-src 'none'; connect-src ipc:" sourceFile="tauri://localhost/index.html" lineNumber=7 sample=""`,
  'observation=fetch index=0 url="ipc://localhost/shell_health" outcome="resolved:200"',
  'observation=attempt index=0 name="registered-call" outcome="resolved"',
  'observation=attempt index=1 name="unregistered-call" outcome="rejected:TypeError"',
  'observation=attempt index=2 name="artifact-load" outcome="error"',
  'observation=attempt index=3 name="policy-probe" outcome="resolved:0"',
  'observation=location_scheme value=tauri:',
  'observation=location_scheme_is_app_protocol value=true',
  'observation=connect_src_policy_captured value=true',
  'observation=connect_src_is_documented_source_for_os value=true',
  'observation=registered_call_completed value=true',
  'observation=registered_call_used_custom_protocol value=true',
  'observation=postmessage_fallback_observed value=false',
  'observation=bridge_connect_src_violation value=true',
  'observation=unregistered_call_rejected value=true',
  'observation=unregistered_call_resolved value=false',
  'observation=artifact_load_attempted value=true',
  'observation=artifact_load_succeeded value=false',
  'gate=session-closed session_id=sess-1',
];

const VP_S3_EXEC_FAIL_LINES = [
  'gate=session-created session_id=sess-1',
  'gate=armed',
  'blocker=unexpected-error: read failed',
  'gate=session-closed session_id=sess-1',
];

const VP_S3_DELETE_FAIL_LINES = [...VP_S3_OK_LINES.slice(0, -1), 'blocker=session-close-failed: delete failed'];
const VP_S3_CREATE_FAIL_LINES = ['blocker=session-create-failed: create failed'];
const VP_S3_USAGE_LINE = 'blocker=usage: vp-s3-scenario.mjs <baseUrl> <applicationPath> <os> (os: linux|macos|windows)';

const SCENARIOS = [
  {
    label: 'vp-s1-scenario.mjs',
    script: VP_S1_SCRIPT,
    argsFor: (baseUrl) => [baseUrl, '/app'],
    readResult: VP_S1_READ_RESULT,
    okLines: VP_S1_OK_LINES,
    execFailLines: VP_S1_EXEC_FAIL_LINES,
    deleteFailLines: VP_S1_DELETE_FAIL_LINES,
    createFailLines: VP_S1_CREATE_FAIL_LINES,
    usageLine: VP_S1_USAGE_LINE,
  },
  {
    label: 'vp-s3-scenario.mjs',
    script: VP_S3_SCRIPT,
    argsFor: (baseUrl) => [baseUrl, '/app', 'linux'],
    readResult: VP_S3_READ_RESULT,
    okLines: VP_S3_OK_LINES,
    execFailLines: VP_S3_EXEC_FAIL_LINES,
    deleteFailLines: VP_S3_DELETE_FAIL_LINES,
    createFailLines: VP_S3_CREATE_FAIL_LINES,
    usageLine: VP_S3_USAGE_LINE,
  },
];

for (const scenario of SCENARIOS) {
  test(scenario.label, async (t) => {
    await t.test('ok: exact transcript in order, exit 0, one DELETE', async () => {
      const fake = await startFakeWebDriver({ behavior: 'ok', readResult: scenario.readResult });
      try {
        const { lines, code } = await run(scenario.script, scenario.argsFor(fake.baseUrl));
        assert.deepEqual(lines, scenario.okLines);
        assert.equal(code, 0);
        // Arm, then read, then delete -- all addressed to the session created.
        assert.deepEqual(fake.requests, [
          'POST /session',
          'POST /session/sess-1/execute/sync',
          'POST /session/sess-1/execute/sync',
          'DELETE /session/sess-1',
        ]);
      } finally {
        await fake.close();
      }
    });

    await t.test('read fails (execute 500): unexpected-error blocker, exit 1, one DELETE', async () => {
      const fake = await startFakeWebDriver({ behavior: 'exec-500' });
      try {
        const { lines, code } = await run(scenario.script, scenario.argsFor(fake.baseUrl));
        assert.deepEqual(lines, scenario.execFailLines);
        assert.equal(code, 1);
        assert.equal(fake.deleteCount(), 1);
      } finally {
        await fake.close();
      }
    });

    await t.test('delete fails: session-close-failed replaces session-closed, exit 1, one DELETE attempt', async () => {
      const fake = await startFakeWebDriver({ behavior: 'delete-500', readResult: scenario.readResult });
      try {
        const { lines, code } = await run(scenario.script, scenario.argsFor(fake.baseUrl));
        assert.deepEqual(lines, scenario.deleteFailLines);
        assert.equal(code, 1);
        assert.equal(fake.deleteCount(), 1);
      } finally {
        await fake.close();
      }
    });

    await t.test('create fails: exactly one session-create-failed blocker, exit 1, no DELETE', async () => {
      const fake = await startFakeWebDriver({ behavior: 'create-500' });
      try {
        const { lines, code } = await run(scenario.script, scenario.argsFor(fake.baseUrl));
        assert.deepEqual(lines, scenario.createFailLines);
        assert.equal(code, 1);
        assert.equal(fake.deleteCount(), 0);
      } finally {
        await fake.close();
      }
    });

    await t.test('usage error: missing arguments, exit 2, no request reaches the fake', async () => {
      const fake = await startFakeWebDriver();
      try {
        const { lines, code } = await run(scenario.script, []);
        assert.deepEqual(lines, [scenario.usageLine]);
        assert.equal(code, 2);
        assert.equal(fake.requests.length, 0);
      } finally {
        await fake.close();
      }
    });

    await t.test(
      'SIGTERM while execute hangs: terminated blocker, one DELETE, exit code 143',
      { skip: process.platform === 'win32' && 'Node cannot deliver a catchable SIGTERM to a child on Windows' },
      async () => {
        const fake = await startFakeWebDriver({ behavior: 'hang' });
        const { child, lines, closed } = spawnScript(scenario.script, scenario.argsFor(fake.baseUrl));
        const timer = setTimeout(() => child.kill('SIGKILL'), CHILD_TIMEOUT_MS);
        try {
          await waitForLine(lines, 'gate=session-created session_id=sess-1');
          child.kill('SIGTERM');
          const { code, signal } = await closed;
          assert.deepEqual(lines, [
            'gate=session-created session_id=sess-1',
            'blocker=terminated: SIGTERM',
            'gate=session-closed session_id=sess-1',
          ]);
          assert.equal(code, 143);
          assert.equal(signal, null);
          assert.equal(fake.deleteCount(), 1);
        } finally {
          clearTimeout(timer);
          if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); // a failed wait left it running
          await closed;
          await fake.close();
        }
      },
    );
  });
}

test('vp-s3-scenario.mjs: unsupported <os> value is a usage error, exit 2, no request reaches the fake', async () => {
  const fake = await startFakeWebDriver();
  try {
    const { lines, code } = await run(VP_S3_SCRIPT, [fake.baseUrl, '/app', 'plan9']);
    assert.deepEqual(lines, [VP_S3_USAGE_LINE]);
    assert.equal(code, 2);
    assert.equal(fake.requests.length, 0);
  } finally {
    await fake.close();
  }
});
