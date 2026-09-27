// `node --test` unit tests for `vp-s3-probe.mjs`'s pure helpers, no WebDriver (mirrors vp-s1-probe.test.mjs).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  bridgeOriginFor, buildArmScript, buildReadScript, connectSrcSources, documentedConnectSrcFor,
  formatFetchLines, formatObservations, formatViolationLines, summarize,
} from './vp-s3-probe.mjs';

test('buildReadScript returns an empty snapshot, never throws, when the probe was never armed', () => {
  const script = buildReadScript();
  assert.match(script, /window\.__vpS3 \? JSON\.parse\(JSON\.stringify\(window\.__vpS3\)\) : \{\}/);
  // Run the page script the way executeScript does, in a scope with no armed probe.
  const snapshot = new Function('window', 'location', script)({}, { protocol: 'tauri:' });
  assert.deepEqual(snapshot, { location_protocol: 'tauri:' });
});

test('summarize on Windows', async (t) => {
  const connect = (blockedURI, originalPolicy = 'connect-src http://ipc.localhost') => ({
    effectiveDirective: 'connect-src', originalPolicy, blockedURI,
  });
  await t.test('the http bridge origin backs the custom-protocol and documented-source facts', () => {
    const summary = summarize({
      violations: [connect('https://vp-s3.invalid/')],
      fetches: [{ url: 'http://ipc.localhost/shell_health', outcome: 'resolved:200' }],
      attempts: [{ name: 'registered-call', outcome: 'resolved' }],
    }, 'windows');
    assert.equal(summary.connect_src_is_documented_source_for_os, true);
    assert.equal(summary.registered_call_used_custom_protocol, true);
    assert.equal(summary.bridge_connect_src_violation, false);
  });
  await t.test('only the bridge origin counts as a bridge violation, never any other http URL', () => {
    for (const uri of ['http://ipc.localhost/shell_health', 'http://ipc.localhost']) {
      assert.equal(summarize({ violations: [connect(uri)] }, 'windows').bridge_connect_src_violation, true, uri);
    }
    for (const uri of ['http://example.invalid/', 'http://ipc.localhost.example.invalid/', 'http']) {
      assert.equal(summarize({ violations: [connect(uri)] }, 'windows').bridge_connect_src_violation, false, uri);
    }
  });
  await t.test("Linux's source is not Windows' documented one", () => {
    const summary = summarize({ violations: [connect('https://vp-s3.invalid/', 'connect-src ipc:')] }, 'windows');
    assert.equal(summary.connect_src_is_documented_source_for_os, false);
  });
});

