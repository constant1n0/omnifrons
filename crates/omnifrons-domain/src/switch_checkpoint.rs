//! The same-device switch checkpoint (docs/target-architecture.md § Model
//! and harness switching): the envelope a switch writes after it stops the
//! outgoing run, and the facts the four conditions are proven from.
//!
//! A switch proves quiescence, shutdown, checkpoint integrity and workspace
//! consistency, and no other plane (handoff-transaction-protocol.md §
//! Document authority). This module holds the facts and derives the first
//! two conditions and the inventory part of the fourth. Integrity is the
//! application's: it seals the encoded body with a SHA-256 through its
//! hasher port, and this module only frames the sealed record.
//!
//! The envelope carries no raw filesystem path (HTP-001-R18): the project
//! is its identity digest, and every file is named by a
//! [`RootRelativePath`].
//!
//! # Encoding
//!
//! The body is a sequence of records `key=<byte length>:<value>\n`, so a
//! value may hold any text, line breaks included. The first record is
//! `schema=omnifrons.switch-checkpoint/<major>.<minor>`. A decoder rejects
//! an unknown major and keeps the fields it does not know, in order, so
//! re-encoding a newer minor loses nothing (versioning-and-compatibility.md:
//! "Reject unknown major; preserve unknown optional minor fields").

use std::fmt;

use crate::adapter::{AdapterId, AgentPrompt};
use crate::executable::Sha256Digest;
use crate::outbox::RunId;
use crate::redaction::scrub_secrets;
use crate::scope::ProcessTerminalState;
use crate::switch_check::DefinitionReference;

/// The schema name in the first record.
pub const SCHEMA_NAME: &str = "omnifrons.switch-checkpoint";
/// The major version this module reads and writes.
pub const MAJOR: u32 = 1;
/// The minor version this module writes.
pub const MINOR: u32 = 0;
/// The most entries an inventory may hold. A walk that reaches it records
/// a capped inventory instead, which leaves consistency unprovable.
pub const INVENTORY_MAX_ENTRIES: usize = 10_000;
/// The longest value a record may carry, in bytes.
pub const RECORD_MAX_BYTES: usize = 64 * 1024;

/// Why a [`RootRelativePath`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The path is empty.
    #[error("the path is empty")]
    Empty,
    /// The path is absolute or names a drive.
    #[error("the path is absolute")]
    Absolute,
    /// The path holds a backslash.
    #[error("the path holds a backslash")]
    Backslash,
    /// A component is `.` or `..`.
    #[error("a path component is `.` or `..`")]
    DotComponent,
    /// A component is empty (a doubled or trailing `/`).
    #[error("a path component is empty")]
    EmptyComponent,
    /// The path holds a line break, NUL or another control character.
    #[error("the path holds a control character")]
    ControlCharacter,
}

/// A path below the active project root, `/`-separated, that cannot name
/// a location outside it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RootRelativePath(String);

impl RootRelativePath {
    /// Validate `text` as a root-relative path.
    ///
    /// # Errors
    ///
    /// [`PathError`] for an empty, absolute or drive-prefixed path, a
    /// backslash, a `.` or `..` or empty component, or a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, PathError> {
        let text = text.into();
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.chars().any(char::is_control) {
            return Err(PathError::ControlCharacter);
        }
        if text.starts_with('/') {
            return Err(PathError::Absolute);
        }
        if text.contains('\\') {
            return Err(PathError::Backslash);
        }
        let mut components = text.split('/');
        let first = components.next().unwrap_or_default();
        let drive =
            first.len() == 2 && first.ends_with(':') && first.as_bytes()[0].is_ascii_alphabetic();
        if drive {
            return Err(PathError::Absolute);
        }
        for component in std::iter::once(first).chain(components) {
            match component {
                "" => return Err(PathError::EmptyComponent),
                "." | ".." => return Err(PathError::DotComponent),
                _ => {}
            }
        }
        Ok(Self(text))
    }

    /// The path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RootRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One file of an [`Inventory`]: its path, the digest of its bytes and its
/// size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    /// The file's path below the root.
    pub path: RootRelativePath,
    /// The SHA-256 of its bytes.
    pub digest: Sha256Digest,
    /// Its size in bytes.
    pub size: u64,
}

