//! The plan model for adapting one agent root to the portable layout
//! (ADR-0005 decisions 2 and 3, sub-slice 2a): the fingerprint of each
//! input file, the identity-file operations a dry run would apply, the
//! refusals a dry run can end in, and the plan id.
//!
//! This module is pure: it holds no I/O and no hasher. The application
//! layer reads the snapshot, decides which operations and refusals
//! apply, builds [`canonical_bytes`], and hashes those bytes through its
//! own `ContentHasher` port to obtain the [`PlanId`]. A plan that is
//! refused has no operations and no id; [`PlanRefusal`] is therefore a
//! separate type from [`Plan`], never a field of it.
//!
//! # Plan id
//!
//! The id is the SHA-256 of [`canonical_bytes`]: the canonical workspace
//! root, the agent path, every input fingerprint, and every operation, in a fixed text form. The
//! form is Omnifrons' own. It does not reproduce the byte layout of any
//! earlier tool, so ids are stable within this implementation only.
//!
//! # Scope of sub-slices 2a and 2b
//!
//! Operations cover the identity files (the canonical `AGENTS.md`, the
//! `CLAUDE.md` wrapper, and the opt-in `GEMINI.md` wrapper) and the one
//! `skills` block write that follows them: an append, a replacement of
//! the current block, or the one-time migration of a legacy
//! `portable-agent:` block ([`BlockMode`]). The public [`Plan`] carries
//! digests and a [`BlockDiff`], never the bytes to write; the
//! application layer keeps those. Backups arrive with the write path.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use crate::definition::IdentityKind;
use crate::executable::Sha256Digest;
use crate::guidance::{ManagedFileKind, SKILLS_TEMPLATE_VERSION};
use crate::hosts::HOST_CONFIG_DIRS;

/// The first line of every [`canonical_bytes`] text. Changing the layout
/// of that text means changing this tag, so an old id can never match a
/// new layout.
pub const CANONICAL_FORM_TAG: &str = "omnifrons-plan-v4";

/// What a plan knows about one input file when it was planned. A later
/// apply compares the file's current fingerprint against this one and
/// refuses a stale plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputFingerprint {
    /// A regular file, by the SHA-256 of its bytes.
    Sha256(Sha256Digest),
    /// A symbolic link, by its own unresolved target text.
    Symlink(String),
    /// Exists, but is neither a regular file nor a symlink.
    Irregular,
    /// Does not exist.
    Absent,
}

impl InputFingerprint {
    /// This fingerprint's line in [`canonical_bytes`]: `sha256 <hex>`,
    /// `symlink <len>:<target>`, `irregular`, or `absent`.
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            Self::Sha256(digest) => format!("sha256 {}", digest.to_hex()),
            Self::Symlink(target) => format!("symlink {}:{target}", target.len()),
            Self::Irregular => "irregular".to_string(),
            Self::Absent => "absent".to_string(),
        }
    }
}

/// How a block write changes the canonical file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMode {
    /// No Omnifrons block exists yet: a new one is added at the end of
    /// the file, after one blank line.
    Append,
    /// The current Omnifrons block is regenerated in place. A block that
    /// was edited by hand is never replaced; it is refused.
    Replace,
    /// An intact legacy `portable-agent:` block is replaced, once, by
    /// the Omnifrons block the `skills` kind defines: its bytes are
    /// regenerated from the local skills and the precedence rule, never
    /// carried over. When a current block sits beside it, the legacy
    /// block is removed and the current one regenerated in place.
    MigrateLegacy,
}

impl BlockMode {
    /// Every mode, for exhaustive iteration and parsing.
    pub const ALL: [Self; 3] = [Self::Append, Self::Replace, Self::MigrateLegacy];

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == token)
    }

    /// This mode's stable token, as written in [`canonical_bytes`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Replace => "replace",
            Self::MigrateLegacy => "migrate-legacy",
        }
    }
}

