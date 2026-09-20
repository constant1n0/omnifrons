//! Stresses concurrent sealed-memfd spawns for a descriptor leaking into a
//! sibling launch. This is the second of two non-atomic close-on-exec
//! windows [`SPAWN_LOCK`] (`src/lib.rs`) closes -- the other being the pty
//! window `tests/concurrent_spawn_fd_inheritance.rs` already stresses.
//! This one is Linux-only: in `spawn_approved`'s `ExecHandle::SealedMemory`
//! arm, `finish_spawn` clears close-on-exec on the launch's own sealed
//! `memfd` right before `command.spawn()` (a fork+exec) and restores it
//! right after, all under [`SPAWN_LOCK`]. Without that lock, a concurrent
//! launch that forked+exec'd while this window was open would inherit a
//! descriptor for *another* launch's sealed executable copy. With
//! `spawn_lock` changed to hand out an unrelated, freshly leaked mutex (no
//! real exclusion), two independent sets of three runs failed every time:
//! 461 and 427 of 3600 children held a memfd that was not their own, one
//! child as many as 5. With the lock restored, 0 of 6000 did, in each set.
//! A passing run shows no leak in that run, not that the window is closed.
//!
//! [`THREADS`] cloned handles each spawn `fake-agent
//! --report-inherited-fds` [`SPAWNS_PER_THREAD`] times via
//! `ExecHandle::SealedMemory` over pipes (`TransportClass::StructuredStreamingCli`,
//! not the pty transport -- that window is the other test's job, and pipes
//! never open a pty at all, so nothing here could contribute to it),
//! reading back its `inherited-fds tty=<n> other=<n> list=...` line. A
//! sealed launch keeps *its own* memfd open in the child by design (the
//! long doc comment above `spawn_approved` in `src/lib.rs`), refined by
//! `fake-agent`'s `--report-inherited-fds` mode to the label `memfd`
//! (`src/bin/fake-agent.rs`'s `other_kind`) -- so such a child legitimately
//! reports exactly one memfd of its own.
//!
//! A leak is measured against a baseline, never against zero: one sealed
//! launch made before any thread starts (so its own memfd is part of the
//! baseline shape too) shows what every child inherits from this process
//! regardless of concurrency -- a CI runner leaves descriptors open and
//! inheritable in its jobs' processes. Two different comparisons follow
//! from that baseline: every non-memfd entry is compared by its exact
//! `<fd>:<kind>` string, exactly as the pty stress test does, because an
//! ambient descriptor keeps the same fd number for the life of this
//! process; a memfd entry is compared by *count* instead, because each
//! launch's own memfd is freshly created (a different fd number every
//! time, racing against seven other threads doing the same) -- so a child
//! is clean only when its memfd count equals the baseline's, and every
//! other entry was already in the baseline's ambient set.
//!
//! `libtest` swallows a passing run's output without `--nocapture`, so the
//! summary and every leaking or reportless child's line go through
//! [`emit_line`], a raw `write(2)` on fd 2 (fields: see
//! [`concurrent_sealed_spawns_leak_no_sibling_memfd_into_a_child`]). Every
//! spawn must reach `Exited { code: Some(0) }` (a hard assertion, like the
//! pty test) and deliver a report line; a child that delivers none is
//! counted separately (`no_report`) rather than silently skipped or
//! counted as clean, and the run still fails if any occurred.

#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use omnifrons_app::{
    EnvPlan, ExecHandle, FramePayload, LaunchPlan, ProcessOutput, ProcessStatus, ProcessSupervisor,
    ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// Below `MAX_RUNNING_CHILDREN = 16` (`src/lib.rs`), mirroring the pty
/// stress test's own shape.
const THREADS: usize = 8;
const SPAWNS_PER_THREAD: usize = 150;
/// Generous headroom; a passing run never approaches it.
const PER_CHILD_DEADLINE: Duration = Duration::from_secs(20);

struct ChildFdReport {
    tty: u32,
    other: u32,
    list: String,
}

impl ChildFdReport {
    /// This child's `<fd>:<kind>` entries.
    fn entries(&self) -> impl Iterator<Item = &str> {
        self.list.split(',').filter(|entry| !entry.is_empty())
    }

