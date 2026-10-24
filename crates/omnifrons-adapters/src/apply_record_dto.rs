//! The JSON shape of one apply record (ADR-0005 sub-slice 2c): `camelCase`
//! fields, `schema: 1`, digests and snapshot ids as lowercase hex, fixed
//! `kebab-case` tags, and `deny_unknown_fields` everywhere -- a file that
//! does not match exactly is corrupt, never guessed at. Every closed set
//! is parsed back through its domain type, so what is on disk is checked
//! again on every read.

use omnifrons_app::apply_record::{APPLY_RECORD_SCHEMA_VERSION, AppliedFile, ApplyRecord};
use omnifrons_app::snapshot_store::SnapshotId;
use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::definition_plan::{BlockMode, InputFingerprint, Operation, PlanId};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::ManagedFileKind;
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::{Corrupt, TimestampDto};

/// One apply record file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyRecordDto {
    schema: u32,
    plan_id: String,
    workspace: String,
    agent: String,
    taken_at: TimestampDto,
    files: Vec<AppliedFileDto>,
    operations: Vec<OperationDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppliedFileDto {
    path: String,
    before: FingerprintDto,
    after: Option<String>,
    snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum FingerprintDto {
    Sha256 { hex: String },
    Symlink { target: String },
    Irregular,
    Absent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum OperationDto {
    #[serde(rename = "write-agents")]
    Agents {
        source: String,
        after: String,
        before: FingerprintDto,
    },
    #[serde(rename = "write-wrapper")]
    Wrapper {
        path: String,
        after: String,
        before: FingerprintDto,
    },
    #[serde(rename = "write-block")]
    Block {
        block_kind: String,
        mode: String,
        body: String,
        after: String,
        before: FingerprintDto,
    },
}

impl From<&InputFingerprint> for FingerprintDto {
    fn from(fingerprint: &InputFingerprint) -> Self {
        match fingerprint {
            InputFingerprint::Sha256(digest) => Self::Sha256 {
                hex: digest.to_hex(),
            },
            InputFingerprint::Symlink(target) => Self::Symlink {
                target: target.clone(),
            },
            InputFingerprint::Irregular => Self::Irregular,
            InputFingerprint::Absent => Self::Absent,
        }
    }
}

impl TryFrom<FingerprintDto> for InputFingerprint {
    type Error = Corrupt;

    fn try_from(dto: FingerprintDto) -> Result<Self, Corrupt> {
        Ok(match dto {
            FingerprintDto::Sha256 { hex } => Self::Sha256(digest(&hex)?),
            FingerprintDto::Symlink { target } => Self::Symlink(target),
            FingerprintDto::Irregular => Self::Irregular,
            FingerprintDto::Absent => Self::Absent,
        })
    }
}

fn digest(hex: &str) -> Result<Sha256Digest, Corrupt> {
    Sha256Digest::from_hex(hex).ok_or(Corrupt)
}

impl From<&Operation> for OperationDto {
    fn from(operation: &Operation) -> Self {
        match operation {
            Operation::WriteAgents {
                source,
                after,
                before,
            } => Self::Agents {
                source: source.file_name().to_string(),
                after: after.to_hex(),
                before: before.into(),
            },
            Operation::WriteWrapper {
                kind,
                after,
                before,
            } => Self::Wrapper {
                path: kind.file_name().to_string(),
                after: after.to_hex(),
                before: before.into(),
            },
            Operation::WriteBlock {
                kind,
                mode,
                body,
                after,
                before,
            } => Self::Block {
                block_kind: kind.as_str().to_string(),
                mode: mode.as_str().to_string(),
                body: body.to_hex(),
                after: after.to_hex(),
                before: before.into(),
            },
        }
    }
}

impl TryFrom<OperationDto> for Operation {
    type Error = Corrupt;

    fn try_from(dto: OperationDto) -> Result<Self, Corrupt> {
        Ok(match dto {
            OperationDto::Agents {
                source,
                after,
                before,
            } => Self::WriteAgents {
                source: IdentityKind::from_file_name(&source).ok_or(Corrupt)?,
                after: digest(&after)?,
                before: before.try_into()?,
            },
            OperationDto::Wrapper {
                path,
                after,
                before,
            } => Self::WriteWrapper {
                kind: IdentityKind::from_file_name(&path).ok_or(Corrupt)?,
                after: digest(&after)?,
                before: before.try_into()?,
            },
            OperationDto::Block {
                block_kind,
                mode,
                body,
                after,
                before,
            } => Self::WriteBlock {
                kind: ManagedFileKind::parse(&block_kind).ok_or(Corrupt)?,
                mode: [
                    BlockMode::Append,
                    BlockMode::Replace,
                    BlockMode::MigrateLegacy,
                ]
                .into_iter()
                .find(|candidate| candidate.as_str() == mode)
                .ok_or(Corrupt)?,
                body: digest(&body)?,
                after: digest(&after)?,
                before: before.try_into()?,
            },
        })
    }
}

impl From<&ApplyRecord> for ApplyRecordDto {
    fn from(record: &ApplyRecord) -> Self {
        Self {
            schema: APPLY_RECORD_SCHEMA_VERSION,
            plan_id: Sha256Digest(record.plan_id.0).to_hex(),
            workspace: record.workspace.clone(),
            agent: record.agent.clone(),
            taken_at: record.taken_at.into(),
            files: record
                .files
                .iter()
                .map(|file| AppliedFileDto {
                    path: file.path.clone(),
                    before: (&file.before).into(),
                    after: file.after.as_ref().map(Sha256Digest::to_hex),
                    snapshot: file.snapshot.map(|id| id.to_hex()),
                })
                .collect(),
            operations: record.operations.iter().map(OperationDto::from).collect(),
        }
    }
}

impl TryFrom<ApplyRecordDto> for ApplyRecord {
    type Error = Corrupt;

    fn try_from(dto: ApplyRecordDto) -> Result<Self, Corrupt> {
        if dto.schema != APPLY_RECORD_SCHEMA_VERSION {
            return Err(Corrupt);
        }
        let files = dto
            .files
            .into_iter()
            .map(|file| {
                Ok(AppliedFile {
                    path: file.path,
                    before: file.before.try_into()?,
                    after: file.after.as_deref().map(digest).transpose()?,
                    snapshot: file
                        .snapshot
                        .as_deref()
                        .map(|hex| SnapshotId::from_hex(hex).ok_or(Corrupt))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, Corrupt>>()?;
        let operations = dto
            .operations
            .into_iter()
            .map(Operation::try_from)
            .collect::<Result<Vec<_>, Corrupt>>()?;
        Ok(Self {
            plan_id: PlanId::from_digest(digest(&dto.plan_id)?),
            workspace: dto.workspace,
            agent: dto.agent,
            taken_at: dto.taken_at.into(),
            files,
            operations,
        })
    }
}
