//! `JsonOutboxPolicyStore`: loads a project's classification policy from
//! `.omnifrons/asset-policy.json` (spike slice 5, HAP-001-R8, R40) as the
//! `omnifrons_app::OutboxPolicyStore` port. JSON with a `schema` integer,
//! the same discipline as the approval store: an absent file is the
//! shipped default policy; a file that is present but does not parse, has
//! any other `schema`, carries an unknown field or token, declares a raw
//! path, or contradicts a fixed rule fails the load rather than being
//! guessed at -- a synchronized policy is untrusted content and can
//! authorize nothing (HAP-001-R40).
//!
//! ```json
//! {
//!   "schema": 1,
//!   "outbox": ".omnifrons/outbox",
//!   "assetRootId": "main",
//!   "rows": [
//!     {"types": ["plain-text"], "extensions": ["csv"], "maxSize": 1048576, "class": "generated-heavy"}
//!   ]
//! }
//! ```
//!
//! `assetRootId` (spike slice 5b) names the asset root publications are
//! destined for: one `[A-Za-z0-9_-]{1,64}` token, never a path; a project
//! that declares none has no destination (HAP-001-R6).

use omnifrons_app::WorkspaceRoot;
use std::io::Read as _;

use omnifrons_app::outbox_policy::{
    OutboxPolicy, OutboxPolicyStore, POLICY_FILE_PATH, POLICY_MAX_BYTES, POLICY_SCHEMA_VERSION,
    PolicyError, PolicyRow,
};
use omnifrons_domain::outbox::{ArtifactClass, DetectedType, OutboxPath};
use omnifrons_domain::publication::AssetRootId;
use serde::Deserialize;

/// The policy file's shape. `deny_unknown_fields` on both levels: an
/// unknown key is corrupt, never silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PolicyFile {
    schema: u32,
    #[serde(default)]
    outbox: Option<String>,
    #[serde(default)]
    rows: Vec<RowFile>,
    /// The asset root publications are destined for (spike slice 5b): one
    /// token, never a path; absent leaves the project without a
    /// destination (HAP-001-R6).
    #[serde(default)]
    asset_root_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RowFile {
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    max_size: Option<u64>,
    class: String,
}

impl RowFile {
    fn into_row(self) -> Result<PolicyRow, PolicyError> {
        let types = self
            .types
            .iter()
            .map(|token| DetectedType::parse(token).ok_or(PolicyError::Corrupt))
            .collect::<Result<Vec<_>, _>>()?;
        let class = ArtifactClass::parse(&self.class).ok_or(PolicyError::Corrupt)?;
        Ok(PolicyRow {
            types,
            extensions: self
                .extensions
                .into_iter()
                .map(|extension| extension.to_ascii_lowercase())
                .collect(),
            max_size: self.max_size,
            class,
        })
    }
}

/// The JSON-backed [`OutboxPolicyStore`]. Stateless: every load re-reads
/// the file fresh.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonOutboxPolicyStore;

impl JsonOutboxPolicyStore {
    /// Build a store.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl OutboxPolicyStore for JsonOutboxPolicyStore {
    fn load(&self, project: &WorkspaceRoot) -> Result<OutboxPolicy, PolicyError> {
        let path = project.path().join(POLICY_FILE_PATH);
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(OutboxPolicy::default_policy());
            }
            Err(_) => return Err(PolicyError::Unreadable),
        };
        // Bounded before a byte is parsed: the size the handle reports, and
        // then the read itself capped one byte past the limit, so a file
        // that grows between the two is still refused rather than parsed.
        if file.metadata().map_err(|_| PolicyError::Unreadable)?.len() > POLICY_MAX_BYTES {
            return Err(PolicyError::TooLarge);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(POLICY_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PolicyError::Unreadable)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > POLICY_MAX_BYTES {
            return Err(PolicyError::TooLarge);
        }
        let text = String::from_utf8(bytes).map_err(|_| PolicyError::Corrupt)?;
        let file: PolicyFile = serde_json::from_str(&text).map_err(|_| PolicyError::Corrupt)?;
        if file.schema != POLICY_SCHEMA_VERSION {
            return Err(PolicyError::Corrupt);
        }
        let outbox = match file.outbox {
            Some(declared) => OutboxPath::new(declared).map_err(PolicyError::InvalidOutboxPath)?,
            None => OutboxPath::default_path(),
        };
        let rows = file
            .rows
            .into_iter()
            .map(RowFile::into_row)
            .collect::<Result<Vec<_>, _>>()?;
        let asset_root_id = match file.asset_root_id {
            Some(token) => Some(AssetRootId::new(token).map_err(|_| PolicyError::Corrupt)?),
            None => None,
        };
        OutboxPolicy::new(outbox, rows)
            .map(|policy| policy.with_asset_root(asset_root_id))
            .map_err(PolicyError::Invalid)
    }
}
