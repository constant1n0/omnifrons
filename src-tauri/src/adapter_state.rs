//! Managed state for the built-in harness adapter surface
//! (`docs/spike-log.md` § Slice 3): the closed, built-in `AdapterCatalog`,
//! and the single active workspace a caller has picked, if any.

use std::sync::Mutex;

use omnifrons_app::{AdapterCatalog, WorkspaceRoot};

/// This shell's managed state for the built-in adapter surface.
///
/// `active_workspace` is a single slot, not a set: this slice supports
/// exactly one active workspace at a time, replaced wholesale by a fresh
/// `workspace_pick` (`docs/spike-log.md` § Slice 3).
pub struct AdapterState {
    pub catalog: AdapterCatalog,
    pub active_workspace: Mutex<Option<WorkspaceRoot>>,
}

impl AdapterState {
    /// Build the managed state over the closed, built-in adapter catalog,
    /// with no active workspace yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            catalog: AdapterCatalog::new(omnifrons_adapters::line_agent::catalog()),
            active_workspace: Mutex::new(None),
        }
    }
}

impl Default for AdapterState {
    fn default() -> Self {
        Self::new()
    }
}
