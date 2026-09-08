//! The filesystem publication adapters (spike slice 5b): `Sha2Hasher`
//! (the real SHA-256 behind every identity), `LocalDirBlobStore` (the
//! dev-mode local-directory provider), `JsonlCatalogStore` (the project's
//! `.omnifrons/catalog.jsonl`), `JsonlPublicationJournal` (the work area's
//! `journal/publications.jsonl`, owner-only), and `FsOutboxEntryOps` (file
//! identity by handle, removal relative to the directory handle). The
//! shared port contracts run first; the adapter-specific cases follow,
//! and one end-to-end transaction over the real adapters proves the
//! HAP-001-R18 swap case with a recovery entry.
//!
//! Platform notes: the symbolic-link fixtures are unix only, gated per
//! test (creating one on Windows needs a privilege the CI runner lacks);
//! the by-handle identity comparison is unix only and disclosed as
//! `Unverifiable` elsewhere; owner-only modes are asserted on unix only.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::{
    FsOutboxEntryOps, JsonlCatalogStore, JsonlPublicationJournal, LocalDirBlobStore, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::blob_store::{
    BlobStoreError, BlobStorePort, DestinationError, DeviceAssetPath, PublishMode,
};
use omnifrons_app::catalog_store::{CatalogStore, CatalogStoreError};
use omnifrons_app::content_hasher::{
    ContentHasher, derive_artifact_approval_id, derive_project_identity,
    derive_publication_identity,
};
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::contract::publication::{
    blob_store_contract, catalog_store_contract, publication_journal_contract, sample_record,
};
use omnifrons_app::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
#[cfg(unix)] // only the unix swap test matches on the error
use omnifrons_app::publication::PublishError;
use omnifrons_app::publication::{CandidateSource, PublishPorts, publish};
use omnifrons_app::publication_journal::{JournalError, PublicationJournal};
use omnifrons_app::run_outbox::DirectoryHandle;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType, RunId};
use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactState, AssetRootId, DisplayName, PublicationIdentity,
};
use sha2::{Digest as _, Sha256};

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-publication-fs-test-{}-{label}-{n}",
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

fn asset_root() -> AssetRootId {
    AssetRootId::new("main").expect("valid")
}

fn open_provider(base: &TempDir, project: &TempDir) -> LocalDirBlobStore {
    let device_path =
        DeviceAssetPath::open(&base.path().join("roots/main"), &[&project.workspace()])
            .expect("the device asset path opens outside the project");
    LocalDirBlobStore::open(&device_path, asset_root())
}

// -- the hasher --

#[test]
fn sha2_hasher_matches_sha2_over_bytes_and_streams() {
    let hasher = Sha2Hasher::new();
    let bytes = b"%PDF-1.7\nomnifrons\n";
    let expected: [u8; 32] = Sha256::digest(bytes).into();
    assert_eq!(hasher.sha256(bytes), Sha256Digest(expected));
    let (streamed, size) = hasher
        .digest_reader(&mut std::io::Cursor::new(bytes))
        .expect("in-memory");
    assert_eq!(streamed, Sha256Digest(expected));
    assert_eq!(size, bytes.len() as u64);
    let empty: [u8; 32] = Sha256::digest(b"").into();
    assert_eq!(hasher.sha256(b""), Sha256Digest(empty));
}

// -- the shared contracts --

#[test]
fn local_dir_blob_store_satisfies_the_blob_store_contract() {
    let base = TempDir::new("blob-contract");
    let project = TempDir::new("blob-contract-project");
    blob_store_contract(|| open_provider(&base, &project));
}

#[test]
fn jsonl_catalog_store_satisfies_the_catalog_store_contract() {
    let project = TempDir::new("catalog-contract");
    catalog_store_contract(|| JsonlCatalogStore::open(&project.workspace()).expect("opens"));
}

#[test]
fn jsonl_journal_satisfies_the_publication_journal_contract() {
    let base = TempDir::new("journal-contract");
    let project = TempDir::new("journal-contract-project");
    let work_area =
        WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("opens");
    publication_journal_contract(|| JsonlPublicationJournal::open(&work_area).expect("opens"));
}

// -- the local-directory provider (dev-mode only) --

