//! `LocalDirBlobStore`: the dev-mode local-directory provider adapter
//! (spike slice 5b), the only `omnifrons_app::blob_store::BlobStorePort`
//! implementation in this slice. It materializes an asset root's contents
//! in a validated device asset path -- a directory outside every
//! registered workspace root (`omnifrons_app::blob_store::DeviceAssetPath`,
//! HAP-001-R14) -- and has **no remote**: `confirm` never yields a
//! confirmation, so a publication through it is `registered` with
//! `provider_state: pending` and never `provider-synced` (HAP-001-R25).
//! Credentials: none; HAP-001-R34's custody rule does not apply to it.
//!
//! Capabilities (HAP-001 § Provider adapter capabilities): `publish_mode:
//! copy` only, `confirmation_kind: "no remote"`, no object limit, quota
//! unknown, no native placeholder. The locator is
//! `local-dir:<asset root id>/<publication hex>`; the copy lives at
//! `<device asset path>/<publication hex>`, written to a per-call staging
//! name (`.<publication hex>.<pid>-<sequence>.part`, so two stages of one
//! identity never share a file) and renamed into place on commit -- a
//! second commit of the same identity replaces the copy, which is what a
//! retry after an interruption needs -- owner-only where the platform
//! expresses that. A staging file left by a crash between `stage` and
//! `commit` is not removed by this adapter (disclosed; a sweep is slice-5c
//! debt). A locator this adapter did not issue -- another adapter's,
//! another asset root's, or anything that is not exactly 64 hex characters
//! after the prefix -- is refused as foreign, never resolved to a path.
//!
//! Dev-mode only: this adapter exists so the transaction, the Catalog, the
//! journal, and the recovery path can be exercised end to end before a
//! real provider adapter (the Proton Drive CLI, slice 5c) exists.

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::blob_store::{
    BlobStoreError, BlobStorePort, DeviceAssetPath, ProviderCapabilities, ProviderConfirmation,
    PublishMode, QuotaVisibility, StagedObject,
};
use omnifrons_domain::publication::{AssetRootId, ProviderLocator, PublicationIdentity};

/// The local-directory provider.
#[derive(Debug, Clone)]
pub struct LocalDirBlobStore {
    root: PathBuf,
    asset_root_id: AssetRootId,
}

impl LocalDirBlobStore {
    /// This adapter's id, as the Catalog record carries it.
    pub const ADAPTER_ID: &'static str = "local-dir";

    /// The adapter's own reason token for its permanent `pending`.
    pub const NO_REMOTE: &'static str = "no remote";

    /// A provider over the already-validated `device_path` for
    /// `asset_root_id`.
    #[must_use]
    pub fn open(device_path: &DeviceAssetPath, asset_root_id: AssetRootId) -> Self {
        Self {
            root: device_path.path().to_path_buf(),
            asset_root_id,
        }
    }

    fn locator_prefix(&self) -> String {
        format!("{}:{}/", Self::ADAPTER_ID, self.asset_root_id)
    }

    /// The publication a locator of this adapter names, or `None` for a
    /// foreign one.
    fn parse_locator(&self, locator: &ProviderLocator) -> Option<PublicationIdentity> {
        let hex = locator.as_str().strip_prefix(&self.locator_prefix())?;
        PublicationIdentity::from_hex(hex)
    }

    fn final_path(&self, identity: &PublicationIdentity) -> PathBuf {
        self.root.join(identity.to_hex())
    }

    /// A staging name unique to this call -- the publication hex, this
    /// process's id, and a process-wide sequence -- so two stages of the
    /// same identity (a second shell; one shell serializes its own
    /// publications) never write into one file (R1-003).
    fn staging_path(&self, identity: &PublicationIdentity) -> PathBuf {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.root.join(format!(
            ".{}.{}-{sequence}.part",
            identity.to_hex(),
            std::process::id()
        ))
    }
}

/// The process-wide staging sequence behind [`LocalDirBlobStore::staging_path`].
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A copy being written: a staging file that becomes the published copy
/// on commit and is removed on abort.
struct LocalStaged {
    file: File,
    staging: PathBuf,
    final_path: PathBuf,
    locator: String,
}

impl Write for LocalStaged {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl StagedObject for LocalStaged {
    fn commit(self: Box<Self>) -> Result<ProviderLocator, BlobStoreError> {
        let committed = self
            .file
            .sync_data()
            .and_then(|()| std::fs::rename(&self.staging, &self.final_path));
        if committed.is_err() {
            let _ = std::fs::remove_file(&self.staging);
            return Err(BlobStoreError::WriteFailed);
        }
        Ok(ProviderLocator::new(self.locator))
    }

    fn abort(self: Box<Self>) {
        let _ = std::fs::remove_file(&self.staging);
    }
}

impl BlobStorePort for LocalDirBlobStore {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            adapter_id: Self::ADAPTER_ID.to_string(),
            publish_mode: PublishMode::Copy,
            confirmation_kind: Self::NO_REMOTE.to_string(),
            object_limit: None,
            quota_visibility: QuotaVisibility::Unknown,
            native_placeholder: false,
        }
    }

    fn stage(
        &self,
        identity: &PublicationIdentity,
    ) -> Result<Box<dyn StagedObject + '_>, BlobStoreError> {
        let staging = self.staging_path(identity);
        let mut options = std::fs::OpenOptions::new();
        // The name is this call's own; a file already there is not ours.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options
            .open(&staging)
            .map_err(|_| BlobStoreError::WriteFailed)?;
        Ok(Box::new(LocalStaged {
            file,
            staging,
            final_path: self.final_path(identity),
            locator: format!("{}{}", self.locator_prefix(), identity.to_hex()),
        }))
    }

    fn open_published(
        &self,
        locator: &ProviderLocator,
    ) -> Result<Box<dyn Read + '_>, BlobStoreError> {
        let identity = self
            .parse_locator(locator)
            .ok_or(BlobStoreError::ForeignLocator)?;
        match File::open(self.final_path(&identity)) {
            Ok(file) => Ok(Box::new(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(BlobStoreError::NotFound)
            }
            Err(_) => Err(BlobStoreError::ReadFailed),
        }
    }

    fn discard(&self, locator: &ProviderLocator) -> Result<(), BlobStoreError> {
        let identity = self
            .parse_locator(locator)
            .ok_or(BlobStoreError::ForeignLocator)?;
        match std::fs::remove_file(self.final_path(&identity)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(BlobStoreError::NotFound)
            }
            Err(_) => Err(BlobStoreError::WriteFailed),
        }
    }

    fn confirm(
        &self,
        _locator: &ProviderLocator,
    ) -> Result<Option<ProviderConfirmation>, BlobStoreError> {
        // No remote: nothing can ever confirm the bytes elsewhere.
        Ok(None)
    }
}
