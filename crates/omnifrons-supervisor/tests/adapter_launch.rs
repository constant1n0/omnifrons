//! End-to-end adapter launch, against the `fake-agent` test binary,
//! through `TokioProcessSupervisor::spawn_approved`'s new `LaunchPlan`
//! parameter (`docs/spike-log.md` § Slice 3): the observed environment
//! equals exactly the allowlist (a planted secret-shaped env var never
//! reaches the child, even though it is present in the launching
//! process's own environment), the observed working directory equals the
//! workspace, the prompt arrives on stdin and EOF is observed (the
//! fake-agent's own `read_to_string` only ever returns once EOF is seen,
//! so completing at all proves this), and the exit code surfaces --
//! including with `--no-eof`, proving the supervisor's own stdin-writer
//! task never hangs even when the child never reads its stdin at all.
//!
//! The planted key is never set in this test process's own environment
//! (`std::env::set_var` is `unsafe` and races every concurrent
//! environment reader in the same process -- R1-005/R3-001): the
//! env-allowlist test re-execs this very test binary, filtered to exactly
//! itself, with the key planted through `Command::env` on that child, and
//! performs the real assertions in that inner run.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    AgentPrompt, EnvPlan, ExecHandle, FramePayload, LaunchPlan, ProcessOutput, ProcessStatus,
    ProcessSupervisor, ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

const REAP_DEADLINE: Duration = Duration::from_secs(10);

fn wait_for_terminal(
    supervisor: &TokioProcessSupervisor,
    id: omnifrons_app::ProcessId,
    deadline: Instant,
) -> ProcessTerminalState {
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => return state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("process did not reach a terminal state in time: {other:?}"),
        }
    }
}

