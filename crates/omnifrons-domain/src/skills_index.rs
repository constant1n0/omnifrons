//! The body of the skills-index block Omnifrons writes under
//! [`crate::guidance::ManagedFileKind::Skills`] (ADR-0005 "Agent
//! identity and portable definition" decision 2, sub-slice 1b's part
//! C): the two headings, the golden precedence rule and its signature
//! phrases, and the row rendering that turns one discovered skill into
//! a Markdown table row. Pure text, like [`crate::guidance`]: nothing
//! here discovers a `SKILL.md`, parses its frontmatter, or decides
//! whether a project needs the block at all -- those belong to
//! ADR-0005's sub-slice 1c.
//!
//! Translated from the `base_omnifrons` prototype's
//! `adapter/portable-agent/scripts/portable_agent.py`: `INDEX_HEADING`,
//! `RULE_HEADING`, `PRECEDENCE_PHRASES`, and `PRECEDENCE_RULE` (lines
//! 104-128), `has_precedence_phrases`, and the row-rendering helpers
//! `table_cell`, `link_target`, and `render_row`. The canonical body
//! this module renders omits the prototype's own sentinels and the
//! trailing newline every rendered line carries there:
//! [`crate::guidance::ManagedBlock`] supplies both, exactly as it
//! already does for the guidance note.

/// The skills-index section's heading (the prototype's `INDEX_HEADING`).
pub const INDEX_HEADING: &str = "## Local skill index";

/// The precedence-rule section's heading (the prototype's
/// `RULE_HEADING`).
pub const RULE_HEADING: &str = "## Local skill precedence";

/// The golden, name-neutral skill-precedence rule (the prototype's
/// `PRECEDENCE_RULE`), verbatim: eight sentences, one per line, stating
/// skill precedence only -- no identity, authority, or tool rule is
/// added to an existing agent. `\n`-joined with no trailing newline, as
/// [`render_body`] embeds it.
pub const PRECEDENCE_RULE: &str = "The skill paths listed in this file form this agent's local skill index.\n\
For every matching task, the path in this index is authoritative: read that\n\
exact workspace file through normal file access, regardless of any native\n\
provider skill registry.\n\
Never substitute a same-named user, global, or provider skill.\n\
A native Skill tool may be used only when it demonstrably resolves to the\n\
exact indexed workspace path.\n\
Resolve companion skill calls through this local index.";

/// Atlas's signature phrases (the prototype's `PRECEDENCE_PHRASES`):
/// when a file's body carries all three, case-folded and
/// whitespace-collapsed, [`has_precedence_phrases`] reports that the
/// agent already states the rule, and Omnifrons never appends another
/// copy of it.
pub const PRECEDENCE_PHRASES: [&str; 3] = [
    "this index is authoritative",
    "Never substitute a same-named user, global, or provider skill",
    "demonstrably resolves to the exact indexed workspace path",
];

/// All three [`PRECEDENCE_PHRASES`] are present in `text`, case-folded
/// and with every run of whitespace collapsed to one space (the
/// prototype's `has_precedence_phrases`): a line wrap or a doubled
/// space inside a phrase never hides it, and case never matters.
#[must_use]
pub fn has_precedence_phrases(text: &str) -> bool {
    let haystack = normalize_whitespace(text);
    PRECEDENCE_PHRASES
        .iter()
        .all(|phrase| haystack.contains(&normalize_whitespace(phrase)))
}

/// Every run of whitespace collapsed to one space, trimmed at both
/// ends, lowercased: the comparison shape `has_precedence_phrases` uses
/// for both the haystack and each phrase.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// One row of the skill index table: a skill's `name` and
/// `description`, and the project-relative POSIX path of its
/// `SKILL.md` (the prototype's `(name, description, path)` row tuple).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRow {
    /// The skill's name (its frontmatter `name:`, or its folder name).
    pub name: String,
    /// The skill's description (its frontmatter `description:`,
    /// verbatim).
    pub description: String,
    /// The project-relative POSIX path of the skill's `SKILL.md`.
    pub path: String,
}

/// A Markdown table cell (the prototype's `table_cell`): embedded line
/// breaks become spaces, and a literal `|` is escaped so it cannot
/// split the cell into another column. A line break is `\n`, `\r\n`, or
/// a bare `\r` -- the prototype's `str.splitlines()` restricted to
/// these three ASCII shapes; its handful of exotic Unicode line and
/// paragraph separators are not reproduced here, since a skill's name
/// or description is not expected to carry one.
#[must_use]
pub fn table_cell(text: &str) -> String {
    split_lines(text).join(" ").replace('|', "\\|")
}

/// Splits `text` on `\n`, `\r\n`, or a bare `\r`, treating `\r\n` as one
/// break rather than two (so it never yields a spurious empty line
/// between the two halves of a CRLF pair), and never yields a trailing
/// empty line for text ending in one -- matching the prototype's
/// `str.splitlines()` for these three ASCII line-break shapes.
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(&text[start..index]);
                index += 1;
                start = index;
            }
            b'\r' => {
                lines.push(&text[start..index]);
                index += 1;
                if bytes.get(index) == Some(&b'\n') {
                    index += 1;
                }
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The link target for a row's path (the prototype's `link_target`):
/// wrapped in angle brackets when the path holds whitespace or `(`,
/// `)`, `<`, or `>` -- the characters that would otherwise break a bare
/// Markdown link target.
#[must_use]
pub fn link_target(path: &str) -> String {
    if path
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'))
    {
        format!("<{path}>")
    } else {
        path.to_string()
    }
}

/// One rendered row of the skill index table (the prototype's
/// `render_row`): the name and description as table cells, and the
/// path as a link whose label is the name with `[` and `]` escaped and
/// whose target [`link_target`] angle-brackets when needed.
#[must_use]
pub fn render_row(row: &SkillRow) -> String {
    let label = table_cell(&row.name)
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!(
        "| {} | {} | [{label}]({}) |",
        table_cell(&row.name),
        table_cell(&row.description),
        link_target(&row.path)
    )
}

/// The canonical skills-block body (`\n`-joined, no trailing newline;
/// the prototype's `render_block`, minus the sentinels and the
/// trailing newline every line carries there -- both
/// [`crate::guidance::ManagedBlock::render`] supplies, through its own
/// `ending` parameter): when `rows` is non-empty, [`INDEX_HEADING`], a
/// blank line, the table header and separator, then one rendered row
/// per entry; then, when `include_rule`, [`RULE_HEADING`], a blank
/// line, and [`PRECEDENCE_RULE`] -- with one blank line between the
/// two sections only when both are present. Empty (no block to write)
/// when `rows` is empty and `include_rule` is `false`.
#[must_use]
pub fn render_body(rows: &[SkillRow], include_rule: bool) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !rows.is_empty() {
        let mut index = vec![
            INDEX_HEADING.to_string(),
            String::new(),
            "| Skill | Use when | Path |".to_string(),
            "| --- | --- | --- |".to_string(),
        ];
        index.extend(rows.iter().map(render_row));
        sections.push(index.join("\n"));
    }
    if include_rule {
        sections.push(format!("{RULE_HEADING}\n\n{PRECEDENCE_RULE}"));
    }
    sections.join("\n\n")
}
