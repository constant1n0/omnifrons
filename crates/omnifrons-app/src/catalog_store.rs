//! The `CatalogStore` port (spike slice 5b, HAP-001 § Catalog record):
//! the Context Catalog's records for one project, append-only -- a record
//! is registered once, and a later identical publication adds an alias to
//! it, never a second record (HAP-001-R23). Persisted by
//! `omnifrons-adapters`' `JsonlCatalogStore` under the project's
//! `.omnifrons/` namespace (D9).

use omnifrons_domain::publication::{CatalogRecord, DisplayName, PublicationIdentity};

/// Why a [`CatalogStore`] operation failed. Closed and exhaustive, never a
/// raw `io::Error` whose text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogStoreError {
    /// The store could not be read.
    #[error("the catalog could not be read")]
    Unreadable,
    /// The store's content is not a valid record log.
    #[error("the catalog's content is corrupt")]
    Corrupt,
    /// A write did not complete.
    #[error("the catalog could not be written")]
    WriteFailed,
    /// A record with this publication identity is already registered.
    #[error("a record with this publication identity is already registered")]
    Duplicate,
    /// No record with this publication identity exists (for an alias).
    #[error("no record with this publication identity exists")]
    Unknown,
}

/// A port over one project's Catalog records.
pub trait CatalogStore {
    /// Every record, in registration order, with its aliases applied.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogStoreError`] if the store cannot be read or is
    /// corrupt.
    fn list(&self) -> Result<Vec<CatalogRecord>, CatalogStoreError>;

    /// The record for `id`, if registered.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogStoreError`] if the store cannot be read or is
    /// corrupt.
    fn find(&self, id: &PublicationIdentity) -> Result<Option<CatalogRecord>, CatalogStoreError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|record| &record.publication_id == id))
    }

    /// Register `record`; refused if its publication identity is already
    /// registered.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogStoreError::Duplicate`] for an existing identity,
    /// or [`CatalogStoreError`] if the write did not complete.
    fn register(&mut self, record: CatalogRecord) -> Result<(), CatalogStoreError>;

    /// Add `name` as an alias of the record for `id` (HAP-001-R23); a name
    /// the record already carries is not duplicated.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogStoreError::Unknown`] if no record has `id`, or
    /// [`CatalogStoreError`] if the write did not complete.
    fn add_alias(
        &mut self,
        id: &PublicationIdentity,
        name: DisplayName,
    ) -> Result<(), CatalogStoreError>;
}
