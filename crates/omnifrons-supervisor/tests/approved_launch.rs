//! `spawn_approved` launches a real, already-vetted executable with no
//! arguments -- never through the demo harness's argv-encoding path
//! (`docs/spike-log.md` § Slice 2). Given `ExecHandle::File` (macOS,
//! Windows, or a Linux probe that fell back), it spawns by the given
//! display path directly. Given `ExecHandle::SealedMemory` (Linux only),
//! it execs the sealed `memfd` itself via `/proc/self/fd/<n>`, with no
//! arguments and `stdin` always `/dev/null` -- never the executable's own
//! bytes -- so the content that runs is exactly what was sealed,
//! regardless of what the display path resolves to by the time this call
//! runs.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    ExecHandle, FramePayload, ProcessOutput, ProcessStatus, ProcessSupervisor, ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous headroom for a short-lived fixture process to run to
/// completion and be confirmed reaped, matching this crate's other
/// output-capture tests' own reasoning about loaded/virtualized CI
/// runners.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

/// Poll `observe` until `id` reaches a terminal state, or panic once
/// `deadline` passes. Output capture's final `State` frame is sent only
/// once a reap is *confirmed* this way (`docs/spike-log.md` § IPC
/// contract), never automatically just because the child exited.
fn wait_for_terminal(
    supervisor: &TokioProcessSupervisor,
    id: omnifrons_app::ProcessId,
    deadline: Instant,
) {
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => return,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("process did not reach a terminal state in time: {other:?}"),
        }
    }
}

/// Collect every `Text` frame's line, in order, discarding the final
/// `State` frame.
fn text_lines(frames: &[omnifrons_app::OutputFrame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text { text, .. } => Some(text.clone()),
            FramePayload::State(_) => None,
        })
        .collect()
}

/// A drop-guard temp directory: removed on drop regardless of which path
/// out of a test (pass, fail, or panic) is taken, so a fixture directory
/// this crate's own tests create under the system temp dir never
/// accumulates across runs.
#[cfg(target_os = "linux")]
struct TempDir(PathBuf);

#[cfg(target_os = "linux")]
impl TempDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-approved-launch-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(target_os = "linux")]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn spawn_approved_runs_the_target_with_no_args_and_streams_its_output() {
    let mut supervisor = TokioProcessSupervisor::new();
    let canonical_path = std::fs::canonicalize(env!("CARGO_BIN_EXE_demo-harness"))
        .expect("the demo-harness binary must canonicalize");
    let file = std::fs::File::open(&canonical_path)
        .expect("opening the demo-harness binary for the File handle must succeed");

    let id = supervisor
        .spawn_approved(ExecHandle::File(file), canonical_path)
        .expect("spawn_approved must succeed for a valid executable");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let frames: Vec<_> = rx.iter().collect();

    // `demo-harness` run with no arguments at all (never the harness's own
    // `<kind> <rate_hz> <lines>` argv shape) prints its usage line to
    // stderr and exits `1` -- proof `spawn_approved` really did launch it
    // with no arguments, and that its output reached this subscriber.
    let saw_usage_line = frames.iter().any(|frame| {
        matches!(
            &frame.payload,
            FramePayload::Text { text, .. } if text.contains("usage: demo-harness")
        )
    });
    assert!(
        saw_usage_line,
        "the target's usage output must reach the subscriber, got {frames:#?}"
    );

    let terminal = frames
        .iter()
        .find_map(|frame| match frame.payload {
            FramePayload::State(state) => Some(state),
            FramePayload::Text { .. } => None,
        })
        .expect("a final state frame must be delivered");
    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(1) },
        "demo-harness with no arguments exits 1 after printing its usage line"
    );
}

