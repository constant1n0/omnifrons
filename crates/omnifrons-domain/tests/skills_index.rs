//! Domain tests for the skills-index block body (ADR-0005 "Agent
//! identity and portable definition" decision 2, sub-slice 1b's part
//! C): the headings, the golden precedence rule and its signature
//! phrases, and the row rendering that escapes a Markdown table cell
//! and a link label. Translated from the `base_omnifrons` prototype's
//! shape assertions in
//! `test_adapt_appends_the_golden_block_and_preserves_the_prefix` and
//! `test_block_golden_shapes_for_echo_nara_and_crlf` -- the body only:
//! this crate's sentinels differ from the prototype's, and CRLF
//! rendering is [`omnifrons_domain::guidance::ManagedBlock::render`]'s
//! own line-ending parameter, not this module's concern.

use omnifrons_domain::skills_index::{
    INDEX_HEADING, PRECEDENCE_PHRASES, PRECEDENCE_RULE, RULE_HEADING, SkillRow,
    has_precedence_phrases, render_body, render_row,
};

fn row(name: &str, description: &str, path: &str) -> SkillRow {
    SkillRow {
        name: name.to_string(),
        description: description.to_string(),
        path: path.to_string(),
    }
}

/// The headings and the precedence rule's own text are the prototype's
/// fixed constants, verbatim (`portable_agent.py`'s `INDEX_HEADING`,
/// `RULE_HEADING`, `PRECEDENCE_RULE`, `PRECEDENCE_PHRASES`).
#[test]
fn the_headings_and_precedence_rule_are_the_prototypes_own_text() {
    assert_eq!(INDEX_HEADING, "## Local skill index");
    assert_eq!(RULE_HEADING, "## Local skill precedence");
    assert_eq!(
        PRECEDENCE_RULE,
        "The skill paths listed in this file form this agent's local skill index.\n\
         For every matching task, the path in this index is authoritative: read that\n\
         exact workspace file through normal file access, regardless of any native\n\
         provider skill registry.\n\
         Never substitute a same-named user, global, or provider skill.\n\
         A native Skill tool may be used only when it demonstrably resolves to the\n\
         exact indexed workspace path.\n\
         Resolve companion skill calls through this local index."
    );
    assert_eq!(PRECEDENCE_PHRASES.len(), 3);
    assert_eq!(PRECEDENCE_PHRASES[0], "this index is authoritative");
    assert_eq!(
        PRECEDENCE_PHRASES[1],
        "Never substitute a same-named user, global, or provider skill"
    );
    assert_eq!(
        PRECEDENCE_PHRASES[2],
        "demonstrably resolves to the exact indexed workspace path"
    );
}

/// Translated from the prototype's `has_precedence_phrases`: all three
/// signature phrases must be present, case-folded and
/// whitespace-collapsed, for the rule to be considered already stated.
#[test]
fn has_precedence_phrases_is_case_folded_and_whitespace_collapsed() {
    assert!(has_precedence_phrases(PRECEDENCE_RULE));
    assert!(has_precedence_phrases(&PRECEDENCE_RULE.to_uppercase()));
    assert!(has_precedence_phrases(
        "This   INDEX is\nauthoritative. Never substitute a same-named user, \
         global,   or provider skill. It demonstrably   resolves to the exact \
         indexed workspace path."
    ));
    assert!(!has_precedence_phrases("This index is authoritative."));
    assert!(!has_precedence_phrases(""));
}

/// Translated from the row shapes in the prototype's `SKILLED_ROWS` and
/// the `nara` case in `test_block_golden_shapes_for_echo_nara_and_crlf`:
/// a short, plain row renders with an unescaped label and a bare link
/// target, and a description carrying a literal `|` is escaped.
#[test]
fn render_row_escapes_pipes_and_leaves_plain_rows_bare() {
    assert_eq!(
        render_row(&row("x", "X.", "skills/x/SKILL.md")),
        "| x | X. | [x](skills/x/SKILL.md) |"
    );
    assert_eq!(
        render_row(&row("beta", "Beta | gamma tasks.", "skills/beta/SKILL.md")),
        "| beta | Beta \\| gamma tasks. | [beta](skills/beta/SKILL.md) |"
    );
    assert_eq!(
        render_row(&row(
            "delta-skill",
            "Delta: \"quoted\" tasks.",
            "skills/delta/SKILL.md"
        )),
        "| delta-skill | Delta: \"quoted\" tasks. | [delta-skill](skills/delta/SKILL.md) |"
    );
}

