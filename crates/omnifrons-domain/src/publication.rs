//! Publication domain types (spike slice 5b, HAP-001 § Publication
//! transaction, § Catalog record, § Artifact states).
//!
//! Framework-independent (`omnifrons-domain` depends on `std` and
//! `thiserror` only): the identities a publication is named by -- the
//! project identity, the publication identity derived from it and the
//! content digest (HAP-001-R23), the asset root identity, the artifact
//! Catalog identity, the provider locator -- the closed state tokens the
//! shell puts on the wire (HAP-001-R26), the display name sanitized under
//! RCS-001's file-name rule (HAP-001-R24), the Catalog record
//! (HAP-001 § Catalog record), the portable reference (AEC-001's `ref`
//! shape), the artifact approval bound to identity facts (HAP-001-R22),
//! and the publication journal's entry shapes (HAP-001-R29).
//!
//! Nothing here hashes, opens, copies, or persists anything: the digests
//! the identities are derived from are computed one layer up, through
//! `omnifrons-app`'s `ContentHasher` port, from the exact preimages this
//! module spells out ([`publication_identity_preimage`],
//! [`artifact_approval_id_preimage`]).

use std::fmt;
use std::time::{Duration, SystemTime};

use crate::adapter::AdapterId;
use crate::executable::{ApprovalId, DeviceLocalUser, Sha256Digest};
use crate::outbox::{ArtifactClass, Attribution, ContentDigest, DetectedType, RunId};

/// The record schema integer every Catalog record this version writes
/// carries (HAP-001 § Catalog record, `record_version`; the version domain
/// proposed to the compatibility policy).
pub const RECORD_VERSION: u32 = 1;

/// The domain tag mixed into an artifact approval id's preimage, so the
/// id can never collide with the slice-2 executable approval id
/// (`omnifrons-approval-v1`) or any other hash computed in this codebase.
pub const ARTIFACT_APPROVAL_ID_DOMAIN: &[u8] = b"omnifrons-artifact-approval-v1";

/// The algorithm identifier a Catalog record carries beside its digest
/// (HAP-001 D13: SHA-256, recorded with its identifier so a record can
/// carry a second algorithm later).
pub const DIGEST_ALGORITHM_SHA256: &str = "sha256";

/// The registered context scope's logical identity the Catalog binds an
/// asset root to (HAP-001 § Definitions, "Project identity"): never a
/// path. In this slice it is the digest of the workspace root's canonical
/// path, a spike default disclosed in `docs/spike-log.md` § Slice 5b; the
/// non-forgeable scope identity is context-orb.md's and RSP-001-R13's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectIdentity(pub Sha256Digest);

impl ProjectIdentity {
    /// The 64-character lowercase hex form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Parse the hex form, or `None` if `text` is not 64 hex characters.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        Sha256Digest::from_hex(text).map(Self)
    }
}

/// The deterministic identifier of a published artifact (HAP-001-R23):
/// `sha256(project identity ‖ content digest)`, the idempotency key of the
/// publication transaction and the `id` of the artifact's `ref`. Derived
/// through `omnifrons-app` from [`publication_identity_preimage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicationIdentity(pub Sha256Digest);

impl PublicationIdentity {
    /// The 64-character lowercase hex form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Parse the hex form, or `None` if `text` is not 64 hex characters.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        Sha256Digest::from_hex(text).map(Self)
    }
}

impl fmt::Display for PublicationIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// The exact bytes a publication identity is the SHA-256 of: the project
/// identity's 32 bytes followed by the content digest's 32 bytes
/// (HAP-001-R23 "derived deterministically from the project identity and
/// the content digest"), nothing else.
#[must_use]
pub fn publication_identity_preimage(
    project: &ProjectIdentity,
    digest: &ContentDigest,
) -> [u8; 64] {
    let mut preimage = [0u8; 64];
    preimage[..32].copy_from_slice(&project.0.0);
    preimage[32..].copy_from_slice(&digest.0);
    preimage
}

