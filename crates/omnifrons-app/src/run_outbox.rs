//! The outbox ports and services (spike slice 5, HAP-001 launch side):
//! validating a declared outbox against a project root, preparing a run
//! subdirectory before spawn ([`RunOutboxPreparer`]), opening and
//! digesting a candidate entry under the single-handle discipline
//! ([`CandidateProber`]), listing a directory without following links
//! ([`OutboxInventory`]), and the pure assembly of probed entries into
//! attributed, classified candidates with the D22 handle cap.
//!
//! `std::fs::File` is a `std` type, not a domain one -- so the held
//! handles ([`DirectoryHandle`], [`RegularCandidate::handle`]) live here,
//! in `omnifrons-app`, exactly as `ExecHandle` does
//! (`crate::executable_prober`), never in `omnifrons-domain`.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};

pub use omnifrons_domain::outbox::{
    ArtifactClass, Attribution, CandidateEntry, CandidateState, ContentDigest, DetectedType,
    OutboxFailure, OutboxPath, ProposedEntry, PublishProposal, RunId,
};

use crate::harness_adapter::WorkspaceRoot;
use crate::outbox_policy::ArtifactClassifier;

/// How many candidate handles may be held open awaiting approval per
/// project (HAP-001 D22, the spike default): an entry beyond the cap stays
/// `candidate` without a held handle and is re-opened under HAP-001-R15
/// when its turn comes (HAP-001-R17).
pub const MAX_HELD_HANDLES: usize = 32;

/// Where a validated outbox declaration points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxLocation {
    /// A real directory inside the project, at this canonical path.
    Present(PathBuf),
    /// Nothing exists at the declared path yet; the preparer creates it
    /// here (the project root joined with the declaration, uncanonicalized
    /// because nothing is there to canonicalize).
    Missing(PathBuf),
}

/// Why a declared outbox path failed validation against a project root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxDeclarationError {
    /// The path canonicalizes outside the project root (HAP-001-R8).
    OutsideProject,
    /// The path itself is a symbolic link (HAP-001-R8).
    IsLink,
    /// Something that is neither a directory nor a link sits at the path
    /// (HAP-001-R10: not a real directory).
    NotADirectory,
    /// The path's metadata could not be read.
    Unreadable,
}

impl OutboxDeclarationError {
    /// The failure token this error renders: `outbox-invalid` for a
    /// declaration that fails HAP-001-R8's two conditions, and
    /// `outbox-unavailable` for an outbox that fails its pre-creation
    /// check (HAP-001-R10).
    #[must_use]
    pub const fn failure(&self) -> OutboxFailure {
        match self {
            Self::OutsideProject | Self::IsLink => OutboxFailure::OutboxInvalid,
            Self::NotADirectory | Self::Unreadable => OutboxFailure::OutboxUnavailable,
        }
    }
}

impl std::fmt::Display for OutboxDeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::OutsideProject => "the declared outbox resolves outside the project root",
            Self::IsLink => "the declared outbox is a link",
            Self::NotADirectory => "the declared outbox is not a directory",
            Self::Unreadable => "the declared outbox could not be read",
        };
        f.write_str(message)
    }
}

impl std::error::Error for OutboxDeclarationError {}

/// Validate `declared` against `project` (HAP-001-R8, R10): the path
/// itself must not be a link, must canonicalize inside the project root,
/// and must be a real directory -- or not exist yet, which the preparer
/// resolves by creating it.
///
/// # Errors
///
/// Returns [`OutboxDeclarationError`] for a link, a path resolving outside
/// the project, a non-directory at the path, or unreadable metadata.
pub fn validate_outbox_declaration(
    project: &WorkspaceRoot,
    declared: &OutboxPath,
) -> Result<OutboxLocation, OutboxDeclarationError> {
    let joined = project.path().join(declared.as_path());
    let metadata = match std::fs::symlink_metadata(&joined) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OutboxLocation::Missing(joined));
        }
        Err(_) => return Err(OutboxDeclarationError::Unreadable),
    };
    if metadata.file_type().is_symlink() {
        return Err(OutboxDeclarationError::IsLink);
    }
    if !metadata.is_dir() {
        return Err(OutboxDeclarationError::NotADirectory);
    }
    let canonical =
        std::fs::canonicalize(&joined).map_err(|_| OutboxDeclarationError::Unreadable)?;
    if !canonical.starts_with(project.path()) {
        return Err(OutboxDeclarationError::OutsideProject);
    }
    Ok(OutboxLocation::Present(canonical))
}

