//! The quarantine remedy (spike slice 5d, HAP-001 § Wrong-root detection
//! and remedies; RCS-001-R10, R11 and its D3).
//!
//! **Where it lives.** `<app-data>/quarantine/`, a *sibling* of the product
//! work area, created and re-checked with exactly the discipline
//! [`crate::work_area::WorkAreaRoot`] runs: owner-only where the platform
//! expresses it, canonicalized before it is used, outside every registered
//! workspace root, and re-canonicalized and re-checked at every use so a
//! workspace registered over it afterwards refuses the next move. Not
//! *inside* the work area: HAP-001 fixes that directory's contents as the
//! journal, the recovery entries, and (since slice 5c) the snapshots, and
//! a quarantined file is none of those. The siting is a spike default under
//! RCS-001 D3 and is disclosed as one (`docs/spike-log.md` § Slice 5d).
//!
//! **The move.** This is the one remedy that removes the original
//! (HAP-001 § Wrong-root detection and remedies leaves the file in place
//! for publish and ignore). It is a handle-anchored move -- the destination
//! name created and the source name then removed, both relative to held
//! directory handles -- when the platform offers one and both ends sit on
//! the same volume, and a copy followed by an unlink otherwise, with the
//! digest re-verified at the destination before the original is unlinked.
//! Where the platform offers no by-handle identity comparison or no
//! relative unlink (Windows; see
//! [`crate::outbox_entry_ops::EntryIdentity::Unverifiable`]), the copy
//! stands and the original is kept, said so rather than silently
//! duplicated -- the residual HAP-001-R19 requires disclosed, not closed.
//!
//! **Both remedies guarantee the same thing at their final name**
//! (spike slice 5d, R1-017 and R1-021), and **both branches of this one
//! do**. Neither ever reaches that name through a `rename`, which replaces
//! a file, a link or a dangling link there without a word: the copy path
//! stages into a `.part-` sibling created exclusively and then creates the
//! destination name *exclusively* too, through `link(2)` from that sibling
//! -- exactly `wrong_root::link_into_place`'s shape -- and the
//! handle-anchored branch does the same, an exclusive `link(2)` at the
//! destination followed by an unlink at the source
//! ([`QuarantineStore::rename_into`] states that as a requirement on any
//! implementation). A name something else already holds is verified like
//! any other destination and left holding exactly what it held, whether
//! that verification passes (identical bytes from an earlier attempt of
//! this same remedy, with one name and no other) or refuses. Both remedies
//! pay the same price for it: a file system with no hard links reports the
//! remedy unavailable rather than replacing.
//!
//! **And the destination is verified the way the outbox entry is**
//! (R1-022): three facts from the one handle it was opened on -- a regular
//! file, one link, and the digest and size the request bound to. Digest and
//! size alone accept a hard link planted at the destination to a file
//! outside the quarantine directory, which is HAP-001-R20's case at this
//! remedy's own final name; the publish remedy's twin refuses it through
//! `CandidateProber::probe`, and so does this one now. Windows reads no
//! link count from a handle in stable `std`, so that refusal cannot be made
//! there -- the same residual the outbox entry carries, disclosed under
//! HAP-001-R19.
//!
//! **What is anchored to a handle, and what is not.** The identity
//! re-check and the original's unlink run relative to the held source
//! directory handle; the handle-anchored branch's own link and unlink are
//! relative to both directory handles. The copy branch's own writes inside
//! the quarantine directory --
//! the staging create, the exclusive create of the destination name, and
//! the cleanup of a destination this copy created -- are **by path**, since
//! `std` offers no `openat`, `linkat` or `unlinkat` and this crate depends
//! on `omnifrons-domain` and `thiserror` alone. The precedent is
//! `publication::write_recovery_entry` and the publish remedy's own
//! copy-in; the quarantine directory is owner-only and outside every
//! registered workspace, which is why this is a disclosed residual rather
//! than a finding (`docs/spike-log.md` § Slice 5d).

