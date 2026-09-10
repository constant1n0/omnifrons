//! `JsonlIgnoreLedger`: the `omnifrons_app::ignore_ledger::IgnoreLedger`
//! over one append-only `ignore/decisions.jsonl` file in the product work
//! area (spike slice 5d, HAP-001 § Wrong-root detection and remedies).
//!
//! The same discipline as the publication journal: device-local,
//! owner-only (`0o600` on unix when created), never a roaming payload; one
//! line per decision, each carrying `schema`, the project identity, the
//! project-relative name, the digest the decision binds to, and the facts
//! the surface showed; every line `sync_data`ed before `record` returns. A
//! line that does not parse exactly, or of another schema, fails the whole
//! read as `Corrupt` -- never "not ignored", which would silently offer a
//! file the user already dismissed.

use std::io::Write as _;
use std::path::PathBuf;

use omnifrons_app::ignore_ledger::{IgnoreEntry, IgnoreLedger, IgnoreLedgerError};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ArtifactClass, DetectedType};
use omnifrons_domain::publication::ProjectIdentity;
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::TimestampDto;

/// The only `schema` value this version writes or accepts.
const SCHEMA_VERSION: u32 = 1;

/// One line of `decisions.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionLine {
    schema: u32,
    project_id: String,
    name: String,
    sha256: String,
    size: u64,
    detected_type: String,
    class: String,
    ignored_at: TimestampDto,
}

impl DecisionLine {
    fn from_domain(project: &ProjectIdentity, entry: &IgnoreEntry) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            project_id: project.to_hex(),
            name: entry.name.clone(),
            sha256: entry.digest.to_hex(),
            size: entry.size,
            detected_type: entry.detected_type.as_str().to_string(),
            class: entry.class.as_str().to_string(),
            ignored_at: entry.ignored_at.into(),
        }
    }

    /// The project this line belongs to and the decision it records, or
    /// `None` when any field does not parse (the whole read then fails
    /// closed).
    fn into_domain(self) -> Option<(ProjectIdentity, IgnoreEntry)> {
        if self.schema != SCHEMA_VERSION {
            return None;
        }
        let project = ProjectIdentity::from_hex(&self.project_id)?;
        let entry = IgnoreEntry {
            name: self.name,
            digest: Sha256Digest::from_hex(&self.sha256)?,
            size: self.size,
            detected_type: DetectedType::parse(&self.detected_type)?,
            class: ArtifactClass::parse(&self.class)?,
            ignored_at: self.ignored_at.into(),
        };
        Some((project, entry))
    }
}

/// The JSONL-backed [`IgnoreLedger`] of one work area. Stateless beyond
/// its path: every call re-reads or appends to the file.
#[derive(Debug, Clone)]
pub struct JsonlIgnoreLedger {
    path: PathBuf,
}

impl JsonlIgnoreLedger {
    /// The ledger file's name under the work area's `ignore/`.
    pub const FILE_NAME: &'static str = "decisions.jsonl";

    /// Open (creating if absent, owner-only on unix) the ledger under
    /// `work_area`'s ignore directory.
    ///
    /// # Errors
    ///
    /// Returns [`IgnoreLedgerError::WriteFailed`] if the file could not be
    /// created or opened.
    pub fn open(work_area: &WorkAreaRoot) -> Result<Self, IgnoreLedgerError> {
        let path = work_area.ignore_dir().join(Self::FILE_NAME);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options
            .open(&path)
            .map_err(|_| IgnoreLedgerError::WriteFailed)?;
        Ok(Self { path })
    }
}

impl IgnoreLedger for JsonlIgnoreLedger {
    fn record(
        &mut self,
        project: &ProjectIdentity,
        entry: &IgnoreEntry,
    ) -> Result<(), IgnoreLedgerError> {
        let line = serde_json::to_string(&DecisionLine::from_domain(project, entry))
            .map_err(|_| IgnoreLedgerError::WriteFailed)?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|_| IgnoreLedgerError::WriteFailed)?;
        writeln!(file, "{line}").map_err(|_| IgnoreLedgerError::WriteFailed)?;
        file.sync_data()
            .map_err(|_| IgnoreLedgerError::WriteFailed)?;
        Ok(())
    }

    fn entries(&self, project: &ProjectIdentity) -> Result<Vec<IgnoreEntry>, IgnoreLedgerError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|_| IgnoreLedgerError::Unreadable)?;
        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: DecisionLine =
                serde_json::from_str(line).map_err(|_| IgnoreLedgerError::Corrupt)?;
            let (recorded, entry) = parsed.into_domain().ok_or(IgnoreLedgerError::Corrupt)?;
            if &recorded == project {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}
