//! `FsProjectTextFile`: the real
//! `omnifrons_app::managed_file::ProjectTextFile` (spike slice 5c,
//! HAP-001 D18) over one managed text file at the workspace root.
//!
//! Reading opens the file exactly once without following a link at its
//! name -- unix `O_NOFOLLOW` (`ELOOP` is refused as not a file, never
//! dereferenced), Windows `FILE_FLAG_OPEN_REPARSE_POINT` with the handle's
//! own metadata refusing a reparse point -- takes the regular-file fact
//! and the size from that handle, bounds the read twice (on the reported
//! size and on the bytes actually read, so a file growing between the two
//! is still refused), and refuses invalid UTF-8. Replacing writes a
//! sibling `.<name>.<pid>-<seq>.part` file created exclusively, syncs it,
//! carries the original's permissions over where the platform expresses
//! them, and renames it over the target, so the target never holds a
//! partial write; a link or a directory at the name is refused rather
//! than replaced. Removing never follows a link either. Nothing here
//! reads a file as an instruction (HAP-001-R22).
//!
//! A write binds to the bytes it was planned from (R1-002): immediately
//! before the rename -- and before the removal -- the target is re-read
//! through the same no-follow open and compared with the caller's
//! `expected`, refusing with [`ProjectTextFileError::Changed`] when it is
//! no longer that. The window between that re-read and the rename itself
//! stays open, and two vectors reach into it (R1-006). First, the target
//! itself: `rename(2)` cannot be made conditional on the target's content
//! through `std`, so a rewrite landing inside the window is still
//! overwritten, silently and captured by no snapshot. Second, the `.part`
//! file: it sits in the workspace root, a directory the harness can write
//! to, under a name predictable but for its sequence number, so an unlink
//! and a symbolic link created at that name inside the window make the
//! rename move the *link*, leaving a symbolic link at the managed file's
//! name and discarding the bytes that were planned. The second is not a
//! silent write: the caller's read-back goes through the same no-follow
//! open, which refuses the planted link as
//! [`ProjectTextFileError::NotAFile`] (`guidance-file-invalid` on the
//! wire), so the write is never reported as having succeeded. Both are
//! residuals this slice discloses rather than closes; closing either
//! needs a lock this product does not take on a file the user owns, or a
//! platform primitive outside `std`.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::managed_file::{MAX_MANAGED_FILE_BYTES, ProjectTextFile, ProjectTextFileError};
use omnifrons_domain::guidance::ManagedTarget;

/// The real, OS-backed [`ProjectTextFile`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsProjectTextFile;

impl FsProjectTextFile {
    /// Build the port.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// The process-wide sequence behind the sibling `.part` names, so two
/// replacements of one file never share a temporary file.
static PART_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// What opening the managed file without following a link yielded.
/// Shared with `jsonl_catalog_store`'s repair (spike slice 5e), which
/// rewrites a file under the project's own `.omnifrons/` and needs the
/// same discipline this module already proved.
pub(crate) enum Opened {
    File(File),
    Absent,
    NotAFile,
    Unreadable,
}

/// Unix: `O_NOFOLLOW` refuses a link at the name with `ELOOP`; a FIFO
/// opens without blocking under `O_NONBLOCK` and is refused from `fstat`.
#[cfg(unix)]
pub(crate) fn open_no_follow(path: &Path) -> Opened {
    use nix::errno::Errno;
    use nix::fcntl::{OFlag, open};
    use nix::sys::stat::Mode;
    match open(
        path,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => Opened::File(File::from(fd)),
        Err(Errno::ENOENT) => Opened::Absent,
        Err(Errno::ELOOP | Errno::ENXIO | Errno::EOPNOTSUPP | Errno::ENOTDIR) => Opened::NotAFile,
        Err(_) => Opened::Unreadable,
    }
}

/// Windows: the reparse point is opened as itself and refused from the
/// handle's metadata; a directory, which will not open as a file, is
/// classified from the path's own metadata.
#[cfg(not(unix))]
pub(crate) fn open_no_follow(path: &Path) -> Opened {
    let opened = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            OpenOptions::new()
                .read(true)
                .custom_flags(
                    crate::fs_run_outbox_preparer::win_flags::FILE_FLAG_OPEN_REPARSE_POINT,
                )
                .open(path)
        }
        #[cfg(not(windows))]
        {
            File::open(path)
        }
    };
    match opened {
        Ok(file) => match file.metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => Opened::NotAFile,
            Ok(_) => Opened::File(file),
            Err(_) => Opened::Unreadable,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Opened::Absent,
        Err(_) => match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Opened::NotAFile
            }
            _ => Opened::Unreadable,
        },
    }
}