use std::ffi::OsStr;
use std::io::{Read, Seek as _, SeekFrom, Write};
use std::path::{Path, PathBuf};

use omnifrons_domain::outbox::ContentDigest;
use omnifrons_domain::publication::DisplayName;

use crate::content_hasher::ContentHasher;
use crate::harness_adapter::WorkspaceRoot;
use crate::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use crate::publication::CandidateSource;
use crate::run_outbox::DirectoryHandle;
use crate::work_area::{ContainmentError, canonical_outside_workspaces};

/// Why a quarantine operation failed. Closed and exhaustive, never a raw
/// `io::Error` whose text can carry a device path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuarantineError {
    /// The quarantine directory could not be created, canonicalized,
    /// opened, or written.
    #[error("the quarantine directory could not be used")]
    Unusable,
    /// The quarantine directory resolves inside (or is) a registered
    /// workspace root (RCS-001-R10).
    #[error("the quarantine directory resolves inside a registered workspace root")]
    InsideWorkspace,
    /// The quarantine directory's permissions could not be made
    /// owner-only.
    #[error("the quarantine directory could not be made owner-only")]
    NotOwnerOnly,
    /// The held handle is not a regular file.
    #[error("the file is not a regular file")]
    NotRegularFile,
    /// The name no longer holds the file the handle was opened on: the
    /// same substitution HAP-001-R18 refuses in a publication, refused
    /// here before anything moves.
    #[error("the path no longer names the file that was opened")]
    PathChanged,
    /// The bytes read from the held handle no longer hash to the digest
    /// the remedy was requested for.
    #[error("the file's content changed since it was found")]
    DigestChanged,
    /// The copy, or the destination it was written to, could not be
    /// completed or read back.
    #[error("the file could not be moved into quarantine")]
    CopyFailed,
    /// The quarantine destination holds a file with more than one link
    /// (HAP-001-R20, applied at the quarantine destination exactly as the
    /// publish remedy applies it at the outbox entry; spike slice 5d,
    /// R1-022). The same bytes answer to another name this product cannot
    /// see, so accepting them as this remedy's own earlier result would
    /// unlink the project's copy in favour of an alias whoever holds that
    /// other name can rewrite afterwards.
    #[error("the quarantine destination has more than one name")]
    DestinationLinked {
        /// How many links the destination handle's own metadata reported.
        link_count: u64,
    },
}

impl From<ContainmentError> for QuarantineError {
    fn from(error: ContainmentError) -> Self {
        match error {
            ContainmentError::Unusable => Self::Unusable,
            ContainmentError::InsideWorkspace => Self::InsideWorkspace,
        }
    }
}

/// The opened, canonical quarantine directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRoot {
    path: PathBuf,
}

impl QuarantineRoot {
    /// Open the quarantine directory configured at `configured`, creating
    /// it owner-only when missing, after checking that it does not resolve
    /// inside any of `workspaces` (RCS-001-R10 at configuration time).
    /// Nothing is created when the check fails.
    ///
    /// # Errors
    ///
    /// Returns [`QuarantineError::InsideWorkspace`] if the configured path
    /// resolves inside a registered workspace root,
    /// [`QuarantineError::NotOwnerOnly`] if it could not be made
    /// owner-only, or [`QuarantineError::Unusable`] if it could not be
    /// created or canonicalized.
    pub fn open(configured: &Path, workspaces: &[&WorkspaceRoot]) -> Result<Self, QuarantineError> {
        crate::work_area::refuse_inside_workspaces(configured, workspaces)?;
        crate::work_area::create_owner_only_dir(configured)
            .map_err(|_| QuarantineError::Unusable)?;
        let path = canonical_outside_workspaces(configured, workspaces)?;
        if !crate::work_area::is_owner_only(&path).map_err(|_| QuarantineError::Unusable)? {
            return Err(QuarantineError::NotOwnerOnly);
        }
        Ok(Self { path })
    }