/// Why [`AssetRootId::new`] rejected a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRootIdError {
    /// The token is empty.
    Empty,
    /// The token exceeds [`AssetRootId::MAX_CHARS`] characters.
    TooLong,
    /// The token contains a character outside `[A-Za-z0-9_-]`.
    InvalidCharacter,
}

impl fmt::Display for AssetRootIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "the asset root id is empty",
            Self::TooLong => "the asset root id exceeds 64 characters",
            Self::InvalidCharacter => {
                "the asset root id must contain only ASCII letters, digits, - and _"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for AssetRootIdError {}

/// An asset root's own Context Catalog identity, `asset_root_id`
/// (HAP-001 § Definitions): one token of `[A-Za-z0-9_-]{1,64}`, the same
/// shape as a run id, so it is never a separator, never `.` or `..`, and
/// can name a device-local directory without a second validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetRootId(String);

impl AssetRootId {
    /// The maximum length of an asset root id, inclusive.
    pub const MAX_CHARS: usize = 64;

    /// Validate `token` as an asset root id.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRootIdError`] if `token` is empty, longer than
    /// [`Self::MAX_CHARS`], or contains a character outside
    /// `[A-Za-z0-9_-]`.
    pub fn new(token: impl Into<String>) -> Result<Self, AssetRootIdError> {
        let token = token.into();
        if token.is_empty() {
            return Err(AssetRootIdError::Empty);
        }
        if token.len() > Self::MAX_CHARS {
            return Err(AssetRootIdError::TooLong);
        }
        if !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(AssetRootIdError::InvalidCharacter);
        }
        Ok(Self(token))
    }

    /// This id's token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetRootId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A published artifact's Context Catalog identity, `catalog_id`
/// (HAP-001 § Definitions): the asset root identity paired with the
/// publication identity, rendered `<asset root id>/<publication hex>`.
/// It is what the artifact's `ref` carries as `locator`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogId {
    /// The asset root the artifact was published to.
    pub asset_root_id: AssetRootId,
    /// The publication identity.
    pub publication_id: PublicationIdentity,
}

impl fmt::Display for CatalogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.asset_root_id, self.publication_id)
    }
}

/// The adapter-scoped, opaque identifier of an object in a blob provider
/// (HAP-001 § Definitions, "Provider locator"): meaningful only to the
/// adapter that issued it, never interpreted here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderLocator(String);

impl ProviderLocator {
    /// Wrap an adapter-issued locator.
    #[must_use]
    pub fn new(locator: impl Into<String>) -> Self {
        Self(locator.into())
    }

    /// The locator's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The record's `provider_state` (HAP-001 § Catalog record): `failed` is
/// reserved for a non-retryable refusal (HAP-001-R30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderState {
    /// The upload has not completed (no remote, or not yet attempted).
    Pending,
    /// The adapter observed its declared confirmation (HAP-001-R25).
    Synced,
    /// The adapter reported a non-retryable refusal.
    Failed,
    /// The provider was unreachable at the last attempt.
    Unavailable,
}

impl ProviderState {
    /// Every provider state, for exhaustive iteration.
    pub const ALL: [Self; 4] = [Self::Pending, Self::Synced, Self::Failed, Self::Unavailable];

    /// This state's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Synced => "synced",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == token)
    }
}

/// The artifact states this slice exposes, spelled as HAP-001's signal
/// mapping spells them (HAP-001-R26: no other token reaches a product
/// surface). The four publication states are distinct facts, never
/// collapsed; the rest are the failure and pending conditions the
/// transaction of this slice can end in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactState {
    /// A candidate entry exists in the outbox.
    Candidate,
    /// The published copy in the device asset path verified against the
    /// validation digest (HAP-001-R21).
    PublishedLocal,
    /// The Catalog record exists and the portable reference is issued
    /// (HAP-001-R24).
    Registered,
    /// The adapter observed its declared confirmation (HAP-001-R25);
    /// never produced by this slice's local-directory adapter.
    ProviderSynced,
    /// Registration failed after a verified `published-local`; resumes on
    /// restart (HAP-001-R29).
    RegistrationPending,
    /// The approval was refused, or the entry cannot be approved or
    /// published in its state (HAP-001-R22).
    Refused,
    /// The published copy's digest differs from the validation digest, or
    /// the held handle's bytes changed since they were digested
    /// (HAP-001-R21).
    IntegrityMismatch,
    /// A repeated request for an existing publication identity
    /// (HAP-001-R23).
    DuplicatePublication,
    /// The path no longer names the handle's file identity, or the handle
    /// is not a regular file (HAP-001-R15, R18).
    OutboxEscape,
    /// The handle's link count is greater than one (HAP-001-R20).
    OutboxLinked,
}

