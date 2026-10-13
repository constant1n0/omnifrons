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
    INDEX_HEADING, PRECEDENCE_PHRASES, PRECEDENCE_RULE, RULE_HEADING, SkillIndexStatus, SkillRow,
    block_needed, has_precedence_phrases, indexed_skill_targets, normalize_relative_target,
    read_skill_index, render_body, render_row, rows_to_generate,
};

fn row(name: &str, description: &str, path: &str) -> SkillRow {
    SkillRow {
        name: name.to_string(),
        description: description.to_string(),
        path: path.to_string(),
        // This helper always produces a plain fixture with no real
        // decode outcome to carry; a test that needs
        // `had_undecodable_bytes: true` builds its `SkillRow` literal
        // directly instead of going through `row`.
        had_undecodable_bytes: false,
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

/// `link_target` percent-encodes a path's `%` and `#` (`%` first, so
/// encoding `#` to `%23` never introduces a `%` that would itself get
/// re-encoded): the prototype's own `link_target` encodes neither, so
/// a path such as `skills/c#/SKILL.md` renders a link
/// `indexed_skill_targets` can never read back -- it always treats a
/// bare `#` as a fragment separator and truncates the target there.
/// Deliberately more correct than the prototype for exactly this
/// reason.
#[test]
fn render_row_percent_encodes_hash_and_percent_in_the_path() {
    assert_eq!(
        render_row(&row("x", "X.", "skills/c#/SKILL.md")),
        "| x | X. | [x](skills/c%23/SKILL.md) |"
    );
}

/// A rendered row's path round-trips through `indexed_skill_targets`
/// for every character the reader would otherwise misread without the
/// encoding above: a `#` (read as a fragment separator), a literal `%`
/// not meant as an escape, and a literal `%23` substring that must not
/// be mistaken for a doubly-escaped `#`.
#[test]
fn render_row_and_indexed_skill_targets_round_trip_hash_and_percent_paths() {
    for path in [
        "skills/c#/SKILL.md",
        "skills/100%/SKILL.md",
        "skills/%23/SKILL.md",
    ] {
        let rendered = render_row(&row("x", "X.", path));
        let text = format!("Intro.\n\n{rendered}\n").into_bytes();
        assert_eq!(
            indexed_skill_targets(&text, "/workspace"),
            vec![path.to_string()],
            "{path} must round-trip through render_row -> indexed_skill_targets"
        );
    }
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

/// Translated from the shape the prototype's `skill_link_targets`/
/// `indexed_skills` scan for (ADR-0005 sub-slice 1c): a Markdown link
/// outside fenced code whose target's basename is `SKILL.md`.
#[test]
fn indexed_skill_targets_extracts_links_outside_fences() {
    let text = b"Intro.\n\n[a](skills/a/SKILL.md)\n\n```\n[b](skills/b/SKILL.md)\n```\n\n[c](skills/c/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec![
            "skills/a/SKILL.md".to_string(),
            "skills/c/SKILL.md".to_string()
        ]
    );
}

/// An angle-bracketed target has its brackets stripped, and a
/// `#fragment` is stripped before the basename check.
#[test]
fn indexed_skill_targets_strips_angle_brackets_and_fragments() {
    let text = b"[sp](<skills/has space/SKILL.md>)\n[x](skills/x/SKILL.md#section)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec![
            "skills/has space/SKILL.md".to_string(),
            "skills/x/SKILL.md".to_string()
        ]
    );
}

/// A link whose target does not end in `SKILL.md`, or that points at an
/// absolute URL, is never indexed.
#[test]
fn indexed_skill_targets_ignores_non_skill_links_and_urls() {
    let text = b"[r](README.md)\n[u](https://example.com/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        Vec::<String>::new()
    );
}