    /// Re-canonicalize and re-check the quarantine directory against
    /// `workspaces` (RCS-001-R10 at every use).
    ///
    /// # Errors
    ///
    /// Returns [`QuarantineError::InsideWorkspace`] if it now resolves
    /// inside a registered workspace root, or [`QuarantineError::Unusable`]
    /// if it can no longer be canonicalized.
    pub fn check(&self, workspaces: &[&WorkspaceRoot]) -> Result<(), QuarantineError> {
        let canonical = canonical_outside_workspaces(&self.path, workspaces)?;
        if canonical != self.path {
            return Err(QuarantineError::Unusable);
        }
        Ok(())
    }

    /// The canonical quarantine path. Device configuration only: never a
    /// record, an event, or an IPC payload value (HAP-001-R5,
    /// RCS-001-R14).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Why a handle-anchored rename was not the move that ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The destination is on another volume.
    DifferentVolume,
    /// The platform offers no handle-anchored rename.
    Unsupported,
    /// The handle-anchored move was attempted and did not happen.
    ///
    /// A destination name something already holds is one of these
    /// (R1-021): the move refuses to replace it, and the copy path that
    /// follows verifies whatever holds the name like any other destination
    /// -- reusing it when it carries these very bytes and refusing when it
    /// does not, with the name left holding exactly what it held either
    /// way.
    Failed,
}

impl FallbackReason {
    /// This reason's fixed token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::DifferentVolume => "different-volume",
            Self::Unsupported => "rename-unsupported",
            Self::Failed => "rename-failed",
        }
    }
}

/// What a handle-anchored rename attempt yielded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameOutcome {
    /// The file the source handle was opened on now sits at the
    /// destination name, and the source name no longer holds it.
    ///
    /// **Both halves are facts about *that* file** (spike slice 5d,
    /// R1-028). An implementation reports this only once it has confirmed
    /// the destination name holds the very file the handle holds -- never
    /// merely that some link was created and some name stopped holding
    /// something. A move that resolves the source name rather than the
    /// handle can link a file substituted at that name in between, and
    /// there the destination is a second name for a file nobody asked
    /// about while the file this remedy was requested for has none at all;
    /// that is a [`Fallback`](Self::Fallback), not this.
    Renamed,
    /// The rename was not used; the copy-then-unlink path applies.
    Fallback(FallbackReason),
}

/// A port over the two things a quarantine move needs from the platform:
/// a handle on the quarantine directory itself, and a rename anchored to
/// two directory handles.
///
/// Everything else -- the copy, the digest, the identity re-check, the
/// unlink -- is already expressed by ports this crate has
/// ([`ContentHasher`], [`OutboxEntryOps`]) or by `std`, exactly as
/// `publication::write_recovery_entry` writes a recovery entry with `std`
/// against a directory the product itself owns.
pub trait QuarantineStore {
    /// Open `root` as a directory handle, without following a link at its
    /// path.
    ///
    /// # Errors
    ///
    /// Returns [`QuarantineError::Unusable`] if it could not be opened.
    fn open_dir(&self, root: &QuarantineRoot) -> Result<DirectoryHandle, QuarantineError>;

