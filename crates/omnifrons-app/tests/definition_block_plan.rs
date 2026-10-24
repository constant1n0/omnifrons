//! The `skills` block and the legacy-block migration as planned
//! operations (ADR-0005 sub-slice 2b), against the in-memory
//! `DefinitionTree` fake: when a block is appended, regenerated or
//! migrated, which hand edits and layout problems refuse the plan, and
//! that the bytes outside the block never change.
//!
//! Only compiled with `--features contract-tests`: the fake lives behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::path::PathBuf;

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::definition::{InMemoryDefinitionTree, TreeSnapshotBuilder};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, verify};
use omnifrons_app::definition_plan::{PlanOutcome, PlannedWrite, plan_with_writes};
use omnifrons_app::definition_tree::TreeSnapshot;
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, WRAPPER};
use omnifrons_domain::definition_plan::{BlockMode, Operation, Plan, PlanOptions, PlanRefusal};
use omnifrons_domain::skills_index::{PRECEDENCE_RULE, RULE_HEADING};

/// The one workspace root every `run` plans under. It lives for the
/// whole test process; the id is scoped to it, so two runs only share an
/// id when they share this root.
fn shared_workspace() -> WorkspaceRoot {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let root = ROOT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-block-plan-shared-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir
    });
    WorkspaceRoot::new(root).expect("valid workspace")
}

const BODY: &[u8] = b"# Sample\n\nYou are Sample, a fixture agent.\n";

fn identity(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn skill(name: &str, description: &str) -> Vec<u8> {
    format!("---\nname: {name}\ndescription: {description}\n---\n").into_bytes()
}

/// A canonical root (`AGENTS.md` plus the wrapper) holding `agents`
/// and the skills `(path, bytes)`.
fn canonical_root(agents: &[u8], skills: &[(&str, Vec<u8>)]) -> TreeSnapshot {
    let mut builder = TreeSnapshotBuilder::new().directory(
        ".",
        vec![
            identity(IdentityKind::Agents, agents),
            identity(IdentityKind::Claude, WRAPPER),
        ],
        Vec::new(),
        Vec::new(),
    );
    for (path, bytes) in skills {
        builder = builder.skill(*path, bytes.clone());
    }
    builder.build()
}

/// A `CLAUDE.md`-only root holding `claude` and the skills.
fn claude_root(claude: &[u8], skills: &[(&str, Vec<u8>)]) -> TreeSnapshot {
    let mut builder = TreeSnapshotBuilder::new().directory(
        ".",
        vec![identity(IdentityKind::Claude, claude)],
        Vec::new(),
        Vec::new(),
    );
    for (path, bytes) in skills {
        builder = builder.skill(*path, bytes.clone());
    }
    builder.build()
}

type Planned = Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError>;

fn run(snapshot: TreeSnapshot, target: Option<&str>, options: PlanOptions) -> Planned {
    // One fixed root, so equal snapshots give equal plan ids.
    let workspace = shared_workspace();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    plan_with_writes(&ports, &workspace, target, &options)
}

fn planned(outcome: Planned) -> (Plan, Vec<PlannedWrite>) {
    match outcome.expect("plan succeeds") {
        (PlanOutcome::Planned(plan), writes) => (plan, writes),
        (PlanOutcome::Refused(refusal), _) => panic!("expected a plan, got {refusal}"),
    }
}

fn refused(outcome: Planned) -> PlanRefusal {
    match outcome.expect("plan succeeds") {
        (PlanOutcome::Refused(refusal), writes) => {
            assert!(writes.is_empty(), "a refusal carries no writes");
            refusal
        }
        (PlanOutcome::Planned(plan), _) => panic!("expected a refusal, got {:?}", plan.operations),
    }
}

fn paths(plan: &Plan) -> Vec<&'static str> {
    plan.operations.iter().map(Operation::path).collect()
}

/// The `AGENTS.md` bytes the block operation of `plan` would write.
fn block_bytes(plan: &Plan, writes: &[PlannedWrite]) -> Vec<u8> {
    assert_eq!(plan.operations.len(), writes.len(), "writes align");
    let index = plan
        .operations
        .iter()
        .position(|operation| matches!(operation, Operation::WriteBlock { .. }))
        .expect("the plan has a block operation");
    writes[index].bytes.clone()
}