/// The adapter's declared capabilities: `copy` only, confirmation kind
/// `no remote` (it never observes one), no object limit, quota unknown,
/// no native placeholder.
#[test]
fn local_dir_capabilities_are_copy_only_with_no_remote() {
    let base = TempDir::new("caps");
    let project = TempDir::new("caps-project");
    let provider = open_provider(&base, &project);
    let capabilities = provider.capabilities();
    assert_eq!(capabilities.adapter_id, LocalDirBlobStore::ADAPTER_ID);
    assert_eq!(LocalDirBlobStore::ADAPTER_ID, "local-dir");
    assert_eq!(capabilities.publish_mode, PublishMode::Copy);
    assert_eq!(capabilities.confirmation_kind, "no remote");
    assert_eq!(capabilities.object_limit, None);
    assert!(!capabilities.native_placeholder);
    let locator = omnifrons_domain::publication::ProviderLocator::new(format!(
        "local-dir:main/{}",
        "ab".repeat(32)
    ));
    assert_eq!(
        provider.confirm(&locator).expect("confirm runs"),
        None,
        "never provider-synced: there is no remote"
    );
}

/// A committed copy lands at `<root>/<publication hex>` with the locator
/// `local-dir:<asset root id>/<publication hex>`, is owner-only on unix,
/// and nothing is visible at the final name before `commit`.
#[test]
fn local_dir_commit_places_the_copy_at_the_publication_hex_with_the_documented_locator() {
    let base = TempDir::new("commit");
    let project = TempDir::new("commit-project");
    let provider = open_provider(&base, &project);
    let identity = PublicationIdentity(Sha256Digest([0xab; 32]));
    let final_path = base.path().join("roots/main").join("ab".repeat(32));

    let mut staged = provider.stage(&identity).expect("stage");
    staged.write_all(b"%PDF-1.7\nbytes\n").expect("write");
    assert!(
        !final_path.exists(),
        "nothing at the final name before commit"
    );
    let locator = staged.commit().expect("commit");
    assert_eq!(
        locator.as_str(),
        format!("local-dir:main/{}", "ab".repeat(32))
    );
    assert_eq!(
        std::fs::read(&final_path).expect("the copy"),
        b"%PDF-1.7\nbytes\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&final_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the copy is owner-only");
    }
    assert!(
        std::fs::read_dir(base.path().join("roots/main"))
            .expect("list")
            .all(|entry| !entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".part")),
        "no staging file is left behind"
    );
}

/// An aborted stage leaves nothing behind, at the final name or as a
/// staging file.
#[test]
fn local_dir_abort_leaves_nothing_behind() {
    let base = TempDir::new("abort");
    let project = TempDir::new("abort-project");
    let provider = open_provider(&base, &project);
    let identity = PublicationIdentity(Sha256Digest([0xcd; 32]));
    let mut staged = provider.stage(&identity).expect("stage");
    staged.write_all(b"never").expect("write");
    staged.abort();
    let entries: Vec<OsString> = std::fs::read_dir(base.path().join("roots/main"))
        .expect("list")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert!(entries.is_empty(), "got {entries:?}");
}

/// A locator another adapter issued -- or one naming another asset root
/// -- is refused, never resolved to a path.
#[test]
fn local_dir_refuses_a_foreign_locator() {
    let base = TempDir::new("foreign");
    let project = TempDir::new("foreign-project");
    let provider = open_provider(&base, &project);
    for text in [
        "proton:main/abcd",
        &format!("local-dir:other/{}", "ab".repeat(32)),
        "local-dir:main/../escape",
        "local-dir:main/abcd",
    ] {
        let locator = omnifrons_domain::publication::ProviderLocator::new(text);
        assert!(
            matches!(
                provider.open_published(&locator).map(|_| ()),
                Err(BlobStoreError::ForeignLocator)
            ),
            "{text} must be refused as foreign"
        );
        assert_eq!(
            provider.discard(&locator),
            Err(BlobStoreError::ForeignLocator)
        );
    }
}