/// Why an [`Inventory`] was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InventoryError {
    /// Two entries name the same path.
    #[error("the inventory names {0} twice")]
    DuplicatePath(String),
    /// More than [`INVENTORY_MAX_ENTRIES`] entries.
    #[error("the inventory holds more than {INVENTORY_MAX_ENTRIES} entries")]
    TooManyEntries,
}

/// The files of the workspace at one moment, sorted by path. A capped
/// inventory (`complete == false`) lists only part of the workspace, so
/// nothing can be proven from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    entries: Vec<InventoryEntry>,
    complete: bool,
}

impl Inventory {
    /// An inventory of `entries`, sorted by path.
    ///
    /// # Errors
    ///
    /// [`InventoryError`] for a repeated path or too many entries.
    pub fn new(mut entries: Vec<InventoryEntry>, complete: bool) -> Result<Self, InventoryError> {
        if entries.len() > INVENTORY_MAX_ENTRIES {
            return Err(InventoryError::TooManyEntries);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(pair) = entries.windows(2).find(|pair| pair[0].path == pair[1].path) {
            return Err(InventoryError::DuplicatePath(pair[0].path.0.clone()));
        }
        Ok(Self { entries, complete })
    }

    /// The entries, sorted by path.
    #[must_use]
    pub fn entries(&self) -> &[InventoryEntry] {
        &self.entries
    }

    /// Whether the walk listed every file.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// How `now` differs from this inventory: [`Consistency::Unproven`]
    /// when either is capped, else every added, removed and modified path.
    #[must_use]
    pub fn compare(&self, now: &Self) -> Consistency {
        if !self.complete || !now.complete {
            return Consistency::Unproven;
        }
        let mut changes = InventoryChanges::default();
        let (mut before, mut after) = (
            self.entries.iter().peekable(),
            now.entries.iter().peekable(),
        );
        loop {
            match (before.peek(), after.peek()) {
                (None, None) => break,
                (Some(old), None) => {
                    changes.removed.push(old.path.clone());
                    before.next();
                }
                (None, Some(new)) => {
                    changes.added.push(new.path.clone());
                    after.next();
                }
                (Some(old), Some(new)) => match old.path.cmp(&new.path) {
                    std::cmp::Ordering::Less => {
                        changes.removed.push(old.path.clone());
                        before.next();
                    }
                    std::cmp::Ordering::Greater => {
                        changes.added.push(new.path.clone());
                        after.next();
                    }
                    std::cmp::Ordering::Equal => {
                        if old.digest != new.digest || old.size != new.size {
                            changes.modified.push(old.path.clone());
                        }
                        before.next();
                        after.next();
                    }
                },
            }
        }
        if changes.is_empty() {
            Consistency::Consistent
        } else {
            Consistency::Changed(changes)
        }
    }
}

/// The paths two inventories disagree on, each list sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InventoryChanges {
    /// Paths only the later inventory lists.
    pub added: Vec<RootRelativePath>,
    /// Paths only the earlier inventory lists.
    pub removed: Vec<RootRelativePath>,
    /// Paths both list with a different digest or size.
    pub modified: Vec<RootRelativePath>,
}

impl InventoryChanges {
    /// Whether no path changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// Whether the workspace is still what a checkpoint recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consistency {
    /// Every file is unchanged.
    Consistent,
    /// Some files changed.
    Changed(InventoryChanges),
    /// One of the inventories is capped, so nothing is proven.
    Unproven,
}

/// The run the switch stopped, as the supervisor reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingRun {
    /// The adapter the run used.
    pub adapter: AdapterId,
    /// The run, or `None` when no run was live or retained to stop.
    pub run: Option<RunId>,
    /// The run's terminal state, or `None` when it was not seen to end.
    pub terminal: Option<ProcessTerminalState>,
}

/// A condition the checkpoint could not prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unproven {
    /// The outgoing run was not seen to reach a terminal state.
    RunNotTerminal,
    /// The supervisor could not prove every descendant stopped
    /// (`orphan-risk/uncertain`).
    DescendantsUnproven,
    /// The inventory is capped, so workspace consistency cannot be proven.
    InventoryIncomplete,
}

