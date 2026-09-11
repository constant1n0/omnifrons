//! The product work area's `recovery/` directory as something a
//! recovery entry can be *probed relative to* (spike slice 5e,
//! HAP-001-R18).
//!
//! A recovery entry is reached by digest: its file name under
//! `recovery/` is the SHA-256 of the bytes the publication held when its
//! outbox path stopped naming them. That is a locator, not an identity
//! check -- a name says nothing about what currently answers to it -- so
//! re-approving one goes through the same single-handle discipline an
//! outbox entry does (`FsCandidateProber`), which needs the directory
//! handle to open relative to. This module is the one seam that hands it
//! over, using the same no-follow directory open the outbox uses.

use std::path::PathBuf;

use omnifrons_app::run_outbox::DirectoryHandle;
use omnifrons_app::work_area::WorkAreaRoot;

use crate::fs_run_outbox_preparer::open_directory_no_follow;

/// Open `work_area`'s `recovery/` directory without following a link at
/// its name, with the path beside it for the platforms that have no
/// relative open.
///
/// # Errors
///
/// Returns any `io::Error` the open produces, including the refusal of a
/// name that is not a real directory.
pub fn open_recovery_dir(work_area: &WorkAreaRoot) -> std::io::Result<(DirectoryHandle, PathBuf)> {
    let path = work_area.recovery_dir();
    let file = open_directory_no_follow(&path)?;
    Ok((DirectoryHandle::new(file), path))
}
