//! The switch check (ADR-0005 slice 4): before a model switch restarts a
//! harness, whether the target host reaches the canonical definition
//! from the active project root, judged from the loader table in
//! [`crate::hosts`].
//!
//! A table row is a claim about a host, never a guarantee, so every
//! [`SwitchVerdict`] other than `Uncertain` carries the row's source,
//! access date and status as a [`Claim`]. The host version in use is
//! unknown to this code, so a stale row is not detected here.
//!
//! Claude Code's `AGENTS.md` fallback follows the row's
//! [`fallback_files`](crate::hosts::HostLoader::fallback_files): it
//! applies only when none of the row's `files` exists at the root or in
//! any ancestor, the same condition the ancestor warnings in
//! [`crate::warnings`] reason about.
//!
//! The binding from an [`AdapterId`] to a host is a small table that
//! never extends the closed adapter set: `claude-code` is bound to
//! "Claude Code", and the two generic adapters take a host the user
//! chooses, or stay uncertain.
//!
//! Pure data and lookups: no filesystem access and no hashing.

use std::fmt::{self, Write as _};

use crate::adapter::AdapterId;
use crate::definition::IdentityKind;
use crate::hosts::{self, HostLoader};

/// The scan's name for the project root itself, used as a [`Shadow::path`]
/// when a host's own file displaces the definition at the root.
pub const ROOT_PATH: &str = ".";

/// How many leading characters of a digest [`DefinitionReference`]'s
/// `Display` shows.
const DIGEST_DISPLAY_LEN: usize = 12;

/// The length of a SHA-256 digest in hex characters.
const DIGEST_HEX_LEN: usize = 64;

/// The identity-file names present at the active project root and at
/// each ancestor folder, filled by the application from the scan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SwitchSite {
    /// The identity-file names present at the active project root.
    pub root_files: Vec<String>,
    /// The ancestor folders, nearest first.
    pub ancestors: Vec<AncestorSite>,
}

/// One ancestor folder of the active project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AncestorSite {
    /// The folder's path, shown verbatim in a verdict.
    pub path: String,
    /// The identity-file names present in that folder.
    pub files: Vec<String>,
}

/// The provenance of a loader-table row: what a verdict rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    /// The host's name.
    pub host: &'static str,
    /// The row's source URL.
    pub source: &'static str,
    /// The row's access date, ISO 8601.
    pub accessed: &'static str,
    /// The row's status text.
    pub status: &'static str,
}

impl Claim {
    fn of(row: &'static HostLoader) -> Self {
        Self {
            host: row.name,
            source: row.source,
            accessed: row.accessed,
            status: row.status,
        }
    }
}

/// The file that blocks the canonical definition: an ancestor's file that
/// shadows Claude Code's `AGENTS.md` fallback, or a host-specific file at
/// the root itself ([`ROOT_PATH`]) that the host loads ahead of the
/// definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    /// The nearest ancestor folder holding one of the host's files, or
    /// [`ROOT_PATH`] when the file sits at the root.
    pub path: String,
    /// The file found there: the first present in the row's `files`
    /// order.
    pub file: &'static str,
}

/// Why a check could not decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncertainReason {
    /// The loader table has no row for this host name.
    UnknownHost {
        /// The name that matched no row.
        name: String,
    },
    /// The adapter is not bound to a host and the user chose none.
    NoHostChosen {
        /// The adapter that needs a host.
        adapter: AdapterId,
    },
}

/// Whether a host reaches the canonical definition from the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchVerdict {
    /// The host loads a file present at the root.
    Reaches {
        /// The file the host loads: the first present in the row's
        /// order.
        file: String,
        /// The row this rests on.
        claim: Claim,
    },
    /// No file the host loads from this root is present.
    Unreached {
        /// The files the host loads, in the row's order.
        expected: Vec<&'static str>,
        /// The nearest ancestor whose own file blocks Claude Code's
        /// `AGENTS.md` fallback; set only when `AGENTS.md` is present
        /// at the root and the host has that fallback.
        shadowed_by: Option<Shadow>,
        /// The fallback file selected at the root (the first present in
        /// the row's `fallback_files` order) when `shadowed_by` is set;
        /// `None` otherwise.
        fallback: Option<&'static str>,
        /// The row this rests on.
        claim: Claim,
    },
    /// The check could not decide.
    Uncertain {
        /// Why not.
        reason: UncertainReason,
    },
}

