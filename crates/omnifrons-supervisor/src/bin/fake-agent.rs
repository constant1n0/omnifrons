//! Test-only helper binary: a synthetic `stream-json` agent CLI, standing
//! in for a real built-in-adapter target
//! (`crates/omnifrons-adapters/src/line_agent.rs`) in this crate's own
//! adapter-launch tests (`tests/adapter_launch.rs`, `tests/adapter_events.rs`).
//!
//! Deliberately always built (no cargo feature gate), exactly like
//! `demo-harness`: tests reach it via `env!("CARGO_BIN_EXE_fake-agent")`.
//! This crate has no dependency on `serde_json` (`docs/spike-log.md` §
//! Slice 3: `omnifrons-supervisor`'s own dependency table stays unchanged),
//! so every JSON line below is hand-built with a small, deliberately
//! minimal escaper -- sufficient for the fixed, self-controlled content
//! this binary ever emits, never a general-purpose JSON writer.
//!
//! Default behavior (no flags, or `--exit-code`/`--no-eof` only): reads
//! stdin to EOF (unless `--no-eof`), then emits, one per stdout line:
//! 1. `{"type":"system","subtype":"init","cwd":...,"model":"fake-model","session_id":"fake-session"}`,
//!    with `cwd` this process's own observed working directory;
//! 2. one assistant `text` block per prompt line read from stdin, each
//!    echoing that line verbatim;
//! 3. one assistant `tool_use` block (`name: "write_file"`, `input:
//!    {"path": "notes.md"}`);
//! 4. one deliberately truncated line, `{"type":"assistant"` (no closing
//!    brace);
//! 5. one stderr line, `diagnostic: hello`;
//! 6. one assistant `text` block whose text is this process's own full
//!    environment, as `KEY=VALUE` lines joined by `\n` (embedded, JSON-
//!    escaped, inside that one text field) -- so a test can read exactly
//!    what environment this child actually observed;
//! 7. `{"type":"result","subtype":"success"}`.
//!
//! Then exits with the code `--exit-code N` requested (default `0`).
//!
//! `--flood N`: emits `N` assistant `text` lines (`"flood <n>"`) with no
//! inter-line sleeping at all, then exits -- for deterministically
//! overflowing an undrained output-delivery channel
//! (`tests/adapter_events.rs`).
//!
//! `--huge-line N`: emits exactly one assistant `text` line whose `text`
//! field is `N` bytes of padding, then exits -- for exercising
//! `LineAssembler`'s truncation cap (`tests/adapter_events.rs`).
//!
//! `--stdin-byte-count`: reads stdin to EOF and prints exactly one plain
//! line, `stdin-byte-count <n>`, with the number of bytes read, then exits
//! -- for proving a `StdinPlan::Null` launch really hands the child an
//! empty stdin on every platform (`tests/approved_launch.rs`).
//!
//! Spike slice 4's pseudo-terminal modes (`tests/pty_launch.rs`):
//!
//! `--pty-report`: prints `isatty stdin=<bool> stdout=<bool>
//! stderr=<bool>`, then `TERM=<value>`, `COLUMNS=<value>`, and
//! `LINES=<value>` (the three values the supervisor itself sets on a PTY
//! child), then one `env-keys <K1> <K2> ...` line naming every
//! environment key this process observes -- names only, never values --
//! and exits 0.
//!
//! `--pty-echo`: reads one line from stdin and prints `typed: <line>`
//! (line terminator stripped), then exits 0.
//!
//! `--pty-corpus`: writes the shared terminal corpus
//! (`omnifrons_app::terminal_normalizer::corpus::bytes`) to stdout
//! verbatim, then a newline and a final `corpus-done` line, and exits 0.
//!
//! `--pty-ignore-sigterm` (unix): installs the demo harness's own
//! `SIGTERM`-ignore disposition, prints `ready` once it is in place (the
//! synchronization point a test waits on before calling `stop`), then
//! sleeps until killed.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut exit_code: u8 = 0;
    let mut flood: Option<u32> = None;
    let mut huge_line: Option<usize> = None;
    let mut stdin_byte_count = false;
    let mut no_eof = false;
    let mut pty_report = false;
    let mut pty_echo = false;
    let mut pty_corpus = false;
    let mut pty_ignore_sigterm = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pty-report" => pty_report = true,
            "--pty-echo" => pty_echo = true,
            "--pty-corpus" => pty_corpus = true,
            "--pty-ignore-sigterm" => pty_ignore_sigterm = true,
            "--exit-code" => {
                i += 1;
                exit_code = args
                    .get(i)
                    .and_then(|value| value.parse().ok())
                    .expect("--exit-code requires a u8 value");
            }
            "--flood" => {
                i += 1;
                flood = Some(
                    args.get(i)
                        .and_then(|value| value.parse().ok())
                        .expect("--flood requires a u32 value"),
                );
            }
            "--huge-line" => {
                i += 1;
                huge_line = Some(
                    args.get(i)
                        .and_then(|value| value.parse().ok())
                        .expect("--huge-line requires a usize value"),
                );
            }
            "--stdin-byte-count" => stdin_byte_count = true,
            "--no-eof" => no_eof = true,
            other => panic!("unknown fake-agent flag: {other}"),
        }
        i += 1;
    }

    let mut stdout = std::io::stdout();

    if pty_report {
        run_pty_report(&mut stdout);
        return ExitCode::from(exit_code);
    }

    if pty_echo {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .expect("reading one line from stdin must succeed");
        let line = line.trim_end_matches(['\r', '\n']);
        emit(&mut stdout, &format!("typed: {line}"));
        return ExitCode::from(exit_code);
    }

    if pty_corpus {
        stdout
            .write_all(&omnifrons_app::terminal_normalizer::corpus::bytes())
            .expect("stdout must accept the corpus");
        stdout
            .write_all(b"\ncorpus-done\n")
            .expect("stdout must accept the corpus-done line");
        stdout.flush().expect("stdout must flush");
        return ExitCode::from(exit_code);
    }

    if pty_ignore_sigterm {
        #[cfg(unix)]
        omnifrons_supervisor::demo::ignore_sigterm();
        emit(&mut stdout, "ready");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    if stdin_byte_count {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .expect("reading stdin to EOF must succeed");
        emit(&mut stdout, &format!("stdin-byte-count {}", bytes.len()));
        return ExitCode::from(exit_code);
    }

    if let Some(n) = flood {
        for line in 1..=n {
            emit(&mut stdout, &assistant_text_line(&format!("flood {line}")));
        }
        return ExitCode::from(exit_code);
    }

    if let Some(bytes) = huge_line {
        let padding = "x".repeat(bytes);
        emit(&mut stdout, &assistant_text_line(&padding));
        return ExitCode::from(exit_code);
    }

    run_default_sequence(&mut stdout, no_eof);
    ExitCode::from(exit_code)
}