/// Normalized like `posixpath.normpath` (the prototype's
/// `indexed_skills`), and de-duplicated once two targets normalize to
/// the same path.
#[test]
fn indexed_skill_targets_normalizes_dot_segments_and_dedupes() {
    let text = b"[a](./skills/./x/../x/SKILL.md)\n[b](skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// A percent-escaped target is decoded before the basename and
/// extension checks (the prototype's `unquote`).
#[test]
fn indexed_skill_targets_percent_decodes() {
    let text = b"[sp](skills/my%20skill/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/my skill/SKILL.md".to_string()]
    );
}

/// An absolute-looking target is relativized against the scanned
/// `root` (the prototype's `indexed_skills`: `os.path.relpath(target,
/// root)` for an absolute target, `posixpath.normpath(target)`
/// otherwise). A target already inside `root` comes back as the plain
/// relative path a discovered skill would carry, so it is never both
/// missing and dangling for the same skill. Verified with
/// `python3 -c "import os;
/// print(os.path.relpath('/workspace/skills/x/SKILL.md', '/workspace'))"`,
/// which prints `skills/x/SKILL.md`.
#[test]
fn indexed_skill_targets_relativizes_an_absolute_target_inside_the_root() {
    let text = b"[a](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// An absolute target outside `root` relativizes to the `../`-prefixed
/// form the prototype's `os.path.relpath` yields, never an unchanged
/// absolute path. Verified with
/// `python3 -c "import os;
/// print(os.path.relpath('/elsewhere/skills/x/SKILL.md', '/workspace'))"`,
/// which prints `../elsewhere/skills/x/SKILL.md`.
#[test]
fn indexed_skill_targets_relativizes_an_absolute_target_outside_the_root() {
    let text = b"[a](/elsewhere/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["../elsewhere/skills/x/SKILL.md".to_string()]
    );
}

/// `root` and an absolute target are each normalized (`.`/`..`
/// segments resolved) before the comparison, exactly as
/// `os.path.relpath`'s own `abspath` does. Verified with
/// `python3 -c "import os;
/// print(os.path.relpath('/workspace/a/../b/SKILL.md', '/workspace'))"`,
/// which prints `b/SKILL.md`.
#[test]
fn indexed_skill_targets_normalizes_dot_segments_before_relativizing() {
    let text = b"[a](/workspace/a/../b/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["b/SKILL.md".to_string()]
    );
}

/// An absolute link to a skill file directly under `root` -- one path
/// component deeper than the root itself, not equal to it -- comes
/// back as the bare filename. This is `relative_to_root`'s ordinary
/// case, not its empty-result `.` case: every target
/// `indexed_skill_targets` recognizes as a skill link already ends in
/// `SKILL.md`, so it can never equal a bare root path and never
/// reaches the `.` branch through this public API. That branch is
/// pinned directly, by calling `relative_to_root` itself, in its own
/// unit tests in `src/skills_index.rs`.
#[test]
fn indexed_skill_targets_relativizes_a_skill_file_directly_under_the_root_to_its_bare_filename() {
    let text = b"[a](/workspace/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["SKILL.md".to_string()]
    );
}

/// A relative target is still normalized exactly as before -- the root
/// parameter changes nothing about [`normalize_relative_target`]'s own
/// behavior, which only ever ran for relative targets.
#[test]
fn indexed_skill_targets_still_normalizes_a_relative_target_as_before() {
    let text = b"[a](./skills/./x/../x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// `indexed_skill_targets` threads `root` straight through to the
/// private `relative_to_root` (`src/skills_index.rs`), which treats a
/// non-absolute `root` as no root at all. An empty root has no
/// leading `/`, so an absolute link target passes through this public
/// function unrelativized rather than being guessed at.
#[test]
fn indexed_skill_targets_with_an_empty_root_leaves_an_absolute_target_unchanged() {
    let text = b"[a](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, ""),
        vec!["/workspace/skills/x/SKILL.md".to_string()]
    );
}

/// The same fallback applies to a `root` that is a relative path
/// rather than an absolute one.
#[test]
fn indexed_skill_targets_with_a_relative_root_leaves_an_absolute_target_unchanged() {
    let text = b"[a](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "relative/root"),
        vec!["/workspace/skills/x/SKILL.md".to_string()]
    );
}

