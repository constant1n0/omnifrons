//! `TerminalNormalizer`: a pure, streaming state machine over the bytes a
//! pseudo-terminal launch produces (spike slice 4, `docs/spike-log.md` §
//! Slice 4), applying RCS-001's terminal control and OSC policy
//! (`docs/renderer-content-security.md` § Terminal control and OSC
//! policy) in core, before anything reaches a renderer:
//!
//! - printable text passes through as [`TerminalChunk::Text`], lossily
//!   decoded, newlines kept, a bare `\r` kept as text (this slice has no
//!   terminal pane; the renderer strips C0 controls);
//! - the only two families that ever become a typed value are titles
//!   (OSC 0/2) and notifications (OSC 9), emitted as sanitized
//!   [`TerminalChunk::Action`]s -- data to show as text, never applied;
//! - every other recognized family is dropped and counted in
//!   [`DropCounts`]: layout controls (there is nothing to apply them to),
//!   hyperlinks (inert by default), clipboard writes *and* queries (the
//!   payload never reaches any output), file-transfer and image protocols,
//!   other control strings, and unknown or malformed sequences.
//!
//! Every OSC/DCS/APC/PM/SOS string is bounded by [`MAX_STRING_BYTES`]
//! (RCS-001 D11's 4 KiB default -- an open owner decision; the spike
//! default is recorded in `docs/spike-log.md` § Slice 4): on overflow, or
//! on a newline before the terminator, the sequence is aborted, counted
//! `malformed`, its consumed payload dropped, and every subsequent byte
//! renders as plain text. Nothing here is ever authorization: a PTY byte
//! stream is untrusted active content (`docs/threat-model.md` HAR-5,
//! PRC-2).

use omnifrons_domain::terminal::{DropCounts, TerminalAction};

/// The maximum payload bytes one OSC/DCS/APC/PM/SOS string (or one CSI
/// parameter/intermediate run) may accumulate before it is aborted as
/// malformed: RCS-001 D11's proposed default, adopted as this spike's
/// default pending the owner decision.
pub const MAX_STRING_BYTES: usize = 4096;

/// The maximum characters a sanitized title or notification carries
/// (RCS-001's policy table: "capped at 256 characters").
pub const MAX_ACTION_TEXT_CHARS: usize = 256;

const ESC: u8 = 0x1B;
const BEL: u8 = 0x07;

/// One unit of normalized output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalChunk {
    /// Plain text: everything outside a recognized sequence, lossily
    /// decoded, with 8-bit C1 controls stripped -- encoded code points
    /// and raw bytes alike.
    Text(String),
    /// A sanitized title or notification.
    Action(TerminalAction),
}

/// Which control-string introducer opened the string being collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringKind {
    /// `ESC P` (device control string; Sixel starts with a `q` final).
    Dcs,
    /// `ESC _` (application program command; Kitty graphics start `G`).
    Apc,
    /// `ESC ^` (privacy message).
    Pm,
    /// `ESC X` (start of string).
    Sos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Plain text.
    Ground,
    /// An ESC was just seen.
    Escape,
    /// `ESC` plus one or more intermediate bytes (charset selection etc.).
    EscapeIntermediate,
    /// Inside `ESC [`, collecting parameter and intermediate bytes.
    Csi,
    /// Inside `ESC ]`, collecting the payload.
    Osc,
    /// An ESC seen inside an OSC: only `\` (ST) may follow.
    OscEscape,
    /// Inside a DCS/APC/PM/SOS string, collecting the payload.
    ControlString(StringKind),
    /// An ESC seen inside a control string: only `\` (ST) may follow.
    ControlStringEscape(StringKind),
}

/// The streaming normalizer. State (the current sequence, the pending
/// text including any held-back incomplete UTF-8 tail, and the drop
/// counts) is kept across [`Self::push`] calls, so a sequence or a
/// multi-byte character split across two pushes is handled exactly as if
/// it had arrived in one.
pub struct TerminalNormalizer {
    state: State,
    /// The parameter/payload bytes of the sequence being collected.
    sequence: Vec<u8>,
    /// Pending text bytes not yet emitted.
    text: Vec<u8>,
    drops: DropCounts,
}

