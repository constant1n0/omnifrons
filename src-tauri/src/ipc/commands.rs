//! The demo-harness IPC commands (`harness_spawn`, `harness_stop`,
//! `harness_observe`) and, as of the spike slice-2 spike, the
//! executable-identity-and-approval commands
//! (`executable_pick_and_probe`, `executable_approve`,
//! `executable_revoke`, `approvals_list`). Registered next to
//! `shell_health` (`crate::shell_health`).
//!
//! No program path or argument vector for a demo-harness request ever
//! crosses IPC (`docs/spike-log.md` § IPC contract): the renderer names a
//! [`dto::HarnessKindDto`] plus small bounded numbers, validated into an
//! `omnifrons_app::HarnessRequest` before the supervisor ever sees it.
//! `harness_spawn`'s `kind: "approved"` case is the one path that does
//! reach a real, caller-supplied executable, and only ever by canonical
//! path after a `LaunchGate` decision -- never a raw argument vector.
//! Every error surfaces as a [`dto::ShellError`] with a fixed catalogue
//! message -- never the underlying `SupervisorError::Spawn`'s `io::Error`
//! text, which can carry a real filesystem path.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use omnifrons_app::{
    ApprovalStore, ApprovalStoreError, ExecutableProber, GateDecision, HarnessKind, HarnessRequest,
    InvalidRequest, OutputFrame, ProcessId, ProcessOutput, ProcessSupervisor, SupervisorError,
};
use omnifrons_domain::executable::{ApprovalId, DenialReason};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::executable_state::{CandidateId, CandidateTable, ExecutableState, ShellLaunchGate};
use crate::ipc::dto::{
    ApprovalDto, CandidateIdDto, EvidenceDto, HarnessFrame, HarnessKindDto, ProbeResultDto,
    ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto, ShellError, ShellErrorCode,
    ShellErrorDetail,
};

impl From<SupervisorError> for ShellError {
    /// Maps every `SupervisorError` to a fixed catalogue message. Never
    /// the underlying error's own text: `SupervisorError::Spawn` wraps an
    /// `io::Error` whose `Display` text can (and for a nonexistent-program
    /// error, does) contain the attempted program path.
    fn from(error: SupervisorError) -> Self {
        let (code, message) = match error {
            SupervisorError::Spawn(_) => (
                ShellErrorCode::SpawnFailed,
                "failed to start the requested process",
            ),
            SupervisorError::UnknownProcess => (
                ShellErrorCode::UnknownProcess,
                "no process with that id is known",
            ),
            SupervisorError::AlreadySubscribed => (
                ShellErrorCode::AlreadySubscribed,
                "output for that process id is already subscribed",
            ),
            SupervisorError::TooManyProcesses => (
                ShellErrorCode::TooManyProcesses,
                "too many processes are already running",
            ),
        };
        Self::new(code, message)
    }
}

impl From<InvalidRequest> for ShellError {
    fn from(_: InvalidRequest) -> Self {
        Self::new(
            ShellErrorCode::InvalidRequest,
            "the requested rate_hz or lines value is out of the accepted range",
        )
    }
}

impl From<ApprovalStoreError> for ShellError {
    /// Maps every `ApprovalStoreError` to the single
    /// `approval-store-unavailable` code: the four closed reasons a store
    /// operation can fail (unreadable, corrupt, a torn write, a colliding
    /// derived id) are all, from a caller's perspective, "the approval
    /// store is not usable right now" -- none of them is actionable
    /// differently by a caller, so splitting them into separate codes
    /// would add surface with no behavioral payoff.
    fn from(_: ApprovalStoreError) -> Self {
        Self::new(
            ShellErrorCode::ApprovalStoreUnavailable,
            "the approval store is unavailable",
        )
    }
}

impl From<DenialReason> for ShellError {
    /// Maps a `LaunchGate::decide` denial to its catalogue error.
    /// `ChangedSinceApproval` is the one case carrying structured detail
    /// (`ShellErrorDetail`): both digests as short hex prefixes, never in
    /// the message text itself (`docs/spike-log.md` § Slice 2).
    fn from(reason: DenialReason) -> Self {
        match reason {
            DenialReason::Unapproved => Self::new(
                ShellErrorCode::Unapproved,
                "this executable has not been approved",
            ),
            DenialReason::Revoked => Self::new(
                ShellErrorCode::Revoked,
                "the approval for this executable was revoked",
            ),
            DenialReason::ShadowedPath { .. } => Self::new(
                ShellErrorCode::ShadowedPath,
                "the approved path now resolves somewhere else",
            ),
            DenialReason::ChangedSinceApproval { recorded, observed } => Self::with_detail(
                ShellErrorCode::ChangedSinceApproval,
                "the executable's content has changed since it was approved",
                ShellErrorDetail {
                    recorded_sha256_short: recorded.short_hex(),
                    observed_sha256_short: observed.short_hex(),
                },
            ),
            DenialReason::ProbeFailed(ref outcome) => Self::from_probe_failure(outcome),
        }
    }
}