/// Contrast case for the two fallback tests above: the same text,
/// relativized once `root` is actually an absolute POSIX path.
#[test]
fn indexed_skill_targets_with_an_absolute_root_relativizes_the_same_target() {
    let text = b"[a](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (behavior change, Windows support): a canonical
/// workspace root rendered as a Windows drive path (`C:/workspace`) now
/// relativizes an absolute Markdown link under the same drive, exactly
/// as a POSIX root already does for a POSIX-rooted link --
/// `relative_to_root`'s own unit tests in `src/skills_index.rs` cite the
/// `ntpath.relpath` oracle this extends.
#[test]
fn indexed_skill_targets_relativizes_a_windows_drive_absolute_target_inside_a_drive_root() {
    let text = b"[a](C:/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "C:/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// An absolute link inside the root names exactly the discovered
/// relative path: neither missing nor dangling.
#[test]
fn read_skill_index_treats_an_absolute_target_inside_the_root_as_its_relative_path() {
    let discovered = vec!["skills/x/SKILL.md".to_string()];
    let text = b"[x](/workspace/skills/x/SKILL.md)\n";
    let status = read_skill_index(text, &discovered, "/workspace");
    assert!(status.missing.is_empty(), "{status:?}");
    assert!(status.dangling.is_empty(), "{status:?}");
}

/// The same absolute-inside-root link is already covered, so
/// `rows_to_generate` never regenerates a row for it.
#[test]
fn rows_to_generate_treats_an_absolute_target_inside_the_root_as_already_covered() {
    let already = row("x", "X.", "skills/x/SKILL.md");
    let discovered = [already];
    let prefix = b"[x](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        rows_to_generate(prefix, &discovered, "/workspace"),
        Vec::<SkillRow>::new()
    );
}

/// Translated from the prototype's `test_scan_reports_skill_index_status`:
/// `missing` is discovered but not indexed, `dangling` is indexed but
/// not discovered, and `discovered` is echoed back for the app layer's
/// report.
#[test]
fn read_skill_index_reports_discovered_indexed_missing_and_dangling() {
    let discovered = vec![
        "skills/alpha/SKILL.md".to_string(),
        "skills/beta/SKILL.md".to_string(),
    ];
    let text = b"[alpha](skills/alpha/SKILL.md) and [gone](skills/gone/SKILL.md)\n";
    let status: SkillIndexStatus = read_skill_index(text, &discovered, "/workspace");
    assert_eq!(status.discovered, discovered);
    assert_eq!(
        status.indexed,
        vec![
            "skills/alpha/SKILL.md".to_string(),
            "skills/gone/SKILL.md".to_string()
        ]
    );
    assert_eq!(status.missing, vec!["skills/beta/SKILL.md".to_string()]);
    assert_eq!(status.dangling, vec!["skills/gone/SKILL.md".to_string()]);
    assert!(!status.precedence);
}

/// Nothing discovered and nothing indexed: every list is empty, and the
/// rule is reported absent unless it is actually stated.
#[test]
fn read_skill_index_with_nothing_discovered_or_indexed() {
    let status = read_skill_index(b"# Plain\n\nNo links here.\n", &[], "/workspace");
    assert_eq!(status.discovered, Vec::<String>::new());
    assert_eq!(status.indexed, Vec::<String>::new());
    assert_eq!(status.missing, Vec::<String>::new());
    assert_eq!(status.dangling, Vec::<String>::new());
    assert!(!status.precedence);
}

/// Translated from the prototype's `test_block_is_not_added_without_need`:
/// with nothing discovered at all, no block is ever needed -- not even
/// when the text already carries a dangling link and states the
/// precedence rule (`plan_block`'s own early return, `if not
/// discovered: return None`, before it even looks at the index status).
#[test]
fn block_needed_is_false_without_any_discovered_skills() {
    let status = read_skill_index(
        b"# Plain\n\nNothing skill-related here.\n",
        &[],
        "/workspace",
    );
    assert!(!block_needed(&status));

    let text =
        format!("# D\n\n[gone](skills/gone/SKILL.md)\n\n{RULE_HEADING}\n\n{PRECEDENCE_RULE}\n");
    let status = read_skill_index(text.as_bytes(), &[], "/workspace");
    assert_eq!(status.dangling, vec!["skills/gone/SKILL.md".to_string()]);
    assert!(status.precedence);
    assert!(!block_needed(&status));
}

/// Everything discovered is indexed and the rule is already stated: no
/// block is needed (the prototype's `atlas` case).
#[test]
fn block_needed_is_false_when_everything_is_indexed_with_precedence() {
    let discovered = vec!["skills/alpha/SKILL.md".to_string()];
    let text = format!(
        "# Atlas\n\n[alpha](skills/alpha/SKILL.md)\n\n{RULE_HEADING}\n\n{PRECEDENCE_RULE}\n"
    );
    let status = read_skill_index(text.as_bytes(), &discovered, "/workspace");
    assert!(status.missing.is_empty());
    assert!(status.precedence);
    assert!(!block_needed(&status));
}

/// A block is needed when a discovered skill is missing from the
/// index, or when the precedence rule is not yet stated, or both.
#[test]
fn block_needed_is_true_when_something_is_missing_or_precedence_is_absent() {
    let discovered = vec!["skills/alpha/SKILL.md".to_string()];
    let nothing_indexed = read_skill_index(b"No links here.\n", &discovered, "/workspace");
    assert!(block_needed(&nothing_indexed));

    let indexed_without_precedence = read_skill_index(
        b"[alpha](skills/alpha/SKILL.md)\n",
        &discovered,
        "/workspace",
    );
    assert!(indexed_without_precedence.missing.is_empty());
    assert!(!indexed_without_precedence.precedence);
    assert!(block_needed(&indexed_without_precedence));
}

/// `rows_to_generate` returns only the discovered rows the prefix text
/// does not already link, in discovery order.
#[test]
fn rows_to_generate_returns_discovered_rows_not_already_indexed() {
    let already = row("alpha", "A.", "skills/alpha/SKILL.md");
    let own = row("own", "Own.", "skills/own/SKILL.md");
    let discovered = [already, own.clone()];
    let prefix = b"[alpha](skills/alpha/SKILL.md)\n";
    assert_eq!(
        rows_to_generate(prefix, &discovered, "/workspace"),
        vec![own]
    );
}

/// Translated from the prototype's
/// `test_parent_block_never_indexes_child_skills` (the generated-rows
/// rule only; discovery and nesting are ADR-0005 sub-slice 1e's job):
/// `rows_to_generate` only ever filters the rows it is handed -- it
/// never discovers a skill on its own, so a nested child's skill never
/// appears unless the caller's own discovery already put it in
/// `discovered`.
#[test]
fn rows_to_generate_only_filters_the_given_rows_never_discovers_more() {
    let own = row("own", "Own.", "skills/own/SKILL.md");
    let discovered = [own.clone()];
    let prefix = b"# Parent\n\nThe child/ folder is another agent.\n";
    let rows = rows_to_generate(prefix, &discovered, "/workspace");
    assert_eq!(rows, vec![own]);
    assert!(
        !rows
            .iter()
            .any(|generated| generated.path.contains("child"))
    );
}

/// Review follow-up (WARNING, correction pass): `indexed_skill_targets`
/// returns normalized targets, but comparing a row's raw,
/// un-normalized `path` against them let a `./`-prefixed row slip past
/// as "not yet covered" and be regenerated as a duplicate link.
#[test]
fn rows_to_generate_treats_a_dot_slash_prefixed_row_as_already_covered() {
    let already = row("x", "X.", "./skills/x/SKILL.md");
    let discovered = [already];
    let prefix = b"[x](skills/x/SKILL.md)\n";
    assert_eq!(
        rows_to_generate(prefix, &discovered, "/workspace"),
        Vec::<SkillRow>::new()
    );
}

/// The same normalization applies to a `.` segment in the middle of a
/// row's path, not only a leading `./`.
#[test]
fn rows_to_generate_treats_a_dot_segment_in_the_middle_as_already_covered() {
    let already = row("x", "X.", "skills/./x/SKILL.md");
    let discovered = [already];
    let prefix = b"[x](skills/x/SKILL.md)\n";
    assert_eq!(
        rows_to_generate(prefix, &discovered, "/workspace"),
        Vec::<SkillRow>::new()
    );
}

/// `rows_to_generate`
/// normalizes an uncovered row's own path too, not only the covered
/// targets it compares against -- a `./`-prefixed row that is not yet
/// covered comes back with its path already normalized, its name and
/// description unchanged.
#[test]
fn rows_to_generate_normalizes_the_path_of_an_uncovered_row() {
    let uncovered = row("x", "X.", "./skills/x/SKILL.md");
    let discovered = [uncovered];
    assert_eq!(
        rows_to_generate(b"", &discovered, "/workspace"),
        vec![row("x", "X.", "skills/x/SKILL.md")]
    );
}

/// A repeated row -- the same path handed in twice -- is de-duplicated,
/// first occurrence kept, exactly as `read_skill_index` de-duplicates
/// `discovered`.
#[test]
fn rows_to_generate_deduplicates_a_repeated_row_first_seen_kept() {
    let dup = row("dup", "Dup.", "skills/dup/SKILL.md");
    let discovered = [dup.clone(), dup.clone()];
    assert_eq!(rows_to_generate(b"", &discovered, "/workspace"), vec![dup]);
}

/// `rows_to_generate` carries a row's `had_undecodable_bytes` flag
/// through its normalized copy unchanged, for a row that has it set as
/// well as a sibling that does not -- the copy must not default the
/// flag to `false` regardless of the input.
#[test]
fn rows_to_generate_carries_had_undecodable_bytes_through_unchanged() {
    let undecodable = SkillRow {
        name: "undecodable".to_string(),
        description: "Undecodable.".to_string(),
        path: "skills/undecodable/SKILL.md".to_string(),
        had_undecodable_bytes: true,
    };
    let clean = row("clean", "Clean.", "skills/clean/SKILL.md");
    let discovered = [undecodable.clone(), clean.clone()];
    assert_eq!(
        rows_to_generate(b"", &discovered, "/workspace"),
        vec![undecodable, clean]
    );
}

/// Review follow-up (correction pass): an invalid `%XX` escape (not two
/// hex digits) is left exactly as written -- the prototype's `unquote`
/// never raises on it.
#[test]
fn indexed_skill_targets_leaves_an_invalid_percent_escape_as_written() {
    let text = b"[a](skills/x%zzdir/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x%zzdir/SKILL.md".to_string()]
    );
}

