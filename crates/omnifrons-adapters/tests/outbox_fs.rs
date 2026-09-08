//! The filesystem outbox adapters (spike slice 5): `FsRunOutboxPreparer`
//! (create-exclusive run subdirectory, owner-only, verified by handle),
//! `FsCandidateProber` (open once without following links, every fact
//! and the digest from that handle), and `FsOutboxInventory` (list without
//! following links, descend one level by handle). The shared port
//! contracts run first; the adapter-specific cases follow.
//!
//! Platform notes: the symbolic-link, hard-link, and FIFO fixtures are
//! unix only, gated per test (creating a symbolic link on Windows needs a
//! privilege the CI runner does not grant, and a FIFO has no Windows
//! equivalent); the link-count check itself is a disclosed residual on
//! Windows, where no stable `std` accessor reads it from a handle
//! (`docs/spike-log.md` § Slice 5).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::{FsCandidateProber, FsOutboxInventory, FsRunOutboxPreparer};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::contract::run_outbox::{
    candidate_prober_contract, run_outbox_preparer_contract,
};
use omnifrons_app::run_outbox::{
    CandidateProbe, CandidateProber, EscapeReason, InventoryError, OutboxDeclarationError,
    OutboxInventory, PrepareError, RunOutboxPreparer,
};
use omnifrons_domain::outbox::{DetectedType, OutboxPath, RunId};
use sha2::{Digest as _, Sha256};

/// A drop-guard temp directory standing in for a project root.
struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-outbox-fs-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test project directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.0).expect("the test project must be a valid workspace")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(token: &str) -> RunId {
    RunId::new(token).expect("valid run id")
}

// -- the shared contracts --

#[test]
fn fs_preparer_satisfies_the_run_outbox_preparer_contract() {
    run_outbox_preparer_contract(FsRunOutboxPreparer::new);
}

#[test]
fn fs_prober_satisfies_the_candidate_prober_contract() {
    candidate_prober_contract(FsRunOutboxPreparer::new, FsCandidateProber::new);
}

// -- the preparer --

#[test]
fn open_outbox_without_creating_reports_a_missing_outbox() {
    let project = TempProject::new("missing");
    let result = FsRunOutboxPreparer::new().open_outbox(
        &project.workspace(),
        &OutboxPath::default_path(),
        false,
    );
    assert!(
        matches!(result, Err(PrepareError::OutboxMissing)),
        "got {result:?}"
    );
    assert!(
        !project.path().join(".omnifrons").exists(),
        "nothing is created"
    );
}

#[test]
fn open_outbox_creates_the_default_outbox_when_asked_and_returns_its_canonical_path() {
    let project = TempProject::new("create");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("creating the default outbox must succeed");
    assert_eq!(
        outbox.path,
        std::fs::canonicalize(project.path().join(".omnifrons/outbox")).expect("canonical")
    );
    assert!(outbox.handle.metadata().expect("fstat").is_dir());
}

#[test]
fn a_regular_file_at_the_declared_outbox_blocks_preparation_as_a_declaration_error() {
    let project = TempProject::new("file-at-outbox");
    std::fs::create_dir_all(project.path().join(".omnifrons")).expect("fixture");
    std::fs::write(project.path().join(".omnifrons/outbox"), b"file").expect("fixture");
    let result = FsRunOutboxPreparer::new().prepare(
        &project.workspace(),
        &OutboxPath::default_path(),
        &run("run-1"),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::Declaration(OutboxDeclarationError::NotADirectory)
    );
}

/// Unix only (symlink fixture): a declared outbox that is a link blocks
/// every launch under it (HAP-001-R8), and nothing is created through it.
#[cfg(unix)]
#[test]
fn a_declared_outbox_that_is_a_link_blocks_preparation() {
    let project = TempProject::new("linked-outbox");
    std::fs::create_dir_all(project.path().join("real")).expect("fixture");
    std::fs::create_dir_all(project.path().join(".omnifrons")).expect("fixture");
    std::os::unix::fs::symlink(
        project.path().join("real"),
        project.path().join(".omnifrons/outbox"),
    )
    .expect("fixture symlink");
    let result = FsRunOutboxPreparer::new().prepare(
        &project.workspace(),
        &OutboxPath::default_path(),
        &run("run-1"),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::Declaration(OutboxDeclarationError::IsLink)
    );
    assert!(
        std::fs::read_dir(project.path().join("real"))
            .expect("list")
            .next()
            .is_none(),
        "nothing may be created through the link"
    );
}

