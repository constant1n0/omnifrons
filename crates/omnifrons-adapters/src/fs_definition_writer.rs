//! `FsDefinitionWriter`: the real
//! `omnifrons_app::definition_writer::DefinitionWriter` (ADR-0005
//! sub-slice 2c) over the workspace's filesystem.
//!
//! A write is a temporary file created exclusively beside the target
//! (`.omnifrons-<pid>-<seq>-<8 hex of the name's hash>.tmp`, a name whose
//! length does not depend on the target's), filled, given its permission
//! bits and synced, then published: by a hard link for a create, which
//! fails when anything appeared at the name, or by a rename over the old
//! file for a replace. The directory is synced afterwards (best effort), the target
//! is read back through a no-follow open, and the temporary file is
//! always removed, on success and on failure alike. Nothing is written
//! through a link: every parent directory and the target itself are
//! inspected without following links, and the temporary file is created
//! with `O_EXCL`, which never follows a link.
//!
//! # Parent directories
//!
//! On Unix the parent directories are pinned. The workspace root is
//! opened with `O_DIRECTORY | O_NOFOLLOW`, and each parent component is
//! opened with `openat` relative to the handle before it, again without
//! following a link. The target check, the temporary file, the link or
//! rename, the read-back and the directory sync all go through that final
//! handle, by name. A parent swapped for a link, or renamed away, after
//! the check therefore cannot redirect the write: it lands in the
//! directory that was checked.
//!
//! On Windows, and on any other target without directory-relative calls,
//! the parents are checked by path and every later step resolves the path
//! again. A concurrent swap of a parent for a link between the check and
//! the publish is not prevented there.
//!
//! The permission bits to publish are read once to create the temporary
//! file and again immediately before publishing, so a target or source
//! narrowed in between is not replaced with the stale, wider bits.
//! (The `NoWiderThan` source is read through its path on every target.)
//!
//! # Exclusive create
//!
//! A create needs the hard link. When the link is refused for a reason
//! other than a name collision, the create fails with
//! `ExclusiveCreateRefused`: `EPERM`, `ENOTSUP`, `EOPNOTSUPP` or
//! `ErrorKind::Unsupported` on Unix, `ERROR_NOT_SUPPORTED` (50) on
//! Windows. The system does not say whether the filesystem has no hard
//! links (FAT and exFAT, some network and FUSE mounts) or the directory
//! refuses new links (an immutable or append-only directory, a
//! protected-hardlinks policy), so the error claims neither. There is
//! deliberately no fallback to a plain create: it would not be atomic and
//! could leave a partial file a rerun cannot recognise. Replacing an
//! existing file is a rename and is unaffected. Any other link failure is
//! `Io`.
//!
//! The target is re-fingerprinted immediately before publishing. A
//! rewrite landing between that check and a rename stays possible
//! (`rename(2)` cannot be made conditional through `std`); a create is
//! not exposed to this, since the link itself fails on an existing name
//! (where it can be made at all).

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::definition_writer::{
    DefinitionWriteError, DefinitionWriter, ModePolicy, WriteRequest, validate_write_path,
};
use omnifrons_domain::definition_plan::InputFingerprint;
use omnifrons_domain::executable::Sha256Digest;

use crate::fs_project_text_file::Opened;
use crate::sha2_hasher::Sha2Hasher;

/// The real, OS-backed [`DefinitionWriter`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsDefinitionWriter;

impl FsDefinitionWriter {
    /// Build the port.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// The process-wide sequence behind the temporary names.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// What kind of entry a name is, read without following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
    Symlink,
    Other,
}

/// An entry's kind and permission bits (`0` where the platform has none).
#[derive(Debug, Clone, Copy)]
struct Meta {
    kind: Kind,
    mode: u32,
}

/// Why a parent component could not be entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildError {
    Missing,
    NotADirectory,
    Other,
}

/// A parent directory the write works in. On Unix it is an open handle,
/// so every operation is relative to the directory that was checked; on
/// other targets it is the path, resolved again by each operation.
struct Dir {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    #[cfg(not(unix))]
    path: std::path::PathBuf,
}

/// Removes the temporary file when dropped, whatever the outcome.
struct TempGuard<'a> {
    dir: &'a Dir,
    name: String,
}

impl Drop for TempGuard<'_> {
    fn drop(&mut self) {
        let _ = self.dir.remove(&self.name);
    }
}