/// Review follow-up: a `%XX` escape whose decoded byte is not valid
/// UTF-8 is replaced with `U+FFFD` rather than discarding the whole
/// decode, even when a validly-decoded escape follows it on the same
/// target (the prototype shows the byte as `\xNN`; this translation
/// uses the UTF-8 replacement character instead, as the `skills_safety`
/// module documents for the same reason).
#[test]
fn indexed_skill_targets_replaces_an_invalid_utf8_escape_next_to_a_valid_one() {
    let text = b"[a](skills/x%E9%20y/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x\u{FFFD} y/SKILL.md".to_string()]
    );
}

/// Review follow-up: a percent-encoded non-UTF-8 directory name in the
/// canonical text still matches a lossily-decoded discovered path (the
/// one ADR-0005 sub-slice 1e's filesystem walk would report for the
/// same on-disk bytes), so it is never reported as both missing and
/// dangling for the same skill.
#[test]
fn read_skill_index_matches_a_percent_encoded_target_to_its_lossily_decoded_discovered_path() {
    let discovered = vec!["skills/x\u{FFFD} y/SKILL.md".to_string()];
    let text = b"[a](skills/x%E9%20y/SKILL.md)\n";
    let status = read_skill_index(text, &discovered, "/workspace");
    assert!(status.missing.is_empty(), "{status:?}");
    assert!(status.dangling.is_empty(), "{status:?}");
}

