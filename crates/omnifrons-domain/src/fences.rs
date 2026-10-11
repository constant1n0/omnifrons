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

/// The character and run length a fence-shaped line opens or closes
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceDelimiter {
    marker: char,
    run: usize,
}

impl FenceDelimiter {
    /// The delimiter `trimmed_line` opens or closes with, or `None`
    /// when it is not a fence line at all: a run of three or more
    /// backticks or tildes at the very start (the shared `is_fence`
    /// rule [`crate::definition::title`] and
    /// [`crate::skills_index::indexed_skill_targets`] both already
    /// applied before this tracker existed).
    fn of(trimmed_line: &str) -> Option<Self> {
        let marker = trimmed_line.chars().next()?;
        if marker != '`' && marker != '~' {
            return None;
        }
        let run = trimmed_line.chars().take_while(|c| *c == marker).count();
        (run >= 3).then_some(Self { marker, run })
    }

    /// Whether `self`, read while a fence opened by `opener` is still
    /// open, is a compatible closer: the same character, with a run at
    /// least as long as the opener's (`CommonMark`'s closing-fence
    /// rule).
    fn closes(self, opener: Self) -> bool {
        self.marker == opener.marker && self.run >= opener.run
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
    /// around it.
    pub(crate) fn advance(&mut self, trimmed_line: &str) -> bool {
        let Some(delimiter) = FenceDelimiter::of(trimmed_line) else {
            return self.opener.is_some();
        };
        match self.opener {
            None => self.opener = Some(delimiter),
            Some(opener) if delimiter.closes(opener) => self.opener = None,
            Some(_) => {
                // A fence-shaped line of the wrong character or too
                // short a run, found while a fence is already open: it
                // is content, not a close (`CommonMark`), so the fence
                // stays open.
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
}
