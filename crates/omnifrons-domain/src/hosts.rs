//! Host instruction-loader table (ADR-0005 sub-slice 1d).
//!
//! `HostLoader` is plain data: no filesystem access, and no behaviour
//! beyond the lookups [`host`], [`order_index`], and
//! [`HostLoader::reads`] provide. The table
//! starts from the nine hosts the `base_omnifrons` prototype's
//! `HOSTS.md` documented (accessed 2026-09-25), and carries each row's
//! source URL, access date, and verification status unchanged. The
//! prototype's own constants -- `HOST_ORDER`, `EXTRA_READERS`,
//! `SKIP_DIRS`, `HOST_CONFIG_DIRS`, `GENERATED_STATE_DIRS`,
//! `CODEX_MAX_BYTES`, `ANTIGRAVITY_MAX_BYTES`, `UPWARD_LOADERS`, and
//! `IMPORT_BLIND_READERS` -- are transcribed verbatim into the items
//! below.
//!
//! ADR-0005 (`docs/adr/0005-agent-identity-and-portable-definition.md`)
//! uses this table for warnings only: it flags when an agent root's
//! canonical files could be shadowed, truncated, or misread by one of
//! these nine hosts, and reports a host-specific extra file without ever
//! touching it. A row is a claim about a host, never a guarantee --
//! Omnifrons guarantees behaviour only through its own built-in adapters,
//! and this table never extends the closed `AdapterId` set.

/// Codex's `project_doc_max_bytes` default (32 KiB), past which it stops
/// adding files (the prototype's `CODEX_MAX_BYTES`).
pub const CODEX_MAX_BYTES: usize = 32 * 1024;

/// The byte size above which Antigravity truncates a single rule file
/// after expanding includes (the prototype's `ANTIGRAVITY_MAX_BYTES`).
pub const ANTIGRAVITY_MAX_BYTES: usize = 24_000;

/// The directory walk a host's Notes column documents, or the lack of
/// one. Variants group hosts whose wording describes the same walk
/// shape; each host's own exact wording is quoted below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Walk {
    /// Claude Code: "Ancestor files load at launch; subdirectory files
    /// load on demand."
    AncestorsAtLaunch,
    /// Codex: "Walks from the git root down to the working directory."
    /// Kimi: "Both load per directory, `.kimi/AGENTS.md` first, from the
    /// project root down to the working directory."
    GitRootToCwd,
    /// Qwen Code: "Walks upward; stops above the git root, or without
    /// git at the parent of the home directory." `OpenCode`: "Traverses
    /// up from the working directory; the first matching file wins."
    /// The prototype's `UPWARD_LOADERS` comment notes that `OpenCode`
    /// "also walks upward but uses the first `AGENTS.md` it finds
    /// (handled apart)" from the load-every-ancestor shape that constant
    /// models.
    Upward,
    /// Cursor: "Supports nested `AGENTS.md` in subdirectories." No
    /// ancestor walk is documented.
    NestedOnly,
    /// Copilot: "The nearest `AGENTS.md` in the directory tree takes
    /// precedence."
    NearestInTree,
    /// Gemini CLI and Antigravity: the Notes column names which files
    /// load but documents no walk direction.
    Undocumented,
}

/// Whether, and how, a host expands an `@`-style import inside its
/// instruction files, per the table's Notes column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportSupport {
    /// No `@` expansion is documented; matches the prototype's
    /// `IMPORT_BLIND_READERS` membership for the same host.
    None,
    /// Claude Code: "Expands `@path` imports." Gemini CLI: "Expands
    /// `@file.md` imports."
    AtPath,
    /// Antigravity: "Truncates any single rule file above 24,000 bytes
    /// after expanding includes."
    Includes,
}

