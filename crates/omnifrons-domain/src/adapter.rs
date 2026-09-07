//! Built-in harness adapter domain types (spike slice 3, ADR-0002).
//!
//! Framework-independent (this module lives in `omnifrons-domain`, which
//! depends on `std` and `thiserror` only): it names the shapes a
//! `HarnessAdapter` port implementation (`omnifrons-app`) and a concrete
//! adapter (`omnifrons-adapters`) agree on, without committing to how a
//! process is actually launched or how its output is captured.
//!
//! `AdapterId` is closed to a built-in set -- never an arbitrary caller-
//! supplied string -- matching `docs/spike-log.md` § IPC contract's own
//! "no program path or argument vector ever crosses IPC" discipline: a
//! caller names one of a small, fixed set of built-in adapter identities,
//! never a program to launch directly.

use std::path::{Path, PathBuf};

/// The wire/API token for the generic `stream-json-cli` built-in adapter.
const STREAM_JSON_CLI_TOKEN: &str = "stream-json-cli";
/// The wire/API token for the `claude-code` built-in adapter.
const CLAUDE_CODE_TOKEN: &str = "claude-code";

/// The closed set of tokens [`AdapterId::parse`] accepts.
const KNOWN_ADAPTER_IDS: [&str; 2] = [STREAM_JSON_CLI_TOKEN, CLAUDE_CODE_TOKEN];

/// A built-in harness adapter's identifier.
///
/// Wraps a `String` for cheap comparison/hashing/display, but is
/// constructible only through [`Self::stream_json_cli`], [`Self::claude_code`],
/// or [`Self::parse`] (which itself only ever accepts one of those two
/// tokens) -- never from an arbitrary caller-supplied string. This is what
/// "closed to a built-in set" means: the field is a plain `String`, but no
/// public constructor can ever produce an `AdapterId` outside the built-in
/// set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    /// The generic `stream-json-cli` built-in adapter's id.
    #[must_use]
    pub fn stream_json_cli() -> Self {
        Self(STREAM_JSON_CLI_TOKEN.to_string())
    }

    /// The `claude-code` built-in adapter's id.
    #[must_use]
    pub fn claude_code() -> Self {
        Self(CLAUDE_CODE_TOKEN.to_string())
    }

    /// Parse `token` into an `AdapterId`, or `None` if it does not name one
    /// of the closed set of built-in adapters.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        if KNOWN_ADAPTER_IDS.contains(&token) {
            Some(Self(token.to_string()))
        } else {
            None
        }
    }

    /// This id's stable wire token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AdapterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How an adapter's underlying process communicates with the harness: a
/// line-oriented structured stream (the only transport class a built-in
/// adapter uses as of slice 3), or a pseudo-terminal (named here so the
/// closed set is stated up front, even though no built-in adapter uses it
/// yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportClass {
    /// A structured, line-delimited streaming CLI (e.g. `stream-json`).
    StructuredStreamingCli,
    /// A pseudo-terminal. Not exercised by any built-in adapter in this
    /// slice.
    Pty,
}

/// How an adapter's underlying process receives the agent prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptChannel {
    /// The prompt is written to the child's stdin, then the write end is
    /// closed.
    StdinThenClose,
    /// The prompt is passed as one of the child's arguments. Not exercised
    /// by any built-in adapter in this slice.
    Argv,
}

/// How a launched process's stdin is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StdinPlan {
    /// Stdin is `/dev/null` (or the platform equivalent): the child never
    /// receives anything on stdin.
    Null,
    /// Stdin is piped; the prompt's bytes are written, then the write end
    /// is closed (signalling EOF to the child), never left open.
    PipePromptThenClose,
}

/// Why [`AgentPrompt::new`] rejected a candidate prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptError {
    /// The prompt's UTF-8 byte length exceeds
    /// [`AgentPrompt::MAX_BYTES`].
    TooLarge,
    /// The prompt contains an interior NUL byte, which cannot be written
    /// safely to a child's stdin/argv on every platform.
    ContainsNul,
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::TooLarge => "prompt exceeds the 16 KiB size cap",
            Self::ContainsNul => "prompt contains an interior NUL byte",
        };
        f.write_str(message)
    }
}

impl std::error::Error for PromptError {}

/// A validated agent prompt: at most 16 KiB of UTF-8 text, with no interior
/// NUL byte.
///
/// Constructible only through [`Self::new`], so every live `AgentPrompt`
/// has already passed both checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentPrompt(String);

impl AgentPrompt {
    /// The maximum UTF-8 byte length a prompt may have, inclusive.
    pub const MAX_BYTES: usize = 16 * 1024;

    /// Validate and build a prompt.
    ///
    /// # Errors
    ///
    /// Returns [`PromptError::TooLarge`] if `text`'s UTF-8 byte length
    /// exceeds [`Self::MAX_BYTES`], or [`PromptError::ContainsNul`] if it
    /// contains an interior NUL byte.
    pub fn new(text: impl Into<String>) -> Result<Self, PromptError> {
        let text = text.into();
        if text.len() > Self::MAX_BYTES {
            return Err(PromptError::TooLarge);
        }
        if text.contains('\0') {
            return Err(PromptError::ContainsNul);
        }
        Ok(Self(text))
    }

    /// The validated prompt text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume this prompt, returning its validated text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

/// A proposed tool call an adapter's underlying process emitted.
///
/// `arguments_text` is the tool's raw argument payload, serialized as text
/// (e.g. the tool's own JSON `input` object, re-serialized) -- this layer
/// makes no claim about the argument shape being valid for any particular
/// tool, only that it was proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallProposal {
    /// The proposed tool's name.
    pub name: String,
    /// The proposed tool's arguments, serialized as text.
    pub arguments_text: String,
}

