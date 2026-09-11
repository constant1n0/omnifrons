//! The Catalog repair commands (spike slice 5e, HAP-001-R23, R39, R40):
//! `catalog_repair_preview` and `catalog_repair`, the two halves of the
//! discipline the guidance installer already proved in this repository --
//! **preview, digest binding, explicit apply** (HAP-001 D18, spike slice
//! 5c).
//!
//! Slice 5b left a `Corrupt` `.omnifrons/catalog.jsonl` fail-closed and
//! repaired by hand. The rules are unchanged and live in
//! `omnifrons_app::catalog_repair`, which also says why the third one --
//! a `record` whose `catalogId` disagrees with its identity -- is
//! **refused rather than implemented**: it is the only line registering
//! that artifact, and HAP-001-R39 makes MRP-001's tombstone kind
//! `artifact` the one way a registered artifact is deleted.
//!
//! **Both halves take the publication surface lock**, and the preview
//! takes it for a different reason than the apply. The apply rewrites the
//! file whose replay-then-append every publication depends on, and also
//! refuses while a run is live, like every other writing command on this
//! surface. The preview writes nothing -- but a read that interleaves
//! with a registration can see a line mid-append and report a corrupt
//! catalog that is not one, which is worse than a view one write old.
//! That is why the "read-only bodies take no lock" exception slice 5c
//! recorded for the guidance surface (R1-009) is *not* followed here: its
//! argument was about staleness, and this is about tearing.

use std::time::SystemTime;

use omnifrons_adapters::JsonlCatalogStore;
use omnifrons_app::catalog_repair::{CatalogRepair as _, CatalogRepairError};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::outbox::ContentDigest;
use tauri::{AppHandle, Manager as _};

use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    CatalogRepairDropDto, CatalogRepairPreviewDto, CatalogRepairRefusalDto, CatalogRepairedDto,
    ShellError, ShellErrorCode,
};
use crate::ipc::publication::{RunActivity, run_active};
use crate::publication_state::PublicationState;
use omnifrons_supervisor::TokioProcessSupervisor;

impl From<CatalogRepairError> for ShellError {
    /// Every way a repair ends without a rewritten catalog, mapped to its
    /// token with a fixed, path-free message.
    fn from(error: CatalogRepairError) -> Self {
        match error {
            CatalogRepairError::Absent => Self::new(
                ShellErrorCode::CatalogUnavailable,
                "the project has no catalog to repair",
            ),
            CatalogRepairError::Unreadable => Self::new(
                ShellErrorCode::CatalogUnavailable,
                "the catalog could not be read",
            ),
            CatalogRepairError::WriteFailed => Self::new(
                ShellErrorCode::CatalogUnavailable,
                "the catalog could not be written",
            ),
            CatalogRepairError::Changed => Self::new(
                ShellErrorCode::CatalogChanged,
                "the catalog changed since it was previewed; preview it again",
            ),
            CatalogRepairError::Refused(_) => Self::new(
                ShellErrorCode::RepairRefused,
                "a line of the catalog registers an artifact and needs an owner decision, not a \
                 repair",
            ),
            CatalogRepairError::NothingToRepair => {
                Self::new(ShellErrorCode::RepairRefused, "the catalog needs no repair")
            }
            CatalogRepairError::Unrepairable => Self::new(
                ShellErrorCode::RepairRefused,
                "these rules would not yield a readable catalog",
            ),
            CatalogRepairError::CopyFailed => Self::new(
                ShellErrorCode::WorkAreaInvalid,
                "the pre-repair copy could not be written to the product work area",
            ),
        }
    }
}

/// What a repair of the active project's Catalog would drop, and the
/// digest [`catalog_repair_for`] has to be given back.
///
/// # Errors
///
/// `catalog-unavailable` when the project has no catalog or it cannot be
/// read.
pub fn catalog_repair_preview_for(
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
) -> Result<CatalogRepairPreviewDto, ShellError> {
    // Not for staleness -- for tearing: a replay racing an append reads a
    // half-written line and calls the catalog corrupt.
    let _surface = publication.lock_surface();
    let store = JsonlCatalogStore::open(workspace)?;
    let plan = store.preview()?;
    Ok(CatalogRepairPreviewDto {
        sha256: plan.sha256.to_hex(),
        repairable: plan.is_repairable(),
        drops: plan
            .drops
            .iter()
            .map(|drop| CatalogRepairDropDto {
                line: drop.line,
                rule: drop.rule.into(),
                publication_id: drop.publication_id.map(|id| id.to_hex()),
            })
            .collect(),
        refusals: plan
            .refusals
            .iter()
            .map(|refusal| CatalogRepairRefusalDto {
                line: refusal.line,
                reason: refusal.refusal.into(),
            })
            .collect(),
        kept_records: u32::try_from(plan.kept_records).unwrap_or(u32::MAX),
    })
}

