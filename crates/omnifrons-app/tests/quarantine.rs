//! The quarantine root (spike slice 5d, RCS-001-R10 and its D3, HAP-001
//! D1's sibling siting): device-local, owner-only, outside every
//! registered workspace, canonicalized when configured and re-checked at
//! every use -- the same discipline `WorkAreaRoot` already runs, applied to
//! the directory RCS-001 sites downloads and attachments in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::quarantine::{QuarantineError, QuarantineRoot};

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-quarantine-test-{}-{label}-{n}",
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
fn opening_a_quarantine_root_creates_it_canonical_and_outside_every_workspace() {
    let base = TempDir::new("open");
    let project = TempDir::new("open-project");
    let configured = base.path().join("quarantine");
    let root =
        QuarantineRoot::open(&configured, &[&project.workspace()]).expect("a fresh quarantine");
    assert_eq!(
        root.path(),
        std::fs::canonicalize(&configured).expect("canonical")
    );
    assert!(root.path().is_dir());
    root.check(&[&project.workspace()])
        .expect("the re-check passes while nothing overlaps");
}

/// RCS-001-R10: the quarantine location is outside any workspace. A
/// configured path inside a registered workspace root is refused, and
/// nothing is created there.
#[test]
fn a_quarantine_root_inside_a_registered_workspace_is_refused() {
    let project = TempDir::new("inside");
    let configured = project.path().join("nested/quarantine");
    assert_eq!(
        QuarantineRoot::open(&configured, &[&project.workspace()]),
        Err(QuarantineError::InsideWorkspace)
    );
    assert!(
        !configured.exists(),
        "a refused quarantine root is never created"
    );
}

/// The re-check at every use, not only at configuration: a workspace
/// registered over the quarantine directory afterwards makes the next
/// check refuse.
#[test]
fn a_workspace_registered_over_the_quarantine_makes_the_next_check_refuse() {
    let base = TempDir::new("recheck");
    let project = TempDir::new("recheck-project");
    let configured = base.path().join("quarantine");
    let root =
        QuarantineRoot::open(&configured, &[&project.workspace()]).expect("a fresh quarantine");
    let overlapping = WorkspaceRoot::new(base.path()).expect("valid workspace");
    assert_eq!(
        root.check(&[&overlapping]),
        Err(QuarantineError::InsideWorkspace),
        "a workspace registered over the quarantine refuses the next use"
    );
}

/// Unix: the quarantine directory is owner-only (0700), like the work
/// area. Windows expresses no owner-only permission in `std` (ACLs are
/// outside it) -- disclosed, not asserted there.
#[cfg(unix)]
#[test]
fn the_quarantine_root_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = TempDir::new("mode");
    let project = TempDir::new("mode-project");
    let root = QuarantineRoot::open(&base.path().join("quarantine"), &[&project.workspace()])
        .expect("a fresh quarantine");
    let mode = std::fs::metadata(root.path())
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "the quarantine root must be owner-only");
}

/// R3-022: `QuarantineError::NotOwnerOnly` is reachable, and reached. A
/// quarantine directory that already exists with wider permissions is
/// refused rather than used: RCS-001-R10 asks for owner-only, and a
/// directory another local account can read is not it.
#[cfg(unix)]
#[test]
fn a_quarantine_root_that_is_not_owner_only_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = TempDir::new("wide-mode");
    let project = TempDir::new("wide-mode-project");
    let configured = base.path().join("quarantine");
    std::fs::create_dir_all(&configured).expect("a pre-existing directory");
    std::fs::set_permissions(&configured, std::fs::Permissions::from_mode(0o755))
        .expect("widen it");

    let opened = QuarantineRoot::open(&configured, &[&project.workspace()]);
    let group_and_other = std::fs::metadata(&configured)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o077;
    if group_and_other == 0 {
        // A restrictive umask or a platform that clamps the mode already
        // made it owner-only: there is no wider directory to refuse.
        return;
    }
    assert_eq!(
        opened,
        Err(QuarantineError::NotOwnerOnly),
        "a quarantine directory another account can reach is refused, never used"
    );
}