    /// Move `source`'s entry into `dest_dir` under `dest_name`, anchored
    /// to both directory handles and never to a re-resolved path.
    ///
    /// An implementation **MUST NOT replace anything already holding
    /// `dest_name`** (spike slice 5d, R1-021). A plain `rename` does, at a
    /// name whose two halves -- an 8-hex short digest and a sanitized
    /// display name -- are both chosen by whoever misplaced the file, so it
    /// would destroy a file the user deliberately preserved on the way to
    /// reporting a verified move, which is the one thing HAP-001-R28 says
    /// this product never does. An implementation that cannot create the
    /// destination name exclusively MUST report a [`RenameOutcome::Fallback`]
    /// instead: the copy path then verifies whatever holds the name like any
    /// other destination and reuses it or refuses, leaving it exactly as it
    /// was found either way.
    fn rename_into(
        &self,
        source: &CandidateSource,
        dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> RenameOutcome;

    /// Open the entry `dest_name` inside `dest_dir` for reading **without
    /// following a link at its name**, so the verification that follows
    /// reads the file that actually arrived.
    ///
    /// A plain `File::open` here would dereference a link sitting at the
    /// destination name and digest whatever it points at; since the
    /// destination name is derived from the digest the request bound to,
    /// that check would then *pass* over a file outside the quarantine
    /// directory and the remedy would report a verified move that never
    /// happened.
    ///
    /// # Errors
    ///
    /// Returns [`QuarantineError::CopyFailed`] if the destination could not
    /// be opened as itself.
    fn open_dest(
        &self,
        dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> Result<std::fs::File, QuarantineError>;
}

/// How the file reached the quarantine directory, and what became of the
/// original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuarantineMove {
    /// Moved by the handle-anchored branch, which reports this only once
    /// the original name no longer holds the file -- unlinked by the move
    /// itself, or already pointed at something else by the time it ran.
    Renamed,
    /// Moved by the handle-anchored branch -- so the file has left the
    /// project and the original name no longer holds it -- but the
    /// destination could not be re-opened and verified afterwards.
    ///
    /// The move happened; the verification did not. A store reports
    /// [`RenameOutcome::Renamed`] only when the destination name holds the
    /// file the handle was opened on and the original name no longer does
    /// (anything short of that is a [`RenameOutcome::Fallback`], and
    /// nothing has moved), so once it has
    /// there is no outcome that can honestly say the file is still where
    /// it was: the failure is reported *about the verification*, under the
    /// destination name the payload carries, and never as a refusal.
    RenamedUnverified,
    /// Copied from the held handle, verified at the destination, and the
    /// original then unlinked relative to its own directory handle.
    CopiedAndUnlinked {
        /// Why the rename was not used.
        fallback: FallbackReason,
    },
    /// Copied and verified, but the original was kept, for this fixed
    /// reason token -- never silently duplicated.
    CopiedOriginalKept {
        /// Why the rename was not used.
        fallback: FallbackReason,
        /// Why the original could not be removed.
        reason: &'static str,
    },
}

/// One file moved into quarantine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quarantined {
    /// The sanitized name the file now carries inside the quarantine
    /// directory -- one name component, never a path (RCS-001's file-name
    /// rule).
    pub name: String,
    /// The digest verified at the destination.
    pub digest: ContentDigest,
    /// How many bytes it carries.
    pub size: u64,
    /// How it got there, and what became of the original.
    pub outcome: QuarantineMove,
}

/// The ports one quarantine move runs over.
pub struct QuarantinePorts<'a> {
    /// The platform's directory handle and handle-anchored rename.
    pub store: &'a dyn QuarantineStore,
    /// SHA-256 over the held handle and over the destination.
    pub hasher: &'a dyn ContentHasher,
    /// The identity check and the removal at the original's name.
    pub entry_ops: &'a dyn OutboxEntryOps,
    /// The quarantine directory, re-checked before anything moves.
    pub root: &'a QuarantineRoot,
    /// Every registered workspace root it is checked against.
    pub workspaces: &'a [&'a WorkspaceRoot],
}

/// The name a quarantined file takes: its sanitized display name behind
/// the short hex of its digest, so two files of the same name never
/// collide and the name is one component under RCS-001's file-name rule.
#[must_use]
pub fn quarantine_name(display: &DisplayName, digest: &ContentDigest) -> String {
    DisplayName::sanitize(&format!("{}-{}", digest.short_hex(), display.as_str()))
        .as_str()
        .to_string()
}

/// A `Read` that copies everything it yields into `sink`, so one pass over
/// the held handle both writes the destination and digests exactly the
/// bytes written -- the same shape `publication`'s own copy uses.
pub(crate) struct Tee<'a> {
    pub(crate) source: &'a mut std::fs::File,
    pub(crate) sink: &'a mut dyn Write,
}

