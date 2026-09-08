//! Reusable contracts for the outbox ports (spike slice 5): any
//! `RunOutboxPreparer` must create a run subdirectory exclusively and
//! refuse anything pre-planted at its path; any `CandidateProber` must
//! digest a regular file from its one handle and refuse a directory, a
//! link, and a hard link. Run by `omnifrons-adapters`' tests against the
//! real filesystem implementations.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_domain::outbox::{OutboxPath, RunId};

use crate::harness_adapter::WorkspaceRoot;
use crate::run_outbox::{
    CandidateProbe, CandidateProber, EscapeReason, PrepareError, RunOutboxPreparer,
};

/// A drop-guard temp directory standing in for a project root, removed on
/// every exit path.
pub struct ContractProject(PathBuf);

impl ContractProject {
    /// Create a fresh, empty project directory.
    ///
    /// # Panics
    ///
    /// Panics if the directory cannot be created.
    #[must_use]
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-outbox-contract-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the contract project directory");
        Self(dir)
    }

    /// The project directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The project as a workspace root.
    ///
    /// # Panics
    ///
    /// Panics if the directory is no longer a valid workspace.
    #[must_use]
    pub fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.0).expect("the contract project must be a valid workspace")
    }
}

impl Drop for ContractProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// SHA-256 of the three bytes `abc` (FIPS 180-4's first test vector), so
/// the contract can check a digest without a hashing dependency.
const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// Exercise the `RunOutboxPreparer` contract: a missing default outbox is
/// created; the run subdirectory is created under it at the canonical
/// path and is a directory by handle; the same run id cannot be prepared
/// twice; a file pre-planted at a run subdirectory's path, and on unix a
/// dangling symbolic link, block preparation with `AlreadyExists`; on unix
/// the created directory is owner-only.
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the preparer violates the contract.
pub fn run_outbox_preparer_contract<P: RunOutboxPreparer>(make: impl Fn() -> P) {
    let project = ContractProject::new("preparer");
    let preparer = make();
    let declared = OutboxPath::default_path();

    let run = RunId::new("run-contract-1").expect("valid");
    let created_run = preparer
        .prepare(&project.workspace(), &declared, &run)
        .expect("preparing a run subdirectory under a missing default outbox must succeed");
    let expected = std::fs::canonicalize(project.path().join(".omnifrons/outbox/run-contract-1"))
        .expect("the created run subdirectory must canonicalize");
    assert_eq!(
        created_run.path, expected,
        "the declared path is the canonical one"
    );
    assert_eq!(created_run.run_id, run);
    assert!(
        created_run
            .handle
            .metadata()
            .expect("fstat on the handle")
            .is_dir(),
        "the handle must be a directory"
    );

    let again = preparer.prepare(&project.workspace(), &declared, &run);
    assert!(
        matches!(again, Err(PrepareError::AlreadyExists)),
        "a second preparation of the same run id must be refused, got {again:?}"
    );

    let planted_file = RunId::new("run-contract-file").expect("valid");
    std::fs::write(
        project.path().join(".omnifrons/outbox/run-contract-file"),
        b"planted",
    )
    .expect("fixture");
    let blocked = preparer.prepare(&project.workspace(), &declared, &planted_file);
    assert!(
        matches!(blocked, Err(PrepareError::AlreadyExists)),
        "a pre-planted file at the run subdirectory's path must block, got {blocked:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = created_run
            .handle
            .metadata()
            .expect("fstat on the handle")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "the run subdirectory must be owner-only"
        );

        // A dangling symbolic link pre-planted at the path: `mkdir` fails
        // with EEXIST on any existing name, a link included, so nothing is
        // ever created through it.
        let planted_link = RunId::new("run-contract-link").expect("valid");
        std::os::unix::fs::symlink(
            "/nonexistent/target",
            project.path().join(".omnifrons/outbox/run-contract-link"),
        )
        .expect("fixture symlink");
        let blocked = preparer.prepare(&project.workspace(), &declared, &planted_link);
        assert!(
            matches!(blocked, Err(PrepareError::AlreadyExists)),
            "a pre-planted link at the run subdirectory's path must block, got {blocked:?}"
        );
        assert!(
            !project.path().join("nonexistent").exists(),
            "nothing may be created through the planted link"
        );
    }
}

/// Exercise the `CandidateProber` contract against a directory the
/// preparer opened: a regular file is digested from its handle (checked
/// against a known SHA-256 vector) with its size; a directory entry is
/// `outbox-escape`; on unix a symbolic link is `outbox-escape` without
/// being dereferenced, and a hard link is `outbox-linked` with the link
/// count read from the handle.
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the prober violates the contract.
pub fn candidate_prober_contract<P: RunOutboxPreparer, C: CandidateProber>(
    make_preparer: impl Fn() -> P,
    make_prober: impl Fn() -> C,
) {
    let project = ContractProject::new("prober");
    let preparer = make_preparer();
    let prober = make_prober();
    let outbox = preparer
        .open_outbox(&project.workspace(), &OutboxPath::default_path(), true)
        .expect("opening (and creating) the default outbox must succeed");

    std::fs::write(outbox.path.join("abc.txt"), b"abc").expect("fixture");
    match prober.probe(
        &outbox.handle,
        &outbox.path,
        std::ffi::OsStr::new("abc.txt"),
    ) {
        CandidateProbe::Regular(regular) => {
            assert_eq!(regular.size, 3);
            assert_eq!(regular.digest.to_hex(), SHA256_ABC);
            assert_eq!(
                regular.handle.metadata().expect("fstat").len(),
                3,
                "the handle must be the file that was digested"
            );
        }
        other => panic!("a regular file must probe Regular, got {other:?}"),
    }

    std::fs::create_dir(outbox.path.join("subdir")).expect("fixture");
    let directory = prober.probe(&outbox.handle, &outbox.path, std::ffi::OsStr::new("subdir"));
    assert!(
        matches!(directory, CandidateProbe::Escape(EscapeReason::NotRegular)),
        "a directory entry must be outbox-escape, got {directory:?}"
    );

    let missing = prober.probe(
        &outbox.handle,
        &outbox.path,
        std::ffi::OsStr::new("missing"),
    );
    assert!(
        matches!(missing, CandidateProbe::Unreadable),
        "a name that vanished must be unreadable, got {missing:?}"
    );

    #[cfg(unix)]
    {
        let outside = ContractProject::new("prober-outside");
        std::fs::write(outside.path().join("target.bin"), b"outside").expect("fixture");
        std::os::unix::fs::symlink(outside.path().join("target.bin"), outbox.path.join("link"))
            .expect("fixture symlink");
        let link = prober.probe(&outbox.handle, &outbox.path, std::ffi::OsStr::new("link"));
        assert!(
            matches!(link, CandidateProbe::Escape(EscapeReason::Link)),
            "a symbolic link must be outbox-escape, got {link:?}"
        );

        std::fs::hard_link(
            outside.path().join("target.bin"),
            outbox.path.join("linked.bin"),
        )
        .expect("fixture hard link");
        let linked = prober.probe(
            &outbox.handle,
            &outbox.path,
            std::ffi::OsStr::new("linked.bin"),
        );
        assert!(
            matches!(linked, CandidateProbe::Linked { link_count: 2 }),
            "a hard link must be outbox-linked with its link count, got {linked:?}"
        );
    }
}
