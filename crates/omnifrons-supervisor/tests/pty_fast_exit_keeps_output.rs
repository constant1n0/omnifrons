//! A pty child that writes and exits at once must still deliver its output.
//!
//! On macOS `finish_spawn` used to release this process's copies of the
//! slave end before any reader was attached to the master. Two measurement
//! runs on macOS CI, 384 000 launches each, lost the whole output of 744 and
//! of 1063 children, all exiting 0 with no read error; Linux lost none of
//! 153 600. The second run timed each step of `finish_spawn`: in all 1063
//! lost launches, tied to their timing by pid, that release blocked for
//! about 600 ms (1029 of them between 600 and 609 ms), and no launch whose
//! release did not block lost anything -- six spawns took over 300 ms in
//! another step (`command.spawn()`, or waiting for the spawn lock) and none
//! of those lost its last line. With nothing reading the master, whatever
//! the child had written was gone once that release returned.
//!
//! Only children that exit the instant they have written lost output: this
//! file's own, `fake-agent --pty-report`, and one other fixture mode.
//! `/bin/sh -c printf` lost 0 of 256 000, and a fixture that `tcdrain`s
//! before exiting 0 of 128 000. Why `sh` never loses is not known.
//!
//! The loss is a race, so this test is statistical and only macOS can fail
//! it: 301 of the 400 measured rounds of 960 such launches lost at least
//! one. [`THREADS`] x [`LAUNCHES_PER_THREAD`] launches make a run on the
//! unfixed code very likely, not certain, to fail; a passing run shows no
//! loss in that run. One line, raw on fd 2 (`libtest` swallows a passing
//! test's output): `pty-fast-exit-keeps-output os=<os> launches=<n>
//! lost=<n> read_errors=<n> slowest_spawn_ms=<n>`.

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

const THREADS: usize = 8;
const LAUNCHES_PER_THREAD: usize = 400;
/// The last line `fake-agent --pty-report` prints begins with this.
const LAST_LINE: &str = "env-keys";
/// Generous headroom; a passing run never approaches it.
const PER_CHILD_DEADLINE: Duration = Duration::from_secs(30);

/// What one launch delivered.
struct Delivered {
    has_last_line: bool,
    read_errors: usize,
    spawn: Duration,
}

/// One PTY launch of `fake-agent --pty-report`, waited out to
/// `Exited { code: Some(0) }` -- a child that could not write exits
/// otherwise, so a lost line is never mistaken for a child that failed.
fn launch(supervisor: &mut TokioProcessSupervisor, label: &str) -> Delivered {
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-fast-exit-{}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let program = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fixture must canonicalize");
    let file = std::fs::File::open(&program).expect("opening the fixture must succeed");
    let plan = LaunchPlan {
        argv: vec!["--pty-report".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    };

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
    assert_eq!(
        state,
        ProcessTerminalState::Exited { code: Some(0) },
        "{label} must exit 0"
    );

    let (mut has_last_line, mut read_errors) = (false, 0);
    for frame in &frames {
        match frame.payload {
            FramePayload::Text {
                stream: OutputStream::Stdout,
                text,
                ..
            } => has_last_line |= text.starts_with(LAST_LINE),
            // A terminal has no stderr: on this path a stderr frame is the
            // supervisor's own read-error diagnostic.
            FramePayload::Text {
                stream: OutputStream::Stderr,
                ..
            } => read_errors += 1,
            FramePayload::State(_) => {}
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Delivered {
        has_last_line,
        read_errors,
        spawn,
    }
}

#[test]
fn every_fast_exiting_pty_child_delivers_its_last_line() {
    let supervisor = TokioProcessSupervisor::new();
    let workers: Vec<std::thread::JoinHandle<Vec<Delivered>>> = (0..THREADS)
        .map(|thread| {
            let mut handle = supervisor.clone();
            std::thread::spawn(move || {
                (0..LAUNCHES_PER_THREAD)
                    .map(|n| launch(&mut handle, &format!("t{thread}-{n}")))
                    .collect()
            })
        })
        .collect();

    let (mut launches, mut lost, mut read_errors) = (0, 0, 0);
    let mut slowest_spawn = Duration::ZERO;
    for worker in workers {
        for delivered in worker.join().expect("a launching thread must not panic") {
            launches += 1;
            lost += usize::from(!delivered.has_last_line);
            read_errors += delivered.read_errors;
            slowest_spawn = slowest_spawn.max(delivered.spawn);
        }
    }

    let line = format!(
        "\npty-fast-exit-keeps-output os={} launches={launches} lost={lost} \
         read_errors={read_errors} slowest_spawn_ms={}\n",
        std::env::consts::OS,
        slowest_spawn.as_millis()
    );
    let _ = nix::unistd::write(io::stderr(), line.as_bytes());

    assert_eq!(
        launches,
        THREADS * LAUNCHES_PER_THREAD,
        "every launch must report"
    );
    assert_eq!(
        lost,
        0,
        "{lost} of {launches} children exited 0 but their last line never arrived \
         (read errors reported: {read_errors}; slowest spawn: {} ms)",
        slowest_spawn.as_millis()
    );
}
