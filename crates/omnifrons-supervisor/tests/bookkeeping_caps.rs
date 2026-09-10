//! A supervisor's own bookkeeping must stay bounded regardless of how many
//! processes a caller spawns over its lifetime: at most 16 `Running`
//! children tracked at once (`spawn` refuses a 17th with
//! `SupervisorError::TooManyProcesses`, without ever spawning it), and at
//! most 256 `Terminal` entries retained, oldest evicted first by
//! termination order (an evicted id then reports `UnknownProcess`/`None`
//! from `stop`/`observe` -- which never signals a real pid again, since an
//! evicted entry was already confirmed reaped before eviction).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    HarnessKind, HarnessRequest, ProcessId, ProcessStatus, ProcessSupervisor, ProcessTerminalState,
    SupervisorError,
};
use omnifrons_supervisor::TokioProcessSupervisor;

// 10s, not a shorter bound: several call sites here wait for a demo harness
// driven by `rate_hz` sleeps to reach a terminal state, and a loaded CI
// runner's coarser timer granularity can stretch those sleeps well past
// what this machine sees, so the deadline needs generous headroom.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

fn demo_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"))
}

/// The *confirmed* terminal state of an observed status, if it is one.
///
/// `Terminal(OrphanRiskUncertain)` from `observe` is deliberately not
/// confirmation: `TokioProcessSupervisor::observe` returns it when
/// `try_wait` itself failed (`crates/omnifrons-supervisor/src/lib.rs`,
/// the `Err(_)` arm), and that arm leaves the entry `Tracked::Running`
/// and records no terminal order -- so the very next `observe` answering
/// `Ok(None)` reports `Running` again. A caller that treats that report
/// as an arrival is asserting something the supervisor never proved
/// (`docs/spike-log.md` § Slice 5d).
///
/// Exhaustive on purpose, with no catch-all: a new `ProcessTerminalState`
/// would otherwise be classified "not proof" silently, and whether a new
/// terminal state is proof of an arrival is exactly the judgment this
/// helper exists to make (spike slice 5d, R3-018). A variant added later
/// breaks this build instead.
fn confirmed_terminal(status: Option<ProcessStatus>) -> Option<ProcessTerminalState> {
    match status {
        Some(ProcessStatus::Terminal(state)) => match state {
            ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed => Some(state),
            ProcessTerminalState::OrphanRiskUncertain => None,
        },
        Some(ProcessStatus::Running) | None => None,
    }
}

/// Wait until `id` reaches a state the supervisor actually proved. An
/// unproven `OrphanRiskUncertain` keeps the loop going rather than
/// standing in for an arrival that never happened.
fn wait_for_terminal(supervisor: &TokioProcessSupervisor, id: ProcessId) {
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        let observed = supervisor.observe(id);
        if confirmed_terminal(observed).is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "process {id:?} did not reach a confirmed terminal state in time; last observed \
             {observed:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_17th_concurrently_running_process_is_refused() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());

    // Long-lived enough (5 Hz, 200 lines ~= 40s) to stay Running for the
    // whole test; stopped explicitly at the end regardless of outcome so no
    // real child leaks past this test.
    let long_lived = || HarnessRequest::new(HarnessKind::DemoLines, 5, 200).unwrap();

    let mut spawned = Vec::new();
    for _ in 0..16 {
        spawned.push(
            supervisor
                .spawn_harness(&long_lived())
                .expect("spawning up to the 16-process cap must succeed"),
        );
    }

    let refused = supervisor
        .spawn_harness(&long_lived())
        .expect_err("a 17th concurrently running process must be refused");
    assert_eq!(
        refused,
        SupervisorError::TooManyProcesses,
        "the 17th spawn must report TooManyProcesses, not spawn a real process"
    );

    #[cfg(unix)]
    {
        for id in spawned {
            let _ = supervisor.stop(id, Duration::from_secs(3));
        }

        // The cap freed up now that every tracked process is Terminal, not
        // merely Running-but-about-to-die: a fresh spawn must succeed again.
        let after_cleanup =
            supervisor.spawn_harness(&HarnessRequest::new(HarnessKind::DemoLines, 200, 1).unwrap());
        assert!(
            after_cleanup.is_ok(),
            "spawning must succeed again once every previous process is confirmed Terminal, got {after_cleanup:?}"
        );
        if let Ok(id) = after_cleanup {
            wait_for_terminal(&supervisor, id);
        }
    }

    // On Windows, `stop` is the honest Job Object placeholder (VP-001
    // VP-S5, `crates/omnifrons-supervisor/src/lib.rs` `windows::stop`): it
    // cannot prove a descendant-free reap, so it reports
    // `OrphanRiskUncertain` and deliberately never evicts the entry to
    // `Terminal`. The running count (entries still `Tracked::Running`)
    // therefore never drops, and the 16-running cap, once reached, stays
    // reached permanently -- a real, documented limitation, not a bug in
    // this test -- until Job Object containment lands
    // (`docs/spike-log.md` § Windows deferral).
    #[cfg(windows)]
    {
        for &id in &spawned {
            let stopped = supervisor
                .stop(id, Duration::from_secs(3))
                .expect("stop must still succeed (best-effort direct-child kill) even though containment is unproven");
            assert_eq!(
                stopped,
                ProcessTerminalState::OrphanRiskUncertain,
                "Windows stop must honestly report OrphanRiskUncertain, never a confirmed reap"
            );
        }

        for &id in &spawned {
            assert_eq!(
                supervisor.observe(id),
                Some(ProcessStatus::Running),
                "an entry stop could not confirm Terminal must stay Running on Windows, \
                 matching stop's own honest OrphanRiskUncertain report"
            );
        }

        let refused_again = supervisor.spawn_harness(&long_lived()).expect_err(
            "the 16-running cap must stay permanently reached on Windows: an unproven \
                 stop never evicts an entry, so an 18th spawn must still be refused",
        );
        assert_eq!(refused_again, SupervisorError::TooManyProcesses);
    }
}