/// HAP-001-R14 at the adapter: a device asset path inside a registered
/// workspace root is refused before the provider exists.
#[test]
fn a_device_asset_path_inside_the_project_cannot_back_a_provider() {
    let project = TempDir::new("inside");
    let result = DeviceAssetPath::open(&project.path().join("assets"), &[&project.workspace()]);
    assert_eq!(result.err(), Some(DestinationError::InsideWorkspace));
}

// -- the JSONL catalog (D9: under the project's `.omnifrons/`) --

/// The catalog lives at `.omnifrons/catalog.jsonl`; an absent file is an
/// empty catalog, and nothing is created by a read.
#[test]
fn the_catalog_lives_under_the_projects_omnifrons_namespace() {
    let project = TempDir::new("catalog-path");
    let mut store = JsonlCatalogStore::open(&project.workspace()).expect("opens");
    assert_eq!(JsonlCatalogStore::FILE_PATH, ".omnifrons/catalog.jsonl");
    assert!(store.list().expect("empty").is_empty());
    assert!(
        !project.path().join(".omnifrons/catalog.jsonl").exists(),
        "a read creates nothing"
    );
    let publication = PublicationIdentity(Sha256Digest([0x11; 32]));
    store
        .register(sample_record(publication, "main", "report.pdf"))
        .expect("register");
    let text = std::fs::read_to_string(project.path().join(".omnifrons/catalog.jsonl"))
        .expect("the catalog file now exists");
    assert_eq!(text.lines().count(), 1, "one line per record");
    let line: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("line")).expect("json");
    assert_eq!(line["schema"], 1);
    assert_eq!(line["event"], "record");
    assert_eq!(line["record"]["publicationId"], "11".repeat(32));
    assert_eq!(
        line["record"]["catalogId"],
        format!("main/{}", "11".repeat(32))
    );
    assert_eq!(line["record"]["digest"]["algorithm"], "sha256");
    assert_eq!(line["record"]["digest"]["hex"], "ab".repeat(32));
    assert_eq!(line["record"]["provider"]["state"], "pending");
    assert_eq!(line["record"]["provider"]["reason"], "no remote");
    assert_eq!(line["record"]["state"], "registered");
    assert_eq!(line["record"]["recordVersion"], 1);
    assert_eq!(line["record"]["provenance"]["producer"]["kind"], "run");
    assert_eq!(line["record"]["provenance"]["producer"]["runId"], "run-1");
    let dumped = text.to_lowercase();
    assert!(
        !dumped.contains(&project.path().to_string_lossy().to_lowercase()),
        "a record carries no device path"
    );
}

/// A truncated or malformed line, or one of another schema, fails the
/// whole read as `Corrupt`; never a partial result.
#[test]
fn a_corrupt_catalog_line_fails_the_whole_read() {
    let project = TempDir::new("catalog-corrupt");
    let mut store = JsonlCatalogStore::open(&project.workspace()).expect("opens");
    let publication = PublicationIdentity(Sha256Digest([0x11; 32]));
    store
        .register(sample_record(publication, "main", "report.pdf"))
        .expect("register");
    let path = project.path().join(".omnifrons/catalog.jsonl");
    let good = std::fs::read_to_string(&path).expect("read");

    std::fs::write(&path, format!("{good}{{\"schema\":1,\"event\":\"rec")).expect("truncate");
    assert_eq!(store.list(), Err(CatalogStoreError::Corrupt));

    std::fs::write(&path, good.replace("\"schema\":1", "\"schema\":2")).expect("reschema");
    assert_eq!(store.list(), Err(CatalogStoreError::Corrupt));

    std::fs::write(&path, format!("{good}not json\n")).expect("garbage");
    assert_eq!(store.find(&publication), Err(CatalogStoreError::Corrupt));
}

// -- the JSONL journal (the work area's `journal/publications.jsonl`) --

/// The journal file is created owner-only under the work area's
/// `journal/` directory on unix (the slice-2 store's discipline).
#[cfg(unix)]
#[test]
fn the_journal_file_is_owner_only_under_the_work_areas_journal_directory() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = TempDir::new("journal-mode");
    let project = TempDir::new("journal-mode-project");
    let work_area =
        WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("opens");
    let _journal = JsonlPublicationJournal::open(&work_area).expect("opens");
    let path = work_area
        .journal_dir()
        .join(JsonlPublicationJournal::FILE_NAME);
    assert_eq!(JsonlPublicationJournal::FILE_NAME, "publications.jsonl");
    let mode = std::fs::metadata(&path)
        .expect("created on open")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

