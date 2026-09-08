//! The `OutboxEntryOps` port (spike slice 5b): the two things the
//! publication transaction does at an entry's *name* after the handle was
//! opened -- ask whether the path still names the handle's file identity
//! (HAP-001-R18, before the bytes move and again before cleanup) and, only
//! when it does, remove the entry (HAP-001-R28). Both are relative to the
//! held directory handle on unix (`fstatat`, `unlinkat`), so no directory
//! component is re-resolved; the bytes themselves never come from here.
//!
//! Where the platform cannot compare file identity from a handle
//! ([`EntryIdentity::Unverifiable`]; Windows, where `std`'s by-handle
//! accessors are unstable and this repository adds no Windows API
//! dependency), the transaction still publishes from the handle and
//! defers the removal, and says so -- the residual HAP-001-R19 requires
//! disclosed rather than closed.

use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;

use crate::run_outbox::DirectoryHandle;

/// Whether the entry named at a path is still the file a held handle
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryIdentity {
    /// The path names the handle's file (same device and inode).
    SameFile,
    /// The path names a different file, or a link.
    DifferentFile,
    /// Nothing sits at the path.
    Missing,
    /// The platform offers no by-handle identity to compare.
    Unverifiable,
}

/// A port for identity checks and removal at an outbox entry's name.
pub trait OutboxEntryOps {
    /// Whether `name` inside `dir` (whose path is `dir_path`) still names
    /// the file `handle` is open on, without following a link at the
    /// name.
    fn identity(
        &self,
        dir: &DirectoryHandle,
        dir_path: &Path,
        name: &OsStr,
        handle: &File,
    ) -> EntryIdentity;

    /// Remove the entry `name` from `dir` (a file, never a directory).
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error`; the caller never renders its
    /// text.
    fn unlink(&self, dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> std::io::Result<()>;
}
