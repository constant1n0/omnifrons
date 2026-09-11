//! The `CatalogRepair` port (spike slice 5e, HAP-001-R23, R39, R40): the
//! two **non-deleting** repairs of a `Corrupt` Catalog, previewed and then
//! applied under a digest binding.
//!
//! Slice 5b left a corrupt `.omnifrons/catalog.jsonl` fail-closed and
//! repaired by hand, naming the three rules its adapter's module doc
//! spells out. Two of them delete nothing:
//!
//! - **`DuplicateRecord`** -- the later of two `record` lines carrying one
//!   publication identity **under the same `assetRootId`**, so both lines
//!   carry one `catalogId`. The surviving line still registers that
//!   identity under that asset root, so no registered artifact is
//!   removed.
//! - **`DanglingAlias`** -- an `alias` no earlier `record` carries. An
//!   alias is a display name under HAP-001-R23's "MAY add a display-name
//!   alias"; dropping it removes no registration.
//!
//! The third is **refused, not implemented**: a `record` whose
//! `catalogId` is not `<assetRootId>/<publicationId>` is the *only* line
//! for that identity, so dropping it deletes a registered artifact, and
//! HAP-001-R39 says a registered artifact is deleted only through
//! MRP-001's tombstone kind `artifact`. Such a line is reported by the
//! preview, refuses the whole repair, and is left exactly where it is --
//! the catalog stays `Corrupt` and the case needs an owner decision. A
//! line that does not parse at all is refused the same way: the Catalog
//! is synchronized, portable, untrusted content (HAP-001-R40), so
//! widening the repair into "drop anything unreadable" would make it a
//! silent record-deletion path driven by content.
//!
//! A fourth line is refused for the same reason as the third, and it is
//! why `DuplicateRecord` is keyed on the whole `catalogId` rather than on
//! the publication identity alone: HAP-001-R23 derives that identity from
//! the project and the content digest, with the asset root **outside** the
//! preimage, while a registered artifact's identity is
//! `CatalogId { asset_root_id, publication_id }`. Two `record` lines
//! sharing one `publicationId` under different `assetRootId`s are
//! therefore two registered artifacts, and the later one is refused
//! (`DuplicateAcrossAssetRoots`), never dropped.
//!
//! The discipline is the guidance installer's, which this repository
//! already proved (HAP-001 D18, spike slice 5c): **preview, digest
//! binding, explicit apply**. [`CatalogRepair::preview`] returns the
//! lines it would drop together with the digest of the bytes it read
//! them from; [`CatalogRepair::repair`] takes that digest back and
//! refuses with [`CatalogRepairError::Changed`] when the file is no
//! longer those bytes, exactly as `ProjectTextFile`'s `expected` does --
//! and for the same reason: the Catalog travels with the workspace, so
//! another device's sync can land between the preview and the apply.

use std::time::SystemTime;

use omnifrons_domain::outbox::ContentDigest;
use omnifrons_domain::publication::PublicationIdentity;

use crate::work_area::WorkAreaRoot;

/// The base name of the pre-repair copy written into the work area's
/// `recovery/` directory: `catalog-<nanoseconds since the epoch>.jsonl`.
pub const REPAIR_COPY_PREFIX: &str = "catalog-";

/// That copy's extension.
pub const REPAIR_COPY_SUFFIX: &str = ".jsonl";

/// A rule the repair may apply, because applying it deletes no
/// registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairRule {
    /// The later of two `record` lines carrying one publication identity.
    DuplicateRecord,
    /// An `alias` line no earlier `record` line carries.
    DanglingAlias,
}

impl RepairRule {
    /// Every rule, so a caller that renders them cannot silently miss one
    /// a later slice adds.
    pub const ALL: [Self; 2] = [Self::DuplicateRecord, Self::DanglingAlias];

    /// This rule's fixed wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateRecord => "duplicate-record",
            Self::DanglingAlias => "dangling-alias",
        }
    }
}

/// Why a line refuses the whole repair instead of being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairRefusal {
    /// A `record` whose `catalogId` is not
    /// `<assetRootId>/<publicationId>`: the only line for that identity,
    /// so dropping it would delete a registered artifact (HAP-001-R39).
    CatalogIdMismatch,
    /// A `record` carrying a `publicationId` an earlier `record` already
    /// carries, under a **different** `assetRootId`. HAP-001-R23 derives
    /// the publication identity from the project and the content digest,
    /// with the asset root outside that preimage, so the two lines are
    /// two `catalogId`s -- two registered artifacts -- and not one
    /// registration written twice. Dropping the later one would delete
    /// the second asset root's registration, which is HAP-001-R39's
    /// tombstone again.
    DuplicateAcrossAssetRoots,
    /// A line that does not decode into this version's shape at all.
    /// Never dropped: the Catalog is untrusted synchronized content
    /// (HAP-001-R40).
    Unparsable,
}

impl RepairRefusal {
    /// Every refusal, for the same reason as [`RepairRule::ALL`].
    pub const ALL: [Self; 3] = [
        Self::CatalogIdMismatch,
        Self::DuplicateAcrossAssetRoots,
        Self::Unparsable,
    ];