/// Linux only: build a sealed, `MFD_CLOEXEC`-created `memfd` containing
/// exactly `content`, mirroring (independently of) what
/// `omnifrons_adapters::fs_prober`'s Linux hashing path itself builds --
/// `omnifrons-supervisor` intentionally does not depend on
/// `omnifrons-adapters` (docs/repository-layout.md § Crate map), so this
/// crate's own tests build the handle shape they need directly, exactly
/// as the shell (`src-tauri`) receives one from a real probe in the real
/// system.
#[cfg(target_os = "linux")]
fn sealed_memfd_with(content: &[u8]) -> std::fs::File {
    use std::io::Write as _;

    use nix::fcntl::{FcntlArg, SealFlag, fcntl};
    use nix::sys::memfd::{MFdFlags, memfd_create};

    let memfd = memfd_create(
        "omnifrons-approved-launch-test-fixture",
        MFdFlags::MFD_CLOEXEC | MFdFlags::MFD_ALLOW_SEALING,
    )
    .expect("memfd_create must succeed for the test fixture");
    let mut file = std::fs::File::from(memfd);
    file.write_all(content)
        .expect("writing the fixture content into the memfd must succeed");
    fcntl(
        &file,
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .expect("sealing the fixture memfd must succeed");
    file
}

/// Linux only: `spawn_approved`'s `ExecHandle::SealedMemory` path execs
/// the sealed `memfd` itself, whose content has nothing to do with
/// whatever the display path resolves to at launch time. This test
/// replaces the display path's own file entirely (unlink and recreate --
/// a genuinely new inode) *and* overwrites it in place (same inode,
/// different bytes) after the sealed handle was built, and confirms the
/// process that actually runs still prints the originally sealed content,
/// never either replacement.
#[cfg(target_os = "linux")]
#[test]
fn sealed_memfd_content_survives_the_display_path_being_replaced_and_overwritten() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("toctou");
    let script_path = dir.path().join("script.sh");
    std::fs::write(&script_path, "#!/bin/sh\necho REPLACED-BEFORE-SEAL\n")
        .expect("write the pre-seal placeholder script");
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod the placeholder script executable");

    let sealed = sealed_memfd_with(b"#!/bin/sh\necho ORIGINAL\n");

    // Replace the display path's directory entry outright (new inode).
    std::fs::remove_file(&script_path).expect("remove the placeholder script");
    std::fs::write(&script_path, "#!/bin/sh\necho REPLACED\n")
        .expect("write the replacement script");
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod the replacement script executable");
    // Also overwrite the *same* inode in place, for good measure.
    std::fs::write(&script_path, "#!/bin/sh\necho OVERWRITTEN\n")
        .expect("overwrite the replacement script in place");

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(ExecHandle::SealedMemory(sealed), script_path.clone())
        .expect("spawn_approved must succeed against a sealed memfd handle");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let lines = text_lines(&rx.iter().collect::<Vec<_>>());

    assert!(
        lines.iter().any(|line| line == "ORIGINAL"),
        "expected the sealed content to execute, got {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line == "REPLACED" || line == "OVERWRITTEN"),
        "neither replacement at the display path must ever execute, got {lines:?}"
    );
}

/// Linux only: `spawn_approved`'s `ExecHandle::SealedMemory` path must
/// never feed the executable's own bytes to the child as `stdin` -- only
/// `/dev/null`. A fixture that reads a line from its own `stdin` and
/// echoes it back gets nothing (immediate EOF), never the sealed script's
/// own source text.
#[cfg(target_os = "linux")]
#[test]
fn spawn_approved_never_feeds_the_executable_as_stdin() {
    let sealed = sealed_memfd_with(b"#!/bin/sh\nread line\necho \"stdin:$line\"\n");

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(
            ExecHandle::SealedMemory(sealed),
            PathBuf::from("/approved/stdin-honesty-fixture"),
        )
        .expect("spawn_approved must succeed against a sealed memfd handle");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let lines = text_lines(&rx.iter().collect::<Vec<_>>());

    assert!(
        lines.iter().any(|line| line == "stdin:"),
        "reading stdin must observe immediate EOF (an empty line), never the executable's own \
         bytes, got {lines:?}"
    );
}
