//! The `HarnessAdapter` port: describe a built-in adapter, turn a launch
//! request into a concrete [`LaunchPlan`], and parse one line of captured
//! output into an [`AdapterEvent`] (spike slice 3, `docs/spike-log.md` §
//! Slice 3).
//!
//! No program path or argument vector for an adapter launch ever crosses
//! IPC beyond a closed [`AdapterId`] and a size-capped [`AgentPrompt`]: the
//! actual argv an adapter's process receives comes only from that
//! adapter's own, fixed `argv_template` -- never from request text
//! (`docs/spike-log.md` § IPC contract's own discipline, extended here to
//! adapters).

use std::path::{Path, PathBuf};

pub use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, AgentPhase, AgentPrompt, PromptChannel, PromptError, StdinPlan,
    ToolCallProposal, TransportClass, WorkspaceRoot, WorkspaceRootError,
};
use omnifrons_domain::scope::ScopeMode;

/// Static metadata describing one built-in adapter -- never a live
/// process, never launch-time state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterDescriptor {
    /// This adapter's closed identifier.
    pub id: AdapterId,
    /// A human-readable display name.
    pub display_name: String,
    /// This adapter's transport class.
    pub transport_class: TransportClass,
    /// How this adapter's process receives the agent prompt.
    pub prompt_channel: PromptChannel,
    /// The fixed argument vector this adapter's process is launched with.
    /// Never includes the prompt or any request-supplied text -- only this
    /// adapter's own, fixed template.
    pub argv_template: Vec<String>,
    /// Extra environment variable names this adapter declares it needs,
    /// beyond the fixed base allowlist (see [`EnvPlan::new`]). A
    /// secret-shaped name here is refused at plan-build time
    /// ([`LaunchPlanError::SecretShapedEnvKey`]).
    pub declared_env: Vec<String>,
    /// The scope-enforcement mode this adapter runs under. Always
    /// [`ScopeMode::Advisory`] for a built-in adapter in this slice --
    /// never presented as a security boundary
    /// (`docs/target-architecture.md` § Proposed invariants, item 5).
    pub scope_mode: ScopeMode,
    /// Free-text notes surfaced alongside this descriptor (e.g. caveats
    /// about credentials or CI exercise).
    pub notes: String,
}

/// A validated request to launch an adapter's process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// The agent prompt to deliver.
    pub prompt: AgentPrompt,
    /// The workspace directory the adapter's process runs in.
    pub workspace: WorkspaceRoot,
}

/// The base set of environment variable names always allowed through an
/// adapter launch, regardless of what the adapter declares.
///
/// `PATH` always; `LANG`/`LC_ALL`/`TMPDIR` always (harmless if unset on a
/// given platform); and, platform-specific, either `HOME` (unix) or
/// `USERPROFILE`/`SystemRoot`/`SystemDrive`/`TEMP`/`TMP` (windows) --
/// matching `docs/spike-log.md` § Slice 3's own listing of the base
/// allowlist.
fn base_env_keys() -> Vec<&'static str> {
    let mut keys = vec!["PATH", "LANG", "LC_ALL", "TMPDIR"];
    if cfg!(windows) {
        keys.extend(["USERPROFILE", "SystemRoot", "SystemDrive", "TEMP", "TMP"]);
    } else {
        keys.push("HOME");
    }
    keys
}

/// Whether `key` looks like it names a secret, case-insensitively:
/// `*_TOKEN`, `*_SECRET`, `*_KEY`, `*_PASSWORD`, `ANTHROPIC_API_KEY`,
/// `AWS_*`, `GH_TOKEN`, `GITHUB_TOKEN` -- refused even when an adapter
/// explicitly declares it ([`EnvPlan::new`]), and re-checked again by
/// `omnifrons-supervisor` immediately before actually setting an
/// environment variable on a spawned child (a single choke point:
/// whatever an [`EnvPlan`] says, the supervisor never sets a
/// secret-shaped key on a real child).
///
/// `ANTHROPIC_API_KEY`, `GH_TOKEN`, and `GITHUB_TOKEN` are already covered
/// by the `_KEY`/`_TOKEN` suffix rules; they are matched explicitly too,
/// verbatim, so the rule set reads as a direct transcription of
/// `docs/spike-log.md` § Slice 3's own listing rather than relying on a
/// reader noticing the overlap.
#[must_use]
pub fn is_secret_shaped(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_KEY")
        || upper.ends_with("_PASSWORD")
        || upper == "ANTHROPIC_API_KEY"
        || upper.starts_with("AWS_")
        || upper == "GH_TOKEN"
        || upper == "GITHUB_TOKEN"
}