/// An open directory handle: the outbox, or a run subdirectory. What a
/// candidate entry is opened *relative to* on unix (`openat`), and what
/// "verify by handle" compares against.
#[derive(Debug)]
pub struct DirectoryHandle(File);

impl DirectoryHandle {
    /// Wrap an already-open directory.
    #[must_use]
    pub const fn new(file: File) -> Self {
        Self(file)
    }

    /// The underlying open file.
    #[must_use]
    pub const fn as_file(&self) -> &File {
        &self.0
    }

    /// `fstat` the handle.
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the underlying call can produce.
    pub fn metadata(&self) -> std::io::Result<std::fs::Metadata> {
        self.0.metadata()
    }

    /// Duplicate the handle: a second descriptor onto the same open
    /// directory, so an inventory can run outside a lock that guards the
    /// original.
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the underlying duplication can produce.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

/// A validated, open outbox: its canonical path and the handle every run
/// subdirectory is created relative to.
#[derive(Debug)]
pub struct OpenedOutbox {
    /// The canonical path of the outbox directory.
    pub path: PathBuf,
    /// The open directory.
    pub handle: DirectoryHandle,
}

/// A run subdirectory created before spawn (HAP-001-R10): the canonical
/// path declared to the harness, and the handle verified to be the
/// directory that path names.
#[derive(Debug)]
pub struct PreparedRunSubdirectory {
    /// The run this subdirectory belongs to.
    pub run_id: RunId,
    /// The canonical path handed to the harness through
    /// `OMNIFRONS_OUTPUT_DIR` -- the one place a raw path is handed out.
    pub path: PathBuf,
    /// The open directory, verified by handle.
    pub handle: DirectoryHandle,
}

impl PreparedRunSubdirectory {
    /// Duplicate the held directory handle alongside the run id and path
    /// (see [`DirectoryHandle::try_clone`]).
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the underlying duplication can produce.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            run_id: self.run_id.clone(),
            path: self.path.clone(),
            handle: self.handle.try_clone()?,
        })
    }
}

/// Which handle-verification fact failed after creation (HAP-001-R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationFailure {
    /// The created path no longer names a directory.
    NotADirectory,
    /// The handle opened relative to the outbox and the handle opened at
    /// the declared path name different files: something was swapped in.
    IdentityMismatch,
    /// The directory's permissions are not owner-only.
    NotOwnerOnly,
}

/// Why a run subdirectory could not be prepared. Every variant renders
/// `outbox-unavailable` at launch (HAP-001-R10, D14), including an
/// invalid declaration, which blocks every launch under it (HAP-001-R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareError {
    /// The outbox declaration failed validation.
    Declaration(OutboxDeclarationError),
    /// Nothing exists at the declared path and the caller asked not to
    /// create it (an inventory of an outbox no run has created yet).
    OutboxMissing,
    /// The outbox could not be created or opened.
    OutboxUnopenable,
    /// Something -- a file, a directory, a link of any kind -- already
    /// sits at the run subdirectory's path: the create-exclusive
    /// operation refused it.
    AlreadyExists,
    /// The run subdirectory could not be created for another reason.
    CreateFailed,
    /// The created directory could not be opened.
    OpenFailed,
    /// Handle verification failed after creation.
    Verification(VerificationFailure),
}

