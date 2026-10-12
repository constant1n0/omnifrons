//! A Markdown fence tracker shared by [`crate::definition::title`] and
//! [`crate::skills_index::indexed_skill_targets`]: both read a document
//! line by line and need to know whether the current line sits inside a
//! fenced code block before treating it as a heading or a link.
//!
//! `CommonMark` closes a fence only with a delimiter of the same
//! character (backtick or tilde) whose run is at least as long as the
//! opener's. The `base_omnifrons` prototype's `is_fence`/`fence_flags`
//! toggle a single boolean on any fence-shaped line instead, with no
//! regard for which character opened the fence or how long either run
//! is -- so, in the prototype, a `~~~` line found inside a backtick
//! fence closes it, and a closing run shorter than the opener's closes
//! it too. This tracker follows `CommonMark`'s compatible-delimiter
//! rule instead: a deliberate, more precise deviation from the
//! prototype, kept because misreading a closing delimiter inside a
//! fence can expose a heading or a link that the fenced code was meant
//! to hide. Every translated prototype test still passes: each of its
//! fixtures opens and closes a fence with the same character and run
//! length, the one case where the naive toggle and the precise rule
//! agree.
//!
//! `CommonMark` also forbids an info string (text trailing the run,
//! other than whitespace) on a closing fence, though it allows one on
//! an opening fence; the prototype's `is_fence` has no notion of an
//! info string at all, so it would toggle a fence closed on, say, a
//! backtick run immediately followed by a language tag, found inside
//! one -- the same kind of misreading the compatible-delimiter rule
//! above already guards against. This tracker refuses to close on
//! such a line -- content, not a close -- for the same reason, and it
//! is the same kind of deviation: no translated prototype fixture
//! carries an info string on a fence line, so this is strictly more
//! precise, never a regression.
//!
//! Trimming is a caller contract, not a judgment this tracker makes on
//! its own: every caller -- the private `unfenced_lines` in
//! `warnings.rs`, [`crate::definition::title`], and
//! [`crate::skills_index::indexed_skill_targets`] -- already trims each
//! line before calling [`FenceTracker::advance`]. That mirrors the
//! prototype's `is_fence`, which also strips each line before testing
//! its prefix, so a backtick or tilde run indented by four or more
//! spaces -- an indented code block in `CommonMark`, never a fence --
//! still opens or closes a fence here, in every one of those three
//! callers, exactly as it does in the prototype's own `unfenced`. This
//! is deliberate prototype parity, not a precision gap this tracker
//! could close on its own: telling an indented code block apart from a
//! fence needs each line's original indentation, and every caller
//! already discards that before this tracker ever sees the line.

/// The character and run length a fence-shaped line opens or closes
/// with, and whether anything other than whitespace follows the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceDelimiter {
    marker: char,
    run: usize,
    /// True when the run of `marker` characters is followed by any
    /// non-whitespace text: an info string, in `CommonMark` terms.
    /// `CommonMark` allows one on an opening fence but never on a
    /// closing one; see [`Self::closes`].
    has_info_string: bool,
}

impl FenceDelimiter {
    /// The delimiter `trimmed_line` opens or closes with, or `None`
    /// when it is not a fence line at all: a run of three or more
    /// backticks or tildes at the very start (the shared `is_fence`
    /// rule [`crate::definition::title`] and
    /// [`crate::skills_index::indexed_skill_targets`] both already
    /// applied before this tracker existed). Indexing
    /// `trimmed_line[run..]` by `run` is always on a character
    /// boundary: every one of the `run` characters counted is the
    /// same one-byte ASCII `marker`, so the byte offset and the
    /// character offset agree.
    fn of(trimmed_line: &str) -> Option<Self> {
        let marker = trimmed_line.chars().next()?;
        if marker != '`' && marker != '~' {
            return None;
        }
        let run = trimmed_line.chars().take_while(|c| *c == marker).count();
        if run < 3 {
            return None;
        }
        let has_info_string = trimmed_line[run..].chars().any(|c| !c.is_whitespace());
        Some(Self {
            marker,
            run,
            has_info_string,
        })
    }

    /// Whether `self`, read while a fence opened by `opener` is still
    /// open, is a compatible closer: the same character, with a run at
    /// least as long as the opener's (`CommonMark`'s closing-fence
    /// rule), and no info string -- `CommonMark` forbids one on a
    /// closing fence, though it allows one on an opening fence (review
    /// follow-up, close it: an earlier version accepted a closer
    /// carrying one, which the module documentation now explains).
    fn closes(self, opener: Self) -> bool {
        self.marker == opener.marker && self.run >= opener.run && !self.has_info_string
    }
}

/// Tracks whether each line of a Markdown document sits inside a fenced
/// code block, `CommonMark`-precise (see the module documentation for
/// the deliberate deviation from the prototype's any-delimiter toggle).
/// `pub(crate)`: an implementation detail shared by [`crate::definition`]
/// and [`crate::skills_index`], never a public type of this crate.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FenceTracker {
    /// The delimiter that opened the fence currently open, if any.
    opener: Option<FenceDelimiter>,
}