/// Review follow-up (SUGGESTION): the regex alternation
/// `(<[^<>]*>|[^()\s]+)` falls through to the bare-target branch when
/// the angle-bracketed attempt cannot close -- here, a nested `<` before
/// any `>` -- rather than matching nothing at all.
#[test]
fn indexed_skill_targets_falls_through_to_bare_target_for_a_nested_angle_bracket() {
    let text = b"[a](<skills/<weird>/SKILL.md>)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/<weird>/SKILL.md".to_string()]
    );
}

/// Review follow-up (SUGGESTION): the regex alternation
/// `(<[^<>]*>|[^()\s]+)` also falls through to the bare-target branch
/// when the bracketed target closes but the remainder does not reach
/// `)` -- here, trailing text right after the closing `>`, with no
/// space before it and so no room for a quoted title -- rather than
/// failing to match at all.
#[test]
fn indexed_skill_targets_falls_through_to_bare_target_when_the_bracket_closes_but_the_remainder_misses_the_paren()
 {
    let text = b"[a](<s>kills/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["<s>kills/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): a double-quoted link title is matched
/// and ignored -- the target is the bare path only.
#[test]
fn indexed_skill_targets_ignores_a_double_quoted_title() {
    let text = b"[a](skills/x/SKILL.md \"A title\")\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): a single-quoted link title is matched
/// and ignored the same way.
#[test]
fn indexed_skill_targets_ignores_a_single_quoted_title() {
    let text = b"[a](skills/x/SKILL.md 'A title')\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): a title whose opening quote never
/// closes on the same line makes the whole link fail to match -- the
/// prototype's regex has no closing quote to anchor on either -- so
/// this line indexes nothing, while a later, well-formed line still
/// does.
#[test]
fn indexed_skill_targets_ignores_a_link_with_an_unterminated_quoted_title() {
    let text = b"[a](skills/x/SKILL.md \"untitled)\n[b](skills/y/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/y/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): text after a properly closed title, but
/// before the closing `)`, also makes the link fail to match.
#[test]
fn indexed_skill_targets_ignores_a_link_with_trailing_content_after_a_closed_quote() {
    let text = b"[a](skills/x/SKILL.md \"title\" extra)\n[b](skills/y/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/y/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): a bare target followed by stray,
/// unquoted text before `)` is not a title, so the link fails to match.
#[test]
fn indexed_skill_targets_ignores_a_bare_target_followed_by_stray_text_before_the_paren() {
    let text = b"[a](skills/x/SKILL.md stray)\n[b](skills/y/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/y/SKILL.md".to_string()]
    );
}

/// Review follow-up (coverage): a tilde fence hides a link exactly as a
/// backtick fence does.
#[test]
fn indexed_skill_targets_excludes_links_inside_a_tilde_fence() {
    let text =
        b"[a](skills/a/SKILL.md)\n\n~~~\n[b](skills/b/SKILL.md)\n~~~\n\n[c](skills/c/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec![
            "skills/a/SKILL.md".to_string(),
            "skills/c/SKILL.md".to_string()
        ]
    );
}