/// How a spawned process's environment is built.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EnvPlan {
    /// Inherit the parent process's entire environment unmodified. Used by
    /// `ProcessSpec::new`'s default, preserving the demo-harness launch
    /// path's existing behavior (`docs/spike-log.md` § Slice 3): the demo
    /// harness is this application's own binary, not a caller-supplied
    /// executable, so full inheritance carries none of the risk an
    /// adapter's process would.
    #[default]
    Inherit,
    /// Clear the child's environment, then set only these allowlisted key
    /// names, each resolved from the parent process's own environment at
    /// spawn time (never a value captured earlier). Built only via
    /// [`EnvPlan::new`], which merges the fixed base allowlist
    /// ([`base_env_keys`]) with an adapter's own declared keys and refuses
    /// any secret-shaped name outright.
    Allowlist(Vec<String>),
}

impl EnvPlan {
    /// Build an allowlist plan from `declared` extra keys, merged with the
    /// fixed base allowlist ([`base_env_keys`]), deduplicated and sorted
    /// for a stable, comparable shape.
    ///
    /// # Errors
    ///
    /// Returns [`LaunchPlanError::SecretShapedEnvKey`] if any declared key
    /// looks secret-shaped ([`is_secret_shaped`]) -- refused even though
    /// the adapter itself asked for it.
    pub fn new(declared: &[String]) -> Result<Self, LaunchPlanError> {
        let mut keys: Vec<String> = base_env_keys().into_iter().map(str::to_string).collect();
        for key in declared {
            if is_secret_shaped(key) {
                return Err(LaunchPlanError::SecretShapedEnvKey(key.clone()));
            }
            keys.push(key.clone());
        }
        keys.sort();
        keys.dedup();
        Ok(Self::Allowlist(keys))
    }

    /// The allowlisted key names this plan carries, or an empty slice for
    /// [`Self::Inherit`] (which has no closed key list -- everything is
    /// inherited).
    #[must_use]
    pub fn allowed_keys(&self) -> &[String] {
        match self {
            Self::Inherit => &[],
            Self::Allowlist(keys) => keys,
        }
    }
}

/// Why [`HarnessAdapter::build_launch`] could not produce a [`LaunchPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlanError {
    /// A declared (or otherwise requested) environment variable name looks
    /// secret-shaped ([`is_secret_shaped`]) and was refused, carrying the
    /// offending key name.
    SecretShapedEnvKey(String),
    /// A candidate working directory's canonical form is not the
    /// workspace root itself.
    CwdOutsideWorkspace,
    /// The adapter's declared [`PromptChannel`] is not one this
    /// implementation can build a plan for.
    PromptChannelUnsupported,
}

impl std::fmt::Display for LaunchPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretShapedEnvKey(key) => {
                write!(
                    f,
                    "environment variable {key} looks secret-shaped and was refused"
                )
            }
            Self::CwdOutsideWorkspace => {
                f.write_str("the working directory resolves outside the workspace")
            }
            Self::PromptChannelUnsupported => {
                f.write_str("this adapter's prompt channel is not supported")
            }
        }
    }
}

impl std::error::Error for LaunchPlanError {}

/// A concrete plan to launch one adapter process: never containing more
/// than what a `HarnessAdapter` implementation's own fixed template and
/// the caller's validated workspace/prompt allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The exact argument vector to launch with -- always exactly the
    /// adapter's own `argv_template`, never request-supplied text.
    pub argv: Vec<String>,
    /// The environment the spawned process receives.
    pub env: EnvPlan,
    /// The working directory the spawned process runs in.
    pub cwd: WorkspaceRoot,
    /// How the spawned process's stdin is configured.
    pub stdin: StdinPlan,
    /// The prompt to deliver, when `stdin` is
    /// [`StdinPlan::PipePromptThenClose`].
    pub prompt: Option<AgentPrompt>,
    /// The scope-enforcement mode this launch runs under.
    pub scope_mode: ScopeMode,
}

