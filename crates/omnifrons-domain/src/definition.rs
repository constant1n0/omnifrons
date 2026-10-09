//! One directory's agent-identity files and the state they put that
//! directory in (ADR-0005 "Agent identity and portable definition" §
//! Classification rules): the identity kinds Omnifrons reads, the
//! `AGENTS.md` import wrapper and the rule that tells a bare import
//! from real content, the shape a file must have before its bytes are
//! trusted at all, and the pure [`classify`] function that turns one
//! directory's identity files into one of nine states with a reason --
//! plus [`title`], the one other pure read a later `verify` service
//! needs. The `base_omnifrons` prototype's `portable_agent.py` is this
//! slice's specification; its scan/verify tests are translated into the
//! tests alongside this module.
//!
//! Framework-independent and filesystem-free, like [`crate::guidance`]:
//! this module never opens a file or walks a directory. Whatever
//! constructs a [`DirectoryIdentity`] -- today, this crate's tests;
//! later, the filesystem adapter of ADR-0005's sub-slice 1f -- has
//! already decided what is present and how it is shaped; `classify`
//! only reads that decision back.

use std::fmt;

/// One of the files Omnifrons reads as an agent's identity (the
/// Classification rules' identity kinds; the prototype's
/// `IDENTITY_KINDS`). [`Self::Agents`] and [`Self::Claude`] are the pair
/// every rule turns on; the other four are secondary
/// ([`Self::is_secondary`]): v1 never treats them as canonical, and a
/// secondary file that disagrees with the chosen source is a conflict,
/// never a tie-breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    /// `AGENTS.md`: the host-neutral source of instructions.
    Agents,
    /// `CLAUDE.md`: Claude Code's entry point, canonical only when it is
    /// exactly the [`WRAPPER`] that imports `AGENTS.md` (or the one
    /// accepted symlink to it).
    Claude,
    /// `GEMINI.md`: Gemini's entry point; secondary.
    Gemini,
    /// `QWEN.md`: Qwen's entry point; secondary.
    Qwen,
    /// `.claude/CLAUDE.md`: Claude Code's project-memory file; secondary.
    ClaudeDir,
    /// `AGENTS.override.md`: a local override some hosts read; secondary.
    AgentsOverride,
}

impl IdentityKind {
    /// Every kind, for exhaustive iteration and parsing.
    pub const ALL: [Self; 6] = [
        Self::Agents,
        Self::Claude,
        Self::Gemini,
        Self::Qwen,
        Self::ClaudeDir,
        Self::AgentsOverride,
    ];

    /// The repository-relative file name this kind is read from. Kept
    /// apart from [`Self::as_str`] -- that is a stable token, and this
    /// is a path -- even though the prototype uses the file name as its
    /// one identity-kind constant.
    #[must_use]
    pub const fn file_name(&self) -> &'static str {
        match self {
            Self::Agents => "AGENTS.md",
            Self::Claude => "CLAUDE.md",
            Self::Gemini => "GEMINI.md",
            Self::Qwen => "QWEN.md",
            Self::ClaudeDir => ".claude/CLAUDE.md",
            Self::AgentsOverride => "AGENTS.override.md",
        }
    }

    /// True for the four kinds `classify` never treats as canonical
    /// (the prototype's `SECONDARY_KINDS`).
    #[must_use]
    pub const fn is_secondary(&self) -> bool {
        !matches!(self, Self::Agents | Self::Claude)
    }

    /// This kind's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Qwen => "qwen",
            Self::ClaudeDir => "claude_dir",
            Self::AgentsOverride => "agents_override",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == token)
    }
}