#[cfg(unix)]
impl Dir {
    /// Opens the workspace root itself, refusing a link at its name.
    fn open_root(path: &Path) -> std::io::Result<Self> {
        use nix::fcntl::{OFlag, open};
        use nix::sys::stat::Mode;
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        Ok(Self {
            fd: open(path, flags, Mode::empty())?,
        })
    }

    /// Opens the directory `name` inside this one, never through a link.
    fn child(&self, name: &str) -> Result<Self, ChildError> {
        use nix::errno::Errno;
        use nix::fcntl::{OFlag, openat};
        use nix::sys::stat::Mode;
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        match openat(&self.fd, name, flags, Mode::empty()) {
            Ok(fd) => Ok(Self { fd }),
            Err(Errno::ENOENT) => Err(ChildError::Missing),
            Err(Errno::ELOOP | Errno::ENOTDIR) => Err(ChildError::NotADirectory),
            Err(_) => Err(ChildError::Other),
        }
    }

    /// The entry `name`, without following a link.
    fn lstat(&self, name: &str) -> std::io::Result<Meta> {
        use nix::fcntl::AtFlags;
        use nix::sys::stat::{SFlag, fstatat};
        let stat = fstatat(&self.fd, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        let kind = match SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT {
            SFlag::S_IFREG => Kind::File,
            SFlag::S_IFDIR => Kind::Directory,
            SFlag::S_IFLNK => Kind::Symlink,
            _ => Kind::Other,
        };
        // `st_mode` is 16 bits on some targets and 32 on others.
        let mode = u32::try_from(u64::from(stat.st_mode) & 0o777).unwrap_or(0);
        Ok(Meta { kind, mode })
    }

    fn read_link(&self, name: &str) -> std::io::Result<String> {
        let target = nix::fcntl::readlinkat(&self.fd, name)?;
        Ok(target.to_string_lossy().into_owned())
    }

    /// Opens the regular file `name` for reading, never through a link; a
    /// FIFO opens without blocking and is refused by the caller's check.
    fn open_read(&self, name: &str) -> Opened {
        use nix::errno::Errno;
        use nix::fcntl::{OFlag, openat};
        use nix::sys::stat::Mode;
        let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK;
        match openat(&self.fd, name, flags, Mode::empty()) {
            Ok(fd) => Opened::File(File::from(fd)),
            Err(Errno::ENOENT) => Opened::Absent,
            Err(Errno::ELOOP | Errno::ENXIO | Errno::EOPNOTSUPP | Errno::ENOTDIR) => {
                Opened::NotAFile
            }
            Err(_) => Opened::Unreadable,
        }
    }

    /// Creates `name` exclusively, writable; owner-only when `private`.
    fn create_new(&self, name: &str, private: bool) -> std::io::Result<File> {
        use nix::fcntl::{OFlag, openat};
        use nix::sys::stat::Mode;
        let mode = if private {
            Mode::S_IRUSR | Mode::S_IWUSR
        } else {
            Mode::from_bits_truncate(0o666)
        };
        let flags = OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY | OFlag::O_CLOEXEC;
        Ok(File::from(openat(&self.fd, name, flags, mode)?))
    }

    fn link(&self, from: &str, to: &str) -> std::io::Result<()> {
        use nix::fcntl::AtFlags;
        Ok(nix::unistd::linkat(
            &self.fd,
            from,
            &self.fd,
            to,
            AtFlags::empty(),
        )?)
    }

    fn rename(&self, from: &str, to: &str) -> std::io::Result<()> {
        Ok(nix::fcntl::renameat(&self.fd, from, &self.fd, to)?)
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        use nix::unistd::{UnlinkatFlags, unlinkat};
        Ok(unlinkat(&self.fd, name, UnlinkatFlags::NoRemoveDir)?)
    }

    /// Best-effort durability for a link or rename inside this directory.
    fn sync(&self) {
        let _ = nix::unistd::fsync(&self.fd);
    }
}

#[cfg(not(unix))]
impl Dir {
    #[allow(
        clippy::unnecessary_wraps,
        reason = "the Unix counterpart opens a handle and can fail"
    )]
    fn open_root(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn child(&self, name: &str) -> Result<Self, ChildError> {
        let path = self.path.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(Self { path }),
            Ok(_) => Err(ChildError::NotADirectory),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ChildError::Missing),
            Err(_) => Err(ChildError::Other),
        }
    }

    fn lstat(&self, name: &str) -> std::io::Result<Meta> {
        let file_type = std::fs::symlink_metadata(self.path.join(name))?.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if file_type.is_dir() {
            Kind::Directory
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        Ok(Meta { kind, mode: 0 })
    }

    fn read_link(&self, name: &str) -> std::io::Result<String> {
        let target = std::fs::read_link(self.path.join(name))?;
        Ok(target.to_string_lossy().into_owned())
    }

    fn open_read(&self, name: &str) -> Opened {
        crate::fs_project_text_file::open_no_follow(&self.path.join(name))
    }

    fn create_new(&self, name: &str, _private: bool) -> std::io::Result<File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path.join(name))
    }

    fn link(&self, from: &str, to: &str) -> std::io::Result<()> {
        std::fs::hard_link(self.path.join(from), self.path.join(to))
    }

    fn rename(&self, from: &str, to: &str) -> std::io::Result<()> {
        std::fs::rename(self.path.join(from), self.path.join(to))
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        std::fs::remove_file(self.path.join(name))
    }

    fn sync(&self) {
        if let Ok(file) = File::open(&self.path) {
            let _ = file.sync_all();
        }
    }
}

