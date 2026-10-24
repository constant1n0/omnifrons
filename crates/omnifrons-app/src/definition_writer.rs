//! The `DefinitionWriter` port (ADR-0005 decision 3, sub-slice 2c): the
//! one operation that puts a planned file's bytes on disk. The apply
//! service (sub-slice 2d) drives it once per planned write; the port
//! itself knows nothing about plans.
//!
//! A write binds to what the plan saw. The caller names the file by a
//! workspace-relative path of plain components and the
//! [`InputFingerprint`] the plan recorded for it, and the implementation
//! re-checks that fingerprint immediately before publishing:
//!
//! - [`InputFingerprint::Absent`]: an exclusive create. Anything that
//!   appeared at the name since planning refuses the write and is left
//!   untouched.
//! - [`InputFingerprint::Sha256`]: the current regular file must still
//!   hash to that digest, else [`DefinitionWriteError::Stale`].
//! - [`InputFingerprint::Symlink`] and [`InputFingerprint::Irregular`]:
//!   never writable ([`DefinitionWriteError::NotWritable`]).
//!
//! A write never goes through a symbolic link: a target or a parent
//! directory that is a link, or a target that is not a regular file,
//! refuses ([`DefinitionWriteError::NotRegular`]). The file is published
//! atomically (a sibling temporary file, flushed, then linked into place
//! for a create or renamed over the old file for a replace), the result
//! is read back, and the digest of what was read is returned, so a
//! caller learns the bytes on disk are the bytes it planned.
//!
//! # Permission bits
//!
//! [`ModePolicy`] names the rule, mirroring the prototype's: a replaced
//! file keeps its own bits; a file that is created takes the bits of a
//! named source file in the same workspace and never more, and a copy
//! over an existing file keeps the narrower of the two. Platforms that
//! do not express permission bits ignore the policy.

use omnifrons_domain::definition_plan::InputFingerprint;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::nesting;

use crate::harness_adapter::WorkspaceRoot;

/// Which permission bits a write publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModePolicy {
    /// A replaced file keeps its own bits; a created file gets the
    /// platform default.
    Keep,
    /// A created file takes the bits of the regular file `source`
    /// (workspace-relative); a replaced file keeps the bits it shares
    /// with `source`, so the write never widens access to the source's
    /// text.
    NoWiderThan {
        /// The workspace-relative path of the source file.
        source: String,
    },
}

/// One write: where, what, and what the plan saw there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest<'a> {
    /// The workspace-relative path of the file, `/`-separated plain
    /// components (see [`validate_write_path`]).
    pub path: &'a str,
    /// The bytes to publish, written untouched.
    pub bytes: &'a [u8],
    /// What the plan recorded at `path`.
    pub expected: &'a InputFingerprint,
    /// The permission rule.
    pub mode: &'a ModePolicy,
}

/// Why a write was refused or failed. Every variant has a fixed
/// [`Self::reason`] token; no message carries an absolute path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionWriteError {
    /// `path` is not a clean workspace-relative path of plain
    /// components.
    #[error("the path is not a plain workspace-relative path: {path}")]
    InvalidPath {
        /// The offending path as given.
        path: String,
    },
    /// The plan recorded a symlink or an irregular file at `path`, which
    /// a write never replaces.
    #[error("the planned input at {path} is not a writable file")]
    NotWritable {
        /// The workspace-relative path.
        path: String,
    },
    /// The target, or a parent directory of it, is a symbolic link or
    /// not a regular file or directory.
    #[error("{path} is not a regular file in real directories")]
    NotRegular {
        /// The workspace-relative path.
        path: String,
    },
    /// A parent directory of `path` does not exist.
    #[error("a parent directory of {path} does not exist")]
    ParentMissing {
        /// The workspace-relative path.
        path: String,
    },
    /// What is at `path` is no longer what the plan recorded.
    #[error("{path} changed after planning")]
    Stale {
        /// The workspace-relative path.
        path: String,
        /// What the plan recorded.
        expected: InputFingerprint,
        /// What was found instead.
        found: InputFingerprint,
    },
    /// The [`ModePolicy`] source is missing or not a regular file.
    #[error("the permission source {path} is not a regular file")]
    SourceUnavailable {
        /// The workspace-relative path of the source.
        path: String,
    },
    /// The bytes read back after publishing differ from the bytes
    /// written.
    #[error("{path} does not hold the written bytes after publishing")]
    ReadBackMismatch {
        /// The workspace-relative path.
        path: String,
    },
    /// The write could not be completed; nothing was published and no
    /// temporary file remains.
    #[error("the write to {path} failed")]
    Io {
        /// The workspace-relative path.
        path: String,
    },
}

