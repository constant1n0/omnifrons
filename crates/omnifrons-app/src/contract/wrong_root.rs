//! Reusable contracts for the wrong-root ports (spike slice 5d): any
//! [`crate::ignore_ledger::IgnoreLedger`] must record a decision against a
//! project, a name and a digest and answer for exactly that triple, and
//! any [`crate::quarantine::QuarantineStore`] must move a held file into
//! the quarantine directory without leaving the original behind once the
//! destination verifies. Run by `omnifrons-adapters`' tests against the
//! real implementations, and by this crate's own tests against the fakes.

use std::time::{Duration, SystemTime};

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ArtifactClass, ContentDigest, DetectedType};
use omnifrons_domain::publication::ProjectIdentity;

use crate::ignore_ledger::{IgnoreEntry, IgnoreLedger, IgnoreLedgerError};

/// A deterministic ignore entry for `name` at `digest`.
#[must_use]
pub fn sample_entry(name: &str, digest: ContentDigest) -> IgnoreEntry {
    IgnoreEntry {
        name: name.to_string(),
        digest,
        size: 12,
        detected_type: DetectedType::Pdf,
        class: ArtifactClass::GeneratedHeavy,
        ignored_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    }
}

/// The in-memory [`IgnoreLedger`] the application tests drive.
#[derive(Debug, Default)]
pub struct InMemoryIgnoreLedger {
    entries: Vec<(ProjectIdentity, IgnoreEntry)>,
}

impl InMemoryIgnoreLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl IgnoreLedger for InMemoryIgnoreLedger {
    fn record(
        &mut self,
        project: &ProjectIdentity,
        entry: &IgnoreEntry,
    ) -> Result<(), IgnoreLedgerError> {
        self.entries.push((*project, entry.clone()));
        Ok(())
    }

    fn entries(&self, project: &ProjectIdentity) -> Result<Vec<IgnoreEntry>, IgnoreLedgerError> {
        Ok(self
            .entries
            .iter()
            .filter(|(recorded, _)| recorded == project)
            .map(|(_, entry)| entry.clone())
            .collect())
    }
}

/// Exercise the [`IgnoreLedger`] contract: a recorded decision answers for
/// its own project, name and digest and for nothing else; entries come
/// back in the order they were recorded; and recording the same decision
/// twice is not an error (the ledger is append-only, like the publication
/// journal).
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the ledger violates the contract.
pub fn ignore_ledger_contract<L: IgnoreLedger>(make: impl Fn() -> L) {
    let project = ProjectIdentity(Sha256Digest([1; 32]));
    let other = ProjectIdentity(Sha256Digest([2; 32]));
    let digest = Sha256Digest([3; 32]);
    let changed = Sha256Digest([4; 32]);

    let mut ledger = make();
    assert!(
        !ledger
            .is_ignored(&project, "docs/report.pdf", &digest)
            .expect("an empty ledger must read cleanly"),
        "nothing is ignored before a decision is recorded"
    );

    ledger
        .record(&project, &sample_entry("docs/report.pdf", digest))
        .expect("recording a decision must succeed");

    assert!(
        ledger
            .is_ignored(&project, "docs/report.pdf", &digest)
            .expect("read"),
        "the recorded decision answers for its own name and digest"
    );
    assert!(
        !ledger
            .is_ignored(&project, "docs/report.pdf", &changed)
            .expect("read"),
        "changed content is offered again"
    );
    assert!(
        !ledger
            .is_ignored(&project, "docs/other.pdf", &digest)
            .expect("read"),
        "another name is not covered"
    );
    assert!(
        !ledger
            .is_ignored(&other, "docs/report.pdf", &digest)
            .expect("read"),
        "an ignore decision is per project"
    );

    ledger
        .record(&project, &sample_entry("docs/second.pdf", changed))
        .expect("a second decision must succeed");
    ledger
        .record(&project, &sample_entry("docs/report.pdf", digest))
        .expect("recording the same decision twice must not be an error");

    let entries = ledger.entries(&project).expect("read");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["docs/report.pdf", "docs/second.pdf", "docs/report.pdf"],
        "entries come back in the order they were recorded"
    );
    assert!(
        ledger.entries(&other).expect("read").is_empty(),
        "another project's ledger stays empty"
    );
}