impl Default for TerminalNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalNormalizer {
    /// A fresh normalizer in the ground state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            sequence: Vec::new(),
            text: Vec::new(),
            drops: DropCounts::default(),
        }
    }

    /// Feed `bytes`, returning the chunks they completed, in order. Text
    /// is emitted at the end of every push except for an incomplete
    /// trailing UTF-8 sequence, which is held back for the next push (or
    /// [`Self::finish`]); a sequence still being collected stays pending.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<TerminalChunk> {
        let mut out = Vec::new();
        for &byte in bytes {
            self.feed(byte, &mut out);
        }
        self.flush_text(&mut out, false);
        out
    }

    /// End of input: flush every pending text byte (lossily, including a
    /// held-back incomplete character) and count a still-open sequence as
    /// `malformed`. The normalizer is back in the ground state afterward.
    pub fn finish(&mut self) -> Vec<TerminalChunk> {
        let mut out = Vec::new();
        if self.state != State::Ground {
            self.drops.malformed = self.drops.malformed.saturating_add(1);
            self.state = State::Ground;
            self.sequence.clear();
        }
        self.flush_text(&mut out, true);
        out
    }

    /// Drain the counts accumulated since the previous call.
    pub fn take_drops(&mut self) -> DropCounts {
        std::mem::take(&mut self.drops)
    }

    /// Run the state machine over one byte. A transition may hand the same
    /// byte back to the next state (an abort whose trigger byte is itself
    /// text, or an ESC that opens a new sequence), which is the loop.
    fn feed(&mut self, byte: u8, out: &mut Vec<TerminalChunk>) {
        loop {
            let reprocess = match self.state {
                State::Ground => {
                    if byte == ESC {
                        self.state = State::Escape;
                    } else {
                        self.text.push(byte);
                    }
                    false
                }
                State::Escape => match byte {
                    b'[' => self.open(State::Csi),
                    b']' => self.open(State::Osc),
                    b'P' => self.open(State::ControlString(StringKind::Dcs)),
                    b'_' => self.open(State::ControlString(StringKind::Apc)),
                    b'^' => self.open(State::ControlString(StringKind::Pm)),
                    b'X' => self.open(State::ControlString(StringKind::Sos)),
                    0x20..=0x2F => {
                        self.state = State::EscapeIntermediate;
                        false
                    }
                    0x30..=0x7E => self.drop_layout(),
                    _ => self.abort_malformed(State::Ground),
                },
                State::EscapeIntermediate => match byte {
                    0x20..=0x2F => false,
                    0x30..=0x7E => self.drop_layout(),
                    _ => self.abort_malformed(State::Ground),
                },
                State::Csi => match byte {
                    0x20..=0x3F => self.accumulate(byte),
                    0x40..=0x7E => {
                        self.classify_csi_final(byte);
                        self.close();
                        false
                    }
                    _ => self.abort_malformed(State::Ground),
                },
                State::Osc => match byte {
                    BEL => {
                        self.dispatch_osc(out);
                        false
                    }
                    ESC => {
                        self.state = State::OscEscape;
                        false
                    }
                    b'\n' => self.abort_malformed(State::Ground),
                    _ => self.accumulate(byte),
                },
                State::OscEscape => match byte {
                    b'\\' => {
                        self.dispatch_osc(out);
                        false
                    }
                    _ => self.abort_malformed(State::Escape),
                },
                State::ControlString(kind) => match byte {
                    BEL => {
                        self.dispatch_control_string(kind);
                        false
                    }
                    ESC => {
                        self.state = State::ControlStringEscape(kind);
                        false
                    }
                    b'\n' => self.abort_malformed(State::Ground),
                    _ => self.accumulate(byte),
                },
                State::ControlStringEscape(kind) => match byte {
                    b'\\' => {
                        self.dispatch_control_string(kind);
                        false
                    }
                    _ => self.abort_malformed(State::Escape),
                },
            };
            if !reprocess {
                break;
            }
        }
    }

    /// Start collecting a sequence in `state`. Never reprocesses.
    fn open(&mut self, state: State) -> bool {
        self.sequence.clear();
        self.state = state;
        false
    }

    /// Back to ground with the sequence buffer released.
    fn close(&mut self) {
        self.sequence.clear();
        self.state = State::Ground;
    }

    /// Drop a completed layout control. Never reprocesses.
    fn drop_layout(&mut self) -> bool {
        self.drops.layout = self.drops.layout.saturating_add(1);
        self.close();
        false
    }

    /// Abort the current sequence as malformed, dropping whatever payload
    /// was consumed, and hand the triggering byte to `next` -- it is text
    /// (a newline, a control byte) or the start of a new escape, never part
    /// of the aborted sequence. Always reprocesses.
    fn abort_malformed(&mut self, next: State) -> bool {
        self.drops.malformed = self.drops.malformed.saturating_add(1);
        self.sequence.clear();
        self.state = next;
        true
    }

    /// Append a payload byte, or abort on overflow (the overflowing byte
    /// then renders as text: reprocessed in the ground state).
    fn accumulate(&mut self, byte: u8) -> bool {
        if self.sequence.len() >= MAX_STRING_BYTES {
            return self.abort_malformed(State::Ground);
        }
        self.sequence.push(byte);
        false
    }

    /// Count a completed CSI by its final byte: the layout controls this
    /// slice has nothing to apply to (SGR, cursor, erase, scroll, mode
    /// set/reset including `?1049`), or unknown.
    fn classify_csi_final(&mut self, final_byte: u8) {
        match final_byte {
            b'm' | b'A'..=b'H' | b'f' | b'J' | b'K' | b'S' | b'T' | b'h' | b'l' => {
                self.drops.layout = self.drops.layout.saturating_add(1);
            }
            _ => self.drops.unknown = self.drops.unknown.saturating_add(1),
        }
    }

    /// A terminated OSC: `Ps ; Pt`. Titles and notifications become
    /// sanitized actions; hyperlinks, clipboard, and the OSC 1337 family
    /// are dropped and counted; anything else is unknown.
    fn dispatch_osc(&mut self, out: &mut Vec<TerminalChunk>) {
        let payload = std::mem::take(&mut self.sequence);
        self.state = State::Ground;
        let (number, text) = split_osc(&payload);
        match number {
            Some(0 | 2) => self.emit_action(out, TerminalAction::Title(sanitize_action_text(text))),
            Some(9) => self.emit_action(
                out,
                TerminalAction::Notification(sanitize_action_text(text)),
            ),
            Some(8) => self.drops.hyperlink = self.drops.hyperlink.saturating_add(1),
            Some(52) => self.drops.clipboard = self.drops.clipboard.saturating_add(1),
            Some(1337) => {
                self.drops.file_transfer = self.drops.file_transfer.saturating_add(1);
            }
            _ => self.drops.unknown = self.drops.unknown.saturating_add(1),
        }
    }

    /// A terminated DCS/APC/PM/SOS string: Sixel (DCS whose final byte is
    /// `q`) and Kitty graphics (APC starting `G`) are file transfer;
    /// everything else is an ignored string.
    fn dispatch_control_string(&mut self, kind: StringKind) {
        let payload = std::mem::take(&mut self.sequence);
        self.state = State::Ground;
        let is_file_transfer = match kind {
            StringKind::Dcs => dcs_final_byte(&payload) == Some(b'q'),
            StringKind::Apc => payload.first() == Some(&b'G'),
            StringKind::Pm | StringKind::Sos => false,
        };
        if is_file_transfer {
            self.drops.file_transfer = self.drops.file_transfer.saturating_add(1);
        } else {
            self.drops.string = self.drops.string.saturating_add(1);
        }
    }

    /// Emit an action after the text that preceded it, so reading order
    /// is preserved.
    fn emit_action(&mut self, out: &mut Vec<TerminalChunk>, action: TerminalAction) {
        self.flush_text(out, false);
        out.push(TerminalChunk::Action(action));
    }

    /// Decode pending text lossily and emit it. Unless `final_flush`, an
    /// incomplete trailing UTF-8 sequence is held back for the next push
    /// rather than turned into a replacement character now.
    ///
    /// 8-bit C1 controls are stripped and counted as unknown sequences in
    /// both forms they can take: an encoded code point (U+0080..=U+009F in
    /// valid UTF-8, stripped after decoding) and a raw byte (0x80..=0x9F
    /// outside any valid UTF-8 sequence, detected before decoding, where
    /// lossy decoding would otherwise have turned it into a replacement
    /// character and counted nothing). A raw introducer -- 0x9B as CSI,
    /// 0x9D as OSC, 0x90 as DCS, 0x9E as PM, 0x9F as APC, 0x9C as ST -- is
    /// deliberately not treated as the start of a sequence: the byte is
    /// dropped and whatever follows it renders as plain text, the
    /// conservative reading for a plain-text rendering. Every other
    /// invalid subsequence becomes one replacement character, exactly as
    /// `String::from_utf8_lossy` would produce.
    fn flush_text(&mut self, out: &mut Vec<TerminalChunk>, final_flush: bool) {
        if self.text.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.text);
        let held_back = if final_flush {
            0
        } else {
            incomplete_utf8_tail_len(&bytes)
        };
        let (head, tail) = bytes.split_at(bytes.len() - held_back);
        self.text = tail.to_vec();
        if head.is_empty() {
            return;
        }
        let mut cleaned = String::with_capacity(head.len());
        for chunk in head.utf8_chunks() {
            for c in chunk.valid().chars() {
                if is_c1_control(c) {
                    self.drops.unknown = self.drops.unknown.saturating_add(1);
                } else {
                    cleaned.push(c);
                }
            }
            match chunk.invalid() {
                // Only the last chunk ends without an invalid run.
                [] => {}
                [byte] if is_raw_c1_byte(*byte) => {
                    self.drops.unknown = self.drops.unknown.saturating_add(1);
                }
                _ => cleaned.push(char::REPLACEMENT_CHARACTER),
            }
        }
        if !cleaned.is_empty() {
            out.push(TerminalChunk::Text(cleaned));
        }
    }
}