impl fmt::Display for IdentityKind {
    /// Its file name -- the form every reason string already uses.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

/// The exact bytes of a `CLAUDE.md` that does nothing but import
/// `AGENTS.md` (the Classification rules' wrapper; the prototype's
/// `WRAPPER`). Byte-exact equality to this constant is rule 8's
/// canonical test; [`is_import_only`] is the looser test that still
/// counts as an import but not as this exact wrapper.
pub const WRAPPER: &[u8] = b"@AGENTS.md\n";

/// True when `bytes` holds only ASCII whitespace and the two accepted
/// spellings of the `AGENTS.md` import (`@AGENTS.md`, `@./AGENTS.md`).
/// Splits like Python's `bytes.split()` -- on runs of `\t \n \x0b \x0c
/// \r` and space, discarding empty runs -- because the prototype's
/// fixtures and this crate's tests both rely on that exact split: a
/// wrapper with a stray blank line or a missing trailing newline is
/// still import-only, just not byte-exact to [`WRAPPER`].
///
/// A leading UTF-8 byte-order mark is not whitespace, so a
/// byte-order-marked file's first token never matches either spelling
/// exactly: the BOM makes that token content-bearing, by design. A
/// `CLAUDE.md` saved with a BOM is signalling "this file has real
/// content", not "this file is the bare wrapper", and `classify` takes
/// it at its word.
#[must_use]
pub fn is_import_only(bytes: &[u8]) -> bool {
    let tokens: Vec<&[u8]> = bytes
        .split(|byte| matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .filter(|token| !token.is_empty())
        .collect();
    !tokens.is_empty()
        && tokens
            .iter()
            .all(|token| *token == b"@AGENTS.md" || *token == b"@./AGENTS.md")
}

/// What an identity file is, on disk, below the one symlink [`classify`]
/// accepts (a `CLAUDE.md` link to `AGENTS.md`): the shape alone decides
/// whether `classify` trusts the bytes at all (rule 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileShape {
    /// An ordinary file; the identity file's `bytes` holds its content.
    Regular,
    /// A symlink; `target` is the link's own text, unresolved -- this
    /// module is directory-unaware, so it never resolves a relative
    /// target against a real location.
    Symlink {
        /// The link's target, exactly as the link holds it.
        target: String,
    },
    /// Exists, but is neither a regular file nor a symlink (for example
    /// a directory where a file was expected).
    Irregular,
}

/// One identity file as found at a directory (the prototype's
/// `IdentityFile`, minus the path: this module is directory-unaware).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityFile {
    /// Which identity kind this is.
    pub kind: IdentityKind,
    /// The file's bytes when [`FileShape::Regular`]; empty otherwise,
    /// since a symlink or an irregular file is never read for content.
    pub bytes: Vec<u8>,
    /// The file's shape.
    pub shape: FileShape,
}

impl IdentityFile {
    /// An ordinary file (the prototype's `regular` property).
    fn is_regular(&self) -> bool {
        matches!(self.shape, FileShape::Regular)
    }

    /// Regular, and not [`is_import_only`] (the prototype's
    /// `content_bearing` property).
    fn is_content_bearing(&self) -> bool {
        self.is_regular() && !is_import_only(&self.bytes)
    }
}

/// Why [`DirectoryIdentity::new`] refused to build a value (review
/// finding 6): construction enforces one identity file per kind, so
/// `classify`'s kind-keyed lookups never have to guess which of several
/// same-kind files is the real one, and `irregular_shape`'s
/// accepted-link check never has to ask whether the one it sees is the
/// one `classify` looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{kind} is present more than once")]
pub struct DuplicateIdentityKindError {
    /// The kind that was repeated.
    pub kind: IdentityKind,
}

/// The identity files one directory holds, and the names of the
/// host-specific extras found alongside them (the prototype's
/// `read_identity_files` plus `find_host_extras`, collapsed into one
/// value: this module never walks a filesystem, so whatever constructs
/// one of these -- a test, or the fs adapter of ADR-0005's sub-slice 1f
/// -- is the one deciding what is present). The fields are private so
/// that [`Self::new`] is the only way to add identity files: see its
/// doc comment for why.
#[derive(Debug, Clone, Default)]
pub struct DirectoryIdentity {
    /// The identity files present, in no particular order.
    identity_files: Vec<IdentityFile>,
    /// The names of host-specific files found alongside the identity
    /// files (for example `CLAUDE.local.md` or `.cursor/rules`): never
    /// read as identity, but enough to turn a canonical root into
    /// [`RootState::HostExtras`].
    host_extras: Vec<String>,
}

