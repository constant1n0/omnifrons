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

/// The file name every discovered or indexed skill ends in (the
/// prototype's `SKILL_FILE`).
const SKILL_FILE: &str = "SKILL.md";

/// A Markdown fence delimiter line (the prototype's `is_fence`, and the
/// same rule [`crate::definition::title`] uses): a trimmed line starting
/// with three backticks or three tildes. Both the opening and closing
/// delimiter toggle the fence state and are themselves excluded from
/// link detection.
fn is_fence_delimiter(trimmed_line: &str) -> bool {
    trimmed_line.starts_with("```") || trimmed_line.starts_with("~~~")
}

/// The index just past the run of whitespace starting at `start`:
/// Unicode `White_Space`, via [`char::is_whitespace`] (review follow-up,
/// correction pass: not just ASCII -- the prototype's `\s` in its
/// `LINK_PATTERN` regex runs over a Python `str`, where it is
/// Unicode-aware by default, so `char::is_whitespace` is the closer
/// match).
fn skip_spaces(chars: &[char], start: usize) -> usize {
    let mut pos = start;
    while chars.get(pos).is_some_and(|c| c.is_whitespace()) {
        pos += 1;
    }
    pos
}

/// Matches `\s*(<[^<>]*>|[^()\s]+)(?:\s+("[^"]*"|'[^']*'))?\s*\)` starting
/// at `start` (already past a line's `](`), the prototype's
/// `LINK_PATTERN` applied to one line. Returns the captured target
/// (angle brackets kept, exactly as written) and the index just past
/// the closing `)`, or `None` when nothing here matches.
fn match_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    let target_start = skip_spaces(chars, start);
    let mut pos = target_start;
    let mut bracketed = false;
    if chars.get(pos) == Some(&'<') {
        let mut scan = pos + 1;
        while chars.get(scan).is_some_and(|c| *c != '<' && *c != '>') {
            scan += 1;
        }
        if chars.get(scan) == Some(&'>') {
            pos = scan + 1;
            bracketed = true;
        }
        // Else: a nested `<` (or no closing `>` at all) before the
        // first `>`, so this is not a valid bracketed target. Fall
        // through to the bare-target branch below, starting over from
        // `target_start` -- the same `<` that opened the failed
        // attempt is just an ordinary character there (review
        // follow-up, correction pass: the regex alternation
        // `(<[^<>]*>|[^()\s]+)` tries its second branch here; an
        // earlier version returned no match at all instead).
    }
    if !bracketed {
        pos = target_start;
        if !chars
            .get(pos)
            .is_some_and(|c| *c != '(' && *c != ')' && !c.is_whitespace())
        {
            return None;
        }
        while chars
            .get(pos)
            .is_some_and(|c| *c != '(' && *c != ')' && !c.is_whitespace())
        {
            pos += 1;
        }
    }
    let target: String = chars[target_start..pos].iter().collect();
    let after_target = pos;
    let ws_pos = skip_spaces(chars, after_target);
    if ws_pos > after_target
        && let Some(quote) = chars
            .get(ws_pos)
            .copied()
            .filter(|c| *c == '"' || *c == '\'')
    {
        let mut end = ws_pos + 1;
        while chars.get(end).is_some_and(|c| *c != quote) {
            end += 1;
        }
        if chars.get(end) == Some(&quote) {
            let close = skip_spaces(chars, end + 1);
            return (chars.get(close) == Some(&')')).then_some((target, close + 1));
        }
        return None;
    }
    let close = skip_spaces(chars, after_target);
    (chars.get(close) == Some(&')')).then_some((target, close + 1))
}

