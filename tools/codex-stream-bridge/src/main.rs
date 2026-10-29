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
use std::process::{Command, Stdio};
use translate::{Out, State, end_of_input, translate};

/// Overrides the Codex executable (default: `codex`, resolved through PATH).
const CODEX_ENV: &str = "CODEX_STREAM_BRIDGE_CODEX";
/// `1` surfaces Codex reasoning items as text blocks.
const REASONING_ENV: &str = "CODEX_STREAM_BRIDGE_REASONING";

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
            return emit(&end_of_input(&mut state)).unwrap_or(1);
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
            Ok(code) if code >= 0 => exit = Some(code),
            Ok(_) => {}
            Err(()) => exit = Some(1),
        }
    }
    let code = exit.unwrap_or_else(|| emit(&end_of_input(&mut state)).unwrap_or(1));

    if code != 0 {
        let _ = child.kill();
    }
    drop(reader);
    let _ = child.wait();
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    code
}

/// Write the outputs, flushing each stdout line. Returns the exit code of a
/// `Finish`, or `-1` when none was present; `Err` when stdout is gone.
fn emit(outs: &[Out]) -> Result<i32, ()> {
    let mut code = -1;
    for out in outs {
        match out {
            Out::Stdout(line) => {
                let mut stdout = std::io::stdout().lock();
                writeln!(stdout, "{line}")
                    .and_then(|()| stdout.flush())
                    .map_err(|_| ())?;
            }
            Out::Stderr(line) => eprintln!("{line}"),
            Out::Finish { exit } => code = *exit,
        }
    }
    Ok(code)
}

/// The harness version for the evidence (`codex --version`), when available.
fn codex_version(codex: &str) -> Option<String> {
    let output = Command::new(codex)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}
