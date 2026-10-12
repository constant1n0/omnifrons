//! The nine warnings a scan reports per agent root (ADR-0005 "Agent
//! identity and portable definition", sub-slice 1d, last part):
//! source-derived observations about how one of the nine documented
//! hosts ([`crate::hosts`]) might shadow, truncate, or misread an
//! agent's canonical files. A warning is data for the user; Omnifrons
//! guarantees behaviour only through its own built-in adapters, never
//! through this table (see [`crate::hosts`]'s own module documentation).
//!
//! Translated from the `base_omnifrons` prototype's
//! `adapter/portable-agent/scripts/portable_agent.py`: `WARNING_ORDER`,
//! `make_warning`, `host_list`, `ancestor_warnings`, `mentions`,
//! `child_warnings`, `import_warnings`, `masked_inline_code`,
//! `boundary_import_warnings`, `size_warnings`, and `agent_warnings`.
//! Framework-independent, like every other module this crate holds: no
//! filesystem access, no regex crate (`omnifrons-domain` depends on
//! `std` and `thiserror` only). Every prototype regex this module needs
//! -- `IMPORT_PATTERN`, and the lookarounds in `mentions` -- is
//! translated into hand-written character scanning instead.
//!
//! # Provenance: [`AgentInput`]
//!
//! This module is pure: it never reads a file or walks a directory.
//! [`AgentInput`] is the shape a future filesystem port (ADR-0005
//! sub-slice 1e/1f) is expected to fill in from one scanned agent root;
//! today, this crate's own tests are the only caller. Each field names
//! the prototype function that consumes it -- see the field docs below.
//!
//! # Known modelling limits
//!
//! Inherited from [`crate::hosts`]'s own documented limits, since this
//! module's warnings are built entirely from that table:
//!
//! - [`crate::hosts::upward_loaders`] does not model Codex's, Gemini
//!   CLI's, Antigravity's, or Kimi's own directory walks (only Claude
//!   Code and Qwen Code are upward loaders in the prototype's
//!   `UPWARD_LOADERS`), so [`ancestor_exposure`](WarningCode::AncestorExposure)/
//!   [`ancestor_shadowing`](WarningCode::AncestorShadowing) can never
//!   name those hosts.
//! - Claude Code's documented fallback to `AGENTS.md`/`.claude/AGENTS.md`
//!   depends on a configurable instruction-files mode; this module, like
//!   [`crate::hosts`], models only the default mode.

use crate::definition::{IdentityKind, is_import_only};
use crate::fences::FenceTracker;
use crate::hosts;
use crate::skills_safety::unsafe_at_tokens;

/// Appended to every warning's detail, verbatim from the prototype's
/// `SOURCE_NOTE`: every row of [`crate::hosts`] is a claim about a host,
/// not a guarantee, and this module's own warnings inherit that caveat.
const SOURCE_NOTE: &str = "source-derived, not runtime-tested";

/// One of the nine warnings a scan can report (the prototype's
/// `WARNING_ORDER`, as a closed enum instead of a string tuple). Variant
/// order is declaration order, and each variant's discriminant is its
/// position in `WARNING_ORDER`, so `order_index` is a plain cast -- no
/// separate lookup table to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WarningCode {
    /// A host also loads an ancestor's identity file next to this
    /// agent's own, in addition to this agent's instructions.
    AncestorExposure = 0,
    /// A host loads an ancestor's identity file instead of this
    /// agent's instructions, because this folder holds no file that
    /// host reads.
    AncestorShadowing = 1,
    /// A nested agent's relative path is never named in this agent's
    /// own canonical text.
    ChildNotMentioned = 2,
    /// `@`-style text a host that does not expand imports would read
    /// literally.
    HostImports = 3,
    /// An `@` Claude Code may load as an import that [`Self::HostImports`]
    /// does not already cover.
    HostImportsBoundary = 4,
    /// A host-specific file present alongside the identity files,
    /// reported and never touched.
    ExtraHostFile = 5,
    /// The canonical text exceeds Codex's default size cap.
    SizeCodex = 6,
    /// The canonical text exceeds Antigravity's truncation cap.
    SizeAntigravity = 7,
    /// A generated tool-state directory exists at this root.
    GeneratedStateDir = 8,
}

