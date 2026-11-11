//! Executable identity and approval domain types (spike slice 2).
//!
//! Framework-independent (this module lives in `omnifrons-domain`, which
//! depends on `std` only for this module): it names the shapes a probe, an
//! approval store, and a launch gate agree on, without committing to how a
//! file is hashed, how approvals are persisted, or how a process is
//! actually launched.

use std::fmt::{self, Write as _};
use std::path::PathBuf;
use std::time::SystemTime;

/// A SHA-256 digest of an executable's full byte content.
///
/// Wraps the raw 32 bytes rather than a hex `String`, so equality and
/// hashing are cheap and exact; [`fmt::Display`] renders the conventional
/// lowercase hex form on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest(pub [u8; 32]);

impl Sha256Digest {
    /// The number of leading hex characters [`Self::short_hex`] renders --
    /// 4 bytes' worth, enough to distinguish digests in a debug detail
    /// field without echoing the full 64-character digest.
    const SHORT_HEX_CHARS: usize = 8;

    /// Render the full 64-character lowercase hex digest.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(self.0.len() * 2);
        for byte in self.0 {
            // `write!` into a `String` never fails.
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Render a short, 8-character hex prefix, for a structured error
    /// detail field that must not carry the full digest
    /// (`docs/spike-log.md` § Slice 2).
    #[must_use]
    pub fn short_hex(&self) -> String {
        self.to_hex()[..Self::SHORT_HEX_CHARS].to_string()
    }

    /// Parse the 64-character hex form [`Self::to_hex`] renders (either
    /// case), or `None` if `text` is not exactly 64 hex characters. Added
    /// in spike slice 5 for the digests an `artifact.publish` proposal
    /// names (`crate::outbox::ProposedEntry`).
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Platform-specific evidence gathered alongside an [`ExecutableIdentity`]
/// probe, informational only -- excluded from [`ExecutableIdentity`]'s own
/// equality (see its doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvidence {
    /// Unix: the file's permission mode bits, as `symlink_metadata` on the
    /// canonicalized path reported them.
    Unix {
        /// The raw permission mode bits (e.g. `0o755`).
        mode: u32,
    },
    /// Windows: the file's extension and attribute bits. Provisional --
    /// Windows executability is presently decided by extension allowlist
    /// alone (`docs/spike-log.md` § Slice 2), not a real ACL or signature
    /// check.
    Windows {
        /// The file's extension, lowercased, without a leading dot.
        extension: String,
        /// The raw Windows file attribute bits.
        attributes: u32,
    },
}

/// A probed executable's identity: where it canonically lives, its size,
/// content digest, and informational platform evidence.
///
/// # Equality
///
/// [`PartialEq`] is implemented by hand, deliberately comparing only
/// `canonical_path`, `size`, and `sha256` -- `modified_at` and `platform`
/// are informational, gathered for display and audit, but must never
/// affect whether two probes of "the same" executable are considered
/// equal (`docs/spike-log.md` § Slice 2: re-probing the same file at two
/// different instants can observe a different `modified_at` with no
/// content change at all).
#[derive(Debug, Clone)]
pub struct ExecutableIdentity {
    /// The fully resolved (symlinks followed) filesystem path.
    pub canonical_path: PathBuf,
    /// The file's size in bytes at the time of the probe that hashed it.
    pub size: u64,
    /// The SHA-256 digest of the file's full byte content.
    pub sha256: Sha256Digest,
    /// The file's last-modified time, when the platform reports one.
    /// Informational only -- excluded from equality.
    pub modified_at: Option<SystemTime>,
    /// Platform-specific evidence gathered alongside the probe.
    /// Informational only -- excluded from equality.
    pub platform: PlatformEvidence,
}

impl PartialEq for ExecutableIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_path == other.canonical_path
            && self.size == other.size
            && self.sha256 == other.sha256
    }
}

impl Eq for ExecutableIdentity {}

/// The outcome of probing a candidate path for its executable identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The candidate resolved to a regular, executable file within the
    /// size cap, and was hashed successfully.
    Identity(ExecutableIdentity),
    /// The canonicalized candidate is not a regular file (e.g. a
    /// directory, a device, a FIFO).
    NotRegularFile,
    /// The candidate is a regular file but does not appear executable on
    /// this platform.
    NotExecutable,
    /// The candidate's metadata or content could not be read.
    Unreadable,
    /// The candidate exceeds the probe's size cap.
    TooLarge,
    /// The candidate is a `#!` script whose interpreter cannot start from
    /// the sealed copy the app launches on Linux: today Node.js, see
    /// [`is_node_script`] (#186). Only a Linux probe reports it.
    UnsupportedInterpreter,
}

