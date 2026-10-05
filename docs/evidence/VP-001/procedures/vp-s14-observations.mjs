// Pure VP-S14 derive/formatting helpers: `summarize`, `formatObservations`,
// `formatCaseLines`, and `formatRecordLines` turn a driver's captured
// snapshots into the probe-observable subset of
// `ExecutableIdentityObservations` (tools/evidence-validator/src/
// derive.rs:479-528) -- `identity_gated` and `static_gate_pinned` are
// bound by T1/T2 (static CI), never by this module. Split out of the
// in-page script builders in the sibling `vp-s14-probe.mjs`, the same way
// VP-S6 splits `vp-s6-observations.mjs` (pure derive helpers) from
// `vp-s6-xdotool.mjs` (interaction primitives). Neither module imports
// the other: this one knows nothing about IPC, `window`, or script
// strings -- it only shapes plain data the driver already captured
// (`beforeList`/`afterList` from `approvals_list()`, `original`/
// `reapproval` from `executable_approve()`, `cases` from `harness_spawn()`,
// all via `vp-s14-probe.mjs`'s builders and `vp-s14-observations.test.mjs`'s
// own fixtures -- never by importing that module directly).
//
// Case sequence (`vp-s14-probe.mjs`'s own header has the full detail):
// (a) launch the approved copy of `true`; (b) overwrite its bytes with
// `false`'s, then launch; (c) `ln -sf` it to another copy of `true`, then
// launch; (d) pick the same path again (now resolving through the
// symlink), approve, then launch. `cases` is keyed `a`/`b`/`c`/`d`
// (`CASE_LABELS`, defined here since only `formatCaseLines` needs the
// fixed order -- `vp-s14-probe.mjs`'s `buildSpawnScript` already takes
// its label as a plain parameter and has no need of this constant).

/** An approval-list entry's (or an approve-call result's) bound identity:
 * `null` when `evidence` is missing, or when any component is absent or of
 * the wrong type (a non-empty path and digest, a non-negative integer
 * size) -- a hollow `{}` must never compare equal to another hollow `{}`
 * through `undefined === undefined`. Shared by every identity
 * comparison below -- `approval_recorded`, `original_record_unchanged`,
 * `record_silently_rebound`, and `reapproval_launched` all compare this
 * same triple (canonical path, size, SHA-256; the Bounded Pass scope). */
function identityOf(record) {
  const evidence = record && typeof record === 'object' ? record.evidence : undefined;
  if (!evidence || typeof evidence !== 'object') return null;
  const { canonicalPath, size, sha256 } = evidence;
  if (typeof canonicalPath !== 'string' || canonicalPath === '') return null;
  if (!Number.isSafeInteger(size) || size < 0) return null;
  if (typeof sha256 !== 'string' || sha256 === '') return null;
  return { canonicalPath, size, sha256 };
}

function identitiesEqual(left, right) {
  return Boolean(left) && Boolean(right)
    && left.canonicalPath === right.canonicalPath && left.size === right.size && left.sha256 === right.sha256;
}

/** The first entry of `list` whose `approvalId` is `approvalId`, or `null`
 * for a non-array list, a non-string id, or no match. */
function findApproval(list, approvalId) {
  if (!Array.isArray(list) || typeof approvalId !== 'string' || approvalId.length === 0) return null;
  return list.find((entry) => entry && entry.approvalId === approvalId) ?? null;
}

/** `cases[label]`, or `{}` for a missing or malformed entry -- every
 * reader below then sees `undefined` fields rather than throwing. */
function caseOf(cases, label) {
  const entry = cases && typeof cases === 'object' ? cases[label] : undefined;
  return entry && typeof entry === 'object' ? entry : {};
}

const isSettled = (entry) => entry.outcome === 'resolved' || entry.outcome === 'rejected';
const isResolvedWithProcess = (entry) => entry.outcome === 'resolved' && typeof entry.processId === 'number';
const isDeniedWithCode = (entry, code) => entry.outcome === 'rejected' && entry.code === code;

export const CASE_LABELS = Object.freeze(['a', 'b', 'c', 'd']);

/** The probe-observable `ExecutableIdentityObservations` fields
 * (derive.rs:479-528), never `identity_gated`/`static_gate_pinned`. Never
 * throws on malformed input.
 *
 * A denial with any other code, or no code (an untyped rejection), sets
 * neither `rewrite_denied_changed`/`shadow_denied_shadowed` nor
 * `changed_launch_allowed`/`shadow_launch_allowed`: the executable was
 * refused, but not provably by the mechanism under test, so `derive_
 * executable_identity` reads Uncertain for that case (VP-S14's own
 * decision, mirroring VP-S13's untyped-rejection rule).
 *
 * A missing approval-list record (the id absent from `afterList`) is
 * neither `original_record_unchanged` nor `record_silently_rebound`: both
 * require the id to actually be present there. Documented, not a bug --
 * a vanished record is a different, unproven failure mode this scenario
 * does not claim to distinguish. */
