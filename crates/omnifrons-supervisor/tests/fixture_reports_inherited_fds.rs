//! The `fake-agent --report-inherited-fds` fixture mode, checked for what
//! it promises and for nothing about the machine it runs on: one line,
//! fields that agree with each other, kinds from a closed vocabulary --
//! over a pseudo-terminal and over pipes.
//!
//! It deliberately does NOT assert "nothing inherited". What a child
//! inherits beyond 0, 1 and 2 depends on what the test process itself was
//! handed: a CI runner leaves descriptors open and inheritable in its jobs'
//! processes (every one of 1200 children of one GitHub Actions job reported
//! the same two), so an absolute zero is a statement about the environment,
//! not about the supervisor. A test that cares about leaks compares against
//! a baseline instead.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    EnvPlan, ExecHandle, FramePayload, LaunchPlan, OutputFrame, ProcessId, ProcessOutput,
    ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous headroom; a passing run never approaches it.
const DEADLINE: Duration = Duration::from_secs(20);
/// Every kind the fixture may report for a descriptor.
const KINDS: [&str; 5] = ["tty", "tty-master", "tty-slave", "memfd", "other"];

fn fake_agent() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize")
}

/// Waits for `id` to exit 0, then returns every stdout line it produced.
fn stdout_lines_after_a_clean_exit(
    supervisor: &mut TokioProcessSupervisor,
    id: ProcessId,
) -> Vec<String> {
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let deadline = Instant::now() + DEADLINE;
    let terminal = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("the fixture did not reach a terminal state in time: {other:?}"),
        }
    };
    assert_eq!(terminal, ProcessTerminalState::Exited { code: Some(0) });
    frames
        .iter()
        .filter_map(|frame: OutputFrame| match frame.payload {
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
                ..
            } => Some(text),
            _ => None,
        })
        .collect()
}

/// Exactly one `inherited-fds` line, its two counts agreeing with its own
/// list, every descriptor above 2 and below the scan bound, every kind known.
fn assert_self_consistent(lines: &[String]) {
    let reports: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("inherited-fds "))
        .collect();
    assert_eq!(reports.len(), 1, "exactly one report line, got {lines:?}");
    let line = reports[0];
    let field = |prefix: &str| {
        line.split(' ')
            .find_map(|word| word.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("the report must carry `{prefix}`: {line:?}"))
    };
    let count = |prefix: &str| -> usize {
        field(prefix)
            .parse()
            .unwrap_or_else(|_| panic!("`{prefix}` must be a number: {line:?}"))
    };

    let entries: Vec<(u32, &str)> = field("list=")
        .split(',')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (fd, kind) = entry
                .split_once(':')
                .unwrap_or_else(|| panic!("an entry must be `<fd>:<kind>`: {entry:?}"));
            let fd = fd.parse().unwrap_or_else(|_| panic!("bad fd in {entry:?}"));
            (fd, kind)
        })
        .collect();
    for (fd, kind) in &entries {
        assert!((3..256).contains(fd), "fd out of the scanned range: {fd}");
        assert!(KINDS.contains(kind), "unknown kind {kind:?} in {line:?}");
    }
    let ttys = entries.iter().filter(|(_, kind)| kind.starts_with("tty"));
    assert_eq!(count("tty="), ttys.count(), "`tty=` must match: {line:?}");
    // `other=` counts every descriptor that is not a terminal, `memfd` too.
    let others = entries.iter().filter(|(_, kind)| !kind.starts_with("tty"));
    assert_eq!(
        count("other="),
        others.count(),
        "`other=` must match: {line:?}"
    );
}

/// Over a pseudo-terminal: the line is read before the child exits, which
/// is what the mode's `tcdrain` is for.
#[test]
fn a_pty_child_reports_one_self_consistent_line() {
    let dir = std::env::temp_dir().join(format!("omnifrons-fd-report-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let plan = LaunchPlan {
        argv: vec!["--report-inherited-fds".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    };
    let file = std::fs::File::open(fake_agent()).expect("opening the fixture must succeed");

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(ExecHandle::File(file), fake_agent(), &plan)
        .expect("spawn_approved must succeed on the PTY path");
    assert_self_consistent(&stdout_lines_after_a_clean_exit(&mut supervisor, id));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Over pipes: standard output is not a terminal, so the mode's `tcdrain`
/// fails with `ENOTTY`, which it must shrug off.
#[test]
fn a_piped_child_reports_one_self_consistent_line() {
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new(fake_agent().to_string_lossy().into_owned())
        .with_args(["--report-inherited-fds"]);
    let id = supervisor.spawn(spec).expect("spawn must succeed");
    assert_self_consistent(&stdout_lines_after_a_clean_exit(&mut supervisor, id));
}
