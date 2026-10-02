// Pure VP-S13 IPC-boundary probe helpers (desktop-stack-verification-plan.md:134:
// typed commands only; path attacks rejected; RCS-001-R14). Produces only
// the eight `IpcBoundaryObservations` fields a runtime probe can observe
// (tools/evidence-validator/src/derive.rs); every other field comes from
// the shell driver and the static CI step -- a later slice.
//
// Attack surface (`src-tauri/src/lib.rs` `generate_handler!`, ~:112-144):
// every dialog or success-side-effecting command is excluded. The four
// included are side-effect-free reads: `guidance_status`/`guidance_preview`
// open a workspace-root file by a validated name, writing nothing
// (`ipc/guidance.rs:213-258`); `candidates_list`'s `runId` is an in-memory
// table key, never a path (`ipc/commands.rs:1506-1525`); `harness_observe`'s
// `id` only reads the process map (`ipc/commands.rs:1956-1968`,
// `omnifrons-supervisor/src/lib.rs:957-965`).
//
// `classifyRejection`'s two typed shapes: (1) the shell's own `ShellError
// { code, message }` (`ipc/dto.rs:1579-1581` kebab-case, :1769-1775), text
// shaped `<code>: <msg>`; (2) Tauri's argument-deserialization failure
// (`tauri-2.11.5 src/ipc/command.rs:63-68,90-104`, `src/error.rs:59-60`
// `Error::InvalidArgs`), rendered by `src/ipc/mod.rs:249-253` as
// `` invalid args `<key>` for command `<cmd>`: <serde error> ``.
//
// `unknown-field` is a paired no-effect check (`docs/spike-log.md:94-96`:
// Tauri ignores an extra key, so the identical-behavior baseline is the
// proof, not a rejection); a differing baseline is `attack_accepted`
// (`BestEffort`). `canary_leaked` is any in-page `canaryHit`, computed on
// the full, uncapped text before `capText` runs.

const MAX_TEXT_LENGTH = 512;
/** `guidanceFile`: the driver's planted symlink to the outside canary. `absentGuidanceFile`: a valid, never-created, never-trapped name. Shared so driver and probe name the same files. */
export const TRAP_NAMES = Object.freeze({ guidanceFile: 'AGENTS.md', absentGuidanceFile: 'VP-S13-ABSENT.md' });

const OVERSIZED_LENGTH = 1024 * 1024; // ~1 MiB
/** The fixed attack corpus: `{ id, class, command, args }`, `args` keyed camelCase as `renderer/src/ipc/harness.ts` invokes (`runId`, not `run_id`). `class` matches `IpcBoundaryObservations`'s doc comments (`derive.rs:324-331`). */
export const ATTACKS = Object.freeze([
  // Malformed: wrong-type (a number where a string is expected, and the reverse).
  { id: 'wrong-type-run-id-number', class: 'wrong-type', command: 'candidates_list', args: { runId: 12345 } },
  { id: 'wrong-type-id-string', class: 'wrong-type', command: 'harness_observe', args: { id: 'not-a-number' } },
  // Malformed: missing-field (a required, non-Option field omitted).
  { id: 'missing-field-kind', class: 'missing-field', command: 'guidance_status', args: { file: 'AGENTS.md' } },
  // Malformed: unknown-field, a paired no-effect check (spike-log.md:94-96); `baselineArgs` omits the extra key, both on the same absent file.
  {
    id: 'unknown-field-extra', class: 'unknown-field', command: 'guidance_preview',
    args: { kind: 'guidance', file: TRAP_NAMES.absentGuidanceFile, vpS13Unknown: true },
    baselineArgs: { kind: 'guidance', file: TRAP_NAMES.absentGuidanceFile },
  },
  // Malformed: oversized (~1 MiB) and id-out-of-range (above 2^53, plus a negative one).
  { id: 'oversized-file', class: 'oversized', command: 'guidance_status', args: { kind: 'guidance', file: 'A'.repeat(OVERSIZED_LENGTH) } },
  { id: 'id-out-of-range-above-2-53', class: 'id-out-of-range', command: 'harness_observe', args: { id: 9_007_199_254_740_993 } },
  { id: 'id-out-of-range-negative', class: 'id-out-of-range', command: 'harness_observe', args: { id: -1 } },
  // Path attack: one class, one command each -- spread across both path-bearing fields (run_id, file); symlink-escape only fits the guidance file (run_id never resolves a path).
  { id: 'traversal-run-id', class: 'traversal', command: 'candidates_list', args: { runId: 'a/../../b' } },
  { id: 'absolute-file', class: 'absolute', command: 'guidance_status', args: { kind: 'guidance', file: '/etc/passwd' } },
  { id: 'control-run-id', class: 'nul-or-control', command: 'candidates_list', args: { runId: 'run\u0007id' } },
  { id: 'symlink-escape-guidance', class: 'symlink-escape', command: 'guidance_preview', args: { kind: 'guidance', file: TRAP_NAMES.guidanceFile } },
]);

