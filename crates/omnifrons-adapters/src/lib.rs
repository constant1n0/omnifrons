//! Adapters over `omnifrons-domain` and `omnifrons-app`'s ports.
//!
//! This crate is where the ports named in the target architecture's
//! component diagram get concrete implementations (docs/target-architecture.md
//! § Components and trust boundaries): `GitCoordinator`, `MemoryCoordinator`
//! (Engram), `KnowledgePort` (Markdown/Obsidian), and `SecretStore` remain
//! unimplemented placeholders. As of the spike slice-2 spike, this crate
//! also implements the executable-identity ports: [`FsExecutableProber`]
//! (`omnifrons_app::ExecutableProber`) and [`JsonlApprovalStore`]
//! (`omnifrons_app::ApprovalStore`), which is why this crate now depends on
//! `sha2` (content hashing), `serde`/`serde_json` (the JSONL record
//! shape), and, unix only, `nix` (`O_NOFOLLOW` on open, and Linux's
//! `memfd`/`fcntl` sealing path) in addition to `omnifrons-domain` and
//! `omnifrons-app`. `FsExecutableProber` no longer exposes a separate
//! `open_for_exec`: the handle a caller launches is the same one
//! [`FsExecutableProber::probe`] itself opened and hashed from
//! (`omnifrons_app::ProbedExecutable`), never a second, independent open
//! by path.
//!
//! `omnifrons-supervisor`, not this crate, implements the `ProcessSupervisor`
//! port, because that adapter needs Tokio (docs/repository-layout.md §
//! Crate map). This crate deliberately depends on no Tokio or Tauri
//! dependency (verified by `tests/deps.rs`), so that a future Git, Engram,
//! Knowledge, or secret-store adapter never has to fight an accidental
//! async or desktop-shell dependency creeping in from this crate.
//!
//! As of spike slice 5 (HAP-001's launch side) this crate also implements
//! the outbox ports over the same dependency set: [`FsRunOutboxPreparer`]
//! (`omnifrons_app::RunOutboxPreparer`), [`FsCandidateProber`]
//! (`omnifrons_app::CandidateProber`, sharing the prober's open-once
//! discipline), [`FsOutboxInventory`] (`omnifrons_app::OutboxInventory`),
//! and [`JsonOutboxPolicyStore`] (`omnifrons_app::OutboxPolicyStore`).
//! The unix `openat`/`mkdirat`/`O_NOFOLLOW`/`O_DIRECTORY` calls they use
//! are all gated by `nix`'s `fs` feature this crate already enables
//! (verified against the vendored 0.31.3 source); no feature and no
//! dependency was added.
//!
//! As of spike slice 5b (HAP-001's publication side) this crate also
//! implements the publication ports over the same dependency set:
//! [`Sha2Hasher`] (`omnifrons_app::content_hasher::ContentHasher`),
//! [`LocalDirBlobStore`] (`omnifrons_app::blob_store::BlobStorePort`; the
//! dev-mode local-directory provider, the only adapter in this slice),
//! [`JsonlCatalogStore`] (`omnifrons_app::catalog_store::CatalogStore`,
//! `.omnifrons/catalog.jsonl`), [`JsonlPublicationJournal`]
//! (`omnifrons_app::publication_journal::PublicationJournal`, the work
//! area's `journal/publications.jsonl`), and [`FsOutboxEntryOps`]
//! (`omnifrons_app::outbox_entry_ops::OutboxEntryOps`; `fstat`/`fstatat`/
//! `unlinkat`, all under the `fs` feature). Dependency tables unchanged.
//!
//! As of spike slice 5c (HAP-001 D18's guidance-note installer) this crate
//! also implements the two installer ports over the same dependency set:
//! [`FsProjectTextFile`] (`omnifrons_app::managed_file::ProjectTextFile`;
//! one `O_NOFOLLOW` open, a sibling `.part` file renamed over the target)
//! and [`FsSnapshotStore`] (`omnifrons_app::snapshot_store::SnapshotStore`;
//! the work area's `snapshots/<project hex>/<id>.json` and `<id>.bytes`).
//! Dependency tables unchanged.

mod catalog_record_dto;
pub mod fs_candidate_prober;
pub mod fs_outbox_entry_ops;
pub mod fs_outbox_inventory;
pub mod fs_prober;
pub mod fs_project_text_file;
pub mod fs_quarantine_store;
pub mod fs_recovery_entries;
pub mod fs_run_outbox_preparer;
pub mod fs_snapshot_store;
pub mod fs_wrong_root_scanner;
pub mod json_outbox_policy_store;
pub mod jsonl_approval_store;
pub mod jsonl_catalog_store;
pub mod jsonl_ignore_ledger;
pub mod jsonl_publication_journal;
pub mod line_agent;
pub mod local_dir_blob_store;
pub mod pty_cli;
pub mod sha2_hasher;

pub use fs_candidate_prober::FsCandidateProber;
pub use fs_outbox_entry_ops::FsOutboxEntryOps;
pub use fs_outbox_inventory::FsOutboxInventory;
pub use fs_prober::FsExecutableProber;
pub use fs_project_text_file::FsProjectTextFile;
pub use fs_quarantine_store::FsQuarantineStore;
pub use fs_recovery_entries::open_recovery_dir;
pub use fs_run_outbox_preparer::FsRunOutboxPreparer;
pub use fs_snapshot_store::FsSnapshotStore;
pub use fs_wrong_root_scanner::FsWrongRootScanner;
pub use json_outbox_policy_store::JsonOutboxPolicyStore;
pub use jsonl_approval_store::JsonlApprovalStore;
pub use jsonl_catalog_store::JsonlCatalogStore;
pub use jsonl_ignore_ledger::JsonlIgnoreLedger;
pub use jsonl_publication_journal::JsonlPublicationJournal;
pub use line_agent::LineAgent;
pub use local_dir_blob_store::LocalDirBlobStore;
pub use omnifrons_app::harness_adapter::HarnessAdapter;
pub use pty_cli::PtyCli;
pub use sha2_hasher::Sha2Hasher;

/// Every built-in adapter this crate provides: the two `stream-json`-shaped
/// line agents (`line_agent::catalog`, spike slice 3) plus the
/// pseudo-terminal fallback (`PtyCli`, spike slice 4). The shell builds its
/// closed `AdapterCatalog` from exactly this list.
#[must_use]
pub fn catalog() -> Vec<Box<dyn HarnessAdapter + Send + Sync>> {
    let mut adapters = line_agent::catalog();
    adapters.push(Box::new(PtyCli::new()));
    adapters
}
