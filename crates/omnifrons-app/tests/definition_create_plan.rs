//! The application's plan for a create (ADR-0005 decision 6, sub-slice 3c),
//! against the in-memory `DefinitionTree` double: the operations per
//! creation state, the skills block appended to the rendered body, the
//! gates and their refusals, and what the plan id covers.
//!
//! Only compiled with `--features contract-tests`: the double lives behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::definition::{InMemoryDefinitionTree, TreeSnapshotBuilder};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::DefinitionPorts;
use omnifrons_app::definition_plan::{PlanOutcome, PlannedWrite, plan, plan_create_with_writes};
use omnifrons_app::definition_tree::{PathProbe, TargetLocation, TreeSnapshot};
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, WRAPPER};
use omnifrons_domain::definition_create::{
    Answers, CREATE_TEMPLATE_VERSION, CreateContext, RawValue, answers_canonical, render,
};
use omnifrons_domain::definition_plan::{
    InputFingerprint, Operation, Plan, PlanOptions, PlanRefusal,
};
use omnifrons_domain::nesting::Ancestor;

// ---------------------------------------------------------------- fixtures

/// A scratch directory that exists while the guard lives.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> (Self, WorkspaceRoot) {
        let path = std::env::temp_dir().join(format!(
            "omnifrons-create-plan-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("fixture dir");
        let root = WorkspaceRoot::new(&path).expect("valid workspace");
        (Self(path), root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(value: &str) -> RawValue {
    RawValue::Text(value.to_string())
}

fn item(body: &str) -> RawValue {
    RawValue::Object(vec![
        ("text".to_string(), text(body)),
        ("said".to_string(), text("Said.")),
        ("status".to_string(), text("confirmed")),
    ])
}

fn pairs(sources: &[&str]) -> Vec<(String, RawValue)> {
    let mut pairs = vec![
        ("name", item("Harbor")),
        ("role", item("a scheduling assistant")),
        ("deliverables", RawValue::List(vec![item("Propose slots.")])),
        (
            "authority",
            RawValue::List(vec![item("Draft and advise only.")]),
        ),
        ("work_language", item("the language of each user")),
    ];
    if !sources.is_empty() {
        pairs.push((
            "sources",
            RawValue::List(sources.iter().map(|path| item(path)).collect()),
        ));
    }
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn answers_from(pairs: Vec<(String, RawValue)>) -> Answers {
    match Answers::validate(&RawValue::Object(pairs)) {
        Ok(answers) => answers,
        Err(errors) => panic!("refused: {errors:#?}"),
    }
}

fn answers() -> Answers {
    answers_from(pairs(&[]))
}

fn regular(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn skill(name: &str) -> Vec<u8> {
    format!("---\nname: {name}\ndescription: Does {name} work.\n---\n").into_bytes()
}

/// An empty snapshot: the target holds nothing.
fn empty() -> TreeSnapshotBuilder {
    TreeSnapshotBuilder::new()
}

type Outcome = (PlanOutcome, Vec<PlannedWrite>);

/// Plans a create of `target` under `workspace` over `snapshot`; `prepare`
/// adds probes and locations to the double.
fn create_under(
    workspace: &WorkspaceRoot,
    snapshot: TreeSnapshot,
    target: &str,
    options: PlanOptions,
    answers: &Answers,
    prepare: impl FnOnce(InMemoryDefinitionTree, &WorkspaceRoot) -> InMemoryDefinitionTree,
) -> Outcome {
    let tree = prepare(
        InMemoryDefinitionTree::new().with_snapshot(workspace, snapshot),
        workspace,
    );
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &FakeHasher,
    };
    plan_create_with_writes(&ports, workspace, Some(target), &options, answers).expect("plan")
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn create_with(
    snapshot: TreeSnapshot,
    target: &str,
    options: PlanOptions,
    answers: &Answers,
    prepare: impl FnOnce(InMemoryDefinitionTree, &WorkspaceRoot) -> InMemoryDefinitionTree,
) -> Outcome {
    let (_guard, workspace) = TempRoot::new(&NEXT.fetch_add(1, Ordering::Relaxed).to_string());
    create_under(&workspace, snapshot, target, options, answers, prepare)
}

fn create(snapshot: TreeSnapshot, target: &str, answers: &Answers) -> Outcome {
    create_with(snapshot, target, PlanOptions::default(), answers, |t, _| t)
}

fn planned(outcome: Outcome) -> (Plan, Vec<PlannedWrite>) {
    match outcome {
        (PlanOutcome::Planned(plan), writes) => (plan, writes),
        (PlanOutcome::Refused(refusal), _) => panic!("refused: {refusal}"),
    }
}

fn refusal(outcome: Outcome) -> PlanRefusal {
    match outcome {
        (PlanOutcome::Refused(refusal), writes) => {
            assert!(writes.is_empty());
            refusal
        }
        (PlanOutcome::Planned(plan), _) => panic!("planned: {plan:?}"),
    }
}

fn digest(bytes: &[u8]) -> omnifrons_domain::executable::Sha256Digest {
    FakeHasher.sha256(bytes)
}

fn body(children: &[&str], enclosed: bool) -> String {
    let children = children.iter().map(ToString::to_string).collect();
    render(
        &answers(),
        &CreateContext::new(children, enclosed).expect("context"),
    )
}

fn paths(writes: &[PlannedWrite]) -> Vec<&'static str> {
    writes.iter().map(|write| write.path).collect()
}

// ------------------------------------------------- operations per state

#[test]
fn a_new_folder_plans_the_generated_agents_file_then_the_wrapper() {
    let answers = answers();
    let (plan, writes) = planned(create(empty().build(), ".", &answers));
    let expected = body(&[], false);
    assert_eq!(paths(&writes), ["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(writes[0].bytes, expected.as_bytes());
    assert_eq!(writes[1].bytes, WRAPPER);
    assert_eq!(
        plan.operations,
        [
            Operation::WriteGenerated {
                template_version: CREATE_TEMPLATE_VERSION.to_string(),
                answers_digest: digest(&answers_canonical(&answers)),
                after: digest(expected.as_bytes()),
                before: InputFingerprint::Absent,
            },
            Operation::WriteWrapper {
                kind: IdentityKind::Claude,
                after: digest(WRAPPER),
                before: InputFingerprint::Absent,
            },
        ]
    );
    assert!(plan.inputs.is_empty());
}

#[test]
fn the_gemini_wrapper_is_planned_only_on_request() {
    let options = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let outcome = create_with(empty().build(), ".", options, &answers(), |t, _| t);
    let (plan, writes) = planned(outcome);
    assert_eq!(paths(&writes), ["AGENTS.md", "CLAUDE.md", "GEMINI.md"]);
    assert!(matches!(
        plan.operations[2],
        Operation::WriteWrapper {
            kind: IdentityKind::Gemini,
            ..
        }
    ));
}

#[test]
fn an_agents_file_holding_the_rendered_bytes_plans_only_the_wrapper() {
    let rendered = body(&[], false);
    let snapshot = empty()
        .directory(
            ".",
            vec![regular(IdentityKind::Agents, rendered.as_bytes())],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let (plan, writes) = planned(create(snapshot, ".", &answers()));
    assert_eq!(paths(&writes), ["CLAUDE.md"]);
    assert_eq!(plan.operations.len(), 1);
    assert!(matches!(
        plan.operations[0],
        Operation::WriteWrapper {
            kind: IdentityKind::Claude,
            before: InputFingerprint::Absent,
            ..
        }
    ));
    assert!(plan.inputs.contains_key("AGENTS.md"));
}

#[test]
fn a_finished_create_plans_nothing() {
    let rendered = body(&[], false);
    let snapshot = empty()
        .directory(
            ".",
            vec![
                regular(IdentityKind::Agents, rendered.as_bytes()),
                regular(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let (plan, writes) = planned(create(snapshot, ".", &answers()));
    assert!(plan.operations.is_empty());
    assert!(writes.is_empty());
}

#[test]
fn a_gemini_wrapper_left_by_this_writer_does_not_make_the_folder_an_existing_agent() {
    let rendered = body(&[], false);
    let files = |gemini: bool| {
        let mut files = vec![
            regular(IdentityKind::Agents, rendered.as_bytes()),
            regular(IdentityKind::Claude, WRAPPER),
        ];
        if gemini {
            files.push(regular(IdentityKind::Gemini, WRAPPER));
        }
        empty()
            .directory(".", files, Vec::new(), Vec::new())
            .build()
    };
    let options = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let done = create_with(files(true), ".", options, &answers(), |t, _| t);
    assert!(planned(done).0.operations.is_empty());
    let missing = create_with(files(false), ".", options, &answers(), |t, _| t);
    let (plan, writes) = planned(missing);
    assert_eq!(paths(&writes), ["GEMINI.md"]);
    assert_eq!(plan.operations.len(), 1);
}

#[test]
fn every_other_identity_file_makes_the_folder_an_existing_agent() {
    for kind in IdentityKind::ALL {
        let snapshot = empty()
            .directory(
                ".",
                vec![regular(kind, b"# Their own file\n")],
                Vec::new(),
                Vec::new(),
            )
            .build();
        let result = refusal(create(snapshot, ".", &answers()));
        assert_eq!(result.code(), "identity_exists", "{kind:?}");
        assert!(result.message().contains(kind.file_name()), "{result}");
    }
}

#[test]
fn an_agents_file_that_is_not_the_rendered_one_is_an_existing_agent() {
    let snapshot = empty()
        .directory(
            ".",
            vec![
                regular(IdentityKind::Agents, b"# Changed by hand\n"),
                regular(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let result = refusal(create(snapshot, ".", &answers()));
    assert_eq!(result.code(), "identity_exists");
    assert!(result.message().contains("render"), "{result}");
}

// ------------------------------------------------------ the skills block

#[test]
fn a_folder_with_a_skill_gets_the_block_after_one_blank_line() {
    let snapshot = empty()
        .skill("skills/triage/SKILL.md", skill("triage"))
        .build();
    let (plan, writes) = planned(create(snapshot, ".", &answers()));
    let bytes = String::from_utf8(writes[0].bytes.clone()).expect("utf-8");
    let rendered = body(&[], false);
    let rest = bytes.strip_prefix(&rendered).expect("the body comes first");
    assert!(
        rest.starts_with("\n<!-- omnifrons:begin skills"),
        "{rest:?}"
    );
    assert!(rest.contains("triage"), "{rest}");
    assert!(rest.contains("Does triage work."), "{rest}");
    assert!(bytes.ends_with('\n'));
    let Operation::WriteGenerated { after, .. } = &plan.operations[0] else {
        panic!("the first operation creates AGENTS.md");
    };
    assert_eq!(*after, digest(&writes[0].bytes));
}

#[test]
fn a_folder_without_a_skill_gets_the_body_alone() {
    let (_, writes) = planned(create(empty().build(), ".", &answers()));
    assert_eq!(writes[0].bytes, body(&[], false).as_bytes());
}

#[test]
fn a_finished_create_with_a_skill_is_still_finished() {
    let first = planned(create(
        empty()
            .skill("skills/triage/SKILL.md", skill("triage"))
            .build(),
        ".",
        &answers(),
    ))
    .1;
    let snapshot = empty()
        .directory(
            ".",
            vec![
                regular(IdentityKind::Agents, &first[0].bytes),
                regular(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill("skills/triage/SKILL.md", skill("triage"))
        .build();
    assert!(
        planned(create(snapshot, ".", &answers()))
            .0
            .operations
            .is_empty()
    );
}

#[test]
fn an_unsafe_skill_is_refused_as_the_adapt_path_refuses_it() {
    let snapshot = empty()
        .skill(
            "skills/mail/SKILL.md",
            b"---\nname: mail-helper\ndescription: Contact @support for assistance\n---\n".to_vec(),
        )
        .build();
    let result = refusal(create(snapshot, ".", &answers()));
    assert_eq!(result.code(), "unsafe_skill_metadata");
}

// -------------------------------------------------------------- plan id

fn id_of(workspace: &WorkspaceRoot, answers: &Answers) -> String {
    let outcome = create_under(
        workspace,
        empty().build(),
        ".",
        PlanOptions::default(),
        answers,
        |t, _| t,
    );
    planned(outcome).0.id.to_string()
}

#[test]
fn the_plan_id_changes_with_every_answer() {
    let (_guard, workspace) = TempRoot::new("ids");
    let base = id_of(&workspace, &answers());
    let mut seen = vec![base.clone()];
    for field in ["name", "role", "work_language"] {
        let mut changed = pairs(&[]);
        for (key, value) in &mut changed {
            if key == field {
                *value = item("Something else");
            }
        }
        seen.push(id_of(&workspace, &answers_from(changed)));
    }
    let mut extended = pairs(&[]);
    extended.push(("tone".to_string(), item("Warm and brief.")));
    seen.push(id_of(&workspace, &answers_from(extended)));
    let mut assumed = pairs(&[]);
    assumed.push((
        "tone".to_string(),
        RawValue::Object(vec![
            ("text".to_string(), text("Warm and brief.")),
            ("said".to_string(), text("Said.")),
            ("status".to_string(), text("assumption")),
        ]),
    ));
    seen.push(id_of(&workspace, &answers_from(assumed)));
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "{seen:#?}");
    assert_eq!(id_of(&workspace, &answers()), base);
}

#[test]
fn the_plan_id_does_not_depend_on_key_order() {
    let (_guard, workspace) = TempRoot::new("order");
    let forward = pairs(&[]);
    let mut backward = forward.clone();
    backward.reverse();
    assert_eq!(
        id_of(&workspace, &answers_from(forward)),
        id_of(&workspace, &answers_from(backward))
    );
}

#[test]
fn the_users_own_words_do_not_change_the_plan_id() {
    let (_guard, workspace) = TempRoot::new("said");
    let mut reworded = pairs(&[]);
    for (_, value) in &mut reworded {
        if let RawValue::Object(parts) = value {
            for (key, part) in parts {
                if key == "said" {
                    *part = text("Different words.");
                }
            }
        }
    }
    assert_eq!(
        id_of(&workspace, &answers()),
        id_of(&workspace, &answers_from(reworded))
    );
}

// ---------------------------------------------------------- nesting

fn parent_snapshot() -> TreeSnapshotBuilder {
    empty().directory(
        ".",
        vec![
            regular(IdentityKind::Agents, b"# Parent\n"),
            regular(IdentityKind::Claude, WRAPPER),
        ],
        Vec::new(),
        Vec::new(),
    )
}

#[test]
fn a_target_below_another_agent_needs_its_own_consent() {
    let result = refusal(create(parent_snapshot().build(), "area/inner", &answers()));
    assert_eq!(result.code(), "nested_agent");
    assert!(result.message().contains("area/inner"), "{result}");
    let consenting = PlanOptions {
        nested_consent: true,
        ..PlanOptions::default()
    };
    let outcome = create_with(
        parent_snapshot().build(),
        "area/inner",
        consenting,
        &answers(),
        |t, _| t,
    );
    let (plan, writes) = planned(outcome);
    assert_eq!(plan.agent, "area/inner");
    assert_eq!(
        writes[0].bytes,
        body(&[], true).as_bytes(),
        "the enclosing agent is named in the file"
    );
}

#[test]
fn an_agent_above_the_workspace_counts_as_an_enclosing_agent() {
    let snapshot = {
        let mut snapshot = empty().build();
        snapshot.ancestors.push(Ancestor {
            path: "/somewhere".to_string(),
            identity_files: vec!["agents".to_string()],
            host_extras: Vec::new(),
        });
        snapshot
    };
    let result = refusal(create(snapshot.clone(), "inner", &answers()));
    assert_eq!(result.code(), "nested_agent");
    let consenting = PlanOptions {
        nested_consent: true,
        ..PlanOptions::default()
    };
    let (_, writes) = planned(create_with(
        snapshot,
        "inner",
        consenting,
        &answers(),
        |t, _| t,
    ));
    assert_eq!(writes[0].bytes, body(&[], true).as_bytes());
}

#[test]
fn the_direct_child_agents_are_derived_from_the_snapshot() {
    let child = |path: &str| {
        (
            path.to_string(),
            vec![regular(IdentityKind::Agents, b"# Child\n")],
        )
    };
    let mut builder = empty();
    for (path, files) in [child("area/inner"), child("sales"), child("sales/deep")] {
        builder = builder.directory(path, files, Vec::new(), Vec::new());
    }
    let (_, writes) = planned(create(builder.build(), ".", &answers()));
    assert_eq!(
        writes[0].bytes,
        body(&["area/inner", "sales"], false).as_bytes(),
        "grandchildren belong to their parent, and children are sorted"
    );
}

#[test]
fn a_skill_inside_a_child_agent_is_not_indexed() {
    let snapshot = empty()
        .directory(
            "sales",
            vec![regular(IdentityKind::Agents, b"# Child\n")],
            Vec::new(),
            Vec::new(),
        )
        .skill("sales/skills/pricing/SKILL.md", skill("pricing"))
        .build();
    let (_, writes) = planned(create(snapshot, ".", &answers()));
    assert_eq!(writes[0].bytes, body(&["sales"], false).as_bytes());
}

// ------------------------------------------------------- unsafe targets

fn located(components: &[&str], home: bool) -> TargetLocation {
    TargetLocation {
        components: components.iter().map(ToString::to_string).collect(),
        home,
    }
}

fn unsafe_target(
    target: &str,
    prepare: impl FnOnce(InMemoryDefinitionTree, &WorkspaceRoot) -> InMemoryDefinitionTree,
) -> PlanRefusal {
    let outcome = create_with(
        empty().build(),
        target,
        PlanOptions::default(),
        &answers(),
        prepare,
    );
    refusal(outcome)
}

#[test]
fn a_symlinked_target_is_refused() {
    let result = unsafe_target("link", |tree, root| {
        tree.with_probe(
            root,
            "link",
            PathProbe::Symlink {
                link: "elsewhere".to_string(),
            },
        )
    });
    assert_eq!(result.code(), "unsafe_target");
    assert!(result.message().contains("symlink"), "{result}");
    assert!(result.message().contains("elsewhere"), "{result}");
}

#[test]
fn the_home_directory_is_refused() {
    let result = unsafe_target("me", |tree, root| {
        tree.with_location(root, "me", located(&["home", "me"], true))
    });
    assert_eq!(result.code(), "unsafe_target");
    assert!(result.message().contains("home directory"), "{result}");
}

#[test]
fn a_host_configuration_folder_is_refused_at_any_depth() {
    let cases: [(&[&str], &str); 4] = [
        (&["work", ".claude"], "host configuration folder (.claude/)"),
        (&["work", ".Claude"], "host configuration folder (.Claude/)"),
        (
            &["work", ".cursor", "rules"],
            "inside the host configuration folder",
        ),
        (
            &[".claude", "a", "b"],
            "inside the host configuration folder",
        ),
    ];
    for (components, fragment) in cases {
        let result = unsafe_target("t", |tree, root| {
            tree.with_location(root, "t", located(components, false))
        });
        assert_eq!(result.code(), "unsafe_target", "{components:?}");
        assert!(
            result.message().contains(fragment),
            "{components:?}: {result}"
        );
    }
}

#[test]
fn a_target_next_to_a_host_folder_is_not_refused() {
    let outcome = create_with(
        empty().build(),
        "t",
        PlanOptions::default(),
        &answers(),
        |tree, root| tree.with_location(root, "t", located(&["work", "claude-notes"], false)),
    );
    planned(outcome);
}

#[test]
fn the_unsafe_target_gate_outranks_an_existing_agent() {
    let snapshot = empty()
        .directory(
            "t",
            vec![regular(IdentityKind::Agents, b"# Theirs\n")],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let outcome = create_with(
        snapshot,
        "t",
        PlanOptions::default(),
        &answers(),
        |tree, root| tree.with_location(root, "t", located(&["work", ".claude"], false)),
    );
    assert_eq!(refusal(outcome).code(), "unsafe_target");
}

#[test]
fn a_target_below_a_pruned_directory_is_refused() {
    for target in ["node_modules/pkg", ".git/hooks", "a/.Git/b"] {
        let result = unsafe_target(target, |tree, _| tree);
        assert_eq!(result.code(), "unsafe_target", "{target}");
        assert!(
            result.message().contains("never enters"),
            "{target}: {result}"
        );
    }
    let result = unsafe_target("node_modules/pkg", |tree, _| tree);
    assert!(result.message().contains("node_modules/"), "{result}");
}

/// The physical components of `root` followed by `below`.
fn under_root(root: &WorkspaceRoot, below: &[&str]) -> Vec<String> {
    let mut components: Vec<String> = root
        .path()
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    components.extend(below.iter().map(ToString::to_string));
    components
}

#[test]
fn a_pruned_directory_below_the_root_in_the_physical_path_is_refused() {
    let result = unsafe_target("t", |tree, root| {
        let components = under_root(root, &["__pycache__", "x"]);
        tree.with_location(
            root,
            "t",
            TargetLocation {
                components,
                home: false,
            },
        )
    });
    assert_eq!(result.code(), "unsafe_target");
    assert!(result.message().contains("__pycache__/"), "{result}");
}

#[test]
fn a_root_that_sits_under_a_pruned_name_still_plans_a_create() {
    let outcome = create_with(
        empty().build(),
        "t",
        PlanOptions::default(),
        &answers(),
        |tree, root| {
            let mut above = under_root(root, &["t"]);
            above.insert(0, "node_modules".to_string());
            let root_only = {
                let mut parts = under_root(root, &[]);
                parts.insert(0, "node_modules".to_string());
                parts
            };
            tree.with_location(
                root,
                ".",
                TargetLocation {
                    components: root_only,
                    home: false,
                },
            )
            .with_location(
                root,
                "t",
                TargetLocation {
                    components: above,
                    home: false,
                },
            )
        },
    );
    planned(outcome);
}

#[test]
fn a_name_that_only_resembles_a_pruned_directory_is_accepted() {
    let outcome = create_with(
        empty().build(),
        "node-modules",
        PlanOptions::default(),
        &answers(),
        |tree, _| tree,
    );
    planned(outcome);
}

#[test]
fn a_target_reached_through_a_linked_folder_is_refused() {
    let result = unsafe_target("alias/child", |tree, root| {
        tree.with_probe(
            root,
            "alias",
            PathProbe::Symlink {
                link: "real".to_string(),
            },
        )
        .with_probe(root, "alias/child", PathProbe::Present)
    });
    assert_eq!(result.code(), "unsafe_target");
    assert!(result.message().contains("alias"), "{result}");
    assert!(result.message().contains("symlink to real"), "{result}");
}

// ------------------------------------------------------------- sources

fn with_sources(
    sources: &[&str],
    snapshot: TreeSnapshot,
    prepare: impl FnOnce(InMemoryDefinitionTree, &WorkspaceRoot) -> InMemoryDefinitionTree,
) -> Outcome {
    create_with(
        snapshot,
        ".",
        PlanOptions::default(),
        &answers_from(pairs(sources)),
        prepare,
    )
}

#[test]
fn sources_that_exist_are_planned() {
    let outcome = with_sources(
        &["docs/brief.md", "notes"],
        empty().build(),
        |tree, root| {
            tree.with_file(root, "docs/brief.md")
                .with_file(root, "notes")
        },
    );
    let (_, writes) = planned(outcome);
    let text = String::from_utf8(writes[0].bytes.clone()).expect("utf-8");
    assert!(text.contains("`docs/brief.md`"), "{text}");
}

#[test]
fn a_missing_source_is_refused_by_position() {
    let outcome = with_sources(
        &["docs/brief.md", "gone.md"],
        empty().build(),
        |tree, root| tree.with_file(root, "docs/brief.md"),
    );
    let result = refusal(outcome);
    assert_eq!(result.code(), "invalid_source");
    assert!(result.message().contains("sources[1].text"), "{result}");
    assert!(
        result.message().contains("gone.md does not exist"),
        "{result}"
    );
}

#[test]
fn a_source_through_a_symlink_is_refused() {
    let outcome = with_sources(&["docs/brief.md"], empty().build(), |tree, root| {
        tree.with_file(root, "docs").with_probe(
            root,
            "docs",
            PathProbe::Symlink {
                link: "../other".to_string(),
            },
        )
    });
    let result = refusal(outcome);
    assert_eq!(result.code(), "invalid_source");
    assert!(result.message().contains("docs is a symlink"), "{result}");
}

fn child_snapshot() -> TreeSnapshot {
    empty()
        .directory(
            "sales",
            vec![regular(IdentityKind::Agents, b"# Child\n")],
            Vec::new(),
            Vec::new(),
        )
        .build()
}

#[test]
fn a_source_inside_a_child_agent_is_refused() {
    for source in ["sales", "sales/pricing.md"] {
        let outcome = with_sources(&[source], child_snapshot(), |tree, root| {
            tree.with_file(root, source)
        });
        let result = refusal(outcome);
        assert_eq!(result.code(), "invalid_source", "{source}");
        assert!(
            result.message().contains("inside the child agent sales/"),
            "{result}"
        );
    }
}

#[test]
fn a_source_around_a_child_agent_is_refused() {
    let snapshot = empty()
        .directory(
            "area/sales",
            vec![regular(IdentityKind::Agents, b"# Child\n")],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let outcome = with_sources(&["area"], snapshot, |tree, root| {
        tree.with_file(root, "area")
    });
    let result = refusal(outcome);
    assert_eq!(result.code(), "invalid_source");
    assert!(
        result
            .message()
            .contains("contains the child agent area/sales/"),
        "{result}"
    );
}

#[test]
fn a_source_beside_a_child_agent_is_allowed() {
    let outcome = with_sources(&["sales-notes.md"], child_snapshot(), |tree, root| {
        tree.with_file(root, "sales-notes.md")
    });
    planned(outcome);
}

#[test]
fn a_source_that_is_not_a_plain_relative_path_is_refused_before_any_probe() {
    for entry in [
        "../x", "/abs", "a/./b", "a//b", "a/", "./a", "..", ".", "a/../b",
    ] {
        // Everything the entry could name is present, so only the shape
        // of the entry can refuse it.
        let outcome = with_sources(&[entry], empty().build(), |tree, root| {
            let mut tree = tree;
            for probe in [
                "../x", "/abs", "a", "a/b", "a/.", "a/./b", "a//b", "a/", "./a", "..", ".", "b",
            ] {
                tree = tree.with_probe(root, probe, PathProbe::Present);
            }
            tree
        });
        let result = refusal(outcome);
        assert_eq!(result.code(), "invalid_source", "{entry}");
        assert!(
            result.message().contains("sources[0].text"),
            "{entry}: {result}"
        );
        assert!(
            result.message().contains("plain relative path"),
            "{entry}: {result}"
        );
        assert!(
            !result.message().contains("does not exist"),
            "{entry}: {result}"
        );
    }
}

#[test]
fn sources_are_resolved_below_the_target() {
    let outcome = create_with(
        empty().build(),
        "team",
        PlanOptions::default(),
        &answers_from(pairs(&["brief.md"])),
        |tree, root| tree.with_file(root, "team/brief.md"),
    );
    planned(outcome);
}

// ----------------------------------------------------- other callers

#[test]
fn the_plain_planner_does_not_create() {
    let (_guard, workspace) = TempRoot::new("plain");
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, empty().build());
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &FakeHasher,
    };
    let outcome = plan(&ports, &workspace, None, &PlanOptions::default()).expect("plan");
    let PlanOutcome::Planned(plan) = outcome else {
        panic!("a folder with no definition plans nothing");
    };
    assert!(plan.operations.is_empty());
}

/// A tree that only snapshots, as the doubles of other tests do.
struct SnapshotOnly(InMemoryDefinitionTree);

impl omnifrons_app::definition_tree::DefinitionTree for SnapshotOnly {
    fn snapshot(
        &self,
        root: &WorkspaceRoot,
    ) -> Result<TreeSnapshot, omnifrons_app::definition_tree::DefinitionTreeError> {
        self.0.snapshot(root)
    }
}

#[test]
fn a_tree_that_cannot_probe_is_an_error_not_an_empty_answer() {
    use omnifrons_app::definition::DefinitionError;
    use omnifrons_app::definition_plan::plan_create;
    use omnifrons_app::definition_tree::{DefinitionTree, DefinitionTreeError};
    let (_guard, workspace) = TempRoot::new("snapshot-only");
    let tree =
        SnapshotOnly(InMemoryDefinitionTree::new().with_snapshot(&workspace, empty().build()));
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &FakeHasher,
    };
    let result = plan_create(
        &ports,
        &workspace,
        None,
        &PlanOptions::default(),
        &answers(),
    );
    assert_eq!(
        result,
        Err(DefinitionError::Tree(DefinitionTreeError::ProbeUnsupported))
    );
    assert_eq!(
        tree.probe(&workspace, "."),
        Err(DefinitionTreeError::ProbeUnsupported)
    );
}