    /// This refusal's fixed wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogIdMismatch => "catalog-id-mismatch",
            Self::DuplicateAcrossAssetRoots => "duplicate-across-asset-roots",
            Self::Unparsable => "unparsable",
        }
    }
}

/// One line the repair would drop, named by its one-based line number --
/// never by its content, which is untrusted synchronized text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedLine {
    /// The line's one-based number in the file the plan was read from.
    pub line: u32,
    /// Why it may be dropped.
    pub rule: RepairRule,
    /// The publication identity the line names, when it is one this
    /// version can parse; `None` for an alias whose identity is not 64
    /// hex characters (which is exactly why no record carries it).
    pub publication_id: Option<PublicationIdentity>,
}

/// One line that refuses the repair, named the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedLine {
    /// The line's one-based number.
    pub line: u32,
    /// Why the repair will not touch it.
    pub refusal: RepairRefusal,
}

/// What a repair would do to the exact bytes it read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairPlan {
    /// The digest of the file the plan was made from. An apply must send
    /// this back; anything else is [`CatalogRepairError::Changed`].
    pub sha256: ContentDigest,
    /// The lines the repair would drop.
    pub drops: Vec<DroppedLine>,
    /// The lines that refuse it.
    pub refusals: Vec<RefusedLine>,
    /// How many registrations the repaired catalog would carry -- the
    /// distinct `catalogId`s the **kept** `record` lines register.
    ///
    /// On a plan that carries a refusal this is an undercount of what the
    /// file holds: a refused `record` line is not counted, although the
    /// refusal is exactly what leaves it in place. That is deliberate --
    /// the number describes the catalog a repair would leave, and a
    /// refused plan leaves the file untouched -- so it is meaningful only
    /// when [`Self::is_repairable`] is `true`.
    pub kept_records: usize,
}

impl RepairPlan {
    /// Whether [`CatalogRepair::repair`] would apply this plan: nothing
    /// refuses it, and there is something to drop.
    #[must_use]
    pub fn is_repairable(&self) -> bool {
        self.refusals.is_empty() && !self.drops.is_empty()
    }
}

/// What an applied repair did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairOutcome {
    /// The digest of the bytes that were repaired -- the one the caller
    /// bound to, and the content of the copy kept in the work area.
    pub original_sha256: ContentDigest,
    /// The digest of the file as it now stands.
    pub sha256: ContentDigest,
    /// How many lines were dropped.
    pub dropped_lines: usize,
    /// How many registrations the repaired catalog carries.
    pub kept_records: usize,
}

/// Why a repair could not be previewed or applied. Closed and
/// exhaustive, never a raw `io::Error` whose text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogRepairError {
    /// The catalog could not be read.
    #[error("the catalog could not be read")]
    Unreadable,
    /// There is no catalog file to repair.
    #[error("the project has no catalog to repair")]
    Absent,
    /// The file is no longer the bytes the caller planned from.
    #[error("the catalog changed since it was previewed")]
    Changed,
    /// A line the repair may not drop (HAP-001-R39, R40).
    #[error("a line of the catalog needs an owner decision, not a repair")]
    Refused(RepairRefusal),
    /// Nothing in the catalog matches either repair rule.
    #[error("the catalog needs no repair")]
    NothingToRepair,
    /// The lines the plan would keep still do not read as a catalog. A
    /// guard: with both rules applied nothing that makes a catalog
    /// `Corrupt` should survive, so reaching this means the rules and the
    /// reader have drifted apart, and refusing beats rewriting the
    /// project's own file into a file that is still corrupt.
    #[error("the repair would not yield a readable catalog")]
    Unrepairable,
    /// The pre-repair copy could not be written into the work area.
    /// Nothing was rewritten: the copy comes first, always.
    #[error("the pre-repair copy could not be written to the product work area")]
    CopyFailed,
    /// The repaired catalog could not be written.
    #[error("the catalog could not be written")]
    WriteFailed,
}

/// A port over one project's Catalog file as *lines*, for the repair the
/// record-level [`crate::catalog_store::CatalogStore`] cannot express: a
/// store that fails closed on a corrupt file has, by construction, no way
/// to show what is wrong with it.
pub trait CatalogRepair {
    /// What a repair would drop, and the digest of the bytes that answer
    /// was taken from.
    ///
    /// # Errors
    ///
    /// [`CatalogRepairError::Absent`] when the project has no catalog, or
    /// [`CatalogRepairError::Unreadable`].
    fn preview(&self) -> Result<RepairPlan, CatalogRepairError>;

    /// Apply the repair to the bytes whose digest is `expected`: the
    /// original is copied into `work_area`'s `recovery/` directory first,
    /// named from `at`, and only then is the file rewritten without the
    /// dropped lines.
    ///
    /// # Errors
    ///
    /// [`CatalogRepairError::Changed`] when the file is no longer
    /// `expected`, [`CatalogRepairError::Refused`] when a line may not be
    /// dropped, [`CatalogRepairError::NothingToRepair`] when no rule
    /// applies, and the copy, write, and read failures above.
    fn repair(
        &mut self,
        work_area: &WorkAreaRoot,
        expected: &ContentDigest,
        at: SystemTime,
    ) -> Result<RepairOutcome, CatalogRepairError>;
}
