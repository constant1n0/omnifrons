//! Adapters over `omnifrons-domain` and `omnifrons-app`'s ports.
//!
//! This crate is where the ports named in the target architecture's
//! component diagram get concrete implementations (docs/target-architecture.md
//! § Components and trust boundaries): `GitCoordinator`, `MemoryCoordinator`
//! (Engram), `KnowledgePort` (Markdown/Obsidian), `BlobStorePort`, and
//! `SecretStore`. None of them exist yet -- this crate is a placeholder
//! naming its future scope, not an implementation.
//!
//! `omnifrons-supervisor`, not this crate, implements the `ProcessSupervisor`
//! port, because that adapter needs Tokio (docs/repository-layout.md §
//! Crate map). This crate deliberately depends on `omnifrons-domain` and
//! `omnifrons-app` only, with no Tokio or Tauri dependency (verified by
//! `tests/deps.rs`), so that a future Git, Engram, Knowledge, or
//! secret-store adapter never has to fight an accidental async or
//! desktop-shell dependency creeping in from this crate.
