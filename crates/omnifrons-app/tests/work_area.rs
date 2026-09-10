//! The product work area (HAP-001-R7, D1) and the device asset path
//! (HAP-001-R14): device-local, owner-only, never inside a registered
//! workspace root, canonicalized and re-checked at every use -- a
//! workspace registered over either afterwards makes the next check
//! refuse.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::blob_store::{DestinationError, DeviceAssetPath};
use omnifrons_app::work_area::{
    IGNORE_DIR, JOURNAL_DIR, RECOVERY_DIR, SNAPSHOTS_DIR, WorkAreaError, WorkAreaRoot,
};

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-work-area-test-{}-{label}-{n}",
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

#[test]
fn opening_a_work_area_creates_it_with_its_journal_and_recovery_directories() {
    let base = TempDir::new("open");
    let project = TempDir::new("open-project");
    let configured = base.path().join("work-area");
    let work_area =
        WorkAreaRoot::open(&configured, &[&project.workspace()]).expect("a fresh work area opens");
    assert_eq!(
        work_area.path(),
        std::fs::canonicalize(&configured).expect("canonical")
    );
    assert!(work_area.journal_dir().is_dir());
    assert!(work_area.recovery_dir().is_dir());
    assert_eq!(work_area.journal_dir(), work_area.path().join(JOURNAL_DIR));
    assert_eq!(
        work_area.recovery_dir(),
        work_area.path().join(RECOVERY_DIR)
    );
    // Spike slice 5c: the guidance installer's snapshots live beside them.
    assert!(work_area.snapshots_dir().is_dir());
    assert_eq!(
        work_area.snapshots_dir(),
        work_area.path().join(SNAPSHOTS_DIR)
    );
    // Spike slice 5d: the wrong-root ignore ledger, the one durable fact a
    // remedy leaves behind, joins them.
    assert!(work_area.ignore_dir().is_dir());
    assert_eq!(work_area.ignore_dir(), work_area.path().join(IGNORE_DIR));
    assert_eq!(
        std::fs::read_dir(work_area.path()).expect("list").count(),
        4,
        "the work area holds journal/, recovery/, snapshots/, and ignore/ and nothing else"
    );
}

/// Unix: the work area and both subdirectories are owner-only (0700).
/// Windows expresses no owner-only permission in `std` (ACLs are outside
/// it) -- disclosed, not asserted there.
#[cfg(unix)]
#[test]
fn the_work_area_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = TempDir::new("mode");
    let project = TempDir::new("mode-project");
    let work_area =
        WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("opens");
    for dir in [
        work_area.path().to_path_buf(),
        work_area.journal_dir(),
        work_area.recovery_dir(),
        work_area.snapshots_dir(),
    ] {
        let mode = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "{} must be owner-only", dir.display());
    }
}

/// HAP-001-R7: a work area configured inside a registered workspace root
/// is refused at configuration time, and nothing is created there.
#[test]
fn a_work_area_inside_a_registered_workspace_is_refused() {
    let project = TempDir::new("inside");
    let configured = project.path().join(".omnifrons").join("work-area");
    let result = WorkAreaRoot::open(&configured, &[&project.workspace()]);
    assert_eq!(result.err(), Some(WorkAreaError::InsideWorkspace));
    assert!(
        !configured.exists(),
        "nothing is created inside the workspace"
    );
}

/// HAP-001-R7: the work area is re-canonicalized and re-checked at every
/// use -- a workspace registered *over* a valid work area afterwards makes
/// the next check refuse with `work-area-invalid`.
#[test]
fn a_workspace_registered_over_the_work_area_makes_the_next_check_refuse() {
    let base = TempDir::new("later");
    let project = TempDir::new("later-project");
    let configured = base.path().join("work-area");
    let work_area =
        WorkAreaRoot::open(&configured, &[&project.workspace()]).expect("opens outside");
    assert_eq!(work_area.check(&[&project.workspace()]), Ok(()));
    // The user now registers the base directory itself as a workspace.
    let over = base.workspace();
    assert_eq!(
        work_area.check(&[&project.workspace(), &over]),
        Err(WorkAreaError::InsideWorkspace)
    );
}