impl SwitchVerdict {
    /// A stable token for this variant: `reaches`, `unreached` or
    /// `uncertain`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Reaches { .. } => "reaches",
            Self::Unreached { .. } => "unreached",
            Self::Uncertain { .. } => "uncertain",
        }
    }
}

fn join_or(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_string(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// `text` with every control character (and U+2028/U+2029) replaced by
/// its escaped form, so a value interpolated into a one-line text keeps
/// it one physical line and its bytes stay visible: `\n` becomes the two
/// characters backslash and `n`, `\r` likewise, any other as `\u{XXXX}`.
fn single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{{{:04X}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// The verdict as one physical line. Every value interpolated that is
/// not a static table entry (a host name the caller chose, an ancestor
/// path, a file name) goes through `single_line`, so a line break in
/// it is shown escaped instead of splitting the line.
impl fmt::Display for SwitchVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reaches { file, claim } => write!(
                f,
                "{} reaches the definition through {} (row accessed {}, {})",
                claim.host,
                single_line(file),
                claim.accessed,
                claim.status.to_lowercase()
            ),
            Self::Unreached {
                expected,
                shadowed_by: Some(shadow),
                fallback,
                claim,
            } if !expected.is_empty() && shadow.path == ROOT_PATH => write!(
                f,
                "{} does not reach the definition: {} is displaced by {} at the root",
                claim.host,
                fallback.unwrap_or("AGENTS.md"),
                shadow.file
            ),
            Self::Unreached {
                expected,
                shadowed_by: Some(shadow),
                fallback,
                claim,
            } if !expected.is_empty() => write!(
                f,
                "{} does not reach the definition: {} is shadowed by {} in {}",
                claim.host,
                fallback.unwrap_or("AGENTS.md"),
                shadow.file,
                single_line(&shadow.path)
            ),
            Self::Unreached {
                expected, claim, ..
            } => match expected.as_slice() {
                [only] => write!(
                    f,
                    "{} does not reach the definition: it loads {only}, which is absent",
                    claim.host
                ),
                many => write!(
                    f,
                    "{} does not reach the definition: it loads {}, none of which is present",
                    claim.host,
                    join_or(many)
                ),
            },
            Self::Uncertain {
                reason: UncertainReason::UnknownHost { name },
            } => write!(
                f,
                "No loader row for host {}: the check is uncertain",
                single_line(name)
            ),
            Self::Uncertain {
                reason: UncertainReason::NoHostChosen { adapter },
            } => write!(
                f,
                "No host is chosen for adapter {}: the check is uncertain",
                single_line(&adapter.to_string())
            ),
        }
    }
}

fn first_present(candidates: &'static [&'static str], present: &[String]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|name| present.iter().any(|file| file == name))
}

/// Whether the host named `host` reaches the canonical definition from
/// the root described by `site`.
#[must_use]
pub fn switch_check(host: &str, site: &SwitchSite) -> SwitchVerdict {
    let Some(row) = hosts::host(host) else {
        return SwitchVerdict::Uncertain {
            reason: UncertainReason::UnknownHost {
                name: host.to_string(),
            },
        };
    };
    let claim = Claim::of(row);
    if let Some(file) = first_present(row.files, &site.root_files) {
        if IdentityKind::from_file_name(file).is_some() {
            return SwitchVerdict::Reaches {
                file: file.to_string(),
                claim,
            };
        }
        // A host-specific file outside the identity kinds (`.hermes.md`,
        // `HERMES.md`, `.cursorrules`) is not the definition: the host loads
        // it ahead of the canonical file, so the definition is displaced at
        // the root itself.
        return unreached(
            row,
            Some(Shadow {
                path: ROOT_PATH.to_string(),
                file,
            }),
            None,
            claim,
        );
    }
    let Some(fallback) = first_present(row.fallback_files, &site.root_files) else {
        return unreached(row, None, None, claim);
    };
    let shadow = site.ancestors.iter().find_map(|ancestor| {
        first_present(row.files, &ancestor.files).map(|file| Shadow {
            path: ancestor.path.clone(),
            file,
        })
    });
    match shadow {
        None => SwitchVerdict::Reaches {
            file: fallback.to_string(),
            claim,
        },
        Some(shadow) => unreached(row, Some(shadow), Some(fallback), claim),
    }
}