/// `Ps ; Pt` -> (`Ps` as a number if it is a bare decimal, `Pt`).
fn split_osc(payload: &[u8]) -> (Option<u32>, &[u8]) {
    let (head, tail) = match payload.iter().position(|&b| b == b';') {
        Some(index) => (&payload[..index], &payload[index + 1..]),
        None => (payload, &payload[payload.len()..]),
    };
    let number = std::str::from_utf8(head)
        .ok()
        .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|text| text.parse().ok());
    (number, tail)
}

/// The first byte of a DCS payload in the final-byte range (`0x40..=0x7E`),
/// after any parameter and intermediate bytes.
fn dcs_final_byte(payload: &[u8]) -> Option<u8> {
    payload
        .iter()
        .copied()
        .find(|byte| (0x40..=0x7E).contains(byte))
}

fn is_c1_control(c: char) -> bool {
    ('\u{80}'..='\u{9f}').contains(&c)
}

/// A raw 8-bit C1 control byte: only meaningful for a byte that is not
/// part of a valid UTF-8 sequence (where 0x80..=0x9F are ordinary
/// continuation bytes, as in `C3 85` for U+00C5).
fn is_raw_c1_byte(byte: u8) -> bool {
    (0x80..=0x9F).contains(&byte)
}

/// Whether `c` is stripped from a title or notification: C0 controls, DEL,
/// C1 controls, and the zero-width and bidirectional format controls the
/// renderer's own plain-text path already treats as non-text
/// (`docs/spike-log.md` § Slice 1's `PlainTextLine`).
fn is_stripped_from_action_text(c: char) -> bool {
    c < '\u{20}'
        || c == '\u{7f}'
        || is_c1_control(c)
        || ('\u{200b}'..='\u{200f}').contains(&c)
        || ('\u{202a}'..='\u{202e}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
}

/// Sanitize a title/notification payload per RCS-001's policy table:
/// lossily decoded, control and bidirectional-override characters
/// stripped, capped at [`MAX_ACTION_TEXT_CHARS`] characters.
#[must_use]
pub fn sanitize_action_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|&c| !is_stripped_from_action_text(c))
        .take(MAX_ACTION_TEXT_CHARS)
        .collect()
}

