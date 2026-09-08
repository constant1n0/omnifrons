//! The declared output directory reaches the child (spike slice 5,
//! HAP-001-R9, D4): a `LaunchPlan` that declares a run subdirectory
//! delivers it through exactly one key, `OMNIFRONS_OUTPUT_DIR`, whose
//! value is the product's own -- a value of the same name planted in the
//! launching process's environment never wins -- and no other key of the
//! observed environment changes; a plan that declares none delivers no
//! such key. The `fake-agent` fixture's outbox modes (`--write-outbox`,
//! `--plant-link`, `--report-output-dir`) are proven here too, since the
//! shell's own tests cannot spawn the fixture.
//!
//! The planted key is never set in this test process's own environment
//! (`std::env::set_var` is `unsafe` and races every concurrent
//! environment reader -- the slice-3 R1-005/R3-001 rule): the planted-value
//! test re-execs this very test binary, filtered to exactly itself, with
//! the key planted through `Command::env` on that child, and performs the
//! real assertions in that inner run. Only key names are ever printed on
//! failure, never observed values.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use omnifrons_app::{
    AgentPrompt, EnvPlan, ExecHandle, FramePayload, LaunchPlan, OUTPUT_DIR_ENV_KEY, ProcessOutput,
    ProcessStatus, ProcessSupervisor, ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;
use sha2::{Digest as _, Sha256};

const REAP_DEADLINE: Duration = Duration::from_secs(10);

/// A drop-guard temp directory, removed on every exit path.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-outbox-launch-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

fn fake_agent_handle() -> (ExecHandle, PathBuf) {
    let canonical = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize");
    let file = std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
    (ExecHandle::File(file), canonical)
}

/// A structured-transport plan for `argv`, with no prompt (the fixture's
/// outbox modes never read stdin) and no output dir declared yet.
fn plan(workspace: &WorkspaceRoot, argv: &[&str]) -> LaunchPlan {
    LaunchPlan {
        argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
        env: EnvPlan::new(&[]).expect("empty declared keys must be accepted"),
        cwd: workspace.clone(),
        stdin: StdinPlan::Null,
        prompt: None::<AgentPrompt>,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::StructuredStreamingCli,
        output_dir: None,
    }
}

/// Launch `plan` against the fixture and return its stdout lines once it
/// is confirmed terminal, asserting a clean exit.
fn run_fixture(plan: &LaunchPlan) -> Vec<String> {
    let (handle, canonical) = fake_agent_handle();
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, plan)
        .expect("spawn_approved must succeed");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");
    let terminal = wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    assert_eq!(
        terminal,
        ProcessTerminalState::Exited { code: Some(0) },
        "the fixture must exit cleanly"
    );
    let frames: Vec<_> = rx.iter().collect();
    collect_stdout_texts(&frames)
}

/// The key names the fixture's `env-keys` line reports.
fn observed_env_keys(lines: &[String]) -> Vec<String> {
    let line = lines
        .iter()
        .find(|line| line.starts_with("env-keys"))
        .expect("an env-keys line must be present");
    line.split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn base_allowlist() -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
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
    } else {
        vec!["PATH", "LANG", "LC_ALL", "TMPDIR", "HOME"]
    }
}

/// Set on the re-exec'd inner run so it performs the real assertions
/// instead of re-exec'ing again.
const INNER_RUN_MARKER: &str = "OMNIFRONS_TEST_INNER_OUTBOX";

/// The exact test name the outer run filters the inner run to.
const PLANTED_VALUE_TEST_NAME: &str =
    "declared_output_dir_reaches_the_child_and_a_planted_parent_value_never_wins";

/// A dummy value planted under the output-dir key in the inner run's
/// environment: not a directory, not a path the product would ever
/// declare -- if it reached the child, the child's write would fail.
const PLANTED_VALUE: &str = "planted-value-not-a-directory";

