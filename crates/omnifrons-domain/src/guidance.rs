//! The agent guidance note and the managed blocks Omnifrons owns in a
//! project's text files (spike slice 5c, HAP-001 § Agent guidance note,
//! D18, R42): the fixed template with only the outbox path substituted,
//! the file each managed kind targets, the sentinels that mark the lines
//! Omnifrons owns -- naming the kind, the template version, and the digest
//! of the body they enclose -- pure locating of one block with its three
//! refusals, the status a file is in, and the pure apply and remove plans
//! that keep line endings as found.
//!
//! Framework-independent and filesystem-free: this crate depends on `std`
//! and `thiserror` only and has no hasher, so every function that needs a
//! digest takes a [`DigestFn`] the caller supplies (`omnifrons-app`'s
//! `ContentHasher`) and this module supplies the exact bytes it hashes --
//! the block's canonical body, `\n`-separated with no trailing newline, so
//! a CRLF file and an LF file carry the same digest for the same note.
//! Nothing here reads a note as an instruction: the note is content the
//! product proposes and the user disposes of, never authority
//! (HAP-001-R22, D18).

use std::fmt;
use std::ops::Range;

use crate::executable::Sha256Digest;
use crate::outbox::{OutboxPath, has_extension};
use crate::publication::{is_bidi_control, is_reserved_device_name};

/// The version the sentinels carry for the template below; a later
/// template gets a later token, and a block carrying an earlier one is
/// `outdated`.
pub const GUIDANCE_TEMPLATE_VERSION: &str = "hap-001-guidance-v1";

/// The guidance file a project is offered the note for when the user names
/// none (HAP-001 § Agent guidance note: "`AGENTS.md` or the harness's
/// equivalent").
pub const DEFAULT_GUIDANCE_FILE: &str = "AGENTS.md";

/// The one file the `ignore` kind manages: the project's Git ignore rules
/// at the workspace root, edited as text and never consulted for a verdict
/// (HAP-001-R32, D17: the ignore rule is proposed as hygiene).
pub const IGNORE_FILE: &str = ".gitignore";

/// The placeholder HAP-001's template carries, substituted with the outbox
/// path and nothing else (HAP-001-R42).
const OUTBOX_PLACEHOLDER: &str = "<outbox path>";

/// HAP-001's fixed template, verbatim: the fenced Markdown block of
/// `docs/heavy-asset-publication.md` § Agent guidance note -- the heading,
/// a blank line, and seven bullets -- with one trailing newline. The
/// document is the source of truth; `tests/guidance.rs` keeps this
/// constant equal to it.
const TEMPLATE: &str = "## Generated files\n\
\n\
- Write every generated file that is not a Markdown note — documents, images, audio, video, datasets, archives, exports — into `<outbox path>`, relative to this project's root. Create the directory if it does not exist.\n\
- If your environment names an output directory for this run, write there instead; it is a subdirectory of `<outbox path>`, and a file you also declare in a publish proposal is recorded as this run's output.\n\
- Keep Markdown notes where this project already keeps its notes, never in `<outbox path>`.\n\
- Never write large or binary files into source or configuration paths, and never commit them; `<outbox path>` should be listed in this project's ignore rules.\n\
- Use a new, descriptive file name for every output; do not overwrite an existing file in `<outbox path>`.\n\
- Treat `<outbox path>` as write-only: do not read, execute, or delete what is there.\n\
- Unless this project has explicitly allowed automatic approval for outputs a run itself declared, a person reviews everything written to `<outbox path>` before it is used anywhere else.\n";

/// A function yielding the SHA-256 of some bytes. This crate has no
/// hasher: the caller supplies the function and this module supplies the
/// bytes it hashes, so the shape of every sentinel digest is decided here
/// and the hasher only supplies the function.
pub type DigestFn<'a> = &'a dyn Fn(&[u8]) -> Sha256Digest;

/// The agent guidance note (HAP-001 § Agent guidance note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidanceNote;

impl GuidanceNote {
    /// HAP-001's fixed template, verbatim, with only `<outbox path>`
    /// substituted by `outbox`'s declared spelling (HAP-001-R42). Ends
    /// with one newline.
    #[must_use]
    pub fn render(outbox: &OutboxPath) -> String {
        TEMPLATE.replace(OUTBOX_PLACEHOLDER, &outbox.to_string())
    }
}

