//! `FsExecutableProber`: the real, filesystem-backed
//! `omnifrons_app::ExecutableProber` adapter.
//!
//! Never echoes a path in an error: every failure reduces to one of
//! [`ProbeOutcome`]'s closed variants, none of which carry the underlying
//! `io::Error`'s own text (which, for a "no such file" error, contains the
//! attempted path).
//!
//! Canonicalizes, then opens the canonical path *exactly once* -- unix:
//! with `O_NOFOLLOW`, so a symlink dropped at that exact path in the
//! narrow window after canonicalization is refused rather than silently
//! followed a second time -- and every subsequent check (regular-file,
//! executable-bit, size cap) and the content hash itself all run against
//! *that one* open file description (`fstat`, then a stream read),
//! never a second, independent open or stat by path. The probe result
//! carries that same handle onward (`omnifrons_app::ProbedExecutable`),
//! so a caller never has to re-open the path a second time to launch what
//! was just vetted.

use std::fs::{self, File};
use std::io::Read as _;
use std::path::Path;

use omnifrons_app::{ExecHandle, ExecutableProber, ProbeOutcome, ProbedExecutable};
use omnifrons_domain::executable::{ExecutableIdentity, PlatformEvidence, Sha256Digest};
use sha2::{Digest, Sha256};

/// The maximum size this prober will hash. A candidate strictly larger is
/// [`ProbeOutcome::TooLarge`]; a candidate at exactly this size is
/// accepted.
const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;

/// The buffer size used to stream-hash a candidate's content, so probing a
/// large file never buffers it into memory whole.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// The real, OS-backed [`ExecutableProber`]: canonicalizes the candidate,
/// opens the canonical path exactly once, confirms *that open file* is a
/// regular, executable file within the size cap, then stream-hashes its
/// content from the same handle.
///
/// Stateless: every [`Self::probe`] call re-reads the filesystem fresh, so
/// one instance may be reused, and shared behind `&self`, freely --
/// exactly what [`omnifrons_app::launch_gate::LaunchGate`] needs to
/// re-probe on every `decide` call, never from a cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsExecutableProber;

impl FsExecutableProber {
    /// Build a prober.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ExecutableProber for FsExecutableProber {
    fn probe(&self, candidate: &Path) -> ProbeOutcome {
        let Ok(canonical_path) = fs::canonicalize(candidate) else {
            return ProbeOutcome::Unreadable;
        };

        let Ok(file) = open_checked(&canonical_path) else {
            return ProbeOutcome::Unreadable;
        };

        // `fstat` on the handle we just opened -- not a second `stat`/
        // `lstat` by path -- so every check below (and the hash itself)
        // is about the exact file description `file` names, immune to
        // whatever the path resolves to from this point on.
        let Ok(metadata) = file.metadata() else {
            return ProbeOutcome::Unreadable;
        };

        if !metadata.is_file() {
            return ProbeOutcome::NotRegularFile;
        }

        let platform = match platform_evidence(&canonical_path, &metadata) {
            Ok(evidence) => evidence,
            Err(outcome) => return outcome,
        };

        let size = metadata.len();
        if size > MAX_EXECUTABLE_BYTES {
            return ProbeOutcome::TooLarge;
        }

        let modified_at = metadata.modified().ok();

        build_identity(file, canonical_path, size, modified_at, platform)
    }
}

/// Open `candidate` (already canonicalized by the caller) for hashing,
/// exactly once.
///
/// Unix: `O_NOFOLLOW` refuses a symlink placed at this exact path in the
/// narrow window between the caller's `canonicalize` and this call --
/// canonicalization already resolved every symlink component once, so
/// any further indirection here is, by construction, a race, never a
/// legitimate symlink this probe should follow a second time.
///
/// Windows has no equivalent flag exposed by `std`'s `OpenOptionsExt`
/// here, so on Windows this is a plain open: a symlink (or, more
/// commonly on that platform, a reparse point) swapped in during that
/// same narrow window would still be followed -- a documented, real
/// limitation (`docs/spike-log.md` § Slice 2's deferred list), not an
/// oversight.
fn open_checked(candidate: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // `nix::fcntl::OFlag::O_NOFOLLOW` rather than a raw `libc`
        // constant: this crate already depends on `nix` for the Linux
        // memfd-sealing path below, so the same crate names this flag
        // too, one fewer place to trust a bare integer is right.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
            .open(candidate)
    }
    #[cfg(not(unix))]
    {
        File::open(candidate)
    }
}