/// One host's documented instruction-loading behaviour: plain data, with
/// no filesystem access and no behaviour beyond this module's lookups,
/// including [`reads`](Self::reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostLoader {
    /// The host's name, exactly as the prototype's `HOST_ORDER` spells
    /// it.
    pub name: &'static str,
    /// The identity/instruction files this host loads when present, per
    /// the table's "Project instruction files" column, in the table's
    /// own order. See [`fallback_files`](Self::fallback_files) for
    /// files this host reads only in their absence.
    pub files: &'static [&'static str],
    /// The files this host reads only when none of
    /// [`files`](Self::files) exists in the working directory or above
    /// it, per the table's Notes column. Claude Code's Notes document
    /// this as "Reads `AGENTS.md` only when no `CLAUDE.md`,
    /// `.claude/CLAUDE.md` or `CLAUDE.local.md` exists in the working
    /// directory or above it (v2.1.277 or later)". The official Claude
    /// Code memory documentation ("When Claude Code reads AGENTS.md",
    /// read 2026-10-12) adds `.claude/AGENTS.md` to that fallback: at
    /// session start it loads every `AGENTS.md` and `.claude/AGENTS.md`
    /// in the working directory and the directories above it. The table
    /// omits the second file, so this row follows the documentation
    /// there, a deliberate deviation. The same documentation makes the
    /// rule depend on a configurable instruction-files mode
    /// (`claude-md-or-agents-md` by default); the table models only the
    /// default. Every other host's Notes document no such conditional
    /// fallback, so theirs is empty.
    pub fallback_files: &'static [&'static str],
    /// The documented directory walk, or [`Walk::Undocumented`] when the
    /// table names no direction.
    pub walk: Walk,
    /// The documented `@`-import support, or [`ImportSupport::None`]
    /// when none is recorded.
    pub imports: ImportSupport,
    /// The files this host loads from every ancestor directory, from
    /// the prototype's `UPWARD_LOADERS`; empty when the prototype does
    /// not model this host's upward walk.
    pub upward_kinds: &'static [&'static str],
    /// Whether this host is in the prototype's `IMPORT_BLIND_READERS`:
    /// it reads `AGENTS.md` without documented `@` expansion.
    pub import_blind: bool,
    /// The byte size past which this host stops adding or truncates
    /// content ([`CODEX_MAX_BYTES`] or [`ANTIGRAVITY_MAX_BYTES`]), or
    /// `None` when the table documents no cap.
    pub size_cap: Option<usize>,
    /// The table's documented source URL for this row.
    pub source: &'static str,
    /// The table's access date for this row, ISO 8601.
    pub accessed: &'static str,
    /// The table's Status column text, verbatim.
    pub status: &'static str,
}

impl HostLoader {
    /// Whether this host can ever load `file`: present in
    /// [`files`](Self::files) (loaded when present) or in
    /// [`fallback_files`](Self::fallback_files) (loaded only when none
    /// of `files` exists). This is the loose "can this host ever read
    /// it" question a switch check asks; it does not touch the
    /// filesystem or resolve which directory would actually supply the
    /// file.
    #[must_use]
    pub fn reads(&self, file: &str) -> bool {
        self.files.contains(&file) || self.fallback_files.contains(&file)
    }
}

/// The nine hosts the `base_omnifrons` prototype's `HOSTS.md` documented
/// (accessed 2026-09-25), in the prototype's `HOST_ORDER`.
pub const HOSTS: [HostLoader; 9] = [
    HostLoader {
        name: "Claude Code",
        files: &["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"],
        fallback_files: &["AGENTS.md", ".claude/AGENTS.md"],
        walk: Walk::AncestorsAtLaunch,
        imports: ImportSupport::AtPath,
        upward_kinds: &["CLAUDE.md", ".claude/CLAUDE.md"],
        import_blind: false,
        size_cap: None,
        source: "https://code.claude.com/docs/en/memory",
        accessed: "2026-09-25",
        status: "Verified",
    },
    HostLoader {
        name: "Codex",
        files: &["AGENTS.override.md", "AGENTS.md"],
        fallback_files: &[],
        walk: Walk::GitRootToCwd,
        imports: ImportSupport::None,
        upward_kinds: &[],
        import_blind: true,
        size_cap: Some(CODEX_MAX_BYTES),
        source: "https://learn.chatgpt.com/docs/agent-configuration/agents-md",
        accessed: "2026-09-25",
        status: "Verified",
    },
    HostLoader {
        name: "Gemini CLI",
        files: &["GEMINI.md"],
        fallback_files: &[],
        walk: Walk::Undocumented,
        imports: ImportSupport::AtPath,
        upward_kinds: &[],
        import_blind: false,
        size_cap: None,
        source: "https://geminicli.com/docs/cli/gemini-md/",
        accessed: "2026-09-25",
        status: "Verified (docs page); configuration reference unverified",
    },
    HostLoader {
        name: "Antigravity",
        // The table's Files column writes this path with a trailing
        // slash; the prototype's EXTRA_READERS key for the same path
        // omits it (see module tests for both literal forms).
        files: &["AGENTS.md", "GEMINI.md", ".agents/rules/"],
        fallback_files: &[],
        walk: Walk::Undocumented,
        imports: ImportSupport::Includes,
        upward_kinds: &[],
        import_blind: false,
        size_cap: Some(ANTIGRAVITY_MAX_BYTES),
        source: "https://antigravity.google/docs/rules/",
        accessed: "2026-09-25",
        status: "Verified",
    },
    HostLoader {
        name: "Qwen Code",
        files: &["QWEN.md", "AGENTS.md"],
        fallback_files: &[],
        walk: Walk::Upward,
        imports: ImportSupport::None,
        upward_kinds: &["QWEN.md", "AGENTS.md"],
        import_blind: true,
        size_cap: None,
        source: "https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/utils/memory-constants.ts",
        accessed: "2026-09-25",
        status: "Verified (source); user docs unverified",
    },
    HostLoader {
        name: "OpenCode",
        files: &["AGENTS.md", "CLAUDE.md"],
        fallback_files: &[],
        walk: Walk::Upward,
        imports: ImportSupport::None,
        upward_kinds: &[],
        import_blind: true,
        size_cap: None,
        source: "https://opencode.ai/docs/rules/",
        accessed: "2026-09-25",
        status: "Verified (docs); source file unverified",
    },
    HostLoader {
        name: "Kimi",
        files: &["AGENTS.md", ".kimi/AGENTS.md"],
        fallback_files: &[],
        walk: Walk::GitRootToCwd,
        imports: ImportSupport::None,
        upward_kinds: &[],
        import_blind: true,
        size_cap: None,
        source: "https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/soul/agent.py",
        accessed: "2026-09-25",
        status: "Verified (kimi-cli source); kimi-code docs unverified",
    },
    HostLoader {
        name: "Cursor",
        files: &["AGENTS.md"],
        fallback_files: &[],
        walk: Walk::NestedOnly,
        imports: ImportSupport::None,
        upward_kinds: &[],
        import_blind: true,
        size_cap: None,
        source: "https://cursor.com/docs/context/rules",
        accessed: "2026-09-25",
        status: "Verified; `.cursorrules` unverified",
    },
    HostLoader {
        name: "Copilot",
        files: &["AGENTS.md"],
        fallback_files: &[],
        walk: Walk::NearestInTree,
        imports: ImportSupport::None,
        upward_kinds: &[],
        import_blind: true,
        size_cap: None,
        source: "https://docs.github.com/en/copilot/how-tos/configure-custom-instructions/add-repository-instructions",
        accessed: "2026-09-25",
        status: "Verified",
    },
];