/// A fence closes only on a compatible delimiter (the same rule
/// [`omnifrons_domain::definition::title`] uses, through the shared
/// `crate::fences::FenceTracker`): a `~~~` line found inside a backtick
/// fence is content, not a close, so the link right after it stays
/// unindexed; a closing run shorter than the opener's does not close it
/// either; a run at least as long as the opener's does.
#[test]
fn indexed_skill_targets_closes_a_fence_only_on_a_compatible_delimiter() {
    let mismatched_char =
        b"```\n~~~\n[hidden](skills/hidden/SKILL.md)\n```\n\n[seen](skills/seen/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(mismatched_char, "/workspace"),
        vec!["skills/seen/SKILL.md".to_string()],
        "a mismatched ~~~ delimiter must not close a backtick fence"
    );

    let shorter_run = b"````\n[hidden](skills/hidden/SKILL.md)\n```\n\
                         [still_hidden](skills/still/SKILL.md)\n````\n\n\
                         [seen](skills/seen2/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(shorter_run, "/workspace"),
        vec!["skills/seen2/SKILL.md".to_string()],
        "a shorter closing run must not close a longer-run fence"
    );

    let longer_run =
        b"```\n[hidden](skills/hidden2/SKILL.md)\n````\n\n[seen](skills/seen3/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(longer_run, "/workspace"),
        vec!["skills/seen3/SKILL.md".to_string()],
        "a longer closing run must close a shorter-run fence"
    );
}

