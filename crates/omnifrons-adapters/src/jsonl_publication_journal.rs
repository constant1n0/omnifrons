//! `JsonlPublicationJournal`: the
//! `omnifrons_app::publication_journal::PublicationJournal` over one
//! append-only `journal/publications.jsonl` file in the product work area
//! (spike slice 5b, HAP-001-R29): device-local, owner-only (`0o600` on
//! unix when created, like the approval store), never a roaming payload.
//! One line per entry -- an `approved` line carrying the identity-bound
//! approval (HAP-001-R22), a `step` line carrying the step, its outcome,
//! the digest, the reason, the UTC time, and, for `published-local`, the
//! record to register after a restart -- each with `schema`. Every line is
//! `sync_data`ed before `append` returns, so the journal entry exists
//! before the next step runs. A line that does not parse exactly, or of
//! another schema, fails the whole replay as `Corrupt`.

use std::io::Write as _;
use std::path::PathBuf;

use omnifrons_app::publication_journal::{JournalError, PublicationJournal};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::executable::{ApprovalId, DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, DetectedType, RunId};
use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactApprovalId, AssetRootId, CatalogRecord, DisplayName, JournalEntry,
    JournalStep, ProjectIdentity, PublicationIdentity, StepEntry, StepOutcome,
};
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::{AttributionDto, Corrupt, RecordDto, TimestampDto};

/// The only `schema` value this version writes or accepts.
const SCHEMA_VERSION: u32 = 1;

/// One line of `publications.jsonl`: the `event` tag, then the fields of
/// the line's own struct. Both payloads are boxed so the enum stays small
/// whichever line is larger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
enum LogEntry {
    Approved(Box<ApprovedLine>),
    Step(Box<StepLine>),
}

/// An `approved` line. `approver` (`DeviceLocalUser`) is a zero-data
/// marker and is not persisted, as in the approval store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovedLine {
    schema: u32,
    approval_id: String,
    publication_id: String,
    project_id: String,
    /// `null` for an approval made from the whole-outbox inventory (spike
    /// slice 5c); a line written before that slice always carries a run.
    run_id: Option<String>,
    name: String,
    display_name: String,
    sha256: String,
    size: u64,
    detected_type: String,
    class: String,
    attribution: AttributionDto,
    asset_root_id: String,
    adapter_id: Option<String>,
    executable_approval_id: Option<u64>,
    approved_at: TimestampDto,
}

/// A `step` line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StepLine {
    schema: u32,
    publication_id: String,
    approval_id: String,
    step: String,
    outcome: String,
    digest: String,
    reason: Option<String>,
    at: TimestampDto,
    record: Option<RecordDto>,
    recovery: Option<String>,
}

impl LogEntry {
    const fn schema(&self) -> u32 {
        match self {
            Self::Approved(line) => line.schema,
            Self::Step(line) => line.schema,
        }
    }

    fn from_domain(entry: &JournalEntry) -> Self {
        match entry {
            JournalEntry::Approved(approval) => Self::Approved(Box::new(ApprovedLine {
                schema: SCHEMA_VERSION,
                approval_id: approval.approval_id.to_hex(),
                publication_id: approval.publication_id.to_hex(),
                project_id: approval.project.to_hex(),
                run_id: approval
                    .run_id
                    .as_ref()
                    .map(|run_id| run_id.as_str().to_string()),
                name: approval.name.clone(),
                display_name: approval.display_name.as_str().to_string(),
                sha256: approval.digest.to_hex(),
                size: approval.size,
                detected_type: approval.detected_type.as_str().to_string(),
                class: approval.class.as_str().to_string(),
                attribution: AttributionDto::from(&approval.attribution),
                asset_root_id: approval.asset_root_id.as_str().to_string(),
                adapter_id: approval
                    .adapter_id
                    .as_ref()
                    .map(|id| id.as_str().to_string()),
                executable_approval_id: approval.executable_approval.map(|id| id.0),
                approved_at: approval.approved_at.into(),
            })),
            JournalEntry::Step(step) => Self::Step(Box::new(StepLine {
                schema: SCHEMA_VERSION,
                publication_id: step.publication_id.to_hex(),
                approval_id: step.approval_id.to_hex(),
                step: step.step.as_str().to_string(),
                outcome: step.outcome.as_str().to_string(),
                digest: step.digest.to_hex(),
                reason: step.reason.clone(),
                at: step.at.into(),
                record: step.record.as_ref().map(RecordDto::from),
                recovery: step.recovery.map(|digest| digest.to_hex()),
            })),
        }
    }

