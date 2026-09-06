//! `JsonlApprovalStore`: an append-only `omnifrons_app::ApprovalStore`
//! backed by one `approvals.jsonl` file. A revocation is a new line
//! referencing an existing approval id, never an edit or rewrite of an
//! earlier line -- this is what makes the store's own history tamper-
//! evident and its growth-on-revoke behavior observable
//! (`crates/omnifrons-adapters/tests/jsonl_store.rs`).
//!
//! `list`/`find_active` reconstruct every approval's current status by
//! replaying the log in order: an `approved` line seeds a fresh, `Active`
//! record; a `revoked` line updates the matching record's status in place.
//! A line that fails to parse -- truncated, malformed, or otherwise not
//! valid JSON matching [`LogEntry`] -- fails the whole read as
//! [`ApprovalStoreError::Corrupt`]; there is no partial-read fallback that
//! silently drops the bad line and returns what could be recovered.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use omnifrons_app::approval_store::{
    ApprovalId, ApprovalRecord, ApprovalStore, ApprovalStoreError, DeviceLocalUser,
    ExecutableIdentity,
};
use omnifrons_domain::executable::{ApprovalStatus, PlatformEvidence, Sha256Digest};
use serde::{Deserialize, Serialize};

/// The backing file's name, always created directly under the directory
/// given to [`JsonlApprovalStore::open`].
const FILE_NAME: &str = "approvals.jsonl";

/// The only [`LogEntry::schema`]/[`LogEntry::Revoked`]'s schema value this
/// version of the store ever writes or accepts. Any other value found on
/// read fails the whole read as [`ApprovalStoreError::Corrupt`] (the
/// shell maps that to `approval-store-unavailable`) rather than guessing
/// at how to interpret a line shaped by some other schema generation
/// (R1-005).
const SCHEMA_VERSION: u32 = 1;

/// The domain separator mixed into [`derive_approval_id`]'s hash input, so
/// an approval id can never collide with a hash computed for an unrelated
/// purpose elsewhere in this codebase even if the same bytes happened to
/// be hashed.
const APPROVAL_ID_DOMAIN: &[u8] = b"omnifrons-approval-v1";

/// Derive an [`ApprovalId`] from `canonical_path`, `sha256`, and
/// `approved_at`, rather than counting existing records
/// (`docs/spike-log.md` § Slice 2: a counted `max() + 1` id, computed from
/// one handle's own `replay()` snapshot, can collide the moment two
/// concurrent instances -- or even two calls racing within one process --
/// both read the same snapshot before either writes). The id is the first
/// 8 bytes of `sha256(domain ++ canonical_path bytes ++ sha256 digest ++
/// approved_at nanoseconds-since-epoch)`, big-endian.
///
/// This does not, by itself, eliminate every race (two callers deriving
/// the same id and both attempting to append before either's write is
/// visible to the other could still collide) -- `record`'s own duplicate
/// check ([`ApprovalStoreError::Duplicate`]) is what actually refuses a
/// second write under an id already on file; there is still no
/// single-instance guard against two processes writing concurrently
/// (`docs/spike-log.md` § Slice 2's deferred list).
fn derive_approval_id(
    canonical_path: &std::path::Path,
    sha256: &Sha256Digest,
    approved_at: SystemTime,
) -> ApprovalId {
    use sha2::{Digest, Sha256};

    let nanos_since_epoch: u128 = approved_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();

    let mut hasher = Sha256::new();
    hasher.update(APPROVAL_ID_DOMAIN);
    hasher.update(canonical_path.to_string_lossy().as_bytes());
    hasher.update(sha256.0);
    hasher.update(nanos_since_epoch.to_be_bytes());
    let digest = hasher.finalize();

    let mut id_bytes = [0u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    ApprovalId(u64::from_be_bytes(id_bytes))
}

/// A wall-clock instant, as it crosses the JSONL boundary: split into
/// whole seconds and sub-second nanoseconds since the Unix epoch, rather
/// than a single wide integer no JSON number type can carry precisely.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct TimestampDto {
    secs: u64,
    nanos: u32,
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
        SystemTime::UNIX_EPOCH + Duration::new(dto.secs, dto.nanos)
    }
}

/// [`PlatformEvidence`], as it crosses the JSONL boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "os", rename_all = "kebab-case")]
enum PlatformDto {
    Unix { mode: u32 },
    Windows { extension: String, attributes: u32 },
}

