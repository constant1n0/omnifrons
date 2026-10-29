//! End-to-end check against a fake `codex` shell script.

#![cfg(unix)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};

fn run(script_body: &str, prompt: &str) -> Option<(i32, Vec<String>, String)> {
    let dir =
        std::env::temp_dir().join(format!("csb-test-{}-{}", std::process::id(), prompt.len()));
    std::fs::create_dir_all(&dir).ok()?;
    let fake = dir.join("fake-codex");
    std::fs::write(&fake, script_body).ok()?;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).ok()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_codex-stream-bridge"))
        .env("CODEX_STREAM_BRIDGE_CODEX", &fake)
        .env("FAKE_DIR", &dir)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(prompt.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let argv = std::fs::read_to_string(dir.join("argv.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    Some((out.status.code()?, lines, argv))
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
    let Some((code, lines, argv)) = run(OK_SCRIPT, "do the thing") else {
        eprintln!("skipped: cannot execute a script here");
        return;
    };
    assert_eq!(code, 0);
    assert_eq!(argv.trim(), "exec --json --skip-git-repo-check -");
    assert!(lines[0].contains(r#""subtype":"init""#) && lines[0].contains(r#""session_id":"t-1""#));
    assert!(lines[1].contains("codex-cli 9.9.9"));
    assert!(lines[2].contains(r#""text":"got it""#));
    assert_eq!(lines[3], r#"{"subtype":"success","type":"result"}"#);
}

#[test]
fn a_codex_exit_without_a_terminal_event_is_unknown_and_nonzero() {
    let script = "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo v; exit 0; fi\ncat >/dev/null\necho '{\"type\":\"thread.started\",\"thread_id\":\"t\"}'\nexit 3\n";
    let Some((code, lines, _)) = run(script, "x") else {
        return;
    };
    assert_eq!(code, 1);
    assert_eq!(
        lines.last().unwrap(),
        r#"{"subtype":"unknown","type":"result"}"#
    );
}

#[test]
fn an_empty_prompt_exits_2_without_stdout() {
    let Some((code, lines, _)) = run(OK_SCRIPT, "  \n") else {
        return;
    };
    assert_eq!(code, 2);
    assert!(lines.is_empty());
}