impl Unproven {
    /// The stable token for this reason.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunNotTerminal => "run-not-terminal",
            Self::DescendantsUnproven => "descendants-unproven",
            Self::InventoryIncomplete => "inventory-incomplete",
        }
    }
}

/// The checkpoint's handoff state (HTP-001-R1): `prepared` once every
/// condition it can prove is proven, `uncertain` with its reasons
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointState {
    /// Quiescence and shutdown are proven and the inventory is complete.
    Prepared,
    /// The reasons, each once, in [`Unproven`] order.
    Uncertain(Vec<Unproven>),
}

/// The facts a switch gathers before it writes its checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointFacts {
    /// When the checkpoint was taken, in seconds since the Unix epoch.
    pub created_unix_seconds: u64,
    /// The active project's identity digest; never its path.
    pub project: Sha256Digest,
    /// The run the switch stopped.
    pub outgoing: OutgoingRun,
    /// The adapter the switch restarts with.
    pub target: AdapterId,
    /// The current goal: the outgoing run's prompt, when there was one.
    pub goal: Option<AgentPrompt>,
    /// The canonical definition by path and digest, when the root has one.
    pub definition: Option<DefinitionReference>,
    /// The workspace inventory taken after the stop.
    pub inventory: Inventory,
}

/// Why a checkpoint could not be built or decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// A terminal state was given for a switch with no outgoing run.
    #[error("a terminal state was given without an outgoing run")]
    TerminalWithoutRun,
    /// The definition reference's path is not root-relative.
    #[error("the definition path is not root-relative")]
    DefinitionPathNotRelative,
    /// The first record is not `schema`.
    #[error("the first record is not the schema")]
    SchemaNotFirst,
    /// The schema names a major this module does not read.
    #[error("unsupported checkpoint major version {0}")]
    UnsupportedMajor(u32),
    /// A required record is missing.
    #[error("the checkpoint has no `{0}` record")]
    MissingField(&'static str),
    /// A record that may appear once appears again.
    #[error("the checkpoint repeats `{0}`")]
    DuplicateField(&'static str),
    /// A known record holds a value it cannot take.
    #[error("the checkpoint's `{0}` value is invalid")]
    InvalidValue(&'static str),
    /// An `integrity` record sits inside the body.
    #[error("an integrity record sits inside the body")]
    IntegrityInBody,
    /// A record key is empty or holds a character outside `[a-z0-9._-]`.
    #[error("a record key is malformed")]
    BadKey,
    /// A record length is empty, has a leading zero, is not decimal, is
    /// longer than [`RECORD_MAX_BYTES`] or splits a character.
    #[error("a record length is malformed")]
    BadLength,
    /// A record value is not followed by a line break.
    #[error("a record value is not followed by a line break")]
    MissingNewline,
    /// The text ends inside a record.
    #[error("the checkpoint ends inside a record")]
    Truncated,
}

/// A switch checkpoint: the facts, the minor it was written with, and any
/// field a newer minor added that this module does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCheckpoint {
    facts: CheckpointFacts,
    minor: u32,
    extra: Vec<(String, String)>,
}

impl SwitchCheckpoint {
    /// A checkpoint of `facts`, at [`MINOR`]. The goal is stored with its
    /// secret-shaped values redacted (TM-001-R6): a goal the scrub cannot
    /// keep within [`AgentPrompt::MAX_BYTES`] is cut at a character
    /// boundary.
    ///
    /// # Errors
    ///
    /// [`CheckpointError::TerminalWithoutRun`] for a terminal state with no
    /// run, [`CheckpointError::DefinitionPathNotRelative`] for a definition
    /// path that could name a raw location.
    pub fn new(mut facts: CheckpointFacts) -> Result<Self, CheckpointError> {
        if facts.outgoing.run.is_none() && facts.outgoing.terminal.is_some() {
            return Err(CheckpointError::TerminalWithoutRun);
        }
        if let Some(definition) = &facts.definition {
            RootRelativePath::new(definition.path())
                .map_err(|_| CheckpointError::DefinitionPathNotRelative)?;
        }
        facts.goal = facts.goal.and_then(|goal| scrubbed_goal(&goal));
        Ok(Self {
            facts,
            minor: MINOR,
            extra: Vec::new(),
        })
    }

    /// The facts the checkpoint holds.
    #[must_use]
    pub const fn facts(&self) -> &CheckpointFacts {
        &self.facts
    }

    /// The minor version it was written with.
    #[must_use]
    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// Fields a newer minor added, in the order they were read.
    #[must_use]
    pub fn extra_fields(&self) -> &[(String, String)] {
        &self.extra
    }

    /// Whether the outgoing run is quiesced: no process it started can
    /// still mutate the workspace. A switch with no run is quiesced.
    ///
    /// # Errors
    ///
    /// [`Unproven::RunNotTerminal`] or [`Unproven::DescendantsUnproven`].
    pub const fn quiescence(&self) -> Result<(), Unproven> {
        self.stop_proof()
    }

    /// Whether the outgoing harness is shut down. The supervisor reports
    /// one terminal state for the whole process group, so in this slice
    /// shutdown rests on the same observation as quiescence.
    ///
    /// # Errors
    ///
    /// [`Unproven::RunNotTerminal`] or [`Unproven::DescendantsUnproven`].
    pub const fn shutdown(&self) -> Result<(), Unproven> {
        self.stop_proof()
    }

    const fn stop_proof(&self) -> Result<(), Unproven> {
        match (&self.facts.outgoing.run, self.facts.outgoing.terminal) {
            (None, _) => Ok(()),
            (Some(_), None) => Err(Unproven::RunNotTerminal),
            (Some(_), Some(ProcessTerminalState::OrphanRiskUncertain)) => {
                Err(Unproven::DescendantsUnproven)
            }
            (Some(_), Some(ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed)) => {
                Ok(())
            }
        }
    }

    /// `prepared`, or `uncertain` with every unproven condition once.
    #[must_use]
    pub fn state(&self) -> CheckpointState {
        let mut reasons = Vec::new();
        for proof in [self.quiescence(), self.shutdown()] {
            if let Err(reason) = proof
                && !reasons.contains(&reason)
            {
                reasons.push(reason);
            }
        }
        if !self.facts.inventory.is_complete() {
            reasons.push(Unproven::InventoryIncomplete);
        }
        if reasons.is_empty() {
            CheckpointState::Prepared
        } else {
            CheckpointState::Uncertain(reasons)
        }
    }
}

fn push_record(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    out.push_str(&value.len().to_string());
    out.push(':');
    out.push_str(value);
    out.push('\n');
}

fn terminal_token(run: Option<&RunId>, terminal: Option<ProcessTerminalState>) -> String {
    match (run, terminal) {
        (None, _) => "none".to_string(),
        (Some(_), None) => "not-terminal".to_string(),
        (Some(_), Some(ProcessTerminalState::Exited { code: Some(code) })) => {
            format!("exited:{code}")
        }
        (Some(_), Some(ProcessTerminalState::Exited { code: None })) => "exited".to_string(),
        (Some(_), Some(ProcessTerminalState::Killed)) => "killed".to_string(),
        (Some(_), Some(ProcessTerminalState::OrphanRiskUncertain)) => {
            "orphan-risk-uncertain".to_string()
        }
    }
}

/// The record that seals a body: `integrity=sha256:<hex>`, to be appended
/// after [`SwitchCheckpoint::encode_body`].
#[must_use]
pub fn integrity_record(digest: &Sha256Digest) -> String {
    let mut out = String::new();
    push_record(
        &mut out,
        "integrity",
        &format!("sha256:{}", digest.to_hex()),
    );
    out
}

/// One decoded record and the byte offset it starts at.
struct Record<'a> {
    start: usize,
    key: &'a str,
    value: &'a str,
}