// -- The two remedies that touch real files --

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_domain::publication::DisplayName;

use crate::content_hasher::ContentHasher;
use crate::contract::publication::FakeHasher;
use crate::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use crate::publication::CandidateSource;
use crate::quarantine::{
    FallbackReason, QuarantineMove, QuarantinePorts, QuarantineRoot, QuarantineStore,
    RenameOutcome, quarantine, quarantine_name,
};
use crate::run_outbox::{
    CandidateProbe, CandidateProber, DirectoryHandle, OpenedOutbox, RegularCandidate,
};
use crate::wrong_root::{CopyInPorts, copy_in_from_misplaced};

/// A drop-guard temp directory the remedy contracts run in.
pub struct ContractDir(PathBuf);

impl ContractDir {
    /// Create a fresh, empty directory.
    ///
    /// # Panics
    ///
    /// Panics if the directory cannot be created.
    #[must_use]
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-remedy-contract-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the contract directory");
        Self(dir)
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ContractDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Open `path` as a directory handle with `std` alone. The real adapters
/// open a directory without following a link at its path
/// (`FsRunOutboxPreparer::open_directory_no_follow`); this crate has no
/// `nix`, so a contract fixture opens the directory it just created
/// itself.
///
/// # Panics
///
/// Panics if the directory cannot be opened.
#[must_use]
pub fn open_contract_dir(path: &Path) -> DirectoryHandle {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt as _;
        // `FILE_FLAG_BACKUP_SEMANTICS`: required to open a directory
        // handle at all on Windows (never `File::open` on a directory).
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000)
            .open(path)
            .expect("the contract directory must open")
    };
    #[cfg(not(windows))]
    let file = File::open(path).expect("the contract directory must open");
    DirectoryHandle::new(file)
}

/// A [`CandidateSource`] over the file `name` inside `dir`: the directory
/// handle, the entry's own name, and one open handle on the file.
///
/// # Panics
///
/// Panics if either open fails.
#[must_use]
pub fn contract_source(dir: &Path, name: &str) -> CandidateSource {
    CandidateSource {
        dir: open_contract_dir(dir),
        dir_path: dir.to_path_buf(),
        file_name: OsString::from(name),
        handle: File::open(dir.join(name)).expect("the contract file must open"),
    }
}

/// An [`OutboxEntryOps`] over `std` alone: file identity by device and
/// inode on unix, unverifiable elsewhere (exactly what `FsOutboxEntryOps`
/// reports), and a real removal.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdEntryOps;

impl StdEntryOps {
    /// Build the ops.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl OutboxEntryOps for StdEntryOps {
    fn identity(
        &self,
        _dir: &DirectoryHandle,
        dir_path: &Path,
        name: &OsStr,
        handle: &File,
    ) -> EntryIdentity {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let Ok(held) = handle.metadata() else {
                return EntryIdentity::DifferentFile;
            };
            match std::fs::symlink_metadata(dir_path.join(name)) {
                Ok(at_path) if at_path.dev() == held.dev() && at_path.ino() == held.ino() => {
                    EntryIdentity::SameFile
                }
                Ok(_) => EntryIdentity::DifferentFile,
                Err(_) => EntryIdentity::Missing,
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (dir_path, name, handle);
            EntryIdentity::Unverifiable
        }
    }

    fn unlink(&self, _dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::fs::remove_file(dir_path.join(name))
        }
        #[cfg(not(unix))]
        {
            let _ = (dir_path, name);
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        }
    }
}

/// A [`QuarantineStore`] that always declines the handle-anchored rename,
/// so a contract run over it exercises the copy-then-unlink branch on
/// every platform.
#[derive(Debug, Clone, Copy, Default)]
pub struct ForcedCopyQuarantineStore;

impl ForcedCopyQuarantineStore {
    /// Build the store.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl QuarantineStore for ForcedCopyQuarantineStore {
    fn open_dir(
        &self,
        root: &QuarantineRoot,
    ) -> Result<DirectoryHandle, crate::quarantine::QuarantineError> {
        Ok(open_contract_dir(root.path()))
    }