/// One planned operation, carrying what an apply needs: the file's
/// relative path, the digest of the bytes to write, and the fingerprint
/// the file must still have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Write `AGENTS.md` with the bytes of `source` (the chosen
    /// canonical file).
    WriteAgents {
        /// The identity file whose bytes become `AGENTS.md`.
        source: IdentityKind,
        /// The SHA-256 of the bytes to write.
        after: Sha256Digest,
        /// What `AGENTS.md` must still look like when this runs.
        before: InputFingerprint,
    },
    /// Write the import-only wrapper (`@AGENTS.md`) over `kind`.
    WriteWrapper {
        /// The file that becomes a wrapper: `CLAUDE.md` or `GEMINI.md`.
        kind: IdentityKind,
        /// The SHA-256 of the wrapper bytes.
        after: Sha256Digest,
        /// What the file must still look like when this runs.
        before: InputFingerprint,
    },
    /// Create `AGENTS.md` from the create interview: the fixed template
    /// rendered from the answers, followed by the `skills` block when the
    /// folder holds skills. It only ever creates a new file, so on a
    /// fresh create `before` is [`InputFingerprint::Absent`]; a rerun
    /// that finds the file already holding these bytes plans nothing.
    WriteGenerated {
        /// The version of the template the bytes were rendered from
        /// ([`crate::definition_create::CREATE_TEMPLATE_VERSION`]).
        template_version: String,
        /// The SHA-256 of the canonical form of the answers
        /// ([`crate::definition_create::answers_canonical`]).
        answers_digest: Sha256Digest,
        /// The SHA-256 of the bytes to write.
        after: Sha256Digest,
        /// What `AGENTS.md` must still look like when this runs.
        before: InputFingerprint,
    },
    /// Write the `skills` block into the canonical `AGENTS.md`. It
    /// follows the identity-file operations, so `before` is the file as
    /// it is when this runs: the current `AGENTS.md`, or the bytes
    /// [`Self::WriteAgents`] has just written.
    WriteBlock {
        /// The kind of block written: always [`ManagedFileKind::Skills`]
        /// in this slice.
        kind: ManagedFileKind,
        /// How the block is added.
        mode: BlockMode,
        /// The SHA-256 of the block's canonical body, as pinned in its
        /// begin sentinel.
        body: Sha256Digest,
        /// The SHA-256 of the whole file after the write.
        after: Sha256Digest,
        /// What `AGENTS.md` must still look like when this runs.
        before: InputFingerprint,
    },
}

impl Operation {
    /// The relative path this operation writes.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::WriteAgents { .. } | Self::WriteGenerated { .. } | Self::WriteBlock { .. } => {
                IdentityKind::Agents.file_name()
            }
            Self::WriteWrapper { kind, .. } => kind.file_name(),
        }
    }

    /// The fingerprint the written file must still have at apply time.
    #[must_use]
    pub const fn before(&self) -> &InputFingerprint {
        match self {
            Self::WriteAgents { before, .. }
            | Self::WriteGenerated { before, .. }
            | Self::WriteWrapper { before, .. }
            | Self::WriteBlock { before, .. } => before,
        }
    }

    /// The SHA-256 of the whole file after this operation runs.
    #[must_use]
    pub const fn after(&self) -> &Sha256Digest {
        match self {
            Self::WriteAgents { after, .. }
            | Self::WriteGenerated { after, .. }
            | Self::WriteWrapper { after, .. }
            | Self::WriteBlock { after, .. } => after,
        }
    }

    /// This operation's line in [`canonical_bytes`].
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            Self::WriteAgents {
                source,
                after,
                before,
            } => format!(
                "write-agents source={} after={} before={}",
                source.file_name(),
                after.to_hex(),
                before.canonical()
            ),
            Self::WriteGenerated {
                template_version,
                answers_digest,
                after,
                before,
            } => format!(
                "write-generated path={} template={}:{template_version} answers={} after={} \
                 before={}",
                self.path(),
                template_version.len(),
                answers_digest.to_hex(),
                after.to_hex(),
                before.canonical()
            ),
            Self::WriteWrapper {
                kind,
                after,
                before,
            } => format!(
                "write-wrapper path={} after={} before={}",
                kind.file_name(),
                after.to_hex(),
                before.canonical()
            ),
            Self::WriteBlock {
                kind,
                mode,
                body,
                after,
                before,
            } => format!(
                "write-block path={} kind={} version={}:{SKILLS_TEMPLATE_VERSION} mode={} \
                 body={} after={} before={}",
                self.path(),
                block_kind_token(*kind),
                SKILLS_TEMPLATE_VERSION.len(),
                mode.as_str(),
                body.to_hex(),
                after.to_hex(),
                before.canonical()
            ),
        }
    }
}

