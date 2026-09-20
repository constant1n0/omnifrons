//! NOT FOR MERGING. A hunt through the real supervisor, written to run many
//! times on macOS CI (`.github/workflows/tail-loss-hunt.yml`, which exists
//! on this branch only) and be thrown away.
//!
//! On macOS the supervisor sometimes loses a pty child's final output: the
//! child exits 0 having written it, no read error is reported, and the
//! reader ends without it. It has been seen as the intermittent failure of
//! `pty_launch`'s environment test, and ONCE in a stress run, where five
//! launches in flight at the same moment each stalled about two seconds and
//! came back empty. It is rare and it comes in bursts: a first version of
//! this file ran 1280 launches on macOS and lost none. So this one is built
//! for volume, and to say everything about a burst when one is caught.
//!
//! Six cells, run one after another, [`LAUNCHES`] each on the PTY path:
//!
//! | cell | child | threads |
//! |---|---|---|
//! | `report_8` | `fake-agent --report-inherited-fds --no-drain` | 8 |
//! | `agent_8` | `fake-agent --pty-report` | 8 |
//! | `agent_1` | the same | 1 |
//! | `sh_8` | `/bin/sh -c printf` | 8 |
//! | `sh_1` | the same | 1 |
//! | `drained_8` | `fake-agent --report-inherited-fds` (`tcdrain`s) | 8 |
//!
//! `report_8` is exactly the child of the run that lost output. Every other
//! child also exits the instant it has written, except `drained_8`.
//!
//! Output, raw on fd 2 (`grep pty-tail-loss`). One line per cell:
//! `pty-tail-loss cell=<name> os=<os> launches=<n> lost=<n>
//! lost_everything=<n> lost_tail_only=<n> read_errors=<n> slow=<n>
//! max_total_ms=<n>`. And one line for every launch that LOST its last line
//! or was SLOW (over [`SLOW`]): `pty-tail-loss launch cell=<name>
//! label=<t-n> pid=<n> verdict=<lost|slow> at_ms=<since the cell began>
//! spawn_ms=<in spawn_approved> terminal_ms=<spawn to terminal state>
//! collect_ms=<terminal state to end of frames> stdout_lines=<n>
//! read_errors=<n>`. `at_ms` is what shows whether losses are simultaneous;
//! a slow launch that lost nothing shows a stall is not enough by itself.
//!
//! The first run of this hunt lost 744 of 384 000 launches on macOS, every
//! one of them after 603 to 718 ms inside `spawn_approved`. So the supervisor
//! itself (this branch only) now prints, for any spawn over 300 ms, the time
//! each step of `finish_spawn` took: `pty-tail-loss spawn-stall pid=<n>
//! lock_us= fork_us= to_release_us= release_us= to_attach_us= attach_us=
//! rest_us= total_us=`. `pid` ties it to the launch line above.
//!
//! It asserts nothing about loss -- only that every child reached
//! `Exited(0)`, so a broken harness cannot pass for a clean result.

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