/// Validate that `candidate_cwd`, once canonicalized, is exactly
/// `workspace`'s own canonical path -- never a parent, sibling, or a
/// symlinked-away directory.
///
/// A guard for future callers whose candidate working directory might
/// differ from the workspace: in this slice, every built-in adapter's
/// `build_launch` always passes the workspace's own path as the candidate,
/// so this call always trivially succeeds there
/// (`docs/spike-log.md` § Slice 3).
///
/// # Errors
///
/// Returns [`LaunchPlanError::CwdOutsideWorkspace`] if `candidate_cwd`
/// cannot be canonicalized, or if its canonical form differs from
/// `workspace`'s own.
pub fn validate_cwd_within_workspace(
    candidate_cwd: &Path,
    workspace: &WorkspaceRoot,
) -> Result<PathBuf, LaunchPlanError> {
    let canonical =
        std::fs::canonicalize(candidate_cwd).map_err(|_| LaunchPlanError::CwdOutsideWorkspace)?;
    if canonical == workspace.path() {
        Ok(canonical)
    } else {
        Err(LaunchPlanError::CwdOutsideWorkspace)
    }
}

/// A port for a built-in harness adapter: describe itself, build a launch
/// plan for a validated request, and parse one captured line of its
/// process's output.
pub trait HarnessAdapter {
    /// This adapter's static descriptor. Must be stable: repeated calls
    /// return the same value.
    fn describe(&self) -> AdapterDescriptor;

    /// Build a concrete launch plan for `request`.
    ///
    /// # Errors
    ///
    /// Returns [`LaunchPlanError`] if the plan cannot be built (a
    /// secret-shaped declared environment key, an unsupported prompt
    /// channel, or a working directory outside the workspace).
    fn build_launch(&self, request: &LaunchRequest) -> Result<LaunchPlan, LaunchPlanError>;

    /// Parse one line of this adapter's captured output into the
    /// [`AdapterEvent`]s it carries, in order.
    ///
    /// Total and never empty: every implementation must return at least
    /// one `AdapterEvent` for every possible `&str` input (including
    /// empty, malformed, or otherwise unparsable text) -- never panic.
    /// Unparsable input maps to exactly one [`AdapterEvent::Unknown`],
    /// carrying `line`'s own bytes verbatim. A single line may
    /// legitimately carry several events (R3-004: an assistant message
    /// with several content blocks yields one event per block).
    fn parse_line(&self, line: &str) -> Vec<AdapterEvent>;
}

/// The closed, built-in catalog of adapter instances a shell may launch.
///
/// Populated once, at construction, from a fixed set of adapters (e.g.
/// `omnifrons_adapters::line_agent::catalog()`) -- never a dynamic
/// registration API. `omnifrons-app` itself never depends on
/// `omnifrons-adapters` (`docs/repository-layout.md` § Crate map), so the
/// concrete adapters this catalog holds are always supplied by a caller
/// (the shell) that depends on both.
pub struct AdapterCatalog {
    adapters: std::collections::HashMap<AdapterId, Box<dyn HarnessAdapter + Send + Sync>>,
}

impl AdapterCatalog {
    /// Build a catalog from `adapters`, keyed by each one's own
    /// [`AdapterDescriptor::id`].
    #[must_use]
    pub fn new(adapters: Vec<Box<dyn HarnessAdapter + Send + Sync>>) -> Self {
        let mut map = std::collections::HashMap::new();
        for adapter in adapters {
            let id = adapter.describe().id;
            map.insert(id, adapter);
        }
        Self { adapters: map }
    }

    /// Look up the adapter registered under `id`, if any.
    #[must_use]
    pub fn get(&self, id: &AdapterId) -> Option<&(dyn HarnessAdapter + Send + Sync)> {
        self.adapters.get(id).map(AsRef::as_ref)
    }

    /// Every descriptor in this catalog, in an unspecified order -- used
    /// by the shell's `adapters_list` command, which returns descriptor
    /// metadata only (no argv, no env values) for every built-in adapter.
    #[must_use]
    pub fn descriptors(&self) -> Vec<AdapterDescriptor> {
        self.adapters
            .values()
            .map(|adapter| adapter.describe())
            .collect()
    }
}

