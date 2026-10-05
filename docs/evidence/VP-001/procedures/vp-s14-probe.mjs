// Pure VP-S14 executable-identity in-page script builders
// (desktop-stack-verification-plan.md:135; ADR 0002:67; TM-001 HAR-3/
// HAR-4). `summarize`, `formatObservations`, `formatCaseLines`, and
// `formatRecordLines` -- the pure derive/formatting side that turns a
// driver's captured snapshots into `ExecutableIdentityObservations`
// (tools/evidence-validator/src/derive.rs:479-528) -- live in the sibling
// `vp-s14-observations.mjs`, split out the same way VP-S6 splits
// `vp-s6-xdotool.mjs` (interaction primitives) from
// `vp-s6-observations.mjs` (pure derive helpers). Neither module imports
// the other: this one only builds strings and knows nothing about
// `ExecutableIdentityObservations`; the sibling only shapes data already
// captured and knows nothing about IPC or `window`.
//
// IPC surface, verified against source (never assumed):
// - `executable_pick_and_probe()` (src-tauri/src/ipc/commands.rs:1673-
//   1726): no args; opens the native "Open File" dialog on a blocking
//   thread, probes the selection, resolves `ProbeResultDto { candidateId,
//   evidence }` (dto.rs:1963-1968) or rejects `ShellError`. Blocks on the
//   dialog, so arm/read is split fire-and-forget, like VP-S13.
// - `executable_approve({ candidateId })` (commands.rs:1736-1754):
//   resolves `ApprovalDto` (dto.rs:1981-1989: `approvalId`, `evidence`,
//   `approvedAt`, `status`, `revokedAt`) or rejects `ShellError`.
// - `approvals_list()` (commands.rs:1782-1792): no args; resolves
//   `ApprovalDto[]`.
// - `harness_spawn({ kind, onFrame })` (commands.rs:1024-1052,
//   dto.rs:23-60): `kind` is `HarnessKindDto`, internally tagged on
//   `type` (not `kind`), kebab-case variant, camelCase fields --
//   `{ type: 'approved', approvalId }` for `spawn_approved_harness`
//   (commands.rs:1094-1116). Resolves `ProcessIdDto` (a bare number,
//   dto.rs:196) or rejects `ShellError`.
// - `harness_stop({ id, deadlineMs })` (commands.rs:1935-1948): resolves
//   `ProcessTerminalStateDto` or rejects `ShellError`.
// All five argument names above are verified camelCase against
// `renderer/src/ipc/harness.ts`'s own `invoke(...)` calls (`harnessSpawn`,
// `executablePickAndProbe`, `executableApprove`, `approvalsList`,
// `harnessStop`), which this module's scripts call `window.__TAURI_INTERNALS__.invoke`
// directly, bypassing that renderer module (there is no `@tauri-apps/api`
// import on this probe page).
//
// `ApprovalIdDto` (dto.rs:1839-1876) is a 16-lowercase-hex-character
// string on the wire (a derived `u64` does not survive a JSON number's
// 53-bit mantissa); `CandidateIdDto` (dto.rs:1837) is a bare number
// (serde's default transparent newtype serialization).
//
// `DenialReason` -> `ShellError` (commands.rs:122-151):
// `shadowed-path` carries no `detail` field at all (`ShellError::new`,
// no `with_detail`); `changed-since-approval` carries
// `detail: { recordedSha256Short, observedSha256Short }` (both short hex
// prefixes, untagged + camelCase fields, dto.rs:1744-1759). `detail` is
// `#[serde(skip_serializing_if = "Option::is_none")]` (dto.rs:1773), so a
// denial without detail omits the key entirely rather than sending `null`.
//
// Channel: this page has no `@tauri-apps/api` module, so `buildSpawnScript`
// hand-builds a Channel-compatible `onFrame` argument that reproduces the
// installed `@tauri-apps/api@2.11.1` package's own `Channel` class
// exactly (node_modules/.pnpm/@tauri-apps+api@2.11.1/node_modules/
// @tauri-apps/api/core.js:60-133): `id = window.__TAURI_INTERNALS__.
// transformCallback(onRawMessage)`, plus a `['__TAURI_TO_IPC_KEY__']`
// method and a `toJSON` delegating to it, both returning
// `` `__CHANNEL__:${id}` ``. That shape round-trips correctly because the
// *native*, Rust-injected `window.__TAURI_INTERNALS__.invoke` (tauri-
// 2.11.5 (cargo registry) scripts/core.js:51-53 for `transformCallback`,
// :81-113 for `invoke`/`ipc`) ultimately serializes its whole message
// with `JSON.stringify(message, replacer)` (tauri-2.11.5 scripts/
// process-ipc-message-fn.js): `JSON.stringify` calls `toJSON()`
// automatically before the replacer ever sees the value, so our object
// collapses to the literal string `__CHANNEL__:<id>` on the wire -- the
// exact shape Tauri's `Channel<T>` argument deserializer expects, and the
// same id `transformCallback` registered, so a frame the backend later
// delivers through `runCallback(id, data)` still reaches our callback.
// This is a verified, exact reproduction of the installed version, not an
// approximation: nothing here depends on behavior `@tauri-apps/api` added
// or changed across versions.
//
// Case sequence (T3b's driver mutates files between these calls; this
// module only builds the scripts -- `vp-s14-observations.mjs` derives
// facts from the snapshots the driver captures):
// (a) launch the approved copy of `true`; (b) overwrite its bytes with
// `false`'s, then launch; (c) `ln -sf` it to another copy of `true`, then
// launch; (d) pick the same path again (now resolving through the
// symlink), approve, then launch. `cases` is keyed `a`/`b`/`c`/`d`.

