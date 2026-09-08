//! `LineAgent`: the first built-in `HarnessAdapter` -- a structured,
//! line-oriented `stream-json` parser shared by the two built-in
//! stream-json-shaped adapters this crate's [`catalog`] provides
//! (`docs/spike-log.md` § Slice 3).
//!
//! `build_launch` never lets request-supplied text into the child's
//! argument vector: `argv` is always exactly the descriptor's own
//! `argv_template`, and the prompt is delivered only via stdin
//! (`StdinPlan::PipePromptThenClose`), never appended to argv.
//!
//! `parse_line` recognizes three `stream-json` shapes -- `{"type":
//! "system","subtype":"init",...}`, `{"type":"assistant",...}` (one event
//! per content block, in order: `text` -> `Message`, `tool_use` ->
//! `ToolCall`, anything else -> `Unknown` with that block's own JSON), and
//! `{"type":"result",...}` -- and maps anything else (invalid JSON, an
//! unrecognized `type`, or a malformed shape) to exactly one
//! [`AdapterEvent::Unknown`], carrying the line's own bytes verbatim.

use omnifrons_app::harness_adapter::{
    AdapterDescriptor, EnvPlan, HarnessAdapter, LaunchPlan, LaunchPlanError, LaunchRequest,
    validate_cwd_within_workspace,
};
use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, AgentPhase, PromptChannel, StdinPlan, ToolCallProposal, TransportClass,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{PUBLISH_PROPOSAL_TOOL_NAME, ProposedEntry, PublishProposal};
use omnifrons_domain::scope::ScopeMode;
use serde_json::Value;

/// The generic `stream-json-cli` adapter's fixed argument vector: empty --
/// this adapter assumes the harness process is already configured (by
/// whatever launched it) to emit `stream-json` on stdout and read a prompt
/// from stdin, with no adapter-supplied flags of its own.
const STREAM_JSON_CLI_ARGV: &[&str] = &[];

/// The `claude-code` adapter's fixed argument vector: run non-interactively
/// (`-p`), emit `stream-json` with verbose framing, and never block on an
/// interactive permission prompt -- `--permission-mode dontAsk` is the
/// harness's most conservative non-interactive mode: every tool call that
/// would otherwise prompt is auto-denied by the harness itself, so only
/// actions the harness already pre-allows ever execute. Disclosed to the
/// user in the descriptor's `notes` (R1-003).
const CLAUDE_CODE_ARGV: &[&str] = &[
    "-p",
    "--output-format",
    "stream-json",
    "--verbose",
    "--permission-mode",
    "dontAsk",
];

/// A structured-streaming-CLI `HarnessAdapter`: launches with a fixed argv
/// template and no request-supplied text, delivers the prompt over stdin,
/// and parses `stream-json` lines.
pub struct LineAgent {
    descriptor: AdapterDescriptor,
}

impl LineAgent {
    /// Build a `LineAgent` over the given static descriptor.
    #[must_use]
    pub fn new(descriptor: AdapterDescriptor) -> Self {
        Self { descriptor }
    }
}

impl HarnessAdapter for LineAgent {
    fn describe(&self) -> AdapterDescriptor {
        self.descriptor.clone()
    }

    fn build_launch(&self, request: &LaunchRequest) -> Result<LaunchPlan, LaunchPlanError> {
        if self.descriptor.prompt_channel != PromptChannel::StdinThenClose {
            return Err(LaunchPlanError::PromptChannelUnsupported);
        }

        let env = EnvPlan::new(&self.descriptor.declared_env)?;

        // `build_launch` always uses the workspace's own path as the
        // working directory; this call is therefore always a no-op guard
        // here (see `validate_cwd_within_workspace`'s own doc comment) --
        // exercised directly for later callers by
        // `crates/omnifrons-app/tests/launch_plan_cwd.rs`.
        validate_cwd_within_workspace(request.workspace.path(), &request.workspace)?;

        Ok(LaunchPlan {
            argv: self.descriptor.argv_template.clone(),
            env,
            cwd: request.workspace.clone(),
            stdin: StdinPlan::PipePromptThenClose,
            prompt: Some(request.prompt.clone()),
            scope_mode: self.descriptor.scope_mode,
            // A line agent is a pipe-and-parser adapter by construction,
            // whatever its descriptor says: the supervisor branches on
            // this field to pick the pipe wiring (`docs/spike-log.md` §
            // Slice 4).
            transport: TransportClass::StructuredStreamingCli,
            output_dir: None,
        })
    }