impl ArtifactState {
    /// Every state, for exhaustive iteration and parsing.
    pub const ALL: [Self; 10] = [
        Self::Candidate,
        Self::PublishedLocal,
        Self::Registered,
        Self::ProviderSynced,
        Self::RegistrationPending,
        Self::Refused,
        Self::IntegrityMismatch,
        Self::DuplicatePublication,
        Self::OutboxEscape,
        Self::OutboxLinked,
    ];

    /// This state's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::PublishedLocal => "published-local",
            Self::Registered => "registered",
            Self::ProviderSynced => "provider-synced",
            Self::RegistrationPending => "registration-pending",
            Self::Refused => "refused",
            Self::IntegrityMismatch => "integrity-mismatch",
            Self::DuplicatePublication => "duplicate-publication",
            Self::OutboxEscape => "outbox-escape",
            Self::OutboxLinked => "outbox-linked",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == token)
    }
}

impl fmt::Display for ArtifactState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The Windows reserved device names a display name must never equal
/// (case-insensitively, with or without an extension).
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A producer-supplied display name, sanitized under RCS-001's file-name
/// rule before it is displayed or referenced (HAP-001-R24): path-traversal
/// sequences removed, reserved device names neutralized, bidirectional
/// override and isolate characters and every control character stripped,
/// and the length capped. Constructible only through [`Self::sanitize`],
/// so every live value has been through the rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

impl DisplayName {
    /// The longest sanitized name, in bytes: the common filesystem limit
    /// on one name component.
    pub const MAX_BYTES: usize = 255;

    /// The name a sanitization that removes everything falls back to.
    pub const EMPTY_FALLBACK: &'static str = "unnamed";

    /// Sanitize `raw`. Never fails and never returns an empty name.
    #[must_use]
    pub fn sanitize(raw: &str) -> Self {
        // 1. Traversal: split on both separators, drop empty, `.` and `..`
        //    components, and join what remains with `_` so the result is
        //    one name that can never be resolved as a path.
        let components: Vec<&str> = raw
            .split(['/', '\\'])
            .filter(|component| !matches!(*component, "" | "." | ".."))
            .collect();
        let joined = components.join("_");

        // 2. Controls and bidirectional overrides/isolates.
        let mut stripped: String = joined
            .chars()
            .filter(|c| !c.is_control() && !is_bidi_control(*c))
            .collect();

        // 3. Length cap on a character boundary.
        if stripped.len() > Self::MAX_BYTES {
            let mut cut = Self::MAX_BYTES;
            while !stripped.is_char_boundary(cut) {
                cut -= 1;
            }
            stripped.truncate(cut);
        }

        // 4. Trailing dots and spaces (dropped silently by Windows).
        let trimmed = stripped.trim_end_matches(['.', ' ']).trim_start();
        if trimmed.is_empty() {
            return Self(Self::EMPTY_FALLBACK.to_string());
        }

        // 5. Reserved device names: the stem before the first dot.
        let stem = trimmed.split('.').next().unwrap_or(trimmed);
        if RESERVED_DEVICE_NAMES
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(stem))
        {
            return Self(format!("_{trimmed}"));
        }
        Self(trimmed.to_string())
    }

    /// The sanitized name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `c` is a bidirectional override, embedding, or isolate control