/// The host named `name`, or `None` if it is not one of the documented
/// nine.
#[must_use]
pub fn host(name: &str) -> Option<&'static HostLoader> {
    HOSTS.iter().find(|host| host.name == name)
}

/// `name`'s position in the prototype's `HOST_ORDER`, the order the
/// prototype sorts warning hosts by.
#[must_use]
pub fn order_index(name: &str) -> Option<usize> {
    HOSTS.iter().position(|host| host.name == name)
}

/// Host-specific files that are reported and never touched, mapped to
/// their readers; verbatim from the prototype's `EXTRA_READERS`, in its
/// order.
#[rustfmt::skip]
pub const EXTRA_READERS: &[(&str, &[&str])] = &[
    ("CLAUDE.local.md", &["Claude Code"]),
    (".kimi/AGENTS.md", &["Kimi"]),
    (".agents/AGENTS.md", &["Antigravity"]),
    (".agents/rules", &["Antigravity"]),
    (".claude/rules", &["Claude Code"]),
    (".cursor/rules", &["Cursor"]),
    (".cursorrules", &["Cursor"]),
    (".github/copilot-instructions.md", &["Copilot"]),
];

/// Directories the prototype's walk never descends into, verbatim from
/// `SKIP_DIRS` (a Python `frozenset`, so this array fixes a deterministic
/// order the prototype's own set does not guarantee).
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "__pycache__",
    ".portable-agent",
];

/// Host configuration folders whose identity-named files belong to the
/// parent root, verbatim from `HOST_CONFIG_DIRS`.
pub const HOST_CONFIG_DIRS: &[&str] = &[".claude", ".kimi", ".agents"];

/// Folders that hold generated tool state, never agent instructions,
/// verbatim from `GENERATED_STATE_DIRS`.
pub const GENERATED_STATE_DIRS: &[&str] = &[".atl"];

/// Each upward-walking host paired with the file kinds it loads from
/// every ancestor directory, derived from [`HOSTS`] so this can never
/// drift from the per-host `upward_kinds` field. Equals the prototype's
/// `UPWARD_LOADERS` tuple, in order.
///
/// Not `#[must_use]`: an `Iterator` return already carries that
/// attribute, and repeating it is itself a lint error under this crate's
/// `clippy::all = "deny"`.
pub fn upward_loaders() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    HOSTS
        .iter()
        .filter(|host| !host.upward_kinds.is_empty())
        .map(|host| (host.name, host.upward_kinds))
}

/// Hosts that read `AGENTS.md` without documented `@` expansion, derived
/// from [`HOSTS`] so this can never drift from the per-host
/// `import_blind` field. Equals the prototype's `IMPORT_BLIND_READERS`
/// tuple, in order.
///
/// Not `#[must_use]`, for the same reason as [`upward_loaders`].
pub fn import_blind_readers() -> impl Iterator<Item = &'static str> {
    HOSTS
        .iter()
        .filter(|host| host.import_blind)
        .map(|host| host.name)
}

/// The readers of a host-specific extra file named `file`, from
/// [`EXTRA_READERS`], or `None` when `file` is not one of its entries.
/// A small lookup [`crate::warnings`] needs for the `extra_host_file`
/// warning; added here because [`EXTRA_READERS`] is private data this
/// module already owns.
#[must_use]
pub fn extra_readers(file: &str) -> Option<&'static [&'static str]> {
    EXTRA_READERS
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, readers)| *readers)
}