/// The default full `stream-json` sequence: init, one text block per
/// prompt line, one `tool_use` block, a truncated line, a stderr
/// diagnostic, an environment dump, then a result.
fn run_default_sequence(stdout: &mut impl Write, no_eof: bool) {
    let prompt_lines: Vec<String> = if no_eof {
        Vec::new()
    } else {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .expect("reading stdin to EOF must succeed");
        input.lines().map(str::to_string).collect()
    };

    let cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    emit(
        stdout,
        &format!(
            r#"{{"type":"system","subtype":"init","cwd":{cwd},"model":"fake-model","session_id":"fake-session"}}"#,
            cwd = json_string(&cwd)
        ),
    );

    for line in &prompt_lines {
        emit(stdout, &assistant_text_line(line));
    }

    emit(
        stdout,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-1","name":"write_file","input":{"path":"notes.md"}}]}}"#,
    );

    // Deliberately truncated: no closing brace at all.
    writeln!(stdout, r#"{{"type":"assistant""#).expect("stdout must accept the truncated line");
    stdout.flush().expect("stdout must flush");

    let mut stderr = std::io::stderr();
    writeln!(stderr, "diagnostic: hello").expect("stderr must accept the diagnostic line");
    stderr.flush().expect("stderr must flush");

    let mut env_pairs: Vec<(String, String)> = std::env::vars().collect();
    env_pairs.sort();
    let env_dump = env_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    emit(stdout, &assistant_text_line(&env_dump));

    emit(stdout, r#"{"type":"result","subtype":"success"}"#);
}

/// `--pty-report`: whether each standard stream is a terminal, the three
/// variables the supervisor sets on a PTY child, and every environment key
/// name (never a value).
fn run_pty_report(stdout: &mut impl Write) {
    use std::io::IsTerminal as _;

    emit(
        stdout,
        &format!(
            "isatty stdin={} stdout={} stderr={}",
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal()
        ),
    );
    for key in ["TERM", "COLUMNS", "LINES"] {
        emit(
            stdout,
            &format!("{key}={}", std::env::var(key).unwrap_or_default()),
        );
    }
    let mut keys: Vec<String> = std::env::vars_os()
        .map(|(key, _value)| key.to_string_lossy().into_owned())
        .collect();
    keys.sort();
    emit(stdout, &format!("env-keys {}", keys.join(" ")));
}

/// Write `line` followed by a newline, then flush -- every emitted line is
/// visible to a reader immediately, never buffered indefinitely.
fn emit(writer: &mut impl Write, line: &str) {
    writeln!(writer, "{line}").expect("stdout must accept fake-agent output");
    writer.flush().expect("stdout must flush");
}

/// Build one assistant `text`-block `stream-json` line.
fn assistant_text_line(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":{}}}]}}}}"#,
        json_string(text)
    )
}

/// A minimal JSON string literal (quotes plus escaping), sufficient for
/// this binary's own fixed, self-controlled content -- never a
/// general-purpose JSON writer (this crate has no `serde_json`
/// dependency, deliberately, per `docs/spike-log.md` § Slice 3).
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