/// How many bytes of a `#!` line the Linux kernel reads
/// (`BINPRM_BUF_SIZE`): an interpreter or argument past this limit is
/// never seen.
pub const SHEBANG_LINE_BYTES: usize = 256;

/// Whether `head`, an executable's leading bytes, is a Node.js script:
/// its `#!` line names `node` (or Debian's `nodejs`) as the interpreter,
/// directly or through `env`.
///
/// Node cannot start from the sealed copy the app launches on Linux
/// through `/proc/self/fd/<n>`: it resolves its entry file's real path and
/// finds `/memfd:… (deleted)` (#186). Scripts for sh, bash, python3 and
/// perl were checked and start there, so only node is recognized.
///
/// The line is read as the kernel reads it: at most
/// [`SHEBANG_LINE_BYTES`], up to the first line feed; the interpreter is
/// the first word after `#!` (spaces and tabs skipped), and the rest of
/// the line is its argument. For `env`, the program is the argument's
/// first word that is neither an option (with the value of `-u`, `-C`,
/// `-P`, `-a`, `--unset`, `--chdir` and `--argv0` skipped, also at the end
/// of a cluster such as `-iu`) nor a `NAME=VALUE` assignment, as `env -S`
/// splits it.
#[must_use]
pub fn is_node_script(head: &[u8]) -> bool {
    let head = &head[..head.len().min(SHEBANG_LINE_BYTES)];
    let Some(rest) = head.strip_prefix(b"#!") else {
        return false;
    };
    let line = rest.split(|&byte| byte == b'\n').next().unwrap_or(&[]);
    let mut words = line
        .split(|&byte| byte == b' ' || byte == b'\t')
        .filter(|word| !word.is_empty());
    let Some(interpreter) = words.next() else {
        return false;
    };
    match file_name(interpreter) {
        b"env" => env_program(words).is_some_and(|program| is_node_name(file_name(program))),
        name => is_node_name(name),
    }
}

/// The program `env` runs, given the words of its argument: the first word
/// that is neither an option (nor an option's value) nor a `NAME=VALUE`
/// assignment.
fn env_program<'a>(mut words: impl Iterator<Item = &'a [u8]>) -> Option<&'a [u8]> {
    while let Some(word) = words.next() {
        if value_is_next_word(word) {
            words.next();
        } else if !word.starts_with(b"-") && !word.contains(&b'=') {
            return Some(word);
        }
    }
    None
}

/// Whether `word` is an `env` option whose value is the next word:
/// `--unset`, `--chdir` or `--argv0`, or a cluster of short options whose
/// first value-taking letter (`u`, `C`, `P`, `a`) ends it, as in `-u` or
/// `-iu`. When such a letter is not last, the rest of the cluster is its
/// value (`-uNAME`), and the next word is not consumed.
fn value_is_next_word(word: &[u8]) -> bool {
    match word {
        b"--unset" | b"--chdir" | b"--argv0" => true,
        [b'-', letters @ ..] if letters.first().is_some_and(|&letter| letter != b'-') => letters
            .iter()
            .position(|letter| matches!(letter, b'u' | b'C' | b'P' | b'a'))
            .is_some_and(|at| at + 1 == letters.len()),
        _ => false,
    }
}

/// The last component of a `/`-separated path.
fn file_name(path: &[u8]) -> &[u8] {
    path.rsplit(|&byte| byte == b'/').next().unwrap_or(path)
}

/// Whether `name` is the file name of the Node.js binary.
fn is_node_name(name: &[u8]) -> bool {
    name == b"node" || name == b"nodejs"
}

/// An approval's opaque identifier, stable across the approval's lifetime
/// (including revocation, which is a status change on the same id, never a
/// new one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApprovalId(pub u64);

/// An opaque marker standing in for "the user of this device", with no
/// name or other identifying attribute captured.
///
/// A device-local approval is not a claim about a specific person's
/// identity -- there is exactly one deliberately uninformative value in
/// this type, `DeviceLocalUser`, distinguishing "approved by whoever was
/// using this device" from no approver information at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeviceLocalUser;

