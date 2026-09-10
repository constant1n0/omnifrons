//! The wrong-root scan (spike slice 5d, HAP-001-R32, R43, R44, D16 at the
//! post-run-scan half of its default): the [`WrongRootScanner`] port that
//! walks the active workspace root without following links and reports
//! every `misplaced` file it finds, and the pure rules -- the exclusions
//! and the per-file verdict -- that a fake and the real adapter share, so
//! the two cannot drift.
//!
//! **No handle survives the scan.** Each file is opened, read, and closed
//! within the walk; nothing survives it but the findings, which carry
//! facts and a project-relative name. *During* the walk an implementation
//! holds the directory handles of the path it is on and no more --
//! [`MAX_SCAN_DEPTH`] of them at worst, never one per directory still to
//! visit. The 32-handle cap (HAP-001 D22) is shared with the approval
//! surface, and a scan whose descriptor use grew with the tree could starve
//! it.
//!
//! **Two exclusions, both the product's own rule and never an ignore rule**
//! (HAP-001-R43, R44): the declared outbox subtree, which this same walk
//! reads as an ingestion source rather than as a wrong root, and a
//! repository's own metadata directory -- decided by what that directory
//! holds ([`is_git_metadata_dir`]) and not by the name `.git`, since a
//! producer writing into the worktree could otherwise exclude anything it
//! liked with one `mkdir`. That raises the cost of the blind spot without
//! closing it: a producer that fabricates a `HEAD` and an `objects/`
//! directory has its exclusion back, which is a heuristic and not the index
//! (see [`is_git_metadata_dir`]). The product work area is excluded by
//! construction: HAP-001-R7 already guarantees it never resolves inside a
//! registered workspace root, and the scan re-checks that before it walks.
//!
//! HAP-001-R32's other arms -- another registered project's root, and the
//! vault -- name things this repository does not have; they are left
//! unimplemented rather than faked (`docs/spike-log.md` § Slice 5d).

use std::io::Seek as _;
use std::io::SeekFrom;
use std::path::Path;

use omnifrons_domain::outbox::{ContentDigest, DetectedType, OutboxPath};
use omnifrons_domain::publication::DisplayName;
use omnifrons_domain::wrong_root::{MisplacedFinding, WrongRootReason, is_misplaced_class};

use crate::content_hasher::ContentHasher;
use crate::harness_adapter::WorkspaceRoot;
use crate::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use crate::outbox_policy::ArtifactClassifier;
use crate::run_outbox::{CandidateProbe, CandidateProber, OpenedOutbox};

/// The directory name Git keeps a repository's own metadata under, which
/// is never product output.
pub const GIT_DIR: &str = ".git";

/// The entry a repository's metadata directory always holds naming its
/// current branch.
pub const GIT_HEAD: &str = "HEAD";

/// The directory a repository's metadata directory always holds its object
/// database in.
pub const GIT_OBJECTS: &str = "objects";

/// Whether a directory named [`GIT_DIR`] is a real repository's metadata
/// directory -- and so excluded from the wrong-root verdict -- given
/// whether it holds [`GIT_HEAD`] and a [`GIT_OBJECTS`] directory.
///
/// **Not "any directory named `.git`", at any depth** (spike slice 5d,
/// R1-006). The walk polices a worktree that a producer writes into, and a
/// name-only exclusion would let that producer carve its own blind spot out
/// of the scan with a single `mkdir .git` next to whatever it wanted to
/// hide. HAP-001 asks for Git's metadata to be left alone, not for a magic
/// directory name to switch the verdict off.
///
/// **This raises the cost of that blind spot; it does not close it.** The
/// test is what the directory holds, and a producer that writes a `HEAD`
/// file and an `objects/` directory beside whatever it wants hidden has its
/// exclusion back -- two commands where a name-only rule needed one. It is
/// a heuristic and not the index: a repository whose metadata layout
/// differs from that is scanned, and a fabricated one is not. Closing it
/// needs the git index, which needs a dependency this slice does not add
/// (`docs/spike-log.md` § Slice 5d, D1).
///
/// A submodule's `.git` is a *file* holding a `gitdir:` pointer rather than
/// a directory, so it is never descended and never classified misplaced;
/// an older submodule that carries a real metadata directory has `HEAD` and
/// `objects/` in it like any repository and is excluded by this same rule.
#[must_use]
pub const fn is_git_metadata_dir(holds_head: bool, holds_objects: bool) -> bool {
    holds_head && holds_objects
}

