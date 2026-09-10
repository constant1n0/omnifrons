//! Runs `omnifrons-app`'s reusable wrong-root contracts (spike slice 5d)
//! against the in-memory fakes -- the real adapters (`omnifrons-adapters`)
//! run them again against themselves.
//!
//! Only compiled with `--features contract-tests`.

#![cfg(feature = "contract-tests")]

use omnifrons_app::content_hasher::ContentHasher as _;
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::contract::wrong_root::{
    ContractDir, ForcedCopyQuarantineStore, InMemoryIgnoreLedger, StdEntryOps, contract_bytes,
    contract_source, copy_in_contract, ignore_ledger_contract, open_contract_dir,
    open_dest_no_follow, quarantine_store_contract,
};
use omnifrons_app::ignore_ledger::{IgnoreEntry, IgnoreLedger as _, retain_unignored};
use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_app::publication::CandidateSource;
use omnifrons_app::quarantine::{
    QuarantineError, QuarantineMove, QuarantinePorts, QuarantineRoot, QuarantineStore,
    RenameOutcome, quarantine, quarantine_name,
};
use omnifrons_app::run_outbox::DirectoryHandle;
use omnifrons_app::wrong_root::{ScannedFile, Verdict, finding_for};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ArtifactClass, DetectedType};
use omnifrons_domain::publication::{DisplayName, ProjectIdentity};
use omnifrons_domain::wrong_root::{MisplacedFinding, WrongRootReason};
use std::ffi::OsStr;
use std::path::Path;
use std::time::UNIX_EPOCH;

fn scanned(name: &str, detected_type: DetectedType, size: u64) -> ScannedFile {
    ScannedFile {
        name: name.to_string(),
        size,
        digest: Sha256Digest([3; 32]),
        detected_type,
    }
}

#[test]
fn in_memory_ignore_ledger_satisfies_the_contract() {
    ignore_ledger_contract(InMemoryIgnoreLedger::new);
}

/// HAP-001 § Wrong-root detection and remedies, the ignore remedy: "the
/// decision is recorded against the file's identity and digest, and the
/// file is not offered again until its content changes". The ledger is the
/// one durable fact a remedy leaves behind; `misplaced` itself is
/// recomputed by every scan and never persisted.
#[test]
fn an_ignored_finding_is_withheld_until_its_content_changes() {
    let project = ProjectIdentity(Sha256Digest([9; 32]));
    let mut ledger = InMemoryIgnoreLedger::new();
    let policy = OutboxPolicy::default_policy();

    let Verdict::Misplaced(first) =
        finding_for(&scanned("docs/report.pdf", DetectedType::Pdf, 10), &policy)
    else {
        panic!("a heavy file outside the outbox is misplaced");
    };
    ledger
        .record(&project, &IgnoreEntry::of(&first, UNIX_EPOCH))
        .expect("recording an ignore decision must succeed");

    let (kept, ignored) = retain_unignored(vec![first.clone()], &ledger, &project)
        .expect("filtering must read the ledger");
    assert!(kept.is_empty(), "an ignored finding is not offered again");
    assert_eq!(ignored, 1);

    // The same name, different bytes: offered again.
    let rewritten = MisplacedFinding::new(
        first.name(),
        11,
        Sha256Digest([4; 32]),
        DetectedType::Pdf,
        ArtifactClass::GeneratedHeavy,
        WrongRootReason::InProjectOutsideOutbox,
    )
    .expect("a project-relative name");
    let (kept, ignored) = retain_unignored(vec![rewritten], &ledger, &project)
        .expect("filtering must read the ledger");
    assert_eq!(kept.len(), 1, "changed content is offered again");
    assert_eq!(ignored, 0);

    // Another project's ledger never speaks for this one.
    let other = ProjectIdentity(Sha256Digest([1; 32]));
    let (kept, _) = retain_unignored(vec![first], &ledger, &other).expect("filtering");
    assert_eq!(kept.len(), 1, "an ignore decision is per project");
}