/** Fires every attack's `invoke` (and a paired entry's `baselineArgs` invoke), plus one informational `workspace_current` read-back; `canary` is checked on the full, uncapped text before `capText` runs. */
export function buildArmScript(attacks, { canary } = {}) {
  const corpusJson = JSON.stringify(attacks);
  const canaryJson = JSON.stringify(typeof canary === 'string' ? canary : '');
  return `
    var MAX_TEXT_LENGTH = ${MAX_TEXT_LENGTH};
    var CANARY = ${canaryJson};
    function capText(text) {
      var source = typeof text === 'string' ? text : String(text == null ? '' : text);
      return source.length > MAX_TEXT_LENGTH ? source.slice(0, MAX_TEXT_LENGTH) : source;
    }
    function hasCanary(text) { return CANARY.length > 0 && typeof text === 'string' && text.indexOf(CANARY) !== -1; }
    // The deserialized IPC error body: Tauri's InvalidArgs string, or the shell's {code, message}.
    function rawError(error) {
      if (typeof error === 'string') return error;
      if (error && typeof error === 'object' && typeof error.code === 'string') return error.code + (typeof error.message === 'string' ? ': ' + error.message : '');
      return String(error);
    }
    function rawValue(value) { try { return JSON.stringify(value); } catch (error) { return String(value); } }
    function settle(record, outcomeKey, responseKey, command, args) {
      function onDone(raw, isResolved) {
        if (hasCanary(raw)) record.canaryHit = true;
        record[outcomeKey] = isResolved ? 'resolved' : 'rejected:' + capText(raw);
        if (isResolved) record[responseKey] = capText(raw);
      }
      try {
        window.__TAURI_INTERNALS__.invoke(command, args).then(function (value) { onDone(rawValue(value), true); }, function (error) { onDone(rawError(error), false); });
      } catch (error) { onDone(rawError(error), false); }
    }
    window.__vpS13 = {
      tauriInternalsPresent: !!window.__TAURI_INTERNALS__, workspaceCurrent: null, workspaceCurrentCanaryHit: false,
      attacks: ${corpusJson}.map(function (attack) {
        var record = { id: attack.id, class: attack.class, command: attack.command, args: attack.args, outcome: 'pending', response: null, canaryHit: false };
        if (attack.baselineArgs) { record.baselineArgs = attack.baselineArgs; record.baselineOutcome = 'pending'; record.baselineResponse = null; }
        return record;
      }),
    };
    if (window.__TAURI_INTERNALS__) {
      window.__TAURI_INTERNALS__.invoke('workspace_current').then(
        function (value) { window.__vpS13.workspaceCurrent = value || null; window.__vpS13.workspaceCurrentCanaryHit = hasCanary(rawValue(value)); },
        function () { window.__vpS13.workspaceCurrent = null; },
      );
      window.__vpS13.attacks.forEach(function (attack) {
        settle(attack, 'outcome', 'response', attack.command, attack.args);
        if (attack.baselineArgs) settle(attack, 'baselineOutcome', 'baselineResponse', attack.command, attack.baselineArgs);
      });
    }
  `;
}

/** Reads back `window.__vpS13`, deep-cloned, plus `location.protocol`; an empty snapshot, never a throw, when never armed. */
export function buildReadScript() {
  return `
    var snapshot = window.__vpS13 ? JSON.parse(JSON.stringify(window.__vpS13)) : {};
    snapshot.location_protocol = location.protocol;
    return snapshot;
  `;
}

