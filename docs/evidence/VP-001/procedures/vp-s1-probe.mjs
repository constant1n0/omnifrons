// Pure VP-S1 CSP probe helpers (desktop-stack-verification-plan.md:122,
// "CSP baseline enforced by the packaged webview"): arm/read scripts run in
// the packaged renderer via `webdriver-session.mjs`'s `executeScript`, plus
// the summary derived from what they read back. Exported separately from
// `vp-s1-scenario.mjs` for `node --test` (mirrors `vp-s6-observations.mjs`).
//
// No third-party app surface exists in this renderer today (ADR-0004; one
// window, no router, no iframes) -- `summarize` always reports
// `third_party_surface_exercised: false`, and `derive_csp`
// (`tools/evidence-validator/src/derive.rs`) returns `Uncertain` whenever
// that is the case: the maintainer's decision on how to file the row until
// that surface exists.

/**
 * Installs a `securitypolicyviolation` listener, then makes exactly three
 * attempts the CSP baseline (docs/renderer-content-security.md § CSP
 * baseline) means to block, each recorded with a name and whether it threw
 * synchronously. `${}` is never used in the returned source -- this
 * function's own template literal would substitute it first.
 */
export function buildArmScript() {
  return `
    window.__vpS1 = { violations: [], attempts: [] };
    window.__vpS1Iframe = null;

    document.addEventListener('securitypolicyviolation', function (e) {
      window.__vpS1.violations.push({
        effectiveDirective: e.effectiveDirective,
        violatedDirective: e.violatedDirective,
        blockedURI: e.blockedURI,
        disposition: e.disposition,
        originalPolicy: e.originalPolicy,
        sourceFile: e.sourceFile,
      });
    });

    function attempt(name, fn) {
      try {
        fn();
        window.__vpS1.attempts.push({ name: name, threw: false });
      } catch (error) {
        window.__vpS1.attempts.push({ name: name, threw: true });
      }
    }

    // (a) what 'script-src' 'self' rejects.
    attempt('inline-script', function () {
      var s = document.createElement('script');
      s.textContent = 'window.__vpS1.inlineRan = true;';
      document.body.appendChild(s);
    });

    // (b) reserved, unreachable '.invalid' host (RFC 2606) -- 'connect-src'
    // 'none' governs dispatch, not DNS.
    window.__vpS1.externalFetchSettled = 'pending';
    attempt('external-fetch', function () {
      fetch('https://vp-s1.invalid/')
        .then(function () { window.__vpS1.externalFetchSettled = 'resolved'; })
        .catch(function (error) {
          window.__vpS1.externalFetchSettled = 'rejected:' + error.name;
        });
    });

    // (c) what 'frame-src' 'none' rejects.
    attempt('framed-context', function () {
      var iframe = document.createElement('iframe');
      iframe.src = 'https://vp-s1.invalid/';
      document.body.appendChild(iframe);
      window.__vpS1Iframe = iframe;
    });

    window.__vpS1.location = { protocol: location.protocol, host: location.host };
    window.__vpS1.metaCsp = (function () {
      var meta = document.querySelector('meta[http-equiv="Content-Security-Policy"]');
      return meta ? meta.getAttribute('content') : null;
    })();
  `;
}

/**
 * Reads back what `buildArmScript` recorded, deep-cloned via
 * `JSON.parse(JSON.stringify)` so a non-serializable value fails loudly.
 * `iframeDocumentUrl` is read from the live `window.__vpS1Iframe` element
 * separately -- a DOM node `JSON.stringify` cannot carry: the URL of the
 * document the frame holds, `cross-origin` when reading it throws, or
 * `null` when no frame was appended. A blocked frame is left holding
 * `about:blank`; the `frame-src` violation event remains the signal.
 */
export function buildReadScript() {
  return `
    var snapshot = JSON.parse(JSON.stringify(window.__vpS1));
    snapshot.inlineRan = window.__vpS1.inlineRan === true;
    snapshot.iframeDocumentUrl = (function () {
      var iframe = window.__vpS1Iframe;
      if (!iframe) return null;
      try {
        return iframe.contentDocument ? iframe.contentDocument.URL : null;
      } catch (error) {
        return 'cross-origin';
      }
    })();
    return snapshot;
  `;
}

/**
 * The facts a VP-S1 row needs -- never an outcome; `derive_csp` is the only
 * place these become `pass`/`fail`/`uncertain`. Discriminating rule: a
 * rejected fetch WITHOUT a matching `connect-src` violation is a network
 * fact (DNS failure against '.invalid'), not a CSP fact.
 */
export function summarize(read) {
  const violations = Array.isArray(read.violations) ? read.violations : [];
  const hasViolation = (prefixes) =>
    violations.some(
      (v) => typeof v.effectiveDirective === 'string' && prefixes.some((p) => v.effectiveDirective.startsWith(p)),
    );
  const firstViolationPolicy = violations.length > 0 ? (violations[0].originalPolicy ?? null) : null;

  return {
    location_scheme: read.location?.protocol ?? null,
    meta_csp_present: read.metaCsp !== null && read.metaCsp !== undefined,
    policy_dump: firstViolationPolicy ?? read.metaCsp ?? null,
    inline_script_violation: hasViolation(['script-src']),
    inline_script_ran: read.inlineRan === true,
    external_fetch_violation: hasViolation(['connect-src']),
    external_fetch_settled: read.externalFetchSettled ?? 'pending',
    framed_context_violation: hasViolation(['frame-src', 'child-src']),
    // The URL of the document the appended frame holds, recorded and never
    // judged: `null` when no iframe was appended at all.
    iframe_document_url: read.iframeDocumentUrl ?? null,
    violation_count: violations.length,
    third_party_surface_exercised: false, // no such surface exists (ADR-0004)
  };
}

const OBSERVATION_ORDER = [
  'location_scheme', 'meta_csp_present', 'policy_dump', 'inline_script_violation', 'inline_script_ran',
  'external_fetch_violation', 'external_fetch_settled', 'framed_context_violation',
  'iframe_document_url', 'violation_count', 'third_party_surface_exercised',
];

/** Collapse a policy dump onto one line; verbatim otherwise. */
function toSingleLine(value) {
  if (value === null || value === undefined) return 'null';
  return value.replace(/\s*\n\s*/g, ' ').replace(/;\s+/g, '; ').trim();
}

/** `summarize`'s facts as `observation=<key> value=<v>` lines, stable order. */
export function formatObservations(summary) {
  return OBSERVATION_ORDER.map((key) => {
    const value = key === 'policy_dump' ? toSingleLine(summary[key]) : summary[key];
    return `observation=${key} value=${value}`;
  });
}