/// The accepted range for `harness_stop`'s `deadline_ms`, inclusive on both
/// ends. A caller-supplied deadline is IPC-controlled input like any other
/// argument, and an unbounded value would let a single `harness_stop` call
/// block its blocking-thread slot indefinitely (`docs/spike-log.md` § IPC
/// contract).
const MIN_DEADLINE_MS: u64 = 1;
const MAX_DEADLINE_MS: u64 = 30_000;

/// Validate `deadline_ms` into a bounded [`Duration`], or a catalogue
/// [`ShellError`] if it falls outside `1..=30_000`.
fn validate_deadline_ms(deadline_ms: u64) -> Result<Duration, ShellError> {
    if (MIN_DEADLINE_MS..=MAX_DEADLINE_MS).contains(&deadline_ms) {
        Ok(Duration::from_millis(deadline_ms))
    } else {
        Err(ShellError::new(
            ShellErrorCode::InvalidRequest,
            "deadlineMs must be between 1 and 30000",
        ))
    }
}

/// Run `f` against a cloned handle to the managed supervisor, on a blocking
/// thread, so a blocking `ProcessSupervisor` call (`spawn`/`stop`/
/// `subscribe`) never stalls the async executor.
///
/// `TokioProcessSupervisor` is itself a cheap `Clone` handle over shared
/// state (`omnifrons_supervisor`'s own doc comment), managed with no outer
/// `Mutex`: cloning it here, once per command, is what lets an unrelated
/// command (say, `harness_observe` of a different process) run concurrently
/// with a long-running `harness_stop` instead of queuing behind it -- a
/// single shared `Mutex<TokioProcessSupervisor>` would otherwise hold the
/// lock for that call's entire, deadline-bounded duration.
///
/// Takes `app` rather than a `State<'_, _>` directly: `State` borrows from
/// the app and is not `'static`, so it cannot be moved into
/// `spawn_blocking`'s closure -- fetching it again, inside the closure,
/// via `AppHandle::state` is the documented way to move managed state onto
/// another thread (Tauri's state-management docs: "if you need to move
/// the state into a thread where using an `AppHandle` is easier").
///
/// # Panics
///
/// Panics if the blocking task itself panics.
async fn with_supervisor<T, F>(app: AppHandle, f: F) -> T
where
    F: FnOnce(&mut TokioProcessSupervisor) -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        f(&mut supervisor)
    })
    .await
    .expect("the blocking supervisor task panicked")
}

/// Start a detached forwarder thread that relays every captured frame from
/// `receiver` onto `on_frame` until the channel closes (the frontend
/// navigated away or dropped it) or the process's output channel itself
/// closes (the process is confirmed terminal and fully drained). Shared
/// by both the demo-harness and approved-launch spawn paths.
fn forward_output(id: ProcessId, receiver: Receiver<OutputFrame>, on_frame: Channel<HarnessFrame>) {
    let id_dto = ProcessIdDto::from(id);
    std::thread::spawn(move || {
        let mut send_failures = 0u32;
        for frame in receiver {
            if on_frame
                .send(HarnessFrame::from_domain(id_dto, frame))
                .is_err()
            {
                send_failures += 1;
                break;
            }
        }
        if send_failures > 0 {
            tracing::warn!(
                pid = id_dto.0,
                send_failures,
                "harness output forwarder stopped after a channel send failure"
            );
        }
    });
}