#[test]
fn prepare_declares_the_path_the_handle_names_and_a_second_run_gets_its_own() {
    let project = TempProject::new("two-runs");
    let preparer = FsRunOutboxPreparer::new();
    let first = preparer
        .prepare(
            &project.workspace(),
            &OutboxPath::default_path(),
            &run("run-a"),
        )
        .expect("first");
    let second = preparer
        .prepare(
            &project.workspace(),
            &OutboxPath::default_path(),
            &run("run-b"),
        )
        .expect("second");
    assert_ne!(first.path, second.path);
    assert_eq!(first.path.file_name(), Some(OsStr::new("run-a")));
    assert_eq!(second.path.file_name(), Some(OsStr::new("run-b")));
    assert!(first.path.is_dir() && second.path.is_dir());
}

// -- the prober --

#[test]
fn a_regular_file_is_digested_from_its_handle_and_its_type_sniffed() {
    use std::io::Read as _;

    let project = TempProject::new("prober-regular");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    let content = {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        bytes.extend(std::iter::repeat_n(0xAB_u8, 200 * 1024));
        bytes
    };
    std::fs::write(outbox.path.join("report.pdf"), &content).expect("fixture");

    let probe =
        FsCandidateProber::new().probe(&outbox.handle, &outbox.path, OsStr::new("report.pdf"));
    let CandidateProbe::Regular(regular) = probe else {
        panic!("expected Regular, got {probe:?}");
    };
    assert_eq!(regular.size, content.len() as u64);
    assert_eq!(regular.digest.0[..], Sha256::digest(&content)[..]);
    assert_eq!(regular.detected_type, DetectedType::Pdf);
    // The held handle is positioned at the start again, ready for the
    // publication copy (slice 5b) to read the same bytes.
    let mut handle = regular.handle;
    let mut first = [0u8; 5];
    handle
        .read_exact(&mut first)
        .expect("read from the held handle");
    assert_eq!(&first, b"%PDF-");
}

/// Unix only: a FIFO is opened without blocking (`O_NONBLOCK`) and refused
/// as not a regular file (HAP-001-R15), never read.
#[cfg(unix)]
#[test]
fn a_fifo_is_outbox_escape_without_blocking() {
    let project = TempProject::new("prober-fifo");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    nix::unistd::mkfifo(&outbox.path.join("pipe"), nix::sys::stat::Mode::S_IRWXU)
        .expect("fixture fifo");
    let started = std::time::Instant::now();
    let probe = FsCandidateProber::new().probe(&outbox.handle, &outbox.path, OsStr::new("pipe"));
    assert!(
        matches!(probe, CandidateProbe::Escape(EscapeReason::NotRegular)),
        "got {probe:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "opening a FIFO with no writer must not block"
    );
}

// -- the inventory --

