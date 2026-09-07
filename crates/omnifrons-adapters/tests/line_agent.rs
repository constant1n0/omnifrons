//! `LineAgent::parse_line` mapping, including malformed input mapping to
//! `Unknown` byte-identical, the shared `HarnessAdapter` contract run
//! against both real built-in adapters (R3-013), and `LineAssembler` +
//! `LineAgent` together turning an over-cap "line" (as
//! `omnifrons-supervisor`'s own output capture would deliver it:
//! consecutive full-cap text frames) into a truncated `Unknown`, never
//! attempting to parse the (incomplete) accumulated JSON.

use omnifrons_adapters::LineAgent;
use omnifrons_adapters::line_agent::catalog;
use omnifrons_app::contract::harness_adapter::harness_adapter_contract;
use omnifrons_app::harness_adapter::HarnessAdapter as _;
use omnifrons_app::{Assembled, AssembledLine, LineAssembler};
use omnifrons_domain::adapter::AdapterEvent;
use omnifrons_domain::output::MAX_TEXT_FRAME_BYTES;

fn built_in_agent(id: &str) -> LineAgent {
    LineAgent::new(
        catalog()
            .into_iter()
            .find(|a| a.describe().id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} must be present in the built-in catalog"))
            .describe(),
    )
}

fn stream_json_cli_agent() -> LineAgent {
    built_in_agent("stream-json-cli")
}

fn claude_code_agent() -> LineAgent {
    built_in_agent("claude-code")
}

/// R3-013: the reusable `HarnessAdapter` contract (`describe` stable;
/// `parse_line` total and yielding exactly one `Unknown` over the shared
/// unparsable corpus -- empty, a truncated fragment, 1 MiB of `'a'`, text
/// carrying U+FFFD) actually runs against both real built-in adapters,
/// not only the fake in `omnifrons-app`'s own tests.
#[test]
fn both_built_in_line_agents_satisfy_the_harness_adapter_contract() {
    harness_adapter_contract(stream_json_cli_agent);
    harness_adapter_contract(claude_code_agent);
}

#[test]
fn malformed_lines_map_to_unknown_byte_identical() {
    let agent = stream_json_cli_agent();
    for line in [
        "",
        "not json at all",
        r#"{"type":"assistant""#,
        r#"{"no_type_field": true}"#,
        r#"{"type":123}"#,
    ] {
        let mut events = agent.parse_line(line);
        assert_eq!(
            events.len(),
            1,
            "an unparsable line yields exactly one event, got {events:?}"
        );
        match events.remove(0) {
            AdapterEvent::Unknown { raw, truncated } => {
                assert_eq!(
                    raw,
                    line.as_bytes(),
                    "raw bytes must be byte-identical to the input line"
                );
                assert!(!truncated, "a parse failure is not itself a truncation");
            }
            other => panic!("expected Unknown for {line:?}, got {other:?}"),
        }
    }
}

/// Simulates a huge (over the 64 KiB `LineAssembler` cap) `stream-json`
/// line arriving as consecutive full-per-frame-cap `continued` text
/// frames, exactly as `omnifrons-supervisor`'s own output-capture framing
/// would deliver one: the assembler must report it truncated, and the
/// shell must never hand the (necessarily incomplete) accumulated bytes to
/// `LineAgent::parse_line` at all.
#[test]
fn an_over_cap_line_is_assembled_then_reported_truncated_never_parsed() {
    let agent = stream_json_cli_agent();
    let mut assembler = LineAssembler::new();

    let full_cap_chunk = "{".repeat(MAX_TEXT_FRAME_BYTES);
    // Comfortably exceed the 64 KiB assembler cap.
    for _ in 0..9 {
        assert_eq!(assembler.push(&full_cap_chunk, true, 0), None);
    }
    let result = assembler.push("}", false, 0); // the line-ending frame

    let event = match result {
        Some(Assembled {
            line: AssembledLine::Truncated { raw },
            ..
        }) => AdapterEvent::Unknown {
            raw,
            truncated: true,
        },
        other => panic!("expected Truncated, got {other:?}"),
    };

    match event {
        AdapterEvent::Unknown { truncated, .. } => assert!(truncated),
        other => panic!("expected Unknown{{truncated: true}}, got {other:?}"),
    }

    // The shell never calls parse_line for a Truncated result; confirm
    // that parse_line, if it *were* called on the raw (necessarily
    // incomplete) accumulated JSON, would itself stay total (never panic)
    // -- belt and suspenders on top of the shell's own routing.
    let huge_incomplete = full_cap_chunk.repeat(9);
    let _ = agent.parse_line(&huge_incomplete);
}