/// What is at a name, as the plan would fingerprint it.
enum Found {
    Absent,
    Regular { digest: Sha256Digest, mode: u32 },
    Symlink(String),
    Irregular,
}

impl Found {
    fn fingerprint(&self) -> InputFingerprint {
        match self {
            Self::Absent => InputFingerprint::Absent,
            Self::Regular { digest, .. } => InputFingerprint::Sha256(*digest),
            Self::Symlink(target) => InputFingerprint::Symlink(target.clone()),
            Self::Irregular => InputFingerprint::Irregular,
        }
    }

    /// The permission bits of a regular file.
    const fn regular_mode(&self) -> Option<u32> {
        match self {
            Self::Regular { mode, .. } => Some(*mode),
            _ => None,
        }
    }
}

/// Inspects `name` in `dir` without following a link, hashing a regular
/// file through a no-follow open.
fn inspect(
    dir: &Dir,
    name: &str,
    hasher: Sha2Hasher,
    error: &dyn Fn() -> DefinitionWriteError,
) -> Result<Found, DefinitionWriteError> {
    let meta = match dir.lstat(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Absent),
        Err(_) => return Err(error()),
    };
    match meta.kind {
        Kind::Symlink => return Ok(Found::Symlink(dir.read_link(name).map_err(|_| error())?)),
        Kind::File => {}
        Kind::Directory | Kind::Other => return Ok(Found::Irregular),
    }
    let mut file = match dir.open_read(name) {
        Opened::File(file) => file,
        Opened::Absent => return Ok(Found::Absent),
        Opened::NotAFile => return Ok(Found::Irregular),
        Opened::Unreadable => return Err(error()),
    };
    let (digest, _) = hasher.digest_reader(&mut file).map_err(|_| error())?;
    Ok(Found::Regular {
        digest,
        mode: meta.mode,
    })
}

/// Checks that every directory between `root` and the target's parent
/// exists and is a real directory, and returns the parent: a pinned
/// handle on Unix, the path elsewhere.
fn check_parents(
    root: &Path,
    components: &[&str],
    path: &str,
) -> Result<Dir, DefinitionWriteError> {
    let io = || DefinitionWriteError::Io {
        path: path.to_string(),
    };
    let mut dir = Dir::open_root(root).map_err(|_| io())?;
    for component in &components[..components.len() - 1] {
        dir = dir.child(component).map_err(|e| match e {
            ChildError::Missing => DefinitionWriteError::ParentMissing {
                path: path.to_string(),
            },
            ChildError::NotADirectory => DefinitionWriteError::NotRegular {
                path: path.to_string(),
            },
            ChildError::Other => io(),
        })?;
    }
    Ok(dir)
}

/// The permission bits of the regular file at `source`, read by path.
fn source_mode(root: &Path, source: &str) -> Result<u32, DefinitionWriteError> {
    let unavailable = || DefinitionWriteError::SourceUnavailable {
        path: source.to_string(),
    };
    let components = validate_write_path(source).map_err(|_| unavailable())?;
    let mut path = root.to_path_buf();
    path.extend(components);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(mode_bits(&metadata)),
        _ => Err(unavailable()),
    }
}

