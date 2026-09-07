//! Test-only helper binary: the launcher target
//! `TokioProcessSupervisor::spawn_harness` execs, and the subject of the
//! supervisor's own output-capture tests
//! (`crates/omnifrons-supervisor/tests/output_capture.rs`,
//! `output_backpressure.rs`, `output_framing.rs`, `demo_harness.rs`).
//!
//! Deliberately always built (no cargo feature gate): tests reach it via
//! `env!("CARGO_BIN_EXE_demo-harness")`, which Cargo only populates for a
//! binary that is actually part of the same package's normal build.
//!
//! Two argv shapes:
//!
//! - `demo-harness <demo-lines|demo-ignores-sigterm> <rate_hz> <lines>` --
//!   the real demo harness behavior (`omnifrons_supervisor::demo::run`).
//! - `demo-harness --long-line <bytes>` / `demo-harness --invalid-utf8` /
//!   `demo-harness --long-line-utf8 <chars>` / `demo-harness --crlf` /
//!   `demo-harness --crlf-line <bytes>` /
//!   `demo-harness --no-trailing-newline` / `demo-harness --burst <lines>`
//!   -- test-only flags that exercise the capture pipeline's line-length
//!   cap, lossy UTF-8 decoding, UTF-8 character-boundary-safe splitting,
//!   CRLF stripping (including a CRLF landing exactly at the per-frame
//!   cap), final unterminated-line delivery, and (`--burst`) deterministic
//!   output-channel overflow, directly and independent of `HarnessKind`.

use std::io::Write as _;
use std::process::ExitCode;

use omnifrons_supervisor::demo;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--long-line") => {
            let bytes: usize = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .expect("--long-line requires a byte count argument");
            emit_long_line(bytes)
        }
        Some("--invalid-utf8") => emit_invalid_utf8(),
        Some("--long-line-utf8") => {
            let chars: usize = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .expect("--long-line-utf8 requires a char count argument");
            emit_long_line_utf8(chars)
        }
        Some("--crlf") => emit_crlf_lines(),
        Some("--crlf-line") => {
            let bytes: usize = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .expect("--crlf-line requires a byte count argument");
            emit_crlf_line(bytes)
        }
        Some("--no-trailing-newline") => emit_no_trailing_newline(),
        Some("--burst") => {
            let lines: u32 = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .expect("--burst requires a line count argument");
            emit_burst(lines)
        }
        Some(kind_arg) => {
            let kind = demo::parse_kind(kind_arg)
                .unwrap_or_else(|| panic!("unknown demo harness kind: {kind_arg}"));
            let rate_hz: u16 = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .expect("a valid rate_hz argument is required");
            let lines: u32 = args
                .get(2)
                .and_then(|value| value.parse().ok())
                .expect("a valid lines argument is required");
            demo::run(&kind, rate_hz, lines)
        }
        None => {
            eprintln!("usage: demo-harness <demo-lines|demo-ignores-sigterm> <rate_hz> <lines>");
            ExitCode::FAILURE
        }
    }
}

/// Emit one line of exactly `bytes` `'x'` characters (plus a trailing
/// newline), then exit `0` -- for exercising `MAX_LINE_BYTES` splitting.
fn emit_long_line(bytes: usize) -> ExitCode {
    let mut stdout = std::io::stdout();
    let line = "x".repeat(bytes);
    writeln!(stdout, "{line}").expect("stdout must accept the long line");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit one line containing a byte sequence that is never valid UTF-8 in
/// any position, then exit `0` -- for exercising lossy decoding.
fn emit_invalid_utf8() -> ExitCode {
    let mut stdout = std::io::stdout();
    stdout
        .write_all(&[b'b', b'a', b'd', 0xFF, b'\n'])
        .expect("stdout must accept the invalid UTF-8 payload");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit one line made of `chars` repetitions of `'€'` (U+20AC, 3 bytes),
/// long enough (at a large enough `chars`) to exceed `MAX_LINE_BYTES` and
/// force a capture-side split -- for exercising UTF-8 character-boundary-
/// safe splitting.
///
/// A 3-byte character is deliberate, not arbitrary: 3 does not evenly
/// divide `MAX_LINE_BYTES` (8192), so a split exactly at the byte cap is
/// guaranteed to land mid-character at least once. A 2-byte character such
/// as `'é'` would not exercise this at all, since 8192 is itself a power of
/// two -- every cap-aligned split would coincidentally already fall on a
/// character boundary, passing even against the bug this flag exists to
/// catch.
fn emit_long_line_utf8(chars: usize) -> ExitCode {
    let mut stdout = std::io::stdout();
    let line = "€".repeat(chars);
    writeln!(stdout, "{line}").expect("stdout must accept the long UTF-8 line");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit two CRLF-terminated lines, then exit `0` -- for exercising the
/// capture pipeline's `\r\n` handling (a line end, with the `\r` stripped).
fn emit_crlf_lines() -> ExitCode {
    let mut stdout = std::io::stdout();
    stdout
        .write_all(b"line one\r\nline two\r\n")
        .expect("stdout must accept CRLF-terminated output");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit one CRLF-terminated line of exactly `bytes` `'x'` characters, then
/// exit `0` -- for exercising a CRLF that lands exactly at (or one byte
/// past) `MAX_LINE_BYTES`, where the capture side must still deliver one
/// un-continued frame with the CR stripped (R3-012).
fn emit_crlf_line(bytes: usize) -> ExitCode {
    let mut stdout = std::io::stdout();
    let line = "x".repeat(bytes);
    write!(stdout, "{line}\r\n").expect("stdout must accept the CRLF line");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit one line with no trailing newline at all, then exit `0` -- for
/// exercising delivery of a final unterminated line at EOF.
fn emit_no_trailing_newline() -> ExitCode {
    let mut stdout = std::io::stdout();
    stdout
        .write_all(b"no trailing newline")
        .expect("stdout must accept output with no trailing newline");
    stdout.flush().expect("stdout must flush");
    ExitCode::SUCCESS
}

/// Emit `lines` lines to stdout with no inter-line sleeping at all (unlike
/// the real demo harness's rate-limited `demo::run`), then exit `0`.
///
/// Deliberately deterministic and platform-timer-independent: with no
/// sleep, a large `lines` count is written far faster than any consumer's
/// bounded channel capacity, so this reliably overflows it regardless of a
/// platform's timer granularity -- unlike driving `HarnessKind::DemoLines`
/// at a high `rate_hz` and hoping enough lines land within a fixed
/// wall-clock window
/// (`crates/omnifrons-supervisor/tests/output_backpressure.rs`).
fn emit_burst(lines: u32) -> ExitCode {
    let mut stdout = std::io::stdout();
    for n in 1..=lines {
        writeln!(stdout, "line {n} out").expect("stdout must accept the burst harness's output");
        stdout.flush().expect("stdout must flush");
    }
    ExitCode::SUCCESS
}