/// The quarantine remedy over the copy-then-unlink branch: the app crate
/// drives a store that always declines the handle-anchored rename, so this
/// run proves the fallback path end to end (the real `FsQuarantineStore`
/// runs the same contract and takes the rename branch on one volume).
#[test]
fn the_copy_then_unlink_branch_satisfies_the_quarantine_contract() {
    // `StdEntryOps` compares by device and inode and unlinks for real on
    // unix, and reports `Unverifiable` with no relative unlink anywhere
    // else -- so the branch this run takes is known, not guessed (R3-008).
    #[cfg(unix)]
    let expected = QuarantineMove::CopiedAndUnlinked {
        fallback: omnifrons_app::quarantine::FallbackReason::DifferentVolume,
    };
    #[cfg(not(unix))]
    let expected = QuarantineMove::CopiedOriginalKept {
        fallback: omnifrons_app::quarantine::FallbackReason::DifferentVolume,
        reason: "identity-check-unavailable",
    };
    quarantine_store_contract(
        &ForcedCopyQuarantineStore::new(),
        &StdEntryOps::new(),
        &expected,
    );
}

/// The publish remedy's copy-in: the misplaced file's bytes reach the
/// outbox root as a new unattributed entry, opened once from the handle
/// that was already held, and the original stays exactly where it was.
#[test]
fn the_copy_in_from_a_misplaced_file_satisfies_its_contract() {
    copy_in_contract(&StdEntryOps::new());
}

// -- The quarantine remedy's two hostile-destination cases (R1-001, R3-004) --

/// A [`QuarantineStore`] that reports whatever `outcome` it was built with
/// and lets the test decide what actually sits at the destination name
/// beforehand: the two facts the remedy must not confuse are "the store
/// says the rename happened" and "the destination name holds the file that
/// was moved".
struct StagedQuarantineStore {
    outcome: RenameOutcome,
    /// Run once, before the reported outcome is returned, with the
    /// destination directory and the name the remedy chose.
    stage: Box<Stage>,
}

/// What a [`StagedQuarantineStore`] does to the destination before it
/// reports its outcome.
type Stage = dyn Fn(&Path, &OsStr) + Send + Sync;

impl QuarantineStore for StagedQuarantineStore {
    fn open_dir(&self, root: &QuarantineRoot) -> Result<DirectoryHandle, QuarantineError> {
        Ok(open_contract_dir(root.path()))
    }

    fn rename_into(
        &self,
        _source: &CandidateSource,
        _dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> RenameOutcome {
        (self.stage)(dest_path, dest_name);
        self.outcome
    }

    fn open_dest(
        &self,
        _dest_dir: &DirectoryHandle,
        dest_path: &Path,
        dest_name: &OsStr,
    ) -> Result<std::fs::File, QuarantineError> {
        open_dest_no_follow(dest_path, dest_name)
    }
}

/// R1-001: the copy path's staging sibling is created **exclusively**, so a
/// link planted at its fully predictable name (`.part-<short digest hex>-<display
/// name>` -- both halves owned by whoever misplaced the file) is never
/// written through. Following it would let this remedy truncate and
/// overwrite any file the user can write, and the destination verification
/// would then digest the victim and *pass*, so the surface would report a
/// successful quarantine while the bytes went somewhere else entirely.
#[cfg(unix)]
#[test]
fn a_link_planted_at_the_staging_name_is_never_written_through() {
    let project = ContractDir::new("staging-link-project");
    let quarantine_dir = ContractDir::new("staging-link-quarantine");
    let victim_dir = ContractDir::new("staging-link-victim");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let victim = victim_dir.path().join("keep-me.txt");
    let victim_bytes = b"the user's own file, outside every project".to_vec();
    std::fs::write(&victim, &victim_bytes).expect("fixture victim");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");
    let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);
    std::os::unix::fs::symlink(&victim, root.path().join(format!(".part-{name}")))
        .expect("the planted staging link");