/// (U+202A..=U+202E, U+2066..=U+2069) -- the characters that let a name
/// render in a different order than it is stored.
const fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// An AEC-001 `ref` of kind `artifact` (HAP-001 § Definitions, "Portable
/// reference"): `id` is the publication identity and `locator` the
/// artifact Catalog identity -- the only form in which an artifact is
/// named outside the device that holds it. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortableReference {
    locator: CatalogId,
}

impl PortableReference {
    /// The `kind` every artifact reference carries.
    pub const KIND: &'static str = "artifact";

    /// A reference to the artifact `locator` names.
    #[must_use]
    pub const fn new(locator: CatalogId) -> Self {
        Self { locator }
    }

    /// The reference's `id`: the publication identity.
    #[must_use]
    pub const fn id(&self) -> &PublicationIdentity {
        &self.locator.publication_id
    }

    /// The reference's `locator`: the artifact Catalog identity.
    #[must_use]
    pub const fn locator(&self) -> &CatalogId {
        &self.locator
    }
}

/// One recorded state transition with its UTC time (HAP-001-R36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// The state entered.
    pub state: ArtifactState,
    /// When it was entered.
    pub at: SystemTime,
}

/// Who produced an artifact (HAP-001-R36): the run whose own proposal
/// named it by digest, with the adapter and the executable approval that
/// governed the launch, or the unattributed fact with the run subdirectory
/// the entry was found under as a location fact only (HAP-001-R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Producer {
    /// An attributed entry.
    Run {
        /// The producing run.
        run_id: RunId,
        /// The adapter that ran it, when the run record kept it.
        adapter_id: Option<AdapterId>,
        /// The executable approval that governed the launch, when the run
        /// record kept it -- never the executable's path.
        executable_approval: Option<ApprovalId>,
    },
    /// An unattributed entry.
    Unattributed {
        /// The run subdirectory the entry was found under, as a location
        /// fact only, never provenance.
        found_under: Option<RunId>,
    },
}

/// The record's provenance (HAP-001-R36): the scope, the producer, and
/// the UTC time of every state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The scope the artifact was published under (the project identity
    /// in this slice).
    pub scope_id: ProjectIdentity,
    /// Who produced it.
    pub producer: Producer,
    /// Every state transition so far, in order.
    pub transitions: Vec<Transition>,
}

/// The record's `provider` field (HAP-001 § Catalog record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRecord {
    /// The provider adapter's id.
    pub adapter_id: String,
    /// The provider locator, once the bytes were placed.
    pub locator: Option<ProviderLocator>,
    /// The confirmation kind observed, once `synced`.
    pub confirmation_kind: Option<String>,
    /// The upload state.
    pub state: ProviderState,
    /// The adapter's own reason token for a non-`synced` state.
    pub reason: Option<String>,
}

/// The Context Catalog's entry for one published artifact
/// (HAP-001 § Catalog record): small portable state, secret-free and
/// path-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRecord {
    /// The publication identity.
    pub publication_id: PublicationIdentity,
    /// The artifact Catalog identity.
    pub catalog_id: CatalogId,
    /// The classification outcome.
    pub class: ArtifactClass,
    /// The detected type.
    pub detected_type: DetectedType,
    /// The size in bytes.
    pub size: u64,
    /// The content digest, computed once from the handle (HAP-001-R16).
    pub digest: ContentDigest,
    /// The display names: the request's, plus aliases from later identical
    /// publications; every one sanitized.
    pub names: Vec<DisplayName>,
    /// Logical identities of related notes, tasks, and runs; never a path.
    pub relationships: Vec<String>,
    /// Scope, producer, and transitions.
    pub provenance: Provenance,
    /// The provider adapter, locator, and upload state.
    pub provider: ProviderRecord,
    /// The latest state.
    pub state: ArtifactState,
    /// The record schema integer.
    pub record_version: u32,
}

impl CatalogRecord {
    /// The asset root the artifact was published to.
    #[must_use]
    pub const fn asset_root_id(&self) -> &AssetRootId {
        &self.catalog_id.asset_root_id
    }

    /// The portable reference this record issues (HAP-001-R24: only a
    /// `registered` record has one).
    #[must_use]
    pub fn reference(&self) -> Option<PortableReference> {
        matches!(
            self.state,
            ArtifactState::Registered | ArtifactState::ProviderSynced
        )
        .then(|| PortableReference::new(self.catalog_id.clone()))
    }

