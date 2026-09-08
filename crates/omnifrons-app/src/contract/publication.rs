//! Fakes and reusable contracts for the publication ports (spike slice
//! 5b): a deterministic `FakeHasher`, an `InMemoryBlobStore` with a knob
//! that corrupts the next committed copy, an `InMemoryCatalogStore` with
//! a knob that fails the next registration, an `InMemoryJournal`, and a
//! `ScriptedEntryOps` -- plus the contracts `omnifrons-adapters`' real
//! stores and provider run against themselves.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType, RunId};
use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactApprovalId, ArtifactState, AssetRootId, CatalogId, CatalogRecord,
    DisplayName, JournalEntry, JournalStep, Producer, ProjectIdentity, Provenance, ProviderLocator,
    ProviderRecord, ProviderState, PublicationIdentity, RECORD_VERSION, StepEntry, StepOutcome,
};

use crate::blob_store::{
    BlobStoreError, BlobStorePort, ProviderCapabilities, ProviderConfirmation, PublishMode,
    QuotaVisibility, StagedObject,
};
use crate::catalog_store::{CatalogStore, CatalogStoreError};
use crate::content_hasher::ContentHasher;
use crate::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use crate::publication_journal::{JournalError, PublicationJournal};
use crate::run_outbox::DirectoryHandle;

/// A deterministic, non-cryptographic [`ContentHasher`] test double: 32
/// bytes derived from an FNV-1a-style fold over the input plus its
/// length. Collision-resistant enough for a test to tell two fixtures
/// apart, and nothing more -- never used outside tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeHasher;

impl FakeHasher {
    fn fold(state: &mut [u64; 4], byte: u8) {
        for (lane, seed) in state.iter_mut().zip([
            0xcbf2_9ce4_8422_2325u64,
            0x1000_0193,
            0x9e37_79b9_7f4a_7c15,
            0xd6e8_feb8_6659_fd93,
        ]) {
            *lane ^= u64::from(byte) ^ seed;
            *lane = lane.wrapping_mul(0x0000_0100_0000_01b3);
            *lane = lane.rotate_left(13);
        }
    }

    fn finish(state: [u64; 4], length: u64) -> Sha256Digest {
        let mut bytes = [0u8; 32];
        for (chunk, lane) in bytes.chunks_mut(8).zip(state) {
            chunk.copy_from_slice(&(lane ^ length).to_be_bytes());
        }
        Sha256Digest(bytes)
    }
}

impl ContentHasher for FakeHasher {
    fn sha256(&self, input: &[u8]) -> Sha256Digest {
        let mut state = [1u64, 2, 3, 4];
        for byte in input {
            Self::fold(&mut state, *byte);
        }
        Self::finish(state, input.len() as u64)
    }

    fn digest_reader(&self, reader: &mut dyn Read) -> std::io::Result<(Sha256Digest, u64)> {
        let mut state = [1u64, 2, 3, 4];
        let mut length: u64 = 0;
        let mut buffer = [0u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            for byte in &buffer[..read] {
                Self::fold(&mut state, *byte);
            }
            length += read as u64;
        }
        Ok((Self::finish(state, length), length))
    }
}

/// An in-memory [`BlobStorePort`] test double: objects live in a map by
/// locator; `corrupt_next_commit` flips one byte of the next committed
/// copy (the HAP-001-R21 fixture), and every stage, abort, and discard is
/// counted.
#[derive(Debug, Clone)]
pub struct InMemoryBlobStore {
    inner: Arc<BlobStoreState>,
}

#[derive(Debug)]
struct BlobStoreState {
    adapter_id: String,
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    staged: AtomicUsize,
    aborted: AtomicUsize,
    discarded: Mutex<Vec<String>>,
    corrupt_next: AtomicBool,
}

impl InMemoryBlobStore {
    /// An empty store whose capabilities name `adapter_id`.
    #[must_use]
    pub fn new(adapter_id: &str) -> Self {
        Self {
            inner: Arc::new(BlobStoreState {
                adapter_id: adapter_id.to_string(),
                objects: Mutex::new(BTreeMap::new()),
                staged: AtomicUsize::new(0),
                aborted: AtomicUsize::new(0),
                discarded: Mutex::new(Vec::new()),
                corrupt_next: AtomicBool::new(false),
            }),
        }
    }

