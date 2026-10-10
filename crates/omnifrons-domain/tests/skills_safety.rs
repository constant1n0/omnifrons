//! Domain tests for the `@`-import safety rule (ADR-0005 "Agent identity
//! and portable definition" decision 2, sub-slice 1c):
//! [`omnifrons_domain::skills_safety::unsafe_at_tokens`] decides whether an
//! `@` in canonical text or generated skill metadata could be loaded as a
//! host import. Translated from the `base_omnifrons` prototype's
//! `test_unsafe_at_tokens_follow_the_conservative_rule`
//! (`tests/test_portable_agent.py`), via its `AT_REFUSED` and `AT_ALLOWED`
//! fixtures.

use omnifrons_domain::skills_safety::unsafe_at_tokens;

/// Every case in the prototype's `AT_REFUSED`, minus "an undecodable
/// byte": that case's input is a lone surrogate produced by Python's
/// `surrogateescape` codec (`"jos\udce9@ejemplo.es"`), which has no
/// equivalent `&str` value in Rust -- a `str` can never hold an unpaired
/// surrogate. The byte-level entry point this rule actually runs behind
/// in Omnifrons is `read_skill_metadata`, whose own tests cover that
/// case's intent: a byte that is not valid UTF-8 is still judged
/// conservatively, just shown as the UTF-8 replacement character rather
/// than the prototype's `\xNN`.
#[test]
fn unsafe_at_tokens_refuses_every_conservative_case() {
    let cases: [(&str, Vec<&str>); 20] = [
        ("Run `make`@secret.md first.", vec!["`make`@secret.md"]),
        ("@README", vec!["@README"]),
        ("**x**@y", vec!["**x**@y"]),
        ("[d](u)@x.md", vec!["[d](u)@x.md"]),
        ("Brief *pro*@secret.md", vec!["*pro*@secret.md"]),
        ("Name it snake_@case.md here.", vec!["snake_@case.md"]),
        ("Escape it as \\@x.md here.", vec!["\\@x.md"]),
        ("See @notes for context.", vec!["@notes"]),
        ("x\u{feff}@y.md", vec!["@y.md"]),
        ("@\u{1f}x.md", vec!["@\u{1f}x.md"]),
        ("Ask me (@", vec!["(@"]),
        ("@a and `b`@c", vec!["@a", "`b`@c"]),
        ("Run \\-@secret.md first.", vec!["\\-@secret.md"]),
        ("x\\.@y.md", vec!["x\\.@y.md"]),
        ("x\\+@y.md", vec!["x\\+@y.md"]),
        ("x\\%@y.md", vec!["x\\%@y.md"]),
        ("x\\\\\\-@y.md", vec!["x\\\\\\-@y.md"]),
        ("\\-\u{0301}@y.md", vec!["\\-\u{0301}@y.md"]),
        ("\u{0301}@y.md", vec!["\u{0301}@y.md"]),
        ("a \u{0301}@y.md", vec!["\u{0301}@y.md"]),
    ];
    for (text, expected) in cases {
        assert_eq!(unsafe_at_tokens(text, true), expected, "case {text:?}");
    }
}

/// Every case in the prototype's `AT_ALLOWED`: an `@` right after a
/// letter or digit (an NFD combining mark included, VER-H-004), after
/// `.`, `%`, `+` or `-` with no odd backslash run before it, or followed
/// by whitespace, is never reported.
#[test]
fn unsafe_at_tokens_allows_every_conservative_case() {
    let allowed = [
        "Reply from ops@example.com only.",
        "Write to jos\u{e9}@ejemplo.es in Spanish.",
        "Copy a.b+c@x.io on every reply.",
        "Accept a.@x.io, b%@x.io, c+@x.io and d-@x.io as written.",
        "Sell 3 @ 5 EUR each.",
        "Meet @\u{a0}noon, or @\u{feff}noon.",
        "",
        "Write to jose\u{301}@ejemplo.es in Spanish.",
        "Mail a\u{301}\u{302}@x.io too.",
        "Keep x\\\\-@y.md as written.",
        "Keep a\\b@c.io as written.",
    ];
    for text in allowed {
        let found: Vec<String> = unsafe_at_tokens(text, true);
        assert!(found.is_empty(), "{text:?} -> {found:?}");
    }
}

