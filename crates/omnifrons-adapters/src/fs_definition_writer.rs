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
//! inspected with `symlink_metadata`, and the temporary file is created
//! with `O_EXCL`, which never follows a link.
//!
//! A create needs the hard link. Where the filesystem cannot make one
//! (FAT and exFAT, and some network or FUSE mounts, answer `EPERM`,
//! `ENOTSUP` or `EOPNOTSUPP` on Unix, `ERROR_NOT_SUPPORTED` or
//! `ERROR_INVALID_FUNCTION` on Windows) the create is refused with
//! `ExclusiveCreateUnsupported`. There is deliberately no fallback to a
//! plain create: it would not be atomic and could leave a partial file a
//! rerun cannot recognise. Replacing an existing file is a rename and is
//! unaffected.
//!
//! The target is re-fingerprinted immediately before publishing. A
//! rewrite landing between that check and a rename stays possible
//! (`rename(2)` cannot be made conditional through `std`); a create is
//! not exposed to this, since the link itself fails on an existing name
//! (where it can be made at all).

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
        write_with(root, request, &|| {}, &|| {})
    }
}

/// [`FsDefinitionWriter::write`], with two seams for tests: `after_temp`
/// runs once the temporary file exists, `before_publish` right before
/// the link or rename.
fn write_with(
    root: &WorkspaceRoot,
    request: &WriteRequest<'_>,
    after_temp: &dyn Fn(),
    before_publish: &dyn Fn(),
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
    let temp = dir.join(temp_name(name, hasher, sequence));
    let _guard = TempGuard(temp.clone());
    write_temp(&temp, request.bytes, mode).map_err(|_| io())?;
    after_temp();

    // The last check before publishing.
    verify(request.expected)?;
    before_publish();
    match request.expected {
        InputFingerprint::Absent => {
            std::fs::hard_link(&temp, &target).map_err(|e| match classify_link_error(&e) {
                LinkFailure::Exists => {
                    let found = inspect(&target, hasher, &io)
                        .map_or(InputFingerprint::Irregular, |found| found.fingerprint());
                    DefinitionWriteError::Stale {
                        path: path.to_string(),
                        expected: InputFingerprint::Absent,
                        found,
                    }
                }
                LinkFailure::Unsupported => DefinitionWriteError::ExclusiveCreateUnsupported {
                    path: path.to_string(),
                },
                LinkFailure::Other => io(),
            })?;
        }
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

/// How a failed hard link is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFailure {
    /// Something already sits at the name.
    Exists,
    /// The filesystem cannot make hard links.
    Unsupported,
    /// Any other failure.
    Other,
}

/// Reads a failed `hard_link`: an existing name, a filesystem without
/// hard links (by the OS's own code), or anything else.
fn classify_link_error(error: &std::io::Error) -> LinkFailure {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return LinkFailure::Exists;
    }
    if error.kind() == std::io::ErrorKind::Unsupported {
        return LinkFailure::Unsupported;
    }
    #[cfg(unix)]
    if let Some(raw) = error.raw_os_error() {
        use nix::errno::Errno;
        if [Errno::EPERM, Errno::ENOTSUP, Errno::EOPNOTSUPP].contains(&Errno::from_raw(raw)) {
            return LinkFailure::Unsupported;
        }
    }
    #[cfg(windows)]
    {
        /// `ERROR_INVALID_FUNCTION` and `ERROR_NOT_SUPPORTED`.
        const UNSUPPORTED: [i32; 2] = [1, 50];
        if error
            .raw_os_error()
            .is_some_and(|raw| UNSUPPORTED.contains(&raw))
        {
            return LinkFailure::Unsupported;
        }
    }
    LinkFailure::Other
}

#[cfg(test)]
mod tests {
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
            assert_eq!(
                classify_link_error(&error),
                LinkFailure::Unsupported,
                "{errno}"
            );
        }
        #[cfg(unix)]
        {
            let denied = std::io::Error::from_raw_os_error(nix::errno::Errno::EACCES as i32);
            assert_eq!(classify_link_error(&denied), LinkFailure::Other);
        }
        #[cfg(windows)]
        for code in [50, 1] {
            let error = std::io::Error::from_raw_os_error(code);
            assert_eq!(
                classify_link_error(&error),
                LinkFailure::Unsupported,
                "{code}"
            );
        }
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
        let temp = scratch.0.join("t.tmp");
        std::fs::write(&temp, b"partial").expect("temp");
        drop(TempGuard(temp.clone()));
        assert!(!temp.exists());
        // Dropping a guard over a file that is already gone is harmless.
        drop(TempGuard(temp));
    }

    #[test]
    fn the_guard_removes_its_file_when_a_later_step_fails() {
        let scratch = Scratch::new("guard-error");
        let temp = scratch.0.join("t.tmp");
        let attempt = |temp: &Path| -> Result<(), DefinitionWriteError> {
            let _guard = TempGuard(temp.to_path_buf());
            std::fs::write(temp, b"partial").expect("temp");
            Err(DefinitionWriteError::Io {
                path: "x".to_string(),
            })
        };
        attempt(&temp).expect_err("failed");
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
            &|| {
                temp_seen.set(scratch.entries().iter().any(|n| n.contains(".tmp")));
                std::fs::write(&target, b"theirs\n").expect("appears");
            },
            &|| {},
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
            &|| {},
            &|| std::fs::write(&target, b"theirs\n").expect("appears"),
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
            &|| std::fs::write(&target, b"edited\n").expect("edit"),
            &|| {},
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "stale");
        assert_eq!(std::fs::read(&target).expect("read"), b"edited\n");
        assert_eq!(scratch.entries(), vec!["AGENTS.md"]);
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
