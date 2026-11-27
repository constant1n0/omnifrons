//! Switch checkpoint contracts (target-architecture.md § Model and harness
//! switching): the envelope a same-device switch writes, its four-condition
//! verdicts, and its canonical encoding.

use omnifrons_domain::adapter::{AdapterId, AgentPrompt};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::RunId;
use omnifrons_domain::scope::ProcessTerminalState;
use omnifrons_domain::switch_check::DefinitionReference;
use omnifrons_domain::switch_checkpoint::{
    CheckpointError, CheckpointFacts, CheckpointState, Consistency, Inventory, InventoryEntry,
    InventoryError, OutgoingRun, PathError, RootRelativePath, SwitchCheckpoint, Unproven,
    integrity_record, split_sealed,
};

fn path(text: &str) -> RootRelativePath {
    RootRelativePath::new(text).expect("a valid root-relative path")
}

fn entry(text: &str, byte: u8, size: u64) -> InventoryEntry {
    InventoryEntry {
        path: path(text),
        digest: Sha256Digest([byte; 32]),
        size,
    }
}

fn inventory(entries: Vec<InventoryEntry>) -> Inventory {
    Inventory::new(entries, true).expect("a valid inventory")
}

fn facts(terminal: Option<ProcessTerminalState>) -> CheckpointFacts {
    CheckpointFacts {
        created_unix_seconds: 1_795_770_799,
        project: Sha256Digest([0x5a; 32]),
        outgoing: OutgoingRun {
            adapter: AdapterId::claude_code(),
            run: Some(RunId::new("run-1795770799-556628907-0").expect("run id")),
            terminal,
        },
        target: AdapterId::stream_json_cli(),
        goal: Some(
            AgentPrompt::new(
                "Draft release notes for version 1.2.0.\nChanges: added an export button.",
            )
            .expect("prompt"),
        ),
        definition: Some(
            DefinitionReference::new("AGENTS.md", "ab".repeat(32)).expect("reference"),
        ),
        inventory: inventory(vec![
            entry("AGENTS.md", 1, 120),
            entry("skills/release-notes/SKILL.md", 2, 300),
        ]),
    }
}

const EXITED: Option<ProcessTerminalState> = Some(ProcessTerminalState::Exited { code: Some(0) });

#[test]
fn a_stopped_run_with_a_complete_inventory_is_prepared() {
    for terminal in [
        EXITED,
        Some(ProcessTerminalState::Killed),
        Some(ProcessTerminalState::Exited { code: None }),
    ] {
        let checkpoint = SwitchCheckpoint::new(facts(terminal)).expect("checkpoint");
        assert_eq!(checkpoint.quiescence(), Ok(()));
        assert_eq!(checkpoint.shutdown(), Ok(()));
        assert_eq!(checkpoint.state(), CheckpointState::Prepared);
    }
}

#[test]
fn a_switch_with_no_outgoing_run_has_nothing_to_stop() {
    let mut no_run = facts(None);
    no_run.outgoing.run = None;
    let checkpoint = SwitchCheckpoint::new(no_run).expect("checkpoint");
    assert_eq!(checkpoint.state(), CheckpointState::Prepared);
}

#[test]
fn a_run_not_seen_terminal_proves_neither_quiescence_nor_shutdown() {
    let checkpoint = SwitchCheckpoint::new(facts(None)).expect("checkpoint");
    assert_eq!(checkpoint.quiescence(), Err(Unproven::RunNotTerminal));
    assert_eq!(checkpoint.shutdown(), Err(Unproven::RunNotTerminal));
    assert_eq!(
        checkpoint.state(),
        CheckpointState::Uncertain(vec![Unproven::RunNotTerminal])
    );
}

#[test]
fn an_orphan_risk_stop_is_uncertain() {
    let checkpoint = SwitchCheckpoint::new(facts(Some(ProcessTerminalState::OrphanRiskUncertain)))
        .expect("checkpoint");
    assert_eq!(checkpoint.quiescence(), Err(Unproven::DescendantsUnproven));
    assert_eq!(checkpoint.shutdown(), Err(Unproven::DescendantsUnproven));
    assert_eq!(
        checkpoint.state(),
        CheckpointState::Uncertain(vec![Unproven::DescendantsUnproven])
    );
}