impl PrepareError {
    /// The failure token this error renders: always
    /// `outbox-unavailable` at launch (HAP-001-R10).
    #[must_use]
    pub const fn failure(&self) -> OutboxFailure {
        OutboxFailure::OutboxUnavailable
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declaration(error) => write!(f, "outbox declaration failed: {error}"),
            Self::OutboxMissing => f.write_str("the outbox does not exist yet"),
            Self::OutboxUnopenable => f.write_str("the outbox could not be created or opened"),
            Self::AlreadyExists => {
                f.write_str("something already sits at the run subdirectory's path")
            }
            Self::CreateFailed => f.write_str("the run subdirectory could not be created"),
            Self::OpenFailed => f.write_str("the created run subdirectory could not be opened"),
            Self::Verification(failure) => {
                write!(f, "run subdirectory verification failed: {failure:?}")
            }
        }
    }
}

impl std::error::Error for PrepareError {}

/// A port for the run-subdirectory creation HAP-001-R10 requires: open a
/// validated outbox by handle, and create a run subdirectory under it
/// exclusively, with owner-only permissions, verified by handle.
pub trait RunOutboxPreparer {
    /// Validate `declared` against `project` and open the outbox
    /// directory without following a link at its path, creating it first
    /// when `create_if_missing` and nothing exists at the declared path.
    ///
    /// # Errors
    ///
    /// Returns [`PrepareError::Declaration`] if the declaration fails
    /// validation, or [`PrepareError::OutboxUnopenable`] if the outbox
    /// could not be created or opened.
    fn open_outbox(
        &self,
        project: &WorkspaceRoot,
        declared: &OutboxPath,
        create_if_missing: bool,
    ) -> Result<OpenedOutbox, PrepareError>;

    /// Prepare `run_id`'s subdirectory under the outbox: open the outbox
    /// ([`Self::open_outbox`], creating it when missing), create the
    /// subdirectory relative to that handle with a create-exclusive
    /// operation and owner-only permissions, open it without following a
    /// link, and verify by handle that the path to be declared names the
    /// directory just created.
    ///
    /// # Errors
    ///
    /// Returns [`PrepareError`] naming the step that failed; the caller
    /// renders `outbox-unavailable` and launches nothing.
    fn prepare(
        &self,
        project: &WorkspaceRoot,
        declared: &OutboxPath,
        run_id: &RunId,
    ) -> Result<PreparedRunSubdirectory, PrepareError>;
}

/// Why a probed entry is `outbox-escape` (HAP-001-R14, R15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeReason {
    /// A link sat at the entry's name; it was never dereferenced.
    Link,
    /// The opened handle is not a regular file: a directory, a FIFO, a
    /// socket, or a device.
    NotRegular,
}

/// A regular file opened once, with every fact taken from that handle.
#[derive(Debug)]
pub struct RegularCandidate {
    /// The bytes digested.
    pub size: u64,
    /// The content digest computed from the handle (HAP-001-R16).
    pub digest: ContentDigest,
    /// The type detected from the same bytes.
    pub detected_type: DetectedType,
    /// The handle, held for the approval that follows (HAP-001-R17).
    pub handle: File,
}

/// The outcome of probing one directory entry under the single-handle
/// discipline.
#[derive(Debug)]
pub enum CandidateProbe {
    /// A regular file with a link count of one.
    Regular(RegularCandidate),
    /// Refused: `outbox-escape`.
    Escape(EscapeReason),
    /// Refused: `outbox-linked` (HAP-001-R20), with the link count read
    /// from the handle.
    Linked {
        /// The handle's link count, greater than one.
        link_count: u64,
    },
    /// The entry could not be opened or read: excluded from the
    /// candidates and counted.
    Unreadable,
}

/// A port for opening one directory entry exactly once, without following
/// links, and taking every identity fact and the digest from that handle
/// (HAP-001-R15, R16, R20).
pub trait CandidateProber {
    /// Probe the entry named `name` inside `dir`, whose path is `dir_path`
    /// (used only where the platform offers no relative open).
    fn probe(&self, dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> CandidateProbe;
}

/// One directory entry an inventory found, with its probe.
#[derive(Debug)]
pub struct InventoriedEntry {
    /// The entry's own name within the directory.
    pub name: OsString,
    /// What opening it yielded.
    pub probe: CandidateProbe,
}

/// Why a directory could not be inventoried or descended into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryError {
    /// The directory could not be listed.
    Unreadable,
    /// The named entry is not a real directory (a link, or not a
    /// directory at all).
    NotADirectory,
}

