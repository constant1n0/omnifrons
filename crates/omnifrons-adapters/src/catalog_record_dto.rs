//! The JSON shape of a Catalog record and of the small values it and the
//! publication journal share (spike slice 5b): `camelCase` fields, fixed
//! `kebab-case` tokens, every closed set parsed back through its domain
//! type's own `parse`, and `deny_unknown_fields` everywhere -- a file that
//! does not match exactly is corrupt, never guessed at. Every display name
//! read back is sanitized again: a Catalog record is portable state under
//! `.omnifrons/`, so what is on disk is untrusted content (HAP-001-R24,
//! R40).

use std::time::{Duration, SystemTime};

use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::executable::{ApprovalId, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType, RunId};
use omnifrons_domain::publication::{
    ArtifactState, AssetRootId, CatalogId, CatalogRecord, DIGEST_ALGORITHM_SHA256, DisplayName,
    Producer, ProjectIdentity, Provenance, ProviderLocator, ProviderRecord, ProviderState,
    PublicationIdentity, Transition,
};
use serde::{Deserialize, Serialize};

/// A line that does not decode into the expected shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Corrupt;

/// A wall-clock instant split into whole seconds and nanoseconds since
/// the Unix epoch (the approval store's own shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimestampDto {
    pub secs: u64,
    pub nanos: u32,
}

impl From<SystemTime> for TimestampDto {
    fn from(time: SystemTime) -> Self {
        let since_epoch = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        Self {
            secs: since_epoch.as_secs(),
            nanos: since_epoch.subsec_nanos(),
        }
    }
}

impl From<TimestampDto> for SystemTime {
    fn from(dto: TimestampDto) -> Self {
        Self::UNIX_EPOCH + Duration::new(dto.secs, dto.nanos)
    }
}

/// An attribution, as the journal carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum AttributionDto {
    Run { run_id: String },
    Unattributed,
}

impl From<&Attribution> for AttributionDto {
    fn from(attribution: &Attribution) -> Self {
        match attribution {
            Attribution::Run(run_id) => Self::Run {
                run_id: run_id.as_str().to_string(),
            },
            Attribution::Unattributed => Self::Unattributed,
        }
    }
}

impl TryFrom<AttributionDto> for Attribution {
    type Error = Corrupt;