/// The record at `pos`, and the offset after it; `None` at the end.
fn next_record(text: &str, pos: usize) -> Result<Option<(Record<'_>, usize)>, CheckpointError> {
    let rest = &text[pos..];
    if rest.is_empty() {
        return Ok(None);
    }
    let equals = rest.find('=').ok_or(CheckpointError::Truncated)?;
    let key = &rest[..equals];
    let key_ok = !key.is_empty()
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        });
    if !key_ok {
        return Err(CheckpointError::BadKey);
    }
    let after_key = &rest[equals + 1..];
    let colon = after_key.find(':').ok_or(CheckpointError::Truncated)?;
    let digits = &after_key[..colon];
    let canonical = !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    let len: usize = if canonical {
        digits.parse().map_err(|_| CheckpointError::BadLength)?
    } else {
        return Err(CheckpointError::BadLength);
    };
    if len > RECORD_MAX_BYTES {
        return Err(CheckpointError::BadLength);
    }
    let value_start = pos + equals + 1 + colon + 1;
    let value_end = value_start
        .checked_add(len)
        .ok_or(CheckpointError::BadLength)?;
    if value_end > text.len() {
        return Err(CheckpointError::Truncated);
    }
    if !text.is_char_boundary(value_end) {
        return Err(CheckpointError::BadLength);
    }
    match text.as_bytes().get(value_end) {
        None => return Err(CheckpointError::Truncated),
        Some(b'\n') => {}
        Some(_) => return Err(CheckpointError::MissingNewline),
    }
    let record = Record {
        start: pos,
        key,
        value: &text[value_start..value_end],
    };
    Ok(Some((record, value_end + 1)))
}

