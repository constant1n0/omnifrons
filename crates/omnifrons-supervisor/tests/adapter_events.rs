//! `--flood` (undrained output overflows the delivery channel, so a later
//! delivered frame's `droppedBefore` is nonzero) and `--huge-line` (a
//! single, over-cap "line" is assembled by `LineAssembler` then reported
//! truncated, never handed to `LineAgent::parse_line`) against the
//! `fake-agent` test binary launched through `spawn_approved`'s
//! `LaunchPlan` (`docs/spike-log.md` § Slice 3).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    AgentPrompt, Assembled, AssembledLine, EnvPlan, ExecHandle, FramePayload, LaunchPlan,
    LineAssembler, ProcessOutput, ProcessStatus, ProcessSupervisor, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::{AdapterEvent, TransportClass};
use omnifrons_domain::output::MAX_TEXT_FRAME_BYTES;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

const REAP_DEADLINE: Duration = Duration::from_secs(10);

fn wait_for_terminal(
    supervisor: &TokioProcessSupervisor,
    id: omnifrons_app::ProcessId,
    deadline: Instant,
) {
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => return,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("process did not reach a terminal state in time: {other:?}"),
        }
    }
}

fn temp_workspace(label: &str) -> (PathBuf, WorkspaceRoot) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-adapter-events-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let workspace = WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");
    (dir, workspace)
}

fn fake_agent_handle() -> (ExecHandle, PathBuf) {
    let canonical = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize");
    let file = std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
    (ExecHandle::File(file), canonical)
}

fn base_plan(workspace: WorkspaceRoot, argv: Vec<String>) -> LaunchPlan {
    LaunchPlan {
        argv,
        env: EnvPlan::new(&[]).expect("empty declared keys must be accepted"),
        cwd: workspace,
        stdin: StdinPlan::PipePromptThenClose,
        prompt: Some(AgentPrompt::new("unused").expect("valid prompt")),
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::StructuredStreamingCli,
        output_dir: None,
    }
}

/// `--flood 20000`, left completely undrained until the process is
/// terminal, must overflow the 1024-frame delivery channel: at least one
/// frame delivered once draining resumes must report a nonzero
/// `dropped_before`.
#[test]
fn flood_undrained_overflows_the_channel_with_a_nonzero_dropped_before() {
    let (dir, workspace) = temp_workspace("flood");
    let (handle, canonical) = fake_agent_handle();
    let plan = base_plan(workspace, vec!["--flood".to_string(), "20000".to_string()]);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    // Deliberately left undrained until the process is confirmed terminal
    // -- exactly the scenario the channel's own bounded, drop-oldest-never
    // (drop-newest, best-effort) backpressure exists for.
    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);

    let frames: Vec<_> = rx.iter().collect();
    let saw_dropped = frames.iter().any(|frame| frame.dropped_before > 0);
    assert!(
        saw_dropped,
        "expected at least one delivered frame with a nonzero dropped_before, got {} frames \
         with all dropped_before == 0",
        frames.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `--huge-line 200000` emits one `stream-json` line far larger than
/// `LineAssembler`'s 64 KiB cap. Feeding the captured stdout text frames
/// through a `LineAssembler` exactly as the shell would must report it
/// truncated, never a successfully assembled (and then parsed) line.
#[test]
fn huge_line_is_assembled_then_reported_truncated() {
    let (dir, workspace) = temp_workspace("huge-line");
    let (handle, canonical) = fake_agent_handle();
    let plan = base_plan(
        workspace,
        vec!["--huge-line".to_string(), "200000".to_string()],
    );

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let frames: Vec<_> = rx.iter().collect();

    let mut assembler = LineAssembler::new();
    let mut saw_truncated = false;
    for frame in &frames {
        if let FramePayload::Text {
            stream: omnifrons_app::OutputStream::Stdout,
            text,
            continued,
        } = &frame.payload
        {
            // Sanity: `omnifrons-supervisor`'s own output-capture framing
            // really did split this huge line into multiple frames, none
            // exceeding the per-frame cap -- otherwise this test would not
            // actually be exercising the assembler's continuation path at
            // all.
            assert!(text.len() <= MAX_TEXT_FRAME_BYTES);
            if let Some(Assembled {
                line: AssembledLine::Truncated { raw },
                ..
            }) = assembler.push(text, *continued, frame.dropped_before)
            {
                saw_truncated = true;
                let event = AdapterEvent::Unknown {
                    raw,
                    truncated: true,
                };
                assert!(matches!(
                    event,
                    AdapterEvent::Unknown {
                        truncated: true,
                        ..
                    }
                ));
            }
        }
    }

    assert!(
        saw_truncated,
        "expected the assembled huge line to be reported Truncated"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
