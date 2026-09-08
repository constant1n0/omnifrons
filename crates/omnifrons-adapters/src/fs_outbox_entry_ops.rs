//! `FsOutboxEntryOps`: the real
//! `omnifrons_app::outbox_entry_ops::OutboxEntryOps` (spike slice 5b,
//! HAP-001-R18, R28).
//!
//! Unix: the held handle's identity comes from `fstat` and the path's from
//! `fstatat(dir, name, AT_SYMLINK_NOFOLLOW)` -- relative to the held
//! directory handle, so no directory component is re-resolved, and a link
//! at the name is compared as itself, never followed; the two are the same
//! file exactly when device and inode agree. Removal is `unlinkat(dir,
//! name)` without `AT_REMOVEDIR`, so a directory at the name is refused.
//! Both calls are gated by `nix`'s `fs` feature this crate already enables
//! (verified against the vendored 0.31.3 source: `sys::stat` is a
//! `feature = "fs"` module, `unistd::unlinkat` sits inside the
//! `feature = "fs"` block, `fcntl::AtFlags` is gated by `fs`, `process`, or
//! `user`); no dependency and no feature was added.
//!
//! Windows: `std`'s by-handle identity accessors are unstable and this
//! repository adds no Windows API dependency, so the identity is
//! `Unverifiable` and `unlink` is unsupported -- the transaction publishes
//! from the handle and defers the removal, disclosed under HAP-001-R19
//! rather than closed.

use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;

use omnifrons_app::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use omnifrons_app::run_outbox::DirectoryHandle;

/// The real, OS-backed [`OutboxEntryOps`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsOutboxEntryOps;

impl FsOutboxEntryOps {
    /// Build the ops.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(unix)]
impl OutboxEntryOps for FsOutboxEntryOps {
    fn identity(
        &self,
        dir: &DirectoryHandle,
        _dir_path: &Path,
        name: &OsStr,
        handle: &File,
    ) -> EntryIdentity {
        use nix::errno::Errno;
        use nix::fcntl::AtFlags;
        use nix::sys::stat::{fstat, fstatat};
        let Ok(held) = fstat(handle) else {
            // A handle whose own metadata cannot be read names nothing the
            // path could still name.
            return EntryIdentity::DifferentFile;
        };
        match fstatat(dir.as_file(), Path::new(name), AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(at_path) if at_path.st_dev == held.st_dev && at_path.st_ino == held.st_ino => {
                EntryIdentity::SameFile
            }
            Err(Errno::ENOENT | Errno::ENOTDIR) => EntryIdentity::Missing,
            // Another file, a link compared as itself, or a stat that could
            // not confirm the identity: none of them is the same file.
            Ok(_) | Err(_) => EntryIdentity::DifferentFile,
        }
    }

    fn unlink(&self, dir: &DirectoryHandle, _dir_path: &Path, name: &OsStr) -> std::io::Result<()> {
        use nix::unistd::{UnlinkatFlags, unlinkat};
        unlinkat(dir.as_file(), Path::new(name), UnlinkatFlags::NoRemoveDir)
            .map_err(std::io::Error::from)
    }
}

#[cfg(not(unix))]
impl OutboxEntryOps for FsOutboxEntryOps {
    fn identity(
        &self,
        _dir: &DirectoryHandle,
        _dir_path: &Path,
        _name: &OsStr,
        _handle: &File,
    ) -> EntryIdentity {
        EntryIdentity::Unverifiable
    }

    fn unlink(
        &self,
        _dir: &DirectoryHandle,
        _dir_path: &Path,
        _name: &OsStr,
    ) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}