/// Spawn a harness or an approved executable, and stream its captured
/// output over `on_frame`. `kind` names one of the two synthetic demo
/// behaviors, or a real, previously approved executable
/// (`docs/spike-log.md` § Slice 2) -- see [`HarnessKindDto`]'s own doc
/// comment for why the wire shape is tagged rather than flat.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::InvalidRequest`] if a demo
/// kind's `rateHz`/`lines` are out of range, the mapped [`SupervisorError`]
/// if the process could not be started, or -- for `kind: "approved"` --
/// the mapped [`DenialReason`] if the `LaunchGate` denies the launch.
#[tauri::command]
pub async fn harness_spawn(
    app: AppHandle,
    kind: HarnessKindDto,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    match kind {
        HarnessKindDto::DemoLines { rate_hz, lines } => {
            spawn_demo_harness(app, HarnessKind::DemoLines, rate_hz, lines, on_frame).await
        }
        HarnessKindDto::DemoIgnoresSigterm { rate_hz, lines } => {
            spawn_demo_harness(
                app,
                HarnessKind::DemoIgnoresSigterm,
                rate_hz,
                lines,
                on_frame,
            )
            .await
        }
        HarnessKindDto::Approved { approval_id } => {
            spawn_approved_harness(app, approval_id.into(), on_frame).await
        }
    }
}

/// Validates `rate_hz`/`lines` into an `HarnessRequest` for `kind`, spawns
/// and subscribes on a blocking thread, then starts the forwarder thread.
async fn spawn_demo_harness(
    app: AppHandle,
    kind: HarnessKind,
    rate_hz: u16,
    lines: u32,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let request = HarnessRequest::new(kind, rate_hz, lines)?;

    let (id, receiver) = with_supervisor(app, move |supervisor| {
        let id = supervisor.spawn_harness(request)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

    forward_output(id, receiver, on_frame);
    Ok(ProcessIdDto::from(id))
}

/// Decide (via the managed `LaunchGate`) whether `approval_id` may launch
/// right now, and if so, spawn *the exact `ProbedExecutable` that decision
/// re-probed* with no arguments and stream its captured output over
/// `on_frame`.
///
/// Unlike an earlier version of this function, there is no separate
/// approval-store lookup for the canonical path once `decide` reports
/// `Allowed`: the decision itself already carries the identity and open
/// handle its own re-probe produced
/// (`omnifrons_app::launch_gate::GateDecision::Allowed`), so this hands
/// that straight to `spawn_approved` rather than re-opening the path a
/// second time by name, which would reopen a TOCTOU window `decide`'s own
/// re-probe had just closed.
///
/// # Errors
///
/// Returns the mapped [`DenialReason`] if the gate denies the launch, or
/// the mapped [`SupervisorError`] if the process could not be started.
async fn spawn_approved_harness(
    app: AppHandle,
    approval_id: ApprovalId,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let decision = with_gate(app.clone(), move |gate| gate.decide(approval_id)).await;
    let executable = match decision {
        GateDecision::Allowed { executable, .. } => executable,
        GateDecision::Denied(reason) => return Err(ShellError::from(reason)),
    };

    let display_path = executable.identity.canonical_path.clone();
    let (id, receiver) = with_supervisor(app, move |supervisor| {
        let id = supervisor.spawn_approved(executable.handle, display_path)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

    forward_output(id, receiver, on_frame);
    Ok(ProcessIdDto::from(id))
}

/// Run `f` against a locked handle to the managed `LaunchGate`, on a
/// blocking thread, so `decide` (which re-probes the filesystem) and
/// `approve_candidate`/`revoke` (which touch the approval store) never
/// stall the async executor.
///
/// # Panics
///
/// Panics if the blocking task itself panics, or if the gate's mutex was
/// poisoned by a prior panic while held.
async fn with_gate<T, F>(app: AppHandle, f: F) -> T
where
    F: FnOnce(&mut ShellLaunchGate) -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ExecutableState>();
        let mut gate = state
            .gate
            .lock()
            .expect("launch gate mutex poisoned by a prior panic");
        f(&mut gate)
    })
    .await
    .expect("the blocking launch-gate task panicked")
}

/// Look up `candidate_id` in `candidates`, or the catalogue `no-candidate`
/// error if it is not known (never probed, or evicted from the bounded
/// table).
fn find_candidate(
    candidates: &CandidateTable,
    candidate_id: CandidateIdDto,
) -> Result<omnifrons_domain::executable::ExecutableIdentity, ShellError> {
    candidates
        .get(CandidateId::from_raw(candidate_id.0))
        .cloned()
        .ok_or_else(|| {
            ShellError::new(
                ShellErrorCode::NoCandidate,
                "no probed candidate with that id is known",
            )
        })
}

/// Open the native file picker and probe whatever the user selected.
///
/// The picker itself and the probe both run on a blocking thread
/// (`tauri::async_runtime::spawn_blocking`): `blocking_pick_file` would
/// deadlock if called directly from the async executor's own thread (it
/// dispatches the dialog back to the platform main thread and blocks
/// waiting for a response), and probing streams a file's content through a
/// synchronous hasher.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::NoCandidate`] if no file
/// was selected or its path could not be resolved, or the mapped
/// [`ProbeOutcome`] failure otherwise.
#[tauri::command]
pub async fn executable_pick_and_probe(app: AppHandle) -> Result<ProbeResultDto, ShellError> {
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog_app.dialog().file().blocking_pick_file()
    })
    .await
    .expect("the blocking file-picker task panicked");

    let Some(picked) = picked else {
        return Err(ShellError::new(
            ShellErrorCode::NoCandidate,
            "no file was selected",
        ));
    };
    let candidate_path = picked.into_path().map_err(|_| {
        ShellError::new(
            ShellErrorCode::NoCandidate,
            "the selected file's path could not be resolved",
        )
    })?;

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        omnifrons_adapters::FsExecutableProber::new().probe(&candidate_path)
    })
    .await
    .expect("the blocking probe task panicked");

    match outcome {
        omnifrons_app::ProbeOutcome::Identity(executable) => {
            // Only the identity is kept in the candidate table -- the
            // handle this probe opened is dropped here. Approval re-probes
            // at launch time anyway (`LaunchGate::decide`), so a stale
            // handle held from pick-and-probe time would buy nothing.
            let identity = executable.identity;
            let state = app.state::<ExecutableState>();
            let candidate_id = state
                .candidates
                .lock()
                .expect("candidate table mutex poisoned by a prior panic")
                .insert(identity.clone());
            Ok(ProbeResultDto {
                candidate_id: CandidateIdDto(candidate_id.raw()),
                evidence: EvidenceDto::from_identity(&identity),
            })
        }
        other => {
            let failure = other
                .as_domain_failure()
                .expect("a non-Identity ProbeOutcome always maps to a domain failure");
            Err(ShellError::from_probe_failure(&failure))
        }
    }
}

