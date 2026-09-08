//! The classification policy (spike slice 5, HAP-001 § Artifact classes
//! and classification policy): a per-project table that maps a detected
//! type, an extension, and a size to an artifact class, plus the declared
//! outbox path. A pure classifier -- classification only, no publication
//! -- and the [`OutboxPolicyStore`] port that loads a project's policy
//! from under `.omnifrons/` (implemented by `omnifrons-adapters`'
//! `JsonOutboxPolicyStore`).
//!
//! Two rules are fixed above every row, as HAP-001-R2 and R4 require:
//! a Markdown note is never `generated-heavy` (it is knowledge-plane
//! content, [`ArtifactClass::PortableText`]), and an executable --
//! identified by content or by extension -- is always
//! [`ArtifactClass::Executable`], quarantine by default. A project row
//! that contradicts either is rejected when the policy is built
//! ([`PolicyViolation`]), never silently ignored: HAP-001-R4 admits an
//! executable routing row only when it records the Asset Policy Owner's
//! approval, which this slice does not model, so every such row is
//! refused.

use omnifrons_domain::outbox::{
    ArtifactClass, DetectedType, OutboxPath, OutboxPathError, has_extension,
};
use omnifrons_domain::publication::AssetRootId;

use crate::harness_adapter::WorkspaceRoot;

/// Where a project's policy lives, relative to the project root
/// (HAP-001-R8: under the reserved `.omnifrons/` namespace, so the outbox
/// location is discoverable from the folder alone).
pub const POLICY_FILE_PATH: &str = ".omnifrons/asset-policy.json";

/// The only `schema` value this version of the policy accepts; any other
/// value is corrupt rather than guessed at, exactly like the approval
/// store's own schema discipline.
pub const POLICY_SCHEMA_VERSION: u32 = 1;

/// The largest policy file a store reads, inclusive (1 MiB): a policy is a
/// small table, and a synchronized file is untrusted content, so the read
/// is bounded before any byte is parsed (HAP-001-R40).
pub const POLICY_MAX_BYTES: u64 = 1024 * 1024;

/// The largest plain-text file the default `git-tracked` row claims,
/// inclusive: source and configuration text small enough to live under
/// Git (HAP-001's `portable` class).
pub const GIT_TRACKED_TEXT_MAX_BYTES: u64 = 1024 * 1024;

/// Extensions that identify an executable by name alone (HAP-001-R4 "by
/// extension list"): the slice-2 Windows executable allowlist plus the
/// shell-script and installer forms.
const EXECUTABLE_EXTENSIONS: [&str; 7] = ["exe", "com", "bat", "cmd", "sh", "ps1", "msi"];

/// Extensions the default `git-tracked` row claims for plain text under
/// [`GIT_TRACKED_TEXT_MAX_BYTES`].
const GIT_TRACKED_TEXT_EXTENSIONS: [&str; 14] = [
    "rs", "toml", "json", "yaml", "yml", "ini", "cfg", "txt", "py", "js", "ts", "css", "html",
    "xml",
];

/// The detected types the default policy routes to the asset root.
const GENERATED_HEAVY_TYPES: [DetectedType; 16] = [
    DetectedType::Pdf,
    DetectedType::OfficeDocument,
    DetectedType::Png,
    DetectedType::Jpeg,
    DetectedType::Gif,
    DetectedType::Webp,
    DetectedType::Mp3,
    DetectedType::Wav,
    DetectedType::Ogg,
    DetectedType::Flac,
    DetectedType::Mp4,
    DetectedType::Matroska,
    DetectedType::Zip,
    DetectedType::Gzip,
    DetectedType::SevenZip,
    DetectedType::Tar,
];