/// A corrupt journal line fails the whole replay.
#[test]
fn a_corrupt_journal_line_fails_the_whole_replay() {
    let base = TempDir::new("journal-corrupt");
    let project = TempDir::new("journal-corrupt-project");
    let work_area =
        WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("opens");
    let journal = JsonlPublicationJournal::open(&work_area).expect("opens");
    let path = work_area
        .journal_dir()
        .join(JsonlPublicationJournal::FILE_NAME);
    std::fs::write(&path, "{\"schema\":1,\"event\":\"appro\n").expect("truncated line");
    assert_eq!(journal.replay(), Err(JournalError::Corrupt));
    std::fs::write(&path, "{\"schema\":7,\"event\":\"approved\"}\n").expect("other schema");
    assert_eq!(journal.replay(), Err(JournalError::Corrupt));
}

// -- entry ops: identity by handle, removal relative to the directory --

fn open_dir(path: &Path) -> DirectoryHandle {
    DirectoryHandle::new(File::open(path).expect("open dir"))
}

/// Unix: the path names the handle's file -> `SameFile`; a different file
/// swapped in -> `DifferentFile`; a symbolic link planted at the name is
/// the link's own identity, never followed -> `DifferentFile`; nothing at
/// the name -> `Missing`. Windows: `Unverifiable` (disclosed).
#[test]
fn entry_identity_follows_the_handle_not_the_name() {
    let dir = TempDir::new("identity");
    std::fs::write(dir.path().join("a.pdf"), b"a").expect("write a");
    let handle = File::open(dir.path().join("a.pdf")).expect("open a");
    let ops = FsOutboxEntryOps::new();
    let dir_handle = open_dir(dir.path());
    let name = OsStr::new("a.pdf");

    let same = ops.identity(&dir_handle, dir.path(), name, &handle);
    #[cfg(unix)]
    assert_eq!(same, EntryIdentity::SameFile);
    #[cfg(not(unix))]
    assert_eq!(same, EntryIdentity::Unverifiable);

    #[cfg(unix)]
    {
        std::fs::remove_file(dir.path().join("a.pdf")).expect("unlink");
        assert_eq!(
            ops.identity(&dir_handle, dir.path(), name, &handle),
            EntryIdentity::Missing
        );
        std::fs::write(dir.path().join("a.pdf"), b"a").expect("a different file, same bytes");
        assert_eq!(
            ops.identity(&dir_handle, dir.path(), name, &handle),
            EntryIdentity::DifferentFile,
            "same bytes at the name is still a different file"
        );
        std::fs::remove_file(dir.path().join("a.pdf")).expect("unlink");
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("a.pdf"))
            .expect("plant a link");
        assert_eq!(
            ops.identity(&dir_handle, dir.path(), name, &handle),
            EntryIdentity::DifferentFile,
            "a link at the name is never followed"
        );
    }
}

/// Unix: `unlink` removes the entry through the directory handle and
/// refuses a directory.
#[cfg(unix)]
#[test]
fn entry_unlink_removes_the_named_file_relative_to_the_directory_handle() {
    let dir = TempDir::new("unlink");
    std::fs::write(dir.path().join("a.pdf"), b"a").expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let ops = FsOutboxEntryOps::new();
    let dir_handle = open_dir(dir.path());
    ops.unlink(&dir_handle, dir.path(), OsStr::new("a.pdf"))
        .expect("unlink");
    assert!(!dir.path().join("a.pdf").exists());
    assert!(
        ops.unlink(&dir_handle, dir.path(), OsStr::new("sub"))
            .is_err(),
        "a directory is never removed as an entry"
    );
    assert!(dir.path().join("sub").is_dir());
}

// -- end to end over the real adapters --

struct RealFixture {
    project: TempDir,
    base: TempDir,
    work_area: WorkAreaRoot,
    run_dir: PathBuf,
}