/// How many directory entries one scan examines before it stops and says
/// so ([`ScanReport::truncated`]). A worktree can hold a dependency tree
/// with hundreds of thousands of files, and a run-end scan must stay
/// bounded in time as much as in memory; a truncated report is disclosed,
/// never silent.
pub const MAX_SCANNED_ENTRIES: u32 = 20_000;

/// How deep the walk descends below the project root before it stops
/// descending. A bound, not a judgment: a deeply nested tree is reported
/// as truncated like an oversized one.
pub const MAX_SCAN_DEPTH: usize = 32;

/// How many entries of a **single** directory one listing collects before
/// it stops and the report says [`ScanReport::truncated`].
///
/// [`MAX_SCANNED_ENTRIES`] is not this bound: it caps the entries a walk
/// *examines*, and a walk that lists a directory in full materializes every
/// name in it before that cap can fire once. One directory holding a few
/// million entries -- as easy for the same-user producer this scan exists
/// to catch to create as any other -- would then be collected and sorted
/// whole, and a run-end scan that runs automatically must stay bounded in
/// memory as much as in time (spike slice 5d, R1-015).
///
/// Set at [`MAX_SCANNED_ENTRIES`] on purpose, so the bound costs no
/// fidelity: every name in a listing is an entry the walk would examine, so
/// a directory wide enough to reach this cap already drives the entry cap
/// past its own limit, and a listing cut short here can never hide a file
/// that a complete listing would have reported without `truncated` being
/// set anyway.
///
/// **Which** names a cut-short listing holds is the platform's own
/// directory order and not a chosen subset: past this bound the report is
/// partial and says so, and nothing about *which* part is claimed.
pub const MAX_DIRECTORY_NAMES: usize = MAX_SCANNED_ENTRIES as usize;

/// Whether the project-relative path `relative` is excluded from the
/// wrong-root verdict by the product's own rule (HAP-001-R43, R44): it is,
/// or lies under, the declared `outbox`.
///
/// Never an ignore rule: HAP-001-R32 forbids treating a `.gitignore` match
/// as changing the verdict, and HAP-001-R44 requires the outbox to be kept
/// out by the product's rule rather than by relying on one.
///
/// Git's metadata is the second exclusion, but it is **not** decided here:
/// it needs the facts a name alone cannot carry, and it is
/// [`is_git_metadata_dir`]'s question.
#[must_use]
pub fn is_excluded(relative: &Path, outbox: &OutboxPath) -> bool {
    relative.starts_with(outbox.as_path())
}

/// The facts one scanned file yielded, before the verdict is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// The project-relative name, `/`-separated.
    pub name: String,
    /// The bytes read from the file's handle.
    pub size: u64,
    /// The digest computed from that handle.
    pub digest: ContentDigest,
    /// The type detected from the same bytes.
    pub detected_type: DetectedType,
}

/// What one examined file's verdict is.
///
/// Three outcomes and not two, because "not misplaced" and "misplaced but
/// unreportable" are different facts and folding them together makes a
/// scan that dropped a file read as a clean one (spike slice 5d, R3-003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Misplaced, and this is the finding to offer.
    Misplaced(MisplacedFinding),
    /// Examined, and HAP-001 keeps it where it is.
    KeptWhereItIs,
    /// Its class makes it misplaced, but the name it was found at is not
    /// one a finding may carry across IPC (RCS-001-R14) -- a control
    /// character in a file name is legal on unix, and a name carrying one
    /// would forge or break the managed blocks this product writes.
    ///
    /// Counted and surfaced by the caller; **never** silently dropped,
    /// which would let one `mkdir`-and-`touch` erase a file from the report
    /// while the report still called itself clean.
    UnreportableName,
}

/// The verdict for `scanned` under `classifier`.
///
/// [`Verdict::KeptWhereItIs`] for a class HAP-001 keeps where it is --
/// `git-tracked` (HAP-001-R1, R3: a Git-managed file is never published and
/// never flagged in its tracked location) and `portable-text`
/// (HAP-001-R2: a Markdown note is refused as an artifact) -- and for the
/// two classes this slice offers no remedy for (`executable`,
/// `unclassified`; see `omnifrons_domain::wrong_root::is_misplaced_class`).
#[must_use]
pub fn finding_for(scanned: &ScannedFile, classifier: &dyn ArtifactClassifier) -> Verdict {
    let class = classifier.classify(&scanned.name, scanned.detected_type, scanned.size);
    if !is_misplaced_class(class) {
        return Verdict::KeptWhereItIs;
    }
    MisplacedFinding::new(
        &scanned.name,
        scanned.size,
        scanned.digest,
        scanned.detected_type,
        class,
        WrongRootReason::InProjectOutsideOutbox,
    )
    .map_or(Verdict::UnreportableName, Verdict::Misplaced)
}

