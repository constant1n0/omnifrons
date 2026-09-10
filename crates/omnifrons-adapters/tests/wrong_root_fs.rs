//! The real wrong-root adapters (spike slice 5d): `JsonlIgnoreLedger` over
//! the work area's `ignore/decisions.jsonl`, `FsQuarantineStore` over the
//! quarantine directory, and `FsWrongRootScanner` over a real worktree.
//!
//! Each runs `omnifrons-app`'s own reusable contract against itself, so
//! the fake and the real implementation cannot drift, and then the facts
//! only a real filesystem can show: persistence across a reopen, a corrupt
//! line failing the whole read closed, and owner-only permissions on unix.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::{
    FsOutboxEntryOps, FsQuarantineStore, FsWrongRootScanner, JsonlIgnoreLedger, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher as _;
use omnifrons_app::contract::wrong_root::{
    contract_bytes, contract_source, copy_in_contract, ignore_ledger_contract,
    quarantine_store_contract, sample_entry,
};
use omnifrons_app::ignore_ledger::{IgnoreLedger as _, IgnoreLedgerError};
use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_app::quarantine::{QuarantineError, QuarantinePorts, QuarantineRoot, quarantine};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_app::wrong_root::{
    MAX_SCAN_DEPTH, MAX_SCANNED_ENTRIES, OpenMisplacedError, ScanRequest, WrongRootScanner as _,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ArtifactClass, DetectedType};
use omnifrons_domain::publication::{DisplayName, ProjectIdentity};
use omnifrons_domain::wrong_root::{MisplacedFinding, WrongRootReason};

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-wrong-root-fs-{}-{label}-{n}",
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

fn work_area(label: &str) -> (TempDir, TempDir, WorkAreaRoot) {
    let base = TempDir::new(label);
    let project = TempDir::new(&format!("{label}-project"));
    let root = WorkAreaRoot::open(&base.path().join("work-area"), &[&project.workspace()])
        .expect("a fresh work area opens");
    (base, project, root)
}

#[test]
fn jsonl_ignore_ledger_satisfies_the_contract() {
    let (base, _project, root) = work_area("contract");
    ignore_ledger_contract(|| JsonlIgnoreLedger::open(&root).expect("the ledger opens"));
    drop(base);
}

/// The one fact only a real store can show: a decision recorded by one
/// handle is read back by the next, so an ignored file stays ignored
/// across a restart.
#[test]
fn a_recorded_decision_survives_a_reopen() {
    let (_base, _project, root) = work_area("reopen");
    let project = ProjectIdentity(Sha256Digest([5; 32]));
    let digest = Sha256Digest([6; 32]);

    let mut ledger = JsonlIgnoreLedger::open(&root).expect("open");
    ledger
        .record(&project, &sample_entry("docs/report.pdf", digest))
        .expect("record");
    drop(ledger);

    let reopened = JsonlIgnoreLedger::open(&root).expect("reopen");
    assert!(
        reopened
            .is_ignored(&project, "docs/report.pdf", &digest)
            .expect("read"),
        "an ignore decision survives a reopen"
    );
}

/// A line that does not parse fails the whole read closed
/// ([`IgnoreLedgerError::Corrupt`]), never "not ignored": a corrupt ledger
/// must not silently offer a file the user already dismissed.
#[test]
fn a_corrupt_line_fails_the_read_closed() {
    let (_base, _project, root) = work_area("corrupt");
    let project = ProjectIdentity(Sha256Digest([5; 32]));
    let digest = Sha256Digest([6; 32]);

    let mut ledger = JsonlIgnoreLedger::open(&root).expect("open");
    ledger
        .record(&project, &sample_entry("docs/report.pdf", digest))
        .expect("record");
    let path = root.ignore_dir().join(JsonlIgnoreLedger::FILE_NAME);
    let mut content = std::fs::read_to_string(&path).expect("read");
    content.push_str("{not json}\n");
    std::fs::write(&path, content).expect("write");

    assert_eq!(
        ledger.entries(&project),
        Err(IgnoreLedgerError::Corrupt),
        "a line that does not parse fails the whole read closed"
    );
}