/// `text_follows` says whether more text follows where `text` is
/// written: an `@` at the very end of a whole file line (`text_follows:
/// false`) is followed by the line break, so no path can start there; the
/// same `@` inside a rendered cell, where text always follows, stays
/// unsafe.
#[test]
fn unsafe_at_tokens_respects_text_follows() {
    let none: Vec<String> = Vec::new();
    assert_eq!(unsafe_at_tokens("Ask me (@", false), none);
    assert_eq!(unsafe_at_tokens("Ask me (@x", false), vec!["(@x"]);
}

/// A long word around an unsafe `@` is shortened to 30 characters before
/// it and 40 after, each side marked with `...` since more was cut.
#[test]
fn unsafe_at_tokens_shortens_long_words() {
    let long = format!("{}_@{}", "a".repeat(35), "b".repeat(45));
    let expected = format!("...{}_@{}...", "a".repeat(29), "b".repeat(39));
    assert_eq!(unsafe_at_tokens(&long, true), vec![expected]);
}

use omnifrons_domain::skills_safety::{
    SkillMetadata, SkillMetadataError, SkillSafetyRefusal, read_skill_metadata,
};

/// The house convention this crate's other reason-token enums use
/// ([`omnifrons_domain::guidance::ManagedFileKind`]): a stable
/// kebab-case token, a `parse` that round-trips it, and an exhaustive
/// `ALL`. The prototype raises the same `UnsafeSkillMetadata` exception,
/// and reports it under the one token `unsafe_skill_metadata`, for both
/// a lone-surrogate refusal and a row-fields refusal.
#[test]
fn skill_safety_refusal_follows_the_house_reason_token_convention() {
    assert_eq!(
        SkillSafetyRefusal::UnsafeSkillMetadata.as_str(),
        "unsafe-skill-metadata"
    );
    for refusal in SkillSafetyRefusal::ALL {
        assert_eq!(SkillSafetyRefusal::parse(refusal.as_str()), Some(refusal));
    }
    assert_eq!(SkillSafetyRefusal::parse("not-a-real-token"), None);
}

/// A `SKILL.md` whose front matter quotes both values as JSON-escaped
/// double-quoted strings (the prototype's `at_skill`).
fn at_skill(name: &str, description: &str) -> Vec<u8> {
    format!(
        "---\nname: {}\ndescription: {}\n---\n",
        json_quote(name),
        json_quote(description)
    )
    .into_bytes()
}

fn json_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Translated from the prototype's `read_skill`: the row's `name` and
/// `description` come from front matter, verbatim.
#[test]
fn read_skill_metadata_reads_name_and_description_from_front_matter() {
    let bytes = at_skill("alpha", "Alpha tasks.");
    let metadata = read_skill_metadata("skills/alpha/SKILL.md", &bytes).unwrap();
    assert_eq!(
        metadata,
        SkillMetadata {
            name: "alpha".to_string(),
            description: "Alpha tasks.".to_string(),
            path: "skills/alpha/SKILL.md".to_string(),
        }
    );
}

/// Without `name:`, the row shows the folder name (the prototype's own
/// `skills/@scope/SKILL.md` fixture).
#[test]
fn read_skill_metadata_falls_back_to_the_folder_name() {
    let bytes = b"---\ndescription: Scoped tasks.\n---\n".to_vec();
    let metadata = read_skill_metadata("skills/@scope/SKILL.md", &bytes).unwrap();
    assert_eq!(metadata.name, "@scope");
    assert_eq!(metadata.description, "Scoped tasks.");
}

/// Without `name:` and without a parent folder, the row falls back all
/// the way to the whole path (the prototype's `name or dirname(basename)
/// or relative`, chained).
#[test]
fn read_skill_metadata_falls_back_to_the_whole_path_without_a_folder() {
    let bytes = b"---\ndescription: Root tasks.\n---\n".to_vec();
    let metadata = read_skill_metadata("SKILL.md", &bytes).unwrap();
    assert_eq!(metadata.name, "SKILL.md");
}

/// A raw byte that is not valid UTF-8 in a plain (unquoted) scalar is
/// carried lossily (VER-H-001): `read_skill_metadata` never refuses or
/// panics on it, unlike a `\u` escape that decodes to a lone surrogate.
#[test]
fn read_skill_metadata_keeps_a_plain_scalar_byte_that_is_not_utf8_lossy() {
    let mut bytes = b"---\nname: es\ndescription: Escribe a jos".to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b"ol@ejemplo.es\n---\n");
    let metadata = read_skill_metadata("skills/es/SKILL.md", &bytes).unwrap();
    assert!(metadata.description.contains('\u{FFFD}'), "{metadata:?}");
    assert!(
        metadata.description.ends_with("ol@ejemplo.es"),
        "{metadata:?}"
    );
}