/// A port for listing a directory without following links and probing
/// every entry through a [`CandidateProber`] (HAP-001-R12: an inventory
/// never dereferences a link found in the outbox).
pub trait OutboxInventory {
    /// Every entry of `dir`, probed.
    ///
    /// # Errors
    ///
    /// Returns [`InventoryError::Unreadable`] if the directory could not
    /// be listed.
    fn inventory(
        &self,
        dir: &DirectoryHandle,
        dir_path: &Path,
    ) -> Result<Vec<InventoriedEntry>, InventoryError>;

    /// Open the subdirectory named `name` of `dir` without following a
    /// link at its name.
    ///
    /// # Errors
    ///
    /// Returns [`InventoryError::NotADirectory`] if the name is a link or
    /// not a directory, or [`InventoryError::Unreadable`] if it could not
    /// be opened.
    fn open_subdirectory(
        &self,
        dir: &DirectoryHandle,
        dir_path: &Path,
        name: &OsStr,
    ) -> Result<DirectoryHandle, InventoryError>;
}

/// One assembled candidate: the entry, its state, and the handle held for
/// it when within the cap.
#[derive(Debug)]
pub struct Candidate {
    /// The entry's facts, attribution, and class.
    pub entry: CandidateEntry,
    /// The state the entry rendered in.
    pub state: CandidateState,
    /// The held handle (HAP-001-R17), `None` for a refused entry or one
    /// beyond the D22 cap.
    pub handle: Option<File>,
}

/// The result of assembling an inventory: the candidates in inventory
/// order, how many entries were unreadable, and the proposals that named
/// a digest no entry carried.
#[derive(Debug)]
pub struct AssembledCandidates {
    /// The candidates, in inventory order.
    pub candidates: Vec<Candidate>,
    /// Entries excluded because they could not be opened or read.
    pub unreadable: u32,
    /// Proposed entries whose digest matched nothing in the directory.
    pub unmatched_proposals: Vec<ProposedEntry>,
}

/// Counts per state and attribution, for the run-end summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CandidateSummary {
    /// Every listed candidate.
    pub total: u32,
    /// Listed as `candidate`.
    pub candidate: u32,
    /// Listed as `outbox-escape`.
    pub outbox_escape: u32,
    /// Listed as `outbox-linked`.
    pub outbox_linked: u32,
    /// Attributed to the run by its own proposal.
    pub attributed: u32,
    /// Unattributed.
    pub unattributed: u32,
    /// Excluded as unreadable.
    pub unreadable: u32,
    /// Proposals that matched nothing.
    pub unmatched_proposals: u32,
}

/// Assemble a run subdirectory's inventory: every entry named
/// `<run id>/<name>`, attributed to `run_id` only when one of the run's
/// own `proposals` names its digest (HAP-001-R11), classified by
/// `classifier`.
#[must_use]
pub fn assemble_run_candidates(
    run_id: &RunId,
    entries: Vec<InventoriedEntry>,
    proposals: &[PublishProposal],
    classifier: &dyn ArtifactClassifier,
) -> AssembledCandidates {
    let mut assembled = assemble(Some(run_id), entries, classifier);
    let mut seen_digests: Vec<ContentDigest> = Vec::new();
    for candidate in &mut assembled.candidates {
        if candidate.state != CandidateState::Candidate {
            continue;
        }
        seen_digests.push(candidate.entry.digest);
        if proposals
            .iter()
            .any(|proposal| proposal.names_digest(&candidate.entry.digest))
        {
            candidate.entry.attribution = Attribution::Run(run_id.clone());
        }
    }
    assembled.unmatched_proposals = proposals
        .iter()
        .flat_map(|proposal| proposal.entries.iter())
        .filter(|entry| !seen_digests.contains(&entry.sha256))
        .cloned()
        .collect();
    assembled
}