/// The phase an agent's [`AdapterEvent::State`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPhase {
    /// The agent process has started and reported its initial state.
    Init,
    /// The agent process reported it finished, with the given subtype
    /// (e.g. `"success"`, `"error"`).
    Finished {
        /// The reported finish subtype.
        subtype: String,
    },
    /// The underlying process itself exited. Named here for completeness
    /// of the closed phase set; no built-in adapter's `parse_line` in this
    /// slice produces this variant -- a process's own exit is reported
    /// through the supervisor's separate `state` frame
    /// (`docs/spike-log.md` § Slice 3), not through `parse_line`.
    Exited,
}

/// One event an adapter's `parse_line` (or the shell's own stderr-to-
/// `Diagnostic` mapping) produced from a captured line of a harness
/// process's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterEvent {
    /// The agent reported a state transition, with zero or more named
    /// text observations (e.g. `cwd`, `model`, `session_id`).
    State {
        /// The reported phase.
        phase: AgentPhase,
        /// Named text observations accompanying this state, in the order
        /// they were reported.
        observations: Vec<(String, String)>,
    },
    /// The agent emitted a plain text message.
    Message {
        /// The message text.
        text: String,
    },
    /// The agent proposed a tool call.
    ToolCall(ToolCallProposal),
    /// A diagnostic line, not part of the agent's own structured protocol
    /// (e.g. the harness process's stderr).
    Diagnostic {
        /// The diagnostic text.
        text: String,
    },
    /// A line (or assembled line) that could not be interpreted.
    Unknown {
        /// The exact, byte-identical raw content that could not be
        /// interpreted.
        raw: Vec<u8>,
        /// Whether `raw` was truncated (e.g. by a line-assembly size cap)
        /// before this event was produced.
        truncated: bool,
    },
}

/// Why [`WorkspaceRoot::new`] rejected a candidate path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRootError {
    /// The candidate could not be canonicalized (it does not exist, or is
    /// not readable).
    Unreadable,
    /// The canonicalized candidate is not a directory.
    NotADirectory,
}

impl std::fmt::Display for WorkspaceRootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Unreadable => "workspace path could not be resolved",
            Self::NotADirectory => "workspace path is not a directory",
        };
        f.write_str(message)
    }
}

impl std::error::Error for WorkspaceRootError {}

/// A validated, canonical filesystem directory an adapter's process runs
/// in.
///
/// Constructible only through [`Self::new`], which canonicalizes (symlinks
/// followed) and confirms the result is a directory -- so every live
/// `WorkspaceRoot` names a real, existing directory by its fully resolved
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceRoot(PathBuf);

impl WorkspaceRoot {
    /// Canonicalize `path` and confirm it is a directory.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceRootError::Unreadable`] if `path` could not be
    /// canonicalized, or [`WorkspaceRootError::NotADirectory`] if the
    /// canonicalized result is not a directory.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, WorkspaceRootError> {
        let canonical =
            std::fs::canonicalize(path.as_ref()).map_err(|_| WorkspaceRootError::Unreadable)?;
        if !canonical.is_dir() {
            return Err(WorkspaceRootError::NotADirectory);
        }
        Ok(Self(canonical))
    }

    /// This workspace's canonical path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Consume this workspace root, returning its canonical path.
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{AdapterEvent, AdapterId, AgentPrompt, PromptError};

    // -- agent_prompt --

    #[test]
    fn agent_prompt_accepts_exactly_16_kib() {
        let text = "a".repeat(AgentPrompt::MAX_BYTES);
        let prompt = AgentPrompt::new(text.clone()).expect("16 KiB must be accepted");
        assert_eq!(
            prompt.as_str(),
            text,
            "the prompt text must be preserved exactly"
        );
    }

    #[test]
    fn agent_prompt_rejects_16_kib_plus_one() {
        let text = "a".repeat(AgentPrompt::MAX_BYTES + 1);
        let error = AgentPrompt::new(text).unwrap_err();
        assert_eq!(error, PromptError::TooLarge);
    }

    #[test]
    fn agent_prompt_rejects_interior_nul() {
        let error = AgentPrompt::new("before\0after").unwrap_err();
        assert_eq!(error, PromptError::ContainsNul);
    }

    #[test]
    fn agent_prompt_preserves_text_exactly() {
        let prompt = AgentPrompt::new("hello, world").expect("plain text must be accepted");
        assert_eq!(prompt.as_str(), "hello, world");
        assert_eq!(prompt.into_string(), "hello, world");
    }

    // -- adapter.rs unit tests --

    #[test]
    fn unknown_event_preserves_raw_bytes_exactly() {
        let raw = vec![0x7B, b'"', b't', 0xFF, b'"'];
        let event = AdapterEvent::Unknown {
            raw: raw.clone(),
            truncated: false,
        };
        match event {
            AdapterEvent::Unknown { raw: got, .. } => {
                assert_eq!(got, raw, "Unknown must preserve the exact raw bytes given");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn adapter_id_closed_set_accepts_only_known_tokens() {
        assert!(AdapterId::parse("stream-json-cli").is_some());
        assert!(AdapterId::parse("claude-code").is_some());
        assert!(
            AdapterId::parse("literally-anything-else").is_none(),
            "AdapterId::parse must reject any token outside the built-in closed set"
        );
        assert!(AdapterId::parse("").is_none());
    }

    #[test]
    fn adapter_id_named_constructors_round_trip_through_parse() {
        assert_eq!(
            AdapterId::parse(AdapterId::stream_json_cli().as_str()),
            Some(AdapterId::stream_json_cli())
        );
        assert_eq!(
            AdapterId::parse(AdapterId::claude_code().as_str()),
            Some(AdapterId::claude_code())
        );
    }
}