/// A complete, correctly-ordered surrogate pair decodes to the one
/// character it encodes (the prototype's `ESCAPED_PAIR`/`DECODED_PAIR`).
/// Built with [`String::push_str`] rather than typed directly: a source
/// literal spelling out this escape next to its own decoded character
/// is exactly the ambiguity this test exists to pin down.
#[test]
fn read_skill_metadata_decodes_a_complete_surrogate_pair() {
    let backslash = '\\';
    let mut description_escape = String::new();
    description_escape.push('x');
    description_escape.push(backslash);
    description_escape.push_str("ud83d");
    description_escape.push(backslash);
    description_escape.push_str("ude00y");
    let text = format!("---\nname: \"p\"\ndescription: \"{description_escape}\"\n---\n");
    let metadata = read_skill_metadata("skills/p/SKILL.md", text.as_bytes()).unwrap();
    assert_eq!(metadata.description, "x\u{1f600}y");
}

/// A lone high surrogate escape is refused, never a crash (the
/// prototype's `LONE_SURROGATES["a high surrogate"]`).
#[test]
fn read_skill_metadata_refuses_a_lone_high_surrogate() {
    let bytes = br#"---
name: "d0"
description: "x\ud800y"
---
"#
    .to_vec();
    let error = read_skill_metadata("skills/d0/SKILL.md", &bytes).unwrap_err();
    let SkillMetadataError::InvalidUnicode(fields) = error;
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].field, "description");
    assert_eq!(fields[0].surrogates, vec![0xD800]);
}

/// A lone low surrogate escape is refused the same way (the prototype's
/// `LONE_SURROGATES["a low surrogate"]`); `U+DC80` also happens to be in
/// the byte-escape range the prototype's `surrogateescape` codec uses,
/// but this one comes from a `\u` escape, not a raw byte.
#[test]
fn read_skill_metadata_refuses_a_lone_low_surrogate() {
    let bytes = br#"---
name: "d1"
description: "x\udc80y"
---
"#
    .to_vec();
    let error = read_skill_metadata("skills/d1/SKILL.md", &bytes).unwrap_err();
    let SkillMetadataError::InvalidUnicode(fields) = error;
    assert_eq!(fields[0].surrogates, vec![0xDC80]);
}

/// A low surrogate immediately followed by a high one does not combine
/// (only high-then-low does): both are reported, in order (the
/// prototype's `LONE_SURROGATES["a reversed pair"]`).
#[test]
fn read_skill_metadata_refuses_a_reversed_surrogate_pair_as_two_lone_halves() {
    let bytes = br#"---
name: "d2"
description: "x\ude00\ud83dy"
---
"#
    .to_vec();
    let error = read_skill_metadata("skills/d2/SKILL.md", &bytes).unwrap_err();
    let SkillMetadataError::InvalidUnicode(fields) = error;
    assert_eq!(fields[0].surrogates, vec![0xDE00, 0xD83D]);
}

/// A raw byte that is not valid UTF-8 and a separate `\u` escape for the
/// same surrogate value in the same field are told apart: only the
/// escape is refused, the byte alone is carried lossily (the prototype's
/// own point: "a byte and an escape for the same surrogate are told
/// apart by count" -- in this Rust translation they cannot be confused
/// at all, since a raw invalid byte never becomes a surrogate codepoint
/// here, only `U+FFFD`).
#[test]
fn read_skill_metadata_tells_a_json_escape_apart_from_a_raw_invalid_byte() {
    let mut bytes = b"---\nname: es\ndescription: \"espa".to_vec();
    bytes.push(0xF1);
    bytes.extend_from_slice(b"ol \\udc80\"\n---\n");
    let error = read_skill_metadata("skills/es/SKILL.md", &bytes).unwrap_err();
    let SkillMetadataError::InvalidUnicode(fields) = error;
    assert_eq!(fields[0].field, "description");
    assert_eq!(fields[0].surrogates, vec![0xDC80]);
}