    /// Flip one byte of the next committed copy.
    pub fn corrupt_next_commit(&self) {
        self.inner.corrupt_next.store(true, Ordering::Release);
    }

    /// How many objects were staged so far.
    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.inner.staged.load(Ordering::Acquire)
    }

    /// How many staged objects were aborted.
    #[must_use]
    pub fn aborted(&self) -> usize {
        self.inner.aborted.load(Ordering::Acquire)
    }

    /// Every locator discarded so far.
    ///
    /// # Panics
    ///
    /// Panics if the store's mutex was poisoned by a prior panic.
    #[must_use]
    pub fn discarded(&self) -> Vec<String> {
        self.inner
            .discarded
            .lock()
            .expect("discarded mutex poisoned")
            .clone()
    }

    /// Every object currently held, by locator.
    ///
    /// # Panics
    ///
    /// Panics if the store's mutex was poisoned by a prior panic.
    #[must_use]
    pub fn objects(&self) -> BTreeMap<String, Vec<u8>> {
        self.inner
            .objects
            .lock()
            .expect("objects mutex poisoned")
            .clone()
    }

    fn locator_for(&self, identity: &PublicationIdentity) -> String {
        format!("{}:{}", self.inner.adapter_id, identity.to_hex())
    }
}

struct InMemoryStaged {
    inner: Arc<BlobStoreState>,
    locator: String,
    bytes: Vec<u8>,
}

impl Write for InMemoryStaged {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl StagedObject for InMemoryStaged {
    fn commit(mut self: Box<Self>) -> Result<ProviderLocator, BlobStoreError> {
        if self.inner.corrupt_next.swap(false, Ordering::AcqRel)
            && let Some(first) = self.bytes.first_mut()
        {
            *first ^= 0xff;
        }
        self.inner
            .objects
            .lock()
            .expect("objects mutex poisoned")
            .insert(self.locator.clone(), std::mem::take(&mut self.bytes));
        Ok(ProviderLocator::new(self.locator.clone()))
    }

    fn abort(self: Box<Self>) {
        self.inner.aborted.fetch_add(1, Ordering::AcqRel);
    }
}

impl BlobStorePort for InMemoryBlobStore {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            adapter_id: self.inner.adapter_id.clone(),
            publish_mode: PublishMode::Copy,
            confirmation_kind: "in-memory".to_string(),
            object_limit: None,
            quota_visibility: QuotaVisibility::Unknown,
            native_placeholder: false,
        }
    }

    fn stage(
        &self,
        identity: &PublicationIdentity,
    ) -> Result<Box<dyn StagedObject + '_>, BlobStoreError> {
        self.inner.staged.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(InMemoryStaged {
            inner: Arc::clone(&self.inner),
            locator: self.locator_for(identity),
            bytes: Vec::new(),
        }))
    }

    fn open_published(
        &self,
        locator: &ProviderLocator,
    ) -> Result<Box<dyn Read + '_>, BlobStoreError> {
        let objects = self.inner.objects.lock().expect("objects mutex poisoned");
        let bytes = objects
            .get(locator.as_str())
            .cloned()
            .ok_or(BlobStoreError::NotFound)?;
        Ok(Box::new(std::io::Cursor::new(bytes)))
    }

    fn discard(&self, locator: &ProviderLocator) -> Result<(), BlobStoreError> {
        let removed = self
            .inner
            .objects
            .lock()
            .expect("objects mutex poisoned")
            .remove(locator.as_str());
        self.inner
            .discarded
            .lock()
            .expect("discarded mutex poisoned")
            .push(locator.as_str().to_string());
        removed.map(|_| ()).ok_or(BlobStoreError::NotFound)
    }

    fn confirm(
        &self,
        _locator: &ProviderLocator,
    ) -> Result<Option<ProviderConfirmation>, BlobStoreError> {
        Ok(None)
    }
}

/// An in-memory [`CatalogStore`] test double with a knob that fails the
/// next `register` (the HAP-001-R29 interruption fixture).
#[derive(Debug, Default)]
pub struct InMemoryCatalogStore {
    records: Vec<CatalogRecord>,
    fail_next: bool,
}