fn block_mode(plan: &Plan) -> Option<BlockMode> {
    plan.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::WriteBlock { mode, .. } => Some(*mode),
            _ => None,
        })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("fixture text is UTF-8")
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The `AGENTS.md` bytes a first adaptation of a `CLAUDE.md`-only root
/// holding `BODY` and `skills` would write: an applied, canonical text
/// with an Omnifrons block.
fn applied(skills: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (plan, writes) = planned(run(claude_root(BODY, skills), None, PlanOptions::default()));
    block_bytes(&plan, &writes)
}

/// A legacy `portable-agent:` block around `body_lines` (the prototype's
/// own sentinels and digest rule: the SHA-256 of the body lines, each
/// ending in `\n`).
fn legacy_block(body_lines: &[&str]) -> String {
    let body = body_lines.join("\n") + "\n";
    format!(
        "<!-- portable-agent:skills:begin sha256={} -->\n{body}<!-- portable-agent:skills:end -->\n",
        FakeHasher.sha256(body.as_bytes()).to_hex()
    )
}

const LEGACY_ROW: &str = "| sample | Sample skill. | [sample](skills/sample/SKILL.md) |";

/// The legacy body lines of a one-row index, with the precedence rule
/// when `with_rule`.
fn legacy_lines(with_rule: bool) -> Vec<&'static str> {
    let mut lines = vec![
        "## Local skill index",
        "",
        "| Skill | Use when | Path |",
        "| --- | --- | --- |",
        LEGACY_ROW,
    ];
    if with_rule {
        lines.push("");
        lines.push(RULE_HEADING);
        lines.push("");
        lines.extend(PRECEDENCE_RULE.lines());
    }
    lines
}

fn sample_skills() -> Vec<(&'static str, Vec<u8>)> {
    vec![("skills/sample/SKILL.md", skill("sample", "Sample skill."))]
}

const BEGIN: &str = "<!-- omnifrons:begin skills ";
const END: &str = "<!-- omnifrons:end skills -->";

fn after_of(operation: &Operation) -> omnifrons_domain::executable::Sha256Digest {
    match operation {
        Operation::WriteAgents { after, .. }
        | Operation::WriteWrapper { after, .. }
        | Operation::WriteBlock { after, .. } => *after,
    }
}

/// Plans over `agents` as the canonical text of a canonical root.
fn plan_canonical(agents: &[u8], skills: &[(&str, Vec<u8>)]) -> Planned {
    run(canonical_root(agents, skills), None, PlanOptions::default())
}

#[test]
fn a_first_adaptation_appends_the_block_after_the_identity_operations() {
    let (plan, writes) = planned(run(
        claude_root(BODY, &sample_skills()),
        None,
        PlanOptions::default(),
    ));
    assert_eq!(paths(&plan), vec!["AGENTS.md", "CLAUDE.md", "AGENTS.md"]);
    assert_eq!(block_mode(&plan), Some(BlockMode::Append));
    // Every planned write hashes to its operation's `after`.
    assert_eq!(writes.len(), plan.operations.len());
    for (operation, write) in plan.operations.iter().zip(&writes) {
        assert_eq!(FakeHasher.sha256(&write.bytes), after_of(operation));
        assert_eq!(write.path, operation.path());
    }
    // The block runs on the file `WriteAgents` leaves.
    let Operation::WriteBlock { before, body, .. } = &plan.operations[2] else {
        panic!("the last operation is the block write");
    };
    assert_eq!(
        *before,
        omnifrons_domain::definition_plan::InputFingerprint::Sha256(FakeHasher.sha256(BODY))
    );
    let after = text(&block_bytes(&plan, &writes));
    assert!(after.starts_with(&text(BODY)), "the text is kept");
    assert!(after.contains(LEGACY_ROW), "{after}");
    assert_eq!(count(&after, BEGIN), 1);
    assert_eq!(count(&after, RULE_HEADING), 1);
    assert!(after.ends_with(&format!("{END}\n")));
    // One blank line between the text and the block.
    assert!(after.contains(&format!("{}\n{BEGIN}", text(BODY))));
    // The body digest is the digest of the block's body lines.
    let body_text = after
        .split_once(BEGIN)
        .and_then(|(_, rest)| rest.split_once('\n'))
        .and_then(|(_, rest)| rest.split_once(&format!("\n{END}")))
        .map(|(body, _)| body.to_string())
        .expect("a block body");
    assert_eq!(*body, FakeHasher.sha256(body_text.as_bytes()));

    let diff = plan.block_diff.as_ref().expect("a block diff");
    assert_eq!(diff.path, "AGENTS.md");
    assert!(diff.before.is_empty());
    assert!(diff.after[0].starts_with(BEGIN));
    assert_eq!(diff.after.last().map(String::as_str), Some(END));
}

