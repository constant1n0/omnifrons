//! `FsExecutableProber` turns a candidate filesystem path into a
//! `ProbedExecutable` (identity plus the exact handle it was hashed from)
//! or the specific reason it could not be probed: canonicalize, open the
//! canonical path exactly once, confirm that open file is a regular,
//! executable file within the size cap, then stream-hash its content from
//! that same handle.
//!
//! Most of this file is unix/Linux only (fixtures built with
//! `PermissionsExt`/`std::os::unix::fs`, or exercising the Linux-only
//! sealed-memfd handle shape) -- see `mod unix_tests` below. The
//! extension-allowlist check itself
//! (`omnifrons_adapters::fs_prober::is_windows_executable_extension`) is
//! unit-tested cross-platform in `src/fs_prober.rs`; `mod windows_tests`
//! below additionally exercises the real Windows probe path end to end
//! (R3-005).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::ProbeOutcome;

/// A drop-guard temp directory: removed on drop regardless of which path
/// out of a test (pass, fail, or panic) is taken, so this file's own
/// fixture directories never accumulate under the system temp dir across
/// runs (R3-004).
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-fs-prober-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Unwrap a successful probe's identity+handle, or panic with the
/// outcome for debugging.
fn expect_identity(outcome: ProbeOutcome) -> omnifrons_app::ProbedExecutable {
    match outcome {
        ProbeOutcome::Identity(executable) => executable,
        other => panic!("expected ProbeOutcome::Identity, got {other:?}"),
    }
}