/// The block kind's token in [`canonical_bytes`]. Only the `skills`
/// kind is ever planned here.
const fn block_kind_token(kind: ManagedFileKind) -> &'static str {
    match kind {
        ManagedFileKind::Skills => "skills",
        ManagedFileKind::Guidance => "guidance",
        ManagedFileKind::Ignore => "ignore",
    }
}

/// The lines of the block a plan adds, replaces or migrates, for the
/// dry run to show. The lines carry no terminators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDiff {
    /// The file the block lives in.
    pub path: &'static str,
    /// The lines of every block the write replaces or removes, in file
    /// order: empty for an append, the current Omnifrons block for a
    /// replacement, the legacy block for a migration, and both blocks
    /// (current first when it comes first) for a migration beside a
    /// current block.
    pub before: Vec<String>,
    /// The block's lines after the write, sentinels included.
    pub after: Vec<String>,
}

/// Something a dry run reports without ever acting on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// A `.portable-agent/` backup folder sits beside the definition.
    /// It is reported and never deleted.
    PortableAgentFolder {
        /// The folder's path, relative to the workspace.
        path: String,
    },
}

/// The flags that change what a plan may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanOptions {
    /// Whether the agent's parent has consented to adapting this nested
    /// agent. A parent's consent never covers a child: this is the
    /// child's own.
    pub nested_consent: bool,
    /// Which of `CLAUDE.md` and `AGENTS.md` becomes canonical when the
    /// two disagree. Only meaningful for a conflicting root.
    pub canonical_from: Option<IdentityKind>,
    /// Whether to also write the `GEMINI.md` wrapper.
    pub with_gemini: bool,
}

/// Why a plan was refused. Every variant carries the human-readable
/// message; [`Self::code`] is the stable reason token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRefusal {
    /// The agent sits below another agent and has no consent of its own.
    NestedAgent(String),
    /// An identity file is a symlink or irregular (other than the one
    /// accepted `CLAUDE.md` link), or the directory cannot be adapted.
    Unsafe(String),
    /// `CLAUDE.md` and `AGENTS.md` differ and no canonical file was
    /// chosen.
    Conflict(String),
    /// A secondary identity file would still differ from the chosen
    /// source.
    UnresolvedFile(String),
    /// The skills block was edited by hand.
    EditedBlock(String),
    /// The skills block markers are not one intact pair.
    BlockLayout(String),
    /// A skill's metadata cannot be written into the index safely.
    UnsafeSkillMetadata(String),
    /// An unmanaged byte copy of the source would stop matching once
    /// the skills block is written.
    DivergingCopy(String),
    /// A create target that must not receive a new agent: a symlink, the
    /// home directory, or a host configuration folder (or a folder
    /// inside one). The message names which.
    UnsafeTarget(String),
    /// A create target that already holds identity files other than this
    /// writer's own result: it is an existing agent, to be adapted
    /// instead. The message names the files.
    IdentityExists(String),
    /// A `sources` entry that is missing, a symlink, or inside or around
    /// a child agent. The message names it.
    InvalidSource(String),
}