#[test]
fn the_block_regenerates_only_when_unedited() {
    let skills = sample_skills();
    let current = applied(&skills);
    let mut more = skills.clone();
    more.push(("skills/second/SKILL.md", skill("second", "Second skill.")));

    let (plan, writes) = planned(plan_canonical(&current, &more));
    assert_eq!(paths(&plan), vec!["AGENTS.md"]);
    assert_eq!(block_mode(&plan), Some(BlockMode::Replace));
    let before = text(&current);
    let after = text(&block_bytes(&plan, &writes));
    let begin = before.find(BEGIN).expect("a block");
    assert_eq!(&after[..begin], &before[..begin], "the prefix is kept");
    assert!(after.contains("| second | Second skill. | [second](skills/second/SKILL.md) |"));
    assert!(after.contains(LEGACY_ROW));
    assert_eq!(count(&after, BEGIN), 1);
    let diff = plan.block_diff.as_ref().expect("a block diff");
    assert!(diff.before[0].starts_with(BEGIN));
    assert!(diff.after.iter().any(|line| line.contains("second")));
    assert!(!diff.before.iter().any(|line| line.contains("second")));

    // Reapplying: the result plans nothing more.
    let (again, writes) = planned(plan_canonical(after.as_bytes(), &more));
    assert!(again.operations.is_empty(), "{:?}", again.operations);
    assert!(again.block_diff.is_none());
    assert!(writes.is_empty());

    // A hand edit inside the block is refused, never replaced.
    let edited = before.replace("Sample skill.", "Edited by hand.");
    let refusal = refused(plan_canonical(edited.as_bytes(), &more));
    assert_eq!(refusal.code(), "edited_block");
}

#[test]
fn text_after_the_current_block_is_kept_byte_for_byte() {
    let skills = sample_skills();
    let mut current = applied(&skills);
    current.extend_from_slice(b"\n## Later notes\r\n\r\nMore.\n");
    let mut more = skills;
    more.push(("skills/second/SKILL.md", skill("second", "Second skill.")));
    let (plan, writes) = planned(plan_canonical(&current, &more));
    let after = block_bytes(&plan, &writes);
    assert!(after.ends_with(b"\n## Later notes\r\n\r\nMore.\n"));
    let begin = text(&current).find(BEGIN).expect("a block");
    assert_eq!(&after[..begin], &current[..begin]);
}

#[test]
fn bytes_that_are_not_utf8_outside_the_block_are_kept() {
    let skills = sample_skills();
    let current = applied(&skills);
    let marker = current
        .windows(BEGIN.len())
        .position(|window| window == BEGIN.as_bytes())
        .expect("a block");
    let mut with_bytes = current[..marker].to_vec();
    with_bytes.extend_from_slice(b"Notes \xff\xfe kept.\n\n");
    let prefix_length = with_bytes.len();
    with_bytes.extend_from_slice(&current[marker..]);
    with_bytes.extend_from_slice(b"\nTail \xff\n");
    let mut more = skills;
    more.push(("skills/second/SKILL.md", skill("second", "Second skill.")));
    let (plan, writes) = planned(plan_canonical(&with_bytes, &more));
    let after = block_bytes(&plan, &writes);
    assert_eq!(&after[..prefix_length], &with_bytes[..prefix_length]);
    assert!(after.ends_with(b"\nTail \xff\n"));
    assert_eq!(block_mode(&plan), Some(BlockMode::Replace));
}

#[test]
fn a_crlf_file_gets_a_crlf_block() {
    let crlf = b"# Sample\r\n\r\nYou are Sample.\r\n";
    let (plan, writes) = planned(run(
        claude_root(crlf, &sample_skills()),
        None,
        PlanOptions::default(),
    ));
    let after = text(&block_bytes(&plan, &writes));
    assert!(after.starts_with("# Sample\r\n\r\nYou are Sample.\r\n\r\n<!-- omnifrons:begin"));
    assert!(!after.replace("\r\n", "").contains('\n'), "no bare LF");
    assert!(after.ends_with(&format!("{END}\r\n")));
}