/// What one scan is asked to walk: the active workspace root, the outbox
/// its policy declares, and the classifier that policy is.
pub struct ScanRequest<'a> {
    /// The project root walked, without following links.
    pub project: &'a WorkspaceRoot,
    /// The declared outbox, excluded from the verdict (HAP-001-R44).
    pub outbox: &'a OutboxPath,
    /// The project's classification policy (HAP-001-R1).
    pub classifier: &'a dyn ArtifactClassifier,
}

/// What one scan found. Counts and findings only: no handle survives the
/// scan and no device path is carried.
///
/// The three counters are **disjoint**: no entry is counted in more than
/// one of them, so a loss cannot hide by being counted twice.
///
/// They do **not** partition the walk, and reading them as a total of
/// everything it touched is wrong (spike slice 5d, R1-018). Three entries
/// are accounted for elsewhere on purpose: a directory the walk descends
/// into is represented by the entries it yields rather than by a counter of
/// its own; the declared outbox increments [`Self::excluded`] without ever
/// being examined, so it costs nothing against [`MAX_SCANNED_ENTRIES`] --
/// safely, because [`is_excluded`] is a prefix test against a path the walk
/// then never descends into, so it can fire at most once in a scan, unlike
/// the metadata-directory exclusion, which is decided *after* the entry is
/// examined and counted; and an entry the walk had no file descriptor
/// left to open is accounted for by [`Self::truncated`], because running
/// out of descriptors is a fact about the process and not about the entry.
/// What the counters do answer for is every entry the walk **examined and
/// reached an outcome for**, each in exactly one place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// The misplaced files, in walk order.
    pub findings: Vec<MisplacedFinding>,
    /// How many files were examined and reached a verdict.
    pub scanned: u32,
    /// How many entries a rule skipped: the declared outbox subtree, a
    /// repository's own metadata directory, and every link -- which is
    /// counted and never dereferenced.
    pub excluded: u32,
    /// How many entries could not be opened, read, **or named**.
    ///
    /// The third is not a rounding error: a file name may legally carry a
    /// control character on unix and a finding may not (RCS-001-R14), so a
    /// heavy file found at such a name is counted here rather than dropped
    /// into a report that would then read as a clean scan.
    pub unreadable: u32,
    /// Whether the walk stopped short: at [`MAX_SCANNED_ENTRIES`], at
    /// [`MAX_SCAN_DEPTH`], or because there was no file descriptor left to
    /// keep walking with. The report is partial and says so, rather than
    /// folding what it could not see into [`Self::unreadable`].
    pub truncated: bool,
}

/// Why a scan could not run at all. Closed and exhaustive, never a raw
/// `io::Error` whose text can carry a device path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScanError {
    /// The project root could not be opened or listed.
    #[error("the project root could not be scanned")]
    Unreadable,
}

/// Why a finding's file could not be re-opened for a remedy. Closed and
/// exhaustive, never a raw `io::Error` whose text can carry a device path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OpenMisplacedError {
    /// The name is not a valid project-relative path, so nothing is
    /// resolved for it at all (RCS-001-R14).
    #[error("the name is not a project-relative path")]
    InvalidName,
    /// Nothing sits at that name any more, or a directory component of it
    /// is a link that was not traversed.
    #[error("nothing sits at that name in the project")]
    NotFound,
    /// The name holds a directory, a link, or another non-regular file --
    /// never dereferenced, never acted on.
    #[error("that name does not hold a regular file")]
    NotRegularFile,
    /// The file could not be read.
    #[error("that file could not be read")]
    Unreadable,
    /// The name holds a file with more than one link (HAP-001-R20, applied
    /// with the misplaced file as the candidate per HAP-001-R32): the same
    /// bytes answer to another name this product cannot see, so neither
    /// moving the file out of the project nor copying it into the outbox
    /// means what the surface would say it means.
    #[error("that file has more than one name")]
    Linked {
        /// How many links the handle's own metadata reported.
        link_count: u64,
    },
}

