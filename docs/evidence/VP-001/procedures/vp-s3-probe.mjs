// Pure VP-S3 typed-IPC bridge probe helpers (desktop-stack-verification-plan.md:124:
// network-shaped transport confined to the documented connect-src source, custom
// protocol handlers explicit). Arm/read scripts run via `webdriver-session.mjs`'s
// `executeScript`; `summarize` derives facts from what they read back (mirrors
// `vp-s1-probe.mjs`).
//
// Transport (renderer-content-security.md CSP baseline; pinned Tauri 2.11.5
// source): `__TAURI_INTERNALS__.invoke(cmd)` is a global `fetch(..., { method:
// 'POST' })` to `ipc://localhost/<cmd>` on Linux/macOS (`ipc-protocol.js:20-85`)
// or `http://ipc.localhost/<cmd>` on Windows (URL from `core.js:13-19`). On
// failure -- a `connect-src` block included -- Tauri warns `'IPC custom
// protocol failed ...'` and falls back to `window.ipc.postMessage(data)`
// (`ipc-protocol.js:55-85`). `artifact:`
// (`img-src`/`media-src`) has no handler yet, so every load fails (RCS-001).
// The deliberate `fetch('https://vp-s3.invalid/')` (`policy-probe`) forces a
// `connect-src` violation so `originalPolicy` yields a dump even when the
// bridge raises none. `identity_gated`/`static_inventory_pinned` are supplied
// elsewhere (the shell driver's gates; CI's protocol_inventory.rs run).
//
// Transcript: `formatObservations` emits `observation=<key> value=<v>` lines;
// `formatFetchLines` emits JSON-encoded `observation=fetch index=<i>
// url=<json> outcome=<json>` lines, one per recorded fetch; `formatViolationLines` is reused from
// `vp-s1-probe.mjs` (re-exported below), never duplicated.

export { formatViolationLines } from './vp-s1-probe.mjs';

/**
 * Wraps fetch/console.warn/postMessage, then makes four attempts: (a) a
 * registered command, (b) an unregistered one, (c) a reserved artifact:
 * load, (d) a deliberate connect-src violation. No `${}` in the source below
 * -- this function's own template literal would substitute it first.
 */
export function buildArmScript() {
  return `
    window.__vpS3 = {
      violations: [], fetches: [], attempts: [], tauriInternalsPresent: false,
      consoleWarnFallbackSeen: false, postMessageWrapped: false, postMessageCallCount: 0,
    };

    document.addEventListener('securitypolicyviolation', function (e) {
      window.__vpS3.violations.push({
        effectiveDirective: e.effectiveDirective, violatedDirective: e.violatedDirective,
        blockedURI: e.blockedURI, disposition: e.disposition, originalPolicy: e.originalPolicy,
        sourceFile: e.sourceFile, lineNumber: e.lineNumber, sample: e.sample,
      });
    });

    // settle(record, promise, onResolve, errorKey) resolves 'rejected:<err[errorKey]>' or onResolve's text.
    function settle(record, promise, onResolve, errorKey) {
      promise.then(
        function (v) { record.outcome = onResolve(v); },
        function (error) { record.outcome = 'rejected:' + (error && error[errorKey] ? error[errorKey] : String(error)); },
      );
    }
    function fetchResolveText(response) { return 'resolved:' + response.status; }
    function pushAttempt(name) {
      var record = { name: name, outcome: 'pending' };
      window.__vpS3.attempts.push(record);
      return record;
    }

    // Wrapped before any invoke, so the bridge's own fetch is captured too.
    var originalFetch = window.fetch.bind(window);
    window.fetch = function (input, init) {
      var url = typeof input === 'string' ? input : (input && input.url) || String(input);
      var record = { url: url, outcome: 'pending' };
      window.__vpS3.fetches.push(record);
      var promise = originalFetch(input, init);
      settle(record, promise, fetchResolveText, 'name');
      return promise;
    };

    var originalWarn = console.warn.bind(console);
    console.warn = function () {
      var args = Array.prototype.slice.call(arguments);
      var text = args.map(function (a) { return typeof a === 'string' ? a : String(a); }).join(' ');
      if (text.indexOf('IPC custom protocol failed') !== -1) window.__vpS3.consoleWarnFallbackSeen = true;
      return originalWarn.apply(console, args);
    };

    if (window.ipc && typeof window.ipc.postMessage === 'function') {
      try {
        var originalPostMessage = window.ipc.postMessage.bind(window.ipc);
        window.ipc.postMessage = function () {
          window.__vpS3.postMessageCallCount += 1;
          return originalPostMessage.apply(window.ipc, arguments);
        };
        window.__vpS3.postMessageWrapped = true;
      } catch (error) {
        window.__vpS3.postMessageWrapped = false;
      }
    }

    // (a)/(b) registered vs. unregistered command; guarded, never a throw.
    window.__vpS3.tauriInternalsPresent = !!window.__TAURI_INTERNALS__;
    if (window.__TAURI_INTERNALS__) {
      var invokeOk = function () { return 'resolved'; };
      settle(pushAttempt('registered-call'), window.__TAURI_INTERNALS__.invoke('shell_health'), invokeOk, 'message');
      settle(pushAttempt('unregistered-call'), window.__TAURI_INTERNALS__.invoke('vp_s3_unregistered_command'), invokeOk, 'message');
    }

    // (c) the reserved, unhandled artifact: scheme (RCS-001).
    var artifactAttempt = pushAttempt('artifact-load');
    var artifactImage = new Image();
    artifactImage.addEventListener('load', function () { artifactAttempt.outcome = 'load'; });
    artifactImage.addEventListener('error', function () { artifactAttempt.outcome = 'error'; });
    artifactImage.src = 'artifact://localhost/vp-s3-probe.png';

    // (d) forces a connect-src violation so its policy yields a dump.
    settle(pushAttempt('policy-probe'), fetch('https://vp-s3.invalid/'), fetchResolveText, 'name');
  `;
}

