//! `FsOutboxInventory`: the real, filesystem-backed
//! `omnifrons_app::OutboxInventory` (spike slice 5, HAP-001-R12): lists a
//! directory's entry *names* and probes each one through
//! [`FsCandidateProber`] -- so every open is relative to the held
//! directory handle on unix and never follows a link -- and descends into
//! a subdirectory only through a no-follow open relative to the same
//! handle.
//!
//! The names come from `std::fs::read_dir` on the directory's path (this
//! crate enables neither `nix`'s `dir` feature nor any other listing
//! API); a name is only ever a hint of what to open relative to the held
//! handle, so a directory swapped at the path after the handle was opened
//! can at most contribute names that then fail to open there
//! (`Unreadable`, excluded), never an open through the swapped directory.
//! Names are sorted so an inventory is deterministic.

use std::ffi::{OsStr, OsString};
use std::path::Path;

use omnifrons_app::run_outbox::{
    CandidateProber as _, DirectoryHandle, InventoriedEntry, InventoryError, OutboxInventory,
};

use crate::fs_candidate_prober::FsCandidateProber;
use crate::fs_run_outbox_preparer::open_child_directory_no_follow;

/// The real, OS-backed [`OutboxInventory`], probing through
/// [`FsCandidateProber`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsOutboxInventory {
    prober: FsCandidateProber,
}

impl FsOutboxInventory {
    /// Build an inventory.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prober: FsCandidateProber::new(),
        }
    }
}

impl OutboxInventory for FsOutboxInventory {
    fn inventory(
        &self,
        dir: &DirectoryHandle,
        dir_path: &Path,
    ) -> Result<Vec<InventoriedEntry>, InventoryError> {
        let mut names: Vec<OsString> = std::fs::read_dir(dir_path)
            .map_err(|_| InventoryError::Unreadable)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<Result<_, _>>()
            .map_err(|_| InventoryError::Unreadable)?;
        names.sort();
        Ok(names
            .into_iter()
            .map(|name| {
                let probe = self.prober.probe(dir, dir_path, &name);
                InventoriedEntry { name, probe }
            })
            .collect())
    }

    fn open_subdirectory(
        &self,
        dir: &DirectoryHandle,
        dir_path: &Path,
        name: &OsStr,
    ) -> Result<DirectoryHandle, InventoryError> {
        match open_child_directory_no_follow(dir.as_file(), dir_path, name) {
            Ok(file) => Ok(DirectoryHandle::new(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(InventoryError::Unreadable)
            }
            // `ELOOP` (a link at the name) and `ENOTDIR` (not a directory)
            // on unix, and the reparse-point/not-a-directory refusal on
            // Windows, all land here: the name is not a real directory.
            Err(_) => Err(InventoryError::NotADirectory),
        }
    }
}