    /// Record `state` as the latest transition at `at`.
    pub fn transition(&mut self, state: ArtifactState, at: SystemTime) {
        self.provenance.transitions.push(Transition { state, at });
        self.state = state;
    }
}

/// An artifact approval's identifier: derived from
/// [`artifact_approval_id_preimage`] (its SHA-256's first 8 bytes), never
/// counted and never from a standing policy (HAP-001-R22, D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArtifactApprovalId(pub u64);

impl ArtifactApprovalId {
    /// The 16-character lowercase hex form the wire carries (a `u64` does
    /// not survive a JSON number's 53-bit mantissa).
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("{:016x}", self.0)
    }

    /// Parse the hex form, or `None` if `text` is not 16 hex characters.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 16 {
            return None;
        }
        u64::from_str_radix(text, 16).ok().map(Self)
    }
}

/// The exact bytes an artifact approval id is the SHA-256 of:
/// [`ARTIFACT_APPROVAL_ID_DOMAIN`] ‖ the publication identity's 32 bytes
/// ‖ the approval instant as nanoseconds since the Unix epoch, big-endian
/// `u128`.
#[must_use]
pub fn artifact_approval_id_preimage(
    publication: &PublicationIdentity,
    approved_at: SystemTime,
) -> Vec<u8> {
    let nanos: u128 = approved_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let mut preimage = ARTIFACT_APPROVAL_ID_DOMAIN.to_vec();
    preimage.extend_from_slice(&publication.0.0);
    preimage.extend_from_slice(&nanos.to_be_bytes());
    preimage
}

/// One explicit, per-artifact approval (HAP-001-R22, D3): the candidate
/// named by run id, outbox-relative name, and digest, bound to the
/// identity facts the approval surface showed, the destination, and the
/// act-as identity. Recorded in the publication journal as its `approved`
/// step; never derived from a standing policy in this slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactApproval {
    /// This approval's derived id.
    pub approval_id: ArtifactApprovalId,
    /// The publication identity the approval is for.
    pub publication_id: PublicationIdentity,
    /// The project the candidate belongs to.
    pub project: ProjectIdentity,
    /// The run whose inventory the candidate was listed in.
    pub run_id: RunId,
    /// The candidate's name relative to the outbox (`<run id>/<file>`).
    pub name: String,
    /// The sanitized display name.
    pub display_name: DisplayName,
    /// The digest computed from the candidate's handle.
    pub digest: ContentDigest,
    /// The size digested.
    pub size: u64,
    /// The detected type.
    pub detected_type: DetectedType,
    /// The class the policy assigned.
    pub class: ArtifactClass,
    /// Whether the run's own proposal named the entry.
    pub attribution: Attribution,
    /// The destination asset root.
    pub asset_root_id: AssetRootId,
    /// The adapter that ran the producing launch, when known.
    pub adapter_id: Option<AdapterId>,
    /// The executable approval that governed the launch, when known.
    pub executable_approval: Option<ApprovalId>,
    /// The act-as identity: the device-local user, opaquely.
    pub approver: DeviceLocalUser,
    /// When the approval was recorded.
    pub approved_at: SystemTime,
}

/// A publication step the journal records (HAP-001-R29: "publication
/// identity, step, outcome, digest, reason, UTC time").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JournalStep {
    /// The published copy was placed and verified (state
    /// `published-local`); the entry carries the record to register.
    PublishedLocal,
    /// The Catalog record was written (state `registered`).
    Registered,
    /// Registration failed after `published-local`.
    RegistrationPending,
    /// The outbox entry's removal after registration.
    Cleanup,
    /// The transaction ended `refused`.
    Refused,
    /// The transaction ended `outbox-escape`; the reason names the
    /// recovery entry by digest when one was written.
    OutboxEscape,
    /// The transaction ended `outbox-linked`.
    OutboxLinked,
    /// The transaction ended `integrity-mismatch`.
    IntegrityMismatch,
    /// The request repeated an existing publication identity.
    DuplicatePublication,
}

