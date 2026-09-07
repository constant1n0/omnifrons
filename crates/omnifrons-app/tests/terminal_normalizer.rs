//! `TerminalNormalizer` (spike slice 4): a pure, streaming state machine
//! turning pseudo-terminal bytes into plain-text chunks plus the two
//! sanitized typed actions RCS-001's terminal policy admits (title,
//! notification), dropping and counting every other recognized family
//! (`docs/renderer-content-security.md` § Terminal control and OSC policy;
//! `docs/spike-log.md` § Slice 4).
//!
//! Every test feeds bytes through `push` (state kept across pushes) and,
//! where the input ends mid-sequence, `finish`; drop counts are drained via
//! `take_drops`.

use omnifrons_app::terminal_normalizer::{
    MAX_ACTION_TEXT_CHARS, MAX_STRING_BYTES, TerminalChunk, TerminalNormalizer, corpus,
};
use omnifrons_domain::terminal::{DropCounts, TerminalAction};

/// Every `Text` chunk's content, concatenated in order.
fn texts(chunks: &[TerminalChunk]) -> String {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            TerminalChunk::Text(text) => Some(text.as_str()),
            TerminalChunk::Action(_) => None,
        })
        .collect()
}

/// Every `Action` chunk, in order.
fn actions(chunks: &[TerminalChunk]) -> Vec<TerminalAction> {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            TerminalChunk::Action(action) => Some(action.clone()),
            TerminalChunk::Text(_) => None,
        })
        .collect()
}

/// Feed `bytes` in one push, then `finish`, returning every chunk and the
/// drained drop counts.
fn normalize(bytes: &[u8]) -> (Vec<TerminalChunk>, DropCounts) {
    let mut normalizer = TerminalNormalizer::new();
    let mut chunks = normalizer.push(bytes);
    chunks.extend(normalizer.finish());
    (chunks, normalizer.take_drops())
}

#[test]
fn plain_text_passes_through_unchanged_with_zero_drops() {
    let (chunks, drops) = normalize(b"hello world\n");
    assert_eq!(
        chunks,
        vec![TerminalChunk::Text("hello world\n".to_string())]
    );
    assert!(drops.is_zero(), "got {drops:?}");
}

/// RCS-001-R18: sanitization never removes textual content outside a
/// recognized sequence -- C0 controls other than ESC (tab, BEL, a bare
/// CR) are text to this normalizer (the renderer strips them), so an input
/// with no escape sequence at all comes out byte-identical.
#[test]
fn text_with_no_escape_sequence_is_returned_verbatim_including_bare_c0() {
    let input = "abc\tdef\x07ghi\rjkl\r\nmno\n";
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(texts(&chunks), input);
    assert!(drops.is_zero(), "got {drops:?}");
}

#[test]
fn sgr_cursor_erase_scroll_and_mode_sequences_are_dropped_and_counted_as_layout() {
    let input = b"\x1b[1;31mred\x1b[0m \x1b[2A\x1b[3B\x1b[4C\x1b[5D\x1b[1E\x1b[1F\x1b[3G\x1b[2;3H\x1b[2;3f\x1b[2J\x1b[K\x1b[2S\x1b[2T\x1b[?1049halt\x1b[?1049l\n";
    let (chunks, drops) = normalize(input);
    assert_eq!(texts(&chunks), "red alt\n");
    assert!(actions(&chunks).is_empty());
    assert_eq!(
        drops,
        DropCounts {
            layout: 17,
            ..DropCounts::default()
        }
    );
}

