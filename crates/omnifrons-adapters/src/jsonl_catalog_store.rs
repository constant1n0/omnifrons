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
//! every approve, publish, and list (R1-001). Repair path for a `Corrupt`
//! catalog (surfaced as `catalog-unavailable`): the file is line-oriented
//! JSON under the project's `.omnifrons/`, so a person removes the
//! offending line by hand -- the later of two `record` lines with one
//! `publicationId`, an `alias` whose `publicationId` no record carries, or
//! a `record` whose `catalogId` is not `<assetRootId>/<publicationId>` --
//! and every other line is kept; a `catalog_repair` command that does this
//! with the same rules is slice-5c debt, named in `docs/spike-log.md`.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::SystemTime;

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::catalog_store::{CatalogStore, CatalogStoreError};
use omnifrons_domain::publication::{CatalogRecord, DisplayName, PublicationIdentity};
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::{RecordDto, TimestampDto};

/// The only `schema` value this version writes or accepts.
const SCHEMA_VERSION: u32 = 1;

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