#[test]
fn layout_refusals_name_their_cause() {
    let skills = sample_skills();
    let mut more = skills.clone();
    more.push(("skills/second/SKILL.md", skill("second", "Second skill.")));
    let current = text(&applied(&skills));
    let block = &current[current.find(BEGIN).expect("a block")..];

    // This crate's own block: a missing end sentinel, or a second block.
    let no_end = current.replace(&format!("{END}\n"), "");
    let duplicate = format!("{current}\n{block}");
    for (label, agents) in [("no end", no_end), ("duplicate", duplicate)] {
        let refusal = refused(plan_canonical(agents.as_bytes(), &more));
        assert_eq!(refusal.code(), "block_layout", "{label}");
        assert!(refusal.message().contains("not one intact pair"), "{label}");
    }

    // The legacy block: text after it, a second begin, no end, a quoted
    // marker.
    let legacy = legacy_block(&legacy_lines(true));
    let fenced = format!(
        "{}\n```text\n<!-- portable-agent:skills:begin sha256={} -->\n```\n",
        text(BODY),
        "0".repeat(64)
    );
    let cases = [
        (
            "after",
            format!("{}\n{legacy}\n## Later notes\n\nMore.\n", text(BODY)),
            "text follows the end marker",
        ),
        (
            "duplicate",
            format!("{}\n{legacy}{legacy}", text(BODY)),
            "second begin marker",
        ),
        (
            "no end",
            format!(
                "{}\n{}",
                text(BODY),
                legacy.replace("<!-- portable-agent:skills:end -->\n", "")
            ),
            "no end marker",
        ),
        ("fenced", fenced, "inside a fenced code block"),
    ];
    for (label, agents, cause) in cases {
        let refusal = refused(plan_canonical(agents.as_bytes(), &skills));
        assert_eq!(refusal.code(), "block_layout", "{label}");
        assert!(refusal.message().contains(cause), "{label}: {refusal}");
        assert!(
            !refusal.message().contains("sha256 no longer matches"),
            "{label}: {refusal}"
        );
    }
}

#[test]
fn a_parents_block_never_indexes_a_childs_skills() {
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(
                IdentityKind::Claude,
                b"# Parent\n\nThe child/ folder is another agent.\n",
            )],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Claude, b"# Child\n")],
            Vec::new(),
            Vec::new(),
        )
        .skill("skills/own/SKILL.md", skill("own", "Own skill."))
        .skill("child/skills/theirs/SKILL.md", skill("theirs", "Theirs."))
        .build();
    let (plan, writes) = planned(run(snapshot, None, PlanOptions::default()));
    let after = text(&block_bytes(&plan, &writes));
    assert!(after.contains("[own](skills/own/SKILL.md)"));
    assert!(!after.contains("theirs"), "{after}");
    let diff = plan.block_diff.expect("a block diff");
    assert!(!diff.after.iter().any(|line| line.contains("theirs")));
}

#[test]
fn a_secondary_byte_copy_refuses_the_block_write() {
    let with_copy = |kind: IdentityKind| {
        TreeSnapshotBuilder::new()
            .directory(
                ".",
                vec![identity(IdentityKind::Claude, BODY), identity(kind, BODY)],
                Vec::new(),
                Vec::new(),
            )
            .skill("skills/sample/SKILL.md", skill("sample", "Sample skill."))
            .build()
    };
    for kind in [IdentityKind::Qwen, IdentityKind::Gemini] {
        let refusal = refused(run(with_copy(kind), None, PlanOptions::default()));
        assert_eq!(refusal.code(), "diverging_copy", "{kind}");
        assert!(refusal.message().contains(kind.file_name()), "{kind}");
    }

    // `--with-gemini` replaces the GEMINI.md copy, so the block is safe.
    let with_gemini = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let (plan, _) = planned(run(with_copy(IdentityKind::Gemini), None, with_gemini));
    assert_eq!(
        paths(&plan),
        vec!["AGENTS.md", "CLAUDE.md", "GEMINI.md", "AGENTS.md"]
    );
    assert_eq!(block_mode(&plan), Some(BlockMode::Append));
}

#[test]
fn unsafe_skill_metadata_refuses_before_any_operation_is_planned() {
    let skills = vec![(
        "skills/mail/SKILL.md",
        b"---\nname: mail-helper\ndescription: Contact @support for assistance\n---\n".to_vec(),
    )];
    let refusal = refused(run(
        claude_root(BODY, &skills),
        None,
        PlanOptions::default(),
    ));
    assert_eq!(refusal.code(), "unsafe_skill_metadata");
    let refusal = refused(plan_canonical(BODY, &skills));
    assert_eq!(refusal.code(), "unsafe_skill_metadata");
}