/// A port over the wrong-root scan.
pub trait WrongRootScanner {
    /// Walk `request`'s project root and report every misplaced file.
    ///
    /// An implementation MUST NOT follow a link, MUST NOT hold any handle
    /// past its own return, MUST bound the handles it holds *during* the
    /// walk by the depth it is at rather than by the size of the tree, and
    /// MUST apply [`is_excluded`], [`is_git_metadata_dir`] and
    /// [`finding_for`] rather than a rule of its own.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::Unreadable`] if the project root itself could
    /// not be walked; an entry that could not be read is counted in
    /// [`ScanReport::unreadable`], never an error, and anything the walk
    /// could not reach at all sets [`ScanReport::truncated`].
    fn scan(&self, request: &ScanRequest<'_>) -> Result<ScanReport, ScanError>;

    /// Re-open the file a finding named, under the single-handle
    /// discipline every candidate entry is opened under (HAP-001-R15,
    /// applied with the misplaced file as the candidate per HAP-001-R32):
    /// one no-follow open per path component, the file opened once
    /// without following a link at its name, every fact taken from that
    /// one handle.
    ///
    /// "Per path component" is what a platform with a relative directory
    /// open can offer. Where it has none, an implementation re-resolves the
    /// path and only the final component is protected, so a link at an
    /// ancestor is followed and the name can resolve outside the project;
    /// that is a residual to disclose, not a licence to follow one at the
    /// final component (`docs/spike-log.md` § Slice 5d, R1-014).
    ///
    /// The returned [`crate::publication::CandidateSource`] carries the
    /// directory the file sits in, opened, so a remedy's identity check
    /// and unlink run relative to that handle and never re-resolve a
    /// directory component.
    ///
    /// # Errors
    ///
    /// Returns [`OpenMisplacedError`] naming what refused.
    fn open_misplaced(
        &self,
        project: &WorkspaceRoot,
        relative: &str,
    ) -> Result<crate::publication::CandidateSource, OpenMisplacedError>;
}

/// Where the publish remedy's copy-in lands: a new, unattributed entry at
/// the **outbox root**, and the handle it was opened with.
///
/// HAP-001 § Wrong-root detection and remedies, the publish remedy: "the
/// file is copied into the outbox as an unattributed entry and the
/// publication transaction runs from step 1". The remedy widens nothing:
/// it produces an ordinary outbox entry that the existing whole-outbox
/// path (`candidates_list { runId: null }` -> `artifact_approve` ->
/// `artifact_publish`, spike slice 5c) then handles, so publication still
/// takes its own explicit per-artifact approval (HAP-001-R22, D3) and the
/// publication transaction still only ever unlinks an entry the product
/// itself created (HAP-001-R28).
#[derive(Debug)]
pub struct CopiedIn {
    /// The new entry's name at the outbox root: one component, sanitized,
    /// never a path.
    pub name: String,
    /// The digest of the bytes copied, verified from the copy itself.
    pub digest: ContentDigest,
    /// How many bytes were copied.
    pub size: u64,
    /// The new entry, re-opened once under the single-handle discipline.
    pub source: crate::publication::CandidateSource,
}

/// Why the publish remedy's copy-in refused or failed. Closed and
/// exhaustive, never a raw `io::Error` whose text can carry a device path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CopyInError {
    /// The held handle is not a regular file.
    #[error("the file is not a regular file")]
    NotRegularFile,
    /// The name no longer holds the file the handle was opened on
    /// (HAP-001-R18's check, applied to the misplaced file: HAP-001-R32
    /// requires the remedy to refuse a path swap of its own).
    #[error("the path no longer names the file that was opened")]
    PathChanged,
    /// The bytes no longer hash to the digest the remedy was requested
    /// for.
    #[error("the file's content changed since it was found")]
    DigestChanged,
    /// The copy could not be written, renamed, or read back.
    #[error("the file could not be copied into the outbox")]
    CopyFailed,
    /// Something this remedy did not create already holds that name at the
    /// outbox root: other bytes, a link, a directory, or a file that
    /// answers to a second name this product cannot see (HAP-001-R20).
    /// Never followed, never overwritten, never adopted.
    #[error("an outbox entry of that name already exists")]
    EntryExists,
    /// The held handle names a file with more than one link (HAP-001-R20,
    /// applied with the misplaced file as the candidate per HAP-001-R32).
    #[error("the file has more than one name")]
    Linked {
        /// How many links the handle's own metadata reported.
        link_count: u64,
    },
}