/// Every record of `text`, in order.
fn records(text: &str) -> Result<Vec<Record<'_>>, CheckpointError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((record, next)) = next_record(text, pos)? {
        out.push(record);
        pos = next;
    }
    Ok(out)
}

/// Split a sealed checkpoint into its body and the digest its last record
/// claims. The caller recomputes the body's digest and compares.
///
/// # Errors
///
/// A [`CheckpointError`] for malformed records,
/// [`CheckpointError::MissingField`] when the last record is not
/// `integrity`, [`CheckpointError::InvalidValue`] when it is not
/// `sha256:<64 lowercase hex>`.
pub fn split_sealed(text: &str) -> Result<(&str, Sha256Digest), CheckpointError> {
    let all = records(text)?;
    let last = all
        .last()
        .filter(|record| record.key == "integrity")
        .ok_or(CheckpointError::MissingField("integrity"))?;
    let digest = last
        .value
        .strip_prefix("sha256:")
        .and_then(lowercase_digest)
        .ok_or(CheckpointError::InvalidValue("integrity"))?;
    Ok((&text[..last.start], digest))
}

fn lowercase_digest(hex: &str) -> Option<Sha256Digest> {
    Sha256Digest::from_hex(hex).filter(|digest| digest.to_hex() == hex)
}

/// The known single-valued records, keyed by name, as a decoder collects
/// them.
#[derive(Default)]
struct Known<'a> {
    created: Option<&'a str>,
    project: Option<&'a str>,
    outgoing_adapter: Option<&'a str>,
    outgoing_run: Option<&'a str>,
    outgoing_terminal: Option<&'a str>,
    target_adapter: Option<&'a str>,
    goal: Option<&'a str>,
    summary: Option<&'a str>,
    definition_path: Option<&'a str>,
    definition_digest: Option<&'a str>,
    inventory: Option<&'a str>,
}

impl<'a> Known<'a> {
    fn slot(&mut self, key: &str) -> Option<(&'static str, &mut Option<&'a str>)> {
        Some(match key {
            "created" => ("created", &mut self.created),
            "project" => ("project", &mut self.project),
            "outgoing.adapter" => ("outgoing.adapter", &mut self.outgoing_adapter),
            "outgoing.run" => ("outgoing.run", &mut self.outgoing_run),
            "outgoing.terminal" => ("outgoing.terminal", &mut self.outgoing_terminal),
            "target.adapter" => ("target.adapter", &mut self.target_adapter),
            "goal" => ("goal", &mut self.goal),
            "summary" => ("summary", &mut self.summary),
            "definition.path" => ("definition.path", &mut self.definition_path),
            "definition.digest" => ("definition.digest", &mut self.definition_digest),
            "inventory" => ("inventory", &mut self.inventory),
            _ => return None,
        })
    }
}

fn required<'a>(value: Option<&'a str>, name: &'static str) -> Result<&'a str, CheckpointError> {
    value.ok_or(CheckpointError::MissingField(name))
}