fn collect_stdout_texts(frames: &[omnifrons_app::OutputFrame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Extract the unescaped text of `"<key>":"..."` from a hand-built
/// `stream-json` line -- this crate has no `serde_json` dependency
/// (`docs/spike-log.md` § Slice 3), so this is a small, deliberately
/// narrow reversal of `fake-agent`'s own minimal escaper, not a
/// general-purpose JSON parser.
fn extract_json_text_field(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = line.find(&needle)? + needle.len();
    let bytes = line.as_bytes();
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(out),
            b'\\' if i + 1 < bytes.len() => {
                match bytes[i + 1] {
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    other => out.push(other as char),
                }
                i += 2;
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    None
}

fn temp_workspace(label: &str) -> (PathBuf, WorkspaceRoot) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-adapter-launch-test-{}-{label}-{n}",
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

/// Set on the re-exec'd inner run of
/// [`observed_env_equals_the_allowlist_excluding_a_planted_secret_shaped_key`]
/// so it performs the real assertions instead of re-exec'ing again.
const INNER_RUN_MARKER: &str = "OMNIFRONS_TEST_INNER";

/// The secret-shaped key planted in the inner run's environment. Its
/// value is a dummy literal, never a credential; the name is chosen to be
/// secret-shaped (`*_TOKEN`) so the test also covers the supervisor's own
/// re-check, on top of the key simply not being allowlisted.
const PLANTED_SECRET_SHAPED_KEY: &str = "FAKE_SECRET_TOKEN";

/// The exact test name the outer run filters the inner run to.
const ENV_ALLOWLIST_TEST_NAME: &str =
    "observed_env_equals_the_allowlist_excluding_a_planted_secret_shaped_key";

/// Outer half of the env-allowlist test: re-exec this test binary,
/// filtered to exactly that one test, with the planted key and the inner
/// marker set on the child through `Command::env` -- this process's own
/// environment is never mutated. Asserts the inner run succeeded and that
/// it really executed exactly one test (so a mistyped filter can never
/// pass vacuously with zero tests run).
fn rerun_env_allowlist_test_with_a_planted_key() {
    let test_binary = std::env::current_exe().expect("the test binary's own path must resolve");
    let output = std::process::Command::new(test_binary)
        .args(["--exact", ENV_ALLOWLIST_TEST_NAME, "--nocapture"])
        .env(INNER_RUN_MARKER, "1")
        .env(PLANTED_SECRET_SHAPED_KEY, "planted")
        .output()
        .expect("re-exec'ing the test binary must start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the inner run failed with status {:?}\n--- inner stdout ---\n{stdout}\n--- inner stderr \
         ---\n{stderr}",
        output.status
    );
    assert!(
        stdout.contains("1 passed"),
        "the inner run must have executed exactly one test, got:\n{stdout}"
    );
}

#[test]
fn observed_env_equals_the_allowlist_excluding_a_planted_secret_shaped_key() {
    if std::env::var_os(INNER_RUN_MARKER).is_none() {
        rerun_env_allowlist_test_with_a_planted_key();
        return;
    }
    assert!(
        std::env::var_os(PLANTED_SECRET_SHAPED_KEY).is_some(),
        "the inner run must see the planted key in its own environment, or the test proves \
         nothing"
    );

    let (dir, workspace) = temp_workspace("env-allowlist");
    let (handle, canonical) = fake_agent_handle();

    let plan = LaunchPlan {
        argv: vec![],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: workspace.clone(),
        stdin: StdinPlan::PipePromptThenClose,
        prompt: Some(AgentPrompt::new("hello\nworld").expect("valid prompt")),
        scope_mode: ScopeMode::Advisory,
    };

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let frames: Vec<_> = rx.iter().collect();
    let texts = collect_stdout_texts(&frames);

    // The prompt arrived on stdin and EOF was seen: `fake-agent`'s
    // `read_to_string` only returns (letting it reach this point at all)
    // once EOF is observed, and both prompt lines were echoed back.
    let echoed_hello = texts.iter().any(|line| line.contains(r#""text":"hello""#));
    let echoed_world = texts.iter().any(|line| line.contains(r#""text":"world""#));
    assert!(
        echoed_hello,
        "expected an echoed 'hello' line, got {texts:#?}"
    );
    assert!(
        echoed_world,
        "expected an echoed 'world' line, got {texts:#?}"
    );

    // The observed cwd equals the workspace.
    let init_line = texts
        .iter()
        .find(|line| line.contains(r#""subtype":"init""#))
        .expect("an init line must be present");
    let observed_cwd =
        extract_json_text_field(init_line, "cwd").expect("cwd field must be present");
    assert_eq!(
        PathBuf::from(observed_cwd),
        workspace.path().to_path_buf(),
        "the child's observed cwd must equal the workspace"
    );

    // The observed env is exactly the base allowlist, never the planted
    // secret-shaped key. Only key names are ever printed on failure --
    // never the observed values.
    let env_dump_line = texts
        .iter()
        .rev()
        .find(|line| line.contains("PATH="))
        .expect("an environment-dump line must be present");
    let env_dump = extract_json_text_field(env_dump_line, "text")
        .expect("the env-dump line must carry a text field");
    let observed_keys: Vec<&str> = env_dump
        .lines()
        .filter_map(|line| line.split_once('=').map(|(key, _value)| key))
        .collect();

    assert!(
        !observed_keys.contains(&PLANTED_SECRET_SHAPED_KEY),
        "a planted secret-shaped env var must never reach the child, observed keys: \
         {observed_keys:?}"
    );

    let expected_allowlist: std::collections::HashSet<&str> = if cfg!(windows) {
        [
            "PATH",
            "LANG",
            "LC_ALL",
            "TMPDIR",
            "USERPROFILE",
            "SystemRoot",
            "SystemDrive",
            "TEMP",
            "TMP",
        ]
        .into_iter()
        .collect()
    } else {
        ["PATH", "LANG", "LC_ALL", "TMPDIR", "HOME"]
            .into_iter()
            .collect()
    };
    for key in &observed_keys {
        assert!(
            expected_allowlist.contains(key),
            "observed env var {key} is not in the expected allowlist {expected_allowlist:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_eof_with_a_custom_exit_code_terminates_without_hanging() {
    let (dir, workspace) = temp_workspace("no-eof-exit-code");
    let (handle, canonical) = fake_agent_handle();

    let plan = LaunchPlan {
        argv: vec![
            "--no-eof".to_string(),
            "--exit-code".to_string(),
            "3".to_string(),
        ],
        env: EnvPlan::new(&[]).expect("empty declared keys must be accepted"),
        cwd: workspace,
        stdin: StdinPlan::PipePromptThenClose,
        prompt: Some(AgentPrompt::new("ignored by --no-eof").expect("valid prompt")),
        scope_mode: ScopeMode::Advisory,
    };

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed");
    let _rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let terminal = wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(3) },
        "--no-eof plus --exit-code 3 must terminate Exited(3) without hanging"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
