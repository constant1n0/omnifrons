//! The product work area (spike slice 5b, HAP-001-R7, D1): Omnifrons's
//! own device-local, owner-only directory holding the publication journal,
//! recovery entries, and -- as of spike slice 5c -- the guidance
//! installer's snapshots (HAP-001 D18) -- never inside a registered
//! workspace root, never declared to a harness. Canonicalized and checked
//! when configured, and re-canonicalized and re-checked at every use
//! ([`WorkAreaRoot::check`]), so a workspace registered over it afterwards
//! refuses the next journal, recovery, or snapshot write with
//! `work-area-invalid`.
//!
//! The vault HAP-001-R7 also names does not exist in this repository yet;
//! the check runs against the registered workspace roots the caller
//! passes (`docs/spike-log.md` § Slice 5b).

use std::path::{Path, PathBuf};

use crate::harness_adapter::WorkspaceRoot;

/// The journal's directory under the work area.
pub const JOURNAL_DIR: &str = "journal";

/// The recovery entries' directory under the work area.
pub const RECOVERY_DIR: &str = "recovery";

/// The guidance installer's snapshots directory under the work area
/// (spike slice 5c, HAP-001 D18): one subdirectory per project identity.
pub const SNAPSHOTS_DIR: &str = "snapshots";

/// The wrong-root ignore ledger's directory under the work area (spike
/// slice 5d, HAP-001 § Wrong-root detection and remedies): the one durable
/// fact a remedy leaves behind. `misplaced` itself is recomputed by every
/// scan and never persisted.
pub const IGNORE_DIR: &str = "ignore";

/// Why a work area could not be opened or fails its re-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkAreaError {
    /// The directory could not be created, canonicalized, or read.
    #[error("the product work area could not be used")]
    Unusable,
    /// The directory resolves inside (or is) a registered workspace root.
    #[error("the product work area resolves inside a registered workspace root")]
    InsideWorkspace,
    /// The directory's permissions could not be made owner-only.
    #[error("the product work area could not be made owner-only")]
    NotOwnerOnly,
}

/// Why a device-local directory fails the containment check every
/// authorized path runs before use (HAP-001-R14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentError {
    /// The path could not be canonicalized.
    Unusable,
    /// The canonical path is, or lies under, a registered workspace root.
    InsideWorkspace,
}

/// Canonicalize `path` and refuse it if it is, or lies under, any of
/// `workspaces` (HAP-001-R7, R14: "canonicalized before any authorization
/// that names them ... re-checked against every registered workspace
/// root").
///
/// # Errors
///
/// Returns [`ContainmentError`].
pub fn canonical_outside_workspaces(
    path: &Path,
    workspaces: &[&WorkspaceRoot],
) -> Result<PathBuf, ContainmentError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| ContainmentError::Unusable)?;
    if workspaces
        .iter()
        .any(|workspace| canonical.starts_with(workspace.path()))
    {
        return Err(ContainmentError::InsideWorkspace);
    }
    Ok(canonical)
}

/// Where a not-yet-existing `path` would canonicalize: its nearest
/// existing ancestor canonicalized, joined with the remaining components.
/// Used to refuse a path *before* creating it, so nothing is ever created
/// inside a workspace root only to be refused afterwards.
fn projected_canonical(path: &Path) -> Result<PathBuf, ContainmentError> {
    let mut existing = path.to_path_buf();
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if existing.exists() {
            break;
        }
        let Some(name) = existing.file_name() else {
            return Err(ContainmentError::Unusable);
        };
        remainder.push(name.to_os_string());
        if !existing.pop() {
            return Err(ContainmentError::Unusable);
        }
    }
    let mut canonical = std::fs::canonicalize(&existing).map_err(|_| ContainmentError::Unusable)?;
    for name in remainder.into_iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

/// Refuse `path` if its projected canonical form is, or lies under, any of
/// `workspaces`, without creating anything.
pub(crate) fn refuse_inside_workspaces(
    path: &Path,
    workspaces: &[&WorkspaceRoot],
) -> Result<(), ContainmentError> {
    let projected = projected_canonical(path)?;
    if workspaces
        .iter()
        .any(|workspace| projected.starts_with(workspace.path()))
    {
        return Err(ContainmentError::InsideWorkspace);
    }
    Ok(())
}

/// Create `dir` if missing and make it owner-only where the platform
/// expresses that (unix mode `0o700`; Windows ACLs are outside `std` and
/// disclosed in `docs/spike-log.md` § Slice 5b).
pub(crate) fn create_owner_only_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        // `DirBuilder::mode` is subject to the umask and does not touch a
        // directory that already existed: set the mode explicitly on the
        // result so it is owner-only either way.
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Whether `dir` is owner-only. Always true where the platform expresses
/// no such permission (the disclosed Windows residual).
#[cfg(unix)]
pub(crate) fn is_owner_only(dir: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(dir)?.permissions().mode();
    // No group or other bit set: the low six bits (0o077) are zero.
    Ok(mode.trailing_zeros() >= 6)
}