    /// How many of this child's entries are its own (or a leaked sibling's)
    /// sealed memfd.
    fn memfd_count(&self) -> usize {
        self.entries()
            .filter(|entry| entry.ends_with(":memfd"))
            .count()
    }
}

/// The one `inherited-fds tty=<n> other=<n> list=...` line among `frames`'
/// stdout text, parsed into a report. `None` if absent or a field fails to
/// parse -- never misread as "zero leaked", and never panics: a child that
/// produced no report line is the caller's `no_report` case, not a harness
/// bug. A `stderr` frame still fails the run: on this fixture mode it can
/// only be the supervisor's own diagnostic.
fn parse_report(frames: &[omnifrons_app::OutputFrame]) -> Option<ChildFdReport> {
    let mut line = None;
    for frame in frames {
        match &frame.payload {
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
                ..
            } if text.starts_with("inherited-fds ") => line = Some(text.clone()),
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stderr,
                text,
                ..
            } => panic!("unexpected stderr frame (a supervisor diagnostic?): {text:?}"),
            _ => {}
        }
    }
    let line = line?;
    let field = |prefix: &str| line.split(' ').find_map(|word| word.strip_prefix(prefix));
    Some(ChildFdReport {
        tty: field("tty=")?.parse().ok()?,
        other: field("other=")?.parse().ok()?,
        list: field("list=")?.to_string(),
    })
}

/// Raw `write(2)` on fd 2 -- see the module doc.
fn emit_line(line: &str) {
    let framed = format!("\n{line}\n");
    let _ = nix::unistd::write(io::stderr(), framed.as_bytes());
}

/// Linux only: build a sealed, `MFD_CLOEXEC`-created `memfd` containing
/// exactly `content`, mirroring what `omnifrons_adapters::fs_prober`'s
/// Linux hashing path builds for a real approved launch --
/// `omnifrons-supervisor` intentionally does not depend on
/// `omnifrons-adapters` (docs/repository-layout.md § Crate map), so this
/// crate's own tests build the handle shape they need directly, exactly
/// as `tests/approved_launch.rs`'s and `tests/pty_launch.rs`'s own
/// (independent, unshared -- there is no test-support module between
/// separate `tests/*.rs` binaries) copies of this same helper do.
fn sealed_memfd_of(content: &[u8]) -> std::fs::File {
    use nix::fcntl::{FcntlArg, SealFlag, fcntl};
    use nix::sys::memfd::{MFdFlags, memfd_create};

    let memfd = memfd_create(
        "omnifrons-concurrent-sealed-spawn-test-fixture",
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

/// One sealed-memfd launch over pipes, waited out to a confirmed
/// `Exited { code: Some(0) }`; `label` is unique per launch, so it doubles
/// as the workspace name. `content` is `fake-agent`'s own binary, read
/// once by the caller and sealed fresh into a new memfd for this one
/// launch -- exactly what a real probe does for a real approval, just
/// repeated concurrently here. `None` when the child produced no
/// `inherited-fds` line at all; the caller counts that separately.
fn launch(
    supervisor: &mut TokioProcessSupervisor,
    label: &str,
    content: &[u8],
) -> Option<ChildFdReport> {
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-sealed-fd-inherit-test-{}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let workspace = WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");

    let sealed = sealed_memfd_of(content);
    let plan = LaunchPlan {
        argv: vec!["--report-inherited-fds".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: workspace,
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::StructuredStreamingCli,
        output_dir: None,
    };

    let id = supervisor
        .spawn_approved(
            ExecHandle::SealedMemory(sealed),
            PathBuf::from(format!("/approved/sealed-fd-inherit-fixture-{label}")),
            &plan,
        )
        .unwrap_or_else(|error| panic!("{label} spawn failed: {error}"));
    let rx = supervisor
        .subscribe(id)
        .expect("subscribe must succeed immediately after a successful spawn");

    let deadline = Instant::now() + PER_CHILD_DEADLINE;
    let terminal = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("{label} no terminal state in time: {other:?}"),
        }
    };
    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(0) },
        "{label} must exit 0"
    );

    let frames: Vec<_> = rx.iter().collect();
    let report = parse_report(&frames);
    let _ = std::fs::remove_dir_all(&dir);
    report
}