/// The kinds of project text file Omnifrons manages one block in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManagedFileKind {
    /// The agent guidance file: a user-named Markdown file at the workspace
    /// root (default [`DEFAULT_GUIDANCE_FILE`]), holding the note.
    Guidance,
    /// The project's [`IGNORE_FILE`], holding the outbox ignore rule.
    Ignore,
}

impl ManagedFileKind {
    /// Every kind, for exhaustive iteration and parsing.
    pub const ALL: [Self; 2] = [Self::Guidance, Self::Ignore];

    /// This kind's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Guidance => "guidance",
            Self::Ignore => "ignore",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == token)
    }

    /// The begin sentinel's fixed prefix, up to and including the space
    /// before the version.
    const fn begin_prefix(self) -> &'static str {
        match self {
            Self::Guidance => "<!-- omnifrons:begin guidance ",
            Self::Ignore => "# omnifrons:begin ignore ",
        }
    }

    /// What follows the digest on the begin sentinel's line.
    const fn begin_suffix(self) -> &'static str {
        match self {
            Self::Guidance => " -->",
            Self::Ignore => "",
        }
    }

    /// The end sentinel, a whole line of its own.
    const fn end_sentinel(self) -> &'static str {
        match self {
            Self::Guidance => "<!-- omnifrons:end guidance -->",
            Self::Ignore => "# omnifrons:end ignore",
        }
    }
}

/// Why [`ManagedFileName::guidance`] refused a name (RCS-001's file-name
/// rule, refusing rather than normalizing: a name the user types for a
/// file the product will write must be exactly what is written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedFileNameError {
    /// The name is empty.
    Empty,
    /// The name carries a path separator: it must be one component at the
    /// workspace root.
    Separator,
    /// The name is `.` or `..`.
    Traversal,
    /// The name carries a control character or a bidirectional override,
    /// embedding, or isolate character.
    Control,
    /// The name exceeds [`ManagedFileName::MAX_BYTES`].
    TooLong,
    /// The name starts with whitespace or ends with a dot or whitespace.
    EdgeCharacter,
    /// The name's stem is a Windows reserved device name.
    ReservedDeviceName,
    /// The guidance file does not end in `.md`.
    NotMarkdown,
}

impl fmt::Display for ManagedFileNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "the file name is empty",
            Self::Separator => "the file name must be one path component with no separator",
            Self::Traversal => "the file name must not be . or ..",
            Self::Control => {
                "the file name must not contain control or bidirectional override characters"
            }
            Self::TooLong => "the file name exceeds 255 bytes",
            Self::EdgeCharacter => {
                "the file name must not start with whitespace or end with a dot or whitespace"
            }
            Self::ReservedDeviceName => "the file name is a reserved device name",
            Self::NotMarkdown => "the guidance file must end in .md",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ManagedFileNameError {}

/// The name of a managed file: one path component at the workspace root.
/// Constructible only through [`Self::guidance`], [`Self::default_guidance`],
/// or [`ManagedTarget::ignore`], so every live value has passed the checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManagedFileName(String);

impl ManagedFileName {
    /// The longest name, in bytes: the common filesystem limit on one name
    /// component.
    pub const MAX_BYTES: usize = 255;

    /// Validate a user-named guidance file: one component ending in `.md`
    /// (case-insensitively), under RCS-001's file-name rule -- no
    /// separator, no traversal, no control or bidirectional override
    /// character, no reserved device name, no edge whitespace or trailing
    /// dot, at most [`Self::MAX_BYTES`].
    ///
    /// # Errors
    ///
    /// Returns [`ManagedFileNameError`] naming the first check that failed.
    pub fn guidance(raw: &str) -> Result<Self, ManagedFileNameError> {
        Self::check_component(raw)?;
        if !has_extension(raw, &["md"]) {
            return Err(ManagedFileNameError::NotMarkdown);
        }
        Ok(Self(raw.to_string()))
    }

    /// [`DEFAULT_GUIDANCE_FILE`].
    #[must_use]
    pub fn default_guidance() -> Self {
        Self(DEFAULT_GUIDANCE_FILE.to_string())
    }