fn rerun_planted_value_test() {
    let test_binary = std::env::current_exe().expect("the test binary's own path must resolve");
    let output = std::process::Command::new(test_binary)
        .args(["--exact", PLANTED_VALUE_TEST_NAME, "--nocapture"])
        .env(INNER_RUN_MARKER, "1")
        .env(OUTPUT_DIR_ENV_KEY, PLANTED_VALUE)
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

/// HAP-001-R9/D4: the child sees the declared run subdirectory under
/// `OMNIFRONS_OUTPUT_DIR` -- proven by the marker file it writes there --
/// even though the launching process's own environment carries a planted
/// value under the same key; and the observed key set is exactly the base
/// allowlist plus that one key.
#[test]
fn declared_output_dir_reaches_the_child_and_a_planted_parent_value_never_wins() {
    if std::env::var_os(INNER_RUN_MARKER).is_none() {
        rerun_planted_value_test();
        return;
    }
    assert_eq!(
        std::env::var(OUTPUT_DIR_ENV_KEY).as_deref(),
        Ok(PLANTED_VALUE),
        "the inner run must see the planted value in its own environment, or the test proves \
         nothing"
    );

    let workspace_dir = TempDir::new("planted-workspace");
    let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("valid workspace");
    let run_dir = TempDir::new("planted-run-dir");
    let declared = std::fs::canonicalize(run_dir.path()).expect("canonical run dir");

    let mut plan = plan(&workspace, &["--report-output-dir", "--write-outbox", "1"]);
    plan.declare_output_dir(declared.clone())
        .expect("declaring the output dir on an allowlist plan must succeed");

    let lines = run_fixture(&plan);

    assert!(
        lines
            .iter()
            .any(|line| line == "output-dir set=true writable=true"),
        "the child must report the key set and writable, got {lines:#?}"
    );
    assert!(
        declared.join("artifact-0.pdf").is_file(),
        "the child must have written into the declared directory, not the planted value"
    );

    let mut expected: Vec<String> = base_allowlist().into_iter().map(str::to_string).collect();
    expected.push(OUTPUT_DIR_ENV_KEY.to_string());
    expected.sort();
    let mut observed = observed_env_keys(&lines);
    observed.sort();
    // The base allowlist's keys are resolved from the parent, so only the
    // ones the parent actually has appear; every observed key must be in
    // the expected set, and the output key must be among them.
    for key in &observed {
        assert!(
            expected.contains(key),
            "observed env var {key} is not in the expected set {expected:?}"
        );
    }
    assert!(
        observed.iter().any(|key| key == OUTPUT_DIR_ENV_KEY),
        "the declared key must reach the child, observed keys: {observed:?}"
    );
}

/// A plan that declares no output directory delivers no
/// `OMNIFRONS_OUTPUT_DIR` at all.
#[test]
fn a_plan_without_an_output_dir_declares_no_key() {
    let workspace_dir = TempDir::new("no-output-dir");
    let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("valid workspace");
    let plan = plan(&workspace, &["--report-output-dir"]);

    let lines = run_fixture(&plan);

    assert!(
        lines
            .iter()
            .any(|line| line == "output-dir set=false writable=false"),
        "got {lines:#?}"
    );
    assert!(
        !observed_env_keys(&lines)
            .iter()
            .any(|key| key == OUTPUT_DIR_ENV_KEY)
    );
}

/// `--write-outbox <n>` writes `n` heavy files cycling document, image,
/// archive (by magic bytes) plus one Markdown note and one
/// executable-shaped file, and emits one `artifact.publish` tool-call
/// proposal naming every even-indexed heavy file by its real SHA-256 --
/// never the note or the executable.
#[test]
fn write_outbox_writes_the_declared_kinds_and_proposes_a_subset_by_real_digest() {
    let workspace_dir = TempDir::new("write-outbox-workspace");
    let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("valid workspace");
    let run_dir = TempDir::new("write-outbox-run-dir");
    let declared = std::fs::canonicalize(run_dir.path()).expect("canonical run dir");

    let mut plan = plan(&workspace, &["--write-outbox", "3"]);
    plan.declare_output_dir(declared.clone()).expect("declare");

    let lines = run_fixture(&plan);

    let expect_magic = |name: &str, magic: &[u8]| {
        let bytes = std::fs::read(declared.join(name))
            .unwrap_or_else(|error| panic!("{name} must have been written: {error}"));
        assert!(
            bytes.starts_with(magic),
            "{name} must start with its magic bytes, got {:?}",
            &bytes[..bytes.len().min(8)]
        );
        bytes
    };
    let pdf = expect_magic("artifact-0.pdf", b"%PDF-");
    let png = expect_magic("artifact-1.png", &[0x89, b'P', b'N', b'G']);
    let zip = expect_magic("artifact-2.zip", b"PK\x03\x04");
    let note = std::fs::read_to_string(declared.join("notes.md")).expect("notes.md");
    assert!(note.starts_with("# "), "the note is Markdown text");
    expect_magic("tool.bin", &[0x7F, b'E', b'L', b'F']);
    assert_eq!(
        std::fs::read_dir(&declared).expect("list").count(),
        5,
        "exactly the five files are written"
    );

    let proposal_line = lines
        .iter()
        .find(|line| line.contains(r#""name":"artifact.publish""#))
        .expect("an artifact.publish tool_use line must be emitted");
    let hex = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    assert!(
        proposal_line.contains(&format!(
            r#"{{"name":"artifact-0.pdf","sha256":"{}"}}"#,
            hex(&pdf)
        )),
        "the even-indexed pdf is proposed by its real digest, got {proposal_line}"
    );
    assert!(
        proposal_line.contains(&format!(
            r#"{{"name":"artifact-2.zip","sha256":"{}"}}"#,
            hex(&zip)
        )),
        "the even-indexed zip is proposed by its real digest, got {proposal_line}"
    );
    assert!(
        !proposal_line.contains(&hex(&png)),
        "the odd-indexed png is not proposed"
    );
    assert!(!proposal_line.contains("notes.md"));
    assert!(!proposal_line.contains("tool.bin"));
}

/// `--propose-missing` adds a proposal entry whose digest matches no file
/// in the directory -- the fixture for the shell's unmatched-proposal
/// diagnostic.
#[test]
fn propose_missing_names_a_digest_no_written_file_carries() {
    let workspace_dir = TempDir::new("propose-missing-workspace");
    let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("valid workspace");
    let run_dir = TempDir::new("propose-missing-run-dir");
    let declared = std::fs::canonicalize(run_dir.path()).expect("canonical run dir");

    let mut plan = plan(&workspace, &["--write-outbox", "1", "--propose-missing"]);
    plan.declare_output_dir(declared.clone()).expect("declare");

    let lines = run_fixture(&plan);
    let proposal_line = lines
        .iter()
        .find(|line| line.contains(r#""name":"artifact.publish""#))
        .expect("an artifact.publish tool_use line must be emitted");
    assert!(
        proposal_line.contains(r#""name":"ghost.pdf""#),
        "got {proposal_line}"
    );
    assert!(!declared.join("ghost.pdf").exists());
}

/// Unix only: `--plant-link` writes `outside.bin` beside the run
/// subdirectory (at the outbox root), then plants `escape-link`, a
/// symbolic link to it, and `linked.bin`, a hard link to it, inside the
/// run subdirectory. Windows: creating a symbolic link needs a privilege
/// the CI runner does not grant, so the fixture prints `symlink-skipped`
/// there and only the hard link is planted; that half is not asserted
/// here because the link-count check itself is a disclosed residual on
/// Windows (no stable `std` accessor reads it from a handle).
#[cfg(unix)]
#[test]
fn plant_link_creates_a_symlink_and_a_hard_link_inside_the_run_subdirectory() {
    use std::os::unix::fs::MetadataExt as _;

    let workspace_dir = TempDir::new("plant-link-workspace");
    let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("valid workspace");
    let outbox = TempDir::new("plant-link-outbox");
    let run_dir = outbox.path().join("run-1");
    std::fs::create_dir(&run_dir).expect("fixture run dir");
    let declared = std::fs::canonicalize(&run_dir).expect("canonical run dir");

    let mut plan = plan(&workspace, &["--plant-link"]);
    plan.declare_output_dir(declared.clone()).expect("declare");

    let lines = run_fixture(&plan);
    assert!(lines.iter().any(|line| line == "planted"), "got {lines:#?}");

    let outside = outbox.path().join("outside.bin");
    assert!(
        outside.is_file(),
        "outside.bin is written at the outbox root"
    );
    let link = std::fs::symlink_metadata(declared.join("escape-link")).expect("escape-link");
    assert!(link.file_type().is_symlink());
    let linked = std::fs::metadata(declared.join("linked.bin")).expect("linked.bin");
    assert_eq!(
        linked.nlink(),
        2,
        "linked.bin is a hard link to outside.bin"
    );
    assert_eq!(
        linked.ino(),
        std::fs::metadata(&outside).expect("outside").ino()
    );
}
