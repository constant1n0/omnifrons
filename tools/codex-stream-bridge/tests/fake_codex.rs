//! End-to-end checks against a fake `codex` shell script (unix only).

#![cfg(unix)]

use serde_json::{Value, json};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A private scratch directory, removed on drop even when a test fails.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("csb-{test}-{}-{n}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("create the scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    lines: Vec<Value>,
    stderr: String,
    argv: String,
    stdin_seen: String,
}

/// Run the bridge with `script` as the fake Codex (`None`: a path that does
/// not exist) and `prompt` on stdin. Every infrastructure failure panics.
fn run(test: &str, script: Option<&str>, prompt: &str) -> Run {
    let scratch = Scratch::new(test);
    let dir = &scratch.0;
    let fake = dir.join("fake-codex");
    if let Some(body) = script {
        std::fs::write(&fake, body).expect("write the fake codex");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("make the fake codex executable");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_codex-stream-bridge"))
        .env("CODEX_STREAM_BRIDGE_CODEX", &fake)
        .env("FAKE_DIR", dir)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the bridge");
    child
        .stdin
        .take()
        .expect("bridge stdin")
        .write_all(prompt.as_bytes())
        .expect("write the prompt");
    let out = child.wait_with_output().expect("wait for the bridge");
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("stdout line is JSON"))
        .collect();
    Run {
        code: out.status.code().expect("bridge exit code"),
        lines,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        argv: std::fs::read_to_string(dir.join("argv.txt")).unwrap_or_default(),
        stdin_seen: std::fs::read_to_string(dir.join("stdin.txt")).unwrap_or_default(),
    }
}

const OK_SCRIPT: &str = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "codex-cli 9.9.9"; exit 0; fi
echo "$@" > "$FAKE_DIR/argv.txt"
cat > "$FAKE_DIR/stdin.txt"
echo '{"type":"thread.started","thread_id":"t-1"}'
echo '{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"got it"}}'
echo '{"type":"turn.completed","usage":{}}'
"#;

#[test]
fn frames_a_successful_run() {
    let r = run("success", Some(OK_SCRIPT), "do the thing");
    assert_eq!(r.code, 0);
    assert_eq!(r.argv.trim(), "exec --json --skip-git-repo-check -");
    assert_eq!(r.stdin_seen, "do the thing");
    assert_eq!(r.lines.len(), 4, "{:?}", r.lines);
    assert_eq!(r.lines[0]["subtype"], "init");
    assert_eq!(r.lines[0]["session_id"], "t-1");
    assert_eq!(r.lines[0]["model"], "codex");
    assert_eq!(r.lines[1]["codex_version"], "codex-cli 9.9.9");
    assert_eq!(
        r.lines[2],
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"got it"}]}})
    );
    assert_eq!(r.lines[3], json!({"type":"result","subtype":"success"}));
    assert!(
        r.stderr.contains("codex exit status: code 0"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_codex_exit_without_a_terminal_event_is_unknown_and_nonzero() {
    let script = "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo v; exit 0; fi\ncat >/dev/null\necho '{\"type\":\"thread.started\",\"thread_id\":\"t\"}'\nexit 3\n";
    let r = run("no-terminal", Some(script), "x");
    assert_eq!(r.code, 1);
    assert_eq!(
        r.lines.last().expect("a last line"),
        &json!({"type":"result","subtype":"unknown"})
    );
    assert!(r.stderr.contains("ended without a terminal event"));
    assert!(
        r.stderr.contains("codex exit status: code 3"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_missing_executable_gets_its_own_diagnostic_and_unknown_result() {
    let r = run("missing-exe", None, "x");
    assert_eq!(r.code, 1);
    assert_eq!(r.lines.len(), 2, "{:?}", r.lines);
    assert_eq!(r.lines[0]["subtype"], "init");
    assert!(
        r.lines[0]["session_id"]
            .as_str()
            .expect("session id")
            .starts_with("codex-bridge-")
    );
    assert_eq!(r.lines[1], json!({"type":"result","subtype":"unknown"}));
    assert!(r.stderr.contains("cannot start"), "{}", r.stderr);
    assert!(!r.stderr.contains("ended without a terminal event"));
}

#[test]
fn an_empty_prompt_exits_2_without_stdout() {
    let r = run("empty-prompt", Some(OK_SCRIPT), "  \n");
    assert_eq!(r.code, 2);
    assert!(r.lines.is_empty());
}