impl PlanRefusal {
    /// This refusal's stable reason token.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NestedAgent(_) => "nested_agent",
            Self::Unsafe(_) => "unsafe",
            Self::Conflict(_) => "conflict",
            Self::UnresolvedFile(_) => "unresolved_file",
            Self::EditedBlock(_) => "edited_block",
            Self::BlockLayout(_) => "block_layout",
            Self::UnsafeSkillMetadata(_) => "unsafe_skill_metadata",
            Self::DivergingCopy(_) => "diverging_copy",
            Self::UnsafeTarget(_) => "unsafe_target",
            Self::IdentityExists(_) => "identity_exists",
            Self::InvalidSource(_) => "invalid_source",
        }
    }

    /// The refusal's message.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::NestedAgent(message)
            | Self::Unsafe(message)
            | Self::Conflict(message)
            | Self::UnresolvedFile(message)
            | Self::EditedBlock(message)
            | Self::BlockLayout(message)
            | Self::UnsafeSkillMetadata(message)
            | Self::DivergingCopy(message)
            | Self::UnsafeTarget(message)
            | Self::IdentityExists(message)
            | Self::InvalidSource(message) => message,
        }
    }
}

impl fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refused ({}): {}", self.code(), self.message())
    }
}

/// Host configuration folder names a create target may not lie in or
/// under, beyond [`HOST_CONFIG_DIRS`]. This is the create script's own
/// list; the scan's [`HOST_CONFIG_DIRS`] stays as it is. Cursor loads
/// Markdown from `.cursor/rules` as configuration of the folder above it.
pub const EXTRA_HOST_CONFIG_DIRS: &[&str] = &[".cursor"];

/// The index of the deepest component of `components` that names a host
/// configuration folder, compared without case (case-insensitive file
/// systems treat `.Claude` as `.claude`). The components are the
/// target's physical path, one name per entry, from the root down.
#[must_use]
pub fn host_config_index(components: &[&str]) -> Option<usize> {
    components.iter().rposition(|component| {
        let folded = component.to_lowercase();
        HOST_CONFIG_DIRS
            .iter()
            .chain(EXTRA_HOST_CONFIG_DIRS)
            .any(|name| name.to_lowercase() == folded)
    })
}

/// Whether a create target lies in a host configuration folder: the
/// target itself or any folder above it is one. The application supplies
/// the physical path.
#[must_use]
pub fn host_config_folder(components: &[&str]) -> bool {
    host_config_index(components).is_some()
}

/// The refusal message for a target that is itself a symlink.
#[must_use]
pub fn symlink_target_message(directory: &str, link: &str) -> String {
    format!(
        "{directory} is a symlink to {link}; pass the real folder (new agents are never \
         written through a symlink)"
    )
}

/// The refusal message for a target that is a host configuration folder.
#[must_use]
pub fn host_config_target_message(directory: &str, name: &str) -> String {
    format!(
        "{directory} is a host configuration folder ({name}/); its CLAUDE.md or AGENTS.md \
         belongs to the folder above it"
    )
}

/// The refusal message for a target inside a host configuration folder:
/// `folder` is that folder's path, `name` its name and `above` the
/// folder that holds it.
#[must_use]
pub fn inside_host_config_message(
    directory: &str,
    folder: &str,
    name: &str,
    above: &str,
) -> String {
    format!(
        "{directory} is inside the host configuration folder {folder} ({name}/); hosts load \
         files there as configuration of {above}, so the new identity would leak into that \
         project. Choose a folder outside it"
    )
}

/// The refusal message for a target that is the home directory.
#[must_use]
pub fn home_target_message(directory: &str) -> String {
    format!(
        "{directory} is the home directory; an agent there would enclose every project below it"
    )
}

/// The refusal message for a target that already holds identity files.
/// `files` are their names; `why` is empty or starts with a space.
#[must_use]
pub fn identity_exists_message(agent: &str, files: &[&str], why: &str) -> String {
    format!(
        "{agent} already has identity files ({}), so it is an existing agent{why}; adapt it \
         instead",
        files.join(", ")
    )
}

/// The refusal message for a create target below another agent that has
/// no consent of its own.
#[must_use]
pub fn nested_create_message(directory: &str, parent: &str) -> String {
    format!(
        "{directory} is inside the agent at {parent}; create it separately with its own consent"
    )
}

