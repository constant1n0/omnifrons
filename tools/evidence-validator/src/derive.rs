//! `derive(Observations) -> (Outcome, ObservedState)`: the sole place a
//! VP-S6 result exists (design.md "Honesty machinery"). A pure function --
//! no I/O, no retry -- that never collapses `Outcome` and `ObservedState`
//! into one compound value such as `orphan-risk/uncertain`; a scenario
//! record always carries them as two separate fields (`result`,
//! `observed_state`).

/// The closed `result` outcome (Closed Outcome Vocabulary requirement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Uncertain,
}

/// The `observed_state` a row may additionally carry alongside `Outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedState {
    /// The public state token for a completed, fully proven run.
    Verify,
    /// A recorded pid/starttime is still alive, or termination could not be
    /// proven -- the demonstrated `uncertain` this change exists to
    /// produce.
    OrphanRisk,
}

/// Every fact the procedure can observe, before any outcome is decided.
/// Field names mirror design.md's honesty-machinery observation table. Six
/// independent booleans, not a state machine: each names one fact the
/// procedure either did or did not observe, and [`derive`] is the single
/// place they combine into an outcome.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observations {
    /// AV1 (digest) and AV2 (`--demo-harness` scan) both completed and gated
    /// before the `WebDriver` session was created.
    pub identity_gated: bool,
    /// The descendant process was observed alive before Stop was triggered.
    pub descendant_alive_before_stop: bool,
    /// The product's own `State:` badge confirmed the stop.
    pub stop_confirmed: bool,
    /// Every recorded `(pid, starttime)` was proven gone after the bounded
    /// wait.
    pub all_pids_proven_gone: bool,
    /// A recorded pid is still alive with the same starttime after the
    /// bounded wait.
    pub any_pid_alive_after_wait: bool,
    /// `/proc` enumeration for a recorded pid could not be read.
    pub enumeration_unreadable: bool,
}

/// Maps observations to the `(result, observed_state)` pair a scenario row
/// must carry as two always-separate values.
///
/// `pass` requires every positive proof (design.md "Honesty machinery" #2);
/// any absent or unreadable observation yields `uncertain`, never a pass. A
/// pid proven alive after the wait always yields `fail`, since it overrides
/// every other observation.
#[must_use]
pub fn derive(observations: Observations) -> (Outcome, ObservedState) {
    if observations.any_pid_alive_after_wait {
        return (Outcome::Fail, ObservedState::OrphanRisk);
    }
    if observations.identity_gated
        && observations.descendant_alive_before_stop
        && observations.stop_confirmed
        && observations.all_pids_proven_gone
        && !observations.enumeration_unreadable
    {
        return (Outcome::Pass, ObservedState::Verify);
    }
    (Outcome::Uncertain, ObservedState::OrphanRisk)
}

/// The public `observed_state` a VP-S1 (CSP) row may additionally carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspObservedState {
    /// The public state token for a fully proven, enforced baseline.
    Enforced,
    /// The baseline was demonstrably not enforced: the inline script ran,
    /// or the external fetch was dispatched.
    Bypassed,
    /// Any fact absent, unreadable, or short of full proof; nothing was
    /// demonstrated either way.
    Unverified,
}

/// Every fact `docs/evidence/VP-001/procedures/vp-s1-probe.mjs` can
/// observe, before any outcome is decided. Mirrors [`Observations`]'s own
/// shape: independent booleans, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CspObservations {
    /// AV1/AV2 both gated before the session was created (`vp-s1-linux.sh`).
    pub identity_gated: bool,
    /// The packaged custom-protocol scheme was observed, never `http:`.
    pub location_scheme_is_app_protocol: bool,
    /// A `securitypolicyviolation` matched `script-src*` (inline script).
    pub inline_script_violation: bool,
    /// The inline script actually ran.
    pub inline_script_ran: bool,
    /// A `securitypolicyviolation` matched `connect-src*` (external fetch).
    pub external_fetch_violation: bool,
    /// The external fetch resolved: the request was dispatched.
    pub external_fetch_resolved: bool,
    /// A `securitypolicyviolation` matched `frame-src*`/`child-src*`.
    pub framed_context_violation: bool,
    /// A policy dump (a violation's `originalPolicy`, or `<meta>`) was captured.
    pub policy_dump_captured: bool,
    /// A third-party app surface was actually exercised by this run.
    pub third_party_surface_exercised: bool,
}