const NO_TAURI_INTERNALS_SCRIPT_PREAMBLE = 'if (window.__TAURI_INTERNALS__) {';

/** Shared by every arm script: the shell's typed `{code, message, detail?}`
 * rejection, or an untyped value, collapsed to one string for the
 * single-slot records (`pick`/`approve`/`list`/`stop`) where only a human-
 * readable rejection matters, never a derive input. */
const RAW_ERROR_HELPER = `
    function rawError(error) {
      if (error && typeof error === 'object' && typeof error.code === 'string') {
        return error.code + (typeof error.message === 'string' ? ': ' + error.message : '');
      }
      return typeof error === 'string' ? error : String(error);
    }
`;

/** Invokes `executable_pick_and_probe` (no args; blocks on the native file
 * dialog, hence fire-and-forget) and records `window.__vpS14.pick`. Stays
 * `{ phase: 'pending' }` when `__TAURI_INTERNALS__` is absent -- never
 * invoked, never a fabricated rejection. */
export function buildPickArmScript() {
  return `
    window.__vpS14 = window.__vpS14 || {};
    window.__vpS14.pick = { phase: 'pending' };
    ${RAW_ERROR_HELPER}
    ${NO_TAURI_INTERNALS_SCRIPT_PREAMBLE}
      try {
        window.__TAURI_INTERNALS__.invoke('executable_pick_and_probe').then(
          function (value) {
            window.__vpS14.pick = { phase: 'resolved', candidateId: value && value.candidateId, evidence: value && value.evidence };
          },
          function (error) { window.__vpS14.pick = { phase: 'rejected', error: rawError(error) }; },
        );
      } catch (error) { window.__vpS14.pick = { phase: 'rejected', error: rawError(error) }; }
    }
  `;
}

/** Reads back the whole `window.__vpS14` snapshot, deep-cloned, plus
 * `location.protocol` -- one generic read for every armed step (pick,
 * approve, list, each spawn case, stop), the same way VP-S13 has exactly
 * one `buildReadScript`. The task's "and a read" for `buildApproveScript`/
 * `buildListScript`/`buildSpawnScript` names this same function: the
 * driver calls it after each arm and stashes the relevant slice itself
 * (e.g. the first `approve` read as `original`, the second as
 * `reapproval`), since a later arm overwrites its own single-slot key but
 * never another one. An empty snapshot, never a throw, when never armed. */
export function buildPickReadScript() {
  return `
    var snapshot = window.__vpS14 ? JSON.parse(JSON.stringify(window.__vpS14)) : {};
    snapshot.location_protocol = location.protocol;
    return snapshot;
  `;
}

/** Invokes `executable_approve({ candidateId })` and records
 * `window.__vpS14.approve`. Called twice over a scenario run (the
 * original candidate, then case (d)'s re-pick); each call overwrites the
 * previous snapshot, so the driver must read between calls. */
export function buildApproveScript(candidateId) {
  return `
    window.__vpS14 = window.__vpS14 || {};
    window.__vpS14.approve = { phase: 'pending' };
    ${RAW_ERROR_HELPER}
    ${NO_TAURI_INTERNALS_SCRIPT_PREAMBLE}
      try {
        window.__TAURI_INTERNALS__.invoke('executable_approve', { candidateId: ${JSON.stringify(candidateId)} }).then(
          function (value) {
            window.__vpS14.approve = { phase: 'resolved', approvalId: value && value.approvalId, evidence: value && value.evidence };
          },
          function (error) { window.__vpS14.approve = { phase: 'rejected', error: rawError(error) }; },
        );
      } catch (error) { window.__vpS14.approve = { phase: 'rejected', error: rawError(error) }; }
    }
  `;
}

/** Invokes `approvals_list()` (no args) and records `window.__vpS14.list`.
 * Called twice (before and after the mutation cases); the driver reads
 * between calls and stashes `approvals` as `beforeList`/`afterList`. */