/// The permission bits of `metadata`; `0` where the platform has none.
#[cfg(unix)]
fn mode_bits(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_bits(_metadata: &std::fs::Metadata) -> u32 {
    0
}

/// The bits to publish given the mode of the file being replaced (`None`
/// for a create): `None` leaves the platform default for a new file.
/// Mirrors the prototype's rule.
fn published_mode(
    root: &Path,
    policy: &ModePolicy,
    current_mode: Option<u32>,
) -> Result<Option<u32>, DefinitionWriteError> {
    match policy {
        ModePolicy::Keep => Ok(current_mode),
        ModePolicy::NoWiderThan { source } => {
            let source = source_mode(root, source)?;
            Ok(Some(current_mode.map_or(source, |mode| mode & source)))
        }
    }
}

/// Gives `file` the permission bits `bits` (a no-op where the platform
/// has none).
#[cfg_attr(
    not(unix),
    allow(
        clippy::unnecessary_wraps,
        reason = "the Unix variant changes the permissions and can fail"
    )
)]
fn apply_mode(file: &File, bits: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(bits))
    }
    #[cfg(not(unix))]
    {
        let _ = (file, bits);
        Ok(())
    }
}

/// Creates the temporary file exclusively and fills it; the returned
/// handle stays open so the bits can be corrected before publishing.
fn write_temp(dir: &Dir, temp: &str, bytes: &[u8], mode: Option<u32>) -> std::io::Result<File> {
    let mut file = dir.create_new(temp, mode.is_some())?;
    if let Some(bits) = mode {
        apply_mode(&file, bits)?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(file)
}

/// The temporary file's name beside a target called `name`: bounded
/// whatever the target's length, and unique per process and write.
fn temp_name(name: &str, hasher: Sha2Hasher, sequence: u64) -> String {
    let tag = hasher.sha256(name.as_bytes()).to_hex();
    format!(
        ".omnifrons-{}-{sequence}-{}.tmp",
        std::process::id(),
        &tag[..8]
    )
}

impl DefinitionWriter for FsDefinitionWriter {
    fn write(
        &self,
        root: &WorkspaceRoot,
        request: &WriteRequest<'_>,
    ) -> Result<Sha256Digest, DefinitionWriteError> {
        write_with(root, request, &Seams::real())
    }
}

/// Publishes `(directory, temporary name, target name)`.
type PublishStep<'a> = &'a dyn Fn(&Dir, &str, &str) -> std::io::Result<()>;

/// The points where a test can step into a write. The production path
/// fills them with no-ops and the real link and rename.
struct Seams<'a> {
    /// Runs once the temporary file exists.
    after_temp: &'a dyn Fn(),
    /// Runs right before the modes are re-read and the link or rename.
    before_publish: &'a dyn Fn(),
    /// Publishes a create.
    link: PublishStep<'a>,
    /// Publishes a replace.
    rename: PublishStep<'a>,
}

impl Seams<'_> {
    fn real() -> Seams<'static> {
        Seams {
            after_temp: &|| {},
            before_publish: &|| {},
            link: &|dir, temp, target| dir.link(temp, target),
            rename: &|dir, temp, target| dir.rename(temp, target),
        }
    }
}

/// The target of one write: the checked parent, the name inside it, and
/// the request's path for error reports.
struct Target<'a> {
    dir: &'a Dir,
    name: &'a str,
    path: &'a str,
    hasher: Sha2Hasher,
}