impl DirectoryIdentity {
    /// Builds a value, rejecting more than one identity file of the
    /// same kind (review finding 6).
    ///
    /// # Errors
    /// Returns [`DuplicateIdentityKindError`] naming the first kind
    /// that repeats, in [`IdentityKind::ALL`] order.
    pub fn new(
        identity_files: Vec<IdentityFile>,
        host_extras: Vec<String>,
    ) -> Result<Self, DuplicateIdentityKindError> {
        for kind in IdentityKind::ALL {
            let seen = identity_files
                .iter()
                .filter(|file| file.kind == kind)
                .count();
            if seen > 1 {
                return Err(DuplicateIdentityKindError { kind });
            }
        }
        Ok(Self {
            identity_files,
            host_extras,
        })
    }

    /// `self`, with its host extras replaced by `host_extras`. Host
    /// extras carry no uniqueness constraint -- several may legitimately
    /// coexist (for example `CLAUDE.local.md` and `.cursor/rules`) -- so
    /// this never fails.
    #[must_use]
    pub fn with_host_extras(mut self, host_extras: Vec<String>) -> Self {
        self.host_extras = host_extras;
        self
    }

    /// The identity file of `kind`, if present (the prototype's
    /// `files.get(kind)`).
    fn find(&self, kind: IdentityKind) -> Option<&IdentityFile> {
        self.identity_files.iter().find(|file| file.kind == kind)
    }
}

/// The state one directory's identity files put it in (the
/// Classification rules' root-state machine, verbatim: the prototype's
/// nine states, same detection order, same reason wording).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootState {
    /// `CLAUDE.md` is exactly the [`WRAPPER`] (or the one accepted
    /// symlink) importing a content-bearing `AGENTS.md`: the steady
    /// state after migration, nothing to do.
    Canonical,
    /// Only `CLAUDE.md` holds content; there is no `AGENTS.md` to wrap.
    ClaudeOnly,
    /// `AGENTS.md` holds content and there is no `CLAUDE.md` wrapper yet.
    AgentsOnly,
    /// `CLAUDE.md` and `AGENTS.md` are byte-identical, but not the
    /// wrapper: an unminimized duplicate rather than an import.
    IdenticalCopies,
    /// `CLAUDE.md` imports `AGENTS.md` but is not byte-exact (a stray
    /// space, a missing newline, extra blank lines): close enough to
    /// read as an import, not close enough to call canonical.
    NearWrapper,
    /// Canonical, plus a host-specific extra file Omnifrons does not
    /// manage (for example `CLAUDE.local.md`): still canonical, with a
    /// note.
    HostExtras,
    /// Two identity files disagree on content and neither reading is
    /// safe to prefer.
    Conflict,
    /// The identity files cannot be adapted as found: a symlink, an
    /// irregular file, a source-less secondary-only directory, or an
    /// import pointing at nothing.
    Unsafe,
    /// No identity file is present; this directory is not an agent root.
    None,
}

impl RootState {
    /// Every state, for exhaustive iteration and parsing.
    pub const ALL: [Self; 9] = [
        Self::Canonical,
        Self::ClaudeOnly,
        Self::AgentsOnly,
        Self::IdenticalCopies,
        Self::NearWrapper,
        Self::HostExtras,
        Self::Conflict,
        Self::Unsafe,
        Self::None,
    ];