#[test]
fn an_unknown_csi_final_byte_is_dropped_and_counted_as_unknown() {
    let (chunks, drops) = normalize(b"\x1b[5zafter\n");
    assert_eq!(texts(&chunks), "after\n");
    assert_eq!(
        drops,
        DropCounts {
            unknown: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn osc_0_and_2_titles_become_sanitized_title_actions_with_either_terminator() {
    // OSC 0 terminated by ST, OSC 2 terminated by BEL. The title carries a
    // C0 control, a tab, an escaped C1 control (U+0085 as UTF-8), and a
    // right-to-left override: all stripped; the visible text stays.
    let input = "\x1b]0;Ti\x01t\tle \u{85}\u{202e}rev\x1b\\mid\x1b]2;second\x07end\n";
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(
        chunks,
        vec![
            TerminalChunk::Action(TerminalAction::Title("Title rev".to_string())),
            TerminalChunk::Text("mid".to_string()),
            TerminalChunk::Action(TerminalAction::Title("second".to_string())),
            TerminalChunk::Text("end\n".to_string()),
        ]
    );
    assert!(
        drops.is_zero(),
        "a recognized title is not a drop, got {drops:?}"
    );
}

#[test]
fn a_title_longer_than_256_characters_is_capped() {
    let long = "T".repeat(MAX_ACTION_TEXT_CHARS + 44);
    let input = format!("\x1b]2;{long}\x07");
    let (chunks, _) = normalize(input.as_bytes());
    assert_eq!(
        actions(&chunks),
        vec![TerminalAction::Title("T".repeat(MAX_ACTION_TEXT_CHARS))]
    );
}

#[test]
fn osc_9_becomes_a_sanitized_notification_action() {
    let input = "\x1b]9;Deploy \u{202d}done\x1b\\notified\n";
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(
        chunks,
        vec![
            TerminalChunk::Action(TerminalAction::Notification("Deploy done".to_string())),
            TerminalChunk::Text("notified\n".to_string()),
        ]
    );
    assert!(drops.is_zero(), "got {drops:?}");
}

#[test]
fn osc_8_hyperlinks_are_dropped_and_counted_while_the_label_stays_text() {
    let input = b"\x1b]8;;https://example.invalid\x07label\x1b]8;;\x07 link\n";
    let (chunks, drops) = normalize(input);
    assert_eq!(texts(&chunks), "label link\n");
    assert!(actions(&chunks).is_empty());
    assert_eq!(
        drops,
        DropCounts {
            hyperlink: 2,
            ..DropCounts::default()
        }
    );
    assert!(
        !texts(&chunks).contains("example.invalid"),
        "the target never renders as text"
    );
}

#[test]
fn osc_52_write_and_query_are_dropped_and_the_payload_never_appears() {
    let sentinel = "clipboard-sentinel-7b2d";
    let input = format!("\x1b]52;c;{sentinel}\x07\x1b]52;c;?\x07clip\n");
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(
        chunks,
        vec![TerminalChunk::Text("clip\n".to_string())],
        "no chunk of any kind carries the clipboard payload"
    );
    assert!(!format!("{chunks:?}").contains(sentinel));
    assert_eq!(
        drops,
        DropCounts {
            clipboard: 2,
            ..DropCounts::default()
        }
    );
}

#[test]
fn file_transfer_and_image_protocols_are_dropped_and_counted() {
    let input = b"\x1b]1337;File=name=eC5wbmc=;inline=1:AAAA\x07file \x1bPq#0;2;0;0;0#0~~\x1b\\sixel \x1b_Gf=24,s=1,v=1;AAAA\x1b\\kitty\n";
    let (chunks, drops) = normalize(input);
    assert_eq!(texts(&chunks), "file sixel kitty\n");
    assert_eq!(
        drops,
        DropCounts {
            file_transfer: 3,
            ..DropCounts::default()
        }
    );
}

#[test]
fn other_osc_numbers_are_unknown_and_other_control_strings_are_counted_as_string() {
    let input = b"\x1b]777;notify;t;b\x07other \x1bP1$r0q\x1b\\\x1b^pm\x07\x1bXsos\x1b\\strings\n";
    let (chunks, drops) = normalize(input);
    assert_eq!(texts(&chunks), "other strings\n");
    assert_eq!(
        drops,
        DropCounts {
            unknown: 1,
            string: 3,
            ..DropCounts::default()
        }
    );
}

#[test]
fn single_character_escapes_and_charset_selection_are_layout() {
    let (chunks, drops) = normalize(b"\x1b7\x1b8\x1bc\x1b(Bsingle\n");
    assert_eq!(texts(&chunks), "single\n");
    assert_eq!(
        drops,
        DropCounts {
            layout: 4,
            ..DropCounts::default()
        }
    );
}

#[test]
fn a_bare_esc_at_eof_is_malformed() {
    let mut normalizer = TerminalNormalizer::new();
    let chunks = normalizer.push(b"tail\x1b");
    assert_eq!(chunks, vec![TerminalChunk::Text("tail".to_string())]);
    assert!(
        normalizer.take_drops().is_zero(),
        "nothing is malformed until EOF proves the ESC was bare"
    );
    assert!(normalizer.finish().is_empty());
    assert_eq!(
        normalizer.take_drops(),
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn an_esc_followed_by_an_unexpected_byte_is_malformed_and_that_byte_is_text() {
    let (chunks, drops) = normalize(b"a\x1b\nnext\n");
    assert_eq!(texts(&chunks), "a\nnext\n");
    assert_eq!(
        drops,
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn a_newline_inside_a_string_aborts_it_as_malformed_and_later_bytes_are_text() {
    let (chunks, drops) = normalize(b"\x1b]0;broken\ntitle\n");
    assert_eq!(texts(&chunks), "\ntitle\n");
    assert!(
        actions(&chunks).is_empty(),
        "the aborted title is never emitted"
    );
    assert!(
        !texts(&chunks).contains("broken"),
        "the consumed payload is dropped"
    );
    assert_eq!(
        drops,
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
}

/// RCS-001 D11 (spike default 4 KiB): a string exceeding the bound is
/// aborted, counted malformed, and every byte from the overflowing one on
/// renders as plain text.
#[test]
fn a_string_exceeding_the_bound_is_aborted_and_the_rest_renders_as_text() {
    let payload_prefix = "2;";
    let within = "x".repeat(MAX_STRING_BYTES - payload_prefix.len());
    let input = format!("\x1b]{payload_prefix}{within}yyyy\n");
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(texts(&chunks), "yyyy\n");
    assert!(actions(&chunks).is_empty());
    assert_eq!(
        drops,
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn a_string_of_exactly_the_bound_is_still_recognized() {
    let payload_prefix = "2;";
    let within = "x".repeat(MAX_STRING_BYTES - payload_prefix.len());
    let input = format!("\x1b]{payload_prefix}{within}\x07");
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(
        actions(&chunks),
        vec![TerminalAction::Title("x".repeat(MAX_ACTION_TEXT_CHARS))]
    );
    assert!(drops.is_zero(), "got {drops:?}");
}

/// A CSI parameter/intermediate run is bounded by the same 4 KiB as a
/// string: exactly at the bound the sequence is still recognized (and
/// dropped as layout, here by its `m` final).
#[test]
fn a_csi_parameter_run_of_exactly_the_bound_is_still_recognized() {
    let params = "1;".repeat(MAX_STRING_BYTES / 2);
    assert_eq!(params.len(), MAX_STRING_BYTES);
    let input = format!("\x1b[{params}mafter\n");
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(texts(&chunks), "after\n");
    assert_eq!(
        drops,
        DropCounts {
            layout: 1,
            ..DropCounts::default()
        }
    );
}

/// One byte past the bound the CSI is aborted as malformed: the
/// overflowing byte and everything after it, the would-be final included,
/// render as plain text.
#[test]
fn a_csi_parameter_run_exceeding_the_bound_is_aborted_and_the_rest_renders_as_text() {
    let params = "1;".repeat(MAX_STRING_BYTES / 2);
    let input = format!("\x1b[{params}1mafter\n");
    let (chunks, drops) = normalize(input.as_bytes());
    assert_eq!(texts(&chunks), "1mafter\n");
    assert_eq!(
        drops,
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn sequences_split_across_pushes_are_still_recognized() {
    let mut normalizer = TerminalNormalizer::new();
    let mut chunks = normalizer.push(b"\x1b[3");
    chunks.extend(normalizer.push(b"1mhi \x1b]0;ti"));
    chunks.extend(normalizer.push(b"tle\x07 done\n"));
    chunks.extend(normalizer.finish());
    assert_eq!(
        chunks,
        vec![
            TerminalChunk::Text("hi ".to_string()),
            TerminalChunk::Action(TerminalAction::Title("title".to_string())),
            TerminalChunk::Text(" done\n".to_string()),
        ]
    );
    assert_eq!(
        normalizer.take_drops(),
        DropCounts {
            layout: 1,
            ..DropCounts::default()
        }
    );
}

#[test]
fn multi_byte_characters_split_across_pushes_are_rejoined() {
    let mut normalizer = TerminalNormalizer::new();
    let euro = "€".as_bytes();
    let mut chunks = normalizer.push(&euro[..2]);
    assert!(
        chunks.is_empty(),
        "an incomplete character is held back, never emitted as U+FFFD"
    );
    chunks.extend(normalizer.push(&euro[2..]));
    chunks.extend(normalizer.push("\n".as_bytes()));
    chunks.extend(normalizer.finish());
    assert_eq!(texts(&chunks), "€\n");
    assert!(!texts(&chunks).contains('\u{FFFD}'));
}

#[test]
fn finish_flushes_a_held_back_incomplete_character_lossily() {
    let mut normalizer = TerminalNormalizer::new();
    assert!(normalizer.push(&"€".as_bytes()[..2]).is_empty());
    assert_eq!(
        normalizer.finish(),
        vec![TerminalChunk::Text("\u{FFFD}".to_string())]
    );
}

#[test]
fn invalid_utf8_is_lossily_decoded_not_dropped() {
    let (chunks, drops) = normalize(b"bad \xff\xfe bytes\n");
    assert_eq!(texts(&chunks), "bad \u{FFFD}\u{FFFD} bytes\n");
    assert!(drops.is_zero(), "got {drops:?}");
}

#[test]
fn c1_control_code_points_in_text_are_stripped_and_counted_as_unknown() {
    let (chunks, drops) = normalize("a\u{85}b\u{9b}c\n".as_bytes());
    assert_eq!(texts(&chunks), "abc\n");
    assert_eq!(
        drops,
        DropCounts {
            unknown: 2,
            ..DropCounts::default()
        }
    );
}

/// A raw 8-bit C1 byte (0x80..=0x9F outside any valid UTF-8 sequence) is
/// dropped and counted `unknown` at the byte level: never lossily decoded
/// into a replacement character, never emitted as text.
#[test]
fn a_raw_8_bit_c1_byte_is_dropped_and_counted_as_unknown() {
    let (chunks, drops) = normalize(b"a\x85b\n");
    assert_eq!(texts(&chunks), "ab\n");
    assert!(
        !texts(&chunks).contains('\u{FFFD}'),
        "a raw C1 byte is dropped, not replaced"
    );
    assert_eq!(
        drops,
        DropCounts {
            unknown: 1,
            ..DropCounts::default()
        }
    );
}

/// An 8-bit introducer (here 0x9B, the 8-bit CSI) is never interpreted as
/// the start of a sequence: the byte itself is dropped and counted, and
/// every byte after it renders as plain text -- conservative, no
/// interpretation in plain-text mode.
#[test]
fn a_raw_8_bit_csi_introducer_is_dropped_and_the_bytes_after_it_are_text() {
    let (chunks, drops) = normalize(b"x\x9b31my\n");
    assert_eq!(texts(&chunks), "x31my\n");
    assert!(actions(&chunks).is_empty());
    assert_eq!(
        drops,
        DropCounts {
            unknown: 1,
            ..DropCounts::default()
        }
    );
}

/// A continuation byte in 0x80..=0x9F inside a valid multi-byte character
/// is not a C1 control: U+00C5 is `C3 85`, and it passes untouched.
#[test]
fn a_valid_two_byte_character_whose_continuation_byte_is_in_the_c1_range_is_untouched() {
    let (chunks, drops) = normalize("\u{c5}\n".as_bytes());
    assert_eq!(texts(&chunks), "\u{c5}\n");
    assert!(drops.is_zero(), "got {drops:?}");
}

/// A raw C1 byte and the valid character after it, split across pushes:
/// the C1 byte is dropped and counted, the character re-joins intact --
/// also when the raw byte arrives together with the next character's lead
/// byte and the continuation byte comes in the following push.
#[test]
fn a_raw_c1_byte_split_from_a_following_valid_character_across_pushes() {
    let one_unknown = DropCounts {
        unknown: 1,
        ..DropCounts::default()
    };

    // The raw byte is processed in the push that carries it -- emitted
    // text before it, the drop counted right away -- not held back as if
    // it could still complete into a character.
    let mut normalizer = TerminalNormalizer::new();
    let first = normalizer.push(b"a\x85");
    assert_eq!(first, vec![TerminalChunk::Text("a".to_string())]);
    assert_eq!(
        normalizer.take_drops(),
        one_unknown,
        "the raw C1 byte is dropped and counted in its own push"
    );
    let mut chunks = first;
    chunks.extend(normalizer.push("\u{e9}\n".as_bytes()));
    chunks.extend(normalizer.finish());
    assert_eq!(texts(&chunks), "a\u{e9}\n");
    assert!(
        normalizer.take_drops().is_zero(),
        "the later pushes drop nothing more"
    );

    // Only the next character's lead byte is held back, never the raw C1
    // byte before it.
    let mut normalizer = TerminalNormalizer::new();
    let e_acute = "\u{e9}".as_bytes();
    let first = normalizer.push(&[b'b', 0x85, e_acute[0]]);
    assert_eq!(first, vec![TerminalChunk::Text("b".to_string())]);
    assert_eq!(
        normalizer.take_drops(),
        one_unknown,
        "the raw C1 byte is dropped and counted in its own push"
    );
    let mut chunks = first;
    chunks.extend(normalizer.push(&e_acute[1..]));
    chunks.extend(normalizer.push(b"\n"));
    chunks.extend(normalizer.finish());
    assert_eq!(texts(&chunks), "b\u{e9}\n");
    assert!(
        normalizer.take_drops().is_zero(),
        "the later pushes drop nothing more"
    );
}

/// An invalid multi-byte run is one replacement character and never a
/// drop, even when its continuation bytes fall in the C1 range: `F0 9F 98`
/// (a four-byte lead with two valid continuations, cut short by `FF`)
/// collapses to exactly one U+FFFD with no count, and the lone `FF` after
/// it -- an isolated invalid byte outside 0x80..=0x9F -- is one more
/// U+FFFD, also uncounted. Only an isolated byte in 0x80..=0x9F is a raw
/// C1 control.
#[test]
fn an_invalid_multi_byte_run_containing_c1_range_bytes_is_one_replacement_and_never_counted() {
    let (chunks, drops) = normalize(b"a\xf0\x9f\x98\xffb\n");
    assert_eq!(texts(&chunks), "a\u{FFFD}\u{FFFD}b\n");
    assert!(
        drops.is_zero(),
        "neither the truncated run nor the lone FF is a raw C1 control, got {drops:?}"
    );

    // The same run at the very end of the input, flushed by `finish`.
    let (chunks, drops) = normalize(b"a\xf0\x9f\x98\xff");
    assert_eq!(texts(&chunks), "a\u{FFFD}\u{FFFD}");
    assert!(drops.is_zero(), "got {drops:?}");
}

/// A four-byte character (U+1F600, `F0 9F 98 80`) split at each of its
/// three interior byte boundaries across two pushes re-joins intact.
#[test]
fn a_four_byte_character_split_at_each_interior_boundary_is_rejoined() {
    let smile = "\u{1F600}".as_bytes();
    assert_eq!(smile.len(), 4);
    for split in 1..4 {
        let mut normalizer = TerminalNormalizer::new();
        let mut chunks = normalizer.push(&smile[..split]);
        assert!(
            chunks.is_empty(),
            "the incomplete head is held back at split {split}, got {chunks:?}"
        );
        chunks.extend(normalizer.push(&smile[split..]));
        chunks.extend(normalizer.push(b"\n"));
        chunks.extend(normalizer.finish());
        assert_eq!(texts(&chunks), "\u{1F600}\n", "at split {split}");
        assert!(
            normalizer.take_drops().is_zero(),
            "nothing is dropped at split {split}"
        );
    }
}

/// `finish()` after a partial four-byte lead flushes it lossily, as one
/// replacement character.
#[test]
fn finish_flushes_a_partial_four_byte_lead_lossily() {
    let smile = "\u{1F600}".as_bytes();
    let mut normalizer = TerminalNormalizer::new();
    assert!(normalizer.push(&smile[..3]).is_empty());
    assert_eq!(
        normalizer.finish(),
        vec![TerminalChunk::Text("\u{FFFD}".to_string())]
    );
    assert!(normalizer.take_drops().is_zero());
}

/// A second `finish()` is a no-op: it returns nothing and counts nothing,
/// since the first already put the normalizer back in the ground state.
#[test]
fn finish_twice_returns_empty_and_adds_no_malformed() {
    let mut normalizer = TerminalNormalizer::new();
    let _ = normalizer.push(b"open\x1b]0;unterminated");
    let _ = normalizer.finish();
    assert_eq!(
        normalizer.take_drops(),
        DropCounts {
            malformed: 1,
            ..DropCounts::default()
        }
    );
    assert!(normalizer.finish().is_empty());
    assert!(
        normalizer.take_drops().is_zero(),
        "a second finish must add no malformed count"
    );
}

#[test]
fn take_drops_drains_and_resets_the_counts() {
    let mut normalizer = TerminalNormalizer::new();
    let _ = normalizer.push(b"\x1b[m\x1b]8;;x\x07");
    assert_eq!(
        normalizer.take_drops(),
        DropCounts {
            layout: 1,
            hyperlink: 1,
            ..DropCounts::default()
        }
    );
    assert!(normalizer.take_drops().is_zero());
    let _ = normalizer.push(b"\x1b[m");
    assert_eq!(
        normalizer.take_drops(),
        DropCounts {
            layout: 1,
            ..DropCounts::default()
        }
    );
}

/// The invariant every other test relies on, over the whole corpus: no
/// chunk -- text or action -- ever carries an ESC byte, a C1 control code
/// point, or the bytes of a dropped sequence.
#[test]
fn no_chunk_ever_contains_esc_or_c1_controls_or_a_dropped_sequence() {
    let (chunks, _) = normalize(&corpus::bytes());
    for chunk in &chunks {
        let content = match chunk {
            TerminalChunk::Text(text)
            | TerminalChunk::Action(
                TerminalAction::Title(text) | TerminalAction::Notification(text),
            ) => text.as_str(),
        };
        assert!(!content.contains('\x1b'), "ESC leaked into {chunk:?}");
        assert!(
            !content.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)),
            "a C1 control leaked into {chunk:?}"
        );
        assert!(!content.contains(corpus::CLIPBOARD_SENTINEL));
        assert!(!content.contains("example.invalid"));
        assert!(!content.contains("File=name"));
    }
}

/// The shared fixture (`corpus`, also emitted verbatim by the supervisor's
/// `fake-agent --pty-corpus`) normalizes to exactly the expected text,
/// actions, and drop counts.
#[test]
fn corpus_normalizes_to_the_expected_text_actions_and_drops() {
    let (chunks, drops) = normalize(&corpus::bytes());
    assert_eq!(texts(&chunks), corpus::expected_text());
    assert_eq!(actions(&chunks), corpus::expected_actions());
    assert_eq!(drops, corpus::expected_drops());
    assert!(!format!("{chunks:?}").contains(corpus::CLIPBOARD_SENTINEL));
}

/// The corpus pushed one byte at a time yields the same text, actions, and
/// drops as one push: the state machine is genuinely streaming.
#[test]
fn corpus_pushed_byte_by_byte_matches_a_single_push() {
    let bytes = corpus::bytes();
    let mut normalizer = TerminalNormalizer::new();
    let mut chunks = Vec::new();
    for byte in &bytes {
        chunks.extend(normalizer.push(std::slice::from_ref(byte)));
    }
    chunks.extend(normalizer.finish());
    assert_eq!(texts(&chunks), corpus::expected_text());
    assert_eq!(actions(&chunks), corpus::expected_actions());
    assert_eq!(normalizer.take_drops(), corpus::expected_drops());
}