fn parse_schema(value: &str) -> Result<(u32, u32), CheckpointError> {
    let invalid = CheckpointError::InvalidValue("schema");
    let version = value
        .strip_prefix(SCHEMA_NAME)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| invalid.clone())?;
    let (major, minor) = version.split_once('.').ok_or_else(|| invalid.clone())?;
    let number = |text: &str| -> Result<u32, CheckpointError> {
        let value: u32 = text.parse().map_err(|_| invalid.clone())?;
        if value.to_string() == text {
            Ok(value)
        } else {
            Err(invalid.clone())
        }
    };
    Ok((number(major)?, number(minor)?))
}

fn parse_terminal(token: &str) -> Result<Option<ProcessTerminalState>, CheckpointError> {
    Ok(match token {
        "none" | "not-terminal" => None,
        "exited" => Some(ProcessTerminalState::Exited { code: None }),
        "killed" => Some(ProcessTerminalState::Killed),
        "orphan-risk-uncertain" => Some(ProcessTerminalState::OrphanRiskUncertain),
        other => {
            let code = other
                .strip_prefix("exited:")
                .and_then(|code| {
                    code.parse::<i32>()
                        .ok()
                        .filter(|parsed| parsed.to_string() == code)
                })
                .ok_or(CheckpointError::InvalidValue("outgoing.terminal"))?;
            Some(ProcessTerminalState::Exited { code: Some(code) })
        }
    })
}

fn parse_entry(value: &str) -> Result<InventoryEntry, CheckpointError> {
    let invalid = CheckpointError::InvalidValue("file");
    let (hex, rest) = value.split_once(' ').ok_or_else(|| invalid.clone())?;
    let (size, path) = rest.split_once(' ').ok_or_else(|| invalid.clone())?;
    let digest = lowercase_digest(hex).ok_or_else(|| invalid.clone())?;
    let size: u64 = size
        .parse()
        .ok()
        .filter(|parsed: &u64| parsed.to_string() == size)
        .ok_or_else(|| invalid.clone())?;
    let path = RootRelativePath::new(path).map_err(|_| invalid)?;
    Ok(InventoryEntry { path, digest, size })
}

impl SwitchCheckpoint {
    /// The body: every record but `integrity`, known fields in a fixed
    /// order, then the fields a newer minor added, in the order read.
    #[must_use]
    pub fn encode_body(&self) -> String {
        let facts = &self.facts;
        let mut out = String::new();
        push_record(
            &mut out,
            "schema",
            &format!("{SCHEMA_NAME}/{MAJOR}.{}", self.minor),
        );
        push_record(&mut out, "created", &facts.created_unix_seconds.to_string());
        push_record(&mut out, "project", &facts.project.to_hex());
        push_record(
            &mut out,
            "outgoing.adapter",
            facts.outgoing.adapter.as_str(),
        );
        if let Some(run) = &facts.outgoing.run {
            push_record(&mut out, "outgoing.run", run.as_str());
        }
        let terminal = terminal_token(facts.outgoing.run.as_ref(), facts.outgoing.terminal);
        push_record(&mut out, "outgoing.terminal", &terminal);
        push_record(&mut out, "target.adapter", facts.target.as_str());
        if let Some(goal) = &facts.goal {
            push_record(&mut out, "goal", goal.as_str());
        }
        push_record(&mut out, "summary", "absent");
        if let Some(definition) = &facts.definition {
            push_record(&mut out, "definition.path", definition.path());
            push_record(&mut out, "definition.digest", definition.digest());
        }
        let completeness = if facts.inventory.is_complete() {
            "complete"
        } else {
            "capped"
        };
        push_record(&mut out, "inventory", completeness);
        for entry in facts.inventory.entries() {
            let line = format!("{} {} {}", entry.digest.to_hex(), entry.size, entry.path);
            push_record(&mut out, "file", &line);
        }
        for (key, value) in &self.extra {
            push_record(&mut out, key, value);
        }
        out
    }