    fn rename_into(
        &self,
        _source: &CandidateSource,
        _dest_dir: &DirectoryHandle,
        _dest_path: &Path,
        _dest_name: &OsStr,
    ) -> RenameOutcome {
        RenameOutcome::Fallback(FallbackReason::DifferentVolume)
    }

    fn open_dest(
        &self,
        _dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> Result<File, crate::quarantine::QuarantineError> {
        open_dest_no_follow(dest_path, dest_name)
    }
}

/// A `std`-only stand-in for the adapters' no-follow open, for the fixture
/// stores this module offers: the name's own `symlink_metadata` decides
/// whether it is a link *before* it is opened, which is a check-then-open
/// where the real adapter has one `openat(O_NOFOLLOW)`. Good enough for a
/// fixture inside a directory the test owns; never the production path.
///
/// # Errors
///
/// Returns [`crate::quarantine::QuarantineError::CopyFailed`] when the name
/// holds a link or cannot be opened.
pub fn open_dest_no_follow(
    dest_path: &Path,
    dest_name: &OsStr,
) -> Result<File, crate::quarantine::QuarantineError> {
    let path = dest_path.join(dest_name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(crate::quarantine::QuarantineError::CopyFailed)
        }
        Ok(_) => File::open(&path).map_err(|_| crate::quarantine::QuarantineError::CopyFailed),
        Err(_) => Err(crate::quarantine::QuarantineError::CopyFailed),
    }
}

/// A [`CandidateProber`] over `std` alone, for the copy-in contract: the
/// real `FsCandidateProber` opens without following links and is covered
/// by its own contract; this one only has to hand back a handle and a
/// digest of the entry the remedy just created.
pub struct StdCandidateProber<'a> {
    hasher: &'a dyn ContentHasher,
}

impl<'a> StdCandidateProber<'a> {
    /// Build a prober digesting through `hasher`.
    #[must_use]
    pub const fn new(hasher: &'a dyn ContentHasher) -> Self {
        Self { hasher }
    }
}

impl CandidateProber for StdCandidateProber<'_> {
    fn probe(&self, _dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> CandidateProbe {
        use std::io::Seek as _;
        let Ok(mut file) = File::open(dir_path.join(name)) else {
            return CandidateProbe::Unreadable;
        };
        let Ok((digest, size)) = self.hasher.digest_reader(&mut file) else {
            return CandidateProbe::Unreadable;
        };
        if file.seek(std::io::SeekFrom::Start(0)).is_err() {
            return CandidateProbe::Unreadable;
        }
        CandidateProbe::Regular(RegularCandidate {
            size,
            digest,
            detected_type: omnifrons_domain::outbox::DetectedType::Pdf,
            handle: file,
        })
    }
}

/// The bytes every remedy contract moves: a PDF by content, so the
/// classification policy calls it `generated-heavy`.
#[must_use]
pub fn contract_bytes() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(&[7u8; 128]);
    bytes
}