/// Apply the repair to the bytes whose digest is `sha256`, after copying
/// the original into the product work area.
///
/// # Errors
///
/// `invalid-request` for a digest that is not 64 hex characters;
/// `run-active` while a supervised process runs; `work-area-invalid` when
/// the work area fails its check or the pre-repair copy cannot be
/// written; `catalog-changed` when the file is no longer those bytes;
/// `repair-refused` when a line may not be dropped or nothing may be;
/// `catalog-unavailable` for a catalog that cannot be read or written.
pub fn catalog_repair_for(
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    sha256: &str,
    now: SystemTime,
) -> Result<CatalogRepairedDto, ShellError> {
    // The Catalog's replay-then-append must not interleave with this
    // rewrite, so the apply holds the same surface lock a publication does.
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let expected = ContentDigest::from_hex(sha256).ok_or_else(|| {
        ShellError::new(
            ShellErrorCode::InvalidRequest,
            "the digest is not 64 hex characters",
        )
    })?;
    // HAP-001-R7: the work area is checked before the copy is written
    // into it.
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut store = JsonlCatalogStore::open(workspace)?;
    let outcome = store.repair(&work_area, &expected, now)?;
    Ok(CatalogRepairedDto {
        original_sha256: outcome.original_sha256.to_hex(),
        sha256: outcome.sha256.to_hex(),
        dropped_lines: u32::try_from(outcome.dropped_lines).unwrap_or(u32::MAX),
        kept_records: u32::try_from(outcome.kept_records).unwrap_or(u32::MAX),
    })
}

/// What a repair of the active project's Catalog would drop (HAP-001-R23,
/// R39, R40).
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`catalog_repair_preview_for`]'s errors.
#[tauri::command]
pub async fn catalog_repair_preview(app: AppHandle) -> Result<CatalogRepairPreviewDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        catalog_repair_preview_for(&workspace, &publication)
    })
    .await
    .expect("the blocking catalog-repair-preview task panicked")
}

