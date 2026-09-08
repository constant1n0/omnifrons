//! `validate_outbox_declaration` (spike slice 5, HAP-001-R8): a declared
//! outbox path must canonicalize inside the project root and must not be
//! a link; a declaration that fails either is `outbox-invalid`, a real
//! directory is `Present`, a path with nothing at it yet is `Missing` (the
//! preparer creates it), and something that is neither a directory nor a
//! link at the path is `outbox-unavailable`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::run_outbox::{
    OutboxDeclarationError, OutboxLocation, validate_outbox_declaration,
};
use omnifrons_domain::outbox::{OutboxFailure, OutboxPath};

/// A drop-guard temp directory, removed on every exit path.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-outbox-declaration-test-{}-{label}-{n}",
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

fn project(dir: &TempDir) -> WorkspaceRoot {
    WorkspaceRoot::new(dir.path()).expect("the fixture dir must be a valid workspace")
}

#[test]
fn a_real_directory_inside_the_project_is_present_at_its_canonical_path() {
    let dir = TempDir::new("present");
    std::fs::create_dir_all(dir.path().join(".omnifrons/outbox")).expect("fixture");
    let location = validate_outbox_declaration(&project(&dir), &OutboxPath::default_path())
        .expect("a real directory inside the project must validate");
    let expected = std::fs::canonicalize(dir.path().join(".omnifrons/outbox")).expect("canonical");
    assert_eq!(location, OutboxLocation::Present(expected));
}

#[test]
fn a_declared_path_with_nothing_at_it_yet_is_missing() {
    let dir = TempDir::new("missing");
    let location = validate_outbox_declaration(&project(&dir), &OutboxPath::default_path())
        .expect("a missing outbox is a valid declaration the preparer will create");
    assert_eq!(
        location,
        OutboxLocation::Missing(project(&dir).path().join(".omnifrons/outbox"))
    );
}

#[test]
fn a_regular_file_at_the_declared_path_is_not_a_directory_and_renders_unavailable() {
    let dir = TempDir::new("file");
    std::fs::create_dir_all(dir.path().join(".omnifrons")).expect("fixture");
    std::fs::write(dir.path().join(".omnifrons/outbox"), b"not a dir").expect("fixture");
    let error =
        validate_outbox_declaration(&project(&dir), &OutboxPath::default_path()).unwrap_err();
    assert_eq!(error, OutboxDeclarationError::NotADirectory);
    assert_eq!(error.failure(), OutboxFailure::OutboxUnavailable);
}

/// Unix only, gated on its own test rather than the whole file (the
/// repository's convention): creating a symlink on Windows needs a
/// privilege the CI runner does not grant, so the fixture -- not the
/// property under test -- would fail there.
#[cfg(unix)]
#[test]
fn a_declared_path_that_is_a_link_is_invalid_even_when_it_points_inside_the_project() {
    let dir = TempDir::new("link-inside");
    std::fs::create_dir_all(dir.path().join("real-outbox")).expect("fixture");
    std::fs::create_dir_all(dir.path().join(".omnifrons")).expect("fixture");
    std::os::unix::fs::symlink(
        dir.path().join("real-outbox"),
        dir.path().join(".omnifrons/outbox"),
    )
    .expect("fixture symlink");
    let error =
        validate_outbox_declaration(&project(&dir), &OutboxPath::default_path()).unwrap_err();
    assert_eq!(error, OutboxDeclarationError::IsLink);
    assert_eq!(error.failure(), OutboxFailure::OutboxInvalid);
}

/// Unix only (symlink fixture, as above): a real directory reached through
/// a linked intermediate component that resolves outside the project is
/// `outbox-invalid` -- the path itself is not a link, so canonicalization
/// is what catches it.
#[cfg(unix)]
#[test]
fn a_declared_path_that_canonicalizes_outside_the_project_is_invalid() {
    let dir = TempDir::new("escape");
    let outside = TempDir::new("escape-target");
    std::fs::create_dir_all(outside.path().join("outbox")).expect("fixture");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("elsewhere"))
        .expect("fixture symlink");
    let declared = OutboxPath::new("elsewhere/outbox").expect("a relative declaration");
    let error = validate_outbox_declaration(&project(&dir), &declared).unwrap_err();
    assert_eq!(error, OutboxDeclarationError::OutsideProject);
    assert_eq!(error.failure(), OutboxFailure::OutboxInvalid);
}

#[test]
fn every_declaration_error_maps_to_exactly_one_launch_failure_token() {
    let cases = [
        (
            OutboxDeclarationError::OutsideProject,
            OutboxFailure::OutboxInvalid,
        ),
        (OutboxDeclarationError::IsLink, OutboxFailure::OutboxInvalid),
        (
            OutboxDeclarationError::NotADirectory,
            OutboxFailure::OutboxUnavailable,
        ),
        (
            OutboxDeclarationError::Unreadable,
            OutboxFailure::OutboxUnavailable,
        ),
    ];
    for (error, failure) in cases {
        assert_eq!(error.failure(), failure, "{error:?}");
    }
}
