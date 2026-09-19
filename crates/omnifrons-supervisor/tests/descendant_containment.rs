//! `stop` must prove what it claims about a stopped child's descendants.
//!
//! VP-001 evidence row `VP-001-VP-S6-02` (CI run 35450847942, pinned
//! `ubuntu-24.04` baseline) recorded the defect these tests pin: an agent
//! spawned a descendant that called `setsid`, `Stop` was triggered, and the
//! product reported `exited (code unreported)` while that descendant was
//! still alive with its original `starttime`. `docs/target-architecture.md`
//! § Required failure states requires the opposite -- "Process descendants
//! unproven stopped -> Orphan-risk/uncertain" -- so a clean terminal state
//! is only honest once every descendant is *proven* gone.
//!
//! These tests are about the claim: whatever `stop` reports must be true
//! of the descendant it reported it about. A descendant the census
//! recorded must be swept and proven gone before a definite terminal state
//! is honest; one the census could never have seen must be reported as
//! orphan-risk/uncertain, and that one the test terminates itself, because
//! a test owns what the product did not promise to contain.
//!
//! Linux only: the census these tests exercise walks `/proc` by parent
//! chain, which exists on Linux and not on the other unixes this workspace
//! builds for. On those, `stop` keeps its pre-existing behaviour and
//! containment stays `unproven` (`src-tauri/src/health.rs`) -- the same
//! documented gap as before, not a new one.
//!
//! Every fixture involved is self-bounding (`fake-agent`'s
//! `DESCENDANT_LIFETIME_SECS`): if a test fails, whatever it spawned still
//! exits on its own shortly after, so a failing run never leaves a process
//! behind on a developer's machine or a CI runner.

#![cfg(target_os = "linux")]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, OutputFrame, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor,
    ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// How long to wait for a fixture's synchronization frame before giving
/// up. Generous, like the other real-process tests in this crate: a loaded
/// CI runner needs scheduling margin that has nothing to do with the
/// behaviour under test.
const FRAME_DEADLINE: Duration = Duration::from_secs(10);

/// The deadline handed to `stop` itself.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// How long a descendant is given to disappear before it counts as having
/// outlived the stop that was supposed to sweep it. A process killed by
/// the sweep is reparented to `init` and reaped within milliseconds; this
/// bound exists only so the assertion never races that hand-off.
const SURVIVAL_OBSERVATION: Duration = Duration::from_secs(3);

fn fake_agent(args: &[&str]) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_fake-agent")).with_args(args.to_vec())
}

/// Block until `rx` delivers a text frame starting with `prefix`, and
/// return the remainder of that line. Panics once `deadline` passes: a
/// fixture that never reaches its synchronization point is a test failure,
/// not something to wait out.
fn wait_for_line(rx: &Receiver<OutputFrame>, prefix: &str, deadline: Instant) -> String {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "did not observe a {prefix:?} frame within the deadline"
        );
        match rx.recv_timeout(remaining) {
            Ok(OutputFrame {
                payload: FramePayload::Text { text, .. },
                ..
            }) if text.starts_with(prefix) => return text[prefix.len()..].trim().to_string(),
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the output channel closed before a {prefix:?} frame was observed")
            }
        }
    }
}

/// Block until the channel delivers the process's final state frame.
fn final_state_frame(rx: &Receiver<OutputFrame>, deadline: Instant) -> ProcessTerminalState {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "did not observe the final state frame within the deadline"
        );
        match rx.recv_timeout(remaining) {
            Ok(OutputFrame {
                payload: FramePayload::State(state),
                ..
            }) => return state,
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the output channel closed before the final state frame")
            }
        }
    }
}

/// Spawn `fake-agent` in a descendant-spawning mode and return the
/// supervisor, its subscription, the stopped child's id, and the
/// descendant's pid -- read from the fixture's own `descendant-pid` line,
/// which it prints only once the descendant has reported itself ready.
fn spawn_with_descendant(
    mode: &str,
) -> (
    TokioProcessSupervisor,
    Receiver<OutputFrame>,
    omnifrons_app::ProcessId,
    u32,
) {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn(fake_agent(&[mode]))
        .expect("spawning the descendant fixture must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");
    let reported = wait_for_line(&rx, "descendant-pid ", Instant::now() + FRAME_DEADLINE);
    let descendant = reported
        .parse::<u32>()
        .expect("the fixture must report a numeric descendant pid");
    (supervisor, rx, id, descendant)
}

fn pid_of(raw: u32) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(i32::try_from(raw).expect("a pid fits in i32"))
}

fn still_running(pid: u32) -> bool {
    nix::sys::signal::kill(pid_of(pid), None) != Err(nix::errno::Errno::ESRCH)
}