/// A row whose name carries `[` or `]`, or whose path carries
/// whitespace or `(`, `)`, `<`, `>`, escapes the link label and
/// angle-brackets the target (the prototype's `render_row`/
/// `link_target`).
#[test]
fn render_row_escapes_brackets_in_the_label_and_angle_brackets_odd_paths() {
    assert_eq!(
        render_row(&row("[odd]", "Odd.", "skills/odd/SKILL.md")),
        "| [odd] | Odd. | [\\[odd\\]](skills/odd/SKILL.md) |"
    );
    assert_eq!(
        render_row(&row("sp", "Sp.", "skills/has space/SKILL.md")),
        "| sp | Sp. | [sp](<skills/has space/SKILL.md>) |"
    );
    for odd in ["skills/(a)/SKILL.md", "skills/<a>/SKILL.md"] {
        let rendered = render_row(&row("p", "P.", odd));
        assert!(rendered.ends_with(&format!("[p](<{odd}>) |")), "{rendered}");
    }
}

/// Embedded line breaks in a name or description become spaces (the
/// prototype's `table_cell`).
#[test]
fn render_row_joins_embedded_line_breaks_with_spaces() {
    assert_eq!(
        render_row(&row("multi\nline", "a\r\nb", "skills/m/SKILL.md")),
        "| multi line | a b | [multi line](skills/m/SKILL.md) |"
    );
}

/// Review follow-up (correction pass): a bare `\r` (no `\n` after it)
/// is also a line break -- the prototype's `str.splitlines()`
/// semantics -- and a `\r\n` pair is one break, not two, so it never
/// leaves a spurious doubled space behind.
#[test]
fn render_row_treats_a_bare_carriage_return_as_a_line_break() {
    assert_eq!(
        render_row(&row("a\rb", "c\rd\r\ne", "skills/r/SKILL.md")),
        "| a b | c d e | [a b](skills/r/SKILL.md) |"
    );
}

/// Translated from the prototype's `SKILLED_ROWS` plus `rule_section()`
/// shape in `test_adapt_appends_the_golden_block_and_preserves_the_prefix`:
/// the index section, a blank line, then the rule section. Our
/// canonical body has no trailing newline, unlike the prototype's
/// byte-level fixture, which always carries one before its own `end`
/// marker.
#[test]
fn render_body_joins_the_index_and_rule_sections_with_one_blank_line() {
    let rows = [
        row("beta", "Beta | gamma tasks.", "skills/beta/SKILL.md"),
        row(
            "delta-skill",
            "Delta: \"quoted\" tasks.",
            "skills/delta/SKILL.md",
        ),
        row("fenced", "Fenced tasks, folded.", "skills/fenced/SKILL.md"),
    ];
    let expected = "## Local skill index\n\
\n\
| Skill | Use when | Path |\n\
| --- | --- | --- |\n\
| beta | Beta \\| gamma tasks. | [beta](skills/beta/SKILL.md) |\n\
| delta-skill | Delta: \"quoted\" tasks. | [delta-skill](skills/delta/SKILL.md) |\n\
| fenced | Fenced tasks, folded. | [fenced](skills/fenced/SKILL.md) |\n\
\n\
## Local skill precedence\n\
\n"
    .to_string()
        + PRECEDENCE_RULE;
    assert_eq!(render_body(&rows, true), expected);
    assert!(
        !render_body(&rows, true).ends_with('\n'),
        "no trailing newline"
    );
}

/// Translated from the `nara` shape in
/// `test_block_golden_shapes_for_echo_nara_and_crlf`: a single row, no
/// rule (that fixture's own `AGENTS.md` already states it).
#[test]
fn render_body_with_rows_and_no_rule_omits_the_rule_section() {
    let rows = [row("x", "X.", "skills/x/SKILL.md")];
    assert_eq!(
        render_body(&rows, false),
        "## Local skill index\n\n| Skill | Use when | Path |\n| --- | --- | --- |\n\
         | x | X. | [x](skills/x/SKILL.md) |"
    );
}

/// Translated from the `echo` shape in the same test: no discovered
/// skills, so only the rule section is rendered, with no leading blank
/// line separating it from an empty index section.
#[test]
fn render_body_with_no_rows_renders_only_the_rule_section() {
    assert_eq!(
        render_body(&[], true),
        format!("{RULE_HEADING}\n\n{PRECEDENCE_RULE}")
    );
}

/// Neither section applies: the empty body (no block would be
/// written).
#[test]
fn render_body_with_nothing_to_say_is_empty() {
    assert_eq!(render_body(&[], false), "");
}
