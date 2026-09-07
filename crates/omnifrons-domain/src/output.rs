//! Process output frames: decoded text lines from a child's stdout/stderr,
//! and the final terminal-state marker for that child's output stream.
//!
//! Framework-independent (this module lives in `omnifrons-domain`, which
//! depends on `std` and `thiserror` only): it names the shape any
//! `ProcessOutput` port implementation produces, without committing to how
//! frames are captured, queued, or delivered.

use crate::scope::ProcessTerminalState;

/// The maximum text bytes a single captured [`OutputFrame`] carries:
/// `omnifrons-supervisor`'s own output-capture framing splits a line
/// longer than this into consecutive frames at a UTF-8 character boundary,
/// marking every frame but the last of such a split
/// [`FramePayload::Text::continued`]. Defined once, here, so the capturing
/// side and any consumer bounding its own reassembly buffer
/// (`omnifrons-app`'s `harness_adapter::LineAssembler`) can never drift
/// onto different values for what is, structurally, one shared framing
/// contract (`docs/spike-log.md` § Slice 3). A frame's length alone says
/// nothing about whether the line continues -- only the flag does.
pub const MAX_TEXT_FRAME_BYTES: usize = 8192;

/// Which standard stream a decoded text frame came from.
///
/// Closed and exhaustive: a process has exactly two output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputStream {
    /// The child's standard output.
    Stdout,
    /// The child's standard error.
    Stderr,
}

/// The content carried by one [`OutputFrame`].
///
/// A terminal-state marker is not itself "from" stdout or stderr, so
/// [`OutputStream`] lives inside [`Self::Text`] rather than as a field on
/// [`OutputFrame`] that every payload would have to populate meaninglessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramePayload {
    /// A decoded line of text from `stream`. Lossy UTF-8 and line-splitting
    /// policy are the capturing adapter's concern, not this type's.
    Text {
        /// Which stream this text came from.
        stream: OutputStream,
        /// The decoded text.
        text: String,
        /// `true` if this frame ends because the capturing adapter's
        /// per-frame cap ([`MAX_TEXT_FRAME_BYTES`]) forced a split and more
        /// of the *same* logical line follows in the next frame of this
        /// stream; `false` if this frame ends at a genuine line end (a
        /// newline, or EOF). Set explicitly by the capturing side, never
        /// inferred by a consumer from the frame's length: a genuine line
        /// of exactly the cap's length is `false`.
        continued: bool,
    },
    /// The process reached this terminal state. Sent once, after every
    /// text frame for this process has been sent.
    State(ProcessTerminalState),
}

/// One frame in a process's output, in a single per-process sequence space
/// shared across stdout, stderr, and the final state frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFrame {
    /// This frame's position in the per-process sequence space. Contiguous
    /// per process: no seq is skipped, even across streams.
    pub seq: u64,
    /// How many frames were dropped (due to a full delivery queue) strictly
    /// before this one and after the last delivered frame. Zero when
    /// nothing was dropped.
    pub dropped_before: u64,
    /// This frame's content.
    pub payload: FramePayload,
}

impl OutputFrame {
    /// Build a frame with `dropped_before` at its default, zero.
    #[must_use]
    pub const fn new(seq: u64, payload: FramePayload) -> Self {
        Self {
            seq,
            dropped_before: 0,
            payload,
        }
    }

    /// Build a frame recording that `dropped_before` earlier frames were
    /// dropped before it was delivered.
    #[must_use]
    pub const fn with_dropped_before(seq: u64, dropped_before: u64, payload: FramePayload) -> Self {
        Self {
            seq,
            dropped_before,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FramePayload, OutputFrame, OutputStream};
    use crate::scope::ProcessTerminalState;

    #[test]
    fn new_frame_defaults_dropped_before_to_zero() {
        let frame = OutputFrame::new(
            1,
            FramePayload::Text {
                stream: OutputStream::Stdout,
                text: "line 1 out".to_string(),
                continued: false,
            },
        );

        assert_eq!(frame.seq, 1);
        assert_eq!(frame.dropped_before, 0);
    }

    #[test]
    fn with_dropped_before_carries_the_given_count() {
        let frame = OutputFrame::with_dropped_before(
            42,
            7,
            FramePayload::State(ProcessTerminalState::Exited { code: Some(0) }),
        );

        assert_eq!(frame.seq, 42);
        assert_eq!(frame.dropped_before, 7);
    }

    #[test]
    fn output_stream_variants_are_distinct() {
        assert_ne!(OutputStream::Stdout, OutputStream::Stderr);
    }

    /// A text frame says explicitly whether more of the same logical line
    /// follows in the next frame (`continued: true`, a forced split at
    /// the per-frame cap) or not -- never inferred from its length.
    #[test]
    fn text_frame_carries_its_continued_flag() {
        let split = FramePayload::Text {
            stream: OutputStream::Stdout,
            text: "a".repeat(8),
            continued: true,
        };
        let whole = FramePayload::Text {
            stream: OutputStream::Stdout,
            text: "a".repeat(8),
            continued: false,
        };
        assert_ne!(
            split, whole,
            "the same text with a different continued flag must be a different payload"
        );
    }

    #[test]
    fn frame_payload_variants_are_distinct() {
        let text = FramePayload::Text {
            stream: OutputStream::Stdout,
            text: "hi".to_string(),
            continued: false,
        };
        let state = FramePayload::State(ProcessTerminalState::Killed);
        assert_ne!(text, state);

        let stdout_text = FramePayload::Text {
            stream: OutputStream::Stdout,
            text: "hi".to_string(),
            continued: false,
        };
        let stderr_text = FramePayload::Text {
            stream: OutputStream::Stderr,
            text: "hi".to_string(),
            continued: false,
        };
        assert_ne!(stdout_text, stderr_text);
    }
}