#[test]
fn inventory_lists_every_entry_with_its_probe_in_name_order() {
    let project = TempProject::new("inventory");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    std::fs::write(outbox.path.join("b.txt"), b"bbb").expect("fixture");
    std::fs::write(outbox.path.join("a.txt"), b"a").expect("fixture");
    std::fs::create_dir(outbox.path.join("sub")).expect("fixture");

    let entries = FsOutboxInventory::new()
        .inventory(&outbox.handle, &outbox.path)
        .expect("inventory");
    let summary: Vec<(String, &'static str)> = entries
        .iter()
        .map(|entry| {
            let kind = match &entry.probe {
                CandidateProbe::Regular(_) => "regular",
                CandidateProbe::Escape(EscapeReason::Link) => "link",
                CandidateProbe::Escape(EscapeReason::NotRegular) => "not-regular",
                CandidateProbe::Linked { .. } => "linked",
                CandidateProbe::Unreadable => "unreadable",
            };
            (entry.name.to_string_lossy().into_owned(), kind)
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("a.txt".to_string(), "regular"),
            ("b.txt".to_string(), "regular"),
            ("sub".to_string(), "not-regular"),
        ]
    );
}

/// Unix only (symlink and hard-link fixtures): the inventory never
/// dereferences a link it finds (HAP-001-R12), and a hard link is caught
/// by its link count.
#[cfg(unix)]
#[test]
fn inventory_never_dereferences_a_planted_link_and_catches_a_hard_link() {
    let project = TempProject::new("inventory-links");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    let outside = TempProject::new("inventory-links-outside");
    std::fs::write(outside.path().join("secret.txt"), b"outside").expect("fixture");
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), outbox.path.join("link"))
        .expect("fixture symlink");
    std::fs::hard_link(outside.path().join("secret.txt"), outbox.path.join("hard"))
        .expect("fixture hard link");

    let entries = FsOutboxInventory::new()
        .inventory(&outbox.handle, &outbox.path)
        .expect("inventory");
    assert_eq!(entries.len(), 2);
    assert!(matches!(
        entries[0].probe,
        CandidateProbe::Linked { link_count: 2 }
    ));
    assert_eq!(entries[0].name, "hard");
    assert!(matches!(
        entries[1].probe,
        CandidateProbe::Escape(EscapeReason::Link)
    ));
    assert_eq!(entries[1].name, "link");
}

#[test]
fn open_subdirectory_opens_a_real_directory_and_refuses_a_file() {
    let project = TempProject::new("subdirectory");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    std::fs::create_dir(outbox.path.join("run-1")).expect("fixture");
    std::fs::write(outbox.path.join("file"), b"x").expect("fixture");
    let inventory = FsOutboxInventory::new();

    let sub = inventory
        .open_subdirectory(&outbox.handle, &outbox.path, OsStr::new("run-1"))
        .expect("a real directory opens");
    assert!(sub.metadata().expect("fstat").is_dir());

    assert_eq!(
        inventory
            .open_subdirectory(&outbox.handle, &outbox.path, OsStr::new("file"))
            .unwrap_err(),
        InventoryError::NotADirectory
    );
    assert_eq!(
        inventory
            .open_subdirectory(&outbox.handle, &outbox.path, OsStr::new("absent"))
            .unwrap_err(),
        InventoryError::Unreadable
    );
}

/// Unix only (symlink fixture): a linked directory is never descended.
#[cfg(unix)]
#[test]
fn open_subdirectory_refuses_a_linked_directory() {
    let project = TempProject::new("subdirectory-link");
    let outbox = FsRunOutboxPreparer::new()
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("outbox");
    let elsewhere = TempProject::new("subdirectory-link-target");
    std::os::unix::fs::symlink(elsewhere.path(), outbox.path.join("linked-dir"))
        .expect("fixture symlink");
    assert_eq!(
        FsOutboxInventory::new()
            .open_subdirectory(&outbox.handle, &outbox.path, OsStr::new("linked-dir"))
            .unwrap_err(),
        InventoryError::NotADirectory
    );
}

/// Unix only (a Unix domain socket has no Windows equivalent here): a
/// socket bound inside the run subdirectory is refused as not a regular
/// file (HAP-001-R15) -- `open(2)` refuses it outright, `ENXIO` on Linux,
/// `EOPNOTSUPP` on macOS -- and is never read (R3-001/R1-009).
#[cfg(unix)]
#[test]
fn a_unix_socket_is_outbox_escape() {
    let project = TempProject::new("prober-socket");
    let subdirectory = FsRunOutboxPreparer::new()
        .prepare(
            &project.workspace(),
            &OutboxPath::default_path(),
            &run("run-sock"),
        )
        .expect("run subdirectory");
    let _listener = std::os::unix::net::UnixListener::bind(subdirectory.path.join("sock"))
        .expect("fixture socket");
    let probe = FsCandidateProber::new().probe(
        &subdirectory.handle,
        &subdirectory.path,
        OsStr::new("sock"),
    );
    assert!(
        matches!(probe, CandidateProbe::Escape(EscapeReason::NotRegular)),
        "a socket must be outbox-escape, got {probe:?}"
    );
    let entries = FsOutboxInventory::new()
        .inventory(&subdirectory.handle, &subdirectory.path)
        .expect("inventory");
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0].probe,
        CandidateProbe::Escape(EscapeReason::NotRegular)
    ));
}