/// The maximum bytes [`LineAssembler`] accumulates for one logical line
/// before reporting it truncated.
pub const LINE_ASSEMBLER_CAP_BYTES: usize = 64 * 1024;

/// The content of one logical line a [`LineAssembler`] completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssembledLine {
    /// A complete logical line, ready for `HarnessAdapter::parse_line`.
    Line(String),
    /// The line is not faithfully reassembled -- it exceeded
    /// [`LINE_ASSEMBLER_CAP_BYTES`] before it ended, raw frames were
    /// dropped while it was still being assembled, or its terminating
    /// frame never arrived ([`LineAssembler::finish`]); `raw` carries the
    /// (capped) bytes actually captured. Never hand `raw` to `parse_line`
    /// as if it were a complete line.
    Truncated {
        /// The captured, capped raw bytes.
        raw: Vec<u8>,
    },
}

/// One completed logical line plus the drop accounting for every raw
/// frame that fed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// The assembled content.
    pub line: AssembledLine,
    /// The sum of `dropped_before` over every raw frame that fed this line
    /// (R3-002: never only the last frame's own count), reset per emitted
    /// line. A consumer that emits one wire frame per assembled line
    /// carries this on that frame; a consumer that emits several attaches
    /// it once, to the first.
    pub dropped_before: u64,
}

/// Joins consecutive captured text frames of one output stream back into
/// logical lines, bounded by [`LINE_ASSEMBLER_CAP_BYTES`].
///
/// `omnifrons-supervisor`'s own output-capture framing
/// (`crates/omnifrons-supervisor/src/output_capture.rs`) splits a single
/// logical line into consecutive frames once it exceeds
/// [`omnifrons_domain::output::MAX_TEXT_FRAME_BYTES`], flagging every frame
/// but the last of such a split `continued: true`
/// (`omnifrons_domain::output::FramePayload::Text`). Continuation is decided
/// from that flag alone, never from a frame's length: a genuine line whose
/// length happens to land exactly on the per-frame cap arrives
/// `continued: false` and is its own line (R3-003 closed the earlier
/// length heuristic's false positive; `docs/spike-log.md` § Slice 3).
pub struct LineAssembler {
    buffer: Vec<u8>,
    truncated: bool,
    /// A `continued` frame has been pushed and its terminator has not
    /// arrived yet.
    pending: bool,
    /// Accumulated `dropped_before` over the frames of the current line.
    dropped_before: u64,
}

