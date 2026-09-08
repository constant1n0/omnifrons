//! `FsRunOutboxPreparer`: the real, filesystem-backed
//! `omnifrons_app::RunOutboxPreparer` (spike slice 5, HAP-001-R10).
//!
//! Before creating a run subdirectory the declared outbox is validated
//! again -- canonicalized, confirmed inside the project root, a real
//! directory, not a link -- and opened *without following a link at its
//! path* (unix: `O_NOFOLLOW | O_DIRECTORY`). The subdirectory is then
//! created *relative to that handle* (unix: `mkdirat`, so no path is
//! re-resolved between the validation and the creation) with a
//! create-exclusive operation that fails on any existing name -- a file,
//! a directory, a link of any kind, dangling or not -- and owner-only
//! permissions; the created directory is opened, again without following
//! a link, relative to the same handle; its permissions are forced to
//! `0o700` through that handle (`fchmod`); and "verify by handle" then
//! means: the handle opened relative to the outbox and a handle opened at
//! the exact path the harness will be given (`O_NOFOLLOW | O_DIRECTORY`
//! again) name the same file (`dev`, `ino`), that file is a directory,
//! and its mode is owner-only. A failure at any step renders
//! `outbox-unavailable` and nothing is launched (HAP-001 D14).
//!
//! One POSIX residual is disclosed rather than closed: there is no atomic
//! create-and-open for directories, so between `mkdirat` returning and the
//! `openat` that follows a process running as the same user can remove the
//! directory just created and place a different *directory* at the name
//! (`O_NOFOLLOW | O_DIRECTORY` refuses a link or a non-directory there,
//! not another directory). What the verification proves is therefore that
//! the directory the harness is pointed at is the one whose identity was
//! captured on that first `openat` -- owner-only, a real directory, the
//! same file the declared path names -- not that no same-user swap
//! happened before that open: TM-001 A6's same-user residual, which
//! HAP-001 § Run subdirectories and attribution already declines to
//! pretend away, and the reason attribution rests on the run's own
//! digest-naming proposal rather than on location.
//!
//! Windows: the outbox and the created directory are opened with
//! `FILE_FLAG_OPEN_REPARSE_POINT` (a reparse point at the path opens as
//! itself and is refused, never followed) and `FILE_FLAG_BACKUP_SEMANTICS`
//! (required to open a directory handle at all); creation is
//! `CreateDirectoryW`, which fails with `ERROR_ALREADY_EXISTS` on any
//! existing name. Two residuals are disclosed rather than closed
//! (HAP-001-R19): the created directory is not relative to the outbox
//! handle (Windows has no `mkdirat`), and the file identity comparison
//! is not performed, because the by-handle identity accessors of
//! `std::os::windows::fs::MetadataExt` are unstable (`windows_by_handle`)
//! and this crate adds no Windows API dependency; owner-only permissions
//! are not expressed either (ACLs are outside `std`). What the handle
//! still guarantees there: it is a directory, and it was not a reparse
//! point when opened.

use std::fs::File;
use std::path::Path;

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::run_outbox::{
    DirectoryHandle, OpenedOutbox, OutboxLocation, OutboxPath, PrepareError,
    PreparedRunSubdirectory, RunId, RunOutboxPreparer, VerificationFailure,
    validate_outbox_declaration,
};

/// Win32 open flags this crate needs on Windows, verified against the
/// vendored `windows-sys` 0.61.2 (`Windows::Win32::Storage::FileSystem`:
/// `FILE_FLAG_BACKUP_SEMANTICS = 33554432`,
/// `FILE_FLAG_OPEN_REPARSE_POINT = 2097152`) rather than assumed; named
/// here so this crate adds no dependency for two constants.
#[cfg(windows)]
pub(crate) mod win_flags {
    /// Required to open a directory handle with `CreateFileW`.
    pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    /// Open a reparse point (a symbolic link or junction) as itself rather
    /// than following it -- the closest Windows has to `O_NOFOLLOW`.
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
}