    /// Decode a body [`Self::encode_body`] wrote, at any minor of
    /// [`MAJOR`]. The caller has already checked the seal.
    ///
    /// # Errors
    ///
    /// A [`CheckpointError`] naming the first problem found.
    pub fn decode_body(body: &str) -> Result<Self, CheckpointError> {
        let all = records(body)?;
        let (schema, rest) = all
            .split_first()
            .ok_or(CheckpointError::MissingField("schema"))?;
        if schema.key != "schema" {
            return Err(CheckpointError::SchemaNotFirst);
        }
        let (major, minor) = parse_schema(schema.value)?;
        if major != MAJOR {
            return Err(CheckpointError::UnsupportedMajor(major));
        }
        let mut known = Known::default();
        let mut files = Vec::new();
        let mut extra = Vec::new();
        for record in rest {
            match record.key {
                "schema" => return Err(CheckpointError::DuplicateField("schema")),
                "integrity" => return Err(CheckpointError::IntegrityInBody),
                "file" => files.push(parse_entry(record.value)?),
                key => match known.slot(key) {
                    Some((name, slot)) => {
                        if slot.replace(record.value).is_some() {
                            return Err(CheckpointError::DuplicateField(name));
                        }
                    }
                    None => extra.push((key.to_string(), record.value.to_string())),
                },
            }
        }
        let facts = Self::decode_facts(&known, files)?;
        let mut checkpoint = Self::new(facts)?;
        checkpoint.minor = minor;
        checkpoint.extra = extra;
        Ok(checkpoint)
    }

    fn decode_facts(
        known: &Known<'_>,
        files: Vec<InventoryEntry>,
    ) -> Result<CheckpointFacts, CheckpointError> {
        let created = required(known.created, "created")?;
        let created_unix_seconds = created
            .parse::<u64>()
            .ok()
            .filter(|parsed| parsed.to_string() == created)
            .ok_or(CheckpointError::InvalidValue("created"))?;
        let project = lowercase_digest(required(known.project, "project")?)
            .ok_or(CheckpointError::InvalidValue("project"))?;
        let adapter = AdapterId::parse(required(known.outgoing_adapter, "outgoing.adapter")?)
            .ok_or(CheckpointError::InvalidValue("outgoing.adapter"))?;
        let run = known
            .outgoing_run
            .map(|run| RunId::new(run).map_err(|_| CheckpointError::InvalidValue("outgoing.run")))
            .transpose()?;
        let token = required(known.outgoing_terminal, "outgoing.terminal")?;
        let terminal = parse_terminal(token)?;
        if terminal_token(run.as_ref(), terminal) != token {
            return Err(CheckpointError::InvalidValue("outgoing.terminal"));
        }
        let target = AdapterId::parse(required(known.target_adapter, "target.adapter")?)
            .ok_or(CheckpointError::InvalidValue("target.adapter"))?;
        let goal = known
            .goal
            .map(|goal| AgentPrompt::new(goal).map_err(|_| CheckpointError::InvalidValue("goal")))
            .transpose()?;
        if required(known.summary, "summary")? != "absent" {
            return Err(CheckpointError::InvalidValue("summary"));
        }
        let definition = match (known.definition_path, known.definition_digest) {
            (None, None) => None,
            (Some(_), None) => return Err(CheckpointError::MissingField("definition.digest")),
            (None, Some(_)) => return Err(CheckpointError::MissingField("definition.path")),
            (Some(path), Some(digest)) => Some(
                DefinitionReference::new(path, digest)
                    .map_err(|_| CheckpointError::InvalidValue("definition.path"))?,
            ),
        };
        let complete = match required(known.inventory, "inventory")? {
            "complete" => true,
            "capped" => false,
            _ => return Err(CheckpointError::InvalidValue("inventory")),
        };
        let inventory =
            Inventory::new(files, complete).map_err(|_| CheckpointError::InvalidValue("file"))?;
        Ok(CheckpointFacts {
            created_unix_seconds,
            project,
            outgoing: OutgoingRun {
                adapter,
                run,
                terminal,
            },
            target,
            goal,
            definition,
            inventory,
        })
    }
}

/// `goal` with its secret-shaped values redacted, cut to
/// [`AgentPrompt::MAX_BYTES`] at a character boundary. `None` only if the
/// result is not a valid prompt, which never keeps the unscrubbed text.
fn scrubbed_goal(goal: &AgentPrompt) -> Option<AgentPrompt> {
    let mut text = scrub_secrets(goal.as_str());
    if text.len() > AgentPrompt::MAX_BYTES {
        let mut end = AgentPrompt::MAX_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    AgentPrompt::new(text).ok()
}
