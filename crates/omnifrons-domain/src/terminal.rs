//! Terminal normalization vocabulary (spike slice 4, `docs/spike-log.md` §
//! Slice 4): the typed actions a pseudo-terminal byte stream may produce
//! once normalized under RCS-001's terminal control and OSC policy
//! (`docs/renderer-content-security.md` § Terminal control and OSC
//! policy), and the per-family counts of everything that policy dropped.
//!
//! Framework-independent (this module lives in `omnifrons-domain`): it
//! names the shapes the normalizer (`omnifrons-app`) produces and the
//! shell forwards, without committing to how bytes are read or rendered.
//! Nothing here is ever authorization: a PTY byte stream is untrusted
//! active content (`docs/threat-model.md` HAR-5, PRC-2), and the only
//! sequences that ever become a typed value are the two sanitized text
//! actions below -- every other family is dropped and counted.

/// How many recognized sequences the normalizer dropped, per family, since
/// the counts were last drained ([`Self::is_zero`] is the "nothing to
/// report" test a forwarder uses before emitting a diagnostics frame).
///
/// Every field is a plain count: this is diagnostics, never a rendering
/// instruction (RCS-001's "dropped; counted in diagnostics" row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DropCounts {
    /// Layout controls this slice has no terminal pane to apply: CSI SGR,
    /// cursor, erase, scroll, and mode set/reset (including the alternate
    /// screen), plus single-character and charset-selection escapes.
    pub layout: u64,
    /// OSC 8 hyperlink open/close sequences (inert by default; the label
    /// text between them stays text).
    pub hyperlink: u64,
    /// OSC 52 clipboard writes and queries (the payload never reaches any
    /// output).
    pub clipboard: u64,
    /// File-transfer and image protocols: the OSC 1337 family, Sixel (DCS
    /// `q`), and Kitty graphics (APC `G`).
    pub file_transfer: u64,
    /// Other DCS/APC/PM/SOS strings, ignored and bounded.
    pub string: u64,
    /// Sequences outside every recognized family: an unknown CSI final, an
    /// OSC number this policy does not name, or an 8-bit C1 control code
    /// point found in text.
    pub unknown: u64,
    /// Malformed sequences: a bare ESC at EOF or followed by an unexpected
    /// byte, a string that met a newline before its terminator, or a string
    /// that exceeded the length bound and was aborted.
    pub malformed: u64,
}

impl DropCounts {
    /// Whether every count is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.layout == 0
            && self.hyperlink == 0
            && self.clipboard == 0
            && self.file_transfer == 0
            && self.string == 0
            && self.unknown == 0
            && self.malformed == 0
    }
}

impl std::ops::AddAssign for DropCounts {
    /// Field-wise saturating accumulation, so a consumer summing counts
    /// over a whole stream can never overflow into a wrong (or panicking)
    /// total.
    fn add_assign(&mut self, other: Self) {
        self.layout = self.layout.saturating_add(other.layout);
        self.hyperlink = self.hyperlink.saturating_add(other.hyperlink);
        self.clipboard = self.clipboard.saturating_add(other.clipboard);
        self.file_transfer = self.file_transfer.saturating_add(other.file_transfer);
        self.string = self.string.saturating_add(other.string);
        self.unknown = self.unknown.saturating_add(other.unknown);
        self.malformed = self.malformed.saturating_add(other.malformed);
    }
}

/// The only two terminal sequences that normalize into a typed value, per
/// RCS-001's policy table: a title (OSC 0/2) and a notification (OSC 9).
/// Both carry already-sanitized text -- control and bidirectional-override
/// characters stripped, capped at 256 characters -- and both are emitted
/// as data for a consumer to show as text, never applied to any chrome,
/// pane header, or system notification by anything in this slice.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TerminalAction {
    /// A sanitized window/icon title the harness asked for.
    Title(String),
    /// A sanitized desktop-notification text the harness asked for.
    Notification(String),
}

#[cfg(test)]
mod tests {
    use super::{DropCounts, TerminalAction};

    #[test]
    fn default_drop_counts_are_zero() {
        assert!(DropCounts::default().is_zero());
    }

    #[test]
    fn any_nonzero_field_makes_is_zero_false() {
        for counts in [
            DropCounts {
                layout: 1,
                ..DropCounts::default()
            },
            DropCounts {
                hyperlink: 1,
                ..DropCounts::default()
            },
            DropCounts {
                clipboard: 1,
                ..DropCounts::default()
            },
            DropCounts {
                file_transfer: 1,
                ..DropCounts::default()
            },
            DropCounts {
                string: 1,
                ..DropCounts::default()
            },
            DropCounts {
                unknown: 1,
                ..DropCounts::default()
            },
            DropCounts {
                malformed: 1,
                ..DropCounts::default()
            },
        ] {
            assert!(!counts.is_zero(), "{counts:?} must not report as zero");
        }
    }

    #[test]
    fn add_assign_accumulates_field_wise_and_saturates() {
        let mut total = DropCounts {
            layout: u64::MAX,
            hyperlink: 1,
            ..DropCounts::default()
        };
        total += DropCounts {
            layout: 1,
            hyperlink: 2,
            malformed: 3,
            ..DropCounts::default()
        };
        assert_eq!(total.layout, u64::MAX, "saturating, never wrapping");
        assert_eq!(total.hyperlink, 3);
        assert_eq!(total.malformed, 3);
    }

    #[test]
    fn title_and_notification_are_distinct_actions() {
        assert_ne!(
            TerminalAction::Title("x".to_string()),
            TerminalAction::Notification("x".to_string())
        );
    }
}
