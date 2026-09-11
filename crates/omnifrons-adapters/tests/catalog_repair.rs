//! The catalog repair (spike slice 5e, HAP-001-R23, R39, R40) over the
//! real `.omnifrons/catalog.jsonl`: the two non-deleting rules previewed
//! and applied under a digest binding, the deleting one refused, and the
//! original copied into the product work area before a byte is rewritten.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::JsonlCatalogStore;
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::catalog_repair::{
    CatalogRepair as _, CatalogRepairError, RepairRefusal, RepairRule,
};
use omnifrons_app::catalog_store::{CatalogStore as _, CatalogStoreError};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::outbox::ContentDigest;

/// A unique temporary directory for one test, removed when that test
/// ends. Every fixture here builds a real filesystem tree, and without
/// the guard each run leaves one behind in the system temporary
/// directory for good.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-catalog-repair-{label}-{}-{n}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A unique temporary directory for one test.
fn temp_dir(label: &str) -> TempDir {
    TempDir::new(label)
}

/// The instant a repair is told it happened. `CatalogRepair::repair` takes
/// the clock as an argument precisely so a test need not read one: the
/// pre-repair copy's name is derived from it, so a fixture that passed
/// `SystemTime::now()` would be naming a file from the wall clock inside
/// a test that asserts on that name.
fn at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)
}

/// One `record` line for `publication_id`, registered under
/// `asset_root_id`, with `catalog_id` written verbatim so a test can make
/// the ids disagree.
fn record_line(publication_id: &str, asset_root_id: &str, catalog_id: &str, name: &str) -> String {
    format!(
        r#"{{"event":"record","schema":1,"record":{{"publicationId":"{publication_id}","catalogId":"{catalog_id}","assetRootId":"{asset_root_id}","class":"generated-heavy","type":"pdf","size":3,"digest":{{"algorithm":"sha256","hex":"{publication_id}"}},"names":["{name}"],"relationships":[],"provenance":{{"scopeId":"{publication_id}","producer":{{"kind":"unattributed","foundUnderRunId":null}},"transitions":[]}},"provider":{{"adapterId":"local-dir","locator":null,"confirmationKind":null,"state":"pending","reason":null}},"state":"registered","recordVersion":1}}}}"#
    )
}

/// One `alias` line naming `publication_id`.
fn alias_line(publication_id: &str, name: &str) -> String {
    format!(
        r#"{{"event":"alias","schema":1,"publicationId":"{publication_id}","name":"{name}","at":{{"secs":1,"nanos":0}}}}"#
    )
}

/// Write `lines` as the project's catalog and return the workspace root.
fn project_with_catalog(dir: &Path, lines: &[String]) -> WorkspaceRoot {
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(".omnifrons")).expect("project");
    std::fs::write(
        root.join(JsonlCatalogStore::FILE_PATH),
        lines.join("\n") + "\n",
    )
    .expect("catalog");
    WorkspaceRoot::new(&root).expect("workspace")
}

fn work_area_in(dir: &Path, workspace: &WorkspaceRoot) -> WorkAreaRoot {
    WorkAreaRoot::open(&dir.join("work-area"), &[workspace]).expect("work area")
}

const ID_A: &str = "aa";
const ID_B: &str = "bb";

fn id(prefix: &str) -> String {
    prefix.repeat(32)
}

/// HAP-001-R23: two `record` lines carrying one publication identity are
/// one registration written twice. The repair drops **the later** line, so
/// the surviving line still registers that identity and no registered
/// artifact is removed (HAP-001-R39).
#[test]
fn the_later_of_two_records_with_one_publication_identity_is_dropped() {
    let dir = temp_dir("duplicate-record");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&a, "root", &format!("root/{a}"), "second.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    assert_eq!(
        store.list().expect_err("a duplicate identity is corrupt"),
        CatalogStoreError::Corrupt,
    );

    let plan = store.preview().expect("preview");
    assert_eq!(plan.drops.len(), 1, "exactly the later line: {plan:?}");
    assert_eq!(plan.drops[0].line, 2, "the later line, never the first");
    assert_eq!(plan.drops[0].rule, RepairRule::DuplicateRecord);
    assert!(plan.refusals.is_empty(), "nothing refuses this: {plan:?}");

    let outcome = store
        .repair(&work_area, &plan.sha256, at())
        .expect("the repair applies");
    assert_eq!(outcome.dropped_lines, 1);
    assert_eq!(outcome.kept_records, 1);

    let records = store.list().expect("the repaired catalog reads");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].publication_id.to_hex(), a);
    assert_eq!(
        records[0]
            .names
            .iter()
            .map(|name| name.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["first.pdf".to_string()],
        "the surviving registration is the first one, not the later",
    );
}