impl RealFixture {
    fn new(label: &str) -> Self {
        let project = TempDir::new(label);
        let base = TempDir::new(&format!("{label}-device"));
        let work_area = WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()])
            .expect("work area");
        let run_dir = project.path().join(".omnifrons/outbox/run-1");
        std::fs::create_dir_all(&run_dir).expect("run dir");
        Self {
            project,
            base,
            work_area,
            run_dir,
        }
    }

    fn approval(&self, name: &str, bytes: &[u8]) -> ArtifactApproval {
        let hasher = Sha2Hasher::new();
        let project = derive_project_identity(&hasher, &self.project.workspace());
        let digest = hasher.sha256(bytes);
        let publication_id = derive_publication_identity(&hasher, &project, &digest);
        let approved_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_399);
        let run_id = RunId::new("run-1").expect("valid");
        ArtifactApproval {
            approval_id: derive_artifact_approval_id(&hasher, &publication_id, approved_at),
            publication_id,
            project,
            run_id: run_id.clone(),
            name: format!("run-1/{name}"),
            display_name: DisplayName::sanitize(name),
            digest,
            size: bytes.len() as u64,
            detected_type: DetectedType::Pdf,
            class: ArtifactClass::GeneratedHeavy,
            attribution: Attribution::Run(run_id),
            asset_root_id: asset_root(),
            adapter_id: None,
            executable_approval: None,
            approver: DeviceLocalUser,
            approved_at,
        }
    }

    fn source(&self, name: &str) -> CandidateSource {
        CandidateSource {
            dir: open_dir(&self.run_dir),
            dir_path: self.run_dir.clone(),
            file_name: OsString::from(name),
            handle: File::open(self.run_dir.join(name)).expect("open entry"),
        }
    }
}

const PDF: &[u8] = b"%PDF-1.7\nomnifrons real adapters\n";

/// The whole transaction over the real adapters: the copy lands in the
/// device asset path, the record in `.omnifrons/catalog.jsonl`, the journal
/// under the work area, and the entry is removed from the outbox only at
/// the end.
#[test]
fn a_publication_over_the_real_adapters_registers_and_removes_the_entry() {
    let fixture = RealFixture::new("real-happy");
    std::fs::write(fixture.run_dir.join("report.pdf"), PDF).expect("entry");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let workspace = fixture.project.workspace();
    let provider = open_provider(&fixture.base, &fixture.project);
    let mut catalog = JsonlCatalogStore::open(&workspace).expect("catalog");
    let mut journal = JsonlPublicationJournal::open(&fixture.work_area).expect("journal");
    let hasher = Sha2Hasher::new();
    let ops = FsOutboxEntryOps::new();
    let clock = FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401));
    let mut states = Vec::new();

    let published = publish(
        PublishPorts {
            hasher: &hasher,
            provider: &provider,
            catalog: &mut catalog,
            journal: &mut journal,
            entry_ops: &ops,
            clock: &clock,
            work_area: &fixture.work_area,
            workspaces: &[&workspace],
        },
        &approval,
        source,
        &mut |event| states.push(event.state),
    )
    .expect("registered");

    assert_eq!(published.state, ArtifactState::Registered);
    let copy = fixture
        .base
        .path()
        .join("roots/main")
        .join(approval.publication_id.to_hex());
    assert_eq!(std::fs::read(&copy).expect("the copy"), PDF);
    assert_eq!(catalog.list().expect("list").len(), 1);
    assert!(
        fixture
            .project
            .path()
            .join(".omnifrons/catalog.jsonl")
            .is_file()
    );
    assert_eq!(
        states,
        vec![ArtifactState::PublishedLocal, ArtifactState::Registered]
    );
    // Unix removes the entry through the directory handle; elsewhere the
    // identity check is unverifiable and the entry stays (disclosed).
    #[cfg(unix)]
    assert!(
        !fixture.run_dir.join("report.pdf").exists(),
        "removed only after both steps"
    );
    #[cfg(not(unix))]
    assert!(fixture.run_dir.join("report.pdf").exists());
    let journal_text = std::fs::read_to_string(
        fixture
            .work_area
            .journal_dir()
            .join(JsonlPublicationJournal::FILE_NAME),
    )
    .expect("journal");
    assert_eq!(
        journal_text.lines().count(),
        3,
        "published-local, registered, cleanup"
    );
}

