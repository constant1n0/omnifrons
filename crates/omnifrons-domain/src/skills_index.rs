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

use std::borrow::Cow;

/// The file name every discovered or indexed skill ends in (the
/// prototype's `SKILL_FILE`).
const SKILL_FILE: &str = "SKILL.md";

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
    if chars.get(target_start) == Some(&'<') {
        let mut scan = target_start + 1;
        while chars.get(scan).is_some_and(|c| *c != '<' && *c != '>') {
            scan += 1;
        }
        if chars.get(scan) == Some(&'>')
            && let Some(matched) = finish_match(chars, target_start, scan + 1)
        {
            return Some(matched);
        }
        // Else: either a nested `<` (or no closing `>` at all) before
        // the first `>`, or a bracketed target that closes but whose
        // remainder never reaches `)` -- so this is not a valid
        // bracketed target after all. Fall through to the bare-target
        // branch below, starting over from `target_start` -- the same
        // `<` that opened the failed attempt is just an ordinary
        // character there (review follow-up, correction pass: the
        // regex alternation `(<[^<>]*>|[^()\s]+)` backtracks into its
        // second branch in both cases; an earlier version returned no
        // match at all instead).
    }
    let mut pos = target_start;
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
    finish_match(chars, target_start, pos)
}

/// The optional quoted title and the closing `)` that follow a
/// matched target, shared by both of [`match_link`]'s branches:
/// returns the target text (`chars[target_start..target_end]`) and the
/// index just past the closing `)`, or `None` when the remainder does
/// not reach it.
fn finish_match(chars: &[char], target_start: usize, target_end: usize) -> Option<(String, usize)> {
    let target: String = chars[target_start..target_end].iter().collect();
    let after_target = target_end;
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

/// A leading `//?/` segment -- the slash-normalized form of Windows'
/// own `\\?\` verbatim prefix, which disables further parsing of the
/// string that follows it -- stripped, with one further rewrite: a
/// verbatim UNC root's own real spelling is `//?/UNC/server/share/...`
/// (the slash-normalized form of `\\?\UNC\server\share\...`), never a
/// flat `//?/server/share/...`, so stripping only `//?/` would leave
/// `UNC/server/share/...` -- no leading `/` at all, which
/// [`is_absolute_path`] then rejects as not absolute. This rewrites
/// that shape to the ordinary `//server/share/...` one before anything
/// else inspects it. A verbatim drive root (`//?/C:/...`) needs no such
/// rewrite and comes back as the plain stripped slice, exactly as
/// before. [`relative_to_root`] and [`indexed_skill_targets`] both
/// apply this, from the root and the target alike, so a caller may
/// render a canonical root or a Markdown link with or without either
/// verbatim spelling and the two forms relativize the same way.
/// Borrowed in every case except the UNC rewrite, which must allocate:
/// no substring of `path` itself spells `//server/share/...`, since the
/// two slashes that open it are never adjacent in the verbatim original.
fn strip_verbatim_prefix(path: &str) -> Cow<'_, str> {
    let Some(rest) = path.strip_prefix("//?/") else {
        return Cow::Borrowed(path);
    };
    match rest.strip_prefix("UNC/") {
        Some(after_unc) => Cow::Owned(format!("//{after_unc}")),
        None => Cow::Borrowed(rest),
    }
}