    /// The checks every managed file name passes, whichever kind.
    fn check_component(raw: &str) -> Result<(), ManagedFileNameError> {
        if raw.is_empty() {
            return Err(ManagedFileNameError::Empty);
        }
        if raw.contains(['/', '\\']) {
            return Err(ManagedFileNameError::Separator);
        }
        if raw == "." || raw == ".." {
            return Err(ManagedFileNameError::Traversal);
        }
        if raw.chars().any(|c| c.is_control() || is_bidi_control(c)) {
            return Err(ManagedFileNameError::Control);
        }
        if raw.len() > Self::MAX_BYTES {
            return Err(ManagedFileNameError::TooLong);
        }
        if raw.starts_with(char::is_whitespace)
            || raw.ends_with(char::is_whitespace)
            || raw.ends_with('.')
        {
            return Err(ManagedFileNameError::EdgeCharacter);
        }
        let stem = raw.split('.').next().unwrap_or(raw);
        if is_reserved_device_name(stem) {
            return Err(ManagedFileNameError::ReservedDeviceName);
        }
        Ok(())
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManagedFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A managed file: the kind and the file it targets at the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManagedTarget {
    kind: ManagedFileKind,
    name: ManagedFileName,
}

impl ManagedTarget {
    /// The guidance file `name`.
    #[must_use]
    pub const fn guidance(name: ManagedFileName) -> Self {
        Self {
            kind: ManagedFileKind::Guidance,
            name,
        }
    }

    /// The project's ignore file, [`IGNORE_FILE`].
    #[must_use]
    pub fn ignore() -> Self {
        Self {
            kind: ManagedFileKind::Ignore,
            name: ManagedFileName(IGNORE_FILE.to_string()),
        }
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> ManagedFileKind {
        self.kind
    }

    /// The file's name at the workspace root.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.name.as_str()
    }

    /// The file's validated name.
    #[must_use]
    pub const fn name(&self) -> &ManagedFileName {
        &self.name
    }
}

/// A file's line-ending convention: kept as found (a CRLF file stays
/// CRLF), decided by the terminator that dominates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    /// The ending that dominates `text`: CRLF only when it outnumbers bare
    /// LF; a tie, or no line at all, is LF.
    #[must_use]
    pub fn dominant(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count().saturating_sub(crlf);
        if crlf > lf { Self::CrLf } else { Self::Lf }
    }

    /// The terminator.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// The ignore rule for `outbox`: root-anchored, one directory, spelled
/// `/<outbox path>/` with a leading `./` and a trailing `/` of the
/// declaration folded away.
fn ignore_rule(outbox: &OutboxPath) -> String {
    let declared = outbox.to_string();
    let mut trimmed = declared.as_str();
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest;
    }
    let trimmed = trimmed.trim_end_matches('/');
    format!("/{trimmed}/")
}

/// The lines Omnifrons owns in a managed file: the body a kind manages,
/// wrapped in that kind's sentinels with the template version and the
/// digest of the canonical body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedBlock {
    kind: ManagedFileKind,
    version: String,
    body: String,
    digest: Sha256Digest,
}

impl ManagedBlock {
    /// A block of `kind` with `body` (canonical: `\n`-separated, no
    /// trailing newline), at [`GUIDANCE_TEMPLATE_VERSION`], its digest
    /// taken from the body bytes through `digest`.
    #[must_use]
    pub fn new(kind: ManagedFileKind, body: impl Into<String>, digest: DigestFn) -> Self {
        let body = body.into();
        let digest = digest(body.as_bytes());
        Self {
            kind,
            version: GUIDANCE_TEMPLATE_VERSION.to_string(),
            body,
            digest,
        }
    }

    /// The guidance block: the note for `outbox`.
    #[must_use]
    pub fn guidance(outbox: &OutboxPath, digest: DigestFn) -> Self {
        let note = GuidanceNote::render(outbox);
        Self::new(
            ManagedFileKind::Guidance,
            note.trim_end_matches('\n'),
            digest,
        )
    }

    /// The ignore block: the one rule for `outbox`.
    #[must_use]
    pub fn ignore(outbox: &OutboxPath, digest: DigestFn) -> Self {
        Self::new(ManagedFileKind::Ignore, ignore_rule(outbox), digest)
    }