/// Exercise the quarantine remedy over `store` and `entry_ops`
/// (HAP-001 § Wrong-root detection and remedies; RCS-001-R10): the file's
/// bytes reach the quarantine directory under a sanitized, digest-prefixed
/// name and verify there, and the move that ran is exactly `expected`.
///
/// `expected` is a parameter and not a disjunction the assertion accepts
/// either half of (spike slice 5d, R3-008): a caller knows which branch its
/// own store and ops take, and a contract that accepts both pins neither
/// for the run that actually took one. Whether the original is gone follows
/// from `expected` rather than from a second guess about the platform.
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the remedy violates the contract.
pub fn quarantine_store_contract(
    store: &dyn QuarantineStore,
    entry_ops: &dyn OutboxEntryOps,
    expected_move: &QuarantineMove,
) {
    let project = ContractDir::new("quarantine-project");
    let quarantine_dir = ContractDir::new("quarantine-root");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");
    let mut source = contract_source(project.path(), "report.pdf");

    let quarantined = quarantine(
        &QuarantinePorts {
            store,
            hasher: &hasher,
            entry_ops,
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("a held, unchanged file moves into quarantine");

    assert_eq!(
        quarantined.name,
        quarantine_name(&DisplayName::sanitize("report.pdf"), &expected),
        "the quarantined name is the sanitized display name behind its digest"
    );
    assert!(
        !quarantined.name.contains('/') && !quarantined.name.contains('\\'),
        "a quarantined name is one component, never a path: {}",
        quarantined.name
    );
    assert_eq!(quarantined.digest, expected);
    assert_eq!(quarantined.size, bytes.len() as u64);
    assert_eq!(
        std::fs::read(root.path().join(&quarantined.name)).expect("the quarantined file"),
        bytes,
        "the bytes at the destination are the bytes that were held"
    );
    assert_eq!(
        &quarantined.outcome, expected_move,
        "the move this store and these ops take is fixed by the caller, not guessed here"
    );

    // Whether the original is gone is a consequence of the move that ran,
    // not a second platform guess. Where `std` offers no by-handle identity
    // comparison and no relative unlink, the copy stands and the original
    // is kept -- stated rather than silently duplicated (HAP-001-R19).
    let original_removed = match expected_move {
        QuarantineMove::Renamed
        | QuarantineMove::RenamedUnverified
        | QuarantineMove::CopiedAndUnlinked { .. } => true,
        QuarantineMove::CopiedOriginalKept { .. } => false,
    };
    assert_eq!(
        !project.path().join("report.pdf").exists(),
        original_removed,
        "the original's fate must follow the move that ran: {:?}",
        quarantined.outcome
    );
}

/// Exercise the publish remedy's copy-in over `entry_ops`
/// (HAP-001-R32's publish remedy): the misplaced file's bytes reach the
/// outbox root as a new entry named by its digest, opened once from the
/// copy, and **the original stays exactly where it was found**
/// (HAP-001-R28: the product never deletes a file it did not create).
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the remedy violates the contract.
pub fn copy_in_contract(entry_ops: &dyn OutboxEntryOps) {
    let project = ContractDir::new("copy-in-project");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");
    let outbox_path = project.path().join("outbox");
    std::fs::create_dir_all(&outbox_path).expect("fixture outbox");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let prober = StdCandidateProber::new(&hasher);
    let outbox = OpenedOutbox {
        path: outbox_path.clone(),
        handle: open_contract_dir(&outbox_path),
    };
    let mut source = contract_source(project.path(), "report.pdf");

    let copied = copy_in_from_misplaced(
        &CopyInPorts {
            hasher: &hasher,
            entry_ops,
            prober: &prober,
        },
        &mut source,
        &outbox,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("a held, unchanged file copies into the outbox");

    assert_eq!(copied.digest, expected);
    assert_eq!(copied.size, bytes.len() as u64);
    assert!(
        !copied.name.contains('/') && !copied.name.contains('\\'),
        "the new entry's name is one component at the outbox root: {}",
        copied.name
    );
    assert_eq!(
        std::fs::read(outbox_path.join(&copied.name)).expect("the copied entry"),
        bytes,
        "the outbox entry carries the bytes that were held"
    );
    assert!(
        project.path().join("report.pdf").exists(),
        "the publish remedy leaves the original exactly where it was found"
    );
    assert_eq!(
        copied.source.file_name,
        OsString::from(&copied.name),
        "the returned source names the new entry, not the misplaced file"
    );

    // Idempotent: running the remedy again over the same bytes reuses the
    // entry rather than creating a second copy or failing.
    let mut again = contract_source(project.path(), "report.pdf");
    let repeated = copy_in_from_misplaced(
        &CopyInPorts {
            hasher: &hasher,
            entry_ops,
            prober: &prober,
        },
        &mut again,
        &outbox,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("a repeated remedy is idempotent");
    assert_eq!(repeated.name, copied.name);
    let entries = std::fs::read_dir(&outbox_path)
        .expect("list")
        .filter_map(Result::ok)
        .count();
    assert_eq!(entries, 1, "no second copy is made");
}