/// `path` is a bare Windows drive letter component (`"C:"`, any ASCII
/// letter, either case) -- the shape a drive-rooted path's first
/// `/`-separated segment takes, never a later one.
fn is_drive_letter_component(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `path`, with any leading `//?/` already stripped, starts with a
/// Windows drive letter followed by `/` (`"C:/..."`, any ASCII letter,
/// either case).
fn is_drive_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

/// Whether `path`, lexically, is absolute: a POSIX path rooted at `/`
/// (a UNC path such as `//server/share/...` already satisfies this,
/// since it too starts with `/`), or a Windows drive path (`X:/...`).
/// [`strip_verbatim_prefix`] is applied first, so a `//?/`-prefixed
/// spelling of either form -- including a verbatim UNC root's own,
/// differently-shaped `//?/UNC/server/share/...` -- is recognized the
/// same way as the unprefixed one.
fn is_absolute_path(path: &str) -> bool {
    let path = strip_verbatim_prefix(path);
    path.starts_with('/') || is_drive_absolute(&path)
}

/// Splits an absolute path (POSIX, Windows drive, or UNC; see
/// [`is_absolute_path`]) into its normalized components, the way
/// `posixpath.normpath` normalizes one that is already absolute: a
/// `.` segment and a repeated `/` are dropped, and a `..` drops the
/// component before it when there is one, or is itself dropped
/// silently when there is not -- an absolute path can never retain a
/// leading `..` the way a relative one does, since there is nothing
/// above its root to name. A leading `//?/` is normalized first,
/// exactly as [`is_absolute_path`] normalizes it before judging --
/// including a verbatim UNC root's own `//?/UNC/server/share/...`
/// shape, rewritten to the ordinary `//server/share/...` one
/// ([`strip_verbatim_prefix`]). A UNC path's own doubled leading `/`
/// collapses the same way any other doubled `/` does, since both its
/// empty segments are dropped alongside it. Each component is owned
/// rather than borrowed from `path`: [`strip_verbatim_prefix`]'s own
/// verbatim-UNC rewrite may need to allocate a new string with no
/// substring of the original `path` to borrow a component from.
/// [`normalize_relative_target`] is the relative-path twin of this
/// helper and keeps an unresolvable leading `..` for exactly that
/// reason.
fn normalized_absolute_components(path: &str) -> Vec<String> {
    let path = strip_verbatim_prefix(path);
    let mut out: Vec<String> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other.to_string()),
        }
    }
    out
}

/// Which filesystem mount an absolute path belongs to: a bare POSIX
/// path (`/`-rooted, not UNC), a Windows drive letter (normalized to
/// lowercase, matching [`is_drive_letter_component`]'s own
/// case-insensitivity), or a UNC `//server/share` root (its server and
/// share both normalized to lowercase -- a real UNC root is
/// case-insensitive the same way a drive letter is). [`relative_to_root`]
/// relativizes a target only when [`mount_of`] agrees the two paths
/// share a mount: `ntpath.relpath` itself raises `ValueError` across a
/// mismatch (its own `splitdrive` plus `normcase`; see
/// `relative_to_root`'s own unit tests for the oracle's exact error
/// text), which this port has no error channel to reproduce, so it
/// falls back to the target unchanged instead -- the same "no
/// meaningful position to relativize against" policy this function's
/// own doc comment already documents for a non-absolute root.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mount {
    /// A `/`-rooted path that is not a UNC root.
    Posix,
    /// A Windows drive letter, lowercased.
    Drive(char),
    /// A UNC root's server and share, both lowercased.
    Unc(String, String),
}

/// `path`'s [`Mount`] ([`strip_verbatim_prefix`] applied first, exactly
/// as every sibling classifier in this module applies it first).
/// `path` is assumed already [`is_absolute_path`] -- every caller here
/// checks that before calling this function, so this function never
/// needs to itself.
fn mount_of(path: &str) -> Mount {
    let path = strip_verbatim_prefix(path);
    if is_drive_absolute(&path) {
        return Mount::Drive(char::from(path.as_bytes()[0].to_ascii_lowercase()));
    }
    // `is_absolute_path` guarantees a leading `/` whenever
    // `is_drive_absolute` just failed; a UNC root's own doubled leading
    // `/` is the only thing that tells it apart from a bare POSIX root
    // here, since both otherwise share a single leading `/`.
    if let Some(rest) = path.strip_prefix("//") {
        let mut parts = rest.splitn(3, '/');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        if !server.is_empty() && !share.is_empty() {
            return Mount::Unc(server.to_lowercase(), share.to_lowercase());
        }
    }
    Mount::Posix
}

