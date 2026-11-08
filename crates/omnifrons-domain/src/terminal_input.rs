//! Input an operator forwards to a live pseudo-terminal child after its
//! prompt was typed (#187): one line of text, or one key from a closed set.
//!
//! Every value here reaches the child through the terminal's line
//! discipline, never as raw bytes chosen by a caller: a key is mapped to
//! fixed bytes on this side, and a line is checked by the same rule that
//! refuses an untypeable prompt ([`line_discipline_interprets`]).

/// Whether a prompt byte would be interpreted by the terminal's line
/// discipline (default `termios`: canonical mode, `ISIG`, `ICRNL`) rather
/// than typed: every C0 control except newline (ends a line the child
/// reads as such) and tab (typed literally), and DEL (`VERASE` on Linux and
/// macOS). Among the refused bytes are `VINTR` (0x03), `VEOF` (0x04),
/// `VSUSP` (0x1A), `VQUIT` (0x1C), ESC (would open a sequence), and `\r`
/// (would act as Enter mid-text). Multi-byte UTF-8 bytes are all
/// `>= 0x80` and never match.
#[must_use]
pub const fn line_discipline_interprets(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B..=0x1F | 0x7F)
}

/// A key from the closed set the operator can send (S3): Enter, Escape and
/// the four arrows. Each maps to fixed bytes ([`Self::bytes`]); no other
/// key, and no raw byte sequence, can be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalKey {
    /// A carriage return, the line discipline's end of line.
    Enter,
    /// A lone ESC byte.
    Escape,
    /// The cursor-up sequence.
    Up,
    /// The cursor-down sequence.
    Down,
    /// The cursor-left sequence.
    Left,
    /// The cursor-right sequence.
    Right,
}

impl TerminalKey {
    /// The fixed bytes this key writes: `\r` for Enter, `0x1B` for Escape,
    /// and the normal cursor-mode (CSI) sequences `ESC [ A`/`B`/`C`/`D` for
    /// Up, Down, Right and Left. The application-mode `ESC O` forms are not
    /// used (an assumption recorded in S3, revertible).
    #[must_use]
    pub const fn bytes(self) -> &'static [u8] {
        match self {
            Self::Enter => b"\r",
            Self::Escape => b"\x1b",
            Self::Up => b"\x1b[A",
            Self::Down => b"\x1b[B",
            Self::Right => b"\x1b[C",
            Self::Left => b"\x1b[D",
        }
    }
}

/// Why a text was refused as a [`TypedLine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedLineError {
    /// The text is empty: there is nothing to type (Enter alone is
    /// [`TerminalKey::Enter`]).
    Empty,
    /// The text holds a line break (`\n` or `\r`) or another byte the line
    /// discipline would interpret rather than type
    /// ([`line_discipline_interprets`]).
    NotTypeable,
    /// The text's UTF-8 byte length exceeds [`TypedLine::MAX_BYTES`].
    TooLarge,
}

impl std::fmt::Display for TypedLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Empty => "the line is empty",
            Self::NotTypeable => {
                "the line holds a line break or a byte the terminal would interpret"
            }
            Self::TooLarge => "the line exceeds the 4,095-byte canonical-buffer size",
        };
        f.write_str(message)
    }
}

impl std::error::Error for TypedLineError {}

/// One line of text the terminal types as such: non-empty, without a line
/// break, without a byte the line discipline would interpret, and at most
/// [`Self::MAX_BYTES`] UTF-8 bytes.
///
/// Constructible only through [`Self::new`], so every live `TypedLine` has
/// already passed every check.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypedLine(String);

impl TypedLine {
    /// The maximum UTF-8 byte length of a line, inclusive: the Linux
    /// canonical-buffer size, past which the kernel truncates a typed line
    /// silently (`docs/spike-log.md`, "Interactive input"). Other kernels
    /// may hold less in canonical mode.
    pub const MAX_BYTES: usize = 4095;

    /// Validate and build a line.
    ///
    /// # Errors
    ///
    /// Returns [`TypedLineError::Empty`] for an empty text,
    /// [`TypedLineError::TooLarge`] if its UTF-8 byte length exceeds
    /// [`Self::MAX_BYTES`], or [`TypedLineError::NotTypeable`] if it holds
    /// `\n`, `\r`, or any other byte [`line_discipline_interprets`].
    pub fn new(text: impl Into<String>) -> Result<Self, TypedLineError> {
        let text = text.into();
        if text.is_empty() {
            return Err(TypedLineError::Empty);
        }
        if text.len() > Self::MAX_BYTES {
            return Err(TypedLineError::TooLarge);
        }
        // `\r` is among the interpreted bytes; `\n` is not (inside a
        // prompt it ends a line the child reads), so it is refused here on
        // its own: a line is one line, and its Enter is appended.
        if text
            .bytes()
            .any(|byte| byte == b'\n' || line_discipline_interprets(byte))
        {
            return Err(TypedLineError::NotTypeable);
        }
        Ok(Self(text))
    }