/// Why a `sources` entry is not a real entry of the new agent's
/// workspace. `Display` is the text of the [`PlanRefusal::InvalidSource`]
/// message after the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceProblem {
    /// The entry does not exist in the folder.
    Missing,
    /// The entry, or a folder on its way, is a symlink; `shown` is the
    /// relative path of the link.
    Symlink {
        /// The relative path of the symlink.
        shown: String,
    },
    /// The entry lies inside the child agent `child`.
    InsideChild {
        /// The child agent's relative path.
        child: String,
    },
    /// The entry holds the child agent `child`.
    ContainsChild {
        /// The child agent's relative path.
        child: String,
    },
}

impl SourceProblem {
    /// The [`PlanRefusal::InvalidSource`] for `entry` (the source's
    /// relative path) at position `index` of the sources.
    #[must_use]
    pub fn refusal(&self, index: usize, entry: &str, directory: &str) -> PlanRefusal {
        let reason = match self {
            Self::Missing => format!("{entry} does not exist in {directory}"),
            Self::Symlink { shown } => {
                format!("{shown} is a symlink; sources are never read through symlinks")
            }
            Self::InsideChild { child } => format!(
                "{entry} is inside the child agent {child}/; a child agent's workspace is \
                 excluded from this agent and must not be handled"
            ),
            Self::ContainsChild { child } => format!(
                "{entry} contains the child agent {child}/; a child agent's workspace is \
                 excluded from this agent and must not be handled (name the files or folders \
                 beside it instead)"
            ),
        };
        PlanRefusal::InvalidSource(format!("sources[{index}].text: {reason}"))
    }
}

/// A plan's id: the SHA-256 of its [`canonical_bytes`], rendered as
/// lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanId(pub [u8; 32]);

impl PlanId {
    /// The id for a digest of [`canonical_bytes`].
    #[must_use]
    pub const fn from_digest(digest: Sha256Digest) -> Self {
        Self(digest.0)
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Sha256Digest(self.0).to_hex().fmt(f)
    }
}

/// A dry-run plan that was not refused. `operations` is empty when the
/// directory holds no identity file (not an agent root) or is already
/// canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The canonical workspace root the plan was made for, as lossy
    /// `/`-separated text. Display only: the id covers the root's raw
    /// platform bytes instead, so a preview never authorises a write in
    /// another workspace, even one whose name differs only in a byte
    /// that is not valid UTF-8.
    pub workspace: String,
    /// The agent root's path, relative to the workspace (`.` for the
    /// workspace itself).
    pub agent: String,
    /// The fingerprint of each identity file found, keyed by file name.
    /// A file the plan does not write is still listed when it exists:
    /// changing it changes the id.
    pub inputs: BTreeMap<String, InputFingerprint>,
    /// The operations to apply, in order: identity files first, the
    /// `skills` block last.
    pub operations: Vec<Operation>,
    /// The block the plan adds, replaces or migrates, when it has a
    /// [`Operation::WriteBlock`]. Display only: the id covers the block
    /// through its operation.
    pub block_diff: Option<BlockDiff>,
    /// What the dry run reports and never acts on. Never part of the
    /// id.
    pub notices: Vec<Notice>,
    /// The id of this plan.
    pub id: PlanId,
}