    fn parse_line(&self, line: &str) -> Vec<AdapterEvent> {
        parse_stream_json_line(line)
    }
}

/// Build the two built-in `stream-json`-shaped adapter instances this
/// crate provides: the generic `stream-json-cli` and `claude-code`.
///
/// Both share the exact same [`LineAgent`] parser; they differ only in
/// their descriptor (argv template, declared env, notes).
#[must_use]
pub fn catalog() -> Vec<Box<dyn HarnessAdapter + Send + Sync>> {
    vec![
        Box::new(LineAgent::new(AdapterDescriptor {
            id: AdapterId::stream_json_cli(),
            display_name: "Stream-JSON CLI".to_string(),
            transport_class: TransportClass::StructuredStreamingCli,
            prompt_channel: PromptChannel::StdinThenClose,
            argv_template: STREAM_JSON_CLI_ARGV
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            declared_env: Vec::new(),
            scope_mode: ScopeMode::Advisory,
            notes: "generic stream-json CLI harness; the caller's approved executable must \
                    already be configured to emit stream-json on stdout and read a prompt \
                    from stdin"
                .to_string(),
        })),
        Box::new(LineAgent::new(AdapterDescriptor {
            id: AdapterId::claude_code(),
            display_name: "Claude Code".to_string(),
            transport_class: TransportClass::StructuredStreamingCli,
            prompt_channel: PromptChannel::StdinThenClose,
            argv_template: CLAUDE_CODE_ARGV.iter().map(|s| (*s).to_string()).collect(),
            declared_env: Vec::new(),
            scope_mode: ScopeMode::Advisory,
            notes: "credentials are harness-owned; not exercised in CI; runs with \
                    --permission-mode dontAsk, so anything that would need a confirmation \
                    is denied by the harness and not executed"
                .to_string(),
        })),
    ]
}

/// Parse one `stream-json` line into the events it carries (never empty),
/// or fall back to exactly one [`AdapterEvent::Unknown`] with `line`'s
/// own bytes, verbatim, for anything unrecognized: invalid JSON, a
/// missing/unrecognized `type` tag, or a shape this parser does not know.
fn parse_stream_json_line(line: &str) -> Vec<AdapterEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![unknown(line)];
    };
    let Some(type_tag) = value.get("type").and_then(Value::as_str) else {
        return vec![unknown(line)];
    };

    match type_tag {
        "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            vec![parse_system_init(&value)]
        }
        "assistant" => parse_assistant(&value).unwrap_or_else(|| vec![unknown(line)]),
        "result" => {
            let subtype = value
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            vec![AdapterEvent::State {
                phase: AgentPhase::Finished { subtype },
                observations: Vec::new(),
            }]
        }
        _ => vec![unknown(line)],
    }
}

/// `{"type":"system","subtype":"init",...}` -> `State { phase: Init,
/// observations }`, with `cwd`/`model`/`session_id` (whichever are
/// present) carried as text observations, in that fixed order.
fn parse_system_init(value: &Value) -> AdapterEvent {
    let mut observations = Vec::new();
    for key in ["cwd", "model", "session_id"] {
        if let Some(field) = value.get(key) {
            observations.push((key.to_string(), value_as_text(field)));
        }
    }
    AdapterEvent::State {
        phase: AgentPhase::Init,
        observations,
    }
}

/// `{"type":"assistant","message":{"content":[...blocks...]}}` -> one
/// event per content block, in order ([`parse_content_block`]). Returns
/// `None` (falling back to one `Unknown` for the whole line at the call
/// site) only if the shape does not match at all: no `message.content`
/// array, or an empty one.
///
/// Every block is mapped (R3-004): a real agent CLI's assistant message
/// can carry several content blocks in one line -- a `text` block
/// announcing a `tool_use` block, say -- and dropping every block after
/// the first would silently lose the proposal a caller most needs to see.
fn parse_assistant(value: &Value) -> Option<Vec<AdapterEvent>> {
    let content = value.get("message")?.get("content")?.as_array()?;
    if content.is_empty() {
        return None;
    }
    Some(content.iter().map(parse_content_block).collect())
}

