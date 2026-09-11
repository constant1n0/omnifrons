//! `JsonlCatalogStore`: the `omnifrons_app::catalog_store::CatalogStore`
//! over one append-only `.omnifrons/catalog.jsonl` file in the project
//! (spike slice 5b, HAP-001 D9: records are small portable state under the
//! reserved namespace, so a receiving device sees the record before it
//! holds the bytes). One line per event -- a `record` when an artifact is
//! registered, an `alias` when a later identical publication adds a
//! display name (HAP-001-R23) -- each carrying `schema`. A line that does
//! not parse exactly, another schema, an alias naming no record, or a
//! record whose ids disagree fails the whole read as `Corrupt`, never a
//! partial result (the approval store's discipline). An absent file is an
//! empty catalog, and a read creates nothing.
//!
//! Writes are replay-then-append with no lock of their own: the caller
//! serializes them -- the shell holds its publication surface lock across
//! every approve, publish, and list (R1-001).
//!
//! **The repair** (spike slice 5e, surfaced as `catalog-corrupt`, which
//! is distinct from the `catalog-unavailable` a catalog that cannot be
//! read at all renders). Slice 5b named three rules and repaired by hand;
//! this store now implements [`CatalogRepair`] for **two** of them, and
//! refuses the third:
//!
//! - the later of two `record` lines carrying one **`catalogId`** --
//!   dropped, because the surviving line still registers that identity
//!   under that asset root;
//! - an `alias` no earlier `record` line carries -- dropped, because an
//!   alias is a display name under HAP-001-R23 and registers nothing;
//! - a `record` whose `catalogId` is not `<assetRootId>/<publicationId>`
//!   -- **refused**, because it is the only line for that identity and
//!   dropping it would delete a registered artifact, which HAP-001-R39
//!   reserves to MRP-001's tombstone kind `artifact`. So is any line this
//!   version cannot decode: the Catalog is synchronized, portable,
//!   untrusted content (HAP-001-R40), and "drop anything unreadable"
//!   would be a record-deletion path driven by content.
//!
//! The duplicate rule keys on the whole `catalogId` and not on the
//! `publicationId` alone, because HAP-001-R23 derives that identity from
//! `sha256(project ‖ content digest)` with the asset root **outside** the
//! preimage: two `record` lines sharing one `publicationId` under
//! different `assetRootId`s are two registered artifacts, so the later of
//! those is **refused** (`DuplicateAcrossAssetRoots`) by the same R39 rule
//! that refuses a mismatched `catalogId`.
//!
//! Preview, digest binding, explicit apply -- the guidance installer's
//! discipline (HAP-001 D18, spike slice 5c). One read decides everything:
//! the digest compared against the caller's binding, the plan, the copy
//! kept in the work area, and the bytes rewritten all come from the same
//! `read_for_repair`, and the target is re-read through that same
//! no-follow open immediately before the rename.

use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::catalog_repair::{
    CatalogRepair, CatalogRepairError, DroppedLine, REPAIR_COPY_PREFIX, REPAIR_COPY_SUFFIX,
    RefusedLine, RepairOutcome, RepairPlan, RepairRefusal, RepairRule,
};
use omnifrons_app::catalog_store::{CatalogStore, CatalogStoreError};
use omnifrons_app::content_hasher::ContentHasher as _;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::outbox::ContentDigest;
use omnifrons_domain::publication::{CatalogId, CatalogRecord, DisplayName, PublicationIdentity};
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::{RecordDto, TimestampDto};
use crate::fs_project_text_file::{Opened, open_no_follow};
use crate::sha2_hasher::Sha2Hasher;

/// The only `schema` value this version writes or accepts.
const SCHEMA_VERSION: u32 = 1;