/// The text the plan id is the SHA-256 of.
///
/// The form is line-based and every free-text field is length-prefixed,
/// so no two different plans share a text:
///
/// ```text
/// omnifrons-plan-v4
/// workspace <len>:<canonical workspace root, raw platform bytes>
/// agent <len>:<path>
/// inputs <count>
/// input <len>:<file name> <fingerprint>      (sorted by file name)
/// operations <count>
/// <operation line>                           (in plan order)
/// ```
///
/// The workspace root is the platform's raw path bytes (on Unix, exactly
/// the bytes of the path), so two roots that differ only in a byte that is
/// not valid UTF-8 never share an id. A plan's own display text for the
/// root is lossy and is never hashed. Each line ends in `\n`. The fingerprint and operation lines are
/// [`InputFingerprint::canonical`] and [`Operation::canonical`]. The
/// operation lines are:
///
/// ```text
/// write-agents source=<file> after=<hex> before=<fingerprint>
/// write-generated path=<file> template=<len>:<version> answers=<hex> after=<hex> before=<fingerprint>
/// write-wrapper path=<file> after=<hex> before=<fingerprint>
/// write-block path=<file> kind=skills version=<len>:<template version> mode=<mode> body=<hex> after=<hex> before=<fingerprint>
/// ```
///
/// A template version is framed as `<len>:<text>` (its length in bytes),
/// so a version holding a space or a line break cannot read as another
/// field or line.
///
/// A block operation therefore changes the id when the block's body, the
/// template version, the mode or the resulting file changes. The diff
/// and the notices of a [`Plan`] are not part of the text. The form does
/// not reproduce any earlier tool's id layout.
#[must_use]
pub fn canonical_bytes(
    workspace: &[u8],
    agent: &str,
    inputs: &BTreeMap<String, InputFingerprint>,
    operations: &[Operation],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(
        format!("{CANONICAL_FORM_TAG}\nworkspace {}:", workspace.len()).as_bytes(),
    );
    out.extend_from_slice(workspace);
    out.push(b'\n');
    let mut text = String::new();
    // Writing into a `String` never fails.
    let _ = writeln!(text, "agent {}:{agent}", agent.len());
    let _ = writeln!(text, "inputs {}", inputs.len());
    for (name, fingerprint) in inputs {
        let _ = writeln!(
            text,
            "input {}:{name} {}",
            name.len(),
            fingerprint.canonical()
        );
    }
    let _ = writeln!(text, "operations {}", operations.len());
    for operation in operations {
        let _ = writeln!(text, "{}", operation.canonical());
    }
    out.extend_from_slice(text.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> Sha256Digest {
        Sha256Digest([byte; 32])
    }

    #[test]
    fn canonical_bytes_are_sorted_and_length_prefixed() {
        let mut inputs = BTreeMap::new();
        inputs.insert("CLAUDE.md".to_string(), InputFingerprint::Irregular);
        inputs.insert(
            "AGENTS.md".to_string(),
            InputFingerprint::Symlink("a\nb".to_string()),
        );
        let text = String::from_utf8(canonical_bytes(b"/w", ".", &inputs, &[])).unwrap();
        assert_eq!(
            text,
            "omnifrons-plan-v4\nworkspace 2:/w\nagent 1:.\ninputs 2\ninput 9:AGENTS.md symlink 3:a\nb\n\
             input 9:CLAUDE.md irregular\noperations 0\n"
        );
    }

    #[test]
    fn a_root_with_a_non_utf8_byte_changes_the_workspace_line_bytes() {
        let none = BTreeMap::new();
        let high = canonical_bytes(b"/w\xff", ".", &none, &[]);
        let low = canonical_bytes(b"/w\xfe", ".", &none, &[]);
        let header = b"omnifrons-plan-v4\nworkspace 3:/w\xff\nagent 1:.\n";
        assert!(high.starts_with(header), "{high:?}");
        assert_ne!(high, low);
        // The prefix counts raw bytes, not the lossy text's characters.
        let lossy = canonical_bytes("/w\u{fffd}".as_bytes(), ".", &none, &[]);
        assert!(lossy.starts_with(b"omnifrons-plan-v4\nworkspace 5:"));
        assert_ne!(high, lossy);
    }

    #[test]
    fn plan_id_displays_as_lowercase_hex() {
        let id = PlanId::from_digest(digest(0xab));
        assert_eq!(id.to_string(), "ab".repeat(32));
    }

    #[test]
    fn refusal_codes_are_the_stable_tokens() {
        assert_eq!(
            PlanRefusal::NestedAgent(String::new()).code(),
            "nested_agent"
        );
        assert_eq!(
            PlanRefusal::EditedBlock(String::new()).code(),
            "edited_block"
        );
    }

    #[test]
    fn every_block_mode_round_trips_through_its_token() {
        for mode in BlockMode::ALL {
            assert_eq!(BlockMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(BlockMode::parse("nope"), None);
        assert_eq!(BlockMode::ALL.len(), 3);
    }
}