/// One assistant content block -> `Message` (a well-formed `text` block),
/// `ArtifactPublish` (a `tool_use` block named `artifact.publish` whose
/// input parses as a publish proposal; spike slice 5), `ToolCall` (any
/// other well-formed `tool_use` block, including an `artifact.publish`
/// whose input does not parse -- surfaced as the plain proposal it is,
/// never dropped), or `Unknown` carrying the block's own JSON for anything
/// else (an unrecognized block `type`, or a known type missing its
/// required field). `raw` is the block's JSON re-serialized compactly --
/// `serde_json` does not expose a block's original byte span within the
/// line -- so it is byte-faithful to the block's content, not necessarily
/// to the line's original whitespace.
fn parse_content_block(block: &Value) -> AdapterEvent {
    let event = match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            block
                .get("text")
                .and_then(Value::as_str)
                .map(|text| AdapterEvent::Message {
                    text: text.to_string(),
                })
        }
        Some("tool_use") => block.get("name").and_then(Value::as_str).map(|name| {
            let input = block.get("input");
            if name == PUBLISH_PROPOSAL_TOOL_NAME
                && let Some(proposal) = input.and_then(parse_publish_proposal)
            {
                return AdapterEvent::ArtifactPublish(proposal);
            }
            AdapterEvent::ToolCall(ToolCallProposal {
                name: name.to_string(),
                arguments_text: input.map(Value::to_string).unwrap_or_default(),
            })
        }),
        _ => None,
    };
    event.unwrap_or_else(|| AdapterEvent::Unknown {
        raw: block.to_string().into_bytes(),
        truncated: false,
    })
}

/// `{"entries": [{"name": ..., "sha256": <64 hex>}, ...]}` -> a
/// [`PublishProposal`], or `None` if the shape does not match exactly:
/// `entries` must be an array and every element must carry a string
/// `name` and a parseable `sha256`. An empty array is a valid, empty
/// proposal. Nothing here is executed or trusted: the digests are claims
/// the shell compares against its own (HAP-001-R11).
fn parse_publish_proposal(input: &Value) -> Option<PublishProposal> {
    let entries = input.get("entries")?.as_array()?;
    let entries = entries
        .iter()
        .map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let sha256 = Sha256Digest::from_hex(entry.get("sha256")?.as_str()?)?;
            Some(ProposedEntry { name, sha256 })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(PublishProposal { entries })
}