/// Approve a previously probed candidate.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::NoCandidate`] if
/// `candidate_id` is not known, or the mapped [`ApprovalStoreError`] if
/// the approval could not be recorded.
#[tauri::command]
pub async fn executable_approve(
    app: AppHandle,
    candidate_id: CandidateIdDto,
) -> Result<ApprovalDto, ShellError> {
    let identity = {
        let state = app.state::<ExecutableState>();
        let candidates = state
            .candidates
            .lock()
            .expect("candidate table mutex poisoned by a prior panic");
        find_candidate(&candidates, candidate_id)?
    };

    let record = with_gate(app, move |gate| gate.approve_candidate(identity))
        .await
        .map_err(ShellError::from)?;

    Ok(ApprovalDto::from_record(&record))
}

/// Revoke a previously recorded approval.
///
/// A no-op, not an error, if `approval_id` is not on record at all (see
/// `omnifrons_app::ApprovalStore::revoke`'s own contract).
///
/// # Errors
///
/// Returns the mapped [`ApprovalStoreError`] if the revocation could not
/// be written.
#[tauri::command]
pub async fn executable_revoke(
    app: AppHandle,
    approval_id: crate::ipc::dto::ApprovalIdDto,
) -> Result<(), ShellError> {
    with_gate(app, move |gate| gate.revoke(approval_id.into()))
        .await
        .map_err(ShellError::from)
}

/// List every approval on record.
///
/// # Errors
///
/// Returns the mapped [`ApprovalStoreError`] if the store could not be
/// read.
#[tauri::command]
pub async fn approvals_list(app: AppHandle) -> Result<Vec<ApprovalDto>, ShellError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ExecutableState>();
        let store = state.open_store()?;
        let records = ApprovalStore::list(&store)?;
        Ok::<_, ApprovalStoreError>(records.iter().map(ApprovalDto::from_record).collect())
    })
    .await
    .expect("the blocking approvals-list task panicked")
    .map_err(ShellError::from)
}

/// Stop a spawned demo harness, gracefully then forcefully, within
/// `deadline_ms`.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::InvalidRequest`] if
/// `deadline_ms` is `0` or exceeds `30_000`, or with
/// [`ShellErrorCode::UnknownProcess`] if `id` is not known to the
/// supervisor.
#[tauri::command]
pub async fn harness_stop(
    app: AppHandle,
    id: ProcessIdDto,
    deadline_ms: u64,
) -> Result<ProcessTerminalStateDto, ShellError> {
    let deadline = validate_deadline_ms(deadline_ms)?;
    with_supervisor(app, move |supervisor| {
        supervisor
            .stop(id.into(), deadline)
            .map(ProcessTerminalStateDto::from)
            .map_err(ShellError::from)
    })
    .await
}