/// One policy row: matched in order, first match wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRow {
    /// The detected types this row matches; empty matches every type.
    pub types: Vec<DetectedType>,
    /// Extensions (lowercase, no dot) this row additionally requires;
    /// empty matches every name.
    pub extensions: Vec<String>,
    /// The largest size this row matches, inclusive; `None` for no bound.
    pub max_size: Option<u64>,
    /// The class a matching entry takes.
    pub class: ArtifactClass,
}

impl PolicyRow {
    /// Whether this row matches the entry.
    fn matches(&self, name: &str, detected: DetectedType, size: u64) -> bool {
        let type_matches = self.types.is_empty() || self.types.contains(&detected);
        let extension_matches = self.extensions.is_empty() || {
            let extensions: Vec<&str> = self.extensions.iter().map(String::as_str).collect();
            has_extension(name, &extensions)
        };
        let size_matches = self.max_size.is_none_or(|max| size <= max);
        type_matches && extension_matches && size_matches
    }
}

/// Why a project's rows were refused when the policy was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyViolation {
    /// A row routes Markdown to `generated-heavy` (HAP-001-R2).
    MarkdownRoutedHeavy,
    /// A row routes an executable type to any class other than
    /// `executable` (HAP-001-R4: only a row recording the Asset Policy
    /// Owner's approval may, and this slice records none).
    ExecutableRoutedAway,
}

impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::MarkdownRoutedHeavy => "a policy row routes Markdown to generated-heavy",
            Self::ExecutableRoutedAway => {
                "a policy row routes an executable type away from quarantine"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for PolicyViolation {}

/// A project's classification policy: the declared outbox path, the rows
/// -- the project's own first, then the shipped defaults -- and, as of
/// spike slice 5b, the asset root identity publications are destined for
/// (a spike default standing in for the scope's asset binding, HAP-001 §
/// Asset binding; the shipped default declares none, and a project without
/// one has no destination, HAP-001-R6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxPolicy {
    outbox: OutboxPath,
    rows: Vec<PolicyRow>,
    asset_root_id: Option<AssetRootId>,
}

impl OutboxPolicy {
    /// The policy the product ships: the default outbox path and the
    /// default rows alone, no asset root.
    #[must_use]
    pub fn default_policy() -> Self {
        Self {
            outbox: OutboxPath::default_path(),
            rows: default_rows(),
            asset_root_id: None,
        }
    }

    /// This policy with `asset_root_id` as its destination asset root.
    #[must_use]
    pub fn with_asset_root(mut self, asset_root_id: Option<AssetRootId>) -> Self {
        self.asset_root_id = asset_root_id;
        self
    }

    /// The asset root identity publications are destined for, if the
    /// policy declares one.
    #[must_use]
    pub fn asset_root_id(&self) -> Option<&AssetRootId> {
        self.asset_root_id.as_ref()
    }

    /// Build a policy from a declared outbox path and the project's own
    /// rows, which precede the defaults.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyViolation`] if a project row contradicts a fixed
    /// rule.
    pub fn new(outbox: OutboxPath, project_rows: Vec<PolicyRow>) -> Result<Self, PolicyViolation> {
        for row in &project_rows {
            if row.class == ArtifactClass::GeneratedHeavy
                && row.types.contains(&DetectedType::Markdown)
            {
                return Err(PolicyViolation::MarkdownRoutedHeavy);
            }
            if row.class != ArtifactClass::Executable
                && row.types.iter().any(DetectedType::is_executable)
            {
                return Err(PolicyViolation::ExecutableRoutedAway);
            }
        }
        let mut rows = project_rows;
        rows.extend(default_rows());
        Ok(Self {
            outbox,
            rows,
            asset_root_id: None,
        })
    }

    /// The declared outbox path.
    #[must_use]
    pub fn outbox(&self) -> &OutboxPath {
        &self.outbox
    }

    /// Every row, the project's own first.
    #[must_use]
    pub fn rows(&self) -> &[PolicyRow] {
        &self.rows
    }
}