test('buildArmScript', async (t) => {
  const script = buildArmScript();

  await t.test('installs the listener, capturing exactly the fields formatViolationLines emits', () => {
    assert.match(script, /addEventListener\('securitypolicyviolation'/);
    const captured = [...script.matchAll(/(\w+): e\.\1\b/g)].map((m) => m[1]).sort();
    const [line] = formatViolationLines([{}]);
    const emitted = [...line.matchAll(/ (\w+)=/g)].map((m) => m[1]).filter((key) => key !== 'index').sort();
    assert.deepEqual(captured, emitted);
  });
  await t.test('wraps window.fetch before any invoke call, so the bridge fetch itself is captured', () => {
    const fetchWrapIndex = script.indexOf('window.fetch = function');
    assert.ok(fetchWrapIndex >= 0);
    assert.ok(script.indexOf("invoke('shell_health')") > fetchWrapIndex);
  });
  await t.test('wraps console.warn to detect the Tauri postMessage-fallback warning text', () => {
    assert.match(script, /console\.warn = function/);
    assert.match(script, /IPC custom protocol failed/);
  });
  await t.test('wraps postMessage behind a presence check, tolerating a non-writable property', () => {
    assert.match(script, /window\.ipc && typeof window\.ipc\.postMessage === 'function'/);
    assert.match(script, /try \{[\s\S]*window\.ipc\.postMessage = function[\s\S]*\} catch/);
  });
  await t.test('makes exactly the four attempts, against their exact targets, guarded behind __TAURI_INTERNALS__', () => {
    const guardIndex = script.indexOf('if (window.__TAURI_INTERNALS__)');
    assert.ok(guardIndex >= 0);
    assert.ok(script.indexOf("invoke('shell_health')") > guardIndex);
    assert.ok(script.indexOf("invoke('vp_s3_unregistered_command')") > guardIndex);
    assert.match(script, /artifact:\/\/localhost\/vp-s3-probe\.png/);
    assert.match(script, /fetch\('https:\/\/vp-s3\.invalid\/'\)/);
    assert.match(script, /'policy-probe'/);
  });
  await t.test('targets the reserved .invalid host exactly once, never a real origin', () => {
    assert.equal((script.match(/vp-s3\.invalid/g) ?? []).length, 1);
    assert.doesNotMatch(script, /http:\/\/localhost/);
  });
});

test('bridgeOriginFor / documentedConnectSrcFor', async (t) => {
  await t.test('linux and macos share the ipc: custom-protocol origin and its documented source', () => {
    for (const os of ['linux', 'macos']) {
      assert.equal(bridgeOriginFor(os), 'ipc://localhost/');
      assert.deepEqual(documentedConnectSrcFor(os), ['ipc:']);
    }
  });
  await t.test('windows uses the http origin WebView2 requires, documented the same way', () => {
    assert.equal(bridgeOriginFor('windows'), 'http://ipc.localhost/');
    assert.deepEqual(documentedConnectSrcFor('windows'), ['http://ipc.localhost']);
  });
  await t.test('both throw for any other OS token', () => {
    assert.throws(() => bridgeOriginFor('bsd'));
    assert.throws(() => documentedConnectSrcFor(undefined));
  });
});

test('connectSrcSources', async (t) => {
  await t.test('returns the connect-src tokens in order, alongside other directives and extra whitespace', () => {
    assert.deepEqual(connectSrcSources("default-src 'none'; connect-src ipc:; img-src 'self' artifact:"), ['ipc:']);
    const spaced = "default-src 'none';   connect-src   ipc:   http://ipc.localhost  ; img-src 'self'";
    assert.deepEqual(connectSrcSources(spaced), ['ipc:', 'http://ipc.localhost']);
  });
  await t.test('returns null when absent, never matches by prefix, and never throws on malformed input', () => {
    assert.equal(connectSrcSources("default-src 'none'; img-src 'self' artifact:"), null);
    assert.equal(connectSrcSources('connect-src-x ipc:'), null);
    for (const malformed of [null, undefined]) assert.equal(connectSrcSources(malformed), null);
  });
});

function readFixture(overrides = {}) {
  return {
    violations: [], fetches: [], attempts: [], consoleWarnFallbackSeen: false, postMessageCallCount: 0,
    location_protocol: 'tauri:', ...overrides,
  };
}

test('summarize', async (t) => {
  await t.test('a fully confined Linux baseline: every field true except the four exceptions', () => {
    const policy = "default-src 'none'; connect-src ipc:; img-src 'self' artifact:";
    const read = readFixture({
      violations: [{ effectiveDirective: 'connect-src', originalPolicy: policy, blockedURI: 'https://vp-s3.invalid/' }],
      fetches: [{ url: 'ipc://localhost/shell_health', outcome: 'resolved:200' }],
      attempts: [
        { name: 'registered-call', outcome: 'resolved' },
        { name: 'unregistered-call', outcome: 'rejected:command vp_s3_unregistered_command not found' },
        { name: 'artifact-load', outcome: 'error' },
        { name: 'policy-probe', outcome: 'rejected:TypeError' },
      ],
    });
    const summary = summarize(read, 'linux');
    const expected = {
      location_scheme_is_app_protocol: true, connect_src_policy_captured: true,
      connect_src_is_documented_source_for_os: true, registered_call_completed: true,
      registered_call_used_custom_protocol: true, unregistered_call_rejected: true, artifact_load_attempted: true,
      artifact_load_succeeded: false, postmessage_fallback_observed: false, bridge_connect_src_violation: false,
      unregistered_call_resolved: false,
    };
    for (const [key, value] of Object.entries(expected)) assert.equal(summary[key], value, key);
  });
  await t.test('a fallback snapshot: either the Tauri warning or a counted postMessage call marks it observed', () => {
    assert.equal(summarize(readFixture({ consoleWarnFallbackSeen: true }), 'linux').postmessage_fallback_observed, true);
    assert.equal(summarize(readFixture({ postMessageCallCount: 1 }), 'linux').postmessage_fallback_observed, true);
  });
  await t.test('a bridge connect-src violation is counted; the deliberate vp-s3.invalid violation is not', () => {
    const v = (blockedURI) => ({ effectiveDirective: 'connect-src', originalPolicy: 'connect-src ipc:', blockedURI });
    for (const uri of ['ipc://localhost/shell_health', 'ipc']) { // CSP3 reports a non-HTTP(S) URL as its bare scheme
      assert.equal(summarize(readFixture({ violations: [v(uri)] }), 'linux').bridge_connect_src_violation, true, uri);
    }
    assert.equal(summarize(readFixture({ violations: [v('https://vp-s3.invalid/')] }), 'linux').bridge_connect_src_violation, false);
  });
  await t.test('an undocumented extra connect-src source renders the OS documentation check false', () => {
    const violation = { effectiveDirective: 'connect-src', originalPolicy: 'connect-src ipc: http://ipc.localhost' };
    const summary = summarize(readFixture({ violations: [violation] }), 'linux');
    assert.equal(summary.connect_src_policy_captured, true);
    assert.equal(summary.connect_src_is_documented_source_for_os, false);
  });
  await t.test('an artifact: load that resolves is recorded as succeeded; a still-pending one as not attempted', () => {
    const summary = summarize(readFixture({ attempts: [{ name: 'artifact-load', outcome: 'load' }] }), 'linux');
    assert.equal(summary.artifact_load_attempted, true);
    assert.equal(summary.artifact_load_succeeded, true);
    assert.equal(summarize(readFixture({ attempts: [{ name: 'artifact-load', outcome: 'pending' }] }), 'linux').artifact_load_attempted, false);
  });
  await t.test('empty or malformed input yields every field false, never throws', () => {
    const summary = summarize({}, 'linux');
    for (const key of [
      'location_scheme_is_app_protocol', 'connect_src_policy_captured', 'connect_src_is_documented_source_for_os',
      'registered_call_completed', 'registered_call_used_custom_protocol', 'postmessage_fallback_observed',
      'bridge_connect_src_violation', 'unregistered_call_rejected', 'unregistered_call_resolved',
      'artifact_load_attempted', 'artifact_load_succeeded',
    ]) {
      assert.equal(summary[key], false, key);
    }
    const summaryNull = summarize(null, undefined);
    assert.equal(summaryNull.location_scheme_is_app_protocol, false);
    assert.equal(summaryNull.registered_call_used_custom_protocol, false);
  });
});

test('formatObservations / formatFetchLines', async (t) => {
  await t.test('formatObservations emits every key in the stable declared order, carrying location_scheme unjudged', () => {
    const lines = formatObservations(summarize(readFixture(), 'linux'));
    const keys = lines.map((line) => line.slice('observation='.length).split(' value=')[0]);
    assert.deepEqual(keys, [
      'location_scheme', 'location_scheme_is_app_protocol', 'connect_src_policy_captured',
      'connect_src_is_documented_source_for_os', 'registered_call_completed', 'registered_call_used_custom_protocol',
      'postmessage_fallback_observed', 'bridge_connect_src_violation', 'unregistered_call_rejected',
      'unregistered_call_resolved', 'artifact_load_attempted', 'artifact_load_succeeded',
    ]);
    const httpLines = formatObservations(summarize(readFixture({ location_protocol: 'http:' }), 'windows'));
    assert.ok(httpLines.includes('observation=location_scheme value=http:'));
  });
  await t.test('formatFetchLines emits one JSON-encoded line per fetch, in order, and tolerates the empty/null cases', () => {
    const fetches = [
      { url: 'ipc://localhost/shell_health', outcome: 'resolved:200' },
      { url: 'https://vp-s3.invalid/', outcome: 'rejected:TypeError' },
    ];
    const lines = formatFetchLines(fetches);
    assert.equal(lines[0], 'observation=fetch index=0 url="ipc://localhost/shell_health" outcome="resolved:200"');
    assert.equal(lines[1], 'observation=fetch index=1 url="https://vp-s3.invalid/" outcome="rejected:TypeError"');
    assert.deepEqual(formatFetchLines([]), []);
    assert.equal(formatFetchLines([null])[0], 'observation=fetch index=0 url=null outcome=null');
  });
});