/// R3-009: "corrupt" is not only "not JSON". A line of another schema, a
/// digest or project id that is not hex, a token no enum knows, and a field
/// this version never wrote are each a ledger this product cannot vouch
/// for, and each must fail the read **closed** -- a ledger read as "nothing
/// is ignored" silently re-offers every file the user already dismissed.
#[test]
fn every_shape_of_corruption_fails_the_read_closed() {
    let (_base, _project, root) = work_area("corrupt-shapes");
    let project = ProjectIdentity(Sha256Digest([5; 32]));
    let path = root.ignore_dir().join(JsonlIgnoreLedger::FILE_NAME);

    let good = |overrides: &str| {
        format!(
            r#"{{"schema":1,"projectId":"{}","name":"docs/report.pdf","sha256":"{}","size":12,"detectedType":"pdf","class":"generated-heavy","ignoredAt":{{"secs":1700000000,"nanos":0}}{overrides}}}"#,
            project.to_hex(),
            Sha256Digest([6; 32]).to_hex(),
        )
    };
    // The line this test mutates is itself a good one: if it were not, every
    // case below would pass for the wrong reason.
    std::fs::write(&path, format!("{}\n", good(""))).expect("write");
    let ledger = JsonlIgnoreLedger::open(&root).expect("open");
    assert_eq!(
        ledger.entries(&project).expect("a good line reads").len(),
        1,
        "the fixture line this test corrupts must itself be readable"
    );

    let corrupt = [
        ("not json at all", "{not json}".to_string()),
        (
            "another schema",
            good("").replace(r#""schema":1"#, r#""schema":2"#),
        ),
        (
            "a digest that is not 64 hex characters",
            good("").replace(&Sha256Digest([6; 32]).to_hex(), "not-hex"),
        ),
        (
            "a project id that is not 64 hex characters",
            good("").replace(&project.to_hex(), "0011"),
        ),
        (
            "a detected type no enum knows",
            good("").replace(r#""detectedType":"pdf""#, r#""detectedType":"pdf-ish""#),
        ),
        (
            "a class no enum knows",
            good("").replace(r#""class":"generated-heavy""#, r#""class":"heavyish""#),
        ),
        (
            "a field this version never wrote",
            good(r#","revoked":true"#),
        ),
    ];
    for (what, line) in corrupt {
        std::fs::write(&path, format!("{line}\n")).expect("write");
        assert_eq!(
            ledger.entries(&project),
            Err(IgnoreLedgerError::Corrupt),
            "{what} must fail the read closed"
        );
        assert_eq!(
            ledger.is_ignored(&project, "docs/report.pdf", &Sha256Digest([6; 32])),
            Err(IgnoreLedgerError::Corrupt),
            "{what} must never be answered as \"not ignored\""
        );
    }
}

/// Unix: the ledger file is owner-only, like the publication journal's.
#[cfg(unix)]
#[test]
fn the_ledger_file_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_base, _project, root) = work_area("mode");
    let mut ledger = JsonlIgnoreLedger::open(&root).expect("open");
    ledger
        .record(
            &ProjectIdentity(Sha256Digest([5; 32])),
            &sample_entry("docs/report.pdf", Sha256Digest([6; 32])),
        )
        .expect("record");
    let path = root.ignore_dir().join(JsonlIgnoreLedger::FILE_NAME);
    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "the ledger file must be owner-only");
}

// -- FsWrongRootScanner: the walk over a real worktree --

/// The two-byte PDF magic plus padding: enough for `DetectedType::detect`
/// to classify the bytes as a PDF from content alone.
fn pdf_bytes() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(&[0u8; 64]);
    bytes
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(&path, bytes).expect("fixture file");
}

/// HAP-001-R32 with HAP-001-R43 and R44: a heavy file anywhere in the
/// project except the outbox renders `misplaced`, with its digest and
/// class taken from its own handle; the outbox is read as an ingestion
/// source, never a wrong root; `.git/` is excluded; a `git-tracked` file
/// and a Markdown note are not misplaced at all.
#[test]
fn the_scan_reports_heavy_files_outside_the_outbox_and_nothing_else() {
    let project = TempDir::new("scan");
    let root = project.path();
    write(root, "docs/report.pdf", &pdf_bytes());
    write(root, "nested/deep/slides.pdf", &pdf_bytes());
    write(root, ".omnifrons/outbox/run-1/in-outbox.pdf", &pdf_bytes());
    // A real repository's metadata, which is what the exclusion is about:
    // `HEAD` and an object database, not just the name (R1-006).
    write(root, ".git/HEAD", b"ref: refs/heads/main\n");
    write(root, ".git/objects/blob.pdf", &pdf_bytes());
    write(root, "src/main.rs", b"fn main() {}\n");
    write(root, "README.md", b"# notes\n");

    let policy = OutboxPolicy::default_policy();
    let workspace = project.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("a real worktree scans");

    // Walk order, not sorted order: `ScanReport::findings` is documented as
    // being in walk order, and sorting the actual result before comparing
    // it asserts that documentation away (R3-015). Depth-first over names
    // sorted within each directory puts `docs/` before `nested/`.
    let names: Vec<&str> = report.findings.iter().map(MisplacedFinding::name).collect();
    assert_eq!(
        names,
        vec!["docs/report.pdf", "nested/deep/slides.pdf"],
        "only the heavy files outside the outbox are misplaced, in walk order"
    );
    for finding in &report.findings {
        assert_eq!(finding.class, ArtifactClass::GeneratedHeavy);
        assert_eq!(finding.detected_type, DetectedType::Pdf);
        assert_eq!(finding.reason, WrongRootReason::InProjectOutsideOutbox);
        assert_eq!(finding.size, pdf_bytes().len() as u64);
        assert_eq!(
            finding.digest,
            Sha2Hasher::new().sha256(&pdf_bytes()),
            "the digest is of the bytes read from the file's own handle"
        );
    }
    assert!(!report.truncated, "a small worktree is not truncated");
    // Determinate, not an inequality: the four files outside the outbox and
    // outside the repository's metadata, and nothing else.
    assert_eq!(
        report.scanned, 4,
        "every non-excluded file was examined, and only those"
    );
    assert_eq!(
        report.excluded, 2,
        "the declared outbox and the repository's metadata directory, each skipped once as a \
         whole subtree"
    );
    assert_eq!(report.unreadable, 0);
}

/// The scan never dereferences a link (the same discipline the outbox
/// inventory follows, HAP-001-R12): a link to a heavy file outside the
/// project is skipped, never reported as a finding inside it.
#[cfg(unix)]
#[test]
fn the_scan_never_follows_a_link() {
    let project = TempDir::new("scan-link");
    let outside = TempDir::new("scan-link-outside");
    write(outside.path(), "real.pdf", &pdf_bytes());
    std::os::unix::fs::symlink(
        outside.path().join("real.pdf"),
        project.path().join("link.pdf"),
    )
    .expect("fixture link");
    std::os::unix::fs::symlink(outside.path(), project.path().join("linked-dir"))
        .expect("fixture directory link");

    let policy = OutboxPolicy::default_policy();
    let workspace = project.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        report.findings.is_empty(),
        "a link is never dereferenced, so it can never produce a finding: {:?}",
        report
            .findings
            .iter()
            .map(MisplacedFinding::name)
            .collect::<Vec<_>>()
    );
}