/// Review follow-up (coverage): CRLF line endings are stripped before a
/// line is scanned for links.
#[test]
fn indexed_skill_targets_handles_crlf_line_endings() {
    let text = b"[a](skills/a/SKILL.md)\r\n[b](skills/b/SKILL.md)\r\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec![
            "skills/a/SKILL.md".to_string(),
            "skills/b/SKILL.md".to_string()
        ]
    );
}

/// Review follow-up (SUGGESTION): a `./`-prefixed discovered path
/// normalizes to the same form `indexed_skill_targets` reports, so it
/// is never wrongly listed as missing.
#[test]
fn read_skill_index_normalizes_a_dot_slash_prefixed_discovered_path() {
    let discovered = vec!["./skills/x/SKILL.md".to_string()];
    let text = b"[x](skills/x/SKILL.md)\n";
    let status = read_skill_index(text, &discovered, "/workspace");
    assert_eq!(status.discovered, vec!["skills/x/SKILL.md".to_string()]);
    assert!(status.missing.is_empty(), "{status:?}");
}

/// Review follow-up (SUGGESTION): a repeated discovered path is
/// de-duplicated, first occurrence kept, both in the echoed
/// `discovered` list and in any derived list.
#[test]
fn read_skill_index_deduplicates_a_repeated_discovered_path() {
    let discovered = vec![
        "skills/x/SKILL.md".to_string(),
        "skills/x/SKILL.md".to_string(),
    ];
    let status = read_skill_index(b"No links here.\n", &discovered, "/workspace");
    assert_eq!(status.discovered, vec!["skills/x/SKILL.md".to_string()]);
    assert_eq!(status.missing, vec!["skills/x/SKILL.md".to_string()]);
}

/// Review follow-up (SUGGESTION): an unresolvable leading `..` is
/// kept, exactly as `posixpath.normpath` keeps it (verified with
/// `python3 -c "import posixpath;
/// print(posixpath.normpath('../SKILL.md'))"`, which prints
/// `../SKILL.md`).
#[test]
fn normalize_relative_target_keeps_an_unresolvable_leading_dot_dot() {
    assert_eq!(normalize_relative_target("../SKILL.md"), "../SKILL.md");
}

/// Review follow-up (SUGGESTION): two real segments fully absorb two
/// trailing `..`, leaving nothing to retain (verified with
/// `python3 -c "import posixpath;
/// print(posixpath.normpath('skills/x/../../SKILL.md'))"`, which
/// prints `SKILL.md` -- not `../SKILL.md`).
#[test]
fn normalize_relative_target_fully_absorbs_two_dot_dots_into_two_real_segments() {
    assert_eq!(
        normalize_relative_target("skills/x/../../SKILL.md"),
        "SKILL.md"
    );
}

/// Review follow-up (SUGGESTION): a `..` that follows an
/// already-retained `..` is retained in turn rather than popping it
/// (verified with `python3 -c "import posixpath;
/// print(posixpath.normpath('../../a/../SKILL.md'))"`, which prints
/// `../../SKILL.md`).
#[test]
fn normalize_relative_target_retains_a_dot_dot_that_follows_another_retained_one() {
    assert_eq!(
        normalize_relative_target("../../a/../SKILL.md"),
        "../../SKILL.md"
    );
}