fn unreached(
    row: &'static HostLoader,
    shadowed_by: Option<Shadow>,
    fallback: Option<&'static str>,
    claim: Claim,
) -> SwitchVerdict {
    SwitchVerdict::Unreached {
        expected: row.files.to_vec(),
        shadowed_by,
        fallback,
        claim,
    }
}

/// The host a built-in adapter is bound to, or `None` for the generic
/// adapters, which take a host the user chooses. The returned name is
/// the exact [`HostLoader::name`] of its row.
#[must_use]
pub fn host_for_adapter(adapter: &AdapterId) -> Option<&'static str> {
    if *adapter == AdapterId::claude_code() {
        Some("Claude Code")
    } else {
        None
    }
}

/// The check for `adapter`: its bound host when it has one, else
/// `chosen_host`, else uncertain.
#[must_use]
pub fn switch_check_for_adapter(
    adapter: &AdapterId,
    chosen_host: Option<&str>,
    site: &SwitchSite,
) -> SwitchVerdict {
    match host_for_adapter(adapter).or(chosen_host) {
        Some(host) => switch_check(host, site),
        None => SwitchVerdict::Uncertain {
            reason: UncertainReason::NoHostChosen {
                adapter: adapter.clone(),
            },
        },
    }
}

/// Why a [`DefinitionReference`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    /// The path is empty.
    #[error("the reference path is empty")]
    EmptyPath,
    /// The path holds a line break or NUL, which would split the
    /// canonical line.
    #[error("the reference path holds a line break or NUL")]
    PathNotSingleLine,
    /// The digest is not exactly 64 lowercase hexadecimal characters.
    #[error("the reference digest is not 64 lowercase hex characters")]
    DigestNotSha256Hex,
}

/// The canonical definition file named for a startup brief: its path and
/// the digest of its bytes. The application fills both; nothing here
/// hashes. Construction checks the shape, so [`Self::canonical`] is one
/// line by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionReference {
    path: String,
    digest: String,
}

impl DefinitionReference {
    /// A reference to `path` with the SHA-256 `digest` of its bytes.
    ///
    /// # Errors
    ///
    /// [`ReferenceError`] for an empty path, a path holding `\n`, `\r`
    /// or NUL, or a digest that is not exactly 64 lowercase hex
    /// characters.
    pub fn new(path: impl Into<String>, digest: impl Into<String>) -> Result<Self, ReferenceError> {
        let path = path.into();
        let digest = digest.into();
        if path.is_empty() {
            return Err(ReferenceError::EmptyPath);
        }
        if path.contains(['\n', '\r', '\0']) {
            return Err(ReferenceError::PathNotSingleLine);
        }
        let hex = digest.len() == DIGEST_HEX_LEN
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !hex {
            return Err(ReferenceError::DigestNotSha256Hex);
        }
        Ok(Self { path, digest })
    }

    /// The definition's path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The digest of its bytes, 64 lowercase hex characters.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// One line, `definition path=<len>:<path> digest=<len>:<digest>`,
    /// each value prefixed with its byte length.
    #[must_use]
    pub fn canonical(&self) -> String {
        format!(
            "definition path={}:{} digest={}:{}",
            self.path.len(),
            self.path,
            self.digest.len(),
            self.digest
        )
    }
}

impl fmt::Display for DefinitionReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix: String = self.digest.chars().take(DIGEST_DISPLAY_LEN).collect();
        write!(f, "{}@{prefix}", self.path)
    }
}