/// The rows the product ships, in order.
fn default_rows() -> Vec<PolicyRow> {
    vec![
        PolicyRow {
            types: GENERATED_HEAVY_TYPES.to_vec(),
            extensions: vec![],
            max_size: None,
            class: ArtifactClass::GeneratedHeavy,
        },
        PolicyRow {
            types: vec![DetectedType::PlainText],
            extensions: GIT_TRACKED_TEXT_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            max_size: Some(GIT_TRACKED_TEXT_MAX_BYTES),
            class: ArtifactClass::GitTracked,
        },
    ]
}

/// A port for classifying a candidate entry: pure over its name, detected
/// type, and size (HAP-001-R1: declared policy, never inference from
/// content shape or location).
pub trait ArtifactClassifier {
    /// The class for an entry named `name` whose handle yielded `detected`
    /// and `size` bytes.
    fn classify(&self, name: &str, detected: DetectedType, size: u64) -> ArtifactClass;
}

impl ArtifactClassifier for OutboxPolicy {
    fn classify(&self, name: &str, detected: DetectedType, size: u64) -> ArtifactClass {
        if detected.is_executable() || has_extension(name, &EXECUTABLE_EXTENSIONS) {
            return ArtifactClass::Executable;
        }
        if detected == DetectedType::Markdown {
            return ArtifactClass::PortableText;
        }
        self.rows
            .iter()
            .find(|row| row.matches(name, detected, size))
            .map_or(ArtifactClass::Unclassified, |row| row.class)
    }
}

/// Why a project's policy could not be loaded.
///
/// Closed and exhaustive, never a raw `io::Error` whose text can carry a
/// real filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// The policy file exists but could not be read.
    #[error("the classification policy could not be read")]
    Unreadable,
    /// The policy file's content is not a valid policy of the accepted
    /// schema.
    #[error("the classification policy is corrupt")]
    Corrupt,
    /// The policy file exceeds [`POLICY_MAX_BYTES`]; nothing of it is
    /// parsed.
    #[error("the classification policy exceeds the size cap")]
    TooLarge,
    /// The declared outbox path is not a valid project-relative path.
    #[error("the declared outbox path is invalid: {0}")]
    InvalidOutboxPath(OutboxPathError),
    /// A project row contradicts a fixed rule.
    #[error("the classification policy is invalid: {0}")]
    Invalid(PolicyViolation),
}

/// A port for loading a project's classification policy from under its
/// `.omnifrons/` namespace: the shipped default when the project declares
/// none, the project's own otherwise.
pub trait OutboxPolicyStore {
    /// Load `project`'s policy.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError`] if a policy file is present but unreadable,
    /// corrupt, or invalid. An absent file is not an error: the default
    /// policy applies.
    fn load(&self, project: &WorkspaceRoot) -> Result<OutboxPolicy, PolicyError>;
}

#[cfg(test)]
mod tests {
    use super::{ArtifactClassifier, OutboxPolicy, PolicyRow};
    use omnifrons_domain::outbox::{ArtifactClass, DetectedType};

    #[test]
    fn a_row_matches_on_type_extension_and_size_together() {
        let row = PolicyRow {
            types: vec![DetectedType::PlainText],
            extensions: vec!["csv".to_string()],
            max_size: Some(100),
            class: ArtifactClass::GeneratedHeavy,
        };
        assert!(row.matches("a.csv", DetectedType::PlainText, 100));
        assert!(!row.matches("a.csv", DetectedType::PlainText, 101));
        assert!(!row.matches("a.txt", DetectedType::PlainText, 10));
        assert!(!row.matches("a.csv", DetectedType::Unknown, 10));
    }

    #[test]
    fn the_default_policy_carries_the_two_default_rows_only() {
        let policy = OutboxPolicy::default_policy();
        assert_eq!(policy.rows().len(), 2);
        assert_eq!(
            policy.classify("x.pdf", DetectedType::Pdf, 1),
            ArtifactClass::GeneratedHeavy
        );
    }
}