#[cfg(unix)]
mod unix_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use omnifrons_adapters::FsExecutableProber;
    use omnifrons_app::{ExecHandle, ExecutableProber, ProbeOutcome};
    use sha2::{Digest, Sha256};

    use super::{TempDir, expect_identity};

    fn write_file(dir: &Path, name: &str, contents: &[u8], mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("failed to write the test fixture file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("failed to set the test fixture file's permissions");
        path
    }

    #[test]
    fn a_directory_is_not_a_regular_file() {
        let dir = TempDir::new("directory");
        let prober = FsExecutableProber::new();

        let outcome = prober.probe(dir.path());

        assert!(
            matches!(outcome, ProbeOutcome::NotRegularFile),
            "expected NotRegularFile, got {outcome:?}"
        );
    }

    /// A non-executable file must be reported `NotExecutable` without ever
    /// hashing its content.
    ///
    /// The prober opens the candidate exactly once and decides
    /// `NotExecutable` from that one open file's own `fstat`, before any
    /// content read -- so the fixture must itself be *readable* (`0o644`),
    /// unlike an earlier version of this test that used `0o000` to prove
    /// the point via a permission error: that trick no longer applies once
    /// the exec-bit decision runs on an already-open handle's `fstat`
    /// rather than a pre-open `stat`/`lstat` by path (the whole point of
    /// the `open` happening exactly once, R1-001). This version proves the
    /// same "content never read" property differently: the fixture is a
    /// several-hundred-megabyte *sparse* file (never actually written),
    /// and `NotExecutable` is asserted to come back within a tight time
    /// budget -- a probe that actually tried to hash that much sparse
    /// content would not.
    ///
    /// Skipped when running as `root` (R3-002): `root` bypasses unix
    /// permission bits entirely, so this fixture would probe as an
    /// ordinary executable file instead of `NotExecutable`, which is not
    /// this test's own bug to chase down -- it is a property of the
    /// account it runs under. Also skips (with a printed reason) if the
    /// backing filesystem does not support sparse files, mirroring the
    /// size-cap test below.
    #[test]
    fn a_non_executable_file_is_reported_without_reading_its_content() {
        if nix::unistd::Uid::effective().is_root() {
            eprintln!(
                "skipping a_non_executable_file_is_reported_without_reading_its_content: \
                 running as root, which bypasses unix permission bits entirely"
            );
            return;
        }

        let dir = TempDir::new("not-executable");
        let path = dir.path().join("not-executable");
        let file = std::fs::File::create(&path).expect("failed to create the sparse fixture file");
        if let Err(error) = file.set_len(600 * 1024 * 1024) {
            eprintln!(
                "skipping a_non_executable_file_is_reported_without_reading_its_content: \
                 set_len failed ({error}), the backing filesystem may not support sparse files \
                 here"
            );
            return;
        }
        drop(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect(
            "failed to set the sparse fixture file's permissions (readable, not executable)",
        );
        let prober = FsExecutableProber::new();

        let started = Instant::now();
        let outcome = prober.probe(&path);
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, ProbeOutcome::NotExecutable),
            "expected NotExecutable, got {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "deciding NotExecutable from fstat alone must not read the (sparse, 600 MiB) \
             content, took {elapsed:?}"
        );
    }

    #[test]
    fn a_symlink_resolves_to_its_canonical_target_and_follows_a_swap() {
        let dir = TempDir::new("symlink");
        let target_a = write_file(dir.path(), "target-a", b"identity a", 0o755);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target_a, &link).expect("failed to create the test symlink");
        let prober = FsExecutableProber::new();

        let outcome_a = prober.probe(&link);
        let identity_a = expect_identity(outcome_a).identity;
        assert_eq!(
            identity_a.canonical_path,
            std::fs::canonicalize(&target_a).expect("target_a must canonicalize")
        );

        std::fs::remove_file(&link).expect("failed to remove the symlink before swapping it");
        let target_b = write_file(dir.path(), "target-b", b"identity b, longer content", 0o755);
        std::os::unix::fs::symlink(&target_b, &link).expect("failed to recreate the test symlink");

        let outcome_b = prober.probe(&link);
        let identity_b = expect_identity(outcome_b).identity;
        assert_eq!(
            identity_b.canonical_path,
            std::fs::canonicalize(&target_b).expect("target_b must canonicalize")
        );
        assert_ne!(
            identity_a.canonical_path, identity_b.canonical_path,
            "re-probing after swapping the symlink's target must observe the new canonical path"
        );
    }

    #[test]
    fn a_zero_byte_file_hashes_correctly() {
        let dir = TempDir::new("zero-byte");
        let path = write_file(dir.path(), "empty", b"", 0o755);
        let prober = FsExecutableProber::new();

        let executable = expect_identity(prober.probe(&path));

        assert_eq!(executable.identity.size, 0);
        assert_eq!(
            &executable.identity.sha256.0[..],
            Sha256::digest(b"").as_slice()
        );
    }

    #[test]
    fn a_five_mebibyte_file_hashes_correctly() {
        let dir = TempDir::new("five-mib");
        let contents = vec![0xab_u8; 5 * 1024 * 1024];
        let path = write_file(dir.path(), "five-mib", &contents, 0o755);
        let prober = FsExecutableProber::new();

        let executable = expect_identity(prober.probe(&path));

        assert_eq!(executable.identity.size, contents.len() as u64);
        assert_eq!(
            &executable.identity.sha256.0[..],
            Sha256::digest(&contents).as_slice()
        );
    }

    /// A sparse file (`set_len`, never actually written) well over the 512
    /// MiB cap must be rejected on size alone, without ever reading its
    /// content -- proven by a 2-second budget (R3-002) rather than the
    /// original "completes near-instantly" hand-wave: a probe that *did*
    /// start reading hundreds of megabytes of zero bytes would still
    /// eventually finish, just not within a budget this generous.
    ///
    /// Skips (with a printed reason, R3-002) if the filesystem backing the
    /// system temp dir does not support sparse files / `set_len` beyond
    /// its available space -- not every CI or contributor filesystem is
    /// guaranteed to allow instantiating a 600 MiB sparse file.
    #[test]
    fn a_file_over_the_size_cap_is_rejected_without_reading() {
        let dir = TempDir::new("too-large");
        let path = dir.path().join("too-large");
        let file = std::fs::File::create(&path).expect("failed to create the sparse fixture file");
        if let Err(error) = file.set_len(600 * 1024 * 1024) {
            eprintln!(
                "skipping a_file_over_the_size_cap_is_rejected_without_reading: set_len failed \
                 ({error}), the backing filesystem may not support sparse files here"
            );
            return;
        }
        drop(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("failed to set the sparse fixture file's permissions");
        let prober = FsExecutableProber::new();

        let started = Instant::now();
        let outcome = prober.probe(&path);
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, ProbeOutcome::TooLarge),
            "expected TooLarge, got {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "rejecting an over-cap file on size alone must not read its content, took \
             {elapsed:?}"
        );
    }

    /// R1-001(a): on every platform other than Linux (where the returned
    /// handle is instead a sealed `memfd` -- a deliberately *different*
    /// inode holding a verified copy of the content, covered by the
    /// Linux-only test below), `ExecHandle::File` must `fstat` to the
    /// exact same `(dev, ino)` as an independent `stat` of the canonical
    /// path taken right after probing -- structural evidence the returned
    /// handle really does refer to the file this probe examined and
    /// hashed, not some other file description.
    ///
    /// Not exercised on Linux in this repository's own CI/dev environment
    /// (this workspace's tests run on Linux only, per
    /// `docs/spike-log.md`'s own dev-mode-only caveat) -- recorded here,
    /// not silently skipped, so this platform gap is visible rather than
    /// assumed covered.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_returned_handle_fstats_to_the_same_inode_as_the_probed_path() {
        use std::os::unix::fs::MetadataExt as _;

        let dir = TempDir::new("handle-identity");
        let path = write_file(dir.path(), "target", b"handle identity fixture", 0o755);
        let prober = FsExecutableProber::new();

        let executable = expect_identity(prober.probe(&path));
        let independent_stat = std::fs::metadata(&executable.identity.canonical_path)
            .expect("independent stat must succeed");
        let handle_stat = executable
            .handle_metadata()
            .expect("handle_metadata must succeed");

        assert_eq!(
            (independent_stat.dev(), independent_stat.ino()),
            (handle_stat.dev(), handle_stat.ino()),
            "the returned handle must fstat to the same (dev, ino) as the probed canonical path"
        );
    }

    /// Linux only: R1-001. The sealed `memfd` a probe returns must
    /// contain exactly the bytes that were hashed (length matches
    /// `identity.size`, and re-hashing its content reproduces
    /// `identity.sha256`), and must be genuinely immutable -- a write
    /// against it fails with `EPERM`.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_sealed_memfd_contains_exactly_the_hashed_bytes_and_rejects_writes() {
        use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

        let dir = TempDir::new("sealed-memfd");
        let contents = b"sealed memfd fixture content, deliberately not a multiple of the hash \
                          buffer size"
            .repeat(3);
        let path = write_file(dir.path(), "target", &contents, 0o755);
        let prober = FsExecutableProber::new();

        let executable = expect_identity(prober.probe(&path));
        let ExecHandle::SealedMemory(mut sealed) = executable.handle else {
            panic!("expected ExecHandle::SealedMemory on Linux");
        };

        assert_eq!(
            executable.identity.size,
            contents.len() as u64,
            "identity.size must match the fixture's real length"
        );

        sealed
            .seek(SeekFrom::Start(0))
            .expect("seeking the sealed memfd to the start must succeed");
        let mut sealed_contents = Vec::new();
        sealed
            .read_to_end(&mut sealed_contents)
            .expect("reading the sealed memfd's content must succeed");
        assert_eq!(
            sealed_contents.len() as u64,
            executable.identity.size,
            "the sealed memfd's length must equal identity.size"
        );
        assert_eq!(
            &Sha256::digest(&sealed_contents)[..],
            &executable.identity.sha256.0[..],
            "re-hashing the sealed memfd's content must reproduce identity.sha256"
        );

        let write_result = sealed.write_all(b"x");
        assert!(
            write_result.is_err(),
            "a sealed memfd must reject further writes, got {write_result:?}"
        );
        if let Err(error) = write_result {
            assert_eq!(
                error.raw_os_error(),
                Some(nix::errno::Errno::EPERM as i32),
                "a sealed memfd's write must fail with EPERM specifically, got {error:?}"
            );
        }
    }

    // R1-001(b) (a symlink placed at the canonical path *after*
    // canonicalization is refused) is tested directly against the
    // private `open_checked` helper, not through the public `probe` entry
    // point: see `refuses_a_symlink_placed_at_the_canonical_path_after_canonicalization`
    // in `src/fs_prober.rs`'s own `#[cfg(test)]` module. `probe`'s own
    // `canonicalize` step resolves any symlink *before* `open_checked`
    // ever runs, so exercising this race through `probe` itself would
    // require forcing a swap in the narrow window between those two
    // calls -- not reliably reproducible from a test.
}