    /// This state's token: the prototype's exact spelling (`canonical`,
    /// `claude_only`, `agents_only`, `identical_copies`, `near_wrapper`,
    /// `host_extras`, `conflict`, `unsafe`, `none`). The underscore
    /// spelling is a public contract a later migration slice reads back
    /// from stored scan output, so it must not be renamed to a
    /// different case or word split without a migration of its own.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::ClaudeOnly => "claude_only",
            Self::AgentsOnly => "agents_only",
            Self::IdenticalCopies => "identical_copies",
            Self::NearWrapper => "near_wrapper",
            Self::HostExtras => "host_extras",
            Self::Conflict => "conflict",
            Self::Unsafe => "unsafe",
            Self::None => "none",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == token)
    }
}

/// The outcome of [`classify`]: the state, which identity file (if any)
/// holds the canonical bytes for that state, and a reason matching the
/// prototype's wording (surfaced as-is by a later `verify` service).
/// Classifying a directory never fails -- it always lands on one of the
/// nine states -- so this carries a reason, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// The state the classified directory is in.
    pub state: RootState,
    /// Which identity kind holds the canonical bytes, for every state
    /// except [`RootState::Conflict`], [`RootState::Unsafe`] and
    /// [`RootState::None`] (none of which has one safe source).
    pub source: Option<IdentityKind>,
    /// Why, in the prototype's own words.
    pub reason: String,
}

impl Classification {
    fn new(state: RootState, source: Option<IdentityKind>, reason: impl Into<String>) -> Self {
        Self {
            state,
            source,
            reason: reason.into(),
        }
    }
}

/// Whether `claude` may stand in for the exact [`WRAPPER`] because it is
/// a symlink to `AGENTS.md` next to a regular one (the Classification
/// rules' one accepted symlink). Accepts the two spellings that name
/// "this directory" without resolving a real path (`AGENTS.md`,
/// `./AGENTS.md`); any other relative form, such as a parent-directory
/// escape, is not accepted here -- resolving it needs the real
/// directory, which this module never sees. The prototype's
/// `links_to_agents` instead normalizes the target against the real
/// path; this is its filesystem-free equivalent for the two spellings
/// that do not need one.
fn is_accepted_claude_link(claude: Option<&IdentityFile>, agents: Option<&IdentityFile>) -> bool {
    let (Some(claude), Some(agents)) = (claude, agents) else {
        return false;
    };
    if !agents.is_regular() {
        return false;
    }
    match &claude.shape {
        FileShape::Symlink { target } => {
            target.strip_prefix("./").unwrap_or(target) == IdentityKind::Agents.file_name()
        }
        FileShape::Regular | FileShape::Irregular => false,
    }
}

/// The first identity file whose shape `classify` refuses to trust
/// as-is: a symlink other than the one `accepted_claude_link` allows,
/// or a path that is neither a regular file nor a symlink (rule 2,
/// checked before any content is compared, because an unsafe shape
/// makes every other signal untrustworthy). Iterates
/// [`IdentityKind::ALL`], not `dir`'s own vector, so the offender
/// reported for two directories holding the same files is the same
/// regardless of the order they were constructed in (review finding 1).
fn irregular_shape(dir: &DirectoryIdentity, accepted_claude_link: bool) -> Option<Classification> {
    for kind in IdentityKind::ALL {
        let Some(file) = dir.find(kind) else {
            continue;
        };
        let accepted = accepted_claude_link && kind == IdentityKind::Claude;
        match &file.shape {
            FileShape::Regular => {}
            FileShape::Symlink { .. } if accepted => {}
            FileShape::Symlink { target } => {
                return Some(Classification::new(
                    RootState::Unsafe,
                    None,
                    format!("{} is a symlink to {target}", kind.file_name()),
                ));
            }
            FileShape::Irregular => {
                return Some(Classification::new(
                    RootState::Unsafe,
                    None,
                    format!("{} is not a regular file", kind.file_name()),
                ));
            }
        }
    }
    None
}