/// HAP-001-R23 derives the publication identity from the project and the
/// content digest, and the **asset root is not in that preimage**. So two
/// `record` lines carrying one `publicationId` under *different*
/// `assetRootId`s are two `catalogId`s -- two distinct registered
/// artifacts -- and not one registration written twice. Dropping the
/// later one would delete a registered artifact, which HAP-001-R39
/// reserves to MRP-001's tombstone kind `artifact`.
#[test]
fn two_records_with_one_identity_under_different_asset_roots_are_refused() {
    let dir = temp_dir("duplicate-across-asset-roots");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&a, "other", &format!("other/{a}"), "second.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let before = std::fs::read(&path).expect("before");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    let plan = store.preview().expect("preview");
    assert!(
        plan.drops.is_empty(),
        "the second asset root's registration may not be dropped: {plan:?}",
    );
    assert_eq!(plan.refusals.len(), 1, "{plan:?}");
    assert_eq!(plan.refusals[0].line, 2);
    assert_eq!(
        plan.refusals[0].refusal,
        RepairRefusal::DuplicateAcrossAssetRoots,
    );
    assert!(!plan.is_repairable(), "{plan:?}");

    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("the repair is refused"),
        CatalogRepairError::Refused(RepairRefusal::DuplicateAcrossAssetRoots),
    );
    assert_eq!(
        std::fs::read(&path).expect("after"),
        before,
        "and both registrations are still there",
    );
}

/// HAP-001-R39: a `record` whose `catalogId` is not
/// `<assetRootId>/<publicationId>` is the **only** line for that identity,
/// so dropping it would delete a registered artifact. The repair refuses
/// and the catalog stays `Corrupt` -- deletion is MRP-001's tombstone
/// kind, not this contract's.
#[test]
fn a_record_whose_catalog_id_disagrees_is_refused_and_never_dropped() {
    let dir = temp_dir("catalog-id-mismatch");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[record_line(&a, "root", &format!("other/{a}"), "report.pdf")],
    );
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    let plan = store.preview().expect("preview");
    assert!(plan.drops.is_empty(), "nothing may be dropped: {plan:?}");
    assert_eq!(plan.refusals.len(), 1);
    assert_eq!(plan.refusals[0].line, 1);
    assert_eq!(plan.refusals[0].refusal, RepairRefusal::CatalogIdMismatch);

    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("the repair is refused"),
        CatalogRepairError::Refused(RepairRefusal::CatalogIdMismatch),
    );
    assert_eq!(
        store.list().expect_err("still corrupt"),
        CatalogStoreError::Corrupt,
        "the line is left exactly where it was",
    );
}

/// `keptRecords` counts the registrations a **repaired** catalog would
/// carry, so on a plan that carries a refusal it is an undercount of the
/// registrations the file holds: a refused `record` is not counted,
/// although the refusal is precisely what leaves it in place. Pinned
/// rather than corrected, because the number describes the catalog a
/// repair would leave and a refused plan leaves the file exactly as it
/// is -- a surface reading `keptRecords` on a `repairable: false` plan is
/// reading an answer to a question that was not asked.
#[test]
fn a_refused_records_identity_is_not_counted_among_the_kept_records() {
    let dir = temp_dir("kept-records-refused");
    let a = id(ID_A);
    let b = id(ID_B);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&b, "root", &format!("other/{b}"), "second.pdf"),
        ],
    );
    let store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");

    assert_eq!(plan.refusals.len(), 1, "{plan:?}");
    assert_eq!(plan.refusals[0].refusal, RepairRefusal::CatalogIdMismatch);
    assert_eq!(
        plan.kept_records, 1,
        "the file holds two registrations and the refused one is not counted: {plan:?}",
    );
    assert!(!plan.is_repairable(), "which is why the count is moot here");
}