impl Default for LineAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl LineAssembler {
    /// Build an assembler with an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            truncated: false,
            pending: false,
            dropped_before: 0,
        }
    }

    /// Feed one raw text frame: its decoded `text`, its `continued` flag,
    /// and its own `dropped_before` count.
    ///
    /// Returns `None` while `continued` says more of the same line follows;
    /// returns `Some` once a `continued: false` frame ends the line,
    /// carrying the assembled [`AssembledLine`] and the sum of
    /// `dropped_before` over every frame that fed it. Bytes beyond
    /// [`LINE_ASSEMBLER_CAP_BYTES`] are dropped as they arrive, so this
    /// never grows its internal buffer past the cap, and the line is
    /// reported [`AssembledLine::Truncated`]. A nonzero `dropped_before`
    /// arriving while a line is still pending also marks it truncated:
    /// frames of that line (or of the lines a lost terminator separated it
    /// from) are missing, so the bytes on hand are not that line -- from
    /// that point nothing more is appended, and `raw` is the contiguous
    /// prefix captured before the gap. Either way the emitted
    /// `dropped_before` is the sum over every frame pushed for the line.
    pub fn push(&mut self, text: &str, continued: bool, dropped_before: u64) -> Option<Assembled> {
        if dropped_before > 0 && self.pending {
            self.truncated = true;
        }
        self.dropped_before = self.dropped_before.saturating_add(dropped_before);

        if !self.truncated {
            if self.buffer.len() + text.len() > LINE_ASSEMBLER_CAP_BYTES {
                self.truncated = true;
            } else {
                self.buffer.extend_from_slice(text.as_bytes());
            }
        }

        if continued {
            self.pending = true;
            return None;
        }
        Some(self.take())
    }

    /// Flush a pending partial line whose terminating frame never arrived
    /// (the stream ended, or the process reached its terminal state, after
    /// a `continued` frame) as [`AssembledLine::Truncated`], with its
    /// accumulated drop count. `None` if nothing is pending; idempotent.
    pub fn finish(&mut self) -> Option<Assembled> {
        if !self.pending {
            return None;
        }
        self.truncated = true;
        Some(self.take())
    }

    /// Emit the current line and reset every per-line field.
    fn take(&mut self) -> Assembled {
        let raw = std::mem::take(&mut self.buffer);
        let dropped_before = std::mem::take(&mut self.dropped_before);
        self.pending = false;
        let line = if std::mem::take(&mut self.truncated) {
            AssembledLine::Truncated { raw }
        } else {
            AssembledLine::Line(String::from_utf8_lossy(&raw).into_owned())
        };
        Assembled {
            line,
            dropped_before,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterCatalog, AdapterDescriptor, AdapterEvent, AdapterId, AgentPhase, AgentPrompt,
        Assembled, AssembledLine, EnvPlan, HarnessAdapter, LaunchPlan, LaunchPlanError,
        LaunchRequest, LineAssembler, PromptChannel, StdinPlan, TransportClass, WorkspaceRoot,
        is_secret_shaped, validate_cwd_within_workspace,
    };
    use omnifrons_domain::scope::ScopeMode;

    // -- is_secret_shaped --

    #[test]
    fn secret_shaped_keys_are_detected_case_insensitively() {
        for key in [
            "MY_TOKEN",
            "my_token",
            "some_secret",
            "API_KEY",
            "DB_PASSWORD",
            "ANTHROPIC_API_KEY",
            "AWS_ACCESS_KEY_ID",
            "GH_TOKEN",
            "GITHUB_TOKEN",
        ] {
            assert!(
                is_secret_shaped(key),
                "{key} must be detected as secret-shaped"
            );
        }
    }

    #[test]
    fn ordinary_keys_are_not_secret_shaped() {
        for key in ["PATH", "HOME", "LANG", "MY_APP_CONFIG_DIR"] {
            assert!(
                !is_secret_shaped(key),
                "{key} must not be flagged as secret-shaped"
            );
        }
    }

    // -- EnvPlan --

    #[test]
    fn env_plan_new_merges_base_and_declared_keys() {
        let plan =
            EnvPlan::new(&["CUSTOM_VAR".to_string()]).expect("declared key must be accepted");
        let keys = plan.allowed_keys();
        assert!(keys.contains(&"PATH".to_string()));
        assert!(keys.contains(&"CUSTOM_VAR".to_string()));
    }

    #[test]
    fn env_plan_new_refuses_secret_shaped_declared_key() {
        let error = EnvPlan::new(&["SOME_SECRET".to_string()]).unwrap_err();
        assert_eq!(
            error,
            LaunchPlanError::SecretShapedEnvKey("SOME_SECRET".to_string())
        );
    }

    // -- validate_cwd_within_workspace --

    #[test]
    fn cwd_matching_workspace_is_accepted() {
        let dir = std::env::temp_dir();
        let workspace = WorkspaceRoot::new(&dir).expect("temp dir must be a valid workspace");
        let result = validate_cwd_within_workspace(&dir, &workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn cwd_outside_workspace_is_rejected() {
        let dir = std::env::temp_dir();
        let workspace = WorkspaceRoot::new(&dir).expect("temp dir must be a valid workspace");
        let elsewhere = std::path::PathBuf::from("/");
        let result = validate_cwd_within_workspace(&elsewhere, &workspace);
        assert_eq!(result.unwrap_err(), LaunchPlanError::CwdOutsideWorkspace);
    }

    // -- LineAssembler --

    /// An intact assembled line carrying `dropped_before`.
    fn line(text: &str, dropped_before: u64) -> Assembled {
        Assembled {
            line: AssembledLine::Line(text.to_string()),
            dropped_before,
        }
    }

    #[test]
    fn line_assembler_passes_through_a_short_single_frame() {
        let mut assembler = LineAssembler::new();
        let result = assembler.push("hello", false, 0);
        assert_eq!(result, Some(line("hello", 0)));
    }

    /// R3-003 (b): a split line is reassembled from the `continued` flag:
    /// a full-cap continued frame, then a short terminator.
    #[test]
    fn line_assembler_joins_a_continuation_then_a_terminator() {
        let continuation = "a".repeat(omnifrons_domain::output::MAX_TEXT_FRAME_BYTES);
        let mut assembler = LineAssembler::new();
        assert_eq!(
            assembler.push(&continuation, true, 0),
            None,
            "a continued frame must not complete a line yet"
        );
        let result = assembler.push("tail", false, 0);
        let expected = format!("{continuation}tail");
        assert_eq!(result, Some(line(&expected, 0)));
    }

    /// R3-003 (a): a genuine line of exactly the per-frame cap's length,
    /// flagged `continued: false`, is its own line -- never merged with
    /// the line that follows it.
    #[test]
    fn line_assembler_emits_an_exactly_cap_frame_as_its_own_line_when_not_continued() {
        let exactly_cap = "a".repeat(omnifrons_domain::output::MAX_TEXT_FRAME_BYTES);
        let mut assembler = LineAssembler::new();
        assert_eq!(
            assembler.push(&exactly_cap, false, 0),
            Some(line(&exactly_cap, 0)),
            "an exactly-cap frame that is not continued must complete its own line"
        );
        assert_eq!(
            assembler.push("next", false, 0),
            Some(line("next", 0)),
            "the following line must not have been merged into the exactly-cap one"
        );
    }

    /// R3-003: continuation is decided by the flag alone, never by length
    /// -- a short frame flagged continued is still a continuation.
    #[test]
    fn line_assembler_treats_a_short_continued_frame_as_a_continuation() {
        let mut assembler = LineAssembler::new();
        assert_eq!(assembler.push("abc", true, 0), None);
        assert_eq!(assembler.push("def", false, 0), Some(line("abcdef", 0)));
    }

    /// R3-002: `dropped_before` on the frame that *starts* a line (frames
    /// lost before this line began, so the line itself is intact) is
    /// carried on the assembled line even though the terminating frame's
    /// own count is zero -- and reset once the line is emitted.
    #[test]
    fn line_assembler_carries_a_leading_frames_dropped_before_onto_the_assembled_line() {
        let mut assembler = LineAssembler::new();
        assert_eq!(assembler.push("first", true, 2), None);
        assert_eq!(
            assembler.push("end", false, 0),
            Some(line("firstend", 2)),
            "the emitted line must carry the continuation frame's dropped_before, not the \
             terminator's zero"
        );
        assert_eq!(
            assembler.push("fresh", false, 0),
            Some(line("fresh", 0)),
            "the count must reset once a line has been emitted"
        );
    }

    #[test]
    fn line_assembler_reports_truncated_once_the_cap_is_exceeded() {
        let mut assembler = LineAssembler::new();
        let chunk = "a".repeat(omnifrons_domain::output::MAX_TEXT_FRAME_BYTES);
        // Push enough full-cap continuation frames to exceed the 64 KiB
        // assembler cap, then a short terminator.
        for _ in 0..9 {
            assert_eq!(assembler.push(&chunk, true, 0), None);
        }
        let result = assembler.push("end", false, 0);
        match result {
            Some(Assembled {
                line: AssembledLine::Truncated { raw },
                dropped_before: 0,
            }) => {
                assert!(
                    raw.len() <= super::LINE_ASSEMBLER_CAP_BYTES,
                    "truncated raw bytes must never exceed the assembler cap"
                );
            }
            other => panic!("expected Truncated with no drops, got {other:?}"),
        }
    }

    /// R3-002: frames lost (a nonzero `dropped_before`) while a line is
    /// still being assembled mean the bytes on hand are not that line:
    /// it is reported truncated (so it is never handed to `parse_line` as
    /// if complete), `raw` is the contiguous prefix captured before the
    /// gap, and `dropped_before` is the SUM over every frame that fed it.
    #[test]
    fn line_assembler_marks_a_line_truncated_when_frames_were_dropped_mid_line() {
        let mut assembler = LineAssembler::new();
        assert_eq!(assembler.push("head", true, 0), None);
        assert_eq!(assembler.push("middle", true, 3), None);
        assert_eq!(
            assembler.push("tail", false, 4),
            Some(Assembled {
                line: AssembledLine::Truncated {
                    raw: b"head".to_vec()
                },
                dropped_before: 7,
            }),
            "a mid-line gap must yield Truncated with the prefix before the gap and the summed \
             drop count"
        );
    }

    /// `finish` flushes a pending, unterminated partial line as truncated
    /// (its terminator never arrived), carrying its accumulated drop count;
    /// with nothing pending it is `None`, and it is idempotent.
    #[test]
    fn line_assembler_finish_flushes_a_pending_partial_line_as_truncated() {
        let mut assembler = LineAssembler::new();
        assert_eq!(
            assembler.finish(),
            None,
            "nothing pending on a fresh assembler"
        );
        assert_eq!(assembler.push("head", true, 1), None);
        assert_eq!(
            assembler.finish(),
            Some(Assembled {
                line: AssembledLine::Truncated {
                    raw: b"head".to_vec()
                },
                dropped_before: 1,
            })
        );
        assert_eq!(assembler.finish(), None, "finish must be idempotent");
        assert_eq!(assembler.push("whole", false, 0), Some(line("whole", 0)));
        assert_eq!(
            assembler.finish(),
            None,
            "a completed line leaves nothing pending"
        );
    }

    #[test]
    fn line_assembler_resets_after_completing_a_line() {
        let mut assembler = LineAssembler::new();
        assembler.push("first", false, 0);
        let second = assembler.push("second", false, 0);
        assert_eq!(second, Some(line("second", 0)));
    }

    // -- AdapterCatalog + a fake HarnessAdapter --

    struct FakeAdapter {
        id: AdapterId,
    }

    impl HarnessAdapter for FakeAdapter {
        fn describe(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                id: self.id.clone(),
                display_name: "Fake".to_string(),
                transport_class: TransportClass::StructuredStreamingCli,
                prompt_channel: PromptChannel::StdinThenClose,
                argv_template: vec![],
                declared_env: vec![],
                scope_mode: ScopeMode::Advisory,
                notes: String::new(),
            }
        }

        fn build_launch(&self, request: &LaunchRequest) -> Result<LaunchPlan, LaunchPlanError> {
            let env = EnvPlan::new(&[])?;
            Ok(LaunchPlan {
                argv: vec![],
                env,
                cwd: request.workspace.clone(),
                stdin: StdinPlan::PipePromptThenClose,
                prompt: Some(request.prompt.clone()),
                scope_mode: ScopeMode::Advisory,
            })
        }

        fn parse_line(&self, line: &str) -> Vec<AdapterEvent> {
            if line == "init" {
                vec![AdapterEvent::State {
                    phase: AgentPhase::Init,
                    observations: vec![],
                }]
            } else {
                vec![AdapterEvent::Unknown {
                    raw: line.as_bytes().to_vec(),
                    truncated: false,
                }]
            }
        }
    }

    #[test]
    fn adapter_catalog_looks_up_by_id() {
        let catalog = AdapterCatalog::new(vec![Box::new(FakeAdapter {
            id: AdapterId::stream_json_cli(),
        })]);
        assert!(catalog.get(&AdapterId::stream_json_cli()).is_some());
        assert!(catalog.get(&AdapterId::claude_code()).is_none());
    }

    #[test]
    fn fake_adapter_build_launch_carries_the_prompt_and_workspace() {
        let adapter = FakeAdapter {
            id: AdapterId::stream_json_cli(),
        };
        let workspace =
            WorkspaceRoot::new(std::env::temp_dir()).expect("temp dir must be a valid workspace");
        let prompt = AgentPrompt::new("hello").expect("short prompt must be valid");
        let request = LaunchRequest {
            prompt: prompt.clone(),
            workspace: workspace.clone(),
        };
        let plan = adapter
            .build_launch(&request)
            .expect("build_launch must succeed");
        assert_eq!(plan.prompt, Some(prompt));
        assert_eq!(plan.cwd, workspace);
    }
}
