//! NOT FOR MERGING. A measurement through the real supervisor, written to
//! run on macOS CI and be thrown away.
//!
//! On macOS the supervisor sometimes loses a pty child's final output: the
//! child exits 0 having written it, no read error is reported, and the
//! reader ends without it. Under 8-way load about 2.6% of
//! write-and-exit-at-once children lose their line; a child that waits for
//! its output to be read (`tcdrain`) never does. The OS-level probe
//! (`tests/pty_exit_drain_probe.rs`) never loses anything for such a child,
//! so whatever differs is one of: the child (a Rust binary against `sh`),
//! the reader (the supervisor's, early and concurrent, against a late plain
//! read loop), or the load.
//!
//! This bisects instead of guessing. Five cells, run one after another so
//! they do not load each other, each launching the same number of children
//! on the PTY path and counting those whose LAST expected line never
//! arrived:
//!
//! | cell | child | threads |
//! |---|---|---|
//! | `agent_8` | `fake-agent --pty-report`, exits at once | 8 |
//! | `agent_1` | the same | 1 |
//! | `sh_8` | `/bin/sh -c printf`, exits at once | 8 |
//! | `sh_1` | the same | 1 |
//! | `drained_8` | `fake-agent --report-inherited-fds`, `tcdrain`s first | 8 |
//!
//! `sh_*` losing too clears the child; `*_1` losing nothing makes it a
//! matter of load; `drained_8` is the control and must lose nothing.
//!
//! One line per cell, raw on fd 2 (`libtest` swallows a passing test's
//! print macros): `pty-tail-loss cell=<name> os=<os> launches=<n> lost=<n>
//! lost_everything=<n> lost_tail_only=<n> read_errors=<n>`. A lost child
//! either delivered nothing at all or only an earlier part; `read_errors`
//! counts the supervisor's own read-error diagnostic frames. It asserts
//! nothing about loss -- only that every child reached `Exited(0)`, so a
//! broken harness cannot pass for a clean result.

#![cfg(unix)]

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    EnvPlan, ExecHandle, FramePayload, LaunchPlan, OutputStream, ProcessOutput, ProcessStatus,
    ProcessSupervisor, ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// Launches per cell: at 2.6% a lossy cell should lose around eight.
const LAUNCHES: usize = 320;
/// Generous headroom; a passing run never approaches it.
const PER_CHILD_DEADLINE: Duration = Duration::from_secs(20);

/// What one cell launches, and the prefix of the last line it must deliver.
#[derive(Clone, Copy)]
struct Child {
    program: &'static str,
    argv: &'static [&'static str],
    last_line: &'static str,
}

const AGENT: Child = Child {
    program: env!("CARGO_BIN_EXE_fake-agent"),
    argv: &["--pty-report"],
    last_line: "env-keys",
};
const DRAINED_AGENT: Child = Child {
    program: env!("CARGO_BIN_EXE_fake-agent"),
    argv: &["--report-inherited-fds"],
    last_line: "inherited-fds ",
};
const SH: Child = Child {
    program: "/bin/sh",
    argv: &["-c", "printf 'first-line\\nlast-line\\n'"],
    last_line: "last-line",
};

/// What one launch delivered.
struct Delivered {
    stdout_lines: usize,
    has_last_line: bool,
    read_errors: usize,
}

fn emit_line(line: &str) {
    let framed = format!("\n{line}\n");
    let _ = nix::unistd::write(io::stderr(), framed.as_bytes());
}

/// One PTY launch waited out to `Exited { code: Some(0) }`.
fn launch(supervisor: &mut TokioProcessSupervisor, child: Child, label: &str) -> Delivered {
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-tail-loss-{}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let program = std::fs::canonicalize(child.program).expect("the program must canonicalize");
    let file = std::fs::File::open(&program).expect("opening the program must succeed");
    let plan = LaunchPlan {
        argv: child.argv.iter().map(|arg| (*arg).to_string()).collect(),
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    };

    let id = supervisor
        .spawn_approved(ExecHandle::File(file), PathBuf::from(&program), &plan)
        .unwrap_or_else(|error| panic!("{label} spawn failed: {error}"));
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let deadline = Instant::now() + PER_CHILD_DEADLINE;
    let terminal = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            other => panic!("{label} no terminal state in time: {other:?}"),
        }
    };
    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(0) },
        "{label} must exit 0 -- a child that could not write exits otherwise"
    );

    let mut delivered = Delivered {
        stdout_lines: 0,
        has_last_line: false,
        read_errors: 0,
    };
    for frame in &frames {
        match frame.payload {
            FramePayload::Text {
                stream: OutputStream::Stdout,
                text,
                ..
            } => {
                delivered.stdout_lines += 1;
                delivered.has_last_line |= text.starts_with(child.last_line);
            }
            FramePayload::Text {
                stream: OutputStream::Stderr,
                ..
            } => delivered.read_errors += 1,
            FramePayload::State(_) => {}
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    delivered
}

/// Runs one cell to completion and emits its line.
fn run_cell(name: &'static str, child: Child, threads: usize) {
    let supervisor = TokioProcessSupervisor::new();
    let per_thread = LAUNCHES / threads;
    let workers: Vec<std::thread::JoinHandle<Vec<Delivered>>> = (0..threads)
        .map(|thread| {
            let mut handle = supervisor.clone();
            std::thread::spawn(move || {
                (0..per_thread)
                    .map(|n| launch(&mut handle, child, &format!("{name}-t{thread}-{n}")))
                    .collect()
            })
        })
        .collect();

    let (mut launches, mut lost, mut lost_everything, mut read_errors) = (0, 0, 0, 0);
    for worker in workers {
        for delivered in worker.join().expect("a cell's thread must not panic") {
            launches += 1;
            read_errors += delivered.read_errors;
            if !delivered.has_last_line {
                lost += 1;
                lost_everything += usize::from(delivered.stdout_lines == 0);
            }
        }
    }
    emit_line(&format!(
        "pty-tail-loss cell={name} os={os} launches={launches} lost={lost} \
         lost_everything={lost_everything} lost_tail_only={tail} read_errors={read_errors}",
        os = std::env::consts::OS,
        tail = lost - lost_everything
    ));
    assert_eq!(launches, per_thread * threads, "every launch must report");
}

#[test]
fn measures_pty_tail_loss_by_child_and_by_load() {
    run_cell("agent_8", AGENT, 8);
    run_cell("agent_1", AGENT, 1);
    run_cell("sh_8", SH, 8);
    run_cell("sh_1", SH, 1);
    run_cell("drained_8", DRAINED_AGENT, 8);
}
