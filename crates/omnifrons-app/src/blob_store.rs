//! The `BlobStorePort` (spike slice 5b, HAP-001 § Provider adapter
//! capabilities): the provider adapter every publication goes through,
//! reduced to what this slice needs -- declared capabilities, a staged
//! object the transaction streams the held handle's bytes into and then
//! commits or aborts, a read-back of the published copy for the digest
//! re-verification HAP-001-R21 requires, a discard for the copy that fails
//! it, and the confirmation observation that would yield
//! `provider-synced` (HAP-001-R25).
//!
//! The port deliberately hands the transaction a sink rather than taking
//! a handle: the copy and its digest are then one pass in the application
//! layer (`crate::publication`), the bytes leave the outbox exactly once,
//! from the held handle, and the adapter never sees a path (design rule 5).
//! Provider upload (step 9) is outside this slice; the port has no
//! `upload` yet.
//!
//! The device asset path (HAP-001 § Destination mapping, per device) is
//! validated here too: canonicalized and re-checked against every
//! registered workspace root at every publication (HAP-001-R14), refused
//! as `destination-invalid` otherwise.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use omnifrons_domain::publication::{ProviderLocator, PublicationIdentity};

use crate::harness_adapter::WorkspaceRoot;
use crate::work_area::{ContainmentError, canonical_outside_workspaces, create_owner_only_dir};

/// How the published bytes leave the outbox (HAP-001 § Provider adapter
/// capabilities). Only `copy` is implemented in this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishMode {
    /// Read from the held handle into the device asset path.
    Copy,
    /// A handle-anchored move the adapter declares; never a path-based
    /// rename. No adapter in this slice declares it.
    Move,
}

impl PublishMode {
    /// This mode's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

/// Whether remaining provider quota can be read through the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaVisibility {
    Visible,
    Unknown,
}

/// What a provider adapter declares, per adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// The adapter's id, as the Catalog record carries it.
    pub adapter_id: String,
    /// How the published bytes leave the outbox.
    pub publish_mode: PublishMode,
    /// The observation that yields `provider-synced` (D6), as the adapter
    /// names it.
    pub confirmation_kind: String,
    /// The largest object the provider accepts, or `None` for unknown.
    pub object_limit: Option<u64>,
    /// Whether quota can be read.
    pub quota_visibility: QuotaVisibility,
    /// Whether the adapter uses a platform placeholder API.
    pub native_placeholder: bool,
}

/// The adapter's declared confirmation, once observed (HAP-001-R25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfirmation {
    /// The confirmation kind observed.
    pub kind: String,
}

/// Why a provider operation failed. Closed: never a raw `io::Error` whose
/// text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BlobStoreError {
    /// The copy could not be staged, written, or committed.
    #[error("the published copy could not be written")]
    WriteFailed,
    /// The published copy could not be read back.
    #[error("the published copy could not be read")]
    ReadFailed,
    /// No object exists at the locator.
    #[error("no published copy exists at the locator")]
    NotFound,
    /// The locator is not one this adapter issued.
    #[error("the locator is not one this adapter issued")]
    ForeignLocator,
}

/// A staged object the transaction writes into, then commits or aborts.
/// Nothing is visible at the final locator before `commit`.
pub trait StagedObject: Write {
    /// Make the staged bytes the published copy and return its locator.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::WriteFailed`] if the copy could not be
    /// committed; the staged bytes are discarded in that case.
    fn commit(self: Box<Self>) -> Result<ProviderLocator, BlobStoreError>;

    /// Discard the staged bytes; nothing was published.
    fn abort(self: Box<Self>);
}

/// A port for one provider adapter's local materialization and read-back.
pub trait BlobStorePort {
    /// The adapter's declared capabilities.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Begin the published copy for `identity`.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::WriteFailed`] if nothing can be staged.
    fn stage(
        &self,
        identity: &PublicationIdentity,
    ) -> Result<Box<dyn StagedObject + '_>, BlobStoreError>;

    /// Read the published copy at `locator` back, for re-verification.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] if the copy cannot be opened.
    fn open_published(
        &self,
        locator: &ProviderLocator,
    ) -> Result<Box<dyn Read + '_>, BlobStoreError>;

    /// Discard the published copy at `locator` (a copy that failed
    /// re-verification, HAP-001-R21).
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] if the copy could not be removed.
    fn discard(&self, locator: &ProviderLocator) -> Result<(), BlobStoreError>;

    /// The adapter's declared confirmation for `locator`, if observed; an
    /// adapter with no remote never observes one.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError`] if the observation itself failed.
    fn confirm(
        &self,
        locator: &ProviderLocator,
    ) -> Result<Option<ProviderConfirmation>, BlobStoreError>;
}

/// Why a device asset path renders `destination-invalid` (HAP-001-R14) or
/// is not configured at all (HAP-001-R6, folded into the same wire code in
/// this slice; `docs/spike-log.md` § Slice 5b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DestinationError {
    /// The project's policy declares no asset root identity.
    #[error("the project declares no asset root")]
    Unconfigured,
    /// The path could not be created or canonicalized.
    #[error("the device asset path could not be used")]
    Unusable,
    /// The path resolves inside a registered workspace root.
    #[error("the device asset path resolves inside a registered workspace root")]
    InsideWorkspace,
}

impl From<ContainmentError> for DestinationError {
    fn from(error: ContainmentError) -> Self {
        match error {
            ContainmentError::Unusable => Self::Unusable,
            ContainmentError::InsideWorkspace => Self::InsideWorkspace,
        }
    }
}

/// A validated, canonical device asset path: where one asset root's
/// contents are materialized on this device. Device configuration only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAssetPath {
    path: PathBuf,
}

impl DeviceAssetPath {
    /// Open the device asset path configured at `configured`, creating it
    /// (owner-only) when missing, after refusing one that would resolve
    /// inside any of `workspaces` -- run at every publication, not only at
    /// configuration (HAP-001-R14). Nothing is created when the check
    /// fails.
    ///
    /// # Errors
    ///
    /// Returns [`DestinationError`].
    pub fn open(
        configured: &Path,
        workspaces: &[&WorkspaceRoot],
    ) -> Result<Self, DestinationError> {
        // Project the canonical form before creating anything: an existing
        // ancestor may be a link into a workspace.
        let existing_ancestor = nearest_existing_ancestor(configured)?;
        canonical_outside_workspaces(&existing_ancestor, workspaces)?;
        create_owner_only_dir(configured).map_err(|_| DestinationError::Unusable)?;
        let path = canonical_outside_workspaces(configured, workspaces)?;
        Ok(Self { path })
    }

    /// The canonical path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The nearest existing ancestor of `path` (or `path` itself).
fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf, DestinationError> {
    let mut candidate = path.to_path_buf();
    while !candidate.exists() {
        if !candidate.pop() {
            return Err(DestinationError::Unusable);
        }
    }
    Ok(candidate)
}