impl FenceTracker {
    /// A tracker positioned before the first line, outside any fence.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feeds one already-trimmed line and reports whether it should be
    /// excluded from heading or link detection: every fence-shaped line
    /// is excluded, whether it opens a fence, closes one, or -- found
    /// while a fence is already open -- is itself content because its
    /// character or run length cannot close the open fence; a
    /// non-fence line is excluded exactly when a fence is still open
    /// around it. Trimming before the call is the caller's job, not
    /// this method's -- see the module documentation for why that is
    /// deliberate prototype parity, not a gap, even for an indented
    /// backtick or tilde run.
    pub(crate) fn advance(&mut self, trimmed_line: &str) -> bool {
        let Some(delimiter) = FenceDelimiter::of(trimmed_line) else {
            return self.opener.is_some();
        };
        match self.opener {
            None => self.opener = Some(delimiter),
            Some(opener) if delimiter.closes(opener) => self.opener = None,
            Some(_) => {
                // A fence-shaped line that cannot close the fence
                // already open -- the wrong character, too short a
                // run, or an info string on an otherwise-matching run
                // -- is content, not a close (`CommonMark`), so the
                // fence stays open.
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::FenceTracker;

    /// A plain line outside any fence is never excluded; the same line
    /// inside an open fence always is.
    #[test]
    fn a_non_fence_line_follows_whether_a_fence_is_open() {
        let mut tracker = FenceTracker::new();
        assert!(!tracker.advance("plain text"));
        assert!(tracker.advance("```"));
        assert!(tracker.advance("still inside"));
        assert!(tracker.advance("```"));
        assert!(!tracker.advance("outside again"));
    }

    /// A `~~~` line found while a backtick fence is open does not close
    /// it, and is itself excluded as fence content.
    #[test]
    fn a_mismatched_character_does_not_close_the_fence() {
        let mut tracker = FenceTracker::new();
        assert!(tracker.advance("```"));
        assert!(tracker.advance("~~~"), "still fence content, not a close");
        assert!(tracker.advance("after the stray ~~~"));
        assert!(
            tracker.advance("````"),
            "same character, longer run: closes"
        );
        assert!(!tracker.advance("outside"));
    }

    /// A closing run shorter than the opener's does not close the
    /// fence; a run at least as long as the opener's does.
    #[test]
    fn a_closing_run_must_be_at_least_as_long_as_the_opener() {
        let mut tracker = FenceTracker::new();
        assert!(tracker.advance("````"));
        assert!(tracker.advance("```"), "shorter run: still open");
        assert!(tracker.advance("````"), "same run: closes");
        assert!(!tracker.advance("outside"));
    }

    /// Review follow-up (close it): an info string on an
    /// otherwise-matching run does not close the fence --
    /// `CommonMark` forbids one on a closing fence -- so the line is
    /// content, and the fence stays open until a plain closer with no
    /// trailing text is found.
    #[test]
    fn an_info_string_on_an_otherwise_matching_line_does_not_close_the_fence() {
        let mut tracker = FenceTracker::new();
        assert!(tracker.advance("```"));
        assert!(
            tracker.advance("```rust"),
            "an info string keeps this line as content, not a close"
        );
        assert!(
            tracker.advance("```"),
            "a plain run of the same length finally closes it"
        );
        assert!(!tracker.advance("outside"));
    }

    /// Trailing whitespace after the run is never an info string, so
    /// it still closes the fence.
    #[test]
    fn trailing_whitespace_after_the_run_still_closes_the_fence() {
        let mut tracker = FenceTracker::new();
        assert!(tracker.advance("```"));
        assert!(
            tracker.advance("``` "),
            "trailing whitespace only: still closes"
        );
        assert!(!tracker.advance("outside"));
    }

    /// An opening fence may carry an info string: `CommonMark` only
    /// forbids one on a closing fence.
    #[test]
    fn an_opening_fence_may_carry_an_info_string() {
        let mut tracker = FenceTracker::new();
        assert!(
            tracker.advance("```rust"),
            "an info string does not block opening"
        );
        assert!(tracker.advance("still inside"));
        assert!(tracker.advance("```"));
        assert!(!tracker.advance("outside"));
    }

    /// A shorter run carrying an info string, found while a
    /// longer-run fence is already open, is content: too short to
    /// close regardless of the info string, and it does not replace
    /// the already-open fence's own opener.
    #[test]
    fn a_shorter_run_with_an_info_string_does_not_open_a_new_fence_while_one_is_open() {
        let mut tracker = FenceTracker::new();
        assert!(tracker.advance("````"));
        assert!(
            tracker.advance("```rust"),
            "too short to close, info string or not: still content"
        );
        assert!(tracker.advance("````"), "same run as the opener: closes");
        assert!(!tracker.advance("outside"));
    }
}