    fn try_from(dto: AttributionDto) -> Result<Self, Corrupt> {
        Ok(match dto {
            AttributionDto::Run { run_id } => Self::Run(RunId::new(run_id).map_err(|_| Corrupt)?),
            AttributionDto::Unattributed => Self::Unattributed,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DigestDto {
    algorithm: String,
    hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ProducerDto {
    Run {
        run_id: String,
        adapter_id: Option<String>,
        executable_approval_id: Option<u64>,
    },
    Unattributed {
        found_under_run_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransitionDto {
    state: String,
    at: TimestampDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProvenanceDto {
    scope_id: String,
    producer: ProducerDto,
    transitions: Vec<TransitionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderDto {
    adapter_id: String,
    locator: Option<String>,
    confirmation_kind: Option<String>,
    state: String,
    reason: Option<String>,
}

/// One Catalog record, as it is written to and read from
/// `.omnifrons/catalog.jsonl` and carried by a `published-local` journal
/// step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordDto {
    publication_id: String,
    catalog_id: String,
    asset_root_id: String,
    class: String,
    #[serde(rename = "type")]
    detected_type: String,
    size: u64,
    digest: DigestDto,
    names: Vec<String>,
    relationships: Vec<String>,
    provenance: ProvenanceDto,
    provider: ProviderDto,
    state: String,
    record_version: u32,
}

impl From<&CatalogRecord> for RecordDto {
    fn from(record: &CatalogRecord) -> Self {
        let producer = match &record.provenance.producer {
            Producer::Run {
                run_id,
                adapter_id,
                executable_approval,
            } => ProducerDto::Run {
                run_id: run_id.as_str().to_string(),
                adapter_id: adapter_id.as_ref().map(|id| id.as_str().to_string()),
                executable_approval_id: executable_approval.map(|id| id.0),
            },
            Producer::Unattributed { found_under } => ProducerDto::Unattributed {
                found_under_run_id: found_under.as_ref().map(|id| id.as_str().to_string()),
            },
        };
        Self {
            publication_id: record.publication_id.to_hex(),
            catalog_id: record.catalog_id.to_string(),
            asset_root_id: record.catalog_id.asset_root_id.as_str().to_string(),
            class: record.class.as_str().to_string(),
            detected_type: record.detected_type.as_str().to_string(),
            size: record.size,
            digest: DigestDto {
                algorithm: DIGEST_ALGORITHM_SHA256.to_string(),
                hex: record.digest.to_hex(),
            },
            names: record
                .names
                .iter()
                .map(|name| name.as_str().to_string())
                .collect(),
            relationships: record.relationships.clone(),
            provenance: ProvenanceDto {
                scope_id: record.provenance.scope_id.to_hex(),
                producer,
                transitions: record
                    .provenance
                    .transitions
                    .iter()
                    .map(|transition| TransitionDto {
                        state: transition.state.as_str().to_string(),
                        at: transition.at.into(),
                    })
                    .collect(),
            },
            provider: ProviderDto {
                adapter_id: record.provider.adapter_id.clone(),
                locator: record
                    .provider
                    .locator
                    .as_ref()
                    .map(|l| l.as_str().to_string()),
                confirmation_kind: record.provider.confirmation_kind.clone(),
                state: record.provider.state.as_str().to_string(),
                reason: record.provider.reason.clone(),
            },
            state: record.state.as_str().to_string(),
            record_version: record.record_version,
        }
    }
}

impl TryFrom<RecordDto> for CatalogRecord {
    type Error = Corrupt;

    fn try_from(dto: RecordDto) -> Result<Self, Corrupt> {
        let publication_id = PublicationIdentity::from_hex(&dto.publication_id).ok_or(Corrupt)?;
        let asset_root_id = AssetRootId::new(dto.asset_root_id).map_err(|_| Corrupt)?;
        let catalog_id = CatalogId {
            asset_root_id,
            publication_id,
        };
        if catalog_id.to_string() != dto.catalog_id {
            return Err(Corrupt);
        }
        if dto.digest.algorithm != DIGEST_ALGORITHM_SHA256 {
            return Err(Corrupt);
        }
        let producer = match dto.provenance.producer {
            ProducerDto::Run {
                run_id,
                adapter_id,
                executable_approval_id,
            } => Producer::Run {
                run_id: RunId::new(run_id).map_err(|_| Corrupt)?,
                adapter_id: match adapter_id {
                    Some(token) => Some(AdapterId::parse(&token).ok_or(Corrupt)?),
                    None => None,
                },
                executable_approval: executable_approval_id.map(ApprovalId),
            },
            ProducerDto::Unattributed { found_under_run_id } => Producer::Unattributed {
                found_under: match found_under_run_id {
                    Some(token) => Some(RunId::new(token).map_err(|_| Corrupt)?),
                    None => None,
                },
            },
        };
        let transitions = dto
            .provenance
            .transitions
            .into_iter()
            .map(|transition| {
                Ok(Transition {
                    state: ArtifactState::parse(&transition.state).ok_or(Corrupt)?,
                    at: transition.at.into(),
                })
            })
            .collect::<Result<Vec<_>, Corrupt>>()?;
        Ok(Self {
            publication_id,
            catalog_id,
            class: ArtifactClass::parse(&dto.class).ok_or(Corrupt)?,
            detected_type: DetectedType::parse(&dto.detected_type).ok_or(Corrupt)?,
            size: dto.size,
            digest: Sha256Digest::from_hex(&dto.digest.hex).ok_or(Corrupt)?,
            names: dto
                .names
                .iter()
                .map(|name| DisplayName::sanitize(name))
                .collect(),
            relationships: dto.relationships,
            provenance: Provenance {
                scope_id: ProjectIdentity::from_hex(&dto.provenance.scope_id).ok_or(Corrupt)?,
                producer,
                transitions,
            },
            provider: ProviderRecord {
                adapter_id: dto.provider.adapter_id,
                locator: dto.provider.locator.map(ProviderLocator::new),
                confirmation_kind: dto.provider.confirmation_kind,
                state: ProviderState::parse(&dto.provider.state).ok_or(Corrupt)?,
                reason: dto.provider.reason,
            },
            state: ArtifactState::parse(&dto.state).ok_or(Corrupt)?,
            record_version: dto.record_version,
        })
    }
}