#[test]
fn an_intact_legacy_block_plans_one_migration_that_regenerates_the_block() {
    // The legacy block holds a stale row; the migration never copies it.
    let stale = legacy_lines(true)
        .into_iter()
        .map(|line| line.replace("Sample skill.", "Old description."))
        .collect::<Vec<_>>();
    let stale: Vec<&str> = stale.iter().map(String::as_str).collect();
    let before = format!("{}\n{}", text(BODY), legacy_block(&stale));
    let (plan, writes) = planned(plan_canonical(before.as_bytes(), &sample_skills()));
    assert_eq!(paths(&plan), vec!["AGENTS.md"]);
    assert_eq!(block_mode(&plan), Some(BlockMode::MigrateLegacy));
    let after = text(&block_bytes(&plan, &writes));
    let prefix = format!("{}\n", text(BODY));
    assert!(after.starts_with(&prefix), "the text outside is kept");
    assert!(!after.contains("portable-agent:"), "{after}");
    assert!(!after.contains("Old description."), "{after}");
    assert_eq!(count(&after, BEGIN), 1);
    assert!(after.contains(LEGACY_ROW), "the row is regenerated");
    assert_eq!(count(&after, RULE_HEADING), 1, "the rule is regenerated");
    assert!(after.ends_with(&format!("{END}\n")));
    let diff = plan.block_diff.as_ref().expect("a block diff");
    assert!(diff.before[0].starts_with("<!-- portable-agent:skills:begin"));
    assert_eq!(
        diff.before.last().map(String::as_str),
        Some("<!-- portable-agent:skills:end -->")
    );
    assert!(diff.after[0].starts_with(BEGIN));
    assert_eq!(diff.after.last().map(String::as_str), Some(END));

    // Migrated once: the result plans nothing more.
    let (again, _) = planned(plan_canonical(after.as_bytes(), &sample_skills()));
    assert!(again.operations.is_empty(), "{:?}", again.operations);
}

#[test]
fn a_migration_does_not_reindex_what_the_text_outside_the_block_links() {
    let outside = format!("{}\nSee [sample](skills/sample/SKILL.md).\n\n", text(BODY));
    let before = format!("{outside}{}", legacy_block(&legacy_lines(true)));
    let mut skills = sample_skills();
    skills.push(("skills/second/SKILL.md", skill("second", "Second skill.")));
    let (plan, writes) = planned(plan_canonical(before.as_bytes(), &skills));
    let after = text(&block_bytes(&plan, &writes));
    assert!(after.starts_with(&outside));
    assert!(after.contains("[second](skills/second/SKILL.md)"));
    assert_eq!(count(&after, "skills/sample/SKILL.md"), 1, "{after}");
}

#[test]
fn whitespace_after_the_legacy_block_is_kept() {
    let before = format!("{}\n{}\n\n", text(BODY), legacy_block(&legacy_lines(true)));
    let (plan, writes) = planned(plan_canonical(before.as_bytes(), &sample_skills()));
    let after = text(&block_bytes(&plan, &writes));
    assert!(after.ends_with(&format!("{END}\n\n\n")), "{after:?}");
}

#[test]
fn a_hand_edited_legacy_block_is_reported_not_migrated() {
    let edited = legacy_block(&legacy_lines(true)).replace("Sample skill.", "Edited by hand.");
    let before = format!("{}\n{edited}", text(BODY));
    let refusal = refused(plan_canonical(before.as_bytes(), &sample_skills()));
    assert_eq!(refusal.code(), "edited_block");
    assert!(
        refusal.message().contains("edited by hand"),
        "{}",
        refusal.message()
    );
    assert!(refusal.message().contains("cannot be regenerated"));
}

#[test]
fn the_rule_is_added_only_when_the_text_lacks_it() {
    // Absent from the text: the new block carries it.
    let (plan, writes) = planned(run(
        claude_root(BODY, &sample_skills()),
        None,
        PlanOptions::default(),
    ));
    assert_eq!(count(&text(&block_bytes(&plan, &writes)), RULE_HEADING), 1);

    // Already stated outside the block: rows only.
    let stated = format!("{}\n{PRECEDENCE_RULE}\n", text(BODY));
    let (plan, writes) = planned(run(
        claude_root(stated.as_bytes(), &sample_skills()),
        None,
        PlanOptions::default(),
    ));
    let after = text(&block_bytes(&plan, &writes));
    assert_eq!(count(&after, RULE_HEADING), 0, "{after}");
    assert!(after.contains(LEGACY_ROW));

    // A legacy block without the rule gets it; one whose text states
    // the rule elsewhere does not.
    let without = format!("{}\n{}", text(BODY), legacy_block(&legacy_lines(false)));
    let (plan, writes) = planned(plan_canonical(without.as_bytes(), &sample_skills()));
    assert_eq!(count(&text(&block_bytes(&plan, &writes)), RULE_HEADING), 1);
    let elsewhere = format!(
        "{}\n{PRECEDENCE_RULE}\n\n{}",
        text(BODY),
        legacy_block(&legacy_lines(false))
    );
    let (plan, writes) = planned(plan_canonical(elsewhere.as_bytes(), &sample_skills()));
    assert_eq!(count(&text(&block_bytes(&plan, &writes)), RULE_HEADING), 0);
}

