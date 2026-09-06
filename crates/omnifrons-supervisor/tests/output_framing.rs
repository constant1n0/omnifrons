//! Exercises the capture pipeline's framing directly, bypassing
//! `HarnessRequest`/`spawn_harness`: a line longer than `MAX_LINE_BYTES`
//! (8192) arrives as consecutive frames with no bytes lost, and invalid
//! UTF-8 is lossily decoded to U+FFFD rather than dropped or erroring.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{FramePayload, OutputStream, ProcessOutput, ProcessStatus, ProcessSupervisor};
use omnifrons_supervisor::TokioProcessSupervisor;

const REAP_DEADLINE: Duration = Duration::from_secs(5);

fn demo_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"))
}

/// Spawn the demo harness binary directly with `args` (bypassing
/// `HarnessRequest`, since these test-only flags have no `HarnessKind`),
/// wait for it to reach a terminal state, and return every frame delivered
/// on its output channel.
fn run_and_collect(args: &[&str]) -> Vec<omnifrons_app::OutputFrame> {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());
    let spec = omnifrons_app::ProcessSpec::new(demo_harness_path().to_string_lossy().into_owned())
        .with_args(args.iter().copied());
    let id = supervisor
        .spawn(spec)
        .expect("spawning demo-harness directly must succeed");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("demo-harness did not reach a terminal state in time: {other:?}"),
        }
    }

    rx.iter().collect()
}

#[test]
fn a_line_longer_than_max_line_bytes_splits_without_losing_bytes() {
    const LONG_LINE_BYTES: usize = 20_000;

    let frames = run_and_collect(&["--long-line", &LONG_LINE_BYTES.to_string()]);

    let mut reassembled = String::new();
    let mut text_frame_count = 0;
    for frame in &frames {
        if let FramePayload::Text { stream, text } = &frame.payload {
            assert_eq!(
                *stream,
                OutputStream::Stdout,
                "the long line is written to stdout"
            );
            assert!(
                text.len() <= 8192,
                "no single frame may exceed MAX_LINE_BYTES, got {} bytes",
                text.len()
            );
            reassembled.push_str(text);
            text_frame_count += 1;
        }
    }

    assert!(
        text_frame_count > 1,
        "a {LONG_LINE_BYTES}-byte line must split into more than one frame, got {text_frame_count}"
    );
    assert_eq!(
        reassembled,
        "x".repeat(LONG_LINE_BYTES),
        "reassembling every text frame in order must recover the exact original line, no bytes lost"
    );
}

#[test]
fn a_long_multibyte_utf8_line_reassembles_exactly_with_no_replacement_characters() {
    // Comfortably past MAX_LINE_BYTES (8192), and a byte width (3, '€' is
    // U+20AC) that does not evenly divide it -- see
    // `demo-harness`'s own `emit_long_line_utf8` doc comment for why that
    // choice matters: it guarantees a capture-side split lands mid-
    // character at least once, rather than merely by coincidence.
    const CHARS: usize = 10_000;

    let frames = run_and_collect(&["--long-line-utf8", &CHARS.to_string()]);

    let mut reassembled = String::new();
    let mut text_frame_count = 0;
    for frame in &frames {
        if let FramePayload::Text { stream, text } = &frame.payload {
            assert_eq!(
                *stream,
                OutputStream::Stdout,
                "the long UTF-8 line is written to stdout"
            );
            assert!(
                text.len() <= 8192,
                "no single frame may exceed MAX_LINE_BYTES, got {} bytes",
                text.len()
            );
            assert!(
                !text.contains('\u{FFFD}'),
                "a multibyte character must never be split mid-character across frames -- \
                 found a replacement character in {text:?}"
            );
            reassembled.push_str(text);
            text_frame_count += 1;
        }
    }

    assert!(
        text_frame_count > 1,
        "a {CHARS}-character '€' line must split into more than one frame, got {text_frame_count}"
    );
    assert_eq!(
        reassembled,
        "€".repeat(CHARS),
        "reassembling every text frame in order must recover the exact original line, with no \
         multibyte character split (and therefore corrupted) across a frame boundary"
    );
}

#[test]
fn crlf_terminated_lines_have_their_trailing_cr_stripped() {
    let frames = run_and_collect(&["--crlf"]);

    let text_frames: Vec<&str> = frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text { text, .. } => Some(text.as_str()),
            FramePayload::State(_) => None,
        })
        .collect();

    assert_eq!(
        text_frames,
        vec!["line one", "line two"],
        "a CRLF line ending must be treated as a line end with the trailing \\r stripped, \
         not left dangling on the decoded text"
    );
}

#[test]
fn a_final_line_with_no_trailing_newline_is_still_delivered_at_eof() {
    let frames = run_and_collect(&["--no-trailing-newline"]);

    let text_frames: Vec<&str> = frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text { text, .. } => Some(text.as_str()),
            FramePayload::State(_) => None,
        })
        .collect();

    assert_eq!(
        text_frames,
        vec!["no trailing newline"],
        "a final line with no trailing newline must still be delivered once EOF is reached, \
         not silently dropped for lacking a terminator"
    );
}

#[test]
fn invalid_utf8_is_lossily_decoded_to_the_replacement_character() {
    let frames = run_and_collect(&["--invalid-utf8"]);

    let text_frames: Vec<&str> = frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text { text, .. } => Some(text.as_str()),
            FramePayload::State(_) => None,
        })
        .collect();

    assert_eq!(
        text_frames.len(),
        1,
        "expected exactly one text frame, got {text_frames:?}"
    );
    assert_eq!(
        text_frames[0], "bad\u{FFFD}",
        "the invalid byte must be lossily replaced with U+FFFD, not dropped or causing an error"
    );
}