/// Without a `\u` escape, the same raw invalid byte is indexed as
/// before: no refusal (the prototype's own follow-up in the same test).
#[test]
fn read_skill_metadata_allows_a_raw_invalid_byte_alone() {
    let mut bytes = b"---\nname: es\ndescription: \"espa".to_vec();
    bytes.push(0xF1);
    bytes.extend_from_slice(b"ol\"\n---\n");
    let metadata = read_skill_metadata("skills/es/SKILL.md", &bytes).unwrap();
    assert!(metadata.description.contains('\u{FFFD}'));
}

use omnifrons_domain::skills_index::SkillRow;
use omnifrons_domain::skills_safety::unsafe_row_fields;

fn row(name: &str, description: &str, path: &str) -> SkillRow {
    SkillRow {
        name: name.to_string(),
        description: description.to_string(),
        path: path.to_string(),
    }
}

/// Translated from the `EMAIL_SKILLS` check in the prototype's
/// `test_block_quotes_at_signs_inside_words`: an `@` right after a
/// letter, an NFD combining mark included, is never refused.
#[test]
fn unsafe_row_fields_allows_at_signs_inside_words() {
    let rows = [
        row(
            "ops",
            "Reply from ops@example.com only.",
            "skills/ops/SKILL.md",
        ),
        row(
            "es",
            "Write to jos\u{e9}@ejemplo.es in Spanish.",
            "skills/es/SKILL.md",
        ),
        row(
            "nfd",
            "Write to jose\u{301}@ejemplo.es in Spanish.",
            "skills/nfd/SKILL.md",
        ),
    ];
    assert_eq!(unsafe_row_fields(&rows), Vec::<String>::new());
}

/// Translated from the "fine" case in the prototype's
/// `test_skill_paths_that_break_the_link_or_import_are_refused`:
/// ordinary characters in an existing folder name -- spaces,
/// parentheses, an in-word `@`, dots, hyphens, underscores -- are
/// indexed as before, never refused.
#[test]
fn unsafe_row_fields_allows_ordinary_path_characters() {
    let rows = [
        row("spaced", "Spaced things.", "skills/My Skill (v2)/SKILL.md"),
        row("team", "Team things.", "skills/team@home/SKILL.md"),
        row("dotted", "Dotted things.", "skills/a.b-c_d/SKILL.md"),
    ];
    assert_eq!(unsafe_row_fields(&rows), Vec::<String>::new());
}

/// Translated from the prototype's `UNSAFE_FIELDS` in
/// `test_block_refuses_unsafe_skill_metadata_before_any_write`: each
/// entry names the skill path, the field, and the exact word quoted.
#[test]
fn unsafe_row_fields_refuses_every_unsafe_field_case() {
    let rows = [
        row("@scope", "Scoped tasks.", "skills/@scope/SKILL.md"),
        row("s0", "Run `make`@secret.md first.", "skills/s0/SKILL.md"),
        row("s1", "@README", "skills/s1/SKILL.md"),
        row("s2", "**x**@y", "skills/s2/SKILL.md"),
        row("s3", "[d](u)@x.md", "skills/s3/SKILL.md"),
        row("Brief *pro*@secret.md", "Briefs.", "skills/s4/SKILL.md"),
    ];
    let problems = unsafe_row_fields(&rows);
    for expected in [
        "'skills/@scope/SKILL.md' (its name holds \"@scope\")",
        "'skills/s0/SKILL.md' (its description holds \"`make`@secret.md\")",
        "'skills/s1/SKILL.md' (its description holds \"@README\")",
        "'skills/s2/SKILL.md' (its description holds \"**x**@y\")",
        "'skills/s3/SKILL.md' (its description holds \"[d](u)@x.md\")",
        "'skills/s4/SKILL.md' (its name holds \"*pro*@secret.md\")",
    ] {
        assert!(
            problems.iter().any(|p| p == expected),
            "missing {expected:?} in {problems:?}"
        );
    }
}