    fn into_domain(self) -> Result<JournalEntry, Corrupt> {
        match self {
            Self::Approved(line) => {
                let line = *line;
                Ok(JournalEntry::Approved(Box::new(ArtifactApproval {
                    approval_id: ArtifactApprovalId::from_hex(&line.approval_id).ok_or(Corrupt)?,
                    publication_id: PublicationIdentity::from_hex(&line.publication_id)
                        .ok_or(Corrupt)?,
                    project: ProjectIdentity::from_hex(&line.project_id).ok_or(Corrupt)?,
                    run_id: match line.run_id {
                        Some(token) => Some(RunId::new(token).map_err(|_| Corrupt)?),
                        None => None,
                    },
                    name: line.name,
                    display_name: DisplayName::sanitize(&line.display_name),
                    digest: Sha256Digest::from_hex(&line.sha256).ok_or(Corrupt)?,
                    size: line.size,
                    detected_type: DetectedType::parse(&line.detected_type).ok_or(Corrupt)?,
                    class: ArtifactClass::parse(&line.class).ok_or(Corrupt)?,
                    attribution: line.attribution.try_into()?,
                    asset_root_id: AssetRootId::new(line.asset_root_id).map_err(|_| Corrupt)?,
                    adapter_id: match line.adapter_id {
                        Some(token) => Some(AdapterId::parse(&token).ok_or(Corrupt)?),
                        None => None,
                    },
                    executable_approval: line.executable_approval_id.map(ApprovalId),
                    approver: DeviceLocalUser,
                    approved_at: line.approved_at.into(),
                })))
            }
            Self::Step(line) => {
                let line = *line;
                Ok(JournalEntry::Step(Box::new(StepEntry {
                    publication_id: PublicationIdentity::from_hex(&line.publication_id)
                        .ok_or(Corrupt)?,
                    approval_id: ArtifactApprovalId::from_hex(&line.approval_id).ok_or(Corrupt)?,
                    step: JournalStep::parse(&line.step).ok_or(Corrupt)?,
                    outcome: StepOutcome::parse(&line.outcome).ok_or(Corrupt)?,
                    digest: Sha256Digest::from_hex(&line.digest).ok_or(Corrupt)?,
                    reason: line.reason,
                    at: line.at.into(),
                    record: match line.record {
                        Some(record) => Some(CatalogRecord::try_from(record)?),
                        None => None,
                    },
                    recovery: match line.recovery {
                        Some(hex) => Some(Sha256Digest::from_hex(&hex).ok_or(Corrupt)?),
                        None => None,
                    },
                })))
            }
        }
    }
}

/// The JSONL-backed [`PublicationJournal`] of one work area. Stateless
/// beyond its path: every call re-reads or appends to the file.
#[derive(Debug, Clone)]
pub struct JsonlPublicationJournal {
    path: PathBuf,
}

impl JsonlPublicationJournal {
    /// The journal file's name under the work area's `journal/`.
    pub const FILE_NAME: &'static str = "publications.jsonl";

    /// Open (creating if absent, owner-only on unix) the journal under
    /// `work_area`'s journal directory.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::WriteFailed`] if the file could not be
    /// created or opened.
    pub fn open(work_area: &WorkAreaRoot) -> Result<Self, JournalError> {
        let path = work_area.journal_dir().join(Self::FILE_NAME);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(&path).map_err(|_| JournalError::WriteFailed)?;
        Ok(Self { path })
    }
}

impl PublicationJournal for JsonlPublicationJournal {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        let line = serde_json::to_string(&LogEntry::from_domain(entry))
            .map_err(|_| JournalError::WriteFailed)?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|_| JournalError::WriteFailed)?;
        writeln!(file, "{line}").map_err(|_| JournalError::WriteFailed)?;
        file.sync_data().map_err(|_| JournalError::WriteFailed)?;
        Ok(())
    }

    fn replay(&self) -> Result<Vec<JournalEntry>, JournalError> {
        let content = std::fs::read_to_string(&self.path).map_err(|_| JournalError::Unreadable)?;
        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LogEntry = serde_json::from_str(line).map_err(|_| JournalError::Corrupt)?;
            if entry.schema() != SCHEMA_VERSION {
                return Err(JournalError::Corrupt);
            }
            entries.push(entry.into_domain().map_err(|_| JournalError::Corrupt)?);
        }
        Ok(entries)
    }
}