#[test]
fn a_canonical_root_with_a_current_block_plans_no_block_operation() {
    let skills = sample_skills();
    let (plan, writes) = planned(plan_canonical(&applied(&skills), &skills));
    assert!(plan.operations.is_empty());
    assert!(plan.block_diff.is_none());
    assert!(plan.notices.is_empty());
    assert!(writes.is_empty());
}

#[test]
fn the_plan_id_changes_when_the_block_body_changes() {
    let plan_with = |description: &str| {
        let skills = vec![("skills/sample/SKILL.md", skill("sample", description))];
        planned(run(
            claude_root(BODY, &skills),
            None,
            PlanOptions::default(),
        ))
        .0
    };
    let first = plan_with("Sample skill.");
    let same = plan_with("Sample skill.");
    let other = plan_with("A different description.");
    assert_eq!(first.id, same.id);
    assert_ne!(first.id, other.id);
    assert_eq!(paths(&first), paths(&other));
}

#[test]
fn a_legacy_block_beside_a_current_block_is_migrated_into_one_block() {
    let skills = sample_skills();
    let current = text(&applied(&skills));
    let legacy = legacy_block(&legacy_lines(true));
    let both = format!("{current}\n{legacy}");
    let workspace = shared_workspace();
    let tree = InMemoryDefinitionTree::new()
        .with_snapshot(&workspace, canonical_root(both.as_bytes(), &skills));
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let report = verify(&ports, &workspace, ".").expect("verify succeeds");
    assert!(
        report
            .problems
            .iter()
            .any(|problem| problem.starts_with("pending: migrate the portable-agent")),
        "{:?}",
        report.problems
    );

    let (plan, writes) = planned(plan_canonical(both.as_bytes(), &skills));
    assert_eq!(paths(&plan), vec!["AGENTS.md"]);
    assert_eq!(block_mode(&plan), Some(BlockMode::MigrateLegacy));
    let after = text(&block_bytes(&plan, &writes));
    assert_eq!(count(&after, BEGIN), 1, "{after}");
    assert!(!after.contains("portable-agent:"), "{after}");
    assert!(after.starts_with(&current[..current.find(BEGIN).expect("a block")]));
    let (again, _) = planned(plan_canonical(after.as_bytes(), &skills));
    assert!(again.operations.is_empty(), "{:?}", again.operations);

    // A hand-edited current block beside it still refuses.
    let edited = both.replace("Sample skill.", "Edited by hand.");
    let refusal = refused(plan_canonical(edited.as_bytes(), &skills));
    assert_eq!(refusal.code(), "edited_block");
}

#[test]
fn a_legacy_block_that_is_not_utf8_is_replaced_wholesale() {
    // The digest covers the exact body bytes, so the block is intact.
    let body: Vec<u8> = [&b"## Local skill index\n\n"[..], b"note \xff stale\n"].concat();
    let mut outside = format!("{}\n", text(BODY)).into_bytes();
    outside.extend_from_slice(b"Notes \xfe kept.\n\n");
    let mut agents = outside.clone();
    agents.extend_from_slice(
        format!(
            "<!-- portable-agent:skills:begin sha256={} -->\n",
            FakeHasher.sha256(&body).to_hex()
        )
        .as_bytes(),
    );
    agents.extend_from_slice(&body);
    agents.extend_from_slice(b"<!-- portable-agent:skills:end -->\n");
    let (plan, writes) = planned(plan_canonical(&agents, &sample_skills()));
    assert_eq!(block_mode(&plan), Some(BlockMode::MigrateLegacy));
    let after = block_bytes(&plan, &writes);
    assert!(after.starts_with(&outside), "bytes outside are identical");
    assert!(!after.contains(&0xff), "nothing of the old body is copied");
    assert!(text(&after[outside.len()..]).starts_with(BEGIN));
}