/// Launches per cell.
const LAUNCHES: usize = 320;
/// A launch slower than this end to end is reported even if it lost nothing.
const SLOW: Duration = Duration::from_millis(500);
/// Generous headroom; a passing run never approaches it.
const PER_CHILD_DEADLINE: Duration = Duration::from_secs(30);

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
const REPORT_NO_DRAIN: Child = Child {
    program: env!("CARGO_BIN_EXE_fake-agent"),
    argv: &["--report-inherited-fds", "--no-drain"],
    last_line: "inherited-fds ",
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

/// What one launch delivered, and how long each part of it took.
struct Delivered {
    label: String,
    pid: u32,
    at: Duration,
    spawn: Duration,
    terminal: Duration,
    collect: Duration,
    stdout_lines: usize,
    has_last_line: bool,
    read_errors: usize,
}

fn emit_line(line: &str) {
    let framed = format!("\n{line}\n");
    let _ = nix::unistd::write(io::stderr(), framed.as_bytes());
}

/// One PTY launch waited out to `Exited { code: Some(0) }`.
fn launch(
    supervisor: &mut TokioProcessSupervisor,
    child: Child,
    label: String,
    cell_began: Instant,
) -> Delivered {
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

    let at = cell_began.elapsed();
    let began = Instant::now();
    let id = supervisor
        .spawn_approved(ExecHandle::File(file), PathBuf::from(&program), &plan)
        .unwrap_or_else(|error| panic!("{label} spawn failed: {error}"));
    let spawn = began.elapsed();
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let deadline = Instant::now() + PER_CHILD_DEADLINE;
    let state = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            other => panic!("{label} no terminal state in time: {other:?}"),
        }
    };
    let terminal = began.elapsed();
    assert_eq!(
        state,
        ProcessTerminalState::Exited { code: Some(0) },
        "{label} must exit 0 -- a child that could not write exits otherwise"
    );

    let (mut stdout_lines, mut has_last_line, mut read_errors) = (0, false, 0);
    for frame in &frames {
        match frame.payload {
            FramePayload::Text {
                stream: OutputStream::Stdout,
                text,
                ..
            } => {
                stdout_lines += 1;
                has_last_line |= text.starts_with(child.last_line);
            }
            FramePayload::Text {
                stream: OutputStream::Stderr,
                ..
            } => read_errors += 1,
            FramePayload::State(_) => {}
        }
    }
    let collect = began.elapsed().saturating_sub(terminal);
    let _ = std::fs::remove_dir_all(&dir);
    Delivered {
        label,
        pid: id.0,
        at,
        spawn,
        terminal,
        collect,
        stdout_lines,
        has_last_line,
        read_errors,
    }
}

/// Runs one cell to completion, emitting a line for every launch that lost
/// its last line or was slow, then the cell's own line.
fn run_cell(name: &'static str, child: Child, threads: usize) {
    let supervisor = TokioProcessSupervisor::new();
    let per_thread = LAUNCHES / threads;
    let cell_began = Instant::now();
    let workers: Vec<std::thread::JoinHandle<Vec<Delivered>>> = (0..threads)
        .map(|thread| {
            let mut handle = supervisor.clone();
            std::thread::spawn(move || {
                (0..per_thread)
                    .map(|n| launch(&mut handle, child, format!("t{thread}-{n}"), cell_began))
                    .collect()
            })
        })
        .collect();

    let (mut launches, mut lost, mut lost_everything, mut read_errors, mut slow) = (0, 0, 0, 0, 0);
    let mut max_total = Duration::ZERO;
    for worker in workers {
        for delivered in worker.join().expect("a cell's thread must not panic") {
            launches += 1;
            read_errors += delivered.read_errors;
            let total = delivered.terminal + delivered.collect;
            max_total = max_total.max(total);
            let is_lost = !delivered.has_last_line;
            lost += usize::from(is_lost);
            lost_everything += usize::from(is_lost && delivered.stdout_lines == 0);
            slow += usize::from(total > SLOW);
            if is_lost || total > SLOW {
                emit_line(&format!(
                    "pty-tail-loss launch cell={name} label={} pid={} verdict={} at_ms={} spawn_ms={} \
                     terminal_ms={} collect_ms={} stdout_lines={} read_errors={}",
                    delivered.label,
                    delivered.pid,
                    if is_lost { "lost" } else { "slow" },
                    delivered.at.as_millis(),
                    delivered.spawn.as_millis(),
                    delivered.terminal.as_millis(),
                    delivered.collect.as_millis(),
                    delivered.stdout_lines,
                    delivered.read_errors
                ));
            }
        }
    }
    emit_line(&format!(
        "pty-tail-loss cell={name} os={os} launches={launches} lost={lost} \
         lost_everything={lost_everything} lost_tail_only={tail} read_errors={read_errors} \
         slow={slow} max_total_ms={max}",
        os = std::env::consts::OS,
        tail = lost - lost_everything,
        max = max_total.as_millis()
    ));
    assert_eq!(launches, per_thread * threads, "every launch must report");
}

#[test]
fn hunts_pty_tail_loss_by_child_and_by_load() {
    run_cell("report_8", REPORT_NO_DRAIN, 8);
    run_cell("agent_8", AGENT, 8);
    run_cell("agent_1", AGENT, 1);
    run_cell("sh_8", SH, 8);
    run_cell("sh_1", SH, 1);
    run_cell("drained_8", DRAINED_AGENT, 8);
}