/// Every `](target)` or `](<target>)` link in `line`, with an optional
/// quoted title (the prototype's `LINK_PATTERN.finditer`, applied per
/// line as the prototype itself applies it). Returns each target
/// exactly as written, angle brackets and all.
fn link_targets_in_line(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == ']'
            && chars[i + 1] == '('
            && let Some((target, next)) = match_link(&chars, i + 2)
        {
            out.push(target);
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

/// The `u8` value of two hexadecimal digits, or `None` when they are
/// not both hex digits.
fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    Some(digit(hi)? * 16 + digit(lo)?)
}

/// Percent-decodes `%XX` escapes in a Markdown link target (the
/// prototype's `urllib.parse.unquote`): a `%XX` that is not valid hex
/// is left exactly as written, byte for byte, like every other
/// character `unquote` does not recognize as an escape. Every
/// successfully-decoded escape is kept, even when assembling it next to
/// the surrounding bytes is not valid UTF-8 -- that invalid sequence is
/// replaced with `U+FFFD` (review follow-up, correction pass: an
/// earlier version discarded the *entire* decode and returned the
/// original percent-encoded text whenever any byte in it failed to
/// validate, which silently undid every escape that had decoded fine).
/// `unquote` never raises either way.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(decoded) = hex_byte(bytes[i + 1], bytes[i + 2])
        {
            out.push(decoded);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The basename of a POSIX path (the prototype's `posixpath.basename`):
/// everything after the last `/`, or the whole text when there is none.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Normalizes a linked `SKILL.md` target the way the prototype's
/// `indexed_skills` does for a path it already knows is relative
/// (`posixpath.normpath`): collapses `.` segments and repeated `/`, and
/// resolves `a/../` by dropping the segment before it, keeping an
/// unresolvable leading `..` as the library call does. A target that
/// looks absolute (starts with `/`) is returned unchanged: the
/// prototype relativizes it against the scanned root
/// (`os.path.relpath`), which needs a real filesystem root this pure
/// function does not have; the caller that does have one (ADR-0005 sub-
/// slice 1e's discovery service) relativizes it before comparing paths.
#[must_use]
pub fn normalize_relative_target(path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => match out.last() {
                Some(last) if *last != ".." => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}

/// The targets of every Markdown link outside fenced code whose
/// unquoted, `#fragment`-stripped target's basename is `SKILL.md` (the
/// prototype's `skill_link_targets` plus `indexed_skills`'
/// normalization, merged into one function here): angle brackets are
/// stripped, the target is percent-decoded, a link to an absolute URL
/// (`://` anywhere in it) is never a local skill link, and the
/// surviving target is normalized through [`normalize_relative_target`].
/// Sorted and de-duplicated, as the prototype's own `sorted(set(...))`
/// is. A hand-written index row in a project's own text is exactly one
/// of these targets; this function does not care whether the row was
/// written by a person or generated by Omnifrons.
#[must_use]
pub fn indexed_skill_targets(text: &[u8]) -> Vec<String> {
    let decoded = String::from_utf8_lossy(text);
    let mut in_fence = false;
    let mut targets: Vec<String> = Vec::new();
    for raw_line in decoded.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if is_fence_delimiter(trimmed) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for raw_target in link_targets_in_line(line) {
            let unbracketed = raw_target
                .strip_prefix('<')
                .and_then(|rest| rest.strip_suffix('>'))
                .map_or(raw_target.as_str(), str::trim);
            let before_fragment = unbracketed.split('#').next().unwrap_or("");
            let target = percent_decode(before_fragment);
            if target.contains("://") {
                continue;
            }
            if basename(&target) != SKILL_FILE {
                continue;
            }
            targets.push(normalize_relative_target(&target));
        }
    }
    targets.sort();
    targets.dedup();
    targets
}

/// How the canonical text indexes the skills discovered in the
/// workspace (the prototype's `SkillIndex`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndexStatus {
    /// The skills the caller discovered on disk, normalized through
    /// [`normalize_relative_target`] and de-duplicated (first
    /// occurrence kept), for the app layer's report (ADR-0005 sub-slice
    /// 1e). Review follow-up, correction pass: no longer echoed back
    /// exactly as given -- a caller-supplied `./`-prefixed or repeated
    /// path is normalized the same way [`indexed_skill_targets`]
    /// already normalizes what it finds, so the two sides of
    /// [`Self::missing`]/[`Self::dangling`] compare on equal footing.
    pub discovered: Vec<String>,
    /// The linked `SKILL.md` targets the text indexes, normalized,
    /// sorted, and de-duplicated ([`indexed_skill_targets`]).
    pub indexed: Vec<String>,
    /// Discovered, but not indexed.
    pub missing: Vec<String>,
    /// Indexed, but not discovered: a link to a `SKILL.md` that does
    /// not actually exist.
    pub dangling: Vec<String>,
    /// The precedence rule is already stated ([`has_precedence_phrases`]).
    pub precedence: bool,
}

/// The index status of `text` with respect to `discovered` (the
/// prototype's `read_skill_index`). `discovered` is normalized through
/// [`normalize_relative_target`] and de-duplicated (first occurrence
/// kept, order preserved) before any comparison, so a caller-supplied
/// `./`-prefixed or repeated path never produces a spurious
/// `missing`/`dangling` pair for the skill it actually names (review
/// follow-up, correction pass). [`SkillIndexStatus::missing`] is the
/// normalized `discovered` minus [`indexed_skill_targets`], in
/// `discovered`'s own order; [`SkillIndexStatus::dangling`] is the
/// indexed targets minus the normalized `discovered`, in their sorted
/// order.
#[must_use]
pub fn read_skill_index(text: &[u8], discovered: &[String]) -> SkillIndexStatus {
    let mut normalized_discovered: Vec<String> = Vec::with_capacity(discovered.len());
    for path in discovered {
        let normalized = normalize_relative_target(path);
        if !normalized_discovered.contains(&normalized) {
            normalized_discovered.push(normalized);
        }
    }
    let indexed = indexed_skill_targets(text);
    let missing = normalized_discovered
        .iter()
        .filter(|path| !indexed.contains(path))
        .cloned()
        .collect();
    let dangling = indexed
        .iter()
        .filter(|path| !normalized_discovered.contains(path))
        .cloned()
        .collect();
    let precedence = has_precedence_phrases(&String::from_utf8_lossy(text));
    SkillIndexStatus {
        discovered: normalized_discovered,
        indexed,
        missing,
        dangling,
        precedence,
    }
}

/// The discovered rows not already linked in `prefix_text` -- the text
/// before an existing Omnifrons skills block, or the whole file when
/// there is none -- in `discovered`'s own order (the prototype's
/// `plan_block`: `covered = set(indexed_skills(root, prefix)); skills =
/// [path for path in discovered if path not in covered]`, together
/// with the row-reading `plan_block` does for each one it keeps). This
/// function only filters the rows it is handed: discovering a
/// project's skills in the first place, and attributing a nested
/// project's skills to their own agent rather than its parent's, is
/// ADR-0005 sub-slice 1e's job, not this one's.
#[must_use]
pub fn rows_to_generate(prefix_text: &[u8], discovered: &[SkillRow]) -> Vec<SkillRow> {
    let covered = indexed_skill_targets(prefix_text);
    discovered
        .iter()
        .filter(|row| !covered.contains(&row.path))
        .cloned()
        .collect()
}

/// Whether the skills block needs to exist at all (the prototype's own
/// guard in `plan_block`, read together: `if not discovered: return
/// None`, then `if not status.missing and status.precedence: return
/// None`). With nothing discovered, no block is ever needed -- not even
/// when the text already carries a dangling link or already states the
/// precedence rule. Otherwise, a block is needed when a discovered
/// skill is missing from the index, or the precedence rule is not yet
/// stated, or both.
#[must_use]
pub fn block_needed(status: &SkillIndexStatus) -> bool {
    !status.discovered.is_empty() && (!status.missing.is_empty() || !status.precedence)
}

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