impl Read for Tee<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.source.read(buf)?;
        self.sink.write_all(&buf[..read])?;
        Ok(read)
    }
}

/// Move the file `source` holds open into the quarantine directory
/// (HAP-001 § Wrong-root detection and remedies, the quarantine remedy).
///
/// `expected` is the digest the remedy was requested for -- the one the
/// surface showed and the request bound to -- and nothing moves unless the
/// held handle's bytes still hash to it.
///
/// # Errors
///
/// Returns [`QuarantineError`] naming the step that refused: the
/// quarantine directory's own containment re-check, a handle that is not a
/// regular file, a name that no longer holds the handle's file, content
/// that changed since it was found, or a copy that could not be completed
/// or read back.
pub fn quarantine(
    ports: &QuarantinePorts<'_>,
    source: &mut CandidateSource,
    expected: &ContentDigest,
    display: &DisplayName,
) -> Result<Quarantined, QuarantineError> {
    // RCS-001-R10 before anything else, exactly as the publication
    // transaction re-checks the work area first (HAP-001-R7).
    ports.root.check(ports.workspaces)?;

    let metadata = source
        .handle
        .metadata()
        .map_err(|_| QuarantineError::NotRegularFile)?;
    if !metadata.file_type().is_file() {
        return Err(QuarantineError::NotRegularFile);
    }

    // HAP-001-R18's check, applied to the misplaced file: the name must
    // still hold the file the handle was opened on before anything moves.
    match ports.entry_ops.identity(
        &source.dir,
        &source.dir_path,
        &source.file_name,
        &source.handle,
    ) {
        EntryIdentity::SameFile | EntryIdentity::Unverifiable => {}
        EntryIdentity::Missing | EntryIdentity::DifferentFile => {
            return Err(QuarantineError::PathChanged);
        }
    }

    // The bytes are the ones the remedy was requested for, taken from the
    // held handle and never from the path.
    source
        .handle
        .seek(SeekFrom::Start(0))
        .map_err(|_| QuarantineError::CopyFailed)?;
    let (held_digest, held_size) = ports
        .hasher
        .digest_reader(&mut source.handle)
        .map_err(|_| QuarantineError::CopyFailed)?;
    if &held_digest != expected {
        return Err(QuarantineError::DigestChanged);
    }

    let dest_dir = ports.store.open_dir(ports.root)?;
    let dest_path = ports.root.path().to_path_buf();
    let name = quarantine_name(display, expected);

    match ports
        .store
        .rename_into(source, &dest_dir, &dest_path, OsStr::new(&name))
    {
        RenameOutcome::Renamed => {
            // The handle-anchored move has already run, and a store
            // reports this outcome only once the destination name holds
            // the handle's file and the original name no longer does. A
            // verification that fails from here on is a fact about the
            // *destination*, never
            // a refusal -- returning one would tell the user the file is
            // still where it was while it sits in the quarantine directory
            // under a name only this payload knows (R3-004, R1-007).
            let outcome = match verify_destination(
                ports, &dest_dir, &dest_path, &name, expected, held_size,
            ) {
                Ok(()) => QuarantineMove::Renamed,
                Err(_) => QuarantineMove::RenamedUnverified,
            };
            Ok(Quarantined {
                name,
                digest: *expected,
                size: held_size,
                outcome,
            })
        }
        RenameOutcome::Fallback(fallback) => {
            // Nothing has moved yet, so a failure here *is* a refusal and
            // the original really is still where it was.
            let copied = copy_into(ports.hasher, source, &dest_path, &name, expected)?;
            if let Err(error) =
                verify_destination(ports, &dest_dir, &dest_path, &name, expected, held_size)
            {
                // The copy is not the file it was meant to be and the
                // original is untouched: leave nothing behind in the
                // quarantine directory to be mistaken for a moved file.
                //
                // **Only what this copy created.** A name something else
                // already held is left exactly as it was found: this product
                // never removes a file it did not create (HAP-001-R28), and
                // that is as true of a cleanup as of a remedy (R1-017).
                if copied == Copied::Created {
                    let _ = std::fs::remove_file(dest_path.join(&name));
                }
                return Err(error);
            }
            let outcome = remove_original(ports, source, fallback);
            Ok(Quarantined {
                name,
                digest: *expected,
                size: held_size,
                outcome,
            })
        }
    }
}