#[test]
fn a_capped_inventory_leaves_workspace_consistency_unprovable() {
    let mut capped = facts(EXITED);
    capped.inventory = Inventory::new(vec![entry("AGENTS.md", 1, 120)], false).expect("inventory");
    let checkpoint = SwitchCheckpoint::new(capped).expect("checkpoint");
    assert_eq!(
        checkpoint.state(),
        CheckpointState::Uncertain(vec![Unproven::InventoryIncomplete])
    );
}

#[test]
fn a_terminal_state_without_a_run_is_refused() {
    let mut odd = facts(EXITED);
    odd.outgoing.run = None;
    assert_eq!(
        SwitchCheckpoint::new(odd).map(|_| ()),
        Err(CheckpointError::TerminalWithoutRun)
    );
}

#[test]
fn the_body_round_trips_through_its_encoding() {
    for terminal in [
        None,
        EXITED,
        Some(ProcessTerminalState::Exited { code: Some(-9) }),
        Some(ProcessTerminalState::Exited { code: None }),
        Some(ProcessTerminalState::Killed),
        Some(ProcessTerminalState::OrphanRiskUncertain),
    ] {
        let checkpoint = SwitchCheckpoint::new(facts(terminal)).expect("checkpoint");
        let body = checkpoint.encode_body();
        assert!(
            body.starts_with("schema=31:omnifrons.switch-checkpoint/1.0\n"),
            "{body}"
        );
        assert_eq!(SwitchCheckpoint::decode_body(&body), Ok(checkpoint));
    }
}

#[test]
fn the_body_records_the_absent_summary_and_no_raw_path() {
    let body = SwitchCheckpoint::new(facts(EXITED))
        .expect("checkpoint")
        .encode_body();
    assert!(body.contains("summary=6:absent\n"), "{body}");
    let goal = "Draft release notes for version 1.2.0.\nChanges: added an export button.";
    assert!(
        body.contains(&format!("goal={}:{goal}\n", goal.len())),
        "{body}"
    );
    assert!(
        !body.contains("=/") && !body.contains(" /"),
        "no absolute path: {body}"
    );
}

#[test]
fn an_unknown_major_is_rejected() {
    let body = SwitchCheckpoint::new(facts(EXITED))
        .expect("checkpoint")
        .encode_body();
    let newer = body.replacen(
        "omnifrons.switch-checkpoint/1.0",
        "omnifrons.switch-checkpoint/2.0",
        1,
    );
    assert_eq!(
        SwitchCheckpoint::decode_body(&newer),
        Err(CheckpointError::UnsupportedMajor(2))
    );
}

#[test]
fn an_unknown_optional_field_of_a_newer_minor_is_preserved() {
    let body = SwitchCheckpoint::new(facts(EXITED))
        .expect("checkpoint")
        .encode_body();
    let newer = format!(
        "{}validations=10:cargo test\n",
        body.replacen(
            "omnifrons.switch-checkpoint/1.0",
            "omnifrons.switch-checkpoint/1.3",
            1
        )
    );
    let decoded = SwitchCheckpoint::decode_body(&newer).expect("a newer minor decodes");
    assert_eq!(decoded.minor(), 3);
    assert_eq!(
        decoded.extra_fields(),
        &[("validations".to_string(), "cargo test".to_string())]
    );
    assert_eq!(
        decoded.encode_body(),
        newer,
        "re-encoding keeps the unknown field"
    );
}