impl JournalStep {
    /// Every step, for exhaustive iteration and parsing.
    pub const ALL: [Self; 9] = [
        Self::PublishedLocal,
        Self::Registered,
        Self::RegistrationPending,
        Self::Cleanup,
        Self::Refused,
        Self::OutboxEscape,
        Self::OutboxLinked,
        Self::IntegrityMismatch,
        Self::DuplicatePublication,
    ];

    /// This step's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PublishedLocal => "published-local",
            Self::Registered => "registered",
            Self::RegistrationPending => "registration-pending",
            Self::Cleanup => "cleanup",
            Self::Refused => "refused",
            Self::OutboxEscape => "outbox-escape",
            Self::OutboxLinked => "outbox-linked",
            Self::IntegrityMismatch => "integrity-mismatch",
            Self::DuplicatePublication => "duplicate-publication",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.as_str() == token)
    }
}

/// A step's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepOutcome {
    /// The step completed.
    Ok,
    /// The step failed; the reason says how.
    Failed,
    /// The step was not performed and is left for later; the reason says
    /// why (a cleanup after a restart, an identity check the platform
    /// cannot perform).
    Deferred,
}

impl StepOutcome {
    /// Every outcome.
    pub const ALL: [Self; 3] = [Self::Ok, Self::Failed, Self::Deferred];

    /// This outcome's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Deferred => "deferred",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == token)
    }
}

/// One publication step's journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEntry {
    /// The publication the step belongs to.
    pub publication_id: PublicationIdentity,
    /// The approval that authorized the transaction.
    pub approval_id: ArtifactApprovalId,
    /// The step.
    pub step: JournalStep,
    /// Its outcome.
    pub outcome: StepOutcome,
    /// The content digest the step worked with.
    pub digest: ContentDigest,
    /// A fixed reason token for a failed or deferred outcome; never a path.
    pub reason: Option<String>,
    /// When the step completed.
    pub at: SystemTime,
    /// The record to register, carried by `published-local` so a restart
    /// can resume registration without the handle (HAP-001-R29).
    pub record: Option<CatalogRecord>,
    /// The recovery entry written by an `outbox-escape` step, named by its
    /// digest under the work area's `recovery/` directory (HAP-001-R18).
    pub recovery: Option<ContentDigest>,
}

/// One line of the publication journal: an approval, or a step. Both
/// payloads are boxed: an approval carries every identity-bound fact and a
/// `published-local` step carries the whole record to register, so the
/// enum itself stays two words wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEntry {
    /// The approval that authorizes a publication (step 6).
    Approved(Box<ArtifactApproval>),
    /// A later step.
    Step(Box<StepEntry>),
}

impl JournalEntry {
    /// The publication this entry belongs to.
    #[must_use]
    pub const fn publication_id(&self) -> &PublicationIdentity {
        match self {
            Self::Approved(approval) => &approval.publication_id,
            Self::Step(step) => &step.publication_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactApprovalId, DisplayName, is_bidi_control};

    #[test]
    fn approval_id_hex_round_trips() {
        let id = ArtifactApprovalId(0x0123_4567_89ab_cdef);
        assert_eq!(id.to_hex(), "0123456789abcdef");
        assert_eq!(ArtifactApprovalId::from_hex("0123456789abcdef"), Some(id));
        assert_eq!(ArtifactApprovalId::from_hex("0123"), None);
        assert_eq!(ArtifactApprovalId::from_hex("zz23456789abcdef"), None);
    }

    #[test]
    fn bidi_controls_cover_overrides_and_isolates_only() {
        assert!(is_bidi_control('\u{202E}'));
        assert!(is_bidi_control('\u{2066}'));
        assert!(!is_bidi_control('\u{200F}'));
        assert!(!is_bidi_control('a'));
    }

    #[test]
    fn a_plain_name_is_unchanged() {
        assert_eq!(DisplayName::sanitize("report.pdf").as_str(), "report.pdf");
    }
}