/// What the copy path found at the destination name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Copied {
    /// The destination name was created by this copy.
    Created,
    /// Something already held the name. Nothing was created there and
    /// nothing replaced; the caller verifies it like any other destination,
    /// so the same bytes from an earlier attempt of this same remedy pass
    /// and anything else refuses.
    AlreadyThere,
}

/// Copy the held handle's bytes into `dest_path`/`name`, created
/// exclusively and owner-only, digesting exactly the bytes written.
fn copy_into(
    hasher: &dyn ContentHasher,
    source: &mut CandidateSource,
    dest_path: &Path,
    name: &str,
    expected: &ContentDigest,
) -> Result<Copied, QuarantineError> {
    let partial = dest_path.join(format!(".part-{name}"));
    let mut options = std::fs::OpenOptions::new();
    // Create-exclusive, exactly as the publish remedy's twin
    // (`wrong_root::write_copy`) creates its own staging sibling: this name
    // is fully predictable -- both the short digest hex and the display
    // name come from the file whoever misplaced it chose -- so a `create`
    // that follows a link planted at it would write the moved bytes
    // straight through into any file the user can write, and the
    // verification below would then digest the victim and pass (R1-001).
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut sink = match options.open(&partial) {
        Ok(sink) => sink,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Something already holds the staging name: a stale partial
            // from an interrupted attempt, or a planted file or link.
            // Removing it unlinks *that name* -- never what a link points
            // at -- and the create below is exclusive again.
            std::fs::remove_file(&partial).map_err(|_| QuarantineError::CopyFailed)?;
            options
                .open(&partial)
                .map_err(|_| QuarantineError::CopyFailed)?
        }
        Err(_) => return Err(QuarantineError::CopyFailed),
    };
    source
        .handle
        .seek(SeekFrom::Start(0))
        .map_err(|_| QuarantineError::CopyFailed)?;
    let written = {
        let mut tee = Tee {
            source: &mut source.handle,
            sink: &mut sink,
        };
        hasher.digest_reader(&mut tee)
    };
    let Ok((digest, _)) = written else {
        let _ = std::fs::remove_file(&partial);
        return Err(QuarantineError::CopyFailed);
    };
    if sink.sync_data().is_err() {
        let _ = std::fs::remove_file(&partial);
        return Err(QuarantineError::CopyFailed);
    }
    drop(sink);
    if &digest != expected {
        let _ = std::fs::remove_file(&partial);
        return Err(QuarantineError::DigestChanged);
    }
    link_into_place(&partial, &dest_path.join(name))
}

/// Give the fully written `partial` the destination's name, created
/// **exclusively**, and unlink the staging sibling either way so the file
/// ends with exactly one name here.
///
/// `rename` replaces whatever holds that name -- a file, a link, a
/// *dangling* link -- without a word, which would make this remedy destroy
/// a file it did not create on its way to reporting a verified move.
/// `link(2)` (`std::fs::hard_link`) is the one exclusive create `std` offers
/// over a file that already exists: it refuses with `AlreadyExists` when
/// anything holds the new name, and never follows a link at it. This is
/// exactly the shape the publish remedy's `wrong_root::link_into_place`
/// uses, so both remedies now guarantee the same thing at their final name
/// (R1-017); the cost both pay is the same too -- a file system with no
/// hard links reports the remedy unavailable rather than replacing.
///
/// `AlreadyExists` is not a refusal here. The caller re-opens the
/// destination through [`QuarantineStore::open_dest`] and verifies it
/// immediately afterwards -- one link, then the digest and the size
/// ([`verify_destination`], R1-022) -- so an earlier attempt of this same
/// remedy verifies and the move completes, while other bytes, or the same
/// bytes at a name that answers to a second one, fail there and the remedy
/// refuses, with the name left holding exactly what it held before.
fn link_into_place(partial: &Path, destination: &Path) -> Result<Copied, QuarantineError> {
    let outcome = match std::fs::hard_link(partial, destination) {
        Ok(()) => Ok(Copied::Created),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(Copied::AlreadyThere),
        Err(_) => Err(QuarantineError::CopyFailed),
    };
    let _ = std::fs::remove_file(partial);
    outcome
}