/// Maps VP-S1 observations to `(result, observed_state)`.
///
/// `inline_script_ran` or `external_fetch_resolved` always yields `Fail`,
/// overriding every other observation. `Pass` requires every positive proof, including
/// `third_party_surface_exercised`: no such surface exists in this renderer
/// today (ADR-0004), so `Pass` is unreachable until one does, by the
/// maintainer's decision -- not a bug here. Anything short of that is
/// `Uncertain`.
///
/// `CspObservedState`'s variant names are chosen here, not taken from
/// RCS-001's signal mapping (docs/renderer-content-security.md § Signal
/// mapping), which has no CSP entry -- the same gap VP-S6-03's
/// `proven-gone` token addressed for containment (records.md).
#[must_use]
pub fn derive_csp(observations: CspObservations) -> (Outcome, CspObservedState) {
    if observations.inline_script_ran || observations.external_fetch_resolved {
        return (Outcome::Fail, CspObservedState::Bypassed);
    }
    if observations.identity_gated
        && observations.location_scheme_is_app_protocol
        && observations.inline_script_violation
        && observations.external_fetch_violation
        && observations.framed_context_violation
        && observations.policy_dump_captured
        && observations.third_party_surface_exercised
    {
        return (Outcome::Pass, CspObservedState::Enforced);
    }
    (Outcome::Uncertain, CspObservedState::Unverified)
}

/// The public `observed_state` a VP-S3 (typed-IPC bridge confinement) row
/// may additionally carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcObservedState {
    /// The public state token for a baseline whose typed-IPC bridge stayed
    /// inside its documented `connect-src` source, with every proof held.
    Confined,
    /// The captured `connect-src` is not exactly the bridge source RCS-001
    /// documents for the baseline's OS -- the plan's own `undocumented-bridge`
    /// token (VP-001 § Signal mapping).
    UndocumentedBridge,
    /// A bridge call fell back to `postMessage`, or a `connect-src`
    /// violation fired for the bridge origin itself.
    FellBack,
    /// An `artifact:` load succeeded although RCS-001 documents no handler
    /// for it yet (renderer-content-security.md § CSP baseline).
    ArtifactServed,
    /// A call to a command the app never registered resolved instead of
    /// being rejected.
    UnregisteredCommandServed,
    /// Any fact absent, unreadable, or short of full proof; nothing was
    /// demonstrated either way.
    Unverified,
}

/// Every fact a VP-S3 probe can observe, before any outcome is decided.
/// Mirrors [`CspObservations`]'s own shape: independent booleans, not a
/// state machine, one baseline (OS) at a time.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IpcObservations {
    /// AV1/AV2 both gated before the session was created.
    pub identity_gated: bool,
    /// The packaged custom-protocol scheme was observed, never `http:`.
    pub location_scheme_is_app_protocol: bool,
    /// A `connect-src` policy dump (a violation's `originalPolicy`, or
    /// `<meta>`) was captured for this baseline's OS.
    pub connect_src_policy_captured: bool,
    /// The captured `connect-src` is exactly the bridge source RCS-001
    /// documents for this baseline's OS (`ipc:` on Linux/macOS,
    /// `http://ipc.localhost` on Windows).
    pub connect_src_is_documented_source_for_os: bool,
    /// A renderer->core call to a command the app registered resolved.
    pub registered_call_completed: bool,
    /// The registered call's `fetch` to the bridge origin was observed
    /// (`ipc://localhost/<cmd>` or `http://ipc.localhost/<cmd>`), not
    /// `postMessage`.
    pub registered_call_used_custom_protocol: bool,
    /// The typed-IPC bridge fell back to `postMessage` for any call -- the
    /// framework's own fallback when the custom-protocol `fetch` fails
    /// (renderer-content-security.md § CSP baseline).
    pub postmessage_fallback_observed: bool,
    /// A `securitypolicyviolation` matched `connect-src*` for the bridge
    /// origin itself.
    pub bridge_connect_src_violation: bool,
    /// A call to a command the app never registered was rejected.
    pub unregistered_call_rejected: bool,
    /// A call to a command the app never registered resolved anyway.
    pub unregistered_call_resolved: bool,
    /// An `artifact:` load (`img-src`/`media-src`) was attempted against
    /// the reserved scheme.
    pub artifact_load_attempted: bool,
    /// The `artifact:` load actually succeeded, although RCS-001 documents
    /// no handler for it yet.
    pub artifact_load_succeeded: bool,
    /// `src-tauri/tests/protocol_inventory.rs` passed on the run's head
    /// commit -- the static half of this evidence.
    pub static_inventory_pinned: bool,
}