    /// The block `kind` manages for `outbox`.
    #[must_use]
    pub fn for_kind(kind: ManagedFileKind, outbox: &OutboxPath, digest: DigestFn) -> Self {
        match kind {
            ManagedFileKind::Guidance => Self::guidance(outbox, digest),
            ManagedFileKind::Ignore => Self::ignore(outbox, digest),
        }
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> ManagedFileKind {
        self.kind
    }

    /// The template version the sentinel carries.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The canonical body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The digest of the canonical body bytes, pinned in the sentinel.
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// The begin sentinel: `<prefix><version> sha256:<hex><suffix>`.
    #[must_use]
    pub fn begin_sentinel(&self) -> String {
        format!(
            "{}{} sha256:{}{}",
            self.kind.begin_prefix(),
            self.version,
            self.digest.to_hex(),
            self.kind.begin_suffix()
        )
    }

    /// The block as written into a file whose lines end with `ending`:
    /// the begin sentinel, the body lines, and the end sentinel, with no
    /// terminator after the end sentinel (the caller adds the file's).
    #[must_use]
    pub fn render(&self, ending: LineEnding) -> String {
        let ending = ending.as_str();
        let mut out = self.begin_sentinel();
        out.push_str(ending);
        for line in self.body.split('\n') {
            out.push_str(line);
            out.push_str(ending);
        }
        out.push_str(self.kind.end_sentinel());
        out
    }

    /// Whether `located` is this block exactly: the same version and the
    /// same canonical body.
    fn is_current(&self, located: &Located) -> bool {
        located.version == self.version && located.body == self.body
    }

    /// Locate the one block of `kind` in `text`, pure parsing: `None` when
    /// no sentinel of the kind is present; the block's span, version,
    /// sentinel digest, and canonical body when exactly one intact block is
    /// present.
    ///
    /// # Errors
    ///
    /// [`ManagedFileError::Malformed`] for a begin without an end, an end
    /// without a begin (or before it), more than one begin or end, or a
    /// begin line that does not parse; [`ManagedFileError::Modified`] when
    /// the body's digest, computed through `digest`, differs from the
    /// sentinel's -- a user edit inside the block.
    pub fn locate(
        kind: ManagedFileKind,
        text: &str,
        digest: DigestFn,
    ) -> Result<Option<Located>, ManagedFileError> {
        let mut begin: Option<(usize, usize, &str)> = None;
        let mut end: Option<(usize, usize)> = None;
        let (mut begins, mut ends) = (0usize, 0usize);
        let mut offset = 0usize;
        for raw in text.split_inclusive('\n') {
            let start = offset;
            offset += raw.len();
            let content = strip_terminator(raw);
            let content_end = start + content.len();
            if content.starts_with(kind.begin_prefix()) {
                begins += 1;
                if begin.is_none() {
                    begin = Some((start, content_end, content));
                }
            } else if content == kind.end_sentinel() {
                ends += 1;
                if end.is_none() && begin.is_some() {
                    end = Some((start, content_end));
                }
            }
        }
        match (begins, ends) {
            (0, 0) => return Ok(None),
            (1, 1) => {}
            _ => return Err(ManagedFileError::Malformed),
        }
        let (
            Some((begin_start, begin_content_end, begin_line)),
            Some((end_start, end_content_end)),
        ) = (begin, end)
        else {
            // An end line before the begin line.
            return Err(ManagedFileError::Malformed);
        };
        let (version, sentinel_digest) =
            parse_begin(kind, begin_line).ok_or(ManagedFileError::Malformed)?;
        // The body: the lines strictly between the sentinels, canonical.
        let between = &text[begin_content_end..end_start];
        let between = between
            .strip_prefix("\r\n")
            .or_else(|| between.strip_prefix('\n'))
            .unwrap_or(between);
        let body = between
            .split_inclusive('\n')
            .map(strip_terminator)
            .collect::<Vec<_>>()
            .join("\n");
        if digest(body.as_bytes()) != sentinel_digest {
            return Err(ManagedFileError::Modified);
        }
        Ok(Some(Located {
            span: begin_start..end_content_end,
            version,
            sentinel_digest,
            body,
        }))
    }
}

/// A line's content without its `\n` or `\r\n` terminator.
fn strip_terminator(line: &str) -> &str {
    let content = line.strip_suffix('\n').unwrap_or(line);
    content.strip_suffix('\r').unwrap_or(content)
}

/// The version and digest a begin sentinel line carries, or `None` when
/// it does not parse.
fn parse_begin(kind: ManagedFileKind, line: &str) -> Option<(String, Sha256Digest)> {
    let rest = line.strip_prefix(kind.begin_prefix())?;
    let rest = rest.strip_suffix(kind.begin_suffix())?;
    let (version, hex) = rest.split_once(" sha256:")?;
    if version.is_empty() || version.contains(char::is_whitespace) {
        return None;
    }
    Some((version.to_string(), Sha256Digest::from_hex(hex)?))
}

/// One intact block found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// The byte span of the block: from the start of the begin line to the
    /// end of the end sentinel, excluding the end line's terminator.
    pub span: Range<usize>,
    /// The template version the sentinel carries.
    pub version: String,
    /// The digest the sentinel carries, equal to the body's.
    pub sentinel_digest: Sha256Digest,
    /// The canonical body between the sentinels.
    pub body: String,
}