/// An approval's current status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    /// The approval is in force.
    Active,
    /// The approval was revoked at the given time. A revoked approval is
    /// never deleted or overwritten in place -- see
    /// `docs/spike-log.md` § Slice 2 on the append-only store.
    Revoked {
        /// When the revocation happened.
        revoked_at: SystemTime,
    },
}

/// One approval on record: which identity was approved, when, by whom
/// (opaquely), and its current status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    /// This approval's stable identifier.
    pub approval_id: ApprovalId,
    /// The executable identity that was approved.
    pub identity: ExecutableIdentity,
    /// When the approval was recorded.
    pub approved_at: SystemTime,
    /// Who approved it (opaque; see [`DeviceLocalUser`]).
    pub approver: DeviceLocalUser,
    /// The approval's current status.
    pub status: ApprovalStatus,
}

/// Whether an approved executable is still where it was approved, at the
/// size it was approved (#184) -- a hint for the approval lists, judged from
/// the filesystem's metadata at the canonical path, so an operator is not
/// offered an approval whose file is gone.
///
/// Never a launch decision: a file of the recorded size can still hold
/// other bytes, and only the pre-launch re-probe's digest comparison can
/// tell. [`Self::Present`] is therefore "nothing seen against it", not
/// "verified".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutablePresence {
    /// A regular file of the recorded size is at the canonical path.
    Present,
    /// The canonical path no longer leads to a regular file this user can
    /// read the metadata of: deleted, its directory deleted, something other
    /// than a regular file in its place, or a symlink that now takes the path
    /// somewhere else.
    Missing,
    /// A regular file is at the canonical path, but not of the recorded
    /// size: its content changed since the approval.
    Changed,
}

impl ExecutablePresence {
    /// Judge from what a metadata read observed at the canonical path:
    /// `None` when no regular file was found there, otherwise its size.
    #[must_use]
    pub const fn judge(recorded_size: u64, observed_regular_file_size: Option<u64>) -> Self {
        match observed_regular_file_size {
            None => Self::Missing,
            Some(size) if size == recorded_size => Self::Present,
            Some(_) => Self::Changed,
        }
    }
}

/// Why a launch was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialReason {
    /// No approval is on record for this identity at all.
    Unapproved,
    /// An approval is on record, but the candidate's content digest no
    /// longer matches it: the file changed since it was approved.
    ChangedSinceApproval {
        /// The digest recorded at approval time.
        recorded: Sha256Digest,
        /// The digest observed by the re-probe that triggered this denial.
        observed: Sha256Digest,
    },
    /// An approval is on record for this content, but at a different
    /// canonical path than the one now resolved: something has shadowed
    /// the approved path.
    ShadowedPath {
        /// The canonical path recorded at approval time.
        approved: PathBuf,
        /// The canonical path the re-probe resolved instead.
        resolved: PathBuf,
    },
    /// An approval is on record, but it was revoked.
    Revoked,
    /// The re-probe immediately before launch itself failed.
    ProbeFailed(ProbeOutcome),
}

/// The public-facing token a [`DenialReason`] reduces to: closed and
/// coarser than the full reason, for any surface that must not echo a
/// recorded/observed digest or path pair.
///
/// Deliberately has no "done"/"ok"/"trusted" variant: every
/// [`DenialReason`] is, definitionally, a denial, so this type only ever
/// names *why launching did not happen*, never that it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicToken {
    /// The candidate is not trusted to launch: unapproved, its approval
    /// was revoked, its content changed since approval, or its path was
    /// shadowed.
    Untrusted,
    /// Whether the candidate can be trusted could not be determined: the
    /// re-probe could not even read it.
    Uncertain,
    /// The re-probe determined the candidate cannot be an executable at
    /// all (not a regular file, not executable, over the size cap, or a
    /// script whose interpreter cannot start from the sealed copy).
    Failed,
}