/// Whether `pid` is still present after polling for it to disappear for at
/// most `budget`. `kill(pid, 0)` still succeeds for a zombie, so this
/// polls until the entry is really gone rather than sampling once.
fn outlived(pid: u32, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if !still_running(pid) {
            return false;
        }
        if Instant::now() >= deadline {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Assert that a recorded descendant was swept, and that whatever `stop`
/// reported is true of it: a definite terminal state is only honest once
/// the descendant is gone, and one still running must be reported as
/// `orphan-risk/uncertain`.
fn assert_swept_and_reported_honestly(terminal: ProcessTerminalState, descendant: u32) {
    let survived = outlived(descendant, SURVIVAL_OBSERVATION);
    assert!(
        !(survived
            && matches!(
                terminal,
                ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed
            )),
        "stop reported {terminal:?} while descendant {descendant} was still alive: that is a \
         clean stop it cannot prove (docs/target-architecture.md § Required failure states)"
    );
    assert!(
        !survived,
        "descendant {descendant} outlived the stop that was supposed to sweep it"
    );
    assert!(
        matches!(
            terminal,
            ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed
        ),
        "with the descendant swept and the group empty, stop must report a definite terminal \
         state, got {terminal:?}"
    );
}

/// Assert that a descendant this stop did not terminate was *reported* as
/// exactly that, and clean it up.
///
/// The cleanup runs before the assertions on purpose: a failing assertion
/// must not be the reason a process is left behind.
fn assert_reported_as_unproven(terminal: ProcessTerminalState, descendant: u32) {
    let alive = still_running(descendant);
    let _ = nix::sys::signal::kill(pid_of(descendant), nix::sys::signal::Signal::SIGKILL);

    assert!(
        alive,
        "descendant {descendant} was already gone, so this test would prove nothing about \
         what stop reports for a descendant that is still running"
    );
    assert_eq!(
        terminal,
        ProcessTerminalState::OrphanRiskUncertain,
        "a recorded descendant still running is not a clean stop \
         (docs/target-architecture.md § Required failure states)"
    );
}

/// The `VP-001-VP-S6-02` shape: a descendant that calls `setsid` leaves the
/// process group `killpg` targets, so signalling the group never reaches
/// it. It is still in the census taken before the signal, which is the
/// whole point of taking one -- and being in the census is what lets the
/// sweep reach it by pid.
#[test]
fn a_setsid_descendant_is_swept_and_the_reported_state_is_honest() {
    let (mut supervisor, _rx, id, descendant) =
        spawn_with_descendant("--spawn-escaping-descendant");

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must return a result");

    assert_swept_and_reported_honestly(terminal, descendant);
}

/// A descendant that stays inside the process group but ignores `SIGTERM`
/// survives the group signal too: the direct child is reaped while it keeps
/// running. The sweep must escalate to `SIGKILL` for it specifically,
/// by pid.
#[test]
fn a_sigterm_ignoring_descendant_in_the_group_is_swept_and_proven_gone() {
    let (mut supervisor, _rx, id, descendant) =
        spawn_with_descendant("--spawn-stubborn-descendant");

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must return a result");

    assert_swept_and_reported_honestly(terminal, descendant);
}

/// A descendant that does not exist yet when the census is taken cannot be
/// in it: that is the residual race this design accepts and does not hide.
/// What must not follow is a clean stop reported over it -- the process
/// group check catches the ones that stayed in the group -- it reports
/// them, it does not sweep them, because they are not in the census the
/// sweep works from. The reported
/// state, a later `observe`, and the final state frame must then all agree
/// on `orphan-risk/uncertain`. One of the three disagreeing would be a
/// caller told two different stories about the same stop.
#[test]
fn a_descendant_born_after_the_census_is_reported_as_orphan_risk_uncertain() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn(fake_agent(&["--spawn-late-descendant"]))
        .expect("spawning the late-descendant fixture must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");
    // Printed once SIGTERM is blocked, so the fixture cannot miss the
    // group signal it waits for.
    wait_for_line(&rx, "ready", Instant::now() + FRAME_DEADLINE);

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must return a result");

    let descendant = wait_for_line(&rx, "descendant-pid ", Instant::now() + FRAME_DEADLINE)
        .parse::<u32>()
        .expect("the fixture must report a numeric descendant pid");
    assert_reported_as_unproven(terminal, descendant);
    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Terminal(
            ProcessTerminalState::OrphanRiskUncertain
        )),
        "observe must report the same state stop did, not the reap that preceded the proof"
    );
    assert_eq!(
        final_state_frame(&rx, Instant::now() + FRAME_DEADLINE),
        ProcessTerminalState::OrphanRiskUncertain,
        "the final state frame must carry the same state stop reported"
    );
}

/// The ordinary case must not become uncertain: a well-behaved child with
/// no descendants at all still reports its own exit code.
///
/// The child has already exited (as an unreaped zombie) by the time `stop`
/// runs, which is exactly the case where a naive "the child must be in
/// `/proc`" census would wrongly conclude it could prove nothing: a zombie
/// still has its `/proc` entry, and `stop` still reports the real code.
#[test]
fn a_child_with_no_descendants_still_reports_its_own_exit_code() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn(fake_agent(&["--no-eof", "--exit-code", "7"]))
        .expect("spawning the fixture must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");
    // The fixture's last line before it exits.
    wait_for_line(
        &rx,
        r#"{"type":"result","subtype":"success"}"#,
        Instant::now() + FRAME_DEADLINE,
    );
    wait_until_zombie(id.0, Instant::now() + FRAME_DEADLINE);

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must return a result");

    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(7) },
        "a child that exited on its own with no descendants must still report its own exit code"
    );
}

/// Block until `pid` is an unreaped zombie (`/proc/<pid>/stat` state `Z`),
/// so the test knows the child has really exited before it calls `stop` --
/// rather than sleeping a hopeful interval and racing `stop`'s own
/// `SIGTERM`, which would replace the exit code with a signal death.
fn wait_until_zombie(pid: u32, deadline: Instant) {
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .expect("the child's /proc entry must exist until it is reaped");
        let after_comm = &stat[stat.rfind(')').expect("stat carries a comm field") + 1..];
        if after_comm.split_whitespace().next() == Some("Z") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the child never became a zombie within the deadline"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