impl Target<'_> {
    fn io(&self) -> DefinitionWriteError {
        DefinitionWriteError::Io {
            path: self.path.to_string(),
        }
    }

    /// Inspects the target and checks it against what the plan expected.
    fn verify(&self, expected: &InputFingerprint) -> Result<Found, DefinitionWriteError> {
        let found = inspect(self.dir, self.name, self.hasher, &|| self.io())?;
        if matches!(expected, InputFingerprint::Sha256(_))
            && matches!(found, Found::Symlink(_) | Found::Irregular)
        {
            return Err(DefinitionWriteError::NotRegular {
                path: self.path.to_string(),
            });
        }
        if &found.fingerprint() != expected {
            return Err(DefinitionWriteError::Stale {
                path: self.path.to_string(),
                expected: expected.clone(),
                found: found.fingerprint(),
            });
        }
        Ok(found)
    }

    /// Re-reads the modes that decide the published bits and gives the
    /// temporary file the final value when it differs from `initial`, the
    /// one it was created with.
    fn refresh_mode(
        &self,
        file: &File,
        request: &WriteRequest<'_>,
        root: &Path,
        initial: Option<u32>,
    ) -> Result<(), DefinitionWriteError> {
        let current = match request.expected {
            InputFingerprint::Sha256(_) => self
                .dir
                .lstat(self.name)
                .ok()
                .filter(|meta| meta.kind == Kind::File)
                .map(|meta| meta.mode),
            _ => None,
        };
        let latest = published_mode(root, request.mode, current)?;
        if let Some(bits) = latest.filter(|_| latest != initial) {
            apply_mode(file, bits)
                .and_then(|()| file.sync_all())
                .map_err(|_| self.io())?;
        }
        Ok(())
    }

    /// Links (a create) or renames (a replace) the temporary file into
    /// place.
    fn publish(
        &self,
        seams: &Seams<'_>,
        expected: &InputFingerprint,
        temp: &str,
    ) -> Result<(), DefinitionWriteError> {
        if !matches!(expected, InputFingerprint::Absent) {
            return (seams.rename)(self.dir, temp, self.name).map_err(|_| self.io());
        }
        (seams.link)(self.dir, temp, self.name).map_err(|e| match classify_link_error(&e) {
            LinkFailure::Exists => {
                let found = inspect(self.dir, self.name, self.hasher, &|| self.io())
                    .map_or(InputFingerprint::Irregular, |found| found.fingerprint());
                DefinitionWriteError::Stale {
                    path: self.path.to_string(),
                    expected: InputFingerprint::Absent,
                    found,
                }
            }
            LinkFailure::Refused => DefinitionWriteError::ExclusiveCreateRefused {
                path: self.path.to_string(),
            },
            LinkFailure::Other => self.io(),
        })
    }

    /// Reads the published file back and compares it to what was written.
    fn read_back(&self, bytes: &[u8]) -> Result<Sha256Digest, DefinitionWriteError> {
        let written = self.hasher.sha256(bytes);
        match inspect(self.dir, self.name, self.hasher, &|| self.io())? {
            Found::Regular { digest, .. } if digest == written => Ok(written),
            _ => Err(DefinitionWriteError::ReadBackMismatch {
                path: self.path.to_string(),
            }),
        }
    }
}

/// [`FsDefinitionWriter::write`], stepping through `seams`.
fn write_with(
    root: &WorkspaceRoot,
    request: &WriteRequest<'_>,
    seams: &Seams<'_>,
) -> Result<Sha256Digest, DefinitionWriteError> {
    let path = request.path;
    let components = validate_write_path(path)?;
    if matches!(
        request.expected,
        InputFingerprint::Symlink(_) | InputFingerprint::Irregular
    ) {
        return Err(DefinitionWriteError::NotWritable {
            path: path.to_string(),
        });
    }
    let hasher = Sha2Hasher::new();
    let dir = check_parents(root.path(), &components, path)?;
    let name = components[components.len() - 1];
    let target = Target {
        dir: &dir,
        name,
        path,
        hasher,
    };

    let current = target.verify(request.expected)?;
    let mode = published_mode(root.path(), request.mode, current.regular_mode())?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = temp_name(name, hasher, sequence);
    let _guard = TempGuard {
        dir: &dir,
        name: temp.clone(),
    };
    let file = write_temp(&dir, &temp, request.bytes, mode).map_err(|_| target.io())?;
    (seams.after_temp)();

    // The last checks before publishing: the content, then the modes.
    target.verify(request.expected)?;
    (seams.before_publish)();
    target.refresh_mode(&file, request, root.path(), mode)?;
    target.publish(seams, request.expected, &temp)?;
    dir.sync();
    target.read_back(request.bytes)
}

/// How a failed hard link is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFailure {
    /// Something already sits at the name.
    Exists,
    /// The link was refused: the filesystem has no hard links or the
    /// directory refuses them; the error cannot say which.
    Refused,
    /// Any other failure.
    Other,
}