/// HAP-001-R18 over the real adapters (unix, where identity is compared by
/// handle): a file swapped at the path after the handle was opened renders
/// `outbox-escape`, nothing reaches the device asset path, and the held
/// bytes survive as a recovery entry outside the project.
#[cfg(unix)]
#[test]
fn a_swapped_entry_is_outbox_escape_with_the_approved_bytes_recovered() {
    let fixture = RealFixture::new("real-swap");
    std::fs::write(fixture.run_dir.join("report.pdf"), PDF).expect("entry");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    std::fs::remove_file(fixture.run_dir.join("report.pdf")).expect("unlink");
    std::fs::write(fixture.run_dir.join("report.pdf"), b"%PDF-1.7\nswapped\n").expect("swap");
    let workspace = fixture.project.workspace();
    let provider = open_provider(&fixture.base, &fixture.project);
    let mut catalog = JsonlCatalogStore::open(&workspace).expect("catalog");
    let mut journal = JsonlPublicationJournal::open(&fixture.work_area).expect("journal");
    let hasher = Sha2Hasher::new();
    let ops = FsOutboxEntryOps::new();
    let clock = FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401));

    let error = publish(
        PublishPorts {
            hasher: &hasher,
            provider: &provider,
            catalog: &mut catalog,
            journal: &mut journal,
            entry_ops: &ops,
            clock: &clock,
            work_area: &fixture.work_area,
            workspaces: &[&workspace],
        },
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("outbox-escape");
    let PublishError::OutboxEscape {
        recovery: Some(recovery),
    } = error
    else {
        panic!("expected outbox-escape with a recovery entry, got {error:?}");
    };
    assert_eq!(recovery, approval.digest);
    let recovered = fixture.work_area.recovery_dir().join(recovery.to_hex());
    assert_eq!(std::fs::read(&recovered).expect("recovery entry"), PDF);
    assert!(
        !recovered.starts_with(fixture.project.path()),
        "the recovery entry is outside the project"
    );
    assert!(
        std::fs::read_dir(fixture.base.path().join("roots/main"))
            .expect("list")
            .next()
            .is_none(),
        "nothing reached the device asset path"
    );
    assert!(catalog.list().expect("list").is_empty());
    assert_eq!(
        std::fs::read(fixture.run_dir.join("report.pdf")).expect("the swapped file stays"),
        b"%PDF-1.7\nswapped\n"
    );
}

// -- the catalog's documented corruption branches (R3-005) --

/// A registered catalog and its file text, for the crafted-line cases.
fn registered_catalog(label: &str) -> (TempDir, JsonlCatalogStore, String) {
    let project = TempDir::new(label);
    let mut store = JsonlCatalogStore::open(&project.workspace()).expect("opens");
    let publication = PublicationIdentity(Sha256Digest([0x11; 32]));
    store
        .register(sample_record(publication, "main", "report.pdf"))
        .expect("register");
    let text = std::fs::read_to_string(project.path().join(".omnifrons/catalog.jsonl"))
        .expect("the catalog file");
    (project, store, text)
}

/// Two record lines carrying the same publication identity are corrupt:
/// one record per identity is the invariant (HAP-001-R23), and a log that
/// breaks it fails closed rather than picking one.
#[test]
fn two_records_with_one_publication_identity_are_corrupt() {
    let (project, store, good) = registered_catalog("catalog-dup-record");
    std::fs::write(
        project.path().join(".omnifrons/catalog.jsonl"),
        format!("{good}{good}"),
    )
    .expect("write");
    assert_eq!(store.list(), Err(CatalogStoreError::Corrupt));
}

/// An alias naming a publication no record line registered is corrupt.
#[test]
fn an_alias_naming_no_record_is_corrupt() {
    let (project, store, good) = registered_catalog("catalog-orphan-alias");
    let orphan = format!(
        "{{\"schema\":1,\"event\":\"alias\",\"publicationId\":\"{}\",\"name\":\"x.pdf\",\"at\":{{\"secs\":1,\"nanos\":0}}}}\n",
        "22".repeat(32)
    );
    std::fs::write(
        project.path().join(".omnifrons/catalog.jsonl"),
        format!("{good}{orphan}"),
    )
    .expect("write");
    assert_eq!(store.list(), Err(CatalogStoreError::Corrupt));
}

