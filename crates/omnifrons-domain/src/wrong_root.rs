//! Wrong-root domain types (spike slice 5d, HAP-001 § Wrong-root
//! detection and remedies, D16 at the post-run-scan half of its default).
//!
//! Framework-independent, like every other module of this crate: the
//! reason a file is `misplaced`, the three remedies HAP-001-R32 fixes, the
//! finding a scan produces -- a project-relative name and the facts taken
//! from the file's own handle, never a device path (RCS-001-R14) -- and
//! the output-discipline report HAP-001-R33 and R34 fix.
//!
//! `misplaced` is a *detection condition*, not a publication state
//! (HAP-001 § Artifact states), so nothing here is a `CandidateState` or an
//! `ArtifactState`: a finding is recomputed by a scan and is never
//! persisted. The one durable fact the remedies leave behind is the ignore
//! decision, which `omnifrons-app`'s `IgnoreLedger` owns.

use std::fmt;

use crate::outbox::{
    ArtifactClass, ContentDigest, DetectedType, OutboxPathError, validate_project_relative,
};
use crate::scope::ScopeMode;

/// Whether a scope's declared write set is the project root itself, or
/// something wider.
///
/// HAP-001-R33 makes `enforced` conditional on both facts -- the mode
/// *and* the write set -- so the write set is a value here rather than an
/// assumption: a sandbox whose declared write set is wider than the
/// project cannot claim that a write outside the project is prevented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclaredWriteSet {
    /// The declared write set is exactly the project root, which contains
    /// the outbox.
    ProjectRoot,
    /// The declared write set reaches beyond the project root.
    Wider,
}

/// Whether writes outside the project root are prevented (`enforced`) or
/// only detected afterwards (`advisory`), reported per scope mode
/// (HAP-001-R33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputDiscipline {
    /// An OS sandbox whose write set is the project root prevents a write
    /// outside it.
    Enforced,
    /// A write outside the project is possible and is detected after the
    /// run, never prevented.
    Advisory,
}

impl OutputDiscipline {
    /// Both values, for exhaustive iteration.
    pub const ALL: [Self; 2] = [Self::Enforced, Self::Advisory];

    /// HAP-001-R34's disclosure, carried in every mode including
    /// `sandbox-enforced`.
    pub const IN_PROJECT_DISCLOSURE: &'static str = "a write inside the project but outside the outbox is detected after the run, never \
         prevented";

    /// HAP-001-R33's additional disclosure, carried under `advisory` only.
    pub const OUTSIDE_PROJECT_DISCLOSURE: &'static str =
        "a write outside the project is possible and is detected after the run, not prevented";

    /// The discipline `mode` reports for a scope whose declared write set
    /// is `write_set` (HAP-001-R33): [`Self::Enforced`] only for
    /// `sandbox-enforced` over the project root, [`Self::Advisory`]
    /// otherwise.
    #[must_use]
    pub const fn for_scope(mode: ScopeMode, write_set: DeclaredWriteSet) -> Self {
        match (mode, write_set) {
            (ScopeMode::SandboxEnforced, DeclaredWriteSet::ProjectRoot) => Self::Enforced,
            _ => Self::Advisory,
        }
    }

    /// This discipline's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::Advisory => "advisory",
        }
    }

    /// Every disclosure this report carries, in a fixed order:
    /// HAP-001-R34's always, HAP-001-R33's under `advisory` too.
    #[must_use]
    pub fn disclosures(&self) -> Vec<&'static str> {
        match self {
            Self::Enforced => vec![Self::IN_PROJECT_DISCLOSURE],
            Self::Advisory => vec![
                Self::IN_PROJECT_DISCLOSURE,
                Self::OUTSIDE_PROJECT_DISCLOSURE,
            ],
        }
    }
}

impl fmt::Display for OutputDiscipline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a file is `misplaced` (HAP-001-R32).
///
/// One variant in this slice. HAP-001-R32's other arms -- another
/// registered project's root, and the vault -- name things this repository
/// does not have (`docs/spike-log.md` § Slice 5d), and a *tracked* path is
/// not a reason at all under the classifier this slice uses: a finding the
/// policy classifies `git-tracked` is never `misplaced` (HAP-001-R1, R3),
/// so it is filtered out before a reason is ever assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrongRootReason {
    /// The file sits inside the project but outside the project's outbox
    /// -- the operative arm of HAP-001-R32, "any location outside the
    /// project's outbox".
    InProjectOutsideOutbox,
}

impl WrongRootReason {
    /// Every reason, for exhaustive iteration.
    pub const ALL: [Self; 1] = [Self::InProjectOutsideOutbox];

    /// This reason's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InProjectOutsideOutbox => "in-project-outside-outbox",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.as_str() == token)
    }
}

impl fmt::Display for WrongRootReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The three remedies a `misplaced` file is offered, and the only three
/// (HAP-001-R32): nothing happens until the user chooses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Remedy {
    /// Move the file into the product's quarantine directory, outside any
    /// workspace (RCS-001-R10).
    Quarantine,
    /// Copy the file into the outbox as an unattributed entry; the
    /// publication transaction runs from step 1 over that entry.
    Publish,
    /// Record the decision against the file's identity and digest; the
    /// file is not offered again until its content changes.
    Ignore,
}

