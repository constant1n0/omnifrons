//! The `ProjectTextFile` port (spike slice 5c, HAP-001 D18): the three
//! things the guidance installer does to a managed text file at the
//! workspace root -- read it whole through one no-follow open (a regular
//! file only, at most [`MAX_MANAGED_FILE_BYTES`], valid UTF-8), replace it
//! atomically through a sibling temporary file renamed over it, and remove
//! it -- never following a link at its name. The bytes come back as bytes
//! so the caller digests exactly what is on disk, and nothing here reads a
//! file as an instruction: a guidance note is content, never authority
//! (HAP-001-R22). Implemented by `omnifrons-adapters`' `FsProjectTextFile`.
//!
//! A write carries the bytes it was planned from (R1-002): `replace` and
//! `remove` take the content the caller read, and the implementation
//! re-reads the target through the same no-follow open immediately before
//! the rename or the removal, refusing with
//! [`ProjectTextFileError::Changed`] when the target is no longer that.
//! Without it a rewrite between the caller's read and the rename would be
//! discarded silently, captured by no snapshot.

use omnifrons_domain::guidance::ManagedTarget;

use crate::harness_adapter::WorkspaceRoot;

/// The largest managed file the installer reads or writes: a guidance
/// file or an ignore file is small text, and a bound keeps a planted
/// multi-gigabyte file from being read into memory.
pub const MAX_MANAGED_FILE_BYTES: u64 = 1024 * 1024;

/// Why a [`ProjectTextFile`] operation failed. Closed and exhaustive,
/// never a raw `io::Error` whose text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProjectTextFileError {
    /// A link, a directory, or another non-regular file sits at the name.
    #[error("the managed file is not a regular file")]
    NotAFile,
    /// The file exceeds [`MAX_MANAGED_FILE_BYTES`].
    #[error("the managed file exceeds the size limit")]
    TooLarge,
    /// The file is not valid UTF-8.
    #[error("the managed file is not valid UTF-8")]
    NotUtf8,
    /// The file could not be opened or read.
    #[error("the managed file could not be read")]
    Unreadable,
    /// The file could not be written, renamed, or removed.
    #[error("the managed file could not be written")]
    WriteFailed,
    /// The target is not what the caller read: other bytes, a file where
    /// the caller found none, or none where the caller found a file. The
    /// write was refused rather than discarding the change (R1-002).
    #[error("the managed file changed between the read and the write")]
    Changed,
}

impl ProjectTextFileError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::NotAFile => "not-a-regular-file",
            Self::TooLarge => "too-large",
            Self::NotUtf8 => "not-utf8",
            Self::Unreadable => "unreadable",
            Self::WriteFailed => "write-failed",
            Self::Changed => "file-changed",
        }
    }
}

/// A port over one managed text file at a workspace root.
pub trait ProjectTextFile {
    /// The file's bytes, or `None` when nothing exists at its name.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectTextFileError::NotAFile`] for a link or a
    /// non-regular file at the name, [`ProjectTextFileError::TooLarge`]
    /// beyond the bound, [`ProjectTextFileError::NotUtf8`] for invalid
    /// UTF-8, or [`ProjectTextFileError::Unreadable`].
    fn read(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
    ) -> Result<Option<Vec<u8>>, ProjectTextFileError>;

    /// Replace the file's content with `bytes`, atomically: a sibling
    /// temporary file written first, then renamed over the target, the
    /// original's permissions preserved where the platform expresses them.
    ///
    /// `expected` is the content the caller read and planned `bytes` from
    /// (`None` for a file the caller found absent); the target is re-read
    /// through the same no-follow open immediately before the rename and
    /// the write is refused when it is no longer that.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectTextFileError::Changed`] when the target is not
    /// `expected` any more, or [`ProjectTextFileError::WriteFailed`] if the
    /// write did not complete; nothing is left at the target's name that
    /// was not there before.
    fn replace(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
        bytes: &[u8],
    ) -> Result<(), ProjectTextFileError>;

    /// Remove the file (a regular file only, never through a link); a file
    /// that does not exist is nothing to remove.
    ///
    /// `expected` binds the removal the same way [`Self::replace`] binds a
    /// write: the content the caller read, or `None` for a file the caller
    /// found absent -- which is also the only `expected` under which
    /// removing an absent file is nothing.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectTextFileError::NotAFile`] for a link or a
    /// non-regular file at the name, [`ProjectTextFileError::Changed`] when
    /// the target is not `expected` any more, or
    /// [`ProjectTextFileError::WriteFailed`] if the removal failed.
    fn remove(
        &self,
        root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
    ) -> Result<(), ProjectTextFileError>;
}