/// A record whose `catalogId` disagrees with the identity rebuilt from its
/// `assetRootId` and `publicationId` is corrupt.
#[test]
fn a_record_whose_catalog_id_disagrees_with_its_identity_is_corrupt() {
    let (project, store, good) = registered_catalog("catalog-id-mismatch");
    let edited = good.replace("\"catalogId\":\"main/", "\"catalogId\":\"other/");
    assert_ne!(edited, good, "the fixture line carries the catalogId");
    std::fs::write(project.path().join(".omnifrons/catalog.jsonl"), edited).expect("write");
    assert_eq!(store.list(), Err(CatalogStoreError::Corrupt));
}

// -- staging names (R1-003) --

/// Two stages of the same identity write two distinct staging files, so a
/// concurrent writer -- another shell, since one shell serializes its own
/// publications -- never truncates a copy in flight; aborting both leaves
/// nothing.
#[test]
fn local_dir_stages_the_same_identity_twice_under_distinct_staging_names() {
    let base = TempDir::new("stage-twice");
    let project = TempDir::new("stage-twice-project");
    let provider = open_provider(&base, &project);
    let identity = PublicationIdentity(Sha256Digest([0xee; 32]));
    let root = base.path().join("roots/main");

    let mut first = provider.stage(&identity).expect("stage");
    let mut second = provider.stage(&identity).expect("stage again");
    first.write_all(b"one").expect("write");
    second.write_all(b"two").expect("write");
    let staging: Vec<OsString> = std::fs::read_dir(&root)
        .expect("list")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().ends_with(".part"))
        .collect();
    assert_eq!(
        staging.len(),
        2,
        "two staged copies, two staging files, got {staging:?}"
    );
    second.abort();
    first.abort();
    assert!(
        std::fs::read_dir(&root).expect("list").next().is_none(),
        "nothing left behind"
    );
}

// -- a non-regular held handle (R3-007) --

/// Unix: a FIFO opened without blocking as the held handle is refused as
/// `outbox-escape` by the handle's own metadata; nothing is staged and no
/// recovery entry is written. (A FIFO has no Windows equivalent.)
#[cfg(unix)]
#[test]
fn a_fifo_held_handle_is_outbox_escape_with_nothing_staged() {
    let fixture = RealFixture::new("fifo");
    let fifo = fixture.run_dir.join("pipe.pdf");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).expect("mkfifo");
    let fd = nix::fcntl::open(
        &fifo,
        nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_NONBLOCK,
        nix::sys::stat::Mode::empty(),
    )
    .expect("a FIFO with no writer opens without blocking under O_NONBLOCK");
    let handle = File::from(fd);
    let approval = fixture.approval("pipe.pdf", b"");
    let source = CandidateSource {
        dir: open_dir(&fixture.run_dir),
        dir_path: fixture.run_dir.clone(),
        file_name: OsString::from("pipe.pdf"),
        handle,
    };
    let workspace = fixture.project.workspace();
    let provider = open_provider(&fixture.base, &fixture.project);
    let mut catalog = JsonlCatalogStore::open(&workspace).expect("catalog");
    let mut journal = JsonlPublicationJournal::open(&fixture.work_area).expect("journal");
    let hasher = Sha2Hasher::new();
    let ops = FsOutboxEntryOps::new();
    let clock = FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401));

    let error = publish(
        PublishPorts {
            hasher: &hasher,
            provider: &provider,
            catalog: &mut catalog,
            journal: &mut journal,
            entry_ops: &ops,
            clock: &clock,
            work_area: &fixture.work_area,
            workspaces: &[&workspace],
        },
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("a FIFO is not a regular file");
    assert!(
        matches!(error, PublishError::OutboxEscape { recovery: None }),
        "got {error:?}"
    );
    assert!(
        std::fs::read_dir(fixture.base.path().join("roots/main"))
            .expect("list")
            .next()
            .is_none(),
        "nothing staged"
    );
    assert!(catalog.list().expect("list").is_empty());
    assert!(
        std::fs::read_dir(fixture.work_area.recovery_dir())
            .expect("list")
            .next()
            .is_none(),
        "no recovery entry for a non-regular handle"
    );
}