export function buildListScript() {
  return `
    window.__vpS14 = window.__vpS14 || {};
    window.__vpS14.list = { phase: 'pending' };
    ${RAW_ERROR_HELPER}
    ${NO_TAURI_INTERNALS_SCRIPT_PREAMBLE}
      try {
        window.__TAURI_INTERNALS__.invoke('approvals_list').then(
          function (value) { window.__vpS14.list = { phase: 'resolved', approvals: Array.isArray(value) ? value : [] }; },
          function (error) { window.__vpS14.list = { phase: 'rejected', error: rawError(error) }; },
        );
      } catch (error) { window.__vpS14.list = { phase: 'rejected', error: rawError(error) }; }
    }
  `;
}

/** Invokes `harness_spawn` with `kind: { type: 'approved', approvalId }`
 * (the verified wire shape -- `dto.rs`'s tag field is `type`, not `kind`)
 * and a hand-built, channel-compatible `onFrame` (see the module header
 * for the exact reproduction this relies on). Records
 * `window.__vpS14.cases[label]` as `{ outcome: 'resolved', processId,
 * code: null }` or `{ outcome: 'rejected', code, message, detail }`;
 * `code`/`detail` are `null` for an untyped rejection (a crash, not a
 * catalogue code). Frames delivered on the channel are counted in
 * `frameCount` only, never stored whole -- the channel's own message-
 * reordering buffer is therefore not reproduced here: irrelevant to a
 * count, where every delivery (in any order) still means one frame. */
export function buildSpawnScript(approvalId, label) {
  return `
    window.__vpS14 = window.__vpS14 || {};
    window.__vpS14.cases = window.__vpS14.cases || {};
    var __label = ${JSON.stringify(label)};
    // Captured at arm time: every callback below writes to this record, never
    // a by-label lookup, so a re-armed label is never touched by an earlier
    // spawn's late settlement or frames.
    var __record = { outcome: 'pending', code: null, frameCount: 0 };
    window.__vpS14.cases[__label] = __record;
    function rawError(error) {
      if (error && typeof error === 'object' && typeof error.code === 'string') {
        return { code: error.code, message: typeof error.message === 'string' ? error.message : null, detail: 'detail' in error ? error.detail : null };
      }
      return { code: null, message: typeof error === 'string' ? error : String(error), detail: null };
    }
    function buildChannel() {
      var id = window.__TAURI_INTERNALS__.transformCallback(function (rawMessage) {
        if (rawMessage && typeof rawMessage === 'object' && 'end' in rawMessage) return;
        __record.frameCount += 1;
      });
      var channel = { id: id };
      channel['__TAURI_TO_IPC_KEY__'] = function () { return '__CHANNEL__:' + id; };
      channel.toJSON = function () { return channel['__TAURI_TO_IPC_KEY__'](); };
      return channel;
    }
    ${NO_TAURI_INTERNALS_SCRIPT_PREAMBLE}
      try {
        window.__TAURI_INTERNALS__.invoke('harness_spawn', { kind: { type: 'approved', approvalId: ${JSON.stringify(approvalId)} }, onFrame: buildChannel() }).then(
          function (value) { Object.assign(__record, { outcome: 'resolved', processId: value, code: null }); },
          function (error) {
            var parsed = rawError(error);
            Object.assign(__record, { outcome: 'rejected', code: parsed.code, message: parsed.message, detail: parsed.detail });
          },
        );
      } catch (error) {
        var parsed = rawError(error);
        Object.assign(__record, { outcome: 'rejected', code: parsed.code, message: parsed.message, detail: parsed.detail });
      }
    }
  `;
}

/** A best-effort `harness_stop({ id, deadlineMs })`, recording
 * `window.__vpS14.stop`. Never consumed by `summarize`: an error because
 * the process already exited (`unknown-process`) is expected and not a
 * scenario failure, so this only needs to record what happened, not judge
 * it. `deadlineMs` defaults to 5000, generous for a harness that should
 * already be winding down. */
export function buildStopScript(processId, deadlineMs = 5000) {
  return `
    window.__vpS14 = window.__vpS14 || {};
    window.__vpS14.stop = { phase: 'pending' };
    ${RAW_ERROR_HELPER}
    ${NO_TAURI_INTERNALS_SCRIPT_PREAMBLE}
      try {
        window.__TAURI_INTERNALS__.invoke('harness_stop', { id: ${JSON.stringify(processId)}, deadlineMs: ${JSON.stringify(deadlineMs)} }).then(
          function (value) { window.__vpS14.stop = { phase: 'resolved', outcome: value }; },
          function (error) { window.__vpS14.stop = { phase: 'rejected', error: rawError(error) }; },
        );
      } catch (error) { window.__vpS14.stop = { phase: 'rejected', error: rawError(error) }; }
    }
  `;
}
