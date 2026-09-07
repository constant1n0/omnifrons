//! `PtyCli` (spike slice 4): the second structurally different built-in
//! adapter -- pseudo-terminal transport, typed prompt, no structured
//! events (`docs/spike-log.md` § Slice 4). Its descriptor discloses the
//! degraded posture in plain English, its `build_launch` produces a
//! `TransportClass::Pty` plan whose argv never carries the prompt, and its
//! `parse_line` is total (the PTY path never calls it).

use omnifrons_adapters::PtyCli;
use omnifrons_app::contract::harness_adapter::harness_adapter_contract;
use omnifrons_app::harness_adapter::HarnessAdapter as _;
use omnifrons_app::{EnvPlan, LaunchPlanError, LaunchRequest, WorkspaceRoot};
use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, AgentPrompt, PromptChannel, StdinPlan, TransportClass,
};
use omnifrons_domain::scope::ScopeMode;

fn workspace() -> WorkspaceRoot {
    WorkspaceRoot::new(std::env::temp_dir()).expect("the system temp dir must be a valid workspace")
}

#[test]
fn pty_cli_descriptor_declares_the_pty_transport_and_the_typed_prompt_channel() {
    let descriptor = PtyCli::new().describe();
    assert_eq!(descriptor.id, AdapterId::pty_cli());
    assert_eq!(descriptor.transport_class, TransportClass::Pty);
    assert_eq!(descriptor.prompt_channel, PromptChannel::PtyTyped);
    assert!(descriptor.argv_template.is_empty());
    assert!(descriptor.declared_env.is_empty());
    assert_eq!(descriptor.scope_mode, ScopeMode::Advisory);
}

/// The notes disclose, in plain English, what a user gives up with this
/// adapter and what the terminal is not.
#[test]
fn pty_cli_notes_disclose_the_degraded_fallback_in_plain_english() {
    let notes = PtyCli::new().describe().notes;
    for required in [
        "degraded fallback",
        "no structured events",
        "plain text",
        "dropped and counted",
        "title",
        "notification",
        "hyperlink",
        "clipboard",
        "file transfer",
        "not a sandbox",
        "Windows",
    ] {
        assert!(
            notes.contains(required),
            "the notes must mention {required:?}, got: {notes}"
        );
    }
}

#[test]
fn pty_cli_build_launch_carries_the_pty_transport_typed_prompt_and_null_stdin() {
    let prompt = AgentPrompt::new("do the thing").expect("valid prompt");
    let request = LaunchRequest {
        prompt: prompt.clone(),
        workspace: workspace(),
    };

    let plan = PtyCli::new()
        .build_launch(&request)
        .expect("build_launch must succeed for a well-formed request");

    assert_eq!(plan.transport, TransportClass::Pty);
    assert!(plan.argv.is_empty(), "the argv template is empty");
    assert_eq!(
        plan.stdin,
        StdinPlan::Null,
        "stdin is unused on the PTY path: the prompt is typed, not piped"
    );
    assert_eq!(plan.prompt, Some(prompt));
    assert_eq!(
        plan.env,
        EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        "the base allowlist only"
    );
    assert_eq!(plan.cwd, workspace());
    assert_eq!(plan.scope_mode, ScopeMode::Advisory);
}

/// A typed prompt reaches the child through the terminal's line
/// discipline, which interprets C0 controls -- 0x03 interrupts, 0x04 ends
/// the input, 0x1A suspends, 0x1C quits, `\r` ends the line mid-text, ESC
/// opens a sequence -- and DEL (erases the previous character) instead of
/// typing them: such a prompt is refused with a typed error at
/// plan-building time, before anything is launched.
#[test]
fn pty_cli_build_launch_rejects_a_prompt_the_line_discipline_would_interpret() {
    for (label, text) in [
        ("VINTR 0x03", "abc\x03def"),
        ("VEOF 0x04", "abc\x04"),
        ("VSUSP 0x1A", "\x1aabc"),
        ("VQUIT 0x1C", "abc\x1cdef"),
        ("CR", "line one\rline two"),
        ("DEL", "abc\x7f"),
        ("ESC", "abc\x1b[31m"),
    ] {
        let request = LaunchRequest {
            prompt: AgentPrompt::new(text).expect("within the prompt cap"),
            workspace: workspace(),
        };
        let result = PtyCli::new().build_launch(&request);
        assert_eq!(
            result,
            Err(LaunchPlanError::PromptNotTypeable),
            "a prompt carrying {label} must be refused as not typeable"
        );
    }
}