/// R3-005: a Windows-only end-to-end check of the real probe path,
/// alongside `is_windows_executable_extension`'s own cross-platform unit
/// tests in `src/fs_prober.rs`. Content is irrelevant to either
/// assertion here -- Windows executability is decided by extension
/// allowlist alone in this spike (`FsExecutableProber`'s own doc
/// comment), a documented, provisional limitation, not an oversight.
///
/// Not exercised in this repository's own Linux-only CI/dev environment
/// -- recorded here, not silently skipped, so this platform gap is
/// visible rather than assumed covered.
#[cfg(windows)]
mod windows_tests {
    use omnifrons_adapters::FsExecutableProber;
    use omnifrons_app::{ExecutableProber, ProbeOutcome};

    use super::{TempDir, expect_identity};

    #[test]
    fn a_txt_file_is_not_executable() {
        let dir = TempDir::new("windows-txt");
        let path = dir.path().join("candidate.txt");
        std::fs::write(&path, b"plain text, irrelevant content")
            .expect("failed to write the .txt fixture file");
        let prober = FsExecutableProber::new();

        let outcome = prober.probe(&path);

        assert!(
            matches!(outcome, ProbeOutcome::NotExecutable),
            "a .txt file must be NotExecutable, got {outcome:?}"
        );
    }

    #[test]
    fn an_exe_named_fixture_passes_the_extension_check_regardless_of_content() {
        let dir = TempDir::new("windows-exe");
        let path = dir.path().join("candidate.exe");
        std::fs::write(
            &path,
            b"not a real PE binary, content is irrelevant to this check",
        )
        .expect("failed to write the .exe fixture file");
        let prober = FsExecutableProber::new();

        let executable = expect_identity(prober.probe(&path));

        assert_eq!(
            executable.identity.canonical_path,
            std::fs::canonicalize(&path).expect("the .exe fixture must canonicalize")
        );
    }
}