impl InMemoryCatalogStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next `register` with `WriteFailed`.
    pub const fn fail_next_register(&mut self) {
        self.fail_next = true;
    }
}

impl CatalogStore for InMemoryCatalogStore {
    fn list(&self) -> Result<Vec<CatalogRecord>, CatalogStoreError> {
        Ok(self.records.clone())
    }

    fn register(&mut self, record: CatalogRecord) -> Result<(), CatalogStoreError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(CatalogStoreError::WriteFailed);
        }
        if self
            .records
            .iter()
            .any(|existing| existing.publication_id == record.publication_id)
        {
            return Err(CatalogStoreError::Duplicate);
        }
        self.records.push(record);
        Ok(())
    }

    fn add_alias(
        &mut self,
        id: &PublicationIdentity,
        name: DisplayName,
    ) -> Result<(), CatalogStoreError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| &record.publication_id == id)
            .ok_or(CatalogStoreError::Unknown)?;
        if !record.names.contains(&name) {
            record.names.push(name);
        }
        Ok(())
    }
}

/// An in-memory [`PublicationJournal`] test double with a knob that fails
/// the next `append` (the crash window between a completed step and its
/// journal line, HAP-001-R29).
#[derive(Debug, Default)]
pub struct InMemoryJournal {
    entries: Vec<JournalEntry>,
    fail_next: bool,
}

impl InMemoryJournal {
    /// An empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next `append` with `WriteFailed`, recording nothing.
    pub const fn fail_next_append(&mut self) {
        self.fail_next = true;
    }
}

impl PublicationJournal for InMemoryJournal {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(JournalError::WriteFailed);
        }
        self.entries.push(entry.clone());
        Ok(())
    }

    fn replay(&self) -> Result<Vec<JournalEntry>, JournalError> {
        Ok(self.entries.clone())
    }
}

/// A scripted [`OutboxEntryOps`] test double: `identity` returns the
/// scripted outcomes in order (repeating the last), and `unlink` records
/// every name it was asked to remove.
#[derive(Debug)]
pub struct ScriptedEntryOps {
    identities: Mutex<VecDeque<EntryIdentity>>,
    unlinked: Mutex<Vec<OsString>>,
}

impl ScriptedEntryOps {
    /// Every identity check yields `identity`.
    #[must_use]
    pub fn always(identity: EntryIdentity) -> Self {
        Self::scripted([identity])
    }

    /// Yield each identity once, in order, then repeat the last.
    ///
    /// # Panics
    ///
    /// Panics if `identities` is empty.
    #[must_use]
    pub fn scripted(identities: impl IntoIterator<Item = EntryIdentity>) -> Self {
        let queue: VecDeque<EntryIdentity> = identities.into_iter().collect();
        assert!(
            !queue.is_empty(),
            "ScriptedEntryOps needs at least one identity"
        );
        Self {
            identities: Mutex::new(queue),
            unlinked: Mutex::new(Vec::new()),
        }
    }

    /// Every name `unlink` was asked to remove, in order.
    ///
    /// # Panics
    ///
    /// Panics if the double's mutex was poisoned by a prior panic.
    #[must_use]
    pub fn unlinked(&self) -> Vec<OsString> {
        self.unlinked
            .lock()
            .expect("unlinked mutex poisoned")
            .clone()
    }
}

impl OutboxEntryOps for ScriptedEntryOps {
    fn identity(
        &self,
        _dir: &DirectoryHandle,
        _dir_path: &Path,
        _name: &OsStr,
        _handle: &File,
    ) -> EntryIdentity {
        let mut queue = self.identities.lock().expect("identities mutex poisoned");
        if queue.len() > 1 {
            queue.pop_front().expect("non-empty")
        } else {
            *queue.front().expect("constructors require one identity")
        }
    }

    fn unlink(
        &self,
        _dir: &DirectoryHandle,
        _dir_path: &Path,
        name: &OsStr,
    ) -> std::io::Result<()> {
        self.unlinked
            .lock()
            .expect("unlinked mutex poisoned")
            .push(name.to_os_string());
        Ok(())
    }
}

// -- fixtures the contracts and the adapters' tests share --