/// Reads a failed `hard_link`: an existing name, a refusal the system
/// reports with a code that covers both "no hard links here" and "this
/// directory refuses links" (`EPERM` is both for `link(2)`), or anything
/// else. Windows' `ERROR_INVALID_FUNCTION` is a generic failure and is
/// not read as a refusal.
fn classify_link_error(error: &std::io::Error) -> LinkFailure {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return LinkFailure::Exists;
    }
    if error.kind() == std::io::ErrorKind::Unsupported {
        return LinkFailure::Refused;
    }
    #[cfg(unix)]
    if let Some(raw) = error.raw_os_error() {
        use nix::errno::Errno;
        if [Errno::EPERM, Errno::ENOTSUP, Errno::EOPNOTSUPP].contains(&Errno::from_raw(raw)) {
            return LinkFailure::Refused;
        }
    }
    #[cfg(windows)]
    {
        /// `ERROR_NOT_SUPPORTED`.
        const NOT_SUPPORTED: i32 = 50;
        if error.raw_os_error() == Some(NOT_SUPPORTED) {
            return LinkFailure::Refused;
        }
    }
    LinkFailure::Other
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn link_errors_are_classified_by_kind_and_raw_code() {
        let exists = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert_eq!(classify_link_error(&exists), LinkFailure::Exists);
        let other = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(classify_link_error(&other), LinkFailure::Other);
        #[cfg(unix)]
        for errno in [
            nix::errno::Errno::EPERM,
            nix::errno::Errno::ENOTSUP,
            nix::errno::Errno::EOPNOTSUPP,
        ] {
            let error = std::io::Error::from_raw_os_error(errno as i32);
            assert_eq!(classify_link_error(&error), LinkFailure::Refused, "{errno}");
        }
        #[cfg(unix)]
        {
            let denied = std::io::Error::from_raw_os_error(nix::errno::Errno::EACCES as i32);
            assert_eq!(classify_link_error(&denied), LinkFailure::Other);
        }
        #[cfg(windows)]
        {
            let not_supported = std::io::Error::from_raw_os_error(50);
            assert_eq!(classify_link_error(&not_supported), LinkFailure::Refused);
            // ERROR_INVALID_FUNCTION is a generic failure, not a link refusal.
            let invalid_function = std::io::Error::from_raw_os_error(1);
            assert_eq!(classify_link_error(&invalid_function), LinkFailure::Other);
        }
        let unsupported = std::io::Error::from(std::io::ErrorKind::Unsupported);
        assert_eq!(classify_link_error(&unsupported), LinkFailure::Refused);
    }

    /// A scratch directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-fs-definition-writer-unit-{}-{label}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn root(&self) -> WorkspaceRoot {
            WorkspaceRoot::new(&self.0).expect("valid workspace")
        }

        fn entries(&self) -> Vec<String> {
            let mut found: Vec<String> = std::fs::read_dir(&self.0)
                .expect("list")
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect();
            found.sort();
            found
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn request<'a>(
        path: &'a str,
        bytes: &'a [u8],
        expected: &'a InputFingerprint,
    ) -> WriteRequest<'a> {
        WriteRequest {
            path,
            bytes,
            expected,
            mode: &ModePolicy::Keep,
        }
    }

    #[test]
    fn the_guard_removes_its_file_when_dropped() {
        let scratch = Scratch::new("guard");
        let dir = Dir::open_root(&scratch.0).expect("dir");
        let temp = scratch.0.join("t.tmp");
        std::fs::write(&temp, b"partial").expect("temp");
        drop(TempGuard {
            dir: &dir,
            name: "t.tmp".to_string(),
        });
        assert!(!temp.exists());
        // Dropping a guard over a file that is already gone is harmless.
        drop(TempGuard {
            dir: &dir,
            name: "t.tmp".to_string(),
        });
    }

    #[test]
    fn the_guard_removes_its_file_when_a_later_step_fails() {
        let scratch = Scratch::new("guard-error");
        let dir = Dir::open_root(&scratch.0).expect("dir");
        let temp = scratch.0.join("t.tmp");
        let attempt = || -> Result<(), DefinitionWriteError> {
            let _guard = TempGuard {
                dir: &dir,
                name: "t.tmp".to_string(),
            };
            std::fs::write(&temp, b"partial").expect("temp");
            Err(DefinitionWriteError::Io {
                path: "x".to_string(),
            })
        };
        attempt().expect_err("failed");
        assert!(!temp.exists());
    }

    #[test]
    fn a_target_appearing_after_the_temp_file_is_stale_and_leaves_no_temp() {
        let scratch = Scratch::new("after-temp");
        let target = scratch.0.join("AGENTS.md");
        let temp_seen = std::cell::Cell::new(false);
        let error = write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &InputFingerprint::Absent),
            &Seams {
                after_temp: &|| {
                    temp_seen.set(scratch.entries().iter().any(|n| n.contains(".tmp")));
                    std::fs::write(&target, b"theirs\n").expect("appears");
                },
                ..Seams::real()
            },
        )
        .expect_err("refused");
        assert!(temp_seen.get(), "the temp file existed at the seam");
        assert_eq!(error.reason(), "stale");
        assert_eq!(std::fs::read(&target).expect("read"), b"theirs\n");
        assert_eq!(scratch.entries(), vec!["AGENTS.md"]);
    }

    #[test]
    fn a_target_appearing_just_before_the_link_maps_to_stale_and_leaves_no_temp() {
        let scratch = Scratch::new("before-publish");
        let target = scratch.0.join("AGENTS.md");
        let error = write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &InputFingerprint::Absent),
            &Seams {
                before_publish: &|| std::fs::write(&target, b"theirs\n").expect("appears"),
                ..Seams::real()
            },
        )
        .expect_err("refused");
        let DefinitionWriteError::Stale { found, .. } = &error else {
            panic!("expected stale, got {error:?}");
        };
        assert_eq!(
            found,
            &InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"theirs\n"))
        );
        assert_eq!(std::fs::read(&target).expect("read"), b"theirs\n");
        assert_eq!(scratch.entries(), vec!["AGENTS.md"]);
    }

    #[test]
    fn a_replace_changed_after_the_temp_file_is_stale_and_leaves_no_temp() {
        let scratch = Scratch::new("replace-after-temp");
        let target = scratch.0.join("AGENTS.md");
        std::fs::write(&target, b"planned\n").expect("seed");
        let planned = InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"planned\n"));
        let error = write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &planned),
            &Seams {
                after_temp: &|| std::fs::write(&target, b"edited\n").expect("edit"),
                ..Seams::real()
            },
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "stale");
        assert_eq!(std::fs::read(&target).expect("read"), b"edited\n");
        assert_eq!(scratch.entries(), vec!["AGENTS.md"]);
    }

    /// A create whose link is made to fail with `error`.
    fn create_with_failing_link(
        scratch: &Scratch,
        error: fn() -> std::io::Error,
    ) -> DefinitionWriteError {
        write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &InputFingerprint::Absent),
            &Seams {
                link: &|_, _, _| Err(error()),
                // A fallback to a plain create would go through rename.
                rename: &|_, _, _| panic!("a create must never fall back to a rename"),
                ..Seams::real()
            },
        )
        .expect_err("refused")
    }

    #[cfg(unix)]
    #[test]
    fn a_link_the_system_refuses_is_a_refused_exclusive_create_and_leaves_nothing() {
        let refusals: [fn() -> std::io::Error; 2] = [
            || std::io::Error::from_raw_os_error(nix::errno::Errno::EPERM as i32),
            || std::io::Error::from_raw_os_error(nix::errno::Errno::ENOTSUP as i32),
        ];
        for refusal in refusals {
            let scratch = Scratch::new("link-refused");
            let error = create_with_failing_link(&scratch, refusal);
            assert_eq!(error.reason(), "exclusive-create-refused");
            assert!(scratch.entries().is_empty(), "{:?}", scratch.entries());
        }
    }

    #[test]
    fn a_generic_link_failure_is_io_and_leaves_nothing() {
        let scratch = Scratch::new("link-generic");
        let error = create_with_failing_link(&scratch, || {
            std::io::Error::from(std::io::ErrorKind::PermissionDenied)
        });
        assert_eq!(error.reason(), "io");
        assert!(scratch.entries().is_empty(), "{:?}", scratch.entries());
    }

    #[test]
    fn a_link_that_reports_an_existing_name_is_stale_and_leaves_the_name_alone() {
        let scratch = Scratch::new("link-exists");
        let target = scratch.0.join("AGENTS.md");
        let error = write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &InputFingerprint::Absent),
            &Seams {
                link: &|_, _, _| {
                    std::fs::write(&target, b"theirs\n").expect("appears");
                    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
                },
                ..Seams::real()
            },
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "stale");
        assert_eq!(scratch.entries(), vec!["AGENTS.md"]);
    }

    #[test]
    fn a_replace_publishes_through_rename_never_a_link() {
        let scratch = Scratch::new("replace-rename");
        let target = scratch.0.join("AGENTS.md");
        std::fs::write(&target, b"planned\n").expect("seed");
        let planned = InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"planned\n"));
        write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &planned),
            &Seams {
                link: &|_, _, _| panic!("a replace must not link"),
                ..Seams::real()
            },
        )
        .expect("replaced");
        assert_eq!(std::fs::read(&target).expect("read"), b"mine\n");
    }

    #[cfg(unix)]
    fn chmod(path: &Path, bits: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).expect("chmod");
    }

    #[cfg(unix)]
    fn bits(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn a_target_narrowed_before_publication_is_replaced_with_the_narrowed_mode() {
        let scratch = Scratch::new("keep-narrowed");
        let target = scratch.0.join("AGENTS.md");
        std::fs::write(&target, b"planned\n").expect("seed");
        chmod(&target, 0o644);
        let planned = InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"planned\n"));
        write_with(
            &scratch.root(),
            &request("AGENTS.md", b"mine\n", &planned),
            &Seams {
                before_publish: &|| chmod(&target, 0o600),
                ..Seams::real()
            },
        )
        .expect("replaced");
        assert_eq!(std::fs::read(&target).expect("read"), b"mine\n");
        assert_eq!(bits(&target), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_source_narrowed_before_publication_narrows_the_published_mode() {
        let scratch = Scratch::new("source-narrowed");
        let target = scratch.0.join("AGENTS.md");
        let source = scratch.0.join("CLAUDE.md");
        std::fs::write(&target, b"planned\n").expect("seed");
        std::fs::write(&source, b"source\n").expect("source");
        chmod(&target, 0o644);
        chmod(&source, 0o644);
        let planned = InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"planned\n"));
        write_with(
            &scratch.root(),
            &WriteRequest {
                path: "AGENTS.md",
                bytes: b"mine\n",
                expected: &planned,
                mode: &ModePolicy::NoWiderThan {
                    source: "CLAUDE.md".to_string(),
                },
            },
            &Seams {
                before_publish: &|| chmod(&source, 0o600),
                ..Seams::real()
            },
        )
        .expect("replaced");
        assert_eq!(bits(&target), 0o600);
    }

    /// Swaps `sub` for a symlink to `elsewhere`, keeping the real
    /// directory as `sub.moved`, while a write into `sub/AGENTS.md` is
    /// paused just before it publishes.
    #[cfg(unix)]
    fn write_while_parent_is_swapped(scratch: &Scratch, expected: &InputFingerprint) {
        let elsewhere = scratch.0.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let swap = || {
            std::fs::rename(scratch.0.join("sub"), scratch.0.join("sub.moved")).expect("move");
            std::os::unix::fs::symlink(&elsewhere, scratch.0.join("sub")).expect("link");
        };
        write_with(
            &scratch.root(),
            &request("sub/AGENTS.md", b"mine\n", expected),
            &Seams {
                before_publish: &swap,
                ..Seams::real()
            },
        )
        .expect("written into the pinned directory");
        assert!(
            std::fs::read_dir(&elsewhere)
                .expect("list")
                .next()
                .is_none(),
            "nothing may appear through the swapped link"
        );
        let moved = scratch.0.join("sub.moved");
        assert_eq!(
            std::fs::read(moved.join("AGENTS.md")).expect("read"),
            b"mine\n"
        );
        let left: Vec<String> = std::fs::read_dir(&moved)
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["AGENTS.md"], "no temporary file is left behind");
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_swapped_for_a_link_before_a_create_still_receives_the_file() {
        let scratch = Scratch::new("swap-create");
        std::fs::create_dir_all(scratch.0.join("sub")).expect("sub");
        write_while_parent_is_swapped(&scratch, &InputFingerprint::Absent);
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_swapped_for_a_link_before_a_replace_still_receives_the_file() {
        let scratch = Scratch::new("swap-replace");
        std::fs::create_dir_all(scratch.0.join("sub")).expect("sub");
        std::fs::write(scratch.0.join("sub/AGENTS.md"), b"planned\n").expect("seed");
        let planned = InputFingerprint::Sha256(Sha2Hasher::new().sha256(b"planned\n"));
        write_while_parent_is_swapped(&scratch, &planned);
    }

    #[test]
    fn the_temp_name_is_bounded_whatever_the_target_name() {
        let hasher = Sha2Hasher::new();
        let short = temp_name("a", hasher, 7);
        let long = temp_name(&"n".repeat(250), hasher, 7);
        assert_eq!(short.len(), long.len());
        assert!(long.len() < 64, "{long}");
        assert_ne!(short, long);
        assert!(long.starts_with(".omnifrons-"));
        assert_eq!(
            Path::new(&long).extension().and_then(|e| e.to_str()),
            Some("tmp")
        );
    }
}