/// HAP-001-R23: an alias is a display name ("MAY add a display-name
/// alias"), so an alias no earlier `record` carries registers nothing and
/// dropping it removes no registration.
#[test]
fn an_alias_no_earlier_record_carries_is_dropped() {
    let dir = temp_dir("dangling-alias");
    let a = id(ID_A);
    let b = id(ID_B);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "report.pdf"),
            alias_line(&b, "ghost.pdf"),
            alias_line(&a, "also.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    let plan = store.preview().expect("preview");
    assert_eq!(plan.drops.len(), 1, "only the dangling one: {plan:?}");
    assert_eq!(plan.drops[0].line, 2);
    assert_eq!(plan.drops[0].rule, RepairRule::DanglingAlias);

    store
        .repair(&work_area, &plan.sha256, at())
        .expect("the repair applies");
    let records = store.list().expect("the repaired catalog reads");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]
            .names
            .iter()
            .map(|name| name.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["report.pdf".to_string(), "also.pdf".to_string()],
        "the surviving alias is still applied, in the order it was written",
    );
}

/// The preview's digest is the binding an apply has to send back: a
/// catalog rewritten between the two is refused and nothing is touched,
/// exactly as `ProjectTextFile`'s `expected` refuses (spike slice 5c
/// R1-002, the window that lost an edit).
#[test]
fn an_apply_whose_digest_is_not_the_catalogs_own_is_refused() {
    let dir = temp_dir("changed");
    let a = id(ID_A);
    let b = id(ID_B);
    let duplicate = record_line(&a, "root", &format!("root/{a}"), "second.pdf");
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            duplicate.clone(),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");

    // Another device's sync lands between the preview and the apply.
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let arrived = [
        record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
        duplicate,
        record_line(&b, "root", &format!("root/{b}"), "third.pdf"),
    ]
    .join("\n")
        + "\n";
    std::fs::write(&path, &arrived).expect("rewrite");

    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("the binding is stale"),
        CatalogRepairError::Changed,
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        arrived,
        "nothing of the arriving bytes was rewritten",
    );
    assert!(
        std::fs::read_dir(work_area.recovery_dir())
            .expect("recovery")
            .next()
            .is_none(),
        "and no pre-repair copy was written for a repair that did not run",
    );
}

/// A healthy catalog is not rewritten at all: a repair that would drop
/// nothing writes no copy and leaves the file alone.
#[test]
fn a_catalog_with_nothing_to_drop_is_left_alone() {
    let dir = temp_dir("nothing");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[record_line(&a, "root", &format!("root/{a}"), "report.pdf")],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let before = std::fs::read(&path).expect("before");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");
    assert!(!plan.is_repairable(), "nothing to do: {plan:?}");
    assert!(plan.drops.is_empty(), "{plan:?}");
    assert!(plan.refusals.is_empty(), "{plan:?}");
    assert_eq!(plan.kept_records, 1);
    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("nothing to repair"),
        CatalogRepairError::NothingToRepair,
    );
    // "Left alone" is the claim in the name, so it is asserted: not one
    // byte rewritten, and no pre-repair copy for a repair that did not
    // run.
    assert_eq!(std::fs::read(&path).expect("after"), before);
    assert!(
        std::fs::read_dir(work_area.recovery_dir())
            .expect("recovery")
            .next()
            .is_none(),
    );
}