impl DefinitionWriteError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::InvalidPath { .. } => "invalid-path",
            Self::NotWritable { .. } => "not-writable",
            Self::NotRegular { .. } => "not-regular",
            Self::ParentMissing { .. } => "parent-missing",
            Self::Stale { .. } => "stale",
            Self::SourceUnavailable { .. } => "source-unavailable",
            Self::ReadBackMismatch { .. } => "read-back-mismatch",
            Self::Io { .. } => "io",
        }
    }
}

/// A port over writing one planned file atomically.
pub trait DefinitionWriter {
    /// Publish `request.bytes` at `request.path` under `root`, after
    /// re-checking `request.expected` immediately before publishing, and
    /// return the digest of the bytes read back.
    ///
    /// # Errors
    ///
    /// [`DefinitionWriteError::InvalidPath`] before touching anything;
    /// [`DefinitionWriteError::NotWritable`] for a recorded symlink or
    /// irregular file; [`DefinitionWriteError::NotRegular`] and
    /// [`DefinitionWriteError::ParentMissing`] for a bad target or
    /// parent; [`DefinitionWriteError::Stale`] when the target no longer
    /// matches the plan; [`DefinitionWriteError::SourceUnavailable`] for
    /// an unusable permission source;
    /// [`DefinitionWriteError::ReadBackMismatch`]; or
    /// [`DefinitionWriteError::Io`]. Every refusal leaves the target
    /// exactly as it was and no temporary file behind (a read-back
    /// mismatch happens after publishing and reports what is on disk).
    fn write(
        &self,
        root: &WorkspaceRoot,
        request: &WriteRequest<'_>,
    ) -> Result<Sha256Digest, DefinitionWriteError>;
}

/// The plain components of a write path, or
/// [`DefinitionWriteError::InvalidPath`].
///
/// The rules are the plan's own ([`nesting::relate`]: relative, no `..`,
/// `.`, empty, doubled or trailing-slash components), plus: the path
/// names a file, so `"."` alone is refused; a NUL is refused everywhere;
/// and on Windows alone a backslash or a colon is refused, since there it
/// could read as more than one component or a stream name. On Unix both
/// are ordinary name characters that the scan lists and the plan
/// accepts, so a write must accept them too.
///
/// # Errors
///
/// [`DefinitionWriteError::InvalidPath`] naming `path`.
pub fn validate_write_path(path: &str) -> Result<Vec<&str>, DefinitionWriteError> {
    let invalid = || DefinitionWriteError::InvalidPath {
        path: path.to_string(),
    };
    if path == "." || path.contains('\0') || (cfg!(windows) && path.contains(['\\', ':'])) {
        return Err(invalid());
    }
    nesting::relate(&[path], &[]).map_err(|_| invalid())?;
    Ok(path.split('/').collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_split_into_components() {
        assert_eq!(
            validate_write_path("AGENTS.md").expect("plain"),
            ["AGENTS.md"]
        );
        assert_eq!(
            validate_write_path("a/b/c.md").expect("plain"),
            ["a", "b", "c.md"]
        );
    }

    #[test]
    fn anything_else_is_invalid() {
        for path in [
            "", ".", "..", "/abs", "../x", "a/../b", "./a", "a//b", "a/", "a/./b", "a\0b",
        ] {
            let error = validate_write_path(path).expect_err(path);
            assert_eq!(error.reason(), "invalid-path", "{path:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_backslash_or_colon_is_invalid_on_windows() {
        for path in ["a\\b", "a:b", "a/b:c"] {
            assert_eq!(
                validate_write_path(path).expect_err(path).reason(),
                "invalid-path"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_backslash_or_colon_is_an_ordinary_name_character_on_unix() {
        assert_eq!(validate_write_path("a\\b").expect("plain"), ["a\\b"]);
        assert_eq!(validate_write_path("d/a:b").expect("plain"), ["d", "a:b"]);
    }

    #[test]
    fn every_error_has_a_distinct_reason() {
        let path = || "p".to_string();
        let reasons = [
            DefinitionWriteError::InvalidPath { path: path() },
            DefinitionWriteError::NotWritable { path: path() },
            DefinitionWriteError::NotRegular { path: path() },
            DefinitionWriteError::ParentMissing { path: path() },
            DefinitionWriteError::Stale {
                path: path(),
                expected: InputFingerprint::Absent,
                found: InputFingerprint::Irregular,
            },
            DefinitionWriteError::SourceUnavailable { path: path() },
            DefinitionWriteError::ReadBackMismatch { path: path() },
            DefinitionWriteError::Io { path: path() },
        ]
        .map(|error| error.reason());
        let mut unique = reasons.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), reasons.len());
    }
}