/// Review follow-up (behavior change, verbatim UNC roots): a scanned
/// workspace root rendered in its real Windows verbatim UNC spelling
/// (`//?/UNC/server/share`, the slash-normalized form of
/// `\\?\UNC\server\share`) relativizes an absolute Markdown link under
/// the same share exactly as the plain `//server/share` spelling
/// already does -- `relative_to_root`'s own unit tests in
/// `src/skills_index.rs` cover the oracle parity this extends to a root
/// threaded through the public API.
#[test]
fn indexed_skill_targets_relativizes_under_a_verbatim_unc_root() {
    let text = b"[a](//server/share/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "//?/UNC/server/share"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// The same verbatim-UNC rewrite applies to a Markdown link's own
/// target, not only the scanned root: a verbatim-spelled link under a
/// plain-spelled root of the same share still relativizes.
#[test]
fn indexed_skill_targets_relativizes_a_verbatim_unc_target_against_a_plain_unc_root() {
    let text = b"[a](//?/UNC/server/share/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "//server/share"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (behavior change, UNC case folding): a scanned
/// workspace root and an absolute Markdown link naming the same UNC
/// share in different case now relativize to the same place, exactly as
/// a mixed-case drive letter already does above --
/// `relative_to_root`'s own unit tests in `src/skills_index.rs` cite the
/// `ntpath.relpath` oracle this extends to a UNC server and share.
#[test]
fn indexed_skill_targets_relativizes_a_case_mismatched_unc_pair() {
    let text = b"[a](//server/share/ws/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "//Server/share/ws"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (behavior change, POSIX doubled-slash targets): an
/// absolute Markdown link whose target carries an accidental doubled
/// leading slash still relativizes against a plain POSIX root, exactly
/// as `posixpath.relpath` collapses the same doubled slash --
/// `relative_to_root`'s own unit tests in `src/skills_index.rs` cite the
/// oracle this extends to the public API.
#[test]
fn indexed_skill_targets_relativizes_a_posix_doubled_slash_target() {
    let text = b"[a](//workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// Review follow-up (behavior change, `..` past a mount): the same
/// drive-pinning fix in the private `relative_to_root`
/// (`src/skills_index.rs`) applies through this public API when the
/// scanned workspace `root` itself carries a `..` that would otherwise
/// climb past its own drive letter. Verified with
/// `ntpath.relpath(r'C:\workspace\skills\x\SKILL.md',
/// r'C:\..\workspace')`, which prints `skills\x\SKILL.md`.
#[test]
fn indexed_skill_targets_of_a_dot_dot_past_a_drive_root_pins_the_drive() {
    let text = b"[a](C:/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "C:/../workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}

/// The same fix for a `..` climbing past a UNC share named in the
/// Markdown link's own target, through this public API. Verified with
/// `ntpath.relpath(r'\\server\share\..\x\SKILL.md', r'\\server\share')`,
/// which prints `x\SKILL.md`.
#[test]
fn indexed_skill_targets_of_a_dot_dot_past_a_unc_share_in_the_target_pins_the_share() {
    let text = b"[a](//server/share/../x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "//server/share"),
        vec!["x/SKILL.md".to_string()]
    );
}

/// The POSIX parity for the same scanned-root `..`, through this
/// public API. Verified with
/// `posixpath.relpath('/workspace/skills/x/SKILL.md', '/../workspace')`,
/// which prints `skills/x/SKILL.md`.
#[test]
fn indexed_skill_targets_of_a_dot_dot_past_the_posix_root_pins_the_root() {
    let text = b"[a](/workspace/skills/x/SKILL.md)\n";
    assert_eq!(
        indexed_skill_targets(text, "/../workspace"),
        vec!["skills/x/SKILL.md".to_string()]
    );
}