/// The pair-rule outcome (rules 4-11) for a directory already known to
/// hold at least one of `CLAUDE.md`/`AGENTS.md` and no untrustworthy
/// shape: `Ok` carries the state, its source, the file holding the
/// canonical bytes, and the reason; `Err` is an early verdict (`unsafe`
/// or `conflict`) that [`classify`] returns as-is, before the
/// secondary-file and host-extras rules get a say.
fn classify_pair<'a>(
    claude: Option<&'a IdentityFile>,
    agents: Option<&'a IdentityFile>,
    accepted_link: bool,
) -> Result<(RootState, IdentityKind, &'a IdentityFile, &'static str), Classification> {
    if let Some(agents_file) = agents
        && !agents_file.is_content_bearing()
    {
        return Err(Classification::new(
            RootState::Unsafe,
            None,
            "AGENTS.md holds only imports and cannot be canonical",
        ));
    }

    match (claude, agents) {
        (Some(claude_file), None) => {
            if !claude_file.is_content_bearing() {
                return Err(Classification::new(
                    RootState::Unsafe,
                    None,
                    "CLAUDE.md imports AGENTS.md, which does not exist",
                ));
            }
            Ok((
                RootState::ClaudeOnly,
                IdentityKind::Claude,
                claude_file,
                "only CLAUDE.md holds the instructions",
            ))
        }
        (None, Some(agents_file)) => Ok((
            RootState::AgentsOnly,
            IdentityKind::Agents,
            agents_file,
            "AGENTS.md has no CLAUDE.md wrapper",
        )),
        (Some(claude_file), Some(agents_file)) => {
            if accepted_link || claude_file.bytes == WRAPPER {
                Ok((
                    RootState::Canonical,
                    IdentityKind::Agents,
                    agents_file,
                    "CLAUDE.md is the exact AGENTS.md wrapper",
                ))
            } else if !claude_file.is_content_bearing() {
                Ok((
                    RootState::NearWrapper,
                    IdentityKind::Agents,
                    agents_file,
                    "CLAUDE.md is a non-exact AGENTS.md wrapper",
                ))
            } else if claude_file.bytes == agents_file.bytes {
                Ok((
                    RootState::IdenticalCopies,
                    IdentityKind::Agents,
                    agents_file,
                    "CLAUDE.md is a byte copy of AGENTS.md",
                ))
            } else {
                Err(Classification::new(
                    RootState::Conflict,
                    None,
                    "CLAUDE.md and AGENTS.md hold different content",
                ))
            }
        }
        // Unreachable given `classify`'s own "both absent" return
        // before this is called; a safe, non-panicking fallback keeps
        // this match exhaustive without an arm that could itself panic.
        (None, None) => Err(Classification::new(
            RootState::None,
            None,
            "no identity files",
        )),
    }
}

/// The rules applied after the pair rule resolves: a content-bearing
/// secondary file that disagrees with the canonical bytes always wins
/// as a conflict; otherwise a canonical root with host extras is
/// reported as [`RootState::HostExtras`], and every other state passes
/// through unchanged.
fn apply_secondary_and_host_extras(
    dir: &DirectoryIdentity,
    state: RootState,
    source: IdentityKind,
    canonical_file: &IdentityFile,
    reason: &str,
) -> Classification {
    let differing: Vec<&str> = IdentityKind::ALL
        .into_iter()
        .filter(IdentityKind::is_secondary)
        .filter_map(|kind| dir.find(kind))
        .filter(|file| file.is_content_bearing() && file.bytes != canonical_file.bytes)
        .map(|file| file.kind.file_name())
        .collect();
    if !differing.is_empty() {
        return Classification::new(
            RootState::Conflict,
            None,
            format!(
                "{} differ from {}",
                differing.join(", "),
                source.file_name()
            ),
        );
    }
    if state == RootState::Canonical && !dir.host_extras.is_empty() {
        return Classification::new(
            RootState::HostExtras,
            Some(IdentityKind::Agents),
            format!(
                "canonical, with host extras: {}",
                dir.host_extras.join(", ")
            ),
        );
    }
    Classification::new(state, Some(source), reason)
}