#[test]
fn the_oldest_terminal_entry_is_evicted_past_the_retention_cap() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());

    // Fast, short-lived, one at a time -- well under the 16-Running cap at
    // any instant -- so this exercises only the 256-entry Terminal
    // retention cap, not the concurrent-Running cap above.
    let short_lived = || HarnessRequest::new(HarnessKind::DemoLines, 1000, 1).unwrap();

    let first_id = supervisor
        .spawn_harness(&short_lived())
        .expect("spawning the first (soon-to-be-evicted) process must succeed");
    wait_for_terminal(&supervisor, first_id);

    // One more than the retention cap: by the time the 257th has gone
    // Terminal, the first must have aged out.
    let mut last_id = first_id;
    for _ in 0..256 {
        last_id = supervisor
            .spawn_harness(&short_lived())
            .expect("spawning a short-lived process must succeed");
        wait_for_terminal(&supervisor, last_id);
    }

    assert_eq!(
        supervisor.observe(first_id),
        None,
        "the oldest Terminal entry must have been evicted past the 256-entry retention cap"
    );
    let stop_after_eviction = supervisor
        .stop(first_id, Duration::from_secs(1))
        .expect_err("stop on an evicted id must report UnknownProcess, never re-signal a pid");
    assert_eq!(stop_after_eviction, SupervisorError::UnknownProcess);

    // The most recently terminated entry must still be retained. The
    // assertion prints what was actually observed: the one time this test
    // failed on windows-latest it printed a fixed message and diagnosed
    // nothing (`docs/spike-log.md` § Slice 5d).
    // `confirmed_terminal` and not `Terminal(_)`: the loop that just ran for
    // this id refuses an unproven `OrphanRiskUncertain`, and an assertion
    // weaker than the wait that preceded it can only ever pass by accident
    // (spike slice 5d, R3-017).
    let observed = supervisor.observe(last_id);
    assert!(
        confirmed_terminal(observed).is_some(),
        "the most recently terminated entry must still be retained in a state the supervisor \
         proved; observed {observed:?} for {last_id:?}"
    );
}

/// A pin on what [`confirmed_terminal`] treats as proof, so the
/// distinction this file depends on cannot drift silently: a reaped exit
/// and a kill are confirmations; the `OrphanRiskUncertain` report
/// `observe` returns when `try_wait` itself failed is not, because the
/// entry it describes is still `Tracked::Running` and no terminal order
/// was recorded for it.
#[test]
fn an_unproven_orphan_risk_report_is_not_a_confirmed_terminal_state() {
    assert_eq!(
        confirmed_terminal(Some(ProcessStatus::Terminal(
            ProcessTerminalState::Exited { code: Some(0) }
        ))),
        Some(ProcessTerminalState::Exited { code: Some(0) })
    );
    assert_eq!(
        confirmed_terminal(Some(ProcessStatus::Terminal(ProcessTerminalState::Killed))),
        Some(ProcessTerminalState::Killed)
    );
    assert_eq!(
        confirmed_terminal(Some(ProcessStatus::Terminal(
            ProcessTerminalState::OrphanRiskUncertain
        ))),
        None,
        "an unproven report is not an arrival"
    );
    assert_eq!(confirmed_terminal(Some(ProcessStatus::Running)), None);
    assert_eq!(confirmed_terminal(None), None);
}
