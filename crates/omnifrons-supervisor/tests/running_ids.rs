//! `TokioProcessSupervisor::running_ids` (spike slice 5b): the ids of every
//! child currently `Running`, each refreshed through the same reap check
//! `observe` performs -- the one source the shell's publication surface
//! consults to freeze `artifact_approve`/`artifact_publish` while a run is
//! live, exactly as `harness_stop` and `harness_observe` consult it.

use std::path::PathBuf;
use std::time::Duration;

use omnifrons_app::{HarnessKind, HarnessRequest, ProcessStatus, ProcessSupervisor as _};
use omnifrons_supervisor::TokioProcessSupervisor;

fn demo_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"))
}

/// A live demo-harness child is listed while it runs and gone from the
/// list once its terminal state is confirmed through `stop`. Windows
/// `stop` is the Job Object placeholder: it reports `OrphanRiskUncertain`
/// and never evicts the entry, so the id stays listed there -- the
/// documented limitation `bookkeeping_caps.rs` asserts, and the reason the
/// shell's publication surface stays frozen after a stop on Windows.
#[test]
fn running_ids_lists_a_live_child_and_forgets_it_once_terminal() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());
    assert!(supervisor.running_ids().is_empty(), "nothing spawned yet");

    // One line per second for ten minutes: live until stopped.
    let request = HarnessRequest::new(HarnessKind::DemoLines, 1, 600).expect("a valid request");
    let id = supervisor
        .spawn_harness(&request)
        .expect("spawn the demo harness");
    assert_eq!(supervisor.running_ids(), vec![id]);

    #[cfg(unix)]
    {
        supervisor
            .stop(id, Duration::from_secs(10))
            .expect("the child stops within the deadline");
        assert!(
            supervisor.running_ids().is_empty(),
            "a terminal child is no longer running"
        );
        assert!(matches!(
            supervisor.observe(id),
            Some(ProcessStatus::Terminal(_))
        ));
    }
    #[cfg(windows)]
    {
        use omnifrons_app::ProcessTerminalState;
        let stopped = supervisor
            .stop(id, Duration::from_secs(10))
            .expect("stop still succeeds as a best-effort direct-child kill");
        assert_eq!(stopped, ProcessTerminalState::OrphanRiskUncertain);
        assert_eq!(
            supervisor.running_ids(),
            vec![id],
            "an unconfirmed stop keeps the id listed on Windows"
        );
        assert_eq!(supervisor.observe(id), Some(ProcessStatus::Running));
    }
}
