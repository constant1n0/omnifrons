//! The three demo-harness IPC commands: `harness_spawn`, `harness_stop`,
//! `harness_observe`. Registered next to `shell_health`
//! (`crate::shell_health`).
//!
//! No program path or argument vector for a harness request ever crosses
//! IPC (`docs/spike-log.md` § IPC contract): the renderer names a
//! [`dto::HarnessKindDto`] plus small bounded numbers, validated into an
//! `omnifrons_app::HarnessRequest` before the supervisor ever sees it.
//! Every error surfaces as a [`dto::ShellError`] with a fixed catalogue
//! message -- never the underlying `SupervisorError::Spawn`'s `io::Error`
//! text, which can carry a real filesystem path.

use std::time::Duration;

use omnifrons_app::{
    HarnessRequest, InvalidRequest, ProcessOutput, ProcessSupervisor, SupervisorError,
};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::ipc::dto::{
    HarnessFrame, HarnessKindDto, ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto,
    ShellError, ShellErrorCode,
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
        Self {
            code,
            message: message.to_string(),
        }
    }
}

impl From<InvalidRequest> for ShellError {
    fn from(_: InvalidRequest) -> Self {
        Self {
            code: ShellErrorCode::InvalidRequest,
            message: "the requested rate_hz or lines value is out of the accepted range"
                .to_string(),
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
        Err(ShellError {
            code: ShellErrorCode::InvalidRequest,
            message: "deadlineMs must be between 1 and 30000".to_string(),
        })
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

/// Spawn a demo harness and stream its captured output over `on_frame`.
///
/// Validates `kind`/`rate_hz`/`lines` into an `HarnessRequest` first, then
/// spawns and subscribes on a blocking thread, then starts a detached
/// forwarder thread that relays every captured frame onto `on_frame` until
/// the channel closes (the frontend navigated away or dropped it) or the
/// process's output channel itself closes (the process is confirmed
/// terminal and fully drained).
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::InvalidRequest`] if
/// `rate_hz`/`lines` are out of range, or the mapped
/// [`SupervisorError`] if the process could not be started.
#[tauri::command]
pub async fn harness_spawn(
    app: AppHandle,
    kind: HarnessKindDto,
    rate_hz: u16,
    lines: u32,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let request = HarnessRequest::new(kind.into(), rate_hz, lines)?;

    let (id, receiver) = with_supervisor(app, move |supervisor| {
        let id = supervisor.spawn_harness(request)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

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

    Ok(id_dto)
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
    use super::ShellError;
    use crate::ipc::dto::ShellErrorCode;
    use omnifrons_app::SupervisorError;

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
}
