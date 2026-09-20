//! Proves concurrent PTY spawns never leak a descriptor into a sibling
//! launch. Before the process-wide spawn lock (`src/lib.rs`'s
//! `SPAWN_LOCK`, held across `src/pty.rs`'s `open_pair`), `openpty`'s
//! non-atomic close-on-exec window let another thread's `fork` inherit
//! this launch's pty master/slave across its own `exec` -- letting one
//! agent read another's terminal and type into it. Before the fix this
//! leaked in 13 of 13 runs (62 of 15600 children; 0 of 3600 with one
//! thread). With only `open_pair`'s lock removed it fails ~1 run in 4.
//!
//! [`THREADS`] cloned handles each spawn `fake-agent
//! --report-inherited-fds` on the PTY path [`SPAWNS_PER_THREAD`] times,
//! reading back its `inherited-fds tty=<n> other=<n> list=...` line
//! (`src/bin/fake-agent.rs`). Does **not** exercise the Linux-only
//! `ExecHandle::SealedMemory` window (`spawn_approved`).
//!
//! A leak is measured against a baseline, never against zero: one launch
//! made before any thread starts shows what every child inherits from this
//! process regardless of concurrency -- a CI runner leaves descriptors open
//! and inheritable in its jobs' processes -- and only a descriptor outside
//! that ambient set counts. An ambient descriptor stays open here for the
//! whole run, so no pty opened later can take its number.
//!
//! `libtest` swallows a passing run's output without `--nocapture`, so the
//! summary and every leaking child's line go through [`emit_line`], a raw
//! `write(2)` on fd 2 (fields: see
//! [`concurrent_pty_spawns_leak_no_descriptor_into_a_sibling`]). Asserts every
//! spawn produced a parseable line (a broken harness is never "no leak"),
//! and `leaked_tty_children == 0 && leaked_other_children == 0`.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use omnifrons_app::{
    EnvPlan, ExecHandle, FramePayload, LaunchPlan, ProcessOutput, ProcessStatus, ProcessSupervisor,
    ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// Below `MAX_RUNNING_CHILDREN = 16` (`src/lib.rs`).
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
}

/// The one `inherited-fds tty=<n> other=<n> list=...` line among `frames`'
/// stdout text, parsed into a report. `None` if absent or a field fails to
/// parse -- never misread as "zero leaked". A `stderr` frame fails the run:
/// on a PTY launch it can only be the supervisor's own diagnostic.
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

/// One PTY launch, waited out to a confirmed `Exited { code: Some(0) }`;
/// `label` is unique per launch, so it doubles as the workspace name.
fn launch(supervisor: &mut TokioProcessSupervisor, label: &str) -> ChildFdReport {
    {
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-fd-inherit-test-{}-{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
        let workspace =
            WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");

        let canonical = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
            .expect("the fake-agent binary must canonicalize");
        let file =
            std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
        let plan = LaunchPlan {
            argv: vec!["--report-inherited-fds".to_string()],
            env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
            cwd: workspace,
            stdin: StdinPlan::Null,
            prompt: None,
            scope_mode: ScopeMode::Advisory,
            transport: TransportClass::Pty,
            output_dir: None,
        };

        let id = supervisor
            .spawn_approved(ExecHandle::File(file), canonical, &plan)
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
        let report = parse_report(&frames)
            .unwrap_or_else(|| panic!("{label}: no inherited-fds line in {frames:?}"));
        let _ = std::fs::remove_dir_all(&dir);
        report
    }
}

#[test]
fn concurrent_pty_spawns_leak_no_descriptor_into_a_sibling() {
    let mut supervisor = TokioProcessSupervisor::new();
    // Before any thread exists: what every child inherits from this process.
    let baseline = launch(&mut supervisor, "baseline");
    let ambient: BTreeSet<&str> = baseline.entries().collect();

    let threads: Vec<std::thread::JoinHandle<Vec<ChildFdReport>>> = (0..THREADS)
        .map(|thread_index| {
            let mut handle = supervisor.clone();
            std::thread::spawn(move || {
                (0..SPAWNS_PER_THREAD)
                    .map(|spawn| launch(&mut handle, &format!("t{thread_index}-{spawn}")))
                    .collect()
            })
        })
        .collect();

    let mut total_spawns: u32 = 0;
    let mut leaked_tty_children: u32 = 0;
    let mut leaked_other_children: u32 = 0;
    let mut max_tty_in_one_child: u32 = 0;
    // Linux only: the fixture tells a pty master from a slave there.
    let mut leaked_master_children: u32 = 0;
    let mut leaked_slave_children: u32 = 0;

    for (thread_index, thread) in threads.into_iter().enumerate() {
        let reports = thread
            .join()
            .unwrap_or_else(|_| panic!("stress thread {thread_index} must not panic"));
        for report in reports {
            total_spawns += 1;
            // Only what is outside the ambient set was leaked by a launch.
            let leaked: Vec<&str> = report
                .entries()
                .filter(|entry| !ambient.contains(entry))
                .collect();
            let ttys = leaked.iter().filter(|entry| entry.contains(":tty")).count();
            if ttys > 0 {
                leaked_tty_children += 1;
                max_tty_in_one_child = max_tty_in_one_child.max(u32::try_from(ttys).unwrap_or(0));
                leaked_master_children +=
                    u32::from(leaked.iter().any(|e| e.ends_with(":tty-master")));
                leaked_slave_children +=
                    u32::from(leaked.iter().any(|e| e.ends_with(":tty-slave")));
            }
            leaked_other_children += u32::from(leaked.iter().any(|e| e.ends_with(":other")));
            if !leaked.is_empty() {
                emit_line(&format!(
                    "concurrent-spawn-fd-inheritance leaked thread={thread_index} \
                     tty={} other={} list={} leaked={}",
                    report.tty,
                    report.other,
                    report.list,
                    leaked.join(",")
                ));
            }
        }
    }

    emit_line(&format!(
        "concurrent-spawn-fd-inheritance os={os} threads={THREADS} spawns={total_spawns} \
         ambient={ambient_count} \
         leaked_tty_children={leaked_tty_children} leaked_other_children={leaked_other_children} \
         max_tty_in_one_child={max_tty_in_one_child} \
         leaked_master_children={leaked_master_children} \
         leaked_slave_children={leaked_slave_children}",
        os = std::env::consts::OS,
        ambient_count = ambient.len()
    ));

    let expected_spawns = u32::try_from(THREADS * SPAWNS_PER_THREAD).expect("fits in u32");
    assert_eq!(
        total_spawns, expected_spawns,
        "every spawn must have produced a parseable report -- a broken harness is never \"no leak\""
    );
    assert_eq!(
        leaked_tty_children, 0,
        "{leaked_tty_children}/{total_spawns} children inherited a sibling's pty fd (max \
         {max_tty_in_one_child}, {leaked_master_children} master, {leaked_slave_children} \
         slave) -- see \"leaked thread=...\" above"
    );
    assert_eq!(
        leaked_other_children, 0,
        "{leaked_other_children}/{total_spawns} children inherited a sibling's non-tty fd -- \
         see \"leaked thread=...\" above"
    );
}