/// The process-wide sequence behind the repair's sibling `.part` name, so
/// two repairs of one project never share a temporary file.
static REPAIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One line of `catalog.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum LogEntry {
    Record {
        schema: u32,
        record: Box<RecordDto>,
    },
    Alias {
        schema: u32,
        publication_id: String,
        name: String,
        at: TimestampDto,
    },
}

impl LogEntry {
    const fn schema(&self) -> u32 {
        match self {
            Self::Record { schema, .. } | Self::Alias { schema, .. } => *schema,
        }
    }
}

/// The JSONL-backed [`CatalogStore`] of one project. Stateless beyond its
/// path: every call re-reads the file.
#[derive(Debug, Clone)]
pub struct JsonlCatalogStore {
    path: PathBuf,
}

impl JsonlCatalogStore {
    /// Where the catalog lives, relative to the project root.
    pub const FILE_PATH: &'static str = ".omnifrons/catalog.jsonl";

    /// The store for `project`. Nothing is created until the first write.
    ///
    /// # Errors
    ///
    /// Never fails today; the `Result` is the port's shape for a store
    /// that may need to prepare itself.
    pub fn open(project: &WorkspaceRoot) -> Result<Self, CatalogStoreError> {
        Ok(Self {
            path: project.path().join(Self::FILE_PATH),
        })
    }

    fn append_line(&self, entry: &LogEntry) -> Result<(), CatalogStoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CatalogStoreError::WriteFailed)?;
        }
        let line = serde_json::to_string(entry).map_err(|_| CatalogStoreError::WriteFailed)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|_| CatalogStoreError::WriteFailed)?;
        writeln!(file, "{line}").map_err(|_| CatalogStoreError::WriteFailed)?;
        file.sync_data()
            .map_err(|_| CatalogStoreError::WriteFailed)?;
        Ok(())
    }

    fn replay(&self) -> Result<Vec<CatalogRecord>, CatalogStoreError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(CatalogStoreError::Unreadable),
        };
        Self::replay_str(&content)
    }

    /// The records `content` carries, with its aliases applied. The one
    /// reader: `replay` calls it over the file, and the repair calls it
    /// over the lines it is about to keep, so what the repair checks and
    /// what the store accepts can never drift apart (spike slice 5e).
    fn replay_str(content: &str) -> Result<Vec<CatalogRecord>, CatalogStoreError> {
        let mut records: Vec<CatalogRecord> = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LogEntry =
                serde_json::from_str(line).map_err(|_| CatalogStoreError::Corrupt)?;
            if entry.schema() != SCHEMA_VERSION {
                return Err(CatalogStoreError::Corrupt);
            }
            match entry {
                LogEntry::Record { record, .. } => {
                    let record =
                        CatalogRecord::try_from(*record).map_err(|_| CatalogStoreError::Corrupt)?;
                    if records
                        .iter()
                        .any(|existing| existing.publication_id == record.publication_id)
                    {
                        return Err(CatalogStoreError::Corrupt);
                    }
                    records.push(record);
                }
                LogEntry::Alias {
                    publication_id,
                    name,
                    ..
                } => {
                    let id = PublicationIdentity::from_hex(&publication_id)
                        .ok_or(CatalogStoreError::Corrupt)?;
                    let record = records
                        .iter_mut()
                        .find(|record| record.publication_id == id)
                        .ok_or(CatalogStoreError::Corrupt)?;
                    let name = DisplayName::sanitize(&name);
                    if !record.names.contains(&name) {
                        record.names.push(name);
                    }
                }
            }
        }
        Ok(records)
    }
}

/// One classified line: its raw bytes including the terminator, its
/// one-based number, and what the rules decided about it.
type ClassifiedLine<'a> = (&'a [u8], u32, LineDecision);

/// What the repair decided about one line of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LineDecision {
    /// The line stays, byte for byte.
    Keep,
    /// The line may be dropped: it deletes no registration.
    Drop(RepairRule, Option<PublicationIdentity>),
    /// The line refuses the whole repair.
    Refuse(RepairRefusal),
}

/// The catalog's lines as the repair sees them: the raw bytes of each
/// line including its terminator (so a kept line is rewritten exactly as
/// it was), its one-based number, and what the rules decided -- plus how
/// many distinct `catalogId`s the kept `record` lines register, which is
/// the record count the repaired catalog would carry.
///
/// That second number counts **kept** lines only, so on a plan that
/// carries a refusal it is an undercount of the registrations the file
/// holds: a refused `record` registers an identity the repair would not
/// remove, and it is deliberately not counted, because the number
/// describes the catalog a repair would leave and a refused plan leaves
/// the file exactly as it is. [`RepairPlan::kept_records`] says the same.
fn classify(bytes: &[u8]) -> (Vec<ClassifiedLine<'_>>, usize) {
    let mut seen: Vec<CatalogId> = Vec::new();
    let mut classified = Vec::new();
    for (index, raw) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let decision = decide(raw, &mut seen);
        classified.push((raw, number, decision));
    }
    (classified, seen.len())
}

/// The two rules and the three refusals, applied to one line. `seen` is
/// the `catalogId`s the `record` lines *before* this one registered,
/// which is exactly what [`JsonlCatalogStore::replay_str`] has in hand at
/// the same point -- so "an alias no earlier record carries" is the
/// condition under which the reader itself fails, and not a second,
/// looser reading of it.
fn decide(raw: &[u8], seen: &mut Vec<CatalogId>) -> LineDecision {
    let trimmed = raw.strip_suffix(b"\n").unwrap_or(raw);
    let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
    let Ok(text) = std::str::from_utf8(trimmed) else {
        return LineDecision::Refuse(RepairRefusal::Unparsable);
    };
    if text.trim().is_empty() {
        return LineDecision::Keep;
    }
    let Ok(entry) = serde_json::from_str::<LogEntry>(text) else {
        return LineDecision::Refuse(RepairRefusal::Unparsable);
    };
    if entry.schema() != SCHEMA_VERSION {
        return LineDecision::Refuse(RepairRefusal::Unparsable);
    }
    match entry {
        LogEntry::Record { record, .. } => {
            let dto = *record;
            match CatalogRecord::try_from(dto.clone()) {
                Ok(record) => {
                    if seen.contains(&record.catalog_id) {
                        LineDecision::Drop(RepairRule::DuplicateRecord, Some(record.publication_id))
                    } else if seen
                        .iter()
                        .any(|existing| existing.publication_id == record.publication_id)
                    {
                        // One `publicationId` under two asset roots is two
                        // `catalogId`s, so this line is the only one
                        // registering *its* artifact. HAP-001-R39 again.
                        LineDecision::Refuse(RepairRefusal::DuplicateAcrossAssetRoots)
                    } else {
                        seen.push(record.catalog_id);
                        LineDecision::Keep
                    }
                }
                // HAP-001-R39: the only line for that identity. Refused,
                // never dropped -- deletion is MRP-001's tombstone kind.
                //
                // This arm comes first **on purpose**, so a line that is
                // both id-disagreeing and undecodable is reported as the
                // mismatch: `ids_disagree` compares the three fields as
                // text precisely so the R39 rule is recognized on a line
                // nothing else about which decodes. Both arms refuse the
                // whole repair and leave the line alone, so only the
                // reason the preview shows is at stake here.
                Err(_) if dto.ids_disagree() => {
                    LineDecision::Refuse(RepairRefusal::CatalogIdMismatch)
                }
                Err(_) => LineDecision::Refuse(RepairRefusal::Unparsable),
            }
        }
        LogEntry::Alias { publication_id, .. } => {
            match PublicationIdentity::from_hex(&publication_id) {
                Some(id) if seen.iter().any(|existing| existing.publication_id == id) => {
                    LineDecision::Keep
                }
                // An identity no `record` line before this one carries -- or
                // one no record could carry, because it is not 64 hex
                // characters. Either way the line adds a display name to
                // nothing (HAP-001-R23).
                id => LineDecision::Drop(RepairRule::DanglingAlias, id),
            }
        }
    }
}

impl JsonlCatalogStore {
    /// The one hop between the canonical workspace root and the catalog
    /// that neither `WorkspaceRoot` nor the no-follow open covers:
    /// `.omnifrons` itself. `WorkspaceRoot::new` canonicalizes the root
    /// and `open_no_follow` refuses a link at the *leaf*, so without this
    /// a `.omnifrons` that is a symbolic link reaches a directory outside
    /// the project and the repair reads -- and rewrites -- a file there.
    /// `symlink_metadata` reports the link itself, never its target.
    ///
    /// **This guards the repair's paths and nothing else.** The store's
    /// own [`Self::append_line`] still creates the directory and opens
    /// `catalog.jsonl` by path with no no-follow of any kind, at the leaf
    /// or above it; closing that is a separate change to the write path
    /// every publication takes. And the window between this check and the
    /// open that follows it stays open, like the one between the
    /// pre-rename re-read and the rename itself: both are the residual
    /// `FsProjectTextFile` already discloses.
    fn guarded_parent(&self) -> Result<(), CatalogRepairError> {
        let Some(parent) = self.path.parent() else {
            return Err(CatalogRepairError::Unreadable);
        };
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
            // A link, a regular file, or anything else at `.omnifrons`:
            // whatever is there, it is not this project's own catalog
            // directory, and a stat that fails for any other reason says
            // nothing either.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(CatalogRepairError::Absent)
            }
            Ok(_) | Err(_) => Err(CatalogRepairError::Unreadable),
        }
    }

    /// Every byte of the catalog, through one open that does not follow a
    /// link at its name, under a `.omnifrons` that is a real directory and
    /// not a link to one. The file sits inside the project, so both names
    /// are ones a same-user producer can plant at; `FsProjectTextFile` set
    /// this discipline for the two other files this product rewrites
    /// inside a workspace, and the repair uses the same open.
    fn read_for_repair(&self) -> Result<Vec<u8>, CatalogRepairError> {
        self.guarded_parent()?;
        let mut file = match open_no_follow(&self.path) {
            Opened::File(file) => file,
            Opened::Absent => return Err(CatalogRepairError::Absent),
            Opened::NotAFile | Opened::Unreadable => {
                return Err(CatalogRepairError::Unreadable);
            }
        };
        let metadata = file
            .metadata()
            .map_err(|_| CatalogRepairError::Unreadable)?;
        if !metadata.file_type().is_file() {
            return Err(CatalogRepairError::Unreadable);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| CatalogRepairError::Unreadable)?;
        Ok(bytes)
    }

    /// The plan `bytes` gives, with `sha256` recorded as the binding an
    /// apply has to send back.
    fn plan_for(bytes: &[u8], sha256: ContentDigest) -> RepairPlan {
        let (classified, kept_records) = classify(bytes);
        let mut drops = Vec::new();
        let mut refusals = Vec::new();
        for (_, line, decision) in classified {
            match decision {
                LineDecision::Keep => {}
                LineDecision::Drop(rule, publication_id) => drops.push(DroppedLine {
                    line,
                    rule,
                    publication_id,
                }),
                LineDecision::Refuse(refusal) => refusals.push(RefusedLine { line, refusal }),
            }
        }
        RepairPlan {
            sha256,
            drops,
            refusals,
            kept_records,
        }
    }

    /// The bytes of every line the plan keeps, in their original order and
    /// byte for byte -- blank lines included -- ending in a newline: the
    /// store appends with `writeln!` in append mode, so a repaired file
    /// that did not end in one would have the next registration glued onto
    /// its last line.
    ///
    /// **One newline is added, never removed.** A last kept line that
    /// carried no terminator gets one; a catalog whose last kept line is
    /// blank already ends in a newline and keeps both of them, so the
    /// repaired file ends in two. That is not "exactly one trailing
    /// newline", and it cannot be: normalising the end of the file would
    /// contradict keeping every kept line byte for byte, and only the
    /// gluing is what the newline is for.
    fn kept_bytes(bytes: &[u8]) -> Vec<u8> {
        let (classified, _) = classify(bytes);
        let mut kept = Vec::with_capacity(bytes.len());
        for (raw, _, decision) in classified {
            if decision == LineDecision::Keep {
                kept.extend_from_slice(raw);
            }
        }
        if !kept.is_empty() && !kept.ends_with(b"\n") {
            kept.push(b'\n');
        }
        kept
    }

    /// The pre-repair copy: the exact bytes the repair read, written into
    /// the work area's `recovery/` directory under a timestamped name,
    /// owner-only, and created exclusively so nothing is ever replaced.
    /// Written **before** the catalog is rewritten, so the repair is
    /// reversible and the untouched bytes sit outside the synchronized
    /// `.omnifrons/` namespace (HAP-001-R7, R40). Returns the path it
    /// wrote, because a rewrite that then fails has to take it back: a
    /// repair that did not happen leaves no copy.
    fn write_repair_copy(
        work_area: &WorkAreaRoot,
        bytes: &[u8],
        at: SystemTime,
    ) -> Result<PathBuf, CatalogRepairError> {
        let nanos = at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| CatalogRepairError::CopyFailed)?
            .as_nanos();
        let path = work_area
            .recovery_dir()
            .join(format!("{REPAIR_COPY_PREFIX}{nanos}{REPAIR_COPY_SUFFIX}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| CatalogRepairError::CopyFailed)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_data())
            .map_err(|_| CatalogRepairError::CopyFailed)?;
        Ok(path)
    }

    /// Replace the catalog with `repaired`, bound to the `expected` bytes
    /// the plan was made from: a sibling `.part` created exclusively, the
    /// original's permissions carried over, and the target re-read through
    /// the same no-follow open immediately before the rename -- the
    /// `FsProjectTextFile` discipline, which exists because an earlier
    /// slice lost an edit in exactly this window.
    fn replace_bytes(&self, expected: &[u8], repaired: &[u8]) -> Result<(), CatalogRepairError> {
        // The `.part` is created in `.omnifrons` before anything is read
        // back, so the directory is checked here too and not only through
        // the re-read below.
        self.guarded_parent()?;
        let existing = std::fs::symlink_metadata(&self.path)
            .ok()
            .filter(std::fs::Metadata::is_file);
        let sequence = REPAIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .path
            .with_file_name(format!(".catalog.{}-{sequence}.part", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| CatalogRepairError::WriteFailed)?;
        let prepared = file
            .write_all(repaired)
            .and_then(|()| file.sync_data())
            .and_then(|()| match &existing {
                Some(metadata) => file.set_permissions(metadata.permissions()),
                None => Ok(()),
            });
        drop(file);
        if prepared.is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(CatalogRepairError::WriteFailed);
        }
        // The last thing before the rename: the target is still the bytes
        // this repair was planned from, or nothing is written.
        let current = self.read_for_repair();
        if current.as_deref() != Ok(expected) {
            let _ = std::fs::remove_file(&temp);
            return Err(CatalogRepairError::Changed);
        }
        if std::fs::rename(&temp, &self.path).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(CatalogRepairError::WriteFailed);
        }
        Ok(())
    }
}

impl CatalogRepair for JsonlCatalogStore {
    fn preview(&self) -> Result<RepairPlan, CatalogRepairError> {
        let bytes = self.read_for_repair()?;
        let sha256 = Sha2Hasher::new().sha256(&bytes);
        Ok(Self::plan_for(&bytes, sha256))
    }

    fn repair(
        &mut self,
        work_area: &WorkAreaRoot,
        expected: &ContentDigest,
        at: SystemTime,
    ) -> Result<RepairOutcome, CatalogRepairError> {
        // One read decides everything: the digest that is compared with
        // the caller's binding, the plan, the copy, and the bytes that are
        // rewritten all come from these bytes and no others.
        let bytes = self.read_for_repair()?;
        let hasher = Sha2Hasher::new();
        let original_sha256 = hasher.sha256(&bytes);
        if &original_sha256 != expected {
            return Err(CatalogRepairError::Changed);
        }
        let plan = Self::plan_for(&bytes, original_sha256);
        if let Some(refused) = plan.refusals.first() {
            return Err(CatalogRepairError::Refused(refused.refusal));
        }
        if plan.drops.is_empty() {
            return Err(CatalogRepairError::NothingToRepair);
        }
        let repaired = Self::kept_bytes(&bytes);
        // The guard: what is about to be written has to read as a catalog.
        let readable = std::str::from_utf8(&repaired)
            .ok()
            .and_then(|text| Self::replay_str(text).ok());
        if readable.is_none() {
            return Err(CatalogRepairError::Unrepairable);
        }
        let copy = Self::write_repair_copy(work_area, &bytes, at)?;
        // The copy is the original of a repair that *happened*. A rewrite
        // that fails here -- `Changed` because the file moved under the
        // apply, `WriteFailed` because the sibling could not be staged --
        // leaves the catalog untouched, so the copy is taken back rather
        // than left in `recovery/` as the original of nothing.
        if let Err(error) = self.replace_bytes(&bytes, &repaired) {
            let _ = std::fs::remove_file(&copy);
            return Err(error);
        }
        Ok(RepairOutcome {
            original_sha256,
            sha256: hasher.sha256(&repaired),
            dropped_lines: plan.drops.len(),
            kept_records: plan.kept_records,
        })
    }
}

impl CatalogStore for JsonlCatalogStore {
    fn list(&self) -> Result<Vec<CatalogRecord>, CatalogStoreError> {
        self.replay()
    }

    fn register(&mut self, record: CatalogRecord) -> Result<(), CatalogStoreError> {
        if self
            .replay()?
            .iter()
            .any(|existing| existing.publication_id == record.publication_id)
        {
            return Err(CatalogStoreError::Duplicate);
        }
        self.append_line(&LogEntry::Record {
            schema: SCHEMA_VERSION,
            record: Box::new(RecordDto::from(&record)),
        })
    }

    fn add_alias(
        &mut self,
        id: &PublicationIdentity,
        name: DisplayName,
    ) -> Result<(), CatalogStoreError> {
        let existing = self
            .replay()?
            .into_iter()
            .find(|record| &record.publication_id == id)
            .ok_or(CatalogStoreError::Unknown)?;
        if existing.names.contains(&name) {
            return Ok(());
        }
        self.append_line(&LogEntry::Alias {
            schema: SCHEMA_VERSION,
            publication_id: id.to_hex(),
            name: name.as_str().to_string(),
            at: SystemTime::now().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::catalog_repair::CatalogRepairError;

    use super::JsonlCatalogStore;

    /// The write binds to the bytes the plan was made from: `replace_bytes`
    /// re-reads the target through the same no-follow open immediately
    /// before the rename and refuses when it is no longer those bytes.
    ///
    /// This is the guard, exercised directly rather than through a race:
    /// without the re-read the rename would land and this would be `Ok`,
    /// silently discarding whatever arrived in the window -- which is the
    /// defect spike slice 5c's R1-002 found in `ProjectTextFile`.
    #[test]
    fn a_write_whose_target_is_not_the_bytes_it_was_planned_from_is_refused() {
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-catalog-rebind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("after the epoch")
                .as_nanos()
        ));
        let root = dir.join("project");
        std::fs::create_dir_all(root.join(".omnifrons")).expect("project");
        let path = root.join(JsonlCatalogStore::FILE_PATH);
        std::fs::write(&path, b"arrived\n").expect("catalog");
        let workspace = WorkspaceRoot::new(&root).expect("workspace");
        let store = JsonlCatalogStore::open(&workspace).expect("store");

        assert_eq!(
            store
                .replace_bytes(b"planned-from\n", b"repaired\n")
                .expect_err("the target is not what was planned from"),
            CatalogRepairError::Changed,
        );
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"arrived\n",
            "and the arriving bytes are still there",
        );
        let siblings: Vec<String> = std::fs::read_dir(root.join(".omnifrons"))
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
            .collect();
        assert_eq!(
            siblings,
            vec!["catalog.jsonl".to_string()],
            "and the refused write left no .part sibling behind",
        );
    }
}
