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

pub mod fs_prober;
pub mod jsonl_approval_store;
pub mod line_agent;
pub mod pty_cli;

pub use fs_prober::FsExecutableProber;
pub use jsonl_approval_store::JsonlApprovalStore;
pub use line_agent::LineAgent;
pub use omnifrons_app::harness_adapter::HarnessAdapter;
pub use pty_cli::PtyCli;

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