/// The link count from a held handle's own metadata, where the platform's
/// stable `std` exposes it -- the same fact `CandidateProber` reads over an
/// outbox entry (HAP-001-R20), read here over the misplaced file because
/// HAP-001-R32 applies R15 through R20 with it as the candidate.
///
/// The `Option` is the platform seam: the by-handle accessor in
/// `std::os::windows::fs::MetadataExt` is unstable and this repository adds
/// no Windows API dependency, so `outbox-linked` cannot be produced there
/// -- a residual disclosed under HAP-001-R19, not a claim. The unix version
/// wraps unconditionally on purpose.
/// Shared with [`crate::quarantine`], which reads the same fact from the
/// handle it opened on the quarantine destination (R1-022), so the two
/// remedies cannot drift on which platform can make the refusal.
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn link_count(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.nlink())
}

#[cfg(not(unix))]
pub(crate) fn link_count(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

/// The ports one copy-in runs over.
pub struct CopyInPorts<'a> {
    /// SHA-256 over the held handle and over the copy.
    pub hasher: &'a dyn ContentHasher,
    /// The identity check at the misplaced file's own name.
    pub entry_ops: &'a dyn OutboxEntryOps,
    /// Opens the new entry once, without following links, taking every
    /// fact from that one handle (HAP-001-R15, R16, R20).
    pub prober: &'a dyn CandidateProber,
}

/// Copy the bytes the misplaced file's held handle names into the outbox
/// root as a new unattributed entry (HAP-001-R32's publish remedy applied
/// under HAP-001-R15 through R20, with the misplaced file as the
/// candidate).
///
/// The original is left exactly where it was found: removing it is a
/// separate explicit user action this product does not offer, because it
/// never deletes a file it did not create (HAP-001-R28).
///
/// # Errors
///
/// Returns [`CopyInError`] naming the step that refused: a handle that is
/// not a regular file, a name that no longer holds it, content that
/// changed since it was found, an entry of that name already carrying
/// other bytes, or a copy that could not be completed or read back.
pub fn copy_in_from_misplaced(
    ports: &CopyInPorts<'_>,
    source: &mut crate::publication::CandidateSource,
    outbox: &OpenedOutbox,
    expected: &ContentDigest,
    display: &DisplayName,
) -> Result<CopiedIn, CopyInError> {
    let metadata = source
        .handle
        .metadata()
        .map_err(|_| CopyInError::NotRegularFile)?;
    if !metadata.file_type().is_file() {
        return Err(CopyInError::NotRegularFile);
    }
    // HAP-001-R20, applied with the misplaced file as the candidate: the
    // same check `CandidateProber` runs over an outbox entry, from the same
    // fact -- the held handle's own `fstat`.
    if let Some(count) = link_count(&metadata)
        && count > 1
    {
        return Err(CopyInError::Linked { link_count: count });
    }
    match ports.entry_ops.identity(
        &source.dir,
        &source.dir_path,
        &source.file_name,
        &source.handle,
    ) {
        EntryIdentity::SameFile | EntryIdentity::Unverifiable => {}
        EntryIdentity::Missing | EntryIdentity::DifferentFile => {
            return Err(CopyInError::PathChanged);
        }
    }

    let name = crate::quarantine::quarantine_name(display, expected);
    let file_name = std::ffi::OsString::from(&name);
    let dir = outbox
        .handle
        .try_clone()
        .map_err(|_| CopyInError::CopyFailed)?;

    // Whether the entry is already there is asked by **opening that name
    // itself**, once, relative to the outbox handle and without following a
    // link at it -- the same single-handle discipline every candidate entry
    // is opened under (HAP-001-R15, R16, R20), and never a guard by one
    // resolution of the name followed by an operation on another (R1-016).
    //
    // The outbox root is the one directory HAP-001 declares hostile, and two
    // resolutions of one name there are a window a same-user process wins by
    // arriving between them: a `symlink_metadata` guard would report the
    // regular file it saw, a following `File::open` would dereference
    // whatever replaced it, and the remedy would then read and hash a file
    // chosen by the winner -- unbounded bytes, at a name it was never asked
    // about. `Path::exists()` is worse again for the reason it always was:
    // it follows a link and calls a *dangling* one absent, so the guard and
    // the operation disagree exactly where it matters (R1-004).
    let existing = match ports.prober.probe(&dir, &outbox.path, &file_name) {
        // Idempotent: the same bytes are already in the outbox from an
        // earlier attempt of this same remedy.
        CandidateProbe::Regular(regular) if regular.digest == *expected => Some(regular.size),
        // Other bytes, a link, a directory, or a file that answers to a
        // second name this product cannot see (HAP-001-R20): the name is
        // held by something this remedy did not create, and it is refused
        // rather than followed, overwritten, or adopted.
        CandidateProbe::Regular(_) | CandidateProbe::Escape(_) | CandidateProbe::Linked { .. } => {
            return Err(CopyInError::EntryExists);
        }
        // Nothing opened at that name: it is not there, or it is there and
        // unreadable. Both go to the write path, whose own create is
        // exclusive and reports `EntryExists` for the second.
        CandidateProbe::Unreadable => None,
    };
    let size = match existing {
        Some(size) => size,
        None => write_copy(ports, source, &outbox.path, &name, expected)?,
    };

    // The new entry is then opened exactly as any outbox entry is: once,
    // relative to the outbox handle, without following a link, with every
    // fact taken from that handle.
    let CandidateProbe::Regular(regular) = ports.prober.probe(&dir, &outbox.path, &file_name)
    else {
        return Err(CopyInError::CopyFailed);
    };
    if regular.digest != *expected {
        return Err(CopyInError::DigestChanged);
    }
    Ok(CopiedIn {
        name,
        digest: regular.digest,
        size,
        source: crate::publication::CandidateSource {
            dir,
            dir_path: outbox.path.clone(),
            file_name,
            handle: regular.handle,
        },
    })
}