impl DenialReason {
    /// Reduce this reason to its [`PublicToken`], per
    /// `docs/spike-log.md` § Slice 2's mapping:
    /// `Unapproved`/`ShadowedPath`/`Revoked`/`ChangedSinceApproval` ->
    /// `Untrusted`; `ProbeFailed(Unreadable)` -> `Uncertain`;
    /// `ProbeFailed(NotRegularFile|NotExecutable|TooLarge|UnsupportedInterpreter)`
    /// -> `Failed`.
    #[must_use]
    pub fn public_token(&self) -> PublicToken {
        match self {
            Self::Unapproved
            | Self::ChangedSinceApproval { .. }
            | Self::ShadowedPath { .. }
            | Self::Revoked => PublicToken::Untrusted,
            Self::ProbeFailed(outcome) => match outcome {
                ProbeOutcome::Unreadable => PublicToken::Uncertain,
                // `Identity(_)` wrapped in `ProbeFailed` is a construction
                // bug elsewhere (a `LaunchGate` never builds this
                // combination), not a real denial category -- folded into
                // `Failed` defensively rather than panicking, since this
                // method must stay total over every value the type
                // technically allows.
                ProbeOutcome::NotRegularFile
                | ProbeOutcome::NotExecutable
                | ProbeOutcome::TooLarge
                | ProbeOutcome::UnsupportedInterpreter
                | ProbeOutcome::Identity(_) => PublicToken::Failed,
            },
        }
    }
}

