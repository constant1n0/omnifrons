//! Managed state for the publication surface (spike slice 5b, HAP-001):
//! where this device keeps the product work area (HAP-001-R7, D1: the
//! product's application-data directory, never inside a workspace root)
//! and the device asset paths the dev-mode local-directory provider
//! materializes asset roots in (HAP-001 § Per device; D8's per-OS default,
//! derived under the same application-data directory and never written
//! down). Both are device configuration only: never a record, an event, or
//! an IPC payload value (HAP-001-R5).
//!
//! Neither directory is opened here: every command that uses them
//! re-canonicalizes and re-checks them against the active workspace root
//! at that moment (`WorkAreaRoot::open`, `DeviceAssetPath::open`), which is
//! what HAP-001-R7 and R14 ask for -- a workspace registered over either
//! after configuration refuses the next use, not the one after a restart.
//!
//! The state also carries the **publication surface lock** (R1-001): the
//! approve, publish, and list bodies each hold it for their whole run, so
//! two publications of one identity can never both pass the Catalog's
//! duplicate check and append two records (HAP-001-R23 under concurrency).
//! The shell has one active workspace, so one lock is the per-project lock
//! HAP-001 needs; a second shell process against the same journal is not
//! serialized by it (no single-instance guard exists, as slice 2 records).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The work area's directory name under the application-data directory.
pub const WORK_AREA_DIR: &str = "work-area";

/// The asset roots' parent directory name under the application-data
/// directory; each asset root identity is one subdirectory of it.
pub const ASSET_ROOTS_DIR: &str = "asset-roots";

/// The quarantine directory's name under the application-data directory
/// (spike slice 5d, RCS-001-R10 and its D3): a **sibling** of the work
/// area, never inside it -- HAP-001 fixes the work area's contents as the
/// journal, the recovery entries, and the snapshots, and a quarantined
/// file is none of those.
pub const QUARANTINE_DIR: &str = "quarantine";

/// This shell's managed state for the publication surface: the configured
/// paths, re-validated at every use, and the surface lock.
#[derive(Debug)]
pub struct PublicationState {
    /// The configured product work area.
    pub work_area: PathBuf,
    /// The parent of every device asset path: `<asset_roots>/<asset root
    /// id>` is where an asset root's contents are materialized.
    pub asset_roots: PathBuf,
    /// The configured quarantine directory (spike slice 5d): RCS-001's
    /// landing place for content the product holds but never executes,
    /// re-canonicalized and re-checked against the active workspace at
    /// every use like the work area.
    pub quarantine: PathBuf,
    /// The publication surface lock: held across a whole approve, publish,
    /// or list, so the Catalog's replay-then-append is never interleaved.
    surface: Mutex<()>,
}

impl PublicationState {
    /// The spike default (D1, D8): both under `app_local_data_dir`, the
    /// same area the approval store and RCS-001-R10's quarantine directory
    /// live in.
    #[must_use]
    pub fn under(app_local_data_dir: &Path) -> Self {
        Self {
            work_area: app_local_data_dir.join(WORK_AREA_DIR),
            asset_roots: app_local_data_dir.join(ASSET_ROOTS_DIR),
            quarantine: app_local_data_dir.join(QUARANTINE_DIR),
            surface: Mutex::new(()),
        }
    }

    /// Take the publication surface lock for the duration of one command
    /// body.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    pub fn lock_surface(&self) -> MutexGuard<'_, ()> {
        self.surface
            .lock()
            .expect("publication surface mutex poisoned by a prior panic")
    }
}

#[cfg(test)]
mod tests {
    use super::{ASSET_ROOTS_DIR, PublicationState, QUARANTINE_DIR, WORK_AREA_DIR};
    #[cfg(unix)]
    use omnifrons_adapters::LocalDirBlobStore;
    #[cfg(unix)]
    use omnifrons_app::{WorkspaceRoot, blob_store::DeviceAssetPath};
    #[cfg(unix)]
    use omnifrons_domain::publication::AssetRootId;

    #[test]
    fn the_default_paths_sit_under_the_application_data_directory() {
        let base = std::path::Path::new("/tmp/app-data");
        let state = PublicationState::under(base);
        assert_eq!(state.work_area, base.join(WORK_AREA_DIR));
        assert_eq!(state.asset_roots, base.join(ASSET_ROOTS_DIR));
        assert_eq!(WORK_AREA_DIR, "work-area");
        assert_eq!(ASSET_ROOTS_DIR, "asset-roots");
        // Spike slice 5d: the quarantine directory is a sibling of the
        // work area, never inside it (RCS-001 D3).
        assert_eq!(state.quarantine, base.join(QUARANTINE_DIR));
        assert_eq!(QUARANTINE_DIR, "quarantine");
        assert!(
            !state.quarantine.starts_with(&state.work_area),
            "the quarantine directory is a sibling of the work area, not part of it"
        );
    }

    /// The surface lock is one lock: a second take waits for the first
    /// guard to drop (proven by `try_lock` failing while a guard is held).
    #[test]
    fn the_surface_lock_excludes_a_second_holder() {
        let state = PublicationState::under(std::path::Path::new("/tmp/app-data"));
        let guard = state.lock_surface();
        assert!(state.surface.try_lock().is_err(), "held");
        drop(guard);
        assert!(state.surface.try_lock().is_ok(), "released");
    }

    /// A nonblocking provider cleanup must not stall the actual shell
    /// publication surface guard when a staging FIFO is encountered.
    #[cfg(unix)]
    #[test]
    fn bound_cleanup_returns_for_a_fifo_while_the_publication_surface_is_held() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::sync::mpsc::sync_channel;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        use std::time::Duration;

        struct TestDir(std::path::PathBuf);
        impl Drop for TestDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "omnifrons-publication-state-fifo-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("owned fixture root");
        let fixture = TestDir(root);
        let workspace_path = fixture.0.join("workspace");
        std::fs::create_dir(&workspace_path).expect("owned workspace");
        let workspace = WorkspaceRoot::new(&workspace_path).expect("workspace");
        let asset_path = DeviceAssetPath::open(&fixture.0.join("assets/main"), &[&workspace])
            .expect("validated provider root");
        std::fs::set_permissions(asset_path.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only provider root");
        let fifo = asset_path
            .path()
            .join(format!(".{}.42-7.part", "ab".repeat(32)));
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("owned FIFO fixture command")
                .success(),
            "mkfifo creates the owned fixture"
        );
        std::fs::set_permissions(&fifo, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only FIFO");

        let provider = LocalDirBlobStore::open(&asset_path, AssetRootId::new("main").expect("id"));
        let state = Arc::new(PublicationState::under(&fixture.0));
        let (done, received) = sync_channel(1);
        let (release, continue_worker) = sync_channel(1);
        let worker_state = Arc::clone(&state);
        let worker = std::thread::spawn(move || {
            let _surface_guard = worker_state.lock_surface();
            done.send(provider.cleanup_abandoned_staging())
                .expect("test receiver remains available");
            continue_worker
                .recv_timeout(Duration::from_secs(1))
                .expect("test releases the guarded worker");
        });

        let report = received
            .recv_timeout(Duration::from_secs(1))
            .expect("FIFO cleanup returns while the real surface guard is held");
        assert!(
            state.surface.try_lock().is_err(),
            "real surface guard remains held"
        );
        release.send(()).expect("worker awaits release");
        worker.join().expect("bounded worker exits");
        assert_eq!(report.inspected, 1);
        assert_eq!(report.retained, 1);
        assert_eq!(report.removed, 0);
        assert_eq!(report.failures, 0);
        assert!(fifo.exists(), "the owned FIFO remains");
    }
}