/** Reads back `window.__vpS3`, deep-cloned, plus `location.protocol` as `location_protocol`. */
export function buildReadScript() {
  return `
    var snapshot = JSON.parse(JSON.stringify(window.__vpS3));
    snapshot.location_protocol = location.protocol;
    return snapshot;
  `;
}

/** The typed-IPC bridge's `connect-src` source per OS. Pure; throws otherwise. */
export function bridgeOriginFor(os) {
  if (os === 'linux' || os === 'macos') return 'ipc://localhost/';
  if (os === 'windows') return 'http://ipc.localhost/';
  throw new RangeError(`bridgeOriginFor: unsupported OS "${os}"`);
}

/** The `connect-src` source RCS-001 documents for `os`. Pure; throws otherwise. */
export function documentedConnectSrcFor(os) {
  if (os === 'linux' || os === 'macos') return ['ipc:'];
  if (os === 'windows') return ['http://ipc.localhost'];
  throw new RangeError(`documentedConnectSrcFor: unsupported OS "${os}"`);
}

/** The `connect-src` directive's source tokens, in order, or `null` when absent. Never throws. */
export function connectSrcSources(policy) {
  if (typeof policy !== 'string') return null;
  const directives = policy.split(';').map((d) => d.trim()).filter((d) => d.length > 0);
  for (const directive of directives) {
    const parts = directive.split(/\s+/).filter((p) => p.length > 0);
    if (parts[0] === 'connect-src') return parts.slice(1);
  }
  return null;
}

/** Runs `fn`, returning `null` on throw (e.g. an unsupported OS token). */
function tryOrNull(fn) {
  try { return fn(); } catch { return null; }
}

/** `true` iff `tokens` is exactly `documented` as a set (same members, no extras). */
function sameSourceSet(tokens, documented) {
  return Array.isArray(tokens) && Array.isArray(documented) && tokens.length === documented.length
    && documented.every((d) => tokens.includes(d));
}