/// The `Result` is the platform seam shared with the unix version, which
/// can fail reading metadata; this one cannot, and wraps on purpose.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn is_owner_only(_dir: &Path) -> std::io::Result<bool> {
    Ok(true)
}

/// The opened, canonical product work area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkAreaRoot {
    path: PathBuf,
}

impl WorkAreaRoot {
    /// Open the work area configured at `configured`, creating it and its
    /// `journal/`, `recovery/`, `snapshots/`, and `ignore/` subdirectories
    /// (owner-only) when missing, after checking that it does not resolve
    /// inside any of `workspaces` (HAP-001-R7 at configuration time).
    /// Nothing is created when the check fails.
    ///
    /// # Errors
    ///
    /// Returns [`WorkAreaError::InsideWorkspace`] if the configured path
    /// resolves inside a registered workspace root,
    /// [`WorkAreaError::NotOwnerOnly`] if it could not be made owner-only,
    /// or [`WorkAreaError::Unusable`] if it could not be created or
    /// canonicalized.
    pub fn open(configured: &Path, workspaces: &[&WorkspaceRoot]) -> Result<Self, WorkAreaError> {
        refuse_inside_workspaces(configured, workspaces).map_err(WorkAreaError::from)?;
        create_owner_only_dir(configured).map_err(|_| WorkAreaError::Unusable)?;
        let path = canonical_outside_workspaces(configured, workspaces)?;
        for sub in [JOURNAL_DIR, RECOVERY_DIR, SNAPSHOTS_DIR, IGNORE_DIR] {
            create_owner_only_dir(&path.join(sub)).map_err(|_| WorkAreaError::Unusable)?;
        }
        let root = Self { path };
        for dir in [
            root.path.clone(),
            root.journal_dir(),
            root.recovery_dir(),
            root.snapshots_dir(),
            root.ignore_dir(),
        ] {
            if !is_owner_only(&dir).map_err(|_| WorkAreaError::Unusable)? {
                return Err(WorkAreaError::NotOwnerOnly);
            }
        }
        Ok(root)
    }

    /// Re-canonicalize and re-check the work area against `workspaces`
    /// (HAP-001-R7 at every use).
    ///
    /// # Errors
    ///
    /// Returns [`WorkAreaError::InsideWorkspace`] if the work area now
    /// resolves inside a registered workspace root, or
    /// [`WorkAreaError::Unusable`] if it can no longer be canonicalized.
    pub fn check(&self, workspaces: &[&WorkspaceRoot]) -> Result<(), WorkAreaError> {
        let canonical = canonical_outside_workspaces(&self.path, workspaces)?;
        if canonical != self.path {
            return Err(WorkAreaError::Unusable);
        }
        Ok(())
    }

    /// Check the work area configured at `configured` against `workspaces`
    /// without creating or opening anything (HAP-001-R7 on every workspace
    /// registration): its projected canonical form -- the nearest existing
    /// ancestor canonicalized, the rest appended -- must not be, or lie
    /// under, a registered workspace root, whether the area exists yet or
    /// not.
    ///
    /// # Errors
    ///
    /// Returns [`WorkAreaError::InsideWorkspace`] if the configured path
    /// resolves inside a registered workspace root, or
    /// [`WorkAreaError::Unusable`] if it cannot be projected at all.
    pub fn check_configured(
        configured: &Path,
        workspaces: &[&WorkspaceRoot],
    ) -> Result<(), WorkAreaError> {
        refuse_inside_workspaces(configured, workspaces).map_err(WorkAreaError::from)
    }

    /// The canonical work area path. Device configuration only: never a
    /// record, event, or IPC payload value (HAP-001-R5).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The journal's directory.
    #[must_use]
    pub fn journal_dir(&self) -> PathBuf {
        self.path.join(JOURNAL_DIR)
    }

    /// The recovery entries' directory.
    #[must_use]
    pub fn recovery_dir(&self) -> PathBuf {
        self.path.join(RECOVERY_DIR)
    }

    /// The guidance installer's snapshots directory (spike slice 5c).
    #[must_use]
    pub fn snapshots_dir(&self) -> PathBuf {
        self.path.join(SNAPSHOTS_DIR)
    }

    /// The wrong-root ignore ledger's directory (spike slice 5d).
    #[must_use]
    pub fn ignore_dir(&self) -> PathBuf {
        self.path.join(IGNORE_DIR)
    }
}

impl From<ContainmentError> for WorkAreaError {
    fn from(error: ContainmentError) -> Self {
        match error {
            ContainmentError::Unusable => Self::Unusable,
            ContainmentError::InsideWorkspace => Self::InsideWorkspace,
        }
    }
}