/// The real, OS-backed [`RunOutboxPreparer`]. Stateless: every call
/// re-validates the declaration against the filesystem fresh.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsRunOutboxPreparer;

impl FsRunOutboxPreparer {
    /// Build a preparer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Open the directory at `path` without following a link at that path,
/// refusing anything that is not a directory.
#[cfg(unix)]
pub(crate) fn open_directory_no_follow(path: &Path) -> std::io::Result<File> {
    use nix::fcntl::{OFlag, open};
    use nix::sys::stat::Mode;
    let fd = open(
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    Ok(File::from(fd))
}

/// Windows: open a directory handle at `path` without following a reparse
/// point there, then confirm from the handle's own metadata that it is a
/// directory and was not a reparse point.
#[cfg(windows)]
pub(crate) fn open_directory_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            win_flags::FILE_FLAG_BACKUP_SEMANTICS | win_flags::FILE_FLAG_OPEN_REPARSE_POINT,
        )
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::from(std::io::ErrorKind::NotADirectory));
    }
    Ok(file)
}

/// Open the subdirectory `name` of the open directory `dir` without
/// following a link at that name: relative to the handle on unix
/// (`openat`), by the joined path on Windows.
pub(crate) fn open_child_directory_no_follow(
    dir: &File,
    dir_path: &Path,
    name: &std::ffi::OsStr,
) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use nix::fcntl::{OFlag, openat};
        use nix::sys::stat::Mode;
        let _ = dir_path;
        let fd = openat(
            dir,
            Path::new(name),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        Ok(File::from(fd))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        open_directory_no_follow(&dir_path.join(name))
    }
}

/// Create the directory `name` under `outbox` exclusively, with owner-only
/// permissions: `mkdirat` relative to the outbox handle on unix (mode
/// `0o700`), `CreateDirectoryW` by path on Windows. Any existing name --
/// a file, a directory, a link of any kind -- is `AlreadyExists`.
fn create_exclusive(outbox: &OpenedOutbox, name: &str) -> Result<(), PrepareError> {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        use nix::sys::stat::{Mode, mkdirat};
        match mkdirat(outbox.handle.as_file(), Path::new(name), Mode::S_IRWXU) {
            Ok(()) => Ok(()),
            Err(Errno::EEXIST) => Err(PrepareError::AlreadyExists),
            Err(_) => Err(PrepareError::CreateFailed),
        }
    }
    #[cfg(not(unix))]
    {
        match std::fs::DirBuilder::new().create(outbox.path.join(name)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(PrepareError::AlreadyExists)
            }
            Err(_) => Err(PrepareError::CreateFailed),
        }
    }
}

/// Force owner-only permissions through the handle (`fchmod`), so a
/// permissive umask cannot widen what `mkdir` created; a no-op on
/// Windows, where the residual is disclosed in the module comment (hence
/// the always-`Ok` result there, allowed for that target only).
#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn set_owner_only(created: &File) -> Result<(), PrepareError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        created
            .set_permissions(std::fs::Permissions::from_mode(0o700))
            .map_err(|_| PrepareError::CreateFailed)
    }
    #[cfg(not(unix))]
    {
        let _ = created;
        Ok(())
    }
}

