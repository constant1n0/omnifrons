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

#[cfg(windows)]
use omnifrons_app::ProcessTerminalState;
use omnifrons_app::{
    HarnessKind, HarnessRequest, ProcessId, ProcessStatus, ProcessSupervisor, SupervisorError,
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

fn wait_for_terminal(supervisor: &TokioProcessSupervisor, id: ProcessId) {
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => return,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            other => panic!("process {id:?} did not reach a terminal state in time: {other:?}"),
        }
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
                .spawn_harness(long_lived())
                .expect("spawning up to the 16-process cap must succeed"),
        );
    }

    let refused = supervisor
        .spawn_harness(long_lived())
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
            supervisor.spawn_harness(HarnessRequest::new(HarnessKind::DemoLines, 200, 1).unwrap());
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

        let refused_again = supervisor.spawn_harness(long_lived()).expect_err(
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
        .spawn_harness(short_lived())
        .expect("spawning the first (soon-to-be-evicted) process must succeed");
    wait_for_terminal(&supervisor, first_id);

    // One more than the retention cap: by the time the 257th has gone
    // Terminal, the first must have aged out.
    let mut last_id = first_id;
    for _ in 0..256 {
        last_id = supervisor
            .spawn_harness(short_lived())
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

    // The most recently terminated entry must still be retained.
    assert!(
        matches!(
            supervisor.observe(last_id),
            Some(ProcessStatus::Terminal(_))
        ),
        "the most recently terminated entry must still be retained"
    );
}
