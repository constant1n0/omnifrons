// `node --test` unit tests for `vp-s1-probe.mjs`'s pure helpers. No
// WebDriver session, process, or display -- `vp-s1-scenario.mjs`'s own
// orchestration is the E2E-only path, never covered here.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { buildArmScript, formatObservations, summarize } from './vp-s1-probe.mjs';

test('buildArmScript', async (t) => {
  const script = buildArmScript();

  await t.test('installs the listener and makes exactly the three named attempts', () => {
    assert.match(script, /addEventListener\('securitypolicyviolation'/);
    assert.match(script, /attempt\('inline-script'/);
    assert.match(script, /attempt\('external-fetch'/);
    assert.match(script, /attempt\('framed-context'/);
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