/// One no-follow open of `path` and everything read through that handle:
/// `Ok(None)` when nothing is there, else the size the handle reports and
/// at most [`MAX_MANAGED_FILE_BYTES`] + 1 bytes -- the extra byte lets the
/// caller tell an oversized file from one exactly at the bound, and the
/// cap keeps a planted multi-gigabyte file out of memory whatever the
/// reported size says. The regular-file fact comes from the handle, never
/// from a stat by path.
fn open_and_read(path: &Path) -> Result<Option<(u64, Vec<u8>)>, ProjectTextFileError> {
    let mut file = match open_no_follow(path) {
        Opened::File(file) => file,
        Opened::Absent => return Ok(None),
        Opened::NotAFile => return Err(ProjectTextFileError::NotAFile),
        Opened::Unreadable => return Err(ProjectTextFileError::Unreadable),
    };
    let metadata = file
        .metadata()
        .map_err(|_| ProjectTextFileError::Unreadable)?;
    if !metadata.file_type().is_file() {
        return Err(ProjectTextFileError::NotAFile);
    }
    let size = metadata.len();
    let mut bytes =
        Vec::with_capacity(usize::try_from(size.min(MAX_MANAGED_FILE_BYTES)).unwrap_or(0));
    (&mut file)
        .take(MAX_MANAGED_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProjectTextFileError::Unreadable)?;
    Ok(Some((size, bytes)))
}

/// Whether the target still carries `expected` -- the bytes the caller
/// read and planned its write from, or `None` for a file the caller found
/// absent -- read through the same no-follow open (R1-002).
fn still_expected(path: &Path, expected: Option<&[u8]>) -> Result<(), ProjectTextFileError> {
    let current = open_and_read(path)?.map(|(_, bytes)| bytes);
    if current.as_deref() == expected {
        Ok(())
    } else {
        Err(ProjectTextFileError::Changed)
    }
}

/// The metadata of a regular file at `path` without following a link:
/// `Ok(None)` when nothing is there, `Err(NotAFile)` for a link or a
/// non-regular file.
fn regular_file_at(path: &Path) -> Result<Option<std::fs::Metadata>, ProjectTextFileError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata)),
        Ok(_) => Err(ProjectTextFileError::NotAFile),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ProjectTextFileError::Unreadable),
    }
}

impl ProjectTextFile for FsProjectTextFile {
    fn read(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
    ) -> Result<Option<Vec<u8>>, ProjectTextFileError> {
        let path = root.path().join(target.file_name());
        // Every fact from the handle just opened, never a stat by path.
        let Some((size, bytes)) = open_and_read(&path)? else {
            return Ok(None);
        };
        // The bound twice: the size the handle reported, and the bytes
        // actually read, so a file growing between the two is still
        // refused.
        if size > MAX_MANAGED_FILE_BYTES
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_MANAGED_FILE_BYTES
        {
            return Err(ProjectTextFileError::TooLarge);
        }
        if std::str::from_utf8(&bytes).is_err() {
            return Err(ProjectTextFileError::NotUtf8);
        }
        Ok(Some(bytes))
    }

    fn replace(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
        bytes: &[u8],
    ) -> Result<(), ProjectTextFileError> {
        let final_path = root.path().join(target.file_name());
        let existing = regular_file_at(&final_path)?;
        let sequence = PART_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = root.path().join(format!(
            ".{}.{}-{sequence}.part",
            target.file_name(),
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| ProjectTextFileError::WriteFailed)?;
        let prepared = file
            .write_all(bytes)
            .and_then(|()| file.sync_data())
            .and_then(|()| match &existing {
                // The original's permissions carry over to the replacement
                // (mode on unix, the read-only bit on Windows).
                Some(metadata) => file.set_permissions(metadata.permissions()),
                None => Ok(()),
            });
        drop(file);
        if prepared.is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(ProjectTextFileError::WriteFailed);
        }
        // R1-002: the last thing before the rename is a fresh read of the
        // target through the same no-follow open. A rewrite that landed
        // since the caller read is refused, not discarded.
        if let Err(error) = still_expected(&final_path, expected) {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        if std::fs::rename(&temp, &final_path).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(ProjectTextFileError::WriteFailed);
        }
        Ok(())
    }

    fn remove(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
    ) -> Result<(), ProjectTextFileError> {
        let path = root.path().join(target.file_name());
        still_expected(&path, expected)?;
        match regular_file_at(&path)? {
            None => Ok(()),
            Some(_) => std::fs::remove_file(&path).map_err(|_| ProjectTextFileError::WriteFailed),
        }
    }
}