/// Newline and tab are the two C0 controls a typed prompt may carry: a
/// newline ends a line the child reads as such, a tab is typed literally.
#[test]
fn pty_cli_build_launch_accepts_newline_and_tab_in_the_prompt() {
    let prompt = AgentPrompt::new("first line\n\tindented second line\n").expect("valid prompt");
    let request = LaunchRequest {
        prompt: prompt.clone(),
        workspace: workspace(),
    };
    let plan = PtyCli::new()
        .build_launch(&request)
        .expect("newline and tab are typeable");
    assert_eq!(plan.prompt, Some(prompt));
}

/// The stdin path is untouched: a line agent pipes the prompt, no line
/// discipline is involved, and every byte a prompt may carry is accepted
/// exactly as before.
#[test]
fn line_agents_still_accept_control_characters_in_the_prompt() {
    let prompt = AgentPrompt::new("abc\x03\x04\x1a\x1c\r\x7f\x1b[31m").expect("valid prompt");
    for adapter in omnifrons_adapters::line_agent::catalog() {
        let request = LaunchRequest {
            prompt: prompt.clone(),
            workspace: workspace(),
        };
        let plan = adapter
            .build_launch(&request)
            .expect("the stdin path accepts control characters in the prompt");
        assert_eq!(
            plan.prompt,
            Some(prompt.clone()),
            "for {}",
            adapter.describe().id
        );
    }
}

#[test]
fn pty_cli_build_launch_never_puts_the_prompt_in_argv() {
    let big_prompt_text = "secret-marker-".repeat(200);
    let prompt = AgentPrompt::new(big_prompt_text).expect("prompt within the cap");
    let request = LaunchRequest {
        prompt,
        workspace: workspace(),
    };
    let plan = PtyCli::new()
        .build_launch(&request)
        .expect("build_launch must succeed");
    assert!(plan.argv.iter().all(|arg| !arg.contains("secret-marker")));
}

/// The PTY path never calls `parse_line`; it must still be total, and it
/// maps every input to exactly one byte-identical `Unknown`.
#[test]
fn pty_cli_parse_line_is_total_and_always_one_unknown() {
    let adapter = PtyCli::new();
    for line in [
        "",
        "not json at all",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        "\x1b[31mred\x1b[0m",
        "before \u{FFFD} after",
    ] {
        let events = adapter.parse_line(line);
        assert_eq!(
            events,
            vec![AdapterEvent::Unknown {
                raw: line.as_bytes().to_vec(),
                truncated: false,
            }],
            "every input yields exactly one byte-identical Unknown, got {events:?} for {line:?}"
        );
    }
}

#[test]
fn pty_cli_satisfies_the_harness_adapter_contract() {
    harness_adapter_contract(PtyCli::new);
}

/// The crate-level catalog carries all three built-in adapters; the
/// slice-3 `line_agent::catalog()` stays exactly the two line agents.
#[test]
fn the_crate_catalog_contains_all_three_built_in_adapters() {
    let ids: Vec<AdapterId> = omnifrons_adapters::catalog()
        .iter()
        .map(|adapter| adapter.describe().id)
        .collect();
    assert_eq!(ids.len(), 3, "got {ids:?}");
    assert!(ids.contains(&AdapterId::stream_json_cli()));
    assert!(ids.contains(&AdapterId::claude_code()));
    assert!(ids.contains(&AdapterId::pty_cli()));
    assert_eq!(omnifrons_adapters::line_agent::catalog().len(), 2);
}

/// The two line agents' plans carry the structured-streaming transport
/// explicitly, so a supervisor branching on `plan.transport` keeps them on
/// the pipe path.
#[test]
fn line_agent_plans_carry_the_structured_streaming_transport() {
    let request = LaunchRequest {
        prompt: AgentPrompt::new("hi").expect("valid prompt"),
        workspace: workspace(),
    };
    for adapter in omnifrons_adapters::line_agent::catalog() {
        let plan = adapter
            .build_launch(&request)
            .expect("build_launch must succeed");
        assert_eq!(
            plan.transport,
            TransportClass::StructuredStreamingCli,
            "for {}",
            adapter.describe().id
        );
    }
}