/** The probe-observable `IpcObservations` fields, plus `location_scheme` for the transcript. Never throws. */
export function summarize(read, os) {
  const source = read ?? {};
  const violations = Array.isArray(source.violations) ? source.violations : [];
  const fetches = Array.isArray(source.fetches) ? source.fetches : [];
  const attempts = Array.isArray(source.attempts) ? source.attempts : [];
  const findAttempt = (name) => attempts.find((a) => a && a.name === name);

  const bridgeOrigin = tryOrNull(() => bridgeOriginFor(os));
  const bridgeScheme = bridgeOrigin ? bridgeOrigin.slice(0, bridgeOrigin.indexOf(':') + 1) : null;
  const documented = tryOrNull(() => documentedConnectSrcFor(os));

  const policyViolations = violations.filter((v) => v && typeof v.originalPolicy === 'string' && v.originalPolicy.length > 0);
  const connectSrcPolicyCaptured = policyViolations.length > 0;
  const connectSrcIsDocumentedSourceForOs = connectSrcPolicyCaptured && documented !== null
    ? sameSourceSet(connectSrcSources(policyViolations[0].originalPolicy), documented)
    : false;

  const registeredAttempt = findAttempt('registered-call');
  const usedCustomProtocol = (f) => f && typeof f.url === 'string' && bridgeOrigin
    && f.url.startsWith(bridgeOrigin + 'shell_health') && typeof f.outcome === 'string' && /^resolved:2\d\d$/.test(f.outcome);

  const postmessageFallbackObserved = source.consoleWarnFallbackSeen === true
    || (typeof source.postMessageCallCount === 'number' && source.postMessageCallCount >= 1);

  const isBridgeViolation = (v) => v && typeof v.effectiveDirective === 'string' && v.effectiveDirective.startsWith('connect-src')
    && typeof v.blockedURI === 'string' && bridgeOrigin
    // CSP3 reports a non-HTTP(S) blocked URL as its bare scheme (`ipc`), so that form counts too.
    && (v.blockedURI.startsWith(bridgeOrigin) || v.blockedURI.startsWith(bridgeScheme) || v.blockedURI === bridgeScheme.slice(0, -1));

  const unregisteredAttempt = findAttempt('unregistered-call');
  const artifactAttempt = findAttempt('artifact-load');

  return {
    location_scheme: source.location_protocol ?? null,
    location_scheme_is_app_protocol: source.location_protocol === 'tauri:',
    connect_src_policy_captured: connectSrcPolicyCaptured,
    connect_src_is_documented_source_for_os: connectSrcIsDocumentedSourceForOs,
    registered_call_completed: registeredAttempt?.outcome === 'resolved',
    registered_call_used_custom_protocol: fetches.some(usedCustomProtocol),
    postmessage_fallback_observed: postmessageFallbackObserved,
    bridge_connect_src_violation: violations.some(isBridgeViolation),
    unregistered_call_rejected: typeof unregisteredAttempt?.outcome === 'string' && unregisteredAttempt.outcome.startsWith('rejected'),
    unregistered_call_resolved: unregisteredAttempt?.outcome === 'resolved',
    artifact_load_attempted: artifactAttempt?.outcome === 'load' || artifactAttempt?.outcome === 'error',
    artifact_load_succeeded: artifactAttempt?.outcome === 'load',
  };
}

const OBSERVATION_ORDER = [
  'location_scheme', 'location_scheme_is_app_protocol', 'connect_src_policy_captured',
  'connect_src_is_documented_source_for_os', 'registered_call_completed', 'registered_call_used_custom_protocol',
  'postmessage_fallback_observed', 'bridge_connect_src_violation', 'unregistered_call_rejected',
  'unregistered_call_resolved', 'artifact_load_attempted', 'artifact_load_succeeded',
];

/** `summarize`'s facts as `observation=<key> value=<v>` lines, stable order. */
export function formatObservations(summary) {
  return OBSERVATION_ORDER.map((key) => `observation=${key} value=${summary[key]}`);
}

/** Every recorded `fetch` call (bridge or not) as one `observation=fetch index=<i> url=<json> outcome=<json>` line. */
export function formatFetchLines(fetches) {
  const list = Array.isArray(fetches) ? fetches : [];
  return list.map((entry, index) => {
    const source = entry ?? {};
    return `observation=fetch index=${index} url=${JSON.stringify(source.url ?? null)} outcome=${JSON.stringify(source.outcome ?? null)}`;
  });
}