/// Maps VP-S3 observations to `(result, observed_state)`.
///
/// Demonstrated failures override every other observation, checked in this
/// fixed order (first match wins):
///
/// 1. `connect_src_policy_captured && !connect_src_is_documented_source_for_os`
///    yields `(Fail, UndocumentedBridge)` -- the plan's own
///    `undocumented-bridge` token.
/// 2. `postmessage_fallback_observed || bridge_connect_src_violation` yields
///    `(Fail, FellBack)`.
/// 3. `artifact_load_succeeded` yields `(Fail, ArtifactServed)`.
/// 4. `unregistered_call_resolved` yields `(Fail, UnregisteredCommandServed)`.
///
/// `Pass` requires every positive proof: `identity_gated`,
/// `location_scheme_is_app_protocol`, `connect_src_policy_captured`,
/// `connect_src_is_documented_source_for_os`, `registered_call_completed`,
/// `registered_call_used_custom_protocol`, `unregistered_call_rejected`,
/// `artifact_load_attempted` (with `artifact_load_succeeded` false, already
/// excluded above), and `static_inventory_pinned`. Anything else is
/// `Uncertain`.
///
/// Unlike [`derive_csp`], `Pass` is reachable here -- per baseline: this
/// function derives one OS's result at a time, and a `Pass` on one baseline
/// never transfers to another OS's row (VP-001-R15).
#[must_use]
pub fn derive_ipc(observations: IpcObservations) -> (Outcome, IpcObservedState) {
    if observations.connect_src_policy_captured
        && !observations.connect_src_is_documented_source_for_os
    {
        return (Outcome::Fail, IpcObservedState::UndocumentedBridge);
    }
    if observations.postmessage_fallback_observed || observations.bridge_connect_src_violation {
        return (Outcome::Fail, IpcObservedState::FellBack);
    }
    if observations.artifact_load_succeeded {
        return (Outcome::Fail, IpcObservedState::ArtifactServed);
    }
    if observations.unregistered_call_resolved {
        return (Outcome::Fail, IpcObservedState::UnregisteredCommandServed);
    }
    if observations.identity_gated
        && observations.location_scheme_is_app_protocol
        && observations.connect_src_policy_captured
        && observations.connect_src_is_documented_source_for_os
        && observations.registered_call_completed
        && observations.registered_call_used_custom_protocol
        && observations.unregistered_call_rejected
        && observations.artifact_load_attempted
        && observations.static_inventory_pinned
    {
        return (Outcome::Pass, IpcObservedState::Confined);
    }
    (Outcome::Uncertain, IpcObservedState::Unverified)
}

/// The public `observed_state` a VP-S13 (typed IPC boundary; path attacks
/// rejected) row may additionally carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcBoundaryObservedState {
    /// The filesystem outside the trap workspace changed between the
    /// before/after snapshot -- the escape this scenario exists to catch.
    Escaped,
    /// The outside canary file's content appeared in any response.
    Leaked,
    /// An attack invoke resolved instead of being rejected: best-effort
    /// interpretation of a malformed or path-attack payload, not the typed
    /// rejection RCS-001-R14 requires.
    BestEffort,
    /// The static capability test showed a generic shell or filesystem
    /// capability still reachable from the renderer.
    GenericCapability,
    /// The public state token for a baseline where every sent attack was
    /// rejected with a typed error and nothing outside the workspace was
    /// touched.
    Rejected,
    /// Any fact absent, unreadable, or short of full proof; nothing was
    /// demonstrated either way.
    Unverified,
}

/// Every fact a VP-S13 probe can observe, before any outcome is decided.
/// Mirrors [`IpcObservations`]'s own shape: independent booleans, not a
/// state machine, one baseline (OS) at a time. The static half (the
/// capability and validator Rust tests, run on the run's own head) and the
/// runtime half (the packaged-build `WebDriver` probe against a prepared
/// trap workspace) each contribute their own facts below.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IpcBoundaryObservations {
    /// AV1/AV2 both gated before the session was created.
    pub identity_gated: bool,
    /// The packaged custom-protocol scheme was observed, never `http:`.
    pub location_scheme_is_app_protocol: bool,
    /// The probe confirmed the trap workspace the shell driver prepared is
    /// the current workspace, selected through the native picker -- so a
    /// rejection below is not merely "no workspace selected".
    pub workspace_selected: bool,
    /// The static capability test (no generic shell or filesystem
    /// capability reachable from the renderer) passed on this run's head
    /// commit -- the retained `vp-001-vp-s13-static-boundary.txt`
    /// artifact's `gate=static-boundary-section name=capabilities` section,
    /// which ends `status=0` only when every test it names ran and passed
    /// (a renamed test fails it rather than silently matching nothing).
    pub generic_capability_absent_pinned: bool,
    /// The static validator Rust tests (path and reference validation)
    /// passed on this run's head commit -- the same artifact's
    /// `gate=static-boundary-section name=validators` section, under the
    /// same proof.
    pub validators_pinned: bool,
    /// At least one payload of each malformed class (wrong types, missing
    /// fields, unknown fields, oversized or out-of-range ids including
    /// above 2^53) was sent to a side-effect-free path-bearing command.
    pub malformed_payloads_sent: bool,
    /// At least one payload of each path-attack class (symlinks escaping
    /// the workspace, traversal and absolute names, NUL and control
    /// characters) was sent to a side-effect-free path-bearing command.
    pub path_attacks_sent: bool,
    /// Every sent attack's invoke rejected.
    pub every_attack_rejected: bool,
    /// Each rejection carried a typed error code or message, not a generic
    /// crash or timeout.
    pub every_rejection_typed: bool,
    /// The before/after filesystem snapshot outside the workspace was
    /// captured.
    pub outside_snapshot_taken: bool,
    /// The outside snapshot changed between before and after.
    pub outside_target_modified: bool,
    /// The outside canary file's content appeared in any response.
    pub canary_leaked: bool,
    /// Any sent attack's invoke resolved instead of being rejected.
    pub attack_accepted: bool,
    /// The static capability test showed a generic shell or filesystem
    /// capability.
    pub generic_capability_observed: bool,
}