/// The outcome of a launch-gate decision: allowed under a specific
/// approval, or denied with a reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDecision {
    /// The launch is allowed, under this approval id.
    Allowed(ApprovalId),
    /// The launch is denied, for this reason.
    Denied(DenialReason),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn identity(path: &str, size: u64, digest_byte: u8) -> ExecutableIdentity {
        ExecutableIdentity {
            canonical_path: PathBuf::from(path),
            size,
            sha256: Sha256Digest([digest_byte; 32]),
            modified_at: Some(SystemTime::UNIX_EPOCH),
            platform: PlatformEvidence::Unix { mode: 0o755 },
        }
    }

    #[test]
    fn presence_is_judged_from_the_regular_file_at_the_path_and_its_size() {
        assert_eq!(
            ExecutablePresence::judge(4096, Some(4096)),
            ExecutablePresence::Present
        );
        assert_eq!(
            ExecutablePresence::judge(4096, Some(4097)),
            ExecutablePresence::Changed
        );
        assert_eq!(
            ExecutablePresence::judge(4096, Some(0)),
            ExecutablePresence::Changed
        );
        assert_eq!(
            ExecutablePresence::judge(4096, None),
            ExecutablePresence::Missing
        );
    }

    #[test]
    fn equality_ignores_modified_at_and_platform_evidence() {
        let mut a = identity("/bin/example", 100, 1);
        let mut b = identity("/bin/example", 100, 1);
        b.modified_at = None;
        b.platform = PlatformEvidence::Windows {
            extension: "exe".to_string(),
            attributes: 0,
        };

        assert_eq!(a, b);

        a.modified_at = Some(SystemTime::now());
        assert_eq!(a, b, "modified_at must never affect equality");
    }

    #[test]
    fn equality_differs_on_digest() {
        let a = identity("/bin/example", 100, 1);
        let b = identity("/bin/example", 100, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn equality_differs_on_size() {
        let a = identity("/bin/example", 100, 1);
        let b = identity("/bin/example", 200, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn equality_differs_on_path() {
        let a = identity("/bin/example", 100, 1);
        let b = identity("/bin/other", 100, 1);
        assert_ne!(a, b);
    }

    /// Every `DenialReason` must map to exactly one `PublicToken`, and
    /// `PublicToken` itself has no "done"-like variant to map to (it names
    /// only negative outcomes: `Untrusted`, `Uncertain`, `Failed`).
    #[test]
    fn every_denial_reason_maps_to_exactly_one_public_token() {
        let recorded = Sha256Digest([1; 32]);
        let observed = Sha256Digest([2; 32]);
        let approved = PathBuf::from("/bin/approved");
        let resolved = PathBuf::from("/bin/resolved");

        let cases = [
            (DenialReason::Unapproved, PublicToken::Untrusted),
            (
                DenialReason::ChangedSinceApproval { recorded, observed },
                PublicToken::Untrusted,
            ),
            (
                DenialReason::ShadowedPath {
                    approved: approved.clone(),
                    resolved: resolved.clone(),
                },
                PublicToken::Untrusted,
            ),
            (DenialReason::Revoked, PublicToken::Untrusted),
            (
                DenialReason::ProbeFailed(ProbeOutcome::Unreadable),
                PublicToken::Uncertain,
            ),
            (
                DenialReason::ProbeFailed(ProbeOutcome::NotRegularFile),
                PublicToken::Failed,
            ),
            (
                DenialReason::ProbeFailed(ProbeOutcome::NotExecutable),
                PublicToken::Failed,
            ),
            (
                DenialReason::ProbeFailed(ProbeOutcome::TooLarge),
                PublicToken::Failed,
            ),
            (
                DenialReason::ProbeFailed(ProbeOutcome::UnsupportedInterpreter),
                PublicToken::Failed,
            ),
        ];

        for (reason, expected) in cases {
            assert_eq!(
                reason.public_token(),
                expected,
                "{reason:?}.public_token() must be {expected:?}"
            );
        }
    }

    /// `PublicToken` has no "done"-like variant at all -- this is a
    /// maintenance trip-wire: adding one (e.g. `Trusted`/`Ok`) makes this
    /// `match` non-exhaustive and fails the build, forcing a deliberate
    /// decision here rather than a silently added "everything is fine"
    /// token for what is, structurally, always a denial's public face.
    #[test]
    fn public_token_has_exactly_three_variants_none_done_like() {
        let assert_exhaustive = |token: PublicToken| match token {
            PublicToken::Untrusted | PublicToken::Uncertain | PublicToken::Failed => {}
        };
        assert_exhaustive(PublicToken::Untrusted);
        assert_exhaustive(PublicToken::Uncertain);
        assert_exhaustive(PublicToken::Failed);
    }

    /// #186: a `#!` line whose interpreter is node, named directly or
    /// through `env` (with its options, assignments and `-S`), is a node
    /// script.
    #[test]
    fn a_shebang_that_runs_node_is_a_node_script() {
        let heads: [&[u8]; 13] = [
            b"#!/usr/bin/env node\nconsole.log('ok')\n",
            b"#!/usr/bin/node\n",
            b"#! /usr/local/bin/node\n",
            b"#!\t/usr/bin/env\tnode\n",
            b"#!/usr/bin/env -S node --no-warnings\n",
            b"#!/usr/bin/env -S -u NODE_OPTIONS node\n",
            b"#!/usr/bin/env -S -uNODE_OPTIONS node\n",
            b"#!/usr/bin/env -S --unset NODE_OPTIONS node\n",
            b"#!/usr/bin/env -S NODE_ENV=production node\n",
            b"#!/usr/bin/env -- node\n",
            b"#!/usr/bin/env node",
            b"#!/usr/bin/nodejs\n",
            b"#!/usr/bin/env nodejs\n",
        ];
        for head in heads {
            assert!(
                is_node_script(head),
                "{:?} must be a node script",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// #186: only node is refused. Other interpreters, a shell wrapper
    /// that runs node on a later line, native binaries and files with no
    /// `#!` line are not node scripts.
    #[test]
    fn other_interpreters_binaries_and_plain_files_are_not_node_scripts() {
        let heads: [&[u8]; 16] = [
            b"#!/usr/bin/env -S -iu node sh\n",
            b"#!/usr/bin/env -S -iC /srv/node sh\n",
            b"#!/usr/bin/env -S --argv0 node sh\n",
            b"#!/bin/sh\necho ok\n",
            b"#!/usr/bin/env bash\n",
            b"#!/usr/bin/env python3\n",
            b"#!/usr/bin/perl\n",
            b"#!/usr/bin/env nodemon\n",
            b"#!/opt/node/bin/python3\n",
            b"#!/usr/bin/env -S deno run\n",
            b"#!/bin/sh\nexec node \"$0.js\"\n",
            b"\x7fELF\x02\x01\x01\x00",
            b"console.log('no shebang')\n",
            b"",
            b"#!",
            b"#!/usr/bin/env\n",
        ];
        for head in heads {
            assert!(
                !is_node_script(head),
                "{:?} must not be a node script",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// #186: the kernel reads at most [`SHEBANG_LINE_BYTES`] of the `#!`
    /// line, so a word past that limit never names the interpreter.
    #[test]
    fn a_word_past_the_shebang_line_limit_is_ignored() {
        let mut head = b"#!/usr/bin/env".to_vec();
        head.resize(SHEBANG_LINE_BYTES + 10, b' ');
        head.extend_from_slice(b"node\n");

        assert!(!is_node_script(&head));
    }
}