/// How many trailing bytes of `bytes` are the incomplete prefix of a
/// multi-byte UTF-8 character (0 if the input ends on a character
/// boundary, or with bytes that can never complete into one).
fn incomplete_utf8_tail_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(3) {
        let byte = bytes[len - back];
        if byte & 0xC0 == 0x80 {
            // A continuation byte: keep looking back for its lead byte.
            continue;
        }
        let expected = match byte {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            // ASCII, or a byte that can never lead a sequence: nothing is
            // incomplete, whatever follows it decodes on its own.
            _ => return 0,
        };
        return if expected > back { back } else { 0 };
    }
    0
}

/// The shared PTY corpus: one byte fixture, emitted verbatim by
/// `omnifrons-supervisor`'s `fake-agent --pty-corpus`, normalized directly
/// by this crate's own tests, and driven through the shell's forwarder by
/// `src-tauri`'s tests -- all three asserting the same expected text,
/// actions, and drop counts below. Not feature-gated: the `fake-agent`
/// binary is built without any dev feature and must emit exactly these
/// bytes.
pub mod corpus {
    use omnifrons_domain::terminal::{DropCounts, TerminalAction};

    use super::{MAX_ACTION_TEXT_CHARS, MAX_STRING_BYTES};

    /// The OSC 52 write payload: must never appear in any output.
    pub const CLIPBOARD_SENTINEL: &str = "clipboard-sentinel-7b2d";

