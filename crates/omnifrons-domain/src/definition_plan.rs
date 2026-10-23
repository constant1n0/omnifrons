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
//! The id is the SHA-256 of [`canonical_bytes`]: the agent path, every
//! input fingerprint, and every operation, in a fixed text form. The
//! form is Omnifrons' own. It does not reproduce the byte layout of any
//! earlier tool, so ids are stable within this implementation only.
//!
//! # Scope of sub-slice 2a
//!
//! Operations cover the identity files only: the canonical `AGENTS.md`,
//! the `CLAUDE.md` wrapper, and the opt-in `GEMINI.md` wrapper. The
//! skills block, the legacy migration, and backups arrive in later
//! sub-slices; the refusals those layers can raise already exist here.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use crate::definition::IdentityKind;
use crate::executable::Sha256Digest;

/// The first line of every [`canonical_bytes`] text. Changing the layout
/// of that text means changing this tag, so an old id can never match a
/// new layout.
pub const CANONICAL_FORM_TAG: &str = "omnifrons-plan-v1";

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

/// One planned identity-file operation, carrying what an apply needs:
/// the file's relative path, the digest of the bytes to write, and the
/// fingerprint the file must still have.
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
}

impl Operation {
    /// The relative path this operation writes.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::WriteAgents { .. } => IdentityKind::Agents.file_name(),
            Self::WriteWrapper { kind, .. } => kind.file_name(),
        }
    }

    /// The fingerprint the written file must still have at apply time.
    #[must_use]
    pub const fn before(&self) -> &InputFingerprint {
        match self {
            Self::WriteAgents { before, .. } | Self::WriteWrapper { before, .. } => before,
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
        }
    }
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
            | Self::DivergingCopy(message) => message,
        }
    }

    /// The refusal for a reason token and message, when `code` is a
    /// block or skill-metadata token a block analysis can produce.
    #[must_use]
    pub fn from_block_code(code: &str, message: String) -> Option<Self> {
        match code {
            "edited_block" => Some(Self::EditedBlock(message)),
            "block_layout" => Some(Self::BlockLayout(message)),
            "unsafe_skill_metadata" => Some(Self::UnsafeSkillMetadata(message)),
            _ => None,
        }
    }
}

impl fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refused ({}): {}", self.code(), self.message())
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
    /// The agent root's path, relative to the workspace (`.` for the
    /// workspace itself).
    pub agent: String,
    /// The fingerprint of each identity file found, keyed by file name.
    /// A file the plan does not write is still listed when it exists:
    /// changing it changes the id.
    pub inputs: BTreeMap<String, InputFingerprint>,
    /// The operations to apply, in order.
    pub operations: Vec<Operation>,
    /// The id of this plan.
    pub id: PlanId,
}

/// The text the plan id is the SHA-256 of.
///
/// The form is line-based and every free-text field is length-prefixed,
/// so no two different plans share a text:
///
/// ```text
/// omnifrons-plan-v1
/// agent <len>:<path>
/// inputs <count>
/// input <len>:<file name> <fingerprint>      (sorted by file name)
/// operations <count>
/// <operation line>                           (in plan order)
/// ```
///
/// Each line ends in `\n`. The fingerprint and operation lines are
/// [`InputFingerprint::canonical`] and [`Operation::canonical`]. The
/// form does not reproduce any earlier tool's id layout.
#[must_use]
pub fn canonical_bytes(
    agent: &str,
    inputs: &BTreeMap<String, InputFingerprint>,
    operations: &[Operation],
) -> Vec<u8> {
    let mut text = String::new();
    // Writing into a `String` never fails.
    let _ = writeln!(text, "{CANONICAL_FORM_TAG}");
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
    text.into_bytes()
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
        let text = String::from_utf8(canonical_bytes(".", &inputs, &[])).unwrap();
        assert_eq!(
            text,
            "omnifrons-plan-v1\nagent 1:.\ninputs 2\ninput 9:AGENTS.md symlink 3:a\nb\n\
             input 9:CLAUDE.md irregular\noperations 0\n"
        );
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
            PlanRefusal::from_block_code("edited_block", "m".into()),
            Some(PlanRefusal::EditedBlock("m".into()))
        );
        assert_eq!(PlanRefusal::from_block_code("other", "m".into()), None);
    }
}