/// Re-open the destination and digest it: the file that arrived is the one
/// that was approved, checked before the original is unlinked.
///
/// Opened through [`QuarantineStore::open_dest`], which never follows a
/// link at the destination name -- a plain open would dereference one and
/// digest whatever it points at, which is exactly how a planted link turns
/// this check into a rubber stamp (R1-001).
///
/// **Three facts, all of them from that one handle**, and the link count is
/// the third for the reason HAP-001-R20 gives at the outbox entry (spike
/// slice 5d, R1-022). Digest and size alone cannot tell this remedy's own
/// earlier result apart from a hard link a same-user producer planted at
/// the destination, pointing at a file outside the quarantine directory
/// that carries the very bytes the request bound to: accepting it unlinks
/// the project's copy and leaves an alias of an inode the producer still
/// holds another name for, and can rewrite once the surface has said the
/// file is safely quarantined. The publish remedy's twin refuses exactly
/// this through `CandidateProber::probe`, and both remedies have to
/// guarantee the same thing at their final name.
fn verify_destination(
    ports: &QuarantinePorts<'_>,
    dest_dir: &DirectoryHandle,
    dest_path: &Path,
    name: &str,
    expected: &ContentDigest,
    expected_size: u64,
) -> Result<(), QuarantineError> {
    let mut file = ports
        .store
        .open_dest(dest_dir, dest_path, OsStr::new(name))?;
    // `fstat` on the handle just opened, never a stat by path.
    let metadata = file.metadata().map_err(|_| QuarantineError::CopyFailed)?;
    if !metadata.file_type().is_file() {
        return Err(QuarantineError::CopyFailed);
    }
    // `None` is the Windows seam: the by-handle accessor in
    // `std::os::windows::fs::MetadataExt` is unstable and this repository
    // adds no Windows API dependency, so this refusal cannot be made there
    // -- the same residual the outbox entry already carries, disclosed
    // under HAP-001-R19 rather than claimed.
    if let Some(count) = crate::wrong_root::link_count(&metadata)
        && count > 1
    {
        return Err(QuarantineError::DestinationLinked { link_count: count });
    }
    let (digest, size) = ports
        .hasher
        .digest_reader(&mut file)
        .map_err(|_| QuarantineError::CopyFailed)?;
    if digest != *expected || size != expected_size {
        return Err(QuarantineError::DigestChanged);
    }
    Ok(())
}

/// Remove the original only while its name still holds the handle's file,
/// and only relative to the directory handle it was opened under.
fn remove_original(
    ports: &QuarantinePorts<'_>,
    source: &CandidateSource,
    fallback: FallbackReason,
) -> QuarantineMove {
    let kept = |reason| QuarantineMove::CopiedOriginalKept { fallback, reason };
    match ports.entry_ops.identity(
        &source.dir,
        &source.dir_path,
        &source.file_name,
        &source.handle,
    ) {
        EntryIdentity::SameFile => {
            match ports
                .entry_ops
                .unlink(&source.dir, &source.dir_path, &source.file_name)
            {
                Ok(()) => QuarantineMove::CopiedAndUnlinked { fallback },
                Err(_) => kept("unlink-failed"),
            }
        }
        EntryIdentity::Missing => kept("original-already-gone"),
        EntryIdentity::DifferentFile => kept("original-changed-during-the-move"),
        EntryIdentity::Unverifiable => kept("identity-check-unavailable"),
    }
}