/// A line can fail both ways at once: its `catalogId` disagrees with its
/// own ids **and** it carries a field this version cannot decode. It is
/// reported `CatalogIdMismatch`, and that precedence is deliberate, not
/// an accident of arm order: `ids_disagree` compares the three fields as
/// **text**, precisely so HAP-001-R39's rule -- the one line that may
/// never be dropped -- is recognized even on a line nothing else about
/// which can be decoded. Both refusals refuse the whole repair and leave
/// the line where it is, so the outcome is the same either way; only the
/// reason the preview shows differs, and this is the reason it shows.
#[test]
fn a_line_that_is_both_undecodable_and_id_disagreeing_reports_the_mismatch() {
    let dir = temp_dir("both-refusals");
    let a = id(ID_A);
    // `catalogId` disagrees with `assetRootId`/`publicationId`, and the
    // `class` token is one no version of this product writes.
    let line = record_line(&a, "root", &format!("other/{a}"), "report.pdf")
        .replace(r#""class":"generated-heavy""#, r#""class":"not-a-class""#);
    assert!(line.contains("not-a-class"), "the fixture edit landed");
    let workspace = project_with_catalog(&dir, &[line]);
    let store = JsonlCatalogStore::open(&workspace).expect("store");

    let plan = store.preview().expect("preview");
    assert_eq!(plan.refusals.len(), 1, "{plan:?}");
    assert_eq!(
        plan.refusals[0].refusal,
        RepairRefusal::CatalogIdMismatch,
        "the R39 rule is recognized from the text, whatever else the line carries",
    );
    assert!(plan.drops.is_empty(), "{plan:?}");
}

/// A project that has never registered anything has no catalog file, and
/// there is no repair to offer for one: `Absent`, which the shell renders
/// `catalog-unavailable` rather than `catalog-corrupt`, because there are
/// no lines to show.
#[test]
fn a_project_with_no_catalog_has_no_repair_to_preview() {
    let dir = temp_dir("absent");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(".omnifrons")).expect("project");
    let workspace = WorkspaceRoot::new(&root).expect("workspace");
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    assert_eq!(
        store.preview().expect_err("nothing to preview"),
        CatalogRepairError::Absent,
    );
    assert_eq!(
        store
            .repair(
                &work_area,
                &ContentDigest::from_hex(&"ab".repeat(32)).expect("hex"),
                at(),
            )
            .expect_err("nothing to repair"),
        CatalogRepairError::Absent,
    );
}

/// A zero-byte catalog is a catalog that registers nothing, not a corrupt
/// one: it reads as an empty list, so there is nothing to drop and
/// nothing refuses.
#[test]
fn an_empty_catalog_file_reads_and_has_nothing_to_repair() {
    let dir = temp_dir("empty-file");
    let workspace = project_with_catalog(&dir, &[]);
    // `project_with_catalog` joins and appends a newline; make it truly
    // zero bytes.
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    std::fs::write(&path, b"").expect("empty");
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    assert!(store.list().expect("an empty catalog reads").is_empty());
    let plan = store.preview().expect("preview");
    assert!(plan.drops.is_empty(), "{plan:?}");
    assert!(plan.refusals.is_empty(), "{plan:?}");
    assert_eq!(plan.kept_records, 0);
    assert!(!plan.is_repairable());
    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("nothing to repair"),
        CatalogRepairError::NothingToRepair,
    );
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"",
        "and it is untouched"
    );
}

/// A directory at the catalog's own name is not a catalog. The
/// regular-file fact comes from the handle, so this is `Unreadable` on
/// every platform -- unix opens a directory read-only and then refuses it
/// from `fstat`, Windows refuses the open and classifies from the path.
#[test]
fn a_directory_at_the_catalogs_name_is_unreadable() {
    let dir = temp_dir("directory");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(JsonlCatalogStore::FILE_PATH)).expect("a directory there");
    let workspace = WorkspaceRoot::new(&root).expect("workspace");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    assert_eq!(
        store.preview().expect_err("not a file"),
        CatalogRepairError::Unreadable,
    );
    let work_area = work_area_in(&dir, &workspace);
    assert_eq!(
        store
            .repair(
                &work_area,
                &ContentDigest::from_hex(&"ab".repeat(32)).expect("hex"),
                at(),
            )
            .expect_err("not a file"),
        CatalogRepairError::Unreadable,
    );
}

/// The boundary the `DanglingAlias` rule reaches at its end, decided
/// rather than left to fall out: a catalog of `alias` lines and no
/// `record` at all is repaired to **zero bytes**, and that is correct.
/// Every one of those lines is a display name attached to nothing
/// (HAP-001-R23), so dropping all of them removes no registration, and a
/// catalog that registered nothing before registers nothing after -- the
/// same state a project that has never published is in. The lines
/// themselves are not lost: the preview names every one of them, and the
/// pre-repair copy in the work area is the file exactly as it was.
#[test]
fn a_catalog_of_aliases_alone_is_repaired_to_an_empty_catalog() {
    let dir = temp_dir("aliases-only");
    let a = id(ID_A);
    let b = id(ID_B);
    let workspace = project_with_catalog(
        &dir,
        &[alias_line(&a, "one.pdf"), alias_line(&b, "two.pdf")],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let original = std::fs::read(&path).expect("original");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    let plan = store.preview().expect("preview");
    assert!(plan.is_repairable(), "{plan:?}");
    assert_eq!(plan.drops.len(), 2, "every line is named: {plan:?}");
    assert_eq!(plan.kept_records, 0);

    let outcome = store
        .repair(&work_area, &plan.sha256, at())
        .expect("the repair applies");
    assert_eq!(outcome.dropped_lines, 2);
    assert_eq!(outcome.kept_records, 0);
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"",
        "nothing was registered, so nothing is left",
    );
    assert!(store.list().expect("the repaired catalog reads").is_empty());

    let copies: Vec<PathBuf> = std::fs::read_dir(work_area.recovery_dir())
        .expect("recovery")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(
        std::fs::read(&copies[0]).expect("copy"),
        original,
        "and every dropped line is still in the pre-repair copy",
    );
}