// -- FsQuarantineStore and the publish remedy's copy-in --

/// The real store runs the same contract the app crate runs against its
/// copy-forcing double: here the source and the quarantine directory sit
/// on one volume, so the handle-anchored rename branch is the one
/// exercised on unix.
#[test]
fn fs_quarantine_store_satisfies_the_quarantine_contract() {
    use omnifrons_app::quarantine::QuarantineMove;
    // Both ends on one volume, so the handle-anchored `renameat` is the
    // branch this run takes on unix; Windows has no rename relative to two
    // directory handles and no relative unlink, so the copy stands and the
    // original is kept. Fixed by the caller rather than accepted as a
    // disjunction (R3-008).
    #[cfg(unix)]
    let expected = QuarantineMove::Renamed;
    #[cfg(not(unix))]
    let expected = QuarantineMove::CopiedOriginalKept {
        fallback: omnifrons_app::quarantine::FallbackReason::Unsupported,
        reason: "identity-check-unavailable",
    };
    quarantine_store_contract(
        &FsQuarantineStore::new(),
        &FsOutboxEntryOps::new(),
        &expected,
    );
}

/// The copy-in contract again, over the real by-handle entry ops.
#[test]
fn the_copy_in_satisfies_its_contract_over_the_real_entry_ops() {
    copy_in_contract(&FsOutboxEntryOps::new());
}

