// `node --test` unit tests for `vp-s1-probe.mjs`'s pure helpers. No
// WebDriver session, process, or display -- `vp-s1-scenario.mjs`'s own
// orchestration is the E2E-only path, never covered here.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { buildArmScript, formatObservations, formatViolationLines, summarize } from './vp-s1-probe.mjs';

test('buildArmScript', async (t) => {
  const script = buildArmScript();

  await t.test('installs the listener and makes exactly the three named attempts', () => {
    assert.match(script, /addEventListener\('securitypolicyviolation'/);
    assert.match(script, /attempt\('inline-script'/);
    assert.match(script, /attempt\('external-fetch'/);
    assert.match(script, /attempt\('framed-context'/);
  });

  await t.test('records every field a securitypolicyviolation event carries, for the per-event transcript lines', () => {
    assert.match(script, /effectiveDirective: e\.effectiveDirective/);
    assert.match(script, /violatedDirective: e\.violatedDirective/);
    assert.match(script, /blockedURI: e\.blockedURI/);
    assert.match(script, /disposition: e\.disposition/);
    assert.match(script, /originalPolicy: e\.originalPolicy/);
    assert.match(script, /sourceFile: e\.sourceFile/);
    assert.match(script, /lineNumber: e\.lineNumber/);
    assert.match(script, /sample: e\.sample/);
  });

  await t.test('captures exactly the fields formatViolationLines emits, no more and no fewer', () => {
    const captured = [...script.matchAll(/(\w+): e\.\1\b/g)].map((m) => m[1]).sort();
    const [line] = formatViolationLines([{}]);
    const emitted = [...line.matchAll(/ (\w+)=/g)].map((m) => m[1]).filter((key) => key !== 'index').sort();
    assert.deepEqual(captured, emitted);
  });

  await t.test('targets the reserved .invalid host for both network-shaped attempts, never a real origin', () => {
    const matches = script.match(/vp-s1\.invalid/g) ?? [];
    assert.equal(matches.length, 2);
    assert.doesNotMatch(script, /http:\/\/localhost/);
  });
});

function readFixture(overrides = {}) {
  return {
    violations: [],
    inlineRan: false,
    externalFetchSettled: 'pending',
    location: { protocol: 'tauri:', host: 'localhost' },
    metaCsp: null,
    iframeDocumentUrl: null,
    ...overrides,
  };
}

const violation = (effectiveDirective, originalPolicy = 'p') => ({ effectiveDirective, originalPolicy });

test('summarize', async (t) => {
  await t.test('an enforced baseline: all three attempts blocked, none ran', () => {
    const read = readFixture({
      violations: [violation('script-src-elem'), violation('connect-src'), violation('frame-src')],
      externalFetchSettled: 'rejected:TypeError',
    });
    const summary = summarize(read);
    assert.equal(summary.inline_script_violation, true);
    assert.equal(summary.inline_script_ran, false);
    assert.equal(summary.external_fetch_violation, true);
    assert.equal(summary.framed_context_violation, true);
    assert.equal(summary.violation_count, 3);
    assert.equal(summary.policy_dump, 'p');
    assert.equal(summary.third_party_surface_exercised, false);
  });

  await t.test('a rejected fetch without a connect-src violation is a network fact, not a CSP fact', () => {
    const summary = summarize(readFixture({ externalFetchSettled: 'rejected:TypeError' }));
    assert.equal(summary.external_fetch_violation, false);
    assert.equal(summary.external_fetch_settled, 'rejected:TypeError');
  });

  await t.test('no violations at all: every violation flag false, policy_dump null', () => {
    const summary = summarize(readFixture());
    assert.equal(summary.violation_count, 0);
    assert.equal(summary.inline_script_violation, false);
    assert.equal(summary.external_fetch_violation, false);
    assert.equal(summary.framed_context_violation, false);
    assert.equal(summary.policy_dump, null);
  });

  await t.test('script-src-elem/-attr and child-src match their family prefix; default-src matches none', () => {
    const summary = summarize(
      readFixture({ violations: [violation('script-src-attr'), violation('child-src'), violation('default-src')] }),
    );
    assert.equal(summary.inline_script_violation, true);
    assert.equal(summary.framed_context_violation, true);
    assert.equal(summary.external_fetch_violation, false);
  });
});

test('formatObservations', async (t) => {
  await t.test('emits every key in the stable declared order, with a literal "null" for a missing policy_dump', () => {
    const lines = formatObservations(summarize(readFixture()));
    const keys = lines.map((line) => line.slice('observation='.length).split(' value=')[0]);
    assert.deepEqual(keys, [
      'location_scheme', 'meta_csp_present', 'policy_dump', 'inline_script_violation', 'inline_script_ran',
      'external_fetch_violation', 'external_fetch_settled', 'framed_context_violation',
      'iframe_document_url', 'violation_count', 'third_party_surface_exercised',
    ]);
    assert.ok(lines.includes('observation=policy_dump value=null'));
    assert.ok(lines.includes('observation=iframe_document_url value=null'));
  });

  await t.test('carries the frame document URL through as read, never judged', () => {
    const lines = formatObservations(summarize(readFixture({ iframeDocumentUrl: 'about:blank' })));
    assert.ok(lines.includes('observation=iframe_document_url value=about:blank'));
  });

  await t.test('collapses a multi-line policy_dump onto one line, verbatim otherwise', () => {
    const summary = summarize(
      readFixture({
        violations: [violation('script-src', "default-src 'none';\n  script-src 'self';\nframe-src 'none'")],
      }),
    );
    const policyLine = formatObservations(summary).find((line) => line.startsWith('observation=policy_dump'));
    assert.equal(policyLine, "observation=policy_dump value=default-src 'none'; script-src 'self'; frame-src 'none'");
  });
});

test('formatViolationLines', async (t) => {
  await t.test('emits one line per event, in event order, each carrying its own 0-based index', () => {
    const violations = [
      violation('script-src-elem', "default-src 'none'; script-src 'self'"),
      violation('connect-src', "default-src 'none'; script-src 'self'"),
    ];
    const lines = formatViolationLines(violations);
    assert.equal(lines.length, 2);
    assert.match(lines[0], /^observation=violation index=0 /);
    assert.match(lines[1], /^observation=violation index=1 /);
  });

  await t.test('JSON-encodes every field value, so spaces and semicolons in originalPolicy stay unambiguous', () => {
    const policy = "default-src 'none'; script-src 'self'; connect-src 'none'";
    const [line] = formatViolationLines([{ effectiveDirective: 'connect-src', originalPolicy: policy }]);
    assert.ok(line.includes(`originalPolicy=${JSON.stringify(policy)}`));
    assert.ok(line.includes(`effectiveDirective=${JSON.stringify('connect-src')}`));
  });

  await t.test('never truncates or normalizes a value -- newlines and quotes survive the JSON encoding verbatim', () => {
    const policy = "default-src 'none';\n  script-src 'self'";
    const [line] = formatViolationLines([{ effectiveDirective: 'script-src', originalPolicy: policy }]);
    assert.ok(line.includes(`originalPolicy=${JSON.stringify(policy)}`));
  });

  await t.test('carries a missing field through as a literal JSON null, never dropped', () => {
    const [line] = formatViolationLines([{ effectiveDirective: 'script-src' }]);
    assert.ok(line.includes('blockedURI=null'));
    assert.ok(line.includes('sourceFile=null'));
    assert.ok(line.includes('lineNumber=null'));
    assert.ok(line.includes('sample=null'));
  });

  await t.test('emits no lines at all for the empty case -- visible instead via the existing observation=violation_count line', () => {
    assert.deepEqual(formatViolationLines([]), []);
    assert.deepEqual(formatViolationLines(undefined), []);
  });

  await t.test('a null entry still yields its line of nulls, never a throw', () => {
    const [line] = formatViolationLines([null]);
    assert.match(line, /^observation=violation index=0 effectiveDirective=null /);
    assert.ok(line.endsWith('sample=null'));
  });
});
