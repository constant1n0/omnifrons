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
    // Struct variants, not unit ones: serde does not deny extra siblings
    // of an internally tagged unit variant, and a record must fail closed.
    Irregular {},
    Absent {},
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
    #[serde(rename = "write-generated")]
    Generated {
        path: String,
        template_version: String,
        answers_digest: String,
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
            InputFingerprint::Irregular => Self::Irregular {},
            InputFingerprint::Absent => Self::Absent {},
        }
    }
}

impl TryFrom<FingerprintDto> for InputFingerprint {
    type Error = Corrupt;

    fn try_from(dto: FingerprintDto) -> Result<Self, Corrupt> {
        Ok(match dto {
            FingerprintDto::Sha256 { hex } => Self::Sha256(digest(&hex)?),
            FingerprintDto::Symlink { target } => Self::Symlink(target),
            FingerprintDto::Irregular {} => Self::Irregular,
            FingerprintDto::Absent {} => Self::Absent,
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
            Operation::WriteGenerated {
                template_version,
                answers_digest,
                after,
                before,
            } => Self::Generated {
                path: operation.path().to_string(),
                template_version: template_version.clone(),
                answers_digest: answers_digest.to_hex(),
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
            OperationDto::Generated {
                path,
                template_version,
                answers_digest,
                after,
                before,
            } => {
                if path != IdentityKind::Agents.file_name() {
                    return Err(Corrupt);
                }
                Self::WriteGenerated {
                    template_version,
                    answers_digest: digest(&answers_digest)?,
                    after: digest(&after)?,
                    before: before.try_into()?,
                }
            }
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
                mode: BlockMode::parse(&mode).ok_or(Corrupt)?,
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

#[cfg(test)]
mod tests {
    use super::ApplyRecordDto;
    use omnifrons_app::apply_record::{AppliedFile, ApplyRecord};
    use omnifrons_app::snapshot_store::SnapshotId;
    use omnifrons_domain::definition::IdentityKind;
    use omnifrons_domain::definition_plan::{BlockMode, InputFingerprint, Operation, PlanId};
    use omnifrons_domain::executable::Sha256Digest;
    use omnifrons_domain::guidance::ManagedFileKind;
    use serde_json::{Value, json};
    use std::time::{Duration, SystemTime};

    fn record() -> ApplyRecord {
        let before = InputFingerprint::Sha256(Sha256Digest([1; 32]));
        ApplyRecord {
            plan_id: PlanId([9; 32]),
            workspace: "w".to_string(),
            agent: ".".to_string(),
            taken_at: SystemTime::UNIX_EPOCH + Duration::from_secs(5),
            files: vec![AppliedFile {
                path: "AGENTS.md".to_string(),
                before: before.clone(),
                after: Some(Sha256Digest([2; 32])),
                snapshot: Some(SnapshotId(3)),
            }],
            operations: vec![
                Operation::WriteAgents {
                    source: IdentityKind::Claude,
                    after: Sha256Digest([2; 32]),
                    before: before.clone(),
                },
                Operation::WriteBlock {
                    kind: ManagedFileKind::Skills,
                    mode: BlockMode::Append,
                    body: Sha256Digest([4; 32]),
                    after: Sha256Digest([5; 32]),
                    before: InputFingerprint::Absent,
                },
            ],
        }
    }

    fn complete() -> Value {
        serde_json::to_value(ApplyRecordDto::from(&record())).expect("serialize")
    }

    /// Whether `value` decodes all the way to a record.
    fn decodes(value: &Value) -> bool {
        serde_json::from_value::<ApplyRecordDto>(value.clone())
            .ok()
            .and_then(|dto| ApplyRecord::try_from(dto).ok())
            .is_some()
    }

    #[test]
    fn the_complete_fixture_round_trips() {
        let value = complete();
        assert!(decodes(&value));
        let dto: ApplyRecordDto = serde_json::from_value(value).expect("parse");
        assert_eq!(ApplyRecord::try_from(dto), Ok(record()));
    }

    #[test]
    fn another_schema_reaches_the_gate_and_is_refused() {
        let mut value = complete();
        value["schema"] = json!(2);
        let dto: ApplyRecordDto =
            serde_json::from_value(value).expect("the shape is complete, so serde accepts it");
        assert!(ApplyRecord::try_from(dto).is_err());
    }

    #[test]
    fn an_unknown_field_at_any_level_is_refused() {
        let mut cases = Vec::new();
        for pointer in [
            "",
            "/files/0",
            "/files/0/before",
            "/operations/0",
            "/operations/0/before",
            "/operations/1/before",
        ] {
            let mut value = complete();
            value
                .pointer_mut(pointer)
                .expect("level")
                .as_object_mut()
                .expect("object")
                .insert("extra".to_string(), json!(true));
            cases.push((pointer, value));
        }
        for (pointer, value) in cases {
            assert!(
                !decodes(&value),
                "an extra field at {pointer:?} was accepted"
            );
        }
    }

    #[test]
    fn an_unknown_tag_or_value_is_refused() {
        let mutations: [(&str, Value); 5] = [
            ("/operations/0/op", json!("write-everything")),
            ("/files/0/before/kind", json!("fifo")),
            ("/operations/1/mode", json!("rewrite")),
            ("/operations/1/blockKind", json!("nonsense")),
            ("/files/0/after", json!("not-hex")),
        ];
        for (pointer, replacement) in mutations {
            let mut value = complete();
            *value.pointer_mut(pointer).expect("field") = replacement;
            assert!(!decodes(&value), "{pointer} accepted a bad value");
        }
        for (pointer, replacement) in [
            ("/planId", json!("abcd")),
            ("/files/0/snapshot", json!("xyz")),
            ("/operations/0/source", json!("OTHER.md")),
        ] {
            let mut value = complete();
            *value.pointer_mut(pointer).expect("field") = replacement;
            assert!(!decodes(&value), "{pointer} accepted a bad value");
        }
    }

    fn generated_record() -> ApplyRecord {
        ApplyRecord {
            operations: vec![Operation::WriteGenerated {
                template_version: "adr-0005-create-v1".to_string(),
                answers_digest: Sha256Digest([6; 32]),
                after: Sha256Digest([7; 32]),
                before: InputFingerprint::Absent,
            }],
            ..record()
        }
    }

    #[test]
    fn a_generated_write_serialises_to_the_pinned_fragment_and_decodes_back() {
        let value = serde_json::to_value(ApplyRecordDto::from(&generated_record())).unwrap();
        assert_eq!(
            value["operations"][0],
            json!({
                "op": "write-generated",
                "path": "AGENTS.md",
                "templateVersion": "adr-0005-create-v1",
                "answersDigest": "06".repeat(32),
                "after": "07".repeat(32),
                "before": {"kind": "absent"},
            })
        );
        let dto: ApplyRecordDto = serde_json::from_value(value).expect("parse");
        assert_eq!(ApplyRecord::try_from(dto), Ok(generated_record()));
    }

    #[test]
    fn a_generated_write_with_another_path_or_a_bad_digest_is_corrupt() {
        let base = serde_json::to_value(ApplyRecordDto::from(&generated_record())).unwrap();
        for (pointer, replacement) in [
            ("/operations/0/path", json!("CLAUDE.md")),
            ("/operations/0/answersDigest", json!("not-hex")),
            ("/operations/0/answersDigest", json!("abcd")),
            ("/operations/0/after", json!("zz")),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).expect("field") = replacement.clone();
            let dto: ApplyRecordDto = serde_json::from_value(value).expect("the shape is complete");
            assert_eq!(
                ApplyRecord::try_from(dto),
                Err(super::Corrupt),
                "{pointer} = {replacement}"
            );
        }
    }
}