/// A registered-looking record for `publication` under `asset_root`.
///
/// # Panics
///
/// Panics if the fixed asset root id is invalid (it is not).
#[must_use]
pub fn sample_record(
    publication: PublicationIdentity,
    asset_root: &str,
    name: &str,
) -> CatalogRecord {
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401);
    let asset_root_id = AssetRootId::new(asset_root).expect("a valid asset root id");
    let mut record = CatalogRecord {
        publication_id: publication,
        catalog_id: CatalogId {
            asset_root_id,
            publication_id: publication,
        },
        class: ArtifactClass::GeneratedHeavy,
        detected_type: DetectedType::Pdf,
        size: 33,
        digest: Sha256Digest([0xab; 32]),
        names: vec![DisplayName::sanitize(name)],
        relationships: Vec::new(),
        provenance: Provenance {
            scope_id: ProjectIdentity(Sha256Digest([7; 32])),
            producer: Producer::Run {
                run_id: RunId::new("run-1").expect("valid"),
                adapter_id: None,
                executable_approval: None,
            },
            transitions: Vec::new(),
        },
        provider: ProviderRecord {
            adapter_id: "fake".to_string(),
            locator: Some(ProviderLocator::new(format!(
                "fake:{}",
                publication.to_hex()
            ))),
            confirmation_kind: None,
            state: ProviderState::Pending,
            reason: Some("no remote".to_string()),
        },
        state: ArtifactState::Candidate,
        record_version: RECORD_VERSION,
    };
    record.transition(ArtifactState::PublishedLocal, at);
    record.transition(ArtifactState::Registered, at + Duration::from_secs(1));
    record
}

/// A sample approval for `publication`.
///
/// # Panics
///
/// Panics if the fixed ids are invalid (they are not).
#[must_use]
pub fn sample_approval(publication: PublicationIdentity, name: &str) -> ArtifactApproval {
    let run_id = RunId::new("run-1").expect("valid");
    ArtifactApproval {
        approval_id: ArtifactApprovalId(0x0123_4567_89ab_cdef),
        publication_id: publication,
        project: ProjectIdentity(Sha256Digest([7; 32])),
        run_id: run_id.clone(),
        name: format!("run-1/{name}"),
        display_name: DisplayName::sanitize(name),
        digest: Sha256Digest([0xab; 32]),
        size: 33,
        detected_type: DetectedType::Pdf,
        class: ArtifactClass::GeneratedHeavy,
        attribution: Attribution::Run(run_id),
        asset_root_id: AssetRootId::new("main").expect("valid"),
        adapter_id: Some(omnifrons_domain::adapter::AdapterId::claude_code()),
        executable_approval: Some(omnifrons_domain::executable::ApprovalId(42)),
        approver: DeviceLocalUser,
        approved_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_399),
    }
}

/// A sample step entry for `publication`.
#[must_use]
pub fn sample_step(
    publication: PublicationIdentity,
    step: JournalStep,
    record: Option<CatalogRecord>,
) -> StepEntry {
    StepEntry {
        publication_id: publication,
        approval_id: ArtifactApprovalId(0x0123_4567_89ab_cdef),
        step,
        outcome: StepOutcome::Ok,
        digest: Sha256Digest([0xab; 32]),
        reason: None,
        at: SystemTime::UNIX_EPOCH + Duration::new(1_725_782_401, 500),
        record,
        recovery: None,
    }
}

// -- contracts --

/// The [`CatalogStore`] contract: absent is empty; a registered record is
/// listed and found with every field intact; a second registration of the
/// same identity is `Duplicate`; an alias is applied on replay and never
/// duplicated; an alias for an unknown identity is `Unknown`.
///
/// # Panics
///
/// Panics on any contract violation.
pub fn catalog_store_contract<S: CatalogStore>(make: impl Fn() -> S) {
    let mut store = make();
    assert!(store.list().expect("an empty store lists").is_empty());

    let publication = PublicationIdentity(Sha256Digest([0x11; 32]));
    let record = sample_record(publication, "main", "report.pdf");
    store.register(record.clone()).expect("first registration");
    assert_eq!(store.list().expect("list"), vec![record.clone()]);
    assert_eq!(
        store.find(&publication).expect("find"),
        Some(record.clone())
    );
    assert_eq!(
        store
            .find(&PublicationIdentity(Sha256Digest([0x22; 32])))
            .expect("find"),
        None
    );

    assert_eq!(
        store.register(record.clone()),
        Err(CatalogStoreError::Duplicate),
        "one record per publication identity (HAP-001-R23)"
    );

    store
        .add_alias(&publication, DisplayName::sanitize("report-copy.pdf"))
        .expect("alias");
    store
        .add_alias(&publication, DisplayName::sanitize("report-copy.pdf"))
        .expect("a repeated alias is a no-op");
    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].names,
        vec![
            DisplayName::sanitize("report.pdf"),
            DisplayName::sanitize("report-copy.pdf")
        ]
    );
    assert_eq!(
        store.add_alias(
            &PublicationIdentity(Sha256Digest([0x22; 32])),
            DisplayName::sanitize("x")
        ),
        Err(CatalogStoreError::Unknown)
    );
}