/// The work area is also refused when it *is* a registered workspace root,
/// not only when it lies below one.
#[test]
fn a_work_area_equal_to_a_workspace_root_is_refused() {
    let base = TempDir::new("equal");
    let result = WorkAreaRoot::open(base.path(), &[&base.workspace()]);
    assert_eq!(result.err(), Some(WorkAreaError::InsideWorkspace));
}

/// A work area whose directory vanished after configuration cannot be
/// used: the re-check fails as unusable rather than passing on a stale
/// canonical path.
#[test]
fn a_work_area_that_vanished_fails_its_re_check() {
    let base = TempDir::new("vanished");
    let project = TempDir::new("vanished-project");
    let configured = base.path().join("work-area");
    let work_area = WorkAreaRoot::open(&configured, &[&project.workspace()]).expect("opens");
    std::fs::remove_dir_all(&configured).expect("remove");
    assert_eq!(
        work_area.check(&[&project.workspace()]),
        Err(WorkAreaError::Unusable)
    );
}

// -- the device asset path (HAP-001-R14) --

#[test]
fn a_device_asset_path_outside_every_workspace_opens_canonical() {
    let base = TempDir::new("asset-root");
    let project = TempDir::new("asset-root-project");
    let configured = base.path().join("roots").join("main");
    let path = DeviceAssetPath::open(&configured, &[&project.workspace()]).expect("opens");
    assert_eq!(
        path.path(),
        std::fs::canonicalize(&configured).expect("canonical")
    );
}

/// HAP-001-R14: a device asset path that resolves inside a registered
/// workspace root renders `destination-invalid`, and nothing is created.
#[test]
fn a_device_asset_path_inside_a_workspace_is_destination_invalid() {
    let project = TempDir::new("asset-inside");
    let configured = project.path().join("assets");
    let result = DeviceAssetPath::open(&configured, &[&project.workspace()]);
    assert_eq!(result.err(), Some(DestinationError::InsideWorkspace));
    assert!(!configured.exists());
}

/// A device asset path whose parent is a symbolic link into a workspace
/// is caught by canonicalization, not by the spelled path.
#[cfg(unix)] // symlink creation needs a privilege the Windows CI runner lacks
#[test]
fn a_device_asset_path_linked_into_a_workspace_is_destination_invalid() {
    let base = TempDir::new("asset-link");
    let project = TempDir::new("asset-link-project");
    let link = base.path().join("link");
    std::os::unix::fs::symlink(project.path(), &link).expect("symlink");
    let result = DeviceAssetPath::open(&link.join("assets"), &[&project.workspace()]);
    assert_eq!(result.err(), Some(DestinationError::InsideWorkspace));
}

/// R1-002 (HAP-001-R7 on every workspace registration): a configured work
/// area is checked against a workspace without being created, whether it
/// exists yet or not, and an existing one registered over afterwards fails
/// the same check.
#[test]
fn a_configured_work_area_is_checked_without_being_created() {
    let base = TempDir::new("check-configured");
    let project = TempDir::new("check-configured-project");
    let configured = base.path().join("work-area");
    assert_eq!(
        WorkAreaRoot::check_configured(&configured, &[&project.workspace()]),
        Ok(())
    );
    assert!(!configured.exists(), "a check creates nothing");
    let inside = project.path().join(".omnifrons").join("work-area");
    assert_eq!(
        WorkAreaRoot::check_configured(&inside, &[&project.workspace()]),
        Err(WorkAreaError::InsideWorkspace)
    );
    assert!(!inside.exists());
    let _opened = WorkAreaRoot::open(&configured, &[&project.workspace()]).expect("opens");
    let over = base.workspace();
    assert_eq!(
        WorkAreaRoot::check_configured(&configured, &[&project.workspace(), &over]),
        Err(WorkAreaError::InsideWorkspace)
    );
}