/// Translated from the prototype's `ESCAPED_AT` in
/// `test_escaped_punctuation_before_an_at_is_refused_and_warned`: a
/// backslash-escaped `.`, `%`, `+` or `-` right before an `@` never
/// keeps it inside a word, in either a name or a description.
#[test]
fn unsafe_row_fields_refuses_escaped_punctuation_before_an_at() {
    let escaped = [
        "Run \\-@secret.md first.",
        "Run \\.@secret.md first.",
        "Run \\+@secret.md first.",
        "Run \\%@secret.md first.",
    ];
    let mut rows = Vec::new();
    for (index, text) in escaped.iter().enumerate() {
        rows.push(row(
            &format!("d{index}"),
            text,
            &format!("skills/d{index}/SKILL.md"),
        ));
        rows.push(row(text, "Briefs.", &format!("skills/n{index}/SKILL.md")));
    }
    let problems = unsafe_row_fields(&rows);
    for (index, text) in escaped.iter().enumerate() {
        let word = text
            .strip_prefix("Run ")
            .unwrap()
            .strip_suffix(" first.")
            .unwrap();
        assert!(
            problems.contains(&format!(
                "'skills/d{index}/SKILL.md' (its description holds \"{word}\")"
            )),
            "{problems:?}"
        );
        assert!(
            problems.contains(&format!(
                "'skills/n{index}/SKILL.md' (its name holds \"{word}\")"
            )),
            "{problems:?}"
        );
    }
}

/// Translated from the prototype's `cases` in
/// `test_skill_paths_that_break_the_link_or_import_are_refused`
/// (VER-H-003): the path cell is checked exactly as `render_row` would
/// write it, so an `@` the bracketed link target exposes, or a `<`,
/// `>`, or embedded line break that would break the link itself, is
/// refused.
#[test]
fn unsafe_row_fields_refuses_paths_that_break_the_link_or_import() {
    let cases: [(&str, Vec<&str>); 5] = [
        (
            "skills/a> @evil.md#x/SKILL.md",
            vec![
                "'skills/a> @evil.md#x/SKILL.md' (its path holds \"@evil.md#x/SKILL.md>\")",
                "'skills/a> @evil.md#x/SKILL.md' (its path holds '>', which breaks the link)",
            ],
        ),
        (
            "skills/<x>/SKILL.md",
            vec!["'skills/<x>/SKILL.md' (its path holds '<', '>', which breaks the link)"],
        ),
        (
            "skills/a\nb/SKILL.md",
            vec!["'skills/a\\nb/SKILL.md' (its path holds '\\n', which breaks the link)"],
        ),
        (
            "skills/@scope/SKILL.md",
            vec!["'skills/@scope/SKILL.md' (its path holds \"skills/@scope/SKILL.md\")"],
        ),
        (
            "skills/x\\-@y/SKILL.md",
            vec!["'skills/x\\\\-@y/SKILL.md' (its path holds \"skills/x\\-@y/SKILL.md\")"],
        ),
    ];
    for (path, expected) in cases {
        let rows = [row("tidy", "Tidy things.", path)];
        let problems = unsafe_row_fields(&rows);
        for word in expected {
            assert!(
                problems.iter().any(|p| p == word),
                "{path:?}: missing {word:?} in {problems:?}"
            );
        }
    }
}

/// Adapted from the prototype's `LATIN1_SKILLS` in
/// `test_undecodable_bytes_never_crash_any_output` (VER-H-001): a raw
/// byte that is not valid UTF-8 right next to an unsafe `@` is still
/// judged conservatively and never crashes anything, and the problem
/// names the undecodable value. The prototype shows the byte itself as
/// `\xe9`; this translation shows `U+FFFD` instead (see the
/// `skills_safety` module documentation), so the quoted word and the
/// appended note differ from the prototype's exact text while the
/// behaviour -- flagged, not crashed -- matches it.
#[test]
fn unsafe_row_fields_notes_a_raw_invalid_byte_next_to_an_unsafe_at() {
    let rows = [
        row(
            "es",
            "Escribe a jos\u{FFFD}@ejemplo.es",
            "skills/es/SKILL.md",
        ),
        row("caf\u{FFFD}@home", "d", "skills/fr/SKILL.md"),
    ];
    let problems = unsafe_row_fields(&rows);
    assert!(
        problems.iter().any(|p| {
            p.starts_with("'skills/es/SKILL.md' (its description holds \"jos\u{FFFD}@ejemplo.es\"")
                && p.contains("not valid UTF-8")
                && p.contains("re-save")
        }),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| {
            p.starts_with("'skills/fr/SKILL.md' (its name holds \"caf\u{FFFD}@home\"")
                && p.contains("not valid UTF-8")
        }),
        "{problems:?}"
    );
}