/// Whether `root`'s and `target`'s components at the same position
/// match: exact equality, except the very first position (`is_first`)
/// compares case-insensitively when both components are a Windows
/// drive letter ([`is_drive_letter_component`]) -- `ntpath.relpath`
/// itself treats `C:` and `c:` as the same mount. Every other
/// position, including a UNC host or share name, compares exactly,
/// like every POSIX component already does.
fn components_match(a: &str, b: &str, is_first: bool) -> bool {
    if is_first && is_drive_letter_component(a) && is_drive_letter_component(b) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The number of `root_components`/`target_components` that match from
/// the start ([`components_match`]), stopping at the first mismatch or
/// the shorter list's end.
fn common_prefix_len(root_components: &[String], target_components: &[String]) -> usize {
    let mut common = 0;
    while common < root_components.len() && common < target_components.len() {
        if !components_match(
            &root_components[common],
            &target_components[common],
            common == 0,
        ) {
            break;
        }
        common += 1;
    }
    common
}

/// The path of an absolute `target` relative to an absolute `root`,
/// lexically (the prototype's `os.path.relpath(target, root)`, which
/// normalizes both paths before diffing them so the result never
/// depends on a real filesystem): both paths' normalized components
/// are compared left to right ([`common_prefix_len`]); the result is
/// one `..` for every `root` component past the common prefix, then
/// every `target` component past it, joined with `/`; `.` when the two
/// paths are equal. `root` is the caller's own scanned workspace root,
/// with or without a trailing `/`, never resolved or read from a
/// filesystem here.
///
/// "Absolute" extends past a POSIX `/`-rooted path (the prototype has
/// no equivalent, since its own workspace root is always POSIX): a
/// Windows drive path (`X:/...`, any ASCII letter, either case) and a
/// UNC path (`//server/share/...`) both count too
/// ([`is_absolute_path`]), and either form may carry a leading `//?/`
/// verbatim prefix -- a drive's own flat `//?/C:/...`, or a UNC root's
/// own, differently-shaped `//?/UNC/server/share/...` -- normalized
/// from `root` and `target` alike before anything else is compared
/// ([`strip_verbatim_prefix`]). Verified against Python's
/// `ntpath.relpath` -- see this function's own unit tests for the exact
/// oracle outputs.
///
/// `target` is relativized only when it shares `root`'s own [`Mount`]
/// ([`mount_of`]): `ntpath.relpath` itself raises `ValueError` across
/// two different mounts (its own `splitdrive` plus `normcase`), which
/// this port has no error channel to reproduce, so a cross-mount
/// `target` instead comes back unchanged -- a deliberate port policy,
/// not a gap, and this function's own unit tests cite the oracle's
/// exact error text for each mount pairing it refuses to relativize.
///
/// When `root` is not itself an absolute path (empty, or a relative
/// path such as `"workspace"`), there is no meaningful position left
/// to relativize `target` against: treating an empty `root` as having
/// zero components, or a relative one as though it were anchored at
/// the same place as `target`, both only produce a plausible-looking
/// but wrong path (review follow-up, close it). This function instead
/// treats a non-absolute `root` as no root at all and returns `target`
/// unchanged -- every caller's own fallback for an absolute target
/// before this function existed to relativize one -- rather than ever
/// panicking: the domain never panics on a caller's input.
fn relative_to_root(target: &str, root: &str) -> String {
    if !is_absolute_path(root) {
        return target.to_string();
    }
    if mount_of(target) != mount_of(root) {
        return target.to_string();
    }
    let root_components = normalized_absolute_components(root);
    let target_components = normalized_absolute_components(target);
    let common = common_prefix_len(&root_components, &target_components);
    let mut out: Vec<&str> =
        Vec::with_capacity((root_components.len() - common) + (target_components.len() - common));
    out.extend(std::iter::repeat_n("..", root_components.len() - common));
    out.extend(target_components[common..].iter().map(String::as_str));
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}

/// Normalizes a linked `SKILL.md` target the way the prototype's
/// `indexed_skills` does for a path it already knows is relative
/// (`posixpath.normpath`): collapses `.` segments and repeated `/`, and
/// resolves `a/../` by dropping the segment before it, keeping an
/// unresolvable leading `..` as the library call does. A target that
/// looks absolute (starts with `/`) is returned unchanged: this pure
/// function takes no root, so it cannot relativize one --
/// [`indexed_skill_targets`] does that itself, against its own `root`
/// parameter, calling this function only for a target that is not
/// absolute.
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
/// surviving target is normalized -- relativized against `root` when
/// it is itself absolute ([`relative_to_root`]), through
/// [`normalize_relative_target`] otherwise, exactly as the prototype's
/// `indexed_skills(root, data)` branches on `posixpath.isabs(target)`.
/// Sorted and de-duplicated, as the prototype's own `sorted(set(...))`
/// is. A hand-written index row in a project's own text is exactly one
/// of these targets; this function does not care whether the row was
/// written by a person or generated by Omnifrons.
///
/// `root` is the caller's scanned workspace root: an absolute path --
/// POSIX (`/workspace`), a Windows drive (`C:/workspace`, either
/// letter case), or UNC (`//server/share`) -- with or without a
/// trailing `/`, and optionally carrying a leading `//?/` verbatim
/// prefix: a drive's own flat `//?/C:/workspace`, or a UNC root's own,
/// differently-shaped `//?/UNC/server/share` ([`strip_verbatim_prefix`]
/// normalizes either spelling the same way before anything classifies
/// it). It is used only to relativize an absolute target lexically
/// ([`relative_to_root`]); a target is judged absolute by that same
/// extended rule, not only a leading `/`, so a Windows-rooted link
/// relativizes against a Windows-rooted root exactly as a POSIX link
/// already does against a POSIX root ([`is_absolute_path`]), and a
/// target on a different mount than `root` is never relativized at all
/// ([`relative_to_root`]'s own documented policy). This function never
/// reads a filesystem. A non-absolute `root` (empty, or a relative
/// path) is treated as no root at all, so an absolute target passes
/// through unrelativized -- [`relative_to_root`] documents the
/// fallback -- and the same holds for [`read_skill_index`] and
/// [`rows_to_generate`] below, which both thread `root` straight
/// through to this function.
#[must_use]
pub fn indexed_skill_targets(text: &[u8], root: &str) -> Vec<String> {
    let decoded = String::from_utf8_lossy(text);
    let mut fence = crate::fences::FenceTracker::new();
    let mut targets: Vec<String> = Vec::new();
    for raw_line in decoded.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if fence.advance(trimmed) {
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
            let normalized = if is_absolute_path(&target) {
                relative_to_root(&target, root)
            } else {
                normalize_relative_target(&target)
            };
            targets.push(normalized);
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
/// `missing`/`dangling` pair for the skill it actually names.
/// [`SkillIndexStatus::missing`] is the normalized `discovered` minus
/// [`indexed_skill_targets`], in `discovered`'s own order;
/// [`SkillIndexStatus::dangling`] is the indexed targets minus the
/// normalized `discovered`, in their sorted order. `root` is threaded
/// straight through to [`indexed_skill_targets`] to relativize an
/// absolute link; `discovered` itself is always relative (a project's
/// own discovery already reports paths relative to its root), so it is
/// never relativized here.
#[must_use]
pub fn read_skill_index(text: &[u8], discovered: &[String], root: &str) -> SkillIndexStatus {
    let mut normalized_discovered: Vec<String> = Vec::with_capacity(discovered.len());
    for path in discovered {
        let normalized = normalize_relative_target(path);
        if !normalized_discovered.contains(&normalized) {
            normalized_discovered.push(normalized);
        }
    }
    let indexed = indexed_skill_targets(text, root);
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
/// with the row-reading `plan_block` does for each one it keeps). Each
/// row's path is normalized through [`normalize_relative_target`] and
/// de-duplicated by that normalized path (first occurrence kept)
/// before comparing against [`indexed_skill_targets`]'s own normalized
/// output, exactly as [`read_skill_index`] already normalizes
/// `discovered` -- otherwise a `./`-prefixed or repeated path would
/// compare unequal to an already-covered target and be regenerated as
/// a duplicate link (review follow-up, correction pass). This function
/// only filters the rows it is handed: discovering a project's skills
/// in the first place, and attributing a nested project's skills to
/// their own agent rather than its parent's, is ADR-0005 sub-slice 1e's
/// job, not this one's. `root` is threaded straight through to
/// [`indexed_skill_targets`] to relativize an absolute link already
/// covering a row.
#[must_use]
pub fn rows_to_generate(prefix_text: &[u8], discovered: &[SkillRow], root: &str) -> Vec<SkillRow> {
    let covered = indexed_skill_targets(prefix_text, root);
    let mut seen: Vec<String> = Vec::with_capacity(discovered.len());
    let mut normalized: Vec<SkillRow> = Vec::with_capacity(discovered.len());
    for row in discovered {
        let path = normalize_relative_target(&row.path);
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        normalized.push(SkillRow {
            name: row.name.clone(),
            description: row.description.clone(),
            path,
            had_undecodable_bytes: row.had_undecodable_bytes,
        });
    }
    normalized.retain(|row| !covered.contains(&row.path));
    normalized
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
    /// True when decoding this skill's
    /// `SKILL.md` bytes to UTF-8 needed to replace at least one byte
    /// that was not valid UTF-8 ([`crate::skills_safety::SkillMetadata::had_undecodable_bytes`],
    /// carried through unchanged). [`crate::skills_safety::unsafe_row_fields`]
    /// conditions its undecodable-bytes note on this flag together
    /// with whether the specific unsafe field's own value contains
    /// `U+FFFD`: this flag alone is set for the *whole* file, so a
    /// field whose invalid byte sits elsewhere in the file (a sibling
    /// field, or the body past the front matter) must not be
    /// decorated with a note about bytes it never actually held. A
    /// legitimate, validly-encoded `U+FFFD` sets this flag to `false`.
    pub had_undecodable_bytes: bool,
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

/// The link target for a row's path (the prototype's `link_target`,
/// deliberately diverging from it in one way): `%` and `#` are
/// percent-encoded first -- `%` to `%25`, then `#` to `%23`, in that
/// order, so encoding `#` never introduces a `%` that gets re-encoded
/// -- then the result is wrapped in angle brackets when it holds
/// whitespace or `(`, `)`, `<`, or `>`, the characters that would
/// otherwise break a bare Markdown link target. The prototype's
/// `link_target` encodes neither `%` nor `#`, so a path such as
/// `skills/c#/SKILL.md` renders a link its own reader (and this
/// port's [`indexed_skill_targets`]) always misreads: a bare `#` is
/// read as a fragment separator and truncates the target right there.
/// Encoding both here, and decoding them back on the read side exactly
/// as any other `%XX` escape ([`percent_decode`]), closes that gap.
#[must_use]
pub fn link_target(path: &str) -> String {
    let escaped = path.replace('%', "%25").replace('#', "%23");
    if escaped
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'))
    {
        format!("<{escaped}>")
    } else {
        escaped
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

#[cfg(test)]
mod tests {
    use super::relative_to_root;

    /// `relative_to_root`'s own empty-result case: a target equal to
    /// `root` relativizes to `.`. Reachable only by calling this
    /// private helper directly -- every target `indexed_skill_targets`
    /// recognizes as a skill link already ends in `SKILL.md`, so it can
    /// never equal a bare root path through that public API. Verified
    /// with `python3 -c "import os;
    /// print(os.path.relpath('/workspace', '/workspace'))"`, which
    /// prints `.`.
    #[test]
    fn relative_to_root_of_a_target_equal_to_the_root_is_dot() {
        assert_eq!(relative_to_root("/workspace", "/workspace"), ".");
    }

    /// A target inside `root` relativizes to the path past their
    /// common prefix. Verified with `python3 -c "import os;
    /// print(os.path.relpath('/workspace/skills/x/SKILL.md',
    /// '/workspace'))"`, which prints `skills/x/SKILL.md`.
    #[test]
    fn relative_to_root_of_a_target_inside_the_root() {
        assert_eq!(
            relative_to_root("/workspace/skills/x/SKILL.md", "/workspace"),
            "skills/x/SKILL.md"
        );
    }

    /// A target outside `root` relativizes to the `../`-prefixed form.
    /// Verified with `python3 -c "import os;
    /// print(os.path.relpath('/elsewhere/skills/x/SKILL.md',
    /// '/workspace'))"`, which prints `../elsewhere/skills/x/SKILL.md`.
    #[test]
    fn relative_to_root_of_a_target_outside_the_root() {
        assert_eq!(
            relative_to_root("/elsewhere/skills/x/SKILL.md", "/workspace"),
            "../elsewhere/skills/x/SKILL.md"
        );
    }

    /// A trailing slash on `root` (documented above as allowed)
    /// changes nothing for an inside target. Verified with
    /// `python3 -c "import os;
    /// print(os.path.relpath('/workspace/skills/x/SKILL.md',
    /// '/workspace/'))"`, which prints `skills/x/SKILL.md` -- the same
    /// as without the trailing slash.
    #[test]
    fn relative_to_root_with_a_trailing_slash_on_the_root_for_an_inside_target() {
        assert_eq!(
            relative_to_root("/workspace/skills/x/SKILL.md", "/workspace/"),
            "skills/x/SKILL.md"
        );
    }

    /// The same trailing slash changes nothing for an outside target.
    /// Verified with `python3 -c "import os;
    /// print(os.path.relpath('/elsewhere/skills/x/SKILL.md',
    /// '/workspace/'))"`, which prints
    /// `../elsewhere/skills/x/SKILL.md` -- unchanged.
    #[test]
    fn relative_to_root_with_a_trailing_slash_on_the_root_for_an_outside_target() {
        assert_eq!(
            relative_to_root("/elsewhere/skills/x/SKILL.md", "/workspace/"),
            "../elsewhere/skills/x/SKILL.md"
        );
    }

    /// A `root` holding dot segments normalizes before the comparison,
    /// exactly like a target does: `/workspace/./sub/../` resolves to
    /// `/workspace`, so an inside target relativizes the same as
    /// against the plain root. Verified with
    /// `python3 -c "import os;
    /// print(os.path.relpath('/workspace/skills/x/SKILL.md',
    /// '/workspace/./sub/../'))"`, which prints `skills/x/SKILL.md`.
    #[test]
    fn relative_to_root_with_dot_segments_in_the_root_for_an_inside_target() {
        assert_eq!(
            relative_to_root("/workspace/skills/x/SKILL.md", "/workspace/./sub/../"),
            "skills/x/SKILL.md"
        );
    }

    /// The same dot-segment root normalization applies for an outside
    /// target. Verified with `python3 -c "import os;
    /// print(os.path.relpath('/elsewhere/skills/x/SKILL.md',
    /// '/workspace/./sub/../'))"`, which prints
    /// `../elsewhere/skills/x/SKILL.md`.
    #[test]
    fn relative_to_root_with_dot_segments_in_the_root_for_an_outside_target() {
        assert_eq!(
            relative_to_root("/elsewhere/skills/x/SKILL.md", "/workspace/./sub/../"),
            "../elsewhere/skills/x/SKILL.md"
        );
    }

    /// Review follow-up (behavior change): `relative_to_root` documents
    /// its `root` as an absolute POSIX path but never checked that
    /// before this fix. An empty root has no leading `/` either, so it
    /// is "no root" -- there is nothing to relativize the target
    /// against -- and the function falls back to the target unchanged
    /// rather than guessing a lexically-wrong relativization. Before
    /// this fix, an empty root made every root component vanish, so
    /// the result silently dropped the target's own leading `/`
    /// (`"workspace/skills/x/SKILL.md"`, not the absolute path it
    /// started from) -- a plausible-looking but wrong path.
    #[test]
    fn relative_to_root_with_an_empty_root_falls_back_to_the_target_unchanged() {
        assert_eq!(
            relative_to_root("/workspace/skills/x/SKILL.md", ""),
            "/workspace/skills/x/SKILL.md"
        );
    }

    /// The same fallback applies to a `root` that is a relative path
    /// rather than an absolute one: before this fix, a relative root
    /// such as `"workspace"` happened to share a leading component with
    /// the target's own normalized components and was silently treated
    /// as if it meant `/workspace`, again producing a plausible-looking
    /// but wrong relativization instead of the documented fallback.
    #[test]
    fn relative_to_root_with_a_relative_root_falls_back_to_the_target_unchanged() {
        assert_eq!(
            relative_to_root("/workspace/skills/x/SKILL.md", "workspace"),
            "/workspace/skills/x/SKILL.md"
        );
    }

    /// A Windows drive-absolute target inside a drive root relativizes
    /// the same way a POSIX target does. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\workspace\skills\x\SKILL.md',
    /// r'C:\workspace'))"`, which prints `skills\x\SKILL.md`
    /// (backslashes converted to `/` here, as every other path in this
    /// module is already POSIX-style).
    #[test]
    fn relative_to_root_of_a_drive_absolute_target_inside_the_root() {
        assert_eq!(
            relative_to_root("C:/workspace/skills/x/SKILL.md", "C:/workspace"),
            "skills/x/SKILL.md"
        );
    }

    /// A drive-absolute target outside the root relativizes to the
    /// `../`-prefixed form. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\elsewhere\skills\x\SKILL.md',
    /// r'C:\workspace'))"`, which prints
    /// `..\elsewhere\skills\x\SKILL.md`.
    #[test]
    fn relative_to_root_of_a_drive_absolute_target_outside_the_root() {
        assert_eq!(
            relative_to_root("C:/elsewhere/skills/x/SKILL.md", "C:/workspace"),
            "../elsewhere/skills/x/SKILL.md"
        );
    }

    /// A drive-absolute target equal to the root relativizes to `.`.
    /// Verified with `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\workspace', r'C:\workspace'))"`, which
    /// prints `.`.
    #[test]
    fn relative_to_root_of_a_drive_absolute_target_equal_to_the_root() {
        assert_eq!(relative_to_root("C:/workspace", "C:/workspace"), ".");
    }

    /// The drive letter compares case-insensitively: a lowercase root
    /// drive still relativizes an uppercase target drive. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\workspace\skills\x\SKILL.md',
    /// r'c:\workspace'))"`, which still prints `skills\x\SKILL.md` --
    /// `ntpath.relpath` itself treats the two drive spellings as the
    /// same mount.
    #[test]
    fn relative_to_root_matches_a_mixed_case_drive_letter() {
        assert_eq!(
            relative_to_root("C:/workspace/skills/x/SKILL.md", "c:/workspace"),
            "skills/x/SKILL.md"
        );
    }

    /// A `//?/`-prefixed root (the slash-normalized form of Windows'
    /// `\\?\` verbatim prefix) strips that prefix before anything else
    /// is compared, so it relativizes a plain-spelled target exactly as
    /// the unprefixed root would. Verified by reusing the plain-root
    /// oracle above -- `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\workspace\skills\x\SKILL.md',
    /// r'C:\workspace'))"` prints `skills\x\SKILL.md` -- since calling
    /// `ntpath.relpath` with the verbatim prefix on only one side raises
    /// (`path is on mount 'C:', start on mount ''`), confirming the
    /// prefix must be stripped before comparing, never passed through.
    #[test]
    fn relative_to_root_strips_a_verbatim_prefixed_root() {
        assert_eq!(
            relative_to_root("C:/workspace/skills/x/SKILL.md", "//?/C:/workspace"),
            "skills/x/SKILL.md"
        );
    }

    /// A UNC root (`//server/share`) is already POSIX-absolute by its
    /// own leading `/`, and a target inside it relativizes the same way
    /// a drive-rooted one does. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'\\server\share\skills\x\SKILL.md',
    /// r'\\server\share'))"`, which prints `skills\x\SKILL.md`.
    #[test]
    fn relative_to_root_of_a_unc_target_inside_the_root() {
        assert_eq!(
            relative_to_root("//server/share/skills/x/SKILL.md", "//server/share"),
            "skills/x/SKILL.md"
        );
    }

    /// A UNC target outside the root, but under the same server and
    /// share, relativizes to the `../`-prefixed form. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'\\server\share\elsewhere\SKILL.md',
    /// r'\\server\share\workspace'))"`, which prints
    /// `..\elsewhere\SKILL.md`.
    #[test]
    fn relative_to_root_of_a_unc_target_outside_the_root() {
        assert_eq!(
            relative_to_root(
                "//server/share/elsewhere/SKILL.md",
                "//server/share/workspace"
            ),
            "../elsewhere/SKILL.md"
        );
    }

    /// A UNC target equal to the root relativizes to `.`. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'\\server\share', r'\\server\share'))"`,
    /// which prints `.`.
    #[test]
    fn relative_to_root_of_a_unc_target_equal_to_the_root() {
        assert_eq!(relative_to_root("//server/share", "//server/share"), ".");
    }

    /// Review follow-up (behavior change, cross-mount targets): a
    /// drive-absolute target and a POSIX root are on different mounts
    /// and must never be relativized against each other -- the same
    /// "no meaningful position to relativize against" fallback this
    /// function's own doc comment already documents for a non-absolute
    /// root, extended to a cross-mount pair of absolute ones. Verified
    /// with `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'C:\x\SKILL.md', r'\workspace'))"`, which
    /// raises `ValueError: path is on mount 'C:', start on mount ''`.
    #[test]
    fn relative_to_root_of_a_drive_absolute_target_against_a_posix_root_is_unchanged() {
        assert_eq!(
            relative_to_root("C:/x/SKILL.md", "/workspace"),
            "C:/x/SKILL.md"
        );
    }

    /// The reverse: a POSIX-absolute target against a drive root is
    /// also on a different mount. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'\x\SKILL.md', r'C:\workspace'))"`, which
    /// raises `ValueError: path is on mount '', start on mount 'C:'`.
    #[test]
    fn relative_to_root_of_a_posix_target_against_a_drive_root_is_unchanged() {
        assert_eq!(
            relative_to_root("/x/SKILL.md", "C:/workspace"),
            "/x/SKILL.md"
        );
    }

    /// Two different drive letters are two different mounts, not a
    /// single "drive" mount they both share. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'D:\x\SKILL.md', r'C:\workspace'))"`,
    /// which raises `ValueError: path is on mount 'D:', start on mount
    /// 'C:'`.
    #[test]
    fn relative_to_root_of_a_target_on_a_different_drive_letter_is_unchanged() {
        assert_eq!(
            relative_to_root("D:/x/SKILL.md", "C:/workspace"),
            "D:/x/SKILL.md"
        );
    }

    /// A UNC target under a different share is a different mount too,
    /// even sharing the same server. Verified with
    /// `python3 -c "import ntpath;
    /// print(ntpath.relpath(r'\\server\other\x\SKILL.md',
    /// r'\\server\share\workspace'))"`, which raises `ValueError: path
    /// is on mount '\\\\server\\other', start on mount
    /// '\\\\server\\share'`.
    #[test]
    fn relative_to_root_of_a_unc_target_on_a_different_share_is_unchanged() {
        assert_eq!(
            relative_to_root("//server/other/x/SKILL.md", "//server/share/workspace"),
            "//server/other/x/SKILL.md"
        );
    }

    /// Review follow-up (behavior change, verbatim UNC roots): the real
    /// Windows verbatim spelling of a UNC root is
    /// `//?/UNC/server/share/...` (the slash-normalized form of
    /// `\\?\UNC\server\share\...`), never a flat `//?/server/share/...`
    /// -- [`strip_verbatim_prefix`] now rewrites it to the ordinary
    /// `//server/share/...` shape before anything classifies it, so a
    /// target inside it relativizes exactly as
    /// `relative_to_root_of_a_unc_target_inside_the_root` above already
    /// does for the plain-spelled root.
    #[test]
    fn relative_to_root_with_a_verbatim_unc_root_relativizes_an_inside_target() {
        assert_eq!(
            relative_to_root("//server/share/skills/x/SKILL.md", "//?/UNC/server/share"),
            "skills/x/SKILL.md"
        );
    }

    /// The same verbatim UNC root rewrite for a target outside it,
    /// mirroring `relative_to_root_of_a_unc_target_outside_the_root`
    /// above.
    #[test]
    fn relative_to_root_with_a_verbatim_unc_root_relativizes_an_outside_target() {
        assert_eq!(
            relative_to_root(
                "//server/share/elsewhere/SKILL.md",
                "//?/UNC/server/share/workspace"
            ),
            "../elsewhere/SKILL.md"
        );
    }

    /// The rewrite applies to a verbatim-spelled `target` too, not only
    /// a verbatim-spelled `root`: the same share, one plain and one
    /// verbatim, still relativizes exactly as two plain spellings
    /// already do (`relative_to_root_of_a_unc_target_inside_the_root`
    /// above).
    #[test]
    fn relative_to_root_with_a_verbatim_unc_target_against_a_plain_unc_root_of_the_same_share() {
        assert_eq!(
            relative_to_root("//?/UNC/server/share/skills/x/SKILL.md", "//server/share"),
            "skills/x/SKILL.md"
        );
    }
}