export function summarize(input) {
  const { beforeList, afterList, original, reapproval, cases, location_protocol } = input ?? {};
  const a = caseOf(cases, 'a');
  const b = caseOf(cases, 'b');
  const c = caseOf(cases, 'c');
  const d = caseOf(cases, 'd');

  const originalId = original && typeof original.approvalId === 'string' ? original.approvalId : null;
  const originalIdentity = identityOf(original);
  const beforeEntry = findApproval(beforeList, originalId);
  const afterEntry = findApproval(afterList, originalId);
  const beforeIdentity = identityOf(beforeEntry);
  const afterIdentity = identityOf(afterEntry);

  const approval_recorded = Boolean(beforeEntry) && Boolean(originalIdentity)
    && identitiesEqual(beforeIdentity, originalIdentity);

  const original_record_unchanged = Boolean(beforeEntry) && Boolean(afterEntry) && identitiesEqual(beforeIdentity, afterIdentity);
  // A rebind needs both identities established: a snapshot missing either
  // is unproven, never the failure fact (the same restraint as a vanished
  // record or an untyped rejection).
  const record_silently_rebound = Boolean(beforeIdentity) && Boolean(afterIdentity)
    && !identitiesEqual(beforeIdentity, afterIdentity);

  const reapprovalId = reapproval && typeof reapproval.approvalId === 'string' ? reapproval.approvalId : null;
  const reapprovalEntry = findApproval(afterList, reapprovalId);
  // An original with no established identity leaves nothing to differ from:
  // never read as a distinct renewal.
  const reapprovalDiffersFromOriginal = Boolean(reapprovalId) && reapprovalId !== originalId && Boolean(reapprovalEntry)
    && Boolean(originalIdentity) && !identitiesEqual(identityOf(reapprovalEntry), originalIdentity);

  return {
    location_scheme_is_app_protocol: location_protocol === 'tauri:',
    approval_recorded,
    unchanged_launch_allowed: isResolvedWithProcess(a),
    rewrite_attempted: isSettled(b),
    rewrite_denied_changed: isDeniedWithCode(b, 'changed-since-approval'),
    shadow_attempted: isSettled(c),
    shadow_denied_shadowed: isDeniedWithCode(c, 'shadowed-path'),
    original_record_unchanged,
    reapproval_launched: reapprovalDiffersFromOriginal && isResolvedWithProcess(d),
    changed_launch_allowed: b.outcome === 'resolved',
    shadow_launch_allowed: c.outcome === 'resolved',
    record_silently_rebound,
  };
}

const OBSERVATION_ORDER = [
  'location_scheme_is_app_protocol', 'approval_recorded', 'unchanged_launch_allowed',
  'rewrite_attempted', 'rewrite_denied_changed', 'shadow_attempted', 'shadow_denied_shadowed',
  'original_record_unchanged', 'reapproval_launched', 'changed_launch_allowed',
  'shadow_launch_allowed', 'record_silently_rebound',
];

/** `summarize`'s facts as `observation=<key> value=<v>` lines, stable
 * declared order (matching `ExecutableIdentityObservations`'s own field
 * order, minus the two gating fields). */
export function formatObservations(summary) {
  const source = summary ?? {};
  return OBSERVATION_ORDER.map((key) => `observation=${key} value=${source[key]}`);
}

/** One `observation=case label=<a|b|c|d> outcome=<json> code=<json|null>`
 * line per case, fixed label order, plus ` process_id=<n>` only for a
 * resolved case. Never a message, a detail, or a path: just enough to
 * confirm which denial code (if any) each case actually hit. */
export function formatCaseLines(cases) {
  const source = cases ?? {};
  return CASE_LABELS.map((label) => {
    const entry = caseOf(source, label);
    const outcome = typeof entry.outcome === 'string' ? entry.outcome : 'pending';
    let line = `observation=case label=${JSON.stringify(label)} outcome=${JSON.stringify(outcome)} code=${JSON.stringify(entry.code ?? null)}`;
    if (outcome === 'resolved' && typeof entry.processId === 'number') line += ` process_id=${entry.processId}`;
    return line;
  });
}

/** The last path component of `value`, tolerating both separators; `null`
 * for a non-string or empty value (mirrors vp-s13-probe.mjs's own
 * unexported `basename`, duplicated here rather than shared -- there is
 * no common util module for these probes to import from). */
function basename(value) {
  if (typeof value !== 'string' || value.length === 0) return null;
  const parts = value.split(/[\\/]/);
  return parts[parts.length - 1] || null;
}

/** The approval-record diff: one `observation=record phase=<before|after>
 * approval_id=<json> basename=<json> size=<json> sha256=<json>
 * status=<json>` line per (id, phase) pair actually present in that
 * phase's list, for each id in `ids` (typically `[originalId,
 * reapprovalId]`, deduplicated). A line is only emitted when that phase's
 * list actually carries the id -- never a fabricated "missing" line.
 *
 * `basename`, never the full `canonicalPath`: the fixture lives under a
 * scratch directory whose name must never appear in retained evidence.
 * `sha256` is kept in full (not `sha256Short`): unlike a path, a digest is
 * not location-sensitive, and the diff's whole point is to show the exact
 * identity bound before and after -- a short prefix could not distinguish
 * a silent rebind to a different file that happens to share one. */
export function formatRecordLines(beforeList, afterList, ids) {
  const idList = [...new Set(Array.isArray(ids) ? ids.filter((id) => typeof id === 'string') : [])];
  const lists = { before: Array.isArray(beforeList) ? beforeList : [], after: Array.isArray(afterList) ? afterList : [] };
  const lines = [];
  for (const id of idList) {
    for (const phase of ['before', 'after']) {
      const entry = findApproval(lists[phase], id);
      if (!entry) continue;
      const evidence = entry.evidence ?? {};
      lines.push(
        `observation=record phase=${JSON.stringify(phase)} approval_id=${JSON.stringify(id)} `
        + `basename=${JSON.stringify(basename(evidence.canonicalPath))} size=${JSON.stringify(evidence.size ?? null)} `
        + `sha256=${JSON.stringify(evidence.sha256 ?? null)} status=${JSON.stringify(entry.status ?? null)}`,
      );
    }
  }
  return lines;
}
