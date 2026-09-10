//! `FsQuarantineStore`: the real, filesystem-backed
//! `omnifrons_app::quarantine::QuarantineStore` (spike slice 5d,
//! RCS-001-R10; HAP-001 § Wrong-root detection and remedies, the
//! quarantine remedy).
//!
//! Two operations, both the ones only the platform can provide:
//!
//! * **the quarantine directory's handle**, opened without following a link
//!   at its path, through the same `open_directory_no_follow` the outbox
//!   preparer uses;
//! * **a handle-anchored move**, relative to *both* held directory handles,
//!   so no directory component of either end is re-resolved and no
//!   path-based operation is ever used (HAP-001-R17's rule for publication,
//!   applied to the move this remedy performs). `EXDEV` -- the destination
//!   is on another volume -- is reported as a fallback rather than an
//!   error, and the application's copy-then-unlink path takes over.
//!
//! **The move is `linkat` then `unlinkat`, never `renameat`** (spike slice
//! 5d, R1-021). `renameat` is atomic, and that was the reason to reach for
//! it -- but POSIX `renameat` *silently replaces* an existing destination,
//! and this branch is the one that actually runs on unix when both ends sit
//! on one volume, which is the normal case. The destination name is an
//! 8-hex short digest and a sanitized display name, both of them chosen by
//! whoever misplaced the file, so a name already held by an earlier
//! quarantined entry is brute-forceable rather than hypothetical -- and
//! replacing it would destroy a file the user deliberately preserved, whose
//! original was already unlinked, on the way to reporting a verified move.
//! HAP-001-R28 says this product never does that, and `link(2)` is the one
//! exclusive create that says so: it refuses with `EEXIST` when anything
//! holds the new name and never follows a link at it, exactly as both
//! remedies' `link_into_place` already does at their final name.
//!
//! What is traded for it is atomicity, in the direction that costs nothing:
//! between the two calls the file answers to *both* names, so an
//! interruption leaves a duplicate rather than a file with no name at all.
//! When the second call fails the first is undone -- the destination this
//! adapter created, and only ever that -- and the copy path starts from the
//! state it found. `EEXIST` reports a fallback too, so a destination
//! something already holds is verified and reused, or refused with the name
//! left holding what it held, by exactly the copy branch's own code.
//!
//! `linkat` and `unlinkat` are gated by `nix`'s `fs` feature this crate
//! already enables (the same one `FsOutboxEntryOps`' `unlinkat` uses); no
//! dependency and no feature was added.
//!
//! Windows: `std` offers no rename relative to two directory handles and
//! this repository adds no Windows API dependency, so the rename reports
//! `Unsupported` and the copy-then-unlink path always runs there -- where
//! the unlink is itself unavailable (`FsOutboxEntryOps` reports
//! `Unverifiable`), so the original is kept and the outcome says so. A
//! residual disclosed under HAP-001-R19, not a claim.

use std::ffi::OsStr;
use std::path::Path;

use omnifrons_app::publication::CandidateSource;
use omnifrons_app::quarantine::{
    FallbackReason, QuarantineError, QuarantineRoot, QuarantineStore, RenameOutcome,
};
use omnifrons_app::run_outbox::DirectoryHandle;

use crate::fs_run_outbox_preparer::open_directory_no_follow;

/// The real, OS-backed [`QuarantineStore`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsQuarantineStore;