/// Verify by handle that `created` -- opened relative to the outbox -- is
/// the directory `declared_path` names: a directory, owner-only, and (on
/// unix) the same file identity as a fresh no-follow open of the exact
/// path the harness will be given.
fn verify_by_handle(created: &File, declared_path: &Path) -> Result<(), PrepareError> {
    let by_handle = created
        .metadata()
        .map_err(|_| PrepareError::Verification(VerificationFailure::NotADirectory))?;
    if !by_handle.is_dir() {
        return Err(PrepareError::Verification(
            VerificationFailure::NotADirectory,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if by_handle.permissions().mode() & 0o777 != 0o700 {
            return Err(PrepareError::Verification(
                VerificationFailure::NotOwnerOnly,
            ));
        }
        let by_path = open_directory_no_follow(declared_path)
            .and_then(|file| file.metadata())
            .map_err(|_| PrepareError::Verification(VerificationFailure::IdentityMismatch))?;
        if (by_path.dev(), by_path.ino()) != (by_handle.dev(), by_handle.ino()) {
            return Err(PrepareError::Verification(
                VerificationFailure::IdentityMismatch,
            ));
        }
    }
    #[cfg(not(unix))]
    {
        // The path must open as a real directory without following a
        // reparse point; its identity is not compared (see the module
        // comment's disclosed residual).
        open_directory_no_follow(declared_path)
            .map_err(|_| PrepareError::Verification(VerificationFailure::IdentityMismatch))?;
    }
    Ok(())
}

impl RunOutboxPreparer for FsRunOutboxPreparer {
    fn open_outbox(
        &self,
        project: &WorkspaceRoot,
        declared: &OutboxPath,
        create_if_missing: bool,
    ) -> Result<OpenedOutbox, PrepareError> {
        let canonical = match validate_outbox_declaration(project, declared)
            .map_err(PrepareError::Declaration)?
        {
            OutboxLocation::Present(canonical) => canonical,
            OutboxLocation::Missing(joined) => {
                if !create_if_missing {
                    return Err(PrepareError::OutboxMissing);
                }
                std::fs::create_dir_all(&joined).map_err(|_| PrepareError::OutboxUnopenable)?;
                // Re-validated after creation: the created directory must
                // itself pass every check before anything is created under
                // it.
                match validate_outbox_declaration(project, declared) {
                    Ok(OutboxLocation::Present(canonical)) => canonical,
                    Ok(OutboxLocation::Missing(_)) => return Err(PrepareError::OutboxUnopenable),
                    Err(error) => return Err(PrepareError::Declaration(error)),
                }
            }
        };
        let file =
            open_directory_no_follow(&canonical).map_err(|_| PrepareError::OutboxUnopenable)?;
        if !file
            .metadata()
            .map_err(|_| PrepareError::OutboxUnopenable)?
            .is_dir()
        {
            return Err(PrepareError::OutboxUnopenable);
        }
        Ok(OpenedOutbox {
            path: canonical,
            handle: DirectoryHandle::new(file),
        })
    }

    fn prepare(
        &self,
        project: &WorkspaceRoot,
        declared: &OutboxPath,
        run_id: &RunId,
    ) -> Result<PreparedRunSubdirectory, PrepareError> {
        let outbox = self.open_outbox(project, declared, true)?;
        create_exclusive(&outbox, run_id.as_str())?;
        let created = open_child_directory_no_follow(
            outbox.handle.as_file(),
            &outbox.path,
            std::ffi::OsStr::new(run_id.as_str()),
        )
        .map_err(|_| PrepareError::OpenFailed)?;
        set_owner_only(&created)?;
        let path = outbox.path.join(run_id.as_str());
        verify_by_handle(&created, &path)?;
        Ok(PreparedRunSubdirectory {
            run_id: run_id.clone(),
            path,
            handle: DirectoryHandle::new(created),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::open_directory_no_follow;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-run-outbox-preparer-unit-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The no-follow directory open refuses a symbolic link placed at the
    /// path, even one pointing at a real directory, and refuses a regular
    /// file -- the two swaps "verify by handle" exists to catch.
    #[test]
    fn refuses_a_link_and_a_file_at_the_path() {
        let dir = TempDir::new("no-follow");
        let real = dir.0.join("real");
        std::fs::create_dir(&real).expect("fixture");
        std::os::unix::fs::symlink(&real, dir.0.join("link")).expect("fixture symlink");
        std::fs::write(dir.0.join("file"), b"x").expect("fixture");

        assert!(open_directory_no_follow(&real).is_ok());
        assert!(open_directory_no_follow(&dir.0.join("link")).is_err());
        assert!(open_directory_no_follow(&dir.0.join("file")).is_err());
    }
}