/// The rename branch is really the one the real store takes when both ends
/// sit on one volume, and it is anchored to the two directory handles: the
/// source name is gone and the destination holds the same inode.
#[cfg(unix)]
#[test]
fn one_volume_takes_the_handle_anchored_rename_branch() {
    use omnifrons_app::quarantine::QuarantineMove;
    use std::os::unix::fs::MetadataExt as _;
    let project = TempDir::new("rename-project");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");
    let before = std::fs::metadata(project.path().join("report.pdf")).expect("metadata");

    let quarantine_base = TempDir::new("rename-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = Sha2Hasher::new();
    let expected = hasher.sha256(&bytes);
    let mut source = contract_source(project.path(), "report.pdf");

    let quarantined = quarantine(
        &QuarantinePorts {
            store: &FsQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("the move succeeds");

    assert_eq!(
        quarantined.outcome,
        QuarantineMove::Renamed,
        "one volume takes the handle-anchored rename, not the copy fallback"
    );
    let after = std::fs::metadata(root.path().join(&quarantined.name)).expect("metadata");
    assert_eq!(
        (before.dev(), before.ino()),
        (after.dev(), after.ino()),
        "a rename moves the same file, it does not make a second one"
    );
    assert!(!project.path().join("report.pdf").exists());
}

/// R1-021: the handle-anchored branch **never replaces a name something
/// else already holds**, which is the promise the copy branch already keeps
/// and the module comment already makes for both.
///
/// POSIX `renameat` replaces an existing destination without a word, so the
/// branch that actually runs on unix, same volume -- the normal case -- was
/// the one that could destroy a file. It is reachable, not theoretical: the
/// destination name is an 8-hex short digest (`SHORT_HEX_CHARS`) and a
/// sanitized display name, both of them chosen by whoever misplaced the
/// file, so a 32-bit prefix collision against an already-quarantined entry
/// is brute-forceable -- and the victim is a file the user deliberately
/// preserved whose original was already unlinked.
///
/// Unix-gated because the branch itself is: Windows has no move relative to
/// two directory handles in `std`, and what it does instead is stated by
/// `a_platform_without_a_handle_anchored_rename_reports_the_fallback` below
/// -- the copy path, whose own destination is already pinned by
/// `the_copy_paths_destination_name_is_created_exclusively_and_never_replaced`
/// in `omnifrons-app`, which runs on every platform.
#[cfg(unix)]
#[test]
fn the_handle_anchored_branch_never_replaces_a_pre_existing_destination() {
    use omnifrons_app::quarantine::quarantine_name;
    let project = TempDir::new("squat-project");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");

    let quarantine_base = TempDir::new("squat-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = Sha2Hasher::new();
    let expected = hasher.sha256(&bytes);
    let display = DisplayName::sanitize("report.pdf");
    let name = quarantine_name(&display, &expected);

    // The name is already held, by other bytes and a different length.
    let planted = b"a file the user deliberately preserved".to_vec();
    std::fs::write(root.path().join(&name), &planted).expect("the planted destination");

    let mut source = contract_source(project.path(), "report.pdf");
    let refused = quarantine(
        &QuarantinePorts {
            store: &FsQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &display,
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

/// R1-021's other half: a destination that already carries **these** bytes
/// is an earlier attempt of this same remedy, so it is verified and reused
/// idempotently rather than replaced -- exactly what the copy branch does
/// with the same name, and the reason `AlreadyExists` is not a refusal.
///
/// Unix-gated for the reason the test above gives: on Windows there is no
/// handle-anchored branch to reach this through.
#[cfg(unix)]
#[test]
fn the_handle_anchored_branch_reuses_a_destination_that_already_carries_these_bytes() {
    use omnifrons_app::quarantine::{FallbackReason, QuarantineMove, quarantine_name};
    let project = TempDir::new("idempotent-project");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");

    let quarantine_base = TempDir::new("idempotent-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = Sha2Hasher::new();
    let expected = hasher.sha256(&bytes);
    let display = DisplayName::sanitize("report.pdf");
    let name = quarantine_name(&display, &expected);
    std::fs::write(root.path().join(&name), &bytes).expect("an earlier attempt's result");

    let mut source = contract_source(project.path(), "report.pdf");
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &FsQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &display,
    )
    .expect("the same bytes at the name are this remedy's own earlier result");

    assert_eq!(
        quarantined.outcome,
        QuarantineMove::CopiedAndUnlinked {
            fallback: FallbackReason::Failed
        },
        "the exclusive create refused the held name, so the copy path ran and reused it"
    );
    assert_eq!(
        std::fs::read(root.path().join(&name)).expect("the quarantined file"),
        bytes
    );
    assert!(
        !project.path().join("report.pdf").exists(),
        "the original is removed once the destination verifies"
    );
    assert_eq!(
        std::fs::read_dir(root.path()).expect("list").count(),
        1,
        "and no second copy is left in the quarantine directory"
    );
}

/// The Windows counterpart of the branch above: `std` offers no rename
/// relative to two directory handles and this repository adds no Windows
/// API dependency, so the store reports the fallback rather than pretending
/// to a move it cannot make, and the copy-then-unlink path runs -- with the
/// unlink itself unavailable, so the original is kept and the outcome says
/// so. A residual disclosed under HAP-001-R19, not a claim.
#[cfg(not(unix))]
#[test]
fn a_platform_without_a_handle_anchored_rename_reports_the_fallback() {
    use omnifrons_app::quarantine::{
        FallbackReason, QuarantineMove, QuarantineStore as _, RenameOutcome,
    };
    let project = TempDir::new("no-rename-project");
    std::fs::write(project.path().join("report.pdf"), contract_bytes()).expect("fixture");
    let quarantine_base = TempDir::new("no-rename-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let store = FsQuarantineStore::new();
    let dest_dir = store
        .open_dir(&root)
        .expect("the quarantine directory opens");
    let source = contract_source(project.path(), "report.pdf");
    assert_eq!(
        store.rename_into(
            &source,
            &dest_dir,
            root.path(),
            std::ffi::OsStr::new("moved.pdf")
        ),
        RenameOutcome::Fallback(FallbackReason::Unsupported)
    );

    let hasher = Sha2Hasher::new();
    let expected = hasher.sha256(&contract_bytes());
    let mut source = contract_source(project.path(), "report.pdf");
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &store,
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            root: &root,
            workspaces: &[],
        },
        &mut source,
        &expected,
        &DisplayName::sanitize("report.pdf"),
    )
    .expect("the copy path still moves the bytes");
    assert!(matches!(
        quarantined.outcome,
        QuarantineMove::CopiedOriginalKept { .. }
    ));
    assert!(project.path().join("report.pdf").exists());
}

/// HAP-001-R32: "a path swap during the remedy is refused". The name is
/// pointed at another file after the handle was opened; nothing moves, and
/// the quarantine directory stays empty.
#[cfg(unix)]
#[test]
fn a_path_swap_during_the_remedy_is_refused() {
    let project = TempDir::new("swap-project");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");

    let quarantine_base = TempDir::new("swap-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = Sha2Hasher::new();
    let expected = hasher.sha256(&bytes);
    let mut source = contract_source(project.path(), "report.pdf");

    // The handle stays open on the original; the *name* now holds another
    // file entirely.
    std::fs::remove_file(project.path().join("report.pdf")).expect("unlink");
    std::fs::write(project.path().join("report.pdf"), b"other bytes").expect("swap");

    assert_eq!(
        quarantine(
            &QuarantinePorts {
                store: &FsQuarantineStore::new(),
                hasher: &hasher,
                entry_ops: &FsOutboxEntryOps::new(),
                root: &root,
                workspaces: &[],
            },
            &mut source,
            &expected,
            &DisplayName::sanitize("report.pdf"),
        ),
        Err(QuarantineError::PathChanged),
        "a name that no longer holds the opened file refuses the remedy"
    );
    assert_eq!(
        std::fs::read_dir(root.path()).expect("list").count(),
        0,
        "nothing reached the quarantine directory"
    );
    assert_eq!(
        std::fs::read(project.path().join("report.pdf")).expect("read"),
        b"other bytes",
        "the substituted file is untouched"
    );
}

/// **A source name swapped *before* the `linkat` linked the intruder, not
/// the approved file** (spike slice 5d, R1-028), and the outcome says so
/// rather than reporting a move.
///
/// `linkat` resolves the source *name*, never the handle this remedy
/// holds, and the guard that follows reads that same name -- so the two
/// races it was written to separate end in the same state on the source
/// side: the name holds another file either way. Only the destination
/// tells them apart. Linked after the swap it is a second name for a file
/// the product was never asked about, while the approved file is left with
/// none at all.
///
/// Arranged at the window's end state rather than raced, exactly as
/// `a_path_swap_during_the_remedy_is_refused` above arranges its own:
/// `rename_into` is a trait method this suite already calls directly.
///
/// Unix-gated because there is no handle-anchored branch to reach this
/// through on Windows, where `rename_anchored` reports `Unsupported`
/// before linking anything --
/// `a_platform_without_a_handle_anchored_rename_reports_the_fallback`
/// above is that half.
#[cfg(unix)]
#[test]
fn a_source_name_swapped_before_the_link_is_not_reported_as_a_move() {
    use omnifrons_app::quarantine::{FallbackReason, QuarantineStore as _, RenameOutcome};

    let project = TempDir::new("pre-link-swap-project");
    std::fs::write(project.path().join("report.pdf"), contract_bytes()).expect("fixture");
    let quarantine_base = TempDir::new("pre-link-swap-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let store = FsQuarantineStore::new();
    let dest_dir = store
        .open_dir(&root)
        .expect("the quarantine directory opens");

    // The handle stays open on the approved file; the *name* holds an
    // intruder by the time the move runs, which is where the pre-`linkat`
    // half of the window ends.
    let source = contract_source(project.path(), "report.pdf");
    std::fs::remove_file(project.path().join("report.pdf")).expect("unlink");
    std::fs::write(project.path().join("report.pdf"), b"intruder bytes").expect("swap");

    assert_eq!(
        store.rename_into(
            &source,
            &dest_dir,
            root.path(),
            std::ffi::OsStr::new("moved.pdf"),
        ),
        RenameOutcome::Fallback(FallbackReason::Failed),
        "the link this call made is not the held file's name, so nothing moved"
    );
    assert_eq!(
        std::fs::read_dir(root.path()).expect("list").count(),
        0,
        "the link this call created is undone rather than left as a second \
         name for a file nobody asked about"
    );
    assert_eq!(
        std::fs::read(project.path().join("report.pdf")).expect("read"),
        b"intruder bytes",
        "and the substituted file is untouched at its own name"
    );
}

/// The remedy binds to the digest the surface showed: content **rewritten
/// in place** under the held handle refuses rather than quarantining bytes
/// the user never saw.
///
/// The rewrite is the point (R3-019): a digest of bytes the file never
/// carried would prove only that two digests differ, not that a file
/// changing under a held handle is noticed.
#[test]
fn content_that_changed_since_it_was_found_refuses_the_remedy() {
    let project = TempDir::new("changed-project");
    let original = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &original).expect("fixture");
    let quarantine_base = TempDir::new("changed-quarantine");
    let root = QuarantineRoot::open(&quarantine_base.path().join("quarantine"), &[])
        .expect("a quarantine root");
    let hasher = Sha2Hasher::new();
    let shown = hasher.sha256(&original);
    let mut source = contract_source(project.path(), "report.pdf");

    // Rewritten in place, so the name still holds the very file the handle
    // was opened on -- HAP-001-R18's check passes and only the content
    // check can catch this.
    let mut rewritten = contract_bytes();
    rewritten.extend_from_slice(b"and one more paragraph");
    std::fs::write(project.path().join("report.pdf"), &rewritten).expect("the rewrite");

    assert_eq!(
        quarantine(
            &QuarantinePorts {
                store: &FsQuarantineStore::new(),
                hasher: &hasher,
                entry_ops: &FsOutboxEntryOps::new(),
                root: &root,
                workspaces: &[],
            },
            &mut source,
            &shown,
            &DisplayName::sanitize("report.pdf"),
        ),
        Err(QuarantineError::DigestChanged),
        "the remedy binds to the digest the surface showed"
    );
    assert_eq!(
        std::fs::read(project.path().join("report.pdf")).expect("read"),
        rewritten,
        "the refused file is left exactly as it was found"
    );
    assert_eq!(
        std::fs::read_dir(root.path()).expect("list").count(),
        0,
        "nothing reached the quarantine directory"
    );
}

/// A [`ContentHasher`] that counts the streams it was asked to digest, so a
/// test can pin that a remedy never *reads* a file it is going to refuse.
struct CountingHasher {
    inner: Sha2Hasher,
    reads: std::sync::atomic::AtomicUsize,
}

impl CountingHasher {
    fn new() -> Self {
        Self {
            inner: Sha2Hasher::new(),
            reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl omnifrons_app::content_hasher::ContentHasher for CountingHasher {
    fn sha256(&self, input: &[u8]) -> Sha256Digest {
        self.inner.sha256(input)
    }

    fn digest_reader(
        &self,
        reader: &mut dyn std::io::Read,
    ) -> std::io::Result<(Sha256Digest, u64)> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.digest_reader(reader)
    }
}

/// R1-016: the publish remedy asks whether its entry is already in the
/// outbox by **opening the name itself**, relative to the outbox handle and
/// without following a link at it -- never by a `symlink_metadata` that a
/// following `File::open` can then disagree with.
///
/// The outbox root is the one directory HAP-001 declares hostile, and a
/// check-then-open there is a window: whoever wins it chooses which file
/// the remedy opens and hashes, and a large one makes it read unbounded
/// bytes. The window itself cannot be raced deterministically from a
/// synchronous test; what is arranged here is the state the two halves
/// disagree about. A hard link at the entry's name is a regular file to
/// `symlink_metadata` -- so the check says "reuse it" and the open reads a
/// file that answers to another name outside the outbox -- while a
/// no-follow open relative to the outbox handle reads the link count from
/// that one handle and refuses it before a byte is read (HAP-001-R20).
#[cfg(unix)]
#[test]
fn an_entry_already_at_the_name_is_refused_from_its_own_handle_and_never_read() {
    use omnifrons_app::quarantine::quarantine_name;
    use omnifrons_app::run_outbox::OpenedOutbox;
    use omnifrons_app::wrong_root::{CopyInError, CopyInPorts, copy_in_from_misplaced};

    let project = TempDir::new("copy-in-planted-project");
    let outside = TempDir::new("copy-in-planted-outside");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");
    let outbox_path = project.path().join("outbox");
    std::fs::create_dir_all(&outbox_path).expect("fixture outbox");

    let hasher = CountingHasher::new();
    let expected = Sha2Hasher::new().sha256(&bytes);
    let display = DisplayName::sanitize("report.pdf");

    // Something already holds the entry's name, and it is a second name for
    // a file that lives outside the outbox entirely.
    std::fs::write(outside.path().join("elsewhere.pdf"), &bytes).expect("fixture");
    std::fs::hard_link(
        outside.path().join("elsewhere.pdf"),
        outbox_path.join(quarantine_name(&display, &expected)),
    )
    .expect("the fixture hard link");

    let outbox = OpenedOutbox {
        path: outbox_path.clone(),
        handle: omnifrons_app::contract::wrong_root::open_contract_dir(&outbox_path),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    let refused = copy_in_from_misplaced(
        &CopyInPorts {
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            prober: &omnifrons_adapters::FsCandidateProber::new(),
        },
        &mut source,
        &outbox,
        &expected,
        &display,
    );

    assert_eq!(
        refused.err(),
        Some(CopyInError::EntryExists),
        "a name already held by something this remedy did not create is refused as such"
    );
    // What this counts is the remedy's own `ContentHasher` port -- the one
    // the removed `symlink_metadata`-then-`File::open` fed. Zero means the
    // remedy never opened and streamed the planted file itself; the prober
    // refuses it from the link count on its one handle, before its own
    // digest pass starts.
    assert_eq!(
        hasher.reads(),
        0,
        "and it is refused from the entry's own handle, without the remedy reading a byte of it"
    );
}

/// The Windows counterpart of the refusal above. The by-handle link-count
/// accessor in `std::os::windows::fs::MetadataExt` is unstable and this
/// repository adds no Windows API dependency, so the entry planted at the
/// name cannot be recognized as answering to a second one and, carrying the
/// expected bytes, is reused as this remedy's own idempotent result. A
/// residual disclosed under HAP-001-R19 and `docs/spike-log.md` § Slice 5d,
/// not a claim -- what the no-follow open still buys on that platform is
/// that a *reparse point* at the name is refused rather than followed.
#[cfg(not(unix))]
#[test]
fn a_platform_without_a_by_handle_link_count_reuses_a_linked_entry() {
    use omnifrons_app::quarantine::quarantine_name;
    use omnifrons_app::run_outbox::OpenedOutbox;
    use omnifrons_app::wrong_root::{CopyInPorts, copy_in_from_misplaced};

    let project = TempDir::new("copy-in-planted-project");
    let outside = TempDir::new("copy-in-planted-outside");
    let bytes = contract_bytes();
    std::fs::write(project.path().join("report.pdf"), &bytes).expect("fixture");
    let outbox_path = project.path().join("outbox");
    std::fs::create_dir_all(&outbox_path).expect("fixture outbox");

    let hasher = CountingHasher::new();
    let expected = Sha2Hasher::new().sha256(&bytes);
    let display = DisplayName::sanitize("report.pdf");
    let name = quarantine_name(&display, &expected);

    std::fs::write(outside.path().join("elsewhere.pdf"), &bytes).expect("fixture");
    std::fs::hard_link(
        outside.path().join("elsewhere.pdf"),
        outbox_path.join(&name),
    )
    .expect("the fixture hard link");

    let outbox = OpenedOutbox {
        path: outbox_path.clone(),
        handle: omnifrons_app::contract::wrong_root::open_contract_dir(&outbox_path),
    };
    let mut source = contract_source(project.path(), "report.pdf");
    let copied = copy_in_from_misplaced(
        &CopyInPorts {
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            prober: &omnifrons_adapters::FsCandidateProber::new(),
        },
        &mut source,
        &outbox,
        &expected,
        &display,
    )
    .expect("without a link count the entry cannot be refused");
    assert_eq!(copied.name, name);
    assert_eq!(
        hasher.reads(),
        0,
        "the entry is still recognized from its own handle, not by a check-then-open"
    );
}

/// The remedy surface names a finding by its project-relative name, and the
/// file behind that name is re-opened under exactly the single-handle
/// discipline an outbox entry is (HAP-001-R15): one no-follow open per
/// component, every fact from the one handle, nothing dereferenced.
#[test]
fn a_finding_is_reopened_by_its_relative_name_under_the_single_handle_discipline() {
    let project = TempDir::new("open-misplaced");
    let bytes = contract_bytes();
    std::fs::create_dir_all(project.path().join("docs")).expect("fixture dir");
    std::fs::write(project.path().join("docs/report.pdf"), &bytes).expect("fixture");
    let workspace = project.workspace();
    let scanner = FsWrongRootScanner::new();

    let mut source = scanner
        .open_misplaced(&workspace, "docs/report.pdf")
        .expect("a regular file at a project-relative name opens");
    assert_eq!(source.file_name, std::ffi::OsString::from("report.pdf"));
    let (digest, size) = Sha2Hasher::new()
        .digest_reader(&mut source.handle)
        .expect("the handle reads");
    assert_eq!(digest, Sha2Hasher::new().sha256(&bytes));
    assert_eq!(size, bytes.len() as u64);

    assert_eq!(
        scanner.open_misplaced(&workspace, "docs/missing.pdf").err(),
        Some(OpenMisplacedError::NotFound)
    );
    assert_eq!(
        scanner.open_misplaced(&workspace, "docs").err(),
        Some(OpenMisplacedError::NotRegularFile),
        "a directory is not a file a remedy can act on"
    );
    for name in ["/etc/passwd", "../escape.pdf"] {
        assert_eq!(
            scanner.open_misplaced(&workspace, name).err(),
            Some(OpenMisplacedError::InvalidName),
            "{name} is not a project-relative name"
        );
    }
}

/// A link at the named file, and a link standing in for a directory
/// component, are both refused rather than dereferenced.
#[cfg(unix)]
#[test]
fn a_link_on_the_way_to_a_finding_is_refused() {
    let project = TempDir::new("open-misplaced-link");
    let outside = TempDir::new("open-misplaced-outside");
    std::fs::write(outside.path().join("real.pdf"), contract_bytes()).expect("fixture");
    std::os::unix::fs::symlink(
        outside.path().join("real.pdf"),
        project.path().join("link.pdf"),
    )
    .expect("fixture link");
    std::os::unix::fs::symlink(outside.path(), project.path().join("linked"))
        .expect("fixture dir link");

    let workspace = project.workspace();
    let scanner = FsWrongRootScanner::new();
    assert_eq!(
        scanner.open_misplaced(&workspace, "link.pdf").err(),
        Some(OpenMisplacedError::NotRegularFile),
        "a link at the name is never dereferenced"
    );
    assert_eq!(
        scanner.open_misplaced(&workspace, "linked/real.pdf").err(),
        Some(OpenMisplacedError::NotFound),
        "a link standing in for a directory component is never traversed"
    );
}

/// R1-003 / HAP-001-R20, applied with the misplaced file as the candidate
/// (HAP-001-R32): re-opening a finding whose file carries more than one
/// name is refused. A hard link inside the project to a file outside it
/// would otherwise be quarantined -- unlinking the project's name while the
/// bytes live on -- or copied into the outbox as this project's own output.
#[cfg(unix)]
#[test]
fn a_finding_whose_file_has_more_than_one_name_is_refused() {
    let project = TempDir::new("open-misplaced-linked");
    let outside = TempDir::new("open-misplaced-linked-outside");
    std::fs::create_dir_all(project.path().join("docs")).expect("fixture dir");
    std::fs::write(outside.path().join("real.pdf"), contract_bytes()).expect("fixture");
    std::fs::hard_link(
        outside.path().join("real.pdf"),
        project.path().join("docs/report.pdf"),
    )
    .expect("the fixture hard link");

    let workspace = project.workspace();
    assert_eq!(
        FsWrongRootScanner::new()
            .open_misplaced(&workspace, "docs/report.pdf")
            .err(),
        Some(OpenMisplacedError::Linked { link_count: 2 }),
        "a candidate with more than one link is refused (HAP-001-R20)"
    );
}

/// The Windows counterpart of the refusal above: the by-handle link-count
/// accessor in `std::os::windows::fs::MetadataExt` is unstable and this
/// repository adds no Windows API dependency, so the count cannot be read
/// and the open proceeds. A residual disclosed under HAP-001-R19 and
/// `docs/spike-log.md` § Slice 5d, not a claim.
#[cfg(not(unix))]
#[test]
fn a_platform_without_a_by_handle_link_count_cannot_refuse_a_linked_finding() {
    let project = TempDir::new("open-misplaced-linked");
    let outside = TempDir::new("open-misplaced-linked-outside");
    std::fs::create_dir_all(project.path().join("docs")).expect("fixture dir");
    std::fs::write(outside.path().join("real.pdf"), contract_bytes()).expect("fixture");
    std::fs::hard_link(
        outside.path().join("real.pdf"),
        project.path().join("docs/report.pdf"),
    )
    .expect("the fixture hard link");

    let workspace = project.workspace();
    let source = FsWrongRootScanner::new()
        .open_misplaced(&workspace, "docs/report.pdf")
        .expect("without a link count the open cannot refuse");
    assert_eq!(source.file_name, std::ffi::OsString::from("report.pdf"));
}

// -- The walk's bounds and what it holds open (R1-002, R3-001, R3-005) --

/// A classifier that answers exactly as the policy does and, on every
/// call, samples how many file descriptors this process holds open --
/// which is the only way to observe what the walk holds *while it is
/// walking*, since nothing survives its return.
#[cfg(target_os = "linux")]
struct DescriptorWatchingPolicy {
    policy: OutboxPolicy,
    peak: std::sync::atomic::AtomicUsize,
}

#[cfg(target_os = "linux")]
fn open_descriptors() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("this platform lists its own descriptors")
        .count()
}

#[cfg(target_os = "linux")]
impl omnifrons_app::outbox_policy::ArtifactClassifier for DescriptorWatchingPolicy {
    fn classify(&self, name: &str, detected: DetectedType, size: u64) -> ArtifactClass {
        self.peak
            .fetch_max(open_descriptors(), std::sync::atomic::Ordering::Relaxed);
        omnifrons_app::outbox_policy::ArtifactClassifier::classify(
            &self.policy,
            name,
            detected,
            size,
        )
    }
}

/// R1-002: the walk holds handles for the path it is *on*, not for the
/// frontier it has yet to visit. A breadth-first walk that queues an open
/// handle per pending directory needs one descriptor per sibling, so a
/// worktree with a few hundred sibling directories -- a `node_modules`, a
/// build tree -- exhausts a 256-descriptor process before it has looked at
/// a single file, and every finding under the directories it then cannot
/// open is lost while `truncated` still reads `false`.
///
/// Linux-only because `/proc/self/fd` is where a process can see its own
/// descriptors; the property it pins is platform-independent.
#[cfg(target_os = "linux")]
#[test]
fn a_wide_tree_is_walked_without_holding_the_whole_frontier_open() {
    const SIBLINGS: usize = 400;
    let project = TempDir::new("wide");
    for n in 0..SIBLINGS {
        write(
            project.path(),
            &format!("dir-{n:04}/report.pdf"),
            &pdf_bytes(),
        );
    }

    let watching = DescriptorWatchingPolicy {
        policy: OutboxPolicy::default_policy(),
        peak: std::sync::atomic::AtomicUsize::new(0),
    };
    let baseline = open_descriptors();
    let workspace = project.workspace();
    let outbox = OutboxPolicy::default_policy();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: outbox.outbox(),
            classifier: &watching,
        })
        .expect("a wide worktree scans");

    assert_eq!(
        report.findings.len(),
        SIBLINGS,
        "every misplaced file under every sibling directory is reported"
    );
    assert!(!report.truncated);
    assert_eq!(report.unreadable, 0);
    let peak = watching.peak.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        peak <= baseline + 128,
        "the walk held {peak} descriptors against a {baseline} baseline over {SIBLINGS} sibling \
         directories: it is holding the frontier open, not the path it is on"
    );
}

/// R3-001: a directory is an entry the walk examined, so it counts against
/// the entry cap like any other. Counting only files lets a tree of empty
/// directories walk without bound while the cap that is supposed to stop it
/// never fires -- and the report still calls itself complete.
#[test]
fn directories_count_against_the_entry_cap_and_past_it_the_report_says_truncated() {
    let project = TempDir::new("entry-cap");
    for n in 0..MAX_SCANNED_ENTRIES {
        std::fs::create_dir(project.path().join(format!("d{n:06}"))).expect("fixture dir");
    }
    let policy = OutboxPolicy::default_policy();
    let workspace = project.workspace();

    let at_the_cap = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        !at_the_cap.truncated,
        "exactly the cap's worth of entries is not truncated"
    );

    std::fs::create_dir(project.path().join("d999999")).expect("one entry past the cap");
    let past_the_cap = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        past_the_cap.truncated,
        "one entry past the cap stops the walk and says so"
    );
}

/// The depth bound, at it and one past it: a tree the walk cannot see the
/// bottom of reports `truncated: true` rather than a clean scan of the part
/// it did see (R3-005).
#[test]
fn the_depth_bound_holds_at_it_and_truncates_one_past_it() {
    let deep = |levels: usize| {
        let mut parts: Vec<String> = (0..levels).map(|level| format!("l{level}")).collect();
        parts.push("report.pdf".to_string());
        parts.join("/")
    };

    let at_the_cap = TempDir::new("depth-at");
    write(at_the_cap.path(), &deep(MAX_SCAN_DEPTH), &pdf_bytes());
    let policy = OutboxPolicy::default_policy();
    let workspace = at_the_cap.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        !report.truncated,
        "the deepest allowed level is still walked"
    );
    assert_eq!(report.findings.len(), 1);

    let past_the_cap = TempDir::new("depth-past");
    write(past_the_cap.path(), &deep(MAX_SCAN_DEPTH + 1), &pdf_bytes());
    let workspace = past_the_cap.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        report.truncated,
        "a scan that could not see the whole tree says so"
    );
    assert!(
        report.findings.is_empty(),
        "and the file below the bound is not in it"
    );
}

/// R3-003: a heavy file whose *name* a finding may not carry -- a control
/// character is legal in a unix file name -- is counted and surfaced, never
/// dropped into a report that then reads as a clean scan.
#[cfg(unix)]
#[test]
fn a_heavy_file_at_a_name_no_finding_may_carry_is_counted_not_dropped() {
    let project = TempDir::new("unreportable-name");
    write(project.path(), "docs/re\nport.pdf", &pdf_bytes());

    let policy = OutboxPolicy::default_policy();
    let workspace = project.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    assert!(
        report.findings.is_empty(),
        "a name no finding may carry produces no finding"
    );
    assert_eq!(
        report.unreadable, 1,
        "but the file is counted, so the report never reads as a clean scan"
    );
}

/// R1-006: Git's metadata is excluded because it is a repository's, not
/// because it is spelled `.git`. A producer writing into the worktree the
/// scan polices must not be able to carve its own blind spot out of the
/// verdict with one `mkdir`.
#[test]
fn only_a_real_repositorys_metadata_directory_is_excluded_from_the_walk() {
    let project = TempDir::new("git-dir");
    // A real repository's metadata: HEAD and an objects database.
    write(project.path(), ".git/HEAD", b"ref: refs/heads/main\n");
    write(project.path(), ".git/objects/ab/cdef", &pdf_bytes());
    // A bare `mkdir` wearing the same name, with a heavy file hidden in it.
    write(project.path(), "vendor/.git/hidden.pdf", &pdf_bytes());
    write(project.path(), "docs/report.pdf", &pdf_bytes());

    let policy = OutboxPolicy::default_policy();
    let workspace = project.workspace();
    let report = FsWrongRootScanner::new()
        .scan(&ScanRequest {
            project: &workspace,
            outbox: policy.outbox(),
            classifier: &policy,
        })
        .expect("scan");
    let names: Vec<&str> = report.findings.iter().map(MisplacedFinding::name).collect();
    assert_eq!(
        names,
        vec!["docs/report.pdf", "vendor/.git/hidden.pdf"],
        "a real repository's metadata is excluded; a directory merely named .git is not"
    );
}