impl WarningCode {
    /// Every code, in the prototype's `WARNING_ORDER`.
    pub const ALL: [Self; 9] = [
        Self::AncestorExposure,
        Self::AncestorShadowing,
        Self::ChildNotMentioned,
        Self::HostImports,
        Self::HostImportsBoundary,
        Self::ExtraHostFile,
        Self::SizeCodex,
        Self::SizeAntigravity,
        Self::GeneratedStateDir,
    ];

    /// This code's stable token, spelled exactly as the prototype's
    /// `WARNING_ORDER` entries.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AncestorExposure => "ancestor_exposure",
            Self::AncestorShadowing => "ancestor_shadowing",
            Self::ChildNotMentioned => "child_not_mentioned",
            Self::HostImports => "host_imports",
            Self::HostImportsBoundary => "host_imports_boundary",
            Self::ExtraHostFile => "extra_host_file",
            Self::SizeCodex => "size_codex",
            Self::SizeAntigravity => "size_antigravity",
            Self::GeneratedStateDir => "generated_state_dir",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == token)
    }

    /// This code's position in `WARNING_ORDER`, the order
    /// [`agent_warnings`] sorts its result by.
    const fn order_index(self) -> usize {
        self as usize
    }
}

/// One warning: its code, the hosts it names (sorted by
/// [`crate::hosts::order_index`] and de-duplicated, never empty unless
/// the warning itself never names a host, such as
/// [`WarningCode::GeneratedStateDir`]), and its detail message (the
/// prototype's `make_warning` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Which of the nine warnings this is.
    pub code: WarningCode,
    /// The hosts this warning names, in [`crate::hosts::order_index`]
    /// order with duplicates removed.
    pub hosts: Vec<&'static str>,
    /// Why, in the prototype's own wording.
    pub detail: String,
}

/// One ancestor in an agent's warning chain (the prototype's
/// `AncestorChain` entries, nearest first): either a root the same scan
/// also discovered, or an ancestor outside it entirely. Mirrors
/// [`crate::nesting::ChainLink`]/[`crate::nesting::Ancestor`], but
/// additionally carries the identity file names found there --
/// [`crate::nesting::ChainLink::Root`] names only a path, with no
/// identity information, and `nesting.rs` is outside this task's
/// allowed edit surface, so this module carries its own, self-contained
/// shape instead of extending that one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainAncestor<'a> {
    /// This ancestor's own location, shown verbatim in a warning's
    /// detail (the prototype's `str(ancestor)` Path formatting): a
    /// scan-relative path for a root inside the scan, or an opaque
    /// outside path for an ancestor above it.
    pub display: &'a str,
    /// The identity-file names found there (for example `"CLAUDE.md"`),
    /// in the port's own discovery order -- the prototype's
    /// `ancestor_files` dict keys, shown the same way in a warning's
    /// detail.
    pub identity_files: &'a [&'static str],
}

/// Everything [`agent_warnings`] needs for one agent root, already
/// decided by a port (ADR-0005 sub-slice 1e/1f's filesystem adapter;
/// today, this crate's tests). Pure data: this module never reads a
/// file or walks a directory to fill in a field itself.
#[derive(Debug, Clone, Copy)]
pub struct AgentInput<'a> {
    /// The identity-file kinds present at this agent's own root (the
    /// prototype's `files` dict keys): consumed by the private
    /// `ancestor_warnings` to tell exposure from shadowing and by the
    /// private `child_warnings` for its reader list.
    pub identity_kinds: &'a [IdentityKind],
    /// The agent's canonical text, already decoded (`None` when there
    /// is none to read, matching the prototype's `text: Optional[bytes]`
    /// parameter being `None`): consumed by the private
    /// `child_warnings`, `import_warnings`, `boundary_import_warnings`,
    /// and `size_warnings`.
    pub text: Option<&'a str>,
    /// The raw bytes of `AGENTS.override.md`, when present at this root
    /// (the prototype's `files.get(OVERRIDE)`): consumed by the private
    /// `import_warnings`' override-only check.
    pub override_bytes: Option<&'a [u8]>,
    /// This agent's warning chain, nearest ancestor first (the
    /// prototype's `AncestorChain`, composing
    /// [`crate::nesting::warning_chain`]'s inside-the-scan roots with
    /// its outside ancestors): consumed by the private
    /// `ancestor_warnings`.
    pub chain: &'a [ChainAncestor<'a>],
    /// The nested agents' paths, relative to this root, POSIX-style
    /// (the prototype's `nested: Sequence[Path]`, relativized):
    /// consumed by the private `child_warnings`.
    pub nested: &'a [&'a str],
    /// The host-specific extra file names present alongside this
    /// root's identity files (the prototype's `extras: Sequence[str]`),
    /// each expected to be one of [`crate::hosts::EXTRA_READERS`]'s
    /// entries: consumed by [`agent_warnings`] directly for the
    /// `extra_host_file` warnings.
    pub host_extras: &'a [&'static str],
    /// Which of [`crate::hosts::GENERATED_STATE_DIRS`]'s entries exist
    /// at this root (the prototype's `os.path.lexists(path / name)`
    /// check): consumed by [`agent_warnings`] directly for the
    /// `generated_state_dir` warnings.
    pub generated_state_dirs_present: &'a [&'static str],
}