/// The state one directory's identity files put it in, why, and which
/// file (if any) holds the canonical bytes (ADR-0005 § Classification
/// rules, the eleven ordered rules plus the secondary-file and
/// host-extras rules, verbatim from the `base_omnifrons` prototype's
/// `classify`). Pure: `dir` already says what is present and how it is
/// shaped, so this never reads a file or fails.
#[must_use]
pub fn classify(dir: &DirectoryIdentity) -> Classification {
    if dir.identity_files.is_empty() {
        return Classification::new(RootState::None, None, "no identity files");
    }

    let claude = dir.find(IdentityKind::Claude);
    let agents = dir.find(IdentityKind::Agents);
    let accepted_link = is_accepted_claude_link(claude, agents);

    if let Some(unsafe_classification) = irregular_shape(dir, accepted_link) {
        return unsafe_classification;
    }

    if claude.is_none() && agents.is_none() {
        // `IdentityKind::ALL` order, not `dir`'s own vector order, so
        // this reason is independent of construction order too (review
        // finding 1).
        let kinds = IdentityKind::ALL
            .into_iter()
            .filter(|kind| dir.find(*kind).is_some())
            .map(|kind| kind.file_name())
            .collect::<Vec<_>>()
            .join(", ");
        return Classification::new(
            RootState::Unsafe,
            None,
            format!("no CLAUDE.md or AGENTS.md source; v1 does not adapt {kinds}"),
        );
    }

    let (state, source, canonical_file, reason) = match classify_pair(claude, agents, accepted_link)
    {
        Ok(resolved) => resolved,
        Err(classification) => return classification,
    };
    apply_secondary_and_host_extras(dir, state, source, canonical_file, reason)
}

/// The first ATX `# ` heading outside a fenced code block, or `None`
/// when there is none (the prototype's `extract_title`). Decodes
/// lossily -- this string is for display, never for a decision -- and
/// strips one leading UTF-8 byte-order mark; unlike [`is_import_only`],
/// a BOM here is cosmetic and never keeps a heading from counting.
/// Tolerates a trailing `\r` on every line, so a CRLF file reads the
/// same title as its LF twin, and trims a closing ATX `#` run, so
/// `"# Title #"` and `"# Title"` return the same title.
#[must_use]
pub fn title(bytes: &[u8]) -> Option<String> {
    let decoded = String::from_utf8_lossy(bytes);
    let content = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);

    let mut in_fence = false;
    for raw_line in content.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if is_fence_delimiter(trimmed) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            let heading = strip_closing_hash_run(heading.trim()).trim();
            if !heading.is_empty() {
                return Some(heading.to_string());
            }
        }
    }
    None
}

/// Strips an ATX closing sequence of trailing `#` characters, but only
/// when that run is preceded by whitespace or is the heading's entire
/// content (`CommonMark`'s rule: a `#` glued to the preceding word, as in
/// `C#`, is content, not a closing sequence; review finding 5). The
/// prototype's `extract_title` instead strips any trailing `#` run
/// unconditionally (`str.rstrip("#")`), which would also strip the `#`
/// from `C#`; this is deliberately more correct there, while still
/// agreeing with the prototype on the degenerate `# #` heading, whose
/// content is a bare closing sequence with no text before it and so
/// reduces to empty either way.
fn strip_closing_hash_run(heading: &str) -> &str {
    let without_trailing_spaces = heading.trim_end_matches(' ');
    let core = without_trailing_spaces.trim_end_matches('#');
    if core.len() == without_trailing_spaces.len() {
        return heading; // No trailing `#` at all: nothing to strip.
    }
    if core.is_empty() || core.ends_with(' ') {
        core
    } else {
        heading
    }
}

/// A Markdown fence delimiter line (the prototype's `is_fence`): a
/// trimmed line starting with three backticks or three tildes. Both the
/// opening and closing delimiter toggle [`title`]'s fence state and are
/// themselves excluded from heading detection.
fn is_fence_delimiter(trimmed_line: &str) -> bool {
    trimmed_line.starts_with("```") || trimmed_line.starts_with("~~~")
}