/// Render a JSON value as observation text: a string value's own content
/// unquoted, anything else its standard JSON text form.
fn value_as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Build the `Unknown` fallback event, carrying `line`'s own bytes
/// verbatim -- byte-identical, never re-serialized or normalized.
fn unknown(line: &str) -> AdapterEvent {
    AdapterEvent::Unknown {
        raw: line.as_bytes().to_vec(),
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{LineAgent, catalog, parse_stream_json_line};
    use omnifrons_app::harness_adapter::HarnessAdapter as _;
    use omnifrons_domain::adapter::{AdapterEvent, AdapterId, AgentPhase};

    /// The single event a line must carry, for the one-event cases.
    fn single(line: &str) -> AdapterEvent {
        let mut events = parse_stream_json_line(line);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one event, got {events:?}"
        );
        events.remove(0)
    }

    #[test]
    fn system_init_maps_to_state_init_with_observations() {
        let line =
            r#"{"type":"system","subtype":"init","cwd":"/work","model":"m1","session_id":"s1"}"#;
        match single(line) {
            AdapterEvent::State {
                phase,
                observations,
            } => {
                assert_eq!(phase, AgentPhase::Init);
                assert_eq!(
                    observations,
                    vec![
                        ("cwd".to_string(), "/work".to_string()),
                        ("model".to_string(), "m1".to_string()),
                        ("session_id".to_string(), "s1".to_string()),
                    ]
                );
            }
            other => panic!("expected State/Init, got {other:?}"),
        }
    }

    #[test]
    fn assistant_text_block_maps_to_message() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(
            single(line),
            AdapterEvent::Message {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn assistant_tool_use_block_maps_to_tool_call() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"write_file","input":{"path":"notes.md"}}]}}"#;
        match single(line) {
            AdapterEvent::ToolCall(proposal) => {
                assert_eq!(proposal.name, "write_file");
                assert!(proposal.arguments_text.contains("notes.md"));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// R3-004: an assistant message carrying several content blocks
    /// yields one event per block, in order -- never only the first.
    #[test]
    fn assistant_message_with_text_then_tool_use_yields_two_events_in_order() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"about to write"},{"type":"tool_use","id":"t1","name":"write_file","input":{"path":"notes.md"}}]}}"#;
        let events = parse_stream_json_line(line);
        assert_eq!(
            events.len(),
            2,
            "one event per content block, got {events:?}"
        );
        assert_eq!(
            events[0],
            AdapterEvent::Message {
                text: "about to write".to_string()
            }
        );
        match &events[1] {
            AdapterEvent::ToolCall(proposal) => {
                assert_eq!(proposal.name, "write_file");
                assert!(proposal.arguments_text.contains("notes.md"));
            }
            other => panic!("expected ToolCall second, got {other:?}"),
        }
    }

    /// R3-004: a content block of a type this parser does not know maps
    /// to `Unknown` carrying that block's own JSON text, while the blocks
    /// around it still map normally.
    #[test]
    fn unrecognized_content_block_maps_to_unknown_with_the_blocks_raw_json() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"thinking","thinking":"..."}]}}"#;
        let events = parse_stream_json_line(line);
        assert_eq!(
            events.len(),
            2,
            "one event per content block, got {events:?}"
        );
        assert_eq!(
            events[0],
            AdapterEvent::Message {
                text: "hi".to_string()
            }
        );
        match &events[1] {
            AdapterEvent::Unknown { raw, truncated } => {
                let raw = std::str::from_utf8(raw).expect("raw is the block's JSON text");
                assert!(raw.contains(r#""type":"thinking""#), "got {raw}");
                assert!(!truncated);
            }
            other => panic!("expected Unknown for the unrecognized block, got {other:?}"),
        }
    }

    #[test]
    fn result_maps_to_state_finished_with_subtype() {
        let line = r#"{"type":"result","subtype":"success"}"#;
        assert_eq!(
            single(line),
            AdapterEvent::State {
                phase: AgentPhase::Finished {
                    subtype: "success".to_string()
                },
                observations: Vec::new(),
            }
        );
    }

    #[test]
    fn invalid_json_maps_to_unknown_byte_identical() {
        let line = r#"{"type":"assistant""#; // deliberately truncated
        match single(line) {
            AdapterEvent::Unknown { raw, truncated } => {
                assert_eq!(raw, line.as_bytes());
                assert!(!truncated);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn unrecognized_type_tag_maps_to_unknown_byte_identical() {
        let line = r#"{"type":"not-a-real-type"}"#;
        match single(line) {
            AdapterEvent::Unknown { raw, truncated } => {
                assert_eq!(raw, line.as_bytes());
                assert!(!truncated);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn empty_line_maps_to_unknown() {
        assert!(matches!(single(""), AdapterEvent::Unknown { .. }));
    }

    #[test]
    fn malformed_assistant_shape_maps_to_unknown_byte_identical() {
        let line = r#"{"type":"assistant","message":{"role":"assistant"}}"#; // no content
        match single(line) {
            AdapterEvent::Unknown { raw, truncated } => {
                assert_eq!(raw, line.as_bytes());
                assert!(!truncated);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn catalog_contains_both_built_in_ids() {
        let adapters = catalog();
        assert_eq!(adapters.len(), 2);
        let ids: Vec<AdapterId> = adapters.iter().map(|a| a.describe().id).collect();
        assert!(ids.contains(&AdapterId::stream_json_cli()));
        assert!(ids.contains(&AdapterId::claude_code()));
    }

    /// R1-003: the `claude-code` descriptor's user-facing notes disclose
    /// what `--permission-mode dontAsk` means -- anything that would need
    /// a confirmation is denied by the harness, not executed.
    #[test]
    fn claude_code_notes_disclose_the_dont_ask_permission_mode() {
        let notes = catalog()
            .into_iter()
            .find(|adapter| adapter.describe().id == AdapterId::claude_code())
            .expect("claude-code must be in the built-in catalog")
            .describe()
            .notes;
        assert!(notes.contains("dontAsk"), "got: {notes}");
        assert!(notes.contains("denied"), "got: {notes}");
        assert!(notes.contains("not executed"), "got: {notes}");
    }

    #[test]
    fn line_agent_describe_is_stable() {
        let adapters = catalog();
        let agent = &adapters[0];
        assert_eq!(agent.describe(), agent.describe());
    }

    /// A `LineAgent` built directly (not via `catalog()`) still parses the
    /// same way -- `parse_line` is a pure function of the line, not of the
    /// descriptor.
    #[test]
    fn parse_line_is_independent_of_the_descriptor_used_to_build_the_agent() {
        let descriptor = catalog().remove(0).describe();
        let agent = LineAgent::new(descriptor);
        let events = agent.parse_line(r#"{"type":"result","subtype":"error"}"#);
        assert_eq!(
            events,
            vec![AdapterEvent::State {
                phase: AgentPhase::Finished {
                    subtype: "error".to_string()
                },
                observations: Vec::new(),
            }]
        );
    }
}