/// Give the fully written `partial` its final name, created **exclusively**.
///
/// `rename` replaces whatever holds the destination name -- a file, a link,
/// a dangling link -- without a word, which is not what this remedy claims
/// to do. `link(2)` (`std::fs::hard_link`) is the one exclusive create `std`
/// offers over a file that already exists: it refuses with `AlreadyExists`
/// when anything holds the new name, and never follows a link at it. The
/// staging sibling is unlinked afterwards, so the entry ends with exactly
/// one name (R1-004).
fn link_into_place(partial: &Path, destination: &Path) -> Result<(), CopyInError> {
    let outcome = match std::fs::hard_link(partial, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(CopyInError::EntryExists)
        }
        Err(_) => Err(CopyInError::CopyFailed),
    };
    let _ = std::fs::remove_file(partial);
    outcome
}

/// Write the held handle's bytes to a `.part-` sibling created
/// exclusively, digesting exactly the bytes written, and give it the
/// entry's name -- itself created exclusively -- only once the digest
/// matches. Returns the byte count.
fn write_copy(
    ports: &CopyInPorts<'_>,
    source: &mut crate::publication::CandidateSource,
    outbox_path: &Path,
    name: &str,
    expected: &ContentDigest,
) -> Result<u64, CopyInError> {
    let partial = outbox_path.join(format!(".part-{name}"));
    let mut options = std::fs::OpenOptions::new();
    // Create-exclusive, so a link or a file planted at the name is
    // refused rather than followed or overwritten.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut sink = match options.open(&partial) {
        Ok(sink) => sink,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // A stale partial from an interrupted attempt: replaced, since
            // the product itself wrote it and it names this digest.
            std::fs::remove_file(&partial).map_err(|_| CopyInError::CopyFailed)?;
            options
                .open(&partial)
                .map_err(|_| CopyInError::CopyFailed)?
        }
        Err(_) => return Err(CopyInError::CopyFailed),
    };
    source
        .handle
        .seek(SeekFrom::Start(0))
        .map_err(|_| CopyInError::CopyFailed)?;
    let written = {
        let mut tee = crate::quarantine::Tee {
            source: &mut source.handle,
            sink: &mut sink,
        };
        ports.hasher.digest_reader(&mut tee)
    };
    let Ok((digest, size)) = written else {
        let _ = std::fs::remove_file(&partial);
        return Err(CopyInError::CopyFailed);
    };
    if sink.sync_data().is_err() {
        let _ = std::fs::remove_file(&partial);
        return Err(CopyInError::CopyFailed);
    }
    drop(sink);
    if &digest != expected {
        let _ = std::fs::remove_file(&partial);
        return Err(CopyInError::DigestChanged);
    }
    link_into_place(&partial, &outbox_path.join(name))?;
    Ok(size)
}