#[test]
fn concurrent_sealed_spawns_leak_no_sibling_memfd_into_a_child() {
    let content = std::fs::read(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must be readable");
    let content = Arc::new(content);

    let mut supervisor = TokioProcessSupervisor::new();
    // Before any thread exists: what every sealed launch's child inherits
    // from this process, including its own memfd -- never asserted against
    // zero (see the module doc).
    let baseline = launch(&mut supervisor, "baseline", &content)
        .expect("the baseline launch must deliver an inherited-fds line");
    let ambient: BTreeSet<&str> = baseline
        .entries()
        .filter(|entry| !entry.ends_with(":memfd"))
        .collect();
    let baseline_memfd_count = baseline.memfd_count();
    // The instrument itself: a sealed launch's child must show its own
    // memfd, labelled as one and counted under `other=` like every
    // descriptor that is not a terminal.
    assert!(
        baseline_memfd_count >= 1,
        "the baseline must report its own memfd: {}",
        baseline.list
    );
    let not_a_tty = baseline.entries().filter(|e| !e.contains(":tty")).count();
    assert_eq!(
        usize::try_from(baseline.other).ok(),
        Some(not_a_tty),
        "`other=` must count memfd entries: {}",
        baseline.list
    );

    let threads: Vec<std::thread::JoinHandle<Vec<Option<ChildFdReport>>>> = (0..THREADS)
        .map(|thread_index| {
            let mut handle = supervisor.clone();
            let content = Arc::clone(&content);
            std::thread::spawn(move || {
                (0..SPAWNS_PER_THREAD)
                    .map(|spawn| launch(&mut handle, &format!("t{thread_index}-{spawn}"), &content))
                    .collect()
            })
        })
        .collect();

    let mut total_spawns: u32 = 0;
    let mut no_report: u32 = 0;
    let mut leaked_children: u32 = 0;
    let mut max_extra_memfd: i64 = 0;

    for (thread_index, thread) in threads.into_iter().enumerate() {
        let reports = thread
            .join()
            .unwrap_or_else(|_| panic!("stress thread {thread_index} must not panic"));
        for report in reports {
            total_spawns += 1;
            let Some(report) = report else {
                no_report += 1;
                emit_line(&format!(
                    "concurrent-sealed-spawn-fd-inheritance no-report thread={thread_index}"
                ));
                continue;
            };
            // Only what is outside the ambient set was leaked by a launch;
            // a memfd is judged by count, never by exact fd number (see the
            // module doc).
            let leaked_other: Vec<&str> = report
                .entries()
                .filter(|entry| !entry.ends_with(":memfd") && !ambient.contains(entry))
                .collect();
            let memfd_count = report.memfd_count();
            let extra_memfd = i64::try_from(memfd_count).unwrap_or(i64::MAX)
                - i64::try_from(baseline_memfd_count).unwrap_or(0);
            if !leaked_other.is_empty() || extra_memfd != 0 {
                leaked_children += 1;
                max_extra_memfd = max_extra_memfd.max(extra_memfd);
                emit_line(&format!(
                    "concurrent-sealed-spawn-fd-inheritance leaked thread={thread_index} \
                     tty={} other={} list={} leaked_other={} memfd_count={} baseline_memfd={}",
                    report.tty,
                    report.other,
                    report.list,
                    leaked_other.join(","),
                    memfd_count,
                    baseline_memfd_count
                ));
            }
        }
    }

    emit_line(&format!(
        "concurrent-sealed-spawn-fd-inheritance os={os} threads={THREADS} spawns={total_spawns} \
         ambient={ambient_count} baseline_memfd={baseline_memfd_count} \
         leaked_children={leaked_children} max_extra_memfd={max_extra_memfd} no_report={no_report}",
        os = std::env::consts::OS,
        ambient_count = ambient.len()
    ));

    let expected_spawns = u32::try_from(THREADS * SPAWNS_PER_THREAD).expect("fits in u32");
    assert_eq!(
        total_spawns, expected_spawns,
        "every spawn must have produced a result -- a broken harness is never \"no leak\""
    );
    assert_eq!(
        no_report, 0,
        "{no_report}/{total_spawns} children produced no inherited-fds line at all -- see \
         \"no-report\" above"
    );
    assert_eq!(
        leaked_children, 0,
        "{leaked_children}/{total_spawns} children held a descriptor beyond their own (max \
         {max_extra_memfd} extra memfd) -- see \"leaked\" above"
    );
}