    let mut source = contract_source(project.path(), "report.pdf");
    let _ = quarantine(
        &QuarantinePorts {
            store: &ForcedCopyQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    );

    assert_eq!(
        std::fs::read(&victim).expect("the victim still exists"),
        victim_bytes,
        "a link at the staging name must never be written through"
    );
}

/// R1-001: the destination verification opens the name **without following
/// a link at it**. A link left at the destination name pointing at a file
/// that happens to carry the very bytes the request bound to would
/// otherwise digest clean, and the remedy would report a verified move over
/// a file that never entered the quarantine directory.
#[cfg(unix)]
#[test]
fn a_link_at_the_destination_name_is_never_dereferenced_by_the_verification() {
    let project = ContractDir::new("dest-link-project");
    let quarantine_dir = ContractDir::new("dest-link-quarantine");
    let outside_dir = ContractDir::new("dest-link-outside");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");
    // The very same bytes: a verification that dereferences the link would
    // find the digest and the size it expects and pass.
    let outside = outside_dir.path().join("decoy.pdf");
    std::fs::write(&outside, &bytes).expect("fixture decoy");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");

    let store = StagedQuarantineStore {
        outcome: RenameOutcome::Renamed,
        stage: Box::new(move |dest_path, dest_name| {
            std::os::unix::fs::symlink(&outside, dest_path.join(dest_name))
                .expect("the planted destination link");
        }),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &store,
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("a rename the store reported is never reported back as a refusal");

    assert_eq!(
        quarantined.outcome,
        QuarantineMove::RenamedUnverified,
        "a link at the destination name is never dereferenced, so the move stays unverified"
    );
}

/// R1-017: the copy path gives its destination its final name **created
/// exclusively**, exactly as the publish remedy's twin does, and never
/// through a `rename` that replaces whatever holds it.
///
/// The two remedies must guarantee the same thing at their final name.
/// `rename` replaces a file, a link, or a dangling link there without a
/// word: a quarantine run over a name something else already held would
/// destroy that file and then report a verified move, which is the one
/// thing HAP-001-R28 says this product never does. Refusing is not enough
/// either -- what the name held must still be there afterwards, because a
/// remedy that removes what it did not create has removed it whether it
/// reported success or not.
#[test]
fn the_copy_paths_destination_name_is_created_exclusively_and_never_replaced() {
    let project = ContractDir::new("dest-exists-project");
    let quarantine_dir = ContractDir::new("dest-exists-quarantine");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");

    // Something else already holds the destination name, carrying other
    // bytes and a different length.
    let planted = b"a file this remedy did not create".to_vec();
    let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);
    std::fs::write(root.path().join(&name), &planted).expect("the planted destination");

    let mut source = contract_source(project.path(), "report.pdf");
    let refused = quarantine(
        &QuarantinePorts {
            store: &ForcedCopyQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    );

    assert_eq!(
        refused.err(),
        Some(QuarantineError::DigestChanged),
        "a destination name already held by other bytes refuses the move"
    );
    assert_eq!(
        std::fs::read(root.path().join(&name)).expect("the planted file is still there"),
        planted,
        "and what held the name is left exactly as it was found, never replaced or removed"
    );
    assert_eq!(
        std::fs::read(project.path().join("report.pdf")).expect("the original"),
        bytes,
        "nothing moved, so the original is still where it was found"
    );
    assert!(
        !root.path().join(format!(".part-{name}")).exists(),
        "and the staging sibling is not left behind"
    );
}

/// R1-022: the two remedies must guarantee the same thing at their final
/// name, and until now they did not. The publish remedy asks
/// `CandidateProber::probe` and refuses `Linked` under HAP-001-R20; the
/// quarantine remedy's own acceptance of a name already held read only the
/// digest and the size, so a same-user producer could plant a hard link at
/// the quarantine destination to a file **outside** the quarantine
/// directory carrying these very bytes: the verification passed, the
/// original was unlinked, and the "quarantined" entry was an alias of an
/// inode the producer still holds another name for and can rewrite
/// afterwards.
#[cfg(unix)]
#[test]
fn a_destination_that_answers_to_a_second_name_is_refused_and_the_original_kept() {
    let project = ContractDir::new("dest-linked-project");
    let quarantine_dir = ContractDir::new("dest-linked-quarantine");
    let outside_dir = ContractDir::new("dest-linked-outside");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");
    let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);

    // The very same bytes, so digest and size verify: only the link count
    // on the destination's own handle can tell this apart from this
    // remedy's own earlier result.
    let outside = outside_dir.path().join("the-producers-copy.pdf");
    std::fs::write(&outside, &bytes).expect("fixture");
    std::fs::hard_link(&outside, root.path().join(&name)).expect("the planted hard link");

    let mut source = contract_source(project.path(), "report.pdf");
    let refused = quarantine(
        &QuarantinePorts {
            store: &ForcedCopyQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    );

    assert_eq!(
        refused.err(),
        Some(QuarantineError::DestinationLinked { link_count: 2 }),
        "a destination that answers to a second name is refused (HAP-001-R20)"
    );
    assert_eq!(
        std::fs::read(project.path().join("report.pdf")).expect("the original"),
        bytes,
        "and the original is kept: nothing was quarantined"
    );
    assert_eq!(
        std::fs::read(&outside).expect("the producer's own name still resolves"),
        bytes,
        "what held the name is left exactly as it was found, never replaced or removed"
    );
}

/// The Windows counterpart of the refusal above. The by-handle link-count
/// accessor in `std::os::windows::fs::MetadataExt` is unstable and this
/// repository adds no Windows API dependency, so a destination answering to
/// a second name cannot be recognized there and, carrying the expected
/// bytes, is reused as this remedy's own idempotent result -- the same
/// residual the outbox entry already carries (HAP-001-R19,
/// `docs/spike-log.md` § Slice 5d), not a claim.
#[cfg(not(unix))]
#[test]
fn a_platform_without_a_by_handle_link_count_reuses_a_linked_destination() {
    let project = ContractDir::new("dest-linked-project");
    let quarantine_dir = ContractDir::new("dest-linked-quarantine");
    let outside_dir = ContractDir::new("dest-linked-outside");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");
    let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);

    let outside = outside_dir.path().join("the-producers-copy.pdf");
    std::fs::write(&outside, &bytes).expect("fixture");
    std::fs::hard_link(&outside, root.path().join(&name)).expect("the planted hard link");

    let mut source = contract_source(project.path(), "report.pdf");
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &ForcedCopyQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("without a link count the destination cannot be refused");
    assert_eq!(quarantined.name, name);
    assert_eq!(
        quarantined.outcome,
        QuarantineMove::CopiedOriginalKept {
            fallback: omnifrons_app::quarantine::FallbackReason::DifferentVolume,
            reason: "identity-check-unavailable"
        },
        "and with no relative unlink either, the original is kept and the payload says so"
    );
}

/// R3-004 and R1-007: once the rename has run the original name is gone by
/// definition, so a verification that then fails may not be reported as a
/// refusal whose fixed message says the content changed and the move did
/// not happen. The move happened; only the verification did not, and the
/// payload says exactly that under the destination name.
#[test]
fn a_verification_that_fails_after_the_rename_is_reported_as_an_unverified_move() {
    let project = ContractDir::new("post-rename-project");
    let quarantine_dir = ContractDir::new("post-rename-quarantine");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");

    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root outside every workspace");

    // A store that really renames -- the original leaves the project -- and
    // whose destination then carries other bytes, exactly as a truncated or
    // corrupted write would leave it.
    let source_path = project.path().join("report.pdf");
    let store = StagedQuarantineStore {
        outcome: RenameOutcome::Renamed,
        stage: Box::new(move |dest_path, dest_name| {
            std::fs::rename(&source_path, dest_path.join(dest_name)).expect("the fixture rename");
            std::fs::write(dest_path.join(dest_name), b"other bytes entirely")
                .expect("the corrupted destination");
        }),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &store,
            hasher: &hasher,
            entry_ops: &StdEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("a rename that already ran is never reported as a refusal that changed nothing");

    assert_eq!(quarantined.outcome, QuarantineMove::RenamedUnverified);
    assert_eq!(
        quarantined.name,
        quarantine_name(&DisplayName::sanitize("report.pdf"), &expected),
        "the payload names the destination the file actually reached"
    );
    assert!(
        !project.path().join("report.pdf").exists(),
        "the rename removed the original name: no outcome may say otherwise"
    );
}

// -- The publish remedy's copy-in against a hostile outbox root (R1-004) --

/// The pieces one copy-in run needs: a project holding `report.pdf`, an
/// outbox directory, and the digest the remedy binds to.
struct CopyInFixture {
    project: ContractDir,
    outbox_dir: ContractDir,
    /// Read only by the unix cases that assert what reached the outbox;
    /// the Windows counterpart asserts the name alone, since without a
    /// by-handle link count it has no refusal to check.
    #[cfg_attr(not(unix), allow(dead_code))]
    bytes: Vec<u8>,
    expected: Sha256Digest,
    name: String,
}

impl CopyInFixture {
    fn new(label: &str) -> Self {
        let project = ContractDir::new(&format!("{label}-project"));
        let outbox_dir = ContractDir::new(&format!("{label}-outbox"));
        let bytes = contract_bytes();
        std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");
        let expected = FakeHasher.sha256(&bytes);
        let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);
        Self {
            project,
            outbox_dir,
            bytes,
            expected,
            name,
        }
    }

    fn run(
        &self,
    ) -> Result<omnifrons_app::wrong_root::CopiedIn, omnifrons_app::wrong_root::CopyInError> {
        let hasher = FakeHasher;
        let prober = omnifrons_app::contract::wrong_root::StdCandidateProber::new(&hasher);
        let outbox = omnifrons_app::run_outbox::OpenedOutbox {
            path: self.outbox_dir.path().to_path_buf(),
            handle: open_contract_dir(self.outbox_dir.path()),
        };
        let mut source = contract_source(self.project.path(), "report.pdf");
        omnifrons_app::wrong_root::copy_in_from_misplaced(
            &omnifrons_app::wrong_root::CopyInPorts {
                hasher: &hasher,
                entry_ops: &StdEntryOps::new(),
                prober: &prober,
            },
            &mut source,
            &outbox,
            &self.expected,
            &DisplayName::sanitize("report.pdf"),
        )
    }
}

/// R1-004: the new entry's final name is created **exclusively**, so
/// anything already standing at it -- including a *dangling* link, which
/// `Path::exists()` reports as absent while `rename` would replace it
/// without following it -- is refused rather than silently overwritten.
#[cfg(unix)]
#[test]
fn a_link_planted_at_the_new_entrys_name_is_refused_not_replaced() {
    let fixture = CopyInFixture::new("entry-name-link");
    let planted = fixture.outbox_dir.path().join(&fixture.name);
    std::os::unix::fs::symlink("nothing-is-here", &planted).expect("the planted dangling link");

    assert_eq!(
        fixture.run().err(),
        Some(omnifrons_app::wrong_root::CopyInError::EntryExists),
        "a name already held by something else is refused, never replaced"
    );
    assert!(
        std::fs::symlink_metadata(&planted)
            .expect("the planted link is still there")
            .file_type()
            .is_symlink(),
        "the remedy replaced the planted link instead of refusing it"
    );
}

/// R1-004's sibling: the staging name is created exclusively too, so a link
/// planted at `.part-<name>` is unlinked and replaced rather than written
/// through into whatever it points at.
#[cfg(unix)]
#[test]
fn a_link_planted_at_the_copy_ins_staging_name_is_never_written_through() {
    let fixture = CopyInFixture::new("entry-part-link");
    let victim_dir = ContractDir::new("entry-part-link-victim");
    let victim = victim_dir.path().join("keep-me.txt");
    let victim_bytes = b"the user's own file, outside every project".to_vec();
    std::fs::write(&victim, &victim_bytes).expect("fixture victim");
    std::os::unix::fs::symlink(
        &victim,
        fixture
            .outbox_dir
            .path()
            .join(format!(".part-{}", fixture.name)),
    )
    .expect("the planted staging link");

    let copied = fixture.run().expect("the copy-in still creates its entry");
    assert_eq!(copied.name, fixture.name);
    assert_eq!(
        std::fs::read(&victim).expect("the victim still exists"),
        victim_bytes,
        "a link at the staging name must never be written through"
    );
    assert_eq!(
        std::fs::read(fixture.outbox_dir.path().join(&fixture.name)).expect("the new entry"),
        fixture.bytes
    );
}

/// R3-012: an entry already standing at the new name carrying **other**
/// bytes is `EntryExists`, never overwritten -- the idempotent reuse is for
/// the same bytes alone.
#[test]
fn an_entry_of_that_name_carrying_other_bytes_is_refused() {
    let project = ContractDir::new("entry-exists-project");
    let outbox_dir = ContractDir::new("entry-exists-outbox");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");
    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let name = quarantine_name(&DisplayName::sanitize("report.pdf"), &expected);
    let squatted = outbox_dir.path().join(&name);
    std::fs::write(&squatted, b"other bytes entirely").expect("the squatting entry");

    let prober = omnifrons_app::contract::wrong_root::StdCandidateProber::new(&hasher);
    let outbox = omnifrons_app::run_outbox::OpenedOutbox {
        path: outbox_dir.path().to_path_buf(),
        handle: open_contract_dir(outbox_dir.path()),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    assert_eq!(
        omnifrons_app::wrong_root::copy_in_from_misplaced(
            &omnifrons_app::wrong_root::CopyInPorts {
                hasher: &hasher,
                entry_ops: &StdEntryOps::new(),
                prober: &prober,
            },
            &mut source,
            &outbox,
            &expected,
            &DisplayName::sanitize("report.pdf"),
        )
        .err(),
        Some(omnifrons_app::wrong_root::CopyInError::EntryExists)
    );
    assert_eq!(
        std::fs::read(&squatted).expect("read"),
        b"other bytes entirely",
        "an entry carrying other bytes is never overwritten"
    );
}

/// R1-003 / HAP-001-R20, applied with the misplaced file as the candidate
/// (HAP-001-R32): a file with more than one name is refused. A hard link
/// inside the project to a file outside it otherwise becomes a publishable
/// outbox candidate carrying that outside file's bytes, and the copy would
/// say the project produced them.
#[cfg(unix)]
#[test]
fn a_misplaced_file_with_more_than_one_name_is_refused_by_the_copy_in() {
    let fixture = CopyInFixture::new("copy-in-linked");
    let elsewhere = ContractDir::new("copy-in-linked-elsewhere");
    std::fs::hard_link(
        fixture.project.path().join("report.pdf"),
        elsewhere.path().join("second-name.pdf"),
    )
    .expect("the fixture hard link");

    assert_eq!(
        fixture.run().err(),
        Some(omnifrons_app::wrong_root::CopyInError::Linked { link_count: 2 }),
        "a candidate with more than one link is refused (HAP-001-R20)"
    );
    assert!(
        !fixture.outbox_dir.path().join(&fixture.name).exists(),
        "nothing reached the outbox"
    );
}

/// The Windows counterpart of the refusal above: the by-handle link-count
/// accessor in `std::os::windows::fs::MetadataExt` is unstable and this
/// repository adds no Windows API dependency, so `outbox-linked` cannot be
/// produced there and the copy-in proceeds. A residual disclosed under
/// HAP-001-R19 and `docs/spike-log.md` § Slice 5d, not a claim.
#[cfg(not(unix))]
#[test]
fn a_platform_without_a_by_handle_link_count_cannot_refuse_a_linked_candidate() {
    let fixture = CopyInFixture::new("copy-in-linked");
    let elsewhere = ContractDir::new("copy-in-linked-elsewhere");
    std::fs::hard_link(
        fixture.project.path().join("report.pdf"),
        elsewhere.path().join("second-name.pdf"),
    )
    .expect("the fixture hard link");

    let copied = fixture
        .run()
        .expect("without a link count the copy-in cannot refuse");
    assert_eq!(copied.name, fixture.name);
}

// -- Every quarantine outcome the wire fixes a token for (R3-002) --

/// An [`OutboxEntryOps`] whose identity answers are scripted, so the
/// outcomes `remove_original` reaches -- each of which the wire turns into
/// a fixed `detail` token -- can be produced on every platform instead of
/// only on whichever one happens to take that branch.
struct ScriptedEntryOps {
    identities: std::sync::Mutex<
        std::collections::VecDeque<omnifrons_app::outbox_entry_ops::EntryIdentity>,
    >,
    /// Whether the unlink succeeds; when it does, the entry really is
    /// removed, so the fixture never claims a removal it did not make.
    unlink_succeeds: bool,
}

impl ScriptedEntryOps {
    fn new(
        identities: &[omnifrons_app::outbox_entry_ops::EntryIdentity],
        unlink_succeeds: bool,
    ) -> Self {
        Self {
            identities: std::sync::Mutex::new(identities.iter().copied().collect()),
            unlink_succeeds,
        }
    }
}

impl omnifrons_app::outbox_entry_ops::OutboxEntryOps for ScriptedEntryOps {
    fn identity(
        &self,
        _dir: &DirectoryHandle,
        _dir_path: &Path,
        _name: &OsStr,
        _handle: &std::fs::File,
    ) -> omnifrons_app::outbox_entry_ops::EntryIdentity {
        self.identities
            .lock()
            .expect("the scripted identities are never poisoned")
            .pop_front()
            .expect("the script must cover every identity check the remedy makes")
    }

    fn unlink(&self, _dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> std::io::Result<()> {
        if self.unlink_succeeds {
            std::fs::remove_file(dir_path.join(name))
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        }
    }
}

/// Run the quarantine remedy over a store that reports `outcome` and ops
/// that answer `identities`, and return the move it produced.
fn quarantine_outcome(
    label: &str,
    outcome: RenameOutcome,
    identities: &[omnifrons_app::outbox_entry_ops::EntryIdentity],
    unlink_succeeds: bool,
) -> QuarantineMove {
    let project = ContractDir::new(label);
    let quarantine_dir = ContractDir::new(&format!("{label}-quarantine"));
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture file");
    let hasher = FakeHasher;
    let expected = hasher.sha256(&bytes);
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let store = StagedQuarantineStore {
        outcome,
        stage: Box::new(|_, _| {}),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    quarantine(
        &QuarantinePorts {
            store: &store,
            hasher: &hasher,
            entry_ops: &ScriptedEntryOps::new(identities, unlink_succeeds),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("the remedy completes")
    .outcome
}

/// R3-002: every [`QuarantineMove`] the wire fixes a `detail` token for is
/// produced by a real run of the remedy, on every platform. Four of them --
/// `rename-failed`, `unlink-failed`, `original-already-gone` and
/// `original-changed-during-the-move` -- were reachable by no test at all
/// before this, so the tokens the contract promises were promises about
/// code nothing had ever run.
#[test]
fn every_quarantine_outcome_the_wire_names_is_produced_by_a_real_run() {
    use omnifrons_app::outbox_entry_ops::EntryIdentity;
    use omnifrons_app::quarantine::FallbackReason;

    // The pre-move check passes, then the post-copy check decides.
    let before = EntryIdentity::SameFile;

    for fallback in [
        FallbackReason::DifferentVolume,
        FallbackReason::Unsupported,
        FallbackReason::Failed,
    ] {
        assert_eq!(
            quarantine_outcome(
                "detail-unlinked",
                RenameOutcome::Fallback(fallback),
                &[before, EntryIdentity::SameFile],
                true,
            ),
            QuarantineMove::CopiedAndUnlinked { fallback },
            "{} is the fallback the copy path ran under",
            fallback.as_str()
        );
    }

    let fallback = FallbackReason::DifferentVolume;
    assert_eq!(
        quarantine_outcome(
            "detail-unlink-failed",
            RenameOutcome::Fallback(fallback),
            &[before, EntryIdentity::SameFile],
            false,
        ),
        QuarantineMove::CopiedOriginalKept {
            fallback,
            reason: "unlink-failed"
        }
    );
    assert_eq!(
        quarantine_outcome(
            "detail-already-gone",
            RenameOutcome::Fallback(fallback),
            &[before, EntryIdentity::Missing],
            true,
        ),
        QuarantineMove::CopiedOriginalKept {
            fallback,
            reason: "original-already-gone"
        }
    );
    assert_eq!(
        quarantine_outcome(
            "detail-changed",
            RenameOutcome::Fallback(fallback),
            &[before, EntryIdentity::DifferentFile],
            true,
        ),
        QuarantineMove::CopiedOriginalKept {
            fallback,
            reason: "original-changed-during-the-move"
        }
    );
    assert_eq!(
        quarantine_outcome(
            "detail-unverifiable",
            RenameOutcome::Fallback(fallback),
            &[EntryIdentity::Unverifiable, EntryIdentity::Unverifiable],
            true,
        ),
        QuarantineMove::CopiedOriginalKept {
            fallback,
            reason: "identity-check-unavailable"
        }
    );
}

/// The three fallback reasons' own tokens, which the payload carries
/// verbatim when the copy path removed the original.
#[test]
fn every_fallback_reason_keeps_its_fixed_token() {
    use omnifrons_app::quarantine::FallbackReason;
    assert_eq!(FallbackReason::DifferentVolume.as_str(), "different-volume");
    assert_eq!(FallbackReason::Unsupported.as_str(), "rename-unsupported");
    assert_eq!(FallbackReason::Failed.as_str(), "rename-failed");
}

/// R3-022: `QuarantineError::NotRegularFile` is reachable, and reached --
/// the handle's own `fstat` decides, so a character device opened as the
/// held handle is refused before anything moves. (Unix names one that is
/// always present; nothing is opened as a directory here.)
#[cfg(unix)]
#[test]
fn a_held_handle_that_is_not_a_regular_file_is_refused() {
    let project = ContractDir::new("not-regular-project");
    let quarantine_dir = ContractDir::new("not-regular-quarantine");
    let root = QuarantineRoot::open(&quarantine_dir.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = FakeHasher;
    let mut source = CandidateSource {
        dir: open_contract_dir(project.path()),
        dir_path: project.path().to_path_buf(),
        file_name: std::ffi::OsString::from("null"),
        handle: std::fs::File::open("/dev/null").expect("a character device opens"),
    };
    assert_eq!(
        quarantine(
            &QuarantinePorts {
                store: &ForcedCopyQuarantineStore::new(),
                hasher: &hasher,
                entry_ops: &StdEntryOps::new(),
                root: &root,
                workspaces: &[],
            },
            &mut source,
            &hasher.sha256(b""),
            &DisplayName::sanitize("null"),
        ),
        Err(QuarantineError::NotRegularFile)
    );
    assert_eq!(
        std::fs::read_dir(root.path()).expect("list").count(),
        0,
        "nothing reached the quarantine directory"
    );
}