/// The [`PublicationJournal`] contract: an empty journal replays empty;
/// every entry kind round-trips in order with every field intact.
///
/// # Panics
///
/// Panics on any contract violation.
pub fn publication_journal_contract<J: PublicationJournal>(make: impl Fn() -> J) {
    let mut journal = make();
    assert!(
        journal
            .replay()
            .expect("an empty journal replays")
            .is_empty()
    );

    let publication = PublicationIdentity(Sha256Digest([0x11; 32]));
    let record = sample_record(publication, "main", "report.pdf");
    let entries = vec![
        JournalEntry::Approved(Box::new(sample_approval(publication, "report.pdf"))),
        JournalEntry::Step(Box::new(sample_step(
            publication,
            JournalStep::PublishedLocal,
            Some(record),
        ))),
        JournalEntry::Step(Box::new(StepEntry {
            outcome: StepOutcome::Failed,
            reason: Some("path-names-a-different-file".to_string()),
            recovery: Some(Sha256Digest([0xcd; 32])),
            ..sample_step(publication, JournalStep::OutboxEscape, None)
        })),
        JournalEntry::Step(Box::new(StepEntry {
            outcome: StepOutcome::Deferred,
            reason: Some("no-handle-after-restart".to_string()),
            ..sample_step(publication, JournalStep::Cleanup, None)
        })),
    ];
    for entry in &entries {
        journal.append(entry).expect("append");
    }
    assert_eq!(journal.replay().expect("replay"), entries);
}

/// The [`BlobStorePort`] contract: a staged and committed copy is read
/// back byte for byte at the returned locator; an aborted stage publishes
/// nothing; a discarded copy is `NotFound` afterwards; the capabilities
/// declare `copy`.
///
/// # Panics
///
/// Panics on any contract violation.
pub fn blob_store_contract<B: BlobStorePort>(make: impl Fn() -> B) {
    let store = make();
    assert_eq!(store.capabilities().publish_mode, PublishMode::Copy);

    let identity = PublicationIdentity(Sha256Digest([0x11; 32]));
    let mut staged = store.stage(&identity).expect("stage");
    staged.write_all(b"%PDF-1.7\nbytes\n").expect("write");
    let locator = staged.commit().expect("commit");
    let mut read_back = Vec::new();
    store
        .open_published(&locator)
        .expect("open")
        .read_to_end(&mut read_back)
        .expect("read");
    assert_eq!(read_back, b"%PDF-1.7\nbytes\n");

    // Staging the same identity again replaces the copy under the same
    // locator: a retry after an interruption never leaves two objects
    // (HAP-001-R29).
    let mut again = store
        .stage(&identity)
        .expect("stage the same identity again");
    again.write_all(b"%PDF-1.7\nsecond\n").expect("write");
    let locator_again = again.commit().expect("commit again");
    assert_eq!(
        locator_again, locator,
        "the same identity, the same locator"
    );
    let mut replaced = Vec::new();
    store
        .open_published(&locator)
        .expect("open")
        .read_to_end(&mut replaced)
        .expect("read");
    assert_eq!(replaced, b"%PDF-1.7\nsecond\n");

    let other = PublicationIdentity(Sha256Digest([0x22; 32]));
    let mut aborted = store.stage(&other).expect("stage");
    aborted.write_all(b"never published").expect("write");
    aborted.abort();

    store.discard(&locator).expect("discard");
    assert!(matches!(
        store.open_published(&locator).map(|_| ()),
        Err(BlobStoreError::NotFound)
    ));
}