    /// The number of `x` bytes that follow the overflow of the
    /// unterminated OSC and therefore render as plain text.
    const OVERFLOW_TAIL: usize = 8;

    /// The corpus, as one byte vector. Ends with a bare ESC.
    #[must_use]
    pub fn bytes() -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        // Plain text.
        out.extend_from_slice(b"plain text line\n");
        // SGR colors.
        out.extend_from_slice(b"\x1b[1;31mred bold\x1b[0m normal\n");
        // Cursor moves: A B C D E F G H f.
        out.extend_from_slice(
            b"\x1b[2A\x1b[3B\x1b[4C\x1b[5D\x1b[1E\x1b[1F\x1b[3G\x1b[2;3H\x1b[2;3fcursor\n",
        );
        // Erase.
        out.extend_from_slice(b"\x1b[2J\x1b[Kerased\n");
        // Scroll.
        out.extend_from_slice(b"\x1b[2S\x1b[2Tscrolled\n");
        // Alternate screen enter/leave.
        out.extend_from_slice(b"\x1b[?1049halt screen\x1b[?1049l\n");
        // OSC 0 title with an embedded C0, a tab, and a bidi override (ST).
        out.extend_from_slice("\x1b]0;Ti\x01t\tle \u{202e}rev\x1b\\after title\n".as_bytes());
        // OSC 2 title longer than the 256-character cap (BEL).
        out.extend_from_slice(b"\x1b]2;");
        out.extend_from_slice("T".repeat(MAX_ACTION_TEXT_CHARS + 44).as_bytes());
        out.extend_from_slice(b"\x07long title\n");
        // OSC 8 hyperlink open, label, close.
        out.extend_from_slice(b"\x1b]8;;https://example.invalid\x07label\x1b]8;;\x07 link\n");
        // OSC 52 write with the sentinel payload, then an OSC 52 query.
        out.extend_from_slice(b"\x1b]52;c;");
        out.extend_from_slice(CLIPBOARD_SENTINEL.as_bytes());
        out.extend_from_slice(b"\x07\x1b]52;c;?\x07clip\n");
        // OSC 9 notification with a bidi override (ST).
        out.extend_from_slice("\x1b]9;Deploy \u{202d}done\x1b\\notified\n".as_bytes());
        // OSC 1337 File=.
        out.extend_from_slice(b"\x1b]1337;File=name=eC5wbmc=;inline=1:AAAA\x07file\n");
        // DCS Sixel.
        out.extend_from_slice(b"\x1bPq#0;2;0;0;0#0~~\x1b\\sixel\n");
        // APC Kitty graphics.
        out.extend_from_slice(b"\x1b_Gf=24,s=1,v=1;AAAA\x1b\\kitty\n");
        // Another OSC number: unknown.
        out.extend_from_slice(b"\x1b]777;notify;t;b\x07other\n");
        // A generic DCS, a PM, and an SOS: ignored strings.
        out.extend_from_slice(b"\x1bP1$r0q\x1b\\\x1b^pm\x1b\\\x1bXsos\x1b\\strings\n");
        // Single-character escapes and a charset selection.
        out.extend_from_slice(b"\x1b7\x1b8\x1bc\x1b(Bsingle\n");
        // An unterminated OSC exceeding the 4 KiB bound: aborted at the
        // overflowing byte; that byte and the rest render as text.
        out.extend_from_slice(b"\x1b]2;");
        out.extend_from_slice("x".repeat(MAX_STRING_BYTES - 2 + OVERFLOW_TAIL).as_bytes());
        out.push(b'\n');
        // An OSC with a newline inside.
        out.extend_from_slice(b"\x1b]0;broken\ntitle\n");
        // Invalid UTF-8 bytes.
        out.extend_from_slice(b"bad \xff\xfe bytes\n");
        // A CR-updated progress bar.
        out.extend_from_slice(b"progress 10%\rprogress 50%\rprogress 100%\n");
        // CRLF lines.
        out.extend_from_slice(b"crlf one\r\ncrlf two\r\n");
        // Multi-byte text.
        out.extend_from_slice("h\u{e9}llo w\u{f6}rld \u{20ac}\n".as_bytes());
        // A bare ESC at the very end.
        out.push(0x1B);
        out
    }

    /// The exact text the corpus normalizes to when fed directly (CRLF
    /// preserved; the supervisor's own line framing strips a `\r` before
    /// `\n`, so a consumer feeding framed lines expects `\r\n` as `\n`).
    #[must_use]
    pub fn expected_text() -> String {
        let mut out = String::new();
        out.push_str("plain text line\n");
        out.push_str("red bold normal\n");
        out.push_str("cursor\n");
        out.push_str("erased\n");
        out.push_str("scrolled\n");
        out.push_str("alt screen\n");
        out.push_str("after title\n");
        out.push_str("long title\n");
        out.push_str("label link\n");
        out.push_str("clip\n");
        out.push_str("notified\n");
        out.push_str("file\n");
        out.push_str("sixel\n");
        out.push_str("kitty\n");
        out.push_str("other\n");
        out.push_str("strings\n");
        out.push_str("single\n");
        out.push_str(&"x".repeat(OVERFLOW_TAIL));
        out.push('\n');
        out.push_str("\ntitle\n");
        out.push_str("bad \u{FFFD}\u{FFFD} bytes\n");
        out.push_str("progress 10%\rprogress 50%\rprogress 100%\n");
        out.push_str("crlf one\r\ncrlf two\r\n");
        out.push_str("h\u{e9}llo w\u{f6}rld \u{20ac}\n");
        out
    }

    /// The typed actions the corpus produces, in order.
    #[must_use]
    pub fn expected_actions() -> Vec<TerminalAction> {
        vec![
            TerminalAction::Title("Title rev".to_string()),
            TerminalAction::Title("T".repeat(MAX_ACTION_TEXT_CHARS)),
            TerminalAction::Notification("Deploy done".to_string()),
        ]
    }

    /// The drop counts the corpus produces, whether fed directly and
    /// finished (the bare trailing ESC is malformed at EOF) or framed with
    /// a newline after it (the ESC is then followed by an unexpected byte:
    /// malformed all the same).
    #[must_use]
    pub fn expected_drops() -> DropCounts {
        DropCounts {
            layout: 2 + 9 + 2 + 2 + 2 + 4,
            hyperlink: 2,
            clipboard: 2,
            file_transfer: 3,
            string: 3,
            unknown: 1,
            malformed: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{incomplete_utf8_tail_len, sanitize_action_text, split_osc};

    #[test]
    fn incomplete_tail_is_detected_for_two_three_and_four_byte_leads() {
        assert_eq!(incomplete_utf8_tail_len(b"abc"), 0);
        assert_eq!(incomplete_utf8_tail_len(b"a\xc3"), 1);
        assert_eq!(incomplete_utf8_tail_len(b"a\xe2\x82"), 2);
        assert_eq!(incomplete_utf8_tail_len(b"a\xf0\x9f\x98"), 3);
        assert_eq!(incomplete_utf8_tail_len("a€".as_bytes()), 0);
        assert_eq!(
            incomplete_utf8_tail_len(b"\x80\x80\x80"),
            0,
            "bare continuation bytes can never complete"
        );
        assert_eq!(incomplete_utf8_tail_len(b""), 0);
    }

    #[test]
    fn split_osc_parses_a_bare_decimal_ps_only() {
        assert_eq!(split_osc(b"0;title"), (Some(0), &b"title"[..]));
        assert_eq!(split_osc(b"1337;File=x"), (Some(1337), &b"File=x"[..]));
        assert_eq!(split_osc(b"2"), (Some(2), &b""[..]));
        assert_eq!(split_osc(b";x"), (None, &b"x"[..]));
        assert_eq!(split_osc(b"a1;x"), (None, &b"x"[..]));
    }

    #[test]
    fn sanitize_strips_controls_and_bidi_and_caps_at_256_characters() {
        assert_eq!(
            sanitize_action_text("a\x01b\u{85}c\u{202e}d\u{200b}e\u{2066}f\x7f".as_bytes()),
            "abcdef"
        );
        assert_eq!(
            sanitize_action_text("\u{e9}".repeat(300).as_bytes())
                .chars()
                .count(),
            256,
            "the cap counts characters, not bytes"
        );
    }
}