/// HAP-001-R40: the Catalog is synchronized, portable, untrusted content,
/// so a line this version cannot decode is never dropped -- that would be
/// a record-deletion path driven by content.
#[test]
fn a_line_that_does_not_parse_refuses_the_repair_rather_than_being_dropped() {
    let dir = temp_dir("unparsable");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "report.pdf"),
            "{\"event\":\"record\",\"schema\":2,\"record\":{}}".to_string(),
            record_line(&a, "root", &format!("root/{a}"), "again.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");
    assert_eq!(plan.refusals.len(), 1);
    assert_eq!(plan.refusals[0].line, 2);
    assert_eq!(plan.refusals[0].refusal, RepairRefusal::Unparsable);
    assert_eq!(
        plan.drops.len(),
        1,
        "the later duplicate is still reported, but the refusal wins: {plan:?}",
    );
    assert_eq!(
        store
            .repair(&work_area, &plan.sha256, at())
            .expect_err("refused"),
        CatalogRepairError::Refused(RepairRefusal::Unparsable),
    );
}

/// The catalog is reached by joining `.omnifrons/catalog.jsonl` onto the
/// canonical workspace root, and the no-follow open guards only the leaf.
/// A `.omnifrons` that is itself a symbolic link therefore reaches a
/// directory outside the project, and both halves of the repair would
/// otherwise read and rewrite a file there. Both refuse instead.
///
/// Unix only: it needs a symbolic-link fixture, and Windows needs a
/// privilege to create one (the gate every symbolic-link fixture in this
/// repository carries).
#[cfg(unix)]
#[test]
fn a_catalog_reached_through_a_linked_omnifrons_directory_is_refused() {
    let dir = temp_dir("linked-parent");
    let a = id(ID_A);
    let outside = dir.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("the directory the link points at");
    let lines = [
        record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
        record_line(&a, "root", &format!("root/{a}"), "second.pdf"),
    ]
    .join("\n")
        + "\n";
    let outside_catalog = outside.join("catalog.jsonl");
    std::fs::write(&outside_catalog, &lines).expect("catalog");

    let root = dir.join("project");
    std::fs::create_dir_all(&root).expect("project");
    std::os::unix::fs::symlink(&outside, root.join(".omnifrons")).expect("link");
    let workspace = WorkspaceRoot::new(&root).expect("workspace");
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");

    assert_eq!(
        store.preview().expect_err("the preview refuses"),
        CatalogRepairError::Unreadable,
        "a catalog outside the project is not this project's catalog",
    );
    assert_eq!(
        store
            .repair(
                &work_area,
                &ContentDigest::from_hex(&"ab".repeat(32)).expect("hex"),
                at(),
            )
            .expect_err("the apply refuses"),
        CatalogRepairError::Unreadable,
    );
    assert_eq!(
        std::fs::read_to_string(&outside_catalog).expect("read"),
        lines,
        "and the file outside the project was not rewritten",
    );
    let siblings: Vec<String> = std::fs::read_dir(&outside)
        .expect("dir")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .collect();
    assert_eq!(
        siblings,
        vec!["catalog.jsonl".to_string()],
        "and no `.part` landed in the link target either",
    );
}