#[test]
fn malformed_bodies_are_rejected() {
    let body = SwitchCheckpoint::new(facts(EXITED))
        .expect("checkpoint")
        .encode_body();
    let cases = [
        (String::new(), CheckpointError::MissingField("schema")),
        (
            "created=10:1795770799\n".to_string(),
            CheckpointError::SchemaNotFirst,
        ),
        (
            body[..body.len() - 3].to_string(),
            CheckpointError::Truncated,
        ),
        (
            body.replacen("summary=6:absent", "summary=6:absenX", 1),
            CheckpointError::InvalidValue("summary"),
        ),
        (
            body.replacen("summary=6:absent\n", "", 1),
            CheckpointError::MissingField("summary"),
        ),
        (
            format!("{body}target.adapter=7:pty-cli\n"),
            CheckpointError::DuplicateField("target.adapter"),
        ),
        (
            body.replacen(
                "target.adapter=15:stream-json-cli",
                "target.adapter=15:stream-json-xyz",
                1,
            ),
            CheckpointError::InvalidValue("target.adapter"),
        ),
        (
            body.replacen("summary=6:", "summary=06:", 1),
            CheckpointError::BadLength,
        ),
        (
            body.replacen("summary=6:absent\n", "summary=6:absent!", 1),
            CheckpointError::MissingNewline,
        ),
        (
            format!("{body}integrity=3:abc\n"),
            CheckpointError::IntegrityInBody,
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(
            SwitchCheckpoint::decode_body(&text),
            Err(expected.clone()),
            "{text}"
        );
    }
}

#[test]
fn root_relative_paths_refuse_anything_that_could_name_a_raw_location() {
    for (text, expected) in [
        ("", PathError::Empty),
        ("/etc/passwd", PathError::Absolute),
        ("C:/Users/x", PathError::Absolute),
        ("a\\b", PathError::Backslash),
        ("a/../b", PathError::DotComponent),
        ("./a", PathError::DotComponent),
        ("a//b", PathError::EmptyComponent),
        ("a/", PathError::EmptyComponent),
        ("a\nb", PathError::ControlCharacter),
        ("a\0b", PathError::ControlCharacter),
    ] {
        assert_eq!(RootRelativePath::new(text), Err(expected), "{text:?}");
    }
    assert_eq!(
        path("skills/release-notes/SKILL.md").as_str(),
        "skills/release-notes/SKILL.md"
    );
}

#[test]
fn an_inventory_is_sorted_and_refuses_duplicates() {
    let sorted = inventory(vec![entry("b", 2, 1), entry("a", 1, 1)]);
    let names: Vec<&str> = sorted.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(
        Inventory::new(vec![entry("a", 1, 1), entry("a", 2, 1)], true).map(|_| ()),
        Err(InventoryError::DuplicatePath("a".to_string()))
    );
}

#[test]
fn comparing_inventories_names_every_change() {
    let before = inventory(vec![entry("a", 1, 1), entry("b", 2, 1), entry("c", 3, 1)]);
    assert_eq!(before.compare(&before.clone()), Consistency::Consistent);
    let after = inventory(vec![entry("a", 1, 1), entry("c", 9, 1), entry("d", 4, 1)]);
    let Consistency::Changed(changes) = before.compare(&after) else {
        panic!("a changed inventory");
    };
    assert_eq!(changes.added, vec![path("d")]);
    assert_eq!(changes.removed, vec![path("b")]);
    assert_eq!(changes.modified, vec![path("c")]);
    let capped = Inventory::new(vec![entry("a", 1, 1)], false).expect("inventory");
    assert_eq!(before.compare(&capped), Consistency::Unproven);
    assert_eq!(capped.compare(&before), Consistency::Unproven);
}

#[test]
fn a_sealed_checkpoint_splits_into_its_body_and_digest() {
    let body = SwitchCheckpoint::new(facts(EXITED))
        .expect("checkpoint")
        .encode_body();
    let digest = Sha256Digest([0xc3; 32]);
    let sealed = format!("{body}{}", integrity_record(&digest));
    assert_eq!(split_sealed(&sealed), Ok((body.as_str(), digest)));
    assert_eq!(
        split_sealed(&body),
        Err(CheckpointError::MissingField("integrity"))
    );
    assert_eq!(
        split_sealed(&format!("{sealed}goal=1:x\n")),
        Err(CheckpointError::MissingField("integrity"))
    );
    let bad = format!("{body}integrity=10:sha256:xyz\n");
    assert_eq!(
        split_sealed(&bad),
        Err(CheckpointError::InvalidValue("integrity"))
    );
}

#[test]
fn a_secret_in_the_goal_is_redacted_before_it_is_stored() {
    let mut secret = facts(EXITED);
    secret.goal = Some(AgentPrompt::new("deploy with bearer abc123XYZ please").expect("prompt"));
    let checkpoint = SwitchCheckpoint::new(secret).expect("checkpoint");
    assert_eq!(
        checkpoint.facts().goal.as_ref().map(AgentPrompt::as_str),
        Some("deploy with bearer <redacted> please")
    );
    assert!(!checkpoint.encode_body().contains("abc123XYZ"));
}
