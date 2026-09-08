//! `LineAgent::parse_line` recognizes an `artifact.publish` `tool_use`
//! block whose input carries `entries: [{ name, sha256 }]` as the typed
//! `AdapterEvent::ArtifactPublish` (spike slice 5, HAP-001-R12, D5) -- a
//! proposal only, like every other tool call -- and falls back to the
//! plain `ToolCall` proposal for the same tool name with a malformed
//! payload, so nothing a harness says is ever dropped.

use omnifrons_adapters::line_agent::catalog;
use omnifrons_domain::adapter::AdapterEvent;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{ProposedEntry, PublishProposal};

fn parse(line: &str) -> Vec<AdapterEvent> {
    catalog().remove(0).parse_line(line)
}

#[test]
fn a_well_formed_artifact_publish_tool_use_becomes_a_typed_proposal() {
    let digest = Sha256Digest([0x5a; 32]).to_hex();
    let line = format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"artifact.publish","input":{{"entries":[{{"name":"report.pdf","sha256":"{digest}"}},{{"name":"figure.png","sha256":"{}"}}]}}}}]}}}}"#,
        Sha256Digest([0x11; 32]).to_hex()
    );
    let events = parse(&line);
    assert_eq!(
        events,
        vec![AdapterEvent::ArtifactPublish(PublishProposal {
            entries: vec![
                ProposedEntry {
                    name: "report.pdf".to_string(),
                    sha256: Sha256Digest([0x5a; 32]),
                },
                ProposedEntry {
                    name: "figure.png".to_string(),
                    sha256: Sha256Digest([0x11; 32]),
                },
            ],
        })]
    );
}

#[test]
fn a_publish_proposal_with_a_malformed_entry_falls_back_to_a_plain_tool_call() {
    for input in [
        r#"{"entries":[{"name":"report.pdf","sha256":"not-hex"}]}"#,
        r#"{"entries":[{"sha256":"5a5a"}]}"#,
        r#"{"entries":"report.pdf"}"#,
        "{}",
        "[]",
    ] {
        let line = format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"artifact.publish","input":{input}}}]}}}}"#
        );
        let events = parse(&line);
        assert_eq!(events.len(), 1, "input {input}");
        match &events[0] {
            AdapterEvent::ToolCall(proposal) => {
                assert_eq!(proposal.name, "artifact.publish");
                assert!(!proposal.arguments_text.is_empty(), "input {input}");
            }
            other => panic!("expected a plain ToolCall fallback for input {input}, got {other:?}"),
        }
    }
}

#[test]
fn an_empty_entries_list_is_still_a_typed_proposal() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"artifact.publish","input":{"entries":[]}}]}}"#;
    assert_eq!(
        parse(line),
        vec![AdapterEvent::ArtifactPublish(PublishProposal {
            entries: vec![]
        })]
    );
}

#[test]
fn other_tool_uses_are_untouched() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"write_file","input":{"entries":[]}}]}}"#;
    assert!(matches!(parse(line)[0], AdapterEvent::ToolCall(_)));
}
