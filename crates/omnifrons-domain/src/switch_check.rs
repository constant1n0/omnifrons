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

use std::fmt;

use crate::adapter::AdapterId;
use crate::hosts::{self, HostLoader};

/// How many leading characters of a digest [`DefinitionReference`]'s
/// `Display` shows.
const DIGEST_DISPLAY_LEN: usize = 12;

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

/// The ancestor file that blocks Claude Code's `AGENTS.md` fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    /// The nearest ancestor folder holding one of the host's files.
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

impl fmt::Display for SwitchVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reaches { file, claim } => write!(
                f,
                "{} reaches the definition through {file} (row accessed {}, {})",
                claim.host,
                claim.accessed,
                claim.status.to_lowercase()
            ),
            Self::Unreached {
                expected,
                shadowed_by: Some(shadow),
                claim,
            } if !expected.is_empty() => write!(
                f,
                "{} does not reach the definition: AGENTS.md is shadowed by {} in {}",
                claim.host, shadow.file, shadow.path
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
            } => write!(f, "No loader row for host {name}: the check is uncertain"),
            Self::Uncertain {
                reason: UncertainReason::NoHostChosen { adapter },
            } => write!(
                f,
                "No host is chosen for adapter {adapter}: the check is uncertain"
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
        return SwitchVerdict::Reaches {
            file: file.to_string(),
            claim,
        };
    }
    let Some(fallback) = first_present(row.fallback_files, &site.root_files) else {
        return unreached(row, None, claim);
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
        Some(shadow) => unreached(row, Some(shadow), claim),
    }
}

fn unreached(row: &'static HostLoader, shadowed_by: Option<Shadow>, claim: Claim) -> SwitchVerdict {
    SwitchVerdict::Unreached {
        expected: row.files.to_vec(),
        shadowed_by,
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

/// The canonical definition file named for a startup brief: its path and
/// the digest of its bytes. The application fills both; nothing here
/// hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionReference {
    /// The definition's path.
    pub path: String,
    /// The digest of its bytes, as text.
    pub digest: String,
}

impl DefinitionReference {
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