    /// The validated text, without its end of line.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the operator sends to a live pseudo-terminal child: one line or
/// one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TerminalInput {
    /// A line, typed followed by a carriage return.
    Line(TypedLine),
    /// A key, written as its fixed bytes.
    Key(TerminalKey),
}

impl TerminalInput {
    /// The exact bytes to write to the terminal: a line's text followed by
    /// `\r` (the line discipline's end of line), or a key's fixed bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Line(line) => {
                let mut bytes = Vec::with_capacity(line.as_str().len() + 1);
                bytes.extend_from_slice(line.as_str().as_bytes());
                bytes.extend_from_slice(TerminalKey::Enter.bytes());
                bytes
            }
            Self::Key(key) => key.bytes().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalInput, TerminalKey, TypedLine, TypedLineError, line_discipline_interprets,
    };

    /// Each key's fixed bytes, exactly as S3 lists them.
    #[test]
    fn every_key_maps_to_its_fixed_bytes() {
        for (key, expected) in [
            (TerminalKey::Enter, &b"\r"[..]),
            (TerminalKey::Escape, &[0x1B][..]),
            (TerminalKey::Up, &b"\x1b[A"[..]),
            (TerminalKey::Down, &b"\x1b[B"[..]),
            (TerminalKey::Right, &b"\x1b[C"[..]),
            (TerminalKey::Left, &b"\x1b[D"[..]),
        ] {
            assert_eq!(key.bytes(), expected, "{key:?}");
            assert_eq!(TerminalInput::Key(key).to_bytes(), expected, "{key:?}");
        }
    }

    /// A line is written as its text followed by one carriage return.
    #[test]
    fn a_line_is_its_text_then_a_carriage_return() {
        let line = TypedLine::new("yes, trust it").expect("a plain line is typeable");
        assert_eq!(line.as_str(), "yes, trust it");
        assert_eq!(TerminalInput::Line(line).to_bytes(), b"yes, trust it\r");
    }

    /// The rule refuses exactly the C0 controls but newline and tab, and
    /// DEL: every one of the 256 byte values is checked.
    #[test]
    fn the_line_discipline_rule_refuses_exactly_c0_but_newline_and_tab_and_del() {
        for byte in 0..=u8::MAX {
            let refused = matches!(byte, 0x00..=0x08 | 0x0B..=0x1F | 0x7F);
            assert_eq!(
                line_discipline_interprets(byte),
                refused,
                "byte {byte:#04x}"
            );
        }
    }

    #[test]
    fn an_empty_line_is_refused() {
        assert_eq!(TypedLine::new(""), Err(TypedLineError::Empty));
    }

    /// A line is one line: Enter is appended, never carried.
    #[test]
    fn a_line_break_is_refused() {
        for text in ["one\ntwo", "one\rtwo", "trailing\n", "trailing\r", "\r\n"] {
            assert_eq!(
                TypedLine::new(text),
                Err(TypedLineError::NotTypeable),
                "{text:?}"
            );
        }
    }

    /// Every byte the line discipline interprets is refused inside a line:
    /// each refused C0 control, one at a time, and DEL.
    #[test]
    fn every_interpreted_control_byte_and_del_is_refused() {
        for byte in (0x00..=0x08).chain(0x0B..=0x1F).chain([0x7F]) {
            let text = format!("a{}b", char::from(byte));
            assert_eq!(
                TypedLine::new(text),
                Err(TypedLineError::NotTypeable),
                "byte {byte:#04x}"
            );
        }
    }

    #[test]
    fn a_line_past_the_canonical_buffer_is_refused() {
        assert_eq!(
            TypedLine::new("x".repeat(TypedLine::MAX_BYTES + 1)),
            Err(TypedLineError::TooLarge)
        );
        assert_eq!(TypedLine::MAX_BYTES + 1, 4096);
    }

    /// Tab is typed literally, multi-byte UTF-8 never matches the rule, and
    /// a line of exactly the maximum size is accepted, multi-byte included.
    #[test]
    fn tab_multi_byte_text_and_exactly_the_maximum_are_accepted() {
        for text in [
            "a\tb".to_string(),
            "héllo — 日本語 ✓".to_string(),
            "x".repeat(TypedLine::MAX_BYTES),
            format!("{}é", "x".repeat(TypedLine::MAX_BYTES - 2)),
        ] {
            let line = TypedLine::new(text.clone()).expect("a typeable line");
            assert_eq!(line.as_str(), text);
        }
    }
}