/// The copy comes first, and it is the bytes that were read -- so the
/// repair is reversible, and the untouched original sits outside the
/// synchronized `.omnifrons/` namespace (HAP-001-R7, R40).
#[test]
fn the_original_is_copied_into_the_work_area_before_the_catalog_is_rewritten() {
    let dir = temp_dir("copy");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&a, "root", &format!("root/{a}"), "second.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let original = std::fs::read(&path).expect("original");
    // A mode that is neither the default nor the copy's, so the assertion
    // below cannot pass by coincidence.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("mode");
    }
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");
    store
        .repair(&work_area, &plan.sha256, at())
        .expect("repair");

    let copies: Vec<PathBuf> = std::fs::read_dir(work_area.recovery_dir())
        .expect("recovery")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(copies.len(), 1, "exactly one copy: {copies:?}");
    let name = copies[0]
        .file_name()
        .expect("name")
        .to_string_lossy()
        .to_string();
    assert!(
        name.starts_with("catalog-")
            && std::path::Path::new(&name)
                .extension()
                .is_some_and(|extension| extension == "jsonl"),
        "named catalog-<timestamp>.jsonl, got {name}",
    );
    assert_eq!(
        std::fs::read(&copies[0]).expect("copy"),
        original,
        "the copy is the bytes the repair read, not a re-read of the file",
    );
    // "Before the catalog is rewritten" is the claim in the name. What is
    // observable at the end is that the copy holds the *pre*-repair bytes
    // while the file holds the repaired ones -- the two differ, so the
    // copy cannot have been taken from the file afterwards. The ordering
    // itself is what `a_copy_that_cannot_be_written_leaves_the_catalog_untouched`
    // proves, by failing the copy and finding the catalog intact.
    assert_ne!(
        std::fs::read(&path).expect("the repaired catalog"),
        std::fs::read(&copies[0]).expect("copy"),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&copies[0])
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only");
        // And the rewritten catalog carries the original's own mode
        // across the rename, rather than the `.part`'s default: the file
        // belongs to the user, and a repair is not the place to change
        // who can read it.
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o640,
        );
    }
}

/// Every kept line is rewritten byte for byte, and the repaired file ends
/// in exactly one newline -- the store appends with `writeln!` in append
/// mode, so a file that did not would have the next registration glued
/// onto its last line.
#[test]
fn kept_lines_are_rewritten_verbatim_and_the_file_ends_in_a_newline() {
    let dir = temp_dir("verbatim");
    let a = id(ID_A);
    let b = id(ID_B);
    let keep_first = record_line(&a, "root", &format!("root/{a}"), "first.pdf");
    let keep_last = record_line(&b, "root", &format!("root/{b}"), "other.pdf");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(".omnifrons")).expect("project");
    // No trailing newline on the last line, and a blank line in the middle.
    std::fs::write(
        root.join(JsonlCatalogStore::FILE_PATH),
        format!(
            "{keep_first}\n\n{}\n{keep_last}",
            record_line(&a, "root", &format!("root/{a}"), "duplicate.pdf")
        ),
    )
    .expect("catalog");
    let workspace = WorkspaceRoot::new(&root).expect("workspace");
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");
    assert_eq!(plan.drops.len(), 1);
    assert_eq!(plan.drops[0].line, 3);
    store
        .repair(&work_area, &plan.sha256, at())
        .expect("repair");

    assert_eq!(
        std::fs::read_to_string(root.join(JsonlCatalogStore::FILE_PATH)).expect("read"),
        format!("{keep_first}\n\n{keep_last}\n"),
        "the kept lines verbatim, the blank line kept, one trailing newline",
    );
    // And the store can still append to it without gluing.
    let records = store.list().expect("reads");
    assert_eq!(records.len(), 2);
}