#[cfg(unix)]
fn platform_evidence(
    _canonical_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<PlatformEvidence, ProbeOutcome> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(ProbeOutcome::NotExecutable);
    }
    Ok(PlatformEvidence::Unix { mode })
}

/// Provisional: Windows executability is decided by extension allowlist
/// alone here, not a real ACL or authenticode signature check
/// (`docs/spike-log.md` § Slice 2). The allowlist check itself is a pure,
/// cross-platform-testable function ([`is_windows_executable_extension`]).
#[cfg(windows)]
fn platform_evidence(
    canonical_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<PlatformEvidence, ProbeOutcome> {
    use std::os::windows::fs::MetadataExt;
    let extension = canonical_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if !is_windows_executable_extension(&extension) {
        return Err(ProbeOutcome::NotExecutable);
    }
    Ok(PlatformEvidence::Windows {
        extension,
        attributes: metadata.file_attributes(),
    })
}

/// Whether `extension` (already lowercased, no leading dot) is on the
/// Windows executable-extension allowlist.
///
/// A pure function with no filesystem or platform dependency, so its own
/// unit tests (below) run on every platform this workspace tests on, not
/// only Windows -- the provisional allowlist itself (`exe`/`com`/`bat`/
/// `cmd`) is worth testing everywhere it is *read*, even though it is
/// only ever *consulted* by [`platform_evidence`] on a real Windows
/// build.
#[must_use]
pub fn is_windows_executable_extension(extension: &str) -> bool {
    const EXECUTABLE_EXTENSIONS: [&str; 4] = ["exe", "com", "bat", "cmd"];
    EXECUTABLE_EXTENSIONS.contains(&extension)
}

/// Stream-hash `file`'s content in [`HASH_BUFFER_BYTES`] chunks, then
/// wrap the result into [`ProbeOutcome::Identity`], carrying `file`
/// onward as the returned [`ProbedExecutable`]'s handle.
///
/// On Linux, the hash is computed *while copying* the bytes into a
/// sealed `memfd` ([`hash_into_sealed_memfd`]), so the returned handle's
/// content is provably exactly the bytes that were hashed, immutable
/// from that point on -- including against this very process. On every
/// other platform, `file` itself (still open, still referring to the
/// inode this probe just examined) is the returned handle; no seal is
/// available there, so nothing prevents that same inode's content from
/// being modified in place after this call returns
/// (`docs/spike-log.md` § Slice 2's deferred list).
fn build_identity(
    file: File,
    canonical_path: std::path::PathBuf,
    size: u64,
    modified_at: Option<std::time::SystemTime>,
    platform: PlatformEvidence,
) -> ProbeOutcome {
    #[cfg(target_os = "linux")]
    {
        let Ok((sha256, handle)) = hash_into_sealed_memfd(file) else {
            return ProbeOutcome::Unreadable;
        };
        ProbeOutcome::Identity(ProbedExecutable {
            identity: ExecutableIdentity {
                canonical_path,
                size,
                sha256,
                modified_at,
                platform,
            },
            handle,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let Ok(sha256) = hash_file(&file) else {
            return ProbeOutcome::Unreadable;
        };
        ProbeOutcome::Identity(ProbedExecutable {
            identity: ExecutableIdentity {
                canonical_path,
                size,
                sha256,
                modified_at,
                platform,
            },
            handle: ExecHandle::File(file),
        })
    }
}

/// Stream-hash an already-open file's content in [`HASH_BUFFER_BYTES`]
/// chunks. Reads via `&File` (not `File`), so the caller keeps ownership
/// of the handle to carry onward as [`ExecHandle::File`].
#[cfg(not(target_os = "linux"))]
fn hash_file(mut file: &File) -> std::io::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(Sha256Digest(digest))
}

/// Linux only: stream-hash `source`'s content while copying it into a
/// fresh, sealed `memfd`, returning both the digest and the sealed
/// handle.
///
/// The `memfd` is created with `MFD_CLOEXEC` (never inherited across an
/// unrelated `exec` before the supervisor deliberately clears that flag
/// at launch time) and `MFD_ALLOW_SEALING`. Once every byte of `source`
/// has been copied and hashed, `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE
/// | F_SEAL_SEAL` is applied: the sealed content is, from that instant on,
/// exactly the bytes that were hashed -- no further write, truncate, or
/// grow is possible against this file description by *any* process,
/// including this one (a later `write` against it fails with `EPERM`).
///
/// # Errors
///
/// Returns any `io::Error` reading `source`, creating the `memfd`,
/// writing into it, or applying the seal can produce.
#[cfg(target_os = "linux")]
fn hash_into_sealed_memfd(mut source: File) -> std::io::Result<(Sha256Digest, ExecHandle)> {
    use std::io::Write as _;

    use nix::fcntl::{FcntlArg, SealFlag, fcntl};
    use nix::sys::memfd::{MFdFlags, memfd_create};

    let memfd = memfd_create(
        "omnifrons-approved-executable",
        MFdFlags::MFD_CLOEXEC | MFdFlags::MFD_ALLOW_SEALING,
    )
    .map_err(std::io::Error::from)?;
    let mut sealed = File::from(memfd);

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        sealed.write_all(&buffer[..read])?;
    }
    drop(source);

    fcntl(
        &sealed,
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .map_err(std::io::Error::from)?;

    let digest: [u8; 32] = hasher.finalize().into();
    Ok((Sha256Digest(digest), ExecHandle::SealedMemory(sealed)))
}

/// R3-005: `is_windows_executable_extension` is a pure function with no
/// platform dependency, so its own unit tests run on every platform this
/// workspace tests on -- not gated to `#[cfg(windows)]` even though the
/// allowlist it implements is only ever *consulted* by
/// `platform_evidence` on a real Windows build.
#[cfg(test)]
mod extension_allowlist_tests {
    use super::is_windows_executable_extension;

    #[test]
    fn accepts_every_allowlisted_extension() {
        for extension in ["exe", "com", "bat", "cmd"] {
            assert!(
                is_windows_executable_extension(extension),
                "{extension} must be on the allowlist"
            );
        }
    }

    #[test]
    fn rejects_a_non_executable_extension() {
        assert!(!is_windows_executable_extension("txt"));
    }

    #[test]
    fn rejects_the_empty_extension() {
        assert!(!is_windows_executable_extension(""));
    }

    #[test]
    fn is_case_sensitive_over_already_lowercased_input() {
        // `platform_evidence` lowercases before calling this function;
        // this function itself does no further normalization, so an
        // uppercase extension (which should never reach it in practice)
        // is rejected rather than silently accepted.
        assert!(!is_windows_executable_extension("EXE"));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::open_checked;

    /// A drop-guard temp directory: removed on drop regardless of which
    /// path out of the test (pass, fail, or panic) is taken.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-fs-prober-unit-test-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
            Self(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `open_checked`'s whole reason to exist: a symlink placed at an
    /// *already-canonicalized* path must be refused, not followed a
    /// second time. This is `FsExecutableProber::probe`'s own defense
    /// against a swap landing in the narrow window between its
    /// `canonicalize` call and its one `open` call -- tested directly
    /// against the helper, since forcing that exact race against `probe`
    /// itself is not reliably reproducible.
    #[test]
    fn refuses_a_symlink_placed_at_the_canonical_path_after_canonicalization() {
        let dir = TempDir::new("symlink-after-canonicalize");
        let real = dir.path().join("real");
        std::fs::write(&real, b"original content").expect("failed to write the fixture file");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755))
            .expect("failed to chmod the fixture file");

        let canonical = std::fs::canonicalize(&real).expect("the fixture file must canonicalize");

        // Simulate a race: something replaces the file at the
        // already-canonicalized path with a symlink after
        // canonicalization ran.
        std::fs::remove_file(&canonical).expect("failed to remove the original fixture file");
        std::os::unix::fs::symlink("/etc/passwd", &canonical)
            .expect("failed to create the replacement symlink");

        let result = open_checked(&canonical);

        assert!(
            result.is_err(),
            "a symlink placed at an already-canonicalized path must be refused by O_NOFOLLOW, \
             got {result:?}"
        );
    }
}