impl From<&PlatformEvidence> for PlatformDto {
    fn from(evidence: &PlatformEvidence) -> Self {
        match evidence {
            PlatformEvidence::Unix { mode } => Self::Unix { mode: *mode },
            PlatformEvidence::Windows {
                extension,
                attributes,
            } => Self::Windows {
                extension: extension.clone(),
                attributes: *attributes,
            },
        }
    }
}

impl From<PlatformDto> for PlatformEvidence {
    fn from(dto: PlatformDto) -> Self {
        match dto {
            PlatformDto::Unix { mode } => Self::Unix { mode },
            PlatformDto::Windows {
                extension,
                attributes,
            } => Self::Windows {
                extension,
                attributes,
            },
        }
    }
}

/// One line of `approvals.jsonl`: either a fresh approval, or a revocation
/// of an existing one.
///
/// `approver` ([`DeviceLocalUser`]) is deliberately not a field here: it
/// is a zero-data marker, so persisting it would carry no information
/// beyond "this store records an approver" -- already implied by every
/// line being an `approved` event at all.
///
/// `schema` is present on every variant (rather than, say, one shared
/// leading line for the whole file) so each line stays fully
/// self-describing on its own -- consistent with this store's own
/// line-at-a-time replay and corrupt-line handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
enum LogEntry {
    Approved {
        schema: u32,
        approval_id: u64,
        canonical_path: String,
        size: u64,
        sha256: String,
        modified_at: Option<TimestampDto>,
        platform: PlatformDto,
        approved_at: TimestampDto,
    },
    Revoked {
        schema: u32,
        approval_id: u64,
        revoked_at: TimestampDto,
    },
}

impl LogEntry {
    /// This line's own `schema` value, regardless of variant.
    const fn schema(&self) -> u32 {
        match self {
            Self::Approved { schema, .. } | Self::Revoked { schema, .. } => *schema,
        }
    }
}

/// Decode a lowercase hex SHA-256 digest string back into its 32 raw
/// bytes, or `None` if `text` is not exactly 64 valid hex characters.
fn parse_sha256_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

fn identity_to_entry(
    identity: &ExecutableIdentity,
    approval_id: u64,
    approved_at: SystemTime,
) -> LogEntry {
    LogEntry::Approved {
        schema: SCHEMA_VERSION,
        approval_id,
        canonical_path: identity.canonical_path.to_string_lossy().into_owned(),
        size: identity.size,
        sha256: identity.sha256.to_hex(),
        modified_at: identity.modified_at.map(TimestampDto::from),
        platform: PlatformDto::from(&identity.platform),
        approved_at: approved_at.into(),
    }
}

/// An append-only, `approvals.jsonl`-backed [`ApprovalStore`].
///
/// Stateless beyond the path it was opened with: every call re-reads (or
/// appends to) the backing file fresh, so this type carries no in-memory
/// cache that could drift from what is actually on disk.
pub struct JsonlApprovalStore {
    path: PathBuf,
}

