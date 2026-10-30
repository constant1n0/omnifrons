//! `codex-stream-bridge`: presents `codex exec --json` as a `stream-json`
//! line agent. Reads the prompt from stdin, runs Codex, and translates its
//! JSONL events (see `translate`) into the shapes the `stream-json-cli`
//! adapter parses. See the crate README for the contract.
//!
//! A compiled binary, not a shebang script, for the same reason as the
//! `vp-s6-agent` fixture: the launch path may exec the file through a sealed
//! descriptor, where interpreter resolution is not something to rely on.

mod translate;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use translate::{Out, State, end_of_input, spawn_failed, translate};

/// Overrides the Codex executable (default: `codex`, resolved through PATH).
const CODEX_ENV: &str = "CODEX_STREAM_BRIDGE_CODEX";
/// `1` surfaces Codex reasoning items as text blocks.
const REASONING_ENV: &str = "CODEX_STREAM_BRIDGE_REASONING";
/// How long `codex --version` may take before the version line is omitted.
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
/// How long Codex may take to exit after its output ended or a result was
/// emitted, before the bridge kills it.
const EXIT_GRACE: Duration = Duration::from_secs(10);

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let mut prompt = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut prompt) {
        eprintln!("codex-stream-bridge: cannot read the prompt from stdin: {err}");
        return 2;
    }
    if prompt.trim().is_empty() {
        eprintln!("codex-stream-bridge: empty prompt on stdin");
        return 2;
    }

    let codex = std::env::var(CODEX_ENV).unwrap_or_else(|_| "codex".to_string());
    let cwd = std::env::current_dir().map_or_else(|_| ".".to_string(), |p| p.display().to_string());
    let reasoning = std::env::var(REASONING_ENV).is_ok_and(|v| v == "1");
    let mut state = State::new(cwd, codex_version(&codex), reasoning);

    let mut child = match Command::new(&codex)
        .args(["exec", "--json", "--skip-git-repo-check", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            eprintln!("codex-stream-bridge: cannot start {codex}: {err}");
            return emit(&spawn_failed(&mut state)).ok().flatten().unwrap_or(1);
        }
    };

    // Feed the prompt from a thread so a full stdout pipe cannot deadlock it.
    let writer = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        })
    });
    let Some(stdout) = child.stdout.take() else {
        eprintln!("codex-stream-bridge: no stdout pipe from codex");
        let _ = child.kill();
        return 1;
    };

    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    let mut exit = None;
    while exit.is_none() {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            continue;
        }
        match emit(&translate(line, &mut state)) {
            Ok(finish) => exit = finish,
            Err(_) => exit = Some(1),
        }
    }
    let code = exit.unwrap_or_else(|| emit(&end_of_input(&mut state)).ok().flatten().unwrap_or(1));

    drop(reader);
    reap(&mut child);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    code
}

/// Wait for Codex to exit within the grace period, killing it after, and
/// write one stderr line with its exit status.
fn reap(child: &mut Child) {
    let status = wait_with_deadline(child, EXIT_GRACE).or_else(|| {
        eprintln!(
            "codex-stream-bridge: codex still running {}s after the end of its output; killing it",
            EXIT_GRACE.as_secs()
        );
        let _ = child.kill();
        child.wait().ok()
    });
    match status {
        Some(status) => eprintln!(
            "codex-stream-bridge: codex exit status: {}",
            describe(status)
        ),
        None => eprintln!("codex-stream-bridge: codex exit status: unavailable"),
    }
}

/// Poll `child` until it exits or `deadline` passes; `None` on timeout.
fn wait_with_deadline(child: &mut Child, deadline: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if start.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => return None,
        }
    }
}

fn describe(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    status
        .code()
        .map_or_else(|| "unknown".to_string(), |code| format!("code {code}"))
}

/// Write the outputs, flushing each stdout line. Returns the exit code of a
/// `Finish`, if present; `Err` when stdout is gone.
fn emit(outs: &[Out]) -> std::io::Result<Option<i32>> {
    let mut finish = None;
    for out in outs {
        match out {
            Out::Stdout(line) => {
                let mut stdout = std::io::stdout().lock();
                writeln!(stdout, "{line}")?;
                stdout.flush()?;
            }
            Out::Stderr(line) => eprintln!("{line}"),
            Out::Finish { exit } => finish = Some(*exit),
        }
    }
    Ok(finish)
}

/// The harness version for the evidence (`codex --version`), when it is
/// available within the deadline.
fn codex_version(codex: &str) -> Option<String> {
    let mut child = Command::new(codex)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(status) = wait_with_deadline(&mut child, VERSION_DEADLINE) else {
        eprintln!("codex-stream-bridge: codex --version timed out; omitting the version line");
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    let text = text.trim().to_string();
    (status.success() && !text.is_empty()).then_some(text)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn deadline_returns_the_status_of_a_quick_child() {
        let mut child = Command::new("true").spawn().expect("spawn true");
        let status = wait_with_deadline(&mut child, Duration::from_secs(5));
        assert!(status.is_some_and(|s| s.success()));
    }

    #[test]
    fn deadline_times_out_on_a_slow_child() {
        let mut child = Command::new("sleep").arg("5").spawn().expect("spawn sleep");
        let start = Instant::now();
        assert!(wait_with_deadline(&mut child, Duration::from_millis(100)).is_none());
        assert!(start.elapsed() < Duration::from_secs(4));
        child.kill().expect("kill");
        child.wait().expect("wait");
    }
}