/// Why a managed file cannot be planned over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ManagedFileError {
    /// A begin without an end, an end without a begin, more than one
    /// block, or a begin line that does not parse.
    #[error("the managed block is malformed")]
    Malformed,
    /// The body's digest differs from the sentinel's: edited inside the
    /// block, which Omnifrons neither replaces nor removes.
    #[error("the managed block was modified inside its sentinels")]
    Modified,
}

/// The state a managed file is in with respect to the block its kind
/// manages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedStatus {
    /// The file does not exist, or carries no block of the kind.
    Absent,
    /// The file carries exactly the block that would be written now.
    Current,
    /// The file carries an intact block that is not the current one: an
    /// earlier template version, or this version rendered for another
    /// outbox path (reported under the version the sentinel carries).
    Outdated {
        /// The version the sentinel carries.
        version: String,
    },
    /// The block was edited inside its sentinels.
    Modified,
    /// The sentinels are not one intact pair.
    Malformed,
}

impl ManagedStatus {
    /// This status's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Current => "current",
            Self::Outdated { .. } => "outdated",
            Self::Modified => "modified",
            Self::Malformed => "malformed",
        }
    }
}

/// The status of a file whose content is `current` (`None` when the file
/// does not exist) with respect to `block`.
#[must_use]
pub fn status(current: Option<&str>, block: &ManagedBlock, digest: DigestFn) -> ManagedStatus {
    let Some(text) = current else {
        return ManagedStatus::Absent;
    };
    match ManagedBlock::locate(block.kind, text, digest) {
        Ok(None) => ManagedStatus::Absent,
        Ok(Some(located)) if block.is_current(&located) => ManagedStatus::Current,
        Ok(Some(located)) => ManagedStatus::Outdated {
            version: located.version,
        },
        Err(ManagedFileError::Modified) => ManagedStatus::Modified,
        Err(ManagedFileError::Malformed) => ManagedStatus::Malformed,
    }
}

/// What an apply would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyAction {
    /// Append the block after a single blank line, creating the file when
    /// absent.
    Insert,
    /// Swap exactly the located span for the block.
    Replace,
    /// The file already carries the block: nothing to write.
    NoOp,
}

impl ApplyAction {
    /// This action's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Replace => "replace",
            Self::NoOp => "no-op",
        }
    }
}

/// The apply plan: the action and the whole resulting file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlan {
    /// What the plan does.
    pub action: ApplyAction,
    /// The file's content afterwards (equal to the current content for a
    /// `NoOp`).
    pub result: String,
}