impl JsonlApprovalStore {
    /// Open (creating if absent) the `approvals.jsonl` file directly under
    /// `dir`. On unix, a freshly created file is given mode `0o600`
    /// (`OpenOptions::mode`) -- an already-existing file's mode is left
    /// untouched, matching normal `open()` semantics (the requested mode
    /// only applies when the call itself creates the file).
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError::WriteFailed`] if `dir` could not be
    /// created, or the backing file could not be opened/created within it.
    pub fn open(dir: PathBuf) -> Result<Self, ApprovalStoreError> {
        std::fs::create_dir_all(&dir).map_err(|_| ApprovalStoreError::WriteFailed)?;
        let mut path = dir;
        path.push(FILE_NAME);

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options
            .open(&path)
            .map_err(|_| ApprovalStoreError::WriteFailed)?;

        Ok(Self { path })
    }

    /// Append one JSON line (plus a trailing newline) to the backing file,
    /// then `sync_data` before returning `Ok` (R3-007): without this, a
    /// revocation (or a fresh approval) could sit unflushed in the OS page
    /// cache and be lost across a crash or power loss, silently
    /// resurrecting a revoked approval -- or forgetting one entirely -- on
    /// the next start, even though the caller was already told the write
    /// succeeded.
    fn append_line(&self, entry: &LogEntry) -> Result<(), ApprovalStoreError> {
        let line = serde_json::to_string(entry).map_err(|_| ApprovalStoreError::WriteFailed)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|_| ApprovalStoreError::WriteFailed)?;
        writeln!(file, "{line}").map_err(|_| ApprovalStoreError::WriteFailed)?;
        file.sync_data()
            .map_err(|_| ApprovalStoreError::WriteFailed)?;
        Ok(())
    }

    /// Replay every line of the backing file into ordered
    /// [`ApprovalRecord`]s. A blank line is skipped (defensive against a
    /// stray trailing newline); any other line that does not parse as a
    /// [`LogEntry`] fails the whole read as
    /// [`ApprovalStoreError::Corrupt`] -- never a partial result silently
    /// missing the bad line.
    fn replay(&self) -> Result<Vec<ApprovalRecord>, ApprovalStoreError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|_| ApprovalStoreError::Unreadable)?;

        let mut order: Vec<u64> = Vec::new();
        let mut records: std::collections::HashMap<u64, ApprovalRecord> =
            std::collections::HashMap::new();

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LogEntry =
                serde_json::from_str(line).map_err(|_| ApprovalStoreError::Corrupt)?;
            if entry.schema() != SCHEMA_VERSION {
                return Err(ApprovalStoreError::Corrupt);
            }
            match entry {
                LogEntry::Approved {
                    schema: _,
                    approval_id,
                    canonical_path,
                    size,
                    sha256,
                    modified_at,
                    platform,
                    approved_at,
                } => {
                    let sha256 = parse_sha256_hex(&sha256).ok_or(ApprovalStoreError::Corrupt)?;
                    let record = ApprovalRecord {
                        approval_id: ApprovalId(approval_id),
                        identity: ExecutableIdentity {
                            canonical_path: PathBuf::from(canonical_path),
                            size,
                            sha256: Sha256Digest(sha256),
                            modified_at: modified_at.map(SystemTime::from),
                            platform: platform.into(),
                        },
                        approved_at: approved_at.into(),
                        approver: DeviceLocalUser,
                        status: ApprovalStatus::Active,
                    };
                    if !records.contains_key(&approval_id) {
                        order.push(approval_id);
                    }
                    records.insert(approval_id, record);
                }
                LogEntry::Revoked {
                    schema: _,
                    approval_id,
                    revoked_at,
                } => {
                    // A revocation for an id not (yet, or ever) recorded is
                    // a no-op when replaying, mirroring `ApprovalStore::
                    // revoke`'s own no-op-for-unknown-id contract.
                    if let Some(record) = records.get_mut(&approval_id) {
                        record.status = ApprovalStatus::Revoked {
                            revoked_at: revoked_at.into(),
                        };
                    }
                }
            }
        }

        Ok(order
            .into_iter()
            .map(|id| {
                records
                    .remove(&id)
                    .expect("every id pushed to order was just inserted into records")
            })
            .collect())
    }
}

impl ApprovalStore for JsonlApprovalStore {
    fn list(&self) -> Result<Vec<ApprovalRecord>, ApprovalStoreError> {
        self.replay()
    }

    fn record(
        &mut self,
        identity: ExecutableIdentity,
        approver: DeviceLocalUser,
        approved_at: SystemTime,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        let approval_id =
            derive_approval_id(&identity.canonical_path, &identity.sha256, approved_at);

        // A derived id is not counted, so it cannot silently collide with
        // a concurrently-issued id the way a `max() + 1` counter can --
        // but a collision is still possible in principle (the same
        // identity approved twice at the exact same nanosecond, or a hash
        // collision), so this checks explicitly rather than assuming the
        // derivation alone rules it out.
        let existing = self.replay()?;
        if existing
            .iter()
            .any(|record| record.approval_id == approval_id)
        {
            return Err(ApprovalStoreError::Duplicate);
        }

        self.append_line(&identity_to_entry(&identity, approval_id.0, approved_at))?;

        Ok(ApprovalRecord {
            approval_id,
            identity,
            approved_at,
            approver,
            status: ApprovalStatus::Active,
        })
    }

    fn revoke(&mut self, id: ApprovalId, at: SystemTime) -> Result<(), ApprovalStoreError> {
        self.append_line(&LogEntry::Revoked {
            schema: SCHEMA_VERSION,
            approval_id: id.0,
            revoked_at: at.into(),
        })
    }

    fn find_active(
        &self,
        identity: &ExecutableIdentity,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError> {
        Ok(self
            .replay()?
            .into_iter()
            .rev()
            .find(|record| &record.identity == identity && record.status == ApprovalStatus::Active))
    }
}