/// Assemble an inventory as unattributed entries (HAP-001-R12: the
/// session-start inventory of the whole outbox proposes, never
/// attributes): named `<name>` at the outbox root, or `<run id>/<name>`
/// under a run subdirectory -- a location fact carried in the name, never
/// provenance (HAP-001-R36).
#[must_use]
pub fn assemble_unattributed_candidates(
    under_run: Option<&RunId>,
    entries: Vec<InventoriedEntry>,
    classifier: &dyn ArtifactClassifier,
) -> AssembledCandidates {
    assemble(under_run, entries, classifier)
}

fn assemble(
    under_run: Option<&RunId>,
    entries: Vec<InventoriedEntry>,
    classifier: &dyn ArtifactClassifier,
) -> AssembledCandidates {
    let mut candidates = Vec::with_capacity(entries.len());
    let mut unreadable = 0u32;
    for entry in entries {
        let file_name = entry.name.to_string_lossy().into_owned();
        let name = match under_run {
            Some(run_id) => format!("{run_id}/{file_name}"),
            None => file_name.clone(),
        };
        let candidate = match entry.probe {
            CandidateProbe::Regular(regular) => Candidate {
                entry: CandidateEntry {
                    name,
                    size: regular.size,
                    digest: regular.digest,
                    detected_type: regular.detected_type,
                    attribution: Attribution::Unattributed,
                    class: classifier.classify(&file_name, regular.detected_type, regular.size),
                },
                state: CandidateState::Candidate,
                handle: Some(regular.handle),
            },
            CandidateProbe::Escape(_) => refused(name, CandidateState::OutboxEscape),
            CandidateProbe::Linked { .. } => refused(name, CandidateState::OutboxLinked),
            CandidateProbe::Unreadable => {
                unreadable = unreadable.saturating_add(1);
                continue;
            }
        };
        candidates.push(candidate);
    }
    AssembledCandidates {
        candidates,
        unreadable,
        unmatched_proposals: Vec::new(),
    }
}

/// A refused entry: nothing digested, nothing classified, no handle.
fn refused(name: String, state: CandidateState) -> Candidate {
    Candidate {
        entry: CandidateEntry {
            name,
            size: 0,
            digest: omnifrons_domain::executable::Sha256Digest([0; 32]),
            detected_type: DetectedType::Unknown,
            attribution: Attribution::Unattributed,
            class: ArtifactClass::Unclassified,
        },
        state,
        handle: None,
    }
}

/// Release every held handle beyond the first `cap` held ones, in order
/// (HAP-001-R17, D22); the entries stay `candidate`. Returns how many
/// handles were released.
pub fn cap_held_handles(candidates: &mut [Candidate], cap: usize) -> usize {
    let mut held = 0usize;
    let mut released = 0usize;
    for candidate in candidates.iter_mut() {
        if candidate.handle.is_none() {
            continue;
        }
        if held < cap {
            held += 1;
        } else {
            candidate.handle = None;
            released += 1;
        }
    }
    released
}

/// Count `assembled` per state and attribution.
#[must_use]
pub fn summarize(assembled: &AssembledCandidates) -> CandidateSummary {
    let mut summary = CandidateSummary {
        unreadable: assembled.unreadable,
        unmatched_proposals: u32::try_from(assembled.unmatched_proposals.len()).unwrap_or(u32::MAX),
        ..CandidateSummary::default()
    };
    for candidate in &assembled.candidates {
        summary.total += 1;
        match candidate.state {
            CandidateState::Candidate => summary.candidate += 1,
            CandidateState::OutboxEscape => summary.outbox_escape += 1,
            CandidateState::OutboxLinked => summary.outbox_linked += 1,
        }
        match candidate.entry.attribution {
            Attribution::Run(_) => summary.attributed += 1,
            Attribution::Unattributed => summary.unattributed += 1,
        }
    }
    summary
}
