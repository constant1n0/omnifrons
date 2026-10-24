//! Filesystem tests for `FsDefinitionWriter` (ADR-0005 sub-slice 2c): the
//! atomic, path-aware write of one planned file against a real temp
//! directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::{FsDefinitionWriter, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::definition_writer::{
    DefinitionWriteError, DefinitionWriter, ModePolicy, WriteRequest,
};
use omnifrons_domain::definition_plan::InputFingerprint;
use omnifrons_domain::executable::Sha256Digest;

/// A drop-guard temp directory standing in for a workspace.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-writer-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.0).expect("valid workspace")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha2Hasher::new().sha256(bytes)
}

fn put(dir: &TempDir, relative: &str, bytes: &[u8]) {
    let target = dir.path().join(relative);
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(target, bytes).expect("write fixture");
}

fn write(
    dir: &TempDir,
    path: &str,
    bytes: &[u8],
    expected: &InputFingerprint,
    mode: &ModePolicy,
) -> Result<Sha256Digest, DefinitionWriteError> {
    FsDefinitionWriter::new().write(
        &dir.workspace(),
        &WriteRequest {
            path,
            bytes,
            expected,
            mode,
        },
    )
}

/// Every directory entry name directly under `dir`.
fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    found.sort();
    found
}

#[test]
fn an_exclusive_create_writes_the_exact_bytes_and_returns_their_digest() {
    let dir = TempDir::new("create");
    let bytes = b"# Agent\r\nline two\nno final newline";
    let written = write(
        &dir,
        "AGENTS.md",
        bytes,
        &InputFingerprint::Absent,
        &ModePolicy::Keep,
    )
    .expect("created");
    assert_eq!(written, digest(bytes));
    assert_eq!(
        std::fs::read(dir.path().join("AGENTS.md")).expect("read"),
        bytes
    );
    assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
}

#[test]
fn a_replace_with_the_planned_digest_publishes_the_new_bytes() {
    let dir = TempDir::new("replace");
    put(&dir, "child/AGENTS.md", b"old\n");
    let written = write(
        &dir,
        "child/AGENTS.md",
        b"new\r\n",
        &InputFingerprint::Sha256(digest(b"old\n")),
        &ModePolicy::Keep,
    )
    .expect("replaced");
    assert_eq!(written, digest(b"new\r\n"));
    assert_eq!(
        std::fs::read(dir.path().join("child/AGENTS.md")).expect("read"),
        b"new\r\n"
    );
    assert_eq!(names(&dir.path().join("child")), vec!["AGENTS.md"]);
}

#[test]
fn a_stale_digest_refuses_and_leaves_the_file_byte_identical() {
    let dir = TempDir::new("stale");
    put(&dir, "AGENTS.md", b"edited meanwhile\n");
    let planned = InputFingerprint::Sha256(digest(b"as planned\n"));
    let error =
        write(&dir, "AGENTS.md", b"new\n", &planned, &ModePolicy::Keep).expect_err("refused");
    assert_eq!(error.reason(), "stale");
    assert_eq!(
        error,
        DefinitionWriteError::Stale {
            path: "AGENTS.md".to_string(),
            expected: planned,
            found: InputFingerprint::Sha256(digest(b"edited meanwhile\n")),
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("AGENTS.md")).expect("read"),
        b"edited meanwhile\n"
    );
    assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
}

#[test]
fn an_exclusive_create_refuses_when_the_file_appeared_and_leaves_it_untouched() {
    let dir = TempDir::new("appeared");
    put(&dir, "AGENTS.md", b"someone else's\n");
    let error = write(
        &dir,
        "AGENTS.md",
        b"mine\n",
        &InputFingerprint::Absent,
        &ModePolicy::Keep,
    )
    .expect_err("refused");
    assert_eq!(
        error,
        DefinitionWriteError::Stale {
            path: "AGENTS.md".to_string(),
            expected: InputFingerprint::Absent,
            found: InputFingerprint::Sha256(digest(b"someone else's\n")),
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("AGENTS.md")).expect("read"),
        b"someone else's\n"
    );
    assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
}

#[test]
fn a_recorded_symlink_or_irregular_input_is_never_writable() {
    let dir = TempDir::new("not-writable");
    for expected in [
        InputFingerprint::Symlink("x".to_string()),
        InputFingerprint::Irregular,
    ] {
        let error =
            write(&dir, "AGENTS.md", b"n\n", &expected, &ModePolicy::Keep).expect_err("refused");
        assert_eq!(error.reason(), "not-writable");
    }
    assert!(names(dir.path()).is_empty());
}