/// `hosts`, de-duplicated and sorted by [`crate::hosts::order_index`]
/// (the prototype's `sorted(set(hosts), key=HOST_ORDER.index)`). A host
/// name [`crate::hosts::order_index`] does not recognize sorts last,
/// after every documented host, rather than panicking -- defensive,
/// since every caller here only ever passes names drawn from
/// [`crate::hosts::HOSTS`] itself.
fn sorted_unique_hosts(hosts: &[&'static str]) -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    for &host in hosts {
        if !seen.contains(&host) {
            seen.push(host);
        }
    }
    seen.sort_by_key(|host| hosts::order_index(host).unwrap_or(usize::MAX));
    seen
}

/// `"Claude Code, Codex"`: `hosts`, de-duplicated and table-ordered, then
/// comma-joined (the prototype's `host_list`).
fn host_list(hosts: &[&'static str]) -> String {
    sorted_unique_hosts(hosts).join(", ")
}

/// Builds one [`Warning`], sorting and de-duplicating `hosts` the same
/// way [`host_list`] renders them into `detail`, so the two can never
/// drift apart (the prototype's `make_warning`).
fn make_warning(code: WarningCode, hosts: &[&'static str], detail: String) -> Warning {
    Warning {
        code,
        hosts: sorted_unique_hosts(hosts),
        detail,
    }
}

/// `n`, with a comma every three digits from the right (the prototype's
/// `f"{n:,}"`). `n` is always a byte count here, never negative, so no
/// sign handling is needed.
fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().rev().enumerate() {
        if index > 0 && index.is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped.chars().rev().collect()
}

/// `"\"a\", \"b\""`: each distinct word of `words` once, double-quoted,
/// in first-seen order (the prototype's `quoted_words`). Neither the
/// word nor the surrounding quotes are escaped, matching the prototype
/// exactly -- [`boundary_import_warnings`] is this module's only caller,
/// and a quoted token never itself holds a double quote (it is built
/// from [`crate::skills_safety::unsafe_at_tokens`], whose shown window
/// stops at whitespace, never at a quote character).
fn quoted_words(words: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for word in words {
        if !seen.contains(&word.as_str()) {
            seen.push(word.as_str());
        }
    }
    seen.into_iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `text`'s lines outside fenced code, fence delimiter lines themselves
/// excluded (the prototype's `unfenced`, applied to `text.split("\n")`).
/// Uses [`FenceTracker`] -- `CommonMark`-precise, a deliberate, documented
/// deviation from the prototype's own any-delimiter toggle (see
/// `fences.rs`'s module documentation); no prototype fixture this module
/// translates tells the two apart.
fn unfenced_lines(text: &str) -> Vec<&str> {
    let mut tracker = FenceTracker::new();
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if !tracker.advance(line.trim()) {
            lines.push(line);
        }
    }
    lines
}

/// Every inline-code span in `line`, as `(start, end)` char-index ranges
/// with `end` one past the closing backtick (the prototype's
/// `INLINE_CODE` regex `` `[^`]*` ``, matched left to right and
/// non-overlapping: a backtick with no closing backtick after it opens
/// no span at all).
fn inline_code_spans(line: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < line.len() {
        if line[index] == '`'
            && let Some(offset) = line[index + 1..].iter().position(|&c| c == '`')
        {
            let close = index + 1 + offset;
            spans.push((index, close + 1));
            index = close + 1;
            continue;
        }
        index += 1;
    }
    spans
}

/// `line` with every inline-code span's bytes replaced by as many
/// backticks (the prototype's `masked_inline_code`): positions stay the
/// same, so an `@` right after a span still follows the same symbol it
/// would in the unmasked line, while an `@` strictly inside a span's
/// content is itself replaced and so can never be found afterwards.
fn masked_inline_code(line: &[char]) -> Vec<char> {
    let mut masked = line.to_vec();
    for (start, end) in inline_code_spans(line) {
        for slot in &mut masked[start..end] {
            *slot = '`';
        }
    }
    masked
}

/// `line` with every inline-code span removed entirely, shortening it
/// (the prototype's `INLINE_CODE.sub("", line)`): used only where a
/// caller needs to know whether an `@`-import pattern exists outside
/// code at all, never a position back into the original line (contrast
/// [`masked_inline_code`], which [`boundary_import_warnings`] needs for
/// exactly that position-preserving reason).
fn remove_inline_code(line: &[char]) -> Vec<char> {
    let mut kept = Vec::new();
    let mut cursor = 0;
    for (start, end) in inline_code_spans(line) {
        kept.extend_from_slice(&line[cursor..start]);
        cursor = end;
    }
    kept.extend_from_slice(&line[cursor..]);
    kept
}

/// True when `c` is whitespace the way Python's `\s` judges it for a
/// `str` pattern (the prototype's bare `\s` inside `IMPORT_PATTERN`,
/// contrasted with [`crate::skills_safety`]'s own `JS_SPACE`, which is a
/// deliberately different set the host's own `@`-import pattern uses):
/// every Unicode `White_Space` character -- [`char::is_whitespace`]
/// already implements exactly that property -- plus `\x1c`-`\x1f`,
/// which `CPython`'s `\s` treats as whitespace for historical reasons
/// though they carry no Unicode whitespace property. Unlike `JS_SPACE`,
/// this set excludes `U+FEFF` (BOM): `CPython`'s `\s` does not treat it
/// as whitespace either.
fn is_regex_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Every match of the prototype's `IMPORT_PATTERN`
/// (`` (?<!\S)@([^\s`@]+\.md)(?!\S) ``) in `line`: an `@` not preceded by
/// a non-whitespace character, followed by one or more characters that
/// are none of whitespace, a backtick, or `@`, ending in a literal
/// `.md`, with whitespace or the end of `line` right after. Returns each
/// match's `@` index and the index right after its `.md`, non-overlapping
/// and left to right, exactly as `re.finditer` would scan this pattern.
///
/// The only valid match at a given `@` is the *whole* contiguous run of
/// allowed characters after it: any shorter prefix would still have a
/// non-whitespace character of that same run right after it, which the
/// trailing `(?!\S)` always refuses. So this never needs the pattern's
/// own backtracking -- it simply takes the full run and checks whether
/// *that* ends in `.md`.
fn import_pattern_matches(line: &[char]) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut index = 0;
    while index < line.len() {
        if line[index] != '@' {
            index += 1;
            continue;
        }
        let preceded_ok = index == 0 || is_regex_space(line[index - 1]);
        let mut end = index + 1;
        while end < line.len() && !is_regex_space(line[end]) && line[end] != '`' && line[end] != '@'
        {
            end += 1;
        }
        let followed_ok = end == line.len() || is_regex_space(line[end]);
        let group_len = end - (index + 1);
        let ends_md =
            group_len >= 4 && line[end - 3] == '.' && line[end - 2] == 'm' && line[end - 1] == 'd';
        if preceded_ok && followed_ok && ends_md {
            matches.push((index, end));
            index = end;
        } else {
            index += 1;
        }
    }
    matches
}

/// A word character the way Python's `\w` judges it for a `str` pattern
/// (Unicode-aware by default): a letter or digit, Unicode included, or
/// `_`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True when `relative` appears in `text` as a whole token: the
/// prototype's `mentions`, whose pattern is
/// `` (?<![\w.-]){name}(?![\w-]) `` with `name` a literal (escaped)
/// string, not a sub-pattern -- so this hand-translation only needs a
/// literal substring search plus the two character-class lookarounds,
/// never a general regex engine.
fn mentions(text: &str, relative: &str) -> bool {
    if relative.is_empty() {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = relative.chars().collect();
    if needle.len() > chars.len() {
        return false;
    }
    for start in 0..=chars.len() - needle.len() {
        if chars[start..start + needle.len()] != needle[..] {
            continue;
        }
        let before_ok = start == 0 || {
            let before = chars[start - 1];
            !(is_word_char(before) || before == '.' || before == '-')
        };
        let after_index = start + needle.len();
        let after_ok = after_index == chars.len() || {
            let after = chars[after_index];
            !(is_word_char(after) || after == '-')
        };
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Exposure and shadowing warnings for every ancestor in `chain` (the
/// prototype's `ancestor_warnings`): a host that loads one of
/// [`crate::hosts::upward_loaders`]'s file kinds from an ancestor is
/// "exposure" when this agent's own root also has that kind, or
/// "shadowing" when it does not. `OpenCode`'s own upward walk -- the
/// first unresolved ancestor `AGENTS.md`, not every ancestor's -- is
/// handled apart, exactly once, matching the prototype's
/// `opencode_resolved` flag.
fn ancestor_warnings(identity_kinds: &[IdentityKind], chain: &[ChainAncestor<'_>]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let mut opencode_resolved = identity_kinds.contains(&IdentityKind::Agents);
    for ancestor in chain {
        let mut exposure: Vec<&'static str> = Vec::new();
        let mut shadowing: Vec<&'static str> = Vec::new();
        for (host, kinds) in hosts::upward_loaders() {
            if kinds
                .iter()
                .any(|kind| ancestor.identity_files.contains(kind))
            {
                let own = kinds.iter().any(|kind| {
                    identity_kinds
                        .iter()
                        .any(|owned| owned.file_name() == *kind)
                });
                if own {
                    exposure.push(host);
                } else {
                    shadowing.push(host);
                }
            }
        }
        if !opencode_resolved
            && ancestor
                .identity_files
                .contains(&IdentityKind::Agents.file_name())
        {
            shadowing.push("OpenCode");
            opencode_resolved = true;
        }
        let described = format!(
            "the ancestor agent {} ({})",
            ancestor.display,
            ancestor.identity_files.join(", ")
        );
        if !exposure.is_empty() {
            let detail = format!(
                "{described} is also loaded here by {}, next to this agent's own instructions ({SOURCE_NOTE})",
                host_list(&exposure)
            );
            warnings.push(make_warning(
                WarningCode::AncestorExposure,
                &exposure,
                detail,
            ));
        }
        if !shadowing.is_empty() {
            let detail = format!(
                "{described} is loaded here by {} instead of this agent's instructions: this folder has no file those hosts read ({SOURCE_NOTE})",
                host_list(&shadowing)
            );
            warnings.push(make_warning(
                WarningCode::AncestorShadowing,
                &shadowing,
                detail,
            ));
        }
    }
    warnings
}

/// `child_not_mentioned` warnings for every nested agent `nested` names
/// that this agent's own canonical `text` never mentions (the
/// prototype's `child_warnings`).
fn child_warnings(identity_kinds: &[IdentityKind], text: &str, nested: &[&str]) -> Vec<Warning> {
    let mut readers: Vec<&'static str> = Vec::new();
    for (host, kinds) in hosts::upward_loaders() {
        if kinds.iter().any(|kind| {
            identity_kinds
                .iter()
                .any(|owned| owned.file_name() == *kind)
        }) {
            readers.push(host);
        }
    }
    if identity_kinds.contains(&IdentityKind::Agents)
        || identity_kinds.contains(&IdentityKind::Claude)
    {
        readers.push("OpenCode");
    }
    let mut warnings = Vec::new();
    for &child in nested {
        if mentions(text, child) {
            continue;
        }
        let detail = format!(
            "the nested agent {child}/ is not mentioned in this agent's instructions; {} can load them inside {child}/, so this identity may apply there too ({SOURCE_NOTE})",
            host_list(&readers)
        );
        warnings.push(make_warning(
            WarningCode::ChildNotMentioned,
            &readers,
            detail,
        ));
    }
    warnings
}

/// `host_imports` warnings: an import-only `AGENTS.override.md` Codex
/// prefers over `AGENTS.md`, and any `@path.md` the canonical text
/// holds outside fenced code and inline code that
/// [`crate::hosts::import_blind_readers`] would read as literal text
/// (the prototype's `import_warnings`).
fn import_warnings(override_bytes: Option<&[u8]>, text: Option<&str>) -> Vec<Warning> {
    let mut warnings = Vec::new();
    if override_bytes.is_some_and(is_import_only) {
        let detail = "AGENTS.override.md holds only @AGENTS.md imports; Codex prefers it over \
             AGENTS.md and does not document @ expansion, so Codex may read no instructions"
            .to_string();
        warnings.push(make_warning(WarningCode::HostImports, &["Codex"], detail));
    }
    if let Some(text) = text {
        let mut found: Vec<String> = Vec::new();
        for line in unfenced_lines(text) {
            let chars: Vec<char> = line.chars().collect();
            let removed = remove_inline_code(&chars);
            for (at, end) in import_pattern_matches(&removed) {
                let path: String = removed[at + 1..end].iter().collect();
                found.push(format!("@{path}"));
            }
        }
        found.sort();
        found.dedup();
        if !found.is_empty() {
            let readers: Vec<&'static str> = hosts::import_blind_readers().collect();
            let detail = format!(
                "the canonical text imports {}; {} read AGENTS.md without documented @ expansion \
                 (OpenCode documents none), so they see only the literal line",
                found.join(", "),
                host_list(&readers)
            );
            warnings.push(make_warning(WarningCode::HostImports, &readers, detail));
        }
    }
    warnings
}

/// `host_imports_boundary` warnings: every `@` outside fenced code,
/// with inline-code spans masked, that
/// [`crate::skills_safety::unsafe_at_tokens`] judges Claude Code might
/// load as an import, excluding any already counted by
/// [`import_warnings`] (the prototype's `boundary_import_warnings`).
///
/// For each unfenced line, this neutralizes (to a backtick) exactly the
/// `@` characters that either sit strictly inside an inline-code span's
/// content, or start an [`import_pattern_matches`] match on the
/// backtick-masked line (position-stable, so the same indexes apply to
/// the unmasked line too) -- then hands the result straight to
/// [`unsafe_at_tokens`]. This reaches the same outcome as the
/// prototype's own index-then-reslice approach (classify on the masked
/// line, render the shown word from the unmasked one) without needing
/// index-level access to that module's private `unsafe_at_indexes`/
/// `at_token`, which are not part of this crate's public surface and
/// `skills_safety.rs` is outside this task's allowed edit surface. The
/// two approaches can diverge only if a neutralized `@` falls inside
/// *another*, separately-reported token's shown window (30 characters
/// before, 40 after) -- which never happens in any fixture this module
/// translates, since each neutralized `@` here sits alone on its own
/// line.
fn boundary_import_warnings(text: &str) -> Vec<Warning> {
    let mut words: Vec<String> = Vec::new();
    for line in unfenced_lines(text) {
        let original: Vec<char> = line.chars().collect();
        let masked = masked_inline_code(&original);
        let plain_starts: Vec<usize> = import_pattern_matches(&masked)
            .into_iter()
            .map(|(at, _)| at)
            .collect();
        let mut neutralized = original.clone();
        for (start, end) in inline_code_spans(&original) {
            for slot in &mut neutralized[start..end] {
                if *slot == '@' {
                    *slot = '`';
                }
            }
        }
        for at in plain_starts {
            if neutralized[at] == '@' {
                neutralized[at] = '`';
            }
        }
        let neutralized: String = neutralized.into_iter().collect();
        words.extend(unsafe_at_tokens(&neutralized, false));
    }
    if words.is_empty() {
        return Vec::new();
    }
    let detail = format!(
        "the canonical text holds {}: Claude Code may load this as an import (it matches an @ \
         after whitespace or at the start of an inline text token, which a code span, emphasis \
         or a link ends, with any file extension or none); other hosts treat it as text. It is \
         reported and never changed; to keep it as text, write \"at\" or rephrase ({SOURCE_NOTE})",
        quoted_words(&words)
    );
    vec![make_warning(
        WarningCode::HostImportsBoundary,
        &["Claude Code"],
        detail,
    )]
}

/// `size_codex`/`size_antigravity` warnings: the prototype's
/// `size_warnings` compares `len(text)` where `text` is the agent's raw
/// *bytes*, not its decoded form. This module's [`AgentInput::text`] is
/// already-decoded `&str`, but `str::len` in Rust already counts UTF-8
/// **bytes**, never code points, so `text.len()` here is the same
/// quantity the prototype compares -- provided the original bytes were
/// themselves valid UTF-8 (the only case this module's already-decoded
/// input can represent at all; a lossy decode of invalid UTF-8 would
/// shift this count slightly, a gap this module cannot observe from
/// decoded text alone).
fn size_warnings(text: &str) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let size = text.len();
    if size > hosts::CODEX_MAX_BYTES {
        let detail = format!(
            "the canonical text is {} bytes; Codex stops adding instruction files at 32 KiB ({} bytes) by default",
            format_count(size),
            format_count(hosts::CODEX_MAX_BYTES)
        );
        warnings.push(make_warning(WarningCode::SizeCodex, &["Codex"], detail));
    }
    if size > hosts::ANTIGRAVITY_MAX_BYTES {
        let detail = format!(
            "the canonical text is {} bytes; Antigravity truncates a rule file above {} bytes",
            format_count(size),
            format_count(hosts::ANTIGRAVITY_MAX_BYTES)
        );
        warnings.push(make_warning(
            WarningCode::SizeAntigravity,
            &["Antigravity"],
            detail,
        ));
    }
    warnings
}

/// Every warning for one agent root, ordered by `WARNING_ORDER` (the
/// prototype's `agent_warnings`).
#[must_use]
pub fn agent_warnings(input: &AgentInput<'_>) -> Vec<Warning> {
    let mut warnings = ancestor_warnings(input.identity_kinds, input.chain);
    if let Some(text) = input.text {
        warnings.extend(child_warnings(input.identity_kinds, text, input.nested));
    }
    warnings.extend(import_warnings(input.override_bytes, input.text));
    if let Some(text) = input.text {
        warnings.extend(boundary_import_warnings(text));
    }
    for &extra in input.host_extras {
        let Some(readers) = hosts::extra_readers(extra) else {
            // Defensive: the prototype's `EXTRA_READERS[extra]` would
            // raise `KeyError` for a name outside its table. A port is
            // expected to supply only recognized names; an unrecognized
            // one is silently skipped here rather than panicking.
            continue;
        };
        let detail = format!(
            "{extra} is read only by {}; it is reported and never changed",
            host_list(readers)
        );
        warnings.push(make_warning(WarningCode::ExtraHostFile, readers, detail));
    }
    if let Some(text) = input.text {
        warnings.extend(size_warnings(text));
    }
    for &name in hosts::GENERATED_STATE_DIRS {
        if !input.generated_state_dirs_present.contains(&name) {
            continue;
        }
        let detail = format!(
            "{name}/ holds generated tool state, not agent instructions; it is not part of the \
             portable definition and the adapter never changes it"
        );
        warnings.push(make_warning(WarningCode::GeneratedStateDir, &[], detail));
    }
    warnings.sort_by_key(|warning| warning.code.order_index());
    warnings
}

// `import_pattern_matches` and `make_warning` are private, so their own
// edge cases are unit-tested here directly rather than through
// `agent_warnings` -- the same in-file pattern `fences.rs` already uses
// for its own private-to-`pub(crate)` `FenceTracker`.
#[cfg(test)]
mod tests {
    use super::{WarningCode, import_pattern_matches, make_warning};

    fn matches_for(line: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = line.chars().collect();
        import_pattern_matches(&chars)
    }

    /// An `@` right after a closing backtick is not preceded by
    /// whitespace, so `IMPORT_PATTERN` never matches it there --
    /// `host_imports_boundary`, not `host_imports`, may still flag it.
    /// An `@` at the very start of the line counts as "not preceded by
    /// a non-whitespace character".
    #[test]
    fn requires_whitespace_or_start_before_the_at() {
        assert!(matches_for("`code`@notes.md").is_empty());
        assert_eq!(matches_for("@notes.md").len(), 1);
    }

    #[test]
    fn refuses_an_at_inside_a_word() {
        assert!(matches_for("ops@example.com").is_empty());
    }

    #[test]
    fn requires_a_literal_md_suffix() {
        assert!(matches_for(" @notes.txt").is_empty());
        assert!(matches_for(" @notes.mdx").is_empty());
    }

    /// Trailing punctuation right after the `.md` is non-whitespace, so
    /// the whole match is refused, not just the punctuation.
    #[test]
    fn refuses_trailing_punctuation() {
        assert!(matches_for(" @notes.md!").is_empty());
        assert_eq!(matches_for(" @notes.md").len(), 1);
    }

    #[test]
    fn make_warning_deduplicates_and_sorts_hosts_by_table_order() {
        let warning = make_warning(
            WarningCode::HostImports,
            &["OpenCode", "Codex", "OpenCode"],
            "detail".to_string(),
        );
        assert_eq!(warning.hosts, vec!["Codex", "OpenCode"]);
    }
}
