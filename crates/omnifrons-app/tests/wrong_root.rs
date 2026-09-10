//! The wrong-root scan's pure rules (spike slice 5d, HAP-001-R32, R43,
//! R44): what the walk excludes by the product's own rule, which class
//! makes a file misplaced, and how the ignore ledger filters a report.

use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_app::wrong_root::{
    ScannedFile, Verdict, finding_for, is_excluded, is_git_metadata_dir,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ArtifactClass, DetectedType, OutboxPath};
use omnifrons_domain::wrong_root::WrongRootReason;
use std::path::Path;

fn scanned(name: &str, detected_type: DetectedType, size: u64) -> ScannedFile {
    ScannedFile {
        name: name.to_string(),
        size,
        digest: Sha256Digest([3; 32]),
        detected_type,
    }
}

/// HAP-001-R43 and R44: the outbox is read as an ingestion source, never
/// as a wrong root, and it is kept out by the product's own rule -- never
/// by an ignore rule.
#[test]
fn the_outbox_subtree_is_excluded_by_the_products_own_rule() {
    let outbox = OutboxPath::default_path();
    let excluded = [
        ".omnifrons/outbox",
        ".omnifrons/outbox/run-1",
        ".omnifrons/outbox/run-1/report.pdf",
    ];
    for relative in excluded {
        assert!(
            is_excluded(Path::new(relative), &outbox),
            "{relative} must be excluded from the wrong-root verdict"
        );
    }
    let scanned = [
        ".omnifrons",
        ".omnifrons/asset-policy.json",
        ".omnifrons/outbox-notes/report.pdf",
        "docs/report.pdf",
        ".gitignore",
        "gitignore/report.pdf",
        // R1-006: a *name* alone never excludes anything any more. A
        // directory called `.git` is excluded by what it holds, which is a
        // fact this pure rule cannot see and does not claim to.
        ".git",
        ".git/objects/ab/cdef",
        "vendor/sub/.git",
        "vendor/sub/.git/config",
    ];
    for relative in scanned {
        assert!(
            !is_excluded(Path::new(relative), &outbox),
            "{relative} must stay inside the scan"
        );
    }
}

/// R1-006: Git's metadata is excluded because it *is* a repository's
/// metadata, not because it is spelled `.git`. A worktree is written into
/// by the very producers this scan polices, and a name-only rule would let
/// one of them carve its own blind spot out of the verdict with a single
/// `mkdir`.
#[test]
fn only_a_real_repositorys_metadata_directory_is_excluded() {
    assert!(
        is_git_metadata_dir(true, true),
        "a directory holding HEAD and objects/ is a repository's metadata"
    );
    for (holds_head, holds_objects) in [(false, false), (true, false), (false, true)] {
        assert!(
            !is_git_metadata_dir(holds_head, holds_objects),
            "an empty `mkdir .git` must not switch the verdict off \
             (HEAD: {holds_head}, objects: {holds_objects})"
        );
    }
}

/// A declared outbox other than the default is excluded by the same rule:
/// the exclusion follows the declaration, never a fixed path.
#[test]
fn the_exclusion_follows_the_declared_outbox_path() {
    let declared = OutboxPath::new("build/out").expect("a project-relative path");
    assert!(is_excluded(Path::new("build/out/report.pdf"), &declared));
    assert!(!is_excluded(Path::new("build/report.pdf"), &declared));
    assert!(!is_excluded(
        Path::new(".omnifrons/outbox/report.pdf"),
        &declared
    ));
}

/// HAP-001-R1 and R3: a file the project's policy classifies `git-tracked`
/// is never `misplaced` in its tracked location, and HAP-001-R2 keeps a
/// Markdown note out of this contract entirely. Only `generated-heavy`
/// produces a finding in this slice.
#[test]
fn only_a_generated_heavy_file_becomes_a_finding() {
    let policy = OutboxPolicy::default_policy();

    let heavy = scanned("docs/report.pdf", DetectedType::Pdf, 4096);
    let Verdict::Misplaced(finding) = finding_for(&heavy, &policy) else {
        panic!("a heavy file outside the outbox is misplaced");
    };
    assert_eq!(finding.name(), "docs/report.pdf");
    assert_eq!(finding.size, 4096);
    assert_eq!(finding.digest, Sha256Digest([3; 32]));
    assert_eq!(finding.detected_type, DetectedType::Pdf);
    assert_eq!(finding.class, ArtifactClass::GeneratedHeavy);
    assert_eq!(finding.reason, WrongRootReason::InProjectOutsideOutbox);

    let kept_where_it_is = [
        scanned("src/main.rs", DetectedType::PlainText, 200),
        scanned("README.md", DetectedType::Markdown, 200),
        scanned("target/debug/tool", DetectedType::ElfExecutable, 200),
        scanned("assets/blob.dat", DetectedType::Unknown, 200),
    ];
    for file in kept_where_it_is {
        assert_eq!(
            finding_for(&file, &policy),
            Verdict::KeptWhereItIs,
            "{} must not be reported as misplaced",
            file.name
        );
    }
}

/// R3-003 and RCS-001-R14: a finding can never carry a device path or a
/// control character, so a heavy file found at such a name yields
/// [`Verdict::UnreportableName`] -- a fact the caller must count and
/// surface. It is **not** folded into "kept where it is": that would drop
/// the file out of the report while the report still called itself clean,
/// and a name is exactly what a hostile producer chooses.
#[test]
fn a_heavy_file_at_a_name_no_finding_may_carry_is_unreportable_not_clean() {
    let policy = OutboxPolicy::default_policy();
    let unreportable = [
        "/tmp/report.pdf",
        "../report.pdf",
        "C:\\out\\report.pdf",
        // Legal on unix, and legal in nothing this product writes it into.
        "docs/re\nport.pdf",
        "docs/re\u{202e}port.pdf",
    ];
    for name in unreportable {
        assert_eq!(
            finding_for(&scanned(name, DetectedType::Pdf, 10), &policy),
            Verdict::UnreportableName,
            "{name:?} must be surfaced as unreportable, never dropped"
        );
    }
}