/// Plan applying `block` to a file whose content is `current` (`None`
/// when the file does not exist): insert after a single blank line at the
/// end -- creating the file when absent -- replace exactly the located
/// span, or do nothing when the file is current. Line endings are kept as
/// found, and the block uses the file's dominant ending.
///
/// # Errors
///
/// Returns [`ManagedFileError`] when the file carries a modified or
/// malformed block: neither is replaced, the user resolves it by hand or
/// restores a snapshot.
pub fn plan_apply(
    current: Option<&str>,
    block: &ManagedBlock,
    digest: DigestFn,
) -> Result<ApplyPlan, ManagedFileError> {
    let Some(text) = current.filter(|text| !text.is_empty()) else {
        return Ok(ApplyPlan {
            action: ApplyAction::Insert,
            result: format!("{}\n", block.render(LineEnding::Lf)),
        });
    };
    let ending = LineEnding::dominant(text);
    match ManagedBlock::locate(block.kind, text, digest)? {
        Some(located) if block.is_current(&located) => Ok(ApplyPlan {
            action: ApplyAction::NoOp,
            result: text.to_string(),
        }),
        Some(located) => {
            let mut result = String::with_capacity(text.len());
            result.push_str(&text[..located.span.start]);
            result.push_str(&block.render(ending));
            result.push_str(&text[located.span.end..]);
            Ok(ApplyPlan {
                action: ApplyAction::Replace,
                result,
            })
        }
        None => {
            let terminator = ending.as_str();
            let mut result = text.to_string();
            if !result.ends_with(terminator) {
                result.push_str(terminator);
            }
            let blank_line = format!("{terminator}{terminator}");
            if !result.ends_with(&blank_line) {
                result.push_str(terminator);
            }
            result.push_str(&block.render(ending));
            result.push_str(terminator);
            Ok(ApplyPlan {
                action: ApplyAction::Insert,
                result,
            })
        }
    }
}

/// The remove plan: the whole resulting file content, or `None` when the
/// file should be removed instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovePlan {
    /// The file's content afterwards, or `None` when the file would be
    /// left empty and Omnifrons created it.
    pub result: Option<String>,
}

/// Plan removing `located` -- exactly the block's lines and one adjacent
/// blank line, the one before it when there is one, else the one after --
/// from a file whose content is `current`. When the file would be left
/// empty and `created_by_us` (the oldest snapshot recorded it as not
/// existing), the plan is to remove the file.
#[must_use]
pub fn plan_remove(current: &str, located: &Located, created_by_us: bool) -> RemovePlan {
    let terminator = LineEnding::dominant(current).as_str();
    let before = &current[..located.span.start];
    let after = &current[located.span.end..];
    // The end line's own terminator goes with the block.
    let after = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    let blank_line = format!("{terminator}{terminator}");
    let mut result = String::with_capacity(before.len() + after.len());
    if let Some(trimmed) = before.strip_suffix(&blank_line) {
        result.push_str(trimmed);
        result.push_str(terminator);
        result.push_str(after);
    } else if let Some(rest) = after.strip_prefix(terminator) {
        result.push_str(before);
        result.push_str(rest);
    } else {
        result.push_str(before);
        result.push_str(after);
    }
    if result.is_empty() && created_by_us {
        return RemovePlan { result: None };
    }
    RemovePlan {
        result: Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::{LineEnding, ManagedFileKind, parse_begin, strip_terminator};
    use crate::executable::Sha256Digest;

    #[test]
    fn terminators_are_stripped_whole() {
        assert_eq!(strip_terminator("a\r\n"), "a");
        assert_eq!(strip_terminator("a\n"), "a");
        assert_eq!(strip_terminator("a"), "a");
        assert_eq!(strip_terminator("a\r"), "a");
    }

    #[test]
    fn a_begin_line_parses_only_in_its_kinds_shape() {
        let hex = Sha256Digest([0xab; 32]).to_hex();
        assert_eq!(
            parse_begin(
                ManagedFileKind::Guidance,
                &format!("<!-- omnifrons:begin guidance v2 sha256:{hex} -->")
            ),
            Some(("v2".to_string(), Sha256Digest([0xab; 32])))
        );
        assert_eq!(
            parse_begin(
                ManagedFileKind::Ignore,
                &format!("# omnifrons:begin ignore v2 sha256:{hex}")
            ),
            Some(("v2".to_string(), Sha256Digest([0xab; 32])))
        );
        assert_eq!(
            parse_begin(
                ManagedFileKind::Ignore,
                &format!("# omnifrons:begin ignore v 2 sha256:{hex}")
            ),
            None
        );
        assert_eq!(
            parse_begin(
                ManagedFileKind::Guidance,
                &format!("# omnifrons:begin ignore v2 sha256:{hex}")
            ),
            None
        );
    }

    #[test]
    fn a_tie_between_endings_is_lf() {
        assert_eq!(LineEnding::dominant("a\r\nb\n"), LineEnding::Lf);
    }
}