impl FsQuarantineStore {
    /// Build a store.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Unix: `linkat` then `unlinkat`, both relative to the held directory
/// handles, so the destination name is **created exclusively** rather than
/// replaced (R1-021) -- and the file the `linkat` actually linked is
/// confirmed to be the held one before either name is touched further
/// (R1-028), because `linkat` resolves the source *name* and the handle is
/// the only thing that says which file this remedy was asked about.
#[cfg(unix)]
fn rename_anchored(
    source: &CandidateSource,
    dest_dir: &DirectoryHandle,
    dest_path: &Path,
    dest_name: &OsStr,
) -> RenameOutcome {
    use crate::fs_outbox_entry_ops::FsOutboxEntryOps;
    use nix::errno::Errno;
    use nix::fcntl::AtFlags;
    use nix::unistd::{UnlinkatFlags, linkat, unlinkat};
    use omnifrons_app::outbox_entry_ops::{EntryIdentity, OutboxEntryOps as _};

    match linkat(
        source.dir.as_file(),
        Path::new(&source.file_name),
        dest_dir.as_file(),
        Path::new(dest_name),
        // Never `AT_SYMLINK_FOLLOW`: the source name is linked as itself.
        AtFlags::empty(),
    ) {
        Ok(()) => {}
        Err(Errno::EXDEV) => return RenameOutcome::Fallback(FallbackReason::DifferentVolume),
        // `EEXIST` -- something already holds the destination name -- and
        // every other errno alike: nothing has moved, and the copy path
        // takes over. It stages, finds the name held, and verifies it like
        // any other destination: reused when it carries these very bytes,
        // refused with the name left holding what it held when it does not.
        Err(_) => return RenameOutcome::Fallback(FallbackReason::Failed),
    }
    // **What the `linkat` actually linked is checked before anything else,
    // and this is what the atomic `renameat` did not need** (spike slice
    // 5d, R1-028). Two calls can be raced where one cannot, and `linkat`
    // resolves the *source name* rather than the handle this remedy holds:
    // a name that stopped holding this file before the call linked
    // whatever holds it now -- a file this product never opened, never
    // verified and was never asked about, given a second name inside the
    // quarantine directory, while the approved file is left with no name
    // at all.
    //
    // Nothing on the source side separates that from a swap *after* the
    // link, because both end with the source name holding another file.
    // The destination does: it holds this file exactly when the `linkat`
    // linked the file the handle was opened on. The comparison is the same
    // by-handle one `remove_original` runs, taken from the same
    // `FsOutboxEntryOps`, so the sites cannot drift.
    if !matches!(
        FsOutboxEntryOps::new().identity(dest_dir, dest_path, dest_name, &source.handle),
        EntryIdentity::SameFile
    ) {
        // Undo the link *this call* created, and only ever that
        // (HAP-001-R28), then let the copy path take over from the state
        // it found -- with its own exclusive create and its own
        // verification of whatever holds the name.
        let _ = unlinkat(
            dest_dir.as_file(),
            Path::new(dest_name),
            UnlinkatFlags::NoRemoveDir,
        );
        return RenameOutcome::Fallback(FallbackReason::Failed);
    }
    // The held file answers to both names now. Removing the source name
    // leaves exactly one, and the move is that pair.
    //
    // **Which name is removed is checked too**, and for the same
    // HAP-001-R28 reason: if the source name stopped holding this file,
    // unlinking it would remove a file this product did not create.
    //
    // A source name that no longer holds this file needs no unlink at all.
    // The check above has already established that the destination link is
    // this file's name, so the move is done -- and undoing that link
    // instead would be worse than useless, leaving the file with no name at
    // all.
    match FsOutboxEntryOps::new().identity(
        &source.dir,
        &source.dir_path,
        &source.file_name,
        &source.handle,
    ) {
        EntryIdentity::Missing | EntryIdentity::DifferentFile => return RenameOutcome::Renamed,
        EntryIdentity::SameFile | EntryIdentity::Unverifiable => {}
    }
    if unlinkat(
        source.dir.as_file(),
        Path::new(&source.file_name),
        UnlinkatFlags::NoRemoveDir,
    )
    .is_ok()
    {
        return RenameOutcome::Renamed;
    }
    // The second half did not run, so the first is undone: this removes the
    // destination *this call* created and nothing else (HAP-001-R28),
    // leaving the copy path exactly the state it found.
    let _ = unlinkat(
        dest_dir.as_file(),
        Path::new(dest_name),
        UnlinkatFlags::NoRemoveDir,
    );
    RenameOutcome::Fallback(FallbackReason::Failed)
}

/// Windows: no rename relative to two directory handles in `std`.
#[cfg(not(unix))]
fn rename_anchored(
    _source: &CandidateSource,
    _dest_dir: &DirectoryHandle,
    _dest_path: &Path,
    _dest_name: &OsStr,
) -> RenameOutcome {
    RenameOutcome::Fallback(FallbackReason::Unsupported)
}

impl QuarantineStore for FsQuarantineStore {
    fn open_dir(&self, root: &QuarantineRoot) -> Result<DirectoryHandle, QuarantineError> {
        open_directory_no_follow(root.path())
            .map(DirectoryHandle::new)
            .map_err(|_| QuarantineError::Unusable)
    }

    fn rename_into(
        &self,
        source: &CandidateSource,
        dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> RenameOutcome {
        rename_anchored(source, dest_dir, dest_path, dest_name)
    }

    fn open_dest(
        &self,
        dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> Result<std::fs::File, QuarantineError> {
        // Exactly the open an outbox entry takes (`openat` with
        // `O_NOFOLLOW` on unix, the reparse point opened as itself on
        // Windows), so a link at the destination name is refused rather
        // than dereferenced by the verification that follows.
        match crate::fs_candidate_prober::open_entry(dest_dir, dest_path, dest_name) {
            crate::fs_candidate_prober::Opened::File(file) => Ok(file),
            crate::fs_candidate_prober::Opened::Refused(_)
            | crate::fs_candidate_prober::Opened::Exhausted => Err(QuarantineError::CopyFailed),
        }
    }
}
