//! `FsDefinitionWriter`: the real
//! `omnifrons_app::definition_writer::DefinitionWriter` (ADR-0005
//! sub-slice 2c) over the workspace's filesystem.
//!
//! A write is a temporary file created exclusively beside the target
//! (`.<name>.<pid>-<seq>.tmp`), filled, given its permission bits and
//! synced, then published: by a hard link for a create, which fails when
//! anything appeared at the name, or by a rename over the old file for a
//! replace. The directory is synced afterwards (best effort), the target
//! is read back through a no-follow open, and the temporary file is
//! always removed, on success and on failure alike. Nothing is written
//! through a link: every parent directory and the target itself are
//! inspected with `symlink_metadata`, and the temporary file is created
//! with `O_EXCL`, which never follows a link.
//!
//! The target is re-fingerprinted immediately before publishing. A
//! rewrite landing between that check and a rename stays possible
//! (`rename(2)` cannot be made conditional through `std`); a create is
//! not exposed to this, since the link itself fails on an existing name.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::definition_writer::{
    DefinitionWriteError, DefinitionWriter, ModePolicy, WriteRequest, validate_write_path,
};
use omnifrons_domain::definition_plan::InputFingerprint;
use omnifrons_domain::executable::Sha256Digest;

use crate::fs_project_text_file::{Opened, open_no_follow};
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

/// Removes the temporary file when dropped, whatever the outcome.
struct TempGuard(PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What is at a path, as the plan would fingerprint it.
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

/// Inspects `path` without following a link, hashing a regular file
/// through a no-follow open.
fn inspect(
    path: &Path,
    hasher: Sha2Hasher,
    error: &dyn Fn() -> DefinitionWriteError,
) -> Result<Found, DefinitionWriteError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Absent),
        Err(_) => return Err(error()),
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let target = std::fs::read_link(path).map_err(|_| error())?;
        return Ok(Found::Symlink(target.to_string_lossy().into_owned()));
    }
    if !file_type.is_file() {
        return Ok(Found::Irregular);
    }
    let mut file = match open_no_follow(path) {
        Opened::File(file) => file,
        Opened::Absent => return Ok(Found::Absent),
        Opened::NotAFile => return Ok(Found::Irregular),
        Opened::Unreadable => return Err(error()),
    };
    let (digest, _) = hasher.digest_reader(&mut file).map_err(|_| error())?;
    Ok(Found::Regular {
        digest,
        mode: mode_bits(&metadata),
    })
}

/// Checks that every directory between `root` and the target's parent
/// exists and is a real directory.
fn check_parents(
    root: &Path,
    components: &[&str],
    path: &str,
) -> Result<PathBuf, DefinitionWriteError> {
    let mut dir = root.to_path_buf();
    for component in &components[..components.len() - 1] {
        dir.push(component);
        match std::fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(DefinitionWriteError::NotRegular {
                    path: path.to_string(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(DefinitionWriteError::ParentMissing {
                    path: path.to_string(),
                });
            }
            Err(_) => {
                return Err(DefinitionWriteError::Io {
                    path: path.to_string(),
                });
            }
        }
    }
    Ok(dir)
}

/// The permission bits of the regular file at `source`.
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

/// The bits to publish: `None` leaves the platform default for a new
/// file. Mirrors the prototype's rule.
fn published_mode(
    root: &Path,
    policy: &ModePolicy,
    current: &Found,
) -> Result<Option<u32>, DefinitionWriteError> {
    let current_mode = match current {
        Found::Regular { mode, .. } => Some(*mode),
        _ => None,
    };
    match policy {
        ModePolicy::Keep => Ok(current_mode),
        ModePolicy::NoWiderThan { source } => {
            let source = source_mode(root, source)?;
            Ok(Some(current_mode.map_or(source, |mode| mode & source)))
        }
    }
}

/// Creates the temporary file exclusively and fills it.
fn write_temp(temp: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if mode.is_some() { 0o600 } else { 0o666 });
    }
    let mut file: File = options.open(temp)?;
    #[cfg(unix)]
    if let Some(bits) = mode {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(bits))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Best-effort durability for a link or rename inside `dir`.
fn sync_directory(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

impl DefinitionWriter for FsDefinitionWriter {
    fn write(
        &self,
        root: &WorkspaceRoot,
        request: &WriteRequest<'_>,
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
        let io = || DefinitionWriteError::Io {
            path: path.to_string(),
        };
        let hasher = Sha2Hasher::new();
        let dir = check_parents(root.path(), &components, path)?;
        let name = components[components.len() - 1];
        let target = dir.join(name);

        let verify = |expected: &InputFingerprint| -> Result<Found, DefinitionWriteError> {
            let found = inspect(&target, hasher, &io)?;
            if matches!(expected, InputFingerprint::Sha256(_))
                && matches!(found, Found::Symlink(_) | Found::Irregular)
            {
                return Err(DefinitionWriteError::NotRegular {
                    path: path.to_string(),
                });
            }
            if &found.fingerprint() != expected {
                return Err(DefinitionWriteError::Stale {
                    path: path.to_string(),
                    expected: expected.clone(),
                    found: found.fingerprint(),
                });
            }
            Ok(found)
        };

        let current = verify(request.expected)?;
        let mode = published_mode(root.path(), request.mode, &current)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".{name}.{}-{sequence}.tmp", std::process::id()));
        let _guard = TempGuard(temp.clone());
        write_temp(&temp, request.bytes, mode).map_err(|_| io())?;

        // The last check before publishing.
        verify(request.expected)?;
        match request.expected {
            InputFingerprint::Absent => std::fs::hard_link(&temp, &target).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    let found = inspect(&target, hasher, &io)
                        .map_or(InputFingerprint::Irregular, |found| found.fingerprint());
                    DefinitionWriteError::Stale {
                        path: path.to_string(),
                        expected: InputFingerprint::Absent,
                        found,
                    }
                } else {
                    io()
                }
            })?,
            _ => std::fs::rename(&temp, &target).map_err(|_| io())?,
        }
        sync_directory(&dir);

        let written = hasher.sha256(request.bytes);
        match inspect(&target, hasher, &io)? {
            Found::Regular { digest, .. } if digest == written => Ok(written),
            _ => Err(DefinitionWriteError::ReadBackMismatch {
                path: path.to_string(),
            }),
        }
    }
}