/// Observe a spawned demo harness's current status.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::UnknownProcess`] if `id`
/// is not known to the supervisor.
#[tauri::command]
pub async fn harness_observe(
    app: AppHandle,
    id: ProcessIdDto,
) -> Result<ProcessStatusDto, ShellError> {
    with_supervisor(app, move |supervisor| {
        supervisor
            .observe(id.into())
            .map(ProcessStatusDto::from)
            .ok_or_else(|| ShellError::from(SupervisorError::UnknownProcess))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{ShellError, find_candidate};
    use crate::executable_state::CandidateTable;
    use crate::ipc::dto::{CandidateIdDto, ShellErrorCode};
    use omnifrons_app::SupervisorError;
    use omnifrons_domain::executable::{DenialReason, Sha256Digest};

    #[test]
    fn spawn_error_maps_to_a_catalogue_message_with_no_path() {
        let secret_path_error = SupervisorError::Spawn(
            "No such file or directory (os error 2): /home/user/secret/path".to_string(),
        );

        let mapped = ShellError::from(secret_path_error);

        assert_eq!(mapped.code, ShellErrorCode::SpawnFailed);
        assert!(
            !mapped.message.contains('/'),
            "the catalogue message must never echo the underlying error's path, got: {}",
            mapped.message
        );
    }

    #[test]
    fn unknown_process_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::UnknownProcess);
        assert_eq!(mapped.code, ShellErrorCode::UnknownProcess);
    }

    #[test]
    fn already_subscribed_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::AlreadySubscribed);
        assert_eq!(mapped.code, ShellErrorCode::AlreadySubscribed);
    }

    #[test]
    fn too_many_processes_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::TooManyProcesses);
        assert_eq!(mapped.code, ShellErrorCode::TooManyProcesses);
    }

    #[test]
    fn deadline_ms_zero_is_rejected_as_invalid_request() {
        let error = super::validate_deadline_ms(0).unwrap_err();
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn deadline_ms_above_30_000_is_rejected_as_invalid_request() {
        let error = super::validate_deadline_ms(30_001).unwrap_err();
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn deadline_ms_boundaries_are_accepted() {
        assert!(super::validate_deadline_ms(1).is_ok());
        assert!(super::validate_deadline_ms(30_000).is_ok());
    }

    #[test]
    fn approve_with_unknown_candidate_is_no_candidate() {
        let candidates = CandidateTable::default();

        let error = find_candidate(&candidates, CandidateIdDto(999)).unwrap_err();

        assert_eq!(error.code, ShellErrorCode::NoCandidate);
    }

    /// R3-009: `executable_approve` (via `find_candidate`) on a candidate
    /// id that has since been evicted from the bounded table must report
    /// the same `no-candidate` code as one that was never probed at all.
    #[test]
    fn approve_with_an_evicted_candidate_is_no_candidate() {
        use omnifrons_domain::executable::{ExecutableIdentity, PlatformEvidence};
        use std::path::PathBuf;

        fn identity(n: u8) -> ExecutableIdentity {
            ExecutableIdentity {
                canonical_path: PathBuf::from(format!("/opt/tool/app-{n}")),
                size: 4096,
                sha256: Sha256Digest([n; 32]),
                modified_at: None,
                platform: PlatformEvidence::Unix { mode: 0o755 },
            }
        }

        let mut candidates = CandidateTable::default();
        let evicted_id = candidates.insert(identity(0));
        // One more than the table's own bound (32) guarantees the first
        // insert above has been evicted by the time this loop finishes.
        for n in 1..=32u8 {
            candidates.insert(identity(n));
        }

        let error = find_candidate(&candidates, CandidateIdDto(evicted_id.raw())).unwrap_err();

        assert_eq!(error.code, ShellErrorCode::NoCandidate);
    }

    #[test]
    fn changed_since_approval_denial_carries_both_short_digests() {
        let recorded = Sha256Digest([1; 32]);
        let observed = Sha256Digest([2; 32]);

        let mapped = ShellError::from(DenialReason::ChangedSinceApproval { recorded, observed });

        assert_eq!(mapped.code, ShellErrorCode::ChangedSinceApproval);
        let detail = mapped
            .detail
            .expect("changed-since-approval must carry a structured detail");
        assert_eq!(detail.recorded_sha256_short, recorded.short_hex());
        assert_eq!(detail.observed_sha256_short, observed.short_hex());
    }
}