/// Maps VP-S13 observations to `(result, observed_state)`.
///
/// Demonstrated failures override every other observation, checked in this
/// fixed order (first match wins). `Escaped` and `Leaked` come first
/// because an observed escape -- the trap workspace boundary itself giving
/// way -- outweighs every other fact this probe could have recorded,
/// including a clean rejection log for every other attack:
///
/// 1. `outside_target_modified` yields `(Fail, Escaped)`.
/// 2. `canary_leaked` yields `(Fail, Leaked)`.
/// 3. `attack_accepted` yields `(Fail, BestEffort)` -- an attack invoke
///    resolved instead of being rejected.
/// 4. `generic_capability_observed` yields `(Fail, GenericCapability)`.
///
/// `attack_accepted` and `every_attack_rejected` are deliberately separate
/// facts, mirroring [`derive_ipc`]'s `artifact_load_attempted` /
/// `artifact_load_succeeded` split: an attack that was never sent is
/// neither accepted nor rejected. Without `malformed_payloads_sent` and
/// `path_attacks_sent` as their own required proofs, `every_attack_rejected`
/// would hold vacuously on an empty attack set and `attack_accepted` would
/// stay unset, which must not read as `Pass`.
///
/// `Pass` requires every positive proof: `identity_gated`,
/// `location_scheme_is_app_protocol`, `workspace_selected`,
/// `generic_capability_absent_pinned`, `validators_pinned`,
/// `malformed_payloads_sent`, `path_attacks_sent`, `every_attack_rejected`,
/// `every_rejection_typed`, and `outside_snapshot_taken`. An unproved fact
/// never counts as a failure on its own: absent proof falls through to
/// `Uncertain`, never to one of the `Fail` states above, which are reserved
/// for an actually demonstrated failure.
///
/// An untyped rejection (a crash or timeout instead of a typed error) is
/// deliberately not a `Fail` state: it is still a rejection, so it is not
/// best-effort interpretation, but it does not show why the payload was
/// refused, and a timeout can come from the harness itself. It leaves
/// `every_rejection_typed` unproved, so the row falls to `Uncertain`, and the
/// probe's own per-attack outcome lines record what actually happened.
#[must_use]
pub fn derive_ipc_boundary(
    observations: IpcBoundaryObservations,
) -> (Outcome, IpcBoundaryObservedState) {
    if observations.outside_target_modified {
        return (Outcome::Fail, IpcBoundaryObservedState::Escaped);
    }
    if observations.canary_leaked {
        return (Outcome::Fail, IpcBoundaryObservedState::Leaked);
    }
    if observations.attack_accepted {
        return (Outcome::Fail, IpcBoundaryObservedState::BestEffort);
    }
    if observations.generic_capability_observed {
        return (Outcome::Fail, IpcBoundaryObservedState::GenericCapability);
    }
    if observations.identity_gated
        && observations.location_scheme_is_app_protocol
        && observations.workspace_selected
        && observations.generic_capability_absent_pinned
        && observations.validators_pinned
        && observations.malformed_payloads_sent
        && observations.path_attacks_sent
        && observations.every_attack_rejected
        && observations.every_rejection_typed
        && observations.outside_snapshot_taken
    {
        return (Outcome::Pass, IpcBoundaryObservedState::Rejected);
    }
    (Outcome::Uncertain, IpcBoundaryObservedState::Unverified)
}