const TAURI_INVALID_ARGS_SHAPE = /^invalid args `[^`]*` for command `[^`]*`: /;
const SHELL_ERROR_CODE_SHAPE = /^[a-z][a-z0-9]*(-[a-z0-9]+)*: /;

/** `typed`: the shell's kebab-case `ShellError` shape (`dto.rs:1579-1581`) or Tauri's arg-deserialization shape; `untyped`: a crash, a timeout, or an empty message. */
export function classifyRejection(message) {
  const text = typeof message === 'string' ? message : '';
  if (text.length === 0) return 'untyped';
  if (TAURI_INVALID_ARGS_SHAPE.test(text) || SHELL_ERROR_CODE_SHAPE.test(text)) return 'typed';
  return 'untyped';
}

const MALFORMED_CLASSES = ['wrong-type', 'missing-field', 'unknown-field', 'oversized', 'id-out-of-range'];
const PATH_ATTACK_CLASSES = ['traversal', 'absolute', 'nul-or-control', 'symlink-escape'];

/** The last path component of `value`, tolerating both separators; `null` for a non-string or empty value. */
function basename(value) {
  if (typeof value !== 'string' || value.length === 0) return null;
  const parts = value.split(/[\\/]/);
  return parts[parts.length - 1] || null;
}

/** The probe-observable `IpcBoundaryObservations` fields. Never throws. `workspace_selected` compares the `displayPath` basename, since the shell returns the full canonical path, never a bare name (`dto.rs:72-88`); canonicalization is the driver's own before/after snapshot. */
export function summarize(read, { expectedWorkspaceBasename } = {}) {
  const source = read ?? {};
  const attacks = Array.isArray(source.attacks) ? source.attacks : [];
  const classesSent = new Set(attacks.filter((a) => a && typeof a.class === 'string').map((a) => a.class));
  const isSettled = (text) => text === 'resolved' || (typeof text === 'string' && text.startsWith('rejected:'));
  const isRejected = (a) => Boolean(a) && typeof a.outcome === 'string' && a.outcome.startsWith('rejected:');
  const isUnknownField = (a) => Boolean(a) && a.class === 'unknown-field'; // paired no-effect check (spike-log.md:94-96), excluded below; differing is `BestEffort`
  const judged = attacks.filter((a) => !isUnknownField(a));
  const paired = attacks.filter(isUnknownField);
  const hasBaseline = (a) => a.baselineOutcome !== undefined;
  const isFullySettled = (a) => isSettled(a.outcome) && (!hasBaseline(a) || isSettled(a.baselineOutcome));
  const pairedSent = paired.length > 0 && paired.every(isFullySettled);
  const pairedEffect = paired.some((a) => hasBaseline(a) && isFullySettled(a) && (a.outcome !== a.baselineOutcome || a.response !== a.baselineResponse));
  const rejections = judged.filter(isRejected);
  const workspaceBasename = basename(source.workspaceCurrent && source.workspaceCurrent.displayPath);
  return {
    location_scheme_is_app_protocol: source.location_protocol === 'tauri:',
    workspace_selected: typeof expectedWorkspaceBasename === 'string' && workspaceBasename === expectedWorkspaceBasename,
    malformed_payloads_sent: MALFORMED_CLASSES.every((cls) => (cls === 'unknown-field' ? pairedSent : classesSent.has(cls))),
    path_attacks_sent: PATH_ATTACK_CLASSES.every((cls) => classesSent.has(cls)),
    every_attack_rejected: judged.every(isRejected), // vacuously true on an empty (judged) set, by design; *_sent above guards Pass
    every_rejection_typed: rejections.every((a) => classifyRejection(a.outcome.slice('rejected:'.length)) === 'typed'),
    attack_accepted: judged.some((a) => a && a.outcome === 'resolved') || pairedEffect,
    canary_leaked: Boolean(source.workspaceCurrentCanaryHit) || attacks.some((a) => a && a.canaryHit === true),
  };
}

const OBSERVATION_ORDER = [
  'location_scheme_is_app_protocol', 'workspace_selected', 'malformed_payloads_sent',
  'path_attacks_sent', 'every_attack_rejected', 'every_rejection_typed',
  'attack_accepted', 'canary_leaked',
];

/** `summarize`'s facts as `observation=<key> value=<v>` lines, stable declared order. */
export function formatObservations(summary) {
  return OBSERVATION_ORDER.map((key) => `observation=${key} value=${summary[key]}`);
}

/** One `observation=attack index=<i> id=<json> class=<json> command=<json> outcome=<json>` line per attack, corpus order; a paired entry also carries ` baseline=<json>`. */
export function formatAttackLines(attacks) {
  const list = Array.isArray(attacks) ? attacks : [];
  return list.map((entry, index) => {
    const source = entry ?? {};
    const line = `observation=attack index=${index} id=${JSON.stringify(source.id ?? null)} `
      + `class=${JSON.stringify(source.class ?? null)} command=${JSON.stringify(source.command ?? null)} `
      + `outcome=${JSON.stringify(source.outcome ?? null)}`;
    return source.baselineOutcome === undefined ? line : `${line} baseline=${JSON.stringify(source.baselineOutcome)}`;
  });
}