/// The other direction of the rule above, and the one that says the two
/// halves of it are not the same claim: a kept blank *last* line is kept
/// like any other, so the repaired file ends in two newlines. "Every kept
/// line byte for byte, blank lines included" and "exactly one trailing
/// newline" cannot both hold, and this repository keeps the first: the
/// appended newline exists so a `writeln!` in append mode does not glue
/// the next registration onto the last line, and a file ending in a blank
/// line already satisfies that. Nothing normalises.
#[test]
fn a_kept_blank_last_line_is_kept_and_the_file_is_not_normalised() {
    let dir = temp_dir("blank-last-line");
    let a = id(ID_A);
    let keep = record_line(&a, "root", &format!("root/{a}"), "first.pdf");
    let drop = record_line(&a, "root", &format!("root/{a}"), "second.pdf");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(".omnifrons")).expect("project");
    // The last line of the file is blank, which a `record` line's own
    // terminator plus one more newline is.
    std::fs::write(
        root.join(JsonlCatalogStore::FILE_PATH),
        format!("{keep}\n{drop}\n\n"),
    )
    .expect("catalog");
    let workspace = WorkspaceRoot::new(&root).expect("workspace");
    let work_area = work_area_in(&dir, &workspace);
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");
    assert_eq!(plan.drops.len(), 1);
    assert_eq!(plan.drops[0].line, 2);
    store
        .repair(&work_area, &plan.sha256, at())
        .expect("repair");

    assert_eq!(
        std::fs::read_to_string(root.join(JsonlCatalogStore::FILE_PATH)).expect("read"),
        format!("{keep}\n\n"),
        "the trailing blank line is kept, so the file ends in two newlines",
    );
    assert_eq!(
        store.list().expect("reads").len(),
        1,
        "and a blank line is still nothing to the reader",
    );
}

/// The copy is written **before** the rewrite, so a rewrite that then
/// fails has to take it back: a repair that never happened must leave no
/// pre-repair copy behind, or `recovery/` accumulates originals for
/// repairs that were refused and a reader cannot tell those from the
/// originals of repairs that ran.
///
/// Unix only: it needs a directory permission bit Windows ACLs do not
/// express through `std` -- the same residual the copy-failure test
/// below carries.
#[cfg(unix)]
#[test]
fn a_rewrite_that_fails_after_the_copy_takes_the_copy_back() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = temp_dir("rewrite-fails");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&a, "root", &format!("root/{a}"), "second.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let original = std::fs::read(&path).expect("original");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");

    // The copy still lands -- it goes to the work area -- and only the
    // sibling `.part` inside `.omnifrons` cannot be created.
    let omnifrons = path.parent().expect("parent").to_path_buf();
    std::fs::set_permissions(&omnifrons, std::fs::Permissions::from_mode(0o500))
        .expect("read-only .omnifrons");
    let outcome = store.repair(&work_area, &plan.sha256, at());
    std::fs::set_permissions(&omnifrons, std::fs::Permissions::from_mode(0o700)).expect("restore");

    assert_eq!(
        outcome.expect_err("the rewrite could not be staged"),
        CatalogRepairError::WriteFailed,
    );
    assert_eq!(
        std::fs::read(&path).expect("read"),
        original,
        "and the catalog was not rewritten",
    );
    assert!(
        std::fs::read_dir(work_area.recovery_dir())
            .expect("recovery")
            .next()
            .is_none(),
        "and no pre-repair copy was left for a repair that did not happen",
    );
}

/// The copy is written before anything is rewritten, so a copy that
/// cannot be written leaves the catalog exactly as it was.
///
/// The failure is arranged by **removing the work area's `recovery/`
/// directory**, which is the one arrangement that reaches this path on
/// every platform: the copy is created exclusively, so a missing parent
/// fails the create with no permission bit involved. An earlier version
/// of this test made the directory read-only instead, which needs a mode
/// Windows ACLs do not express through `std` and gated the whole test to
/// unix for no behaviour that differs there.
#[test]
fn a_copy_that_cannot_be_written_leaves_the_catalog_untouched() {
    let dir = temp_dir("copy-fails");
    let a = id(ID_A);
    let workspace = project_with_catalog(
        &dir,
        &[
            record_line(&a, "root", &format!("root/{a}"), "first.pdf"),
            record_line(&a, "root", &format!("root/{a}"), "second.pdf"),
        ],
    );
    let work_area = work_area_in(&dir, &workspace);
    let path = workspace.path().join(JsonlCatalogStore::FILE_PATH);
    let original = std::fs::read(&path).expect("original");
    let mut store = JsonlCatalogStore::open(&workspace).expect("store");
    let plan = store.preview().expect("preview");

    std::fs::remove_dir_all(work_area.recovery_dir()).expect("no recovery directory");
    let outcome = store.repair(&work_area, &plan.sha256, at());

    assert_eq!(
        outcome.expect_err("the copy could not be written"),
        CatalogRepairError::CopyFailed,
    );
    assert_eq!(
        std::fs::read(&path).expect("read"),
        original,
        "and the catalog was not rewritten",
    );
}