#[test]
fn an_invalid_path_refuses_before_touching_anything() {
    let dir = TempDir::new("invalid-path");
    for path in [
        "", ".", "/abs", "../x", "a/../b", "./a", "a//b", "a/", "a\0b",
    ] {
        let error = write(
            &dir,
            path,
            b"n\n",
            &InputFingerprint::Absent,
            &ModePolicy::Keep,
        )
        .expect_err(path);
        assert_eq!(error.reason(), "invalid-path", "{path}");
    }
    assert!(names(dir.path()).is_empty());
}

#[test]
fn a_missing_parent_directory_refuses() {
    let dir = TempDir::new("no-parent");
    let error = write(
        &dir,
        "missing/AGENTS.md",
        b"n\n",
        &InputFingerprint::Absent,
        &ModePolicy::Keep,
    )
    .expect_err("refused");
    assert_eq!(error.reason(), "parent-missing");
    assert!(names(dir.path()).is_empty());
}

#[test]
fn a_directory_at_the_target_is_not_a_regular_file() {
    let dir = TempDir::new("dir-target");
    std::fs::create_dir(dir.path().join("AGENTS.md")).expect("mkdir");
    let error = write(
        &dir,
        "AGENTS.md",
        b"n\n",
        &InputFingerprint::Sha256(digest(b"x")),
        &ModePolicy::Keep,
    )
    .expect_err("refused");
    assert_eq!(error.reason(), "not-regular");
    assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    fn mode_of(dir: &TempDir, relative: &str) -> u32 {
        std::fs::symlink_metadata(dir.path().join(relative))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    fn chmod(dir: &TempDir, relative: &str, mode: u32) {
        std::fs::set_permissions(
            dir.path().join(relative),
            std::fs::Permissions::from_mode(mode),
        )
        .expect("chmod");
    }

    #[test]
    fn a_name_with_a_backslash_or_colon_is_written_and_read_back_like_any_other() {
        let dir = TempDir::new("odd-names");
        for name in ["a\\b.md", "d:e.md"] {
            let written = write(
                &dir,
                name,
                b"odd\r\n",
                &InputFingerprint::Absent,
                &ModePolicy::Keep,
            )
            .expect("created");
            assert_eq!(written, digest(b"odd\r\n"));
            assert_eq!(
                std::fs::read(dir.path().join(name)).expect("read"),
                b"odd\r\n"
            );
        }
        assert_eq!(names(dir.path()), vec!["a\\b.md", "d:e.md"]);
    }

    #[test]
    fn a_target_that_became_a_symlink_refuses_without_following_it() {
        let dir = TempDir::new("symlink-target");
        let outside = TempDir::new("symlink-outside");
        put(&outside, "secret.md", b"outside\n");
        symlink(
            outside.path().join("secret.md"),
            dir.path().join("AGENTS.md"),
        )
        .expect("symlink");
        let error = write(
            &dir,
            "AGENTS.md",
            b"n\n",
            &InputFingerprint::Sha256(digest(b"outside\n")),
            &ModePolicy::Keep,
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "not-regular");
        assert_eq!(
            std::fs::read(outside.path().join("secret.md")).expect("read"),
            b"outside\n"
        );
        assert!(
            std::fs::symlink_metadata(dir.path().join("AGENTS.md"))
                .expect("metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
    }

    #[test]
    fn a_symlink_that_appeared_where_a_create_was_planned_is_stale_and_untouched() {
        let dir = TempDir::new("symlink-appeared");
        let outside = TempDir::new("symlink-appeared-outside");
        symlink(outside.path().join("new.md"), dir.path().join("AGENTS.md")).expect("symlink");
        let error = write(
            &dir,
            "AGENTS.md",
            b"n\n",
            &InputFingerprint::Absent,
            &ModePolicy::Keep,
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "stale");
        assert!(!outside.path().join("new.md").exists());
    }

    #[test]
    fn a_parent_directory_that_is_a_symlink_refuses() {
        let dir = TempDir::new("symlink-parent");
        let outside = TempDir::new("symlink-parent-outside");
        symlink(outside.path(), dir.path().join("child")).expect("symlink");
        let error = write(
            &dir,
            "child/AGENTS.md",
            b"n\n",
            &InputFingerprint::Absent,
            &ModePolicy::Keep,
        )
        .expect_err("refused");
        assert_eq!(error.reason(), "not-regular");
        assert!(names(outside.path()).is_empty());
    }

    #[test]
    fn a_replaced_file_keeps_its_mode() {
        let dir = TempDir::new("keep-mode");
        put(&dir, "AGENTS.md", b"old\n");
        chmod(&dir, "AGENTS.md", 0o640);
        write(
            &dir,
            "AGENTS.md",
            b"new\n",
            &InputFingerprint::Sha256(digest(b"old\n")),
            &ModePolicy::Keep,
        )
        .expect("replaced");
        assert_eq!(mode_of(&dir, "AGENTS.md"), 0o640);
    }

    #[test]
    fn a_created_file_takes_the_source_mode() {
        let dir = TempDir::new("source-mode");
        put(&dir, "CLAUDE.md", b"src\n");
        chmod(&dir, "CLAUDE.md", 0o600);
        write(
            &dir,
            "AGENTS.md",
            b"src\n",
            &InputFingerprint::Absent,
            &ModePolicy::NoWiderThan {
                source: "CLAUDE.md".to_string(),
            },
        )
        .expect("created");
        assert_eq!(mode_of(&dir, "AGENTS.md"), 0o600);
    }

    #[test]
    fn a_copy_over_an_existing_file_never_widens_beyond_the_source() {
        let dir = TempDir::new("never-widen");
        put(&dir, "CLAUDE.md", b"src\n");
        chmod(&dir, "CLAUDE.md", 0o644);
        put(&dir, "AGENTS.md", b"old\n");
        chmod(&dir, "AGENTS.md", 0o600);
        write(
            &dir,
            "AGENTS.md",
            b"src\n",
            &InputFingerprint::Sha256(digest(b"old\n")),
            &ModePolicy::NoWiderThan {
                source: "CLAUDE.md".to_string(),
            },
        )
        .expect("replaced");
        assert_eq!(mode_of(&dir, "AGENTS.md"), 0o600);

        // And the other way: a wider existing file narrows to the source.
        chmod(&dir, "CLAUDE.md", 0o640);
        chmod(&dir, "AGENTS.md", 0o666);
        write(
            &dir,
            "AGENTS.md",
            b"src2\n",
            &InputFingerprint::Sha256(digest(b"src\n")),
            &ModePolicy::NoWiderThan {
                source: "CLAUDE.md".to_string(),
            },
        )
        .expect("replaced");
        assert_eq!(mode_of(&dir, "AGENTS.md"), 0o640);
    }

    #[test]
    fn an_unusable_permission_source_refuses_and_leaves_nothing_behind() {
        let dir = TempDir::new("bad-source");
        let outside = TempDir::new("bad-source-outside");
        put(&outside, "real.md", b"x\n");
        symlink(outside.path().join("real.md"), dir.path().join("CLAUDE.md")).expect("symlink");
        for source in ["CLAUDE.md", "MISSING.md"] {
            let error = write(
                &dir,
                "AGENTS.md",
                b"n\n",
                &InputFingerprint::Absent,
                &ModePolicy::NoWiderThan {
                    source: source.to_string(),
                },
            )
            .expect_err("refused");
            assert_eq!(error.reason(), "source-unavailable", "{source}");
        }
        assert_eq!(names(dir.path()), vec!["CLAUDE.md"]);
    }

    #[test]
    fn no_temporary_file_remains_after_a_failed_publish() {
        let dir = TempDir::new("no-temp");
        put(&dir, "AGENTS.md", b"changed\n");
        let _ = write(
            &dir,
            "AGENTS.md",
            b"n\n",
            &InputFingerprint::Sha256(digest(b"planned\n")),
            &ModePolicy::Keep,
        );
        let _ = write(
            &dir,
            "AGENTS.md",
            b"n\n",
            &InputFingerprint::Absent,
            &ModePolicy::Keep,
        );
        assert_eq!(names(dir.path()), vec!["AGENTS.md"]);
    }
}

#[cfg(windows)]
#[test]
fn a_backslash_or_colon_in_a_name_is_refused_on_windows() {
    let dir = TempDir::new("windows-names");
    for path in ["a\\b.md", "d:e.md"] {
        let error = write(
            &dir,
            path,
            b"n\n",
            &InputFingerprint::Absent,
            &ModePolicy::Keep,
        )
        .expect_err(path);
        assert_eq!(error.reason(), "invalid-path");
    }
}