/// Apply that repair to the bytes `sha256` names.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`catalog_repair_for`]'s errors.
#[tauri::command]
pub async fn catalog_repair(
    app: AppHandle,
    sha256: String,
) -> Result<CatalogRepairedDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        catalog_repair_for(
            &workspace,
            &publication,
            &supervisor,
            &sha256,
            SystemTime::now(),
        )
    })
    .await
    .expect("the blocking catalog-repair task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    use omnifrons_adapters::JsonlCatalogStore;
    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::catalog_store::CatalogStoreError;

    use omnifrons_app::catalog_repair::{CatalogRepairError, RepairRefusal};

    use super::{catalog_repair_for, catalog_repair_preview_for};
    use crate::ipc::dto::{RepairRefusalTag, RepairRuleTag, ShellError, ShellErrorCode};
    use crate::ipc::publication::RunActivity;
    use crate::publication_state::PublicationState;

    struct Idle;
    impl RunActivity for Idle {
        fn any_running(&self) -> bool {
            false
        }
    }

    struct Busy;
    impl RunActivity for Busy {
        fn any_running(&self) -> bool {
            true
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-catalog-repair-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
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

    fn id(prefix: &str) -> String {
        prefix.repeat(32)
    }

    fn record_line(publication_id: &str, catalog_id: &str, name: &str) -> String {
        format!(
            r#"{{"event":"record","schema":1,"record":{{"publicationId":"{publication_id}","catalogId":"{catalog_id}","assetRootId":"main","class":"generated-heavy","type":"pdf","size":3,"digest":{{"algorithm":"sha256","hex":"{publication_id}"}},"names":["{name}"],"relationships":[],"provenance":{{"scopeId":"{publication_id}","producer":{{"kind":"unattributed","foundUnderRunId":null}},"transitions":[]}},"provider":{{"adapterId":"local-dir","locator":null,"confirmationKind":null,"state":"pending","reason":null}},"state":"registered","recordVersion":1}}}}"#
        )
    }

    struct Fixture {
        project: TempDir,
        device: TempDir,
        publication: PublicationState,
    }

    impl Fixture {
        fn new(label: &str, lines: &[String]) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            let path = project.path().join(JsonlCatalogStore::FILE_PATH);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, lines.join("\n") + "\n").expect("catalog");
            let publication = PublicationState::under(device.path());
            Self {
                project,
                device,
                publication,
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            WorkspaceRoot::new(self.project.path()).expect("workspace")
        }

        fn catalog(&self) -> String {
            std::fs::read_to_string(self.project.path().join(JsonlCatalogStore::FILE_PATH))
                .expect("read")
        }

        fn now() -> SystemTime {
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_725_782_401)
        }
    }

    /// A corrupt catalog is now its own code: `catalog-unavailable` says
    /// "could not be read at all" and has no repair to offer, while
    /// `catalog-corrupt` says "these lines" and does.
    #[test]
    fn a_corrupt_catalog_is_distinct_from_an_unavailable_one() {
        assert_eq!(
            ShellError::from(CatalogStoreError::Corrupt).code,
            ShellErrorCode::CatalogCorrupt,
        );
        assert_eq!(
            ShellError::from(CatalogStoreError::Unreadable).code,
            ShellErrorCode::CatalogUnavailable,
        );
        assert_eq!(
            ShellError::from(CatalogStoreError::WriteFailed).code,
            ShellErrorCode::CatalogUnavailable,
        );
    }

    /// Every way a repair ends without a rewritten catalog, and the code
    /// and message each renders. Five of these eight arms had no test of
    /// their own, so a code could have moved between them unnoticed --
    /// and `repair-refused` carries three different messages, which is
    /// the whole argument for not adding two more codes.
    #[test]
    fn every_catalog_repair_error_maps_to_its_code_with_a_slash_free_message() {
        for (error, code, message) in [
            (
                CatalogRepairError::Absent,
                ShellErrorCode::CatalogUnavailable,
                "the project has no catalog to repair",
            ),
            (
                CatalogRepairError::Unreadable,
                ShellErrorCode::CatalogUnavailable,
                "the catalog could not be read",
            ),
            (
                CatalogRepairError::WriteFailed,
                ShellErrorCode::CatalogUnavailable,
                "the catalog could not be written",
            ),
            (
                CatalogRepairError::Changed,
                ShellErrorCode::CatalogChanged,
                "the catalog changed since it was previewed; preview it again",
            ),
            (
                CatalogRepairError::Refused(RepairRefusal::CatalogIdMismatch),
                ShellErrorCode::RepairRefused,
                "a line of the catalog registers an artifact and needs an owner decision, not a \
                 repair",
            ),
            (
                CatalogRepairError::NothingToRepair,
                ShellErrorCode::RepairRefused,
                "the catalog needs no repair",
            ),
            (
                CatalogRepairError::Unrepairable,
                ShellErrorCode::RepairRefused,
                "these rules would not yield a readable catalog",
            ),
            (
                CatalogRepairError::CopyFailed,
                ShellErrorCode::WorkAreaInvalid,
                "the pre-repair copy could not be written to the product work area",
            ),
        ] {
            let rendered = ShellError::from(error);
            assert_eq!(rendered.code, code, "{error:?}");
            assert_eq!(rendered.message, message, "{error:?}");
            assert!(!rendered.message.contains('/'), "{}", rendered.message);
            assert!(rendered.detail.is_none(), "{error:?}");
        }
        // Every refusal renders the one `repair-refused` message: the
        // reason is the preview's to report, line by line, and an apply
        // that refuses says only that it refused.
        for refusal in RepairRefusal::ALL {
            assert_eq!(
                ShellError::from(CatalogRepairError::Refused(refusal)).code,
                ShellErrorCode::RepairRefused,
                "{refusal:?}",
            );
        }
    }

    /// The preview names the lines by number and the digest to send back;
    /// it never echoes a line's content, which is untrusted synchronized
    /// text (HAP-001-R40, RCS-001-R14).
    #[test]
    fn the_preview_names_the_lines_it_would_drop_and_the_digest_to_send_back() {
        let a = id("aa");
        let fixture = Fixture::new(
            "preview",
            &[
                record_line(&a, &format!("main/{a}"), "first.pdf"),
                record_line(&a, &format!("main/{a}"), "second.pdf"),
            ],
        );
        let preview = catalog_repair_preview_for(&fixture.workspace(), &fixture.publication)
            .expect("preview");
        assert!(preview.repairable);
        assert_eq!(preview.drops.len(), 1);
        assert_eq!(preview.drops[0].line, 2);
        assert_eq!(preview.drops[0].rule, RepairRuleTag::DuplicateRecord);
        assert_eq!(preview.drops[0].publication_id.as_deref(), Some(a.as_str()));
        assert!(preview.refusals.is_empty());
        assert_eq!(preview.kept_records, 1);
        assert_eq!(preview.sha256.len(), 64);
    }

    /// The apply is bound to that digest and reports what it dropped.
    #[test]
    fn the_repair_applies_the_preview_and_reports_what_it_dropped() {
        let a = id("aa");
        let fixture = Fixture::new(
            "apply",
            &[
                record_line(&a, &format!("main/{a}"), "first.pdf"),
                record_line(&a, &format!("main/{a}"), "second.pdf"),
            ],
        );
        let workspace = fixture.workspace();
        let preview =
            catalog_repair_preview_for(&workspace, &fixture.publication).expect("preview");
        let repaired = catalog_repair_for(
            &workspace,
            &fixture.publication,
            &Idle,
            &preview.sha256,
            Fixture::now(),
        )
        .expect("repair");
        assert_eq!(repaired.original_sha256, preview.sha256);
        assert_eq!(repaired.dropped_lines, 1);
        assert_eq!(repaired.kept_records, 1);
        assert_ne!(repaired.sha256, repaired.original_sha256);
        assert!(
            !catalog_repair_preview_for(&workspace, &fixture.publication)
                .expect("preview again")
                .repairable,
            "and there is nothing left to repair",
        );
        // The copy of the original is in the work area, outside the
        // synchronized namespace.
        let copies: Vec<PathBuf> = std::fs::read_dir(
            fixture
                .device
                .path()
                .join(crate::publication_state::WORK_AREA_DIR)
                .join(omnifrons_app::work_area::RECOVERY_DIR),
        )
        .expect("recovery")
        .map(|entry| entry.expect("entry").path())
        .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
    }

    /// A digest that is not the catalog's own is `catalog-changed`, and
    /// nothing is rewritten.
    #[test]
    fn a_repair_bound_to_other_bytes_is_catalog_changed() {
        let a = id("aa");
        let fixture = Fixture::new(
            "changed",
            &[
                record_line(&a, &format!("main/{a}"), "first.pdf"),
                record_line(&a, &format!("main/{a}"), "second.pdf"),
            ],
        );
        let before = fixture.catalog();
        let error = catalog_repair_for(
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &id("cc"),
            Fixture::now(),
        )
        .expect_err("stale binding");
        assert_eq!(error.code, ShellErrorCode::CatalogChanged);
        assert_eq!(fixture.catalog(), before);
    }

    /// A `catalogId` that disagrees with its identity is the only line
    /// registering that artifact: `repair-refused`, nothing rewritten
    /// (HAP-001-R39).
    #[test]
    fn a_line_that_registers_an_artifact_is_repair_refused() {
        let a = id("aa");
        let fixture = Fixture::new(
            "refused",
            &[record_line(&a, &format!("other/{a}"), "report.pdf")],
        );
        let workspace = fixture.workspace();
        let before = fixture.catalog();
        let preview =
            catalog_repair_preview_for(&workspace, &fixture.publication).expect("preview");
        assert!(!preview.repairable);
        assert_eq!(preview.refusals.len(), 1);
        assert_eq!(
            preview.refusals[0].reason,
            RepairRefusalTag::CatalogIdMismatch
        );
        let error = catalog_repair_for(
            &workspace,
            &fixture.publication,
            &Idle,
            &preview.sha256,
            Fixture::now(),
        )
        .expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::RepairRefused);
        assert_eq!(fixture.catalog(), before);
    }

    /// The apply is a write on the publication surface: frozen while a
    /// supervised process runs, like every other one (spike slice 5b's
    /// R1-001 default).
    #[test]
    fn a_repair_is_refused_while_a_run_is_active() {
        let a = id("aa");
        let fixture = Fixture::new(
            "run-active",
            &[
                record_line(&a, &format!("main/{a}"), "first.pdf"),
                record_line(&a, &format!("main/{a}"), "second.pdf"),
            ],
        );
        let workspace = fixture.workspace();
        let before = fixture.catalog();
        let preview = catalog_repair_preview_for(&workspace, &fixture.publication)
            .expect("the preview is not frozen");
        let error = catalog_repair_for(
            &workspace,
            &fixture.publication,
            &Busy,
            &preview.sha256,
            Fixture::now(),
        )
        .expect_err("frozen");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert_eq!(fixture.catalog(), before);
    }
}