impl Remedy {
    /// Every remedy, in the order HAP-001 § Wrong-root detection and
    /// remedies lists them.
    pub const ALL: [Self; 3] = [Self::Quarantine, Self::Publish, Self::Ignore];

    /// This remedy's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Quarantine => "quarantine",
            Self::Publish => "publish",
            Self::Ignore => "ignore",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|remedy| remedy.as_str() == token)
    }
}

impl fmt::Display for Remedy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One `misplaced` file a wrong-root scan found: the project-relative name
/// it was found at, the facts taken from its own handle, the class the
/// policy assigned, and why it is misplaced.
///
/// Constructible only through [`Self::new`], which validates the name as a
/// project-relative path exactly as `OutboxPath` validates a declaration:
/// never absolute, never escaping through `..`, always `/`-separated,
/// never carrying a control or bidirectional character. An absolute path
/// -- a device path -- can therefore never reach a record, an event, or an
/// IPC payload through a finding (RCS-001-R14, HAP-001 design rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MisplacedFinding {
    name: String,
    /// The number of bytes digested from the file's handle.
    pub size: u64,
    /// The content digest computed from that handle (HAP-001 D13).
    pub digest: ContentDigest,
    /// The content type detected from the same bytes.
    pub detected_type: DetectedType,
    /// The class the project's classification policy assigned.
    pub class: ArtifactClass,
    /// Why the file is misplaced.
    pub reason: WrongRootReason,
}

impl MisplacedFinding {
    /// Build a finding for the file found at the project-relative `name`.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxPathError`] if `name` is empty, absolute, contains
    /// a `..` component or a backslash, or carries a control,
    /// line-separator, or bidirectional character.
    pub fn new(
        name: impl AsRef<str>,
        size: u64,
        digest: ContentDigest,
        detected_type: DetectedType,
        class: ArtifactClass,
        reason: WrongRootReason,
    ) -> Result<Self, OutboxPathError> {
        let name = name.as_ref();
        validate_project_relative(name)?;
        Ok(Self {
            name: name.to_string(),
            size,
            digest,
            detected_type,
            class,
            reason,
        })
    }

    /// The project-relative name the file was found at. Producer-supplied
    /// text: a consumer renders it as plain text only.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether `name` and `digest` name this finding -- the pair a remedy
    /// request binds to, exactly as `artifact_approve` binds a candidate
    /// (HAP-001-R22, R32).
    #[must_use]
    pub fn is_named_by(&self, name: &str, digest: &ContentDigest) -> bool {
        self.name == name && &self.digest == digest
    }
}

/// Whether `class` makes a file found outside the outbox `misplaced`.
///
/// `generated-heavy` and nothing else in this slice:
///
/// * `git-tracked` is never misplaced -- HAP-001-R1 and R3 keep a
///   Git-managed file where it is, and the classifier's own `git-tracked`
///   class is what stands in for tracked-ness here (`docs/spike-log.md` §
///   Slice 5d discloses that this is the policy's class, not the index).
/// * `portable-text` is a Markdown note, which HAP-001-R2 refuses as an
///   artifact of this contract at all.
/// * `executable` and `unclassified` are artifact classes with no
///   destination this slice can offer: HAP-001-R4 routes an executable to
///   quarantine only as a *candidate entry*'s class, and HAP-001-R1 blocks
///   an unclassified entry pending the Asset Policy Owner. Flagging either
///   would offer a publish remedy the approval surface refuses
///   (`artifact_approve` admits `generated-heavy` alone) and would flag
///   every build output in a worktree. Narrowed on purpose, and disclosed.
#[must_use]
pub const fn is_misplaced_class(class: ArtifactClass) -> bool {
    matches!(class, ArtifactClass::GeneratedHeavy)
}

#[cfg(test)]
mod tests {
    use super::{MisplacedFinding, Remedy, WrongRootReason, is_misplaced_class};
    use crate::executable::Sha256Digest;
    use crate::outbox::{ArtifactClass, DetectedType};

    fn finding(name: &str) -> Result<MisplacedFinding, crate::outbox::OutboxPathError> {
        MisplacedFinding::new(
            name,
            1,
            Sha256Digest([7; 32]),
            DetectedType::Pdf,
            ArtifactClass::GeneratedHeavy,
            WrongRootReason::InProjectOutsideOutbox,
        )
    }

    #[test]
    fn a_finding_keeps_the_relative_name_it_was_found_at() {
        let finding = finding("docs/report.pdf").expect("a project-relative name");
        assert_eq!(finding.name(), "docs/report.pdf");
        assert!(finding.is_named_by("docs/report.pdf", &Sha256Digest([7; 32])));
        assert!(!finding.is_named_by("docs/report.pdf", &Sha256Digest([8; 32])));
    }

    #[test]
    fn every_remedy_and_reason_token_round_trips() {
        for remedy in Remedy::ALL {
            assert_eq!(Remedy::parse(remedy.as_str()), Some(remedy));
        }
        for reason in WrongRootReason::ALL {
            assert_eq!(WrongRootReason::parse(reason.as_str()), Some(reason));
        }
    }

    #[test]
    fn only_generated_heavy_is_a_misplaced_class() {
        for class in ArtifactClass::ALL {
            assert_eq!(
                is_misplaced_class(class),
                class == ArtifactClass::GeneratedHeavy,
                "{class:?}"
            );
        }
    }
}
